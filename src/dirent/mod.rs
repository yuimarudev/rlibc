//! Directory stream (`dirent`) C ABI interfaces for Linux `x86_64`.
//!
//! This module implements:
//! - `opendir`
//! - `readdir`
//! - `closedir`
//! - `rewinddir`
//!
//! The implementation uses Linux syscalls directly (`openat`, `getdents64`,
//! `close`, `lseek`) and stores stream state in an opaque handle returned from
//! `opendir`.

use crate::abi::errno::{EFAULT, EINVAL, EIO};
use crate::abi::types::{c_char, c_int, c_long, c_ulong};
use crate::errno::set_errno;
use crate::fs::AT_FDCWD;
use crate::syscall::{syscall1, syscall3};
use core::ptr;

const SYS_CLOSE: c_long = 3;
const SYS_LSEEK: c_long = 8;
const SYS_GETDENTS64: c_long = 217;
const SYS_OPENAT: c_long = 257;
const O_DIRECTORY: c_int = 0o200_000;
const O_CLOEXEC: c_int = 0o2_000_000;
const O_RDONLY: c_int = 0;
const SEEK_SET: c_int = 0;
const DIRENT_BUFFER_SIZE: usize = 4096;
const DIRENT_NAME_CAPACITY: usize = 256;
const LINUX_DIRENT64_FIXED_SIZE: usize = 19;

/// C ABI-compatible directory entry object returned by `readdir`.
///
/// Lifetime contract:
/// - The returned pointer from `readdir` points to storage owned by the
///   associated stream handle.
/// - The storage becomes invalid after `closedir` on that handle.
/// - A subsequent `readdir` call on the same handle may overwrite this value.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Dirent {
  /// Inode number of the directory entry.
  pub d_ino: c_ulong,
  /// Offset cookie for the next directory read position.
  pub d_off: c_long,
  /// Record size (in bytes) in the source kernel entry stream.
  pub d_reclen: u16,
  /// Directory entry type (`DT_*` value from kernel dirent record).
  pub d_type: u8,
  /// NUL-terminated entry name bytes.
  pub d_name: [c_char; DIRENT_NAME_CAPACITY],
}

/// Opaque directory stream handle used by `opendir`/`readdir`/`closedir`.
///
/// This type is intentionally zero-sized for external callers. The actual
/// state is stored in an internal allocation whose pointer value is cast to
/// `*mut Dir`.
#[repr(C)]
pub struct Dir {
  _private: [u8; 0],
}

struct DirStream {
  fd: c_int,
  cursor: usize,
  filled: usize,
  buffer: [u8; DIRENT_BUFFER_SIZE],
  entry: Dirent,
}

