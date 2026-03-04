#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::{c_char, c_int, c_long};
use rlibc::abi::errno::{
  EADDRINUSE, EAFNOSUPPORT, EAGAIN, EBADF, EFAULT, EINVAL, ENOENT, ENOTSOCK,
};
use rlibc::errno::__errno_location;
use rlibc::fcntl::{F_GETFL, O_NONBLOCK, fcntl};
use rlibc::socket::{
  AF_UNIX, SOCK_CLOEXEC, SOCK_NONBLOCK, SOCK_STREAM, SaFamilyT, Sockaddr, SockaddrUn, SocklenT,
  accept, bind, connect, listen, socket,
};
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

unsafe extern "C" {
  fn close(fd: c_int) -> c_int;
}

const F_GETFD: c_int = 1;
const FD_CLOEXEC: c_int = 1;

fn errno_value() -> c_int {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe { __errno_location().read() }
}

fn set_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe {
    __errno_location().write(value);
  }
}

fn to_socklen(value: usize) -> SocklenT {
  SocklenT::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize must fit socklen_t on x86_64 Linux"))
}

fn unique_socket_path() -> PathBuf {
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("clock moved backwards")
    .as_nanos();

  std::env::temp_dir().join(format!("rlibc-i050-{nanos}-{}.sock", std::process::id()))
}

fn cleanup_socket_path(path: &Path) {
  if path.exists() {
    fs::remove_file(path)
      .unwrap_or_else(|error| panic!("failed to remove stale socket {}: {error}", path.display()));
  }
}

fn close_fd(fd: c_int) {
  // SAFETY: `fd` is expected to be a live descriptor returned by the kernel.
  let result = unsafe { close(fd) };

  assert_eq!(result, 0, "close({fd}) failed with errno={}", errno_value());
}

fn sockaddr_un_for_path(path: &Path) -> SockaddrUn {
  let path_bytes = path.as_os_str().as_bytes();

  assert!(
    path_bytes.len() < 108,
    "socket path too long for sockaddr_un: {}",
    path.display(),
  );

  let mut address = SockaddrUn {
    sun_family: SaFamilyT::try_from(AF_UNIX)
      .unwrap_or_else(|_| unreachable!("AF_UNIX must fit sa_family_t")),
    sun_path: [0; 108],
  };

  for (index, byte) in path_bytes.iter().enumerate() {
    address.sun_path[index] =
      c_char::try_from(*byte).unwrap_or_else(|_| unreachable!("unix path byte must fit c_char"));
  }

  address.sun_path[path_bytes.len()] = 0;

  address
}

#[test]
fn socket_invalid_domain_returns_minus_one_and_errno() {
  set_errno(0);

  // SAFETY: invalid domain is intentional for errno-path verification.
  let fd = unsafe { socket(-1, SOCK_STREAM, 0) };

  assert_eq!(fd, -1);
  assert!(
    matches!(errno_value(), EAFNOSUPPORT | EINVAL),
    "expected EAFNOSUPPORT or EINVAL, got {}",
    errno_value(),
  );
}

#[test]
fn socket_invalid_domain_overwrites_existing_errno() {
  set_errno(EADDRINUSE);

  // SAFETY: invalid domain is intentional for errno-path verification.
  let fd = unsafe { socket(-1, SOCK_STREAM, 0) };

  assert_eq!(fd, -1);
  assert!(
    matches!(errno_value(), EAFNOSUPPORT | EINVAL),
    "expected EAFNOSUPPORT or EINVAL, got {}",
    errno_value(),
  );
}

#[test]
fn connect_invalid_fd_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: invalid descriptor is intentional and pointers are not dereferenced on this path.
  let result = unsafe { connect(-1, core::ptr::null(), 0) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn connect_invalid_fd_overwrites_existing_errno_with_ebadf() {
  set_errno(EADDRINUSE);

  // SAFETY: invalid descriptor is intentional and pointers are not dereferenced on this path.
  let result = unsafe { connect(-1, core::ptr::null(), 0) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn connect_non_socket_fd_returns_minus_one_and_errno_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));

  set_errno(0);
  // SAFETY: valid non-socket fd and null address exercise `connect(2)` errno contract.
  let result = unsafe { connect(file.as_raw_fd(), core::ptr::null(), 0) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
}

#[test]
fn connect_non_socket_fd_overwrites_existing_errno_with_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));

  set_errno(EADDRINUSE);
  // SAFETY: valid non-socket fd and null address exercise `connect(2)` errno contract.
  let result = unsafe { connect(file.as_raw_fd(), core::ptr::null(), 0) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
}

