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
use crate::stdlib::env_core::environ;
use core::ffi::c_char;
#[cfg(test)]
use core::ffi::c_void;
#[cfg(test)]
use core::mem;
use core::ptr;
#[cfg(test)]
use core::sync::atomic::{AtomicUsize, Ordering};
use std::ffi::{CStr, CString};
#[cfg(test)]
use std::io;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(test)]
use std::sync::TryLockError;
use std::sync::{Mutex, MutexGuard, OnceLock};

static ENV_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PUTENV_ALIASES: OnceLock<Mutex<Vec<PutEnvAlias>>> = OnceLock::new();
static OWNED_ENV: OnceLock<Mutex<OwnedEnviron>> = OnceLock::new();
#[cfg(test)]
static HOST_GETENV_FALLBACK: OnceLock<Option<HostGetenvFn>> = OnceLock::new();
#[cfg(test)]
static HOST_ENV_FNS: OnceLock<Option<HostEnvFns>> = OnceLock::new();
#[cfg(test)]
static FORCE_HOST_ENV_UNAVAILABLE_FOR_TEST: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
#[cfg(test)]
const GLIBC_DLSYM_VERSION_CANDIDATES: [&[u8]; 2] = [b"GLIBC_2.34\0", b"GLIBC_2.2.5\0"];
#[cfg(test)]
const SYMBOL_GETENV: &[u8] = b"getenv\0";
#[cfg(test)]
const SYMBOL_SETENV: &[u8] = b"setenv\0";
#[cfg(test)]
const SYMBOL_UNSETENV: &[u8] = b"unsetenv\0";

