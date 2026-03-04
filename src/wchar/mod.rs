//! Restartable and compatibility multibyte conversion primitives.
//!
//! This module provides UTF-8-only implementations of:
//! - restartable APIs: `mbrtowc`, `mbrlen`, `mbsinit`
//! - compatibility wrappers: `mblen`, `mbtowc`, `wctomb`, `mbstowcs`, `wcstombs`
//!
//! The state model is intentionally small and uses `mbstate_t` to carry partial
//! UTF-8 sequences across calls.

use crate::abi::errno::EILSEQ;
use crate::abi::types::{c_char, c_int, size_t};
use crate::errno::set_errno;
use core::cell::UnsafeCell;
use core::ptr;

const MAX_UTF8_BYTES: usize = 4;
const MBR_ERR_INVALID: size_t = size_t::MAX;
const MBR_ERR_INCOMPLETE: size_t = size_t::MAX - 1;
const MB_ERR: c_int = -1;

/// C `wchar_t` type for Linux `x86_64`.
///
/// This target uses a signed 32-bit `wchar_t`.
pub type wchar_t = c_int;

/// UTF-8 decode outcome for one call to [`decode_utf8`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Utf8DecodeResult {
  /// A complete Unicode scalar value was decoded.
  ///
  /// `consumed` reports how many bytes from the current input slice were
  /// consumed in this call.
  Complete { code_point: u32, consumed: usize },
  /// More input is required to complete a scalar value.
  ///
  /// Any partial bytes are retained in [`mbstate_t`].
  Incomplete { consumed: usize },
  /// Invalid UTF-8 was encountered.
  ///
  /// `consumed` reports how many bytes from the current input slice were read
  /// before invalidity became certain. This may be less than the nominal UTF-8
  /// sequence width when a malformed prefix can be rejected early.
  ///
  /// On this result the state is reset to initial.
  Invalid { consumed: usize },
}

/// UTF-8 encode error for [`encode_utf8`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Utf8EncodeError {
  /// The input value is not a valid Unicode scalar value.
  InvalidScalarValue,
}

/// Restartable multibyte conversion state (`mbstate_t`).
///
/// Contract:
/// - initial state is all-zero (`mbstate_t::new()` or zeroed memory)
/// - non-initial state stores a partial UTF-8 sequence that was previously
///   reported as incomplete by `mbrtowc`/`mbrlen`
/// - callers must preserve this object across calls when using restartable
///   decoding
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct mbstate_t {
  bytes: [u8; MAX_UTF8_BYTES],
  pending_len: u8,
  expected_len: u8,
  reserved: [u8; 2],
}

impl mbstate_t {
  /// Returns an initial `mbstate_t` with no pending bytes.
  #[must_use]
  pub const fn new() -> Self {
    Self {
      bytes: [0; MAX_UTF8_BYTES],
      pending_len: 0,
      expected_len: 0,
      reserved: [0; 2],
    }
  }

  /// Returns whether the state currently has no pending bytes.
  ///
  /// This returns `true` only for the canonical initial representation
  /// (`pending_len == 0`, `expected_len == 0`, and all payload bytes zero).
  /// Snapshots with stale carry bytes or reserved metadata are treated as
  /// non-initial/corrupted.
  #[must_use]
  pub const fn is_initial(self) -> bool {
    self.pending_len == 0
      && self.expected_len == 0
      && bytes_are_zero(self.bytes)
      && short_bytes_are_zero(self.reserved)
  }

  /// Resets this state to the initial all-zero value.
  pub const fn reset(&mut self) {
    *self = Self::new();
  }

  fn pending_len(self) -> usize {
    usize::from(self.pending_len)
  }

  fn expected_len(self) -> usize {
    usize::from(self.expected_len)
  }

  fn set_partial(&mut self, bytes: [u8; MAX_UTF8_BYTES], pending_len: usize, expected_len: usize) {
    self.bytes = bytes;
    self.pending_len = u8::try_from(pending_len)
      .unwrap_or_else(|_| unreachable!("pending byte count must fit into u8"));
    self.expected_len = u8::try_from(expected_len)
      .unwrap_or_else(|_| unreachable!("expected byte count must fit into u8"));
    self.reserved = [0; 2];
  }
}

impl Default for mbstate_t {
  fn default() -> Self {
    Self::new()
  }
}

thread_local! {
  static INTERNAL_STATE: UnsafeCell<mbstate_t> = const { UnsafeCell::new(mbstate_t::new()) };
}

fn usize_from_size_t(value: size_t) -> usize {
  usize::try_from(value)
    .unwrap_or_else(|_| unreachable!("size_t must fit into usize on x86_64 Linux"))
}

fn size_t_from_usize(value: usize) -> size_t {
  size_t::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize must fit into size_t on x86_64 Linux"))
}

const fn bool_to_c_int(value: bool) -> c_int {
  if value { 1 } else { 0 }
}

const fn is_continuation_byte(byte: u8) -> bool {
  (byte & 0b1100_0000) == 0b1000_0000
}

const fn bytes_are_zero(bytes: [u8; MAX_UTF8_BYTES]) -> bool {
  let mut index = 0;

  while index < MAX_UTF8_BYTES {
    if bytes[index] != 0 {
      return false;
    }

    index += 1;
  }

  true
}

