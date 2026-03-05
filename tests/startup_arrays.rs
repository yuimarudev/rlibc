use core::ptr;
use rlibc::startup::{InitFiniFn, run_fini_array_range, run_init_array_range};
use std::sync::{Mutex, MutexGuard, OnceLock};

static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static CALL_LOG: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();

fn test_lock() -> &'static Mutex<()> {
  TEST_LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_test() -> MutexGuard<'static, ()> {
  match test_lock().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn log() -> &'static Mutex<Vec<u8>> {
  CALL_LOG.get_or_init(|| Mutex::new(Vec::new()))
}

fn reset_log() {
  let mut guard = match log().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  };

  guard.clear();
}

fn push_call(marker: u8) {
  let mut guard = match log().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  };

  guard.push(marker);
}

fn snapshot_log() -> Vec<u8> {
  let guard = match log().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  };

  guard.clone()
}

fn byte_ptr_as_init_fini(ptr: *const u8) -> *const InitFiniFn {
  ptr.addr() as *const InitFiniFn
}

unsafe extern "C" fn first() {
  push_call(1);
}

unsafe extern "C" fn second() {
  push_call(2);
}

unsafe extern "C" fn third() {
  push_call(3);
}

#[test]
fn init_array_runs_in_forward_order() {
  let _test_guard = lock_test();

  reset_log();

  let init_entries: [InitFiniFn; 3] = [first, second, third];
  // SAFETY: `init_entries` is a valid contiguous range of constructors.
  unsafe {
    run_init_array_range(
      init_entries.as_ptr(),
      init_entries.as_ptr().add(init_entries.len()),
    );
  }

  assert_eq!(snapshot_log(), vec![1, 2, 3]);
}

#[test]
fn fini_array_runs_in_reverse_order() {
  let _test_guard = lock_test();

  reset_log();

  let fini_entries: [InitFiniFn; 3] = [first, second, third];
  // SAFETY: `fini_entries` is a valid contiguous range of destructors.
  unsafe {
    run_fini_array_range(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().add(fini_entries.len()),
    );
  }

  assert_eq!(snapshot_log(), vec![3, 2, 1]);
}

#[test]
fn empty_ranges_do_not_invoke_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 1] = [first];
  // SAFETY: both pointers are the same, representing an empty range.
  unsafe {
    run_init_array_range(entries.as_ptr(), entries.as_ptr());
    run_fini_array_range(entries.as_ptr(), entries.as_ptr());
  }

  assert!(snapshot_log().is_empty());
}

