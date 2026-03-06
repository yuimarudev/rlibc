use core::ffi::{c_char, c_int};
use rlibc::abi::errno::EINVAL;
use rlibc::errno::__errno_location;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStringExt;
use std::sync::{Mutex, MutexGuard, OnceLock};

unsafe extern "C" {
  fn getenv(name: *const c_char) -> *mut c_char;
  fn setenv(name: *const c_char, value: *const c_char, overwrite: c_int) -> c_int;
  fn unsetenv(name: *const c_char) -> c_int;
  fn clearenv() -> c_int;
  fn putenv(string: *mut c_char) -> c_int;
}

struct EnvScope {
  _lock: MutexGuard<'static, ()>,
  snapshot: Vec<(Vec<u8>, Vec<u8>)>,
}

impl EnvScope {
  fn new() -> Self {
    let lock = lock_env();
    let snapshot = snapshot_env();

    Self {
      _lock: lock,
      snapshot,
    }
  }
}

impl Drop for EnvScope {
  fn drop(&mut self) {
    restore_env(&self.snapshot);
  }
}

fn env_lock() -> &'static Mutex<()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_env() -> MutexGuard<'static, ()> {
  env_lock()
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn snapshot_env() -> Vec<(Vec<u8>, Vec<u8>)> {
  std::env::vars_os()
    .map(|(name, value)| (name.into_vec(), value.into_vec()))
    .collect()
}

fn c_string(input: &str) -> CString {
  CString::new(input)
    .unwrap_or_else(|_| unreachable!("test literals must not include interior NUL bytes"))
}

fn visible_proc_environ_name() -> CString {
  let proc_environ =
    std::fs::read("/proc/self/environ").expect("expected /proc/self/environ to be readable");

  for entry in proc_environ.split(|byte| *byte == 0) {
    if entry.is_empty() || !entry.contains(&b'=') {
      continue;
    }

    let equal_pos = entry
      .iter()
      .position(|byte| *byte == b'=')
      .expect("proc environ entry must contain '='");
    let name = CString::new(entry[..equal_pos].to_vec())
      .expect("proc environ name must not contain interior NUL");

    if getenv_bytes(&name).is_some() {
      return name;
    }
  }

  panic!("expected at least one /proc/self/environ entry visible before clearenv");
}

fn restore_env(snapshot: &[(Vec<u8>, Vec<u8>)]) {
  // SAFETY: C ABI allows clearing process environment; failures are ignored in cleanup.
  unsafe {
    let _ = clearenv();
  }

  for (name, value) in snapshot {
    if let (Ok(name_c), Ok(value_c)) = (
      CString::new(name.as_slice()),
      CString::new(value.as_slice()),
    ) {
      // SAFETY: `name_c`/`value_c` are valid NUL-terminated strings.
      unsafe {
        let _ = setenv(name_c.as_ptr(), value_c.as_ptr(), 1);
      }
    }
  }
}

fn getenv_bytes(name: &CString) -> Option<Vec<u8>> {
  // SAFETY: `name` is a valid NUL-terminated key.
  let value_ptr = unsafe { getenv(name.as_ptr()) };

  if value_ptr.is_null() {
    return None;
  }

  // SAFETY: `getenv` returns a valid NUL-terminated string for existing keys.
  Some(unsafe { CStr::from_ptr(value_ptr) }.to_bytes().to_vec())
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns valid TLS storage for this thread.
  unsafe { __errno_location().read() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns valid writable TLS storage.
  unsafe {
    __errno_location().write(value);
  }
}

#[test]
fn setenv_respects_overwrite_flag() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I017_SETENV_OVERWRITE");
  let first = c_string("first");
  let second = c_string("second");
  let third = c_string("third");

  write_errno(123);

  // SAFETY: `name`/`value` pointers are valid NUL-terminated strings.
  let create_result = unsafe { setenv(name.as_ptr(), first.as_ptr(), 1) };
  // SAFETY: `name`/`value` pointers are valid NUL-terminated strings.
  let no_overwrite_result = unsafe { setenv(name.as_ptr(), second.as_ptr(), 0) };
  // SAFETY: `name`/`value` pointers are valid NUL-terminated strings.
  let overwrite_result = unsafe { setenv(name.as_ptr(), third.as_ptr(), 1) };

  assert_eq!(create_result, 0);
  assert_eq!(no_overwrite_result, 0);
  assert_eq!(overwrite_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"third".to_vec()));
  assert_eq!(read_errno(), 123);
}

#[test]
fn setenv_without_overwrite_preserves_putenv_alias() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I017_SETENV_ALIAS_KEEP");
  let replacement = c_string("replacement");
  let prefix = b"RLIBC_I017_SETENV_ALIAS_KEEP=";
  let mut entry = b"RLIBC_I017_SETENV_ALIAS_KEEP=alpha\0".to_vec();

  write_errno(55);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"alpha".to_vec()));

  // SAFETY: `name` and `replacement` are valid NUL-terminated strings.
  let set_result = unsafe { setenv(name.as_ptr(), replacement.as_ptr(), 0) };

  assert_eq!(set_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&name), Some(b"omega".to_vec()));
  assert_eq!(read_errno(), 55);
}

