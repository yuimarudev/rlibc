//! C ABI conversions from strings to integer values.
//!
//! This module implements the `strto*` family used by libc-compatible callers.
//! It follows C-style pointer contracts (`nptr`, `endptr`), base autodetection,
//! and `errno` updates on invalid base / range errors.

use crate::abi::errno::{EINVAL, ERANGE};
use crate::abi::types::{c_int, c_long, c_ulong};
use crate::errno::set_errno;
use core::ffi::{c_char, c_longlong, c_ulonglong};

const MIN_BASE: c_int = 2;
const MAX_BASE: c_int = 36;

#[derive(Copy, Clone, Eq, PartialEq)]
enum ParseStatus {
  InvalidBase,
  NoConversion,
  Converted,
}

struct ParseResult {
  value: u128,
  end: *const c_char,
  status: ParseStatus,
  overflow: bool,
  negative: bool,
}

const fn is_ascii_space(byte: u8) -> bool {
  matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)
}

const fn digit_value(byte: u8) -> Option<u32> {
  match byte {
    b'0'..=b'9' => Some((byte - b'0') as u32),
    b'a'..=b'z' => Some((byte - b'a' + 10) as u32),
    b'A'..=b'Z' => Some((byte - b'A' + 10) as u32),
    _ => None,
  }
}

const fn is_valid_digit_for_base(byte: u8, base: u32) -> bool {
  match digit_value(byte) {
    Some(value) => value < base,
    None => false,
  }
}

const unsafe fn read_byte(ptr: *const c_char) -> u8 {
  // SAFETY: Callers uphold that `ptr` points into a readable C string.
  unsafe { ptr.read().cast_unsigned() }
}

const unsafe fn write_endptr(endptr: *mut *mut c_char, value: *const c_char) {
  if endptr.is_null() {
    return;
  }

  // SAFETY: Caller provided a valid pointer-to-pointer when non-null.
  unsafe {
    endptr.write(value.cast_mut());
  }
}

unsafe fn parse_unsigned_magnitude(nptr: *const c_char, base: c_int, limit: u128) -> ParseResult {
  let mut cursor = nptr;

  while is_ascii_space(unsafe { read_byte(cursor) }) {
    // SAFETY: Advancing one byte stays within the caller-provided C string.
    cursor = unsafe { cursor.add(1) };
  }

  let mut negative = false;
  let sign = unsafe { read_byte(cursor) };

  if sign == b'+' {
    // SAFETY: Advancing one byte stays within the caller-provided C string.
    cursor = unsafe { cursor.add(1) };
  } else if sign == b'-' {
    negative = true;
    // SAFETY: Advancing one byte stays within the caller-provided C string.
    cursor = unsafe { cursor.add(1) };
  }

  let mut digits_ptr = cursor;
  let resolved_base: u32;

  if base == 0 {
    let first = unsafe { read_byte(digits_ptr) };

    if first == b'0' {
      let second = unsafe { read_byte(digits_ptr.add(1)) };

      if second == b'x' || second == b'X' {
        let third = unsafe { read_byte(digits_ptr.add(2)) };

        if is_valid_digit_for_base(third, 16) {
          resolved_base = 16;
          // SAFETY: Prefix bytes were already read from this location.
          digits_ptr = unsafe { digits_ptr.add(2) };
        } else {
          resolved_base = 8;
        }
      } else {
        resolved_base = 8;
      }
    } else {
      resolved_base = 10;
    }
  } else if (MIN_BASE..=MAX_BASE).contains(&base) {
    resolved_base = u32::try_from(base).unwrap_or_else(|_| unreachable!("checked base range"));

    if resolved_base == 16 {
      let first = unsafe { read_byte(digits_ptr) };

      if first == b'0' {
        let second = unsafe { read_byte(digits_ptr.add(1)) };

        if second == b'x' || second == b'X' {
          let third = unsafe { read_byte(digits_ptr.add(2)) };

          if is_valid_digit_for_base(third, 16) {
            // SAFETY: Prefix bytes were already read from this location.
            digits_ptr = unsafe { digits_ptr.add(2) };
          }
        }
      }
    }
  } else {
    return ParseResult {
      value: 0,
      end: nptr,
      status: ParseStatus::InvalidBase,
      overflow: false,
      negative,
    };
  }

  let radix = u128::from(resolved_base);
  let mut value = 0_u128;
  let mut overflow = false;
  let mut converted = false;
  let mut scan = digits_ptr;

  loop {
    let byte = unsafe { read_byte(scan) };
    let Some(digit) = digit_value(byte) else {
      break;
    };

    if digit >= resolved_base {
      break;
    }

    converted = true;

    if !overflow {
      let digit_u128 = u128::from(digit);
      let cutoff = (limit - digit_u128) / radix;

      if value > cutoff {
        overflow = true;
        value = limit;
      } else {
        value = value * radix + digit_u128;
      }
    }

    // SAFETY: We only advance while reading a valid digit from this C string.
    scan = unsafe { scan.add(1) };
  }

  if !converted {
    return ParseResult {
      value: 0,
      end: nptr,
      status: ParseStatus::NoConversion,
      overflow: false,
      negative,
    };
  }

  ParseResult {
    value,
    end: scan,
    status: ParseStatus::Converted,
    overflow,
    negative,
  }
}