#[test]
fn connect_null_addr_with_nonzero_len_returns_minus_one_and_errno_efault() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(connect null addr) failed with errno={}",
    errno_value()
  );

  set_errno(0);
  // SAFETY: null address pointer with non-zero length intentionally exercises `connect(2)` fault path.
  let result = unsafe {
    connect(
      fd,
      core::ptr::null(),
      to_socklen(core::mem::size_of::<SockaddrUn>()),
    )
  };
  let connect_errno = errno_value();

  close_fd(fd);

  assert_eq!(result, -1);
  assert_eq!(connect_errno, EFAULT);
}

#[test]
fn connect_null_addr_with_nonzero_len_overwrites_existing_errno_with_efault() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(connect null addr overwrite errno) failed with errno={}",
    errno_value()
  );

  set_errno(EADDRINUSE);
  // SAFETY: null address pointer with non-zero length intentionally exercises `connect(2)` fault path.
  let result = unsafe {
    connect(
      fd,
      core::ptr::null(),
      to_socklen(core::mem::size_of::<SockaddrUn>()),
    )
  };
  let connect_errno = errno_value();

  close_fd(fd);

  assert_eq!(result, -1);
  assert_eq!(connect_errno, EFAULT);
}

#[test]
fn connect_missing_unix_socket_path_returns_minus_one_and_errno_enoent() {
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let client_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    client_fd >= 0,
    "socket(client) failed with errno={}",
    errno_value()
  );

  set_errno(0);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let connect_result = unsafe {
    connect(
      client_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };
  let connect_errno = errno_value();

  close_fd(client_fd);

  assert_eq!(connect_result, -1);
  assert_eq!(connect_errno, ENOENT);
}

#[test]
fn connect_missing_unix_socket_path_overwrites_existing_errno() {
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let client_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    client_fd >= 0,
    "socket(client overwrite errno) failed with errno={}",
    errno_value()
  );

  set_errno(EADDRINUSE);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let connect_result = unsafe {
    connect(
      client_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };
  let connect_errno = errno_value();

  close_fd(client_fd);

  assert_eq!(connect_result, -1);
  assert_eq!(connect_errno, ENOENT);
}

#[test]
fn listen_invalid_fd_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: invalid descriptor is intentional for errno-path verification.
  let result = unsafe { listen(-1, 16) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn listen_invalid_fd_overwrites_existing_errno_with_ebadf() {
  set_errno(EADDRINUSE);

  // SAFETY: invalid descriptor is intentional for errno-path verification.
  let result = unsafe { listen(-1, 16) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn listen_non_socket_fd_returns_minus_one_and_errno_enotsock() {
  let file = std::fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));

  set_errno(0);
  // SAFETY: integer fd and backlog argument follow `listen(2)` ABI.
  let result = unsafe { listen(file.as_raw_fd(), 8) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
}

#[test]
fn listen_non_socket_fd_overwrites_existing_errno_with_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));

  set_errno(EADDRINUSE);
  // SAFETY: integer fd and backlog argument follow `listen(2)` ABI.
  let result = unsafe { listen(file.as_raw_fd(), 8) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
}

#[test]
fn bind_invalid_fd_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: invalid descriptor is intentional and null address is not dereferenced on this path.
  let result = unsafe { bind(-1, core::ptr::null(), 0) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn bind_invalid_fd_overwrites_existing_errno_with_ebadf() {
  set_errno(EADDRINUSE);

  // SAFETY: invalid descriptor is intentional and null address is not dereferenced on this path.
  let result = unsafe { bind(-1, core::ptr::null(), 0) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn bind_null_addr_with_nonzero_len_returns_minus_one_and_errno_efault() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(bind null addr) failed with errno={}",
    errno_value()
  );

  set_errno(EADDRINUSE);
  // SAFETY: null address pointer with non-zero length intentionally exercises `bind(2)` fault path.
  let result = unsafe {
    bind(
      fd,
      core::ptr::null(),
      to_socklen(core::mem::size_of::<SockaddrUn>()),
    )
  };
  let bind_errno = errno_value();

  close_fd(fd);

  assert_eq!(result, -1);
  assert_eq!(bind_errno, EFAULT);
}

#[test]
fn bind_null_addr_with_nonzero_len_sets_errno_efault_when_seeded_with_zero() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(bind null addr seed zero) failed with errno={}",
    errno_value()
  );

  set_errno(0);
  // SAFETY: null address pointer with non-zero length intentionally exercises `bind(2)` fault path.
  let result = unsafe {
    bind(
      fd,
      core::ptr::null(),
      to_socklen(core::mem::size_of::<SockaddrUn>()),
    )
  };
  let bind_errno = errno_value();

  close_fd(fd);

  assert_eq!(result, -1);
  assert_eq!(bind_errno, EFAULT);
}