const fn short_bytes_are_zero(bytes: [u8; 2]) -> bool {
  bytes[0] == 0 && bytes[1] == 0
}

fn expected_len_from_lead_byte(byte: u8) -> Option<usize> {
  if byte < 0x80 {
    return Some(1);
  }

  if (0xC2..=0xDF).contains(&byte) {
    return Some(2);
  }

  if (0xE0..=0xEF).contains(&byte) {
    return Some(3);
  }

  if (0xF0..=0xF4).contains(&byte) {
    return Some(4);
  }

  None
}

fn decode_utf8_scalar(bytes: [u8; MAX_UTF8_BYTES], len: usize) -> Option<u32> {
  match len {
    1 => {
      let byte = bytes[0];

      if byte < 0x80 {
        Some(u32::from(byte))
      } else {
        None
      }
    }
    2 => {
      let b0 = bytes[0];
      let b1 = bytes[1];

      if !(0xC2..=0xDF).contains(&b0) || !is_continuation_byte(b1) {
        return None;
      }

      Some((u32::from(b0 & 0x1F) << 6) | u32::from(b1 & 0x3F))
    }
    3 => {
      let b0 = bytes[0];
      let b1 = bytes[1];
      let b2 = bytes[2];

      if !(0xE0..=0xEF).contains(&b0) || !is_continuation_byte(b1) || !is_continuation_byte(b2) {
        return None;
      }

      if (b0 == 0xE0 && b1 < 0xA0) || (b0 == 0xED && b1 > 0x9F) {
        return None;
      }

      Some((u32::from(b0 & 0x0F) << 12) | (u32::from(b1 & 0x3F) << 6) | u32::from(b2 & 0x3F))
    }
    4 => {
      let b0 = bytes[0];
      let b1 = bytes[1];
      let b2 = bytes[2];
      let b3 = bytes[3];

      if !(0xF0..=0xF4).contains(&b0)
        || !is_continuation_byte(b1)
        || !is_continuation_byte(b2)
        || !is_continuation_byte(b3)
      {
        return None;
      }

      if (b0 == 0xF0 && b1 < 0x90) || (b0 == 0xF4 && b1 > 0x8F) {
        return None;
      }

      let scalar = (u32::from(b0 & 0x07) << 18)
        | (u32::from(b1 & 0x3F) << 12)
        | (u32::from(b2 & 0x3F) << 6)
        | u32::from(b3 & 0x3F);

      if scalar > 0x10_FFFF {
        return None;
      }

      Some(scalar)
    }
    _ => None,
  }
}

fn partial_prefix_is_valid(
  bytes: [u8; MAX_UTF8_BYTES],
  pending_len: usize,
  expected_len: usize,
) -> bool {
  if pending_len == 0 || pending_len >= expected_len {
    return false;
  }

  if !(2..=MAX_UTF8_BYTES).contains(&expected_len) {
    return false;
  }

  let lead = bytes[0];

  if expected_len_from_lead_byte(lead) != Some(expected_len) {
    return false;
  }

  for continuation in bytes.iter().take(pending_len).skip(1) {
    if !is_continuation_byte(*continuation) {
      return false;
    }
  }

  // Bytes beyond `pending_len` must remain zero for states produced by this
  // implementation. Non-zero trailing bytes indicate a corrupted snapshot.
  for trailing in bytes.iter().skip(pending_len) {
    if *trailing != 0 {
      return false;
    }
  }

  if pending_len >= 2 {
    let second = bytes[1];

    if expected_len == 3 && ((lead == 0xE0 && second < 0xA0) || (lead == 0xED && second > 0x9F)) {
      return false;
    }

    if expected_len == 4 && ((lead == 0xF0 && second < 0x90) || (lead == 0xF4 && second > 0x8F)) {
      return false;
    }
  }

  true
}

fn encode_utf8_scalar(scalar: wchar_t) -> Option<([u8; MAX_UTF8_BYTES], usize)> {
  let scalar = u32::try_from(scalar).ok()?;

  if (0xD800..=0xDFFF).contains(&scalar) || scalar > 0x10_FFFF {
    return None;
  }

  let mut out = [0_u8; MAX_UTF8_BYTES];

  if scalar <= 0x7F {
    out[0] = u8::try_from(scalar).unwrap_or_else(|_| unreachable!("single-byte scalar"));

    return Some((out, 1));
  }

  if scalar <= 0x7FF {
    let top =
      u8::try_from((scalar >> 6) & 0x1F).unwrap_or_else(|_| unreachable!("two-byte top segment"));
    let low = u8::try_from(scalar & 0x3F).unwrap_or_else(|_| unreachable!("two-byte low segment"));

    out[0] = 0xC0 | top;
    out[1] = 0x80 | low;

    return Some((out, 2));
  }

  if scalar <= 0xFFFF {
    let top = u8::try_from((scalar >> 12) & 0x0F)
      .unwrap_or_else(|_| unreachable!("three-byte top segment"));
    let mid = u8::try_from((scalar >> 6) & 0x3F)
      .unwrap_or_else(|_| unreachable!("three-byte middle segment"));
    let low =
      u8::try_from(scalar & 0x3F).unwrap_or_else(|_| unreachable!("three-byte low segment"));

    out[0] = 0xE0 | top;
    out[1] = 0x80 | mid;
    out[2] = 0x80 | low;

    return Some((out, 3));
  }

  let top =
    u8::try_from((scalar >> 18) & 0x07).unwrap_or_else(|_| unreachable!("four-byte top segment"));
  let upper_mid = u8::try_from((scalar >> 12) & 0x3F)
    .unwrap_or_else(|_| unreachable!("four-byte upper-middle segment"));
  let lower_mid = u8::try_from((scalar >> 6) & 0x3F)
    .unwrap_or_else(|_| unreachable!("four-byte lower-middle segment"));
  let low = u8::try_from(scalar & 0x3F).unwrap_or_else(|_| unreachable!("four-byte low segment"));

  out[0] = 0xF0 | top;
  out[1] = 0x80 | upper_mid;
  out[2] = 0x80 | lower_mid;
  out[3] = 0x80 | low;

  Some((out, 4))
}

