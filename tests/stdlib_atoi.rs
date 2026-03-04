use core::ffi::{c_char, c_int, c_long, c_longlong};
use rlibc::abi::errno::{EINVAL, ERANGE};
use rlibc::errno::__errno_location;
use rlibc::stdlib::{atoi, atol, atoll};

const fn as_c_char_ptr(bytes: &[u8]) -> *const c_char {
  bytes.as_ptr().cast::<c_char>()
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns valid thread-local errno storage.
  unsafe { __errno_location().read() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns valid thread-local errno storage.
  unsafe {
    __errno_location().write(value);
  }
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

  c_int::try_from(signed).unwrap_or_else(|_| unreachable!("wrapped value must fit in c_int"))
}

#[test]
fn atoi_family_uses_base10_fixed_parsing() {
  let input = b"010\0";
  let input_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_int, 10);
  assert_eq!(parsed_long, 10);
  assert_eq!(parsed_wide, 10);
}

#[test]
fn atoi_family_null_input_sets_einval() {
  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let parsed_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let parsed_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let parsed_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_null_input_overwrites_prior_errno_with_einval() {
  write_errno(ERANGE);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let parsed_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ERANGE);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let parsed_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ERANGE);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let parsed_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_successful_conversion() {
  let input = b"42\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 42);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 42);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 42);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_no_conversion() {
  let input = b"+abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_overwrites_einval_with_erange_on_overflow_after_null_input() {
  let input = b"999999999999999999999999999999\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_int, c_long_to_c_int_wrapping(c_long::MAX));

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_long, c_long::MAX);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_wide, c_longlong::MAX);
}