#[test]
fn setenv_overwrite_replaces_putenv_alias_and_preserves_errno() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I017_SETENV_OVERWRITE_ALIAS_REPLACE");
  let replacement = c_string("replacement");
  let prefix = b"RLIBC_I017_SETENV_OVERWRITE_ALIAS_REPLACE=";
  let mut entry = b"RLIBC_I017_SETENV_OVERWRITE_ALIAS_REPLACE=alpha\0".to_vec();

  write_errno(67);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"alpha".to_vec()));

  // SAFETY: `name` and `replacement` are valid NUL-terminated strings.
  let set_result = unsafe { setenv(name.as_ptr(), replacement.as_ptr(), 1) };

  assert_eq!(set_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"replacement".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&name), Some(b"replacement".to_vec()));
  assert_eq!(read_errno(), 67);
}

#[test]
fn setenv_overwrite_i016_replaces_empty_putenv_alias_and_preserves_errno() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I016_SETENV_OVERWRITE_EMPTY_ALIAS_REPLACE");
  let replacement = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_OVERWRITE_EMPTY_ALIAS_REPLACE=";
  let mut entry = b"RLIBC_I016_SETENV_OVERWRITE_EMPTY_ALIAS_REPLACE=\0\0".to_vec();

  write_errno(68);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&name), Some(Vec::new()));

  // SAFETY: `name` and `replacement` are valid NUL-terminated strings.
  let set_result = unsafe { setenv(name.as_ptr(), replacement.as_ptr(), 1) };

  assert_eq!(set_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"replacement".to_vec()));

  let value_start = prefix.len();

  entry[value_start] = b'z';

  assert_eq!(getenv_bytes(&name), Some(b"replacement".to_vec()));
  assert_eq!(read_errno(), 68);
}

#[test]
fn setenv_overwrite_i016_replaces_rebound_empty_putenv_alias_and_preserves_errno() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I016_SETENV_OVERWRITE_REBOUND_EMPTY_ALIAS_REPLACE");
  let replacement = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_OVERWRITE_REBOUND_EMPTY_ALIAS_REPLACE=";
  let mut first = b"RLIBC_I016_SETENV_OVERWRITE_REBOUND_EMPTY_ALIAS_REPLACE=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_SETENV_OVERWRITE_REBOUND_EMPTY_ALIAS_REPLACE=\0\0".to_vec();

  write_errno(69);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&name), Some(Vec::new()));

  // SAFETY: `name` and `replacement` are valid NUL-terminated strings.
  let set_result = unsafe { setenv(name.as_ptr(), replacement.as_ptr(), 1) };

  assert_eq!(set_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"replacement".to_vec()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");
  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&name), Some(b"replacement".to_vec()));
  assert_eq!(read_errno(), 69);
}

#[test]
fn unsetenv_removes_entry() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I017_UNSETENV");
  let value = c_string("present");

  write_errno(41);

  // SAFETY: `name`/`value` pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(name.as_ptr(), value.as_ptr(), 1) };

  assert_eq!(set_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"present".to_vec()));

  // SAFETY: `name` pointer is a valid NUL-terminated string.
  let unset_result = unsafe { unsetenv(name.as_ptr()) };

  assert_eq!(unset_result, 0);
  assert_eq!(getenv_bytes(&name), None);
  assert_eq!(read_errno(), 41);
}

#[test]
fn unsetenv_missing_name_preserves_other_putenv_alias_and_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_UNSETENV_MISSING_ALIAS_TRACKED");
  let missing_name = c_string("RLIBC_I017_UNSETENV_MISSING_ALIAS_TARGET");
  let prefix = b"RLIBC_I017_UNSETENV_MISSING_ALIAS_TRACKED=";
  let mut entry = b"RLIBC_I017_UNSETENV_MISSING_ALIAS_TRACKED=alpha\0".to_vec();

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  write_errno(45);

  // SAFETY: `missing_name` is a valid NUL-terminated string.
  let unset_result = unsafe { unsetenv(missing_name.as_ptr()) };

  assert_eq!(unset_result, 0);
  assert_eq!(read_errno(), 45);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn unsetenv_missing_name_i016_without_alias_preserves_errno() {
  let _env = EnvScope::new();
  let missing_name = c_string("RLIBC_I016_UNSETENV_MISSING_NO_ALIAS_TARGET");
  let untouched_name = c_string("RLIBC_I016_UNSETENV_MISSING_NO_ALIAS_UNTOUCHED");
  let untouched_value = c_string("stable");

  write_errno(46);

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(untouched_name.as_ptr(), untouched_value.as_ptr(), 1) };

  assert_eq!(set_result, 0);
  assert_eq!(getenv_bytes(&untouched_name), Some(b"stable".to_vec()));

  // SAFETY: `missing_name` is a valid NUL-terminated string.
  let unset_result = unsafe { unsetenv(missing_name.as_ptr()) };

  assert_eq!(unset_result, 0);
  assert_eq!(read_errno(), 46);
  assert_eq!(getenv_bytes(&untouched_name), Some(b"stable".to_vec()));
  assert_eq!(getenv_bytes(&missing_name), None);
}

