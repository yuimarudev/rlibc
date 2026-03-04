//! Signal-set operations and signal-delivery C ABI functions.
//!
//! This module provides Linux `x86_64` syscall-backed wrappers for:
//! - signal-set operations (`sigemptyset`, `sigfillset`, `sigaddset`,
//!   `sigdelset`, `sigismember`)
//! - handler installation (`sigaction`)
//! - signal delivery/mask interfaces (`raise`, `kill`, `sigprocmask`)
//!
//! Return contract for exported entry points:
//! - success: `0` (or `0/1` for `sigismember`)
//! - failure: `-1` and thread-local `errno` set from kernel `-errno` or
//!   argument validation

use crate::abi::errno::{EFAULT, EINVAL};
use crate::abi::types::{c_int, c_long, c_ulong, c_void, size_t};
use crate::errno::set_errno;
use crate::syscall::{syscall0, syscall2, syscall3, syscall4, syscall6};

const SYS_GETPID: c_long = 39;
const SYS_KILL: c_long = 62;
const SYS_PROCESS_VM_READV: c_long = 310;
const SYS_RT_SIGACTION: c_long = 13;
const SYS_RT_SIGPROCMASK: c_long = 14;
const SYS_GETTID: c_long = 186;
const SYS_TGKILL: c_long = 234;
const SIGSET_WORDS: usize = 16;
const BITS_PER_WORD: usize = core::mem::size_of::<c_ulong>() * 8;
const MAX_KERNEL_SIGNAL: c_int = 64;

core::arch::global_asm!(
  ".globl rlibc_rt_sigreturn_trampoline",
  ".type rlibc_rt_sigreturn_trampoline,@function",
  "rlibc_rt_sigreturn_trampoline:",
  "mov rax, 15",
  "syscall",
  ".size rlibc_rt_sigreturn_trampoline, .-rlibc_rt_sigreturn_trampoline",
);

unsafe extern "C" {
  fn rlibc_rt_sigreturn_trampoline();
}

/// `sigprocmask` operation: block the signals in `set`.
pub const SIG_BLOCK: c_int = 0;
/// `sigprocmask` operation: unblock the signals in `set`.
pub const SIG_UNBLOCK: c_int = 1;
/// `sigprocmask` operation: replace current mask with `set`.
pub const SIG_SETMASK: c_int = 2;
/// `SIGABRT` signal number on Linux `x86_64`.
pub const SIGABRT: c_int = 6;
/// `SIGKILL` signal number on Linux `x86_64`.
pub const SIGKILL: c_int = 9;
/// `SIGUSR1` signal number on Linux `x86_64`.
pub const SIGUSR1: c_int = 10;
/// `SIGUSR2` signal number on Linux `x86_64`.
pub const SIGUSR2: c_int = 12;
/// `SIGSTOP` signal number on Linux `x86_64`.
pub const SIGSTOP: c_int = 19;
/// Default signal disposition marker.
pub const SIG_DFL: usize = 0;
/// Ignore signal disposition marker.
pub const SIG_IGN: usize = 1;
/// `sigaction` flag: restart interrupted syscalls where possible.
pub const SA_RESTART: c_ulong = 0x1000_0000;
/// `sigaction` flag: use `sa_restorer` trampoline (Linux specific).
pub const SA_RESTORER: c_ulong = 0x0400_0000;
/// `sigaction` flag: pass extended signal context (`sa_sigaction` style).
pub const SA_SIGINFO: c_ulong = 0x0000_0004;

/// Linux/glibc-compatible `sigset_t` layout for `x86_64`.
///
/// This is represented as 1024 bits (`16 * unsigned long`) to match user-space
/// ABI expectations. `sigprocmask` forwards only the kernel-required first
/// word-size (`8` bytes on this target) via `rt_sigprocmask` size argument.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SigSet {
  /// Raw signal-set words (`bit(sig - 1)` marks membership).
  ///
  /// This corresponds to the C ABI `sigset_t.__val` storage on Linux.
  pub bits: [c_ulong; SIGSET_WORDS],
}

