#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_void;
use core::ptr;
use rlibc::abi::errno::{EDEADLK, EINVAL, ESRCH};
use rlibc::abi::types::c_int;
use rlibc::pthread::{pthread_attr_t, pthread_create, pthread_detach, pthread_join, pthread_t};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

unsafe extern "C" {
  fn pthread_attr_init(attr: *mut pthread_attr_t) -> c_int;
  fn pthread_attr_destroy(attr: *mut pthread_attr_t) -> c_int;
  fn pthread_attr_setstacksize(attr: *mut pthread_attr_t, stacksize: usize) -> c_int;
}

#[repr(C)]
struct SelfJoinState {
  observed_join_result: AtomicI32,
  self_id: AtomicU64,
}

#[repr(C)]
struct DetachCleanupState {
  release: AtomicBool,
  exited: AtomicBool,
}

const unsafe extern "C" fn return_argument(arg: *mut c_void) -> *mut c_void {
  arg
}

unsafe extern "C" fn attempt_self_join(arg: *mut c_void) -> *mut c_void {
  let state_ptr = arg.cast::<SelfJoinState>();
  // SAFETY: `arg` comes from `SelfJoinState` allocated by this test.
  let state = unsafe { &*state_ptr };
  let mut self_id = 0_u64;

  while self_id == 0 {
    self_id = state.self_id.load(Ordering::Acquire);

    if self_id == 0 {
      std::thread::yield_now();
    }
  }

  let thread = pthread_t::try_from(self_id)
    .unwrap_or_else(|_| unreachable!("u64 must fit into pthread_t on x86_64"));
  // SAFETY: `thread` is set by the creating thread before this call proceeds.
  let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

  state
    .observed_join_result
    .store(join_result, Ordering::Release);

  ptr::null_mut()
}

unsafe extern "C" fn wait_for_release_then_exit(arg: *mut c_void) -> *mut c_void {
  let state_ptr = arg.cast::<DetachCleanupState>();
  // SAFETY: `arg` comes from `DetachCleanupState` allocated by this test.
  let state = unsafe { &*state_ptr };

  while !state.release.load(Ordering::Acquire) {
    std::thread::yield_now();
  }

  state.exited.store(true, Ordering::Release);

  ptr::null_mut()
}

fn create_joinable_thread(
  start_routine: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
  arg: *mut c_void,
) -> pthread_t {
  let mut thread = 0 as pthread_t;
  // SAFETY: `thread` points to writable storage, `attr` is null, callback is valid.
  let create_result =
    unsafe { pthread_create(&raw mut thread, ptr::null(), Some(start_routine), arg) };

  assert_eq!(create_result, 0, "pthread_create must succeed");

  thread
}

fn create_native_joinable_thread(
  start_routine: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
  arg: *mut c_void,
) -> pthread_t {
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");
  // SAFETY: `attrs` has been initialized by libc and callback pointer is valid.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(start_routine),
      arg,
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0, "native pthread_create must succeed");

  thread
}

fn wait_until_detached_handle_released(thread: pthread_t, context: &str) {
  let mut observed_release = false;

  for _ in 0..10_000 {
    // SAFETY: join on detached thread is intentional for contract observation.
    let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

    if join_result == ESRCH {
      observed_release = true;

      break;
    }

    assert_eq!(
      join_result, EINVAL,
      "{context}: detached handle must stay EINVAL until release"
    );

    std::thread::yield_now();
  }

  assert!(
    observed_release,
    "{context}: detached handle must be released in bounded retries"
  );
}

fn release_and_wait_thread_exit(state_ptr: *mut DetachCleanupState, context: &str) {
  // SAFETY: `state_ptr` remains valid until converted back into a Box by caller.
  unsafe {
    (*state_ptr).release.store(true, Ordering::Release);
  }

  let mut observed_exit = false;

  for _ in 0..10_000 {
    // SAFETY: `state_ptr` remains valid until converted back into a Box by caller.
    if unsafe { (*state_ptr).exited.load(Ordering::Acquire) } {
      observed_exit = true;

      break;
    }

    std::thread::yield_now();
  }

  assert!(
    observed_exit,
    "{context}: worker thread must exit in bounded retries"
  );
}

fn wait_until_detach_reports_esrch(thread: pthread_t, context: &str) {
  let mut observed_release = false;

  for _ in 0..10_000 {
    let detach_result = pthread_detach(thread);

    if detach_result == ESRCH {
      observed_release = true;

      break;
    }

    assert_eq!(
      detach_result, EINVAL,
      "{context}: detached handle must stay EINVAL until release"
    );

    std::thread::yield_now();
  }

  assert!(
    observed_release,
    "{context}: detached handle must be released in bounded retries"
  );
}

fn assert_released_handle_stays_esrch(thread: pthread_t, context: &str) {
  for _ in 0..64 {
    let detach_result = pthread_detach(thread);
    // SAFETY: handle was already observed as released by detach/join probes.
    let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

    assert_eq!(
      detach_result, ESRCH,
      "{context}: detach must stay ESRCH after release"
    );
    assert_eq!(
      join_result, ESRCH,
      "{context}: join must stay ESRCH after release"
    );
  }
}

fn assert_detached_handle_stays_einval(thread: pthread_t, context: &str) {
  for _ in 0..64 {
    let detach_result = pthread_detach(thread);
    // SAFETY: detached native handle remains detached in forwarded pthread path.
    let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

    assert_eq!(
      detach_result, EINVAL,
      "{context}: detach must stay EINVAL for detached native handle"
    );
    assert_eq!(
      join_result, EINVAL,
      "{context}: join must stay EINVAL for detached native handle"
    );
  }
}

fn assert_detached_handle_stays_einval_pre_exit(thread: pthread_t, context: &str) {
  for _ in 0..64 {
    let detach_result = pthread_detach(thread);
    // SAFETY: the detached worker is intentionally kept blocked pre-exit.
    let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

    assert_eq!(
      detach_result, EINVAL,
      "{context}: detach must stay EINVAL while detached thread is still running"
    );
    assert_eq!(
      join_result, EINVAL,
      "{context}: join must stay EINVAL while detached thread is still running"
    );
  }
}

fn pthread_i041_serial_guard() -> std::sync::MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  LOCK
    .get_or_init(|| Mutex::new(()))
    .lock()
    .unwrap_or_else(PoisonError::into_inner)
}

