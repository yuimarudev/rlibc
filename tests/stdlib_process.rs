#![cfg(unix)]

use core::ffi::{c_int, c_long, c_void};
use core::ptr;
use rlibc::abi::types::size_t;
use rlibc::signal::{SIG_IGN, SigAction, sigaction};
use rlibc::syscall::syscall4;
use std::env;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const CHILD_SCENARIO_ENV: &str = "RLIBC_STDLIB_PROCESS_SCENARIO";
const CHILD_RUNNER_TEST: &str = "process_child_entrypoint";
const SCENARIO_ATEXIT_ORDER: &str = "atexit_order";
const SCENARIO_ATEXIT_DYNAMIC_ORDER: &str = "atexit_dynamic_order";
const SCENARIO_ATEXIT_REGISTRATION_LIMIT_SUCCESS: &str = "atexit_registration_limit_success";
const SCENARIO_ATEXIT_REGISTRATION_LIMIT: &str = "atexit_registration_limit";
const SCENARIO_UNKNOWN: &str = "i018_unknown_scenario";
const SCENARIO_UNDERSCORE_EXIT: &str = "underscore_exit";
const SCENARIO_EXIT_STATUS: &str = "exit_status";
const SCENARIO_EXIT_LIFO_STATUS: &str = "exit_lifo_status";
const SCENARIO_ABORT: &str = "abort";
const SCENARIO_ABORT_BLOCKED: &str = "abort_blocked";
const SCENARIO_ABORT_IGNORED: &str = "abort_ignored";
const SCENARIO_ABORT_CAUGHT: &str = "abort_caught";
const ABORT_SIGNAL: i32 = 6;
const SIG_BLOCK: c_long = 0;
const SYS_RT_SIGPROCMASK: c_long = 14;
const SIGABRT_MASK: c_long = 1 << (ABORT_SIGNAL - 1);
const KERNEL_SIGSET_SIZE: c_long = 8;
const REGISTRATION_FAILURE_EXIT_CODE: c_int = 90;
const SIG_BLOCK_FAILURE_EXIT_CODE: c_int = 91;
const SIGACTION_FAILURE_EXIT_CODE: c_int = 92;
const UNDERSCORE_EXIT_CODE: c_int = 55;
const EXIT_STATUS_CODE: c_int = 37;
const EXIT_LIFO_STATUS_CODE: c_int = 73;
const PANIC_EXIT_STATUS_CODE: c_int = 101;
const ATEXIT_REGISTRATION_MAX_SUCCESS: usize = 32;
const ATEXIT_REGISTRATION_ATTEMPTS: usize = ATEXIT_REGISTRATION_MAX_SUCCESS + 1;
const FAILED_OUTPUT_TOKEN: &[u8] = b"FAILED";
const PROCESS_CHILD_ENTRYPOINT_TOKEN: &[u8] = b"process_child_entrypoint";
static ATEXIT_STEP: AtomicU8 = AtomicU8::new(0);
static ATEXIT_ORDER_FAILED: AtomicBool = AtomicBool::new(false);
static ATEXIT_DYNAMIC_STEP: AtomicU8 = AtomicU8::new(0);
static ATEXIT_DYNAMIC_ORDER_FAILED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
  fn atexit(function: extern "C" fn()) -> c_int;
  fn exit(status: c_int) -> !;
  fn _Exit(status: c_int) -> !;
  fn abort() -> !;
  fn write(fd: c_int, buffer: *const c_void, count: size_t) -> isize;
}

fn sz(len: usize) -> size_t {
  size_t::try_from(len)
    .unwrap_or_else(|_| unreachable!("usize does not fit into size_t on this target"))
}

fn write_stdout_bytes(bytes: &[u8]) {
  let pointer = bytes.as_ptr().cast::<c_void>();
  // SAFETY: `bytes` points to initialized memory readable for `bytes.len()` bytes.
  let _result = unsafe { write(1, pointer, sz(bytes.len())) };
}

fn register_atexit(handler: extern "C" fn()) {
  // SAFETY: function pointer has C ABI and static lifetime.
  let result = unsafe { atexit(handler) };

  if result != 0 {
    // SAFETY: child process cannot recover if registration fails.
    unsafe { _Exit(REGISTRATION_FAILURE_EXIT_CODE) };
  }
}

fn format_output(output: &Output) -> String {
  format!(
    "status={:?}, stdout={:?}, stderr={:?}",
    output.status,
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr)
  )
}

fn marker_occurrences(haystack: &[u8], marker: &[u8]) -> usize {
  haystack
    .windows(marker.len())
    .filter(|window| *window == marker)
    .count()
}

fn ptr_to_c_long<T>(pointer: *const T) -> c_long {
  c_long::try_from(pointer as usize)
    .unwrap_or_else(|_| unreachable!("pointer does not fit into c_long on this target"))
}

