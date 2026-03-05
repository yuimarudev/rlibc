use rlibc::wchar::{Utf8DecodeResult, Utf8EncodeError, decode_utf8, encode_utf8, mbstate_t};

fn decode_once(bytes: &[u8]) -> Utf8DecodeResult {
  let mut state = mbstate_t::new();

  decode_utf8(&mut state, bytes)
}

const fn write_state_bytes(state: &mut mbstate_t, raw: [u8; 8]) {
  // SAFETY: `mbstate_t` ABI layout is fixed and public for C interop; tests use
  // raw byte injection to emulate externally corrupted state.
  unsafe {
    core::ptr::copy_nonoverlapping(
      raw.as_ptr(),
      std::ptr::from_mut::<mbstate_t>(state).cast::<u8>(),
      raw.len(),
    );
  }
}

#[test]
fn decode_utf8_accepts_boundary_sequences() {
  let cases: [(&[u8], u32); 7] = [
    (&[0x41], 0x41),
    (&[0xC2, 0x80], 0x80),
    (&[0xDF, 0xBF], 0x7FF),
    (&[0xE0, 0xA0, 0x80], 0x800),
    (&[0xEF, 0xBF, 0xBF], 0xFFFF),
    (&[0xF0, 0x90, 0x80, 0x80], 0x10000),
    (&[0xF4, 0x8F, 0xBF, 0xBF], 0x0010_FFFF),
  ];

  for (bytes, expected) in cases {
    let mut state = mbstate_t::new();
    let actual = decode_utf8(&mut state, bytes);

    assert_eq!(
      actual,
      Utf8DecodeResult::Complete {
        code_point: expected,
        consumed: bytes.len(),
      },
    );
    assert!(state.is_initial());
  }
}

