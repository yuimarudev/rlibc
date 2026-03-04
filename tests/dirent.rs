#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::{c_char, c_int};
use rlibc::abi::errno::{EFAULT, EINVAL, ENOENT, ENOTDIR};
use rlibc::dirent::{Dir, closedir, opendir, readdir, rewinddir};
use rlibc::errno::__errno_location;
use std::collections::BTreeSet;
use std::ffi::{CStr, CString, OsStr};
use std::fs::{self, File};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir {
  path: PathBuf,
}

impl TempDir {
  fn new(prefix: &str) -> Self {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let now_nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time must be after unix epoch")
      .as_nanos();
    let path = std::env::temp_dir().join(format!("rlibc-{prefix}-{pid}-{now_nanos}-{counter}"));

    fs::create_dir_all(&path).expect("failed to create temporary test directory");

    Self { path }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TempDir {
  fn drop(&mut self) {
    if let Err(error) = remove_path_recursive(self.path()) {
      eprintln!(
        "failed to clean temporary directory {}: {error}",
        self.path.display()
      );
    }
  }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local storage for this thread.
  unsafe {
    __errno_location().write(value);
  }
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns readable thread-local storage for this thread.
  unsafe { __errno_location().read() }
}

fn path_to_c_string(path: &Path) -> CString {
  CString::new(path.as_os_str().as_bytes()).expect("path must not contain interior NUL bytes")
}

fn remove_path_recursive(path: &Path) -> Result<(), String> {
  if !path.exists() {
    return Ok(());
  }

  let metadata = fs::symlink_metadata(path)
    .map_err(|error| format!("symlink_metadata failed for {}: {error}", path.display()))?;

  if !metadata.file_type().is_dir() {
    return fs::remove_file(path)
      .map_err(|error| format!("remove_file failed for {}: {error}", path.display()));
  }

  let c_path = path_to_c_string(path);

  write_errno(0);
  // SAFETY: `c_path` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(c_path.as_ptr().cast::<c_char>()) };

  if dir.is_null() {
    return Err(format!(
      "opendir failed for {} with errno={}",
      path.display(),
      read_errno(),
    ));
  }

  loop {
    write_errno(0);
    // SAFETY: `dir` is a live handle returned by `opendir` above.
    let entry = unsafe { readdir(dir) };

    if entry.is_null() {
      let errno_value = read_errno();

      if errno_value != 0 {
        // SAFETY: `dir` is still owned by this function.
        let close_rc = unsafe { closedir(dir) };

        if close_rc != 0 {
          return Err(format!(
            "readdir/closedir failed for {} with errno={} then {}",
            path.display(),
            errno_value,
            read_errno(),
          ));
        }

        return Err(format!(
          "readdir failed for {} with errno={errno_value}",
          path.display(),
        ));
      }

      break;
    }

    // SAFETY: `entry` points to a valid `Dirent` produced by `readdir` for this stream.
    let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr().cast::<c_char>()) };
    let name_bytes = name.to_bytes();

    if name_bytes == b"." || name_bytes == b".." {
      continue;
    }

    let child = path.join(OsStr::from_bytes(name_bytes));

    remove_path_recursive(&child)?;
  }

  // SAFETY: `dir` is a live handle owned by this function.
  let close_rc = unsafe { closedir(dir) };

  if close_rc != 0 {
    return Err(format!(
      "closedir failed for {} with errno={}",
      path.display(),
      read_errno(),
    ));
  }

  fs::remove_dir(path).map_err(|error| format!("remove_dir failed for {}: {error}", path.display()))
}

fn collect_names(dir: *mut Dir) -> BTreeSet<String> {
  let mut names = BTreeSet::new();
  let mut iterations = 0usize;

  loop {
    iterations += 1;
    assert!(
      iterations <= 1024,
      "readdir did not reach end-of-stream after 1024 iterations",
    );

    // SAFETY: `dir` is a live `DIR*` opened by `opendir` in each call site.
    let entry = unsafe { readdir(dir) };

    if entry.is_null() {
      break;
    }

    // SAFETY: `readdir` contract returns a pointer to a valid dirent object for this stream.
    let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr().cast::<c_char>()) }
      .to_str()
      .expect("directory entry name must be valid UTF-8 in this test")
      .to_string();

    if name != "." && name != ".." {
      names.insert(name);
    }
  }

  names
}