/// Decodes one UTF-8 scalar value using restartable state.
///
/// Input/output contract:
/// - consumes bytes from `input` plus any bytes already pending in `state`
/// - returns [`Utf8DecodeResult::Complete`] when one scalar is decoded
/// - returns [`Utf8DecodeResult::Incomplete`] when additional bytes are needed
/// - returns [`Utf8DecodeResult::Invalid`] for malformed/overlong/surrogate/
///   out-of-range sequences
/// - malformed prefixes are rejected as soon as they become impossible to
///   complete into a valid scalar
///
/// State contract:
/// - incomplete decode keeps pending bytes in `state`
/// - complete or invalid decode resets `state` to initial
#[must_use]
pub fn decode_utf8(state: &mut mbstate_t, input: &[u8]) -> Utf8DecodeResult {
  let mut bytes = state.bytes;
  let mut buffered_len = state.pending_len();
  let mut expected_len = state.expected_len();
  let mut consumed = 0usize;

  if !short_bytes_are_zero(state.reserved) {
    state.reset();

    return Utf8DecodeResult::Invalid { consumed };
  }

  if buffered_len == 0 {
    if expected_len != 0 || !bytes_are_zero(bytes) {
      state.reset();

      return Utf8DecodeResult::Invalid { consumed };
    }

    expected_len = 0;
  } else if !(2..=MAX_UTF8_BYTES).contains(&expected_len)
    || buffered_len >= expected_len
    || !partial_prefix_is_valid(bytes, buffered_len, expected_len)
  {
    state.reset();

    return Utf8DecodeResult::Invalid { consumed };
  }

  while buffered_len < MAX_UTF8_BYTES {
    if expected_len != 0 && buffered_len == expected_len {
      break;
    }

    if consumed == input.len() {
      break;
    }

    let byte = input[consumed];

    if buffered_len == 0 {
      let Some(next_expected_len) = expected_len_from_lead_byte(byte) else {
        state.reset();

        return Utf8DecodeResult::Invalid { consumed };
      };

      expected_len = next_expected_len;
    } else if !is_continuation_byte(byte) {
      state.reset();

      return Utf8DecodeResult::Invalid { consumed };
    }

    bytes[buffered_len] = byte;
    buffered_len += 1;
    consumed += 1;

    if buffered_len < expected_len && !partial_prefix_is_valid(bytes, buffered_len, expected_len) {
      state.reset();

      return Utf8DecodeResult::Invalid { consumed };
    }
  }

  if expected_len == 0 || buffered_len < expected_len {
    state.set_partial(bytes, buffered_len, expected_len);

    return Utf8DecodeResult::Incomplete { consumed };
  }

  let Some(code_point) = decode_utf8_scalar(bytes, expected_len) else {
    state.reset();

    return Utf8DecodeResult::Invalid { consumed };
  };

  state.reset();

  Utf8DecodeResult::Complete {
    code_point,
    consumed,
  }
}

/// Encodes one Unicode scalar value into UTF-8.
///
/// On success, writes canonical UTF-8 bytes to `output` and returns the number
/// of bytes written (`1..=4`).
///
/// # Errors
/// Returns [`Utf8EncodeError::InvalidScalarValue`] when `code_point` is not a
/// Unicode scalar value (surrogate range or above `U+10FFFF`).
pub fn encode_utf8(
  code_point: u32,
  output: &mut [u8; MAX_UTF8_BYTES],
) -> Result<usize, Utf8EncodeError> {
  let scalar = wchar_t::try_from(code_point).map_err(|_| Utf8EncodeError::InvalidScalarValue)?;
  let Some((encoded, encoded_len)) = encode_utf8_scalar(scalar) else {
    return Err(Utf8EncodeError::InvalidScalarValue);
  };

  output[..encoded_len].copy_from_slice(&encoded[..encoded_len]);

  Ok(encoded_len)
}

fn reset_state_and_fail(state: &mut mbstate_t) -> size_t {
  state.reset();
  set_errno(EILSEQ);
  MBR_ERR_INVALID
}

const fn c_char_to_u8(value: c_char) -> u8 {
  value.to_ne_bytes()[0]
}

const fn c_char_from_u8(value: u8) -> c_char {
  c_char::from_ne_bytes([value])
}

fn mbr_result_to_int(result: size_t) -> c_int {
  if result == MBR_ERR_INVALID || result == MBR_ERR_INCOMPLETE {
    if result == MBR_ERR_INCOMPLETE {
      set_errno(EILSEQ);
    }

    return MB_ERR;
  }

  c_int::try_from(result).unwrap_or_else(|_| unreachable!("mbrtowc byte count must fit into c_int"))
}

