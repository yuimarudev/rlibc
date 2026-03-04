#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_int;
use rlibc::errno::__errno_location;
use std::sync::{Arc, Barrier, mpsc};
use std::thread;

type NestedThreadSample = (usize, c_int, c_int);

type ParentThreadSample = (usize, c_int, c_int, NestedThreadSample);

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local storage for the calling thread.
  unsafe {
    __errno_location().write(value);
  }
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns readable thread-local storage for the calling thread.
  unsafe { __errno_location().read() }
}

fn usize_to_i32(value: usize) -> i32 {
  i32::try_from(value).expect("test index must fit in i32")
}

#[test]
fn errno_location_storage_isolated_across_simultaneous_threads() {
  write_errno(111);

  let main_addr = __errno_location() as usize;
  let barrier = Arc::new(Barrier::new(3));
  let (sender, receiver) = mpsc::channel();
  let mut handles = Vec::new();

  for value in [211, 311] {
    let barrier = Arc::clone(&barrier);
    let sender = sender.clone();

    handles.push(thread::spawn(move || {
      let child_addr = __errno_location() as usize;
      let child_initial = read_errno();

      barrier.wait();
      write_errno(value);

      let child_final = read_errno();

      sender
        .send((child_addr, child_initial, child_final))
        .expect("send child errno sample");
    }));
  }

  drop(sender);
  barrier.wait();

  let mut samples = Vec::new();

  for sample in receiver {
    samples.push(sample);
  }

  for handle in handles {
    handle.join().expect("child thread panicked");
  }

  assert_eq!(samples.len(), 2, "expected two child errno samples");
  assert_eq!(
    read_errno(),
    111,
    "main thread errno must stay unchanged across child writes",
  );

  let (addr_a, initial_a, final_a) = samples[0];
  let (addr_b, initial_b, final_b) = samples[1];

  assert_eq!(initial_a, 0, "child thread A errno must start at zero");
  assert_eq!(initial_b, 0, "child thread B errno must start at zero");
  assert_ne!(
    addr_a, main_addr,
    "child thread A must not alias main-thread errno storage",
  );
  assert_ne!(
    addr_b, main_addr,
    "child thread B must not alias main-thread errno storage",
  );
  assert_ne!(
    addr_a, addr_b,
    "simultaneously live child threads must not share errno storage",
  );

  let mut finals = [final_a, final_b];

  finals.sort_unstable();
  assert_eq!(
    finals,
    [211, 311],
    "each child must keep its own errno write"
  );
}

#[test]
fn errno_location_pointer_remains_stable_after_child_thread_activity() {
  write_errno(515);

  let main_before = __errno_location() as usize;
  let child = thread::spawn(|| {
    write_errno(777);

    (__errno_location() as usize, read_errno())
  });
  let (child_addr, child_errno) = child.join().expect("child thread panicked");
  let main_after = __errno_location() as usize;

  assert_eq!(child_errno, 777, "child thread errno write must persist");
  assert_ne!(
    child_addr, main_before,
    "child thread must not alias main-thread errno storage",
  );
  assert_eq!(
    main_after, main_before,
    "main-thread errno pointer must remain stable after child thread activity",
  );
  assert_eq!(
    read_errno(),
    515,
    "main-thread errno value must stay unchanged by child thread writes",
  );
}

#[test]
fn new_thread_starts_with_zero_errno_after_previous_thread_exit() {
  write_errno(909);

  let first_child_errno = thread::spawn(|| {
    write_errno(1234);

    read_errno()
  })
  .join()
  .expect("first child thread panicked");
  let second_child_initial = thread::spawn(read_errno)
    .join()
    .expect("second child thread panicked");

  assert_eq!(
    first_child_errno, 1234,
    "first child thread must observe its own errno write",
  );
  assert_eq!(
    second_child_initial, 0,
    "new child thread must start with zero errno even after prior thread writes",
  );
  assert_eq!(
    read_errno(),
    909,
    "main thread errno must stay unchanged across child thread lifetimes",
  );
}

#[test]
fn main_errno_and_pointer_stay_stable_across_many_child_threads() {
  write_errno(707);

  let main_addr = __errno_location() as usize;

  for offset in 0..8_i32 {
    let child_value = 1000 + offset;
    let (child_addr, child_errno) = thread::spawn(move || {
      write_errno(child_value);

      (__errno_location() as usize, read_errno())
    })
    .join()
    .expect("child thread panicked");

    assert_eq!(
      child_errno, child_value,
      "child thread must observe its own errno write",
    );
    assert_ne!(
      child_addr, main_addr,
      "child thread must not alias main-thread errno storage",
    );
    assert_eq!(
      __errno_location() as usize,
      main_addr,
      "main-thread errno pointer must remain stable across child activity",
    );
    assert_eq!(
      read_errno(),
      707,
      "main-thread errno value must stay unchanged across child activity",
    );
  }
}

#[test]
fn simultaneous_many_threads_keep_distinct_errno_storage_and_values() {
  const CHILD_COUNT: usize = 6;

  write_errno(606);

  let main_addr = __errno_location() as usize;
  let barrier = Arc::new(Barrier::new(CHILD_COUNT + 1));
  let (sender, receiver) = mpsc::channel();
  let mut handles = Vec::new();

  for index in 0..CHILD_COUNT {
    let barrier = Arc::clone(&barrier);
    let sender = sender.clone();

    handles.push(thread::spawn(move || {
      let child_addr = __errno_location() as usize;
      let child_initial = read_errno();
      let child_value = 2000 + usize_to_i32(index);

      barrier.wait();
      write_errno(child_value);

      sender
        .send((child_addr, child_initial, read_errno()))
        .expect("send child errno sample");
    }));
  }

  drop(sender);
  barrier.wait();

  let mut child_addrs = Vec::new();
  let mut child_values = Vec::new();

  for (child_addr, child_initial, child_final) in receiver {
    assert_eq!(
      child_initial, 0,
      "each child thread errno must start at zero",
    );
    child_addrs.push(child_addr);
    child_values.push(child_final);
  }

  for handle in handles {
    handle.join().expect("child thread panicked");
  }

  assert_eq!(child_addrs.len(), CHILD_COUNT, "expected all child samples");
  assert_eq!(child_values.len(), CHILD_COUNT, "expected all child values");

  for child_addr in &child_addrs {
    assert_ne!(
      *child_addr, main_addr,
      "child thread must not alias main-thread errno storage",
    );
  }

  child_addrs.sort_unstable();
  child_addrs.dedup();
  assert_eq!(
    child_addrs.len(),
    CHILD_COUNT,
    "simultaneously live child threads must each have distinct errno storage",
  );

  child_values.sort_unstable();
  assert_eq!(
    child_values,
    (0..CHILD_COUNT)
      .map(|index| 2000 + usize_to_i32(index))
      .collect::<Vec<_>>(),
    "each child thread must preserve its own errno write",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable after many child threads",
  );
  assert_eq!(
    read_errno(),
    606,
    "main-thread errno value must stay unchanged across child thread writes",
  );
}

#[test]
fn sequential_child_threads_start_at_zero_and_do_not_clobber_main_errno() {
  write_errno(5150);

  let main_addr = __errno_location() as usize;

  for index in 0..10_i32 {
    let main_value = 5150 + index;

    write_errno(main_value);

    let (child_addr, child_initial, child_final) = thread::spawn(move || {
      let child_addr = __errno_location() as usize;
      let child_initial = read_errno();
      let child_value = 7000 + index;

      write_errno(child_value);

      (child_addr, child_initial, read_errno())
    })
    .join()
    .expect("child thread panicked");

    assert_eq!(
      child_initial, 0,
      "new child thread must start with zero errno on each creation",
    );
    assert_eq!(
      child_final,
      7000 + index,
      "child thread must observe its own errno write",
    );
    assert_ne!(
      child_addr, main_addr,
      "child thread must not alias main-thread errno storage",
    );
    assert_eq!(
      __errno_location() as usize,
      main_addr,
      "main-thread errno pointer must remain stable across sequential children",
    );
    assert_eq!(
      read_errno(),
      main_value,
      "child thread writes must not clobber the main-thread errno value",
    );
  }
}

#[test]
fn child_errno_pointer_stays_stable_while_main_errno_changes() {
  write_errno(808);

  let main_addr = __errno_location() as usize;
  let (child_sender, main_receiver) = mpsc::channel();
  let (main_sender, child_receiver) = mpsc::channel();
  let child = thread::spawn(move || {
    let first_addr = __errno_location() as usize;
    let first_initial = read_errno();

    child_sender
      .send((first_addr, first_initial))
      .expect("send initial child sample");
    child_receiver
      .recv()
      .expect("receive main continuation signal");

    let second_addr = __errno_location() as usize;

    write_errno(1701);

    let third_addr = __errno_location() as usize;
    let child_final = read_errno();

    (second_addr, third_addr, child_final)
  });
  let (child_first_addr, child_first_initial) =
    main_receiver.recv().expect("receive initial child sample");

  write_errno(909);
  main_sender
    .send(())
    .expect("send continuation signal to child");

  let (child_second_addr, child_third_addr, child_final) =
    child.join().expect("child thread panicked");

  assert_eq!(
    child_first_initial, 0,
    "child thread errno must start at zero",
  );
  assert_ne!(
    child_first_addr, main_addr,
    "child thread must not alias main-thread errno storage",
  );
  assert_eq!(
    child_second_addr, child_first_addr,
    "child thread errno pointer must stay stable across main-thread updates",
  );
  assert_eq!(
    child_third_addr, child_first_addr,
    "child thread errno pointer must stay stable after child writes",
  );
  assert_eq!(
    child_final, 1701,
    "child thread must observe its own errno write",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable during child activity",
  );
  assert_eq!(
    read_errno(),
    909,
    "main-thread errno value must stay independent from child writes",
  );
}

