#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ptr;
use rlibc::abi::errno::{EBUSY, EDEADLK, EINVAL};
use rlibc::abi::types::c_int;
use rlibc::pthread::{
  PTHREAD_PROCESS_SHARED, pthread_rwlock_destroy, pthread_rwlock_init, pthread_rwlock_rdlock,
  pthread_rwlock_t, pthread_rwlock_tryrdlock, pthread_rwlock_trywrlock, pthread_rwlock_unlock,
  pthread_rwlock_wrlock, pthread_rwlockattr_t,
};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::Duration;

unsafe extern "C" {
  fn pthread_rwlockattr_init(attr: *mut pthread_rwlockattr_t) -> c_int;
  fn pthread_rwlockattr_setpshared(attr: *mut pthread_rwlockattr_t, pshared: c_int) -> c_int;
  fn pthread_rwlockattr_destroy(attr: *mut pthread_rwlockattr_t) -> c_int;
}

const fn new_rwlock() -> pthread_rwlock_t {
  pthread_rwlock_t { __size: [0_u8; 56] }
}

fn init_rwlock(rwlock: &mut pthread_rwlock_t) {
  let rwlock_ptr = ptr::from_mut(rwlock);
  // SAFETY: `rwlock_ptr` points to writable storage for one pthread_rwlock_t.
  let init_result = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::null()) };

  assert_eq!(init_result, 0, "pthread_rwlock_init must succeed");
}

fn rwlock_addr(rwlock: &mut pthread_rwlock_t) -> usize {
  ptr::from_mut(rwlock).addr()
}

#[test]
fn pthread_rwlock_init_and_destroy_succeed() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  // SAFETY: `rwlock` is initialized and points to valid lock storage.
  let destroy_result = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_result, 0);
}

#[test]
fn pthread_rwlock_destroy_uninitialized_returns_einval() {
  let mut rwlock = pthread_rwlock_t { __size: [0_u8; 56] };
  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  // SAFETY: pointer refers to rwlock storage that was never initialized.
  let destroy_result = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_result, EINVAL);
}

#[test]
fn pthread_rwlock_can_reinitialize_after_destroy() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  // SAFETY: lock was initialized by `init_rwlock`.
  let first_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(first_destroy, 0);
  // SAFETY: lock storage is reusable after successful destroy.
  let reinit_result = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::null()) };

  assert_eq!(reinit_result, 0);
  // SAFETY: lock was reinitialized immediately above.
  let second_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(second_destroy, 0);
}

#[test]
fn pthread_rwlock_second_destroy_returns_einval() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  // SAFETY: lock was initialized by `init_rwlock`.
  let first_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(first_destroy, 0);
  // SAFETY: lock was already destroyed; destroy must fail with EINVAL.
  let second_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(second_destroy, EINVAL);
}

#[test]
fn pthread_rwlock_init_accepts_non_null_attr_pointer_as_default_attributes() {
  let mut rwlock = pthread_rwlock_t { __size: [0_u8; 56] };
  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let attr = pthread_rwlockattr_t { __size: [0_u8; 8] };
  // SAFETY: pointer inputs are valid; this baseline treats non-null rwlock attrs as default attrs.
  let init_with_attr = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::from_ref(&attr)) };

  assert_eq!(init_with_attr, 0);
  // SAFETY: lock was initialized with non-null attrs in this test.
  let destroy_result = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_result, 0);
}

#[test]
fn pthread_rwlock_init_accepts_raw_attr_bytes_without_native_attr_init() {
  let mut rwlock = new_rwlock();
  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let attr = pthread_rwlockattr_t {
    __size: [
      0xFF_u8, 0x7A_u8, 0x13_u8, 0xC4_u8, 0x02_u8, 0x00_u8, 0x00_u8, 0x00_u8,
    ],
  };
  // SAFETY: `rwlock_ptr` is valid and this implementation accepts non-null
  // attr bytes as compatibility/default attributes without requiring native
  // attr object initialization.
  let init_result = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::from_ref(&attr)) };

  assert_eq!(init_result, 0);
  // SAFETY: lock was initialized successfully with raw attr bytes.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  let destroy_result = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_result, 0);
}

#[test]
fn pthread_rwlock_init_accepts_process_shared_attr_as_default_attributes() {
  let mut rwlock = new_rwlock();
  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let mut attr = pthread_rwlockattr_t { __size: [0_u8; 8] };
  let attr_ptr = ptr::from_mut(&mut attr);

  // SAFETY: `attr_ptr` points to writable storage for native rwlock attrs.
  let attr_init = unsafe { pthread_rwlockattr_init(attr_ptr) };

  assert_eq!(attr_init, 0);
  // SAFETY: `attr_ptr` has been initialized by native pthread API.
  let set_pshared_result =
    unsafe { pthread_rwlockattr_setpshared(attr_ptr, PTHREAD_PROCESS_SHARED) };

  assert_eq!(set_pshared_result, 0);
  // SAFETY: `rwlock_ptr` is valid and attr bytes are accepted as default attrs.
  let init_result = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::from_ref(&attr)) };

  assert_eq!(init_result, 0);
  // SAFETY: initialization succeeded, so destroy must succeed.
  let destroy_result = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_result, 0);
  // SAFETY: native attr object was initialized above and must be destroyed once.
  let attr_destroy = unsafe { pthread_rwlockattr_destroy(attr_ptr) };

  assert_eq!(attr_destroy, 0);
}

