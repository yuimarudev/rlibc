use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use rlibc::dlfcn::{RTLD_LAZY, RTLD_NOW, dlclose, dlerror, dlopen, dlsym};
use rlibc::errno::__errno_location;
use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::{fs, thread};

const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;

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

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe {
    __errno_location().write(value);
  }
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns readable thread-local errno storage.
  unsafe { __errno_location().read() }
}

fn c_string(input: &str) -> CString {
  CString::new(input)
    .unwrap_or_else(|_| unreachable!("test literals must not include interior NUL bytes"))
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

#[test]
fn dlerror_returns_null_without_pending_error() {
  clear_pending_dlerror();

  assert!(dlerror().is_null(), "no pending error must return null");
}

#[test]
fn dlerror_returns_message_once_for_dlclose_failure() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let first = take_dlerror_message().expect("expected pending message after dlclose failure");
  let second = take_dlerror_message();

  assert!(
    first.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {first}",
  );
  assert!(second.is_none(), "second call must clear pending error");
}

#[test]
fn dlerror_newer_error_replaces_older_pending_message() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  // SAFETY: passing null symbol pointer is intentional to trigger error-path behavior.
  let _ = unsafe { dlsym(ptr::null_mut(), ptr::null()) };
  let message = take_dlerror_message().expect("expected most recent pending message");

  assert!(
    message.contains("dlsym symbol pointer is null"),
    "newest error should replace prior pending message, got: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlerror_stays_empty_after_successful_dlsym_lookup() {
  clear_pending_dlerror();

  // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
  let resolved = unsafe { dlsym(ptr::null_mut(), c"getenv".as_ptr()) };

  assert!(
    !resolved.is_null(),
    "dlsym should resolve getenv for RTLD_DEFAULT",
  );
  assert!(
    take_dlerror_message().is_none(),
    "successful dlsym from clean state must not create a dlerror message",
  );
}

#[test]
fn dlerror_pending_message_survives_successful_dlsym() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
  let resolved = unsafe { dlsym(ptr::null_mut(), c"getenv".as_ptr()) };

  assert!(
    !resolved.is_null(),
    "dlsym should resolve getenv for RTLD_DEFAULT",
  );

  let message = take_dlerror_message()
    .expect("successful dlsym should not clear a pre-existing pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlerror_pending_message_survives_successful_dlsym_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(4242);

  // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
  let resolved = unsafe { dlsym(ptr::null_mut(), c"getenv".as_ptr()) };

  assert!(
    !resolved.is_null(),
    "dlsym should resolve getenv for RTLD_DEFAULT",
  );
  assert_eq!(
    read_errno(),
    4242,
    "successful dlsym should preserve errno while pending dlerror exists",
  );

  let message = take_dlerror_message()
    .expect("successful dlsym should not clear a pre-existing pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading pending dlerror must preserve errno"
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "clearing pending dlerror must preserve errno"
  );
}

#[test]
fn dlerror_pending_message_survives_successful_dlsym_with_rtld_next_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(4242);

  // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the C ABI contract.
  let resolved = unsafe { dlsym(RTLD_NEXT, c"getenv".as_ptr()) };

  assert!(
    !resolved.is_null(),
    "dlsym should resolve getenv for RTLD_NEXT",
  );
  assert_eq!(
    read_errno(),
    4242,
    "successful dlsym should preserve errno while pending dlerror exists",
  );

  let message = take_dlerror_message()
    .expect("successful RTLD_NEXT dlsym should not clear a pre-existing pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading pending dlerror must preserve errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "clearing pending dlerror must preserve errno",
  );
}

#[test]
fn dlerror_pending_message_survives_successful_rtld_next_dlsym_then_dlopen_and_dlclose_and_preserves_errno()
 {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(5151);

  // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the C ABI contract.
  let resolved = unsafe { dlsym(RTLD_NEXT, c"getenv".as_ptr()) };

  assert!(
    !resolved.is_null(),
    "dlsym should resolve getenv for RTLD_NEXT",
  );
  assert_eq!(
    read_errno(),
    5151,
    "successful RTLD_NEXT dlsym should preserve errno while pending dlerror exists",
  );

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    read_errno(),
    5151,
    "successful dlopen should preserve errno while pending dlerror exists",
  );

  assert_eq!(dlclose(handle), 0, "dlclose should release test handle");
  assert_eq!(read_errno(), 5151, "successful dlclose must preserve errno");

  let message = take_dlerror_message()
    .expect("successful RTLD_NEXT dlsym/dlopen/dlclose sequence should not clear pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    5151,
    "reading pending dlerror must preserve errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    5151,
    "clearing pending dlerror must preserve errno",
  );
}