#[test]
fn simultaneous_children_keep_stable_errno_pointer_within_each_thread() {
  const CHILD_COUNT: usize = 4;

  write_errno(333);

  let main_addr = __errno_location() as usize;
  let barrier = Arc::new(Barrier::new(CHILD_COUNT + 1));
  let (sender, receiver) = mpsc::channel();
  let mut handles = Vec::new();

  for index in 0..CHILD_COUNT {
    let barrier = Arc::clone(&barrier);
    let sender = sender.clone();

    handles.push(thread::spawn(move || {
      let before_addr = __errno_location() as usize;
      let initial = read_errno();
      let child_value = 3100 + usize_to_i32(index);

      barrier.wait();
      write_errno(child_value);
      barrier.wait();

      let after_addr = __errno_location() as usize;
      let final_value = read_errno();

      sender
        .send((before_addr, after_addr, initial, final_value))
        .expect("send child pointer/value sample");
    }));
  }

  drop(sender);
  barrier.wait();
  barrier.wait();

  let mut addresses = Vec::new();
  let mut finals = Vec::new();

  for (before_addr, after_addr, initial, final_value) in receiver {
    assert_eq!(initial, 0, "child thread errno must start at zero");
    assert_eq!(
      before_addr, after_addr,
      "child thread errno pointer must stay stable within the same thread",
    );
    assert_ne!(
      before_addr, main_addr,
      "child thread must not alias main-thread errno storage",
    );

    addresses.push(before_addr);
    finals.push(final_value);
  }

  for handle in handles {
    handle.join().expect("child thread panicked");
  }

  addresses.sort_unstable();
  addresses.dedup();
  assert_eq!(
    addresses.len(),
    CHILD_COUNT,
    "simultaneously live child threads must have distinct errno storage",
  );

  finals.sort_unstable();
  assert_eq!(
    finals,
    (0..CHILD_COUNT)
      .map(|index| 3100 + usize_to_i32(index))
      .collect::<Vec<_>>(),
    "child threads must preserve their own errno writes",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable during child activity",
  );
  assert_eq!(
    read_errno(),
    333,
    "main-thread errno value must stay unchanged across child writes",
  );
}

#[test]
fn errno_tls_regression_monitor_over_repeated_rounds() {
  const ROUNDS: usize = 12;
  const CHILD_COUNT: usize = 3;

  write_errno(4242);

  let main_addr = __errno_location() as usize;

  for round in 0..ROUNDS {
    let main_value = 4242 + usize_to_i32(round);

    write_errno(main_value);

    let barrier = Arc::new(Barrier::new(CHILD_COUNT + 1));
    let (sender, receiver) = mpsc::channel();
    let mut handles = Vec::new();

    for child_index in 0..CHILD_COUNT {
      let barrier = Arc::clone(&barrier);
      let sender = sender.clone();

      handles.push(thread::spawn(move || {
        let before_addr = __errno_location() as usize;
        let initial = read_errno();
        let child_value = 9000 + usize_to_i32(round) * 10 + usize_to_i32(child_index);

        barrier.wait();
        write_errno(child_value);

        let after_addr = __errno_location() as usize;
        let final_value = read_errno();

        sender
          .send((before_addr, after_addr, initial, final_value))
          .expect("send child regression sample");
      }));
    }

    drop(sender);
    barrier.wait();

    let mut child_addrs = Vec::new();
    let mut child_values = Vec::new();

    for (before_addr, after_addr, initial, final_value) in receiver {
      assert_eq!(
        initial, 0,
        "each child thread must start with zero errno on every round",
      );
      assert_eq!(
        before_addr, after_addr,
        "child-thread errno pointer must stay stable within a round",
      );
      assert_ne!(
        before_addr, main_addr,
        "child thread must not alias main-thread errno storage",
      );

      child_addrs.push(before_addr);
      child_values.push(final_value);
    }

    for handle in handles {
      handle.join().expect("child thread panicked");
    }

    child_addrs.sort_unstable();
    child_addrs.dedup();
    assert_eq!(
      child_addrs.len(),
      CHILD_COUNT,
      "simultaneously live children must keep distinct errno storage each round",
    );

    child_values.sort_unstable();
    assert_eq!(
      child_values,
      (0..CHILD_COUNT)
        .map(|index| 9000 + usize_to_i32(round) * 10 + usize_to_i32(index))
        .collect::<Vec<_>>(),
      "each child thread must preserve its own errno write per round",
    );
    assert_eq!(
      __errno_location() as usize,
      main_addr,
      "main-thread errno pointer must remain stable across all rounds",
    );
    assert_eq!(
      read_errno(),
      main_value,
      "main-thread errno value must remain independent across all rounds",
    );
  }
}

#[test]
fn errno_tls_regression_monitor_with_alternating_main_values() {
  let main_addr = __errno_location() as usize;
  let main_values = [-37, 0, 12, -91, 2048, -2048, 77];

  for (round, main_value) in main_values.into_iter().enumerate() {
    write_errno(main_value);

    let expected_child_value = 5000 + usize_to_i32(round);
    let (child_addr, child_initial, child_final) = thread::spawn(move || {
      let child_addr = __errno_location() as usize;
      let child_initial = read_errno();

      write_errno(expected_child_value);

      (child_addr, child_initial, read_errno())
    })
    .join()
    .expect("child thread panicked");

    assert_eq!(
      child_initial, 0,
      "newly spawned child thread must always start with zero errno",
    );
    assert_eq!(
      child_final, expected_child_value,
      "child thread must preserve its own errno write",
    );
    assert_ne!(
      child_addr, main_addr,
      "child thread must not alias main-thread errno storage",
    );
    assert_eq!(
      __errno_location() as usize,
      main_addr,
      "main-thread errno pointer must remain stable across alternating values",
    );
    assert_eq!(
      read_errno(),
      main_value,
      "child writes must not clobber main-thread errno across alternating values",
    );
  }
}

#[test]
fn child_errno_stays_zero_until_child_write_despite_main_updates() {
  write_errno(11);

  let main_addr = __errno_location() as usize;
  let sync = Arc::new(Barrier::new(2));
  let child_sync = Arc::clone(&sync);
  let (sender, receiver) = mpsc::channel();
  let child = thread::spawn(move || {
    let child_addr = __errno_location() as usize;
    let child_initial = read_errno();

    child_sync.wait();
    child_sync.wait();

    let child_before_write = read_errno();

    write_errno(777);

    sender
      .send((child_addr, child_initial, child_before_write, read_errno()))
      .expect("send child write sample");
  });

  sync.wait();

  let main_updates = [-5, 0, 44, -120];

  for value in main_updates {
    write_errno(value);
    assert_eq!(
      read_errno(),
      value,
      "main-thread errno write must be observable in main thread",
    );
  }

  sync.wait();
  child.join().expect("child thread panicked");

  let (child_addr, child_initial, child_before_write, child_final) =
    receiver.recv().expect("receive child write sample");

  assert_eq!(
    child_initial, 0,
    "new child thread must start with zero errno",
  );
  assert_eq!(
    child_before_write, 0,
    "child thread errno must stay zero before child performs any write",
  );
  assert_eq!(
    child_final, 777,
    "child thread must observe its own errno write",
  );
  assert_ne!(
    child_addr, main_addr,
    "child thread must not alias main-thread errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable across child activity",
  );
  assert_eq!(
    read_errno(),
    -120,
    "child writes must not clobber the latest main-thread errno value",
  );
}

#[test]
fn new_child_starts_zero_while_another_child_holds_nonzero_errno() {
  write_errno(300);

  let main_addr = __errno_location() as usize;
  let (first_ready_sender, first_ready_receiver) = mpsc::channel();
  let (release_sender, release_receiver) = mpsc::channel();
  let first_child = thread::spawn(move || {
    let first_addr = __errno_location() as usize;
    let first_initial = read_errno();

    write_errno(12345);

    let first_final = read_errno();

    first_ready_sender
      .send((first_addr, first_initial, first_final))
      .expect("send first child sample");
    release_receiver
      .recv()
      .expect("receive first child release signal");
  });
  let (first_addr, first_initial, first_final) = first_ready_receiver
    .recv()
    .expect("receive first child sample");

  write_errno(-77);

  let (second_addr, second_initial, second_final) = thread::spawn(|| {
    let second_addr = __errno_location() as usize;
    let second_initial = read_errno();

    write_errno(54321);

    (second_addr, second_initial, read_errno())
  })
  .join()
  .expect("second child thread panicked");

  release_sender
    .send(())
    .expect("send first child release signal");
  first_child.join().expect("first child thread panicked");

  assert_eq!(first_initial, 0, "first child must start with zero errno");
  assert_eq!(
    first_final, 12345,
    "first child must preserve its own errno write",
  );
  assert_eq!(
    second_initial, 0,
    "second child must start with zero errno even while first child is alive",
  );
  assert_eq!(
    second_final, 54321,
    "second child must preserve its own errno write",
  );
  assert_ne!(
    first_addr, main_addr,
    "first child must not alias main-thread errno storage",
  );
  assert_ne!(
    second_addr, main_addr,
    "second child must not alias main-thread errno storage",
  );
  assert_ne!(
    second_addr, first_addr,
    "simultaneously alive child threads must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable across child activity",
  );
  assert_eq!(
    read_errno(),
    -77,
    "child writes must not clobber main-thread errno",
  );
}

