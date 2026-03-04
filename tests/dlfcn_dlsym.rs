use core::ffi::{c_char, c_void};
use core::ptr;
use rlibc::abi::types::c_int;
use rlibc::dlfcn::{RTLD_NOW, dlclose, dlerror, dlopen, dlsym};
use rlibc::errno::__errno_location;
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::{fs, thread};

const RTLD_DEFAULT: *mut c_void = ptr::null_mut();
const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
const MISSING_SYMBOL_DETAIL: &[u8] = b"rlibc_i055_missing_symbol_detail\0";

const fn symbol_ptr(bytes: &'static [u8]) -> *const c_char {
  bytes.as_ptr().cast()
}

fn c_string_from_path(path: &Path) -> CString {
  CString::new(path.to_string_lossy().as_bytes()).expect("path must not contain interior NUL")
}

fn first_loaded_shared_object() -> Option<PathBuf> {
  let maps = fs::read_to_string("/proc/self/maps").ok()?;

  for line in maps.lines() {
    let path = line.split_ascii_whitespace().last()?;

    if !path.starts_with('/') || !path.contains(".so") {
      continue;
    }

    let candidate = PathBuf::from(path);

    if candidate.is_file() {
      return Some(candidate);
    }
  }

  None
}

fn take_dlerror_message() -> Option<String> {
  let message_ptr = dlerror();

  if message_ptr.is_null() {
    return None;
  }

  // SAFETY: `dlerror` returns either null or a valid NUL-terminated C string.
  let message = unsafe { CStr::from_ptr(message_ptr.cast_const()) };

  Some(message.to_string_lossy().into_owned())
}

fn clear_pending_dlerror() {
  while take_dlerror_message().is_some() {}
}

fn read_errno() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage for `errno`.
  unsafe { errno_ptr.read() }
}

fn write_errno(value: c_int) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage for `errno`.
  unsafe { errno_ptr.write(value) };
}

#[test]
fn dlsym_resolves_next_getenv_symbol() {
  clear_pending_dlerror();

  // SAFETY: `RTLD_NEXT` is a valid loader sentinel and the symbol string is NUL-terminated.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"getenv\0")) };

  assert!(
    !resolved.is_null(),
    "dlsym(RTLD_NEXT, getenv) must resolve on Linux/glibc",
  );
}

#[test]
fn dlsym_returns_null_for_missing_symbol() {
  clear_pending_dlerror();

  // SAFETY: `RTLD_DEFAULT` and symbol pointer follow the C ABI contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"rlibc_i057_missing_symbol\0")) };

  assert!(resolved.is_null(), "missing symbol should return null");
}

#[test]
fn dlsym_missing_symbol_dlerror_includes_requested_symbol_detail() {
  clear_pending_dlerror();

  // SAFETY: `RTLD_DEFAULT` and symbol pointer follow the C ABI contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(MISSING_SYMBOL_DETAIL)) };

  assert!(resolved.is_null(), "missing symbol should return null");

  let message =
    take_dlerror_message().expect("missing symbol lookup should set detailed dlerror message");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message prefix: {message}",
  );
  assert!(
    message.contains("rlibc_i055_missing_symbol_detail"),
    "dlerror should include the requested missing symbol name: {message}",
  );
  assert!(
    message.contains("rlibc_i055_missing_symbol_detail:"),
    "dlerror should append host detail text after symbol name: {message}",
  );
}

