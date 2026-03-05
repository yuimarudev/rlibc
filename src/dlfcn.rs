//! Dynamic loader C ABI helpers.
//!
//! This module provides minimal dynamic-loader interfaces:
//! - `dlopen` path/flag validation and host-loader delegation,
//! - `dlsym` symbol lookup delegation,
//! - `dlerror` thread-local error retrieval,
//! - `dlclose` refcount-only lifecycle behavior.
//!
//! The lifecycle model is intentionally conservative: final close marks a
//! handle as closed but does not attempt to unmap code/data in this phase.

use crate::abi::errno::{
  EACCES, EADDRINUSE, EADDRNOTAVAIL, EAGAIN, ECONNABORTED, ECONNREFUSED, ECONNRESET, EEXIST,
  EHOSTUNREACH, EINTR, EINVAL, EISDIR, ENETDOWN, ENETUNREACH, ENOENT, ENOEXEC, ENOTCONN, ENOTDIR,
  EPIPE, ETIMEDOUT,
};
use crate::abi::types::{c_char, c_int, c_void};
use crate::errno::{__errno_location, set_errno};
use core::{mem, ptr};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

const DLCLOSE_SUCCESS: c_int = 0;
const DLCLOSE_FAILURE: c_int = -1;
const DLERROR_INVALID_HANDLE: &str = "rlibc: invalid dynamic-loader handle";
const DLERROR_ALREADY_CLOSED: &str = "rlibc: dynamic-loader handle already closed";
const DLERROR_NULL_SYMBOL: &str = "rlibc: dlsym symbol pointer is null";
const DLERROR_DLOPEN_NULL_PATH: &str = "rlibc: dlopen path pointer is null";
const DLERROR_DLOPEN_INVALID_FLAGS: &str = "rlibc: dlopen received invalid flags";
const DLERROR_DLOPEN_NOT_ELF: &str = "rlibc: dlopen target is not a valid ELF image";
const DLERROR_DLOPEN_PATH_OPEN_FAILED: &str = "rlibc: dlopen target path could not be opened";
const DLERROR_HOST_DLOPEN_UNAVAILABLE: &str = "rlibc: host dlopen resolver unavailable";
const DLERROR_HOST_DLOPEN_FAILED: &str = "rlibc: host dlopen call failed";
const DLERROR_HOST_DLSYM_UNAVAILABLE: &str = "rlibc: host dlsym resolver unavailable";
const DLERROR_SYMBOL_NOT_FOUND: &str = "rlibc: requested symbol was not found";
const TRACKABLE_NULL_HANDLE_ID: usize = 0;
/// Runtime loader mode flag: resolve symbols lazily.
pub const RTLD_LAZY: c_int = 0x0001;
/// Runtime loader mode flag: resolve symbols immediately.
pub const RTLD_NOW: c_int = 0x0002;
/// Runtime loader visibility flag: make symbols available for later lookups.
pub const RTLD_GLOBAL: c_int = 0x0100;
/// Runtime loader visibility flag: keep symbols local to the opened object.
pub const RTLD_LOCAL: c_int = 0;
const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
const RTLD_BINDING_MASK: c_int = RTLD_LAZY | RTLD_NOW;
const RTLD_SUPPORTED_MASK: c_int = RTLD_LAZY | RTLD_NOW | RTLD_GLOBAL;
const DLOPEN_SYMBOL_NAME: &[u8] = b"dlopen\0";
const DLERROR_SYMBOL_NAME: &[u8] = b"dlerror\0";
const DLSYM_SYMBOL_NAME: &[u8] = b"dlsym\0";
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const GLIBC_DLSYM_VERSION_CANDIDATES: [&[u8]; 2] = [b"GLIBC_2.34\0", b"GLIBC_2.2.5\0"];

thread_local! {
  static DLERROR_STATE: RefCell<DlErrorState> = const { RefCell::new(DlErrorState::new()) };
}

static DL_HANDLE_REGISTRY: OnceLock<Mutex<DlHandleRegistry>> = OnceLock::new();
static HOST_DLOPEN: OnceLock<Option<HostDlopenFn>> = OnceLock::new();
static HOST_DLERROR: OnceLock<Option<HostDlerrorFn>> = OnceLock::new();
static HOST_DLSYM: OnceLock<Option<HostDlsymFn>> = OnceLock::new();

type HostDlopenFn = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;

type HostDlerrorFn = unsafe extern "C" fn() -> *mut c_char;

type HostDlsymFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

