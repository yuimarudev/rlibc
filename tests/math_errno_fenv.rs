use core::ffi::c_int;
use rlibc::abi::errno::{EDOM, ERANGE};
use rlibc::errno::__errno_location;
use rlibc::fenv::{
  FE_ALL_EXCEPT, FE_DIVBYZERO, FE_INEXACT, FE_INVALID, FE_OVERFLOW, FE_UNDERFLOW, feclearexcept,
  feraiseexcept, fetestexcept,
};
use rlibc::math::{exp, log, sqrt};
use std::thread;

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns valid thread-local errno storage.
  unsafe { *__errno_location() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns valid writable thread-local errno storage.
  unsafe {
    *__errno_location() = value;
  }
}

fn clear_all_excepts() {
  let clear_status = feclearexcept(FE_ALL_EXCEPT);

  assert_eq!(clear_status, 0, "feclearexcept must succeed");
}

fn assert_f64_eq(actual: f64, expected: f64) {
  assert!(
    (actual - expected).abs() <= f64::EPSILON,
    "expected {expected}, got {actual}",
  );
}

#[test]
fn sqrt_negative_sets_errno_and_invalid_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = sqrt(-1.0);

  assert!(result.is_nan());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
}

#[test]
fn sqrt_negative_does_not_raise_divbyzero_overflow_or_underflow() {
  clear_all_excepts();
  write_errno(0);

  let result = sqrt(-1.0);

  assert!(result.is_nan());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn sqrt_negative_infinity_sets_errno_and_invalid_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = sqrt(f64::NEG_INFINITY);

  assert!(result.is_nan());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
}

#[test]
fn sqrt_negative_infinity_does_not_raise_divbyzero_overflow_or_underflow() {
  clear_all_excepts();
  write_errno(0);

  let result = sqrt(f64::NEG_INFINITY);

  assert!(result.is_nan());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn sqrt_negative_zero_preserves_sign_and_does_not_signal_errors() {
  clear_all_excepts();
  write_errno(ERANGE);

  let result = sqrt(-0.0);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_negative());
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
}

#[test]
fn sqrt_positive_zero_preserves_sign_and_does_not_signal_errors() {
  clear_all_excepts();
  write_errno(ERANGE);

  let result = sqrt(0.0);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
}

#[test]
fn log_zero_sets_erange_and_divbyzero_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = log(0.0);

  assert!(result.is_infinite() && result.is_sign_negative());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
}

#[test]
fn log_zero_does_not_raise_invalid_overflow_or_underflow() {
  clear_all_excepts();
  write_errno(0);

  let result = log(0.0);

  assert!(result.is_infinite() && result.is_sign_negative());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn log_negative_zero_sets_erange_and_divbyzero_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = log(-0.0);

  assert!(result.is_infinite() && result.is_sign_negative());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
}

#[test]
fn log_negative_zero_does_not_raise_invalid_overflow_or_underflow() {
  clear_all_excepts();
  write_errno(0);

  let result = log(-0.0);

  assert!(result.is_infinite() && result.is_sign_negative());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn log_negative_sets_edom_and_invalid_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = log(-1.0);

  assert!(result.is_nan());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
}

#[test]
fn log_negative_does_not_raise_divbyzero_overflow_or_underflow() {
  clear_all_excepts();
  write_errno(0);

  let result = log(-1.0);

  assert!(result.is_nan());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn log_negative_infinity_sets_edom_and_invalid_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = log(f64::NEG_INFINITY);

  assert!(result.is_nan());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
}