#[test]
fn pthread_rwlock_init_accepts_invalid_pshared_attr_payload_as_default_attributes() {
  let mut rwlock = new_rwlock();
  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let mut attr = pthread_rwlockattr_t { __size: [0_u8; 8] };
  let attr_ptr = ptr::from_mut(&mut attr);

  // SAFETY: `attr_ptr` points to writable storage for native rwlock attrs.
  let attr_init = unsafe { pthread_rwlockattr_init(attr_ptr) };

  assert_eq!(attr_init, 0);
  // SAFETY: test-only mutation of raw attr bytes to inject an invalid
  // process-shared selector value (`2`) in the pshared word.
  unsafe {
    attr.__size[4] = 2;
    attr.__size[5] = 0;
    attr.__size[6] = 0;
    attr.__size[7] = 0;
  }
  // SAFETY: `rwlock_ptr` is valid and attr payload bytes are accepted as default attrs.
  let init_result = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::from_ref(&attr)) };

  assert_eq!(init_result, 0);
  // SAFETY: initialization succeeded, so destroy must succeed.
  let destroy_result = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_result, 0);
  // SAFETY: native attr object was initialized above and must be destroyed once.
  let attr_destroy = unsafe { pthread_rwlockattr_destroy(attr_ptr) };

  assert_eq!(attr_destroy, 0);
}

#[test]
fn pthread_rwlock_reinit_with_non_null_attr_still_returns_ebusy() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let mut attr = pthread_rwlockattr_t { __size: [0_u8; 8] };
  let attr_ptr = ptr::from_mut(&mut attr);
  // SAFETY: `attr_ptr` points to writable storage for native rwlock attrs.
  let attr_init = unsafe { pthread_rwlockattr_init(attr_ptr) };

  assert_eq!(attr_init, 0);
  // SAFETY: this path intentionally sets process-shared attr bytes, which are
  // accepted as default attrs by this implementation.
  let set_pshared_result =
    unsafe { pthread_rwlockattr_setpshared(attr_ptr, PTHREAD_PROCESS_SHARED) };

  assert_eq!(set_pshared_result, 0);
  // SAFETY: `rwlock_ptr` already refers to initialized lock storage.
  let second_init = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::from_ref(&attr)) };

  assert_eq!(second_init, EBUSY);
  // SAFETY: native attr object was initialized above and must be destroyed once.
  let attr_destroy = unsafe { pthread_rwlockattr_destroy(attr_ptr) };

  assert_eq!(attr_destroy, 0);
  // SAFETY: lock remains initialized from the first init.
  let destroy_result = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_result, 0);
}

#[test]
fn pthread_rwlock_can_reinitialize_with_non_null_attr_after_destroy() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  // SAFETY: lock was initialized by `init_rwlock`.
  let first_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(first_destroy, 0);

  let mut attr = pthread_rwlockattr_t { __size: [0_u8; 8] };
  let attr_ptr = ptr::from_mut(&mut attr);
  // SAFETY: `attr_ptr` points to writable storage for native rwlock attrs.
  let attr_init = unsafe { pthread_rwlockattr_init(attr_ptr) };

  assert_eq!(attr_init, 0);
  // SAFETY: this path intentionally sets process-shared attr bytes, which are
  // accepted as default attrs by this implementation.
  let set_pshared_result =
    unsafe { pthread_rwlockattr_setpshared(attr_ptr, PTHREAD_PROCESS_SHARED) };

  assert_eq!(set_pshared_result, 0);
  // SAFETY: lock storage was destroyed and can be reinitialized.
  let reinit_result = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::from_ref(&attr)) };

  assert_eq!(reinit_result, 0);
  // SAFETY: reinitialization above succeeded.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: current thread holds a read lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock remains initialized after unlock.
  let second_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(second_destroy, 0);
  // SAFETY: native attr object was initialized above and must be destroyed once.
  let attr_destroy = unsafe { pthread_rwlockattr_destroy(attr_ptr) };

  assert_eq!(attr_destroy, 0);
}

#[test]
fn pthread_rwlock_can_reinitialize_with_invalid_attr_payload_after_destroy() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  // SAFETY: lock was initialized by `init_rwlock`.
  let first_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(first_destroy, 0);

  let mut attr = pthread_rwlockattr_t { __size: [0_u8; 8] };
  let attr_ptr = ptr::from_mut(&mut attr);
  // SAFETY: `attr_ptr` points to writable storage for native rwlock attrs.
  let attr_init = unsafe { pthread_rwlockattr_init(attr_ptr) };

  assert_eq!(attr_init, 0);
  // SAFETY: test-only mutation of raw attr bytes to inject an invalid
  // process-shared selector value (`2`) in the pshared word.
  unsafe {
    attr.__size[4] = 2;
    attr.__size[5] = 0;
    attr.__size[6] = 0;
    attr.__size[7] = 0;
  }
  // SAFETY: lock storage was destroyed and can be reinitialized.
  let reinit_result = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::from_ref(&attr)) };

  assert_eq!(reinit_result, 0);
  // SAFETY: reinitialization above succeeded.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock remains initialized after unlock.
  let second_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(second_destroy, 0);
  // SAFETY: native attr object was initialized above and must be destroyed once.
  let attr_destroy = unsafe { pthread_rwlockattr_destroy(attr_ptr) };

  assert_eq!(attr_destroy, 0);
}