#[test]
fn unsetenv_missing_name_i016_preserves_empty_putenv_alias_and_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_UNSETENV_MISSING_EMPTY_ALIAS_TRACKED");
  let missing_name = c_string("RLIBC_I016_UNSETENV_MISSING_EMPTY_ALIAS_TARGET");
  let prefix = b"RLIBC_I016_UNSETENV_MISSING_EMPTY_ALIAS_TRACKED=";
  let mut entry = b"RLIBC_I016_UNSETENV_MISSING_EMPTY_ALIAS_TRACKED=\0\0".to_vec();

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  write_errno(47);

  // SAFETY: `missing_name` is a valid NUL-terminated string.
  let unset_result = unsafe { unsetenv(missing_name.as_ptr()) };

  assert_eq!(unset_result, 0);
  assert_eq!(read_errno(), 47);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  entry[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn unsetenv_i016_removes_empty_putenv_alias_without_resurrection() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_UNSETENV_EMPTY_ALIAS_REMOVE");
  let prefix = b"RLIBC_I016_UNSETENV_EMPTY_ALIAS_REMOVE=";
  let mut entry = b"RLIBC_I016_UNSETENV_EMPTY_ALIAS_REMOVE=\0\0".to_vec();

  write_errno(47);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: `tracked_name` is a valid NUL-terminated string.
  let unset_result = unsafe { unsetenv(tracked_name.as_ptr()) };

  assert_eq!(unset_result, 0);
  assert_eq!(read_errno(), 47);
  assert_eq!(getenv_bytes(&tracked_name), None);

  let value_start = prefix.len();

  entry[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), None);
}

#[test]
fn unsetenv_i016_removes_rebound_empty_putenv_alias_without_resurrection() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_UNSETENV_REBOUND_EMPTY_ALIAS_REMOVE");
  let prefix = b"RLIBC_I016_UNSETENV_REBOUND_EMPTY_ALIAS_REMOVE=";
  let mut first = b"RLIBC_I016_UNSETENV_REBOUND_EMPTY_ALIAS_REMOVE=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_UNSETENV_REBOUND_EMPTY_ALIAS_REMOVE=\0\0".to_vec();

  write_errno(48);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: `tracked_name` is a valid NUL-terminated string.
  let unset_result = unsafe { unsetenv(tracked_name.as_ptr()) };

  assert_eq!(unset_result, 0);
  assert_eq!(read_errno(), 48);
  assert_eq!(getenv_bytes(&tracked_name), None);

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");
  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), None);
}

#[test]
fn clearenv_clears_existing_entries() {
  let _env = EnvScope::new();
  let first_name = c_string("RLIBC_I017_CLEARENV_A");
  let first_value = c_string("value-a");
  let second_name = c_string("RLIBC_I017_CLEARENV_B");
  let second_value = c_string("value-b");

  write_errno(77);

  // SAFETY: pointers are valid NUL-terminated strings.
  let first_set = unsafe { setenv(first_name.as_ptr(), first_value.as_ptr(), 1) };
  // SAFETY: pointers are valid NUL-terminated strings.
  let second_set = unsafe { setenv(second_name.as_ptr(), second_value.as_ptr(), 1) };

  assert_eq!(first_set, 0);
  assert_eq!(second_set, 0);
  assert_eq!(getenv_bytes(&first_name), Some(b"value-a".to_vec()));
  assert_eq!(getenv_bytes(&second_name), Some(b"value-b".to_vec()));

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  let clear_result = unsafe { clearenv() };

  assert_eq!(clear_result, 0);
  assert_eq!(getenv_bytes(&first_name), None);
  assert_eq!(getenv_bytes(&second_name), None);
  assert_eq!(read_errno(), 77);
}

#[test]
fn clearenv_when_empty_preserves_errno() {
  let _env = EnvScope::new();
  let missing_name = c_string("RLIBC_I017_CLEARENV_EMPTY_MISSING");

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  assert_eq!(unsafe { clearenv() }, 0);
  assert_eq!(getenv_bytes(&missing_name), None);

  write_errno(86);

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  let clear_result = unsafe { clearenv() };

  assert_eq!(clear_result, 0);
  assert_eq!(read_errno(), 86);
  assert_eq!(getenv_bytes(&missing_name), None);
}

#[test]
fn clearenv_when_empty_does_not_resurrect_proc_environ_entries() {
  let _env = EnvScope::new();
  let proc_name = visible_proc_environ_name();

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  assert_eq!(unsafe { clearenv() }, 0);
  assert_eq!(getenv_bytes(&proc_name), None);

  write_errno(96);

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  assert_eq!(unsafe { clearenv() }, 0);
  assert_eq!(read_errno(), 96);
  assert_eq!(
    getenv_bytes(&proc_name),
    None,
    "clearenv on an already empty environment must not resurrect /proc/self/environ entries",
  );
}

