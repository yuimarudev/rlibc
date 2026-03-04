//! ASCII `ctype` primitives for the `C` locale.
//!
//! The exported functions accept `int` values following C rules:
//! - valid domain is `EOF` or `unsigned char` range (`0..=255`)
//! - out-of-range values return `0` for predicates
//! - out-of-range values are returned unchanged by `tolower` / `toupper`

use crate::abi::types::c_int;

const ASCII_7BIT_MAX: c_int = 0x7F;

fn as_ascii_byte(c: c_int) -> Option<u8> {
  u8::try_from(c).ok()
}

fn classify_ascii(c: c_int, predicate: impl FnOnce(u8) -> bool) -> c_int {
  as_ascii_byte(c).map_or(0, |byte| c_int::from(u8::from(predicate(byte))))
}

fn map_ascii(c: c_int, map: impl FnOnce(u8) -> u8) -> c_int {
  as_ascii_byte(c).map_or(c, |byte| c_int::from(map(byte)))
}

const fn is_ascii7bit_int(c: c_int) -> bool {
  c >= 0 && c <= ASCII_7BIT_MAX
}

const fn project_ascii7bit(c: c_int) -> c_int {
  c & ASCII_7BIT_MAX
}

const fn is_in_ascii_range(byte: u8, start: u8, end: u8) -> bool {
  byte >= start && byte <= end
}

const fn is_c_locale_space(byte: u8) -> bool {
  matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

const fn is_c_locale_blank(byte: u8) -> bool {
  matches!(byte, b' ' | b'\t')
}

const fn is_c_locale_print(byte: u8) -> bool {
  is_in_ascii_range(byte, 0x20, 0x7E)
}

const fn is_c_locale_graph(byte: u8) -> bool {
  is_in_ascii_range(byte, 0x21, 0x7E)
}

const fn is_c_locale_cntrl(byte: u8) -> bool {
  byte <= 0x1F || byte == 0x7F
}

const fn is_c_locale_digit(byte: u8) -> bool {
  is_in_ascii_range(byte, b'0', b'9')
}

const fn is_c_locale_lower(byte: u8) -> bool {
  is_in_ascii_range(byte, b'a', b'z')
}

const fn is_c_locale_upper(byte: u8) -> bool {
  is_in_ascii_range(byte, b'A', b'Z')
}

const fn is_c_locale_alpha(byte: u8) -> bool {
  is_c_locale_lower(byte) || is_c_locale_upper(byte)
}

const fn is_c_locale_alnum(byte: u8) -> bool {
  is_c_locale_alpha(byte) || is_c_locale_digit(byte)
}

const fn is_c_locale_hex_upper(byte: u8) -> bool {
  is_in_ascii_range(byte, b'A', b'F')
}

const fn is_c_locale_hex_lower(byte: u8) -> bool {
  is_in_ascii_range(byte, b'a', b'f')
}

const fn is_c_locale_xdigit(byte: u8) -> bool {
  is_c_locale_digit(byte) || is_c_locale_hex_upper(byte) || is_c_locale_hex_lower(byte)
}

const fn is_c_locale_punct(byte: u8) -> bool {
  matches!(
    byte,
    b'!'..=b'/' | b':'..=b'@' | b'['..=b'`' | b'{'..=b'~'
  )
}

const fn to_c_locale_lower(byte: u8) -> u8 {
  byte.to_ascii_lowercase()
}

const fn to_c_locale_upper(byte: u8) -> u8 {
  byte.to_ascii_uppercase()
}

/// Returns non-zero when `c` is an ASCII alphabetic character.
#[unsafe(no_mangle)]
pub extern "C" fn isalpha(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_alpha)
}

/// Returns non-zero when `c` is an ASCII decimal digit.
#[unsafe(no_mangle)]
pub extern "C" fn isdigit(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_digit)
}

/// Returns non-zero when `c` is an ASCII alphanumeric character.
#[unsafe(no_mangle)]
pub extern "C" fn isalnum(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_alnum)
}

/// Returns non-zero when `c` is ASCII lowercase.
#[unsafe(no_mangle)]
pub extern "C" fn islower(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_lower)
}

/// Returns non-zero when `c` is ASCII uppercase.
#[unsafe(no_mangle)]
pub extern "C" fn isupper(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_upper)
}

/// Returns non-zero when `c` is ASCII hexadecimal digit.
#[unsafe(no_mangle)]
pub extern "C" fn isxdigit(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_xdigit)
}

/// Returns non-zero when `c` is `' '` or `'\t'`.
#[unsafe(no_mangle)]
pub extern "C" fn isblank(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_blank)
}

/// Returns non-zero when `c` is one of the C locale whitespace bytes:
/// `' '`, `'\t'`, `'\n'`, `'\v'`, `'\f'`, or `'\r'`.
#[unsafe(no_mangle)]
pub extern "C" fn isspace(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_space)
}

/// Returns non-zero when `c` is ASCII control character.
#[unsafe(no_mangle)]
pub extern "C" fn iscntrl(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_cntrl)
}

/// Returns non-zero when `c` is ASCII printable character.
#[unsafe(no_mangle)]
pub extern "C" fn isprint(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_print)
}

/// Returns non-zero when `c` is ASCII graphical character (printable except space).
#[unsafe(no_mangle)]
pub extern "C" fn isgraph(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_graph)
}

/// Returns non-zero when `c` is ASCII punctuation.
#[unsafe(no_mangle)]
pub extern "C" fn ispunct(c: c_int) -> c_int {
  classify_ascii(c, is_c_locale_punct)
}

/// Returns non-zero when `c` is representable as a 7-bit ASCII byte (`0..=127`).
///
/// This mirrors the traditional libc extension behavior and is locale-independent.
/// Output contract: returns `1` when `c` is in `0..=127`, otherwise `0`.
#[must_use]
pub const extern "C" fn isascii(c: c_int) -> c_int {
  if is_ascii7bit_int(c) { 1 } else { 0 }
}

/// Projects `c` into the 7-bit ASCII domain by clearing high-order bits.
///
/// The return value is always in `0..=127`, including for negative inputs.
#[must_use]
pub const extern "C" fn toascii(c: c_int) -> c_int {
  project_ascii7bit(c)
}

/// Converts ASCII uppercase letters to lowercase.
/// Values outside the C `unsigned char` domain (and `EOF`) are returned unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn tolower(c: c_int) -> c_int {
  map_ascii(c, to_c_locale_lower)
}

/// Converts ASCII lowercase letters to uppercase.
/// Values outside the C `unsigned char` domain (and `EOF`) are returned unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn toupper(c: c_int) -> c_int {
  map_ascii(c, to_c_locale_upper)
}
