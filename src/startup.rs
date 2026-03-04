//! Startup-related constructor/destructor array helpers.
//!
//! ELF startup code conventionally exposes constructor/destructor ranges through
//! linker-provided `init_array` / `fini_array` boundaries. This module provides
//! minimal routines that iterate those ranges using libc-style ordering rules.

use crate::abi::types::{c_char, c_int};
use crate::stdlib::{environ, exit};
use core::mem;
use core::ptr::addr_of;

const MISSING_MAIN_STATUS: c_int = 127;

/// Function pointer type used by `init_array` and `fini_array` entries.
pub type InitFiniFn = unsafe extern "C" fn();

/// Function pointer type for C `main`.
///
/// The startup path passes:
/// - `argc`: argument count
/// - `argv`: argument vector
/// - `envp`: environment vector
///
/// and expects the process exit status as return value.
pub type StartMainFn =
  unsafe extern "C" fn(argc: c_int, argv: *mut *mut c_char, envp: *mut *mut c_char) -> c_int;

const _: [(); mem::size_of::<InitFiniFn>()] = [(); mem::size_of::<usize>()];
const _: [(); mem::align_of::<InitFiniFn>()] = [(); mem::align_of::<usize>()];
const _: [(); mem::size_of::<Option<InitFiniFn>>()] = [(); mem::size_of::<InitFiniFn>()];
const _: [(); mem::align_of::<Option<InitFiniFn>>()] = [(); mem::align_of::<InitFiniFn>()];

unsafe extern "C" {
  static __init_array_start: InitFiniFn;
  static __init_array_end: InitFiniFn;
  static __fini_array_start: InitFiniFn;
  static __fini_array_end: InitFiniFn;
}

#[derive(Clone, Copy)]
struct StartupArgs {
  argc: c_int,
  argv: *mut *mut c_char,
  envp: *mut *mut c_char,
}

#[derive(Clone, Copy)]
struct InitFiniRange {
  start: *const InitFiniFn,
  end: *const InitFiniFn,
}

impl StartupArgs {
  const fn new(count: c_int, arg_ptr: *mut *mut c_char, env_ptr: *mut *mut c_char) -> Self {
    Self {
      argc: count,
      argv: arg_ptr,
      envp: env_ptr,
    }
  }
}

impl InitFiniRange {
  const fn new(start: *const InitFiniFn, end: *const InitFiniFn) -> Self {
    Self { start, end }
  }
}

fn entry_count(start: *const InitFiniFn, end: *const InitFiniFn) -> usize {
  if start.is_null() || end.is_null() {
    return 0;
  }

  let start_addr = start as usize;
  let end_addr = end as usize;

  if end_addr <= start_addr {
    return 0;
  }

  let entry_align = mem::align_of::<InitFiniFn>();

  if !start_addr.is_multiple_of(entry_align) || !end_addr.is_multiple_of(entry_align) {
    return 0;
  }

  let entry_size = mem::size_of::<InitFiniFn>();
  let distance = end_addr - start_addr;

  if distance > isize::MAX as usize {
    return 0;
  }

  if !distance.is_multiple_of(entry_size) {
    return 0;
  }

  distance / entry_size
}

unsafe fn read_array_entry(start: *const InitFiniFn, index: usize) -> Option<InitFiniFn> {
  // SAFETY: `index < count` is guaranteed by callers, and `start` originates
  // from an aligned valid range shape accepted by `entry_count`.
  let raw = unsafe { core::ptr::read_unaligned(start.add(index).cast::<usize>()) };

  if raw == 0 {
    return None;
  }

  if !raw.is_multiple_of(core::mem::align_of::<InitFiniFn>()) {
    return None;
  }

  // SAFETY: non-null/aligned raw address is interpreted as constructor slot value.
  Some(unsafe { core::mem::transmute::<usize, InitFiniFn>(raw) })
}

/// Runs an `init_array` range from `start` to `end` in forward order.
///
/// The iteration contract matches ELF process startup behavior where
/// constructors run in ascending address order.
///
/// # Safety
/// - `start..end` must describe a valid contiguous range of `InitFiniFn`
///   entries, with `end` equal to one-past-the-last element.
/// - Each function pointer in the range must be callable at this point in the
///   startup sequence.
/// - Passing pointers from different allocations or invalid pointers causes
///   undefined behavior.
///
/// Defensive behavior:
/// - if `start` or `end` is null, this function performs no calls.
/// - if `end` is before `start`, or the byte distance is not a whole number of
///   [`InitFiniFn`] entries, this function performs no calls.
/// - if either pointer address is not aligned for [`InitFiniFn`] entries, this
///   function performs no calls.
/// - if an entry slot contains a null function pointer value, that slot is
///   skipped and iteration continues.
/// - if an entry slot contains a non-null value that is not aligned for
///   [`InitFiniFn`], that slot is skipped and iteration continues.
pub unsafe fn run_init_array_range(start: *const InitFiniFn, end: *const InitFiniFn) {
  let count = entry_count(start, end);

  for index in 0..count {
    // SAFETY: `index < count` and `count` is derived from the validated
    // `start..end` range shape.
    let Some(constructor) = (unsafe { read_array_entry(start, index) }) else {
      continue;
    };
    // SAFETY: each entry points to a callable constructor by contract.
    unsafe { constructor() };
  }
}

