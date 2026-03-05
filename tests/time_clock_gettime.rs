use core::ptr;
use rlibc::abi::errno::{EFAULT, EINVAL};
use rlibc::abi::types::c_int;
use rlibc::errno::__errno_location;
use rlibc::time::{
  CLOCK_BOOTTIME, CLOCK_BOOTTIME_ALARM, CLOCK_MONOTONIC, CLOCK_MONOTONIC_COARSE,
  CLOCK_MONOTONIC_RAW, CLOCK_PROCESS_CPUTIME_ID, CLOCK_REALTIME, CLOCK_REALTIME_ALARM,
  CLOCK_REALTIME_COARSE, CLOCK_SGI_CYCLE, CLOCK_TAI, CLOCK_THREAD_CPUTIME_ID, CLOCKFD,
  clock_gettime, clockid_t, clockid_to_fd, fd_to_clockid, timespec,
};

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` must return a valid pointer for the current thread.
  unsafe { *__errno_location() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` must return a valid writable pointer for the current thread.
  unsafe {
    *__errno_location() = value;
  }
}

#[test]
fn clock_id_constants_match_linux_values() {
  assert_eq!(CLOCK_REALTIME, 0);
  assert_eq!(CLOCK_MONOTONIC, 1);
  assert_eq!(CLOCK_PROCESS_CPUTIME_ID, 2);
  assert_eq!(CLOCK_THREAD_CPUTIME_ID, 3);
  assert_eq!(CLOCK_MONOTONIC_RAW, 4);
  assert_eq!(CLOCK_REALTIME_COARSE, 5);
  assert_eq!(CLOCK_MONOTONIC_COARSE, 6);
  assert_eq!(CLOCK_BOOTTIME, 7);
  assert_eq!(CLOCK_REALTIME_ALARM, 8);
  assert_eq!(CLOCK_BOOTTIME_ALARM, 9);
  assert_eq!(CLOCK_SGI_CYCLE, 10);
  assert_eq!(CLOCK_TAI, 11);
}

#[test]
fn dynamic_clockid_helpers_roundtrip_non_negative_fd_values() {
  assert_eq!(CLOCKFD, 3);

  let fds: [c_int; 6] = [0, 1, 2, 42, 256, 1_024];

  for fd in fds {
    let dynamic_clock_id = fd_to_clockid(fd);

    assert_eq!(dynamic_clock_id & 0b111, CLOCKFD);
    assert_eq!(clockid_to_fd(dynamic_clock_id), fd);
  }
}

#[test]
fn dynamic_clockid_helpers_zero_fd_is_tagged_dynamic_clock_id() {
  let zero_fd: c_int = 0;
  let dynamic_clock_id = fd_to_clockid(zero_fd);

  assert_eq!(dynamic_clock_id & 0b111, CLOCKFD);
  assert_eq!(clockid_to_fd(dynamic_clock_id), zero_fd);
}

#[test]
fn dynamic_clockid_helpers_preserve_clockfd_tag_and_expected_roundtrip_bits() {
  let known_fd: c_int = 5;
  let expected_dynamic_clock_id: clockid_t = -45;
  let dynamic_clock_id = fd_to_clockid(known_fd);

  assert_eq!(dynamic_clock_id, expected_dynamic_clock_id);
  assert_eq!(dynamic_clock_id & 0b111, CLOCKFD);
  assert_eq!(clockid_to_fd(dynamic_clock_id), known_fd);
}

#[test]
fn dynamic_clockid_helpers_roundtrip_upper_lossless_fd_boundary() {
  let fd_at_lossless_boundary: c_int = 0x0FFF_FFFF;
  let dynamic_clock_id = fd_to_clockid(fd_at_lossless_boundary);

  assert_eq!(dynamic_clock_id & 0b111, CLOCKFD);
  assert_eq!(clockid_to_fd(dynamic_clock_id), fd_at_lossless_boundary);
}

#[test]
fn dynamic_clockid_helpers_large_fd_can_lose_high_bits_by_design() {
  let fd_above_lossless_boundary: c_int = 0x1000_0000;
  let dynamic_clock_id = fd_to_clockid(fd_above_lossless_boundary);

  assert_eq!(dynamic_clock_id & 0b111, CLOCKFD);
  assert_ne!(clockid_to_fd(dynamic_clock_id), fd_above_lossless_boundary);
}

#[test]
fn dynamic_clockid_helpers_max_fd_aliases_to_negative_one_after_encoding() {
  let max_fd = c_int::MAX;
  let dynamic_clock_id = fd_to_clockid(max_fd);

  assert_eq!(dynamic_clock_id & 0b111, CLOCKFD);
  assert_eq!(clockid_to_fd(dynamic_clock_id), -1);
  assert_ne!(clockid_to_fd(dynamic_clock_id), max_fd);
}

#[test]
fn dynamic_clockid_helpers_min_fd_aliases_to_zero_fd_after_encoding() {
  let min_fd = c_int::MIN;
  let min_dynamic_clock_id = fd_to_clockid(min_fd);
  let zero_dynamic_clock_id = fd_to_clockid(0);

  assert_eq!(min_dynamic_clock_id, zero_dynamic_clock_id);
  assert_eq!(min_dynamic_clock_id & 0b111, CLOCKFD);
  assert_eq!(clockid_to_fd(min_dynamic_clock_id), 0);
  assert_ne!(clockid_to_fd(min_dynamic_clock_id), min_fd);
}

#[test]
fn dynamic_clockid_helpers_negative_fd_can_alias_static_clock_id_by_contract() {
  let negative_fd: c_int = -1;
  let dynamic_clock_id = fd_to_clockid(negative_fd);

  assert_eq!(dynamic_clock_id, CLOCKFD);
  assert_eq!(dynamic_clock_id, CLOCK_THREAD_CPUTIME_ID);
  assert_eq!(clockid_to_fd(dynamic_clock_id), negative_fd);
}

