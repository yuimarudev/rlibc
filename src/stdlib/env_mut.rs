//! Mutable environment-variable C ABI functions.
//!
//! This module provides minimal implementations of:
//! - `setenv`
//! - `unsetenv`
//! - `putenv`
//! - `clearenv`
//!
//! Semantics focus:
//! - mutation operations are serialized by a process-wide mutex,
//! - `putenv` preserves caller-buffer aliasing for lookups via `getenv`.

use crate::abi::errno::EINVAL;
use crate::abi::types::c_int;
use crate::errno::__errno_location;
use core::ffi::{c_char, c_void};
use core::mem;
#[cfg(test)]
use core::sync::atomic::{AtomicUsize, Ordering};
use std::ffi::{CStr, CString};
use std::io;
#[cfg(test)]
use std::sync::TryLockError;
use std::sync::{Mutex, MutexGuard, OnceLock};

static ENV_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PUTENV_ALIASES: OnceLock<Mutex<Vec<PutEnvAlias>>> = OnceLock::new();
static HOST_ENV_FNS: OnceLock<Option<HostEnvFns>> = OnceLock::new();
#[cfg(test)]
static FORCE_HOST_ENV_UNAVAILABLE_FOR_TEST: AtomicUsize = AtomicUsize::new(0);
const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
const SYMBOL_GETENV: &[u8] = b"getenv\0";
const SYMBOL_SETENV: &[u8] = b"setenv\0";
const SYMBOL_UNSETENV: &[u8] = b"unsetenv\0";
const SYMBOL_CLEARENV: &[u8] = b"clearenv\0";

unsafe extern "C" {
  fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct PutEnvAlias {
  name: Box<[u8]>,
  entry_addr: usize,
}

#[derive(Clone, Copy)]
struct HostEnvFns {
  getenv: unsafe extern "C" fn(*const c_char) -> *mut c_char,
  setenv: unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> c_int,
  unsetenv: unsafe extern "C" fn(*const c_char) -> c_int,
  clearenv: unsafe extern "C" fn() -> c_int,
}

/// Test-only guard that forces host environment symbol resolution to appear unavailable.
#[cfg(test)]
pub(super) struct HostEnvUnavailableGuard;

#[cfg(test)]
impl Drop for HostEnvUnavailableGuard {
  fn drop(&mut self) {
    loop {
      let current = FORCE_HOST_ENV_UNAVAILABLE_FOR_TEST.load(Ordering::SeqCst);

      if current == 0 {
        return;
      }

      if FORCE_HOST_ENV_UNAVAILABLE_FOR_TEST
        .compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
      {
        return;
      }
    }
  }
}

fn env_mutation_guard() -> MutexGuard<'static, ()> {
  match ENV_MUTATION_LOCK.get_or_init(|| Mutex::new(())).lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn putenv_alias_guard() -> MutexGuard<'static, Vec<PutEnvAlias>> {
  match PUTENV_ALIASES.get_or_init(|| Mutex::new(Vec::new())).lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn resolve_symbol(symbol: &'static [u8]) -> Option<*mut c_void> {
  // SAFETY: `symbol` is a static NUL-terminated symbol name.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol.as_ptr().cast()) };

  if resolved.is_null() {
    return None;
  }

  Some(resolved)
}

fn resolve_host_env_fns() -> Option<HostEnvFns> {
  let getenv_ptr = resolve_symbol(SYMBOL_GETENV)?;
  let setenv_ptr = resolve_symbol(SYMBOL_SETENV)?;
  let unsetenv_ptr = resolve_symbol(SYMBOL_UNSETENV)?;
  let clearenv_ptr = resolve_symbol(SYMBOL_CLEARENV)?;

  Some(HostEnvFns {
    // SAFETY: `dlsym` returns C function addresses for the exact signatures below.
    getenv: unsafe {
      mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_char) -> *mut c_char>(getenv_ptr)
    },
    // SAFETY: `dlsym` returns C function addresses for the exact signatures below.
    setenv: unsafe {
      mem::transmute::<
        *mut c_void,
        unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> c_int,
      >(setenv_ptr)
    },
    // SAFETY: `dlsym` returns C function addresses for the exact signatures below.
    unsetenv: unsafe {
      mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_char) -> c_int>(unsetenv_ptr)
    },
    // SAFETY: `dlsym` returns C function addresses for the exact signatures below.
    clearenv: unsafe {
      mem::transmute::<*mut c_void, unsafe extern "C" fn() -> c_int>(clearenv_ptr)
    },
  })
}

fn host_env_fns() -> Option<&'static HostEnvFns> {
  #[cfg(test)]
  if FORCE_HOST_ENV_UNAVAILABLE_FOR_TEST.load(Ordering::SeqCst) > 0 {
    return None;
  }

  HOST_ENV_FNS.get_or_init(resolve_host_env_fns).as_ref()
}

/// Enables a test-only mode where host environment function lookup is disabled.
///
/// While the returned guard is alive, `host_getenv` behaves as if host resolver
/// symbols are unavailable and callers can validate fallback paths.
#[cfg(test)]
pub(super) fn force_host_env_unavailable_for_test() -> HostEnvUnavailableGuard {
  FORCE_HOST_ENV_UNAVAILABLE_FOR_TEST.fetch_add(1, Ordering::SeqCst);

  HostEnvUnavailableGuard
}

fn host_errno_or(default_errno: c_int) -> c_int {
  io::Error::last_os_error()
    .raw_os_error()
    .and_then(|value| c_int::try_from(value).ok())
    .unwrap_or(default_errno)
}

fn host_setenv(name: &CStr, value: &CStr, overwrite: c_int) -> Result<(), c_int> {
  let host = host_env_fns().ok_or(EINVAL)?;
  // SAFETY: `name`/`value` are valid NUL-terminated strings.
  let rc = unsafe { (host.setenv)(name.as_ptr(), value.as_ptr(), overwrite) };

  if rc == 0 {
    return Ok(());
  }

  Err(host_errno_or(EINVAL))
}

fn host_unsetenv(name: &CStr) -> Result<(), c_int> {
  let host = host_env_fns().ok_or(EINVAL)?;
  // SAFETY: `name` is a valid NUL-terminated string.
  let rc = unsafe { (host.unsetenv)(name.as_ptr()) };

  if rc == 0 {
    return Ok(());
  }

  Err(host_errno_or(EINVAL))
}

