#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::{c_char, c_int, c_uint, c_void};
use rlibc::abi::errno::{EAGAIN, EBADF, EFAULT, ENOENT, ENOTDIR, ENOTSOCK, EPIPE};
use rlibc::abi::types::{size_t, ssize_t};
use rlibc::errno::__errno_location;
use rlibc::unistd::{
  AT_FDCWD, MSG_DONTWAIT, MSG_NOSIGNAL, MSG_PEEK, MSG_WAITALL, open, openat, read, recv, send,
  write,
};
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const O_RDONLY: c_int = 0;

unsafe extern "C" {
  fn close(fd: c_int) -> c_int;
}

fn sz(len: usize) -> size_t {
  size_t::try_from(len)
    .unwrap_or_else(|_| unreachable!("usize does not fit into size_t on this target"))
}

fn errno_value() -> c_int {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe { __errno_location().read() }
}

fn set_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe { __errno_location().write(value) };
}

fn unique_temp_path(prefix: &str) -> PathBuf {
  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("system time before unix epoch")
    .as_nanos();

  std::env::temp_dir().join(format!("rlibc-{prefix}-{}-{timestamp}", std::process::id()))
}

fn close_fd(fd: c_int) {
  // SAFETY: `fd` is a file descriptor returned by open/openat in this test.
  let result = unsafe { close(fd) };

  assert_eq!(result, 0, "close({fd}) failed");
}