#[test]
fn clearenv_clears_putenv_aliases_and_preserves_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_CLEARENV_ALIAS_TRACKED");
  let prefix = b"RLIBC_I017_CLEARENV_ALIAS_TRACKED=";
  let mut entry = b"RLIBC_I017_CLEARENV_ALIAS_TRACKED=alpha\0".to_vec();

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  write_errno(84);

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  let clear_result = unsafe { clearenv() };

  assert_eq!(clear_result, 0);
  assert_eq!(read_errno(), 84);
  assert_eq!(getenv_bytes(&tracked_name), None);

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), None);
}

#[test]
fn clearenv_i016_clears_empty_putenv_alias_without_resurrection() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_CLEARENV_EMPTY_ALIAS_TRACKED");
  let prefix = b"RLIBC_I016_CLEARENV_EMPTY_ALIAS_TRACKED=";
  let mut entry = b"RLIBC_I016_CLEARENV_EMPTY_ALIAS_TRACKED=\0\0".to_vec();

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  write_errno(85);

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  let clear_result = unsafe { clearenv() };

  assert_eq!(clear_result, 0);
  assert_eq!(read_errno(), 85);
  assert_eq!(getenv_bytes(&tracked_name), None);

  let value_start = prefix.len();

  entry[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), None);
}

#[test]
fn clearenv_i016_clears_rebound_empty_putenv_alias_without_resurrection() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_CLEARENV_REBOUND_EMPTY_ALIAS_TRACKED");
  let prefix = b"RLIBC_I016_CLEARENV_REBOUND_EMPTY_ALIAS_TRACKED=";
  let mut first = b"RLIBC_I016_CLEARENV_REBOUND_EMPTY_ALIAS_TRACKED=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_CLEARENV_REBOUND_EMPTY_ALIAS_TRACKED=\0\0".to_vec();

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  write_errno(87);

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  let clear_result = unsafe { clearenv() };

  assert_eq!(clear_result, 0);
  assert_eq!(read_errno(), 87);
  assert_eq!(getenv_bytes(&tracked_name), None);

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");
  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), None);
}

#[test]
fn setenv_after_clearenv_keeps_proc_environ_entries_cleared() {
  let _env = EnvScope::new();
  let proc_name = visible_proc_environ_name();
  let name = c_string("RLIBC_I017_SETENV_AFTER_CLEARENV_EMPTY_ENV");
  let value = c_string("fresh");

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  assert_eq!(unsafe { clearenv() }, 0);
  assert_eq!(getenv_bytes(&proc_name), None);

  // SAFETY: `name`/`value` pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(name.as_ptr(), value.as_ptr(), 1) };

  assert_eq!(set_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"fresh".to_vec()));
  assert_eq!(
    getenv_bytes(&proc_name),
    None,
    "adding a new entry after clearenv must not resurrect prior host environment entries",
  );
}

#[test]
fn putenv_tracks_backing_buffer_updates() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I017_PUTENV_TRACKING");
  let prefix = b"RLIBC_I017_PUTENV_TRACKING=";
  let mut entry = b"RLIBC_I017_PUTENV_TRACKING=alpha\0".to_vec();

  write_errno(88);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&name), Some(b"omega".to_vec()));
  assert_eq!(read_errno(), 88);
}

#[test]
fn putenv_same_name_rebinds_to_latest_buffer() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I016_PUTENV_REBIND");
  let prefix = b"RLIBC_I016_PUTENV_REBIND=";
  let mut first = b"RLIBC_I016_PUTENV_REBIND=first\0".to_vec();
  let mut second = b"RLIBC_I016_PUTENV_REBIND=second\0".to_vec();

  write_errno(99);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_result, 0);
  assert_eq!(second_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"second".to_vec()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"stale");

  assert_eq!(getenv_bytes(&name), Some(b"second".to_vec()));

  second[value_start..value_start + 6].copy_from_slice(b"latest");

  assert_eq!(getenv_bytes(&name), Some(b"latest".to_vec()));
  assert_eq!(read_errno(), 99);
}

#[test]
fn putenv_rebind_i016_from_empty_value_detaches_old_alias() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I016_PUTENV_REBIND_EMPTY_TO_VALUE");
  let prefix = b"RLIBC_I016_PUTENV_REBIND_EMPTY_TO_VALUE=";
  let mut first = b"RLIBC_I016_PUTENV_REBIND_EMPTY_TO_VALUE=\0\0".to_vec();
  let mut second = b"RLIBC_I016_PUTENV_REBIND_EMPTY_TO_VALUE=beta\0".to_vec();

  write_errno(57);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_result = unsafe { putenv(first.as_mut_ptr().cast()) };

  assert_eq!(first_result, 0);
  assert_eq!(getenv_bytes(&name), Some(Vec::new()));

  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(second_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"beta".to_vec()));

  let value_start = prefix.len();

  first[value_start] = b'z';

  assert_eq!(getenv_bytes(&name), Some(b"beta".to_vec()));

  second[value_start..value_start + 4].copy_from_slice(b"zeta");

  assert_eq!(getenv_bytes(&name), Some(b"zeta".to_vec()));
  assert_eq!(read_errno(), 57);
}