fn size_t_error_with_eilseq() -> size_t {
  set_errno(EILSEQ);

  size_t::MAX
}

fn reset_internal_multibyte_state() {
  // SAFETY: null `s` and null `ps` request reset of internal conversion state.
  unsafe {
    mbrtowc(ptr::null_mut(), ptr::null(), 0, ptr::null_mut());
  }
}

fn resolve_state_ptr(ps: *mut mbstate_t) -> *mut mbstate_t {
  if ps.is_null() {
    INTERNAL_STATE.with(UnsafeCell::get)
  } else {
    ps
  }
}

const unsafe fn read_input_byte(ptr: *const c_char, index: usize) -> u8 {
  // SAFETY: caller upholds that `ptr.add(index)` is readable.
  let byte = unsafe { ptr.add(index).read() };

  c_char_to_u8(byte)
}

const unsafe fn bytes_until_nul(ptr: *const c_char) -> usize {
  let mut length = 0usize;

  loop {
    // SAFETY: caller guarantees `ptr` points into a readable NUL-terminated C string.
    let byte = unsafe { read_input_byte(ptr, length) };

    if byte == 0 {
      return length;
    }

    length += 1;
  }
}

unsafe fn mbrtowc_with_state(
  pwc: *mut wchar_t,
  s: *const c_char,
  n: size_t,
  state_ptr: *mut mbstate_t,
) -> size_t {
  // SAFETY: caller and wrapper ensure `state_ptr` points to valid writable state.
  let state = unsafe { &mut *state_ptr };

  if s.is_null() {
    state.reset();

    if !pwc.is_null() {
      // SAFETY: caller provided writable `pwc` when non-null.
      unsafe {
        pwc.write(0);
      }
    }

    return 0;
  }

  let max_input = usize_from_size_t(n);
  let mut bytes = state.bytes;
  let mut buffered_len = state.pending_len();
  let mut expected_len = state.expected_len();
  let mut consumed = 0usize;

  if !short_bytes_are_zero(state.reserved) {
    return reset_state_and_fail(state);
  }

  if buffered_len == 0 {
    if expected_len != 0 || !bytes_are_zero(bytes) {
      return reset_state_and_fail(state);
    }

    expected_len = 0;
  } else if !(2..=MAX_UTF8_BYTES).contains(&expected_len)
    || buffered_len >= expected_len
    || !partial_prefix_is_valid(bytes, buffered_len, expected_len)
  {
    return reset_state_and_fail(state);
  }

  if max_input == 0 {
    return MBR_ERR_INCOMPLETE;
  }

  while buffered_len < MAX_UTF8_BYTES {
    if expected_len != 0 && buffered_len == expected_len {
      break;
    }

    if consumed == max_input {
      break;
    }

    // SAFETY: caller guarantees at least `n` readable bytes from `s`.
    let byte = unsafe { read_input_byte(s, consumed) };

    if buffered_len == 0 {
      let Some(next_expected_len) = expected_len_from_lead_byte(byte) else {
        return reset_state_and_fail(state);
      };

      expected_len = next_expected_len;
    } else if !is_continuation_byte(byte) {
      return reset_state_and_fail(state);
    }

    bytes[buffered_len] = byte;
    buffered_len += 1;
    consumed += 1;

    if buffered_len < expected_len && !partial_prefix_is_valid(bytes, buffered_len, expected_len) {
      return reset_state_and_fail(state);
    }
  }

  if expected_len == 0 || buffered_len < expected_len {
    state.set_partial(bytes, buffered_len, expected_len);

    return MBR_ERR_INCOMPLETE;
  }

  let Some(scalar) = decode_utf8_scalar(bytes, expected_len) else {
    return reset_state_and_fail(state);
  };

  state.reset();

  if !pwc.is_null() {
    let wide = wchar_t::try_from(scalar)
      .unwrap_or_else(|_| unreachable!("decoded scalar must fit wchar_t on this target"));

    // SAFETY: caller provided writable `pwc` when non-null.
    unsafe {
      pwc.write(wide);
    }
  }

  if scalar == 0 {
    return 0;
  }

  size_t_from_usize(consumed)
}

/// C ABI entry point for `mbrtowc`.
///
/// Decodes at most `n` bytes from `s` using `ps` restart state and optionally
/// stores the resulting wide character in `pwc`.
///
/// Return value contract:
/// - `0` on decoded NUL (`U+0000`)
/// - `1..=n` on successful non-NUL decode
/// - `(size_t)-2` on incomplete but potentially valid sequence
/// - `(size_t)-1` on invalid sequence (and sets `errno = EILSEQ`)
/// - `(size_t)-1` when `ps` holds an impossible/corrupted state snapshot
///   (even if `n == 0`)
///
/// State contract:
/// - `ps == NULL` uses an internal thread-local state object
/// - `s == NULL` resets the chosen state to initial and returns `0`
///
/// # Safety
/// - when `s` is non-null, caller must provide at least `n` readable bytes
/// - when `pwc` is non-null and conversion succeeds, `pwc` must be writable
/// - when `ps` is non-null, it must point to writable `mbstate_t` storage
///
/// # Errors
/// - sets `errno = EILSEQ` and returns `(size_t)-1` for invalid UTF-8
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbrtowc(
  pwc: *mut wchar_t,
  s: *const c_char,
  n: size_t,
  ps: *mut mbstate_t,
) -> size_t {
  if ps.is_null() {
    return INTERNAL_STATE.with(|state| {
      // SAFETY: thread-local state pointer is valid for the current thread.
      unsafe { mbrtowc_with_state(pwc, s, n, state.get()) }
    });
  }

  // SAFETY: caller provided a non-null state pointer.
  unsafe { mbrtowc_with_state(pwc, s, n, ps) }
}