fn run_child_scenario(scenario: &str) -> Output {
  let current_executable = env::current_exe().expect("failed to resolve current test executable");

  Command::new(current_executable)
    .arg("--exact")
    .arg(CHILD_RUNNER_TEST)
    .arg("--nocapture")
    .env(CHILD_SCENARIO_ENV, scenario)
    .stderr(Stdio::null())
    .output()
    .expect("failed to execute child test process")
}

fn mark_step(expected: u8, next: u8) {
  if ATEXIT_STEP
    .compare_exchange(expected, next, Ordering::Relaxed, Ordering::Relaxed)
    .is_err()
  {
    ATEXIT_ORDER_FAILED.store(true, Ordering::Relaxed);
  }
}

extern "C" fn atexit_first_registered() {
  mark_step(2, 3);
  write_stdout_bytes(b"{1}");

  let failed =
    ATEXIT_ORDER_FAILED.load(Ordering::Relaxed) || ATEXIT_STEP.load(Ordering::Relaxed) != 3;
  let exit_code = i32::from(failed);
  // SAFETY: this callback is the final state reporter for the child process.
  unsafe { _Exit(exit_code) };
}

extern "C" fn atexit_second_registered() {
  mark_step(1, 2);
  write_stdout_bytes(b"{2}");
}

extern "C" fn atexit_third_registered() {
  mark_step(0, 1);
  write_stdout_bytes(b"{3}");
}

extern "C" fn atexit_probe_handler() {
  write_stdout_bytes(b"{X}");
}

extern "C" fn atexit_exit_status_handler() {
  write_stdout_bytes(b"{E}");
}

extern "C" fn atexit_exit_lifo_first_handler() {
  write_stdout_bytes(b"{A}");
}

extern "C" fn atexit_exit_lifo_second_handler() {
  write_stdout_bytes(b"{B}");
}

extern "C" fn atexit_exit_lifo_third_handler() {
  write_stdout_bytes(b"{C}");
}

fn mark_dynamic_step(expected: u8, next: u8) {
  if ATEXIT_DYNAMIC_STEP
    .compare_exchange(expected, next, Ordering::Relaxed, Ordering::Relaxed)
    .is_err()
  {
    ATEXIT_DYNAMIC_ORDER_FAILED.store(true, Ordering::Relaxed);
  }
}

extern "C" fn atexit_dynamic_first_registered() {
  mark_dynamic_step(3, 4);
  write_stdout_bytes(b"{1}");

  let failed = ATEXIT_DYNAMIC_ORDER_FAILED.load(Ordering::Relaxed)
    || ATEXIT_DYNAMIC_STEP.load(Ordering::Relaxed) != 4;
  let exit_code = i32::from(failed);
  // SAFETY: this callback is the final state reporter for the child process.
  unsafe { _Exit(exit_code) };
}

extern "C" fn atexit_dynamic_registering() {
  mark_dynamic_step(1, 2);
  write_stdout_bytes(b"{2}");
  register_atexit(atexit_dynamic_late_registered);
}

extern "C" fn atexit_dynamic_late_registered() {
  mark_dynamic_step(2, 3);
  write_stdout_bytes(b"{4}");
}

extern "C" fn atexit_dynamic_third_registered() {
  mark_dynamic_step(0, 1);
  write_stdout_bytes(b"{3}");
}

fn run_atexit_order_scenario() -> ! {
  ATEXIT_STEP.store(0, Ordering::Relaxed);
  ATEXIT_ORDER_FAILED.store(false, Ordering::Relaxed);

  register_atexit(atexit_first_registered);
  register_atexit(atexit_second_registered);
  register_atexit(atexit_third_registered);
  // SAFETY: intentionally triggers process termination to run atexit handlers.
  unsafe { exit(0) }
}

fn run_atexit_dynamic_order_scenario() -> ! {
  ATEXIT_DYNAMIC_STEP.store(0, Ordering::Relaxed);
  ATEXIT_DYNAMIC_ORDER_FAILED.store(false, Ordering::Relaxed);

  register_atexit(atexit_dynamic_first_registered);
  register_atexit(atexit_dynamic_registering);
  register_atexit(atexit_dynamic_third_registered);
  // SAFETY: intentionally triggers process termination to run atexit handlers.
  unsafe { exit(0) }
}

fn run_atexit_registration_limit_success_scenario() -> ! {
  for _attempt in 0..ATEXIT_REGISTRATION_MAX_SUCCESS {
    register_atexit(atexit_probe_handler);
  }
  // SAFETY: intentionally runs registered handlers and exits with success.
  unsafe { exit(0) }
}

fn run_atexit_registration_limit_scenario() -> ! {
  for _attempt in 0..ATEXIT_REGISTRATION_ATTEMPTS {
    register_atexit(atexit_probe_handler);
  }
  // SAFETY: scenario reached only if all registrations unexpectedly succeeded.
  unsafe { exit(0) }
}

