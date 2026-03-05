//! Minimal time-related C ABI interfaces.
//!
//! This module currently provides:
//! - `clock_gettime` with Linux `clockid_t` forwarding (including exported
//!   `CLOCK_REALTIME`, `CLOCK_MONOTONIC`, `CLOCK_MONOTONIC_RAW`,
//!   `CLOCK_PROCESS_CPUTIME_ID`, `CLOCK_THREAD_CPUTIME_ID`,
//!   `CLOCK_REALTIME_COARSE`, `CLOCK_MONOTONIC_COARSE`, and
//!   `CLOCK_BOOTTIME`/`CLOCK_REALTIME_ALARM`/`CLOCK_BOOTTIME_ALARM`/
//!   `CLOCK_SGI_CYCLE`/`CLOCK_TAI` constants)
//! - `gettimeofday`
//! - calendar conversion core (`gmtime_r`, `gmtime`, `timegm`)
//! - `strftime` (`C`/`POSIX` locale baseline)
//! - C-compatible `timespec`, `timeval`, and `timezone` structs
//!
//! The implementation uses raw Linux syscalls on `x86_64` and sets `errno`
//! for kernel-reported failures.

use crate::abi::errno::{EFAULT, EINVAL, ENOSYS, ERANGE};
use crate::abi::types::{c_char, c_int, c_long, size_t};
use crate::errno::set_errno;
use crate::syscall::syscall2;
use core::cell::UnsafeCell;
use core::ptr;

/// `CLOCK_REALTIME` clock source identifier.
pub const CLOCK_REALTIME: c_int = 0;
/// `CLOCK_MONOTONIC` clock source identifier.
pub const CLOCK_MONOTONIC: c_int = 1;
/// `CLOCK_PROCESS_CPUTIME_ID` clock source identifier.
///
/// This Linux clock tracks CPU time consumed by the current process.
pub const CLOCK_PROCESS_CPUTIME_ID: c_int = 2;
/// `CLOCK_THREAD_CPUTIME_ID` clock source identifier.
///
/// This Linux clock tracks CPU time consumed by the current thread.
pub const CLOCK_THREAD_CPUTIME_ID: c_int = 3;
/// `CLOCKFD` selector used by Linux dynamic clock ids.
///
/// This value is a bit-pattern tag, not a standalone clock source. Build a
/// dynamic clock id from a file descriptor with [`fd_to_clockid`], and extract
/// it again with [`clockid_to_fd`].
pub const CLOCKFD: c_int = 3;
/// `CLOCK_MONOTONIC_RAW` clock source identifier.
///
/// This Linux clock reports a monotonic timesource without NTP adjustments.
pub const CLOCK_MONOTONIC_RAW: c_int = 4;
/// `CLOCK_REALTIME_COARSE` clock source identifier.
///
/// This Linux clock can trade precision for reduced overhead.
pub const CLOCK_REALTIME_COARSE: c_int = 5;
/// `CLOCK_MONOTONIC_COARSE` clock source identifier.
///
/// This Linux clock is monotonic and can trade precision for reduced overhead.
pub const CLOCK_MONOTONIC_COARSE: c_int = 6;
/// `CLOCK_BOOTTIME` clock source identifier.
///
/// This Linux clock includes suspended time, unlike `CLOCK_MONOTONIC`.
pub const CLOCK_BOOTTIME: c_int = 7;
/// `CLOCK_REALTIME_ALARM` clock source identifier.
///
/// This Linux clock tracks `CLOCK_REALTIME` and is used by alarm-capable timer
/// facilities.
pub const CLOCK_REALTIME_ALARM: c_int = 8;
/// `CLOCK_BOOTTIME_ALARM` clock source identifier.
///
/// This Linux clock tracks `CLOCK_BOOTTIME` and is used by alarm-capable timer
/// facilities.
pub const CLOCK_BOOTTIME_ALARM: c_int = 9;
/// `CLOCK_SGI_CYCLE` clock source identifier.
///
/// This legacy Linux clock id is architecture-dependent and may be unsupported
/// on modern kernels.
pub const CLOCK_SGI_CYCLE: c_int = 10;
/// `CLOCK_TAI` clock source identifier.
///
/// This Linux clock reports International Atomic Time and does not include leap
/// second steps applied to `CLOCK_REALTIME`.
pub const CLOCK_TAI: c_int = 11;
const SYS_GETTIMEOFDAY: c_long = 96;
const SYS_CLOCK_GETTIME: c_long = 228;
const SECONDS_PER_DAY: i128 = 86_400;
const MAX_NUMERIC_UTC_OFFSET_SECONDS: u128 = 86_340;
const DAYS_PER_400_YEARS: i128 = 146_097;
const DAYS_BEFORE_MONTH: [c_int; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const C_LOCALE_WEEKDAY_SHORT: [&[u8]; 7] = [b"Sun", b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat"];
const C_LOCALE_WEEKDAY_LONG: [&[u8]; 7] = [
  b"Sunday",
  b"Monday",
  b"Tuesday",
  b"Wednesday",
  b"Thursday",
  b"Friday",
  b"Saturday",
];
const C_LOCALE_MONTH_SHORT: [&[u8]; 12] = [
  b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec",
];
const C_LOCALE_MONTH_LONG: [&[u8]; 12] = [
  b"January",
  b"February",
  b"March",
  b"April",
  b"May",
  b"June",
  b"July",
  b"August",
  b"September",
  b"October",
  b"November",
  b"December",
];

/// Linux `clockid_t` on the primary target.
pub type clockid_t = c_int;

/// Linux `time_t` on the primary target.
pub type time_t = c_long;

/// C-compatible `timespec` layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct timespec {
  /// Whole seconds component.
  pub tv_sec: c_long,
  /// Nanoseconds component (`0..1_000_000_000`).
  pub tv_nsec: c_long,
}

/// C-compatible `timeval` layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct timeval {
  /// Whole seconds component.
  pub tv_sec: c_long,
  /// Microseconds component (`0..1_000_000`).
  pub tv_usec: c_long,
}

/// C-compatible legacy timezone descriptor.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct timezone {
  /// Minutes west of Greenwich.
  pub tz_minuteswest: c_int,
  /// Daylight saving time correction type.
  pub tz_dsttime: c_int,
}

/// C-compatible calendar broken-down time descriptor (`struct tm`).
///
/// Field contracts follow the C/POSIX baseline:
/// - `tm_mon` is in `0..=11` (January = `0`)
/// - `tm_year` is years since 1900
/// - `tm_wday` is in `0..=6` (Sunday = `0`)
/// - `tm_yday` is in `0..=365` (January 1st = `0`)
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct tm {
  /// Seconds after the minute (`0..=60` for leap-second capable inputs).
  pub tm_sec: c_int,
  /// Minutes after the hour (`0..=59`).
  pub tm_min: c_int,
  /// Hours since midnight (`0..=23`).
  pub tm_hour: c_int,
  /// Day of the month (`1..=31`).
  pub tm_mday: c_int,
  /// Months since January (`0..=11`).
  pub tm_mon: c_int,
  /// Years since 1900.
  pub tm_year: c_int,
  /// Days since Sunday (`0..=6`).
  pub tm_wday: c_int,
  /// Days since January 1 (`0..=365`).
  pub tm_yday: c_int,
  /// Daylight Saving Time flag.
  pub tm_isdst: c_int,
  /// Seconds east of UTC (glibc extension; preserved for ABI compatibility).
  pub tm_gmtoff: c_long,
  /// Timezone abbreviation pointer (glibc extension; may be null).
  pub tm_zone: *const c_char,
}

struct OutputBuffer {
  dst: *mut c_char,
  max: usize,
  required: usize,
}

