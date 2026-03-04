//! Errno-related definitions.
//!
//! This module provides thread-local errno storage and the exported C ABI
//! accessor used by libc-compatible code.

use crate::abi::types::c_int;
use std::cell::Cell;

thread_local! {
  static ERRNO: Cell<c_int> = const { Cell::new(0) };
}

#[unsafe(no_mangle)]
/// C ABI entry point for `__errno_location`.
///
/// Returns a non-null pointer to calling-thread-local `errno` storage.
///
/// Returns:
/// - non-null pointer to writable calling-thread-local `errno`
/// - the same pointer value on repeated calls within the same live thread
///
/// C contract notes:
/// - Returned pointer is valid only while the calling thread is alive.
/// - Repeated calls on the same thread return the same storage location.
/// - The returned storage is thread-local and must not be shared across
///   threads after thread termination.
#[must_use]
pub extern "C" fn __errno_location() -> *mut c_int {
  ERRNO.with(Cell::as_ptr)
}

/// Writes `value` into the calling thread's `errno` storage.
///
/// This internal helper keeps `errno` writes centralized for C ABI entry
/// points that must report errors.
pub(crate) fn set_errno(value: c_int) {
  let errno_ptr = __errno_location();

  debug_assert!(
    !errno_ptr.is_null(),
    "__errno_location must not return a null pointer",
  );

  // SAFETY: `__errno_location` returns writable thread-local `errno` storage.
  unsafe {
    errno_ptr.write(value);
  }
}

#[cfg(test)]
mod tests {
  use std::ptr::NonNull;
  use std::sync::{Arc, Barrier, mpsc};
  use std::thread;

  use super::{__errno_location, c_int, set_errno};

  fn checked_errno_ptr(errno_ptr: *mut c_int) -> NonNull<c_int> {
    NonNull::new(errno_ptr).expect("__errno_location returned null")
  }

  fn read_errno(errno_ptr: NonNull<c_int>) -> c_int {
    // SAFETY: `checked_errno_ptr` guarantees a non-null pointer and we read a single c_int value.
    unsafe { errno_ptr.as_ptr().read() }
  }

  fn write_errno(errno_ptr: NonNull<c_int>, value: c_int) {
    // SAFETY: `checked_errno_ptr` guarantees a non-null pointer and we write a single c_int value.
    unsafe {
      errno_ptr.as_ptr().write(value);
    }
  }

  #[test]
  fn errno_is_zero_on_first_access() {
    let child = thread::spawn(|| {
      let errno_ptr = checked_errno_ptr(__errno_location());

      read_errno(errno_ptr)
    });
    let initial_value = child.join().expect("child thread panicked");

    assert_eq!(initial_value, 0, "initial errno must be zero");
  }

  #[test]
  fn write_via_errno_location_pointer_is_reflected() {
    let errno_ptr = checked_errno_ptr(__errno_location());

    write_errno(errno_ptr, 27);

    let errno_ptr_again = checked_errno_ptr(__errno_location());

    assert_eq!(
      read_errno(errno_ptr_again),
      27,
      "write through errno pointer must be visible",
    );
  }

  #[test]
  fn errno_location_pointer_is_stable_within_same_thread() {
    let first = checked_errno_ptr(__errno_location());
    let second = checked_errno_ptr(__errno_location());

    assert_eq!(
      first, second,
      "__errno_location must return stable TLS storage on the same thread",
    );
  }

  #[test]
  fn errno_location_pointer_is_distinct_across_live_threads() {
    let main_ptr = checked_errno_ptr(__errno_location());
    let main_addr = main_ptr.as_ptr() as usize;
    let child = thread::spawn(|| {
      let child_ptr = checked_errno_ptr(__errno_location());

      child_ptr.as_ptr() as usize
    });
    let child_addr = child.join().expect("child thread panicked");

    assert_ne!(
      main_addr, child_addr,
      "__errno_location must provide distinct TLS storage addresses across live threads",
    );
    assert_eq!(
      checked_errno_ptr(__errno_location()).as_ptr() as usize,
      main_addr,
      "main thread errno pointer must remain stable after child thread access",
    );
  }