#[test]
fn bind_non_socket_fd_returns_minus_one_and_errno_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));

  set_errno(0);
  // SAFETY: valid non-socket fd and null address mirror `bind(2)` errno-path contract checks.
  let result = unsafe { bind(file.as_raw_fd(), core::ptr::null(), 0) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
}

#[test]
fn bind_non_socket_fd_overwrites_existing_errno_with_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));

  set_errno(EADDRINUSE);
  // SAFETY: valid non-socket fd and null address mirror `bind(2)` errno-path contract checks.
  let result = unsafe { bind(file.as_raw_fd(), core::ptr::null(), 0) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
}

#[test]
fn bind_missing_parent_directory_returns_minus_one_and_errno_enoent() {
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("clock moved backwards")
    .as_nanos();
  let missing_dir = std::env::temp_dir().join(format!(
    "rlibc-i050-missing-dir-{nanos}-{}",
    std::process::id()
  ));
  let socket_path = missing_dir.join("socket.sock");
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  if missing_dir.exists() {
    fs::remove_dir_all(&missing_dir).unwrap_or_else(|error| {
      panic!(
        "failed to clean stale directory {}: {error}",
        missing_dir.display()
      )
    });
  }

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(bind missing parent) failed with errno={}",
    errno_value()
  );

  set_errno(EADDRINUSE);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let bind_result = unsafe {
    bind(
      fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };
  let bind_errno = errno_value();

  close_fd(fd);

  assert_eq!(bind_result, -1);
  assert_eq!(bind_errno, ENOENT);
}