impl OutputBuffer {
  const fn new(dst: *mut c_char, max: usize) -> Self {
    Self {
      dst,
      max,
      required: 0,
    }
  }

  const fn push_byte(&mut self, byte: u8) {
    if self.max != 0 && self.required < self.max - 1 {
      // SAFETY: Writes are bounded to at most `max - 1` bytes, preserving one
      // byte for the trailing NUL terminator.
      unsafe {
        self
          .dst
          .add(self.required)
          .write(c_char::from_ne_bytes([byte]));
      }
    }

    self.required += 1;
  }

  fn push_bytes(&mut self, bytes: &[u8]) {
    for &byte in bytes {
      self.push_byte(byte);
    }
  }

  fn terminate(&mut self) {
    if self.max == 0 {
      return;
    }

    let terminator_index = self.required.min(self.max - 1);

    // SAFETY: `terminator_index` is in-bounds for `[0, max)` when `max > 0`.
    unsafe {
      self.dst.add(terminator_index).write(0);
    }
  }

  fn finish(&mut self) -> size_t {
    self.terminate();

    if self.max == 0 || self.required >= self.max {
      return 0;
    }

    to_size_t(self.required)
  }
}

/// Converts a file descriptor into a Linux dynamic `clockid_t`.
///
/// # Contract
/// - Input should be a non-negative file descriptor.
/// - Output uses Linux dynamic-clock encoding (`CLOCKFD` tagged).
/// - Negative inputs are outside this contract and can alias fixed Linux
///   clock ids (for example `-1` encodes to `CLOCK_THREAD_CPUTIME_ID`).
///
/// The returned value can be passed to [`clock_gettime`]. Kernel support and
/// accepted file descriptor types are platform-dependent.
///
/// Roundtrip behavior with [`clockid_to_fd`] matches Linux macro semantics:
/// on 32-bit `clockid_t`, `fd <= 0x0fff_ffff` is lossless, while larger values
/// can lose high bits through the shift/tag encoding.
#[must_use]
pub const fn fd_to_clockid(fd: c_int) -> clockid_t {
  (!fd << 3) | CLOCKFD
}

/// Extracts the file descriptor encoded in a Linux dynamic `clockid_t`.
///
/// # Contract
/// - `clock_id` should be a value produced by [`fd_to_clockid`] and tagged
///   with [`CLOCKFD`].
/// - For non-dynamic clock ids, the returned value is not meaningful.
/// - If the original encoded input violated [`fd_to_clockid`] contract
///   (negative file descriptor), the decoded value should not be reused as a
///   valid file descriptor.
///
/// For dynamic ids created from large file descriptors, the decoded value may
/// differ from the original input due to Linux shift/tag encoding limits.
#[must_use]
pub const fn clockid_to_fd(clock_id: clockid_t) -> c_int {
  !(clock_id >> 3)
}

const fn unix_epoch_tm() -> tm {
  tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 0,
    tm_year: 70,
    tm_wday: 4,
    tm_yday: 0,
    tm_isdst: 0,
    tm_gmtoff: 0,
    tm_zone: ptr::null(),
  }
}

fn len_from_size_t(n: size_t) -> usize {
  usize::try_from(n)
    .unwrap_or_else(|_| unreachable!("size_t does not fit into usize on this target"))
}

fn to_size_t(n: usize) -> size_t {
  size_t::try_from(n)
    .unwrap_or_else(|_| unreachable!("usize does not fit into size_t on this target"))
}

fn decode_syscall_status(raw: c_long) -> Result<(), c_int> {
  if raw < 0 {
    let errno = c_int::try_from(raw.unsigned_abs()).unwrap_or(c_int::MAX);

    return Err(errno);
  }

  Ok(())
}

