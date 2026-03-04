//! Linux resource-limit C ABI interfaces.
//!
//! This module implements Linux `x86_64` wrappers for:
//! - `getrlimit`
//! - `setrlimit`
//! - `prlimit64`
//!
//! Wrappers follow libc-style status contracts:
//! - success: `0`
//! - failure: `-1` and thread-local `errno` set

use crate::abi::errno::EFAULT;
use crate::abi::types::{c_int, c_long, c_ulong};
use crate::errno::set_errno;
use crate::syscall::syscall4;
use core::ptr;

const SYS_PRLIMIT64: c_long = 302;
/// `RLIMIT_CPU` selector.
pub const RLIMIT_CPU: c_int = 0;
/// `RLIMIT_FSIZE` selector.
pub const RLIMIT_FSIZE: c_int = 1;
/// `RLIMIT_DATA` selector.
pub const RLIMIT_DATA: c_int = 2;
/// `RLIMIT_STACK` selector.
pub const RLIMIT_STACK: c_int = 3;
/// `RLIMIT_CORE` selector.
pub const RLIMIT_CORE: c_int = 4;
/// `RLIMIT_RSS` selector.
pub const RLIMIT_RSS: c_int = 5;
/// `RLIMIT_NPROC` selector.
pub const RLIMIT_NPROC: c_int = 6;
/// `RLIMIT_NOFILE` selector.
pub const RLIMIT_NOFILE: c_int = 7;
/// `RLIMIT_MEMLOCK` selector.
pub const RLIMIT_MEMLOCK: c_int = 8;
/// `RLIMIT_AS` selector.
pub const RLIMIT_AS: c_int = 9;
/// `RLIMIT_LOCKS` selector.
pub const RLIMIT_LOCKS: c_int = 10;
/// `RLIMIT_SIGPENDING` selector.
pub const RLIMIT_SIGPENDING: c_int = 11;
/// `RLIMIT_MSGQUEUE` selector.
pub const RLIMIT_MSGQUEUE: c_int = 12;
/// `RLIMIT_NICE` selector.
pub const RLIMIT_NICE: c_int = 13;
/// `RLIMIT_RTPRIO` selector.
pub const RLIMIT_RTPRIO: c_int = 14;
/// `RLIMIT_RTTIME` selector.
pub const RLIMIT_RTTIME: c_int = 15;
/// Sentinel value that represents "no enforced limit".
pub const RLIM_INFINITY: c_ulong = c_ulong::MAX;

/// Resource limit scalar type for Linux `x86_64`.
pub type rlim_t = c_ulong;

/// Linux `struct rlimit` layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RLimit {
  /// Current (soft) limit.
  pub rlim_cur: rlim_t,
  /// Maximum (hard) limit.
  pub rlim_max: rlim_t,
}

fn ptr_arg<T>(ptr: *const T) -> c_long {
  // Preserve all pointer bits (including high-bit invalid user pointers) so
  // the kernel can return `EFAULT` instead of this wrapper panicking.
  c_long::from_ne_bytes(ptr.addr().to_ne_bytes())
}

fn mut_ptr_arg<T>(ptr: *mut T) -> c_long {
  ptr_arg(ptr.cast_const())
}

fn errno_from_raw(raw: c_long) -> c_int {
  c_int::try_from(-raw).unwrap_or(c_int::MAX)
}

/// C ABI entry point for `prlimit64`.
///
/// Reads and/or updates resource limits for `pid` and `resource`.
///
/// C contract:
/// - when `new_limit` is non-null, the new soft/hard limit pair is applied
/// - when `old_limit` is non-null, current limits are written to that buffer
/// - when both pointers are null, the call is a no-op validity/permission check
/// - `pid == 0` targets the calling process
///
/// # Safety
/// - If non-null, `new_limit` must be readable as one [`RLimit`].
/// - If non-null, `old_limit` must be writable as one [`RLimit`].
/// - `resource` must be a valid Linux `RLIMIT_*` selector.
///
/// # Errors
/// Returns `-1` and sets `errno` to kernel-provided failures (for example
/// `EINVAL`, `EPERM`, `EFAULT`, `ESRCH`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn prlimit64(
  pid: c_int,
  resource: c_int,
  new_limit: *const RLimit,
  old_limit: *mut RLimit,
) -> c_int {
  // SAFETY: syscall number and argument registers match Linux x86_64 ABI.
  let raw = unsafe {
    syscall4(
      SYS_PRLIMIT64,
      c_long::from(pid),
      c_long::from(resource),
      ptr_arg(new_limit),
      mut_ptr_arg(old_limit),
    )
  };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  0
}

/// C ABI entry point for `getrlimit`.
///
/// Reads current limits for `resource` in the calling process into `rlim`.
///
/// # Safety
/// - `rlim` must be writable as one [`RLimit`].
/// - `resource` must be a valid Linux `RLIMIT_*` selector.
///
/// # Errors
/// Returns `-1` and sets:
/// - `EFAULT` when `rlim` is null.
/// - kernel-provided failures for syscall errors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getrlimit(resource: c_int, rlim: *mut RLimit) -> c_int {
  if rlim.is_null() {
    set_errno(EFAULT);

    return -1;
  }

  // SAFETY: delegated to `prlimit64` with `pid == 0` and no write request.
  unsafe { prlimit64(0, resource, ptr::null(), rlim) }
}

/// C ABI entry point for `setrlimit`.
///
/// Applies new limits for `resource` in the calling process from `rlim`.
///
/// # Safety
/// - `rlim` must be readable as one [`RLimit`].
/// - `resource` must be a valid Linux `RLIMIT_*` selector.
///
/// # Errors
/// Returns `-1` and sets:
/// - `EFAULT` when `rlim` is null.
/// - kernel-provided failures for syscall errors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setrlimit(resource: c_int, rlim: *const RLimit) -> c_int {
  if rlim.is_null() {
    set_errno(EFAULT);

    return -1;
  }

  // SAFETY: delegated to `prlimit64` with `pid == 0` and no readback buffer.
  unsafe { prlimit64(0, resource, rlim, ptr::null_mut()) }
}

#[cfg(test)]
mod tests {
  use super::{RLimit, ptr_arg};
  use core::ptr::with_exposed_provenance;

  #[test]
  fn ptr_arg_accepts_high_bit_addresses_without_panicking() {
    let high_bit_ptr = with_exposed_provenance::<RLimit>(usize::MAX);
    let encoded = ptr_arg(high_bit_ptr);

    assert_eq!(encoded, -1);
  }
}
