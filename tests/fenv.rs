use core::mem::{align_of, size_of};
use rlibc::abi::types::c_int;
use rlibc::errno::__errno_location;
use rlibc::fenv::{
  FE_ALL_EXCEPT, FE_DFL_ENV, FE_DIVBYZERO, FE_DOWNWARD, FE_INEXACT, FE_INVALID, FE_OVERFLOW,
  FE_TONEAREST, FE_TOWARDZERO, FE_UNDERFLOW, FE_UPWARD, feclearexcept, fegetenv, fegetexceptflag,
  fegetround, feholdexcept, fenv_t, feraiseexcept, fesetenv, fesetexceptflag, fesetround,
  fetestexcept, feupdateenv, fexcept_t,
};
use std::thread;

fn read_errno() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns writable storage for current thread.
  unsafe { errno_ptr.read() }
}

fn write_errno(value: c_int) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns writable storage for current thread.
  unsafe {
    errno_ptr.write(value);
  }
}

fn reset_fenv_state() {
  // SAFETY: `FE_DFL_ENV` is a valid sentinel for `fesetenv`.
  assert_eq!(unsafe { fesetenv(FE_DFL_ENV) }, 0);
  assert_eq!(feclearexcept(FE_ALL_EXCEPT), 0);
}

fn to_fexcept(value: c_int) -> fexcept_t {
  fexcept_t::try_from(value).unwrap_or_else(|_| panic!("fexcept value out of range: {value}"))
}

#[test]
fn fenv_layout_and_constants_match_x86_64_linux_baseline() {
  assert_eq!(size_of::<fexcept_t>(), 2);
  assert_eq!(align_of::<fexcept_t>(), 2);
  assert_eq!(size_of::<fenv_t>(), 32);
  assert_eq!(align_of::<fenv_t>(), 4);

  assert_eq!(FE_INVALID, 1);
  assert_eq!(FE_DIVBYZERO, 4);
  assert_eq!(FE_OVERFLOW, 8);
  assert_eq!(FE_UNDERFLOW, 16);
  assert_eq!(FE_INEXACT, 32);
  assert_eq!(FE_ALL_EXCEPT, 61);

  assert_eq!(FE_TONEAREST, 0);
  assert_eq!(FE_DOWNWARD, 1024);
  assert_eq!(FE_UPWARD, 2048);
  assert_eq!(FE_TOWARDZERO, 3072);
}

#[test]
fn fesetround_round_trip_rejects_invalid_mode_and_preserves_errno() {
  reset_fenv_state();
  write_errno(73);

  assert_eq!(fegetround(), FE_TONEAREST);
  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_ne!(fesetround(12345), 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(read_errno(), 73);

  reset_fenv_state();
}

#[test]
fn fesetround_accepts_all_supported_modes() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_TONEAREST), 0);
  assert_eq!(fegetround(), FE_TONEAREST);
  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fesetround(FE_TOWARDZERO), 0);
  assert_eq!(fegetround(), FE_TOWARDZERO);

  reset_fenv_state();
}

#[test]
fn fesetround_success_preserves_errno() {
  reset_fenv_state();
  write_errno(58);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(read_errno(), 58);

  assert_eq!(fesetround(FE_TOWARDZERO), 0);
  assert_eq!(fegetround(), FE_TOWARDZERO);
  assert_eq!(read_errno(), 58);

  reset_fenv_state();
}

#[test]
fn fegetround_does_not_modify_errno() {
  reset_fenv_state();
  write_errno(39);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(read_errno(), 39);

  assert_eq!(fesetround(FE_TONEAREST), 0);
  assert_eq!(fegetround(), FE_TONEAREST);
  assert_eq!(read_errno(), 39);

  reset_fenv_state();
}

#[test]
fn exception_flags_support_clear_raise_get_and_set_roundtrip() {
  reset_fenv_state();
  write_errno(64);

  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO | FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);

  let mut saved_flags: fexcept_t = 0;
  // SAFETY: pointer to `saved_flags` is valid for one write.
  assert_eq!(
    unsafe { fegetexceptflag(&raw mut saved_flags, FE_DIVBYZERO) },
    0
  );
  assert_eq!(saved_flags, 4);

  assert_eq!(feclearexcept(FE_ALL_EXCEPT), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
  // SAFETY: pointer to `saved_flags` is valid for one read.
  assert_eq!(
    unsafe { fesetexceptflag(&raw const saved_flags, FE_DIVBYZERO) },
    0
  );
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  assert_eq!(read_errno(), 64);

  reset_fenv_state();
}

