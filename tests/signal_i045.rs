#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::{c_int, c_void};
use core::ptr;
use rlibc::abi::errno::{EFAULT, EINVAL};
use rlibc::errno::__errno_location;
use rlibc::signal::{
  SA_RESTART, SA_RESTORER, SIGKILL, SIGSTOP, SIGUSR1, SigAction, SigSet, sigaction, sigaddset,
  sigdelset, sigemptyset, sigfillset, sigismember,
};
use std::env;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

const ERRNO_SENTINEL: c_int = 31337;
const MAX_KERNEL_SIGNAL: c_int = 64;
const CHILD_SCENARIO_ENV: &str = "RLIBC_SIGNAL_I045_SCENARIO";
const CHILD_SCENARIO_INVALID_ACT_POINTER: &str = "invalid_act_pointer";
const CHILD_SCENARIO_INVALID_ACT_POINTER_KEEPS_HANDLER: &str = "invalid_act_pointer_keeps_handler";
const CHILD_SCENARIO_INVALID_ACT_POINTER_KEEPS_OLDACT: &str = "invalid_act_pointer_keeps_oldact";
const CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_ACT: &str =
  "invalid_signal_precedence_over_invalid_act";
const CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_ACT: &str =
  "reserved_signal_precedence_over_invalid_act";
const CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_OLDACT: &str =
  "reserved_signal_precedence_over_invalid_oldact";
const CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_OLDACT: &str =
  "invalid_signal_precedence_over_invalid_oldact";
const CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_ACT_AND_OLDACT: &str =
  "invalid_signal_precedence_over_invalid_act_and_oldact";
const CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_ACT_AND_OLDACT: &str =
  "reserved_signal_precedence_over_invalid_act_and_oldact";
const CHILD_RUNNER_TEST: &str = "signal_i045_child_entrypoint";
const CHILD_EXIT_CODE_SUCCESS: c_int = 0;
const CHILD_EXIT_CODE_ASSERTION_FAILED: c_int = 1;
const CHILD_EXIT_CODE_MMAP_FAILED: c_int = 2;
const MAP_FAILED: isize = -1;
const MAP_ANONYMOUS: c_int = 0x20;
const MAP_PRIVATE: c_int = 0x02;
const PROT_NONE: c_int = 0;
const PAGE_SIZE: usize = 4096;

unsafe extern "C" {
  fn _Exit(status: c_int) -> !;
  fn mmap(
    addr: *mut c_void,
    len: usize,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: isize,
  ) -> *mut c_void;
  fn munmap(addr: *mut c_void, len: usize) -> c_int;
  fn raise(sig: c_int) -> c_int;
}

static HANDLER_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

struct SigactionRestoreGuard {
  previous: SigAction,
  restore_signal: c_int,
  armed: bool,
}

impl Drop for SigactionRestoreGuard {
  fn drop(&mut self) {
    if !self.armed {
      return;
    }

    // SAFETY: restoring a previously returned kernel action snapshot for the same signal.
    let _ = unsafe {
      sigaction(
        self.restore_signal,
        &raw const self.previous,
        ptr::null_mut(),
      )
    };
  }
}

extern "C" fn usr1_counter_handler(_signal: c_int) {
  HANDLER_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
}

fn signal_lock() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  LOCK
    .get_or_init(|| Mutex::new(()))
    .lock()
    .expect("signal test lock poisoned")
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

fn assert_invalid_signum_for_set_ops(signum: c_int) {
  let mut set = SigSet::empty();

  write_errno(0);
  // SAFETY: set pointer is valid and writable.
  let add_status = unsafe { sigaddset(&raw mut set, signum) };

  assert_eq!(add_status, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: set pointer is valid and writable.
  let del_status = unsafe { sigdelset(&raw mut set, signum) };

  assert_eq!(del_status, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: set pointer is valid and readable.
  let member_status = unsafe { sigismember(&raw const set, signum) };

  assert_eq!(member_status, -1);
  assert_eq!(read_errno(), EINVAL);
}

fn format_child_output(output: &Output) -> String {
  format!(
    "status={:?}, signal={:?}, stdout={:?}, stderr={:?}",
    output.status,
    output.status.signal(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  )
}

fn run_child_scenario(scenario: &str) -> Output {
  let current_executable = env::current_exe().expect("failed to resolve current test executable");

  Command::new(current_executable)
    .arg("--exact")
    .arg(CHILD_RUNNER_TEST)
    .arg("--nocapture")
    .env(CHILD_SCENARIO_ENV, scenario)
    .output()
    .expect("failed to execute signal_i045 child scenario")
}

fn run_invalid_act_pointer_child_scenario() -> ! {
  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_action = mapping.cast::<SigAction>().cast_const();

  write_errno(0);
  // SAFETY: intentionally passing an inaccessible action pointer; libc ABI
  // contract requires graceful `-1/EFAULT` instead of crashing.
  let status = unsafe { sigaction(SIGUSR1, invalid_action, ptr::null_mut()) };
  let errno_value = read_errno();

  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };

  if status == -1 && errno_value == EFAULT {
    // SAFETY: child process reports success to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
  }

  // SAFETY: child process reports assertion failure to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
}

fn run_invalid_act_pointer_keeps_handler_child_scenario() -> ! {
  let mut mask = SigSet::empty();

  // SAFETY: valid pointer for writable signal-set storage.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  if empty_status != 0 {
    // SAFETY: child process reports assertion failure to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
  }

  HANDLER_CALL_COUNT.store(0, Ordering::SeqCst);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };

  // SAFETY: valid install action pointer.
  let install_status = unsafe { sigaction(SIGUSR1, &raw const install_action, ptr::null_mut()) };

  if install_status != 0 {
    // SAFETY: child process reports assertion failure to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
  }

  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_action = mapping.cast::<SigAction>().cast_const();

  write_errno(0);
  // SAFETY: intentionally passing an inaccessible action pointer; contract is
  // `-1/EFAULT` without mutating existing disposition.
  let invalid_status = unsafe { sigaction(SIGUSR1, invalid_action, ptr::null_mut()) };
  let invalid_errno = read_errno();

  if invalid_status != -1 || invalid_errno != EFAULT {
    // SAFETY: best-effort cleanup before exiting child scenario.
    let _result = unsafe { munmap(mapping, PAGE_SIZE) };
    // SAFETY: child process reports assertion failure to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
  }

  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };

  // SAFETY: handler was installed above on SIGUSR1.
  let raise_status = unsafe { raise(SIGUSR1) };

  if raise_status != 0 || HANDLER_CALL_COUNT.load(Ordering::SeqCst) != 1 {
    // SAFETY: child process reports assertion failure to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
  }

  // SAFETY: child process reports success to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
}

