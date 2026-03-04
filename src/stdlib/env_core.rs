//! Environment core C ABI primitives.
//!
//! This module provides `getenv` and `environ` for libc-like callers.

use crate::stdlib::env_mut::{host_getenv, lookup_putenv_alias_value};
use core::ffi::c_char;
use core::ptr;
use std::ffi::CStr;

/// C ABI variable for process environment vector access.
///
/// Contract in this phase:
/// - Exported as `environ` for ABI/link compatibility.
/// - Initialized to null before startup and then bound to startup `envp`.
/// - Not automatically synchronized with host libc `environ` reallocation.
/// - Callers should prefer `getenv`/`setenv`/`unsetenv`/`putenv`/`clearenv`
///   for environment access and mutation.
#[unsafe(no_mangle)]
pub static mut environ: *mut *mut c_char = ptr::null_mut();

unsafe fn lookup_environ_value(name_bytes: &[u8]) -> *mut c_char {
  // SAFETY: Reading the process-global pointer does not dereference it.
  let mut cursor = unsafe { environ };

  if cursor.is_null() {
    return ptr::null_mut();
  }

  loop {
    // SAFETY: `cursor` points inside a NUL-terminated `char**` vector.
    let entry = unsafe { cursor.read() };

    if entry.is_null() {
      return ptr::null_mut();
    }

    // SAFETY: each non-null entry points to a NUL-terminated `NAME=VALUE` string.
    let entry_bytes = unsafe { CStr::from_ptr(entry).to_bytes() };

    if let Some(equal_pos) = entry_bytes.iter().position(|byte| *byte == b'=')
      && &entry_bytes[..equal_pos] == name_bytes
    {
      // SAFETY: `equal_pos` points at `=`, so `add(1)` lands on value bytes.
      return unsafe { entry.add(equal_pos + 1) };
    }

    // SAFETY: advance to the next entry in the same NUL-terminated pointer array.
    cursor = unsafe { cursor.add(1) };
  }
}

/// C ABI entry point for `getenv`.
///
/// Returns the value pointer for `name` from process environment.
/// Lookup order:
/// 1. active `putenv` alias tracking (caller-buffer semantics)
/// 2. host libc `getenv` when symbol resolution is available
/// 3. process `environ` vector fallback when host lookup is unavailable
///
/// Returns null when `name` is null, invalid, or not found.
///
/// # Safety
/// `name` may be null. If non-null, it must point to a valid NUL-terminated
/// C string.
///
/// # Errors
/// This implementation does not modify `errno`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv(name: *const c_char) -> *mut c_char {
  if name.is_null() {
    return ptr::null_mut();
  }

  // SAFETY: The caller contract guarantees `name` points to a NUL-terminated C string.
  let name_bytes = unsafe { CStr::from_ptr(name).to_bytes() };

  if name_bytes.is_empty() || name_bytes.contains(&b'=') {
    return ptr::null_mut();
  }

  if let Some(alias_value) = lookup_putenv_alias_value(name_bytes) {
    return alias_value;
  }

  // SAFETY: `name` remains valid for the duration of this call.
  if let Some(host_value) = unsafe { host_getenv(name) } {
    return host_value;
  }

  // SAFETY: `name` remains valid for the duration of this call.
  unsafe { lookup_environ_value(name_bytes) }
}

#[cfg(test)]
mod tests {
  use crate::stdlib::env_mut::force_host_env_unavailable_for_test;
  use crate::stdlib::lock_environ_for_test;
  use core::ffi::c_char;
  use core::ptr;
  use std::env;
  use std::ffi::{CStr, CString};

  use super::{environ, getenv, lookup_environ_value};

  struct EnvironReset {
    previous: *mut *mut c_char,
  }

  impl EnvironReset {
    fn capture() -> Self {
      // SAFETY: Reading the raw global pointer does not dereference it.
      let previous = unsafe { environ };

      Self { previous }
    }
  }

  impl Drop for EnvironReset {
    fn drop(&mut self) {
      // SAFETY: Restoring the saved global pointer keeps test side effects local.
      unsafe {
        environ = self.previous;
      }
    }
  }

  #[test]
  fn getenv_returns_null_for_null_name_pointer() {
    // SAFETY: `getenv` explicitly accepts a null pointer and returns null.
    let value_ptr = unsafe { getenv(ptr::null()) };

    assert!(value_ptr.is_null());
  }