#[cfg(test)]
unsafe extern "C" {
  fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[cfg(test)]
#[link(name = "dl")]
unsafe extern "C" {
  #[link_name = "dlvsym"]
  fn host_dlvsym(handle: *mut c_void, symbol: *const c_char, version: *const c_char)
  -> *mut c_void;
}

#[cfg(test)]
type HostGetenvFn = unsafe extern "C" fn(*const c_char) -> *mut c_char;

struct PutEnvAlias {
  name: Box<[u8]>,
  entry_addr: usize,
}

struct OwnedEnviron {
  initialized: bool,
  entries: Vec<CString>,
  pointers: Vec<*mut c_char>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct HostEnvFns {
  setenv: unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> c_int,
  unsetenv: unsafe extern "C" fn(*const c_char) -> c_int,
}

// SAFETY: Access to `OwnedEnviron` is serialized behind `OWNED_ENV` and the
// process-wide environment mutation lock; pointer fields only reference entries
// owned by this struct and are never sent independently across threads.
unsafe impl Send for OwnedEnviron {}

impl OwnedEnviron {
  const fn new() -> Self {
    Self {
      initialized: false,
      entries: Vec::new(),
      pointers: Vec::new(),
    }
  }

  fn ensure_initialized(&mut self) {
    if self.initialized {
      return;
    }

    // SAFETY: Reading the raw process-global `environ` pointer does not dereference it.
    let environ_ptr = unsafe { environ };

    self.entries = if environ_ptr.is_null() {
      copy_entries_from_host_env()
    } else {
      // SAFETY: The source pointer originates from startup/host environment and may be null.
      unsafe { copy_entries_from_environ(environ_ptr) }
    };
    self.initialized = true;
    self.publish();
  }

  fn contains_name(&self, name_bytes: &[u8]) -> bool {
    self.find_name_index(name_bytes).is_some()
  }

  fn set_name_value(&mut self, name_bytes: &[u8], value_bytes: &[u8]) {
    let entry = build_env_entry(name_bytes, value_bytes);

    if let Some(index) = self.find_name_index(name_bytes) {
      self.entries[index] = entry;

      let mut kept_first = false;

      self.entries.retain(|entry| {
        if !env_entry_has_name(entry, name_bytes) {
          return true;
        }

        if kept_first {
          return false;
        }

        kept_first = true;

        true
      });
    } else {
      self.entries.push(entry);
    }

    self.publish();
  }

  fn remove_name(&mut self, name_bytes: &[u8]) {
    let original_len = self.entries.len();

    self
      .entries
      .retain(|entry| !env_entry_has_name(entry, name_bytes));

    if self.entries.len() != original_len {
      self.publish();
    }
  }

  fn clear(&mut self) {
    self.entries.clear();
    self.publish();
  }

  fn find_name_index(&self, name_bytes: &[u8]) -> Option<usize> {
    self
      .entries
      .iter()
      .position(|entry| env_entry_has_name(entry, name_bytes))
  }

  fn publish(&mut self) {
    if self.entries.is_empty() {
      self.pointers.clear();
      // SAFETY: Publishing a null pointer is valid for empty process environment.
      unsafe {
        environ = ptr::null_mut();
      }

      return;
    }

    self.pointers.clear();
    self.pointers.reserve(self.entries.len() + 1);

    for entry in &mut self.entries {
      self.pointers.push(entry.as_ptr().cast_mut());
    }

    self.pointers.push(ptr::null_mut());

    // SAFETY: `self.pointers` is kept alive by this `OwnedEnviron` state.
    unsafe {
      environ = self.pointers.as_mut_ptr();
    }
  }
}

fn env_entry_has_name(entry: &CString, name_bytes: &[u8]) -> bool {
  let bytes = entry.to_bytes();

  bytes
    .iter()
    .position(|byte| *byte == b'=')
    .is_some_and(|equal_pos| &bytes[..equal_pos] == name_bytes)
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

/// Acquires the shared environment lock used by mutators and lookup fast paths.
pub(super) fn lock_environ_state() -> MutexGuard<'static, ()> {
  env_mutation_guard()
}

/// Ensures the owned environment snapshot has been initialized.
///
/// Callers must hold [`lock_environ_state`] for the full duration to serialize
/// bootstrap against concurrent mutation entry points.
pub(super) fn ensure_owned_environ_initialized_for_lookup() {
  let mut owned_env = owned_environ_guard();

  owned_env.ensure_initialized();
}

/// Reports whether the owned environment snapshot has already been initialized.
///
/// Callers should hold [`lock_environ_state`] while consulting this state so
/// the answer stays consistent with concurrent mutation/bootstrap paths.
pub(super) fn owned_environ_initialized_for_lookup() -> bool {
  let owned_env = owned_environ_guard();

  owned_env.initialized
}

#[cfg(test)]
pub(super) fn reset_owned_environ_for_test() {
  let _guard = env_mutation_guard();

  if let Some(state) = OWNED_ENV.get() {
    let mut owned_env = match state.lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    owned_env.initialized = false;
    owned_env.entries.clear();
    owned_env.pointers.clear();
  }

  clear_putenv_aliases();
  // SAFETY: test-only reset while environment mutation is serialized.
  unsafe {
    environ = ptr::null_mut();
  }
}

#[cfg(test)]
pub(super) fn force_owned_environ_empty_for_test() {
  let _guard = env_mutation_guard();
  let mut owned_env = owned_environ_guard();

  owned_env.initialized = true;
  owned_env.entries.clear();
  owned_env.pointers.clear();
  drop(owned_env);

  // SAFETY: test-only state injection while environment mutation is serialized.
  unsafe {
    environ = ptr::null_mut();
  }
}

fn owned_environ_guard() -> MutexGuard<'static, OwnedEnviron> {
  match OWNED_ENV
    .get_or_init(|| Mutex::new(OwnedEnviron::new()))
    .lock()
  {
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

unsafe fn copy_entries_from_environ(mut environ_ptr: *mut *mut c_char) -> Vec<CString> {
  let mut copied = Vec::new();

  while !environ_ptr.is_null() {
    // SAFETY: `environ_ptr` walks a NUL-terminated `char**` environment array.
    let entry_ptr = unsafe { environ_ptr.read() };

    if entry_ptr.is_null() {
      break;
    }

    // SAFETY: each non-null environment entry points to a NUL-terminated string.
    let bytes = unsafe { CStr::from_ptr(entry_ptr).to_bytes() };
    let owned =
      CString::new(bytes).unwrap_or_else(|_| unreachable!("environment entries are NUL-free"));

    copied.push(owned);

    // SAFETY: advance within the same NUL-terminated pointer array.
    environ_ptr = unsafe { environ_ptr.add(1) };
  }

  copied
}

#[cfg(unix)]
fn copy_entries_from_proc_environ() -> Vec<CString> {
  let Ok(contents) = std::fs::read("/proc/self/environ") else {
    return Vec::new();
  };
  let mut copied = Vec::new();

  for entry in contents.split(|byte| *byte == 0) {
    if entry.is_empty() {
      continue;
    }

    let Some(equal_pos) = entry.iter().position(|byte| *byte == b'=') else {
      continue;
    };
    let name_bytes = &entry[..equal_pos];
    let value_bytes = &entry[equal_pos + 1..];

    if name_bytes.is_empty() || name_bytes.contains(&b'=') {
      continue;
    }

    copied.push(build_env_entry(name_bytes, value_bytes));
  }

  copied
}

#[cfg(unix)]
fn copy_entries_from_host_env() -> Vec<CString> {
  let proc_entries = copy_entries_from_proc_environ();

  if !proc_entries.is_empty() {
    return proc_entries;
  }

  let mut copied = Vec::new();

  for (name, value) in std::env::vars_os() {
    let name_bytes = name.into_vec();
    let value_bytes = value.into_vec();

    if name_bytes.is_empty() || name_bytes.contains(&b'=') {
      continue;
    }

    if name_bytes.contains(&0) || value_bytes.contains(&0) {
      continue;
    }

    copied.push(build_env_entry(&name_bytes, &value_bytes));
  }

  copied
}

#[cfg(not(unix))]
fn copy_entries_from_host_env() -> Vec<CString> {
  Vec::new()
}

fn build_env_entry(name_bytes: &[u8], value_bytes: &[u8]) -> CString {
  let mut entry = Vec::with_capacity(name_bytes.len() + 1 + value_bytes.len());

  entry.extend_from_slice(name_bytes);
  entry.push(b'=');
  entry.extend_from_slice(value_bytes);

  CString::new(entry).unwrap_or_else(|_| unreachable!("validated name/value are NUL-free"))
}

#[cfg(test)]
fn resolve_symbol(symbol: &'static [u8]) -> Option<*mut c_void> {
  if let Some(versioned) = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved =
      unsafe { host_dlvsym(RTLD_NEXT, symbol.as_ptr().cast(), version.as_ptr().cast()) };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  }) {
    return Some(versioned);
  }

  // SAFETY: `symbol` is a static NUL-terminated symbol name.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol.as_ptr().cast()) };

  if resolved.is_null() {
    return None;
  }

  Some(resolved)
}

#[cfg(test)]
fn resolve_host_getenv() -> Option<HostGetenvFn> {
  let getenv_ptr = resolve_symbol(SYMBOL_GETENV)?;

  // SAFETY: `dlsym` returned `getenv` with matching C signature.
  Some(unsafe {
    mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_char) -> *mut c_char>(getenv_ptr)
  })
}

