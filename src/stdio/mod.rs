//! C stdio buffering and formatting interfaces.
//!
//! This module currently provides:
//! - a minimal `FILE` registry used to track stream usage for `setbuf`/
//!   `setbuffer`/`setlinebuf`/`setvbuf`
//! - `fclose` host delegation with stream-registry cleanup on success
//! - `fflush` for registered streams and `fflush(NULL)`
//! - `tmpfile`, `fopen`, `fread`, `fputs`
//! - `fileno` / `fileno_unlocked` delegation with standard-stream fast paths
//! - `setbuf`/`setbuffer`/`setlinebuf` wrappers plus a minimal `setvbuf`
//!   entry point with mode/size validation and per-stream buffering
//!   configuration tracking
//! - per-`FILE` lock scaffolding for `flockfile`/`ftrylockfile`/`funlockfile`
//! - an incremental `vsnprintf` subset (`%%`, `%s`, `%c`, `%p`, `%d/%i/%u/%x/%X/%o`)
//! - C ABI wrappers for `printf`/`fprintf`/`vprintf`/`vfprintf`

use crate::abi::errno::{EBUSY, EINVAL};
use crate::abi::types::{c_int, size_t};
use crate::errno::{__errno_location, set_errno};
use core::ffi::{c_char, c_void};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Error as IoError;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::{self, ThreadId};

/// C stdio `EOF` status code.
pub const EOF: c_int = -1;
/// Fully buffered mode (`setvbuf`).
pub const _IOFBF: c_int = 0;
/// Line buffered mode (`setvbuf`).
pub const _IOLBF: c_int = 1;
/// Unbuffered mode (`setvbuf`).
pub const _IONBF: c_int = 2;
/// Default stdio buffer size used by `setbuf`-family compatibility wrappers.
pub const BUFSIZ: size_t = 8192;
const STDOUT_FILENO: c_int = 1;
const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
const FCLOSE_SYMBOL_NAME: &[u8] = b"fclose\0";
const FFLUSH_SYMBOL_NAME: &[u8] = b"fflush\0";
const FILENO_SYMBOL_NAME: &[u8] = b"fileno\0";
const FOPEN_SYMBOL_NAME: &[u8] = b"fopen\0";
const FPUTS_SYMBOL_NAME: &[u8] = b"fputs\0";
const FREAD_SYMBOL_NAME: &[u8] = b"fread\0";
const TMPFILE_SYMBOL_NAME: &[u8] = b"tmpfile\0";
const VFPRINTF_SYMBOL_NAME: &[u8] = b"vfprintf\0";
const GLIBC_DLSYM_VERSION_CANDIDATES: [&[u8]; 2] = [b"GLIBC_2.34\0", b"GLIBC_2.2.5\0"];

/// Opaque C `FILE` handle type used by stdio entry points.
#[repr(C)]
pub struct File {
  _private: [u8; 0],
}

/// Public C ABI type alias for stdio stream handles.
pub type FILE = File;

#[repr(C)]
struct GlibcFilePrefix {
  _flags: c_int,
  _io_read_ptr: *mut c_char,
  _io_read_end: *mut c_char,
  _io_read_base: *mut c_char,
  _io_write_base: *mut c_char,
  _io_write_ptr: *mut c_char,
  _io_write_end: *mut c_char,
  _io_buf_base: *mut c_char,
  _io_buf_end: *mut c_char,
  _io_save_base: *mut c_char,
  _io_backup_base: *mut c_char,
  _io_save_end: *mut c_char,
  _markers: *mut c_void,
  _chain: *mut c_void,
  fileno: c_int,
}

struct StreamState {
  stream_key: usize,
  buffering_mode: c_int,
  buffer_size: usize,
  user_buffer_addr: usize,
  explicit_buffering_config: bool,
  io_active: bool,
  host_backed_io: bool,
  host_stream_identity: Option<u64>,
  file_lock_owner: Option<ThreadId>,
  file_lock_depth: usize,
}

struct StreamFileLockGuard {
  stream: *mut FILE,
  locked: bool,
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
  fp_offset: u32,
  reg_save_area: *const u8,
  overflow_arg_area: *const u64,
}

type HostVfprintfFn =
  unsafe extern "C" fn(stream: *mut FILE, format: *const c_char, ap: *mut c_void) -> c_int;

type HostFcloseFn = unsafe extern "C" fn(*mut FILE) -> c_int;

type HostFflushFn = unsafe extern "C" fn(*mut FILE) -> c_int;

type HostFilenoFn = unsafe extern "C" fn(*mut FILE) -> c_int;

type HostFopenFn = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut FILE;

type HostFputsFn = unsafe extern "C" fn(*const c_char, *mut FILE) -> c_int;

type HostFreadFn = unsafe extern "C" fn(*mut c_void, size_t, size_t, *mut FILE) -> size_t;

type HostTmpfileFn = unsafe extern "C" fn() -> *mut FILE;

#[cfg(test)]
static HOST_VFPRINTF_UNAVAILABLE_FOR_TESTS: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static HOST_FFLUSH_UNAVAILABLE_FOR_TESTS: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static HOST_FILENO_UNAVAILABLE_FOR_TESTS: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static HOST_STDOUT_UNAVAILABLE_FOR_TESTS: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static HOST_FFLUSH_FORCED_FAILURE_ERRNO_FOR_TESTS: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum VfprintfWritePath {
  NoStreamIo,
  InternalStreamIo,
  HostDelegated,
}

impl StreamFileLockGuard {
  fn acquire(stream: *mut FILE) -> Self {
    let locked = lock_stream_file_internal(stream, false) == 0;

    Self { stream, locked }
  }

  const fn disarm(&mut self) {
    self.locked = false;
  }
}

impl Drop for StreamFileLockGuard {
  fn drop(&mut self) {
    if !self.locked {
      return;
    }

    // SAFETY: this guard only unlocks a stream it locked through the same API.
    unsafe {
      funlockfile(self.stream);
    }
  }
}

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
        fp_offset: 176,
        reg_save_area: core::ptr::null(),
        overflow_arg_area: core::ptr::null(),
      };
    }

    // SAFETY: caller must pass a valid SysV va_list pointer.
    let va_list = unsafe { &*ap.cast::<SysVVaList>() };

    Self {
      gp_offset: va_list.gp_offset,
      fp_offset: va_list.fp_offset,
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

  const fn skip_long_double(&mut self) -> Result<(), ()> {
    if self.overflow_arg_area.is_null() {
      return Err(());
    }

    // SAFETY: SysV passes `long double` varargs in the overflow area; reading
    // the two eight-byte halves validates availability before advancing.
    unsafe {
      let _ = self.overflow_arg_area.read_unaligned();
      let _ = self.overflow_arg_area.add(1).read_unaligned();

      self.overflow_arg_area = self.overflow_arg_area.add(2);
    }

    Ok(())
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

fn stream_registry_wait_cv() -> &'static Condvar {
  static WAIT_CV: OnceLock<Condvar> = OnceLock::new();

  WAIT_CV.get_or_init(Condvar::new)
}

fn wait_for_stream_registry_change(
  registry: MutexGuard<'static, Vec<StreamState>>,
) -> MutexGuard<'static, Vec<StreamState>> {
  match stream_registry_wait_cv().wait(registry) {
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
    file_lock_owner: None,
    file_lock_depth: 0,
  });

  registry
    .last_mut()
    .unwrap_or_else(|| unreachable!("stream state was just inserted"))
}

const fn stream_state_is_prunable(stream_state: &StreamState) -> bool {
  stream_state.buffering_mode == _IONBF
    && stream_state.buffer_size == 0
    && stream_state.user_buffer_addr == 0
    && !stream_state.explicit_buffering_config
    && !stream_state.io_active
    && !stream_state.host_backed_io
    && stream_state.host_stream_identity.is_none()
    && stream_state.file_lock_owner.is_none()
    && stream_state.file_lock_depth == 0
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

fn host_standard_stream_fd(stream: *mut FILE) -> Option<c_int> {
  // SAFETY: reading host libc standard stream pointers for pointer comparison only.
  let (stdin, stdout, stderr) = unsafe { (host_stdin, host_stdout, host_stderr) };

  if stream == stdin {
    return Some(0);
  }

  if stream == stdout {
    return Some(1);
  }

  if stream == stderr {
    return Some(2);
  }

  None
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
      stream_state
        .explicit_buffering_config
        .then_some(stream_state.buffering_mode)
    });

  drop(registry);

  explicit_mode
}

