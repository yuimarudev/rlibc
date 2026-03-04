#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::{c_char, c_int};
use rlibc::abi::errno::ENOTDIR;
use rlibc::dirent::{Dirent, closedir, opendir, readdir};
use rlibc::errno::__errno_location;
use rlibc::glob::{
  GLOB_ABORTED, GLOB_APPEND, GLOB_DOOFFS, GLOB_ERR, GLOB_NOCHECK, GLOB_NOESCAPE, GLOB_NOMATCH,
  Glob, glob, globfree,
};
use std::ffi::{CStr, CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

static ERRFUNC_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
  path: PathBuf,
}

struct GlobState {
  inner: Glob,
}

impl GlobState {
  const fn new() -> Self {
    Self {
      inner: Glob {
        gl_pathc: 0,
        gl_pathv: core::ptr::null_mut(),
        gl_offs: 0,
        gl_flags: 0,
      },
    }
  }

  const fn as_mut_ptr(&mut self) -> *mut Glob {
    &raw mut self.inner
  }

  fn path_count(&self) -> usize {
    usize::try_from(self.inner.gl_pathc)
      .unwrap_or_else(|_| unreachable!("gl_pathc must fit usize on x86_64"))
  }

  fn offsets(&self) -> usize {
    usize::try_from(self.inner.gl_offs)
      .unwrap_or_else(|_| unreachable!("gl_offs must fit usize on x86_64"))
  }

  fn paths(&self) -> Vec<String> {
    let mut paths = Vec::with_capacity(self.path_count());
    let mut index = 0_usize;

    while index < self.path_count() {
      // SAFETY: `glob` initializes `gl_pathv` with at least `gl_offs + gl_pathc + 1` entries.
      let entry_ptr = unsafe { self.inner.gl_pathv.add(self.offsets() + index).read() };

      assert!(!entry_ptr.is_null(), "glob returned a null path entry");
      // SAFETY: every path entry is a NUL-terminated string allocated by `glob`.
      let bytes = unsafe { CStr::from_ptr(entry_ptr).to_bytes() };

      paths.push(String::from_utf8_lossy(bytes).into_owned());
      index += 1;
    }

    paths
  }

  const fn raw(&self) -> &Glob {
    &self.inner
  }
}

impl Drop for GlobState {
  fn drop(&mut self) {
    // SAFETY: `globfree` accepts empty state and state previously returned by `glob`.
    unsafe {
      globfree(self.as_mut_ptr());
    }
  }
}

