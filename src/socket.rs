//! Minimal socket C ABI interfaces for Linux `x86_64`.
//!
//! This module provides syscall-backed wrappers for issue I050:
//! - `socket`
//! - `connect`
//! - `bind`
//! - `listen`
//! - `accept`
//!
//! Contract:
//! - success values follow libc shape (`fd` for `socket`/`accept`, `0` for others)
//! - failures return `-1` and set thread-local `errno`

use crate::abi::types::{c_char, c_int, c_long, c_uint, c_ushort};
use crate::errno::set_errno;
use crate::syscall::{syscall2, syscall3};

const SYS_SOCKET: c_long = 41;
const SYS_CONNECT: c_long = 42;
const SYS_ACCEPT: c_long = 43;
const SYS_BIND: c_long = 49;
const SYS_LISTEN: c_long = 50;
/// IPv4/IPv6-independent Unix-domain socket family identifier.
pub const AF_UNIX: c_int = 1;
/// Sequenced, reliable, connection-based byte stream socket type.
pub const SOCK_STREAM: c_int = 1;
/// Socket flag requesting close-on-exec behavior on the returned descriptor.
pub const SOCK_CLOEXEC: c_int = 0o2_000_000;
/// Socket flag requesting non-blocking I/O on the returned descriptor.
pub const SOCK_NONBLOCK: c_int = 0o4_000;

/// Linux `socklen_t` for the primary target ABI.
pub type SocklenT = c_uint;

/// Linux `sa_family_t` for the primary target ABI.
pub type SaFamilyT = c_ushort;

/// Generic socket address payload used by socket APIs.
///
/// This matches Linux `struct sockaddr` layout on `x86_64`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Sockaddr {
  /// Address family selector (`AF_*`).
  pub sa_family: SaFamilyT,
  /// Family-specific bytes for the concrete address type.
  pub sa_data: [c_char; 14],
}

/// Unix-domain socket address payload (`AF_UNIX`).
///
/// ABI notes:
/// - `sun_family` must be set to [`AF_UNIX`] for Unix-path sockets.
/// - `sun_path` is a byte string that is typically NUL-terminated for pathname
///   sockets.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SockaddrUn {
  /// Address family selector (`AF_UNIX`).
  pub sun_family: SaFamilyT,
  /// Socket pathname bytes.
  pub sun_path: [c_char; 108],
}

fn ptr_arg<T>(ptr: *const T) -> c_long {
  c_long::try_from(ptr.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit c_long on x86_64"))
}

fn mut_ptr_arg<T>(ptr: *mut T) -> c_long {
  ptr_arg(ptr.cast_const())
}

fn errno_from_raw(raw: c_long) -> c_int {
  c_int::try_from(-raw).unwrap_or(c_int::MAX)
}

fn fd_result(raw: c_long) -> c_int {
  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("fd must fit c_int"))
}

fn status_result(raw: c_long) -> c_int {
  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  0
}

/// C ABI entry point for `socket`.
///
/// Creates an endpoint for communication in the specified `domain`.
///
/// Returns:
/// - non-negative file descriptor on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// The caller must provide a valid `(domain, socket_type, protocol)`
/// combination accepted by Linux `socket(2)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn socket(domain: c_int, socket_type: c_int, protocol: c_int) -> c_int {
  let raw = unsafe {
    syscall3(
      SYS_SOCKET,
      c_long::from(domain),
      c_long::from(socket_type),
      c_long::from(protocol),
    )
  };

  fd_result(raw)
}

/// C ABI entry point for `connect`.
///
/// Initiates a connection on `sockfd` to peer address `addr`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// - `addr` must be readable for `addrlen` bytes.
/// - `sockfd` must refer to a socket descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn connect(sockfd: c_int, addr: *const Sockaddr, addrlen: SocklenT) -> c_int {
  let raw = unsafe {
    syscall3(
      SYS_CONNECT,
      c_long::from(sockfd),
      ptr_arg(addr),
      c_long::from(addrlen),
    )
  };

  status_result(raw)
}

/// C ABI entry point for `bind`.
///
/// Assigns local address `addr` to socket descriptor `sockfd`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// - `addr` must be readable for `addrlen` bytes.
/// - `sockfd` must refer to a socket descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bind(sockfd: c_int, addr: *const Sockaddr, addrlen: SocklenT) -> c_int {
  let raw = unsafe {
    syscall3(
      SYS_BIND,
      c_long::from(sockfd),
      ptr_arg(addr),
      c_long::from(addrlen),
    )
  };

  status_result(raw)
}

/// C ABI entry point for `listen`.
///
/// Marks `sockfd` as a passive socket and sets its pending queue depth.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// `sockfd` must refer to a valid stream/socket endpoint compatible with
/// `listen(2)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn listen(sockfd: c_int, backlog: c_int) -> c_int {
  let raw = unsafe { syscall2(SYS_LISTEN, c_long::from(sockfd), c_long::from(backlog)) };

  status_result(raw)
}

/// C ABI entry point for `accept`.
///
/// Accepts a pending connection from a listening socket.
///
/// Returns:
/// - non-negative accepted file descriptor on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// - `addr` may be null when caller does not request peer address output.
/// - when `addr` is non-null, `addrlen` must be non-null and writable.
/// - when both are non-null, `addr` must be writable for at least `*addrlen`
///   bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn accept(
  sockfd: c_int,
  addr: *mut Sockaddr,
  addrlen: *mut SocklenT,
) -> c_int {
  let raw = unsafe {
    syscall3(
      SYS_ACCEPT,
      c_long::from(sockfd),
      mut_ptr_arg(addr),
      mut_ptr_arg(addrlen),
    )
  };

  fd_result(raw)
}