fn stream_can_use_internal_vfprintf_path(stream: *mut FILE) -> bool {
  let key = stream_key(stream);
  let registry = stream_registry_guard();
  let can_use_internal_path = registry
    .iter()
    .find(|stream_state| stream_state.stream_key == key)
    .is_some_and(|stream_state| stream_state.io_active && !stream_state.host_backed_io);

  drop(registry);

  can_use_internal_path
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

const fn is_float_conversion(conversion: u8) -> bool {
  matches!(
    conversion,
    b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A'
  )
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
  let is_float = is_float_conversion(conversion);
  let is_pointer = is_pointer_conversion(conversion);
  let is_count = is_count_conversion(conversion);

  if !is_string_or_char
    && !is_integer_conversion(conversion)
    && !is_float
    && !is_pointer
    && !is_count
  {
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
  } else if is_float {
    if length != LengthModifier::Default && length != LengthModifier::L {
      return None;
    }
  } else if !is_count
    && ((flags & (FormatDirective::FORCE_SIGN | FormatDirective::LEADING_SPACE_FOR_POSITIVE)) != 0
      && conversion != b'd'
      && conversion != b'i')
    || (!is_count
      && (flags & FormatDirective::ALTERNATE_FORM) != 0
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
  let digits = if precision == Some(0) && value == 0 {
    Vec::new()
  } else {
    unsigned_to_ascii(value, base, uppercase)
  };
  let octal_alternate_with_explicit_precision =
    directive.conversion == b'o' && directive.alternate_form() && value != 0 && precision.is_some();
  let octal_alternate_prefix = if directive.conversion == b'o'
    && directive.alternate_form()
    && ((value == 0 && precision == Some(0)) || (value != 0 && precision.is_none()))
  {
    b"0".as_slice()
  } else {
    b"".as_slice()
  };

  if octal_alternate_with_explicit_precision {
    let forced_digits = digits.len().saturating_add(1);

    effective_precision = Some(effective_precision.unwrap_or(0).max(forced_digits));
  }

  let prefix: &[u8] = if directive.alternate_form() && value != 0 {
    match directive.conversion {
      b'o' => octal_alternate_prefix,
      b'x' => b"0x",
      b'X' => b"0X",
      _ => b"",
    }
  } else {
    octal_alternate_prefix
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

const fn internal_vfprintf_supports_directive(directive: FormatDirective) -> bool {
  matches!(
    directive.conversion,
    b's' | b'c' | b'p' | b'n' | b'd' | b'i' | b'u' | b'x' | b'X' | b'o'
  ) && !(directive.conversion == b'c' && directive.precision.is_some())
}

const fn line_buffer_newline_analysis_supports_directive(directive: FormatDirective) -> bool {
  internal_vfprintf_supports_directive(directive) || is_float_conversion(directive.conversion)
}

fn advance_line_buffer_float_argument(
  arg_cursor: &mut VarArgCursor,
  length: LengthModifier,
) -> Result<(), ()> {
  match length {
    // Newline analysis does not need the rendered `double` payload. Advancing
    // the FP cursor without reading keeps later argument positions aligned
    // while avoiding dependence on the saved FP register bytes.
    LengthModifier::Default => {
      if arg_cursor.fp_offset < 176 && !arg_cursor.reg_save_area.is_null() {
        arg_cursor.fp_offset = arg_cursor.fp_offset.saturating_add(16);
      } else if !arg_cursor.overflow_arg_area.is_null() {
        // SAFETY: advancing by one u64 slot consumes one default-promoted `double`.
        arg_cursor.overflow_arg_area = unsafe { arg_cursor.overflow_arg_area.add(1) };
      } else {
        return Err(());
      }
    }
    LengthModifier::L => {
      arg_cursor.skip_long_double()?;
    }
    LengthModifier::Hh
    | LengthModifier::H
    | LengthModifier::Ll
    | LengthModifier::J
    | LengthModifier::Z
    | LengthModifier::T => {
      return Err(());
    }
  }

  Ok(())
}

unsafe fn formatted_output_requires_host_line_buffer_flush(
  format: *const c_char,
  ap: *mut c_void,
) -> bool {
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
      return true;
    };

    if !line_buffer_newline_analysis_supports_directive(directive) {
      return true;
    }

    let mut left_align = directive.left_align();

    if resolve_width(directive.width, &mut arg_cursor, &mut left_align).is_err() {
      return true;
    }

    let Ok(precision) = resolve_precision(directive.precision, &mut arg_cursor) else {
      return true;
    };

    match directive.conversion {
      b's' => {
        let Ok(source) = arg_cursor.next_ptr::<c_char>() else {
          return true;
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
          return true;
        };
        let low = u32::from_ne_bytes(value.to_ne_bytes()) & u32::from(u8::MAX);
        let emitted = u8::try_from(low).unwrap_or_else(|_| unreachable!("masked to 8 bits"));

        if emitted == b'\n' {
          return true;
        }
      }
      b'd' | b'i' => {
        if read_signed_argument(&mut arg_cursor, directive.length).is_err() {
          return true;
        }
      }
      b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => {
        if advance_line_buffer_float_argument(&mut arg_cursor, directive.length).is_err() {
          return true;
        }
      }
      b'u' | b'x' | b'X' | b'o' => {
        if read_unsigned_argument(&mut arg_cursor, directive.length).is_err() {
          return true;
        }
      }
      b'p' => {
        if arg_cursor.next_ptr::<c_void>().is_err() {
          return true;
        }
      }
      b'n' => {
        if consume_count_conversion_argument(&mut arg_cursor, directive.length).is_err() {
          return true;
        }
      }
      _ => unreachable!("newline analysis only reaches internal formatter directives"),
    }

    index = directive.next_index;
  }

  false
}

unsafe fn internal_vfprintf_supports_format(format: *const c_char) -> bool {
  if format.is_null() {
    return false;
  }

  // SAFETY: caller guarantees `format` points to a readable NUL-terminated C string.
  let format_len = unsafe { c_string_len(format) };
  // SAFETY: `format_len` was measured from `format` above.
  let format_bytes = unsafe { core::slice::from_raw_parts(format.cast::<u8>(), format_len) };
  let mut index = 0_usize;

  while let Some(byte) = format_bytes.get(index).copied() {
    if byte != b'%' {
      index += 1;
      continue;
    }

    if index + 1 == format_bytes.len() {
      return false;
    }

    if format_bytes[index + 1] == b'%' {
      index += 2;
      continue;
    }

    let Some(directive) = parse_format_directive(format_bytes, index + 1) else {
      return false;
    };

    if !internal_vfprintf_supports_directive(directive) {
      return false;
    }

    index = directive.next_index;
  }

  true
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
  #[link_name = "dlvsym"]
  fn host_dlvsym(handle: *mut c_void, symbol: *const c_char, version: *const c_char)
  -> *mut c_void;
  fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

unsafe extern "C" {
  #[link_name = "tmpfile"]
  fn host_tmpfile_ffi() -> *mut FILE;
  #[link_name = "fopen"]
  fn host_fopen_ffi(path: *const c_char, mode: *const c_char) -> *mut FILE;
  #[link_name = "fread"]
  fn host_fread_ffi(ptr: *mut c_void, size: size_t, nmemb: size_t, stream: *mut FILE) -> size_t;
  #[link_name = "fputs"]
  fn host_fputs_ffi(s: *const c_char, stream: *mut FILE) -> c_int;
  #[link_name = "stdin"]
  static mut host_stdin: *mut FILE;
  #[link_name = "stdout"]
  static mut host_stdout: *mut FILE;
  #[link_name = "stderr"]
  static mut host_stderr: *mut FILE;
}

unsafe fn host_fileno(stream: *mut FILE) -> c_int {
  #[cfg(test)]
  if HOST_FILENO_UNAVAILABLE_FOR_TESTS.load(Ordering::Relaxed) {
    set_errno(EINVAL);

    return -1;
  }

  let _resolved_host_fileno = host_fileno_fn();

  // SAFETY: on Linux/glibc `FILE*` begins with `_IO_FILE`, whose `_fileno`
  // field is stable for a live host-backed stream handle.
  let descriptor = unsafe { (*stream.cast::<GlibcFilePrefix>()).fileno };

  if descriptor < 0 {
    set_errno(EINVAL);
  }

  descriptor
}

fn host_stdout_stream() -> *mut FILE {
  #[cfg(test)]
  if HOST_STDOUT_UNAVAILABLE_FOR_TESTS.load(Ordering::Relaxed) {
    return core::ptr::null_mut();
  }

  // SAFETY: reading host-provided global `stdout` pointer.
  unsafe { host_stdout }
}

fn is_local_vfprintf_symbol(symbol_ptr: *mut c_void) -> bool {
  let local_symbol =
    vfprintf as unsafe extern "C" fn(*mut FILE, *const c_char, *mut c_void) -> c_int;

  symbol_ptr == local_symbol as *const () as *mut c_void
}

fn fail_vsnprintf_with_einval(sink: &mut OutputSink) -> c_int {
  set_errno(EINVAL);
  sink.terminate();

  -1
}

fn resolve_host_vfprintf() -> Option<HostVfprintfFn> {
  let versioned_symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        VFPRINTF_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  });
  let symbol_ptr = versioned_symbol_ptr.unwrap_or_else(|| {
    // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
    unsafe { dlsym(RTLD_NEXT, VFPRINTF_SYMBOL_NAME.as_ptr().cast()) }
  });

  if symbol_ptr.is_null() || is_local_vfprintf_symbol(symbol_ptr) {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `vfprintf`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostVfprintfFn>(symbol_ptr) })
}

fn resolve_host_fflush() -> Option<HostFflushFn> {
  let versioned_symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FFLUSH_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  });
  let symbol_ptr = versioned_symbol_ptr.unwrap_or_else(|| {
    // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
    unsafe { dlsym(RTLD_NEXT, FFLUSH_SYMBOL_NAME.as_ptr().cast()) }
  });

  if symbol_ptr.is_null() {
    return None;
  }

  let local_symbol = fflush as unsafe extern "C" fn(*mut FILE) -> c_int;

  if symbol_ptr == local_symbol as *const () as *mut c_void {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fflush`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFflushFn>(symbol_ptr) })
}

fn resolve_host_fclose() -> Option<HostFcloseFn> {
  let versioned_symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FCLOSE_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  });
  let symbol_ptr = versioned_symbol_ptr.unwrap_or_else(|| {
    // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
    unsafe { dlsym(RTLD_NEXT, FCLOSE_SYMBOL_NAME.as_ptr().cast()) }
  });

  if symbol_ptr.is_null() {
    return None;
  }

  let local_symbol = fclose as unsafe extern "C" fn(*mut FILE) -> c_int;

  if symbol_ptr == local_symbol as *const () as *mut c_void {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fclose`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFcloseFn>(symbol_ptr) })
}

fn resolve_host_fileno() -> Option<HostFilenoFn> {
  let symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FILENO_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  })?;

  if symbol_ptr == (fileno as unsafe extern "C" fn(*mut FILE) -> c_int) as *const () as *mut c_void
  {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fileno_unlocked`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFilenoFn>(symbol_ptr) })
}

fn resolve_host_tmpfile() -> Option<HostTmpfileFn> {
  let versioned_symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        TMPFILE_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  });
  let symbol_ptr = versioned_symbol_ptr.unwrap_or_else(|| {
    // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
    unsafe { dlsym(RTLD_NEXT, TMPFILE_SYMBOL_NAME.as_ptr().cast()) }
  });

  if symbol_ptr.is_null() {
    return None;
  }

  if symbol_ptr == (tmpfile as unsafe extern "C" fn() -> *mut FILE) as *const () as *mut c_void {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `tmpfile`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostTmpfileFn>(symbol_ptr) })
}

fn resolve_host_fopen() -> Option<HostFopenFn> {
  let versioned_symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FOPEN_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  });
  let symbol_ptr = versioned_symbol_ptr.unwrap_or_else(|| {
    // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
    unsafe { dlsym(RTLD_NEXT, FOPEN_SYMBOL_NAME.as_ptr().cast()) }
  });

  if symbol_ptr.is_null() {
    return None;
  }

  if symbol_ptr
    == (fopen as unsafe extern "C" fn(*const c_char, *const c_char) -> *mut FILE) as *const ()
      as *mut c_void
  {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fopen`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFopenFn>(symbol_ptr) })
}

fn resolve_host_fputs() -> Option<HostFputsFn> {
  let versioned_symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FPUTS_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  });
  let symbol_ptr = versioned_symbol_ptr.unwrap_or_else(|| {
    // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
    unsafe { dlsym(RTLD_NEXT, FPUTS_SYMBOL_NAME.as_ptr().cast()) }
  });

  if symbol_ptr.is_null() {
    return None;
  }

  if symbol_ptr
    == (fputs as unsafe extern "C" fn(*const c_char, *mut FILE) -> c_int) as *const ()
      as *mut c_void
  {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fputs`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFputsFn>(symbol_ptr) })
}

fn resolve_host_fread() -> Option<HostFreadFn> {
  let versioned_symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FREAD_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  });
  let symbol_ptr = versioned_symbol_ptr.unwrap_or_else(|| {
    // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
    unsafe { dlsym(RTLD_NEXT, FREAD_SYMBOL_NAME.as_ptr().cast()) }
  });

  if symbol_ptr.is_null() {
    return None;
  }

  if symbol_ptr
    == (fread as unsafe extern "C" fn(*mut c_void, size_t, size_t, *mut FILE) -> size_t)
      as *const () as *mut c_void
  {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fread`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFreadFn>(symbol_ptr) })
}

#[cfg(test)]
fn set_host_vfprintf_unavailable_for_tests(unavailable: bool) {
  HOST_VFPRINTF_UNAVAILABLE_FOR_TESTS.store(unavailable, Ordering::Relaxed);
}

#[cfg(test)]
fn set_host_fflush_unavailable_for_tests(unavailable: bool) {
  HOST_FFLUSH_UNAVAILABLE_FOR_TESTS.store(unavailable, Ordering::Relaxed);
}

#[cfg(test)]
fn set_host_fileno_unavailable_for_tests(unavailable: bool) {
  HOST_FILENO_UNAVAILABLE_FOR_TESTS.store(unavailable, Ordering::Relaxed);
}

#[cfg(test)]
fn set_host_stdout_unavailable_for_tests(unavailable: bool) {
  HOST_STDOUT_UNAVAILABLE_FOR_TESTS.store(unavailable, Ordering::Relaxed);
}

#[cfg(test)]
fn set_host_fflush_forced_failure_errno_for_tests(errno: c_int) {
  HOST_FFLUSH_FORCED_FAILURE_ERRNO_FOR_TESTS.store(errno, Ordering::Relaxed);
}

#[cfg(test)]
fn host_fflush_forced_failure_errno_for_tests() -> Option<c_int> {
  let errno = HOST_FFLUSH_FORCED_FAILURE_ERRNO_FOR_TESTS.load(Ordering::Relaxed);

  (errno != 0).then_some(errno)
}

fn host_vfprintf() -> Option<HostVfprintfFn> {
  static HOST_VFPRINTF: OnceLock<Option<HostVfprintfFn>> = OnceLock::new();

  #[cfg(test)]
  if HOST_VFPRINTF_UNAVAILABLE_FOR_TESTS.load(Ordering::Relaxed) {
    return None;
  }

  *HOST_VFPRINTF.get_or_init(resolve_host_vfprintf)
}

fn host_fflush_fn() -> Option<HostFflushFn> {
  static HOST_FFLUSH: OnceLock<Option<HostFflushFn>> = OnceLock::new();

  #[cfg(test)]
  if HOST_FFLUSH_UNAVAILABLE_FOR_TESTS.load(Ordering::Relaxed) {
    return None;
  }

  *HOST_FFLUSH.get_or_init(resolve_host_fflush)
}

fn host_fclose_fn() -> Option<HostFcloseFn> {
  static HOST_FCLOSE: OnceLock<Option<HostFcloseFn>> = OnceLock::new();

  *HOST_FCLOSE.get_or_init(resolve_host_fclose)
}

fn host_fileno_fn() -> Option<HostFilenoFn> {
  static HOST_FILENO: OnceLock<Option<HostFilenoFn>> = OnceLock::new();

  #[cfg(test)]
  if HOST_FILENO_UNAVAILABLE_FOR_TESTS.load(Ordering::Relaxed) {
    return None;
  }

  *HOST_FILENO.get_or_init(resolve_host_fileno)
}

fn resolve_host_tmpfile() -> Option<HostTmpfileFn> {
  let symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        TMPFILE_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  })?;

  if symbol_ptr == (tmpfile as unsafe extern "C" fn() -> *mut FILE) as *const () as *mut c_void {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `tmpfile`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostTmpfileFn>(symbol_ptr) })
}