#[test]
fn open_existing_path_returns_fd_reads_bytes_and_keeps_errno() {
  let file_path = unique_temp_path("open-existing");
  let expected = b"rlibc-i019-open";

  fs::write(&file_path, expected).expect("failed to create temp file for open test");

  let path_cstr =
    CString::new(file_path.as_os_str().as_encoded_bytes()).expect("path must not contain NUL");

  set_errno(1234);

  // SAFETY: `path_cstr` points to a valid NUL-terminated path string.
  let fd = unsafe {
    open(
      path_cstr.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert!(fd >= 0, "open failed with errno={}", errno_value());
  assert_eq!(errno_value(), 1234);

  let mut received = [0_u8; 32];
  // SAFETY: `received` is writable for `received.len()` bytes.
  let read_len = unsafe {
    read(
      fd,
      received.as_mut_ptr().cast::<c_void>(),
      sz(received.len()),
    )
  };

  assert_eq!(
    read_len,
    ssize_t::try_from(expected.len())
      .unwrap_or_else(|_| unreachable!("expected length must fit ssize_t")),
  );
  assert_eq!(&received[..expected.len()], expected);

  close_fd(fd);
  fs::remove_file(file_path).expect("failed to remove temp file for open test");
}

#[test]
fn open_missing_path_returns_minus_one_and_errno_enoent() {
  let missing_path = unique_temp_path("open-missing");
  let missing_cstr = CString::new(missing_path.as_os_str().as_encoded_bytes())
    .expect("missing path must not contain NUL");

  set_errno(0);

  // SAFETY: `missing_cstr` points to a valid NUL-terminated path string.
  let fd = unsafe {
    open(
      missing_cstr.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn open_null_path_returns_minus_one_and_errno_efault() {
  set_errno(0);

  // SAFETY: null path pointer is intentional to validate errno propagation.
  let fd = unsafe { open(core::ptr::null(), O_RDONLY, c_uint::from(0_u8)) };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), EFAULT);
}

#[test]
fn open_empty_path_returns_minus_one_and_errno_enoent() {
  let empty_path = CString::new("").expect("empty path must not contain interior NUL");

  set_errno(0);

  // SAFETY: `empty_path` points to a valid NUL-terminated path string.
  let fd = unsafe {
    open(
      empty_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn read_invalid_fd_returns_minus_one_and_errno_ebadf() {
  let mut buffer = [0_u8; 8];

  set_errno(0);

  // SAFETY: `buffer` is writable for `buffer.len()` bytes.
  let result = unsafe { read(-1, buffer.as_mut_ptr().cast::<c_void>(), sz(buffer.len())) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn read_invalid_fd_with_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let result = unsafe { read(-1, core::ptr::null_mut(), sz(1)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn read_invalid_fd_with_huge_count_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let result = unsafe { read(-1, byte.as_mut_ptr().cast::<c_void>(), size_t::MAX) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn read_invalid_fd_with_huge_count_and_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let result = unsafe { read(-1, core::ptr::null_mut(), size_t::MAX) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn read_invalid_fd_with_zero_count_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let result = unsafe { read(-1, byte.as_mut_ptr().cast::<c_void>(), sz(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn read_invalid_fd_with_zero_count_and_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let result = unsafe { read(-1, core::ptr::null_mut(), sz(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn read_write_only_fd_returns_minus_one_and_errno_ebadf() {
  let file_path = unique_temp_path("read-write-only-fd");
  let mut byte = [0_u8; 1];

  fs::write(&file_path, b"seed").expect("failed to create temp file for write-only read test");

  let file = File::options()
    .write(true)
    .open(&file_path)
    .expect("failed to open temp file as write-only descriptor");

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally opened write-only.
  let result = unsafe { read(file.as_raw_fd(), byte.as_mut_ptr().cast::<c_void>(), sz(1)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove temp file for write-only read test");
}

#[test]
fn read_write_only_fd_with_zero_count_returns_minus_one_and_errno_ebadf() {
  let file_path = unique_temp_path("read-write-only-fd-zero-count");
  let mut byte = [0xA7_u8; 1];

  fs::write(&file_path, b"seed")
    .expect("failed to create temp file for write-only zero-count read test");

  let file = File::options()
    .write(true)
    .open(&file_path)
    .expect("failed to open temp file as write-only descriptor");

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally opened write-only.
  let result = unsafe { read(file.as_raw_fd(), byte.as_mut_ptr().cast::<c_void>(), sz(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0xA7_u8]);

  drop(file);
  fs::remove_file(file_path)
    .expect("failed to remove temp file for write-only zero-count read test");
}

#[test]
fn read_write_only_fd_with_huge_count_returns_minus_one_and_errno_ebadf() {
  let file_path = unique_temp_path("read-write-only-fd-huge-count");
  let mut byte = [0xA8_u8; 1];

  fs::write(&file_path, b"seed")
    .expect("failed to create temp file for write-only huge-count read test");

  let file = File::options()
    .write(true)
    .open(&file_path)
    .expect("failed to open temp file as write-only descriptor");

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally opened write-only.
  let result = unsafe {
    read(
      file.as_raw_fd(),
      byte.as_mut_ptr().cast::<c_void>(),
      size_t::MAX,
    )
  };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0xA8_u8]);

  drop(file);
  fs::remove_file(file_path)
    .expect("failed to remove temp file for write-only huge-count read test");
}

#[test]
fn read_write_only_fd_failure_does_not_modify_file_contents() {
  let file_path = unique_temp_path("read-write-only-fd-no-mutate");
  let original = b"seed-data";
  let mut byte = [0xA9_u8; 1];

  fs::write(&file_path, original)
    .expect("failed to create temp file for write-only read no-mutate test");

  let file = File::options()
    .write(true)
    .open(&file_path)
    .expect("failed to open temp file as write-only descriptor");

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally opened write-only.
  let result = unsafe { read(file.as_raw_fd(), byte.as_mut_ptr().cast::<c_void>(), sz(1)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0xA9_u8]);

  drop(file);

  let current =
    fs::read(&file_path).expect("failed to read file contents after write-only read failure");

  assert_eq!(current, original);

  fs::remove_file(file_path)
    .expect("failed to remove temp file for write-only read no-mutate test");
}

#[test]
fn read_write_only_fd_zero_count_failure_does_not_modify_file_contents() {
  let file_path = unique_temp_path("read-write-only-fd-zero-count-no-mutate");
  let original = b"seed-data";
  let mut byte = [0xAA_u8; 1];

  fs::write(&file_path, original)
    .expect("failed to create temp file for write-only zero-count read no-mutate test");

  let file = File::options()
    .write(true)
    .open(&file_path)
    .expect("failed to open temp file as write-only descriptor");

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally opened write-only.
  let result = unsafe { read(file.as_raw_fd(), byte.as_mut_ptr().cast::<c_void>(), sz(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0xAA_u8]);

  drop(file);

  let current = fs::read(&file_path)
    .expect("failed to read file contents after write-only zero-count read failure");

  assert_eq!(current, original);

  fs::remove_file(file_path)
    .expect("failed to remove temp file for write-only zero-count read no-mutate test");
}

#[test]
fn read_write_only_fd_huge_count_failure_does_not_modify_file_contents() {
  let file_path = unique_temp_path("read-write-only-fd-huge-count-no-mutate");
  let original = b"seed-data";
  let mut byte = [0xAB_u8; 1];

  fs::write(&file_path, original)
    .expect("failed to create temp file for write-only huge-count read no-mutate test");

  let file = File::options()
    .write(true)
    .open(&file_path)
    .expect("failed to open temp file as write-only descriptor");

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally opened write-only.
  let result = unsafe {
    read(
      file.as_raw_fd(),
      byte.as_mut_ptr().cast::<c_void>(),
      size_t::MAX,
    )
  };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0xAB_u8]);

  drop(file);

  let current = fs::read(&file_path)
    .expect("failed to read file contents after write-only huge-count read failure");

  assert_eq!(current, original);

  fs::remove_file(file_path)
    .expect("failed to remove temp file for write-only huge-count read no-mutate test");
}

#[test]
fn write_invalid_fd_returns_minus_one_and_errno_ebadf() {
  let payload = b"invalid-fd";

  set_errno(0);

  // SAFETY: buffer is valid; fd is intentionally invalid.
  let result = unsafe { write(-1, payload.as_ptr().cast::<c_void>(), sz(payload.len())) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn write_null_buffer_returns_minus_one_and_errno_efault() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for write null-buffer test");

  set_errno(0);

  // SAFETY: null pointer is intentional to validate errno propagation.
  let result = unsafe { write(writer.as_raw_fd(), core::ptr::null(), sz(1)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EFAULT);
}

#[test]
fn write_zero_count_with_null_buffer_returns_zero_and_keeps_errno() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for zero-count write null-buffer test");

  set_errno(1357);

  // SAFETY: pointer may be null because `count == 0`, so no bytes are read.
  let result = unsafe { write(writer.as_raw_fd(), core::ptr::null(), sz(0)) };

  assert_eq!(result, 0);
  assert_eq!(errno_value(), 1357);
}

#[test]
fn write_zero_count_with_null_buffer_does_not_enqueue_bytes() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for zero-count write queue test");
  let mut byte = [0_u8; 1];

  set_errno(4111);

  // SAFETY: pointer may be null because `count == 0`, so no bytes are read.
  let written = unsafe { write(writer.as_raw_fd(), core::ptr::null(), sz(0)) };

  assert_eq!(written, 0);
  assert_eq!(errno_value(), 4111);

  set_errno(0);

  // SAFETY: `byte` is writable and `MSG_DONTWAIT` avoids blocking when queue is empty.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      byte.as_mut_ptr().cast::<c_void>(),
      sz(1),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EAGAIN);
}

#[test]
fn write_invalid_fd_with_huge_count_returns_minus_one_and_errno_ebadf() {
  let byte = [0_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is readable and fd is intentionally invalid.
  let result = unsafe { write(-1, byte.as_ptr().cast::<c_void>(), size_t::MAX) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn write_invalid_fd_with_huge_count_and_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let result = unsafe { write(-1, core::ptr::null(), size_t::MAX) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn write_invalid_fd_with_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let result = unsafe { write(-1, core::ptr::null(), sz(1)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn write_invalid_fd_with_zero_count_returns_minus_one_and_errno_ebadf() {
  let byte = [0_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is readable and fd is intentionally invalid.
  let result = unsafe { write(-1, byte.as_ptr().cast::<c_void>(), sz(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn write_invalid_fd_with_zero_count_and_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let result = unsafe { write(-1, core::ptr::null(), sz(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn write_read_only_fd_returns_minus_one_and_errno_ebadf() {
  let file_path = unique_temp_path("write-read-only-fd");
  let payload = [0x7A_u8];

  fs::write(&file_path, b"seed").expect("failed to create temp file for read-only write test");

  let file = File::open(&file_path).expect("failed to open temp file as read-only descriptor");

  set_errno(0);

  // SAFETY: `payload` is readable and fd is intentionally opened read-only.
  let result = unsafe { write(file.as_raw_fd(), payload.as_ptr().cast::<c_void>(), sz(1)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove temp file for read-only write test");
}

#[test]
fn write_read_only_fd_with_zero_count_returns_minus_one_and_errno_ebadf() {
  let file_path = unique_temp_path("write-read-only-fd-zero-count");
  let payload = [0x7B_u8; 1];

  fs::write(&file_path, b"seed")
    .expect("failed to create temp file for read-only zero-count write test");

  let file = File::open(&file_path).expect("failed to open temp file as read-only descriptor");

  set_errno(0);

  // SAFETY: `payload` is readable and fd is intentionally opened read-only.
  let result = unsafe { write(file.as_raw_fd(), payload.as_ptr().cast::<c_void>(), sz(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(payload, [0x7B_u8; 1]);

  drop(file);
  fs::remove_file(file_path)
    .expect("failed to remove temp file for read-only zero-count write test");
}

#[test]
fn write_read_only_fd_with_huge_count_returns_minus_one_and_errno_ebadf() {
  let file_path = unique_temp_path("write-read-only-fd-huge-count");
  let payload = [0x7C_u8; 1];

  fs::write(&file_path, b"seed")
    .expect("failed to create temp file for read-only huge-count write test");

  let file = File::open(&file_path).expect("failed to open temp file as read-only descriptor");

  set_errno(0);

  // SAFETY: `payload` is readable and fd is intentionally opened read-only.
  let result = unsafe {
    write(
      file.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      size_t::MAX,
    )
  };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(payload, [0x7C_u8; 1]);

  drop(file);
  fs::remove_file(file_path)
    .expect("failed to remove temp file for read-only huge-count write test");
}

#[test]
fn write_read_only_fd_failure_does_not_modify_file_contents() {
  let file_path = unique_temp_path("write-read-only-fd-no-mutate");
  let original = b"seed-data";
  let payload = [0x55_u8; 1];

  fs::write(&file_path, original)
    .expect("failed to create temp file for read-only write no-mutate test");

  let file = File::open(&file_path).expect("failed to open temp file as read-only descriptor");

  set_errno(0);

  // SAFETY: `payload` is readable and fd is intentionally opened read-only.
  let result = unsafe { write(file.as_raw_fd(), payload.as_ptr().cast::<c_void>(), sz(1)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);

  drop(file);

  let current =
    fs::read(&file_path).expect("failed to read file contents after read-only write failure");

  assert_eq!(current, original);

  fs::remove_file(file_path)
    .expect("failed to remove temp file for read-only write no-mutate test");
}

#[test]
fn write_read_only_fd_zero_count_failure_does_not_modify_file_contents() {
  let file_path = unique_temp_path("write-read-only-fd-zero-count-no-mutate");
  let original = b"seed-data";
  let payload = [0x56_u8; 1];

  fs::write(&file_path, original)
    .expect("failed to create temp file for read-only zero-count write no-mutate test");

  let file = File::open(&file_path).expect("failed to open temp file as read-only descriptor");

  set_errno(0);

  // SAFETY: `payload` is readable and fd is intentionally opened read-only.
  let result = unsafe { write(file.as_raw_fd(), payload.as_ptr().cast::<c_void>(), sz(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(payload, [0x56_u8; 1]);

  drop(file);

  let current = fs::read(&file_path)
    .expect("failed to read file contents after read-only zero-count write failure");

  assert_eq!(current, original);

  fs::remove_file(file_path)
    .expect("failed to remove temp file for read-only zero-count write no-mutate test");
}

#[test]
fn write_read_only_fd_huge_count_failure_does_not_modify_file_contents() {
  let file_path = unique_temp_path("write-read-only-fd-huge-count-no-mutate");
  let original = b"seed-data";
  let payload = [0x57_u8; 1];

  fs::write(&file_path, original)
    .expect("failed to create temp file for read-only huge-count write no-mutate test");

  let file = File::open(&file_path).expect("failed to open temp file as read-only descriptor");

  set_errno(0);

  // SAFETY: `payload` is readable and fd is intentionally opened read-only.
  let result = unsafe {
    write(
      file.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      size_t::MAX,
    )
  };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(payload, [0x57_u8; 1]);

  drop(file);

  let current = fs::read(&file_path)
    .expect("failed to read file contents after read-only huge-count write failure");

  assert_eq!(current, original);

  fs::remove_file(file_path)
    .expect("failed to remove temp file for read-only huge-count write no-mutate test");
}

#[test]
fn write_sends_bytes_to_unix_stream_peer() {
  let payload = b"rlibc-i019-write";
  let (mut reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for write test");

  // SAFETY: `payload` is readable for `payload.len()` bytes for this call.
  let written = unsafe {
    write(
      writer.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
    )
  };

  assert_eq!(
    written,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );

  let mut received = [0_u8; 64];
  let received_len = reader
    .read(&mut received)
    .expect("failed to read payload from unix stream peer");

  assert_eq!(&received[..received_len], payload);
}

#[test]
fn read_null_buffer_returns_minus_one_and_errno_efault() {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for read null-buffer test");

  writer
    .write_all(b"x")
    .expect("failed to seed stream before null-buffer read");

  set_errno(0);

  // SAFETY: null buffer pointer is intentional to validate errno propagation.
  let read_len = unsafe { read(reader.as_raw_fd(), core::ptr::null_mut(), sz(1)) };

  assert_eq!(read_len, -1);
  assert_eq!(errno_value(), EFAULT);
}

#[test]
fn read_zero_count_returns_zero_and_keeps_errno() {
  let (reader, _writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for read zero-count test");
  let mut scratch = [0xAB_u8];

  set_errno(4321);

  // SAFETY: pointer is valid and `count == 0`, so no bytes need to be written.
  let read_len = unsafe {
    read(
      reader.as_raw_fd(),
      scratch.as_mut_ptr().cast::<c_void>(),
      sz(0),
    )
  };

  assert_eq!(read_len, 0);
  assert_eq!(errno_value(), 4321);
  assert_eq!(scratch, [0xAB_u8]);
}

#[test]
fn read_zero_count_does_not_consume_queued_bytes_and_keeps_errno() {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for read zero-count queue test");
  let mut scratch = [0xCC_u8];
  let mut consumed = [0_u8; 1];

  writer
    .write_all(b"r")
    .expect("failed to seed stream before zero-count read");

  set_errno(4333);

  // SAFETY: pointer is valid and `count == 0`, so no bytes are written.
  let read_len = unsafe {
    read(
      reader.as_raw_fd(),
      scratch.as_mut_ptr().cast::<c_void>(),
      sz(0),
    )
  };

  assert_eq!(read_len, 0);
  assert_eq!(errno_value(), 4333);
  assert_eq!(scratch, [0xCC_u8]);

  // SAFETY: `consumed` is writable for one byte and one byte is queued.
  let consume_len = unsafe {
    read(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(consumed.len()),
    )
  };

  assert_eq!(consume_len, 1);
  assert_eq!(consumed, [b'r']);
  assert_eq!(errno_value(), 4333);
}

#[test]
fn read_zero_count_with_null_buffer_returns_zero_and_keeps_errno() {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for read zero-count null-buffer test");
  let mut byte = [0_u8; 1];

  writer
    .write_all(b"q")
    .expect("failed to seed stream before zero-count null-buffer read");

  set_errno(2469);

  // SAFETY: pointer may be null because `count == 0`, so no bytes are written.
  let read_len = unsafe { read(reader.as_raw_fd(), core::ptr::null_mut(), sz(0)) };

  assert_eq!(read_len, 0);
  assert_eq!(errno_value(), 2469);

  // SAFETY: `byte` is writable for one byte and the stream has one queued byte.
  let followup_len = unsafe {
    read(
      reader.as_raw_fd(),
      byte.as_mut_ptr().cast::<c_void>(),
      sz(1),
    )
  };

  assert_eq!(followup_len, 1);
  assert_eq!(byte, [b'q']);
  assert_eq!(errno_value(), 2469);
}

#[test]
fn write_zero_count_returns_zero_and_keeps_errno() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for write zero-count test");
  let scratch = [0xCD_u8];

  set_errno(2468);

  // SAFETY: pointer is valid and `count == 0`, so no bytes are read from `scratch`.
  let written = unsafe { write(writer.as_raw_fd(), scratch.as_ptr().cast::<c_void>(), sz(0)) };

  assert_eq!(written, 0);
  assert_eq!(errno_value(), 2468);
}

#[test]
fn write_zero_count_with_non_null_buffer_does_not_enqueue_bytes() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for zero-count non-null write queue test");
  let payload = [0x5A_u8];
  let mut received = [0_u8; 1];

  set_errno(4122);

  // SAFETY: pointer is valid and `count == 0`, so no bytes are read from `payload`.
  let written = unsafe { write(writer.as_raw_fd(), payload.as_ptr().cast::<c_void>(), sz(0)) };

  assert_eq!(written, 0);
  assert_eq!(errno_value(), 4122);

  set_errno(0);

  // SAFETY: buffer is writable and `MSG_DONTWAIT` prevents blocking on empty queue.
  let recv_len = unsafe {
    recv(
      reader.as_raw_fd(),
      received.as_mut_ptr().cast::<c_void>(),
      sz(received.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(recv_len, -1);
  assert_eq!(errno_value(), EAGAIN);
}

#[test]
fn send_sends_bytes_to_unix_stream_peer() {
  let payload = b"rlibc-i051-send";
  let (mut reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send test");

  set_errno(9090);

  // SAFETY: `payload` is readable for `payload.len()` bytes for this call.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      0,
    )
  };

  assert_eq!(
    sent,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 9090);

  let mut received = [0_u8; 64];
  let received_len = reader
    .read(&mut received)
    .expect("failed to read payload from unix stream peer");

  assert_eq!(&received[..received_len], payload);
}

#[test]
fn send_nosignal_success_keeps_errno_and_delivers_payload() {
  let payload = b"rlibc-i051-send-nosignal";
  let (mut reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send nosignal success test");

  set_errno(9091);

  // SAFETY: `payload` is readable and socket descriptor is valid.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_NOSIGNAL,
    )
  };

  assert_eq!(
    sent,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 9091);

  let mut received = [0_u8; 64];
  let received_len = reader
    .read(&mut received)
    .expect("failed to read payload from unix stream peer");

  assert_eq!(&received[..received_len], payload);
}

#[test]
fn send_dontwait_success_keeps_errno_and_delivers_payload() {
  let payload = b"rlibc-i051-send-dontwait";
  let (mut reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send dontwait success test");

  set_errno(9092);

  // SAFETY: `payload` is readable and socket descriptor is valid.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(
    sent,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 9092);

  let mut received = [0_u8; 64];
  let received_len = reader
    .read(&mut received)
    .expect("failed to read payload from unix stream peer");

  assert_eq!(&received[..received_len], payload);
}

#[test]
fn send_nosignal_with_dontwait_success_keeps_errno_and_delivers_payload() {
  let payload = b"rlibc-i051-send-nosignal-dontwait";
  let (mut reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send nosignal+dontwait success test");

  set_errno(9093);

  // SAFETY: `payload` is readable and socket descriptor is valid.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_NOSIGNAL | MSG_DONTWAIT,
    )
  };

  assert_eq!(
    sent,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 9093);

  let mut received = [0_u8; 64];
  let received_len = reader
    .read(&mut received)
    .expect("failed to read payload from unix stream peer");

  assert_eq!(&received[..received_len], payload);
}

#[test]
fn recv_reads_bytes_sent_by_peer_socket() {
  let payload = b"rlibc-i051-recv";
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for recv test");

  writer
    .write_all(payload)
    .expect("failed to seed payload for recv test");

  let mut received = [0_u8; 64];

  set_errno(8181);

  // SAFETY: `received` is writable for `received.len()` bytes for this call.
  let recv_len = unsafe {
    recv(
      reader.as_raw_fd(),
      received.as_mut_ptr().cast::<c_void>(),
      sz(received.len()),
      0,
    )
  };

  assert_eq!(
    recv_len,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 8181);
  assert_eq!(&received[..payload.len()], payload);
}

#[test]
fn send_and_recv_zero_length_return_zero_and_keep_errno() {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for zero-length send/recv tests");
  let send_probe = [0xA5_u8];
  let mut recv_probe = [0x5A_u8];

  set_errno(7001);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are read.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      send_probe.as_ptr().cast::<c_void>(),
      sz(0),
      0,
    )
  };

  assert_eq!(sent, 0);
  assert_eq!(errno_value(), 7001);

  writer
    .write_all(b"z")
    .expect("failed to seed stream before zero-length recv");

  set_errno(7002);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are written.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      recv_probe.as_mut_ptr().cast::<c_void>(),
      sz(0),
      0,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 7002);
  assert_eq!(recv_probe, [0x5A_u8]);

  let mut consumed = [0_u8; 1];
  // SAFETY: buffer is writable for one byte and the socket has one queued byte.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(consumed.len()),
      0,
    )
  };

  assert_eq!(consume_len, 1);
  assert_eq!(consumed, *b"z");
}

#[test]
fn recv_dontwait_zero_length_returns_zero_and_keeps_errno_without_consuming_queue() {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for recv dontwait zero-length test");
  let payload = b"xy";
  let mut recv_probe = [0x5A_u8];

  writer
    .write_all(payload)
    .expect("failed to seed payload before zero-length recv with dontwait");

  set_errno(7003);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are written.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      recv_probe.as_mut_ptr().cast::<c_void>(),
      sz(0),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 7003);
  assert_eq!(recv_probe, [0x5A_u8]);

  let mut consumed = [0_u8; 2];
  // SAFETY: buffer is writable for two bytes and the socket has two queued bytes.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(consumed.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(consume_len, 2);
  assert_eq!(consumed, *payload);
}

#[test]
fn send_nosignal_with_dontwait_zero_length_returns_zero_and_keeps_errno_without_enqueuing() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send nosignal+dontwait zero-length test");
  let send_probe = [0xA6_u8];
  let mut recv_probe = [0_u8; 1];

  set_errno(7004);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are read.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      send_probe.as_ptr().cast::<c_void>(),
      sz(0),
      MSG_NOSIGNAL | MSG_DONTWAIT,
    )
  };

  assert_eq!(sent, 0);
  assert_eq!(errno_value(), 7004);

  set_errno(0);
  // SAFETY: `recv_probe` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      recv_probe.as_mut_ptr().cast::<c_void>(),
      sz(recv_probe.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EAGAIN);
}

#[test]
fn send_dontwait_zero_length_returns_zero_and_keeps_errno_without_enqueuing() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send dontwait zero-length test");
  let send_probe = [0xA7_u8];
  let mut recv_probe = [0xCC_u8; 1];

  set_errno(7008);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are read.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      send_probe.as_ptr().cast::<c_void>(),
      sz(0),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(sent, 0);
  assert_eq!(errno_value(), 7008);

  set_errno(0);
  // SAFETY: `recv_probe` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      recv_probe.as_mut_ptr().cast::<c_void>(),
      sz(recv_probe.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EAGAIN);
  assert_eq!(recv_probe, [0xCC_u8; 1]);
}

#[test]
fn send_nosignal_zero_length_returns_zero_and_keeps_errno_without_enqueuing() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send nosignal zero-length test");
  let send_probe = [0xA8_u8];
  let mut recv_probe = [0xDD_u8; 1];

  set_errno(7009);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are read.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      send_probe.as_ptr().cast::<c_void>(),
      sz(0),
      MSG_NOSIGNAL,
    )
  };

  assert_eq!(sent, 0);
  assert_eq!(errno_value(), 7009);

  set_errno(0);
  // SAFETY: `recv_probe` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      recv_probe.as_mut_ptr().cast::<c_void>(),
      sz(recv_probe.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EAGAIN);
  assert_eq!(recv_probe, [0xDD_u8; 1]);
}

#[test]
fn recv_waitall_with_dontwait_zero_length_returns_zero_and_keeps_errno_without_consuming_queue() {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for recv waitall+dontwait zero-length test");
  let payload = b"pq";
  let mut recv_probe = [0x5A_u8];

  writer
    .write_all(payload)
    .expect("failed to seed payload before zero-length recv with waitall+dontwait");

  set_errno(7005);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are written.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      recv_probe.as_mut_ptr().cast::<c_void>(),
      sz(0),
      MSG_WAITALL | MSG_DONTWAIT,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 7005);
  assert_eq!(recv_probe, [0x5A_u8]);

  let mut consumed = [0_u8; 2];
  // SAFETY: buffer is writable for two bytes and the socket has two queued bytes.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(consumed.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(consume_len, 2);
  assert_eq!(consumed, *payload);
}

#[test]
fn recv_waitall_with_dontwait_and_peek_zero_length_returns_zero_and_keeps_errno_without_consuming_queue()
 {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for recv waitall+dontwait+peek zero-length test");
  let payload = b"uv";
  let mut recv_probe = [0x5A_u8];

  writer
    .write_all(payload)
    .expect("failed to seed payload before zero-length recv with waitall+dontwait+peek");

  set_errno(7006);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are written.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      recv_probe.as_mut_ptr().cast::<c_void>(),
      sz(0),
      MSG_WAITALL | MSG_DONTWAIT | MSG_PEEK,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 7006);
  assert_eq!(recv_probe, [0x5A_u8]);

  let mut consumed = [0_u8; 2];
  // SAFETY: buffer is writable for two bytes and the socket has two queued bytes.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(consumed.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(consume_len, 2);
  assert_eq!(consumed, *payload);
}

#[test]
fn recv_waitall_with_peek_zero_length_returns_zero_and_keeps_errno_without_consuming_queue() {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for recv waitall+peek zero-length test");
  let payload = b"wx";
  let mut recv_probe = [0x5A_u8];

  writer
    .write_all(payload)
    .expect("failed to seed payload before zero-length recv with waitall+peek");

  set_errno(7007);
  // SAFETY: pointer is valid and `len == 0`, so no bytes are written.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      recv_probe.as_mut_ptr().cast::<c_void>(),
      sz(0),
      MSG_WAITALL | MSG_PEEK,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 7007);
  assert_eq!(recv_probe, [0x5A_u8]);

  let mut consumed = [0_u8; 2];
  // SAFETY: buffer is writable for two bytes and the socket has two queued bytes.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(consumed.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(consume_len, 2);
  assert_eq!(consumed, *payload);
}

#[test]
fn send_invalid_fd_returns_minus_one_and_errno_ebadf() {
  let payload = [0x11_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe { send(-1, payload.as_ptr().cast::<c_void>(), sz(payload.len()), 0) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_zero_length_and_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let sent = unsafe { send(-1, core::ptr::null(), sz(0), 0) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_zero_length_returns_minus_one_and_errno_ebadf() {
  let payload = [0x14_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe { send(-1, payload.as_ptr().cast::<c_void>(), sz(0), 0) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_huge_length_and_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let sent = unsafe { send(-1, core::ptr::null(), size_t::MAX, 0) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_huge_length_returns_minus_one_and_errno_ebadf() {
  let payload = [0x12_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe { send(-1, payload.as_ptr().cast::<c_void>(), size_t::MAX, 0) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_nosignal_and_dontwait_flags_returns_minus_one_and_errno_ebadf() {
  let payload = [0x13_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe {
    send(
      -1,
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_NOSIGNAL | MSG_DONTWAIT,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_nosignal_flag_returns_minus_one_and_errno_ebadf() {
  let payload = [0x16_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe {
    send(
      -1,
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_NOSIGNAL,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_zero_length_and_nosignal_flag_returns_minus_one_and_errno_ebadf() {
  let payload = [0x18_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe { send(-1, payload.as_ptr().cast::<c_void>(), sz(0), MSG_NOSIGNAL) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_zero_length_and_dontwait_flag_returns_minus_one_and_errno_ebadf() {
  let payload = [0x19_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe { send(-1, payload.as_ptr().cast::<c_void>(), sz(0), MSG_DONTWAIT) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_dontwait_flag_returns_minus_one_and_errno_ebadf() {
  let payload = [0x17_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe {
    send(
      -1,
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn send_invalid_fd_with_zero_length_and_nosignal_and_dontwait_flags_returns_minus_one_and_errno_ebadf()
 {
  let payload = [0x15_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let sent = unsafe {
    send(
      -1,
      payload.as_ptr().cast::<c_void>(),
      sz(0),
      MSG_NOSIGNAL | MSG_DONTWAIT,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn recv_invalid_fd_returns_minus_one_and_errno_ebadf() {
  let mut payload = [0x22_u8];

  set_errno(0);

  // SAFETY: payload pointer is valid and fd is intentionally invalid.
  let received = unsafe {
    recv(
      -1,
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      0,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn recv_invalid_fd_with_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let received = unsafe { recv(-1, core::ptr::null_mut(), sz(1), 0) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn recv_invalid_fd_with_zero_length_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0x23_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe { recv(-1, byte.as_mut_ptr().cast::<c_void>(), sz(0), 0) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x23_u8; 1]);
}

#[test]
fn recv_invalid_fd_with_zero_length_and_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let received = unsafe { recv(-1, core::ptr::null_mut(), sz(0), 0) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn recv_invalid_fd_with_huge_length_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe { recv(-1, byte.as_mut_ptr().cast::<c_void>(), size_t::MAX, 0) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn recv_invalid_fd_with_huge_length_and_null_buffer_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: null pointer is intentional and fd is intentionally invalid.
  let received = unsafe { recv(-1, core::ptr::null_mut(), size_t::MAX, 0) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn recv_invalid_fd_with_waitall_flag_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0x24_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe { recv(-1, byte.as_mut_ptr().cast::<c_void>(), sz(1), MSG_WAITALL) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x24_u8; 1]);
}

#[test]
fn recv_invalid_fd_with_zero_length_and_waitall_flag_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0x28_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe { recv(-1, byte.as_mut_ptr().cast::<c_void>(), sz(0), MSG_WAITALL) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x28_u8; 1]);
}

#[test]
fn recv_invalid_fd_with_peek_flag_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0x26_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe { recv(-1, byte.as_mut_ptr().cast::<c_void>(), sz(1), MSG_PEEK) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x26_u8; 1]);
}

#[test]
fn recv_invalid_fd_with_zero_length_and_peek_flag_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0x29_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe { recv(-1, byte.as_mut_ptr().cast::<c_void>(), sz(0), MSG_PEEK) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x29_u8; 1]);
}

#[test]
fn recv_invalid_fd_with_zero_length_and_peek_and_dontwait_flags_returns_minus_one_and_errno_ebadf()
{
  let mut byte = [0x2A_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe {
    recv(
      -1,
      byte.as_mut_ptr().cast::<c_void>(),
      sz(0),
      MSG_PEEK | MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x2A_u8; 1]);
}

#[test]
fn recv_invalid_fd_with_peek_and_dontwait_flags_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0x25_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe {
    recv(
      -1,
      byte.as_mut_ptr().cast::<c_void>(),
      sz(byte.len()),
      MSG_PEEK | MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x25_u8; 1]);
}

#[test]
fn recv_invalid_fd_with_waitall_and_peek_flags_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0x27_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe {
    recv(
      -1,
      byte.as_mut_ptr().cast::<c_void>(),
      sz(byte.len()),
      MSG_WAITALL | MSG_PEEK,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x27_u8; 1]);
}

#[test]
fn recv_invalid_fd_with_zero_length_and_waitall_and_peek_flags_returns_minus_one_and_errno_ebadf() {
  let mut byte = [0x2B_u8; 1];

  set_errno(0);

  // SAFETY: `byte` is writable and fd is intentionally invalid.
  let received = unsafe {
    recv(
      -1,
      byte.as_mut_ptr().cast::<c_void>(),
      sz(0),
      MSG_WAITALL | MSG_PEEK,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(byte, [0x2B_u8; 1]);
}

#[test]
fn recv_dontwait_on_empty_socket_returns_eagain() {
  let (reader, _writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for dontwait recv test");
  let mut payload = [0xCD_u8; 1];

  set_errno(0);

  // SAFETY: `payload` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EAGAIN);
  assert_eq!(payload, [0xCD_u8; 1]);
}

#[test]
fn recv_peek_with_dontwait_on_empty_socket_returns_eagain_and_keeps_buffer() {
  let (reader, _writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for peek+dontwait recv test");
  let mut payload = [0x9A_u8; 3];

  set_errno(0);

  // SAFETY: `payload` is writable and MSG_DONTWAIT avoids blocking.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_PEEK | MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EAGAIN);
  assert_eq!(payload, [0x9A_u8; 3]);
}

#[test]
fn recv_dontwait_on_closed_peer_returns_zero_and_keeps_errno() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for dontwait eof recv test");
  let mut payload = [0_u8; 1];

  drop(writer);
  set_errno(8118);

  // SAFETY: `payload` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 8118);
  assert_eq!(payload, [0_u8; 1]);
}

#[test]
fn recv_peek_on_closed_peer_returns_zero_and_keeps_errno() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for peek eof recv test");
  let mut payload = [0x4D_u8; 1];

  drop(writer);
  set_errno(8123);

  // SAFETY: `payload` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_PEEK,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 8123);
  assert_eq!(payload, [0x4D_u8; 1]);
}

#[test]
fn recv_peek_with_dontwait_on_closed_peer_returns_zero_and_keeps_errno() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for peek+dontwait eof recv test");
  let mut payload = [0x3E_u8; 1];

  drop(writer);
  set_errno(8124);

  // SAFETY: `payload` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_PEEK | MSG_DONTWAIT,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 8124);
  assert_eq!(payload, [0x3E_u8; 1]);
}

#[test]
fn recv_waitall_on_closed_peer_returns_zero_and_keeps_errno() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall eof recv test");
  let mut payload = [0x59_u8; 1];

  drop(writer);
  set_errno(8122);

  // SAFETY: `payload` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_WAITALL,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 8122);
  assert_eq!(payload, [0x59_u8; 1]);
}

#[test]
fn recv_waitall_with_dontwait_on_closed_peer_returns_zero_and_keeps_errno() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+dontwait eof recv test");
  let mut payload = [0x6A_u8; 1];

  drop(writer);
  set_errno(8120);

  // SAFETY: `payload` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_WAITALL | MSG_DONTWAIT,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 8120);
  assert_eq!(payload, [0x6A_u8; 1]);
}

#[test]
fn recv_waitall_with_dontwait_and_peek_on_closed_peer_returns_zero_and_keeps_errno() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+dontwait+peek eof recv test");
  let mut payload = [0x7B_u8; 1];

  drop(writer);
  set_errno(8119);

  // SAFETY: `payload` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_WAITALL | MSG_DONTWAIT | MSG_PEEK,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 8119);
  assert_eq!(payload, [0x7B_u8; 1]);
}

#[test]
fn recv_waitall_with_peek_on_closed_peer_returns_zero_and_keeps_errno() {
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+peek eof recv test");
  let mut payload = [0x8C_u8; 1];

  drop(writer);
  set_errno(8121);

  // SAFETY: `payload` is writable for one byte.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_WAITALL | MSG_PEEK,
    )
  };

  assert_eq!(received, 0);
  assert_eq!(errno_value(), 8121);
  assert_eq!(payload, [0x8C_u8; 1]);
}

#[test]
fn recv_waitall_with_dontwait_on_empty_socket_returns_eagain() {
  let (reader, _writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+dontwait recv test");
  let mut payload = [0_u8; 4];

  set_errno(0);

  // SAFETY: `payload` is writable and the socket descriptor is valid.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_WAITALL | MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EAGAIN);
  assert_eq!(payload, [0_u8; 4]);
}

#[test]
fn recv_waitall_with_dontwait_and_peek_on_empty_socket_returns_eagain() {
  let (reader, _writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+dontwait+peek recv test");
  let mut payload = [0xA5_u8; 4];

  set_errno(0);

  // SAFETY: `payload` is writable and the socket descriptor is valid.
  let received = unsafe {
    recv(
      reader.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_WAITALL | MSG_DONTWAIT | MSG_PEEK,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), EAGAIN);
  assert_eq!(payload, [0xA5_u8; 4]);
}

#[test]
fn recv_waitall_with_dontwait_returns_available_bytes_and_consumes_queue() {
  let payload = b"partial";
  let request_len = payload.len() + 4_usize;
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+dontwait partial recv test");
  let mut received = [0_u8; 16];

  writer
    .write_all(payload)
    .expect("failed to seed payload for waitall+dontwait partial recv test");

  set_errno(9559);

  // SAFETY: `received` is writable and socket descriptor is valid.
  let first_read = unsafe {
    recv(
      reader.as_raw_fd(),
      received.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      MSG_WAITALL | MSG_DONTWAIT,
    )
  };

  assert_eq!(
    first_read,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 9559);
  assert_eq!(&received[..payload.len()], payload);

  let mut probe = [0_u8; 1];

  set_errno(0);

  // SAFETY: `probe` is writable and socket descriptor is valid.
  let second_read = unsafe {
    recv(
      reader.as_raw_fd(),
      probe.as_mut_ptr().cast::<c_void>(),
      sz(probe.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(second_read, -1);
  assert_eq!(errno_value(), EAGAIN);
  assert_eq!(probe, [0_u8; 1]);
}

#[test]
fn recv_waitall_with_dontwait_reads_requested_length_and_preserves_remainder() {
  let payload = b"dontwait-complete";
  let request_len = 8_usize;
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+dontwait complete recv test");
  let mut first_chunk = [0_u8; 32];
  let mut remainder = [0_u8; 32];

  writer
    .write_all(payload)
    .expect("failed to seed payload for waitall+dontwait complete recv test");

  set_errno(9880);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let first_len = unsafe {
    recv(
      reader.as_raw_fd(),
      first_chunk.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      MSG_WAITALL | MSG_DONTWAIT,
    )
  };

  assert_eq!(
    first_len,
    ssize_t::try_from(request_len)
      .unwrap_or_else(|_| unreachable!("request length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 9880);
  assert_eq!(&first_chunk[..request_len], &payload[..request_len]);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let remainder_len = unsafe {
    recv(
      reader.as_raw_fd(),
      remainder.as_mut_ptr().cast::<c_void>(),
      sz(payload.len() - request_len),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(
    remainder_len,
    ssize_t::try_from(payload.len() - request_len)
      .unwrap_or_else(|_| unreachable!("remaining payload length must fit ssize_t")),
  );
  assert_eq!(
    &remainder[..payload.len() - request_len],
    &payload[request_len..]
  );
}

#[test]
fn recv_waitall_with_dontwait_and_peek_returns_available_bytes_without_consuming_queue() {
  let payload = b"tri-flag";
  let request_len = payload.len() + 5_usize;
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+dontwait+peek recv test");
  let mut peeked = [0_u8; 16];
  let mut consumed = [0_u8; 16];

  writer
    .write_all(payload)
    .expect("failed to seed payload for waitall+dontwait+peek recv test");

  set_errno(9779);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let peek_len = unsafe {
    recv(
      reader.as_raw_fd(),
      peeked.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      MSG_WAITALL | MSG_DONTWAIT | MSG_PEEK,
    )
  };

  assert_eq!(
    peek_len,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 9779);
  assert_eq!(&peeked[..payload.len()], payload);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(
    consume_len,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(&consumed[..payload.len()], payload);

  let mut probe = [0_u8; 1];

  set_errno(0);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let drained_read = unsafe {
    recv(
      reader.as_raw_fd(),
      probe.as_mut_ptr().cast::<c_void>(),
      sz(probe.len()),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(drained_read, -1);
  assert_eq!(errno_value(), EAGAIN);
  assert_eq!(probe, [0_u8; 1]);
}

#[test]
fn recv_waitall_with_dontwait_and_peek_reads_requested_length_without_consuming_bytes() {
  let payload = b"tri-flag-complete";
  let request_len = 6_usize;
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+dontwait+peek complete recv test");
  let mut peeked = [0_u8; 32];
  let mut consumed = [0_u8; 32];
  let mut remainder = [0_u8; 32];

  writer
    .write_all(payload)
    .expect("failed to seed payload for waitall+dontwait+peek complete recv test");

  set_errno(9901);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let peek_len = unsafe {
    recv(
      reader.as_raw_fd(),
      peeked.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      MSG_WAITALL | MSG_DONTWAIT | MSG_PEEK,
    )
  };

  assert_eq!(
    peek_len,
    ssize_t::try_from(request_len)
      .unwrap_or_else(|_| unreachable!("request length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 9901);
  assert_eq!(&peeked[..request_len], &payload[..request_len]);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(
    consume_len,
    ssize_t::try_from(request_len)
      .unwrap_or_else(|_| unreachable!("request length must fit ssize_t")),
  );
  assert_eq!(&consumed[..request_len], &payload[..request_len]);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let remainder_len = unsafe {
    recv(
      reader.as_raw_fd(),
      remainder.as_mut_ptr().cast::<c_void>(),
      sz(payload.len() - request_len),
      MSG_DONTWAIT,
    )
  };

  assert_eq!(
    remainder_len,
    ssize_t::try_from(payload.len() - request_len)
      .unwrap_or_else(|_| unreachable!("remaining payload length must fit ssize_t")),
  );
  assert_eq!(
    &remainder[..payload.len() - request_len],
    &payload[request_len..]
  );
}

#[test]
fn recv_peek_reads_without_consuming_stream_bytes() {
  let payload = b"peek-behavior";
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for peek recv test");

  writer
    .write_all(payload)
    .expect("failed to seed payload for peek recv test");

  let mut peeked = [0_u8; 32];
  let mut consumed = [0_u8; 32];

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let peek_len = unsafe {
    recv(
      reader.as_raw_fd(),
      peeked.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_PEEK,
    )
  };

  assert_eq!(
    peek_len,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(&peeked[..payload.len()], payload);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      0,
    )
  };

  assert_eq!(
    consume_len,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(&consumed[..payload.len()], payload);
}

#[test]
fn recv_peek_with_dontwait_reads_without_consuming_available_bytes() {
  let payload = b"peek-dontwait";
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for peek+dontwait recv test");

  writer
    .write_all(payload)
    .expect("failed to seed payload for peek+dontwait recv test");

  let mut peeked = [0_u8; 32];
  let mut consumed = [0_u8; 32];

  set_errno(7331);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let peek_len = unsafe {
    recv(
      reader.as_raw_fd(),
      peeked.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_PEEK | MSG_DONTWAIT,
    )
  };

  assert_eq!(
    peek_len,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 7331);
  assert_eq!(&peeked[..payload.len()], payload);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      0,
    )
  };

  assert_eq!(
    consume_len,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(&consumed[..payload.len()], payload);
}

#[test]
fn recv_waitall_with_peek_reads_requested_length_without_consuming_bytes() {
  let payload = b"waitall-peek-combo";
  let request_len = 7_usize;
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall+peek recv test");

  writer
    .write_all(payload)
    .expect("failed to seed payload for waitall+peek recv test");

  let mut peeked = [0_u8; 32];
  let mut consumed = [0_u8; 32];
  let mut remainder = [0_u8; 32];

  set_errno(8448);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let peek_len = unsafe {
    recv(
      reader.as_raw_fd(),
      peeked.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      MSG_WAITALL | MSG_PEEK,
    )
  };

  assert_eq!(
    peek_len,
    ssize_t::try_from(request_len)
      .unwrap_or_else(|_| unreachable!("request length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 8448);
  assert_eq!(&peeked[..request_len], &payload[..request_len]);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let consume_len = unsafe {
    recv(
      reader.as_raw_fd(),
      consumed.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      0,
    )
  };

  assert_eq!(
    consume_len,
    ssize_t::try_from(request_len)
      .unwrap_or_else(|_| unreachable!("request length must fit ssize_t")),
  );
  assert_eq!(&consumed[..request_len], &payload[..request_len]);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let remainder_len = unsafe {
    recv(
      reader.as_raw_fd(),
      remainder.as_mut_ptr().cast::<c_void>(),
      sz(payload.len() - request_len),
      0,
    )
  };

  assert_eq!(
    remainder_len,
    ssize_t::try_from(payload.len() - request_len)
      .unwrap_or_else(|_| unreachable!("remaining payload length must fit ssize_t")),
  );
  assert_eq!(
    &remainder[..payload.len() - request_len],
    &payload[request_len..]
  );
}

#[test]
fn recv_waitall_returns_partial_bytes_after_peer_shutdown() {
  let payload = b"waitall";
  let request_len = payload.len() + 3;
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall recv test");

  writer
    .write_all(payload)
    .expect("failed to seed payload for waitall recv test");
  writer
    .shutdown(Shutdown::Write)
    .expect("failed to shutdown writer for waitall recv test");

  let mut received = [0_u8; 32];

  set_errno(5151);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let recv_len = unsafe {
    recv(
      reader.as_raw_fd(),
      received.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      MSG_WAITALL,
    )
  };

  assert_eq!(
    recv_len,
    ssize_t::try_from(payload.len())
      .unwrap_or_else(|_| unreachable!("payload length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 5151);
  assert_eq!(&received[..payload.len()], payload);

  let mut eof_probe = [0_u8; 1];
  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let eof_len = unsafe {
    recv(
      reader.as_raw_fd(),
      eof_probe.as_mut_ptr().cast::<c_void>(),
      sz(eof_probe.len()),
      0,
    )
  };

  assert_eq!(eof_len, 0);
}

#[test]
fn recv_waitall_reads_requested_length_and_preserves_remainder() {
  let payload = b"waitall-complete";
  let request_len = 6_usize;
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for waitall complete recv test");

  writer
    .write_all(payload)
    .expect("failed to seed payload for waitall complete recv test");

  let mut first_chunk = [0_u8; 16];
  let mut remainder = [0_u8; 16];

  set_errno(6262);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let first_len = unsafe {
    recv(
      reader.as_raw_fd(),
      first_chunk.as_mut_ptr().cast::<c_void>(),
      sz(request_len),
      MSG_WAITALL,
    )
  };

  assert_eq!(
    first_len,
    ssize_t::try_from(request_len)
      .unwrap_or_else(|_| unreachable!("request length must fit ssize_t")),
  );
  assert_eq!(errno_value(), 6262);
  assert_eq!(&first_chunk[..request_len], &payload[..request_len]);

  // SAFETY: destination buffer is writable and socket descriptor is valid.
  let remainder_len = unsafe {
    recv(
      reader.as_raw_fd(),
      remainder.as_mut_ptr().cast::<c_void>(),
      sz(payload.len() - request_len),
      0,
    )
  };

  assert_eq!(
    remainder_len,
    ssize_t::try_from(payload.len() - request_len)
      .unwrap_or_else(|_| unreachable!("remaining payload length must fit ssize_t")),
  );
  assert_eq!(
    &remainder[..payload.len() - request_len],
    &payload[request_len..]
  );
}

#[test]
fn send_non_socket_fd_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("send-non-socket");
  let payload = [0x44_u8];

  fs::write(&file_path, b"not-socket").expect("failed to create non-socket fd test file");

  let file = File::open(&file_path).expect("failed to open non-socket fd test file");

  set_errno(0);

  // SAFETY: payload pointer is valid and file descriptor is intentionally not a socket.
  let sent = unsafe {
    send(
      file.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      0,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove non-socket fd test file");
}

#[test]
fn send_non_socket_fd_with_zero_length_and_null_buffer_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("send-non-socket-zero-null");

  fs::write(&file_path, b"not-socket").expect("failed to create non-socket zero/null fd test file");

  let file = File::open(&file_path).expect("failed to open non-socket zero/null fd test file");

  set_errno(0);

  // SAFETY: fd is intentionally not a socket and null pointer is passed with zero-length payload.
  let sent = unsafe { send(file.as_raw_fd(), core::ptr::null(), sz(0), 0) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove non-socket zero/null fd test file");
}

#[test]
fn send_non_socket_fd_with_huge_length_and_null_buffer_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("send-non-socket-huge-null");

  fs::write(&file_path, b"not-socket").expect("failed to create non-socket huge/null fd test file");

  let file = File::open(&file_path).expect("failed to open non-socket huge/null fd test file");

  set_errno(0);

  // SAFETY: fd is intentionally not a socket and null pointer is passed for invalid huge-length call.
  let sent = unsafe { send(file.as_raw_fd(), core::ptr::null(), size_t::MAX, 0) };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove non-socket huge/null fd test file");
}

#[test]
fn send_non_socket_fd_with_huge_length_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("send-non-socket-huge");
  let payload = [0x45_u8];

  fs::write(&file_path, b"not-socket").expect("failed to create non-socket huge fd test file");

  let file = File::open(&file_path).expect("failed to open non-socket huge fd test file");

  set_errno(0);

  // SAFETY: payload pointer is valid and descriptor is intentionally not a socket.
  let sent = unsafe {
    send(
      file.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      size_t::MAX,
      0,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove non-socket huge fd test file");
}

#[test]
fn send_non_socket_fd_with_nosignal_and_dontwait_flags_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("send-non-socket-flags");
  let payload = [0x46_u8];

  fs::write(&file_path, b"not-socket").expect("failed to create non-socket flag fd test file");

  let file = File::open(&file_path).expect("failed to open non-socket flag fd test file");

  set_errno(0);

  // SAFETY: payload pointer is valid and descriptor is intentionally not a socket.
  let sent = unsafe {
    send(
      file.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_NOSIGNAL | MSG_DONTWAIT,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove non-socket flag fd test file");
}

#[test]
fn send_nosignal_after_peer_shutdown_returns_minus_one_and_errno_epipe() {
  let payload = [0x66_u8];
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send nosignal test");

  drop(reader);
  set_errno(0);

  // SAFETY: payload pointer is valid and descriptor is a socket endpoint.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_NOSIGNAL,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EPIPE);
}

#[test]
fn send_nosignal_with_dontwait_after_peer_shutdown_returns_minus_one_and_errno_epipe() {
  let payload = [0x67_u8];
  let (reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for send nosignal+dontwait test");

  drop(reader);
  set_errno(1212);

  // SAFETY: payload pointer is valid and descriptor is a socket endpoint.
  let sent = unsafe {
    send(
      writer.as_raw_fd(),
      payload.as_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_NOSIGNAL | MSG_DONTWAIT,
    )
  };

  assert_eq!(sent, -1);
  assert_eq!(errno_value(), EPIPE);
}

#[test]
fn recv_non_socket_fd_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("recv-non-socket");
  let mut payload = [0x55_u8];

  fs::write(&file_path, b"not-socket").expect("failed to create recv non-socket fd test file");

  let file = File::open(&file_path).expect("failed to open recv non-socket fd test file");

  set_errno(0);

  // SAFETY: payload pointer is valid and file descriptor is intentionally not a socket.
  let received = unsafe {
    recv(
      file.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      0,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove recv non-socket fd test file");
}

#[test]
fn recv_non_socket_fd_with_zero_length_and_null_buffer_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("recv-non-socket-zero-null");

  fs::write(&file_path, b"not-socket")
    .expect("failed to create recv non-socket zero/null fd test file");

  let file = File::open(&file_path).expect("failed to open recv non-socket zero/null fd test file");

  set_errno(0);

  // SAFETY: fd is intentionally not a socket and null pointer is passed with zero-length payload.
  let received = unsafe { recv(file.as_raw_fd(), core::ptr::null_mut(), sz(0), 0) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove recv non-socket zero/null fd test file");
}

#[test]
fn recv_non_socket_fd_with_huge_length_and_null_buffer_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("recv-non-socket-huge-null");

  fs::write(&file_path, b"not-socket")
    .expect("failed to create recv non-socket huge/null fd test file");

  let file = File::open(&file_path).expect("failed to open recv non-socket huge/null fd test file");

  set_errno(0);

  // SAFETY: fd is intentionally not a socket and null pointer is passed for invalid huge-length call.
  let received = unsafe { recv(file.as_raw_fd(), core::ptr::null_mut(), size_t::MAX, 0) };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove recv non-socket huge/null fd test file");
}

#[test]
fn recv_non_socket_fd_with_huge_length_returns_minus_one_and_errno_enotsock() {
  let file_path = unique_temp_path("recv-non-socket-huge");
  let mut payload = [0x56_u8];

  fs::write(&file_path, b"not-socket").expect("failed to create recv non-socket huge fd test file");

  let file = File::open(&file_path).expect("failed to open recv non-socket huge fd test file");

  set_errno(0);

  // SAFETY: payload pointer is valid and descriptor is intentionally not a socket.
  let received = unsafe {
    recv(
      file.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      size_t::MAX,
      0,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove recv non-socket huge fd test file");
}

#[test]
fn recv_non_socket_fd_with_waitall_and_peek_and_dontwait_flags_returns_minus_one_and_errno_enotsock()
 {
  let file_path = unique_temp_path("recv-non-socket-flags");
  let mut payload = [0x57_u8];

  fs::write(&file_path, b"not-socket").expect("failed to create recv non-socket flag fd test file");

  let file = File::open(&file_path).expect("failed to open recv non-socket flag fd test file");

  set_errno(0);

  // SAFETY: payload pointer is valid and descriptor is intentionally not a socket.
  let received = unsafe {
    recv(
      file.as_raw_fd(),
      payload.as_mut_ptr().cast::<c_void>(),
      sz(payload.len()),
      MSG_WAITALL | MSG_PEEK | MSG_DONTWAIT,
    )
  };

  assert_eq!(received, -1);
  assert_eq!(errno_value(), ENOTSOCK);

  drop(file);
  fs::remove_file(file_path).expect("failed to remove recv non-socket flag fd test file");
}

#[test]
fn openat_opens_file_relative_to_directory_fd() {
  let directory = unique_temp_path("openat-dir");
  let file_name = "sample.txt";
  let file_path = directory.join(file_name);
  let expected = b"openat-relative";

  fs::create_dir_all(&directory).expect("failed to create temp directory for openat test");
  fs::write(&file_path, expected).expect("failed to create test file for openat test");

  let directory_handle =
    File::open(&directory).expect("failed to open temp directory for openat test");
  let directory_fd = directory_handle.as_raw_fd();
  let relative_path = CString::new(file_name).expect("file name must not contain NUL");

  set_errno(777);

  // SAFETY: directory fd is valid and `relative_path` is a valid C string.
  let fd = unsafe {
    openat(
      directory_fd,
      relative_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert!(fd >= 0, "openat failed with errno={}", errno_value());
  assert_eq!(errno_value(), 777);

  let mut buffer = [0_u8; 32];
  // SAFETY: `buffer` is writable for `buffer.len()` bytes.
  let read_len = unsafe { read(fd, buffer.as_mut_ptr().cast::<c_void>(), sz(buffer.len())) };

  assert_eq!(
    read_len,
    ssize_t::try_from(expected.len())
      .unwrap_or_else(|_| unreachable!("expected length must fit ssize_t")),
  );
  assert_eq!(&buffer[..expected.len()], expected);

  close_fd(fd);
  drop(directory_handle);
  fs::remove_file(file_path).expect("failed to remove openat temp file");
  fs::remove_dir(directory).expect("failed to remove openat temp directory");
}

#[test]
fn openat_with_at_fdcwd_opens_relative_path_from_process_cwd() {
  let file_name = format!(
    "rlibc-openat-at-fdcwd-{}-{}",
    std::process::id(),
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time before unix epoch")
      .as_nanos()
  );
  let file_path = std::env::current_dir()
    .expect("failed to read current directory for AT_FDCWD test")
    .join(&file_name);
  let expected = b"openat-at-fdcwd";
  let relative_path = CString::new(file_name.as_str()).expect("file name must not contain NUL");

  fs::write(&file_path, expected).expect("failed to create cwd-scoped file for AT_FDCWD test");

  set_errno(6161);

  // SAFETY: `relative_path` points to a valid NUL-terminated relative path.
  let fd = unsafe {
    openat(
      AT_FDCWD,
      relative_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert!(fd >= 0, "openat failed with errno={}", errno_value());
  assert_eq!(errno_value(), 6161);

  let mut buffer = [0_u8; 32];
  // SAFETY: `buffer` is writable for `buffer.len()` bytes.
  let read_len = unsafe { read(fd, buffer.as_mut_ptr().cast::<c_void>(), sz(buffer.len())) };

  assert_eq!(
    read_len,
    ssize_t::try_from(expected.len())
      .unwrap_or_else(|_| unreachable!("expected length must fit ssize_t")),
  );
  assert_eq!(&buffer[..expected.len()], expected);

  close_fd(fd);
  fs::remove_file(file_path).expect("failed to remove cwd-scoped file for AT_FDCWD test");
}

#[test]
fn openat_with_at_fdcwd_missing_relative_path_returns_minus_one_and_errno_enoent() {
  let missing_name = format!(
    "rlibc-openat-at-fdcwd-missing-{}-{}",
    std::process::id(),
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time before unix epoch")
      .as_nanos()
  );
  let missing_path = CString::new(missing_name).expect("path must not contain NUL");

  set_errno(0);

  // SAFETY: `missing_path` points to a valid NUL-terminated relative path.
  let fd = unsafe {
    openat(
      AT_FDCWD,
      missing_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn openat_with_at_fdcwd_empty_relative_path_returns_minus_one_and_errno_enoent() {
  let empty_path = CString::new("").expect("empty path must not contain interior NUL");

  set_errno(0);

  // SAFETY: `empty_path` points to a valid NUL-terminated path.
  let fd = unsafe {
    openat(
      AT_FDCWD,
      empty_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn openat_with_at_fdcwd_and_absolute_path_opens_file_and_keeps_errno() {
  let file_path = unique_temp_path("openat-at-fdcwd-absolute");
  let expected = b"openat-at-fdcwd-absolute";
  let absolute_path = CString::new(file_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  fs::write(&file_path, expected)
    .expect("failed to create temp file for AT_FDCWD absolute openat test");

  set_errno(6262);

  // SAFETY: `absolute_path` points to a valid NUL-terminated absolute path.
  let fd = unsafe {
    openat(
      AT_FDCWD,
      absolute_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert!(fd >= 0, "openat failed with errno={}", errno_value());
  assert_eq!(errno_value(), 6262);

  let mut buffer = [0_u8; 32];
  // SAFETY: `buffer` is writable for `buffer.len()` bytes.
  let read_len = unsafe { read(fd, buffer.as_mut_ptr().cast::<c_void>(), sz(buffer.len())) };

  assert_eq!(
    read_len,
    ssize_t::try_from(expected.len())
      .unwrap_or_else(|_| unreachable!("expected length must fit ssize_t")),
  );
  assert_eq!(&buffer[..expected.len()], expected);

  close_fd(fd);
  fs::remove_file(file_path).expect("failed to remove temp file for AT_FDCWD absolute openat test");
}

#[test]
fn openat_with_at_fdcwd_and_absolute_missing_path_returns_minus_one_and_errno_enoent() {
  let missing_path = unique_temp_path("openat-at-fdcwd-absolute-missing");
  let absolute_path = CString::new(missing_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  set_errno(0);

  // SAFETY: `absolute_path` points to a valid NUL-terminated absolute path.
  let fd = unsafe {
    openat(
      AT_FDCWD,
      absolute_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn openat_with_at_fdcwd_and_absolute_missing_path_overwrites_errno_to_enoent() {
  let missing_path = unique_temp_path("openat-at-fdcwd-absolute-missing-errno");
  let absolute_path = CString::new(missing_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  set_errno(7373);

  // SAFETY: `absolute_path` points to a valid NUL-terminated absolute path.
  let fd = unsafe {
    openat(
      AT_FDCWD,
      absolute_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn openat_with_at_fdcwd_and_null_path_returns_minus_one_and_errno_efault() {
  set_errno(0);

  // SAFETY: null path pointer is intentional to validate errno propagation.
  let fd = unsafe { openat(AT_FDCWD, core::ptr::null(), O_RDONLY, c_uint::from(0_u8)) };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), EFAULT);
}

#[test]
fn openat_null_path_returns_minus_one_and_errno_efault() {
  let directory = unique_temp_path("openat-null-path");

  fs::create_dir_all(&directory)
    .expect("failed to create temp directory for openat null-path test");

  let directory_handle =
    File::open(&directory).expect("failed to open temp directory for openat null-path test");

  set_errno(0);

  // SAFETY: null path pointer is intentional to validate errno propagation.
  let fd = unsafe {
    openat(
      directory_handle.as_raw_fd(),
      core::ptr::null(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), EFAULT);

  drop(directory_handle);
  fs::remove_dir(directory).expect("failed to remove temp directory for openat null-path test");
}

#[test]
fn openat_invalid_directory_fd_returns_minus_one_and_errno_ebadf() {
  let relative_path = CString::new("relative.txt").expect("path must not contain interior NUL");

  set_errno(0);

  // SAFETY: path points to a valid NUL-terminated string; fd is intentionally invalid.
  let fd = unsafe {
    openat(
      -1,
      relative_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn openat_empty_relative_path_returns_minus_one_and_errno_enoent() {
  let directory = unique_temp_path("openat-empty-relative");
  let empty_path = CString::new("").expect("empty path must not contain interior NUL");

  fs::create_dir_all(&directory).expect("failed to create temp directory for openat empty-path");

  let directory_handle =
    File::open(&directory).expect("failed to open temp directory for openat empty-path");

  set_errno(0);

  // SAFETY: directory fd and path pointer are valid; empty relative path must fail with ENOENT.
  let fd = unsafe {
    openat(
      directory_handle.as_raw_fd(),
      empty_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);

  drop(directory_handle);
  fs::remove_dir(directory).expect("failed to remove temp directory for openat empty-path");
}

#[test]
fn openat_missing_relative_path_returns_minus_one_and_errno_enoent() {
  let directory = unique_temp_path("openat-missing-relative");
  let missing_file = CString::new("missing-child.txt").expect("path must not contain interior NUL");

  fs::create_dir_all(&directory)
    .expect("failed to create temp directory for openat missing-relative test");

  let directory_handle =
    File::open(&directory).expect("failed to open temp directory for openat missing-relative test");

  set_errno(0);

  // SAFETY: directory fd and path pointer are valid; target file intentionally does not exist.
  let fd = unsafe {
    openat(
      directory_handle.as_raw_fd(),
      missing_file.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);

  drop(directory_handle);
  fs::remove_dir(directory).expect("failed to remove temp directory for openat missing-relative");
}

#[test]
fn openat_non_directory_fd_returns_minus_one_and_errno_enotdir() {
  let file_path = unique_temp_path("openat-non-directory");
  let regular_file = File::create(&file_path).expect("failed to create regular file for openat");
  let relative_path = CString::new("child.txt").expect("path must not contain interior NUL");

  set_errno(0);

  // SAFETY: path points to a valid NUL-terminated string; fd is intentionally not a directory.
  let fd = unsafe {
    openat(
      regular_file.as_raw_fd(),
      relative_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOTDIR);

  drop(regular_file);
  fs::remove_file(file_path).expect("failed to remove regular file for openat");
}

#[test]
fn openat_with_absolute_path_ignores_invalid_directory_fd() {
  let file_path = unique_temp_path("openat-absolute");
  let expected = b"openat-absolute";

  fs::write(&file_path, expected).expect("failed to create temp file for absolute openat test");

  let absolute_path = CString::new(file_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  set_errno(999);

  // SAFETY: absolute path points to a valid NUL-terminated string.
  let fd = unsafe {
    openat(
      -1,
      absolute_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert!(fd >= 0, "openat failed with errno={}", errno_value());
  assert_eq!(errno_value(), 999);

  let mut buffer = [0_u8; 32];
  // SAFETY: `buffer` is writable for `buffer.len()` bytes.
  let read_len = unsafe { read(fd, buffer.as_mut_ptr().cast::<c_void>(), sz(buffer.len())) };

  assert_eq!(
    read_len,
    ssize_t::try_from(expected.len())
      .unwrap_or_else(|_| unreachable!("expected length must fit ssize_t")),
  );
  assert_eq!(&buffer[..expected.len()], expected);

  close_fd(fd);
  fs::remove_file(file_path).expect("failed to remove temp file for absolute openat test");
}

#[test]
fn openat_with_absolute_path_ignores_non_directory_fd_and_keeps_errno() {
  let dirfd_file_path = unique_temp_path("openat-absolute-nondirfd");
  let target_file_path = unique_temp_path("openat-absolute-target");
  let expected = b"openat-absolute-nondirfd";

  fs::write(&dirfd_file_path, b"dirfd-file")
    .expect("failed to create non-directory dirfd file for absolute openat test");
  fs::write(&target_file_path, expected)
    .expect("failed to create target file for absolute openat with non-directory fd test");

  let non_directory_file =
    File::open(&dirfd_file_path).expect("failed to open non-directory dirfd file");
  let absolute_path = CString::new(target_file_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  set_errno(2027);

  // SAFETY: `absolute_path` points to a valid NUL-terminated absolute path.
  let fd = unsafe {
    openat(
      non_directory_file.as_raw_fd(),
      absolute_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert!(fd >= 0, "openat failed with errno={}", errno_value());
  assert_eq!(errno_value(), 2027);

  let mut buffer = [0_u8; 32];
  // SAFETY: `buffer` is writable for `buffer.len()` bytes.
  let read_len = unsafe { read(fd, buffer.as_mut_ptr().cast::<c_void>(), sz(buffer.len())) };

  assert_eq!(
    read_len,
    ssize_t::try_from(expected.len())
      .unwrap_or_else(|_| unreachable!("expected length must fit ssize_t")),
  );
  assert_eq!(&buffer[..expected.len()], expected);

  close_fd(fd);
  drop(non_directory_file);
  fs::remove_file(target_file_path)
    .expect("failed to remove target file for absolute openat with non-directory fd test");
  fs::remove_file(dirfd_file_path)
    .expect("failed to remove non-directory dirfd file for absolute openat test");
}

#[test]
fn openat_with_absolute_path_ignores_directory_fd_and_keeps_errno() {
  let directory = unique_temp_path("openat-absolute-dirfd");
  let target_file_path = unique_temp_path("openat-absolute-dirfd-target");
  let expected = b"openat-absolute-dirfd";

  fs::create_dir_all(&directory)
    .expect("failed to create temp directory for absolute openat directory-fd test");
  fs::write(&target_file_path, expected)
    .expect("failed to create target file for absolute openat directory-fd test");

  let directory_handle = File::open(&directory)
    .expect("failed to open directory fd for absolute openat directory-fd test");
  let absolute_path = CString::new(target_file_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  set_errno(3031);

  // SAFETY: `absolute_path` points to a valid NUL-terminated absolute path.
  let fd = unsafe {
    openat(
      directory_handle.as_raw_fd(),
      absolute_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert!(fd >= 0, "openat failed with errno={}", errno_value());
  assert_eq!(errno_value(), 3031);

  let mut buffer = [0_u8; 32];
  // SAFETY: `buffer` is writable for `buffer.len()` bytes.
  let read_len = unsafe { read(fd, buffer.as_mut_ptr().cast::<c_void>(), sz(buffer.len())) };

  assert_eq!(
    read_len,
    ssize_t::try_from(expected.len())
      .unwrap_or_else(|_| unreachable!("expected length must fit ssize_t")),
  );
  assert_eq!(&buffer[..expected.len()], expected);

  close_fd(fd);
  drop(directory_handle);
  fs::remove_file(target_file_path)
    .expect("failed to remove target file for absolute openat directory-fd test");
  fs::remove_dir(directory)
    .expect("failed to remove temp directory for absolute openat directory-fd test");
}

#[test]
fn openat_with_absolute_missing_path_ignores_non_directory_fd_and_sets_enoent() {
  let dirfd_file_path = unique_temp_path("openat-absolute-missing-nondirfd");
  let missing_target_path = unique_temp_path("openat-absolute-missing-target");

  fs::write(&dirfd_file_path, b"dirfd-file")
    .expect("failed to create non-directory dirfd file for missing absolute openat test");

  let non_directory_file =
    File::open(&dirfd_file_path).expect("failed to open non-directory dirfd file");
  let absolute_missing_path = CString::new(missing_target_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  set_errno(0);

  // SAFETY: `absolute_missing_path` points to a valid NUL-terminated absolute path.
  let fd = unsafe {
    openat(
      non_directory_file.as_raw_fd(),
      absolute_missing_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);

  drop(non_directory_file);
  fs::remove_file(dirfd_file_path)
    .expect("failed to remove non-directory dirfd file for missing absolute openat test");
}

#[test]
fn openat_with_absolute_missing_path_ignores_invalid_directory_fd_and_sets_enoent() {
  let missing_target_path = unique_temp_path("openat-absolute-missing-invalid-dirfd");
  let absolute_missing_path = CString::new(missing_target_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  set_errno(0);

  // SAFETY: `absolute_missing_path` points to a valid NUL-terminated absolute path.
  let fd = unsafe {
    openat(
      -1,
      absolute_missing_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn openat_with_absolute_missing_path_ignores_directory_fd_and_sets_enoent() {
  let directory = unique_temp_path("openat-absolute-missing-dirfd");
  let missing_target_path = unique_temp_path("openat-absolute-missing-dirfd-target");
  let absolute_missing_path = CString::new(missing_target_path.as_os_str().as_encoded_bytes())
    .expect("absolute path must not contain NUL");

  fs::create_dir_all(&directory)
    .expect("failed to create temp directory for absolute missing openat dirfd test");

  let directory_handle =
    File::open(&directory).expect("failed to open directory fd for absolute missing openat test");

  set_errno(0);

  // SAFETY: `absolute_missing_path` points to a valid NUL-terminated absolute path.
  let fd = unsafe {
    openat(
      directory_handle.as_raw_fd(),
      absolute_missing_path.as_ptr().cast::<c_char>(),
      O_RDONLY,
      c_uint::from(0_u8),
    )
  };

  assert_eq!(fd, -1);
  assert_eq!(errno_value(), ENOENT);

  drop(directory_handle);
  fs::remove_dir(directory)
    .expect("failed to remove temp directory for absolute missing openat dirfd test");
}
