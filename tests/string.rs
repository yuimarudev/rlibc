use core::ffi::c_char;
use core::ptr;
use rlibc::string::{strlen, strnlen};

const fn as_c_char_ptr(bytes: &[u8]) -> *const c_char {
  bytes.as_ptr().cast::<c_char>()
}

#[test]
fn strlen_and_strnlen_return_zero_for_empty_string() {
  let input = b"\0";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let actual_len = unsafe { strlen(string_ptr) };
  // SAFETY: pointer is readable for at least 8 bytes or until NUL.
  let bounded_len = unsafe { strnlen(string_ptr, 8) };

  assert_eq!(actual_len, 0);
  assert_eq!(bounded_len, 0);
}

#[test]
fn strlen_and_strnlen_stop_at_first_embedded_nul() {
  let input = b"ab\0cd\0";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let actual_len = unsafe { strlen(string_ptr) };
  // SAFETY: pointer is readable for `input.len()` bytes.
  let bounded_len = unsafe { strnlen(string_ptr, input.len()) };

  assert_eq!(actual_len, 2);
  assert_eq!(bounded_len, 2);
}

#[test]
fn strnlen_stops_at_maximum_bound() {
  let input = b"abcdef\0";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: pointer is readable for at least 3 bytes.
  let actual = unsafe { strnlen(string_ptr, 3) };

  assert_eq!(actual, 3);
}

#[test]
fn strnlen_returns_n_when_limit_matches_first_nul_position() {
  let input = b"rust\0lang";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: pointer is readable for at least 4 bytes.
  let actual = unsafe { strnlen(string_ptr, 4) };

  assert_eq!(actual, 4);
}

#[test]
fn strnlen_handles_non_terminated_buffer_with_explicit_bound() {
  let input = b"abcdef";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: pointer is readable for exactly `input.len()` bytes.
  let actual = unsafe { strnlen(string_ptr, input.len()) };

  assert_eq!(actual, input.len());
}

#[test]
fn strnlen_stops_before_terminal_nul_when_limit_covers_full_buffer() {
  let input = b"abc\0";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: pointer is readable for exactly `input.len()` bytes.
  let actual = unsafe { strnlen(string_ptr, input.len()) };

  assert_eq!(actual, 3);
}

#[test]
fn strnlen_is_monotonic_and_caps_at_first_nul() {
  let input = b"abcd\0tail";
  let string_ptr = as_c_char_ptr(input);
  let expected_by_bound = [0_usize, 1, 2, 3, 4, 4, 4, 4];

  for (bound, expected) in expected_by_bound.iter().copied().enumerate() {
    // SAFETY: pointer is readable for at least `bound` bytes in this loop.
    let actual = unsafe { strnlen(string_ptr, bound) };

    assert_eq!(actual, expected, "bound={bound}");
  }
}

#[test]
fn strnlen_matches_strlen_when_bound_covers_full_string() {
  let input = b"kernel\0tail";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let actual_len = unsafe { strlen(string_ptr) };

  for bound in [actual_len, actual_len + 1, input.len()] {
    // SAFETY: pointer is readable for at least `bound` bytes in this loop.
    let bounded_len = unsafe { strnlen(string_ptr, bound) };

    assert_eq!(bounded_len, actual_len, "bound={bound}");
  }
}

#[test]
fn strnlen_matches_min_of_bound_and_strlen_across_range() {
  let input = b"abcde\0rest";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let full_len = unsafe { strlen(string_ptr) };

  for bound in 0..=(input.len() + 2) {
    // SAFETY: pointer is readable for at least `input.len()` bytes and
    // `strnlen` stops at first NUL.
    let bounded_len = unsafe { strnlen(string_ptr, bound) };

    assert_eq!(
      bounded_len,
      core::cmp::min(bound, full_len),
      "bound={bound}"
    );
  }
}

#[test]
fn strnlen_matches_strlen_for_usize_max_bound_on_terminated_input() {
  let input = b"max\0bound";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let full_len = unsafe { strlen(string_ptr) };
  // SAFETY: the first NUL appears in-bounds, so scanning stops before
  // attempting to traverse the full `usize::MAX` limit.
  let bounded_len = unsafe { strnlen(string_ptr, usize::MAX) };

  assert_eq!(bounded_len, full_len);
}