#[test]
fn exception_flag_apis_reject_unsupported_masks_without_mutating_state() {
  reset_fenv_state();
  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);

  let unsupported_mask = FE_ALL_EXCEPT | 0x40;
  let mut snapshot: fexcept_t = to_fexcept(FE_INEXACT);
  let incoming: fexcept_t = to_fexcept(FE_INEXACT);

  assert_ne!(feclearexcept(unsupported_mask), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  assert_ne!(feraiseexcept(unsupported_mask), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  // SAFETY: pointer to `snapshot` is valid for one write.
  assert_ne!(
    unsafe { fegetexceptflag(&raw mut snapshot, unsupported_mask) },
    0
  );
  assert_eq!(snapshot, to_fexcept(FE_INEXACT));
  // SAFETY: pointer to `incoming` is valid for one read.
  assert_ne!(
    unsafe { fesetexceptflag(&raw const incoming, unsupported_mask) },
    0
  );
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);

  reset_fenv_state();
}

#[test]
fn fetestexcept_ignores_unsupported_mask_bits() {
  reset_fenv_state();

  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let unsupported_mask = FE_ALL_EXCEPT | 0x40;

  assert_eq!(fetestexcept(unsupported_mask), FE_OVERFLOW);
  assert_eq!(fetestexcept(0x40), 0);

  reset_fenv_state();
}

#[test]
fn fetestexcept_zero_mask_returns_zero_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(0), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn fetestexcept_does_not_modify_errno() {
  reset_fenv_state();
  write_errno(87);

  assert_eq!(feraiseexcept(FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_INEXACT);
  assert_eq!(read_errno(), 87);

  reset_fenv_state();
}

#[test]
fn fetestexcept_unsupported_mask_preserves_errno_and_state() {
  reset_fenv_state();
  write_errno(98);

  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);
  assert_eq!(fetestexcept(0x40), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);
  assert_eq!(read_errno(), 98);

  reset_fenv_state();
}

#[test]
fn feclearexcept_zero_mask_is_noop() {
  reset_fenv_state();

  assert_eq!(feraiseexcept(FE_DIVBYZERO | FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);
  assert_eq!(feclearexcept(0), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);

  reset_fenv_state();
}

#[test]
fn feclearexcept_unsupported_mask_preserves_errno_and_state() {
  reset_fenv_state();
  write_errno(55);

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);

  let unsupported_mask = FE_ALL_EXCEPT | 0x40;

  assert_ne!(feclearexcept(unsupported_mask), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  assert_eq!(read_errno(), 55);

  reset_fenv_state();
}

#[test]
fn feclearexcept_success_preserves_errno() {
  reset_fenv_state();
  write_errno(52);

  assert_eq!(feraiseexcept(FE_DIVBYZERO | FE_INEXACT), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);

  assert_eq!(feclearexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_INEXACT);
  assert_eq!(read_errno(), 52);

  reset_fenv_state();
}

#[test]
fn feraiseexcept_zero_mask_is_noop() {
  reset_fenv_state();

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  assert_eq!(feraiseexcept(0), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);

  reset_fenv_state();
}

#[test]
fn feraiseexcept_unsupported_mask_preserves_errno_and_state() {
  reset_fenv_state();
  write_errno(66);

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);

  let unsupported_mask = FE_ALL_EXCEPT | 0x40;

  assert_ne!(feraiseexcept(unsupported_mask), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  assert_eq!(read_errno(), 66);

  reset_fenv_state();
}

#[test]
fn feraiseexcept_success_preserves_errno() {
  reset_fenv_state();
  write_errno(67);

  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  assert_eq!(read_errno(), 67);

  reset_fenv_state();
}

#[test]
fn fesetexceptflag_rejects_null_pointer_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(
    unsafe { fesetexceptflag(core::ptr::null(), FE_DIVBYZERO) },
    0
  );
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);

  reset_fenv_state();
}

#[test]
fn fesetexceptflag_zero_mask_is_noop() {
  reset_fenv_state();

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);

  let incoming: fexcept_t = to_fexcept(FE_INEXACT);
  // SAFETY: pointer to `incoming` is valid for one read.
  assert_eq!(unsafe { fesetexceptflag(&raw const incoming, 0) }, 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);

  reset_fenv_state();
}

