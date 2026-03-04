#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::{c_char, c_void};
use rlibc::abi::errno::{EFAULT, EINVAL, ENAMETOOLONG};
use rlibc::abi::types::{c_int, c_long, size_t};
use rlibc::resource::{RLIM_INFINITY, RLIMIT_NOFILE, RLimit, getrlimit, setrlimit};
use rlibc::system::{
  _SC_CLK_TCK, _SC_NPROCESSORS_CONF, _SC_NPROCESSORS_ONLN, _SC_OPEN_MAX, _SC_PAGE_SIZE,
  _SC_PAGESIZE, gethostname, getpagesize, sysconf,
};

const EXPECTED_SC_CLK_TCK: c_int = 2;
const EXPECTED_SC_OPEN_MAX: c_int = 4;
const EXPECTED_SC_PAGESIZE: c_int = 30;
const EXPECTED_SC_PAGE_SIZE: c_int = EXPECTED_SC_PAGESIZE;
const EXPECTED_SC_NPROCESSORS_CONF: c_int = 83;
const EXPECTED_SC_NPROCESSORS_ONLN: c_int = 84;
const EXPECTED_CLK_TCK_VALUE: c_long = 100;
const UNSUPPORTED_NAME: c_int = 9_999;
const ERRNO_SENTINEL: c_int = 777;
const CPU_POSSIBLE_PATH: &str = "/sys/devices/system/cpu/possible";
const CPU_PRESENT_PATH: &str = "/sys/devices/system/cpu/present";
const AFFINITY_MASK_BYTES: usize = 128;

unsafe extern "C" {
  fn __errno_location() -> *mut c_int;
  fn sched_getaffinity(pid: c_int, cpusetsize: usize, mask: *mut c_void) -> c_int;
}

struct RLimitRestoreGuard {
  resource: c_int,
  original: RLimit,
}

impl Drop for RLimitRestoreGuard {
  fn drop(&mut self) {
    // SAFETY: `original` came from a successful `getrlimit` call for `resource`.
    let _ = unsafe { setrlimit(self.resource, &raw const self.original) };
  }
}

fn errno_ptr() -> *mut c_int {
  // SAFETY: `__errno_location` returns a thread-local writable errno pointer.
  let pointer = unsafe { __errno_location() };

  assert!(!pointer.is_null(), "__errno_location returned null");

  pointer
}

fn set_errno(value: c_int) {
  let pointer = errno_ptr();

  // SAFETY: `errno_ptr` guarantees a valid writable pointer.
  unsafe { pointer.write(value) };
}

fn read_errno() -> c_int {
  let pointer = errno_ptr();

  // SAFETY: `errno_ptr` guarantees a valid readable pointer.
  unsafe { pointer.read() }
}

fn query(name: c_int) -> c_long {
  sysconf(name)
}

fn assert_open_max_query_succeeded(open_max: c_long) {
  assert!(
    open_max >= 0,
    "_SC_OPEN_MAX must not return -1 for a supported selector",
  );
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

fn current_affinity_cpu_count() -> Option<c_long> {
  let mut mask = [0_u8; AFFINITY_MASK_BYTES];
  // SAFETY: `mask` points to writable memory of `mask.len()` bytes.
  let result = unsafe { sched_getaffinity(0, mask.len(), mask.as_mut_ptr().cast::<c_void>()) };

  if result != 0 {
    return None;
  }

  let count = mask
    .iter()
    .map(|byte| usize::try_from(byte.count_ones()).unwrap_or(0))
    .sum::<usize>();
  let count = count.max(1);

  c_long::try_from(count).ok()
}

fn parsed_cpu_count_from_sysfs(path: &str) -> Option<c_long> {
  let parsed_count = std::fs::read_to_string(path)
    .ok()
    .and_then(|contents| parse_cpu_range_list(&contents))?;

  c_long::try_from(parsed_count).ok()
}

#[test]
fn sysconf_selector_constants_match_linux_x86_64_values() {
  assert_eq!(_SC_CLK_TCK, EXPECTED_SC_CLK_TCK);
  assert_eq!(_SC_OPEN_MAX, EXPECTED_SC_OPEN_MAX);
  assert_eq!(_SC_PAGESIZE, EXPECTED_SC_PAGESIZE);
  assert_eq!(_SC_PAGE_SIZE, EXPECTED_SC_PAGE_SIZE);
  assert_eq!(_SC_NPROCESSORS_CONF, EXPECTED_SC_NPROCESSORS_CONF);
  assert_eq!(_SC_NPROCESSORS_ONLN, EXPECTED_SC_NPROCESSORS_ONLN);
}

#[test]
fn sysconf_pagesize_and_page_size_are_consistent() {
  set_errno(ERRNO_SENTINEL);

  let pagesize_value = query(_SC_PAGESIZE);
  let page_size_alias_value = query(_SC_PAGE_SIZE);

  assert!(pagesize_value > 0, "pagesize must be positive");
  assert_eq!(pagesize_value, page_size_alias_value);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful sysconf must not clobber errno",
  );
}

