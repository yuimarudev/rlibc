use core::ffi::{c_char, c_int, c_long, c_longlong, c_ulong, c_ulonglong, c_void};
use core::ptr::null_mut;
use rlibc::abi::errno::{EINVAL, ERANGE};
use rlibc::errno::__errno_location;
use rlibc::stdlib::{strtol, strtoll, strtoul, strtoull};
#[cfg(unix)]
use std::env;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
#[cfg(unix)]
use std::process::{Command, Output, Stdio};
use std::thread;

const BIG_DECIMAL: &[u8] = b"999999999999999999999999999999\0";
const BIG_NEGATIVE_DECIMAL: &[u8] = b"-999999999999999999999999999999\0";
#[cfg(unix)]
const CHILD_SCENARIO_ENV: &str = "RLIBC_STDLIB_CONV_SCENARIO";
#[cfg(unix)]
const CHILD_RUNNER_TEST: &str = "stdlib_conv_child_entrypoint";
#[cfg(unix)]
const SCENARIO_GUARDED_PREFIX_READ: &str = "guarded_prefix_read";
#[cfg(unix)]
const PROT_NONE: c_int = 0;
#[cfg(unix)]
const PROT_READ: c_int = 1;
#[cfg(unix)]
const PROT_WRITE: c_int = 2;
#[cfg(unix)]
const MAP_PRIVATE: c_int = 2;
#[cfg(unix)]
const MAP_ANONYMOUS: c_int = 0x20;

#[cfg(unix)]
unsafe extern "C" {
  fn getpagesize() -> c_int;
  fn mmap(
    addr: *mut c_void,
    length: usize,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: isize,
  ) -> *mut c_void;
  fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
  fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

fn set_errno(value: c_int) {
  // SAFETY: `__errno_location` returns a valid thread-local pointer for the calling thread.
  unsafe {
    __errno_location().write(value);
  }
}

fn errno_value() -> c_int {
  // SAFETY: `__errno_location` returns a valid thread-local pointer for the calling thread.
  unsafe { __errno_location().read() }
}

fn end_offset(start: *const c_char, end: *mut c_char) -> usize {
  assert!(!end.is_null(), "endptr must be written for this test");

  // SAFETY: both pointers are within the same string buffer used by each test case.
  let delta = unsafe { end.cast_const().offset_from(start) };

  usize::try_from(delta)
    .unwrap_or_else(|_| unreachable!("end pointer must not precede input pointer"))
}

#[cfg(unix)]
fn format_output(output: &Output) -> String {
  format!(
    "status={:?}, stdout={:?}, stderr={:?}",
    output.status,
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr)
  )
}

#[cfg(unix)]
fn run_child_scenario(scenario: &str) -> Output {
  let current_executable = env::current_exe().expect("failed to resolve current test executable");

  Command::new(current_executable)
    .arg("--exact")
    .arg(CHILD_RUNNER_TEST)
    .arg("--nocapture")
    .env(CHILD_SCENARIO_ENV, scenario)
    .stderr(Stdio::null())
    .output()
    .expect("failed to execute child test process")
}

#[cfg(unix)]
unsafe fn run_guarded_prefix_read_scenario() -> c_int {
  let page_size = unsafe { getpagesize() };

  if page_size <= 0 {
    return 60;
  }

  let page = usize::try_from(page_size)
    .unwrap_or_else(|_| unreachable!("positive page size must fit usize"));
  let mapping_len = page * 2;
  let mapping = unsafe {
    mmap(
      null_mut(),
      mapping_len,
      PROT_READ | PROT_WRITE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };
  let map_failed = usize::MAX as *mut c_void;

  if mapping == map_failed {
    return 61;
  }

  let second_page = unsafe { mapping.cast::<u8>().add(page).cast::<c_void>() };

  if unsafe { mprotect(second_page, page, PROT_NONE) } != 0 {
    let _result = unsafe { munmap(mapping, mapping_len) };

    return 62;
  }

  let parse_start = unsafe { mapping.cast::<u8>().add(page - 2) };

  unsafe {
    parse_start.write(b'0');
    parse_start.add(1).write(0);
  }

  let mut signed_end = null_mut();

  set_errno(91);
  // SAFETY: points to a NUL-terminated string that is readable up to the terminator.
  let signed = unsafe { strtol(parse_start.cast(), &raw mut signed_end, 0) };

  if signed != 0 || errno_value() != 91 || signed_end != unsafe { parse_start.add(1).cast() } {
    let _result = unsafe { munmap(mapping, mapping_len) };

    return 63;
  }

  let mut unsigned_end = null_mut();

  set_errno(92);
  // SAFETY: points to a NUL-terminated string that is readable up to the terminator.
  let unsigned = unsafe { strtoul(parse_start.cast(), &raw mut unsigned_end, 16) };

  if unsigned != 0 || errno_value() != 92 || unsigned_end != unsafe { parse_start.add(1).cast() } {
    let _result = unsafe { munmap(mapping, mapping_len) };

    return 64;
  }

  if unsafe { munmap(mapping, mapping_len) } != 0 {
    return 65;
  }

  0
}

#[test]
fn base_zero_autodetects_prefix_rules_for_all_variants() {
  let alpha_input = b"010z\0";
  let mut alpha_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 0) };

  assert_eq!(alpha_value, 8);
  assert_eq!(errno_value(), 0);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 3);

  let beta_input = b"0x20g\0";
  let mut beta_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 0) };

  assert_eq!(beta_value, 32);
  assert_eq!(errno_value(), 0);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 4);

  let gamma_input = b"42q\0";
  let mut gamma_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 0) };

  assert_eq!(gamma_value, 42);
  assert_eq!(errno_value(), 0);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 2);

  let delta_input = b"0777!\0";
  let mut delta_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 0) };

  assert_eq!(delta_value, 511);
  assert_eq!(errno_value(), 0);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 4);
}

#[test]
fn base_zero_does_not_treat_0b_as_binary_prefix() {
  let alpha_input = b"0b101\0";
  let mut alpha_end = null_mut();

  set_errno(121);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 0) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 121);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 1);

  let beta_input = b"-0B11\0";
  let mut beta_end = null_mut();

  set_errno(122);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 0) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 122);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 2);

  let gamma_input = b"+0b77\0";
  let mut gamma_end = null_mut();

  set_errno(123);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 0) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 123);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 2);

  let delta_input = b" \t0B1\0";
  let mut delta_end = null_mut();

  set_errno(124);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 0) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 124);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 3);
}

