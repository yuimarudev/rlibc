//! C stdio buffering and formatting interfaces.
//!
//! This module currently provides:
//! - a minimal `FILE` registry used to track stream usage for `setvbuf`
//! - `fflush` for registered streams and `fflush(NULL)`
//! - a minimal `setvbuf` entry point with mode/size validation and per-stream
//!   buffering configuration tracking
//! - an incremental `vsnprintf` subset (`%%`, `%s`, `%c`, `%p`, `%d/%i/%u/%x/%X/%o`)
//! - C ABI wrappers for `printf`/`fprintf`/`vprintf`/`vfprintf`

use crate::abi::errno::EINVAL;
use crate::abi::types::{c_int, size_t};
use crate::errno::{__errno_location, set_errno};
use core::ffi::{c_char, c_void};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Error as IoError;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// C stdio `EOF` status code.
pub const EOF: c_int = -1;
/// Fully buffered mode (`setvbuf`).
pub const _IOFBF: c_int = 0;
/// Line buffered mode (`setvbuf`).
pub const _IOLBF: c_int = 1;
/// Unbuffered mode (`setvbuf`).
pub const _IONBF: c_int = 2;
const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
const VFPRINTF_SYMBOL_NAME: &[u8] = b"vfprintf\0";
const ERRNO_LOCATION_SYMBOL_NAME: &[u8] = b"__errno_location\0";

/// Opaque C `FILE` handle type used by stdio entry points.
#[repr(C)]
pub struct File {
  _private: [u8; 0],
}

/// Public C ABI type alias for stdio stream handles.
pub type FILE = File;

struct StreamState {
  stream_key: usize,
  buffering_mode: c_int,
  buffer_size: usize,
  user_buffer_addr: usize,
  explicit_buffering_config: bool,
  io_active: bool,
  host_backed_io: bool,
  host_stream_identity: Option<u64>,
}

struct OutputSink {
  buffer: *mut c_char,
  capacity: usize,
  bytes_written: usize,
  bytes_required: usize,
}

#[repr(C)]
struct SysVVaList {
  gp_offset: u32,
  fp_offset: u32,
  overflow_arg_area: *mut c_void,
  reg_save_area: *mut c_void,
}

#[derive(Clone, Copy)]
enum CountSpec {
  Literal(usize),
  FromArgs,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LengthModifier {
  Default,
  Hh,
  H,
  L,
  Ll,
  J,
  T,
  Z,
}

#[derive(Clone, Copy)]
struct FormatDirective {
  flags: u8,
  width: Option<CountSpec>,
  precision: Option<CountSpec>,
  length: LengthModifier,
  conversion: u8,
  next_index: usize,
}

struct VarArgCursor {
  gp_offset: u32,
  reg_save_area: *const u8,
  overflow_arg_area: *const u64,
}

type HostVfprintfFn =
  unsafe extern "C" fn(stream: *mut FILE, format: *const c_char, ap: *mut c_void) -> c_int;

type HostErrnoLocationFn = unsafe extern "C" fn() -> *mut c_int;

impl FormatDirective {
  const ALTERNATE_FORM: u8 = 1 << 4;
  const FORCE_SIGN: u8 = 1 << 1;
  const LEADING_SPACE_FOR_POSITIVE: u8 = 1 << 2;
  const LEFT_ALIGN: u8 = 1 << 0;
  const ZERO_PAD: u8 = 1 << 3;

  const fn has_flag(self, flag: u8) -> bool {
    self.flags & flag != 0
  }

  const fn left_align(self) -> bool {
    self.has_flag(Self::LEFT_ALIGN)
  }

  const fn force_sign(self) -> bool {
    self.has_flag(Self::FORCE_SIGN)
  }

  const fn leading_space_for_positive(self) -> bool {
    self.has_flag(Self::LEADING_SPACE_FOR_POSITIVE)
  }

  const fn zero_pad(self) -> bool {
    self.has_flag(Self::ZERO_PAD)
  }

  const fn alternate_form(self) -> bool {
    self.has_flag(Self::ALTERNATE_FORM)
  }
}

impl VarArgCursor {
  unsafe fn from_va_list(ap: *mut c_void) -> Self {
    if ap.is_null() {
      return Self {
        gp_offset: 48,
        reg_save_area: core::ptr::null(),
        overflow_arg_area: core::ptr::null(),
      };
    }

    // SAFETY: caller must pass a valid SysV va_list pointer.
    let va_list = unsafe { &*ap.cast::<SysVVaList>() };

    Self {
      gp_offset: va_list.gp_offset,
      reg_save_area: va_list.reg_save_area.cast::<u8>(),
      overflow_arg_area: va_list.overflow_arg_area.cast::<u64>(),
    }
  }

  fn next_u64(&mut self) -> Result<u64, ()> {
    if self.gp_offset < 48 && !self.reg_save_area.is_null() {
      let slot_offset =
        usize::try_from(self.gp_offset).unwrap_or_else(|_| unreachable!("u32 fits usize"));
      // SAFETY: `gp_offset < 48` guarantees one full u64 slot in GP save area.
      let value = unsafe {
        self
          .reg_save_area
          .add(slot_offset)
          .cast::<u64>()
          .read_unaligned()
      };

      self.gp_offset = self.gp_offset.saturating_add(8);

      return Ok(value);
    }

    if self.overflow_arg_area.is_null() {
      return Err(());
    }

    // SAFETY: `overflow_arg_area` points to readable u64 slots and advances by one.
    let value = unsafe { self.overflow_arg_area.read_unaligned() };
    // SAFETY: pointer arithmetic over slot-sized va_list overflow area.
    self.overflow_arg_area = unsafe { self.overflow_arg_area.add(1) };

    Ok(value)
  }

  fn next_ptr<T>(&mut self) -> Result<*const T, ()> {
    let raw = self.next_u64()?;
    let addr = usize::try_from(raw).map_err(|_| ())?;

    Ok(addr as *const T)
  }

  fn next_u32(&mut self) -> Result<u32, ()> {
    let raw = self.next_u64()?;

    u32::try_from(raw & u64::from(u32::MAX)).map_err(|_| ())
  }

  fn next_c_int(&mut self) -> Result<c_int, ()> {
    let low = self.next_u32()?;

    Ok(c_int::from_ne_bytes(low.to_ne_bytes()))
  }
}

impl OutputSink {
  const fn new(buffer: *mut c_char, capacity: usize) -> Self {
    Self {
      buffer,
      capacity,
      bytes_written: 0,
      bytes_required: 0,
    }
  }

  fn push_byte(&mut self, byte: u8) {
    self.push_repeated(byte, 1);
  }

  fn push_repeated(&mut self, byte: u8, repeat: usize) {
    self.bytes_required = self.bytes_required.saturating_add(repeat);

    if repeat == 0 || self.capacity == 0 || self.buffer.is_null() {
      return;
    }

    let writable_limit = self.capacity.saturating_sub(1);

    if self.bytes_written >= writable_limit {
      return;
    }

    let writable_len = repeat.min(writable_limit - self.bytes_written);

    // SAFETY: destination points to writable output buffer and length is clamped.
    unsafe {
      core::ptr::write_bytes(
        self.buffer.cast::<u8>().add(self.bytes_written),
        byte,
        writable_len,
      );
    }
    self.bytes_written += writable_len;
  }

  fn push_bytes(&mut self, bytes: &[u8]) {
    self.bytes_required = self.bytes_required.saturating_add(bytes.len());

    if bytes.is_empty() || self.capacity == 0 || self.buffer.is_null() {
      return;
    }

    let writable_limit = self.capacity.saturating_sub(1);

    if self.bytes_written >= writable_limit {
      return;
    }

    let writable_len = bytes.len().min(writable_limit - self.bytes_written);

    // SAFETY: destination/source are valid for `writable_len` bytes.
    unsafe {
      core::ptr::copy_nonoverlapping(
        bytes.as_ptr(),
        self.buffer.cast::<u8>().add(self.bytes_written),
        writable_len,
      );
    }
    self.bytes_written += writable_len;
  }

  fn terminate(&mut self) {
    if self.capacity == 0 || self.buffer.is_null() {
      return;
    }

    let terminator_index = self.bytes_written.min(self.capacity - 1);

    // SAFETY: `terminator_index < capacity` guarantees an in-bounds write.
    unsafe {
      self.buffer.cast::<u8>().add(terminator_index).write(0);
    }
  }
}

const fn is_valid_buffering_mode(mode: c_int) -> bool {
  mode == _IOFBF || mode == _IOLBF || mode == _IONBF
}

fn size_t_to_usize(value: size_t) -> usize {
  usize::try_from(value)
    .unwrap_or_else(|_| unreachable!("size_t must fit usize on x86_64-unknown-linux-gnu"))
}

fn required_len_to_c_int(required_len: usize) -> c_int {
  c_int::try_from(required_len).unwrap_or(c_int::MAX)
}

fn stream_registry() -> &'static Mutex<Vec<StreamState>> {
  static REGISTRY: OnceLock<Mutex<Vec<StreamState>>> = OnceLock::new();

  REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

fn stream_registry_guard() -> MutexGuard<'static, Vec<StreamState>> {
  match stream_registry().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn stream_key(stream: *mut FILE) -> usize {
  stream.addr()
}

fn stream_state_mut_or_insert(registry: &mut Vec<StreamState>, key: usize) -> &mut StreamState {
  if let Some(position) = registry.iter().position(|state| state.stream_key == key) {
    return registry
      .get_mut(position)
      .unwrap_or_else(|| unreachable!("stream position from `position` must remain valid"));
  }

  registry.push(StreamState {
    stream_key: key,
    buffering_mode: _IONBF,
    buffer_size: 0,
    user_buffer_addr: 0,
    explicit_buffering_config: false,
    io_active: false,
    host_backed_io: false,
    host_stream_identity: None,
  });

  registry
    .last_mut()
    .unwrap_or_else(|| unreachable!("stream state was just inserted"))
}

fn mark_all_streams_as_io_active() {
  // SAFETY: reading host libc standard stream pointers for stream tracking only.
  let (stdin, stdout, stderr) = unsafe { (host_stdin, host_stdout, host_stderr) };
  let mut registry = stream_registry_guard();

  for stream_state in &mut *registry {
    stream_state.io_active = true;
  }

  mark_host_stream_as_io_active(&mut registry, stdin);
  mark_host_stream_as_io_active(&mut registry, stdout);
  mark_host_stream_as_io_active(&mut registry, stderr);

  drop(registry);
}

fn mark_host_stream_as_io_active(registry: &mut Vec<StreamState>, stream: *mut FILE) {
  if stream.is_null() {
    return;
  }

  let stream_state = stream_state_mut_or_insert(registry, stream_key(stream));

  stream_state.io_active = true;
  stream_state.host_backed_io = true;
  stream_state.host_stream_identity = read_host_stream_identity(stream);
}

fn mark_stream_as_io_active(stream: *mut FILE) -> bool {
  let host_standard_stream = is_host_standard_stream(stream);
  let key = stream_key(stream);
  let mut registry = stream_registry_guard();
  let host_backed_io = {
    let stream_state = stream_state_mut_or_insert(&mut registry, key);

    stream_state.io_active = true;

    if host_standard_stream {
      stream_state.host_backed_io = true;
      stream_state.host_stream_identity = read_host_stream_identity(stream);
    }

    stream_state.host_backed_io
  };

  drop(registry);

  host_backed_io
}

fn is_host_standard_stream(stream: *mut FILE) -> bool {
  // SAFETY: reading host libc standard stream pointers for pointer comparison only.
  let (stdin, stdout, stderr) = unsafe { (host_stdin, host_stdout, host_stderr) };

  stream == stdin || stream == stdout || stream == stderr
}

fn mark_stream_as_host_io_active(stream: *mut FILE) {
  let key = stream_key(stream);
  let current_identity = read_host_stream_identity(stream);
  let mut registry = stream_registry_guard();

  {
    let stream_state = stream_state_mut_or_insert(&mut registry, key);
    let previous_identity = stream_state.host_stream_identity;

    if previous_identity.is_some()
      && current_identity.is_some()
      && previous_identity != current_identity
    {
      // Reset stale buffering metadata when a recycled pointer now refers to a
      // different host stream instance.
      stream_state.buffering_mode = _IONBF;
      stream_state.buffer_size = 0;
      stream_state.user_buffer_addr = 0;
      stream_state.explicit_buffering_config = false;
    }

    stream_state.io_active = true;
    stream_state.host_backed_io = true;
    stream_state.host_stream_identity = current_identity;
  }

  drop(registry);
}

fn stream_explicit_buffering_mode(stream: *mut FILE) -> Option<c_int> {
  let key = stream_key(stream);
  let registry = stream_registry_guard();
  let explicit_mode = registry
    .iter()
    .find(|stream_state| stream_state.stream_key == key)
    .and_then(|stream_state| {
      (stream_state.explicit_buffering_config && !stream_state.io_active)
        .then_some(stream_state.buffering_mode)
    });

  drop(registry);

  explicit_mode
}

const unsafe fn c_string_len(ptr: *const c_char) -> usize {
  let mut len = 0_usize;
  let mut cursor = ptr;

  loop {
    // SAFETY: caller guarantees `ptr` points to a readable NUL-terminated C string.
    let byte = unsafe { cursor.read() };

    if byte == 0 {
      return len;
    }

    len = len.saturating_add(1);
    // SAFETY: moving through the same readable NUL-terminated C string.
    cursor = unsafe { cursor.add(1) };
  }
}

unsafe fn c_string_prefix_len(ptr: *const c_char, precision: Option<usize>) -> usize {
  let mut len = 0_usize;
  let mut cursor = ptr;
  let max_len = precision.unwrap_or(usize::MAX);

  while len < max_len {
    // SAFETY: caller guarantees `ptr` points to a readable NUL-terminated C string.
    let byte = unsafe { cursor.read() };

    if byte == 0 {
      break;
    }

    len = len.saturating_add(1);
    // SAFETY: moving through the same readable NUL-terminated C string.
    cursor = unsafe { cursor.add(1) };
  }

  len
}

unsafe fn c_string_prefix_contains_byte(
  ptr: *const c_char,
  target: u8,
  precision: Option<usize>,
) -> bool {
  let target_char = c_char::from_ne_bytes([target]);
  let mut cursor = ptr;
  let mut len = 0_usize;
  let max_len = precision.unwrap_or(usize::MAX);

  while len < max_len {
    // SAFETY: caller guarantees `ptr` points to a readable NUL-terminated C string.
    let byte = unsafe { cursor.read() };

    if byte == 0 {
      return false;
    }

    if byte == target_char {
      return true;
    }

    len = len.saturating_add(1);
    // SAFETY: moving through the same readable NUL-terminated C string.
    cursor = unsafe { cursor.add(1) };
  }

  false
}

fn parse_number(bytes: &[u8], mut index: usize) -> Option<(usize, usize)> {
  let mut value = 0_usize;
  let mut consumed = false;

  while let Some(byte) = bytes.get(index).copied() {
    if !byte.is_ascii_digit() {
      break;
    }

    consumed = true;
    value = value
      .saturating_mul(10)
      .saturating_add(usize::from(byte - b'0'));
    index += 1;
  }

  if consumed { Some((value, index)) } else { None }
}

fn parse_length_modifier(bytes: &[u8], index: usize) -> Option<(LengthModifier, usize)> {
  match bytes.get(index).copied() {
    Some(b'h') => {
      if bytes.get(index + 1).copied() == Some(b'h') {
        Some((LengthModifier::Hh, index + 2))
      } else {
        Some((LengthModifier::H, index + 1))
      }
    }
    Some(b'l') => {
      if bytes.get(index + 1).copied() == Some(b'l') {
        Some((LengthModifier::Ll, index + 2))
      } else {
        Some((LengthModifier::L, index + 1))
      }
    }
    Some(b'j') => Some((LengthModifier::J, index + 1)),
    Some(b't') => Some((LengthModifier::T, index + 1)),
    Some(b'z') => Some((LengthModifier::Z, index + 1)),
    Some(b'L') => None,
    _ => Some((LengthModifier::Default, index)),
  }
}

const fn is_integer_conversion(conversion: u8) -> bool {
  matches!(conversion, b'd' | b'i' | b'u' | b'x' | b'X' | b'o')
}

const fn is_pointer_conversion(conversion: u8) -> bool {
  conversion == b'p'
}

const fn is_count_conversion(conversion: u8) -> bool {
  conversion == b'n'
}

fn parse_format_directive(bytes: &[u8], mut index: usize) -> Option<FormatDirective> {
  let mut flags = 0_u8;

  while let Some(byte) = bytes.get(index).copied() {
    match byte {
      b'-' => {
        flags |= FormatDirective::LEFT_ALIGN;
        index += 1;
      }
      b'+' => {
        flags |= FormatDirective::FORCE_SIGN;
        index += 1;
      }
      b' ' => {
        flags |= FormatDirective::LEADING_SPACE_FOR_POSITIVE;
        index += 1;
      }
      b'0' => {
        flags |= FormatDirective::ZERO_PAD;
        index += 1;
      }
      b'#' => {
        flags |= FormatDirective::ALTERNATE_FORM;
        index += 1;
      }
      _ => break,
    }
  }

  let width = match bytes.get(index).copied() {
    Some(b'*') => {
      index += 1;
      Some(CountSpec::FromArgs)
    }
    Some(byte) if byte.is_ascii_digit() => {
      let (value, next) = parse_number(bytes, index)?;

      index = next;
      Some(CountSpec::Literal(value))
    }
    _ => None,
  };
  let precision = if bytes.get(index).copied() == Some(b'.') {
    index += 1;

    match bytes.get(index).copied() {
      Some(b'*') => {
        index += 1;
        Some(CountSpec::FromArgs)
      }
      Some(byte) if byte.is_ascii_digit() => {
        let (value, next) = parse_number(bytes, index)?;

        index = next;
        Some(CountSpec::Literal(value))
      }
      _ => Some(CountSpec::Literal(0)),
    }
  } else {
    None
  };
  let (length, next_index) = parse_length_modifier(bytes, index)?;

  index = next_index;

  let conversion = *bytes.get(index)?;
  let is_string_or_char = conversion == b's' || conversion == b'c';
  let is_pointer = is_pointer_conversion(conversion);
  let is_count = is_count_conversion(conversion);

  if !is_string_or_char && !is_integer_conversion(conversion) && !is_pointer && !is_count {
    return None;
  }

  if is_string_or_char {
    if length != LengthModifier::Default {
      return None;
    }

    if flags
      & (FormatDirective::FORCE_SIGN
        | FormatDirective::LEADING_SPACE_FOR_POSITIVE
        | FormatDirective::ZERO_PAD
        | FormatDirective::ALTERNATE_FORM)
      != 0
    {
      return None;
    }
  } else if is_pointer {
    if length != LengthModifier::Default {
      return None;
    }
  } else if is_count {
    let disallowed_flags = FormatDirective::FORCE_SIGN
      | FormatDirective::LEADING_SPACE_FOR_POSITIVE
      | FormatDirective::ZERO_PAD
      | FormatDirective::ALTERNATE_FORM
      | FormatDirective::LEFT_ALIGN;

    if (flags & disallowed_flags) != 0 || width.is_some() || precision.is_some() {
      return None;
    }
  } else if ((flags & (FormatDirective::FORCE_SIGN | FormatDirective::LEADING_SPACE_FOR_POSITIVE))
    != 0
    && conversion != b'd'
    && conversion != b'i')
    || ((flags & FormatDirective::ALTERNATE_FORM) != 0
      && conversion != b'x'
      && conversion != b'X'
      && conversion != b'o')
  {
    return None;
  }

  Some(FormatDirective {
    flags,
    width,
    precision,
    length,
    conversion,
    next_index: index + 1,
  })
}

fn resolve_width(
  width_spec: Option<CountSpec>,
  arg_cursor: &mut VarArgCursor,
  left_align: &mut bool,
) -> Result<usize, ()> {
  match width_spec {
    None => Ok(0),
    Some(CountSpec::Literal(value)) => Ok(value),
    Some(CountSpec::FromArgs) => {
      let value = arg_cursor.next_c_int()?;

      if value < 0 {
        *left_align = true;

        let abs_width = i64::from(value).unsigned_abs();

        return Ok(usize::try_from(abs_width).unwrap_or(usize::MAX));
      }

      Ok(usize::try_from(value).map_err(|_| ())?)
    }
  }
}

fn resolve_precision(
  precision_spec: Option<CountSpec>,
  arg_cursor: &mut VarArgCursor,
) -> Result<Option<usize>, ()> {
  match precision_spec {
    None => Ok(None),
    Some(CountSpec::Literal(value)) => Ok(Some(value)),
    Some(CountSpec::FromArgs) => {
      let value = arg_cursor.next_c_int()?;

      if value < 0 {
        return Ok(None);
      }

      Ok(Some(usize::try_from(value).map_err(|_| ())?))
    }
  }
}

fn digit_to_ascii(value: u8, uppercase: bool) -> u8 {
  match value {
    0..=9 => b'0' + value,
    10..=15 if uppercase => b'A' + (value - 10),
    10..=15 => b'a' + (value - 10),
    _ => unreachable!("nibble value must be in range 0..=15"),
  }
}

fn unsigned_to_ascii(mut value: u128, base: u8, uppercase: bool) -> Vec<u8> {
  debug_assert!(base == 8 || base == 10 || base == 16);

  if value == 0 {
    return vec![b'0'];
  }

  let radix = u128::from(base);
  let mut reversed = [0_u8; 128];
  let mut len = 0_usize;

  while value != 0 {
    let digit = u8::try_from(value % radix).unwrap_or_else(|_| unreachable!("digit fits in u8"));

    reversed[len] = digit_to_ascii(digit, uppercase);
    len += 1;
    value /= radix;
  }

  let mut rendered = Vec::with_capacity(len);

  for digit in reversed[..len].iter().rev().copied() {
    rendered.push(digit);
  }

  rendered
}

fn promoted_c_int_to_i8(value: c_int) -> i8 {
  let low = u32::from_ne_bytes(value.to_ne_bytes()) & u32::from(u8::MAX);
  let as_u8 = u8::try_from(low).unwrap_or_else(|_| unreachable!("masked to 8 bits"));

  i8::from_ne_bytes([as_u8])
}

fn promoted_c_int_to_i16(value: c_int) -> i16 {
  let low = u32::from_ne_bytes(value.to_ne_bytes()) & u32::from(u16::MAX);
  let as_u16 = u16::try_from(low).unwrap_or_else(|_| unreachable!("masked to 16 bits"));

  i16::from_ne_bytes(as_u16.to_ne_bytes())
}

fn promoted_u32_to_u8(value: u32) -> u8 {
  let low = value & u32::from(u8::MAX);

  u8::try_from(low).unwrap_or_else(|_| unreachable!("masked to 8 bits"))
}

fn promoted_u32_to_u16(value: u32) -> u16 {
  let low = value & u32::from(u16::MAX);

  u16::try_from(low).unwrap_or_else(|_| unreachable!("masked to 16 bits"))
}

fn read_signed_argument(arg_cursor: &mut VarArgCursor, length: LengthModifier) -> Result<i128, ()> {
  match length {
    LengthModifier::Default => Ok(i128::from(arg_cursor.next_c_int()?)),
    LengthModifier::Hh => Ok(i128::from(promoted_c_int_to_i8(arg_cursor.next_c_int()?))),
    LengthModifier::H => Ok(i128::from(promoted_c_int_to_i16(arg_cursor.next_c_int()?))),
    LengthModifier::L | LengthModifier::Ll | LengthModifier::J => {
      let raw = arg_cursor.next_u64()?;

      Ok(i128::from(i64::from_ne_bytes(raw.to_ne_bytes())))
    }
    LengthModifier::T => {
      let raw = arg_cursor.next_u64()?;
      let usize_value = usize::try_from(raw).map_err(|_| ())?;
      let signed_value = isize::from_ne_bytes(usize_value.to_ne_bytes());

      Ok(signed_value as i128)
    }
    LengthModifier::Z => {
      let raw = arg_cursor.next_u64()?;
      let usize_value = usize::try_from(raw).map_err(|_| ())?;
      let signed_value = isize::from_ne_bytes(usize_value.to_ne_bytes());

      Ok(signed_value as i128)
    }
  }
}

fn read_unsigned_argument(
  arg_cursor: &mut VarArgCursor,
  length: LengthModifier,
) -> Result<u128, ()> {
  match length {
    LengthModifier::Default => Ok(u128::from(arg_cursor.next_u32()?)),
    LengthModifier::Hh => Ok(u128::from(promoted_u32_to_u8(arg_cursor.next_u32()?))),
    LengthModifier::H => Ok(u128::from(promoted_u32_to_u16(arg_cursor.next_u32()?))),
    LengthModifier::L | LengthModifier::Ll | LengthModifier::J => {
      Ok(u128::from(arg_cursor.next_u64()?))
    }
    LengthModifier::T => {
      let raw = arg_cursor.next_u64()?;
      let as_usize = usize::try_from(raw).map_err(|_| ())?;

      u128::try_from(as_usize).map_err(|_| ())
    }
    LengthModifier::Z => {
      let raw = arg_cursor.next_u64()?;
      let as_usize = usize::try_from(raw).map_err(|_| ())?;

      u128::try_from(as_usize).map_err(|_| ())
    }
  }
}

fn render_formatted_integer(
  sink: &mut OutputSink,
  directive: &FormatDirective,
  left_align: bool,
  width: usize,
  precision: Option<usize>,
  value: u128,
  is_negative: bool,
) {
  let (base, uppercase) = match directive.conversion {
    b'd' | b'i' | b'u' => (10_u8, false),
    b'x' => (16_u8, false),
    b'X' => (16_u8, true),
    b'o' => (8_u8, false),
    _ => unreachable!("integer renderer called for non-integer conversion"),
  };
  let is_signed_conversion = directive.conversion == b'd' || directive.conversion == b'i';
  let sign = if is_negative {
    Some(b'-')
  } else if is_signed_conversion && directive.force_sign() {
    Some(b'+')
  } else if is_signed_conversion && directive.leading_space_for_positive() {
    Some(b' ')
  } else {
    None
  };
  let mut effective_precision = precision;
  let mut digits = if precision == Some(0) && value == 0 {
    Vec::new()
  } else {
    unsigned_to_ascii(value, base, uppercase)
  };

  if directive.conversion == b'o' && directive.alternate_form() {
    if value == 0 && precision == Some(0) {
      digits.push(b'0');
    } else if value != 0 {
      let forced_digits = digits.len().saturating_add(1);

      effective_precision = Some(effective_precision.unwrap_or(0).max(forced_digits));
    }
  }

  let prefix: &[u8] = if directive.alternate_form() && value != 0 {
    match directive.conversion {
      b'x' => b"0x",
      b'X' => b"0X",
      _ => b"",
    }
  } else {
    b""
  };
  let precision_zeros = effective_precision.map_or(0, |target| target.saturating_sub(digits.len()));
  let sign_len = usize::from(sign.is_some());
  let base_len = sign_len + prefix.len() + precision_zeros + digits.len();
  let use_zero_pad = directive.zero_pad() && !left_align && effective_precision.is_none();

  if !left_align && !use_zero_pad && width > base_len {
    sink.push_repeated(b' ', width - base_len);
  }

  if let Some(sign_byte) = sign {
    sink.push_byte(sign_byte);
  }

  sink.push_bytes(prefix);

  if !left_align && use_zero_pad && width > base_len {
    sink.push_repeated(b'0', width - base_len);
  }

  sink.push_repeated(b'0', precision_zeros);
  sink.push_bytes(&digits);

  if left_align && width > base_len {
    sink.push_repeated(b' ', width - base_len);
  }
}

fn render_formatted_pointer(
  sink: &mut OutputSink,
  left_align: bool,
  zero_pad: bool,
  width: usize,
  precision: Option<usize>,
  sign: Option<u8>,
  pointer_addr: usize,
) {
  let prefix = b"0x";
  // Keep one hexadecimal digit for null pointers even with `%.0p` to avoid
  // rendering an empty pointer payload.
  let pointer_value = u64::try_from(pointer_addr)
    .unwrap_or_else(|_| unreachable!("pointer address must fit u64 on supported targets"));
  let digits = unsigned_to_ascii(u128::from(pointer_value), 16, false);
  let precision_zeros = precision.map_or(0, |target| target.saturating_sub(digits.len()));
  let sign_len = usize::from(sign.is_some());
  let content_len = sign_len + prefix.len() + precision_zeros + digits.len();
  let use_zero_pad = zero_pad && !left_align && precision.is_none();

  if !left_align && !use_zero_pad && width > content_len {
    sink.push_repeated(b' ', width - content_len);
  }

  if let Some(sign_byte) = sign {
    sink.push_byte(sign_byte);
  }

  sink.push_bytes(prefix);

  if !left_align && use_zero_pad && width > content_len {
    sink.push_repeated(b'0', width - content_len);
  }

  sink.push_repeated(b'0', precision_zeros);
  sink.push_bytes(&digits);

  if left_align && width > content_len {
    sink.push_repeated(b' ', width - content_len);
  }
}

fn write_count_conversion(
  arg_cursor: &mut VarArgCursor,
  length: LengthModifier,
  bytes_required: usize,
) -> Result<(), ()> {
  let count_value = c_int::try_from(bytes_required).map_err(|_| ())?;

  match length {
    LengthModifier::Default => {
      let pointer = arg_cursor.next_ptr::<c_int>()?.cast_mut();

      if pointer.is_null() {
        return Err(());
      }

      // SAFETY: pointer comes from caller-provided `%n` argument and null is rejected above.
      unsafe {
        pointer.write(count_value);
      }
    }
    LengthModifier::Hh => {
      let pointer = arg_cursor.next_ptr::<i8>()?.cast_mut();

      if pointer.is_null() {
        return Err(());
      }

      let value = i8::try_from(count_value).map_err(|_| ())?;

      // SAFETY: pointer comes from caller-provided `%hhn` argument and null is rejected above.
      unsafe {
        pointer.write(value);
      }
    }
    LengthModifier::H => {
      let pointer = arg_cursor.next_ptr::<i16>()?.cast_mut();

      if pointer.is_null() {
        return Err(());
      }

      let value = i16::try_from(count_value).map_err(|_| ())?;

      // SAFETY: pointer comes from caller-provided `%hn` argument and null is rejected above.
      unsafe {
        pointer.write(value);
      }
    }
    LengthModifier::L | LengthModifier::Ll | LengthModifier::J => {
      let pointer = arg_cursor.next_ptr::<i64>()?.cast_mut();

      if pointer.is_null() {
        return Err(());
      }

      // SAFETY: pointer comes from caller-provided `%ln`/`%lln`/`%jn` argument and null is rejected above.
      unsafe {
        pointer.write(i64::from(count_value));
      }
    }
    LengthModifier::Z | LengthModifier::T => {
      let pointer = arg_cursor.next_ptr::<isize>()?.cast_mut();

      if pointer.is_null() {
        return Err(());
      }

      let value = isize::try_from(count_value).map_err(|_| ())?;

      // SAFETY: pointer comes from caller-provided `%zn`/`%tn` argument and null is rejected above.
      unsafe {
        pointer.write(value);
      }
    }
  }

  Ok(())
}

fn consume_count_conversion_argument(
  arg_cursor: &mut VarArgCursor,
  length: LengthModifier,
) -> Result<(), ()> {
  match length {
    LengthModifier::Default => {
      if arg_cursor.next_ptr::<c_int>()?.is_null() {
        return Err(());
      }
    }
    LengthModifier::Hh => {
      if arg_cursor.next_ptr::<i8>()?.is_null() {
        return Err(());
      }
    }
    LengthModifier::H => {
      if arg_cursor.next_ptr::<i16>()?.is_null() {
        return Err(());
      }
    }
    LengthModifier::L | LengthModifier::Ll | LengthModifier::J => {
      if arg_cursor.next_ptr::<i64>()?.is_null() {
        return Err(());
      }
    }
    LengthModifier::Z | LengthModifier::T => {
      if arg_cursor.next_ptr::<isize>()?.is_null() {
        return Err(());
      }
    }
  }

  Ok(())
}

unsafe fn formatted_output_contains_newline(format: *const c_char, ap: *mut c_void) -> bool {
  fn remaining_format_contains_newline(format_bytes: &[u8], index: usize) -> bool {
    format_bytes
      .get(index..)
      .is_some_and(|remaining| remaining.contains(&b'\n'))
  }

  // SAFETY: caller guarantees `format` points to a readable NUL-terminated C string.
  let format_len = unsafe { c_string_len(format) };
  // SAFETY: `format_len` was measured from `format` above.
  let format_bytes = unsafe { core::slice::from_raw_parts(format.cast::<u8>(), format_len) };
  // SAFETY: this copies va_list cursor state without mutating caller-owned `ap`.
  let mut arg_cursor = unsafe { VarArgCursor::from_va_list(ap) };
  let mut index = 0_usize;

  while let Some(byte) = format_bytes.get(index).copied() {
    if byte != b'%' {
      if byte == b'\n' {
        return true;
      }

      index = index.saturating_add(1);
      continue;
    }

    index = index.saturating_add(1);

    if format_bytes.get(index).copied() == Some(b'%') {
      index = index.saturating_add(1);
      continue;
    }

    let Some(directive) = parse_format_directive(format_bytes, index) else {
      return remaining_format_contains_newline(format_bytes, index);
    };
    let mut left_align = directive.left_align();

    if resolve_width(directive.width, &mut arg_cursor, &mut left_align).is_err() {
      return remaining_format_contains_newline(format_bytes, index);
    }

    let Ok(precision) = resolve_precision(directive.precision, &mut arg_cursor) else {
      return remaining_format_contains_newline(format_bytes, index);
    };

    match directive.conversion {
      b's' => {
        let Ok(source) = arg_cursor.next_ptr::<c_char>() else {
          return remaining_format_contains_newline(format_bytes, index);
        };

        if !source.is_null() {
          // SAFETY: `%s` consumes a readable NUL-terminated string when non-null.
          if unsafe { c_string_prefix_contains_byte(source, b'\n', precision) } {
            return true;
          }
        }
      }
      b'c' => {
        let Ok(value) = arg_cursor.next_c_int() else {
          return remaining_format_contains_newline(format_bytes, index);
        };
        let low = u32::from_ne_bytes(value.to_ne_bytes()) & u32::from(u8::MAX);
        let emitted = u8::try_from(low).unwrap_or_else(|_| unreachable!("masked to 8 bits"));

        if emitted == b'\n' {
          return true;
        }
      }
      b'd' | b'i' => {
        if read_signed_argument(&mut arg_cursor, directive.length).is_err() {
          return remaining_format_contains_newline(format_bytes, index);
        }
      }
      b'u' | b'x' | b'X' | b'o' => {
        if read_unsigned_argument(&mut arg_cursor, directive.length).is_err() {
          return remaining_format_contains_newline(format_bytes, index);
        }
      }
      b'p' => {
        if arg_cursor.next_ptr::<c_void>().is_err() {
          return remaining_format_contains_newline(format_bytes, index);
        }
      }
      b'n' => {
        if consume_count_conversion_argument(&mut arg_cursor, directive.length).is_err() {
          return remaining_format_contains_newline(format_bytes, index);
        }
      }
      _ => return remaining_format_contains_newline(format_bytes, index),
    }

    index = directive.next_index;
  }

  false
}

fn render_padding(sink: &mut OutputSink, width: usize, content_len: usize, left_align: bool) {
  if left_align || width <= content_len {
    return;
  }

  sink.push_repeated(b' ', width - content_len);
}

fn render_trailing_padding(
  sink: &mut OutputSink,
  width: usize,
  content_len: usize,
  left_align: bool,
) {
  if !left_align || width <= content_len {
    return;
  }

  sink.push_repeated(b' ', width - content_len);
}

#[link(name = "dl")]
unsafe extern "C" {
  fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

unsafe extern "C" {
  #[link_name = "_IO_fflush"]
  fn host_fflush_raw(stream: *mut FILE) -> c_int;
  fn fileno(stream: *mut FILE) -> c_int;
  #[link_name = "stdin"]
  static mut host_stdin: *mut FILE;
  #[link_name = "stdout"]
  static mut host_stdout: *mut FILE;
  #[link_name = "stderr"]
  static mut host_stderr: *mut FILE;
}

unsafe fn host_fflush(stream: *mut FILE) -> c_int {
  // SAFETY: caller upholds host `fflush` contract for `stream`.
  unsafe { host_fflush_raw(stream) }
}

unsafe fn host_fileno(stream: *mut FILE) -> c_int {
  // SAFETY: caller upholds host `fileno` contract for `stream`.
  unsafe { fileno(stream) }
}

fn fail_vsnprintf_with_einval(sink: &mut OutputSink) -> c_int {
  set_errno(EINVAL);
  sink.terminate();

  -1
}

fn resolve_host_vfprintf() -> Option<HostVfprintfFn> {
  // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
  let symbol_ptr = unsafe { dlsym(RTLD_NEXT, VFPRINTF_SYMBOL_NAME.as_ptr().cast()) };

  if symbol_ptr.is_null() {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `vfprintf`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostVfprintfFn>(symbol_ptr) })
}

fn host_vfprintf() -> Option<HostVfprintfFn> {
  static HOST_VFPRINTF: OnceLock<Option<HostVfprintfFn>> = OnceLock::new();

  *HOST_VFPRINTF.get_or_init(resolve_host_vfprintf)
}

fn resolve_host_errno_location() -> Option<HostErrnoLocationFn> {
  // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
  let symbol_ptr = unsafe { dlsym(RTLD_NEXT, ERRNO_LOCATION_SYMBOL_NAME.as_ptr().cast()) };

  if symbol_ptr.is_null() {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `__errno_location`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostErrnoLocationFn>(symbol_ptr) })
}