#[test]
fn dlsym_rtld_next_missing_symbol_preserves_errno_and_sets_dlerror() {
  clear_pending_dlerror();
  write_errno(2525);

  // SAFETY: `RTLD_NEXT` is a valid loader sentinel and symbol is a valid NUL-terminated string.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"rlibc_i057_next_missing_symbol\0")) };

  assert!(
    resolved.is_null(),
    "missing RTLD_NEXT symbol should return null"
  );

  let message = take_dlerror_message().expect("missing RTLD_NEXT symbol should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 2525, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_next_missing_symbol_dlerror_includes_requested_symbol_detail() {
  clear_pending_dlerror();
  write_errno(2727);

  let missing_symbol = b"rlibc_i057_next_missing_symbol_detail\0";
  // SAFETY: `RTLD_NEXT` is a valid loader sentinel and symbol is a valid
  // NUL-terminated string.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(missing_symbol)) };

  assert!(
    resolved.is_null(),
    "missing RTLD_NEXT symbol should return null"
  );

  let message =
    take_dlerror_message().expect("missing RTLD_NEXT symbol should set detailed dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message prefix: {message}",
  );
  assert!(
    message.contains("rlibc_i057_next_missing_symbol_detail"),
    "dlerror should include requested RTLD_NEXT symbol name: {message}",
  );
  assert!(
    message.contains("rlibc_i057_next_missing_symbol_detail:"),
    "dlerror should append host detail text after RTLD_NEXT symbol name: {message}",
  );
  assert_eq!(read_errno(), 2727, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_empty_symbol_dlerror_uses_empty_symbol_placeholder() {
  clear_pending_dlerror();
  write_errno(2828);

  // SAFETY: `RTLD_DEFAULT` handle and NUL-only symbol follow the C ABI; empty
  // symbol name is intentional for error-path coverage.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"\0")) };

  assert!(resolved.is_null(), "empty symbol lookup should fail");

  let message = take_dlerror_message().expect("empty symbol lookup should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message prefix: {message}",
  );
  assert!(
    message.contains("requested symbol was not found: <empty symbol>"),
    "dlerror should include explicit empty-symbol placeholder in its base message: {message}",
  );
  assert!(
    message.contains("<empty symbol>"),
    "dlerror should use the empty-symbol placeholder: {message}",
  );
  assert_eq!(read_errno(), 2828, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_next_empty_symbol_dlerror_uses_empty_symbol_placeholder() {
  clear_pending_dlerror();
  write_errno(2929);

  // SAFETY: `RTLD_NEXT` handle and NUL-only symbol follow the C ABI; empty
  // symbol name is intentional for error-path coverage.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"\0")) };

  assert!(
    resolved.is_null(),
    "empty RTLD_NEXT symbol lookup should fail"
  );

  let message = take_dlerror_message().expect("empty RTLD_NEXT symbol lookup should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message prefix: {message}",
  );
  assert!(
    message.contains("requested symbol was not found: <empty symbol>"),
    "dlerror should include explicit empty-symbol placeholder in its base message: {message}",
  );
  assert!(
    message.contains("<empty symbol>"),
    "dlerror should use the empty-symbol placeholder: {message}",
  );
  assert_eq!(read_errno(), 2929, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_does_not_modify_errno_on_success_or_failure() {
  clear_pending_dlerror();

  write_errno(1234);
  // SAFETY: `RTLD_NEXT` and symbol pointer follow C ABI contract.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"getenv\0")) };

  assert!(
    !resolved.is_null(),
    "getenv should resolve through RTLD_NEXT"
  );
  assert_eq!(read_errno(), 1234, "successful dlsym must preserve errno");

  write_errno(4321);
  // SAFETY: `RTLD_DEFAULT` and symbol pointer follow C ABI contract.
  let missing = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"rlibc_i057_missing_symbol\0")) };

  assert!(missing.is_null(), "missing symbol should return null");

  let message = take_dlerror_message().expect("missing symbol should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 4321, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_returns_null_for_null_symbol_pointer() {
  clear_pending_dlerror();
  write_errno(7070);

  // SAFETY: Passing null symbol pointer is invalid by contract and should fail safely.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, ptr::null()) };

  assert!(resolved.is_null(), "null symbol pointer should return null");

  let message = take_dlerror_message().expect("null symbol pointer should set dlerror");

  assert!(
    message.contains("dlsym symbol pointer is null"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 7070, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_returns_null_for_unknown_handle_and_sets_dlerror() {
  clear_pending_dlerror();

  let unknown_handle = 0x00DE_C0DE_usize as *mut c_void;

  write_errno(6060);

  // SAFETY: `symbol` is a valid NUL-terminated string and `handle` is intentionally invalid.
  let resolved = unsafe { dlsym(unknown_handle, symbol_ptr(b"getenv\0")) };
  let message = take_dlerror_message().expect("invalid handle should set dlerror");

  assert!(resolved.is_null(), "unknown handle should return null");
  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 6060, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_unknown_handle_with_null_symbol_reports_invalid_handle_first() {
  clear_pending_dlerror();

  let unknown_handle = 0x00C0_FFEE_usize as *mut c_void;

  write_errno(6262);

  // SAFETY: both arguments are intentionally invalid to assert deterministic
  // validation precedence on non-special handles.
  let resolved = unsafe { dlsym(unknown_handle, ptr::null()) };
  let message =
    take_dlerror_message().expect("unknown-handle lookup with null symbol should set dlerror");

  assert!(
    resolved.is_null(),
    "invalid-handle lookup should return null"
  );
  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    !message.contains("dlsym symbol pointer is null"),
    "non-special handle validation should win over null-symbol validation: {message}",
  );
  assert_eq!(read_errno(), 6262, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_returns_null_for_closed_handle_and_preserves_errno() {
  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  write_errno(2026);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(read_errno(), 2026, "successful dlopen must preserve errno");

  let mut close_attempts = 0_usize;

  loop {
    let close_status = dlclose(handle);

    close_attempts = close_attempts.saturating_add(1);

    if close_status != 0 {
      break;
    }

    assert!(
      close_attempts < 64,
      "failed to observe a closed-handle state after repeated dlclose calls",
    );
  }

  clear_pending_dlerror();

  write_errno(3030);
  // SAFETY: symbol pointer is valid and NUL-terminated.
  let resolved = unsafe { dlsym(handle, symbol_ptr(b"getenv\0")) };

  assert!(resolved.is_null(), "closed handle lookup should fail");

  let message = take_dlerror_message().expect("closed handle lookup should set dlerror");

  assert!(
    message.contains("already closed"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 3030, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_resolves_symbol_for_reopened_handle_after_close() {
  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let first_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !first_handle.is_null(),
    "initial dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    dlclose(first_handle),
    0,
    "closing initial handle should succeed",
  );

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let reopened_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !reopened_handle.is_null(),
    "reopened dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );

  write_errno(4545);

  // SAFETY: symbol pointer is valid and NUL-terminated.
  let resolved = unsafe { dlsym(reopened_handle, symbol_ptr(b"getenv\0")) };

  assert!(!resolved.is_null(), "reopened handle lookup should succeed");
  assert_eq!(read_errno(), 4545, "successful dlsym must preserve errno");
  assert_eq!(
    dlclose(reopened_handle),
    0,
    "closing reopened handle should succeed",
  );
}

#[test]
fn dlsym_reopened_handle_missing_symbol_preserves_errno_and_reports_not_found() {
  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let first_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !first_handle.is_null(),
    "initial dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    dlclose(first_handle),
    0,
    "closing initial handle should succeed",
  );

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let reopened_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !reopened_handle.is_null(),
    "reopened dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );

  write_errno(5151);

  // SAFETY: symbol pointer is valid and NUL-terminated.
  let resolved = unsafe {
    dlsym(
      reopened_handle,
      symbol_ptr(b"rlibc_i057_reopen_missing_symbol\0"),
    )
  };

  assert!(resolved.is_null(), "missing symbol lookup should fail");

  let message = take_dlerror_message().expect("missing symbol lookup should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 5151, "failed dlsym must preserve errno");
  assert_eq!(
    dlclose(reopened_handle),
    0,
    "closing reopened handle should succeed",
  );
}

#[test]
fn dlsym_missing_symbol_replaces_prior_closed_handle_error() {
  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );

  let mut close_attempts = 0_usize;

  loop {
    let close_status = dlclose(handle);

    close_attempts = close_attempts.saturating_add(1);

    if close_status != 0 {
      break;
    }

    assert!(
      close_attempts < 64,
      "failed to observe a closed-handle state after repeated dlclose calls",
    );
  }

  let mut probe_attempts = 0_usize;
  let closed_lookup = loop {
    write_errno(6161);

    // SAFETY: closed-handle path validates the handle before delegating to host resolver.
    let resolved = unsafe { dlsym(handle, symbol_ptr(b"strlen\0")) };

    if resolved.is_null() {
      break resolved;
    }

    let _ = dlclose(handle);

    probe_attempts = probe_attempts.saturating_add(1);
    assert!(
      probe_attempts < 64,
      "closed handle lookup should eventually fail after repeated close attempts",
    );
  };

  assert!(closed_lookup.is_null(), "closed handle lookup should fail");
  assert_eq!(read_errno(), 6161, "failed dlsym must preserve errno");

  write_errno(7171);

  // SAFETY: symbol pointer is valid and NUL-terminated.
  let missing_lookup = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"rlibc_i057_latest_missing\0")) };

  assert!(
    missing_lookup.is_null(),
    "missing symbol lookup should fail"
  );

  let message =
    take_dlerror_message().expect("latest missing-symbol error should replace prior dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 7171, "failed dlsym must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlsym_success_does_not_clear_prior_error_message() {
  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(dlclose(handle), 0, "first close should succeed");

  write_errno(8181);

  // SAFETY: closed-handle path validates the handle before delegating to host resolver.
  let closed_lookup = unsafe { dlsym(handle, symbol_ptr(b"strlen\0")) };

  assert!(closed_lookup.is_null(), "closed handle lookup should fail");
  assert_eq!(read_errno(), 8181, "failed dlsym must preserve errno");

  write_errno(9191);

  // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy C ABI contract.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"getenv\0")) };

  assert!(
    !resolved.is_null(),
    "successful dlsym should resolve getenv through RTLD_NEXT",
  );
  assert_eq!(read_errno(), 9191, "successful dlsym must preserve errno");

  let message =
    take_dlerror_message().expect("successful dlsym should not clear prior pending dlerror");

  assert!(
    message.contains("already closed"),
    "unexpected preserved dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlsym_error_state_is_thread_local_between_main_and_child_threads() {
  clear_pending_dlerror();

  let child_message = thread::spawn(|| {
    clear_pending_dlerror();

    let unknown_handle = 0x0BAD_5EED_usize as *mut c_void;
    // SAFETY: `symbol` is a valid NUL-terminated string and `handle` is intentionally invalid.
    let resolved = unsafe { dlsym(unknown_handle, symbol_ptr(b"getenv\0")) };

    assert!(resolved.is_null(), "unknown handle should return null");

    take_dlerror_message()
  })
  .join()
  .expect("child thread panicked")
  .expect("child thread should observe dlsym error");

  assert!(
    child_message.contains("invalid dynamic-loader handle"),
    "unexpected child dlerror message: {child_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "child-thread dlsym failure must not leak dlerror state into main thread",
  );
}