#[test]
fn accept_invalid_fd_returns_minus_one_and_errno_ebadf() {
  set_errno(0);

  // SAFETY: invalid descriptor is intentional and null output pointers are accepted by API contract.
  let result = unsafe { accept(-1, core::ptr::null_mut(), core::ptr::null_mut()) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn accept_invalid_fd_overwrites_existing_errno_with_ebadf() {
  set_errno(EADDRINUSE);

  // SAFETY: invalid descriptor is intentional and null output pointers are accepted by API contract.
  let result = unsafe { accept(-1, core::ptr::null_mut(), core::ptr::null_mut()) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
}

#[test]
fn accept_invalid_fd_with_null_addr_and_non_null_addrlen_overwrites_errno_with_ebadf() {
  let expected_len = to_socklen(core::mem::size_of::<SockaddrUn>());
  let mut peer_len = expected_len;

  set_errno(EADDRINUSE);
  // SAFETY: invalid descriptor is intentional; null addr and writable addrlen pointer are valid call forms.
  let result = unsafe { accept(-1, core::ptr::null_mut(), core::ptr::addr_of_mut!(peer_len)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(
    peer_len, expected_len,
    "accept should not modify addrlen when descriptor is invalid"
  );
}

#[test]
fn accept_invalid_fd_with_null_addr_and_non_null_addrlen_returns_minus_one_and_errno_ebadf() {
  let expected_len = to_socklen(core::mem::size_of::<SockaddrUn>());
  let mut peer_len = expected_len;

  set_errno(0);
  // SAFETY: invalid descriptor is intentional; null addr and writable addrlen pointer are valid call forms.
  let result = unsafe { accept(-1, core::ptr::null_mut(), core::ptr::addr_of_mut!(peer_len)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EBADF);
  assert_eq!(
    peer_len, expected_len,
    "accept should not modify addrlen when descriptor is invalid"
  );
}

#[test]
fn accept_non_socket_fd_returns_minus_one_and_errno_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));

  set_errno(0);
  // SAFETY: valid non-socket fd with null peer output pointers matches `accept(2)` contract.
  let result = unsafe {
    accept(
      file.as_raw_fd(),
      core::ptr::null_mut(),
      core::ptr::null_mut(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
}

#[test]
fn accept_non_socket_fd_overwrites_existing_errno_with_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));

  set_errno(EADDRINUSE);
  // SAFETY: valid non-socket fd with null peer output pointers matches `accept(2)` contract.
  let result = unsafe {
    accept(
      file.as_raw_fd(),
      core::ptr::null_mut(),
      core::ptr::null_mut(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
}

#[test]
fn accept_non_socket_fd_with_null_addr_and_non_null_addrlen_overwrites_errno_with_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));
  let expected_len = to_socklen(core::mem::size_of::<SockaddrUn>());
  let mut peer_len = expected_len;

  set_errno(EADDRINUSE);
  // SAFETY: valid non-socket fd, null addr, and writable addrlen pointer exercise ENOTSOCK path.
  let result = unsafe {
    accept(
      file.as_raw_fd(),
      core::ptr::null_mut(),
      core::ptr::addr_of_mut!(peer_len),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
  assert_eq!(
    peer_len, expected_len,
    "accept should not modify addrlen on non-socket fd failure"
  );
}

#[test]
fn accept_non_socket_fd_with_null_addr_and_non_null_addrlen_returns_minus_one_and_errno_enotsock() {
  let file = fs::File::open("/dev/null")
    .unwrap_or_else(|error| panic!("failed to open /dev/null for test: {error}"));
  let expected_len = to_socklen(core::mem::size_of::<SockaddrUn>());
  let mut peer_len = expected_len;

  set_errno(0);
  // SAFETY: valid non-socket fd, null addr, and writable addrlen pointer exercise ENOTSOCK path.
  let result = unsafe {
    accept(
      file.as_raw_fd(),
      core::ptr::null_mut(),
      core::ptr::addr_of_mut!(peer_len),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), ENOTSOCK);
  assert_eq!(
    peer_len, expected_len,
    "accept should not modify addrlen on non-socket fd failure"
  );
}

#[test]
fn accept_on_non_listening_socket_returns_minus_one_and_errno_einval() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(non-listening) failed with errno={}",
    errno_value()
  );

  set_errno(0);
  // SAFETY: null output pointers are valid when peer address output is not requested.
  let result = unsafe { accept(fd, core::ptr::null_mut(), core::ptr::null_mut()) };
  let accept_errno = errno_value();

  close_fd(fd);

  assert_eq!(result, -1);
  assert_eq!(accept_errno, EINVAL);
}

#[test]
fn accept_on_non_listening_socket_overwrites_existing_errno_with_einval() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(non-listening overwrite errno) failed with errno={}",
    errno_value()
  );

  set_errno(EADDRINUSE);
  // SAFETY: null output pointers are valid when peer address output is not requested.
  let result = unsafe { accept(fd, core::ptr::null_mut(), core::ptr::null_mut()) };
  let accept_errno = errno_value();

  close_fd(fd);

  assert_eq!(result, -1);
  assert_eq!(accept_errno, EINVAL);
}

#[test]
fn accept_on_non_listening_socket_with_null_addr_and_non_null_addrlen_returns_minus_one_and_errno_einval()
 {
  let expected_len = to_socklen(core::mem::size_of::<SockaddrUn>());
  let mut peer_len = expected_len;

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(non-listening null addr, non-null addrlen) failed with errno={}",
    errno_value()
  );

  set_errno(0);
  // SAFETY: null addr with writable addrlen pointer is a valid call form.
  let result = unsafe { accept(fd, core::ptr::null_mut(), core::ptr::addr_of_mut!(peer_len)) };
  let accept_errno = errno_value();

  close_fd(fd);

  assert_eq!(result, -1);
  assert_eq!(accept_errno, EINVAL);
  assert_eq!(
    peer_len, expected_len,
    "accept should not modify addrlen when addr is null on non-listening socket"
  );
}

#[test]
fn accept_on_non_listening_socket_with_null_addr_and_non_null_addrlen_overwrites_errno_with_einval()
{
  let expected_len = to_socklen(core::mem::size_of::<SockaddrUn>());
  let mut peer_len = expected_len;

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    fd >= 0,
    "socket(non-listening null addr, non-null addrlen) failed with errno={}",
    errno_value()
  );

  set_errno(EADDRINUSE);
  // SAFETY: null addr with writable addrlen pointer is a valid call form.
  let result = unsafe { accept(fd, core::ptr::null_mut(), core::ptr::addr_of_mut!(peer_len)) };
  let accept_errno = errno_value();

  close_fd(fd);

  assert_eq!(result, -1);
  assert_eq!(accept_errno, EINVAL);
  assert_eq!(
    peer_len, expected_len,
    "accept should not modify addrlen when addr is null on non-listening socket"
  );
}

