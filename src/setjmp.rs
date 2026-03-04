//! `setjmp`/`longjmp` non-local jump primitives for `x86_64` Linux.
//!
//! This module provides the minimal C ABI surface for issue `I047`:
//! - `setjmp`
//! - `longjmp`
//!
//! Implementation scope is intentionally limited to `x86_64-unknown-linux-gnu`.
//! The saved context includes SysV callee-saved general-purpose registers, the
//! resumed stack pointer, and the saved return address.

use crate::abi::types::{c_int, c_long};
use core::arch::asm;

/// Storage backing `setjmp`/`longjmp` execution context on `x86_64`.
///
/// Contract:
/// - This type is ABI-facing storage used by `setjmp` and `longjmp`.
/// - Callers must treat values as opaque and initialize them only via `setjmp`.
/// - One buffer must not be used concurrently from multiple threads.
///
/// Layout note:
/// - This storage must stay layout-compatible with the internal saved-register
///   context used by assembly restore logic.
pub type jmp_buf = [c_long; 8];

#[repr(C)]
struct JumpContext {
  rbx: c_long,
  rbp: c_long,
  r12: c_long,
  r13: c_long,
  r14: c_long,
  r15: c_long,
  rsp: c_long,
  rip: c_long,
}

const _: [(); core::mem::size_of::<JumpContext>()] = [(); core::mem::size_of::<jmp_buf>()];
const _: [(); core::mem::align_of::<JumpContext>()] = [(); core::mem::align_of::<jmp_buf>()];

const fn as_context_mut(env: *mut jmp_buf) -> *mut JumpContext {
  env.cast::<JumpContext>()
}

const fn as_context(env: *const jmp_buf) -> *const JumpContext {
  env.cast::<JumpContext>()
}

const fn normalize_longjmp_value(value: c_int) -> c_int {
  if value == 0 { 1 } else { value }
}

/// Saves the current execution context in `env`.
///
/// Return value contract:
/// - Returns `0` when called directly.
/// - Returns non-zero when resumed by [`longjmp`].
///
/// # Safety
/// - `env` must be a valid, writable pointer to `jmp_buf` storage.
/// - The storage must remain valid until a matching `longjmp` path is no longer
///   possible.
/// - Using an `env` captured in a frame that has already returned is undefined.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setjmp(env: *mut jmp_buf) -> c_int {
  let context = as_context_mut(env);
  let return_code: c_int;

  // SAFETY: caller guarantees `context` points to writable `jmp_buf` storage.
  // Register save/restore layout matches this module's `x86_64` context layout.
  unsafe {
    asm!(
      "mov [rdi + 0x00], rbx",
      "mov [rdi + 0x08], rbp",
      "mov [rdi + 0x10], r12",
      "mov [rdi + 0x18], r13",
      "mov [rdi + 0x20], r14",
      "mov [rdi + 0x28], r15",
      "lea rax, [rsp + 8]",
      "mov [rdi + 0x30], rax",
      "mov rax, [rsp]",
      "mov [rdi + 0x38], rax",
      "xor eax, eax",
      in("rdi") context,
      lateout("eax") return_code,
      options(nostack, preserves_flags),
    );
  }

  return_code
}

/// Restores a context captured by [`setjmp`] and resumes execution there.
///
/// Value contract:
/// - Resumed `setjmp` returns `value` when `value != 0`.
/// - Resumed `setjmp` returns `1` when `value == 0`.
/// - C header declaration uses `RLIBC_NORETURN` (C++11/MSVC/C11/GNU-attribute fallback)
///   to reflect diverging control flow for C-family callers.
///
/// # Safety
/// - `env` must point to a valid context previously initialized by `setjmp`.
/// - The target stack frame represented by `env` must still be alive.
/// - Jumping over Rust frames with active destructors is undefined behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn longjmp(env: *const jmp_buf, value: c_int) -> ! {
  let context = as_context(env);
  let resume_value = normalize_longjmp_value(value);

  // SAFETY: caller upholds that `context` is a live setjmp-saved context.
  // This assembly restores saved non-volatile registers and transfers control
  // to saved RIP with resumed return value in `eax`.
  unsafe {
    asm!(
      "mov rbx, [rdi + 0x00]",
      "mov rbp, [rdi + 0x08]",
      "mov r12, [rdi + 0x10]",
      "mov r13, [rdi + 0x18]",
      "mov r14, [rdi + 0x20]",
      "mov r15, [rdi + 0x28]",
      "mov rsp, [rdi + 0x30]",
      "mov rdx, [rdi + 0x38]",
      "mov eax, esi",
      "jmp rdx",
      in("rdi") context,
      in("esi") resume_value,
      options(noreturn),
    );
  }
}

#[cfg(test)]
mod tests {
  use crate::abi::types::c_int;

  use super::normalize_longjmp_value;

  #[test]
  fn normalize_longjmp_value_maps_zero_to_one() {
    assert_eq!(normalize_longjmp_value(0), 1);
  }

  #[test]
  fn normalize_longjmp_value_keeps_non_zero_values() {
    for value in [1, -7, c_int::MAX, c_int::MIN] {
      assert_eq!(normalize_longjmp_value(value), value);
    }
  }
}