impl TempDir {
  fn new() -> Self {
    let timestamp = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system clock before unix epoch")
      .as_nanos();
    let path = std::env::temp_dir().join(format!("rlibc-i037-{timestamp}-{}", std::process::id()));

    fs::create_dir_all(&path).unwrap_or_else(|error| {
      panic!("failed to create temp dir {}: {error}", path.display());
    });

    Self { path }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TempDir {
  fn drop(&mut self) {
    if !self.path.exists() {
      return;
    }

    if let Err(error) = remove_dir_tree(self.path()) {
      eprintln!(
        "failed to clean temporary directory {}: {error}",
        self.path.display()
      );
    }
  }
}

fn dirent_name_bytes(entry: *const Dirent) -> Vec<u8> {
  // SAFETY: `entry` is returned by `readdir` and valid until the next `readdir`.
  let entry_ref = unsafe { &*entry };
  // SAFETY: `readdir` provides a NUL-terminated entry name buffer.
  let name = unsafe { CStr::from_ptr(entry_ref.d_name.as_ptr()) };

  name.to_bytes().to_vec()
}

fn remove_dir_tree(path: &Path) -> io::Result<()> {
  let path_c = CString::new(path.as_os_str().as_bytes())
    .unwrap_or_else(|error| panic!("temporary directory path contains interior NUL: {error}"));
  // SAFETY: `path_c` is a valid NUL-terminated string for this call.
  let dir = unsafe { opendir(path_c.as_ptr()) };

  if dir.is_null() {
    return Err(io::Error::other(format!(
      "opendir failed with errno={}",
      read_errno()
    )));
  }

  let mut traversal_result: io::Result<()> = Ok(());

  loop {
    write_errno(0);
    // SAFETY: `dir` remains valid until `closedir`.
    let entry = unsafe { readdir(dir) };

    if entry.is_null() {
      let errno_value = read_errno();

      if errno_value != 0 {
        traversal_result = Err(io::Error::other(format!(
          "readdir failed with errno={errno_value}"
        )));
      }

      break;
    }

    let entry_name = dirent_name_bytes(entry);

    if entry_name == b"." || entry_name == b".." {
      continue;
    }

    let child_path = path.join(OsStr::from_bytes(&entry_name));
    let file_type = fs::symlink_metadata(&child_path)?.file_type();

    if file_type.is_dir() {
      remove_dir_tree(&child_path)?;
      continue;
    }

    fs::remove_file(&child_path)?;
  }

  // SAFETY: `dir` is still a live handle from `opendir`.
  let close_rc = unsafe { closedir(dir) };

  if close_rc != 0 {
    let errno_value = read_errno();

    if traversal_result.is_ok() {
      traversal_result = Err(io::Error::other(format!(
        "closedir failed with errno={errno_value}"
      )));
    }
  }

  traversal_result?;
  fs::remove_dir(path)
}

fn create_file(path: &Path) {
  fs::write(path, b"fixture")
    .unwrap_or_else(|error| panic!("failed to create file {}: {error}", path.display()));
}

fn pattern(base: &Path, tail: &str) -> CString {
  let pattern_path = base.join(tail);

  CString::new(pattern_path.as_os_str().as_bytes())
    .unwrap_or_else(|error| panic!("pattern contains interior NUL: {error}"))
}

fn stringify_path(path: &Path) -> String {
  path.as_os_str().to_string_lossy().into_owned()
}

fn run_glob(pattern: &CString, flags: c_int, state: &mut GlobState) -> c_int {
  run_glob_with_errfunc(pattern, flags, None, state)
}

fn run_glob_with_errfunc(
  pattern: &CString,
  flags: c_int,
  errfunc: Option<unsafe extern "C" fn(*const c_char, c_int) -> c_int>,
  state: &mut GlobState,
) -> c_int {
  // SAFETY: pattern is NUL-terminated and state points to writable `Glob`.
  unsafe { glob(pattern.as_ptr(), flags, errfunc, state.as_mut_ptr()) }
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns valid writable thread-local storage.
  unsafe { *__errno_location() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns valid writable thread-local storage.
  unsafe {
    *__errno_location() = value;
  }
}

#[test]
fn glob_star_matches_multiple_entries_and_sorts_output() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("atom.txt"));
  create_file(&temp_dir.path().join("alpha.txt"));
  create_file(&temp_dir.path().join("beta.txt"));
  create_file(&temp_dir.path().join(".hidden.txt"));

  let pattern = pattern(temp_dir.path(), "a*.txt");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 2);
  assert_eq!(
    state.paths(),
    vec![
      stringify_path(&temp_dir.path().join("alpha.txt")),
      stringify_path(&temp_dir.path().join("atom.txt")),
    ],
  );
}

#[test]
fn glob_escaped_leading_dot_pattern_matches_hidden_entries() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join(".alpha.cfg"));
  create_file(&temp_dir.path().join(".beta.cfg"));
  create_file(&temp_dir.path().join("alpha.cfg"));

  let pattern = pattern(temp_dir.path(), "\\.*.cfg");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 2);
  assert_eq!(
    state.paths(),
    vec![
      stringify_path(&temp_dir.path().join(".alpha.cfg")),
      stringify_path(&temp_dir.path().join(".beta.cfg")),
    ],
  );
}

#[test]
fn glob_escaped_separator_matches_directory_boundary() {
  let temp_dir = TempDir::new();
  let nested_dir = temp_dir.path().join("nested");

  fs::create_dir(&nested_dir).unwrap_or_else(|error| {
    panic!(
      "failed to create directory {}: {error}",
      nested_dir.display()
    );
  });
  create_file(&nested_dir.join("item.txt"));

  let pattern = pattern(temp_dir.path(), "nested\\/item.txt");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(
    state.paths(),
    vec![stringify_path(&nested_dir.join("item.txt"))]
  );
}

#[test]
fn glob_repeated_separator_in_pattern_is_preserved_in_result_path() {
  let temp_dir = TempDir::new();
  let nested_dir = temp_dir.path().join("nested");

  fs::create_dir(&nested_dir).unwrap_or_else(|error| {
    panic!(
      "failed to create directory {}: {error}",
      nested_dir.display()
    );
  });
  create_file(&nested_dir.join("item.txt"));

  let pattern = pattern(temp_dir.path(), "nested//*.txt");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(
    state.paths(),
    vec![format!("{}//{}", stringify_path(&nested_dir), "item.txt")],
  );
}