#[test]
fn atoi_family_overwrites_einval_with_erange_on_underflow_after_null_input() {
  let input = b"-999999999999999999999999999999\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_int, c_long_to_c_int_wrapping(c_long::MIN));

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_long, c_long::MIN);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_wide, c_longlong::MIN);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_signed_successful_conversion() {
  let input = b" \t+7\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 7);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 7);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 7);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_repeated_null_input_keeps_einval() {
  write_errno(ERANGE);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let first_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(first_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let second_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(second_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ERANGE);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let first_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(first_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let second_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(second_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ERANGE);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let first_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(first_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let second_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(second_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_sign_only_no_conversion() {
  let input = b" \t+\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_minus_only_no_conversion() {
  let input = b" \t-\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_sign_space_digit_no_conversion() {
  let input = b"+ 7\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_whitespace_only_no_conversion() {
  let input = b" \t \n\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_minus_space_digit_no_conversion() {
  let input = b"- 7\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_keeps_einval_after_null_input_then_double_plus_digit_no_conversion() {
  let input = b"++7\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_int = unsafe { atoi(core::ptr::null()) };

  assert_eq!(null_int, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_long = unsafe { atol(core::ptr::null()) };

  assert_eq!(null_long, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: This test verifies null-pointer handling policy for wrapper entry points.
  let null_wide = unsafe { atoll(core::ptr::null()) };

  assert_eq!(null_wide, 0);
  assert_eq!(read_errno(), EINVAL);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn atoi_family_supports_partial_decimal_conversion() {
  let input = b"123abc\0";
  let input_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_int, 123);
  assert_eq!(parsed_long, 123);
  assert_eq!(parsed_wide, 123);
}

#[test]
fn atoi_family_returns_zero_when_input_is_not_convertible() {
  let input = b"abc123\0";
  let input_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(parsed_long, 0);
  assert_eq!(parsed_wide, 0);
}

#[test]
fn atoi_family_accepts_leading_space_and_sign() {
  let input = b"\t -42rest\0";
  let input_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_int, -42);
  assert_eq!(parsed_long, -42);
  assert_eq!(parsed_wide, -42);
}

#[test]
fn atoi_family_preserves_errno_when_no_conversion_occurs() {
  let input = b"+abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_minus_prefixed_non_digit_input() {
  let input = b"-abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_whitespace_minus_prefixed_non_digit_input() {
  let input = b" \t-abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_whitespace_plus_prefixed_non_digit_input() {
  let input = b" \t+abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_double_sign_non_digit_input() {
  let input = b"+-abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_reversed_double_sign_non_digit_input() {
  let input = b"-+abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_double_sign_then_digit_input() {
  for input in [b"+-1\0".as_slice(), b"-+1\0".as_slice()] {
    let input_ptr = as_c_char_ptr(input);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, 0);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_for_double_plus_non_digit_input() {
  let input = b"++abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_double_minus_non_digit_input() {
  let input = b"--abc\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_sign_then_space_then_digits_input() {
  for input in [b"+ 123\0".as_slice(), b"- 123\0".as_slice()] {
    let input_ptr = as_c_char_ptr(input);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, 0);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_for_sign_then_control_whitespace_before_digits_input() {
  for input in [b"+\t123\0".as_slice(), b"-\n123\0".as_slice()] {
    let input_ptr = as_c_char_ptr(input);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, 0);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_for_whitespace_double_sign_then_digit_input() {
  for input in [b" \t+-1\0".as_slice(), b" \t-+1\0".as_slice()] {
    let input_ptr = as_c_char_ptr(input);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, 0);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_for_sign_only_input() {
  for input in [b"+\0".as_slice(), b"-\0".as_slice()] {
    let input_ptr = as_c_char_ptr(input);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, 0);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_for_sign_then_non_space_control_before_digits_input() {
  for input in [b"+\x0c123\0".as_slice(), b"-\x0b123\0".as_slice()] {
    let input_ptr = as_c_char_ptr(input);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, 0);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_for_sign_then_whitespace_only_input() {
  for input in [b"+ \t\0".as_slice(), b"- \n\0".as_slice()] {
    let input_ptr = as_c_char_ptr(input);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, 0);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_for_whitespace_and_sign_without_digits() {
  let input = b" \t+\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_whitespace_and_minus_without_digits() {
  let input = b" \t-\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_for_empty_and_whitespace_only_input() {
  for input in [b"\0".as_slice(), b" \n\t\0".as_slice()] {
    let input_ptr = as_c_char_ptr(input);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, 0);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, 0);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_propagates_erange_from_overflowing_input() {
  let input = b"999999999999999999999999999999\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_int, c_long_to_c_int_wrapping(c_long::MAX));

  write_errno(0);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_long, c_long::MAX);

  write_errno(0);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_wide, c_longlong::MAX);
}

#[test]
fn atoi_family_propagates_erange_from_underflowing_input() {
  let input = b"-999999999999999999999999999999\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(0);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_int, c_long_to_c_int_wrapping(c_long::MIN));

  write_errno(0);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_long, c_long::MIN);

  write_errno(0);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(read_errno(), ERANGE);
  assert_eq!(parsed_wide, c_longlong::MIN);
}

#[test]
fn atoi_narrows_without_erange_for_non_erange_long_values() {
  if c_long::BITS <= c_int::BITS {
    return;
  }

  let just_above_c_int_max = c_long::from(c_int::MAX) + 1;
  let just_below_c_int_min = c_long::from(c_int::MIN) - 1;

  for value in [just_above_c_int_max, just_below_c_int_min] {
    let input = format!("{value}\0");
    let input_ptr = as_c_char_ptr(input.as_bytes());
    let expected_atoll =
      c_longlong::try_from(value).unwrap_or_else(|_| unreachable!("c_long fits c_longlong"));

    write_errno(0);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(read_errno(), 0);
    assert_eq!(parsed_int, c_long_to_c_int_wrapping(value));

    write_errno(0);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(read_errno(), 0);
    assert_eq!(parsed_long, value);

    write_errno(0);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(read_errno(), 0);
    assert_eq!(parsed_wide, expected_atoll);
  }
}

#[test]
fn atoi_family_accepts_exact_long_boundaries_without_erange() {
  let long_min = format!("{}\0", c_long::MIN);
  let long_max = format!("{}\0", c_long::MAX);

  for (input, long_expected) in [(long_min, c_long::MIN), (long_max, c_long::MAX)] {
    let input_ptr = as_c_char_ptr(input.as_bytes());
    let expected_wide = c_longlong::try_from(long_expected)
      .unwrap_or_else(|_| unreachable!("c_long fits c_longlong"));

    write_errno(0);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(read_errno(), 0);
    assert_eq!(parsed_int, c_long_to_c_int_wrapping(long_expected));

    write_errno(0);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(read_errno(), 0);
    assert_eq!(parsed_long, long_expected);

    write_errno(0);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(read_errno(), 0);
    assert_eq!(parsed_wide, expected_wide);
  }
}

#[test]
fn atoi_family_preserves_existing_errno_on_successful_conversion() {
  let input = b"123\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 123);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 123);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 123);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_existing_errno_on_signed_successful_conversion() {
  let input = b" \t+7\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 7);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 7);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 7);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_partial_signed_successful_conversion() {
  let input = b" \t-42rest\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, -42);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, -42);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, -42);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_negative_zero_successful_conversion() {
  let input = b"-0\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_zero_with_trailing_text_conversion() {
  let input = b"0tail\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_signed_zero_with_trailing_text_conversion() {
  let input = b"+0rest\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_c_int_boundary_successful_conversion() {
  for value in [c_int::MIN, c_int::MAX] {
    let input = format!("{value}\0");
    let input_ptr = as_c_char_ptr(input.as_bytes());
    let long_expected = c_long::from(value);
    let longlong_expected = c_longlong::from(value);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, value);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, long_expected);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, longlong_expected);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_on_exact_long_boundaries_successful_conversion() {
  for value in [c_long::MIN, c_long::MAX] {
    let input = format!("{value}\0");
    let input_ptr = as_c_char_ptr(input.as_bytes());
    let expected_wide =
      c_longlong::try_from(value).unwrap_or_else(|_| unreachable!("c_long fits c_longlong"));

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_int = unsafe { atoi(input_ptr) };

    assert_eq!(parsed_int, c_long_to_c_int_wrapping(value));
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_long = unsafe { atol(input_ptr) };

    assert_eq!(parsed_long, value);
    assert_eq!(read_errno(), ERANGE);

    write_errno(ERANGE);
    // SAFETY: `input` is NUL-terminated and readable.
    let parsed_wide = unsafe { atoll(input_ptr) };

    assert_eq!(parsed_wide, expected_wide);
    assert_eq!(read_errno(), ERANGE);
  }
}

#[test]
fn atoi_family_preserves_errno_on_plus_prefixed_long_max_conversion() {
  let input = format!("+{}\0", c_long::MAX);
  let input_ptr = as_c_char_ptr(input.as_bytes());
  let expected_wide =
    c_longlong::try_from(c_long::MAX).unwrap_or_else(|_| unreachable!("c_long fits c_longlong"));

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, c_long_to_c_int_wrapping(c_long::MAX));
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, c_long::MAX);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, expected_wide);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_whitespace_plus_long_max_partial_conversion() {
  let input = format!(" \t+{}tail\0", c_long::MAX);
  let input_ptr = as_c_char_ptr(input.as_bytes());
  let expected_wide =
    c_longlong::try_from(c_long::MAX).unwrap_or_else(|_| unreachable!("c_long fits c_longlong"));

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, c_long_to_c_int_wrapping(c_long::MAX));
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, c_long::MAX);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, expected_wide);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_whitespace_long_min_partial_conversion() {
  let input = format!(" \t{}tail\0", c_long::MIN);
  let input_ptr = as_c_char_ptr(input.as_bytes());
  let expected_wide =
    c_longlong::try_from(c_long::MIN).unwrap_or_else(|_| unreachable!("c_long fits c_longlong"));

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, c_long_to_c_int_wrapping(c_long::MIN));
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, c_long::MIN);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, expected_wide);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_whitespace_negative_zero_partial_conversion() {
  let input = b" \t-0tail\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_whitespace_plus_zero_partial_conversion() {
  let input = b" \t+0tail\0";
  let input_ptr = as_c_char_ptr(input);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, 0);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, 0);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_plus_prefixed_c_int_max_partial_conversion() {
  let input = format!("+{}tail\0", c_int::MAX);
  let input_ptr = as_c_char_ptr(input.as_bytes());
  let expected_long = c_long::from(c_int::MAX);
  let expected_wide = c_longlong::from(c_int::MAX);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, c_int::MAX);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, expected_long);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, expected_wide);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_whitespace_c_int_min_partial_conversion() {
  let input = format!(" \t{}tail\0", c_int::MIN);
  let input_ptr = as_c_char_ptr(input.as_bytes());
  let expected_long = c_long::from(c_int::MIN);
  let expected_wide = c_longlong::from(c_int::MIN);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let got_int = unsafe { atoi(input_ptr) };

  assert_eq!(got_int, c_int::MIN);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let got_long = unsafe { atol(input_ptr) };

  assert_eq!(got_long, expected_long);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let got_wide = unsafe { atoll(input_ptr) };

  assert_eq!(got_wide, expected_wide);
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn atoi_family_preserves_errno_on_whitespace_plus_c_int_max_partial_conversion() {
  let input = format!(" \t+{}tail\0", c_int::MAX);
  let input_ptr = as_c_char_ptr(input.as_bytes());
  let expected_long = c_long::from(c_int::MAX);
  let expected_wide = c_longlong::from(c_int::MAX);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_int = unsafe { atoi(input_ptr) };

  assert_eq!(parsed_int, c_int::MAX);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_long = unsafe { atol(input_ptr) };

  assert_eq!(parsed_long, expected_long);
  assert_eq!(read_errno(), ERANGE);

  write_errno(ERANGE);
  // SAFETY: `input` is NUL-terminated and readable.
  let parsed_wide = unsafe { atoll(input_ptr) };

  assert_eq!(parsed_wide, expected_wide);
  assert_eq!(read_errno(), ERANGE);
}