/// C ABI entry point for `mbrlen`.
///
/// This function is equivalent to calling `mbrtowc(NULL, s, n, ps)`.
///
/// # Safety
/// - matches [`mbrtowc`] safety requirements for `s`, `n`, and `ps`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbrlen(s: *const c_char, n: size_t, ps: *mut mbstate_t) -> size_t {
  // SAFETY: forwarding preserves `mbrtowc` contract exactly.
  unsafe { mbrtowc(ptr::null_mut(), s, n, ps) }
}

/// C ABI entry point for `mbsinit`.
///
/// Returns non-zero when `ps` is in the initial conversion state.
/// A null `ps` is defined as initial state.
///
/// # Safety
/// - when `ps` is non-null, it must point to readable `mbstate_t` storage
#[must_use]
#[unsafe(no_mangle)]
pub const unsafe extern "C" fn mbsinit(ps: *const mbstate_t) -> c_int {
  if ps.is_null() {
    return 1;
  }

  // SAFETY: caller guarantees `ps` points to readable state.
  let state = unsafe { ps.read() };

  bool_to_c_int(state.is_initial())
}

/// C ABI entry point for `wcrtomb`.
///
/// Converts a single wide character `wc` into UTF-8 bytes written to `s`.
///
/// Return value contract:
/// - `1..=4` on successful encoding;
/// - `(size_t)-1` on invalid `wc` and sets `errno = EILSEQ`.
///
/// State contract:
/// - `ps == NULL` uses an internal thread-local state object;
/// - `s == NULL` resets the selected state, ignores `wc`, and returns `1`
///   (UTF-8 length for NUL).
/// - `s != NULL` rejects non-initial/invalid `mbstate_t` snapshots with
///   `(size_t)-1`, sets `errno = EILSEQ`, and resets state before returning.
///
/// # Safety
/// - when `s` is non-null, caller must provide writable storage for at least
///   4 bytes.
/// - when `ps` is non-null, it must point to writable `mbstate_t` storage.
///
/// # Errors
/// - sets `errno = EILSEQ` when `wc` is not a valid Unicode scalar value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcrtomb(s: *mut c_char, wc: wchar_t, ps: *mut mbstate_t) -> size_t {
  let state_ptr = resolve_state_ptr(ps);
  // SAFETY: `resolve_state_ptr` always returns writable state storage.
  let state = unsafe { &mut *state_ptr };

  if s.is_null() {
    state.reset();

    return size_t_from_usize(1);
  }

  if !state.is_initial() {
    return reset_state_and_fail(state);
  }

  let Some((encoded, encoded_len)) = encode_utf8_scalar(wc) else {
    return reset_state_and_fail(state);
  };

  // SAFETY: caller guarantees `s` is writable for at least 4 bytes.
  unsafe {
    ptr::copy_nonoverlapping(encoded.as_ptr(), s.cast::<u8>(), encoded_len);
  }
  state.reset();

  size_t_from_usize(encoded_len)
}

