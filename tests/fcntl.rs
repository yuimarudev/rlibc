#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::{c_int, c_long};
use rlibc::abi::errno::{EBADF, EINVAL};
use rlibc::errno::__errno_location;
use rlibc::fcntl::{
  F_DUPFD, F_DUPFD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL, FD_CLOEXEC, O_ACCMODE, O_NONBLOCK,
  fcntl,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static UNIQUE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns valid thread-local storage for the
  // calling thread.
  unsafe { __errno_location().read() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns valid thread-local storage for the
  // calling thread.
  unsafe {
    __errno_location().write(value);
  }
}

fn as_c_long(value: c_int) -> c_long {
  c_long::from(value)
}

fn unique_temp_path() -> PathBuf {
  let counter = UNIQUE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("clock moved backwards")
    .as_nanos();
  let pid = std::process::id();

  std::env::temp_dir().join(format!("rlibc-i021-fcntl-{pid}-{nanos}-{counter}"))
}

fn create_read_only_temp_file(bytes: &[u8]) -> (PathBuf, File) {
  let path = unique_temp_path();
  let mut writer = File::create(&path)
    .unwrap_or_else(|error| panic!("failed to create temp file {}: {error}", path.display()));

  writer
    .write_all(bytes)
    .unwrap_or_else(|error| panic!("failed to write temp file {}: {error}", path.display()));
  drop(writer);

  let reader = OpenOptions::new()
    .read(true)
    .open(&path)
    .unwrap_or_else(|error| panic!("failed to reopen temp file {}: {error}", path.display()));

  (path, reader)
}

#[test]
fn fcntl_getfd_invalid_fd_sets_ebadf() {
  write_errno(0);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_GETFD, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_getfd_closed_fd_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes no pointer args.
  let result = unsafe { fcntl(closed_fd, F_GETFD, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_getfd_closed_fd_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes no pointer args.
  let result = unsafe { fcntl(closed_fd, F_GETFD, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_setfd_invalid_fd_sets_ebadf() {
  write_errno(0);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_SETFD, as_c_long(FD_CLOEXEC)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_setfd_closed_fd_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes descriptor flags.
  let result = unsafe { fcntl(closed_fd, F_SETFD, as_c_long(FD_CLOEXEC)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_setfd_closed_fd_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes descriptor flags.
  let result = unsafe { fcntl(closed_fd, F_SETFD, as_c_long(FD_CLOEXEC)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_getfd_success_keeps_errno_unchanged() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl getfd errno test");
  let fd = writer.as_raw_fd();

  write_errno(EBADF);
  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let result = unsafe { fcntl(fd, F_GETFD, as_c_long(0)) };

  assert!(result >= 0);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_setfd_success_keeps_errno_unchanged() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl setfd errno test");
  let fd = writer.as_raw_fd();

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let flags_before = unsafe { fcntl(fd, F_GETFD, as_c_long(0)) };

  assert!(flags_before >= 0);

  write_errno(EBADF);
  // SAFETY: `fd` is valid and command takes descriptor flags as the 3rd argument.
  let result = unsafe { fcntl(fd, F_SETFD, as_c_long(flags_before | FD_CLOEXEC)) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_setfd_roundtrip_cloexec_bit() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl descriptor-flag test");
  let fd = writer.as_raw_fd();

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let original_flags = unsafe { fcntl(fd, F_GETFD, as_c_long(0)) };

  assert!(original_flags >= 0);

  // SAFETY: `fd` is valid and command takes descriptor flags as the 3rd argument.
  let set_result = unsafe { fcntl(fd, F_SETFD, as_c_long(original_flags | FD_CLOEXEC)) };

  assert_eq!(set_result, 0);

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let with_cloexec = unsafe { fcntl(fd, F_GETFD, as_c_long(0)) };

  assert!(with_cloexec >= 0);
  assert_ne!(with_cloexec & FD_CLOEXEC, 0);

  // SAFETY: `fd` is valid and command takes descriptor flags as the 3rd argument.
  let clear_result = unsafe { fcntl(fd, F_SETFD, as_c_long(with_cloexec & !FD_CLOEXEC)) };

  assert_eq!(clear_result, 0);

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let cleared_flags = unsafe { fcntl(fd, F_GETFD, as_c_long(0)) };

  assert!(cleared_flags >= 0);
  assert_eq!(cleared_flags & FD_CLOEXEC, 0);
}

#[test]
fn fcntl_getfl_invalid_fd_sets_ebadf() {
  write_errno(0);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_GETFL, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_getfl_invalid_fd_overwrites_errno_with_ebadf() {
  write_errno(EINVAL);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_GETFL, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_getfl_closed_fd_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes no pointer args.
  let result = unsafe { fcntl(closed_fd, F_GETFL, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_getfl_closed_fd_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes no pointer args.
  let result = unsafe { fcntl(closed_fd, F_GETFL, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_setfl_invalid_fd_sets_ebadf() {
  write_errno(0);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_SETFL, as_c_long(O_NONBLOCK)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_setfl_invalid_fd_overwrites_errno_with_ebadf() {
  write_errno(EINVAL);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_SETFL, as_c_long(O_NONBLOCK)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_setfl_closed_fd_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes integer flags.
  let result = unsafe { fcntl(closed_fd, F_SETFL, as_c_long(O_NONBLOCK)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_setfl_closed_fd_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes integer flags.
  let result = unsafe { fcntl(closed_fd, F_SETFL, as_c_long(O_NONBLOCK)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_invalid_fd_sets_ebadf() {
  write_errno(0);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_DUPFD, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_dupfd_cloexec_invalid_fd_sets_ebadf() {
  write_errno(0);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_DUPFD_CLOEXEC, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_dupfd_cloexec_invalid_fd_overwrites_errno_with_ebadf() {
  write_errno(EINVAL);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_DUPFD_CLOEXEC, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_dupfd_cloexec_closed_fd_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes an integer minimum.
  let result = unsafe { fcntl(closed_fd, F_DUPFD_CLOEXEC, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_cloexec_closed_fd_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes an integer minimum.
  let result = unsafe { fcntl(closed_fd, F_DUPFD_CLOEXEC, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_cloexec_closed_fd_with_negative_minimum_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes an integer minimum.
  let result = unsafe { fcntl(closed_fd, F_DUPFD_CLOEXEC, as_c_long(-1)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_cloexec_closed_fd_with_negative_minimum_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes an integer minimum.
  let result = unsafe { fcntl(closed_fd, F_DUPFD_CLOEXEC, as_c_long(-1)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_cloexec_closed_fd_with_excessive_minimum_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes an integer minimum.
  let result = unsafe { fcntl(closed_fd, F_DUPFD_CLOEXEC, as_c_long(c_int::MAX)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_cloexec_sets_fd_cloexec_on_duplicate() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl cloexec duplicate test");
  let original_fd = writer.as_raw_fd();
  let minimum_fd = original_fd + 1;

  // SAFETY: `original_fd` is valid and `minimum_fd` is an integer argument.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD_CLOEXEC, as_c_long(minimum_fd)) };

  assert!(duplicate_fd >= minimum_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD_CLOEXEC` and owned here.
  let duplicate = unsafe { OwnedFd::from_raw_fd(duplicate_fd) };

  // SAFETY: `duplicate` is valid and command takes no pointer arguments.
  let duplicate_flags = unsafe { fcntl(duplicate.as_raw_fd(), F_GETFD, as_c_long(0)) };

  assert!(duplicate_flags >= 0);
  assert_ne!(duplicate_flags & FD_CLOEXEC, 0);
}

#[test]
fn fcntl_dupfd_cloexec_success_keeps_errno_unchanged() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl cloexec errno test");
  let original_fd = writer.as_raw_fd();
  let minimum_fd = original_fd + 1;

  write_errno(EBADF);
  // SAFETY: `original_fd` is valid and `minimum_fd` is an integer argument.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD_CLOEXEC, as_c_long(minimum_fd)) };

  assert!(duplicate_fd >= minimum_fd);
  assert_eq!(read_errno(), EBADF);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD_CLOEXEC` and owned here.
  let duplicate = unsafe { OwnedFd::from_raw_fd(duplicate_fd) };

  // SAFETY: `duplicate` is valid and command takes no pointer arguments.
  let duplicate_flags = unsafe { fcntl(duplicate.as_raw_fd(), F_GETFD, as_c_long(0)) };

  assert!(duplicate_flags >= 0);
  assert_ne!(duplicate_flags & FD_CLOEXEC, 0);
}

#[test]
fn fcntl_dupfd_cloexec_negative_minimum_sets_einval() {
  let (path, file) = create_read_only_temp_file(b"x");

  write_errno(0);
  // SAFETY: `fd` is valid and `F_DUPFD_CLOEXEC` expects an integer minimum descriptor.
  let result = unsafe { fcntl(file.as_raw_fd(), F_DUPFD_CLOEXEC, as_c_long(-1)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);

  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_cloexec_excessive_minimum_sets_einval() {
  let (path, file) = create_read_only_temp_file(b"x");

  write_errno(0);
  // SAFETY: `fd` is valid and `F_DUPFD_CLOEXEC` expects an integer minimum descriptor.
  let result = unsafe { fcntl(file.as_raw_fd(), F_DUPFD_CLOEXEC, as_c_long(c_int::MAX)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);

  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_invalid_fd_overwrites_errno_with_ebadf() {
  write_errno(EINVAL);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_DUPFD, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_dupfd_invalid_fd_with_negative_minimum_sets_ebadf() {
  write_errno(0);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_DUPFD, as_c_long(-1)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_dupfd_invalid_fd_with_negative_minimum_overwrites_errno_with_ebadf() {
  write_errno(EINVAL);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_DUPFD, as_c_long(-1)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_dupfd_invalid_fd_with_excessive_minimum_sets_ebadf() {
  write_errno(0);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_DUPFD, as_c_long(c_int::MAX)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_dupfd_invalid_fd_with_excessive_minimum_overwrites_errno_with_ebadf() {
  write_errno(EINVAL);

  // SAFETY: command does not dereference pointers and intentionally passes an invalid fd.
  let result = unsafe { fcntl(-1, F_DUPFD, as_c_long(c_int::MAX)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_dupfd_closed_fd_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes
  // integer minimum descriptor.
  let result = unsafe { fcntl(closed_fd, F_DUPFD, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_closed_fd_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes
  // integer minimum descriptor.
  let result = unsafe { fcntl(closed_fd, F_DUPFD, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_closed_fd_with_negative_minimum_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes
  // integer minimum descriptor.
  let result = unsafe { fcntl(closed_fd, F_DUPFD, as_c_long(-1)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_closed_fd_with_negative_minimum_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes
  // integer minimum descriptor.
  let result = unsafe { fcntl(closed_fd, F_DUPFD, as_c_long(-1)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_closed_fd_with_excessive_minimum_sets_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(0);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes
  // integer minimum descriptor.
  let result = unsafe { fcntl(closed_fd, F_DUPFD, as_c_long(c_int::MAX)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_closed_fd_with_excessive_minimum_overwrites_errno_with_ebadf() {
  let (path, file) = create_read_only_temp_file(b"x");
  let closed_fd = file.as_raw_fd();

  drop(file);
  write_errno(EINVAL);

  // SAFETY: `closed_fd` was previously valid but is now closed; command takes
  // integer minimum descriptor.
  let result = unsafe { fcntl(closed_fd, F_DUPFD, as_c_long(c_int::MAX)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EBADF);

  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_negative_minimum_sets_einval() {
  let (path, file) = create_read_only_temp_file(b"x");

  write_errno(0);
  // SAFETY: `fd` is valid and `F_DUPFD` expects an integer minimum descriptor.
  let result = unsafe { fcntl(file.as_raw_fd(), F_DUPFD, as_c_long(-1)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);

  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_excessive_minimum_sets_einval() {
  let (path, file) = create_read_only_temp_file(b"x");

  write_errno(0);
  // SAFETY: `fd` is valid and `F_DUPFD` expects an integer minimum descriptor.
  let result = unsafe { fcntl(file.as_raw_fd(), F_DUPFD, as_c_long(c_int::MAX)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);

  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_invalid_command_sets_einval() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl invalid command test");

  write_errno(0);
  // SAFETY: command does not dereference pointers and intentionally passes an unknown command.
  let result = unsafe { fcntl(writer.as_raw_fd(), -1, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fcntl_invalid_command_overwrites_errno_with_einval() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl invalid command overwrite test");

  write_errno(EBADF);
  // SAFETY: command does not dereference pointers and intentionally passes an unknown command.
  let result = unsafe { fcntl(writer.as_raw_fd(), -1, as_c_long(0)) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fcntl_getfl_success_keeps_errno_unchanged() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl errno test");

  write_errno(EBADF);
  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let result = unsafe { fcntl(writer.as_raw_fd(), F_GETFL, as_c_long(0)) };

  assert!(result >= 0);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_setfl_roundtrip_getfl_nonblock_bit() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl test");
  let fd = writer.as_raw_fd();

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let flags_before = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(flags_before >= 0);

  // SAFETY: `fd` is valid and command takes integer flags as the 3rd argument.
  let set_result = unsafe { fcntl(fd, F_SETFL, as_c_long(flags_before | O_NONBLOCK)) };

  assert_eq!(set_result, 0);

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let flags_after = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(flags_after >= 0);
  assert_ne!(flags_after & O_NONBLOCK, 0);
}

#[test]
fn fcntl_setfl_can_clear_nonblock_bit() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl clear-nonblock test");
  let fd = writer.as_raw_fd();

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let initial_flags = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(initial_flags >= 0);

  // SAFETY: `fd` is valid and command takes integer flags as the 3rd argument.
  let clear_result = unsafe { fcntl(fd, F_SETFL, as_c_long(initial_flags & !O_NONBLOCK)) };

  assert_eq!(clear_result, 0);

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let flags_without_nonblock = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(flags_without_nonblock >= 0);
  assert_eq!(flags_without_nonblock & O_NONBLOCK, 0);

  // SAFETY: `fd` is valid and command takes integer flags as the 3rd argument.
  let set_result = unsafe { fcntl(fd, F_SETFL, as_c_long(flags_without_nonblock | O_NONBLOCK)) };

  assert_eq!(set_result, 0);

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let flags_with_nonblock = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(flags_with_nonblock >= 0);
  assert_ne!(flags_with_nonblock & O_NONBLOCK, 0);
}

#[test]
fn fcntl_setfl_nonblock_changes_read_behavior() {
  let (reader, mut writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl nonblock behavior test");
  let fd = reader.as_raw_fd();

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let initial_flags = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(initial_flags >= 0);

  // SAFETY: `fd` is valid and command takes integer flags as the 3rd argument.
  let set_result = unsafe { fcntl(fd, F_SETFL, as_c_long(initial_flags | O_NONBLOCK)) };

  assert_eq!(set_result, 0);

  let (result_tx, result_rx) = mpsc::channel();
  let mut reader_clone = reader
    .try_clone()
    .expect("failed to clone unix stream reader for nonblock behavior test");
  let handle = std::thread::spawn(move || {
    let mut buf = [0_u8; 1];
    let outcome = reader_clone.read(&mut buf).map_err(|error| error.kind());

    result_tx
      .send(outcome)
      .expect("failed to send read outcome from worker thread");
  });
  let read_outcome = match result_rx.recv_timeout(Duration::from_millis(200)) {
    Ok(outcome) => outcome,
    Err(mpsc::RecvTimeoutError::Timeout) => {
      writer
        .write_all(b"x")
        .expect("failed to wake potentially blocked read call");

      let resumed = result_rx
        .recv_timeout(Duration::from_millis(200))
        .expect("read worker did not resume after wake byte");

      panic!("read blocked despite O_NONBLOCK; resumed outcome: {resumed:?}");
    }
    Err(mpsc::RecvTimeoutError::Disconnected) => {
      panic!("read worker disconnected before sending outcome");
    }
  };

  handle.join().expect("nonblock read worker thread panicked");
  assert_eq!(read_outcome, Err(std::io::ErrorKind::WouldBlock));
}

#[test]
fn fcntl_setfl_success_keeps_errno_unchanged() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl setfl errno test");
  let fd = writer.as_raw_fd();

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let flags_before = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(flags_before >= 0);

  write_errno(EBADF);
  // SAFETY: `fd` is valid and command takes integer flags as the 3rd argument.
  let set_result = unsafe { fcntl(fd, F_SETFL, as_c_long(flags_before | O_NONBLOCK)) };

  assert_eq!(set_result, 0);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fcntl_setfl_preserves_access_mode_bits() {
  let (path, file) = create_read_only_temp_file(b"mode");
  let fd = file.as_raw_fd();

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let flags_before = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(flags_before >= 0);

  // SAFETY: `fd` is valid and command takes integer flags as the 3rd argument.
  let set_result = unsafe { fcntl(fd, F_SETFL, as_c_long(flags_before | O_NONBLOCK)) };

  assert_eq!(set_result, 0);

  // SAFETY: `fd` is valid and command takes no pointer arguments.
  let flags_after = unsafe { fcntl(fd, F_GETFL, as_c_long(0)) };

  assert!(flags_after >= 0);
  assert_eq!(flags_after & O_ACCMODE, flags_before & O_ACCMODE);

  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_success_keeps_errno_unchanged() {
  let (path, file) = create_read_only_temp_file(b"dup");
  let original_fd = file.as_raw_fd();
  let minimum_fd = original_fd + 1;

  write_errno(EINVAL);
  // SAFETY: `original_fd` is valid and `minimum_fd` is an integer argument.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(minimum_fd)) };

  assert!(duplicate_fd >= minimum_fd);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let duplicate = unsafe { File::from_raw_fd(duplicate_fd) };

  drop(duplicate);
  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_skips_taken_minimum_descriptor() {
  let (path, file) = create_read_only_temp_file(b"taken");
  let original_fd = file.as_raw_fd();
  let first_minimum = original_fd + 2;

  // SAFETY: `original_fd` is valid and `first_minimum` is an integer argument.
  let first_duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(first_minimum)) };

  assert!(first_duplicate_fd >= first_minimum);

  // SAFETY: `first_duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let first_duplicate = unsafe { File::from_raw_fd(first_duplicate_fd) };
  let taken_minimum = first_duplicate.as_raw_fd();

  // SAFETY: `original_fd` is valid and `taken_minimum` is an integer argument.
  let second_duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(taken_minimum)) };

  assert!(second_duplicate_fd > taken_minimum);

  // SAFETY: `second_duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let second_duplicate = unsafe { File::from_raw_fd(second_duplicate_fd) };

  drop(second_duplicate);
  drop(first_duplicate);
  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_shares_file_status_flags() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl dupfd status-flag test");
  let original_fd = writer.as_raw_fd();

  // SAFETY: `original_fd` is valid and the minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` was freshly returned by `F_DUPFD` and is uniquely
  // owned by this test.
  let duplicate = unsafe { std::os::unix::net::UnixStream::from_raw_fd(duplicate_fd) };

  // SAFETY: `duplicate_fd` is valid and command takes no pointer arguments.
  let duplicate_flags_before = unsafe { fcntl(duplicate.as_raw_fd(), F_GETFL, as_c_long(0)) };

  assert!(duplicate_flags_before >= 0);

  // SAFETY: `duplicate_fd` is valid and command takes integer flags.
  let set_result = unsafe {
    fcntl(
      duplicate.as_raw_fd(),
      F_SETFL,
      as_c_long(duplicate_flags_before | O_NONBLOCK),
    )
  };

  assert_eq!(set_result, 0);

  // SAFETY: `original_fd` is valid and command takes no pointer arguments.
  let original_flags_after = unsafe { fcntl(original_fd, F_GETFL, as_c_long(0)) };

  assert!(original_flags_after >= 0);
  assert_ne!(original_flags_after & O_NONBLOCK, 0);
}

#[test]
fn fcntl_dupfd_inherits_existing_status_flags_on_creation() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl flag inheritance test");
  let original_fd = writer.as_raw_fd();

  // SAFETY: `original_fd` is valid and command takes no pointer arguments.
  let original_flags_before = unsafe { fcntl(original_fd, F_GETFL, as_c_long(0)) };

  assert!(original_flags_before >= 0);

  // SAFETY: `original_fd` is valid and command takes integer flags.
  let set_result = unsafe {
    fcntl(
      original_fd,
      F_SETFL,
      as_c_long(original_flags_before | O_NONBLOCK),
    )
  };

  assert_eq!(set_result, 0);

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let duplicate = unsafe { std::os::unix::net::UnixStream::from_raw_fd(duplicate_fd) };

  // SAFETY: `duplicate_fd` is valid and command takes no pointer arguments.
  let duplicate_flags = unsafe { fcntl(duplicate.as_raw_fd(), F_GETFL, as_c_long(0)) };

  assert!(duplicate_flags >= 0);
  assert_ne!(duplicate_flags & O_NONBLOCK, 0);
}

#[test]
fn fcntl_dupfd_status_flag_sync_is_bidirectional() {
  let (_reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl bidirectional status-flag test");
  let original_fd = writer.as_raw_fd();

  // SAFETY: `original_fd` is valid and the minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` was freshly returned by `F_DUPFD` and is uniquely
  // owned by this test.
  let duplicate = unsafe { std::os::unix::net::UnixStream::from_raw_fd(duplicate_fd) };

  // SAFETY: `original_fd` is valid and command takes no pointer arguments.
  let original_flags_before = unsafe { fcntl(original_fd, F_GETFL, as_c_long(0)) };

  assert!(original_flags_before >= 0);

  // SAFETY: `original_fd` is valid and command takes integer flags.
  let set_nonblock_result = unsafe {
    fcntl(
      original_fd,
      F_SETFL,
      as_c_long(original_flags_before | O_NONBLOCK),
    )
  };

  assert_eq!(set_nonblock_result, 0);

  // SAFETY: duplicated descriptor is valid and command takes no pointer args.
  let duplicate_flags_after_set = unsafe { fcntl(duplicate.as_raw_fd(), F_GETFL, as_c_long(0)) };

  assert!(duplicate_flags_after_set >= 0);
  assert_ne!(duplicate_flags_after_set & O_NONBLOCK, 0);

  // SAFETY: duplicated descriptor is valid and command takes integer flags.
  let clear_nonblock_result = unsafe {
    fcntl(
      duplicate.as_raw_fd(),
      F_SETFL,
      as_c_long(duplicate_flags_after_set & !O_NONBLOCK),
    )
  };

  assert_eq!(clear_nonblock_result, 0);

  // SAFETY: `original_fd` is valid and command takes no pointer arguments.
  let original_flags_after_clear = unsafe { fcntl(original_fd, F_GETFL, as_c_long(0)) };

  assert!(original_flags_after_clear >= 0);
  assert_eq!(original_flags_after_clear & O_NONBLOCK, 0);
}

#[test]
fn fcntl_dupfd_with_minimum_equal_to_source_fd_returns_distinct_fd() {
  let (path, file) = create_read_only_temp_file(b"dup-min-equal");
  let source_fd = file.as_raw_fd();

  // SAFETY: `source_fd` is valid and minimum descriptor is an integer.
  let duplicated_fd = unsafe { fcntl(source_fd, F_DUPFD, as_c_long(source_fd)) };

  assert!(duplicated_fd >= source_fd);
  assert_ne!(duplicated_fd, source_fd);

  // SAFETY: `duplicated_fd` is freshly returned by `F_DUPFD` and uniquely
  // owned by this test.
  let duplicated = unsafe { File::from_raw_fd(duplicated_fd) };

  drop(duplicated);
  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_duplicate_remains_usable_after_original_close() {
  let (mut reader, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl duplicate usability test");
  let original_fd = writer.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { std::os::unix::net::UnixStream::from_raw_fd(duplicate_fd) };

  drop(writer);

  duplicate
    .write_all(b"z")
    .expect("failed to write through duplicated descriptor");

  let mut received = [0_u8; 1];

  reader
    .read_exact(&mut received)
    .expect("failed to read byte written via duplicated descriptor");

  assert_eq!(received, [b'z']);
}

#[test]
fn fcntl_dupfd_original_stream_survives_duplicate_drop() {
  let (mut peer, mut original) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl original stream survivability test");
  let original_fd = original.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let duplicate = unsafe { std::os::unix::net::UnixStream::from_raw_fd(duplicate_fd) };

  drop(duplicate);

  original
    .write_all(b"r")
    .expect("failed to write from original after dropping duplicate descriptor");

  let mut received = [0_u8; 1];

  peer
    .read_exact(&mut received)
    .expect("failed to read byte from peer after original wrote post-duplicate-drop");

  assert_eq!(received, [b'r']);
}

#[test]
fn fcntl_dupfd_read_write_duplicate_can_read_from_peer() {
  let (mut peer, writer) = std::os::unix::net::UnixStream::pair()
    .expect("failed to create unix stream pair for fcntl duplicate read-write test");
  let original_fd = writer.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { std::os::unix::net::UnixStream::from_raw_fd(duplicate_fd) };
  let mut received = [0_u8; 1];

  peer
    .write_all(b"q")
    .expect("failed to write byte from peer endpoint");
  duplicate
    .read_exact(&mut received)
    .expect("failed to read byte from duplicated read-write descriptor");

  assert_eq!(received, [b'q']);
}

#[test]
fn fcntl_dupfd_file_duplicate_survives_original_drop() {
  let (path, file) = create_read_only_temp_file(b"dup-file");
  let original_fd = file.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { File::from_raw_fd(duplicate_fd) };

  drop(file);

  let mut bytes = [0_u8; 8];

  duplicate
    .read_exact(&mut bytes)
    .expect("failed to read from duplicate after dropping original descriptor");

  assert_eq!(&bytes, b"dup-file");

  drop(duplicate);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_original_file_survives_duplicate_drop() {
  let (path, mut original) = create_read_only_temp_file(b"drop-dup");
  let original_fd = original.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let duplicate = unsafe { File::from_raw_fd(duplicate_fd) };

  drop(duplicate);

  let mut bytes = [0_u8; 8];

  original
    .read_exact(&mut bytes)
    .expect("failed to read from original after dropping duplicate descriptor");

  assert_eq!(&bytes, b"drop-dup");

  drop(original);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_write_duplicate_survives_original_drop() {
  let path = unique_temp_path();
  let file = OpenOptions::new()
    .create(true)
    .truncate(true)
    .write(true)
    .open(&path)
    .unwrap_or_else(|error| panic!("failed to create temp file {}: {error}", path.display()));
  let original_fd = file.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { File::from_raw_fd(duplicate_fd) };

  drop(file);

  duplicate
    .write_all(b"dup-write")
    .expect("failed to write using duplicate after dropping original descriptor");
  duplicate
    .flush()
    .expect("failed to flush duplicate descriptor write");
  drop(duplicate);

  let mut verify = OpenOptions::new()
    .read(true)
    .open(&path)
    .unwrap_or_else(|error| panic!("failed to reopen temp file {}: {error}", path.display()));
  let mut content = Vec::new();

  verify
    .read_to_end(&mut content)
    .expect("failed to read back duplicate descriptor write");
  assert_eq!(content, b"dup-write");

  drop(verify);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_writes_share_file_offset_between_original_and_duplicate() {
  let path = unique_temp_path();
  let mut original = OpenOptions::new()
    .create(true)
    .truncate(true)
    .read(true)
    .write(true)
    .open(&path)
    .unwrap_or_else(|error| panic!("failed to create temp file {}: {error}", path.display()));
  let original_fd = original.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { File::from_raw_fd(duplicate_fd) };

  original
    .write_all(b"ab")
    .expect("failed to write prefix with original descriptor");
  duplicate
    .write_all(b"cd")
    .expect("failed to write suffix with duplicate descriptor");
  duplicate
    .flush()
    .expect("failed to flush duplicate descriptor write");

  drop(duplicate);
  drop(original);

  let mut verify = OpenOptions::new()
    .read(true)
    .open(&path)
    .unwrap_or_else(|error| panic!("failed to reopen temp file {}: {error}", path.display()));
  let mut content = Vec::new();

  verify
    .read_to_end(&mut content)
    .expect("failed to read file content written by shared-offset descriptors");
  assert_eq!(content, b"abcd");

  drop(verify);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_can_duplicate_already_duplicated_fd() {
  let (path, mut original) = create_read_only_temp_file(b"abc");
  let original_fd = original.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and valid.
  let second_duplicate_fd = unsafe { fcntl(duplicate_fd, F_DUPFD, as_c_long(duplicate_fd + 1)) };

  assert!(second_duplicate_fd > duplicate_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { File::from_raw_fd(duplicate_fd) };
  // SAFETY: `second_duplicate_fd` is freshly returned by `F_DUPFD` and
  // uniquely owned here.
  let mut second_duplicate = unsafe { File::from_raw_fd(second_duplicate_fd) };
  let mut first = [0_u8; 1];
  let mut second = [0_u8; 1];
  let mut third = [0_u8; 1];

  original
    .read_exact(&mut first)
    .expect("failed to read from original descriptor");
  duplicate
    .read_exact(&mut second)
    .expect("failed to read from first duplicate descriptor");
  second_duplicate
    .read_exact(&mut third)
    .expect("failed to read from second duplicate descriptor");

  assert_eq!(first[0], b'a');
  assert_eq!(second[0], b'b');
  assert_eq!(third[0], b'c');

  drop(second_duplicate);
  drop(duplicate);
  drop(original);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_preserves_access_mode_bits() {
  let (path, file) = create_read_only_temp_file(b"mode");
  let original_fd = file.as_raw_fd();

  // SAFETY: `original_fd` is valid and command takes no pointer arguments.
  let original_flags = unsafe { fcntl(original_fd, F_GETFL, as_c_long(0)) };

  assert!(original_flags >= 0);

  let original_access_mode = original_flags & O_ACCMODE;

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let duplicate = unsafe { File::from_raw_fd(duplicate_fd) };

  // SAFETY: `duplicate_fd` is valid and command takes no pointer arguments.
  let duplicate_flags = unsafe { fcntl(duplicate.as_raw_fd(), F_GETFL, as_c_long(0)) };

  assert!(duplicate_flags >= 0);
  assert_eq!(duplicate_flags & O_ACCMODE, original_access_mode);

  drop(duplicate);
  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_preserves_write_only_access_mode_bits() {
  let path = unique_temp_path();
  let file = OpenOptions::new()
    .create(true)
    .truncate(true)
    .write(true)
    .open(&path)
    .unwrap_or_else(|error| panic!("failed to create temp file {}: {error}", path.display()));
  let original_fd = file.as_raw_fd();

  // SAFETY: `original_fd` is valid and command takes no pointer arguments.
  let original_flags = unsafe { fcntl(original_fd, F_GETFL, as_c_long(0)) };

  assert!(original_flags >= 0);

  let original_access_mode = original_flags & O_ACCMODE;

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let duplicate = unsafe { File::from_raw_fd(duplicate_fd) };

  // SAFETY: `duplicate_fd` is valid and command takes no pointer arguments.
  let duplicate_flags = unsafe { fcntl(duplicate.as_raw_fd(), F_GETFL, as_c_long(0)) };

  assert!(duplicate_flags >= 0);
  assert_eq!(duplicate_flags & O_ACCMODE, original_access_mode);

  drop(duplicate);
  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_write_only_duplicate_read_returns_ebadf() {
  let path = unique_temp_path();
  let file = OpenOptions::new()
    .create(true)
    .truncate(true)
    .write(true)
    .open(&path)
    .unwrap_or_else(|error| panic!("failed to create temp file {}: {error}", path.display()));
  let original_fd = file.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { File::from_raw_fd(duplicate_fd) };
  let mut buf = [0_u8; 1];
  let error = duplicate
    .read(&mut buf)
    .expect_err("read on write-only duplicated descriptor unexpectedly succeeded");

  assert_eq!(error.raw_os_error(), Some(EBADF));

  drop(duplicate);
  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_read_only_duplicate_write_returns_ebadf() {
  let (path, file) = create_read_only_temp_file(b"seed");
  let original_fd = file.as_raw_fd();

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { File::from_raw_fd(duplicate_fd) };
  let error = duplicate
    .write(b"x")
    .expect_err("write on read-only duplicated descriptor unexpectedly succeeded");

  assert_eq!(error.raw_os_error(), Some(EBADF));

  drop(duplicate);
  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_preserves_read_write_access_mode_bits() {
  let path = unique_temp_path();
  let file = OpenOptions::new()
    .create(true)
    .truncate(true)
    .read(true)
    .write(true)
    .open(&path)
    .unwrap_or_else(|error| panic!("failed to create temp file {}: {error}", path.display()));
  let original_fd = file.as_raw_fd();

  // SAFETY: `original_fd` is valid and command takes no pointer arguments.
  let original_flags = unsafe { fcntl(original_fd, F_GETFL, as_c_long(0)) };

  assert!(original_flags >= 0);

  let original_access_mode = original_flags & O_ACCMODE;

  // SAFETY: `original_fd` is valid and minimum descriptor is an integer.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(original_fd + 1)) };

  assert!(duplicate_fd > original_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let duplicate = unsafe { File::from_raw_fd(duplicate_fd) };

  // SAFETY: `duplicate_fd` is valid and command takes no pointer arguments.
  let duplicate_flags = unsafe { fcntl(duplicate.as_raw_fd(), F_GETFL, as_c_long(0)) };

  assert!(duplicate_flags >= 0);
  assert_eq!(duplicate_flags & O_ACCMODE, original_access_mode);

  drop(duplicate);
  drop(file);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}

#[test]
fn fcntl_dupfd_respects_minimum_and_shares_offset() {
  let (path, mut original) = create_read_only_temp_file(b"abcdef");
  let original_fd = original.as_raw_fd();
  let minimum_fd = original_fd + 5;

  // SAFETY: `original_fd` is valid and `minimum_fd` is an integer argument.
  let duplicate_fd = unsafe { fcntl(original_fd, F_DUPFD, as_c_long(minimum_fd)) };

  assert!(duplicate_fd >= minimum_fd);

  // SAFETY: `duplicate_fd` is freshly returned by `F_DUPFD` and uniquely owned here.
  let mut duplicate = unsafe { File::from_raw_fd(duplicate_fd) };
  let mut first = [0_u8; 1];
  let mut second = [0_u8; 1];

  original
    .read_exact(&mut first)
    .expect("failed to read first byte from original fd");
  duplicate
    .read_exact(&mut second)
    .expect("failed to read second byte from duplicated fd");

  assert_eq!(first[0], b'a');
  assert_eq!(second[0], b'b');

  drop(duplicate);
  drop(original);
  fs::remove_file(&path)
    .unwrap_or_else(|error| panic!("failed to remove temp file {}: {error}", path.display()));
}