#[test]
fn dynamic_clockid_alias_for_negative_fd_follows_thread_cputime_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(63);

  let result = clock_gettime(fd_to_clockid(-1), &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 63);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn dynamic_clockid_helpers_negative_fd_minus_two_can_alias_tai_by_contract() {
  let negative_fd: c_int = -2;
  let dynamic_clock_id = fd_to_clockid(negative_fd);

  assert_eq!(dynamic_clock_id, CLOCK_TAI);
  assert_eq!(clockid_to_fd(dynamic_clock_id), negative_fd);
}

#[test]
fn dynamic_clockid_alias_for_negative_fd_minus_two_follows_tai_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(65);

  let result = clock_gettime(fd_to_clockid(-2), &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 65);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn dynamic_clockid_alias_for_negative_fd_minus_two_matches_static_tai_result_class() {
  let mut alias_ts = timespec {
    tv_sec: 111,
    tv_nsec: 222,
  };
  let alias_before = alias_ts;
  let mut static_ts = timespec {
    tv_sec: 333,
    tv_nsec: 444,
  };
  let static_before = static_ts;

  write_errno(65);

  let alias_result = clock_gettime(fd_to_clockid(-2), &raw mut alias_ts);
  let alias_errno = read_errno();

  write_errno(66);

  let static_result = clock_gettime(CLOCK_TAI, &raw mut static_ts);
  let static_errno = read_errno();

  assert_eq!(alias_result, static_result);

  if alias_result == 0 {
    assert!(alias_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&alias_ts.tv_nsec));
    assert_eq!(alias_errno, 65);

    assert!(static_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&static_ts.tv_nsec));
    assert_eq!(static_errno, 66);

    return;
  }

  assert_eq!(alias_result, -1);
  assert_eq!(alias_errno, EINVAL);
  assert_eq!(alias_ts, alias_before);
  assert_eq!(static_errno, EINVAL);
  assert_eq!(static_ts, static_before);
}

#[test]
fn dynamic_clockid_alias_for_negative_fd_matches_static_thread_cputime_result_class() {
  let mut alias_ts = timespec {
    tv_sec: 211,
    tv_nsec: 322,
  };
  let alias_before = alias_ts;
  let mut static_ts = timespec {
    tv_sec: 433,
    tv_nsec: 544,
  };
  let static_before = static_ts;

  write_errno(61);
  let alias_result = clock_gettime(fd_to_clockid(-1), &raw mut alias_ts);
  let alias_errno = read_errno();

  write_errno(62);
  let static_result = clock_gettime(CLOCK_THREAD_CPUTIME_ID, &raw mut static_ts);
  let static_errno = read_errno();

  assert_eq!(alias_result, static_result);

  if alias_result == 0 {
    assert!(alias_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&alias_ts.tv_nsec));
    assert_eq!(alias_errno, 61);

    assert!(static_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&static_ts.tv_nsec));
    assert_eq!(static_errno, 62);

    return;
  }

  assert_eq!(alias_result, -1);
  assert_eq!(alias_errno, EINVAL);
  assert_eq!(alias_ts, alias_before);
  assert_eq!(static_errno, EINVAL);
  assert_eq!(static_ts, static_before);
}

#[test]
fn dynamic_clockid_for_zero_fd_follows_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 555,
    tv_nsec: 666,
  };
  let before = ts;

  write_errno(71);

  let result = clock_gettime(fd_to_clockid(0), &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 71);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(ts, before);
}

#[test]
fn dynamic_clockid_alias_for_min_fd_matches_zero_fd_result_class() {
  let mut min_ts = timespec {
    tv_sec: 101,
    tv_nsec: 202,
  };
  let min_before = min_ts;
  let mut zero_ts = timespec {
    tv_sec: 303,
    tv_nsec: 404,
  };
  let zero_before = zero_ts;

  write_errno(73);

  let min_result = clock_gettime(fd_to_clockid(c_int::MIN), &raw mut min_ts);
  let min_errno = read_errno();

  write_errno(74);

  let zero_result = clock_gettime(fd_to_clockid(0), &raw mut zero_ts);
  let zero_errno = read_errno();

  assert_eq!(fd_to_clockid(c_int::MIN), fd_to_clockid(0));
  assert_eq!(min_result, zero_result);

  if min_result == 0 {
    assert!(min_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&min_ts.tv_nsec));
    assert_eq!(min_errno, 73);

    assert!(zero_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&zero_ts.tv_nsec));
    assert_eq!(zero_errno, 74);

    return;
  }

  assert_eq!(min_result, -1);
  assert_eq!(min_errno, zero_errno);
  assert_eq!(min_ts, min_before);
  assert_eq!(zero_ts, zero_before);
}

#[test]
fn dynamic_clockid_alias_for_min_fd_after_null_timespec_matches_zero_fd_errno_contract() {
  let mut min_ts = timespec {
    tv_sec: 505,
    tv_nsec: 606,
  };
  let min_before = min_ts;
  let mut zero_ts = timespec {
    tv_sec: 707,
    tv_nsec: 808,
  };
  let zero_before = zero_ts;

  write_errno(EFAULT);
  let min_result = clock_gettime(fd_to_clockid(c_int::MIN), &raw mut min_ts);
  let min_errno = read_errno();

  write_errno(EFAULT);
  let zero_result = clock_gettime(fd_to_clockid(0), &raw mut zero_ts);
  let zero_errno = read_errno();

  assert_eq!(fd_to_clockid(c_int::MIN), fd_to_clockid(0));
  assert_eq!(min_result, zero_result);

  if min_result == 0 {
    assert!(min_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&min_ts.tv_nsec));
    assert_eq!(min_errno, EFAULT);

    assert!(zero_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&zero_ts.tv_nsec));
    assert_eq!(zero_errno, EFAULT);

    return;
  }

  assert_eq!(min_result, -1);
  assert_eq!(min_errno, EINVAL);
  assert_eq!(zero_errno, EINVAL);
  assert_eq!(min_ts, min_before);
  assert_eq!(zero_ts, zero_before);
}