fn host_errno_location() -> Option<HostErrnoLocationFn> {
  static HOST_ERRNO_LOCATION: OnceLock<Option<HostErrnoLocationFn>> = OnceLock::new();

  *HOST_ERRNO_LOCATION.get_or_init(resolve_host_errno_location)
}

fn read_host_errno() -> Option<c_int> {
  let host_errno_location = host_errno_location()?;
  // SAFETY: host symbol resolves to `__errno_location`, which returns thread-local errno storage.
  let errno_ptr = unsafe { host_errno_location() };

  if errno_ptr.is_null() {
    return None;
  }

  // SAFETY: host `__errno_location` returned readable thread-local storage.
  Some(unsafe { errno_ptr.read() })
}

fn set_errno_from_host_flush_failure() {
  let host_errno = read_host_errno()
    .and_then(|value| (value > 0).then_some(value))
    .or_else(|| IoError::last_os_error().raw_os_error())
    .unwrap_or(0);

  if host_errno > 0 {
    set_errno(host_errno);
  } else {
    set_errno(EINVAL);
  }
}

fn read_host_stream_identity(stream: *mut FILE) -> Option<u64> {
  if stream.is_null() {
    return None;
  }

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  // SAFETY: `stream` is expected to be a valid host-backed `FILE*`.
  let stream_fd = unsafe { host_fileno(stream) };
  let stream_identity = if stream_fd < 0 {
    None
  } else {
    let proc_fd_path = format!("/proc/self/fd/{stream_fd}");

    std::fs::read_link(proc_fd_path).ok().map(|target| {
      let mut hasher = DefaultHasher::new();

      stream_fd.hash(&mut hasher);
      target.hash(&mut hasher);
      hasher.finish()
    })
  };

  // SAFETY: restore caller-visible errno after identity probing.
  unsafe {
    __errno_location().write(errno_before);
  }

  stream_identity
}