fn run_underscore_exit_scenario() -> ! {
  register_atexit(atexit_probe_handler);
  // SAFETY: intentionally bypasses atexit processing.
  unsafe { _Exit(UNDERSCORE_EXIT_CODE) }
}

fn run_exit_status_scenario() -> ! {
  register_atexit(atexit_exit_status_handler);
  // SAFETY: intentionally triggers process termination to verify `exit` status propagation.
  unsafe { exit(EXIT_STATUS_CODE) }
}

fn run_exit_lifo_status_scenario() -> ! {
  register_atexit(atexit_exit_lifo_first_handler);
  register_atexit(atexit_exit_lifo_second_handler);
  register_atexit(atexit_exit_lifo_third_handler);
  // SAFETY: intentionally triggers process termination to verify LIFO + status propagation.
  unsafe { exit(EXIT_LIFO_STATUS_CODE) }
}

fn run_abort_scenario() -> ! {
  register_atexit(atexit_probe_handler);
  // SAFETY: intentionally raises SIGABRT.
  unsafe { abort() }
}

fn block_sigabrt_for_current_thread() {
  let set = SIGABRT_MASK;

  // SAFETY: issues `rt_sigprocmask(SIG_BLOCK, &set, NULL, sizeof(kernel_sigset_t))`
  // with valid pointers and Linux x86_64 kernel sigset size.
  let raw = unsafe {
    syscall4(
      SYS_RT_SIGPROCMASK,
      SIG_BLOCK,
      ptr_to_c_long(&raw const set),
      0,
      KERNEL_SIGSET_SIZE,
    )
  };

  if raw < 0 {
    // SAFETY: child process cannot continue if signal mask setup fails.
    unsafe { _Exit(SIG_BLOCK_FAILURE_EXIT_CODE) };
  }
}

fn ignore_sigabrt_for_process() {
  let ignore_action = SigAction {
    sa_handler: SIG_IGN,
    ..SigAction::default()
  };
  // SAFETY: valid pointer to `SigAction`; `oldact` is intentionally unused.
  let status = unsafe { sigaction(ABORT_SIGNAL, &raw const ignore_action, ptr::null_mut()) };

  if status != 0 {
    // SAFETY: child process cannot continue if signal disposition setup fails.
    unsafe { _Exit(SIGACTION_FAILURE_EXIT_CODE) };
  }
}

extern "C" fn caught_sigabrt_handler(_: c_int) {
  write_stdout_bytes(b"{H}");
}

fn catch_sigabrt_for_process() {
  let caught_action = SigAction {
    sa_handler: caught_sigabrt_handler as *const () as usize,
    ..SigAction::default()
  };
  // SAFETY: valid pointer to `SigAction`; `oldact` is intentionally unused.
  let status = unsafe { sigaction(ABORT_SIGNAL, &raw const caught_action, ptr::null_mut()) };

  if status != 0 {
    // SAFETY: child process cannot continue if signal disposition setup fails.
    unsafe { _Exit(SIGACTION_FAILURE_EXIT_CODE) };
  }
}

fn run_abort_blocked_scenario() -> ! {
  register_atexit(atexit_probe_handler);
  block_sigabrt_for_current_thread();
  // SAFETY: intentionally raises SIGABRT after blocking it, so abort must unblock.
  unsafe { abort() }
}

fn run_abort_ignored_scenario() -> ! {
  register_atexit(atexit_probe_handler);
  ignore_sigabrt_for_process();
  // SAFETY: intentionally raises SIGABRT after installing `SIG_IGN`.
  unsafe { abort() }
}

fn run_abort_caught_scenario() -> ! {
  register_atexit(atexit_probe_handler);
  catch_sigabrt_for_process();
  // SAFETY: intentionally raises SIGABRT after installing a returning handler.
  unsafe { abort() }
}

#[test]
fn process_child_entrypoint() {
  let Ok(scenario) = env::var(CHILD_SCENARIO_ENV) else {
    return;
  };

  match scenario.as_str() {
    SCENARIO_ATEXIT_ORDER => run_atexit_order_scenario(),
    SCENARIO_ATEXIT_DYNAMIC_ORDER => run_atexit_dynamic_order_scenario(),
    SCENARIO_ATEXIT_REGISTRATION_LIMIT_SUCCESS => run_atexit_registration_limit_success_scenario(),
    SCENARIO_ATEXIT_REGISTRATION_LIMIT => run_atexit_registration_limit_scenario(),
    SCENARIO_UNDERSCORE_EXIT => run_underscore_exit_scenario(),
    SCENARIO_EXIT_STATUS => run_exit_status_scenario(),
    SCENARIO_EXIT_LIFO_STATUS => run_exit_lifo_status_scenario(),
    SCENARIO_ABORT => run_abort_scenario(),
    SCENARIO_ABORT_BLOCKED => run_abort_blocked_scenario(),
    SCENARIO_ABORT_IGNORED => run_abort_ignored_scenario(),
    SCENARIO_ABORT_CAUGHT => run_abort_caught_scenario(),
    _ => panic!("unknown child scenario: {scenario}"),
  }
}

