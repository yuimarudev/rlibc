//! Math APIs with `errno`/`fenv` integration.
//!
//! This module exports a small baseline set of libm-style C ABI entry points:
//! - `sqrt`
//! - `log`
//! - `exp`
//!
//! Error signaling model:
//! - Domain errors set `errno = EDOM` and raise `FE_INVALID`.
//! - Pole/range errors set `errno = ERANGE` and raise the corresponding
//!   floating-point exception. Overflow/underflow paths for `exp` also raise
//!   `FE_INEXACT` to reflect IEEE-754 inexact-result signaling.
//! - Successful operations keep `errno` unchanged and do not raise new
//!   exception flags.

use crate::abi::errno::{EDOM, ERANGE};
use crate::abi::types::c_int;
use crate::errno::set_errno;
use crate::fenv::{FE_DIVBYZERO, FE_INEXACT, FE_INVALID, FE_OVERFLOW, FE_UNDERFLOW, feraiseexcept};

fn raise_math_error(errno_value: c_int, except_flag: c_int) {
  set_errno(errno_value);

  let _status = feraiseexcept(except_flag);
}

fn exp_reports_underflow(x: f64, result: f64) -> bool {
  x.is_finite() && x < 0.0 && (result == 0.0 || result.is_subnormal())
}

/// C ABI entry point for `sqrt`.
///
/// Computes the square root of `x`.
///
/// # Errors
/// - For negative inputs (`x < 0`, including `-inf`), returns `NaN`, sets
///   `errno = EDOM`, and raises `FE_INVALID`.
/// - For signed zero inputs (`+0.0` / `-0.0`), returns the corresponding zero
///   and preserves `errno`.
/// - For all other inputs, returns the mathematical result and preserves
///   `errno`.
#[unsafe(no_mangle)]
pub extern "C" fn sqrt(x: f64) -> f64 {
  if x < 0.0 {
    raise_math_error(EDOM, FE_INVALID);

    return f64::NAN;
  }

  libm::sqrt(x)
}

/// C ABI entry point for `log`.
///
/// Computes the natural logarithm of `x`.
///
/// # Errors
/// - For `x == 0.0` (including signed zero), returns negative infinity, sets
///   `errno = ERANGE`, and raises `FE_DIVBYZERO`.
/// - For negative inputs (`x < 0`, including `-inf`), returns `NaN`, sets
///   `errno = EDOM`, and raises `FE_INVALID`.
/// - For all other inputs, returns the mathematical result and preserves
///   `errno`.
#[unsafe(no_mangle)]
pub extern "C" fn log(x: f64) -> f64 {
  if x == 0.0 {
    raise_math_error(ERANGE, FE_DIVBYZERO);

    return f64::NEG_INFINITY;
  }

  if x < 0.0 {
    raise_math_error(EDOM, FE_INVALID);

    return f64::NAN;
  }

  libm::log(x)
}

/// C ABI entry point for `exp`.
///
/// Computes `e^x`.
///
/// # Errors
/// - For finite `x` where the result overflows to infinity, returns `+inf`,
///   sets `errno = ERANGE`, and raises `FE_OVERFLOW | FE_INEXACT`.
/// - For finite negative `x` where the result underflows into the tiny range
///   (subnormal or zero), returns the computed value, sets `errno = ERANGE`,
///   and raises `FE_UNDERFLOW | FE_INEXACT`. Tiny but still normal results do
///   not trigger range-error reporting.
/// - For non-finite inputs (`NaN`, `+inf`, `-inf`), returns libm-consistent
///   results and preserves `errno`.
/// - For all other inputs, returns the mathematical result and preserves
///   `errno`.
#[unsafe(no_mangle)]
pub extern "C" fn exp(x: f64) -> f64 {
  let result = libm::exp(x);

  if x.is_finite() && result.is_infinite() {
    raise_math_error(ERANGE, FE_OVERFLOW | FE_INEXACT);

    return f64::INFINITY;
  }

  if exp_reports_underflow(x, result) {
    raise_math_error(ERANGE, FE_UNDERFLOW | FE_INEXACT);

    return result;
  }

  result
}