#[test]
fn min_fd_and_zero_fd_alias_after_null_then_invalid_clock_overwrite_errno_with_einval() {
  let dynamic_clock_ids: [clockid_t; 2] = [fd_to_clockid(c_int::MIN), fd_to_clockid(0)];

  assert_eq!(dynamic_clock_ids[0], dynamic_clock_ids[1]);

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 910 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 920 + i64::try_from(index).unwrap_or(0),
    };
    let before = invalid_ts;

    write_errno(EINVAL);

    let null_result = clock_gettime(*dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let invalid_result = clock_gettime(9_999, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(invalid_ts, before);
  }
}

#[test]
fn min_fd_and_zero_fd_alias_after_null_share_non_null_result_class_and_errno_contract() {
  let mut min_ts = timespec {
    tv_sec: 1_111,
    tv_nsec: 2_222,
  };
  let min_before = min_ts;
  let mut zero_ts = timespec {
    tv_sec: 3_333,
    tv_nsec: 4_444,
  };
  let zero_before = zero_ts;

  let min_dynamic_clock_id = fd_to_clockid(c_int::MIN);
  let zero_dynamic_clock_id = fd_to_clockid(0);

  assert_eq!(min_dynamic_clock_id, zero_dynamic_clock_id);

  write_errno(EINVAL);
  let min_null_result = clock_gettime(min_dynamic_clock_id, ptr::null_mut());
  assert_eq!(min_null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(EINVAL);
  let zero_null_result = clock_gettime(zero_dynamic_clock_id, ptr::null_mut());
  assert_eq!(zero_null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(EFAULT);
  let min_result = clock_gettime(min_dynamic_clock_id, &raw mut min_ts);
  let min_errno = read_errno();

  write_errno(EFAULT);
  let zero_result = clock_gettime(zero_dynamic_clock_id, &raw mut zero_ts);
  let zero_errno = read_errno();

  assert_eq!(min_result, zero_result);

  if min_result == 0 {
    assert!(min_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&min_ts.tv_nsec));
    assert_eq!(min_errno, EFAULT);

    assert!(zero_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&zero_ts.tv_nsec));
    assert_eq!(zero_errno, EFAULT);

    return;
  }

  assert_eq!(min_result, -1);
  assert_eq!(min_errno, EINVAL);
  assert_eq!(zero_errno, EINVAL);
  assert_eq!(min_ts, min_before);
  assert_eq!(zero_ts, zero_before);
}

#[test]
fn dynamic_clockid_alias_for_max_fd_follows_thread_cputime_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(64);

  let result = clock_gettime(fd_to_clockid(c_int::MAX), &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 64);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn dynamic_clockid_alias_set_preserves_output_on_failure() {
  let dynamic_clock_ids: [clockid_t; 2] = [fd_to_clockid(-1), fd_to_clockid(c_int::MAX)];

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut ts = timespec {
      tv_sec: 8_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 9_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = ts;
    let sentinel = 70 + c_int::try_from(index).unwrap_or(0);

    write_errno(sentinel);

    let result = clock_gettime(*dynamic_clock_id, &raw mut ts);

    if result == 0 {
      assert!(ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&ts.tv_nsec));
      assert_eq!(read_errno(), sentinel);

      continue;
    }

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts, before);
  }
}

#[test]
fn dynamic_clockid_alias_set_after_invalid_clock_id_keeps_errno_einval() {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 1_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 2_000 + i64::try_from(index).unwrap_or(0),
    };
    let mut dynamic_ts = timespec {
      tv_sec: 3_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 4_000 + i64::try_from(index).unwrap_or(0),
    };
    let dynamic_before = dynamic_ts;

    write_errno(0);

    let invalid_result = clock_gettime(9_999, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);

    let dynamic_result = clock_gettime(*dynamic_clock_id, &raw mut dynamic_ts);

    if dynamic_result == 0 {
      assert!(dynamic_ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&dynamic_ts.tv_nsec));
      assert_eq!(read_errno(), EINVAL);
      continue;
    }

    assert_eq!(dynamic_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(dynamic_ts, dynamic_before);
  }
}