#[test]
fn sysconf_pagesize_matches_getpagesize() {
  set_errno(ERRNO_SENTINEL);

  let sysconf_pagesize = query(_SC_PAGESIZE);
  let libc_pagesize = c_long::from(getpagesize());

  assert!(libc_pagesize > 0, "getpagesize must be positive");
  assert_eq!(sysconf_pagesize, libc_pagesize);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful sysconf must not clobber errno",
  );
}

#[test]
fn sysconf_page_size_alias_matches_getpagesize() {
  set_errno(ERRNO_SENTINEL);

  let alias_pagesize = query(_SC_PAGE_SIZE);
  let libc_pagesize = c_long::from(getpagesize());

  assert!(libc_pagesize > 0, "getpagesize must be positive");
  assert_eq!(alias_pagesize, libc_pagesize);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful sysconf alias query must not clobber errno",
  );
}

#[test]
fn sysconf_pagesize_alias_preserves_enametoolong_from_gethostname_zero_length_failure() {
  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let primary = query(_SC_PAGESIZE);
  let alias = query(_SC_PAGE_SIZE);

  assert!(primary > 0, "_SC_PAGESIZE must be positive");
  assert_eq!(primary, alias);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful pagesize selector queries must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_pagesize_alias_preserves_enametoolong_from_gethostname_failure() {
  let mut short_buffer = [0 as c_char; 1];

  set_errno(0);

  // SAFETY: `short_buffer` is valid writable memory and `len` matches it.
  let gethostname_result =
    unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let primary = query(_SC_PAGESIZE);
  let alias = query(_SC_PAGE_SIZE);

  assert!(primary > 0, "_SC_PAGESIZE must be positive");
  assert_eq!(primary, alias);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful pagesize selector queries must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_pagesize_alias_preserves_efault_from_gethostname_null_failure() {
  set_errno(0);

  // SAFETY: null `name` with nonzero `len` must fail with `EFAULT`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let primary = query(_SC_PAGESIZE);
  let alias = query(_SC_PAGE_SIZE);

  assert!(primary > 0, "_SC_PAGESIZE must be positive");
  assert_eq!(primary, alias);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful pagesize selector queries must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_pagesize_alias_success_does_not_clear_errno_after_prior_failure() {
  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let primary = query(_SC_PAGESIZE);
  let alias = query(_SC_PAGE_SIZE);

  assert!(primary > 0, "_SC_PAGESIZE must be positive");
  assert_eq!(primary, alias);
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful pagesize selector queries must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_clk_tck_is_positive_and_open_max_is_non_negative() {
  set_errno(ERRNO_SENTINEL);

  let clk_tck = query(_SC_CLK_TCK);
  let open_max = query(_SC_OPEN_MAX);

  assert!(clk_tck > 0, "_SC_CLK_TCK must be positive");
  assert_open_max_query_succeeded(open_max);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sysconf_clk_tck_matches_i038_contract_constant() {
  set_errno(ERRNO_SENTINEL);

  let clk_tck = query(_SC_CLK_TCK);

  assert_eq!(clk_tck, EXPECTED_CLK_TCK_VALUE);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful _SC_CLK_TCK query must not clobber errno",
  );
}

#[test]
fn sysconf_clk_tck_success_does_not_clear_errno_after_prior_failure() {
  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let clk_tck = query(_SC_CLK_TCK);

  assert!(clk_tck > 0, "_SC_CLK_TCK must be positive");
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful _SC_CLK_TCK query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_clk_tck_preserves_efault_from_gethostname_null_failure() {
  set_errno(0);

  // SAFETY: null `name` with nonzero `len` must fail with `EFAULT`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let clk_tck = query(_SC_CLK_TCK);

  assert!(clk_tck > 0, "_SC_CLK_TCK must be positive");
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful _SC_CLK_TCK query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_clk_tck_preserves_enametoolong_from_gethostname_failure() {
  let mut short_buffer = [0 as c_char; 1];

  set_errno(0);

  // SAFETY: `short_buffer` is valid writable memory and `len` matches it.
  let gethostname_result =
    unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let clk_tck = query(_SC_CLK_TCK);

  assert!(clk_tck > 0, "_SC_CLK_TCK must be positive");
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_CLK_TCK query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_clk_tck_preserves_enametoolong_from_gethostname_zero_length_failure() {
  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let clk_tck = query(_SC_CLK_TCK);

  assert!(clk_tck > 0, "_SC_CLK_TCK must be positive");
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_CLK_TCK query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_open_max_matches_rlimit_nofile_or_clamps_non_representable_value() {
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `limits` points to writable storage for one `RLimit`.
  let status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut limits) };

  assert_eq!(status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  set_errno(ERRNO_SENTINEL);

  let open_max = query(_SC_OPEN_MAX);

  if limits.rlim_cur == RLIM_INFINITY {
    assert!(open_max > 0, "_SC_OPEN_MAX fallback must remain positive");
  } else if let Ok(expected) = c_long::try_from(limits.rlim_cur) {
    assert_eq!(open_max, expected);
  } else {
    assert_eq!(
      open_max,
      c_long::MAX,
      "_SC_OPEN_MAX must clamp finite non-representable RLIMIT_NOFILE to c_long::MAX",
    );
  }

  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful _SC_OPEN_MAX query must not clobber errno",
  );
}

#[test]
fn sysconf_open_max_returns_zero_when_rlimit_nofile_soft_limit_is_zero() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed"
  );

  set_errno(ERRNO_SENTINEL);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 0);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful _SC_OPEN_MAX query must not clobber errno",
  );
}

#[test]
fn sysconf_open_max_preserves_enametoolong_from_gethostname_failure() {
  let mut short_buffer = [0 as c_char; 1];

  set_errno(0);

  // SAFETY: `short_buffer` is valid writable memory and `len` matches it.
  let gethostname_result =
    unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let open_max = query(_SC_OPEN_MAX);

  assert_open_max_query_succeeded(open_max);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_OPEN_MAX query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_open_max_preserves_enametoolong_from_gethostname_zero_length_failure() {
  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let open_max = query(_SC_OPEN_MAX);

  assert_open_max_query_succeeded(open_max);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_OPEN_MAX query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_open_max_preserves_efault_from_gethostname_null_failure() {
  set_errno(0);

  // SAFETY: null name pointer with nonzero len is the tested contract path.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let open_max = query(_SC_OPEN_MAX);

  assert_open_max_query_succeeded(open_max);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful _SC_OPEN_MAX query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_open_max_zero_soft_limit_success_preserves_errno_after_prior_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed",
  );

  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 0);
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful _SC_OPEN_MAX query must preserve errno from prior failure even when value is zero",
  );
}

#[test]
fn sysconf_open_max_one_soft_limit_repeated_success_preserves_errno_after_prior_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");
  assert!(
    original.rlim_max >= 1,
    "setrlimit(RLIMIT_NOFILE, soft=1) requires hard limit >= 1",
  );

  let temporary = RLimit {
    rlim_cur: 1,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=1) must succeed",
  );

  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let first_open_max = query(_SC_OPEN_MAX);
  let first_errno = read_errno();
  let second_open_max = query(_SC_OPEN_MAX);
  let second_errno = read_errno();

  assert_eq!(first_open_max, 1);
  assert_eq!(second_open_max, 1);
  assert_eq!(
    first_errno, EINVAL,
    "first successful _SC_OPEN_MAX query must preserve prior errno",
  );
  assert_eq!(
    second_errno, EINVAL,
    "second successful _SC_OPEN_MAX query must preserve prior errno",
  );
}