#[cfg(test)]
fn host_getenv_fallback() -> Option<HostGetenvFn> {
  #[cfg(test)]
  if FORCE_HOST_ENV_UNAVAILABLE_FOR_TEST.load(Ordering::SeqCst) > 0 {
    return None;
  }

  *HOST_GETENV_FALLBACK.get_or_init(resolve_host_getenv)
}

#[cfg(test)]
fn resolve_host_env_fns() -> Option<HostEnvFns> {
  let setenv_ptr = resolve_symbol(SYMBOL_SETENV)?;
  let unsetenv_ptr = resolve_symbol(SYMBOL_UNSETENV)?;

  Some(HostEnvFns {
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
  })
}

#[cfg(test)]
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

#[cfg(test)]
fn host_errno_or(default_errno: c_int) -> c_int {
  io::Error::last_os_error()
    .raw_os_error()
    .and_then(|value| c_int::try_from(value).ok())
    .unwrap_or(default_errno)
}

#[cfg(test)]
fn host_setenv(name: &CStr, value: &CStr, overwrite: c_int) -> Result<(), c_int> {
  let host = host_env_fns().ok_or(EINVAL)?;
  // SAFETY: `name`/`value` are valid NUL-terminated strings.
  let rc = unsafe { (host.setenv)(name.as_ptr(), value.as_ptr(), overwrite) };

  if rc == 0 {
    return Ok(());
  }

  Err(host_errno_or(EINVAL))
}

#[cfg(test)]
fn host_unsetenv(name: &CStr) -> Result<(), c_int> {
  let host = host_env_fns().ok_or(EINVAL)?;
  // SAFETY: `name` is a valid NUL-terminated string.
  let rc = unsafe { (host.unsetenv)(name.as_ptr()) };

  if rc == 0 {
    return Ok(());
  }

  Err(host_errno_or(EINVAL))
}