fn host_clearenv() -> Result<(), c_int> {
  let host = host_env_fns().ok_or(EINVAL)?;
  // SAFETY: `clearenv` takes no arguments and mutates process environment.
  let rc = unsafe { (host.clearenv)() };

  if rc == 0 {
    return Ok(());
  }

  Err(host_errno_or(EINVAL))
}

pub(super) unsafe fn host_getenv(name: *const c_char) -> Option<*mut c_char> {
  let host = host_env_fns()?;

  // SAFETY: Caller must provide a valid C string pointer or null.
  Some(unsafe { (host.getenv)(name) })
}

fn set_errno(errno_value: c_int) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage for the
  // calling thread.
  unsafe {
    errno_ptr.write(errno_value);
  }
}

fn fail_with_errno(errno_value: c_int) -> c_int {
  set_errno(errno_value);

  -1
}

fn current_errno() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage for the
  // calling thread.
  unsafe { errno_ptr.read() }
}

unsafe fn read_c_string(ptr: *const c_char) -> Result<Vec<u8>, c_int> {
  if ptr.is_null() {
    return Err(EINVAL);
  }

  // SAFETY: Caller must provide a valid pointer to a NUL-terminated C string.
  let bytes = unsafe { CStr::from_ptr(ptr).to_bytes() };

  Ok(bytes.to_vec())
}

fn validate_name_bytes(name_bytes: &[u8]) -> Result<(), c_int> {
  if name_bytes.is_empty() || name_bytes.contains(&b'=') {
    return Err(EINVAL);
  }

  Ok(())
}

fn remove_putenv_alias(name_bytes: &[u8]) {
  let mut aliases = putenv_alias_guard();

  aliases.retain(|alias| alias.name.as_ref() != name_bytes);
}

fn clear_putenv_aliases() {
  let mut aliases = putenv_alias_guard();

  aliases.clear();
}

fn update_putenv_alias(name_bytes: &[u8], entry_ptr: *mut c_char) {
  let entry_addr = entry_ptr as usize;
  let mut aliases = putenv_alias_guard();

  aliases.retain(|alias| alias.name.as_ref() != name_bytes);

  aliases.push(PutEnvAlias {
    name: name_bytes.to_vec().into_boxed_slice(),
    entry_addr,
  });
}

const fn parse_alias_value_ptr_impl(
  entry_ptr: *mut c_char,
  expected_name: &[u8],
) -> Option<*mut c_char> {
  if entry_ptr.is_null() {
    return None;
  }

  let mut cursor = entry_ptr.cast_const();
  let mut index = 0;

  while index < expected_name.len() {
    // SAFETY: `entry_ptr` is expected to point to a valid `NAME=VALUE\0` buffer.
    let byte = unsafe { cursor.read().to_ne_bytes()[0] };

    if byte != expected_name[index] {
      return None;
    }

    // SAFETY: We advance inside the same NUL-terminated string.
    cursor = unsafe { cursor.add(1) };
    index += 1;
  }

  // SAFETY: `cursor` points at the byte that should follow the expected name.
  let separator = unsafe { cursor.read().to_ne_bytes()[0] };

  if separator != b'=' {
    return None;
  }

  // SAFETY: `cursor` currently points at `=`, so `add(1)` is in-bounds for
  // the same NUL-terminated string.
  Some(unsafe { cursor.add(1).cast_mut() })
}

#[cfg(not(test))]
const fn parse_alias_value_ptr(
  entry_ptr: *mut c_char,
  expected_name: &[u8],
) -> Option<*mut c_char> {
  parse_alias_value_ptr_impl(entry_ptr, expected_name)
}

#[cfg(test)]
fn parse_alias_value_ptr(entry_ptr: *mut c_char, expected_name: &[u8]) -> Option<*mut c_char> {
  assert_lookup_keeps_alias_lock_during_parse();

  parse_alias_value_ptr_impl(entry_ptr, expected_name)
}

#[cfg(test)]
fn assert_lookup_keeps_alias_lock_during_parse() {
  let lock = PUTENV_ALIASES.get_or_init(|| Mutex::new(Vec::new()));

  match lock.try_lock() {
    Err(TryLockError::WouldBlock) => {}
    Err(TryLockError::Poisoned(poisoned)) => {
      let guard = poisoned.into_inner();

      drop(guard);
      panic!("putenv alias lock was poisoned before lookup parse contract check");
    }
    Ok(guard) => {
      drop(guard);
      panic!("lookup_putenv_alias_value must parse while holding alias lock");
    }
  }
}

pub(super) fn lookup_putenv_alias_value(name_bytes: &[u8]) -> Option<*mut c_char> {
  let mut aliases = putenv_alias_guard();
  let mut resolved_alias_index = None;
  let mut resolved_value_ptr = None;
  let mut remove_indices = Vec::new();

  for (alias_index, alias) in aliases.iter().enumerate() {
    if alias.name.as_ref() != name_bytes {
      continue;
    }

    let entry_ptr = alias.entry_addr as *mut c_char;
    let value_ptr = parse_alias_value_ptr(entry_ptr, name_bytes);

    if let Some(value_ptr) = value_ptr {
      if let Some(previous_index) = resolved_alias_index {
        remove_indices.push(previous_index);
      }

      resolved_alias_index = Some(alias_index);
      resolved_value_ptr = Some(value_ptr);
    } else {
      remove_indices.push(alias_index);
    }
  }

  remove_indices.sort_unstable();
  remove_indices.dedup();

  for alias_index in remove_indices.into_iter().rev() {
    aliases.remove(alias_index);
  }

  resolved_value_ptr
}

