//! String-related C ABI functions.
//!
//! These functions mirror `libc`-style byte-string behavior. Lengths are
//! measured in bytes, not Unicode scalar values.

use core::ffi::c_char;

/// C ABI entry point for `strlen`.
///
/// Returns the number of non-NUL bytes before the first NUL byte.
/// This return value is the computed length and should be consumed by callers.
///
/// # Safety
/// - `s` must point to a valid NUL-terminated byte sequence.
/// - Passing a pointer without a terminating NUL byte is undefined behavior.
#[unsafe(no_mangle)]
#[must_use]
pub const unsafe extern "C" fn strlen(s: *const c_char) -> usize {
  let mut len = 0_usize;

  // SAFETY: C caller must provide a valid NUL-terminated byte sequence.
  unsafe {
    while s.add(len).read() != 0 {
      len += 1;
    }
  }

  len
}

/// C ABI entry point for `strnlen`.
///
/// Returns the number of non-NUL bytes before the first NUL byte, but at most
/// `n`. When `n == 0`, this function returns `0` immediately.
/// This return value is the computed bounded length and should be consumed by
/// callers.
///
/// # Safety
/// - At most `n` bytes are read.
/// - If `n == 0`, no memory is accessed and `s` may be null.
/// - For `n > 0`, caller must provide at least `n` readable bytes unless a NUL
///   appears earlier.
#[unsafe(no_mangle)]
#[must_use]
pub const unsafe extern "C" fn strnlen(s: *const c_char, n: usize) -> usize {
  if n == 0 {
    return 0;
  }

  let mut len = 0_usize;

  // SAFETY: C caller must provide a valid readable byte sequence up to `n` bytes.
  unsafe {
    while len < n && s.add(len).read() != 0 {
      len += 1;
    }
  }

  len
}

#[cfg(test)]
mod tests {
  use core::ptr;

  use super::{strlen, strnlen};

  #[test]
  fn strlen_counts_until_nul() {
    let bytes = b"hello\0world";
    let ptr = bytes.as_ptr().cast();

    // SAFETY: `bytes` is NUL-terminated.
    let actual = unsafe { strlen(ptr) };

    assert_eq!(actual, 5);
  }

  #[test]
  fn strnlen_stops_at_nul_before_limit() {
    let bytes = b"abc\0def";
    let ptr = bytes.as_ptr().cast();

    // SAFETY: We only request reads up to a valid in-bounds limit.
    let actual = unsafe { strnlen(ptr, 10) };

    assert_eq!(actual, 3);
  }

  #[test]
  fn strnlen_stops_at_limit_when_no_nul_before_n() {
    let bytes = b"abcdef";
    let ptr = bytes.as_ptr().cast();

    // SAFETY: We only request reads up to a valid in-bounds limit.
    let actual = unsafe { strnlen(ptr, 4) };

    assert_eq!(actual, 4);
  }

  #[test]
  fn strnlen_returns_zero_when_limit_is_zero() {
    let bytes = b"abcdef";
    let ptr = bytes.as_ptr().cast();

    // SAFETY: `n == 0` does not dereference the pointer.
    let actual = unsafe { strnlen(ptr, 0) };

    assert_eq!(actual, 0);
  }

  #[test]
  fn strnlen_allows_null_pointer_when_limit_is_zero() {
    // SAFETY: `n == 0` does not dereference the pointer.
    let actual = unsafe { strnlen(ptr::null(), 0) };

    assert_eq!(actual, 0);
  }
}