/// Runs a `fini_array` range from `end` back to `start` in reverse order.
///
/// The iteration contract matches ELF process teardown behavior where
/// destructors run in descending address order.
///
/// # Safety
/// - `start..end` must describe a valid contiguous range of `InitFiniFn`
///   entries, with `end` equal to one-past-the-last element.
/// - Each function pointer in the range must be callable at this point in the
///   teardown sequence.
/// - Passing pointers from different allocations or invalid pointers causes
///   undefined behavior.
///
/// Defensive behavior:
/// - if `start` or `end` is null, this function performs no calls.
/// - if `end` is before `start`, or the byte distance is not a whole number of
///   [`InitFiniFn`] entries, this function performs no calls.
/// - if either pointer address is not aligned for [`InitFiniFn`] entries, this
///   function performs no calls.
/// - if an entry slot contains a null function pointer value, that slot is
///   skipped and iteration continues.
/// - if an entry slot contains a non-null value that is not aligned for
///   [`InitFiniFn`], that slot is skipped and iteration continues.
pub unsafe fn run_fini_array_range(start: *const InitFiniFn, end: *const InitFiniFn) {
  let count = entry_count(start, end);

  for index in (0..count).rev() {
    // SAFETY: `index < count` and `count` is derived from the validated
    // `start..end` range shape.
    let Some(destructor) = (unsafe { read_array_entry(start, index) }) else {
      continue;
    };
    // SAFETY: each entry points to a callable destructor by contract.
    unsafe { destructor() };
  }
}

/// Runs the libc startup flow for a resolved `main` entrypoint.
///
/// The sequence is:
/// 1. bind `environ` to `envp`
/// 2. run constructors in `init_array` order
/// 3. call `main`
/// 4. run destructors in `fini_array` reverse order
///
/// Returns the `main` return value as process exit status.
///
/// # Safety
/// - `main` must be a valid C ABI entrypoint.
/// - `args` (`argc`/`argv`/`envp`) must satisfy the C runtime pointer contract.
/// - `init_range` and `fini_range` must each describe valid contiguous
///   [`InitFiniFn`] ranges (or empty ranges).
///
/// Defensive behavior:
/// - null endpoints in `init`/`fini` ranges are treated as empty ranges.
unsafe fn run_startup_main(
  main: StartMainFn,
  args: StartupArgs,
  init_range: InitFiniRange,
  fini_range: InitFiniRange,
) -> c_int {
  // SAFETY: startup owns initialization of the process-global `environ` pointer.
  unsafe {
    environ = args.envp;
  }

  // SAFETY: caller guarantees init-array pointer contract.
  unsafe {
    run_init_array_range(init_range.start, init_range.end);
  }

  // SAFETY: caller guarantees `main` and argument pointers follow C ABI contract.
  let exit_status = unsafe { main(args.argc, args.argv, args.envp) };

  // SAFETY: caller guarantees fini-array pointer contract.
  unsafe {
    run_fini_array_range(fini_range.start, fini_range.end);
  }

  exit_status
}

fn terminate_with_exit(status: c_int) -> ! {
  exit(status)
}

unsafe fn run_libc_start_main_with(
  main: Option<StartMainFn>,
  args: StartupArgs,
  init_range: InitFiniRange,
  fini_range: InitFiniRange,
  terminate: fn(c_int) -> !,
) -> ! {
  // SAFETY: startup owns initialization of the process-global `environ` pointer.
  unsafe {
    environ = args.envp;
  }

  let Some(main) = main else {
    terminate(MISSING_MAIN_STATUS)
  };

  // SAFETY: caller guarantees startup pointer contracts.
  let status = unsafe { run_startup_main(main, args, init_range, fini_range) };

  terminate(status)
}

/// C ABI entry point for libc process startup.
///
/// This function is called from crt `_start` stubs. It binds `environ`, runs
/// constructor/destructor arrays around `main`, and terminates the process via
/// [`exit`] with `main`'s return value.
///
/// Contract notes:
/// - This function does not return.
/// - Constructor/destructor ranges are read from linker-provided
///   `__init_array_*` / `__fini_array_*` boundaries.
/// - Startup binds the process-global `environ` pointer to `envp` before
///   checking `main`.
/// - If `main` is null, startup terminates immediately with status `127`
///   without invoking constructors or destructors.
///
/// # Safety
/// - `main` must be `Some` with a valid C ABI `main` entrypoint.
/// - `argv`/`envp` must satisfy C runtime pointer requirements for startup.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __libc_start_main(
  main: Option<StartMainFn>,
  arg_count: c_int,
  arg_vector: *mut *mut c_char,
  env_vector: *mut *mut c_char,
) -> ! {
  // SAFETY: `_start` and linker-provided array boundaries satisfy startup flow
  // pointer contracts.
  unsafe {
    run_libc_start_main_with(
      main,
      StartupArgs::new(arg_count, arg_vector, env_vector),
      InitFiniRange::new(addr_of!(__init_array_start), addr_of!(__init_array_end)),
      InitFiniRange::new(addr_of!(__fini_array_start), addr_of!(__fini_array_end)),
      terminate_with_exit,
    )
  }
}