#[test]
fn accept_nonblocking_without_pending_connection_returns_minus_one_and_errno_eagain() {
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let server_fd = unsafe { socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0) };

  assert!(
    server_fd >= 0,
    "socket(server) failed with errno={}",
    errno_value()
  );

  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let bind_result = unsafe {
    bind(
      server_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(bind_result, 0, "bind failed with errno={}", errno_value());

  // SAFETY: `server_fd` is a valid socket descriptor.
  let listen_result = unsafe { listen(server_fd, 8) };

  assert_eq!(
    listen_result,
    0,
    "listen failed with errno={}",
    errno_value()
  );

  set_errno(0);
  // SAFETY: null output pointers are valid when peer address output is not requested.
  let accept_result = unsafe { accept(server_fd, core::ptr::null_mut(), core::ptr::null_mut()) };
  let accept_errno = errno_value();

  close_fd(server_fd);
  cleanup_socket_path(&socket_path);

  assert_eq!(accept_result, -1);
  assert_eq!(accept_errno, EAGAIN);
}

#[test]
fn accept_nonblocking_without_pending_connection_overwrites_existing_errno_with_eagain() {
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let server_fd = unsafe { socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0) };

  assert!(
    server_fd >= 0,
    "socket(server overwrite errno) failed with errno={}",
    errno_value()
  );

  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let bind_result = unsafe {
    bind(
      server_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(bind_result, 0, "bind failed with errno={}", errno_value());

  // SAFETY: `server_fd` is a valid socket descriptor.
  let listen_result = unsafe { listen(server_fd, 8) };

  assert_eq!(
    listen_result,
    0,
    "listen failed with errno={}",
    errno_value()
  );

  set_errno(EADDRINUSE);
  // SAFETY: null output pointers are valid when peer address output is not requested.
  let accept_result = unsafe { accept(server_fd, core::ptr::null_mut(), core::ptr::null_mut()) };
  let accept_errno = errno_value();

  close_fd(server_fd);
  cleanup_socket_path(&socket_path);

  assert_eq!(accept_result, -1);
  assert_eq!(accept_errno, EAGAIN);
}

#[test]
fn socket_with_sock_nonblock_sets_nonblocking_flag() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0) };

  assert!(
    fd >= 0,
    "socket(SOCK_NONBLOCK) failed with errno={}",
    errno_value()
  );

  // SAFETY: `fd` is expected to be a live descriptor returned by the kernel.
  let file_status_flags = unsafe { fcntl(fd, F_GETFL, c_long::from(0)) };

  close_fd(fd);

  assert!(
    file_status_flags >= 0,
    "fcntl(F_GETFL) failed with errno={}",
    errno_value()
  );
  assert_ne!(
    file_status_flags & O_NONBLOCK,
    0,
    "SOCK_NONBLOCK should set O_NONBLOCK"
  );
}

#[test]
fn socket_with_sock_cloexec_sets_close_on_exec_flag() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0) };

  assert!(
    fd >= 0,
    "socket(SOCK_CLOEXEC) failed with errno={}",
    errno_value()
  );

  // SAFETY: `fd` is expected to be a live descriptor returned by the kernel.
  let descriptor_flags = unsafe { fcntl(fd, F_GETFD, c_long::from(0)) };

  close_fd(fd);

  assert!(
    descriptor_flags >= 0,
    "fcntl(F_GETFD) failed with errno={}",
    errno_value()
  );
  assert_ne!(
    descriptor_flags & FD_CLOEXEC,
    0,
    "SOCK_CLOEXEC should set FD_CLOEXEC"
  );
}