#[test]
fn glob_triple_root_separator_preserves_double_slash_result() {
  let pattern =
    CString::new("///").unwrap_or_else(|error| panic!("pattern contains interior NUL: {error}"));
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(state.paths(), vec![String::from("//")]);
}

#[test]
fn glob_escaped_and_literal_leading_separators_preserve_double_slash_result() {
  let pattern =
    CString::new("\\//").unwrap_or_else(|error| panic!("pattern contains interior NUL: {error}"));
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(state.paths(), vec![String::from("//")]);
}

#[test]
fn glob_escaped_double_root_with_dot_segment_collapses_to_single_root_prefix() {
  let pattern =
    CString::new("\\//.").unwrap_or_else(|error| panic!("pattern contains interior NUL: {error}"));
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(state.paths(), vec![String::from("/.")]);
}

#[test]
fn glob_double_root_dot_subpath_preserves_double_prefix() {
  let pattern = CString::new("//./tmp")
    .unwrap_or_else(|error| panic!("pattern contains interior NUL: {error}"));
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(state.paths(), vec![String::from("//./tmp")]);
}

#[test]
fn glob_dot_star_includes_dot_and_dotdot_entries() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join(".alpha.cfg"));

  let pattern = pattern(temp_dir.path(), ".*");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 3);

  let mut paths = state.paths();

  paths.sort();

  let mut expected = vec![
    stringify_path(&temp_dir.path().join(".")),
    stringify_path(&temp_dir.path().join("..")),
    stringify_path(&temp_dir.path().join(".alpha.cfg")),
  ];

  expected.sort();

  assert_eq!(paths, expected);
}

#[test]
fn glob_trailing_slash_matches_directories_only_and_preserves_slash() {
  let temp_dir = TempDir::new();
  let nested_dir = temp_dir.path().join("nested");

  fs::create_dir(&nested_dir).unwrap_or_else(|error| {
    panic!(
      "failed to create directory {}: {error}",
      nested_dir.display()
    );
  });
  create_file(&temp_dir.path().join("plain.txt"));

  let pattern = pattern(temp_dir.path(), "*/");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(
    state.paths(),
    vec![format!("{}/", stringify_path(&nested_dir))],
  );
}

#[test]
fn glob_bracket_leading_dot_class_matches_hidden_entries() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join(".alpha.cfg"));
  create_file(&temp_dir.path().join("aalpha.cfg"));

  let pattern = pattern(temp_dir.path(), "[.]alpha.cfg");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(
    state.paths(),
    vec![stringify_path(&temp_dir.path().join(".alpha.cfg"))],
  );
}

#[test]
fn glob_literal_escaped_dot_matches_hidden_entry() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join(".alpha.cfg"));
  create_file(&temp_dir.path().join("alpha.cfg"));

  let pattern = pattern(temp_dir.path(), "\\.alpha.cfg");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(
    state.paths(),
    vec![stringify_path(&temp_dir.path().join(".alpha.cfg"))],
  );
}

#[test]
fn glob_literal_escaped_dot_with_noescape_does_not_match_hidden_entry() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join(".alpha.cfg"));

  let pattern = pattern(temp_dir.path(), "\\.alpha.cfg");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_negated_bracket_prefix_does_not_match_hidden_entries() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join(".alpha.cfg"));
  create_file(&temp_dir.path().join("beta.cfg"));
  create_file(&temp_dir.path().join("alpha.cfg"));

  let pattern = pattern(temp_dir.path(), "[!a]*.cfg");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(
    state.paths(),
    vec![stringify_path(&temp_dir.path().join("beta.cfg"))],
  );
}

#[test]
fn glob_question_mark_matches_exactly_one_character() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("x1.log"));
  create_file(&temp_dir.path().join("xy.log"));
  create_file(&temp_dir.path().join("xyz.log"));

  let pattern = pattern(temp_dir.path(), "x?.log");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(
    state.paths(),
    vec![
      stringify_path(&temp_dir.path().join("x1.log")),
      stringify_path(&temp_dir.path().join("xy.log")),
    ],
  );
}