#[test]
fn next_child_starts_zero_after_live_child_and_main_updates() {
  write_errno(600);

  let main_addr = __errno_location() as usize;
  let start_sync = Arc::new(Barrier::new(2));
  let release_sync = Arc::new(Barrier::new(2));
  let child_start_sync = Arc::clone(&start_sync);
  let child_release_sync = Arc::clone(&release_sync);
  let first_child = thread::spawn(move || {
    let first_addr = __errno_location() as usize;
    let first_initial = read_errno();

    write_errno(1001);
    child_start_sync.wait();
    child_release_sync.wait();

    (first_addr, first_initial, read_errno())
  });

  start_sync.wait();

  for value in [-9, 7, 0, 88] {
    write_errno(value);
    assert_eq!(
      read_errno(),
      value,
      "main-thread errno write must be observable in main thread",
    );
  }

  release_sync.wait();

  let (first_addr, first_initial, first_final) =
    first_child.join().expect("first child thread panicked");
  let (second_addr, second_initial, second_final) = thread::spawn(|| {
    let second_addr = __errno_location() as usize;
    let second_initial = read_errno();

    write_errno(2002);

    (second_addr, second_initial, read_errno())
  })
  .join()
  .expect("second child thread panicked");

  assert_eq!(
    first_initial, 0,
    "first child thread must start with zero errno",
  );
  assert_eq!(
    first_final, 1001,
    "first child thread must keep its own errno despite main-thread updates",
  );
  assert_eq!(
    second_initial, 0,
    "newly spawned child after first child exit must start with zero errno",
  );
  assert_eq!(
    second_final, 2002,
    "second child thread must preserve its own errno write",
  );
  assert_ne!(
    first_addr, main_addr,
    "first child thread must not alias main-thread errno storage",
  );
  assert_ne!(
    second_addr, main_addr,
    "second child thread must not alias main-thread errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable across child lifetimes",
  );
  assert_eq!(
    read_errno(),
    88,
    "child writes must not clobber latest main-thread errno value",
  );
}

#[test]
fn main_and_child_errno_stay_isolated_across_interleaved_rounds() {
  const ROUNDS: usize = 5;

  write_errno(700);

  let main_addr = __errno_location() as usize;
  let start_sync = Arc::new(Barrier::new(2));
  let step_sync = Arc::new(Barrier::new(2));
  let child_start_sync = Arc::clone(&start_sync);
  let child_step_sync = Arc::clone(&step_sync);
  let child = thread::spawn(move || {
    let child_addr = __errno_location() as usize;
    let child_initial = read_errno();
    let mut expected = 41_i32;
    let mut snapshots = Vec::new();

    write_errno(expected);
    child_start_sync.wait();

    for _ in 0..ROUNDS {
      child_step_sync.wait();
      snapshots.push(read_errno());

      expected += 1;
      write_errno(expected);
      snapshots.push(read_errno());
      child_step_sync.wait();
    }

    (child_addr, child_initial, snapshots, expected)
  });

  start_sync.wait();

  for round in 0..ROUNDS {
    let main_value = -300 - usize_to_i32(round);

    write_errno(main_value);
    step_sync.wait();
    assert_eq!(
      read_errno(),
      main_value,
      "main-thread errno must remain unchanged before child round update",
    );
    step_sync.wait();
    assert_eq!(
      read_errno(),
      main_value,
      "main-thread errno must remain unchanged after child round update",
    );
  }

  let (child_addr, child_initial, snapshots, child_final) =
    child.join().expect("child thread panicked");

  assert_eq!(child_initial, 0, "child thread must start with zero errno");
  assert_ne!(
    child_addr, main_addr,
    "child thread must not alias main-thread errno storage",
  );
  assert_eq!(
    snapshots.len(),
    ROUNDS * 2,
    "expected two child snapshots per round",
  );

  let mut expected_before = 41_i32;

  for chunk in snapshots.chunks_exact(2) {
    assert_eq!(
      chunk[0], expected_before,
      "child errno before round write must match previous child value",
    );
    expected_before += 1;
    assert_eq!(
      chunk[1], expected_before,
      "child errno after round write must match child update",
    );
  }

  assert_eq!(
    child_final, expected_before,
    "child final errno must match last round update",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable across interleaved rounds",
  );
  assert_eq!(
    read_errno(),
    -300 - (usize_to_i32(ROUNDS) - 1),
    "child updates must not clobber final main-thread errno value",
  );
}

#[test]
fn two_children_and_main_remain_isolated_across_interleaved_rounds() {
  const ROUNDS: usize = 4;

  write_errno(8080);

  let main_addr = __errno_location() as usize;
  let begin_round = Arc::new(Barrier::new(3));
  let end_round = Arc::new(Barrier::new(3));
  let spawn_child = |base: i32,
                     begin_round: Arc<Barrier>,
                     end_round: Arc<Barrier>|
   -> thread::JoinHandle<(usize, i32, Vec<i32>, i32)> {
    thread::spawn(move || {
      let child_addr = __errno_location() as usize;
      let child_initial = read_errno();
      let mut expected_before = base;
      let mut snapshots = Vec::new();

      write_errno(expected_before);

      for _ in 0..ROUNDS {
        begin_round.wait();
        snapshots.push(read_errno());

        expected_before += 1;
        write_errno(expected_before);
        snapshots.push(read_errno());
        end_round.wait();
      }

      (child_addr, child_initial, snapshots, expected_before)
    })
  };
  let left_child = spawn_child(100, Arc::clone(&begin_round), Arc::clone(&end_round));
  let right_child = spawn_child(300, Arc::clone(&begin_round), Arc::clone(&end_round));

  for round in 0..ROUNDS {
    let main_value = -900 - usize_to_i32(round);

    write_errno(main_value);
    begin_round.wait();
    assert_eq!(
      read_errno(),
      main_value,
      "main-thread errno must stay isolated before child round updates",
    );
    end_round.wait();
    assert_eq!(
      read_errno(),
      main_value,
      "main-thread errno must stay isolated after child round updates",
    );
  }

  let (left_addr, left_initial, left_snapshots, left_final) =
    left_child.join().expect("child A panicked");
  let (right_addr, right_initial, right_snapshots, right_final) =
    right_child.join().expect("child B panicked");

  assert_eq!(left_initial, 0, "child A must start with zero errno");
  assert_eq!(right_initial, 0, "child B must start with zero errno");
  assert_ne!(
    left_addr, main_addr,
    "child A must not alias main-thread errno storage",
  );
  assert_ne!(
    right_addr, main_addr,
    "child B must not alias main-thread errno storage",
  );
  assert_ne!(
    left_addr, right_addr,
    "simultaneously live child threads must not share errno storage",
  );
  assert_eq!(
    left_snapshots.len(),
    ROUNDS * 2,
    "child A should provide two snapshots per round",
  );
  assert_eq!(
    right_snapshots.len(),
    ROUNDS * 2,
    "child B should provide two snapshots per round",
  );

  let mut expected_left = 100_i32;

  for chunk in left_snapshots.chunks_exact(2) {
    assert_eq!(
      chunk[0], expected_left,
      "child A pre-write errno must match its previous value",
    );
    expected_left += 1;
    assert_eq!(
      chunk[1], expected_left,
      "child A post-write errno must match its updated value",
    );
  }

  let mut expected_right = 300_i32;

  for chunk in right_snapshots.chunks_exact(2) {
    assert_eq!(
      chunk[0], expected_right,
      "child B pre-write errno must match its previous value",
    );
    expected_right += 1;
    assert_eq!(
      chunk[1], expected_right,
      "child B post-write errno must match its updated value",
    );
  }

  assert_eq!(
    left_final, expected_left,
    "child A final errno must match last round update",
  );
  assert_eq!(
    right_final, expected_right,
    "child B final errno must match last round update",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable across interleaved rounds",
  );
  assert_eq!(
    read_errno(),
    -900 - (usize_to_i32(ROUNDS) - 1),
    "child updates must not clobber the final main-thread errno value",
  );
}

#[test]
fn child_tls_storage_reuse_does_not_leak_errno_values() {
  let main_addr = __errno_location() as usize;

  for round in 0..16_i32 {
    let main_value = 1200 + round;

    write_errno(main_value);

    let (first_addr, first_final) = thread::spawn(move || {
      let first_addr = __errno_location() as usize;
      let first_value = 3000 + round;

      write_errno(first_value);

      (first_addr, read_errno())
    })
    .join()
    .expect("first child thread panicked");
    let (second_addr, second_initial) = thread::spawn(|| {
      let second_addr = __errno_location() as usize;

      (second_addr, read_errno())
    })
    .join()
    .expect("second child thread panicked");

    assert_eq!(
      first_final,
      3000 + round,
      "first child thread must preserve its own errno write",
    );
    assert_eq!(
      second_initial, 0,
      "new child thread must start with zero errno even if TLS storage gets reused",
    );
    assert_ne!(
      first_addr, main_addr,
      "first child thread must not alias main-thread errno storage",
    );
    assert_ne!(
      second_addr, main_addr,
      "second child thread must not alias main-thread errno storage",
    );
    assert_eq!(
      __errno_location() as usize,
      main_addr,
      "main-thread errno pointer must remain stable across child recreation",
    );
    assert_eq!(
      read_errno(),
      main_value,
      "child thread activity must not clobber main-thread errno",
    );
  }
}

