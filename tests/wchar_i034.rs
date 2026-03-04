#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_char;
use core::ptr;
use rlibc::abi::errno::EILSEQ;
use rlibc::abi::types::{c_int, size_t};
use rlibc::errno::__errno_location;
use rlibc::wchar::{mbrlen, mbrtowc, mbsinit, mbsrtowcs, mbstate_t, wchar_t, wcrtomb, wcsrtombs};

fn sz(value: usize) -> size_t {
  size_t::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize must fit into size_t on x86_64 Linux"))
}

fn errno_ptr() -> *mut c_int {
  // `__errno_location` returns writable TLS errno storage.
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

const fn write_state_bytes(state: &mut mbstate_t, raw: [u8; 8]) {
  // SAFETY: `mbstate_t` ABI layout is fixed and public for C interop;
  // tests use raw byte injection to emulate externally corrupted state.
  unsafe {
    core::ptr::copy_nonoverlapping(
      raw.as_ptr(),
      std::ptr::from_mut::<mbstate_t>(state).cast::<u8>(),
      raw.len(),
    );
  }
}

#[test]
fn mbsinit_tracks_partial_then_null_input_reset_on_explicit_state() {
  let prefix = [0xE3_u8, 0x81];
  let mut state = mbstate_t::new();
  let mut out = -1_i32;

  // SAFETY: state pointer is valid readable storage.
  let initial = unsafe { mbsinit(&raw const state) };

  assert_eq!(initial, 1);

  // SAFETY: prefix is readable and state/out pointers are valid.
  let partial = unsafe {
    mbrtowc(
      &raw mut out,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  // SAFETY: state pointer is valid readable storage.
  let during_partial = unsafe { mbsinit(&raw const state) };

  assert_eq!(during_partial, 0);

  // SAFETY: null input requests state reset.
  let reset_result = unsafe { mbrtowc(&raw mut out, ptr::null(), sz(0), &raw mut state) };

  assert_eq!(reset_result, sz(0));

  // SAFETY: state pointer is valid readable storage.
  let after_reset = unsafe { mbsinit(&raw const state) };

  assert_eq!(after_reset, 1);
}

#[test]
fn mbsinit_reports_initial_after_null_input_reset_of_corrupted_state() {
  let mut state = mbstate_t::new();
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  // SAFETY: state pointer is valid readable storage.
  let before_reset = unsafe { mbsinit(&raw const state) };

  assert_eq!(before_reset, 0);

  // SAFETY: null input requests state reset.
  let reset_result = unsafe { mbrlen(ptr::null(), sz(0), &raw mut state) };

  assert_eq!(reset_result, sz(0));

  // SAFETY: state pointer is valid readable storage.
  let after_reset = unsafe { mbsinit(&raw const state) };

  assert_eq!(after_reset, 1);
}

#[test]
fn mbsinit_reports_initial_after_wcrtomb_null_destination_reset_of_corrupted_state() {
  let mut state = mbstate_t::new();
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  // SAFETY: state pointer is valid readable storage.
  let before_reset = unsafe { mbsinit(&raw const state) };

  assert_eq!(before_reset, 0);

  set_errno(0);
  // SAFETY: null destination requests state reset and ignores `wc`.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), wchar_t::from(b'Q'), &raw mut state) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 0);

  // SAFETY: state pointer is valid readable storage.
  let after_reset = unsafe { mbsinit(&raw const state) };

  assert_eq!(after_reset, 1);
}

#[test]
fn mbsinit_reports_initial_after_wcrtomb_null_destination_reset_of_corrupted_state_with_invalid_wc()
{
  let mut state = mbstate_t::new();
  let surrogate = wchar_t::try_from(0xD800_i32).expect("surrogate fits into wchar_t");
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  // SAFETY: state pointer is valid readable storage.
  let before_reset = unsafe { mbsinit(&raw const state) };

  assert_eq!(before_reset, 0);

  set_errno(7878);
  // SAFETY: null destination requests state reset and must ignore invalid `wc`.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), surrogate, &raw mut state) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 7878);

  // SAFETY: state pointer is valid readable storage.
  let after_reset = unsafe { mbsinit(&raw const state) };

  assert_eq!(after_reset, 1);
}

#[test]
fn mbrtowc_null_input_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let mut out = -1_i32;
  let input = b"A\0";
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(3333);
  // SAFETY: null input requests state reset; `out` is writable.
  let reset_result = unsafe { mbrtowc(&raw mut out, ptr::null(), sz(0), &raw mut state) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(out, 0);
  assert_eq!(errno_value(), 3333);

  set_errno(0);
  // SAFETY: input is readable for at least one byte and `out` is writable.
  let decoded = unsafe {
    mbrtowc(
      &raw mut out,
      input.as_ptr().cast::<c_char>(),
      sz(1),
      &raw mut state,
    )
  };

  assert_eq!(decoded, sz(1));
  assert_eq!(out, i32::from(b'A'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbrtowc_null_input_with_null_pwc_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let mut out = -1_i32;
  let input = b"A\0";
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(3434);
  // SAFETY: null input requests reset; null `pwc` is explicitly allowed.
  let reset_result = unsafe { mbrtowc(ptr::null_mut(), ptr::null(), sz(0), &raw mut state) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(errno_value(), 3434);

  set_errno(0);
  // SAFETY: input is readable for at least one byte and `out` is writable.
  let decoded = unsafe {
    mbrtowc(
      &raw mut out,
      input.as_ptr().cast::<c_char>(),
      sz(1),
      &raw mut state,
    )
  };

  assert_eq!(decoded, sz(1));
  assert_eq!(out, i32::from(b'A'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbrtowc_null_input_with_null_pwc_ignores_n_and_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let mut out = -1_i32;
  let input = b"A\0";
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(3535);
  // SAFETY: null input requests reset; null `pwc` is allowed and `n` is ignored.
  let reset_result = unsafe { mbrtowc(ptr::null_mut(), ptr::null(), sz(9), &raw mut state) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(errno_value(), 3535);

  set_errno(0);
  // SAFETY: input is readable for at least one byte and `out` is writable.
  let decoded = unsafe {
    mbrtowc(
      &raw mut out,
      input.as_ptr().cast::<c_char>(),
      sz(1),
      &raw mut state,
    )
  };

  assert_eq!(decoded, sz(1));
  assert_eq!(out, i32::from(b'A'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbrtowc_null_input_ignores_n_and_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let mut out = -1_i32;
  let input = b"A\0";
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(4545);
  // SAFETY: null input requests reset; `n` must be ignored in this branch.
  let reset_result = unsafe { mbrtowc(&raw mut out, ptr::null(), sz(9), &raw mut state) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(out, 0);
  assert_eq!(errno_value(), 4545);

  set_errno(0);
  // SAFETY: input is readable for at least one byte and `out` is writable.
  let decoded = unsafe {
    mbrtowc(
      &raw mut out,
      input.as_ptr().cast::<c_char>(),
      sz(1),
      &raw mut state,
    )
  };

  assert_eq!(decoded, sz(1));
  assert_eq!(out, i32::from(b'A'));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbrlen_null_input_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let input = b"A\0";
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(4444);
  // SAFETY: null input requests state reset by API contract.
  let reset_result = unsafe { mbrlen(ptr::null(), sz(0), &raw mut state) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(errno_value(), 4444);

  set_errno(0);
  // SAFETY: input is readable for at least one byte.
  let decoded = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(1), &raw mut state) };

  assert_eq!(decoded, sz(1));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbrlen_null_input_ignores_n_and_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let input = b"A\0";
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(4747);
  // SAFETY: null input requests reset; `n` must be ignored in this branch.
  let reset_result = unsafe { mbrlen(ptr::null(), sz(9), &raw mut state) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(errno_value(), 4747);

  set_errno(0);
  // SAFETY: input is readable for at least one byte.
  let decoded = unsafe { mbrlen(input.as_ptr().cast::<c_char>(), sz(1), &raw mut state) };

  assert_eq!(decoded, sz(1));
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbrtowc_null_input_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut src = suffix.as_ptr().cast::<c_char>();
  let original = src;
  let mut dst = [0_i32; 2];

  // SAFETY: prefix is readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(5555);
  // SAFETY: null input requests reset on internal state.
  let reset_result = unsafe { mbrtowc(&raw mut scratch_wide, ptr::null(), sz(0), ptr::null_mut()) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(scratch_wide, 0);
  assert_eq!(errno_value(), 5555);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn mbrtowc_null_input_with_null_pwc_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut src = suffix.as_ptr().cast::<c_char>();
  let original = src;

  // SAFETY: prefix is readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(5656);
  // SAFETY: null input requests reset on internal state; null `pwc` is allowed.
  let reset_result = unsafe { mbrtowc(ptr::null_mut(), ptr::null(), sz(0), ptr::null_mut()) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(errno_value(), 5656);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), ptr::null_mut()) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn mbrtowc_null_input_with_null_pwc_ignores_n_and_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut src = suffix.as_ptr().cast::<c_char>();
  let original = src;

  // SAFETY: prefix is readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(5757);
  // SAFETY: null input requests reset; null `pwc` is allowed and `n` is ignored.
  let reset_result = unsafe { mbrtowc(ptr::null_mut(), ptr::null(), sz(9), ptr::null_mut()) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(errno_value(), 5757);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), ptr::null_mut()) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn mbrtowc_null_input_ignores_n_and_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut src = suffix.as_ptr().cast::<c_char>();
  let original = src;

  // SAFETY: prefix is readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(6868);
  // SAFETY: null input requests reset and this branch must ignore `n`.
  let reset_result = unsafe { mbrtowc(&raw mut scratch_wide, ptr::null(), sz(9), ptr::null_mut()) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(scratch_wide, 0);
  assert_eq!(errno_value(), 6868);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), ptr::null_mut()) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn mbrlen_null_input_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: prefix is readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrlen(
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(6666);
  // SAFETY: null input requests reset on internal state.
  let reset_result = unsafe { mbrlen(ptr::null(), sz(0), ptr::null_mut()) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(errno_value(), 6666);

  set_errno(0);
  // SAFETY: suffix is readable for one byte.
  let decoded = unsafe { mbrlen(suffix.as_ptr().cast::<c_char>(), sz(1), ptr::null_mut()) };

  assert_eq!(decoded, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mbrlen_null_input_ignores_n_and_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];

  // SAFETY: prefix is readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrlen(
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(6969);
  // SAFETY: null input requests reset and this branch must ignore `n`.
  let reset_result = unsafe { mbrlen(ptr::null(), sz(9), ptr::null_mut()) };

  assert_eq!(reset_result, sz(0));
  assert_eq!(errno_value(), 6969);

  set_errno(0);
  // SAFETY: suffix is readable for one byte.
  let decoded = unsafe { mbrlen(suffix.as_ptr().cast::<c_char>(), sz(1), ptr::null_mut()) };

  assert_eq!(decoded, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wcrtomb_encodes_ascii_byte() {
  let mut state = mbstate_t::new();
  let mut out = [0_u8; 8];

  set_errno(0);

  // SAFETY: `out` is writable and `state` is a valid conversion state object.
  let written = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(written, sz(1));
  assert_eq!(out[0], b'A');
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_encodes_four_byte_utf8_scalar() {
  let mut state = mbstate_t::new();
  let mut out = [0_u8; 8];
  let sushi = wchar_t::try_from(0x1F363_i32).expect("scalar fits into wchar_t");

  // SAFETY: `out` is writable and `state` is a valid conversion state object.
  let written = unsafe { wcrtomb(out.as_mut_ptr().cast::<c_char>(), sushi, &raw mut state) };

  assert_eq!(written, sz(4));
  assert_eq!(&out[..4], b"\xF0\x9F\x8D\xA3");
}

#[test]
fn wcrtomb_rejects_surrogate_and_sets_eilseq() {
  let mut state = mbstate_t::new();
  let mut out = [0_u8; 8];
  let surrogate = wchar_t::try_from(0xD800_i32).expect("surrogate fits into wchar_t");

  set_errno(0);

  // SAFETY: `out` is writable and `state` is a valid conversion state object.
  let written = unsafe { wcrtomb(out.as_mut_ptr().cast::<c_char>(), surrogate, &raw mut state) };

  assert_eq!(written, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wcrtomb_null_destination_resets_state_before_future_decode() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let surrogate = wchar_t::try_from(0xD800_i32).expect("surrogate fits into wchar_t");
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut src = suffix.as_ptr().cast::<c_char>();
  let mut dst = [0_i32; 2];
  let original = src;

  // SAFETY: prefix bytes are readable and state points to writable conversion state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(777);
  // SAFETY: null destination requests state reset by C contract.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), surrogate, &raw mut state) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 777);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn wcrtomb_null_destination_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let mut dst = [0_i32; 2];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(7171);
  // SAFETY: null destination requests reset and ignores `wc`.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), wchar_t::from(b'Q'), &raw mut state) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 7171);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(1));
  assert_eq!(dst[0], i32::from(b'A'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_null_destination_ignores_invalid_wchar_and_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let surrogate = wchar_t::try_from(0xD800_i32).expect("surrogate fits into wchar_t");
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let mut dst = [0_i32; 2];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(8181);
  // SAFETY: null destination requests reset and must ignore invalid `wc`.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), surrogate, &raw mut state) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 8181);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(1));
  assert_eq!(dst[0], i32::from(b'A'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_null_destination_resets_reserved_state() {
  let mut state = mbstate_t::new();
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let mut dst = [0_i32; 2];
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 1, 0];

  write_state_bytes(&mut state, reserved);
  set_errno(6161);
  // SAFETY: null destination requests reset and ignores `wc`.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), wchar_t::from(b'Q'), &raw mut state) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 6161);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(1));
  assert_eq!(dst[0], i32::from(b'A'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_null_destination_resets_second_reserved_byte_state() {
  let mut state = mbstate_t::new();
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let mut dst = [0_i32; 2];
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 0, 1];

  write_state_bytes(&mut state, reserved);
  set_errno(6262);
  // SAFETY: null destination requests reset and ignores `wc`.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), wchar_t::from(b'Q'), &raw mut state) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 6262);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(1));
  assert_eq!(dst[0], i32::from(b'A'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_non_null_destination_rejects_corrupted_state_and_resets_for_next_call() {
  let mut state = mbstate_t::new();
  let mut out = [0x55_u8; 4];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let first = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(out[0], 0x55);

  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let second = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(out[0], b'A');
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_non_null_destination_rejects_stale_zero_length_state_and_resets_for_next_call() {
  let mut state = mbstate_t::new();
  let mut out = [0x66_u8; 4];
  // bytes=[0xE3, 0, 0, 0], pending_len=0, expected_len=0 (stale bytes snapshot).
  let stale_state_bytes = [0xE3_u8, 0, 0, 0, 0, 0, 0, 0];

  write_state_bytes(&mut state, stale_state_bytes);
  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let first = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(out[0], 0x66);

  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let second = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(out[0], b'A');
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_non_null_destination_rejects_reserved_state_and_resets_for_next_call() {
  let mut state = mbstate_t::new();
  let mut out = [0x77_u8; 4];
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 1, 0];

  write_state_bytes(&mut state, reserved);
  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let first = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(out[0], 0x77);

  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let second = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(out[0], b'A');
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_non_null_destination_rejects_second_reserved_byte_state_and_resets_for_next_call() {
  let mut state = mbstate_t::new();
  let mut out = [0x55_u8; 4];
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 0, 1];

  write_state_bytes(&mut state, reserved);
  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let first = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(out[0], 0x55);

  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let second = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(out[0], b'A');
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcrtomb_non_null_destination_rejects_pending_state_and_resets_for_next_call() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut out = [0x33_u8; 4];
  let mut src = suffix.as_ptr().cast::<c_char>();
  let original_src = src;
  let mut dst = [0_i32; 2];

  // SAFETY: pointers are valid and `prefix` is readable.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let first = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(out[0], 0x33);

  set_errno(0);
  // SAFETY: output buffer and state pointers are valid.
  let second = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(errno_value(), 0);
  assert_eq!(out[0], b'A');

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original_src);
}

#[test]
fn wcrtomb_non_null_destination_rejects_internal_pending_state_and_resets_for_next_call() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut out = [0x44_u8; 4];
  let mut src = suffix.as_ptr().cast::<c_char>();
  let original_src = src;
  let mut dst = [0_i32; 2];

  // SAFETY: pointers are valid and `prefix` is readable.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: output pointer is valid and null `ps` selects internal state.
  let first = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      ptr::null_mut(),
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(out[0], 0x44);

  set_errno(0);
  // SAFETY: output pointer is valid and null `ps` selects internal state.
  let second = unsafe {
    wcrtomb(
      out.as_mut_ptr().cast::<c_char>(),
      wchar_t::from(b'A'),
      ptr::null_mut(),
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(errno_value(), 0);
  assert_eq!(out[0], b'A');

  set_errno(0);
  // SAFETY: pointers are valid and null `ps` reuses the same internal state.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original_src);
}

#[test]
fn wcrtomb_null_destination_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut src = suffix.as_ptr().cast::<c_char>();
  let mut dst = [0_i32; 2];
  let original = src;

  // SAFETY: prefix bytes are readable and null `ps` selects thread-local internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(9001);
  // SAFETY: null destination requests internal-state reset by C contract.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), wchar_t::from(b'Q'), ptr::null_mut()) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 9001);

  set_errno(0);
  // SAFETY: pointers are valid and null `ps` continues to use internal state.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn wcrtomb_null_destination_ignores_invalid_wchar_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let surrogate = wchar_t::try_from(0xD800_i32).expect("surrogate fits into wchar_t");
  let mut scratch_wide = -1_i32;
  let mut src = suffix.as_ptr().cast::<c_char>();
  let mut dst = [0_i32; 2];
  let original = src;

  // SAFETY: prefix bytes are readable and null `ps` selects thread-local internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(4242);
  // SAFETY: null destination requests internal-state reset and ignores `wc` by contract.
  let reset_result = unsafe { wcrtomb(ptr::null_mut(), surrogate, ptr::null_mut()) };

  assert_eq!(reset_result, sz(1));
  assert_eq!(errno_value(), 4242);

  set_errno(0);
  // SAFETY: pointers are valid and null `ps` continues to use internal state.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn mbsrtowcs_converts_complete_utf8_string_and_nulls_src() {
  let input = b"A\xF0\x9F\x8D\xA3\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 4];

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(2));
  assert_eq!(dst[0], i32::from(b'A'));
  assert_eq!(dst[1], 0x1F363_i32);
  assert!(src.is_null());
}

#[test]
fn mbsrtowcs_null_src_pointer_sets_eilseq() {
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 2];

  set_errno(0);

  // SAFETY: null `src` pointer must be rejected by contract.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      ptr::null_mut(),
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn mbsrtowcs_null_src_value_resets_pending_state() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut dst = [0_i32; 2];
  let mut null_src: *const c_char = ptr::null();
  let mut suffix_src = suffix.as_ptr().cast::<c_char>();
  let original_suffix = suffix_src;

  // SAFETY: prefix bytes are readable and `state` is valid writable storage.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(31337);
  // SAFETY: `null_src` pointer storage is valid; `*src == NULL` requests reset path.
  let reset_call = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut null_src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(reset_call, sz(0));
  assert!(null_src.is_null());
  assert_eq!(errno_value(), 31337);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut suffix_src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(suffix_src, original_suffix);
}

#[test]
fn mbsrtowcs_null_src_value_resets_pending_state_when_dst_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut null_src: *const c_char = ptr::null();
  let mut suffix_src = suffix.as_ptr().cast::<c_char>();
  let original_suffix = suffix_src;

  // SAFETY: prefix bytes are readable and `state` is valid writable storage.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(2626);
  // SAFETY: pointer storage is valid and `*src == NULL` triggers reset path.
  let reset_call = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut null_src, sz(0), &raw mut state) };

  assert_eq!(reset_call, sz(0));
  assert!(null_src.is_null());
  assert_eq!(errno_value(), 2626);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut suffix_src, sz(0), &raw mut state) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(suffix_src, original_suffix);
}

#[test]
fn mbsrtowcs_null_src_value_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut dst = [0_i32; 2];
  let mut null_src: *const c_char = ptr::null();
  let mut suffix_src = suffix.as_ptr().cast::<c_char>();
  let original_suffix = suffix_src;

  // SAFETY: prefix bytes are readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(8181);
  // SAFETY: pointer storage is valid and null `ps` selects internal reset path.
  let reset_call = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut null_src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(reset_call, sz(0));
  assert!(null_src.is_null());
  assert_eq!(errno_value(), 8181);

  set_errno(0);
  // SAFETY: pointers are valid and null `ps` reuses internal restart state.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut suffix_src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(suffix_src, original_suffix);
}

#[test]
fn mbsrtowcs_null_src_value_resets_internal_state_when_ps_is_null_and_dst_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut null_src: *const c_char = ptr::null();
  let mut suffix_src = suffix.as_ptr().cast::<c_char>();
  let original_suffix = suffix_src;

  // SAFETY: prefix bytes are readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(2020);
  // SAFETY: pointer storage is valid and null `ps` selects internal reset path.
  let reset_call = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut null_src, sz(0), ptr::null_mut()) };

  assert_eq!(reset_call, sz(0));
  assert!(null_src.is_null());
  assert_eq!(errno_value(), 2020);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted =
    unsafe { mbsrtowcs(ptr::null_mut(), &raw mut suffix_src, sz(0), ptr::null_mut()) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(suffix_src, original_suffix);
}

#[test]
fn mbsrtowcs_null_src_value_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let mut null_src: *const c_char = ptr::null();
  let mut dst = [0_i32; 2];
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(9090);
  // SAFETY: pointer storage is valid and `*src == NULL` triggers reset path.
  let reset_call = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut null_src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(reset_call, sz(0));
  assert!(null_src.is_null());
  assert_eq!(errno_value(), 9090);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(1));
  assert_eq!(dst[0], i32::from(b'A'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_null_src_value_resets_corrupted_state_when_dst_null() {
  let mut state = mbstate_t::new();
  let mut null_src: *const c_char = ptr::null();
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(1212);
  // SAFETY: pointer storage is valid and `*src == NULL` triggers reset path.
  let reset_call = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut null_src, sz(0), &raw mut state) };

  assert_eq!(reset_call, sz(0));
  assert!(null_src.is_null());
  assert_eq!(errno_value(), 1212);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(converted, sz(1));
  assert_eq!(src, input.as_ptr().cast::<c_char>());
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_respects_output_limit_and_updates_src() {
  let input = b"ab\xF0\x9F\x8D\xA3\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 2];

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(2));
  assert_eq!(dst, [i32::from(b'a'), i32::from(b'b')]);
  assert!(!src.is_null());
  // SAFETY: `src` still points into `input`.
  assert_eq!(unsafe { *src.cast::<u8>() }, 0xF0);
}

#[test]
fn mbsrtowcs_zero_len_does_not_validate_or_advance_src() {
  let input = [0xFF_u8, 0_u8];
  let mut src = input.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 1];
  let original = src;

  set_errno(6161);

  // SAFETY: pointers are valid; `len == 0` should short-circuit before decoding.
  let converted = unsafe { mbsrtowcs(dst.as_mut_ptr(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(converted, sz(0));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 6161);
}

#[test]
fn mbsrtowcs_zero_len_does_not_validate_or_advance_src_with_corrupted_state() {
  let input = [0xFF_u8, 0_u8];
  let mut src = input.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 1];
  let original = src;
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  // SAFETY: state pointer is valid readable storage.
  let before = unsafe { mbsinit(&raw const state) };

  assert_eq!(before, 0);

  set_errno(6262);
  // SAFETY: pointers are valid; `len == 0` should short-circuit before validating state/input.
  let converted = unsafe { mbsrtowcs(dst.as_mut_ptr(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(converted, sz(0));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 6262);

  // SAFETY: state pointer is valid readable storage.
  let after = unsafe { mbsinit(&raw const state) };

  assert_eq!(after, 0);
}

#[test]
fn mbsrtowcs_null_dst_counts_without_advancing_src() {
  let input = b"ab\xF0\x9F\x8D\xA3\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let original = src;

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(converted, sz(3));
  assert_eq!(src, original);
}

#[test]
fn mbsrtowcs_null_dst_ignores_len_parameter() {
  let input = b"ab\xF0\x9F\x8D\xA3\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let original = src;

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(1), &raw mut state) };

  assert_eq!(converted, sz(3));
  assert_eq!(src, original);
}

#[test]
fn mbsrtowcs_invalid_sequence_sets_eilseq_and_keeps_src() {
  let input = [0xFF_u8, 0_u8];
  let mut src = input.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 2];
  let original = src;

  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn mbsrtowcs_corrupted_state_sets_eilseq_and_resets_for_next_call() {
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let original = src;
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 2];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(0);

  // SAFETY: pointers are valid and input is NUL-terminated.
  let first = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let second = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(dst[0], i32::from(b'A'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_null_dst_corrupted_state_sets_eilseq_and_resets_for_next_call() {
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let original = src;
  let mut state = mbstate_t::new();
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(0);

  // SAFETY: pointers are valid and input is NUL-terminated.
  let first = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let second = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(second, sz(1));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_reserved_state_sets_eilseq_and_resets_for_next_call() {
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let original = src;
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 2];
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 1, 0];

  write_state_bytes(&mut state, reserved);
  set_errno(0);

  // SAFETY: pointers are valid and input is NUL-terminated.
  let first = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let second = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(dst[0], i32::from(b'A'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_null_dst_reserved_state_sets_eilseq_and_resets_for_next_call() {
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let original = src;
  let mut state = mbstate_t::new();
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 1, 0];

  write_state_bytes(&mut state, reserved);
  set_errno(0);

  // SAFETY: pointers are valid and input is NUL-terminated.
  let first = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let second = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(second, sz(1));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_reserved_second_byte_state_sets_eilseq_and_resets_for_next_call() {
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let original = src;
  let mut state = mbstate_t::new();
  let mut dst = [0_i32; 2];
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 0, 1];

  write_state_bytes(&mut state, reserved);
  set_errno(0);

  // SAFETY: pointers are valid and input is NUL-terminated.
  let first = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let second = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(dst[0], i32::from(b'A'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_null_dst_reserved_second_byte_state_sets_eilseq_and_resets_for_next_call() {
  let input = b"A\0";
  let mut src = input.as_ptr().cast::<c_char>();
  let original = src;
  let mut state = mbstate_t::new();
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 0, 1];

  write_state_bytes(&mut state, reserved);
  set_errno(0);

  // SAFETY: pointers are valid and input is NUL-terminated.
  let first = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and input is NUL-terminated.
  let second = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(second, sz(1));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_invalid_sequence_resets_pending_state() {
  let prefix = [0xE3_u8, 0x81];
  let invalid = [0xFF_u8, 0_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut invalid_src = invalid.as_ptr().cast::<c_char>();
  let original_invalid_src = invalid_src;
  let mut suffix_src = suffix.as_ptr().cast::<c_char>();
  let original_suffix_src = suffix_src;
  let mut dst = [0_i32; 2];

  // SAFETY: prefix bytes are readable and `state` is valid writable storage.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: pointers are valid and `invalid` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut invalid_src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(invalid_src, original_invalid_src);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let resumed = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut suffix_src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(resumed, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(suffix_src, original_suffix_src);
}

#[test]
fn mbsrtowcs_invalid_sequence_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let invalid = [0xFF_u8, 0_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut invalid_src = invalid.as_ptr().cast::<c_char>();
  let original_invalid_src = invalid_src;
  let mut suffix_src = suffix.as_ptr().cast::<c_char>();
  let original_suffix_src = suffix_src;
  let mut dst = [0_i32; 2];

  // SAFETY: prefix bytes are readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: pointers are valid and `invalid` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut invalid_src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(invalid_src, original_invalid_src);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let resumed = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut suffix_src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(resumed, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(suffix_src, original_suffix_src);
}

#[test]
fn mbsrtowcs_null_dst_invalid_sequence_sets_eilseq_and_keeps_src() {
  let input = [0xFF_u8, 0_u8];
  let mut src = input.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let original = src;

  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn mbsrtowcs_null_dst_invalid_sequence_resets_pending_state() {
  let prefix = [0xE3_u8, 0x81];
  let invalid = [0xFF_u8, 0_u8];
  let suffix = [0x82_u8, 0_u8];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut invalid_src = invalid.as_ptr().cast::<c_char>();
  let original_invalid_src = invalid_src;
  let mut suffix_src = suffix.as_ptr().cast::<c_char>();
  let original_suffix_src = suffix_src;

  // SAFETY: prefix bytes are readable and `state` is valid writable storage.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: pointers are valid and `invalid` is NUL-terminated.
  let converted =
    unsafe { mbsrtowcs(ptr::null_mut(), &raw mut invalid_src, sz(0), &raw mut state) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(invalid_src, original_invalid_src);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let resumed = unsafe { mbsrtowcs(ptr::null_mut(), &raw mut suffix_src, sz(0), &raw mut state) };

  assert_eq!(resumed, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(suffix_src, original_suffix_src);
}

#[test]
fn mbsrtowcs_resumes_from_pending_state() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, b'B', 0_u8];
  let mut src = suffix.as_ptr().cast::<c_char>();
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut dst = [0_i32; 4];

  // SAFETY: pointers are valid and prefix bytes are readable.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);

  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(2));
  assert_eq!(dst[0], 0x3042_i32);
  assert_eq!(dst[1], i32::from(b'B'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn mbsrtowcs_resumes_from_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, b'Z', 0_u8];
  let mut scratch_wide = -1_i32;
  let mut src = suffix.as_ptr().cast::<c_char>();
  let mut dst = [0_i32; 4];

  // SAFETY: prefix bytes are readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);

  // SAFETY: `src` points to a NUL-terminated buffer and null `ps` reuses
  // internal state populated by the prior `mbrtowc` call.
  let converted = unsafe {
    mbsrtowcs(
      dst.as_mut_ptr(),
      &raw mut src,
      sz(dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, sz(2));
  assert_eq!(dst[0], 0x3042_i32);
  assert_eq!(dst[1], i32::from(b'Z'));
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_null_src_pointer_sets_eilseq() {
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 4];

  set_errno(0);

  // SAFETY: null `src` pointer must be rejected by contract.
  let converted = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      ptr::null_mut(),
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
}

#[test]
fn wcsrtombs_null_src_value_resets_pending_state() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut null_wide_src: *const wchar_t = ptr::null();
  let mut multibyte_src = suffix.as_ptr().cast::<c_char>();
  let mut wide_dst = [0_i32; 2];
  let mut narrow_dst = [0_u8; 4];
  let original_multibyte_src = multibyte_src;

  // SAFETY: prefix bytes are readable and `state` is valid writable storage.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(5150);
  // SAFETY: `null_wide_src` points to readable pointer storage with `*src == NULL`.
  let reset_result = unsafe {
    wcsrtombs(
      narrow_dst.as_mut_ptr().cast::<c_char>(),
      &raw mut null_wide_src,
      sz(narrow_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(reset_result, sz(0));
  assert!(null_wide_src.is_null());
  assert_eq!(errno_value(), 5150);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      wide_dst.as_mut_ptr(),
      &raw mut multibyte_src,
      sz(wide_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(multibyte_src, original_multibyte_src);
}

#[test]
fn wcsrtombs_null_src_value_resets_pending_state_when_dst_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut null_wide_src: *const wchar_t = ptr::null();
  let mut multibyte_src = suffix.as_ptr().cast::<c_char>();
  let mut wide_dst = [0_i32; 2];
  let original_multibyte_src = multibyte_src;

  // SAFETY: prefix bytes are readable and `state` is valid writable storage.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(3131);
  // SAFETY: pointer storage is valid and `*src == NULL` triggers reset path.
  let reset_result = unsafe {
    wcsrtombs(
      ptr::null_mut(),
      &raw mut null_wide_src,
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(reset_result, sz(0));
  assert!(null_wide_src.is_null());
  assert_eq!(errno_value(), 3131);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      wide_dst.as_mut_ptr(),
      &raw mut multibyte_src,
      sz(wide_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(multibyte_src, original_multibyte_src);
}

#[test]
fn wcsrtombs_null_src_value_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut null_wide_src: *const wchar_t = ptr::null();
  let mut multibyte_src = suffix.as_ptr().cast::<c_char>();
  let mut wide_dst = [0_i32; 2];
  let mut narrow_dst = [0_u8; 4];
  let original_multibyte_src = multibyte_src;

  // SAFETY: prefix bytes are readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(9191);
  // SAFETY: `null_wide_src` points to valid pointer storage and null `ps` uses internal state.
  let reset_result = unsafe {
    wcsrtombs(
      narrow_dst.as_mut_ptr().cast::<c_char>(),
      &raw mut null_wide_src,
      sz(narrow_dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(reset_result, sz(0));
  assert!(null_wide_src.is_null());
  assert_eq!(errno_value(), 9191);

  set_errno(0);
  // SAFETY: pointers are valid and null `ps` reuses internal restart state.
  let converted = unsafe {
    mbsrtowcs(
      wide_dst.as_mut_ptr(),
      &raw mut multibyte_src,
      sz(wide_dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(multibyte_src, original_multibyte_src);
}

#[test]
fn wcsrtombs_null_src_value_resets_internal_state_when_ps_is_null_and_dst_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let mut scratch_wide = -1_i32;
  let mut null_wide_src: *const wchar_t = ptr::null();
  let mut multibyte_src = suffix.as_ptr().cast::<c_char>();
  let mut wide_dst = [0_i32; 2];
  let original_multibyte_src = multibyte_src;

  // SAFETY: prefix bytes are readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(1414);
  // SAFETY: pointer storage is valid and null `ps` selects internal reset path.
  let reset_result = unsafe {
    wcsrtombs(
      ptr::null_mut(),
      &raw mut null_wide_src,
      sz(0),
      ptr::null_mut(),
    )
  };

  assert_eq!(reset_result, sz(0));
  assert!(null_wide_src.is_null());
  assert_eq!(errno_value(), 1414);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      wide_dst.as_mut_ptr(),
      &raw mut multibyte_src,
      sz(wide_dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(multibyte_src, original_multibyte_src);
}

#[test]
fn wcsrtombs_null_src_value_resets_corrupted_state() {
  let mut state = mbstate_t::new();
  let mut null_wide_src: *const wchar_t = ptr::null();
  let mut narrow_dst = [0_u8; 4];
  let probe_input = b"A\0";
  let mut probe_src = probe_input.as_ptr().cast::<c_char>();
  let mut probe_dst = [0_i32; 2];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(5151);
  // SAFETY: pointer storage is valid and `*src == NULL` triggers reset path.
  let reset_result = unsafe {
    wcsrtombs(
      narrow_dst.as_mut_ptr().cast::<c_char>(),
      &raw mut null_wide_src,
      sz(narrow_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(reset_result, sz(0));
  assert!(null_wide_src.is_null());
  assert_eq!(errno_value(), 5151);

  set_errno(0);
  // SAFETY: pointers are valid and `probe_input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      probe_dst.as_mut_ptr(),
      &raw mut probe_src,
      sz(probe_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(1));
  assert_eq!(probe_dst[0], i32::from(b'A'));
  assert!(probe_src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_null_src_value_resets_corrupted_state_when_dst_null() {
  let mut state = mbstate_t::new();
  let mut null_wide_src: *const wchar_t = ptr::null();
  let probe_input = b"A\0";
  let mut probe_src = probe_input.as_ptr().cast::<c_char>();
  let mut probe_dst = [0_i32; 2];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(2323);
  // SAFETY: pointer storage is valid and `*src == NULL` triggers reset path.
  let reset_result = unsafe {
    wcsrtombs(
      ptr::null_mut(),
      &raw mut null_wide_src,
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(reset_result, sz(0));
  assert!(null_wide_src.is_null());
  assert_eq!(errno_value(), 2323);

  set_errno(0);
  // SAFETY: pointers are valid and `probe_input` is NUL-terminated.
  let converted = unsafe {
    mbsrtowcs(
      probe_dst.as_mut_ptr(),
      &raw mut probe_src,
      sz(probe_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(1));
  assert_eq!(probe_dst[0], i32::from(b'A'));
  assert!(probe_src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_converts_complete_wide_string_and_nulls_src() {
  let input = [i32::from(b'A'), 0x1F363_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 16];

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(5));
  assert_eq!(&dst[..5], b"A\xF0\x9F\x8D\xA3");
  assert!(src.is_null());
}

#[test]
fn wcsrtombs_respects_output_limit_and_updates_src() {
  let input = [i32::from(b'A'), 0x1F363_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 1];

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(1));
  assert_eq!(dst[0], b'A');
  assert!(!src.is_null());
  // SAFETY: `src` still points into `input`.
  assert_eq!(unsafe { *src }, 0x1F363_i32);
}

#[test]
fn wcsrtombs_zero_len_does_not_validate_or_advance_src() {
  let input = [0xD800_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 1];
  let original = src;

  set_errno(7171);

  // SAFETY: pointers are valid; `len == 0` should short-circuit before encoding.
  let converted = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(0));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 7171);
}

#[test]
fn wcsrtombs_zero_len_does_not_validate_or_advance_src_with_corrupted_state() {
  let input = [0xD800_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 1];
  let original = src;
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  // SAFETY: state pointer is valid readable storage.
  let before = unsafe { mbsinit(&raw const state) };

  assert_eq!(before, 0);

  set_errno(7272);
  // SAFETY: pointers are valid; `len == 0` should short-circuit before validating state/input.
  let converted = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(0));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 7272);

  // SAFETY: state pointer is valid readable storage.
  let after = unsafe { mbsinit(&raw const state) };

  assert_eq!(after, 0);
}

#[test]
fn wcsrtombs_zero_len_does_not_validate_or_advance_src_with_reserved_state() {
  let input = [0xD800_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 1];
  let original = src;
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 1, 0];

  write_state_bytes(&mut state, reserved);
  // SAFETY: state pointer is valid readable storage.
  let before = unsafe { mbsinit(&raw const state) };

  assert_eq!(before, 0);

  set_errno(7373);
  // SAFETY: pointers are valid; `len == 0` should short-circuit before validating state/input.
  let converted = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(0));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 7373);

  // SAFETY: state pointer is valid readable storage.
  let after = unsafe { mbsinit(&raw const state) };

  assert_eq!(after, 0);
}

#[test]
fn wcsrtombs_zero_len_does_not_validate_or_advance_src_with_second_reserved_byte_state() {
  let input = [0xD800_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 1];
  let original = src;
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 0, 1];

  write_state_bytes(&mut state, reserved);
  // SAFETY: state pointer is valid readable storage.
  let before = unsafe { mbsinit(&raw const state) };

  assert_eq!(before, 0);

  set_errno(7474);
  // SAFETY: pointers are valid; `len == 0` should short-circuit before validating state/input.
  let converted = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(0),
      &raw mut state,
    )
  };

  assert_eq!(converted, sz(0));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 7474);

  // SAFETY: state pointer is valid readable storage.
  let after = unsafe { mbsinit(&raw const state) };

  assert_eq!(after, 0);
}

#[test]
fn wcsrtombs_null_dst_counts_without_advancing_src() {
  let input = [i32::from(b'A'), 0x1F363_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let original = src;

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe { wcsrtombs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(converted, sz(5));
  assert_eq!(src, original);
}

#[test]
fn wcsrtombs_null_dst_ignores_len_parameter() {
  let input = [i32::from(b'A'), 0x1F363_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let original = src;

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe { wcsrtombs(ptr::null_mut(), &raw mut src, sz(1), &raw mut state) };

  assert_eq!(converted, sz(5));
  assert_eq!(src, original);
}

#[test]
fn wcsrtombs_invalid_wchar_sets_eilseq_and_keeps_src() {
  let input = [0xD800_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 8];
  let original = src;

  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn wcsrtombs_corrupted_state_sets_eilseq_and_resets_for_next_call() {
  let input = [0xD800_i32, 0_i32];
  let continuation = b"A\0";
  let mut src = input.as_ptr();
  let original = src;
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 8];
  let mut continuation_src = continuation.as_ptr().cast::<c_char>();
  let mut continuation_dst = [0_i32; 2];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let first = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and `continuation` is NUL-terminated.
  let second = unsafe {
    mbsrtowcs(
      continuation_dst.as_mut_ptr(),
      &raw mut continuation_src,
      sz(continuation_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(continuation_dst[0], i32::from(b'A'));
  assert!(continuation_src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_corrupted_state_with_valid_wchar_sets_eilseq_and_resets_for_next_call() {
  let input = [i32::from(b'A'), 0_i32];
  let continuation = b"A\0";
  let mut src = input.as_ptr();
  let original = src;
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 8];
  let mut continuation_src = continuation.as_ptr().cast::<c_char>();
  let mut continuation_dst = [0_i32; 2];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let first = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and `continuation` is NUL-terminated.
  let second = unsafe {
    mbsrtowcs(
      continuation_dst.as_mut_ptr(),
      &raw mut continuation_src,
      sz(continuation_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(continuation_dst[0], i32::from(b'A'));
  assert!(continuation_src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_stale_zero_length_state_with_valid_wchar_sets_eilseq_and_resets_for_next_call() {
  let input = [i32::from(b'A'), 0_i32];
  let mut src = input.as_ptr();
  let original = src;
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 8];
  // bytes=[0xE3, 0, 0, 0], pending_len=0, expected_len=0 (stale bytes snapshot).
  let stale_state_bytes = [0xE3_u8, 0, 0, 0, 0, 0, 0, 0];

  write_state_bytes(&mut state, stale_state_bytes);
  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let first = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let second = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(&dst[..2], b"A\0");
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_reserved_state_with_valid_wchar_sets_eilseq_and_resets_for_next_call() {
  let input = [i32::from(b'A'), 0_i32];
  let mut src = input.as_ptr();
  let original = src;
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 8];
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 1, 0];

  write_state_bytes(&mut state, reserved);
  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let first = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let second = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(&dst[..2], b"A\0");
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_second_reserved_byte_state_with_valid_wchar_sets_eilseq_and_resets_for_next_call() {
  let input = [i32::from(b'A'), 0_i32];
  let mut src = input.as_ptr();
  let original = src;
  let mut state = mbstate_t::new();
  let mut dst = [0_u8; 8];
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 0, 1];

  write_state_bytes(&mut state, reserved);
  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let first = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let second = unsafe {
    wcsrtombs(
      dst.as_mut_ptr().cast::<c_char>(),
      &raw mut src,
      sz(dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(&dst[..2], b"A\0");
  assert!(src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_null_dst_corrupted_state_sets_eilseq_and_resets_for_next_call() {
  let input = [0xD800_i32, 0_i32];
  let continuation = b"A\0";
  let mut src = input.as_ptr();
  let original = src;
  let mut state = mbstate_t::new();
  let mut continuation_src = continuation.as_ptr().cast::<c_char>();
  let mut continuation_dst = [0_i32; 2];
  // bytes=[0xE3, 0, 0, 0], pending_len=1, expected_len=0 (impossible state).
  let corrupted = [0xE3_u8, 0, 0, 0, 1, 0, 0, 0];

  write_state_bytes(&mut state, corrupted);
  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let first = unsafe { wcsrtombs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and `continuation` is NUL-terminated.
  let second = unsafe {
    mbsrtowcs(
      continuation_dst.as_mut_ptr(),
      &raw mut continuation_src,
      sz(continuation_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(second, sz(1));
  assert_eq!(continuation_dst[0], i32::from(b'A'));
  assert!(continuation_src.is_null());
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_null_dst_reserved_state_sets_eilseq_and_resets_for_next_call() {
  let input = [i32::from(b'A'), 0_i32];
  let mut src = input.as_ptr();
  let original = src;
  let mut state = mbstate_t::new();
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[0]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 1, 0];

  write_state_bytes(&mut state, reserved);
  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let first = unsafe { wcsrtombs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let second = unsafe { wcsrtombs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(second, sz(1));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_null_dst_second_reserved_byte_state_sets_eilseq_and_resets_for_next_call() {
  let input = [i32::from(b'A'), 0_i32];
  let mut src = input.as_ptr();
  let original = src;
  let mut state = mbstate_t::new();
  // bytes=[0, 0, 0, 0], pending_len=0, expected_len=0, reserved[1]=1.
  let reserved = [0_u8, 0, 0, 0, 0, 0, 0, 1];

  write_state_bytes(&mut state, reserved);
  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let first = unsafe { wcsrtombs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(first, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);

  set_errno(0);
  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let second = unsafe { wcsrtombs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(second, sz(1));
  assert_eq!(src, original);
  assert_eq!(errno_value(), 0);
}

#[test]
fn wcsrtombs_invalid_wchar_resets_pending_state() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let input = [0xD800_i32, 0_i32];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut wide_src = input.as_ptr();
  let original_wide_src = wide_src;
  let mut multibyte_src = suffix.as_ptr().cast::<c_char>();
  let original_multibyte_src = multibyte_src;
  let mut wide_dst = [0_i32; 2];
  let mut narrow_dst = [0_u8; 8];

  // SAFETY: prefix bytes are readable and `state` is valid writable storage.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: pointers are valid and `state` is explicit restart storage.
  let converted = unsafe {
    wcsrtombs(
      narrow_dst.as_mut_ptr().cast::<c_char>(),
      &raw mut wide_src,
      sz(narrow_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide_src, original_wide_src);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let resumed = unsafe {
    mbsrtowcs(
      wide_dst.as_mut_ptr(),
      &raw mut multibyte_src,
      sz(wide_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(resumed, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(multibyte_src, original_multibyte_src);
}

#[test]
fn wcsrtombs_invalid_wchar_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let input = [0xD800_i32, 0_i32];
  let mut scratch_wide = -1_i32;
  let mut wide_src = input.as_ptr();
  let original_wide_src = wide_src;
  let mut multibyte_src = suffix.as_ptr().cast::<c_char>();
  let original_multibyte_src = multibyte_src;
  let mut wide_dst = [0_i32; 2];
  let mut narrow_dst = [0_u8; 8];

  // SAFETY: prefix bytes are readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: pointers are valid and null `ps` selects internal state.
  let converted = unsafe {
    wcsrtombs(
      narrow_dst.as_mut_ptr().cast::<c_char>(),
      &raw mut wide_src,
      sz(narrow_dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide_src, original_wide_src);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let resumed = unsafe {
    mbsrtowcs(
      wide_dst.as_mut_ptr(),
      &raw mut multibyte_src,
      sz(wide_dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(resumed, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(multibyte_src, original_multibyte_src);
}

#[test]
fn wcsrtombs_null_dst_invalid_wchar_sets_eilseq_and_keeps_src() {
  let input = [0xD800_i32, 0_i32];
  let mut src = input.as_ptr();
  let mut state = mbstate_t::new();
  let original = src;

  set_errno(0);

  // SAFETY: pointers are valid and `input` is NUL-terminated.
  let converted = unsafe { wcsrtombs(ptr::null_mut(), &raw mut src, sz(0), &raw mut state) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(src, original);
}

#[test]
fn wcsrtombs_null_dst_invalid_wchar_resets_pending_state() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let input = [0xD800_i32, 0_i32];
  let mut state = mbstate_t::new();
  let mut scratch_wide = -1_i32;
  let mut wide_src = input.as_ptr();
  let original_wide_src = wide_src;
  let mut multibyte_src = suffix.as_ptr().cast::<c_char>();
  let original_multibyte_src = multibyte_src;
  let mut wide_dst = [0_i32; 2];

  // SAFETY: prefix bytes are readable and `state` is valid writable storage.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      &raw mut state,
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: pointers are valid and `state` is explicit restart storage.
  let converted = unsafe { wcsrtombs(ptr::null_mut(), &raw mut wide_src, sz(0), &raw mut state) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide_src, original_wide_src);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let resumed = unsafe {
    mbsrtowcs(
      wide_dst.as_mut_ptr(),
      &raw mut multibyte_src,
      sz(wide_dst.len()),
      &raw mut state,
    )
  };

  assert_eq!(resumed, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(multibyte_src, original_multibyte_src);
}

#[test]
fn wcsrtombs_null_dst_invalid_wchar_resets_internal_state_when_ps_is_null() {
  let prefix = [0xE3_u8, 0x81];
  let suffix = [0x82_u8, 0_u8];
  let input = [0xD800_i32, 0_i32];
  let mut scratch_wide = -1_i32;
  let mut wide_src = input.as_ptr();
  let original_wide_src = wide_src;
  let mut multibyte_src = suffix.as_ptr().cast::<c_char>();
  let original_multibyte_src = multibyte_src;
  let mut wide_dst = [0_i32; 2];

  // SAFETY: prefix bytes are readable and null `ps` selects internal state.
  let partial = unsafe {
    mbrtowc(
      &raw mut scratch_wide,
      prefix.as_ptr().cast::<c_char>(),
      sz(prefix.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(partial, size_t::MAX - 1);

  set_errno(0);
  // SAFETY: pointers are valid and null `ps` selects the thread-local state path.
  let converted = unsafe { wcsrtombs(ptr::null_mut(), &raw mut wide_src, sz(0), ptr::null_mut()) };

  assert_eq!(converted, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(wide_src, original_wide_src);

  set_errno(0);
  // SAFETY: pointers are valid and `suffix` is NUL-terminated.
  let resumed = unsafe {
    mbsrtowcs(
      wide_dst.as_mut_ptr(),
      &raw mut multibyte_src,
      sz(wide_dst.len()),
      ptr::null_mut(),
    )
  };

  assert_eq!(resumed, size_t::MAX);
  assert_eq!(errno_value(), EILSEQ);
  assert_eq!(multibyte_src, original_multibyte_src);
}