#[test]
fn base_zero_octal_stops_before_invalid_octal_digit() {
  let alpha_input = b"09\0";
  let mut alpha_end = null_mut();

  set_errno(71);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 0) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 71);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 1);

  let beta_input = b"-08\0";
  let mut beta_end = null_mut();

  set_errno(72);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 0) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 72);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 2);

  let gamma_input = b"+09\0";
  let mut gamma_end = null_mut();

  set_errno(73);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 0) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 73);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 2);

  let delta_input = b" \t09\0";
  let mut delta_end = null_mut();

  set_errno(74);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 0) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 74);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 3);
}

#[test]
fn endptr_points_to_first_unparsed_byte_or_input_start() {
  let alpha_input = b"123abc\0";
  let mut alpha_end = null_mut();
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, 123);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 3);

  let beta_input = b"-7fX\0";
  let mut beta_end = null_mut();
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 16) };

  assert_eq!(beta_value, -127);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 3);

  let gamma_input = b"0019\0";
  let mut gamma_end = null_mut();
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 8) };

  assert_eq!(gamma_value, 1);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 3);

  let delta_input = b"15!\0";
  let mut delta_end = null_mut();
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_value, 15);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 2);

  let no_digits = b"xyz\0";
  let mut no_digits_end = null_mut();
  // SAFETY: pointers are valid and input is NUL-terminated.
  let no_digits_alpha = unsafe { strtol(no_digits.as_ptr().cast(), &raw mut no_digits_end, 10) };

  assert_eq!(no_digits_alpha, 0);
  assert_eq!(end_offset(no_digits.as_ptr().cast(), no_digits_end), 0);

  no_digits_end = null_mut();
  // SAFETY: pointers are valid and input is NUL-terminated.
  let no_digits_beta = unsafe { strtoll(no_digits.as_ptr().cast(), &raw mut no_digits_end, 10) };

  assert_eq!(no_digits_beta, 0);
  assert_eq!(end_offset(no_digits.as_ptr().cast(), no_digits_end), 0);

  no_digits_end = null_mut();
  // SAFETY: pointers are valid and input is NUL-terminated.
  let no_digits_gamma = unsafe { strtoul(no_digits.as_ptr().cast(), &raw mut no_digits_end, 10) };

  assert_eq!(no_digits_gamma, 0);
  assert_eq!(end_offset(no_digits.as_ptr().cast(), no_digits_end), 0);

  no_digits_end = null_mut();
  // SAFETY: pointers are valid and input is NUL-terminated.
  let no_digits_delta = unsafe { strtoull(no_digits.as_ptr().cast(), &raw mut no_digits_end, 10) };

  assert_eq!(no_digits_delta, 0);
  assert_eq!(end_offset(no_digits.as_ptr().cast(), no_digits_end), 0);
}

#[test]
fn erange_is_set_on_overflow_or_underflow() {
  let mut alpha_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_overflow = unsafe { strtol(BIG_DECIMAL.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_overflow, c_long::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(
    end_offset(BIG_DECIMAL.as_ptr().cast(), alpha_end),
    BIG_DECIMAL.len() - 1
  );

  alpha_end = null_mut();
  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_underflow =
    unsafe { strtol(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_underflow, c_long::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(
    end_offset(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), alpha_end),
    BIG_NEGATIVE_DECIMAL.len() - 1
  );

  let mut beta_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_overflow = unsafe { strtoll(BIG_DECIMAL.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_overflow, c_longlong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(
    end_offset(BIG_DECIMAL.as_ptr().cast(), beta_end),
    BIG_DECIMAL.len() - 1
  );

  beta_end = null_mut();
  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_underflow =
    unsafe { strtoll(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_underflow, c_longlong::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(
    end_offset(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), beta_end),
    BIG_NEGATIVE_DECIMAL.len() - 1
  );

  let mut gamma_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_overflow = unsafe { strtoul(BIG_DECIMAL.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_overflow, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(
    end_offset(BIG_DECIMAL.as_ptr().cast(), gamma_end),
    BIG_DECIMAL.len() - 1
  );

  let mut delta_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_overflow = unsafe { strtoull(BIG_DECIMAL.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_overflow, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(
    end_offset(BIG_DECIMAL.as_ptr().cast(), delta_end),
    BIG_DECIMAL.len() - 1
  );
}

#[test]
fn overflow_with_suffix_consumes_all_valid_digits_before_suffix() {
  let alpha_input = b"999999999999999999999999999999xyz\0";
  let mut alpha_end = null_mut();

  set_errno(171);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, c_long::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 30);

  let beta_input = b"-999999999999999999999999999999xyz\0";
  let mut beta_end = null_mut();

  set_errno(172);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_value, c_longlong::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 31);

  let gamma_input = b"999999999999999999999999999999xyz\0";
  let mut gamma_end = null_mut();

  set_errno(173);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_value, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 30);

  let delta_input = b"999999999999999999999999999999xyz\0";
  let mut delta_end = null_mut();

  set_errno(174);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_value, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 30);
}

#[test]
fn negative_overflow_with_suffix_consumes_all_valid_digits_before_suffix() {
  let alpha_input = b"-999999999999999999999999999999xyz\0";
  let mut alpha_end = null_mut();

  set_errno(175);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, c_long::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 31);

  let beta_input = b"-999999999999999999999999999999xyz\0";
  let mut beta_end = null_mut();

  set_errno(176);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_value, c_longlong::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 31);

  let gamma_input = b"-999999999999999999999999999999xyz\0";
  let mut gamma_end = null_mut();

  set_errno(177);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_value, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 31);

  let delta_input = b"-999999999999999999999999999999xyz\0";
  let mut delta_end = null_mut();

  set_errno(178);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_value, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 31);
}

#[test]
fn positive_overflow_with_whitespace_and_plus_consumes_digits_before_suffix() {
  let alpha_input = b" \t+999999999999999999999999999999xyz\0";
  let mut alpha_end = null_mut();

  set_errno(179);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, c_long::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 33);

  let beta_input = b" \t+999999999999999999999999999999xyz\0";
  let mut beta_end = null_mut();

  set_errno(180);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_value, c_longlong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 33);

  let gamma_input = b" \t+999999999999999999999999999999xyz\0";
  let mut gamma_end = null_mut();

  set_errno(181);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_value, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 33);

  let delta_input = b" \t+999999999999999999999999999999xyz\0";
  let mut delta_end = null_mut();

  set_errno(182);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_value, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 33);
}