#[test]
fn glob_bracket_expression_matches_set_and_range() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("a1.bin"));
  create_file(&temp_dir.path().join("b1.bin"));
  create_file(&temp_dir.path().join("c1.bin"));
  create_file(&temp_dir.path().join("d1.bin"));

  let pattern = pattern(temp_dir.path(), "[a-c]1.bin");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(
    state.paths(),
    vec![
      stringify_path(&temp_dir.path().join("a1.bin")),
      stringify_path(&temp_dir.path().join("b1.bin")),
      stringify_path(&temp_dir.path().join("c1.bin")),
    ],
  );
}

#[test]
fn glob_bracket_expression_descending_range_returns_nomatch() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("a1.bin"));
  create_file(&temp_dir.path().join("b1.bin"));
  create_file(&temp_dir.path().join("z1.bin"));

  let pattern = pattern(temp_dir.path(), "[z-a]1.bin");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_bracket_expression_treats_escaped_hyphen_as_literal() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("a1.bin"));
  create_file(&temp_dir.path().join("-1.bin"));
  create_file(&temp_dir.path().join("c1.bin"));
  create_file(&temp_dir.path().join("b1.bin"));

  let pattern = pattern(temp_dir.path(), "[a\\-c]1.bin");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(
    state.paths(),
    vec![
      stringify_path(&temp_dir.path().join("-1.bin")),
      stringify_path(&temp_dir.path().join("a1.bin")),
      stringify_path(&temp_dir.path().join("c1.bin")),
    ],
  );
}

#[test]
fn glob_no_match_returns_glob_nomatch() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("exists.txt"));

  let pattern = pattern(temp_dir.path(), "missing-*.txt");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_nocheck_returns_literal_pattern_when_no_match() {
  let temp_dir = TempDir::new();
  let pattern = pattern(temp_dir.path(), "still-missing-*.txt");
  let pattern_string = String::from_utf8_lossy(pattern.as_bytes()).into_owned();
  let mut state = GlobState::new();
  let result = run_glob(&pattern, GLOB_NOCHECK, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(state.paths(), vec![pattern_string]);
}

#[test]
fn glob_nocheck_collapses_double_root_single_segment_and_trims_trailing_separators() {
  let pattern = CString::new("//still-missing-i037///")
    .unwrap_or_else(|error| panic!("pattern contains interior NUL: {error}"));
  let mut state = GlobState::new();
  let result = run_glob(&pattern, GLOB_NOCHECK, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(state.paths(), vec![String::from("/still-missing-i037")]);
}

#[test]
fn glob_nocheck_preserves_escaped_trailing_separator_literal() {
  let temp_dir = TempDir::new();
  let pattern = pattern(temp_dir.path(), "still-missing-i037\\/");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, GLOB_NOCHECK, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);
  assert_eq!(state.paths(), vec![stringify_path(temp_dir.path()) + "/still-missing-i037\\/"]);
}

#[test]
fn glob_dooffs_reserves_leading_null_slots_and_terminator() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("alpha.txt"));
  create_file(&temp_dir.path().join("beta.txt"));

  let pattern = pattern(temp_dir.path(), "*.txt");
  let mut state = GlobState::new();

  state.inner.gl_offs = 2;

  write_errno(0);

  let result = run_glob(&pattern, GLOB_DOOFFS, &mut state);

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.offsets(), 2);
  assert_eq!(state.path_count(), 2);
  assert_eq!(
    state.paths(),
    vec![
      stringify_path(&temp_dir.path().join("alpha.txt")),
      stringify_path(&temp_dir.path().join("beta.txt")),
    ],
  );

  let offsets = state.offsets();
  let path_count = state.path_count();

  // SAFETY: successful `glob` guarantees `gl_pathv` layout with offsets + count + terminator.
  unsafe {
    let pathv = state.raw().gl_pathv;

    assert!(!pathv.is_null());
    assert!(pathv.read().is_null());
    assert!(pathv.add(1).read().is_null());
    assert!(pathv.add(offsets + path_count).read().is_null());
  }
}

