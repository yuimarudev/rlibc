#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use rlibc::abi::errno::{EBUSY, EINVAL, ENOTSUP, EPERM, ETIMEDOUT};
use rlibc::pthread::{
  PTHREAD_MUTEX_RECURSIVE, PTHREAD_PROCESS_PRIVATE, PTHREAD_PROCESS_SHARED, pthread_cond_broadcast,
  pthread_cond_destroy, pthread_cond_init, pthread_cond_signal, pthread_cond_t,
  pthread_cond_timedwait, pthread_cond_wait, pthread_condattr_destroy, pthread_condattr_getpshared,
  pthread_condattr_init, pthread_condattr_setpshared, pthread_condattr_t, pthread_mutex_destroy,
  pthread_mutex_init, pthread_mutex_lock, pthread_mutex_t, pthread_mutex_unlock,
  pthread_mutexattr_destroy, pthread_mutexattr_init, pthread_mutexattr_settype,
  pthread_mutexattr_t,
};
use rlibc::time::{CLOCK_REALTIME, clock_gettime, timespec};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::{ptr, thread};

fn init_mutex() -> pthread_mutex_t {
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutex_init(&raw mut mutex, ptr::null()), 0);

  mutex
}

fn init_cond() -> pthread_cond_t {
  let mut cond = pthread_cond_t::default();

  assert_eq!(pthread_cond_init(&raw mut cond, ptr::null()), 0);

  cond
}

fn init_recursive_mutex() -> pthread_mutex_t {
  let mut attr = pthread_mutexattr_t::default();
  let mut mutex = pthread_mutex_t::default();

  assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_mutexattr_settype(&raw mut attr, PTHREAD_MUTEX_RECURSIVE),
    0
  );
  assert_eq!(pthread_mutex_init(&raw mut mutex, &raw const attr), 0);
  assert_eq!(pthread_mutexattr_destroy(&raw mut attr), 0);

  mutex
}

fn destroy_sync_objects(mutex: &mut pthread_mutex_t, cond: &mut pthread_cond_t) {
  assert_eq!(pthread_cond_destroy(cond), 0);
  assert_eq!(pthread_mutex_destroy(mutex), 0);
}

