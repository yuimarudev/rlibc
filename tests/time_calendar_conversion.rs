use core::ptr;
use rlibc::abi::errno::{EINVAL, ERANGE};
use rlibc::abi::types::c_int;
use rlibc::errno::__errno_location;
use rlibc::time::{gmtime, gmtime_r, localtime, localtime_r, mktime, time_t, timegm, tm};

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns a valid pointer for the current thread.
  unsafe { *__errno_location() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns a valid writable pointer for the current thread.
  unsafe {
    *__errno_location() = value;
  }
}

const fn zero_tm() -> tm {
  tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 0,
    tm_year: 0,
    tm_wday: 0,
    tm_yday: 0,
    tm_isdst: 0,
    tm_gmtoff: 0,
    tm_zone: ptr::null(),
  }
}

fn assert_normalized_calendar_metadata(value: &tm) {
  assert!((0..=6).contains(&value.tm_wday));
  assert!((0..=365).contains(&value.tm_yday));
}

fn assert_utc_baseline_output_fields(value: &tm) {
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn gmtime_r_epoch_zero_produces_unix_epoch_utc() {
  let timer: time_t = 0;
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

  assert_eq!(result_ptr, &raw mut out);
  assert_eq!(out.tm_year, 70);
  assert_eq!(out.tm_mon, 0);
  assert_eq!(out.tm_mday, 1);
  assert_eq!(out.tm_hour, 0);
  assert_eq!(out.tm_min, 0);
  assert_eq!(out.tm_sec, 0);
  assert_eq!(out.tm_wday, 4);
  assert_eq!(out.tm_yday, 0);
  assert_eq!(out.tm_isdst, 0);
  assert_eq!(out.tm_gmtoff, 0);
  assert!(out.tm_zone.is_null());
  assert_eq!(read_errno(), 0);
}

#[test]
fn gmtime_r_success_does_not_modify_errno() {
  let timer: time_t = 86_400;
  let mut out = zero_tm();

  write_errno(777);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

  assert_eq!(result_ptr, &raw mut out);
  assert_eq!(out.tm_mday, 2);
  assert_eq!(read_errno(), 777);
}

#[test]
fn gmtime_r_negative_one_maps_to_last_second_before_epoch() {
  let timer: time_t = -1;
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

  assert_eq!(result_ptr, &raw mut out);
  assert_eq!(out.tm_year, 69);
  assert_eq!(out.tm_mon, 11);
  assert_eq!(out.tm_mday, 31);
  assert_eq!(out.tm_hour, 23);
  assert_eq!(out.tm_min, 59);
  assert_eq!(out.tm_sec, 59);
  assert_eq!(out.tm_wday, 3);
  assert_eq!(out.tm_yday, 364);
  assert_eq!(read_errno(), 0);
}

#[test]
fn localtime_r_epoch_zero_matches_utc_baseline() {
  let timer: time_t = 0;
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { localtime_r(&raw const timer, &raw mut out) };

  assert_eq!(result_ptr, &raw mut out);
  assert_eq!(out.tm_year, 70);
  assert_eq!(out.tm_mon, 0);
  assert_eq!(out.tm_mday, 1);
  assert_eq!(out.tm_hour, 0);
  assert_eq!(out.tm_min, 0);
  assert_eq!(out.tm_sec, 0);
  assert_eq!(out.tm_wday, 4);
  assert_eq!(out.tm_yday, 0);
  assert_eq!(out.tm_isdst, 0);
  assert_eq!(out.tm_gmtoff, 0);
  assert!(out.tm_zone.is_null());
  assert_eq!(read_errno(), 0);
}

#[test]
fn localtime_r_success_does_not_modify_errno() {
  let timer: time_t = 172_800;
  let mut out = zero_tm();

  write_errno(888);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { localtime_r(&raw const timer, &raw mut out) };

  assert_eq!(result_ptr, &raw mut out);
  assert_eq!(out.tm_mday, 3);
  assert_eq!(read_errno(), 888);
}

#[test]
fn gmtime_r_and_timegm_round_trip_leap_day_timestamp() {
  let timestamp: time_t = 951_827_696;
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let gmtime_ptr = unsafe { gmtime_r(&raw const timestamp, &raw mut out) };

  assert_eq!(gmtime_ptr, &raw mut out);

  out.tm_isdst = -1;

  // SAFETY: pointer is valid for the duration of the call.
  let round_tripped = unsafe { timegm(&raw mut out) };

  assert_eq!(round_tripped, timestamp);
  assert_eq!(read_errno(), 0);
}

#[test]
fn timegm_normalizes_fields_and_updates_calendar_metadata() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 24,
    tm_mday: 29,
    tm_mon: 1,
    tm_year: 124,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 123,
    tm_zone: ptr::dangling(),
  };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let timestamp = unsafe { timegm(&raw mut value) };

  assert_eq!(timestamp, 1_709_251_200);
  assert_eq!(value.tm_year, 124);
  assert_eq!(value.tm_mon, 2);
  assert_eq!(value.tm_mday, 1);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_eq!(value.tm_wday, 5);
  assert_eq!(value.tm_yday, 60);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
  assert_eq!(read_errno(), 0);
}