#[test]
fn negative_overflow_with_whitespace_and_minus_consumes_digits_before_suffix() {
  let alpha_input = b" \t-999999999999999999999999999999xyz\0";
  let mut alpha_end = null_mut();

  set_errno(183);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, c_long::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 33);

  let beta_input = b" \t-999999999999999999999999999999xyz\0";
  let mut beta_end = null_mut();

  set_errno(184);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_value, c_longlong::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 33);

  let gamma_input = b" \t-999999999999999999999999999999xyz\0";
  let mut gamma_end = null_mut();

  set_errno(185);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_value, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 33);

  let delta_input = b" \t-999999999999999999999999999999xyz\0";
  let mut delta_end = null_mut();

  set_errno(186);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_value, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 33);
}

#[test]
fn hex_overflow_with_prefix_consumes_digits_before_suffix() {
  let alpha_input = b" \t+0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFp\0";
  let mut alpha_end = null_mut();

  set_errno(187);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 0) };

  assert_eq!(alpha_value, c_long::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 37);

  let beta_input = b" \t+0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFp\0";
  let mut beta_end = null_mut();

  set_errno(188);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 16) };

  assert_eq!(beta_value, c_longlong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 37);

  let gamma_input = b" \t+0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFp\0";
  let mut gamma_end = null_mut();

  set_errno(189);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 0) };

  assert_eq!(gamma_value, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 37);

  let delta_input = b" \t+0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFp\0";
  let mut delta_end = null_mut();

  set_errno(190);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 16) };

  assert_eq!(delta_value, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 37);
}

#[test]
fn hex_negative_overflow_with_prefix_consumes_digits_before_suffix() {
  let alpha_input = b" \t-0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFp\0";
  let mut alpha_end = null_mut();

  set_errno(195);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 0) };

  assert_eq!(alpha_value, c_long::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 37);

  let beta_input = b" \t-0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFp\0";
  let mut beta_end = null_mut();

  set_errno(196);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 16) };

  assert_eq!(beta_value, c_longlong::MIN);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 37);

  let gamma_input = b" \t-0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFp\0";
  let mut gamma_end = null_mut();

  set_errno(197);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 0) };

  assert_eq!(gamma_value, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 37);

  let delta_input = b" \t-0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFp\0";
  let mut delta_end = null_mut();

  set_errno(198);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 16) };

  assert_eq!(delta_value, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 37);
}

#[test]
fn exact_boundaries_parse_without_erange() {
  let alpha_input = format!("{}\0", c_long::MIN);
  let mut alpha_end = null_mut();

  set_errno(111);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, c_long::MIN);
  assert_eq!(errno_value(), 111);
  assert_eq!(
    end_offset(alpha_input.as_ptr().cast(), alpha_end),
    alpha_input.len() - 1
  );

  let beta_input = format!("{}\0", c_longlong::MAX);
  let mut beta_end = null_mut();

  set_errno(112);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_value, c_longlong::MAX);
  assert_eq!(errno_value(), 112);
  assert_eq!(
    end_offset(beta_input.as_ptr().cast(), beta_end),
    beta_input.len() - 1
  );

  let gamma_input = format!("{}\0", c_ulong::MAX);
  let mut gamma_end = null_mut();

  set_errno(113);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_value, c_ulong::MAX);
  assert_eq!(errno_value(), 113);
  assert_eq!(
    end_offset(gamma_input.as_ptr().cast(), gamma_end),
    gamma_input.len() - 1
  );

  let delta_input = format!("{}\0", c_ulonglong::MAX);
  let mut delta_end = null_mut();

  set_errno(114);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_value, c_ulonglong::MAX);
  assert_eq!(errno_value(), 114);
  assert_eq!(
    end_offset(delta_input.as_ptr().cast(), delta_end),
    delta_input.len() - 1
  );
}

#[test]
fn signed_boundary_opposites_parse_without_erange() {
  let alpha_input = format!("+{}\0", c_long::MAX);
  let mut alpha_end = null_mut();

  set_errno(131);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, c_long::MAX);
  assert_eq!(errno_value(), 131);
  assert_eq!(
    end_offset(alpha_input.as_ptr().cast(), alpha_end),
    alpha_input.len() - 1
  );

  let beta_input = format!("{}\0", c_longlong::MIN);
  let mut beta_end = null_mut();

  set_errno(132);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_value, c_longlong::MIN);
  assert_eq!(errno_value(), 132);
  assert_eq!(
    end_offset(beta_input.as_ptr().cast(), beta_end),
    beta_input.len() - 1
  );
}

