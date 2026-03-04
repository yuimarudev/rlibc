//! Minimal floating-point environment (`fenv`) entry points.
//!
//! This module provides a baseline `fe*` surface for the primary target
//! (`x86_64-unknown-linux-gnu`).
//!
//! Current model notes:
//! - Environment state is tracked per-thread.
//! - Rounding mode and exception flags are modeled and preserved.
//! - Trap-enable state and architecture-specific extension APIs are out of
//!   scope for this issue.

use crate::abi::types::c_int;
use core::cell::Cell;

/// Invalid operation exception mask.
pub const FE_INVALID: c_int = 0x01;
/// Divide-by-zero exception mask.
pub const FE_DIVBYZERO: c_int = 0x04;
/// Overflow exception mask.
pub const FE_OVERFLOW: c_int = 0x08;
/// Underflow exception mask.
pub const FE_UNDERFLOW: c_int = 0x10;
/// Inexact result exception mask.
pub const FE_INEXACT: c_int = 0x20;
/// Mask of all baseline exceptions supported by this implementation.
pub const FE_ALL_EXCEPT: c_int = 0x3d;
/// Round-to-nearest mode.
pub const FE_TONEAREST: c_int = 0x0000;
/// Round-toward-negative-infinity mode.
pub const FE_DOWNWARD: c_int = 0x0400;
/// Round-toward-positive-infinity mode.
pub const FE_UPWARD: c_int = 0x0800;
/// Round-toward-zero mode.
pub const FE_TOWARDZERO: c_int = 0x0c00;
/// Sentinel pointer that requests the default floating-point environment.
///
/// This mirrors the conventional C macro contract:
/// `#define FE_DFL_ENV ((const fenv_t *) -1)`.
pub const FE_DFL_ENV: *const fenv_t = usize::MAX as *const fenv_t;

/// Floating-point exception bitmask type for Linux `x86_64`.
pub type fexcept_t = u16;

/// Opaque floating-point environment snapshot used by `fe*env` APIs.
///
/// ABI notes:
/// - Size is fixed to 32 bytes on Linux `x86_64`.
/// - Alignment is fixed to 4 bytes.
/// - Fields are internal implementation details and intentionally opaque.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct fenv_t {
  words: [u32; 8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FenvState {
  except_flags: c_int,
  round_mode: c_int,
}

impl FenvState {
  const DEFAULT: Self = Self {
    except_flags: 0,
    round_mode: FE_TONEAREST,
  };
}

thread_local! {
  static FENV_STATE: Cell<FenvState> = const { Cell::new(FenvState::DEFAULT) };
}

const fn normalize_exception_mask(excepts: c_int) -> c_int {
  excepts & FE_ALL_EXCEPT
}

const fn has_unsupported_exception_bits(excepts: c_int) -> bool {
  excepts & !FE_ALL_EXCEPT != 0
}

const fn is_valid_round_mode(round_mode: c_int) -> bool {
  matches!(
    round_mode,
    FE_TONEAREST | FE_DOWNWARD | FE_UPWARD | FE_TOWARDZERO
  )
}

fn with_state<T>(f: impl FnOnce(FenvState) -> T) -> T {
  FENV_STATE.with(|state| f(state.get()))
}

fn update_state(f: impl FnOnce(FenvState) -> FenvState) {
  FENV_STATE.with(|state| state.set(f(state.get())));
}

fn encode_state(state: FenvState) -> fenv_t {
  let round_mode = u32::try_from(state.round_mode)
    .unwrap_or_else(|_| unreachable!("round mode must be non-negative"));
  let except_flags = u32::try_from(state.except_flags)
    .unwrap_or_else(|_| unreachable!("exception flags must be non-negative"));
  let mut words = [0_u32; 8];

  words[0] = round_mode;
  words[1] = except_flags;

  fenv_t { words }
}

fn has_nonzero_reserved_words(env: &fenv_t) -> bool {
  env.words[2..].iter().any(|word| *word != 0)
}

fn decode_state(env: &fenv_t) -> Option<FenvState> {
  let round_mode = c_int::try_from(env.words[0]).ok()?;

  if !is_valid_round_mode(round_mode) {
    return None;
  }

  if has_nonzero_reserved_words(env) {
    return None;
  }

  let except_flags = c_int::try_from(env.words[1]).ok()?;

  if has_unsupported_exception_bits(except_flags) {
    return None;
  }

  Some(FenvState {
    except_flags: normalize_exception_mask(except_flags),
    round_mode,
  })
}

fn is_default_environment_ptr(env: *const fenv_t) -> bool {
  env == FE_DFL_ENV
}

/// C ABI entry point for `feclearexcept`.
///
/// Clears the floating-point exception flags selected by `excepts`.
///
/// Returns:
/// - `0` on success
/// - non-zero when `excepts` includes unsupported exception bits
#[unsafe(no_mangle)]
pub extern "C" fn feclearexcept(excepts: c_int) -> c_int {
  if has_unsupported_exception_bits(excepts) {
    return 1;
  }

  let clear_mask = normalize_exception_mask(excepts);

  update_state(|mut state| {
    state.except_flags &= !clear_mask;
    state
  });

  0
}

/// C ABI entry point for `fegetexceptflag`.
///
/// Stores selected floating-point exception flags into `flagp`.
///
/// Returns:
/// - `0` on success
/// - non-zero when `flagp` is null or `excepts` includes unsupported bits
///
/// # Safety
/// - `flagp` must be valid for writing one `fexcept_t` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fegetexceptflag(flagp: *mut fexcept_t, excepts: c_int) -> c_int {
  if flagp.is_null() || has_unsupported_exception_bits(excepts) {
    return 1;
  }

  let capture_mask = normalize_exception_mask(excepts);
  let captured = with_state(|state| state.except_flags & capture_mask);
  let captured = fexcept_t::try_from(captured)
    .unwrap_or_else(|_| unreachable!("exception mask must fit fexcept_t"));

  // SAFETY: non-null was checked and caller upholds pointer validity.
  unsafe {
    flagp.write(captured);
  }

  0
}