#[test]
fn gmtime_r_and_timegm_report_einval_for_null_inputs() {
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: null timer pointer is intentionally passed to validate error handling.
  let gmtime_result = unsafe { gmtime_r(ptr::null(), &raw mut out) };

  assert!(gmtime_result.is_null());
  assert_eq!(read_errno(), EINVAL);

  let timer: time_t = 0;

  write_errno(0);

  // SAFETY: null result pointer is intentionally passed to validate error handling.
  let gmtime_null_result = unsafe { gmtime_r(&raw const timer, ptr::null_mut()) };

  assert!(gmtime_null_result.is_null());
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  // SAFETY: null `tm` pointer is intentionally passed to validate error handling.
  let timegm_result = unsafe { timegm(ptr::null_mut()) };

  assert_eq!(timegm_result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn gmtime_r_error_path_keeps_errno_thread_local_across_threads() {
  write_errno(901);

  let child_errno = std::thread::spawn(|| {
    let mut out = zero_tm();

    write_errno(0);

    // SAFETY: null `timer` is intentional for error-path contract validation.
    let result_ptr = unsafe { gmtime_r(ptr::null(), &raw mut out) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, EINVAL);
  assert_eq!(read_errno(), 901);
}

#[test]
fn gmtime_r_erange_path_keeps_errno_thread_local_across_threads() {
  write_errno(911);

  let child_errno = std::thread::spawn(|| {
    let timer = time_t::MAX;
    let mut out = zero_tm();

    write_errno(0);

    // SAFETY: pointers are valid and `time_t::MAX` exercises ERANGE path.
    let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 911);
}

#[test]
fn gmtime_r_erange_minimum_path_keeps_errno_thread_local_across_threads() {
  write_errno(913);

  let child_errno = std::thread::spawn(|| {
    let timer = time_t::MIN;
    let mut out = zero_tm();

    write_errno(0);

    // SAFETY: pointers are valid and `time_t::MIN` exercises ERANGE path.
    let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 913);
}

#[test]
fn gmtime_r_null_result_short_circuits_before_timer_read() {
  let invalid_timer: *const time_t = ptr::dangling();

  write_errno(0);

  // SAFETY: null `result` is intentional; this validates short-circuit before
  // reading an invalid non-null `timer`.
  let gmtime_result = unsafe { gmtime_r(invalid_timer, ptr::null_mut()) };

  assert!(gmtime_result.is_null());
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn gmtime_r_null_result_path_keeps_errno_thread_local_across_threads() {
  write_errno(915);

  let child_errno = std::thread::spawn(|| {
    let invalid_timer: *const time_t = ptr::dangling();

    write_errno(0);

    // SAFETY: null `result` is intentional; this validates short-circuit before
    // reading an invalid non-null `timer`.
    let result_ptr = unsafe { gmtime_r(invalid_timer, ptr::null_mut()) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, EINVAL);
  assert_eq!(read_errno(), 915);
}

#[test]
fn gmtime_r_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(917);

  let child_errno = std::thread::spawn(|| {
    let timer: time_t = 86_400;
    let mut out = zero_tm();

    write_errno(731);

    // SAFETY: pointers are valid for the duration of the call.
    let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

    assert_eq!(result_ptr, &raw mut out);
    assert_eq!(out.tm_mday, 2);
    assert_normalized_calendar_metadata(&out);
    assert_utc_baseline_output_fields(&out);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 731);
  assert_eq!(read_errno(), 917);
}

#[test]
fn gmtime_r_einval_null_timer_does_not_mutate_output_buffer() {
  let mut out = tm {
    tm_sec: 7,
    tm_min: 6,
    tm_hour: 5,
    tm_mday: 4,
    tm_mon: 3,
    tm_year: 2,
    tm_wday: 1,
    tm_yday: 9,
    tm_isdst: -1,
    tm_gmtoff: 321,
    tm_zone: ptr::dangling(),
  };
  let original = out;

  write_errno(0);

  // SAFETY: null `timer` is intentional for error-path contract validation.
  let result_ptr = unsafe { gmtime_r(ptr::null(), &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(out, original);
}

#[test]
fn gmtime_reuses_thread_local_storage_and_overwrites_previous_result() {
  let first: time_t = 0;
  let second: time_t = 86_400;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let first_ptr = unsafe { gmtime(&raw const first) };

  assert!(!first_ptr.is_null());

  // SAFETY: `first_ptr` is non-null and points to thread-local `tm` storage.
  let first_value = unsafe { *first_ptr };

  // SAFETY: pointers are valid for the duration of the call.
  let second_ptr = unsafe { gmtime(&raw const second) };

  assert_eq!(second_ptr, first_ptr);

  // SAFETY: `second_ptr` is non-null and points to thread-local `tm` storage.
  let second_value = unsafe { *second_ptr };

  assert_eq!(first_value.tm_mday, 1);
  assert_eq!(second_value.tm_mday, 2);
  assert_eq!(second_value.tm_wday, 5);
  assert_eq!(read_errno(), 0);
}

#[test]
fn gmtime_success_does_not_modify_errno() {
  let timer: time_t = 0;

  write_errno(1234);

  // SAFETY: pointer is valid for the duration of the call.
  let result_ptr = unsafe { gmtime(&raw const timer) };

  assert!(!result_ptr.is_null());
  assert_eq!(read_errno(), 1234);
}

#[test]
fn localtime_reuses_thread_local_storage_and_overwrites_previous_result() {
  let first: time_t = 0;
  let second: time_t = 86_400;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let first_ptr = unsafe { localtime(&raw const first) };

  assert!(!first_ptr.is_null());

  // SAFETY: `first_ptr` is non-null and points to thread-local `tm` storage.
  let first_value = unsafe { *first_ptr };

  // SAFETY: pointers are valid for the duration of the call.
  let second_ptr = unsafe { localtime(&raw const second) };

  assert_eq!(second_ptr, first_ptr);

  // SAFETY: `second_ptr` is non-null and points to thread-local `tm` storage.
  let second_value = unsafe { *second_ptr };

  assert_eq!(first_value.tm_mday, 1);
  assert_eq!(second_value.tm_mday, 2);
  assert_eq!(second_value.tm_wday, 5);
  assert_eq!(read_errno(), 0);
}

#[test]
fn localtime_success_does_not_modify_errno() {
  let timer: time_t = 86_400;

  write_errno(4321);

  // SAFETY: pointer is valid for the duration of the call.
  let result_ptr = unsafe { localtime(&raw const timer) };

  assert!(!result_ptr.is_null());
  assert_eq!(read_errno(), 4321);
}

#[test]
fn gmtime_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(935);

  let child_errno = std::thread::spawn(|| {
    let timer: time_t = 86_400;

    write_errno(735);

    // SAFETY: pointer is valid for the duration of the call.
    let result_ptr = unsafe { gmtime(&raw const timer) };

    assert!(!result_ptr.is_null());

    // SAFETY: `result_ptr` is non-null and points to child-thread-local `tm`.
    let value = unsafe { *result_ptr };

    assert_eq!(value.tm_mday, 2);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 735);
  assert_eq!(read_errno(), 935);
}

#[test]
fn localtime_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(936);

  let child_errno = std::thread::spawn(|| {
    let timer: time_t = 172_800;

    write_errno(736);

    // SAFETY: pointer is valid for the duration of the call.
    let result_ptr = unsafe { localtime(&raw const timer) };

    assert!(!result_ptr.is_null());

    // SAFETY: `result_ptr` is non-null and points to child-thread-local `tm`.
    let value = unsafe { *result_ptr };

    assert_eq!(value.tm_mday, 3);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 736);
  assert_eq!(read_errno(), 936);
}

#[test]
fn gmtime_and_localtime_share_nonreentrant_thread_local_storage() {
  let gmtime_input: time_t = 0;
  let localtime_input: time_t = 86_400;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let gmtime_ptr = unsafe { gmtime(&raw const gmtime_input) };

  assert!(!gmtime_ptr.is_null());

  // SAFETY: pointers are valid for the duration of the call.
  let localtime_ptr = unsafe { localtime(&raw const localtime_input) };

  assert!(!localtime_ptr.is_null());
  assert_eq!(localtime_ptr, gmtime_ptr);

  // SAFETY: `localtime_ptr` is non-null and points to thread-local `tm` storage.
  let overwritten = unsafe { *localtime_ptr };

  assert_eq!(overwritten.tm_mday, 2);
  assert_eq!(overwritten.tm_wday, 5);
  assert_eq!(read_errno(), 0);
}

#[test]
fn gmtime_and_localtime_storage_is_thread_local_across_threads() {
  let main_timer: time_t = 0;

  write_errno(629);

  // SAFETY: pointer is valid for the duration of the call.
  let main_ptr = unsafe { gmtime(&raw const main_timer) };

  assert!(!main_ptr.is_null());

  // SAFETY: `main_ptr` is non-null and points to this thread's TLS-backed `tm`.
  let main_snapshot = unsafe { *main_ptr };
  let child_mday = std::thread::spawn(|| {
    let child_timer: time_t = 172_800;

    write_errno(731);

    // SAFETY: pointer is valid for the duration of the call.
    let local_ptr = unsafe { localtime(&raw const child_timer) };

    assert!(!local_ptr.is_null());

    // SAFETY: pointer is valid for the duration of the call.
    let gmt_ptr = unsafe { gmtime(&raw const child_timer) };

    assert_eq!(gmt_ptr, local_ptr);

    // SAFETY: `gmt_ptr` is non-null and points to child-thread-local `tm`.
    let snapshot = unsafe { *gmt_ptr };

    assert_eq!(read_errno(), 731);

    snapshot.tm_mday
  })
  .join()
  .expect("child thread should not panic");

  // SAFETY: `main_ptr` still refers to the main thread's TLS-backed `tm`.
  let main_after_child = unsafe { *main_ptr };

  assert_eq!(main_after_child.tm_sec, main_snapshot.tm_sec);
  assert_eq!(main_after_child.tm_min, main_snapshot.tm_min);
  assert_eq!(main_after_child.tm_hour, main_snapshot.tm_hour);
  assert_eq!(main_after_child.tm_mday, main_snapshot.tm_mday);
  assert_eq!(main_after_child.tm_mon, main_snapshot.tm_mon);
  assert_eq!(main_after_child.tm_year, main_snapshot.tm_year);
  assert_eq!(main_after_child.tm_wday, main_snapshot.tm_wday);
  assert_eq!(main_after_child.tm_yday, main_snapshot.tm_yday);
  assert_eq!(main_after_child.tm_isdst, main_snapshot.tm_isdst);
  assert_eq!(main_after_child.tm_gmtoff, main_snapshot.tm_gmtoff);
  assert_eq!(main_after_child.tm_zone, main_snapshot.tm_zone);
  assert_eq!(main_after_child.tm_mday, 1);
  assert_eq!(child_mday, 3);
  assert_eq!(read_errno(), 629);
}

#[test]
fn gmtime_error_path_keeps_errno_thread_local_across_threads() {
  write_errno(777);

  let child_errno = std::thread::spawn(|| {
    write_errno(0);

    // SAFETY: null `timer` is intentional for error-path contract validation.
    let result_ptr = unsafe { gmtime(ptr::null()) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, EINVAL);
  assert_eq!(read_errno(), 777);
}

#[test]
fn localtime_error_path_keeps_errno_thread_local_across_threads() {
  write_errno(888);

  let child_errno = std::thread::spawn(|| {
    write_errno(0);

    // SAFETY: null `timer` is intentional for error-path contract validation.
    let result_ptr = unsafe { localtime(ptr::null()) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, EINVAL);
  assert_eq!(read_errno(), 888);
}

#[test]
fn gmtime_einval_in_child_does_not_clobber_parent_thread_storage() {
  let baseline_timer: time_t = 172_800;

  write_errno(641);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to parent-thread-local `tm`.
  let baseline = unsafe { *storage_ptr };

  std::thread::spawn(|| {
    write_errno(0);

    // SAFETY: null `timer` is intentional for error-path contract validation.
    let failed_ptr = unsafe { gmtime(ptr::null()) };

    assert!(failed_ptr.is_null());
    assert_eq!(read_errno(), EINVAL);
  })
  .join()
  .expect("child thread should not panic");

  // SAFETY: `storage_ptr` still refers to parent-thread-local `tm`.
  let parent_after_child = unsafe { *storage_ptr };

  assert_eq!(parent_after_child, baseline);
  assert_eq!(read_errno(), 641);
}

#[test]
fn localtime_einval_in_child_does_not_clobber_parent_thread_storage() {
  let baseline_timer: time_t = 86_400;

  write_errno(642);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to parent-thread-local `tm`.
  let baseline = unsafe { *storage_ptr };

  std::thread::spawn(|| {
    write_errno(0);

    // SAFETY: null `timer` is intentional for error-path contract validation.
    let failed_ptr = unsafe { localtime(ptr::null()) };

    assert!(failed_ptr.is_null());
    assert_eq!(read_errno(), EINVAL);
  })
  .join()
  .expect("child thread should not panic");

  // SAFETY: `storage_ptr` still refers to parent-thread-local `tm`.
  let parent_after_child = unsafe { *storage_ptr };

  assert_eq!(parent_after_child, baseline);
  assert_eq!(read_errno(), 642);
}

#[test]
fn gmtime_erange_in_child_does_not_clobber_parent_thread_storage() {
  let baseline_timer: time_t = 172_800;

  write_errno(643);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to parent-thread-local `tm`.
  let baseline = unsafe { *storage_ptr };

  std::thread::spawn(|| {
    let underflow_timer: time_t = time_t::MIN;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let failed_ptr = unsafe { gmtime(&raw const underflow_timer) };

    assert!(failed_ptr.is_null());
    assert_eq!(read_errno(), ERANGE);
  })
  .join()
  .expect("child thread should not panic");

  // SAFETY: `storage_ptr` still refers to parent-thread-local `tm`.
  let parent_after_child = unsafe { *storage_ptr };

  assert_eq!(parent_after_child, baseline);
  assert_eq!(read_errno(), 643);
}

#[test]
fn localtime_erange_in_child_does_not_clobber_parent_thread_storage() {
  let baseline_timer: time_t = 86_400;

  write_errno(644);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to parent-thread-local `tm`.
  let baseline = unsafe { *storage_ptr };

  std::thread::spawn(|| {
    let underflow_timer: time_t = time_t::MIN;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let failed_ptr = unsafe { localtime(&raw const underflow_timer) };

    assert!(failed_ptr.is_null());
    assert_eq!(read_errno(), ERANGE);
  })
  .join()
  .expect("child thread should not panic");

  // SAFETY: `storage_ptr` still refers to parent-thread-local `tm`.
  let parent_after_child = unsafe { *storage_ptr };

  assert_eq!(parent_after_child, baseline);
  assert_eq!(read_errno(), 644);
}

#[test]
fn gmtime_erange_max_in_child_does_not_clobber_parent_thread_storage() {
  let baseline_timer: time_t = 172_800;

  write_errno(645);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to parent-thread-local `tm`.
  let baseline = unsafe { *storage_ptr };

  std::thread::spawn(|| {
    let overflow_timer: time_t = time_t::MAX;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let failed_ptr = unsafe { gmtime(&raw const overflow_timer) };

    assert!(failed_ptr.is_null());
    assert_eq!(read_errno(), ERANGE);
  })
  .join()
  .expect("child thread should not panic");

  // SAFETY: `storage_ptr` still refers to parent-thread-local `tm`.
  let parent_after_child = unsafe { *storage_ptr };

  assert_eq!(parent_after_child, baseline);
  assert_eq!(read_errno(), 645);
}

#[test]
fn localtime_erange_max_in_child_does_not_clobber_parent_thread_storage() {
  let baseline_timer: time_t = 86_400;

  write_errno(646);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to parent-thread-local `tm`.
  let baseline = unsafe { *storage_ptr };

  std::thread::spawn(|| {
    let overflow_timer: time_t = time_t::MAX;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let failed_ptr = unsafe { localtime(&raw const overflow_timer) };

    assert!(failed_ptr.is_null());
    assert_eq!(read_errno(), ERANGE);
  })
  .join()
  .expect("child thread should not panic");

  // SAFETY: `storage_ptr` still refers to parent-thread-local `tm`.
  let parent_after_child = unsafe { *storage_ptr };

  assert_eq!(parent_after_child, baseline);
  assert_eq!(read_errno(), 646);
}

#[test]
fn gmtime_erange_path_keeps_errno_thread_local_across_threads() {
  write_errno(515);

  let child_errno = std::thread::spawn(|| {
    let overflow_timer: time_t = time_t::MAX;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result_ptr = unsafe { gmtime(&raw const overflow_timer) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 515);
}

#[test]
fn localtime_erange_path_keeps_errno_thread_local_across_threads() {
  write_errno(616);

  let child_errno = std::thread::spawn(|| {
    let overflow_timer: time_t = time_t::MAX;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result_ptr = unsafe { localtime(&raw const overflow_timer) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 616);
}

#[test]
fn gmtime_erange_minimum_path_keeps_errno_thread_local_across_threads() {
  write_errno(717);

  let child_errno = std::thread::spawn(|| {
    let underflow_timer: time_t = time_t::MIN;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result_ptr = unsafe { gmtime(&raw const underflow_timer) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 717);
}

#[test]
fn localtime_erange_minimum_path_keeps_errno_thread_local_across_threads() {
  write_errno(818);

  let child_errno = std::thread::spawn(|| {
    let underflow_timer: time_t = time_t::MIN;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result_ptr = unsafe { localtime(&raw const underflow_timer) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 818);
}

#[test]
fn gmtime_erange_does_not_clobber_thread_local_storage() {
  let baseline_timer: time_t = 86_400;
  let overflow_timer: time_t = time_t::MAX;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let failed_ptr = unsafe { gmtime(&raw const overflow_timer) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn gmtime_erange_minimum_does_not_clobber_thread_local_storage() {
  let baseline_timer: time_t = 86_400;
  let underflow_timer: time_t = time_t::MIN;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let failed_ptr = unsafe { gmtime(&raw const underflow_timer) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn gmtime_einval_null_timer_does_not_clobber_thread_local_storage() {
  let baseline_timer: time_t = 86_400;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: null `timer` is intentional for error-path contract validation.
  let failed_ptr = unsafe { gmtime(ptr::null()) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn localtime_erange_does_not_clobber_thread_local_storage() {
  let baseline_timer: time_t = 172_800;
  let overflow_timer: time_t = time_t::MAX;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let failed_ptr = unsafe { localtime(&raw const overflow_timer) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn localtime_erange_minimum_does_not_clobber_thread_local_storage() {
  let baseline_timer: time_t = 172_800;
  let underflow_timer: time_t = time_t::MIN;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let failed_ptr = unsafe { localtime(&raw const underflow_timer) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn localtime_einval_null_timer_does_not_clobber_thread_local_storage() {
  let baseline_timer: time_t = 172_800;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: null `timer` is intentional for error-path contract validation.
  let failed_ptr = unsafe { localtime(ptr::null()) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn gmtime_einval_null_timer_preserves_storage_written_by_localtime() {
  let baseline_timer: time_t = 172_800;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: null `timer` is intentional for error-path contract validation.
  let failed_ptr = unsafe { gmtime(ptr::null()) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn localtime_einval_null_timer_preserves_storage_written_by_gmtime() {
  let baseline_timer: time_t = 86_400;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: null `timer` is intentional for error-path contract validation.
  let failed_ptr = unsafe { localtime(ptr::null()) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn gmtime_erange_preserves_storage_written_by_localtime() {
  let baseline_timer: time_t = 172_800;
  let overflow_timer: time_t = time_t::MAX;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let failed_ptr = unsafe { gmtime(&raw const overflow_timer) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn localtime_erange_preserves_storage_written_by_gmtime() {
  let baseline_timer: time_t = 86_400;
  let overflow_timer: time_t = time_t::MAX;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let failed_ptr = unsafe { localtime(&raw const overflow_timer) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn gmtime_erange_minimum_preserves_storage_written_by_localtime() {
  let baseline_timer: time_t = 172_800;
  let underflow_timer: time_t = time_t::MIN;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { localtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let failed_ptr = unsafe { gmtime(&raw const underflow_timer) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn localtime_erange_minimum_preserves_storage_written_by_gmtime() {
  let baseline_timer: time_t = 86_400;
  let underflow_timer: time_t = time_t::MIN;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let storage_ptr = unsafe { gmtime(&raw const baseline_timer) };

  assert!(!storage_ptr.is_null());

  // SAFETY: `storage_ptr` is non-null and points to thread-local `tm` storage.
  let baseline = unsafe { *storage_ptr };

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let failed_ptr = unsafe { localtime(&raw const underflow_timer) };

  assert!(failed_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);

  // SAFETY: `storage_ptr` remains valid thread-local storage.
  let after_failure = unsafe { *storage_ptr };

  assert_eq!(after_failure, baseline);
}

#[test]
fn gmtime_r_reports_erange_for_time_t_maximum() {
  let timer: time_t = time_t::MAX;
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn gmtime_r_erange_does_not_mutate_output_buffer() {
  let timer: time_t = time_t::MAX;
  let mut out = tm {
    tm_sec: 7,
    tm_min: 6,
    tm_hour: 5,
    tm_mday: 4,
    tm_mon: 3,
    tm_year: 2,
    tm_wday: 1,
    tm_yday: 9,
    tm_isdst: -1,
    tm_gmtoff: 123,
    tm_zone: ptr::dangling(),
  };
  let original = out;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(out, original);
}

#[test]
fn localtime_r_erange_does_not_mutate_output_buffer() {
  let timer: time_t = time_t::MAX;
  let mut out = tm {
    tm_sec: 17,
    tm_min: 16,
    tm_hour: 15,
    tm_mday: 14,
    tm_mon: 13,
    tm_year: 12,
    tm_wday: 11,
    tm_yday: 10,
    tm_isdst: -1,
    tm_gmtoff: 456,
    tm_zone: ptr::dangling(),
  };
  let original = out;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { localtime_r(&raw const timer, &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(out, original);
}

#[test]
fn gmtime_r_reports_erange_for_time_t_minimum() {
  let timer: time_t = time_t::MIN;
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn gmtime_r_erange_minimum_does_not_mutate_output_buffer() {
  let timer: time_t = time_t::MIN;
  let mut out = tm {
    tm_sec: 47,
    tm_min: 46,
    tm_hour: 45,
    tm_mday: 44,
    tm_mon: 43,
    tm_year: 42,
    tm_wday: 41,
    tm_yday: 40,
    tm_isdst: -1,
    tm_gmtoff: 789,
    tm_zone: ptr::dangling(),
  };
  let original = out;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { gmtime_r(&raw const timer, &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(out, original);
}

#[test]
fn localtime_r_reports_erange_for_time_t_minimum() {
  let timer: time_t = time_t::MIN;
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { localtime_r(&raw const timer, &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);
}

#[test]
fn localtime_r_erange_minimum_does_not_mutate_output_buffer() {
  let timer: time_t = time_t::MIN;
  let mut out = tm {
    tm_sec: 37,
    tm_min: 36,
    tm_hour: 35,
    tm_mday: 34,
    tm_mon: 33,
    tm_year: 32,
    tm_wday: 31,
    tm_yday: 30,
    tm_isdst: -1,
    tm_gmtoff: 987,
    tm_zone: ptr::dangling(),
  };
  let original = out;

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let result_ptr = unsafe { localtime_r(&raw const timer, &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(out, original);
}

#[test]
fn timegm_allows_valid_minus_one_timestamp_without_changing_errno() {
  let mut value = tm {
    tm_sec: 59,
    tm_min: 59,
    tm_hour: 23,
    tm_mday: 31,
    tm_mon: 11,
    tm_year: 69,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 123,
    tm_zone: ptr::dangling(),
  };

  write_errno(123);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), 123);
  assert_eq!(value.tm_year, 69);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 23);
  assert_eq!(value.tm_min, 59);
  assert_eq!(value.tm_sec, 59);
  assert_eq!(value.tm_wday, 3);
  assert_eq!(value.tm_yday, 364);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn timegm_valid_minus_one_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(963);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 59,
      tm_min: 59,
      tm_hour: 23,
      tm_mday: 31,
      tm_mon: 11,
      tm_year: 69,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };

    write_errno(743);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value.tm_year, 69);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 59);
    assert_eq!(value.tm_sec, 59);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 743);
  assert_eq!(read_errno(), 963);
}

#[test]
fn mktime_allows_valid_minus_one_timestamp_without_changing_errno() {
  let mut value = tm {
    tm_sec: 59,
    tm_min: 59,
    tm_hour: 23,
    tm_mday: 31,
    tm_mon: 11,
    tm_year: 69,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 99,
    tm_zone: ptr::dangling(),
  };

  write_errno(321);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), 321);
  assert_eq!(value.tm_year, 69);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 23);
  assert_eq!(value.tm_min, 59);
  assert_eq!(value.tm_sec, 59);
  assert_eq!(value.tm_wday, 3);
  assert_eq!(value.tm_yday, 364);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn mktime_valid_minus_one_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(964);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 59,
      tm_min: 59,
      tm_hour: 23,
      tm_mday: 31,
      tm_mon: 11,
      tm_year: 69,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };

    write_errno(744);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value.tm_year, 69);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 59);
    assert_eq!(value.tm_sec, 59);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 744);
  assert_eq!(read_errno(), 964);
}

#[test]
fn mktime_positive_tm_isdst_hint_subtracts_one_hour_under_utc_baseline() {
  let mut utc_baseline = tm {
    tm_sec: 12,
    tm_min: 34,
    tm_hour: 5,
    tm_mday: 20,
    tm_mon: 6,
    tm_year: 124,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 11,
    tm_zone: ptr::dangling(),
  };
  let mut dst_hint = tm {
    tm_isdst: 1,
    ..utc_baseline
  };

  write_errno(612);

  // SAFETY: pointer is valid for the duration of the call.
  let baseline_seconds = unsafe { timegm(&raw mut utc_baseline) };
  // SAFETY: pointer is valid for the duration of the call.
  let dst_hint_seconds = unsafe { mktime(&raw mut dst_hint) };

  assert_ne!(baseline_seconds, -1);
  assert_ne!(dst_hint_seconds, -1);
  assert_eq!(dst_hint_seconds, baseline_seconds - 3_600);
  assert_eq!(read_errno(), 612);
  assert_eq!(dst_hint.tm_year, 124);
  assert_eq!(dst_hint.tm_mon, 6);
  assert_eq!(dst_hint.tm_mday, 20);
  assert_eq!(dst_hint.tm_hour, 4);
  assert_eq!(dst_hint.tm_min, 34);
  assert_eq!(dst_hint.tm_sec, 12);
  assert_normalized_calendar_metadata(&dst_hint);
  assert_utc_baseline_output_fields(&dst_hint);
}

#[test]
fn mktime_large_positive_tm_isdst_hint_matches_tm_isdst_one_under_utc_baseline() {
  let seed = tm {
    tm_sec: 12,
    tm_min: 34,
    tm_hour: 5,
    tm_mday: 20,
    tm_mon: 6,
    tm_year: 124,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_one = tm {
    tm_isdst: 1,
    ..seed
  };
  let mut isdst_large = tm {
    tm_isdst: 7,
    ..seed
  };

  write_errno(616);

  // SAFETY: pointer is valid for the duration of the call.
  let one_seconds = unsafe { mktime(&raw mut isdst_one) };
  // SAFETY: pointer is valid for the duration of the call.
  let large_seconds = unsafe { mktime(&raw mut isdst_large) };

  assert_ne!(one_seconds, -1);
  assert_ne!(large_seconds, -1);
  assert_eq!(large_seconds, one_seconds);
  assert_eq!(read_errno(), 616);
  assert_eq!(isdst_large, isdst_one);
  assert_normalized_calendar_metadata(&isdst_large);
  assert_utc_baseline_output_fields(&isdst_large);
}

#[test]
fn mktime_large_positive_tm_isdst_hint_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(968);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 12,
      tm_min: 34,
      tm_hour: 5,
      tm_mday: 20,
      tm_mon: 6,
      tm_year: 124,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: 9,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(783);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, 124);
    assert_eq!(value.tm_mon, 6);
    assert_eq!(value.tm_mday, 20);
    assert_eq!(value.tm_hour, 4);
    assert_eq!(value.tm_min, 34);
    assert_eq!(value.tm_sec, 12);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 783);
  assert_eq!(read_errno(), 968);
}

#[test]
fn mktime_zero_tm_isdst_matches_minus_one_under_utc_baseline() {
  let seed = tm {
    tm_sec: 12,
    tm_min: 34,
    tm_hour: 5,
    tm_mday: 20,
    tm_mon: 6,
    tm_year: 124,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_zero = tm {
    tm_isdst: 0,
    ..seed
  };

  write_errno(617);

  // SAFETY: pointers are valid for the duration of the call.
  let unknown_seconds = unsafe { mktime(&raw mut isdst_unknown) };
  // SAFETY: pointer is valid for the duration of the call.
  let zero_seconds = unsafe { mktime(&raw mut isdst_zero) };

  assert_ne!(unknown_seconds, -1);
  assert_ne!(zero_seconds, -1);
  assert_eq!(zero_seconds, unknown_seconds);
  assert_eq!(read_errno(), 617);
  assert_eq!(isdst_zero, isdst_unknown);
  assert_normalized_calendar_metadata(&isdst_zero);
  assert_utc_baseline_output_fields(&isdst_zero);
}

#[test]
fn mktime_zero_tm_isdst_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(969);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 12,
      tm_min: 34,
      tm_hour: 5,
      tm_mday: 20,
      tm_mon: 6,
      tm_year: 124,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: 0,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(784);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, 124);
    assert_eq!(value.tm_mon, 6);
    assert_eq!(value.tm_mday, 20);
    assert_eq!(value.tm_hour, 5);
    assert_eq!(value.tm_min, 34);
    assert_eq!(value.tm_sec, 12);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 784);
  assert_eq!(read_errno(), 969);
}

#[test]
fn mktime_min_negative_tm_isdst_matches_minus_one_under_utc_baseline() {
  let seed = tm {
    tm_sec: 12,
    tm_min: 34,
    tm_hour: 5,
    tm_mday: 20,
    tm_mon: 6,
    tm_year: 124,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_min_negative = tm {
    tm_isdst: c_int::MIN,
    ..seed
  };

  write_errno(618);

  // SAFETY: pointers are valid for the duration of the call.
  let unknown_seconds = unsafe { mktime(&raw mut isdst_unknown) };
  // SAFETY: pointer is valid for the duration of the call.
  let min_negative_seconds = unsafe { mktime(&raw mut isdst_min_negative) };

  assert_ne!(unknown_seconds, -1);
  assert_ne!(min_negative_seconds, -1);
  assert_eq!(min_negative_seconds, unknown_seconds);
  assert_eq!(read_errno(), 618);
  assert_eq!(isdst_min_negative, isdst_unknown);
  assert_normalized_calendar_metadata(&isdst_min_negative);
  assert_utc_baseline_output_fields(&isdst_min_negative);
}

#[test]
fn mktime_min_negative_tm_isdst_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(970);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 12,
      tm_min: 34,
      tm_hour: 5,
      tm_mday: 20,
      tm_mon: 6,
      tm_year: 124,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MIN,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(785);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, 124);
    assert_eq!(value.tm_mon, 6);
    assert_eq!(value.tm_mday, 20);
    assert_eq!(value.tm_hour, 5);
    assert_eq!(value.tm_min, 34);
    assert_eq!(value.tm_sec, 12);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 785);
  assert_eq!(read_errno(), 970);
}

#[test]
fn mktime_min_negative_tm_isdst_matches_minus_one_for_valid_minus_one_input() {
  let seed = tm {
    tm_sec: 59,
    tm_min: 59,
    tm_hour: 23,
    tm_mday: 31,
    tm_mon: 11,
    tm_year: 69,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_min_negative = tm {
    tm_isdst: c_int::MIN,
    ..seed
  };

  write_errno(623);

  // SAFETY: pointer is valid for the duration of the call.
  let unknown_seconds = unsafe { mktime(&raw mut isdst_unknown) };
  // SAFETY: pointer is valid for the duration of the call.
  let min_negative_seconds = unsafe { mktime(&raw mut isdst_min_negative) };

  assert_eq!(unknown_seconds, -1);
  assert_eq!(min_negative_seconds, unknown_seconds);
  assert_eq!(read_errno(), 623);
  assert_eq!(isdst_min_negative, isdst_unknown);
  assert_eq!(isdst_min_negative.tm_year, 69);
  assert_eq!(isdst_min_negative.tm_mon, 11);
  assert_eq!(isdst_min_negative.tm_mday, 31);
  assert_eq!(isdst_min_negative.tm_hour, 23);
  assert_eq!(isdst_min_negative.tm_min, 59);
  assert_eq!(isdst_min_negative.tm_sec, 59);
  assert_normalized_calendar_metadata(&isdst_min_negative);
  assert_utc_baseline_output_fields(&isdst_min_negative);
}

#[test]
fn mktime_min_negative_tm_isdst_valid_minus_one_path_keeps_errno_thread_local_across_threads() {
  write_errno(973);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 59,
      tm_min: 59,
      tm_hour: 23,
      tm_mday: 31,
      tm_mon: 11,
      tm_year: 69,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MIN,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(788);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value.tm_year, 69);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 59);
    assert_eq!(value.tm_sec, 59);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 788);
  assert_eq!(read_errno(), 973);
}

#[test]
fn mktime_min_negative_tm_isdst_matches_minus_one_for_tm_year_minimum_boundary() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 0,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_min_negative = tm {
    tm_isdst: c_int::MIN,
    ..seed
  };

  write_errno(624);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(625);
  // SAFETY: pointer is valid for the duration of the call.
  let min_negative_result = unsafe { mktime(&raw mut isdst_min_negative) };
  let min_negative_errno = read_errno();

  assert_ne!(unknown_result, -1);
  assert_eq!(min_negative_result, unknown_result);
  assert_eq!(unknown_errno, 624);
  assert_eq!(min_negative_errno, 625);
  assert_eq!(isdst_min_negative, isdst_unknown);
  assert_normalized_calendar_metadata(&isdst_min_negative);
  assert_utc_baseline_output_fields(&isdst_min_negative);
}

#[test]
fn mktime_min_negative_tm_isdst_tm_year_minimum_path_keeps_errno_thread_local_across_threads() {
  write_errno(974);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 0,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MIN,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = tm {
      tm_isdst: -1,
      ..value
    };

    write_errno(791);
    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    assert_ne!(unknown_result, -1);
    assert_eq!(read_errno(), 791);

    write_errno(792);
    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, unknown_result);
    assert_eq!(value, unknown);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 792);
  assert_eq!(read_errno(), 974);
}

#[test]
fn mktime_min_negative_tm_isdst_matches_minus_one_for_month_borrow_boundary_at_tm_year_minimum() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: -1,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_min_negative = tm {
    tm_isdst: c_int::MIN,
    ..seed
  };

  write_errno(634);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(635);
  // SAFETY: pointer is valid for the duration of the call.
  let min_negative_result = unsafe { mktime(&raw mut isdst_min_negative) };
  let min_negative_errno = read_errno();

  assert_eq!(min_negative_result, unknown_result);
  if unknown_result == -1 {
    assert_eq!(unknown_errno, ERANGE);
    assert_eq!(min_negative_errno, ERANGE);
    assert_eq!(isdst_unknown, seed);
    assert_eq!(isdst_min_negative.tm_sec, seed.tm_sec);
    assert_eq!(isdst_min_negative.tm_min, seed.tm_min);
    assert_eq!(isdst_min_negative.tm_hour, seed.tm_hour);
    assert_eq!(isdst_min_negative.tm_mday, seed.tm_mday);
    assert_eq!(isdst_min_negative.tm_mon, seed.tm_mon);
    assert_eq!(isdst_min_negative.tm_year, seed.tm_year);
    assert_eq!(isdst_min_negative.tm_wday, seed.tm_wday);
    assert_eq!(isdst_min_negative.tm_yday, seed.tm_yday);
    assert_eq!(isdst_min_negative.tm_isdst, c_int::MIN);
    assert_eq!(isdst_min_negative.tm_gmtoff, seed.tm_gmtoff);
    assert_eq!(isdst_min_negative.tm_zone, seed.tm_zone);
  } else {
    assert_eq!(unknown_errno, 634);
    assert_eq!(min_negative_errno, 635);
    assert_normalized_calendar_metadata(&isdst_min_negative);
    assert_utc_baseline_output_fields(&isdst_min_negative);
    assert_eq!(isdst_min_negative, isdst_unknown);
  }
}

#[test]
fn mktime_min_negative_tm_isdst_month_borrow_boundary_keeps_errno_thread_local_across_threads() {
  write_errno(979);

  let (child_reference_result, child_errno) = std::thread::spawn(|| {
    let seed = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: -1,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = seed;
    let mut value = tm {
      tm_isdst: c_int::MIN,
      ..seed
    };

    write_errno(799);

    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    let unknown_errno = read_errno();

    write_errno(800);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };
    let result_errno = read_errno();

    assert_eq!(result, unknown_result);
    if unknown_result == -1 {
      assert_eq!(unknown_errno, ERANGE);
      assert_eq!(result_errno, ERANGE);
      assert_eq!(unknown, seed);
      assert_eq!(value.tm_sec, seed.tm_sec);
      assert_eq!(value.tm_min, seed.tm_min);
      assert_eq!(value.tm_hour, seed.tm_hour);
      assert_eq!(value.tm_mday, seed.tm_mday);
      assert_eq!(value.tm_mon, seed.tm_mon);
      assert_eq!(value.tm_year, seed.tm_year);
      assert_eq!(value.tm_wday, seed.tm_wday);
      assert_eq!(value.tm_yday, seed.tm_yday);
      assert_eq!(value.tm_isdst, c_int::MIN);
      assert_eq!(value.tm_gmtoff, seed.tm_gmtoff);
      assert_eq!(value.tm_zone, seed.tm_zone);
    } else {
      assert_eq!(unknown_errno, 799);
      assert_eq!(result_errno, 800);
      assert_normalized_calendar_metadata(&value);
      assert_utc_baseline_output_fields(&value);
      assert_eq!(value, unknown);
    }

    (unknown_result, result_errno)
  })
  .join()
  .expect("child thread should not panic");

  if child_reference_result == -1 {
    assert_eq!(child_errno, ERANGE);
  } else {
    assert_eq!(child_errno, 800);
  }
  assert_eq!(read_errno(), 979);
}

#[test]
fn mktime_min_negative_tm_isdst_matches_minus_one_for_tm_year_maximum_boundary() {
  let seed = tm {
    tm_sec: 59,
    tm_min: 59,
    tm_hour: 23,
    tm_mday: 31,
    tm_mon: 11,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_min_negative = tm {
    tm_isdst: c_int::MIN,
    ..seed
  };

  write_errno(628);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(629);
  // SAFETY: pointer is valid for the duration of the call.
  let min_negative_result = unsafe { mktime(&raw mut isdst_min_negative) };
  let min_negative_errno = read_errno();

  assert_ne!(unknown_result, -1);
  assert_eq!(min_negative_result, unknown_result);
  assert_eq!(unknown_errno, 628);
  assert_eq!(min_negative_errno, 629);
  assert_eq!(isdst_min_negative, isdst_unknown);
  assert_normalized_calendar_metadata(&isdst_min_negative);
  assert_utc_baseline_output_fields(&isdst_min_negative);
}

#[test]
fn mktime_min_negative_tm_isdst_tm_year_maximum_path_keeps_errno_thread_local_across_threads() {
  write_errno(976);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 59,
      tm_min: 59,
      tm_hour: 23,
      tm_mday: 31,
      tm_mon: 11,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MIN,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = tm {
      tm_isdst: -1,
      ..value
    };

    write_errno(793);
    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    assert_ne!(unknown_result, -1);
    assert_eq!(read_errno(), 793);

    write_errno(794);
    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, unknown_result);
    assert_eq!(value, unknown);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 794);
  assert_eq!(read_errno(), 976);
}

#[test]
fn mktime_min_negative_tm_isdst_matches_minus_one_for_day_carry_boundary_at_tm_year_maximum() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: 11,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_min_negative = tm {
    tm_isdst: c_int::MIN,
    ..seed
  };

  write_errno(630);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(631);
  // SAFETY: pointer is valid for the duration of the call.
  let min_negative_result = unsafe { mktime(&raw mut isdst_min_negative) };
  let min_negative_errno = read_errno();

  assert_eq!(min_negative_result, unknown_result);
  if unknown_result == -1 {
    assert_eq!(unknown_errno, ERANGE);
    assert_eq!(min_negative_errno, ERANGE);
    assert_eq!(isdst_unknown, seed);
    assert_eq!(isdst_min_negative.tm_sec, seed.tm_sec);
    assert_eq!(isdst_min_negative.tm_min, seed.tm_min);
    assert_eq!(isdst_min_negative.tm_hour, seed.tm_hour);
    assert_eq!(isdst_min_negative.tm_mday, seed.tm_mday);
    assert_eq!(isdst_min_negative.tm_mon, seed.tm_mon);
    assert_eq!(isdst_min_negative.tm_year, seed.tm_year);
    assert_eq!(isdst_min_negative.tm_wday, seed.tm_wday);
    assert_eq!(isdst_min_negative.tm_yday, seed.tm_yday);
    assert_eq!(isdst_min_negative.tm_isdst, c_int::MIN);
    assert_eq!(isdst_min_negative.tm_gmtoff, seed.tm_gmtoff);
    assert_eq!(isdst_min_negative.tm_zone, seed.tm_zone);
  } else {
    assert_eq!(unknown_errno, 630);
    assert_eq!(min_negative_errno, 631);
    assert_normalized_calendar_metadata(&isdst_min_negative);
    assert_utc_baseline_output_fields(&isdst_min_negative);
    assert_eq!(isdst_min_negative, isdst_unknown);
  }
}

#[test]
fn mktime_min_negative_tm_isdst_day_carry_boundary_keeps_errno_thread_local_across_threads() {
  write_errno(977);

  let (child_reference_result, child_errno) = std::thread::spawn(|| {
    let seed = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 32,
      tm_mon: 11,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = seed;
    let mut value = tm {
      tm_isdst: c_int::MIN,
      ..seed
    };

    write_errno(795);

    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    let unknown_errno = read_errno();

    write_errno(796);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };
    let result_errno = read_errno();

    assert_eq!(result, unknown_result);
    if unknown_result == -1 {
      assert_eq!(unknown_errno, ERANGE);
      assert_eq!(result_errno, ERANGE);
      assert_eq!(unknown, seed);
      assert_eq!(value.tm_sec, seed.tm_sec);
      assert_eq!(value.tm_min, seed.tm_min);
      assert_eq!(value.tm_hour, seed.tm_hour);
      assert_eq!(value.tm_mday, seed.tm_mday);
      assert_eq!(value.tm_mon, seed.tm_mon);
      assert_eq!(value.tm_year, seed.tm_year);
      assert_eq!(value.tm_wday, seed.tm_wday);
      assert_eq!(value.tm_yday, seed.tm_yday);
      assert_eq!(value.tm_isdst, c_int::MIN);
      assert_eq!(value.tm_gmtoff, seed.tm_gmtoff);
      assert_eq!(value.tm_zone, seed.tm_zone);
    } else {
      assert_eq!(unknown_errno, 795);
      assert_eq!(result_errno, 796);
      assert_normalized_calendar_metadata(&value);
      assert_utc_baseline_output_fields(&value);
      assert_eq!(value, unknown);
    }

    (unknown_result, result_errno)
  })
  .join()
  .expect("child thread should not panic");

  if child_reference_result == -1 {
    assert_eq!(child_errno, ERANGE);
  } else {
    assert_eq!(child_errno, 796);
  }
  assert_eq!(read_errno(), 977);
}

#[test]
fn mktime_min_negative_tm_isdst_matches_minus_one_for_month_carry_boundary_at_tm_year_maximum() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 12,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_min_negative = tm {
    tm_isdst: c_int::MIN,
    ..seed
  };

  write_errno(632);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(633);
  // SAFETY: pointer is valid for the duration of the call.
  let min_negative_result = unsafe { mktime(&raw mut isdst_min_negative) };
  let min_negative_errno = read_errno();

  assert_eq!(min_negative_result, unknown_result);
  if unknown_result == -1 {
    assert_eq!(unknown_errno, ERANGE);
    assert_eq!(min_negative_errno, ERANGE);
    assert_eq!(isdst_unknown, seed);
    assert_eq!(isdst_min_negative.tm_sec, seed.tm_sec);
    assert_eq!(isdst_min_negative.tm_min, seed.tm_min);
    assert_eq!(isdst_min_negative.tm_hour, seed.tm_hour);
    assert_eq!(isdst_min_negative.tm_mday, seed.tm_mday);
    assert_eq!(isdst_min_negative.tm_mon, seed.tm_mon);
    assert_eq!(isdst_min_negative.tm_year, seed.tm_year);
    assert_eq!(isdst_min_negative.tm_wday, seed.tm_wday);
    assert_eq!(isdst_min_negative.tm_yday, seed.tm_yday);
    assert_eq!(isdst_min_negative.tm_isdst, c_int::MIN);
    assert_eq!(isdst_min_negative.tm_gmtoff, seed.tm_gmtoff);
    assert_eq!(isdst_min_negative.tm_zone, seed.tm_zone);
  } else {
    assert_eq!(unknown_errno, 632);
    assert_eq!(min_negative_errno, 633);
    assert_normalized_calendar_metadata(&isdst_min_negative);
    assert_utc_baseline_output_fields(&isdst_min_negative);
    assert_eq!(isdst_min_negative, isdst_unknown);
  }
}

#[test]
fn mktime_min_negative_tm_isdst_month_carry_boundary_keeps_errno_thread_local_across_threads() {
  write_errno(978);

  let (child_reference_result, child_errno) = std::thread::spawn(|| {
    let seed = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 12,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = seed;
    let mut value = tm {
      tm_isdst: c_int::MIN,
      ..seed
    };

    write_errno(797);

    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    let unknown_errno = read_errno();

    write_errno(798);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };
    let result_errno = read_errno();

    assert_eq!(result, unknown_result);
    if unknown_result == -1 {
      assert_eq!(unknown_errno, ERANGE);
      assert_eq!(result_errno, ERANGE);
      assert_eq!(unknown, seed);
      assert_eq!(value.tm_sec, seed.tm_sec);
      assert_eq!(value.tm_min, seed.tm_min);
      assert_eq!(value.tm_hour, seed.tm_hour);
      assert_eq!(value.tm_mday, seed.tm_mday);
      assert_eq!(value.tm_mon, seed.tm_mon);
      assert_eq!(value.tm_year, seed.tm_year);
      assert_eq!(value.tm_wday, seed.tm_wday);
      assert_eq!(value.tm_yday, seed.tm_yday);
      assert_eq!(value.tm_isdst, c_int::MIN);
      assert_eq!(value.tm_gmtoff, seed.tm_gmtoff);
      assert_eq!(value.tm_zone, seed.tm_zone);
    } else {
      assert_eq!(unknown_errno, 797);
      assert_eq!(result_errno, 798);
      assert_normalized_calendar_metadata(&value);
      assert_utc_baseline_output_fields(&value);
      assert_eq!(value, unknown);
    }

    (unknown_result, result_errno)
  })
  .join()
  .expect("child thread should not panic");

  if child_reference_result == -1 {
    assert_eq!(child_errno, ERANGE);
  } else {
    assert_eq!(child_errno, 798);
  }
  assert_eq!(read_errno(), 978);
}

#[test]
fn mktime_large_negative_tm_isdst_matches_minus_one_under_utc_baseline() {
  let seed = tm {
    tm_sec: 12,
    tm_min: 34,
    tm_hour: 5,
    tm_mday: 20,
    tm_mon: 6,
    tm_year: 124,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_large_negative = tm {
    tm_isdst: -7,
    ..seed
  };

  write_errno(619);

  // SAFETY: pointers are valid for the duration of the call.
  let unknown_seconds = unsafe { mktime(&raw mut isdst_unknown) };
  // SAFETY: pointer is valid for the duration of the call.
  let large_negative_seconds = unsafe { mktime(&raw mut isdst_large_negative) };

  assert_ne!(unknown_seconds, -1);
  assert_ne!(large_negative_seconds, -1);
  assert_eq!(large_negative_seconds, unknown_seconds);
  assert_eq!(read_errno(), 619);
  assert_eq!(isdst_large_negative, isdst_unknown);
  assert_normalized_calendar_metadata(&isdst_large_negative);
  assert_utc_baseline_output_fields(&isdst_large_negative);
}

#[test]
fn mktime_large_negative_tm_isdst_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(971);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 12,
      tm_min: 34,
      tm_hour: 5,
      tm_mday: 20,
      tm_mon: 6,
      tm_year: 124,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -7,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(786);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, 124);
    assert_eq!(value.tm_mon, 6);
    assert_eq!(value.tm_mday, 20);
    assert_eq!(value.tm_hour, 5);
    assert_eq!(value.tm_min, 34);
    assert_eq!(value.tm_sec, 12);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 786);
  assert_eq!(read_errno(), 971);
}

#[test]
fn mktime_large_negative_tm_isdst_matches_minus_one_for_valid_minus_one_input() {
  let seed = tm {
    tm_sec: 59,
    tm_min: 59,
    tm_hour: 23,
    tm_mday: 31,
    tm_mon: 11,
    tm_year: 69,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_large_negative = tm {
    tm_isdst: -7,
    ..seed
  };

  write_errno(622);

  // SAFETY: pointer is valid for the duration of the call.
  let unknown_seconds = unsafe { mktime(&raw mut isdst_unknown) };
  // SAFETY: pointer is valid for the duration of the call.
  let large_negative_seconds = unsafe { mktime(&raw mut isdst_large_negative) };

  assert_eq!(unknown_seconds, -1);
  assert_eq!(large_negative_seconds, unknown_seconds);
  assert_eq!(read_errno(), 622);
  assert_eq!(isdst_large_negative, isdst_unknown);
  assert_eq!(isdst_large_negative.tm_year, 69);
  assert_eq!(isdst_large_negative.tm_mon, 11);
  assert_eq!(isdst_large_negative.tm_mday, 31);
  assert_eq!(isdst_large_negative.tm_hour, 23);
  assert_eq!(isdst_large_negative.tm_min, 59);
  assert_eq!(isdst_large_negative.tm_sec, 59);
  assert_normalized_calendar_metadata(&isdst_large_negative);
  assert_utc_baseline_output_fields(&isdst_large_negative);
}

#[test]
fn mktime_large_negative_tm_isdst_valid_minus_one_path_keeps_errno_thread_local_across_threads() {
  write_errno(972);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 59,
      tm_min: 59,
      tm_hour: 23,
      tm_mday: 31,
      tm_mon: 11,
      tm_year: 69,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -7,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(787);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value.tm_year, 69);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 59);
    assert_eq!(value.tm_sec, 59);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 787);
  assert_eq!(read_errno(), 972);
}

#[test]
fn mktime_large_negative_tm_isdst_matches_minus_one_for_tm_year_minimum_boundary() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 0,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_large_negative = tm {
    tm_isdst: -7,
    ..seed
  };

  write_errno(626);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(627);
  // SAFETY: pointer is valid for the duration of the call.
  let large_negative_result = unsafe { mktime(&raw mut isdst_large_negative) };
  let large_negative_errno = read_errno();

  assert_ne!(unknown_result, -1);
  assert_eq!(large_negative_result, unknown_result);
  assert_eq!(unknown_errno, 626);
  assert_eq!(large_negative_errno, 627);
  assert_eq!(isdst_large_negative, isdst_unknown);
  assert_normalized_calendar_metadata(&isdst_large_negative);
  assert_utc_baseline_output_fields(&isdst_large_negative);
}

#[test]
fn mktime_large_negative_tm_isdst_tm_year_minimum_path_keeps_errno_thread_local_across_threads() {
  write_errno(975);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 0,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -7,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = tm {
      tm_isdst: -1,
      ..value
    };

    write_errno(789);
    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    assert_ne!(unknown_result, -1);
    assert_eq!(read_errno(), 789);

    write_errno(790);
    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, unknown_result);
    assert_eq!(value, unknown);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 790);
  assert_eq!(read_errno(), 975);
}

#[test]
fn mktime_large_negative_tm_isdst_matches_minus_one_for_day_carry_boundary_at_tm_year_maximum() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: 11,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_large_negative = tm {
    tm_isdst: -7,
    ..seed
  };

  write_errno(628);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(629);
  // SAFETY: pointer is valid for the duration of the call.
  let large_negative_result = unsafe { mktime(&raw mut isdst_large_negative) };
  let large_negative_errno = read_errno();

  assert_eq!(large_negative_result, unknown_result);
  if unknown_result == -1 {
    assert_eq!(unknown_errno, ERANGE);
    assert_eq!(large_negative_errno, ERANGE);
    assert_eq!(isdst_unknown, seed);
    assert_eq!(isdst_large_negative.tm_sec, seed.tm_sec);
    assert_eq!(isdst_large_negative.tm_min, seed.tm_min);
    assert_eq!(isdst_large_negative.tm_hour, seed.tm_hour);
    assert_eq!(isdst_large_negative.tm_mday, seed.tm_mday);
    assert_eq!(isdst_large_negative.tm_mon, seed.tm_mon);
    assert_eq!(isdst_large_negative.tm_year, seed.tm_year);
    assert_eq!(isdst_large_negative.tm_wday, seed.tm_wday);
    assert_eq!(isdst_large_negative.tm_yday, seed.tm_yday);
    assert_eq!(isdst_large_negative.tm_isdst, -7);
    assert_eq!(isdst_large_negative.tm_gmtoff, seed.tm_gmtoff);
    assert_eq!(isdst_large_negative.tm_zone, seed.tm_zone);
  } else {
    assert_eq!(unknown_errno, 628);
    assert_eq!(large_negative_errno, 629);
    assert_normalized_calendar_metadata(&isdst_large_negative);
    assert_utc_baseline_output_fields(&isdst_large_negative);
    assert_eq!(isdst_large_negative, isdst_unknown);
  }
}

#[test]
fn mktime_large_negative_tm_isdst_day_carry_boundary_keeps_errno_thread_local_across_threads() {
  write_errno(976);

  let (child_reference_result, child_errno) = std::thread::spawn(|| {
    let seed = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 32,
      tm_mon: 11,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = seed;
    let mut value = tm {
      tm_isdst: -7,
      ..seed
    };

    write_errno(791);

    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    let unknown_errno = read_errno();

    write_errno(792);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };
    let result_errno = read_errno();

    assert_eq!(result, unknown_result);
    if unknown_result == -1 {
      assert_eq!(unknown_errno, ERANGE);
      assert_eq!(result_errno, ERANGE);
      assert_eq!(unknown, seed);
      assert_eq!(value.tm_sec, seed.tm_sec);
      assert_eq!(value.tm_min, seed.tm_min);
      assert_eq!(value.tm_hour, seed.tm_hour);
      assert_eq!(value.tm_mday, seed.tm_mday);
      assert_eq!(value.tm_mon, seed.tm_mon);
      assert_eq!(value.tm_year, seed.tm_year);
      assert_eq!(value.tm_wday, seed.tm_wday);
      assert_eq!(value.tm_yday, seed.tm_yday);
      assert_eq!(value.tm_isdst, -7);
      assert_eq!(value.tm_gmtoff, seed.tm_gmtoff);
      assert_eq!(value.tm_zone, seed.tm_zone);
    } else {
      assert_eq!(unknown_errno, 791);
      assert_eq!(result_errno, 792);
      assert_normalized_calendar_metadata(&value);
      assert_utc_baseline_output_fields(&value);
      assert_eq!(value, unknown);
    }

    (unknown_result, result_errno)
  })
  .join()
  .expect("child thread should not panic");

  if child_reference_result == -1 {
    assert_eq!(child_errno, ERANGE);
  } else {
    assert_eq!(child_errno, 792);
  }
  assert_eq!(read_errno(), 976);
}

#[test]
fn mktime_large_negative_tm_isdst_matches_minus_one_for_day_borrow_boundary_at_tm_year_minimum() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 0,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_large_negative = tm {
    tm_isdst: -7,
    ..seed
  };

  write_errno(642);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(643);
  // SAFETY: pointer is valid for the duration of the call.
  let large_negative_result = unsafe { mktime(&raw mut isdst_large_negative) };
  let large_negative_errno = read_errno();

  assert_eq!(large_negative_result, unknown_result);
  if unknown_result == -1 {
    assert_eq!(unknown_errno, ERANGE);
    assert_eq!(large_negative_errno, ERANGE);
    assert_eq!(isdst_unknown, seed);
    assert_eq!(isdst_large_negative.tm_sec, seed.tm_sec);
    assert_eq!(isdst_large_negative.tm_min, seed.tm_min);
    assert_eq!(isdst_large_negative.tm_hour, seed.tm_hour);
    assert_eq!(isdst_large_negative.tm_mday, seed.tm_mday);
    assert_eq!(isdst_large_negative.tm_mon, seed.tm_mon);
    assert_eq!(isdst_large_negative.tm_year, seed.tm_year);
    assert_eq!(isdst_large_negative.tm_wday, seed.tm_wday);
    assert_eq!(isdst_large_negative.tm_yday, seed.tm_yday);
    assert_eq!(isdst_large_negative.tm_isdst, -7);
    assert_eq!(isdst_large_negative.tm_gmtoff, seed.tm_gmtoff);
    assert_eq!(isdst_large_negative.tm_zone, seed.tm_zone);
  } else {
    assert_eq!(unknown_errno, 642);
    assert_eq!(large_negative_errno, 643);
    assert_normalized_calendar_metadata(&isdst_large_negative);
    assert_utc_baseline_output_fields(&isdst_large_negative);
    assert_eq!(isdst_large_negative, isdst_unknown);
  }
}

#[test]
fn mktime_large_negative_tm_isdst_day_borrow_boundary_keeps_errno_thread_local_across_threads() {
  write_errno(982);

  let (child_reference_result, child_errno) = std::thread::spawn(|| {
    let seed = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 0,
      tm_mon: 0,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = seed;
    let mut value = tm {
      tm_isdst: -7,
      ..seed
    };

    write_errno(807);

    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    let unknown_errno = read_errno();

    write_errno(808);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };
    let result_errno = read_errno();

    assert_eq!(result, unknown_result);
    if unknown_result == -1 {
      assert_eq!(unknown_errno, ERANGE);
      assert_eq!(result_errno, ERANGE);
      assert_eq!(unknown, seed);
      assert_eq!(value.tm_sec, seed.tm_sec);
      assert_eq!(value.tm_min, seed.tm_min);
      assert_eq!(value.tm_hour, seed.tm_hour);
      assert_eq!(value.tm_mday, seed.tm_mday);
      assert_eq!(value.tm_mon, seed.tm_mon);
      assert_eq!(value.tm_year, seed.tm_year);
      assert_eq!(value.tm_wday, seed.tm_wday);
      assert_eq!(value.tm_yday, seed.tm_yday);
      assert_eq!(value.tm_isdst, -7);
      assert_eq!(value.tm_gmtoff, seed.tm_gmtoff);
      assert_eq!(value.tm_zone, seed.tm_zone);
    } else {
      assert_eq!(unknown_errno, 807);
      assert_eq!(result_errno, 808);
      assert_normalized_calendar_metadata(&value);
      assert_utc_baseline_output_fields(&value);
      assert_eq!(value, unknown);
    }

    (unknown_result, result_errno)
  })
  .join()
  .expect("child thread should not panic");

  if child_reference_result == -1 {
    assert_eq!(child_errno, ERANGE);
  } else {
    assert_eq!(child_errno, 808);
  }
  assert_eq!(read_errno(), 982);
}

#[test]
fn mktime_large_negative_tm_isdst_matches_minus_one_for_month_borrow_boundary_at_tm_year_minimum() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: -1,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_large_negative = tm {
    tm_isdst: -7,
    ..seed
  };

  write_errno(640);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(641);
  // SAFETY: pointer is valid for the duration of the call.
  let large_negative_result = unsafe { mktime(&raw mut isdst_large_negative) };
  let large_negative_errno = read_errno();

  assert_eq!(large_negative_result, unknown_result);
  if unknown_result == -1 {
    assert_eq!(unknown_errno, ERANGE);
    assert_eq!(large_negative_errno, ERANGE);
    assert_eq!(isdst_unknown, seed);
    assert_eq!(isdst_large_negative.tm_sec, seed.tm_sec);
    assert_eq!(isdst_large_negative.tm_min, seed.tm_min);
    assert_eq!(isdst_large_negative.tm_hour, seed.tm_hour);
    assert_eq!(isdst_large_negative.tm_mday, seed.tm_mday);
    assert_eq!(isdst_large_negative.tm_mon, seed.tm_mon);
    assert_eq!(isdst_large_negative.tm_year, seed.tm_year);
    assert_eq!(isdst_large_negative.tm_wday, seed.tm_wday);
    assert_eq!(isdst_large_negative.tm_yday, seed.tm_yday);
    assert_eq!(isdst_large_negative.tm_isdst, -7);
    assert_eq!(isdst_large_negative.tm_gmtoff, seed.tm_gmtoff);
    assert_eq!(isdst_large_negative.tm_zone, seed.tm_zone);
  } else {
    assert_eq!(unknown_errno, 640);
    assert_eq!(large_negative_errno, 641);
    assert_normalized_calendar_metadata(&isdst_large_negative);
    assert_utc_baseline_output_fields(&isdst_large_negative);
    assert_eq!(isdst_large_negative, isdst_unknown);
  }
}

#[test]
fn mktime_large_negative_tm_isdst_month_borrow_boundary_keeps_errno_thread_local_across_threads() {
  write_errno(981);

  let (child_reference_result, child_errno) = std::thread::spawn(|| {
    let seed = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: -1,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = seed;
    let mut value = tm {
      tm_isdst: -7,
      ..seed
    };

    write_errno(805);

    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    let unknown_errno = read_errno();

    write_errno(806);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };
    let result_errno = read_errno();

    assert_eq!(result, unknown_result);
    if unknown_result == -1 {
      assert_eq!(unknown_errno, ERANGE);
      assert_eq!(result_errno, ERANGE);
      assert_eq!(unknown, seed);
      assert_eq!(value.tm_sec, seed.tm_sec);
      assert_eq!(value.tm_min, seed.tm_min);
      assert_eq!(value.tm_hour, seed.tm_hour);
      assert_eq!(value.tm_mday, seed.tm_mday);
      assert_eq!(value.tm_mon, seed.tm_mon);
      assert_eq!(value.tm_year, seed.tm_year);
      assert_eq!(value.tm_wday, seed.tm_wday);
      assert_eq!(value.tm_yday, seed.tm_yday);
      assert_eq!(value.tm_isdst, -7);
      assert_eq!(value.tm_gmtoff, seed.tm_gmtoff);
      assert_eq!(value.tm_zone, seed.tm_zone);
    } else {
      assert_eq!(unknown_errno, 805);
      assert_eq!(result_errno, 806);
      assert_normalized_calendar_metadata(&value);
      assert_utc_baseline_output_fields(&value);
      assert_eq!(value, unknown);
    }

    (unknown_result, result_errno)
  })
  .join()
  .expect("child thread should not panic");

  if child_reference_result == -1 {
    assert_eq!(child_errno, ERANGE);
  } else {
    assert_eq!(child_errno, 806);
  }
  assert_eq!(read_errno(), 981);
}

#[test]
fn mktime_large_negative_tm_isdst_matches_minus_one_for_month_carry_boundary_at_tm_year_maximum() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 12,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_unknown = seed;
  let mut isdst_large_negative = tm {
    tm_isdst: -7,
    ..seed
  };

  write_errno(636);
  // SAFETY: pointer is valid for the duration of the call.
  let unknown_result = unsafe { mktime(&raw mut isdst_unknown) };
  let unknown_errno = read_errno();

  write_errno(637);
  // SAFETY: pointer is valid for the duration of the call.
  let large_negative_result = unsafe { mktime(&raw mut isdst_large_negative) };
  let large_negative_errno = read_errno();

  assert_eq!(large_negative_result, unknown_result);
  if unknown_result == -1 {
    assert_eq!(unknown_errno, ERANGE);
    assert_eq!(large_negative_errno, ERANGE);
    assert_eq!(isdst_unknown, seed);
    assert_eq!(isdst_large_negative.tm_sec, seed.tm_sec);
    assert_eq!(isdst_large_negative.tm_min, seed.tm_min);
    assert_eq!(isdst_large_negative.tm_hour, seed.tm_hour);
    assert_eq!(isdst_large_negative.tm_mday, seed.tm_mday);
    assert_eq!(isdst_large_negative.tm_mon, seed.tm_mon);
    assert_eq!(isdst_large_negative.tm_year, seed.tm_year);
    assert_eq!(isdst_large_negative.tm_wday, seed.tm_wday);
    assert_eq!(isdst_large_negative.tm_yday, seed.tm_yday);
    assert_eq!(isdst_large_negative.tm_isdst, -7);
    assert_eq!(isdst_large_negative.tm_gmtoff, seed.tm_gmtoff);
    assert_eq!(isdst_large_negative.tm_zone, seed.tm_zone);
  } else {
    assert_eq!(unknown_errno, 636);
    assert_eq!(large_negative_errno, 637);
    assert_normalized_calendar_metadata(&isdst_large_negative);
    assert_utc_baseline_output_fields(&isdst_large_negative);
    assert_eq!(isdst_large_negative, isdst_unknown);
  }
}

#[test]
fn mktime_large_negative_tm_isdst_month_carry_boundary_keeps_errno_thread_local_across_threads() {
  write_errno(980);

  let (child_reference_result, child_errno) = std::thread::spawn(|| {
    let seed = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 12,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let mut unknown = seed;
    let mut value = tm {
      tm_isdst: -7,
      ..seed
    };

    write_errno(801);

    // SAFETY: pointer is valid for the duration of the call.
    let unknown_result = unsafe { mktime(&raw mut unknown) };
    let unknown_errno = read_errno();

    write_errno(802);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };
    let result_errno = read_errno();

    assert_eq!(result, unknown_result);
    if unknown_result == -1 {
      assert_eq!(unknown_errno, ERANGE);
      assert_eq!(result_errno, ERANGE);
      assert_eq!(unknown, seed);
      assert_eq!(value.tm_sec, seed.tm_sec);
      assert_eq!(value.tm_min, seed.tm_min);
      assert_eq!(value.tm_hour, seed.tm_hour);
      assert_eq!(value.tm_mday, seed.tm_mday);
      assert_eq!(value.tm_mon, seed.tm_mon);
      assert_eq!(value.tm_year, seed.tm_year);
      assert_eq!(value.tm_wday, seed.tm_wday);
      assert_eq!(value.tm_yday, seed.tm_yday);
      assert_eq!(value.tm_isdst, -7);
      assert_eq!(value.tm_gmtoff, seed.tm_gmtoff);
      assert_eq!(value.tm_zone, seed.tm_zone);
    } else {
      assert_eq!(unknown_errno, 801);
      assert_eq!(result_errno, 802);
      assert_normalized_calendar_metadata(&value);
      assert_utc_baseline_output_fields(&value);
      assert_eq!(value, unknown);
    }

    (unknown_result, result_errno)
  })
  .join()
  .expect("child thread should not panic");

  if child_reference_result == -1 {
    assert_eq!(child_errno, ERANGE);
  } else {
    assert_eq!(child_errno, 802);
  }
  assert_eq!(read_errno(), 980);
}

#[test]
fn mktime_positive_tm_isdst_hint_can_borrow_previous_day_under_utc_baseline() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 30,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 0,
    tm_year: 70,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: 1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };

  write_errno(613);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1_800);
  assert_eq!(read_errno(), 613);
  assert_eq!(value.tm_year, 69);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 23);
  assert_eq!(value.tm_min, 30);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_utc_baseline_output_fields(&value);
}

#[test]
fn mktime_positive_tm_isdst_hint_valid_minus_one_input_returns_minus_3601_without_errno_change() {
  let mut value = tm {
    tm_sec: 59,
    tm_min: 59,
    tm_hour: 23,
    tm_mday: 31,
    tm_mon: 11,
    tm_year: 69,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: 1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };

  write_errno(614);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -3_601);
  assert_eq!(read_errno(), 614);
  assert_eq!(value.tm_year, 69);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 22);
  assert_eq!(value.tm_min, 59);
  assert_eq!(value.tm_sec, 59);
  assert_normalized_calendar_metadata(&value);
  assert_utc_baseline_output_fields(&value);
}

#[test]
fn mktime_positive_tm_isdst_hint_valid_minus_one_input_keeps_errno_thread_local_across_threads() {
  write_errno(965);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 59,
      tm_min: 59,
      tm_hour: 23,
      tm_mday: 31,
      tm_mon: 11,
      tm_year: 69,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: 1,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(781);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -3_601);
    assert_eq!(value.tm_year, 69);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 22);
    assert_eq!(value.tm_min, 59);
    assert_eq!(value.tm_sec, 59);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 781);
  assert_eq!(read_errno(), 965);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_matches_tm_isdst_one_for_valid_minus_one_input() {
  let seed = tm {
    tm_sec: 59,
    tm_min: 59,
    tm_hour: 23,
    tm_mday: 31,
    tm_mon: 11,
    tm_year: 69,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_one = tm {
    tm_isdst: 1,
    ..seed
  };
  let mut isdst_max = tm {
    tm_isdst: c_int::MAX,
    ..seed
  };

  write_errno(620);

  // SAFETY: pointer is valid for the duration of the call.
  let one_seconds = unsafe { mktime(&raw mut isdst_one) };
  // SAFETY: pointer is valid for the duration of the call.
  let max_seconds = unsafe { mktime(&raw mut isdst_max) };

  assert_eq!(one_seconds, -3_601);
  assert_eq!(max_seconds, one_seconds);
  assert_eq!(read_errno(), 620);
  assert_eq!(isdst_max, isdst_one);
  assert_normalized_calendar_metadata(&isdst_max);
  assert_utc_baseline_output_fields(&isdst_max);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_valid_minus_one_path_keeps_errno_thread_local_across_threads()
{
  write_errno(971);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 59,
      tm_min: 59,
      tm_hour: 23,
      tm_mday: 31,
      tm_mon: 11,
      tm_year: 69,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MAX,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(786);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -3_601);
    assert_eq!(value.tm_year, 69);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 22);
    assert_eq!(value.tm_min, 59);
    assert_eq!(value.tm_sec, 59);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 786);
  assert_eq!(read_errno(), 971);
}