#[test]
fn pthread_rwlock_init_with_process_shared_attr_preserves_rwlock_semantics() {
  let mut rwlock = new_rwlock();
  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let mut attr = pthread_rwlockattr_t { __size: [0_u8; 8] };
  let attr_ptr = ptr::from_mut(&mut attr);
  // SAFETY: `attr_ptr` points to writable storage for native rwlock attrs.
  let attr_init = unsafe { pthread_rwlockattr_init(attr_ptr) };

  assert_eq!(attr_init, 0);
  // SAFETY: `attr_ptr` has been initialized and accepts pshared writes.
  let set_pshared_result =
    unsafe { pthread_rwlockattr_setpshared(attr_ptr, PTHREAD_PROCESS_SHARED) };

  assert_eq!(set_pshared_result, 0);
  // SAFETY: `rwlock_ptr` is valid and this implementation accepts non-null attrs.
  let init_result = unsafe { pthread_rwlock_init(rwlock_ptr, ptr::from_ref(&attr)) };

  assert_eq!(init_result, 0);
  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: while this thread holds a read lock, writer try-lock must be busy.
  assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, EBUSY);
  // SAFETY: current thread holds the read lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: writer ownership blocks read try-lock, including same-thread reentry.
  assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, EBUSY);
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
  // SAFETY: native attr object was initialized above and must be destroyed once.
  let attr_destroy = unsafe { pthread_rwlockattr_destroy(attr_ptr) };

  assert_eq!(attr_destroy, 0);
}

#[test]
fn pthread_rwlock_operations_reject_null_pointer() {
  let mut attr = pthread_rwlockattr_t { __size: [0_u8; 8] };
  let attr_ptr = ptr::from_mut(&mut attr);
  // SAFETY: `attr_ptr` points to writable storage for native rwlock attrs.
  let attr_init = unsafe { pthread_rwlockattr_init(attr_ptr) };

  assert_eq!(attr_init, 0);
  // SAFETY: `attr_ptr` has been initialized and accepts pshared writes.
  let set_pshared_result =
    unsafe { pthread_rwlockattr_setpshared(attr_ptr, PTHREAD_PROCESS_SHARED) };

  assert_eq!(set_pshared_result, 0);
  // SAFETY: null pointer is intentional for contract validation.
  let init_result = unsafe { pthread_rwlock_init(ptr::null_mut(), ptr::null()) };
  // SAFETY: null rwlock is intentional; even with non-null attrs, null pointer must be rejected.
  let init_with_shared_attr_result = unsafe { pthread_rwlock_init(ptr::null_mut(), attr_ptr) };
  // SAFETY: null pointer is intentional for contract validation.
  let destroy_result = unsafe { pthread_rwlock_destroy(ptr::null_mut()) };
  // SAFETY: null pointer is intentional for contract validation.
  let rdlock_result = unsafe { pthread_rwlock_rdlock(ptr::null_mut()) };
  // SAFETY: null pointer is intentional for contract validation.
  let wrlock_result = unsafe { pthread_rwlock_wrlock(ptr::null_mut()) };
  // SAFETY: null pointer is intentional for contract validation.
  let tryrdlock_result = unsafe { pthread_rwlock_tryrdlock(ptr::null_mut()) };
  // SAFETY: null pointer is intentional for contract validation.
  let trywrlock_result = unsafe { pthread_rwlock_trywrlock(ptr::null_mut()) };
  // SAFETY: null pointer is intentional for contract validation.
  let unlock_result = unsafe { pthread_rwlock_unlock(ptr::null_mut()) };

  assert_eq!(init_result, EINVAL);
  assert_eq!(init_with_shared_attr_result, EINVAL);
  assert_eq!(destroy_result, EINVAL);
  assert_eq!(rdlock_result, EINVAL);
  assert_eq!(wrlock_result, EINVAL);
  assert_eq!(tryrdlock_result, EINVAL);
  assert_eq!(trywrlock_result, EINVAL);
  assert_eq!(unlock_result, EINVAL);
  // SAFETY: native attr object was initialized above and must be destroyed once.
  let attr_destroy = unsafe { pthread_rwlockattr_destroy(attr_ptr) };

  assert_eq!(attr_destroy, 0);
}