#[test]
fn invalid_base_sets_einval_for_all_variants() {
  let input = b"123\0";

  set_errno(0);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_invalid_base = unsafe { strtol(input.as_ptr().cast(), null_mut(), 1) };

  assert_eq!(alpha_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(0);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_invalid_base = unsafe { strtoll(input.as_ptr().cast(), null_mut(), 37) };

  assert_eq!(beta_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(0);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_invalid_base = unsafe { strtoul(input.as_ptr().cast(), null_mut(), 1) };

  assert_eq!(gamma_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(0);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_invalid_base = unsafe { strtoull(input.as_ptr().cast(), null_mut(), 37) };

  assert_eq!(delta_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn invalid_base_preserves_existing_endptr_value() {
  let input = b" \t-123\0";
  let alpha_expected = c"alpha_sentinel".as_ptr().cast_mut();
  let mut alpha_end = alpha_expected;

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_result = unsafe { strtol(input.as_ptr().cast(), &raw mut alpha_end, 1) };

  assert_eq!(alpha_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(alpha_end, alpha_expected);

  let beta_expected = c"beta_sentinel".as_ptr().cast_mut();
  let mut beta_end = beta_expected;

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_result = unsafe { strtoll(input.as_ptr().cast(), &raw mut beta_end, 37) };

  assert_eq!(beta_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(beta_end, beta_expected);

  let gamma_expected = c"gamma_sentinel".as_ptr().cast_mut();
  let mut gamma_end = gamma_expected;

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_result = unsafe { strtoul(input.as_ptr().cast(), &raw mut gamma_end, -9) };

  assert_eq!(gamma_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(gamma_end, gamma_expected);

  let delta_expected = c"delta_sentinel".as_ptr().cast_mut();
  let mut delta_end = delta_expected;

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_result = unsafe { strtoull(input.as_ptr().cast(), &raw mut delta_end, -2) };

  assert_eq!(delta_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(delta_end, delta_expected);
}

#[test]
fn invalid_base_with_null_endptr_overwrites_existing_errno() {
  let input = b"123\0";

  set_errno(221);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_invalid_base = unsafe { strtol(input.as_ptr().cast(), null_mut(), 1) };

  assert_eq!(alpha_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(222);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_invalid_base = unsafe { strtoll(input.as_ptr().cast(), null_mut(), 37) };

  assert_eq!(beta_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(223);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_invalid_base = unsafe { strtoul(input.as_ptr().cast(), null_mut(), 1) };

  assert_eq!(gamma_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(224);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_invalid_base = unsafe { strtoull(input.as_ptr().cast(), null_mut(), 37) };

  assert_eq!(delta_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn invalid_base_with_whitespace_and_sign_and_null_endptr_overwrites_errno() {
  let input = b" \t-123\0";

  set_errno(231);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_invalid_base = unsafe { strtol(input.as_ptr().cast(), null_mut(), 1) };

  assert_eq!(alpha_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(232);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_invalid_base = unsafe { strtoll(input.as_ptr().cast(), null_mut(), 37) };

  assert_eq!(beta_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(233);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_invalid_base = unsafe { strtoul(input.as_ptr().cast(), null_mut(), -9) };

  assert_eq!(gamma_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);

  set_errno(234);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_invalid_base = unsafe { strtoull(input.as_ptr().cast(), null_mut(), -2) };

  assert_eq!(delta_invalid_base, 0);
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn hex_prefix_without_hex_digit_consumes_only_leading_zero() {
  let auto_input = b"0x\0";
  let mut auto_end = null_mut();

  set_errno(31);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let auto_signed = unsafe { strtol(auto_input.as_ptr().cast(), &raw mut auto_end, 0) };

  assert_eq!(auto_signed, 0);
  assert_eq!(errno_value(), 31);
  assert_eq!(end_offset(auto_input.as_ptr().cast(), auto_end), 1);

  let explicit_input = b"0Xg\0";
  let mut explicit_end = null_mut();

  set_errno(32);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let explicit_signed =
    unsafe { strtoll(explicit_input.as_ptr().cast(), &raw mut explicit_end, 16) };

  assert_eq!(explicit_signed, 0);
  assert_eq!(errno_value(), 32);
  assert_eq!(end_offset(explicit_input.as_ptr().cast(), explicit_end), 1);

  let mut auto_unsigned_end = null_mut();

  set_errno(33);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let auto_unsigned = unsafe { strtoul(auto_input.as_ptr().cast(), &raw mut auto_unsigned_end, 0) };

  assert_eq!(auto_unsigned, 0);
  assert_eq!(errno_value(), 33);
  assert_eq!(end_offset(auto_input.as_ptr().cast(), auto_unsigned_end), 1);

  let mut explicit_unsigned_end = null_mut();

  set_errno(34);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let explicit_unsigned = unsafe {
    strtoull(
      explicit_input.as_ptr().cast(),
      &raw mut explicit_unsigned_end,
      16,
    )
  };

  assert_eq!(explicit_unsigned, 0);
  assert_eq!(errno_value(), 34);
  assert_eq!(
    end_offset(explicit_input.as_ptr().cast(), explicit_unsigned_end),
    1
  );
}

#[test]
fn hex_prefix_without_hex_digit_after_space_and_sign_consumes_only_zero() {
  let auto_input = b" \t-0x\0";
  let mut auto_signed_end = null_mut();

  set_errno(51);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let auto_signed = unsafe { strtol(auto_input.as_ptr().cast(), &raw mut auto_signed_end, 0) };

  assert_eq!(auto_signed, 0);
  assert_eq!(errno_value(), 51);
  assert_eq!(end_offset(auto_input.as_ptr().cast(), auto_signed_end), 4);

  let explicit_input = b" \n+0Xg\0";
  let mut explicit_signed_end = null_mut();

  set_errno(52);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let explicit_signed = unsafe {
    strtoll(
      explicit_input.as_ptr().cast(),
      &raw mut explicit_signed_end,
      16,
    )
  };

  assert_eq!(explicit_signed, 0);
  assert_eq!(errno_value(), 52);
  assert_eq!(
    end_offset(explicit_input.as_ptr().cast(), explicit_signed_end),
    4
  );

  let mut auto_unsigned_end = null_mut();

  set_errno(53);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let auto_unsigned = unsafe { strtoul(auto_input.as_ptr().cast(), &raw mut auto_unsigned_end, 0) };

  assert_eq!(auto_unsigned, 0);
  assert_eq!(errno_value(), 53);
  assert_eq!(end_offset(auto_input.as_ptr().cast(), auto_unsigned_end), 4);

  let mut explicit_unsigned_end = null_mut();

  set_errno(54);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let explicit_unsigned = unsafe {
    strtoull(
      explicit_input.as_ptr().cast(),
      &raw mut explicit_unsigned_end,
      16,
    )
  };

  assert_eq!(explicit_unsigned, 0);
  assert_eq!(errno_value(), 54);
  assert_eq!(
    end_offset(explicit_input.as_ptr().cast(), explicit_unsigned_end),
    4
  );
}

#[test]
fn hex_prefix_without_hex_digit_with_null_endptr_preserves_errno() {
  let auto_input = b"0x\0";

  set_errno(171);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_value = unsafe { strtol(auto_input.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 171);

  let explicit_input = b"0Xg\0";

  set_errno(172);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_value = unsafe { strtoll(explicit_input.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 172);

  set_errno(173);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_value = unsafe { strtoul(auto_input.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 173);

  set_errno(174);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_value = unsafe { strtoull(explicit_input.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 174);
}

#[test]
fn hex_prefix_without_hex_digit_after_space_and_sign_with_null_endptr_preserves_errno() {
  let auto_input = b" \t-0x\0";

  set_errno(181);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_value = unsafe { strtol(auto_input.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 181);

  let explicit_input = b" \n+0Xg\0";

  set_errno(182);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_value = unsafe { strtoll(explicit_input.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 182);

  set_errno(183);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_value = unsafe { strtoul(auto_input.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 183);

  set_errno(184);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_value = unsafe { strtoull(explicit_input.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 184);
}

#[test]
fn hex_prefix_with_digit_after_space_and_sign_parses_expected_value() {
  let auto_signed_input = b" \t-0x1fz\0";
  let mut auto_signed_end = null_mut();

  set_errno(61);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let auto_signed = unsafe {
    strtol(
      auto_signed_input.as_ptr().cast(),
      &raw mut auto_signed_end,
      0,
    )
  };

  assert_eq!(auto_signed, -31);
  assert_eq!(errno_value(), 61);
  assert_eq!(
    end_offset(auto_signed_input.as_ptr().cast(), auto_signed_end),
    7
  );

  let explicit_signed_input = b" \n+0X2a!\0";
  let mut explicit_signed_end = null_mut();

  set_errno(62);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let explicit_signed = unsafe {
    strtoll(
      explicit_signed_input.as_ptr().cast(),
      &raw mut explicit_signed_end,
      16,
    )
  };

  assert_eq!(explicit_signed, 42);
  assert_eq!(errno_value(), 62);
  assert_eq!(
    end_offset(explicit_signed_input.as_ptr().cast(), explicit_signed_end),
    7
  );

  let auto_unsigned_input = b" \t+0x2a@\0";
  let mut auto_unsigned_end = null_mut();

  set_errno(63);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let auto_unsigned = unsafe {
    strtoul(
      auto_unsigned_input.as_ptr().cast(),
      &raw mut auto_unsigned_end,
      0,
    )
  };

  assert_eq!(auto_unsigned, 42);
  assert_eq!(errno_value(), 63);
  assert_eq!(
    end_offset(auto_unsigned_input.as_ptr().cast(), auto_unsigned_end),
    7
  );

  let explicit_unsigned_input = b" \n+0X2a!\0";
  let mut explicit_unsigned_end = null_mut();

  set_errno(64);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let explicit_unsigned = unsafe {
    strtoull(
      explicit_unsigned_input.as_ptr().cast(),
      &raw mut explicit_unsigned_end,
      16,
    )
  };

  assert_eq!(explicit_unsigned, 42);
  assert_eq!(errno_value(), 64);
  assert_eq!(
    end_offset(
      explicit_unsigned_input.as_ptr().cast(),
      explicit_unsigned_end
    ),
    7
  );
}

#[test]
fn non_hex_base_treats_x_as_stop_after_leading_zero() {
  let alpha_input = b"0x10\0";
  let mut alpha_end = null_mut();

  set_errno(101);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 101);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 1);

  let beta_input = b"-0x7\0";
  let mut beta_end = null_mut();

  set_errno(102);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 8) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 102);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 2);

  let gamma_input = b"+0x99\0";
  let mut gamma_end = null_mut();

  set_errno(103);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 103);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 2);

  let delta_input = b" \t0x1\0";
  let mut delta_end = null_mut();

  set_errno(104);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 2) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 104);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 3);
}

#[test]
fn non_hex_base_with_null_endptr_preserves_errno() {
  let alpha_input = b"0x10\0";

  set_errno(211);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 211);

  let beta_input = b"-0x7\0";

  set_errno(212);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), null_mut(), 8) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 212);

  let gamma_input = b"+0x99\0";

  set_errno(213);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 213);

  let delta_input = b" \t0x1\0";

  set_errno(214);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), null_mut(), 2) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 214);
}

#[test]
fn invalid_base_preserves_endptr_value_even_with_whitespace_and_sign() {
  let input = b" \t-123\0";
  let signed_expected = c"sentinel".as_ptr().cast_mut();
  let mut signed_end = signed_expected;

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let signed = unsafe { strtol(input.as_ptr().cast(), &raw mut signed_end, 1) };

  assert_eq!(signed, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(signed_end, signed_expected);

  let unsigned_expected = c"sentinel".as_ptr().cast_mut();
  let mut unsigned_end = unsigned_expected;

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let unsigned = unsafe { strtoull(input.as_ptr().cast(), &raw mut unsigned_end, -9) };

  assert_eq!(unsigned, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(unsigned_end, unsigned_expected);
}

#[test]
fn whitespace_and_sign_are_consumed_before_digits() {
  let alpha_input = b" \t\n+17z\0";
  let mut alpha_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, 17);
  assert_eq!(errno_value(), 0);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 6);

  let beta_input = b" \r-1f!\0";
  let mut beta_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 16) };

  assert_eq!(beta_value, -31);
  assert_eq!(errno_value(), 0);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 5);
}

#[test]
fn conversion_stops_before_internal_whitespace() {
  let alpha_input = b"12 34\0";
  let mut alpha_end = null_mut();

  set_errno(161);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, 12);
  assert_eq!(errno_value(), 161);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 2);

  let beta_input = b"-1f \t\0";
  let mut beta_end = null_mut();

  set_errno(162);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 16) };

  assert_eq!(beta_value, -31);
  assert_eq!(errno_value(), 162);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 3);

  let gamma_input = b"+77 \n\0";
  let mut gamma_end = null_mut();

  set_errno(163);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 8) };

  assert_eq!(gamma_value, 63);
  assert_eq!(errno_value(), 163);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 3);

  let delta_input = b"101 \r\0";
  let mut delta_end = null_mut();

  set_errno(164);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 2) };

  assert_eq!(delta_value, 5);
  assert_eq!(errno_value(), 164);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 3);
}