fn assert_native_detach_after_exit_state(
  thread: pthread_t,
  first_detach_after_exit: c_int,
  join_after_detach: c_int,
  second_detach_after_exit: c_int,
  context: &str,
) {
  match first_detach_after_exit {
    0 | EINVAL => {
      assert_eq!(
        join_after_detach, EINVAL,
        "{context}: join must report detached state after detach-after-exit"
      );
      assert_eq!(
        second_detach_after_exit, EINVAL,
        "{context}: repeated detach must remain EINVAL after detach-after-exit"
      );
      assert_detached_handle_stays_einval(thread, context);
    }
    ESRCH => {
      assert_eq!(
        join_after_detach, ESRCH,
        "{context}: join must report released handle when detach-after-exit sees ESRCH"
      );
      assert_eq!(
        second_detach_after_exit, ESRCH,
        "{context}: repeated detach must stay ESRCH after release"
      );
      assert_released_handle_stays_esrch(thread, context);
    }
    unexpected => {
      panic!(
        "{context}: unexpected detach-after-exit result {unexpected}; expected 0/EINVAL/ESRCH"
      );
    }
  }
}

#[test]
fn pthread_create_and_join_round_trip_return_value() {
  let _serial = pthread_i041_serial_guard();
  let mut payload = 0x1234_u64;
  let arg = ptr::from_mut(&mut payload).cast::<c_void>();
  let thread = create_joinable_thread(return_argument, arg);
  let mut returned = ptr::null_mut();

  // SAFETY: thread id was produced by `pthread_create`; `returned` is writable.
  let join_result = unsafe { pthread_join(thread, &raw mut returned) };

  assert_eq!(join_result, 0);
  assert_eq!(returned, arg);
}

#[test]
fn pthread_create_rejects_null_output_pointer() {
  let _serial = pthread_i041_serial_guard();
  // SAFETY: null output pointer is intentional for contract validation.
  let create_result = unsafe {
    pthread_create(
      ptr::null_mut(),
      ptr::null(),
      Some(return_argument),
      ptr::null_mut(),
    )
  };

  assert_eq!(create_result, EINVAL);
}

#[test]
fn pthread_create_rejects_null_start_routine() {
  let _serial = pthread_i041_serial_guard();
  let mut thread = 0 as pthread_t;
  // SAFETY: null callback is intentional for contract validation.
  let create_result =
    unsafe { pthread_create(&raw mut thread, ptr::null(), None, ptr::null_mut()) };

  assert_eq!(create_result, EINVAL);
}

#[test]
fn pthread_create_allows_non_null_attributes() {
  let _serial = pthread_i041_serial_guard();
  let mut thread = 0 as pthread_t;
  let attrs = pthread_attr_t { __size: [0_u8; 56] };

  // SAFETY: `attrs` is initialized and passed by address.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(return_argument),
      ptr::null_mut(),
    )
  };

  assert_eq!(create_result, 0);

  // SAFETY: thread id was produced by `pthread_create`.
  let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

  assert_eq!(join_result, 0);
}

#[test]
fn pthread_create_accepts_initialized_native_attributes() {
  let _serial = pthread_i041_serial_guard();
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };

  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");

  // SAFETY: `attrs` has been initialized by libc.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(return_argument),
      ptr::null_mut(),
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0);

  // SAFETY: thread id was produced by `pthread_create`.
  let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

  assert_eq!(join_result, 0);
}

#[test]
fn pthread_create_accepts_non_zero_initialized_attributes() {
  let _serial = pthread_i041_serial_guard();
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };

  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");

  // SAFETY: `attrs` is initialized; 2 MiB is a valid non-zero stack size.
  let set_stack_result =
    unsafe { pthread_attr_setstacksize(ptr::from_mut(&mut attrs), 2 * 1024 * 1024) };

  assert_eq!(
    set_stack_result, 0,
    "pthread_attr_setstacksize must succeed"
  );
  // SAFETY: reading initialized raw attribute bytes is valid for regression checks.
  let has_non_zero_byte = unsafe { attrs.__size.iter().any(|byte| *byte != 0) };

  assert!(
    has_non_zero_byte,
    "initialized non-default pthread_attr_t must contain non-zero payload bytes",
  );

  // SAFETY: `attrs` has been initialized and configured by libc.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(return_argument),
      ptr::null_mut(),
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0);

  // SAFETY: thread id was produced by `pthread_create`.
  let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

  assert_eq!(join_result, 0);
}

#[test]
fn native_attr_non_zero_initialized_attributes_round_trip_return_value() {
  let _serial = pthread_i041_serial_guard();
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  let mut payload = 0x0A11_CE55_u64;
  let arg = ptr::from_mut(&mut payload).cast::<c_void>();
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");

  // SAFETY: `attrs` is initialized; 2 MiB is a valid non-zero stack size.
  let set_stack_result =
    unsafe { pthread_attr_setstacksize(ptr::from_mut(&mut attrs), 2 * 1024 * 1024) };

  assert_eq!(
    set_stack_result, 0,
    "pthread_attr_setstacksize must succeed"
  );
  // SAFETY: `attrs` has been initialized and configured by libc.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(return_argument),
      arg,
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };
  let mut returned = ptr::null_mut();

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0);
  // SAFETY: thread id was produced by `pthread_create`; `returned` is writable.
  let join_result = unsafe { pthread_join(thread, &raw mut returned) };
  // SAFETY: repeated join validates consumed-handle behavior.
  let second_join = unsafe { pthread_join(thread, ptr::null_mut()) };
  let detach_after_join = pthread_detach(thread);

  assert_eq!(join_result, 0);
  assert_eq!(returned, arg);
  assert_eq!(second_join, ESRCH);
  assert_eq!(detach_after_join, ESRCH);
}

#[test]
fn native_attr_thread_handle_is_consumed_after_join() {
  let _serial = pthread_i041_serial_guard();
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");

  // SAFETY: `attrs` has been initialized by libc.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(return_argument),
      ptr::null_mut(),
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0);

  // SAFETY: thread id was produced by `pthread_create`.
  let first_join = unsafe { pthread_join(thread, ptr::null_mut()) };
  // SAFETY: repeated join validates consumed-handle behavior.
  let second_join = unsafe { pthread_join(thread, ptr::null_mut()) };
  let detach_after_join = pthread_detach(thread);

  assert_eq!(first_join, 0);
  assert_eq!(second_join, ESRCH);
  assert_eq!(detach_after_join, ESRCH);
}

#[test]
fn native_attr_create_and_join_round_trip_return_value() {
  let _serial = pthread_i041_serial_guard();
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  let mut payload = 0xCAFE_BABE_u64;
  let arg = ptr::from_mut(&mut payload).cast::<c_void>();
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");

  // SAFETY: `attrs` has been initialized by libc and `arg` lives through join.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(return_argument),
      arg,
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };
  let mut returned = ptr::null_mut();

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0);
  // SAFETY: thread id was produced by `pthread_create`; `returned` is writable.
  let join_result = unsafe { pthread_join(thread, &raw mut returned) };
  // SAFETY: repeated join validates consumed-handle behavior.
  let second_join = unsafe { pthread_join(thread, ptr::null_mut()) };
  let detach_after_join = pthread_detach(thread);

  assert_eq!(join_result, 0);
  assert_eq!(returned, arg);
  assert_eq!(second_join, ESRCH);
  assert_eq!(detach_after_join, ESRCH);
}

