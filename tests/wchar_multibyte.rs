#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_char;
use core::mem::{align_of, size_of};
use core::ptr;
use rlibc::abi::errno::EILSEQ;
use rlibc::abi::types::{c_int, size_t};
use rlibc::errno::__errno_location;
use rlibc::wchar::{mbrlen, mbrtowc, mbsinit, mbstate_t, wchar_t};
use std::thread;

const ERRNO_SENTINEL: c_int = 777;
const MBR_ERR_INVALID: size_t = size_t::MAX;
const MBR_ERR_INCOMPLETE: size_t = size_t::MAX - 1;
const HIRAGANA_A: wchar_t = 0x3042;
const KANJI_SUSHI: wchar_t = 0x5BFF;

fn sz(value: usize) -> size_t {
  size_t::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize must fit into size_t on x86_64 Linux"))
}

fn errno_ptr() -> *mut c_int {
  // `__errno_location` returns the thread-local errno storage pointer.
  let pointer = __errno_location();

  assert!(!pointer.is_null(), "__errno_location returned null");

  pointer
}

fn set_errno(value: c_int) {
  let pointer = errno_ptr();

  // SAFETY: `errno_ptr` returns writable thread-local storage.
  unsafe {
    pointer.write(value);
  }
}

fn errno_value() -> c_int {
  let pointer = errno_ptr();

  // SAFETY: `errno_ptr` returns readable thread-local storage.
  unsafe { pointer.read() }
}

fn reset_internal_state() {
  // SAFETY: null `s` + null `ps` requests reset of internal TLS conversion state.
  let result = unsafe { mbrtowc(ptr::null_mut(), ptr::null(), sz(0), ptr::null_mut()) };

  assert_eq!(result, 0);
}

