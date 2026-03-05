#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::mem::{align_of, size_of};
use rlibc::abi::errno::{EFAULT, EINVAL, ESRCH};
use rlibc::abi::types::c_int;
use rlibc::resource::{RLIMIT_NOFILE, RLimit, getrlimit, prlimit64, setrlimit};
use std::sync::{Mutex, OnceLock};

const INVALID_RESOURCE: c_int = -1;
const ERRNO_SENTINEL: c_int = 1777;

unsafe extern "C" {
  fn __errno_location() -> *mut c_int;
}

fn errno_ptr() -> *mut c_int {
  // SAFETY: `__errno_location` returns a thread-local writable errno pointer.
  let pointer = unsafe { __errno_location() };

  assert!(!pointer.is_null(), "__errno_location returned null");

  pointer
}

fn set_errno(value: c_int) {
  // SAFETY: `errno_ptr` guarantees a valid writable pointer.
  unsafe {
    errno_ptr().write(value);
  }
}

fn read_errno() -> c_int {
  // SAFETY: `errno_ptr` guarantees a valid readable pointer.
  unsafe { errno_ptr().read() }
}

fn process_wide_rlimit_lock() -> std::sync::MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  LOCK
    .get_or_init(|| Mutex::new(()))
    .lock()
    .expect("resource-limit test lock poisoned")
}

#[test]
fn rlimit_layout_matches_linux_x86_64() {
  assert_eq!(size_of::<RLimit>(), 16);
  assert_eq!(align_of::<RLimit>(), 8);
}

#[test]
fn getrlimit_nofile_returns_current_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };

  set_errno(ERRNO_SENTINEL);

  let status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut limits) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(limits.rlim_cur > 0);
  assert!(limits.rlim_max >= limits.rlim_cur);
}

#[test]
fn prlimit64_pid_zero_reads_same_value_as_getrlimit() {
  let _guard = process_wide_rlimit_lock();
  let mut via_getrlimit = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut via_prlimit = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };

  set_errno(ERRNO_SENTINEL);

  let get_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut via_getrlimit) };

  assert_eq!(get_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  set_errno(ERRNO_SENTINEL);

  let pr_status = unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), &raw mut via_prlimit) };

  assert_eq!(pr_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert_eq!(via_prlimit, via_getrlimit);
}

#[test]
fn prlimit64_with_both_limit_pointers_null_is_noop() {
  let _guard = process_wide_rlimit_lock();

  set_errno(ERRNO_SENTINEL);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn prlimit64_returns_previous_limits_when_setting_new_limit() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut previous = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let lowered = RLimit {
    rlim_cur: original.rlim_cur.saturating_sub(1),
    rlim_max: original.rlim_max,
  };

  set_errno(ERRNO_SENTINEL);

  let set_status = unsafe { prlimit64(0, RLIMIT_NOFILE, &raw const lowered, &raw mut previous) };

  assert_eq!(set_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert_eq!(
    previous, original,
    "prlimit64 must return the pre-update limit in old_limit"
  );

  let get_lowered = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_lowered, 0, "post-set getrlimit must succeed");
  assert_eq!(observed, lowered);

  let restore_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const original) };

  assert_eq!(restore_status, 0, "restoring original limit must succeed");
}

#[test]
fn prlimit64_with_new_limit_and_null_old_limit_updates_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let lowered = RLimit {
    rlim_cur: original.rlim_cur.saturating_sub(1),
    rlim_max: original.rlim_max,
  };

  set_errno(ERRNO_SENTINEL);

  let set_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, &raw const lowered, core::ptr::null_mut()) };

  assert_eq!(set_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let get_lowered = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_lowered, 0, "post-set getrlimit must succeed");
  assert_eq!(observed, lowered);

  let restore_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const original) };

  assert_eq!(restore_status, 0, "restoring original limit must succeed");
}

#[test]
fn setrlimit_can_roundtrip_soft_nofile_limit() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let lowered = RLimit {
    rlim_cur: original.rlim_cur.saturating_sub(1),
    rlim_max: original.rlim_max,
  };

  set_errno(ERRNO_SENTINEL);

  let set_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const lowered) };

  assert_eq!(set_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let get_lowered = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_lowered, 0, "post-set getrlimit must succeed");
  assert_eq!(observed.rlim_cur, lowered.rlim_cur);
  assert_eq!(observed.rlim_max, lowered.rlim_max);

  let restore_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const original) };

  assert_eq!(restore_status, 0, "restoring original limit must succeed");
}