#[test]
fn native_attr_create_rejects_null_output_pointer() {
  let _serial = pthread_i041_serial_guard();
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");
  // SAFETY: null output pointer is intentional for contract validation.
  let create_result = unsafe {
    pthread_create(
      ptr::null_mut(),
      ptr::from_ref(&attrs),
      Some(return_argument),
      ptr::null_mut(),
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, EINVAL);
}

#[test]
fn native_attr_null_start_preserves_output_slot_with_initialized_attr() {
  let _serial = pthread_i041_serial_guard();
  let sentinel = pthread_t::MAX - 3;
  let mut thread = sentinel;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");
  // SAFETY: null callback is intentional for contract validation.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      None,
      ptr::null_mut(),
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, EINVAL);
  assert_eq!(thread, sentinel);
}

#[test]
fn native_attr_detach_prevents_join_and_second_detach() {
  let _serial = pthread_i041_serial_guard();
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");
  // SAFETY: `attrs` has been initialized by libc.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(return_argument),
      ptr::null_mut(),
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0);

  let first_detach = pthread_detach(thread);
  // SAFETY: detached native thread must not be joinable.
  let join_after_detach = unsafe { pthread_join(thread, ptr::null_mut()) };
  let second_detach = pthread_detach(thread);

  assert_eq!(first_detach, 0);
  assert_eq!(join_after_detach, EINVAL);
  assert_eq!(second_detach, EINVAL);
}

#[test]
fn native_attr_detached_thread_handle_remains_detached_after_thread_exit() {
  let _serial = pthread_i041_serial_guard();
  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");
  // SAFETY: `attrs` has been initialized by libc; `state_ptr` remains valid until reclaimed below.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(wait_for_release_then_exit),
      state_ptr.cast::<c_void>(),
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0);

  let first_detach = pthread_detach(thread);
  let second_detach_before_exit = pthread_detach(thread);
  // SAFETY: checking join behavior of a detached-but-running thread.
  let join_before_exit = unsafe { pthread_join(thread, ptr::null_mut()) };

  assert_eq!(first_detach, 0);
  assert_eq!(second_detach_before_exit, EINVAL);
  assert_eq!(join_before_exit, EINVAL);

  release_and_wait_thread_exit(
    state_ptr,
    "detached native-attr thread must exit in bounded retries",
  );

  let detach_after_exit = pthread_detach(thread);
  // SAFETY: detached native handle remains detached in forwarded pthread path.
  let join_after_exit = unsafe { pthread_join(thread, ptr::null_mut()) };

  assert_eq!(detach_after_exit, EINVAL);
  assert_eq!(join_after_exit, EINVAL);
  assert_detached_handle_stays_einval(
    thread,
    "native detached handle post-exit stable detached state",
  );

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn native_attr_detach_after_thread_exit_keeps_detached_state() {
  let _serial = pthread_i041_serial_guard();
  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let mut thread = 0 as pthread_t;
  let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
  // SAFETY: `attrs` points to writable storage and is initialized by libc.
  let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

  assert_eq!(init_result, 0, "pthread_attr_init must succeed");
  // SAFETY: `attrs` has been initialized by libc; `state_ptr` remains valid until reclaimed below.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      Some(wait_for_release_then_exit),
      state_ptr.cast::<c_void>(),
    )
  };
  // SAFETY: `attrs` was initialized and must be destroyed once.
  let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

  assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
  assert_eq!(create_result, 0);

  release_and_wait_thread_exit(
    state_ptr,
    "native-attr joinable thread must exit before detach-after-exit check",
  );

  let detach_after_exit = pthread_detach(thread);
  // SAFETY: detached native handle remains detached in forwarded pthread path.
  let join_after_detach = unsafe { pthread_join(thread, ptr::null_mut()) };
  let second_detach_after_exit = pthread_detach(thread);

  assert_native_detach_after_exit_state(
    thread,
    detach_after_exit,
    join_after_detach,
    second_detach_after_exit,
    "native detach-after-exit path must keep detached state stable",
  );

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn native_attr_detached_state_repeats_without_stale_state() {
  let _serial = pthread_i041_serial_guard();
  let iterations = 24_usize;

  for _ in 0..iterations {
    let state = Box::new(DetachCleanupState {
      release: AtomicBool::new(false),
      exited: AtomicBool::new(false),
    });
    let state_ptr = Box::into_raw(state);
    let mut thread = 0 as pthread_t;
    let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
    // SAFETY: `attrs` points to writable storage and is initialized by libc.
    let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

    assert_eq!(init_result, 0, "pthread_attr_init must succeed");
    // SAFETY: `attrs` has been initialized by libc; `state_ptr` remains valid until reclaimed below.
    let create_result = unsafe {
      pthread_create(
        &raw mut thread,
        ptr::from_ref(&attrs),
        Some(wait_for_release_then_exit),
        state_ptr.cast::<c_void>(),
      )
    };
    // SAFETY: `attrs` was initialized and must be destroyed once.
    let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

    assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
    assert_eq!(create_result, 0);

    let first_detach = pthread_detach(thread);
    let second_detach_before_exit = pthread_detach(thread);
    // SAFETY: checking join behavior of a detached native thread.
    let join_before_exit = unsafe { pthread_join(thread, ptr::null_mut()) };

    assert_eq!(first_detach, 0);
    assert_eq!(second_detach_before_exit, EINVAL);
    assert_eq!(join_before_exit, EINVAL);

    release_and_wait_thread_exit(
      state_ptr,
      "repeated native detached thread must exit in bounded retries",
    );
    assert_detached_handle_stays_einval(
      thread,
      "repeated native detached thread must keep detached state stable",
    );

    // SAFETY: ownership is reclaimed exactly once here.
    unsafe {
      drop(Box::from_raw(state_ptr));
    }
  }
}