fn ptr_to_sys_arg<T>(ptr: *const T) -> c_long {
  c_long::try_from(ptr.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit into c_long on x86_64 Linux"))
}

const fn is_leap_year(year: i128) -> bool {
  (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn day_of_year(year: i128, month: i128, day: i128) -> Option<c_int> {
  let month_index = usize::try_from(month - 1).ok()?;
  let mut yday = i128::from(*DAYS_BEFORE_MONTH.get(month_index)?);

  yday += day - 1;

  if month > 2 && is_leap_year(year) {
    yday += 1;
  }

  c_int::try_from(yday).ok()
}

const fn civil_from_days(days_since_epoch: i128) -> (i128, i128, i128) {
  let shifted_days = days_since_epoch + 719_468;
  let era = if shifted_days >= 0 {
    shifted_days
  } else {
    shifted_days - (DAYS_PER_400_YEARS - 1)
  } / DAYS_PER_400_YEARS;
  let day_of_era = shifted_days - era * DAYS_PER_400_YEARS;
  let year_of_era =
    (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
  let mut year = year_of_era + era * 400;
  let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
  let month_phase = (5 * day_of_year + 2) / 153;
  let day = day_of_year - (153 * month_phase + 2) / 5 + 1;
  let month = month_phase + if month_phase < 10 { 3 } else { -9 };

  if month <= 2 {
    year += 1;
  }

  (year, month, day)
}

const fn days_from_civil(year: i128, month: i128, day: i128) -> i128 {
  let leap_adjustment = if month <= 2 { 1 } else { 0 };
  let adjusted_year = year - leap_adjustment;
  let era = if adjusted_year >= 0 {
    adjusted_year
  } else {
    adjusted_year - 399
  } / 400;
  let year_of_era = adjusted_year - era * 400;
  let month_phase = month + if month > 2 { -3 } else { 9 };
  let day_of_year = (153 * month_phase + 2) / 5 + day - 1;
  let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

  era * DAYS_PER_400_YEARS + day_of_era - 719_468
}

fn tm_from_unix_seconds(seconds: i128) -> Option<tm> {
  let days_since_epoch = seconds.div_euclid(SECONDS_PER_DAY);
  let seconds_within_day = seconds.rem_euclid(SECONDS_PER_DAY);
  let (year, month, day) = civil_from_days(days_since_epoch);
  let tm_year = c_int::try_from(year - 1900).ok()?;
  let tm_yday = day_of_year(year, month, day)?;
  let tm_hour = c_int::try_from(seconds_within_day / 3_600).ok()?;
  let tm_min = c_int::try_from((seconds_within_day % 3_600) / 60).ok()?;
  let tm_sec = c_int::try_from(seconds_within_day % 60).ok()?;
  let month_index = c_int::try_from(month - 1).ok()?;
  let day_of_month = c_int::try_from(day).ok()?;
  let weekday_index = c_int::try_from((days_since_epoch + 4).rem_euclid(7)).ok()?;

  Some(tm {
    tm_sec,
    tm_min,
    tm_hour,
    tm_mday: day_of_month,
    tm_mon: month_index,
    tm_year,
    tm_wday: weekday_index,
    tm_yday,
    tm_isdst: 0,
    tm_gmtoff: 0,
    tm_zone: ptr::null(),
  })
}

fn normalized_tm_to_unix_seconds(time_parts: &tm) -> i128 {
  let mut full_year = i128::from(time_parts.tm_year) + 1900;
  let mut zero_based_month = i128::from(time_parts.tm_mon);

  full_year += zero_based_month.div_euclid(12);
  zero_based_month = zero_based_month.rem_euclid(12);

  let one_based_month = zero_based_month + 1;
  let day_offset = i128::from(time_parts.tm_mday) - 1;
  let day_seconds = i128::from(time_parts.tm_hour) * 3_600
    + i128::from(time_parts.tm_min) * 60
    + i128::from(time_parts.tm_sec);
  let carry_days = day_seconds.div_euclid(SECONDS_PER_DAY);
  let normalized_day_seconds = day_seconds.rem_euclid(SECONDS_PER_DAY);
  let base_days = days_from_civil(full_year, one_based_month, 1);
  let days_since_epoch = base_days + day_offset + carry_days;

  days_since_epoch * SECONDS_PER_DAY + normalized_day_seconds
}

fn normalized_tm_to_unix_seconds_for_mktime(time_parts: &tm) -> i128 {
  let unix_seconds = normalized_tm_to_unix_seconds(time_parts);

  // UTC-baseline local-time policy:
  // - `tm_isdst > 0` is treated as "DST in effect", so local wall-clock input
  //   is interpreted as one hour ahead of standard UTC baseline.
  // - `tm_isdst <= 0` is interpreted as standard/unknown time with no offset
  //   adjustment in this baseline implementation.
  if time_parts.tm_isdst > 0 {
    return unix_seconds - 3_600;
  }

  unix_seconds
}

fn month_name(mon: c_int, long_name: bool) -> &'static [u8] {
  let Ok(index) = usize::try_from(mon) else {
    return b"?";
  };

  if long_name {
    return C_LOCALE_MONTH_LONG.get(index).copied().unwrap_or(b"?");
  }

  C_LOCALE_MONTH_SHORT.get(index).copied().unwrap_or(b"?")
}

fn weekday_name(wday: c_int, long_name: bool) -> &'static [u8] {
  let Ok(index) = usize::try_from(wday) else {
    return b"?";
  };

  if long_name {
    return C_LOCALE_WEEKDAY_LONG.get(index).copied().unwrap_or(b"?");
  }

  C_LOCALE_WEEKDAY_SHORT.get(index).copied().unwrap_or(b"?")
}

const fn max_yday_for_full_year(full_year: i128) -> c_int {
  if is_leap_year(full_year) {
    return 365;
  }

  364
}

const fn is_valid_tm_yday_for_year(full_year: i128, yday: c_int) -> bool {
  if yday < 0 {
    return false;
  }

  yday <= max_yday_for_full_year(full_year)
}

const fn full_year_from_tm_year(tm_year: c_int) -> i128 {
  tm_year as i128 + 1900
}

fn append_year(buffer: &mut OutputBuffer, value: c_int) {
  let full_year = full_year_from_tm_year(value);

  append_full_year(buffer, full_year);
}

fn append_full_year(buffer: &mut OutputBuffer, full_year: i128) {
  if full_year >= 0 {
    buffer.push_bytes(format!("{full_year:04}").as_bytes());

    return;
  }

  let magnitude = full_year.unsigned_abs();

  buffer.push_bytes(format!("-{magnitude:04}").as_bytes());
}

fn append_century(buffer: &mut OutputBuffer, value: c_int) {
  let full_year = full_year_from_tm_year(value);
  let century = full_year.div_euclid(100);

  buffer.push_bytes(format!("{century:02}").as_bytes());
}

fn append_year_two_digits(buffer: &mut OutputBuffer, value: c_int) {
  let full_year = full_year_from_tm_year(value);
  let year_in_century = full_year.rem_euclid(100);

  buffer.push_bytes(format!("{year_in_century:02}").as_bytes());
}

fn append_epoch_seconds(buffer: &mut OutputBuffer, time_parts: &tm) {
  let unix_seconds = normalized_tm_to_unix_seconds(time_parts);

  buffer.push_bytes(format!("{unix_seconds}").as_bytes());
}

const fn is_valid_numeric_utc_offset_seconds(total_seconds: i128) -> bool {
  let absolute_seconds = total_seconds.unsigned_abs();

  absolute_seconds <= MAX_NUMERIC_UTC_OFFSET_SECONDS && absolute_seconds.is_multiple_of(60)
}

fn append_numeric_utc_offset(buffer: &mut OutputBuffer, gmtoff: c_long) {
  let total_seconds = i128::from(gmtoff);

  if !is_valid_numeric_utc_offset_seconds(total_seconds) {
    buffer.push_byte(b'?');

    return;
  }

  let absolute_seconds = total_seconds.unsigned_abs();
  let total_minutes = absolute_seconds / 60;
  let hours = total_minutes / 60;
  let minutes = total_minutes % 60;
  let sign = if total_seconds < 0 { '-' } else { '+' };

  buffer.push_bytes(format!("{sign}{hours:02}{minutes:02}").as_bytes());
}

const fn append_timezone_abbreviation(buffer: &mut OutputBuffer, zone: *const c_char) {
  if zone.is_null() {
    return;
  }

  let mut cursor = zone;

  loop {
    // SAFETY: caller-provided `tm_zone` must be readable and NUL-terminated
    // when non-null.
    let byte = unsafe { cursor.read() }.to_ne_bytes()[0];

    if byte == 0 {
      break;
    }

    buffer.push_byte(byte);
    // SAFETY: advances within the same readable NUL-terminated C string.
    cursor = unsafe { cursor.add(1) };
  }
}

fn append_padded_decimal(buffer: &mut OutputBuffer, value: c_int, width: usize) {
  buffer.push_bytes(format!("{value:0width$}").as_bytes());
}

fn append_padded_decimal_in_range(
  buffer: &mut OutputBuffer,
  value: c_int,
  width: usize,
  min: c_int,
  max: c_int,
) {
  if value < min || value > max {
    buffer.push_byte(b'?');

    return;
  }

  append_padded_decimal(buffer, value, width);
}

fn append_month_number(buffer: &mut OutputBuffer, mon: c_int) {
  match mon {
    0..=11 => append_padded_decimal(buffer, mon + 1, 2),
    _ => buffer.push_byte(b'?'),
  }
}

fn append_day_of_month(buffer: &mut OutputBuffer, mday: c_int) {
  append_padded_decimal_in_range(buffer, mday, 2, 1, 31);
}

fn append_day_of_month_space_padded(buffer: &mut OutputBuffer, mday: c_int) {
  if !(1..=31).contains(&mday) {
    buffer.push_byte(b'?');

    return;
  }

  buffer.push_bytes(format!("{mday:2}").as_bytes());
}

fn append_hour_24(buffer: &mut OutputBuffer, hour: c_int) {
  append_padded_decimal_in_range(buffer, hour, 2, 0, 23);
}

fn append_hour_24_space_padded(buffer: &mut OutputBuffer, hour: c_int) {
  if !(0..=23).contains(&hour) {
    buffer.push_byte(b'?');

    return;
  }

  buffer.push_bytes(format!("{hour:2}").as_bytes());
}

const fn normalize_hour_12(hour: c_int) -> Option<c_int> {
  match hour {
    1..=11 => Some(hour),
    0 | 12 => Some(12),
    13..=23 => Some(hour - 12),
    _ => None,
  }
}

fn append_hour_12(buffer: &mut OutputBuffer, hour: c_int) {
  let Some(normalized) = normalize_hour_12(hour) else {
    buffer.push_byte(b'?');

    return;
  };

  append_padded_decimal(buffer, normalized, 2);
}

fn append_hour_12_space_padded(buffer: &mut OutputBuffer, hour: c_int) {
  let Some(normalized) = normalize_hour_12(hour) else {
    buffer.push_byte(b'?');

    return;
  };

  buffer.push_bytes(format!("{normalized:2}").as_bytes());
}

fn append_minute(buffer: &mut OutputBuffer, minute: c_int) {
  append_padded_decimal_in_range(buffer, minute, 2, 0, 59);
}

fn append_second(buffer: &mut OutputBuffer, second: c_int) {
  append_padded_decimal_in_range(buffer, second, 2, 0, 60);
}

fn append_meridiem(buffer: &mut OutputBuffer, hour: c_int, lowercase: bool) {
  let token = match hour {
    0..=11 => {
      if lowercase {
        b"am".as_slice()
      } else {
        b"AM".as_slice()
      }
    }
    12..=23 => {
      if lowercase {
        b"pm".as_slice()
      } else {
        b"PM".as_slice()
      }
    }
    _ => b"?".as_slice(),
  };

  buffer.push_bytes(token);
}

fn append_posix_weekday(buffer: &mut OutputBuffer, wday: c_int) {
  match wday {
    0 => buffer.push_byte(b'7'),
    1..=6 => buffer.push_bytes(format!("{wday}").as_bytes()),
    _ => buffer.push_byte(b'?'),
  }
}

fn append_c_weekday(buffer: &mut OutputBuffer, wday: c_int) {
  match wday {
    0..=6 => buffer.push_bytes(format!("{wday}").as_bytes()),
    _ => buffer.push_byte(b'?'),
  }
}

fn append_julian_day(buffer: &mut OutputBuffer, tm_year: c_int, yday: c_int) {
  let full_year = full_year_from_tm_year(tm_year);

  if !is_valid_tm_yday_for_year(full_year, yday) {
    buffer.push_byte(b'?');

    return;
  }

  append_padded_decimal(buffer, yday + 1, 3);
}

fn append_week_number(
  buffer: &mut OutputBuffer,
  tm_year: c_int,
  yday: c_int,
  wday: c_int,
  monday_based: bool,
) {
  if !(0..=6).contains(&wday) {
    buffer.push_byte(b'?');

    return;
  }

  let full_year = full_year_from_tm_year(tm_year);

  if !is_valid_tm_yday_for_year(full_year, yday) {
    buffer.push_byte(b'?');

    return;
  }

  let week_start_offset = if monday_based {
    if wday == 0 { 6 } else { wday - 1 }
  } else {
    wday
  };
  let week_number = (yday + 7 - week_start_offset) / 7;

  append_padded_decimal(buffer, week_number, 2);
}

const fn iso_weeks_in_year(year: i128) -> c_int {
  let january_first_weekday = (days_from_civil(year, 1, 1) + 4).rem_euclid(7);

  if january_first_weekday == 4 || (january_first_weekday == 3 && is_leap_year(year)) {
    return 53;
  }

  52
}

fn iso_week_context(tm_year: c_int, yday: c_int, wday: c_int) -> Option<(i128, c_int)> {
  if !(0..=365).contains(&yday) || !(0..=6).contains(&wday) {
    return None;
  }

  let full_year = full_year_from_tm_year(tm_year);

  if !is_valid_tm_yday_for_year(full_year, yday) {
    return None;
  }

  let iso_weekday = if wday == 0 { 7 } else { wday };
  let mut iso_year = full_year;
  let mut week_number = (yday + 10 - iso_weekday) / 7;

  if week_number < 1 {
    iso_year -= 1;
    week_number = iso_weeks_in_year(iso_year);
  } else if week_number > iso_weeks_in_year(full_year) {
    iso_year += 1;
    week_number = 1;
  }

  Some((iso_year, week_number))
}

fn append_iso_week_year(
  buffer: &mut OutputBuffer,
  tm_year: c_int,
  yday: c_int,
  wday: c_int,
  two_digits: bool,
) {
  let Some((iso_year, _)) = iso_week_context(tm_year, yday, wday) else {
    buffer.push_byte(b'?');

    return;
  };

  if two_digits {
    let year_in_century = iso_year.rem_euclid(100);

    buffer.push_bytes(format!("{year_in_century:02}").as_bytes());

    return;
  }

  append_full_year(buffer, iso_year);
}

fn append_iso_week_number(buffer: &mut OutputBuffer, tm_year: c_int, yday: c_int, wday: c_int) {
  let Some((_, week_number)) = iso_week_context(tm_year, yday, wday) else {
    buffer.push_byte(b'?');

    return;
  };

  append_padded_decimal(buffer, week_number, 2);
}

fn append_c_locale_datetime(buffer: &mut OutputBuffer, time_parts: &tm) {
  buffer.push_bytes(weekday_name(time_parts.tm_wday, false));
  buffer.push_byte(b' ');
  buffer.push_bytes(month_name(time_parts.tm_mon, false));
  buffer.push_byte(b' ');
  append_day_of_month_space_padded(buffer, time_parts.tm_mday);
  buffer.push_byte(b' ');
  append_hour_24(buffer, time_parts.tm_hour);
  buffer.push_byte(b':');
  append_minute(buffer, time_parts.tm_min);
  buffer.push_byte(b':');
  append_second(buffer, time_parts.tm_sec);
  buffer.push_byte(b' ');
  append_year(buffer, time_parts.tm_year);
}

const fn normalize_alternative_modifier_token(modifier: u8, token: u8) -> Option<u8> {
  match modifier {
    b'E' => match token {
      b'c' | b'C' | b'x' | b'X' | b'y' | b'Y' | b'z' | b'u' | b'r' | b'R' | b'p' | b'P' | b'T'
      | b's' | b'n' | b't' => Some(token),
      _ => None,
    },
    b'O' => match token {
      b'd' | b'e' | b'H' | b'I' | b'm' | b'M' | b'S' | b'u' | b'U' | b'V' | b'w' | b'W' | b'y'
      | b'Y' | b'C' | b'G' | b'g' | b'j' | b'k' | b'l' | b'p' | b'P' | b'R' | b'T' | b'r'
      | b'z' | b's' | b'n' | b't' => Some(token),
      _ => None,
    },
    _ => None,
  }
}

const fn is_alternative_modifier(token: u8) -> bool {
  matches!(token, b'E' | b'O')
}

const fn append_verbatim_percent_token(buffer: &mut OutputBuffer, token: u8) {
  buffer.push_byte(b'%');
  buffer.push_byte(token);
}

const fn append_verbatim_alternative_modifier(buffer: &mut OutputBuffer, modifier: u8, token: u8) {
  append_verbatim_percent_token(buffer, modifier);
  buffer.push_byte(token);
}

fn append_token(buffer: &mut OutputBuffer, token: u8, time_parts: &tm) {
  match token {
    b'%' => buffer.push_byte(b'%'),
    b'c' => append_c_locale_datetime(buffer, time_parts),
    b'C' => append_century(buffer, time_parts.tm_year),
    b'y' => append_year_two_digits(buffer, time_parts.tm_year),
    b'Y' => append_year(buffer, time_parts.tm_year),
    b's' => append_epoch_seconds(buffer, time_parts),
    b'm' => append_month_number(buffer, time_parts.tm_mon),
    b'd' => append_day_of_month(buffer, time_parts.tm_mday),
    b'e' => append_day_of_month_space_padded(buffer, time_parts.tm_mday),
    b'H' => append_hour_24(buffer, time_parts.tm_hour),
    b'k' => append_hour_24_space_padded(buffer, time_parts.tm_hour),
    b'I' => append_hour_12(buffer, time_parts.tm_hour),
    b'l' => append_hour_12_space_padded(buffer, time_parts.tm_hour),
    b'p' => append_meridiem(buffer, time_parts.tm_hour, false),
    b'P' => append_meridiem(buffer, time_parts.tm_hour, true),
    b'Z' => append_timezone_abbreviation(buffer, time_parts.tm_zone),
    b'z' => append_numeric_utc_offset(buffer, time_parts.tm_gmtoff),
    b'M' => append_minute(buffer, time_parts.tm_min),
    b'S' => append_second(buffer, time_parts.tm_sec),
    b'D' | b'x' => {
      append_month_number(buffer, time_parts.tm_mon);
      buffer.push_byte(b'/');
      append_day_of_month(buffer, time_parts.tm_mday);
      buffer.push_byte(b'/');
      append_year_two_digits(buffer, time_parts.tm_year);
    }
    b'F' => {
      append_year(buffer, time_parts.tm_year);
      buffer.push_byte(b'-');
      append_month_number(buffer, time_parts.tm_mon);
      buffer.push_byte(b'-');
      append_day_of_month(buffer, time_parts.tm_mday);
    }
    b'T' | b'X' => {
      append_hour_24(buffer, time_parts.tm_hour);
      buffer.push_byte(b':');
      append_minute(buffer, time_parts.tm_min);
      buffer.push_byte(b':');
      append_second(buffer, time_parts.tm_sec);
    }
    b'R' => {
      append_hour_24(buffer, time_parts.tm_hour);
      buffer.push_byte(b':');
      append_minute(buffer, time_parts.tm_min);
    }
    b'v' => {
      append_day_of_month_space_padded(buffer, time_parts.tm_mday);
      buffer.push_byte(b'-');
      buffer.push_bytes(month_name(time_parts.tm_mon, false));
      buffer.push_byte(b'-');
      append_year(buffer, time_parts.tm_year);
    }
    b'r' => {
      append_hour_12(buffer, time_parts.tm_hour);
      buffer.push_byte(b':');
      append_minute(buffer, time_parts.tm_min);
      buffer.push_byte(b':');
      append_second(buffer, time_parts.tm_sec);
      buffer.push_byte(b' ');
      append_meridiem(buffer, time_parts.tm_hour, false);
    }
    b'a' => buffer.push_bytes(weekday_name(time_parts.tm_wday, false)),
    b'A' => buffer.push_bytes(weekday_name(time_parts.tm_wday, true)),
    b'h' | b'b' => buffer.push_bytes(month_name(time_parts.tm_mon, false)),
    b'B' => buffer.push_bytes(month_name(time_parts.tm_mon, true)),
    b'j' => append_julian_day(buffer, time_parts.tm_year, time_parts.tm_yday),
    b'U' => append_week_number(
      buffer,
      time_parts.tm_year,
      time_parts.tm_yday,
      time_parts.tm_wday,
      false,
    ),
    b'W' => append_week_number(
      buffer,
      time_parts.tm_year,
      time_parts.tm_yday,
      time_parts.tm_wday,
      true,
    ),
    b'G' => append_iso_week_year(
      buffer,
      time_parts.tm_year,
      time_parts.tm_yday,
      time_parts.tm_wday,
      false,
    ),
    b'g' => append_iso_week_year(
      buffer,
      time_parts.tm_year,
      time_parts.tm_yday,
      time_parts.tm_wday,
      true,
    ),
    b'V' => append_iso_week_number(
      buffer,
      time_parts.tm_year,
      time_parts.tm_yday,
      time_parts.tm_wday,
    ),
    b'u' => append_posix_weekday(buffer, time_parts.tm_wday),
    b'w' => append_c_weekday(buffer, time_parts.tm_wday),
    b'n' => buffer.push_byte(b'\n'),
    b't' => buffer.push_byte(b'\t'),
    _ => append_verbatim_percent_token(buffer, token),
  }
}

thread_local! {
  static GMTIME_STORAGE: UnsafeCell<tm> = const {
    UnsafeCell::new(unix_epoch_tm())
  };
}

/// C ABI entry point for `gmtime_r`.
///
/// Converts epoch seconds from `timer` into a UTC broken-down calendar value
/// written to `result`.
///
/// Returns:
/// - `result` on success (`errno` is not modified by this wrapper)
/// - null on failure (`errno` set to `EINVAL` for null pointers, `ERANGE` for
///   unrepresentable calendar years)
///
/// Failure-side output mutation contract:
/// - `ERANGE`: `result` is left unchanged.
/// - `EINVAL` with `timer == NULL` and non-null `result`: `result` is left unchanged.
/// - `EINVAL` with `result == NULL`: this wrapper returns before attempting to
///   read `timer`.
///
/// # Safety
/// - `timer` must point to a readable `time_t`.
/// - `result` must point to writable storage for one [`tm`] value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gmtime_r(timer: *const time_t, result: *mut tm) -> *mut tm {
  if timer.is_null() || result.is_null() {
    set_errno(EINVAL);

    return ptr::null_mut();
  }

  // SAFETY: Null was checked above; caller guarantees readable `timer`.
  let seconds = i128::from(unsafe { timer.read() });
  let Some(converted) = tm_from_unix_seconds(seconds) else {
    set_errno(ERANGE);

    return ptr::null_mut();
  };

  // SAFETY: `result` validity is guaranteed by the caller contract.
  unsafe {
    result.write(converted);
  }

  result
}

/// C ABI entry point for `gmtime`.
///
/// Converts epoch seconds from `timer` into UTC using per-thread static
/// storage and returns a pointer to that storage.
///
/// Returns:
/// - pointer to per-thread storage on success (`errno` is not modified by this
///   wrapper)
/// - null on failure (`errno` set to `EINVAL` or `ERANGE`)
///
/// Notes:
/// - The returned storage is the same nonreentrant per-thread object used by
///   [`localtime`], so calls to either function may overwrite the previous
///   result.
///
/// Failure-side storage mutation contract:
/// - `ERANGE`: the per-thread storage is left unchanged.
/// - `EINVAL` with `timer == NULL`: the per-thread storage is left unchanged.
/// - The no-clobber guarantee above applies even when the storage was last
///   written by [`localtime`].
///
/// # Safety
/// - `timer` must point to a readable [`time_t`] value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gmtime(timer: *const time_t) -> *mut tm {
  if timer.is_null() {
    set_errno(EINVAL);

    return ptr::null_mut();
  }

  GMTIME_STORAGE.with(|storage| {
    let result_ptr = storage.get();
    // SAFETY: `timer` and `result_ptr` satisfy `gmtime_r` pointer contract.
    let converted = unsafe { gmtime_r(timer, result_ptr) };

    if converted.is_null() {
      return ptr::null_mut();
    }

    result_ptr
  })
}