#[test]
fn strnlen_counts_bytes_even_when_bound_splits_utf8_scalar() {
  let input = b"\xE5\xAF\xBF\0";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: pointer is readable for at least 2 bytes.
  let bounded_len = unsafe { strnlen(string_ptr, 2) };

  assert_eq!(bounded_len, 2);
}

#[test]
fn strnlen_returns_zero_for_empty_string_with_usize_max_bound() {
  let input = b"\0tail";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let bounded_len = unsafe { strnlen(string_ptr, usize::MAX) };

  assert_eq!(bounded_len, 0);
}

#[test]
fn strlen_and_strnlen_work_with_offset_pointer_into_buffer() {
  let input = b"xxcore\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated sequence.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: pointer is readable for at least 6 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 6) };

  assert_eq!(actual_len, 4);
  assert_eq!(bounded_len, 4);
}

#[test]
fn strnlen_applies_bound_on_offset_pointer() {
  let input = b"xxcore\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: pointer is readable for at least 2 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 2) };

  assert_eq!(bounded_len, 2);
}

#[test]
fn strnlen_is_monotonic_and_caps_at_nul_for_offset_pointer() {
  let input = b"xxcore\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  let expected_by_bound = [0_usize, 1, 2, 3, 4, 4, 4, 4, 4];

  for (bound, expected) in expected_by_bound.iter().copied().enumerate() {
    // SAFETY: offset pointer is readable for at least `bound` bytes.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(bounded_len, expected, "bound={bound}");
  }
}

#[test]
fn strnlen_returns_n_when_offset_limit_matches_first_nul_position() {
  let input = b"xxcore\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: pointer is readable for at least 4 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 4) };

  assert_eq!(bounded_len, 4);
}

#[test]
fn strnlen_stops_at_first_nul_from_offset_pointer() {
  let input = b"xxa\0b\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: pointer is readable for at least 4 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 4) };

  assert_eq!(bounded_len, 1);
}

#[test]
fn strlen_and_strnlen_handle_single_remaining_byte_before_nul_from_offset() {
  let input = b"ab\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` and points at a byte
  // immediately before the terminating NUL.
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated sequence.
  let actual_len = unsafe { strlen(offset_ptr) };

  for (bound, expected) in [(0_usize, 0_usize), (1, 1), (2, 1)] {
    // SAFETY: scanning stops at in-bounds NUL before bound is exhausted.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(bounded_len, expected, "bound={bound}");
  }

  // SAFETY: scanning stops at in-bounds NUL before exhausting `usize::MAX`.
  let max_bound_len = unsafe { strnlen(offset_ptr, usize::MAX) };

  assert_eq!(actual_len, 1);
  assert_eq!(max_bound_len, actual_len);
}

#[test]
fn strnlen_matches_min_of_bound_and_strlen_for_single_remaining_byte_offset() {
  let input = b"ab\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` and points one byte before NUL.
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated sequence.
  let full_len = unsafe { strlen(offset_ptr) };

  for bound in 0..=(input.len() + 2) {
    // SAFETY: scanning stops at in-bounds NUL regardless of larger bounds.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(
      bounded_len,
      core::cmp::min(bound, full_len),
      "bound={bound}"
    );
  }
}

#[test]
fn strlen_and_strnlen_return_zero_on_offset_to_nul() {
  let input = b"ab\0cd\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input` and points at NUL.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated sequence.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: pointer is readable for at least 1 byte.
  let bounded_len = unsafe { strnlen(offset_ptr, 1) };

  assert_eq!(actual_len, 0);
  assert_eq!(bounded_len, 0);
}

#[test]
fn strnlen_returns_zero_on_offset_to_nul_with_usize_max_bound() {
  let input = b"ab\0cd\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input` and points at NUL.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated sequence.
  let bounded_len = unsafe { strnlen(offset_ptr, usize::MAX) };

  assert_eq!(bounded_len, 0);
}

#[test]
fn strnlen_stays_zero_for_all_bounds_on_offset_to_nul() {
  let input = b"ab\0cd\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input` and points at NUL.
  let offset_ptr = unsafe { base_ptr.add(2) };
  let expected_by_bound = [0_usize, 0, 0, 0, 0];

  for (bound, expected) in expected_by_bound.iter().copied().enumerate() {
    // SAFETY: `offset_ptr` points at NUL, so reads stop immediately for `n > 0`.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(bounded_len, expected, "bound={bound}");
  }
}