/// C ABI entry point for `setenv`.
///
/// Sets environment variable `name` to `value`.
/// - If `overwrite == 0` and `name` already exists, the existing value is kept.
/// - If `overwrite != 0`, existing value is replaced.
/// - If an existing value was introduced via `putenv` aliasing and
///   `overwrite == 0`, that aliasing remains active.
///
/// Returns `0` on success. Returns `-1` and sets `errno` on failure.
///
/// # Errors
/// - `EINVAL`: `name`/`value` is null, `name` is empty, or `name` contains `=`.
///
/// On success, this implementation preserves the caller's current `errno`.
/// On failure, this implementation leaves existing `putenv` alias tracking
/// unchanged.
///
/// # Safety
/// - `name` and `value` must be valid pointers to NUL-terminated strings.
/// - Caller must avoid concurrent process-environment access outside this
///   module's synchronized entry points.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setenv(
  name: *const c_char,
  value: *const c_char,
  overwrite: c_int,
) -> c_int {
  // SAFETY: Validity requirements are documented in this function's `# Safety`.
  let name_bytes = match unsafe { read_c_string(name) } {
    Ok(bytes) => bytes,
    Err(errno_value) => return fail_with_errno(errno_value),
  };

  // SAFETY: Validity requirements are documented in this function's `# Safety`.
  let value_bytes = match unsafe { read_c_string(value) } {
    Ok(bytes) => bytes,
    Err(errno_value) => return fail_with_errno(errno_value),
  };

  if let Err(errno_value) = validate_name_bytes(&name_bytes) {
    return fail_with_errno(errno_value);
  }

  let name_c = CString::new(name_bytes.clone()).unwrap_or_else(|_| unreachable!("validated name"));
  let value_c = CString::new(value_bytes).unwrap_or_else(|_| unreachable!("validated value"));
  let previous_errno = current_errno();
  let _guard = env_mutation_guard();
  let existed_before = if overwrite == 0 {
    // SAFETY: `name_c` is a valid NUL-terminated key for host lookup.
    unsafe { host_getenv(name_c.as_ptr()) }.is_some_and(|value_ptr| !value_ptr.is_null())
  } else {
    false
  };

  if let Err(errno_value) = host_setenv(&name_c, &value_c, overwrite) {
    return fail_with_errno(errno_value);
  }

  if overwrite != 0 || !existed_before {
    remove_putenv_alias(&name_bytes);
  }

  set_errno(previous_errno);

  0
}

/// C ABI entry point for `unsetenv`.
///
/// Removes environment variable `name`.
/// Returns `0` on success. Returns `-1` and sets `errno` on failure.
///
/// # Errors
/// - `EINVAL`: `name` is null, empty, or contains `=`.
///
/// On success, this implementation preserves the caller's current `errno`.
/// On failure, this implementation leaves existing `putenv` alias tracking
/// unchanged.
///
/// # Safety
/// - `name` must be a valid pointer to a NUL-terminated string.
/// - Caller must avoid concurrent process-environment access outside this
///   module's synchronized entry points.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unsetenv(name: *const c_char) -> c_int {
  // SAFETY: Validity requirements are documented in this function's `# Safety`.
  let name_bytes = match unsafe { read_c_string(name) } {
    Ok(bytes) => bytes,
    Err(errno_value) => return fail_with_errno(errno_value),
  };

  if let Err(errno_value) = validate_name_bytes(&name_bytes) {
    return fail_with_errno(errno_value);
  }

  let name_c = CString::new(name_bytes.clone()).unwrap_or_else(|_| unreachable!("validated name"));
  let previous_errno = current_errno();
  let _guard = env_mutation_guard();

  if let Err(errno_value) = host_unsetenv(&name_c) {
    return fail_with_errno(errno_value);
  }

  remove_putenv_alias(&name_bytes);
  set_errno(previous_errno);

  0
}

/// C ABI entry point for `putenv`.
///
/// Applies a `NAME=VALUE` style string to the process environment.
/// - If `string` contains `=`, sets `NAME` to `VALUE` and preserves aliasing:
///   later caller-buffer updates are visible to `getenv(NAME)`.
///   Reapplying the same `NAME` rebinds alias tracking to the latest buffer.
/// - If `string` has no `=`, removes `NAME`.
///
/// Returns `0` on success. Returns `-1` and sets `errno` on failure.
///
/// # Errors
/// - `EINVAL`: `string` is null, empty, or encodes an invalid name.
///
/// On success, this implementation preserves the caller's current `errno`.
/// On failure, this implementation leaves existing `putenv` alias tracking
/// unchanged.
///
/// # Safety
/// - `string` must be a valid pointer to a NUL-terminated string.
/// - When `string` contains `=`, the pointed buffer must remain valid as long
///   as aliasing behavior is required by callers.
/// - Caller must avoid concurrent process-environment access outside this
///   module's synchronized entry points.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putenv(string: *mut c_char) -> c_int {
  // SAFETY: Validity requirements are documented in this function's `# Safety`.
  let bytes = match unsafe { read_c_string(string.cast_const()) } {
    Ok(bytes) => bytes,
    Err(errno_value) => return fail_with_errno(errno_value),
  };
  let previous_errno = current_errno();
  let _guard = env_mutation_guard();

  if let Some(eq_pos) = bytes.iter().position(|byte| *byte == b'=') {
    let name_bytes = &bytes[..eq_pos];
    let value_bytes = &bytes[eq_pos + 1..];

    if let Err(errno_value) = validate_name_bytes(name_bytes) {
      return fail_with_errno(errno_value);
    }

    let name_c = CString::new(name_bytes).unwrap_or_else(|_| unreachable!("validated name"));
    let value_c = CString::new(value_bytes).unwrap_or_else(|_| unreachable!("validated value"));

    if let Err(errno_value) = host_setenv(&name_c, &value_c, 1) {
      return fail_with_errno(errno_value);
    }

    update_putenv_alias(name_bytes, string);

    set_errno(previous_errno);

    return 0;
  }

  if let Err(errno_value) = validate_name_bytes(&bytes) {
    return fail_with_errno(errno_value);
  }

  let name_c = CString::new(bytes).unwrap_or_else(|_| unreachable!("validated name"));

  if let Err(errno_value) = host_unsetenv(&name_c) {
    return fail_with_errno(errno_value);
  }

  remove_putenv_alias(name_c.to_bytes());
  set_errno(previous_errno);

  0
}

/// C ABI entry point for `clearenv`.
///
/// Removes every environment variable currently visible to this process.
/// Returns `0` on success. Returns `-1` and sets `errno` on failure.
///
/// # Errors
/// Propagates host `clearenv` failures via `errno`.
///
/// On success, this implementation preserves the caller's current `errno`.
/// On failure, this implementation leaves existing `putenv` alias tracking
/// unchanged.
///
/// # Safety
/// Caller must avoid concurrent process-environment access outside this
/// module's synchronized entry points.
#[unsafe(no_mangle)]
pub extern "C" fn clearenv() -> c_int {
  let previous_errno = current_errno();
  let _guard = env_mutation_guard();

  if let Err(errno_value) = host_clearenv() {
    return fail_with_errno(errno_value);
  }

  clear_putenv_aliases();
  set_errno(previous_errno);

  0
}

#[cfg(test)]
mod tests {
  use std::ffi::{CStr, CString};
  use std::sync::{Mutex, OnceLock};