#[test]
fn socket_with_nonblock_and_cloexec_sets_both_flags() {
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0) };

  assert!(
    fd >= 0,
    "socket(SOCK_NONBLOCK|SOCK_CLOEXEC) failed with errno={}",
    errno_value()
  );

  // SAFETY: `fd` is expected to be a live descriptor returned by the kernel.
  let file_status_flags = unsafe { fcntl(fd, F_GETFL, c_long::from(0)) };
  // SAFETY: `fd` is expected to be a live descriptor returned by the kernel.
  let descriptor_flags = unsafe { fcntl(fd, F_GETFD, c_long::from(0)) };

  close_fd(fd);

  assert!(
    file_status_flags >= 0,
    "fcntl(F_GETFL) failed with errno={}",
    errno_value()
  );
  assert!(
    descriptor_flags >= 0,
    "fcntl(F_GETFD) failed with errno={}",
    errno_value()
  );
  assert_ne!(
    file_status_flags & O_NONBLOCK,
    0,
    "SOCK_NONBLOCK should set O_NONBLOCK"
  );
  assert_ne!(
    descriptor_flags & FD_CLOEXEC,
    0,
    "SOCK_CLOEXEC should set FD_CLOEXEC"
  );
}

#[test]
fn socket_success_does_not_overwrite_existing_errno() {
  let errno_before = EADDRINUSE;

  set_errno(errno_before);
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(fd >= 0, "socket failed with errno={}", errno_value());
  assert_eq!(
    errno_value(),
    errno_before,
    "successful socket should not overwrite errno"
  );

  close_fd(fd);
}