fn open_directory(path: &Path) -> *mut Dir {
  let c_path = path_to_c_string(path);

  // SAFETY: `c_path` is a valid NUL-terminated path.
  let dir = unsafe { opendir(c_path.as_ptr().cast::<c_char>()) };

  assert!(!dir.is_null(), "opendir failed with errno={}", read_errno());

  dir
}

fn close_directory(dir: *mut Dir) {
  // SAFETY: `dir` was returned by `opendir` and not yet closed.
  let rc = unsafe { closedir(dir) };

  assert_eq!(rc, 0, "closedir failed with errno={}", read_errno());
}

#[test]
fn opendir_missing_path_returns_null_and_errno_enoent() {
  let temp_dir = TempDir::new("i036-missing");
  let missing = temp_dir.path().join("does-not-exist");
  let missing_c = path_to_c_string(&missing);

  write_errno(0);
  // SAFETY: `missing_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(missing_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_null_path_returns_null_and_errno_efault() {
  write_errno(0);
  // SAFETY: null path pointer is intentionally used to validate errno propagation.
  let dir = unsafe { opendir(core::ptr::null()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn opendir_empty_path_returns_null_and_errno_enoent() {
  let empty_path = CString::new("").expect("empty path CString construction must succeed");

  write_errno(0);
  // SAFETY: `empty_path` is a valid NUL-terminated C string.
  let dir = unsafe { opendir(empty_path.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_regular_file_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-notdir");
  let regular_file = temp_dir.path().join("regular.txt");

  fs::write(&regular_file, b"content").expect("failed to create regular file for opendir test");

  let regular_file_c = path_to_c_string(&regular_file);

  write_errno(0);
  // SAFETY: `regular_file_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(regular_file_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_regular_file_with_trailing_slash_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-notdir-slash");
  let regular_file = temp_dir.path().join("regular.txt");
  let mut regular_file_with_slash = regular_file.as_os_str().as_bytes().to_vec();

  fs::write(&regular_file, b"content").expect("failed to create regular file for opendir test");
  regular_file_with_slash.push(b'/');

  let regular_file_c =
    CString::new(regular_file_with_slash).expect("regular file path with slash must be valid");

  write_errno(0);
  // SAFETY: `regular_file_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(regular_file_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_regular_file_with_double_trailing_slash_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-notdir-double-slash");
  let regular_file = temp_dir.path().join("regular.txt");
  let mut regular_file_with_double_slash = regular_file.as_os_str().as_bytes().to_vec();

  fs::write(&regular_file, b"content").expect("failed to create regular file for opendir test");
  regular_file_with_double_slash.push(b'/');
  regular_file_with_double_slash.push(b'/');

  let regular_file_c = CString::new(regular_file_with_double_slash)
    .expect("regular file path with double slash must be valid");

  write_errno(0);
  // SAFETY: `regular_file_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(regular_file_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_regular_file_with_dot_suffix_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-notdir-dot");
  let regular_file = temp_dir.path().join("regular.txt");
  let mut regular_file_with_dot = regular_file.as_os_str().as_bytes().to_vec();

  fs::write(&regular_file, b"content").expect("failed to create regular file for opendir test");
  regular_file_with_dot.extend_from_slice(b"/.");

  let regular_file_c =
    CString::new(regular_file_with_dot).expect("regular file path with dot must be valid");

  write_errno(0);
  // SAFETY: `regular_file_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(regular_file_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_regular_file_with_dot_slash_suffix_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-notdir-dot-slash");
  let regular_file = temp_dir.path().join("regular.txt");
  let mut regular_file_with_dot_slash = regular_file.as_os_str().as_bytes().to_vec();

  fs::write(&regular_file, b"content").expect("failed to create regular file for opendir test");
  regular_file_with_dot_slash.extend_from_slice(b"/./");

  let regular_file_c = CString::new(regular_file_with_dot_slash)
    .expect("regular file path with dot-slash suffix must be valid");

  write_errno(0);
  // SAFETY: `regular_file_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(regular_file_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_success_returns_non_null_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-open-ok");
  let path_c = path_to_c_string(temp_dir.path());

  fs::create_dir(temp_dir.path().join("child")).expect("failed to create child directory");

  write_errno(4242);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(!dir.is_null(), "opendir failed with errno={}", read_errno());
  assert_eq!(
    read_errno(),
    4242,
    "successful opendir must not overwrite errno"
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_symlink_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-symlink");
  let target_dir = temp_dir.path().join("target");
  let symlink_path = temp_dir.path().join("target-link");

  fs::create_dir(&target_dir).expect("failed to create symlink target directory");
  symlink(&target_dir, &symlink_path).expect("failed to create directory symlink");

  let path_c = path_to_c_string(&symlink_path);

  write_errno(6060);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept symlink-to-directory path, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    6060,
    "successful opendir on symlink path must not overwrite errno"
  );

  close_directory(dir);
}

#[test]
fn opendir_broken_symlink_returns_null_and_errno_enoent() {
  let temp_dir = TempDir::new("i036-broken-symlink");
  let missing_target = temp_dir.path().join("missing-target");
  let symlink_path = temp_dir.path().join("missing-target-link");

  symlink(&missing_target, &symlink_path).expect("failed to create broken symlink");

  let path_c = path_to_c_string(&symlink_path);

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_broken_symlink_with_trailing_slash_returns_null_and_errno_enoent() {
  let temp_dir = TempDir::new("i036-broken-symlink-slash");
  let missing_target = temp_dir.path().join("missing-target");
  let symlink_path = temp_dir.path().join("missing-target-link");
  let mut path_with_slash = symlink_path.as_os_str().as_bytes().to_vec();

  symlink(&missing_target, &symlink_path).expect("failed to create broken symlink");

  path_with_slash.push(b'/');

  let path_c = CString::new(path_with_slash).expect("broken symlink path with slash must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_broken_symlink_with_double_trailing_slash_returns_null_and_errno_enoent() {
  let temp_dir = TempDir::new("i036-broken-symlink-double-slash");
  let missing_target = temp_dir.path().join("missing-target");
  let symlink_path = temp_dir.path().join("missing-target-link");
  let mut path_with_double_slash = symlink_path.as_os_str().as_bytes().to_vec();

  symlink(&missing_target, &symlink_path).expect("failed to create broken symlink");
  path_with_double_slash.push(b'/');
  path_with_double_slash.push(b'/');

  let path_c = CString::new(path_with_double_slash)
    .expect("broken symlink path with double slash must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_broken_symlink_with_dot_suffix_returns_null_and_errno_enoent() {
  let temp_dir = TempDir::new("i036-broken-symlink-dot");
  let missing_target = temp_dir.path().join("missing-target");
  let symlink_path = temp_dir.path().join("missing-target-link");
  let mut path_with_dot_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  symlink(&missing_target, &symlink_path).expect("failed to create broken symlink");
  path_with_dot_suffix.extend_from_slice(b"/.");

  let path_c =
    CString::new(path_with_dot_suffix).expect("broken symlink path with dot must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_broken_symlink_with_dot_slash_suffix_returns_null_and_errno_enoent() {
  let temp_dir = TempDir::new("i036-broken-symlink-dot-slash");
  let missing_target = temp_dir.path().join("missing-target");
  let symlink_path = temp_dir.path().join("missing-target-link");
  let mut path_with_dot_slash_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  symlink(&missing_target, &symlink_path).expect("failed to create broken symlink");
  path_with_dot_slash_suffix.extend_from_slice(b"/./");

  let path_c = CString::new(path_with_dot_slash_suffix)
    .expect("broken symlink path with dot-slash suffix must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_broken_symlink_with_dot_dot_suffix_returns_null_and_errno_enoent() {
  let temp_dir = TempDir::new("i036-broken-symlink-dot-dot");
  let missing_target = temp_dir.path().join("missing-target");
  let symlink_path = temp_dir.path().join("missing-target-link");
  let mut path_with_dot_dot_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  symlink(&missing_target, &symlink_path).expect("failed to create broken symlink");
  path_with_dot_dot_suffix.extend_from_slice(b"/..");

  let path_c = CString::new(path_with_dot_dot_suffix)
    .expect("broken symlink path with dot-dot suffix must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_broken_symlink_with_dot_dot_slash_suffix_returns_null_and_errno_enoent() {
  let temp_dir = TempDir::new("i036-broken-symlink-dot-dot-slash");
  let missing_target = temp_dir.path().join("missing-target");
  let symlink_path = temp_dir.path().join("missing-target-link");
  let mut path_with_dot_dot_slash_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  symlink(&missing_target, &symlink_path).expect("failed to create broken symlink");
  path_with_dot_dot_slash_suffix.extend_from_slice(b"/../");

  let path_c = CString::new(path_with_dot_dot_slash_suffix)
    .expect("broken symlink path with dot-dot-slash suffix must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn opendir_file_symlink_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-file-symlink");
  let target_file = temp_dir.path().join("target.txt");
  let symlink_path = temp_dir.path().join("target-file-link");

  fs::write(&target_file, b"data").expect("failed to create symlink target file");
  symlink(&target_file, &symlink_path).expect("failed to create file symlink");

  let path_c = path_to_c_string(&symlink_path);

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_file_symlink_with_trailing_slash_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-file-symlink-slash");
  let target_file = temp_dir.path().join("target.txt");
  let symlink_path = temp_dir.path().join("target-file-link");
  let mut path_with_slash = symlink_path.as_os_str().as_bytes().to_vec();

  fs::write(&target_file, b"data").expect("failed to create symlink target file");
  symlink(&target_file, &symlink_path).expect("failed to create file symlink");

  path_with_slash.push(b'/');

  let path_c = CString::new(path_with_slash).expect("file symlink path with slash must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_file_symlink_with_double_trailing_slash_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-file-symlink-double-slash");
  let target_file = temp_dir.path().join("target.txt");
  let symlink_path = temp_dir.path().join("target-file-link");
  let mut path_with_double_slash = symlink_path.as_os_str().as_bytes().to_vec();

  fs::write(&target_file, b"data").expect("failed to create symlink target file");
  symlink(&target_file, &symlink_path).expect("failed to create file symlink");

  path_with_double_slash.push(b'/');
  path_with_double_slash.push(b'/');

  let path_c = CString::new(path_with_double_slash)
    .expect("file symlink path with double slash must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_file_symlink_with_dot_suffix_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-file-symlink-dot");
  let target_file = temp_dir.path().join("target.txt");
  let symlink_path = temp_dir.path().join("target-file-link");
  let mut path_with_dot_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  fs::write(&target_file, b"data").expect("failed to create symlink target file");
  symlink(&target_file, &symlink_path).expect("failed to create file symlink");

  path_with_dot_suffix.extend_from_slice(b"/.");

  let path_c =
    CString::new(path_with_dot_suffix).expect("file symlink path with dot must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_file_symlink_with_dot_slash_suffix_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-file-symlink-dot-slash");
  let target_file = temp_dir.path().join("target.txt");
  let symlink_path = temp_dir.path().join("target-file-link");
  let mut path_with_dot_slash_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  fs::write(&target_file, b"data").expect("failed to create symlink target file");
  symlink(&target_file, &symlink_path).expect("failed to create file symlink");
  path_with_dot_slash_suffix.extend_from_slice(b"/./");

  let path_c = CString::new(path_with_dot_slash_suffix)
    .expect("file symlink path with dot-slash suffix must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_file_symlink_with_dot_dot_suffix_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-file-symlink-dot-dot");
  let target_file = temp_dir.path().join("target.txt");
  let symlink_path = temp_dir.path().join("target-file-link");
  let mut path_with_dot_dot_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  fs::write(&target_file, b"data").expect("failed to create symlink target file");
  symlink(&target_file, &symlink_path).expect("failed to create file symlink");
  path_with_dot_dot_suffix.extend_from_slice(b"/..");

  let path_c = CString::new(path_with_dot_dot_suffix)
    .expect("file symlink path with dot-dot suffix must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_file_symlink_with_dot_dot_slash_suffix_returns_null_and_errno_enotdir() {
  let temp_dir = TempDir::new("i036-file-symlink-dot-dot-slash");
  let target_file = temp_dir.path().join("target.txt");
  let symlink_path = temp_dir.path().join("target-file-link");
  let mut path_with_dot_dot_slash_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  fs::write(&target_file, b"data").expect("failed to create symlink target file");
  symlink(&target_file, &symlink_path).expect("failed to create file symlink");
  path_with_dot_dot_slash_suffix.extend_from_slice(b"/../");

  let path_c = CString::new(path_with_dot_dot_slash_suffix)
    .expect("file symlink path with dot-dot-slash suffix must be valid");

  write_errno(0);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(dir.is_null());
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn opendir_directory_with_trailing_slash_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-trailing-slash");
  let mut path_with_slash = temp_dir.path().as_os_str().as_bytes().to_vec();

  path_with_slash.push(b'/');

  let path_c = CString::new(path_with_slash).expect("path with trailing slash must be valid");

  write_errno(7878);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept directory paths with trailing slash, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    7878,
    "successful opendir with trailing slash must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_with_double_trailing_slash_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-double-trailing-slash");
  let mut path_with_double_slash = temp_dir.path().as_os_str().as_bytes().to_vec();

  path_with_double_slash.push(b'/');
  path_with_double_slash.push(b'/');

  let path_c =
    CString::new(path_with_double_slash).expect("path with double trailing slash must be valid");

  write_errno(8080);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept directory paths with repeated trailing slash, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    8080,
    "successful opendir with repeated trailing slash must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_with_dot_suffix_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-dot-suffix");
  let mut path_with_dot_suffix = temp_dir.path().as_os_str().as_bytes().to_vec();

  path_with_dot_suffix.extend_from_slice(b"/.");

  let path_c = CString::new(path_with_dot_suffix).expect("path with dot suffix must be valid");

  write_errno(8484);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept directory paths with '/.' suffix, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    8484,
    "successful opendir with '/.' suffix must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_with_dot_slash_suffix_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-dot-slash-suffix");
  let mut path_with_dot_slash_suffix = temp_dir.path().as_os_str().as_bytes().to_vec();

  path_with_dot_slash_suffix.extend_from_slice(b"/./");

  let path_c =
    CString::new(path_with_dot_slash_suffix).expect("path with dot-slash suffix must be valid");

  write_errno(8585);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept directory paths with '/./' suffix, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    8585,
    "successful opendir with '/./' suffix must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_with_dot_dot_suffix_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-dot-dot-suffix");
  let mut path_with_dot_dot_suffix = temp_dir.path().as_os_str().as_bytes().to_vec();

  path_with_dot_dot_suffix.extend_from_slice(b"/..");

  let path_c =
    CString::new(path_with_dot_dot_suffix).expect("path with dot-dot suffix must be valid");

  write_errno(8686);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept directory paths with '/..' suffix, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    8686,
    "successful opendir with '/..' suffix must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_with_dot_dot_slash_suffix_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-dot-dot-slash-suffix");
  let mut path_with_dot_dot_slash_suffix = temp_dir.path().as_os_str().as_bytes().to_vec();

  path_with_dot_dot_slash_suffix.extend_from_slice(b"/../");

  let path_c = CString::new(path_with_dot_dot_slash_suffix)
    .expect("path with dot-dot-slash suffix must be valid");

  write_errno(8787);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept directory paths with '/../' suffix, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    8787,
    "successful opendir with '/../' suffix must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_symlink_with_trailing_slash_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-symlink-trailing-slash");
  let target_dir = temp_dir.path().join("target");
  let symlink_path = temp_dir.path().join("target-link");
  let mut path_with_slash = symlink_path.as_os_str().as_bytes().to_vec();

  fs::create_dir(&target_dir).expect("failed to create symlink target directory");
  symlink(&target_dir, &symlink_path).expect("failed to create directory symlink");

  path_with_slash.push(b'/');

  let path_c = CString::new(path_with_slash).expect("symlink path with slash must be valid");

  write_errno(9191);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept symlink-to-directory path with trailing slash, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    9191,
    "successful opendir on symlink path with slash must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_symlink_with_double_trailing_slash_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-symlink-double-trailing-slash");
  let target_dir = temp_dir.path().join("target");
  let symlink_path = temp_dir.path().join("target-link");
  let mut path_with_double_slash = symlink_path.as_os_str().as_bytes().to_vec();

  fs::create_dir(&target_dir).expect("failed to create symlink target directory");
  symlink(&target_dir, &symlink_path).expect("failed to create directory symlink");

  path_with_double_slash.push(b'/');
  path_with_double_slash.push(b'/');

  let path_c =
    CString::new(path_with_double_slash).expect("symlink path with double slash must be valid");

  write_errno(9393);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept symlink-to-directory path with repeated trailing slash, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    9393,
    "successful opendir on symlink path with repeated trailing slash must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_symlink_with_dot_suffix_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-symlink-dot-suffix");
  let target_dir = temp_dir.path().join("target");
  let symlink_path = temp_dir.path().join("target-link");
  let mut path_with_dot_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  fs::create_dir(&target_dir).expect("failed to create symlink target directory");
  symlink(&target_dir, &symlink_path).expect("failed to create directory symlink");

  path_with_dot_suffix.extend_from_slice(b"/.");

  let path_c =
    CString::new(path_with_dot_suffix).expect("symlink path with dot suffix must be valid");

  write_errno(9595);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept symlink-to-directory path with '/.' suffix, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    9595,
    "successful opendir on symlink path with '/.' suffix must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_symlink_with_dot_slash_suffix_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-symlink-dot-slash-suffix");
  let target_dir = temp_dir.path().join("target");
  let symlink_path = temp_dir.path().join("target-link");
  let mut path_with_dot_slash_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  fs::create_dir(&target_dir).expect("failed to create symlink target directory");
  symlink(&target_dir, &symlink_path).expect("failed to create directory symlink");

  path_with_dot_slash_suffix.extend_from_slice(b"/./");

  let path_c = CString::new(path_with_dot_slash_suffix)
    .expect("symlink path with dot-slash suffix must be valid");

  write_errno(9696);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept symlink-to-directory path with '/./' suffix, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    9696,
    "successful opendir on symlink path with '/./' suffix must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_symlink_with_dot_dot_suffix_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-symlink-dot-dot-suffix");
  let target_dir = temp_dir.path().join("target");
  let symlink_path = temp_dir.path().join("target-link");
  let mut path_with_dot_dot_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  fs::create_dir(&target_dir).expect("failed to create symlink target directory");
  symlink(&target_dir, &symlink_path).expect("failed to create directory symlink");

  path_with_dot_dot_suffix.extend_from_slice(b"/..");

  let path_c =
    CString::new(path_with_dot_dot_suffix).expect("symlink path with dot-dot suffix must be valid");

  write_errno(9797);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept symlink-to-directory path with '/..' suffix, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    9797,
    "successful opendir on symlink path with '/..' suffix must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn opendir_directory_symlink_with_dot_dot_slash_suffix_succeeds_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-symlink-dot-dot-slash-suffix");
  let target_dir = temp_dir.path().join("target");
  let symlink_path = temp_dir.path().join("target-link");
  let mut path_with_dot_dot_slash_suffix = symlink_path.as_os_str().as_bytes().to_vec();

  fs::create_dir(&target_dir).expect("failed to create symlink target directory");
  symlink(&target_dir, &symlink_path).expect("failed to create directory symlink");

  path_with_dot_dot_slash_suffix.extend_from_slice(b"/../");

  let path_c = CString::new(path_with_dot_dot_slash_suffix)
    .expect("symlink path with dot-dot-slash suffix must be valid");

  write_errno(9898);
  // SAFETY: `path_c` is a valid NUL-terminated path string.
  let dir = unsafe { opendir(path_c.as_ptr().cast::<c_char>()) };

  assert!(
    !dir.is_null(),
    "opendir should accept symlink-to-directory path with '/../' suffix, errno={}",
    read_errno(),
  );
  assert_eq!(
    read_errno(),
    9898,
    "successful opendir on symlink path with '/../' suffix must not overwrite errno",
  );

  close_directory(dir);
}

#[test]
fn readdir_returns_entries_and_end_of_stream_keeps_errno() {
  let temp_dir = TempDir::new("i036-readdir");
  let first = "alpha.txt";
  let second = "beta.txt";

  fs::write(temp_dir.path().join(first), b"alpha").expect("failed to write alpha test entry");
  fs::create_dir(temp_dir.path().join(second)).expect("failed to create beta test entry");

  let dir = open_directory(temp_dir.path());

  write_errno(2468);

  let names = collect_names(dir);

  assert!(names.contains(first), "entry set did not include {first}");
  assert!(names.contains(second), "entry set did not include {second}");
  assert_eq!(read_errno(), 2468, "end-of-stream must not overwrite errno");

  close_directory(dir);
}

#[test]
fn readdir_empty_directory_yields_no_user_entries_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-empty-readdir");
  let dir = open_directory(temp_dir.path());

  write_errno(7777);

  let names = collect_names(dir);

  assert!(
    names.is_empty(),
    "empty directory must not report regular entries"
  );
  assert_eq!(
    read_errno(),
    7777,
    "readdir end-of-stream on empty directory must not overwrite errno"
  );

  write_errno(8888);

  // SAFETY: `dir` is still a live stream and repeated EOF reads are valid.
  let second = unsafe { readdir(dir) };

  assert!(
    second.is_null(),
    "repeated readdir call at EOF must remain end-of-stream",
  );
  assert_eq!(
    read_errno(),
    8888,
    "repeated EOF reads must continue preserving caller errno"
  );

  close_directory(dir);
}

