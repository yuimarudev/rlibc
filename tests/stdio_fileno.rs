use core::ffi::c_int;
use rlibc::abi::errno::EINVAL;
use rlibc::errno::__errno_location;
use rlibc::stdio::{FILE, fclose, fileno, fileno_unlocked};

unsafe extern "C" {
  #[link_name = "stdin"]
  static mut host_stdin: *mut FILE;
  #[link_name = "stdout"]
  static mut host_stdout: *mut FILE;
  #[link_name = "stderr"]
  static mut host_stderr: *mut FILE;
  fn tmpfile() -> *mut FILE;
}

fn read_errno() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage for `errno`.
  unsafe { errno_ptr.read() }
}

fn write_errno(value: c_int) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage for `errno`.
  unsafe { errno_ptr.write(value) };
}

#[test]
fn fileno_symbols_match_c_abi_signatures() {
  let _: unsafe extern "C" fn(*mut FILE) -> c_int = fileno;
  let _: unsafe extern "C" fn(*mut FILE) -> c_int = fileno_unlocked;
}

#[test]
fn fileno_standard_streams_match_unlocked_variant_and_preserve_errno() {
  let checks = [
    // SAFETY: host stdio globals are valid process-lifetime `FILE*` values.
    ("stdin", unsafe { host_stdin }, 0),
    // SAFETY: host stdio globals are valid process-lifetime `FILE*` values.
    ("stdout", unsafe { host_stdout }, 1),
    // SAFETY: host stdio globals are valid process-lifetime `FILE*` values.
    ("stderr", unsafe { host_stderr }, 2),
  ];

  for (index, (label, stream, expected_fd)) in checks.into_iter().enumerate() {
    let index_offset =
      c_int::try_from(index).unwrap_or_else(|_| unreachable!("test index must fit c_int"));
    let preserved_errno = 4100 + index_offset;

    write_errno(preserved_errno);

    // SAFETY: host standard streams are valid `FILE*`.
    let locked_fd = unsafe { fileno(stream) };
    // SAFETY: host standard streams are valid `FILE*`.
    let unlocked_fd = unsafe { fileno_unlocked(stream) };

    assert_eq!(
      locked_fd, expected_fd,
      "{label} should resolve to fd {expected_fd}"
    );
    assert_eq!(
      unlocked_fd, expected_fd,
      "{label} unlocked lookup should resolve to fd {expected_fd}",
    );
    assert_eq!(
      locked_fd, unlocked_fd,
      "{label} fileno variants should agree"
    );
    assert_eq!(
      read_errno(),
      preserved_errno,
      "{label} descriptor lookup should preserve errno",
    );
  }
}

#[test]
fn fileno_tmpfile_matches_unlocked_variant_and_preserves_errno() {
  // SAFETY: host libc provides `tmpfile` and returns either null or a valid `FILE*`.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile should yield a valid host stream"
  );

  write_errno(4200);

  // SAFETY: `stream` was returned by host `tmpfile`.
  let locked_fd = unsafe { fileno(stream) };
  // SAFETY: `stream` was returned by host `tmpfile`.
  let unlocked_fd = unsafe { fileno_unlocked(stream) };

  assert!(locked_fd >= 0, "tmpfile stream should expose a descriptor");
  assert_eq!(
    locked_fd, unlocked_fd,
    "fileno_unlocked should match fileno for tmpfile streams",
  );
  assert_eq!(
    read_errno(),
    4200,
    "successful tmpfile descriptor lookup should preserve errno",
  );

  // SAFETY: `stream` originated from host `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(
    close_status, 0,
    "tmpfile stream should be closable through rlibc fclose"
  );
}

#[test]
fn fileno_null_stream_returns_einval_for_locked_and_unlocked_variants() {
  write_errno(4300);

  // SAFETY: null stream is intentionally exercised for the wrapper's error path.
  let locked_fd = unsafe { fileno(core::ptr::null_mut()) };

  assert_eq!(locked_fd, -1, "fileno(null) should fail safely");
  assert_eq!(
    read_errno(),
    EINVAL,
    "fileno(null) should set errno = EINVAL"
  );

  write_errno(4301);

  // SAFETY: null stream is intentionally exercised for the wrapper's error path.
  let unlocked_fd = unsafe { fileno_unlocked(core::ptr::null_mut()) };

  assert_eq!(unlocked_fd, -1, "fileno_unlocked(null) should fail safely");
  assert_eq!(
    read_errno(),
    EINVAL,
    "fileno_unlocked(null) should set errno = EINVAL",
  );
}