#[test]
fn mktime_positive_tm_isdst_hint_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(961);

  let child_errno = std::thread::spawn(|| {
    let mut utc_baseline = tm {
      tm_sec: 12,
      tm_min: 34,
      tm_hour: 5,
      tm_mday: 20,
      tm_mon: 6,
      tm_year: 124,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 11,
      tm_zone: ptr::dangling(),
    };
    let mut dst_hint = tm {
      tm_isdst: 1,
      ..utc_baseline
    };

    write_errno(741);

    // SAFETY: pointer is valid for the duration of the call.
    let baseline_seconds = unsafe { timegm(&raw mut utc_baseline) };

    assert_ne!(baseline_seconds, -1);
    assert_eq!(read_errno(), 741);

    write_errno(742);

    // SAFETY: pointer is valid for the duration of the call.
    let hinted_seconds = unsafe { mktime(&raw mut dst_hint) };

    assert_ne!(hinted_seconds, -1);
    assert_eq!(hinted_seconds, baseline_seconds - 3_600);
    assert_eq!(dst_hint.tm_year, 124);
    assert_eq!(dst_hint.tm_mon, 6);
    assert_eq!(dst_hint.tm_mday, 20);
    assert_eq!(dst_hint.tm_hour, 4);
    assert_eq!(dst_hint.tm_min, 34);
    assert_eq!(dst_hint.tm_sec, 12);
    assert_normalized_calendar_metadata(&dst_hint);
    assert_utc_baseline_output_fields(&dst_hint);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 742);
  assert_eq!(read_errno(), 961);
}