#[test]
fn pthread_cond_wait_wakes_after_signal() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let ready = Arc::new(AtomicBool::new(false));
  let observed = Arc::new(AtomicBool::new(false));
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_ready = Arc::clone(&ready);
  let waiter_observed = Arc::clone(&observed);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while !waiter_ready.load(Ordering::Acquire) {
      assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    }

    waiter_observed.store(true, Ordering::Release);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  ready.store(true, Ordering::Release);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert!(
    observed.load(Ordering::Acquire),
    "waiter must observe signaled condition",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_broadcast_wakes_all_waiters() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let phase = Arc::new(AtomicUsize::new(0));
  let awake_count = Arc::new(AtomicUsize::new(0));
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let waiter_phase = Arc::clone(&phase);
      let waiter_awake_count = Arc::clone(&awake_count);
      let started_tx = started_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");

        while waiter_phase.load(Ordering::Acquire) == 0 {
          assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
        }

        waiter_awake_count.fetch_add(1, Ordering::AcqRel);
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    phase.store(1, Ordering::Release);
    assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  });

  assert_eq!(
    awake_count.load(Ordering::Acquire),
    2,
    "broadcast must wake all blocked waiters",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_broadcast_wakes_all_waiters_with_recursive_mutex() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let phase = Arc::new(AtomicUsize::new(0));
  let awake_count = Arc::new(AtomicUsize::new(0));
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let waiter_phase = Arc::clone(&phase);
      let waiter_awake_count = Arc::clone(&awake_count);
      let started_tx = started_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");

        while waiter_phase.load(Ordering::Acquire) == 0 {
          assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
        }

        waiter_awake_count.fetch_add(1, Ordering::AcqRel);
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    phase.store(1, Ordering::Release);
    assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  });

  assert_eq!(
    awake_count.load(Ordering::Acquire),
    2,
    "broadcast must wake all blocked waiters with recursive mutex",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_returns_etimedout_for_past_deadline() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);

  let mut past_deadline = now;

  past_deadline.tv_sec = past_deadline.tv_sec.saturating_sub(1);

  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const past_deadline),
    ETIMEDOUT,
  );
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_returns_etimedout_for_current_deadline() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);

  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const now),
    ETIMEDOUT
  );
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_returns_etimedout_after_future_deadline() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  let mut future_deadline = now;

  future_deadline.tv_nsec += 100_000_000;

  if future_deadline.tv_nsec >= 1_000_000_000 {
    future_deadline.tv_sec = future_deadline.tv_sec.saturating_add(1);
    future_deadline.tv_nsec -= 1_000_000_000;
  }

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const future_deadline),
    ETIMEDOUT,
  );
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_wakes_after_signal_before_deadline() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let ready = Arc::new(AtomicBool::new(false));
  let observed = Arc::new(AtomicBool::new(false));
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_ready = Arc::clone(&ready);
  let waiter_observed = Arc::clone(&observed);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let deadline = timespec {
      tv_sec: now.tv_sec.saturating_add(2),
      tv_nsec: now.tv_nsec,
    };

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while !waiter_ready.load(Ordering::Acquire) {
      assert_eq!(
        pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
        0
      );
    }

    waiter_observed.store(true, Ordering::Release);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  ready.store(true, Ordering::Release);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert!(
    observed.load(Ordering::Acquire),
    "timedwait waiter must observe signaled condition",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_wakes_after_signal_before_deadline_with_zero_initialized_cond() {
  let mut mutex = init_mutex();
  let mut cond = pthread_cond_t::default();
  let ready = Arc::new(AtomicBool::new(false));
  let observed = Arc::new(AtomicBool::new(false));
  let mutex_addr = (&raw mut mutex).addr();
  let cond_addr = (&raw mut cond).addr();
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_ready = Arc::clone(&ready);
  let waiter_observed = Arc::clone(&observed);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let deadline = timespec {
      tv_sec: now.tv_sec.saturating_add(2),
      tv_nsec: now.tv_nsec,
    };

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while !waiter_ready.load(Ordering::Acquire) {
      assert_eq!(
        pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
        0
      );
    }

    waiter_observed.store(true, Ordering::Release);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  ready.store(true, Ordering::Release);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert!(
    observed.load(Ordering::Acquire),
    "zero-initialized timedwait waiter must observe signaled condition",
  );
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_timedwait_wakes_after_signal_with_zero_initialized_cond_and_recursive_mutex() {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let ready = Arc::new(AtomicBool::new(false));
  let observed = Arc::new(AtomicBool::new(false));
  let mutex_addr = (&raw mut mutex).addr();
  let cond_addr = (&raw mut cond).addr();
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_ready = Arc::clone(&ready);
  let waiter_observed = Arc::clone(&observed);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let deadline = timespec {
      tv_sec: now.tv_sec.saturating_add(2),
      tv_nsec: now.tv_nsec,
    };

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while !waiter_ready.load(Ordering::Acquire) {
      assert_eq!(
        pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
        0
      );
    }

    waiter_observed.store(true, Ordering::Release);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  ready.store(true, Ordering::Release);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert!(
    observed.load(Ordering::Acquire),
    "zero-initialized recursive timedwait waiter must observe signaled condition",
  );
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_timedwait_wakes_after_signal_with_recursive_mutex() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let ready = Arc::new(AtomicBool::new(false));
  let observed = Arc::new(AtomicBool::new(false));
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_ready = Arc::clone(&ready);
  let waiter_observed = Arc::clone(&observed);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let deadline = timespec {
      tv_sec: now.tv_sec.saturating_add(2),
      tv_nsec: now.tv_nsec,
    };

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while !waiter_ready.load(Ordering::Acquire) {
      assert_eq!(
        pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
        0
      );
    }

    waiter_observed.store(true, Ordering::Release);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  ready.store(true, Ordering::Release);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert!(
    observed.load(Ordering::Acquire),
    "recursive-mutex timedwait waiter must observe signaled condition",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_rejects_null_abstime_without_unlocking_mutex() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, ptr::null()),
    EINVAL,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "timedwait argument validation must not drop caller mutex ownership",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_rejects_null_abstime_and_preserves_recursive_lock_depth() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, ptr::null()),
    EINVAL,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "null-abstime rejection must preserve recursive depth level 2 -> 1",
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "second unlock must succeed when recursive depth is preserved",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_rejects_invalid_tv_nsec_without_unlocking_mutex() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let invalid_deadline = timespec {
    tv_sec: 1,
    tv_nsec: 1_000_000_000,
  };

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const invalid_deadline),
    EINVAL,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "timedwait must preserve mutex ownership when abstime is invalid",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_rejects_negative_tv_nsec_without_unlocking_mutex() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let invalid_deadline = timespec {
    tv_sec: 1,
    tv_nsec: -1,
  };

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const invalid_deadline),
    EINVAL,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "timedwait must preserve mutex ownership when tv_nsec is negative",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_requires_mutex_ownership() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  let deadline = timespec {
    tv_sec: now.tv_sec.saturating_add(1),
    tv_nsec: now.tv_nsec,
  };

  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const deadline),
    EPERM,
  );
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_rejects_null_cond_without_unlocking_mutex() {
  let mut mutex = init_mutex();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  let deadline = timespec {
    tv_sec: now.tv_sec.saturating_add(1),
    tv_nsec: now.tv_nsec,
  };

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(ptr::null_mut(), &raw mut mutex, &raw const deadline),
    EINVAL,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "null cond rejection must not change mutex ownership",
  );
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_timedwait_accepts_zero_initialized_cond_and_times_out() {
  let mut mutex = init_mutex();
  let mut cond = pthread_cond_t::default();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const now),
    ETIMEDOUT,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "timedwait timeout path must return with caller mutex re-locked",
  );
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_timedwait_rejects_null_mutex_pointer() {
  let mut cond = init_cond();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  let deadline = timespec {
    tv_sec: now.tv_sec.saturating_add(1),
    tv_nsec: now.tv_nsec,
  };

  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, ptr::null_mut(), &raw const deadline),
    EINVAL,
  );
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_timedwait_rejects_uninitialized_mutex_pointer() {
  let mut cond = init_cond();
  let mut mutex = pthread_mutex_t::default();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  let deadline = timespec {
    tv_sec: now.tv_sec.saturating_add(1),
    tv_nsec: now.tv_nsec,
  };

  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const deadline),
    EINVAL
  );
  assert_eq!(
    pthread_cond_destroy(&raw mut cond),
    0,
    "timedwait EINVAL path must not corrupt cond state",
  );
}