/// C ABI entry point for `localtime_r`.
///
/// Converts epoch seconds from `timer` into a broken-down local calendar value
/// written to `result`.
///
/// Current baseline behavior:
/// - This implementation uses UTC-only conversion and is currently equivalent
///   to [`gmtime_r`].
///
/// Returns:
/// - `result` on success (`errno` is not modified by this wrapper)
/// - null on failure (`errno` set to `EINVAL` for null pointers, `ERANGE` for
///   unrepresentable calendar years)
///
/// Failure-side output mutation contract:
/// - `ERANGE`: `result` is left unchanged.
/// - `EINVAL` with `timer == NULL` and non-null `result`: `result` is left unchanged.
/// - `EINVAL` with `result == NULL`: this wrapper returns before attempting to
///   read `timer`.
///
/// # Safety
/// - `timer` must point to a readable [`time_t`].
/// - `result` must point to writable storage for one [`tm`] value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn localtime_r(timer: *const time_t, result: *mut tm) -> *mut tm {
  // SAFETY: Pointer contracts are identical to `gmtime_r`.
  unsafe { gmtime_r(timer, result) }
}

/// C ABI entry point for `localtime`.
///
/// Converts epoch seconds from `timer` into local broken-down time using
/// per-thread static storage and returns a pointer to that storage.
///
/// Current baseline behavior:
/// - This implementation uses UTC-only conversion and is currently equivalent
///   to [`gmtime`] conversion semantics.
/// - The returned storage is the same nonreentrant per-thread object used by
///   [`gmtime`], so calls to either function may overwrite the previous result.
///
/// Returns:
/// - pointer to per-thread storage on success (`errno` is not modified by this
///   wrapper)
/// - null on failure (`errno` set to `EINVAL` or `ERANGE`)
///
/// Failure-side storage mutation contract:
/// - `ERANGE`: the per-thread storage is left unchanged.
/// - `EINVAL` with `timer == NULL`: the per-thread storage is left unchanged.
/// - The no-clobber guarantee above applies even when the storage was last
///   written by [`gmtime`].
///
/// # Safety
/// - `timer` must point to a readable [`time_t`] value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn localtime(timer: *const time_t) -> *mut tm {
  if timer.is_null() {
    set_errno(EINVAL);

    return ptr::null_mut();
  }

  GMTIME_STORAGE.with(|storage| {
    let result_ptr = storage.get();
    // SAFETY: `timer` and `result_ptr` satisfy `localtime_r` pointer contract.
    let converted = unsafe { localtime_r(timer, result_ptr) };

    if converted.is_null() {
      return ptr::null_mut();
    }

    result_ptr
  })
}