unsafe fn forward_host_vfprintf(
  stream: *mut FILE,
  format: *const c_char,
  ap: *mut c_void,
) -> (c_int, bool) {
  let Some(host_vfprintf) = host_vfprintf() else {
    set_errno(EINVAL);

    return (-1, false);
  };

  // SAFETY: caller upholds host `vfprintf` contracts for stream/format/va_list.
  (unsafe { host_vfprintf(stream, format, ap) }, true)
}

/// C ABI entry point for `fflush`.
///
/// Contract:
/// - `fflush(NULL)` delegates flush-all behavior to host libc and marks all
///   tracked streams as having observed I/O.
/// - `fflush(NULL)` also marks host `stdin`/`stdout`/`stderr` as host-backed
///   I/O-active when available, so later [`setvbuf`] reconfiguration attempts
///   on those streams are rejected.
/// - `fflush(stream)` marks one stream as having observed I/O.
/// - for host `stdin`/`stdout`/`stderr` streams and streams with prior
///   successful host-backed output via [`vfprintf`], `fflush(stream)` delegates
///   per-stream flushing to host libc.
/// - when `stream` is not yet tracked by this module, a stream-state entry is
///   created and marked as I/O-active.
///
/// Returns:
/// - `0` on success
/// - [`EOF`] when host libc reports a delegated flush failure (`fflush(NULL)`
///   or host-backed `fflush(stream)`)
///
/// Current limitation:
/// - non-null streams without known host-backed I/O currently perform only
///   stream-state tracking and do not delegate per-stream flush semantics.
///
/// # Safety
/// - when non-null, `stream` must be a valid `FILE*` handle for the calling
///   program's stdio subsystem.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fflush(stream: *mut FILE) -> c_int {
  if stream.is_null() {
    // SAFETY: `__errno_location` returns writable thread-local errno storage.
    let errno_before = unsafe { __errno_location().read() };
    // SAFETY: host flush-all contract for null stream pointer.
    let flush_status = unsafe { host_fflush(core::ptr::null_mut()) };

    mark_all_streams_as_io_active();

    if flush_status == 0 {
      // SAFETY: restore prior errno for success-path parity with previous contract/tests.
      unsafe {
        __errno_location().write(errno_before);
      }
    } else {
      set_errno_from_host_flush_failure();
    }

    return flush_status;
  }

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let host_backed = mark_stream_as_io_active(stream);

  if !host_backed {
    return 0;
  }

  // SAFETY: host-backed stream was validated by prior successful host `vfprintf`.
  let flush_status = unsafe { host_fflush(stream) };

  if flush_status == 0 {
    // SAFETY: restore prior errno for success-path parity with module contract/tests.
    unsafe {
      __errno_location().write(errno_before);
    }
  } else {
    set_errno_from_host_flush_failure();
  }

  flush_status
}