  #[test]
  fn getenv_rejects_name_with_equal_sign() {
    let invalid_name = CString::new("A=B").expect("CString::new failed for invalid_name");

    // SAFETY: `invalid_name` is a valid NUL-terminated C string.
    let value_ptr = unsafe { getenv(invalid_name.as_ptr()) };

    assert!(value_ptr.is_null());
  }

  #[test]
  fn getenv_rejects_empty_name() {
    let empty_name = CString::new("").expect("CString::new failed for empty_name");

    // SAFETY: `empty_name` is a valid NUL-terminated C string.
    let value_ptr = unsafe { getenv(empty_name.as_ptr()) };

    assert!(value_ptr.is_null());
  }

  #[test]
  fn getenv_reads_existing_variable_value() {
    let _guard = lock_environ_for_test();
    let name = CString::new("RLIBC_I016_ENV_CORE_UNIT").expect("CString::new failed for name");
    // SAFETY: environment mutation is serialized by `lock_environ_for_test`.
    unsafe {
      env::set_var("RLIBC_I016_ENV_CORE_UNIT", "value");
    }

    // SAFETY: `name` is a valid NUL-terminated key.
    let value_ptr = unsafe { getenv(name.as_ptr()) };

    assert!(!value_ptr.is_null());
    // SAFETY: `getenv` returns a valid NUL-terminated value pointer when non-null.
    let value = unsafe { CStr::from_ptr(value_ptr) }.to_bytes().to_vec();

    assert_eq!(value, b"value");

    // SAFETY: environment mutation is serialized by `lock_environ_for_test`.
    unsafe {
      env::remove_var("RLIBC_I016_ENV_CORE_UNIT");
    }
  }

  #[test]
  fn environ_placeholder_can_be_forced_to_null() {
    let _guard = lock_environ_for_test();
    let _environ_reset = EnvironReset::capture();

    // SAFETY: test-local reset; `EnvironReset` restores prior value on drop.
    unsafe {
      environ = ptr::null_mut();
    }

    // SAFETY: Reading the raw placeholder pointer does not dereference it.
    let environ_ptr = unsafe { environ };

    assert!(environ_ptr.is_null());
  }

  #[test]
  fn lookup_environ_value_returns_value_for_matching_entry() {
    let _guard = lock_environ_for_test();
    let _environ_reset = EnvironReset::capture();
    let mut first_entry = b"RLIBC_I016_ENV_SCAN_FIRST=first\0".to_vec();
    let mut target_entry = b"RLIBC_I016_ENV_SCAN_TARGET=expected\0".to_vec();
    let mut envp = [
      first_entry.as_mut_ptr().cast::<c_char>(),
      target_entry.as_mut_ptr().cast::<c_char>(),
      ptr::null_mut(),
    ];
    let name = b"RLIBC_I016_ENV_SCAN_TARGET";

    // SAFETY: `envp` references valid test-owned `NAME=VALUE` C strings and
    // remains alive for this lookup.
    unsafe {
      environ = envp.as_mut_ptr();
    }

    // SAFETY: `environ` was set above to a valid NUL-terminated vector.
    let value_ptr = unsafe { lookup_environ_value(name) };

    assert!(!value_ptr.is_null());
    // SAFETY: returned pointer references a NUL-terminated value in `target_entry`.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) }
      .to_bytes()
      .to_vec();

    assert_eq!(value, b"expected");
  }

  #[test]
  fn getenv_falls_back_to_environ_when_host_getenv_is_unavailable() {
    let _guard = lock_environ_for_test();
    let _host_unavailable = force_host_env_unavailable_for_test();
    let _environ_reset = EnvironReset::capture();
    let mut target_entry = b"RLIBC_I016_GETENV_FALLBACK=from_environ\0".to_vec();
    let mut envp = [target_entry.as_mut_ptr().cast::<c_char>(), ptr::null_mut()];
    let name =
      CString::new("RLIBC_I016_GETENV_FALLBACK").expect("CString::new failed for fallback key");

    // SAFETY: `envp` references a valid NUL-terminated environment vector.
    unsafe {
      environ = envp.as_mut_ptr();
    }

    // SAFETY: `name` is a valid NUL-terminated key.
    let value_ptr = unsafe { getenv(name.as_ptr()) };

    assert!(!value_ptr.is_null());
    // SAFETY: `getenv` returned a value pointer within `target_entry`.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) }
      .to_bytes()
      .to_vec();

    assert_eq!(value, b"from_environ");
  }
}
