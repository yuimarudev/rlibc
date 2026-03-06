//! Environment core C ABI primitives.
//!
//! This module provides `getenv` and `environ` for libc-like callers.

use crate::errno::__errno_location;
use crate::stdlib::env_mut::{
  ensure_owned_environ_initialized_for_lookup, lock_environ_state, lookup_putenv_alias_value,
  owned_environ_initialized_for_lookup,
};
use core::ffi::c_char;
use core::ptr;
use std::cell::RefCell;
use std::ffi::{CStr, CString};

thread_local! {
  static PROC_ENV_FALLBACK_VALUE: RefCell<Option<CString>> = const { RefCell::new(None) };
}

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

fn lookup_proc_environ_value(name_bytes: &[u8]) -> *mut c_char {
  let Ok(contents) = std::fs::read("/proc/self/environ") else {
    return ptr::null_mut();
  };

  for entry in contents.split(|byte| *byte == 0) {
    if entry.is_empty() {
      continue;
    }

    let Some(equal_pos) = entry.iter().position(|byte| *byte == b'=') else {
      continue;
    };

    if &entry[..equal_pos] != name_bytes {
      continue;
    }

    let value_bytes = &entry[equal_pos + 1..];
    let owned_value =
      CString::new(value_bytes).unwrap_or_else(|_| unreachable!("split entry cannot contain NUL"));

    return PROC_ENV_FALLBACK_VALUE.with(|slot| {
      let mut slot = slot.borrow_mut();

      *slot = Some(owned_value);

      slot
        .as_ref()
        .map_or(ptr::null_mut(), |value| value.as_ptr().cast_mut())
    });
  }

  ptr::null_mut()
}

/// C ABI entry point for `getenv`.
///
/// Returns the value pointer for `name` from process environment.
/// Lookup order:
/// 1. active `putenv` alias tracking (caller-buffer semantics)
/// 2. process `environ` vector (bootstrapped from host environment snapshot
///    when internal `environ` is still null)
/// 3. `/proc/self/environ` fallback only before the owned snapshot has been
///    initialized
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

  let _guard = lock_environ_state();

  // SAFETY: The caller contract guarantees `name` points to a NUL-terminated C string.
  let name_bytes = unsafe { CStr::from_ptr(name).to_bytes() };

  if name_bytes.is_empty() || name_bytes.contains(&b'=') {
    return ptr::null_mut();
  }

  if let Some(alias_value) = lookup_putenv_alias_value(name_bytes) {
    return alias_value;
  }

  // Lazily bootstrap the owned environment snapshot for callers that invoke
  // `getenv` before startup has bound `environ`.
  let owned_snapshot_was_initialized = owned_environ_initialized_for_lookup();

  if unsafe { environ }.is_null() && !owned_snapshot_was_initialized {
    ensure_owned_environ_initialized_for_lookup();
  }

  // SAFETY: `name` remains valid for the duration of this call.
  let environ_value = unsafe { lookup_environ_value(name_bytes) };

  if !environ_value.is_null() {
    return environ_value;
  }

  // SAFETY: reading raw pointer value without dereferencing.
  if unsafe { environ }.is_null() && !owned_environ_initialized_for_lookup() {
    // SAFETY: `__errno_location` returns readable/writable TLS errno.
    let saved_errno = unsafe { __errno_location().read() };
    let fallback_value = lookup_proc_environ_value(name_bytes);
    // SAFETY: preserve `getenv` contract that leaves errno unchanged.
    unsafe {
      __errno_location().write(saved_errno);
    }

    return fallback_value;
  }

  ptr::null_mut()
}

#[cfg(test)]
mod tests {
  use crate::errno::__errno_location;
  use crate::stdlib::env_mut::{
    clearenv, force_owned_environ_empty_for_test, reset_owned_environ_for_test,
  };
  use crate::stdlib::lock_environ_for_test;
  use core::ffi::c_char;
  use core::ptr;
  use std::env;
  use std::ffi::{CStr, CString};

  use super::{environ, getenv, lookup_environ_value};

  struct OwnedEnvironReset;