#[test]
fn native_attr_detach_after_exit_repeats_without_stale_state() {
  let _serial = pthread_i041_serial_guard();
  let iterations = 24_usize;

  for _ in 0..iterations {
    let state = Box::new(DetachCleanupState {
      release: AtomicBool::new(false),
      exited: AtomicBool::new(false),
    });
    let state_ptr = Box::into_raw(state);
    let mut thread = 0 as pthread_t;
    let mut attrs = pthread_attr_t { __size: [0_u8; 56] };
    // SAFETY: `attrs` points to writable storage and is initialized by libc.
    let init_result = unsafe { pthread_attr_init(ptr::from_mut(&mut attrs)) };

    assert_eq!(init_result, 0, "pthread_attr_init must succeed");
    // SAFETY: `attrs` has been initialized by libc; `state_ptr` remains valid until reclaimed below.
    let create_result = unsafe {
      pthread_create(
        &raw mut thread,
        ptr::from_ref(&attrs),
        Some(wait_for_release_then_exit),
        state_ptr.cast::<c_void>(),
      )
    };
    // SAFETY: `attrs` was initialized and must be destroyed once.
    let destroy_result = unsafe { pthread_attr_destroy(ptr::from_mut(&mut attrs)) };

    assert_eq!(destroy_result, 0, "pthread_attr_destroy must succeed");
    assert_eq!(create_result, 0);

    release_and_wait_thread_exit(
      state_ptr,
      "repeated native joinable thread must exit before detach-after-exit check",
    );

    let first_detach_after_exit = pthread_detach(thread);
    // SAFETY: detached native handle remains detached in forwarded pthread path.
    let join_after_detach = unsafe { pthread_join(thread, ptr::null_mut()) };
    let second_detach_after_exit = pthread_detach(thread);

    assert_native_detach_after_exit_state(
      thread,
      first_detach_after_exit,
      join_after_detach,
      second_detach_after_exit,
      "repeated native detach-after-exit path must keep detached state stable",
    );

    // SAFETY: ownership is reclaimed exactly once here.
    unsafe {
      drop(Box::from_raw(state_ptr));
    }
  }
}

#[test]
fn local_and_native_detached_handle_states_remain_isolated() {
  let _serial = pthread_i041_serial_guard();
  let native_state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let native_state_ptr = Box::into_raw(native_state);
  let native_thread = create_native_joinable_thread(
    wait_for_release_then_exit,
    native_state_ptr.cast::<c_void>(),
  );
  let native_detach_result = pthread_detach(native_thread);

  assert_eq!(native_detach_result, 0);
  assert_detached_handle_stays_einval_pre_exit(
    native_thread,
    "native detached thread must remain detached before exit",
  );

  let local_state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let local_state_ptr = Box::into_raw(local_state);
  let local_thread =
    create_joinable_thread(wait_for_release_then_exit, local_state_ptr.cast::<c_void>());
  let local_detach_result = pthread_detach(local_thread);

  assert_eq!(local_detach_result, 0);
  assert_detached_handle_stays_einval_pre_exit(
    local_thread,
    "local detached thread must not be released before exit",
  );

  release_and_wait_thread_exit(
    local_state_ptr,
    "local detached thread must exit before isolation local-release checks",
  );

  wait_until_detached_handle_released(
    local_thread,
    "local detached thread must converge to released handle after exit",
  );
  wait_until_detach_reports_esrch(
    local_thread,
    "local detached thread must report ESRCH on detach after release",
  );
  assert_released_handle_stays_esrch(
    local_thread,
    "local detached thread must remain ESRCH after release",
  );
  assert_detached_handle_stays_einval_pre_exit(
    native_thread,
    "native detached thread must remain isolated while local handle converges to ESRCH",
  );

  release_and_wait_thread_exit(
    native_state_ptr,
    "native detached thread must exit before isolation native post-checks",
  );

  let native_detach_after_exit = pthread_detach(native_thread);
  // SAFETY: post-exit detach/join probing is intentional for detached native path.
  let native_join_after_exit = unsafe { pthread_join(native_thread, ptr::null_mut()) };
  let native_second_detach_after_exit = pthread_detach(native_thread);

  assert_native_detach_after_exit_state(
    native_thread,
    native_detach_after_exit,
    native_join_after_exit,
    native_second_detach_after_exit,
    "native detached thread must keep isolated post-exit state",
  );

  // SAFETY: ownership is reclaimed exactly once per allocated state.
  unsafe {
    drop(Box::from_raw(native_state_ptr));
    drop(Box::from_raw(local_state_ptr));
  }
}

#[test]
fn pthread_create_failure_does_not_overwrite_output_slot() {
  let _serial = pthread_i041_serial_guard();
  let sentinel = pthread_t::MAX - 1;
  let mut thread = sentinel;

  // SAFETY: null callback is intentional for contract validation.
  let create_result =
    unsafe { pthread_create(&raw mut thread, ptr::null(), None, ptr::null_mut()) };

  assert_eq!(create_result, EINVAL);
  assert_eq!(thread, sentinel);
}

#[test]
fn pthread_create_null_start_with_non_null_attr_preserves_output_slot() {
  let _serial = pthread_i041_serial_guard();
  let sentinel = pthread_t::MAX - 2;
  let mut thread = sentinel;
  let attrs = pthread_attr_t { __size: [0_u8; 56] };
  // SAFETY: null start routine is intentional for contract validation.
  let create_result = unsafe {
    pthread_create(
      &raw mut thread,
      ptr::from_ref(&attrs),
      None,
      ptr::null_mut(),
    )
  };

  assert_eq!(create_result, EINVAL);
  assert_eq!(
    thread, sentinel,
    "pthread_create failure must preserve caller-provided output slot",
  );
}

#[test]
fn pthread_join_unknown_thread_returns_esrch() {
  let _serial = pthread_i041_serial_guard();
  // SAFETY: unknown thread id is intentional for contract validation.
  let join_result = unsafe { pthread_join(pthread_t::MAX, ptr::null_mut()) };

  assert_eq!(join_result, ESRCH);
}

#[test]
fn pthread_detach_unknown_thread_returns_esrch() {
  let _serial = pthread_i041_serial_guard();
  let detach_result = pthread_detach(pthread_t::MAX);

  assert_eq!(detach_result, ESRCH);
}

#[test]
fn pthread_unknown_plausible_native_handle_stays_esrch_after_probe() {
  let _serial = pthread_i041_serial_guard();
  let plausible_unknown_native = 0x0000_0001_0000_1234_u64 as pthread_t;

  // SAFETY: unknown-but-plausible native handle is intentional for contract validation.
  let first_join = unsafe { pthread_join(plausible_unknown_native, ptr::null_mut()) };
  let first_detach = pthread_detach(plausible_unknown_native);

  assert_eq!(first_join, ESRCH);
  assert_eq!(first_detach, ESRCH);

  for _ in 0..64 {
    // SAFETY: repeated probe validates consumed/unknown stability.
    let join_result = unsafe { pthread_join(plausible_unknown_native, ptr::null_mut()) };
    let detach_result = pthread_detach(plausible_unknown_native);

    assert_eq!(join_result, ESRCH);
    assert_eq!(detach_result, ESRCH);
  }
}