#[test]
fn conversion_stops_before_internal_whitespace_with_null_endptr() {
  let alpha_input = b"12 34\0";

  set_errno(171);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(alpha_value, 12);
  assert_eq!(errno_value(), 171);

  let beta_input = b"-1f \t\0";

  set_errno(172);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(beta_value, -31);
  assert_eq!(errno_value(), 172);

  let gamma_input = b"+77 \n\0";

  set_errno(173);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), null_mut(), 8) };

  assert_eq!(gamma_value, 63);
  assert_eq!(errno_value(), 173);

  let delta_input = b"101 \r\0";

  set_errno(174);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), null_mut(), 2) };

  assert_eq!(delta_value, 5);
  assert_eq!(errno_value(), 174);
}

#[test]
fn explicit_base_parses_binary_and_base36_digits() {
  let binary_input = b"1011x\0";
  let mut binary_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let binary = unsafe { strtol(binary_input.as_ptr().cast(), &raw mut binary_end, 2) };

  assert_eq!(binary, 11);
  assert_eq!(errno_value(), 0);
  assert_eq!(end_offset(binary_input.as_ptr().cast(), binary_end), 4);

  let base36_input = b"z!\0";
  let mut base36_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let base36 = unsafe { strtoul(base36_input.as_ptr().cast(), &raw mut base36_end, 36) };

  assert_eq!(base36, 35);
  assert_eq!(errno_value(), 0);
  assert_eq!(end_offset(base36_input.as_ptr().cast(), base36_end), 1);

  let base36_large_input = b"10?\0";
  let mut base36_large_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let base36_large = unsafe {
    strtoull(
      base36_large_input.as_ptr().cast(),
      &raw mut base36_large_end,
      36,
    )
  };

  assert_eq!(base36_large, 36);
  assert_eq!(errno_value(), 0);
  assert_eq!(
    end_offset(base36_large_input.as_ptr().cast(), base36_large_end),
    2
  );
}

