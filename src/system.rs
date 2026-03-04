//! Minimal system-information APIs.
//!
//! This module provides a Linux `x86_64` subset for issue I038:
//! - `uname`
//! - `gethostname`
//! - `getpagesize`
//! - `sysinfo`
//! - `sysconf` (minimal selector subset)

use crate::abi::errno::{EFAULT, EINVAL, ENAMETOOLONG};
use crate::abi::types::{c_char, c_int, c_long, c_uint, c_ulong, c_ushort, size_t};
use crate::errno::set_errno;
use crate::resource::{RLIM_INFINITY, RLIMIT_NOFILE, RLimit};
use crate::syscall::{syscall1, syscall3, syscall4};
use core::{ptr, slice};

const SYS_UNAME: c_long = 63;
const SYS_SYSINFO: c_long = 99;
const SYS_SCHED_GETAFFINITY: c_long = 204;
const SYS_PRLIMIT64: c_long = 302;
const SCHED_AFFINITY_BYTES: usize = 128;
const MAX_SCHED_AFFINITY_BYTES: usize = 16 * 1024;
const CPU_POSSIBLE_PATH: &str = "/sys/devices/system/cpu/possible";
const CPU_PRESENT_PATH: &str = "/sys/devices/system/cpu/present";
/// `_SC_CLK_TCK` selector.
pub const _SC_CLK_TCK: c_int = 2;
/// `_SC_OPEN_MAX` selector.
pub const _SC_OPEN_MAX: c_int = 4;
/// `_SC_PAGESIZE` selector.
pub const _SC_PAGESIZE: c_int = 30;
/// `_SC_PAGE_SIZE` selector alias.
pub const _SC_PAGE_SIZE: c_int = _SC_PAGESIZE;
/// `_SC_NPROCESSORS_CONF` selector.
pub const _SC_NPROCESSORS_CONF: c_int = 83;
/// `_SC_NPROCESSORS_ONLN` selector.
pub const _SC_NPROCESSORS_ONLN: c_int = 84;
const SYS_PAGESIZE: c_int = 4096;
const CLK_TCK_VALUE: c_long = 100;
const OPEN_MAX_FALLBACK_VALUE: c_long = 1024;
const UTSNAME_FIELD_LEN: usize = 65;
const UTSNAME_MAX_PAYLOAD_LEN: usize = UTSNAME_FIELD_LEN - 1;

/// C-compatible `uname` payload.
///
/// ABI contract on `x86_64-unknown-linux-gnu`:
/// - field order and each array length must match Linux `struct utsname`
/// - every field is a fixed-size byte array of length `65`
/// - kernel writes NUL-terminated strings into these arrays on success
///
/// Callers should treat each field as a C string with a maximum payload length
/// of `64` bytes plus trailing NUL.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct UtsName {
  /// Operating system name.
  pub sysname: [c_char; UTSNAME_FIELD_LEN],
  /// Network node hostname.
  pub nodename: [c_char; UTSNAME_FIELD_LEN],
  /// OS release level.
  pub release: [c_char; UTSNAME_FIELD_LEN],
  /// OS version level.
  pub version: [c_char; UTSNAME_FIELD_LEN],
  /// Hardware identifier.
  pub machine: [c_char; UTSNAME_FIELD_LEN],
  /// Domain name.
  pub domainname: [c_char; UTSNAME_FIELD_LEN],
}

/// C-compatible `sysinfo` payload for Linux `x86_64`.
///
/// ABI contract:
/// - layout follows Linux `struct sysinfo`
/// - memory counters (`totalram`, `freeram`, etc.) are expressed in units of
///   `mem_unit` bytes
/// - `loads` values use Linux fixed-point scaling (`1 << 16`)
///
/// This type is intentionally `#[repr(C)]` and must remain layout-compatible
/// with the kernel ABI.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SysInfo {
  /// Seconds since boot.
  pub uptime: c_long,
  /// 1, 5, 15 minute load averages.
  pub loads: [c_ulong; 3],
  /// Total usable main memory.
  pub totalram: c_ulong,
  /// Available memory.
  pub freeram: c_ulong,
  /// Shared memory amount.
  pub sharedram: c_ulong,
  /// Buffer memory amount.
  pub bufferram: c_ulong,
  /// Total swap size.
  pub totalswap: c_ulong,
  /// Free swap size.
  pub freeswap: c_ulong,
  /// Current process count.
  pub procs: c_ushort,
  /// ABI padding.
  pub pad: c_ushort,
  /// Total high memory.
  pub totalhigh: c_ulong,
  /// Free high memory.
  pub freehigh: c_ulong,
  /// Memory unit size in bytes.
  pub mem_unit: c_uint,
}