#[test]
fn mbrtowc_decodes_ascii_and_consumes_one_byte() {
  let input = b"A";
  let mut output: wchar_t = -1;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: `input` is readable for one byte and pointers are valid.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbstate_t_layout_matches_public_header_contract() {
  assert_eq!(size_of::<mbstate_t>(), 8);
  assert_eq!(align_of::<mbstate_t>(), 1);
}

#[test]
fn mbrtowc_decodes_multibyte_utf8_code_point() {
  let input = [0xE5_u8, 0xAF, 0xBF];
  let mut output: wchar_t = -1;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: `input` is readable for three bytes and pointers are valid.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, sz(3));
  assert_eq!(output, KANJI_SUSHI);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_incomplete_sequence_resumes_with_same_state() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [0x82_u8];
  let mut output: wchar_t = -1;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: first chunk pointer is readable and state/output pointers are valid.
  let first_result = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let first_state = unsafe { mbsinit(&raw const state) };

  // SAFETY: second chunk pointer is readable and state/output pointers are valid.
  let second_result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let second_state = unsafe { mbsinit(&raw const state) };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  assert_eq!(
    first_state, 0,
    "state must be non-initial while sequence is partial"
  );
  assert_eq!(
    second_result,
    sz(1),
    "second call should consume only newly provided byte"
  );
  assert_eq!(
    second_state, 1,
    "state must return to initial after successful completion"
  );
  assert_eq!(output, HIRAGANA_A);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_partial_sequence_rejects_invalid_second_byte_bounds_as_eilseq() {
  let first_chunk = [0xE0_u8];
  let second_chunk = [0x80_u8];
  let resume_chunk = [b'Z'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrtowc(
      &raw mut output,
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  assert_eq!(output, wchar_t::from(b'Z'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_partial_sequence_rejects_invalid_surrogate_second_byte_upper_bound_as_eilseq() {
  let first_chunk = [0xED_u8];
  let second_chunk = [0xA0_u8];
  let resume_chunk = [b'Y'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrtowc(
      &raw mut output,
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  assert_eq!(output, wchar_t::from(b'Y'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_partial_sequence_rejects_invalid_surrogate_second_byte_with_trailing_input_as_eilseq() {
  let first_chunk = [0xED_u8];
  let second_chunk = [0xA0_u8, b'Y'];
  let resume_chunk = [b'Y'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrtowc(
      &raw mut output,
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  assert_eq!(output, wchar_t::from(b'Y'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_partial_sequence_rejects_invalid_four_byte_second_byte_upper_bound_as_eilseq() {
  let first_chunk = [0xF4_u8];
  let second_chunk = [0x90_u8];
  let resume_chunk = [b'Y'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrtowc(
      &raw mut output,
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  assert_eq!(output, wchar_t::from(b'Y'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_partial_sequence_rejects_invalid_four_byte_second_byte_lower_bound_as_eilseq() {
  let first_chunk = [0xF0_u8];
  let second_chunk = [0x80_u8];
  let resume_chunk = [b'Y'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrtowc(
      &raw mut output,
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  assert_eq!(output, wchar_t::from(b'Y'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_with_null_input_resets_state() {
  let first_chunk = [0xE3_u8, 0x81];
  let mut output: wchar_t = -1;
  let mut state = mbstate_t::new();

  // SAFETY: first chunk pointer is readable and state/output pointers are valid.
  let incomplete_result = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let mid_state = unsafe { mbsinit(&raw const state) };

  // SAFETY: null `s` requests state reset per `mbrtowc` contract.
  let reset_result = unsafe { mbrtowc(&raw mut output, ptr::null(), sz(0), &raw mut state) };
  // SAFETY: state pointer is valid.
  let final_state = unsafe { mbsinit(&raw const state) };

  assert_eq!(incomplete_result, MBR_ERR_INCOMPLETE);
  assert_eq!(mid_state, 0);
  assert_eq!(reset_result, 0);
  assert_eq!(output, 0);
  assert_eq!(final_state, 1);
}

#[test]
fn mbrtowc_invalid_sequence_sets_eilseq_and_preserves_output_slot() {
  let input = [0x80_u8];
  let output_sentinel: wchar_t = 12345;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  set_errno(0);

  // SAFETY: pointers are valid and `input` is readable.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_corrupted_state() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xE3;
  raw_state[4] = 1;
  raw_state[5] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents any input read.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_nonzero_expected_and_zero_pending() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[4] = 0;
  raw_state[5] = 2;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_nonzero_expected_and_zero_pending() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[4] = 0;
  raw_state[5] = 2;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_zero_lengths_and_stale_bytes() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: initial-length fields with stale carry bytes.
  raw_state[0] = 0x41;
  raw_state[4] = 0;
  raw_state[5] = 0;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_zero_lengths_and_stale_bytes_then_retries_same_input() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: initial-length fields with stale carry bytes.
  raw_state[0] = 0x41;
  raw_state[4] = 0;
  raw_state[5] = 0;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_zero_lengths_and_stale_bytes() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: initial-length fields with stale carry bytes.
  raw_state[0] = 0x41;
  raw_state[4] = 0;
  raw_state[5] = 0;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_expected_length_above_utf8_max() {
  let input = [0x80_u8, 0x80, 0x80, 0x80];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xF0;
  raw_state[4] = 1;
  raw_state[5] = 5;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_expected_length_above_utf8_max() {
  let input = [0x80_u8, 0x80, 0x80, 0x80];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xF0;
  raw_state[4] = 1;
  raw_state[5] = 5;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_ascii_pending_but_multibyte_expected_length() {
  let input = [0x80_u8];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: ASCII lead implies expected_len=1, not 2.
  raw_state[0] = b'A';
  raw_state[4] = 1;
  raw_state[5] = 2;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_ascii_pending_but_multibyte_expected_length() {
  let input = [0x80_u8];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: ASCII lead implies expected_len=1, not 2.
  raw_state[0] = b'A';
  raw_state[4] = 1;
  raw_state[5] = 2;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_multibyte_pending_but_single_byte_expected_length() {
  let input = [0x80_u8];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: multibyte lead implies expected_len >= 2, not 1.
  raw_state[0] = 0xE3;
  raw_state[4] = 1;
  raw_state[5] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_multibyte_pending_but_single_byte_expected_length_then_retries_same_input()
 {
  let input = [0xE3_u8, 0x81, 0x82];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: multibyte lead implies expected_len >= 2, not 1.
  raw_state[0] = 0xE3;
  raw_state[4] = 1;
  raw_state[5] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and full input is readable.
  let retried = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(input.len()));
  assert_eq!(output, HIRAGANA_A);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_multibyte_pending_but_single_byte_expected_length_then_retries_same_input()
 {
  let input = [0xE3_u8, 0x81, 0x82];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: multibyte lead implies expected_len >= 2, not 1.
  raw_state[0] = 0xE3;
  raw_state[4] = 1;
  raw_state[5] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and full input is readable.
  let retried = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(input.len()));
  assert_eq!(output, HIRAGANA_A);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_nonzero_trailing_bytes_beyond_pending() {
  let input = [0x81_u8, 0x82];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: bytes beyond pending_len carry stale data.
  raw_state[0] = 0xE3;
  raw_state[2] = 0x80;
  raw_state[4] = 1;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_corrupted_pending_lead_prefix() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the buffered lead byte, preserving length counters.
  raw_state[0] = 0x80;

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_corrupted_pending_lead_prefix() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the buffered lead byte, preserving length counters.
  raw_state[0] = 0x80;

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_corrupted_pending_second_byte_bounds() {
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xE0;
  raw_state[1] = 0x80;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  let continuation = [0x80_u8];
  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let overlong_prefix_result = unsafe {
    mbrtowc(
      &raw mut output,
      continuation.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(overlong_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xED, 0xA0, 0, 0], pending_len=2, expected_len=3.
  // For 0xED lead, second byte must be <= 0x9F.
  raw_state[0] = 0xED;
  raw_state[1] = 0xA0;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let surrogate_prefix_result = unsafe {
    mbrtowc(
      &raw mut output,
      continuation.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(surrogate_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xF4, 0x90, 0, 0], pending_len=2, expected_len=4.
  // For 0xF4 lead, second byte must be <= 0x8F.
  raw_state[0] = 0xF4;
  raw_state[1] = 0x90;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 4;

  set_errno(0);

  let trailing = [0x80_u8, 0x80];
  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let out_of_range_prefix_result = unsafe {
    mbrtowc(
      &raw mut output,
      trailing.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(out_of_range_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xF0, 0x80, 0, 0], pending_len=2, expected_len=4.
  // For 0xF0 lead, second byte must be >= 0x90.
  raw_state[0] = 0xF0;
  raw_state[1] = 0x80;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 4;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let too_low_second_byte_result = unsafe {
    mbrtowc(
      &raw mut output,
      trailing.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(too_low_second_byte_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_corrupted_pending_second_byte_bounds() {
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();
  let continuation = [0x80_u8];

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xE0;
  raw_state[1] = 0x80;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let overlong_prefix_result = unsafe {
    mbrtowc(
      &raw mut output,
      continuation.as_ptr().cast::<c_char>(),
      sz(continuation.len()),
      &raw mut state,
    )
  };

  assert_eq!(overlong_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xED, 0xA0, 0, 0], pending_len=2, expected_len=3.
  // For 0xED lead, second byte must be <= 0x9F.
  raw_state[0] = 0xED;
  raw_state[1] = 0xA0;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let surrogate_prefix_result = unsafe {
    mbrtowc(
      &raw mut output,
      continuation.as_ptr().cast::<c_char>(),
      sz(continuation.len()),
      &raw mut state,
    )
  };

  assert_eq!(surrogate_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xF4, 0x90, 0, 0], pending_len=2, expected_len=4.
  // For 0xF4 lead, second byte must be <= 0x8F.
  raw_state[0] = 0xF4;
  raw_state[1] = 0x90;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 4;

  set_errno(0);

  let trailing = [0x80_u8, 0x80];
  // SAFETY: pointers are valid and input is readable.
  let out_of_range_prefix_result = unsafe {
    mbrtowc(
      &raw mut output,
      trailing.as_ptr().cast::<c_char>(),
      sz(trailing.len()),
      &raw mut state,
    )
  };

  assert_eq!(out_of_range_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xF0, 0x80, 0, 0], pending_len=2, expected_len=4.
  // For 0xF0 lead, second byte must be >= 0x90.
  raw_state[0] = 0xF0;
  raw_state[1] = 0x80;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 4;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let too_low_second_byte_result = unsafe {
    mbrtowc(
      &raw mut output,
      trailing.as_ptr().cast::<c_char>(),
      sz(trailing.len()),
      &raw mut state,
    )
  };

  assert_eq!(too_low_second_byte_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_corrupted_pending_second_byte_bounds_then_retries_same_input() {
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();
  let input = [b'A'];

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xE0;
  raw_state[1] = 0x80;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable.
  let retried = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_where_pending_exceeds_expected() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the expected length so pending length is larger than expected.
  raw_state[5] = 1;

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_where_pending_equals_expected() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the expected length so pending and expected become equal.
  raw_state[5] = raw_state[4];

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_where_pending_equals_expected() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the expected length so pending and expected become equal.
  raw_state[5] = raw_state[4];

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_where_pending_exceeds_expected() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the expected length so pending length is larger than expected.
  raw_state[5] = 1;

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_where_pending_expected_lengths_mismatch() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only expected length: lead byte 0xE3 implies expected_len=3.
  raw_state[5] = 4;

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_where_pending_expected_lengths_mismatch() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only expected length: lead byte 0xE3 implies expected_len=3.
  raw_state[5] = 4;

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_preserves_valid_pending_state_as_incomplete() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [0x81_u8, 0x82];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let state_after_initial = unsafe { mbsinit(&raw const state) };

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let zero_len = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let state_after_zero_len = unsafe { mbsinit(&raw const state) };

  // SAFETY: pointers are valid and continuation bytes are readable.
  let resumed = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let state_after_resume = unsafe { mbsinit(&raw const state) };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  assert_eq!(state_after_initial, 0);
  assert_eq!(zero_len, MBR_ERR_INCOMPLETE);
  assert_eq!(state_after_zero_len, 0);
  assert_eq!(resumed, sz(2));
  assert_eq!(output, HIRAGANA_A);
  assert_eq!(state_after_resume, 1);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_pending_but_zero_expected_length() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only expected length: keep pending byte count non-zero.
  raw_state[5] = 0;

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_pending_but_zero_expected_length() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only expected length: keep pending byte count non-zero.
  raw_state[5] = 0;

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_nonzero_expected_and_zero_pending() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[4] = 0;
  raw_state[5] = 2;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_zero_lengths_and_stale_bytes() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: initial-length fields with stale carry bytes.
  raw_state[0] = 0x41;
  raw_state[4] = 0;
  raw_state[5] = 0;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_zero_lengths_and_stale_bytes_then_retries_same_input() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: initial-length fields with stale carry bytes.
  raw_state[0] = 0x41;
  raw_state[4] = 0;
  raw_state[5] = 0;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let first = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_zero_lengths_and_stale_bytes() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: initial-length fields with stale carry bytes.
  raw_state[0] = 0x41;
  raw_state[4] = 0;
  raw_state[5] = 0;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_partial_sequence_rejects_invalid_second_byte_bounds_as_eilseq() {
  let first_chunk = [0xE0_u8];
  let second_chunk = [0x80_u8];
  let resume_chunk = [b'Q'];
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrlen(
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_partial_sequence_rejects_invalid_surrogate_second_byte_upper_bound_as_eilseq() {
  let first_chunk = [0xED_u8];
  let second_chunk = [0xA0_u8];
  let resume_chunk = [b'Q'];
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrlen(
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_partial_sequence_rejects_invalid_surrogate_second_byte_with_trailing_input_as_eilseq() {
  let first_chunk = [0xED_u8];
  let second_chunk = [0xA0_u8, b'Q'];
  let resume_chunk = [b'Q'];
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrlen(
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_partial_sequence_rejects_invalid_four_byte_second_byte_upper_bound_as_eilseq() {
  let first_chunk = [0xF4_u8];
  let second_chunk = [0x90_u8];
  let resume_chunk = [b'Q'];
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrlen(
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_partial_sequence_rejects_invalid_four_byte_second_byte_lower_bound_as_eilseq() {
  let first_chunk = [0xF0_u8];
  let second_chunk = [0x80_u8];
  let resume_chunk = [b'Q'];
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let first_result = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(first_result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and second chunk is readable.
  let second_result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(second_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and resume chunk is readable.
  let resume_result = unsafe {
    mbrlen(
      resume_chunk.as_ptr().cast::<c_char>(),
      sz(resume_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(resume_result, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_rejects_state_with_nonzero_expected_and_zero_pending() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[4] = 0;
  raw_state[5] = 2;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_expected_length_above_utf8_max() {
  let input = [0x80_u8, 0x80, 0x80, 0x80];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xF0;
  raw_state[4] = 1;
  raw_state[5] = 5;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_expected_length_above_utf8_max() {
  let input = [0x80_u8, 0x80, 0x80, 0x80];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xF0;
  raw_state[4] = 1;
  raw_state[5] = 5;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_ascii_pending_but_multibyte_expected_length() {
  let input = [0x80_u8];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: ASCII lead implies expected_len=1, not 2.
  raw_state[0] = b'A';
  raw_state[4] = 1;
  raw_state[5] = 2;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_ascii_pending_but_multibyte_expected_length() {
  let input = [0x80_u8];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: ASCII lead implies expected_len=1, not 2.
  raw_state[0] = b'A';
  raw_state[4] = 1;
  raw_state[5] = 2;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_multibyte_pending_but_single_byte_expected_length() {
  let input = [0x80_u8];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: multibyte lead implies expected_len >= 2, not 1.
  raw_state[0] = 0xE3;
  raw_state[4] = 1;
  raw_state[5] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_multibyte_pending_but_single_byte_expected_length_then_retries_same_input()
 {
  let input = [0xE3_u8, 0x81, 0x82];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: multibyte lead implies expected_len >= 2, not 1.
  raw_state[0] = 0xE3;
  raw_state[4] = 1;
  raw_state[5] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let first = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and full input is readable.
  let retried = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(input.len()));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_multibyte_pending_but_single_byte_expected_length_then_retries_same_input()
 {
  let input = [0xE3_u8, 0x81, 0x82];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: multibyte lead implies expected_len >= 2, not 1.
  raw_state[0] = 0xE3;
  raw_state[4] = 1;
  raw_state[5] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let first = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and full input is readable.
  let retried = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(input.len()));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_nonzero_trailing_bytes_beyond_pending() {
  let input = [0x81_u8, 0x82];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted pending state: bytes beyond pending_len carry stale data.
  raw_state[0] = 0xE3;
  raw_state[2] = 0x80;
  raw_state[4] = 1;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_pending_but_zero_expected_length() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only expected length: keep pending byte count non-zero.
  raw_state[5] = 0;

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_preserves_valid_pending_state_as_incomplete() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [0x81_u8, 0x82];
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let state_after_initial = unsafe { mbsinit(&raw const state) };

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let zero_len = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let state_after_zero_len = unsafe { mbsinit(&raw const state) };

  // SAFETY: pointers are valid and continuation bytes are readable.
  let resumed = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };
  // SAFETY: state pointer is valid.
  let state_after_resume = unsafe { mbsinit(&raw const state) };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  assert_eq!(state_after_initial, 0);
  assert_eq!(zero_len, MBR_ERR_INCOMPLETE);
  assert_eq!(state_after_zero_len, 0);
  assert_eq!(resumed, sz(2));
  assert_eq!(state_after_resume, 1);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_with_zero_n_rejects_corrupted_pending_lead_prefix() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the buffered lead byte, preserving length counters.
  raw_state[0] = 0x80;

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_corrupted_pending_lead_prefix() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the buffered lead byte, preserving length counters.
  raw_state[0] = 0x80;

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_corrupted_pending_second_byte_bounds() {
  let mut state = mbstate_t::new();
  let continuation = [0x80_u8];

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xE0;
  raw_state[1] = 0x80;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let overlong_prefix_result = unsafe {
    mbrlen(
      continuation.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(overlong_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xED, 0xA0, 0, 0], pending_len=2, expected_len=3.
  // For 0xED lead, second byte must be <= 0x9F.
  raw_state[0] = 0xED;
  raw_state[1] = 0xA0;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let surrogate_prefix_result = unsafe {
    mbrlen(
      continuation.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(surrogate_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xF4, 0x90, 0, 0], pending_len=2, expected_len=4.
  // For 0xF4 lead, second byte must be <= 0x8F.
  raw_state[0] = 0xF4;
  raw_state[1] = 0x90;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 4;

  set_errno(0);

  let trailing = [0x80_u8, 0x80];
  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let out_of_range_prefix_result =
    unsafe { mbrlen(trailing.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(out_of_range_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xF0, 0x80, 0, 0], pending_len=2, expected_len=4.
  // For 0xF0 lead, second byte must be >= 0x90.
  raw_state[0] = 0xF0;
  raw_state[1] = 0x80;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 4;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let too_low_second_byte_result =
    unsafe { mbrlen(trailing.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(too_low_second_byte_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_corrupted_pending_second_byte_bounds() {
  let mut state = mbstate_t::new();
  let continuation = [0x80_u8];

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xE0;
  raw_state[1] = 0x80;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let overlong_prefix_result = unsafe {
    mbrlen(
      continuation.as_ptr().cast::<c_char>(),
      sz(continuation.len()),
      &raw mut state,
    )
  };

  assert_eq!(overlong_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xED, 0xA0, 0, 0], pending_len=2, expected_len=3.
  // For 0xED lead, second byte must be <= 0x9F.
  raw_state[0] = 0xED;
  raw_state[1] = 0xA0;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let surrogate_prefix_result = unsafe {
    mbrlen(
      continuation.as_ptr().cast::<c_char>(),
      sz(continuation.len()),
      &raw mut state,
    )
  };

  assert_eq!(surrogate_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xF4, 0x90, 0, 0], pending_len=2, expected_len=4.
  // For 0xF4 lead, second byte must be <= 0x8F.
  raw_state[0] = 0xF4;
  raw_state[1] = 0x90;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 4;

  set_errno(0);

  let trailing = [0x80_u8, 0x80];
  // SAFETY: pointers are valid and input is readable.
  let out_of_range_prefix_result = unsafe {
    mbrlen(
      trailing.as_ptr().cast::<c_char>(),
      sz(trailing.len()),
      &raw mut state,
    )
  };

  assert_eq!(out_of_range_prefix_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // bytes=[0xF0, 0x80, 0, 0], pending_len=2, expected_len=4.
  // For 0xF0 lead, second byte must be >= 0x90.
  raw_state[0] = 0xF0;
  raw_state[1] = 0x80;
  raw_state[2] = 0;
  raw_state[3] = 0;
  raw_state[4] = 2;
  raw_state[5] = 4;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let too_low_second_byte_result = unsafe {
    mbrlen(
      trailing.as_ptr().cast::<c_char>(),
      sz(trailing.len()),
      &raw mut state,
    )
  };

  assert_eq!(too_low_second_byte_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_corrupted_pending_second_byte_bounds_then_retries_same_input() {
  let mut state = mbstate_t::new();
  let input = [b'A'];

  // bytes=[0xE0, 0x80, 0, 0], pending_len=2, expected_len=3.
  // For 0xE0 lead, second byte must be >= 0xA0.
  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  raw_state[0] = 0xE0;
  raw_state[1] = 0x80;
  raw_state[4] = 2;
  raw_state[5] = 3;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable.
  let first = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable.
  let retried = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_where_pending_equals_expected() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the expected length so pending and expected become equal.
  raw_state[5] = raw_state[4];

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_where_pending_equals_expected() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the expected length so pending and expected become equal.
  raw_state[5] = raw_state[4];

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_where_pending_exceeds_expected() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the expected length so pending length is larger than expected.
  raw_state[5] = 1;

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_where_pending_exceeds_expected() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only the expected length so pending length is larger than expected.
  raw_state[5] = 1;

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_where_pending_expected_lengths_mismatch() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only expected length: lead byte 0xE3 implies expected_len=3.
  raw_state[5] = 4;

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_where_pending_expected_lengths_mismatch() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only expected length: lead byte 0xE3 implies expected_len=3.
  raw_state[5] = 4;

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_pending_but_zero_expected_length() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: pointers are valid and first chunk is readable.
  let initial = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(initial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };
  // Corrupt only expected length: keep pending byte count non-zero.
  raw_state[5] = 0;

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_matches_mbrtowc_null_output_for_success_incomplete_and_error() {
  let success_input = [0xF0_u8, 0x9F, 0x8D, 0xA3];
  let incomplete_input = [0xF0_u8, 0x9F, 0x8D];
  let invalid_input = [0xF5_u8];
  let mut success_state_len = mbstate_t::new();
  let mut success_state_wc = mbstate_t::new();
  let mut incomplete_state_len = mbstate_t::new();
  let mut incomplete_state_wc = mbstate_t::new();
  let mut invalid_state_len = mbstate_t::new();
  let mut invalid_state_wc = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and buffers are readable for the requested lengths.
  let success_len = unsafe {
    mbrlen(
      success_input.as_ptr().cast::<c_char>(),
      sz(success_input.len()),
      &raw mut success_state_len,
    )
  };
  // SAFETY: pointers are valid and buffers are readable for the requested lengths.
  let success_wc = unsafe {
    mbrtowc(
      ptr::null_mut(),
      success_input.as_ptr().cast::<c_char>(),
      sz(success_input.len()),
      &raw mut success_state_wc,
    )
  };
  // SAFETY: pointers are valid and buffers are readable for the requested lengths.
  let incomplete_len = unsafe {
    mbrlen(
      incomplete_input.as_ptr().cast::<c_char>(),
      sz(incomplete_input.len()),
      &raw mut incomplete_state_len,
    )
  };
  // SAFETY: pointers are valid and buffers are readable for the requested lengths.
  let incomplete_wc = unsafe {
    mbrtowc(
      ptr::null_mut(),
      incomplete_input.as_ptr().cast::<c_char>(),
      sz(incomplete_input.len()),
      &raw mut incomplete_state_wc,
    )
  };

  set_errno(0);

  // SAFETY: pointers are valid and buffers are readable for the requested lengths.
  let invalid_len = unsafe {
    mbrlen(
      invalid_input.as_ptr().cast::<c_char>(),
      sz(invalid_input.len()),
      &raw mut invalid_state_len,
    )
  };

  set_errno(0);

  // SAFETY: pointers are valid and buffers are readable for the requested lengths.
  let invalid_wc = unsafe {
    mbrtowc(
      ptr::null_mut(),
      invalid_input.as_ptr().cast::<c_char>(),
      sz(invalid_input.len()),
      &raw mut invalid_state_wc,
    )
  };

  assert_eq!(success_len, success_wc);
  assert_eq!(success_len, sz(4));
  assert_eq!(incomplete_len, incomplete_wc);
  assert_eq!(incomplete_len, MBR_ERR_INCOMPLETE);
  assert_eq!(invalid_len, invalid_wc);
  assert_eq!(invalid_len, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mbrlen_with_null_input_resets_state_to_initial() {
  let first_chunk = [0xE3_u8, 0x81];
  let mut state = mbstate_t::new();
  let mut output: wchar_t = -1;

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable.
  let partial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);

  // SAFETY: null `s` requests reset semantics for `mbrlen`.
  let reset = unsafe { mbrlen(ptr::null(), sz(0), &raw mut state) };

  assert_eq!(reset, 0);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_with_zero_n_preserves_internal_thread_local_state_when_ps_is_null() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [0x81_u8, 0x82];

  reset_internal_state();
  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let first = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      ptr::null_mut(),
    )
  };

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let zero_probe = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      ptr::null_mut(),
    )
  };

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let resumed = unsafe {
    mbrlen(
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(first, MBR_ERR_INCOMPLETE);
  assert_eq!(zero_probe, MBR_ERR_INCOMPLETE);
  assert_eq!(resumed, sz(2));
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  reset_internal_state();
}

#[test]
fn mbrtowc_with_zero_n_preserves_internal_thread_local_state_when_ps_is_null() {
  let first_chunk = [0xE3_u8];
  let second_chunk = [0x81_u8, 0x82];
  let mut output: wchar_t = -1;

  reset_internal_state();
  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      ptr::null_mut(),
    )
  };

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let zero_probe = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      ptr::null_mut(),
    )
  };

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let resumed = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(first, MBR_ERR_INCOMPLETE);
  assert_eq!(zero_probe, MBR_ERR_INCOMPLETE);
  assert_eq!(resumed, sz(2));
  assert_eq!(output, HIRAGANA_A);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  reset_internal_state();
}

#[test]
fn mbrtowc_with_zero_n_does_not_seed_internal_state_when_ps_is_null() {
  let first_chunk = [0xE3_u8];
  let continuation_only = [0x81_u8, 0x82];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;

  reset_internal_state();
  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let zero_probe = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      ptr::null_mut(),
    )
  };

  assert_eq!(zero_probe, MBR_ERR_INCOMPLETE);
  assert_eq!(output, output_sentinel);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let continuation_result = unsafe {
    mbrtowc(
      &raw mut output,
      continuation_only.as_ptr().cast::<c_char>(),
      sz(continuation_only.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(continuation_result, MBR_ERR_INVALID);
  assert_eq!(output, output_sentinel);
  assert_eq!(errno_value(), EILSEQ);

  reset_internal_state();
}

#[test]
fn mbrtowc_with_zero_n_does_not_seed_explicit_state_when_initial() {
  let first_chunk = [0xE3_u8];
  let continuation_only = [0x81_u8, 0x82];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let zero_probe = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(zero_probe, MBR_ERR_INCOMPLETE);
  assert_eq!(output, output_sentinel);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for the requested length.
  let continuation_result = unsafe {
    mbrtowc(
      &raw mut output,
      continuation_only.as_ptr().cast::<c_char>(),
      sz(continuation_only.len()),
      &raw mut state,
    )
  };

  assert_eq!(continuation_result, MBR_ERR_INVALID);
  assert_eq!(output, output_sentinel);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_does_not_seed_internal_state_when_ps_is_null() {
  let first_chunk = [0xE3_u8];
  let continuation_only = [0x81_u8, 0x82];

  reset_internal_state();
  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let zero_probe = unsafe {
    mbrlen(
      first_chunk.as_ptr().cast::<c_char>(),
      sz(0),
      ptr::null_mut(),
    )
  };

  assert_eq!(zero_probe, MBR_ERR_INCOMPLETE);
  assert_eq!(errno_value(), ERRNO_SENTINEL);

  set_errno(0);

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let continuation_result = unsafe {
    mbrlen(
      continuation_only.as_ptr().cast::<c_char>(),
      sz(continuation_only.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(continuation_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);

  reset_internal_state();
}

#[test]
fn mbrlen_with_zero_n_does_not_seed_explicit_state_when_initial() {
  let first_chunk = [0xE3_u8];
  let continuation_only = [0x81_u8, 0x82];
  let mut state = mbstate_t::new();

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let zero_probe = unsafe { mbrlen(first_chunk.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(zero_probe, MBR_ERR_INCOMPLETE);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for the requested length.
  let continuation_result = unsafe {
    mbrlen(
      continuation_only.as_ptr().cast::<c_char>(),
      sz(continuation_only.len()),
      &raw mut state,
    )
  };

  assert_eq!(continuation_result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbsinit_treats_null_as_initial_and_reports_non_initial_state() {
  let input = [0xE3_u8, 0x81];
  let mut state = mbstate_t::new();
  let mut output: wchar_t = -1;

  // SAFETY: null state pointer is explicitly allowed by `mbsinit`.
  assert_ne!(unsafe { mbsinit(ptr::null()) }, 0);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // SAFETY: pointers are valid and input is readable.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INCOMPLETE);
  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbsinit_reports_non_initial_for_corrupted_zero_lengths_with_stale_bytes() {
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: zero pending/expected lengths with stale payload byte.
  raw_state[0] = 0x41;
  raw_state[4] = 0;
  raw_state[5] = 0;

  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbsinit_reports_non_initial_for_nonzero_reserved_bytes() {
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbsinit_reports_non_initial_for_nonzero_second_reserved_byte() {
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  // SAFETY: state pointer is valid.
  assert_eq!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_nonzero_reserved_bytes() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_nonzero_reserved_bytes_then_retries_same_input() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_rejects_state_with_nonzero_second_reserved_byte() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_rejects_state_with_nonzero_second_reserved_byte_then_retries_same_input() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_nonzero_reserved_bytes() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_nonzero_reserved_bytes_then_retries_same_input() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_nonzero_second_reserved_byte() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_with_zero_n_rejects_state_with_nonzero_second_reserved_byte_then_retries_same_input() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(output, output_sentinel);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_rejects_state_with_nonzero_reserved_bytes() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_nonzero_reserved_bytes_then_retries_same_input() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let first = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_rejects_state_with_nonzero_second_reserved_byte() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_rejects_state_with_nonzero_second_reserved_byte_then_retries_same_input() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let first = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_nonzero_reserved_bytes() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_nonzero_reserved_bytes_then_retries_same_input() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let first = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_nonzero_second_reserved_byte() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let result = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(result, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_with_zero_n_rejects_state_with_nonzero_second_reserved_byte_then_retries_same_input() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(0);

  // SAFETY: pointers are valid; `n == 0` prevents additional input reads.
  let first = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(0), &raw mut state) };

  assert_eq!(first, MBR_ERR_INVALID);
  assert_eq!(errno_value(), EILSEQ);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  set_errno(ERRNO_SENTINEL);

  // SAFETY: pointers are valid and input is readable for one byte.
  let retried = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(retried, sz(1));
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
}

#[test]
fn mbrtowc_null_input_resets_state_with_nonzero_second_reserved_byte() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(ERRNO_SENTINEL);

  // SAFETY: null `s` requests reset; `n` is ignored by contract.
  let reset_result = unsafe { mbrtowc(&raw mut output, ptr::null(), sz(9), &raw mut state) };

  assert_eq!(reset_result, 0);
  assert_eq!(output, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  output = output_sentinel;

  // SAFETY: pointers are valid and input is readable for one byte.
  let resumed_result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(resumed_result, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_null_input_resets_state_with_nonzero_reserved_bytes() {
  let input = [b'A'];
  let output_sentinel: wchar_t = -1;
  let mut output = output_sentinel;
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(ERRNO_SENTINEL);

  // SAFETY: null `s` requests reset; `n` is ignored by contract.
  let reset_result = unsafe { mbrtowc(&raw mut output, ptr::null(), sz(9), &raw mut state) };

  assert_eq!(reset_result, 0);
  assert_eq!(output, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  output = output_sentinel;

  // SAFETY: pointers are valid and input is readable for one byte.
  let resumed_result = unsafe {
    mbrtowc(
      &raw mut output,
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(resumed_result, sz(1));
  assert_eq!(output, wchar_t::from(b'A'));
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_null_input_resets_state_with_nonzero_second_reserved_byte() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero second reserved
  // payload byte.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[7] = 1;

  set_errno(ERRNO_SENTINEL);

  // SAFETY: null `s` requests reset; `n` is ignored by contract.
  let reset_result = unsafe { mbrlen(ptr::null(), sz(11), &raw mut state) };

  assert_eq!(reset_result, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let resumed_result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(resumed_result, sz(1));
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrlen_null_input_resets_state_with_nonzero_reserved_bytes() {
  let input = [b'A'];
  let mut state = mbstate_t::new();

  // SAFETY: `mbstate_t` layout is fixed by ABI contract and verified in this
  // test module (`size_of::<mbstate_t>() == 8`).
  let raw_state = unsafe {
    core::slice::from_raw_parts_mut((&raw mut state).cast::<u8>(), size_of::<mbstate_t>())
  };

  // Corrupted state: canonical initial lengths with non-zero reserved payload.
  raw_state[4] = 0;
  raw_state[5] = 0;
  raw_state[6] = 1;

  set_errno(ERRNO_SENTINEL);

  // SAFETY: null `s` requests reset; `n` is ignored by contract.
  let reset_result = unsafe { mbrlen(ptr::null(), sz(11), &raw mut state) };

  assert_eq!(reset_result, 0);
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);

  // SAFETY: pointers are valid and input is readable for one byte.
  let resumed_result = unsafe {
    mbrlen(
      input.as_ptr().cast::<c_char>(),
      sz(input.len()),
      &raw mut state,
    )
  };

  assert_eq!(resumed_result, sz(1));
  assert_eq!(errno_value(), ERRNO_SENTINEL);
  // SAFETY: state pointer is valid.
  assert_ne!(unsafe { mbsinit(&raw const state) }, 0);
}

#[test]
fn mbrtowc_uses_internal_thread_local_state_when_ps_is_null() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [0x82_u8];
  let mut output: wchar_t = -1;

  // SAFETY: null `s` with null state requests reset of internal TLS state.
  let reset = unsafe { mbrtowc(ptr::null_mut(), ptr::null(), sz(0), ptr::null_mut()) };

  assert_eq!(reset, 0);

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let first = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      ptr::null_mut(),
    )
  };
  // SAFETY: pointers are valid and state pointer is intentionally null.
  let second = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(first, MBR_ERR_INCOMPLETE);
  assert_eq!(second, sz(1));
  assert_eq!(output, HIRAGANA_A);
}

#[test]
fn mbrtowc_internal_state_is_thread_local() {
  let first_chunk = [0xE3_u8, 0x81];
  let second_chunk = [0x82_u8];
  let mut output: wchar_t = -1;

  reset_internal_state();

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let main_partial = unsafe {
    mbrtowc(
      &raw mut output,
      first_chunk.as_ptr().cast::<c_char>(),
      sz(first_chunk.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(main_partial, MBR_ERR_INCOMPLETE);

  let child = thread::spawn(move || {
    let mut child_output: wchar_t = -1;

    reset_internal_state();
    set_errno(0);

    // SAFETY: pointers are valid and state pointer is intentionally null.
    let child_result = unsafe {
      mbrtowc(
        &raw mut child_output,
        second_chunk.as_ptr().cast::<c_char>(),
        sz(second_chunk.len()),
        ptr::null_mut(),
      )
    };

    (child_result, child_output, errno_value())
  });
  let (child_result, child_output, child_errno) = child.join().expect("child thread panicked");

  assert_eq!(child_result, MBR_ERR_INVALID);
  assert_eq!(child_output, -1);
  assert_eq!(child_errno, EILSEQ);

  // SAFETY: pointers are valid and state pointer is intentionally null.
  let main_complete = unsafe {
    mbrtowc(
      &raw mut output,
      second_chunk.as_ptr().cast::<c_char>(),
      sz(second_chunk.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(main_complete, sz(1));
  assert_eq!(output, HIRAGANA_A);

  reset_internal_state();
}

#[test]
fn invalid_multibyte_sequence_sets_errno_only_for_calling_thread() {
  let main_thread_errno = 91;
  let input = [0x80_u8];

  set_errno(main_thread_errno);

  let child = thread::spawn(move || {
    let mut child_state = mbstate_t::new();
    let mut output: wchar_t = -1;

    set_errno(0);

    // SAFETY: pointers are valid and input is readable.
    let result = unsafe {
      mbrtowc(
        &raw mut output,
        input.as_ptr().cast::<c_char>(),
        sz(input.len()),
        &raw mut child_state,
      )
    };

    (result, errno_value())
  });
  let (child_result, child_errno) = child.join().expect("child thread panicked");

  assert_eq!(child_result, MBR_ERR_INVALID);
  assert_eq!(child_errno, EILSEQ);
  assert_eq!(errno_value(), main_thread_errno);
}