#[test]
fn pthread_unknown_probe_does_not_poison_next_local_thread_id() {
  let _serial = pthread_i041_serial_guard();
  let baseline = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let baseline_join = unsafe { pthread_join(baseline, ptr::null_mut()) };

  assert_eq!(baseline_join, 0);

  let Some(poisoned_candidate) = baseline.checked_add(1) else {
    panic!("pthread_t id increment overflowed unexpectedly");
  };
  // SAFETY: probing an unknown id is intentional for regression coverage.
  let unknown_join = unsafe { pthread_join(poisoned_candidate, ptr::null_mut()) };
  let unknown_detach = pthread_detach(poisoned_candidate);

  assert_eq!(unknown_join, ESRCH);
  assert_eq!(unknown_detach, ESRCH);

  let next_thread = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let next_join = unsafe { pthread_join(next_thread, ptr::null_mut()) };
  // SAFETY: repeated join validates consumed-handle behavior.
  let second_join = unsafe { pthread_join(next_thread, ptr::null_mut()) };

  assert_ne!(
    next_thread, poisoned_candidate,
    "unknown-handle probe must not reserve a local thread id that becomes unjoinable",
  );
  assert_eq!(next_join, 0);
  assert_eq!(second_join, ESRCH);
}

#[test]
fn pthread_unknown_probe_detach_first_does_not_poison_next_local_thread_id() {
  let _serial = pthread_i041_serial_guard();
  let baseline = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let baseline_join = unsafe { pthread_join(baseline, ptr::null_mut()) };

  assert_eq!(baseline_join, 0);

  let Some(poisoned_candidate) = baseline.checked_add(1) else {
    panic!("pthread_t id increment overflowed unexpectedly");
  };
  let unknown_detach = pthread_detach(poisoned_candidate);
  // SAFETY: probing an unknown id is intentional for regression coverage.
  let unknown_join = unsafe { pthread_join(poisoned_candidate, ptr::null_mut()) };

  assert_eq!(unknown_detach, ESRCH);
  assert_eq!(unknown_join, ESRCH);

  for _ in 0..32 {
    let detach_result = pthread_detach(poisoned_candidate);
    // SAFETY: repeated unknown probe validates detach-first consumed stability.
    let join_result = unsafe { pthread_join(poisoned_candidate, ptr::null_mut()) };

    assert_eq!(detach_result, ESRCH);
    assert_eq!(join_result, ESRCH);
  }

  let next_thread = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let next_join = unsafe { pthread_join(next_thread, ptr::null_mut()) };
  // SAFETY: repeated join validates consumed-handle behavior.
  let second_join = unsafe { pthread_join(next_thread, ptr::null_mut()) };

  assert_ne!(
    next_thread, poisoned_candidate,
    "detach-first unknown-handle probe must not reserve local id as consumed-native",
  );
  assert_eq!(next_join, 0);
  assert_eq!(second_join, ESRCH);
}

#[test]
fn pthread_unknown_probe_multiple_candidates_do_not_poison_local_sequence() {
  let _serial = pthread_i041_serial_guard();
  let baseline = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let baseline_join = unsafe { pthread_join(baseline, ptr::null_mut()) };

  assert_eq!(baseline_join, 0);

  let Some(poisoned_first) = baseline.checked_add(1) else {
    panic!("first pthread_t id increment overflowed unexpectedly");
  };
  let Some(poisoned_second) = poisoned_first.checked_add(1) else {
    panic!("second pthread_t id increment overflowed unexpectedly");
  };
  let first_detach = pthread_detach(poisoned_first);
  // SAFETY: probing unknown ids is intentional for regression coverage.
  let first_join = unsafe { pthread_join(poisoned_first, ptr::null_mut()) };
  // SAFETY: probing unknown ids is intentional for regression coverage.
  let second_join = unsafe { pthread_join(poisoned_second, ptr::null_mut()) };
  let second_detach = pthread_detach(poisoned_second);

  assert_eq!(first_detach, ESRCH);
  assert_eq!(first_join, ESRCH);
  assert_eq!(second_join, ESRCH);
  assert_eq!(second_detach, ESRCH);

  for _ in 0..32 {
    let detach_first = pthread_detach(poisoned_first);
    // SAFETY: repeated unknown probe validates stable consumed state.
    let join_first = unsafe { pthread_join(poisoned_first, ptr::null_mut()) };
    // SAFETY: repeated unknown probe validates stable consumed state.
    let join_second = unsafe { pthread_join(poisoned_second, ptr::null_mut()) };
    let detach_second = pthread_detach(poisoned_second);

    assert_eq!(detach_first, ESRCH);
    assert_eq!(join_first, ESRCH);
    assert_eq!(join_second, ESRCH);
    assert_eq!(detach_second, ESRCH);
  }

  let next_first = create_joinable_thread(return_argument, ptr::null_mut());
  let next_second = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread ids were produced by `pthread_create`.
  let next_first_join = unsafe { pthread_join(next_first, ptr::null_mut()) };
  // SAFETY: thread ids were produced by `pthread_create`.
  let next_second_join = unsafe { pthread_join(next_second, ptr::null_mut()) };
  // SAFETY: repeated join validates consumed-handle behavior.
  let next_first_second_join = unsafe { pthread_join(next_first, ptr::null_mut()) };
  // SAFETY: repeated join validates consumed-handle behavior.
  let next_second_second_join = unsafe { pthread_join(next_second, ptr::null_mut()) };

  assert_ne!(next_first, poisoned_first);
  assert_ne!(next_first, poisoned_second);
  assert_ne!(next_second, poisoned_first);
  assert_ne!(next_second, poisoned_second);
  assert_eq!(next_first_join, 0);
  assert_eq!(next_second_join, 0);
  assert_eq!(next_first_second_join, ESRCH);
  assert_eq!(next_second_second_join, ESRCH);
}

#[test]
fn pthread_unknown_probe_interleaved_with_local_threads_stays_stable() {
  let _serial = pthread_i041_serial_guard();
  let baseline = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let baseline_join = unsafe { pthread_join(baseline, ptr::null_mut()) };
  let mut poisoned_candidates = Vec::new();

  assert_eq!(baseline_join, 0);

  for step in 1_u64..=8_u64 {
    let Some(unknown_candidate) = baseline.checked_add(step as pthread_t) else {
      panic!("pthread_t id increment overflowed unexpectedly");
    };
    let (first_result, second_result) = if step % 2 == 0 {
      let detach_result = pthread_detach(unknown_candidate);
      // SAFETY: probing unknown ids is intentional for regression coverage.
      let join_result = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };

      (detach_result, join_result)
    } else {
      // SAFETY: probing unknown ids is intentional for regression coverage.
      let join_result = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };
      let detach_result = pthread_detach(unknown_candidate);

      (join_result, detach_result)
    };

    assert_eq!(first_result, ESRCH);
    assert_eq!(second_result, ESRCH);
    poisoned_candidates.push(unknown_candidate);

    let local_thread = create_joinable_thread(return_argument, ptr::null_mut());
    // SAFETY: thread id was produced by `pthread_create`.
    let local_join = unsafe { pthread_join(local_thread, ptr::null_mut()) };
    // SAFETY: repeated join validates consumed-handle behavior.
    let local_second_join = unsafe { pthread_join(local_thread, ptr::null_mut()) };

    assert!(
      !poisoned_candidates.contains(&local_thread),
      "interleaved unknown probes must not reserve local id as consumed-native",
    );
    assert_eq!(local_join, 0);
    assert_eq!(local_second_join, ESRCH);
  }

  for poisoned in poisoned_candidates {
    let detach_result = pthread_detach(poisoned);
    // SAFETY: repeated unknown probes validate stable consumed state.
    let join_result = unsafe { pthread_join(poisoned, ptr::null_mut()) };

    assert_eq!(detach_result, ESRCH);
    assert_eq!(join_result, ESRCH);
  }
}

