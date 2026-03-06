#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use rlibc::abi::errno::EBUSY;
use rlibc::abi::types::c_int;
use rlibc::stdio::{FILE, flockfile, ftrylockfile, funlockfile};
use std::sync::{Mutex, MutexGuard, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

unsafe extern "C" {
  fn fclose(stream: *mut FILE) -> c_int;
  fn fflush(stream: *mut FILE) -> c_int;
  fn tmpfile() -> *mut FILE;
}

fn test_lock() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  match LOCK.get_or_init(|| Mutex::new(())).lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn open_tmpfile(context: &str) -> *mut FILE {
  // SAFETY: host libc returns either a valid temporary stream or null.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null(), "{context}");

  stream
}

fn close_tmpfile(stream: *mut FILE, context: &str) {
  // SAFETY: `stream` came from `tmpfile` and remains open here.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0, "{context}");
}

#[test]
fn recursive_same_thread_trylock_keeps_shared_file_busy_until_final_unlock() {
  let _guard = test_lock();
  let stream = open_tmpfile("tmpfile must provide a shared FILE* for recursive locking");
  let stream_addr = stream.addr();

  // SAFETY: `stream` is a valid shared FILE* from `tmpfile`.
  let first_lock_status = unsafe { ftrylockfile(stream) };
  // SAFETY: same-thread recursive file locking must succeed on the same FILE*.
  let second_lock_status = unsafe { ftrylockfile(stream) };

  assert_eq!(first_lock_status, 0);
  assert_eq!(second_lock_status, 0);

  // SAFETY: the current thread owns one recursive level of the FILE lock.
  unsafe {
    funlockfile(stream);
  }

  let (busy_tx, busy_rx) = mpsc::channel();
  let busy_handle = thread::spawn(move || {
    let shared_stream = stream_addr as *mut FILE;
    // SAFETY: `shared_stream` is the same FILE* shared with the worker thread.
    let status = unsafe { ftrylockfile(shared_stream) };

    if status == 0 {
      // SAFETY: this worker unexpectedly acquired the shared FILE* lock above.
      unsafe {
        funlockfile(shared_stream);
      }
    }

    busy_tx
      .send(status)
      .expect("busy try-lock status must reach the test thread");
  });
  let busy_status = busy_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("other thread must observe the shared FILE* as still locked");

  assert_eq!(busy_status, EBUSY);

  busy_handle
    .join()
    .expect("busy try-lock worker thread must complete successfully");

  // SAFETY: the current thread still owns the final recursive level.
  unsafe {
    funlockfile(stream);
  }

  let (acquired_tx, acquired_rx) = mpsc::channel();
  let acquired_handle = thread::spawn(move || {
    let shared_stream = stream_addr as *mut FILE;
    // SAFETY: the final unlock has released the same shared FILE*.
    let status = unsafe { ftrylockfile(shared_stream) };

    if status == 0 {
      // SAFETY: this worker thread acquired the shared FILE* lock above.
      unsafe {
        funlockfile(shared_stream);
      }
    }

    acquired_tx
      .send(status)
      .expect("post-release try-lock status must reach the test thread");
  });
  let acquired_status = acquired_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("other thread must acquire the shared FILE* after the final unlock");

  assert_eq!(acquired_status, 0);

  acquired_handle
    .join()
    .expect("post-release try-lock worker thread must complete successfully");

  close_tmpfile(
    stream,
    "closing the shared FILE* after recursive locking must succeed",
  );
}

#[test]
fn trylock_on_shared_file_returns_ebusy_without_waiting_for_release() {
  let _guard = test_lock();
  let stream = open_tmpfile("tmpfile must provide a shared FILE* for try-lock timing");
  let stream_addr = stream.addr();

  // SAFETY: `stream` is a valid shared FILE* from `tmpfile`.
  unsafe {
    flockfile(stream);
  }

  let (status_tx, status_rx) = mpsc::channel();
  let handle = thread::spawn(move || {
    let shared_stream = stream_addr as *mut FILE;
    // SAFETY: `shared_stream` is a valid FILE* shared with the worker thread.
    let status = unsafe { ftrylockfile(shared_stream) };

    if status == 0 {
      // SAFETY: this worker unexpectedly acquired the shared FILE* lock above.
      unsafe {
        funlockfile(shared_stream);
      }
    }

    status_tx
      .send(status)
      .expect("worker try-lock status must reach the test thread");
  });
  let status = status_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("ftrylockfile must return immediately while another thread owns the FILE*");

  assert_eq!(status, EBUSY);

  // SAFETY: the current thread owns the FILE* lock acquired above.
  unsafe {
    funlockfile(stream);
  }

  handle
    .join()
    .expect("non-blocking try-lock worker thread must complete successfully");

  close_tmpfile(
    stream,
    "closing the shared FILE* after non-blocking try-lock must succeed",
  );
}

#[test]
fn blocking_flockfile_stays_blocked_through_fflush_until_final_recursive_unlock() {
  let _guard = test_lock();
  let stream = open_tmpfile("tmpfile must provide a shared FILE* for blocking flockfile");
  let stream_addr = stream.addr();

  // SAFETY: `stream` is a valid shared FILE* from `tmpfile`.
  let first_lock_status = unsafe { ftrylockfile(stream) };
  // SAFETY: same-thread recursive file locking must succeed on the same FILE*.
  let second_lock_status = unsafe { ftrylockfile(stream) };

  assert_eq!(first_lock_status, 0);
  assert_eq!(second_lock_status, 0);

  let (ready_tx, ready_rx) = mpsc::channel();
  let (acquired_tx, acquired_rx) = mpsc::channel();
  let handle = thread::spawn(move || {
    let shared_stream = stream_addr as *mut FILE;

    ready_tx
      .send(())
      .expect("worker readiness signal must reach the test thread");

    // SAFETY: `shared_stream` is the same FILE* shared with the worker thread.
    unsafe {
      flockfile(shared_stream);
    }

    acquired_tx
      .send(())
      .expect("worker acquisition signal must reach the test thread");

    // SAFETY: this worker thread acquired the shared FILE* lock above.
    unsafe {
      funlockfile(shared_stream);
    }
  });

  ready_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("worker thread must start before blocking-lock assertions");

  assert!(
    acquired_rx
      .recv_timeout(Duration::from_millis(100))
      .is_err(),
    "worker flockfile must stay blocked while another thread owns the FILE lock",
  );

  // SAFETY: the current thread owns the same FILE* lock and may flush it.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert!(
    acquired_rx
      .recv_timeout(Duration::from_millis(100))
      .is_err(),
    "fflush must not release or bypass the recursive FILE lock",
  );

  // SAFETY: the current thread owns one recursive level of the FILE lock.
  unsafe {
    funlockfile(stream);
  }

  assert!(
    acquired_rx
      .recv_timeout(Duration::from_millis(100))
      .is_err(),
    "worker flockfile must remain blocked until the final recursive unlock",
  );

  // SAFETY: the current thread still owns the final recursive level.
  unsafe {
    funlockfile(stream);
  }

  acquired_rx
    .recv_timeout(Duration::from_secs(1))
    .expect("worker must acquire the FILE lock after the final recursive unlock");

  handle
    .join()
    .expect("blocking flockfile worker thread must complete successfully");

  close_tmpfile(
    stream,
    "closing the shared FILE* after blocking flockfile and fflush must succeed",
  );
}

#[test]
fn fclose_succeeds_while_the_same_thread_recursively_owns_the_stream_lock() {
  let _guard = test_lock();
  let stream =
    open_tmpfile("tmpfile must provide a shared FILE* for recursive flockfile + fclose coverage");

  // SAFETY: `stream` is a valid shared FILE* from `tmpfile`.
  let first_lock_status = unsafe { ftrylockfile(stream) };
  // SAFETY: same-thread recursive file locking must succeed on the same FILE*.
  let second_lock_status = unsafe { ftrylockfile(stream) };

  assert_eq!(first_lock_status, 0);
  assert_eq!(second_lock_status, 0);

  // SAFETY: the current thread owns the FILE* lock recursively and may close it.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(
    close_status, 0,
    "fclose must succeed even when the same thread still owns recursive FILE locks",
  );
}