#[test]
fn mktime_positive_tm_isdst_hint_underflow_sets_erange_without_mutating_input() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 0,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: 1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn mktime_positive_tm_isdst_hint_underflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(962);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 0,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: 1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 962);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_matches_tm_isdst_one_for_underflow_error_at_tm_year_minimum() {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 0,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_one = tm {
    tm_isdst: 1,
    ..seed
  };
  let mut isdst_max = tm {
    tm_isdst: c_int::MAX,
    ..seed
  };

  write_errno(621);
  // SAFETY: pointer is valid for the duration of the call.
  let one_result = unsafe { mktime(&raw mut isdst_one) };
  let one_errno = read_errno();

  write_errno(622);
  // SAFETY: pointer is valid for the duration of the call.
  let max_result = unsafe { mktime(&raw mut isdst_max) };
  let max_errno = read_errno();

  assert_eq!(one_result, -1);
  assert_eq!(max_result, one_result);
  assert_eq!(one_errno, ERANGE);
  assert_eq!(max_errno, ERANGE);
  assert_eq!(isdst_one.tm_sec, seed.tm_sec);
  assert_eq!(isdst_one.tm_min, seed.tm_min);
  assert_eq!(isdst_one.tm_hour, seed.tm_hour);
  assert_eq!(isdst_one.tm_mday, seed.tm_mday);
  assert_eq!(isdst_one.tm_mon, seed.tm_mon);
  assert_eq!(isdst_one.tm_year, seed.tm_year);
  assert_eq!(isdst_one.tm_wday, seed.tm_wday);
  assert_eq!(isdst_one.tm_yday, seed.tm_yday);
  assert_eq!(isdst_one.tm_isdst, 1);
  assert_eq!(isdst_one.tm_gmtoff, seed.tm_gmtoff);
  assert_eq!(isdst_one.tm_zone, seed.tm_zone);
  assert_eq!(isdst_max.tm_sec, seed.tm_sec);
  assert_eq!(isdst_max.tm_min, seed.tm_min);
  assert_eq!(isdst_max.tm_hour, seed.tm_hour);
  assert_eq!(isdst_max.tm_mday, seed.tm_mday);
  assert_eq!(isdst_max.tm_mon, seed.tm_mon);
  assert_eq!(isdst_max.tm_year, seed.tm_year);
  assert_eq!(isdst_max.tm_wday, seed.tm_wday);
  assert_eq!(isdst_max.tm_yday, seed.tm_yday);
  assert_eq!(isdst_max.tm_isdst, c_int::MAX);
  assert_eq!(isdst_max.tm_gmtoff, seed.tm_gmtoff);
  assert_eq!(isdst_max.tm_zone, seed.tm_zone);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_underflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(972);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 0,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MAX,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 972);
}