#[test]
fn readdir_preserves_255_byte_entry_name() {
  let temp_dir = TempDir::new("i036-long-name");
  let long_name = "a".repeat(255);

  fs::write(temp_dir.path().join(&long_name), b"payload").expect("failed to write long-name entry");

  let dir = open_directory(temp_dir.path());
  let names = collect_names(dir);

  assert!(
    names.contains(&long_name),
    "readdir result did not preserve full 255-byte entry name",
  );

  close_directory(dir);
}

#[test]
fn readdir_reuses_stream_owned_entry_pointer_across_calls() {
  let temp_dir = TempDir::new("i036-readdir-ptr");

  fs::write(temp_dir.path().join("entry"), b"value")
    .expect("failed to create directory entry for pointer reuse test");

  let dir = open_directory(temp_dir.path());

  write_errno(0);
  // SAFETY: `dir` is a live stream returned by `opendir`.
  let first = unsafe { readdir(dir) };

  assert!(
    !first.is_null(),
    "first readdir call should produce at least one entry",
  );

  write_errno(0);
  // SAFETY: `dir` remains a live stream and second read is valid.
  let second = unsafe { readdir(dir) };

  assert!(
    !second.is_null(),
    "second readdir call should still produce a stream entry",
  );

  assert_eq!(
    first, second,
    "same DIR* stream must reuse stream-owned dirent storage pointer",
  );
  assert_eq!(
    read_errno(),
    0,
    "successful readdir calls must not set errno",
  );

  close_directory(dir);
}