#[test]
fn pthread_cond_wait_accepts_zero_initialized_cond() {
  let mut mutex = init_mutex();
  let mut cond = pthread_cond_t::default();
  let ready = Arc::new(AtomicBool::new(false));
  let observed = Arc::new(AtomicBool::new(false));
  let mutex_addr = (&raw mut mutex).addr();
  let cond_addr = (&raw mut cond).addr();
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_ready = Arc::clone(&ready);
  let waiter_observed = Arc::clone(&observed);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while !waiter_ready.load(Ordering::Acquire) {
      assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    }

    waiter_observed.store(true, Ordering::Release);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  ready.store(true, Ordering::Release);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert!(
    observed.load(Ordering::Acquire),
    "wait on zero-initialized cond must wake after signal",
  );
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_wait_with_recursive_mutex_nested_lock_depth_restores_depth() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex).addr();
  let cond_addr = (&raw mut cond).addr();
  let (locked_tx, locked_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    locked_tx
      .send(())
      .expect("failed to send nested recursive-lock signal");
    assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    assert_eq!(
      pthread_mutex_unlock(mutex_ptr),
      0,
      "waiter unlock must restore recursive depth level 2 -> 1 after cond_wait",
    );
    assert_eq!(
      pthread_mutex_unlock(mutex_ptr),
      0,
      "waiter must still own recursive mutex after cond_wait relock",
    );
  });

  locked_rx
    .recv()
    .expect("waiter did not acquire recursive mutex");
  assert_eq!(
    pthread_mutex_lock(&raw mut mutex),
    0,
    "main thread lock must proceed only after waiter fully releases recursive depth",
  );
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_wait_wakes_after_signal_with_recursive_mutex() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let ready = Arc::new(AtomicBool::new(false));
  let observed = Arc::new(AtomicBool::new(false));
  let mutex_addr = (&raw mut mutex).addr();
  let cond_addr = (&raw mut cond).addr();
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_ready = Arc::clone(&ready);
  let waiter_observed = Arc::clone(&observed);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while !waiter_ready.load(Ordering::Acquire) {
      assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    }

    waiter_observed.store(true, Ordering::Release);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  ready.store(true, Ordering::Release);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert!(
    observed.load(Ordering::Acquire),
    "waiter must observe signal when using recursive mutex",
  );
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_wait_wakes_after_signal_with_zero_initialized_cond_and_recursive_mutex() {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let ready = Arc::new(AtomicBool::new(false));
  let observed = Arc::new(AtomicBool::new(false));
  let mutex_addr = (&raw mut mutex).addr();
  let cond_addr = (&raw mut cond).addr();
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_ready = Arc::clone(&ready);
  let waiter_observed = Arc::clone(&observed);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while !waiter_ready.load(Ordering::Acquire) {
      assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    }

    waiter_observed.store(true, Ordering::Release);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  ready.store(true, Ordering::Release);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert!(
    observed.load(Ordering::Acquire),
    "zero-initialized recursive wait waiter must observe signaled condition",
  );
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_signal_and_broadcast_accept_zero_initialized_cond() {
  let mut cond = pthread_cond_t::default();

  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_zero_initialized_destroy_transitions_to_destroyed_state() {
  let mut cond = pthread_cond_t::default();

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), EINVAL);
  assert_eq!(pthread_cond_broadcast(&raw mut cond), EINVAL);
  assert_eq!(
    pthread_cond_destroy(&raw mut cond),
    0,
    "destroy remains idempotent after zero-init -> destroyed transition",
  );
  assert_eq!(pthread_cond_init(&raw mut cond, ptr::null()), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_signal_and_broadcast_succeed_with_no_waiters() {
  let mut cond = init_cond();

  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_destroy_rejects_null_pointer() {
  assert_eq!(pthread_cond_destroy(ptr::null_mut()), EINVAL);
}

#[test]
fn pthread_cond_destroy_is_idempotent_after_successful_destroy() {
  let mut cond = init_cond();

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(
    pthread_cond_destroy(&raw mut cond),
    0,
    "libc-compatible cond destroy should be idempotent",
  );
}

#[test]
fn pthread_cond_signal_and_broadcast_reject_destroyed_cond() {
  let mut cond = init_cond();

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), EINVAL);
  assert_eq!(pthread_cond_broadcast(&raw mut cond), EINVAL);
}

#[test]
fn pthread_cond_wait_rejects_destroyed_cond_without_unlocking_mutex() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_wait(&raw mut cond, &raw mut mutex), EINVAL);
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "destroyed cond rejection must preserve mutex ownership",
  );
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_timedwait_rejects_destroyed_cond_without_unlocking_mutex() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  let deadline = timespec {
    tv_sec: now.tv_sec.saturating_add(1),
    tv_nsec: now.tv_nsec,
  };

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const deadline),
    EINVAL,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "destroyed cond timedwait rejection must preserve mutex ownership",
  );
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_timedwait_with_recursive_mutex_nested_lock_depth_restores_depth() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const now),
    ETIMEDOUT,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "timedwait must restore recursive ownership level 2 -> 1 after wake path",
  );
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_timedwait_with_zero_initialized_cond_and_recursive_mutex_nested_lock_depth_restores_depth()
 {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const now),
    ETIMEDOUT,
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "zero-initialized timedwait must restore recursive ownership level 2 -> 1",
  );
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_validates_null_arguments() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();

  assert_eq!(pthread_cond_signal(ptr::null_mut()), EINVAL);
  assert_eq!(pthread_cond_broadcast(ptr::null_mut()), EINVAL);
  assert_eq!(pthread_cond_wait(ptr::null_mut(), &raw mut mutex), EINVAL);
  assert_eq!(pthread_cond_wait(&raw mut cond, ptr::null_mut()), EINVAL);

  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_init_accepts_initialized_attr() {
  let mut cond = pthread_cond_t::default();
  let mut attr = pthread_condattr_t::default();

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(pthread_cond_init(&raw mut cond, &raw const attr), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_cond_init_rejects_null_cond_pointer() {
  assert_eq!(pthread_cond_init(ptr::null_mut(), ptr::null()), EINVAL);
}

#[test]
fn pthread_cond_init_rejects_reinitialization_with_ebusy() {
  let mut cond = pthread_cond_t::default();

  assert_eq!(pthread_cond_init(&raw mut cond, ptr::null()), 0);
  assert_eq!(pthread_cond_init(&raw mut cond, ptr::null()), EBUSY);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_init_allows_reinitialization_after_destroy() {
  let mut cond = pthread_cond_t::default();

  assert_eq!(pthread_cond_init(&raw mut cond, ptr::null()), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_cond_init(&raw mut cond, ptr::null()), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_init_rejects_uninitialized_attr() {
  let mut cond = pthread_cond_t::default();
  let attr = pthread_condattr_t::default();

  assert_eq!(pthread_cond_init(&raw mut cond, &raw const attr), EINVAL);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_init_accepts_destroyed_attr() {
  let mut cond = pthread_cond_t::default();
  let mut attr = pthread_condattr_t::default();

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
  assert_eq!(pthread_cond_init(&raw mut cond, &raw const attr), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_init_rejects_invalid_pshared_attr_and_keeps_cond_uninitialized() {
  let mut cond = pthread_cond_t::default();
  let mut attr = pthread_condattr_t::default();

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);

  // SAFETY: `pthread_condattr_t` is `#[repr(C)]` and starts with
  // `pshared: c_int`; test-only mutation injects an invalid selector.
  unsafe {
    (&raw mut attr)
      .cast::<rlibc::abi::types::c_int>()
      .write(1234);
  }

  assert_eq!(pthread_cond_init(&raw mut cond, &raw const attr), EINVAL);
  assert_eq!(
    pthread_cond_destroy(&raw mut cond),
    0,
    "zero-initialized cond remains destroyable when init rejects invalid pshared value",
  );
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_cond_init_rejects_process_shared_attr_and_keeps_cond_uninitialized() {
  let mut cond = pthread_cond_t::default();
  let mut attr = pthread_condattr_t::default();

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_setpshared(&raw mut attr, PTHREAD_PROCESS_SHARED),
    ENOTSUP,
  );

  // SAFETY: `pthread_condattr_t` is `#[repr(C)]` and begins with `pshared: c_int`.
  // This test-only mutation forces the unsupported process-shared branch.
  unsafe {
    (&raw mut attr)
      .cast::<rlibc::abi::types::c_int>()
      .write(PTHREAD_PROCESS_SHARED);
  }

  assert_eq!(pthread_cond_init(&raw mut cond, &raw const attr), ENOTSUP);
  assert_eq!(
    pthread_cond_destroy(&raw mut cond),
    0,
    "zero-initialized cond remains destroyable when init rejects shared attributes",
  );
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_condattr_init_rejects_null_pointer() {
  assert_eq!(pthread_condattr_init(ptr::null_mut()), EINVAL);
}

#[test]
fn pthread_condattr_destroy_rejects_null_pointer() {
  assert_eq!(pthread_condattr_destroy(ptr::null_mut()), EINVAL);
}

#[test]
fn pthread_condattr_destroy_keeps_attr_usable() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    0
  );
  assert_eq!(observed_pshared, PTHREAD_PROCESS_PRIVATE);
  assert_eq!(
    pthread_condattr_setpshared(&raw mut attr, PTHREAD_PROCESS_PRIVATE),
    0,
  );
}

#[test]
fn pthread_condattr_destroy_accepts_uninitialized_attr() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    EINVAL,
    "destroy on uninitialized attr must leave it uninitialized",
  );
}

#[test]
fn pthread_condattr_reinit_restores_default_private_after_destroy() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    0
  );
  assert_eq!(observed_pshared, PTHREAD_PROCESS_PRIVATE);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_cond_init_accepts_reinitialized_attr_after_destroy() {
  let mut cond = pthread_cond_t::default();
  let mut attr = pthread_condattr_t::default();

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(pthread_cond_init(&raw mut cond, &raw const attr), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_condattr_destroy_is_idempotent_for_initialized_attr() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    0
  );
  assert_eq!(
    observed_pshared, PTHREAD_PROCESS_PRIVATE,
    "double-destroy keeps prior attr configuration available",
  );
}

#[test]
fn pthread_condattr_getpshared_returns_default_private() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    0,
  );
  assert_eq!(observed_pshared, PTHREAD_PROCESS_PRIVATE);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_condattr_setpshared_private_round_trip() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_setpshared(&raw mut attr, PTHREAD_PROCESS_PRIVATE),
    0,
  );
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    0,
  );
  assert_eq!(observed_pshared, PTHREAD_PROCESS_PRIVATE);
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_condattr_setpshared_shared_returns_enotsup() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_setpshared(&raw mut attr, PTHREAD_PROCESS_SHARED),
    ENOTSUP,
  );
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    0,
  );
  assert_eq!(
    observed_pshared, PTHREAD_PROCESS_PRIVATE,
    "unsupported shared setting must keep previous process-private value",
  );
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_condattr_setpshared_invalid_returns_einval() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(pthread_condattr_setpshared(&raw mut attr, 9999), EINVAL);
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    0,
  );
  assert_eq!(
    observed_pshared, PTHREAD_PROCESS_PRIVATE,
    "invalid pshared value must keep existing process-private setting",
  );
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_condattr_getset_reject_uninitialized_attr() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, &raw mut observed_pshared),
    EINVAL,
  );
  assert_eq!(
    pthread_condattr_setpshared(&raw mut attr, PTHREAD_PROCESS_PRIVATE),
    EINVAL,
  );
}