#[test]
fn mktime_positive_tm_isdst_hint_can_recover_day_carry_overflow_at_tm_year_maximum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: 11,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: 1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };

  write_errno(614);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 614);
  assert_eq!(value.tm_year, c_int::MAX);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 23);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_utc_baseline_output_fields(&value);
}

#[test]
fn mktime_positive_tm_isdst_hint_recovery_path_keeps_errno_thread_local_across_threads() {
  write_errno(963);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 32,
      tm_mon: 11,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: 1,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(743);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MAX);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 0);
    assert_eq!(value.tm_sec, 0);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 743);
  assert_eq!(read_errno(), 963);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_matches_tm_isdst_one_during_day_carry_recovery_at_tm_year_maximum()
 {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: 11,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_one = tm {
    tm_isdst: 1,
    ..seed
  };
  let mut isdst_max = tm {
    tm_isdst: c_int::MAX,
    ..seed
  };

  write_errno(623);

  // SAFETY: pointer is valid for the duration of the call.
  let one_seconds = unsafe { mktime(&raw mut isdst_one) };
  // SAFETY: pointer is valid for the duration of the call.
  let max_seconds = unsafe { mktime(&raw mut isdst_max) };

  assert_ne!(one_seconds, -1);
  assert_ne!(max_seconds, -1);
  assert_eq!(max_seconds, one_seconds);
  assert_eq!(read_errno(), 623);
  assert_eq!(isdst_max, isdst_one);
  assert_normalized_calendar_metadata(&isdst_max);
  assert_utc_baseline_output_fields(&isdst_max);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_day_carry_recovery_path_keeps_errno_thread_local_across_threads()
 {
  write_errno(973);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 32,
      tm_mon: 11,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MAX,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(787);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MAX);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 0);
    assert_eq!(value.tm_sec, 0);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 787);
  assert_eq!(read_errno(), 973);
}