#[test]
fn atexit_registration_limit_allows_minimum_slots_and_runs_handlers_on_exit() {
  let output = run_child_scenario(SCENARIO_ATEXIT_REGISTRATION_LIMIT_SUCCESS);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let probe_count = marker_occurrences(&output.stdout, b"{X}");
  let other_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}")
    + marker_occurrences(&output.stdout, b"{E}")
    + marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}")
    + marker_occurrences(&output.stdout, b"{H}");

  assert_eq!(
    output.status.code(),
    Some(0),
    "atexit registration-limit success scenario should complete with status 0: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(PANIC_EXIT_STATUS_CODE),
    "atexit registration-limit success scenario should not report panic exit status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(REGISTRATION_FAILURE_EXIT_CODE),
    "atexit registration-limit success scenario should not report registration failure code: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "atexit registration-limit success scenario should not terminate via signal: {output_context}"
  );
  assert!(
    output.status.success(),
    "atexit registration-limit success scenario should report success: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "atexit registration-limit success scenario should start child test harness: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "atexit registration-limit success scenario should early-terminate without child test completion marker: {output_context}"
  );
  assert_eq!(
    probe_count, ATEXIT_REGISTRATION_MAX_SUCCESS,
    "atexit registration-limit success scenario should run all accepted probe handlers: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "atexit registration-limit success scenario should not report FAILED markers: {output_context}"
  );
  assert_eq!(
    other_marker_count, 0,
    "atexit registration-limit success scenario should not emit unrelated handler markers: {output_context}"
  );
}

#[test]
fn atexit_registration_limit_failure_exits_without_running_handlers() {
  let output = run_child_scenario(SCENARIO_ATEXIT_REGISTRATION_LIMIT);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let probe_count = marker_occurrences(&output.stdout, b"{X}");
  let other_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}")
    + marker_occurrences(&output.stdout, b"{E}")
    + marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}")
    + marker_occurrences(&output.stdout, b"{H}");

  assert_eq!(
    output.status.code(),
    Some(REGISTRATION_FAILURE_EXIT_CODE),
    "atexit registration-limit scenario should fail with registration failure code: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(0),
    "atexit registration-limit scenario should not report success exit status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(PANIC_EXIT_STATUS_CODE),
    "atexit registration-limit scenario should not use panic exit status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(EXIT_STATUS_CODE),
    "atexit registration-limit scenario should not report exit-status scenario code: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(EXIT_LIFO_STATUS_CODE),
    "atexit registration-limit scenario should not report exit-lifo scenario code: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(SIG_BLOCK_FAILURE_EXIT_CODE),
    "atexit registration-limit scenario should not report SIG_BLOCK failure status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(SIGACTION_FAILURE_EXIT_CODE),
    "atexit registration-limit scenario should not report sigaction setup failure status: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "atexit registration-limit scenario should not terminate via signal: {output_context}"
  );
  assert!(
    !output.status.success(),
    "atexit registration-limit scenario should not report success: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "atexit registration-limit scenario should start child test harness before failing registration: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "atexit registration-limit scenario should early-exit without child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "atexit registration-limit scenario should early-exit before child harness FAILED summary in stdout: {output_context}"
  );
  assert_eq!(
    probe_count, 0,
    "atexit registration-limit scenario should not run probe handlers after registration failure: {output_context}"
  );
  assert_eq!(
    other_marker_count, 0,
    "atexit registration-limit scenario should not emit unrelated handler markers: {output_context}"
  );
}