#[test]
fn dlerror_pending_message_survives_successful_rtld_next_then_rtld_default_dlsym_and_preserves_errno()
 {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(7373);

  // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the C ABI contract.
  let next_resolved = unsafe { dlsym(RTLD_NEXT, c"strlen".as_ptr()) };

  assert!(
    !next_resolved.is_null(),
    "dlsym should resolve strlen for RTLD_NEXT",
  );
  assert_eq!(
    read_errno(),
    7373,
    "successful RTLD_NEXT dlsym should preserve errno while pending dlerror exists",
  );

  // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
  let default_resolved = unsafe { dlsym(ptr::null_mut(), c"dlopen".as_ptr()) };

  assert!(
    !default_resolved.is_null(),
    "dlsym should resolve dlopen for RTLD_DEFAULT",
  );
  assert_eq!(
    read_errno(),
    7373,
    "successful RTLD_DEFAULT dlsym should preserve errno while pending dlerror exists",
  );

  let message = take_dlerror_message()
    .expect("successful RTLD_NEXT/RTLD_DEFAULT dlsym sequence should not clear pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    7373,
    "reading pending dlerror must preserve errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    7373,
    "clearing pending dlerror must preserve errno",
  );
}

#[test]
fn dlerror_pending_message_survives_successful_rtld_default_then_rtld_next_dlsym_and_preserves_errno()
 {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(7474);

  // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
  let default_resolved = unsafe { dlsym(ptr::null_mut(), c"dlopen".as_ptr()) };

  assert!(
    !default_resolved.is_null(),
    "dlsym should resolve dlopen for RTLD_DEFAULT",
  );
  assert_eq!(
    read_errno(),
    7474,
    "successful RTLD_DEFAULT dlsym should preserve errno while pending dlerror exists",
  );

  // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the C ABI contract.
  let next_resolved = unsafe { dlsym(RTLD_NEXT, c"strlen".as_ptr()) };

  assert!(
    !next_resolved.is_null(),
    "dlsym should resolve strlen for RTLD_NEXT",
  );
  assert_eq!(
    read_errno(),
    7474,
    "successful RTLD_NEXT dlsym should preserve errno while pending dlerror exists",
  );

  let message = take_dlerror_message()
    .expect("successful RTLD_DEFAULT/RTLD_NEXT dlsym sequence should not clear pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    7474,
    "reading pending dlerror must preserve errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    7474,
    "clearing pending dlerror must preserve errno",
  );
}

#[test]
fn dlerror_rtld_next_missing_symbol_replaces_pending_message_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(9191);

  // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the C ABI contract.
  let resolved = unsafe { dlsym(RTLD_NEXT, c"rlibc_i055_rtld_next_missing_symbol".as_ptr()) };

  assert!(
    resolved.is_null(),
    "missing RTLD_NEXT symbol lookup should fail"
  );
  assert_eq!(read_errno(), 9191, "failed dlsym must preserve errno");

  let message = take_dlerror_message()
    .expect("missing RTLD_NEXT symbol failure should replace pending dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 9191, "reading dlerror must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    9191,
    "clearing pending dlerror must preserve errno",
  );
}

#[test]
fn dlerror_pending_message_survives_successful_dlopen() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(4242);

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    read_errno(),
    4242,
    "successful dlopen should preserve errno while pending dlerror exists",
  );

  let message = take_dlerror_message()
    .expect("successful dlopen should not clear a pre-existing pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading pending dlerror must preserve errno"
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "clearing pending dlerror must preserve errno"
  );

  assert_eq!(dlclose(handle), 0, "dlclose should release test handle");
  assert_eq!(read_errno(), 4242, "successful dlclose must preserve errno");
}