#[link(name = "dl")]
unsafe extern "C" {
  #[link_name = "dlvsym"]
  fn host_dlvsym(handle: *mut c_void, symbol: *const c_char, version: *const c_char)
  -> *mut c_void;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseOutcome {
  Success,
  AlreadyClosed,
  InvalidHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DlHandleState {
  Open { refcount: usize },
  Closed,
}

struct DlHandleRegistry {
  handles: HashMap<usize, DlHandleState>,
  next_handle_id: usize,
}

struct DlErrorState {
  pending_message: Option<CString>,
  last_returned_message: Option<CString>,
}

impl DlErrorState {
  const fn new() -> Self {
    Self {
      pending_message: None,
      last_returned_message: None,
    }
  }

  fn set_pending_message(&mut self, message: &str) {
    let sanitized_message = message.replace('\0', " ");
    let c_message = CString::new(sanitized_message)
      .unwrap_or_else(|_| unreachable!("NUL bytes were replaced before CString conversion"));

    self.pending_message = Some(c_message);
  }

  fn take_message_ptr(&mut self) -> *mut c_char {
    let Some(message) = self.pending_message.take() else {
      self.last_returned_message = None;

      return ptr::null_mut();
    };

    self.last_returned_message = Some(message);

    self
      .last_returned_message
      .as_mut()
      .map_or(ptr::null_mut(), |value| value.as_ptr().cast_mut())
  }
}

impl DlHandleRegistry {
  fn new() -> Self {
    Self {
      handles: HashMap::new(),
      next_handle_id: 1,
    }
  }

  fn register_open_handle(&mut self, handle: *mut c_void) {
    if handle.is_null() {
      self.clear_trackable_null_handle_entry();

      return;
    }
    self.clear_trackable_null_handle_entry();

    let handle_id = handle as usize;
    let Some(next_after_handle) = handle_id.checked_add(1) else {
      if let Some(handle_state) = self.handles.get_mut(&handle_id) {
        *handle_state = match *handle_state {
          DlHandleState::Open { refcount } => DlHandleState::Open {
            refcount: refcount.saturating_add(1),
          },
          DlHandleState::Closed => DlHandleState::Open { refcount: 1 },
        };
      }

      return;
    };

    self.next_handle_id = self.next_handle_id.max(next_after_handle);

    if let Some(DlHandleState::Open { refcount }) = self.handles.get_mut(&handle_id) {
      *refcount = refcount.saturating_add(1);

      return;
    }

    self
      .handles
      .insert(handle_id, DlHandleState::Open { refcount: 1 });
  }

  fn close_handle(&mut self, handle_id: usize) -> CloseOutcome {
    if handle_id == TRACKABLE_NULL_HANDLE_ID {
      self.clear_trackable_null_handle_entry();

      return CloseOutcome::InvalidHandle;
    }

    self.clear_trackable_null_handle_entry();

    let Some(handle_state) = self.handles.get_mut(&handle_id) else {
      return CloseOutcome::InvalidHandle;
    };

    match handle_state {
      DlHandleState::Open { refcount } => {
        if *refcount > 1 {
          *refcount -= 1;

          return CloseOutcome::Success;
        }

        *handle_state = DlHandleState::Closed;

        CloseOutcome::Success
      }
      DlHandleState::Closed => CloseOutcome::AlreadyClosed,
    }
  }

  fn validate_dlsym_handle(&self, handle_id: usize) -> Result<(), &'static str> {
    match self.handles.get(&handle_id) {
      Some(DlHandleState::Open { .. }) => Ok(()),
      Some(DlHandleState::Closed) => Err(DLERROR_ALREADY_CLOSED),
      None => Err(DLERROR_INVALID_HANDLE),
    }
  }

  fn clear_trackable_null_handle_entry(&mut self) {
    self.handles.remove(&TRACKABLE_NULL_HANDLE_ID);
  }

  #[cfg(test)]
  fn allocate_test_handle(&mut self, initial_refcount: usize) -> *mut c_void {
    let handle_id = self.next_handle_id;
    let refcount = initial_refcount.max(1);

    self.next_handle_id = self.next_handle_id.saturating_add(1);
    self
      .handles
      .insert(handle_id, DlHandleState::Open { refcount });

    handle_id as *mut c_void
  }

  #[cfg(test)]
  fn handle_state(&self, handle_id: usize) -> Option<DlHandleState> {
    self.handles.get(&handle_id).copied()
  }
}

fn handle_registry() -> &'static Mutex<DlHandleRegistry> {
  DL_HANDLE_REGISTRY.get_or_init(|| Mutex::new(DlHandleRegistry::new()))
}

fn handle_registry_guard() -> MutexGuard<'static, DlHandleRegistry> {
  match handle_registry().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn set_dlerror_message(message: &str) {
  DLERROR_STATE.with(|state| {
    state.borrow_mut().set_pending_message(message);
  });
}

fn set_dlsym_missing_symbol_message(symbol: *const c_char, detail: Option<&str>) {
  // SAFETY: `dlsym` validates `symbol` is non-null and its safety contract
  // requires a valid NUL-terminated C string.
  let symbol_name = unsafe { CStr::from_ptr(symbol) };

  if symbol_name.to_bytes().is_empty() {
    let base_message = format!("{DLERROR_SYMBOL_NOT_FOUND}: <empty symbol>");

    set_dlerror_message(&base_message);

    return;
  }

  let symbol_label = symbol_name.to_string_lossy().into_owned();
  let base_message = format!("{DLERROR_SYMBOL_NOT_FOUND}: {symbol_label}");
  let normalized_detail =
    detail.and_then(|value| normalize_dlsym_missing_symbol_detail(&symbol_label, value));

  set_dlerror_message_with_detail(&base_message, normalized_detail.as_deref());
}

fn normalize_dlsym_missing_symbol_detail(symbol_label: &str, detail: &str) -> Option<String> {
  let trimmed_detail = detail.trim();

  if trimmed_detail.is_empty() {
    return None;
  }

  let mut normalized = trimmed_detail;

  while let Some(after_symbol) = normalized.strip_prefix(symbol_label) {
    let after_symbol = after_symbol.trim_start();

    if after_symbol.is_empty() {
      return None;
    }

    let Some(after_separator) = after_symbol.strip_prefix(':') else {
      break;
    };
    let mut collapsed = after_separator;

    while let Some(rest) = collapsed.trim_start().strip_prefix(':') {
      collapsed = rest;
    }

    let collapsed = collapsed.trim_start();

    if collapsed.is_empty() {
      return None;
    }

    normalized = collapsed;
  }

  Some(normalized.to_owned())
}

fn is_dlsym_special_handle(handle: *mut c_void) -> bool {
  handle.is_null() || handle == RTLD_NEXT
}

fn validate_dlsym_handle(handle: *mut c_void) -> Result<(), &'static str> {
  if is_dlsym_special_handle(handle) {
    return Ok(());
  }

  let registry = handle_registry_guard();

  registry.validate_dlsym_handle(handle as usize)
}

fn resolve_host_symbol(symbol_name: &[u8]) -> Option<*mut c_void> {
  GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: `symbol_name` and each version are valid NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        symbol_name.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  })
}

fn resolve_host_dlopen() -> Option<HostDlopenFn> {
  let resolved = resolve_host_symbol(DLOPEN_SYMBOL_NAME)?;

  // SAFETY: `resolved` is the runtime loader address of `dlopen` and therefore
  // matches `HostDlopenFn`.
  Some(unsafe { mem::transmute::<*mut c_void, HostDlopenFn>(resolved) })
}

fn resolve_host_dlsym() -> Option<HostDlsymFn> {
  let resolved = resolve_host_symbol(DLSYM_SYMBOL_NAME)?;

  // SAFETY: `resolved` is the runtime loader address of `dlsym` and therefore
  // matches `HostDlsymFn`.
  Some(unsafe { mem::transmute::<*mut c_void, HostDlsymFn>(resolved) })
}

fn resolve_host_dlerror() -> Option<HostDlerrorFn> {
  let resolved = resolve_host_symbol(DLERROR_SYMBOL_NAME)?;

  // SAFETY: `resolved` is the runtime loader address of `dlerror` and therefore
  // matches `HostDlerrorFn`.
  Some(unsafe { mem::transmute::<*mut c_void, HostDlerrorFn>(resolved) })
}

fn host_dlopen() -> Option<HostDlopenFn> {
  *HOST_DLOPEN.get_or_init(resolve_host_dlopen)
}

fn host_dlerror() -> Option<HostDlerrorFn> {
  *HOST_DLERROR.get_or_init(resolve_host_dlerror)
}

fn host_dlsym() -> Option<HostDlsymFn> {
  *HOST_DLSYM.get_or_init(resolve_host_dlsym)
}

