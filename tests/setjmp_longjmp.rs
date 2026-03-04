#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_int;
use core::mem::MaybeUninit;
use rlibc::errno::__errno_location;
use rlibc::setjmp::{jmp_buf, longjmp, setjmp};

#[inline(never)]
unsafe fn jump_once(value: c_int) -> c_int {
  let mut env = MaybeUninit::<jmp_buf>::uninit();
  // SAFETY: `env` points to writable `jmp_buf` storage for this frame.
  let result = unsafe { setjmp(env.as_mut_ptr()) };

  if result == 0 {
    // SAFETY: `env` was initialized by `setjmp` above and this frame is alive.
    unsafe { longjmp(env.as_ptr(), value) };
  }

  result
}

#[inline(never)]
unsafe fn outer_jump_target() -> c_int {
  let mut outer_env = MaybeUninit::<jmp_buf>::uninit();
  // SAFETY: `outer_env` points to writable `jmp_buf` storage for this frame.
  let outer_result = unsafe { setjmp(outer_env.as_mut_ptr()) };

  if outer_result == 0 {
    // SAFETY: `outer_env` was initialized by `setjmp` above and this frame is alive.
    unsafe { trigger_inner_then_jump_outer(outer_env.as_ptr()) };
  }

  outer_result
}

#[inline(never)]
unsafe fn trigger_inner_then_jump_outer(outer_env: *const jmp_buf) {
  let mut inner_env = MaybeUninit::<jmp_buf>::uninit();
  // SAFETY: `inner_env` points to writable `jmp_buf` storage for this frame.
  let inner_result = unsafe { setjmp(inner_env.as_mut_ptr()) };

  if inner_result == 0 {
    // SAFETY: `inner_env` was initialized by `setjmp` above and this frame is alive.
    unsafe { longjmp(inner_env.as_ptr(), 33) };
  }

  if inner_result == 33 {
    // SAFETY: `outer_env` points to an outer frame that is still alive.
    unsafe { longjmp(outer_env, 11) };
  }
}

fn read_errno() -> c_int {
  let errno_ptr = __errno_location();

  assert!(
    !errno_ptr.is_null(),
    "__errno_location must return non-null pointer",
  );

  // SAFETY: pointer was checked for null and points to calling-thread errno storage.
  unsafe { errno_ptr.read() }
}

fn write_errno(value: c_int) {
  let errno_ptr = __errno_location();

  assert!(
    !errno_ptr.is_null(),
    "__errno_location must return non-null pointer",
  );

  // SAFETY: pointer was checked for null and points to writable thread-local errno.
  unsafe {
    errno_ptr.write(value);
  }
}

#[inline(never)]
unsafe fn outer_jump_target_with_values(inner_value: c_int, outer_value: c_int) -> c_int {
  let mut outer_env = MaybeUninit::<jmp_buf>::uninit();
  // SAFETY: `outer_env` points to writable `jmp_buf` storage for this frame.
  let outer_result = unsafe { setjmp(outer_env.as_mut_ptr()) };

  if outer_result == 0 {
    // SAFETY: `outer_env` was initialized by `setjmp` above and this frame is alive.
    unsafe {
      trigger_inner_then_jump_outer_with_values(outer_env.as_ptr(), inner_value, outer_value);
    };
  }

  outer_result
}

#[inline(never)]
unsafe fn trigger_inner_then_jump_outer_with_values(
  outer_env: *const jmp_buf,
  inner_value: c_int,
  outer_value: c_int,
) {
  let mut inner_env = MaybeUninit::<jmp_buf>::uninit();
  // SAFETY: `inner_env` points to writable `jmp_buf` storage for this frame.
  let inner_result = unsafe { setjmp(inner_env.as_mut_ptr()) };

  if inner_result == 0 {
    // SAFETY: `inner_env` was initialized by `setjmp` above and this frame is alive.
    unsafe { longjmp(inner_env.as_ptr(), inner_value) };
  }

  let expected_inner_result = if inner_value == 0 { 1 } else { inner_value };

  if inner_result == expected_inner_result {
    // SAFETY: `outer_env` points to an outer frame that is still alive.
    unsafe { longjmp(outer_env, outer_value) };
  }
}