fn resolve_host_fopen() -> Option<HostFopenFn> {
  let symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FOPEN_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  })?;

  if symbol_ptr
    == (fopen as unsafe extern "C" fn(*const c_char, *const c_char) -> *mut FILE) as *const ()
      as *mut c_void
  {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fopen`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFopenFn>(symbol_ptr) })
}

fn resolve_host_fread() -> Option<HostFreadFn> {
  let symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FREAD_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  })?;

  if symbol_ptr
    == (fread as unsafe extern "C" fn(*mut c_void, size_t, size_t, *mut FILE) -> size_t)
      as *const () as *mut c_void
  {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fread`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFreadFn>(symbol_ptr) })
}

fn resolve_host_fputs() -> Option<HostFputsFn> {
  let symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol and version are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        FPUTS_SYMBOL_NAME.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  })?;

  if symbol_ptr
    == (fputs as unsafe extern "C" fn(*const c_char, *mut FILE) -> c_int) as *const ()
      as *mut c_void
  {
    return None;
  }

  // SAFETY: `symbol_ptr` resolves to host libc's `fputs`.
  Some(unsafe { core::mem::transmute::<*mut c_void, HostFputsFn>(symbol_ptr) })
}

fn host_tmpfile_fn() -> Option<HostTmpfileFn> {
  static HOST_TMPFILE: OnceLock<Option<HostTmpfileFn>> = OnceLock::new();

  *HOST_TMPFILE.get_or_init(resolve_host_tmpfile)
}

fn host_fopen_fn() -> Option<HostFopenFn> {
  static HOST_FOPEN: OnceLock<Option<HostFopenFn>> = OnceLock::new();

  *HOST_FOPEN.get_or_init(resolve_host_fopen)
}

fn host_fread_fn() -> Option<HostFreadFn> {
  static HOST_FREAD: OnceLock<Option<HostFreadFn>> = OnceLock::new();

  *HOST_FREAD.get_or_init(resolve_host_fread)
}

fn host_fputs_fn() -> Option<HostFputsFn> {
  static HOST_FPUTS: OnceLock<Option<HostFputsFn>> = OnceLock::new();

  *HOST_FPUTS.get_or_init(resolve_host_fputs)
}

fn set_errno_from_host_flush_failure() {
  // SAFETY: `__errno_location` returns readable thread-local errno storage.
  let observed_errno = unsafe { __errno_location().read() };
  let host_errno = (observed_errno > 0)
    .then_some(observed_errno)
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
) -> (c_int, VfprintfWritePath) {
  let has_explicit_buffering = stream_explicit_buffering_mode(stream).is_some();
  let can_use_internal_path = stream_can_use_internal_vfprintf_path(stream);

  if !has_explicit_buffering
    && can_use_internal_path
    && unsafe { internal_vfprintf_supports_format(format) }
  {
    // SAFETY: format support check succeeded for the internal formatter.
    return unsafe { forward_internal_vfprintf(stream, format, ap) };
  }

  if let Some(host_vfprintf) = host_vfprintf() {
    // SAFETY: caller upholds host `vfprintf` contracts for stream/format/va_list.
    return (
      unsafe { host_vfprintf(stream, format, ap) },
      VfprintfWritePath::HostDelegated,
    );
  }

  // SAFETY: pointers were pre-validated by `vfprintf`.
  unsafe { forward_internal_vfprintf(stream, format, ap) }
}

unsafe fn forward_internal_vfprintf(
  stream: *mut FILE,
  format: *const c_char,
  ap: *mut c_void,
) -> (c_int, VfprintfWritePath) {
  // SAFETY: pointers were pre-validated by `vfprintf` and `vsnprintf` only reads `ap`.
  let required_len = unsafe { vsnprintf(core::ptr::null_mut(), 0, format, ap) };

  if required_len < 0 {
    return (-1, VfprintfWritePath::NoStreamIo);
  }

  let rendered_capacity = usize::try_from(required_len)
    .unwrap_or_else(|_| unreachable!("non-negative `c_int` length must fit `usize`"))
    .saturating_add(1);
  let mut rendered = vec![0_u8; rendered_capacity];
  let rendered_capacity_size_t = size_t::try_from(rendered_capacity)
    .unwrap_or_else(|_| unreachable!("`usize` must fit `size_t` on x86_64"));

  // SAFETY: `rendered` points to writable memory and `format`/`ap` were validated above.
  let rendered_len = unsafe {
    vsnprintf(
      rendered.as_mut_ptr().cast::<c_char>(),
      rendered_capacity_size_t,
      format,
      ap,
    )
  };

  if rendered_len < 0 {
    return (-1, VfprintfWritePath::NoStreamIo);
  }

  let rendered_len_usize = usize::try_from(rendered_len)
    .unwrap_or_else(|_| unreachable!("non-negative `c_int` length must fit `usize`"));
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let stream_fd = if let Some(standard_stream_fd) = host_standard_stream_fd(stream) {
    standard_stream_fd
  } else {
    // SAFETY: clear stale errno before delegated host stream descriptor lookup.
    unsafe {
      __errno_location().write(0);
    }

    // SAFETY: `stream` was validated by `vfprintf`.
    let resolved_fd = unsafe { host_fileno(stream) };

    if resolved_fd < 0 {
      // SAFETY: read current errno produced by host `fileno`.
      let observed_errno = unsafe { __errno_location().read() };

      if observed_errno <= 0 {
        set_errno(EINVAL);
      }

      return (-1, VfprintfWritePath::InternalStreamIo);
    }

    resolved_fd
  };
  let mut written_offset = 0_usize;

  while written_offset < rendered_len_usize {
    let remaining_len = rendered_len_usize - written_offset;
    let write_len = size_t::try_from(remaining_len)
      .unwrap_or_else(|_| unreachable!("`usize` must fit `size_t` on x86_64"));
    // SAFETY: pointer/length pair targets readable rendered bytes.
    let written = unsafe {
      crate::unistd::write(
        stream_fd,
        rendered.as_ptr().add(written_offset).cast::<c_void>(),
        write_len,
      )
    };

    if written < 0 {
      return (-1, VfprintfWritePath::InternalStreamIo);
    }

    if written == 0 {
      set_errno(EINVAL);

      return (-1, VfprintfWritePath::InternalStreamIo);
    }

    let written_len = usize::try_from(written)
      .unwrap_or_else(|_| unreachable!("positive `ssize_t` must fit `usize`"));

    written_offset = written_offset.saturating_add(written_len);
  }

  // SAFETY: preserve caller-observed errno on successful internal fallback write.
  unsafe {
    __errno_location().write(errno_before);
  }

  (rendered_len, VfprintfWritePath::InternalStreamIo)
}

unsafe fn forward_internal_vprintf_stdout(format: *const c_char, ap: *mut c_void) -> c_int {
  // SAFETY: caller validates `format`/`ap` pointers and `vsnprintf` only reads
  // the va_list payload.
  let required_len = unsafe { vsnprintf(core::ptr::null_mut(), 0, format, ap) };

  if required_len < 0 {
    return -1;
  }

  let rendered_capacity = usize::try_from(required_len)
    .unwrap_or_else(|_| unreachable!("non-negative `c_int` length must fit `usize`"))
    .saturating_add(1);
  let mut rendered = vec![0_u8; rendered_capacity];
  let rendered_capacity_size_t = size_t::try_from(rendered_capacity)
    .unwrap_or_else(|_| unreachable!("`usize` must fit `size_t` on x86_64"));

  // SAFETY: `rendered` points to writable memory and `format`/`ap` were validated above.
  let rendered_len = unsafe {
    vsnprintf(
      rendered.as_mut_ptr().cast::<c_char>(),
      rendered_capacity_size_t,
      format,
      ap,
    )
  };

  if rendered_len < 0 {
    return -1;
  }

  let rendered_len_usize = usize::try_from(rendered_len)
    .unwrap_or_else(|_| unreachable!("non-negative `c_int` length must fit `usize`"));
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let mut written_offset = 0_usize;

  while written_offset < rendered_len_usize {
    let remaining_len = rendered_len_usize - written_offset;
    let write_len = size_t::try_from(remaining_len)
      .unwrap_or_else(|_| unreachable!("`usize` must fit `size_t` on x86_64"));
    // SAFETY: pointer/length pair targets readable rendered bytes.
    let written = unsafe {
      crate::unistd::write(
        STDOUT_FILENO,
        rendered.as_ptr().add(written_offset).cast::<c_void>(),
        write_len,
      )
    };

    if written < 0 {
      return -1;
    }

    if written == 0 {
      set_errno(EINVAL);

      return -1;
    }

    let written_len = usize::try_from(written)
      .unwrap_or_else(|_| unreachable!("positive `ssize_t` must fit `usize`"));

    written_offset = written_offset.saturating_add(written_len);
  }

  // SAFETY: preserve caller-observed errno on successful internal stdout write.
  unsafe {
    __errno_location().write(errno_before);
  }

  rendered_len
}

fn clear_stream_tracking_state(stream: *mut FILE) {
  let key = stream_key(stream);
  let removed = {
    let mut registry = stream_registry_guard();
    let len_before = registry.len();

    registry.retain(|stream_state| stream_state.stream_key != key);

    len_before != registry.len()
  };

  if removed {
    stream_registry_wait_cv().notify_all();
  }
}

fn lock_stream_file_internal(stream: *mut FILE, try_only: bool) -> c_int {
  if stream.is_null() {
    set_errno(EINVAL);

    return EINVAL;
  }

  let key = stream_key(stream);
  let current = thread::current().id();
  let mut registry = stream_registry_guard();

  loop {
    let stream_state = stream_state_mut_or_insert(&mut registry, key);

    match stream_state.file_lock_owner {
      None => {
        stream_state.file_lock_owner = Some(current);
        stream_state.file_lock_depth = 1;

        return 0;
      }
      Some(owner) if owner == current => {
        stream_state.file_lock_depth = stream_state.file_lock_depth.saturating_add(1);

        return 0;
      }
      Some(_) if try_only => return EBUSY,
      Some(_) => registry = wait_for_stream_registry_change(registry),
    }
  }
}

/// C ABI entry point for `flockfile`.
///
/// Behavior:
/// - acquires a per-stream recursive lock for `stream`
/// - blocks until another owning thread releases the lock
/// - same-thread reentry increases the tracked recursive depth
///
/// # Safety
/// - `stream` must be a valid `FILE*` when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flockfile(stream: *mut FILE) {
  let _ = lock_stream_file_internal(stream, false);
}

/// C ABI entry point for `ftrylockfile`.
///
/// Behavior:
/// - acquires the per-stream recursive lock immediately when available
/// - same-thread reentry succeeds and increments the tracked recursive depth
/// - returns [`EBUSY`] when another thread currently owns the lock
///
/// Returns:
/// - `0` on success
/// - [`EBUSY`] when the lock is currently owned by another thread
/// - [`EINVAL`] when `stream` is null
///
/// # Safety
/// - `stream` must be a valid `FILE*` when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftrylockfile(stream: *mut FILE) -> c_int {
  lock_stream_file_internal(stream, true)
}

/// C ABI entry point for `funlockfile`.
///
/// Behavior:
/// - releases one recursive ownership level held by the current thread
/// - wakes blocked [`flockfile`] callers on final release
/// - removes stream entries that exist only for lock tracking once unlocked
///
/// # Safety
/// - `stream` must be a valid `FILE*` when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn funlockfile(stream: *mut FILE) {
  if stream.is_null() {
    set_errno(EINVAL);

    return;
  }

  let key = stream_key(stream);
  let current = thread::current().id();
  let mut registry = stream_registry_guard();
  let Some(position) = registry
    .iter()
    .position(|stream_state| stream_state.stream_key == key)
  else {
    return;
  };
  let mut notify_waiters = false;
  let remove_stream_state = {
    let stream_state = registry
      .get_mut(position)
      .unwrap_or_else(|| unreachable!("stream position from `position` must remain valid"));

    if stream_state.file_lock_owner != Some(current) || stream_state.file_lock_depth == 0 {
      return;
    }

    if stream_state.file_lock_depth > 1 {
      stream_state.file_lock_depth -= 1;

      false
    } else {
      stream_state.file_lock_owner = None;
      stream_state.file_lock_depth = 0;
      notify_waiters = true;

      stream_state_is_prunable(stream_state)
    }
  };

  if remove_stream_state {
    registry.remove(position);
  }

  drop(registry);

  if notify_waiters {
    stream_registry_wait_cv().notify_all();
  }
}