#[cfg(test)]
mod tests {
  use super::{
    InitFiniFn, InitFiniRange, MISSING_MAIN_STATUS, StartMainFn, StartupArgs, entry_count,
    run_libc_start_main_with, run_startup_main,
  };
  use crate::abi::types::{c_char, c_int};
  use crate::stdlib::{environ, lock_environ_for_test};
  use core::{mem, ptr};
  use std::panic;
  use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
  use std::sync::{Mutex, MutexGuard, OnceLock};

  static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  static CALL_LOG: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();
  static OBSERVED_ARGC: AtomicI32 = AtomicI32::new(-1);
  static OBSERVED_ARGV: AtomicUsize = AtomicUsize::new(0);
  static OBSERVED_ENVP: AtomicUsize = AtomicUsize::new(0);
  static OBSERVED_ENVIRON_DURING_INIT: AtomicUsize = AtomicUsize::new(0);
  static TRAPPED_EXIT_STATUS: AtomicI32 = AtomicI32::new(-1);

  struct EnvironRestore {
    previous: *mut *mut c_char,
  }

  struct TestGuards {
    _startup: MutexGuard<'static, ()>,
    _environ: MutexGuard<'static, ()>,
  }

  impl EnvironRestore {
    fn capture() -> Self {
      // SAFETY: Reading the process-global pointer does not dereference it.
      let previous = unsafe { environ };

      Self { previous }
    }
  }

  impl Drop for EnvironRestore {
    fn drop(&mut self) {
      // SAFETY: Restoring the saved pointer keeps test side effects local.
      unsafe {
        environ = self.previous;
      }
    }
  }