/// C ABI entry point for `strtol`.
///
/// Converts the initial part of `nptr` to a signed `long` integer using `base`
/// (`0` enables C-style autodetection). `endptr`, when non-null, receives the
/// first unparsed byte.
///
/// # Safety
/// - `nptr` must point to a readable NUL-terminated C string.
/// - If `endptr` is non-null, it must be writable for one pointer.
///
/// # Errors
/// - Sets `errno = EINVAL` when `base` is neither `0` nor `2..=36`.
///   In that case this implementation leaves `endptr` unchanged.
/// - Sets `errno = ERANGE` on overflow/underflow.
/// - Successful conversions and no-conversion paths preserve the caller's existing `errno` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtol(
  nptr: *const c_char,
  endptr: *mut *mut c_char,
  base: c_int,
) -> c_long {
  let signed_max = c_long::MAX as u128;
  let signed_magnitude_limit = signed_max + 1;
  let parsed = unsafe { parse_unsigned_magnitude(nptr, base, signed_magnitude_limit) };

  if matches!(parsed.status, ParseStatus::InvalidBase) {
    set_errno(EINVAL);

    return 0;
  }

  unsafe { write_endptr(endptr, parsed.end) };

  let positive_overflow = !parsed.negative && parsed.value > signed_max;

  if parsed.overflow || positive_overflow {
    set_errno(ERANGE);

    return if parsed.negative {
      c_long::MIN
    } else {
      c_long::MAX
    };
  }

  if !matches!(parsed.status, ParseStatus::Converted) {
    return 0;
  }

  if parsed.negative {
    if parsed.value == signed_magnitude_limit {
      return c_long::MIN;
    }

    let magnitude =
      i128::try_from(parsed.value).unwrap_or_else(|_| unreachable!("magnitude fits i128"));
    let signed_value = -magnitude;

    return c_long::try_from(signed_value).unwrap_or_else(|_| unreachable!("value fits c_long"));
  }

  c_long::try_from(parsed.value).unwrap_or_else(|_| unreachable!("value fits c_long"))
}