#[test]
fn decode_utf8_consumes_only_one_scalar_from_fresh_input() {
  let mut state = mbstate_t::new();
  let bytes = [0x41_u8, 0x42];
  let result = decode_utf8(&mut state, &bytes);

  assert_eq!(
    result,
    Utf8DecodeResult::Complete {
      code_point: 0x41,
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_invalid_leading_and_continuation_bytes() {
  let invalid_leads: [&[u8]; 4] = [
    &[0x80],
    &[0xC0, 0x80],
    &[0xC1, 0x80],
    &[0xF5, 0x80, 0x80, 0x80],
  ];

  for input in invalid_leads {
    assert_eq!(
      decode_once(input),
      Utf8DecodeResult::Invalid { consumed: 0 }
    );
  }

  assert_eq!(
    decode_once(&[0xE2, 0x28, 0xA1]),
    Utf8DecodeResult::Invalid { consumed: 1 },
  );
}

#[test]
fn decode_utf8_rejects_overlong_surrogate_and_out_of_range_sequences() {
  assert_eq!(
    decode_once(&[0xE0, 0x80, 0x80]),
    Utf8DecodeResult::Invalid { consumed: 2 },
  );
  assert_eq!(
    decode_once(&[0xF0, 0x80, 0x80, 0x80]),
    Utf8DecodeResult::Invalid { consumed: 2 },
  );
  assert_eq!(
    decode_once(&[0xED, 0xA0, 0x80]),
    Utf8DecodeResult::Invalid { consumed: 2 },
  );
  assert_eq!(
    decode_once(&[0xF4, 0x90, 0x80, 0x80]),
    Utf8DecodeResult::Invalid { consumed: 2 },
  );
}

#[test]
fn decode_utf8_supports_incremental_partial_then_complete() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xE3]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0x81, 0x82]);

  assert_eq!(
    second,
    Utf8DecodeResult::Complete {
      code_point: 0x3042,
      consumed: 2,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_keeps_valid_pending_state_on_empty_input() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xE3]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let state_after_first = state;
  let empty_probe = decode_utf8(&mut state, &[]);

  assert_eq!(empty_probe, Utf8DecodeResult::Incomplete { consumed: 0 });
  assert_eq!(state, state_after_first);
  assert!(!state.is_initial());

  let resumed = decode_utf8(&mut state, &[0x81, 0x82]);

  assert_eq!(
    resumed,
    Utf8DecodeResult::Complete {
      code_point: 0x3042,
      consumed: 2,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_on_empty_input_with_initial_state_returns_incomplete_without_state_change() {
  let mut state = mbstate_t::new();
  let baseline = state;
  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Incomplete { consumed: 0 });
  assert_eq!(state, baseline);
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_consumes_only_required_bytes_after_partial_state() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xE3]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0x81, 0x82, 0x41]);

  assert_eq!(
    second,
    Utf8DecodeResult::Complete {
      code_point: 0x3042,
      consumed: 2,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_supports_incremental_partial_then_invalid() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xF0, 0x9F]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 2 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0x41]);

  assert_eq!(second, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_partial_then_invalid_continuation_reports_consumed_prefix() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xE3]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0x81, 0x41]);

  assert_eq!(second, Utf8DecodeResult::Invalid { consumed: 1 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_partial_then_invalid_second_byte_bounds_reports_consumed_prefix() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xE0]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0x80]);

  assert_eq!(second, Utf8DecodeResult::Invalid { consumed: 1 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_partial_then_invalid_surrogate_second_byte_upper_bound_reports_consumed_prefix() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xED]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0xA0]);

  assert_eq!(second, Utf8DecodeResult::Invalid { consumed: 1 });
  assert!(state.is_initial());

  let resumed = decode_utf8(&mut state, &[0x41]);

  assert_eq!(
    resumed,
    Utf8DecodeResult::Complete {
      code_point: 0x41,
      consumed: 1,
    }
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_partial_then_invalid_surrogate_second_byte_with_trailing_input_reports_consumed_prefix()
 {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xED]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0xA0, 0x41]);

  assert_eq!(second, Utf8DecodeResult::Invalid { consumed: 1 });
  assert!(state.is_initial());

  let resumed = decode_utf8(&mut state, &[0x41]);

  assert_eq!(
    resumed,
    Utf8DecodeResult::Complete {
      code_point: 0x41,
      consumed: 1,
    }
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_partial_then_invalid_four_byte_second_byte_upper_bound_reports_consumed_prefix() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xF4]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0x90]);

  assert_eq!(second, Utf8DecodeResult::Invalid { consumed: 1 });
  assert!(state.is_initial());

  let resumed = decode_utf8(&mut state, &[0x41]);

  assert_eq!(
    resumed,
    Utf8DecodeResult::Complete {
      code_point: 0x41,
      consumed: 1,
    }
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_partial_then_invalid_four_byte_second_byte_lower_bound_reports_consumed_prefix() {
  let mut state = mbstate_t::new();
  let first = decode_utf8(&mut state, &[0xF0]);

  assert_eq!(first, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  let second = decode_utf8(&mut state, &[0x80]);

  assert_eq!(second, Utf8DecodeResult::Invalid { consumed: 1 });
  assert!(state.is_initial());

  let resumed = decode_utf8(&mut state, &[0x41]);

  assert_eq!(
    resumed,
    Utf8DecodeResult::Complete {
      code_point: 0x41,
      consumed: 1,
    }
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_prefix_without_consuming_input() {
  let mut state = mbstate_t::new();

  // bytes=[0x80, 0x80, 0, 0], pending_len=2, expected_len=2 (invalid lead byte).
  write_state_bytes(&mut state, [0x80, 0x80, 0x00, 0x00, 0x02, 0x02, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[0xA0, 0xA1]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_prefix_on_empty_input() {
  let mut state = mbstate_t::new();

  // bytes=[0x80, 0, 0, 0], pending_len=1, expected_len=2 (invalid lead byte).
  write_state_bytes(&mut state, [0x80, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_prefix_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0x80, 0x80, 0, 0], pending_len=2, expected_len=2 (invalid lead byte).
  write_state_bytes(&mut state, [0x80, 0x80, 0x00, 0x00, 0x02, 0x02, 0x00, 0x00]);

  let input = [b'A', b'B'];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_prefix_then_empty_input_is_clean_initial_state() {
  let mut state = mbstate_t::new();

  // bytes=[0x80, 0x80, 0, 0], pending_len=2, expected_len=2 (invalid lead byte).
  write_state_bytes(&mut state, [0x80, 0x80, 0x00, 0x00, 0x02, 0x02, 0x00, 0x00]);

  let first = decode_utf8(&mut state, &[b'A']);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let empty_probe = decode_utf8(&mut state, &[]);

  assert_eq!(empty_probe, Utf8DecodeResult::Incomplete { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_second_byte_bounds_without_consuming_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  write_state_bytes(&mut state, [0xE0, 0x80, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00]);

  let overlong_prefix_result = decode_utf8(&mut state, &[0x80]);

  assert_eq!(
    overlong_prefix_result,
    Utf8DecodeResult::Invalid { consumed: 0 }
  );
  assert!(state.is_initial());

  // bytes=[0xED, 0xA0, 0, 0], pending_len=2, expected_len=3.
  // For 0xED lead, second byte must be <= 0x9F.
  write_state_bytes(&mut state, [0xED, 0xA0, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00]);

  let surrogate_prefix_result = decode_utf8(&mut state, &[0x80]);

  assert_eq!(
    surrogate_prefix_result,
    Utf8DecodeResult::Invalid { consumed: 0 },
  );
  assert!(state.is_initial());

  // bytes=[0xF4, 0x90, 0, 0], pending_len=2, expected_len=4.
  // For 0xF4 lead, second byte must be <= 0x8F.
  write_state_bytes(&mut state, [0xF4, 0x90, 0x00, 0x00, 0x02, 0x04, 0x00, 0x00]);

  let out_of_range_prefix_result = decode_utf8(&mut state, &[0x80, 0x80]);

  assert_eq!(
    out_of_range_prefix_result,
    Utf8DecodeResult::Invalid { consumed: 0 },
  );
  assert!(state.is_initial());

  // bytes=[0xF0, 0x80, 0, 0], pending_len=2, expected_len=4.
  // For 0xF0 lead, second byte must be >= 0x90.
  write_state_bytes(&mut state, [0xF0, 0x80, 0x00, 0x00, 0x02, 0x04, 0x00, 0x00]);

  let too_low_second_byte_result = decode_utf8(&mut state, &[0x80, 0x80]);

  assert_eq!(
    too_low_second_byte_result,
    Utf8DecodeResult::Invalid { consumed: 0 },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_second_byte_bounds_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  write_state_bytes(&mut state, [0xE0, 0x80, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00]);

  let input = [b'A', b'B'];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_second_byte_bounds_on_empty_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  write_state_bytes(&mut state, [0xE0, 0x80, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00]);

  let overlong_prefix_result = decode_utf8(&mut state, &[]);

  assert_eq!(
    overlong_prefix_result,
    Utf8DecodeResult::Invalid { consumed: 0 }
  );
  assert!(state.is_initial());

  // bytes=[0xED, 0xA0, 0, 0], pending_len=2, expected_len=3.
  // For 0xED lead, second byte must be <= 0x9F.
  write_state_bytes(&mut state, [0xED, 0xA0, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00]);

  let surrogate_prefix_result = decode_utf8(&mut state, &[]);

  assert_eq!(
    surrogate_prefix_result,
    Utf8DecodeResult::Invalid { consumed: 0 },
  );
  assert!(state.is_initial());

  // bytes=[0xF4, 0x90, 0, 0], pending_len=2, expected_len=4.
  // For 0xF4 lead, second byte must be <= 0x8F.
  write_state_bytes(&mut state, [0xF4, 0x90, 0x00, 0x00, 0x02, 0x04, 0x00, 0x00]);

  let out_of_range_prefix_result = decode_utf8(&mut state, &[]);

  assert_eq!(
    out_of_range_prefix_result,
    Utf8DecodeResult::Invalid { consumed: 0 },
  );
  assert!(state.is_initial());

  // bytes=[0xF0, 0x80, 0, 0], pending_len=2, expected_len=4.
  // For 0xF0 lead, second byte must be >= 0x90.
  write_state_bytes(&mut state, [0xF0, 0x80, 0x00, 0x00, 0x02, 0x04, 0x00, 0x00]);

  let too_low_second_byte_result = decode_utf8(&mut state, &[]);

  assert_eq!(
    too_low_second_byte_result,
    Utf8DecodeResult::Invalid { consumed: 0 },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_length_relationship_without_consuming_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0x81, 0, 0], pending_len=2, expected_len=2 (pending == expected).
  write_state_bytes(&mut state, [0xE3, 0x81, 0x00, 0x00, 0x02, 0x02, 0x00, 0x00]);

  let equal_lengths_result = decode_utf8(&mut state, &[0x82]);

  assert_eq!(
    equal_lengths_result,
    Utf8DecodeResult::Invalid { consumed: 0 }
  );
  assert!(state.is_initial());

  // bytes=[0xE3, 0x81, 0x82, 0], pending_len=3, expected_len=2 (pending > expected).
  write_state_bytes(&mut state, [0xE3, 0x81, 0x82, 0x00, 0x03, 0x02, 0x00, 0x00]);

  let pending_exceeds_result = decode_utf8(&mut state, &[0x41]);

  assert_eq!(
    pending_exceeds_result,
    Utf8DecodeResult::Invalid { consumed: 0 }
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_length_relationship_on_empty_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0x81, 0, 0], pending_len=2, expected_len=2 (pending == expected).
  write_state_bytes(&mut state, [0xE3, 0x81, 0x00, 0x00, 0x02, 0x02, 0x00, 0x00]);

  let equal_lengths_result = decode_utf8(&mut state, &[]);

  assert_eq!(
    equal_lengths_result,
    Utf8DecodeResult::Invalid { consumed: 0 }
  );
  assert!(state.is_initial());

  // bytes=[0xE3, 0x81, 0x82, 0], pending_len=3, expected_len=2 (pending > expected).
  write_state_bytes(&mut state, [0xE3, 0x81, 0x82, 0x00, 0x03, 0x02, 0x00, 0x00]);

  let pending_exceeds_result = decode_utf8(&mut state, &[]);

  assert_eq!(
    pending_exceeds_result,
    Utf8DecodeResult::Invalid { consumed: 0 }
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_length_relationship_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0x81, 0, 0], pending_len=2, expected_len=2 (pending == expected).
  write_state_bytes(&mut state, [0xE3, 0x81, 0x00, 0x00, 0x02, 0x02, 0x00, 0x00]);

  let input = [0xE3, 0x81, 0x82];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: 0x3042,
      consumed: input.len(),
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_expected_length_mismatch_without_consuming_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=4.
  // Lead byte 0xE3 implies expected_len=3, so this state is impossible.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x04, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[0x81, 0x82, 0x83]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_expected_length_mismatch_on_empty_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=4.
  // Lead byte 0xE3 implies expected_len=3, so this state is impossible.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x04, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_corrupted_pending_expected_length_mismatch_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=4.
  // Lead byte 0xE3 implies expected_len=3, so this state is impossible.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x04, 0x00, 0x00]);

  let input = [0xE3, 0x81, 0x82];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: 0x3042,
      consumed: input.len(),
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_pending_state_with_expected_length_above_utf8_max_on_empty_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xF0, 0, 0, 0], pending_len=1, expected_len=5.
  // UTF-8 scalars are at most 4 bytes, so expected_len=5 is always corrupted.
  write_state_bytes(&mut state, [0xF0, 0x00, 0x00, 0x00, 0x01, 0x05, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_pending_state_with_expected_length_above_utf8_max_without_consuming_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xF0, 0, 0, 0], pending_len=1, expected_len=5.
  // UTF-8 scalars are at most 4 bytes, so expected_len=5 is always corrupted.
  write_state_bytes(&mut state, [0xF0, 0x00, 0x00, 0x00, 0x01, 0x05, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[0x80, 0x80, 0x80, 0x80]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_pending_state_with_zero_expected_length_without_consuming_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0.
  // Non-zero pending bytes with zero expected length are impossible state.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[0x81, 0x82]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_pending_state_with_zero_expected_length_on_empty_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0.
  // Non-zero pending bytes with zero expected length are impossible state.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_pending_state_with_zero_expected_length_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0.
  // Non-zero pending bytes with zero expected length are impossible state.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);

  let input = [b'A', b'B'];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_pending_with_nonzero_expected_length_without_consuming_input() {
  let mut state = mbstate_t::new();

  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=3.
  // Zero pending bytes with non-zero expected length are impossible state.
  write_state_bytes(&mut state, [0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[0xE3, 0x81, 0x82]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_pending_with_nonzero_expected_length_on_empty_input() {
  let mut state = mbstate_t::new();

  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=3.
  // Zero pending bytes with non-zero expected length are impossible state.
  write_state_bytes(&mut state, [0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_pending_with_nonzero_expected_length_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=3.
  // Zero pending bytes with non-zero expected length are impossible state.
  write_state_bytes(&mut state, [0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00]);

  let input = [b'A', b'B'];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_lengths_with_stale_bytes_without_consuming_input() {
  let mut state = mbstate_t::new();

  // bytes=[0x41, 0, 0, 0], pending_len=0, expected_len=0.
  // Fully initial lengths with stale bytes are impossible in this
  // implementation's state machine.
  write_state_bytes(&mut state, [0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[0x42]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_lengths_with_stale_bytes_on_empty_input() {
  let mut state = mbstate_t::new();

  // bytes=[0x41, 0, 0, 0], pending_len=0, expected_len=0.
  // Fully initial lengths with stale bytes are impossible in this
  // implementation's state machine.
  write_state_bytes(&mut state, [0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_lengths_with_stale_bytes_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0x41, 0, 0, 0], pending_len=0, expected_len=0.
  // Fully initial lengths with stale bytes are impossible in this
  // implementation's state machine.
  write_state_bytes(&mut state, [0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

  let input = [b'A', b'B'];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_state_with_nonzero_reserved_bytes() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=3, reserved[0]=1.
  // Non-zero reserved bytes are treated as corrupted snapshots.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x03, 0x01, 0x00]);

  let result = decode_utf8(&mut state, &[0x81, 0x82]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_state_with_nonzero_reserved_bytes_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=3, reserved[0]=1.
  // Non-zero reserved bytes are treated as corrupted snapshots.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x03, 0x01, 0x00]);

  let input = [b'A', b'B'];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_state_with_nonzero_second_reserved_byte() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=3, reserved[1]=1.
  // Non-zero reserved bytes are treated as corrupted snapshots.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x03, 0x00, 0x01]);

  let result = decode_utf8(&mut state, &[0x81, 0x82]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_state_with_nonzero_second_reserved_byte_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=3, reserved[1]=1.
  // Non-zero reserved bytes are treated as corrupted snapshots.
  write_state_bytes(&mut state, [0xE3, 0x00, 0x00, 0x00, 0x01, 0x03, 0x00, 0x01]);

  let input = [b'A', b'B'];
  let first = decode_utf8(&mut state, &input);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_lengths_with_nonzero_reserved_bytes() {
  let mut state = mbstate_t::new();

  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  // Non-zero reserved bytes are always treated as corrupted snapshots.
  write_state_bytes(&mut state, [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]);

  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_lengths_with_nonzero_reserved_bytes_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  // Non-zero reserved bytes are always treated as corrupted snapshots.
  write_state_bytes(&mut state, [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]);

  let first = decode_utf8(&mut state, &[]);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let input = [b'A', b'B'];
  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_lengths_with_nonzero_second_reserved_byte() {
  let mut state = mbstate_t::new();

  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  // Non-zero reserved bytes are always treated as corrupted snapshots.
  write_state_bytes(&mut state, [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);

  let result = decode_utf8(&mut state, &[]);

  assert_eq!(result, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());
}

#[test]
fn decode_utf8_rejects_zero_lengths_with_nonzero_second_reserved_byte_then_retries_same_input() {
  let mut state = mbstate_t::new();

  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  // Non-zero reserved bytes are always treated as corrupted snapshots.
  write_state_bytes(&mut state, [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);

  let first = decode_utf8(&mut state, &[]);

  assert_eq!(first, Utf8DecodeResult::Invalid { consumed: 0 });
  assert!(state.is_initial());

  let input = [b'A', b'B'];
  let retried = decode_utf8(&mut state, &input);

  assert_eq!(
    retried,
    Utf8DecodeResult::Complete {
      code_point: u32::from(b'A'),
      consumed: 1,
    },
  );
  assert!(state.is_initial());
}

#[test]
fn encode_utf8_round_trips_through_decoder() {
  let code_points: [u32; 5] = [0x24, 0x7FF, 0x20AC, 0x10348, 0x1F363];

  for code_point in code_points {
    let mut encoded = [0_u8; 4];
    let written = encode_utf8(code_point, &mut encoded).expect("valid scalar must encode");
    let mut state = mbstate_t::new();
    let decoded = decode_utf8(&mut state, &encoded[..written]);

    assert_eq!(
      decoded,
      Utf8DecodeResult::Complete {
        code_point,
        consumed: written,
      },
    );
    assert!(state.is_initial());
  }
}

#[test]
fn encode_utf8_rejects_non_scalar_values() {
  let mut buffer = [0_u8; 4];

  assert_eq!(
    encode_utf8(0xD800, &mut buffer),
    Err(Utf8EncodeError::InvalidScalarValue),
  );
  assert_eq!(
    encode_utf8(0x0011_0000, &mut buffer),
    Err(Utf8EncodeError::InvalidScalarValue),
  );
}

#[test]
fn mbstate_t_is_zero_initializable_and_resettable() {
  let mut state = mbstate_t::default();

  assert_eq!(state, mbstate_t::new());
  assert!(state.is_initial());

  let partial = decode_utf8(&mut state, &[0xE3]);

  assert_eq!(partial, Utf8DecodeResult::Incomplete { consumed: 1 });
  assert!(!state.is_initial());

  state.reset();

  assert_eq!(state, mbstate_t::new());
  assert!(state.is_initial());
}