#[test]
fn pthread_unknown_probe_does_not_break_local_detached_eventual_release() {
  let _serial = pthread_i041_serial_guard();
  let baseline = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let baseline_join = unsafe { pthread_join(baseline, ptr::null_mut()) };
  let Some(poisoned_first) = baseline.checked_add(1) else {
    panic!("first pthread_t id increment overflowed unexpectedly");
  };
  let Some(poisoned_second) = poisoned_first.checked_add(1) else {
    panic!("second pthread_t id increment overflowed unexpectedly");
  };

  assert_eq!(baseline_join, 0);

  // SAFETY: probing unknown ids is intentional for regression coverage.
  let first_join = unsafe { pthread_join(poisoned_first, ptr::null_mut()) };
  let first_detach = pthread_detach(poisoned_first);
  let second_detach = pthread_detach(poisoned_second);
  // SAFETY: probing unknown ids is intentional for regression coverage.
  let second_join = unsafe { pthread_join(poisoned_second, ptr::null_mut()) };

  assert_eq!(first_join, ESRCH);
  assert_eq!(first_detach, ESRCH);
  assert_eq!(second_detach, ESRCH);
  assert_eq!(second_join, ESRCH);

  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let thread = create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
  let first_detach_local = pthread_detach(thread);

  assert_ne!(thread, poisoned_first);
  assert_ne!(thread, poisoned_second);
  assert_eq!(first_detach_local, 0);
  assert_detached_handle_stays_einval_pre_exit(
    thread,
    "local detached thread must stay EINVAL before release after unknown probes",
  );

  release_and_wait_thread_exit(
    state_ptr,
    "local detached thread must exit after unknown probe sequence",
  );
  wait_until_detached_handle_released(
    thread,
    "local detached thread join path must still converge to ESRCH",
  );
  wait_until_detach_reports_esrch(
    thread,
    "local detached thread detach path must still converge to ESRCH",
  );
  assert_released_handle_stays_esrch(
    thread,
    "local detached thread must stay ESRCH after release despite unknown probes",
  );

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn pthread_released_detached_handle_stays_esrch_with_neighbor_unknown_probes() {
  let _serial = pthread_i041_serial_guard();
  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let detached_thread =
    create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
  let first_detach = pthread_detach(detached_thread);
  let Some(unknown_candidate) = detached_thread.checked_add(1) else {
    panic!("pthread_t id increment overflowed unexpectedly");
  };

  assert_eq!(first_detach, 0);
  assert_ne!(unknown_candidate, detached_thread);
  assert_detached_handle_stays_einval_pre_exit(
    detached_thread,
    "detached thread must stay EINVAL before release in neighbor-probe regression",
  );

  release_and_wait_thread_exit(
    state_ptr,
    "detached thread must exit before released-state regression probes",
  );
  wait_until_detached_handle_released(
    detached_thread,
    "detached thread join path must converge to ESRCH before neighbor probes",
  );
  wait_until_detach_reports_esrch(
    detached_thread,
    "detached thread detach path must converge to ESRCH before neighbor probes",
  );

  for probe_index in 0..64 {
    let (first_unknown, second_unknown) = if probe_index % 2 == 0 {
      // SAFETY: probing unknown ids is intentional for regression coverage.
      let join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };
      let detach = pthread_detach(unknown_candidate);

      (join, detach)
    } else {
      let detach = pthread_detach(unknown_candidate);
      // SAFETY: probing unknown ids is intentional for regression coverage.
      let join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };

      (detach, join)
    };
    // SAFETY: detached handle was already observed as released.
    let released_join = unsafe { pthread_join(detached_thread, ptr::null_mut()) };
    let released_detach = pthread_detach(detached_thread);

    assert_eq!(first_unknown, ESRCH);
    assert_eq!(second_unknown, ESRCH);
    assert_eq!(released_join, ESRCH);
    assert_eq!(released_detach, ESRCH);
  }

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn pthread_released_detached_unknown_probes_do_not_break_next_local_joinable_thread() {
  let _serial = pthread_i041_serial_guard();
  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let detached_thread =
    create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
  let first_detach = pthread_detach(detached_thread);
  let Some(unknown_candidate) = detached_thread.checked_add(1) else {
    panic!("pthread_t id increment overflowed unexpectedly");
  };

  assert_eq!(first_detach, 0);
  assert_detached_handle_stays_einval_pre_exit(
    detached_thread,
    "detached thread must stay EINVAL before release in joinable-follow-up regression",
  );

  release_and_wait_thread_exit(
    state_ptr,
    "detached thread must exit before released-handle neighbor probes",
  );
  wait_until_detached_handle_released(
    detached_thread,
    "detached thread join path must converge to ESRCH before follow-up create/join checks",
  );
  wait_until_detach_reports_esrch(
    detached_thread,
    "detached thread detach path must converge to ESRCH before follow-up create/join checks",
  );

  for _ in 0..32 {
    // SAFETY: probing unknown ids is intentional for regression coverage.
    let unknown_join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };
    let unknown_detach = pthread_detach(unknown_candidate);

    assert_eq!(unknown_join, ESRCH);
    assert_eq!(unknown_detach, ESRCH);
  }

  let local_thread = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let local_join = unsafe { pthread_join(local_thread, ptr::null_mut()) };
  // SAFETY: repeated join validates consumed-handle behavior.
  let local_second_join = unsafe { pthread_join(local_thread, ptr::null_mut()) };

  assert_ne!(
    local_thread, unknown_candidate,
    "unknown probes after detached-handle release must not reserve next local id",
  );
  assert_eq!(local_join, 0);
  assert_eq!(local_second_join, ESRCH);
  assert_released_handle_stays_esrch(
    detached_thread,
    "released detached handle must remain ESRCH after follow-up local thread lifecycle",
  );

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn pthread_unknown_probe_does_not_break_native_attr_round_trip() {
  let _serial = pthread_i041_serial_guard();
  let baseline = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let baseline_join = unsafe { pthread_join(baseline, ptr::null_mut()) };
  let Some(poisoned_candidate) = baseline.checked_add(1) else {
    panic!("pthread_t id increment overflowed unexpectedly");
  };

  assert_eq!(baseline_join, 0);

  for _ in 0..32 {
    // SAFETY: probing unknown ids is intentional for regression coverage.
    let unknown_join = unsafe { pthread_join(poisoned_candidate, ptr::null_mut()) };
    let unknown_detach = pthread_detach(poisoned_candidate);

    assert_eq!(unknown_join, ESRCH);
    assert_eq!(unknown_detach, ESRCH);
  }

  let mut payload = 0x7A11_CE42_u64;
  let arg = ptr::from_mut(&mut payload).cast::<c_void>();
  let native_thread = create_native_joinable_thread(return_argument, arg);
  let mut returned = ptr::null_mut();
  // SAFETY: native thread id was produced by `pthread_create` with initialized attrs.
  let join_result = unsafe { pthread_join(native_thread, &raw mut returned) };
  // SAFETY: repeated join validates consumed-handle behavior on native attr path.
  let second_join = unsafe { pthread_join(native_thread, ptr::null_mut()) };
  let detach_after_join = pthread_detach(native_thread);

  assert_eq!(join_result, 0);
  assert_eq!(returned, arg);
  assert_eq!(second_join, ESRCH);
  assert_eq!(detach_after_join, ESRCH);
}

