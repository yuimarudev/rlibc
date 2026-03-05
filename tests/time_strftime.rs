use core::ffi::{c_char, c_int};
use core::ptr;
use rlibc::abi::errno::EINVAL;
use rlibc::abi::types::size_t;
use rlibc::errno::__errno_location;
use rlibc::time::{strftime, timegm, tm};

fn sz(len: usize) -> size_t {
  size_t::try_from(len)
    .unwrap_or_else(|_| unreachable!("usize does not fit into size_t on this target"))
}

const fn as_c_char_ptr(bytes: &[u8]) -> *const c_char {
  bytes.as_ptr().cast::<c_char>()
}

const fn fixture_tm() -> tm {
  tm {
    tm_sec: 5,
    tm_min: 4,
    tm_hour: 3,
    tm_mday: 2,
    tm_mon: 0,
    tm_year: 124,
    tm_wday: 2,
    tm_yday: 1,
    tm_isdst: 0,
    tm_gmtoff: 0,
    tm_zone: ptr::null(),
  }
}

fn c_string_prefix(buffer: &[u8]) -> &[u8] {
  let Some(end) = buffer.iter().position(|&byte| byte == 0) else {
    return buffer;
  };

  &buffer[..end]
}

fn read_errno() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe { errno_ptr.read() }
}

fn write_errno(value: c_int) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe {
    errno_ptr.write(value);
  }
}

fn run_strftime(format: &[u8], time_parts: &tm, cap: usize) -> (usize, Vec<u8>) {
  let mut output = vec![b'X'; cap.max(1)];
  // SAFETY: pointers originate from live Rust allocations and `format` is
  // always passed as a NUL-terminated byte string in this test module.
  let written = unsafe {
    strftime(
      output.as_mut_ptr().cast::<c_char>(),
      sz(cap),
      as_c_char_ptr(format),
      core::ptr::from_ref(time_parts),
    )
  };
  let written = usize::try_from(written)
    .unwrap_or_else(|_| unreachable!("size_t return must fit usize on this target"));

  (written, output)
}

