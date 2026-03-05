use core::ffi::{c_char, c_int};
use rlibc::abi::errno::EILSEQ;
use rlibc::abi::types::size_t;
use rlibc::errno::__errno_location;
use rlibc::wchar::{mblen, mbstowcs, mbtowc, wchar_t, wcstombs, wctomb};

fn set_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe {
    __errno_location().write(value);
  }
}

fn errno_value() -> c_int {
  // SAFETY: `__errno_location` returns readable thread-local errno storage.
  unsafe { __errno_location().read() }
}

fn to_size_t(value: usize) -> size_t {
  size_t::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize must fit into size_t on this target"))
}

const fn c_char_from_u8(byte: u8) -> c_char {
  c_char::from_ne_bytes([byte])
}

#[test]
fn mbtowc_decodes_ascii_and_multibyte_utf8() {
  let ascii = b"A\0";
  let mut wide: wchar_t = 0;
  // SAFETY: pointers are valid and readable for `n` bytes.
  let ascii_len = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast::<c_char>(), to_size_t(1)) };

  assert_eq!(ascii_len, 1);
  assert_eq!(wide, 65);

  let sushi = b"\xF0\x9F\x8D\xA3\0";
  // SAFETY: pointers are valid and readable for `n` bytes.
  let sushi_len = unsafe { mbtowc(&raw mut wide, sushi.as_ptr().cast::<c_char>(), to_size_t(4)) };

  assert_eq!(sushi_len, 4);
  assert_eq!(wide, 0x1F363);
}