#[test]
fn mktime_positive_tm_isdst_hint_can_recover_month_carry_overflow_at_tm_year_maximum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 12,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: 1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };

  write_errno(966);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 966);
  assert_eq!(value.tm_year, c_int::MAX);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 23);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_utc_baseline_output_fields(&value);
}

#[test]
fn mktime_positive_tm_isdst_hint_month_carry_recovery_path_keeps_errno_thread_local_across_threads()
{
  write_errno(967);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 12,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: 1,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(782);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MAX);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 0);
    assert_eq!(value.tm_sec, 0);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 782);
  assert_eq!(read_errno(), 967);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_matches_tm_isdst_one_during_month_carry_recovery_at_tm_year_maximum()
 {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 12,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_one = tm {
    tm_isdst: 1,
    ..seed
  };
  let mut isdst_max = tm {
    tm_isdst: c_int::MAX,
    ..seed
  };

  write_errno(618);

  // SAFETY: pointer is valid for the duration of the call.
  let one_seconds = unsafe { mktime(&raw mut isdst_one) };
  // SAFETY: pointer is valid for the duration of the call.
  let max_seconds = unsafe { mktime(&raw mut isdst_max) };

  assert_ne!(one_seconds, -1);
  assert_ne!(max_seconds, -1);
  assert_eq!(max_seconds, one_seconds);
  assert_eq!(read_errno(), 618);
  assert_eq!(isdst_max, isdst_one);
  assert_normalized_calendar_metadata(&isdst_max);
  assert_utc_baseline_output_fields(&isdst_max);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_recovery_path_keeps_errno_thread_local_across_threads() {
  write_errno(969);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 12,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MAX,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(784);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MAX);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 0);
    assert_eq!(value.tm_sec, 0);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 784);
  assert_eq!(read_errno(), 969);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_matches_tm_isdst_one_during_day_borrow_recovery_at_tm_year_minimum()
 {
  let seed = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 0,
    tm_year: c_int::MIN + 1,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };
  let mut isdst_one = tm {
    tm_isdst: 1,
    ..seed
  };
  let mut isdst_max = tm {
    tm_isdst: c_int::MAX,
    ..seed
  };

  write_errno(619);

  // SAFETY: pointer is valid for the duration of the call.
  let one_seconds = unsafe { mktime(&raw mut isdst_one) };
  // SAFETY: pointer is valid for the duration of the call.
  let max_seconds = unsafe { mktime(&raw mut isdst_max) };

  assert_ne!(one_seconds, -1);
  assert_ne!(max_seconds, -1);
  assert_eq!(max_seconds, one_seconds);
  assert_eq!(read_errno(), 619);
  assert_eq!(isdst_max, isdst_one);
  assert_normalized_calendar_metadata(&isdst_max);
  assert_utc_baseline_output_fields(&isdst_max);
}