#[test]
fn invalid_resource_sets_einval_for_getrlimit() {
  let _guard = process_wide_rlimit_lock();
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };

  set_errno(0);

  let status = unsafe { getrlimit(INVALID_RESOURCE, &raw mut limits) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn getrlimit_invalid_resource_does_not_overwrite_output() {
  let _guard = process_wide_rlimit_lock();
  let sentinel = RLimit {
    rlim_cur: 1357,
    rlim_max: 2468,
  };
  let mut limits = sentinel;

  set_errno(0);

  let status = unsafe { getrlimit(INVALID_RESOURCE, &raw mut limits) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    limits, sentinel,
    "failed getrlimit with invalid resource must not overwrite output buffer"
  );
}

#[test]
fn getrlimit_success_keeps_errno_set_by_prior_failure() {
  let _guard = process_wide_rlimit_lock();
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };

  set_errno(0);

  let failed_status = unsafe { getrlimit(INVALID_RESOURCE, &raw mut limits) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EINVAL);

  let success_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut limits) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful getrlimit must not clear existing errno"
  );
  assert!(limits.rlim_max >= limits.rlim_cur);
}

#[test]
fn invalid_resource_sets_einval_for_setrlimit() {
  let _guard = process_wide_rlimit_lock();
  let mut current = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut current) };

  assert_eq!(get_status, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let set_status = unsafe { setrlimit(INVALID_RESOURCE, &raw const current) };

  assert_eq!(set_status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setrlimit_invalid_resource_does_not_modify_current_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let requested = if original.rlim_cur > 0 {
    RLimit {
      rlim_cur: original.rlim_cur - 1,
      rlim_max: original.rlim_max,
    }
  } else if original.rlim_max > 0 {
    let tightened = original.rlim_max - 1;

    RLimit {
      rlim_cur: tightened,
      rlim_max: tightened,
    }
  } else {
    panic!("unexpected zero RLIMIT_NOFILE hard and soft limits");
  };

  set_errno(0);

  let set_status = unsafe { setrlimit(INVALID_RESOURCE, &raw const requested) };

  assert_eq!(set_status, -1);
  assert_eq!(read_errno(), EINVAL);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed setrlimit with invalid resource must not modify current process limits"
  );
}

#[test]
fn setrlimit_success_keeps_errno_set_by_prior_failure() {
  let _guard = process_wide_rlimit_lock();
  let mut current = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut current) };

  assert_eq!(get_status, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let failed_status = unsafe { setrlimit(INVALID_RESOURCE, &raw const current) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EINVAL);

  let success_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const current) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful setrlimit must not clear existing errno"
  );
}

#[test]
fn setrlimit_success_keeps_errno_set_by_prior_null_input_failure() {
  let _guard = process_wide_rlimit_lock();
  let mut current = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut current) };

  assert_eq!(get_status, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let failed_status = unsafe { setrlimit(RLIMIT_NOFILE, core::ptr::null()) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const current) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful setrlimit must not clear existing errno"
  );
}

#[test]
fn prlimit64_nonexistent_pid_sets_esrch() {
  let _guard = process_wide_rlimit_lock();
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };

  set_errno(0);

  let status = unsafe {
    prlimit64(
      c_int::MAX,
      RLIMIT_NOFILE,
      core::ptr::null(),
      &raw mut limits,
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), ESRCH);
}

#[test]
fn prlimit64_nonexistent_pid_does_not_overwrite_old_limit() {
  let _guard = process_wide_rlimit_lock();
  let sentinel = RLimit {
    rlim_cur: 4321,
    rlim_max: 8765,
  };
  let mut old_limit = sentinel;

  set_errno(0);

  let status = unsafe {
    prlimit64(
      c_int::MAX,
      RLIMIT_NOFILE,
      core::ptr::null(),
      &raw mut old_limit,
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), ESRCH);
  assert_eq!(
    old_limit, sentinel,
    "failed prlimit64 for nonexistent pid must not overwrite old_limit buffer"
  );
}