#[test]
fn new_child_starts_zero_after_two_live_children_exit() {
  write_errno(910);

  let main_addr = __errno_location() as usize;
  let hold = Arc::new(Barrier::new(3));
  let release = Arc::new(Barrier::new(3));
  let spawn_live_child = |value: i32,
                          hold: Arc<Barrier>,
                          release: Arc<Barrier>|
   -> thread::JoinHandle<(usize, i32, i32)> {
    thread::spawn(move || {
      let child_addr = __errno_location() as usize;
      let child_initial = read_errno();

      hold.wait();
      write_errno(value);

      let child_final = read_errno();

      release.wait();

      (child_addr, child_initial, child_final)
    })
  };
  let first_live_child = spawn_live_child(1111, Arc::clone(&hold), Arc::clone(&release));
  let second_live_child = spawn_live_child(2222, Arc::clone(&hold), Arc::clone(&release));

  hold.wait();
  write_errno(-901);
  release.wait();

  let first_result = first_live_child.join().expect("child A panicked");
  let second_result = second_live_child.join().expect("child B panicked");
  let next_result = thread::spawn(|| {
    let next_thread_addr = __errno_location() as usize;
    let next_thread_initial = read_errno();

    write_errno(3333);

    (next_thread_addr, next_thread_initial, read_errno())
  })
  .join()
  .expect("child C panicked");
  let (first_addr, first_initial, first_final) = first_result;
  let (second_addr, second_initial, second_final) = second_result;
  let (next_addr, next_initial, next_final) = next_result;

  assert_eq!(first_initial, 0, "child A must start with zero errno");
  assert_eq!(second_initial, 0, "child B must start with zero errno");
  assert_eq!(
    next_initial, 0,
    "new child after prior live children exit must start with zero errno",
  );
  assert_eq!(first_final, 1111, "child A must preserve its own errno");
  assert_eq!(second_final, 2222, "child B must preserve its own errno");
  assert_eq!(next_final, 3333, "child C must preserve its own errno");
  assert_ne!(
    first_addr, main_addr,
    "child A must not alias main-thread errno storage",
  );
  assert_ne!(
    second_addr, main_addr,
    "child B must not alias main-thread errno storage",
  );
  assert_ne!(
    next_addr, main_addr,
    "child C must not alias main-thread errno storage",
  );
  assert_ne!(
    first_addr, second_addr,
    "simultaneously live child threads must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable after child lifetimes",
  );
  assert_eq!(
    read_errno(),
    -901,
    "child writes must not clobber latest main-thread errno value",
  );
}

#[test]
fn child_pointer_stays_stable_across_interleaved_main_updates() {
  const ROUNDS: usize = 6;

  write_errno(444);

  let main_addr = __errno_location() as usize;
  let sync = Arc::new(Barrier::new(2));
  let child_sync = Arc::clone(&sync);
  let child = thread::spawn(move || {
    let child_addr = __errno_location() as usize;
    let child_initial = read_errno();
    let mut pointer_samples = Vec::new();
    let mut value_samples = Vec::new();
    let mut child_expected = 8000_i32;
    let mut before_expected = 0_i32;

    for _ in 0..ROUNDS {
      child_sync.wait();

      let before_addr = __errno_location() as usize;
      let before_value = read_errno();

      child_expected += 1;
      write_errno(child_expected);

      let after_addr = __errno_location() as usize;
      let after_value = read_errno();

      pointer_samples.push((before_addr, after_addr));
      value_samples.push((before_value, after_value));
      before_expected = child_expected;
      child_sync.wait();
    }

    (
      child_addr,
      child_initial,
      pointer_samples,
      value_samples,
      before_expected,
    )
  });

  for round in 0..ROUNDS {
    let main_value = -500 - usize_to_i32(round);

    write_errno(main_value);
    sync.wait();
    assert_eq!(
      read_errno(),
      main_value,
      "main-thread errno must remain isolated before child update",
    );
    sync.wait();
    assert_eq!(
      read_errno(),
      main_value,
      "main-thread errno must remain isolated after child update",
    );
  }

  let (child_addr, child_initial, pointer_samples, value_samples, child_final) =
    child.join().expect("child thread panicked");

  assert_eq!(child_initial, 0, "child thread must start with zero errno");
  assert_ne!(
    child_addr, main_addr,
    "child thread must not alias main-thread errno storage",
  );
  assert_eq!(
    pointer_samples.len(),
    ROUNDS,
    "expected one pair of child pointer samples per round",
  );
  assert_eq!(
    value_samples.len(),
    ROUNDS,
    "expected one pair of child value samples per round",
  );

  let mut before_expected = 0_i32;
  let mut after_expected = 8000_i32;

  for (round, ((before_addr, after_addr), (before_value, after_value))) in
    pointer_samples.iter().zip(value_samples.iter()).enumerate()
  {
    assert_eq!(
      *before_addr, child_addr,
      "child pointer before write must remain stable (round {round})",
    );
    assert_eq!(
      *after_addr, child_addr,
      "child pointer after write must remain stable (round {round})",
    );
    assert_eq!(
      *before_value, before_expected,
      "child value before write must match previous child write (round {round})",
    );

    after_expected += 1;
    assert_eq!(
      *after_value, after_expected,
      "child value after write must match current child write (round {round})",
    );
    before_expected = after_expected;
  }

  assert_eq!(
    child_final, before_expected,
    "child final errno must match final child write",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable across interleaved updates",
  );
  assert_eq!(
    read_errno(),
    -500 - (usize_to_i32(ROUNDS) - 1),
    "child updates must not clobber final main-thread errno value",
  );
}

#[test]
fn child_zero_start_persists_across_repeated_multi_child_generations() {
  let main_addr = __errno_location() as usize;

  for round in 0..10_i32 {
    let main_value = 200 + round;

    write_errno(main_value);

    let start = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(3));
    let spawn_live_child = |value: i32,
                            start: Arc<Barrier>,
                            release: Arc<Barrier>|
     -> thread::JoinHandle<(usize, i32, i32)> {
      thread::spawn(move || {
        let child_addr = __errno_location() as usize;
        let child_initial = read_errno();

        start.wait();
        write_errno(value);

        let child_final = read_errno();

        release.wait();

        (child_addr, child_initial, child_final)
      })
    };
    let first_live_child =
      spawn_live_child(4000 + round * 10, Arc::clone(&start), Arc::clone(&release));
    let second_live_child =
      spawn_live_child(5000 + round * 10, Arc::clone(&start), Arc::clone(&release));

    start.wait();
    release.wait();

    let first_result = first_live_child.join().expect("child A panicked");
    let second_result = second_live_child.join().expect("child B panicked");
    let next_result = thread::spawn(move || {
      let next_thread_addr = __errno_location() as usize;
      let next_thread_initial = read_errno();
      let next_thread_value = 6000 + round * 10;

      write_errno(next_thread_value);

      (next_thread_addr, next_thread_initial, read_errno())
    })
    .join()
    .expect("child C panicked");
    let (first_addr, first_initial, first_final) = first_result;
    let (second_addr, second_initial, second_final) = second_result;
    let (next_addr, next_initial, next_final) = next_result;

    assert_eq!(first_initial, 0, "child A must start with zero errno");
    assert_eq!(second_initial, 0, "child B must start with zero errno");
    assert_eq!(
      next_initial, 0,
      "child C must start with zero errno after A/B thread lifetimes",
    );
    assert_eq!(
      first_final,
      4000 + round * 10,
      "child A must preserve its own errno write",
    );
    assert_eq!(
      second_final,
      5000 + round * 10,
      "child B must preserve its own errno write",
    );
    assert_eq!(
      next_final,
      6000 + round * 10,
      "child C must preserve its own errno write",
    );
    assert_ne!(
      first_addr, second_addr,
      "simultaneously live child A/B threads must not share errno storage",
    );
    assert_ne!(
      first_addr, main_addr,
      "child A must not alias main-thread errno storage",
    );
    assert_ne!(
      second_addr, main_addr,
      "child B must not alias main-thread errno storage",
    );
    assert_ne!(
      next_addr, main_addr,
      "child C must not alias main-thread errno storage",
    );
    assert_eq!(
      __errno_location() as usize,
      main_addr,
      "main-thread errno pointer must remain stable across generations",
    );
    assert_eq!(
      read_errno(),
      main_value,
      "child-thread activity must not clobber main-thread errno",
    );
  }
}

#[test]
fn boundary_errno_values_do_not_leak_across_live_children_and_next_spawn() {
  write_errno(-1);

  let main_addr = __errno_location() as usize;
  let hold = Arc::new(Barrier::new(3));
  let release = Arc::new(Barrier::new(3));
  let spawn_live_child = |value: c_int,
                          hold: Arc<Barrier>,
                          release: Arc<Barrier>|
   -> thread::JoinHandle<(usize, c_int, c_int)> {
    thread::spawn(move || {
      let child_addr = __errno_location() as usize;
      let child_initial = read_errno();

      hold.wait();
      write_errno(value);

      let child_final = read_errno();

      release.wait();

      (child_addr, child_initial, child_final)
    })
  };
  let lower_child = spawn_live_child(c_int::MIN, Arc::clone(&hold), Arc::clone(&release));
  let upper_child = spawn_live_child(c_int::MAX, Arc::clone(&hold), Arc::clone(&release));

  hold.wait();
  write_errno(77);
  release.wait();

  let lower_result = lower_child.join().expect("min child thread panicked");
  let upper_result = upper_child.join().expect("max child thread panicked");
  let next_result = thread::spawn(|| {
    let next_thread_storage = __errno_location() as usize;
    let next_thread_initial = read_errno();

    write_errno(5555);

    (next_thread_storage, next_thread_initial, read_errno())
  })
  .join()
  .expect("next child thread panicked");
  let (lower_storage, lower_initial_errno, lower_final_errno) = lower_result;
  let (upper_storage, upper_initial_errno, upper_final_errno) = upper_result;
  let (next_storage, next_initial_errno, next_final_errno) = next_result;

  assert_eq!(
    lower_initial_errno, 0,
    "min child must start with zero errno"
  );
  assert_eq!(
    upper_initial_errno, 0,
    "max child must start with zero errno"
  );
  assert_eq!(
    next_initial_errno, 0,
    "new child after boundary-value children must start with zero errno",
  );
  assert_eq!(
    lower_final_errno,
    c_int::MIN,
    "min child must preserve c_int::MIN"
  );
  assert_eq!(
    upper_final_errno,
    c_int::MAX,
    "max child must preserve c_int::MAX"
  );
  assert_eq!(
    next_final_errno, 5555,
    "next child must preserve its own errno write"
  );
  assert_ne!(
    lower_storage, main_addr,
    "min child must not alias main-thread errno storage",
  );
  assert_ne!(
    upper_storage, main_addr,
    "max child must not alias main-thread errno storage",
  );
  assert_ne!(
    next_storage, main_addr,
    "next child must not alias main-thread errno storage",
  );
  assert_ne!(
    lower_storage, upper_storage,
    "simultaneously live boundary children must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable across boundary-value children",
  );
  assert_eq!(
    read_errno(),
    77,
    "child writes must not clobber latest main-thread errno value",
  );
}