#[test]
fn sysconf_open_max_zero_soft_limit_success_does_not_clobber_errno_sentinel() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed",
  );

  set_errno(ERRNO_SENTINEL);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 0);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful _SC_OPEN_MAX query with soft limit zero must leave errno unchanged",
  );
}

#[test]
fn sysconf_open_max_zero_soft_limit_repeated_success_keeps_value_and_errno() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed",
  );

  set_errno(ERRNO_SENTINEL);

  let first_open_max = query(_SC_OPEN_MAX);
  let first_errno = read_errno();
  let second_open_max = query(_SC_OPEN_MAX);
  let second_errno = read_errno();

  assert_eq!(first_open_max, 0);
  assert_eq!(second_open_max, 0);
  assert_eq!(
    first_errno, ERRNO_SENTINEL,
    "first successful _SC_OPEN_MAX query must leave errno unchanged",
  );
  assert_eq!(
    second_errno, ERRNO_SENTINEL,
    "second successful _SC_OPEN_MAX query must also leave errno unchanged",
  );
}

#[test]
fn sysconf_open_max_zero_soft_limit_repeated_success_preserves_errno_after_prior_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed",
  );

  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let first_open_max = query(_SC_OPEN_MAX);
  let first_errno = read_errno();
  let second_open_max = query(_SC_OPEN_MAX);
  let second_errno = read_errno();

  assert_eq!(first_open_max, 0);
  assert_eq!(second_open_max, 0);
  assert_eq!(
    first_errno, EINVAL,
    "first successful _SC_OPEN_MAX query must preserve errno from prior failure",
  );
  assert_eq!(
    second_errno, EINVAL,
    "second successful _SC_OPEN_MAX query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_open_max_zero_soft_limit_repeated_success_preserves_enametoolong_from_gethostname_zero_length_failure()
 {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed",
  );

  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let first_open_max = query(_SC_OPEN_MAX);
  let first_errno = read_errno();
  let second_open_max = query(_SC_OPEN_MAX);
  let second_errno = read_errno();

  assert_eq!(first_open_max, 0);
  assert_eq!(second_open_max, 0);
  assert_eq!(
    first_errno, ENAMETOOLONG,
    "first successful _SC_OPEN_MAX query must preserve ENAMETOOLONG",
  );
  assert_eq!(
    second_errno, ENAMETOOLONG,
    "second successful _SC_OPEN_MAX query must preserve ENAMETOOLONG",
  );
}

