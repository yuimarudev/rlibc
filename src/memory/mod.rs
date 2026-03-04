//! Memory-related C ABI functions.
//!
//! These functions provide the `libc`-style memory primitives exported with C
//! symbol names. They intentionally operate on raw pointers and follow C
//! contracts, not Rust slice safety contracts.

use crate::abi::types::{c_int, size_t};
use core::ffi::c_void;

/// Converts `size_t` into `usize` on the current primary target.
///
/// # Panics
/// Panics only if `size_t` cannot fit in `usize`, which should not happen for
/// the current `x86_64` Linux target profile.
fn len_from_size_t(n: size_t) -> usize {
  usize::try_from(n)
    .unwrap_or_else(|_| unreachable!("size_t does not fit into usize on this target"))
}

const fn fill_bytes(bytes: *mut u8, byte: u8, len: usize) {
  // SAFETY: We write exactly `len` bytes in-bounds; callers provide a valid writable region.
  unsafe {
    let mut idx = 0usize;

    while idx < len {
      bytes.add(idx).write(byte);
      idx += 1;
    }
  }
}

const fn copy_bytes_forward(dst: *mut u8, src: *const u8, len: usize) {
  // SAFETY: Callers provide valid readable/writable regions for `len` bytes.
  unsafe {
    let mut idx = 0usize;

    while idx < len {
      dst.add(idx).write(src.add(idx).read());
      idx += 1;
    }
  }
}

const fn copy_bytes_backward(dst: *mut u8, src: *const u8, len: usize) {
  // SAFETY: Callers provide valid readable/writable regions for `len` bytes.
  unsafe {
    let mut remaining = len;

    while remaining != 0 {
      let idx = remaining - 1;

      dst.add(idx).write(src.add(idx).read());
      remaining -= 1;
    }
  }
}

/// C ABI entry point for `memmove`.
///
/// Copies `n` bytes from `src` to `dst`, correctly handling overlap.
/// Returns `dst`.
///
/// Contract notes:
/// - Copy direction is selected to preserve bytes under overlap
///   (`dst` inside source range uses backward copy, otherwise forward copy).
/// - Adjacent non-overlapping ranges (for example `dst == src + n`) are copied
///   as non-overlap cases.
/// - Implementation is byte-wise and self-contained so the exported symbol does
///   not depend on any external `memmove` runtime symbol.
///
/// # Safety
/// - If `n == 0`, pointers may be null and no memory access is performed.
/// - If `n > 0`, caller must provide readable/writable regions for `n` bytes.
/// - `src` and `dst` may overlap; bytes are copied as if from a temporary buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: size_t) -> *mut c_void {
  let len = len_from_size_t(n);

  if len == 0 || dst.cast_const() == src {
    return dst;
  }

  let dst_bytes = dst.cast::<u8>();
  let src_bytes = src.cast::<u8>();
  let dst_addr = dst_bytes.addr();
  let src_addr = src_bytes.addr();

  // Avoid `ptr::copy` here to prevent symbol-level self-recursion of exported `memmove`.
  // We copy forward unless destination starts inside source range, where backward copy is required.
  if dst_addr <= src_addr || (dst_addr - src_addr) >= len {
    copy_bytes_forward(dst_bytes, src_bytes, len);
  } else {
    copy_bytes_backward(dst_bytes, src_bytes, len);
  }

  dst
}

/// C ABI entry point for `memcpy`.
///
/// Copies `n` bytes from `src` to `dst` and returns `dst`.
///
/// # Safety
/// - For overlapping regions, behavior is undefined by C.
/// - This implementation currently reuses `memmove` logic.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: size_t) -> *mut c_void {
  // SAFETY: Safety requirements are identical to `memmove`.
  unsafe { memmove(dst, src, n) }
}

/// C ABI entry point for `memset`.
///
/// Writes `n` bytes of `c as unsigned char` to `s` and returns `s`.
///
/// # Safety
/// - If `n == 0`, pointer may be null and no memory access is performed.
/// - If `n > 0`, caller must provide a writable region for `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: size_t) -> *mut c_void {
  let len = len_from_size_t(n);

  if len == 0 {
    return s;
  }

  let byte = c.to_le_bytes()[0];
  let bytes = s.cast::<u8>();

  fill_bytes(bytes, byte, len);

  s
}

/// C ABI entry point for `memcmp`.
///
/// Compares the first `n` bytes from `left` and `right`.
/// Returns:
/// - `< 0` if `left` is lexicographically smaller,
/// - `0` if equal,
/// - `> 0` if greater.
///
/// # Safety
/// - If `n == 0`, no memory access is performed.
/// - If `n > 0`, caller must provide readable regions for `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(left: *const c_void, right: *const c_void, n: size_t) -> c_int {
  let len = len_from_size_t(n);

  if len == 0 {
    return 0;
  }

  let left_bytes = left.cast::<u8>();
  let right_bytes = right.cast::<u8>();

  // SAFETY: We only read indices in `0..len`; callers provide readable regions for `len > 0`.
  unsafe {
    let mut idx = 0usize;

    while idx < len {
      let left_byte = left_bytes.add(idx).read();
      let right_byte = right_bytes.add(idx).read();

      if left_byte != right_byte {
        return c_int::from(left_byte) - c_int::from(right_byte);
      }

      idx += 1;
    }
  }

  0
}
