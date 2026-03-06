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
  EACCES, EADDRINUSE, EADDRNOTAVAIL, EAGAIN, EBUSY, ECONNABORTED, ECONNREFUSED, ECONNRESET, EEXIST,
  EHOSTUNREACH, EINTR, EINVAL, EISDIR, ENETDOWN, ENETUNREACH, ENOENT, ENOEXEC, ENOTCONN, ENOTDIR,
  ENOTEMPTY, EPIPE, ETIMEDOUT,
};
use crate::abi::types::{
  c_char, c_int, c_long, c_longlong, c_uint, c_ulong, c_ulonglong, c_void, size_t, ssize_t,
};
use crate::errno::{__errno_location, set_errno};
use core::{mem, ptr};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

const DLCLOSE_SUCCESS: c_int = 0;
const DLCLOSE_FAILURE: c_int = -1;
const DLERROR_INVALID_HANDLE: &str = "rlibc: invalid dynamic-loader handle";
const DLERROR_ALREADY_CLOSED: &str = "rlibc: dynamic-loader handle already closed";
const DLERROR_NULL_SYMBOL: &str = "rlibc: dlsym symbol pointer is null";
const DLERROR_DLOPEN_INVALID_FLAGS: &str = "rlibc: dlopen received invalid flags";
const DLERROR_DLOPEN_NOT_ELF: &str = "rlibc: dlopen target is not a valid ELF image";
const DLERROR_DLOPEN_PATH_OPEN_FAILED: &str = "rlibc: dlopen target path could not be opened";
const DLERROR_DLOPEN_MAIN_PROGRAM_UNAVAILABLE: &str =
  "rlibc: main-program dlopen handle is unavailable";
const DLERROR_HOST_DLOPEN_UNAVAILABLE: &str = "rlibc: host dlopen resolver unavailable";
const DLERROR_HOST_DLOPEN_FAILED: &str = "rlibc: host dlopen call failed";
const DLERROR_HOST_DLSYM_UNAVAILABLE: &str = "rlibc: host dlsym resolver unavailable";
const DLERROR_SYMBOL_NOT_FOUND: &str = "rlibc: requested symbol was not found";
const TRACKABLE_NULL_HANDLE_ID: usize = 0;
const INTERNAL_MAIN_PROGRAM_HANDLE_ID: usize = usize::MAX - 2;
const INTERNAL_PROC_SELF_EXE_HANDLE_ID: usize = usize::MAX - 1;
const PROC_SELF_EXE_PATH: &str = "/proc/self/exe";
/// Runtime loader mode flag: resolve symbols lazily.
pub const RTLD_LAZY: c_int = 0x0001;
/// Runtime loader mode flag: resolve symbols immediately.
pub const RTLD_NOW: c_int = 0x0002;
/// Runtime loader visibility flag: make symbols available for later lookups.
pub const RTLD_GLOBAL: c_int = 0x0100;
/// Runtime loader visibility flag: keep symbols local to the opened object.
pub const RTLD_LOCAL: c_int = 0;
/// Runtime loader handle sentinel: search for the next definition after the caller.
pub const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
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
#[cfg(test)]
static FORCED_INTERNAL_DLOPEN_HANDLE_FOR_TESTS: AtomicUsize = AtomicUsize::new(0);

type HostDlopenFn = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;

type HostDlerrorFn = unsafe extern "C" fn() -> *mut c_char;

type HostDlsymFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

