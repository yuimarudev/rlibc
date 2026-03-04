//! C ABI wrappers for decimal ASCII-to-integer conversion.
//!
//! This module provides `atoi`, `atol`, and `atoll` as thin wrappers over
//! this crate's `strtol`/`strtoll` entry points with fixed base-10 parsing.
//! The wrappers intentionally preserve `errno` when no conversion is possible
//! (`endptr == nptr`), matching this repository's policy for invalid input.

use crate::abi::errno::EINVAL;
use crate::abi::types::{c_int, c_long};
use crate::errno::{__errno_location, set_errno};
use crate::stdlib::conv::{strtol as conv_strtol, strtoll as conv_strtoll};
use core::ffi::{c_char, c_longlong};

const DECIMAL_BASE: c_int = 10;

fn restore_errno_if_no_conversion(nptr: *const c_char, endptr: *mut c_char, saved_errno: c_int) {
  if core::ptr::eq(endptr.cast_const(), nptr) {
    // SAFETY: `__errno_location` returns a valid thread-local errno pointer for this thread.
    unsafe {
      __errno_location().write(saved_errno);
    }
  }
}

unsafe fn parse_decimal_long(nptr: *const c_char) -> c_long {
  if nptr.is_null() {
    set_errno(EINVAL);

    return 0;
  }

  let mut endptr = core::ptr::null_mut();

  // SAFETY: `__errno_location` returns a valid thread-local errno pointer for this thread.
  let saved_errno = unsafe { __errno_location().read() };
  // SAFETY: Delegates parsing contract to this crate's C ABI `strtol`.
  let parsed = unsafe { conv_strtol(nptr, core::ptr::addr_of_mut!(endptr), DECIMAL_BASE) };

  restore_errno_if_no_conversion(nptr, endptr, saved_errno);

  parsed
}

unsafe fn parse_decimal_long_long(nptr: *const c_char) -> c_longlong {
  if nptr.is_null() {
    set_errno(EINVAL);

    return 0;
  }

  let mut endptr = core::ptr::null_mut();

  // SAFETY: `__errno_location` returns a valid thread-local errno pointer for this thread.
  let saved_errno = unsafe { __errno_location().read() };
  // SAFETY: Delegates parsing contract to this crate's C ABI `strtoll`.
  let parsed = unsafe { conv_strtoll(nptr, core::ptr::addr_of_mut!(endptr), DECIMAL_BASE) };

  restore_errno_if_no_conversion(nptr, endptr, saved_errno);

  parsed
}

fn c_long_to_c_int_wrapping(value: c_long) -> c_int {
  let modulus = 1_i128 << c_int::BITS;
  let sign_bit = 1_i128 << (c_int::BITS - 1);
  let wrapped = i128::from(value).rem_euclid(modulus);
  let signed = if wrapped >= sign_bit {
    wrapped - modulus
  } else {
    wrapped
  };

  c_int::try_from(signed)
    .unwrap_or_else(|_| unreachable!("wrapped c_long value must fit into c_int"))
}

/// C ABI entry point for `atoi`.
///
/// Parses a leading decimal integer from `nptr` using `strtol(nptr, &endptr, 10)`
/// and converts the resulting `long` into `int` with C-style narrowing semantics.
///
/// Input/output contract:
/// - Accepts optional leading ASCII whitespace and sign, as handled by `strtol`.
/// - Stops conversion at the first non-digit.
/// - Returns `0` when no conversion is possible.
/// - On `endptr == nptr` (no conversion), restores `errno` to its entry value.
///
/// # Errors
/// - Sets `errno = EINVAL` and returns `0` when `nptr` is null.
/// - On range errors from delegated `strtol`, `errno` is left as `ERANGE`.
/// - On no conversion (`endptr == nptr`), this wrapper restores `errno` to its
///   entry value.
///
/// # Safety
/// - `nptr` may be null; this wrapper then returns `0` and sets `errno = EINVAL`.
/// - For non-null input, `nptr` must point to a readable NUL-terminated byte string
///   accepted by `strtol`.
/// - Passing an invalid or non-terminated non-null pointer is undefined behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoi(nptr: *const c_char) -> c_int {
  // SAFETY: Caller upholds the C string pointer contract required by `strtol`.
  let parsed = unsafe { parse_decimal_long(nptr) };

  c_long_to_c_int_wrapping(parsed)
}

/// C ABI entry point for `atol`.
///
/// Parses a leading decimal integer from `nptr` via `strtol(nptr, &endptr, 10)`.
///
/// Input/output contract:
/// - Parsing behavior follows `strtol` with base 10.
/// - Returns `0` when no conversion is possible.
/// - On `endptr == nptr` (no conversion), restores `errno` to its entry value.
///
/// # Errors
/// - Sets `errno = EINVAL` and returns `0` when `nptr` is null.
/// - On range errors from delegated `strtol`, `errno` is left as `ERANGE`.
/// - On no conversion (`endptr == nptr`), this wrapper restores `errno` to its
///   entry value.
///
/// # Safety
/// - `nptr` may be null; this wrapper then returns `0` and sets `errno = EINVAL`.
/// - For non-null input, `nptr` must point to a readable NUL-terminated byte string
///   accepted by `strtol`.
/// - Passing an invalid or non-terminated non-null pointer is undefined behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atol(nptr: *const c_char) -> c_long {
  // SAFETY: Caller upholds the C string pointer contract required by `strtol`.
  unsafe { parse_decimal_long(nptr) }
}

/// C ABI entry point for `atoll`.
///
/// Parses a leading decimal integer from `nptr` via `strtoll(nptr, &endptr, 10)`.
///
/// Input/output contract:
/// - Parsing behavior follows `strtoll` with base 10.
/// - Returns `0` when no conversion is possible.
/// - On `endptr == nptr` (no conversion), restores `errno` to its entry value.
///
/// # Errors
/// - Sets `errno = EINVAL` and returns `0` when `nptr` is null.
/// - On range errors from delegated `strtoll`, `errno` is left as `ERANGE`.
/// - On no conversion (`endptr == nptr`), this wrapper restores `errno` to its
///   entry value.
///
/// # Safety
/// - `nptr` may be null; this wrapper then returns `0` and sets `errno = EINVAL`.
/// - For non-null input, `nptr` must point to a readable NUL-terminated byte string
///   accepted by `strtoll`.
/// - Passing an invalid or non-terminated non-null pointer is undefined behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoll(nptr: *const c_char) -> c_longlong {
  // SAFETY: Caller upholds the C string pointer contract required by `strtoll`.
  unsafe { parse_decimal_long_long(nptr) }
}