#[test]
fn unsigned_negative_inputs_wrap_without_errno() {
  let minus_one = b"-1\0";
  let mut gamma_end = null_mut();

  set_errno(123);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(minus_one.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_value, c_ulong::MAX);
  assert_eq!(errno_value(), 123);
  assert_eq!(end_offset(minus_one.as_ptr().cast(), gamma_end), 2);

  let mut delta_end = null_mut();

  set_errno(456);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(minus_one.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_value, c_ulonglong::MAX);
  assert_eq!(errno_value(), 456);
  assert_eq!(end_offset(minus_one.as_ptr().cast(), delta_end), 2);
}

#[test]
fn invalid_base_with_endptr_preserves_existing_pointer() {
  let input = b"9\0";
  let alpha_expected = c"sentinel".as_ptr().cast_mut();
  let mut alpha_end = alpha_expected;

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_result = unsafe { strtol(input.as_ptr().cast(), &raw mut alpha_end, -2) };

  assert_eq!(alpha_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(alpha_end, alpha_expected);

  let delta_expected = c"sentinel".as_ptr().cast_mut();
  let mut delta_end = delta_expected;

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_result = unsafe { strtoull(input.as_ptr().cast(), &raw mut delta_end, 37) };

  assert_eq!(delta_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(delta_end, delta_expected);
}

#[test]
fn invalid_base_overwrites_existing_errno_and_preserves_endptr() {
  let input = b" \t+123\0";
  let alpha_expected = c"sentinel".as_ptr().cast_mut();
  let mut alpha_end = alpha_expected;

  set_errno(ERANGE);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_result = unsafe { strtol(input.as_ptr().cast(), &raw mut alpha_end, 1) };

  assert_eq!(alpha_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(alpha_end, alpha_expected);

  let beta_expected = c"sentinel".as_ptr().cast_mut();
  let mut beta_end = beta_expected;

  set_errno(ERANGE);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_result = unsafe { strtoll(input.as_ptr().cast(), &raw mut beta_end, 37) };

  assert_eq!(beta_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(beta_end, beta_expected);

  let gamma_expected = c"sentinel".as_ptr().cast_mut();
  let mut gamma_end = gamma_expected;

  set_errno(ERANGE);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_result = unsafe { strtoul(input.as_ptr().cast(), &raw mut gamma_end, -9) };

  assert_eq!(gamma_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(gamma_end, gamma_expected);

  let delta_expected = c"sentinel".as_ptr().cast_mut();
  let mut delta_end = delta_expected;

  set_errno(ERANGE);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_result = unsafe { strtoull(input.as_ptr().cast(), &raw mut delta_end, -2) };

  assert_eq!(delta_result, 0);
  assert_eq!(errno_value(), EINVAL);
  assert_eq!(delta_end, delta_expected);
}

#[test]
fn successful_conversions_leave_errno_unchanged() {
  let alpha_input = b"42\0";

  set_errno(11);
  // SAFETY: input pointer is valid and NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(alpha_value, 42);
  assert_eq!(errno_value(), 11);

  let beta_input = b"-99\0";

  set_errno(12);
  // SAFETY: input pointer is valid and NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(beta_value, -99);
  assert_eq!(errno_value(), 12);

  let gamma_input = b"77\0";

  set_errno(13);
  // SAFETY: input pointer is valid and NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), null_mut(), 8) };

  assert_eq!(gamma_value, 63);
  assert_eq!(errno_value(), 13);

  let delta_input = b"101\0";

  set_errno(14);
  // SAFETY: input pointer is valid and NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), null_mut(), 2) };

  assert_eq!(delta_value, 5);
  assert_eq!(errno_value(), 14);
}

#[test]
fn no_digit_after_optional_sign_reports_no_conversion() {
  let signed_input = b" \t+\0";
  let mut alpha_end = null_mut();

  set_errno(21);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(signed_input.as_ptr().cast(), &raw mut alpha_end, 10) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 21);
  assert_eq!(end_offset(signed_input.as_ptr().cast(), alpha_end), 0);

  let mut beta_end = null_mut();

  set_errno(22);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(signed_input.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 22);
  assert_eq!(end_offset(signed_input.as_ptr().cast(), beta_end), 0);

  let unsigned_input = b" \n-\0";
  let mut gamma_end = null_mut();

  set_errno(23);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(unsigned_input.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 23);
  assert_eq!(end_offset(unsigned_input.as_ptr().cast(), gamma_end), 0);

  let mut delta_end = null_mut();

  set_errno(24);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(unsigned_input.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 24);
  assert_eq!(end_offset(unsigned_input.as_ptr().cast(), delta_end), 0);
}

#[test]
fn no_digit_after_optional_sign_with_base_zero_reports_no_conversion() {
  let signed_input = b" \t+\0";
  let mut alpha_end = null_mut();

  set_errno(241);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(signed_input.as_ptr().cast(), &raw mut alpha_end, 0) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 241);
  assert_eq!(end_offset(signed_input.as_ptr().cast(), alpha_end), 0);

  let mut beta_end = null_mut();

  set_errno(242);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(signed_input.as_ptr().cast(), &raw mut beta_end, 0) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 242);
  assert_eq!(end_offset(signed_input.as_ptr().cast(), beta_end), 0);

  let unsigned_input = b" \n-\0";
  let mut gamma_end = null_mut();

  set_errno(243);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(unsigned_input.as_ptr().cast(), &raw mut gamma_end, 0) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 243);
  assert_eq!(end_offset(unsigned_input.as_ptr().cast(), gamma_end), 0);

  let mut delta_end = null_mut();

  set_errno(244);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(unsigned_input.as_ptr().cast(), &raw mut delta_end, 0) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 244);
  assert_eq!(end_offset(unsigned_input.as_ptr().cast(), delta_end), 0);
}

#[test]
fn sign_then_space_then_digit_with_base_zero_reports_no_conversion() {
  let signed_input = b" \t+ 17\0";
  let mut alpha_end = null_mut();

  set_errno(261);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(signed_input.as_ptr().cast(), &raw mut alpha_end, 0) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 261);
  assert_eq!(end_offset(signed_input.as_ptr().cast(), alpha_end), 0);

  let mut beta_end = null_mut();

  set_errno(262);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(signed_input.as_ptr().cast(), &raw mut beta_end, 0) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 262);
  assert_eq!(end_offset(signed_input.as_ptr().cast(), beta_end), 0);

  let unsigned_input = b" \n- 42\0";
  let mut gamma_end = null_mut();

  set_errno(263);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(unsigned_input.as_ptr().cast(), &raw mut gamma_end, 0) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 263);
  assert_eq!(end_offset(unsigned_input.as_ptr().cast(), gamma_end), 0);

  let mut delta_end = null_mut();

  set_errno(264);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(unsigned_input.as_ptr().cast(), &raw mut delta_end, 0) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 264);
  assert_eq!(end_offset(unsigned_input.as_ptr().cast(), delta_end), 0);
}