#[test]
fn reversed_ranges_do_not_invoke_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 3] = [first, second, third];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`, and adding `entries.len()` yields
  // the canonical one-past-the-end pointer for the same allocation.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: `start` and `end` come from one valid contiguous array allocation.
  unsafe {
    run_init_array_range(end, start);
    run_fini_array_range(end, start);
  }

  assert!(snapshot_log().is_empty());
}

#[test]
fn misaligned_ranges_do_not_invoke_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: `base` comes from `raw` and these byte offsets remain in-bounds.
  let start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `start`; intentionally misaligned.
  let end = byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: I006 defensive contract treats misaligned ranges as empty/no-op.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert!(snapshot_log().is_empty());
}

#[test]
fn oversized_distance_ranges_do_not_invoke_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let start_addr = core::mem::align_of::<InitFiniFn>() * 2;
  let oversized_distance = (isize::MAX as usize) + 1;

  assert_eq!(oversized_distance % core::mem::size_of::<InitFiniFn>(), 0);

  let end_addr = start_addr + oversized_distance;
  let start = start_addr as *const InitFiniFn;
  let end = end_addr as *const InitFiniFn;

  // SAFETY: oversized distance is rejected by defensive range checks and should no-op.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert!(snapshot_log().is_empty());
}

#[test]
fn null_endpoint_ranges_do_not_invoke_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let aligned_non_null = core::mem::align_of::<InitFiniFn>() * 2;
  let fake_aligned = aligned_non_null as *const InitFiniFn;

  // SAFETY: I006 defensive contract treats null-ended ranges as empty/no-op.
  unsafe {
    run_init_array_range(ptr::null(), fake_aligned);
    run_fini_array_range(ptr::null(), fake_aligned);
    run_init_array_range(fake_aligned, ptr::null());
    run_fini_array_range(fake_aligned, ptr::null());
  }

  assert!(snapshot_log().is_empty());
}

#[test]
fn partially_null_ranges_do_not_invoke_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: I006 defensive contract treats ranges with one null endpoint as empty/no-op.
  unsafe {
    run_init_array_range(start, ptr::null());
    run_fini_array_range(start, ptr::null());
    run_init_array_range(ptr::null(), end);
    run_fini_array_range(ptr::null(), end);
  }

  assert!(snapshot_log().is_empty());
}

#[test]
fn null_entries_in_valid_ranges_are_skipped() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [Option<InitFiniFn>; 3] = [Some(first), None, Some(second)];

  assert_eq!(
    core::mem::size_of::<Option<InitFiniFn>>(),
    core::mem::size_of::<InitFiniFn>(),
  );
  assert_eq!(
    core::mem::align_of::<Option<InitFiniFn>>(),
    core::mem::align_of::<InitFiniFn>(),
  );

  let start = entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `start` points to contiguous pointer-sized entries.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: range shape is valid; null entries should be skipped defensively.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn null_entries_at_range_edges_are_skipped() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [Option<InitFiniFn>; 4] = [None, Some(first), Some(second), None];

  assert_eq!(
    core::mem::size_of::<Option<InitFiniFn>>(),
    core::mem::size_of::<InitFiniFn>(),
  );
  assert_eq!(
    core::mem::align_of::<Option<InitFiniFn>>(),
    core::mem::align_of::<InitFiniFn>(),
  );

  let start = entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `start` points to contiguous pointer-sized entries.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: range shape is valid; null edge entries should be skipped defensively.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn misaligned_non_null_entries_in_valid_ranges_are_skipped() {
  let _test_guard = lock_test();

  reset_log();

  let entry_align = core::mem::align_of::<InitFiniFn>();

  if entry_align == 1 {
    return;
  }

  let entries: [usize; 3] = [1, first as *const () as usize, second as *const () as usize];
  let start = entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `start` points to contiguous pointer-sized entries.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: range shape is valid; misaligned non-null slots should be skipped.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn misaligned_non_null_entries_at_range_edges_are_skipped() {
  let _test_guard = lock_test();

  reset_log();

  let entry_align = core::mem::align_of::<InitFiniFn>();

  if entry_align == 1 {
    return;
  }

  let entries: [usize; 4] = [
    first as *const () as usize,
    second as *const () as usize,
    first as *const () as usize,
    1,
  ];
  let start = entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `start` points to contiguous pointer-sized entries.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: range shape is valid; misaligned non-null edge slots should be skipped.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 1, 1, 2, 1]);
}

#[test]
fn mixed_null_and_misaligned_non_null_entries_are_skipped() {
  let _test_guard = lock_test();

  reset_log();

  let entry_align = core::mem::align_of::<InitFiniFn>();

  if entry_align == 1 {
    return;
  }

  let entries: [usize; 6] = [
    0,
    1,
    first as *const () as usize,
    second as *const () as usize,
    1,
    0,
  ];
  let start = entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `start` points to contiguous pointer-sized entries.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: range shape is valid; null and misaligned non-null slots should be skipped.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn interleaved_null_and_misaligned_non_null_entries_are_skipped() {
  let _test_guard = lock_test();

  reset_log();

  let entry_align = core::mem::align_of::<InitFiniFn>();

  if entry_align == 1 {
    return;
  }

  let init_entries: [usize; 5] = [
    first as *const () as usize,
    0,
    1,
    second as *const () as usize,
    0,
  ];
  let fini_entries: [usize; 5] = [
    second as *const () as usize,
    0,
    1,
    first as *const () as usize,
    0,
  ];
  let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `init_start` points to contiguous pointer-sized entries.
  let init_end = unsafe { init_start.add(init_entries.len()) };
  let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `fini_start` points to contiguous pointer-sized entries.
  let fini_end = unsafe { fini_start.add(fini_entries.len()) };

  // SAFETY: range shapes are valid; interleaved null and misaligned non-null
  // slots should be skipped defensively.
  unsafe {
    run_init_array_range(init_start, init_end);
    run_fini_array_range(fini_start, fini_end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 1, 2]);
}

#[test]
fn repeated_valid_entries_with_mixed_skips_preserve_order() {
  let _test_guard = lock_test();

  reset_log();

  let entry_align = core::mem::align_of::<InitFiniFn>();

  if entry_align == 1 {
    return;
  }

  let entries: [usize; 5] = [
    first as *const () as usize,
    0,
    first as *const () as usize,
    1,
    second as *const () as usize,
  ];
  let start = entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `start` points to contiguous pointer-sized entries.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: range shape is valid; null and misaligned non-null slots should
  // be skipped while repeated valid entries preserve order.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 1, 2, 2, 1, 1]);
}

#[test]
fn alternating_repeated_entries_with_mixed_skips_preserve_order() {
  let _test_guard = lock_test();

  reset_log();

  let entry_align = core::mem::align_of::<InitFiniFn>();

  if entry_align == 1 {
    return;
  }

  let entries: [usize; 8] = [
    first as *const () as usize,
    0,
    1,
    second as *const () as usize,
    first as *const () as usize,
    1,
    second as *const () as usize,
    0,
  ];
  let start = entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `start` points to contiguous pointer-sized entries.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: range shape is valid; null and misaligned non-null slots should
  // be skipped while alternating repeated valid entries preserve order.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 1, 2, 2, 1, 2, 1]);
}

#[test]
fn repeated_valid_entries_with_mixed_skips_support_distinct_init_and_fini_ranges() {
  let _test_guard = lock_test();

  reset_log();

  let entry_align = core::mem::align_of::<InitFiniFn>();

  if entry_align == 1 {
    return;
  }

  let init_entries: [usize; 5] = [
    first as *const () as usize,
    0,
    first as *const () as usize,
    1,
    third as *const () as usize,
  ];
  let fini_entries: [usize; 5] = [
    third as *const () as usize,
    0,
    second as *const () as usize,
    1,
    second as *const () as usize,
  ];
  let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `init_start` points to contiguous pointer-sized entries.
  let init_end = unsafe { init_start.add(init_entries.len()) };
  let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
  // SAFETY: `fini_start` points to contiguous pointer-sized entries.
  let fini_end = unsafe { fini_start.add(fini_entries.len()) };

  // SAFETY: range shapes are valid; null and misaligned non-null slots should
  // be skipped while preserving repeated valid entries in each distinct range.
  unsafe {
    run_init_array_range(init_start, init_end);
    run_fini_array_range(fini_start, fini_end);
  }

  assert_eq!(snapshot_log(), vec![1, 1, 3, 2, 2, 3]);
}

#[test]
fn valid_init_with_partially_null_fini_only_runs_init_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: init range is valid; fini range has null endpoint and is no-op.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, ptr::null());
  }

  assert_eq!(snapshot_log(), vec![1, 2]);
}

#[test]
fn valid_init_with_null_start_fini_only_runs_init_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: init range is valid; fini range has null start and is no-op.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(ptr::null(), end);
  }

  assert_eq!(snapshot_log(), vec![1, 2]);
}

#[test]
fn valid_init_with_reversed_fini_only_runs_init_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: init range is valid; reversed fini range is no-op.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(end, start);
  }

  assert_eq!(snapshot_log(), vec![1, 2]);
}

#[test]
fn valid_init_with_misaligned_fini_only_runs_init_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: init range is valid; misaligned fini range is no-op.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(misaligned_start, misaligned_end);
  }

  assert_eq!(snapshot_log(), vec![1, 2]);
}

#[test]
fn valid_init_with_oversized_fini_only_runs_init_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let oversized_start_addr = core::mem::align_of::<InitFiniFn>() * 2;
  let oversized_distance = (isize::MAX as usize) + 1;

  assert_eq!(oversized_distance % core::mem::size_of::<InitFiniFn>(), 0);

  let oversized_end_addr = oversized_start_addr + oversized_distance;
  let oversized_start = oversized_start_addr as *const InitFiniFn;
  let oversized_end = oversized_end_addr as *const InitFiniFn;

  // SAFETY: init range is valid; oversized fini range is rejected and no-op.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(oversized_start, oversized_end);
  }

  assert_eq!(snapshot_log(), vec![1, 2]);
}

#[test]
fn valid_fini_with_oversized_init_only_runs_fini_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let oversized_start_addr = core::mem::align_of::<InitFiniFn>() * 2;
  let oversized_distance = (isize::MAX as usize) + 1;

  assert_eq!(oversized_distance % core::mem::size_of::<InitFiniFn>(), 0);

  let oversized_end_addr = oversized_start_addr + oversized_distance;
  let oversized_start = oversized_start_addr as *const InitFiniFn;
  let oversized_end = oversized_end_addr as *const InitFiniFn;

  // SAFETY: oversized init range is rejected and no-op; fini range is valid.
  unsafe {
    run_init_array_range(oversized_start, oversized_end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![2, 1]);
}

#[test]
fn valid_fini_with_partially_null_init_only_runs_fini_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: init range has null endpoint and is no-op; fini range is valid.
  unsafe {
    run_init_array_range(start, ptr::null());
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![2, 1]);
}

#[test]
fn valid_fini_with_null_start_init_only_runs_fini_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: null-start init range is no-op; fini range is valid.
  unsafe {
    run_init_array_range(ptr::null(), end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![2, 1]);
}

#[test]
fn valid_fini_with_empty_init_only_runs_fini_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: empty init range is no-op; fini range is valid.
  unsafe {
    run_init_array_range(start, start);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![2, 1]);
}

#[test]
fn valid_fini_with_reversed_init_only_runs_fini_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: reversed init range is no-op; fini range is valid.
  unsafe {
    run_init_array_range(end, start);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![2, 1]);
}

#[test]
fn valid_fini_with_misaligned_init_only_runs_fini_handlers() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: misaligned init range is no-op; fini range is valid.
  unsafe {
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![2, 1]);
}

#[test]
fn mixed_valid_and_partially_null_sequences_preserve_expected_order() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: valid ranges should execute; partial-null ranges should no-op.
  unsafe {
    run_init_array_range(start, end);
    run_init_array_range(start, ptr::null());
    run_fini_array_range(start, end);
    run_fini_array_range(ptr::null(), end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn mixed_valid_and_reversed_sequences_preserve_expected_order() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: valid ranges should execute; reversed ranges should no-op.
  unsafe {
    run_init_array_range(start, end);
    run_init_array_range(end, start);
    run_fini_array_range(start, end);
    run_fini_array_range(end, start);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn mixed_valid_and_null_endpoint_sequences_preserve_expected_order() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: valid ranges should execute; null-endpoint ranges should no-op.
  unsafe {
    run_init_array_range(start, end);
    run_init_array_range(ptr::null(), end);
    run_fini_array_range(start, end);
    run_fini_array_range(start, ptr::null());
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn mixed_valid_and_empty_sequences_preserve_expected_order() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };

  // SAFETY: valid ranges should execute; empty ranges should no-op.
  unsafe {
    run_init_array_range(start, end);
    run_init_array_range(start, start);
    run_fini_array_range(start, end);
    run_fini_array_range(start, start);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn mixed_valid_and_misaligned_sequences_preserve_expected_order() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: valid ranges should execute; misaligned ranges should no-op.
  unsafe {
    run_init_array_range(start, end);
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, end);
    run_fini_array_range(misaligned_start, misaligned_end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn noop_sequences_do_not_affect_subsequent_valid_runs() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: defensive no-op paths should not interfere with later valid ranges.
  unsafe {
    run_init_array_range(ptr::null(), end);
    run_init_array_range(end, start);
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, ptr::null());
    run_fini_array_range(end, start);
    run_fini_array_range(misaligned_start, misaligned_end);
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn noop_sequences_do_not_affect_repeated_valid_runs() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: defensive no-op paths should not interfere with repeated valid ranges.
  unsafe {
    run_init_array_range(ptr::null(), end);
    run_fini_array_range(start, ptr::null());
    run_init_array_range(end, start);
    run_fini_array_range(end, start);
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(misaligned_start, misaligned_end);
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1, 1, 2, 2, 1]);
}

#[test]
fn all_noop_variants_do_not_affect_valid_sequences() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: all no-op variants should not interfere with later valid ranges.
  unsafe {
    run_init_array_range(start, start);
    run_init_array_range(ptr::null(), end);
    run_init_array_range(end, start);
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, start);
    run_fini_array_range(start, ptr::null());
    run_fini_array_range(end, start);
    run_fini_array_range(misaligned_start, misaligned_end);
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1]);
}

#[test]
fn all_noop_variants_do_not_affect_repeated_valid_sequences() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 2] = [first, second];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: no-op variants should not interfere with repeated valid traversals.
  unsafe {
    run_init_array_range(start, start);
    run_init_array_range(ptr::null(), end);
    run_init_array_range(end, start);
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, start);
    run_fini_array_range(start, ptr::null());
    run_fini_array_range(end, start);
    run_fini_array_range(misaligned_start, misaligned_end);
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 2, 1, 1, 2, 2, 1]);
}

#[test]
fn noop_sequences_preserve_three_entry_ordering() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 3] = [first, second, third];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: no-op variants should not alter ordering of subsequent valid traversals.
  unsafe {
    run_init_array_range(ptr::null(), end);
    run_init_array_range(end, start);
    run_init_array_range(misaligned_start, misaligned_end);
    run_init_array_range(start, start);
    run_init_array_range(start, end);

    run_fini_array_range(start, ptr::null());
    run_fini_array_range(end, start);
    run_fini_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, start);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 3, 3, 2, 1]);
}

#[test]
fn interleaved_noop_and_valid_runs_preserve_three_entry_ordering() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 3] = [first, second, third];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: no-op variants should not affect interleaved valid traversals.
  unsafe {
    run_init_array_range(start, end);
    run_init_array_range(ptr::null(), end);
    run_init_array_range(end, start);
    run_init_array_range(start, end);
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, end);
    run_fini_array_range(start, ptr::null());
    run_fini_array_range(end, start);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 3, 1, 2, 3, 3, 2, 1, 3, 2, 1]);
}

#[test]
fn fini_first_with_noop_variants_then_init_preserves_three_entry_ordering() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 3] = [first, second, third];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: no-op variants should not affect ordering of valid traversals.
  unsafe {
    run_fini_array_range(start, end);
    run_fini_array_range(start, ptr::null());
    run_fini_array_range(end, start);
    run_fini_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, start);

    run_init_array_range(ptr::null(), end);
    run_init_array_range(end, start);
    run_init_array_range(misaligned_start, misaligned_end);
    run_init_array_range(start, start);
    run_init_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![3, 2, 1, 1, 2, 3]);
}

#[test]
fn interleaved_valid_and_noop_rounds_remain_deterministic() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 3] = [first, second, third];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: no-op rounds should not perturb valid traversal rounds.
  unsafe {
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
    run_init_array_range(ptr::null(), end);
    run_fini_array_range(start, ptr::null());
    run_init_array_range(end, start);
    run_fini_array_range(end, start);
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(misaligned_start, misaligned_end);
    run_init_array_range(start, end);
    run_fini_array_range(start, end);
  }

  assert_eq!(snapshot_log(), vec![1, 2, 3, 3, 2, 1, 1, 2, 3, 3, 2, 1]);
}

#[test]
fn repeated_noop_variants_keep_log_empty() {
  let _test_guard = lock_test();

  reset_log();

  let entries: [InitFiniFn; 3] = [first, second, third];
  let start = entries.as_ptr();
  // SAFETY: `start` points into `entries`; adding `entries.len()` yields one-past-end.
  let end = unsafe { start.add(entries.len()) };
  let raw = [0_u8; core::mem::size_of::<InitFiniFn>() + 2];
  let base = raw.as_ptr();
  // SAFETY: these offsets are in-bounds of `raw` and intentionally misaligned.
  let misaligned_start = byte_ptr_as_init_fini(unsafe { base.add(1) });
  // SAFETY: same allocation/provenance as `misaligned_start`.
  let misaligned_end =
    byte_ptr_as_init_fini(unsafe { base.add(1 + core::mem::size_of::<InitFiniFn>()) });

  // SAFETY: all variants below are defensive no-op inputs.
  unsafe {
    run_init_array_range(start, start);
    run_init_array_range(ptr::null(), end);
    run_init_array_range(end, start);
    run_init_array_range(misaligned_start, misaligned_end);
    run_fini_array_range(start, start);
    run_fini_array_range(start, ptr::null());
    run_fini_array_range(end, start);
    run_fini_array_range(misaligned_start, misaligned_end);

    run_init_array_range(start, start);
    run_init_array_range(ptr::null(), end);
    run_fini_array_range(start, ptr::null());
    run_fini_array_range(end, start);
  }

  assert!(snapshot_log().is_empty());
}
