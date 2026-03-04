//! Minimal `glob(3)` implementation.
//!
//! This module provides the C ABI entry points:
//! - `glob`
//! - `globfree`
//!
//! Supported pattern operators in this phase:
//! - `*`
//! - `?`
//! - bracket expressions `[]`
//!
//! Behavior intentionally stays minimal for I037 and focuses on predictable
//! memory ownership with `globfree`.

use crate::abi::errno::{EACCES, EINVAL, ENOMEM};
use crate::abi::types::{c_char, c_int, size_t};
use crate::dirent::{Dirent, closedir, opendir, readdir};
use crate::errno::{__errno_location, set_errno};
use core::ptr;
use std::ffi::{CStr, CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const PATH_SEPARATOR: u8 = b'/';
/// `glob` flag: abort immediately on directory read errors.
pub const GLOB_ERR: c_int = 0x0001;
/// `glob` flag: do not sort matched paths.
pub const GLOB_NOSORT: c_int = 0x0004;
/// `glob` flag: honor `gl_offs` as leading null entries in `gl_pathv`.
pub const GLOB_DOOFFS: c_int = 0x0008;
/// `glob` flag: return input pattern when nothing matches.
pub const GLOB_NOCHECK: c_int = 0x0010;
/// `glob` flag: append results to an existing `glob_t` buffer.
pub const GLOB_APPEND: c_int = 0x0020;
/// `glob` flag: disable backslash escaping.
pub const GLOB_NOESCAPE: c_int = 0x0040;
/// `glob` result code: memory allocation failed.
pub const GLOB_NOSPACE: c_int = 1;
/// `glob` result code: traversal aborted due to callback or `GLOB_ERR`.
pub const GLOB_ABORTED: c_int = 2;
/// `glob` result code: no path matched.
pub const GLOB_NOMATCH: c_int = 3;

/// C callback type used by `glob` for traversal-error reporting.
///
/// Callback contract:
/// - first argument: path that triggered the traversal error
/// - second argument: Linux errno value associated with that error
/// - return non-zero to stop traversal (`glob` returns `GLOB_ABORTED`)
pub type GlobErrorFn = Option<unsafe extern "C" fn(*const c_char, c_int) -> c_int>;

#[derive(Clone)]
struct CandidatePath {
  fs_path: PathBuf,
  result_path: Vec<u8>,
}

/// Minimal `glob_t` storage used by this implementation.
///
/// Contract:
/// - `gl_pathc` stores the number of matched path strings.
/// - `gl_pathv` points to a null-terminated vector of C strings.
/// - `gl_offs` stores the requested leading null slots when `GLOB_DOOFFS` is used.
/// - Every successful `glob` allocation can be released with `globfree`.
#[derive(Debug)]
#[repr(C)]
pub struct Glob {
  /// Number of matched path entries.
  pub gl_pathc: size_t,
  /// Pointer to a null-terminated array of matched path pointers.
  pub gl_pathv: *mut *mut c_char,
  /// Number of leading null slots reserved in `gl_pathv`.
  pub gl_offs: size_t,
  /// Flags used for the last successful call.
  pub gl_flags: c_int,
}

fn usize_from_size_t(value: size_t) -> usize {
  usize::try_from(value).unwrap_or_else(|_| unreachable!("size_t must fit usize on x86_64 Linux"))
}

fn size_t_from_usize(value: usize) -> size_t {
  size_t::try_from(value).unwrap_or_else(|_| unreachable!("usize must fit size_t on x86_64 Linux"))
}

fn segment_has_pattern_meta(segment: &[u8], flags: c_int) -> bool {
  let no_escape = flags & GLOB_NOESCAPE != 0;
  let mut index = 0_usize;

  while index < segment.len() {
    let current = segment[index];

    if current == b'\\' && !no_escape && index + 1 < segment.len() {
      index += 2;

      continue;
    }

    if matches!(current, b'*' | b'?') {
      return true;
    }

    if current == b'[' && bracket_match(segment, index, b'.', no_escape).is_some() {
      return true;
    }

    index += 1;
  }

  false
}

fn join_result_path(base: &[u8], separator_count: usize, name: &[u8]) -> Vec<u8> {
  let mut joined = Vec::with_capacity(base.len() + separator_count + name.len());

  joined.extend_from_slice(base);
  joined.extend(std::iter::repeat_n(PATH_SEPARATOR, separator_count));

  joined.extend_from_slice(name);

  joined
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns writable thread-local storage.
  unsafe { *__errno_location() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local storage.
  unsafe {
    *__errno_location() = value;
  }
}

fn dirent_name_bytes(entry: *const Dirent) -> Vec<u8> {
  // SAFETY: `entry` originates from `readdir` and remains valid until the
  // next `readdir` call on the same handle.
  let entry_ref = unsafe { &*entry };
  // SAFETY: `readdir` guarantees NUL-terminated `d_name`.
  let name = unsafe { CStr::from_ptr(entry_ref.d_name.as_ptr()) };

  name.to_bytes().to_vec()
}

fn read_directory_names(path: &Path) -> Result<Vec<Vec<u8>>, c_int> {
  let path_c = CString::new(path.as_os_str().as_bytes()).map_err(|_| EINVAL)?;
  // SAFETY: `path_c` is a valid NUL-terminated C string.
  let dir = unsafe { opendir(path_c.as_ptr()) };

  if dir.is_null() {
    let errno_value = read_errno();

    return Err(if errno_value == 0 {
      EACCES
    } else {
      errno_value
    });
  }

  let mut names = Vec::new();
  let mut read_result: Result<(), c_int> = Ok(());

  loop {
    write_errno(0);
    // SAFETY: `dir` stays valid until `closedir`.
    let entry = unsafe { readdir(dir) };

    if entry.is_null() {
      let errno_value = read_errno();

      if errno_value != 0 {
        read_result = Err(errno_value);
      }

      break;
    }

    let name_bytes = dirent_name_bytes(entry);

    names.push(name_bytes);
  }

  // SAFETY: `dir` comes from `opendir`.
  let close_result = unsafe { closedir(dir) };

  if close_result != 0 && read_result.is_ok() {
    let errno_value = read_errno();

    read_result = Err(if errno_value == 0 {
      EACCES
    } else {
      errno_value
    });
  }

  read_result.map(|()| names)
}

fn should_match_hidden_name(segment_pattern: &[u8], flags: c_int) -> bool {
  if segment_pattern.first().is_some_and(|byte| *byte == b'.') {
    return true;
  }

  let no_escape = flags & GLOB_NOESCAPE != 0;

  if segment_pattern.first().is_some_and(|byte| *byte == b'[')
    && !segment_pattern
      .get(1)
      .is_some_and(|byte| *byte == b'!' || *byte == b'^')
    && let Some((matches_dot, _next_index)) = bracket_match(segment_pattern, 0, b'.', no_escape)
    && matches_dot
  {
    return true;
  }

  if flags & GLOB_NOESCAPE != 0 {
    return false;
  }

  segment_pattern.len() >= 2 && segment_pattern[0] == b'\\' && segment_pattern[1] == b'.'
}

fn unescape_segment(segment_pattern: &[u8], flags: c_int) -> Vec<u8> {
  if flags & GLOB_NOESCAPE != 0 {
    return segment_pattern.to_vec();
  }

  let mut unescaped = Vec::with_capacity(segment_pattern.len());
  let mut index = 0_usize;

  while index < segment_pattern.len() {
    if segment_pattern[index] == b'\\' && index + 1 < segment_pattern.len() {
      unescaped.push(segment_pattern[index + 1]);
      index += 2;

      continue;
    }

    unescaped.push(segment_pattern[index]);
    index += 1;
  }

  unescaped
}

fn normalize_escaped_separators(pattern: &[u8]) -> Vec<u8> {
  let mut normalized = Vec::with_capacity(pattern.len());
  let mut index = 0_usize;

  while index < pattern.len() {
    if pattern[index] == b'\\' && index + 1 < pattern.len() && pattern[index + 1] == PATH_SEPARATOR
    {
      normalized.push(PATH_SEPARATOR);
      index += 2;

      continue;
    }

    normalized.push(pattern[index]);
    index += 1;
  }

  normalized
}

fn parse_segments_with_separators(pattern: &[u8]) -> (usize, Vec<&[u8]>, Vec<usize>, usize) {
  let mut leading_separators = 0_usize;

  while leading_separators < pattern.len() && pattern[leading_separators] == PATH_SEPARATOR {
    leading_separators += 1;
  }

  let mut segments = Vec::new();
  let mut separators_before = Vec::new();
  let mut pending_separators = 0_usize;
  let mut index = leading_separators;

  while index < pattern.len() {
    let start = index;

    while index < pattern.len() && pattern[index] != PATH_SEPARATOR {
      index += 1;
    }

    if start < index {
      segments.push(&pattern[start..index]);
      separators_before.push(pending_separators);
      pending_separators = 0;
    }

    while index < pattern.len() && pattern[index] == PATH_SEPARATOR {
      pending_separators += 1;
      index += 1;
    }
  }

  (
    leading_separators,
    segments,
    separators_before,
    pending_separators,
  )
}

fn leading_separators_include_escaped(pattern: &[u8], flags: c_int) -> bool {
  if flags & GLOB_NOESCAPE != 0 {
    return false;
  }

  let mut index = 0_usize;
  let mut found_escaped_separator = false;

  while index < pattern.len() {
    if pattern[index] == PATH_SEPARATOR {
      index += 1;

      continue;
    }

    if pattern[index] == b'\\' && index + 1 < pattern.len() && pattern[index + 1] == PATH_SEPARATOR
    {
      found_escaped_separator = true;
      index += 2;

      continue;
    }

    break;
  }

  found_escaped_separator
}

fn root_only_result_path(
  leading_separators: usize,
  leading_has_escaped_separator: bool,
) -> Vec<u8> {
  let slash_count =
    if leading_separators >= 3 || (leading_separators == 2 && leading_has_escaped_separator) {
      2
    } else {
      1
    };
  let mut path = Vec::with_capacity(slash_count);

  path.extend(std::iter::repeat_n(PATH_SEPARATOR, slash_count));

  path
}

fn separator_is_escaped(pattern: &[u8], separator_index: usize, flags: c_int) -> bool {
  if flags & GLOB_NOESCAPE != 0
    || separator_index == 0
    || pattern[separator_index] != PATH_SEPARATOR
  {
    return false;
  }

  let mut backslash_count = 0_usize;
  let mut index = separator_index;

  while index > 0 && pattern[index - 1] == b'\\' {
    backslash_count += 1;
    index -= 1;
  }

  backslash_count % 2 == 1
}

fn nocheck_fallback_pattern(pattern: &[u8], flags: c_int) -> Vec<u8> {
  let mut end = pattern.len();

  while end > 0
    && pattern[end - 1] == PATH_SEPARATOR
    && !separator_is_escaped(pattern, end - 1, flags)
  {
    end -= 1;
  }

  let trimmed = if end == 0 { pattern } else { &pattern[..end] };
  let mut normalized = trimmed.to_vec();

  if normalized.starts_with(b"//")
    && normalized.len() > 2
    && !normalized[2..].contains(&PATH_SEPARATOR)
  {
    normalized.remove(0);
  }

  normalized
}

fn bracket_match(
  pattern: &[u8],
  bracket_index: usize,
  candidate: u8,
  no_escape: bool,
) -> Option<(bool, usize)> {
  let mut index = bracket_index + 1;

  if index >= pattern.len() {
    return None;
  }

  let mut invert = false;

  if pattern[index] == b'!' || pattern[index] == b'^' {
    invert = true;
    index += 1;
  }

  let mut matched = false;
  let mut has_any_item = false;

  if index < pattern.len() && pattern[index] == b']' {
    has_any_item = true;
    matched |= candidate == b']';
    index += 1;
  }

  while index < pattern.len() {
    let current = pattern[index];

    if current == b']' && has_any_item {
      let final_match = if invert { !matched } else { matched };

      return Some((final_match, index + 1));
    }

    let (literal, literal_escaped) = if current == b'\\' && !no_escape && index + 1 < pattern.len()
    {
      (pattern[index + 1], true)
    } else {
      (current, false)
    };

    if !literal_escaped
      && index + 1 < pattern.len()
      && pattern[index + 1] == b'-'
      && index + 2 < pattern.len()
      && pattern[index + 2] != b']'
    {
      let (range_end, consumed) =
        if pattern[index + 2] == b'\\' && !no_escape && index + 3 < pattern.len() {
          (pattern[index + 3], 4_usize)
        } else {
          (pattern[index + 2], 3_usize)
        };

      if literal <= range_end && (literal..=range_end).contains(&candidate) {
        matched = true;
      }

      has_any_item = true;
      index += consumed;

      continue;
    }

    if candidate == literal {
      matched = true;
    }

    has_any_item = true;
    index += if literal_escaped { 2 } else { 1 };
  }

  None
}

fn pattern_matches_from(
  pattern: &[u8],
  mut pattern_index: usize,
  candidate: &[u8],
  mut candidate_index: usize,
  flags: c_int,
) -> bool {
  let no_escape = flags & GLOB_NOESCAPE != 0;

  while pattern_index < pattern.len() {
    let current = pattern[pattern_index];

    if current == b'*' {
      while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
      }

      if pattern_index == pattern.len() {
        return true;
      }

      let mut scan_index = candidate_index;

      while scan_index <= candidate.len() {
        if pattern_matches_from(pattern, pattern_index, candidate, scan_index, flags) {
          return true;
        }

        scan_index += 1;
      }

      return false;
    }

    if candidate_index >= candidate.len() {
      return false;
    }

    if current == b'?' {
      pattern_index += 1;
      candidate_index += 1;

      continue;
    }

    if current == b'[' {
      let Some((matched, next_index)) = bracket_match(
        pattern,
        pattern_index,
        candidate[candidate_index],
        no_escape,
      ) else {
        if candidate[candidate_index] != b'[' {
          return false;
        }

        pattern_index += 1;
        candidate_index += 1;

        continue;
      };

      if !matched {
        return false;
      }

      pattern_index = next_index;
      candidate_index += 1;

      continue;
    }

    if current == b'\\' && !no_escape {
      if pattern_index + 1 >= pattern.len() {
        if candidate[candidate_index] != b'\\' {
          return false;
        }

        pattern_index += 1;
        candidate_index += 1;

        continue;
      }

      let escaped = pattern[pattern_index + 1];

      if candidate[candidate_index] != escaped {
        return false;
      }

      pattern_index += 2;
      candidate_index += 1;

      continue;
    }

    if candidate[candidate_index] != current {
      return false;
    }

    pattern_index += 1;
    candidate_index += 1;
  }

  candidate_index == candidate.len()
}

fn pattern_matches(segment_pattern: &[u8], candidate_name: &[u8], flags: c_int) -> bool {
  pattern_matches_from(segment_pattern, 0, candidate_name, 0, flags)
}

fn callback_path(candidate: &CandidatePath) -> Vec<u8> {
  if candidate.result_path.is_empty() {
    return b".".to_vec();
  }

  candidate.result_path.clone()
}

fn process_directory_error(
  candidate: &CandidatePath,
  errno_value: c_int,
  flags: c_int,
  errfunc: GlobErrorFn,
) -> Result<(), c_int> {
  if let Some(callback) = errfunc {
    let callback_arg = CString::new(callback_path(candidate)).map_err(|_| GLOB_ABORTED)?;

    // SAFETY: Callback ABI is provided by caller and invoked with a temporary
    // NUL-terminated path pointer and Linux errno integer value.
    let callback_result = unsafe { callback(callback_arg.as_ptr(), errno_value) };

    if callback_result != 0 {
      set_errno(errno_value);

      return Err(GLOB_ABORTED);
    }
  }

  if flags & GLOB_ERR != 0 {
    set_errno(errno_value);

    return Err(GLOB_ABORTED);
  }

  Ok(())
}

fn collect_matches(
  pattern: &[u8],
  flags: c_int,
  errfunc: GlobErrorFn,
) -> Result<Vec<Vec<u8>>, c_int> {
  let leading_has_escaped_separator = leading_separators_include_escaped(pattern, flags);
  let normalized_pattern_storage =
    if flags & GLOB_NOESCAPE == 0 && pattern.windows(2).any(|window| window == b"\\/") {
      Some(normalize_escaped_separators(pattern))
    } else {
      None
    };
  let normalized_pattern = normalized_pattern_storage.as_deref().unwrap_or(pattern);
  let (leading_separators, segments, separators_before, trailing_separators) =
    parse_segments_with_separators(normalized_pattern);
  let is_absolute = leading_separators > 0;
  let has_trailing_separator = trailing_separators > 0;

  if is_absolute && segments.is_empty() {
    return Ok(vec![root_only_result_path(
      leading_separators,
      leading_has_escaped_separator,
    )]);
  }

  if segments.is_empty() {
    return Ok(Vec::new());
  }

  let mut candidates = if is_absolute {
    let root_prefix_count = if leading_separators == 2 && segments.len() == 1 {
      1
    } else {
      leading_separators
    };
    let mut root_prefix = Vec::with_capacity(root_prefix_count);

    root_prefix.extend(std::iter::repeat_n(PATH_SEPARATOR, root_prefix_count));

    vec![CandidatePath {
      fs_path: PathBuf::from("/"),
      result_path: root_prefix,
    }]
  } else {
    vec![CandidatePath {
      fs_path: PathBuf::from("."),
      result_path: Vec::new(),
    }]
  };

  for (segment_index, segment) in segments.iter().enumerate() {
    let mut next_candidates = Vec::new();
    let has_meta = segment_has_pattern_meta(segment, flags);
    let include_hidden = should_match_hidden_name(segment, flags);
    let separator_count = if segment_index == 0 {
      0
    } else {
      separators_before[segment_index]
    };

    for candidate in &candidates {
      if has_meta {
        let directory_entries = match read_directory_names(&candidate.fs_path) {
          Ok(entries) => entries,
          Err(errno_value) => {
            process_directory_error(candidate, errno_value, flags, errfunc)?;

            continue;
          }
        };

        for name_bytes in directory_entries {
          if name_bytes.first().is_some_and(|byte| *byte == b'.') && !include_hidden {
            continue;
          }

          if !pattern_matches(segment, &name_bytes, flags) {
            continue;
          }

          next_candidates.push(CandidatePath {
            fs_path: candidate.fs_path.join(OsStr::from_bytes(&name_bytes)),
            result_path: join_result_path(&candidate.result_path, separator_count, &name_bytes),
          });
        }

        continue;
      }

      let literal_segment = unescape_segment(segment, flags);
      let literal_name = OsStr::from_bytes(&literal_segment);
      let next_path = candidate.fs_path.join(literal_name);

      if !next_path.exists() {
        continue;
      }

      next_candidates.push(CandidatePath {
        fs_path: next_path,
        result_path: join_result_path(&candidate.result_path, separator_count, &literal_segment),
      });
    }

    candidates = next_candidates;

    if candidates.is_empty() {
      break;
    }
  }

  if has_trailing_separator {
    candidates.retain(|candidate| candidate.fs_path.is_dir());

    for candidate in &mut candidates {
      candidate.result_path.push(PATH_SEPARATOR);
    }
  }

  Ok(
    candidates
      .into_iter()
      .map(|path| path.result_path)
      .collect(),
  )
}

unsafe fn clone_existing_paths(pglob: *const Glob, offsets: usize) -> Vec<Vec<u8>> {
  // SAFETY: Caller guarantees `pglob` points to a valid `Glob`.
  let path_count = unsafe { usize_from_size_t((*pglob).gl_pathc) };
  // SAFETY: Caller guarantees `pglob` points to a valid `Glob`.
  let pathv = unsafe { (*pglob).gl_pathv };

  if path_count == 0 || pathv.is_null() {
    return Vec::new();
  }

  let mut cloned = Vec::with_capacity(path_count);
  let mut index = 0_usize;

  while index < path_count {
    // SAFETY: `glob`/`globfree` store a valid null-terminated path vector with
    // `offsets + path_count + 1` entries.
    let entry = unsafe { pathv.add(offsets + index).read() };

    if entry.is_null() {
      break;
    }

    // SAFETY: `entry` points to a NUL-terminated string allocated by `glob`.
    let bytes = unsafe { CStr::from_ptr(entry).to_bytes() };

    cloned.push(bytes.to_vec());
    index += 1;
  }

  cloned
}

fn allocate_pathv(paths: &[Vec<u8>], offsets: usize) -> Result<*mut *mut c_char, c_int> {
  let total_entries = offsets + paths.len() + 1;
  let mut path_vector = vec![ptr::null_mut(); total_entries];

  for (index, path) in paths.iter().enumerate() {
    let c_string = CString::new(path.as_slice()).map_err(|_| ENOMEM)?;

    path_vector[offsets + index] = c_string.into_raw();
  }

  let boxed_slice = path_vector.into_boxed_slice();
  let leaked = Box::into_raw(boxed_slice);

  Ok(leaked.cast())
}

unsafe fn free_pathv(pathv: *mut *mut c_char, path_count: usize, offsets: usize) {
  if pathv.is_null() {
    return;
  }

  let mut index = 0_usize;

  while index < path_count {
    // SAFETY: Caller provides a pointer produced by `allocate_pathv`, so index
    // arithmetic within `offsets + path_count` is valid.
    let entry = unsafe { pathv.add(offsets + index).read() };

    if !entry.is_null() {
      // SAFETY: Each non-null entry is owned by this `glob_t` allocation.
      let _owned_entry = unsafe { CString::from_raw(entry) };
    }

    index += 1;
  }

  let total_entries = offsets + path_count + 1;
  // SAFETY: `pathv` originates from `Box<[...]>` with `total_entries` length.
  let vector_slice = ptr::slice_from_raw_parts_mut(pathv, total_entries);
  // SAFETY: Reconstructs and drops the original boxed allocation exactly once.
  let _owned_vector = unsafe { Box::from_raw(vector_slice) };
}

/// C ABI entry point for `glob`.
///
/// Runs pathname expansion using minimal wildcard support (`*`, `?`, `[]`).
///
/// Returns:
/// - `0` on success
/// - `GLOB_NOMATCH` when nothing matches and `GLOB_NOCHECK` is not set
/// - `GLOB_ABORTED` when traversal aborts via callback or `GLOB_ERR`
/// - `GLOB_NOSPACE` when allocation fails
///
/// `errno` contract:
/// - `EINVAL` for null `pattern`/`pglob`
/// - `ENOMEM` when result allocation fails
/// - underlying directory errno (for example `ENOTDIR`, `EACCES`) when
///   returning `GLOB_ABORTED` due to callback stop or `GLOB_ERR`
///
/// # Safety
/// - `pattern` must point to a valid NUL-terminated C string.
/// - `pglob` must point to writable `Glob` storage.
/// - If provided, `errfunc` must follow the C callback ABI contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn glob(
  pattern: *const c_char,
  flags: c_int,
  errfunc: GlobErrorFn,
  pglob: *mut Glob,
) -> c_int {
  if pattern.is_null() || pglob.is_null() {
    set_errno(EINVAL);

    return GLOB_ABORTED;
  }

  // SAFETY: Callers provide a valid NUL-terminated pattern pointer.
  let pattern_bytes = unsafe { CStr::from_ptr(pattern).to_bytes() };
  let mut matched_paths = match collect_matches(pattern_bytes, flags, errfunc) {
    Ok(paths) => paths,
    Err(code) => return code,
  };

  if matched_paths.is_empty() {
    if flags & GLOB_NOCHECK == 0 {
      return GLOB_NOMATCH;
    }

    matched_paths.push(nocheck_fallback_pattern(pattern_bytes, flags));
  }

  if flags & GLOB_NOSORT == 0 {
    matched_paths.sort();
  }

  let offsets = if flags & GLOB_APPEND != 0 || flags & GLOB_DOOFFS != 0 {
    // SAFETY: `pglob` is non-null and writable by function precondition.
    unsafe { usize_from_size_t((*pglob).gl_offs) }
  } else {
    0
  };
  let mut final_paths = if flags & GLOB_APPEND == 0 {
    Vec::new()
  } else {
    // SAFETY: `pglob` points to initialized `Glob` storage by C caller contract.
    unsafe { clone_existing_paths(pglob, offsets) }
  };

  final_paths.extend(matched_paths);

  let allocated = match allocate_pathv(&final_paths, offsets) {
    Ok(pathv) => pathv,
    Err(errno_value) => {
      set_errno(errno_value);

      return GLOB_NOSPACE;
    }
  };

  // SAFETY: `pglob` is non-null and writable by function precondition.
  unsafe {
    if !(*pglob).gl_pathv.is_null() {
      globfree(pglob);
    }

    (*pglob).gl_pathc = size_t_from_usize(final_paths.len());
    (*pglob).gl_pathv = allocated;
    (*pglob).gl_offs = size_t_from_usize(offsets);
    (*pglob).gl_flags = flags;
  }

  0
}

