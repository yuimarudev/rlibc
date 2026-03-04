use core::mem::{align_of, size_of};
use core::ptr;
use rlibc::abi::errno::{EFAULT, EINVAL};
use rlibc::abi::types::{c_int, c_long};
use rlibc::errno::__errno_location;
use rlibc::time::{
  CLOCK_MONOTONIC, CLOCK_REALTIME, clock_gettime, gettimeofday, timespec, timeval, timezone,
};

fn read_errno() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid TLS storage for the calling thread.
  unsafe { errno_ptr.read() }
}

fn write_errno(value: c_int) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid TLS storage for the calling thread.
  unsafe {
    errno_ptr.write(value);
  }
}

#[test]
fn gettimeofday_populates_timeval_and_usec_is_in_range_without_changing_errno() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(91);
  // SAFETY: `tv` is a valid mutable pointer and `tz` is explicitly null.
  let rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(rc, 0);
  assert_eq!(read_errno(), 91);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_accepts_null_tv_and_tz() {
  write_errno(77);
  // SAFETY: libc contract permits null output pointers.
  let rc = unsafe { gettimeofday(ptr::null_mut(), ptr::null_mut()) };

  assert_eq!(rc, 0);
  assert_eq!(read_errno(), 77);
}

#[test]
fn gettimeofday_invalid_pointer_sets_efault() {
  write_errno(0);
  // SAFETY: pointer is intentionally invalid to validate errno path.
  let rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), ptr::null_mut()) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_invalid_tv_with_valid_timezone_pointer_sets_efault() {
  let mut tz = timezone {
    tz_minuteswest: 0,
    tz_dsttime: 0,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid and `tz` is valid writable storage.
  let rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), &raw mut tz) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_failure_overwrites_existing_errno() {
  write_errno(123);
  // SAFETY: pointer is intentionally invalid to validate errno overwrite behavior.
  let rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), ptr::null_mut()) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_invalid_tv_with_valid_timezone_pointer_overwrites_existing_errno() {
  let mut tz = timezone {
    tz_minuteswest: 0,
    tz_dsttime: 0,
  };

  write_errno(47);
  // SAFETY: `tv` is intentionally invalid and `tz` is valid writable storage.
  let rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), &raw mut tz) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_null_after_invalid_tv_with_valid_timezone_failure_keeps_errno_unchanged() {
  let mut tz = timezone {
    tz_minuteswest: 0,
    tz_dsttime: 0,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid and `tz` is valid writable storage.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), &raw mut tz) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: libc contract permits both output pointers to be null.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_valid_tv_after_invalid_tv_with_valid_timezone_failure_keeps_errno_unchanged() {
  let mut tz = timezone {
    tz_minuteswest: 0,
    tz_dsttime: 0,
  };
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid and `tz` is valid writable storage.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), &raw mut tz) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_valid_tv_and_tz_after_invalid_tv_with_valid_timezone_failure_keeps_errno_unchanged()
{
  let mut fail_tz = timezone {
    tz_minuteswest: 0,
    tz_dsttime: 0,
  };
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid and `fail_tz` is valid writable storage.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), &raw mut fail_tz) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` and `tz` are valid writable pointers.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_null_tv_with_valid_timezone_after_invalid_tv_with_valid_timezone_failure_keeps_errno_unchanged()
 {
  let mut fail_tz = timezone {
    tz_minuteswest: 0,
    tz_dsttime: 0,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid and `fail_tz` is valid writable storage.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), &raw mut fail_tz) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tz` is valid writable storage and `tv` is null by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_success_after_failure_keeps_errno_unchanged() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);
  // SAFETY: pointer is intentionally invalid to set errno to EFAULT.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), ptr::null_mut()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is a valid mutable pointer and `tz` is explicitly null.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_valid_tv_and_tz_after_invalid_tv_failure_keeps_errno_unchanged() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid to set errno to EFAULT.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), ptr::null_mut()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` and `tz` are valid writable pointers.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_null_null_after_failure_keeps_errno_unchanged() {
  write_errno(0);
  // SAFETY: pointer is intentionally invalid to set errno to EFAULT.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), ptr::null_mut()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: libc contract permits both output pointers to be null.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_null_after_invalid_timezone_failure_keeps_errno_unchanged() {
  let mut tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to set errno to EFAULT.
  let fail_rc = unsafe { gettimeofday(&raw mut tv, std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: libc contract permits both output pointers to be null.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_valid_tv_after_invalid_timezone_failure_keeps_errno_unchanged() {
  let mut fail_tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to set errno to EFAULT.
  let fail_rc = unsafe { gettimeofday(&raw mut fail_tv, std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_invalid_timezone_pointer_sets_efault() {
  let mut tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to validate errno propagation.
  let rc = unsafe { gettimeofday(&raw mut tv, std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_invalid_timezone_pointer_overwrites_existing_errno() {
  let mut tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };

  write_errno(88);
  // SAFETY: `tz` is intentionally invalid to validate errno overwrite behavior.
  let rc = unsafe { gettimeofday(&raw mut tv, std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_accepts_timezone_pointer_and_preserves_errno() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let mut tz = timezone {
    tz_minuteswest: 1234,
    tz_dsttime: 4321,
  };

  write_errno(123);
  // SAFETY: `tv` and `tz` are valid mutable pointers.
  let rc = unsafe { gettimeofday(&raw mut tv, &raw mut tz) };

  assert_eq!(rc, 0);
  assert_eq!(read_errno(), 123);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_null_tv_with_invalid_timezone_pointer_sets_efault() {
  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to validate error propagation.
  let rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_tv_with_invalid_timezone_pointer_overwrites_existing_errno() {
  write_errno(66);
  // SAFETY: `tz` is intentionally invalid to validate errno overwrite behavior.
  let rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_valid_tv_and_tz_after_null_tv_invalid_timezone_failure_keeps_errno_unchanged() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to force `EFAULT` with `tv` null.
  let fail_rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` and `tz` are valid writable pointers.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_valid_tv_after_null_tv_invalid_timezone_failure_keeps_errno_unchanged() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to force `EFAULT` with `tv` null.
  let fail_rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_null_null_after_null_tv_invalid_timezone_failure_keeps_errno_unchanged() {
  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to force `EFAULT` with `tv` null.
  let fail_rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: libc contract permits both output pointers to be null.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_tv_with_valid_timezone_after_null_tv_invalid_timezone_failure_keeps_errno_unchanged()
 {
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to force `EFAULT` with `tv` null.
  let fail_rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tz` is valid writable storage and `tv` is null by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_both_invalid_pointers_set_efault() {
  write_errno(0);
  // SAFETY: both pointers are intentionally invalid to validate kernel error propagation.
  let rc = unsafe {
    gettimeofday(
      std::ptr::dangling_mut::<timeval>(),
      std::ptr::dangling_mut::<timezone>(),
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_both_invalid_pointers_overwrite_existing_errno() {
  write_errno(64);
  // SAFETY: both pointers are intentionally invalid to validate errno overwrite behavior.
  let rc = unsafe {
    gettimeofday(
      std::ptr::dangling_mut::<timeval>(),
      std::ptr::dangling_mut::<timezone>(),
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_null_after_both_invalid_pointers_failure_keeps_errno_unchanged() {
  write_errno(0);
  // SAFETY: both pointers are intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe {
    gettimeofday(
      std::ptr::dangling_mut::<timeval>(),
      std::ptr::dangling_mut::<timezone>(),
    )
  };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: libc contract permits both output pointers to be null.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_tv_with_valid_timezone_after_both_invalid_pointers_failure_keeps_errno_unchanged()
 {
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: both pointers are intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe {
    gettimeofday(
      std::ptr::dangling_mut::<timeval>(),
      std::ptr::dangling_mut::<timezone>(),
    )
  };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tz` is valid writable storage and `tv` is null by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_valid_tv_after_both_invalid_pointers_failure_keeps_errno_unchanged() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);
  // SAFETY: both pointers are intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe {
    gettimeofday(
      std::ptr::dangling_mut::<timeval>(),
      std::ptr::dangling_mut::<timezone>(),
    )
  };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is a valid writable pointer and `tz` is null.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_valid_tv_and_tz_after_both_invalid_pointers_failure_keeps_errno_unchanged() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: both pointers are intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe {
    gettimeofday(
      std::ptr::dangling_mut::<timeval>(),
      std::ptr::dangling_mut::<timezone>(),
    )
  };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` and `tz` are valid writable pointers.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_accepts_null_tv_with_valid_timezone_pointer() {
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(55);
  // SAFETY: `tz` is a valid writable pointer and `tv` is explicitly null.
  let rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(rc, 0);
  assert_eq!(read_errno(), 55);
}

#[test]
fn gettimeofday_null_tv_with_valid_timezone_after_failure_keeps_errno_unchanged() {
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: pointer is intentionally invalid to set errno to EFAULT.
  let fail_rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tz` is valid writable storage and `tv` is null by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_tv_with_valid_timezone_after_valid_tv_invalid_timezone_failure_keeps_errno_unchanged()
 {
  let mut tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe { gettimeofday(&raw mut tv, std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tz` is valid writable storage and `tv` is null by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_tv_valid_timezone_after_failure_keeps_errno_unchanged() {
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: pointer is intentionally invalid to set errno to EFAULT.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), ptr::null_mut()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tz` is valid writable storage and `tv` is null by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_valid_tv_and_tz_after_failure_keeps_errno_unchanged() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to force an `EFAULT` failure first.
  let fail_rc = unsafe { gettimeofday(&raw mut tv, std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` and `tz` are valid writable pointers.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn timeval_and_timezone_match_x86_64_linux_layout() {
  assert_eq!(size_of::<timespec>(), 16);
  assert_eq!(align_of::<timespec>(), 8);
  assert_eq!(size_of::<timeval>(), 16);
  assert_eq!(align_of::<timeval>(), 8);
  assert_eq!(size_of::<timezone>(), 8);
  assert_eq!(align_of::<timezone>(), 4);
}

#[test]
fn clockid_t_and_time_t_match_x86_64_linux_abi() {
  assert_eq!(size_of::<rlibc::time::clockid_t>(), 4);
  assert_eq!(align_of::<rlibc::time::clockid_t>(), 4);
  assert_eq!(size_of::<rlibc::time::time_t>(), 8);
  assert_eq!(align_of::<rlibc::time::time_t>(), 8);
}

#[test]
fn clockid_t_and_time_t_match_c_primitive_alias_abi_on_x86_64_linux() {
  assert_eq!(size_of::<rlibc::time::clockid_t>(), size_of::<c_int>());
  assert_eq!(align_of::<rlibc::time::clockid_t>(), align_of::<c_int>());
  assert_eq!(size_of::<rlibc::time::time_t>(), size_of::<c_long>());
  assert_eq!(align_of::<rlibc::time::time_t>(), align_of::<c_long>());
}

#[test]
fn clockid_t_and_time_t_are_signed_on_x86_64_linux() {
  let clock_id_neg: rlibc::time::clockid_t = -1;
  let time_neg: rlibc::time::time_t = -1;

  assert!(clock_id_neg < 0);
  assert!(time_neg < 0);
}

#[test]
fn time_struct_fields_are_signed_on_x86_64_linux() {
  let ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  assert!(ts.tv_sec < 0);
  assert!(ts.tv_nsec < 0);
  assert!(tv.tv_sec < 0);
  assert!(tv.tv_usec < 0);
  assert!(tz.tz_minuteswest < 0);
  assert!(tz.tz_dsttime < 0);
}

#[test]
fn time_t_exceeds_32bit_unix_second_range_on_x86_64_linux() {
  let year_3000: rlibc::time::time_t = 32_503_680_000;
  let year_1900: rlibc::time::time_t = -2_208_988_800;

  assert!(year_3000 > 2_147_483_647 as rlibc::time::time_t);
  assert!(year_1900 < 0);
}

#[test]
fn clock_gettime_populates_timespec_and_nsec_is_in_range_without_changing_errno() {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(31);

  let rc = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(rc, 0);
  assert_eq!(read_errno(), 31);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn timespec_field_offsets_match_x86_64_linux_abi() {
  let ts_uninit = core::mem::MaybeUninit::<timespec>::uninit();
  let ts_base = ts_uninit.as_ptr().addr();
  // SAFETY: We only compute field addresses from an uninitialized allocation;
  // no reads or writes are performed.
  let ts_sec_offset = unsafe { core::ptr::addr_of!((*ts_uninit.as_ptr()).tv_sec).addr() - ts_base };
  // SAFETY: We only compute field addresses from an uninitialized allocation;
  // no reads or writes are performed.
  let ts_nsec_offset =
    unsafe { core::ptr::addr_of!((*ts_uninit.as_ptr()).tv_nsec).addr() - ts_base };

  assert_eq!(ts_sec_offset, 0);
  assert_eq!(ts_nsec_offset, 8);
}

#[test]
fn timeval_and_timezone_field_offsets_match_x86_64_linux_abi() {
  let tv_uninit = core::mem::MaybeUninit::<timeval>::uninit();
  let tv_base = tv_uninit.as_ptr().addr();
  // SAFETY: We only compute field addresses from an uninitialized allocation;
  // no reads or writes are performed.
  let tv_sec_offset = unsafe { core::ptr::addr_of!((*tv_uninit.as_ptr()).tv_sec).addr() - tv_base };
  // SAFETY: We only compute field addresses from an uninitialized allocation;
  // no reads or writes are performed.
  let tv_usec_offset =
    unsafe { core::ptr::addr_of!((*tv_uninit.as_ptr()).tv_usec).addr() - tv_base };

  assert_eq!(tv_sec_offset, 0);
  assert_eq!(tv_usec_offset, 8);

  let timezone_uninit = core::mem::MaybeUninit::<timezone>::uninit();
  let timezone_base = timezone_uninit.as_ptr().addr();
  // SAFETY: We only compute field addresses from an uninitialized allocation;
  // no reads or writes are performed.
  let tz_minuteswest_offset = unsafe {
    core::ptr::addr_of!((*timezone_uninit.as_ptr()).tz_minuteswest).addr() - timezone_base
  };
  // SAFETY: We only compute field addresses from an uninitialized allocation;
  // no reads or writes are performed.
  let tz_dsttime_offset =
    unsafe { core::ptr::addr_of!((*timezone_uninit.as_ptr()).tz_dsttime).addr() - timezone_base };

  assert_eq!(tz_minuteswest_offset, 0);
  assert_eq!(tz_dsttime_offset, 4);
}

#[test]
fn gettimeofday_realtime_seconds_stay_close_to_clock_gettime() {
  let mut tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(9);
  // SAFETY: `tv` is a valid writable pointer and `tz` is null.
  let gettimeofday_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };
  let clock_gettime_rc = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(gettimeofday_rc, 0);
  assert_eq!(clock_gettime_rc, 0);
  assert_eq!(read_errno(), 9);

  let sec_delta = (tv.tv_sec - ts.tv_sec).abs();

  assert!(
    sec_delta <= 1,
    "realtime second delta must stay within 1 second, got {sec_delta}",
  );
}

#[test]
fn clock_gettime_success_after_gettimeofday_failure_keeps_errno_efault() {
  let mut tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe { gettimeofday(&raw mut tv, std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_gettimeofday_failure_keeps_errno_efault() {
  let mut tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe { gettimeofday(&raw mut tv, std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_success_after_gettimeofday_both_invalid_pointers_failure_keeps_errno_efault() {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: both pointers are intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe {
    gettimeofday(
      std::ptr::dangling_mut::<timeval>(),
      std::ptr::dangling_mut::<timezone>(),
    )
  };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_gettimeofday_both_invalid_pointers_failure_keeps_errno_efault()
 {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: both pointers are intentionally invalid to force `EFAULT`.
  let fail_rc = unsafe {
    gettimeofday(
      std::ptr::dangling_mut::<timeval>(),
      std::ptr::dangling_mut::<timezone>(),
    )
  };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_success_after_gettimeofday_null_tv_invalid_timezone_failure_keeps_errno_efault() {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid while `tv` is null to force `EFAULT`.
  let fail_rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_gettimeofday_null_tv_invalid_timezone_failure_keeps_errno_efault()
 {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: `tz` is intentionally invalid while `tv` is null to force `EFAULT`.
  let fail_rc = unsafe { gettimeofday(ptr::null_mut(), std::ptr::dangling_mut::<timezone>()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_success_after_gettimeofday_invalid_tv_with_valid_timezone_failure_keeps_errno_efault()
 {
  let mut tz = timezone {
    tz_minuteswest: 0,
    tz_dsttime: 0,
  };
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid while `tz` is valid writable storage.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), &raw mut tz) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_gettimeofday_invalid_tv_with_valid_timezone_failure_keeps_errno_efault()
 {
  let mut tz = timezone {
    tz_minuteswest: 0,
    tz_dsttime: 0,
  };
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid while `tz` is valid writable storage.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), &raw mut tz) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_success_after_gettimeofday_invalid_tv_null_timezone_failure_keeps_errno_efault() {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid while `tz` is null.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), ptr::null_mut()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_gettimeofday_invalid_tv_null_timezone_failure_keeps_errno_efault()
 {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);
  // SAFETY: `tv` is intentionally invalid while `tz` is null.
  let fail_rc = unsafe { gettimeofday(std::ptr::dangling_mut::<timeval>(), ptr::null_mut()) };

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_invalid_clock_id_overwrites_existing_errno_to_einval() {
  let mut invalid_clock_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(61);

  let rc = clock_gettime(c_int::MAX, &raw mut invalid_clock_ts);

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_extreme_negative_invalid_clock_id_overwrites_existing_errno_to_einval() {
  let mut invalid_clock_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(57);

  let rc = clock_gettime(c_int::MIN, &raw mut invalid_clock_ts);

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_null_timespec_overwrites_existing_errno_to_efault() {
  write_errno(29);

  let rc = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_invalid_clock_id_with_null_timespec_overwrites_existing_errno_to_efault() {
  write_errno(83);

  let rc = clock_gettime(c_int::MAX, ptr::null_mut());

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_extreme_negative_invalid_clock_id_with_null_timespec_overwrites_existing_errno_to_efault()
 {
  write_errno(39);

  let rc = clock_gettime(c_int::MIN, ptr::null_mut());

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_success_after_clock_gettime_invalid_clock_id_keeps_errno_einval() {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(c_int::MAX, &raw mut ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn clock_gettime_success_after_invalid_clock_id_failure_keeps_errno_einval() {
  let mut invalid_clock_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut realtime_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(c_int::MAX, &raw mut invalid_clock_ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut realtime_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(realtime_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&realtime_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_success_after_extreme_negative_invalid_clock_id_failure_keeps_errno_einval() {
  let mut invalid_clock_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut realtime_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(c_int::MIN, &raw mut invalid_clock_ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut realtime_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(realtime_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&realtime_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_success_after_large_positive_invalid_clock_id_failure_keeps_errno_einval() {
  let mut invalid_clock_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut realtime_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(123_456, &raw mut invalid_clock_ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut realtime_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(realtime_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&realtime_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_invalid_clock_id_failure_keeps_errno_einval() {
  let mut invalid_clock_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut monotonic_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(c_int::MAX, &raw mut invalid_clock_ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut monotonic_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(monotonic_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&monotonic_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_extreme_negative_invalid_clock_id_failure_keeps_errno_einval()
 {
  let mut invalid_clock_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut monotonic_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(c_int::MIN, &raw mut invalid_clock_ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut monotonic_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(monotonic_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&monotonic_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_large_positive_invalid_clock_id_failure_keeps_errno_einval()
 {
  let mut invalid_clock_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut monotonic_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(123_456, &raw mut invalid_clock_ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut monotonic_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(monotonic_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&monotonic_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn gettimeofday_success_after_clock_gettime_extreme_negative_invalid_clock_id_keeps_errno_einval() {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(c_int::MIN, &raw mut ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_success_after_clock_gettime_large_positive_invalid_clock_id_keeps_errno_einval() {
  let mut ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(123_456, &raw mut ts);

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_success_after_clock_gettime_null_timespec_with_invalid_clock_id_keeps_errno_efault()
{
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(c_int::MAX, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_success_after_clock_gettime_null_timespec_with_extreme_negative_invalid_clock_id_keeps_errno_efault()
 {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(c_int::MIN, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_success_after_clock_gettime_null_timespec_with_large_positive_invalid_clock_id_keeps_errno_efault()
 {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(123_456, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_success_after_clock_gettime_null_timespec_with_monotonic_clock_keeps_errno_efault()
{
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_MONOTONIC, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_success_after_clock_gettime_null_timespec_failure_keeps_errno_efault() {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` is valid writable storage and `tz` is null by contract.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_null_null_after_clock_gettime_null_timespec_failure_keeps_errno_efault() {
  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: Both pointers are null and this call is permitted by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_null_after_clock_gettime_monotonic_null_timespec_failure_keeps_errno_efault() {
  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_MONOTONIC, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: Both pointers are null and this call is permitted by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), ptr::null_mut()) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_tv_with_valid_timezone_after_clock_gettime_monotonic_null_timespec_failure_keeps_errno_efault()
 {
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_MONOTONIC, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tz` is valid writable storage and `tv` is null by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_null_tv_with_valid_timezone_after_clock_gettime_realtime_null_timespec_failure_keeps_errno_efault()
 {
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tz` is valid writable storage and `tv` is null by contract.
  let ok_rc = unsafe { gettimeofday(ptr::null_mut(), &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gettimeofday_valid_tv_and_tz_after_clock_gettime_monotonic_null_timespec_failure_keeps_errno_efault()
 {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_MONOTONIC, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` and `tz` are valid writable pointers.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn gettimeofday_valid_tv_and_tz_after_clock_gettime_realtime_null_timespec_failure_keeps_errno_efault()
 {
  let mut tv = timeval {
    tv_sec: -1,
    tv_usec: -1,
  };
  let mut tz = timezone {
    tz_minuteswest: -1,
    tz_dsttime: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: `tv` and `tz` are valid writable pointers.
  let ok_rc = unsafe { gettimeofday(&raw mut tv, &raw mut tz) };

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(tv.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000).contains(&tv.tv_usec),
    "tv_usec must be in [0, 1_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_null_timespec_failure_keeps_errno_efault() {
  let mut monotonic_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut monotonic_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(monotonic_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&monotonic_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_realtime_success_after_monotonic_null_timespec_failure_keeps_errno_efault() {
  let mut realtime_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_MONOTONIC, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut realtime_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(realtime_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&realtime_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_monotonic_success_after_monotonic_null_timespec_failure_keeps_errno_efault() {
  let mut monotonic_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_MONOTONIC, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_MONOTONIC, &raw mut monotonic_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(monotonic_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&monotonic_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn clock_gettime_realtime_success_after_realtime_null_timespec_failure_keeps_errno_efault() {
  let mut realtime_ts = timespec {
    tv_sec: -1,
    tv_nsec: -1,
  };

  write_errno(0);

  let fail_rc = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(fail_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  let ok_rc = clock_gettime(CLOCK_REALTIME, &raw mut realtime_ts);

  assert_eq!(ok_rc, 0);
  assert_eq!(read_errno(), EFAULT);
  assert!(realtime_ts.tv_sec >= 0, "tv_sec must be non-negative");
  assert!(
    (0..1_000_000_000).contains(&realtime_ts.tv_nsec),
    "tv_nsec must be in [0, 1_000_000_000)",
  );
}

#[test]
fn gettimeofday_timestamp_is_bracketed_by_clock_gettime_samples() {
  let mut before = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };
  let mut tv = timeval {
    tv_sec: 0,
    tv_usec: 0,
  };
  let mut after = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(44);

  let before_rc = clock_gettime(CLOCK_REALTIME, &raw mut before);
  // SAFETY: `tv` is valid writable storage and `tz` is null.
  let gettimeofday_rc = unsafe { gettimeofday(&raw mut tv, ptr::null_mut()) };
  let after_rc = clock_gettime(CLOCK_REALTIME, &raw mut after);

  assert_eq!(before_rc, 0);
  assert_eq!(gettimeofday_rc, 0);
  assert_eq!(after_rc, 0);
  assert_eq!(read_errno(), 44);

  let before_us = i128::from(before.tv_sec) * 1_000_000 + i128::from(before.tv_nsec) / 1_000;
  let tv_us = i128::from(tv.tv_sec) * 1_000_000 + i128::from(tv.tv_usec);
  let after_us = i128::from(after.tv_sec) * 1_000_000 + i128::from(after.tv_nsec) / 1_000;
  let lower_bound = before_us - 1_000_000;
  let upper_bound = after_us + 1_000_000;

  assert!(
    (lower_bound..=upper_bound).contains(&tv_us),
    "gettimeofday timestamp {tv_us}us must be near bracket [{before_us}us, {after_us}us]",
  );
}