fn clear_host_dlerror_state() {
  let Some(host_dlerror_fn) = host_dlerror() else {
    return;
  };

  // SAFETY: calling host `dlerror` clears the host thread-local pending loader error.
  let _ = unsafe { host_dlerror_fn() };
}

fn take_host_dlerror_message() -> Option<String> {
  let host_dlerror_fn = host_dlerror()?;
  // SAFETY: host `dlerror` returns null or a valid NUL-terminated message pointer.
  let message_ptr = unsafe { host_dlerror_fn() };

  if message_ptr.is_null() {
    return None;
  }

  // SAFETY: non-null host `dlerror` pointer is a valid NUL-terminated C string.
  let message = unsafe { CStr::from_ptr(message_ptr.cast_const()) };

  Some(message.to_string_lossy().into_owned())
}

fn set_dlerror_message_with_detail(base_message: &str, detail: Option<&str>) {
  let Some(detail_text) = detail.map(str::trim).filter(|value| !value.is_empty()) else {
    set_dlerror_message(base_message);

    return;
  };
  let composed_message = format!("{base_message}: {detail_text}");

  set_dlerror_message(&composed_message);
}

fn current_errno() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage.
  unsafe { errno_ptr.read() }
}

const fn validate_dlopen_flags(flags: c_int) -> Result<(), c_int> {
  if flags & !RTLD_SUPPORTED_MASK != 0 {
    return Err(EINVAL);
  }

  let binding_mode = flags & RTLD_BINDING_MASK;

  if binding_mode == 0 || binding_mode == RTLD_BINDING_MASK {
    return Err(EINVAL);
  }

  Ok(())
}

const fn io_error_kind_errno(error_kind: io::ErrorKind) -> Option<c_int> {
  match error_kind {
    io::ErrorKind::NotFound => Some(ENOENT),
    io::ErrorKind::NotADirectory => Some(ENOTDIR),
    io::ErrorKind::AlreadyExists => Some(EEXIST),
    io::ErrorKind::WouldBlock => Some(EAGAIN),
    io::ErrorKind::TimedOut => Some(ETIMEDOUT),
    io::ErrorKind::BrokenPipe => Some(EPIPE),
    io::ErrorKind::ConnectionRefused => Some(ECONNREFUSED),
    io::ErrorKind::ConnectionReset => Some(ECONNRESET),
    io::ErrorKind::ConnectionAborted => Some(ECONNABORTED),
    io::ErrorKind::NotConnected => Some(ENOTCONN),
    io::ErrorKind::AddrInUse => Some(EADDRINUSE),
    io::ErrorKind::AddrNotAvailable => Some(EADDRNOTAVAIL),
    io::ErrorKind::NetworkUnreachable => Some(ENETUNREACH),
    io::ErrorKind::NetworkDown => Some(ENETDOWN),
    io::ErrorKind::HostUnreachable => Some(EHOSTUNREACH),
    io::ErrorKind::PermissionDenied => Some(EACCES),
    io::ErrorKind::InvalidInput => Some(EINVAL),
    io::ErrorKind::Interrupted => Some(EINTR),
    io::ErrorKind::IsADirectory => Some(EISDIR),
    _ => None,
  }
}

fn io_error_errno(error: &io::Error, fallback_errno: c_int) -> c_int {
  error
    .raw_os_error()
    .and_then(|value| c_int::try_from(value).ok())
    .filter(|value| *value != 0)
    .or_else(|| io_error_kind_errno(error.kind()))
    .unwrap_or(fallback_errno)
}

fn open_loader_file(path: &Path) -> Result<File, c_int> {
  File::open(path).map_err(|error| io_error_errno(&error, ENOENT))
}

fn validate_elf_image(path: &Path) -> Result<(), c_int> {
  let mut file = open_loader_file(path)?;
  let mut magic = [0_u8; 4];

  match file.read_exact(&mut magic) {
    Ok(()) => {}
    Err(error) => {
      let errno_value = if error.kind() == io::ErrorKind::UnexpectedEof {
        ENOEXEC
      } else {
        io_error_errno(&error, ENOEXEC)
      };

      return Err(errno_value);
    }
  }

  if magic != ELF_MAGIC {
    return Err(ENOEXEC);
  }

  Ok(())
}

#[cfg(test)]
fn clear_dlerror_state() {
  DLERROR_STATE.with(|state| {
    let mut state = state.borrow_mut();

    state.pending_message = None;
    state.last_returned_message = None;
  });
}

/// C ABI entry point for `dlerror`.
///
/// Returns a thread-local error-message pointer for the most recent dynamic
/// loader failure in the current thread and clears that pending message
/// (clear-on-read).
///
/// Returns:
/// - null pointer when no pending dynamic-loader error exists;
/// - non-null pointer to a NUL-terminated thread-local message otherwise.
///
/// Message pointers remain valid until the next `dlerror` update in the same
/// thread or until the thread exits.
///
/// This function does not modify `errno`.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn dlerror() -> *mut c_char {
  DLERROR_STATE.with(|state| state.borrow_mut().take_message_ptr())
}

/// C ABI entry point for `dlclose`.
///
/// Decrements the loader-managed reference count for `handle`.
///
/// Behavior in this phase:
/// - if the handle refcount remains above zero, it is decremented and this
///   function returns `0`;
/// - if the final reference is closed, the handle transitions to a closed state
///   and this function returns `0`;
/// - closed/unknown/null handles return `-1` and set a thread-local message
///   retrievable through `dlerror`.
///
/// This implementation intentionally uses a refcount-only safety model and does
/// not unmap object code/data on final close.
///
/// Thread-safety:
/// - handle-state transitions are guarded by an internal mutex, so concurrent
///   callers cannot underflow reference counts.
#[unsafe(no_mangle)]
pub extern "C" fn dlclose(handle: *mut c_void) -> c_int {
  if handle.is_null() {
    {
      let mut registry = handle_registry_guard();

      registry.clear_trackable_null_handle_entry();
    }

    set_dlerror_message(DLERROR_INVALID_HANDLE);

    return DLCLOSE_FAILURE;
  }

  let handle_id = handle as usize;
  let close_outcome = {
    let mut registry = handle_registry_guard();

    registry.close_handle(handle_id)
  };

  match close_outcome {
    CloseOutcome::Success => DLCLOSE_SUCCESS,
    CloseOutcome::AlreadyClosed => {
      set_dlerror_message(DLERROR_ALREADY_CLOSED);
      DLCLOSE_FAILURE
    }
    CloseOutcome::InvalidHandle => {
      set_dlerror_message(DLERROR_INVALID_HANDLE);
      DLCLOSE_FAILURE
    }
  }
}