#[test]
fn unknown_child_scenario_fails_without_running_known_handlers() {
  let output = run_child_scenario(SCENARIO_UNKNOWN);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let handler_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}")
    + marker_occurrences(&output.stdout, b"{E}")
    + marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}")
    + marker_occurrences(&output.stdout, b"{X}")
    + marker_occurrences(&output.stdout, b"{H}");

  assert!(
    !output.status.success(),
    "unknown child scenario should fail in child entrypoint: {output_context}"
  );
  assert_eq!(
    output.status.code(),
    Some(PANIC_EXIT_STATUS_CODE),
    "unknown child scenario should fail with panic exit status {PANIC_EXIT_STATUS_CODE}: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(UNDERSCORE_EXIT_CODE),
    "unknown child scenario should not report _Exit scenario status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(EXIT_STATUS_CODE),
    "unknown child scenario should not report exit-status scenario code: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(EXIT_LIFO_STATUS_CODE),
    "unknown child scenario should not report exit-lifo scenario code: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(REGISTRATION_FAILURE_EXIT_CODE),
    "unknown child scenario should not report atexit registration failure status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(SIG_BLOCK_FAILURE_EXIT_CODE),
    "unknown child scenario should not report SIG_BLOCK failure status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(SIGACTION_FAILURE_EXIT_CODE),
    "unknown child scenario should not report sigaction setup failure status: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "unknown child scenario should not terminate due to signal: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "unknown child scenario should start child test harness before panic is reported: {output_context}"
  );
  assert!(
    child_entrypoint_count >= 1,
    "unknown child scenario should fail in process_child_entrypoint path: {output_context}"
  );
  assert!(
    failed_marker_count >= 1,
    "unknown child scenario should be reported as FAILED by child harness: {output_context}"
  );
  assert_eq!(
    handler_marker_count, 0,
    "unknown child scenario should not execute known handlers: {output_context}"
  );
}

#[test]
fn atexit_runs_handlers_in_reverse_registration_order() {
  let output = run_child_scenario(SCENARIO_ATEXIT_ORDER);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let first_count = marker_occurrences(&output.stdout, b"{1}");
  let second_count = marker_occurrences(&output.stdout, b"{2}");
  let third_count = marker_occurrences(&output.stdout, b"{3}");
  let dynamic_only_count = marker_occurrences(&output.stdout, b"{4}");
  let exit_marker_count = marker_occurrences(&output.stdout, b"{E}");
  let lifo_marker_count = marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}");
  let probe_marker_count = marker_occurrences(&output.stdout, b"{X}");
  let caught_handler_count = marker_occurrences(&output.stdout, b"{H}");

  assert_eq!(
    output.status.code(),
    Some(0),
    "atexit order scenario failed: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "atexit order scenario should not terminate due to signal: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "atexit order scenario should start child test harness before status-preserving termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "atexit order scenario should terminate before child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "atexit order scenario should terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert!(
    output.stdout.ends_with(b"{3}{2}{1}"),
    "unexpected callback order output: {output_context}"
  );
  assert_eq!(
    first_count, 1,
    "atexit order scenario should run first callback exactly once: {output_context}"
  );
  assert_eq!(
    second_count, 1,
    "atexit order scenario should run second callback exactly once: {output_context}"
  );
  assert_eq!(
    third_count, 1,
    "atexit order scenario should run third callback exactly once: {output_context}"
  );
  assert_eq!(
    dynamic_only_count, 0,
    "atexit order scenario should not run dynamic-only callback: {output_context}"
  );
  assert_eq!(
    exit_marker_count, 0,
    "atexit order scenario should not run exit-status callback: {output_context}"
  );
  assert_eq!(
    lifo_marker_count, 0,
    "atexit order scenario should not run LIFO exit callbacks: {output_context}"
  );
  assert_eq!(
    probe_marker_count, 0,
    "atexit order scenario should not run abort/_Exit probe callback: {output_context}"
  );
  assert_eq!(
    caught_handler_count, 0,
    "atexit order scenario should not run abort caught-handler callback: {output_context}"
  );
}

#[test]
fn atexit_runs_handlers_registered_during_exit() {
  let output = run_child_scenario(SCENARIO_ATEXIT_DYNAMIC_ORDER);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let first_count = marker_occurrences(&output.stdout, b"{1}");
  let second_count = marker_occurrences(&output.stdout, b"{2}");
  let third_count = marker_occurrences(&output.stdout, b"{3}");
  let dynamic_only_count = marker_occurrences(&output.stdout, b"{4}");
  let exit_marker_count = marker_occurrences(&output.stdout, b"{E}");
  let lifo_marker_count = marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}");
  let probe_marker_count = marker_occurrences(&output.stdout, b"{X}");
  let caught_handler_count = marker_occurrences(&output.stdout, b"{H}");

  assert_eq!(
    output.status.code(),
    Some(0),
    "atexit dynamic-order scenario failed: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "atexit dynamic-order scenario should not terminate due to signal: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "atexit dynamic-order scenario should start child test harness before status-preserving termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "atexit dynamic-order scenario should terminate before child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "atexit dynamic-order scenario should terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert!(
    output.stdout.ends_with(b"{3}{2}{4}{1}"),
    "unexpected dynamic callback order output: {output_context}"
  );
  assert_eq!(
    first_count, 1,
    "atexit dynamic-order scenario should run first callback exactly once: {output_context}"
  );
  assert_eq!(
    second_count, 1,
    "atexit dynamic-order scenario should run second callback exactly once: {output_context}"
  );
  assert_eq!(
    third_count, 1,
    "atexit dynamic-order scenario should run third callback exactly once: {output_context}"
  );
  assert_eq!(
    dynamic_only_count, 1,
    "atexit dynamic-order scenario should run dynamic callback exactly once: {output_context}"
  );
  assert_eq!(
    exit_marker_count, 0,
    "atexit dynamic-order scenario should not run exit-status callback: {output_context}"
  );
  assert_eq!(
    lifo_marker_count, 0,
    "atexit dynamic-order scenario should not run LIFO exit callbacks: {output_context}"
  );
  assert_eq!(
    probe_marker_count, 0,
    "atexit dynamic-order scenario should not run abort/_Exit probe callback: {output_context}"
  );
  assert_eq!(
    caught_handler_count, 0,
    "atexit dynamic-order scenario should not run abort caught-handler callback: {output_context}"
  );
}