/// C ABI entry point for `mbsrtowcs`.
///
/// Converts a multibyte UTF-8 string referenced by `*src` into wide characters.
///
/// Return value contract:
/// - number of converted non-NUL wide characters on success;
/// - `(size_t)-1` on invalid sequence and sets `errno = EILSEQ`.
///
/// Contract notes:
/// - on complete conversion with non-null `dst`, writes trailing wide NUL and
///   stores `NULL` into `*src`;
/// - when output capacity is exhausted, stores next unread byte pointer into
///   `*src`;
/// - when `dst == NULL`, counts required output length and leaves `*src`
///   unchanged.
/// - when `ps` carries pending bytes from a prior partial decode, conversion
///   resumes from that state before consuming further bytes from `*src`.
///
/// # Safety
/// - `src` must be non-null and point to readable pointer storage.
/// - when `*src` is non-null, it must point to a readable NUL-terminated UTF-8
///   string.
/// - when `dst` is non-null, it must be writable for at least `len` elements.
/// - when `ps` is non-null, it must point to writable `mbstate_t` storage.
///
/// # Errors
/// - sets `errno = EILSEQ` and returns `(size_t)-1` on invalid UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbsrtowcs(
  dst: *mut wchar_t,
  src: *mut *const c_char,
  len: size_t,
  ps: *mut mbstate_t,
) -> size_t {
  if src.is_null() {
    return size_t_error_with_eilseq();
  }

  let state_ptr = resolve_state_ptr(ps);
  // SAFETY: caller provides readable pointer storage at `src`.
  let mut input_cursor = unsafe { src.read() };

  if input_cursor.is_null() {
    // SAFETY: `resolve_state_ptr` always returns writable state storage.
    unsafe {
      (*state_ptr).reset();
    }

    return size_t_from_usize(0);
  }

  if dst.is_null() {
    let mut converted = 0usize;

    loop {
      let mut wide = 0 as wchar_t;
      // SAFETY: caller guarantees readable NUL-terminated bytes at `input_cursor`.
      let available = unsafe { bytes_until_nul(input_cursor) + 1 };
      let n = size_t_from_usize(available);
      let wide_ptr: *mut wchar_t = &raw mut wide;
      // SAFETY: `input_cursor` has at least `n` readable bytes and `state_ptr` is valid.
      let result = unsafe { mbrtowc_with_state(wide_ptr, input_cursor, n, state_ptr) };

      if result == MBR_ERR_INVALID {
        return MBR_ERR_INVALID;
      }

      if result == MBR_ERR_INCOMPLETE {
        // SAFETY: `state_ptr` is valid writable storage.
        return unsafe { reset_state_and_fail(&mut *state_ptr) };
      }

      if result == 0 {
        return size_t_from_usize(converted);
      }

      converted += 1;

      let consumed = usize_from_size_t(result);
      // SAFETY: `result` bytes were successfully consumed from `input_cursor`.
      input_cursor = unsafe { input_cursor.add(consumed) };
    }
  }

  let capacity = usize_from_size_t(len);

  if capacity == 0 {
    return size_t_from_usize(0);
  }

  let mut output_index = 0usize;

  while output_index < capacity {
    let mut wide = 0 as wchar_t;
    // SAFETY: caller guarantees readable NUL-terminated bytes at `input_cursor`.
    let available = unsafe { bytes_until_nul(input_cursor) + 1 };
    let n = size_t_from_usize(available);
    let wide_ptr: *mut wchar_t = &raw mut wide;
    // SAFETY: `input_cursor` has at least `n` readable bytes and `state_ptr` is valid.
    let result = unsafe { mbrtowc_with_state(wide_ptr, input_cursor, n, state_ptr) };

    if result == MBR_ERR_INVALID {
      // SAFETY: caller provides writable pointer storage at `src`.
      unsafe {
        src.write(input_cursor);
      }

      return MBR_ERR_INVALID;
    }

    if result == MBR_ERR_INCOMPLETE {
      // SAFETY: caller provides writable pointer storage at `src`.
      unsafe {
        src.write(input_cursor);
      }

      // SAFETY: `state_ptr` is valid writable storage.
      return unsafe { reset_state_and_fail(&mut *state_ptr) };
    }

    if result == 0 {
      // SAFETY: `output_index < capacity` guarantees destination capacity.
      unsafe {
        dst.add(output_index).write(0);
        src.write(ptr::null());
      }

      return size_t_from_usize(output_index);
    }

    // SAFETY: `output_index < capacity` guarantees destination capacity.
    unsafe {
      dst.add(output_index).write(wide);
    }

    let consumed = usize_from_size_t(result);
    // SAFETY: `result` bytes were successfully consumed from `input_cursor`.
    input_cursor = unsafe { input_cursor.add(consumed) };
    output_index += 1;
  }

  // SAFETY: caller provides writable pointer storage at `src`.
  unsafe {
    src.write(input_cursor);
  }

  size_t_from_usize(output_index)
}

