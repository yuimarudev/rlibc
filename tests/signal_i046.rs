#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_int;
use core::ptr;
use rlibc::abi::errno::{EINVAL, ESRCH};
use rlibc::abi::types::c_ulong;
use rlibc::errno::__errno_location;
use rlibc::signal::{
  SIG_BLOCK, SIG_SETMASK, SIG_UNBLOCK, SIGKILL, SIGSTOP, SIGUSR1, SigAction, SigSet, kill, raise,
  sigaction, sigprocmask,
};
use std::process;
use std::sync::{Mutex, MutexGuard, OnceLock};

const INVALID_SIGNAL: c_int = 9_999;
const FIRST_OUT_OF_RANGE_SIGNAL: c_int = 65;
const NEGATIVE_SIGNAL: c_int = -1;
const ERRNO_SENTINEL: c_int = 31_337;

struct SigmaskRestoreGuard {
  original: SigSet,
}

impl SigmaskRestoreGuard {
  fn capture() -> Self {
    let mut original = SigSet::empty();
    // SAFETY: `oldset` points to writable storage; reading current mask does not
    // require a non-null `set`.
    let status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut original) };

    assert_eq!(status, 0, "failed to capture current signal mask");

    Self { original }
  }
}

impl Drop for SigmaskRestoreGuard {
  fn drop(&mut self) {
    // SAFETY: restoring previously captured mask for the same thread.
    let _status = unsafe { sigprocmask(SIG_SETMASK, &raw const self.original, ptr::null_mut()) };
  }
}

fn signal_lock() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  LOCK
    .get_or_init(|| Mutex::new(()))
    .lock()
    .expect("signal_i046 lock poisoned")
}

fn current_pid() -> c_int {
  c_int::try_from(process::id())
    .unwrap_or_else(|_| unreachable!("process id must fit into c_int on Linux x86_64"))
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local storage.
  unsafe {
    __errno_location().write(value);
  }
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns readable thread-local storage.
  unsafe { __errno_location().read() }
}