#[test]
fn underscore_exit_skips_atexit_handlers() {
  let output = run_child_scenario(SCENARIO_UNDERSCORE_EXIT);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let probe_count = marker_occurrences(&output.stdout, b"{X}");
  let exit_status_marker_count = marker_occurrences(&output.stdout, b"{E}");
  let lifo_marker_count = marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}");
  let ordered_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}");
  let caught_handler_count = marker_occurrences(&output.stdout, b"{H}");

  assert_eq!(
    output.status.code(),
    Some(UNDERSCORE_EXIT_CODE),
    "_Exit scenario failed: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(PANIC_EXIT_STATUS_CODE),
    "_Exit scenario should not use panic exit status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(EXIT_STATUS_CODE),
    "_Exit scenario should not report exit-status scenario code: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(EXIT_LIFO_STATUS_CODE),
    "_Exit scenario should not report exit-lifo scenario code: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(REGISTRATION_FAILURE_EXIT_CODE),
    "_Exit scenario should not report atexit registration failure status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(SIG_BLOCK_FAILURE_EXIT_CODE),
    "_Exit scenario should not report SIG_BLOCK failure status: {output_context}"
  );
  assert_ne!(
    output.status.code(),
    Some(SIGACTION_FAILURE_EXIT_CODE),
    "_Exit scenario should not report sigaction setup failure status: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "_Exit scenario should not terminate due to signal: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "_Exit scenario should start child test harness before early termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "_Exit scenario should early-terminate without child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "_Exit scenario should early-terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert!(
    !output.status.success(),
    "_Exit scenario should not report success: {output_context}"
  );
  assert_eq!(
    probe_count, 0,
    "atexit handler unexpectedly ran during _Exit: {output_context}"
  );
  assert_eq!(
    exit_status_marker_count, 0,
    "_Exit scenario should not run exit-status handler: {output_context}"
  );
  assert_eq!(
    lifo_marker_count, 0,
    "_Exit scenario should not run LIFO-only handlers: {output_context}"
  );
  assert_eq!(
    ordered_marker_count, 0,
    "_Exit scenario should not run order/dynamic handlers: {output_context}"
  );
  assert_eq!(
    caught_handler_count, 0,
    "_Exit scenario should not run abort caught-handler marker: {output_context}"
  );
}

#[test]
fn exit_runs_atexit_handlers_and_preserves_status_code() {
  let output = run_child_scenario(SCENARIO_EXIT_STATUS);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let marker_count = marker_occurrences(&output.stdout, b"{E}");
  let lifo_marker_count = marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}");
  let ordered_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}");
  let probe_count = marker_occurrences(&output.stdout, b"{X}");
  let caught_handler_count = marker_occurrences(&output.stdout, b"{H}");

  assert_eq!(
    output.status.code(),
    Some(EXIT_STATUS_CODE),
    "exit scenario returned unexpected status: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "exit scenario should not terminate due to signal: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "exit scenario should start child test harness before status-preserving termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "exit scenario should terminate before child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "exit scenario should terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert_eq!(
    marker_count, 1,
    "exit scenario should run registered atexit handler exactly once: {output_context}"
  );
  assert!(
    output.stdout.ends_with(b"{E}"),
    "exit scenario should emit its atexit marker last: {output_context}"
  );
  assert_eq!(
    lifo_marker_count, 0,
    "exit scenario should not run LIFO-only handlers: {output_context}"
  );
  assert_eq!(
    ordered_marker_count, 0,
    "exit scenario should not run order/dynamic handlers: {output_context}"
  );
  assert_eq!(
    probe_count, 0,
    "exit scenario should not run abort/_Exit probe handler: {output_context}"
  );
  assert_eq!(
    caught_handler_count, 0,
    "exit scenario should not run abort caught-handler marker: {output_context}"
  );
}

