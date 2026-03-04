//! Minimal `fcntl` interfaces for Linux `x86_64`.
//!
//! This module implements the subset targeted by issue I021:
//! - `F_GETFL`
//! - `F_SETFL`
//! - `F_DUPFD`
//! - `F_GETFD`
//! - `F_SETFD`
//! - `F_DUPFD_CLOEXEC`
//!
//! The exported symbol uses a fixed third argument because current repository
//! tests exercise only commands that require one integer argument.

use crate::abi::types::{c_int, c_long};
use crate::errno::set_errno;
use crate::syscall::syscall3;

/// Duplicate file descriptor command (`fcntl`).
pub const F_DUPFD: c_int = 0;
/// Get descriptor flags command (`fcntl`).
pub const F_GETFD: c_int = 1;
/// Set descriptor flags command (`fcntl`).
pub const F_SETFD: c_int = 2;
/// Duplicate descriptor command that sets `FD_CLOEXEC` on the new descriptor.
pub const F_DUPFD_CLOEXEC: c_int = 1030;
/// Get file status flags command (`fcntl`).
pub const F_GETFL: c_int = 3;
/// Set file status flags command (`fcntl`).
pub const F_SETFL: c_int = 4;
/// Close-on-exec descriptor flag for `F_GETFD` / `F_SETFD`.
pub const FD_CLOEXEC: c_int = 1;
/// Access mode bit mask for `O_*` flags.
pub const O_ACCMODE: c_int = 0o3;
/// Non-blocking mode flag.
pub const O_NONBLOCK: c_int = 0o4000;
const SYS_FCNTL: c_long = 72;

/// C ABI entry point for `fcntl` (minimal 3-argument form).
///
/// Returns command-dependent non-negative values on success.
/// Returns `-1` and sets `errno` on failure.
///
/// The third argument is modeled as `c_long` so integer flags and pointer-sized
/// command payloads preserve bit width on `x86_64`.
///
/// # Safety
/// The caller must provide a valid `fd` and command/argument combination
/// according to Linux `fcntl(2)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fcntl(fd: c_int, cmd: c_int, arg: c_long) -> c_int {
  let raw = unsafe { syscall3(SYS_FCNTL, c_long::from(fd), c_long::from(cmd), arg) };

  if raw < 0 {
    let errno = c_int::try_from(-raw).unwrap_or(c_int::MAX);

    set_errno(errno);

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("fcntl result must fit c_int"))
}
