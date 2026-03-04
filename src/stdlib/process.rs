//! Process termination C ABI functions.
//!
//! This module provides a minimal implementation of process-exit related C ABI
//! entry points:
//! - `atexit`
//! - `exit`
//! - `_Exit`
//! - `abort`
//!
//! Design notes:
//! - `atexit` handlers are stored process-wide and executed in LIFO order.
//! - `exit` executes currently registered handlers and then terminates.
//! - `_Exit` and `abort` terminate immediately without running handlers.

use crate::abi::types::c_int;
use crate::signal::{SIGABRT, raise};
use crate::syscall::{syscall1, syscall4};
use core::ffi::c_long;
use std::sync::{Mutex, MutexGuard, OnceLock};

static AT_EXIT_HANDLERS: OnceLock<Mutex<Vec<AtExitHandler>>> = OnceLock::new();
const SYS_EXIT_GROUP: c_long = 231;
const SYS_RT_SIGACTION: c_long = 13;
const SYS_RT_SIGPROCMASK: c_long = 14;
const SIG_UNBLOCK: c_long = 1;
const SIG_DFL: usize = 0;
const SIGABRT_MASK: c_long = (1 as c_long) << (SIGABRT as c_long - 1);
const KERNEL_SIGSET_SIZE: c_long = 8;
const SIGABRT_STATUS_OFFSET: c_int = 128;
const ATEXIT_MAX_HANDLERS: usize = 32;

type AtExitHandler = extern "C" fn();

#[repr(C)]
struct KernelSigAction {
  handler: usize,
  flags: c_long,
  restorer: usize,
  mask: c_long,
}

fn handlers() -> &'static Mutex<Vec<AtExitHandler>> {
  AT_EXIT_HANDLERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn lock_handlers() -> MutexGuard<'static, Vec<AtExitHandler>> {
  match handlers().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn run_atexit_handlers() {
  loop {
    let next_handler = {
      let mut guard = lock_handlers();

      guard.pop()
    };
    let Some(handler) = next_handler else {
      break;
    };

    handler();
  }
}

fn terminate_immediately(status: c_int) -> ! {
  loop {
    // SAFETY: `exit_group` is terminal for the current process; repeating is a
    // defensive fallback if the syscall unexpectedly returns.
    let _ = unsafe { syscall1(SYS_EXIT_GROUP, c_long::from(status)) };
  }
}

fn ptr_to_c_long<T>(pointer: *const T) -> c_long {
  c_long::try_from(pointer as usize)
    .unwrap_or_else(|_| unreachable!("pointer does not fit into c_long on this target"))
}

fn reset_sigabrt_disposition_to_default() {
  let action = KernelSigAction {
    handler: SIG_DFL,
    flags: 0,
    restorer: 0,
    mask: 0,
  };

  // SAFETY: `rt_sigaction(SIGABRT, &action, NULL, sizeof(kernel_sigset_t))`
  // uses a valid pointer and the Linux x86_64 kernel sigset size.
  let _ = unsafe {
    syscall4(
      SYS_RT_SIGACTION,
      c_long::from(SIGABRT),
      ptr_to_c_long(&raw const action),
      0,
      KERNEL_SIGSET_SIZE,
    )
  };
}

fn unblock_sigabrt_for_current_thread() {
  let sigset = SIGABRT_MASK;

  // SAFETY: `rt_sigprocmask(SIG_UNBLOCK, &sigset, NULL, sizeof(kernel_sigset_t))`
  // uses valid pointers and x86_64 Linux kernel sigset size.
  let _ = unsafe {
    syscall4(
      SYS_RT_SIGPROCMASK,
      SIG_UNBLOCK,
      ptr_to_c_long(&raw const sigset),
      0,
      KERNEL_SIGSET_SIZE,
    )
  };
}

/// C ABI entry point for `atexit`.
///
/// Registers a process-global termination handler that is executed by `exit`.
/// Handlers execute in reverse registration order (LIFO).
///
/// Returns:
/// - `0` on success
/// - non-zero on failure (`function` is null)
///
/// Contract notes:
/// - This minimal implementation stores handlers in memory for the lifetime of
///   the process.
/// - Handlers are consumed when `exit` runs and are not executed by `_Exit` or
///   `abort`.
/// - This implementation accepts up to 32 registrations. Additional
///   registrations fail with non-zero status.
#[unsafe(no_mangle)]
pub extern "C" fn atexit(function: Option<AtExitHandler>) -> c_int {
  let Some(handler) = function else {
    return 1;
  };
  let mut guard = lock_handlers();

  if guard.len() >= ATEXIT_MAX_HANDLERS {
    return 1;
  }

  guard.push(handler);

  0
}

