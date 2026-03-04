//! Syscall-related definitions.
//!
//! This module currently contains shared helpers for decoding Linux-style raw
//! syscall return values into Rust results.

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
pub mod x86_64;

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
pub use x86_64::{syscall0, syscall1, syscall2, syscall3, syscall4, syscall5, syscall6};

/// Decodes a raw syscall return value.
///
/// Linux syscalls return:
/// - values in `-4095..=-1` as `-errno` failures
/// - all other register values as successful results
///
/// This helper applies that ABI contract and converts it into `Result`.
/// Successful values preserve the original register bits via `raw as usize`.
///
/// # Errors
/// Returns `Err(errno)` only when `raw` is in Linux's errno window
/// `-4095..=-1`.
#[must_use = "callers must handle decoded syscall status"]
pub fn decode_raw(raw: isize) -> Result<usize, i32> {
  const MAX_ERRNO: isize = 4095;

  if (-MAX_ERRNO..=-1).contains(&raw) {
    let errno = i32::try_from(-raw).unwrap_or(i32::MAX);

    return Err(errno);
  }

  Ok(raw.cast_unsigned())
}

#[cfg(test)]
mod tests {
  use super::decode_raw;

  #[test]
  fn decode_raw_negative_libc_errno_becomes_err_errno() {
    let libc_errno: isize = 22;

    assert_eq!(decode_raw(-libc_errno), Err(22));
  }

  #[test]
  fn decode_raw_non_negative_value_becomes_ok_value() {
    assert_eq!(decode_raw(7), Ok(7));
  }

  #[test]
  fn decode_raw_zero_becomes_ok_zero() {
    assert_eq!(decode_raw(0), Ok(0));
  }

  #[test]
  fn decode_raw_isize_max_becomes_ok_usize_value() {
    let expected = usize::try_from(isize::MAX).expect("isize::MAX should fit into usize");

    assert_eq!(decode_raw(isize::MAX), Ok(expected));
  }

  #[test]
  fn decode_raw_negative_one_becomes_err_one() {
    assert_eq!(decode_raw(-1), Err(1));
  }

  #[test]
  fn decode_raw_linux_errno_upper_bound_becomes_err_4095() {
    assert_eq!(decode_raw(-4095), Err(4095));
  }

  #[test]
  fn decode_raw_negative_value_just_below_linux_errno_range_is_success() {
    let raw = -4096_isize;

    assert_eq!(decode_raw(raw), Ok(raw.cast_unsigned()));
  }

  #[test]
  fn decode_raw_negative_i32_max_is_success_value() {
    let raw_errno = isize::try_from(i32::MAX).expect("i32::MAX should fit into isize");

    assert_eq!(decode_raw(-raw_errno), Ok((-raw_errno).cast_unsigned()));
  }

  #[test]
  fn decode_raw_negative_value_just_above_i32_max_is_success_value() {
    let above_i32_max = i64::from(i32::MAX) + 1;

    if let Ok(raw_errno) = isize::try_from(above_i32_max) {
      assert_eq!(decode_raw(-raw_errno), Ok((-raw_errno).cast_unsigned()));
    }
  }

  #[test]
  fn decode_raw_negative_isize_max_is_success_value() {
    assert_eq!(decode_raw(-isize::MAX), Ok((-isize::MAX).cast_unsigned()));
  }

  #[test]
  fn decode_raw_isize_min_is_success_value() {
    assert_eq!(decode_raw(isize::MIN), Ok(isize::MIN.cast_unsigned()));
  }
}