#[test]
fn pthread_unknown_probe_during_local_detached_lifecycle_stays_eventually_releasable() {
  let _serial = pthread_i041_serial_guard();
  let baseline = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let baseline_join = unsafe { pthread_join(baseline, ptr::null_mut()) };

  assert_eq!(baseline_join, 0);

  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let detached_thread =
    create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
  let detach_result = pthread_detach(detached_thread);
  let Some(mut unknown_candidate) = detached_thread.checked_add(1) else {
    panic!("pthread_t id increment overflowed unexpectedly");
  };

  if unknown_candidate == baseline {
    let Some(next_candidate) = unknown_candidate.checked_add(1) else {
      panic!("pthread_t second id increment overflowed unexpectedly");
    };

    unknown_candidate = next_candidate;
  }

  assert_eq!(detach_result, 0);
  assert_ne!(unknown_candidate, baseline);
  assert_ne!(unknown_candidate, detached_thread);
  assert_detached_handle_stays_einval_pre_exit(
    detached_thread,
    "detached thread should stay EINVAL before explicit release while unknown probes run",
  );

  for iteration in 0..24 {
    let (first, second) = if iteration % 2 == 0 {
      // SAFETY: probing unknown ids is intentional for regression coverage.
      let join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };
      let detach = pthread_detach(unknown_candidate);

      (join, detach)
    } else {
      let detach = pthread_detach(unknown_candidate);
      // SAFETY: probing unknown ids is intentional for regression coverage.
      let join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };

      (detach, join)
    };

    assert_eq!(first, ESRCH);
    assert_eq!(second, ESRCH);
  }

  release_and_wait_thread_exit(
    state_ptr,
    "detached thread should exit after release even when unknown probes are interleaved",
  );
  wait_until_detached_handle_released(
    detached_thread,
    "detached thread join path should converge to ESRCH after release",
  );
  wait_until_detach_reports_esrch(
    detached_thread,
    "detached thread detach path should converge to ESRCH after release",
  );
  assert_released_handle_stays_esrch(
    detached_thread,
    "detached thread should stay ESRCH after release",
  );

  for _ in 0..8 {
    // SAFETY: probing unknown ids is intentional for regression coverage.
    let join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };
    let detach = pthread_detach(unknown_candidate);

    assert_eq!(join, ESRCH);
    assert_eq!(detach, ESRCH);
  }

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn pthread_unknown_probe_does_not_break_native_detached_post_exit_state() {
  let _serial = pthread_i041_serial_guard();
  let baseline = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let baseline_join = unsafe { pthread_join(baseline, ptr::null_mut()) };

  assert_eq!(baseline_join, 0);

  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let native_thread =
    create_native_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
  let first_detach = pthread_detach(native_thread);
  let Some(mut unknown_candidate) = native_thread.checked_add(1) else {
    panic!("pthread_t id increment overflowed unexpectedly");
  };

  if unknown_candidate == baseline {
    let Some(next_candidate) = unknown_candidate.checked_add(1) else {
      panic!("pthread_t second id increment overflowed unexpectedly");
    };

    unknown_candidate = next_candidate;
  }

  assert_eq!(first_detach, 0);
  assert_ne!(unknown_candidate, baseline);
  assert_ne!(unknown_candidate, native_thread);
  assert_detached_handle_stays_einval_pre_exit(
    native_thread,
    "native detached thread must stay EINVAL before exit under unknown probes",
  );

  for probe_index in 0..24 {
    let (first, second) = if probe_index % 2 == 0 {
      // SAFETY: probing unknown ids is intentional for regression coverage.
      let join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };
      let detach = pthread_detach(unknown_candidate);

      (join, detach)
    } else {
      let detach = pthread_detach(unknown_candidate);
      // SAFETY: probing unknown ids is intentional for regression coverage.
      let join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };

      (detach, join)
    };

    assert_eq!(first, ESRCH);
    assert_eq!(second, ESRCH);
  }

  release_and_wait_thread_exit(
    state_ptr,
    "native detached thread must exit in bounded retries after unknown probes",
  );

  let detach_after_exit = pthread_detach(native_thread);
  // SAFETY: post-exit probes are intentional for detached-native state validation.
  let join_after_exit = unsafe { pthread_join(native_thread, ptr::null_mut()) };
  let second_detach_after_exit = pthread_detach(native_thread);

  assert_native_detach_after_exit_state(
    native_thread,
    detach_after_exit,
    join_after_exit,
    second_detach_after_exit,
    "native detached post-exit state must remain valid after unknown probes",
  );

  for _ in 0..8 {
    // SAFETY: probing unknown ids is intentional for regression coverage.
    let join = unsafe { pthread_join(unknown_candidate, ptr::null_mut()) };
    let detach = pthread_detach(unknown_candidate);

    assert_eq!(join, ESRCH);
    assert_eq!(detach, ESRCH);
  }

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn pthread_join_is_single_consumer() {
  let _serial = pthread_i041_serial_guard();
  let thread = create_joinable_thread(return_argument, ptr::null_mut());

  // SAFETY: thread id was produced by `pthread_create`.
  let first_join = unsafe { pthread_join(thread, ptr::null_mut()) };
  // SAFETY: second join reuses same thread id intentionally.
  let second_join = unsafe { pthread_join(thread, ptr::null_mut()) };

  assert_eq!(first_join, 0);
  assert_eq!(second_join, ESRCH);
}