/// C ABI entry point for `exit`.
///
/// Runs currently registered `atexit` handlers in LIFO order, then terminates
/// the process with `status`.
///
/// Contract notes:
/// - This function does not return.
/// - This implementation executes handlers in LIFO order until no handlers
///   remain, including handlers registered by other handlers.
#[unsafe(no_mangle)]
pub extern "C" fn exit(status: c_int) -> ! {
  run_atexit_handlers();
  terminate_immediately(status)
}

/// C ABI entry point for `_Exit`.
///
/// Terminates the process immediately with `status` without running `atexit`
/// handlers.
///
/// Contract notes:
/// - This function does not return.
#[unsafe(no_mangle)]
pub extern "C" fn _Exit(status: c_int) -> ! {
  terminate_immediately(status)
}

/// C ABI entry point for `abort`.
///
/// Abnormally terminates the process immediately, without running `atexit`
/// handlers.
///
/// Contract notes:
/// - This function does not return.
/// - The implementation first unblocks `SIGABRT` and raises it using the
///   current process disposition.
/// - If the process still survives (for example, `SIGABRT` was ignored or a
///   handler returned), the implementation restores `SIGABRT` to default
///   disposition and raises it again.
/// - If signal termination still does not occur, it falls back to immediate
///   status-based termination.
#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
  unblock_sigabrt_for_current_thread();

  let _ = raise(SIGABRT);

  reset_sigabrt_disposition_to_default();

  let _ = raise(SIGABRT);

  terminate_immediately(SIGABRT_STATUS_OFFSET + SIGABRT)
}

#[cfg(test)]
mod tests {
  use super::{ATEXIT_MAX_HANDLERS, atexit, lock_handlers, run_atexit_handlers};
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::sync::{Mutex, OnceLock};

  static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  static CALL_ORDER: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();
  static DYNAMIC_REGISTRATION_FAILED: AtomicBool = AtomicBool::new(false);

  fn test_lock() -> &'static Mutex<()> {
    TEST_LOCK.get_or_init(|| Mutex::new(()))
  }

  fn call_order_log() -> &'static Mutex<Vec<u8>> {
    CALL_ORDER.get_or_init(|| Mutex::new(Vec::new()))
  }

  fn clear_state() {
    {
      let mut handlers = lock_handlers();

      handlers.clear();
    }

    let mut log = match call_order_log().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    log.clear();
  }

  fn push_call(marker: u8) {
    let mut log = match call_order_log().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    log.push(marker);
  }

  fn snapshot_calls() -> Vec<u8> {
    let log = match call_order_log().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    log.clone()
  }

  extern "C" fn first_handler() {
    push_call(1);
  }

  extern "C" fn second_handler() {
    push_call(2);
  }

  extern "C" fn third_handler() {
    push_call(3);
  }

  extern "C" fn dynamic_handler() {
    push_call(4);
  }

  extern "C" fn registering_handler() {
    push_call(2);

    if atexit(Some(dynamic_handler)) != 0 {
      DYNAMIC_REGISTRATION_FAILED.store(true, Ordering::Relaxed);
    }
  }

  #[test]
  fn atexit_returns_non_zero_for_null_handler() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    clear_state();

    let status = atexit(None);

    assert_ne!(status, 0);
  }

  #[test]
  fn atexit_handlers_run_in_lifo_order() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    clear_state();

    assert_eq!(atexit(Some(first_handler)), 0);
    assert_eq!(atexit(Some(second_handler)), 0);
    assert_eq!(atexit(Some(third_handler)), 0);

    run_atexit_handlers();

    assert_eq!(snapshot_calls(), vec![3, 2, 1]);
  }

  #[test]
  fn atexit_handler_can_register_followup_handler() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    clear_state();
    DYNAMIC_REGISTRATION_FAILED.store(false, Ordering::Relaxed);

    assert_eq!(atexit(Some(first_handler)), 0);
    assert_eq!(atexit(Some(registering_handler)), 0);
    assert_eq!(atexit(Some(third_handler)), 0);

    run_atexit_handlers();

    assert!(!DYNAMIC_REGISTRATION_FAILED.load(Ordering::Relaxed));
    assert_eq!(snapshot_calls(), vec![3, 2, 4, 1]);
  }

  #[test]
  fn atexit_rejects_registration_when_slot_limit_is_reached() {
    let _test_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    clear_state();

    for _index in 0..ATEXIT_MAX_HANDLERS {
      assert_eq!(atexit(Some(first_handler)), 0);
    }

    assert_ne!(atexit(Some(first_handler)), 0);
  }
}
