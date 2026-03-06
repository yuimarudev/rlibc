#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use rlibc::abi::errno::EINVAL;
use rlibc::abi::types::{c_char, c_int, size_t};
use rlibc::errno::__errno_location;
use rlibc::stdio::{FILE, fileno, fileno_unlocked, fopen, fputs, fread, tmpfile};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, OnceLock};

unsafe extern "C" {
  #[link_name = "fileno"]
  fn host_fileno(stream: *mut FILE) -> c_int;
  fn fclose(stream: *mut FILE) -> c_int;
  fn rewind(stream: *mut FILE);
}

fn test_lock() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  match LOCK.get_or_init(|| Mutex::new(())).lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
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

fn as_size_t(value: usize) -> size_t {
  size_t::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize must fit into size_t on this target"))
}

#[test]
fn fileno_unlocked_null_stream_returns_minus_one_and_einval() {
  let _guard = test_lock();

  write_errno(0);

  // SAFETY: null pointer is intentional to validate error handling.
  let descriptor = unsafe { fileno_unlocked(core::ptr::null_mut()) };

  assert_eq!(descriptor, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn tmpfile_and_host_backed_file_io_wrappers_round_trip_and_preserve_errno() {
  let _guard = test_lock();

  write_errno(6100);

  // SAFETY: `tmpfile` creates a host-managed stream or returns null.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null(), "tmpfile must return a valid host FILE*");
  assert_eq!(read_errno(), 6100, "successful tmpfile must preserve errno");

  // SAFETY: `stream` came from `tmpfile` and remains open here.
  let expected_descriptor = unsafe { host_fileno(stream) };

  assert!(
    expected_descriptor >= 0,
    "host fileno(tmpfile()) must expose a valid descriptor",
  );

  write_errno(6101);

  // SAFETY: `stream` is a valid `FILE*` returned from `tmpfile`.
  let descriptor = unsafe { fileno(stream) };

  assert_eq!(descriptor, expected_descriptor);
  assert_eq!(
    read_errno(),
    6101,
    "successful fileno(tmpfile()) must preserve errno"
  );

  write_errno(6102);

  // SAFETY: `stream` is a valid `FILE*` returned from `tmpfile`.
  let unlocked_descriptor = unsafe { fileno_unlocked(stream) };

  assert_eq!(unlocked_descriptor, expected_descriptor);
  assert_eq!(
    read_errno(),
    6102,
    "successful fileno_unlocked(tmpfile()) must preserve errno",
  );

  let payload = CString::new("rlibc tmpfile payload").unwrap_or_else(|_| unreachable!());

  write_errno(6103);

  // SAFETY: payload and stream are valid for `fputs`.
  let write_status = unsafe { fputs(payload.as_ptr(), stream) };

  assert!(write_status >= 0, "fputs(tmpfile()) must succeed");
  assert_eq!(read_errno(), 6103, "successful fputs must preserve errno");

  // SAFETY: rewinds the still-open host stream before reading it back.
  unsafe { rewind(stream) };

  let mut readback = [0_u8; 64];

  write_errno(6104);

  // SAFETY: buffer and stream are valid for `fread`.
  let elements_read = unsafe {
    fread(
      readback.as_mut_ptr().cast(),
      1,
      as_size_t(readback.len()),
      stream,
    )
  };

  assert!(
    elements_read >= as_size_t(payload.as_bytes().len()),
    "fread(tmpfile()) must read back the payload",
  );
  assert_eq!(read_errno(), 6104, "successful fread must preserve errno");
  assert_eq!(
    &readback[..payload.as_bytes().len()],
    payload.as_bytes(),
    "fread(tmpfile()) must return the bytes written by fputs",
  );

  // SAFETY: `stream` came from `tmpfile` and remains open here.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0, "tmpfile stream must be closable");
}

#[test]
fn fopen_reads_proc_self_exe_and_preserves_errno_on_success() {
  let _guard = test_lock();
  let path = CString::new("/proc/self/exe").unwrap_or_else(|_| unreachable!());
  let mode = CString::new("rb").unwrap_or_else(|_| unreachable!());
  let mut readback = [0_u8; 16];

  write_errno(6200);

  // SAFETY: path and mode are valid NUL-terminated strings.
  let stream = unsafe {
    fopen(
      path.as_ptr().cast::<c_char>(),
      mode.as_ptr().cast::<c_char>(),
    )
  };

  assert!(!stream.is_null(), "fopen(/proc/self/exe, rb) must succeed");
  assert_eq!(read_errno(), 6200, "successful fopen must preserve errno");

  write_errno(6201);

  // SAFETY: buffer and stream are valid for `fread`.
  let elements_read = unsafe {
    fread(
      readback.as_mut_ptr().cast(),
      1,
      as_size_t(readback.len()),
      stream,
    )
  };

  assert!(
    elements_read > 0,
    "fread(fopen(/proc/self/exe)) must read bytes"
  );
  assert_eq!(read_errno(), 6201, "successful fread must preserve errno");

  // SAFETY: `stream` came from `fopen` and remains open here.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0, "fopen stream must be closable");
}