#[test]
fn sysconf_open_max_zero_soft_limit_preserves_enametoolong_from_gethostname_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed",
  );

  let mut short_buffer = [0 as c_char; 1];

  set_errno(0);

  // SAFETY: `short_buffer` is valid writable memory and `len` matches it.
  let gethostname_result =
    unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 0);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_OPEN_MAX query must preserve errno from prior gethostname failure even when value is zero",
  );
}

#[test]
fn sysconf_open_max_zero_soft_limit_preserves_enametoolong_from_gethostname_zero_length_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed",
  );

  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 0);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_OPEN_MAX query must preserve ENAMETOOLONG even when soft limit is zero",
  );
}

#[test]
fn sysconf_open_max_zero_soft_limit_preserves_efault_from_gethostname_null_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");

  let temporary = RLimit {
    rlim_cur: 0,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=0) must succeed",
  );

  set_errno(0);

  // SAFETY: null `name` with nonzero `len` must fail with `EFAULT`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful _SC_OPEN_MAX query must preserve EFAULT even when soft limit is zero",
  );
}

#[test]
fn sysconf_open_max_one_soft_limit_success_preserves_errno_after_prior_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");
  assert!(
    original.rlim_max >= 1,
    "setrlimit(RLIMIT_NOFILE, soft=1) requires hard limit >= 1",
  );

  let temporary = RLimit {
    rlim_cur: 1,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=1) must succeed",
  );

  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 1);
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful _SC_OPEN_MAX query must preserve errno from prior failure even when value is one",
  );
}

#[test]
fn sysconf_open_max_one_soft_limit_success_does_not_clobber_errno_sentinel() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");
  assert!(
    original.rlim_max >= 1,
    "setrlimit(RLIMIT_NOFILE, soft=1) requires hard limit >= 1",
  );

  let temporary = RLimit {
    rlim_cur: 1,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=1) must succeed",
  );

  set_errno(ERRNO_SENTINEL);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 1);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful _SC_OPEN_MAX query with soft limit one must leave errno unchanged",
  );
}