/// C ABI entry point for `setvbuf`.
///
/// This implementation validates buffering arguments, stores the requested
/// buffering configuration in an internal registry, and tracks whether the
/// stream has already observed I/O through this module.
///
/// Returns:
/// - `0` on success
/// - [`EOF`] on invalid arguments
///
/// # Errors
/// - Sets `errno = EINVAL` when:
///   - `stream` is null
///   - `mode` is not one of `_IOFBF`, `_IOLBF`, `_IONBF`
///   - `mode` is `_IOFBF`/`_IOLBF` and `size == 0`
///   - the stream has already observed I/O activity
///
/// Success-path behavior:
/// - For streams before I/O activity, the requested `mode`, `size`, and
///   caller-provided `buffer` address are recorded in internal stream state.
/// - Previously unseen stream handles are inserted into the internal registry
///   on first successful `setvbuf` call.
/// - When a stream pointer key is reused for a different host-backed stream
///   lifecycle, stale internal I/O-active tracking is discarded before
///   applying the new buffering configuration.
/// - `_IONBF` canonicalizes tracked buffer metadata to `(size=0, buffer=0)`
///   because unbuffered mode ignores caller-provided buffering storage.
/// - Successful configuration clears prior host-backed flush delegation state;
///   host-backed behavior is re-established only after a later host-forwarded
///   [`vfprintf`] call on the same stream.
/// - On success, preserves caller-observed `errno` value.
///
/// # Safety
/// - `stream` must be a valid `FILE*` handle when non-null.
/// - `buffer` is accepted as opaque and is never dereferenced.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setvbuf(
  stream: *mut FILE,
  buffer: *mut c_char,
  mode: c_int,
  size: size_t,
) -> c_int {
  if stream.is_null() || !is_valid_buffering_mode(mode) || (mode != _IONBF && size == 0) {
    set_errno(EINVAL);

    return EOF;
  }

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let key = stream_key(stream);
  let mut registry = stream_registry_guard();
  let stream_state = stream_state_mut_or_insert(&mut registry, key);

  if stream_state.io_active && stream_state.host_backed_io {
    let previous_identity = stream_state.host_stream_identity;
    let current_identity = read_host_stream_identity(stream);

    if previous_identity.is_some()
      && current_identity.is_some()
      && previous_identity != current_identity
    {
      stream_state.io_active = false;
      stream_state.host_backed_io = false;
      stream_state.host_stream_identity = current_identity;
    }
  }

  if stream_state.io_active {
    set_errno(EINVAL);

    return EOF;
  }

  let (tracked_size, tracked_buffer_addr) = if mode == _IONBF {
    (0, 0)
  } else {
    (size_t_to_usize(size), buffer.addr())
  };

  stream_state.buffering_mode = mode;
  stream_state.buffer_size = tracked_size;
  stream_state.user_buffer_addr = tracked_buffer_addr;
  stream_state.explicit_buffering_config = true;
  stream_state.host_backed_io = false;
  stream_state.host_stream_identity = None;
  drop(registry);

  // SAFETY: success path restores caller-observed errno for libc parity.
  unsafe {
    __errno_location().write(errno_before);
  }

  0
}

/// C ABI entry point for `vsnprintf` (incremental formatter subset).
///
/// Supported behavior:
/// - literal-byte copying
/// - escaped percent (`%%`) handling
/// - `%s` with optional width/precision (`%5.3s`, `%.*s`, `%*.*s`);
///   null pointers use `(null)` and still honor precision/width
/// - `%c` with optional width and left-adjust flag (`%-3c`)
/// - `%p` with optional width/precision and flags `0`/`-`/`+`/space/`#`
///   (`%12p`, `%.6p`, `%12.6p`, `%012p`, `%+p`, `% p`, `%#p`);
///   null pointers keep one hexadecimal digit for `%.0p` (`0x0`)
/// - `%n` with optional length modifiers (`%n`, `%hhn`, `%hn`, `%ln`, `%lln`, `%jn`, `%tn`, `%zn`)
///   without flags/width/precision directives
/// - integer conversions `%d/%i/%u/%x/%X/%o` with flags `- + space 0 #`
/// - length modifiers `hh/h/l/ll/j/t/z` for integer conversions
/// - required-length reporting and truncating NUL termination
///
/// Current limitation:
/// - unsupported flags/conversions/length modifiers return `-1` and set
///   `errno=EINVAL`.
///
/// `ap` decoding note for the current phase:
/// - this implementation reads arguments from `x86_64` `SysV` `va_list` GP/overflow
///   slots as packed `u64` words.
///
/// # Safety
/// - `format` must be a valid readable NUL-terminated C string.
/// - If `n > 0`, `s` must point to at least `n` writable bytes.
/// - when directives consume arguments, `ap` must reference a valid `x86_64`
///   `SysV` `va_list` with readable slots for all required arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsnprintf(
  s: *mut c_char,
  n: size_t,
  format: *const c_char,
  ap: *mut c_void,
) -> c_int {
  let capacity = size_t_to_usize(n);

  if format.is_null() || (capacity != 0 && s.is_null()) {
    set_errno(EINVAL);

    return -1;
  }

  // SAFETY: `format` is non-null and points to a readable C string.
  let format_len = unsafe { c_string_len(format) };
  // SAFETY: `format_len` was computed by scanning to NUL.
  let format_bytes = unsafe { core::slice::from_raw_parts(format.cast::<u8>(), format_len) };
  // SAFETY: `ap` is decoded only as required by parsed directives.
  let mut arg_cursor = unsafe { VarArgCursor::from_va_list(ap) };
  let mut sink = OutputSink::new(s, capacity);
  let mut index = 0_usize;

  while let Some(byte) = format_bytes.get(index).copied() {
    if byte != b'%' {
      sink.push_byte(byte);
      index += 1;
      continue;
    }

    if index + 1 == format_bytes.len() {
      return fail_vsnprintf_with_einval(&mut sink);
    }

    if format_bytes[index + 1] == b'%' {
      sink.push_byte(b'%');
      index += 2;
      continue;
    }

    let Some(directive) = parse_format_directive(format_bytes, index + 1) else {
      return fail_vsnprintf_with_einval(&mut sink);
    };
    let mut left_align = directive.left_align();
    let Ok(width) = resolve_width(directive.width, &mut arg_cursor, &mut left_align) else {
      return fail_vsnprintf_with_einval(&mut sink);
    };
    let Ok(precision) = resolve_precision(directive.precision, &mut arg_cursor) else {
      return fail_vsnprintf_with_einval(&mut sink);
    };

    match directive.conversion {
      b's' => {
        let null_fallback = b"(null)";
        let source = match arg_cursor.next_ptr::<c_char>() {
          Ok(pointer) if pointer.is_null() => {
            let source_len = precision
              .unwrap_or(null_fallback.len())
              .min(null_fallback.len());

            &null_fallback[..source_len]
          }
          Ok(pointer) => {
            // SAFETY: `pointer` must reference a readable C string.
            let source_len = unsafe { c_string_prefix_len(pointer, precision) };
            // SAFETY: `source_len` was computed from the same C string.
            unsafe { core::slice::from_raw_parts(pointer.cast::<u8>(), source_len) }
          }
          Err(()) => return fail_vsnprintf_with_einval(&mut sink),
        };
        let content_len = source.len();

        render_padding(&mut sink, width, content_len, left_align);
        sink.push_bytes(source);
        render_trailing_padding(&mut sink, width, content_len, left_align);
      }
      b'c' => {
        if precision.is_some() {
          return fail_vsnprintf_with_einval(&mut sink);
        }

        let Ok(value) = arg_cursor.next_c_int() else {
          return fail_vsnprintf_with_einval(&mut sink);
        };
        let low = u32::from_ne_bytes(value.to_ne_bytes()) & u32::from(u8::MAX);
        let byte =
          u8::try_from(low).unwrap_or_else(|_| unreachable!("masked low byte must fit into u8"));

        render_padding(&mut sink, width, 1, left_align);
        sink.push_byte(byte);
        render_trailing_padding(&mut sink, width, 1, left_align);
      }
      b'p' => {
        let Ok(pointer) = arg_cursor.next_ptr::<c_void>() else {
          return fail_vsnprintf_with_einval(&mut sink);
        };
        let sign = if directive.force_sign() {
          Some(b'+')
        } else if directive.leading_space_for_positive() {
          Some(b' ')
        } else {
          None
        };

        render_formatted_pointer(
          &mut sink,
          left_align,
          directive.zero_pad(),
          width,
          precision,
          sign,
          pointer.addr(),
        );
      }
      b'n' => {
        if write_count_conversion(&mut arg_cursor, directive.length, sink.bytes_required).is_err() {
          return fail_vsnprintf_with_einval(&mut sink);
        }
      }
      b'd' | b'i' => {
        let Ok(signed_value) = read_signed_argument(&mut arg_cursor, directive.length) else {
          return fail_vsnprintf_with_einval(&mut sink);
        };
        let is_negative = signed_value.is_negative();
        let magnitude = if is_negative {
          signed_value.unsigned_abs()
        } else {
          u128::try_from(signed_value).unwrap_or_else(|_| unreachable!("non-negative i128 to u128"))
        };

        render_formatted_integer(
          &mut sink,
          &directive,
          left_align,
          width,
          precision,
          magnitude,
          is_negative,
        );
      }
      b'u' | b'x' | b'X' | b'o' => {
        let Ok(unsigned_value) = read_unsigned_argument(&mut arg_cursor, directive.length) else {
          return fail_vsnprintf_with_einval(&mut sink);
        };

        render_formatted_integer(
          &mut sink,
          &directive,
          left_align,
          width,
          precision,
          unsigned_value,
          false,
        );
      }
      _ => return fail_vsnprintf_with_einval(&mut sink),
    }

    index = directive.next_index;
  }

  sink.terminate();

  required_len_to_c_int(sink.bytes_required)
}

/// C ABI entry point for `vfprintf`.
///
/// Current phase behavior:
/// - resolves host libc `vfprintf` via `dlsym(RTLD_NEXT, "vfprintf")`.
/// - forwards stream/format/`va_list` to the resolved host symbol.
/// - writes to the provided `stream`.
/// - when forwarding reaches host libc (after argument validation), marks
///   `stream` as host-backed and I/O-active even when host `vfprintf` fails, so
///   subsequent [`setvbuf`] reconfiguration attempts are rejected and
///   [`fflush(stream)`] can continue delegated per-stream behavior.
/// - when `stream` was explicitly configured through [`setvbuf`] with `_IONBF`,
///   successful writes immediately delegate `fflush(stream)` so bytes become
///   observable on the underlying descriptor without requiring a separate flush.
/// - when `stream` was explicitly configured through [`setvbuf`] with `_IOLBF`,
///   successful writes that emit newline bytes delegate `fflush(stream)` so
///   line-buffered observability is preserved even when newline bytes are
///   produced through formatted arguments (for example `%s` payload expansion).
///
/// Returns:
/// - non-negative byte count on success (excluding the implicit NUL terminator)
/// - negative value on failure
///
/// # Errors
/// - Sets `errno = EINVAL` and returns `-1` when `stream`, `format`, or `ap`
///   is null.
/// - Sets `errno = EINVAL` and returns `-1` when host `vfprintf` resolution
///   fails.
/// - for explicit `_IONBF` streams and newline-emitting explicit `_IOLBF`
///   writes, returns `-1` when post-write delegated `fflush(stream)` fails;
///   `errno` is populated from host failure details.
/// - otherwise propagates host libc stream/format failures and `errno` while
///   still marking the stream as host-backed I/O-active.
///
/// # Safety
/// - `stream` must be a valid writable `FILE*`.
/// - `format` must point to a valid NUL-terminated format string.
/// - `ap` must refer to a valid C `va_list` matching `format`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vfprintf(
  stream: *mut FILE,
  format: *const c_char,
  ap: *mut c_void,
) -> c_int {
  if stream.is_null() || format.is_null() || ap.is_null() {
    set_errno(EINVAL);

    return -1;
  }

  let explicit_mode_before_write = stream_explicit_buffering_mode(stream);
  let line_buffered_emits_newline = if matches!(explicit_mode_before_write, Some(_IOLBF)) {
    // SAFETY: pointers were validated and helper only reads format/va_list.
    unsafe { formatted_output_contains_newline(format, ap) }
  } else {
    false
  };

  // SAFETY: pointers were validated non-null and caller upholds C ABI contracts.
  let (status, delegated_to_host) = unsafe { forward_host_vfprintf(stream, format, ap) };

  if delegated_to_host {
    mark_stream_as_host_io_active(stream);
  }

  if status >= 0 {
    let should_flush_after_write = match explicit_mode_before_write {
      Some(_IONBF) => true,
      Some(_IOLBF) => line_buffered_emits_newline,
      _ => false,
    };

    if should_flush_after_write {
      // SAFETY: `__errno_location` returns writable thread-local errno storage.
      let errno_before = unsafe { __errno_location().read() };
      // SAFETY: successful host-backed write established stream validity.
      let flush_status = unsafe { host_fflush(stream) };

      if flush_status != 0 {
        set_errno_from_host_flush_failure();

        return -1;
      }

      // SAFETY: preserve caller-observed errno on successful delegated flush.
      unsafe {
        __errno_location().write(errno_before);
      }
    }
  }

  status
}