/// Linux `sigaction` payload for `x86_64`.
///
/// `sa_handler` stores either a function pointer encoded as address-sized
/// integer, or one of [`SIG_DFL`]/[`SIG_IGN`]. `sa_mask` uses [`SigSet`]
/// userspace layout while kernel syscalls consume the kernel-sized leading
/// subset (`8` bytes on this target).
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SigAction {
  /// Signal disposition (`SIG_DFL`, `SIG_IGN`, or handler pointer value).
  pub sa_handler: usize,
  /// `SA_*` behavior flags.
  pub sa_flags: c_ulong,
  /// Optional restorer trampoline pointer (`SA_RESTORER` usage).
  pub sa_restorer: usize,
  /// Signal mask applied while running handler.
  pub sa_mask: SigSet,
}

impl Default for SigSet {
  fn default() -> Self {
    Self::empty()
  }
}

impl Default for SigAction {
  fn default() -> Self {
    Self {
      sa_handler: SIG_DFL,
      sa_flags: 0,
      sa_restorer: 0,
      sa_mask: SigSet::empty(),
    }
  }
}

impl SigSet {
  /// Returns an empty signal set.
  #[must_use]
  pub const fn empty() -> Self {
    Self {
      bits: [0; SIGSET_WORDS],
    }
  }

  /// Adds `sig` to this signal set.
  ///
  /// Returns `true` when `sig` is representable in this set, otherwise `false`.
  pub fn add_signal(&mut self, sig: c_int) -> bool {
    let Some((word, bit)) = signal_word_and_bit(sig) else {
      return false;
    };

    self.bits[word] |= bit;
    true
  }

  /// Removes `sig` from this signal set.
  ///
  /// Returns `true` when `sig` is representable in this set, otherwise `false`.
  pub fn remove_signal(&mut self, sig: c_int) -> bool {
    let Some((word, bit)) = signal_word_and_bit(sig) else {
      return false;
    };

    self.bits[word] &= !bit;
    true
  }

  /// Reports whether `sig` is currently present in this signal set.
  #[must_use]
  pub fn contains_signal(&self, sig: c_int) -> bool {
    let Some((word, bit)) = signal_word_and_bit(sig) else {
      return false;
    };

    (self.bits[word] & bit) != 0
  }
}

fn signal_word_and_bit(sig: c_int) -> Option<(usize, c_ulong)> {
  if !(1..=MAX_KERNEL_SIGNAL).contains(&sig) {
    return None;
  }

  let zero_based = usize::try_from(sig - 1).ok()?;
  let word = zero_based / BITS_PER_WORD;

  if word >= SIGSET_WORDS {
    return None;
  }

  let bit = (1 as c_ulong) << (zero_based % BITS_PER_WORD);

  Some((word, bit))
}

const fn is_valid_delivery_signal(sig: c_int) -> bool {
  sig == 0 || (sig >= 1 && sig <= MAX_KERNEL_SIGNAL)
}

const fn is_valid_sigprocmask_how(how: c_int) -> bool {
  matches!(how, SIG_BLOCK | SIG_UNBLOCK | SIG_SETMASK)
}