/// C ABI entry point for `wcsrtombs`.
///
/// Converts a wide-character string referenced by `*src` into UTF-8 bytes.
///
/// Return value contract:
/// - number of converted non-NUL bytes on success;
/// - `(size_t)-1` on invalid `wchar_t` scalar and sets `errno = EILSEQ`.
///
/// Contract notes:
/// - on complete conversion with non-null `dst`, writes trailing NUL and
///   stores `NULL` into `*src`;
/// - when output byte capacity is exhausted, stores pointer to next unread
///   wide character into `*src`;
/// - when `dst == NULL`, counts required output byte length and leaves `*src`
///   unchanged.
/// - when `dst != NULL` and `len == 0`, returns `0` without validating
///   conversion state or wide input, and leaves `*src` unchanged.
/// - non-initial or otherwise invalid `mbstate_t` snapshots are rejected with
///   `(size_t)-1`, `errno = EILSEQ`, and reset state before returning.
///
/// # Safety
/// - `src` must be non-null and point to readable pointer storage.
/// - when `*src` is non-null, it must point to a readable wide string
///   terminated by wide NUL.
/// - when `dst` is non-null, it must be writable for at least `len` bytes.
/// - when `ps` is non-null, it must point to writable `mbstate_t` storage.
///
/// # Errors
/// - sets `errno = EILSEQ` and returns `(size_t)-1` on invalid wide input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsrtombs(
  dst: *mut c_char,
  src: *mut *const wchar_t,
  len: size_t,
  ps: *mut mbstate_t,
) -> size_t {
  if src.is_null() {
    return size_t_error_with_eilseq();
  }

  let state_ptr = resolve_state_ptr(ps);
  // SAFETY: `resolve_state_ptr` always returns writable state storage.
  let state = unsafe { &mut *state_ptr };
  // SAFETY: caller provides readable pointer storage at `src`.
  let mut input_cursor = unsafe { src.read() };

  if input_cursor.is_null() {
    state.reset();

    return size_t_from_usize(0);
  }

  if !dst.is_null() && len == size_t_from_usize(0) {
    return size_t_from_usize(0);
  }

  if !state.is_initial() {
    return reset_state_and_fail(state);
  }

  if dst.is_null() {
    let mut byte_count = 0usize;

    loop {
      // SAFETY: caller guarantees readable wide input at `input_cursor`.
      let wide = unsafe { input_cursor.read() };

      if wide == 0 {
        state.reset();

        return size_t_from_usize(byte_count);
      }

      let Some((_, encoded_len)) = encode_utf8_scalar(wide) else {
        return reset_state_and_fail(state);
      };

      byte_count += encoded_len;
      // SAFETY: advancing by one `wchar_t` within readable wide input.
      input_cursor = unsafe { input_cursor.add(1) };
    }
  }

  let capacity = usize_from_size_t(len);

  if capacity == 0 {
    return size_t_from_usize(0);
  }

  let mut byte_count = 0usize;

  loop {
    // SAFETY: caller guarantees readable wide input at `input_cursor`.
    let wide = unsafe { input_cursor.read() };

    if wide == 0 {
      if byte_count < capacity {
        // SAFETY: at least one output slot remains for trailing NUL.
        unsafe {
          dst.add(byte_count).write(c_char_from_u8(0));
          src.write(ptr::null());
        }
        state.reset();

        return size_t_from_usize(byte_count);
      }

      // SAFETY: caller provides writable pointer storage at `src`.
      unsafe {
        src.write(input_cursor);
      }

      return size_t_from_usize(byte_count);
    }

    let Some((encoded, encoded_len)) = encode_utf8_scalar(wide) else {
      // SAFETY: caller provides writable pointer storage at `src`.
      unsafe {
        src.write(input_cursor);
      }

      return reset_state_and_fail(state);
    };

    if byte_count + encoded_len > capacity {
      // SAFETY: caller provides writable pointer storage at `src`.
      unsafe {
        src.write(input_cursor);
      }

      return size_t_from_usize(byte_count);
    }

    // SAFETY: capacity check above ensures destination range is writable.
    unsafe {
      ptr::copy_nonoverlapping(
        encoded.as_ptr(),
        dst.add(byte_count).cast::<u8>(),
        encoded_len,
      );
      input_cursor = input_cursor.add(1);
    }
    byte_count += encoded_len;
  }
}

/// C ABI entry point for `mblen`.
///
/// Determines the byte length of the next multibyte character in `s`, using an
/// internal conversion state object.
///
/// Return value contract:
/// - `0` when `s == NULL` (state reset) or when `*s == '\0'`
/// - `1..` for a successfully decoded non-NUL character
/// - `-1` on invalid or incomplete UTF-8 sequence (`errno = EILSEQ`)
///
/// # Safety
/// - when `s` is non-null, `s` must point to at least `n` readable bytes
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblen(s: *const c_char, n: size_t) -> c_int {
  if s.is_null() {
    reset_internal_multibyte_state();

    return 0;
  }

  // SAFETY: forwarding preserves `mbrlen` safety contract.
  let result = unsafe { mbrlen(s, n, ptr::null_mut()) };

  mbr_result_to_int(result)
}

/// C ABI entry point for `mbtowc`.
///
/// Converts the next multibyte character in `s` to a wide character and
/// stores it to `pwc` when non-null, using an internal conversion state.
///
/// Return value contract:
/// - `0` when `s == NULL` (state reset) or when decoding `'\0'`
/// - `1..` for a successfully decoded non-NUL character
/// - `-1` on invalid or incomplete UTF-8 sequence (`errno = EILSEQ`)
///
/// # Safety
/// - when `s` is non-null, `s` must point to at least `n` readable bytes
/// - when `pwc` is non-null and conversion succeeds, it must be writable
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbtowc(pwc: *mut wchar_t, s: *const c_char, n: size_t) -> c_int {
  if s.is_null() {
    reset_internal_multibyte_state();

    return 0;
  }

  // SAFETY: forwarding preserves `mbrtowc` safety contract.
  let result = unsafe { mbrtowc(pwc, s, n, ptr::null_mut()) };

  mbr_result_to_int(result)
}

/// C ABI entry point for `wctomb`.
///
/// Encodes the wide character `wc` into UTF-8 and writes it to `s`.
///
/// Return value contract:
/// - `0` when `s == NULL` (state reset request)
/// - `1..=4` for successful UTF-8 encoding
/// - `-1` on invalid `wc` (`errno = EILSEQ`)
///
/// # Safety
/// - when `s` is non-null, caller must provide writable space for at least
///   4 bytes
///
/// # Errors
/// - sets `errno = EILSEQ` when `wc` is not a valid Unicode scalar value
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wctomb(s: *mut c_char, wc: wchar_t) -> c_int {
  if s.is_null() {
    reset_internal_multibyte_state();

    return 0;
  }

  let Some((encoded, encoded_len)) = encode_utf8_scalar(wc) else {
    set_errno(EILSEQ);

    return MB_ERR;
  };

  for (index, byte) in encoded.iter().take(encoded_len).enumerate() {
    // SAFETY: caller guarantees `s` has enough writable storage.
    unsafe {
      s.add(index).write(c_char_from_u8(*byte));
    }
  }

  c_int::try_from(encoded_len)
    .unwrap_or_else(|_| unreachable!("UTF-8 byte count must fit into c_int"))
}