/// C ABI entry point for `vprintf`.
///
/// Current phase behavior:
/// - resolves host libc `stdout`.
/// - forwards to [`vfprintf`] with that stream.
///
/// Returns:
/// - non-negative byte count on success
/// - negative value on failure
///
/// # Errors
/// - Sets `errno = EINVAL` and returns `-1` when `format`/`ap` is null or
///   host `stdout` is unavailable.
/// - otherwise propagates host libc stream/format failures and `errno`.
///
/// # Safety
/// - `format` must point to a valid NUL-terminated format string.
/// - `ap` must refer to a valid C `va_list` matching `format`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vprintf(format: *const c_char, ap: *mut c_void) -> c_int {
  if format.is_null() || ap.is_null() {
    set_errno(EINVAL);

    return -1;
  }

  // SAFETY: reading host-provided global `stdout` pointer.
  let stdout_stream = unsafe { host_stdout };

  if stdout_stream.is_null() {
    set_errno(EINVAL);

    return -1;
  }

  // SAFETY: pointers are non-null and contracts are delegated to `vfprintf`.
  unsafe { vfprintf(stdout_stream, format, ap) }
}

unsafe extern "C" {
  /// C ABI entry point for `printf`.
  ///
  /// Current phase behavior:
  /// - accepts C variadic arguments and forwards to [`vprintf`].
  /// - writes to the process `stdout` stream selected by the host libc.
  /// - when forwarding succeeds, `stdout` is marked as I/O-active so later
  ///   [`setvbuf`] reconfiguration on that stream is rejected.
  ///
  /// Returns:
  /// - non-negative byte count on success
  /// - negative value on failure
  ///
  /// # Errors
  /// - propagates [`vprintf`] validation/forwarding errors.
  /// - on downstream host-libc failure, preserves the host-provided `errno`.
  ///
  /// # Safety
  /// - `format` must point to a valid NUL-terminated format string.
  /// - variadic arguments must match the format contract.
  pub fn printf(format: *const c_char, ...) -> c_int;

  /// C ABI entry point for `fprintf`.
  ///
  /// Current phase behavior:
  /// - accepts C variadic arguments and forwards to [`vfprintf`].
  /// - writes to the `stream` handle interpreted by the host libc.
  /// - when forwarding succeeds, the target stream is marked as I/O-active so
  ///   later [`setvbuf`] reconfiguration on that handle is rejected.
  ///
  /// Returns:
  /// - non-negative byte count on success
  /// - negative value on failure
  ///
  /// # Errors
  /// - propagates [`vfprintf`] validation/forwarding errors.
  /// - on downstream host-libc failure, preserves the host-provided `errno`.
  ///
  /// # Safety
  /// - `stream` must be a valid `FILE*`.
  /// - `format` must point to a valid NUL-terminated format string.
  /// - variadic arguments must match the format contract.
  pub fn fprintf(stream: *mut FILE, format: *const c_char, ...) -> c_int;
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::errno::__errno_location;
  use core::ptr;
  use std::ffi::CString;

  unsafe extern "C" {
    fn close(fd: c_int) -> c_int;
    fn fclose(stream: *mut FILE) -> c_int;
    fn fileno(stream: *mut FILE) -> c_int;
    fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE;
    fn fputs(s: *const c_char, stream: *mut FILE) -> c_int;
    fn tmpfile() -> *mut FILE;
  }

  fn test_lock() -> MutexGuard<'static, ()> {
    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    match TEST_LOCK.get_or_init(|| Mutex::new(())).lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    }
  }

  fn as_file_ptr(marker: &mut u8) -> *mut FILE {
    ptr::from_mut(marker).cast::<FILE>()
  }

  fn as_size_t(value: usize) -> size_t {
    size_t::try_from(value).unwrap_or_else(|_| unreachable!("usize must fit size_t on target"))
  }

  fn c_string(input: &str) -> CString {
    CString::new(input)
      .unwrap_or_else(|_| unreachable!("test literals must not include interior NUL bytes"))
  }

  fn write_errno(value: c_int) {
    // SAFETY: `__errno_location` points to writable thread-local errno.
    unsafe {
      __errno_location().write(value);
    }
  }

  fn read_errno() -> c_int {
    // SAFETY: `__errno_location` points to readable thread-local errno.
    unsafe { __errno_location().read() }
  }

  fn clear_stream_registry_for_tests() {
    stream_registry_guard().clear();
  }

  fn buffering_snapshot_for_tests(stream: *mut FILE) -> Option<(c_int, usize, usize, bool)> {
    let key = stream_key(stream);
    let registry = stream_registry_guard();

    registry
      .iter()
      .find(|state| state.stream_key == key)
      .map(|state| {
        (
          state.buffering_mode,
          state.buffer_size,
          state.user_buffer_addr,
          state.io_active,
        )
      })
  }

  fn host_backed_snapshot_for_tests(stream: *mut FILE) -> Option<bool> {
    let key = stream_key(stream);
    let registry = stream_registry_guard();

    registry
      .iter()
      .find(|state| state.stream_key == key)
      .map(|state| state.host_backed_io)
  }

  #[test]
  fn fflush_null_tracks_host_std_streams_as_active_host_backed_streams() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides standard stream global pointers.
    let stdin_stream = unsafe { host_stdin };
    // SAFETY: host libc provides standard stream global pointers.
    let stdout_stream = unsafe { host_stdout };
    // SAFETY: host libc provides standard stream global pointers.
    let stderr_stream = unsafe { host_stderr };

    for (stream_name, stream) in [
      ("stdin", stdin_stream),
      ("stdout", stdout_stream),
      ("stderr", stderr_stream),
    ] {
      assert!(
        !stream.is_null(),
        "host {stream_name} pointer must be available"
      );
    }

    write_errno(91);

    // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
    let flush_status = unsafe { fflush(ptr::null_mut()) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 91);

    for (stream_name, stream) in [
      ("stdin", stdin_stream),
      ("stdout", stdout_stream),
      ("stderr", stderr_stream),
    ] {
      assert_eq!(
        host_backed_snapshot_for_tests(stream),
        Some(true),
        "host-backed marker must be set for {stream_name}",
      );
      assert_eq!(
        buffering_snapshot_for_tests(stream)
          .map(|(_mode, _size, _buffer_addr, io_active)| io_active),
        Some(true),
        "io_active marker must be set for {stream_name}",
      );
    }

    let mut user_buffer = [0_u8; 8];

    write_errno(0);

    // SAFETY: stream and user buffer pointers are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stdout_stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IONBF,
        as_size_t(0),
      )
    };

    assert_eq!(setvbuf_status, EOF);
    assert_eq!(read_errno(), EINVAL);
  }

  #[test]
  fn fflush_null_failure_still_tracks_host_std_streams_as_active_host_backed() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides standard stream global pointers.
    let stdin_stream = unsafe { host_stdin };
    // SAFETY: host libc provides standard stream global pointers.
    let stdout_stream = unsafe { host_stdout };
    // SAFETY: host libc provides standard stream global pointers.
    let stderr_stream = unsafe { host_stderr };

    for (stream_name, stream) in [
      ("stdin", stdin_stream),
      ("stdout", stdout_stream),
      ("stderr", stderr_stream),
    ] {
      assert!(
        !stream.is_null(),
        "host {stream_name} pointer must be available"
      );
    }

    // SAFETY: host libc provides a valid temporary stream or null on failure.
    let failing_stream = unsafe { tmpfile() };

    assert!(
      !failing_stream.is_null(),
      "tmpfile must provide a stream for failure injection"
    );

    let payload = c_string("i022-flush-null-failure\n");

    // SAFETY: stream and payload pointer are valid for host `fputs`.
    let write_status = unsafe { fputs(payload.as_ptr(), failing_stream) };

    assert!(write_status >= 0, "priming failure stream must succeed");

    // SAFETY: `fileno` expects a valid host stream handle.
    let failing_fd = unsafe { fileno(failing_stream) };

    assert!(
      failing_fd >= 0,
      "failure stream must expose a file descriptor"
    );

    // SAFETY: explicit fd close is used to force host `fflush(NULL)` failure.
    let close_status = unsafe { close(failing_fd) };

    assert_eq!(close_status, 0, "closing failure stream fd must succeed");

    write_errno(0);

    // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
    let flush_status = unsafe { fflush(ptr::null_mut()) };

    assert_eq!(flush_status, EOF);
    assert_ne!(read_errno(), 0);

    for (stream_name, stream) in [
      ("stdin", stdin_stream),
      ("stdout", stdout_stream),
      ("stderr", stderr_stream),
    ] {
      assert_eq!(
        host_backed_snapshot_for_tests(stream),
        Some(true),
        "host-backed marker must be retained for {stream_name} on failure",
      );
      assert_eq!(
        buffering_snapshot_for_tests(stream)
          .map(|(_mode, _size, _buffer_addr, io_active)| io_active),
        Some(true),
        "io_active marker must be retained for {stream_name} on failure",
      );
    }

    // SAFETY: even after injected fd close, `fclose` is still needed to release FILE state.
    let _ = unsafe { fclose(failing_stream) };
  }

  #[test]
  fn fflush_stdout_tracks_host_backed_state_without_prior_host_write() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides `stdout` global stream pointer.
    let stdout_stream = unsafe { host_stdout };

    assert!(
      !stdout_stream.is_null(),
      "host stdout pointer must be available"
    );
    assert_eq!(host_backed_snapshot_for_tests(stdout_stream), None);

    write_errno(73);

    // SAFETY: host `stdout` pointer comes from libc and is valid for `fflush`.
    let flush_status = unsafe { fflush(stdout_stream) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 73);
    assert_eq!(host_backed_snapshot_for_tests(stdout_stream), Some(true));
    assert_eq!(
      buffering_snapshot_for_tests(stdout_stream)
        .map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(true),
    );
  }

  #[test]
  fn fflush_stderr_tracks_host_backed_state_without_prior_host_write() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides `stderr` global stream pointer.
    let stderr_stream = unsafe { host_stderr };

    assert!(
      !stderr_stream.is_null(),
      "host stderr pointer must be available"
    );
    assert_eq!(host_backed_snapshot_for_tests(stderr_stream), None);

    write_errno(74);

    // SAFETY: host `stderr` pointer comes from libc and is valid for `fflush`.
    let flush_status = unsafe { fflush(stderr_stream) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 74);
    assert_eq!(host_backed_snapshot_for_tests(stderr_stream), Some(true));
    assert_eq!(
      buffering_snapshot_for_tests(stderr_stream)
        .map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(true),
    );
  }

  #[test]
  fn fflush_stdin_tracks_host_backed_state_without_prior_host_write() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides `stdin` global stream pointer.
    let stdin_stream = unsafe { host_stdin };

    assert!(
      !stdin_stream.is_null(),
      "host stdin pointer must be available"
    );
    assert_eq!(host_backed_snapshot_for_tests(stdin_stream), None);

    write_errno(75);

    // SAFETY: host `stdin` pointer comes from libc and is valid for `fflush`.
    let flush_status = unsafe { fflush(stdin_stream) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 75);
    assert_eq!(host_backed_snapshot_for_tests(stdin_stream), Some(true));
    assert_eq!(
      buffering_snapshot_for_tests(stdin_stream)
        .map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(true),
    );
  }

  #[test]
  fn fflush_non_host_stream_marks_io_active_without_host_backing() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut user_buffer = [0_u8; 8];

    assert_eq!(buffering_snapshot_for_tests(stream), None);
    assert_eq!(host_backed_snapshot_for_tests(stream), None);

    write_errno(82);

    // SAFETY: marker-backed stream pointer is stable for this call.
    let flush_status = unsafe { fflush(stream) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 82);
    assert_eq!(host_backed_snapshot_for_tests(stream), Some(false));
    assert_eq!(
      buffering_snapshot_for_tests(stream).map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(true),
    );

    write_errno(0);

    // SAFETY: stream and buffer pointers are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IONBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(setvbuf_status, EOF);
    assert_eq!(read_errno(), EINVAL);
  }

  #[test]
  fn setvbuf_tracks_configuration_for_new_stream_handle() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut user_buffer = [0_u8; 32];
    let expected_addr = user_buffer.as_mut_ptr().cast::<c_char>().addr();

    assert_eq!(buffering_snapshot_for_tests(stream), None);

    write_errno(44);

    // SAFETY: stream and user buffer are valid for this call.
    let status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(status, 0);
    assert_eq!(read_errno(), 44);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOFBF, user_buffer.len(), expected_addr, false))
    );
  }

  #[test]
  fn setvbuf_replaces_pending_configuration_before_io_activity() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut first_buffer = [0_u8; 8];
    let mut second_buffer = [0_u8; 16];
    let second_addr = second_buffer.as_mut_ptr().cast::<c_char>().addr();

    // SAFETY: stream and user buffer are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        first_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(first_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    // SAFETY: stream and user buffer are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        second_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(second_buffer.len()),
      )
    };

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, second_buffer.len(), second_addr, false))
    );
  }

  #[test]
  fn setvbuf_unbuffered_mode_clears_buffer_tracking_state() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut buffered = [0_u8; 24];
    let mut ignored = [0_u8; 48];

    // SAFETY: stream and user buffer are valid for this call.
    let buffered_status = unsafe {
      setvbuf(
        stream,
        buffered.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(buffered.len()),
      )
    };

    assert_eq!(buffered_status, 0);

    // SAFETY: stream and user buffer are valid for this call.
    let unbuffered_status = unsafe {
      setvbuf(
        stream,
        ignored.as_mut_ptr().cast::<c_char>(),
        _IONBF,
        as_size_t(ignored.len()),
      )
    };

    assert_eq!(unbuffered_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IONBF, 0, 0, false))
    );
  }

  #[test]
  fn setvbuf_keeps_previous_configuration_when_rejected_after_io_activity() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut initial_buffer = [0_u8; 8];
    let mut rejected_buffer = [0_u8; 64];
    let initial_addr = initial_buffer.as_mut_ptr().cast::<c_char>().addr();

    // SAFETY: stream and user buffer are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    // SAFETY: stream pointer is valid for this call.
    let flush_status = unsafe { fflush(stream) };

    assert_eq!(flush_status, 0);

    write_errno(0);

    // SAFETY: stream and user buffer are valid for this call.
    let rejected_status = unsafe {
      setvbuf(
        stream,
        rejected_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(rejected_buffer.len()),
      )
    };

    assert_eq!(rejected_status, EOF);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOFBF, initial_buffer.len(), initial_addr, true))
    );
  }

  #[test]
  fn setvbuf_success_clears_stale_host_backed_flag() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut user_buffer = [0_u8; 8];
    let expected_addr = user_buffer.as_mut_ptr().cast::<c_char>().addr();
    let key = stream_key(stream);
    let mut registry = stream_registry_guard();
    let stream_state = stream_state_mut_or_insert(&mut registry, key);

    stream_state.host_backed_io = true;
    stream_state.io_active = false;

    drop(registry);

    assert_eq!(host_backed_snapshot_for_tests(stream), Some(true));

    // SAFETY: stream and user buffer pointers are valid for this call.
    let status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOFBF, user_buffer.len(), expected_addr, false))
    );
    assert_eq!(host_backed_snapshot_for_tests(stream), Some(false));
  }

  #[test]
  fn setvbuf_rejects_reconfiguration_after_vfprintf_io_activity() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let format = c_string("ok");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IONBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(39);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status =
      unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(write_status, 2);
    assert_eq!(read_errno(), 39);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(second_status, EOF);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IONBF, 0, 0, true))
    );
  }

  #[test]
  fn setvbuf_rejects_reconfiguration_after_vfprintf_host_failure() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let format = c_string("i022-vfprintf-host-failure\n");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IONBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    // SAFETY: `fileno` expects a valid host stream handle.
    let stream_fd = unsafe { fileno(stream) };

    assert!(stream_fd >= 0);

    // SAFETY: explicit fd close is used to force host `vfprintf(stream, ..)` failure.
    let close_status = unsafe { close(stream_fd) };

    assert_eq!(close_status, 0);

    write_errno(0);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status =
      unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

    if write_status == -1 {
      assert_ne!(read_errno(), 0);
    } else {
      assert!(
        write_status >= 0,
        "closed-fd host write attempt must return success count or failure",
      );

      write_errno(0);

      // SAFETY: stream pointer remains valid for delegated host flush attempt.
      let flush_status = unsafe { fflush(stream) };

      assert_eq!(
        flush_status, EOF,
        "successful host write count on closed fd must surface failure on delegated flush",
      );
      assert_ne!(
        read_errno(),
        0,
        "delegated flush failure must set errno on closed-fd stream",
      );
    }

    assert_eq!(host_backed_snapshot_for_tests(stream), Some(true));
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IONBF, 0, 0, true))
    );

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(second_status, EOF);
    assert_eq!(read_errno(), EINVAL);

    // SAFETY: even after injected fd close, `fclose` is still needed to release FILE state.
    let _ = unsafe { fclose(stream) };
  }

  #[test]
  fn setvbuf_allows_reconfiguration_when_host_stream_key_is_reused() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stale_stream = unsafe { tmpfile() };
    // SAFETY: host libc returns either a valid stream pointer or null.
    let fresh_stream = unsafe { tmpfile() };

    assert!(!stale_stream.is_null());
    assert!(!fresh_stream.is_null());

    let mut stale_buffer = [0_u8; 8];
    let mut fresh_buffer = [0_u8; 16];
    let fresh_buffer_addr = fresh_buffer.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("i025-reused");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and user buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stale_stream,
        stale_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(stale_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(41);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status = unsafe {
      vfprintf(
        stale_stream,
        format.as_ptr(),
        ptr::addr_of_mut!(empty_ap).cast(),
      )
    };

    assert_eq!(write_status, 11);
    assert_eq!(read_errno(), 41);
    assert_eq!(host_backed_snapshot_for_tests(stale_stream), Some(true));

    {
      let stale_key = stream_key(stale_stream);
      let fresh_key = stream_key(fresh_stream);

      stream_registry_guard()
        .iter_mut()
        .find(|state| state.stream_key == stale_key)
        .unwrap_or_else(|| unreachable!("stale stream state should be present after vfprintf"))
        .stream_key = fresh_key;
    }

    write_errno(73);

    // SAFETY: stream and user buffer pointers are valid for this call.
    let reconfigure_status = unsafe {
      setvbuf(
        fresh_stream,
        fresh_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(fresh_buffer.len()),
      )
    };

    // SAFETY: stream came from `tmpfile`.
    let stale_close_status = unsafe { fclose(stale_stream) };
    // SAFETY: stream came from `tmpfile`.
    let fresh_close_status = unsafe { fclose(fresh_stream) };

    assert_eq!(stale_close_status, 0);
    assert_eq!(fresh_close_status, 0);
    assert_eq!(reconfigure_status, 0);
    assert_eq!(read_errno(), 73);
    assert_eq!(
      buffering_snapshot_for_tests(fresh_stream),
      Some((_IOFBF, fresh_buffer.len(), fresh_buffer_addr, false))
    );
    assert_eq!(host_backed_snapshot_for_tests(fresh_stream), Some(false));
  }

  #[test]
  fn setvbuf_allows_reconfiguration_when_host_stream_key_is_reused_after_vfprintf_failure() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let path = c_string("/dev/null");
    let mode = c_string("r");
    let format = c_string("%s");
    let payload = c_string("x");
    let mut fresh_buffer = [0_u8; 16];
    let fresh_buffer_addr = fresh_buffer.as_mut_ptr().cast::<c_char>().addr();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stale_stream = unsafe { fopen(path.as_ptr(), mode.as_ptr()) };
    // SAFETY: host libc returns either a valid stream pointer or null.
    let fresh_stream = unsafe { tmpfile() };

    assert!(!stale_stream.is_null());
    assert!(!fresh_stream.is_null());

    // SAFETY: stream pointer is valid and unbuffered mode accepts null buffer.
    let initial_status = unsafe { setvbuf(stale_stream, ptr::null_mut(), _IONBF, 0) };

    assert_eq!(initial_status, 0);

    write_errno(17);

    // SAFETY: stream and format pointers are valid and satisfy `fprintf("%s", payload)`.
    let write_status = unsafe { fprintf(stale_stream, format.as_ptr(), payload.as_ptr()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), 17);

    {
      let stale_key = stream_key(stale_stream);
      let fresh_key = stream_key(fresh_stream);

      stream_registry_guard()
        .iter_mut()
        .find(|state| state.stream_key == stale_key)
        .unwrap_or_else(|| unreachable!("stale stream state should be present after vfprintf"))
        .stream_key = fresh_key;
    }

    write_errno(29);

    // SAFETY: stream and user buffer pointers are valid for this call.
    let reconfigure_status = unsafe {
      setvbuf(
        fresh_stream,
        fresh_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(fresh_buffer.len()),
      )
    };

    // SAFETY: streams came from host allocation APIs.
    let stale_close_status = unsafe { fclose(stale_stream) };
    // SAFETY: stream came from `tmpfile`.
    let fresh_close_status = unsafe { fclose(fresh_stream) };

    assert_eq!(stale_close_status, 0);
    assert_eq!(fresh_close_status, 0);
    assert_eq!(reconfigure_status, 0);
    assert_eq!(read_errno(), 29);
    assert_eq!(
      buffering_snapshot_for_tests(fresh_stream),
      Some((_IOFBF, fresh_buffer.len(), fresh_buffer_addr, false))
    );
    assert_eq!(host_backed_snapshot_for_tests(fresh_stream), Some(false));
  }

  #[test]
  fn setvbuf_allows_reconfiguration_after_vfprintf_null_ap_error() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("%s");

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(0);

    // SAFETY: null va_list pointer intentionally exercises API error contract.
    let write_status = unsafe { vfprintf(stream, format.as_ptr(), ptr::null_mut()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
  }

  #[test]
  fn setvbuf_vfprintf_null_ap_error_keeps_other_stream_reconfigurable() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("%s");

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null va_list pointer intentionally exercises API error contract.
    let write_status = unsafe { vfprintf(stream_a, format.as_ptr(), ptr::null_mut()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn setvbuf_allows_reconfiguration_after_vfprintf_null_format_error() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status = unsafe { vfprintf(stream, ptr::null(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
  }

  #[test]
  fn setvbuf_vfprintf_null_format_error_keeps_other_stream_reconfigurable() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status =
      unsafe { vfprintf(stream_a, ptr::null(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn setvbuf_allows_reconfiguration_after_vfprintf_null_stream_error() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("noop");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(0);

    // SAFETY: null stream pointer intentionally exercises API error contract.
    let write_status = unsafe {
      vfprintf(
        ptr::null_mut(),
        format.as_ptr(),
        ptr::addr_of_mut!(empty_ap).cast(),
      )
    };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
  }

  #[test]
  fn setvbuf_vfprintf_null_stream_error_keeps_other_stream_reconfigurable() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("noop");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null stream pointer intentionally exercises API error contract.
    let write_status = unsafe {
      vfprintf(
        ptr::null_mut(),
        format.as_ptr(),
        ptr::addr_of_mut!(empty_ap).cast(),
      )
    };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn setvbuf_allows_reconfiguration_after_printf_null_format_error() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides `stdout` global stream pointer.
    let stdout_stream = unsafe { host_stdout };

    assert!(
      !stdout_stream.is_null(),
      "host stdout pointer must be available"
    );

    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stdout_stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status = unsafe { printf(ptr::null()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stdout_stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stdout_stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
  }

  #[test]
  fn setvbuf_printf_null_format_error_keeps_other_stream_reconfigurable() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status = unsafe { printf(ptr::null()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn setvbuf_printf_null_format_error_preserves_errno_for_other_stream_reconfiguration() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status = unsafe { printf(ptr::null()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(67);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(read_errno(), 67);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn setvbuf_allows_reconfiguration_after_vprintf_null_format_error() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status = unsafe { vprintf(ptr::null(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
  }

  #[test]
  fn setvbuf_vprintf_null_format_error_keeps_other_stream_reconfigurable() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status = unsafe { vprintf(ptr::null(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn setvbuf_allows_reconfiguration_after_vprintf_null_ap_error() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("%s");

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(0);

    // SAFETY: null va_list pointer intentionally exercises API error contract.
    let write_status = unsafe { vprintf(format.as_ptr(), ptr::null_mut()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
  }

  #[test]
  fn setvbuf_vprintf_null_ap_error_keeps_other_stream_reconfigurable() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("%s");

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null va_list pointer intentionally exercises API error contract.
    let write_status = unsafe { vprintf(format.as_ptr(), ptr::null_mut()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn setvbuf_allows_reconfiguration_after_fprintf_null_stream_error() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("%s");
    let payload = c_string("abc");

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(0);

    // SAFETY: null stream pointer intentionally exercises API error contract.
    let write_status = unsafe { fprintf(ptr::null_mut(), format.as_ptr(), payload.as_ptr()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
  }

  #[test]
  fn setvbuf_fprintf_null_stream_error_keeps_other_stream_reconfigurable() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("%s");
    let payload = c_string("abc");

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null stream pointer intentionally exercises API error contract.
    let write_status = unsafe { fprintf(ptr::null_mut(), format.as_ptr(), payload.as_ptr()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn setvbuf_allows_reconfiguration_after_fprintf_null_format_error() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let payload = c_string("abc");

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(first_status, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status = unsafe { fprintf(stream, ptr::null(), payload.as_ptr()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
  }

  #[test]
  fn setvbuf_fprintf_null_format_error_keeps_other_stream_reconfigurable() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker_a = 0_u8;
    let stream_a = as_file_ptr(&mut marker_a);
    let mut marker_b = 0_u8;
    let stream_b = as_file_ptr(&mut marker_b);
    let mut initial_buffer_a = [0_u8; 8];
    let mut initial_buffer_b = [0_u8; 8];
    let mut replacement_buffer_b = [0_u8; 16];
    let replacement_addr_b = replacement_buffer_b.as_mut_ptr().cast::<c_char>().addr();
    let payload = c_string("abc");

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_a = unsafe {
      setvbuf(
        stream_a,
        initial_buffer_a.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_a.len()),
      )
    };
    // SAFETY: stream and initial buffer pointers are valid for this call.
    let first_status_b = unsafe {
      setvbuf(
        stream_b,
        initial_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer_b.len()),
      )
    };

    assert_eq!(first_status_a, 0);
    assert_eq!(first_status_b, 0);

    write_errno(0);

    // SAFETY: null format pointer intentionally exercises API error contract.
    let write_status = unsafe { fprintf(stream_a, ptr::null(), payload.as_ptr()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let second_status_b = unsafe {
      setvbuf(
        stream_b,
        replacement_buffer_b.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer_b.len()),
      )
    };

    assert_eq!(second_status_b, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream_b),
      Some((
        _IOLBF,
        replacement_buffer_b.len(),
        replacement_addr_b,
        false
      ))
    );
  }

  #[test]
  fn fflush_host_backed_stream_failure_sets_errno_and_keeps_stream_io_active() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut initial_buffer = [0_u8; 8];
    let initial_addr = initial_buffer.as_mut_ptr().cast::<c_char>().addr();
    let mut replacement_buffer = [0_u8; 16];
    let format = c_string("i022");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and initial buffer pointers are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stream,
        initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(initial_buffer.len()),
      )
    };

    assert_eq!(setvbuf_status, 0);

    write_errno(61);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status =
      unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(write_status, 4);
    assert_eq!(read_errno(), 61);

    // SAFETY: `fileno` expects a valid host stream handle.
    let stream_fd = unsafe { fileno(stream) };

    assert!(stream_fd >= 0);

    // SAFETY: explicit fd close is used to force host `fflush(stream)` failure.
    let close_status = unsafe { close(stream_fd) };

    assert_eq!(close_status, 0);

    write_errno(0);

    // SAFETY: host stream pointer is valid for `fflush`.
    let flush_status = unsafe { fflush(stream) };

    assert_eq!(flush_status, EOF);
    assert_ne!(read_errno(), 0);
    assert_eq!(host_backed_snapshot_for_tests(stream), Some(true));
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOFBF, initial_buffer.len(), initial_addr, true))
    );

    write_errno(0);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let rejected_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(rejected_status, EOF);
    assert_eq!(read_errno(), EINVAL);

    // SAFETY: even after injected fd close, `fclose` is still needed to release FILE state.
    let _ = unsafe { fclose(stream) };
  }

  #[test]
  fn fflush_host_backed_stream_failure_does_not_mark_other_stream_io_active() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let failing_stream = unsafe { tmpfile() };

    assert!(!failing_stream.is_null());

    let mut marker = 0_u8;
    let unaffected_stream = as_file_ptr(&mut marker);
    let mut unaffected_initial_buffer = [0_u8; 8];
    let mut unaffected_replacement_buffer = [0_u8; 16];
    let unaffected_replacement_addr = unaffected_replacement_buffer
      .as_mut_ptr()
      .cast::<c_char>()
      .addr();
    let mut failing_initial_buffer = [0_u8; 8];
    let format = c_string("i022-isolation");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and buffer pointers are valid for this call.
    let unaffected_first_status = unsafe {
      setvbuf(
        unaffected_stream,
        unaffected_initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(unaffected_initial_buffer.len()),
      )
    };

    assert_eq!(unaffected_first_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(unaffected_stream),
      Some((
        _IOFBF,
        8,
        unaffected_initial_buffer
          .as_mut_ptr()
          .cast::<c_char>()
          .addr(),
        false
      ))
    );
    assert_eq!(
      host_backed_snapshot_for_tests(unaffected_stream),
      Some(false)
    );

    // SAFETY: stream and buffer pointers are valid for this call.
    let failing_first_status = unsafe {
      setvbuf(
        failing_stream,
        failing_initial_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(failing_initial_buffer.len()),
      )
    };

    assert_eq!(failing_first_status, 0);

    write_errno(33);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status = unsafe {
      vfprintf(
        failing_stream,
        format.as_ptr(),
        ptr::addr_of_mut!(empty_ap).cast(),
      )
    };

    assert_eq!(write_status, 14);
    assert_eq!(read_errno(), 33);

    // SAFETY: `fileno` expects a valid host stream handle.
    let failing_fd = unsafe { fileno(failing_stream) };

    assert!(failing_fd >= 0);

    // SAFETY: explicit fd close is used to force host `fflush(stream)` failure.
    let close_status = unsafe { close(failing_fd) };

    assert_eq!(close_status, 0);

    write_errno(0);

    // SAFETY: host stream pointer is valid for `fflush`.
    let flush_status = unsafe { fflush(failing_stream) };

    assert_eq!(flush_status, EOF);
    assert_ne!(read_errno(), 0);
    assert_eq!(host_backed_snapshot_for_tests(failing_stream), Some(true));

    write_errno(79);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let unaffected_second_status = unsafe {
      setvbuf(
        unaffected_stream,
        unaffected_replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(unaffected_replacement_buffer.len()),
      )
    };

    assert_eq!(unaffected_second_status, 0);
    assert_eq!(read_errno(), 79);
    assert_eq!(
      buffering_snapshot_for_tests(unaffected_stream),
      Some((
        _IOLBF,
        unaffected_replacement_buffer.len(),
        unaffected_replacement_addr,
        false
      ))
    );
    assert_eq!(
      host_backed_snapshot_for_tests(unaffected_stream),
      Some(false)
    );

    // SAFETY: even after injected fd close, `fclose` is still needed to release FILE state.
    let _ = unsafe { fclose(failing_stream) };
  }
}