#[test]
fn exit_runs_all_handlers_in_lifo_order_and_preserves_status_code() {
  let output = run_child_scenario(SCENARIO_EXIT_LIFO_STATUS);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let first_count = marker_occurrences(&output.stdout, b"{A}");
  let second_count = marker_occurrences(&output.stdout, b"{B}");
  let third_count = marker_occurrences(&output.stdout, b"{C}");
  let exit_status_handler_count = marker_occurrences(&output.stdout, b"{E}");
  let ordered_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}");
  let probe_count = marker_occurrences(&output.stdout, b"{X}");
  let caught_handler_count = marker_occurrences(&output.stdout, b"{H}");

  assert_eq!(
    output.status.code(),
    Some(EXIT_LIFO_STATUS_CODE),
    "exit LIFO scenario returned unexpected status: {output_context}"
  );
  assert_eq!(
    output.status.signal(),
    None,
    "exit LIFO scenario should not terminate due to signal: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "exit LIFO scenario should start child test harness before status-preserving termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "exit LIFO scenario should terminate before child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "exit LIFO scenario should terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert!(
    output.stdout.ends_with(b"{C}{B}{A}"),
    "exit LIFO scenario ran handlers in unexpected order: {output_context}"
  );
  assert_eq!(
    first_count, 1,
    "exit LIFO scenario should run first-registered handler exactly once: {output_context}"
  );
  assert_eq!(
    second_count, 1,
    "exit LIFO scenario should run second-registered handler exactly once: {output_context}"
  );
  assert_eq!(
    third_count, 1,
    "exit LIFO scenario should run third-registered handler exactly once: {output_context}"
  );
  assert_eq!(
    exit_status_handler_count, 0,
    "exit LIFO scenario should not run single-handler exit-status marker: {output_context}"
  );
  assert_eq!(
    ordered_marker_count, 0,
    "exit LIFO scenario should not run order/dynamic handlers: {output_context}"
  );
  assert_eq!(
    probe_count, 0,
    "exit LIFO scenario should not run abort/_Exit probe handler: {output_context}"
  );
  assert_eq!(
    caught_handler_count, 0,
    "exit LIFO scenario should not run abort caught-handler marker: {output_context}"
  );
}

#[test]
fn abort_skips_atexit_handlers() {
  let output = run_child_scenario(SCENARIO_ABORT);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let handler_markers = marker_occurrences(&output.stdout, b"{H}");
  let ordered_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}");
  let exit_status_marker_count = marker_occurrences(&output.stdout, b"{E}");
  let lifo_marker_count = marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}");

  assert_eq!(
    output.status.signal(),
    Some(ABORT_SIGNAL),
    "abort scenario did not terminate with SIGABRT: {output_context}"
  );
  assert_eq!(
    output.status.code(),
    None,
    "abort scenario should terminate via signal, not exit status: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "abort scenario should start child test harness before signal termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "abort scenario should terminate before child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "abort scenario should terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert!(
    !output.stdout.windows(3).any(|window| window == b"{X}"),
    "atexit handler unexpectedly ran during abort: {output_context}"
  );
  assert_eq!(
    handler_markers, 0,
    "abort scenario should not run caught-signal handler marker: {output_context}"
  );
  assert_eq!(
    ordered_marker_count, 0,
    "abort scenario should not run atexit order/dynamic markers: {output_context}"
  );
  assert_eq!(
    exit_status_marker_count, 0,
    "abort scenario should not run exit-status marker: {output_context}"
  );
  assert_eq!(
    lifo_marker_count, 0,
    "abort scenario should not run LIFO exit markers: {output_context}"
  );
}

#[test]
fn abort_unblocks_sigabrt_and_skips_atexit_handlers() {
  let output = run_child_scenario(SCENARIO_ABORT_BLOCKED);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let handler_markers = marker_occurrences(&output.stdout, b"{H}");
  let ordered_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}");
  let exit_status_marker_count = marker_occurrences(&output.stdout, b"{E}");
  let lifo_marker_count = marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}");

  assert_eq!(
    output.status.signal(),
    Some(ABORT_SIGNAL),
    "abort blocked-signal scenario did not terminate with SIGABRT: {output_context}"
  );
  assert_eq!(
    output.status.code(),
    None,
    "abort blocked-signal scenario should terminate via signal, not exit status: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "abort blocked-signal scenario should start child test harness before signal termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "abort blocked-signal scenario should terminate before child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "abort blocked-signal scenario should terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert!(
    !output.stdout.windows(3).any(|window| window == b"{X}"),
    "atexit handler unexpectedly ran during abort blocked-signal scenario: {output_context}"
  );
  assert_eq!(
    handler_markers, 0,
    "abort blocked-signal scenario should not run caught-signal handler marker: {output_context}"
  );
  assert_eq!(
    ordered_marker_count, 0,
    "abort blocked-signal scenario should not run atexit order/dynamic markers: {output_context}"
  );
  assert_eq!(
    exit_status_marker_count, 0,
    "abort blocked-signal scenario should not run exit-status marker: {output_context}"
  );
  assert_eq!(
    lifo_marker_count, 0,
    "abort blocked-signal scenario should not run LIFO exit markers: {output_context}"
  );
}