#[test]
fn fesetexceptflag_unsupported_mask_preserves_errno_and_state() {
  reset_fenv_state();
  write_errno(77);

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);

  let incoming: fexcept_t = to_fexcept(FE_INEXACT);
  let unsupported_mask = FE_ALL_EXCEPT | 0x40;
  // SAFETY: pointer to `incoming` is valid for one read.
  assert_ne!(
    unsafe { fesetexceptflag(&raw const incoming, unsupported_mask) },
    0
  );
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  assert_eq!(read_errno(), 77);

  reset_fenv_state();
}

#[test]
fn fesetexceptflag_success_preserves_errno() {
  reset_fenv_state();
  write_errno(68);

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);

  let incoming =
    fexcept_t::try_from(FE_INEXACT).expect("FE_INEXACT must fit in `fexcept_t` for ABI tests");
  // SAFETY: pointer to `incoming` is valid for one read.
  assert_eq!(
    unsafe { fesetexceptflag(&raw const incoming, FE_INEXACT) },
    0
  );
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);
  assert_eq!(read_errno(), 68);

  reset_fenv_state();
}

#[test]
fn fegetexceptflag_rejects_null_pointer_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(
    unsafe { fegetexceptflag(core::ptr::null_mut(), FE_DIVBYZERO) },
    0
  );
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);

  reset_fenv_state();
}

#[test]
fn fegetexceptflag_zero_mask_writes_zero_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(feraiseexcept(FE_DIVBYZERO | FE_INEXACT), 0);

  let mut snapshot: fexcept_t = to_fexcept(FE_INEXACT);
  // SAFETY: pointer to `snapshot` is valid for one write.
  assert_eq!(unsafe { fegetexceptflag(&raw mut snapshot, 0) }, 0);
  assert_eq!(snapshot, 0);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);

  reset_fenv_state();
}

#[test]
fn fegetexceptflag_unsupported_mask_preserves_errno_and_out_param() {
  reset_fenv_state();
  write_errno(44);

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);

  let mut snapshot: fexcept_t = to_fexcept(FE_INEXACT);
  let unsupported_mask = FE_ALL_EXCEPT | 0x40;
  // SAFETY: pointer to `snapshot` is valid for one write.
  assert_ne!(
    unsafe { fegetexceptflag(&raw mut snapshot, unsupported_mask) },
    0
  );
  assert_eq!(snapshot, to_fexcept(FE_INEXACT));
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);
  assert_eq!(read_errno(), 44);

  reset_fenv_state();
}

#[test]
fn fegetexceptflag_success_preserves_errno() {
  reset_fenv_state();
  write_errno(69);

  assert_eq!(feraiseexcept(FE_DIVBYZERO | FE_INEXACT), 0);

  let mut snapshot: fexcept_t = 0;
  // SAFETY: pointer to `snapshot` is valid for one write.
  assert_eq!(
    unsafe { fegetexceptflag(&raw mut snapshot, FE_DIVBYZERO | FE_INEXACT) },
    0
  );
  assert_eq!(snapshot, to_fexcept(FE_DIVBYZERO | FE_INEXACT));
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);
  assert_eq!(read_errno(), 69);

  reset_fenv_state();
}