#[test]
fn dynamic_clockid_alias_min_and_zero_after_invalid_clock_id_keep_errno_einval() {
  let dynamic_clock_ids: [clockid_t; 2] = [fd_to_clockid(c_int::MIN), fd_to_clockid(0)];

  assert_eq!(dynamic_clock_ids[0], dynamic_clock_ids[1]);

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 10_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 11_000 + i64::try_from(index).unwrap_or(0),
    };
    let mut dynamic_ts = timespec {
      tv_sec: 12_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 13_000 + i64::try_from(index).unwrap_or(0),
    };
    let dynamic_before = dynamic_ts;

    write_errno(0);

    let invalid_result = clock_gettime(9_999, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);

    let dynamic_result = clock_gettime(*dynamic_clock_id, &raw mut dynamic_ts);

    if dynamic_result == 0 {
      assert!(dynamic_ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&dynamic_ts.tv_nsec));
      assert_eq!(read_errno(), EINVAL);
      continue;
    }

    assert_eq!(dynamic_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(dynamic_ts, dynamic_before);
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_follow_kernel_errno_contract() {
  let clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (index, clock_id) in clock_ids.iter().enumerate() {
    let mut ts = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };
    let sentinel = 200 + c_int::try_from(index).unwrap_or(0);

    write_errno(sentinel);

    let result = clock_gettime(*clock_id, &raw mut ts);

    if result == 0 {
      assert!(ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&ts.tv_nsec));
      assert_eq!(read_errno(), sentinel);
      continue;
    }

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EINVAL);
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_failure_path_preserves_output() {
  let clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (index, clock_id) in clock_ids.iter().enumerate() {
    let mut ts = timespec {
      tv_sec: 1_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 2_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = ts;
    let sentinel = 500 + c_int::try_from(index).unwrap_or(0);

    write_errno(sentinel);

    let result = clock_gettime(*clock_id, &raw mut ts);

    if result == 0 {
      assert!(ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&ts.tv_nsec));
      assert_eq!(read_errno(), sentinel);
      continue;
    }

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts, before);
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_with_null_timespec_set_efault() {
  let clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (index, clock_id) in clock_ids.iter().enumerate() {
    let sentinel = 300 + c_int::try_from(index).unwrap_or(0);

    write_errno(sentinel);

    let result = clock_gettime(*clock_id, ptr::null_mut());

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_with_null_timespec_overwrite_existing_errno_with_efault() {
  let clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for clock_id in clock_ids {
    write_errno(EINVAL);

    let result = clock_gettime(clock_id, ptr::null_mut());

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_null_timespec_after_prior_call_sets_efault() {
  let clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (index, clock_id) in clock_ids.iter().enumerate() {
    let mut ts = timespec {
      tv_sec: 8_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 9_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = ts;

    write_errno(0);

    let prior_result = clock_gettime(*clock_id, &raw mut ts);

    if prior_result == 0 {
      assert!(ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&ts.tv_nsec));
      assert_eq!(read_errno(), 0);
    } else {
      assert_eq!(prior_result, -1);
      assert_eq!(read_errno(), EINVAL);
      assert_eq!(ts, before);
    }

    let null_result = clock_gettime(*clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_invalid_clock_id_set_with_null_timespec_still_sets_efault() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for (index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let sentinel = 400 + c_int::try_from(index).unwrap_or(0);

    write_errno(sentinel);

    let result = clock_gettime(*invalid_clock_id, ptr::null_mut());

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_invalid_clock_id_set_with_null_timespec_overwrites_existing_errno_with_efault() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for invalid_clock_id in invalid_clock_ids {
    write_errno(EINVAL);

    let result = clock_gettime(invalid_clock_id, ptr::null_mut());

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_invalid_clock_id_set_after_null_timespec_overwrites_errno_with_einval() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for (index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 700 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 800 + i64::try_from(index).unwrap_or(0),
    };
    let before = invalid_ts;

    write_errno(0);

    let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let invalid_result = clock_gettime(*invalid_clock_id, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(invalid_ts, before);
  }
}

#[test]
fn clock_gettime_invalid_clock_id_set_after_invalid_clock_id_set_with_null_timespec_overwrites_errno_with_einval()
 {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for (index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 1_500 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 2_500 + i64::try_from(index).unwrap_or(0),
    };
    let before = invalid_ts;

    write_errno(0);

    let null_invalid_result = clock_gettime(*invalid_clock_id, ptr::null_mut());

    assert_eq!(null_invalid_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let invalid_result = clock_gettime(*invalid_clock_id, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(invalid_ts, before);
  }
}

#[test]
fn clock_gettime_monotonic_success_after_invalid_clock_id_set_with_null_timespec_keeps_errno_efault()
 {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for (index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let mut valid_ts = timespec {
      tv_sec: i64::try_from(index).unwrap_or(0),
      tv_nsec: 0,
    };

    write_errno(0);

    let invalid_result = clock_gettime(*invalid_clock_id, ptr::null_mut());

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let valid_result = clock_gettime(CLOCK_MONOTONIC, &raw mut valid_ts);

    assert_eq!(valid_result, 0);
    assert!(valid_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&valid_ts.tv_nsec));
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_realtime_success_after_invalid_clock_id_set_with_null_timespec_keeps_errno_efault()
{
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for (index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let mut valid_ts = timespec {
      tv_sec: 300 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 1,
    };

    write_errno(0);

    let invalid_result = clock_gettime(*invalid_clock_id, ptr::null_mut());

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let valid_result = clock_gettime(CLOCK_REALTIME, &raw mut valid_ts);

    assert_eq!(valid_result, 0);
    assert!(valid_ts.tv_sec > 0);
    assert!((0..1_000_000_000).contains(&valid_ts.tv_nsec));
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_after_invalid_clock_id_set_with_null_timespec_follow_kernel_errno_contract()
 {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];
  let valid_clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (invalid_index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    write_errno(0);

    let null_invalid_result = clock_gettime(*invalid_clock_id, ptr::null_mut());

    assert_eq!(null_invalid_result, -1);
    assert_eq!(read_errno(), EFAULT);

    for (valid_index, valid_clock_id) in valid_clock_ids.iter().enumerate() {
      let mut ts = timespec {
        tv_sec: 6_000
          + i64::try_from(invalid_index * valid_clock_ids.len() + valid_index).unwrap_or(0),
        tv_nsec: 7_000 + i64::try_from(valid_index).unwrap_or(0),
      };
      let before = ts;
      let errno_before_call = read_errno();
      let result = clock_gettime(*valid_clock_id, &raw mut ts);

      if result == 0 {
        assert!(ts.tv_sec >= 0);
        assert!((0..1_000_000_000).contains(&ts.tv_nsec));
        assert_eq!(read_errno(), errno_before_call);
        continue;
      }

      assert_eq!(result, -1);
      assert_eq!(read_errno(), EINVAL);
      assert_eq!(ts, before);
    }
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_after_null_timespec_follow_kernel_errno_contract() {
  let clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (index, clock_id) in clock_ids.iter().enumerate() {
    let mut ts = timespec {
      tv_sec: i64::try_from(index).unwrap_or(0),
      tv_nsec: 0,
    };

    write_errno(0);

    let null_result = clock_gettime(*clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let result = clock_gettime(*clock_id, &raw mut ts);

    if result == 0 {
      assert!(ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&ts.tv_nsec));
      assert_eq!(read_errno(), EFAULT);
      continue;
    }

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EINVAL);
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_after_null_timespec_preserve_output_on_failure() {
  let clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (index, clock_id) in clock_ids.iter().enumerate() {
    let mut ts = timespec {
      tv_sec: 4_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 5_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = ts;

    write_errno(0);

    let null_result = clock_gettime(*clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let result = clock_gettime(*clock_id, &raw mut ts);

    if result == 0 {
      assert!(ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&ts.tv_nsec));
      assert_eq!(read_errno(), EFAULT);
      continue;
    }

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts, before);
  }
}

#[test]
fn clock_gettime_realtime_writes_timestamp_and_preserves_errno() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(73);

  let result = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(result, 0);
  assert!(ts.tv_sec > 0);
  assert!((0..1_000_000_000).contains(&ts.tv_nsec));
  assert_eq!(read_errno(), 73);
}

#[test]
fn clock_gettime_monotonic_clock_id_is_forwarded_to_kernel() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(91);

  let result = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

  assert_eq!(result, 0);
  assert!(ts.tv_sec >= 0);
  assert!((0..1_000_000_000).contains(&ts.tv_nsec));
  assert_eq!(read_errno(), 91);
}

#[test]
fn clock_gettime_monotonic_is_non_decreasing_across_calls() {
  let mut first = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };
  let mut second = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(44);

  assert_eq!(clock_gettime(CLOCK_MONOTONIC, &raw mut first), 0);
  assert_eq!(clock_gettime(CLOCK_MONOTONIC, &raw mut second), 0);
  assert!((0..1_000_000_000).contains(&first.tv_nsec));
  assert!((0..1_000_000_000).contains(&second.tv_nsec));

  let first_ns = i128::from(first.tv_sec) * 1_000_000_000 + i128::from(first.tv_nsec);
  let second_ns = i128::from(second.tv_sec) * 1_000_000_000 + i128::from(second.tv_nsec);

  assert!(second_ns >= first_ns);
  assert_eq!(read_errno(), 44);
}

#[test]
fn clock_gettime_boottime_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(55);

  let result = clock_gettime(CLOCK_BOOTTIME, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 55);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_realtime_coarse_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(66);

  let result = clock_gettime(CLOCK_REALTIME_COARSE, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 66);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_monotonic_raw_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(67);

  let result = clock_gettime(CLOCK_MONOTONIC_RAW, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 67);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_monotonic_coarse_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(68);

  let result = clock_gettime(CLOCK_MONOTONIC_COARSE, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 68);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_tai_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(69);

  let result = clock_gettime(CLOCK_TAI, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 69);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_realtime_alarm_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(70);

  let result = clock_gettime(CLOCK_REALTIME_ALARM, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 70);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_boottime_alarm_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(71);

  let result = clock_gettime(CLOCK_BOOTTIME_ALARM, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 71);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_process_cputime_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(72);

  let result = clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 72);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_thread_cputime_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(74);

  let result = clock_gettime(CLOCK_THREAD_CPUTIME_ID, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 74);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_sgi_cycle_reports_support_via_kernel_errno_contract() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(75);

  let result = clock_gettime(CLOCK_SGI_CYCLE, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), 75);

    return;
  }

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_invalid_clock_id_returns_minus_one_and_sets_errno() {
  let mut ts = timespec {
    tv_sec: 123,
    tv_nsec: 456,
  };
  let before = ts;
  let invalid_clock_id: clockid_t = -1;

  write_errno(0);

  let result = clock_gettime(invalid_clock_id, &raw mut ts);

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(ts, before);
}

#[test]
fn clock_gettime_large_positive_invalid_clock_id_returns_einval_without_clobbering_output() {
  let mut ts = timespec {
    tv_sec: 777,
    tv_nsec: 888,
  };
  let before = ts;
  let invalid_clock_id: clockid_t = 9_999;

  write_errno(0);

  let result = clock_gettime(invalid_clock_id, &raw mut ts);

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(ts, before);
}

#[test]
fn clock_gettime_max_positive_invalid_clock_id_returns_einval_without_clobbering_output() {
  let mut ts = timespec {
    tv_sec: 901,
    tv_nsec: 234,
  };
  let before = ts;
  let invalid_clock_id: clockid_t = c_int::MAX;

  write_errno(0);

  let result = clock_gettime(invalid_clock_id, &raw mut ts);

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(ts, before);
}

#[test]
fn clock_gettime_invalid_clock_id_set_preserves_output_and_sets_einval() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for (index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let mut ts = timespec {
      tv_sec: 500 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 600 + i64::try_from(index).unwrap_or(0),
    };
    let before = ts;

    write_errno(EFAULT);

    let result = clock_gettime(*invalid_clock_id, &raw mut ts);

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts, before);
  }
}

#[test]
fn clock_gettime_invalid_clock_id_set_after_realtime_success_preserves_latest_output() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for invalid_clock_id in invalid_clock_ids {
    let mut ts = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    write_errno(0);

    let success_result = clock_gettime(CLOCK_REALTIME, &raw mut ts);

    assert_eq!(success_result, 0);
    assert!(ts.tv_sec > 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));

    let before = ts;

    write_errno(EFAULT);

    let invalid_result = clock_gettime(invalid_clock_id, &raw mut ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts, before);
  }
}

#[test]
fn clock_gettime_repeated_invalid_clock_id_set_after_realtime_success_never_clobbers_output() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(0);

  let success_result = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(success_result, 0);
  assert!(ts.tv_sec > 0);
  assert!((0..1_000_000_000).contains(&ts.tv_nsec));

  let baseline = ts;

  for invalid_clock_id in invalid_clock_ids {
    write_errno(EFAULT);

    let invalid_result = clock_gettime(invalid_clock_id, &raw mut ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts, baseline);
  }
}

#[test]
fn clock_gettime_invalid_clock_id_set_after_monotonic_success_preserves_latest_output() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for invalid_clock_id in invalid_clock_ids {
    let mut ts = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    write_errno(0);

    let success_result = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

    assert_eq!(success_result, 0);
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));

    let before = ts;

    write_errno(EFAULT);

    let invalid_result = clock_gettime(invalid_clock_id, &raw mut ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(ts, before);
  }
}

#[test]
fn clock_gettime_max_positive_invalid_clock_id_overwrites_existing_errno_with_einval() {
  let mut ts = timespec {
    tv_sec: 101,
    tv_nsec: 202,
  };
  let before = ts;

  write_errno(EFAULT);

  let invalid_clock_id: clockid_t = c_int::MAX;
  let result = clock_gettime(invalid_clock_id, &raw mut ts);

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(ts, before);
}

#[test]
fn clock_gettime_extreme_negative_invalid_clock_id_overwrites_existing_errno_with_einval() {
  let mut ts = timespec {
    tv_sec: 102,
    tv_nsec: 203,
  };
  let before = ts;

  write_errno(EFAULT);

  let invalid_clock_id: clockid_t = c_int::MIN;
  let result = clock_gettime(invalid_clock_id, &raw mut ts);

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(ts, before);
}

#[test]
fn clock_gettime_extreme_negative_invalid_clock_id_returns_einval_without_clobbering_output() {
  let mut ts = timespec {
    tv_sec: 999,
    tv_nsec: 111,
  };
  let before = ts;
  let invalid_clock_id: clockid_t = c_int::MIN;

  write_errno(0);

  let result = clock_gettime(invalid_clock_id, &raw mut ts);

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(ts, before);
}

#[test]
fn clock_gettime_invalid_clock_id_overwrites_existing_errno_with_einval() {
  let mut ts = timespec {
    tv_sec: 10,
    tv_nsec: 20,
  };
  let before = ts;

  write_errno(123);

  let result = clock_gettime(-1, &raw mut ts);

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(ts, before);
}

#[test]
fn clock_gettime_success_after_invalid_clock_id_keeps_errno_einval() {
  let mut invalid_ts = timespec {
    tv_sec: 11,
    tv_nsec: 22,
  };
  let mut valid_ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(0);

  let invalid_result = clock_gettime(-1, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let valid_result = clock_gettime(CLOCK_REALTIME, &raw mut valid_ts);

  assert_eq!(valid_result, 0);
  assert!(valid_ts.tv_sec > 0);
  assert!((0..1_000_000_000).contains(&valid_ts.tv_nsec));
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_success_after_large_positive_invalid_clock_id_keeps_errno_einval() {
  let mut invalid_ts = timespec {
    tv_sec: 12,
    tv_nsec: 34,
  };
  let mut valid_ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(0);

  let invalid_clock_id: clockid_t = 9_999;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let valid_result = clock_gettime(CLOCK_REALTIME, &raw mut valid_ts);

  assert_eq!(valid_result, 0);
  assert!(valid_ts.tv_sec > 0);
  assert!((0..1_000_000_000).contains(&valid_ts.tv_nsec));
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_success_after_extreme_negative_invalid_clock_id_keeps_errno_einval() {
  let mut invalid_ts = timespec {
    tv_sec: 56,
    tv_nsec: 78,
  };
  let mut valid_ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(0);

  let invalid_clock_id: clockid_t = c_int::MIN;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let valid_result = clock_gettime(CLOCK_REALTIME, &raw mut valid_ts);

  assert_eq!(valid_result, 0);
  assert!(valid_ts.tv_sec > 0);
  assert!((0..1_000_000_000).contains(&valid_ts.tv_nsec));
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_success_after_max_positive_invalid_clock_id_keeps_errno_einval() {
  let mut invalid_ts = timespec {
    tv_sec: 57,
    tv_nsec: 79,
  };
  let mut valid_ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(0);

  let invalid_clock_id: clockid_t = c_int::MAX;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let valid_result = clock_gettime(CLOCK_REALTIME, &raw mut valid_ts);

  assert_eq!(valid_result, 0);
  assert!(valid_ts.tv_sec > 0);
  assert!((0..1_000_000_000).contains(&valid_ts.tv_nsec));
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn clock_gettime_monotonic_success_after_invalid_clock_id_set_keeps_errno_einval() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];

  for (index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 58 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 80 + i64::try_from(index).unwrap_or(0),
    };
    let mut valid_ts = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    write_errno(0);

    let invalid_result = clock_gettime(*invalid_clock_id, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);

    let valid_result = clock_gettime(CLOCK_MONOTONIC, &raw mut valid_ts);

    assert_eq!(valid_result, 0);
    assert!(valid_ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&valid_ts.tv_nsec));
    assert_eq!(read_errno(), EINVAL);
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_after_invalid_clock_id_set_keep_errno_einval() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];
  let valid_clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (invalid_index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 90 + i64::try_from(invalid_index).unwrap_or(0),
      tv_nsec: 120 + i64::try_from(invalid_index).unwrap_or(0),
    };

    write_errno(0);

    let invalid_result = clock_gettime(*invalid_clock_id, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);

    for (valid_index, valid_clock_id) in valid_clock_ids.iter().enumerate() {
      let mut ts = timespec {
        tv_sec: i64::try_from(valid_index).unwrap_or(0),
        tv_nsec: 0,
      };
      let result = clock_gettime(*valid_clock_id, &raw mut ts);

      if result == 0 {
        assert!(ts.tv_sec >= 0);
        assert!((0..1_000_000_000).contains(&ts.tv_nsec));
        assert_eq!(read_errno(), EINVAL);
        continue;
      }

      assert_eq!(result, -1);
      assert_eq!(read_errno(), EINVAL);
    }
  }
}

#[test]
fn clock_gettime_all_exported_clock_ids_after_invalid_clock_id_set_preserve_output_on_failure() {
  let invalid_clock_ids: [clockid_t; 4] = [-1, 9_999, c_int::MIN, c_int::MAX];
  let valid_clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (invalid_index, invalid_clock_id) in invalid_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 190 + i64::try_from(invalid_index).unwrap_or(0),
      tv_nsec: 220 + i64::try_from(invalid_index).unwrap_or(0),
    };

    write_errno(0);

    let invalid_result = clock_gettime(*invalid_clock_id, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);

    for (valid_index, valid_clock_id) in valid_clock_ids.iter().enumerate() {
      let mut ts = timespec {
        tv_sec: 6_000 + i64::try_from(valid_index).unwrap_or(0),
        tv_nsec: 7_000 + i64::try_from(valid_index).unwrap_or(0),
      };
      let before = ts;
      let result = clock_gettime(*valid_clock_id, &raw mut ts);

      if result == 0 {
        assert!(ts.tv_sec >= 0);
        assert!((0..1_000_000_000).contains(&ts.tv_nsec));
        assert_eq!(read_errno(), EINVAL);
        continue;
      }

      assert_eq!(result, -1);
      assert_eq!(read_errno(), EINVAL);
      assert_eq!(ts, before);
    }
  }
}

