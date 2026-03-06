//! Minimal unistd/process I/O entry points.
//!
//! This module implements Linux `x86_64` syscall-backed wrappers for:
//!
//! - `access`
//! - `close`
//! - `dup`
//! - `dup2`
//! - `dup3`
//! - `getpid`
//! - `getppid`
//! - `getpgid`
//! - `getpgrp`
//! - `getsid`
//! - `gettid`
//! - `getuid`
//! - `geteuid`
//! - `getgid`
//! - `getegid`
//! - `isatty`
//! - `lseek`
//! - `open`
//! - `openat`
//! - `pipe`
//! - `pipe2`
//! - `fsync`
//! - `fdatasync`
//! - `sync`
//! - `syncfs`
//! - `read`
//! - `recv`
//! - `send`
//! - `unlink`
//! - `write`
//!
//! To keep the Rust module surface aligned with the existing `<unistd.h>`
//! header without moving the underlying C entry points, this module also
//! re-exports the `crate::system`-backed `<unistd.h>` APIs and selectors:
//! - `gethostname`
//! - `getpagesize`
//! - `sysconf`
//! - `_SC_CLK_TCK`
//! - `_SC_OPEN_MAX`
//! - `_SC_PAGESIZE` / `_SC_PAGE_SIZE`
//! - `_SC_NPROCESSORS_CONF` / `_SC_NPROCESSORS_ONLN`
//! - `HOST_NAME_MAX`
//!
//! Each wrapper follows libc-style return contracts:
//! - success: non-negative value
//! - failure: `-1` and thread-local `errno` set

use crate::abi::types::{c_int, c_long, c_uint, size_t, ssize_t};
use crate::errno::set_errno;
use crate::syscall::{syscall0, syscall1, syscall2, syscall3, syscall4, syscall6};
pub use crate::system::{
  _SC_CLK_TCK, _SC_NPROCESSORS_CONF, _SC_NPROCESSORS_ONLN, _SC_OPEN_MAX, _SC_PAGE_SIZE,
  _SC_PAGESIZE, gethostname, getpagesize, sysconf,
};
use core::ffi::{c_char, c_void};

const SYS_READ: c_long = 0;
const SYS_WRITE: c_long = 1;
const SYS_OPEN: c_long = 2;
const SYS_CLOSE: c_long = 3;
const SYS_LSEEK: c_long = 8;
const SYS_IOCTL: c_long = 16;
const SYS_UNLINK: c_long = 87;
const SYS_ACCESS: c_long = 21;
const SYS_GETPID: c_long = 39;
const SYS_GETUID: c_long = 102;
const SYS_GETGID: c_long = 104;
const SYS_GETEUID: c_long = 107;
const SYS_GETEGID: c_long = 108;
const SYS_GETPPID: c_long = 110;
const SYS_GETPGRP: c_long = 111;
const SYS_GETPGID: c_long = 121;
const SYS_GETSID: c_long = 124;
const SYS_GETTID: c_long = 186;
const SYS_DUP: c_long = 32;
const SYS_DUP2: c_long = 33;
const SYS_DUP3: c_long = 292;
const SYS_PIPE: c_long = 22;
const SYS_PIPE2: c_long = 293;
const SYS_OPENAT: c_long = 257;
const SYS_FSYNC: c_long = 74;
const SYS_FDATASYNC: c_long = 75;
const SYS_SYNC: c_long = 162;
const SYS_SYNCFS: c_long = 306;
const SYS_SENDTO: c_long = 44;
const SYS_RECVFROM: c_long = 45;
const TCGETS: c_long = 0x5401;
const KERNEL_TERMIOS_SIZE: usize = 60;
/// Linux `AT_FDCWD` sentinel used by `openat` for current-directory resolution.
pub const AT_FDCWD: c_int = -100;
/// Seek relative to the beginning of a file.
pub const SEEK_SET: c_int = 0;
/// Seek relative to the current file offset.
pub const SEEK_CUR: c_int = 1;
/// Seek relative to the end of a file.
pub const SEEK_END: c_int = 2;
/// Message flag for peeking at receive-queue data without consuming it.
pub const MSG_PEEK: c_int = 0x2;
/// Message flag for non-blocking send/receive behavior.
pub const MSG_DONTWAIT: c_int = 0x40;
/// Message flag requesting full-buffer receive where possible.
pub const MSG_WAITALL: c_int = 0x100;
/// Message flag suppressing `SIGPIPE` generation on send failures.
pub const MSG_NOSIGNAL: c_int = 0x4_000;
/// Maximum hostname payload length exposed through `<unistd.h>`.
///
/// This matches the Linux `utsname.nodename` payload limit copied by
/// [`gethostname`], excluding the trailing NUL terminator.
pub const HOST_NAME_MAX: c_int = 64;

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