#[test]
fn bind_same_unix_path_twice_returns_minus_one_and_errno_eaddrinuse() {
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let first_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    first_fd >= 0,
    "socket(first) failed with errno={}",
    errno_value()
  );

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let second_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    second_fd >= 0,
    "socket(second) failed with errno={}",
    errno_value()
  );

  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let first_bind = unsafe {
    bind(
      first_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(
    first_bind,
    0,
    "initial bind failed with errno={}",
    errno_value()
  );

  set_errno(0);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let second_bind = unsafe {
    bind(
      second_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };
  let bind_errno = errno_value();

  close_fd(second_fd);
  close_fd(first_fd);
  cleanup_socket_path(&socket_path);

  assert_eq!(second_bind, -1);
  assert_eq!(bind_errno, EADDRINUSE);
}

#[test]
fn bind_same_unix_path_twice_overwrites_existing_errno_with_eaddrinuse() {
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let first_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    first_fd >= 0,
    "socket(first overwrite errno) failed with errno={}",
    errno_value()
  );

  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let second_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    second_fd >= 0,
    "socket(second overwrite errno) failed with errno={}",
    errno_value()
  );

  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let first_bind = unsafe {
    bind(
      first_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(
    first_bind,
    0,
    "initial bind failed with errno={}",
    errno_value()
  );

  set_errno(EINVAL);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let second_bind = unsafe {
    bind(
      second_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };
  let bind_errno = errno_value();

  close_fd(second_fd);
  close_fd(first_fd);
  cleanup_socket_path(&socket_path);

  assert_eq!(second_bind, -1);
  assert_eq!(bind_errno, EADDRINUSE);
}

#[test]
fn socket_bind_listen_connect_accept_unix_stream_round_trip_succeeds() {
  let errno_before = EADDRINUSE;
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  set_errno(errno_before);
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let server_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    server_fd >= 0,
    "socket(server) failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful socket should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let bind_result = unsafe {
    bind(
      server_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(bind_result, 0, "bind failed with errno={}", errno_value());
  assert_eq!(
    errno_value(),
    errno_before,
    "successful bind should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: `server_fd` is a valid socket descriptor.
  let listen_result = unsafe { listen(server_fd, 8) };

  assert_eq!(
    listen_result,
    0,
    "listen failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful listen should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let client_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    client_fd >= 0,
    "socket(client) failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful socket should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let connect_result = unsafe {
    connect(
      client_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(
    connect_result,
    0,
    "connect failed with errno={}",
    errno_value(),
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful connect should not overwrite errno"
  );

  let mut peer_address = SockaddrUn {
    sun_family: 0,
    sun_path: [0; 108],
  };
  let mut peer_len = address_len;

  set_errno(errno_before);
  // SAFETY: output pointers are valid writable storage for kernel peer address output.
  let accepted_fd = unsafe {
    accept(
      server_fd,
      core::ptr::addr_of_mut!(peer_address).cast::<Sockaddr>(),
      core::ptr::addr_of_mut!(peer_len),
    )
  };

  assert!(
    accepted_fd >= 0,
    "accept failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful accept should not overwrite errno"
  );
  assert_eq!(
    c_int::from(peer_address.sun_family),
    AF_UNIX,
    "accept should report AF_UNIX peer family"
  );

  let min_peer_len = to_socklen(core::mem::size_of::<SaFamilyT>());

  assert!(
    peer_len >= min_peer_len,
    "peer sockaddr length must include at least sa_family_t"
  );
  assert!(
    peer_len <= address_len,
    "peer sockaddr length must not exceed caller-provided buffer length"
  );

  close_fd(accepted_fd);
  close_fd(client_fd);
  close_fd(server_fd);
  cleanup_socket_path(&socket_path);
}

#[test]
fn accept_with_null_peer_pointers_succeeds_for_pending_unix_stream_connection() {
  let errno_before = EADDRINUSE;
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  set_errno(errno_before);
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let server_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    server_fd >= 0,
    "socket(server) failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful socket should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let bind_result = unsafe {
    bind(
      server_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(bind_result, 0, "bind failed with errno={}", errno_value());
  assert_eq!(
    errno_value(),
    errno_before,
    "successful bind should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: `server_fd` is a valid socket descriptor.
  let listen_result = unsafe { listen(server_fd, 8) };

  assert_eq!(
    listen_result,
    0,
    "listen failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful listen should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let client_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    client_fd >= 0,
    "socket(client) failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful socket should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let connect_result = unsafe {
    connect(
      client_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(
    connect_result,
    0,
    "connect failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful connect should not overwrite errno"
  );

  set_errno(errno_before);
  // SAFETY: Linux accepts null peer address pointers when the caller does not request peer info.
  let accepted_fd = unsafe { accept(server_fd, core::ptr::null_mut(), core::ptr::null_mut()) };

  assert!(
    accepted_fd >= 0,
    "accept(null,null) failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful accept should not overwrite errno"
  );

  close_fd(accepted_fd);
  close_fd(client_fd);
  close_fd(server_fd);
  cleanup_socket_path(&socket_path);
}

#[test]
fn accept_with_null_addr_and_non_null_addrlen_succeeds_for_pending_unix_stream_connection() {
  let errno_before = EADDRINUSE;
  let socket_path = unique_socket_path();
  let address = sockaddr_un_for_path(&socket_path);
  let address_len = to_socklen(core::mem::size_of::<SockaddrUn>());

  cleanup_socket_path(&socket_path);

  set_errno(errno_before);
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let server_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    server_fd >= 0,
    "socket(server) failed with errno={}",
    errno_value()
  );

  set_errno(errno_before);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let bind_result = unsafe {
    bind(
      server_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(bind_result, 0, "bind failed with errno={}", errno_value());

  set_errno(errno_before);
  // SAFETY: `server_fd` is a valid socket descriptor.
  let listen_result = unsafe { listen(server_fd, 8) };

  assert_eq!(
    listen_result,
    0,
    "listen failed with errno={}",
    errno_value()
  );

  set_errno(errno_before);
  // SAFETY: arguments follow Linux `socket(2)` contract for AF_UNIX stream sockets.
  let client_fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) };

  assert!(
    client_fd >= 0,
    "socket(client) failed with errno={}",
    errno_value()
  );

  set_errno(errno_before);
  // SAFETY: `address` points to initialized `sockaddr_un` data.
  let connect_result = unsafe {
    connect(
      client_fd,
      core::ptr::addr_of!(address).cast::<Sockaddr>(),
      address_len,
    )
  };

  assert_eq!(
    connect_result,
    0,
    "connect failed with errno={}",
    errno_value()
  );

  let mut peer_len = address_len;

  set_errno(errno_before);
  // SAFETY: Linux ignores addrlen when addr is null and still permits accepting a pending peer.
  let accepted_fd = unsafe {
    accept(
      server_fd,
      core::ptr::null_mut(),
      core::ptr::addr_of_mut!(peer_len),
    )
  };

  assert!(
    accepted_fd >= 0,
    "accept(null,&len) failed with errno={}",
    errno_value()
  );
  assert_eq!(
    errno_value(),
    errno_before,
    "successful accept should not overwrite errno"
  );
  assert_eq!(
    peer_len, address_len,
    "accept should not modify addrlen when addr is null"
  );

  close_fd(accepted_fd);
  close_fd(client_fd);
  close_fd(server_fd);
  cleanup_socket_path(&socket_path);
}