#[test]
fn prlimit64_nonexistent_pid_with_new_limit_does_not_overwrite_old_limit() {
  let _guard = process_wide_rlimit_lock();
  let mut current = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut current) };

  assert_eq!(get_status, 0, "precondition getrlimit must succeed");

  let sentinel = RLimit {
    rlim_cur: 2468,
    rlim_max: 9753,
  };
  let mut old_limit = sentinel;
  let requested = RLimit {
    rlim_cur: current.rlim_cur.saturating_sub(1),
    rlim_max: current.rlim_max,
  };

  set_errno(0);

  let status = unsafe {
    prlimit64(
      c_int::MAX,
      RLIMIT_NOFILE,
      &raw const requested,
      &raw mut old_limit,
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), ESRCH);
  assert_eq!(
    old_limit, sentinel,
    "failed prlimit64 for nonexistent pid must not overwrite old_limit even when new_limit is provided"
  );
}

#[test]
fn prlimit64_nonexistent_pid_with_new_limit_does_not_modify_current_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let requested = if original.rlim_cur > 0 {
    RLimit {
      rlim_cur: original.rlim_cur - 1,
      rlim_max: original.rlim_max,
    }
  } else if original.rlim_max > 0 {
    let tightened = original.rlim_max - 1;

    RLimit {
      rlim_cur: tightened,
      rlim_max: tightened,
    }
  } else {
    panic!("unexpected zero RLIMIT_NOFILE hard and soft limits");
  };

  assert_ne!(
    requested, original,
    "precondition expected an alternate valid limit candidate"
  );

  set_errno(0);

  let status = unsafe {
    prlimit64(
      c_int::MAX,
      RLIMIT_NOFILE,
      &raw const requested,
      core::ptr::null_mut(),
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), ESRCH);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed prlimit64 for nonexistent pid must not modify current process limits"
  );
}

#[test]
fn prlimit64_nonexistent_pid_with_null_pointers_sets_esrch() {
  let _guard = process_wide_rlimit_lock();

  set_errno(0);

  let status = unsafe {
    prlimit64(
      c_int::MAX,
      RLIMIT_NOFILE,
      core::ptr::null(),
      core::ptr::null_mut(),
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), ESRCH);
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_failure() {
  let _guard = process_wide_rlimit_lock();

  set_errno(0);

  let failed_status = unsafe {
    prlimit64(
      c_int::MAX,
      RLIMIT_NOFILE,
      core::ptr::null(),
      core::ptr::null_mut(),
    )
  };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), ESRCH);

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    ESRCH,
    "successful prlimit64 must not clear existing errno"
  );
}

#[test]
fn prlimit64_invalid_resource_with_null_pointers_sets_einval() {
  let _guard = process_wide_rlimit_lock();

  set_errno(0);

  let status = unsafe {
    prlimit64(
      0,
      INVALID_RESOURCE,
      core::ptr::null(),
      core::ptr::null_mut(),
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_invalid_resource_failure() {
  let _guard = process_wide_rlimit_lock();

  set_errno(0);

  let failed_status = unsafe {
    prlimit64(
      0,
      INVALID_RESOURCE,
      core::ptr::null(),
      core::ptr::null_mut(),
    )
  };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EINVAL);

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EINVAL,
    "successful prlimit64 must not clear existing errno"
  );
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_efault_failure() {
  let _guard = process_wide_rlimit_lock();
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(1);

  set_errno(0);

  let failed_status = unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), invalid_old_limit) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful prlimit64 must not clear existing errno"
  );
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_low_invalid_old_limit_efault_and_preserves_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(1);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let failed_status = unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), invalid_old_limit) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful prlimit64 must not clear existing errno"
  );

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-success getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed prlimit64 read with low invalid old_limit pointer must not modify current process limits"
  );
}

#[test]
fn prlimit64_high_bit_old_limit_pointer_sets_efault() {
  let _guard = process_wide_rlimit_lock();
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(usize::MAX);

  set_errno(0);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), invalid_old_limit) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn prlimit64_high_bit_old_limit_pointer_sets_efault_and_preserves_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(usize::MAX);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), invalid_old_limit) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed prlimit64 read with high-bit invalid old_limit pointer must not modify current process limits"
  );
}

#[test]
fn prlimit64_low_invalid_old_limit_pointer_sets_efault_and_preserves_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(1);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), invalid_old_limit) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed prlimit64 read with invalid old_limit pointer must not modify current process limits"
  );
}