#[test]
fn no_digit_after_optional_sign_with_explicit_base_reports_no_conversion() {
  let signed_input = b" \t+\0";
  let mut alpha_end = null_mut();

  set_errno(201);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(signed_input.as_ptr().cast(), &raw mut alpha_end, 2) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 201);
  assert_eq!(end_offset(signed_input.as_ptr().cast(), alpha_end), 0);

  let mut beta_end = null_mut();

  set_errno(202);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(signed_input.as_ptr().cast(), &raw mut beta_end, 8) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 202);
  assert_eq!(end_offset(signed_input.as_ptr().cast(), beta_end), 0);

  let unsigned_input = b" \n-\0";
  let mut gamma_end = null_mut();

  set_errno(203);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(unsigned_input.as_ptr().cast(), &raw mut gamma_end, 16) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 203);
  assert_eq!(end_offset(unsigned_input.as_ptr().cast(), gamma_end), 0);

  let mut delta_end = null_mut();

  set_errno(204);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(unsigned_input.as_ptr().cast(), &raw mut delta_end, 36) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 204);
  assert_eq!(end_offset(unsigned_input.as_ptr().cast(), delta_end), 0);
}

#[test]
fn invalid_first_digit_for_explicit_base_reports_no_conversion() {
  let alpha_input = b"2\0";
  let mut alpha_end = null_mut();

  set_errno(141);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), &raw mut alpha_end, 2) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 141);
  assert_eq!(end_offset(alpha_input.as_ptr().cast(), alpha_end), 0);

  let beta_input = b" -8\0";
  let mut beta_end = null_mut();

  set_errno(142);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), &raw mut beta_end, 8) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 142);
  assert_eq!(end_offset(beta_input.as_ptr().cast(), beta_end), 0);

  let gamma_input = b"+g\0";
  let mut gamma_end = null_mut();

  set_errno(143);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), &raw mut gamma_end, 16) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 143);
  assert_eq!(end_offset(gamma_input.as_ptr().cast(), gamma_end), 0);

  let delta_input = b"\t-z\0";
  let mut delta_end = null_mut();

  set_errno(144);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), &raw mut delta_end, 35) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 144);
  assert_eq!(end_offset(delta_input.as_ptr().cast(), delta_end), 0);
}

#[test]
fn invalid_first_digit_with_null_endptr_preserves_errno() {
  let alpha_input = b"2\0";

  set_errno(151);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), null_mut(), 2) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 151);

  let beta_input = b" -8\0";

  set_errno(152);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_value = unsafe { strtoll(beta_input.as_ptr().cast(), null_mut(), 8) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 152);

  let gamma_input = b"+g\0";

  set_errno(153);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 153);

  let delta_input = b"\t-z\0";

  set_errno(154);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_value = unsafe { strtoull(delta_input.as_ptr().cast(), null_mut(), 35) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 154);
}

#[test]
fn whitespace_only_input_reports_no_conversion() {
  let whitespace_only = b" \t\n\0";
  let mut alpha_end = null_mut();

  set_errno(81);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let alpha_value = unsafe { strtol(whitespace_only.as_ptr().cast(), &raw mut alpha_end, 0) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 81);
  assert_eq!(end_offset(whitespace_only.as_ptr().cast(), alpha_end), 0);

  let mut beta_end = null_mut();

  set_errno(82);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let beta_value = unsafe { strtoll(whitespace_only.as_ptr().cast(), &raw mut beta_end, 10) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 82);
  assert_eq!(end_offset(whitespace_only.as_ptr().cast(), beta_end), 0);

  let mut gamma_end = null_mut();

  set_errno(83);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_value = unsafe { strtoul(whitespace_only.as_ptr().cast(), &raw mut gamma_end, 8) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 83);
  assert_eq!(end_offset(whitespace_only.as_ptr().cast(), gamma_end), 0);

  let mut delta_end = null_mut();

  set_errno(84);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_value = unsafe { strtoull(whitespace_only.as_ptr().cast(), &raw mut delta_end, 36) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 84);
  assert_eq!(end_offset(whitespace_only.as_ptr().cast(), delta_end), 0);
}