/// C ABI entry point for `strtoll`.
///
/// Converts the initial part of `nptr` to a signed `long long` integer using
/// `base` (`0` enables C-style autodetection).
///
/// # Safety
/// - `nptr` must point to a readable NUL-terminated C string.
/// - If `endptr` is non-null, it must be writable for one pointer.
///
/// # Errors
/// - Sets `errno = EINVAL` when `base` is neither `0` nor `2..=36`.
///   In that case this implementation leaves `endptr` unchanged.
/// - Sets `errno = ERANGE` on overflow/underflow.
/// - Successful conversions and no-conversion paths preserve the caller's existing `errno` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoll(
  nptr: *const c_char,
  endptr: *mut *mut c_char,
  base: c_int,
) -> c_longlong {
  let signed_max = c_longlong::MAX as u128;
  let signed_magnitude_limit = signed_max + 1;
  let parsed = unsafe { parse_unsigned_magnitude(nptr, base, signed_magnitude_limit) };

  if matches!(parsed.status, ParseStatus::InvalidBase) {
    set_errno(EINVAL);

    return 0;
  }

  unsafe { write_endptr(endptr, parsed.end) };

  let positive_overflow = !parsed.negative && parsed.value > signed_max;

  if parsed.overflow || positive_overflow {
    set_errno(ERANGE);

    return if parsed.negative {
      c_longlong::MIN
    } else {
      c_longlong::MAX
    };
  }

  if !matches!(parsed.status, ParseStatus::Converted) {
    return 0;
  }

  if parsed.negative {
    if parsed.value == signed_magnitude_limit {
      return c_longlong::MIN;
    }

    let magnitude =
      i128::try_from(parsed.value).unwrap_or_else(|_| unreachable!("magnitude fits i128"));
    let signed_value = -magnitude;

    return c_longlong::try_from(signed_value)
      .unwrap_or_else(|_| unreachable!("value fits c_longlong"));
  }

  c_longlong::try_from(parsed.value).unwrap_or_else(|_| unreachable!("value fits c_longlong"))
}

/// C ABI entry point for `strtoul`.
///
/// Converts the initial part of `nptr` to an unsigned `long` integer.
/// Negative inputs follow C unsigned wraparound semantics.
///
/// # Safety
/// - `nptr` must point to a readable NUL-terminated C string.
/// - If `endptr` is non-null, it must be writable for one pointer.
///
/// # Errors
/// - Sets `errno = EINVAL` when `base` is neither `0` nor `2..=36`.
///   In that case this implementation leaves `endptr` unchanged.
/// - Sets `errno = ERANGE` on overflow.
/// - Successful conversions and no-conversion paths preserve the caller's existing `errno` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoul(
  nptr: *const c_char,
  endptr: *mut *mut c_char,
  base: c_int,
) -> c_ulong {
  let unsigned_max = u128::from(c_ulong::MAX);
  let parsed = unsafe { parse_unsigned_magnitude(nptr, base, unsigned_max) };

  if matches!(parsed.status, ParseStatus::InvalidBase) {
    set_errno(EINVAL);

    return 0;
  }

  unsafe { write_endptr(endptr, parsed.end) };

  if parsed.overflow {
    set_errno(ERANGE);

    return c_ulong::MAX;
  }

  if !matches!(parsed.status, ParseStatus::Converted) {
    return 0;
  }

  let value =
    c_ulong::try_from(parsed.value).unwrap_or_else(|_| unreachable!("value fits c_ulong"));

  if parsed.negative {
    value.wrapping_neg()
  } else {
    value
  }
}

/// C ABI entry point for `strtoull`.
///
/// Converts the initial part of `nptr` to an unsigned `long long` integer.
/// Negative inputs follow C unsigned wraparound semantics.
///
/// # Safety
/// - `nptr` must point to a readable NUL-terminated C string.
/// - If `endptr` is non-null, it must be writable for one pointer.
///
/// # Errors
/// - Sets `errno = EINVAL` when `base` is neither `0` nor `2..=36`.
///   In that case this implementation leaves `endptr` unchanged.
/// - Sets `errno = ERANGE` on overflow.
/// - Successful conversions and no-conversion paths preserve the caller's existing `errno` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoull(
  nptr: *const c_char,
  endptr: *mut *mut c_char,
  base: c_int,
) -> c_ulonglong {
  let unsigned_max = u128::from(c_ulonglong::MAX);
  let parsed = unsafe { parse_unsigned_magnitude(nptr, base, unsigned_max) };

  if matches!(parsed.status, ParseStatus::InvalidBase) {
    set_errno(EINVAL);

    return 0;
  }

  unsafe { write_endptr(endptr, parsed.end) };

  if parsed.overflow {
    set_errno(ERANGE);

    return c_ulonglong::MAX;
  }

  if !matches!(parsed.status, ParseStatus::Converted) {
    return 0;
  }

  let value =
    c_ulonglong::try_from(parsed.value).unwrap_or_else(|_| unreachable!("value fits c_ulonglong"));

  if parsed.negative {
    value.wrapping_neg()
  } else {
    value
  }
}