#[test]
fn glob_append_preserves_offsets_and_appends_matches() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("alpha.txt"));
  create_file(&temp_dir.path().join("beta.txt"));

  let first_pattern = pattern(temp_dir.path(), "alpha*.txt");
  let second_pattern = pattern(temp_dir.path(), "beta*.txt");
  let mut state = GlobState::new();

  state.inner.gl_offs = 1;

  write_errno(0);

  let first_result = run_glob(&first_pattern, GLOB_DOOFFS, &mut state);
  let second_result = run_glob(&second_pattern, GLOB_APPEND, &mut state);

  assert_eq!(first_result, 0);
  assert_eq!(second_result, 0);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.offsets(), 1);
  assert_eq!(state.path_count(), 2);
  assert_eq!(
    state.paths(),
    vec![
      stringify_path(&temp_dir.path().join("alpha.txt")),
      stringify_path(&temp_dir.path().join("beta.txt")),
    ],
  );

  let offsets = state.offsets();
  let path_count = state.path_count();

  // SAFETY: successful `glob` keeps one leading offset slot and one trailing terminator.
  unsafe {
    let pathv = state.raw().gl_pathv;

    assert!(!pathv.is_null());
    assert!(pathv.read().is_null());
    assert!(pathv.add(offsets + path_count).read().is_null());
  }
}

#[test]
fn globfree_clears_state_and_is_safe_to_call_twice() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("entry.txt"));

  let pattern = pattern(temp_dir.path(), "*.txt");
  let mut state = GlobState::new();
  let result = run_glob(&pattern, 0, &mut state);

  assert_eq!(result, 0);
  assert_eq!(state.path_count(), 1);

  // SAFETY: state was initialized by `glob`.
  unsafe {
    globfree(state.as_mut_ptr());
    globfree(state.as_mut_ptr());
  }

  assert_eq!(state.raw().gl_pathc, 0);
  assert!(state.raw().gl_pathv.is_null());
  assert_eq!(state.raw().gl_offs, 0);
}

unsafe extern "C" fn aborting_errfunc(_epath: *const c_char, _eerrno: c_int) -> c_int {
  ERRFUNC_CALL_COUNT.fetch_add(1, Ordering::Relaxed);

  1
}

#[test]
fn glob_err_flag_aborts_on_directory_error_and_sets_errno() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/*");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_ABORTED);
  assert_eq!(read_errno(), ENOTDIR);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_literal_wildcard_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\*");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_literal_question_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\?");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_literal_open_bracket_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_closing_bracket_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_empty_bracket_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_caret_negated_bracket_class_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[^ab]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_negated_bracket_class_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[!ab]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_bracket_class_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[ab]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_unclosed_bracket_class_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[ab");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_unclosed_negated_bracket_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[!ab");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_ignores_escaped_unclosed_caret_negated_bracket_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[^ab");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_wildcard_as_meta() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\*");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_ABORTED);
  assert_eq!(read_errno(), ENOTDIR);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_question_as_meta() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\?");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_ABORTED);
  assert_eq!(read_errno(), ENOTDIR);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_bracket_class_as_meta() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[ab]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_ABORTED);
  assert_eq!(read_errno(), ENOTDIR);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_negated_bracket_class_as_meta() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[!ab]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_ABORTED);
  assert_eq!(read_errno(), ENOTDIR);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_caret_negated_bracket_class_as_meta() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[^ab]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_ABORTED);
  assert_eq!(read_errno(), ENOTDIR);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_open_bracket_as_literal_when_unclosed() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_unclosed_negated_bracket_as_literal() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[!ab");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_unclosed_caret_negated_bracket_as_literal() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[^ab");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_empty_bracket_as_literal() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\[]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_with_noescape_treats_escaped_closing_bracket_as_literal() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/\\]");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR | GLOB_NOESCAPE, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_err_flag_treats_unclosed_bracket_literal_on_non_directory_prefix() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  let pattern = pattern(temp_dir.path(), "not-a-directory/[abc");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob(&pattern, GLOB_ERR, &mut state);

  assert_eq!(result, GLOB_NOMATCH);
  assert_eq!(read_errno(), 0);
  assert_eq!(state.path_count(), 0);
}

#[test]
fn glob_errfunc_nonzero_return_aborts_and_sets_errno() {
  let temp_dir = TempDir::new();

  create_file(&temp_dir.path().join("not-a-directory"));

  ERRFUNC_CALL_COUNT.store(0, Ordering::Relaxed);

  let pattern = pattern(temp_dir.path(), "not-a-directory/*");
  let mut state = GlobState::new();

  write_errno(0);

  let result = run_glob_with_errfunc(&pattern, 0, Some(aborting_errfunc), &mut state);

  assert_eq!(result, GLOB_ABORTED);
  assert_eq!(read_errno(), ENOTDIR);
  assert_eq!(ERRFUNC_CALL_COUNT.load(Ordering::Relaxed), 1);
  assert_eq!(state.path_count(), 0);
}