#[cfg(test)]
pub(super) unsafe fn host_getenv(name: *const c_char) -> Option<*mut c_char> {
  let host = host_getenv_fallback()?;

  // SAFETY: Caller must provide a valid C string pointer or null.
  Some(unsafe { host(name) })
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

fn remove_putenv_aliases_for_name_or_entry(
  name_bytes: &[u8],
  entry_ptr: *mut c_char,
) -> Vec<Vec<u8>> {
  let entry_addr = entry_ptr as usize;
  let mut aliases = putenv_alias_guard();
  let mut removed_names = Vec::new();

  aliases.retain(|alias| {
    if alias.name.as_ref() == name_bytes || alias.entry_addr == entry_addr {
      removed_names.push(alias.name.to_vec());

      return false;
    }

    true
  });
  drop(aliases);

  removed_names.sort_unstable();
  removed_names.dedup();

  removed_names
}

fn clear_putenv_aliases() {
  let mut aliases = putenv_alias_guard();

  aliases.clear();
}

fn remove_owned_name_if_initialized(name_bytes: &[u8]) {
  let mut owned_env = owned_environ_guard();

  if !owned_env.initialized {
    return;
  }

  owned_env.remove_name(name_bytes);
}

fn rebind_putenv_alias(name_bytes: &[u8], entry_ptr: *mut c_char) -> Vec<Vec<u8>> {
  let entry_addr = entry_ptr as usize;
  let mut aliases = putenv_alias_guard();
  let mut renamed_names = Vec::new();

  aliases.retain(|alias| {
    if alias.name.as_ref() == name_bytes {
      return false;
    }

    if alias.entry_addr == entry_addr {
      renamed_names.push(alias.name.to_vec());

      return false;
    }

    true
  });

  aliases.push(PutEnvAlias {
    name: name_bytes.to_vec().into_boxed_slice(),
    entry_addr,
  });
  drop(aliases);

  renamed_names.sort_unstable();
  renamed_names.dedup();

  renamed_names
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

  let removed_stale_aliases = resolved_value_ptr.is_none() && !remove_indices.is_empty();

  for alias_index in remove_indices.into_iter().rev() {
    aliases.remove(alias_index);
  }

  drop(aliases);

  if removed_stale_aliases {
    remove_owned_name_if_initialized(name_bytes);
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

  let previous_errno = current_errno();
  let _guard = env_mutation_guard();

  {
    let mut owned_env = owned_environ_guard();

    owned_env.ensure_initialized();
  }

  let existed_before = if overwrite == 0 {
    if lookup_putenv_alias_value(name_bytes.as_slice()).is_some() {
      true
    } else {
      let owned_env = owned_environ_guard();

      owned_env.contains_name(name_bytes.as_slice())
    }
  } else {
    false
  };

  if overwrite == 0 && existed_before {
    set_errno(previous_errno);

    return 0;
  }

  let mut owned_env = owned_environ_guard();

  owned_env.set_name_value(name_bytes.as_slice(), value_bytes.as_slice());
  drop(owned_env);

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

  let previous_errno = current_errno();
  let _guard = env_mutation_guard();
  let mut owned_env = owned_environ_guard();

  owned_env.ensure_initialized();
  owned_env.remove_name(name_bytes.as_slice());
  drop(owned_env);

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
  let mut owned_env = owned_environ_guard();

  owned_env.ensure_initialized();

  if let Some(eq_pos) = bytes.iter().position(|byte| *byte == b'=') {
    let name_bytes = &bytes[..eq_pos];
    let value_bytes = &bytes[eq_pos + 1..];

    if let Err(errno_value) = validate_name_bytes(name_bytes) {
      return fail_with_errno(errno_value);
    }

    owned_env.set_name_value(name_bytes, value_bytes);
    drop(owned_env);

    let renamed_names = rebind_putenv_alias(name_bytes, string);

    for renamed_name in renamed_names {
      if lookup_putenv_alias_value(renamed_name.as_slice()).is_none() {
        remove_owned_name_if_initialized(renamed_name.as_slice());
      }
    }

    set_errno(previous_errno);

    return 0;
  }

  if let Err(errno_value) = validate_name_bytes(&bytes) {
    return fail_with_errno(errno_value);
  }

  owned_env.remove_name(&bytes);
  drop(owned_env);

  let removed_names = remove_putenv_aliases_for_name_or_entry(&bytes, string);

  for removed_name in removed_names {
    if lookup_putenv_alias_value(removed_name.as_slice()).is_none() {
      remove_owned_name_if_initialized(removed_name.as_slice());
    }
  }

  set_errno(previous_errno);

  0
}

/// C ABI entry point for `clearenv`.
///
/// Removes every environment variable currently visible to this process.
/// Returns `0` on success. Returns `-1` and sets `errno` on failure.
///
/// # Errors
/// This implementation does not report additional errors.
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
  let mut owned_env = owned_environ_guard();

  owned_env.ensure_initialized();
  owned_env.clear();
  drop(owned_env);

  clear_putenv_aliases();

  set_errno(previous_errno);

  0
}

#[cfg(test)]
mod tests {
  use std::ffi::{CStr, CString};
  use std::sync::MutexGuard;

  use crate::abi::errno::EINVAL;
  use crate::errno::__errno_location;
  use crate::stdlib::env_core::getenv;
  use crate::stdlib::env_mut::{
    clearenv, force_host_env_unavailable_for_test, host_env_fns, host_getenv, host_setenv,
    host_unsetenv, lookup_putenv_alias_value, putenv, remove_putenv_alias,
    reset_owned_environ_for_test, setenv, unsetenv,
  };
  use crate::stdlib::lock_environ_for_test;

  static TEST_LOCK: SharedEnvTestLock = SharedEnvTestLock;

  struct SharedEnvTestLock;

  struct SharedEnvTestLockPoisoned;

  struct OwnedEnvironReset;

  impl SharedEnvTestLock {
    fn lock(&self) -> Result<MutexGuard<'static, ()>, SharedEnvTestLockPoisoned> {
      let _ = self;

      if std::thread::panicking() {
        Err(SharedEnvTestLockPoisoned)
      } else {
        Ok(lock_environ_for_test())
      }
    }
  }

  impl SharedEnvTestLockPoisoned {
    fn into_inner(self) -> MutexGuard<'static, ()> {
      let _ = self;

      lock_environ_for_test()
    }
  }

  impl Drop for OwnedEnvironReset {
    fn drop(&mut self) {
      reset_owned_environ_for_test();
    }
  }

  fn test_lock() -> &'static SharedEnvTestLock {
    &TEST_LOCK
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

  fn set_owned_entries_for_test(entries: &[&[u8]]) {
    let mut owned_env = super::owned_environ_guard();

    owned_env.initialized = true;
    owned_env.entries = entries
      .iter()
      .map(|entry| {
        CString::new((*entry).to_vec())
          .unwrap_or_else(|_| unreachable!("test entries must be NUL-free"))
      })
      .collect();
    owned_env.publish();
  }

  fn owned_entry_count(name_bytes: &[u8]) -> usize {
    let owned_env = super::owned_environ_guard();

    owned_env
      .entries
      .iter()
      .filter(|entry| super::env_entry_has_name(entry, name_bytes))
      .count()
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
  fn lookup_putenv_alias_value_drops_stale_owned_copy_after_name_prefix_mismatch() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let _owned_reset = OwnedEnvironReset;
    let key = CString::new("RLIBC_I017_ALIAS_RENAME_STALE_COPY").expect("CString::new failed");
    let mut entry = b"RLIBC_I017_ALIAS_RENAME_STALE_COPY=value\0".to_vec();
    let original_first = entry[0];

    reset_owned_environ_for_test();
    remove_putenv_alias(key.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    // SAFETY: `key` is a valid NUL-terminated environment variable name.
    assert!(!unsafe { getenv(key.as_ptr()) }.is_null());

    entry[0] = b'X';

    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
    // SAFETY: `key` is a valid NUL-terminated environment variable name.
    assert!(unsafe { getenv(key.as_ptr()) }.is_null());

    entry[0] = original_first;

    // SAFETY: `key` is a valid NUL-terminated environment variable name.
    assert!(unsafe { getenv(key.as_ptr()) }.is_null());
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
    let key = b"RLIBC_I017_PUTENV_EMPTY_NAME_NO_ALIAS";
    let mut entry = b"=value\0".to_vec();

    remove_putenv_alias(key);
    assert!(lookup_putenv_alias_value(key).is_none());
    write_errno(67);

    // SAFETY: `entry` points to a mutable NUL-terminated string.
    let rc = unsafe { putenv(entry.as_mut_ptr().cast()) };

    assert_eq!(rc, -1);
    assert_eq!(read_errno(), EINVAL);
    assert!(lookup_putenv_alias_value(key).is_none());
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
  fn putenv_without_equal_missing_name_preserves_errno_and_does_not_create_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = CString::new("RLIBC_I017_PUTENV_UNSET_MISSING_SUCCESS_NO_ALIAS")
      .expect("CString::new failed");
    let mut entry_unset = b"RLIBC_I017_PUTENV_UNSET_MISSING_SUCCESS_NO_ALIAS\0".to_vec();

    remove_putenv_alias(key.as_bytes());
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());

    write_errno(66);

    // SAFETY: `entry_unset` points to a mutable NUL-terminated `NAME` string.
    let rc = unsafe { putenv(entry_unset.as_mut_ptr().cast()) };

    assert_eq!(rc, 0);
    assert_eq!(read_errno(), 66);
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
  fn putenv_renamed_buffer_rebind_removes_old_name_copy() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let _owned_reset = OwnedEnvironReset;
    let old_name =
      CString::new("RLIBC_I017_PUTENV_RENAME_BUFFER_OLD").expect("CString::new failed");
    let new_name =
      CString::new("RLIBC_I017_PUTENV_RENAME_BUFFER_NEW").expect("CString::new failed");
    let mut entry = b"RLIBC_I017_PUTENV_RENAME_BUFFER_OLD=alpha\0".to_vec();

    reset_owned_environ_for_test();
    remove_putenv_alias(old_name.as_bytes());
    remove_putenv_alias(new_name.as_bytes());

    // SAFETY: `entry` points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    // SAFETY: `old_name` is a valid NUL-terminated environment variable name.
    assert!(!unsafe { getenv(old_name.as_ptr()) }.is_null());

    entry[..new_name.as_bytes().len()].copy_from_slice(new_name.as_bytes());

    // SAFETY: `entry` still points to a mutable NUL-terminated `NAME=VALUE` string.
    assert_eq!(unsafe { putenv(entry.as_mut_ptr().cast()) }, 0);
    // SAFETY: `old_name` is a valid NUL-terminated environment variable name.
    assert!(unsafe { getenv(old_name.as_ptr()) }.is_null());

    // SAFETY: `new_name` is a valid NUL-terminated environment variable name.
    let value_ptr = unsafe { getenv(new_name.as_ptr()) };

    assert!(!value_ptr.is_null());
    // SAFETY: `value_ptr` points to a NUL-terminated value string.
    let value = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(value.to_bytes(), b"alpha");
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

    assert_eq!(rc, 0);
    assert!(lookup_putenv_alias_value(key).is_some());
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

    assert_eq!(rc, 0);
    assert_eq!(read_errno(), 11);
    assert!(lookup_putenv_alias_value(key).is_some());
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

    assert_eq!(rc, 0);
    assert_eq!(read_errno(), 19);

    second[value_offset..value_offset + 5].copy_from_slice(b"bravo");

    let value_ptr = lookup_putenv_alias_value(key).expect("latest alias must be tracked");
    // SAFETY: `value_ptr` points inside test-owned `second` NUL-terminated buffer.
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

    assert_eq!(set_rc, 0);
    assert_eq!(read_errno(), 37);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
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

    assert_eq!(set_rc, 0);
    assert_eq!(read_errno(), 73);
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
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_some());

    entry[value_offset..value_offset + 5].copy_from_slice(b"omega");

    let value_ptr = lookup_putenv_alias_value(key.as_bytes()).expect("alias must remain active");
    // SAFETY: `value_ptr` points inside test-owned `entry` NUL-terminated buffer.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"omega");
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

    // SAFETY: `key` is a valid NUL-terminated key.
    let current = unsafe { getenv(key.as_ptr()) };

    assert!(!current.is_null());

    // SAFETY: `current` is non-null and points to a NUL-terminated value.
    let value_now = unsafe { CStr::from_ptr(current) };

    assert_eq!(value_now.to_bytes(), b"replacement");
  }

  #[test]
  fn setenv_overwrite_collapses_owned_duplicate_entries() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let _owned_reset = OwnedEnvironReset;
    let key = CString::new("RLIBC_I017_SETENV_DUPLICATE_KEY").expect("CString::new failed");
    let value = CString::new("fresh").expect("CString::new failed");

    reset_owned_environ_for_test();
    set_owned_entries_for_test(&[
      b"RLIBC_I017_SETENV_DUPLICATE_KEY=old-a",
      b"RLIBC_I017_SETENV_DUPLICATE_KEY=old-b",
      b"RLIBC_I017_SETENV_DUPLICATE_OTHER=keep",
    ]);

    assert_eq!(owned_entry_count(key.as_bytes()), 2);

    // SAFETY: `key` and `value` are valid NUL-terminated strings.
    assert_eq!(unsafe { setenv(key.as_ptr(), value.as_ptr(), 1) }, 0);
    assert_eq!(owned_entry_count(key.as_bytes()), 1);

    // SAFETY: `key` is a valid NUL-terminated environment variable name.
    let value_ptr = unsafe { getenv(key.as_ptr()) };

    assert!(!value_ptr.is_null());
    // SAFETY: `value_ptr` points to a NUL-terminated value string.
    let current = unsafe { CStr::from_ptr(value_ptr.cast_const()) };

    assert_eq!(current.to_bytes(), b"fresh");
  }

  #[test]
  fn setenv_no_overwrite_ignores_host_only_value_and_updates_owned_environ() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key =
      CString::new("RLIBC_I016_SETENV_NO_OVERWRITE_IGNORE_HOST_ONLY").expect("CString::new failed");
    let value = CString::new("owned").expect("CString::new failed");

    remove_putenv_alias(key.as_bytes());
    assert_eq!(clearenv(), 0);

    let host_value = CString::new("host_only").expect("CString::new failed");
    let host_set = host_setenv(&key, &host_value, 1);

    assert!(host_set.is_ok());

    write_errno(52);

    // SAFETY: `key` and `value` are valid NUL-terminated strings.
    let set_rc = unsafe { setenv(key.as_ptr(), value.as_ptr(), 0) };

    assert_eq!(set_rc, 0);
    assert_eq!(read_errno(), 52);

    // SAFETY: `key` is a valid NUL-terminated key.
    let resolved = unsafe { getenv(key.as_ptr()) };

    assert!(!resolved.is_null());
    // SAFETY: `resolved` points to a NUL-terminated value string.
    let current = unsafe { CStr::from_ptr(resolved.cast_const()) };

    assert_eq!(current.to_bytes(), b"owned");

    let cleanup = host_unsetenv(&key);

    assert!(cleanup.is_ok());
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

    assert_eq!(set_rc, 0);
    assert_eq!(read_errno(), 29);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
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
  fn unsetenv_removes_all_owned_duplicate_entries() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let _owned_reset = OwnedEnvironReset;
    let key = CString::new("RLIBC_I017_UNSETENV_DUPLICATE_KEY").expect("CString::new failed");

    reset_owned_environ_for_test();
    set_owned_entries_for_test(&[
      b"RLIBC_I017_UNSETENV_DUPLICATE_KEY=old-a",
      b"RLIBC_I017_UNSETENV_DUPLICATE_KEY=old-b",
      b"RLIBC_I017_UNSETENV_DUPLICATE_OTHER=keep",
    ]);

    assert_eq!(owned_entry_count(key.as_bytes()), 2);

    // SAFETY: `key` is a valid NUL-terminated environment variable name.
    assert_eq!(unsafe { unsetenv(key.as_ptr()) }, 0);
    assert_eq!(owned_entry_count(key.as_bytes()), 0);
    // SAFETY: `key` is a valid NUL-terminated environment variable name.
    assert!(unsafe { getenv(key.as_ptr()) }.is_null());
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

    assert_eq!(unset_rc, 0);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
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

    assert_eq!(unset_rc, 0);
    assert_eq!(read_errno(), 37);
    assert!(lookup_putenv_alias_value(key.as_bytes()).is_none());
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

    assert_eq!(put_unset_rc, 0);
    assert!(lookup_putenv_alias_value(key).is_none());
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

    assert_eq!(rc, 0);
    assert_eq!(read_errno(), 62);
    assert!(lookup_putenv_alias_value(key).is_none());
  }

  #[test]
  fn putenv_unset_failure_sets_errno_and_preserves_alias() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let key = b"RLIBC_I017_PUTENV_UNSET_FAIL_ERRNO_ALIAS";
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

    assert_eq!(rc, 0);
    assert_eq!(read_errno(), 61);
    assert!(lookup_putenv_alias_value(key).is_none());
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

    assert_eq!(clear_rc, 0);
    assert!(lookup_putenv_alias_value(key).is_none());
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

    assert_eq!(clear_rc, 0);
    assert_eq!(read_errno(), 73);
    assert!(lookup_putenv_alias_value(key).is_none());
  }
}