fn ptr_arg<T>(ptr: *const T) -> c_long {
  c_long::try_from(ptr.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit into c_long on x86_64 Linux"))
}

fn mut_ptr_arg<T>(ptr: *mut T) -> c_long {
  ptr_arg(ptr.cast_const())
}

fn errno_from_raw(raw: c_long) -> c_int {
  c_int::try_from(-raw).unwrap_or(c_int::MAX)
}

fn invalid_argument() -> c_int {
  set_errno(EINVAL);
  -1
}

fn bad_address() -> c_int {
  set_errno(EFAULT);
  -1
}

fn status_from_raw(raw: c_long) -> c_int {
  if raw < 0 {
    set_errno(errno_from_raw(raw));

    return -1;
  }

  0
}

fn kernel_sigset_size() -> c_long {
  c_long::try_from(core::mem::size_of::<c_ulong>())
    .unwrap_or_else(|_| unreachable!("sigset size must fit c_long on x86_64 Linux"))
}

const fn sigaction_struct_size() -> usize {
  core::mem::size_of::<SigAction>()
}

fn sigaction_struct_size_as_size_t() -> size_t {
  size_t::try_from(sigaction_struct_size())
    .unwrap_or_else(|_| unreachable!("SigAction size must fit size_t on x86_64 Linux"))
}

fn sigaction_struct_size_as_c_long() -> c_long {
  c_long::try_from(sigaction_struct_size())
    .unwrap_or_else(|_| unreachable!("SigAction size must fit c_long on x86_64 Linux"))
}

fn copy_sigaction_from_user(user_action: *const SigAction) -> Result<SigAction, c_int> {
  #[repr(C)]
  struct IoVec {
    iov_base: *mut c_void,
    iov_len: size_t,
  }

  let mut local_action = SigAction::default();
  let local_iov = IoVec {
    iov_base: (&raw mut local_action).cast::<c_void>(),
    iov_len: sigaction_struct_size_as_size_t(),
  };
  let remote_iov = IoVec {
    iov_base: user_action.cast_mut().cast::<c_void>(),
    iov_len: sigaction_struct_size_as_size_t(),
  };
  // SAFETY: `getpid` is an argument-free syscall.
  let process_id = unsafe { syscall0(SYS_GETPID) };

  if process_id < 0 {
    return Err(errno_from_raw(process_id));
  }

  // SAFETY: issues `process_vm_readv(getpid(), &local_iov, 1, &remote_iov, 1, 0)`.
  // This lets the kernel validate `user_action` and report EFAULT instead of
  // dereferencing the pointer in userspace.
  let copied_bytes = unsafe {
    syscall6(
      SYS_PROCESS_VM_READV,
      process_id,
      ptr_arg(&raw const local_iov),
      1,
      ptr_arg(&raw const remote_iov),
      1,
      0,
    )
  };

  if copied_bytes < 0 {
    return Err(errno_from_raw(copied_bytes));
  }

  if copied_bytes != sigaction_struct_size_as_c_long() {
    return Err(EFAULT);
  }

  Ok(local_action)
}

/// C ABI entry point for `sigemptyset`.
///
/// Clears all bits in `set`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure (`set == NULL`, `errno = EFAULT`)
///
/// # Safety
/// - `set` must point to writable [`SigSet`] storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigemptyset(set: *mut SigSet) -> c_int {
  let Some(set_ref) = (unsafe { set.as_mut() }) else {
    return bad_address();
  };

  *set_ref = SigSet::empty();

  0
}

/// C ABI entry point for `sigfillset`.
///
/// Sets all bits in `set`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure (`set == NULL`, `errno = EFAULT`)
///
/// # Safety
/// - `set` must point to writable [`SigSet`] storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigfillset(set: *mut SigSet) -> c_int {
  let Some(set_ref) = (unsafe { set.as_mut() }) else {
    return bad_address();
  };

  *set_ref = SigSet {
    bits: [c_ulong::MAX; SIGSET_WORDS],
  };

  0
}

/// C ABI entry point for `sigaddset`.
///
/// Adds `signum` to `set`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno` (`EFAULT` or `EINVAL`)
///
/// Valid signal range:
/// - `signum` must be in `1..=64` on this Linux `x86_64` target profile.
///
/// # Safety
/// - `set` must point to writable [`SigSet`] storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaddset(set: *mut SigSet, signum: c_int) -> c_int {
  let Some(set_ref) = (unsafe { set.as_mut() }) else {
    return bad_address();
  };
  let Some((word, bit)) = signal_word_and_bit(signum) else {
    return invalid_argument();
  };

  set_ref.bits[word] |= bit;

  0
}

/// C ABI entry point for `sigdelset`.
///
/// Removes `signum` from `set`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno` (`EFAULT` or `EINVAL`)
///
/// Valid signal range:
/// - `signum` must be in `1..=64` on this Linux `x86_64` target profile.
///
/// # Safety
/// - `set` must point to writable [`SigSet`] storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigdelset(set: *mut SigSet, signum: c_int) -> c_int {
  let Some(set_ref) = (unsafe { set.as_mut() }) else {
    return bad_address();
  };
  let Some((word, bit)) = signal_word_and_bit(signum) else {
    return invalid_argument();
  };

  set_ref.bits[word] &= !bit;

  0
}