/// C ABI entry point for `timegm`.
///
/// Converts UTC broken-down calendar components in `time_parts` to Unix epoch
/// seconds. The input fields are normalized following C-style carry/borrow
/// behavior (for example, out-of-range hour or month values are folded into
/// neighboring fields).
///
/// On success this function updates `time_parts` with normalized UTC fields
/// (`tm_wday`, `tm_yday`, etc.) and returns the epoch seconds value.
///
/// Returns `-1` on error with `errno` set to:
/// - `EINVAL` for null input pointer
/// - `ERANGE` when the resulting timestamp cannot fit `time_t` or when the
///   normalized year does not fit `tm_year`
///
/// On `ERANGE`, this function does not modify `time_parts`.
///
/// Notes:
/// - A return value of `-1` can also be a valid successful timestamp
///   (`1969-12-31 23:59:59 UTC`). Callers that need to disambiguate success
///   from failure should inspect `errno` around the call.
/// - On success this wrapper does not modify `errno`.
///
/// # Safety
/// - `time_parts` must point to readable and writable storage for one [`tm`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn timegm(time_parts: *mut tm) -> time_t {
  if time_parts.is_null() {
    set_errno(EINVAL);

    return -1;
  }

  // SAFETY: Null was checked above; caller guarantees readable storage.
  let input = unsafe { time_parts.read() };
  let seconds = normalized_tm_to_unix_seconds(&input);
  let Ok(output_seconds) = c_long::try_from(seconds) else {
    set_errno(ERANGE);

    return -1;
  };
  let Some(normalized) = tm_from_unix_seconds(seconds) else {
    set_errno(ERANGE);

    return -1;
  };

  // SAFETY: `time_parts` validity is guaranteed by the caller contract.
  unsafe {
    time_parts.write(normalized);
  }

  output_seconds
}

