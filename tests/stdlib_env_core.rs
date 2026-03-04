use rlibc::stdlib::env::core::getenv;
use std::env;
use std::ffi::{CStr, CString};
use std::sync::{Mutex, MutexGuard, PoisonError};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvCleanup<'a> {
  names: &'a [&'a str],
}

impl<'a> EnvCleanup<'a> {
  const fn new(names: &'a [&'a str]) -> Self {
    Self { names }
  }
}

impl Drop for EnvCleanup<'_> {
  fn drop(&mut self) {
    for name in self.names {
      // SAFETY: environment mutation is serialized by `ENV_LOCK` in each test.
      unsafe { env::remove_var(name) };
    }
  }
}

fn lock_env() -> MutexGuard<'static, ()> {
  ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn getenv_bytes(name: &str) -> Option<Vec<u8>> {
  let c_name = CString::new(name).expect("name must not contain interior NUL bytes");
  // SAFETY: `c_name` is a valid NUL-terminated C string pointer.
  let value_ptr = unsafe { getenv(c_name.as_ptr()) }.cast_const();

  if value_ptr.is_null() {
    return None;
  }

  // SAFETY: `getenv` returns a pointer to a valid NUL-terminated value string.
  Some(unsafe { CStr::from_ptr(value_ptr) }.to_bytes().to_vec())
}

fn set_env_var(name: &str, value: &str) {
  // SAFETY: environment mutation is serialized by `ENV_LOCK` in each test.
  unsafe { env::set_var(name, value) };
}

#[test]
fn getenv_returns_value_for_exact_name_match() {
  let _guard = lock_env();
  let exact_name = "RLIBC_I016_GETENV_EXACT";
  let suffix_name = "RLIBC_I016_GETENV_EXACT_SUFFIX";
  let cleanup_names = [exact_name, suffix_name];
  let _cleanup = EnvCleanup::new(&cleanup_names);

  set_env_var(exact_name, "expected");
  set_env_var(suffix_name, "unexpected");

  assert_eq!(
    getenv_bytes(exact_name).as_deref(),
    Some(b"expected".as_slice())
  );
}

#[test]
fn getenv_returns_none_for_undefined_variable() {
  let _guard = lock_env();
  let name = "RLIBC_I016_GETENV_UNDEFINED";
  let cleanup_names = [name];
  let _cleanup = EnvCleanup::new(&cleanup_names);

  // SAFETY: environment mutation is serialized by `ENV_LOCK` in each test.
  unsafe { env::remove_var(name) };

  assert_eq!(getenv_bytes(name), None);
}

#[test]
fn getenv_returns_empty_string_for_empty_value() {
  let _guard = lock_env();
  let name = "RLIBC_I016_GETENV_EMPTY";
  let cleanup_names = [name];
  let _cleanup = EnvCleanup::new(&cleanup_names);

  set_env_var(name, "");

  assert_eq!(getenv_bytes(name).as_deref(), Some([].as_slice()));
}
