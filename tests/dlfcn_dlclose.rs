use core::ffi::{c_char, c_int, c_void};
use rlibc::dlfcn::{RTLD_NOW, dlclose, dlerror, dlopen, dlsym};
use rlibc::errno::__errno_location;
use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::{fs, thread};

type StrlenFn = unsafe extern "C" fn(*const c_char) -> usize;

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
  unsafe { __errno_location().write(value) };
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns readable thread-local errno storage.
  unsafe { __errno_location().read() }
}

fn loaded_libc_path() -> Option<PathBuf> {
  let maps = fs::read_to_string("/proc/self/maps").ok()?;

  for line in maps.lines() {
    let path = line.split_ascii_whitespace().last()?;

    if !path.starts_with('/') || !path.contains("libc.so") {
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
fn dlclose_unknown_handle_sets_error_message_once() {
  clear_pending_dlerror();

  let unknown_handle = 0x0BAD_C0DE_usize as *mut c_void;

  assert_eq!(dlclose(unknown_handle), -1);

  let message = take_dlerror_message().expect("expected dlerror message after dlclose failure");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_rtld_next_like_handle_sets_invalid_message_and_preserves_errno() {
  clear_pending_dlerror();
  write_errno(9190);

  let rtld_next_like_handle = (-1_isize) as *mut c_void;

  assert_eq!(dlclose(rtld_next_like_handle), -1);
  assert_eq!(
    read_errno(),
    9190,
    "RTLD_NEXT-like handle failure must preserve caller errno",
  );

  let message =
    take_dlerror_message().expect("expected invalid-handle dlerror for RTLD_NEXT-like handle");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_null_handle_sets_error_message() {
  clear_pending_dlerror();

  assert_eq!(dlclose(core::ptr::null_mut()), -1);

  let message = take_dlerror_message().expect("expected dlerror message for null handle");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
}

#[test]
fn dlclose_second_close_after_final_reference_sets_closed_handle_error() {
  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
    .expect("libc path should not contain interior NUL");

  // SAFETY: `libc_cstr` is a valid NUL-terminated path.
  let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "expected dlopen to open libc: {}",
    libc_path.display(),
  );

  assert_eq!(dlclose(handle), 0, "first dlclose should succeed");
  assert_eq!(dlclose(handle), -1, "second dlclose should fail");

  let message = take_dlerror_message().expect("expected dlerror message after second close");

  assert!(
    message.contains("dynamic-loader handle already closed"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_second_close_preserves_errno_and_reports_closed_handle_error() {
  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
    .expect("libc path should not contain interior NUL");

  // SAFETY: `libc_cstr` is a valid NUL-terminated path.
  let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "expected dlopen to open libc: {}",
    libc_path.display(),
  );
  assert_eq!(dlclose(handle), 0, "first dlclose should succeed");

  write_errno(7373);
  assert_eq!(dlclose(handle), -1, "second dlclose should fail");
  assert_eq!(
    read_errno(),
    7373,
    "closed-handle dlclose failure must preserve errno",
  );

  let message = take_dlerror_message().expect("expected dlerror message after second close");

  assert!(
    message.contains("dynamic-loader handle already closed"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_success_preserves_errno_and_keeps_dlerror_empty() {
  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
    .expect("libc path should not contain interior NUL");

  // SAFETY: `libc_cstr` is a valid NUL-terminated path.
  let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "expected dlopen to open libc: {}",
    libc_path.display(),
  );

  write_errno(7070);

  assert_eq!(dlclose(handle), 0, "dlclose should succeed");
  assert_eq!(read_errno(), 7070, "successful dlclose must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "successful dlclose from clean state must not create dlerror",
  );
}

#[test]
fn dlclose_does_not_invalidate_symbol_pointer_in_refcount_only_phase() {
  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
    .expect("libc path should not contain interior NUL");

  // SAFETY: `libc_cstr` is a valid NUL-terminated path.
  let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "expected dlopen to open libc: {}",
    libc_path.display(),
  );

  let strlen_symbol = unsafe {
    // SAFETY: symbol name pointer is valid and NUL-terminated.
    dlsym(handle, c"strlen".as_ptr())
  };

  assert!(
    !strlen_symbol.is_null(),
    "expected dlsym(handle, strlen) to resolve"
  );

  // SAFETY: `strlen_symbol` comes from resolving libc's `strlen`.
  let strlen_fn = unsafe { core::mem::transmute::<*mut c_void, StrlenFn>(strlen_symbol) };
  // SAFETY: argument is a valid NUL-terminated string.
  let before_close_len = unsafe { strlen_fn(c"rlibc".as_ptr()) };

  assert_eq!(before_close_len, 5);
  assert_eq!(dlclose(handle), 0, "first dlclose should succeed");

  let after_close_len = unsafe {
    // SAFETY: I058 contract is refcount-only close; no eager unmap should occur.
    strlen_fn(c"rlibc".as_ptr())
  };

  assert_eq!(
    after_close_len, 5,
    "symbol pointer should remain callable after final close in this phase",
  );
}

#[test]
fn dlclose_failure_preserves_errno_for_null_and_unknown_handles() {
  clear_pending_dlerror();
  write_errno(9191);

  assert_eq!(dlclose(core::ptr::null_mut()), -1);
  assert_eq!(
    read_errno(),
    9191,
    "null-handle failure must preserve caller errno",
  );

  let null_message = take_dlerror_message().expect("expected dlerror for null handle");

  assert!(
    null_message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {null_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );

  let unknown_handle = 0x0D15_EA5E_usize as *mut c_void;

  write_errno(9292);
  assert_eq!(dlclose(unknown_handle), -1);
  assert_eq!(
    read_errno(),
    9292,
    "unknown-handle failure must preserve caller errno",
  );

  let unknown_message = take_dlerror_message().expect("expected dlerror for unknown handle");

  assert!(
    unknown_message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {unknown_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_failure_replaces_prior_dlsym_dlerror_and_preserves_errno() {
  clear_pending_dlerror();

  // SAFETY: null symbol pointer is intentional to trigger dlsym error-path behavior.
  let resolved = unsafe { dlsym(core::ptr::null_mut(), core::ptr::null()) };

  assert!(resolved.is_null(), "null symbol pointer lookup must fail");

  let unknown_handle = 0xA11CEusize as *mut c_void;

  write_errno(9494);
  assert_eq!(dlclose(unknown_handle), -1);
  assert_eq!(
    read_errno(),
    9494,
    "unknown-handle dlclose failure must preserve caller errno",
  );

  let message = take_dlerror_message()
    .expect("dlclose failure should replace prior pending dlsym error message");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    !message.contains("dlsym symbol pointer is null"),
    "latest dlclose failure must replace prior dlsym error message",
  );
  assert_eq!(
    read_errno(),
    9494,
    "reading dlerror must not mutate errno after dlclose failure",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_unknown_failure_replaces_pending_closed_handle_message() {
  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
    .expect("libc path should not contain interior NUL");
  let unknown_handle = 0xE11Ausize as *mut c_void;

  // SAFETY: `libc_cstr` is a valid NUL-terminated path.
  let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "expected dlopen to open libc: {}",
    libc_path.display(),
  );
  assert_eq!(dlclose(handle), 0, "first dlclose should succeed");
  assert_eq!(dlclose(handle), -1, "second dlclose should fail");

  write_errno(9393);
  assert_eq!(
    dlclose(unknown_handle),
    -1,
    "unknown-handle close should fail"
  );
  assert_eq!(
    read_errno(),
    9393,
    "unknown-handle failure must preserve caller errno",
  );

  let message =
    take_dlerror_message().expect("expected invalid-handle dlerror after unknown-handle close");

  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    !message.contains("already closed"),
    "dlerror should be replaced by the latest close failure: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_closed_failure_replaces_pending_unknown_handle_message() {
  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
    .expect("libc path should not contain interior NUL");
  let unknown_handle = 0xBEEFusize as *mut c_void;

  write_errno(9696);
  assert_eq!(
    dlclose(unknown_handle),
    -1,
    "unknown-handle close should fail"
  );
  assert_eq!(
    read_errno(),
    9696,
    "unknown-handle failure must preserve caller errno",
  );

  // SAFETY: `libc_cstr` is a valid NUL-terminated path.
  let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "expected dlopen to open libc: {}",
    libc_path.display(),
  );
  assert_eq!(dlclose(handle), 0, "first dlclose should succeed");

  write_errno(9797);
  assert_eq!(dlclose(handle), -1, "second dlclose should fail");
  assert_eq!(
    read_errno(),
    9797,
    "closed-handle failure must preserve caller errno",
  );

  let message = take_dlerror_message()
    .expect("closed-handle failure should replace pending unknown-handle message");

  assert!(
    message.contains("already closed"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    !message.contains("invalid dynamic-loader handle"),
    "dlerror should be replaced by latest closed-handle failure: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_pending_unknown_message_isolated_from_child_closed_failure() {
  clear_pending_dlerror();

  let unknown_handle = 0xFEEDusize as *mut c_void;

  write_errno(9898);
  assert_eq!(
    dlclose(unknown_handle),
    -1,
    "unknown-handle close should fail"
  );
  assert_eq!(
    read_errno(),
    9898,
    "main-thread unknown-handle failure must preserve errno",
  );

  let child = thread::spawn(move || {
    clear_pending_dlerror();

    let libc_path = loaded_libc_path().expect("child expected libc path in /proc/self/maps");
    let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
      .expect("child libc path should not contain interior NUL");

    // SAFETY: `libc_cstr` is a valid NUL-terminated path.
    let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

    assert!(
      !handle.is_null(),
      "child expected dlopen to open libc: {}",
      libc_path.display(),
    );
    assert_eq!(dlclose(handle), 0, "child first dlclose should succeed");

    write_errno(9797);
    assert_eq!(dlclose(handle), -1, "child second dlclose should fail");
    assert_eq!(
      read_errno(),
      9797,
      "child closed-handle failure must preserve errno",
    );

    let message = take_dlerror_message().expect("child expected closed-handle dlerror");

    assert!(
      message.contains("already closed"),
      "child unexpected dlerror message: {message}",
    );
    assert!(
      !message.contains("invalid dynamic-loader handle"),
      "child message should come from latest closed-handle failure: {message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "child second dlerror call should clear pending state",
    );
  });

  child.join().expect("child thread panicked");
  assert_eq!(
    read_errno(),
    9898,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending unknown-handle dlerror should remain isolated");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "main unexpected dlerror message: {main_message}",
  );
  assert!(
    !main_message.contains("already closed"),
    "main pending message should not be replaced by child thread failure: {main_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "main second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_pending_closed_message_isolated_from_child_unknown_failure() {
  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
    .expect("libc path should not contain interior NUL");

  // SAFETY: `libc_cstr` is a valid NUL-terminated path.
  let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "expected dlopen to open libc: {}",
    libc_path.display(),
  );
  assert_eq!(dlclose(handle), 0, "first dlclose should succeed");

  write_errno(8888);
  assert_eq!(dlclose(handle), -1, "second dlclose should fail");
  assert_eq!(
    read_errno(),
    8888,
    "main-thread closed-handle failure must preserve errno",
  );

  let child = thread::spawn(|| {
    clear_pending_dlerror();

    let unknown_handle = 0xC0DEusize as *mut c_void;

    write_errno(8787);
    assert_eq!(
      dlclose(unknown_handle),
      -1,
      "child unknown-handle close should fail",
    );
    assert_eq!(
      read_errno(),
      8787,
      "child unknown-handle failure must preserve errno",
    );

    let message = take_dlerror_message().expect("child expected unknown-handle dlerror");

    assert!(
      message.contains("invalid dynamic-loader handle"),
      "child unexpected dlerror message: {message}",
    );
    assert!(
      !message.contains("already closed"),
      "child message should come from latest unknown-handle failure: {message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "child second dlerror call should clear pending state",
    );
  });

  child.join().expect("child thread panicked");
  assert_eq!(
    read_errno(),
    8888,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending closed-handle dlerror should remain isolated");

  assert!(
    main_message.contains("already closed"),
    "main unexpected dlerror message: {main_message}",
  );
  assert!(
    !main_message.contains("invalid dynamic-loader handle"),
    "main pending message should not be replaced by child thread failure: {main_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "main second dlerror call should clear pending state",
  );
}

#[test]
fn dlclose_pending_unknown_message_isolated_from_child_unknown_failure() {
  clear_pending_dlerror();

  let main_unknown_handle = 0xD00Dusize as *mut c_void;

  write_errno(8585);
  assert_eq!(
    dlclose(main_unknown_handle),
    -1,
    "main unknown-handle close should fail",
  );
  assert_eq!(
    read_errno(),
    8585,
    "main-thread unknown-handle failure must preserve errno",
  );

  let child = thread::spawn(|| {
    clear_pending_dlerror();

    let child_unknown_handle = 0xABCDusize as *mut c_void;

    write_errno(8484);
    assert_eq!(
      dlclose(child_unknown_handle),
      -1,
      "child unknown-handle close should fail",
    );
    assert_eq!(
      read_errno(),
      8484,
      "child unknown-handle failure must preserve errno",
    );

    let child_message = take_dlerror_message().expect("child expected unknown-handle dlerror");

    assert!(
      child_message.contains("invalid dynamic-loader handle"),
      "child unexpected dlerror message: {child_message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "child second dlerror call should clear pending state",
    );
    assert_eq!(
      read_errno(),
      8484,
      "child reading/clearing dlerror must preserve errno",
    );
  });

  child.join().expect("child thread panicked");
  assert_eq!(
    read_errno(),
    8585,
    "child thread operations must not mutate main-thread errno",
  );

  let main_message = take_dlerror_message()
    .expect("main-thread pending unknown-handle dlerror should remain isolated");

  assert!(
    main_message.contains("invalid dynamic-loader handle"),
    "main unexpected dlerror message: {main_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "main second dlerror call should clear pending state",
  );
  assert_eq!(
    read_errno(),
    8585,
    "main reading/clearing dlerror must preserve errno",
  );
}

#[test]
fn dlclose_child_unknown_failure_does_not_create_main_pending_error() {
  clear_pending_dlerror();
  write_errno(8383);

  let child = thread::spawn(|| {
    clear_pending_dlerror();

    let unknown_handle = 0xAAAAusize as *mut c_void;

    write_errno(8282);
    assert_eq!(
      dlclose(unknown_handle),
      -1,
      "child unknown-handle close should fail",
    );
    assert_eq!(
      read_errno(),
      8282,
      "child unknown-handle failure must preserve errno",
    );

    let child_message = take_dlerror_message().expect("child expected unknown-handle dlerror");

    assert!(
      child_message.contains("invalid dynamic-loader handle"),
      "child unexpected dlerror message: {child_message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "child second dlerror call should clear pending state",
    );
    assert_eq!(
      read_errno(),
      8282,
      "child reading/clearing dlerror must preserve errno",
    );
  });

  child.join().expect("child thread panicked");
  assert_eq!(
    read_errno(),
    8383,
    "child thread operations must not mutate main-thread errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "child dlclose failure must not create main-thread pending dlerror",
  );
  assert_eq!(
    read_errno(),
    8383,
    "main reading empty dlerror must preserve errno",
  );
}

#[test]
fn dlclose_child_closed_failure_does_not_create_main_pending_error() {
  clear_pending_dlerror();
  write_errno(8181);

  let child = thread::spawn(|| {
    clear_pending_dlerror();

    let libc_path = loaded_libc_path().expect("child expected libc path in /proc/self/maps");
    let libc_cstr = CString::new(libc_path.to_string_lossy().as_bytes())
      .expect("child libc path should not contain interior NUL");

    // SAFETY: `libc_cstr` is a valid NUL-terminated path.
    let handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

    assert!(
      !handle.is_null(),
      "child expected dlopen to open libc: {}",
      libc_path.display(),
    );
    assert_eq!(dlclose(handle), 0, "child first dlclose should succeed");

    write_errno(8080);
    assert_eq!(dlclose(handle), -1, "child second dlclose should fail");
    assert_eq!(
      read_errno(),
      8080,
      "child closed-handle failure must preserve errno",
    );

    let child_message = take_dlerror_message().expect("child expected closed-handle dlerror");

    assert!(
      child_message.contains("already closed"),
      "child unexpected dlerror message: {child_message}",
    );
    assert!(
      take_dlerror_message().is_none(),
      "child second dlerror call should clear pending state",
    );
    assert_eq!(
      read_errno(),
      8080,
      "child reading/clearing dlerror must preserve errno",
    );
  });

  child.join().expect("child thread panicked");
  assert_eq!(
    read_errno(),
    8181,
    "child thread operations must not mutate main-thread errno",
  );
  assert!(
    take_dlerror_message().is_none(),
    "child closed-handle dlclose failure must not create main-thread pending dlerror",
  );
  assert_eq!(
    read_errno(),
    8181,
    "main reading empty dlerror must preserve errno",
  );
}