/// C ABI entry point for `dlopen`.
///
/// Opens a shared object from `filename` using `flags` and returns an opaque
/// loader handle.
///
/// Supported flags in this minimal phase:
/// - one of `RTLD_LAZY` or `RTLD_NOW`,
/// - optional `RTLD_GLOBAL` (with `RTLD_LOCAL` represented as zero).
///
/// Returns:
/// - non-null handle on success;
/// - null on failure, sets thread-local `errno`, and records a thread-local
///   diagnostic retrievable via [`dlerror`].
///
/// On success the returned handle is registered in this libc's internal handle
/// registry so it can be consumed by [`dlclose`] in later calls.
///
/// Error contract in this phase:
/// - `EINVAL` for null path pointer or unsupported/invalid flags;
/// - `ENOEXEC` for non-ELF input files;
/// - host OS error codes (for example `ENOENT`) when the target cannot be
///   opened by path (`dlerror` includes the failing path detail);
/// - host runtime loader failures report `dlerror` with
///   `rlibc: host dlopen call failed` plus host detail text when available.
///
/// On success, this function preserves the calling thread's previous `errno`.
///
/// # Safety
/// - `filename` must point to a valid NUL-terminated C string.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void {
  if filename.is_null() {
    set_errno(EINVAL);
    set_dlerror_message(DLERROR_DLOPEN_NULL_PATH);

    return ptr::null_mut();
  }

  if let Err(errno_value) = validate_dlopen_flags(flags) {
    set_errno(errno_value);
    set_dlerror_message(DLERROR_DLOPEN_INVALID_FLAGS);

    return ptr::null_mut();
  }

  // SAFETY: caller guarantees `filename` points to a valid NUL-terminated string.
  let path_cstr = unsafe { CStr::from_ptr(filename) };
  let path = Path::new(OsStr::from_bytes(path_cstr.to_bytes()));

  if let Err(errno_value) = validate_elf_image(path) {
    set_errno(errno_value);

    if errno_value == ENOEXEC {
      set_dlerror_message(DLERROR_DLOPEN_NOT_ELF);
    } else {
      let path_detail = path.to_string_lossy();

      set_dlerror_message_with_detail(DLERROR_DLOPEN_PATH_OPEN_FAILED, Some(path_detail.as_ref()));
    }

    return ptr::null_mut();
  }

  let Some(host_dlopen) = host_dlopen() else {
    set_errno(EINVAL);
    set_dlerror_message(DLERROR_HOST_DLOPEN_UNAVAILABLE);

    return ptr::null_mut();
  };
  let saved_errno = current_errno();

  clear_host_dlerror_state();
  // SAFETY: host loader expects a valid path pointer and supported mode flags.
  let handle = unsafe { host_dlopen(filename, flags) };

  if handle.is_null() {
    let errno_value = io_error_errno(&io::Error::last_os_error(), EINVAL);
    let host_detail = take_host_dlerror_message();

    set_errno(errno_value);
    set_dlerror_message_with_detail(DLERROR_HOST_DLOPEN_FAILED, host_detail.as_deref());

    return ptr::null_mut();
  }

  {
    let mut registry = handle_registry_guard();

    registry.register_open_handle(handle);
  }

  set_errno(saved_errno);

  handle
}

/// C ABI entry point for `dlsym`.
///
/// Resolves `symbol` in the context described by `handle` and returns a
/// function/object address on success.
///
/// This implementation delegates lookup to the host runtime loader resolver and
/// reports failures through `dlerror` state.
///
/// Accepted handle values:
/// - `RTLD_DEFAULT` (null pointer),
/// - `RTLD_NEXT`,
/// - handles previously returned by this crate's [`dlopen`] and not yet closed.
///
/// Returns:
/// - non-null symbol pointer on success
/// - null pointer on failure (`dlerror` stores a deterministic thread-local
///   message for null `symbol`, invalid/closed `handle`, unresolved host
///   resolver, or missing symbol)
///
/// Validation precedence:
/// - for non-special handles, handle validity is checked first;
/// - null `symbol` is reported when the handle is one of the accepted
///   resolver sentinels or an open tracked handle.
/// - missing-symbol failures include the requested symbol name in `dlerror`
///   and append host-loader detail text when available.
/// - empty symbol names are rendered as `<empty symbol>` in diagnostics and
///   use a canonical message without a host-detail suffix.
/// - leading/trailing whitespace in host detail is trimmed before duplicate
///   symbol-prefix normalization.
/// - when host detail already starts with `<symbol>:`, that duplicate symbol
///   prefix is normalized away in the final diagnostic.
/// - duplicate-prefix normalization is only applied when `<symbol>` is
///   followed by a separator colon (with optional surrounding whitespace).
/// - duplicate symbol-prefix normalization also accepts optional whitespace
///   before the host-detail colon (for example `<symbol> : detail`).
/// - repeated colons after the symbol prefix are collapsed during
///   normalization (for example `<symbol>::detail`).
/// - colon-collapsing also normalizes whitespace-delimited colon runs (for
///   example `<symbol>: : detail`).
/// - repeated `<symbol>:` prefix chains in host detail are also collapsed to
///   avoid duplicate symbol echoes in the final diagnostic.
/// - host details containing only the symbol label (with optional surrounding
///   spaces) are treated as empty and omitted.
///
/// This function preserves the calling thread's `errno` value.
///
/// # Safety
/// - `symbol` must point to a valid NUL-terminated C string.
/// - `handle` must be a valid dynamic-loader handle or one of the loader
///   sentinel values expected by the host runtime.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
  let saved_errno = current_errno();
  let fail_with_message = |message: &'static str| -> *mut c_void {
    set_dlerror_message(message);
    set_errno(saved_errno);
    ptr::null_mut()
  };

  if let Err(message) = validate_dlsym_handle(handle) {
    return fail_with_message(message);
  }

  if symbol.is_null() {
    return fail_with_message(DLERROR_NULL_SYMBOL);
  }

  let Some(host_dlsym) = host_dlsym() else {
    return fail_with_message(DLERROR_HOST_DLSYM_UNAVAILABLE);
  };

  clear_host_dlerror_state();

  // SAFETY: Caller must provide loader-valid `handle`/`symbol` arguments.
  let resolved = unsafe { host_dlsym(handle, symbol) };

  set_errno(saved_errno);

  if resolved.is_null() {
    let host_detail = take_host_dlerror_message();

    set_dlsym_missing_symbol_message(symbol, host_detail.as_deref());
    set_errno(saved_errno);
  }

  resolved
}

#[cfg(test)]
mod tests {
  use std::ffi::CStr;
  use std::{io, thread};