#[test]
fn strnlen_matches_strlen_for_usize_max_bound_on_offset_pointer() {
  let input = b"xxcore\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated sequence.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: scanning stops at the in-bounds first NUL.
  let bounded_len = unsafe { strnlen(offset_ptr, usize::MAX) };

  assert_eq!(bounded_len, actual_len);
}

#[test]
fn strnlen_matches_min_of_bound_and_strlen_across_range_for_offset_pointer() {
  let input = b"xxcore\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated sequence.
  let full_len = unsafe { strlen(offset_ptr) };

  for bound in 0..=(input.len() + 2) {
    // SAFETY: scanning stops at in-bounds NUL regardless of larger bounds.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(
      bounded_len,
      core::cmp::min(bound, full_len),
      "bound={bound}"
    );
  }
}

#[test]
fn strnlen_returns_zero_for_zero_bound_on_offset_pointer() {
  let input = b"xxcore\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: `n == 0` guarantees no dereference.
  let bounded_len = unsafe { strnlen(offset_ptr, 0) };

  assert_eq!(bounded_len, 0);
}

#[test]
fn strnlen_caps_at_offset_nul_even_with_large_finite_bound() {
  let input = b"xxcore\0tail";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated sequence.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: scanning stops at in-bounds NUL before bound is exhausted.
  let bounded_len = unsafe { strnlen(offset_ptr, 64) };

  assert_eq!(bounded_len, actual_len);
}

#[test]
fn strnlen_handles_non_terminated_slice_via_offset_and_bound() {
  let input = b"xxabcdef";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: offset pointer is readable for at least 4 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 4) };

  assert_eq!(bounded_len, 4);
}

#[test]
fn strnlen_handles_single_byte_bound_on_non_terminated_offset_slice() {
  let input = b"xxabcdef";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: offset pointer is readable for at least 1 byte.
  let bounded_len = unsafe { strnlen(offset_ptr, 1) };

  assert_eq!(bounded_len, 1);
}

#[test]
fn strnlen_handles_full_non_terminated_slice_via_offset_and_bound() {
  let input = b"xxabcdef";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: offset pointer is readable for at least 6 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 6) };

  assert_eq!(bounded_len, 6);
}

#[test]
fn strnlen_handles_near_full_bound_on_non_terminated_offset_slice() {
  let input = b"xxabcdef";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: offset pointer is readable for at least 5 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 5) };

  assert_eq!(bounded_len, 5);
}

#[test]
fn strnlen_is_monotonic_on_non_terminated_offset_slice() {
  let input = b"xxabcdef";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  let expected_by_bound = [0_usize, 1, 2, 3, 4, 5, 6];

  for (bound, expected) in expected_by_bound.iter().copied().enumerate() {
    // SAFETY: offset pointer is readable for at least `bound` bytes.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(bounded_len, expected, "bound={bound}");
  }
}

#[test]
fn strnlen_returns_zero_for_zero_bound_on_non_terminated_offset_slice() {
  let input = b"xxabcdef";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 2 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(2) };
  // SAFETY: `n == 0` guarantees no dereference.
  let bounded_len = unsafe { strnlen(offset_ptr, 0) };

  assert_eq!(bounded_len, 0);
}

#[test]
fn strlen_and_strnlen_count_utf8_bytes_from_offset_pointer() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 3 bytes stays within `input` and lands at a valid byte.
  let offset_ptr = unsafe { base_ptr.add(3) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated byte sequence.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: pointer is readable for at least `input.len() - 3` bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, input.len() - 3) };

  assert_eq!(actual_len, 3);
  assert_eq!(bounded_len, 3);
}

#[test]
fn strnlen_counts_bytes_when_offset_bound_splits_utf8_scalar() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 3 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(3) };
  // SAFETY: offset pointer is readable for at least 2 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 2) };

  assert_eq!(bounded_len, 2);
}

#[test]
fn strnlen_matches_min_of_bound_and_strlen_across_range_for_utf8_offset_pointer() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 3 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(3) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated byte sequence.
  let full_len = unsafe { strlen(offset_ptr) };

  for bound in 0..=(input.len() + 2) {
    // SAFETY: scanning stops at in-bounds NUL regardless of larger bounds.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(
      bounded_len,
      core::cmp::min(bound, full_len),
      "bound={bound}"
    );
  }
}

#[test]
fn strnlen_is_monotonic_and_caps_at_nul_for_utf8_offset_pointer() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 3 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(3) };
  let expected_by_bound = [0_usize, 1, 2, 3, 3, 3, 3];

  for (bound, expected) in expected_by_bound.iter().copied().enumerate() {
    // SAFETY: scanning stops at the in-bounds NUL before any out-of-range read.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(bounded_len, expected, "bound={bound}");
  }
}