/// C ABI entry point for `globfree`.
///
/// Releases storage previously allocated by `glob` and resets the structure.
///
/// # Safety
/// - `pglob` must be null or point to a writable `Glob` instance.
/// - If non-null, `pglob` must either be zero-initialized or previously filled
///   by this module's `glob` implementation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn globfree(pglob: *mut Glob) {
  if pglob.is_null() {
    return;
  }

  // SAFETY: `pglob` is non-null and expected to point to initialized `Glob`.
  let path_count = unsafe { usize_from_size_t((*pglob).gl_pathc) };
  // SAFETY: `pglob` is non-null and expected to point to initialized `Glob`.
  let offsets = unsafe { usize_from_size_t((*pglob).gl_offs) };
  // SAFETY: `pglob` is non-null and expected to point to initialized `Glob`.
  let pathv = unsafe { (*pglob).gl_pathv };

  // SAFETY: storage contracts are documented on this function.
  unsafe {
    free_pathv(pathv, path_count, offsets);
  }

  // SAFETY: `pglob` is non-null and writable.
  unsafe {
    (*pglob).gl_pathc = 0;
    (*pglob).gl_pathv = ptr::null_mut();
    (*pglob).gl_offs = 0;
    (*pglob).gl_flags = 0;
  }
}

#[cfg(test)]
mod tests {
  use super::{GLOB_NOESCAPE, pattern_matches};

