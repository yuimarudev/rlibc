#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use rlibc::abi::errno::{EBUSY, EDEADLK, EINVAL, ENOTSUP, EPERM};
use rlibc::errno::__errno_location;
use rlibc::pthread::{
  PTHREAD_MUTEX_ERRORCHECK, PTHREAD_MUTEX_RECURSIVE, PTHREAD_PROCESS_PRIVATE,
  PTHREAD_PROCESS_SHARED, pthread_mutex_destroy, pthread_mutex_init, pthread_mutex_lock,
  pthread_mutex_t, pthread_mutex_trylock, pthread_mutex_unlock, pthread_mutexattr_destroy,
  pthread_mutexattr_getpshared, pthread_mutexattr_gettype, pthread_mutexattr_init,
  pthread_mutexattr_setpshared, pthread_mutexattr_settype, pthread_mutexattr_t,
};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn pthread_mutexattr_type_round_trip_recursive() {
  let mut attr = pthread_mutexattr_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_settype(&raw mut attr, PTHREAD_MUTEX_RECURSIVE),
    0
  );

  let mut observed_type = 0;

  assert_eq!(
    pthread_mutexattr_gettype(&raw const attr, &raw mut observed_type),
    0
  );
  assert_eq!(observed_type, PTHREAD_MUTEX_RECURSIVE);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_mutexattr_settype_invalid_returns_einval() {
  let mut attr = pthread_mutexattr_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(pthread_mutexattr_settype(&raw mut attr, 9999), EINVAL);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_mutexattr_pshared_shared_returns_enotsup() {
  let mut attr = pthread_mutexattr_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_setpshared(&raw mut attr, PTHREAD_PROCESS_SHARED),
    ENOTSUP,
  );
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_mutexattr_getpshared_returns_default_private() {
  let mut attr = pthread_mutexattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_getpshared(&raw const attr, &raw mut observed_pshared),
    0
  );
  assert_eq!(observed_pshared, PTHREAD_PROCESS_PRIVATE);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_mutexattr_operations_after_destroy_return_einval() {
  let mut attr = pthread_mutexattr_t::default();
  let mut observed_type = 0;
  let mut observed_pshared = 0;

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_gettype(&raw const attr, &raw mut observed_type),
    EINVAL
  );
  assert_eq!(
    pthread_mutexattr_getpshared(&raw const attr, &raw mut observed_pshared),
    EINVAL
  );
  assert_eq!(
    pthread_mutexattr_settype(&raw mut attr, PTHREAD_MUTEX_RECURSIVE),
    EINVAL
  );
  assert_eq!(
    pthread_mutexattr_setpshared(&raw mut attr, PTHREAD_PROCESS_PRIVATE),
    EINVAL
  );
}

#[test]
fn pthread_mutex_init_with_destroyed_attr_returns_einval() {
  let mut attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), EINVAL);
}

#[test]
fn pthread_mutex_init_with_destroyed_attr_keeps_mutex_uninitialized() {
  let mut attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), EINVAL);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), EINVAL);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), EINVAL);
}

#[test]
fn pthread_mutex_failed_init_allows_later_valid_init() {
  let mut attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), EINVAL);

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_init_with_uninitialized_attr_returns_einval() {
  let attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), EINVAL);
}

#[test]
fn pthread_mutex_init_with_uninitialized_attr_preserves_uninitialized_state() {
  let attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), EINVAL);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), EINVAL);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), EINVAL);

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_init_after_pshared_shared_rejection_uses_private_attr() {
  let mut attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_setpshared(&raw mut attr, PTHREAD_PROCESS_SHARED),
    ENOTSUP
  );
  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_mutexattr_operations_reject_null_pointer() {
  let mut observed_type = 0;
  let mut observed_pshared = 0;

  assert_eq!(pthread_mutexattr_init(core::ptr::null_mut()), EINVAL);
  assert_eq!(pthread_mutexattr_destroy(core::ptr::null_mut()), EINVAL);
  assert_eq!(
    pthread_mutexattr_gettype(core::ptr::null(), &raw mut observed_type),
    EINVAL
  );
  assert_eq!(
    pthread_mutexattr_gettype(&pthread_mutexattr_t::default(), core::ptr::null_mut()),
    EINVAL
  );
  assert_eq!(
    pthread_mutexattr_settype(core::ptr::null_mut(), PTHREAD_MUTEX_RECURSIVE),
    EINVAL
  );
  assert_eq!(
    pthread_mutexattr_getpshared(core::ptr::null(), &raw mut observed_pshared),
    EINVAL
  );
  assert_eq!(
    pthread_mutexattr_getpshared(&pthread_mutexattr_t::default(), core::ptr::null_mut()),
    EINVAL
  );
  assert_eq!(
    pthread_mutexattr_setpshared(core::ptr::null_mut(), PTHREAD_PROCESS_PRIVATE),
    EINVAL
  );
}