  #[test]
  fn errno_location_pointer_is_distinct_across_multiple_live_threads() {
    let main_addr = checked_errno_ptr(__errno_location()).as_ptr() as usize;
    let barrier = Arc::new(Barrier::new(3));
    let (sender, receiver) = mpsc::channel();
    let mut children = Vec::new();

    for _ in 0..2 {
      let barrier = Arc::clone(&barrier);
      let sender = sender.clone();

      children.push(thread::spawn(move || {
        let child_addr = checked_errno_ptr(__errno_location()).as_ptr() as usize;

        sender.send(child_addr).expect("send child errno pointer");
        barrier.wait();
      }));
    }

    drop(sender);

    let child_addrs = [
      receiver.recv().expect("receive first child errno pointer"),
      receiver.recv().expect("receive second child errno pointer"),
    ];

    barrier.wait();

    for child in children {
      child.join().expect("child thread panicked");
    }

    assert_eq!(child_addrs.len(), 2, "expected two child pointer samples");
    assert_ne!(
      main_addr, child_addrs[0],
      "main and first child must have different TLS errno addresses",
    );
    assert_ne!(
      main_addr, child_addrs[1],
      "main and second child must have different TLS errno addresses",
    );
    assert_ne!(
      child_addrs[0], child_addrs[1],
      "simultaneously live child threads must not share TLS errno addresses",
    );
  }

  #[test]
  fn set_errno_helper_updates_tls_errno() {
    set_errno(64);

    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      64,
      "set_errno helper must write into current thread errno storage",
    );
  }

  #[test]
  fn set_errno_helper_keeps_errno_pointer_stable_within_thread() {
    let before = checked_errno_ptr(__errno_location());

    set_errno(5);

    let after = checked_errno_ptr(__errno_location());

    assert_eq!(
      before, after,
      "set_errno helper must not change calling-thread errno storage identity",
    );
  }

  #[test]
  fn set_errno_helper_overwrites_existing_thread_local_value() {
    write_errno(checked_errno_ptr(__errno_location()), -3);

    set_errno(22);

    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      22,
      "set_errno helper must overwrite current thread-local errno value",
    );
  }

  #[test]
  fn set_errno_helper_preserves_negative_values() {
    set_errno(-91);

    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      -91,
      "set_errno helper must preserve the exact c_int value",
    );
  }

  #[test]
  fn set_errno_helper_preserves_c_int_boundaries() {
    set_errno(c_int::MIN);
    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      c_int::MIN,
      "set_errno helper must preserve c_int::MIN",
    );

    set_errno(c_int::MAX);
    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      c_int::MAX,
      "set_errno helper must preserve c_int::MAX",
    );
  }

  #[test]
  fn set_errno_helper_can_reset_errno_to_zero() {
    set_errno(99);
    set_errno(0);

    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      0,
      "set_errno helper must allow resetting errno back to zero",
    );
  }

  #[test]
  fn set_errno_helper_is_thread_local() {
    set_errno(13);

    let child = thread::spawn(|| {
      let child_initial = read_errno(checked_errno_ptr(__errno_location()));

      set_errno(77);

      let child_after_write = read_errno(checked_errno_ptr(__errno_location()));

      (child_initial, child_after_write)
    });
    let (child_initial, child_after_write) = child.join().expect("child thread panicked");

    assert_eq!(
      child_initial, 0,
      "child thread errno must start at zero before helper writes",
    );
    assert_eq!(
      child_after_write, 77,
      "child thread must keep its own errno value"
    );
    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      13,
      "main thread errno must not be clobbered by child helper writes",
    );
  }

  #[test]
  fn set_errno_helper_multiple_writes_remain_thread_local() {
    set_errno(100);

    let sync = Arc::new(Barrier::new(2));
    let child_sync = Arc::clone(&sync);
    let child = thread::spawn(move || {
      set_errno(-1);
      child_sync.wait();
      set_errno(-2);

      read_errno(checked_errno_ptr(__errno_location()))
    });

    sync.wait();
    set_errno(200);

    let child_final = child.join().expect("child thread panicked");

    assert_eq!(
      child_final, -2,
      "child thread must keep its own latest errno write",
    );
    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      200,
      "main thread must keep its own latest errno write",
    );
  }

  #[test]
  fn errno_is_isolated_between_threads() {
    let main_errno_ptr = checked_errno_ptr(__errno_location());

    write_errno(main_errno_ptr, 11);

    let child = thread::spawn(|| {
      let child_errno_ptr = checked_errno_ptr(__errno_location());
      let child_initial = read_errno(child_errno_ptr);

      write_errno(child_errno_ptr, 99);

      let child_after_write = read_errno(checked_errno_ptr(__errno_location()));

      (child_initial, child_after_write)
    });
    let (child_initial, child_after_write) = child.join().expect("child thread panicked");

    assert_eq!(child_initial, 0, "child thread errno must start at zero");
    assert_eq!(child_after_write, 99, "child thread write must stick");
    assert_eq!(
      read_errno(checked_errno_ptr(__errno_location())),
      11,
      "main thread errno must not be overwritten by child thread",
    );
  }
}