#[inline(never)]
unsafe fn jump_once_preserving_errno(initial_errno: c_int, value: c_int) -> (c_int, c_int) {
  write_errno(initial_errno);

  let mut env = MaybeUninit::<jmp_buf>::uninit();
  // SAFETY: `env` points to writable `jmp_buf` storage for this frame.
  let result = unsafe { setjmp(env.as_mut_ptr()) };

  if result == 0 {
    assert_eq!(read_errno(), initial_errno);
    // SAFETY: `env` was initialized by `setjmp` above and this frame is alive.
    unsafe { longjmp(env.as_ptr(), value) };
  }

  (result, read_errno())
}

#[test]
fn setjmp_returns_zero_on_direct_path() {
  let mut env = MaybeUninit::<jmp_buf>::uninit();
  // SAFETY: `env` points to writable `jmp_buf` storage for this frame.
  let result = unsafe { setjmp(env.as_mut_ptr()) };

  assert_eq!(result, 0);
}

#[test]
fn setjmp_does_not_mutate_errno_on_direct_path() {
  write_errno(41);

  let mut env = MaybeUninit::<jmp_buf>::uninit();
  // SAFETY: `env` points to writable `jmp_buf` storage for this frame.
  let result = unsafe { setjmp(env.as_mut_ptr()) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 41);
}

#[test]
fn longjmp_returns_non_zero_value_to_saved_point() {
  // SAFETY: helper function owns the frame-local jump buffer it uses.
  let result = unsafe { jump_once(7) };

  assert_eq!(result, 7);
}

#[test]
fn longjmp_negative_non_zero_value_is_preserved() {
  // SAFETY: helper function owns the frame-local jump buffer it uses.
  let (result, errno_value) = unsafe { jump_once_preserving_errno(91, -13) };

  assert_eq!(result, -13);
  assert_eq!(errno_value, 91);
}

#[test]
fn longjmp_minimum_int_value_is_preserved() {
  // SAFETY: helper function owns the frame-local jump buffer it uses.
  let (result, errno_value) = unsafe { jump_once_preserving_errno(92, c_int::MIN) };

  assert_eq!(result, c_int::MIN);
  assert_eq!(errno_value, 92);
}

#[test]
fn longjmp_maximum_int_value_is_preserved() {
  // SAFETY: helper function owns the frame-local jump buffer it uses.
  let (result, errno_value) = unsafe { jump_once_preserving_errno(93, c_int::MAX) };

  assert_eq!(result, c_int::MAX);
  assert_eq!(errno_value, 93);
}

#[test]
fn longjmp_value_zero_is_translated_to_one() {
  // SAFETY: helper function owns the frame-local jump buffer it uses.
  let result = unsafe { jump_once(0) };

  assert_eq!(result, 1);
}

#[test]
fn longjmp_does_not_mutate_errno_on_resume_path() {
  // SAFETY: helper function owns the frame-local jump buffer it uses.
  let (result, errno_value) = unsafe { jump_once_preserving_errno(77, 9) };

  assert_eq!(result, 9);
  assert_eq!(errno_value, 77);
}

#[test]
fn longjmp_zero_translation_does_not_mutate_errno() {
  // SAFETY: helper function owns the frame-local jump buffer it uses.
  let (result, errno_value) = unsafe { jump_once_preserving_errno(88, 0) };

  assert_eq!(result, 1);
  assert_eq!(errno_value, 88);
}

#[test]
fn longjmp_restores_the_target_frame_when_nested() {
  // SAFETY: helper functions only jump within frames that remain alive.
  let result = unsafe { outer_jump_target() };

  assert_eq!(result, 11);
}

#[test]
fn nested_longjmp_zero_value_is_translated_to_one() {
  // SAFETY: helper functions only jump within frames that remain alive.
  let result = unsafe { outer_jump_target_with_values(33, 0) };

  assert_eq!(result, 1);
}

#[test]
fn nested_longjmp_preserves_errno_across_two_jumps() {
  write_errno(109);
  // SAFETY: helper functions only jump within frames that remain alive.
  let result = unsafe { outer_jump_target_with_values(-27, 44) };

  assert_eq!(result, 44);
  assert_eq!(read_errno(), 109);
}

#[test]
fn nested_inner_zero_translation_still_reaches_outer_target() {
  write_errno(121);
  // SAFETY: helper functions only jump within frames that remain alive.
  let result = unsafe { outer_jump_target_with_values(0, -58) };

  assert_eq!(result, -58);
  assert_eq!(read_errno(), 121);
}

