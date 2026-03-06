#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_void;
use core::ptr;
use rlibc::abi::errno::EINVAL;
use rlibc::abi::types::{c_int, size_t};
use rlibc::errno::__errno_location;
use rlibc::stdio::{FILE, fclose, fileno, fopen, fputs, fread, tmpfile};
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::{env, fs, process};

unsafe extern "C" {
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

fn unique_temp_path(label: &str) -> PathBuf {
  static NEXT_ID: AtomicU64 = AtomicU64::new(1);
  let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

  env::temp_dir().join(format!(
    "rlibc-stdio-host-io-{label}-{}-{id}.tmp",
    process::id()
  ))
}

#[test]
fn tmpfile_round_trips_with_fputs_and_fread() {
  let _guard = test_lock();
  let payload = CString::new("rlibc tmpfile payload").unwrap_or_else(|_| unreachable!());

  write_errno(5100);

  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile should return a valid host FILE*"
  );
  assert_eq!(read_errno(), 5100, "successful tmpfile must preserve errno");
  assert!(
    unsafe { fileno(stream) } >= 0,
    "tmpfile result should expose a readable descriptor"
  );

  write_errno(5101);

  let write_status = unsafe { fputs(payload.as_ptr(), stream) };

  assert!(
    write_status >= 0,
    "fputs should succeed for a tmpfile stream"
  );
  assert_eq!(read_errno(), 5101, "successful fputs must preserve errno");

  unsafe { rewind(stream) };

  let mut buffer = [0_u8; 64];
  let expected_len = payload.as_bytes().len();

  write_errno(5102);

  let items_read = unsafe {
    fread(
      buffer.as_mut_ptr().cast::<c_void>(),
      1,
      expected_len as size_t,
      stream,
    )
  };

  assert_eq!(
    items_read, expected_len as size_t,
    "fread should recover the exact tmpfile payload",
  );
  assert_eq!(read_errno(), 5102, "successful fread must preserve errno");
  assert_eq!(&buffer[..expected_len], payload.as_bytes());

  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0, "tmpfile stream must be closable");
}

#[test]
fn fopen_reads_existing_file_with_fread() {
  let _guard = test_lock();
  let path = unique_temp_path("fopen-read");
  let bytes = b"rlibc fopen data";
  let path_cstring = CString::new(path.as_os_str().as_encoded_bytes()).unwrap_or_else(|_| {
    panic!(
      "temporary path should not contain interior NUL bytes: {}",
      path.display()
    )
  });
  let mode = CString::new("rb").unwrap_or_else(|_| unreachable!());

  fs::write(&path, bytes)
    .unwrap_or_else(|error| panic!("failed to seed temporary file {}: {error}", path.display()));

  write_errno(5200);

  let stream = unsafe { fopen(path_cstring.as_ptr(), mode.as_ptr()) };

  assert!(
    !stream.is_null(),
    "fopen should open a seeded temporary file"
  );
  assert_eq!(read_errno(), 5200, "successful fopen must preserve errno");

  let mut buffer = [0_u8; 64];

  write_errno(5201);

  let items_read = unsafe {
    fread(
      buffer.as_mut_ptr().cast::<c_void>(),
      1,
      bytes.len() as size_t,
      stream,
    )
  };

  assert_eq!(items_read, bytes.len() as size_t);
  assert_eq!(read_errno(), 5201, "successful fread must preserve errno");
  assert_eq!(&buffer[..bytes.len()], bytes);

  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0, "opened file stream must be closable");

  fs::remove_file(&path).unwrap_or_else(|error| {
    panic!(
      "failed to remove temporary file after fopen test {}: {error}",
      path.display()
    )
  });
}

#[test]
fn fopen_null_arguments_return_null_and_einval() {
  let _guard = test_lock();
  let mode = CString::new("rb").unwrap_or_else(|_| unreachable!());
  let path = CString::new("/tmp").unwrap_or_else(|_| unreachable!());

  write_errno(0);

  let null_path = unsafe { fopen(ptr::null(), mode.as_ptr()) };

  assert!(null_path.is_null());
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  let null_mode = unsafe { fopen(path.as_ptr(), ptr::null()) };

  assert!(null_mode.is_null());
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fputs_null_stream_returns_eof_and_einval() {
  let _guard = test_lock();
  let payload = CString::new("rlibc").unwrap_or_else(|_| unreachable!());

  write_errno(0);

  let status = unsafe { fputs(payload.as_ptr(), ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fread_null_stream_returns_zero_and_einval() {
  let _guard = test_lock();
  let mut buffer = [0_u8; 8];

  write_errno(0);

  let items_read = unsafe {
    fread(
      buffer.as_mut_ptr().cast::<c_void>(),
      1,
      buffer.len() as size_t,
      ptr::null_mut(),
    )
  };

  assert_eq!(items_read, 0);
  assert_eq!(read_errno(), EINVAL);
}