#[test]
fn dlerror_pending_message_survives_successful_dlopen_lazy_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(4242);

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LAZY) };

  assert!(
    !handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    read_errno(),
    4242,
    "successful dlopen should preserve errno while pending dlerror exists",
  );

  let message = take_dlerror_message()
    .expect("successful dlopen should not clear a pre-existing pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading pending dlerror must preserve errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "clearing pending dlerror must preserve errno",
  );

  assert_eq!(dlclose(handle), 0, "dlclose should release test handle");
  assert_eq!(read_errno(), 4242, "successful dlclose must preserve errno");
}

#[test]
fn dlerror_pending_message_survives_successful_dlopen_then_dlsym_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);
  write_errno(4242);

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    read_errno(),
    4242,
    "successful dlopen should preserve errno while pending dlerror exists",
  );

  // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
  let resolved = unsafe { dlsym(ptr::null_mut(), c"getenv".as_ptr()) };

  assert!(
    !resolved.is_null(),
    "dlsym should resolve getenv for RTLD_DEFAULT",
  );
  assert_eq!(
    read_errno(),
    4242,
    "successful dlsym should preserve errno while pending dlerror exists",
  );

  let message = take_dlerror_message()
    .expect("successful dlopen/dlsym sequence should not clear a pre-existing pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading pending dlerror must preserve errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "clearing pending dlerror must preserve errno",
  );

  assert_eq!(dlclose(handle), 0, "dlclose should release test handle");
  assert_eq!(read_errno(), 4242, "successful dlclose must preserve errno");
}

#[test]
fn dlerror_reopened_handle_missing_symbol_replaces_pending_message() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let first_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !first_handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    dlclose(first_handle),
    0,
    "dlclose should release first handle"
  );

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let reopened_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !reopened_handle.is_null(),
    "reopened dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );

  write_errno(9191);

  // SAFETY: `reopened_handle` was produced by dlopen and symbol string is NUL-terminated.
  let resolved = unsafe {
    dlsym(
      reopened_handle,
      c"rlibc_i057_dlerror_reopen_missing_symbol".as_ptr(),
    )
  };

  assert!(resolved.is_null(), "missing symbol lookup should fail");

  let message = take_dlerror_message()
    .expect("newer reopened-handle dlsym failure should replace pending dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 9191, "failed dlsym must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );

  assert_eq!(
    dlclose(reopened_handle),
    0,
    "dlclose should release reopened handle",
  );
}

#[test]
fn dlerror_stays_empty_after_successful_dlopen() {
  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert!(
    take_dlerror_message().is_none(),
    "successful dlopen from a clean state must not create a dlerror message",
  );

  assert_eq!(dlclose(handle), 0, "dlclose should release test handle");
}

#[test]
fn dlerror_stays_empty_after_successful_dlclose() {
  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert!(
    take_dlerror_message().is_none(),
    "successful dlopen from a clean state must not create a dlerror message",
  );

  assert_eq!(dlclose(handle), 0, "dlclose should release test handle");
  assert!(
    take_dlerror_message().is_none(),
    "successful dlclose from a clean state must not create a dlerror message",
  );
}