#[test]
fn mbtowc_invalid_sequence_returns_minus_one_and_sets_eilseq() {
  let invalid = [0xE3_u8, 0x28_u8, 0xA1_u8, 0_u8];

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mbtowc(core::ptr::null_mut(), invalid.as_ptr().cast(), to_size_t(3)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mbtowc_with_null_output_pointer_decodes_ascii_and_keeps_errno() {
  let ascii = [b'Z', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(777);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mbtowc(core::ptr::null_mut(), ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(result, 1);
  assert_eq!(errno_value(), 777);
}

#[test]
fn mbtowc_invalid_sequence_does_not_overwrite_output_on_error() {
  let invalid = [0xE3_u8, 0x28_u8, 0xA1_u8, 0_u8];
  let mut wide: wchar_t = 0x1234;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mbtowc(&raw mut wide, invalid.as_ptr().cast(), to_size_t(3)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, 0x1234);
}

#[test]
fn mblen_and_mbtowc_null_reset_requests_return_zero() {
  // SAFETY: null reset call is part of C API contract.
  let mblen_reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };
  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mblen_reset, 0);
  assert_eq!(mbtowc_reset, 0);
}

#[test]
fn mblen_decoding_nul_returns_zero_without_touching_errno() {
  let input = [0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(321);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mblen(input.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(result, 0);
  assert_eq!(errno_value(), 321);
}

#[test]
fn mbtowc_decoding_nul_returns_zero_writes_nul_and_keeps_errno() {
  let input = [0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(654);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mbtowc(&raw mut wide, input.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(result, 0);
  assert_eq!(wide, 0);
  assert_eq!(errno_value(), 654);
}

#[test]
fn mblen_decoding_nul_ignores_following_invalid_byte() {
  let input = [0_u8, 0xFF_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(901);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mblen(input.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(result, 0);
  assert_eq!(errno_value(), 901);
}

#[test]
fn mbtowc_decoding_nul_with_null_output_ignores_following_invalid_byte() {
  let input = [0_u8, 0xFF_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(902);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mbtowc(core::ptr::null_mut(), input.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(result, 0);
  assert_eq!(errno_value(), 902);
}

#[test]
fn mblen_zero_n_with_non_null_input_returns_minus_one_and_sets_eilseq() {
  let input = [b'A', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mblen(input.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mblen_zero_n_probe_does_not_consume_fresh_ascii_input() {
  let input = [b'A', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let probe = unsafe { mblen(input.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let decoded = unsafe { mblen(input.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(decoded, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mblen_zero_n_preserves_internal_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and `n == 0` is an API-valid probe call.
  let zero_probe = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(zero_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let completed = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(completed, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_zero_n_with_non_null_input_returns_minus_one_and_sets_eilseq() {
  let input = [b'A', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let result = unsafe { mbtowc(&raw mut wide, input.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);
}

#[test]
fn mbtowc_zero_n_probe_does_not_consume_fresh_ascii_input() {
  let input = [b'A', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let probe = unsafe { mbtowc(&raw mut wide, input.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(probe, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let decoded = unsafe { mbtowc(&raw mut wide, input.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(decoded, 1);
  assert_eq!(errno_value(), 0);
  assert_eq!(wide, i32::from(b'A'));
}

#[test]
fn mbtowc_zero_n_probe_with_null_output_does_not_consume_fresh_ascii_input() {
  let input = [b'A', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let probe = unsafe { mbtowc(core::ptr::null_mut(), input.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let decoded = unsafe { mbtowc(core::ptr::null_mut(), input.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(decoded, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_zero_n_probe_with_null_output_preserves_internal_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and `n == 0` is an API-valid probe call.
  let zero_probe = unsafe { mbtowc(core::ptr::null_mut(), suffix.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(zero_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let completed = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(completed, 1);
  assert_eq!(wide, 0x3042);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_zero_n_preserves_internal_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and `n == 0` is an API-valid probe call.
  let zero_probe = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(zero_probe, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let completed = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(completed, 1);
  assert_eq!(wide, 0x3042);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_uses_internal_state_across_calls() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let completed = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(completed, 1);
  assert_eq!(wide, 0x3042);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_with_null_output_pointer_preserves_internal_state_across_calls() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let completed = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(completed, 1);
  assert_eq!(wide, 0x3042);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_with_null_output_pointer_invalid_continuation_clears_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let invalid = [b'A', 0_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let invalid_probe =
    unsafe { mbtowc(core::ptr::null_mut(), invalid.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(invalid_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);
}

#[test]
fn mbtowc_with_null_output_pointer_invalid_continuation_recovery_allows_new_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let invalid = [b'A', 0_u8];
  let ascii = [b'B', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let invalid_probe =
    unsafe { mbtowc(core::ptr::null_mut(), invalid.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(invalid_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'B'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_with_null_output_pointer_invalid_then_zero_n_probe_allows_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let invalid = [b'A', 0_u8];
  let ascii = [b'C', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let invalid_probe =
    unsafe { mbtowc(core::ptr::null_mut(), invalid.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(invalid_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and `n == 0` is an API-valid probe call.
  let zero_probe = unsafe { mbtowc(core::ptr::null_mut(), ascii.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(zero_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(core::ptr::null_mut(), ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_invalid_continuation_recovery_allows_new_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let invalid = [b'A', 0_u8];
  let ascii = [b'B', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(&raw mut wide, prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let invalid_probe = unsafe { mbtowc(&raw mut wide, invalid.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(invalid_probe, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'B'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_invalid_continuation_then_zero_n_probe_allows_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let invalid = [b'A', 0_u8];
  let ascii = [b'C', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(&raw mut wide, prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let invalid_probe = unsafe { mbtowc(&raw mut wide, invalid.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(invalid_probe, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and `n == 0` is an API-valid probe call.
  let zero_probe = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(zero_probe, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'C'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_and_mblen_share_internal_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let completed = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(completed, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_null_reset_discards_internal_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  // SAFETY: null reset call is part of C API contract.
  let second_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(second_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mbtowc_null_reset_with_zero_n_and_null_output_keeps_errno_and_allows_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'K', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(1771);
  // SAFETY: null source reset ignores output and must not alter `errno`.
  let second_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(second_reset, 0);
  assert_eq!(errno_value(), 1771);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'K'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_null_reset_with_zero_n_and_output_pointer_keeps_output_and_errno() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'I', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(&raw mut wide, prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(2112);
  // SAFETY: null source reset must ignore `n` and not write through `pwc`.
  let second_reset = unsafe { mbtowc(&raw mut wide, core::ptr::null(), to_size_t(0)) };

  assert_eq!(second_reset, 0);
  assert_eq!(wide, -1);
  assert_eq!(errno_value(), 2112);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'I'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_null_reset_with_nonzero_n_discards_pending_state_without_writing_output() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(733);
  // SAFETY: null reset call ignores `n` and must not write to `pwc`.
  let second_reset = unsafe { mbtowc(&raw mut wide, core::ptr::null(), to_size_t(7)) };

  assert_eq!(second_reset, 0);
  assert_eq!(wide, -1);
  assert_eq!(errno_value(), 733);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);
}

#[test]
fn mbtowc_null_reset_with_nonzero_n_allows_ascii_decode_with_output_pointer() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'F', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(&raw mut wide, prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);

  set_errno(977);
  // SAFETY: null reset call ignores `n` and must not write through `pwc`.
  let second_reset = unsafe { mbtowc(&raw mut wide, core::ptr::null(), to_size_t(15)) };

  assert_eq!(second_reset, 0);
  assert_eq!(wide, -1);
  assert_eq!(errno_value(), 977);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'F'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbtowc_null_reset_with_nonzero_n_and_null_output_discards_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(944);
  // SAFETY: null reset call ignores `n` and does not write through null `pwc`.
  let second_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(9)) };

  assert_eq!(second_reset, 0);
  assert_eq!(errno_value(), 944);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);
}

#[test]
fn mbtowc_null_reset_with_nonzero_n_and_null_output_allows_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'D', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(955);
  // SAFETY: null reset call ignores `n` and does not write through null `pwc`.
  let second_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(11)) };

  assert_eq!(second_reset, 0);
  assert_eq!(errno_value(), 955);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'D'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mblen_uses_internal_state_across_calls() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let completed = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(completed, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mblen_invalid_continuation_clears_internal_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let invalid = [b'A', 0_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let invalid_probe = unsafe { mblen(invalid.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(invalid_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mblen_invalid_continuation_recovery_allows_new_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let invalid = [b'A', 0_u8];
  let ascii = [b'B', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let invalid_probe = unsafe { mblen(invalid.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(invalid_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mblen(ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mblen_invalid_continuation_then_zero_n_probe_allows_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let invalid = [b'A', 0_u8];
  let ascii = [b'C', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let invalid_probe = unsafe { mblen(invalid.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(invalid_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and `n == 0` is an API-valid probe call.
  let zero_probe = unsafe { mblen(ascii.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(zero_probe, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mblen(ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mblen_and_mbtowc_share_internal_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let completed = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(completed, 1);
  assert_eq!(wide, 0x3042);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mblen_null_reset_discards_internal_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  // SAFETY: null reset call is part of C API contract.
  let second_reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(second_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mblen_null_reset_with_nonzero_n_discards_internal_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(744);
  // SAFETY: null reset call ignores `n` and must only reset internal state.
  let second_reset = unsafe { mblen(core::ptr::null(), to_size_t(8)) };

  assert_eq!(second_reset, 0);
  assert_eq!(errno_value(), 744);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mblen_null_reset_with_nonzero_n_allows_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'E', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(966);
  // SAFETY: null reset call ignores `n` and must only reset internal state.
  let second_reset = unsafe { mblen(core::ptr::null(), to_size_t(13)) };

  assert_eq!(second_reset, 0);
  assert_eq!(errno_value(), 966);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mblen(ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_discards_mblen_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mblen_reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(mblen_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  // SAFETY: null reset call is part of C API contract.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0) };

  assert_eq!(wctomb_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_with_invalid_wc_discards_mblen_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mblen_reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(mblen_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(2448);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0xD800) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 2448);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_with_valid_wc_discards_mblen_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mblen_reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(mblen_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(1776);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 1776);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_discards_internal_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  // SAFETY: null reset call is part of C API contract.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0) };

  assert_eq!(wctomb_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_ignores_wc_and_keeps_errno() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(1234);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0xD800) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 1234);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_after_pending_state_allows_fresh_mblen_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'H', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mblen_reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(mblen_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(1993);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0xD800) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 1993);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mblen(ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_invalid_wc_allows_fresh_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'G', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(1881);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0xD800) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 1881);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'G'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_invalid_wc_discards_mbtowc_null_output_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(2992);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0xD800) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 2992);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(core::ptr::null_mut(), suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_with_invalid_wc_allows_fresh_mbtowc_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'L', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(3113);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0xD800) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 3113);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(core::ptr::null_mut(), ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_invalid_wc_allows_fresh_mbtowc_ascii_decode_with_output() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'M', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(3223);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0xD800) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 3223);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'M'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_invalid_wc_after_null_output_pending_allows_fresh_mblen_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'P', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(3553);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0xD800) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 3553);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mblen(ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_valid_wc_discards_mbtowc_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(2112);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 2112);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_with_valid_wc_discards_mbtowc_null_output_pending_state() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(2772);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 2772);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(core::ptr::null_mut(), suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_with_valid_wc_allows_fresh_mbtowc_ascii_decode_after_null_output_pending() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'N', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(3443);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 3443);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'N'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_valid_wc_after_null_output_pending_allows_fresh_mblen_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'Q', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(3663);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 3663);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mblen(ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_valid_wc_after_null_output_pending_allows_fresh_mbtowc_ascii_decode_with_null_output()
 {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'R', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(3773);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 3773);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(core::ptr::null_mut(), ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_valid_wc_after_null_output_pending_discards_state_for_mblen_suffix_decode()
 {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(3993);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 3993);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mblen(suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wctomb_null_reset_with_valid_wc_discards_null_output_pending_state_for_mbtowc_output_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mbtowc(core::ptr::null_mut(), prefix.as_ptr().cast(), to_size_t(2)) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(3883);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 3883);
  assert_eq!(wide, -1);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let trailing = unsafe { mbtowc(&raw mut wide, suffix.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(trailing, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide, -1);
}

#[test]
fn wctomb_null_reset_with_valid_wc_allows_fresh_mbtowc_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'K', 0_u8];
  let mut wide: wchar_t = -1;

  // SAFETY: null reset call is part of C API contract.
  let mbtowc_reset = unsafe { mbtowc(core::ptr::null_mut(), core::ptr::null(), to_size_t(0)) };

  assert_eq!(mbtowc_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe {
    mbtowc(
      &raw mut wide,
      prefix.as_ptr().cast(),
      to_size_t(prefix.len()),
    )
  };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(2333);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 2333);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mbtowc(&raw mut wide, ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(wide, i32::from(b'K'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_null_reset_with_valid_wc_allows_fresh_mblen_ascii_decode() {
  let prefix = [0xE3_u8, 0x81_u8];
  let ascii = [b'J', 0_u8];

  // SAFETY: null reset call is part of C API contract.
  let mblen_reset = unsafe { mblen(core::ptr::null(), to_size_t(0)) };

  assert_eq!(mblen_reset, 0);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let partial = unsafe { mblen(prefix.as_ptr().cast(), to_size_t(prefix.len())) };

  assert_eq!(partial, -1);
  assert_eq!(errno_value(), EILSEQ);

  set_errno(2007);
  // SAFETY: null destination requests reset and ignores `wc`.
  let wctomb_reset = unsafe { wctomb(core::ptr::null_mut(), 0x1F363) };

  assert_eq!(wctomb_reset, 0);
  assert_eq!(errno_value(), 2007);

  set_errno(0);
  // SAFETY: pointers are valid and readable for `n` bytes.
  let recovered = unsafe { mblen(ascii.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(recovered, 1);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_handles_reset_and_utf8_encoding() {
  // SAFETY: null reset call is part of C API contract.
  let reset = unsafe { wctomb(core::ptr::null_mut(), 0) };

  assert_eq!(reset, 0);

  let mut ascii_out = [0 as c_char; 8];
  // SAFETY: destination buffer is writable.
  let ascii_len = unsafe { wctomb(ascii_out.as_mut_ptr(), 0x41) };

  assert_eq!(ascii_len, 1);
  assert_eq!(ascii_out[0], c_char_from_u8(b'A'));

  let mut utf8_out = [0 as c_char; 8];
  // SAFETY: destination buffer is writable.
  let utf8_len = unsafe { wctomb(utf8_out.as_mut_ptr(), 0x1F363) };

  assert_eq!(utf8_len, 4);
  assert_eq!(utf8_out[0], c_char_from_u8(0xF0));
  assert_eq!(utf8_out[1], c_char_from_u8(0x9F));
  assert_eq!(utf8_out[2], c_char_from_u8(0x8D));
  assert_eq!(utf8_out[3], c_char_from_u8(0xA3));
}

#[test]
fn wctomb_successful_encodes_do_not_modify_errno() {
  let mut ascii_out = [c_char_from_u8(0x7A); 8];
  let mut utf8_out = [c_char_from_u8(0x7A); 8];

  set_errno(2718);
  // SAFETY: destination buffer is writable.
  let ascii_len = unsafe { wctomb(ascii_out.as_mut_ptr(), 0x41) };

  assert_eq!(ascii_len, 1);
  assert_eq!(ascii_out[0], c_char_from_u8(b'A'));
  assert_eq!(ascii_out[1], c_char_from_u8(0x7A));
  assert_eq!(errno_value(), 2718);

  set_errno(3141);
  // SAFETY: destination buffer is writable.
  let utf8_len = unsafe { wctomb(utf8_out.as_mut_ptr(), 0x1F363) };

  assert_eq!(utf8_len, 4);
  assert_eq!(utf8_out[0], c_char_from_u8(0xF0));
  assert_eq!(utf8_out[1], c_char_from_u8(0x9F));
  assert_eq!(utf8_out[2], c_char_from_u8(0x8D));
  assert_eq!(utf8_out[3], c_char_from_u8(0xA3));
  assert_eq!(utf8_out[4], c_char_from_u8(0x7A));
  assert_eq!(errno_value(), 3141);
}

#[test]
fn wctomb_rejects_invalid_scalar_and_sets_eilseq_without_writing_output() {
  let mut out = [c_char_from_u8(0x7A); 4];

  set_errno(0);
  // SAFETY: destination buffer is writable.
  let result = unsafe { wctomb(out.as_mut_ptr(), 0xD800) };

  assert_eq!(result, -1);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(out, [c_char_from_u8(0x7A); 4]);
}

#[test]
fn wctomb_encodes_nul_as_single_zero_byte() {
  let mut out = [c_char_from_u8(0x7A); 4];

  set_errno(0);
  // SAFETY: destination buffer is writable.
  let result = unsafe { wctomb(out.as_mut_ptr(), 0) };

  assert_eq!(result, 1);
  assert_eq!(out[0], c_char_from_u8(0));
  assert_eq!(out[1], c_char_from_u8(0x7A));
  assert_eq!(errno_value(), 0);
}

#[test]
fn wctomb_encodes_nul_without_modifying_errno() {
  let mut out = [c_char_from_u8(0x7A); 4];

  set_errno(4242);
  // SAFETY: destination buffer is writable.
  let result = unsafe { wctomb(out.as_mut_ptr(), 0) };

  assert_eq!(result, 1);
  assert_eq!(out[0], c_char_from_u8(0));
  assert_eq!(out[1], c_char_from_u8(0x7A));
  assert_eq!(errno_value(), 4242);
}

#[test]
fn mbstowcs_respects_destination_bound() {
  let src = b"ab\0";
  let mut dst = [0 as wchar_t; 1];
  // SAFETY: pointers are valid; destination has one element.
  let converted = unsafe { mbstowcs(dst.as_mut_ptr(), src.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(converted, to_size_t(1));
  assert_eq!(dst[0], i32::from(b'a'));
}

#[test]
fn mbstowcs_stops_before_validating_when_output_limit_is_reached() {
  let src = [b'a', 0xFF_u8, 0_u8];
  let mut dst = [0 as wchar_t; 1];

  set_errno(1999);
  // SAFETY: pointers are valid and destination has one element.
  let converted = unsafe { mbstowcs(dst.as_mut_ptr(), src.as_ptr().cast(), to_size_t(1)) };

  assert_eq!(converted, to_size_t(1));
  assert_eq!(dst[0], i32::from(b'a'));
  assert_eq!(errno_value(), 1999);
}

#[test]
fn mbstowcs_with_zero_len_does_not_validate_source() {
  let invalid = [0xFF_u8, 0_u8];
  let mut dst = [0 as wchar_t; 1];

  set_errno(1777);
  // SAFETY: pointers are valid and destination length is explicitly zero.
  let converted = unsafe { mbstowcs(dst.as_mut_ptr(), invalid.as_ptr().cast(), to_size_t(0)) };

  assert_eq!(converted, to_size_t(0));
  assert_eq!(errno_value(), 1777);
  assert_eq!(dst[0], 0);
}

#[test]
fn wcstombs_respects_destination_bound() {
  let src = [0x41 as wchar_t, 0x1F363 as wchar_t, 0 as wchar_t];
  let mut dst = [0 as c_char; 1];
  // SAFETY: pointers are valid; destination has one byte.
  let converted = unsafe { wcstombs(dst.as_mut_ptr(), src.as_ptr(), to_size_t(1)) };

  assert_eq!(converted, to_size_t(1));
  assert_eq!(dst[0], c_char_from_u8(b'A'));
}

#[test]
fn wcstombs_stops_before_validating_when_output_limit_is_reached() {
  let src = [0x41 as wchar_t, 0xD800 as wchar_t, 0 as wchar_t];
  let mut dst = [0 as c_char; 1];

  set_errno(2111);
  // SAFETY: pointers are valid and destination has one byte.
  let converted = unsafe { wcstombs(dst.as_mut_ptr(), src.as_ptr(), to_size_t(1)) };

  assert_eq!(converted, to_size_t(1));
  assert_eq!(dst[0], c_char_from_u8(b'A'));
  assert_eq!(errno_value(), 2111);
}

#[test]
fn wcstombs_with_zero_len_does_not_validate_source() {
  let invalid = [0xD800 as wchar_t, 0 as wchar_t];
  let mut dst = [0 as c_char; 1];

  set_errno(1888);
  // SAFETY: pointers are valid and destination length is explicitly zero.
  let converted = unsafe { wcstombs(dst.as_mut_ptr(), invalid.as_ptr(), to_size_t(0)) };

  assert_eq!(converted, to_size_t(0));
  assert_eq!(errno_value(), 1888);
  assert_eq!(dst[0], 0);
}

#[test]
fn mbstowcs_and_wcstombs_report_eilseq_on_invalid_input() {
  let invalid_mb = [0xF0_u8, 0x28_u8, 0x8C_u8, 0x28_u8, 0_u8];
  let mut wide_dst = [0 as wchar_t; 8];

  set_errno(0);
  // SAFETY: pointers are valid and source is NUL-terminated.
  let mb_result = unsafe {
    mbstowcs(
      wide_dst.as_mut_ptr(),
      invalid_mb.as_ptr().cast(),
      to_size_t(8),
    )
  };

  assert_eq!(mb_result, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);

  let invalid_wc = [0xD800 as wchar_t, 0 as wchar_t];
  let mut mb_dst = [0 as c_char; 8];

  set_errno(0);
  // SAFETY: pointers are valid and source is terminated with wide NUL.
  let wc_result = unsafe { wcstombs(mb_dst.as_mut_ptr(), invalid_wc.as_ptr(), to_size_t(8)) };

  assert_eq!(wc_result, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
}