/// C ABI entry point for `sigismember`.
///
/// Reports whether `signum` is currently present in `set`.
///
/// Returns:
/// - `1` when member
/// - `0` when not member
/// - `-1` on failure and sets `errno` (`EFAULT` or `EINVAL`)
///
/// Valid signal range:
/// - `signum` must be in `1..=64` on this Linux `x86_64` target profile.
///
/// # Safety
/// - `set` must point to readable [`SigSet`] storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigismember(set: *const SigSet, signum: c_int) -> c_int {
  let Some(set_ref) = (unsafe { set.as_ref() }) else {
    return bad_address();
  };
  let Some((word, bit)) = signal_word_and_bit(signum) else {
    return invalid_argument();
  };

  if (set_ref.bits[word] & bit) != 0 {
    return 1;
  }

  0
}

/// C ABI entry point for `sigaction`.
///
/// Installs and/or fetches signal disposition for `signum`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno`
///
/// Error contract:
/// - rejects out-of-range `signum` with `errno = EINVAL`
/// - rejects attempts to alter `SIGKILL` and `SIGSTOP` with `errno = EINVAL`
/// - reports `errno = EFAULT` when `act` points to unreadable memory
/// - when `act` is non-null and `SA_RESTORER` is omitted, `rlibc` provides an
///   internal `rt_sigreturn` trampoline and sets `SA_RESTORER` for kernel ABI
/// - when `act` is non-null and `sa_restorer == 0`, `rlibc` fills
///   `sa_restorer` with an internal `rt_sigreturn` trampoline pointer
/// - when `oldact` is non-null, words outside the kernel-exposed mask range
///   (`oldact.sa_mask.bits[1..]`) are cleared to zero
///
/// Valid signal range:
/// - `signum` must be in `1..=64` on this Linux `x86_64` target profile.
///
/// # Safety
/// - `act`, when non-null, must point to readable [`SigAction`].
/// - `oldact`, when non-null, must point to writable [`SigAction`] storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaction(
  signum: c_int,
  act: *const SigAction,
  oldact: *mut SigAction,
) -> c_int {
  if signal_word_and_bit(signum).is_none() || signum == SIGKILL || signum == SIGSTOP {
    return invalid_argument();
  }

  let kernel_action_storage = if act.is_null() {
    None
  } else {
    let mut kernel_action = match copy_sigaction_from_user(act) {
      Ok(action) => action,
      Err(errno_value) => {
        set_errno(errno_value);

        return -1;
      }
    };
    let trampoline_pointer = rlibc_rt_sigreturn_trampoline as *const () as usize;

    if (kernel_action.sa_flags & SA_RESTORER) == 0 {
      kernel_action.sa_flags |= SA_RESTORER;
    }

    if kernel_action.sa_restorer == 0 {
      kernel_action.sa_restorer = trampoline_pointer;
    }

    Some(kernel_action)
  };
  let act_for_syscall = kernel_action_storage
    .as_ref()
    .map_or(core::ptr::null(), std::ptr::from_ref);
  let raw = unsafe {
    syscall4(
      SYS_RT_SIGACTION,
      c_long::from(signum),
      ptr_arg(act_for_syscall.cast::<c_void>()),
      mut_ptr_arg(oldact.cast::<c_void>()),
      kernel_sigset_size(),
    )
  };
  let status = status_from_raw(raw);

  if status == 0 && !oldact.is_null() {
    // SAFETY: `oldact` is non-null and accepted by the kernel on successful
    // `rt_sigaction`; only userspace-only high mask words are normalized.
    unsafe {
      (&mut (*oldact).sa_mask.bits)[1..].fill(0);
    }
  }

  status
}

/// C ABI entry point for `kill`.
///
/// Sends signal number `sig` to process `pid`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno` to the kernel-provided error code
///
/// Valid signal range:
/// - `sig` must be `0` (existence probe) or `1..=64`.
#[unsafe(no_mangle)]
pub extern "C" fn kill(pid: c_int, sig: c_int) -> c_int {
  if !is_valid_delivery_signal(sig) {
    return invalid_argument();
  }

  // SAFETY: syscall number and integer arguments follow Linux x86_64 ABI.
  let raw = unsafe { syscall2(SYS_KILL, c_long::from(pid), c_long::from(sig)) };

  status_from_raw(raw)
}