fn run_invalid_act_pointer_keeps_oldact_child_scenario() -> ! {
  let sentinel = SigAction {
    sa_handler: usize::MAX - 1,
    sa_flags: SA_RESTART,
    sa_restorer: usize::MAX,
    sa_mask: SigSet {
      bits: [u64::MAX; 16],
    },
  };
  let mut old_action = sentinel;
  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_action = mapping.cast::<SigAction>().cast_const();

  write_errno(0);
  // SAFETY: intentionally passing an unreadable `act` pointer while using a
  // valid writable `oldact` buffer.
  let status = unsafe { sigaction(SIGUSR1, invalid_action, &raw mut old_action) };
  let errno_value = read_errno();
  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };

  if status == -1 && errno_value == EFAULT && old_action == sentinel {
    // SAFETY: child process reports success to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
  }

  // SAFETY: child process reports assertion failure to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
}

fn run_invalid_signal_precedence_over_invalid_act_child_scenario() -> ! {
  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_action = mapping.cast::<SigAction>().cast_const();

  write_errno(0);
  // SAFETY: invalid signal number must be rejected before dereferencing `act`.
  let status = unsafe { sigaction(0, invalid_action, ptr::null_mut()) };
  let errno_value = read_errno();
  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };

  if status == -1 && errno_value == EINVAL {
    // SAFETY: child process reports success to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
  }

  // SAFETY: child process reports assertion failure to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
}

fn run_reserved_signal_precedence_over_invalid_act_child_scenario() -> ! {
  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_action = mapping.cast::<SigAction>().cast_const();

  for reserved_signum in [SIGKILL, SIGSTOP] {
    write_errno(0);
    // SAFETY: reserved signals must be rejected before touching unreadable `act`.
    let status = unsafe { sigaction(reserved_signum, invalid_action, ptr::null_mut()) };
    let errno_value = read_errno();

    if status != -1 || errno_value != EINVAL {
      // SAFETY: best-effort cleanup for child scenario resources.
      let _result = unsafe { munmap(mapping, PAGE_SIZE) };
      // SAFETY: child process reports assertion failure to parent test.
      unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
    }
  }

  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };
  // SAFETY: child process reports success to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
}

fn run_reserved_signal_precedence_over_invalid_oldact_child_scenario() -> ! {
  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_old_action = mapping.cast::<SigAction>();

  for reserved_signum in [SIGKILL, SIGSTOP] {
    write_errno(0);
    // SAFETY: reserved signals must be rejected before touching unreadable `oldact`.
    let status = unsafe { sigaction(reserved_signum, ptr::null(), invalid_old_action) };
    let errno_value = read_errno();

    if status != -1 || errno_value != EINVAL {
      // SAFETY: best-effort cleanup for child scenario resources.
      let _result = unsafe { munmap(mapping, PAGE_SIZE) };
      // SAFETY: child process reports assertion failure to parent test.
      unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
    }
  }

  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };
  // SAFETY: child process reports success to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
}

fn run_invalid_signal_precedence_over_invalid_oldact_child_scenario() -> ! {
  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_old_action = mapping.cast::<SigAction>();

  write_errno(0);
  // SAFETY: invalid signal number must be rejected before touching unreadable `oldact`.
  let status = unsafe { sigaction(0, ptr::null(), invalid_old_action) };
  let errno_value = read_errno();
  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };

  if status == -1 && errno_value == EINVAL {
    // SAFETY: child process reports success to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
  }

  // SAFETY: child process reports assertion failure to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
}

fn run_invalid_signal_precedence_over_invalid_act_and_oldact_child_scenario() -> ! {
  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_action = mapping.cast::<SigAction>().cast_const();
  let invalid_old_action = mapping.cast::<SigAction>();

  write_errno(0);
  // SAFETY: invalid signal number must be rejected before touching unreadable
  // `act` and unreadable `oldact`.
  let status = unsafe { sigaction(0, invalid_action, invalid_old_action) };
  let errno_value = read_errno();
  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };

  if status == -1 && errno_value == EINVAL {
    // SAFETY: child process reports success to parent test.
    unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
  }

  // SAFETY: child process reports assertion failure to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
}