#[test]
fn mktime_max_positive_tm_isdst_hint_min_recovery_path_keeps_errno_thread_local_across_threads() {
  write_errno(970);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 0,
      tm_year: c_int::MIN + 1,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: c_int::MAX,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(785);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MIN);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 0);
    assert_eq!(value.tm_sec, 0);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 785);
  assert_eq!(read_errno(), 970);
}

#[test]
fn mktime_positive_tm_isdst_hint_can_recover_day_borrow_underflow_at_tm_year_minimum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 0,
    tm_year: c_int::MIN + 1,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: 1,
    tm_gmtoff: 17,
    tm_zone: ptr::dangling(),
  };

  write_errno(615);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 615);
  assert_eq!(value.tm_year, c_int::MIN);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 23);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_utc_baseline_output_fields(&value);
}

#[test]
fn mktime_positive_tm_isdst_hint_min_recovery_path_keeps_errno_thread_local_across_threads() {
  write_errno(964);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 0,
      tm_year: c_int::MIN + 1,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: 1,
      tm_gmtoff: 17,
      tm_zone: ptr::dangling(),
    };

    write_errno(744);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MIN);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_eq!(value.tm_hour, 23);
    assert_eq!(value.tm_min, 0);
    assert_eq!(value.tm_sec, 0);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 744);
  assert_eq!(read_errno(), 964);
}

#[test]
fn localtime_r_and_mktime_round_trip_under_utc_baseline() {
  let timestamp: time_t = 1_709_251_234;
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: pointers are valid for the duration of the call.
  let local_ptr = unsafe { localtime_r(&raw const timestamp, &raw mut out) };

  assert_eq!(local_ptr, &raw mut out);

  out.tm_isdst = -1;

  // SAFETY: pointer is valid for the duration of the call.
  let round_tripped = unsafe { mktime(&raw mut out) };

  assert_eq!(round_tripped, timestamp);
  assert_eq!(read_errno(), 0);
}

#[test]
fn timegm_reports_erange_when_normalized_year_overflows_c_int_maximum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: c_int::MAX,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn mktime_reports_erange_when_normalized_year_overflows_c_int_maximum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: c_int::MAX,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn timegm_reports_erange_when_day_carry_overflows_tm_year_maximum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: 11,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn mktime_reports_erange_when_day_carry_overflows_tm_year_maximum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: 11,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn timegm_reports_erange_when_month_carry_overflows_tm_year_maximum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 12,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn mktime_reports_erange_when_month_carry_overflows_tm_year_maximum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: 12,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn timegm_reports_erange_when_normalized_year_underflows_c_int_minimum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: c_int::MIN,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn timegm_reports_erange_when_day_borrow_underflows_tm_year_minimum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 0,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn mktime_reports_erange_when_day_borrow_underflows_tm_year_minimum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 0,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn timegm_reports_erange_when_month_borrow_underflows_tm_year_minimum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: -1,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn mktime_reports_erange_when_month_borrow_underflows_tm_year_minimum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: -1,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn mktime_reports_erange_when_normalized_year_underflows_c_int_minimum() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 1,
    tm_mon: c_int::MIN,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 42,
    tm_zone: ptr::dangling(),
  };
  let original = value;

  write_errno(0);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ERANGE);
  assert_eq!(value, original);
}

#[test]
fn timegm_erange_underflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(925);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: c_int::MIN,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 925);
}

#[test]
fn mktime_erange_underflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(926);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: c_int::MIN,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 926);
}

#[test]
fn timegm_erange_overflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(921);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: c_int::MAX,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 921);
}

#[test]
fn mktime_erange_overflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(922);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: c_int::MAX,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 922);
}

#[test]
fn timegm_day_carry_overflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(951);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 32,
      tm_mon: 11,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 951);
}

#[test]
fn mktime_day_carry_overflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(952);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 32,
      tm_mon: 11,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 952);
}

#[test]
fn timegm_day_borrow_underflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(953);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 0,
      tm_mon: 0,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 953);
}

#[test]
fn mktime_day_borrow_underflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(954);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 0,
      tm_mon: 0,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 954);
}

#[test]
fn timegm_month_carry_overflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(955);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 12,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 955);
}

#[test]
fn mktime_month_carry_overflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(956);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: 12,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 956);
}

#[test]
fn timegm_month_borrow_underflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(957);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: -1,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 957);
}