#[test]
fn pthread_detach_after_successful_join_returns_esrch() {
  let _serial = pthread_i041_serial_guard();
  let thread = create_joinable_thread(return_argument, ptr::null_mut());
  // SAFETY: thread id was produced by `pthread_create`.
  let first_join = unsafe { pthread_join(thread, ptr::null_mut()) };
  let detach_after_join = pthread_detach(thread);

  assert_eq!(first_join, 0);
  assert_eq!(detach_after_join, ESRCH);
}

#[test]
fn pthread_detach_prevents_join_and_second_detach() {
  let _serial = pthread_i041_serial_guard();
  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let thread = create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
  let detach_result = pthread_detach(thread);
  // SAFETY: joining a detached thread is an intentional failure-path check.
  let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };
  let second_detach_result = pthread_detach(thread);

  assert_eq!(detach_result, 0);
  assert_eq!(join_result, EINVAL);
  assert_eq!(second_detach_result, EINVAL);

  release_and_wait_thread_exit(
    state_ptr,
    "detached thread must observe release flag before cleanup",
  );

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn pthread_detach_after_thread_exit_releases_joinable_handle() {
  let _serial = pthread_i041_serial_guard();
  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let thread = create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());

  release_and_wait_thread_exit(
    state_ptr,
    "joinable thread must exit before detach-after-exit assertion",
  );

  let detach_after_exit = pthread_detach(thread);

  assert_eq!(detach_after_exit, 0);
  wait_until_detached_handle_released(thread, "detach-after-exit local thread handle release");
  wait_until_detach_reports_esrch(thread, "detach-after-exit local thread detach release");
  assert_released_handle_stays_esrch(thread, "detach-after-exit local thread stable release");

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn detached_thread_handle_is_released_after_thread_exit() {
  let _serial = pthread_i041_serial_guard();
  let state = Box::new(DetachCleanupState {
    release: AtomicBool::new(false),
    exited: AtomicBool::new(false),
  });
  let state_ptr = Box::into_raw(state);
  let thread = create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
  let first_detach = pthread_detach(thread);
  let second_detach_before_exit = pthread_detach(thread);
  // SAFETY: checking join behavior of a detached-but-running thread.
  let join_before_exit = unsafe { pthread_join(thread, ptr::null_mut()) };

  assert_eq!(first_detach, 0);
  assert_eq!(second_detach_before_exit, EINVAL);
  assert_eq!(join_before_exit, EINVAL);

  release_and_wait_thread_exit(state_ptr, "detached thread must exit in bounded retries");

  wait_until_detached_handle_released(thread, "local detached-thread post-exit release");
  wait_until_detach_reports_esrch(thread, "local detached-thread post-exit detach release");
  assert_released_handle_stays_esrch(thread, "local detached-thread stable release");

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn detached_thread_eventual_release_repeats_without_stale_state() {
  let _serial = pthread_i041_serial_guard();
  let iterations = 32_usize;

  for _ in 0..iterations {
    let state = Box::new(DetachCleanupState {
      release: AtomicBool::new(false),
      exited: AtomicBool::new(false),
    });
    let state_ptr = Box::into_raw(state);
    let thread = create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
    let first_detach = pthread_detach(thread);
    let second_detach_before_exit = pthread_detach(thread);
    // SAFETY: checking join behavior of a detached-but-running thread.
    let join_before_exit = unsafe { pthread_join(thread, ptr::null_mut()) };

    assert_eq!(first_detach, 0);
    assert_eq!(second_detach_before_exit, EINVAL);
    assert_eq!(join_before_exit, EINVAL);

    release_and_wait_thread_exit(
      state_ptr,
      "repeated detached thread must exit in bounded retries",
    );
    wait_until_detached_handle_released(thread, "repeated detached-thread local join release");
    wait_until_detach_reports_esrch(thread, "repeated detached-thread local detach release");
    assert_released_handle_stays_esrch(thread, "repeated detached-thread stable release");

    // SAFETY: ownership is reclaimed exactly once here.
    unsafe {
      drop(Box::from_raw(state_ptr));
    }
  }
}

#[test]
fn detached_thread_pre_exit_state_repeats_without_premature_release() {
  let _serial = pthread_i041_serial_guard();
  let iterations = 24_usize;

  for _ in 0..iterations {
    let state = Box::new(DetachCleanupState {
      release: AtomicBool::new(false),
      exited: AtomicBool::new(false),
    });
    let state_ptr = Box::into_raw(state);
    let thread = create_joinable_thread(wait_for_release_then_exit, state_ptr.cast::<c_void>());
    let first_detach = pthread_detach(thread);

    assert_eq!(first_detach, 0);
    assert_detached_handle_stays_einval_pre_exit(
      thread,
      "local detached thread must not be released before worker exit",
    );

    release_and_wait_thread_exit(
      state_ptr,
      "repeated local detached thread must exit in bounded retries",
    );
    wait_until_detached_handle_released(thread, "local detached-thread post-exit join release");
    wait_until_detach_reports_esrch(thread, "local detached-thread post-exit detach release");
    assert_released_handle_stays_esrch(
      thread,
      "local detached-thread post-exit stable released state",
    );

    // SAFETY: ownership is reclaimed exactly once here.
    unsafe {
      drop(Box::from_raw(state_ptr));
    }
  }
}

#[test]
fn pthread_join_self_returns_edeadlk() {
  let _serial = pthread_i041_serial_guard();
  let state = Box::new(SelfJoinState {
    observed_join_result: AtomicI32::new(0),
    self_id: AtomicU64::new(0),
  });
  let state_ptr = Box::into_raw(state);
  let thread = create_joinable_thread(attempt_self_join, state_ptr.cast::<c_void>());

  // SAFETY: `state_ptr` is valid and uniquely owned by this test.
  unsafe {
    (*state_ptr).self_id.store(thread as u64, Ordering::Release);
  }

  // SAFETY: thread id was produced by `pthread_create`.
  let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };
  // SAFETY: `state_ptr` remains valid until converted back into a Box below.
  let observed_join_result = unsafe { (*state_ptr).observed_join_result.load(Ordering::Acquire) };

  assert_eq!(join_result, 0);
  assert_eq!(observed_join_result, EDEADLK);

  // SAFETY: ownership is reclaimed exactly once here.
  unsafe {
    drop(Box::from_raw(state_ptr));
  }
}

#[test]
fn pthread_create_join_smoke_multiple_threads() {
  let _serial = pthread_i041_serial_guard();
  let count = 16_usize;
  let mut thread_ids = Vec::with_capacity(count);

  for _ in 0..count {
    thread_ids.push(create_joinable_thread(return_argument, ptr::null_mut()));
  }

  for thread in thread_ids {
    // SAFETY: thread id was produced by `pthread_create`.
    let join_result = unsafe { pthread_join(thread, ptr::null_mut()) };

    assert_eq!(join_result, 0);
  }
}