  impl Drop for OwnedEnvironReset {
    fn drop(&mut self) {
      reset_owned_environ_for_test();
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
    let _owned_reset = OwnedEnvironReset;
    let name = CString::new("RLIBC_I016_ENV_CORE_UNIT").expect("CString::new failed for name");

    reset_owned_environ_for_test();

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
    let _owned_reset = OwnedEnvironReset;

    reset_owned_environ_for_test();

    // SAFETY: test-local reset while environment access is serialized.
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
    let _owned_reset = OwnedEnvironReset;
    let mut first_entry = b"RLIBC_I016_ENV_SCAN_FIRST=first\0".to_vec();
    let mut target_entry = b"RLIBC_I016_ENV_SCAN_TARGET=expected\0".to_vec();
    let mut envp = [
      first_entry.as_mut_ptr().cast::<c_char>(),
      target_entry.as_mut_ptr().cast::<c_char>(),
      ptr::null_mut(),
    ];
    let name = b"RLIBC_I016_ENV_SCAN_TARGET";

    reset_owned_environ_for_test();

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
  fn getenv_reads_from_environ_when_present() {
    let _guard = lock_environ_for_test();
    let _owned_reset = OwnedEnvironReset;
    let mut target_entry = b"RLIBC_I016_GETENV_FALLBACK=from_environ\0".to_vec();
    let mut envp = [target_entry.as_mut_ptr().cast::<c_char>(), ptr::null_mut()];
    let name =
      CString::new("RLIBC_I016_GETENV_FALLBACK").expect("CString::new failed for fallback key");

    reset_owned_environ_for_test();

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

  #[test]
  fn getenv_returns_null_when_environ_is_empty_and_variable_is_absent() {
    let _guard = lock_environ_for_test();
    let _owned_reset = OwnedEnvironReset;
    let name =
      CString::new("RLIBC_I016_GETENV_ABSENT").expect("CString::new failed for absent key");

    reset_owned_environ_for_test();

    // SAFETY: test-local setup while environment access is serialized.
    unsafe {
      env::remove_var("RLIBC_I016_GETENV_ABSENT");
      environ = ptr::null_mut();
    }

    // SAFETY: `name` is a valid NUL-terminated key.
    let value_ptr = unsafe { getenv(name.as_ptr()) };

    assert!(value_ptr.is_null());
  }

  #[test]
  fn getenv_bootstraps_environ_snapshot_when_internal_environ_is_empty() {
    let _guard = lock_environ_for_test();
    let _owned_reset = OwnedEnvironReset;

    reset_owned_environ_for_test();

    let Some((bootstrap_name, _)) = env::vars_os().next() else {
      return;
    };
    let bootstrap_name = bootstrap_name
      .into_string()
      .expect("bootstrap variable name should be valid UTF-8 in this test");
    let name = CString::new(bootstrap_name).expect("CString::new failed for bootstrap variable");

    // SAFETY: test-local setup while environment access is serialized.
    unsafe {
      environ = ptr::null_mut();
      __errno_location().write(67);
    }

    // SAFETY: `name` is a valid NUL-terminated key.
    let value_ptr = unsafe { getenv(name.as_ptr()) };

    assert!(!value_ptr.is_null());
    // SAFETY: `getenv` preserves errno on successful lookup.
    assert_eq!(unsafe { __errno_location().read() }, 67);
  }

  #[test]
  fn getenv_does_not_fall_back_to_proc_environ_after_clearenv() {
    let _guard = lock_environ_for_test();
    let _owned_reset = OwnedEnvironReset;

    reset_owned_environ_for_test();

    let proc_environ = std::fs::read("/proc/self/environ")
      .expect("expected /proc/self/environ to be readable for getenv fallback test");
    let first_entry = proc_environ
      .split(|byte| *byte == 0)
      .find(|entry| !entry.is_empty() && entry.contains(&b'='))
      .expect("expected at least one proc environ entry");
    let equal_pos = first_entry
      .iter()
      .position(|byte| *byte == b'=')
      .expect("proc environ entry must contain '='");
    let name = CString::new(first_entry[..equal_pos].to_vec())
      .expect("proc environ name must not contain interior NUL");

    force_owned_environ_empty_for_test();
    assert_eq!(clearenv(), 0, "clearenv should succeed");

    // SAFETY: `name` is still a valid NUL-terminated key.
    let after_clear = unsafe { getenv(name.as_ptr()) };

    assert!(
      after_clear.is_null(),
      "getenv must not resurrect cleared variables from /proc/self/environ",
    );
  }
}