#[test]
fn raise_invalid_signal_returns_minus_one_and_sets_errno() {
  let _lock = signal_lock();

  write_errno(0);

  let status = raise(INVALID_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn raise_negative_signal_returns_minus_one_and_sets_errno() {
  let _lock = signal_lock();

  write_errno(0);

  let status = raise(NEGATIVE_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn raise_signal_just_above_kernel_range_returns_minus_one_and_sets_errno() {
  let _lock = signal_lock();

  write_errno(0);

  let status = raise(FIRST_OUT_OF_RANGE_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn raise_zero_signal_succeeds_without_changing_errno() {
  let _lock = signal_lock();

  write_errno(ERRNO_SENTINEL);

  let status = raise(0);

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn kill_with_zero_signal_for_current_process_succeeds() {
  let _lock = signal_lock();

  write_errno(ERRNO_SENTINEL);

  let status = kill(current_pid(), 0);

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn kill_with_invalid_signal_returns_minus_one_and_sets_errno() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(current_pid(), INVALID_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn kill_with_process_group_target_and_invalid_signal_returns_einval() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(0, INVALID_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn kill_with_broadcast_target_and_invalid_signal_returns_einval() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(-1, INVALID_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn kill_with_negative_signal_returns_minus_one_and_sets_errno() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(current_pid(), NEGATIVE_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn kill_with_signal_just_above_kernel_range_returns_minus_one_and_sets_errno() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(current_pid(), FIRST_OUT_OF_RANGE_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn kill_with_nonexistent_pid_sets_esrch() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(c_int::MAX, 0);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), ESRCH);
}

#[test]
fn kill_with_nonexistent_pid_and_nonzero_signal_sets_esrch() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(c_int::MAX, SIGUSR1);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), ESRCH);
}

#[test]
fn kill_with_nonexistent_pid_and_invalid_signal_returns_einval() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(c_int::MAX, INVALID_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn kill_with_nonexistent_pid_and_negative_signal_returns_einval() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(c_int::MAX, NEGATIVE_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn kill_with_nonexistent_pid_and_first_out_of_range_signal_returns_einval() {
  let _lock = signal_lock();

  write_errno(0);

  let status = kill(c_int::MAX, FIRST_OUT_OF_RANGE_SIGNAL);

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_sigkill_even_when_only_querying() {
  let _lock = signal_lock();
  let mut oldact = SigAction::default();

  write_errno(0);
  // SAFETY: `oldact` points to writable storage and `act == NULL` requests query-only mode.
  let status = unsafe { sigaction(SIGKILL, ptr::null(), &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_sigstop_even_when_only_querying() {
  let _lock = signal_lock();
  let mut oldact = SigAction::default();

  write_errno(0);
  // SAFETY: `oldact` points to writable storage and `act == NULL` requests query-only mode.
  let status = unsafe { sigaction(SIGSTOP, ptr::null(), &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_forbidden_query_keeps_oldact_storage_unchanged() {
  let _lock = signal_lock();
  let mut oldact = SigAction {
    sa_handler: usize::MAX,
    sa_flags: c_ulong::MAX,
    sa_restorer: usize::MAX - 1,
    sa_mask: SigSet {
      bits: [c_ulong::MAX; 16],
    },
  };
  let before = oldact;

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query mode with writable `oldact` for a forbidden signal.
  let status = unsafe { sigaction(SIGSTOP, ptr::null(), &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    oldact, before,
    "oldact must remain unchanged when sigaction query rejects SIGSTOP",
  );
}

#[test]
fn sigaction_forbidden_sigkill_query_keeps_oldact_storage_unchanged() {
  let _lock = signal_lock();
  let mut oldact = SigAction {
    sa_handler: usize::MAX,
    sa_flags: c_ulong::MAX,
    sa_restorer: usize::MAX - 1,
    sa_mask: SigSet {
      bits: [c_ulong::MAX; 16],
    },
  };
  let before = oldact;

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query mode with writable `oldact` for a forbidden signal.
  let status = unsafe { sigaction(SIGKILL, ptr::null(), &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    oldact, before,
    "oldact must remain unchanged when sigaction query rejects SIGKILL",
  );
}

#[test]
fn sigaction_rejects_sigkill_install_attempt() {
  let _lock = signal_lock();
  let newact = SigAction {
    sa_handler: 1,
    ..SigAction::default()
  };

  write_errno(0);
  // SAFETY: `newact` points to readable storage and `oldact == NULL` is allowed.
  let status = unsafe { sigaction(SIGKILL, &raw const newact, ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_sigstop_install_attempt() {
  let _lock = signal_lock();
  let newact = SigAction {
    sa_handler: 1,
    ..SigAction::default()
  };

  write_errno(0);
  // SAFETY: `newact` points to readable storage and `oldact == NULL` is allowed.
  let status = unsafe { sigaction(SIGSTOP, &raw const newact, ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_negative_signal_install_attempt() {
  let _lock = signal_lock();
  let newact = SigAction {
    sa_handler: 1,
    ..SigAction::default()
  };

  write_errno(0);
  // SAFETY: `newact` points to readable storage and `oldact == NULL` is allowed.
  let status = unsafe { sigaction(NEGATIVE_SIGNAL, &raw const newact, ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_signal_just_above_kernel_range_install_attempt() {
  let _lock = signal_lock();
  let newact = SigAction {
    sa_handler: 1,
    ..SigAction::default()
  };

  write_errno(0);
  // SAFETY: `newact` points to readable storage and `oldact == NULL` is allowed.
  let status = unsafe {
    sigaction(
      FIRST_OUT_OF_RANGE_SIGNAL,
      &raw const newact,
      ptr::null_mut(),
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_zero_signal_install_attempt() {
  let _lock = signal_lock();
  let newact = SigAction {
    sa_handler: 1,
    ..SigAction::default()
  };

  write_errno(0);
  // SAFETY: `newact` points to readable storage and `oldact == NULL` is allowed.
  let status = unsafe { sigaction(0, &raw const newact, ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_zero_signal_query() {
  let _lock = signal_lock();
  let mut oldact = SigAction::default();

  write_errno(0);
  // SAFETY: query mode with writable `oldact`.
  let status = unsafe { sigaction(0, ptr::null(), &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_negative_signal_query() {
  let _lock = signal_lock();
  let mut oldact = SigAction::default();

  write_errno(0);
  // SAFETY: query mode with writable `oldact`.
  let status = unsafe { sigaction(NEGATIVE_SIGNAL, ptr::null(), &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_signal_just_above_kernel_range() {
  let _lock = signal_lock();
  let mut oldact = SigAction::default();

  write_errno(0);
  // SAFETY: query mode with writable `oldact`.
  let status = unsafe { sigaction(FIRST_OUT_OF_RANGE_SIGNAL, ptr::null(), &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_invalid_signal_keeps_oldact_storage_unchanged() {
  let _lock = signal_lock();
  let mut oldact = SigAction {
    sa_handler: usize::MAX,
    sa_flags: c_ulong::MAX,
    sa_restorer: usize::MAX - 1,
    sa_mask: SigSet {
      bits: [c_ulong::MAX; 16],
    },
  };
  let before = oldact;

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query mode with writable `oldact`.
  let status = unsafe { sigaction(FIRST_OUT_OF_RANGE_SIGNAL, ptr::null(), &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    oldact, before,
    "oldact must remain unchanged when sigaction rejects signum"
  );
}

#[test]
fn sigaction_invalid_install_keeps_oldact_storage_unchanged() {
  let _lock = signal_lock();
  let newact = SigAction {
    sa_handler: 1,
    ..SigAction::default()
  };
  let mut oldact = SigAction {
    sa_handler: usize::MAX,
    sa_flags: c_ulong::MAX,
    sa_restorer: usize::MAX - 1,
    sa_mask: SigSet {
      bits: [c_ulong::MAX; 16],
    },
  };
  let before = oldact;

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid pointers for install/query with invalid signum.
  let status = unsafe {
    sigaction(
      FIRST_OUT_OF_RANGE_SIGNAL,
      &raw const newact,
      &raw mut oldact,
    )
  };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    oldact, before,
    "oldact must remain unchanged when sigaction install rejects signum",
  );
}

#[test]
fn sigaction_negative_install_keeps_oldact_storage_unchanged() {
  let _lock = signal_lock();
  let newact = SigAction {
    sa_handler: 1,
    ..SigAction::default()
  };
  let mut oldact = SigAction {
    sa_handler: usize::MAX,
    sa_flags: c_ulong::MAX,
    sa_restorer: usize::MAX - 1,
    sa_mask: SigSet {
      bits: [c_ulong::MAX; 16],
    },
  };
  let before = oldact;

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid pointers for install/query with invalid signum.
  let status = unsafe { sigaction(NEGATIVE_SIGNAL, &raw const newact, &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    oldact, before,
    "oldact must remain unchanged when sigaction install rejects negative signum",
  );
}

#[test]
fn sigaction_forbidden_install_keeps_oldact_storage_unchanged() {
  let _lock = signal_lock();
  let newact = SigAction {
    sa_handler: 1,
    ..SigAction::default()
  };
  let mut oldact = SigAction {
    sa_handler: usize::MAX,
    sa_flags: c_ulong::MAX,
    sa_restorer: usize::MAX - 1,
    sa_mask: SigSet {
      bits: [c_ulong::MAX; 16],
    },
  };
  let before = oldact;

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid pointers for install/query with forbidden signum.
  let status = unsafe { sigaction(SIGKILL, &raw const newact, &raw mut oldact) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    oldact, before,
    "oldact must remain unchanged when sigaction install rejects SIGKILL",
  );
}

#[test]
fn sigaction_query_clears_userspace_only_old_mask_words() {
  let _lock = signal_lock();
  let mut oldact = SigAction {
    sa_mask: SigSet {
      bits: [c_ulong::MAX; 16],
    },
    ..SigAction::default()
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query mode with writable `oldact` for a valid signal.
  let status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut oldact) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    oldact.sa_mask.bits[1..].iter().all(|word| *word == 0),
    "userspace-only oldact.sa_mask words must be cleared after successful query",
  );
}

#[test]
fn sigaction_query_with_null_oldact_succeeds_without_errno_change() {
  let _lock = signal_lock();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid signal query path with null `act` and null `oldact`.
  let status = unsafe { sigaction(SIGUSR1, ptr::null(), ptr::null_mut()) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sigprocmask_rejects_invalid_how() {
  let _lock = signal_lock();
  let set = SigSet::empty();
  let mut oldset = SigSet::empty();

  write_errno(0);
  // SAFETY: pointers are valid for requested access.
  let status = unsafe { sigprocmask(123, &raw const set, &raw mut oldset) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigprocmask_invalid_how_with_set_and_null_oldset_sets_einval() {
  let _lock = signal_lock();
  let set = SigSet::empty();

  write_errno(0);
  // SAFETY: readable `set` with null `oldset` is valid input shape.
  let status = unsafe { sigprocmask(123, &raw const set, ptr::null_mut()) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigprocmask_ignores_how_when_set_is_null() {
  let _lock = signal_lock();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: null `set` requests a no-op query path where Linux ignores `how`.
  let status = unsafe { sigprocmask(123, ptr::null(), ptr::null_mut()) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sigprocmask_ignores_extreme_negative_how_when_set_is_null() {
  let _lock = signal_lock();
  let mut oldset = SigSet {
    bits: [c_ulong::MAX; 16],
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: null `set` requests query semantics; Linux ignores `how`.
  let status = unsafe { sigprocmask(c_int::MIN, ptr::null(), &raw mut oldset) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    oldset.bits[1..].iter().all(|word| *word == c_ulong::MAX),
    "userspace-only oldset words should remain untouched by kernel-sized query",
  );
}

#[test]
fn sigprocmask_invalid_how_keeps_oldset_storage_unchanged() {
  let _lock = signal_lock();
  let set = SigSet::empty();
  let mut oldset = SigSet {
    bits: [c_ulong::MAX; 16],
  };
  let before = oldset;

  write_errno(ERRNO_SENTINEL);
  // SAFETY: pointers are valid for requested access.
  let status = unsafe { sigprocmask(123, &raw const set, &raw mut oldset) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    oldset, before,
    "oldset must remain unchanged when sigprocmask rejects invalid `how`",
  );
}

#[test]
fn sigprocmask_invalid_how_does_not_change_current_mask() {
  let _lock = signal_lock();
  let _restore = SigmaskRestoreGuard::capture();
  let mut target = SigSet::empty();
  let mut observed = SigSet::empty();

  assert!(target.add_signal(SIGUSR1));

  write_errno(ERRNO_SENTINEL);
  // SAFETY: unblocks SIGUSR1 before invalid request.
  let unblock_status = unsafe { sigprocmask(SIG_UNBLOCK, &raw const target, ptr::null_mut()) };

  assert_eq!(unblock_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: readable `set`; invalid `how` should fail without mutating mask.
  let invalid_status = unsafe { sigprocmask(123, &raw const target, ptr::null_mut()) };

  assert_eq!(invalid_status, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query current mask after invalid request.
  let read_status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut observed) };

  assert_eq!(read_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    !observed.contains_signal(SIGUSR1),
    "invalid how must not change thread mask state",
  );
}

#[test]
fn sigprocmask_invalid_how_with_oldset_does_not_change_current_mask() {
  let _lock = signal_lock();
  let _restore = SigmaskRestoreGuard::capture();
  let mut target = SigSet::empty();
  let mut oldset = SigSet {
    bits: [c_ulong::MAX; 16],
  };
  let before_oldset = oldset;
  let mut observed = SigSet::empty();

  assert!(target.add_signal(SIGUSR1));

  write_errno(ERRNO_SENTINEL);
  // SAFETY: unblocks SIGUSR1 before invalid request.
  let unblock_status = unsafe { sigprocmask(SIG_UNBLOCK, &raw const target, ptr::null_mut()) };

  assert_eq!(unblock_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: readable `set`, writable `oldset`; invalid `how` should fail
  // without mutating either state.
  let invalid_status = unsafe { sigprocmask(123, &raw const target, &raw mut oldset) };

  assert_eq!(invalid_status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    oldset, before_oldset,
    "invalid how must not mutate caller-provided oldset storage",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query current mask after invalid request.
  let read_status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut observed) };

  assert_eq!(read_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    !observed.contains_signal(SIGUSR1),
    "invalid how with non-null oldset must not change thread mask state",
  );
}

#[test]
fn sigprocmask_invalid_how_with_null_set_and_oldset_still_succeeds() {
  let _lock = signal_lock();
  let mut oldset = SigSet {
    bits: [c_ulong::MAX; 16],
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: null `set` triggers read-only query semantics where Linux ignores
  // `how` and writes current mask into `oldset`.
  let status = unsafe { sigprocmask(123, ptr::null(), &raw mut oldset) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    oldset.bits[1..].iter().all(|word| *word == c_ulong::MAX),
    "userspace-only oldset words should remain untouched by kernel-sized query",
  );
}

#[test]
fn sigprocmask_allows_read_only_oldset_with_null_set() {
  let _lock = signal_lock();
  let mut oldset = SigSet::empty();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: `oldset` is valid writable storage; null `set` means read-only query.
  let status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut oldset) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sigprocmask_query_preserves_userspace_only_oldset_words() {
  let _lock = signal_lock();
  let mut oldset = SigSet {
    bits: [c_ulong::MAX; 16],
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: `oldset` is writable and null `set` requests read-only query.
  let status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut oldset) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    oldset.bits[1..].iter().all(|word| *word == c_ulong::MAX),
    "userspace-only oldset words should remain untouched by kernel-sized query",
  );
}

#[test]
fn sigprocmask_block_with_oldset_preserves_userspace_only_words() {
  let _lock = signal_lock();
  let _restore = SigmaskRestoreGuard::capture();
  let mut target = SigSet::empty();
  let mut oldset = SigSet {
    bits: [c_ulong::MAX; 16],
  };

  assert!(target.add_signal(SIGUSR1));

  write_errno(ERRNO_SENTINEL);
  // SAFETY: `target` is readable and `oldset` is writable.
  let status = unsafe { sigprocmask(SIG_BLOCK, &raw const target, &raw mut oldset) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    oldset.bits[1..].iter().all(|word| *word == c_ulong::MAX),
    "userspace-only oldset words should remain untouched on successful SIG_BLOCK",
  );
}

#[test]
fn sigprocmask_setmask_with_oldset_preserves_userspace_only_words() {
  let _lock = signal_lock();
  let _restore = SigmaskRestoreGuard::capture();
  let mut target = SigSet::empty();
  let replacement = SigSet::empty();
  let mut oldset = SigSet {
    bits: [c_ulong::MAX; 16],
  };
  let mut observed = SigSet::empty();

  assert!(target.add_signal(SIGUSR1));

  write_errno(ERRNO_SENTINEL);
  // SAFETY: block SIGUSR1 so SIG_SETMASK has observable replacement effect.
  let block_status = unsafe { sigprocmask(SIG_BLOCK, &raw const target, ptr::null_mut()) };

  assert_eq!(block_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: replace current mask with an empty set and capture previous one.
  let setmask_status = unsafe { sigprocmask(SIG_SETMASK, &raw const replacement, &raw mut oldset) };

  assert_eq!(setmask_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    oldset.contains_signal(SIGUSR1),
    "oldset must report SIGUSR1 as previously blocked",
  );
  assert!(
    oldset.bits[1..].iter().all(|word| *word == c_ulong::MAX),
    "userspace-only oldset words should remain untouched on successful SIG_SETMASK",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query current mask after replacement.
  let read_status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut observed) };

  assert_eq!(read_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    !observed.contains_signal(SIGUSR1),
    "SIGUSR1 should be unblocked after SIG_SETMASK with empty replacement",
  );
}

#[test]
fn sigprocmask_setmask_with_null_oldset_replaces_mask() {
  let _lock = signal_lock();
  let _restore = SigmaskRestoreGuard::capture();
  let mut target = SigSet::empty();
  let replacement = SigSet::empty();
  let mut observed = SigSet::empty();

  assert!(target.add_signal(SIGUSR1));

  write_errno(ERRNO_SENTINEL);
  // SAFETY: block SIGUSR1 so SIG_SETMASK replacement is observable.
  let block_status = unsafe { sigprocmask(SIG_BLOCK, &raw const target, ptr::null_mut()) };

  assert_eq!(block_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: replace current mask with an empty set; null oldset is allowed.
  let setmask_status = unsafe { sigprocmask(SIG_SETMASK, &raw const replacement, ptr::null_mut()) };

  assert_eq!(setmask_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query current mask after replacement.
  let read_status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut observed) };

  assert_eq!(read_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    !observed.contains_signal(SIGUSR1),
    "SIGUSR1 should be unblocked after SIG_SETMASK with empty replacement and null oldset",
  );
}

#[test]
fn sigprocmask_unblock_with_oldset_preserves_userspace_only_words() {
  let _lock = signal_lock();
  let _restore = SigmaskRestoreGuard::capture();
  let mut target = SigSet::empty();
  let mut oldset = SigSet {
    bits: [c_ulong::MAX; 16],
  };
  let mut observed = SigSet::empty();

  assert!(target.add_signal(SIGUSR1));

  write_errno(ERRNO_SENTINEL);
  // SAFETY: block SIGUSR1 so SIG_UNBLOCK has observable effect.
  let block_status = unsafe { sigprocmask(SIG_BLOCK, &raw const target, ptr::null_mut()) };

  assert_eq!(block_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: unblock SIGUSR1 and capture previous mask in `oldset`.
  let unblock_status = unsafe { sigprocmask(SIG_UNBLOCK, &raw const target, &raw mut oldset) };

  assert_eq!(unblock_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    oldset.contains_signal(SIGUSR1),
    "oldset must report SIGUSR1 as previously blocked",
  );
  assert!(
    oldset.bits[1..].iter().all(|word| *word == c_ulong::MAX),
    "userspace-only oldset words should remain untouched on successful SIG_UNBLOCK",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query current mask after unblock.
  let read_status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut observed) };

  assert_eq!(read_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    !observed.contains_signal(SIGUSR1),
    "SIGUSR1 should be unblocked after successful SIG_UNBLOCK",
  );
}

#[test]
fn sigprocmask_noop_with_null_set_and_oldset_succeeds() {
  let _lock = signal_lock();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: null `set` + null `oldset` is a valid no-op query path.
  let status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), ptr::null_mut()) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sigprocmask_noop_with_null_set_and_oldset_ignores_extreme_how() {
  let _lock = signal_lock();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: null `set` + null `oldset` is a valid no-op query path where
  // `how` is ignored.
  let status = unsafe { sigprocmask(c_int::MIN, ptr::null(), ptr::null_mut()) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sigprocmask_block_and_unblock_sigusr1_roundtrip() {
  let _lock = signal_lock();
  let _restore = SigmaskRestoreGuard::capture();
  let mut target = SigSet::empty();
  let mut observed = SigSet::empty();

  assert!(target.add_signal(SIGUSR1));

  write_errno(ERRNO_SENTINEL);
  // SAFETY: `target` is readable and contains the signal to block.
  let block_status = unsafe { sigprocmask(SIG_BLOCK, &raw const target, ptr::null_mut()) };

  assert_eq!(block_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: read current mask into `observed`.
  let read_status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut observed) };

  assert_eq!(read_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    observed.contains_signal(SIGUSR1),
    "SIGUSR1 should be blocked after SIG_BLOCK",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: `target` is readable and contains the signal to unblock.
  let unblock_status = unsafe { sigprocmask(SIG_UNBLOCK, &raw const target, ptr::null_mut()) };

  assert_eq!(unblock_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  // SAFETY: read current mask after unblock.
  let verify_status = unsafe { sigprocmask(SIG_BLOCK, ptr::null(), &raw mut observed) };

  assert_eq!(verify_status, 0);
  assert!(
    !observed.contains_signal(SIGUSR1),
    "SIGUSR1 should be unblocked after SIG_UNBLOCK",
  );
}