#[test]
fn readdir_separate_streams_use_distinct_entry_storage() {
  let temp_dir = TempDir::new("i036-readdir-separate-streams");

  fs::write(temp_dir.path().join("entry"), b"value")
    .expect("failed to create directory entry for separate-stream test");

  let first_stream = open_directory(temp_dir.path());
  let second_stream = open_directory(temp_dir.path());

  write_errno(0);

  // SAFETY: `first_stream` is a live stream returned by `opendir`.
  let first_entry = unsafe { readdir(first_stream) };

  write_errno(0);

  // SAFETY: `second_stream` is another live stream returned by `opendir`.
  let second_entry = unsafe { readdir(second_stream) };

  assert!(
    !first_entry.is_null(),
    "first stream must produce at least one entry",
  );
  assert!(
    !second_entry.is_null(),
    "second stream must produce at least one entry",
  );
  assert_ne!(
    first_entry, second_entry,
    "different DIR* handles must not alias the same dirent storage",
  );
  assert_eq!(
    read_errno(),
    0,
    "successful readdir calls across streams must not set errno",
  );

  close_directory(first_stream);
  close_directory(second_stream);
}

#[test]
fn i036_readdir_null_pointer_returns_null_and_errno_einval() {
  write_errno(0);
  // SAFETY: null pointer is intentionally passed to validate error handling.
  let entry = unsafe { readdir(core::ptr::null_mut()) };

  assert!(entry.is_null());
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn rewinddir_restarts_iteration_from_beginning() {
  let temp_dir = TempDir::new("i036-rewind");

  fs::write(temp_dir.path().join("one"), b"1").expect("failed to create entry one");
  fs::write(temp_dir.path().join("two"), b"2").expect("failed to create entry two");

  let dir = open_directory(temp_dir.path());
  let first_pass = collect_names(dir);

  write_errno(8642);
  // SAFETY: `dir` is a live stream returned from `opendir`.
  unsafe { rewinddir(dir) };

  let second_pass = collect_names(dir);

  assert_eq!(first_pass, second_pass);
  assert_eq!(
    read_errno(),
    8642,
    "successful rewinddir must not overwrite errno"
  );

  close_directory(dir);
}

#[test]
fn rewinddir_empty_directory_preserves_errno_and_allows_repeat_eof() {
  let temp_dir = TempDir::new("i036-rewind-empty");
  let dir = open_directory(temp_dir.path());

  write_errno(2222);

  let first_pass = collect_names(dir);

  assert!(
    first_pass.is_empty(),
    "empty directory should expose no user-visible entries",
  );
  assert_eq!(
    read_errno(),
    2222,
    "empty-directory EOF before rewind must preserve errno",
  );

  write_errno(3333);
  // SAFETY: `dir` is a live stream returned by `opendir`.
  unsafe { rewinddir(dir) };

  let second_pass = collect_names(dir);

  assert!(
    second_pass.is_empty(),
    "rewinddir on empty directory should keep user-visible set empty",
  );
  assert_eq!(
    read_errno(),
    3333,
    "successful rewinddir + EOF re-read must preserve errno",
  );

  close_directory(dir);
}

#[test]
fn i036_rewinddir_null_pointer_sets_errno_einval() {
  write_errno(0);
  // SAFETY: null pointer is intentionally passed to validate error handling.
  unsafe { rewinddir(core::ptr::null_mut()) };

  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn closedir_null_pointer_returns_minus_one_and_errno_einval() {
  write_errno(0);
  // SAFETY: null pointer is intentionally passed to validate error handling.
  let rc = unsafe { closedir(core::ptr::null_mut()) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn closedir_success_returns_zero_and_keeps_errno() {
  let temp_dir = TempDir::new("i036-close");

  File::create(temp_dir.path().join("entry")).expect("failed to create close test entry");

  let dir = open_directory(temp_dir.path());

  write_errno(1357);
  // SAFETY: `dir` is a live stream returned by `opendir`.
  let rc = unsafe { closedir(dir) };

  assert_eq!(rc, 0);
  assert_eq!(
    read_errno(),
    1357,
    "successful closedir must not overwrite errno"
  );
}

#[test]
fn remove_path_recursive_removes_nested_directory_tree() {
  let temp_dir = TempDir::new("i036-cleanup");
  let nested_dir = temp_dir.path().join("nested");
  let leaf_file = nested_dir.join("leaf.txt");

  fs::create_dir_all(&nested_dir).expect("failed to create nested directory for cleanup test");
  fs::write(&leaf_file, b"cleanup").expect("failed to create nested file for cleanup test");

  remove_path_recursive(temp_dir.path())
    .expect("remove_path_recursive must remove nested directory trees");
  assert!(
    !temp_dir.path().exists(),
    "cleanup helper should remove the root test directory",
  );
}