  #[test]
  fn pattern_matches_star_segment() {
    assert!(pattern_matches(b"a*.txt", b"alpha.txt", 0));
    assert!(pattern_matches(b"a*.txt", b"atom.txt", 0));
    assert!(!pattern_matches(b"a*.txt", b"beta.txt", 0));
  }

  #[test]
  fn pattern_matches_question_segment() {
    assert!(pattern_matches(b"x?.log", b"x1.log", 0));
    assert!(pattern_matches(b"x?.log", b"xy.log", 0));
    assert!(!pattern_matches(b"x?.log", b"xyz.log", 0));
  }

  #[test]
  fn pattern_matches_bracket_segment() {
    assert!(pattern_matches(b"[a-c]1.bin", b"a1.bin", 0));
    assert!(pattern_matches(b"[a-c]1.bin", b"b1.bin", 0));
    assert!(!pattern_matches(b"[a-c]1.bin", b"d1.bin", 0));
  }

  #[test]
  fn pattern_matches_bracket_escaped_hyphen_literal() {
    assert!(pattern_matches(b"[a\\-c]1.bin", b"a1.bin", 0));
    assert!(pattern_matches(b"[a\\-c]1.bin", b"-1.bin", 0));
    assert!(pattern_matches(b"[a\\-c]1.bin", b"c1.bin", 0));
    assert!(!pattern_matches(b"[a\\-c]1.bin", b"b1.bin", 0));
  }

  #[test]
  fn pattern_matches_bracket_descending_range_is_empty() {
    assert!(!pattern_matches(b"[z-a]1.bin", b"a1.bin", 0));
    assert!(!pattern_matches(b"[z-a]1.bin", b"z1.bin", 0));
  }

  #[test]
  fn pattern_matches_escaped_dot_segment() {
    assert!(pattern_matches(b"\\.*.cfg", b".alpha.cfg", 0));
    assert!(!pattern_matches(b"\\.*.cfg", b".alpha.cfg", GLOB_NOESCAPE));
  }
}