#[test]
fn prlimit64_high_bit_old_limit_with_new_limit_sets_efault_and_applies_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(usize::MAX);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let requested = RLimit {
    rlim_cur: original.rlim_cur.saturating_sub(1),
    rlim_max: original.rlim_max,
  };

  set_errno(0);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, &raw const requested, invalid_old_limit) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, requested,
    "prlimit64 may apply new limits before failing with EFAULT for invalid old_limit pointer"
  );

  let restore_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const original) };

  assert_eq!(restore_status, 0, "restoring original limit must succeed");
}

#[test]
fn prlimit64_low_invalid_old_limit_with_new_limit_sets_efault_and_applies_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(1);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let requested = RLimit {
    rlim_cur: original.rlim_cur.saturating_sub(1),
    rlim_max: original.rlim_max,
  };

  set_errno(0);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, &raw const requested, invalid_old_limit) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, requested,
    "prlimit64 may apply new limits before failing with EFAULT for invalid old_limit pointer"
  );

  let restore_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const original) };

  assert_eq!(restore_status, 0, "restoring original limit must succeed");
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_low_invalid_old_limit_with_new_limit_efault() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(1);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let requested = RLimit {
    rlim_cur: original.rlim_cur.saturating_sub(1),
    rlim_max: original.rlim_max,
  };

  set_errno(0);

  let failed_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, &raw const requested, invalid_old_limit) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful prlimit64 must not clear existing errno"
  );

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-success getrlimit must succeed");
  assert_eq!(
    observed, requested,
    "prlimit64 may apply new limits before failing with EFAULT for invalid old_limit pointer"
  );

  let restore_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const original) };

  assert_eq!(restore_status, 0, "restoring original limit must succeed");
}

#[test]
fn prlimit64_high_bit_new_limit_pointer_sets_efault_and_preserves_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_new_limit = core::ptr::with_exposed_provenance::<RLimit>(usize::MAX);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, invalid_new_limit, core::ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed prlimit64 with invalid new_limit pointer must not modify current process limits"
  );
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_high_bit_new_limit_efault() {
  let _guard = process_wide_rlimit_lock();
  let invalid_new_limit = core::ptr::with_exposed_provenance::<RLimit>(usize::MAX);

  set_errno(0);

  let failed_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, invalid_new_limit, core::ptr::null_mut()) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful prlimit64 must not clear existing errno"
  );
}

#[test]
fn prlimit64_low_invalid_new_limit_pointer_sets_efault_and_preserves_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_new_limit = core::ptr::with_exposed_provenance::<RLimit>(1);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, invalid_new_limit, core::ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed prlimit64 with low invalid new_limit pointer must not modify current process limits"
  );
}

#[test]
fn prlimit64_low_invalid_new_limit_does_not_overwrite_old_limit_output() {
  let _guard = process_wide_rlimit_lock();
  let invalid_new_limit = core::ptr::with_exposed_provenance::<RLimit>(1);
  let sentinel = RLimit {
    rlim_cur: 1234,
    rlim_max: 5678,
  };
  let mut old_limit = sentinel;

  set_errno(0);

  let status = unsafe { prlimit64(0, RLIMIT_NOFILE, invalid_new_limit, &raw mut old_limit) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
  assert_eq!(
    old_limit, sentinel,
    "failed prlimit64 with low invalid new_limit pointer must not overwrite old_limit output"
  );
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_low_invalid_new_limit_efault() {
  let _guard = process_wide_rlimit_lock();
  let invalid_new_limit = core::ptr::with_exposed_provenance::<RLimit>(1);

  set_errno(0);

  let failed_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, invalid_new_limit, core::ptr::null_mut()) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful prlimit64 must not clear existing errno"
  );
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_high_bit_old_limit_efault() {
  let _guard = process_wide_rlimit_lock();
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(usize::MAX);

  set_errno(0);

  let failed_status = unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), invalid_old_limit) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful prlimit64 must not clear existing errno"
  );
}