fn ptr_arg<T>(ptr: *const T) -> c_long {
  c_long::try_from(ptr.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit c_long on x86_64"))
}

fn mut_ptr_arg<T>(ptr: *mut T) -> c_long {
  ptr_arg(ptr.cast_const())
}

fn errno_from_raw(raw: c_long) -> c_int {
  c_int::try_from(-raw).unwrap_or(c_int::MAX)
}

fn nodename_payload_len(nodename: &[c_char; UTSNAME_FIELD_LEN]) -> usize {
  nodename
    .iter()
    .take(UTSNAME_MAX_PAYLOAD_LEN)
    .position(|&ch| ch == 0)
    .unwrap_or(UTSNAME_MAX_PAYLOAD_LEN)
}

fn write_nodename(
  output: &mut [c_char],
  nodename: &[c_char; UTSNAME_FIELD_LEN],
) -> Result<(), c_int> {
  let name_len = nodename_payload_len(nodename);
  let required = name_len + 1;

  if output.len() < required {
    return Err(ENAMETOOLONG);
  }

  output[..name_len].copy_from_slice(&nodename[..name_len]);
  output[name_len] = 0;

  Ok(())
}

fn online_processor_count() -> c_long {
  online_processor_count_with_retries(query_online_processor_count)
}

fn online_processor_count_with_retries<F>(mut query: F) -> c_long
where
  F: FnMut(usize) -> Result<usize, c_int>,
{
  let mut mask_len = SCHED_AFFINITY_BYTES;

  loop {
    match query(mask_len) {
      Ok(count) => {
        let count = count.max(1);

        return c_long::try_from(count).unwrap_or(c_long::MAX);
      }
      Err(EINVAL) => {
        let Some(next_len) = mask_len.checked_mul(2) else {
          return 1;
        };

        if next_len > MAX_SCHED_AFFINITY_BYTES {
          return 1;
        }

        mask_len = next_len;
      }
      Err(_) => return 1,
    }
  }
}

fn query_online_processor_count(mask_len: usize) -> Result<usize, c_int> {
  let mut affinity_mask = vec![0_u8; mask_len];
  let mask_len = c_long::try_from(affinity_mask.len())
    .unwrap_or_else(|_| unreachable!("affinity mask length must fit c_long on x86_64"));
  let mask_ptr = mut_ptr_arg(affinity_mask.as_mut_ptr());
  let raw = unsafe { syscall3(SYS_SCHED_GETAFFINITY, 0, mask_len, mask_ptr) };

  if raw < 0 {
    return Err(errno_from_raw(raw));
  }

  let readable_bytes = usize::try_from(raw).unwrap_or(affinity_mask.len());
  let scan_len = readable_bytes.min(affinity_mask.len());
  let count = affinity_mask[..scan_len]
    .iter()
    .map(|byte| usize::try_from(byte.count_ones()).unwrap_or(0))
    .sum::<usize>();

  Ok(count.max(1))
}

fn parse_cpu_index(token: &str) -> Option<usize> {
  if token.is_empty() || !token.as_bytes().iter().all(u8::is_ascii_digit) {
    return None;
  }

  token.parse::<usize>().ok()
}

fn parse_cpu_range_list(input: &str) -> Option<usize> {
  let mut total = 0usize;
  let mut next_min_cpu = 0usize;
  let mut parsed_any = false;
  let mut tokens = input.trim().split(',').peekable();

  while let Some(raw_token) = tokens.next() {
    let token = raw_token.trim();

    if token.is_empty() {
      return None;
    }

    let (start, end) = if let Some((start_text, end_text)) = token.split_once('-') {
      let start = parse_cpu_index(start_text)?;
      let end = parse_cpu_index(end_text)?;

      if end < start {
        return None;
      }

      (start, end)
    } else {
      let value = parse_cpu_index(token)?;

      (value, value)
    };

    if parsed_any && start < next_min_cpu {
      return None;
    }

    let token_count = end.checked_sub(start)?.checked_add(1)?;

    parsed_any = true;
    total = total.checked_add(token_count)?;

    if tokens.peek().is_some() {
      next_min_cpu = end.checked_add(1)?;
    }
  }

  Some(total)
}

fn configured_processor_count() -> c_long {
  configured_processor_count_with(
    || std::fs::read_to_string(CPU_POSSIBLE_PATH).ok(),
    || std::fs::read_to_string(CPU_PRESENT_PATH).ok(),
    online_processor_count,
  )
}

fn parse_configured_processor_count(contents: &str) -> Option<c_long> {
  parse_cpu_range_list(contents)
    .filter(|count| *count > 0)
    .and_then(|count| c_long::try_from(count).ok())
}

fn configured_processor_count_with<P, R, F>(
  mut read_possible: P,
  mut read_present: R,
  mut fallback_online: F,
) -> c_long
where
  P: FnMut() -> Option<String>,
  R: FnMut() -> Option<String>,
  F: FnMut() -> c_long,
{
  let configured = read_possible()
    .and_then(|contents| parse_configured_processor_count(&contents))
    .or_else(|| read_present().and_then(|contents| parse_configured_processor_count(&contents)))
    .unwrap_or_else(&mut fallback_online);

  configured.max(1)
}

fn open_max_from_soft_limit(rlim_cur: c_ulong) -> c_long {
  if rlim_cur == RLIM_INFINITY {
    return OPEN_MAX_FALLBACK_VALUE;
  }

  c_long::try_from(rlim_cur).unwrap_or(c_long::MAX)
}

fn open_max_value() -> c_long {
  open_max_value_with(|limits| {
    // SAFETY: syscall number and argument registers follow Linux x86_64 ABI and
    // `limits` points to writable storage for one `struct rlimit`.
    unsafe {
      syscall4(
        SYS_PRLIMIT64,
        0,
        c_long::from(RLIMIT_NOFILE),
        0,
        mut_ptr_arg(ptr::addr_of_mut!(*limits)),
      )
    }
  })
}

fn open_max_value_with<F>(mut query_limits: F) -> c_long
where
  F: FnMut(&mut RLimit) -> c_long,
{
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let raw = query_limits(&mut limits);

  if raw != 0 {
    return OPEN_MAX_FALLBACK_VALUE;
  }

  open_max_from_soft_limit(limits.rlim_cur)
}

/// C ABI entry point for `uname`.
///
/// Writes system identity fields into `buf`.
///
/// # Safety
/// - `buf` must point to writable storage for one [`UtsName`].
///
/// # Errors
/// - Returns `-1` and sets `errno = EFAULT` when `buf` is null.
/// - Returns `-1` and sets `errno` to the underlying syscall error for kernel
///   failures.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uname(buf: *mut UtsName) -> c_int {
  if buf.is_null() {
    set_errno(EFAULT);

    return -1;
  }

  let raw = unsafe { syscall1(SYS_UNAME, mut_ptr_arg(buf)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  0
}

/// C ABI entry point for `gethostname`.
///
/// Copies the current nodename into `name`, including trailing NUL.
/// On success, exactly `nodename` bytes plus the terminating NUL are written.
/// If the kernel-provided nodename is not NUL-terminated within the first
/// Linux payload `64` bytes, this implementation copies exactly `64` bytes and
/// appends one terminating NUL when `len` is sufficient.
///
/// # Safety
/// - `name` must be writable for `len` bytes.
///
/// # Errors
/// - Returns `-1` and sets `errno = ENAMETOOLONG` when `len == 0` or when
///   `len` is smaller than the required `nodename + NUL` size.
/// - Returns `-1` and sets `errno = EFAULT` when `name` is null and `len > 0`.
/// - Propagates `uname` syscall failures.
///
/// On these failure paths, this implementation leaves the destination buffer
/// unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gethostname(name: *mut c_char, len: size_t) -> c_int {
  let output_len = usize::try_from(len)
    .unwrap_or_else(|_| unreachable!("size_t must fit into usize on x86_64 Linux"));

  if output_len == 0 {
    set_errno(ENAMETOOLONG);

    return -1;
  }

  if name.is_null() {
    set_errno(EFAULT);

    return -1;
  }

  let mut uts = UtsName {
    sysname: [0; UTSNAME_FIELD_LEN],
    nodename: [0; UTSNAME_FIELD_LEN],
    release: [0; UTSNAME_FIELD_LEN],
    version: [0; UTSNAME_FIELD_LEN],
    machine: [0; UTSNAME_FIELD_LEN],
    domainname: [0; UTSNAME_FIELD_LEN],
  };

  // SAFETY: `uts` is valid writable storage.
  if unsafe { uname(ptr::addr_of_mut!(uts)) } != 0 {
    return -1;
  }

  // SAFETY: null is rejected above and caller guarantees `name` writable for
  // `output_len` bytes.
  let output = unsafe { slice::from_raw_parts_mut(name, output_len) };

  if let Err(errno) = write_nodename(output, &uts.nodename) {
    set_errno(errno);

    return -1;
  }

  0
}

/// C ABI entry point for `getpagesize`.
///
/// Returns the system memory page size used by this target profile.
///
/// On `x86_64-unknown-linux-gnu` this implementation intentionally returns the
/// stable Linux baseline page size (`4096`) and does not mutate `errno`.
#[must_use]
#[unsafe(no_mangle)]
pub const extern "C" fn getpagesize() -> c_int {
  SYS_PAGESIZE
}

/// C ABI entry point for `sysinfo`.
///
/// # Safety
/// - `info` must point to writable storage for one [`SysInfo`].
///
/// # Errors
/// - Returns `-1` and sets `errno = EFAULT` when `info` is null.
/// - Returns `-1` and sets `errno` to the underlying syscall error for kernel
///   failures.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sysinfo(info: *mut SysInfo) -> c_int {
  if info.is_null() {
    set_errno(EFAULT);

    return -1;
  }

  let raw = unsafe { syscall1(SYS_SYSINFO, mut_ptr_arg(info)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  0
}

/// C ABI entry point for `sysconf` (minimal selector set).
///
/// Supported selectors:
/// - `_SC_CLK_TCK` (`2`)
/// - `_SC_OPEN_MAX` (`4`): soft `RLIMIT_NOFILE` when available, fallback
///   `1024` when limit lookup fails or is unbounded. Finite soft limits above
///   the representable `c_long` range are clamped to `c_long::MAX`. Any
///   nonzero `prlimit64` status is treated as lookup failure.
/// - `_SC_PAGESIZE` / `_SC_PAGE_SIZE` (`30`)
/// - `_SC_NPROCESSORS_CONF` (`83`): configured CPUs (from Linux
///   `/sys/devices/system/cpu/possible`, then `/sys/devices/system/cpu/present`,
///   or online fallback when sysfs data is unavailable or malformed)
/// - `_SC_NPROCESSORS_ONLN` (`84`): online CPUs visible to the current process.
///   Uses `sched_getaffinity` and retries with larger affinity masks when the
///   kernel reports `EINVAL` for undersized buffers.
///
/// On success, returns a nonnegative value and leaves `errno` unchanged.
/// `_SC_OPEN_MAX` may legitimately return `0` when the current soft
/// `RLIMIT_NOFILE` is zero.
///
/// # Errors
/// Unsupported selectors return `-1` with `errno = EINVAL`.
#[unsafe(no_mangle)]
pub extern "C" fn sysconf(name: c_int) -> c_long {
  match name {
    _SC_CLK_TCK => CLK_TCK_VALUE,
    _SC_OPEN_MAX => open_max_value(),
    _SC_PAGESIZE => c_long::from(SYS_PAGESIZE),
    _SC_NPROCESSORS_CONF => configured_processor_count(),
    _SC_NPROCESSORS_ONLN => online_processor_count(),
    _ => {
      set_errno(EINVAL);
      -1
    }
  }
}

#[cfg(test)]
mod tests {
  use crate::abi::errno::EFAULT;
  use crate::abi::types::{c_char, c_long, c_ulong};

  use super::{
    MAX_SCHED_AFFINITY_BYTES, OPEN_MAX_FALLBACK_VALUE, RLIM_INFINITY, UTSNAME_FIELD_LEN,
    UTSNAME_MAX_PAYLOAD_LEN, configured_processor_count_with, nodename_payload_len,
    online_processor_count_with_retries, open_max_from_soft_limit, open_max_value_with,
    parse_cpu_range_list, write_nodename,
  };

  #[test]
  fn parse_cpu_range_list_accepts_single_values_and_ranges() {
    assert_eq!(parse_cpu_range_list("0"), Some(1));
    assert_eq!(parse_cpu_range_list("0-3"), Some(4));
    assert_eq!(parse_cpu_range_list("0-3,8,10-11\n"), Some(7));
    assert_eq!(parse_cpu_range_list("0-2,3-4"), Some(5));
    assert_eq!(parse_cpu_range_list(" 0-1 , 3 , 5-6 "), Some(5));
  }

  #[test]
  fn parse_cpu_range_list_accepts_single_maximum_cpu_index() {
    assert_eq!(parse_cpu_range_list(&usize::MAX.to_string()), Some(1));
  }

  #[test]
  fn parse_cpu_range_list_accepts_near_maximum_contiguous_ranges() {
    let near_max = usize::MAX - 1;
    let input = format!("{near_max}-{near_max},{}", usize::MAX);

    assert_eq!(parse_cpu_range_list(&input), Some(2));
  }

  #[test]
  fn parse_cpu_range_list_accepts_maximum_terminal_range() {
    let near_max = usize::MAX - 1;
    let input = format!("{near_max}-{}", usize::MAX);

    assert_eq!(parse_cpu_range_list(&input), Some(2));
  }

  #[test]
  fn parse_cpu_range_list_rejects_invalid_tokens() {
    assert_eq!(parse_cpu_range_list(""), None);
    assert_eq!(parse_cpu_range_list("0,,1"), None);
    assert_eq!(parse_cpu_range_list("0-1,"), None);
    assert_eq!(parse_cpu_range_list("0-1, "), None);
    assert_eq!(parse_cpu_range_list("3-1"), None);
    assert_eq!(parse_cpu_range_list("0,0"), None);
    assert_eq!(parse_cpu_range_list("0-2,2-3"), None);
    assert_eq!(parse_cpu_range_list("2,1"), None);
    assert_eq!(parse_cpu_range_list("+1"), None);
    assert_eq!(parse_cpu_range_list("0,+1"), None);
    assert_eq!(parse_cpu_range_list("+0-1"), None);
    assert_eq!(parse_cpu_range_list("0-+1"), None);
    assert_eq!(parse_cpu_range_list("cpu0"), None);

    let max_then_extra = format!("{},0", usize::MAX);

    assert_eq!(parse_cpu_range_list(&max_then_extra), None);
  }

  #[test]
  fn nodename_payload_len_stops_at_first_nul() {
    let mut nodename = [c_char::from_ne_bytes([b'x']); UTSNAME_FIELD_LEN];

    nodename[4] = 0;

    assert_eq!(nodename_payload_len(&nodename), 4);
  }

  #[test]
  fn nodename_payload_len_caps_at_linux_payload_max_without_nul() {
    let nodename = [c_char::from_ne_bytes([b'x']); UTSNAME_FIELD_LEN];

    assert_eq!(nodename_payload_len(&nodename), UTSNAME_MAX_PAYLOAD_LEN);
  }

  #[test]
  fn nodename_payload_len_ignores_terminator_slot() {
    let mut nodename = [c_char::from_ne_bytes([b'x']); UTSNAME_FIELD_LEN];

    nodename[UTSNAME_MAX_PAYLOAD_LEN] = 0;

    assert_eq!(nodename_payload_len(&nodename), UTSNAME_MAX_PAYLOAD_LEN);
  }

  #[test]
  fn write_nodename_rejects_short_buffer_without_mutation() {
    let mut nodename = [c_char::from_ne_bytes([b'a']); UTSNAME_FIELD_LEN];

    nodename[3] = 0;

    let mut output = [c_char::from_ne_bytes([b'z']); 3];

    assert_eq!(
      write_nodename(&mut output, &nodename),
      Err(crate::abi::errno::ENAMETOOLONG)
    );
    assert!(
      output
        .iter()
        .all(|&byte| byte == c_char::from_ne_bytes([b'z']))
    );
  }

  #[test]
  fn write_nodename_writes_payload_cap_and_explicit_nul() {
    let nodename = [c_char::from_ne_bytes([b'x']); UTSNAME_FIELD_LEN];
    let mut output = [c_char::from_ne_bytes([b'?']); UTSNAME_FIELD_LEN + 2];

    assert_eq!(write_nodename(&mut output, &nodename), Ok(()));
    assert!(
      output[..UTSNAME_MAX_PAYLOAD_LEN]
        .iter()
        .all(|&byte| byte == c_char::from_ne_bytes([b'x']))
    );
    assert_eq!(output[UTSNAME_MAX_PAYLOAD_LEN], 0);
    assert!(
      output[UTSNAME_FIELD_LEN..]
        .iter()
        .all(|&byte| byte == c_char::from_ne_bytes([b'?']))
    );
  }

  #[test]
  fn write_nodename_handles_empty_payload_and_preserves_tail() {
    let mut nodename = [c_char::from_ne_bytes([b'x']); UTSNAME_FIELD_LEN];

    nodename[0] = 0;

    let mut output = [c_char::from_ne_bytes([b'!']); 4];

    assert_eq!(write_nodename(&mut output, &nodename), Ok(()));
    assert_eq!(output[0], 0);
    assert_eq!(output[1], c_char::from_ne_bytes([b'!']));
    assert_eq!(output[2], c_char::from_ne_bytes([b'!']));
    assert_eq!(output[3], c_char::from_ne_bytes([b'!']));
  }

  #[test]
  fn online_processor_count_retries_after_einval_and_uses_successful_count() {
    let mut calls = Vec::new();
    let count = online_processor_count_with_retries(|mask_len| {
      calls.push(mask_len);

      if calls.len() == 1 {
        return Err(crate::abi::errno::EINVAL);
      }

      Ok(7)
    });

    assert_eq!(count, 7);
    assert_eq!(calls, vec![128, 256]);
  }

  #[test]
  fn online_processor_count_normalizes_zero_to_one() {
    let count = online_processor_count_with_retries(|_| Ok(0));

    assert_eq!(count, 1);
  }

  #[test]
  fn online_processor_count_returns_one_for_non_einval_failure() {
    let count = online_processor_count_with_retries(|_| Err(EFAULT));

    assert_eq!(count, 1);
  }

  #[test]
  fn online_processor_count_retry_growth_respects_maximum_mask() {
    let mut calls = Vec::new();
    let count = online_processor_count_with_retries(|mask_len| {
      calls.push(mask_len);
      Err(crate::abi::errno::EINVAL)
    });

    assert_eq!(count, 1);
    assert_eq!(calls, vec![128, 256, 512, 1024, 2048, 4096, 8192, 16384]);
    assert_eq!(calls.last().copied(), Some(MAX_SCHED_AFFINITY_BYTES));
  }

  #[test]
  fn online_processor_count_returns_one_when_retries_exceed_limit() {
    let count = online_processor_count_with_retries(|_| Err(crate::abi::errno::EINVAL));

    assert_eq!(count, 1);
  }

  #[test]
  fn configured_processor_count_uses_possible_file_when_parseable() {
    let count = configured_processor_count_with(
      || Some(String::from("0-2,5")),
      || Some(String::from("0-7")),
      || 1,
    );

    assert_eq!(count, 4);
  }

  #[test]
  fn configured_processor_count_uses_possible_file_when_present_is_missing() {
    let count = configured_processor_count_with(|| Some(String::from("0-3")), || None, || 7);

    assert_eq!(count, 4);
  }

  #[test]
  fn configured_processor_count_uses_present_file_when_possible_file_missing() {
    let count = configured_processor_count_with(|| None, || Some(String::from("0-3")), || 7);

    assert_eq!(count, 4);
  }

  #[test]
  fn configured_processor_count_uses_present_file_when_possible_file_is_invalid() {
    let count = configured_processor_count_with(
      || Some(String::from("0,0")),
      || Some(String::from("0-1")),
      || 9,
    );

    assert_eq!(count, 2);
  }

  #[test]
  fn configured_processor_count_uses_present_file_when_possible_is_whitespace_only() {
    let count = configured_processor_count_with(
      || Some(String::from("   ")),
      || Some(String::from("0-1")),
      || 9,
    );

    assert_eq!(count, 2);
  }

  #[test]
  fn configured_processor_count_uses_present_file_when_possible_file_has_signed_tokens() {
    let count = configured_processor_count_with(
      || Some(String::from("+0-1")),
      || Some(String::from("0-2")),
      || 9,
    );

    assert_eq!(count, 3);
  }

  #[test]
  fn configured_processor_count_falls_back_when_possible_has_signed_tokens_and_present_missing() {
    let count = configured_processor_count_with(|| Some(String::from("+0-1")), || None, || 8);

    assert_eq!(count, 8);
  }

  #[test]
  fn configured_processor_count_keeps_possible_value_when_present_has_signed_tokens() {
    let count = configured_processor_count_with(
      || Some(String::from("0-1")),
      || Some(String::from("+0-7")),
      || 9,
    );

    assert_eq!(count, 2);
  }

  #[test]
  fn configured_processor_count_keeps_possible_value_when_present_is_descending() {
    let count = configured_processor_count_with(
      || Some(String::from("0-2")),
      || Some(String::from("2,1")),
      || 9,
    );

    assert_eq!(count, 3);
  }

  #[test]
  fn configured_processor_count_keeps_possible_value_when_present_count_does_not_fit_c_long() {
    let count = configured_processor_count_with(
      || Some(String::from("0-1")),
      || Some(String::from("0-9223372036854775808")),
      || 9,
    );

    assert_eq!(count, 2);
  }

  #[test]
  fn configured_processor_count_falls_back_when_both_sysfs_sources_are_unusable() {
    let count = configured_processor_count_with(
      || Some(String::from("0,0")),
      || Some(String::from("2,1")),
      || 7,
    );

    assert_eq!(count, 7);
  }

  #[test]
  fn configured_processor_count_falls_back_when_sysfs_sources_use_signed_tokens() {
    let count = configured_processor_count_with(
      || Some(String::from("0,+1")),
      || Some(String::from("0-+1")),
      || 6,
    );

    assert_eq!(count, 6);
  }

  #[test]
  fn configured_processor_count_uses_present_file_when_possible_count_does_not_fit_c_long() {
    let count = configured_processor_count_with(
      || Some(String::from("0-9223372036854775808")),
      || Some(String::from("0-2")),
      || 5,
    );

    assert_eq!(count, 3);
  }

  #[test]
  fn configured_processor_count_falls_back_when_counts_do_not_fit_c_long() {
    let count = configured_processor_count_with(
      || Some(String::from("0-9223372036854775808")),
      || Some(String::from("0-9223372036854775808")),
      || 9,
    );

    assert_eq!(count, 9);
  }

  #[test]
  fn configured_processor_count_normalizes_non_positive_fallback_values() {
    let zero_fallback = configured_processor_count_with(|| None, || None, || 0);
    let negative_fallback = configured_processor_count_with(|| None, || None, || -7);

    assert_eq!(zero_fallback, 1);
    assert_eq!(negative_fallback, 1);
  }

  #[test]
  fn parse_cpu_range_list_rejects_overflowing_total_count() {
    let overflowing = format!("0-{},0", usize::MAX);

    assert_eq!(parse_cpu_range_list(&overflowing), None);
  }

  #[test]
  fn open_max_from_soft_limit_uses_fallback_for_infinity() {
    assert_eq!(
      open_max_from_soft_limit(c_ulong::MAX),
      OPEN_MAX_FALLBACK_VALUE
    );
  }

  #[test]
  fn open_max_from_soft_limit_clamps_non_representable_finite_limit() {
    assert_eq!(open_max_from_soft_limit(c_ulong::MAX - 1), c_long::MAX);
  }

  #[test]
  fn open_max_from_soft_limit_returns_value_when_representable() {
    assert_eq!(open_max_from_soft_limit(c_ulong::from(4_096_u32)), 4_096);
  }

  #[test]
  fn open_max_from_soft_limit_accepts_near_maximum_representable_value() {
    let near_max = c_ulong::try_from(c_long::MAX - 1)
      .unwrap_or_else(|_| unreachable!("c_long::MAX - 1 must fit into c_ulong on this target"));

    assert_eq!(open_max_from_soft_limit(near_max), c_long::MAX - 1);
  }

  #[test]
  fn open_max_from_soft_limit_accepts_largest_representable_value() {
    let max_representable = c_ulong::try_from(c_long::MAX)
      .unwrap_or_else(|_| unreachable!("c_long::MAX must fit into c_ulong on this target"));

    assert_eq!(open_max_from_soft_limit(max_representable), c_long::MAX);
  }

  #[test]
  fn open_max_from_soft_limit_clamps_first_non_representable_value() {
    let first_non_representable = c_ulong::try_from(c_long::MAX)
      .unwrap_or_else(|_| unreachable!("c_long::MAX must fit into c_ulong on this target"))
      .checked_add(1)
      .unwrap_or_else(|| unreachable!("c_long::MAX + 1 must fit into c_ulong on this target"));

    assert_eq!(
      open_max_from_soft_limit(first_non_representable),
      c_long::MAX
    );
  }

  #[test]
  fn open_max_from_soft_limit_preserves_minimum_positive_limit() {
    assert_eq!(open_max_from_soft_limit(c_ulong::from(1_u8)), 1);
  }

  #[test]
  fn open_max_from_soft_limit_preserves_zero_soft_limit() {
    assert_eq!(open_max_from_soft_limit(c_ulong::from(0_u8)), 0);
  }

  #[test]
  fn open_max_value_with_uses_fallback_when_limit_query_fails() {
    assert_eq!(open_max_value_with(|_| -1), OPEN_MAX_FALLBACK_VALUE);
  }

  #[test]
  fn open_max_value_with_handles_most_negative_query_status() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(4_096_u32);
      c_long::MIN
    });

    assert_eq!(value, OPEN_MAX_FALLBACK_VALUE);
  }

  #[test]
  fn open_max_value_with_uses_fallback_for_small_negative_query_status() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(4_096_u32);
      -2
    });

    assert_eq!(value, OPEN_MAX_FALLBACK_VALUE);
  }

  #[test]
  fn open_max_value_with_ignores_soft_limit_when_limit_query_fails() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(64_u8);
      -1
    });

    assert_eq!(value, OPEN_MAX_FALLBACK_VALUE);
  }

  #[test]
  fn open_max_value_with_uses_fallback_for_infinite_soft_limit() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::MAX;
      0
    });

    assert_eq!(value, OPEN_MAX_FALLBACK_VALUE);
  }

  #[test]
  fn open_max_value_with_clamps_non_representable_soft_limit() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::MAX - 1;
      0
    });

    assert_eq!(value, c_long::MAX);
  }

  #[test]
  fn open_max_value_with_uses_soft_limit_when_query_succeeds() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(8_192_u32);
      0
    });

    assert_eq!(value, 8_192);
  }

  #[test]
  fn open_max_value_with_preserves_minimum_positive_soft_limit() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(1_u8);
      0
    });

    assert_eq!(value, 1);
  }

  #[test]
  fn open_max_value_with_accepts_largest_representable_soft_limit() {
    let max_representable = c_ulong::try_from(c_long::MAX)
      .unwrap_or_else(|_| unreachable!("c_long::MAX must fit into c_ulong on this target"));
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = max_representable;
      0
    });

    assert_eq!(value, c_long::MAX);
  }

  #[test]
  fn open_max_value_with_uses_soft_limit_even_when_hard_limit_differs() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(512_u16);
      limits.rlim_max = c_ulong::from(8_192_u32);
      0
    });

    assert_eq!(value, 512);
  }

  #[test]
  fn open_max_value_with_uses_soft_limit_when_hard_limit_is_lower() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(4_096_u32);
      limits.rlim_max = c_ulong::from(64_u8);
      0
    });

    assert_eq!(value, 4_096);
  }

  #[test]
  fn open_max_value_with_uses_soft_limit_when_hard_limit_is_infinite() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(256_u16);
      limits.rlim_max = RLIM_INFINITY;
      0
    });

    assert_eq!(value, 256);
  }

  #[test]
  fn open_max_value_with_uses_fallback_for_nonzero_query_status() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(2_048_u32);
      1
    });

    assert_eq!(value, OPEN_MAX_FALLBACK_VALUE);
  }

  #[test]
  fn open_max_value_with_uses_fallback_for_nonzero_status_even_with_minimum_soft_limit() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(1_u8);
      1
    });

    assert_eq!(value, OPEN_MAX_FALLBACK_VALUE);
  }

  #[test]
  fn open_max_value_with_uses_fallback_for_largest_positive_query_status() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(2_048_u32);
      c_long::MAX
    });

    assert_eq!(value, OPEN_MAX_FALLBACK_VALUE);
  }

  #[test]
  fn open_max_value_with_preserves_zero_soft_limit() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(0_u8);
      0
    });

    assert_eq!(value, 0);
  }

  #[test]
  fn open_max_value_with_preserves_zero_soft_limit_even_with_infinite_hard_limit() {
    let value = open_max_value_with(|limits| {
      limits.rlim_cur = c_ulong::from(0_u8);
      limits.rlim_max = RLIM_INFINITY;
      0
    });

    assert_eq!(value, 0);
  }
}