#[link(name = "dl")]
unsafe extern "C" {
  #[link_name = "dlsym"]
  fn host_dlsym_unversioned(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;

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
  internal_handles: HashSet<usize>,
  main_program_handles: HashSet<usize>,
  next_handle_id: usize,
}

struct DlErrorState {
  pending_message: Option<CString>,
  last_returned_message: Option<CString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DlsymHandleKind {
  Default,
  Next,
  MainProgram,
  Internal,
  External,
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
      internal_handles: HashSet::new(),
      main_program_handles: HashSet::new(),
      next_handle_id: 1,
    }
  }

  fn register_open_handle(&mut self, handle: *mut c_void) {
    self.register_open_handle_with_origin(handle, false, false);
  }

  fn register_open_internal_handle(&mut self, handle: *mut c_void) {
    self.register_open_handle_with_origin(handle, true, false);
  }

  fn register_open_main_program_handle(&mut self, handle: *mut c_void) {
    let is_internal = handle as usize == INTERNAL_MAIN_PROGRAM_HANDLE_ID;

    self.register_open_handle_with_origin(handle, is_internal, true);
  }

  fn register_open_handle_with_origin(
    &mut self,
    handle: *mut c_void,
    is_internal: bool,
    is_main_program: bool,
  ) {
    if handle.is_null() {
      self.clear_trackable_null_handle_entry();

      return;
    }

    self.clear_trackable_null_handle_entry();

    let handle_id = handle as usize;

    if is_internal {
      if let Some(handle_state) = self.handles.get_mut(&handle_id) {
        *handle_state = match *handle_state {
          DlHandleState::Open { refcount } => DlHandleState::Open {
            refcount: refcount.saturating_add(1),
          },
          DlHandleState::Closed => DlHandleState::Open { refcount: 1 },
        };
        self.set_handle_origin(handle_id, true, is_main_program);

        return;
      }

      self.set_handle_origin(handle_id, true, is_main_program);
      self
        .handles
        .insert(handle_id, DlHandleState::Open { refcount: 1 });

      return;
    }

    let Some(next_after_handle) = handle_id.checked_add(1) else {
      if let Some(handle_state) = self.handles.get_mut(&handle_id) {
        *handle_state = match *handle_state {
          DlHandleState::Open { refcount } => DlHandleState::Open {
            refcount: refcount.saturating_add(1),
          },
          DlHandleState::Closed => DlHandleState::Open { refcount: 1 },
        };
        self.set_handle_origin(handle_id, false, is_main_program);
      }

      return;
    };

    self.next_handle_id = self.next_handle_id.max(next_after_handle);

    if let Some(DlHandleState::Open { refcount }) = self.handles.get_mut(&handle_id) {
      *refcount = refcount.saturating_add(1);
      self.set_handle_origin(handle_id, false, is_main_program);

      return;
    }

    self.set_handle_origin(handle_id, false, is_main_program);
    self
      .handles
      .insert(handle_id, DlHandleState::Open { refcount: 1 });
  }

  fn set_handle_origin(&mut self, handle_id: usize, is_internal: bool, is_main_program: bool) {
    if is_internal {
      self.internal_handles.insert(handle_id);
    } else {
      self.internal_handles.remove(&handle_id);
    }

    if is_main_program {
      self.main_program_handles.insert(handle_id);
    } else {
      self.main_program_handles.remove(&handle_id);
    }
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

  fn is_internal_open_handle(&self, handle_id: usize) -> bool {
    matches!(
      self.handles.get(&handle_id),
      Some(DlHandleState::Open { .. })
    ) && self.internal_handles.contains(&handle_id)
  }

  fn is_main_program_open_handle(&self, handle_id: usize) -> bool {
    matches!(
      self.handles.get(&handle_id),
      Some(DlHandleState::Open { .. })
    ) && self.main_program_handles.contains(&handle_id)
  }

  fn clear_trackable_null_handle_entry(&mut self) {
    self.handles.remove(&TRACKABLE_NULL_HANDLE_ID);
    self.internal_handles.remove(&TRACKABLE_NULL_HANDLE_ID);
    self.main_program_handles.remove(&TRACKABLE_NULL_HANDLE_ID);
  }

  #[cfg(test)]
  fn allocate_test_handle(&mut self, initial_refcount: usize) -> *mut c_void {
    let handle_id = self.next_handle_id;
    let refcount = initial_refcount.max(1);

    self.next_handle_id = self.next_handle_id.saturating_add(1);
    self.internal_handles.remove(&handle_id);
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

fn classify_dlsym_handle(
  handle: *mut c_void,
  registry: Option<&DlHandleRegistry>,
) -> Result<DlsymHandleKind, &'static str> {
  if handle.is_null() {
    return Ok(DlsymHandleKind::Default);
  }

  if handle == RTLD_NEXT {
    return Ok(DlsymHandleKind::Next);
  }

  let Some(registry) = registry else {
    let registry_guard = handle_registry_guard();

    return classify_dlsym_handle(handle, Some(&registry_guard));
  };

  registry.validate_dlsym_handle(handle as usize)?;

  Ok(if registry.is_main_program_open_handle(handle as usize) {
    DlsymHandleKind::MainProgram
  } else if registry.is_internal_open_handle(handle as usize) {
    DlsymHandleKind::Internal
  } else {
    DlsymHandleKind::External
  })
}

#[cfg(test)]
fn validate_dlsym_handle(handle: *mut c_void) -> Result<(), &'static str> {
  classify_dlsym_handle(handle, None).map(|_| ())
}

#[cfg(test)]
fn is_registered_internal_dlopen_handle(handle: *mut c_void) -> bool {
  matches!(
    classify_dlsym_handle(handle, None),
    Ok(DlsymHandleKind::Internal)
  )
}

fn resolve_rlibc_symbol(symbol_name: &[u8]) -> Option<*mut c_void> {
  resolve_rlibc_symbol_stdlib_io_fs(symbol_name)
    .or_else(|| resolve_rlibc_symbol_system_resource_time(symbol_name))
    .or_else(|| resolve_rlibc_symbol_net_socket(symbol_name))
    .or_else(|| resolve_rlibc_symbol_pthread_sync(symbol_name))
    .or_else(|| resolve_rlibc_symbol_misc(symbol_name))
    .or_else(|| resolve_rlibc_symbol_stdio_pthread_dlfcn(symbol_name))
}

fn resolve_rlibc_symbol_stdlib_io_fs(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"environ" => {
      let symbol: *mut *mut *mut c_char = &raw mut crate::stdlib::environ;

      Some(symbol.cast::<c_void>())
    }
    b"getenv" => {
      let symbol =
        crate::stdlib::env::core::getenv as unsafe extern "C" fn(*const c_char) -> *mut c_char;

      Some(symbol as *const () as *mut c_void)
    }
    b"setenv" => {
      let symbol = crate::stdlib::env::mutating::setenv
        as unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"unsetenv" => {
      let symbol =
        crate::stdlib::env::mutating::unsetenv as unsafe extern "C" fn(*const c_char) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"putenv" => {
      let symbol =
        crate::stdlib::env::mutating::putenv as unsafe extern "C" fn(*mut c_char) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"clearenv" => {
      let symbol = crate::stdlib::env::mutating::clearenv as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"malloc" => {
      let symbol = crate::stdlib::alloc::malloc_c_abi as unsafe extern "C" fn(usize) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"calloc" => {
      let symbol =
        crate::stdlib::alloc::calloc_c_abi as unsafe extern "C" fn(usize, usize) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"aligned_alloc" => {
      let symbol = crate::stdlib::alloc::aligned_alloc_c_abi
        as unsafe extern "C" fn(usize, usize) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"posix_memalign" => {
      let symbol = crate::stdlib::alloc::posix_memalign_c_abi
        as unsafe extern "C" fn(*mut *mut c_void, usize, usize) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"memalign" => {
      let symbol =
        crate::stdlib::alloc::memalign_c_abi as unsafe extern "C" fn(usize, usize) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"valloc" => {
      let symbol = crate::stdlib::alloc::valloc_c_abi as unsafe extern "C" fn(usize) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"pvalloc" => {
      let symbol =
        crate::stdlib::alloc::pvalloc_c_abi as unsafe extern "C" fn(usize) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"realloc" => {
      let symbol = crate::stdlib::alloc::realloc_c_abi
        as unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"reallocarray" => {
      let symbol = crate::stdlib::alloc::reallocarray_c_abi
        as unsafe extern "C" fn(*mut c_void, usize, usize) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"free" => {
      let symbol = crate::stdlib::alloc::free_c_abi as unsafe extern "C" fn(*mut c_void);

      Some(symbol as *const () as *mut c_void)
    }
    b"cfree" => {
      let symbol = crate::stdlib::alloc::cfree_c_abi as unsafe extern "C" fn(*mut c_void);

      Some(symbol as *const () as *mut c_void)
    }
    b"open" => {
      let symbol =
        crate::unistd::open as unsafe extern "C" fn(*const c_char, c_int, c_uint) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"openat" => {
      let symbol =
        crate::unistd::openat as unsafe extern "C" fn(c_int, *const c_char, c_int, c_uint) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"access" => {
      let symbol = crate::unistd::access as unsafe extern "C" fn(*const c_char, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"unlink" => {
      let symbol = crate::unistd::unlink as unsafe extern "C" fn(*const c_char) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"close" => {
      let symbol = crate::unistd::close as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"dup" => {
      let symbol = crate::unistd::dup as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"dup2" => {
      let symbol = crate::unistd::dup2 as extern "C" fn(c_int, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"dup3" => {
      let symbol = crate::unistd::dup3 as extern "C" fn(c_int, c_int, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"lseek" => {
      let symbol = crate::unistd::lseek as extern "C" fn(c_int, c_long, c_int) -> c_long;

      Some(symbol as *const () as *mut c_void)
    }
    b"getpid" => {
      let symbol = crate::unistd::getpid as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getppid" => {
      let symbol = crate::unistd::getppid as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getpgid" => {
      let symbol = crate::unistd::getpgid as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getpgrp" => {
      let symbol = crate::unistd::getpgrp as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getsid" => {
      let symbol = crate::unistd::getsid as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"gettid" => {
      let symbol = crate::unistd::gettid as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getuid" => {
      let symbol = crate::unistd::getuid as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"geteuid" => {
      let symbol = crate::unistd::geteuid as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getgid" => {
      let symbol = crate::unistd::getgid as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getegid" => {
      let symbol = crate::unistd::getegid as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isatty" => {
      let symbol = crate::unistd::isatty as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"read" => {
      let symbol =
        crate::unistd::read as unsafe extern "C" fn(c_int, *mut c_void, size_t) -> ssize_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"write" => {
      let symbol =
        crate::unistd::write as unsafe extern "C" fn(c_int, *const c_void, size_t) -> ssize_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"send" => {
      let symbol =
        crate::unistd::send as unsafe extern "C" fn(c_int, *const c_void, size_t, c_int) -> ssize_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"recv" => {
      let symbol =
        crate::unistd::recv as unsafe extern "C" fn(c_int, *mut c_void, size_t, c_int) -> ssize_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"pipe" => {
      let symbol = crate::unistd::pipe as unsafe extern "C" fn(*mut c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pipe2" => {
      let symbol = crate::unistd::pipe2 as unsafe extern "C" fn(*mut c_int, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fsync" => {
      let symbol = crate::unistd::fsync as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fdatasync" => {
      let symbol = crate::unistd::fdatasync as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sync" => {
      let symbol = crate::unistd::sync as extern "C" fn();

      Some(symbol as *const () as *mut c_void)
    }
    b"syncfs" => {
      let symbol = crate::unistd::syncfs as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fstat" => {
      let symbol = crate::fs::fstat as unsafe extern "C" fn(c_int, *mut crate::fs::Stat) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fstatat" => {
      let symbol = crate::fs::fstatat
        as unsafe extern "C" fn(c_int, *const c_char, *mut crate::fs::Stat, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"stat" => {
      let symbol =
        crate::fs::stat as unsafe extern "C" fn(*const c_char, *mut crate::fs::Stat) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"lstat" => {
      let symbol =
        crate::fs::lstat as unsafe extern "C" fn(*const c_char, *mut crate::fs::Stat) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_system_resource_time(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"uname" => {
      let symbol =
        crate::system::uname as unsafe extern "C" fn(*mut crate::system::UtsName) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"gethostname" => {
      let symbol = crate::system::gethostname as unsafe extern "C" fn(*mut c_char, size_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getpagesize" => {
      let symbol = crate::system::getpagesize as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sysinfo" => {
      let symbol =
        crate::system::sysinfo as unsafe extern "C" fn(*mut crate::system::SysInfo) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sysconf" => {
      let symbol = crate::system::sysconf as extern "C" fn(c_int) -> c_long;

      Some(symbol as *const () as *mut c_void)
    }
    b"prlimit64" => {
      let symbol = crate::resource::prlimit64
        as unsafe extern "C" fn(
          c_int,
          c_int,
          *const crate::resource::RLimit,
          *mut crate::resource::RLimit,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getrlimit" => {
      let symbol = crate::resource::getrlimit
        as unsafe extern "C" fn(c_int, *mut crate::resource::RLimit) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"setrlimit" => {
      let symbol = crate::resource::setrlimit
        as unsafe extern "C" fn(c_int, *const crate::resource::RLimit) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"clock_gettime" => {
      let symbol = crate::time::clock_gettime
        as extern "C" fn(crate::time::clockid_t, *mut crate::time::timespec) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"gettimeofday" => {
      let symbol = crate::time::gettimeofday
        as unsafe extern "C" fn(*mut crate::time::timeval, *mut crate::time::timezone) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"strftime" => {
      let symbol = crate::time::strftime
        as unsafe extern "C" fn(
          *mut c_char,
          size_t,
          *const c_char,
          *const crate::time::tm,
        ) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"gmtime" => {
      let symbol = crate::time::gmtime
        as unsafe extern "C" fn(*const crate::time::time_t) -> *mut crate::time::tm;

      Some(symbol as *const () as *mut c_void)
    }
    b"gmtime_r" => {
      let symbol = crate::time::gmtime_r
        as unsafe extern "C" fn(
          *const crate::time::time_t,
          *mut crate::time::tm,
        ) -> *mut crate::time::tm;

      Some(symbol as *const () as *mut c_void)
    }
    b"localtime" => {
      let symbol = crate::time::localtime
        as unsafe extern "C" fn(*const crate::time::time_t) -> *mut crate::time::tm;

      Some(symbol as *const () as *mut c_void)
    }
    b"localtime_r" => {
      let symbol = crate::time::localtime_r
        as unsafe extern "C" fn(
          *const crate::time::time_t,
          *mut crate::time::tm,
        ) -> *mut crate::time::tm;

      Some(symbol as *const () as *mut c_void)
    }
    b"timegm" => {
      let symbol =
        crate::time::timegm as unsafe extern "C" fn(*mut crate::time::tm) -> crate::time::time_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"mktime" => {
      let symbol =
        crate::time::mktime as unsafe extern "C" fn(*mut crate::time::tm) -> crate::time::time_t;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_net_socket(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"getaddrinfo" => {
      let symbol = crate::netdb::getaddrinfo
        as unsafe extern "C" fn(
          *const c_char,
          *const c_char,
          *const crate::netdb::addrinfo,
          *mut *mut crate::netdb::addrinfo,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"getnameinfo" => {
      let symbol = crate::netdb::getnameinfo
        as unsafe extern "C" fn(
          *const crate::netdb::sockaddr,
          crate::netdb::socklen_t,
          *mut c_char,
          crate::netdb::socklen_t,
          *mut c_char,
          crate::netdb::socklen_t,
          c_int,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"freeaddrinfo" => {
      let symbol = crate::netdb::freeaddrinfo as unsafe extern "C" fn(*mut crate::netdb::addrinfo);

      Some(symbol as *const () as *mut c_void)
    }
    b"gai_strerror" => {
      let symbol = crate::netdb::gai_strerror as extern "C" fn(c_int) -> *const c_char;

      Some(symbol as *const () as *mut c_void)
    }
    b"socket" => {
      let symbol = crate::socket::socket as unsafe extern "C" fn(c_int, c_int, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"connect" => {
      let symbol = crate::socket::connect
        as unsafe extern "C" fn(
          c_int,
          *const crate::socket::Sockaddr,
          crate::socket::SocklenT,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"bind" => {
      let symbol = crate::socket::bind
        as unsafe extern "C" fn(
          c_int,
          *const crate::socket::Sockaddr,
          crate::socket::SocklenT,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"listen" => {
      let symbol = crate::socket::listen as unsafe extern "C" fn(c_int, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"accept" => {
      let symbol = crate::socket::accept
        as unsafe extern "C" fn(
          c_int,
          *mut crate::socket::Sockaddr,
          *mut crate::socket::SocklenT,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_stdio_pthread_dlfcn(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"__errno_location" => {
      let symbol = crate::errno::__errno_location as extern "C" fn() -> *mut c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fclose" => {
      let symbol = crate::stdio::fclose as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fflush" => {
      let symbol = crate::stdio::fflush as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fileno" => {
      let symbol = crate::stdio::fileno as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fileno_unlocked" => {
      let symbol =
        crate::stdio::fileno_unlocked as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"tmpfile" => {
      let symbol = crate::stdio::tmpfile as unsafe extern "C" fn() -> *mut crate::stdio::FILE;

      Some(symbol as *const () as *mut c_void)
    }
    b"fopen" => {
      let symbol = crate::stdio::fopen
        as unsafe extern "C" fn(*const c_char, *const c_char) -> *mut crate::stdio::FILE;

      Some(symbol as *const () as *mut c_void)
    }
    b"fputs" => {
      let symbol = crate::stdio::fputs
        as unsafe extern "C" fn(*const c_char, *mut crate::stdio::FILE) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fread" => {
      let symbol = crate::stdio::fread
        as unsafe extern "C" fn(*mut c_void, size_t, size_t, *mut crate::stdio::FILE) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"setbuffer" => {
      let symbol = crate::stdio::setbuffer
        as unsafe extern "C" fn(*mut crate::stdio::FILE, *mut c_char, size_t);

      Some(symbol as *const () as *mut c_void)
    }
    b"setbuf" => {
      let symbol =
        crate::stdio::setbuf as unsafe extern "C" fn(*mut crate::stdio::FILE, *mut c_char);

      Some(symbol as *const () as *mut c_void)
    }
    b"setlinebuf" => {
      let symbol = crate::stdio::setlinebuf as unsafe extern "C" fn(*mut crate::stdio::FILE);

      Some(symbol as *const () as *mut c_void)
    }
    b"flockfile" => {
      let symbol = crate::stdio::flockfile as unsafe extern "C" fn(*mut crate::stdio::FILE);

      Some(symbol as *const () as *mut c_void)
    }
    b"ftrylockfile" => {
      let symbol =
        crate::stdio::ftrylockfile as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"funlockfile" => {
      let symbol = crate::stdio::funlockfile as unsafe extern "C" fn(*mut crate::stdio::FILE);

      Some(symbol as *const () as *mut c_void)
    }
    b"setvbuf" => {
      let symbol = crate::stdio::setvbuf
        as unsafe extern "C" fn(*mut crate::stdio::FILE, *mut c_char, c_int, size_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"vsnprintf" => {
      let symbol = crate::stdio::vsnprintf
        as unsafe extern "C" fn(*mut c_char, size_t, *const c_char, *mut c_void) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"vfprintf" => {
      let symbol = crate::stdio::vfprintf
        as unsafe extern "C" fn(*mut crate::stdio::FILE, *const c_char, *mut c_void) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"vprintf" => {
      let symbol =
        crate::stdio::vprintf as unsafe extern "C" fn(*const c_char, *mut c_void) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"printf" => {
      let symbol = crate::stdio::printf as unsafe extern "C" fn(*const c_char, ...) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fprintf" => {
      let symbol = crate::stdio::fprintf
        as unsafe extern "C" fn(*mut crate::stdio::FILE, *const c_char, ...) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_create" => {
      let symbol = crate::pthread::pthread_create
        as unsafe extern "C" fn(
          *mut crate::pthread::pthread_t,
          *const crate::pthread::pthread_attr_t,
          Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
          *mut c_void,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_join" => {
      let symbol = crate::pthread::pthread_join
        as unsafe extern "C" fn(crate::pthread::pthread_t, *mut *mut c_void) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_detach" => {
      let symbol =
        crate::pthread::pthread_detach as extern "C" fn(crate::pthread::pthread_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"dlopen" => {
      let symbol =
        crate::dlfcn::dlopen as unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"dlsym" => {
      let symbol =
        crate::dlfcn::dlsym as unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"dlclose" => {
      let symbol = crate::dlfcn::dlclose as extern "C" fn(*mut c_void) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"dlerror" => {
      let symbol = crate::dlfcn::dlerror as extern "C" fn() -> *mut c_char;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_pthread_sync(symbol_name: &[u8]) -> Option<*mut c_void> {
  resolve_rlibc_symbol_pthread_mutex(symbol_name)
    .or_else(|| resolve_rlibc_symbol_pthread_cond(symbol_name))
    .or_else(|| resolve_rlibc_symbol_pthread_rwlock(symbol_name))
}

fn resolve_rlibc_symbol_pthread_mutex(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"pthread_mutexattr_init" => {
      let symbol = crate::pthread::pthread_mutexattr_init
        as extern "C" fn(*mut crate::pthread::pthread_mutexattr_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutexattr_destroy" => {
      let symbol = crate::pthread::pthread_mutexattr_destroy
        as extern "C" fn(*mut crate::pthread::pthread_mutexattr_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutexattr_gettype" => {
      let symbol = crate::pthread::pthread_mutexattr_gettype
        as extern "C" fn(*const crate::pthread::pthread_mutexattr_t, *mut c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutexattr_settype" => {
      let symbol = crate::pthread::pthread_mutexattr_settype
        as extern "C" fn(*mut crate::pthread::pthread_mutexattr_t, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutexattr_getpshared" => {
      let symbol = crate::pthread::pthread_mutexattr_getpshared
        as extern "C" fn(*const crate::pthread::pthread_mutexattr_t, *mut c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutexattr_setpshared" => {
      let symbol = crate::pthread::pthread_mutexattr_setpshared
        as extern "C" fn(*mut crate::pthread::pthread_mutexattr_t, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutex_init" => {
      let symbol = crate::pthread::pthread_mutex_init
        as extern "C" fn(
          *mut crate::pthread::pthread_mutex_t,
          *const crate::pthread::pthread_mutexattr_t,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutex_destroy" => {
      let symbol = crate::pthread::pthread_mutex_destroy
        as extern "C" fn(*mut crate::pthread::pthread_mutex_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutex_lock" => {
      let symbol = crate::pthread::pthread_mutex_lock
        as extern "C" fn(*mut crate::pthread::pthread_mutex_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutex_trylock" => {
      let symbol = crate::pthread::pthread_mutex_trylock
        as extern "C" fn(*mut crate::pthread::pthread_mutex_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_mutex_unlock" => {
      let symbol = crate::pthread::pthread_mutex_unlock
        as extern "C" fn(*mut crate::pthread::pthread_mutex_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_pthread_cond(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"pthread_condattr_init" => {
      let symbol = crate::pthread::pthread_condattr_init
        as extern "C" fn(*mut crate::pthread::pthread_condattr_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_condattr_destroy" => {
      let symbol = crate::pthread::pthread_condattr_destroy
        as extern "C" fn(*mut crate::pthread::pthread_condattr_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_condattr_getpshared" => {
      let symbol = crate::pthread::pthread_condattr_getpshared
        as extern "C" fn(*const crate::pthread::pthread_condattr_t, *mut c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_condattr_setpshared" => {
      let symbol = crate::pthread::pthread_condattr_setpshared
        as extern "C" fn(*mut crate::pthread::pthread_condattr_t, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_cond_init" => {
      let symbol = crate::pthread::pthread_cond_init
        as extern "C" fn(
          *mut crate::pthread::pthread_cond_t,
          *const crate::pthread::pthread_condattr_t,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_cond_destroy" => {
      let symbol = crate::pthread::pthread_cond_destroy
        as extern "C" fn(*mut crate::pthread::pthread_cond_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_cond_wait" => {
      let symbol = crate::pthread::pthread_cond_wait
        as extern "C" fn(
          *mut crate::pthread::pthread_cond_t,
          *mut crate::pthread::pthread_mutex_t,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_cond_timedwait" => {
      let symbol = crate::pthread::pthread_cond_timedwait
        as extern "C" fn(
          *mut crate::pthread::pthread_cond_t,
          *mut crate::pthread::pthread_mutex_t,
          *const crate::time::timespec,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_cond_signal" => {
      let symbol = crate::pthread::pthread_cond_signal
        as extern "C" fn(*mut crate::pthread::pthread_cond_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_cond_broadcast" => {
      let symbol = crate::pthread::pthread_cond_broadcast
        as extern "C" fn(*mut crate::pthread::pthread_cond_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_pthread_rwlock(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"pthread_rwlock_init" => {
      let symbol = crate::pthread::pthread_rwlock_init
        as unsafe extern "C" fn(
          *mut crate::pthread::pthread_rwlock_t,
          *const crate::pthread::pthread_rwlockattr_t,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_rwlock_destroy" => {
      let symbol = crate::pthread::pthread_rwlock_destroy
        as unsafe extern "C" fn(*mut crate::pthread::pthread_rwlock_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_rwlock_rdlock" => {
      let symbol = crate::pthread::pthread_rwlock_rdlock
        as unsafe extern "C" fn(*mut crate::pthread::pthread_rwlock_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_rwlock_tryrdlock" => {
      let symbol = crate::pthread::pthread_rwlock_tryrdlock
        as unsafe extern "C" fn(*mut crate::pthread::pthread_rwlock_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_rwlock_wrlock" => {
      let symbol = crate::pthread::pthread_rwlock_wrlock
        as unsafe extern "C" fn(*mut crate::pthread::pthread_rwlock_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_rwlock_trywrlock" => {
      let symbol = crate::pthread::pthread_rwlock_trywrlock
        as unsafe extern "C" fn(*mut crate::pthread::pthread_rwlock_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"pthread_rwlock_unlock" => {
      let symbol = crate::pthread::pthread_rwlock_unlock
        as unsafe extern "C" fn(*mut crate::pthread::pthread_rwlock_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_misc(symbol_name: &[u8]) -> Option<*mut c_void> {
  resolve_rlibc_symbol_process_fenv_wchar_startup(symbol_name)
    .or_else(|| resolve_rlibc_symbol_ctype_memory_string_numeric(symbol_name))
    .or_else(|| resolve_rlibc_symbol_signal(symbol_name))
    .or_else(|| resolve_rlibc_symbol_dirent_glob_fcntl(symbol_name))
    .or_else(|| resolve_rlibc_symbol_locale_math_setjmp(symbol_name))
}

fn resolve_rlibc_symbol_process_fenv_wchar_startup(symbol_name: &[u8]) -> Option<*mut c_void> {
  resolve_rlibc_symbol_process_and_startup(symbol_name)
    .or_else(|| resolve_rlibc_symbol_fenv(symbol_name))
    .or_else(|| resolve_rlibc_symbol_wchar(symbol_name))
}

fn resolve_rlibc_symbol_process_and_startup(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"atexit" => {
      let symbol = crate::stdlib::atexit as extern "C" fn(Option<extern "C" fn()>) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"exit" => {
      let symbol = crate::stdlib::exit as extern "C" fn(c_int) -> !;

      Some(symbol as *const () as *mut c_void)
    }
    b"_Exit" => {
      let symbol = crate::stdlib::_Exit as extern "C" fn(c_int) -> !;

      Some(symbol as *const () as *mut c_void)
    }
    b"abort" => {
      let symbol = crate::stdlib::abort as extern "C" fn() -> !;

      Some(symbol as *const () as *mut c_void)
    }
    b"__libc_start_main" => {
      let symbol = crate::startup::__libc_start_main
        as unsafe extern "C" fn(
          Option<crate::startup::StartMainFn>,
          c_int,
          *mut *mut c_char,
          *mut *mut c_char,
        ) -> !;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_fenv(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"feclearexcept" => {
      let symbol = crate::fenv::feclearexcept as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fegetexceptflag" => {
      let symbol = crate::fenv::fegetexceptflag
        as unsafe extern "C" fn(*mut crate::fenv::fexcept_t, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"feraiseexcept" => {
      let symbol = crate::fenv::feraiseexcept as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fesetexceptflag" => {
      let symbol = crate::fenv::fesetexceptflag
        as unsafe extern "C" fn(*const crate::fenv::fexcept_t, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fetestexcept" => {
      let symbol = crate::fenv::fetestexcept as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fegetround" => {
      let symbol = crate::fenv::fegetround as extern "C" fn() -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fesetround" => {
      let symbol = crate::fenv::fesetround as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fegetenv" => {
      let symbol = crate::fenv::fegetenv as unsafe extern "C" fn(*mut crate::fenv::fenv_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"feholdexcept" => {
      let symbol =
        crate::fenv::feholdexcept as unsafe extern "C" fn(*mut crate::fenv::fenv_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"fesetenv" => {
      let symbol =
        crate::fenv::fesetenv as unsafe extern "C" fn(*const crate::fenv::fenv_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"feupdateenv" => {
      let symbol =
        crate::fenv::feupdateenv as unsafe extern "C" fn(*const crate::fenv::fenv_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_wchar(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"mbrtowc" => {
      let symbol = crate::wchar::mbrtowc
        as unsafe extern "C" fn(
          *mut crate::wchar::wchar_t,
          *const c_char,
          size_t,
          *mut crate::wchar::mbstate_t,
        ) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"mbrlen" => {
      let symbol = crate::wchar::mbrlen
        as unsafe extern "C" fn(*const c_char, size_t, *mut crate::wchar::mbstate_t) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"mbsinit" => {
      let symbol =
        crate::wchar::mbsinit as unsafe extern "C" fn(*const crate::wchar::mbstate_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"wcrtomb" => {
      let symbol = crate::wchar::wcrtomb
        as unsafe extern "C" fn(
          *mut c_char,
          crate::wchar::wchar_t,
          *mut crate::wchar::mbstate_t,
        ) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"mbsrtowcs" => {
      let symbol = crate::wchar::mbsrtowcs
        as unsafe extern "C" fn(
          *mut crate::wchar::wchar_t,
          *mut *const c_char,
          size_t,
          *mut crate::wchar::mbstate_t,
        ) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"wcsrtombs" => {
      let symbol = crate::wchar::wcsrtombs
        as unsafe extern "C" fn(
          *mut c_char,
          *mut *const crate::wchar::wchar_t,
          size_t,
          *mut crate::wchar::mbstate_t,
        ) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"mblen" => {
      let symbol = crate::wchar::mblen as unsafe extern "C" fn(*const c_char, size_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"mbtowc" => {
      let symbol = crate::wchar::mbtowc
        as unsafe extern "C" fn(*mut crate::wchar::wchar_t, *const c_char, size_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"wctomb" => {
      let symbol =
        crate::wchar::wctomb as unsafe extern "C" fn(*mut c_char, crate::wchar::wchar_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"mbstowcs" => {
      let symbol = crate::wchar::mbstowcs
        as unsafe extern "C" fn(*mut crate::wchar::wchar_t, *const c_char, size_t) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    b"wcstombs" => {
      let symbol = crate::wchar::wcstombs
        as unsafe extern "C" fn(*mut c_char, *const crate::wchar::wchar_t, size_t) -> size_t;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_ctype_memory_string_numeric(symbol_name: &[u8]) -> Option<*mut c_void> {
  resolve_rlibc_symbol_ctype(symbol_name)
    .or_else(|| resolve_rlibc_symbol_memory_string(symbol_name))
    .or_else(|| resolve_rlibc_symbol_numeric_conversion(symbol_name))
}

fn resolve_rlibc_symbol_ctype(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"isalnum" => {
      let symbol = crate::ctype::isalnum as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isalpha" => {
      let symbol = crate::ctype::isalpha as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isblank" => {
      let symbol = crate::ctype::isblank as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"iscntrl" => {
      let symbol = crate::ctype::iscntrl as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isdigit" => {
      let symbol = crate::ctype::isdigit as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isgraph" => {
      let symbol = crate::ctype::isgraph as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"islower" => {
      let symbol = crate::ctype::islower as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isprint" => {
      let symbol = crate::ctype::isprint as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"ispunct" => {
      let symbol = crate::ctype::ispunct as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isspace" => {
      let symbol = crate::ctype::isspace as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isupper" => {
      let symbol = crate::ctype::isupper as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"isxdigit" => {
      let symbol = crate::ctype::isxdigit as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"tolower" => {
      let symbol = crate::ctype::tolower as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"toupper" => {
      let symbol = crate::ctype::toupper as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_memory_string(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"memcmp" => {
      let symbol = crate::memory::memcmp
        as unsafe extern "C" fn(*const c_void, *const c_void, size_t) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"memcpy" => {
      let symbol = crate::memory::memcpy
        as unsafe extern "C" fn(*mut c_void, *const c_void, size_t) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"memmove" => {
      let symbol = crate::memory::memmove
        as unsafe extern "C" fn(*mut c_void, *const c_void, size_t) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"memset" => {
      let symbol =
        crate::memory::memset as unsafe extern "C" fn(*mut c_void, c_int, size_t) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    b"strlen" => {
      let symbol = crate::string::strlen as unsafe extern "C" fn(*const c_char) -> usize;

      Some(symbol as *const () as *mut c_void)
    }
    b"strnlen" => {
      let symbol = crate::string::strnlen as unsafe extern "C" fn(*const c_char, usize) -> usize;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_numeric_conversion(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"atoi" => {
      let symbol = crate::stdlib::atoi::atoi as unsafe extern "C" fn(*const c_char) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"atol" => {
      let symbol = crate::stdlib::atoi::atol as unsafe extern "C" fn(*const c_char) -> c_long;

      Some(symbol as *const () as *mut c_void)
    }
    b"atoll" => {
      let symbol = crate::stdlib::atoi::atoll as unsafe extern "C" fn(*const c_char) -> c_longlong;

      Some(symbol as *const () as *mut c_void)
    }
    b"strtol" => {
      let symbol = crate::stdlib::conv::strtol
        as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_long;

      Some(symbol as *const () as *mut c_void)
    }
    b"strtoll" => {
      let symbol = crate::stdlib::conv::strtoll
        as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_longlong;

      Some(symbol as *const () as *mut c_void)
    }
    b"strtoul" => {
      let symbol = crate::stdlib::conv::strtoul
        as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_ulong;

      Some(symbol as *const () as *mut c_void)
    }
    b"strtoull" => {
      let symbol = crate::stdlib::conv::strtoull
        as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_ulonglong;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_signal(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"sigemptyset" => {
      let symbol =
        crate::signal::sigemptyset as unsafe extern "C" fn(*mut crate::signal::SigSet) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sigfillset" => {
      let symbol =
        crate::signal::sigfillset as unsafe extern "C" fn(*mut crate::signal::SigSet) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sigaddset" => {
      let symbol = crate::signal::sigaddset
        as unsafe extern "C" fn(*mut crate::signal::SigSet, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sigdelset" => {
      let symbol = crate::signal::sigdelset
        as unsafe extern "C" fn(*mut crate::signal::SigSet, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sigismember" => {
      let symbol = crate::signal::sigismember
        as unsafe extern "C" fn(*const crate::signal::SigSet, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sigaction" => {
      let symbol = crate::signal::sigaction
        as unsafe extern "C" fn(
          c_int,
          *const crate::signal::SigAction,
          *mut crate::signal::SigAction,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"kill" => {
      let symbol = crate::signal::kill as extern "C" fn(c_int, c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"raise" => {
      let symbol = crate::signal::raise as extern "C" fn(c_int) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"sigprocmask" => {
      let symbol = crate::signal::sigprocmask
        as unsafe extern "C" fn(
          c_int,
          *const crate::signal::SigSet,
          *mut crate::signal::SigSet,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_dirent_glob_fcntl(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"fcntl" => {
      let symbol = crate::fcntl::fcntl as unsafe extern "C" fn(c_int, c_int, c_long) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"opendir" => {
      let symbol =
        crate::dirent::opendir as unsafe extern "C" fn(*const c_char) -> *mut crate::dirent::Dir;

      Some(symbol as *const () as *mut c_void)
    }
    b"readdir" => {
      let symbol = crate::dirent::readdir
        as unsafe extern "C" fn(*mut crate::dirent::Dir) -> *mut crate::dirent::Dirent;

      Some(symbol as *const () as *mut c_void)
    }
    b"closedir" => {
      let symbol =
        crate::dirent::closedir as unsafe extern "C" fn(*mut crate::dirent::Dir) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"rewinddir" => {
      let symbol = crate::dirent::rewinddir as unsafe extern "C" fn(*mut crate::dirent::Dir);

      Some(symbol as *const () as *mut c_void)
    }
    b"glob" => {
      let symbol = crate::glob::glob
        as unsafe extern "C" fn(
          *const c_char,
          c_int,
          crate::glob::GlobErrorFn,
          *mut crate::glob::Glob,
        ) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"globfree" => {
      let symbol = crate::glob::globfree as unsafe extern "C" fn(*mut crate::glob::Glob);

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_rlibc_symbol_locale_math_setjmp(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    b"setlocale" => {
      let symbol =
        crate::locale::setlocale as unsafe extern "C" fn(c_int, *const c_char) -> *mut c_char;

      Some(symbol as *const () as *mut c_void)
    }
    b"sqrt" => {
      let symbol = crate::math::sqrt as extern "C" fn(f64) -> f64;

      Some(symbol as *const () as *mut c_void)
    }
    b"log" => {
      let symbol = crate::math::log as extern "C" fn(f64) -> f64;

      Some(symbol as *const () as *mut c_void)
    }
    b"exp" => {
      let symbol = crate::math::exp as extern "C" fn(f64) -> f64;

      Some(symbol as *const () as *mut c_void)
    }
    b"setjmp" => {
      let symbol =
        crate::setjmp::setjmp as unsafe extern "C" fn(*mut crate::setjmp::jmp_buf) -> c_int;

      Some(symbol as *const () as *mut c_void)
    }
    b"longjmp" => {
      let symbol =
        crate::setjmp::longjmp as unsafe extern "C" fn(*const crate::setjmp::jmp_buf, c_int) -> !;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
}

fn resolve_host_symbol(symbol_name: &[u8]) -> Option<*mut c_void> {
  for handle in [RTLD_NEXT, ptr::null_mut()] {
    if let Some(resolved) = resolve_host_symbol_from_handle(symbol_name, handle) {
      return Some(resolved);
    }
  }

  None
}

fn resolve_host_symbol_from_handle(symbol_name: &[u8], handle: *mut c_void) -> Option<*mut c_void> {
  let rlibc_symbol = resolve_rlibc_dlfcn_symbol(symbol_name);

  if let Some(versioned) = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: `symbol_name` and each version are valid NUL-terminated strings.
    let resolved =
      unsafe { host_dlvsym(handle, symbol_name.as_ptr().cast(), version.as_ptr().cast()) };

    if resolved.is_null() || rlibc_symbol == Some(resolved) {
      return None;
    }

    Some(resolved)
  }) {
    return Some(versioned);
  }

  // SAFETY: `symbol_name` is a valid NUL-terminated symbol string.
  let resolved = unsafe { host_dlsym_unversioned(handle, symbol_name.as_ptr().cast()) };

  if resolved.is_null() || rlibc_symbol == Some(resolved) {
    return None;
  }

  Some(resolved)
}

fn resolve_rlibc_dlfcn_symbol(symbol_name: &[u8]) -> Option<*mut c_void> {
  match symbol_name {
    DLOPEN_SYMBOL_NAME => {
      let symbol =
        crate::dlfcn::dlopen as unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    DLERROR_SYMBOL_NAME => {
      let symbol = crate::dlfcn::dlerror as extern "C" fn() -> *mut c_char;

      Some(symbol as *const () as *mut c_void)
    }
    DLSYM_SYMBOL_NAME => {
      let symbol =
        crate::dlfcn::dlsym as unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

      Some(symbol as *const () as *mut c_void)
    }
    _ => None,
  }
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

fn perform_host_dlsym_lookup(
  host_handle: *mut c_void,
  symbol: *const c_char,
) -> Option<(*mut c_void, Option<String>)> {
  let host_dlsym_fn = host_dlsym()?;

  clear_host_dlerror_state();

  // SAFETY: callers validate `symbol` and provide a host-compatible lookup handle.
  let resolved = unsafe { host_dlsym_fn(host_handle, symbol) };
  let host_detail = if resolved.is_null() {
    take_host_dlerror_message()
  } else {
    None
  };

  Some((resolved, host_detail))
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
    io::ErrorKind::DirectoryNotEmpty => Some(ENOTEMPTY),
    io::ErrorKind::ResourceBusy => Some(EBUSY),
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
fn set_forced_internal_dlopen_handle_for_tests(handle: *mut c_void) {
  FORCED_INTERNAL_DLOPEN_HANDLE_FOR_TESTS.store(handle as usize, Ordering::Relaxed);
}

#[cfg(test)]
fn forced_internal_dlopen_handle_for_tests() -> Option<*mut c_void> {
  let forced_handle = FORCED_INTERNAL_DLOPEN_HANDLE_FOR_TESTS.load(Ordering::Relaxed);

  (forced_handle != 0).then_some(forced_handle as *mut c_void)
}

fn is_proc_self_exe_path(path: &Path) -> bool {
  path == Path::new(PROC_SELF_EXE_PATH)
}

fn canonical_path(path: &Path) -> Option<std::path::PathBuf> {
  path.canonicalize().ok()
}

fn is_internal_loader_executable_path(path: &Path) -> bool {
  if is_proc_self_exe_path(path) {
    return true;
  }

  let Some(path_canonical) = canonical_path(path) else {
    return false;
  };
  let Some(proc_self_exe_canonical) = canonical_path(Path::new(PROC_SELF_EXE_PATH)) else {
    return false;
  };

  path_canonical == proc_self_exe_canonical
}

const fn internal_proc_self_exe_handle() -> *mut c_void {
  INTERNAL_PROC_SELF_EXE_HANDLE_ID as *mut c_void
}

const fn internal_main_program_handle() -> *mut c_void {
  INTERNAL_MAIN_PROGRAM_HANDLE_ID as *mut c_void
}

fn is_internal_main_program_handle(handle: *mut c_void) -> bool {
  handle as usize == INTERNAL_MAIN_PROGRAM_HANDLE_ID
}

fn try_open_internal_elf(path: &Path, _flags: c_int) -> Option<*mut c_void> {
  if is_internal_loader_executable_path(path) {
    return Some(internal_proc_self_exe_handle());
  }

  None
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
/// - `EINVAL` for unsupported/invalid flags;
/// - `ENOEXEC` for non-ELF input files;
/// - host OS error codes (for example `ENOENT`) when the target cannot be
///   opened by path (`dlerror` includes the failing path detail);
/// - `filename == NULL` resolves a main-program handle distinct from the
///   internal `/proc/self/exe` fast path;
/// - when host `dlopen(NULL, ...)` is unavailable, this crate synthesizes a
///   main-program handle instead of collapsing onto the `/proc/self/exe`
///   internal handle;
/// - `/proc/self/exe` continues to use an internal loader handle and does not
///   require host `dlopen` delegation;
/// - host runtime loader failures report `dlerror` with
///   `rlibc: host dlopen call failed` plus host detail text when available.
///
/// On success, this function preserves the calling thread's previous `errno`.
///
/// # Safety
/// - `filename` may be null to request the main-program handle.
/// - when `filename` is non-null, it must point to a valid NUL-terminated C
///   string.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void {
  if let Err(errno_value) = validate_dlopen_flags(flags) {
    set_errno(errno_value);
    set_dlerror_message(DLERROR_DLOPEN_INVALID_FLAGS);

    return ptr::null_mut();
  }

  let saved_errno = current_errno();

  if filename.is_null() {
    if let Some(host_dlopen) = host_dlopen() {
      clear_host_dlerror_state();
      // SAFETY: null filename requests the main-program handle from the host loader.
      let handle = unsafe { host_dlopen(ptr::null(), flags) };

      if !handle.is_null() {
        {
          let mut registry = handle_registry_guard();

          registry.register_open_main_program_handle(handle);
        }

        set_errno(saved_errno);

        return handle;
      }

      set_errno(EINVAL);
      set_dlerror_message_with_detail(
        DLERROR_HOST_DLOPEN_FAILED,
        take_host_dlerror_message().as_deref(),
      );

      return ptr::null_mut();
    }

    let internal_handle = {
      #[cfg(test)]
      {
        forced_internal_dlopen_handle_for_tests()
          .or_else(|| try_open_internal_elf(Path::new(PROC_SELF_EXE_PATH), flags))
          .map(|_| internal_main_program_handle())
      }

      #[cfg(not(test))]
      {
        try_open_internal_elf(Path::new(PROC_SELF_EXE_PATH), flags)
          .map(|_| internal_main_program_handle())
      }
    };

    if let Some(handle) = internal_handle {
      {
        let mut registry = handle_registry_guard();

        registry.register_open_main_program_handle(handle);
      }

      set_errno(saved_errno);

      return handle;
    }

    set_errno(EINVAL);
    set_dlerror_message(DLERROR_DLOPEN_MAIN_PROGRAM_UNAVAILABLE);

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

  let internal_handle = {
    #[cfg(test)]
    {
      forced_internal_dlopen_handle_for_tests().or_else(|| try_open_internal_elf(path, flags))
    }

    #[cfg(not(test))]
    {
      try_open_internal_elf(path, flags)
    }
  };

  if let Some(handle) = internal_handle
    && !handle.is_null()
  {
    {
      let mut registry = handle_registry_guard();

      registry.register_open_internal_handle(handle);
    }

    set_errno(saved_errno);

    return handle;
  }

  let Some(host_dlopen) = host_dlopen() else {
    set_errno(EINVAL);
    set_dlerror_message(DLERROR_HOST_DLOPEN_UNAVAILABLE);

    return ptr::null_mut();
  };

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
/// This implementation resolves known `rlibc` symbols directly and otherwise
/// delegates lookup to the host runtime loader resolver for non-internal
/// handles. Lookups for internal `dlopen` handles stay within `rlibc` symbol
/// resolution, except that the synthetic main-program handle may fall back to
/// host `RTLD_DEFAULT` lookup when an `rlibc` symbol is not found. Host
/// main-program handles returned by `dlopen(NULL, ...)` resolve `rlibc`
/// symbols with `RTLD_DEFAULT`-like priority before delegating unresolved
/// names through the original host handle.
///
/// Lookup behavior:
/// - resolves selected `rlibc`-implemented symbols directly (for example
///   `getenv`) before consulting host runtime resolver hooks for
///   `RTLD_DEFAULT`;
/// - for `RTLD_NEXT`, consults the host "next" definition first and falls back
///   to selected `rlibc`-implemented symbols only when the host resolver is
///   unavailable or reports the symbol missing;
/// - otherwise delegates lookup to host runtime loader resolver;
/// - when `RTLD_NEXT` host resolution is unavailable or returns null, retries
///   selected `rlibc`-implemented symbols as a compatibility fallback.
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
///   before the host-detail colon (for example `<symbol> : detail` or
///   `<symbol>\t: detail`).
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

  if !handle.is_null() && handle != RTLD_NEXT {
    let registry_guard = handle_registry_guard();

    // SAFETY: wrapper forwards the original C ABI arguments unchanged.
    return unsafe {
      dlsym_with_registry_guard(handle, symbol, saved_errno, Some(&registry_guard))
    };
  }

  // SAFETY: wrapper forwards the original C ABI arguments unchanged.
  unsafe { dlsym_with_registry_guard(handle, symbol, saved_errno, None) }
}

unsafe fn dlsym_with_registry_guard(
  handle: *mut c_void,
  symbol: *const c_char,
  saved_errno: c_int,
  registry_guard: Option<&MutexGuard<'static, DlHandleRegistry>>,
) -> *mut c_void {
  let fail_with_message = |message: &'static str| -> *mut c_void {
    set_dlerror_message(message);
    set_errno(saved_errno);
    ptr::null_mut()
  };
  let handle_kind = match classify_dlsym_handle(handle, registry_guard.map(std::ops::Deref::deref))
  {
    Ok(classification) => classification,
    Err(message) => {
      return fail_with_message(message);
    }
  };

  if symbol.is_null() {
    return fail_with_message(DLERROR_NULL_SYMBOL);
  }

  // SAFETY: `symbol` was validated non-null and caller provides a valid C string.
  let symbol_name = unsafe { CStr::from_ptr(symbol) }.to_bytes();

  if handle_kind == DlsymHandleKind::Internal {
    if let Some(rlibc_symbol) = resolve_rlibc_symbol(symbol_name) {
      set_errno(saved_errno);

      return rlibc_symbol;
    }

    if handle == internal_proc_self_exe_handle()
      && let Some((resolved, _host_detail)) = perform_host_dlsym_lookup(ptr::null_mut(), symbol)
      && !resolved.is_null()
    {
      set_errno(saved_errno);

      return resolved;
    }

    set_dlsym_missing_symbol_message(symbol, None);
    set_errno(saved_errno);

    return ptr::null_mut();
  }

  if matches!(
    handle_kind,
    DlsymHandleKind::Default | DlsymHandleKind::MainProgram
  ) && let Some(rlibc_symbol) = resolve_rlibc_symbol(symbol_name)
  {
    set_errno(saved_errno);

    return rlibc_symbol;
  }

  let host_lookup_handle = match handle_kind {
    DlsymHandleKind::Default => ptr::null_mut(),
    DlsymHandleKind::Next | DlsymHandleKind::External => handle,
    DlsymHandleKind::MainProgram => {
      if is_internal_main_program_handle(handle) {
        ptr::null_mut()
      } else {
        handle
      }
    }
    DlsymHandleKind::Internal => unreachable!("internal handles return before host lookup"),
  };
  let Some((resolved, host_detail)) = perform_host_dlsym_lookup(host_lookup_handle, symbol) else {
    if handle_kind == DlsymHandleKind::Next
      && let Some(rlibc_symbol) = resolve_rlibc_symbol(symbol_name)
    {
      set_errno(saved_errno);

      return rlibc_symbol;
    }

    return fail_with_message(DLERROR_HOST_DLSYM_UNAVAILABLE);
  };

  set_errno(saved_errno);

  if resolved.is_null() {
    if handle_kind == DlsymHandleKind::Next
      && let Some(rlibc_symbol) = resolve_rlibc_symbol(symbol_name)
    {
      set_errno(saved_errno);

      return rlibc_symbol;
    }

    set_dlsym_missing_symbol_message(symbol, host_detail.as_deref());
    set_errno(saved_errno);
  }

  resolved
}

#[cfg(test)]
mod tests {
  use std::ffi::CStr;
  use std::path::Path;
  use std::{io, thread};

  use super::{
    DLCLOSE_FAILURE, DLCLOSE_SUCCESS, DLERROR_ALREADY_CLOSED, DLERROR_INVALID_HANDLE,
    DlHandleRegistry, DlHandleState, DlsymHandleKind, INTERNAL_MAIN_PROGRAM_HANDLE_ID,
    INTERNAL_PROC_SELF_EXE_HANDLE_ID, PROC_SELF_EXE_PATH, RTLD_BINDING_MASK, RTLD_GLOBAL, RTLD_NOW,
    RTLD_SUPPORTED_MASK, TRACKABLE_NULL_HANDLE_ID, classify_dlsym_handle, clear_dlerror_state,
    clear_host_dlerror_state, dlclose, dlerror, dlopen, dlsym,
    forced_internal_dlopen_handle_for_tests, handle_registry_guard, host_dlopen,
    internal_main_program_handle, io_error_errno, is_registered_internal_dlopen_handle,
    resolve_rlibc_symbol, set_dlerror_message_with_detail, set_dlsym_missing_symbol_message,
    set_forced_internal_dlopen_handle_for_tests, try_open_internal_elf, validate_dlopen_flags,
    validate_dlsym_handle,
  };
  use crate::abi::errno::{EACCES, EAGAIN, EEXIST, EISDIR, ENOENT, ENOEXEC, ENOTDIR};
  use crate::abi::types::{
    c_char, c_int, c_long, c_longlong, c_ulong, c_ulonglong, c_void, size_t,
  };
  use crate::dirent::{Dir, Dirent, closedir, opendir, readdir, rewinddir};
  use crate::fcntl::fcntl;
  use crate::glob::{Glob, GlobErrorFn, glob, globfree};
  use crate::locale::setlocale;
  use crate::math::{exp, log, sqrt};
  use crate::pthread::{
    pthread_cond_broadcast, pthread_cond_destroy, pthread_cond_init, pthread_cond_signal,
    pthread_cond_t, pthread_cond_timedwait, pthread_cond_wait, pthread_condattr_destroy,
    pthread_condattr_getpshared, pthread_condattr_init, pthread_condattr_setpshared,
    pthread_condattr_t, pthread_mutex_destroy, pthread_mutex_init, pthread_mutex_lock,
    pthread_mutex_t, pthread_mutex_trylock, pthread_mutex_unlock, pthread_mutexattr_destroy,
    pthread_mutexattr_getpshared, pthread_mutexattr_gettype, pthread_mutexattr_init,
    pthread_mutexattr_setpshared, pthread_mutexattr_settype, pthread_mutexattr_t,
    pthread_rwlock_destroy, pthread_rwlock_init, pthread_rwlock_rdlock, pthread_rwlock_t,
    pthread_rwlock_tryrdlock, pthread_rwlock_trywrlock, pthread_rwlock_unlock,
    pthread_rwlock_wrlock, pthread_rwlockattr_t,
  };
  use crate::setjmp::{jmp_buf, longjmp, setjmp};
  use crate::signal::{
    SigAction, SigSet, kill, raise, sigaction, sigaddset, sigdelset, sigemptyset, sigfillset,
    sigismember, sigprocmask,
  };
  use crate::time::timespec;

  struct ForcedInternalDlopenHandleGuard;

  impl Drop for ForcedInternalDlopenHandleGuard {
    fn drop(&mut self) {
      set_forced_internal_dlopen_handle_for_tests(core::ptr::null_mut());
    }
  }

  fn force_internal_dlopen_handle_for_tests(
    handle: *mut c_void,
  ) -> ForcedInternalDlopenHandleGuard {
    set_forced_internal_dlopen_handle_for_tests(handle);

    ForcedInternalDlopenHandleGuard
  }

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
  fn try_open_internal_elf_returns_internal_handle_for_proc_self_exe() {
    let _guard = force_internal_dlopen_handle_for_tests(core::ptr::null_mut());
    let outcome = try_open_internal_elf(Path::new(PROC_SELF_EXE_PATH), RTLD_NOW);

    assert_eq!(
      outcome,
      Some(INTERNAL_PROC_SELF_EXE_HANDLE_ID as *mut c_void)
    );
  }

  #[test]
  fn try_open_internal_elf_returns_none_for_other_paths() {
    let _guard = force_internal_dlopen_handle_for_tests(core::ptr::null_mut());
    let outcome = try_open_internal_elf(Path::new("/tmp/not-internal-loader-target.so"), RTLD_NOW);

    assert_eq!(outcome, None);
  }

  #[test]
  fn try_open_internal_elf_returns_internal_handle_for_current_exe_path() {
    let _guard = force_internal_dlopen_handle_for_tests(core::ptr::null_mut());
    let current_exe = std::env::current_exe().expect("current_exe path should resolve");
    let outcome = try_open_internal_elf(current_exe.as_path(), RTLD_NOW);

    assert_eq!(
      outcome,
      Some(INTERNAL_PROC_SELF_EXE_HANDLE_ID as *mut c_void)
    );
  }

  #[test]
  fn try_open_internal_elf_returns_forced_test_handle_override() {
    let forced_handle = core::ptr::dangling_mut::<c_void>();
    let _guard = force_internal_dlopen_handle_for_tests(forced_handle);
    let outcome = forced_internal_dlopen_handle_for_tests()
      .or_else(|| try_open_internal_elf(Path::new("/proc/self/exe"), RTLD_NOW));

    assert_eq!(outcome, Some(forced_handle));
  }

  #[test]
  fn dlopen_proc_self_exe_returns_registered_internal_handle() {
    reset_thread_local_error_state();

    // SAFETY: `PROC_SELF_EXE_PATH` is a valid NUL-terminated C string.
    let handle = unsafe { dlopen(c"/proc/self/exe".as_ptr(), RTLD_NOW) };

    assert_eq!(handle, INTERNAL_PROC_SELF_EXE_HANDLE_ID as *mut c_void);
    assert!(is_registered_internal_dlopen_handle(handle));
    assert_eq!(dlclose(handle), DLCLOSE_SUCCESS);
  }

  #[test]
  fn dlsym_internal_handle_resolves_rlibc_symbols_without_host_lookup() {
    reset_thread_local_error_state();

    // SAFETY: `PROC_SELF_EXE_PATH` is a valid NUL-terminated C string.
    let handle = unsafe { dlopen(c"/proc/self/exe".as_ptr(), RTLD_NOW) };

    assert!(is_registered_internal_dlopen_handle(handle));

    // SAFETY: handle was returned by `dlopen`; symbol string is valid.
    let resolved = unsafe { dlsym(handle, c"strlen".as_ptr()) };
    let expected =
      resolve_rlibc_symbol(b"strlen").unwrap_or_else(|| unreachable!("strlen must resolve"));

    assert_eq!(resolved, expected);
    assert_eq!(dlclose(handle), DLCLOSE_SUCCESS);
  }

  #[test]
  fn dlsym_internal_handle_missing_symbol_sets_deterministic_error_without_host_detail() {
    reset_thread_local_error_state();

    // SAFETY: `PROC_SELF_EXE_PATH` is a valid NUL-terminated C string.
    let handle = unsafe { dlopen(c"/proc/self/exe".as_ptr(), RTLD_NOW) };

    assert!(is_registered_internal_dlopen_handle(handle));

    // SAFETY: handle was returned by `dlopen`; symbol string is valid.
    let resolved = unsafe { dlsym(handle, c"__rlibc_missing_internal_symbol__".as_ptr()) };

    assert!(resolved.is_null());

    let message = take_dlerror_message().expect("expected dlerror for unresolved internal symbol");

    assert_eq!(
      message,
      "rlibc: requested symbol was not found: __rlibc_missing_internal_symbol__",
    );
    assert_eq!(dlclose(handle), DLCLOSE_SUCCESS);
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
  fn set_dlsym_missing_symbol_message_deduplicates_symbol_prefix_with_tab_before_separator() {
    reset_thread_local_error_state();

    set_dlsym_missing_symbol_message(
      c"dup_symbol".as_ptr(),
      Some("dup_symbol\t: host loader unresolved entry"),
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
  fn resolve_rlibc_symbol_maps_pthread_sync_symbols() {
    let checks = [
      (
        b"pthread_mutexattr_init".as_slice(),
        (pthread_mutexattr_init as extern "C" fn(*mut pthread_mutexattr_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_mutexattr_destroy".as_slice(),
        (pthread_mutexattr_destroy as extern "C" fn(*mut pthread_mutexattr_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_mutexattr_gettype".as_slice(),
        (pthread_mutexattr_gettype
          as extern "C" fn(*const pthread_mutexattr_t, *mut c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_mutexattr_settype".as_slice(),
        (pthread_mutexattr_settype as extern "C" fn(*mut pthread_mutexattr_t, c_int) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_mutexattr_getpshared".as_slice(),
        (pthread_mutexattr_getpshared
          as extern "C" fn(*const pthread_mutexattr_t, *mut c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_mutexattr_setpshared".as_slice(),
        (pthread_mutexattr_setpshared as extern "C" fn(*mut pthread_mutexattr_t, c_int) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_mutex_init".as_slice(),
        (pthread_mutex_init
          as extern "C" fn(*mut pthread_mutex_t, *const pthread_mutexattr_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_mutex_destroy".as_slice(),
        (pthread_mutex_destroy as extern "C" fn(*mut pthread_mutex_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_mutex_lock".as_slice(),
        (pthread_mutex_lock as extern "C" fn(*mut pthread_mutex_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_mutex_trylock".as_slice(),
        (pthread_mutex_trylock as extern "C" fn(*mut pthread_mutex_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_mutex_unlock".as_slice(),
        (pthread_mutex_unlock as extern "C" fn(*mut pthread_mutex_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_condattr_init".as_slice(),
        (pthread_condattr_init as extern "C" fn(*mut pthread_condattr_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_condattr_destroy".as_slice(),
        (pthread_condattr_destroy as extern "C" fn(*mut pthread_condattr_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_condattr_getpshared".as_slice(),
        (pthread_condattr_getpshared
          as extern "C" fn(*const pthread_condattr_t, *mut c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_condattr_setpshared".as_slice(),
        (pthread_condattr_setpshared as extern "C" fn(*mut pthread_condattr_t, c_int) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_cond_init".as_slice(),
        (pthread_cond_init
          as extern "C" fn(*mut pthread_cond_t, *const pthread_condattr_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_cond_destroy".as_slice(),
        (pthread_cond_destroy as extern "C" fn(*mut pthread_cond_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_cond_wait".as_slice(),
        (pthread_cond_wait as extern "C" fn(*mut pthread_cond_t, *mut pthread_mutex_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_cond_timedwait".as_slice(),
        (pthread_cond_timedwait
          as extern "C" fn(*mut pthread_cond_t, *mut pthread_mutex_t, *const timespec) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_cond_signal".as_slice(),
        (pthread_cond_signal as extern "C" fn(*mut pthread_cond_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_cond_broadcast".as_slice(),
        (pthread_cond_broadcast as extern "C" fn(*mut pthread_cond_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_rwlock_init".as_slice(),
        (pthread_rwlock_init
          as unsafe extern "C" fn(*mut pthread_rwlock_t, *const pthread_rwlockattr_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_rwlock_destroy".as_slice(),
        (pthread_rwlock_destroy as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_rwlock_rdlock".as_slice(),
        (pthread_rwlock_rdlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_rwlock_tryrdlock".as_slice(),
        (pthread_rwlock_tryrdlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_rwlock_wrlock".as_slice(),
        (pthread_rwlock_wrlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pthread_rwlock_trywrlock".as_slice(),
        (pthread_rwlock_trywrlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"pthread_rwlock_unlock".as_slice(),
        (pthread_rwlock_unlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int) as *const ()
          as *mut c_void,
      ),
    ];

    for (symbol_name, expected_ptr) in checks {
      let resolved = resolve_rlibc_symbol(symbol_name);

      assert_eq!(resolved, Some(expected_ptr), "missing symbol mapping");
    }
  }

  #[test]
  fn resolve_rlibc_symbol_maps_signal_and_misc_symbols() {
    let checks = [
      (
        b"sigemptyset".as_slice(),
        (sigemptyset as unsafe extern "C" fn(*mut SigSet) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"sigfillset".as_slice(),
        (sigfillset as unsafe extern "C" fn(*mut SigSet) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"sigaddset".as_slice(),
        (sigaddset as unsafe extern "C" fn(*mut SigSet, c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"sigdelset".as_slice(),
        (sigdelset as unsafe extern "C" fn(*mut SigSet, c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"sigismember".as_slice(),
        (sigismember as unsafe extern "C" fn(*const SigSet, c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"sigaction".as_slice(),
        (sigaction as unsafe extern "C" fn(c_int, *const SigAction, *mut SigAction) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"kill".as_slice(),
        (kill as extern "C" fn(c_int, c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"raise".as_slice(),
        (raise as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"sigprocmask".as_slice(),
        (sigprocmask as unsafe extern "C" fn(c_int, *const SigSet, *mut SigSet) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"fcntl".as_slice(),
        (fcntl as unsafe extern "C" fn(c_int, c_int, c_long) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"opendir".as_slice(),
        (opendir as unsafe extern "C" fn(*const c_char) -> *mut Dir) as *const () as *mut c_void,
      ),
      (
        b"readdir".as_slice(),
        (readdir as unsafe extern "C" fn(*mut Dir) -> *mut Dirent) as *const () as *mut c_void,
      ),
      (
        b"closedir".as_slice(),
        (closedir as unsafe extern "C" fn(*mut Dir) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"rewinddir".as_slice(),
        (rewinddir as unsafe extern "C" fn(*mut Dir)) as *const () as *mut c_void,
      ),
      (
        b"glob".as_slice(),
        (glob as unsafe extern "C" fn(*const c_char, c_int, GlobErrorFn, *mut Glob) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"globfree".as_slice(),
        (globfree as unsafe extern "C" fn(*mut Glob)) as *const () as *mut c_void,
      ),
      (
        b"setlocale".as_slice(),
        (setlocale as unsafe extern "C" fn(c_int, *const c_char) -> *mut c_char) as *const ()
          as *mut c_void,
      ),
      (
        b"sqrt".as_slice(),
        (sqrt as extern "C" fn(f64) -> f64) as *const () as *mut c_void,
      ),
      (
        b"log".as_slice(),
        (log as extern "C" fn(f64) -> f64) as *const () as *mut c_void,
      ),
      (
        b"exp".as_slice(),
        (exp as extern "C" fn(f64) -> f64) as *const () as *mut c_void,
      ),
      (
        b"setjmp".as_slice(),
        (setjmp as unsafe extern "C" fn(*mut jmp_buf) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"longjmp".as_slice(),
        (longjmp as unsafe extern "C" fn(*const jmp_buf, c_int) -> !) as *const () as *mut c_void,
      ),
    ];

    for (symbol_name, expected_ptr) in checks {
      let resolved = resolve_rlibc_symbol(symbol_name);

      assert_eq!(resolved, Some(expected_ptr), "missing symbol mapping");
    }
  }

  #[test]
  fn resolve_rlibc_symbol_maps_ctype_memory_string_numeric_symbols() {
    let checks = [
      (
        b"isalnum".as_slice(),
        (crate::ctype::isalnum as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isalpha".as_slice(),
        (crate::ctype::isalpha as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isblank".as_slice(),
        (crate::ctype::isblank as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"iscntrl".as_slice(),
        (crate::ctype::iscntrl as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isdigit".as_slice(),
        (crate::ctype::isdigit as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isgraph".as_slice(),
        (crate::ctype::isgraph as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"islower".as_slice(),
        (crate::ctype::islower as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isprint".as_slice(),
        (crate::ctype::isprint as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"ispunct".as_slice(),
        (crate::ctype::ispunct as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isspace".as_slice(),
        (crate::ctype::isspace as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isupper".as_slice(),
        (crate::ctype::isupper as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isxdigit".as_slice(),
        (crate::ctype::isxdigit as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"tolower".as_slice(),
        (crate::ctype::tolower as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"toupper".as_slice(),
        (crate::ctype::toupper as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"memcmp".as_slice(),
        (crate::memory::memcmp
          as unsafe extern "C" fn(*const c_void, *const c_void, size_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"memcpy".as_slice(),
        (crate::memory::memcpy
          as unsafe extern "C" fn(*mut c_void, *const c_void, size_t) -> *mut c_void)
          as *const () as *mut c_void,
      ),
      (
        b"memmove".as_slice(),
        (crate::memory::memmove
          as unsafe extern "C" fn(*mut c_void, *const c_void, size_t) -> *mut c_void)
          as *const () as *mut c_void,
      ),
      (
        b"memset".as_slice(),
        (crate::memory::memset as unsafe extern "C" fn(*mut c_void, c_int, size_t) -> *mut c_void)
          as *const () as *mut c_void,
      ),
      (
        b"strlen".as_slice(),
        (crate::string::strlen as unsafe extern "C" fn(*const c_char) -> usize) as *const ()
          as *mut c_void,
      ),
      (
        b"strnlen".as_slice(),
        (crate::string::strnlen as unsafe extern "C" fn(*const c_char, usize) -> usize) as *const ()
          as *mut c_void,
      ),
      (
        b"atoi".as_slice(),
        (crate::stdlib::atoi::atoi as unsafe extern "C" fn(*const c_char) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"atol".as_slice(),
        (crate::stdlib::atoi::atol as unsafe extern "C" fn(*const c_char) -> c_long) as *const ()
          as *mut c_void,
      ),
      (
        b"atoll".as_slice(),
        (crate::stdlib::atoi::atoll as unsafe extern "C" fn(*const c_char) -> c_longlong)
          as *const () as *mut c_void,
      ),
      (
        b"strtol".as_slice(),
        (crate::stdlib::conv::strtol
          as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_long)
          as *const () as *mut c_void,
      ),
      (
        b"strtoll".as_slice(),
        (crate::stdlib::conv::strtoll
          as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_longlong)
          as *const () as *mut c_void,
      ),
      (
        b"strtoul".as_slice(),
        (crate::stdlib::conv::strtoul
          as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_ulong)
          as *const () as *mut c_void,
      ),
      (
        b"strtoull".as_slice(),
        (crate::stdlib::conv::strtoull
          as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_ulonglong)
          as *const () as *mut c_void,
      ),
      (
        b"gai_strerror".as_slice(),
        (crate::netdb::gai_strerror as extern "C" fn(c_int) -> *const c_char) as *const ()
          as *mut c_void,
      ),
    ];

    for (symbol_name, expected_ptr) in checks {
      let resolved = resolve_rlibc_symbol(symbol_name);

      assert_eq!(resolved, Some(expected_ptr), "missing symbol mapping");
    }
  }

  #[test]
  fn resolve_rlibc_symbol_maps_process_fenv_wchar_startup_symbols() {
    let checks = [
      (
        b"atexit".as_slice(),
        (crate::stdlib::atexit as extern "C" fn(Option<extern "C" fn()>) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"exit".as_slice(),
        (crate::stdlib::exit as extern "C" fn(c_int) -> !) as *const () as *mut c_void,
      ),
      (
        b"_Exit".as_slice(),
        (crate::stdlib::_Exit as extern "C" fn(c_int) -> !) as *const () as *mut c_void,
      ),
      (
        b"abort".as_slice(),
        (crate::stdlib::abort as extern "C" fn() -> !) as *const () as *mut c_void,
      ),
      (
        b"__libc_start_main".as_slice(),
        (crate::startup::__libc_start_main
          as unsafe extern "C" fn(
            Option<crate::startup::StartMainFn>,
            c_int,
            *mut *mut c_char,
            *mut *mut c_char,
          ) -> !) as *const () as *mut c_void,
      ),
      (
        b"feclearexcept".as_slice(),
        (crate::fenv::feclearexcept as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"fegetexceptflag".as_slice(),
        (crate::fenv::fegetexceptflag
          as unsafe extern "C" fn(*mut crate::fenv::fexcept_t, c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"feraiseexcept".as_slice(),
        (crate::fenv::feraiseexcept as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"fesetexceptflag".as_slice(),
        (crate::fenv::fesetexceptflag
          as unsafe extern "C" fn(*const crate::fenv::fexcept_t, c_int) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"fetestexcept".as_slice(),
        (crate::fenv::fetestexcept as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"fegetround".as_slice(),
        (crate::fenv::fegetround as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"fesetround".as_slice(),
        (crate::fenv::fesetround as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"fegetenv".as_slice(),
        (crate::fenv::fegetenv as unsafe extern "C" fn(*mut crate::fenv::fenv_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"feholdexcept".as_slice(),
        (crate::fenv::feholdexcept as unsafe extern "C" fn(*mut crate::fenv::fenv_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"fesetenv".as_slice(),
        (crate::fenv::fesetenv as unsafe extern "C" fn(*const crate::fenv::fenv_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"feupdateenv".as_slice(),
        (crate::fenv::feupdateenv as unsafe extern "C" fn(*const crate::fenv::fenv_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"mbrtowc".as_slice(),
        (crate::wchar::mbrtowc
          as unsafe extern "C" fn(
            *mut crate::wchar::wchar_t,
            *const c_char,
            size_t,
            *mut crate::wchar::mbstate_t,
          ) -> size_t) as *const () as *mut c_void,
      ),
      (
        b"mbrlen".as_slice(),
        (crate::wchar::mbrlen
          as unsafe extern "C" fn(*const c_char, size_t, *mut crate::wchar::mbstate_t) -> size_t)
          as *const () as *mut c_void,
      ),
      (
        b"wcrtomb".as_slice(),
        (crate::wchar::wcrtomb
          as unsafe extern "C" fn(
            *mut c_char,
            crate::wchar::wchar_t,
            *mut crate::wchar::mbstate_t,
          ) -> size_t) as *const () as *mut c_void,
      ),
      (
        b"mbsrtowcs".as_slice(),
        (crate::wchar::mbsrtowcs
          as unsafe extern "C" fn(
            *mut crate::wchar::wchar_t,
            *mut *const c_char,
            size_t,
            *mut crate::wchar::mbstate_t,
          ) -> size_t) as *const () as *mut c_void,
      ),
      (
        b"wcsrtombs".as_slice(),
        (crate::wchar::wcsrtombs
          as unsafe extern "C" fn(
            *mut c_char,
            *mut *const crate::wchar::wchar_t,
            size_t,
            *mut crate::wchar::mbstate_t,
          ) -> size_t) as *const () as *mut c_void,
      ),
      (
        b"mblen".as_slice(),
        (crate::wchar::mblen as unsafe extern "C" fn(*const c_char, size_t) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"mbtowc".as_slice(),
        (crate::wchar::mbtowc
          as unsafe extern "C" fn(*mut crate::wchar::wchar_t, *const c_char, size_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"wctomb".as_slice(),
        (crate::wchar::wctomb as unsafe extern "C" fn(*mut c_char, crate::wchar::wchar_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"mbstowcs".as_slice(),
        (crate::wchar::mbstowcs
          as unsafe extern "C" fn(*mut crate::wchar::wchar_t, *const c_char, size_t) -> size_t)
          as *const () as *mut c_void,
      ),
      (
        b"wcstombs".as_slice(),
        (crate::wchar::wcstombs
          as unsafe extern "C" fn(*mut c_char, *const crate::wchar::wchar_t, size_t) -> size_t)
          as *const () as *mut c_void,
      ),
    ];

    for (symbol_name, expected_ptr) in checks {
      let resolved = resolve_rlibc_symbol(symbol_name);

      assert_eq!(resolved, Some(expected_ptr), "missing symbol mapping");
    }
  }

  #[test]
  fn resolve_rlibc_symbol_maps_alloc_compat_and_environ_symbols() {
    let checks = [
      (
        b"aligned_alloc".as_slice(),
        (crate::stdlib::alloc::aligned_alloc_c_abi
          as unsafe extern "C" fn(usize, usize) -> *mut c_void) as *const () as *mut c_void,
      ),
      (
        b"posix_memalign".as_slice(),
        (crate::stdlib::alloc::posix_memalign_c_abi
          as unsafe extern "C" fn(*mut *mut c_void, usize, usize) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"memalign".as_slice(),
        (crate::stdlib::alloc::memalign_c_abi as unsafe extern "C" fn(usize, usize) -> *mut c_void)
          as *const () as *mut c_void,
      ),
      (
        b"valloc".as_slice(),
        (crate::stdlib::alloc::valloc_c_abi as unsafe extern "C" fn(usize) -> *mut c_void)
          as *const () as *mut c_void,
      ),
      (
        b"pvalloc".as_slice(),
        (crate::stdlib::alloc::pvalloc_c_abi as unsafe extern "C" fn(usize) -> *mut c_void)
          as *const () as *mut c_void,
      ),
      (
        b"mbsinit".as_slice(),
        (crate::wchar::mbsinit as unsafe extern "C" fn(*const crate::wchar::mbstate_t) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"environ".as_slice(),
        (&raw mut crate::stdlib::environ).cast::<c_void>(),
      ),
    ];

    for (symbol_name, expected_ptr) in checks {
      let resolved = resolve_rlibc_symbol(symbol_name);

      assert_eq!(resolved, Some(expected_ptr), "missing symbol mapping");
    }
  }

  #[test]
  fn resolve_rlibc_symbol_maps_printf_wrapper_symbols() {
    let checks = [
      (
        b"fclose".as_slice(),
        (crate::stdio::fclose as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"fflush".as_slice(),
        (crate::stdio::fflush as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"fileno".as_slice(),
        (crate::stdio::fileno as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"fileno_unlocked".as_slice(),
        (crate::stdio::fileno_unlocked as unsafe extern "C" fn(*mut crate::stdio::FILE) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"tmpfile".as_slice(),
        (crate::stdio::tmpfile as unsafe extern "C" fn() -> *mut crate::stdio::FILE) as *const ()
          as *mut c_void,
      ),
      (
        b"fopen".as_slice(),
        (crate::stdio::fopen
          as unsafe extern "C" fn(*const c_char, *const c_char) -> *mut crate::stdio::FILE)
          as *const () as *mut c_void,
      ),
      (
        b"fputs".as_slice(),
        (crate::stdio::fputs
          as unsafe extern "C" fn(*const c_char, *mut crate::stdio::FILE) -> c_int)
          as *const () as *mut c_void,
      ),
      (
        b"fread".as_slice(),
        (crate::stdio::fread
          as unsafe extern "C" fn(*mut c_void, size_t, size_t, *mut crate::stdio::FILE) -> size_t)
          as *const () as *mut c_void,
      ),
      (
        b"setbuffer".as_slice(),
        (crate::stdio::setbuffer
          as unsafe extern "C" fn(*mut crate::stdio::FILE, *mut c_char, size_t))
          as *const () as *mut c_void,
      ),
      (
        b"setbuf".as_slice(),
        (crate::stdio::setbuf as unsafe extern "C" fn(*mut crate::stdio::FILE, *mut c_char))
          as *const () as *mut c_void,
      ),
      (
        b"setlinebuf".as_slice(),
        (crate::stdio::setlinebuf as unsafe extern "C" fn(*mut crate::stdio::FILE)) as *const ()
          as *mut c_void,
      ),
      (
        b"printf".as_slice(),
        (crate::stdio::printf as unsafe extern "C" fn(*const c_char, ...) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"fprintf".as_slice(),
        (crate::stdio::fprintf
          as unsafe extern "C" fn(*mut crate::stdio::FILE, *const c_char, ...) -> c_int)
          as *const () as *mut c_void,
      ),
    ];

    for (symbol_name, expected_ptr) in checks {
      let resolved = resolve_rlibc_symbol(symbol_name);

      assert_eq!(resolved, Some(expected_ptr), "missing symbol mapping");
    }
  }

  #[test]
  fn resolve_rlibc_symbol_maps_unistd_close_dup_dup2_dup3_pipe_and_sync_symbols() {
    let checks = [
      (
        b"access".as_slice(),
        (crate::unistd::access as unsafe extern "C" fn(*const c_char, c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"unlink".as_slice(),
        (crate::unistd::unlink as unsafe extern "C" fn(*const c_char) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"close".as_slice(),
        (crate::unistd::close as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"dup".as_slice(),
        (crate::unistd::dup as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"dup2".as_slice(),
        (crate::unistd::dup2 as extern "C" fn(c_int, c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"dup3".as_slice(),
        (crate::unistd::dup3 as extern "C" fn(c_int, c_int, c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"lseek".as_slice(),
        (crate::unistd::lseek as extern "C" fn(c_int, c_long, c_int) -> c_long) as *const ()
          as *mut c_void,
      ),
      (
        b"getpid".as_slice(),
        (crate::unistd::getpid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"getppid".as_slice(),
        (crate::unistd::getppid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"getpgid".as_slice(),
        (crate::unistd::getpgid as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"getpgrp".as_slice(),
        (crate::unistd::getpgrp as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"getsid".as_slice(),
        (crate::unistd::getsid as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"gettid".as_slice(),
        (crate::unistd::gettid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"getuid".as_slice(),
        (crate::unistd::getuid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"geteuid".as_slice(),
        (crate::unistd::geteuid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"getgid".as_slice(),
        (crate::unistd::getgid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"getegid".as_slice(),
        (crate::unistd::getegid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      ),
      (
        b"isatty".as_slice(),
        (crate::unistd::isatty as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"pipe".as_slice(),
        (crate::unistd::pipe as unsafe extern "C" fn(*mut c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"pipe2".as_slice(),
        (crate::unistd::pipe2 as unsafe extern "C" fn(*mut c_int, c_int) -> c_int) as *const ()
          as *mut c_void,
      ),
      (
        b"fsync".as_slice(),
        (crate::unistd::fsync as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"fdatasync".as_slice(),
        (crate::unistd::fdatasync as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
      (
        b"sync".as_slice(),
        (crate::unistd::sync as extern "C" fn()) as *const () as *mut c_void,
      ),
      (
        b"syncfs".as_slice(),
        (crate::unistd::syncfs as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      ),
    ];

    for (symbol_name, expected_ptr) in checks {
      let resolved = resolve_rlibc_symbol(symbol_name);

      assert_eq!(resolved, Some(expected_ptr), "missing symbol mapping");
    }
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
  fn classify_dlsym_handle_marks_registered_main_program_handle() {
    let handle = 0xD0usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry.register_open_main_program_handle(handle);

    assert_eq!(
      classify_dlsym_handle(handle, Some(&registry)),
      Ok(DlsymHandleKind::MainProgram)
    );
  }

  #[test]
  fn internal_main_program_handle_is_distinct_from_proc_self_exe_handle() {
    assert_eq!(
      INTERNAL_MAIN_PROGRAM_HANDLE_ID + 1,
      INTERNAL_PROC_SELF_EXE_HANDLE_ID
    );
    assert_ne!(
      internal_main_program_handle(),
      INTERNAL_PROC_SELF_EXE_HANDLE_ID as *mut c_void,
    );
  }

  #[test]
  fn register_open_handle_reclassifies_main_program_handle_as_external() {
    let handle = 0xD1usize as *mut c_void;
    let mut registry = DlHandleRegistry::new();

    registry.register_open_main_program_handle(handle);
    registry.register_open_handle(handle);

    assert_eq!(
      classify_dlsym_handle(handle, Some(&registry)),
      Ok(DlsymHandleKind::External)
    );
  }

  #[test]
  fn dlsym_rtld_next_prefers_host_fclose_over_rlibc_symbol() {
    reset_thread_local_error_state();

    // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the dlsym ABI contract.
    let resolved = unsafe { dlsym(super::RTLD_NEXT, c"fclose".as_ptr()) };
    let rlibc_symbol =
      resolve_rlibc_symbol(b"fclose").unwrap_or_else(|| unreachable!("fclose must resolve"));

    assert!(
      !resolved.is_null(),
      "RTLD_NEXT fclose lookup should succeed"
    );
    assert_ne!(
      resolved, rlibc_symbol,
      "RTLD_NEXT fclose should prefer the host symbol over rlibc's wrapper",
    );
  }

  #[test]
  fn dlsym_rtld_next_prefers_host_strlen_over_rlibc_symbol() {
    reset_thread_local_error_state();

    // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the dlsym ABI contract.
    let resolved = unsafe { dlsym(super::RTLD_NEXT, c"strlen".as_ptr()) };
    let rlibc_symbol =
      resolve_rlibc_symbol(b"strlen").unwrap_or_else(|| unreachable!("strlen must resolve"));

    assert!(
      !resolved.is_null(),
      "RTLD_NEXT strlen lookup should succeed"
    );
    assert_ne!(
      resolved, rlibc_symbol,
      "RTLD_NEXT strlen should prefer the host symbol over rlibc's wrapper",
    );
  }

  #[test]
  fn dlsym_internal_main_program_handle_uses_rtld_default_host_fallback() {
    reset_thread_local_error_state();

    let handle = internal_main_program_handle();

    {
      let mut registry = handle_registry_guard();

      registry.register_open_main_program_handle(handle);
    }

    // SAFETY: registered synthetic main-program handle and symbol are valid.
    let from_main_handle = unsafe { dlsym(handle, c"puts".as_ptr()) };
    // SAFETY: RTLD_DEFAULT handle and symbol are valid.
    let from_default = unsafe { dlsym(core::ptr::null_mut(), c"puts".as_ptr()) };

    assert!(
      !from_main_handle.is_null(),
      "synthetic main-program handle should resolve host-only symbols through RTLD_DEFAULT",
    );
    assert_eq!(
      from_main_handle, from_default,
      "synthetic main-program handle should mirror RTLD_DEFAULT host fallback",
    );
    assert_eq!(dlclose(handle), DLCLOSE_SUCCESS);
  }

  #[test]
  fn dlsym_host_main_program_handle_resolves_rlibc_symbols_before_host_lookup() {
    reset_thread_local_error_state();

    let host_dlopen = host_dlopen().expect("host dlopen resolver should be available");

    clear_host_dlerror_state();
    // SAFETY: null filename requests the host main-program handle.
    let handle = unsafe { host_dlopen(core::ptr::null(), RTLD_NOW) };

    assert!(!handle.is_null(), "host main-program handle should resolve");

    {
      let mut registry = handle_registry_guard();

      registry.register_open_main_program_handle(handle);
    }

    // SAFETY: handle was registered as an open main-program handle and symbol is valid.
    let resolved = unsafe { dlsym(handle, c"dlopen".as_ptr()) };
    let expected =
      resolve_rlibc_symbol(b"dlopen").unwrap_or_else(|| unreachable!("dlopen must resolve"));

    assert_eq!(
      resolved, expected,
      "host main-program handle should prioritize rlibc-owned symbols like RTLD_DEFAULT",
    );
    assert_eq!(dlclose(handle), DLCLOSE_SUCCESS);
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
  fn register_open_handle_with_max_handle_cleans_trackable_zero_entry() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry.handles.insert(
      TRACKABLE_NULL_HANDLE_ID,
      DlHandleState::Open { refcount: 2 },
    );

    registry.register_open_handle(max_handle);

    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(registry.handle_state(max_handle as usize), None);

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
  fn register_open_handle_with_max_handle_updates_existing_entry_and_cleans_trackable_zero_entry() {
    let mut registry = DlHandleRegistry::new();
    let max_handle = usize::MAX as *mut c_void;

    registry.handles.insert(
      TRACKABLE_NULL_HANDLE_ID,
      DlHandleState::Open { refcount: 8 },
    );
    registry
      .handles
      .insert(max_handle as usize, DlHandleState::Open { refcount: 3 });

    registry.register_open_handle(max_handle);

    assert_eq!(registry.handle_state(TRACKABLE_NULL_HANDLE_ID), None);
    assert_eq!(
      registry.handle_state(max_handle as usize),
      Some(DlHandleState::Open { refcount: 4 })
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
  fn io_error_errno_maps_resource_busy_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::ResourceBusy, "resource busy");

    assert_eq!(io_error_errno(&error, ENOENT), crate::abi::errno::EBUSY);
  }

  #[test]
  fn io_error_errno_maps_directory_not_empty_kind_when_raw_errno_is_absent() {
    let error = io::Error::new(io::ErrorKind::DirectoryNotEmpty, "directory not empty");

    assert_eq!(io_error_errno(&error, ENOENT), crate::abi::errno::ENOTEMPTY);
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