#[test]
fn strftime_formats_minimal_tokens_for_c_locale_fixture() {
  let expected = b"2024-01-02 03:04:05";
  let (written, output) = run_strftime(b"%Y-%m-%d %H:%M:%S\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_literal_percent_and_mixed_text() {
  let expected = b"stamp % 2024 done";
  let (written, output) = run_strftime(b"stamp %% %Y done\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_extended_tokens_in_c_locale() {
  let expected = b"2024-01-02|03:04:05|03:04|Tue|Tuesday|Jan|January|002|2";
  let (written, output) = run_strftime(b"%F|%T|%R|%a|%A|%b|%B|%j|%w\0", &fixture_tm(), 128);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_returns_zero_and_keeps_buffer_when_max_is_zero() {
  let mut output = [0xA5_u8; 8];
  let time_parts = fixture_tm();
  // SAFETY: pointers originate from live values and `format` is NUL-terminated.
  let written = unsafe {
    strftime(
      output.as_mut_ptr().cast::<c_char>(),
      sz(0),
      as_c_char_ptr(b"%Y\0"),
      core::ptr::from_ref(&time_parts),
    )
  };

  assert_eq!(written, 0);
  assert_eq!(output, [0xA5_u8; 8]);
}

#[test]
fn strftime_exact_fit_writes_nul_terminator() {
  let expected = b"2024-01-02";
  let cap = expected.len() + 1;
  let (written, output) = run_strftime(b"%F\0", &fixture_tm(), cap);

  assert_eq!(written, expected.len());
  assert_eq!(&output[..expected.len()], expected);
  assert_eq!(output[expected.len()], 0);
}

#[test]
fn strftime_one_byte_short_returns_zero_with_valid_prefix() {
  let expected = b"2024-01-02";
  let cap = expected.len();
  let (written, output) = run_strftime(b"%F\0", &fixture_tm(), cap);

  assert_eq!(written, 0);
  assert_eq!(&output[..cap - 1], &expected[..cap - 1]);
  assert_eq!(output[cap - 1], 0);
}

#[test]
fn strftime_composite_tokens_expand_without_parser_recursion() {
  let expected = b"2024-01-02 03:04:05 03:04";
  let (written, output) = run_strftime(b"%F %T %R\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_trailing_percent_is_treated_as_literal_percent() {
  let expected = b"tail%";
  let (written, output) = run_strftime(b"tail%\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_unknown_token_is_emitted_verbatim() {
  let expected = b"left %q right";
  let (written, output) = run_strftime(b"left %q right\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_weekday_and_month_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = -1;
  time_parts.tm_mon = 99;

  let expected = b"?|?|?|?";
  let (written, output) = run_strftime(b"%a|%A|%b|%B\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_max_one_returns_zero_and_writes_only_nul() {
  let (written, output) = run_strftime(b"abcdef\0", &fixture_tm(), 1);

  assert_eq!(written, 0);
  assert_eq!(output, vec![0]);
}

#[test]
fn strftime_truncation_writes_valid_prefix_and_nul() {
  let cap = 5;
  let (written, output) = run_strftime(b"abcdef\0", &fixture_tm(), cap);

  assert_eq!(written, 0);
  assert_eq!(&output[..cap - 1], b"abcd");
  assert_eq!(output[cap - 1], 0);
}

#[test]
fn strftime_formats_negative_year_with_sign_and_padding() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = -1901;

  let expected = b"-0001";
  let (written, output) = run_strftime(b"%Y\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_five_digit_year_without_truncation() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 8100;

  let expected = b"10000";
  let (written, output) = run_strftime(b"%Y\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_julian_day_with_three_digit_padding_at_bounds() {
  let mut day_one = fixture_tm();

  day_one.tm_yday = 0;

  let mut day_last = fixture_tm();

  day_last.tm_yday = 365;

  let first_expected = b"001";
  let last_expected = b"366";
  let (first_written, first_output) = run_strftime(b"%j\0", &day_one, 8);
  let (last_written, last_output) = run_strftime(b"%j\0", &day_last, 8);

  assert_eq!(first_written, first_expected.len());
  assert_eq!(c_string_prefix(&first_output), first_expected);
  assert_eq!(last_written, last_expected.len());
  assert_eq!(c_string_prefix(&last_output), last_expected);
}

#[test]
fn strftime_invalid_julian_day_fallback_to_question_mark() {
  let mut negative = fixture_tm();

  negative.tm_yday = -1;

  let mut too_large = fixture_tm();

  too_large.tm_yday = 366;

  let expected = b"?";
  let (negative_written, negative_output) = run_strftime(b"%j\0", &negative, 8);
  let (too_large_written, too_large_output) = run_strftime(b"%j\0", &too_large, 8);

  assert_eq!(negative_written, expected.len());
  assert_eq!(c_string_prefix(&negative_output), expected);
  assert_eq!(too_large_written, expected.len());
  assert_eq!(c_string_prefix(&too_large_output), expected);
}

#[test]
fn strftime_julian_day_rejects_leap_day_index_for_non_leap_year() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 123;
  time_parts.tm_yday = 365;

  let expected = b"?";
  let (written, output) = run_strftime(b"%j\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_additional_posix_tokens() {
  let expected = b"20|24|01/02/24|2|\n|\t";
  let (written, output) = run_strftime(b"%C|%y|%D|%u|%n|%t\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_alternative_modifiers_in_c_locale() {
  let expected = b"2024|2|01|02|03|04|05|2|01|Tue Jan  2 03:04:05 2024|01/02/24|03:04:05";
  let (written, output) = run_strftime(
    b"%EY|%Eu|%Om|%Od|%OH|%OM|%OS|%Ou|%OV|%Ec|%Ex|%EX\0",
    &fixture_tm(),
    160,
  );

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_alternative_modifier_u_keeps_sunday_mapping() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = 0;

  let expected = b"7";
  let (written, output) = run_strftime(b"%Eu\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_alternative_modifier_s_aliases_epoch_seconds_token() {
  let expected = b"1704164645|1704164645";
  let (written, output) = run_strftime(b"%Es|%s\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_alternative_modifier_s_aliases_negative_epoch_seconds() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 69;
  time_parts.tm_mon = 11;
  time_parts.tm_mday = 31;
  time_parts.tm_hour = 23;
  time_parts.tm_min = 59;
  time_parts.tm_sec = 59;

  let expected = b"-1|-1";
  let (written, output) = run_strftime(b"%Es|%s\0", &time_parts, 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_alternative_modifier_n_and_t_alias_control_tokens() {
  let expected = b"\n|\n|\t|\t";
  let (written, output) = run_strftime(b"%En|%n|%Et|%t\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_e_alternative_modifier_u_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = 7;

  let expected = b"?";
  let (written, output) = run_strftime(b"%Eu\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_unknown_alternative_modifiers_are_emitted_verbatim() {
  let expected = b"%Oq|%Eq";
  let (written, output) = run_strftime(b"%Oq|%Eq\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_supported_year_alternative_modifiers_including_oy_alias() {
  let expected = b"20|24|2024|2024";
  let (written, output) = run_strftime(b"%EC|%Ey|%EY|%OY\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_o_alternative_modifier_c_aliases_century_token() {
  let expected = b"20|20";
  let (written, output) = run_strftime(b"%OC|%C\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_o_alternative_modifier_c_aliases_negative_century_token() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = -1901;

  let expected = b"-1|-1";
  let (written, output) = run_strftime(b"%OC|%C\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_o_alternative_modifier_j_aliases_julian_day_token() {
  let expected = b"002|002";
  let (written, output) = run_strftime(b"%Oj|%j\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_o_alternative_modifier_j_aliases_leap_year_upper_bound() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 124;
  time_parts.tm_yday = 365;

  let expected = b"366|366";
  let (written, output) = run_strftime(b"%Oj|%j\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_o_alternative_modifier_j_aliases_non_leap_year_upper_bound() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 123;
  time_parts.tm_yday = 364;

  let expected = b"365|365";
  let (written, output) = run_strftime(b"%Oj|%j\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_o_alternative_modifier_s_aliases_epoch_seconds_token() {
  let expected = b"1704164645|1704164645";
  let (written, output) = run_strftime(b"%Os|%s\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_o_alternative_modifier_s_aliases_negative_epoch_seconds() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 69;
  time_parts.tm_mon = 11;
  time_parts.tm_mday = 31;
  time_parts.tm_hour = 23;
  time_parts.tm_min = 59;
  time_parts.tm_sec = 59;

  let expected = b"-1|-1";
  let (written, output) = run_strftime(b"%Os|%s\0", &time_parts, 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_o_alternative_modifier_n_and_t_alias_control_tokens() {
  let expected = b"\n|\n|\t|\t";
  let (written, output) = run_strftime(b"%On|%n|%Ot|%t\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_o_alternative_modifier_j_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 123;
  time_parts.tm_yday = 365;

  let expected = b"?";
  let (written, output) = run_strftime(b"%Oj\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_unsupported_o_aliases_for_c_locale_composite_tokens_are_emitted_verbatim() {
  let expected = b"%Ox|%OX|%Oc";
  let (written, output) = run_strftime(b"%Ox|%OX|%Oc\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_unsupported_o_aliases_for_c_locale_composite_tokens_do_not_consume_following_conversions()
 {
  let expected = b"%Ox2024|%OX01|%Oc02";
  let (written, output) = run_strftime(b"%Ox%Y|%OX%m|%Oc%d\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_e_alternative_year_modifiers_for_negative_years() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = -1901;

  let expected = b"-1|99|-0001";
  let (written, output) = run_strftime(b"%EC|%Ey|%EY\0", &time_parts, 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_unsupported_alternative_modifier_combinations_are_emitted_verbatim() {
  let expected = b"%OE|%Ew";
  let (written, output) = run_strftime(b"%OE|%Ew\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_unsupported_alternative_modifiers_do_not_consume_following_conversions() {
  let expected = b"%EO2024|%OE01|%Oq02";
  let (written, output) = run_strftime(b"%EO%Y|%OE%m|%Oq%d\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_and_o_alternative_modifier_z_aliases_numeric_utc_offset() {
  let mut time_parts = fixture_tm();

  time_parts.tm_gmtoff = 9 * 3_600;

  let expected = b"+0900|+0900|+0900";
  let (written, output) = run_strftime(b"%Ez|%Oz|%z\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_and_o_alternative_modifier_z_aliases_negative_half_hour_offset() {
  let mut time_parts = fixture_tm();

  time_parts.tm_gmtoff = -(30 * 60);

  let expected = b"-0030|-0030|-0030";
  let (written, output) = run_strftime(b"%Ez|%Oz|%z\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_and_o_alternative_modifier_z_aliases_zero_offset() {
  let mut time_parts = fixture_tm();

  time_parts.tm_gmtoff = 0;

  let expected = b"+0000|+0000|+0000";
  let (written, output) = run_strftime(b"%Ez|%Oz|%z\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_and_o_alternative_modifier_z_aliases_single_minute_offsets() {
  let mut east_one_minute = fixture_tm();
  let mut west_one_minute = fixture_tm();

  east_one_minute.tm_gmtoff = 60;
  west_one_minute.tm_gmtoff = -60;

  let east_expected = b"+0001|+0001|+0001";
  let west_expected = b"-0001|-0001|-0001";
  let (east_written, east_output) = run_strftime(b"%Ez|%Oz|%z\0", &east_one_minute, 32);
  let (west_written, west_output) = run_strftime(b"%Ez|%Oz|%z\0", &west_one_minute, 32);

  assert_eq!(east_written, east_expected.len());
  assert_eq!(c_string_prefix(&east_output), east_expected);
  assert_eq!(west_written, west_expected.len());
  assert_eq!(c_string_prefix(&west_output), west_expected);
}

#[test]
fn strftime_e_and_o_alternative_modifier_z_aliases_supported_bounds() {
  let mut east_max = fixture_tm();
  let mut west_max = fixture_tm();

  east_max.tm_gmtoff = 23 * 3_600 + 59 * 60;
  west_max.tm_gmtoff = -(23 * 3_600 + 59 * 60);

  let east_expected = b"+2359|+2359|+2359";
  let west_expected = b"-2359|-2359|-2359";
  let (east_written, east_output) = run_strftime(b"%Ez|%Oz|%z\0", &east_max, 32);
  let (west_written, west_output) = run_strftime(b"%Ez|%Oz|%z\0", &west_max, 32);

  assert_eq!(east_written, east_expected.len());
  assert_eq!(c_string_prefix(&east_output), east_expected);
  assert_eq!(west_written, west_expected.len());
  assert_eq!(c_string_prefix(&west_output), west_expected);
}

#[test]
fn strftime_invalid_e_and_o_alternative_modifier_z_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_gmtoff = 1;

  let expected = b"?|?|?";
  let (written, output) = run_strftime(b"%Ez|%Oz|%z\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_e_and_o_alternative_modifier_z_reject_out_of_range_offsets() {
  let mut too_large = fixture_tm();
  let mut too_small = fixture_tm();

  too_large.tm_gmtoff = 24 * 3_600;
  too_small.tm_gmtoff = -(24 * 3_600);

  let expected = b"?|?|?";
  let (too_large_written, too_large_output) = run_strftime(b"%Ez|%Oz|%z\0", &too_large, 16);
  let (too_small_written, too_small_output) = run_strftime(b"%Ez|%Oz|%z\0", &too_small, 16);

  assert_eq!(too_large_written, expected.len());
  assert_eq!(c_string_prefix(&too_large_output), expected);
  assert_eq!(too_small_written, expected.len());
  assert_eq!(c_string_prefix(&too_small_output), expected);
}

#[test]
fn strftime_e_and_o_alternative_modifier_z_reject_near_boundary_non_minute_offsets() {
  let mut time_parts = fixture_tm();

  time_parts.tm_gmtoff = 23 * 3_600 + 58 * 60 + 59;

  let expected = b"?|?|?";
  let (written, output) = run_strftime(b"%Ez|%Oz|%z\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_percent_after_alternative_modifier_is_emitted_verbatim() {
  let expected = b"%E%|%O%";
  let (written, output) = run_strftime(b"%E%|%O%\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_additional_o_alternative_modifiers_in_c_locale() {
  let expected = b" 2|03|2|24";
  let (written, output) = run_strftime(b"%Oe|%OI|%Ow|%Oy\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_o_alternative_space_padded_hour_tokens() {
  let mut morning = fixture_tm();

  morning.tm_hour = 3;

  let mut afternoon = fixture_tm();

  afternoon.tm_hour = 15;

  let morning_expected = b" 3| 3| 3| 3";
  let afternoon_expected = b"15| 3|15| 3";
  let (morning_written, morning_output) = run_strftime(b"%Ok|%Ol|%k|%l\0", &morning, 32);
  let (afternoon_written, afternoon_output) = run_strftime(b"%Ok|%Ol|%k|%l\0", &afternoon, 32);

  assert_eq!(morning_written, morning_expected.len());
  assert_eq!(c_string_prefix(&morning_output), morning_expected);
  assert_eq!(afternoon_written, afternoon_expected.len());
  assert_eq!(c_string_prefix(&afternoon_output), afternoon_expected);
}

#[test]
fn strftime_formats_o_alternative_space_padded_hour_tokens_at_boundaries() {
  let mut midnight = fixture_tm();
  let mut noon = fixture_tm();

  midnight.tm_hour = 0;
  noon.tm_hour = 12;

  let midnight_expected = b" 0|12| 0|12";
  let noon_expected = b"12|12|12|12";
  let (midnight_written, midnight_output) = run_strftime(b"%Ok|%Ol|%k|%l\0", &midnight, 32);
  let (noon_written, noon_output) = run_strftime(b"%Ok|%Ol|%k|%l\0", &noon, 32);

  assert_eq!(midnight_written, midnight_expected.len());
  assert_eq!(c_string_prefix(&midnight_output), midnight_expected);
  assert_eq!(noon_written, noon_expected.len());
  assert_eq!(c_string_prefix(&noon_output), noon_expected);
}

#[test]
fn strftime_formats_o_alternative_meridiem_and_clock_alias_tokens() {
  let mut morning = fixture_tm();

  morning.tm_hour = 3;

  let mut afternoon = fixture_tm();

  afternoon.tm_hour = 15;

  let morning_expected = b"AM|AM|am|am|03:04|03:04|03:04:05|03:04:05|03:04:05 AM|03:04:05 AM";
  let afternoon_expected = b"PM|PM|pm|pm|15:04|15:04|15:04:05|15:04:05|03:04:05 PM|03:04:05 PM";
  let (morning_written, morning_output) =
    run_strftime(b"%Op|%p|%OP|%P|%OR|%R|%OT|%T|%Or|%r\0", &morning, 128);
  let (afternoon_written, afternoon_output) =
    run_strftime(b"%Op|%p|%OP|%P|%OR|%R|%OT|%T|%Or|%r\0", &afternoon, 128);

  assert_eq!(morning_written, morning_expected.len());
  assert_eq!(c_string_prefix(&morning_output), morning_expected);
  assert_eq!(afternoon_written, afternoon_expected.len());
  assert_eq!(c_string_prefix(&afternoon_output), afternoon_expected);
}

#[test]
fn strftime_formats_o_alternative_meridiem_and_clock_alias_tokens_at_boundaries() {
  let mut midnight = fixture_tm();
  let mut noon = fixture_tm();

  midnight.tm_hour = 0;
  noon.tm_hour = 12;

  let midnight_expected = b"AM|AM|am|am|00:04|00:04|00:04:05|00:04:05|12:04:05 AM|12:04:05 AM";
  let noon_expected = b"PM|PM|pm|pm|12:04|12:04|12:04:05|12:04:05|12:04:05 PM|12:04:05 PM";
  let (midnight_written, midnight_output) =
    run_strftime(b"%Op|%p|%OP|%P|%OR|%R|%OT|%T|%Or|%r\0", &midnight, 128);
  let (noon_written, noon_output) =
    run_strftime(b"%Op|%p|%OP|%P|%OR|%R|%OT|%T|%Or|%r\0", &noon, 128);

  assert_eq!(midnight_written, midnight_expected.len());
  assert_eq!(c_string_prefix(&midnight_output), midnight_expected);
  assert_eq!(noon_written, noon_expected.len());
  assert_eq!(c_string_prefix(&noon_output), noon_expected);
}

#[test]
fn strftime_invalid_hour_for_o_alternative_meridiem_and_clock_aliases_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_hour = 24;

  let expected = b"?|?|?:04|?:04:05|?:04:05 ?";
  let (written, output) = run_strftime(b"%Op|%OP|%OR|%OT|%Or\0", &time_parts, 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_o_alternative_space_padded_hour_tokens_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_hour = 24;

  let expected = b"?|?";
  let (written, output) = run_strftime(b"%Ok|%Ol\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_trailing_alternative_modifiers_are_emitted_verbatim() {
  let expected = b"tail%E|%O";
  let (written, output) = run_strftime(b"tail%E|%O\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_alternative_week_number_modifiers() {
  let expected = b"00|01";
  let (written, output) = run_strftime(b"%OU|%OW\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_alternative_week_number_modifiers_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 123;
  time_parts.tm_yday = 365;
  time_parts.tm_wday = 0;

  let expected = b"?|?";
  let (written, output) = run_strftime(b"%OU|%OW\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_ou_and_ov_alternative_modifiers_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = -1;
  time_parts.tm_yday = 400;

  let expected = b"?|?";
  let (written, output) = run_strftime(b"%Ou|%OV\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_alternative_iso_week_number_modifier_matches_year_boundaries() {
  let mut january_first_2023 = fixture_tm();

  january_first_2023.tm_year = 123;
  january_first_2023.tm_mon = 0;
  january_first_2023.tm_mday = 1;
  january_first_2023.tm_wday = 0;
  january_first_2023.tm_yday = 0;

  let mut december_thirty_first_2018 = fixture_tm();

  december_thirty_first_2018.tm_year = 118;
  december_thirty_first_2018.tm_mon = 11;
  december_thirty_first_2018.tm_mday = 31;
  december_thirty_first_2018.tm_wday = 1;
  december_thirty_first_2018.tm_yday = 364;

  let january_first_expected = b"52";
  let december_thirty_first_expected = b"01";
  let (january_first_written, january_first_output) =
    run_strftime(b"%OV\0", &january_first_2023, 16);
  let (december_thirty_first_written, december_thirty_first_output) =
    run_strftime(b"%OV\0", &december_thirty_first_2018, 16);

  assert_eq!(january_first_written, january_first_expected.len());
  assert_eq!(
    c_string_prefix(&january_first_output),
    january_first_expected
  );
  assert_eq!(
    december_thirty_first_written,
    december_thirty_first_expected.len()
  );
  assert_eq!(
    c_string_prefix(&december_thirty_first_output),
    december_thirty_first_expected
  );
}

#[test]
fn strftime_alternative_iso_week_year_modifiers_match_base_tokens() {
  let expected = b"2024|24|2024|24";
  let (written, output) = run_strftime(b"%OG|%Og|%G|%g\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_alternative_iso_week_year_modifiers_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = 7;
  time_parts.tm_yday = 400;

  let expected = b"?|?|?|?";
  let (written, output) = run_strftime(b"%OG|%Og|%G|%g\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_alternative_weekday_modifiers_keep_sunday_mapping() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = 0;

  let expected = b"7|0";
  let (written, output) = run_strftime(b"%Ou|%Ow\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_ow_alternative_modifier_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = 7;

  let expected = b"?";
  let (written, output) = run_strftime(b"%Ow\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_ou_alternative_modifier_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = 7;

  let expected = b"?";
  let (written, output) = run_strftime(b"%Ou\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_numeric_o_alternative_modifiers_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_mon = 12;
  time_parts.tm_mday = 0;
  time_parts.tm_hour = 24;
  time_parts.tm_min = 60;
  time_parts.tm_sec = 61;

  let expected = b"?|?|?|?|?|?";
  let (written, output) = run_strftime(b"%Om|%Od|%OH|%OI|%OM|%OS\0", &time_parts, 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_oe_alternative_modifier_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_mday = 0;

  let expected = b"?";
  let (written, output) = run_strftime(b"%Oe\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_alternative_week_number_modifiers_allow_last_day_index_for_leap_year() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 124;
  time_parts.tm_yday = 365;
  time_parts.tm_wday = 2;

  let expected = b"52|53";
  let (written, output) = run_strftime(b"%OU|%OW\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_posix_date_representation_token_v() {
  let expected = b" 2-Jan-2024";
  let (written, output) = run_strftime(b"%v\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_components_for_token_v_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_mday = 0;
  time_parts.tm_mon = 12;

  let expected = b"?-?-2024";
  let (written, output) = run_strftime(b"%v\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_posix_week_number_tokens_u_and_w() {
  let expected = b"00|01";
  let (written, output) = run_strftime(b"%U|%W\0", &fixture_tm(), 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_iso_week_number_token_v_uppercase() {
  let expected = b"01";
  let (written, output) = run_strftime(b"%V\0", &fixture_tm(), 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_iso_week_number_token_handles_year_boundaries() {
  let mut january_first_2023 = fixture_tm();

  january_first_2023.tm_year = 123;
  january_first_2023.tm_mon = 0;
  january_first_2023.tm_mday = 1;
  january_first_2023.tm_wday = 0;
  january_first_2023.tm_yday = 0;

  let mut december_thirty_first_2018 = fixture_tm();

  december_thirty_first_2018.tm_year = 118;
  december_thirty_first_2018.tm_mon = 11;
  december_thirty_first_2018.tm_mday = 31;
  december_thirty_first_2018.tm_wday = 1;
  december_thirty_first_2018.tm_yday = 364;

  let january_first_expected = b"52";
  let december_thirty_first_expected = b"01";
  let (january_first_written, january_first_output) =
    run_strftime(b"%V\0", &january_first_2023, 16);
  let (december_thirty_first_written, december_thirty_first_output) =
    run_strftime(b"%V\0", &december_thirty_first_2018, 16);

  assert_eq!(january_first_written, january_first_expected.len());
  assert_eq!(
    c_string_prefix(&january_first_output),
    january_first_expected
  );
  assert_eq!(
    december_thirty_first_written,
    december_thirty_first_expected.len()
  );
  assert_eq!(
    c_string_prefix(&december_thirty_first_output),
    december_thirty_first_expected
  );
}

#[test]
fn strftime_invalid_iso_week_number_inputs_fallback_to_question_mark() {
  let mut invalid_day_of_year = fixture_tm();
  let mut invalid_weekday = fixture_tm();

  invalid_day_of_year.tm_yday = 366;
  invalid_weekday.tm_wday = 7;

  let expected = b"?";
  let (invalid_day_of_year_written, invalid_day_of_year_output) =
    run_strftime(b"%V\0", &invalid_day_of_year, 16);
  let (invalid_weekday_written, invalid_weekday_output) =
    run_strftime(b"%V\0", &invalid_weekday, 16);

  assert_eq!(invalid_day_of_year_written, expected.len());
  assert_eq!(c_string_prefix(&invalid_day_of_year_output), expected);
  assert_eq!(invalid_weekday_written, expected.len());
  assert_eq!(c_string_prefix(&invalid_weekday_output), expected);
}

#[test]
fn strftime_posix_week_number_tokens_handle_year_start_boundary() {
  let mut time_parts = fixture_tm();

  time_parts.tm_yday = 0;
  time_parts.tm_wday = 0;

  let expected = b"01|00";
  let (written, output) = run_strftime(b"%U|%W\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_week_number_inputs_above_range_fallback_to_question_mark() {
  let mut invalid_weekday = fixture_tm();
  let mut invalid_day_of_year = fixture_tm();

  invalid_weekday.tm_wday = 7;
  invalid_day_of_year.tm_yday = 366;

  let expected = b"?|?";
  let (invalid_weekday_written, invalid_weekday_output) =
    run_strftime(b"%U|%W\0", &invalid_weekday, 16);
  let (invalid_day_of_year_written, invalid_day_of_year_output) =
    run_strftime(b"%U|%W\0", &invalid_day_of_year, 16);

  assert_eq!(invalid_weekday_written, expected.len());
  assert_eq!(c_string_prefix(&invalid_weekday_output), expected);
  assert_eq!(invalid_day_of_year_written, expected.len());
  assert_eq!(c_string_prefix(&invalid_day_of_year_output), expected);
}

#[test]
fn strftime_invalid_week_number_yday_for_non_leap_year_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 123;
  time_parts.tm_yday = 365;
  time_parts.tm_wday = 0;

  let expected = b"?|?";
  let (written, output) = run_strftime(b"%U|%W\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_week_number_tokens_allow_last_day_index_for_leap_year() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 124;
  time_parts.tm_yday = 365;
  time_parts.tm_wday = 2;

  let expected = b"52|53";
  let (written, output) = run_strftime(b"%U|%W\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_iso_week_year_and_week_number_tokens() {
  let expected = b"2024|24|01";
  let (written, output) = run_strftime(b"%G|%g|%V\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_iso_week_year_tokens_with_century_rollover() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 100;
  time_parts.tm_mon = 0;
  time_parts.tm_mday = 3;
  time_parts.tm_wday = 1;
  time_parts.tm_yday = 2;

  let expected = b"2000|00|01";
  let (written, output) = run_strftime(b"%G|%g|%V\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_iso_week_tokens_handle_cross_year_boundaries() {
  let mut january_first_2021 = fixture_tm();

  january_first_2021.tm_year = 121;
  january_first_2021.tm_mon = 0;
  january_first_2021.tm_mday = 1;
  january_first_2021.tm_wday = 5;
  january_first_2021.tm_yday = 0;

  let mut december_thirty_first_2018 = fixture_tm();

  december_thirty_first_2018.tm_year = 118;
  december_thirty_first_2018.tm_mon = 11;
  december_thirty_first_2018.tm_mday = 31;
  december_thirty_first_2018.tm_wday = 1;
  december_thirty_first_2018.tm_yday = 364;

  let first_expected = b"2020|20|53";
  let second_expected = b"2019|19|01";
  let (first_written, first_output) = run_strftime(b"%G|%g|%V\0", &january_first_2021, 32);
  let (second_written, second_output) =
    run_strftime(b"%G|%g|%V\0", &december_thirty_first_2018, 32);

  assert_eq!(first_written, first_expected.len());
  assert_eq!(c_string_prefix(&first_output), first_expected);
  assert_eq!(second_written, second_expected.len());
  assert_eq!(c_string_prefix(&second_output), second_expected);
}

#[test]
fn strftime_iso_week_tokens_handle_leap_year_end_boundary() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 124;
  time_parts.tm_mon = 11;
  time_parts.tm_mday = 31;
  time_parts.tm_wday = 2;
  time_parts.tm_yday = 365;

  let expected = b"2025|25|01";
  let (written, output) = run_strftime(b"%G|%g|%V\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_iso_week_inputs_fallback_to_question_mark() {
  let mut invalid_weekday = fixture_tm();
  let mut invalid_day_of_year_non_leap = fixture_tm();

  invalid_weekday.tm_wday = 7;
  invalid_day_of_year_non_leap.tm_year = 123;
  invalid_day_of_year_non_leap.tm_yday = 365;
  invalid_day_of_year_non_leap.tm_wday = 0;

  let expected = b"?|?|?";
  let (invalid_weekday_written, invalid_weekday_output) =
    run_strftime(b"%G|%g|%V\0", &invalid_weekday, 32);
  let (invalid_day_of_year_written, invalid_day_of_year_output) =
    run_strftime(b"%G|%g|%V\0", &invalid_day_of_year_non_leap, 32);

  assert_eq!(invalid_weekday_written, expected.len());
  assert_eq!(c_string_prefix(&invalid_weekday_output), expected);
  assert_eq!(invalid_day_of_year_written, expected.len());
  assert_eq!(c_string_prefix(&invalid_day_of_year_output), expected);
}

#[test]
fn strftime_formats_epoch_seconds_token_s() {
  let expected = b"1704164645";
  let (written, output) = run_strftime(b"%s\0", &fixture_tm(), 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_negative_epoch_seconds_for_pre_unix_epoch_datetime() {
  let mut time_parts = fixture_tm();

  time_parts.tm_year = 69;
  time_parts.tm_mon = 11;
  time_parts.tm_mday = 31;
  time_parts.tm_hour = 23;
  time_parts.tm_min = 59;
  time_parts.tm_sec = 59;

  let expected = b"-1";
  let (written, output) = run_strftime(b"%s\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_epoch_seconds_token_s_matches_timegm_normalization_for_out_of_range_fields() {
  let mut time_parts = fixture_tm();

  time_parts.tm_mon = 14;
  time_parts.tm_mday = 0;
  time_parts.tm_hour = -1;
  time_parts.tm_min = 61;
  time_parts.tm_sec = -30;

  let mut timegm_input = time_parts;
  // SAFETY: `timegm_input` is a valid mutable `tm` pointer.
  let expected_seconds = unsafe { timegm(core::ptr::from_mut(&mut timegm_input)) };
  let expected = expected_seconds.to_string();
  let (written, output) = run_strftime(b"%s\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected.as_bytes());
}

#[test]
fn strftime_formats_numeric_utc_offset_token_z() {
  let mut utc = fixture_tm();
  let mut jst = fixture_tm();
  let mut est = fixture_tm();

  utc.tm_gmtoff = 0;
  jst.tm_gmtoff = 9 * 3_600;
  est.tm_gmtoff = -(5 * 3_600 + 30 * 60);

  let utc_expected = b"+0000";
  let jst_expected = b"+0900";
  let est_expected = b"-0530";
  let (utc_written, utc_output) = run_strftime(b"%z\0", &utc, 16);
  let (jst_written, jst_output) = run_strftime(b"%z\0", &jst, 16);
  let (est_written, est_output) = run_strftime(b"%z\0", &est, 16);

  assert_eq!(utc_written, utc_expected.len());
  assert_eq!(c_string_prefix(&utc_output), utc_expected);
  assert_eq!(jst_written, jst_expected.len());
  assert_eq!(c_string_prefix(&jst_output), jst_expected);
  assert_eq!(est_written, est_expected.len());
  assert_eq!(c_string_prefix(&est_output), est_expected);
}

#[test]
fn strftime_formats_numeric_utc_offset_token_z_at_supported_bounds() {
  let mut east_max = fixture_tm();
  let mut west_max = fixture_tm();

  east_max.tm_gmtoff = 23 * 3_600 + 59 * 60;
  west_max.tm_gmtoff = -(23 * 3_600 + 59 * 60);

  let east_expected = b"+2359";
  let west_expected = b"-2359";
  let (east_written, east_output) = run_strftime(b"%z\0", &east_max, 16);
  let (west_written, west_output) = run_strftime(b"%z\0", &west_max, 16);

  assert_eq!(east_written, east_expected.len());
  assert_eq!(c_string_prefix(&east_output), east_expected);
  assert_eq!(west_written, west_expected.len());
  assert_eq!(c_string_prefix(&west_output), west_expected);
}

#[test]
fn strftime_formats_numeric_utc_offset_token_z_for_single_minute_offsets() {
  let mut east_one_minute = fixture_tm();
  let mut west_one_minute = fixture_tm();

  east_one_minute.tm_gmtoff = 60;
  west_one_minute.tm_gmtoff = -60;

  let east_expected = b"+0001";
  let west_expected = b"-0001";
  let (east_written, east_output) = run_strftime(b"%z\0", &east_one_minute, 16);
  let (west_written, west_output) = run_strftime(b"%z\0", &west_one_minute, 16);

  assert_eq!(east_written, east_expected.len());
  assert_eq!(c_string_prefix(&east_output), east_expected);
  assert_eq!(west_written, west_expected.len());
  assert_eq!(c_string_prefix(&west_output), west_expected);
}

#[test]
fn strftime_formats_numeric_utc_offset_token_z_for_subhour_offsets() {
  let mut east_fifty_nine_minutes = fixture_tm();
  let mut west_fifty_nine_minutes = fixture_tm();

  east_fifty_nine_minutes.tm_gmtoff = 59 * 60;
  west_fifty_nine_minutes.tm_gmtoff = -(59 * 60);

  let east_expected = b"+0059";
  let west_expected = b"-0059";
  let (east_written, east_output) = run_strftime(b"%z\0", &east_fifty_nine_minutes, 16);
  let (west_written, west_output) = run_strftime(b"%z\0", &west_fifty_nine_minutes, 16);

  assert_eq!(east_written, east_expected.len());
  assert_eq!(c_string_prefix(&east_output), east_expected);
  assert_eq!(west_written, west_expected.len());
  assert_eq!(c_string_prefix(&west_output), west_expected);
}

#[test]
fn strftime_formats_numeric_utc_offset_token_z_for_hour_only_offsets() {
  let mut east_one_hour = fixture_tm();
  let mut west_one_hour = fixture_tm();

  east_one_hour.tm_gmtoff = 3_600;
  west_one_hour.tm_gmtoff = -3_600;

  let east_expected = b"+0100";
  let west_expected = b"-0100";
  let (east_written, east_output) = run_strftime(b"%z\0", &east_one_hour, 16);
  let (west_written, west_output) = run_strftime(b"%z\0", &west_one_hour, 16);

  assert_eq!(east_written, east_expected.len());
  assert_eq!(c_string_prefix(&east_output), east_expected);
  assert_eq!(west_written, west_expected.len());
  assert_eq!(c_string_prefix(&west_output), west_expected);
}

#[test]
fn strftime_formats_numeric_utc_offset_token_z_for_half_hour_offsets() {
  let mut east_half_hour = fixture_tm();
  let mut west_half_hour = fixture_tm();

  east_half_hour.tm_gmtoff = 30 * 60;
  west_half_hour.tm_gmtoff = -(30 * 60);

  let east_expected = b"+0030";
  let west_expected = b"-0030";
  let (east_written, east_output) = run_strftime(b"%z\0", &east_half_hour, 16);
  let (west_written, west_output) = run_strftime(b"%z\0", &west_half_hour, 16);

  assert_eq!(east_written, east_expected.len());
  assert_eq!(c_string_prefix(&east_output), east_expected);
  assert_eq!(west_written, west_expected.len());
  assert_eq!(c_string_prefix(&west_output), west_expected);
}

#[test]
fn strftime_formats_numeric_utc_offset_token_z_for_quarter_hour_offsets() {
  let mut east_twelve_forty_five = fixture_tm();
  let mut west_twelve_forty_five = fixture_tm();

  east_twelve_forty_five.tm_gmtoff = 12 * 3_600 + 45 * 60;
  west_twelve_forty_five.tm_gmtoff = -(12 * 3_600 + 45 * 60);

  let east_expected = b"+1245";
  let west_expected = b"-1245";
  let (east_written, east_output) = run_strftime(b"%z\0", &east_twelve_forty_five, 16);
  let (west_written, west_output) = run_strftime(b"%z\0", &west_twelve_forty_five, 16);

  assert_eq!(east_written, east_expected.len());
  assert_eq!(c_string_prefix(&east_output), east_expected);
  assert_eq!(west_written, west_expected.len());
  assert_eq!(c_string_prefix(&west_output), west_expected);
}

#[test]
fn strftime_invalid_numeric_utc_offset_fallback_to_question_mark() {
  let mut non_minute = fixture_tm();
  let mut too_large = fixture_tm();
  let mut too_small = fixture_tm();

  non_minute.tm_gmtoff = 1;
  too_large.tm_gmtoff = 24 * 3_600;
  too_small.tm_gmtoff = -(24 * 3_600);

  let expected = b"?";
  let (non_minute_written, non_minute_output) = run_strftime(b"%z\0", &non_minute, 16);
  let (too_large_written, too_large_output) = run_strftime(b"%z\0", &too_large, 16);
  let (too_small_written, too_small_output) = run_strftime(b"%z\0", &too_small, 16);

  assert_eq!(non_minute_written, expected.len());
  assert_eq!(c_string_prefix(&non_minute_output), expected);
  assert_eq!(too_large_written, expected.len());
  assert_eq!(c_string_prefix(&too_large_output), expected);
  assert_eq!(too_small_written, expected.len());
  assert_eq!(c_string_prefix(&too_small_output), expected);
}

#[test]
fn strftime_invalid_numeric_utc_offset_rejects_negative_non_minute_offsets() {
  let mut time_parts = fixture_tm();

  time_parts.tm_gmtoff = -1;

  let expected = b"?";
  let (written, output) = run_strftime(b"%z\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_numeric_utc_offset_rejects_near_boundary_non_minute_offsets() {
  let mut east_near_boundary_non_minute = fixture_tm();
  let mut west_near_boundary_non_minute = fixture_tm();

  east_near_boundary_non_minute.tm_gmtoff = 23 * 3_600 + 58 * 60 + 59;
  west_near_boundary_non_minute.tm_gmtoff = -(23 * 3_600 + 58 * 60 + 59);

  let expected = b"?";
  let (east_written, east_output) = run_strftime(b"%z\0", &east_near_boundary_non_minute, 16);
  let (west_written, west_output) = run_strftime(b"%z\0", &west_near_boundary_non_minute, 16);

  assert_eq!(east_written, expected.len());
  assert_eq!(c_string_prefix(&east_output), expected);
  assert_eq!(west_written, expected.len());
  assert_eq!(c_string_prefix(&west_output), expected);
}

#[test]
fn strftime_formats_posix_week_number_tokens() {
  let mut january_second_2024 = fixture_tm();

  january_second_2024.tm_year = 124;
  january_second_2024.tm_mon = 0;
  january_second_2024.tm_mday = 2;
  january_second_2024.tm_wday = 2;
  january_second_2024.tm_yday = 1;

  let mut january_first_2023 = fixture_tm();

  january_first_2023.tm_year = 123;
  january_first_2023.tm_mon = 0;
  january_first_2023.tm_mday = 1;
  january_first_2023.tm_wday = 0;
  january_first_2023.tm_yday = 0;

  let first_expected = b"00|01";
  let second_expected = b"01|00";
  let (first_written, first_output) = run_strftime(b"%U|%W\0", &january_second_2024, 16);
  let (second_written, second_output) = run_strftime(b"%U|%W\0", &january_first_2023, 16);

  assert_eq!(first_written, first_expected.len());
  assert_eq!(c_string_prefix(&first_output), first_expected);
  assert_eq!(second_written, second_expected.len());
  assert_eq!(c_string_prefix(&second_output), second_expected);
}

#[test]
fn strftime_invalid_week_number_inputs_fallback_to_question_mark() {
  let mut invalid_day_of_year = fixture_tm();
  let mut invalid_weekday = fixture_tm();

  invalid_day_of_year.tm_yday = 366;
  invalid_weekday.tm_wday = -1;

  let expected = b"?|?";
  let (invalid_day_of_year_written, invalid_day_of_year_output) =
    run_strftime(b"%U|%W\0", &invalid_day_of_year, 16);
  let (invalid_weekday_written, invalid_weekday_output) =
    run_strftime(b"%U|%W\0", &invalid_weekday, 16);

  assert_eq!(invalid_day_of_year_written, expected.len());
  assert_eq!(c_string_prefix(&invalid_day_of_year_output), expected);
  assert_eq!(invalid_weekday_written, expected.len());
  assert_eq!(c_string_prefix(&invalid_weekday_output), expected);
}

#[test]
fn strftime_formats_timezone_abbreviation_token_z_uppercase() {
  let mut time_parts = fixture_tm();

  time_parts.tm_zone = as_c_char_ptr(b"JST\0");

  let expected = b"JST";
  let (written, output) = run_strftime(b"%Z\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_null_timezone_abbreviation_writes_empty_output() {
  let mut time_parts = fixture_tm();

  time_parts.tm_zone = ptr::null();

  let expected = b"";
  let (written, output) = run_strftime(b"%Z\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_empty_timezone_abbreviation_writes_empty_output() {
  let mut time_parts = fixture_tm();

  time_parts.tm_zone = as_c_char_ptr(b"\0");

  let expected = b"";
  let (written, output) = run_strftime(b"%Z\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_more_c_locale_alias_tokens() {
  let expected = b"01/02/24|03:04:05|Jan| 2";
  let (written, output) = run_strftime(b"%x|%X|%h|%e\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_c_locale_datetime_token_c() {
  let expected = b"Tue Jan  2 03:04:05 2024";
  let (written, output) = run_strftime(b"%c\0", &fixture_tm(), 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_components_in_c_locale_datetime_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = -1;
  time_parts.tm_mon = 12;
  time_parts.tm_mday = 0;
  time_parts.tm_hour = 24;
  time_parts.tm_min = 60;
  time_parts.tm_sec = 61;

  let expected = b"? ? ? ?:?:? 2024";
  let (written, output) = run_strftime(b"%c\0", &time_parts, 64);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_components_in_alternative_c_locale_aliases_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = -1;
  time_parts.tm_mon = 12;
  time_parts.tm_mday = 0;
  time_parts.tm_hour = 24;
  time_parts.tm_min = 60;
  time_parts.tm_sec = 61;

  let expected = b"? ? ? ?:?:? 2024|?/?/24|?:?:?";
  let (written, output) = run_strftime(b"%Ec|%Ex|%EX\0", &time_parts, 96);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_e_alternative_clock_alias_tokens() {
  let mut morning = fixture_tm();

  morning.tm_hour = 3;

  let mut afternoon = fixture_tm();

  afternoon.tm_hour = 15;

  let morning_expected = b"03:04:05 AM|03:04:05 AM|03:04|03:04";
  let afternoon_expected = b"03:04:05 PM|03:04:05 PM|15:04|15:04";
  let (morning_written, morning_output) = run_strftime(b"%Er|%r|%ER|%R\0", &morning, 64);
  let (afternoon_written, afternoon_output) = run_strftime(b"%Er|%r|%ER|%R\0", &afternoon, 64);

  assert_eq!(morning_written, morning_expected.len());
  assert_eq!(c_string_prefix(&morning_output), morning_expected);
  assert_eq!(afternoon_written, afternoon_expected.len());
  assert_eq!(c_string_prefix(&afternoon_output), afternoon_expected);
}

#[test]
fn strftime_invalid_hour_for_e_alternative_clock_aliases_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_hour = 24;

  let expected = b"?:04:05 ?|?:04";
  let (written, output) = run_strftime(b"%Er|%ER\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_e_alternative_clock_alias_tokens_at_boundaries() {
  let mut midnight = fixture_tm();

  midnight.tm_hour = 0;

  let mut noon = fixture_tm();

  noon.tm_hour = 12;

  let midnight_expected = b"12:04:05 AM|12:04:05 AM|00:04|00:04";
  let noon_expected = b"12:04:05 PM|12:04:05 PM|12:04|12:04";
  let (midnight_written, midnight_output) = run_strftime(b"%Er|%r|%ER|%R\0", &midnight, 64);
  let (noon_written, noon_output) = run_strftime(b"%Er|%r|%ER|%R\0", &noon, 64);

  assert_eq!(midnight_written, midnight_expected.len());
  assert_eq!(c_string_prefix(&midnight_output), midnight_expected);
  assert_eq!(noon_written, noon_expected.len());
  assert_eq!(c_string_prefix(&noon_output), noon_expected);
}

#[test]
fn strftime_formats_e_alternative_meridiem_and_time_alias_tokens() {
  let mut morning = fixture_tm();

  morning.tm_hour = 3;

  let mut afternoon = fixture_tm();

  afternoon.tm_hour = 15;

  let morning_expected = b"AM|AM|03:04:05|03:04:05";
  let afternoon_expected = b"PM|PM|15:04:05|15:04:05";
  let (morning_written, morning_output) = run_strftime(b"%Ep|%p|%ET|%T\0", &morning, 64);
  let (afternoon_written, afternoon_output) = run_strftime(b"%Ep|%p|%ET|%T\0", &afternoon, 64);

  assert_eq!(morning_written, morning_expected.len());
  assert_eq!(c_string_prefix(&morning_output), morning_expected);
  assert_eq!(afternoon_written, afternoon_expected.len());
  assert_eq!(c_string_prefix(&afternoon_output), afternoon_expected);
}

#[test]
fn strftime_e_alternative_uppercase_meridiem_alias_token() {
  let mut morning = fixture_tm();

  morning.tm_hour = 3;

  let mut afternoon = fixture_tm();

  afternoon.tm_hour = 15;

  let morning_expected = b"am|am";
  let afternoon_expected = b"pm|pm";
  let (morning_written, morning_output) = run_strftime(b"%EP|%P\0", &morning, 32);
  let (afternoon_written, afternoon_output) = run_strftime(b"%EP|%P\0", &afternoon, 32);

  assert_eq!(morning_written, morning_expected.len());
  assert_eq!(c_string_prefix(&morning_output), morning_expected);
  assert_eq!(afternoon_written, afternoon_expected.len());
  assert_eq!(c_string_prefix(&afternoon_output), afternoon_expected);
}

#[test]
fn strftime_e_alternative_meridiem_and_time_alias_tokens_at_boundaries() {
  let mut midnight = fixture_tm();

  midnight.tm_hour = 0;

  let mut noon = fixture_tm();

  noon.tm_hour = 12;

  let midnight_expected = b"AM|AM|am|am|00:04:05|00:04:05";
  let noon_expected = b"PM|PM|pm|pm|12:04:05|12:04:05";
  let (midnight_written, midnight_output) =
    run_strftime(b"%Ep|%p|%EP|%P|%ET|%T\0", &midnight, 64);
  let (noon_written, noon_output) = run_strftime(b"%Ep|%p|%EP|%P|%ET|%T\0", &noon, 64);

  assert_eq!(midnight_written, midnight_expected.len());
  assert_eq!(c_string_prefix(&midnight_output), midnight_expected);
  assert_eq!(noon_written, noon_expected.len());
  assert_eq!(c_string_prefix(&noon_output), noon_expected);
}

#[test]
fn strftime_invalid_hour_for_e_alternative_meridiem_and_time_aliases_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_hour = 24;

  let expected = b"?|?:04:05";
  let (written, output) = run_strftime(b"%Ep|%ET\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_hour_for_e_alternative_uppercase_meridiem_alias_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_hour = 24;

  let expected = b"?";
  let (written, output) = run_strftime(b"%EP\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_posix_meridiem_tokens() {
  let mut morning = fixture_tm();

  morning.tm_hour = 3;

  let mut afternoon = fixture_tm();

  afternoon.tm_hour = 15;

  let morning_expected = b"AM|am";
  let afternoon_expected = b"PM|pm";
  let (morning_written, morning_output) = run_strftime(b"%p|%P\0", &morning, 32);
  let (afternoon_written, afternoon_output) = run_strftime(b"%p|%P\0", &afternoon, 32);

  assert_eq!(morning_written, morning_expected.len());
  assert_eq!(c_string_prefix(&morning_output), morning_expected);
  assert_eq!(afternoon_written, afternoon_expected.len());
  assert_eq!(c_string_prefix(&afternoon_output), afternoon_expected);
}

#[test]
fn strftime_invalid_hour_for_meridiem_tokens_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_hour = 24;

  let expected = b"?|?";
  let (written, output) = run_strftime(b"%p|%P\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_posix_twelve_hour_tokens() {
  let mut morning = fixture_tm();

  morning.tm_hour = 3;

  let mut afternoon = fixture_tm();

  afternoon.tm_hour = 15;

  let morning_expected = b"03|03:04:05 AM";
  let afternoon_expected = b"03|03:04:05 PM";
  let (morning_written, morning_output) = run_strftime(b"%I|%r\0", &morning, 32);
  let (afternoon_written, afternoon_output) = run_strftime(b"%I|%r\0", &afternoon, 32);

  assert_eq!(morning_written, morning_expected.len());
  assert_eq!(c_string_prefix(&morning_output), morning_expected);
  assert_eq!(afternoon_written, afternoon_expected.len());
  assert_eq!(c_string_prefix(&afternoon_output), afternoon_expected);
}

#[test]
fn strftime_invalid_hour_for_twelve_hour_tokens_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_hour = 24;

  let expected = b"?|?:04:05 ?";
  let (written, output) = run_strftime(b"%I|%r\0", &time_parts, 32);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_space_padded_hour_tokens() {
  let mut morning = fixture_tm();

  morning.tm_hour = 3;

  let mut afternoon = fixture_tm();

  afternoon.tm_hour = 15;

  let morning_expected = b" 3| 3";
  let afternoon_expected = b"15| 3";
  let (morning_written, morning_output) = run_strftime(b"%k|%l\0", &morning, 32);
  let (afternoon_written, afternoon_output) = run_strftime(b"%k|%l\0", &afternoon, 32);

  assert_eq!(morning_written, morning_expected.len());
  assert_eq!(c_string_prefix(&morning_output), morning_expected);
  assert_eq!(afternoon_written, afternoon_expected.len());
  assert_eq!(c_string_prefix(&afternoon_output), afternoon_expected);
}

#[test]
fn strftime_invalid_hour_for_space_padded_hour_tokens_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_hour = 24;

  let expected = b"?|?";
  let (written, output) = run_strftime(b"%k|%l\0", &time_parts, 16);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_formats_posix_weekday_u_for_sunday_as_seven() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = 0;

  let expected = b"7";
  let (written, output) = run_strftime(b"%u\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_posix_weekday_u_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = 99;

  let expected = b"?";
  let (written, output) = run_strftime(b"%u\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_weekday_w_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_wday = -1;

  let expected = b"?";
  let (written, output) = run_strftime(b"%w\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_day_for_token_e_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_mday = 0;

  let expected = b"?";
  let (written, output) = run_strftime(b"%e\0", &time_parts, 8);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_invalid_numeric_fields_fallback_to_question_mark() {
  let mut time_parts = fixture_tm();

  time_parts.tm_mon = 12;
  time_parts.tm_mday = 0;
  time_parts.tm_hour = 24;
  time_parts.tm_min = -1;
  time_parts.tm_sec = 61;

  let expected = b"?|?|?|?|?|2024-?-?|?:?:?|?:?|?/?/24";
  let (written, output) = run_strftime(b"%m|%d|%H|%M|%S|%F|%T|%R|%D\0", &time_parts, 96);

  assert_eq!(written, expected.len());
  assert_eq!(c_string_prefix(&output), expected);
}

#[test]
fn strftime_null_format_pointer_returns_zero_and_sets_einval() {
  let mut output = [0xCC_u8; 8];
  let time_parts = fixture_tm();

  write_errno(0);
  // SAFETY: `output` and `time_parts` are valid pointers; `format` is intentionally null.
  let written = unsafe {
    strftime(
      output.as_mut_ptr().cast(),
      sz(output.len()),
      ptr::null(),
      core::ptr::from_ref(&time_parts),
    )
  };

  assert_eq!(written, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(output, [0xCC_u8; 8]);
}

#[test]
fn strftime_null_tm_pointer_returns_zero_and_sets_einval() {
  let mut output = [0xCC_u8; 8];

  write_errno(0);
  // SAFETY: `output` and `format` are valid pointers; `time_ptr` is intentionally null.
  let written = unsafe {
    strftime(
      output.as_mut_ptr().cast(),
      sz(output.len()),
      as_c_char_ptr(b"%Y\0"),
      ptr::null(),
    )
  };

  assert_eq!(written, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(output, [0xCC_u8; 8]);
}

#[test]
fn strftime_null_output_pointer_with_non_zero_max_returns_zero_and_sets_einval() {
  let time_parts = fixture_tm();

  write_errno(0);
  // SAFETY: `format` and `time_parts` are valid; `s` is intentionally null with non-zero max.
  let written = unsafe {
    strftime(
      ptr::null_mut(),
      sz(4),
      as_c_char_ptr(b"%Y\0"),
      core::ptr::from_ref(&time_parts),
    )
  };

  assert_eq!(written, 0);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn strftime_null_output_pointer_with_zero_max_keeps_errno() {
  let time_parts = fixture_tm();

  write_errno(77);
  // SAFETY: `format` and `time_parts` are valid; `s` is null but `max == 0` so no writes occur.
  let written = unsafe {
    strftime(
      ptr::null_mut(),
      sz(0),
      as_c_char_ptr(b"%Y\0"),
      core::ptr::from_ref(&time_parts),
    )
  };

  assert_eq!(written, 0);
  assert_eq!(read_errno(), 77);
}