/// C ABI entry point for `mktime`.
///
/// Converts broken-down local calendar components in `time_parts` to Unix
/// epoch seconds and normalizes calendar fields in-place.
///
/// Current baseline behavior:
/// - This implementation uses UTC-only conversion.
/// - `tm_isdst > 0` is interpreted as a daylight-saving hint and subtracts
///   one hour from the interpreted wall-clock input before normalization.
/// - `tm_isdst <= 0` is interpreted as standard/unknown local time under the
///   UTC baseline (no extra offset adjustment).
///
/// On success this function updates `time_parts` with normalized fields
/// (`tm_wday`, `tm_yday`, etc.) and returns epoch seconds.
///
/// Returns `-1` on error with `errno` set to:
/// - `EINVAL` for null input pointer
/// - `ERANGE` when the resulting timestamp cannot fit `time_t` or when the
///   normalized year does not fit `tm_year`
///
/// On `ERANGE`, this function does not modify `time_parts`.
///
/// Notes:
/// - A return value of `-1` can also be a valid successful timestamp
///   (`1969-12-31 23:59:59 UTC`). Callers that need to disambiguate success
///   from failure should inspect `errno` around the call.
/// - On success this wrapper does not modify `errno`.
/// - On success this wrapper normalizes the output to UTC-baseline fields and
///   sets `tm_isdst` to `0`.
///
/// # Safety
/// - `time_parts` must point to readable and writable storage for one [`tm`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mktime(time_parts: *mut tm) -> time_t {
  if time_parts.is_null() {
    set_errno(EINVAL);

    return -1;
  }

  // SAFETY: Null was checked above; caller guarantees readable storage.
  let input = unsafe { time_parts.read() };
  let seconds = normalized_tm_to_unix_seconds_for_mktime(&input);
  let Ok(output_seconds) = c_long::try_from(seconds) else {
    set_errno(ERANGE);

    return -1;
  };
  let Some(normalized) = tm_from_unix_seconds(seconds) else {
    set_errno(ERANGE);

    return -1;
  };

  // SAFETY: `time_parts` validity is guaranteed by the caller contract.
  unsafe {
    time_parts.write(normalized);
  }

  output_seconds
}

/// C ABI entry point for `clock_gettime`.
///
/// Writes the current time for `clock_id` to `tp`.
///
/// Return contract:
/// - `0` on success
/// - `-1` on failure, with `errno` set
///
/// Error contract:
/// - `EFAULT` when `tp` is null
/// - kernel-provided errno values (for example `EINVAL` for unsupported
///   `clock_id`) for syscall failures
/// - when the `clock_gettime` syscall itself is unavailable (`ENOSYS`) and
///   `clock_id == CLOCK_REALTIME`, this implementation falls back to
///   `gettimeofday`; in that path, `gettimeofday` kernel errors are reported
///   through `errno`
/// - in the ENOSYS fallback path, non-null invalid `tp` pointers are still
///   validated by the kernel and reported as `EFAULT`
/// - `EINVAL` when ENOSYS fallback receives a `tv_usec` value outside
///   `[0, 1_000_000)` and therefore cannot represent a valid
///   `timespec.tv_nsec`
///
/// Null output pointers are rejected before issuing the syscall. This means
/// `clock_gettime(invalid_clock_id, NULL)` reports `EFAULT`.
fn syscall_clock_gettime(clock_id: clockid_t, tp: *mut timespec) -> c_long {
  unsafe {
    syscall2(
      SYS_CLOCK_GETTIME,
      c_long::from(clock_id),
      ptr_to_sys_arg(tp.cast_const()),
    )
  }
}

fn syscall_gettimeofday_for_clock(tv: *mut timeval) -> c_long {
  unsafe { syscall2(SYS_GETTIMEOFDAY, ptr_to_sys_arg(tv.cast_const()), 0) }
}

fn clock_gettime_with_syscalls(
  clock_id: clockid_t,
  tp: *mut timespec,
  clock_gettime_syscall: fn(clockid_t, *mut timespec) -> c_long,
  gettimeofday_syscall: fn(*mut timeval) -> c_long,
) -> c_int {
  if tp.is_null() {
    set_errno(EFAULT);

    return -1;
  }

  let raw = clock_gettime_syscall(clock_id, tp);

  if let Err(errno) = decode_syscall_status(raw) {
    if errno == ENOSYS && clock_id == CLOCK_REALTIME {
      let mut fallback_time = timeval {
        tv_sec: 0,
        tv_usec: 0,
      };
      let fallback_raw = gettimeofday_syscall(&raw mut fallback_time);

      if let Err(fallback_errno) = decode_syscall_status(fallback_raw) {
        set_errno(fallback_errno);

        return -1;
      }

      let tv_usec = fallback_time.tv_usec;

      if !(0..1_000_000).contains(&tv_usec) {
        set_errno(EINVAL);

        return -1;
      }

      // Preserve kernel-side pointer validation (`EFAULT`) before userspace write.
      let probe_raw = gettimeofday_syscall(tp.cast::<timeval>());

      if let Err(probe_errno) = decode_syscall_status(probe_raw) {
        set_errno(probe_errno);

        return -1;
      }

      unsafe {
        (*tp).tv_sec = fallback_time.tv_sec;
        (*tp).tv_nsec = tv_usec * 1_000;
      }

      return 0;
    }

    set_errno(errno);

    return -1;
  }

  0
}