/// C ABI entry point for `fileno`.
///
/// Contract:
/// - returns the descriptor backing `stream`
/// - resolves host `stdin`/`stdout`/`stderr` without delegating through host
///   libc so those streams still work when host `fileno` is unavailable during
///   tests
/// - preserves the calling thread's `errno` value on success
///
/// Returns:
/// - non-negative descriptor on success
/// - `-1` when `stream` is null or host descriptor resolution fails
///
/// # Errors
/// - Sets `errno = EINVAL` when `stream` is null
/// - Otherwise propagates host `fileno` failure `errno`
///
/// # Safety
/// - `stream` must be a valid host `FILE*` when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fileno(stream: *mut FILE) -> c_int {
  if stream.is_null() {
    set_errno(EINVAL);

    return -1;
  }

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };

  if let Some(fd) = host_standard_stream_fd(stream) {
    // SAFETY: preserve caller-observed errno on successful descriptor lookup.
    unsafe {
      __errno_location().write(errno_before);
    }

    return fd;
  }

  // SAFETY: non-null `stream` is delegated to host libc `fileno`.
  let descriptor = unsafe { host_fileno(stream) };

  if descriptor >= 0 {
    // SAFETY: preserve caller-observed errno on successful descriptor lookup.
    unsafe {
      __errno_location().write(errno_before);
    }
  }

  descriptor
}

/// C ABI entry point for `fileno_unlocked`.
///
/// Contract:
/// - matches [`fileno`] for the current implementation
/// - preserves the calling thread's `errno` value on success
///
/// Returns:
/// - non-negative descriptor on success
/// - `-1` when `stream` is null or host descriptor resolution fails
///
/// # Errors
/// - Sets `errno = EINVAL` when `stream` is null
/// - Otherwise propagates host `fileno` failure `errno`
///
/// # Safety
/// - `stream` must be a valid host `FILE*` when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fileno_unlocked(stream: *mut FILE) -> c_int {
  // SAFETY: `fileno_unlocked` currently shares `fileno`'s contract exactly.
  unsafe { fileno(stream) }
}

/// C ABI entry point for `tmpfile`.
///
/// Contract:
/// - delegates stream creation to host libc `tmpfile`
/// - clears stale per-pointer stream tracking before returning a recycled host
///   stream address
/// - preserves the calling thread's `errno` value on success
///
/// Returns:
/// - non-null `FILE*` on success
/// - null on failure
///
/// # Errors
/// - Sets `errno = EINVAL` when host `tmpfile` resolution fails
/// - Otherwise propagates host libc `tmpfile` failure `errno`.
///
/// # Safety
/// - The returned stream, when non-null, follows host libc `FILE*` lifetime rules.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tmpfile() -> *mut FILE {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  // SAFETY: host libc owns the `tmpfile` contract.
  let stream = unsafe { host_tmpfile_ffi() };

  if !stream.is_null() {
    clear_stream_tracking_state(stream);

    // SAFETY: preserve caller-observed errno on successful stream creation.
    unsafe {
      __errno_location().write(errno_before);
    }
  }

  stream
}

/// C ABI entry point for `fopen`.
///
/// Contract:
/// - delegates stream opening to host libc `fopen`
/// - preserves the calling thread's `errno` value on success
///
/// Returns:
/// - non-null `FILE*` on success
/// - null on failure
///
/// # Errors
/// - Propagates host libc `fopen` failure `errno`.
///
/// # Safety
/// - `path` and `mode` must point to valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
  if path.is_null() || mode.is_null() {
    set_errno(EINVAL);

    return core::ptr::null_mut();
  }

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let Some(host_fopen) = host_fopen_fn() else {
    set_errno(EINVAL);

    return core::ptr::null_mut();
  };
  // SAFETY: host libc owns the `fopen` contract.
  let stream = unsafe { host_fopen(path, mode) };

  if !stream.is_null() {
    clear_stream_tracking_state(stream);

    // SAFETY: preserve caller-observed errno on successful stream creation.
    unsafe {
      __errno_location().write(errno_before);
    }
  }

  stream
}

/// C ABI entry point for `fread`.
///
/// Contract:
/// - delegates reads to host libc `fread`
/// - preserves the calling thread's `errno` value when the full requested item
///   count is read, or when either multiplicand is zero
///
/// Returns:
/// - the number of elements transferred, matching host libc `fread`
///
/// # Errors
/// - Sets `errno = EINVAL` when host `fread` resolution fails
/// - Short reads and failures preserve host libc `errno` so callers can
///   distinguish EOF from downstream read errors.
///
/// # Safety
/// - `ptr` must be writable for `size * nmemb` bytes when that product is nonzero.
/// - `stream` must be a valid host `FILE*`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fread(
  ptr: *mut c_void,
  size: size_t,
  nmemb: size_t,
  stream: *mut FILE,
) -> size_t {
  if size == 0 || nmemb == 0 {
    return 0;
  }

  if ptr.is_null() || stream.is_null() {
    set_errno(EINVAL);

    return 0;
  }

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let Some(host_fread) = host_fread_fn() else {
    set_errno(EINVAL);

    return 0;
  };
  // SAFETY: host libc owns the `fread` contract.
  let elements_read = unsafe { host_fread(ptr, size, nmemb, stream) };

  if elements_read > 0 {
    mark_stream_as_host_io_active(stream);
  }

  if size == 0 || nmemb == 0 || elements_read == nmemb {
    // SAFETY: preserve caller-observed errno when host `fread` completed successfully.
    unsafe {
      __errno_location().write(errno_before);
    }
  }

  elements_read
}

/// C ABI entry point for `fputs`.
///
/// Contract:
/// - delegates writes to host libc `fputs`
/// - preserves the calling thread's `errno` value on success
///
/// Returns:
/// - non-negative value on success
/// - [`EOF`] on failure
///
/// # Errors
/// - Sets `errno = EINVAL` when host `fputs` resolution fails
/// - Propagates host libc `fputs` failure `errno`.
///
/// # Safety
/// - `s` must point to a valid NUL-terminated string.
/// - `stream` must be a valid host `FILE*`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputs(s: *const c_char, stream: *mut FILE) -> c_int {
  if s.is_null() || stream.is_null() {
    set_errno(EINVAL);

    return EOF;
  }

  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let Some(host_fputs) = host_fputs_fn() else {
    set_errno(EINVAL);

    return EOF;
  };
  // SAFETY: host libc owns the `fputs` contract.
  let result = unsafe { host_fputs(s, stream) };

  if result >= 0 {
    mark_stream_as_host_io_active(stream);

    // SAFETY: preserve caller-observed errno on successful writes.
    unsafe {
      __errno_location().write(errno_before);
    }
  }

  result
}

/// C ABI entry point for `fclose`.
///
/// Behavior:
/// - delegates stream close to host libc `fclose`.
/// - after host close attempt (success or failure), removes buffered/
///   stream-activity tracking for the closed stream key from this module's
///   internal registry to avoid stale pointer-key reuse.
///
/// Returns:
/// - `0` on success
/// - [`EOF`] when `stream` is null or host `fclose` resolution fails
///
/// # Errors
/// - Sets `errno = EINVAL` when:
///   - `stream` is null
///   - host `fclose` symbol resolution fails
/// - On host `fclose` failure, preserves host-provided `errno`.
///
/// # Safety
/// - `stream` must be a valid host `FILE*` when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fclose(stream: *mut FILE) -> c_int {
  if stream.is_null() {
    set_errno(EINVAL);

    return EOF;
  }

  let Some(host_fclose_fn) = host_fclose_fn() else {
    set_errno(EINVAL);

    return EOF;
  };
  let mut stream_lock = StreamFileLockGuard::acquire(stream);
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  // SAFETY: caller guarantees `stream` is valid for host `fclose`.
  let close_status = unsafe { host_fclose_fn(stream) };

  // `fclose` invalidates the host `FILE*` on both success and failure, so the
  // close path must not attempt a trailing `funlockfile(stream)` in `Drop`.
  stream_lock.disarm();

  clear_stream_tracking_state(stream);

  if close_status == 0 {
    // SAFETY: restore caller-observed errno on successful close.
    unsafe {
      __errno_location().write(errno_before);
    }
  }

  close_status
}

/// C ABI entry point for `fflush`.
///
/// Contract:
/// - `fflush(NULL)` marks all tracked streams as having observed I/O.
/// - when host `fflush` is available, `fflush(NULL)` delegates flush-all
///   behavior to host libc.
/// - when host `fflush` is unavailable, `fflush(NULL)` succeeds via internal
///   tracking fallback.
/// - `fflush(NULL)` also marks host `stdin`/`stdout`/`stderr` as host-backed
///   I/O-active when available, so later [`setvbuf`] reconfiguration attempts
///   on those streams are rejected.
/// - `fflush(stream)` marks one stream as having observed I/O.
/// - for host `stdin`/`stdout`/`stderr` streams and streams with prior
///   successful host-backed output via [`vfprintf`], `fflush(stream)` delegates
///   per-stream flushing to host libc when host `fflush` is available.
/// - when `stream` is not yet tracked by this module, a stream-state entry is
///   created and marked as I/O-active.
///
/// Returns:
/// - `0` on success
/// - [`EOF`] when host libc reports a delegated flush failure (`fflush(NULL)`
///   or host-backed `fflush(stream)`), if host delegation is available
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

    mark_all_streams_as_io_active();

    #[cfg(test)]
    if let Some(errno) = host_fflush_forced_failure_errno_for_tests() {
      set_errno(errno);

      return EOF;
    }

    if let Some(host_fflush_fn) = host_fflush_fn() {
      // SAFETY: host flush-all contract for null stream pointer.
      let flush_status = unsafe { host_fflush_fn(core::ptr::null_mut()) };

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

    // SAFETY: internal fallback preserves caller-observed errno on success.
    unsafe {
      __errno_location().write(errno_before);
    }

    return 0;
  }

  let _stream_lock = StreamFileLockGuard::acquire(stream);
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let host_backed = mark_stream_as_io_active(stream);

  if !host_backed {
    return 0;
  }

  let Some(host_fflush_fn) = host_fflush_fn() else {
    // SAFETY: preserve caller-observed errno when host flush symbol is unavailable.
    unsafe {
      __errno_location().write(errno_before);
    }

    return 0;
  };

  #[cfg(test)]
  if let Some(errno) = host_fflush_forced_failure_errno_for_tests() {
    set_errno(errno);

    return EOF;
  }

  // SAFETY: host-backed stream was validated by prior successful host `vfprintf`.
  let flush_status = unsafe { host_fflush_fn(stream) };

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

/// C ABI entry point for `setbuffer`.
///
/// This compatibility wrapper maps the BSD/glibc `setbuffer` contract onto
/// [`setvbuf`] tracking:
/// - non-null `buffer` selects fully buffered mode with the supplied `size`
/// - null `buffer` selects unbuffered mode and ignores `size`
///
/// This wrapper preserves the underlying [`setvbuf`] error behavior; because
/// the C ABI return type is `void`, callers must observe `errno` when they
/// need to diagnose invalid arguments or late reconfiguration attempts.
///
/// # Safety
/// - `stream` must be a valid `FILE*` handle when non-null.
/// - when non-null, `buffer` must remain valid according to the surrounding C
///   stdio contract for the selected buffering mode.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setbuffer(stream: *mut FILE, buffer: *mut c_char, size: size_t) {
  let mode = if buffer.is_null() { _IONBF } else { _IOFBF };
  let effective_size = if mode == _IONBF { 0 } else { size };

  // SAFETY: wrapper forwards the original stream/buffer pointers into `setvbuf`.
  let _ = unsafe { setvbuf(stream, buffer, mode, effective_size) };
}

/// C ABI entry point for `setbuf`.
///
/// This wrapper follows the glibc `setbuf` contract:
/// - non-null `buffer` selects fully buffered mode with [`BUFSIZ`] bytes
/// - null `buffer` selects unbuffered mode
///
/// Failures are surfaced only through the underlying [`setvbuf`] side effects
/// (`errno` / unchanged stream state), matching the `void` C ABI.
///
/// # Safety
/// - `stream` must be a valid `FILE*` handle when non-null.
/// - when non-null, `buffer` must reference at least [`BUFSIZ`] writable bytes
///   for the lifetime required by the surrounding C stdio contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setbuf(stream: *mut FILE, buffer: *mut c_char) {
  let mode = if buffer.is_null() { _IONBF } else { _IOFBF };
  let size = if mode == _IONBF { 0 } else { BUFSIZ };

  // SAFETY: wrapper forwards the original stream/buffer pointers into `setvbuf`.
  let _ = unsafe { setvbuf(stream, buffer, mode, size) };
}