#[test]
fn pthread_rwlock_operations_after_destroy_return_einval() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  // SAFETY: lock was initialized by `init_rwlock`.
  let destroy_result = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_result, 0);
  // SAFETY: lock storage has been destroyed and is intentionally used for contract checks.
  let rdlock_result = unsafe { pthread_rwlock_rdlock(rwlock_ptr) };
  // SAFETY: lock storage has been destroyed and is intentionally used for contract checks.
  let wrlock_result = unsafe { pthread_rwlock_wrlock(rwlock_ptr) };
  // SAFETY: lock storage has been destroyed and is intentionally used for contract checks.
  let tryrdlock_result = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };
  // SAFETY: lock storage has been destroyed and is intentionally used for contract checks.
  let trywrlock_result = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };
  // SAFETY: lock storage has been destroyed and is intentionally used for contract checks.
  let unlock_result = unsafe { pthread_rwlock_unlock(rwlock_ptr) };

  assert_eq!(rdlock_result, EINVAL);
  assert_eq!(wrlock_result, EINVAL);
  assert_eq!(tryrdlock_result, EINVAL);
  assert_eq!(trywrlock_result, EINVAL);
  assert_eq!(unlock_result, EINVAL);
}

#[test]
fn pthread_rwlock_allows_two_concurrent_readers() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_addr = rwlock_addr(&mut rwlock);
  let entered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let acquired = Arc::new(Barrier::new(3));
  let release = Arc::new(Barrier::new(3));
  let spawn_reader = |entered: Arc<std::sync::atomic::AtomicUsize>,
                      acquired: Arc<Barrier>,
                      release: Arc<Barrier>| {
    thread::spawn(move || {
      let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
      // SAFETY: pointer is derived from a live lock object for the duration of this test.
      assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
      entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
      acquired.wait();
      release.wait();
      // SAFETY: current thread holds a read lock on `rwlock_ptr`.
      assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
    })
  };
  let reader_a = spawn_reader(
    Arc::clone(&entered),
    Arc::clone(&acquired),
    Arc::clone(&release),
  );
  let reader_b = spawn_reader(
    Arc::clone(&entered),
    Arc::clone(&acquired),
    Arc::clone(&release),
  );

  acquired.wait();
  assert_eq!(entered.load(std::sync::atomic::Ordering::SeqCst), 2);
  release.wait();

  reader_a.join().expect("reader A panicked");
  reader_b.join().expect("reader B panicked");

  // SAFETY: lock is initialized and no thread holds it anymore.
  assert_eq!(
    unsafe { pthread_rwlock_destroy(ptr::from_mut(&mut rwlock)) },
    0
  );
}