#[test]
fn strnlen_caps_at_nul_for_large_finite_bound_on_utf8_offset_pointer() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 3 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(3) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated byte sequence.
  let full_len = unsafe { strlen(offset_ptr) };
  // SAFETY: scanning stops at in-bounds NUL before bound is exhausted.
  let bounded_len = unsafe { strnlen(offset_ptr, 64) };

  assert_eq!(bounded_len, full_len);
}

#[test]
fn strnlen_returns_zero_for_zero_bound_on_utf8_offset_pointer() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 3 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(3) };
  // SAFETY: `n == 0` guarantees no dereference.
  let bounded_len = unsafe { strnlen(offset_ptr, 0) };

  assert_eq!(bounded_len, 0);
}

#[test]
fn strnlen_matches_strlen_for_usize_max_on_utf8_offset_pointer() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 3 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(3) };
  // SAFETY: `offset_ptr` points to a valid NUL-terminated byte sequence.
  let full_len = unsafe { strlen(offset_ptr) };
  // SAFETY: scanning stops at the in-bounds NUL before exhausting `usize::MAX`.
  let bounded_len = unsafe { strnlen(offset_ptr, usize::MAX) };

  assert_eq!(bounded_len, full_len);
}

#[test]
fn strnlen_returns_n_when_limit_matches_first_nul_for_utf8_offset_pointer() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 3 bytes stays within `input`.
  let offset_ptr = unsafe { base_ptr.add(3) };
  // SAFETY: pointer is readable for at least 3 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 3) };

  assert_eq!(bounded_len, 3);
}

#[test]
fn strlen_and_strnlen_count_bytes_from_mid_utf8_scalar_offset_pointer() {
  let input = b"\xE5\xAF\xBF\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` still points into a valid NUL-terminated byte string.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: offset pointer is readable for at least 2 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 2) };

  assert_eq!(actual_len, 2);
  assert_eq!(bounded_len, 2);
}

#[test]
fn strlen_and_strnlen_stop_at_first_nul_from_mid_utf8_offset_with_utf8_tail() {
  let input = b"\xE5\xAF\xBF\0\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` points into a valid NUL-terminated byte string.
  let actual_len = unsafe { strlen(offset_ptr) };

  for bound in [2_usize, input.len(), usize::MAX] {
    // SAFETY: scanning stops at first in-bounds NUL before bound is exhausted.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(bounded_len, 2, "bound={bound}");
  }

  assert_eq!(actual_len, 2);
}

#[test]
fn strnlen_is_monotonic_and_caps_at_nul_for_mid_utf8_scalar_offset_pointer() {
  let input = b"\xE5\xAF\xBF\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  let expected_by_bound = [0_usize, 1, 2, 2, 2];

  for (bound, expected) in expected_by_bound.iter().copied().enumerate() {
    // SAFETY: scanning stops at the in-bounds NUL before any out-of-range read.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(bounded_len, expected, "bound={bound}");
  }
}

#[test]
fn strnlen_matches_strlen_for_usize_max_on_mid_utf8_scalar_offset_pointer() {
  let input = b"\xE5\xAF\xBF\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` still points into a valid NUL-terminated byte string.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: scanning stops at in-bounds NUL before exhausting `usize::MAX`.
  let bounded_len = unsafe { strnlen(offset_ptr, usize::MAX) };

  assert_eq!(bounded_len, actual_len);
}

#[test]
fn strnlen_caps_at_nul_for_large_finite_bound_on_mid_utf8_offset() {
  let input = b"\xE5\xAF\xBF\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` still points into a valid NUL-terminated byte string.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: scanning stops at in-bounds NUL before bound is exhausted.
  let bounded_len = unsafe { strnlen(offset_ptr, 64) };

  assert_eq!(bounded_len, actual_len);
}

#[test]
fn strnlen_returns_zero_for_zero_bound_on_mid_utf8_scalar_offset_pointer() {
  let input = b"\xE5\xAF\xBF\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `n == 0` guarantees no dereference.
  let bounded_len = unsafe { strnlen(offset_ptr, 0) };

  assert_eq!(bounded_len, 0);
}