#[test]
fn pthread_mutex_init_lock_unlock_destroy_success() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_init_on_initialized_mutex_returns_ebusy() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), EBUSY);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_reinit_failure_preserves_existing_state() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), EBUSY);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_reinit_after_destroy_succeeds() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_trylock_reports_ebusy_when_already_locked() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_trylock(&raw mut mutex), EBUSY);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_errorcheck_self_lock_returns_edeadlk() {
  let mut attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_settype(&raw mut attr, PTHREAD_MUTEX_ERRORCHECK),
    0
  );
  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), EDEADLK);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_mutex_recursive_allows_nested_locking() {
  let mut attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_settype(&raw mut attr, PTHREAD_MUTEX_RECURSIVE),
    0
  );
  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_mutex_unlock_from_non_owner_returns_eperm() {
  let mut mutex = pthread_mutex_t::default();
  let mutex_addr = (&raw mut mutex).addr();
  let (locked_tx, locked_rx) = mpsc::channel();
  let (release_tx, release_rx) = mpsc::channel();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);

  let worker = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    locked_tx.send(()).expect("failed to notify locked state");
    release_rx
      .recv()
      .expect("failed waiting for release signal");
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  locked_rx.recv().expect("worker did not lock mutex");
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), EPERM);
  release_tx.send(()).expect("failed to release worker");
  worker.join().expect("worker panicked");
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_locked_returns_ebusy() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), EBUSY);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_locked_failure_preserves_mutex_state() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), EBUSY);

  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_second_destroy_returns_einval() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), EINVAL);
}

#[test]
fn pthread_mutex_operations_after_destroy_return_einval() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), EINVAL);
  assert_eq!(pthread_mutex_trylock(&raw mut mutex), EINVAL);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), EINVAL);
}

#[test]
fn pthread_mutex_operations_reject_null_pointer() {
  assert_eq!(
    pthread_mutex_init(core::ptr::null_mut(), core::ptr::null()),
    EINVAL
  );
  assert_eq!(pthread_mutex_destroy(core::ptr::null_mut()), EINVAL);
  assert_eq!(pthread_mutex_lock(core::ptr::null_mut()), EINVAL);
  assert_eq!(pthread_mutex_trylock(core::ptr::null_mut()), EINVAL);
  assert_eq!(pthread_mutex_unlock(core::ptr::null_mut()), EINVAL);
}

#[test]
fn pthread_mutex_operations_on_uninitialized_mutex_return_einval() {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_lock(&raw mut mutex), EINVAL);
  assert_eq!(pthread_mutex_trylock(&raw mut mutex), EINVAL);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), EINVAL);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), EINVAL);
}

#[test]
fn pthread_mutex_destroy_with_pending_waiter_returns_ebusy() {
  let mut mutex = pthread_mutex_t::default();
  let mutex_addr = (&raw mut mutex).addr();
  let (started_tx, started_rx) = mpsc::channel();
  let (acquired_tx, acquired_rx) = mpsc::channel();
  let (release_tx, release_rx) = mpsc::channel();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);

  let worker = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;

    started_tx.send(()).expect("failed to send start signal");
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    acquired_tx
      .send(())
      .expect("failed to send acquired notification");
    release_rx
      .recv()
      .expect("failed to receive release notification");
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("worker did not start");
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_millis(100)),
    Err(mpsc::RecvTimeoutError::Timeout),
    "worker must remain blocked while owner holds mutex",
  );

  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while a waiter still references this mutex",
  );

  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_secs(1)),
    Ok(()),
    "waiter must eventually acquire mutex after owner unlocks",
  );
  release_tx
    .send(())
    .expect("failed to send release notification");
  worker.join().expect("worker thread panicked");
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_lock_blocks_until_owner_unlocks() {
  let mut mutex = pthread_mutex_t::default();
  let mutex_addr = (&raw mut mutex).addr();
  let (started_tx, started_rx) = mpsc::channel();
  let (acquired_tx, acquired_rx) = mpsc::channel();
  let (release_tx, release_rx) = mpsc::channel();

  assert_eq!(pthread_mutex_init(&raw mut mutex, core::ptr::null()), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);

  let worker = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;

    started_tx.send(()).expect("failed to send start signal");
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    acquired_tx
      .send(())
      .expect("failed to send acquired notification");
    release_rx
      .recv()
      .expect("failed to receive release notification");
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("worker did not start");
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_millis(100)),
    Err(mpsc::RecvTimeoutError::Timeout),
    "contending thread must stay blocked while owner holds mutex",
  );

  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_secs(1)),
    Ok(()),
    "contending thread must acquire mutex after owner unlocks",
  );

  release_tx
    .send(())
    .expect("failed to send release notification");
  worker.join().expect("worker thread panicked");
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_errors_do_not_mutate_errno() {
  let mut attr = pthread_mutexattr_t::default();
  let errno_ptr = __errno_location();
  let sentinel = 1234;

  assert!(!errno_ptr.is_null(), "__errno_location returned null");

  // SAFETY: `__errno_location` returns writable TLS errno for this thread.
  unsafe {
    errno_ptr.write(sentinel);
  }

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(pthread_mutexattr_settype(&raw mut attr, 9999), EINVAL);
  // SAFETY: `errno_ptr` points to readable TLS errno for this thread.
  assert_eq!(
    unsafe { errno_ptr.read() },
    sentinel,
    "pthread error returns must not require errno writes",
  );

  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_setpshared(&raw mut attr, PTHREAD_PROCESS_PRIVATE),
    EINVAL
  );
  // SAFETY: `errno_ptr` points to readable TLS errno for this thread.
  assert_eq!(
    unsafe { errno_ptr.read() },
    sentinel,
    "pthread error returns must preserve existing errno state",
  );
}