#[test]
fn pthread_rwlock_trywrlock_returns_ebusy_while_reader_holds_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  let try_result = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(try_result, EBUSY);
  // SAFETY: current thread holds a read lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_remains_ebusy_until_recursive_reader_depth_is_released() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: recursive reader acquisition by the same thread is supported.
  assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, 0);
  // SAFETY: writer acquisition while current thread still holds read ownership must be busy.
  let first_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(first_try, EBUSY);
  // SAFETY: releases one reader depth level.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: one recursive reader depth is still held by this thread.
  let second_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(second_try, EBUSY);
  // SAFETY: releases the final read ownership depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no readers remain, writer acquisition should now succeed.
  let third_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(third_try, 0);
  // SAFETY: current thread holds the write lock acquired by `third_try`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_remains_ebusy_until_blocking_recursive_reader_depth_is_released() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: recursive reader acquisition by the same thread is supported.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: writer acquisition while current thread still holds read ownership must be busy.
  let first_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(first_try, EBUSY);
  // SAFETY: releases one reader depth level.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: one recursive reader depth is still held by this thread.
  let second_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(second_try, EBUSY);
  // SAFETY: releases the final read ownership depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no readers remain, writer acquisition should now succeed.
  let third_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(third_try, 0);
  // SAFETY: current thread holds the write lock acquired by `third_try`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_returns_ebusy_while_other_thread_writer_holds_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let writer_ready = Arc::new(Barrier::new(2));
  let writer_release = Arc::new(Barrier::new(2));
  let ready_for_writer = Arc::clone(&writer_ready);
  let release_for_writer = Arc::clone(&writer_release);
  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    ready_for_writer.wait();
    release_for_writer.wait();
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  writer_ready.wait();
  // SAFETY: lock is initialized and currently held as writer by another thread.
  let try_result = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(try_result, EBUSY);
  writer_release.wait();
  writer.join().expect("writer thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_succeeds_after_other_thread_writer_releases() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let writer_ready = Arc::new(Barrier::new(2));
  let writer_release = Arc::new(Barrier::new(2));
  let ready_for_writer = Arc::clone(&writer_ready);
  let release_for_writer = Arc::clone(&writer_release);
  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    ready_for_writer.wait();
    release_for_writer.wait();
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  writer_ready.wait();
  // SAFETY: lock is initialized and currently held as writer by another thread.
  let first_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(first_try, EBUSY);
  writer_release.wait();
  writer.join().expect("writer thread panicked");
  // SAFETY: writer released lock; trywrlock should now acquire.
  let second_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(second_try, 0);
  // SAFETY: current thread holds the write lock acquired by `second_try`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_remains_ebusy_until_other_thread_writer_releases() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let (ready_tx, ready_rx) = mpsc::channel::<()>();
  let (release_tx, release_rx) = mpsc::channel::<()>();
  let (done_tx, done_rx) = mpsc::channel::<()>();
  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    ready_tx.send(()).expect("failed to signal writer ready");
    release_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("failed to receive writer release signal");
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
    done_tx.send(()).expect("failed to signal writer done");
  });

  ready_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("writer failed to acquire write lock");
  // SAFETY: lock is initialized and currently held as writer by another thread.
  let first_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(first_try, EBUSY);
  // SAFETY: writer still owns lock, so try-lock remains busy.
  let second_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(second_try, EBUSY);
  release_tx.send(()).expect("failed to release writer");
  done_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("writer failed to release write lock");
  // SAFETY: writer released lock; writer try-lock should now succeed.
  let third_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(third_try, 0);
  // SAFETY: current thread holds the write lock acquired by `third_try`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  writer.join().expect("writer thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_returns_ebusy_while_other_thread_reader_holds_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let reader_ready = Arc::new(Barrier::new(2));
  let reader_release = Arc::new(Barrier::new(2));
  let ready_for_reader = Arc::clone(&reader_ready);
  let release_for_reader = Arc::clone(&reader_release);
  let reader = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
    ready_for_reader.wait();
    release_for_reader.wait();
    // SAFETY: current thread holds the read lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  reader_ready.wait();
  // SAFETY: lock is initialized and currently held as reader by another thread.
  let try_result = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(try_result, EBUSY);
  reader_release.wait();
  reader.join().expect("reader thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_succeeds_after_other_thread_reader_releases() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let reader_ready = Arc::new(Barrier::new(2));
  let reader_release = Arc::new(Barrier::new(2));
  let ready_for_reader = Arc::clone(&reader_ready);
  let release_for_reader = Arc::clone(&reader_release);
  let reader = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
    ready_for_reader.wait();
    release_for_reader.wait();
    // SAFETY: current thread holds the read lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  reader_ready.wait();
  // SAFETY: lock is initialized and currently held as reader by another thread.
  let first_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(first_try, EBUSY);
  reader_release.wait();
  reader.join().expect("reader thread panicked");
  // SAFETY: reader released lock; trywrlock should now acquire.
  let second_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(second_try, 0);
  // SAFETY: current thread holds the write lock acquired by `second_try`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_remains_ebusy_until_all_other_readers_release() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let (ready1_tx, ready1_rx) = mpsc::channel::<()>();
  let (ready2_tx, ready2_rx) = mpsc::channel::<()>();
  let (release1_tx, release1_rx) = mpsc::channel::<()>();
  let (release2_tx, release2_rx) = mpsc::channel::<()>();
  let (done1_tx, done1_rx) = mpsc::channel::<()>();
  let (done2_tx, done2_rx) = mpsc::channel::<()>();
  let reader1 = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
    ready1_tx.send(()).expect("failed to signal reader1 ready");
    release1_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("failed to receive reader1 release signal");
    // SAFETY: current thread holds the read lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
    done1_tx.send(()).expect("failed to signal reader1 done");
  });
  let reader2 = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
    ready2_tx.send(()).expect("failed to signal reader2 ready");
    release2_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("failed to receive reader2 release signal");
    // SAFETY: current thread holds the read lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
    done2_tx.send(()).expect("failed to signal reader2 done");
  });

  ready1_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("reader1 failed to acquire read lock");
  ready2_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("reader2 failed to acquire read lock");
  // SAFETY: lock is initialized and held by readers in other threads.
  let first_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(first_try, EBUSY);
  release1_tx.send(()).expect("failed to release reader1");
  done1_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("reader1 failed to release read lock");
  // SAFETY: one reader still holds lock, so writer acquisition remains busy.
  let second_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(second_try, EBUSY);
  release2_tx.send(()).expect("failed to release reader2");
  done2_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("reader2 failed to release read lock");
  // SAFETY: all readers released lock; writer try-lock should now succeed.
  let third_try = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(third_try, 0);
  // SAFETY: current thread holds the write lock acquired by `third_try`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  reader1.join().expect("reader1 thread panicked");
  reader2.join().expect("reader2 thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_tryrdlock_returns_ebusy_while_writer_holds_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let (result_tx, result_rx) = mpsc::channel::<i32>();

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);

  let try_reader = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    let try_result = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

    result_tx
      .send(try_result)
      .expect("failed to send tryrdlock result");
  });
  let try_result = result_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("failed to receive tryrdlock result");

  assert_eq!(try_result, EBUSY);
  try_reader.join().expect("try-reader thread panicked");
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_tryrdlock_returns_ebusy_while_other_thread_writer_holds_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let writer_ready = Arc::new(Barrier::new(2));
  let writer_release = Arc::new(Barrier::new(2));
  let ready_for_writer = Arc::clone(&writer_ready);
  let release_for_writer = Arc::clone(&writer_release);
  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    ready_for_writer.wait();
    release_for_writer.wait();
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  writer_ready.wait();
  // SAFETY: lock is initialized and currently held as writer by another thread.
  let try_result = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

  assert_eq!(try_result, EBUSY);
  writer_release.wait();
  writer.join().expect("writer thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_tryrdlock_succeeds_after_other_thread_writer_releases() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let writer_ready = Arc::new(Barrier::new(2));
  let writer_release = Arc::new(Barrier::new(2));
  let ready_for_writer = Arc::clone(&writer_ready);
  let release_for_writer = Arc::clone(&writer_release);
  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    ready_for_writer.wait();
    release_for_writer.wait();
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  writer_ready.wait();
  // SAFETY: lock is initialized and currently held as writer by another thread.
  let first_try = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

  assert_eq!(first_try, EBUSY);
  writer_release.wait();
  writer.join().expect("writer thread panicked");
  // SAFETY: writer released lock; tryrdlock should now acquire.
  let second_try = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

  assert_eq!(second_try, 0);
  // SAFETY: current thread holds the read lock acquired by `second_try`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_tryrdlock_remains_ebusy_until_other_thread_writer_releases() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let (ready_tx, ready_rx) = mpsc::channel::<()>();
  let (release_tx, release_rx) = mpsc::channel::<()>();
  let (done_tx, done_rx) = mpsc::channel::<()>();
  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    ready_tx.send(()).expect("failed to signal writer ready");
    release_rx
      .recv_timeout(Duration::from_secs(1))
      .expect("failed to receive writer release signal");
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
    done_tx.send(()).expect("failed to signal writer done");
  });

  ready_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("writer failed to acquire write lock");
  // SAFETY: lock is initialized and currently held as writer by another thread.
  let first_try = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

  assert_eq!(first_try, EBUSY);
  // SAFETY: writer still owns lock, so reader try-lock remains busy.
  let second_try = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

  assert_eq!(second_try, EBUSY);
  release_tx.send(()).expect("failed to release writer");
  done_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("writer failed to release write lock");
  // SAFETY: writer released lock; reader try-lock should now succeed.
  let third_try = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

  assert_eq!(third_try, 0);
  // SAFETY: current thread holds the read lock acquired by `third_try`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  writer.join().expect("writer thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_tryrdlock_succeeds_while_other_thread_reader_holds_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let reader_ready = Arc::new(Barrier::new(2));
  let reader_release = Arc::new(Barrier::new(2));
  let ready_for_reader = Arc::clone(&reader_ready);
  let release_for_reader = Arc::clone(&reader_release);
  let reader = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
    ready_for_reader.wait();
    release_for_reader.wait();
    // SAFETY: current thread holds a read lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  reader_ready.wait();
  // SAFETY: lock is initialized and held by another reader; shared reader acquire is allowed.
  let try_result = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

  assert_eq!(try_result, 0);
  // SAFETY: current thread acquired a read lock via `pthread_rwlock_tryrdlock`.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  reader_release.wait();
  reader.join().expect("reader thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_recursive_tryrdlock_requires_matching_unlock_count() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: recursive reader acquisition by the same thread is supported.
  assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, 0);
  // SAFETY: release one recursive reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: release the final recursive reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no ownership remains for current thread; extra unlock must fail.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, EINVAL);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_repeated_failed_trywrlock_while_reader_held_preserves_reader_depth() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: recursive reader acquisition by the same thread is supported.
  assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, 0);

  for _ in 0..3 {
    // SAFETY: writer try-reentry while current thread holds read ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, EBUSY);
  }

  // SAFETY: failed writer try-reentries must not change reader depth accounting.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: release the final reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no ownership remains; extra unlock must fail.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, EINVAL);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_failed_trywrlock_while_reader_held_allows_writer_after_release() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: recursive reader acquisition by the same thread is supported.
  assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, 0);

  for _ in 0..3 {
    // SAFETY: writer try-reentry while current thread holds read ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, EBUSY);
  }

  // SAFETY: release one recursive reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: release the final reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: after reader release, writer try-lock should succeed.
  assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, 0);
  // SAFETY: current thread now holds writer ownership.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_failed_trywrlock_while_reader_held_still_allows_reader_reacquire() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);

  for _ in 0..3 {
    // SAFETY: writer try-reentry while current thread holds read ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, EBUSY);
  }

  // SAFETY: failed writer try-reentries must not block additional same-thread reader acquire.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: same-thread recursive reader try-acquire remains allowed.
  assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, 0);

  // SAFETY: release one reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: release second reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: release final reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no ownership remains; extra unlock must fail.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, EINVAL);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_tryrdlock_returns_ebusy_for_same_thread_writer_reentry() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  let try_result = unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) };

  assert_eq!(try_result, EBUSY);
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_failed_tryrdlock_reentry_does_not_require_extra_unlock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: same-thread reader try-reentry under writer ownership must fail.
  assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, EBUSY);
  // SAFETY: current thread releases its single writer ownership.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no ownership remains; extra unlock must fail.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, EINVAL);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_trywrlock_returns_ebusy_for_same_thread_writer_reentry() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  let try_result = unsafe { pthread_rwlock_trywrlock(rwlock_ptr) };

  assert_eq!(try_result, EBUSY);
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_failed_trywrlock_reentry_does_not_require_extra_unlock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: same-thread writer try-reentry must fail and must not increase ownership depth.
  assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, EBUSY);
  // SAFETY: current thread releases its single writer ownership.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no ownership remains; extra unlock must fail.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, EINVAL);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_multiple_failed_writer_reentries_preserve_single_ownership_depth() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);

  for _ in 0..3 {
    // SAFETY: reader try-reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, EBUSY);
    // SAFETY: writer try-reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, EBUSY);
    // SAFETY: blocking reader reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, EDEADLK);
    // SAFETY: blocking writer reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, EDEADLK);
  }

  // SAFETY: failed reentries above must not increase writer ownership depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no ownership remains; extra unlock must fail.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, EINVAL);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_failed_writer_reentries_still_allow_subsequent_writer_reacquire() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);

  for _ in 0..3 {
    // SAFETY: reader try-reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, EBUSY);
    // SAFETY: writer try-reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, EBUSY);
    // SAFETY: blocking reader reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, EDEADLK);
    // SAFETY: blocking writer reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, EDEADLK);
  }

  // SAFETY: release original writer ownership.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: failed reentries above must not poison future writer acquisition.
  assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, 0);
  // SAFETY: current thread holds writer ownership acquired above.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_failed_writer_reentries_still_allow_subsequent_reader_reacquire() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);

  for _ in 0..3 {
    // SAFETY: reader try-reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, EBUSY);
    // SAFETY: writer try-reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_trywrlock(rwlock_ptr) }, EBUSY);
    // SAFETY: blocking reader reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, EDEADLK);
    // SAFETY: blocking writer reentry under same-thread writer ownership must fail.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, EDEADLK);
  }

  // SAFETY: release original writer ownership.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: failed reentries above must not poison subsequent reader acquisition.
  assert_eq!(unsafe { pthread_rwlock_tryrdlock(rwlock_ptr) }, 0);
  // SAFETY: recursive reader acquisition remains valid after successful tryrdlock.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: release recursive reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: release final reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_rdlock_returns_edeadlk_for_same_thread_writer_reentry() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  let rdlock_result = unsafe { pthread_rwlock_rdlock(rwlock_ptr) };

  assert_eq!(rdlock_result, EDEADLK);
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_wrlock_returns_edeadlk_for_same_thread_writer_reentry() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  let wrlock_result = unsafe { pthread_rwlock_wrlock(rwlock_ptr) };

  assert_eq!(wrlock_result, EDEADLK);
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_failed_wrlock_reentry_does_not_require_extra_unlock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: same-thread writer reentry must fail and must not increase ownership depth.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, EDEADLK);
  // SAFETY: current thread releases its single writer ownership.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: no ownership remains; extra unlock must fail.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, EINVAL);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_destroy_returns_ebusy_when_lock_is_held() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized, but held by this thread as reader.
  let destroy_while_held = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_while_held, EBUSY);
  // SAFETY: current thread holds a read lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_destroy_remains_ebusy_until_recursive_reader_depth_is_fully_released() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: recursive reader acquisition by the same thread is supported.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized, but reader ownership is still held.
  let first_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(first_destroy, EBUSY);
  // SAFETY: current thread releases one recursive read depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: one reader depth is still held by this thread.
  let second_destroy = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(second_destroy, EBUSY);
  // SAFETY: current thread releases the final reader depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and no longer held.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_destroy_returns_ebusy_when_write_lock_is_held() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized, but held by this thread as writer.
  let destroy_while_held = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_while_held, EBUSY);
  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_destroy_returns_ebusy_while_other_thread_holds_write_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let writer_ready = Arc::new(Barrier::new(2));
  let writer_release = Arc::new(Barrier::new(2));
  let ready_for_writer = Arc::clone(&writer_ready);
  let release_for_writer = Arc::clone(&writer_release);
  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    ready_for_writer.wait();
    release_for_writer.wait();
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  writer_ready.wait();
  // SAFETY: lock is initialized, but held by another thread as writer.
  let destroy_while_held = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_while_held, EBUSY);
  writer_release.wait();
  writer.join().expect("writer thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_destroy_returns_ebusy_while_other_thread_holds_read_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let reader_ready = Arc::new(Barrier::new(2));
  let reader_release = Arc::new(Barrier::new(2));
  let ready_for_reader = Arc::clone(&reader_ready);
  let release_for_reader = Arc::clone(&reader_release);
  let reader = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
    ready_for_reader.wait();
    release_for_reader.wait();
    // SAFETY: current thread holds a read lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  reader_ready.wait();
  // SAFETY: lock is initialized, but held by another thread as reader.
  let destroy_while_held = unsafe { pthread_rwlock_destroy(rwlock_ptr) };

  assert_eq!(destroy_while_held, EBUSY);
  reader_release.wait();
  reader.join().expect("reader thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_wrlock_waits_until_readers_unlock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let start_writer = Arc::new(Barrier::new(2));
  let (acquired_tx, acquired_rx) = mpsc::channel::<()>();
  let writer_start = Arc::clone(&start_writer);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);

  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;

    writer_start.wait();
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    acquired_tx
      .send(())
      .expect("failed to signal writer acquisition");
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  start_writer.wait();
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_millis(100)),
    Err(mpsc::RecvTimeoutError::Timeout),
    "writer must stay blocked while reader lock is held",
  );

  // SAFETY: current thread holds the read lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_secs(1)),
    Ok(()),
    "writer must acquire lock after reader unlocks",
  );
  writer.join().expect("writer thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_wrlock_waits_until_recursive_reader_depth_is_fully_released() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let start_writer = Arc::new(Barrier::new(2));
  let (acquired_tx, acquired_rx) = mpsc::channel::<()>();
  let writer_start = Arc::clone(&start_writer);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
  // SAFETY: recursive reader acquisition by the same thread is supported.
  assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);

  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;

    writer_start.wait();
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    acquired_tx
      .send(())
      .expect("failed to signal recursive-depth writer acquisition");
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  start_writer.wait();
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_millis(100)),
    Err(mpsc::RecvTimeoutError::Timeout),
    "writer must stay blocked while recursive reader depth is held",
  );

  // SAFETY: current thread releases one recursive read depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_millis(100)),
    Err(mpsc::RecvTimeoutError::Timeout),
    "writer must stay blocked until final recursive read depth is released",
  );

  // SAFETY: current thread releases the final read depth.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_secs(1)),
    Ok(()),
    "writer must acquire lock after final recursive reader unlock",
  );

  writer.join().expect("writer thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_rdlock_waits_until_writer_unlocks() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let start_reader = Arc::new(Barrier::new(2));
  let (acquired_tx, acquired_rx) = mpsc::channel::<()>();
  let reader_start = Arc::clone(&start_reader);

  // SAFETY: `rwlock_ptr` points to initialized lock storage.
  assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);

  let reader = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;

    reader_start.wait();
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
    acquired_tx
      .send(())
      .expect("failed to signal reader acquisition");
    // SAFETY: current thread holds a read lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  start_reader.wait();
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_millis(100)),
    Err(mpsc::RecvTimeoutError::Timeout),
    "reader must stay blocked while writer lock is held",
  );

  // SAFETY: current thread holds the write lock.
  assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  assert_eq!(
    acquired_rx.recv_timeout(Duration::from_secs(1)),
    Ok(()),
    "reader must acquire lock after writer unlocks",
  );
  reader.join().expect("reader thread panicked");
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_unlock_without_ownership_returns_einval() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  // SAFETY: lock is initialized but not currently held by this thread.
  let unlock_result = unsafe { pthread_rwlock_unlock(rwlock_ptr) };

  assert_eq!(unlock_result, EINVAL);
  // SAFETY: lock is initialized and unlocked.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_unlock_returns_einval_for_non_owner_while_writer_holds_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let writer_ready = Arc::new(Barrier::new(2));
  let writer_release = Arc::new(Barrier::new(2));
  let ready_for_writer = Arc::clone(&writer_ready);
  let release_for_writer = Arc::clone(&writer_release);
  let writer = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_wrlock(rwlock_ptr) }, 0);
    ready_for_writer.wait();
    release_for_writer.wait();
    // SAFETY: current thread holds the write lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  writer_ready.wait();
  // SAFETY: this thread does not own `rwlock_ptr`; another thread holds writer ownership.
  let unlock_result = unsafe { pthread_rwlock_unlock(rwlock_ptr) };

  assert_eq!(unlock_result, EINVAL);
  writer_release.wait();
  writer.join().expect("writer thread panicked");
  // SAFETY: lock is initialized and unlocked after writer exit.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}