fn abi_u32_bits_as_c_int(raw: c_long) -> c_int {
  let value = c_uint::try_from(raw)
    .unwrap_or_else(|_| unreachable!("identity syscall return must fit Linux uid_t/gid_t width"));

  c_int::from_ne_bytes(value.to_ne_bytes())
}

/// C ABI entry point for `access`.
///
/// Checks real-user access permissions for `pathname` against `mode`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EACCES`, `ENOENT`, `ENOTDIR`, and other kernel-provided `access(2)` errno values
///   are forwarded unchanged.
///
/// # Safety
/// - `pathname` must be a valid NUL-terminated string pointer when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn access(pathname: *const c_char, mode: c_int) -> c_int {
  // SAFETY: syscall number, pointer argument, and integer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall2(SYS_ACCESS, ptr_arg(pathname), c_long::from(mode)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("access return value must fit c_int"))
}

/// C ABI entry point for `unlink`.
///
/// Removes the directory entry named by `pathname`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `ENOENT`, `EISDIR`, and other kernel-provided `unlink(2)` errno values are forwarded
///   unchanged.
///
/// # Safety
/// - `pathname` must be a valid NUL-terminated string pointer when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlink(pathname: *const c_char) -> c_int {
  // SAFETY: syscall number and pointer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_UNLINK, ptr_arg(pathname)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("unlink return value must fit c_int"))
}

/// C ABI entry point for `close`.
///
/// Closes `fd`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `fd` is not a valid open file descriptor.
/// - other kernel-provided `close(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn close(fd: c_int) -> c_int {
  // SAFETY: syscall number and integer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_CLOSE, c_long::from(fd)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("close return value must fit c_int"))
}

/// C ABI entry point for `dup`.
///
/// Duplicates `oldfd` and returns a new descriptor number.
///
/// Returns:
/// - duplicated descriptor number on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `oldfd` is not a valid open file descriptor.
/// - other kernel-provided `dup(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn dup(oldfd: c_int) -> c_int {
  // SAFETY: syscall number and integer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_DUP, c_long::from(oldfd)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("dup return value must fit c_int"))
}

/// C ABI entry point for `dup2`.
///
/// Duplicates `oldfd` onto descriptor number `newfd`.
///
/// Returns:
/// - duplicated descriptor number (`newfd`) on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `oldfd` or `newfd` is invalid.
/// - other kernel-provided `dup2(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn dup2(oldfd: c_int, newfd: c_int) -> c_int {
  // SAFETY: syscall number and integer arguments follow Linux x86_64 ABI.
  let raw = unsafe { syscall2(SYS_DUP2, c_long::from(oldfd), c_long::from(newfd)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("dup2 return value must fit c_int"))
}

/// C ABI entry point for `dup3`.
///
/// Duplicates `oldfd` onto descriptor number `newfd` with Linux-specific
/// descriptor flags.
///
/// Returns:
/// - duplicated descriptor number (`newfd`) on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `oldfd` or `newfd` is invalid.
/// - `EINVAL`: `oldfd == newfd` or unsupported `flags` are provided.
/// - other kernel-provided `dup3(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn dup3(oldfd: c_int, newfd: c_int, flags: c_int) -> c_int {
  // SAFETY: syscall number and integer arguments follow Linux x86_64 ABI.
  let raw = unsafe {
    syscall3(
      SYS_DUP3,
      c_long::from(oldfd),
      c_long::from(newfd),
      c_long::from(flags),
    )
  };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("dup3 return value must fit c_int"))
}

/// C ABI entry point for `getpid`.
///
/// Returns the caller process identifier.
///
/// Returns:
/// - positive process id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - forwards kernel-provided `getpid(2)` errno values when present.
#[unsafe(no_mangle)]
pub extern "C" fn getpid() -> c_int {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let raw = unsafe { syscall0(SYS_GETPID) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("getpid return value must fit c_int"))
}

/// C ABI entry point for `getppid`.
///
/// Returns the parent process identifier of the caller.
///
/// Returns:
/// - positive parent process id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - forwards kernel-provided `getppid(2)` errno values when present.
#[unsafe(no_mangle)]
pub extern "C" fn getppid() -> c_int {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let raw = unsafe { syscall0(SYS_GETPPID) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("getppid return value must fit c_int"))
}

/// C ABI entry point for `getpgid`.
///
/// Returns the process group identifier for `pid` (`0` means caller).
///
/// Returns:
/// - positive process group id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `ESRCH`: no process exists for `pid`.
/// - `EINVAL`: invalid `pid` argument.
/// - other kernel-provided `getpgid(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn getpgid(pid: c_int) -> c_int {
  // SAFETY: syscall number and integer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_GETPGID, c_long::from(pid)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("getpgid return value must fit c_int"))
}