  use super::{
    DLCLOSE_FAILURE, DLCLOSE_SUCCESS, DLERROR_ALREADY_CLOSED, DLERROR_INVALID_HANDLE,
    DlHandleRegistry, DlHandleState, RTLD_BINDING_MASK, RTLD_GLOBAL, RTLD_NOW, RTLD_SUPPORTED_MASK,
    TRACKABLE_NULL_HANDLE_ID, clear_dlerror_state, dlclose, dlerror, handle_registry_guard,
    io_error_errno, set_dlerror_message_with_detail, set_dlsym_missing_symbol_message,
    validate_dlopen_flags, validate_dlsym_handle,
  };
  use crate::abi::errno::{EACCES, EAGAIN, EEXIST, EISDIR, ENOENT, ENOEXEC, ENOTDIR};
  use crate::abi::types::c_void;

  fn allocate_test_handle(initial_refcount: usize) -> *mut c_void {
    let mut registry = handle_registry_guard();

    registry.allocate_test_handle(initial_refcount)
  }

  fn test_handle_state(handle: *mut c_void) -> Option<DlHandleState> {
    let registry = handle_registry_guard();

    registry.handle_state(handle as usize)
  }

  fn take_dlerror_message() -> Option<String> {
    let message_ptr = dlerror();

    if message_ptr.is_null() {
      return None;
    }

    // SAFETY: `dlerror` returns either null or a valid NUL-terminated message pointer.
    let message = unsafe { CStr::from_ptr(message_ptr.cast_const()) };

    Some(message.to_string_lossy().into_owned())
  }

  fn reset_thread_local_error_state() {
    clear_dlerror_state();

    while take_dlerror_message().is_some() {}
  }

  #[test]
  fn set_dlerror_message_with_empty_detail_uses_base_message() {
    reset_thread_local_error_state();

    set_dlerror_message_with_detail("rlibc: requested symbol was not found", Some(""));

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(message, "rlibc: requested symbol was not found");
  }

