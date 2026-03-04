use core::ffi::c_void;
use core::ptr::null_mut;
use rlibc::abi::errno::EINVAL;
use rlibc::abi::types::{c_int, c_long, ssize_t};
use rlibc::errno::__errno_location;
use rlibc::stdio::{_IONBF, EOF, FILE, fflush, setvbuf, vfprintf};
use std::sync::{Mutex, MutexGuard, OnceLock};

const SEEK_SET: c_int = 0;

#[repr(C)]
struct SysVVaList {
  gp_offset: u32,
  fp_offset: u32,
  overflow_arg_area: *mut c_void,
  reg_save_area: *mut c_void,
}

unsafe extern "C" {
  fn close(fd: c_int) -> c_int;
  fn fclose(stream: *mut FILE) -> c_int;
  fn fileno(stream: *mut FILE) -> c_int;
  fn fputs(s: *const i8, stream: *mut FILE) -> c_int;
  fn lseek(fd: c_int, offset: c_long, whence: c_int) -> c_long;
  fn read(fd: c_int, buf: *mut c_void, count: usize) -> ssize_t;
  fn tmpfile() -> *mut FILE;
  #[link_name = "stdin"]
  static mut host_stdin: *mut FILE;
  #[link_name = "stdout"]
  static mut host_stdout: *mut FILE;
  #[link_name = "stderr"]
  static mut host_stderr: *mut FILE;
}

fn as_expected_fflush_signature(
  function: unsafe extern "C" fn(*mut FILE) -> c_int,
) -> unsafe extern "C" fn(*mut FILE) -> c_int {
  function
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` must return a valid pointer for the current thread.
  unsafe { *__errno_location() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` must return a valid writable pointer for the current thread.
  unsafe {
    *__errno_location() = value;
  }
}

fn test_lock() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  match LOCK.get_or_init(|| Mutex::new(())).lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

#[test]
fn fflush_symbol_matches_c_abi_signature() {
  let signature = as_expected_fflush_signature(fflush);
  let _ = signature;
}

#[test]
fn eof_constant_is_negative() {
  const {
    assert!(EOF < 0);
  };
}

#[test]
fn fflush_null_stream_returns_success_when_no_streams_are_registered() {
  let _guard = test_lock();

  write_errno(0);

  // SAFETY: C contract allows null stream pointer for `fflush(NULL)`.
  let result = unsafe { fflush(null_mut()) };

  assert_eq!(result, 0);
}

#[test]
fn fflush_null_stream_is_idempotent_without_registered_streams() {
  let _guard = test_lock();

  write_errno(0);

  // SAFETY: C contract allows null stream pointer for `fflush(NULL)`.
  let first = unsafe { fflush(null_mut()) };
  // SAFETY: C contract allows null stream pointer for `fflush(NULL)`.
  let second = unsafe { fflush(null_mut()) };

  assert_eq!(first, 0);
  assert_eq!(second, 0);
}

#[test]
fn fflush_null_stream_success_path_does_not_clobber_errno() {
  let _guard = test_lock();

  write_errno(123);

  // SAFETY: C contract allows null stream pointer for `fflush(NULL)`.
  let result = unsafe { fflush(null_mut()) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 123);
}

#[test]
fn fflush_unregistered_non_null_stream_is_noop_success() {
  let _guard = test_lock();
  let mut marker = 0_u8;
  let stream = core::ptr::from_mut(&mut marker).cast::<FILE>();

  write_errno(77);

  // SAFETY: `stream` is intentionally an unregistered pointer to verify the
  // minimal implementation's no-op contract.
  let result = unsafe { fflush(stream) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 77);
}

#[test]
fn fflush_unregistered_stream_blocks_late_setvbuf() {
  let _guard = test_lock();
  let mut marker = 0_u8;
  let stream = core::ptr::from_mut(&mut marker).cast::<FILE>();
  let mut user_buffer = [0_u8; 8];

  write_errno(55);

  // SAFETY: stream pointer is stable for this call.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 55);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fflush_registered_stream_is_idempotent_and_blocks_late_setvbuf() {
  let _guard = test_lock();
  let mut marker = 0_u8;
  let stream = core::ptr::from_mut(&mut marker).cast::<FILE>();
  let mut user_buffer = [0_u8; 8];

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let initial_setvbuf = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(initial_setvbuf, 0);

  write_errno(44);

  // SAFETY: stream pointer is stable for this call.
  let first_flush = unsafe { fflush(stream) };
  // SAFETY: stream pointer is stable for this call.
  let second_flush = unsafe { fflush(stream) };

  assert_eq!(first_flush, 0);
  assert_eq!(second_flush, 0);
  assert_eq!(read_errno(), 44);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let late_setvbuf = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(late_setvbuf, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fflush_host_backed_stream_flushes_pending_output() {
  let _guard = test_lock();
  let payload = b"i022-host-backed\n\0";
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: null_mut(),
    reg_save_area: null_mut(),
  };

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile must provide a writable host stream"
  );

  // SAFETY: `fileno` expects a valid host stream.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "tmpfile stream must expose a file descriptor");

  write_errno(81);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(
    write_status >= 0,
    "host-backed write through rlibc::vfprintf must succeed",
  );
  assert_eq!(
    read_errno(),
    81,
    "successful write must preserve rlibc errno"
  );

  write_errno(73);

  // SAFETY: stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 73);

  // SAFETY: file descriptor is valid and seekable.
  let seek_status = unsafe { lseek(fd, 0, SEEK_SET) };

  assert_eq!(seek_status, 0, "rewinding flushed stream fd must succeed");

  let mut read_back = [0_u8; 64];
  // SAFETY: read buffer is writable and descriptor is valid.
  let read_status = unsafe { read(fd, read_back.as_mut_ptr().cast(), read_back.len()) };

  assert!(
    read_status > 0,
    "fflush(stream) must flush host-backed buffered data",
  );

  let read_len =
    usize::try_from(read_status).unwrap_or_else(|_| unreachable!("positive ssize_t fits usize"));

  assert_eq!(&read_back[..read_len], &payload[..payload.len() - 1]);

  // SAFETY: stream came from `tmpfile` and must be closed to release resources.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0, "closing tmpfile stream must succeed");
}