/// C ABI entry point for `setlinebuf`.
///
/// `setlinebuf` is a legacy compatibility wrapper that configures a stream for
/// line buffering using the crate's default tracked buffer size [`BUFSIZ`].
/// The underlying buffering behavior still flows through [`setvbuf`] tracking
/// and therefore inherits its late-reconfiguration rejection once I/O has
/// occurred on the stream.
///
/// # Safety
/// - `stream` must be a valid `FILE*` handle when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setlinebuf(stream: *mut FILE) {
  // SAFETY: wrapper forwards the validated stream handle into `setvbuf`.
  let _ = unsafe { setvbuf(stream, core::ptr::null_mut(), _IOLBF, BUFSIZ) };
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

  let _stream_lock = StreamFileLockGuard::acquire(stream);
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  let errno_before = unsafe { __errno_location().read() };
  let key = stream_key(stream);
  let mut registry = stream_registry_guard();
  let stream_state = stream_state_mut_or_insert(&mut registry, key);

  if stream_state.io_active && stream_state.host_backed_io {
    let previous_identity = stream_state.host_stream_identity;
    let current_identity = read_host_stream_identity(stream);
    let recycled_with_new_identity = previous_identity
      .zip(current_identity)
      .is_some_and(|(previous, current)| previous != current);
    let recycled_after_unidentifiable_previous =
      previous_identity.is_none() && current_identity.is_some();

    if recycled_with_new_identity || recycled_after_unidentifiable_previous {
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
/// - resolves host libc `vfprintf` via `dlvsym(RTLD_NEXT, "vfprintf", GLIBC_*)`
///   and falls back to `dlsym(RTLD_NEXT, "vfprintf")` when versioned lookup is
///   unavailable.
/// - forwards fresh host `FILE*` streams to the resolved host symbol so host
///   buffering and ordering guarantees remain intact even for format strings
///   supported by the internal formatter subset.
/// - when host `vfprintf` is unavailable, or a stream is already known to use
///   direct descriptor emission instead of host stdio buffering, falls back to
///   internal `vsnprintf` subset formatting then emits bytes through
///   `write(..)` using direct descriptor mapping for host `stdin`/`stdout`/
///   `stderr` and `fileno(stream)` for other streams.
/// - writes to the provided `stream`.
/// - successful internal fallback writes restore the caller-observed `errno`
///   value after stream descriptor emission.
/// - when forwarding reaches host libc (after argument validation), marks
///   `stream` as host-backed and I/O-active even when host `vfprintf` fails, so
///   subsequent [`setvbuf`] reconfiguration attempts are rejected and
///   [`fflush(stream)`] can continue delegated per-stream behavior.
/// - when internal fallback path reaches stream-descriptor emission, marks
///   `stream` as I/O-active without host-backed flush delegation.
/// - when `stream` was explicitly configured through [`setvbuf`] with `_IONBF`,
///   successful writes immediately delegate `fflush(stream)` so bytes become
///   observable on the underlying descriptor without requiring a separate flush.
/// - when `stream` was explicitly configured through [`setvbuf`] with `_IOLBF`,
///   successful writes that emit newline bytes, or that use host-only format
///   shapes this module cannot safely analyze for newline emission, delegate
///   `fflush(stream)` so line-buffered observability is preserved.
///
/// Returns:
/// - non-negative byte count on success (excluding the implicit NUL terminator)
/// - negative value on failure
///
/// # Errors
/// - Sets `errno = EINVAL` and returns `-1` when `stream`, `format`, or `ap`
///   is null.
/// - returns `-1` and preserves formatter/stream errno when both host
///   forwarding and internal fallback fail.
/// - for explicit `_IONBF` streams and explicit `_IOLBF` writes that require
///   conservative post-write flushing, returns `-1` when delegated
///   `fflush(stream)` fails; `errno` is populated from host failure details.
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

  let _stream_lock = StreamFileLockGuard::acquire(stream);
  let explicit_mode_before_write = stream_explicit_buffering_mode(stream);
  let line_buffered_requires_flush = if matches!(explicit_mode_before_write, Some(_IOLBF)) {
    // SAFETY: pointers were validated and helper only reads format/va_list.
    unsafe { formatted_output_requires_host_line_buffer_flush(format, ap) }
  } else {
    false
  };

  // SAFETY: pointers were validated non-null and caller upholds C ABI contracts.
  let (status, write_path) = unsafe { forward_host_vfprintf(stream, format, ap) };

  match write_path {
    VfprintfWritePath::HostDelegated => mark_stream_as_host_io_active(stream),
    VfprintfWritePath::InternalStreamIo => {
      let _ = mark_stream_as_io_active(stream);
    }
    VfprintfWritePath::NoStreamIo => {}
  }

  if status >= 0 {
    let should_flush_after_write = match explicit_mode_before_write {
      Some(_IONBF) => true,
      Some(_IOLBF) => line_buffered_requires_flush,
      _ => false,
    };

    if should_flush_after_write && matches!(write_path, VfprintfWritePath::HostDelegated) {
      // SAFETY: `__errno_location` returns writable thread-local errno storage.
      let errno_before = unsafe { __errno_location().read() };
      // SAFETY: successful host-backed write established stream validity.
      let flush_status = unsafe { fflush(stream) };

      if flush_status != 0 {
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
/// - resolves host libc `stdout` and forwards to [`vfprintf`] with that stream.
/// - when host `stdout` is unavailable, falls back to internal `vsnprintf`
///   formatting and writes bytes directly to descriptor `1`.
///
/// Returns:
/// - non-negative byte count on success
/// - negative value on failure
///
/// # Errors
/// - Sets `errno = EINVAL` and returns `-1` when `format`/`ap` is null.
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

  let stdout_stream = host_stdout_stream();

  if !stdout_stream.is_null() {
    // SAFETY: pointers are non-null and contracts are delegated to `vfprintf`.
    return unsafe { vfprintf(stdout_stream, format, ap) };
  }

  // SAFETY: pointers were validated and formatter contract matches `vprintf`.
  unsafe { forward_internal_vprintf_stdout(format, ap) }
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
  use std::sync::mpsc;
  use std::thread;
  use std::time::Duration;

  unsafe extern "C" {
    fn close(fd: c_int) -> c_int;
    fn fclose(stream: *mut FILE) -> c_int;
    fn fread(ptr: *mut c_void, size: size_t, nmemb: size_t, stream: *mut FILE) -> size_t;
    fn fileno(stream: *mut FILE) -> c_int;
    fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE;
    fn fputs(s: *const c_char, stream: *mut FILE) -> c_int;
    fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    fn rewind(stream: *mut FILE);
    fn tmpfile() -> *mut FILE;
  }

  struct HostVfprintfUnavailableGuard;

  struct HostFilenoUnavailableGuard;
  struct HostFflushUnavailableGuard;
  struct HostFflushFailureGuard;
  struct HostStdoutUnavailableGuard;

  #[repr(align(16))]
  struct TestRegSaveArea {
    bytes: [u8; 176],
  }

  struct OwnedSysVVaList {
    va_list: SysVVaList,
    _reg_save_area: Box<TestRegSaveArea>,
    _overflow_words: Vec<u64>,
  }

  impl OwnedSysVVaList {
    fn new(gp_values: &[u64], fp_values: &[f64], overflow_words: &[u64]) -> Self {
      Self::with_offsets(0, 48, gp_values, fp_values, overflow_words)
    }

    fn with_offsets(
      gp_offset: u32,
      fp_offset: u32,
      gp_values: &[u64],
      fp_values: &[f64],
      overflow_words: &[u64],
    ) -> Self {
      let mut reg_save_area = Box::new(TestRegSaveArea { bytes: [0_u8; 176] });

      for (slot, value) in gp_values.iter().copied().take(6).enumerate() {
        let start = usize::try_from(gp_offset)
          .unwrap_or_else(|_| unreachable!("gp_offset must fit usize"))
          + slot * core::mem::size_of::<u64>();
        let end = start + core::mem::size_of::<u64>();

        reg_save_area.bytes[start..end].copy_from_slice(&value.to_ne_bytes());
      }

      for (slot, value) in fp_values.iter().copied().take(8).enumerate() {
        let start = usize::try_from(fp_offset)
          .unwrap_or_else(|_| unreachable!("fp_offset must fit usize"))
          + slot * 16;
        let end = start + core::mem::size_of::<f64>();

        reg_save_area.bytes[start..end].copy_from_slice(&value.to_ne_bytes());
      }

      let mut overflow_storage = overflow_words.to_vec();
      let overflow_arg_area = if overflow_storage.is_empty() {
        ptr::null_mut()
      } else {
        overflow_storage.as_mut_ptr().cast::<c_void>()
      };
      let va_list = SysVVaList {
        gp_offset,
        fp_offset,
        overflow_arg_area,
        reg_save_area: reg_save_area.bytes.as_mut_ptr().cast::<c_void>(),
      };

      Self {
        va_list,
        _reg_save_area: reg_save_area,
        _overflow_words: overflow_storage,
      }
    }

    fn as_mut_ptr(&mut self) -> *mut c_void {
      ptr::from_mut(&mut self.va_list).cast::<c_void>()
    }
  }

  impl Drop for HostVfprintfUnavailableGuard {
    fn drop(&mut self) {
      set_host_vfprintf_unavailable_for_tests(false);
    }
  }

  impl Drop for HostFilenoUnavailableGuard {
    fn drop(&mut self) {
      set_host_fileno_unavailable_for_tests(false);
    }
  }

  impl Drop for HostFflushUnavailableGuard {
    fn drop(&mut self) {
      set_host_fflush_unavailable_for_tests(false);
    }
  }

  impl Drop for HostFflushFailureGuard {
    fn drop(&mut self) {
      set_host_fflush_forced_failure_errno_for_tests(0);
    }
  }

  impl Drop for HostStdoutUnavailableGuard {
    fn drop(&mut self) {
      set_host_stdout_unavailable_for_tests(false);
    }
  }

  fn force_host_vfprintf_unavailable_for_tests() -> HostVfprintfUnavailableGuard {
    set_host_vfprintf_unavailable_for_tests(true);

    HostVfprintfUnavailableGuard
  }

  fn force_host_fileno_unavailable_for_tests() -> HostFilenoUnavailableGuard {
    set_host_fileno_unavailable_for_tests(true);

    HostFilenoUnavailableGuard
  }

  fn force_host_fflush_unavailable_for_tests() -> HostFflushUnavailableGuard {
    set_host_fflush_unavailable_for_tests(true);

    HostFflushUnavailableGuard
  }

  fn force_host_fflush_failure_for_tests(errno: c_int) -> HostFflushFailureGuard {
    set_host_fflush_forced_failure_errno_for_tests(errno);

    HostFflushFailureGuard
  }

  fn force_host_stdout_unavailable_for_tests() -> HostStdoutUnavailableGuard {
    set_host_stdout_unavailable_for_tests(true);

    HostStdoutUnavailableGuard
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

  fn overflow_only_va_list(words: &mut [u64]) -> SysVVaList {
    let overflow_arg_area = if words.is_empty() {
      ptr::null_mut()
    } else {
      words.as_mut_ptr().cast::<c_void>()
    };

    SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area,
      reg_save_area: ptr::null_mut(),
    }
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

  fn file_lock_snapshot_for_tests(stream: *mut FILE) -> Option<(bool, usize)> {
    let key = stream_key(stream);
    let current = thread::current().id();
    let registry = stream_registry_guard();

    registry
      .iter()
      .find(|state| state.stream_key == key)
      .map(|state| {
        (
          state.file_lock_owner == Some(current),
          state.file_lock_depth,
        )
      })
  }

  #[test]
  fn ftrylockfile_same_thread_is_recursive_and_funlockfile_releases_all_depth() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);

    write_errno(73);

    // SAFETY: test-only pseudo stream pointer is used only for registry-key tracking.
    let first_lock_status = unsafe { ftrylockfile(stream) };
    // SAFETY: same-thread recursive acquisition is supported by stdio FILE locks.
    let second_lock_status = unsafe { ftrylockfile(stream) };

    assert_eq!(first_lock_status, 0);
    assert_eq!(second_lock_status, 0);
    assert_eq!(read_errno(), 73);
    assert_eq!(file_lock_snapshot_for_tests(stream), Some((true, 2)));

    // SAFETY: current thread owns one recursive level of the FILE lock.
    unsafe {
      funlockfile(stream);
    }

    assert_eq!(file_lock_snapshot_for_tests(stream), Some((true, 1)));

    // SAFETY: current thread owns the remaining FILE lock depth.
    unsafe {
      funlockfile(stream);
    }

    assert_eq!(file_lock_snapshot_for_tests(stream), None);
    assert_eq!(read_errno(), 73);
  }

  #[test]
  fn ftrylockfile_returns_ebusy_for_other_thread_while_stream_is_locked() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let stream_addr = stream.addr();

    // SAFETY: pseudo stream pointer is used only for registry-key tracking.
    unsafe {
      flockfile(stream);
    }

    let (status_tx, status_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
      let stream = stream_addr as *mut FILE;
      // SAFETY: pseudo stream pointer is used only for registry-key tracking.
      let status = unsafe { ftrylockfile(stream) };

      status_tx
        .send(status)
        .expect("busy status must be sent back to the test thread");
    });
    let other_thread_status = status_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("other thread must report immediate try-lock result");

    assert_eq!(other_thread_status, EBUSY);

    // SAFETY: current thread owns the FILE lock acquired above.
    unsafe {
      funlockfile(stream);
    }

    handle
      .join()
      .expect("try-lock worker thread must complete successfully");
  }

  #[test]
  fn flockfile_waits_until_other_thread_releases_the_stream_lock() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let stream_addr = stream.addr();

    // SAFETY: pseudo stream pointer is used only for registry-key tracking.
    unsafe {
      flockfile(stream);
    }

    let (ready_tx, ready_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
      let stream = stream_addr as *mut FILE;

      ready_tx
        .send(())
        .expect("worker readiness signal must reach the test thread");

      // SAFETY: pseudo stream pointer is used only for registry-key tracking.
      unsafe {
        flockfile(stream);
      }

      acquired_tx
        .send(())
        .expect("worker acquisition signal must reach the test thread");

      // SAFETY: worker thread now owns the FILE lock it just acquired.
      unsafe {
        funlockfile(stream);
      }
    });

    ready_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker thread must start before acquisition assertions");

    assert!(
      acquired_rx
        .recv_timeout(Duration::from_millis(100))
        .is_err(),
      "worker flockfile must stay blocked while another thread owns the FILE lock",
    );

    // SAFETY: current thread owns the FILE lock acquired above.
    unsafe {
      funlockfile(stream);
    }

    acquired_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker must acquire the FILE lock after release");

    handle
      .join()
      .expect("blocking flockfile worker thread must complete successfully");
  }

  #[test]
  fn setvbuf_waits_until_other_thread_releases_the_stream_lock() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let stream_addr = stream.addr();

    // SAFETY: pseudo stream pointer is used only for registry-key tracking.
    unsafe {
      flockfile(stream);
    }

    let (ready_tx, ready_rx) = mpsc::channel();
    let (status_tx, status_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
      let stream = stream_addr as *mut FILE;
      let mut user_buffer = [0_u8; 8];
      let buffer_addr = user_buffer.as_mut_ptr().cast::<c_char>().addr();

      ready_tx
        .send(())
        .expect("worker readiness signal must reach the test thread");

      // SAFETY: pseudo stream pointer is used only for registry-key tracking.
      let status = unsafe {
        setvbuf(
          stream,
          user_buffer.as_mut_ptr().cast::<c_char>(),
          _IOFBF,
          as_size_t(user_buffer.len()),
        )
      };

      status_tx
        .send((status, buffer_addr))
        .expect("worker setvbuf status must reach the test thread");
    });

    ready_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker thread must start before blocking assertions");

    assert!(
      status_rx.recv_timeout(Duration::from_millis(100)).is_err(),
      "setvbuf must stay blocked while another thread owns the FILE lock",
    );

    // SAFETY: current thread owns the FILE lock acquired above.
    unsafe {
      funlockfile(stream);
    }

    let (setvbuf_status, buffer_addr) = status_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker setvbuf must complete after lock release");

    assert_eq!(setvbuf_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOFBF, 8, buffer_addr, false))
    );

    handle
      .join()
      .expect("blocking setvbuf worker thread must complete successfully");
  }

  #[test]
  fn fflush_waits_until_other_thread_releases_the_stream_lock() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let stream_addr = stream.addr();

    // SAFETY: pseudo stream pointer is used only for registry-key tracking.
    unsafe {
      flockfile(stream);
    }

    let (ready_tx, ready_rx) = mpsc::channel();
    let (status_tx, status_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
      let stream = stream_addr as *mut FILE;

      ready_tx
        .send(())
        .expect("worker readiness signal must reach the test thread");

      // SAFETY: pseudo stream pointer is used only for registry-key tracking.
      let status = unsafe { fflush(stream) };

      status_tx
        .send(status)
        .expect("worker fflush status must reach the test thread");
    });

    ready_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker thread must start before blocking assertions");

    assert!(
      status_rx.recv_timeout(Duration::from_millis(100)).is_err(),
      "fflush(stream) must stay blocked while another thread owns the FILE lock",
    );

    // SAFETY: current thread owns the FILE lock acquired above.
    unsafe {
      funlockfile(stream);
    }

    let flush_status = status_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker fflush must complete after lock release");

    assert_eq!(flush_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream).map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(true),
    );

    handle
      .join()
      .expect("blocking fflush worker thread must complete successfully");
  }

  #[test]
  fn vfprintf_waits_until_other_thread_releases_the_stream_lock() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let stream_addr = stream.addr();
    let format = c_string("");

    // SAFETY: valid host FILE pointer is tracked only by local FILE lock scaffolding.
    unsafe {
      flockfile(stream);
    }

    let (ready_tx, ready_rx) = mpsc::channel();
    let (status_tx, status_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
      let stream = stream_addr as *mut FILE;
      let mut empty_ap = SysVVaList {
        gp_offset: 48,
        fp_offset: 0,
        overflow_arg_area: ptr::null_mut(),
        reg_save_area: ptr::null_mut(),
      };

      ready_tx
        .send(())
        .expect("worker readiness signal must reach the test thread");

      // SAFETY: stream and format are valid; empty format consumes no variadic args.
      let status = unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

      status_tx
        .send(status)
        .expect("worker vfprintf status must reach the test thread");
    });

    ready_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker thread must start before blocking assertions");

    assert!(
      status_rx.recv_timeout(Duration::from_millis(100)).is_err(),
      "vfprintf must stay blocked while another thread owns the FILE lock",
    );

    // SAFETY: current thread owns the FILE lock acquired above.
    unsafe {
      funlockfile(stream);
    }

    let write_status = status_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker vfprintf must complete after lock release");

    assert_eq!(write_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream).map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(true),
    );

    handle
      .join()
      .expect("blocking vfprintf worker thread must complete successfully");

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
  }

  #[test]
  fn fclose_waits_until_other_thread_releases_the_stream_lock() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let stream_addr = stream.addr();

    // SAFETY: valid host FILE pointer is tracked only by local FILE lock scaffolding.
    unsafe {
      flockfile(stream);
    }

    let (ready_tx, ready_rx) = mpsc::channel();
    let (status_tx, status_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
      let stream = stream_addr as *mut FILE;

      ready_tx
        .send(())
        .expect("worker readiness signal must reach the test thread");

      // SAFETY: stream came from `tmpfile`.
      let status = unsafe { fclose(stream) };

      status_tx
        .send(status)
        .expect("worker fclose status must reach the test thread");
    });

    ready_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker thread must start before blocking assertions");

    assert!(
      status_rx.recv_timeout(Duration::from_millis(100)).is_err(),
      "fclose must stay blocked while another thread owns the FILE lock",
    );

    // SAFETY: current thread owns the FILE lock acquired above.
    unsafe {
      funlockfile(stream);
    }

    let close_status = status_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("worker fclose must complete after lock release");

    assert_eq!(close_status, 0);
    assert_eq!(buffering_snapshot_for_tests(stream), None);
    assert_eq!(host_backed_snapshot_for_tests(stream), None);

    handle
      .join()
      .expect("blocking fclose worker thread must complete successfully");
  }

  #[test]
  fn fclose_clears_stream_file_lock_tracking_state() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    // SAFETY: valid host FILE pointer is tracked only by local FILE lock scaffolding.
    unsafe {
      flockfile(stream);
    }

    assert_eq!(file_lock_snapshot_for_tests(stream), Some((true, 1)));

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(file_lock_snapshot_for_tests(stream), None);
  }

  #[test]
  fn fclose_success_clears_stream_tracking_state() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let user_buffer = Box::leak(Box::new([0_u8; 8]));

    // SAFETY: stream and buffer pointers are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(setvbuf_status, 0);
    assert!(
      buffering_snapshot_for_tests(stream).is_some(),
      "setvbuf must register stream tracking state before close",
    );

    write_errno(97);

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(read_errno(), 97);
    assert_eq!(buffering_snapshot_for_tests(stream), None);
    assert_eq!(host_backed_snapshot_for_tests(stream), None);
  }

  #[test]
  fn fclose_failure_still_clears_stream_tracking_state() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut user_buffer = [0_u8; 8];

    // SAFETY: stream and buffer pointers are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(setvbuf_status, 0);
    assert!(
      buffering_snapshot_for_tests(stream).is_some(),
      "setvbuf must register stream tracking state before close",
    );
    // SAFETY: `fileno` expects a valid host FILE pointer.
    let fd = unsafe { fileno(stream) };

    assert!(fd >= 0);
    // SAFETY: descriptor comes from host `fileno`.
    let close_fd_status = unsafe { close(fd) };

    assert_eq!(close_fd_status, 0);
    write_errno(101);

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, EOF);
    assert_ne!(read_errno(), 0);
    assert_eq!(buffering_snapshot_for_tests(stream), None);
    assert_eq!(host_backed_snapshot_for_tests(stream), None);
  }

  #[test]
  fn is_local_vfprintf_symbol_accepts_local_vfprintf_pointer() {
    let local_symbol =
      vfprintf as unsafe extern "C" fn(*mut FILE, *const c_char, *mut c_void) -> c_int;
    let symbol_ptr = local_symbol as *const () as *mut c_void;

    assert!(is_local_vfprintf_symbol(symbol_ptr));
  }

  #[test]
  fn is_local_vfprintf_symbol_rejects_other_local_function_pointer() {
    let local_symbol = fflush as unsafe extern "C" fn(*mut FILE) -> c_int;
    let symbol_ptr = local_symbol as *const () as *mut c_void;

    assert!(!is_local_vfprintf_symbol(symbol_ptr));
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
  fn fflush_null_without_host_symbol_still_tracks_host_std_streams() {
    let _guard = test_lock();
    let _host_fflush_guard = force_host_fflush_unavailable_for_tests();

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

    write_errno(37);

    // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
    let flush_status = unsafe { fflush(ptr::null_mut()) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 37);

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
  }

  #[test]
  fn fflush_null_without_host_symbol_marks_registered_stream_io_active() {
    let _guard = test_lock();
    let _host_fflush_guard = force_host_fflush_unavailable_for_tests();

    clear_stream_registry_for_tests();

    let mut marker = 0_u8;
    let stream = as_file_ptr(&mut marker);
    let mut user_buffer = [0_u8; 8];
    let user_buffer_addr = user_buffer.as_mut_ptr().cast::<c_char>().addr();

    // SAFETY: local test stream marker and user buffer pointer are valid.
    let setvbuf_status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(setvbuf_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOFBF, user_buffer.len(), user_buffer_addr, false)),
      "registered stream must start with io_active=false before flush-all",
    );

    write_errno(71);

    // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
    let flush_status = unsafe { fflush(ptr::null_mut()) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 71);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOFBF, user_buffer.len(), user_buffer_addr, true)),
      "fflush(NULL) must mark registered stream io_active when host resolver is unavailable",
    );
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

    let dev_full_path = c_string("/dev/full");
    let write_mode = c_string("w");

    // SAFETY: host libc owns `fopen` and receives valid NUL-terminated strings.
    let failing_stream = unsafe { fopen(dev_full_path.as_ptr(), write_mode.as_ptr()) };

    assert!(
      !failing_stream.is_null(),
      "/dev/full must provide a stream for failure injection"
    );

    let payload = c_string("i022-flush-null-failure\n");

    // SAFETY: stream and payload pointer are valid for host `fputs`.
    let write_status = unsafe { fputs(payload.as_ptr(), failing_stream) };

    assert!(write_status >= 0, "priming failure stream must succeed");

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

    // SAFETY: host libc still owns the `/dev/full` FILE state and must release it.
    let failing_close_status = unsafe { fclose(failing_stream) };

    assert_eq!(failing_close_status, 0);
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
  fn fflush_stdout_without_host_symbol_tracks_host_backed_state() {
    let _guard = test_lock();
    let _host_fflush_guard = force_host_fflush_unavailable_for_tests();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides `stdout` global stream pointer.
    let stdout_stream = unsafe { host_stdout };

    assert!(
      !stdout_stream.is_null(),
      "host stdout pointer must be available"
    );
    assert_eq!(host_backed_snapshot_for_tests(stdout_stream), None);

    write_errno(58);

    // SAFETY: host `stdout` pointer comes from libc and is valid for `fflush`.
    let flush_status = unsafe { fflush(stdout_stream) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 58);
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

    assert_eq!(second_status, EOF);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IONBF, 0, 0, true))
    );

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(buffering_snapshot_for_tests(stream), None);
  }

  #[test]
  fn vfprintf_unbuffered_stream_succeeds_when_host_fflush_symbol_is_unavailable() {
    let _guard = test_lock();
    let _host_fflush_guard = force_host_fflush_unavailable_for_tests();

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
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IONBF, 0, 0, true))
    );
    assert_eq!(host_backed_snapshot_for_tests(stream), Some(true));

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
  fn vfprintf_without_host_symbol_falls_back_to_internal_formatter_and_write() {
    let _guard = test_lock();
    let _host_vfprintf_guard = force_host_vfprintf_unavailable_for_tests();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let format = c_string("i022-vfprintf-fallback\n");
    let expected_bytes = b"i022-vfprintf-fallback\n";
    let mut read_buffer = [0_u8; 64];
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    write_errno(0);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status =
      unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(
      write_status,
      c_int::try_from(format.as_bytes().len())
        .unwrap_or_else(|_| unreachable!("literal length fits"))
    );
    assert_eq!(read_errno(), 0);

    // SAFETY: stream originated from `tmpfile` and remains valid.
    unsafe { rewind(stream) };

    // SAFETY: read buffer and stream pointers are valid for `fread`.
    let read_count = unsafe {
      fread(
        read_buffer.as_mut_ptr().cast::<c_void>(),
        1,
        as_size_t(read_buffer.len()),
        stream,
      )
    };
    let read_len = usize::try_from(read_count)
      .unwrap_or_else(|_| unreachable!("`size_t` read count must fit usize"));

    assert_eq!(read_len, expected_bytes.len());
    assert_eq!(&read_buffer[..expected_bytes.len()], expected_bytes);
    assert_eq!(host_backed_snapshot_for_tests(stream), Some(false));
    assert_eq!(
      buffering_snapshot_for_tests(stream).map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(true)
    );

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
  }

  #[test]
  fn vfprintf_supported_literal_preserves_host_buffering_order_when_host_symbol_is_available() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    if host_vfprintf().is_none() {
      return;
    }

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let prefix = c_string("host-");
    let format = c_string("rlibc");
    let expected_bytes = b"host-rlibc";
    let mut read_buffer = [0_u8; 64];
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    // SAFETY: stream and prefix are valid for host `fputs`.
    let prefix_status = unsafe { fputs(prefix.as_ptr(), stream) };

    assert!(prefix_status >= 0, "host prefix write must succeed");

    write_errno(67);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status =
      unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(
      write_status,
      c_int::try_from(format.as_bytes().len())
        .unwrap_or_else(|_| unreachable!("literal length fits"))
    );
    assert_eq!(read_errno(), 67);
    assert_eq!(
      host_backed_snapshot_for_tests(stream),
      Some(true),
      "supported literal output on a fresh host FILE* must use host buffering",
    );

    write_errno(19);

    // SAFETY: stream came from host libc and remains valid for flush.
    let flush_status = unsafe { fflush(stream) };

    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 19);

    // SAFETY: stream originated from `tmpfile` and remains valid.
    unsafe { rewind(stream) };

    // SAFETY: read buffer and stream pointers are valid for `fread`.
    let read_count = unsafe {
      fread(
        read_buffer.as_mut_ptr().cast::<c_void>(),
        1,
        as_size_t(read_buffer.len()),
        stream,
      )
    };
    let read_len = usize::try_from(read_count)
      .unwrap_or_else(|_| unreachable!("`size_t` read count must fit usize"));

    assert_eq!(read_len, expected_bytes.len());
    assert_eq!(&read_buffer[..expected_bytes.len()], expected_bytes);
    assert_eq!(
      buffering_snapshot_for_tests(stream).map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(true)
    );

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
  }

  #[test]
  fn vfprintf_unsupported_percent_sequence_uses_host_path_when_available() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    if host_vfprintf().is_none() {
      return;
    }

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let format = c_string("%");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    write_errno(0);

    // SAFETY: stream and format are valid; trailing `%` format is rejected.
    let _ = unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(
      host_backed_snapshot_for_tests(stream),
      Some(true),
      "unsupported format should still fall back to host vfprintf when available",
    );

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
  }

  #[test]
  fn internal_vfprintf_supports_format_rejects_char_precision_shapes() {
    let precision_literal = c_string("%.1c");
    let precision_from_args = c_string("%.*c");

    // SAFETY: each format pointer addresses a readable NUL-terminated string.
    let literal_supported =
      unsafe { internal_vfprintf_supports_format(precision_literal.as_ptr()) };
    // SAFETY: each format pointer addresses a readable NUL-terminated string.
    let from_args_supported =
      unsafe { internal_vfprintf_supports_format(precision_from_args.as_ptr()) };

    assert!(!literal_supported);
    assert!(!from_args_supported);
  }

  #[test]
  fn line_buffer_newline_analysis_defers_non_newline_percent_f_output() {
    let format = c_string("%f");
    let mut ap = OwnedSysVVaList::new(&[], &[1.5_f64], &[]);

    // SAFETY: test format and synthetic SysV va_list stay alive for the call.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "plain %f output without newline should stay deferred for line-buffered host writes",
    );
  }

  #[test]
  fn line_buffer_newline_analysis_defers_mixed_percent_e_percent_s_without_newline() {
    let format = c_string("%e%s");
    let suffix = c_string("tail");
    let mut ap = OwnedSysVVaList::new(&[suffix.as_ptr() as usize as u64], &[1.25_f64], &[]);

    // SAFETY: test format/suffix and synthetic SysV va_list stay alive for the call.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "mixed %e/%s output without newline should stay deferred for line-buffered host writes",
    );
  }

  #[test]
  fn line_buffer_newline_analysis_defers_dynamic_width_and_precision_percent_f_without_newline() {
    let format = c_string("%*.*f");
    let width = u32::try_from(9).unwrap_or_else(|_| unreachable!("fits u32"));
    let precision = u32::try_from(3).unwrap_or_else(|_| unreachable!("fits u32"));
    let mut ap = OwnedSysVVaList::new(&[u64::from(width), u64::from(precision)], &[1.25_f64], &[]);

    // SAFETY: test format and synthetic SysV va_list stay alive for the call.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "dynamic width/precision %f output without newline should stay deferred",
    );
  }

  #[test]
  fn line_buffer_newline_analysis_defers_percent_f_with_escaped_percent_without_newline() {
    let format = c_string("%f%%");
    let mut ap = OwnedSysVVaList::new(&[], &[2.5_f64], &[]);

    // SAFETY: test format and synthetic SysV va_list stay alive for the call.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "floating-point output followed by escaped percent should stay deferred without newline",
    );
  }

  #[test]
  fn line_buffer_newline_analysis_defers_float_without_newline() {
    let format = c_string("%f");
    let mut float_args = [1.25_f64.to_bits()];
    let mut ap = overflow_only_va_list(&mut float_args);

    // SAFETY: format pointer and synthetic va_list remain valid for this read-only analysis.
    let requires_flush = unsafe {
      formatted_output_requires_host_line_buffer_flush(
        format.as_ptr(),
        ptr::addr_of_mut!(ap).cast(),
      )
    };

    assert!(
      !requires_flush,
      "non-newline %f output should stay deferred for line-buffered host writes",
    );
  }

  #[test]
  fn line_buffer_newline_analysis_defers_escaped_percent_after_float_without_newline() {
    let format = c_string("%f%%");
    let mut float_args = [2.5_f64.to_bits()];
    let mut ap = overflow_only_va_list(&mut float_args);

    // SAFETY: format pointer and synthetic va_list remain valid for this read-only analysis.
    let requires_flush = unsafe {
      formatted_output_requires_host_line_buffer_flush(
        format.as_ptr(),
        ptr::addr_of_mut!(ap).cast(),
      )
    };

    assert!(
      !requires_flush,
      "escaped-percent output after %f should stay deferred without a newline",
    );
  }

  #[test]
  fn line_buffer_newline_analysis_tracks_newline_after_float_prefixed_suffix() {
    let format = c_string("%a%s");
    let suffix = c_string("tail\n");
    let suffix_addr = suffix.as_ptr().addr();
    let suffix_word = u64::try_from(suffix_addr)
      .unwrap_or_else(|_| unreachable!("pointer address must fit u64 on x86_64"));
    let mut mixed_args = [1.25_f64.to_bits(), suffix_word];
    let mut ap = overflow_only_va_list(&mut mixed_args);

    // SAFETY: format pointer and synthetic va_list remain valid for this read-only analysis.
    let requires_flush = unsafe {
      formatted_output_requires_host_line_buffer_flush(
        format.as_ptr(),
        ptr::addr_of_mut!(ap).cast(),
      )
    };

    assert!(
      requires_flush,
      "newline carried by a downstream %s after a float directive must trigger a flush",
    );
  }

  #[test]
  fn vfprintf_line_buffered_host_write_flushes_when_newline_shape_is_host_only() {
    const SEEK_END: c_int = 2;
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    if host_vfprintf().is_none() || host_fflush_fn().is_none() {
      return;
    }

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut user_buffer = [0_u8; 8];
    let format = c_string("%lc");
    let mut overflow_words = [u64::from(u32::from('\n'))];
    let mut ap = overflow_only_va_list(&mut overflow_words);

    // SAFETY: stream and user buffer are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(setvbuf_status, 0);

    write_errno(65);

    // SAFETY: stream, format, and va_list pointers are valid for `%lc`.
    let write_status = unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(ap).cast()) };

    assert_eq!(write_status, 1);
    assert_eq!(read_errno(), 65);
    assert_eq!(host_backed_snapshot_for_tests(stream), Some(true));

    // SAFETY: `fileno` expects a valid host stream handle.
    let stream_fd = unsafe { fileno(stream) };

    assert!(stream_fd >= 0);

    // SAFETY: probing file length through the host fd observes whether `vfprintf`
    // forced the tracked line-buffer flush.
    let flushed_len = unsafe { lseek(stream_fd, 0, SEEK_END) };

    assert_eq!(flushed_len, 1);

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
  }

  #[test]
  fn vfprintf_line_buffered_host_write_defers_float_without_newline_until_fflush() {
    const SEEK_END: c_int = 2;
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    if host_vfprintf().is_none() || host_fflush_fn().is_none() {
      return;
    }

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut user_buffer = [0_u8; 8];
    let format = c_string("%f");
    let mut ap = OwnedSysVVaList::new(&[], &[1.5_f64], &[]);

    // SAFETY: stream and user buffer are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(setvbuf_status, 0);

    // SAFETY: stream, format, and synthetic va_list are valid for `%f`.
    let write_status = unsafe { vfprintf(stream, format.as_ptr(), ap.as_mut_ptr()) };

    assert!(write_status > 0);

    // SAFETY: `fileno` expects a valid host stream handle.
    let stream_fd = unsafe { fileno(stream) };

    assert!(stream_fd >= 0);

    // SAFETY: probing file length through the host fd observes pre-fflush visibility.
    let flushed_len = unsafe { lseek(stream_fd, 0, SEEK_END) };

    assert_eq!(flushed_len, 0);

    // SAFETY: stream came from `tmpfile`.
    let flush_status = unsafe { fflush(stream) };

    assert_eq!(flush_status, 0);

    // SAFETY: probing file length after explicit flush is valid for the host fd.
    let flushed_len_after = unsafe { lseek(stream_fd, 0, SEEK_END) };

    assert_eq!(flushed_len_after, i64::from(write_status));

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
  }

  #[test]
  fn fprintf_line_buffered_host_write_defers_float_without_newline_until_fflush() {
    const SEEK_END: c_int = 2;
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    if host_vfprintf().is_none() || host_fflush_fn().is_none() {
      return;
    }

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut user_buffer = [0_u8; 8];
    let format = c_string("%f");

    // SAFETY: stream and user buffer are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IOLBF,
        as_size_t(user_buffer.len()),
      )
    };

    assert_eq!(setvbuf_status, 0);

    // SAFETY: stream and variadic argument satisfy `fprintf("%f", double)`.
    let write_status = unsafe { fprintf(stream, format.as_ptr(), 1.5_f64) };

    assert!(write_status > 0);

    // SAFETY: `fileno` expects a valid host stream handle.
    let stream_fd = unsafe { fileno(stream) };

    assert!(stream_fd >= 0);

    // SAFETY: probing file length through the host fd observes pre-fflush visibility.
    let flushed_len = unsafe { lseek(stream_fd, 0, SEEK_END) };

    assert_eq!(flushed_len, 0);

    // SAFETY: stream came from `tmpfile`.
    let flush_status = unsafe { fflush(stream) };

    assert_eq!(flush_status, 0);

    // SAFETY: probing file length after explicit flush is valid for the host fd.
    let flushed_len_after = unsafe { lseek(stream_fd, 0, SEEK_END) };

    assert_eq!(flushed_len_after, i64::from(write_status));

    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
  }

  #[test]
  fn formatted_output_requires_host_line_buffer_flush_ignores_float_without_newline() {
    let format = c_string("%f");
    let mut ap = OwnedSysVVaList::new(&[], &[1.25_f64], &[]);

    // SAFETY: format string and synthetic SysV va_list match `%f`.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "line-buffer analysis must not force flush for `%f` output without newline",
    );
  }

  #[test]
  fn formatted_output_requires_host_line_buffer_flush_ignores_mixed_float_and_string_without_newline()
   {
    let format = c_string("%e%s");
    let suffix = c_string("tail");
    let gp_values = [suffix.as_ptr().addr() as u64];
    let mut ap = OwnedSysVVaList::new(&gp_values, &[1.25_f64], &[]);

    // SAFETY: format string and synthetic SysV va_list match `%e%s`.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "line-buffer analysis must defer `%e%s` when neither conversion emits newline",
    );
  }

  #[test]
  fn formatted_output_requires_host_line_buffer_flush_ignores_dynamic_float_with_escaped_percent_without_newline()
   {
    let format = c_string("%*.*f%%");
    let gp_values = [9_u64, 3_u64];
    let mut ap = OwnedSysVVaList::new(&gp_values, &[1.25_f64], &[]);

    // SAFETY: format string and synthetic SysV va_list match `%*.*f%%`.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "line-buffer analysis must not force flush for `%*.*f%%` output without newline",
    );
  }

  #[test]
  fn formatted_output_requires_host_line_buffer_flush_ignores_fprintf_style_mixed_float_and_string_without_newline()
   {
    let format = c_string("%e%s");
    let suffix = c_string("tail");
    let gp_values = [suffix.as_ptr().addr() as u64];
    let mut ap = OwnedSysVVaList::with_offsets(16, 48, &gp_values, &[1.25_f64], &[]);

    // SAFETY: synthetic SysV va_list matches `fprintf(stream, format, double, char*)`.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "line-buffer analysis must defer `fprintf`-style `%e%s` without newline",
    );
  }

  #[test]
  fn formatted_output_requires_host_line_buffer_flush_ignores_fprintf_style_dynamic_float_without_newline()
   {
    let format = c_string("%*.*f%%");
    let gp_values = [9_u64, 3_u64];
    let mut ap = OwnedSysVVaList::with_offsets(16, 48, &gp_values, &[1.25_f64], &[]);

    // SAFETY: synthetic SysV va_list matches `fprintf(stream, format, int, int, double)`.
    let requires_flush =
      unsafe { formatted_output_requires_host_line_buffer_flush(format.as_ptr(), ap.as_mut_ptr()) };

    assert!(
      !requires_flush,
      "line-buffer analysis must defer `fprintf`-style `%*.*f%%` without newline",
    );
  }

  #[test]
  fn vfprintf_without_host_symbol_uses_standard_stream_fd_without_fileno() {
    let _guard = test_lock();
    let _host_vfprintf_guard = force_host_vfprintf_unavailable_for_tests();
    let _host_fileno_guard = force_host_fileno_unavailable_for_tests();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides `stdout` global stream pointer.
    let stdout_stream = unsafe { host_stdout };

    assert!(
      !stdout_stream.is_null(),
      "host stdout pointer must be available"
    );

    let format = c_string("i022-vfprintf-stdout-fallback\n");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    write_errno(0);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status = unsafe {
      vfprintf(
        stdout_stream,
        format.as_ptr(),
        ptr::addr_of_mut!(empty_ap).cast(),
      )
    };

    assert_eq!(
      write_status,
      c_int::try_from(format.as_bytes().len())
        .unwrap_or_else(|_| unreachable!("literal length fits"))
    );
    assert_eq!(read_errno(), 0);
  }

  #[test]
  fn vfprintf_without_host_symbol_uses_stderr_fd_without_fileno() {
    let _guard = test_lock();
    let _host_vfprintf_guard = force_host_vfprintf_unavailable_for_tests();
    let _host_fileno_guard = force_host_fileno_unavailable_for_tests();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides `stderr` global stream pointer.
    let stderr_stream = unsafe { host_stderr };

    assert!(
      !stderr_stream.is_null(),
      "host stderr pointer must be available"
    );

    let format = c_string("i022-vfprintf-stderr-fallback\n");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    write_errno(0);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status = unsafe {
      vfprintf(
        stderr_stream,
        format.as_ptr(),
        ptr::addr_of_mut!(empty_ap).cast(),
      )
    };

    assert_eq!(
      write_status,
      c_int::try_from(format.as_bytes().len())
        .unwrap_or_else(|_| unreachable!("literal length fits"))
    );
    assert_eq!(read_errno(), 0);
  }

  #[test]
  fn vfprintf_unbuffered_host_write_succeeds_without_host_fflush_symbol() {
    let _guard = test_lock();
    let _host_fflush_guard = force_host_fflush_unavailable_for_tests();

    clear_stream_registry_for_tests();

    // SAFETY: host libc provides `stdout` global stream pointer.
    let stdout_stream = unsafe { host_stdout };

    assert!(
      !stdout_stream.is_null(),
      "host stdout pointer must be available"
    );

    let mut user_buffer = [0_u8; 1];
    // SAFETY: stream and buffer pointers are valid for this call.
    let setvbuf_status = unsafe {
      setvbuf(
        stdout_stream,
        user_buffer.as_mut_ptr().cast::<c_char>(),
        _IONBF,
        as_size_t(0),
      )
    };

    assert_eq!(setvbuf_status, 0);

    let format = c_string("");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    write_errno(66);

    // SAFETY: stream and format are valid; format consumes no variadic args.
    let write_status = unsafe {
      vfprintf(
        stdout_stream,
        format.as_ptr(),
        ptr::addr_of_mut!(empty_ap).cast(),
      )
    };

    assert_eq!(write_status, 0);
    assert_eq!(read_errno(), 66);
  }

  #[test]
  fn vfprintf_without_host_symbol_unsupported_format_keeps_stream_reconfigurable() {
    let _guard = test_lock();
    let _host_vfprintf_guard = force_host_vfprintf_unavailable_for_tests();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());

    let mut initial_buffer = [0_u8; 8];
    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let format = c_string("%f");
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

    // SAFETY: stream and format are valid; `%f` is unsupported by internal subset.
    let write_status =
      unsafe { vfprintf(stream, format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(write_status, -1);
    assert_eq!(read_errno(), EINVAL);
    assert_eq!(
      buffering_snapshot_for_tests(stream).map(|(_mode, _size, _buffer_addr, io_active)| io_active),
      Some(false)
    );
    assert_eq!(host_backed_snapshot_for_tests(stream), Some(false));

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
    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(buffering_snapshot_for_tests(stream), None);
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
    let format = c_string("%m");
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

    assert!(write_status >= 0);
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

    assert_eq!(reconfigure_status, 0);
    assert_eq!(read_errno(), 73);
    assert_eq!(
      buffering_snapshot_for_tests(fresh_stream),
      Some((_IOFBF, fresh_buffer.len(), fresh_buffer_addr, false))
    );
    assert_eq!(host_backed_snapshot_for_tests(fresh_stream), Some(false));
    // SAFETY: stream came from `tmpfile`.
    let stale_close_status = unsafe { fclose(stale_stream) };
    // SAFETY: stream came from `tmpfile`.
    let fresh_close_status = unsafe { fclose(fresh_stream) };

    assert_eq!(stale_close_status, 0);
    assert_eq!(fresh_close_status, 0);
    assert_eq!(buffering_snapshot_for_tests(fresh_stream), None);
    assert_eq!(host_backed_snapshot_for_tests(fresh_stream), None);
  }

  #[test]
  fn setvbuf_allows_reconfiguration_when_host_stream_key_is_reused_after_vfprintf_failure() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    let path = c_string("/dev/null");
    let mode = c_string("r");
    let format = c_string("%");
    let mut fresh_buffer = [0_u8; 16];
    let fresh_buffer_addr = fresh_buffer.as_mut_ptr().cast::<c_char>().addr();
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

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

    // SAFETY: stream and format are valid; trailing `%` format is rejected.
    let write_status = unsafe {
      vfprintf(
        stale_stream,
        format.as_ptr(),
        ptr::addr_of_mut!(empty_ap).cast(),
      )
    };

    assert_eq!(write_status, -1);
    assert_ne!(read_errno(), 0);

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

    assert_eq!(reconfigure_status, 0);
    assert_eq!(read_errno(), 29);
    assert_eq!(
      buffering_snapshot_for_tests(fresh_stream),
      Some((_IOFBF, fresh_buffer.len(), fresh_buffer_addr, false))
    );
    assert_eq!(host_backed_snapshot_for_tests(fresh_stream), Some(false));
    // SAFETY: streams came from host allocation APIs.
    let stale_close_status = unsafe { fclose(stale_stream) };
    // SAFETY: stream came from `tmpfile`.
    let fresh_close_status = unsafe { fclose(fresh_stream) };

    assert_eq!(stale_close_status, 0);
    assert_eq!(fresh_close_status, 0);
    assert_eq!(buffering_snapshot_for_tests(fresh_stream), None);
    assert_eq!(host_backed_snapshot_for_tests(fresh_stream), None);
  }

  #[test]
  fn setvbuf_allows_reconfiguration_when_stale_host_entry_lacks_identity() {
    let _guard = test_lock();

    clear_stream_registry_for_tests();

    // SAFETY: host libc returns either a valid stream pointer or null.
    let stream = unsafe { tmpfile() };

    assert!(!stream.is_null());
    assert!(
      read_host_stream_identity(stream).is_some(),
      "tmpfile stream should expose a stable host identity in this environment",
    );

    let mut replacement_buffer = [0_u8; 16];
    let replacement_addr = replacement_buffer.as_mut_ptr().cast::<c_char>().addr();
    let key = stream_key(stream);
    let mut registry = stream_registry_guard();
    let stream_state = stream_state_mut_or_insert(&mut registry, key);

    stream_state.io_active = true;
    stream_state.host_backed_io = true;
    stream_state.host_stream_identity = None;
    stream_state.explicit_buffering_config = true;
    stream_state.buffering_mode = _IONBF;
    stream_state.buffer_size = 0;
    stream_state.user_buffer_addr = 0;
    drop(registry);

    write_errno(53);

    // SAFETY: stream and replacement buffer pointers are valid for this call.
    let reconfigure_status = unsafe {
      setvbuf(
        stream,
        replacement_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(replacement_buffer.len()),
      )
    };

    assert_eq!(reconfigure_status, 0);
    assert_eq!(read_errno(), 53);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOFBF, replacement_buffer.len(), replacement_addr, false))
    );
    assert_eq!(host_backed_snapshot_for_tests(stream), Some(false));
    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(buffering_snapshot_for_tests(stream), None);
    assert_eq!(host_backed_snapshot_for_tests(stream), None);
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

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(buffering_snapshot_for_tests(stream), None);
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

    assert_eq!(second_status, 0);
    assert_eq!(
      buffering_snapshot_for_tests(stream),
      Some((_IOLBF, replacement_buffer.len(), replacement_addr, false))
    );
    // SAFETY: stream came from `tmpfile`.
    let close_status = unsafe { fclose(stream) };

    assert_eq!(close_status, 0);
    assert_eq!(buffering_snapshot_for_tests(stream), None);
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
  fn vprintf_without_host_stdout_uses_internal_stdout_fd_fallback() {
    let _guard = test_lock();
    let _stdout_guard = force_host_stdout_unavailable_for_tests();

    clear_stream_registry_for_tests();

    let format = c_string("");
    let mut empty_ap = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area: ptr::null_mut(),
      reg_save_area: ptr::null_mut(),
    };

    write_errno(83);

    // SAFETY: format and va_list pointers are valid and format consumes no args.
    let write_status = unsafe { vprintf(format.as_ptr(), ptr::addr_of_mut!(empty_ap).cast()) };

    assert_eq!(write_status, 0);
    assert_eq!(read_errno(), 83);
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
    let format = c_string("%m");
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

    assert!(write_status >= 0);
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
    let format = c_string("%m");
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

    assert!(write_status >= 0);
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