fn run_reserved_signal_precedence_over_invalid_act_and_oldact_child_scenario() -> ! {
  // SAFETY: anonymous `PROT_NONE` mapping is used to force inaccessible memory.
  let mapping = unsafe {
    mmap(
      ptr::null_mut(),
      PAGE_SIZE,
      PROT_NONE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };

  if mapping as isize == MAP_FAILED {
    // SAFETY: child process exits immediately on setup failure.
    unsafe { _Exit(CHILD_EXIT_CODE_MMAP_FAILED) };
  }

  let invalid_action = mapping.cast::<SigAction>().cast_const();
  let invalid_old_action = mapping.cast::<SigAction>();

  for reserved_signum in [SIGKILL, SIGSTOP] {
    write_errno(0);
    // SAFETY: reserved signals must be rejected before touching unreadable
    // `act` and unreadable `oldact`.
    let status = unsafe { sigaction(reserved_signum, invalid_action, invalid_old_action) };
    let errno_value = read_errno();

    if status != -1 || errno_value != EINVAL {
      // SAFETY: best-effort cleanup for child scenario resources.
      let _result = unsafe { munmap(mapping, PAGE_SIZE) };
      // SAFETY: child process reports assertion failure to parent test.
      unsafe { _Exit(CHILD_EXIT_CODE_ASSERTION_FAILED) };
    }
  }

  // SAFETY: best-effort cleanup for child scenario resources.
  let _result = unsafe { munmap(mapping, PAGE_SIZE) };
  // SAFETY: child process reports success to parent test.
  unsafe { _Exit(CHILD_EXIT_CODE_SUCCESS) };
}

#[test]
fn sigemptyset_clears_set_and_keeps_errno_on_success() {
  let _lock = signal_lock();
  let mut set = SigSet::empty();

  set.bits.fill(!0);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let status = unsafe { sigemptyset(&raw mut set) };

  assert_eq!(status, 0);
  assert!(set.bits.iter().all(|word| *word == 0));
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sigfillset_marks_kernel_signal_range_and_keeps_errno() {
  let _lock = signal_lock();
  let mut set = SigSet::empty();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let status = unsafe { sigfillset(&raw mut set) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and readable.
  assert_eq!(unsafe { sigismember(&raw const set, 1) }, 1);
  // SAFETY: set pointer is valid and readable.
  assert_eq!(unsafe { sigismember(&raw const set, SIGUSR1) }, 1);
  // SAFETY: set pointer is valid and readable.
  assert_eq!(unsafe { sigismember(&raw const set, MAX_KERNEL_SIGNAL) }, 1);
}

#[test]
fn sigaddset_and_sigdelset_roundtrip_membership() {
  let _lock = signal_lock();
  let mut set = SigSet::empty();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let add_status = unsafe { sigaddset(&raw mut set, SIGUSR1) };

  assert_eq!(add_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  // SAFETY: set pointer is valid and readable.
  let after_add = unsafe { sigismember(&raw const set, SIGUSR1) };

  assert_eq!(after_add, 1);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let del_status = unsafe { sigdelset(&raw mut set, SIGUSR1) };

  assert_eq!(del_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  // SAFETY: set pointer is valid and readable.
  let after_del = unsafe { sigismember(&raw const set, SIGUSR1) };

  assert_eq!(after_del, 0);
}

#[test]
fn set_operations_are_idempotent_for_existing_and_absent_members() {
  let _lock = signal_lock();
  let mut set = SigSet::empty();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let first_add_status = unsafe { sigaddset(&raw mut set, SIGUSR1) };

  assert_eq!(first_add_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let second_add_status = unsafe { sigaddset(&raw mut set, SIGUSR1) };

  assert_eq!(second_add_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and readable.
  assert_eq!(unsafe { sigismember(&raw const set, SIGUSR1) }, 1);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let first_del_status = unsafe { sigdelset(&raw mut set, 1) };

  assert_eq!(first_del_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let second_del_status = unsafe { sigdelset(&raw mut set, 1) };

  assert_eq!(second_del_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and readable.
  assert_eq!(unsafe { sigismember(&raw const set, 1) }, 0);
}

#[test]
fn set_operations_handle_boundary_signal_numbers() {
  let _lock = signal_lock();
  let mut set = SigSet::empty();

  for signum in [1, MAX_KERNEL_SIGNAL] {
    write_errno(ERRNO_SENTINEL);
    // SAFETY: set pointer is valid and writable.
    let add_status = unsafe { sigaddset(&raw mut set, signum) };

    assert_eq!(add_status, 0);
    assert_eq!(read_errno(), ERRNO_SENTINEL);

    // SAFETY: set pointer is valid and readable.
    let member_after_add = unsafe { sigismember(&raw const set, signum) };

    assert_eq!(member_after_add, 1);

    write_errno(ERRNO_SENTINEL);
    // SAFETY: set pointer is valid and writable.
    let del_status = unsafe { sigdelset(&raw mut set, signum) };

    assert_eq!(del_status, 0);
    assert_eq!(read_errno(), ERRNO_SENTINEL);

    // SAFETY: set pointer is valid and readable.
    let member_after_del = unsafe { sigismember(&raw const set, signum) };

    assert_eq!(member_after_del, 0);
  }
}

#[test]
fn set_operations_reject_invalid_signal_numbers() {
  let _lock = signal_lock();

  assert_invalid_signum_for_set_ops(0);
  assert_invalid_signum_for_set_ops(-1);
  assert_invalid_signum_for_set_ops(MAX_KERNEL_SIGNAL + 1);
}

#[test]
fn invalid_set_operations_preserve_existing_membership() {
  let _lock = signal_lock();
  let mut set = SigSet::empty();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let add_usr1_status = unsafe { sigaddset(&raw mut set, SIGUSR1) };

  assert_eq!(add_usr1_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: set pointer is valid and writable.
  let add_sig1_status = unsafe { sigaddset(&raw mut set, 1) };

  assert_eq!(add_sig1_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let baseline = set;

  write_errno(0);
  // SAFETY: set pointer is valid and writable; invalid signum is intentional.
  let invalid_add_status = unsafe { sigaddset(&raw mut set, 0) };

  assert_eq!(invalid_add_status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    set, baseline,
    "invalid sigaddset must not mutate existing membership",
  );

  write_errno(0);
  // SAFETY: set pointer is valid and writable; invalid signum is intentional.
  let invalid_del_status = unsafe { sigdelset(&raw mut set, MAX_KERNEL_SIGNAL + 1) };

  assert_eq!(invalid_del_status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    set, baseline,
    "invalid sigdelset must not mutate existing membership",
  );

  write_errno(0);
  // SAFETY: set pointer is valid and readable; invalid signum is intentional.
  let invalid_member_status = unsafe { sigismember(&raw const set, -1) };

  assert_eq!(invalid_member_status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    set, baseline,
    "invalid sigismember must not mutate set membership",
  );
}

#[test]
fn set_operations_reject_null_set_pointer() {
  let _lock = signal_lock();

  write_errno(0);
  // SAFETY: null pointer is intentional for error-path validation.
  let empty_status = unsafe { sigemptyset(ptr::null_mut()) };

  assert_eq!(empty_status, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);
  // SAFETY: null pointer is intentional for error-path validation.
  let fill_status = unsafe { sigfillset(ptr::null_mut()) };

  assert_eq!(fill_status, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);
  // SAFETY: null pointer is intentional for error-path validation.
  let add_status = unsafe { sigaddset(ptr::null_mut(), SIGUSR1) };

  assert_eq!(add_status, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);
  // SAFETY: null pointer is intentional for error-path validation.
  let del_status = unsafe { sigdelset(ptr::null_mut(), SIGUSR1) };

  assert_eq!(del_status, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);
  // SAFETY: null pointer is intentional for error-path validation.
  let member_status = unsafe { sigismember(ptr::null(), SIGUSR1) };

  assert_eq!(member_status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn null_set_pointer_takes_precedence_over_invalid_signal_numbers() {
  let _lock = signal_lock();

  write_errno(0);
  // SAFETY: null pointer and invalid signum are intentional; pointer validity
  // must be checked before signal-range validation.
  let add_status = unsafe { sigaddset(ptr::null_mut(), 0) };

  assert_eq!(add_status, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);
  // SAFETY: null pointer and invalid signum are intentional; pointer validity
  // must be checked before signal-range validation.
  let del_status = unsafe { sigdelset(ptr::null_mut(), MAX_KERNEL_SIGNAL + 1) };

  assert_eq!(del_status, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);
  // SAFETY: null pointer and invalid signum are intentional; pointer validity
  // must be checked before signal-range validation.
  let member_status = unsafe { sigismember(ptr::null(), -1) };

  assert_eq!(member_status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn sigaction_rejects_reserved_signals() {
  let _lock = signal_lock();
  let mut old_action = SigAction::default();

  write_errno(0);
  // SAFETY: valid writable `old_action`; testing reserved signal rejection.
  let kill_status = unsafe { sigaction(SIGKILL, ptr::null(), &raw mut old_action) };

  assert_eq!(kill_status, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);
  // SAFETY: valid writable `old_action`; testing reserved signal rejection.
  let stop_status = unsafe { sigaction(SIGSTOP, ptr::null(), &raw mut old_action) };

  assert_eq!(stop_status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_rejects_out_of_range_signals() {
  let _lock = signal_lock();
  let mut old_action = SigAction::default();

  for invalid_signum in [0, -1, MAX_KERNEL_SIGNAL + 1] {
    write_errno(0);
    // SAFETY: `old_action` is valid writable storage; invalid signum is intentional.
    let status = unsafe { sigaction(invalid_signum, ptr::null(), &raw mut old_action) };

    assert_eq!(status, -1, "signum={invalid_signum} must be rejected");
    assert_eq!(
      read_errno(),
      EINVAL,
      "signum={invalid_signum} must report EINVAL",
    );
  }
}

#[test]
fn sigaction_invalid_signal_takes_precedence_over_pointer_validation() {
  let _lock = signal_lock();
  let dangling = std::ptr::NonNull::<SigAction>::dangling();
  let old_action = dangling.as_ptr();
  let action = old_action.cast_const();

  write_errno(0);
  // SAFETY: invalid `signum` is intentional and must be rejected before any
  // pointer dereference; dangling pointers are not accessed in this path.
  let status = unsafe { sigaction(0, action, old_action) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn sigaction_reserved_signal_takes_precedence_over_pointer_validation() {
  let _lock = signal_lock();
  let dangling = std::ptr::NonNull::<SigAction>::dangling();
  let old_action = dangling.as_ptr();
  let action = old_action.cast_const();

  for reserved_signum in [SIGKILL, SIGSTOP] {
    write_errno(0);
    // SAFETY: reserved signals must be rejected before any pointer access; the
    // dangling pointers are intentionally unreachable on this validation path.
    let status = unsafe { sigaction(reserved_signum, action, old_action) };

    assert_eq!(
      status, -1,
      "reserved signal {reserved_signum} must be rejected",
    );
    assert_eq!(
      read_errno(),
      EINVAL,
      "reserved signal {reserved_signum} must set EINVAL",
    );
  }
}

#[test]
fn sigaction_query_with_invalid_oldact_pointer_reports_efault() {
  let _lock = signal_lock();
  let dangling = std::ptr::NonNull::<SigAction>::dangling();
  let old_action = dangling.as_ptr();

  write_errno(0);
  // SAFETY: valid signal with intentionally invalid output pointer should be
  // rejected by kernel with EFAULT on the query-only path.
  let status = unsafe { sigaction(SIGUSR1, ptr::null(), old_action) };

  assert_eq!(status, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn sigaction_query_with_invalid_oldact_pointer_keeps_current_handler_active() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();
  let mut previous_action = SigAction::default();

  HANDLER_CALL_COUNT.store(0, Ordering::SeqCst);

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: installing handler with valid read/write pointers.
  let install_status =
    unsafe { sigaction(SIGUSR1, &raw const install_action, &raw mut previous_action) };

  assert_eq!(install_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let mut restore_guard = SigactionRestoreGuard {
    previous: previous_action,
    restore_signal: SIGUSR1,
    armed: true,
  };
  let dangling = std::ptr::NonNull::<SigAction>::dangling();
  let invalid_old_action = dangling.as_ptr();

  write_errno(0);
  // SAFETY: query path with intentionally invalid output pointer.
  let query_status = unsafe { sigaction(SIGUSR1, ptr::null(), invalid_old_action) };

  assert_eq!(query_status, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: handler was installed above on SIGUSR1.
  let raise_status = unsafe { raise(SIGUSR1) };

  assert_eq!(raise_status, 0);
  assert_eq!(
    HANDLER_CALL_COUNT.load(Ordering::SeqCst),
    1,
    "failed query with invalid oldact must not uninstall current handler",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously returned action for `SIGUSR1`.
  let restore_status =
    unsafe { sigaction(SIGUSR1, &raw const restore_guard.previous, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  restore_guard.armed = false;
}

#[test]
fn sigaction_invalid_oldact_pointer_reports_efault_and_still_updates_disposition() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();
  let mut before_action = SigAction::default();
  let mut after_action = SigAction::default();

  HANDLER_CALL_COUNT.store(0, Ordering::SeqCst);

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query into valid writable storage.
  let before_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut before_action) };

  assert_eq!(before_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };
  let dangling = std::ptr::NonNull::<SigAction>::dangling();
  let invalid_old_action = dangling.as_ptr();

  write_errno(0);
  // SAFETY: valid install action with intentionally invalid oldact pointer;
  // kernel reports EFAULT while still applying the new action on Linux.
  let invalid_status = unsafe { sigaction(SIGUSR1, &raw const install_action, invalid_old_action) };

  assert_eq!(invalid_status, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: query into valid writable storage.
  let after_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut after_action) };

  assert_eq!(after_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert_eq!(
    after_action.sa_handler, install_action.sa_handler,
    "invalid oldact still installs the requested handler",
  );
  assert_eq!(
    after_action.sa_mask, install_action.sa_mask,
    "invalid oldact path must preserve requested mask bits",
  );
  assert_eq!(
    after_action.sa_flags & SA_RESTART,
    SA_RESTART,
    "invalid oldact path must preserve requested restart policy",
  );
  assert_ne!(
    after_action.sa_flags & SA_RESTORER,
    0,
    "invalid oldact install path must keep kernel restorer flag",
  );
  assert_ne!(
    after_action.sa_restorer, 0,
    "invalid oldact install path must keep non-null restorer pointer",
  );

  // SAFETY: handler was installed above on SIGUSR1.
  let raise_status = unsafe { raise(SIGUSR1) };

  assert_eq!(raise_status, 0);
  assert_eq!(
    HANDLER_CALL_COUNT.load(Ordering::SeqCst),
    1,
    "installed handler must run even when sigaction returned EFAULT",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restore previously queried action snapshot for test isolation.
  let restore_status = unsafe { sigaction(SIGUSR1, &raw const before_action, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sigaction_install_with_null_oldact_succeeds_and_keeps_errno() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();
  let mut previous_action = SigAction::default();

  HANDLER_CALL_COUNT.store(0, Ordering::SeqCst);

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid output pointer for querying current action.
  let query_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut previous_action) };

  assert_eq!(query_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid action pointer; oldact intentionally null.
  let install_status = unsafe { sigaction(SIGUSR1, &raw const install_action, ptr::null_mut()) };

  assert_eq!(install_status, 0);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful install with null oldact must not clobber errno",
  );

  // SAFETY: `raise` takes plain signal number and handler was installed above.
  let raise_status = unsafe { raise(SIGUSR1) };

  assert_eq!(raise_status, 0);
  assert_eq!(HANDLER_CALL_COUNT.load(Ordering::SeqCst), 1);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously queried action snapshot.
  let restore_status = unsafe { sigaction(SIGUSR1, &raw const previous_action, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
}

#[test]
fn sigaction_validation_failure_keeps_oldact_unchanged() {
  let _lock = signal_lock();
  let sentinel = SigAction {
    sa_handler: usize::MAX - 1,
    sa_flags: SA_RESTART,
    sa_restorer: usize::MAX,
    sa_mask: SigSet {
      bits: [u64::MAX; 16],
    },
  };
  let mut old_action = sentinel;

  write_errno(0);
  // SAFETY: invalid signal is intentional; function must reject before touching
  // the output slot.
  let invalid_status = unsafe { sigaction(0, ptr::null(), &raw mut old_action) };

  assert_eq!(invalid_status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    old_action, sentinel,
    "validation failure must not mutate oldact output buffer",
  );

  write_errno(0);
  // SAFETY: reserved signal is intentional; function must reject before touching
  // the output slot.
  let reserved_status = unsafe { sigaction(SIGKILL, ptr::null(), &raw mut old_action) };

  assert_eq!(reserved_status, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(
    old_action, sentinel,
    "reserved-signal rejection must not mutate oldact output buffer",
  );
}

#[test]
fn sigaction_installs_handler_and_restores_previous_action() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();

  HANDLER_CALL_COUNT.store(0, Ordering::SeqCst);

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };
  let mut previous_action = SigAction::default();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: pointers are valid for kernel read/write as required by `sigaction`.
  let install_status =
    unsafe { sigaction(SIGUSR1, &raw const install_action, &raw mut previous_action) };

  assert_eq!(install_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let mut restore_guard = SigactionRestoreGuard {
    previous: previous_action,
    restore_signal: SIGUSR1,
    armed: true,
  };

  // SAFETY: `raise` takes a plain signal number; handler was installed above.
  let raise_status = unsafe { raise(SIGUSR1) };

  assert_eq!(raise_status, 0);
  assert_eq!(HANDLER_CALL_COUNT.load(Ordering::SeqCst), 1);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously returned action for `SIGUSR1`.
  let restore_status =
    unsafe { sigaction(SIGUSR1, &raw const restore_guard.previous, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  restore_guard.armed = false;
}

#[test]
fn sigaction_query_clears_non_kernel_old_mask_words() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();

  HANDLER_CALL_COUNT.store(0, Ordering::SeqCst);

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };
  let mut previous_action = SigAction::default();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: pointers are valid for kernel read/write as required by `sigaction`.
  let install_status =
    unsafe { sigaction(SIGUSR1, &raw const install_action, &raw mut previous_action) };

  assert_eq!(install_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let mut restore_guard = SigactionRestoreGuard {
    previous: previous_action,
    restore_signal: SIGUSR1,
    armed: true,
  };
  let mut queried_action = SigAction::default();

  queried_action.sa_mask.bits.fill(!0);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: querying action with a valid output pointer.
  let query_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut queried_action) };

  assert_eq!(query_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    queried_action.sa_mask.bits[1..]
      .iter()
      .all(|word| *word == 0),
    "oldact mask words above kernel range must be cleared",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously returned action for `SIGUSR1`.
  let restore_status =
    unsafe { sigaction(SIGUSR1, &raw const restore_guard.previous, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  restore_guard.armed = false;
}

#[test]
fn sigaction_auto_sets_restorer_when_omitted() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };
  let mut previous_action = SigAction::default();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: pointers are valid for kernel read/write as required by `sigaction`.
  let install_status =
    unsafe { sigaction(SIGUSR1, &raw const install_action, &raw mut previous_action) };

  assert_eq!(install_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let mut restore_guard = SigactionRestoreGuard {
    previous: previous_action,
    restore_signal: SIGUSR1,
    armed: true,
  };
  let mut queried_action = SigAction::default();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: querying action with a valid output pointer.
  let query_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut queried_action) };

  assert_eq!(query_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert_ne!(
    queried_action.sa_flags & SA_RESTORER,
    0,
    "kernel-facing action must include SA_RESTORER",
  );
  assert_ne!(
    queried_action.sa_restorer, 0,
    "restorer trampoline pointer must be non-null",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously returned action for `SIGUSR1`.
  let restore_status =
    unsafe { sigaction(SIGUSR1, &raw const restore_guard.previous, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  restore_guard.armed = false;
}

#[test]
fn sigaction_fills_restorer_when_flag_is_set_but_pointer_is_zero() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART | SA_RESTORER,
    sa_restorer: 0,
    sa_mask: mask,
  };
  let mut previous_action = SigAction::default();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: pointers are valid for kernel read/write as required by `sigaction`.
  let install_status =
    unsafe { sigaction(SIGUSR1, &raw const install_action, &raw mut previous_action) };

  assert_eq!(install_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let mut restore_guard = SigactionRestoreGuard {
    previous: previous_action,
    restore_signal: SIGUSR1,
    armed: true,
  };
  let mut queried_action = SigAction::default();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: querying action with a valid output pointer.
  let query_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut queried_action) };

  assert_eq!(query_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert_ne!(queried_action.sa_flags & SA_RESTORER, 0);
  assert_ne!(
    queried_action.sa_restorer, 0,
    "restorer must be auto-filled when caller passes zero pointer",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously returned action for `SIGUSR1`.
  let restore_status =
    unsafe { sigaction(SIGUSR1, &raw const restore_guard.previous, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  restore_guard.armed = false;
}

#[test]
fn sigaction_query_preserves_kernel_visible_mask_words() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();

  assert!(mask.add_signal(SIGUSR1));
  assert!(mask.add_signal(MAX_KERNEL_SIGNAL));

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };
  let mut previous_action = SigAction::default();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: pointers are valid for kernel read/write as required by `sigaction`.
  let install_status =
    unsafe { sigaction(SIGUSR1, &raw const install_action, &raw mut previous_action) };

  assert_eq!(install_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let mut restore_guard = SigactionRestoreGuard {
    previous: previous_action,
    restore_signal: SIGUSR1,
    armed: true,
  };
  let mut queried_action = SigAction::default();

  queried_action.sa_mask.bits.fill(!0);
  write_errno(ERRNO_SENTINEL);
  // SAFETY: querying action with a valid output pointer.
  let query_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut queried_action) };

  assert_eq!(query_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert!(
    queried_action.sa_mask.contains_signal(SIGUSR1),
    "kernel-visible mask bits must remain observable after normalization",
  );
  assert!(
    queried_action.sa_mask.contains_signal(MAX_KERNEL_SIGNAL),
    "highest kernel-visible signal bit must remain set after normalization",
  );
  assert!(
    queried_action.sa_mask.bits[1..]
      .iter()
      .all(|word| *word == 0),
    "userspace-only mask words above kernel range must be cleared",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously returned action for `SIGUSR1`.
  let restore_status =
    unsafe { sigaction(SIGUSR1, &raw const restore_guard.previous, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  restore_guard.armed = false;
}

#[test]
fn sigaction_allows_null_act_and_null_oldact_for_valid_signal() {
  let _lock = signal_lock();

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid signal number with intentional null pointers for query/set slots.
  let status = unsafe { sigaction(SIGUSR1, ptr::null(), ptr::null_mut()) };

  assert_eq!(status, 0);
  assert_eq!(
    read_errno(),
    ERRNO_SENTINEL,
    "successful no-op sigaction call must not clobber errno",
  );
}

#[test]
fn sigaction_noop_with_null_act_and_oldact_keeps_installed_handler_active() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();
  let mut previous_action = SigAction::default();

  HANDLER_CALL_COUNT.store(0, Ordering::SeqCst);

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: pointers are valid for kernel read/write as required by `sigaction`.
  let install_status =
    unsafe { sigaction(SIGUSR1, &raw const install_action, &raw mut previous_action) };

  assert_eq!(install_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let mut restore_guard = SigactionRestoreGuard {
    previous: previous_action,
    restore_signal: SIGUSR1,
    armed: true,
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid signal with intentional no-op null pointers.
  let noop_status = unsafe { sigaction(SIGUSR1, ptr::null(), ptr::null_mut()) };

  assert_eq!(noop_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  // SAFETY: handler was installed above on SIGUSR1.
  let raise_status = unsafe { raise(SIGUSR1) };

  assert_eq!(raise_status, 0);
  assert_eq!(
    HANDLER_CALL_COUNT.load(Ordering::SeqCst),
    1,
    "no-op sigaction must not uninstall the currently configured handler",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously returned action for `SIGUSR1`.
  let restore_status =
    unsafe { sigaction(SIGUSR1, &raw const restore_guard.previous, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  restore_guard.armed = false;
}

#[test]
fn sigaction_noop_with_null_act_and_oldact_keeps_disposition_snapshot() {
  let _lock = signal_lock();
  let mut mask = SigSet::empty();
  let mut previous_action = SigAction::default();
  let mut before_noop = SigAction::default();
  let mut after_noop = SigAction::default();

  // SAFETY: set pointer is valid and writable.
  let empty_status = unsafe { sigemptyset(&raw mut mask) };

  assert_eq!(empty_status, 0);
  // Keep one visible mask bit to assert no-op does not alter queried mask.
  assert!(mask.add_signal(SIGUSR1));

  let install_action = SigAction {
    sa_handler: usr1_counter_handler as *const () as usize,
    sa_flags: SA_RESTART,
    sa_restorer: 0,
    sa_mask: mask,
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: pointers are valid for kernel read/write as required by `sigaction`.
  let install_status =
    unsafe { sigaction(SIGUSR1, &raw const install_action, &raw mut previous_action) };

  assert_eq!(install_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  let mut restore_guard = SigactionRestoreGuard {
    previous: previous_action,
    restore_signal: SIGUSR1,
    armed: true,
  };

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid query pointer.
  let query_before_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut before_noop) };

  assert_eq!(query_before_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid signal with intentional no-op null pointers.
  let noop_status = unsafe { sigaction(SIGUSR1, ptr::null(), ptr::null_mut()) };

  assert_eq!(noop_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);

  write_errno(ERRNO_SENTINEL);
  // SAFETY: valid query pointer.
  let query_after_status = unsafe { sigaction(SIGUSR1, ptr::null(), &raw mut after_noop) };

  assert_eq!(query_after_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  assert_eq!(
    after_noop, before_noop,
    "no-op sigaction must keep queried disposition snapshot unchanged",
  );

  write_errno(ERRNO_SENTINEL);
  // SAFETY: restoring previously returned action for `SIGUSR1`.
  let restore_status =
    unsafe { sigaction(SIGUSR1, &raw const restore_guard.previous, ptr::null_mut()) };

  assert_eq!(restore_status, 0);
  assert_eq!(read_errno(), ERRNO_SENTINEL);
  restore_guard.armed = false;
}

#[test]
fn sigaction_invalid_act_pointer_reports_efault_without_child_segfault() {
  let _lock = signal_lock();
  let output = run_child_scenario(CHILD_SCENARIO_INVALID_ACT_POINTER);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "invalid-act child scenario must report -1/EFAULT without SIGSEGV: {}",
    format_child_output(&output),
  );
}

#[test]
fn sigaction_invalid_act_pointer_keeps_existing_handler_active() {
  let _lock = signal_lock();
  let output = run_child_scenario(CHILD_SCENARIO_INVALID_ACT_POINTER_KEEPS_HANDLER);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "invalid-act child scenario must keep pre-installed handler active: {}",
    format_child_output(&output),
  );
}

#[test]
fn sigaction_invalid_act_pointer_keeps_oldact_buffer_unchanged() {
  let _lock = signal_lock();
  let output = run_child_scenario(CHILD_SCENARIO_INVALID_ACT_POINTER_KEEPS_OLDACT);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "invalid-act child scenario must not mutate oldact output buffer: {}",
    format_child_output(&output),
  );
}

#[test]
fn sigaction_invalid_signal_takes_precedence_over_unreadable_act_pointer() {
  let _lock = signal_lock();
  let output = run_child_scenario(CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_ACT);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "invalid signal must be rejected before touching unreadable act pointer: {}",
    format_child_output(&output),
  );
}

#[test]
fn sigaction_reserved_signal_takes_precedence_over_unreadable_act_pointer() {
  let _lock = signal_lock();
  let output = run_child_scenario(CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_ACT);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "reserved signals must be rejected before touching unreadable act pointer: {}",
    format_child_output(&output),
  );
}

#[test]
fn sigaction_reserved_signal_takes_precedence_over_unreadable_oldact_pointer() {
  let _lock = signal_lock();
  let output = run_child_scenario(CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_OLDACT);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "reserved signals must be rejected before touching unreadable oldact pointer: {}",
    format_child_output(&output),
  );
}

#[test]
fn sigaction_invalid_signal_takes_precedence_over_unreadable_oldact_pointer() {
  let _lock = signal_lock();
  let output = run_child_scenario(CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_OLDACT);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "invalid signal must be rejected before touching unreadable oldact pointer: {}",
    format_child_output(&output),
  );
}

#[test]
fn sigaction_invalid_signal_takes_precedence_over_unreadable_act_and_oldact_pointers() {
  let _lock = signal_lock();
  let output =
    run_child_scenario(CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_ACT_AND_OLDACT);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "invalid signal must be rejected before touching unreadable act/oldact pointers: {}",
    format_child_output(&output),
  );
}

#[test]
fn sigaction_reserved_signal_takes_precedence_over_unreadable_act_and_oldact_pointers() {
  let _lock = signal_lock();
  let output =
    run_child_scenario(CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_ACT_AND_OLDACT);

  assert_eq!(
    output.status.code(),
    Some(CHILD_EXIT_CODE_SUCCESS),
    "reserved signals must be rejected before touching unreadable act/oldact pointers: {}",
    format_child_output(&output),
  );
}

#[test]
fn signal_i045_child_entrypoint() {
  let Ok(scenario) = env::var(CHILD_SCENARIO_ENV) else {
    return;
  };

  match scenario.as_str() {
    CHILD_SCENARIO_INVALID_ACT_POINTER => run_invalid_act_pointer_child_scenario(),
    CHILD_SCENARIO_INVALID_ACT_POINTER_KEEPS_HANDLER => {
      run_invalid_act_pointer_keeps_handler_child_scenario()
    }
    CHILD_SCENARIO_INVALID_ACT_POINTER_KEEPS_OLDACT => {
      run_invalid_act_pointer_keeps_oldact_child_scenario()
    }
    CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_ACT => {
      run_invalid_signal_precedence_over_invalid_act_child_scenario()
    }
    CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_ACT => {
      run_reserved_signal_precedence_over_invalid_act_child_scenario()
    }
    CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_OLDACT => {
      run_reserved_signal_precedence_over_invalid_oldact_child_scenario()
    }
    CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_OLDACT => {
      run_invalid_signal_precedence_over_invalid_oldact_child_scenario()
    }
    CHILD_SCENARIO_INVALID_SIGNAL_PRECEDENCE_OVER_INVALID_ACT_AND_OLDACT => {
      run_invalid_signal_precedence_over_invalid_act_and_oldact_child_scenario()
    }
    CHILD_SCENARIO_RESERVED_SIGNAL_PRECEDENCE_OVER_INVALID_ACT_AND_OLDACT => {
      run_reserved_signal_precedence_over_invalid_act_and_oldact_child_scenario()
    }
    _ => panic!("unknown signal_i045 child scenario: {scenario}"),
  }
}