/// C ABI entry point for `mbstowcs`.
///
/// Converts a multibyte C string `src` into wide characters in `dst`, up to
/// `len` output elements.
///
/// Return value contract:
/// - number of wide characters converted (excluding terminating NUL)
/// - `(size_t)-1` on conversion error (`errno = EILSEQ`)
///
/// Contract notes:
/// - conversion starts in initial shift state on each call
/// - when `dst` is null, the function returns the required output length
///   without writing output
/// - when `dst` is non-null and `len == 0`, the function returns `0` without
///   validating or consuming `src`
/// - when `dst` is non-null and the converted count reaches `len`, the
///   function returns immediately without validating additional source bytes
///
/// # Safety
/// - `src` must point to a readable NUL-terminated multibyte string
/// - when `dst` is non-null, `dst` must be writable for at least `len`
///   `wchar_t` elements
///
/// # Errors
/// - sets `errno = EILSEQ` and returns `(size_t)-1` on invalid input
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbstowcs(dst: *mut wchar_t, src: *const c_char, len: size_t) -> size_t {
  if src.is_null() {
    return size_t_error_with_eilseq();
  }

  let limit = usize_from_size_t(len);

  if !dst.is_null() && limit == 0 {
    return size_t_from_usize(0);
  }

  let mut cursor = src;
  let mut count = 0usize;
  let mut state = mbstate_t::new();

  loop {
    if !dst.is_null() && count == limit {
      return size_t_from_usize(count);
    }

    let mut wide = 0 as wchar_t;
    // SAFETY: `src` must be NUL-terminated by contract; adding one includes terminator.
    let available = unsafe { bytes_until_nul(cursor) + 1 };
    let bytes_to_try = size_t_from_usize(available);
    let wide_ptr: *mut wchar_t = &raw mut wide;
    let state_ptr: *mut mbstate_t = &raw mut state;
    // SAFETY: `cursor` points into a readable NUL-terminated string.
    let result = unsafe { mbrtowc(wide_ptr, cursor, bytes_to_try, state_ptr) };

    if result == MBR_ERR_INVALID || result == MBR_ERR_INCOMPLETE {
      return size_t_error_with_eilseq();
    }

    if result == 0 {
      if !dst.is_null() && count < limit {
        // SAFETY: `count < limit` ensures destination slot is in-bounds.
        unsafe {
          dst.add(count).write(0);
        }
      }

      return size_t_from_usize(count);
    }

    if !dst.is_null() {
      // SAFETY: `count < limit` ensures destination slot is in-bounds.
      unsafe {
        dst.add(count).write(wide);
      }
    }

    let consumed = usize_from_size_t(result);
    // SAFETY: `result` bytes were successfully consumed from `cursor`.
    cursor = unsafe { cursor.add(consumed) };
    count += 1;
  }
}

/// C ABI entry point for `wcstombs`.
///
/// Converts a wide-character string `src` into UTF-8 bytes in `dst`, up to
/// `len` output bytes.
///
/// Return value contract:
/// - number of output bytes produced (excluding terminating NUL)
/// - `(size_t)-1` on conversion error (`errno = EILSEQ`)
///
/// Contract notes:
/// - conversion starts in initial shift state on each call
/// - when `dst` is null, the function returns the required UTF-8 byte length
///   without writing output
/// - when `dst` is non-null and `len == 0`, the function returns `0` without
///   validating or consuming `src`
/// - when `dst` is non-null and produced bytes reach `len`, the function
///   returns immediately without validating additional wide input
///
/// # Safety
/// - `src` must point to a readable wide-character string terminated by wide NUL
/// - when `dst` is non-null, `dst` must be writable for at least `len` bytes
///
/// # Errors
/// - sets `errno = EILSEQ` and returns `(size_t)-1` on invalid `wchar_t` input
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstombs(dst: *mut c_char, src: *const wchar_t, len: size_t) -> size_t {
  if src.is_null() {
    return size_t_error_with_eilseq();
  }

  let limit = usize_from_size_t(len);

  if !dst.is_null() && limit == 0 {
    return size_t_from_usize(0);
  }

  let mut byte_count = 0usize;
  let mut index = 0usize;

  loop {
    if !dst.is_null() && byte_count == limit {
      return size_t_from_usize(byte_count);
    }

    // SAFETY: caller guarantees `src` points to a readable wide string.
    let wide = unsafe { src.add(index).read() };

    if wide == 0 {
      if !dst.is_null() && byte_count < limit {
        // SAFETY: `byte_count < limit` ensures write is in-bounds.
        unsafe {
          dst.add(byte_count).write(c_char_from_u8(0));
        }
      }

      return size_t_from_usize(byte_count);
    }

    let Some((encoded, encoded_len)) = encode_utf8_scalar(wide) else {
      return size_t_error_with_eilseq();
    };

    if !dst.is_null() {
      if byte_count + encoded_len > limit {
        return size_t_from_usize(byte_count);
      }

      for (byte_index, byte) in encoded.iter().take(encoded_len).enumerate() {
        // SAFETY: bounds checked by `byte_count + encoded_len <= limit`.
        unsafe {
          dst
            .add(byte_count + byte_index)
            .write(c_char_from_u8(*byte));
        }
      }
    }

    byte_count += encoded_len;
    index += 1;
  }
}