#[test]
fn putenv_rebind_i016_from_value_to_empty_detaches_old_alias() {
  let _env = EnvScope::new();
  let name = c_string("RLIBC_I016_PUTENV_REBIND_VALUE_TO_EMPTY");
  let prefix = b"RLIBC_I016_PUTENV_REBIND_VALUE_TO_EMPTY=";
  let mut first = b"RLIBC_I016_PUTENV_REBIND_VALUE_TO_EMPTY=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_PUTENV_REBIND_VALUE_TO_EMPTY=\0\0".to_vec();

  write_errno(58);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_result = unsafe { putenv(first.as_mut_ptr().cast()) };

  assert_eq!(first_result, 0);
  assert_eq!(getenv_bytes(&name), Some(b"alpha".to_vec()));

  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(second_result, 0);
  assert_eq!(getenv_bytes(&name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&name), Some(b"z".to_vec()));
  assert_eq!(read_errno(), 58);
}

#[test]
fn setenv_invalid_name_keeps_existing_putenv_alias() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_INVALID_ALIAS");
  let invalid_name = c_string("RLIBC_I016=INVALID_NAME");
  let value = c_string("value");
  let prefix = b"RLIBC_I016_SETENV_INVALID_ALIAS=";
  let mut entry = b"RLIBC_I016_SETENV_INVALID_ALIAS=alpha\0".to_vec();

  write_errno(7);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(invalid_name.as_ptr(), value.as_ptr(), 1) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn setenv_invalid_name_no_overwrite_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_SETENV_INVALID_NO_OVERWRITE_ALIAS");
  let invalid_name = c_string("RLIBC_I017=INVALID_NO_OVERWRITE_NAME");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I017_SETENV_INVALID_NO_OVERWRITE_ALIAS=";
  let mut entry = b"RLIBC_I017_SETENV_INVALID_NO_OVERWRITE_ALIAS=alpha\0".to_vec();

  write_errno(9);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(invalid_name.as_ptr(), value.as_ptr(), 0) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn setenv_invalid_name_i016_no_overwrite_preserves_rebound_empty_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_INVALID_NO_OVERWRITE_REBOUND_EMPTY_ALIAS");
  let invalid_name = c_string("RLIBC_I016=INVALID_NO_OVERWRITE_REBOUND_EMPTY_NAME");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_INVALID_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_SETENV_INVALID_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_SETENV_INVALID_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(10);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(invalid_name.as_ptr(), value.as_ptr(), 0) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn setenv_invalid_name_i016_overwrite_preserves_rebound_empty_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_INVALID_OVERWRITE_REBOUND_EMPTY_ALIAS");
  let invalid_name = c_string("RLIBC_I016=INVALID_OVERWRITE_REBOUND_EMPTY_NAME");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_INVALID_OVERWRITE_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_SETENV_INVALID_OVERWRITE_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_SETENV_INVALID_OVERWRITE_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(14);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(invalid_name.as_ptr(), value.as_ptr(), 1) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn setenv_empty_name_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_SETENV_EMPTY_NAME_ALIAS");
  let empty_name = c_string("");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I017_SETENV_EMPTY_NAME_ALIAS=";
  let mut entry = b"RLIBC_I017_SETENV_EMPTY_NAME_ALIAS=alpha\0".to_vec();

  write_errno(11);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(empty_name.as_ptr(), value.as_ptr(), 1) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn setenv_empty_name_no_overwrite_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_EMPTY_NO_OVERWRITE_ALIAS");
  let empty_name = c_string("");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_EMPTY_NO_OVERWRITE_ALIAS=";
  let mut entry = b"RLIBC_I016_SETENV_EMPTY_NO_OVERWRITE_ALIAS=alpha\0".to_vec();

  write_errno(11);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(empty_name.as_ptr(), value.as_ptr(), 0) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn setenv_empty_name_i016_no_overwrite_preserves_rebound_empty_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_EMPTY_NO_OVERWRITE_REBOUND_EMPTY_ALIAS");
  let empty_name = c_string("");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_EMPTY_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_SETENV_EMPTY_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_SETENV_EMPTY_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(12);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(empty_name.as_ptr(), value.as_ptr(), 0) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn setenv_empty_name_i016_overwrite_preserves_rebound_empty_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_EMPTY_OVERWRITE_REBOUND_EMPTY_ALIAS");
  let empty_name = c_string("");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_EMPTY_OVERWRITE_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_SETENV_EMPTY_OVERWRITE_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_SETENV_EMPTY_OVERWRITE_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(15);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(empty_name.as_ptr(), value.as_ptr(), 1) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn setenv_null_value_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_SETENV_NULL_VALUE_ALIAS");
  let prefix = b"RLIBC_I017_SETENV_NULL_VALUE_ALIAS=";
  let mut entry = b"RLIBC_I017_SETENV_NULL_VALUE_ALIAS=alpha\0".to_vec();

  write_errno(58);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: null value pointer is passed intentionally to validate EINVAL path.
  let set_result = unsafe { setenv(tracked_name.as_ptr(), core::ptr::null(), 1) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn setenv_null_value_no_overwrite_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_NULL_VALUE_NO_OVERWRITE_ALIAS");
  let prefix = b"RLIBC_I016_SETENV_NULL_VALUE_NO_OVERWRITE_ALIAS=";
  let mut entry = b"RLIBC_I016_SETENV_NULL_VALUE_NO_OVERWRITE_ALIAS=alpha\0".to_vec();

  write_errno(12);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: null value pointer is passed intentionally to validate EINVAL path.
  let set_result = unsafe { setenv(tracked_name.as_ptr(), core::ptr::null(), 0) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn setenv_null_value_i016_overwrite_preserves_rebound_empty_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_NULL_VALUE_OVERWRITE_REBOUND_EMPTY_ALIAS");
  let prefix = b"RLIBC_I016_SETENV_NULL_VALUE_OVERWRITE_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_SETENV_NULL_VALUE_OVERWRITE_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_SETENV_NULL_VALUE_OVERWRITE_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(16);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: null value pointer is passed intentionally to validate EINVAL path.
  let set_result = unsafe { setenv(tracked_name.as_ptr(), core::ptr::null(), 1) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn setenv_null_name_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_SETENV_NULL_NAME_ALIAS");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I017_SETENV_NULL_NAME_ALIAS=";
  let mut entry = b"RLIBC_I017_SETENV_NULL_NAME_ALIAS=alpha\0".to_vec();

  write_errno(62);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: null name pointer is passed intentionally to validate EINVAL path.
  let set_result = unsafe { setenv(core::ptr::null(), value.as_ptr(), 1) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn setenv_null_name_no_overwrite_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_NULL_NAME_NO_OVERWRITE_ALIAS");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_NULL_NAME_NO_OVERWRITE_ALIAS=";
  let mut entry = b"RLIBC_I016_SETENV_NULL_NAME_NO_OVERWRITE_ALIAS=alpha\0".to_vec();

  write_errno(13);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: null name pointer is passed intentionally to validate EINVAL path.
  let set_result = unsafe { setenv(core::ptr::null(), value.as_ptr(), 0) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn setenv_null_name_i016_overwrite_preserves_rebound_empty_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_NULL_NAME_OVERWRITE_REBOUND_EMPTY_ALIAS");
  let value = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_NULL_NAME_OVERWRITE_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_SETENV_NULL_NAME_OVERWRITE_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_SETENV_NULL_NAME_OVERWRITE_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(17);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: null name pointer is passed intentionally to validate EINVAL path.
  let set_result = unsafe { setenv(core::ptr::null(), value.as_ptr(), 1) };

  assert_eq!(set_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn unsetenv_invalid_name_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_UNSETENV_INVALID_ALIAS");
  let invalid_name = c_string("RLIBC_I017=UNSETENV_INVALID_NAME");
  let prefix = b"RLIBC_I017_UNSETENV_INVALID_ALIAS=";
  let mut entry = b"RLIBC_I017_UNSETENV_INVALID_ALIAS=alpha\0".to_vec();

  write_errno(64);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: `invalid_name` is a valid NUL-terminated string.
  let unset_result = unsafe { unsetenv(invalid_name.as_ptr()) };

  assert_eq!(unset_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn unsetenv_empty_name_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_UNSETENV_EMPTY_ALIAS");
  let empty_name = c_string("");
  let prefix = b"RLIBC_I017_UNSETENV_EMPTY_ALIAS=";
  let mut entry = b"RLIBC_I017_UNSETENV_EMPTY_ALIAS=alpha\0".to_vec();

  write_errno(68);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: `empty_name` is a valid empty NUL-terminated string.
  let unset_result = unsafe { unsetenv(empty_name.as_ptr()) };

  assert_eq!(unset_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn unsetenv_empty_name_i016_preserves_rebound_empty_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_UNSETENV_EMPTY_REBOUND_EMPTY_ALIAS");
  let empty_name = c_string("");
  let prefix = b"RLIBC_I016_UNSETENV_EMPTY_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_UNSETENV_EMPTY_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_UNSETENV_EMPTY_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(69);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: `empty_name` is a valid empty NUL-terminated string.
  let unset_result = unsafe { unsetenv(empty_name.as_ptr()) };

  assert_eq!(unset_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn unsetenv_null_name_preserves_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_UNSETENV_NULL_ALIAS");
  let prefix = b"RLIBC_I017_UNSETENV_NULL_ALIAS=";
  let mut entry = b"RLIBC_I017_UNSETENV_NULL_ALIAS=alpha\0".to_vec();

  write_errno(72);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  // SAFETY: null name pointer is passed intentionally to validate EINVAL path.
  let unset_result = unsafe { unsetenv(core::ptr::null()) };

  assert_eq!(unset_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn unsetenv_null_name_i016_preserves_rebound_empty_putenv_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_UNSETENV_NULL_REBOUND_EMPTY_ALIAS");
  let prefix = b"RLIBC_I016_UNSETENV_NULL_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_UNSETENV_NULL_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_UNSETENV_NULL_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(73);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: null name pointer is passed intentionally to validate EINVAL path.
  let unset_result = unsafe { unsetenv(core::ptr::null()) };

  assert_eq!(unset_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
}

#[test]
fn putenv_invalid_name_preserves_existing_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_PUTENV_INVALID_ALIAS");
  let mut tracked_entry = b"RLIBC_I017_PUTENV_INVALID_ALIAS=alpha\0".to_vec();
  let value_start = b"RLIBC_I017_PUTENV_INVALID_ALIAS=".len();
  let mut invalid_entry = b"=broken\0".to_vec();

  // SAFETY: `tracked_entry` is mutable and NUL-terminated for C.
  let tracked_result = unsafe { putenv(tracked_entry.as_mut_ptr().cast()) };

  assert_eq!(tracked_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  write_errno(35);

  // SAFETY: `invalid_entry` is mutable and NUL-terminated for C.
  let invalid_result = unsafe { putenv(invalid_entry.as_mut_ptr().cast()) };

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  tracked_entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn putenv_invalid_name_without_existing_alias_sets_errno_and_keeps_absent() {
  let _env = EnvScope::new();
  let missing_name = c_string("RLIBC_I017_PUTENV_INVALID_NO_ALIAS");
  let mut invalid_entry = b"=broken\0".to_vec();

  assert_eq!(getenv_bytes(&missing_name), None);
  write_errno(38);

  // SAFETY: `invalid_entry` is mutable and NUL-terminated for C.
  let invalid_result = unsafe { putenv(invalid_entry.as_mut_ptr().cast()) };

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&missing_name), None);
}

#[test]
fn putenv_empty_string_preserves_existing_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_PUTENV_EMPTY_ALIAS");
  let prefix = b"RLIBC_I017_PUTENV_EMPTY_ALIAS=";
  let mut tracked_entry = b"RLIBC_I017_PUTENV_EMPTY_ALIAS=alpha\0".to_vec();
  let mut empty_entry = b"\0".to_vec();

  // SAFETY: `tracked_entry` is mutable and NUL-terminated for C.
  let tracked_result = unsafe { putenv(tracked_entry.as_mut_ptr().cast()) };

  assert_eq!(tracked_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  write_errno(39);

  // SAFETY: `empty_entry` is a mutable NUL-terminated empty C string.
  let empty_result = unsafe { putenv(empty_entry.as_mut_ptr().cast()) };

  assert_eq!(empty_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  tracked_entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn putenv_empty_value_i016_tracks_alias_and_preserves_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_PUTENV_EMPTY_VALUE_ALIAS");
  let prefix = b"RLIBC_I016_PUTENV_EMPTY_VALUE_ALIAS=";
  let mut entry = b"RLIBC_I016_PUTENV_EMPTY_VALUE_ALIAS=\0\0".to_vec();

  write_errno(52);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(read_errno(), 52);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  entry[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
  assert_eq!(read_errno(), 52);
}

#[test]
fn setenv_no_overwrite_i016_keeps_empty_putenv_alias() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_NO_OVERWRITE_EMPTY_ALIAS");
  let replacement = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_NO_OVERWRITE_EMPTY_ALIAS=";
  let mut entry = b"RLIBC_I016_SETENV_NO_OVERWRITE_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(53);

  // SAFETY: `entry` is mutable and NUL-terminated for C.
  let put_result = unsafe { putenv(entry.as_mut_ptr().cast()) };

  assert_eq!(put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(tracked_name.as_ptr(), replacement.as_ptr(), 0) };

  assert_eq!(set_result, 0);
  assert_eq!(read_errno(), 53);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  entry[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
  assert_eq!(read_errno(), 53);
}

#[test]
fn setenv_no_overwrite_i016_keeps_rebound_empty_putenv_alias() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I016_SETENV_NO_OVERWRITE_REBOUND_EMPTY_ALIAS");
  let replacement = c_string("replacement");
  let prefix = b"RLIBC_I016_SETENV_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=";
  let mut first = b"RLIBC_I016_SETENV_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=alpha\0".to_vec();
  let mut second = b"RLIBC_I016_SETENV_NO_OVERWRITE_REBOUND_EMPTY_ALIAS=\0\0".to_vec();

  write_errno(54);

  // SAFETY: `first` is mutable and NUL-terminated for C.
  let first_put_result = unsafe { putenv(first.as_mut_ptr().cast()) };
  // SAFETY: `second` is mutable and NUL-terminated for C.
  let second_put_result = unsafe { putenv(second.as_mut_ptr().cast()) };

  assert_eq!(first_put_result, 0);
  assert_eq!(second_put_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  // SAFETY: pointers are valid NUL-terminated strings.
  let set_result = unsafe { setenv(tracked_name.as_ptr(), replacement.as_ptr(), 0) };

  assert_eq!(set_result, 0);
  assert_eq!(read_errno(), 54);
  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  let value_start = prefix.len();

  first[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(Vec::new()));

  second[value_start] = b'z';

  assert_eq!(getenv_bytes(&tracked_name), Some(b"z".to_vec()));
  assert_eq!(read_errno(), 54);
}

#[test]
fn putenv_without_equal_unsets_variable_and_preserves_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_PUTENV_UNSET_ALIAS");
  let prefix = b"RLIBC_I017_PUTENV_UNSET_ALIAS=";
  let mut tracked_entry = b"RLIBC_I017_PUTENV_UNSET_ALIAS=alpha\0".to_vec();
  let mut unset_entry = b"RLIBC_I017_PUTENV_UNSET_ALIAS\0".to_vec();

  // SAFETY: `tracked_entry` is mutable and NUL-terminated for C.
  let tracked_result = unsafe { putenv(tracked_entry.as_mut_ptr().cast()) };

  assert_eq!(tracked_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  write_errno(43);

  // SAFETY: `unset_entry` is mutable and NUL-terminated for C.
  let unset_result = unsafe { putenv(unset_entry.as_mut_ptr().cast()) };

  assert_eq!(unset_result, 0);
  assert_eq!(read_errno(), 43);
  assert_eq!(getenv_bytes(&tracked_name), None);

  let value_start = prefix.len();

  tracked_entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), None);
}

#[test]
fn putenv_without_equal_missing_name_preserves_errno_and_keeps_absent() {
  let _env = EnvScope::new();
  let missing_name = c_string("RLIBC_I017_PUTENV_UNSET_MISSING");
  let mut unset_entry = b"RLIBC_I017_PUTENV_UNSET_MISSING\0".to_vec();

  assert_eq!(getenv_bytes(&missing_name), None);
  write_errno(45);

  // SAFETY: `unset_entry` is mutable and NUL-terminated for C.
  let unset_result = unsafe { putenv(unset_entry.as_mut_ptr().cast()) };

  assert_eq!(unset_result, 0);
  assert_eq!(read_errno(), 45);
  assert_eq!(getenv_bytes(&missing_name), None);
}

#[test]
fn putenv_without_equal_missing_name_preserves_other_putenv_alias_and_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_PUTENV_UNSET_MISSING_KEEP_ALIAS");
  let missing_name = c_string("RLIBC_I017_PUTENV_UNSET_MISSING_KEEP_ALIAS_TARGET");
  let prefix = b"RLIBC_I017_PUTENV_UNSET_MISSING_KEEP_ALIAS=";
  let mut tracked_entry = b"RLIBC_I017_PUTENV_UNSET_MISSING_KEEP_ALIAS=alpha\0".to_vec();
  let mut unset_entry = b"RLIBC_I017_PUTENV_UNSET_MISSING_KEEP_ALIAS_TARGET\0".to_vec();

  // SAFETY: `tracked_entry` is mutable and NUL-terminated for C.
  let tracked_result = unsafe { putenv(tracked_entry.as_mut_ptr().cast()) };

  assert_eq!(tracked_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));
  assert_eq!(getenv_bytes(&missing_name), None);

  write_errno(46);

  // SAFETY: `unset_entry` is mutable and NUL-terminated for C.
  let unset_result = unsafe { putenv(unset_entry.as_mut_ptr().cast()) };

  assert_eq!(unset_result, 0);
  assert_eq!(read_errno(), 46);
  assert_eq!(getenv_bytes(&missing_name), None);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  tracked_entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}

#[test]
fn putenv_null_pointer_preserves_existing_alias_and_sets_errno() {
  let _env = EnvScope::new();
  let tracked_name = c_string("RLIBC_I017_PUTENV_NULL_ALIAS");
  let prefix = b"RLIBC_I017_PUTENV_NULL_ALIAS=";
  let mut tracked_entry = b"RLIBC_I017_PUTENV_NULL_ALIAS=alpha\0".to_vec();

  // SAFETY: `tracked_entry` is mutable and NUL-terminated for C.
  let tracked_result = unsafe { putenv(tracked_entry.as_mut_ptr().cast()) };

  assert_eq!(tracked_result, 0);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  write_errno(36);

  // SAFETY: null pointer is passed intentionally to validate EINVAL path.
  let null_result = unsafe { putenv(core::ptr::null_mut()) };

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(getenv_bytes(&tracked_name), Some(b"alpha".to_vec()));

  let value_start = prefix.len();

  tracked_entry[value_start..value_start + 5].copy_from_slice(b"omega");

  assert_eq!(getenv_bytes(&tracked_name), Some(b"omega".to_vec()));
}