#[test]
fn fegetenv_and_fesetenv_restore_rounding_mode_and_exception_flags() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut saved = fenv_t::default();
  // SAFETY: pointer to `saved` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut saved) }, 0);

  assert_eq!(fesetround(FE_TOWARDZERO), 0);
  assert_eq!(feclearexcept(FE_ALL_EXCEPT), 0);
  assert_eq!(fegetround(), FE_TOWARDZERO);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);

  // SAFETY: pointer to `saved` is valid for one read.
  assert_eq!(unsafe { fesetenv(&raw const saved) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn fegetenv_rejects_null_pointer_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(unsafe { fegetenv(core::ptr::null_mut()) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn fegetenv_null_pointer_rejection_preserves_errno() {
  reset_fenv_state();
  write_errno(57);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(unsafe { fegetenv(core::ptr::null_mut()) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);
  assert_eq!(read_errno(), 57);

  reset_fenv_state();
}

#[test]
fn fegetenv_success_preserves_errno() {
  reset_fenv_state();
  write_errno(71);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut snapshot = fenv_t::default();
  // SAFETY: pointer to `snapshot` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut snapshot) }, 0);
  assert_eq!(read_errno(), 71);

  assert_eq!(unsafe { fesetenv(&raw const snapshot) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn fesetenv_rejects_unsupported_exception_bits_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut invalid = fenv_t::default();
  // SAFETY: pointer to `invalid` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut invalid) }, 0);

  let unsupported_mask = FE_ALL_EXCEPT | 0x40;
  let env_words = core::ptr::addr_of_mut!(invalid).cast::<u32>();
  // SAFETY: `fenv_t` is `#[repr(C)]` and stores `words: [u32; 8]` at offset 0.
  // Index 1 corresponds to encoded exception flags in this implementation.
  unsafe {
    env_words.add(1).write(
      u32::try_from(unsupported_mask)
        .unwrap_or_else(|_| unreachable!("mask must fit u32 on this target")),
    );
  }

  // SAFETY: pointer to `invalid` is valid for one read.
  assert_ne!(unsafe { fesetenv(&raw const invalid) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn fesetenv_rejects_null_pointer_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(unsafe { fesetenv(core::ptr::null()) }, 0);

  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn fesetenv_null_pointer_rejection_preserves_errno() {
  reset_fenv_state();
  write_errno(56);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(unsafe { fesetenv(core::ptr::null()) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);
  assert_eq!(read_errno(), 56);

  reset_fenv_state();
}

#[test]
fn fesetenv_rejects_invalid_round_mode_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut invalid = fenv_t::default();
  // SAFETY: pointer to `invalid` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut invalid) }, 0);

  let env_words = core::ptr::addr_of_mut!(invalid).cast::<u32>();
  // SAFETY: `fenv_t` is `#[repr(C)]` and stores `words: [u32; 8]` at offset 0.
  // Index 0 corresponds to encoded rounding mode in this implementation.
  unsafe {
    env_words.add(0).write(0xFFFF_u32);
  }

  // SAFETY: pointer to `invalid` is valid for one read.
  assert_ne!(unsafe { fesetenv(&raw const invalid) }, 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn fesetenv_invalid_round_mode_rejection_preserves_errno() {
  reset_fenv_state();
  write_errno(58);

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut invalid = fenv_t::default();
  // SAFETY: pointer to `invalid` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut invalid) }, 0);

  let env_words = core::ptr::addr_of_mut!(invalid).cast::<u32>();
  // SAFETY: `fenv_t` is `#[repr(C)]` and stores `words: [u32; 8]` at offset 0.
  // Index 0 corresponds to encoded rounding mode in this implementation.
  unsafe {
    env_words.add(0).write(0xFFFF_u32);
  }

  // SAFETY: pointer to `invalid` is valid for one read.
  assert_ne!(unsafe { fesetenv(&raw const invalid) }, 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);
  assert_eq!(read_errno(), 58);

  reset_fenv_state();
}

#[test]
fn fesetenv_rejects_nonzero_reserved_words_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut invalid = fenv_t::default();
  // SAFETY: pointer to `invalid` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut invalid) }, 0);

  let env_words = core::ptr::addr_of_mut!(invalid).cast::<u32>();
  // SAFETY: `fenv_t` is `#[repr(C)]` and stores `words: [u32; 8]` at offset 0.
  // Indexes 2..=7 are reserved/opaque in this implementation and must remain zero.
  unsafe {
    env_words.add(2).write(1);
  }

  // SAFETY: pointer to `invalid` is valid for one read.
  assert_ne!(unsafe { fesetenv(&raw const invalid) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn fesetenv_with_default_env_resets_round_and_clears_flags() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO | FE_INEXACT), 0);
  // SAFETY: `FE_DFL_ENV` is a valid sentinel for `fesetenv`.
  assert_eq!(unsafe { fesetenv(FE_DFL_ENV) }, 0);
  assert_eq!(fegetround(), FE_TONEAREST);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);

  reset_fenv_state();
}

#[test]
fn fesetenv_with_default_env_preserves_errno() {
  reset_fenv_state();
  write_errno(54);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);
  // SAFETY: `FE_DFL_ENV` is a valid sentinel for `fesetenv`.
  assert_eq!(unsafe { fesetenv(FE_DFL_ENV) }, 0);

  assert_eq!(fegetround(), FE_TONEAREST);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
  assert_eq!(read_errno(), 54);

  reset_fenv_state();
}

#[test]
fn fesetenv_success_preserves_errno() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut saved = fenv_t::default();
  // SAFETY: pointer to `saved` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut saved) }, 0);

  assert_eq!(fesetround(FE_TOWARDZERO), 0);
  assert_eq!(feclearexcept(FE_ALL_EXCEPT), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  write_errno(53);

  // SAFETY: pointer to `saved` is valid for one read.
  assert_eq!(unsafe { fesetenv(&raw const saved) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);
  assert_eq!(read_errno(), 53);

  reset_fenv_state();
}

#[test]
fn feupdateenv_rejects_invalid_round_mode_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_INEXACT), 0);

  let mut invalid = fenv_t::default();
  // SAFETY: pointer to `invalid` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut invalid) }, 0);

  let env_words = core::ptr::addr_of_mut!(invalid).cast::<u32>();
  // SAFETY: `fenv_t` is `#[repr(C)]` and stores `words: [u32; 8]` at offset 0.
  // Index 0 corresponds to encoded rounding mode in this implementation.
  unsafe {
    env_words.add(0).write(0xFFFF_u32);
  }

  // SAFETY: pointer to `invalid` is valid for one read.
  assert_ne!(unsafe { feupdateenv(&raw const invalid) }, 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_INEXACT);

  reset_fenv_state();
}

#[test]
fn feupdateenv_rejects_unsupported_exception_bits_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut invalid = fenv_t::default();
  // SAFETY: pointer to `invalid` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut invalid) }, 0);

  let unsupported_mask = FE_ALL_EXCEPT | 0x40;
  let env_words = core::ptr::addr_of_mut!(invalid).cast::<u32>();
  // SAFETY: `fenv_t` is `#[repr(C)]` and stores `words: [u32; 8]` at offset 0.
  // Index 0 is encoded round mode and index 1 is encoded exception flags.
  unsafe {
    env_words.add(0).write(
      u32::try_from(FE_TOWARDZERO)
        .unwrap_or_else(|_| unreachable!("round mode must fit u32 on this target")),
    );
    env_words.add(1).write(
      u32::try_from(unsupported_mask)
        .unwrap_or_else(|_| unreachable!("mask must fit u32 on this target")),
    );
  }

  // SAFETY: pointer to `invalid` is valid for one read.
  assert_ne!(unsafe { feupdateenv(&raw const invalid) }, 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn feupdateenv_rejects_nonzero_reserved_words_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut invalid = fenv_t::default();
  // SAFETY: pointer to `invalid` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut invalid) }, 0);

  let env_words = core::ptr::addr_of_mut!(invalid).cast::<u32>();
  // SAFETY: `fenv_t` is `#[repr(C)]` and stores `words: [u32; 8]` at offset 0.
  // Indexes 2..=7 are reserved/opaque in this implementation and must remain zero.
  unsafe {
    env_words.add(2).write(1);
  }

  // SAFETY: pointer to `invalid` is valid for one read.
  assert_ne!(unsafe { feupdateenv(&raw const invalid) }, 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}

#[test]
fn feupdateenv_with_default_env_resets_round_and_reraises_pending_flags() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO | FE_INEXACT), 0);
  // SAFETY: `FE_DFL_ENV` is a valid sentinel for `feupdateenv`.
  assert_eq!(unsafe { feupdateenv(FE_DFL_ENV) }, 0);

  assert_eq!(fegetround(), FE_TONEAREST);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);

  reset_fenv_state();
}

#[test]
fn feupdateenv_with_default_env_preserves_errno() {
  reset_fenv_state();
  write_errno(89);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO | FE_INEXACT), 0);
  // SAFETY: `FE_DFL_ENV` is a valid sentinel for `feupdateenv`.
  assert_eq!(unsafe { feupdateenv(FE_DFL_ENV) }, 0);

  assert_eq!(fegetround(), FE_TONEAREST);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO | FE_INEXACT);
  assert_eq!(read_errno(), 89);

  reset_fenv_state();
}

#[test]
fn feupdateenv_success_preserves_errno() {
  reset_fenv_state();
  write_errno(88);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut saved = fenv_t::default();
  // SAFETY: pointer to `saved` is valid for one write.
  assert_eq!(unsafe { fegetenv(&raw mut saved) }, 0);

  assert_eq!(fesetround(FE_TOWARDZERO), 0);
  assert_eq!(feclearexcept(FE_ALL_EXCEPT), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);

  // SAFETY: pointer to `saved` is valid for one read.
  assert_eq!(unsafe { feupdateenv(&raw const saved) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW | FE_DIVBYZERO);
  assert_eq!(read_errno(), 88);

  reset_fenv_state();
}

#[test]
fn feupdateenv_rejects_null_pointer_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(unsafe { feupdateenv(core::ptr::null()) }, 0);

  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_DIVBYZERO);

  reset_fenv_state();
}