#[test]
fn zero_reset_in_main_does_not_leak_into_live_children_or_next_child_start() {
  write_errno(123);

  let main_addr = __errno_location() as usize;
  let start = Arc::new(Barrier::new(3));
  let release = Arc::new(Barrier::new(3));
  let spawn_live_child = |value: c_int,
                          start: Arc<Barrier>,
                          release: Arc<Barrier>|
   -> thread::JoinHandle<(usize, c_int, c_int)> {
    thread::spawn(move || {
      let child_addr = __errno_location() as usize;
      let child_initial = read_errno();

      start.wait();
      write_errno(value);
      release.wait();

      (child_addr, child_initial, read_errno())
    })
  };
  let first_live_child = spawn_live_child(7001, Arc::clone(&start), Arc::clone(&release));
  let second_live_child = spawn_live_child(-7002, Arc::clone(&start), Arc::clone(&release));

  start.wait();

  for value in [111, -222, 0] {
    write_errno(value);
    assert_eq!(
      read_errno(),
      value,
      "main-thread errno update must stay visible in main thread",
    );
  }

  release.wait();

  let first_result = first_live_child.join().expect("child A thread panicked");
  let second_result = second_live_child.join().expect("child B thread panicked");
  let next_result = thread::spawn(|| {
    let next_thread_storage = __errno_location() as usize;

    (next_thread_storage, read_errno())
  })
  .join()
  .expect("child C thread panicked");
  let (first_storage, first_initial, first_final) = first_result;
  let (second_storage, second_initial, second_final) = second_result;
  let (next_storage, next_initial) = next_result;

  assert_eq!(first_initial, 0, "child A must start with zero errno");
  assert_eq!(second_initial, 0, "child B must start with zero errno");
  assert_eq!(
    next_initial, 0,
    "new child after live children exit must start with zero errno",
  );
  assert_eq!(first_final, 7001, "child A must preserve its own errno");
  assert_eq!(second_final, -7002, "child B must preserve its own errno");
  assert_ne!(
    first_storage, main_addr,
    "child A must not alias main-thread errno storage",
  );
  assert_ne!(
    second_storage, main_addr,
    "child B must not alias main-thread errno storage",
  );
  assert_ne!(
    next_storage, main_addr,
    "child C must not alias main-thread errno storage",
  );
  assert_ne!(
    first_storage, second_storage,
    "simultaneously live children must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_addr,
    "main-thread errno pointer must remain stable through reset sequence",
  );
  assert_eq!(
    read_errno(),
    0,
    "child writes must not clobber final main-thread errno reset to zero",
  );
}

#[test]
fn grandchild_thread_starts_zero_and_stays_isolated_from_parent_and_main() {
  write_errno(1212);

  let main_storage = __errno_location() as usize;
  let parent_result = thread::spawn(|| {
    let parent_storage = __errno_location() as usize;
    let parent_initial_errno = read_errno();

    write_errno(8181);

    let grandchild_result = thread::spawn(|| {
      let grandchild_storage = __errno_location() as usize;
      let grandchild_initial_errno = read_errno();

      write_errno(-9191);

      (grandchild_storage, grandchild_initial_errno, read_errno())
    })
    .join()
    .expect("grandchild thread panicked");
    let parent_final_errno = read_errno();

    (
      parent_storage,
      parent_initial_errno,
      parent_final_errno,
      grandchild_result,
    )
  })
  .join()
  .expect("parent thread panicked");
  let (parent_storage, parent_initial_errno, parent_final_errno, grandchild_result) = parent_result;
  let (grandchild_storage, grandchild_initial_errno, grandchild_final_errno) = grandchild_result;

  assert_eq!(
    parent_initial_errno, 0,
    "parent child thread must start with zero errno",
  );
  assert_eq!(
    grandchild_initial_errno, 0,
    "grandchild thread must start with zero errno",
  );
  assert_eq!(
    parent_final_errno, 8181,
    "grandchild writes must not clobber parent thread errno",
  );
  assert_eq!(
    grandchild_final_errno, -9191,
    "grandchild thread must preserve its own errno write",
  );
  assert_ne!(
    parent_storage, main_storage,
    "parent child thread must not alias main-thread errno storage",
  );
  assert_ne!(
    grandchild_storage, main_storage,
    "grandchild thread must not alias main-thread errno storage",
  );
  assert_ne!(
    grandchild_storage, parent_storage,
    "parent and grandchild threads must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_storage,
    "main-thread errno pointer must remain stable after nested child lifetimes",
  );
  assert_eq!(
    read_errno(),
    1212,
    "nested child writes must not clobber main-thread errno",
  );
}

#[test]
fn sequential_grandchildren_from_same_parent_start_zero_each_time() {
  write_errno(2020);

  let main_storage = __errno_location() as usize;
  let parent_summary = thread::spawn(|| {
    let parent_storage = __errno_location() as usize;
    let parent_initial_errno = read_errno();

    write_errno(3030);

    let prior_spawn = thread::spawn(|| {
      let prior_storage = __errno_location() as usize;
      let prior_initial_errno = read_errno();

      write_errno(4141);

      (prior_storage, prior_initial_errno, read_errno())
    })
    .join()
    .expect("prior grandchild thread panicked");
    let parent_after_prior = read_errno();
    let fresh_spawn = thread::spawn(|| {
      let fresh_storage = __errno_location() as usize;
      let fresh_initial_errno = read_errno();

      write_errno(-5151);

      (fresh_storage, fresh_initial_errno, read_errno())
    })
    .join()
    .expect("fresh grandchild thread panicked");
    let parent_final_errno = read_errno();

    (
      parent_storage,
      parent_initial_errno,
      parent_after_prior,
      parent_final_errno,
      prior_spawn,
      fresh_spawn,
    )
  })
  .join()
  .expect("parent thread panicked");
  let (
    parent_storage,
    parent_initial_errno,
    parent_after_prior,
    parent_final_errno,
    prior_spawn,
    fresh_spawn,
  ) = parent_summary;
  let (prior_storage, prior_initial_errno, prior_final_errno) = prior_spawn;
  let (fresh_storage, fresh_initial_errno, fresh_final_errno) = fresh_spawn;

  assert_eq!(
    parent_initial_errno, 0,
    "parent child thread must start with zero errno",
  );
  assert_eq!(
    prior_initial_errno, 0,
    "first grandchild thread must start with zero errno",
  );
  assert_eq!(
    fresh_initial_errno, 0,
    "second grandchild thread must also start with zero errno",
  );
  assert_eq!(
    prior_final_errno, 4141,
    "first grandchild thread must preserve its own errno write",
  );
  assert_eq!(
    fresh_final_errno, -5151,
    "second grandchild thread must preserve its own errno write",
  );
  assert_eq!(
    parent_after_prior, 3030,
    "first grandchild writes must not clobber parent thread errno",
  );
  assert_eq!(
    parent_final_errno, 3030,
    "second grandchild writes must not clobber parent thread errno",
  );
  assert_ne!(
    parent_storage, main_storage,
    "parent child thread must not alias main-thread errno storage",
  );
  assert_ne!(
    prior_storage, main_storage,
    "first grandchild thread must not alias main-thread errno storage",
  );
  assert_ne!(
    fresh_storage, main_storage,
    "second grandchild thread must not alias main-thread errno storage",
  );
  assert_ne!(
    prior_storage, parent_storage,
    "first grandchild thread must not share storage with parent thread",
  );
  assert_ne!(
    fresh_storage, parent_storage,
    "second grandchild thread must not share storage with parent thread",
  );
  assert_eq!(
    __errno_location() as usize,
    main_storage,
    "main-thread errno pointer must remain stable after nested child generations",
  );
  assert_eq!(
    read_errno(),
    2020,
    "nested child writes must not clobber main-thread errno",
  );
}