#[test]
fn sysconf_open_max_one_soft_limit_repeated_success_keeps_value_and_errno() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");
  assert!(
    original.rlim_max >= 1,
    "setrlimit(RLIMIT_NOFILE, soft=1) requires hard limit >= 1",
  );

  let temporary = RLimit {
    rlim_cur: 1,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=1) must succeed",
  );

  set_errno(ERRNO_SENTINEL);

  let first_open_max = query(_SC_OPEN_MAX);
  let first_errno = read_errno();
  let second_open_max = query(_SC_OPEN_MAX);
  let second_errno = read_errno();

  assert_eq!(first_open_max, 1);
  assert_eq!(second_open_max, 1);
  assert_eq!(
    first_errno, ERRNO_SENTINEL,
    "first successful _SC_OPEN_MAX query must leave errno unchanged",
  );
  assert_eq!(
    second_errno, ERRNO_SENTINEL,
    "second successful _SC_OPEN_MAX query must also leave errno unchanged",
  );
}

#[test]
fn sysconf_open_max_one_soft_limit_repeated_success_preserves_enametoolong_from_gethostname_failure()
 {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");
  assert!(
    original.rlim_max >= 1,
    "setrlimit(RLIMIT_NOFILE, soft=1) requires hard limit >= 1",
  );

  let temporary = RLimit {
    rlim_cur: 1,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=1) must succeed",
  );

  let mut short_buffer = [0 as c_char; 1];

  set_errno(0);

  // SAFETY: `short_buffer` is valid writable memory and `len` matches it.
  let gethostname_result =
    unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let first_open_max = query(_SC_OPEN_MAX);
  let first_errno = read_errno();
  let second_open_max = query(_SC_OPEN_MAX);
  let second_errno = read_errno();

  assert_eq!(first_open_max, 1);
  assert_eq!(second_open_max, 1);
  assert_eq!(
    first_errno, ENAMETOOLONG,
    "first successful _SC_OPEN_MAX query must preserve ENAMETOOLONG",
  );
  assert_eq!(
    second_errno, ENAMETOOLONG,
    "second successful _SC_OPEN_MAX query must preserve ENAMETOOLONG",
  );
}

#[test]
fn sysconf_open_max_one_soft_limit_preserves_enametoolong_from_gethostname_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");
  assert!(
    original.rlim_max >= 1,
    "setrlimit(RLIMIT_NOFILE, soft=1) requires hard limit >= 1",
  );

  let temporary = RLimit {
    rlim_cur: 1,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=1) must succeed",
  );

  let mut short_buffer = [0 as c_char; 1];

  set_errno(0);

  // SAFETY: `short_buffer` is valid writable memory and `len` matches it.
  let gethostname_result =
    unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 1);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_OPEN_MAX query must preserve ENAMETOOLONG from prior gethostname failure even when value is one",
  );
}

#[test]
fn sysconf_open_max_one_soft_limit_preserves_enametoolong_from_gethostname_zero_length_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");
  assert!(
    original.rlim_max >= 1,
    "setrlimit(RLIMIT_NOFILE, soft=1) requires hard limit >= 1",
  );

  let temporary = RLimit {
    rlim_cur: 1,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=1) must succeed",
  );

  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 1);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_OPEN_MAX query must preserve ENAMETOOLONG even when soft limit is one",
  );
}

#[test]
fn sysconf_open_max_one_soft_limit_preserves_efault_from_gethostname_null_failure() {
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  // SAFETY: `original` points to writable storage for one `RLimit`.
  let read_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(read_status, 0, "getrlimit(RLIMIT_NOFILE) must succeed");
  assert!(
    original.rlim_max >= 1,
    "setrlimit(RLIMIT_NOFILE, soft=1) requires hard limit >= 1",
  );

  let temporary = RLimit {
    rlim_cur: 1,
    rlim_max: original.rlim_max,
  };
  let _restore_guard = RLimitRestoreGuard {
    resource: RLIMIT_NOFILE,
    original,
  };
  // SAFETY: `temporary` points to initialized `RLimit` data.
  let write_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const temporary) };

  assert_eq!(
    write_status, 0,
    "setrlimit(RLIMIT_NOFILE, soft=1) must succeed",
  );

  set_errno(0);

  // SAFETY: null `name` with nonzero `len` must fail with `EFAULT`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let open_max = query(_SC_OPEN_MAX);

  assert_eq!(open_max, 1);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful _SC_OPEN_MAX query must preserve EFAULT even when soft limit is one",
  );
}