#[test]
fn feupdateenv_null_pointer_rejection_preserves_errno() {
  reset_fenv_state();
  write_errno(90);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(unsafe { feupdateenv(core::ptr::null()) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);
  assert_eq!(read_errno(), 90);

  reset_fenv_state();
}

#[test]
fn feholdexcept_rejects_null_pointer_without_mutating_state() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_INEXACT), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(unsafe { feholdexcept(core::ptr::null_mut()) }, 0);

  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_INEXACT);

  reset_fenv_state();
}

#[test]
fn feholdexcept_null_pointer_rejection_preserves_errno() {
  reset_fenv_state();
  write_errno(92);

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_INEXACT), 0);
  // SAFETY: passing a null pointer is intentional for contract validation.
  assert_ne!(unsafe { feholdexcept(core::ptr::null_mut()) }, 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_INEXACT);
  assert_eq!(read_errno(), 92);

  reset_fenv_state();
}

#[test]
fn feholdexcept_snapshot_can_be_restored_with_fesetenv() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW | FE_INEXACT), 0);

  let mut held = fenv_t::default();
  // SAFETY: pointer to `held` is valid for one write.
  assert_eq!(unsafe { feholdexcept(&raw mut held) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  // SAFETY: pointer to `held` is valid for one read.
  assert_eq!(unsafe { fesetenv(&raw const held) }, 0);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW | FE_INEXACT);

  reset_fenv_state();
}