#[test]
fn no_conversion_with_null_endptr_preserves_errno() {
  let signed_input = b" \t+\0";

  set_errno(41);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_value = unsafe { strtol(signed_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 41);

  set_errno(42);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_value = unsafe { strtoll(signed_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 42);

  let unsigned_input = b" \n-\0";

  set_errno(43);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_value = unsafe { strtoul(unsigned_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 43);

  set_errno(44);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_value = unsafe { strtoull(unsigned_input.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 44);
}

#[test]
fn no_digit_after_optional_sign_with_base_zero_and_null_endptr_preserves_errno() {
  let signed_input = b" \t+\0";

  set_errno(251);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_value = unsafe { strtol(signed_input.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 251);

  set_errno(252);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_value = unsafe { strtoll(signed_input.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 252);

  let unsigned_input = b" \n-\0";

  set_errno(253);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_value = unsafe { strtoul(unsigned_input.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 253);

  set_errno(254);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_value = unsafe { strtoull(unsigned_input.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 254);
}

#[test]
fn no_digit_after_optional_sign_with_explicit_base_and_null_endptr_preserves_errno() {
  let alpha_input = b" \t+\0";

  set_errno(191);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_value = unsafe { strtol(alpha_input.as_ptr().cast(), null_mut(), 2) };

  assert_eq!(alpha_value, 0);
  assert_eq!(errno_value(), 191);

  set_errno(192);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_value = unsafe { strtoll(alpha_input.as_ptr().cast(), null_mut(), 8) };

  assert_eq!(beta_value, 0);
  assert_eq!(errno_value(), 192);

  let gamma_input = b" \n-\0";

  set_errno(193);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_value = unsafe { strtoul(gamma_input.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(gamma_value, 0);
  assert_eq!(errno_value(), 193);

  set_errno(194);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_value = unsafe { strtoull(gamma_input.as_ptr().cast(), null_mut(), 36) };

  assert_eq!(delta_value, 0);
  assert_eq!(errno_value(), 194);
}

#[test]
fn unsigned_negative_overflow_sets_erange() {
  let mut gamma_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let gamma_overflow =
    unsafe { strtoul(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), &raw mut gamma_end, 10) };

  assert_eq!(gamma_overflow, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(
    end_offset(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), gamma_end),
    BIG_NEGATIVE_DECIMAL.len() - 1
  );

  let mut delta_end = null_mut();

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let delta_overflow =
    unsafe { strtoull(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), &raw mut delta_end, 10) };

  assert_eq!(delta_overflow, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
  assert_eq!(
    end_offset(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), delta_end),
    BIG_NEGATIVE_DECIMAL.len() - 1
  );
}

#[test]
fn overflow_with_null_endptr_sets_erange() {
  set_errno(301);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_overflow = unsafe { strtol(BIG_DECIMAL.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(alpha_overflow, c_long::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(302);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_underflow = unsafe { strtoll(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(beta_underflow, c_longlong::MIN);
  assert_eq!(errno_value(), ERANGE);

  set_errno(303);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_overflow = unsafe { strtoul(BIG_DECIMAL.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(gamma_overflow, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(304);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_overflow = unsafe { strtoull(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), null_mut(), 10) };

  assert_eq!(delta_overflow, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
}

#[test]
fn hex_overflow_with_null_endptr_sets_erange() {
  let big_hex = b"0xffffffffffffffffffffffffffffffff\0";

  set_errno(311);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_overflow = unsafe { strtol(big_hex.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(alpha_overflow, c_long::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(312);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_overflow = unsafe { strtoll(big_hex.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(beta_overflow, c_longlong::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(313);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_overflow = unsafe { strtoul(big_hex.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(gamma_overflow, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(314);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_overflow = unsafe { strtoull(big_hex.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(delta_overflow, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
}

#[test]
fn hex_negative_overflow_with_null_endptr_sets_erange() {
  let big_negative_hex = b"-0xffffffffffffffffffffffffffffffff\0";

  set_errno(321);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_underflow = unsafe { strtol(big_negative_hex.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(alpha_underflow, c_long::MIN);
  assert_eq!(errno_value(), ERANGE);

  set_errno(322);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_underflow = unsafe { strtoll(big_negative_hex.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(beta_underflow, c_longlong::MIN);
  assert_eq!(errno_value(), ERANGE);

  set_errno(323);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_overflow = unsafe { strtoul(big_negative_hex.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(gamma_overflow, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(324);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_overflow = unsafe { strtoull(big_negative_hex.as_ptr().cast(), null_mut(), 0) };

  assert_eq!(delta_overflow, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
}

#[test]
fn explicit_hex_overflow_with_null_endptr_sets_erange() {
  let big_hex_digits = b"ffffffffffffffffffffffffffffffff\0";

  set_errno(331);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let alpha_overflow = unsafe { strtol(big_hex_digits.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(alpha_overflow, c_long::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(332);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let beta_overflow = unsafe { strtoll(big_hex_digits.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(beta_overflow, c_longlong::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(333);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let gamma_overflow = unsafe { strtoul(big_hex_digits.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(gamma_overflow, c_ulong::MAX);
  assert_eq!(errno_value(), ERANGE);

  set_errno(334);
  // SAFETY: input pointer is valid and NUL-terminated; null endptr is allowed.
  let delta_overflow = unsafe { strtoull(big_hex_digits.as_ptr().cast(), null_mut(), 16) };

  assert_eq!(delta_overflow, c_ulonglong::MAX);
  assert_eq!(errno_value(), ERANGE);
}

#[test]
fn conversion_error_errno_is_thread_local() {
  let input = b"123\0";

  set_errno(777);

  let child = thread::spawn(move || {
    set_errno(0);

    // SAFETY: input pointer is valid and NUL-terminated; invalid base is intentional.
    let value = unsafe { strtol(input.as_ptr().cast(), null_mut(), 1) };

    (value, errno_value())
  });
  let (child_value, child_errno) = child.join().expect("child thread panicked");

  assert_eq!(child_value, 0);
  assert_eq!(child_errno, EINVAL);
  assert_eq!(errno_value(), 777);
}

#[test]
fn overflow_error_errno_is_thread_local() {
  set_errno(778);

  let child = thread::spawn(move || {
    let mut child_end = null_mut();

    set_errno(0);

    // SAFETY: pointer is valid and NUL-terminated.
    let child_value = unsafe { strtoull(BIG_DECIMAL.as_ptr().cast(), &raw mut child_end, 10) };

    (
      child_value,
      errno_value(),
      end_offset(BIG_DECIMAL.as_ptr().cast(), child_end),
    )
  });
  let (child_value, child_errno, child_offset) = child.join().expect("child thread panicked");

  assert_eq!(child_value, c_ulonglong::MAX);
  assert_eq!(child_errno, ERANGE);
  assert_eq!(child_offset, BIG_DECIMAL.len() - 1);
  assert_eq!(errno_value(), 778);
}

#[test]
fn underflow_error_errno_is_thread_local() {
  set_errno(779);

  let child = thread::spawn(move || {
    let mut child_end = null_mut();

    set_errno(0);

    // SAFETY: pointer is valid and NUL-terminated.
    let child_value =
      unsafe { strtol(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), &raw mut child_end, 10) };

    (
      child_value,
      errno_value(),
      end_offset(BIG_NEGATIVE_DECIMAL.as_ptr().cast(), child_end),
    )
  });
  let (child_value, child_errno, child_offset) = child.join().expect("child thread panicked");

  assert_eq!(child_value, c_long::MIN);
  assert_eq!(child_errno, ERANGE);
  assert_eq!(child_offset, BIG_NEGATIVE_DECIMAL.len() - 1);
  assert_eq!(errno_value(), 779);
}

#[cfg(unix)]
#[test]
fn stdlib_conv_child_entrypoint() {
  let Ok(scenario) = env::var(CHILD_SCENARIO_ENV) else {
    return;
  };
  let exit_code = match scenario.as_str() {
    SCENARIO_GUARDED_PREFIX_READ => unsafe { run_guarded_prefix_read_scenario() },
    _ => panic!("unknown child scenario: {scenario}"),
  };

  std::process::exit(exit_code);
}

#[cfg(unix)]
#[test]
fn hex_prefix_probe_does_not_read_past_nul() {
  let output = run_child_scenario(SCENARIO_GUARDED_PREFIX_READ);
  let output_context = format_output(&output);

  assert_eq!(
    output.status.code(),
    Some(0),
    "guarded-prefix scenario failed: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "guarded-prefix scenario terminated by signal: {output_context}"
  );
}
