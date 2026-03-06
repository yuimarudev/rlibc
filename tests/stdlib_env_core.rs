use core::ffi::{c_char, c_int};
use rlibc::stdlib::env::core::getenv;
use std::env;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStringExt;
use std::sync::{Mutex, MutexGuard, PoisonError};

unsafe extern "C" {
  fn clearenv() -> c_int;
  fn setenv(name: *const c_char, value: *const c_char, overwrite: c_int) -> c_int;
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

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

fn lock_env() -> MutexGuard<'static, ()> {
  ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn snapshot_env() -> Vec<(Vec<u8>, Vec<u8>)> {
  env::vars_os()
    .map(|(name, value)| (name.into_vec(), value.into_vec()))
    .collect()
}

fn restore_env(snapshot: &[(Vec<u8>, Vec<u8>)]) {
  // SAFETY: C ABI allows clearing process environment; cleanup ignores failures.
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

fn getenv_bytes_c(name: &CStr) -> Option<Vec<u8>> {
  // SAFETY: `name` is a valid NUL-terminated C string pointer.
  let value_ptr = unsafe { getenv(name.as_ptr()) }.cast_const();

  if value_ptr.is_null() {
    return None;
  }

  // SAFETY: `getenv` returns a pointer to a valid NUL-terminated value string.
  Some(unsafe { CStr::from_ptr(value_ptr) }.to_bytes().to_vec())
}

fn getenv_bytes(name: &str) -> Option<Vec<u8>> {
  let c_name = CString::new(name).expect("name must not contain interior NUL bytes");

  getenv_bytes_c(&c_name)
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

    if getenv_bytes_c(&name).is_some() {
      return name;
    }
  }

  panic!("expected at least one /proc/self/environ entry visible before clearenv");
}

fn set_env_var(name: &str, value: &str) {
  // SAFETY: environment mutation is serialized by `ENV_LOCK` in each test.
  unsafe { env::set_var(name, value) };
}

#[test]
fn getenv_returns_value_for_exact_name_match() {
  let _env = EnvScope::new();
  let exact_name = "RLIBC_I016_GETENV_EXACT";
  let suffix_name = "RLIBC_I016_GETENV_EXACT_SUFFIX";

  set_env_var(exact_name, "expected");
  set_env_var(suffix_name, "unexpected");

  assert_eq!(
    getenv_bytes(exact_name).as_deref(),
    Some(b"expected".as_slice())
  );
}

#[test]
fn getenv_returns_none_for_undefined_variable() {
  let _env = EnvScope::new();
  let name = "RLIBC_I016_GETENV_UNDEFINED";

  // SAFETY: environment mutation is serialized by `ENV_LOCK` in each test.
  unsafe { env::remove_var(name) };

  assert_eq!(getenv_bytes(name), None);
}

#[test]
fn getenv_returns_empty_string_for_empty_value() {
  let _env = EnvScope::new();
  let name = "RLIBC_I016_GETENV_EMPTY";

  set_env_var(name, "");

  assert_eq!(getenv_bytes(name).as_deref(), Some([].as_slice()));
}

#[test]
fn getenv_does_not_resurrect_proc_environ_entry_after_clearenv() {
  let _env = EnvScope::new();
  let proc_name = visible_proc_environ_name();

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  assert_eq!(unsafe { clearenv() }, 0);
  assert_eq!(
    getenv_bytes_c(&proc_name),
    None,
    "getenv must not resurrect cleared entries from /proc/self/environ",
  );
}

#[test]
fn getenv_does_not_resurrect_proc_environ_entry_after_repeated_empty_lookups() {
  let _env = EnvScope::new();
  let proc_name = visible_proc_environ_name();

  // SAFETY: `clearenv` mutates process environment and takes no pointers.
  assert_eq!(unsafe { clearenv() }, 0);
  assert_eq!(getenv_bytes_c(&proc_name), None);
  assert_eq!(getenv_bytes_c(&proc_name), None);
}