#[test]
fn fflush_host_backed_stream_failure_sets_errno_and_blocks_late_setvbuf() {
  let _guard = test_lock();
  let payload = b"i022-host-backed-failure\n\0";
  let mut user_buffer = [0_u8; 16];
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: null_mut(),
    reg_save_area: null_mut(),
  };

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile must provide a writable host stream"
  );

  // SAFETY: `fileno` expects a valid host stream.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "tmpfile stream must expose a file descriptor");

  write_errno(41);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming host-backed stream must succeed");
  assert_eq!(read_errno(), 41);

  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(fd) };

  assert_eq!(close_status, 0, "closing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, EOF);
  assert_ne!(read_errno(), 0, "fflush(stream) failure must set errno");

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, `fclose` is still required to release FILE state.
  let _ = unsafe { fclose(stream) };
}

#[test]
fn fflush_null_stream_returns_eof_when_any_host_stream_flush_fails() {
  let _guard = test_lock();
  let payload = b"i022-flush-all\n\0";

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let valid_stream = unsafe { tmpfile() };

  assert!(
    !valid_stream.is_null(),
    "tmpfile for valid stream must succeed"
  );
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let valid_write = unsafe { fputs(payload.as_ptr().cast(), valid_stream) };

  assert!(valid_write >= 0, "priming valid stream must succeed");
  // SAFETY: stream and payload pointers are valid for host fputs.
  let failing_write = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(failing_write >= 0, "priming failing stream must succeed");

  // SAFETY: `fileno` expects a valid host `FILE*`.
  let valid_fd = unsafe { fileno(valid_stream) };

  assert!(valid_fd >= 0, "valid stream must have an fd");
  // SAFETY: `fileno` expects a valid host `FILE*`.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all output streams.
  let flush_status = unsafe { fflush(null_mut()) };

  assert_eq!(flush_status, EOF);
  assert_ne!(read_errno(), 0);

  // SAFETY: valid file descriptor seek/read after flush-all to verify other streams still flush.
  let seek_status = unsafe { lseek(valid_fd, 0, SEEK_SET) };

  assert_eq!(seek_status, 0, "seeking valid stream fd must succeed");

  let mut read_back = [0_u8; 64];
  // SAFETY: read target buffer is writable and fd is expected readable.
  let read_status = unsafe { read(valid_fd, read_back.as_mut_ptr().cast(), read_back.len()) };

  assert!(
    read_status > 0,
    "fflush(NULL) must flush other valid streams despite one failure",
  );

  let read_len =
    usize::try_from(read_status).unwrap_or_else(|_| unreachable!("positive ssize_t fits usize"));

  assert_eq!(&read_back[..read_len], &payload[..payload.len() - 1]);

  // SAFETY: valid stream must be closed to release host resources.
  let valid_close = unsafe { fclose(valid_stream) };

  assert_eq!(valid_close, 0, "closing valid stream must succeed");
  // SAFETY: even after injected fd close, `fclose` is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn fflush_null_failure_still_marks_stdout_as_io_active_for_late_setvbuf() {
  let _guard = test_lock();
  let payload = b"i022-flush-all-failure\n\0";
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let failing_write = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(failing_write >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host `FILE*`.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all output streams.
  let flush_status = unsafe { fflush(null_mut()) };

  assert_eq!(flush_status, EOF);
  assert_ne!(read_errno(), 0);

  write_errno(0);
  // SAFETY: host libc provides `stdout` global stream pointer.
  let stdout_stream = unsafe { host_stdout };

  assert!(
    !stdout_stream.is_null(),
    "host stdout pointer must be available"
  );

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status =
    unsafe { setvbuf(stdout_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, `fclose` is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn fflush_null_failure_still_marks_stderr_as_io_active_for_late_setvbuf() {
  let _guard = test_lock();
  let payload = b"i022-flush-all-failure-stderr\n\0";
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let failing_write = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(failing_write >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host `FILE*`.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all output streams.
  let flush_status = unsafe { fflush(null_mut()) };

  assert_eq!(flush_status, EOF);
  assert_ne!(read_errno(), 0);

  write_errno(0);
  // SAFETY: host libc provides `stderr` global stream pointer.
  let stderr_stream = unsafe { host_stderr };

  assert!(
    !stderr_stream.is_null(),
    "host stderr pointer must be available"
  );

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status =
    unsafe { setvbuf(stderr_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, `fclose` is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn fflush_null_failure_still_marks_stdin_as_io_active_for_late_setvbuf() {
  let _guard = test_lock();
  let payload = b"i022-flush-all-failure-stdin\n\0";
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let failing_write = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(failing_write >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host `FILE*`.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all output streams.
  let flush_status = unsafe { fflush(null_mut()) };

  assert_eq!(flush_status, EOF);
  assert_ne!(read_errno(), 0);

  write_errno(0);
  // SAFETY: host libc provides `stdin` global stream pointer.
  let stdin_stream = unsafe { host_stdin };

  assert!(
    !stdin_stream.is_null(),
    "host stdin pointer must be available"
  );

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status = unsafe { setvbuf(stdin_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, `fclose` is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn fflush_null_failure_marks_tracked_stream_active_for_late_setvbuf() {
  let _guard = test_lock();
  let payload = b"i022-flush-all-failure-tracked\n\0";
  let mut marker = 0_u8;
  let tracked_stream = core::ptr::from_mut(&mut marker).cast::<FILE>();
  let mut initial_buffer = [0_u8; 8];
  let mut replacement_buffer = [0_u8; 16];

  write_errno(0);

  // SAFETY: tracked stream and buffer pointers are valid for this call.
  let initial_setvbuf = unsafe {
    setvbuf(
      tracked_stream,
      initial_buffer.as_mut_ptr().cast(),
      _IONBF,
      0,
    )
  };

  assert_eq!(initial_setvbuf, 0);

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let failing_write = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(failing_write >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host `FILE*`.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all output streams.
  let flush_status = unsafe { fflush(null_mut()) };

  assert_eq!(flush_status, EOF);
  assert_ne!(read_errno(), 0);

  write_errno(0);

  // SAFETY: tracked stream and buffer pointers are valid for this call.
  let late_setvbuf = unsafe {
    setvbuf(
      tracked_stream,
      replacement_buffer.as_mut_ptr().cast(),
      _IONBF,
      0,
    )
  };

  assert_eq!(late_setvbuf, EOF);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, `fclose` is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn fflush_null_marks_stdout_as_io_active_for_late_setvbuf() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  write_errno(91);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 91);

  write_errno(0);
  // SAFETY: host libc provides `stdout` global stream pointer.
  let stdout_stream = unsafe { host_stdout };

  assert!(
    !stdout_stream.is_null(),
    "host stdout pointer must be available"
  );

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status =
    unsafe { setvbuf(stdout_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fflush_null_marks_stderr_as_io_active_for_late_setvbuf() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  write_errno(17);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 17);

  write_errno(0);
  // SAFETY: host libc provides `stderr` global stream pointer.
  let stderr_stream = unsafe { host_stderr };

  assert!(
    !stderr_stream.is_null(),
    "host stderr pointer must be available"
  );

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status =
    unsafe { setvbuf(stderr_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fflush_null_marks_stdin_as_io_active_for_late_setvbuf() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  write_errno(23);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 23);

  write_errno(0);
  // SAFETY: host libc provides `stdin` global stream pointer.
  let stdin_stream = unsafe { host_stdin };

  assert!(
    !stdin_stream.is_null(),
    "host stdin pointer must be available"
  );

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status = unsafe { setvbuf(stdin_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fflush_stderr_non_null_marks_stream_active_for_late_setvbuf() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides `stderr` global stream pointer.
  let stderr_stream = unsafe { host_stderr };

  assert!(
    !stderr_stream.is_null(),
    "host stderr pointer must be available"
  );

  write_errno(29);
  // SAFETY: host stderr pointer is valid for `fflush`.
  let flush_status = unsafe { fflush(stderr_stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 29);

  write_errno(0);
  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status =
    unsafe { setvbuf(stderr_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fflush_stdout_non_null_marks_stream_active_for_late_setvbuf() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides `stdout` global stream pointer.
  let stdout_stream = unsafe { host_stdout };

  assert!(
    !stdout_stream.is_null(),
    "host stdout pointer must be available"
  );

  write_errno(33);
  // SAFETY: host stdout pointer is valid for `fflush`.
  let flush_status = unsafe { fflush(stdout_stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 33);

  write_errno(0);
  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status =
    unsafe { setvbuf(stdout_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fflush_stdin_non_null_marks_stream_active_for_late_setvbuf() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides `stdin` global stream pointer.
  let stdin_stream = unsafe { host_stdin };

  assert!(
    !stdin_stream.is_null(),
    "host stdin pointer must be available"
  );

  write_errno(31);
  // SAFETY: host stdin pointer is valid for `fflush`.
  let flush_status = unsafe { fflush(stdin_stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 31);

  write_errno(0);
  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status = unsafe { setvbuf(stdin_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}