#[test]
fn log_negative_infinity_does_not_raise_divbyzero_overflow_or_underflow() {
  clear_all_excepts();
  write_errno(0);

  let result = log(f64::NEG_INFINITY);

  assert!(result.is_nan());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_overflow_sets_erange_and_overflow_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = exp(1000.0);

  assert!(result.is_infinite());
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn exp_overflow_does_not_raise_invalid_divbyzero_or_underflow() {
  clear_all_excepts();
  write_errno(0);

  let result = exp(1000.0);

  assert!(result.is_infinite());
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_overflow_also_raises_inexact_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = exp(1000.0);

  assert!(result.is_infinite());
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
}

#[test]
fn exp_overflow_preserves_preexisting_invalid_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_INVALID);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_INVALID), 0);

  let result = exp(1000.0);

  assert!(result.is_infinite());
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_overflow_preserves_preexisting_divbyzero_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_DIVBYZERO);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);

  let result = exp(1000.0);

  assert!(result.is_infinite());
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_overflow_preserves_preexisting_underflow_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_UNDERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);

  let result = exp(1000.0);

  assert!(result.is_infinite());
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
}

#[test]
fn exp_underflow_sets_erange_and_underflow_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = exp(-1000.0);

  assert_f64_eq(result, 0.0);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_underflow_does_not_raise_invalid_divbyzero_or_overflow() {
  clear_all_excepts();
  write_errno(0);

  let result = exp(-1000.0);

  assert_f64_eq(result, 0.0);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn exp_underflow_also_raises_inexact_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = exp(-1000.0);

  assert_f64_eq(result, 0.0);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
}

#[test]
fn exp_underflow_preserves_preexisting_invalid_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_INVALID);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_INVALID), 0);

  let result = exp(-1000.0);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn exp_underflow_preserves_preexisting_divbyzero_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_DIVBYZERO);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);

  let result = exp(-1000.0);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn exp_underflow_preserves_preexisting_overflow_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_OVERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);

  let result = exp(-1000.0);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
}

#[test]
fn exp_subnormal_underflow_sets_erange_and_underflow_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = exp(-740.0);

  assert!(
    result > 0.0,
    "expected positive subnormal result, got {result}"
  );
  assert!(result.is_sign_positive());
  assert!(result < f64::MIN_POSITIVE);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_subnormal_underflow_also_raises_inexact_exception() {
  clear_all_excepts();
  write_errno(0);

  let result = exp(-740.0);

  assert!(result.is_sign_positive());
  assert!(result < f64::MIN_POSITIVE);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn exp_subnormal_underflow_preserves_preexisting_invalid_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_INVALID);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_INVALID), 0);

  let result = exp(-740.0);

  assert!(result > 0.0);
  assert!(result.is_sign_positive());
  assert!(result < f64::MIN_POSITIVE);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn exp_subnormal_underflow_preserves_preexisting_divbyzero_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_DIVBYZERO);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);

  let result = exp(-740.0);

  assert!(result > 0.0);
  assert!(result.is_sign_positive());
  assert!(result < f64::MIN_POSITIVE);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn exp_subnormal_underflow_preserves_preexisting_overflow_and_raises_inexact() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_OVERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);

  let result = exp(-740.0);

  assert!(result > 0.0);
  assert!(result.is_sign_positive());
  assert!(result < f64::MIN_POSITIVE);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
}

#[test]
fn exp_tiny_normal_does_not_set_range_error_or_underflow() {
  clear_all_excepts();
  write_errno(EDOM);

  let result = exp(-708.0);

  assert!(result.is_finite());
  assert!(result.is_sign_positive());
  assert!(result.is_normal());
  assert_eq!(read_errno(), EDOM);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_negative_infinity_does_not_set_range_error_or_underflow() {
  clear_all_excepts();
  write_errno(EDOM);

  let result = exp(f64::NEG_INFINITY);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), EDOM);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
}

#[test]
fn exp_negative_infinity_preserves_preexisting_invalid_exception_flag() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_INVALID);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_INVALID), 0);

  let result = exp(f64::NEG_INFINITY);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_negative_infinity_preserves_preexisting_divbyzero_exception_flag() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_DIVBYZERO);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);

  let result = exp(f64::NEG_INFINITY);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_negative_infinity_preserves_preexisting_overflow_exception_flag() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_OVERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);

  let result = exp(f64::NEG_INFINITY);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_negative_infinity_preserves_preexisting_underflow_exception_flag() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_UNDERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);

  let result = exp(f64::NEG_INFINITY);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn exp_negative_infinity_preserves_preexisting_inexact_exception_flag() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_INEXACT);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_INEXACT), 0);

  let result = exp(f64::NEG_INFINITY);

  assert_f64_eq(result, 0.0);
  assert!(result.is_sign_positive());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_negative_zero_preserves_errno_and_does_not_raise_exceptions() {
  clear_all_excepts();
  write_errno(ERANGE);

  let result = exp(-0.0);

  assert_f64_eq(result, 1.0);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
}