#[unsafe(no_mangle)]
pub extern "C" fn clock_gettime(clock_id: clockid_t, tp: *mut timespec) -> c_int {
  clock_gettime_with_syscalls(
    clock_id,
    tp,
    syscall_clock_gettime,
    syscall_gettimeofday_for_clock,
  )
}

/// C ABI entry point for `gettimeofday`.
///
/// Writes wall-clock time to `tv` and optional timezone info to `tz`.
///
/// Return contract:
/// - `0` on success (and `errno` is not modified by this wrapper)
/// - `-1` on failure, with thread-local `errno` overwritten from the kernel
///   error code
///
/// Error contract:
/// - `EFAULT` when `tv` or `tz` is a non-null invalid pointer
/// - kernel-provided errno values for other syscall failures
///
/// Notes:
/// - When both pointers are null, this implementation returns success without
///   invoking the syscall and therefore preserves `errno`.
/// - `tz` is a legacy interface on Linux; callers that do not explicitly need
///   timezone output should pass null.
/// - When `tv` is null and `tz` is non-null, this implementation still invokes
///   the kernel so that valid `tz` pointers can be serviced and invalid ones
///   can produce `EFAULT`.
///
/// # Safety
/// - `tv` and `tz` are optional, but when non-null each must point to writable
///   storage of its respective type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gettimeofday(tv: *mut timeval, tz: *mut timezone) -> c_int {
  if tv.is_null() && tz.is_null() {
    return 0;
  }

  let raw = unsafe {
    syscall2(
      SYS_GETTIMEOFDAY,
      ptr_to_sys_arg(tv.cast_const()),
      ptr_to_sys_arg(tz.cast_const()),
    )
  };

  if let Err(errno) = decode_syscall_status(raw) {
    set_errno(errno);

    return -1;
  }

  0
}

/// C ABI entry point for `strftime`.
///
/// Formats `time_ptr` into `s` according to `format` using the `C`/`POSIX`
/// locale baseline.
///
/// Supported conversion specifiers:
/// - minimal: `%% %Y %m %d %H %M %S`
/// - extended: `%c %F %T %R %r %v %a %A %b %B %j %U %W %G %g %V %w %C %y %D %u %n %t %x %X %h %e %I %k %l %p %P %s %Z %z`
///
/// Notes:
/// - `%m %d %H %k %M %S` expect `tm_mon/tm_mday/tm_hour/tm_min/tm_sec` in their
///   C baseline ranges (`0..=11`, `1..=31`, `0..=23`, `0..=59`, `0..=60`);
///   out-of-range values are emitted as `?`.
/// - `%I/%l/%p/%P/%r` expect `tm_hour` in `0..=23`; out-of-range values are
///   emitted as `?` for the hour and meridiem fragments.
/// - POSIX alternative modifiers `%E` and `%O` are accepted for their `C`
///   locale aliases and are normalized to the corresponding unmodified token:
///   `%Ec/%EC/%Ex/%EX/%Ey/%EY/%Ez/%Eu/%Ep/%EP/%Er/%ER/%ET/%Es/%En/%Et` and
///   `%OC/%Od/%Oe/%OH/%OI/%Oj/%Ok/%Ol/%Om/%OM/%OS/%Ou/%OU/%OV/%Ow/%OW/%Oy/%OY/%OG/%Og/%Op/%OP/%OR/%OT/%Or/%Os/%On/%Ot/%Oz`.
///   Unsupported combinations are emitted verbatim (for example `%Oq`).
/// - `%e` expects `tm_mday` in `1..=31`; out-of-range values are emitted as
///   `?`.
/// - `%j` expects a year-consistent `tm_yday` (`0..=364` for non-leap years,
///   `0..=365` for leap years); out-of-range values are emitted as `?`.
/// - `%U/%W` expect `tm_wday` in `0..=6` and a year-consistent `tm_yday`
///   (`0..=364` for non-leap years, `0..=365` for leap years); invalid fields
///   are emitted as `?`.
/// - `%G/%g/%V` use ISO-8601 week rules and expect `tm_wday` in `0..=6` plus
///   a `tm_yday`/`tm_year` pair that is valid for the target year;
///   out-of-range fields are emitted as `?`.
/// - `%v` is formatted as `%e-%b-%Y` and therefore inherits `%e/%b/%Y`
///   fallback behavior for malformed fields.
/// - `%s` returns Unix epoch seconds by normalizing `tm` fields under the same
///   UTC baseline used by [`timegm`] and [`mktime`].
/// - `%Z` copies the NUL-terminated timezone abbreviation referenced by
///   `tm_zone`; null `tm_zone` emits an empty field.
/// - `%z` expects `tm_gmtoff` to be a whole-minute UTC offset in
///   `-23:59..=+23:59`; out-of-range values and second-level offsets are
///   emitted as `?`.
/// - `%c` is formatted as `%a %b %e %T %Y`.
///
/// Return contract:
/// - success: number of bytes written, excluding the trailing NUL
/// - truncation (`max == 0` or output does not fit): `0`
///
/// # Errors
/// Returns `0` and sets `errno` to `EINVAL` when:
/// - `format` is null
/// - `time_ptr` is null
/// - `s` is null while `max > 0`
///
/// # Safety
/// - `s` must point to writable storage for at least `max` bytes when
///   `max > 0`.
/// - `format` must point to a readable NUL-terminated byte string.
/// - `time_ptr` must point to a readable `tm` value.
/// - when `%Z` is present and `tm_zone` is non-null, `tm_zone` must point to a
///   readable NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strftime(
  s: *mut c_char,
  max: size_t,
  format: *const c_char,
  time_ptr: *const tm,
) -> size_t {
  let max_len = len_from_size_t(max);

  if format.is_null() || time_ptr.is_null() || (max_len != 0 && s.is_null()) {
    set_errno(EINVAL);

    return 0;
  }

  let mut buffer = OutputBuffer::new(s, max_len);
  // SAFETY: Caller guarantees `time_ptr` points to a readable `tm`.
  let time_parts = unsafe { &*time_ptr };
  let mut index = 0usize;

  loop {
    // SAFETY: Caller guarantees `format` points to a readable NUL-terminated string.
    let current = unsafe { format.add(index).read() };

    if current == 0 {
      break;
    }

    let byte = current.to_ne_bytes()[0];

    if byte != b'%' {
      buffer.push_byte(byte);
      index += 1;

      continue;
    }

    index += 1;
    // SAFETY: Caller guarantees `format` points to a readable NUL-terminated string.
    let token = unsafe { format.add(index).read() };

    if token == 0 {
      buffer.push_byte(b'%');

      break;
    }

    let token_byte = token.to_ne_bytes()[0];

    if is_alternative_modifier(token_byte) {
      // SAFETY: Caller guarantees `format` points to a readable NUL-terminated string.
      let modified_token = unsafe { format.add(index + 1).read() };

      if modified_token == 0 {
        append_verbatim_percent_token(&mut buffer, token_byte);

        break;
      }

      let modified_byte = modified_token.to_ne_bytes()[0];

      if let Some(base_token) = normalize_alternative_modifier_token(token_byte, modified_byte) {
        append_token(&mut buffer, base_token, time_parts);
      } else {
        append_verbatim_alternative_modifier(&mut buffer, token_byte, modified_byte);
      }

      index += 2;

      continue;
    }

    append_token(&mut buffer, token_byte, time_parts);
    index += 1;
  }

  buffer.finish()
}