  fn test_lock() -> &'static Mutex<()> {
    TEST_LOCK.get_or_init(|| Mutex::new(()))
  }

  fn lock_test() -> TestGuards {
    let startup_guard = match test_lock().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let environ_guard = lock_environ_for_test();

    TestGuards {
      _startup: startup_guard,
      _environ: environ_guard,
    }
  }

  fn log() -> &'static Mutex<Vec<u8>> {
    CALL_LOG.get_or_init(|| Mutex::new(Vec::new()))
  }

  fn reset_state() {
    match log().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    }
    .clear();
    OBSERVED_ARGC.store(-1, Ordering::Relaxed);
    OBSERVED_ARGV.store(0, Ordering::Relaxed);
    OBSERVED_ENVP.store(0, Ordering::Relaxed);
    OBSERVED_ENVIRON_DURING_INIT.store(0, Ordering::Relaxed);
    TRAPPED_EXIT_STATUS.store(-1, Ordering::Relaxed);

    // SAFETY: test code owns resetting this process-global pointer here.
    unsafe {
      environ = ptr::null_mut();
    }
  }

  fn push_call(marker: u8) {
    let mut guard = match log().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    guard.push(marker);
  }

  fn snapshot_calls() -> Vec<u8> {
    let guard = match log().lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };

    guard.clone()
  }

  unsafe extern "C" fn init_first() {
    // SAFETY: Reading process-global pointer in test-only constructor.
    let current_environ = unsafe { environ };

    OBSERVED_ENVIRON_DURING_INIT.store(current_environ as usize, Ordering::Relaxed);
    push_call(1);
  }

  unsafe extern "C" fn init_second() {
    push_call(2);
  }

  unsafe extern "C" fn fini_first() {
    push_call(4);
  }

  unsafe extern "C" fn fini_second() {
    push_call(5);
  }

  unsafe extern "C" fn test_main(
    count: c_int,
    arg_ptr: *mut *mut c_char,
    env_ptr: *mut *mut c_char,
  ) -> c_int {
    OBSERVED_ARGC.store(count, Ordering::Relaxed);
    OBSERVED_ARGV.store(arg_ptr as usize, Ordering::Relaxed);
    OBSERVED_ENVP.store(env_ptr as usize, Ordering::Relaxed);
    push_call(3);

    77
  }

  fn trap_exit(status: c_int) -> ! {
    TRAPPED_EXIT_STATUS.store(status, Ordering::Relaxed);
    push_call(6);
    panic!("trap_exit")
  }

  #[test]
  fn startup_main_binds_environ_and_runs_init_main_fini_sequence() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 2] = [init_first, init_second];
    let fini_entries: [InitFiniFn; 2] = [fini_first, fini_second];
    let args = StartupArgs::new(2, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );

    // SAFETY: test arrays are valid contiguous Init/Fini ranges and function
    // pointers are valid test stubs.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4]);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(
      OBSERVED_ARGV.load(Ordering::Relaxed),
      argv_storage.as_mut_ptr() as usize
    );
    assert_eq!(
      OBSERVED_ENVP.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize
    );
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_supports_empty_init_and_fini_ranges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let entries: [InitFiniFn; 1] = [init_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let empty_range = InitFiniRange::new(entries.as_ptr(), entries.as_ptr());

    // SAFETY: init/fini ranges are empty because start == end.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, empty_range, empty_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3]);

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_skips_null_entries_inside_valid_ranges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [Option<InitFiniFn>; 3] = [Some(init_first), None, Some(init_second)];
    let fini_entries: [Option<InitFiniFn>; 3] = [Some(fini_first), None, Some(fini_second)];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());

    assert_eq!(
      mem::size_of::<Option<InitFiniFn>>(),
      mem::size_of::<InitFiniFn>(),
    );
    assert_eq!(
      mem::align_of::<Option<InitFiniFn>>(),
      mem::align_of::<InitFiniFn>(),
    );

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);

    // SAFETY: ranges are contiguous pointer-sized entries. Null entries should be skipped.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_skips_null_entries_at_range_edges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [Option<InitFiniFn>; 4] = [None, Some(init_first), Some(init_second), None];
    let fini_entries: [Option<InitFiniFn>; 4] = [None, Some(fini_first), Some(fini_second), None];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());

    assert_eq!(
      mem::size_of::<Option<InitFiniFn>>(),
      mem::size_of::<InitFiniFn>(),
    );
    assert_eq!(
      mem::align_of::<Option<InitFiniFn>>(),
      mem::align_of::<InitFiniFn>(),
    );

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);

    // SAFETY: ranges are contiguous pointer-sized entries. Null edge entries
    // should be skipped defensively.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_skips_misaligned_non_null_entries_inside_valid_ranges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 3] = [
      1,
      init_first as *const () as usize,
      init_second as *const () as usize,
    ];
    let fini_entries: [usize; 3] = [
      1,
      fini_first as *const () as usize,
      fini_second as *const () as usize,
    ];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);

    // SAFETY: ranges are contiguous pointer-sized entries. Misaligned non-null
    // slots should be skipped defensively.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_skips_misaligned_non_null_entries_at_range_edges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 4] = [
      init_first as *const () as usize,
      init_second as *const () as usize,
      init_first as *const () as usize,
      1,
    ];
    let fini_entries: [usize; 4] = [
      fini_first as *const () as usize,
      fini_second as *const () as usize,
      fini_first as *const () as usize,
      1,
    ];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);

    // SAFETY: ranges are contiguous pointer-sized entries. Misaligned non-null
    // edge slots should be skipped defensively.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 2, 1, 3, 4, 5, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_skips_mixed_null_and_misaligned_non_null_entries() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 6] = [
      0,
      1,
      init_first as *const () as usize,
      init_second as *const () as usize,
      1,
      0,
    ];
    let fini_entries: [usize; 6] = [
      0,
      1,
      fini_first as *const () as usize,
      fini_second as *const () as usize,
      1,
      0,
    ];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);

    // SAFETY: ranges are contiguous pointer-sized entries. Null and misaligned
    // non-null slots should be skipped defensively.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_skips_mixed_misaligned_edge_and_null_inner_entries() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 5] = [
      1,
      init_first as *const () as usize,
      0,
      init_second as *const () as usize,
      1,
    ];
    let fini_entries: [usize; 5] = [
      1,
      fini_first as *const () as usize,
      0,
      fini_second as *const () as usize,
      1,
    ];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);

    // SAFETY: ranges are contiguous pointer-sized entries. Misaligned non-null
    // edge slots and null inner slots should be skipped defensively.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_skips_interleaved_null_and_misaligned_entries() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 5] = [
      init_first as *const () as usize,
      0,
      1,
      init_second as *const () as usize,
      0,
    ];
    let fini_entries: [usize; 5] = [
      fini_second as *const () as usize,
      0,
      1,
      fini_first as *const () as usize,
      0,
    ];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);

    // SAFETY: ranges are contiguous pointer-sized entries. Interleaved null
    // and misaligned non-null slots should be skipped defensively.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 4, 5]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_treats_null_init_and_fini_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let null_range = InitFiniRange::new(ptr::null(), ptr::null());

    // SAFETY: null init/fini endpoints are accepted as defensive empty ranges.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, null_range, null_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3]);
    assert_eq!(
      OBSERVED_ARGV.load(Ordering::Relaxed),
      argv_storage.as_mut_ptr() as usize
    );
    assert_eq!(
      OBSERVED_ENVP.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize
    );
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for null ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_treats_partially_null_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(init_entries.as_ptr(), ptr::null());
    let fini_range = InitFiniRange::new(
      ptr::null(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );

    // SAFETY: null endpoints are accepted as defensive empty ranges.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for partial-null ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_treats_inverse_partially_null_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(
      ptr::null(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(fini_entries.as_ptr(), ptr::null());

    // SAFETY: null endpoints are accepted as defensive empty ranges.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for partial-null ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_treats_reversed_init_and_fini_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(
      init_entries.as_ptr().wrapping_add(init_entries.len()),
      init_entries.as_ptr(),
    );
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
      fini_entries.as_ptr(),
    );

    // SAFETY: reversed ranges are accepted as defensive empty ranges.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for reversed ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_treats_oversized_init_and_fini_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let oversized_start_addr = mem::align_of::<InitFiniFn>() * 2;
    let oversized_distance = (isize::MAX as usize) + 1;

    assert_eq!(oversized_distance % mem::size_of::<InitFiniFn>(), 0);

    let oversized_end_addr = oversized_start_addr + oversized_distance;
    let oversized_start = oversized_start_addr as *const InitFiniFn;
    let oversized_end = oversized_end_addr as *const InitFiniFn;
    let oversized_range = InitFiniRange::new(oversized_start, oversized_end);

    // SAFETY: oversized ranges are accepted as defensive empty ranges.
    let status = unsafe {
      run_startup_main(
        test_main as StartMainFn,
        args,
        oversized_range,
        oversized_range,
      )
    };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for oversized ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_init_with_partially_null_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(fini_entries.as_ptr(), ptr::null());

    // SAFETY: valid init range runs; partial-null fini range is defensive no-op.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_init_with_inverse_partially_null_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(ptr::null(), fini_entries.as_ptr());

    // SAFETY: valid init range runs; inverse partial-null fini range is defensive no-op.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_init_with_misaligned_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let entry_size = mem::size_of::<InitFiniFn>();
    let aligned_base = mem::align_of::<InitFiniFn>() * 2;
    let misaligned_start = (aligned_base + 1) as *const InitFiniFn;
    let misaligned_end = (aligned_base + 1 + entry_size) as *const InitFiniFn;
    let fini_range = InitFiniRange::new(misaligned_start, misaligned_end);

    // SAFETY: valid init range runs; misaligned fini range is defensive no-op.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_init_with_oversized_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let oversized_start_addr = mem::align_of::<InitFiniFn>() * 2;
    let oversized_distance = (isize::MAX as usize) + 1;

    assert_eq!(oversized_distance % mem::size_of::<InitFiniFn>(), 0);

    let oversized_end_addr = oversized_start_addr + oversized_distance;
    let oversized_start = oversized_start_addr as *const InitFiniFn;
    let oversized_end = oversized_end_addr as *const InitFiniFn;
    let fini_range = InitFiniRange::new(oversized_start, oversized_end);

    // SAFETY: valid init range runs; oversized fini range is defensive no-op.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_init_with_empty_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(fini_entries.as_ptr(), fini_entries.as_ptr());

    // SAFETY: valid init range runs; empty fini range is defensive no-op.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![1, 3]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_storage.as_mut_ptr() as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_fini_with_empty_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(init_entries.as_ptr(), init_entries.as_ptr());
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );

    // SAFETY: empty init range is defensive no-op; valid fini range still runs.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for empty ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_fini_with_partially_null_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(init_entries.as_ptr(), ptr::null());
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );

    // SAFETY: partial-null init range is no-op; valid fini range still runs.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for partial-null ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_fini_with_inverse_partially_null_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let init_range = InitFiniRange::new(ptr::null(), init_entries.as_ptr());
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );

    // SAFETY: inverse partial-null init range is no-op; valid fini range still runs.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for inverse partial-null ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_fini_with_misaligned_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let entry_size = mem::size_of::<InitFiniFn>();
    let aligned_base = mem::align_of::<InitFiniFn>() * 2;
    let misaligned_start = (aligned_base + 1) as *const InitFiniFn;
    let misaligned_end = (aligned_base + 1 + entry_size) as *const InitFiniFn;
    let init_range = InitFiniRange::new(misaligned_start, misaligned_end);
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );

    // SAFETY: misaligned init range is defensive no-op; valid fini range still runs.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for misaligned ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn startup_main_runs_valid_fini_with_oversized_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let args = StartupArgs::new(1, argv_storage.as_mut_ptr(), envp_storage.as_mut_ptr());
    let oversized_start_addr = mem::align_of::<InitFiniFn>() * 2;
    let oversized_distance = (isize::MAX as usize) + 1;

    assert_eq!(oversized_distance % mem::size_of::<InitFiniFn>(), 0);

    let oversized_end_addr = oversized_start_addr + oversized_distance;
    let oversized_start = oversized_start_addr as *const InitFiniFn;
    let oversized_end = oversized_end_addr as *const InitFiniFn;
    let init_range = InitFiniRange::new(oversized_start, oversized_end);
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );

    // SAFETY: oversized init range is defensive no-op; valid fini range still runs.
    let status =
      unsafe { run_startup_main(test_main as StartMainFn, args, init_range, fini_range) };

    assert_eq!(status, 77);
    assert_eq!(snapshot_calls(), vec![3, 4]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for oversized ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_storage.as_mut_ptr());
  }

  #[test]
  fn libc_start_main_path_terminates_with_main_status() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 2] = [init_first, init_second];
    let fini_entries: [InitFiniFn; 2] = [fini_first, fini_second];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: startup ranges and main pointer are valid test stubs.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4, 6]);

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_skips_null_entries_inside_valid_ranges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [Option<InitFiniFn>; 3] = [Some(init_first), None, Some(init_second)];
    let fini_entries: [Option<InitFiniFn>; 3] = [Some(fini_first), None, Some(fini_second)];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);

    assert_eq!(
      mem::size_of::<Option<InitFiniFn>>(),
      mem::size_of::<InitFiniFn>(),
    );
    assert_eq!(
      mem::align_of::<Option<InitFiniFn>>(),
      mem::align_of::<InitFiniFn>(),
    );

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: range shapes are valid and null slots should be skipped.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_skips_misaligned_non_null_entries_inside_valid_ranges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 3] = [
      1,
      init_first as *const () as usize,
      init_second as *const () as usize,
    ];
    let fini_entries: [usize; 3] = [
      1,
      fini_first as *const () as usize,
      fini_second as *const () as usize,
    ];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: ranges are contiguous pointer-sized entries. Misaligned
      // non-null slots should be skipped defensively.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_skips_misaligned_non_null_entries_at_range_edges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 4] = [
      init_first as *const () as usize,
      init_second as *const () as usize,
      init_first as *const () as usize,
      1,
    ];
    let fini_entries: [usize; 4] = [
      fini_first as *const () as usize,
      fini_second as *const () as usize,
      fini_first as *const () as usize,
      1,
    ];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: ranges are contiguous pointer-sized entries. Misaligned
      // non-null edge slots should be skipped defensively.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 2, 1, 3, 4, 5, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_skips_mixed_null_and_misaligned_non_null_entries() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 6] = [
      0,
      1,
      init_first as *const () as usize,
      init_second as *const () as usize,
      1,
      0,
    ];
    let fini_entries: [usize; 6] = [
      0,
      1,
      fini_first as *const () as usize,
      fini_second as *const () as usize,
      1,
      0,
    ];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: ranges are contiguous pointer-sized entries. Null and
      // misaligned non-null slots should be skipped defensively.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_skips_interleaved_null_and_misaligned_entries() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 5] = [
      init_first as *const () as usize,
      0,
      1,
      init_second as *const () as usize,
      0,
    ];
    let fini_entries: [usize; 5] = [
      fini_second as *const () as usize,
      0,
      1,
      fini_first as *const () as usize,
      0,
    ];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: ranges are contiguous pointer-sized entries. Interleaved null
      // and misaligned non-null slots should be skipped defensively.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_preserves_repeated_valid_entries_with_mixed_skips() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 5] = [
      init_first as *const () as usize,
      0,
      init_first as *const () as usize,
      1,
      init_second as *const () as usize,
    ];
    let fini_entries: [usize; 5] = [
      fini_first as *const () as usize,
      0,
      fini_first as *const () as usize,
      1,
      fini_second as *const () as usize,
    ];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: ranges are contiguous pointer-sized entries. Null and
      // misaligned non-null slots should be skipped while preserving repeated
      // valid entries.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 1, 2, 3, 5, 4, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_skips_mixed_misaligned_edge_and_null_inner_entries() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [usize; 5] = [
      1,
      init_first as *const () as usize,
      0,
      init_second as *const () as usize,
      1,
    ];
    let fini_entries: [usize; 5] = [
      1,
      fini_first as *const () as usize,
      0,
      fini_second as *const () as usize,
      1,
    ];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);

    assert_eq!(mem::size_of::<usize>(), mem::size_of::<InitFiniFn>());
    assert_eq!(mem::align_of::<usize>(), mem::align_of::<InitFiniFn>());

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: ranges are contiguous pointer-sized entries. Misaligned
      // non-null edge slots and null inner slots should be skipped defensively.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_skips_null_entries_at_range_edges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 2];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [Option<InitFiniFn>; 4] = [None, Some(init_first), Some(init_second), None];
    let fini_entries: [Option<InitFiniFn>; 4] = [None, Some(fini_first), Some(fini_second), None];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(2, argv_ptr, envp_ptr);

    assert_eq!(
      mem::size_of::<Option<InitFiniFn>>(),
      mem::size_of::<InitFiniFn>(),
    );
    assert_eq!(
      mem::align_of::<Option<InitFiniFn>>(),
      mem::align_of::<InitFiniFn>(),
    );

    let init_start = init_entries.as_ptr().cast::<InitFiniFn>();
    let fini_start = fini_entries.as_ptr().cast::<InitFiniFn>();
    // SAFETY: these pointers come from contiguous local arrays.
    let init_end = unsafe { init_start.add(init_entries.len()) };
    // SAFETY: these pointers come from contiguous local arrays.
    let fini_end = unsafe { fini_start.add(fini_entries.len()) };
    let init_range = InitFiniRange::new(init_start, init_end);
    let fini_range = InitFiniRange::new(fini_start, fini_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: range shapes are valid and null edge slots should be skipped.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), 2);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), argv_ptr as usize);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), envp_ptr as usize);
    assert_eq!(snapshot_calls(), vec![1, 2, 3, 5, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_with_null_main_terminates_immediately() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let sentinel_environ = argv_ptr;
    // SAFETY: test seeds a sentinel value to verify null-main fail-fast still
    // performs startup-time environ binding.
    unsafe {
      environ = sentinel_environ;
    }

    let args = StartupArgs::new(0, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: this test intentionally passes a null `main` to verify
      // fail-fast termination behavior.
      unsafe {
        run_libc_start_main_with(None, args, init_range, fini_range, trap_exit);
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(
      TRAPPED_EXIT_STATUS.load(Ordering::Relaxed),
      MISSING_MAIN_STATUS
    );
    assert_eq!(OBSERVED_ARGC.load(Ordering::Relaxed), -1);
    assert_eq!(OBSERVED_ARGV.load(Ordering::Relaxed), 0);
    assert_eq!(OBSERVED_ENVP.load(Ordering::Relaxed), 0);
    assert_eq!(OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed), 0);
    assert_eq!(snapshot_calls(), vec![6]);

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_treats_reversed_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr().wrapping_add(init_entries.len()),
      init_entries.as_ptr(),
    );
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
      fini_entries.as_ptr(),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: reversed init/fini ranges are expected to be treated as empty.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for reversed ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_treats_null_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let null_range = InitFiniRange::new(ptr::null(), ptr::null());
    let unwind = panic::catch_unwind(|| {
      // SAFETY: null init/fini ranges are expected to be treated as empty.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          null_range,
          null_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for null ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_treats_misaligned_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let entry_size = mem::size_of::<InitFiniFn>();
    let aligned_base = mem::align_of::<InitFiniFn>() * 2;
    let misaligned_start = (aligned_base + 1) as *const InitFiniFn;
    let misaligned_end = (aligned_base + 1 + entry_size) as *const InitFiniFn;
    let misaligned_range = InitFiniRange::new(misaligned_start, misaligned_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: misaligned ranges are expected to be treated as defensive empty.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          misaligned_range,
          misaligned_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for misaligned ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_treats_oversized_ranges_as_empty() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let oversized_start_addr = mem::align_of::<InitFiniFn>() * 2;
    let oversized_distance = (isize::MAX as usize) + 1;

    assert_eq!(oversized_distance % mem::size_of::<InitFiniFn>(), 0);

    let oversized_end_addr = oversized_start_addr + oversized_distance;
    let oversized_start = oversized_start_addr as *const InitFiniFn;
    let oversized_end = oversized_end_addr as *const InitFiniFn;
    let oversized_range = InitFiniRange::new(oversized_start, oversized_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: oversized ranges are expected to be treated as defensive empty.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          oversized_range,
          oversized_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for oversized ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_fini_with_partially_null_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(ptr::null(), init_entries.as_ptr());
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: partial-null init range is defensive no-op; fini range is valid.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for partial-null ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_fini_with_inverse_partially_null_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(init_entries.as_ptr(), ptr::null());
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: inverse partial init range is defensive no-op; fini range is valid.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for inverse partial-null ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_init_with_partially_null_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(fini_entries.as_ptr(), ptr::null());
    let unwind = panic::catch_unwind(|| {
      // SAFETY: init range is valid; partial-null fini range is defensive no-op.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![1, 3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_init_with_inverse_partially_null_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(ptr::null(), fini_entries.as_ptr());
    let unwind = panic::catch_unwind(|| {
      // SAFETY: init range is valid; inverse partial fini range is defensive no-op.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![1, 3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_init_with_reversed_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
      fini_entries.as_ptr(),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: init range is valid; reversed fini range is defensive no-op.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![1, 3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_init_with_misaligned_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );

    // Synthetic addresses are used only for entry-count validation; they are
    // intentionally misaligned and never dereferenced.
    let misaligned_start_addr = (mem::align_of::<InitFiniFn>() * 2) + 1;
    let misaligned_end_addr = misaligned_start_addr + mem::size_of::<InitFiniFn>();
    let fini_range = InitFiniRange::new(
      misaligned_start_addr as *const InitFiniFn,
      misaligned_end_addr as *const InitFiniFn,
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: init range is valid; misaligned fini range is defensive no-op.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![1, 3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_init_with_oversized_fini_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr(),
      init_entries.as_ptr().wrapping_add(init_entries.len()),
    );
    let oversized_start_addr = mem::align_of::<InitFiniFn>() * 2;
    let oversized_distance = (isize::MAX as usize) + 1;

    assert_eq!(oversized_distance % mem::size_of::<InitFiniFn>(), 0);

    let oversized_end_addr = oversized_start_addr + oversized_distance;
    let oversized_start = oversized_start_addr as *const InitFiniFn;
    let oversized_end = oversized_end_addr as *const InitFiniFn;
    let fini_range = InitFiniRange::new(oversized_start, oversized_end);
    let unwind = panic::catch_unwind(|| {
      // SAFETY: init range is valid; oversized fini range is defensive no-op.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![1, 3, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      envp_ptr as usize,
      "init constructors should observe bound environ",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_fini_with_reversed_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let init_entries: [InitFiniFn; 1] = [init_first];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let init_range = InitFiniRange::new(
      init_entries.as_ptr().wrapping_add(init_entries.len()),
      init_entries.as_ptr(),
    );
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: reversed init range is defensive no-op; fini range is valid.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for reversed ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_fini_with_misaligned_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);

    // Synthetic addresses are used only for entry-count validation; they are
    // intentionally misaligned and never dereferenced.
    let misaligned_start_addr = (mem::align_of::<InitFiniFn>() * 2) + 1;
    let misaligned_end_addr = misaligned_start_addr + mem::size_of::<InitFiniFn>();
    let init_range = InitFiniRange::new(
      misaligned_start_addr as *const InitFiniFn,
      misaligned_end_addr as *const InitFiniFn,
    );
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: misaligned init range is defensive no-op; fini range is valid.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for misaligned ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn libc_start_main_path_runs_valid_fini_with_oversized_init_range() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let mut argv_storage = [ptr::null_mut::<c_char>(); 1];
    let mut envp_storage = [ptr::null_mut::<c_char>(); 1];
    let fini_entries: [InitFiniFn; 1] = [fini_first];
    let argv_ptr = argv_storage.as_mut_ptr();
    let envp_ptr = envp_storage.as_mut_ptr();
    let args = StartupArgs::new(1, argv_ptr, envp_ptr);
    let oversized_start_addr = mem::align_of::<InitFiniFn>() * 2;
    let oversized_distance = (isize::MAX as usize) + 1;

    assert_eq!(oversized_distance % mem::size_of::<InitFiniFn>(), 0);

    let oversized_end_addr = oversized_start_addr + oversized_distance;
    let oversized_start = oversized_start_addr as *const InitFiniFn;
    let oversized_end = oversized_end_addr as *const InitFiniFn;
    let init_range = InitFiniRange::new(oversized_start, oversized_end);
    let fini_range = InitFiniRange::new(
      fini_entries.as_ptr(),
      fini_entries.as_ptr().wrapping_add(fini_entries.len()),
    );
    let unwind = panic::catch_unwind(|| {
      // SAFETY: oversized init range is defensive no-op; fini range is valid.
      unsafe {
        run_libc_start_main_with(
          Some(test_main as StartMainFn),
          args,
          init_range,
          fini_range,
          trap_exit,
        );
      }
    });

    assert!(unwind.is_err(), "trap_exit must panic to stop control flow");
    assert_eq!(TRAPPED_EXIT_STATUS.load(Ordering::Relaxed), 77);
    assert_eq!(snapshot_calls(), vec![3, 4, 6]);
    assert_eq!(
      OBSERVED_ENVIRON_DURING_INIT.load(Ordering::Relaxed),
      0,
      "init constructors must not run for oversized ranges",
    );

    // SAFETY: reading process-global pointer in test for assertion.
    let bound_environ = unsafe { environ };

    assert_eq!(bound_environ, envp_ptr);
  }

  #[test]
  fn entry_count_rejects_misaligned_pointer_ranges() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    // These are intentionally synthetic addresses used only for integer-based
    // range validation in `entry_count`; the test never dereferences them.
    let aligned_base = mem::align_of::<InitFiniFn>() * 2;
    let start_addr = aligned_base + 1;
    let end_addr = start_addr + mem::size_of::<InitFiniFn>();
    let start = start_addr as *const InitFiniFn;
    let end = end_addr as *const InitFiniFn;

    assert_eq!(entry_count(start, end), 0);
  }

  #[test]
  fn entry_count_rejects_ranges_larger_than_isize_max() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let start_addr = mem::align_of::<InitFiniFn>() * 2;
    let oversized_distance = (isize::MAX as usize) + 1;

    assert_eq!(oversized_distance % mem::size_of::<InitFiniFn>(), 0);

    let end_addr = start_addr + oversized_distance;
    let start = start_addr as *const InitFiniFn;
    let end = end_addr as *const InitFiniFn;

    assert_eq!(entry_count(start, end), 0);
  }

  #[test]
  fn entry_count_rejects_ranges_with_null_endpoint() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let aligned_non_null = mem::align_of::<InitFiniFn>() * 2;
    let aligned_end = (aligned_non_null + mem::size_of::<InitFiniFn>()) as *const InitFiniFn;

    assert_eq!(entry_count(ptr::null(), aligned_end), 0);
    assert_eq!(
      entry_count(aligned_non_null as *const InitFiniFn, ptr::null()),
      0
    );
  }

  #[test]
  fn read_array_entry_rejects_misaligned_non_null_pointer_values() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let entry_align = mem::align_of::<InitFiniFn>();

    if entry_align == 1 {
      return;
    }

    let raw_entries = [1_usize];
    let start = raw_entries.as_ptr().cast::<InitFiniFn>();

    // SAFETY: test constructs a one-element backing array and reads index 0.
    let entry = unsafe { super::read_array_entry(start, 0) };

    assert!(
      entry.is_none(),
      "misaligned non-null entry values must be treated as defensive no-op",
    );
  }

  #[test]
  fn read_array_entry_accepts_aligned_non_null_pointer_values() {
    let _test_guards = lock_test();
    let _environ_restore = EnvironRestore::capture();

    reset_state();

    let raw_entries = [init_first as *const () as usize];
    let start = raw_entries.as_ptr().cast::<InitFiniFn>();

    // SAFETY: test constructs a one-element backing array and reads index 0.
    let entry = unsafe { super::read_array_entry(start, 0) };
    let callback = entry.expect("aligned non-null entry value should be accepted");

    // SAFETY: callback comes from a valid test function pointer.
    unsafe {
      callback();
    }

    assert_eq!(snapshot_calls(), vec![1]);
  }
}