#[test]
fn dlerror_pending_message_survives_successful_dlclose_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );

  write_errno(6262);
  assert_eq!(dlclose(handle), 0, "dlclose should release test handle");
  assert_eq!(read_errno(), 6262, "successful dlclose must preserve errno");

  let message = take_dlerror_message()
    .expect("successful dlclose should not clear a pre-existing pending dlerror");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected preserved dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    6262,
    "reading dlerror must not mutate errno after successful dlclose",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlerror_closed_handle_dlclose_failure_replaces_pending_message_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    dlclose(handle),
    0,
    "first dlclose should release test handle"
  );

  write_errno(8484);
  assert_eq!(dlclose(handle), -1, "second dlclose should fail");
  assert_eq!(read_errno(), 8484, "failed dlclose must preserve errno");

  let message = take_dlerror_message()
    .expect("closed-handle dlclose failure should replace older pending dlerror");

  assert!(
    message.contains("dynamic-loader handle already closed"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    8484,
    "reading dlerror must not mutate errno after failed dlclose",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_successful_dlopen() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let shared_object_path_text = shared_object_path.to_string_lossy().into_owned();
  let child = thread::spawn(move || {
    clear_pending_dlerror();

    let path_cstr = CString::new(shared_object_path_text.as_bytes())
      .expect("shared object path must not contain interior NUL");

    // SAFETY: `path_cstr` is a valid NUL-terminated C string.
    let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

    assert!(!handle.is_null(), "child dlopen should succeed");

    let child_message_after_success = take_dlerror_message();

    assert_eq!(
      dlclose(handle),
      0,
      "child dlclose should release test handle"
    );

    child_message_after_success
  });
  let child_message = child.join().expect("child thread panicked");

  assert!(
    child_message.is_none(),
    "child successful dlopen must not create dlerror state",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending error should survive child successful dlopen");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_successful_dlopen_and_preserves_main_errno() {
  clear_pending_dlerror();
  write_errno(4242);

  assert_eq!(dlclose(ptr::null_mut()), -1);
  assert_eq!(read_errno(), 4242, "main-thread errno must be preserved");

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let shared_object_path_text = shared_object_path.to_string_lossy().into_owned();
  let child = thread::spawn(move || {
    clear_pending_dlerror();
    write_errno(3131);

    let path_cstr = CString::new(shared_object_path_text.as_bytes())
      .expect("shared object path must not contain interior NUL");

    // SAFETY: `path_cstr` is a valid NUL-terminated C string.
    let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

    assert!(!handle.is_null(), "child dlopen should succeed");
    assert_eq!(
      read_errno(),
      3131,
      "child successful dlopen must preserve errno"
    );

    let child_message_after_success = take_dlerror_message();

    assert_eq!(
      dlclose(handle),
      0,
      "child dlclose should release test handle"
    );

    child_message_after_success
  });
  let child_message = child.join().expect("child thread panicked");

  assert!(
    child_message.is_none(),
    "child successful dlopen must not create dlerror state",
  );

  assert_eq!(
    read_errno(),
    4242,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending error should survive child successful dlopen");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading main-thread dlerror must not mutate errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "clearing main-thread pending dlerror must not mutate errno",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_successful_dlclose_and_preserves_main_errno() {
  clear_pending_dlerror();
  write_errno(4242);

  assert_eq!(dlclose(ptr::null_mut()), -1);
  assert_eq!(read_errno(), 4242, "main-thread errno must be preserved");

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let shared_object_path_text = shared_object_path.to_string_lossy().into_owned();
  let child = thread::spawn(move || {
    clear_pending_dlerror();
    write_errno(3131);

    let path_cstr = CString::new(shared_object_path_text.as_bytes())
      .expect("shared object path must not contain interior NUL");

    // SAFETY: `path_cstr` is a valid NUL-terminated C string.
    let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

    assert!(!handle.is_null(), "child dlopen should succeed");
    assert!(
      take_dlerror_message().is_none(),
      "child successful dlopen must not create dlerror state",
    );
    assert_eq!(
      read_errno(),
      3131,
      "child successful dlopen must preserve errno"
    );

    assert_eq!(
      dlclose(handle),
      0,
      "child dlclose should release test handle"
    );
    assert_eq!(
      read_errno(),
      3131,
      "child successful dlclose must preserve errno"
    );

    take_dlerror_message()
  });
  let child_message = child.join().expect("child thread panicked");

  assert!(
    child_message.is_none(),
    "child successful dlclose must not create dlerror state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending error should survive child successful dlclose");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading main-thread dlerror must not mutate errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_failed_dlclose_and_preserves_main_errno() {
  clear_pending_dlerror();
  write_errno(4242);

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(!handle.is_null(), "main-thread dlopen should succeed");
  assert_eq!(
    dlclose(handle),
    0,
    "main-thread first dlclose should release handle",
  );
  assert_eq!(
    dlclose(handle),
    -1,
    "main-thread second dlclose should fail with already-closed error",
  );
  assert_eq!(read_errno(), 4242, "main-thread errno must be preserved");

  let child = thread::spawn(|| {
    clear_pending_dlerror();
    write_errno(3131);

    let unknown_handle = 0x0D15_EA5E_usize as *mut c_void;

    assert_eq!(
      dlclose(unknown_handle),
      -1,
      "child unknown-handle close should fail",
    );
    assert_eq!(
      read_errno(),
      3131,
      "child failed dlclose must preserve errno"
    );

    let child_message =
      take_dlerror_message().expect("child thread should observe its own dlerror message");

    assert!(
      child_message.contains("invalid dynamic-loader handle"),
      "unexpected child message: {child_message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "child second call must clear child pending state",
    );
  });

  child.join().expect("child thread panicked");

  assert_eq!(
    read_errno(),
    4242,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message =
    take_dlerror_message().expect("main-thread pending error should survive child failed dlclose");

  assert!(
    main_message.contains("already closed"),
    "unexpected main-thread message: {main_message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading main-thread dlerror must not mutate errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_successful_dlsym() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let child = thread::spawn(|| {
    clear_pending_dlerror();

    // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
    let resolved = unsafe { dlsym(ptr::null_mut(), c"dlopen".as_ptr()) };

    assert!(
      !resolved.is_null(),
      "child dlsym should resolve dlopen from RTLD_DEFAULT",
    );

    take_dlerror_message()
  });
  let child_message = child.join().expect("child thread panicked");

  assert!(
    child_message.is_none(),
    "child successful dlsym must not create dlerror state",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending error should survive child successful dlsym");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_successful_dlsym_and_preserves_errno() {
  clear_pending_dlerror();
  write_errno(4242);

  assert_eq!(dlclose(ptr::null_mut()), -1);
  assert_eq!(read_errno(), 4242, "main-thread errno must be preserved");

  let child = thread::spawn(|| {
    clear_pending_dlerror();
    write_errno(3131);

    // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
    let resolved = unsafe { dlsym(ptr::null_mut(), c"strlen".as_ptr()) };

    assert!(!resolved.is_null(), "child dlsym should resolve strlen");
    assert_eq!(read_errno(), 3131, "successful dlsym must preserve errno");

    take_dlerror_message()
  });
  let child_message = child.join().expect("child thread panicked");

  assert!(
    child_message.is_none(),
    "child successful dlsym must not create dlerror state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending error should survive child successful dlsym");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading main-thread dlerror must not mutate errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_successful_rtld_next_dlsym_and_preserves_main_errno()
{
  clear_pending_dlerror();
  write_errno(4242);

  assert_eq!(dlclose(ptr::null_mut()), -1);
  assert_eq!(read_errno(), 4242, "main-thread errno must be preserved");

  let child = thread::spawn(|| {
    clear_pending_dlerror();
    write_errno(3131);

    // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the C ABI contract.
    let resolved = unsafe { dlsym(RTLD_NEXT, c"getenv".as_ptr()) };

    assert!(
      !resolved.is_null(),
      "child dlsym should resolve getenv through RTLD_NEXT"
    );
    assert_eq!(
      read_errno(),
      3131,
      "child successful RTLD_NEXT dlsym must preserve errno",
    );

    take_dlerror_message()
  });
  let child_message = child.join().expect("child thread panicked");

  assert!(
    child_message.is_none(),
    "child successful RTLD_NEXT dlsym must not create dlerror state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending error should survive child successful RTLD_NEXT dlsym");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading main-thread dlerror must not mutate errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "clearing main-thread pending dlerror must preserve errno",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_missing_symbol_failure_and_preserves_errno() {
  clear_pending_dlerror();
  write_errno(4242);

  assert_eq!(dlclose(ptr::null_mut()), -1);
  assert_eq!(read_errno(), 4242, "main-thread errno must be preserved");

  let child = thread::spawn(|| {
    clear_pending_dlerror();
    write_errno(3131);

    // SAFETY: null handle denotes RTLD_DEFAULT and symbol string is NUL-terminated.
    let resolved = unsafe { dlsym(ptr::null_mut(), c"rlibc_i057_child_missing_symbol".as_ptr()) };

    assert!(
      resolved.is_null(),
      "child missing-symbol lookup should fail"
    );
    assert_eq!(read_errno(), 3131, "failed child dlsym must preserve errno");

    let child_message =
      take_dlerror_message().expect("child thread should observe missing-symbol dlerror");

    assert!(
      child_message.contains("requested symbol was not found"),
      "unexpected child message: {child_message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "child second call must clear child pending state",
    );
  });

  child.join().expect("child thread panicked");

  assert_eq!(
    read_errno(),
    4242,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending error should survive child missing-symbol failure");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading main-thread dlerror must not mutate errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
}

#[test]
fn dlerror_pending_message_isolated_from_child_rtld_next_missing_symbol_failure_and_preserves_main_errno()
 {
  clear_pending_dlerror();
  write_errno(4242);

  assert_eq!(dlclose(ptr::null_mut()), -1);
  assert_eq!(read_errno(), 4242, "main-thread errno must be preserved");

  let child = thread::spawn(|| {
    clear_pending_dlerror();
    write_errno(3131);

    // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the C ABI contract.
    let resolved = unsafe {
      dlsym(
        RTLD_NEXT,
        c"rlibc_i055_child_rtld_next_missing_symbol".as_ptr(),
      )
    };

    assert!(
      resolved.is_null(),
      "child RTLD_NEXT missing-symbol lookup should fail",
    );
    assert_eq!(
      read_errno(),
      3131,
      "failed child RTLD_NEXT dlsym must preserve errno",
    );

    let child_message =
      take_dlerror_message().expect("child thread should observe RTLD_NEXT missing-symbol dlerror");

    assert!(
      child_message.contains("requested symbol was not found"),
      "unexpected child message: {child_message}",
    );
    assert_eq!(
      read_errno(),
      3131,
      "reading child dlerror must preserve child errno",
    );
    assert!(
      take_dlerror_message().is_none(),
      "child second call must clear child pending state",
    );
    assert_eq!(
      read_errno(),
      3131,
      "clearing child pending dlerror must preserve child errno",
    );
  });

  child.join().expect("child thread panicked");

  assert_eq!(
    read_errno(),
    4242,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending error should survive child RTLD_NEXT missing-symbol failure");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
  assert_eq!(
    read_errno(),
    4242,
    "reading main-thread dlerror must not mutate errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear main-thread pending state",
  );
  assert_eq!(
    read_errno(),
    4242,
    "clearing main-thread pending dlerror must preserve errno",
  );
}

#[test]
fn dlerror_is_thread_local_between_main_and_child_threads() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let child = thread::spawn(|| {
    clear_pending_dlerror();

    let child_initial = take_dlerror_message();

    // SAFETY: passing null symbol pointer is intentional to trigger error-path behavior.
    let resolved_is_null = unsafe { dlsym(ptr::null_mut(), ptr::null()).is_null() };
    let child_message =
      take_dlerror_message().expect("child thread should receive its own dlerror message");
    let child_after_clear = take_dlerror_message();

    (
      resolved_is_null,
      child_initial,
      child_message,
      child_after_clear,
    )
  });
  let (resolved_is_null, child_initial, child_message, child_after_clear) =
    child.join().expect("child thread panicked");
  let main_message = take_dlerror_message().expect("main thread should keep its pending message");

  assert!(resolved_is_null, "null symbol pointer must fail");
  assert!(
    child_initial.is_none(),
    "child thread must not inherit main-thread pending error",
  );
  assert!(
    child_message.contains("dlsym symbol pointer is null"),
    "unexpected child message: {child_message}",
  );
  assert!(
    child_after_clear.is_none(),
    "child second call must clear child pending error",
  );
  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "unexpected main-thread message: {main_message}",
  );
}

#[test]
fn dlerror_does_not_modify_errno() {
  clear_pending_dlerror();

  write_errno(777);
  assert_eq!(dlclose(ptr::null_mut()), -1);

  let _message = take_dlerror_message().expect("expected pending message");

  assert_eq!(read_errno(), 777);
}

#[test]
fn dlerror_reports_dlopen_invalid_flags_failure() {
  clear_pending_dlerror();

  let missing_path = c_string("/definitely/missing/rlibc_i055_dlopen_invalid_flags.so");
  let invalid_flags = RTLD_NOW | RTLD_LAZY;

  // SAFETY: `missing_path` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), invalid_flags) };
  let message =
    take_dlerror_message().expect("dlopen invalid flags failure should set a dlerror message");

  assert!(handle.is_null(), "invalid flags must fail");
  assert!(
    message.contains("invalid flags"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlerror_reports_dlopen_null_filename_failure() {
  clear_pending_dlerror();

  // SAFETY: null pointer is intentional to validate input checking.
  let handle = unsafe { dlopen(ptr::null(), RTLD_NOW) };
  let message = take_dlerror_message().expect("null filename failure should set dlerror message");

  assert!(handle.is_null(), "null filename must fail");
  assert!(
    message.contains("path pointer is null"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlerror_reports_dlopen_missing_path_failure() {
  clear_pending_dlerror();

  let missing_path = c_string("/definitely/missing/rlibc_i055_dlopen_missing_path.so");
  // SAFETY: `missing_path` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), RTLD_NOW) };
  let message = take_dlerror_message().expect("missing path failure should set dlerror message");

  assert!(handle.is_null(), "missing path must fail");
  assert!(
    message.contains("target path could not be opened"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlerror_missing_path_pending_message_survives_successful_dlopen_and_preserves_errno() {
  clear_pending_dlerror();

  let missing_path =
    c_string("/definitely/missing/rlibc_i055_missing_path_pending_survives_successful_dlopen.so");

  // SAFETY: `missing_path` is a valid NUL-terminated C string.
  let missing_handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(missing_handle.is_null(), "missing path must fail");

  let failure_errno = read_errno();

  assert_ne!(
    failure_errno, 0,
    "missing-path dlopen failure should set a non-zero errno",
  );

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let shared_object_cstr = CString::new(shared_object_path.to_string_lossy().as_bytes())
    .expect("shared object path must not contain interior NUL");

  // SAFETY: `shared_object_cstr` is a valid NUL-terminated C string.
  let success_handle = unsafe { dlopen(shared_object_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !success_handle.is_null(),
    "dlopen should succeed for loaded shared object: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "successful dlopen must preserve errno while pending missing-path dlerror exists",
  );

  let message = take_dlerror_message()
    .expect("successful dlopen should not clear pending missing-path dlerror");

  assert!(
    message.contains("target path could not be opened"),
    "unexpected preserved dlerror message: {message}",
  );
  assert!(
    message.contains("missing_path_pending_survives_successful_dlopen"),
    "preserved message should include missing-path detail: {message}",
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "reading pending dlerror must preserve errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "clearing pending dlerror must preserve errno",
  );

  assert_eq!(
    dlclose(success_handle),
    0,
    "dlclose should release test handle"
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "successful dlclose must preserve errno"
  );
}

#[test]
fn dlerror_reports_dlopen_non_elf_failure() {
  clear_pending_dlerror();

  let non_elf_path = std::env::temp_dir().join(format!(
    "rlibc_i055_dlerror_non_elf_{}_{}",
    std::process::id(),
    thread::current().name().unwrap_or("main"),
  ));

  fs::write(&non_elf_path, b"not an elf image")
    .expect("failed to create non-ELF fixture for dlerror test");

  let path_cstr = CString::new(non_elf_path.to_string_lossy().as_bytes())
    .expect("temp path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };
  let message = take_dlerror_message().expect("non-ELF failure should set dlerror message");

  assert!(handle.is_null(), "non-ELF input must fail");
  assert!(
    message.contains("not a valid ELF image"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );

  fs::remove_file(&non_elf_path).expect("failed to remove non-ELF fixture");
}

#[test]
fn dlerror_reports_host_dlopen_failure_with_detail_for_malformed_elf() {
  clear_pending_dlerror();

  let malformed_elf_path = std::env::temp_dir().join(format!(
    "rlibc_i055_dlerror_malformed_elf_{}_{}",
    std::process::id(),
    thread::current().name().unwrap_or("main"),
  ));

  fs::write(&malformed_elf_path, b"\x7FELFbroken")
    .expect("failed to create malformed-ELF fixture for dlerror test");

  let path_cstr = CString::new(malformed_elf_path.to_string_lossy().as_bytes())
    .expect("temp path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };
  let message =
    take_dlerror_message().expect("host dlopen failure should set a detailed dlerror message");

  assert!(handle.is_null(), "malformed ELF input must fail");
  assert!(
    message.contains("host dlopen call failed"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    message.contains("malformed_elf"),
    "dlerror should include host detail text for the failing path: {message}",
  );
  assert!(
    message.contains(':'),
    "dlerror should contain a detail separator for host failure diagnostics: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );

  fs::remove_file(&malformed_elf_path).expect("failed to remove malformed-ELF fixture");
}

#[test]
fn dlerror_host_dlopen_failure_read_and_clear_preserve_errno() {
  clear_pending_dlerror();

  let malformed_elf_path = std::env::temp_dir().join(format!(
    "rlibc_i055_dlerror_malformed_errno_{}_{}",
    std::process::id(),
    thread::current().name().unwrap_or("main"),
  ));

  fs::write(&malformed_elf_path, b"\x7FELFbroken")
    .expect("failed to create malformed-ELF fixture for errno test");

  let path_cstr = CString::new(malformed_elf_path.to_string_lossy().as_bytes())
    .expect("temp path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(handle.is_null(), "malformed ELF input must fail");

  let failure_errno = read_errno();

  assert_ne!(
    failure_errno, 0,
    "host dlopen failure should set a non-zero errno",
  );

  let message = take_dlerror_message().expect("host dlopen failure should set dlerror message");

  assert!(
    message.contains("host dlopen call failed"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "reading dlerror must preserve errno after host dlopen failure",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "clearing dlerror must preserve errno after host dlopen failure",
  );

  fs::remove_file(&malformed_elf_path).expect("failed to remove malformed-ELF fixture");
}

#[test]
fn dlerror_host_dlopen_failure_replaces_older_pending_message_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let malformed_elf_path = std::env::temp_dir().join(format!(
    "rlibc_i055_dlerror_malformed_replace_{}_{}",
    std::process::id(),
    thread::current().name().unwrap_or("main"),
  ));

  fs::write(&malformed_elf_path, b"\x7FELFbroken")
    .expect("failed to create malformed-ELF fixture for replacement test");

  let path_cstr = CString::new(malformed_elf_path.to_string_lossy().as_bytes())
    .expect("temp path must not contain interior NUL");

  // SAFETY: `path_cstr` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(handle.is_null(), "malformed ELF input must fail");

  let failure_errno = read_errno();

  assert_ne!(
    failure_errno, 0,
    "host dlopen failure should set a non-zero errno",
  );

  let message =
    take_dlerror_message().expect("host dlopen failure should replace older pending dlerror");

  assert!(
    message.contains("host dlopen call failed"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    message.contains("malformed_replace"),
    "dlerror should include host detail text for the failing path: {message}",
  );
  assert!(
    !message.contains("invalid dynamic-loader handle"),
    "newer host dlopen failure must replace older pending message: {message}",
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "reading dlerror must preserve errno after host dlopen failure",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "clearing dlerror must preserve errno after host dlopen failure",
  );

  fs::remove_file(&malformed_elf_path).expect("failed to remove malformed-ELF fixture");
}

#[test]
fn dlerror_dlopen_failure_replaces_older_pending_message() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let missing_path = c_string("/definitely/missing/rlibc_i055_dlopen_replaces_error.so");
  // SAFETY: `missing_path` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), RTLD_NOW) };
  let message =
    take_dlerror_message().expect("newer dlopen failure should replace pending dlerror");

  assert!(handle.is_null(), "missing path must fail");
  assert!(
    message.contains("target path could not be opened"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlerror_dlopen_missing_path_failure_replaces_older_pending_message_and_preserves_errno() {
  clear_pending_dlerror();

  assert_eq!(dlclose(ptr::null_mut()), -1);

  let missing_path =
    c_string("/definitely/missing/rlibc_i055_dlopen_replaces_error_preserve_errno.so");
  // SAFETY: `missing_path` is a valid NUL-terminated C string.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(handle.is_null(), "missing path must fail");

  let failure_errno = read_errno();

  assert_ne!(
    failure_errno, 0,
    "missing-path dlopen failure should set a non-zero errno",
  );

  let message =
    take_dlerror_message().expect("newer dlopen missing-path failure should replace pending error");

  assert!(
    message.contains("target path could not be opened"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    !message.contains("invalid dynamic-loader handle"),
    "newer dlopen failure must replace older pending message: {message}",
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "reading dlerror must preserve errno after missing-path dlopen failure",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
  assert_eq!(
    read_errno(),
    failure_errno,
    "clearing dlerror must preserve errno after missing-path dlopen failure",
  );
}