#[test]
fn feholdexcept_success_preserves_errno() {
  reset_fenv_state();
  write_errno(91);

  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let mut held = fenv_t::default();
  // SAFETY: pointer to `held` is valid for one write.
  assert_eq!(unsafe { feholdexcept(&raw mut held) }, 0);
  assert_eq!(read_errno(), 91);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);

  // SAFETY: pointer to `held` is valid for one read.
  assert_eq!(unsafe { fesetenv(&raw const held) }, 0);

  reset_fenv_state();
}

#[test]
fn feholdexcept_and_feupdateenv_merge_saved_and_pending_exceptions() {
  reset_fenv_state();

  assert_eq!(fesetround(FE_DOWNWARD), 0);
  assert_eq!(feraiseexcept(FE_INEXACT), 0);

  let mut held = fenv_t::default();
  // SAFETY: pointer to `held` is valid for one write.
  assert_eq!(unsafe { feholdexcept(&raw mut held) }, 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);

  assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);
  // SAFETY: pointer to `held` is valid for one read.
  assert_eq!(unsafe { feupdateenv(&raw const held) }, 0);
  assert_eq!(fegetround(), FE_DOWNWARD);
  assert_eq!(
    fetestexcept(FE_ALL_EXCEPT),
    FE_INEXACT | FE_DIVBYZERO,
    "saved flags and pending flags must be merged",
  );

  reset_fenv_state();
}

#[test]
fn floating_point_environment_state_is_isolated_between_threads() {
  reset_fenv_state();
  assert_eq!(fesetround(FE_UPWARD), 0);
  assert_eq!(feraiseexcept(FE_OVERFLOW), 0);

  let child = thread::spawn(|| {
    reset_fenv_state();
    assert_eq!(fegetround(), FE_TONEAREST);
    assert_eq!(fetestexcept(FE_ALL_EXCEPT), 0);
    assert_eq!(fesetround(FE_DOWNWARD), 0);
    assert_eq!(feraiseexcept(FE_DIVBYZERO), 0);

    (fegetround(), fetestexcept(FE_ALL_EXCEPT))
  });
  let (child_round, child_flags) = child.join().expect("child thread panicked");

  assert_eq!(child_round, FE_DOWNWARD);
  assert_eq!(child_flags, FE_DIVBYZERO);
  assert_eq!(fegetround(), FE_UPWARD);
  assert_eq!(fetestexcept(FE_ALL_EXCEPT), FE_OVERFLOW);

  reset_fenv_state();
}