#[test]
fn pthread_condattr_getpshared_rejects_null_pointers() {
  let mut attr = pthread_condattr_t::default();
  let mut observed_pshared = -1;

  assert_eq!(pthread_condattr_init(&raw mut attr), 0);
  assert_eq!(
    pthread_condattr_getpshared(ptr::null(), &raw mut observed_pshared),
    EINVAL,
  );
  assert_eq!(
    pthread_condattr_getpshared(&raw const attr, ptr::null_mut()),
    EINVAL
  );
  assert_eq!(pthread_condattr_destroy(&raw mut attr), 0);
}

#[test]
fn pthread_condattr_setpshared_rejects_null_attribute_pointer() {
  assert_eq!(
    pthread_condattr_setpshared(ptr::null_mut(), PTHREAD_PROCESS_PRIVATE),
    EINVAL,
  );
}

#[test]
fn pthread_cond_wait_requires_mutex_ownership() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();

  assert_eq!(pthread_cond_wait(&raw mut cond, &raw mut mutex), EPERM);
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_cond_wait_rejects_null_cond_without_unlocking_mutex() {
  let mut mutex = init_mutex();

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_wait(ptr::null_mut(), &raw mut mutex), EINVAL);
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "null cond rejection must preserve caller mutex ownership",
  );
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_wait_rejects_null_cond_and_preserves_recursive_lock_depth() {
  let mut mutex = init_recursive_mutex();

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_wait(ptr::null_mut(), &raw mut mutex), EINVAL);
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "null-cond rejection must preserve recursive depth level 2 -> 1",
  );
  assert_eq!(
    pthread_mutex_unlock(&raw mut mutex),
    0,
    "second unlock must succeed when recursive depth is preserved",
  );
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_wait_rejects_uninitialized_mutex_pointer() {
  let mut mutex = pthread_mutex_t::default();
  let mut cond = init_cond();

  assert_eq!(pthread_cond_wait(&raw mut cond, &raw mut mutex), EINVAL);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_wait_rejects_destroyed_mutex_pointer() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();

  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
  assert_eq!(pthread_cond_wait(&raw mut cond, &raw mut mutex), EINVAL);
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_timedwait_rejects_destroyed_mutex_pointer() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

  let deadline = timespec {
    tv_sec: now.tv_sec.saturating_add(1),
    tv_nsec: now.tv_nsec,
  };

  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
  assert_eq!(
    pthread_cond_timedwait(&raw mut cond, &raw mut mutex, &raw const deadline),
    EINVAL,
  );
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
}