#[test]
fn prlimit64_success_keeps_errno_set_by_prior_high_bit_old_limit_with_new_limit_efault() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_old_limit = core::ptr::with_exposed_provenance_mut::<RLimit>(usize::MAX);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let requested = RLimit {
    rlim_cur: original.rlim_cur.saturating_sub(1),
    rlim_max: original.rlim_max,
  };

  set_errno(0);

  let failed_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, &raw const requested, invalid_old_limit) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, requested,
    "prlimit64 may apply new limits before failing with EFAULT for invalid old_limit pointer"
  );

  let success_status =
    unsafe { prlimit64(0, RLIMIT_NOFILE, core::ptr::null(), core::ptr::null_mut()) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful prlimit64 must not clear existing errno"
  );

  let restore_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const original) };

  assert_eq!(restore_status, 0, "restoring original limit must succeed");
}

#[test]
fn prlimit64_invalid_resource_does_not_overwrite_old_limit() {
  let _guard = process_wide_rlimit_lock();
  let sentinel = RLimit {
    rlim_cur: 1234,
    rlim_max: 5678,
  };
  let mut old_limit = sentinel;

  set_errno(0);

  let status = unsafe { prlimit64(0, INVALID_RESOURCE, core::ptr::null(), &raw mut old_limit) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    old_limit, sentinel,
    "failed prlimit64 must not overwrite old_limit buffer"
  );
}

#[test]
fn prlimit64_invalid_resource_with_new_limit_does_not_overwrite_old_limit() {
  let _guard = process_wide_rlimit_lock();
  let mut current = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut current) };

  assert_eq!(get_status, 0, "precondition getrlimit must succeed");

  let sentinel = RLimit {
    rlim_cur: 1111,
    rlim_max: 2222,
  };
  let mut old_limit = sentinel;
  let requested = RLimit {
    rlim_cur: current.rlim_cur.saturating_sub(1),
    rlim_max: current.rlim_max,
  };

  set_errno(0);

  let status = unsafe {
    prlimit64(
      0,
      INVALID_RESOURCE,
      &raw const requested,
      &raw mut old_limit,
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    old_limit, sentinel,
    "failed prlimit64 with invalid resource must not overwrite old_limit even when new_limit is provided"
  );
}

#[test]
fn prlimit64_invalid_resource_with_new_limit_does_not_modify_current_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  let requested = if original.rlim_cur > 0 {
    RLimit {
      rlim_cur: original.rlim_cur - 1,
      rlim_max: original.rlim_max,
    }
  } else if original.rlim_max > 0 {
    let tightened = original.rlim_max - 1;

    RLimit {
      rlim_cur: tightened,
      rlim_max: tightened,
    }
  } else {
    panic!("unexpected zero RLIMIT_NOFILE hard and soft limits");
  };

  assert_ne!(
    requested, original,
    "precondition expected an alternate valid limit candidate"
  );

  set_errno(0);

  let status = unsafe {
    prlimit64(
      0,
      INVALID_RESOURCE,
      &raw const requested,
      core::ptr::null_mut(),
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed prlimit64 for invalid resource must not modify current process limits"
  );
}

#[test]
fn setrlimit_rejects_soft_limit_above_hard_limit() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");
  assert!(
    original.rlim_max > 0,
    "RLIMIT_NOFILE hard limit must be non-zero"
  );

  let invalid = RLimit {
    rlim_cur: original.rlim_max,
    rlim_max: original.rlim_max - 1,
  };

  set_errno(0);

  let set_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const invalid) };

  assert_eq!(set_status, -1);
  assert_eq!(read_errno(), EINVAL);

  let get_after = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_after, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed setrlimit must not change limits"
  );
}