/// C ABI entry point for `feraiseexcept`.
///
/// Raises (sets) floating-point exception flags selected by `excepts`.
///
/// Returns:
/// - `0` on success
/// - non-zero when `excepts` includes unsupported exception bits
#[unsafe(no_mangle)]
pub extern "C" fn feraiseexcept(excepts: c_int) -> c_int {
  if has_unsupported_exception_bits(excepts) {
    return 1;
  }

  let raise_mask = normalize_exception_mask(excepts);

  update_state(|mut state| {
    state.except_flags |= raise_mask;
    state
  });

  0
}

/// C ABI entry point for `fesetexceptflag`.
///
/// Updates floating-point exception flags from `*flagp`, restricted to
/// `excepts`.
///
/// Returns:
/// - `0` on success
/// - non-zero when `flagp` is null or `excepts` includes unsupported bits
///
/// # Safety
/// - `flagp` must be valid for reading one `fexcept_t` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fesetexceptflag(flagp: *const fexcept_t, excepts: c_int) -> c_int {
  if flagp.is_null() || has_unsupported_exception_bits(excepts) {
    return 1;
  }

  let replace_mask = normalize_exception_mask(excepts);
  // SAFETY: non-null was checked and caller upholds pointer validity.
  let incoming = unsafe { c_int::from(flagp.read()) } & replace_mask;

  update_state(|mut state| {
    state.except_flags = (state.except_flags & !replace_mask) | incoming;
    state
  });

  0
}

/// C ABI entry point for `fetestexcept`.
///
/// Returns the subset of currently raised exception flags selected by
/// `excepts`.
#[unsafe(no_mangle)]
pub extern "C" fn fetestexcept(excepts: c_int) -> c_int {
  let test_mask = normalize_exception_mask(excepts);

  with_state(|state| state.except_flags & test_mask)
}

/// C ABI entry point for `fegetround`.
///
/// Returns the current rounding mode.
#[unsafe(no_mangle)]
pub extern "C" fn fegetround() -> c_int {
  with_state(|state| state.round_mode)
}

/// C ABI entry point for `fesetround`.
///
/// Sets the current rounding mode to `round_mode`.
///
/// Returns:
/// - `0` on success
/// - non-zero when `round_mode` is unsupported
#[unsafe(no_mangle)]
pub extern "C" fn fesetround(round_mode: c_int) -> c_int {
  if !is_valid_round_mode(round_mode) {
    return 1;
  }

  update_state(|mut state| {
    state.round_mode = round_mode;
    state
  });

  0
}

/// C ABI entry point for `fegetenv`.
///
/// Writes the current floating-point environment into `*envp`.
///
/// Returns:
/// - `0` on success
/// - non-zero when `envp` is null
///
/// # Safety
/// - `envp` must be valid for writing one `fenv_t` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fegetenv(envp: *mut fenv_t) -> c_int {
  if envp.is_null() {
    return 1;
  }

  let env = with_state(encode_state);
  // SAFETY: non-null was checked and caller upholds pointer validity.
  unsafe {
    envp.write(env);
  }

  0
}

/// C ABI entry point for `feholdexcept`.
///
/// Stores the current environment in `*envp` and then clears all exception
/// flags in the active environment.
///
/// Returns:
/// - `0` on success
/// - non-zero when `envp` is null
///
/// # Safety
/// - `envp` must be valid for writing one `fenv_t` value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn feholdexcept(envp: *mut fenv_t) -> c_int {
  if envp.is_null() {
    return 1;
  }

  let env = with_state(encode_state);
  // SAFETY: non-null was checked and caller upholds pointer validity.
  unsafe {
    envp.write(env);
  }
  update_state(|mut state| {
    state.except_flags = 0;
    state
  });

  0
}

/// C ABI entry point for `fesetenv`.
///
/// Restores the floating-point environment from `*envp`, or resets to default
/// when `envp == FE_DFL_ENV`.
///
/// Returns:
/// - `0` on success
/// - non-zero when `envp` is null or encodes an unsupported state
///
/// # Safety
/// - `envp` must either equal `FE_DFL_ENV` or point to a readable `fenv_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fesetenv(envp: *const fenv_t) -> c_int {
  if is_default_environment_ptr(envp) {
    update_state(|_| FenvState::DEFAULT);

    return 0;
  }

  if envp.is_null() {
    return 1;
  }

  // SAFETY: null/default-sentinel pointers were handled above and caller
  // upholds validity for the remaining case.
  let saved = unsafe { envp.read() };
  let Some(decoded) = decode_state(&saved) else {
    return 1;
  };

  update_state(|_| decoded);

  0
}

/// C ABI entry point for `feupdateenv`.
///
/// Restores the environment from `*envp` (or `FE_DFL_ENV`) and then re-raises
/// exception flags that were set before restoration.
///
/// Returns:
/// - `0` on success
/// - non-zero when `envp` is null or encodes an unsupported state
///
/// # Safety
/// - `envp` must either equal `FE_DFL_ENV` or point to a readable `fenv_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn feupdateenv(envp: *const fenv_t) -> c_int {
  let raised_before_restore = fetestexcept(FE_ALL_EXCEPT);

  // SAFETY: caller upholds the pointer contract.
  if unsafe { fesetenv(envp) } != 0 {
    return 1;
  }

  let _ = feraiseexcept(raised_before_restore);

  0
}