#[test]
fn exp_positive_infinity_preserves_preexisting_invalid_exception_flag() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_INVALID);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_INVALID), 0);

  let result = exp(f64::INFINITY);

  assert!(result.is_infinite() && result.is_sign_positive());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn exp_positive_infinity_preserves_preexisting_divbyzero_exception_flag() {
  clear_all_excepts();
  write_errno(EDOM);

  let raise_status = feraiseexcept(FE_DIVBYZERO);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);

  let result = exp(f64::INFINITY);

  assert!(result.is_infinite() && result.is_sign_positive());
  assert_eq!(read_errno(), EDOM);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn positive_infinity_inputs_do_not_set_errno_or_exceptions() {
  clear_all_excepts();
  write_errno(ERANGE);

  let sqrt_result = sqrt(f64::INFINITY);
  let log_result = log(f64::INFINITY);
  let exp_result = exp(f64::INFINITY);

  assert!(sqrt_result.is_infinite() && sqrt_result.is_sign_positive());
  assert!(log_result.is_infinite() && log_result.is_sign_positive());
  assert!(exp_result.is_infinite() && exp_result.is_sign_positive());
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
}

#[test]
fn nan_inputs_do_not_set_errno_or_exceptions() {
  clear_all_excepts();
  write_errno(ERANGE);

  let sqrt_result = sqrt(f64::NAN);
  let log_result = log(f64::NAN);
  let exp_result = exp(f64::NAN);

  assert!(sqrt_result.is_nan());
  assert!(log_result.is_nan());
  assert!(exp_result.is_nan());
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
}

#[test]
fn successful_math_calls_do_not_set_errno_or_exceptions() {
  clear_all_excepts();
  write_errno(0);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
}

#[test]
fn successful_math_calls_preserve_preexisting_errno_and_no_exceptions() {
  clear_all_excepts();
  write_errno(ERANGE);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
}

#[test]
fn successful_math_calls_preserve_preexisting_exception_flags() {
  clear_all_excepts();
  write_errno(ERANGE);

  let raise_status = feraiseexcept(FE_DIVBYZERO);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn successful_math_calls_preserve_preexisting_invalid_exception_flag() {
  clear_all_excepts();
  write_errno(ERANGE);

  let raise_status = feraiseexcept(FE_INVALID);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_INVALID), 0);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn successful_math_calls_preserve_preexisting_overflow_exception_flag() {
  clear_all_excepts();
  write_errno(ERANGE);

  let raise_status = feraiseexcept(FE_OVERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}

#[test]
fn successful_math_calls_preserve_preexisting_underflow_exception_flag() {
  clear_all_excepts();
  write_errno(ERANGE);

  let raise_status = feraiseexcept(FE_UNDERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), ERANGE);
  assert_ne!(fetestexcept(FE_UNDERFLOW), 0);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_INVALID), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
}

#[test]
fn successful_math_calls_preserve_multiple_preexisting_exception_flags() {
  clear_all_excepts();
  write_errno(ERANGE);

  let raise_status = feraiseexcept(FE_INVALID | FE_OVERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");

  let pre_flags = fetestexcept(FE_ALL_EXCEPT);

  assert_ne!(pre_flags & FE_INVALID, 0);
  assert_ne!(pre_flags & FE_OVERFLOW, 0);
  assert_eq!(pre_flags & FE_DIVBYZERO, 0);
  assert_eq!(pre_flags & FE_UNDERFLOW, 0);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), ERANGE);

  let post_flags = fetestexcept(FE_ALL_EXCEPT);

  assert_ne!(post_flags & FE_INVALID, 0);
  assert_ne!(post_flags & FE_OVERFLOW, 0);
  assert_eq!(post_flags & FE_DIVBYZERO, 0);
  assert_eq!(post_flags & FE_UNDERFLOW, 0);
}