#[test]
fn strnlen_matches_min_of_bound_and_strlen_across_range_for_mid_utf8_offset() {
  let input = b"\xE5\xAF\xBF\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` still points into a valid NUL-terminated byte string.
  let full_len = unsafe { strlen(offset_ptr) };

  for bound in 0..=(input.len() + 2) {
    // SAFETY: scanning stops at in-bounds NUL regardless of larger bounds.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(
      bounded_len,
      core::cmp::min(bound, full_len),
      "bound={bound}"
    );
  }
}

#[test]
fn strlen_and_strnlen_count_bytes_from_mid_utf8_scalar_with_tail_scalar() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` points into a valid NUL-terminated byte string.
  let actual_len = unsafe { strlen(offset_ptr) };
  // SAFETY: pointer is readable for at least 5 bytes from `offset_ptr`.
  let bounded_len = unsafe { strnlen(offset_ptr, 5) };

  assert_eq!(actual_len, 5);
  assert_eq!(bounded_len, 5);
}

#[test]
fn strnlen_matches_min_of_bound_and_strlen_for_mid_utf8_with_tail_scalar() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` still points into a valid NUL-terminated byte string.
  let full_len = unsafe { strlen(offset_ptr) };

  for bound in 0..=(input.len() + 2) {
    // SAFETY: scanning stops at in-bounds NUL regardless of larger bounds.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(
      bounded_len,
      core::cmp::min(bound, full_len),
      "bound={bound}"
    );
  }
}

#[test]
fn strnlen_is_monotonic_and_caps_at_nul_for_mid_utf8_with_tail_scalar() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  let expected_by_bound = [0_usize, 1, 2, 3, 4, 5, 5];

  for (bound, expected) in expected_by_bound.iter().copied().enumerate() {
    // SAFETY: scanning stops at the in-bounds NUL before any out-of-range read.
    let bounded_len = unsafe { strnlen(offset_ptr, bound) };

    assert_eq!(bounded_len, expected, "bound={bound}");
  }
}

#[test]
fn strnlen_caps_at_nul_for_large_finite_bound_on_mid_utf8_with_tail_scalar() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` still points into a valid NUL-terminated byte string.
  let full_len = unsafe { strlen(offset_ptr) };
  // SAFETY: scanning stops at in-bounds NUL before bound is exhausted.
  let bounded_len = unsafe { strnlen(offset_ptr, 64) };

  assert_eq!(bounded_len, full_len);
}

#[test]
fn strnlen_matches_strlen_for_usize_max_on_mid_utf8_with_tail_scalar() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `offset_ptr` still points into a valid NUL-terminated byte string.
  let full_len = unsafe { strlen(offset_ptr) };
  // SAFETY: scanning stops at in-bounds NUL before exhausting `usize::MAX`.
  let bounded_len = unsafe { strnlen(offset_ptr, usize::MAX) };

  assert_eq!(bounded_len, full_len);
}

#[test]
fn strnlen_returns_n_when_limit_matches_first_nul_for_mid_utf8_with_tail_scalar() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: pointer is readable for at least 5 bytes.
  let bounded_len = unsafe { strnlen(offset_ptr, 5) };

  assert_eq!(bounded_len, 5);
}

#[test]
fn strnlen_returns_zero_for_zero_bound_on_mid_utf8_with_tail_scalar() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\0";
  let base_ptr = as_c_char_ptr(input);
  // SAFETY: offset by 1 byte stays within `input` (mid-scalar byte).
  let offset_ptr = unsafe { base_ptr.add(1) };
  // SAFETY: `n == 0` guarantees no dereference.
  let bounded_len = unsafe { strnlen(offset_ptr, 0) };

  assert_eq!(bounded_len, 0);
}

#[test]
fn strlen_and_strnlen_count_utf8_byte_length() {
  let input = b"\xE5\xAF\xBF\xE5\x8F\xB8\xF0\x9F\x8D\xA3\0";
  let string_ptr = as_c_char_ptr(input);
  // SAFETY: `input` is NUL-terminated and readable.
  let actual_len = unsafe { strlen(string_ptr) };
  // SAFETY: pointer is readable for `input.len()` bytes.
  let bounded_len = unsafe { strnlen(string_ptr, input.len()) };

  assert_eq!(actual_len, 10);
  assert_eq!(bounded_len, 10);
}

#[test]
fn strnlen_allows_null_pointer_when_limit_is_zero() {
  // SAFETY: `n == 0` means no memory is dereferenced.
  let actual = unsafe { strnlen(ptr::null(), 0) };

  assert_eq!(actual, 0);
}