#[test]
fn sysconf_online_preserves_enametoolong_from_gethostname_zero_length_failure() {
  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let online = query(_SC_NPROCESSORS_ONLN);

  assert!(online > 0, "_SC_NPROCESSORS_ONLN must be positive");
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_NPROCESSORS_ONLN query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_configured_preserves_enametoolong_from_gethostname_zero_length_failure() {
  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let configured = query(_SC_NPROCESSORS_CONF);

  assert!(configured > 0, "_SC_NPROCESSORS_CONF must be positive");
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_NPROCESSORS_CONF query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_nprocessors_values_are_positive() {
  set_errno(ERRNO_SENTINEL);

  let configured = query(_SC_NPROCESSORS_CONF);
  let online = query(_SC_NPROCESSORS_ONLN);

  assert!(configured > 0, "_SC_NPROCESSORS_CONF must be positive");
  assert!(online > 0, "_SC_NPROCESSORS_ONLN must be positive");
  assert!(configured >= online);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sysconf_nprocessors_onln_matches_sched_getaffinity_view() {
  set_errno(ERRNO_SENTINEL);

  let expected = current_affinity_cpu_count().expect("sched_getaffinity must succeed on linux");
  let actual = query(_SC_NPROCESSORS_ONLN);

  assert_eq!(actual, expected);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sysconf_nprocessors_onln_repeated_success_keeps_errno_sentinel() {
  set_errno(ERRNO_SENTINEL);

  let first_online = query(_SC_NPROCESSORS_ONLN);
  let first_errno = read_errno();
  let second_online = query(_SC_NPROCESSORS_ONLN);
  let second_errno = read_errno();

  assert!(first_online > 0, "_SC_NPROCESSORS_ONLN must be positive");
  assert!(second_online > 0, "_SC_NPROCESSORS_ONLN must be positive");
  assert_eq!(first_errno, ERRNO_SENTINEL);
  assert_eq!(second_errno, ERRNO_SENTINEL);
}

#[test]
fn sysconf_nprocessors_onln_preserves_efault_from_gethostname_null_failure() {
  set_errno(0);

  // SAFETY: null `name` with nonzero `len` must fail with `EFAULT`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let online = query(_SC_NPROCESSORS_ONLN);

  assert!(online > 0, "_SC_NPROCESSORS_ONLN must be positive");
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful _SC_NPROCESSORS_ONLN query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_nprocessors_onln_preserves_enametoolong_from_gethostname_failure() {
  let mut short_buffer = [0 as c_char; 1];

  set_errno(0);

  // SAFETY: `short_buffer` is valid writable memory and `len` matches it.
  let gethostname_result =
    unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let online = query(_SC_NPROCESSORS_ONLN);

  assert!(online > 0, "_SC_NPROCESSORS_ONLN must be positive");
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_NPROCESSORS_ONLN query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_nprocessors_onln_preserves_enametoolong_from_gethostname_zero_length_failure() {
  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let online = query(_SC_NPROCESSORS_ONLN);

  assert!(online > 0, "_SC_NPROCESSORS_ONLN must be positive");
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_NPROCESSORS_ONLN query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_nprocessors_conf_preserves_efault_from_gethostname_null_failure() {
  set_errno(0);

  // SAFETY: null `name` with nonzero `len` must fail with `EFAULT`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let configured = query(_SC_NPROCESSORS_CONF);

  assert!(configured > 0, "_SC_NPROCESSORS_CONF must be positive");
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful _SC_NPROCESSORS_CONF query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_nprocessors_conf_preserves_enametoolong_from_gethostname_failure() {
  let mut short_buffer = [0 as c_char; 1];

  set_errno(0);

  // SAFETY: `short_buffer` is valid writable memory and `len` matches it.
  let gethostname_result =
    unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let configured = query(_SC_NPROCESSORS_CONF);

  assert!(configured > 0, "_SC_NPROCESSORS_CONF must be positive");
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_NPROCESSORS_CONF query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_nprocessors_conf_preserves_enametoolong_from_gethostname_zero_length_failure() {
  set_errno(0);

  // SAFETY: `len == 0` must fail with `ENAMETOOLONG` without dereferencing `name`.
  let gethostname_result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(gethostname_result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let configured = query(_SC_NPROCESSORS_CONF);

  assert!(configured > 0, "_SC_NPROCESSORS_CONF must be positive");
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful _SC_NPROCESSORS_CONF query must preserve errno from prior failure",
  );
}

#[test]
fn sysconf_nprocessors_conf_matches_cpu_possible_or_present_when_parseable() {
  set_errno(ERRNO_SENTINEL);

  let configured = query(_SC_NPROCESSORS_CONF);

  assert!(configured > 0, "_SC_NPROCESSORS_CONF must be positive");

  let expected = parsed_cpu_count_from_sysfs(CPU_POSSIBLE_PATH)
    .or_else(|| parsed_cpu_count_from_sysfs(CPU_PRESENT_PATH));

  if let Some(expected) = expected {
    assert_eq!(configured, expected);
  }

  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sysconf_unsupported_name_sets_einval() {
  set_errno(0);

  let result = query(UNSUPPORTED_NAME);

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sysconf_success_does_not_clear_errno_after_prior_failure() {
  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let supported_result = query(_SC_OPEN_MAX);

  assert_open_max_query_succeeded(supported_result);
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful sysconf must leave errno unchanged after earlier failure",
  );
}

#[test]
fn sysconf_online_success_does_not_clear_errno_after_prior_failure() {
  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let online_result = query(_SC_NPROCESSORS_ONLN);

  assert!(online_result > 0, "_SC_NPROCESSORS_ONLN must be positive");
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful sysconf must leave errno unchanged after earlier failure",
  );
}

#[test]
fn sysconf_configured_success_does_not_clear_errno_after_prior_failure() {
  set_errno(0);

  let unsupported_result = query(UNSUPPORTED_NAME);

  assert_eq!(unsupported_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let configured_result = query(_SC_NPROCESSORS_CONF);

  assert!(
    configured_result > 0,
    "_SC_NPROCESSORS_CONF must be positive"
  );
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful sysconf must leave errno unchanged after earlier failure",
  );
}

#[test]
fn parse_cpu_range_list_rejects_duplicate_and_overlapping_entries() {
  assert_eq!(parse_cpu_range_list("0,0"), None);
  assert_eq!(parse_cpu_range_list("0-2,2-3"), None);
  assert_eq!(parse_cpu_range_list("0-1,1-2"), None);
}

#[test]
fn parse_cpu_range_list_rejects_descending_sequence() {
  assert_eq!(parse_cpu_range_list("2,1"), None);
}

#[test]
fn parse_cpu_range_list_rejects_descending_range_bounds() {
  assert_eq!(parse_cpu_range_list("3-1"), None);
}

#[test]
fn parse_cpu_range_list_rejects_internal_empty_tokens() {
  assert_eq!(parse_cpu_range_list("0,,1"), None);
  assert_eq!(parse_cpu_range_list("0, ,1"), None);
}

#[test]
fn parse_cpu_range_list_rejects_leading_empty_tokens() {
  assert_eq!(parse_cpu_range_list(",0-1"), None);
  assert_eq!(parse_cpu_range_list(" ,0-1"), None);
}

#[test]
fn parse_cpu_range_list_rejects_malformed_range_tokens() {
  assert_eq!(parse_cpu_range_list("0-1-2"), None);
  assert_eq!(parse_cpu_range_list("1--2"), None);
  assert_eq!(parse_cpu_range_list("-1"), None);
  assert_eq!(parse_cpu_range_list("1-"), None);
}

#[test]
fn parse_cpu_range_list_rejects_signed_numeric_tokens() {
  assert_eq!(parse_cpu_range_list("+1"), None);
  assert_eq!(parse_cpu_range_list("0,+1"), None);
  assert_eq!(parse_cpu_range_list("+0-1"), None);
  assert_eq!(parse_cpu_range_list("0-+1"), None);
}

#[test]
fn parse_cpu_range_list_rejects_whitespace_inside_range_bounds() {
  assert_eq!(parse_cpu_range_list("0 -1"), None);
  assert_eq!(parse_cpu_range_list("0- 1"), None);
  assert_eq!(parse_cpu_range_list("0 - 1"), None);
}

#[test]
fn parse_cpu_range_list_rejects_overflowing_total_count() {
  let overflowing = format!("0-{},0", usize::MAX);

  assert_eq!(parse_cpu_range_list(&overflowing), None);
}

#[test]
fn parse_cpu_range_list_rejects_full_usize_span_range() {
  let full_span = format!("0-{}", usize::MAX);

  assert_eq!(parse_cpu_range_list(&full_span), None);
}

#[test]
fn parse_cpu_range_list_rejects_empty_or_trailing_tokens() {
  assert_eq!(parse_cpu_range_list(""), None);
  assert_eq!(parse_cpu_range_list("   "), None);
  assert_eq!(parse_cpu_range_list("0-1,"), None);
  assert_eq!(parse_cpu_range_list("0-1, "), None);
}

#[test]
fn parse_cpu_range_list_accepts_whitespace_and_adjacent_ranges() {
  assert_eq!(parse_cpu_range_list(" 0-1 , 3 , 5-6 "), Some(5));
  assert_eq!(parse_cpu_range_list("0-2,3-4"), Some(5));
}

#[test]
fn parse_cpu_range_list_accepts_sysfs_style_trailing_newline() {
  assert_eq!(parse_cpu_range_list("0-3\n"), Some(4));
}

#[test]
fn parse_cpu_range_list_accepts_single_maximum_cpu_index() {
  assert_eq!(parse_cpu_range_list(&usize::MAX.to_string()), Some(1));
}

#[test]
fn parse_cpu_range_list_accepts_near_maximum_contiguous_ranges() {
  let near_max = format!("{}-{}", usize::MAX - 2, usize::MAX);

  assert_eq!(parse_cpu_range_list(&near_max), Some(3));
}

#[test]
fn parse_cpu_range_list_accepts_maximum_terminal_range() {
  let near_max = usize::MAX - 1;
  let input = format!("{near_max}-{}", usize::MAX);

  assert_eq!(parse_cpu_range_list(&input), Some(2));
}

#[test]
fn parse_cpu_range_list_accepts_terminal_maximum_single_after_prefix() {
  let input = format!("0,{}", usize::MAX);

  assert_eq!(parse_cpu_range_list(&input), Some(2));
}

#[test]
fn parse_cpu_range_list_accepts_terminal_maximum_range_after_prefix() {
  let input = format!("0,{}-{}", usize::MAX - 1, usize::MAX);

  assert_eq!(parse_cpu_range_list(&input), Some(3));
}

#[test]
fn parse_cpu_range_list_accepts_terminal_maximum_single_after_near_max_range() {
  let input = format!("{}-{},{}", usize::MAX - 2, usize::MAX - 1, usize::MAX);

  assert_eq!(parse_cpu_range_list(&input), Some(3));
}

#[test]
fn parse_cpu_range_list_accepts_terminal_maximum_range_after_near_max_prefix_range() {
  let input = format!(
    "{}-{},{}-{}",
    usize::MAX - 3,
    usize::MAX - 2,
    usize::MAX - 1,
    usize::MAX
  );

  assert_eq!(parse_cpu_range_list(&input), Some(4));
}

#[test]
fn parse_cpu_range_list_accepts_near_maximum_non_contiguous_terminal_single() {
  let input = format!("{}-{},{}", usize::MAX - 3, usize::MAX - 2, usize::MAX);

  assert_eq!(parse_cpu_range_list(&input), Some(3));
}

#[test]
fn parse_cpu_range_list_rejects_out_of_range_numeric_tokens() {
  let too_large = format!("{}0", usize::MAX);

  assert_eq!(parse_cpu_range_list(&too_large), None);
  assert_eq!(parse_cpu_range_list(&format!("0-{too_large}")), None);
  assert_eq!(parse_cpu_range_list(&format!("{too_large}-0")), None);
}

#[test]
fn parse_cpu_range_list_rejects_non_terminal_maximum_tokens() {
  let input = format!("{},0", usize::MAX);

  assert_eq!(parse_cpu_range_list(&input), None);
}

#[test]
fn parse_cpu_range_list_rejects_non_terminal_maximum_range_tokens() {
  let input = format!("{}-{},0", usize::MAX - 1, usize::MAX);

  assert_eq!(parse_cpu_range_list(&input), None);
}

#[test]
fn parse_cpu_range_list_rejects_near_maximum_overlapping_ranges() {
  let input = format!(
    "{}-{},{}-{}",
    usize::MAX - 2,
    usize::MAX - 1,
    usize::MAX - 1,
    usize::MAX
  );

  assert_eq!(parse_cpu_range_list(&input), None);
}