#[cfg(test)]
mod tests {
  use super::{
    CLOCK_REALTIME, OutputBuffer, append_timezone_abbreviation, c_int, c_long,
    clock_gettime_with_syscalls, clockid_t, decode_syscall_status, timespec, timeval,
  };
  use crate::abi::errno::{EFAULT, EINVAL, ENOSYS};
  use crate::errno::__errno_location;
  use core::sync::atomic::{AtomicUsize, Ordering};

  fn read_errno() -> c_int {
    unsafe { *__errno_location() }
  }

  fn write_errno(value: c_int) {
    unsafe {
      *__errno_location() = value;
    }
  }

  #[test]
  fn decode_syscall_status_non_negative_is_ok() {
    assert_eq!(decode_syscall_status(0), Ok(()));
    assert_eq!(decode_syscall_status(7), Ok(()));
  }

  #[test]
  fn decode_syscall_status_negative_errno_is_err() {
    assert_eq!(decode_syscall_status(-1), Err(1));
    assert_eq!(decode_syscall_status(-22), Err(22));
  }

  #[test]
  fn decode_syscall_status_handles_most_negative_without_overflow() {
    assert_eq!(decode_syscall_status(c_long::MIN), Err(c_int::MAX));
  }

  #[test]
  fn append_timezone_abbreviation_null_zone_writes_empty_output() {
    let mut raw = [0xAA_u8; 4];
    let mut buffer = OutputBuffer::new(raw.as_mut_ptr().cast(), raw.len());

    append_timezone_abbreviation(&mut buffer, core::ptr::null());

    let written = usize::try_from(buffer.finish())
      .unwrap_or_else(|_| unreachable!("size_t must fit usize on this target"));

    assert_eq!(written, 0);
    assert_eq!(raw[0], 0);
  }

  #[test]
  fn clock_gettime_realtime_falls_back_to_gettimeofday_on_enosys() {
    static GETTIMEOFDAY_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn clock_gettime_enosys(_: clockid_t, _: *mut timespec) -> c_long {
      -c_long::from(ENOSYS)
    }

    fn gettimeofday_success(tv: *mut timeval) -> c_long {
      GETTIMEOFDAY_CALLS.fetch_add(1, Ordering::Relaxed);

      unsafe {
        (*tv).tv_sec = 123;
        (*tv).tv_usec = 456_789;
      }

      0
    }

    GETTIMEOFDAY_CALLS.store(0, Ordering::Relaxed);

    let mut ts = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };
    let result = clock_gettime_with_syscalls(
      CLOCK_REALTIME,
      &raw mut ts,
      clock_gettime_enosys,
      gettimeofday_success,
    );

    assert_eq!(result, 0);
    assert_eq!(ts.tv_sec, 123);
    assert_eq!(ts.tv_nsec, 456_789_000);
    assert_eq!(GETTIMEOFDAY_CALLS.load(Ordering::Relaxed), 2);
  }

  #[test]
  fn clock_gettime_realtime_fallback_reports_probe_efault_and_keeps_output_unchanged() {
    static GETTIMEOFDAY_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn clock_gettime_enosys(_: clockid_t, _: *mut timespec) -> c_long {
      -c_long::from(ENOSYS)
    }

    fn gettimeofday_probe_efault(tv: *mut timeval) -> c_long {
      let call_index = GETTIMEOFDAY_CALLS.fetch_add(1, Ordering::Relaxed);

      if call_index == 0 {
        unsafe {
          (*tv).tv_sec = 123;
          (*tv).tv_usec = 456_789;
        }

        return 0;
      }

      -c_long::from(EFAULT)
    }

    GETTIMEOFDAY_CALLS.store(0, Ordering::Relaxed);

    let mut ts = timespec {
      tv_sec: 99,
      tv_nsec: 88,
    };

    write_errno(77);

    let result = clock_gettime_with_syscalls(
      CLOCK_REALTIME,
      &raw mut ts,
      clock_gettime_enosys,
      gettimeofday_probe_efault,
    );

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EFAULT);
    assert_eq!(ts.tv_sec, 99);
    assert_eq!(ts.tv_nsec, 88);
    assert_eq!(GETTIMEOFDAY_CALLS.load(Ordering::Relaxed), 2);
  }

  #[test]
  fn clock_gettime_realtime_fallback_propagates_gettimeofday_error_without_probe() {
    static GETTIMEOFDAY_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn clock_gettime_enosys(_: clockid_t, _: *mut timespec) -> c_long {
      -c_long::from(ENOSYS)
    }

    fn gettimeofday_efault(_: *mut timeval) -> c_long {
      GETTIMEOFDAY_CALLS.fetch_add(1, Ordering::Relaxed);
      -c_long::from(EFAULT)
    }

    GETTIMEOFDAY_CALLS.store(0, Ordering::Relaxed);

    let mut ts = timespec {
      tv_sec: 77,
      tv_nsec: 66,
    };

    write_errno(42);

    let result = clock_gettime_with_syscalls(
      CLOCK_REALTIME,
      &raw mut ts,
      clock_gettime_enosys,
      gettimeofday_efault,
    );

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EFAULT);
    assert_eq!(ts.tv_sec, 77);
    assert_eq!(ts.tv_nsec, 66);
    assert_eq!(GETTIMEOFDAY_CALLS.load(Ordering::Relaxed), 1);
  }

  #[test]
  fn clock_gettime_realtime_fallback_rejects_out_of_range_usec() {
    static GETTIMEOFDAY_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn clock_gettime_enosys(_: clockid_t, _: *mut timespec) -> c_long {
      -c_long::from(ENOSYS)
    }

    fn gettimeofday_invalid_usec(tv: *mut timeval) -> c_long {
      GETTIMEOFDAY_CALLS.fetch_add(1, Ordering::Relaxed);

      unsafe {
        (*tv).tv_sec = 123;
        (*tv).tv_usec = 1_000_000;
      }

      0
    }

    GETTIMEOFDAY_CALLS.store(0, Ordering::Relaxed);

    let mut ts = timespec {
      tv_sec: 11,
      tv_nsec: 22,
    };

    write_errno(91);

    let result = clock_gettime_with_syscalls(
      CLOCK_REALTIME,
      &raw mut ts,
      clock_gettime_enosys,
      gettimeofday_invalid_usec,
    );

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts.tv_sec, 11);
    assert_eq!(ts.tv_nsec, 22);
    assert_eq!(GETTIMEOFDAY_CALLS.load(Ordering::Relaxed), 1);
  }

  #[test]
  fn clock_gettime_realtime_fallback_rejects_negative_usec() {
    static GETTIMEOFDAY_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn clock_gettime_enosys(_: clockid_t, _: *mut timespec) -> c_long {
      -c_long::from(ENOSYS)
    }

    fn gettimeofday_negative_usec(tv: *mut timeval) -> c_long {
      GETTIMEOFDAY_CALLS.fetch_add(1, Ordering::Relaxed);

      unsafe {
        (*tv).tv_sec = 123;
        (*tv).tv_usec = -1;
      }

      0
    }

    GETTIMEOFDAY_CALLS.store(0, Ordering::Relaxed);

    let mut ts = timespec {
      tv_sec: 33,
      tv_nsec: 44,
    };

    write_errno(19);

    let result = clock_gettime_with_syscalls(
      CLOCK_REALTIME,
      &raw mut ts,
      clock_gettime_enosys,
      gettimeofday_negative_usec,
    );

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts.tv_sec, 33);
    assert_eq!(ts.tv_nsec, 44);
    assert_eq!(GETTIMEOFDAY_CALLS.load(Ordering::Relaxed), 1);
  }
}
