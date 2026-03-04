//! Raw Linux syscall entry points for `x86_64`.
//!
//! These helpers execute the `syscall` instruction directly and return the raw
//! kernel return value. Use [`crate::syscall::decode_raw`] to convert the raw
//! return value into `Result`.

use crate::abi::types::c_long;
use core::arch::asm;

/// Performs a raw Linux syscall with zero arguments.
///
/// # Safety
/// The caller must ensure `number` is a valid Linux syscall number for
/// `x86_64`, and that invoking it at the call site is safe for process state.
#[must_use]
pub unsafe fn syscall0(number: c_long) -> c_long {
  // SAFETY: caller upholds syscall contract for `number` and side effects.
  unsafe { syscall6(number, 0, 0, 0, 0, 0, 0) }
}

/// Performs a raw Linux syscall with one argument.
///
/// # Safety
/// The caller must ensure the syscall number and argument satisfy the Linux
/// syscall ABI contract for `x86_64`.
#[must_use]
pub unsafe fn syscall1(number: c_long, arg0: c_long) -> c_long {
  // SAFETY: caller upholds syscall contract for `number` and all arguments.
  unsafe { syscall6(number, arg0, 0, 0, 0, 0, 0) }
}

/// Performs a raw Linux syscall with two arguments.
///
/// # Safety
/// The caller must ensure the syscall number and arguments satisfy the Linux
/// syscall ABI contract for `x86_64`.
#[must_use]
pub unsafe fn syscall2(number: c_long, arg0: c_long, arg1: c_long) -> c_long {
  // SAFETY: caller upholds syscall contract for `number` and all arguments.
  unsafe { syscall6(number, arg0, arg1, 0, 0, 0, 0) }
}

/// Performs a raw Linux syscall with three arguments.
///
/// # Safety
/// The caller must ensure the syscall number and arguments satisfy the Linux
/// syscall ABI contract for `x86_64`.
#[must_use]
pub unsafe fn syscall3(number: c_long, arg0: c_long, arg1: c_long, arg2: c_long) -> c_long {
  // SAFETY: caller upholds syscall contract for `number` and all arguments.
  unsafe { syscall6(number, arg0, arg1, arg2, 0, 0, 0) }
}

/// Performs a raw Linux syscall with four arguments.
///
/// # Safety
/// The caller must ensure the syscall number and arguments satisfy the Linux
/// syscall ABI contract for `x86_64`.
#[must_use]
pub unsafe fn syscall4(
  number: c_long,
  arg0: c_long,
  arg1: c_long,
  arg2: c_long,
  arg3: c_long,
) -> c_long {
  // SAFETY: caller upholds syscall contract for `number` and all arguments.
  unsafe { syscall6(number, arg0, arg1, arg2, arg3, 0, 0) }
}

/// Performs a raw Linux syscall with five arguments.
///
/// # Safety
/// The caller must ensure the syscall number and arguments satisfy the Linux
/// syscall ABI contract for `x86_64`.
#[must_use]
pub unsafe fn syscall5(
  number: c_long,
  arg0: c_long,
  arg1: c_long,
  arg2: c_long,
  arg3: c_long,
  arg4: c_long,
) -> c_long {
  // SAFETY: caller upholds syscall contract for `number` and all arguments.
  unsafe { syscall6(number, arg0, arg1, arg2, arg3, arg4, 0) }
}

/// Performs a raw Linux syscall with six arguments.
///
/// The Linux `x86_64` syscall register mapping is:
/// - `rax`: syscall number
/// - `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9`: arguments 0 through 5
///
/// # Safety
/// The caller must ensure:
/// - `number` and argument values satisfy the target syscall contract.
/// - Pointer arguments are valid for the required reads/writes.
/// - Aliasing and lifetime requirements expected by the kernel are upheld.
#[must_use]
pub unsafe fn syscall6(
  number: c_long,
  arg0: c_long,
  arg1: c_long,
  arg2: c_long,
  arg3: c_long,
  arg4: c_long,
  arg5: c_long,
) -> c_long {
  let mut return_value = number;

  // SAFETY: register mapping follows Linux x86_64 syscall ABI; caller guarantees
  // syscall number/arguments are valid for the selected syscall.
  unsafe {
    asm!(
      "syscall",
      inlateout("rax") return_value,
      in("rdi") arg0,
      in("rsi") arg1,
      in("rdx") arg2,
      in("r10") arg3,
      in("r8") arg4,
      in("r9") arg5,
      lateout("rcx") _,
      lateout("r11") _,
      options(nostack),
    );
  }

  return_value
}