#[test]
fn getrlimit_null_output_sets_efault() {
  let _guard = process_wide_rlimit_lock();

  set_errno(0);

  let status = unsafe { getrlimit(RLIMIT_NOFILE, core::ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn getrlimit_success_keeps_errno_set_by_prior_null_output_failure() {
  let _guard = process_wide_rlimit_lock();
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };

  set_errno(0);

  let failed_status = unsafe { getrlimit(RLIMIT_NOFILE, core::ptr::null_mut()) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut limits) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful getrlimit must not clear existing errno"
  );
  assert!(limits.rlim_max >= limits.rlim_cur);
}

#[test]
fn getrlimit_null_output_prioritizes_efault_over_invalid_resource() {
  let _guard = process_wide_rlimit_lock();

  set_errno(0);

  let status = unsafe { getrlimit(INVALID_RESOURCE, core::ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn getrlimit_high_bit_output_sets_efault() {
  let _guard = process_wide_rlimit_lock();
  let invalid_output = core::ptr::with_exposed_provenance_mut::<RLimit>(usize::MAX);

  set_errno(0);

  let status = unsafe { getrlimit(RLIMIT_NOFILE, invalid_output) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn getrlimit_success_keeps_errno_set_by_prior_high_bit_output_efault() {
  let _guard = process_wide_rlimit_lock();
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_output = core::ptr::with_exposed_provenance_mut::<RLimit>(usize::MAX);

  set_errno(0);

  let failed_status = unsafe { getrlimit(RLIMIT_NOFILE, invalid_output) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut limits) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful getrlimit must not clear existing errno"
  );
  assert!(limits.rlim_max >= limits.rlim_cur);
}

#[test]
fn getrlimit_low_invalid_output_sets_efault() {
  let _guard = process_wide_rlimit_lock();
  let invalid_output = core::ptr::with_exposed_provenance_mut::<RLimit>(1);

  set_errno(0);

  let status = unsafe { getrlimit(RLIMIT_NOFILE, invalid_output) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn getrlimit_success_keeps_errno_set_by_prior_low_invalid_output_efault() {
  let _guard = process_wide_rlimit_lock();
  let mut limits = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_output = core::ptr::with_exposed_provenance_mut::<RLimit>(1);

  set_errno(0);

  let failed_status = unsafe { getrlimit(RLIMIT_NOFILE, invalid_output) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut limits) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful getrlimit must not clear existing errno"
  );
  assert!(limits.rlim_max >= limits.rlim_cur);
}

#[test]
fn setrlimit_null_input_sets_efault() {
  let _guard = process_wide_rlimit_lock();

  set_errno(0);

  let status = unsafe { setrlimit(RLIMIT_NOFILE, core::ptr::null()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn setrlimit_high_bit_input_sets_efault_and_preserves_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_limit = core::ptr::with_exposed_provenance::<RLimit>(usize::MAX);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let set_status = unsafe { setrlimit(RLIMIT_NOFILE, invalid_limit) };

  assert_eq!(set_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed setrlimit with invalid high-bit input pointer must not modify current process limits"
  );
}

#[test]
fn setrlimit_low_invalid_input_sets_efault_and_preserves_limits() {
  let _guard = process_wide_rlimit_lock();
  let mut original = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let mut observed = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_limit = core::ptr::with_exposed_provenance::<RLimit>(1);
  let get_original = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut original) };

  assert_eq!(get_original, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let set_status = unsafe { setrlimit(RLIMIT_NOFILE, invalid_limit) };

  assert_eq!(set_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let get_observed = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut observed) };

  assert_eq!(get_observed, 0, "post-failure getrlimit must succeed");
  assert_eq!(
    observed, original,
    "failed setrlimit with low invalid input pointer must not modify current process limits"
  );
}

#[test]
fn setrlimit_success_keeps_errno_set_by_prior_low_invalid_input_efault() {
  let _guard = process_wide_rlimit_lock();
  let mut current = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_limit = core::ptr::with_exposed_provenance::<RLimit>(1);
  let get_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut current) };

  assert_eq!(get_status, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let failed_status = unsafe { setrlimit(RLIMIT_NOFILE, invalid_limit) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const current) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful setrlimit must not clear existing errno"
  );
}

#[test]
fn setrlimit_success_keeps_errno_set_by_prior_high_bit_input_efault() {
  let _guard = process_wide_rlimit_lock();
  let mut current = RLimit {
    rlim_cur: 0,
    rlim_max: 0,
  };
  let invalid_limit = core::ptr::with_exposed_provenance::<RLimit>(usize::MAX);
  let get_status = unsafe { getrlimit(RLIMIT_NOFILE, &raw mut current) };

  assert_eq!(get_status, 0, "precondition getrlimit must succeed");

  set_errno(0);

  let failed_status = unsafe { setrlimit(RLIMIT_NOFILE, invalid_limit) };

  assert_eq!(failed_status, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_status = unsafe { setrlimit(RLIMIT_NOFILE, &raw const current) };

  assert_eq!(success_status, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful setrlimit must not clear existing errno"
  );
}

#[test]
fn setrlimit_null_input_prioritizes_efault_over_invalid_resource() {
  let _guard = process_wide_rlimit_lock();

  set_errno(0);

  let status = unsafe { setrlimit(INVALID_RESOURCE, core::ptr::null()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
}