  #[test]
  fn set_dlsym_missing_symbol_message_empty_symbol_ignores_detail_payload() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(c"".as_ptr(), Some("host detail should be ignored"));

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: <empty symbol>"
    );
  }

  #[test]
  fn set_dlsym_missing_symbol_message_deduplicates_symbol_prefixed_host_detail() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("dup_symbol: host loader unresolved entry"),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: dup_symbol: host loader unresolved entry",
    );
  }

  #[test]
  fn set_dlsym_missing_symbol_message_deduplicates_symbol_prefix_with_spacing() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("dup_symbol : host loader unresolved entry"),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: dup_symbol: host loader unresolved entry",
    );
  }

  #[test]
  fn set_dlsym_missing_symbol_message_deduplicates_symbol_prefix_with_leading_space() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("  dup_symbol: host loader unresolved entry"),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: dup_symbol: host loader unresolved entry",
    );
  }

  #[test]
  fn set_dlsym_missing_symbol_message_keeps_non_separator_symbol_prefix_detail() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup".as_ptr(),
      Some("dup_symbol: host loader unresolved entry"),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: dup: dup_symbol: host loader unresolved entry",
    );
  }

  #[test]
  fn set_dlsym_missing_symbol_message_ignores_empty_detail_after_symbol_prefix() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(c"dup_symbol".as_ptr(), Some("dup_symbol :   "));

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(message, "rlibc: requested symbol was not found: dup_symbol");
  }

  #[test]
  fn set_dlsym_missing_symbol_message_ignores_symbol_only_detail() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(c"dup_symbol".as_ptr(), Some("dup_symbol"));

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(message, "rlibc: requested symbol was not found: dup_symbol");
  }

  #[test]
  fn set_dlsym_missing_symbol_message_collapses_repeated_colons_after_symbol_prefix() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("dup_symbol:: host loader unresolved entry"),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: dup_symbol: host loader unresolved entry",
    );
  }

  #[test]
  fn set_dlsym_missing_symbol_message_collapses_spaced_repeated_colons_after_symbol_prefix() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("dup_symbol: : host loader unresolved entry"),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: dup_symbol: host loader unresolved entry",
    );
  }

  #[test]
  fn set_dlsym_missing_symbol_message_collapses_repeated_symbol_prefix_chain() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("dup_symbol: dup_symbol: host loader unresolved entry"),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: dup_symbol: host loader unresolved entry",
    );
  }

  #[test]
  fn set_dlsym_missing_symbol_message_omits_detail_after_full_symbol_prefix_chain_collapse() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("dup_symbol: dup_symbol: dup_symbol"),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(message, "rlibc: requested symbol was not found: dup_symbol");
  }

  #[test]
  fn set_dlsym_missing_symbol_message_omits_detail_after_spaced_prefix_chain_collapse() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("  dup_symbol : dup_symbol :   "),
    );

    let message = take_dlerror_message().expect("expected pending dlerror message");

    assert_eq!(message, "rlibc: requested symbol was not found: dup_symbol");
  }

  #[test]
  fn dlclose_decrements_refcount_when_handle_is_still_open() {
    reset_thread_local_error_state();

    let handle = allocate_test_handle(2);

    assert_eq!(dlclose(handle), DLCLOSE_SUCCESS);
    assert_eq!(
      test_handle_state(handle),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn dlclose_final_reference_marks_handle_closed() {
    reset_thread_local_error_state();

    let handle = allocate_test_handle(1);

    assert_eq!(dlclose(handle), DLCLOSE_SUCCESS);
    assert_eq!(test_handle_state(handle), Some(DlHandleState::Closed));
  }

  #[test]
  fn dlclose_reports_error_when_handle_is_already_closed() {
    reset_thread_local_error_state();

    let handle = allocate_test_handle(1);

    assert_eq!(dlclose(handle), DLCLOSE_SUCCESS);
    assert_eq!(dlclose(handle), DLCLOSE_FAILURE);

    let message = take_dlerror_message().expect("expected dlerror message after close failure");

    assert!(
      message.contains(DLERROR_ALREADY_CLOSED),
      "unexpected dlerror message: {message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "second dlerror call must clear pending state",
    );
  }

  #[test]
  fn dlclose_reports_error_when_handle_is_unknown() {
    reset_thread_local_error_state();

    let unknown_handle = 0xFF00usize as *mut c_void;

    assert_eq!(dlclose(unknown_handle), DLCLOSE_FAILURE);

    let message = take_dlerror_message().expect("expected dlerror message for unknown handle");

    assert!(
      message.contains(DLERROR_INVALID_HANDLE),
      "unexpected dlerror message: {message}",
    );
  }

  #[test]
  fn dlclose_reports_error_when_handle_is_null() {
    reset_thread_local_error_state();

    assert_eq!(dlclose(core::ptr::null_mut()), DLCLOSE_FAILURE);

    let message = take_dlerror_message().expect("expected dlerror message for null handle");

    assert!(
      message.contains(DLERROR_INVALID_HANDLE),
      "unexpected dlerror message: {message}",
    );
  }

  #[test]
  fn dlclose_null_handle_cleans_trackable_zero_entry() {
    reset_thread_local_error_state();

    {
      let mut registry = handle_registry_guard();

      registry.handles.insert(
        TRACKABLE_NULL_HANDLE_ID,
        DlHandleState::Open { refcount: 2 },
      );
    }

    assert_eq!(dlclose(core::ptr::null_mut()), DLCLOSE_FAILURE);
    assert_eq!(test_handle_state(core::ptr::null_mut()), None);

    let message = take_dlerror_message().expect("expected dlerror message after null-handle close");

    assert!(
      message.contains(DLERROR_INVALID_HANDLE),
      "unexpected dlerror message: {message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "second dlerror call should clear pending state",
    );
  }

  #[test]
  fn dlclose_null_handle_cleanup_keeps_other_handles_open() {
    reset_thread_local_error_state();

    let tracked_handle = allocate_test_handle(2);

    {
      let mut registry = handle_registry_guard();

      registry.handles.insert(
        TRACKABLE_NULL_HANDLE_ID,
        DlHandleState::Open { refcount: 2 },
      );
    }

    assert_eq!(dlclose(core::ptr::null_mut()), DLCLOSE_FAILURE);
    assert_eq!(test_handle_state(core::ptr::null_mut()), None);
    assert_eq!(
      test_handle_state(tracked_handle),
      Some(DlHandleState::Open { refcount: 2 })
    );

    let message = take_dlerror_message().expect("expected dlerror message after null-handle close");

    assert!(
      message.contains(DLERROR_INVALID_HANDLE),
      "unexpected dlerror message: {message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "second dlerror call should clear pending state",
    );
  }

  #[test]
  fn concurrent_dlclose_calls_do_not_underflow_refcount() {
    let handle = allocate_test_handle(1);
    let handle_value = handle as usize;
    let mut workers = Vec::new();

    for _ in 0..8 {
      workers.push(thread::spawn(move || dlclose(handle_value as *mut c_void)));
    }

    let mut success_count = 0usize;
    let mut failure_count = 0usize;

    for worker in workers {
      let close_result = worker.join().expect("worker thread panicked");

      if close_result == DLCLOSE_SUCCESS {
        success_count += 1;
      } else if close_result == DLCLOSE_FAILURE {
        failure_count += 1;
      } else {
        panic!("unexpected dlclose result: {close_result}");
      }
    }

    assert_eq!(
      success_count, 1,
      "exactly one close should consume final reference"
    );
    assert_eq!(failure_count, 7, "all other close attempts must fail");
    assert_eq!(test_handle_state(handle), Some(DlHandleState::Closed));
  }

  #[test]
  fn validate_dlsym_handle_accepts_special_handles_and_rejects_closed_entries() {
    let tracked_handle = allocate_test_handle(1);

    assert_eq!(validate_dlsym_handle(core::ptr::null_mut()), Ok(()));
    assert_eq!(validate_dlsym_handle(super::RTLD_NEXT), Ok(()));
    assert_eq!(dlclose(tracked_handle), DLCLOSE_SUCCESS);
    assert_eq!(
      validate_dlsym_handle(tracked_handle),
      Err(DLERROR_ALREADY_CLOSED)
    );
  }

  #[test]
  fn validate_dlsym_handle_rejects_unknown_non_special_handle() {
    let unknown_handle = 0xDEADusize as *mut c_void;

    assert_eq!(
      validate_dlsym_handle(unknown_handle),
      Err(DLERROR_INVALID_HANDLE)
    );
  }

  #[test]
  fn validate_dlopen_flags_accepts_supported_binding_modes() {
    assert_eq!(validate_dlopen_flags(RTLD_NOW), Ok(()));
    assert_eq!(validate_dlopen_flags(RTLD_NOW | RTLD_GLOBAL), Ok(()));
  }

  #[test]
  fn validate_dlopen_flags_rejects_invalid_bit_patterns() {
    assert!(validate_dlopen_flags(0).is_err());
    assert!(validate_dlopen_flags(RTLD_BINDING_MASK).is_err());
    assert!(validate_dlopen_flags(RTLD_SUPPORTED_MASK | 0x8000).is_err());
  }

  #[test]
  fn register_open_handle_tracks_refcount_for_reused_handle() {
    let handle = 0xD1usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry.register_open_handle(handle);
    registry.register_open_handle(handle);

    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open { refcount: 2 })
    );
  }

  #[test]
  fn register_open_handle_reopens_closed_handle_with_fresh_refcount() {
    let handle = 0xD2usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry.register_open_handle(handle);
    assert_eq!(
      registry.close_handle(handle as usize),
      crate::dlfcn::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Closed)
    );

    registry.register_open_handle(handle);

    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn reopen_then_close_marks_handle_closed_again() {
    let handle = 0xD3usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry.register_open_handle(handle);
    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Closed)
    );

    registry.register_open_handle(handle);
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );

    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Closed)
    );
  }

  #[test]
  fn reopen_then_double_open_restarts_refcount_chain() {
    let handle = 0xD4usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry.register_open_handle(handle);
    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Closed)
    );

    registry.register_open_handle(handle);
    registry.register_open_handle(handle);
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open { refcount: 2 })
    );

    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );

    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Closed)
    );
  }

  #[test]
  fn register_open_handle_saturates_refcount_at_max() {
    let handle = 0xD5usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry.handles.insert(
      handle as usize,
      DlHandleState::Open {
        refcount: usize::MAX,
      },
    );

    registry.register_open_handle(handle);

    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open {
        refcount: usize::MAX,
      })
    );
  }

  #[test]
  fn close_handle_decrements_saturated_refcount_without_underflow() {
    let handle = 0xD6usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry.handles.insert(
      handle as usize,
      DlHandleState::Open {
        refcount: usize::MAX,
      },
    );

    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open {
        refcount: usize::MAX - 1,
      })
    );
  }

  #[test]
  fn close_handle_with_zero_refcount_marks_closed_without_underflow() {
    let handle = 0xD7usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry
      .handles
      .insert(handle as usize, DlHandleState::Open { refcount: 0 });

    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Closed)
    );
  }

  #[test]
  fn register_open_handle_recovers_zero_refcount_entry() {
    let handle = 0xD8usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry
      .handles
      .insert(handle as usize, DlHandleState::Open { refcount: 0 });

    registry.register_open_handle(handle);

    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_recovers_zero_refcount_entry_and_keeps_cursor_monotonic() {
    let handle = 0x2A0usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry
      .handles
      .insert(handle as usize, DlHandleState::Open { refcount: 0 });

    registry.register_open_handle(handle);

    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 0x2A1usize);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn allocate_test_handle_clamps_zero_refcount_to_one() {
    let mut registry = DlHandleRegistry::new();
    let handle = registry.allocate_test_handle(0);

    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn close_unknown_handle_does_not_mutate_other_open_handle() {
    let mut registry = DlHandleRegistry::new();
    let tracked_handle = registry.allocate_test_handle(2);
    let unknown_handle_id = 0xFFFFusize;

    assert_eq!(
      registry.close_handle(unknown_handle_id),
      super::CloseOutcome::InvalidHandle
    );
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Open { refcount: 2 })
    );
  }

  #[test]
  fn close_unknown_handle_cleans_trackable_zero_entry_and_preserves_open_handles() {
    let mut registry = DlHandleRegistry::new();
    let tracked_handle = registry.allocate_test_handle(2);
    let unknown_handle_id = 0xFFF0usize;

    registry.handles.insert(
      TRACKABLE_NULL_HANDLE_ID,
      DlHandleState::Open { refcount: 3 },
    );

    assert_eq!(
      registry.close_handle(unknown_handle_id),
      super::CloseOutcome::InvalidHandle
    );
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Open { refcount: 2 })
    );
  }

  #[test]
  fn close_closed_handle_cleans_trackable_zero_entry_and_preserves_closed_state() {
    let mut registry = DlHandleRegistry::new();
    let tracked_handle = registry.allocate_test_handle(1);

    assert_eq!(
      registry.close_handle(tracked_handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Closed)
    );

    registry.handles.insert(
      TRACKABLE_NULL_HANDLE_ID,
      DlHandleState::Open { refcount: 2 },
    );

    assert_eq!(
      registry.close_handle(tracked_handle as usize),
      super::CloseOutcome::AlreadyClosed
    );
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Closed)
    );
  }

  #[test]
  fn close_open_handle_cleans_trackable_zero_entry_and_preserves_refcount_transition() {
    let mut registry = DlHandleRegistry::new();
    let tracked_handle = registry.allocate_test_handle(2);

    registry.handles.insert(
      TRACKABLE_NULL_HANDLE_ID,
      DlHandleState::Open { refcount: 5 },
    );

    assert_eq!(
      registry.close_handle(tracked_handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn close_open_handle_cleans_trackable_zero_closed_entry_and_preserves_refcount_transition() {
    let mut registry = DlHandleRegistry::new();
    let tracked_handle = registry.allocate_test_handle(2);

    registry
      .handles
      .insert(TRACKABLE_NULL_HANDLE_ID, DlHandleState::Closed);

    assert_eq!(
      registry.close_handle(tracked_handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn close_final_reference_cleans_trackable_zero_entry_and_marks_closed() {
    let mut registry = DlHandleRegistry::new();
    let tracked_handle = registry.allocate_test_handle(1);

    registry.handles.insert(
      TRACKABLE_NULL_HANDLE_ID,
      DlHandleState::Open { refcount: 4 },
    );

    assert_eq!(
      registry.close_handle(tracked_handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Closed)
    );
  }

  #[test]
  fn close_final_reference_cleans_trackable_zero_closed_entry_and_marks_closed() {
    let mut registry = DlHandleRegistry::new();
    let tracked_handle = registry.allocate_test_handle(1);

    registry
      .handles
      .insert(TRACKABLE_NULL_HANDLE_ID, DlHandleState::Closed);

    assert_eq!(
      registry.close_handle(tracked_handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Closed)
    );
  }

  #[test]
  fn close_handle_rejects_trackable_zero_entry_as_invalid_and_cleans_state() {
    let mut registry = DlHandleRegistry::new();

    registry.handles.insert(
      TRACKABLE_NULL_HANDLE_ID,
      DlHandleState::Open { refcount: 2 },
    );

    assert_eq!(registry.close_handle(0), super::CloseOutcome::InvalidHandle);
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn close_handle_rejects_trackable_zero_closed_entry_idempotently() {
    let mut registry = DlHandleRegistry::new();

    registry
      .handles
      .insert(TRACKABLE_NULL_HANDLE_ID, DlHandleState::Closed);

    assert_eq!(
      registry.close_handle(TRACKABLE_NULL_HANDLE_ID),
      super::CloseOutcome::InvalidHandle
    );
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.close_handle(TRACKABLE_NULL_HANDLE_ID),
      super::CloseOutcome::InvalidHandle
    );
    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_advances_allocation_cursor() {
    let mut registry = DlHandleRegistry::new();
    let existing_handle = 0xD8usize as *mut c_void;

    registry.register_open_handle(existing_handle);

    let newly_allocated = registry.allocate_test_handle(1);

    assert_eq!(newly_allocated as usize, 0xD9usize);
    assert_eq!(
      registry.handle_state(newly_allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_with_max_handle_keeps_allocation_cursor_stable() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry.register_open_handle(max_handle);

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_ignores_null_handle_without_tracking_state() {
    let mut registry = DlHandleRegistry::new();

    registry.register_open_handle(core::ptr::null_mut());

    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_null_handle_clears_trackable_zero_entry() {
    let mut registry = DlHandleRegistry::new();

    registry.handles.insert(
      TRACKABLE_NULL_HANDLE_ID,
      DlHandleState::Open { refcount: 3 },
    );

    registry.register_open_handle(core::ptr::null_mut());

    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_non_null_cleans_trackable_zero_entry_and_tracks_real_handle() {
    let mut registry = DlHandleRegistry::new();
    let tracked_handle = 0xD8usize as *mut c_void;

    registry
      .handles
      .insert(TRACKABLE_NULL_HANDLE_ID, DlHandleState::Closed);

    registry.register_open_handle(tracked_handle);

    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.handle_state(tracked_handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_with_max_handle_updates_existing_entry() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry.handles.insert(
      max_handle as usize,
      DlHandleState::Open {
        refcount: usize::MAX,
      },
    );

    registry.register_open_handle(max_handle);

    assert_eq!(
      registry.handle_state(max_handle as usize),
      Some(DlHandleState::Open {
        refcount: usize::MAX,
      })
    );

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_with_max_handle_reopens_closed_entry() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry
      .handles
      .insert(max_handle as usize, DlHandleState::Closed);

    registry.register_open_handle(max_handle);

    assert_eq!(
      registry.handle_state(max_handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_with_max_handle_recovers_zero_refcount_entry() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry
      .handles
      .insert(max_handle as usize, DlHandleState::Open { refcount: 0 });

    registry.register_open_handle(max_handle);

    assert_eq!(
      registry.handle_state(max_handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_with_max_handle_increments_existing_refcount() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry
      .handles
      .insert(max_handle as usize, DlHandleState::Open { refcount: 1 });

    registry.register_open_handle(max_handle);

    assert_eq!(
      registry.handle_state(max_handle as usize),
      Some(DlHandleState::Open { refcount: 2 })
    );

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn close_handle_with_max_handle_entry_keeps_cursor_stable() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry
      .handles
      .insert(max_handle as usize, DlHandleState::Open { refcount: 2 });

    assert_eq!(
      registry.close_handle(max_handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(max_handle as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
    assert_eq!(
      registry.close_handle(max_handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(max_handle as usize),
      Some(DlHandleState::Closed)
    );

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 1);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn register_open_handle_with_max_handle_does_not_create_trackable_entry() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry.register_open_handle(max_handle);

    assert_eq!(registry.handle_state(max_handle as usize), None);
    assert_eq!(
      registry.close_handle(max_handle as usize),
      super::CloseOutcome::InvalidHandle
    );
  }

  #[test]
  fn register_open_handle_with_lower_id_does_not_rewind_allocation_cursor() {
    let mut registry = DlHandleRegistry::new();
    let high_handle = 0x200usize as *mut c_void;
    let low_handle = 0x10usize as *mut c_void;

    registry.register_open_handle(high_handle);
    registry.register_open_handle(low_handle);

    let allocated = registry.allocate_test_handle(1);

    assert_eq!(allocated as usize, 0x201usize);
    assert_eq!(
      registry.handle_state(allocated as usize),
      Some(DlHandleState::Open { refcount: 1 })
    );
  }

  #[test]
  fn close_already_closed_handle_keeps_closed_state() {
    let mut registry = DlHandleRegistry::new();
    let handle = registry.allocate_test_handle(1);

    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::Success
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Closed)
    );

    assert_eq!(
      registry.close_handle(handle as usize),
      super::CloseOutcome::AlreadyClosed
    );
    assert_eq!(
      registry.handle_state(handle as usize),
      Some(DlHandleState::Closed)
    );
  }

  #[test]
  fn io_error_errno_prefers_non_zero_raw_os_error() {
    let error = io::Error::from_raw_os_error(EACCES);

    assert_eq!(io_error_errno(&error, ENOENT), EACCES);
  }

  #[test]
  fn io_error_errno_uses_error_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");

    assert_eq!(io_error_errno(&error, ENOENT), EACCES);
  }

  #[test]
  fn io_error_errno_maps_is_a_directory_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::IsADirectory, "is a directory");

    assert_eq!(io_error_errno(&error, ENOENT), EISDIR);
  }

  #[test]
  fn io_error_errno_maps_not_a_directory_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::NotADirectory, "not a directory");

    assert_eq!(io_error_errno(&error, ENOENT), ENOTDIR);
  }

  #[test]
  fn io_error_errno_maps_already_exists_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::AlreadyExists, "already exists");

    assert_eq!(io_error_errno(&error, ENOENT), EEXIST);
  }

  #[test]
  fn io_error_errno_maps_would_block_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::WouldBlock, "would block");

    assert_eq!(io_error_errno(&error, ENOENT), EAGAIN);
  }

  #[test]
  fn io_error_errno_maps_timed_out_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::TimedOut, "timed out");

    assert_eq!(io_error_errno(&error, ENOENT), crate::abi::errno::ETIMEDOUT);
  }

  #[test]
  fn io_error_errno_maps_broken_pipe_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe");

    assert_eq!(io_error_errno(&error, ENOENT), crate::abi::errno::EPIPE);
  }

  #[test]
  fn io_error_errno_maps_connection_refused_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::ConnectionRefused, "connection refused");

    assert_eq!(
      io_error_errno(&error, ENOENT),
      crate::abi::errno::ECONNREFUSED
    );
  }

  #[test]
  fn io_error_errno_maps_connection_reset_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::ConnectionReset, "connection reset");

    assert_eq!(
      io_error_errno(&error, ENOENT),
      crate::abi::errno::ECONNRESET
    );
  }

  #[test]
  fn io_error_errno_maps_connection_aborted_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::ConnectionAborted, "connection aborted");

    assert_eq!(
      io_error_errno(&error, ENOENT),
      crate::abi::errno::ECONNABORTED
    );
  }

  #[test]
  fn io_error_errno_maps_not_connected_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::NotConnected, "not connected");

    assert_eq!(io_error_errno(&error, ENOENT), crate::abi::errno::ENOTCONN);
  }

  #[test]
  fn io_error_errno_maps_addr_in_use_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::AddrInUse, "address in use");

    assert_eq!(
      io_error_errno(&error, ENOENT),
      crate::abi::errno::EADDRINUSE
    );
  }

  #[test]
  fn io_error_errno_maps_addr_not_available_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::AddrNotAvailable, "address not available");

    assert_eq!(
      io_error_errno(&error, ENOENT),
      crate::abi::errno::EADDRNOTAVAIL
    );
  }

  #[test]
  fn io_error_errno_maps_network_unreachable_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::NetworkUnreachable, "network unreachable");

    assert_eq!(
      io_error_errno(&error, ENOENT),
      crate::abi::errno::ENETUNREACH
    );
  }

  #[test]
  fn io_error_errno_maps_network_down_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::NetworkDown, "network down");

    assert_eq!(io_error_errno(&error, ENOENT), crate::abi::errno::ENETDOWN);
  }

  #[test]
  fn io_error_errno_maps_host_unreachable_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::HostUnreachable, "host unreachable");

    assert_eq!(
      io_error_errno(&error, ENOENT),
      crate::abi::errno::EHOSTUNREACH
    );
  }

  #[test]
  fn io_error_errno_preserves_einprogress_raw_errno_when_present() {
    let error = io::Error::from_raw_os_error(crate::abi::errno::EINPROGRESS);

    assert_eq!(
      io_error_errno(&error, ENOENT),
      crate::abi::errno::EINPROGRESS
    );
  }

  #[test]
  fn io_error_errno_falls_back_when_kind_mapping_is_unavailable() {
    let error = io::Error::other("opaque host error");

    assert_eq!(io_error_errno(&error, ENOEXEC), ENOEXEC);
  }
}