#[test]
fn pthread_cond_destroy_returns_ebusy_while_waiter_is_blocked() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_destroy(&raw mut cond), EBUSY);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_cond_waiter_references_mutex() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while cond-wait still references mutex state",
  );

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_zero_initialized_cond_waiter_references_mutex() {
  let mut mutex = init_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while zero-initialized cond-wait keeps mutex referenced",
  );

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_zero_initialized_cond_timedwaiter_references_mutex() {
  let mut mutex = init_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let deadline = timespec {
      tv_sec: now.tv_sec.saturating_add(10),
      tv_nsec: now.tv_nsec,
    };

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(
      pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
      0
    );
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while zero-initialized timedwait keeps mutex referenced",
  );

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_cond_timedwaiter_references_mutex() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let deadline = timespec {
      tv_sec: now.tv_sec.saturating_add(10),
      tv_nsec: now.tv_nsec,
    };

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(
      pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
      0
    );
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while cond-timedwait still references mutex state",
  );

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_mutex_destroy_succeeds_after_cond_timedwait_timeout_releases_reference() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let mut deadline = now;

    deadline.tv_nsec += 100_000_000;

    if deadline.tv_nsec >= 1_000_000_000 {
      deadline.tv_sec = deadline.tv_sec.saturating_add(1);
      deadline.tv_nsec -= 1_000_000_000;
    }

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(
      pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
      ETIMEDOUT
    );
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while timedwait keeps a live mutex reference",
  );

  waiter.join().expect("waiter thread panicked");
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    0,
    "timeout path must release mutex reference once waiter returns",
  );
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_recursive_cond_timedwaiter_references_mutex() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let deadline = timespec {
      tv_sec: now.tv_sec.saturating_add(10),
      tv_nsec: now.tv_nsec,
    };

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(
      pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
      0
    );
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
    assert_eq!(
      pthread_mutex_unlock(mutex_ptr),
      0,
      "recursive timedwait waiter must regain full recursive depth before exit",
    );
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while recursive cond-timedwait keeps a mutex reference",
  );

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  destroy_sync_objects(&mut mutex, &mut cond);
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_zero_initialized_recursive_cond_timedwaiter_references_mutex()
 {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let deadline = timespec {
      tv_sec: now.tv_sec.saturating_add(10),
      tv_nsec: now.tv_nsec,
    };

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(
      pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
      0
    );
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
    assert_eq!(
      pthread_mutex_unlock(mutex_ptr),
      0,
      "zero-initialized recursive timedwait waiter must regain full recursive depth before exit",
    );
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while zero-initialized recursive cond timedwait keeps a mutex reference",
  );

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_succeeds_after_recursive_cond_timedwait_timeout_releases_reference() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let mut deadline = now;

    deadline.tv_nsec += 500_000_000;

    if deadline.tv_nsec >= 1_000_000_000 {
      deadline.tv_sec = deadline.tv_sec.saturating_add(1);
      deadline.tv_nsec -= 1_000_000_000;
    }

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(
      pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
      ETIMEDOUT
    );
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
    assert_eq!(
      pthread_mutex_unlock(mutex_ptr),
      0,
      "recursive timedwait waiter must restore recursive depth after timeout",
    );
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while recursive timedwait keeps a live mutex reference",
  );

  waiter.join().expect("waiter thread panicked");
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    0,
    "recursive timeout path must release mutex reference once waiter returns",
  );
}