#[test]
fn concurrent_grandchildren_from_same_parent_do_not_clobber_parent_or_main_errno() {
  write_errno(-6060);

  let main_storage = __errno_location() as usize;
  let parent_summary = thread::spawn(|| {
    let parent_storage = __errno_location() as usize;
    let parent_initial_errno = read_errno();

    write_errno(7070);

    let sync_point = Arc::new(Barrier::new(3));
    let spawn_grandchild = |target_errno: c_int,
                            sync_point: Arc<Barrier>|
     -> thread::JoinHandle<(usize, c_int, c_int, c_int)> {
      thread::spawn(move || {
        let grandchild_storage = __errno_location() as usize;
        let grandchild_initial_errno = read_errno();

        sync_point.wait();

        let before_write_errno = read_errno();

        write_errno(target_errno);

        let after_write_errno = read_errno();

        sync_point.wait();

        (
          grandchild_storage,
          grandchild_initial_errno,
          before_write_errno,
          after_write_errno,
        )
      })
    };
    let blue_grandchild = spawn_grandchild(8081, Arc::clone(&sync_point));
    let amber_grandchild = spawn_grandchild(-8082, Arc::clone(&sync_point));

    sync_point.wait();

    let parent_during_grandchildren = read_errno();

    sync_point.wait();

    let blue_report = blue_grandchild
      .join()
      .expect("blue grandchild thread panicked");
    let amber_report = amber_grandchild
      .join()
      .expect("amber grandchild thread panicked");
    let parent_final_errno = read_errno();

    (
      parent_storage,
      parent_initial_errno,
      parent_during_grandchildren,
      parent_final_errno,
      blue_report,
      amber_report,
    )
  })
  .join()
  .expect("parent thread panicked");
  let (
    parent_storage,
    parent_initial_errno,
    parent_during_grandchildren,
    parent_final_errno,
    blue_report,
    amber_report,
  ) = parent_summary;
  let (blue_storage, blue_initial_errno, blue_before_write, blue_after_write) = blue_report;
  let (amber_storage, amber_initial_errno, amber_before_write, amber_after_write) = amber_report;

  assert_eq!(
    parent_initial_errno, 0,
    "parent child thread must start with zero errno",
  );
  assert_eq!(
    blue_initial_errno, 0,
    "blue grandchild thread must start with zero errno",
  );
  assert_eq!(
    amber_initial_errno, 0,
    "amber grandchild thread must start with zero errno",
  );
  assert_eq!(
    blue_before_write, 0,
    "blue grandchild errno before first write must remain zero",
  );
  assert_eq!(
    amber_before_write, 0,
    "amber grandchild errno before first write must remain zero",
  );
  assert_eq!(
    blue_after_write, 8081,
    "blue grandchild thread must preserve its own errno write",
  );
  assert_eq!(
    amber_after_write, -8082,
    "amber grandchild thread must preserve its own errno write",
  );
  assert_eq!(
    parent_during_grandchildren, 7070,
    "concurrent grandchildren must not clobber parent errno mid-flight",
  );
  assert_eq!(
    parent_final_errno, 7070,
    "concurrent grandchildren must not clobber parent errno after joins",
  );
  assert_ne!(
    parent_storage, main_storage,
    "parent child thread must not alias main-thread errno storage",
  );
  assert_ne!(
    blue_storage, main_storage,
    "blue grandchild thread must not alias main-thread errno storage",
  );
  assert_ne!(
    amber_storage, main_storage,
    "amber grandchild thread must not alias main-thread errno storage",
  );
  assert_ne!(
    blue_storage, parent_storage,
    "blue grandchild thread must not share storage with parent thread",
  );
  assert_ne!(
    amber_storage, parent_storage,
    "amber grandchild thread must not share storage with parent thread",
  );
  assert_ne!(
    blue_storage, amber_storage,
    "simultaneously live grandchildren must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_storage,
    "main-thread errno pointer must remain stable after concurrent grandchildren",
  );
  assert_eq!(
    read_errno(),
    -6060,
    "concurrent grandchild writes must not clobber main-thread errno",
  );
}

#[test]
fn new_grandchild_after_concurrent_grandchildren_starts_zero() {
  write_errno(9090);

  let main_storage = __errno_location() as usize;
  let parent_summary = thread::spawn(|| {
    let parent_storage = __errno_location() as usize;
    let parent_initial_errno = read_errno();

    write_errno(-9190);

    let gate = Arc::new(Barrier::new(3));
    let spawn_nested =
      |target_errno: c_int, gate: Arc<Barrier>| -> thread::JoinHandle<(usize, c_int, c_int)> {
        thread::spawn(move || {
          let nested_storage = __errno_location() as usize;
          let nested_initial_errno = read_errno();

          gate.wait();
          write_errno(target_errno);
          gate.wait();

          (nested_storage, nested_initial_errno, read_errno())
        })
      };
    let left_nested = spawn_nested(1112, Arc::clone(&gate));
    let right_nested = spawn_nested(-1113, Arc::clone(&gate));

    gate.wait();

    let parent_during_concurrency = read_errno();

    gate.wait();

    let left_report = left_nested.join().expect("left nested thread panicked");
    let right_report = right_nested.join().expect("right nested thread panicked");
    let post_nested_report = thread::spawn(|| {
      let post_storage = __errno_location() as usize;
      let post_initial_errno = read_errno();

      write_errno(1213);

      (post_storage, post_initial_errno, read_errno())
    })
    .join()
    .expect("post nested thread panicked");
    let parent_final_errno = read_errno();

    (
      parent_storage,
      parent_initial_errno,
      parent_during_concurrency,
      parent_final_errno,
      left_report,
      right_report,
      post_nested_report,
    )
  })
  .join()
  .expect("parent thread panicked");
  let (
    parent_storage,
    parent_initial_errno,
    parent_during_concurrency,
    parent_final_errno,
    left_report,
    right_report,
    post_nested_report,
  ) = parent_summary;
  let (left_storage, left_initial_errno, left_final_errno) = left_report;
  let (right_storage, right_initial_errno, right_final_errno) = right_report;
  let (post_storage, post_initial_errno, post_final_errno) = post_nested_report;

  assert_eq!(
    parent_initial_errno, 0,
    "parent child thread must start with zero errno",
  );
  assert_eq!(
    left_initial_errno, 0,
    "left nested thread must start with zero errno",
  );
  assert_eq!(
    right_initial_errno, 0,
    "right nested thread must start with zero errno",
  );
  assert_eq!(
    post_initial_errno, 0,
    "new nested thread after concurrent pair must also start with zero errno",
  );
  assert_eq!(
    left_final_errno, 1112,
    "left nested thread must preserve its own errno write",
  );
  assert_eq!(
    right_final_errno, -1113,
    "right nested thread must preserve its own errno write",
  );
  assert_eq!(
    post_final_errno, 1213,
    "post nested thread must preserve its own errno write",
  );
  assert_eq!(
    parent_during_concurrency, -9190,
    "concurrent nested threads must not clobber parent errno mid-flight",
  );
  assert_eq!(
    parent_final_errno, -9190,
    "nested thread writes must not clobber parent errno",
  );
  assert_ne!(
    parent_storage, main_storage,
    "parent child thread must not alias main-thread errno storage",
  );
  assert_ne!(
    left_storage, main_storage,
    "left nested thread must not alias main-thread errno storage",
  );
  assert_ne!(
    right_storage, main_storage,
    "right nested thread must not alias main-thread errno storage",
  );
  assert_ne!(
    post_storage, main_storage,
    "post nested thread must not alias main-thread errno storage",
  );
  assert_ne!(
    left_storage, right_storage,
    "simultaneously live nested threads must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_storage,
    "main-thread errno pointer must remain stable after nested generations",
  );
  assert_eq!(
    read_errno(),
    9090,
    "nested thread writes must not clobber main-thread errno",
  );
}

#[test]
fn sibling_parent_trees_keep_errno_isolation_across_nested_threads() {
  write_errno(1313);

  let main_storage = __errno_location() as usize;
  let ready_gate = Arc::new(Barrier::new(3));
  let release_gate = Arc::new(Barrier::new(3));
  let nested_gate = Arc::new(Barrier::new(2));
  let spawn_parent_tree = |parent_target: c_int,
                           nested_target: c_int,
                           ready_gate: Arc<Barrier>,
                           release_gate: Arc<Barrier>,
                           nested_gate: Arc<Barrier>|
   -> thread::JoinHandle<ParentThreadSample> {
    thread::spawn(move || {
      let parent_storage = __errno_location() as usize;
      let parent_initial_errno = read_errno();

      write_errno(parent_target);
      ready_gate.wait();

      let nested_report = thread::spawn(move || {
        let nested_storage = __errno_location() as usize;
        let nested_initial_errno = read_errno();

        nested_gate.wait();
        write_errno(nested_target);
        nested_gate.wait();

        (nested_storage, nested_initial_errno, read_errno())
      })
      .join()
      .expect("nested thread panicked");
      let parent_final_errno = read_errno();

      release_gate.wait();
      (
        parent_storage,
        parent_initial_errno,
        parent_final_errno,
        nested_report,
      )
    })
  };
  let aurora_handle = spawn_parent_tree(
    2221,
    3331,
    Arc::clone(&ready_gate),
    Arc::clone(&release_gate),
    Arc::clone(&nested_gate),
  );
  let zephyr_handle = spawn_parent_tree(
    -2222,
    -3332,
    Arc::clone(&ready_gate),
    Arc::clone(&release_gate),
    Arc::clone(&nested_gate),
  );

  ready_gate.wait();
  assert_eq!(
    read_errno(),
    1313,
    "sibling parent tree activity must not clobber main-thread errno",
  );
  release_gate.wait();

  let aurora_report = aurora_handle.join().expect("aurora parent thread panicked");
  let zephyr_report = zephyr_handle.join().expect("zephyr parent thread panicked");
  let (aurora_parent_storage, aurora_parent_initial, aurora_parent_final, aurora_nested_report) =
    aurora_report;
  let (zephyr_parent_storage, zephyr_parent_initial, zephyr_parent_final, zephyr_nested_report) =
    zephyr_report;
  let (aurora_nested_storage, aurora_nested_initial, aurora_nested_final) = aurora_nested_report;
  let (zephyr_nested_storage, zephyr_nested_initial, zephyr_nested_final) = zephyr_nested_report;

  assert_eq!(
    aurora_parent_initial, 0,
    "aurora parent thread must start with zero errno",
  );
  assert_eq!(
    zephyr_parent_initial, 0,
    "zephyr parent thread must start with zero errno",
  );
  assert_eq!(
    aurora_nested_initial, 0,
    "aurora nested thread must start with zero errno",
  );
  assert_eq!(
    zephyr_nested_initial, 0,
    "zephyr nested thread must start with zero errno",
  );
  assert_eq!(
    aurora_parent_final, 2221,
    "aurora nested writes must not clobber aurora parent errno",
  );
  assert_eq!(
    zephyr_parent_final, -2222,
    "zephyr nested writes must not clobber zephyr parent errno",
  );
  assert_eq!(
    aurora_nested_final, 3331,
    "aurora nested thread must preserve its own errno write",
  );
  assert_eq!(
    zephyr_nested_final, -3332,
    "zephyr nested thread must preserve its own errno write",
  );
  assert_ne!(
    aurora_parent_storage, main_storage,
    "aurora parent thread must not alias main-thread errno storage",
  );
  assert_ne!(
    zephyr_parent_storage, main_storage,
    "zephyr parent thread must not alias main-thread errno storage",
  );
  assert_ne!(
    aurora_nested_storage, main_storage,
    "aurora nested thread must not alias main-thread errno storage",
  );
  assert_ne!(
    zephyr_nested_storage, main_storage,
    "zephyr nested thread must not alias main-thread errno storage",
  );
  assert_ne!(
    aurora_parent_storage, zephyr_parent_storage,
    "simultaneously live parent threads must not share errno storage",
  );
  assert_ne!(
    aurora_nested_storage, zephyr_nested_storage,
    "simultaneously live nested threads must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_storage,
    "main-thread errno pointer must remain stable after sibling parent trees",
  );
  assert_eq!(
    read_errno(),
    1313,
    "nested sibling tree writes must not clobber main-thread errno",
  );
}

