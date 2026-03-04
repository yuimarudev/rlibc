//! File metadata C ABI functions.
//!
//! This module provides Linux `x86_64` implementations of:
//! - `stat`
//! - `fstat`
//! - `lstat`
//! - `fstatat`
//!
//! ABI notes:
//! - `Stat` and `Timespec` are laid out to match Linux `x86_64` userspace
//!   expectations for the `newfstatat`/`fstat` syscall family.
//! - Wrappers return `0` on success and `-1` on failure while updating
//!   thread-local `errno`.

use crate::abi::types::{c_int, c_long, c_uint, c_ulong};
use crate::errno::set_errno;
use crate::syscall::{decode_raw, syscall2, syscall4};
use core::ffi::c_char;

const SYS_FSTAT: c_long = 5;
const SYS_NEWFSTATAT: c_long = 262;
/// Special directory file descriptor value that means "use current working directory".
pub const AT_FDCWD: c_int = -100;
/// `fstatat` flag that requests link metadata instead of following symlinks.
pub const AT_SYMLINK_NOFOLLOW: c_int = 0x100;
/// `fstatat` flag that allows `path` to be an empty string and uses `fd` directly.
///
/// On Linux, this is valid only when `fd` refers to an open file descriptor and
/// `path` points to a NUL-terminated empty string (`""`).
pub const AT_EMPTY_PATH: c_int = 0x1000;

/// Signed Unix timestamp component type for this target ABI.
pub type TimeT = c_long;

/// File mode bitfield type for this target ABI.
pub type ModeT = c_uint;

/// POSIX timespec used by `Stat` timestamp fields.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct Timespec {
  /// Seconds since Unix epoch.
  pub tv_sec: TimeT,
  /// Nanoseconds offset in `[0, 1_000_000_000)`.
  pub tv_nsec: c_long,
}

/// Linux `x86_64` file metadata layout used by `stat` syscall family.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct Stat {
  /// ID of device containing file.
  pub st_dev: c_ulong,
  /// Inode number.
  pub st_ino: c_ulong,
  /// Number of hard links.
  pub st_nlink: c_ulong,
  /// File type and mode bits.
  pub st_mode: ModeT,
  /// User ID of owner.
  pub st_uid: c_uint,
  /// Group ID of owner.
  pub st_gid: c_uint,
  /// Padding field used by the ABI.
  pub pad0: c_int,
  /// Device ID (if special file).
  pub st_rdev: c_ulong,
  /// Total size, in bytes.
  pub st_size: c_long,
  /// Preferred I/O block size.
  pub st_blksize: c_long,
  /// Number of 512B blocks allocated.
  pub st_blocks: c_long,
  /// Last access time.
  pub st_atim: Timespec,
  /// Last modification time.
  pub st_mtim: Timespec,
  /// Last status change time.
  pub st_ctim: Timespec,
  /// Reserved for future ABI extensions.
  pub glibc_reserved: [c_long; 3],
}

fn syscall_status(raw: c_long) -> c_int {
  let raw_isize = isize::try_from(raw)
    .unwrap_or_else(|_| unreachable!("c_long must fit into isize on x86_64 Linux"));

  match decode_raw(raw_isize) {
    Ok(_) => 0,
    Err(errno_value) => {
      set_errno(errno_value);
      -1
    }
  }
}

fn ptr_to_sys_arg<T>(ptr: *const T) -> c_long {
  c_long::try_from(ptr.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit into c_long on x86_64 Linux"))
}

fn ptr_to_mut_sys_arg<T>(ptr: *mut T) -> c_long {
  ptr_to_sys_arg(ptr.cast_const())
}

/// C ABI entry point for `fstat`.
///
/// Writes metadata for `fd` into `stat_buf`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// - `stat_buf` must be writable for one `Stat` value.
/// - `fd` must be a valid open file descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat(fd: c_int, stat_buf: *mut Stat) -> c_int {
  // SAFETY: syscall number/arguments are passed per Linux `x86_64` syscall ABI.
  let raw = unsafe { syscall2(SYS_FSTAT, c_long::from(fd), ptr_to_mut_sys_arg(stat_buf)) };

  syscall_status(raw)
}

/// C ABI entry point for `fstatat`.
///
/// Writes metadata for `path` resolved relative to `fd` (or current working
/// directory when `fd == AT_FDCWD`) into `stat_buf`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// - `path` must point to a valid NUL-terminated string.
/// - `stat_buf` must be writable for one `Stat` value.
/// - `flag` must contain only supported Linux `AT_*` bits for this syscall.
/// - When `flag` includes [`AT_EMPTY_PATH`], `path` may be `""` and metadata is
///   read from `fd` directly.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstatat(
  fd: c_int,
  path: *const c_char,
  stat_buf: *mut Stat,
  flag: c_int,
) -> c_int {
  // SAFETY: syscall number/arguments are passed per Linux `x86_64` syscall ABI.
  let raw = unsafe {
    syscall4(
      SYS_NEWFSTATAT,
      c_long::from(fd),
      ptr_to_sys_arg(path),
      ptr_to_mut_sys_arg(stat_buf),
      c_long::from(flag),
    )
  };

  syscall_status(raw)
}

/// C ABI entry point for `stat`.
///
/// Equivalent to `fstatat(AT_FDCWD, path, stat_buf, 0)`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// - `path` must point to a valid NUL-terminated string.
/// - `stat_buf` must be writable for one `Stat` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stat(path: *const c_char, stat_buf: *mut Stat) -> c_int {
  // SAFETY: delegated to `fstatat` with equivalent ABI contract.
  unsafe { fstatat(AT_FDCWD, path, stat_buf, 0) }
}

/// C ABI entry point for `lstat`.
///
/// Equivalent to `fstatat(AT_FDCWD, path, stat_buf, AT_SYMLINK_NOFOLLOW)`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno`
///
/// # Safety
/// - `path` must point to a valid NUL-terminated string.
/// - `stat_buf` must be writable for one `Stat` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lstat(path: *const c_char, stat_buf: *mut Stat) -> c_int {
  // SAFETY: delegated to `fstatat` with equivalent ABI contract.
  unsafe { fstatat(AT_FDCWD, path, stat_buf, AT_SYMLINK_NOFOLLOW) }
}