#[test]
fn pthread_mutex_destroy_succeeds_after_zero_initialized_recursive_cond_timedwait_timeout_releases_reference()
 {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut now = timespec {
      tv_sec: 0,
      tv_nsec: 0,
    };

    assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

    let mut deadline = now;

    deadline.tv_nsec += 500_000_000;

    if deadline.tv_nsec >= 1_000_000_000 {
      deadline.tv_sec = deadline.tv_sec.saturating_add(1);
      deadline.tv_nsec -= 1_000_000_000;
    }

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(
      pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
      ETIMEDOUT
    );
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
    assert_eq!(
      pthread_mutex_unlock(mutex_ptr),
      0,
      "zero-initialized recursive timedwait waiter must restore recursive depth after timeout",
    );
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while zero-initialized recursive timedwait keeps a live mutex reference",
  );

  waiter.join().expect("waiter thread panicked");
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    0,
    "zero-initialized recursive timeout path must release mutex reference once waiter returns",
  );
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_multiple_cond_timedwaiters_reference_mutex() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;
        let mut now = timespec {
          tv_sec: 0,
          tv_nsec: 0,
        };

        assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

        let mut deadline = now;

        deadline.tv_nsec += 500_000_000;

        if deadline.tv_nsec >= 1_000_000_000 {
          deadline.tv_sec = deadline.tv_sec.saturating_add(1);
          deadline.tv_nsec -= 1_000_000_000;
        }

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(
          pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
          ETIMEDOUT
        );
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while timedwait waiters still reference mutex state",
    );
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_multiple_zero_initialized_cond_timedwaiters_reference_mutex()
 {
  let mut mutex = init_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;
        let mut now = timespec {
          tv_sec: 0,
          tv_nsec: 0,
        };

        assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

        let mut deadline = now;

        deadline.tv_nsec += 500_000_000;

        if deadline.tv_nsec >= 1_000_000_000 {
          deadline.tv_sec = deadline.tv_sec.saturating_add(1);
          deadline.tv_nsec -= 1_000_000_000;
        }

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(
          pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
          ETIMEDOUT
        );
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while zero-initialized timedwait waiters still reference mutex state",
    );
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_multiple_recursive_cond_timedwaiters_reference_mutex()
{
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;
        let mut now = timespec {
          tv_sec: 0,
          tv_nsec: 0,
        };

        assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

        let mut deadline = now;

        deadline.tv_nsec += 500_000_000;

        if deadline.tv_nsec >= 1_000_000_000 {
          deadline.tv_sec = deadline.tv_sec.saturating_add(1);
          deadline.tv_nsec -= 1_000_000_000;
        }

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(
          pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
          ETIMEDOUT
        );
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        assert_eq!(
          pthread_mutex_unlock(mutex_ptr),
          0,
          "recursive timedwait waiter must restore recursive depth after timeout",
        );
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while recursive timedwait waiters still reference mutex state",
    );
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_returns_ebusy_while_multiple_zero_initialized_recursive_cond_timedwaiters_reference_mutex()
 {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;
        let mut now = timespec {
          tv_sec: 0,
          tv_nsec: 0,
        };

        assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

        let mut deadline = now;

        deadline.tv_nsec += 500_000_000;

        if deadline.tv_nsec >= 1_000_000_000 {
          deadline.tv_sec = deadline.tv_sec.saturating_add(1);
          deadline.tv_nsec -= 1_000_000_000;
        }

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(
          pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
          ETIMEDOUT
        );
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        assert_eq!(
          pthread_mutex_unlock(mutex_ptr),
          0,
          "zero-initialized recursive timedwait waiter must restore recursive depth after timeout",
        );
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while zero-initialized recursive timedwait waiters still reference mutex state",
    );
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_stays_ebusy_after_single_signal_while_second_timedwaiter_is_blocked() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let (woke_tx, woke_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();
      let woke_tx = woke_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;
        let mut now = timespec {
          tv_sec: 0,
          tv_nsec: 0,
        };

        assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

        let deadline = timespec {
          tv_sec: now.tv_sec.saturating_add(10),
          tv_nsec: now.tv_nsec,
        };

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(
          pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
          0
        );
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        woke_tx.send(()).expect("failed to send wake signal");
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while two timedwait waiters reference mutex state",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_signal(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

    woke_rx
      .recv()
      .expect("first timedwait waiter did not wake after signal");
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must remain busy while another timedwait waiter stays blocked",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    woke_rx
      .recv()
      .expect("second timedwait waiter did not wake after broadcast");
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_stays_ebusy_after_single_signal_while_second_waiter_is_blocked() {
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let (woke_tx, woke_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();
      let woke_tx = woke_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        woke_tx.send(()).expect("failed to send wake signal");
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while two cond-wait waiters reference mutex state",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_signal(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

    woke_rx
      .recv()
      .expect("first waiter did not wake after signal");
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must remain busy while another cond-wait waiter stays blocked",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    woke_rx
      .recv()
      .expect("second waiter did not wake after broadcast");
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_stays_ebusy_after_single_signal_while_second_recursive_waiter_is_blocked()
{
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let (woke_tx, woke_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();
      let woke_tx = woke_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        assert_eq!(
          pthread_mutex_unlock(mutex_ptr),
          0,
          "recursive cond-wait waiter must restore recursive depth after wake",
        );
        woke_tx.send(()).expect("failed to send wake signal");
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while two recursive cond-wait waiters reference mutex state",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_signal(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

    woke_rx
      .recv()
      .expect("first recursive waiter did not wake after signal");
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must remain busy while another recursive cond-wait waiter stays blocked",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    woke_rx
      .recv()
      .expect("second recursive waiter did not wake after broadcast");
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_stays_ebusy_after_single_signal_while_second_zero_initialized_cond_waiter_is_blocked()
 {
  let mut mutex = init_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let (woke_tx, woke_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();
      let woke_tx = woke_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        woke_tx.send(()).expect("failed to send wake signal");
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while two zero-initialized cond-wait waiters reference mutex state",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_signal(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

    woke_rx
      .recv()
      .expect("first waiter did not wake after signal");
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must remain busy while another zero-initialized cond-wait waiter stays blocked",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    woke_rx
      .recv()
      .expect("second waiter did not wake after broadcast");
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_succeeds_after_recursive_cond_wait_signal_releases_reference() {
  let mut mutex = init_recursive_mutex();
  let mut cond = init_cond();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
    assert_eq!(
      pthread_mutex_unlock(mutex_ptr),
      0,
      "recursive cond-wait waiter must restore recursive depth after wake",
    );
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while recursive cond-wait keeps a live mutex reference",
  );

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    0,
    "recursive signal wake path must release mutex reference once waiter returns",
  );
}

#[test]
fn pthread_mutex_destroy_succeeds_after_zero_initialized_recursive_cond_wait_signal_releases_reference()
 {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");
    assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
    assert_eq!(
      pthread_mutex_unlock(mutex_ptr),
      0,
      "zero-initialized recursive cond-wait waiter must restore recursive depth after wake",
    );
  });

  started_rx.recv().expect("waiter did not start");
  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    EBUSY,
    "destroy must fail while zero-initialized recursive cond-wait keeps a live mutex reference",
  );

  assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
  assert_eq!(pthread_cond_signal(&raw mut cond), 0);
  assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

  waiter.join().expect("waiter thread panicked");
  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(
    pthread_mutex_destroy(&raw mut mutex),
    0,
    "zero-initialized recursive signal wake path must release mutex reference once waiter returns",
  );
}

#[test]
fn pthread_mutex_destroy_stays_ebusy_after_single_signal_while_second_zero_initialized_recursive_cond_waiter_is_blocked()
 {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let (woke_tx, woke_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();
      let woke_tx = woke_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        woke_tx.send(()).expect("failed to send wake signal");
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while two zero-initialized recursive cond-wait waiters reference mutex state",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_signal(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

    woke_rx
      .recv()
      .expect("first waiter did not wake after signal");
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must remain busy while another zero-initialized recursive cond-wait waiter stays blocked",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    woke_rx
      .recv()
      .expect("second waiter did not wake after broadcast");
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_mutex_destroy_stays_ebusy_after_single_signal_while_second_zero_initialized_recursive_cond_timedwaiter_is_blocked()
 {
  let mut mutex = init_recursive_mutex();
  let mut cond = pthread_cond_t::default();
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let (woke_tx, woke_rx) = mpsc::channel();

  thread::scope(|scope| {
    for _ in 0..2 {
      let started_tx = started_tx.clone();
      let woke_tx = woke_tx.clone();

      scope.spawn(move || {
        let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
        let cond_ptr = cond_addr as *mut pthread_cond_t;
        let mut now = timespec {
          tv_sec: 0,
          tv_nsec: 0,
        };

        assert_eq!(clock_gettime(CLOCK_REALTIME, &raw mut now), 0);

        let deadline = timespec {
          tv_sec: now.tv_sec.saturating_add(10),
          tv_nsec: now.tv_nsec,
        };

        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
        started_tx
          .send(())
          .expect("failed to send waiter start signal");
        assert_eq!(
          pthread_cond_timedwait(cond_ptr, mutex_ptr, &raw const deadline),
          0
        );
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
        woke_tx.send(()).expect("failed to send wake signal");
      });
    }

    started_rx.recv().expect("first waiter did not start");
    started_rx.recv().expect("second waiter did not start");

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must fail while two zero-initialized recursive timedwait waiters reference mutex state",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_signal(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);

    woke_rx
      .recv()
      .expect("first waiter did not wake after signal");
    assert_eq!(
      pthread_mutex_destroy(&raw mut mutex),
      EBUSY,
      "destroy must remain busy while another zero-initialized recursive timedwait waiter stays blocked",
    );

    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    assert_eq!(pthread_cond_broadcast(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
    woke_rx
      .recv()
      .expect("second waiter did not wake after broadcast");
  });

  assert_eq!(pthread_cond_destroy(&raw mut cond), 0);
  assert_eq!(pthread_mutex_destroy(&raw mut mutex), 0);
}

#[test]
fn pthread_cond_signal_wait_round_trips_without_lost_wakeups() {
  const ROUNDS: usize = 8;
  let mut mutex = init_mutex();
  let mut cond = init_cond();
  let phase = Arc::new(AtomicUsize::new(0));
  let mutex_addr = (&raw mut mutex) as usize;
  let cond_addr = (&raw mut cond) as usize;
  let (started_tx, started_rx) = mpsc::channel();
  let waiter_phase = Arc::clone(&phase);
  let waiter = thread::spawn(move || {
    let mutex_ptr = mutex_addr as *mut pthread_mutex_t;
    let cond_ptr = cond_addr as *mut pthread_cond_t;
    let mut observed = 0usize;

    assert_eq!(pthread_mutex_lock(mutex_ptr), 0);
    started_tx
      .send(())
      .expect("failed to send waiter start signal");

    while observed < ROUNDS {
      while waiter_phase.load(Ordering::Acquire) == observed {
        assert_eq!(pthread_cond_wait(cond_ptr, mutex_ptr), 0);
      }

      observed += 1;
    }

    assert_eq!(pthread_mutex_unlock(mutex_ptr), 0);
  });

  started_rx.recv().expect("waiter did not start");

  for next_phase in 1..=ROUNDS {
    assert_eq!(pthread_mutex_lock(&raw mut mutex), 0);
    phase.store(next_phase, Ordering::Release);
    assert_eq!(pthread_cond_signal(&raw mut cond), 0);
    assert_eq!(pthread_mutex_unlock(&raw mut mutex), 0);
  }

  waiter.join().expect("waiter thread panicked");
  assert_eq!(phase.load(Ordering::Acquire), ROUNDS);
  destroy_sync_objects(&mut mutex, &mut cond);
}