/// C ABI entry point for `getpgrp`.
///
/// Returns the process group identifier of the caller.
///
/// Returns:
/// - positive process group id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - forwards kernel-provided `getpgrp(2)` errno values when present.
#[unsafe(no_mangle)]
pub extern "C" fn getpgrp() -> c_int {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let raw = unsafe { syscall0(SYS_GETPGRP) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("getpgrp return value must fit c_int"))
}

/// C ABI entry point for `getsid`.
///
/// Returns the session identifier for `pid` (`0` means caller).
///
/// Returns:
/// - positive session id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `ESRCH`: no process exists for `pid`.
/// - other kernel-provided `getsid(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn getsid(pid: c_int) -> c_int {
  // SAFETY: syscall number and integer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_GETSID, c_long::from(pid)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("getsid return value must fit c_int"))
}

/// C ABI entry point for `gettid`.
///
/// Returns the caller thread identifier.
///
/// Returns:
/// - positive thread id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - forwards kernel-provided `gettid(2)` errno values when present.
#[unsafe(no_mangle)]
pub extern "C" fn gettid() -> c_int {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let raw = unsafe { syscall0(SYS_GETTID) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("gettid return value must fit c_int"))
}

/// C ABI entry point for `getuid`.
///
/// Returns the real user identifier of the caller.
///
/// Returns:
/// - non-negative user id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - forwards kernel-provided `getuid(2)` errno values when present.
#[unsafe(no_mangle)]
pub extern "C" fn getuid() -> c_int {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let raw = unsafe { syscall0(SYS_GETUID) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  abi_u32_bits_as_c_int(raw)
}

/// C ABI entry point for `geteuid`.
///
/// Returns the effective user identifier of the caller.
///
/// Returns:
/// - non-negative effective user id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - forwards kernel-provided `geteuid(2)` errno values when present.
#[unsafe(no_mangle)]
pub extern "C" fn geteuid() -> c_int {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let raw = unsafe { syscall0(SYS_GETEUID) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  abi_u32_bits_as_c_int(raw)
}

/// C ABI entry point for `getgid`.
///
/// Returns the real group identifier of the caller.
///
/// Returns:
/// - non-negative group id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - forwards kernel-provided `getgid(2)` errno values when present.
#[unsafe(no_mangle)]
pub extern "C" fn getgid() -> c_int {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let raw = unsafe { syscall0(SYS_GETGID) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  abi_u32_bits_as_c_int(raw)
}

/// C ABI entry point for `getegid`.
///
/// Returns the effective group identifier of the caller.
///
/// Returns:
/// - non-negative effective group id on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - forwards kernel-provided `getegid(2)` errno values when present.
#[unsafe(no_mangle)]
pub extern "C" fn getegid() -> c_int {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let raw = unsafe { syscall0(SYS_GETEGID) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  abi_u32_bits_as_c_int(raw)
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

/// C ABI entry point for `lseek`.
///
/// Repositions the file offset for `fd` according to `whence`.
///
/// Returns:
/// - resulting file offset on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `fd` is not a valid open file descriptor.
/// - `EINVAL`: `whence` is invalid or the resulting offset would be negative.
/// - other kernel-provided `lseek(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn lseek(fd: c_int, offset: c_long, whence: c_int) -> c_long {
  // SAFETY: syscall number and integer arguments follow Linux x86_64 ABI.
  let raw = unsafe { syscall3(SYS_LSEEK, c_long::from(fd), offset, c_long::from(whence)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  raw
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

/// C ABI entry point for `pipe`.
///
/// Creates a unidirectional pipe and writes read/write descriptors into
/// `pipefd[0]` and `pipefd[1]`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Safety
/// - `pipefd` must point to writable storage for two `int` file descriptors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe(pipefd: *mut c_int) -> c_int {
  // SAFETY: syscall number and pointer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_PIPE, mut_ptr_arg(pipefd)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("pipe return value must fit c_int"))
}

/// C ABI entry point for `pipe2`.
///
/// Creates a unidirectional pipe and writes read/write descriptors into
/// `pipefd[0]` and `pipefd[1]`, applying Linux-specific creation `flags`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Safety
/// - `pipefd` must point to writable storage for two `int` file descriptors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe2(pipefd: *mut c_int, flags: c_int) -> c_int {
  // SAFETY: syscall number, pointer argument, and flags follow Linux x86_64 ABI.
  let raw = unsafe { syscall2(SYS_PIPE2, mut_ptr_arg(pipefd), c_long::from(flags)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("pipe2 return value must fit c_int"))
}

/// C ABI entry point for `fsync`.
///
/// Flushes all modified file data and metadata for `fd` to stable storage.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `fd` is not a valid open file descriptor.
/// - other kernel-provided `fsync(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn fsync(fd: c_int) -> c_int {
  // SAFETY: syscall number and integer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_FSYNC, c_long::from(fd)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("fsync return value must fit c_int"))
}

/// C ABI entry point for `fdatasync`.
///
/// Flushes modified file data for `fd` to stable storage.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `fd` is not a valid open file descriptor.
/// - other kernel-provided `fdatasync(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn fdatasync(fd: c_int) -> c_int {
  // SAFETY: syscall number and integer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_FDATASYNC, c_long::from(fd)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("fdatasync return value must fit c_int"))
}

/// C ABI entry point for `sync`.
///
/// Flushes filesystem buffers to stable storage.
///
/// Returns no value and preserves the calling thread's `errno`.
#[unsafe(no_mangle)]
pub extern "C" fn sync() {
  // SAFETY: syscall number follows Linux x86_64 ABI.
  let _ = unsafe { syscall0(SYS_SYNC) };
}

/// C ABI entry point for `syncfs`.
///
/// Flushes dirty data for the filesystem containing `fd` to stable storage.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Errors
/// - `EBADF`: `fd` is not a valid open file descriptor.
/// - other kernel-provided `syncfs(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn syncfs(fd: c_int) -> c_int {
  // SAFETY: syscall number and integer argument follow Linux x86_64 ABI.
  let raw = unsafe { syscall1(SYS_SYNCFS, c_long::from(fd)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  c_int::try_from(raw).unwrap_or_else(|_| unreachable!("syncfs return value must fit c_int"))
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

/// C ABI entry point for `isatty`.
///
/// Returns whether `fd` refers to a terminal device by issuing
/// `ioctl(fd, TCGETS, ...)`.
///
/// Returns:
/// - `1` when `fd` refers to a terminal
/// - `0` when `fd` is not a terminal or the descriptor is invalid
///
/// # Errors
/// - `EBADF`: `fd` is not a valid descriptor.
/// - `ENOTTY`: `fd` does not refer to a terminal.
/// - other kernel-provided `ioctl(2)` errno values are forwarded unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn isatty(fd: c_int) -> c_int {
  let mut termios = [0_u8; KERNEL_TERMIOS_SIZE];

  // SAFETY: syscall number, integer arguments, and termios output pointer
  // follow Linux x86_64 `ioctl(TCGETS)` ABI.
  let raw = unsafe {
    syscall3(
      SYS_IOCTL,
      c_long::from(fd),
      TCGETS,
      mut_ptr_arg(termios.as_mut_ptr()),
    )
  };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return 0;
  }

  1
}

#[cfg(test)]
mod tests {
  use crate::abi::types::{c_char, c_int, c_long, size_t};

  use super::{
    _SC_CLK_TCK, _SC_NPROCESSORS_CONF, _SC_NPROCESSORS_ONLN, _SC_OPEN_MAX, _SC_PAGE_SIZE,
    _SC_PAGESIZE, HOST_NAME_MAX, gethostname, getpagesize, sysconf,
  };

  #[test]
  fn unistd_reexports_system_backed_header_apis() {
    let unistd_gethostname = gethostname as unsafe extern "C" fn(*mut c_char, size_t) -> c_int;
    let system_gethostname =
      crate::system::gethostname as unsafe extern "C" fn(*mut c_char, size_t) -> c_int;
    let unistd_getpagesize = getpagesize as extern "C" fn() -> c_int;
    let system_getpagesize = crate::system::getpagesize as extern "C" fn() -> c_int;
    let unistd_sysconf = sysconf as extern "C" fn(c_int) -> c_long;
    let system_sysconf = crate::system::sysconf as extern "C" fn(c_int) -> c_long;

    assert_eq!(
      unistd_gethostname as *const (), system_gethostname as *const (),
      "unistd::gethostname should remain the system-backed C entry point",
    );
    assert_eq!(
      unistd_getpagesize as *const (), system_getpagesize as *const (),
      "unistd::getpagesize should remain the system-backed C entry point",
    );
    assert_eq!(
      unistd_sysconf as *const (), system_sysconf as *const (),
      "unistd::sysconf should remain the system-backed C entry point",
    );
    assert_eq!(_SC_CLK_TCK, crate::system::_SC_CLK_TCK);
    assert_eq!(_SC_OPEN_MAX, crate::system::_SC_OPEN_MAX);
    assert_eq!(_SC_PAGESIZE, crate::system::_SC_PAGESIZE);
    assert_eq!(_SC_PAGE_SIZE, crate::system::_SC_PAGE_SIZE);
    assert_eq!(_SC_NPROCESSORS_CONF, crate::system::_SC_NPROCESSORS_CONF);
    assert_eq!(_SC_NPROCESSORS_ONLN, crate::system::_SC_NPROCESSORS_ONLN);
    assert_eq!(HOST_NAME_MAX, 64);
  }
}