#[test]
fn child_starts_zero_after_sibling_parent_trees_exit() {
  write_errno(-4040);

  let main_storage = __errno_location() as usize;
  let ready_gate = Arc::new(Barrier::new(3));
  let spawn_tree = |parent_target: c_int, nested_target: c_int, ready_gate: Arc<Barrier>| {
    thread::spawn(move || {
      let parent_storage = __errno_location() as usize;
      let parent_initial_errno = read_errno();

      write_errno(parent_target);

      let nested_report = thread::spawn(move || {
        let nested_storage = __errno_location() as usize;
        let nested_initial_errno = read_errno();

        write_errno(nested_target);

        (nested_storage, nested_initial_errno, read_errno())
      })
      .join()
      .expect("nested thread panicked");
      let parent_final_errno = read_errno();

      ready_gate.wait();
      (
        parent_storage,
        parent_initial_errno,
        parent_final_errno,
        nested_report,
      )
    })
  };
  let atlas_handle = spawn_tree(5050, 6060, Arc::clone(&ready_gate));
  let boreal_handle = spawn_tree(-5051, -6061, Arc::clone(&ready_gate));

  ready_gate.wait();
  assert_eq!(
    read_errno(),
    -4040,
    "sibling parent tree activity must not clobber main-thread errno",
  );

  let atlas_report = atlas_handle.join().expect("atlas parent thread panicked");
  let boreal_report = boreal_handle.join().expect("boreal parent thread panicked");
  let post_report = thread::spawn(|| {
    let post_storage = __errno_location() as usize;
    let post_initial_errno = read_errno();

    write_errno(7070);

    (post_storage, post_initial_errno, read_errno())
  })
  .join()
  .expect("post child thread panicked");
  let (atlas_parent_storage, atlas_parent_initial, atlas_parent_final, atlas_nested_report) =
    atlas_report;
  let (boreal_parent_storage, boreal_parent_initial, boreal_parent_final, boreal_nested_report) =
    boreal_report;
  let (atlas_nested_storage, atlas_nested_initial, atlas_nested_final) = atlas_nested_report;
  let (boreal_nested_storage, boreal_nested_initial, boreal_nested_final) = boreal_nested_report;
  let (post_storage, post_initial_errno, post_final_errno) = post_report;

  assert_eq!(
    atlas_parent_initial, 0,
    "atlas parent thread must start with zero errno",
  );
  assert_eq!(
    boreal_parent_initial, 0,
    "boreal parent thread must start with zero errno",
  );
  assert_eq!(
    atlas_nested_initial, 0,
    "atlas nested thread must start with zero errno",
  );
  assert_eq!(
    boreal_nested_initial, 0,
    "boreal nested thread must start with zero errno",
  );
  assert_eq!(
    post_initial_errno, 0,
    "new child after sibling parent trees must start with zero errno",
  );
  assert_eq!(
    atlas_parent_final, 5050,
    "atlas nested writes must not clobber atlas parent errno",
  );
  assert_eq!(
    boreal_parent_final, -5051,
    "boreal nested writes must not clobber boreal parent errno",
  );
  assert_eq!(
    atlas_nested_final, 6060,
    "atlas nested thread must preserve its own errno write",
  );
  assert_eq!(
    boreal_nested_final, -6061,
    "boreal nested thread must preserve its own errno write",
  );
  assert_eq!(
    post_final_errno, 7070,
    "post child thread must preserve its own errno write",
  );
  assert_ne!(
    atlas_parent_storage, main_storage,
    "atlas parent thread must not alias main-thread errno storage",
  );
  assert_ne!(
    boreal_parent_storage, main_storage,
    "boreal parent thread must not alias main-thread errno storage",
  );
  assert_ne!(
    atlas_nested_storage, main_storage,
    "atlas nested thread must not alias main-thread errno storage",
  );
  assert_ne!(
    boreal_nested_storage, main_storage,
    "boreal nested thread must not alias main-thread errno storage",
  );
  assert_ne!(
    post_storage, main_storage,
    "post child thread must not alias main-thread errno storage",
  );
  assert_ne!(
    atlas_parent_storage, boreal_parent_storage,
    "simultaneously live parent threads must not share errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_storage,
    "main-thread errno pointer must remain stable after sibling parent trees",
  );
  assert_eq!(
    read_errno(),
    -4040,
    "sibling parent tree and post child writes must not clobber main-thread errno",
  );
}

#[test]
fn nested_child_starts_zero_after_main_updates_before_spawn() {
  write_errno(1717);

  let main_storage = __errno_location() as usize;
  let gate_ready = Arc::new(Barrier::new(2));
  let gate_release = Arc::new(Barrier::new(2));
  let trunk_handle = {
    let gate_ready = Arc::clone(&gate_ready);
    let gate_release = Arc::clone(&gate_release);

    thread::spawn(move || {
      let trunk_storage = __errno_location() as usize;
      let trunk_initial_errno = read_errno();

      write_errno(2626);
      gate_ready.wait();
      gate_release.wait();

      let sprout_report = thread::spawn(|| {
        let sprout_storage = __errno_location() as usize;
        let sprout_initial_errno = read_errno();

        write_errno(-3737);

        (sprout_storage, sprout_initial_errno, read_errno())
      })
      .join()
      .expect("sprout thread panicked");
      let trunk_final_errno = read_errno();

      (
        trunk_storage,
        trunk_initial_errno,
        trunk_final_errno,
        sprout_report,
      )
    })
  };

  gate_ready.wait();

  for main_value in [88, -99, 0, 1234] {
    write_errno(main_value);
    assert_eq!(
      read_errno(),
      main_value,
      "main-thread errno updates must remain visible before nested child spawn",
    );
  }

  gate_release.wait();

  let trunk_report = trunk_handle.join().expect("trunk thread panicked");
  let after_report = thread::spawn(|| {
    let after_storage = __errno_location() as usize;
    let after_initial_errno = read_errno();

    write_errno(4848);

    (after_storage, after_initial_errno, read_errno())
  })
  .join()
  .expect("after thread panicked");
  let (trunk_storage, trunk_initial_errno, trunk_final_errno, sprout_report) = trunk_report;
  let (sprout_storage, sprout_initial_errno, sprout_final_errno) = sprout_report;
  let (after_storage, after_initial_errno, after_final_errno) = after_report;

  assert_eq!(
    trunk_initial_errno, 0,
    "trunk thread must start with zero errno"
  );
  assert_eq!(
    sprout_initial_errno, 0,
    "nested child thread must start with zero errno",
  );
  assert_eq!(
    after_initial_errno, 0,
    "new child thread after trunk exit must start with zero errno",
  );
  assert_eq!(
    trunk_final_errno, 2626,
    "nested child writes must not clobber trunk errno",
  );
  assert_eq!(
    sprout_final_errno, -3737,
    "nested child thread must preserve its own errno write",
  );
  assert_eq!(
    after_final_errno, 4848,
    "new child thread must preserve its own errno write",
  );
  assert_ne!(
    trunk_storage, main_storage,
    "trunk thread must not alias main-thread errno storage",
  );
  assert_ne!(
    sprout_storage, main_storage,
    "nested child thread must not alias main-thread errno storage",
  );
  assert_ne!(
    after_storage, main_storage,
    "new child thread must not alias main-thread errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_storage,
    "main-thread errno pointer must remain stable across pre-spawn updates",
  );
  assert_eq!(
    read_errno(),
    1234,
    "trunk and child writes must not clobber latest main-thread errno value",
  );
}

#[test]
fn nested_boundary_errno_values_stay_isolated_from_parent_and_main() {
  write_errno(c_int::MAX - 1);

  let main_storage = __errno_location() as usize;
  let trunk_report = thread::spawn(|| {
    let trunk_storage = __errno_location() as usize;
    let trunk_initial_errno = read_errno();

    write_errno(c_int::MIN + 11);

    let sprout_report = thread::spawn(|| {
      let sprout_storage = __errno_location() as usize;
      let sprout_initial_errno = read_errno();

      write_errno(c_int::MAX - 13);

      (sprout_storage, sprout_initial_errno, read_errno())
    })
    .join()
    .expect("sprout thread panicked");
    let trunk_final_errno = read_errno();

    (
      trunk_storage,
      trunk_initial_errno,
      trunk_final_errno,
      sprout_report,
    )
  })
  .join()
  .expect("trunk thread panicked");
  let post_report = thread::spawn(|| {
    let post_storage = __errno_location() as usize;
    let post_initial_errno = read_errno();

    write_errno(-7777);

    (post_storage, post_initial_errno, read_errno())
  })
  .join()
  .expect("post thread panicked");
  let (trunk_storage, trunk_initial_errno, trunk_final_errno, sprout_report) = trunk_report;
  let (sprout_storage, sprout_initial_errno, sprout_final_errno) = sprout_report;
  let (post_storage, post_initial_errno, post_final_errno) = post_report;

  assert_eq!(
    trunk_initial_errno, 0,
    "trunk thread must start with zero errno"
  );
  assert_eq!(
    sprout_initial_errno, 0,
    "nested sprout thread must start with zero errno",
  );
  assert_eq!(
    post_initial_errno, 0,
    "new child thread after nested boundary writes must start with zero errno",
  );
  assert_eq!(
    trunk_final_errno,
    c_int::MIN + 11,
    "nested sprout writes must not clobber trunk errno",
  );
  assert_eq!(
    sprout_final_errno,
    c_int::MAX - 13,
    "nested sprout thread must preserve its boundary errno write",
  );
  assert_eq!(
    post_final_errno, -7777,
    "post child thread must preserve its own errno write",
  );
  assert_ne!(
    trunk_storage, main_storage,
    "trunk thread must not alias main-thread errno storage",
  );
  assert_ne!(
    sprout_storage, main_storage,
    "nested sprout thread must not alias main-thread errno storage",
  );
  assert_ne!(
    post_storage, main_storage,
    "post child thread must not alias main-thread errno storage",
  );
  assert_eq!(
    __errno_location() as usize,
    main_storage,
    "main-thread errno pointer must remain stable across nested boundary writes",
  );
  assert_eq!(
    read_errno(),
    c_int::MAX - 1,
    "nested boundary writes must not clobber main-thread errno",
  );
}