#[test]
fn clock_gettime_null_timespec_returns_minus_one_and_sets_efault() {
  write_errno(0);

  let result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_prioritizes_efault_over_invalid_clock_id() {
  write_errno(0);

  let invalid_clock_id: clockid_t = -1;
  let result = clock_gettime(invalid_clock_id, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_dynamic_clock_id_with_null_timespec_prioritizes_efault() {
  let dynamic_clock_id = fd_to_clockid(0);

  write_errno(EINVAL);

  let result = clock_gettime(dynamic_clock_id, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_dynamic_clock_id_alias_set_with_null_timespec_overwrites_errno_with_efault() {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for dynamic_clock_id in dynamic_clock_ids {
    write_errno(EINVAL);

    let result = clock_gettime(dynamic_clock_id, ptr::null_mut());

    assert_eq!(result, -1);
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_dynamic_clock_id_after_null_timespec_follows_errno_contract() {
  let dynamic_clock_id = fd_to_clockid(0);
  let mut ts = timespec {
    tv_sec: 1_234,
    tv_nsec: 5_678,
  };
  let before = ts;

  write_errno(EINVAL);

  let null_result = clock_gettime(dynamic_clock_id, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let result = clock_gettime(dynamic_clock_id, &raw mut ts);

  if result == 0 {
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), EFAULT);

    return;
  }

  assert_eq!(result, -1);
  assert_ne!(read_errno(), EFAULT);
  assert_eq!(ts, before);
}

#[test]
fn clock_gettime_dynamic_clock_id_alias_set_after_null_timespec_preserves_output_on_failure() {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut ts = timespec {
      tv_sec: 4_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 5_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = ts;

    write_errno(EINVAL);

    let null_result = clock_gettime(*dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let result = clock_gettime(*dynamic_clock_id, &raw mut ts);

    if result == 0 {
      assert!(ts.tv_sec >= 0);
      assert!((0..1_000_000_000).contains(&ts.tv_nsec));
      assert_eq!(read_errno(), EFAULT);

      continue;
    }

    assert_eq!(result, -1);
    assert_ne!(read_errno(), EFAULT);
    assert_eq!(ts, before);
  }
}

#[test]
fn clock_gettime_monotonic_success_after_dynamic_clock_id_alias_set_null_timespec_keeps_errno_efault()
 {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for dynamic_clock_id in dynamic_clock_ids {
    let mut ts = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    write_errno(EINVAL);

    let null_result = clock_gettime(dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let success_result = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

    assert_eq!(success_result, 0);
    assert!(ts.tv_sec >= 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn clock_gettime_realtime_success_after_dynamic_clock_id_alias_set_null_timespec_keeps_errno_efault()
 {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for dynamic_clock_id in dynamic_clock_ids {
    let mut ts = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    write_errno(EINVAL);

    let null_result = clock_gettime(dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let success_result = clock_gettime(CLOCK_REALTIME, &raw mut ts);

    assert_eq!(success_result, 0);
    assert!(ts.tv_sec > 0);
    assert!((0..1_000_000_000).contains(&ts.tv_nsec));
    assert_eq!(read_errno(), EFAULT);
  }
}

#[test]
fn dynamic_alias_null_then_all_clocks_follow_errno_contract() {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];
  let valid_clock_ids: [clockid_t; 12] = [
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,
    CLOCK_REALTIME_COARSE,
    CLOCK_MONOTONIC_COARSE,
    CLOCK_BOOTTIME,
    CLOCK_REALTIME_ALARM,
    CLOCK_BOOTTIME_ALARM,
    CLOCK_SGI_CYCLE,
    CLOCK_TAI,
  ];

  for (alias_index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    write_errno(EINVAL);

    let null_result = clock_gettime(*dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    for (valid_index, valid_clock_id) in valid_clock_ids.iter().enumerate() {
      let mut ts = timespec {
        tv_sec: 8_000
          + i64::try_from(alias_index).unwrap_or(0)
          + i64::try_from(valid_index).unwrap_or(0),
        tv_nsec: 9_000
          + i64::try_from(alias_index).unwrap_or(0)
          + i64::try_from(valid_index).unwrap_or(0),
      };
      let before = ts;

      write_errno(EFAULT);
      let errno_before_call = read_errno();
      let result = clock_gettime(*valid_clock_id, &raw mut ts);

      if result == 0 {
        assert!(ts.tv_sec >= 0);
        assert!((0..1_000_000_000).contains(&ts.tv_nsec));
        assert_eq!(read_errno(), errno_before_call);

        continue;
      }

      assert_eq!(result, -1);
      assert_eq!(read_errno(), EINVAL);
      assert_eq!(ts, before);
    }
  }
}

#[test]
fn invalid_clock_id_after_dynamic_alias_null_timespec_overwrites_errno_with_einval() {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 10_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 20_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = invalid_ts;

    write_errno(EINVAL);

    let null_result = clock_gettime(*dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let invalid_result = clock_gettime(9_999, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(invalid_ts, before);
  }
}

#[test]
fn max_positive_invalid_clock_id_after_dynamic_alias_null_timespec_overwrites_errno_with_einval() {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 30_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 40_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = invalid_ts;

    write_errno(EINVAL);

    let null_result = clock_gettime(*dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let invalid_clock_id: clockid_t = c_int::MAX;
    let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(invalid_ts, before);
  }
}

#[test]
fn extreme_negative_invalid_clock_id_after_dynamic_alias_null_timespec_overwrites_errno_with_einval()
 {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 50_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 60_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = invalid_ts;

    write_errno(EINVAL);

    let null_result = clock_gettime(*dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let invalid_clock_id: clockid_t = c_int::MIN;
    let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(invalid_ts, before);
  }
}

#[test]
fn negative_one_invalid_clock_id_after_dynamic_alias_null_timespec_overwrites_errno_with_einval() {
  let dynamic_clock_ids: [clockid_t; 3] = [
    fd_to_clockid(-1),
    fd_to_clockid(-2),
    fd_to_clockid(c_int::MAX),
  ];

  for (index, dynamic_clock_id) in dynamic_clock_ids.iter().enumerate() {
    let mut invalid_ts = timespec {
      tv_sec: 70_000 + i64::try_from(index).unwrap_or(0),
      tv_nsec: 80_000 + i64::try_from(index).unwrap_or(0),
    };
    let before = invalid_ts;

    write_errno(EINVAL);

    let null_result = clock_gettime(*dynamic_clock_id, ptr::null_mut());

    assert_eq!(null_result, -1);
    assert_eq!(read_errno(), EFAULT);

    let invalid_clock_id: clockid_t = -1;
    let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

    assert_eq!(invalid_result, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(invalid_ts, before);
  }
}

#[test]
fn clock_gettime_null_timespec_prioritizes_efault_over_large_positive_invalid_clock_id() {
  write_errno(77);

  let invalid_clock_id: clockid_t = 9_999;
  let result = clock_gettime(invalid_clock_id, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_prioritizes_efault_over_max_positive_invalid_clock_id() {
  write_errno(77);

  let invalid_clock_id: clockid_t = c_int::MAX;
  let result = clock_gettime(invalid_clock_id, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_prioritizes_efault_over_extreme_negative_invalid_clock_id() {
  write_errno(77);

  let invalid_clock_id: clockid_t = c_int::MIN;
  let result = clock_gettime(invalid_clock_id, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_overwrites_existing_errno_with_efault() {
  write_errno(123);

  let result = clock_gettime(CLOCK_MONOTONIC, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_with_sgi_cycle_still_sets_efault() {
  write_errno(0);

  let result = clock_gettime(CLOCK_SGI_CYCLE, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_with_process_cputime_still_sets_efault() {
  write_errno(0);

  let result = clock_gettime(CLOCK_PROCESS_CPUTIME_ID, ptr::null_mut());

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_success_after_null_timespec_keeps_errno_efault() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(0);

  let null_result = clock_gettime(CLOCK_MONOTONIC, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_result = clock_gettime(CLOCK_REALTIME, &raw mut ts);

  assert_eq!(success_result, 0);
  assert!(ts.tv_sec > 0);
  assert!((0..1_000_000_000).contains(&ts.tv_nsec));
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_monotonic_success_after_null_timespec_keeps_errno_efault() {
  let mut ts = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  write_errno(0);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let success_result = clock_gettime(CLOCK_MONOTONIC, &raw mut ts);

  assert_eq!(success_result, 0);
  assert!(ts.tv_sec >= 0);
  assert!((0..1_000_000_000).contains(&ts.tv_nsec));
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_after_invalid_clock_id_overwrites_errno_with_efault() {
  let mut invalid_ts = timespec {
    tv_sec: 33,
    tv_nsec: 44,
  };

  write_errno(0);

  let invalid_result = clock_gettime(-1, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_after_large_positive_invalid_clock_id_overwrites_errno_with_efault()
{
  let mut invalid_ts = timespec {
    tv_sec: 34,
    tv_nsec: 45,
  };
  let before = invalid_ts;

  write_errno(0);

  let invalid_clock_id: clockid_t = 9_999;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(invalid_ts, before);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_after_max_positive_invalid_clock_id_overwrites_errno_with_efault() {
  let mut invalid_ts = timespec {
    tv_sec: 34,
    tv_nsec: 45,
  };
  let before = invalid_ts;

  write_errno(0);

  let invalid_clock_id: clockid_t = c_int::MAX;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(invalid_ts, before);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_null_timespec_after_extreme_negative_invalid_clock_id_overwrites_errno_with_efault()
 {
  let mut invalid_ts = timespec {
    tv_sec: 35,
    tv_nsec: 46,
  };
  let before = invalid_ts;

  write_errno(0);

  let invalid_clock_id: clockid_t = c_int::MIN;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(invalid_ts, before);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn clock_gettime_invalid_clock_id_after_null_timespec_overwrites_errno_with_einval() {
  let mut invalid_ts = timespec {
    tv_sec: 55,
    tv_nsec: 66,
  };
  let before = invalid_ts;

  write_errno(0);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let invalid_result = clock_gettime(-1, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(invalid_ts, before);
}

#[test]
fn clock_gettime_large_positive_invalid_clock_id_after_null_timespec_overwrites_errno_with_einval()
{
  let mut invalid_ts = timespec {
    tv_sec: 77,
    tv_nsec: 88,
  };
  let before = invalid_ts;

  write_errno(0);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let invalid_clock_id: clockid_t = 9_999;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(invalid_ts, before);
}

#[test]
fn clock_gettime_max_positive_invalid_clock_id_after_null_timespec_overwrites_errno_with_einval() {
  let mut invalid_ts = timespec {
    tv_sec: 87,
    tv_nsec: 98,
  };
  let before = invalid_ts;

  write_errno(0);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let invalid_clock_id: clockid_t = c_int::MAX;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(invalid_ts, before);
}

#[test]
fn clock_gettime_extreme_negative_invalid_clock_id_after_null_timespec_overwrites_errno_with_einval()
 {
  let mut invalid_ts = timespec {
    tv_sec: 99,
    tv_nsec: 111,
  };
  let before = invalid_ts;

  write_errno(0);

  let null_result = clock_gettime(CLOCK_REALTIME, ptr::null_mut());

  assert_eq!(null_result, -1);
  assert_eq!(read_errno(), EFAULT);

  let invalid_clock_id: clockid_t = c_int::MIN;
  let invalid_result = clock_gettime(invalid_clock_id, &raw mut invalid_ts);

  assert_eq!(invalid_result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(invalid_ts, before);
}