#[test]
fn nested_both_zero_values_translate_to_one_and_preserve_errno() {
  write_errno(122);
  // SAFETY: helper functions only jump within frames that remain alive.
  let result = unsafe { outer_jump_target_with_values(0, 0) };

  assert_eq!(result, 1);
  assert_eq!(read_errno(), 122);
}

#[test]
fn nested_outer_value_contract_handles_extremes_and_zero() {
  for (index, (outer_value, expected)) in [
    (c_int::MAX, c_int::MAX),
    (c_int::MIN, c_int::MIN),
    (-1, -1),
    (0, 1),
  ]
  .into_iter()
  .enumerate()
  {
    let cycle_index = c_int::try_from(index).expect("index must fit in c_int");
    let initial_errno = 130 + cycle_index;

    write_errno(initial_errno);

    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(33, outer_value) };

    assert_eq!(result, expected);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn nested_inner_value_contract_handles_extremes_and_zero() {
  for (index, (inner_value, expected_outer)) in
    [(c_int::MAX, 9), (c_int::MIN, -5), (-1, 17), (0, 23)]
      .into_iter()
      .enumerate()
  {
    let cycle_index = c_int::try_from(index).expect("index must fit in c_int");
    let initial_errno = 140 + cycle_index;

    write_errno(initial_errno);

    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(inner_value, expected_outer) };

    assert_eq!(result, expected_outer);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn nested_value_matrix_preserves_outer_contract_and_errno() {
  for (index, (inner_value, outer_value)) in [
    (1, 2),
    (-9, 0),
    (c_int::MAX, c_int::MIN),
    (c_int::MIN, c_int::MAX),
    (0, -11),
  ]
  .into_iter()
  .enumerate()
  {
    let cycle_index = c_int::try_from(index).expect("index must fit in c_int");
    let initial_errno = 150 + cycle_index;

    write_errno(initial_errno);

    let expected_outer = if outer_value == 0 { 1 } else { outer_value };
    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

    assert_eq!(result, expected_outer);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn nested_value_matrix_is_repeatable_across_multiple_passes() {
  for pass in 0usize..2usize {
    for (case_index, (inner_value, outer_value, expected_outer)) in [
      (0, 0, 1),
      (1, 0, 1),
      (-3, 7, 7),
      (c_int::MIN, c_int::MAX, c_int::MAX),
      (c_int::MAX, c_int::MIN, c_int::MIN),
    ]
    .into_iter()
    .enumerate()
    {
      let pass_index = c_int::try_from(pass).expect("pass index must fit in c_int");
      let case_index = c_int::try_from(case_index).expect("case index must fit in c_int");
      let initial_errno = 170 + pass_index * 10 + case_index;

      write_errno(initial_errno);

      // SAFETY: helper functions only jump within frames that remain alive.
      let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

      assert_eq!(result, expected_outer);
      assert_eq!(read_errno(), initial_errno);
    }
  }
}

#[test]
fn nested_jumps_preserve_errno_even_for_zero_negative_and_extreme_seed_values() {
  for (inner_value, outer_value, initial_errno, expected_outer) in [
    (0, 0, 0, 1),
    (1, 9, -1, 9),
    (-1, -9, c_int::MAX, -9),
    (c_int::MIN, 0, c_int::MIN, 1),
    (c_int::MAX, c_int::MAX, c_int::MIN + 1, c_int::MAX),
  ] {
    write_errno(initial_errno);

    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

    assert_eq!(result, expected_outer);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn nested_jumps_with_extreme_errno_seeds_remain_stable_across_replays() {
  for _round in 0usize..3usize {
    for (inner_value, outer_value, initial_errno, expected_outer) in [
      (0, 0, 0, 1),
      (1, 17, -1, 17),
      (-1, -17, c_int::MAX, -17),
      (c_int::MIN, c_int::MAX, c_int::MIN, c_int::MAX),
      (c_int::MAX, c_int::MIN, c_int::MIN + 1, c_int::MIN),
    ] {
      write_errno(initial_errno);

      // SAFETY: helper functions only jump within frames that remain alive.
      let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

      assert_eq!(result, expected_outer);
      assert_eq!(read_errno(), initial_errno);
    }
  }
}

#[test]
fn nested_value_contract_is_stable_when_case_order_is_reversed() {
  let cases = [
    (0, 0, 0, 1),
    (1, 17, -1, 17),
    (-1, -17, c_int::MAX, -17),
    (c_int::MIN, c_int::MAX, c_int::MIN, c_int::MAX),
    (c_int::MAX, c_int::MIN, c_int::MIN + 1, c_int::MIN),
  ];

  for &(inner_value, outer_value, initial_errno, expected_outer) in &cases {
    write_errno(initial_errno);

    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

    assert_eq!(result, expected_outer);
    assert_eq!(read_errno(), initial_errno);
  }

  for &(inner_value, outer_value, initial_errno, expected_outer) in cases.iter().rev() {
    write_errno(initial_errno);

    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

    assert_eq!(result, expected_outer);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn outer_zero_translation_is_independent_from_inner_value() {
  for (index, inner_value) in [0, 1, -1, c_int::MAX, c_int::MIN].into_iter().enumerate() {
    let cycle_index = c_int::try_from(index).expect("index must fit in c_int");
    let initial_errno = 190 + cycle_index;

    write_errno(initial_errno);

    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(inner_value, 0) };

    assert_eq!(result, 1);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn outer_zero_translation_remains_stable_over_repeated_rounds() {
  for round in 0usize..3usize {
    for (case_index, inner_value) in [0, 7, -7, c_int::MAX, c_int::MIN].into_iter().enumerate() {
      let round_as_c_int = c_int::try_from(round).expect("round index must fit in c_int");
      let case_index = c_int::try_from(case_index).expect("case index must fit in c_int");
      let initial_errno = 210 + round_as_c_int * 10 + case_index;

      write_errno(initial_errno);

      // SAFETY: helper functions only jump within frames that remain alive.
      let result = unsafe { outer_jump_target_with_values(inner_value, 0) };

      assert_eq!(result, 1);
      assert_eq!(read_errno(), initial_errno);
    }
  }
}

#[test]
fn outer_non_zero_value_is_independent_from_inner_value_matrix() {
  for (outer_index, outer_value) in [5, -5, c_int::MAX, c_int::MIN].into_iter().enumerate() {
    for (inner_index, inner_value) in [0, 1, -1, c_int::MAX, c_int::MIN].into_iter().enumerate() {
      let outer_index = c_int::try_from(outer_index).expect("outer index must fit in c_int");
      let inner_index = c_int::try_from(inner_index).expect("inner index must fit in c_int");
      let initial_errno = 260 + outer_index * 10 + inner_index;

      write_errno(initial_errno);

      // SAFETY: helper functions only jump within frames that remain alive.
      let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

      assert_eq!(result, outer_value);
      assert_eq!(read_errno(), initial_errno);
    }
  }
}

#[test]
fn outer_non_zero_values_remain_stable_when_alternated_repeatedly() {
  for (round_index, outer_value) in [7, -7, c_int::MAX, c_int::MIN, 19]
    .into_iter()
    .cycle()
    .take(10)
    .enumerate()
  {
    let round_index = c_int::try_from(round_index).expect("round index must fit in c_int");
    let initial_errno = 320 + round_index;

    write_errno(initial_errno);

    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(0, outer_value) };

    assert_eq!(result, outer_value);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn alternating_outer_zero_and_non_zero_values_do_not_cross_contaminate() {
  for (cycle_index, (inner_value, outer_value, expected_outer)) in [
    (0, 0, 1),
    (7, 13, 13),
    (-7, 0, 1),
    (c_int::MAX, -19, -19),
    (c_int::MIN, 0, 1),
    (1, c_int::MAX, c_int::MAX),
    (0, c_int::MIN, c_int::MIN),
  ]
  .into_iter()
  .enumerate()
  {
    let cycle_index = c_int::try_from(cycle_index).expect("cycle index must fit in c_int");
    let initial_errno = 360 + cycle_index;

    write_errno(initial_errno);

    // SAFETY: helper functions only jump within frames that remain alive.
    let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

    assert_eq!(result, expected_outer);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn alternating_outer_zero_and_non_zero_sequence_is_repeatable() {
  for round in 0usize..3usize {
    for (case_index, (inner_value, outer_value, expected_outer)) in [
      (0, 0, 1),
      (7, 13, 13),
      (-7, 0, 1),
      (c_int::MAX, -19, -19),
      (c_int::MIN, 0, 1),
      (1, c_int::MAX, c_int::MAX),
      (0, c_int::MIN, c_int::MIN),
    ]
    .into_iter()
    .enumerate()
    {
      let round_index = c_int::try_from(round).expect("round index must fit in c_int");
      let case_index = c_int::try_from(case_index).expect("case index must fit in c_int");
      let initial_errno = 430 + round_index * 10 + case_index;

      write_errno(initial_errno);

      // SAFETY: helper functions only jump within frames that remain alive.
      let result = unsafe { outer_jump_target_with_values(inner_value, outer_value) };

      assert_eq!(result, expected_outer);
      assert_eq!(read_errno(), initial_errno);
    }
  }
}

#[test]
fn reusing_single_jmp_buf_handles_alternating_zero_and_extreme_values() {
  let mut env = MaybeUninit::<jmp_buf>::uninit();

  for (cycle, (input, expected)) in [
    (0, 1),
    (c_int::MAX, c_int::MAX),
    (0, 1),
    (c_int::MIN, c_int::MIN),
    (0, 1),
    (-1, -1),
    (0, 1),
    (17, 17),
  ]
  .into_iter()
  .enumerate()
  {
    let cycle_as_c_int = c_int::try_from(cycle).expect("cycle index must fit in c_int");
    let initial_errno = 520 + cycle_as_c_int;

    write_errno(initial_errno);

    // SAFETY: `env` points to writable `jmp_buf` storage for this frame.
    let result = unsafe { setjmp(env.as_mut_ptr()) };

    if result == 0 {
      assert_eq!(read_errno(), initial_errno);
      // SAFETY: `env` was initialized by `setjmp` above and this frame is alive.
      unsafe { longjmp(env.as_ptr(), input) };
    }

    assert_eq!(result, expected);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn reusing_single_jmp_buf_preserves_zero_negative_and_extreme_errno_seeds() {
  let mut env = MaybeUninit::<jmp_buf>::uninit();

  for (input, expected, initial_errno) in [
    (0, 1, 0),
    (7, 7, -1),
    (-7, -7, c_int::MAX),
    (c_int::MAX, c_int::MAX, c_int::MIN),
    (c_int::MIN, c_int::MIN, c_int::MIN + 1),
  ] {
    write_errno(initial_errno);

    // SAFETY: `env` points to writable `jmp_buf` storage for this frame.
    let result = unsafe { setjmp(env.as_mut_ptr()) };

    if result == 0 {
      assert_eq!(read_errno(), initial_errno);
      // SAFETY: `env` was initialized by `setjmp` above and this frame is alive.
      unsafe { longjmp(env.as_ptr(), input) };
    }

    assert_eq!(result, expected);
    assert_eq!(read_errno(), initial_errno);
  }
}

#[test]
fn repeated_save_and_restore_cycles_remain_deterministic() {
  for (input, expected) in [
    (2, 2),
    (5, 5),
    (9, 9),
    (17, 17),
    (-1, -1),
    (c_int::MAX, c_int::MAX),
    (c_int::MIN, c_int::MIN),
    (0, 1),
  ] {
    // SAFETY: helper function owns the frame-local jump buffer it uses.
    let result = unsafe { jump_once(input) };

    assert_eq!(result, expected);
  }
}

#[test]
fn reusing_single_jmp_buf_across_cycles_preserves_result_and_errno() {
  let mut env = MaybeUninit::<jmp_buf>::uninit();

  for (cycle, (input, expected)) in [
    (3, 3),
    (-9, -9),
    (0, 1),
    (c_int::MAX, c_int::MAX),
    (c_int::MIN, c_int::MIN),
  ]
  .into_iter()
  .enumerate()
  {
    let cycle_as_c_int = c_int::try_from(cycle).expect("cycle index must fit in c_int");
    let initial_errno = 100 + cycle_as_c_int;

    write_errno(initial_errno);

    // SAFETY: `env` points to writable `jmp_buf` storage for this frame.
    let result = unsafe { setjmp(env.as_mut_ptr()) };

    if result == 0 {
      assert_eq!(read_errno(), initial_errno);
      // SAFETY: `env` was initialized by `setjmp` above and this frame is alive.
      unsafe { longjmp(env.as_ptr(), input) };
    }

    assert_eq!(result, expected);
    assert_eq!(read_errno(), initial_errno);
  }
}