#[test]
fn repeated_nested_rounds_keep_nested_and_fresh_child_zero_start() {
  let main_storage = __errno_location() as usize;

  for round in 0..6_i32 {
    let main_value = -800 + round;

    write_errno(main_value);

    let trunk_report = thread::spawn(move || {
      let trunk_storage = __errno_location() as usize;
      let trunk_initial_errno = read_errno();
      let trunk_target = 1200 + round;

      write_errno(trunk_target);

      let nested_report = thread::spawn(move || {
        let nested_storage = __errno_location() as usize;
        let nested_initial_errno = read_errno();

        write_errno(2200 + round);

        (nested_storage, nested_initial_errno, read_errno())
      })
      .join()
      .expect("nested thread panicked");
      let trunk_final_errno = read_errno();

      (
        trunk_storage,
        trunk_initial_errno,
        trunk_final_errno,
        nested_report,
      )
    })
    .join()
    .expect("trunk thread panicked");
    let fresh_report = thread::spawn(|| {
      let fresh_storage = __errno_location() as usize;
      let fresh_initial_errno = read_errno();

      (fresh_storage, fresh_initial_errno)
    })
    .join()
    .expect("fresh child thread panicked");
    let (trunk_storage, trunk_initial_errno, trunk_final_errno, nested_report) = trunk_report;
    let (nested_storage, nested_initial_errno, nested_final_errno) = nested_report;
    let (fresh_storage, fresh_initial_errno) = fresh_report;

    assert_eq!(
      trunk_initial_errno, 0,
      "trunk thread must start with zero errno"
    );
    assert_eq!(
      nested_initial_errno, 0,
      "nested thread must start with zero errno in each round",
    );
    assert_eq!(
      fresh_initial_errno, 0,
      "fresh child after nested round must start with zero errno",
    );
    assert_eq!(
      trunk_final_errno,
      1200 + round,
      "nested writes must not clobber trunk thread errno",
    );
    assert_eq!(
      nested_final_errno,
      2200 + round,
      "nested thread must preserve its own errno write",
    );
    assert_ne!(
      trunk_storage, main_storage,
      "trunk thread must not alias main-thread errno storage",
    );
    assert_ne!(
      nested_storage, main_storage,
      "nested thread must not alias main-thread errno storage",
    );
    assert_ne!(
      fresh_storage, main_storage,
      "fresh child thread must not alias main-thread errno storage",
    );
    assert_eq!(
      __errno_location() as usize,
      main_storage,
      "main-thread errno pointer must remain stable across repeated nested rounds",
    );
    assert_eq!(
      read_errno(),
      main_value,
      "nested and fresh child activity must not clobber main-thread errno per round",
    );
  }
}

#[test]
fn nested_thread_pointer_remains_stable_across_repeated_writes() {
  const REPEATS: usize = 5;

  write_errno(-111);

  let main_slot = __errno_location() as usize;
  let stem_report = thread::spawn(|| {
    let stem_slot = __errno_location() as usize;
    let stem_zero = read_errno();

    write_errno(5151);

    let bud_report = thread::spawn(|| {
      let bud_slot = __errno_location() as usize;
      let bud_zero = read_errno();
      let mut slot_pairs = Vec::new();
      let mut value_pairs = Vec::new();
      let mut running = 9000_i32;

      for _ in 0..REPEATS {
        let left_ptr = __errno_location() as usize;
        let before_errno = read_errno();

        running += 1;
        write_errno(running);

        let right_ptr = __errno_location() as usize;
        let after_errno = read_errno();

        slot_pairs.push((left_ptr, right_ptr));
        value_pairs.push((before_errno, after_errno));
      }

      (bud_slot, bud_zero, slot_pairs, value_pairs, running)
    })
    .join()
    .expect("bud thread panicked");
    let stem_steady = read_errno();

    (stem_slot, stem_zero, stem_steady, bud_report)
  })
  .join()
  .expect("stem thread panicked");
  let (stem_slot, stem_zero, stem_steady, bud_report) = stem_report;
  let (bud_slot, bud_zero, slot_pairs, value_pairs, bud_end) = bud_report;

  assert_eq!(stem_zero, 0, "stem thread must start with zero errno");
  assert_eq!(bud_zero, 0, "bud thread must start with zero errno");
  assert_eq!(
    stem_steady, 5151,
    "bud writes must not clobber stem thread errno",
  );
  assert_eq!(
    slot_pairs.len(),
    REPEATS,
    "expected one pointer pair per nested write",
  );
  assert_eq!(
    value_pairs.len(),
    REPEATS,
    "expected one value pair per nested write",
  );

  let mut prior_value = 0_i32;
  let mut next_target = 9000_i32;

  for ((left_ptr, right_ptr), (before_errno, after_errno)) in
    slot_pairs.iter().zip(value_pairs.iter())
  {
    assert_eq!(
      *left_ptr, bud_slot,
      "nested pointer before write must remain stable",
    );
    assert_eq!(
      *right_ptr, bud_slot,
      "nested pointer after write must remain stable",
    );
    assert_eq!(
      *before_errno, prior_value,
      "nested errno before write must match previous nested value",
    );
    next_target += 1;
    assert_eq!(
      *after_errno, next_target,
      "nested errno after write must match current nested value",
    );
    prior_value = next_target;
  }

  assert_eq!(
    bud_end, prior_value,
    "nested final errno must match final nested write",
  );
  assert_ne!(
    stem_slot, main_slot,
    "stem thread must not alias main-thread errno storage",
  );
  assert_ne!(
    bud_slot, main_slot,
    "bud thread must not alias main-thread errno storage",
  );
  assert_ne!(
    bud_slot, stem_slot,
    "nested bud thread must not share errno storage with stem thread",
  );
  assert_eq!(
    __errno_location() as usize,
    main_slot,
    "main-thread errno pointer must remain stable across nested writes",
  );
  assert_eq!(
    read_errno(),
    -111,
    "nested writes must not clobber main-thread errno",
  );
}

#[test]
fn three_level_nested_threads_keep_errno_zero_start_and_isolation() {
  write_errno(77);

  let main_slot = __errno_location() as usize;
  let trunk_report = thread::spawn(|| {
    let trunk_slot = __errno_location() as usize;
    let trunk_zero = read_errno();

    write_errno(1001);

    let branch_report = thread::spawn(|| {
      let branch_slot = __errno_location() as usize;
      let branch_zero = read_errno();

      write_errno(2002);

      let leaf_report = thread::spawn(|| {
        let leaf_slot = __errno_location() as usize;
        let leaf_zero = read_errno();

        write_errno(-3003);

        (leaf_slot, leaf_zero, read_errno())
      })
      .join()
      .expect("leaf thread panicked");
      let branch_final = read_errno();

      (branch_slot, branch_zero, branch_final, leaf_report)
    })
    .join()
    .expect("branch thread panicked");
    let trunk_final = read_errno();

    (trunk_slot, trunk_zero, trunk_final, branch_report)
  })
  .join()
  .expect("trunk thread panicked");
  let (trunk_slot, trunk_zero, trunk_final, branch_report) = trunk_report;
  let (branch_slot, branch_zero, branch_final, leaf_report) = branch_report;
  let (leaf_slot, leaf_zero, leaf_final) = leaf_report;

  assert_eq!(trunk_zero, 0, "trunk thread must start with zero errno");
  assert_eq!(branch_zero, 0, "branch thread must start with zero errno");
  assert_eq!(leaf_zero, 0, "leaf thread must start with zero errno");
  assert_eq!(
    trunk_final, 1001,
    "branch/leaf writes must not clobber trunk errno",
  );
  assert_eq!(
    branch_final, 2002,
    "leaf writes must not clobber branch errno",
  );
  assert_eq!(
    leaf_final, -3003,
    "leaf thread must preserve its own errno write"
  );
  assert_ne!(
    trunk_slot, main_slot,
    "trunk thread must not alias main-thread errno storage",
  );
  assert_ne!(
    branch_slot, main_slot,
    "branch thread must not alias main-thread errno storage",
  );
  assert_ne!(
    leaf_slot, main_slot,
    "leaf thread must not alias main-thread errno storage",
  );
  assert_ne!(
    branch_slot, trunk_slot,
    "branch thread must not share errno storage with trunk thread",
  );
  assert_ne!(
    leaf_slot, branch_slot,
    "leaf thread must not share errno storage with branch thread",
  );
  assert_eq!(
    __errno_location() as usize,
    main_slot,
    "main-thread errno pointer must remain stable across three-level nesting",
  );
  assert_eq!(
    read_errno(),
    77,
    "nested writes must not clobber main-thread errno"
  );
}