#[test]
fn successful_math_calls_preserve_divbyzero_and_underflow_exception_flags() {
  clear_all_excepts();
  write_errno(ERANGE);

  let raise_status = feraiseexcept(FE_DIVBYZERO | FE_UNDERFLOW);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");

  let pre_flags = fetestexcept(FE_ALL_EXCEPT);

  assert_ne!(pre_flags & FE_DIVBYZERO, 0);
  assert_ne!(pre_flags & FE_UNDERFLOW, 0);
  assert_eq!(pre_flags & FE_INVALID, 0);
  assert_eq!(pre_flags & FE_OVERFLOW, 0);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), ERANGE);

  let post_flags = fetestexcept(FE_ALL_EXCEPT);

  assert_ne!(post_flags & FE_DIVBYZERO, 0);
  assert_ne!(post_flags & FE_UNDERFLOW, 0);
  assert_eq!(post_flags & FE_INVALID, 0);
  assert_eq!(post_flags & FE_OVERFLOW, 0);
}

#[test]
fn successful_math_calls_preserve_all_preexisting_exception_flags() {
  clear_all_excepts();
  write_errno(ERANGE);

  let raise_status = feraiseexcept(FE_ALL_EXCEPT);

  assert_eq!(raise_status, 0, "feraiseexcept must succeed");

  let pre_flags = fetestexcept(FE_ALL_EXCEPT);

  assert_ne!(pre_flags & FE_INVALID, 0);
  assert_ne!(pre_flags & FE_DIVBYZERO, 0);
  assert_ne!(pre_flags & FE_OVERFLOW, 0);
  assert_ne!(pre_flags & FE_UNDERFLOW, 0);

  let sqrt_result = sqrt(4.0);
  let log_result = log(1.0);
  let exp_result = exp(0.0);

  assert_f64_eq(sqrt_result, 2.0);
  assert_f64_eq(log_result, 0.0);
  assert_f64_eq(exp_result, 1.0);
  assert_eq!(read_errno(), ERANGE);

  let post_flags = fetestexcept(FE_ALL_EXCEPT);

  assert_ne!(post_flags & FE_INVALID, 0);
  assert_ne!(post_flags & FE_DIVBYZERO, 0);
  assert_ne!(post_flags & FE_OVERFLOW, 0);
  assert_ne!(post_flags & FE_UNDERFLOW, 0);
}

#[test]
fn errno_and_fenv_state_are_thread_local() {
  clear_all_excepts();
  write_errno(0);

  let main_result = sqrt(-1.0);

  assert!(main_result.is_nan());

  let main_errno = read_errno();
  let main_flags = fetestexcept(FE_ALL_EXCEPT);
  let child = thread::spawn(|| {
    clear_all_excepts();
    write_errno(0);

    let child_initial_errno = read_errno();
    let child_initial_flags = fetestexcept(FE_ALL_EXCEPT);
    let child_result = log(0.0);

    assert!(child_result.is_infinite() && child_result.is_sign_negative());

    let child_errno_after = read_errno();
    let child_flags_after = fetestexcept(FE_ALL_EXCEPT);

    (
      child_initial_errno,
      child_initial_flags,
      child_errno_after,
      child_flags_after,
    )
  });
  let (child_initial_errno, child_initial_flags, child_errno_after, child_flags_after) =
    child.join().expect("child thread panicked");

  assert_eq!(child_initial_errno, 0);
  assert_eq!(child_initial_flags, 0);
  assert_eq!(child_errno_after, ERANGE);
  assert_ne!(child_flags_after & FE_DIVBYZERO, 0);
  assert_eq!(main_errno, EDOM);
  assert_ne!(main_flags & FE_INVALID, 0);
  assert_eq!(read_errno(), EDOM);
  assert_eq!(fetestexcept(FE_INVALID), FE_INVALID);
  assert_eq!(fetestexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(FE_UNDERFLOW), 0);
}
