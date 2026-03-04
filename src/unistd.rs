//! Minimal unistd I/O entry points.
//!
//! This module implements Linux `x86_64` syscall-backed wrappers for:
//! - `open`
//! - `openat`
//! - `read`
//! - `recv`
//! - `send`
//! - `write`
//!
//! Each wrapper follows libc-style return contracts:
//! - success: non-negative value
//! - failure: `-1` and thread-local `errno` set

use crate::abi::types::{c_int, c_long, c_uint, size_t, ssize_t};
use crate::errno::set_errno;
use crate::syscall::{syscall3, syscall4, syscall6};
use core::ffi::{c_char, c_void};

const SYS_READ: c_long = 0;
const SYS_WRITE: c_long = 1;
const SYS_OPEN: c_long = 2;
const SYS_OPENAT: c_long = 257;
const SYS_SENDTO: c_long = 44;
const SYS_RECVFROM: c_long = 45;
/// Special `openat` directory descriptor that resolves relative paths against
/// the process current working directory.
///
/// This matches Linux `AT_FDCWD` from `fcntl.h` and can be passed as `dirfd`
/// to [`openat`] when no directory file descriptor is available.
pub const AT_FDCWD: c_int = -100;
/// Message flag for peeking at receive-queue data without consuming it.
pub const MSG_PEEK: c_int = 0x2;
/// Message flag for non-blocking send/receive behavior.
pub const MSG_DONTWAIT: c_int = 0x40;
/// Message flag requesting full-buffer receive where possible.
pub const MSG_WAITALL: c_int = 0x100;
/// Message flag suppressing `SIGPIPE` generation on send failures.
pub const MSG_NOSIGNAL: c_int = 0x4_000;

fn ptr_arg<T>(ptr: *const T) -> c_long {
  c_long::try_from(ptr.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit c_long on x86_64"))
}

fn mut_ptr_arg<T>(ptr: *mut T) -> c_long {
  ptr_arg(ptr.cast_const())
}

const fn size_arg(count: size_t) -> c_long {
  c_long::from_ne_bytes(count.to_ne_bytes())
}

fn errno_from_raw(raw: c_long) -> c_int {
  c_int::try_from(-raw).unwrap_or(c_int::MAX)
}

/// C ABI entry point for `open`.
///
/// Opens `path` with `flags` and optional `mode` and returns a file descriptor
/// on success.
///
/// Returns:
/// - non-negative file descriptor on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Safety
/// - `path` must point to a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: c_uint) -> c_int {
  let raw = unsafe {
    syscall3(
      SYS_OPEN,
      ptr_arg(path),
      c_long::from(flags),
      c_long::from(mode),
    )
  };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("fd must fit c_int"))
}

/// C ABI entry point for `openat`.
///
/// Opens `path` relative to `dirfd` and returns a file descriptor on success.
///
/// Returns:
/// - non-negative file descriptor on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Safety
/// - `path` must point to a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openat(
  dirfd: c_int,
  path: *const c_char,
  flags: c_int,
  mode: c_uint,
) -> c_int {
  let raw = unsafe {
    syscall4(
      SYS_OPENAT,
      c_long::from(dirfd),
      ptr_arg(path),
      c_long::from(flags),
      c_long::from(mode),
    )
  };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("fd must fit c_int"))
}

/// C ABI entry point for `read`.
///
/// Reads up to `count` bytes from `fd` into `buf`.
/// `count` is forwarded as its raw machine-word value per Linux syscall ABI.
///
/// # Safety
/// - `buf` must be writable for at least `count` bytes when `count > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
  let count_long = size_arg(count);
  let raw = unsafe { syscall3(SYS_READ, c_long::from(fd), mut_ptr_arg(buf), count_long) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  raw
}

/// C ABI entry point for `write`.
///
/// Writes up to `count` bytes from `buf` to `fd`.
/// `count` is forwarded as its raw machine-word value per Linux syscall ABI.
///
/// # Safety
/// - `buf` must be readable for at least `count` bytes when `count > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
  let count_long = size_arg(count);
  let raw = unsafe { syscall3(SYS_WRITE, c_long::from(fd), ptr_arg(buf), count_long) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  raw
}

/// C ABI entry point for `send`.
///
/// Sends up to `len` bytes from `buf` to the socket `sockfd` with message
/// `flags`.
///
/// Returns:
/// - non-negative transferred byte count on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `sockfd` is not a valid descriptor.
/// - `ENOTSOCK`: `sockfd` does not refer to a socket.
/// - `EPIPE`: peer closed the connection (kernel may also raise `SIGPIPE`
///   unless `MSG_NOSIGNAL` is set).
/// - other Linux socket-layer errno values are forwarded unchanged.
///
/// # Safety
/// - `buf` must be readable for at least `len` bytes when `len > 0`.
/// - `sockfd` must reference a valid socket descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send(
  sockfd: c_int,
  buf: *const c_void,
  len: size_t,
  flags: c_int,
) -> ssize_t {
  let raw = unsafe {
    syscall6(
      SYS_SENDTO,
      c_long::from(sockfd),
      ptr_arg(buf),
      size_arg(len),
      c_long::from(flags),
      0,
      0,
    )
  };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  raw
}

/// C ABI entry point for `recv`.
///
/// Receives up to `len` bytes from socket `sockfd` into `buf` with message
/// `flags`.
///
/// Returns:
/// - non-negative transferred byte count on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `sockfd` is not a valid descriptor.
/// - `ENOTSOCK`: `sockfd` does not refer to a socket.
/// - `EAGAIN`: no data is currently available with non-blocking flags.
/// - other Linux socket-layer errno values are forwarded unchanged.
///
/// # Safety
/// - `buf` must be writable for at least `len` bytes when `len > 0`.
/// - `sockfd` must reference a valid socket descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recv(
  sockfd: c_int,
  buf: *mut c_void,
  len: size_t,
  flags: c_int,
) -> ssize_t {
  let raw = unsafe {
    syscall6(
      SYS_RECVFROM,
      c_long::from(sockfd),
      mut_ptr_arg(buf),
      size_arg(len),
      c_long::from(flags),
      0,
      0,
    )
  };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  raw
}