/// C ABI entry point for `raise`.
///
/// Sends `sig` to the calling thread via `tgkill(getpid(), gettid(), sig)`.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno` to the kernel-provided error code
///
/// Valid signal range:
/// - `sig` must be `0` (existence probe semantics) or `1..=64`.
#[unsafe(no_mangle)]
pub extern "C" fn raise(sig: c_int) -> c_int {
  if !is_valid_delivery_signal(sig) {
    return invalid_argument();
  }

  // SAFETY: `getpid` and `gettid` are argument-free syscalls.
  let process_id = unsafe { syscall0(SYS_GETPID) };

  if process_id < 0 {
    return status_from_raw(process_id);
  }

  // SAFETY: `gettid` is argument-free and thread-local by kernel definition.
  let thread_id = unsafe { syscall0(SYS_GETTID) };

  if thread_id < 0 {
    return status_from_raw(thread_id);
  }

  // SAFETY: syscall number and arguments follow Linux x86_64 ABI.
  let raw = unsafe { syscall3(SYS_TGKILL, process_id, thread_id, c_long::from(sig)) };

  status_from_raw(raw)
}

/// C ABI entry point for `sigprocmask`.
///
/// Updates and/or reads the calling thread signal mask.
///
/// Returns:
/// - `0` on success
/// - `-1` on failure and sets `errno` to the kernel-provided error code
///
/// ABI note:
/// - Linux `rt_sigprocmask` updates only the kernel-sized leading word on this
///   target. This wrapper preserves remaining user-space words in `oldset`
///   unchanged instead of normalizing them.
/// - When `set` is null (query-only usage), `how` is treated as ignored and
///   normalized to `SIG_SETMASK` before issuing the syscall.
/// - When both `set` and `oldset` are null, this wrapper returns success
///   immediately as a no-op without issuing a syscall.
/// - When `set` is non-null and `how` is not one of `SIG_BLOCK`,
///   `SIG_UNBLOCK`, or `SIG_SETMASK`, this wrapper returns `-1` with
///   `errno = EINVAL` without issuing a syscall.
///
/// # Safety
/// - `set`, when non-null, must point to a readable [`SigSet`].
/// - `oldset`, when non-null, must point to writable [`SigSet`] storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigprocmask(how: c_int, set: *const SigSet, oldset: *mut SigSet) -> c_int {
  if set.is_null() && oldset.is_null() {
    return 0;
  }

  if !set.is_null() && !is_valid_sigprocmask_how(how) {
    return invalid_argument();
  }

  let effective_how = if set.is_null() { SIG_SETMASK } else { how };

  // SAFETY: syscall number and arguments follow Linux x86_64 ABI.
  let raw = unsafe {
    syscall4(
      SYS_RT_SIGPROCMASK,
      c_long::from(effective_how),
      ptr_arg(set),
      mut_ptr_arg(oldset),
      kernel_sigset_size(),
    )
  };

  status_from_raw(raw)
}

#[cfg(test)]
mod tests {
  use super::{MAX_KERNEL_SIGNAL, SIGUSR1, SigSet, signal_word_and_bit};

  #[test]
  fn sigset_helpers_track_inserted_signal_bits() {
    let mut set = SigSet::empty();

    assert!(!set.contains_signal(SIGUSR1));
    assert!(set.add_signal(SIGUSR1));
    assert!(set.contains_signal(SIGUSR1));
  }

  #[test]
  fn sigset_helpers_reject_non_positive_signal_numbers() {
    let mut set = SigSet::empty();

    assert!(!set.add_signal(0));
    assert!(!set.contains_signal(0));
    assert!(!set.add_signal(-1));
    assert!(!set.contains_signal(-1));
  }

  #[test]
  fn sigset_helpers_remove_inserted_signal_bits() {
    let mut set = SigSet::empty();

    assert!(set.add_signal(SIGUSR1));
    assert!(set.contains_signal(SIGUSR1));
    assert!(set.remove_signal(SIGUSR1));
    assert!(!set.contains_signal(SIGUSR1));
  }

  #[test]
  fn signal_indexing_rejects_out_of_kernel_range() {
    assert!(signal_word_and_bit(MAX_KERNEL_SIGNAL).is_some());
    assert!(signal_word_and_bit(MAX_KERNEL_SIGNAL + 1).is_none());
  }
}