#[test]
fn pthread_rwlock_unlock_returns_einval_for_non_owner_while_reader_holds_lock() {
  let mut rwlock = new_rwlock();

  init_rwlock(&mut rwlock);

  let rwlock_ptr = ptr::from_mut(&mut rwlock);
  let rwlock_addr = rwlock_ptr.addr();
  let reader_ready = Arc::new(Barrier::new(2));
  let reader_release = Arc::new(Barrier::new(2));
  let ready_for_reader = Arc::clone(&reader_ready);
  let release_for_reader = Arc::clone(&reader_release);
  let reader = thread::spawn(move || {
    let rwlock_ptr = rwlock_addr as *mut pthread_rwlock_t;
    // SAFETY: pointer is derived from a live lock object for the duration of this test.
    assert_eq!(unsafe { pthread_rwlock_rdlock(rwlock_ptr) }, 0);
    ready_for_reader.wait();
    release_for_reader.wait();
    // SAFETY: current thread holds the read lock.
    assert_eq!(unsafe { pthread_rwlock_unlock(rwlock_ptr) }, 0);
  });

  reader_ready.wait();
  // SAFETY: this thread does not own `rwlock_ptr`; another thread holds read ownership.
  let unlock_result = unsafe { pthread_rwlock_unlock(rwlock_ptr) };

  assert_eq!(unlock_result, EINVAL);
  reader_release.wait();
  reader.join().expect("reader thread panicked");
  // SAFETY: lock is initialized and unlocked after reader exit.
  assert_eq!(unsafe { pthread_rwlock_destroy(rwlock_ptr) }, 0);
}