#[test]
fn abort_ignored_sigabrt_still_terminates_with_signal_and_skips_atexit_handlers() {
  let output = run_child_scenario(SCENARIO_ABORT_IGNORED);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let handler_markers = marker_occurrences(&output.stdout, b"{H}");
  let ordered_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}");
  let exit_status_marker_count = marker_occurrences(&output.stdout, b"{E}");
  let lifo_marker_count = marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}");

  assert_eq!(
    output.status.signal(),
    Some(ABORT_SIGNAL),
    "abort ignored-signal scenario did not terminate with SIGABRT: {output_context}"
  );
  assert_eq!(
    output.status.code(),
    None,
    "abort ignored-signal scenario should terminate via signal, not exit status: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "abort ignored-signal scenario should start child test harness before signal termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "abort ignored-signal scenario should terminate before child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "abort ignored-signal scenario should terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert!(
    !output.stdout.windows(3).any(|window| window == b"{X}"),
    "atexit handler unexpectedly ran during abort ignored-signal scenario: {output_context}"
  );
  assert_eq!(
    handler_markers, 0,
    "abort ignored-signal scenario should not run caught-signal handler marker: {output_context}"
  );
  assert_eq!(
    ordered_marker_count, 0,
    "abort ignored-signal scenario should not run atexit order/dynamic markers: {output_context}"
  );
  assert_eq!(
    exit_status_marker_count, 0,
    "abort ignored-signal scenario should not run exit-status marker: {output_context}"
  );
  assert_eq!(
    lifo_marker_count, 0,
    "abort ignored-signal scenario should not run LIFO exit markers: {output_context}"
  );
}

#[test]
fn abort_caught_sigabrt_runs_handler_then_terminates_with_signal() {
  let output = run_child_scenario(SCENARIO_ABORT_CAUGHT);
  let output_context = format_output(&output);
  let runner_banner_count = marker_occurrences(&output.stdout, b"running 1 test");
  let child_entrypoint_count = marker_occurrences(&output.stdout, PROCESS_CHILD_ENTRYPOINT_TOKEN);
  let failed_marker_count = marker_occurrences(&output.stdout, FAILED_OUTPUT_TOKEN);
  let handler_markers = marker_occurrences(&output.stdout, b"{H}");
  let ordered_marker_count = marker_occurrences(&output.stdout, b"{1}")
    + marker_occurrences(&output.stdout, b"{2}")
    + marker_occurrences(&output.stdout, b"{3}")
    + marker_occurrences(&output.stdout, b"{4}");
  let exit_status_marker_count = marker_occurrences(&output.stdout, b"{E}");
  let lifo_marker_count = marker_occurrences(&output.stdout, b"{A}")
    + marker_occurrences(&output.stdout, b"{B}")
    + marker_occurrences(&output.stdout, b"{C}");

  assert_eq!(
    output.status.signal(),
    Some(ABORT_SIGNAL),
    "abort caught-signal scenario did not terminate with SIGABRT: {output_context}"
  );
  assert_eq!(
    output.status.code(),
    None,
    "abort caught-signal scenario should terminate via signal, not exit status: {output_context}"
  );
  assert!(
    runner_banner_count >= 1,
    "abort caught-signal scenario should start child test harness before signal termination: {output_context}"
  );
  assert_eq!(
    child_entrypoint_count, 0,
    "abort caught-signal scenario should terminate before child test completion marker: {output_context}"
  );
  assert_eq!(
    failed_marker_count, 0,
    "abort caught-signal scenario should terminate before child harness FAILED summary in stdout: {output_context}"
  );
  assert!(
    handler_markers >= 1,
    "SIGABRT handler did not run before abort termination: {output_context}"
  );
  assert_eq!(
    handler_markers, 1,
    "SIGABRT handler should run exactly once before default abort termination: {output_context}"
  );
  assert!(
    output.stdout.ends_with(b"{H}"),
    "abort caught-signal scenario should end with caught-handler marker: {output_context}"
  );
  assert!(
    !output.stdout.windows(3).any(|window| window == b"{X}"),
    "atexit handler unexpectedly ran during abort caught-signal scenario: {output_context}"
  );
  assert_eq!(
    ordered_marker_count, 0,
    "abort caught-signal scenario should not run atexit order/dynamic markers: {output_context}"
  );
  assert_eq!(
    exit_status_marker_count, 0,
    "abort caught-signal scenario should not run exit-status marker: {output_context}"
  );
  assert_eq!(
    lifo_marker_count, 0,
    "abort caught-signal scenario should not run LIFO exit markers: {output_context}"
  );
}