  use crate::abi::errno::EINVAL;
  use crate::errno::__errno_location;
  use crate::stdlib::env_mut::{
    clearenv, force_host_env_unavailable_for_test, host_env_fns, host_getenv, host_unsetenv,
    lookup_putenv_alias_value, putenv, remove_putenv_alias, setenv, unsetenv,
  };

  static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  fn test_lock() -> &'static Mutex<()> {
    TEST_LOCK.get_or_init(|| Mutex::new(()))
  }

  fn read_errno() -> i32 {
    // SAFETY: `__errno_location` returns valid TLS for this thread.
    unsafe { __errno_location().read() }
  }

  fn write_errno(value: i32) {
    // SAFETY: `__errno_location` returns writable TLS for this thread.
    unsafe {
      __errno_location().write(value);
    }
  }

  #[test]
  fn lookup_putenv_alias_value_tracks_buffer_updates() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_TRACK_TEST";
    let mut entry = b"RLIBC_I017_ALIAS_TRACK_TEST=alpha\0".to_vec();

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(rc, 0);

    let value_offset = b"RLIBC_I017_ALIAS_TRACK_TEST=".len();

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must exist");
    // SAFETY: alias pointer references the same NUL-terminated entry buffer.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(value.to_bytes(), b"omega");
  }

  #[test]
  fn lookup_putenv_alias_value_holds_alias_lock_while_parsing() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_LOCK_PARSE";
    let mut entry = b"RLIBC_I017_ALIAS_LOCK_PARSE=value\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);

    let value_ptr =
      lookup_putenv_alias_value(key).expect("alias lookup must succeed while lock is held");
    // SAFETY: alias pointer references test-owned NUL-terminated buffer.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(value.to_bytes(), b"value");
  }

  #[test]
  fn lookup_putenv_alias_value_rejects_entry_with_renamed_name_prefix() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_RENAME_PREFIX";
    let mut entry = b"RLIBC_I017_ALIAS_RENAME_PREFIX=value\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[0] = b'X';

    assert!(lookup_putenv_alias_value(key).is_none());
  }

  #[test]
  fn lookup_putenv_alias_value_drops_stale_alias_after_name_prefix_mismatch() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_RENAME_DROP";
    let mut entry = b"RLIBC_I017_ALIAS_RENAME_DROP=value\0".to_vec();
    let original_first = entry[0];

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[0] = b'X';

    assert!(lookup_putenv_alias_value(key).is_none());

    entry[0] = original_first;

    assert!(lookup_putenv_alias_value(key).is_none());
  }

  #[test]
  fn lookup_putenv_alias_value_skips_stale_duplicate_and_returns_valid_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_DUPLICATE_FALLBACK";
    let stale_entry = b"XLIBC_I017_ALIAS_DUPLICATE_FALLBACK=stale\0".to_vec();
    let valid_entry = b"RLIBC_I017_ALIAS_DUPLICATE_FALLBACK=alive\0".to_vec();
    let stale_addr = stale_entry.as_ptr() as usize;
    let valid_addr = valid_entry.as_ptr() as usize;

    remove_putenv_alias(key);

    {
      let mut aliases = super::putenv_alias_guard();

      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: stale_addr,
      });
      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: valid_addr,
      });
    }

    let value_ptr =
      lookup_putenv_alias_value(key).expect("lookup should fallback to valid duplicate alias");
    // SAFETY: `value_ptr` points into `valid_entry` NUL-terminated buffer.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(value.to_bytes(), b"alive");

    let matching: Vec<usize> = super::putenv_alias_guard()
      .iter()
      .filter(|alias| alias.name.as_ref() == key)
      .map(|alias| alias.entry_addr)
      .collect();

    assert_eq!(matching, vec![valid_addr]);
  }

  #[test]
  fn lookup_putenv_alias_value_prefers_latest_valid_duplicate_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_DUPLICATE_LATEST";
    let first_entry = b"RLIBC_I017_ALIAS_DUPLICATE_LATEST=old\0".to_vec();
    let second_entry = b"RLIBC_I017_ALIAS_DUPLICATE_LATEST=new\0".to_vec();
    let first_addr = first_entry.as_ptr() as usize;
    let second_addr = second_entry.as_ptr() as usize;

    remove_putenv_alias(key);

    {
      let mut aliases = super::putenv_alias_guard();

      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: first_addr,
      });
      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: second_addr,
      });
    }

    let value_ptr =
      lookup_putenv_alias_value(key).expect("lookup should resolve the latest valid duplicate");
    // SAFETY: `value_ptr` points into `second_entry` NUL-terminated buffer.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(value.to_bytes(), b"new");

    let matching: Vec<usize> = {
      let aliases = super::putenv_alias_guard();

      aliases
        .iter()
        .filter(|alias| alias.name.as_ref() == key)
        .map(|alias| alias.entry_addr)
        .collect()
    };

    assert_eq!(matching, vec![second_addr]);
  }

  #[test]
  fn lookup_putenv_alias_value_prunes_interleaved_stale_and_keeps_latest_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_INTERLEAVED_STALE";
    let first_entry = b"RLIBC_I017_ALIAS_INTERLEAVED_STALE=first\0".to_vec();
    let stale_entry = b"XLIBC_I017_ALIAS_INTERLEAVED_STALE=stale\0".to_vec();
    let latest_entry = b"RLIBC_I017_ALIAS_INTERLEAVED_STALE=latest\0".to_vec();
    let first_addr = first_entry.as_ptr() as usize;
    let stale_addr = stale_entry.as_ptr() as usize;
    let latest_addr = latest_entry.as_ptr() as usize;

    remove_putenv_alias(key);

    {
      let mut aliases = super::putenv_alias_guard();

      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: first_addr,
      });
      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: stale_addr,
      });
      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: latest_addr,
      });
    }

    let value_ptr =
      lookup_putenv_alias_value(key).expect("lookup should resolve the latest valid alias");
    // SAFETY: `value_ptr` points into `latest_entry` NUL-terminated buffer.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(value.to_bytes(), b"latest");

    let matching: Vec<usize> = {
      let aliases = super::putenv_alias_guard();

      aliases
        .iter()
        .filter(|alias| alias.name.as_ref() == key)
        .map(|alias| alias.entry_addr)
        .collect()
    };

    assert_eq!(matching, vec![latest_addr]);
  }

  #[test]
  fn putenv_rejects_invalid_empty_name() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let mut entry = b"=value\0".to_vec();

    // SAFETY: `entry` points to a mutable NUL-terminated string.
    let rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(rc, -1);
  }

  #[test]
  fn putenv_without_equal_removes_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I017_ALIAS_REMOVE").expect("CString::new failed");
    let mut entry_set = b"RLIBC_I017_ALIAS_REMOVE=value\0".to_vec();
    let mut entry_unset = b"RLIBC_I017_ALIAS_REMOVE\0".to_vec();

    // SAFETY: entries are mutable and NUL-terminated.
    let rc_set = unsafe { putenv(entry_set.as_mut_ptr().cast()) };

    assert_eq!(rc_set, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    // SAFETY: entries are mutable and NUL-terminated.
    let rc_unset = unsafe { putenv(entry_unset.as_mut_ptr().cast()) };

    assert_eq!(rc_unset, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn putenv_same_name_rebinds_alias_to_latest_buffer() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I016_ALIAS_REBIND";
    let mut first = b"RLIBC_I016_ALIAS_REBIND=first\0".to_vec();
    let mut second = b"RLIBC_I016_ALIAS_REBIND=second\0".to_vec();

    // SAFETY: `first` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(first.as_mut_ptr().cast()) }, 0);

    // SAFETY: `second` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(second.as_mut_ptr().cast()) }, 0);

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must be rebound");
    // SAFETY: alias pointer references latest `NAME=VALUE` entry buffer.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(value.to_bytes(), b"second");
  }

  #[test]
  fn putenv_rebind_collapses_duplicate_alias_entries() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_PUTENV_REBIND_COLLAPSE";
    let stale_entry = b"XLIBC_I017_PUTENV_REBIND_COLLAPSE=stale\0".to_vec();
    let old_entry = b"RLIBC_I017_PUTENV_REBIND_COLLAPSE=old\0".to_vec();
    let stale_addr = stale_entry.as_ptr() as usize;
    let old_addr = old_entry.as_ptr() as usize;
    let mut latest_entry = b"RLIBC_I017_PUTENV_REBIND_COLLAPSE=latest\0".to_vec();
    let latest_addr = latest_entry.as_ptr() as usize;

    remove_putenv_alias(key);

    {
      let mut aliases = super::putenv_alias_guard();

      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: stale_addr,
      });
      aliases.push(super::PutEnvAlias {
        name: key.to_vec().into_boxed_slice(),
        entry_addr: old_addr,
      });
    }

    // SAFETY: `latest_entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(latest_entry.as_mut_ptr().cast()) }, 0);

    let matching: Vec<usize> = {
      let aliases = super::putenv_alias_guard();

      aliases
        .iter()
        .filter(|alias| alias.name.as_ref() == key)
        .map(|alias| alias.entry_addr)
        .collect()
    };

    assert_eq!(matching, vec![latest_addr]);

    let value_ptr =
      lookup_putenv_alias_value(key).expect("lookup should resolve latest putenv alias value");
    // SAFETY: `value_ptr` points into `latest_entry` NUL-terminated buffer.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(value.to_bytes(), b"latest");
  }

  #[test]
  fn host_env_unavailable_guard_is_nestable() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let baseline_available = host_env_fns().is_some();
    let outer = force_host_env_unavailable_for_test();

    assert!(host_env_fns().is_none());

    {
      let inner = force_host_env_unavailable_for_test();

      assert!(host_env_fns().is_none());
      drop(inner);
    }

    assert!(host_env_fns().is_none());
    drop(outer);
    assert_eq!(host_env_fns().is_some(), baseline_available);
  }

  #[test]
  fn putenv_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_FAIL_PATH";
    let mut entry = b"RLIBC_I017_ALIAS_FAIL_PATH=value\0".to_vec();
    let _host_unavailable = force_host_env_unavailable_for_test();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(rc, -1);
    assert!(lookup_putenv_alias_value(key).is_none());
  }

  #[test]
  fn putenv_failure_sets_errno_and_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_ALIAS_FAIL_ERRNO_PATH";
    let mut entry = b"RLIBC_I017_ALIAS_FAIL_ERRNO_PATH=value\0".to_vec();

    remove_putenv_alias(key);
    write_errno(11);

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_none());
  }

  #[test]
  fn putenv_failure_sets_errno_and_preserves_existing_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_PUTENV_FAIL_ERRNO_ALIAS";
    let value_offset = b"RLIBC_I017_PUTENV_FAIL_ERRNO_ALIAS=".len();
    let mut first = b"RLIBC_I017_PUTENV_FAIL_ERRNO_ALIAS=alpha\0".to_vec();
    let mut second = b"RLIBC_I017_PUTENV_FAIL_ERRNO_ALIAS=omega\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `first` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(first.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(19);

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `second` points to a mutable NUL-terminated `NAME=VALUE` string.
    let rc = unsafe { putenv(second.as_mut_ptr().cast()) };

    assert_eq!(rc, -1);
    assert_eq!(read_errno(), EINVAL);

    first[value_offset..value_offset + 5].copy_from_slice(b"bravo");

    let value_ptr = lookup_putenv_alias_value(key).expect("existing alias must be preserved");
    // SAFETY: `value_ptr` points inside test-owned `first` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"bravo");
  }

  #[test]
  fn putenv_null_pointer_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_PUTENV_NULL_PTR_ALIAS";
    let value_offset = b"RLIBC_I017_PUTENV_NULL_PTR_ALIAS=".len();
    let mut entry = b"RLIBC_I017_PUTENV_NULL_PTR_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(45);

    // SAFETY: null pointer is passed intentionally to validate `EINVAL` path.
    let rc = unsafe { putenv(core::ptr::null_mut()) };

    assert_eq!(rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_failure_does_not_remove_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_FAIL_ALIAS").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_FAIL_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_FAIL_ALIAS=value\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(37);

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `key` and `value` are valid NUL-terminated strings.
    let set_rc = unsafe { setenv(key.as_ptr(), value.as_ptr(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key.as_bytes()).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_no_overwrite_failure_keeps_alias_and_sets_errno() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I016_SETENV_FAIL_NO_OVERWRITE_ALIAS").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_FAIL_NO_OVERWRITE_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_FAIL_NO_OVERWRITE_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(73);

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `key` and `value` are valid NUL-terminated strings.
    let set_rc = unsafe { setenv(key.as_ptr(), value.as_ptr(), 0) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key.as_bytes()).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_no_overwrite_success_without_existing_value_drops_stale_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I016_SETENV_NO_OVERWRITE_STALE_ALIAS").expect("CString::new failed");
    let value = CString::new("fresh").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_NO_OVERWRITE_STALE_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_NO_OVERWRITE_STALE_ALIAS=stale\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    let unset_result = host_unsetenv(&key);

    assert!(unset_result.is_ok());
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    let before = unsafe { host_getenv(key.as_ptr()) }.expect("host resolver must be available");

    assert!(before.is_null());

    write_errno(91);

    // SAFETY: `key` and `value` are valid NUL-terminated strings.
    let set_rc = unsafe { setenv(key.as_ptr(), value.as_ptr(), 0) };

    assert_eq!(set_rc, 0);
    assert_eq!(read_errno(), 91);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let after = unsafe { host_getenv(key.as_ptr()) }.expect("host resolver must be available");

    assert!(!after.is_null());

    // SAFETY: `after` is a valid non-null NUL-terminated value pointer.
    let current = unsafe { CStr::from_ptr(after.cast_const()) };

    assert_eq!(current.to_bytes(), b"fresh");
  }

  #[test]
  fn setenv_overwrite_success_replaces_existing_putenv_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I016_SETENV_OVERWRITE_REPLACES_ALIAS").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_OVERWRITE_REPLACES_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_OVERWRITE_REPLACES_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(64);

    // SAFETY: `key` and `value` are valid NUL-terminated strings.
    let set_rc = unsafe { setenv(key.as_ptr(), value.as_ptr(), 1) };

    assert_eq!(set_rc, 0);
    assert_eq!(read_errno(), 64);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let current = unsafe { host_getenv(key.as_ptr()) }.expect("host resolver must be available");

    assert!(!current.is_null());

    // SAFETY: `current` is non-null and points to a NUL-terminated value.
    let value_now = unsafe { CStr::from_ptr(current.cast_const()) };

    assert_eq!(value_now.to_bytes(), b"replacement");
  }

  #[test]
  fn setenv_invalid_name_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I016_SETENV_INVALID_NAME_ALIAS";
    let value = CString::new("replacement").expect("CString::new failed");
    let invalid_name =
      CString::new("RLIBC_I016_SETENV_INVALID_NAME_ALIAS=").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_INVALID_NAME_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_INVALID_NAME_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(27);

    // SAFETY: `invalid_name` and `value` are valid NUL-terminated strings.
    let set_rc = unsafe { setenv(invalid_name.as_ptr(), value.as_ptr(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_null_name_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I016_SETENV_NULL_NAME_ALIAS";
    let value = CString::new("replacement").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_NULL_NAME_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_NULL_NAME_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(83);

    // SAFETY: passing null name pointer intentionally to validate EINVAL path.
    let set_rc = unsafe { setenv(core::ptr::null(), value.as_ptr(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_null_name_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_NULL_NAME_NO_ALIAS").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(31);

    // SAFETY: passing null name pointer intentionally to validate EINVAL path.
    let set_rc = unsafe { setenv(core::ptr::null(), value.as_ptr(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn setenv_null_name_no_overwrite_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_NULL_NAME_NO_OVERWRITE_NO_ALIAS")
      .expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(33);

    // SAFETY: passing null name pointer intentionally to validate EINVAL path.
    let set_rc = unsafe { setenv(core::ptr::null(), value.as_ptr(), 0) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn setenv_null_value_i016_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_NULL_VALUE_ALIAS").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_NULL_VALUE_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_NULL_VALUE_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(39);

    // SAFETY: passing null value pointer intentionally to validate EINVAL path.
    let set_rc = unsafe { setenv(key.as_ptr(), core::ptr::null(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key.as_bytes()).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_null_value_no_overwrite_i016_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I016_SETENV_NULL_VALUE_NO_OVERWRITE_ALIAS").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_NULL_VALUE_NO_OVERWRITE_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_NULL_VALUE_NO_OVERWRITE_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(45);

    // SAFETY: passing null value pointer intentionally to validate EINVAL path.
    let set_rc = unsafe { setenv(key.as_ptr(), core::ptr::null(), 0) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key.as_bytes()).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_null_value_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_NULL_VALUE_NO_ALIAS").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(17);

    // SAFETY: passing null value pointer intentionally to validate EINVAL path.
    let set_rc = unsafe { setenv(key.as_ptr(), core::ptr::null(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn setenv_null_value_no_overwrite_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_NULL_VALUE_NO_OVERWRITE_NO_ALIAS")
      .expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(18);

    // SAFETY: passing null value pointer intentionally to validate EINVAL path.
    let set_rc = unsafe { setenv(key.as_ptr(), core::ptr::null(), 0) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn setenv_invalid_name_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_INVALID_NAME_NO_ALIAS").expect("CString::new failed");
    let invalid_name =
      CString::new("RLIBC_I016_SETENV_INVALID_NAME_NO_ALIAS=").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(23);

    // SAFETY: passing an invalid name (contains '=') to validate `EINVAL`.
    let set_rc = unsafe { setenv(invalid_name.as_ptr(), value.as_ptr(), 0) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn setenv_invalid_name_overwrite_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_INVALID_NAME_OVERWRITE_NO_ALIAS")
      .expect("CString::new failed");
    let invalid_name = CString::new("RLIBC_I016_SETENV_INVALID_NAME_OVERWRITE_NO_ALIAS=")
      .expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(24);

    // SAFETY: passing an invalid name (contains '=') to validate `EINVAL`.
    let set_rc = unsafe { setenv(invalid_name.as_ptr(), value.as_ptr(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn setenv_empty_name_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_EMPTY_NAME_NO_ALIAS").expect("CString::new failed");
    let empty_name = CString::new("").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(26);

    // SAFETY: passing an empty name to validate `EINVAL`.
    let set_rc = unsafe { setenv(empty_name.as_ptr(), value.as_ptr(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn setenv_empty_name_no_overwrite_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_SETENV_EMPTY_NAME_NO_OVERWRITE_NO_ALIAS")
      .expect("CString::new failed");
    let empty_name = CString::new("").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(27);

    // SAFETY: passing an empty name to validate `EINVAL`.
    let set_rc = unsafe { setenv(empty_name.as_ptr(), value.as_ptr(), 0) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn setenv_empty_name_no_overwrite_i016_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I016_SETENV_EMPTY_NAME_NO_OVERWRITE_ALIAS").expect("CString::new failed");
    let empty_name = CString::new("").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_SETENV_EMPTY_NAME_NO_OVERWRITE_ALIAS=".len();
    let mut entry = b"RLIBC_I016_SETENV_EMPTY_NAME_NO_OVERWRITE_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(28);

    // SAFETY: passing an empty name to validate `EINVAL`.
    let set_rc = unsafe { setenv(empty_name.as_ptr(), value.as_ptr(), 0) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key.as_bytes()).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_null_value_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I017_SETENV_NULL_VALUE_ALIAS").expect("CString::new failed");
    let value_offset = b"RLIBC_I017_SETENV_NULL_VALUE_ALIAS=".len();
    let mut entry = b"RLIBC_I017_SETENV_NULL_VALUE_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(59);

    // SAFETY: passing null value pointer intentionally to validate `EINVAL` path.
    let set_rc = unsafe { setenv(key.as_ptr(), core::ptr::null(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key.as_bytes()).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn setenv_failure_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I017_SETENV_FAIL_ERRNO_ALIAS").expect("CString::new failed");
    let value = CString::new("replacement").expect("CString::new failed");
    let mut entry = b"RLIBC_I017_SETENV_FAIL_ERRNO_ALIAS=value\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(29);

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `key` and `value` are valid NUL-terminated strings.
    let set_rc = unsafe { setenv(key.as_ptr(), value.as_ptr(), 1) };

    assert_eq!(set_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());
  }

  #[test]
  fn unsetenv_invalid_name_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I016_UNSETENV_INVALID_NAME_NO_ALIAS").expect("CString::new failed");
    let invalid_name =
      CString::new("RLIBC_I016_UNSETENV_INVALID_NAME_NO_ALIAS=").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(31);

    // SAFETY: passing an invalid name (contains '=') to validate `EINVAL`.
    let unset_rc = unsafe { unsetenv(invalid_name.as_ptr()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn unsetenv_null_name_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_UNSETENV_NULL_NAME_NO_ALIAS").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(53);

    // SAFETY: passing null name pointer intentionally to validate `EINVAL`.
    let unset_rc = unsafe { unsetenv(core::ptr::null()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn unsetenv_null_name_i017_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I017_UNSETENV_NULL_NAME_NO_ALIAS").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(53);

    // SAFETY: passing null name pointer intentionally to validate `EINVAL`.
    let unset_rc = unsafe { unsetenv(core::ptr::null()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn unsetenv_empty_name_i016_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I016_UNSETENV_EMPTY_NAME_NO_ALIAS").expect("CString::new failed");
    let empty_name = CString::new("").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(57);

    // SAFETY: passing empty name intentionally to validate `EINVAL`.
    let unset_rc = unsafe { unsetenv(empty_name.as_ptr()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn unsetenv_empty_name_i016_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I016_UNSETENV_EMPTY_NAME_ALIAS";
    let empty_name = CString::new("").expect("CString::new failed");
    let value_offset = b"RLIBC_I016_UNSETENV_EMPTY_NAME_ALIAS=".len();
    let mut entry = b"RLIBC_I016_UNSETENV_EMPTY_NAME_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(58);

    // SAFETY: passing empty name intentionally to validate `EINVAL`.
    let unset_rc = unsafe { unsetenv(empty_name.as_ptr()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn unsetenv_empty_name_i017_failure_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I017_UNSETENV_EMPTY_NAME_NO_ALIAS").expect("CString::new failed");
    let empty_name = CString::new("").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(57);

    // SAFETY: passing empty name intentionally to validate `EINVAL`.
    let unset_rc = unsafe { unsetenv(empty_name.as_ptr()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn unsetenv_missing_name_success_preserves_alias_and_errno() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_UNSETENV_MISSING_SUCCESS_ALIAS";
    let missing_name =
      CString::new("RLIBC_I017_UNSETENV_MISSING_SUCCESS_TARGET").expect("CString::new failed");
    let value_offset = b"RLIBC_I017_UNSETENV_MISSING_SUCCESS_ALIAS=".len();
    let mut entry = b"RLIBC_I017_UNSETENV_MISSING_SUCCESS_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(73);

    // SAFETY: `missing_name` is a valid NUL-terminated environment variable name.
    let unset_rc = unsafe { unsetenv(missing_name.as_ptr()) };

    assert_eq!(unset_rc, 0);
    assert_eq!(read_errno(), 73);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn unsetenv_missing_name_success_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I017_UNSETENV_MISSING_SUCCESS_NO_ALIAS").expect("CString::new failed");
    let missing_name = CString::new("RLIBC_I017_UNSETENV_MISSING_SUCCESS_NO_ALIAS_TARGET")
      .expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(79);

    // SAFETY: `missing_name` is a valid NUL-terminated environment variable name.
    let unset_rc = unsafe { unsetenv(missing_name.as_ptr()) };

    assert_eq!(unset_rc, 0);
    assert_eq!(read_errno(), 79);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn unsetenv_missing_name_success_i016_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I016_UNSETENV_MISSING_SUCCESS_NO_ALIAS").expect("CString::new failed");
    let missing_name = CString::new("RLIBC_I016_UNSETENV_MISSING_SUCCESS_NO_ALIAS_TARGET")
      .expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    write_errno(80);

    // SAFETY: `missing_name` is a valid NUL-terminated environment variable name.
    let unset_rc = unsafe { unsetenv(missing_name.as_ptr()) };

    assert_eq!(unset_rc, 0);
    assert_eq!(read_errno(), 80);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
  }

  #[test]
  fn unsetenv_failure_does_not_remove_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I017_UNSETENV_FAIL_ALIAS").expect("CString::new failed");
    let mut entry = b"RLIBC_I017_UNSETENV_FAIL_ALIAS=value\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `key` is a valid NUL-terminated environment variable name.
    let unset_rc = unsafe { unsetenv(key.as_ptr()) };

    assert_eq!(unset_rc, -1);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());
  }

  #[test]
  fn unsetenv_failure_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I017_UNSETENV_FAIL_ERRNO_ALIAS").expect("CString::new failed");
    let mut entry = b"RLIBC_I017_UNSETENV_FAIL_ERRNO_ALIAS=value\0".to_vec();

    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    write_errno(37);

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `key` is a valid NUL-terminated environment variable name.
    let unset_rc = unsafe { unsetenv(key.as_ptr()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());
  }

  #[test]
  fn unsetenv_invalid_name_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_UNSETENV_INVALID_ALIAS";
    let invalid_name =
      CString::new("RLIBC_I017_UNSETENV=INVALID_ALIAS").expect("CString::new failed");
    let value_offset = b"RLIBC_I017_UNSETENV_INVALID_ALIAS=".len();
    let mut entry = b"RLIBC_I017_UNSETENV_INVALID_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(17);

    // SAFETY: `invalid_name` is a valid NUL-terminated string.
    let unset_rc = unsafe { unsetenv(invalid_name.as_ptr()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"bravo");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"bravo");
  }

  #[test]
  fn unsetenv_null_name_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_UNSETENV_NULL_NAME_ALIAS";
    let value_offset = b"RLIBC_I017_UNSETENV_NULL_NAME_ALIAS=".len();
    let mut entry = b"RLIBC_I017_UNSETENV_NULL_NAME_ALIAS=alpha\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(21);

    // SAFETY: passing null name pointer intentionally to validate `EINVAL` path.
    let unset_rc = unsafe { unsetenv(core::ptr::null()) };

    assert_eq!(unset_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must remain");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
  }

  #[test]
  fn putenv_unset_failure_does_not_remove_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_PUTENV_UNSET_FAIL_ALIAS";
    let mut entry_set = b"RLIBC_I017_PUTENV_UNSET_FAIL_ALIAS=value\0".to_vec();
    let mut entry_unset = b"RLIBC_I017_PUTENV_UNSET_FAIL_ALIAS\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry_set` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_set_rc = unsafe { putenv(entry_set.as_mut_ptr().cast()) };

    assert_eq!(put_set_rc, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `entry_unset` points to a mutable NUL-terminated `NAME` string.
    let put_unset_rc = unsafe { putenv(entry_unset.as_mut_ptr().cast()) };

    assert_eq!(put_unset_rc, -1);
    assert!(lookup_putenv_alias_value(key).is_some());
  }

  #[test]
  fn putenv_unset_failure_sets_errno_and_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_PUTENV_UNSET_FAIL_NO_ALIAS";
    let mut entry_unset = b"RLIBC_I017_PUTENV_UNSET_FAIL_NO_ALIAS\0".to_vec();

    remove_putenv_alias(key);
    assert!(lookup_putenv_alias_value(key).is_none());

    write_errno(62);

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `entry_unset` points to a mutable NUL-terminated `NAME` string.
    let rc = unsafe { putenv(entry_unset.as_mut_ptr().cast()) };

    assert_eq!(rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_none());
  }

  #[test]
  fn putenv_unset_failure_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_PUTENV_UNSET_FAIL_ERRNO_ALIAS";
    let value_offset = b"RLIBC_I017_PUTENV_UNSET_FAIL_ERRNO_ALIAS=".len();
    let mut entry_set = b"RLIBC_I017_PUTENV_UNSET_FAIL_ERRNO_ALIAS=alpha\0".to_vec();
    let mut entry_unset = b"RLIBC_I017_PUTENV_UNSET_FAIL_ERRNO_ALIAS\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry_set` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry_set.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(61);

    let _host_unavailable = force_host_env_unavailable_for_test();

    // SAFETY: `entry_unset` points to a mutable NUL-terminated `NAME` string.
    let rc = unsafe { putenv(entry_unset.as_mut_ptr().cast()) };

    assert_eq!(rc, -1);
    assert_eq!(read_errno(), EINVAL);

    entry_set[value_offset..value_offset + 5].copy_from_slice(b"bravo");

    let value_ptr = lookup_putenv_alias_value(key).expect("alias must be preserved");
    // SAFETY: `value_ptr` points inside test-owned `entry_set` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"bravo");
  }

  #[test]
  fn clearenv_failure_does_not_remove_aliases() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_CLEARENV_FAIL_ALIAS";
    let mut entry = b"RLIBC_I017_CLEARENV_FAIL_ALIAS=value\0".to_vec();

    remove_putenv_alias(key);

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    let put_rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(put_rc, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    let _host_unavailable = force_host_env_unavailable_for_test();
    let clear_rc = clearenv();

    assert_eq!(clear_rc, -1);
    assert!(lookup_putenv_alias_value(key).is_some());
  }

  #[test]
  fn clearenv_success_clears_all_aliases() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key_a = b"RLIBC_I017_CLEARENV_CLEAR_ALIAS_A";
    let key_b = b"RLIBC_I017_CLEARENV_CLEAR_ALIAS_B";
    let mut entry_a = b"RLIBC_I017_CLEARENV_CLEAR_ALIAS_A=value-a\0".to_vec();
    let mut entry_b = b"RLIBC_I017_CLEARENV_CLEAR_ALIAS_B=value-b\0".to_vec();

    remove_putenv_alias(key_a);
    remove_putenv_alias(key_b);

    // SAFETY: entries point to mutable NUL-terminated `NAME=VALUE` strings.
    assert_eq!(unsafe { putenv(entry_a.as_mut_ptr().cast()) }, 0);
    // SAFETY: entries point to mutable NUL-terminated `NAME=VALUE` strings.
    assert_eq!(unsafe { putenv(entry_b.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key_a).is_some());
    assert!(lookup_putenv_alias_value(key_b).is_some());

    assert_eq!(clearenv(), 0);
    assert!(lookup_putenv_alias_value(key_a).is_none());
    assert!(lookup_putenv_alias_value(key_b).is_none());
  }

  #[test]
  fn clearenv_success_with_no_aliases_preserves_errno() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_CLEARENV_NO_ALIAS";

    remove_putenv_alias(key);
    assert!(lookup_putenv_alias_value(key).is_none());

    write_errno(95);

    assert_eq!(clearenv(), 0);
    assert_eq!(read_errno(), 95);
    assert!(lookup_putenv_alias_value(key).is_none());
  }

  #[test]
  fn clearenv_success_preserves_errno_and_clears_aliases() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_CLEARENV_SUCCESS_ERRNO_ALIAS";
    let mut entry = b"RLIBC_I017_CLEARENV_SUCCESS_ERRNO_ALIAS=value\0".to_vec();

    remove_putenv_alias(key);
    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(91);
    assert_eq!(clearenv(), 0);
    assert_eq!(read_errno(), 91);
    assert!(lookup_putenv_alias_value(key).is_none());
  }

  #[test]
  fn clearenv_failure_sets_errno_and_preserves_aliases() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_CLEARENV_FAIL_ERRNO_ALIAS";
    let mut entry = b"RLIBC_I017_CLEARENV_FAIL_ERRNO_ALIAS=value\0".to_vec();

    remove_putenv_alias(key);
    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    assert!(lookup_putenv_alias_value(key).is_some());

    write_errno(73);

    let _host_unavailable = force_host_env_unavailable_for_test();
    let clear_rc = clearenv();

    assert_eq!(clear_rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_some());
  }
}