fn ptr_arg<T>(ptr: *const T) -> c_long {
  c_long::try_from(ptr.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit c_long on x86_64"))
}

fn mut_ptr_arg<T>(ptr: *mut T) -> c_long {
  ptr_arg(ptr.cast_const())
}

fn usize_arg(value: usize) -> c_long {
  c_long::try_from(value).unwrap_or_else(|_| unreachable!("usize must fit c_long on x86_64"))
}

fn errno_from_raw(raw: c_long) -> c_int {
  c_int::try_from(-raw).unwrap_or(c_int::MAX)
}

fn reset_entry_name(entry: &mut Dirent) {
  for slot in &mut entry.d_name {
    *slot = 0;
  }
}

fn parse_u64(bytes: &[u8]) -> u64 {
  u64::from_ne_bytes(
    bytes
      .try_into()
      .unwrap_or_else(|_| unreachable!("slice length is fixed")),
  )
}

fn parse_i64(bytes: &[u8]) -> i64 {
  i64::from_ne_bytes(
    bytes
      .try_into()
      .unwrap_or_else(|_| unreachable!("slice length is fixed")),
  )
}

fn parse_u16(bytes: &[u8]) -> u16 {
  u16::from_ne_bytes(
    bytes
      .try_into()
      .unwrap_or_else(|_| unreachable!("slice length is fixed")),
  )
}

fn fill_user_entry(
  stream: &mut DirStream,
  ino: u64,
  off: i64,
  reclen: u16,
  d_type: u8,
  name_bytes: &[u8],
) {
  stream.entry.d_ino =
    c_ulong::try_from(ino).unwrap_or_else(|_| unreachable!("u64 must fit c_ulong on x86_64"));
  stream.entry.d_off =
    c_long::try_from(off).unwrap_or_else(|_| unreachable!("i64 must fit c_long on x86_64"));
  stream.entry.d_reclen = reclen;
  stream.entry.d_type = d_type;
  reset_entry_name(&mut stream.entry);

  for (index, byte) in name_bytes.iter().copied().enumerate() {
    stream.entry.d_name[index] = c_char::from_ne_bytes([byte]);
  }
}

fn refill(stream: &mut DirStream) -> Result<usize, c_int> {
  // SAFETY: arguments follow Linux `getdents64` ABI; `buffer` is valid writable memory.
  let raw = unsafe {
    syscall3(
      SYS_GETDENTS64,
      c_long::from(stream.fd),
      mut_ptr_arg(stream.buffer.as_mut_ptr()),
      usize_arg(DIRENT_BUFFER_SIZE),
    )
  };

  if raw < 0 {
    return Err(errno_from_raw(raw));
  }

  let bytes_read = usize::try_from(raw)
    .unwrap_or_else(|_| unreachable!("non-negative getdents64 result must fit usize"));

  stream.cursor = 0;
  stream.filled = bytes_read;

  Ok(bytes_read)
}

fn next_entry(stream: &mut DirStream) -> Result<*mut Dirent, c_int> {
  if stream.cursor >= stream.filled {
    let bytes_read = refill(stream)?;

    if bytes_read == 0 {
      return Ok(ptr::null_mut());
    }
  }

  let available = stream.filled - stream.cursor;

  if available < LINUX_DIRENT64_FIXED_SIZE {
    return Err(EIO);
  }

  let record_start = stream.cursor;
  let record = &stream.buffer[record_start..stream.filled];
  let record_len = usize::from(parse_u16(&record[16..18]));

  if record_len < LINUX_DIRENT64_FIXED_SIZE || record_len > available {
    return Err(EIO);
  }

  let (ino, off, reclen, d_type, name_copy, copy_len) = {
    let record = &stream.buffer[record_start..record_start + record_len];
    let ino = parse_u64(&record[0..8]);
    let off = parse_i64(&record[8..16]);
    let reclen = parse_u16(&record[16..18]);
    let d_type = record[18];
    let raw_name_bytes = &record[LINUX_DIRENT64_FIXED_SIZE..record_len];
    let raw_name_len = raw_name_bytes
      .iter()
      .position(|byte| *byte == 0)
      .unwrap_or(raw_name_bytes.len());
    let copy_len = raw_name_len.min(DIRENT_NAME_CAPACITY.saturating_sub(1));
    let mut name_copy = [0_u8; DIRENT_NAME_CAPACITY];

    name_copy[0..copy_len].copy_from_slice(&raw_name_bytes[0..copy_len]);

    (ino, off, reclen, d_type, name_copy, copy_len)
  };

  fill_user_entry(stream, ino, off, reclen, d_type, &name_copy[0..copy_len]);
  stream.cursor += record_len;

  Ok(ptr::from_mut(&mut stream.entry))
}

/// C ABI entry point for `opendir`.
///
/// Opens a directory stream for `path`.
///
/// Returns:
/// - non-null stream handle on success
/// - null on failure and sets thread-local `errno`
///
/// # Safety
/// - `path` must point to a readable NUL-terminated C string.
///
/// # Errors
/// - Returns null and sets `errno = EFAULT` when `path` is null.
/// - Returns null and sets `errno` to the kernel-reported open error for
///   other failures (for example `ENOENT`, `ENOTDIR`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendir(path: *const c_char) -> *mut Dir {
  if path.is_null() {
    set_errno(EFAULT);

    return ptr::null_mut();
  }

  let flags = c_long::from(O_RDONLY | O_DIRECTORY | O_CLOEXEC);
  // SAFETY: syscall arguments follow Linux `openat` ABI.
  let raw = unsafe { syscall3(SYS_OPENAT, c_long::from(AT_FDCWD), ptr_arg(path), flags) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return ptr::null_mut();
  }

  let fd = c_int::try_from(raw).unwrap_or_else(|_| unreachable!("fd must fit c_int"));
  let stream = Box::new(DirStream {
    fd,
    cursor: 0,
    filled: 0,
    buffer: [0; DIRENT_BUFFER_SIZE],
    entry: Dirent {
      d_ino: 0,
      d_off: 0,
      d_reclen: 0,
      d_type: 0,
      d_name: [0; DIRENT_NAME_CAPACITY],
    },
  });

  Box::into_raw(stream).cast::<Dir>()
}

/// C ABI entry point for `readdir`.
///
/// Reads the next entry from `dir`.
///
/// Returns:
/// - non-null pointer to stream-owned `Dirent` storage on success
/// - null on end-of-stream (without modifying `errno`)
/// - null on failure and sets thread-local `errno`
///
/// # Safety
/// - `dir` must be a live handle previously returned by `opendir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readdir(dir: *mut Dir) -> *mut Dirent {
  if dir.is_null() {
    set_errno(EINVAL);

    return ptr::null_mut();
  }

  // SAFETY: caller provides a live `opendir` handle.
  let stream = unsafe { &mut *dir.cast::<DirStream>() };

  match next_entry(stream) {
    Ok(entry) => entry,
    Err(errno_value) => {
      set_errno(errno_value);

      ptr::null_mut()
    }
  }
}

/// C ABI entry point for `closedir`.
///
/// Closes `dir` and releases its stream state.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets thread-local `errno`
///
/// # Safety
/// - `dir` must be null or a live handle previously returned by `opendir`.
/// - Passing an already-closed handle is undefined.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn closedir(dir: *mut Dir) -> c_int {
  if dir.is_null() {
    set_errno(EINVAL);

    return -1;
  }

  // SAFETY: caller provides ownership of a live `opendir` handle.
  let stream = unsafe { Box::from_raw(dir.cast::<DirStream>()) };
  // SAFETY: syscall arguments follow Linux `close` ABI.
  let raw = unsafe { syscall1(SYS_CLOSE, c_long::from(stream.fd)) };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  0
}

/// C ABI entry point for `rewinddir`.
///
/// Resets stream position to the beginning of the directory.
///
/// On failure, this function sets thread-local `errno`.
///
/// # Safety
/// - `dir` must be a live handle previously returned by `opendir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rewinddir(dir: *mut Dir) {
  if dir.is_null() {
    set_errno(EINVAL);

    return;
  }

  // SAFETY: caller provides a live `opendir` handle.
  let stream = unsafe { &mut *dir.cast::<DirStream>() };
  // SAFETY: syscall arguments follow Linux `lseek` ABI.
  let raw = unsafe {
    syscall3(
      SYS_LSEEK,
      c_long::from(stream.fd),
      c_long::from(0_u8),
      c_long::from(SEEK_SET),
    )
  };

  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return;
  }

  stream.cursor = 0;
  stream.filled = 0;
}