#[test]
fn mktime_month_borrow_underflow_path_keeps_errno_thread_local_across_threads() {
  write_errno(958);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 1,
      tm_mon: -1,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };
    let original = value;

    write_errno(0);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_eq!(result, -1);
    assert_eq!(value, original);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 958);
}

#[test]
fn timegm_einval_path_keeps_errno_thread_local_across_threads() {
  write_errno(923);

  let child_errno = std::thread::spawn(|| {
    write_errno(0);

    // SAFETY: null pointer is intentional for error-path contract validation.
    let result = unsafe { timegm(ptr::null_mut()) };

    assert_eq!(result, -1);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, EINVAL);
  assert_eq!(read_errno(), 923);
}

#[test]
fn mktime_einval_path_keeps_errno_thread_local_across_threads() {
  write_errno(924);

  let child_errno = std::thread::spawn(|| {
    write_errno(0);

    // SAFETY: null pointer is intentional for error-path contract validation.
    let result = unsafe { mktime(ptr::null_mut()) };

    assert_eq!(result, -1);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, EINVAL);
  assert_eq!(read_errno(), 924);
}

#[test]
fn timegm_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(927);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 12,
      tm_min: 34,
      tm_hour: 5,
      tm_mday: 20,
      tm_mon: 6,
      tm_year: 124,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };

    write_errno(732);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_ne!(result, -1);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 732);
  assert_eq!(read_errno(), 927);
}

#[test]
fn mktime_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(928);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 12,
      tm_min: 34,
      tm_hour: 5,
      tm_mday: 20,
      tm_mon: 6,
      tm_year: 124,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };

    write_errno(733);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_normalized_calendar_metadata(&value);
    assert_utc_baseline_output_fields(&value);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 733);
  assert_eq!(read_errno(), 928);
}

#[test]
fn timegm_boundary_normalization_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(981);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 32,
      tm_mon: -1,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };

    write_errno(771);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MAX);
    assert_eq!(value.tm_mon, 0);
    assert_eq!(value.tm_mday, 1);
    assert_normalized_calendar_metadata(&value);
    assert_eq!(value.tm_isdst, 0);
    assert_eq!(value.tm_gmtoff, 0);
    assert!(value.tm_zone.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 771);
  assert_eq!(read_errno(), 981);
}

#[test]
fn mktime_boundary_normalization_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(982);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 0,
      tm_mon: 12,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };

    write_errno(772);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MIN);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_normalized_calendar_metadata(&value);
    assert_eq!(value.tm_isdst, 0);
    assert_eq!(value.tm_gmtoff, 0);
    assert!(value.tm_zone.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 772);
  assert_eq!(read_errno(), 982);
}

#[test]
fn timegm_boundary_normalization_min_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(983);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 0,
      tm_mon: 12,
      tm_year: c_int::MIN,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };

    write_errno(773);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { timegm(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MIN);
    assert_eq!(value.tm_mon, 11);
    assert_eq!(value.tm_mday, 31);
    assert_normalized_calendar_metadata(&value);
    assert_eq!(value.tm_isdst, 0);
    assert_eq!(value.tm_gmtoff, 0);
    assert!(value.tm_zone.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 773);
  assert_eq!(read_errno(), 983);
}

#[test]
fn mktime_boundary_normalization_max_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(984);

  let child_errno = std::thread::spawn(|| {
    let mut value = tm {
      tm_sec: 0,
      tm_min: 0,
      tm_hour: 0,
      tm_mday: 32,
      tm_mon: -1,
      tm_year: c_int::MAX,
      tm_wday: -1,
      tm_yday: -1,
      tm_isdst: -1,
      tm_gmtoff: 42,
      tm_zone: ptr::dangling(),
    };

    write_errno(774);

    // SAFETY: pointer is valid for the duration of the call.
    let result = unsafe { mktime(&raw mut value) };

    assert_ne!(result, -1);
    assert_eq!(value.tm_year, c_int::MAX);
    assert_eq!(value.tm_mon, 0);
    assert_eq!(value.tm_mday, 1);
    assert_normalized_calendar_metadata(&value);
    assert_eq!(value.tm_isdst, 0);
    assert_eq!(value.tm_gmtoff, 0);
    assert!(value.tm_zone.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 774);
  assert_eq!(read_errno(), 984);
}

#[test]
fn timegm_normalization_can_recover_tm_year_max_with_day_borrow() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 12,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(741);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 741);
  assert_eq!(value.tm_year, c_int::MAX);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn timegm_normalization_can_recover_tm_year_max_minus_one_with_day_carry() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: 11,
    tm_year: c_int::MAX - 1,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(747);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 747);
  assert_eq!(value.tm_year, c_int::MAX);
  assert_eq!(value.tm_mon, 0);
  assert_eq!(value.tm_mday, 1);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn timegm_normalization_can_recover_tm_year_min_with_day_carry() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: -1,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(742);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 742);
  assert_eq!(value.tm_year, c_int::MIN);
  assert_eq!(value.tm_mon, 0);
  assert_eq!(value.tm_mday, 1);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn timegm_normalization_can_recover_tm_year_min_plus_one_with_day_borrow() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 0,
    tm_year: c_int::MIN + 1,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(745);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 745);
  assert_eq!(value.tm_year, c_int::MIN);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn timegm_normalization_can_recover_tm_year_max_with_month_borrow_then_day_carry() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: -1,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(761);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 761);
  assert_eq!(value.tm_year, c_int::MAX);
  assert_eq!(value.tm_mon, 0);
  assert_eq!(value.tm_mday, 1);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn timegm_normalization_can_recover_tm_year_min_with_month_carry_then_day_borrow() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 12,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(762);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { timegm(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 762);
  assert_eq!(value.tm_year, c_int::MIN);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn mktime_normalization_can_recover_tm_year_max_with_day_borrow() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 12,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(743);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 743);
  assert_eq!(value.tm_year, c_int::MAX);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn mktime_normalization_can_recover_tm_year_max_minus_one_with_day_carry() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: 11,
    tm_year: c_int::MAX - 1,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(748);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 748);
  assert_eq!(value.tm_year, c_int::MAX);
  assert_eq!(value.tm_mon, 0);
  assert_eq!(value.tm_mday, 1);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn mktime_normalization_can_recover_tm_year_min_with_day_carry() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: -1,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(744);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 744);
  assert_eq!(value.tm_year, c_int::MIN);
  assert_eq!(value.tm_mon, 0);
  assert_eq!(value.tm_mday, 1);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn mktime_normalization_can_recover_tm_year_min_plus_one_with_day_borrow() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 0,
    tm_year: c_int::MIN + 1,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(746);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 746);
  assert_eq!(value.tm_year, c_int::MIN);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn mktime_normalization_can_recover_tm_year_max_with_month_borrow_then_day_carry() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 32,
    tm_mon: -1,
    tm_year: c_int::MAX,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(763);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 763);
  assert_eq!(value.tm_year, c_int::MAX);
  assert_eq!(value.tm_mon, 0);
  assert_eq!(value.tm_mday, 1);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn mktime_normalization_can_recover_tm_year_min_with_month_carry_then_day_borrow() {
  let mut value = tm {
    tm_sec: 0,
    tm_min: 0,
    tm_hour: 0,
    tm_mday: 0,
    tm_mon: 12,
    tm_year: c_int::MIN,
    tm_wday: -1,
    tm_yday: -1,
    tm_isdst: -1,
    tm_gmtoff: 77,
    tm_zone: ptr::dangling(),
  };

  write_errno(764);

  // SAFETY: pointer is valid for the duration of the call.
  let result = unsafe { mktime(&raw mut value) };

  assert_ne!(result, -1);
  assert_eq!(read_errno(), 764);
  assert_eq!(value.tm_year, c_int::MIN);
  assert_eq!(value.tm_mon, 11);
  assert_eq!(value.tm_mday, 31);
  assert_eq!(value.tm_hour, 0);
  assert_eq!(value.tm_min, 0);
  assert_eq!(value.tm_sec, 0);
  assert_normalized_calendar_metadata(&value);
  assert_eq!(value.tm_isdst, 0);
  assert_eq!(value.tm_gmtoff, 0);
  assert!(value.tm_zone.is_null());
}

#[test]
fn localtime_r_localtime_and_mktime_report_einval_for_null_inputs() {
  let mut out = zero_tm();

  write_errno(0);

  // SAFETY: null `timer` is intentional to validate error handling.
  let localtime_r_null_timer = unsafe { localtime_r(ptr::null(), &raw mut out) };

  assert!(localtime_r_null_timer.is_null());
  assert_eq!(read_errno(), EINVAL);

  let timer: time_t = 0;

  write_errno(0);

  // SAFETY: null `result` is intentional to validate error handling.
  let localtime_r_null_result = unsafe { localtime_r(&raw const timer, ptr::null_mut()) };

  assert!(localtime_r_null_result.is_null());
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  // SAFETY: null `timer` is intentional to validate error handling.
  let localtime_null_timer = unsafe { localtime(ptr::null()) };

  assert!(localtime_null_timer.is_null());
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  // SAFETY: null `tm` is intentional to validate error handling.
  let mktime_null_input = unsafe { mktime(ptr::null_mut()) };

  assert_eq!(mktime_null_input, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn localtime_r_error_path_keeps_errno_thread_local_across_threads() {
  write_errno(902);

  let child_errno = std::thread::spawn(|| {
    let mut out = zero_tm();

    write_errno(0);

    // SAFETY: null `timer` is intentional for error-path contract validation.
    let result_ptr = unsafe { localtime_r(ptr::null(), &raw mut out) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, EINVAL);
  assert_eq!(read_errno(), 902);
}

#[test]
fn localtime_r_erange_path_keeps_errno_thread_local_across_threads() {
  write_errno(912);

  let child_errno = std::thread::spawn(|| {
    let timer = time_t::MAX;
    let mut out = zero_tm();

    write_errno(0);

    // SAFETY: pointers are valid and `time_t::MAX` exercises ERANGE path.
    let result_ptr = unsafe { localtime_r(&raw const timer, &raw mut out) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 912);
}

#[test]
fn localtime_r_erange_minimum_path_keeps_errno_thread_local_across_threads() {
  write_errno(914);

  let child_errno = std::thread::spawn(|| {
    let timer = time_t::MIN;
    let mut out = zero_tm();

    write_errno(0);

    // SAFETY: pointers are valid and `time_t::MIN` exercises ERANGE path.
    let result_ptr = unsafe { localtime_r(&raw const timer, &raw mut out) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, ERANGE);
  assert_eq!(read_errno(), 914);
}

#[test]
fn localtime_r_null_result_short_circuits_before_timer_read() {
  let invalid_timer: *const time_t = ptr::dangling();

  write_errno(0);

  // SAFETY: null `result` is intentional; this validates short-circuit before
  // reading an invalid non-null `timer`.
  let localtime_r_result = unsafe { localtime_r(invalid_timer, ptr::null_mut()) };

  assert!(localtime_r_result.is_null());
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn localtime_r_null_result_path_keeps_errno_thread_local_across_threads() {
  write_errno(916);

  let child_errno = std::thread::spawn(|| {
    let invalid_timer: *const time_t = ptr::dangling();

    write_errno(0);

    // SAFETY: null `result` is intentional; this validates short-circuit before
    // reading an invalid non-null `timer`.
    let result_ptr = unsafe { localtime_r(invalid_timer, ptr::null_mut()) };

    assert!(result_ptr.is_null());

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, EINVAL);
  assert_eq!(read_errno(), 916);
}

#[test]
fn localtime_r_success_path_keeps_errno_thread_local_across_threads() {
  write_errno(918);

  let child_errno = std::thread::spawn(|| {
    let timer: time_t = 172_800;
    let mut out = zero_tm();

    write_errno(734);

    // SAFETY: pointers are valid for the duration of the call.
    let result_ptr = unsafe { localtime_r(&raw const timer, &raw mut out) };

    assert_eq!(result_ptr, &raw mut out);
    assert_eq!(out.tm_mday, 3);
    assert_normalized_calendar_metadata(&out);
    assert_utc_baseline_output_fields(&out);

    read_errno()
  })
  .join()
  .expect("child thread should not panic");

  assert_eq!(child_errno, 734);
  assert_eq!(read_errno(), 918);
}

#[test]
fn localtime_r_einval_null_timer_does_not_mutate_output_buffer() {
  let mut out = tm {
    tm_sec: 27,
    tm_min: 26,
    tm_hour: 25,
    tm_mday: 24,
    tm_mon: 23,
    tm_year: 22,
    tm_wday: 21,
    tm_yday: 20,
    tm_isdst: -1,
    tm_gmtoff: 654,
    tm_zone: ptr::dangling(),
  };
  let original = out;

  write_errno(0);

  // SAFETY: null `timer` is intentional for error-path contract validation.
  let result_ptr = unsafe { localtime_r(ptr::null(), &raw mut out) };

  assert!(result_ptr.is_null());
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(out, original);
}
