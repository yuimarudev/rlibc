#![cfg(unix)]

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use rlibc::abi::errno::EINVAL;
use rlibc::abi::types::{c_char, c_int, c_long, size_t};
use rlibc::errno::__errno_location;
use rlibc::stdio::{_IOFBF, _IOLBF, _IONBF, FILE, fflush, fprintf, setvbuf, vfprintf};
use std::sync::{Mutex, MutexGuard, OnceLock};

const EOF_STATUS: c_int = -1;
const SEEK_END: c_int = 2;

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
  fn tmpfile() -> *mut FILE;
  #[link_name = "stdin"]
  static mut host_stdin: *mut FILE;
  #[link_name = "stdout"]
  static mut host_stdout: *mut FILE;
  #[link_name = "stderr"]
  static mut host_stderr: *mut FILE;
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns writable/readable thread-local storage.
  unsafe { __errno_location().read() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local storage.
  unsafe {
    __errno_location().write(value);
  }
}

const fn as_file_ptr(marker: &mut u8) -> *mut FILE {
  ptr::from_mut(marker).cast::<FILE>()
}

fn as_size_t(value: usize) -> size_t {
  size_t::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize does not fit in size_t on this target"))
}

fn synthetic_untracked_stream() -> *mut FILE {
  static NEXT_STREAM_ID: AtomicUsize = AtomicUsize::new(1);
  const BASE_ADDR: usize = 0x2000_0000_0000;
  const STRIDE: usize = 0x1000;
  let stream_id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);
  let stream_addr = BASE_ADDR.saturating_add(stream_id.saturating_mul(STRIDE));

  stream_addr as *mut FILE
}

fn test_lock() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  match LOCK.get_or_init(|| Mutex::new(())).lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

#[test]
fn setvbuf_rejects_null_stream_with_einval() {
  write_errno(0);

  // SAFETY: null pointer call is intentionally used to verify error handling.
  let status = unsafe { setvbuf(ptr::null_mut(), ptr::null_mut(), _IONBF, 0) };

  assert_eq!(status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_unknown_mode_with_einval() {
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);

  write_errno(0);

  // SAFETY: stream points to a stable marker for this call.
  let status = unsafe { setvbuf(stream, ptr::null_mut(), 999, 32) };

  assert_eq!(status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_zero_size_for_buffered_modes() {
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);
  let modes = [_IOFBF, _IOLBF];

  for mode in modes {
    write_errno(0);

    // SAFETY: stream points to a stable marker for this call.
    let status = unsafe { setvbuf(stream, ptr::null_mut(), mode, 0) };

    assert_eq!(status, EOF_STATUS, "mode={mode} must fail on zero size");
    assert_eq!(read_errno(), EINVAL, "mode={mode} must set EINVAL");
  }
}

#[test]
fn setvbuf_accepts_unbuffered_mode_with_null_buffer() {
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);

  write_errno(77);

  // SAFETY: stream points to a stable marker for this call.
  let status = unsafe { setvbuf(stream, ptr::null_mut(), _IONBF, 0) };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), 77);
}

#[test]
fn setvbuf_accepts_unbuffered_mode_with_non_zero_size() {
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);
  let mut user_buffer = [0_u8; 8];

  write_errno(66);

  // SAFETY: stream and buffer pointers are valid for this call.
  let status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast::<c_char>(),
      _IONBF,
      as_size_t(user_buffer.len()),
    )
  };

  assert_eq!(status, 0);
  assert_eq!(read_errno(), 66);
}

#[test]
fn setvbuf_unbuffered_mode_makes_vfprintf_output_immediately_observable() {
  let _guard = test_lock();
  let payload = b"i023-ionbf-immediate\0";
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };
  let mut stream = ptr::null_mut::<FILE>();
  let mut setvbuf_status = EOF_STATUS;
  let mut retry_count = 0_usize;
  let max_retry_count = 64_usize;
  let mut skipped_streams = Vec::new();

  while retry_count < max_retry_count {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    stream = unsafe { tmpfile() };
    assert!(
      !stream.is_null(),
      "tmpfile stream must be available for I023 test",
    );

    write_errno(17);

    // SAFETY: stream pointer is valid and unbuffered mode accepts null buffer.
    setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IONBF, 0) };

    if setvbuf_status == 0 {
      break;
    }

    skipped_streams.push(stream);

    retry_count = retry_count.saturating_add(1);
  }

  assert_eq!(
    setvbuf_status, 0,
    "setvbuf must succeed for a fresh tmpfile stream after retrying pointer reuse-prone host streams"
  );
  assert_eq!(read_errno(), 17);

  write_errno(19);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let written = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(written >= 0, "host-backed vfprintf write must succeed");
  assert_eq!(read_errno(), 19);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let payload_len = payload.len().saturating_sub(1);
  let expected_end = c_long::try_from(payload_len)
    .unwrap_or_else(|_| unreachable!("payload length must fit c_long"));

  assert_eq!(
    end_offset, expected_end,
    "unbuffered mode must make vfprintf output observable without explicit fflush",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);

  for skipped_stream in skipped_streams {
    // SAFETY: each stream came from `tmpfile` and remained open for this test.
    let skipped_close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(skipped_close_status, 0);
  }
}

#[test]
fn setvbuf_line_buffered_mode_flushes_newline_vfprintf_output() {
  let _guard = test_lock();
  let payload = b"i023-iolbf-newline\n\0";
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };
  let mut stream = ptr::null_mut::<FILE>();
  let mut setvbuf_status = EOF_STATUS;
  let mut retry_count = 0_usize;
  let max_retry_count = 64_usize;
  let mut skipped_streams = Vec::new();

  while retry_count < max_retry_count {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    stream = unsafe { tmpfile() };
    assert!(
      !stream.is_null(),
      "tmpfile stream must be available for I023 line-buffered test",
    );

    write_errno(23);

    // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
    setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

    if setvbuf_status == 0 {
      break;
    }

    skipped_streams.push(stream);

    retry_count = retry_count.saturating_add(1);
  }

  assert_eq!(
    setvbuf_status, 0,
    "setvbuf must succeed for a fresh tmpfile stream after retrying pointer reuse-prone host streams"
  );
  assert_eq!(read_errno(), 23);

  write_errno(29);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let written = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(
    written >= 0,
    "host-backed vfprintf write must succeed for line-buffered newline flush test",
  );
  assert_eq!(read_errno(), 29);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let payload_len = payload.len().saturating_sub(1);
  let expected_end = c_long::try_from(payload_len)
    .unwrap_or_else(|_| unreachable!("payload length must fit c_long"));

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode with newline must flush vfprintf output without explicit fflush",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);

  for skipped_stream in skipped_streams {
    // SAFETY: each stream came from `tmpfile` and remained open for this test.
    let skipped_close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(skipped_close_status, 0);
  }
}

#[test]
fn setvbuf_line_buffered_mode_flushes_percent_s_newline_payload() {
  let _guard = test_lock();
  let format = b"%s\0";
  let payload = b"i023-iolbf-percent-s-newline\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered percent-s test"
  );

  write_errno(31);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 31);

  write_errno(37);

  // SAFETY: stream and variadic argument satisfy `fprintf("%s", payload)` contract.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      payload.as_ptr().cast::<c_char>(),
    )
  };
  let payload_len = payload.len().saturating_sub(1);
  let expected_written =
    c_int::try_from(payload_len).unwrap_or_else(|_| unreachable!("payload length must fit c_int"));

  assert_eq!(written, expected_written);
  assert_eq!(read_errno(), 37);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::try_from(payload_len)
    .unwrap_or_else(|_| unreachable!("payload length must fit c_long"));

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush newline emitted through %s payload",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_percent_c_newline_payload() {
  let _guard = test_lock();
  let format = b"%c\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered %c newline test"
  );

  write_errno(79);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 79);

  write_errno(83);

  // SAFETY: stream/format pointers are valid and `%c` consumes one promoted `int` argument.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), c_int::from(b'\n')) };

  assert_eq!(written, 1);
  assert_eq!(read_errno(), 83);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset, 1,
    "line-buffered mode must flush when newline is emitted through %c payload",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_non_newline_percent_c_until_fflush() {
  let _guard = test_lock();
  let format = b"%c\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered %c defer test"
  );

  write_errno(89);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 89);

  write_errno(97);

  // SAFETY: stream/format pointers are valid and `%c` consumes one promoted `int` argument.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), c_int::from(b'Z')) };

  assert_eq!(written, 1);
  assert_eq!(read_errno(), 97);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer non-newline %c payload until explicit fflush",
  );

  write_errno(101);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 101);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush, 1,
    "fflush must make deferred non-newline %c payload visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_dynamic_width_percent_c_newline_payload() {
  let _guard = test_lock();
  let format = b"%*c\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered dynamic-width %c newline test"
  );

  write_errno(103);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 103);

  write_errno(107);

  // SAFETY: stream/format pointers are valid and `%*c` consumes one width and one promoted `int`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(3),
      c_int::from(b'\n'),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(read_errno(), 107);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset,
    c_long::from(written),
    "line-buffered mode must flush when dynamic-width %c emits newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_dynamic_width_percent_c_non_newline_until_fflush() {
  let _guard = test_lock();
  let format = b"%*c\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered dynamic-width %c defer test"
  );

  write_errno(109);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 109);

  write_errno(113);

  // SAFETY: stream/format pointers are valid and `%*c` consumes one width and one promoted `int`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(3),
      c_int::from(b'Q'),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(read_errno(), 113);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer dynamic-width non-newline %c payload until fflush",
  );

  write_errno(127);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 127);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(written),
    "fflush must make deferred dynamic-width non-newline %c payload visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_negative_dynamic_width_percent_c_newline_payload() {
  let _guard = test_lock();
  let format = b"%*c\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 negative dynamic-width %c newline test"
  );

  write_errno(131);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 131);

  write_errno(137);

  // SAFETY: stream/format pointers are valid and `%*c` consumes one width and one promoted `int`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(-4),
      c_int::from(b'\n'),
    )
  };

  assert_eq!(written, 4);
  assert_eq!(read_errno(), 137);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset,
    c_long::from(written),
    "line-buffered mode must flush when negative dynamic-width %c emits newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_negative_dynamic_width_percent_c_non_newline_until_fflush() {
  let _guard = test_lock();
  let format = b"%*c\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 negative dynamic-width %c defer test"
  );

  write_errno(139);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 139);

  write_errno(149);

  // SAFETY: stream/format pointers are valid and `%*c` consumes one width and one promoted `int`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(-4),
      c_int::from(b'Q'),
    )
  };

  assert_eq!(written, 4);
  assert_eq!(read_errno(), 149);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer negative dynamic-width non-newline %c payload until fflush",
  );

  write_errno(151);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 151);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(written),
    "fflush must make deferred negative dynamic-width non-newline %c payload visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_newline_after_percent_f_format() {
  let _guard = test_lock();
  let format = b"%f\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered %f newline test"
  );

  write_errno(31);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 31);

  write_errno(37);

  // SAFETY: stream and format string are valid; `%f` consumes one promoted `double` variadic argument.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 1.25_f64) };

  assert!(
    written >= 0,
    "host-backed fprintf write must succeed for line-buffered %f newline test",
  );
  assert_eq!(read_errno(), 37);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when newline is emitted after %f conversion",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_dynamic_width_percent_f_newline_payload() {
  let _guard = test_lock();
  let format = b"%*f\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered dynamic-width %f newline test"
  );

  write_errno(41);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 41);

  write_errno(43);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*f\\n", int, double)` contract.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 8, 1.25_f64) };

  assert!(
    written > 0,
    "dynamic-width floating-point output with trailing newline must produce bytes",
  );
  assert_eq!(read_errno(), 43);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when dynamic-width %f output includes literal newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_negative_dynamic_width_percent_f_newline_payload() {
  let _guard = test_lock();
  let format = b"%*f\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered negative dynamic-width %f newline test"
  );

  write_errno(131);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 131);

  write_errno(137);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*f\\n", int, double)` contract.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), -8, 1.25_f64) };

  assert!(
    written > 0,
    "negative dynamic-width floating-point output with trailing newline must produce bytes",
  );
  assert_eq!(read_errno(), 137);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when negative dynamic-width %f output includes literal newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_percent_s_newline_after_percent_f_without_literal_newline() {
  let _guard = test_lock();
  let format = b"%f%s\0";
  let suffix = b"tail\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 mixed %f/%s newline propagation test"
  );

  write_errno(43);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 43);

  write_errno(47);

  // SAFETY: stream/format are valid and variadic args satisfy `fprintf("%f%s", double, char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      1.25_f64,
      suffix.as_ptr().cast::<c_char>(),
    )
  };

  assert!(
    written > 0,
    "line-buffered mixed %f/%s output with newline in %s payload must produce bytes",
  );
  assert_eq!(read_errno(), 47);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when downstream %s emits newline after %f",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_percent_s_newline_after_percent_e_without_literal_newline() {
  let _guard = test_lock();
  let format = b"%e%s\0";
  let suffix = b"tail\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 mixed %e/%s newline propagation test"
  );

  write_errno(49);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 49);

  write_errno(53);

  // SAFETY: stream/format are valid and variadic args satisfy `fprintf("%e%s", double, char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      1.25_f64,
      suffix.as_ptr().cast::<c_char>(),
    )
  };

  assert!(
    written > 0,
    "line-buffered mixed %e/%s output with newline in %s payload must produce bytes",
  );
  assert_eq!(read_errno(), 53);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when downstream %s emits newline after %e",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_percent_s_without_newline_after_percent_e_until_fflush() {
  let _guard = test_lock();
  let format = b"%e%s\0";
  let suffix = b"tail\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 mixed %e/%s defer test"
  );

  write_errno(79);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 79);

  write_errno(83);

  // SAFETY: stream/format are valid and variadic args satisfy `fprintf("%e%s", double, char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      1.25_f64,
      suffix.as_ptr().cast::<c_char>(),
    )
  };

  assert!(
    written > 0,
    "line-buffered mixed %e/%s output without newline must produce bytes",
  );
  assert_eq!(read_errno(), 83);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer when downstream %s emits no newline after %e",
  );

  write_errno(89);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 89);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(written),
    "fflush must make deferred mixed %e/%s output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_percent_s_newline_after_percent_g_without_literal_newline() {
  let _guard = test_lock();
  let format = b"%g%s\0";
  let suffix = b"tail\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 mixed %g/%s newline propagation test"
  );

  write_errno(59);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 59);

  write_errno(61);

  // SAFETY: stream/format are valid and variadic args satisfy `fprintf("%g%s", double, char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      1.25_f64,
      suffix.as_ptr().cast::<c_char>(),
    )
  };

  assert!(
    written > 0,
    "line-buffered mixed %g/%s output with newline in %s payload must produce bytes",
  );
  assert_eq!(read_errno(), 61);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when downstream %s emits newline after %g",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_percent_s_newline_after_percent_a_without_literal_newline() {
  let _guard = test_lock();
  let format = b"%a%s\0";
  let suffix = b"tail\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 mixed %a/%s newline propagation test"
  );

  write_errno(73);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 73);

  write_errno(79);

  // SAFETY: stream/format are valid and variadic args satisfy `fprintf("%a%s", double, char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      1.25_f64,
      suffix.as_ptr().cast::<c_char>(),
    )
  };

  assert!(
    written > 0,
    "line-buffered mixed %a/%s output with newline in %s payload must produce bytes",
  );
  assert_eq!(read_errno(), 79);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when downstream %s emits newline after %a",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_percent_s_without_newline_after_percent_g_until_fflush() {
  let _guard = test_lock();
  let format = b"%g%s\0";
  let suffix = b"tail\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 mixed %g/%s defer test"
  );

  write_errno(67);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 67);

  write_errno(71);

  // SAFETY: stream/format are valid and variadic args satisfy `fprintf("%g%s", double, char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      1.25_f64,
      suffix.as_ptr().cast::<c_char>(),
    )
  };

  assert!(
    written > 0,
    "line-buffered mixed %g/%s output without newline must produce bytes",
  );
  assert_eq!(read_errno(), 71);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer when downstream %s emits no newline after %g",
  );

  write_errno(73);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 73);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(written),
    "fflush must make deferred mixed %g/%s output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_percent_s_newline_after_percent_a_without_literal_newline() {
  let _guard = test_lock();
  let format = b"%a%s\0";
  let suffix = b"tail\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 mixed %a/%s newline propagation test"
  );

  write_errno(67);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 67);

  write_errno(71);

  // SAFETY: stream/format are valid and variadic args satisfy `fprintf("%a%s", double, char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      1.25_f64,
      suffix.as_ptr().cast::<c_char>(),
    )
  };

  assert!(
    written > 0,
    "line-buffered mixed %a/%s output with newline in %s payload must produce bytes",
  );
  assert_eq!(read_errno(), 71);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when downstream %s emits newline after %a",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_dynamic_width_and_precision_percent_f_newline_payload() {
  let _guard = test_lock();
  let format = b"%*.*f\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered dynamic width/precision %f newline test"
  );

  write_errno(47);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 47);

  write_errno(53);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*f\\n", int, int, double)`.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 9, 3, 1.25_f64) };

  assert!(
    written > 0,
    "dynamic width/precision floating-point output with trailing newline must produce bytes",
  );
  assert_eq!(read_errno(), 53);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when dynamic width/precision %f output includes literal newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_dynamic_width_and_precision_percent_f_with_escaped_percent_and_newline()
 {
  let _guard = test_lock();
  let format = b"%*.*f%%\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 dynamic width/precision %f%% newline test"
  );

  write_errno(71);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 71);

  write_errno(73);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*f%%\\n", int, int, double)`.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 9, 3, 1.25_f64) };

  assert!(
    written > 0,
    "dynamic width/precision floating-point output with escaped percent and newline must produce bytes",
  );
  assert_eq!(read_errno(), 73);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when dynamic width/precision %f output is followed by escaped percent and newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_negative_dynamic_width_and_precision_percent_f_with_escaped_percent_and_newline()
 {
  let _guard = test_lock();
  let format = b"%*.*f%%\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 negative dynamic width/precision %f%% newline test"
  );

  write_errno(79);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 79);

  write_errno(83);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*f%%\\n", int, int, double)`.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), -9, 3, 1.25_f64) };

  assert!(
    written > 0,
    "negative dynamic width/precision floating-point output with escaped percent and newline must produce bytes",
  );
  assert_eq!(read_errno(), 83);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when negative-width dynamic %f output is followed by escaped percent and newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_dynamic_width_and_precision_percent_f_with_escaped_percent_until_fflush()
 {
  let _guard = test_lock();
  let format = b"%*.*f%%\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 dynamic width/precision %f%% defer test"
  );

  write_errno(79);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 79);

  write_errno(83);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*f%%", int, int, double)`.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 9, 3, 1.25_f64) };

  assert!(
    written > 0,
    "dynamic width/precision floating-point output with escaped percent must produce bytes",
  );
  assert_eq!(read_errno(), 83);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer dynamic width/precision %f%% output without newline until fflush",
  );

  write_errno(89);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 89);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(written),
    "fflush must make deferred dynamic width/precision %f%% output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_dynamic_width_and_precision_percent_f_without_newline_until_fflush()
 {
  let _guard = test_lock();
  let format = b"%*.*f\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered dynamic width/precision %f defer test"
  );

  write_errno(59);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 59);

  write_errno(61);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*f", int, int, double)`.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 9, 3, 1.25_f64) };

  assert!(
    written > 0,
    "dynamic width/precision floating-point output without newline must produce bytes",
  );
  assert_eq!(read_errno(), 61);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer dynamic width/precision %f output until explicit fflush",
  );

  write_errno(67);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 67);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(written),
    "fflush must make deferred dynamic width/precision %f output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_literal_newline_after_unsupported_directive() {
  let _guard = test_lock();
  let format = b"%f\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered unsupported-directive test"
  );

  write_errno(67);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 67);

  write_errno(71);

  // SAFETY: stream pointer is valid and variadic arg matches `%f`.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 1.25_f64) };

  assert!(written > 0, "line-buffered `%f\\n` write must succeed");
  assert_eq!(read_errno(), 71);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when format literal contains newline after unsupported directive",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_non_newline_percent_s_until_fflush() {
  let _guard = test_lock();
  let format = b"%s\0";
  let payload = b"i023-iolbf-percent-s-without-newline\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered defer test"
  );

  write_errno(43);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 43);

  write_errno(47);

  // SAFETY: stream and variadic argument satisfy `fprintf("%s", payload)` contract.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      payload.as_ptr().cast::<c_char>(),
    )
  };
  let payload_len = payload.len().saturating_sub(1);
  let expected_written =
    c_int::try_from(payload_len).unwrap_or_else(|_| unreachable!("payload length must fit c_int"));

  assert_eq!(written, expected_written);
  assert_eq!(read_errno(), 47);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer non-newline payload visibility until fflush",
  );

  write_errno(53);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 53);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::try_from(payload_len)
    .unwrap_or_else(|_| unreachable!("payload length must fit c_long"));

  assert_eq!(
    end_offset_after_flush, expected_end,
    "fflush must make deferred line-buffered payload visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_dynamic_precision_percent_s_newline_outside_cutoff() {
  let _guard = test_lock();
  let format = b"%.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered dynamic precision test"
  );

  write_errno(55);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 55);

  write_errno(59);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%.*s", int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      2,
      payload.as_ptr().cast::<c_char>(),
    )
  };

  assert_eq!(
    written, 2,
    "precision cutoff must limit emitted bytes to the requested prefix",
  );
  assert_eq!(read_errno(), 59);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer when `%.*s` precision excludes source newline",
  );

  write_errno(61);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 61);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(2),
    "fflush must make deferred `%.*s` prefix output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_dynamic_precision_percent_s_newline_within_cutoff() {
  let _guard = test_lock();
  let format = b"%.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered dynamic precision flush test"
  );

  write_errno(63);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 63);

  write_errno(69);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%.*s", int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      3,
      payload.as_ptr().cast::<c_char>(),
    )
  };

  assert_eq!(
    written, 3,
    "precision cutoff that includes newline must emit the expected prefix bytes",
  );
  assert_eq!(read_errno(), 69);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset,
    c_long::from(3),
    "line-buffered mode must flush when `%.*s` precision includes an emitted newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_dynamic_precision_percent_s_with_negative_precision() {
  let _guard = test_lock();
  let format = b"%.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered negative-precision test"
  );

  write_errno(71);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 71);

  write_errno(73);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%.*s", int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      -1,
      payload.as_ptr().cast::<c_char>(),
    )
  };
  let payload_len = payload.len().saturating_sub(1);
  let expected_written =
    c_int::try_from(payload_len).unwrap_or_else(|_| unreachable!("payload length must fit c_int"));

  assert_eq!(
    written, expected_written,
    "negative precision must behave as unspecified precision for `%s`",
  );
  assert_eq!(read_errno(), 73);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::try_from(payload_len)
    .unwrap_or_else(|_| unreachable!("payload length must fit c_long"));

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when negative precision leaves newline in emitted `%s` output",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_dynamic_width_and_precision_percent_s_newline_outside_cutoff()
{
  let _guard = test_lock();
  let format = b"%*.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 dynamic width+precision defer test"
  );

  write_errno(77);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 77);

  write_errno(79);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*s", int, int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(5),
      c_int::from(2),
      payload.as_ptr().cast::<c_char>(),
    )
  };

  assert_eq!(
    written, 5,
    "dynamic width+precision must emit a padded two-byte prefix without newline",
  );
  assert_eq!(read_errno(), 79);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer when `%*.*s` precision excludes source newline",
  );

  write_errno(83);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 83);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(5),
    "fflush must make deferred `%*.*s` output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_dynamic_width_and_precision_percent_s_newline_within_cutoff()
{
  let _guard = test_lock();
  let format = b"%*.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 dynamic width+precision flush test"
  );

  write_errno(85);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 85);

  write_errno(87);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*s", int, int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(6),
      c_int::from(3),
      payload.as_ptr().cast::<c_char>(),
    )
  };

  assert_eq!(
    written, 6,
    "dynamic width+precision should emit a six-byte field when precision includes newline",
  );
  assert_eq!(read_errno(), 87);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset,
    c_long::from(6),
    "line-buffered mode must flush when `%*.*s` precision includes an emitted newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_negative_dynamic_width_and_precision_percent_s_newline_within_cutoff()
 {
  let _guard = test_lock();
  let format = b"%*.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 negative dynamic width+precision %s flush test"
  );

  write_errno(91);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 91);

  write_errno(97);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*s", int, int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(-6),
      c_int::from(3),
      payload.as_ptr().cast::<c_char>(),
    )
  };

  assert_eq!(
    written, 6,
    "negative width+precision should emit a six-byte field when precision includes newline",
  );
  assert_eq!(read_errno(), 97);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset,
    c_long::from(6),
    "line-buffered mode must flush when negative-width `%*.*s` precision includes an emitted newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_dynamic_width_and_precision_percent_s_with_negative_precision()
 {
  let _guard = test_lock();
  let format = b"%*.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 dynamic width+negative precision flush test"
  );

  write_errno(91);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 91);

  write_errno(97);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*s", int, int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(6),
      c_int::from(-1),
      payload.as_ptr().cast::<c_char>(),
    )
  };

  assert_eq!(
    written, 6,
    "negative precision should behave as unspecified precision and honor dynamic width",
  );
  assert_eq!(read_errno(), 97);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset,
    c_long::from(6),
    "line-buffered mode must flush when negative precision leaves newline in `%*.*s` output",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_negative_dynamic_width_and_negative_precision_percent_s_with_newline()
 {
  let _guard = test_lock();
  let format = b"%*.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 negative width+negative precision %s flush test"
  );

  write_errno(101);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 101);

  write_errno(103);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*s", int, int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      c_int::from(-6),
      c_int::from(-1),
      payload.as_ptr().cast::<c_char>(),
    )
  };

  assert_eq!(
    written, 6,
    "negative width and negative precision should emit a six-byte field when newline remains in `%s` output",
  );
  assert_eq!(read_errno(), 103);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset,
    c_long::from(6),
    "line-buffered mode must flush when negative precision keeps newline in negative-width `%*.*s` output",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_dynamic_precision_percent_s_with_negative_precision_without_newline()
 {
  let _guard = test_lock();
  let format = b"%.*s\0";
  let payload = b"abcdef\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered negative-precision defer test"
  );

  write_errno(79);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 79);

  write_errno(83);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%.*s", int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      -1,
      payload.as_ptr().cast::<c_char>(),
    )
  };
  let payload_len = payload.len().saturating_sub(1);
  let expected_written =
    c_int::try_from(payload_len).unwrap_or_else(|_| unreachable!("payload length must fit c_int"));

  assert_eq!(
    written, expected_written,
    "negative precision must keep full `%s` output when no precision cutoff applies",
  );
  assert_eq!(read_errno(), 83);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer negative-precision `%s` output when no newline is emitted",
  );

  write_errno(89);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 89);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::try_from(payload_len)
    .unwrap_or_else(|_| unreachable!("payload length must fit c_long"));

  assert_eq!(
    end_offset_after_flush, expected_end,
    "fflush must make deferred negative-precision `%s` output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_dynamic_width_and_precision_percent_s_without_newline() {
  let _guard = test_lock();
  let format = b"%*.*s\0";
  let payload = b"ab\ncd\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered dynamic width/precision test"
  );

  write_errno(79);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 79);

  write_errno(83);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*.*s", int, int, const char*)`.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr().cast(),
      -6,
      2,
      payload.as_ptr().cast::<c_char>(),
    )
  };

  assert_eq!(
    written, 6,
    "width + precision formatting should emit padded two-byte prefix without newline",
  );
  assert_eq!(read_errno(), 83);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer when dynamic precision excludes source newline",
  );

  write_errno(89);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 89);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(6),
    "fflush must make deferred dynamic-width/precision `%s` output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_non_newline_percent_f_until_fflush() {
  let _guard = test_lock();
  let format = b"%f\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I023 line-buffered float defer test"
  );

  write_errno(61);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 61);

  write_errno(67);

  // SAFETY: stream and variadic argument satisfy `fprintf("%f", double)` contract.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 1.5_f64) };

  assert!(
    written > 0,
    "floating-point output must produce at least one byte"
  );
  assert_eq!(read_errno(), 67);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer non-newline %f output visibility until fflush",
  );

  write_errno(71);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 71);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert!(
    end_offset_after_flush > 0,
    "fflush must make deferred line-buffered float output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_negative_dynamic_width_percent_f_without_newline_until_fflush()
{
  let _guard = test_lock();
  let format = b"%*f\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 negative dynamic-width %f defer test"
  );

  write_errno(173);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 173);

  write_errno(179);

  // SAFETY: stream and variadic arguments satisfy `fprintf("%*f", int, double)` contract.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), -8, 1.25_f64) };

  assert!(
    written > 0,
    "negative dynamic-width floating-point output must produce bytes",
  );
  assert_eq!(read_errno(), 179);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer negative dynamic-width %f output without newline until fflush",
  );

  write_errno(181);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 181);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_after_flush,
    c_long::from(written),
    "fflush must make deferred negative dynamic-width %f output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_defers_unsupported_directive_with_escaped_percent_until_fflush() {
  let _guard = test_lock();
  let format = b"%f%%\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 line-buffered escaped-percent defer test"
  );

  write_errno(73);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 73);

  write_errno(79);

  // SAFETY: stream and variadic argument satisfy `fprintf("%f%%", double)` contract.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 2.5_f64) };

  assert!(
    written > 0,
    "floating-point output with trailing escaped percent must produce bytes"
  );
  assert_eq!(read_errno(), 79);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_before_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert_eq!(
    end_offset_before_flush, 0,
    "line-buffered mode must defer unsupported-directive output without newline until fflush",
  );

  write_errno(83);

  // SAFETY: stream pointer came from `tmpfile` and is valid for host flush.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 83);
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset_after_flush = unsafe { lseek(fd, 0, SEEK_END) };

  assert!(
    end_offset_after_flush > 0,
    "fflush must make deferred escaped-percent output visible",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_line_buffered_mode_flushes_unsupported_directive_with_escaped_percent_and_literal_newline()
 {
  let _guard = test_lock();
  let format = b"%f%%\n\0";
  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let stream = unsafe { tmpfile() };

  assert!(
    !stream.is_null(),
    "tmpfile stream must be available for I022 escaped-percent newline flush test"
  );

  write_errno(89);

  // SAFETY: stream pointer is valid and line-buffered mode accepts null buffer with non-zero size.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(64)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 89);

  write_errno(97);

  // SAFETY: stream and variadic argument satisfy `fprintf("%f%%\\n", double)` contract.
  let written = unsafe { fprintf(stream, format.as_ptr().cast(), 2.5_f64) };

  assert!(
    written > 0,
    "floating-point output with escaped percent and newline must produce bytes"
  );
  assert_eq!(read_errno(), 97);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: valid descriptor and `SEEK_END` are passed to host `lseek`.
  let end_offset = unsafe { lseek(fd, 0, SEEK_END) };
  let expected_end = c_long::from(written);

  assert_eq!(
    end_offset, expected_end,
    "line-buffered mode must flush when escaped-percent output is followed by literal newline",
  );

  // SAFETY: stream came from `tmpfile`.
  let close_status = unsafe { fclose(stream) };

  assert_eq!(close_status, 0);
}

#[test]
fn setvbuf_accepts_buffered_modes_with_non_zero_size() {
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);
  let mut user_buffer = [0_u8; 64];
  let buffer_ptr = user_buffer.as_mut_ptr().cast::<c_char>();

  for mode in [_IOFBF, _IOLBF] {
    write_errno(91);

    // SAFETY: stream and buffer pointers are valid for this call.
    let status = unsafe { setvbuf(stream, buffer_ptr, mode, as_size_t(user_buffer.len())) };

    assert_eq!(status, 0, "mode={mode} with non-zero size must succeed");
    assert_eq!(read_errno(), 91);
  }
}

#[test]
fn setvbuf_accepts_buffered_modes_with_null_buffer_and_non_zero_size() {
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);

  for mode in [_IOFBF, _IOLBF] {
    write_errno(27);

    // SAFETY: stream pointer is valid and this minimal implementation treats
    // `buffer` as opaque.
    let status = unsafe { setvbuf(stream, ptr::null_mut(), mode, as_size_t(8)) };

    assert_eq!(status, 0, "mode={mode} with null buffer should succeed");
    assert_eq!(read_errno(), 27, "success path must preserve errno");
  }
}

#[test]
fn setvbuf_does_not_modify_user_buffer() {
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);
  let mut user_buffer = [0xA5_u8; 16];
  let before = user_buffer;

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast::<c_char>(),
      _IOFBF,
      as_size_t(user_buffer.len()),
    )
  };

  assert_eq!(status, 0);
  assert_eq!(user_buffer, before);
}

#[test]
fn setvbuf_rejects_reconfiguration_after_stream_was_used() {
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];

  // SAFETY: stream and buffer pointers are valid for this call.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast::<c_char>(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);

  write_errno(67);

  // SAFETY: stream pointer is stable for this call.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 67);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_reconfiguration_after_fflush_null_marks_tracked_stream_active() {
  let _guard = test_lock();
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];

  // SAFETY: stream and buffer pointers are valid for this call.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast::<c_char>(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);

  write_errno(73);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 73);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_synthetic_reconfiguration_after_successful_fflush_null() {
  let _guard = test_lock();
  let stream = synthetic_untracked_stream();
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];

  write_errno(41);

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast::<c_char>(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);
  assert_eq!(read_errno(), 41);

  write_errno(73);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 73);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_allows_synthetic_first_configuration_after_successful_fflush_null() {
  let _guard = test_lock();
  let stream = synthetic_untracked_stream();
  let mut user_buffer = [0_u8; 16];

  write_errno(59);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 59);

  write_errno(61);

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(user_buffer.len()),
    )
  };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 61);
}

#[test]
fn setvbuf_rejects_synthetic_reconfiguration_after_non_null_fflush_and_preserves_errno() {
  let _guard = test_lock();
  let stream = synthetic_untracked_stream();
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];

  write_errno(13);

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast::<c_char>(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);
  assert_eq!(read_errno(), 13);

  write_errno(83);

  // SAFETY: non-host synthetic stream is a valid opaque key for `fflush`.
  let flush_status = unsafe { fflush(stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 83);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_reconfiguration_after_failed_fflush_null_marks_tracked_stream_active() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-tracked-null-failure\n\0";
  let mut marker = 0_u8;
  let stream = as_file_ptr(&mut marker);
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];

  // SAFETY: stream and buffer pointers are valid for this call.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast::<c_char>(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);

  let mut skipped_failing_streams = Vec::new();
  let failing_stream = loop {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    let candidate = unsafe { tmpfile() };

    assert!(
      !candidate.is_null(),
      "tmpfile for failing stream must succeed"
    );

    write_errno(0);

    // SAFETY: stream pointer is valid and buffered mode accepts null buffer with non-zero size.
    let primed_setvbuf_status =
      unsafe { setvbuf(candidate, ptr::null_mut(), _IOFBF, as_size_t(64)) };

    if primed_setvbuf_status == 0 {
      assert_eq!(read_errno(), 0);
      break candidate;
    }

    assert_eq!(primed_setvbuf_status, EOF_STATUS);
    assert_eq!(read_errno(), EINVAL);
    skipped_failing_streams.push(candidate);
    assert!(
      skipped_failing_streams.len() < 16,
      "failed to acquire a primed failing stream for fflush failure isolation test",
    );
  };

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(write_status >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };

  for skipped_stream in skipped_failing_streams {
    // SAFETY: each skipped stream came from `tmpfile` and must be closed.
    let skipped_close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(skipped_close_status, 0);
  }
}

#[test]
fn setvbuf_rejects_stdout_reconfiguration_after_fflush_null() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  write_errno(19);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 19);

  // SAFETY: host libc provides `stdout` global stream pointer.
  let stdout_stream = unsafe { host_stdout };

  assert!(
    !stdout_stream.is_null(),
    "host stdout pointer must be available"
  );

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let status = unsafe { setvbuf(stdout_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_stderr_reconfiguration_after_fflush_null() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  write_errno(29);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 29);

  // SAFETY: host libc provides `stderr` global stream pointer.
  let stderr_stream = unsafe { host_stderr };

  assert!(
    !stderr_stream.is_null(),
    "host stderr pointer must be available"
  );

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let status = unsafe { setvbuf(stderr_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_stdin_reconfiguration_after_fflush_null() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  write_errno(31);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 31);

  // SAFETY: host libc provides `stdin` global stream pointer.
  let stdin_stream = unsafe { host_stdin };

  assert!(
    !stdin_stream.is_null(),
    "host stdin pointer must be available"
  );

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let status = unsafe { setvbuf(stdin_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_stdout_reconfiguration_after_failed_fflush_null() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-null-failure\n\0";
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(write_status >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  // SAFETY: host libc provides `stdout` global stream pointer.
  let stdout_stream = unsafe { host_stdout };

  assert!(
    !stdout_stream.is_null(),
    "host stdout pointer must be available"
  );

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let setvbuf_status =
    unsafe { setvbuf(stdout_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_rejects_stderr_reconfiguration_after_failed_fflush_null() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-null-failure-stderr\n\0";
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(write_status >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  // SAFETY: host libc provides `stderr` global stream pointer.
  let stderr_stream = unsafe { host_stderr };

  assert!(
    !stderr_stream.is_null(),
    "host stderr pointer must be available"
  );

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let setvbuf_status =
    unsafe { setvbuf(stderr_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_allows_synthetic_untracked_stream_after_failed_non_null_fflush() {
  let _guard = test_lock();
  let payload = b"i023-setvbuf-non-null-failure-synthetic-untracked\n\0";
  let mut user_buffer = [0_u8; 16];
  let stream = synthetic_untracked_stream();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  write_errno(31);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming failing stream must succeed");
  assert_eq!(read_errno(), 31);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(stream) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let flush_status = unsafe { fflush(failing_stream) };

  if flush_status == EOF_STATUS {
    assert_ne!(read_errno(), 0);
  } else {
    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 0);
  }

  write_errno(67);

  // SAFETY: this minimal implementation treats stream pointer as an opaque key.
  let setvbuf_status = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 67);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_allows_synthetic_untracked_buffered_stream_after_failed_non_null_fflush() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-non-null-failure-synthetic-untracked-buffered\n\0";
  let stream = synthetic_untracked_stream();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  write_errno(43);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming failing stream must succeed");
  assert_eq!(read_errno(), 43);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(stream) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let flush_status = unsafe { fflush(failing_stream) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(79);

  // SAFETY: this minimal implementation treats stream pointer as an opaque key.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOFBF, as_size_t(32)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 79);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_allows_synthetic_untracked_line_buffered_stream_after_failed_non_null_fflush() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-non-null-failure-synthetic-untracked-line-buffered\n\0";
  let mut user_buffer = [0_u8; 32];
  let stream = synthetic_untracked_stream();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  write_errno(37);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming failing stream must succeed");
  assert_eq!(read_errno(), 37);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(stream) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let flush_status = unsafe { fflush(failing_stream) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(89);

  // SAFETY: this minimal implementation treats stream pointer as an opaque key.
  let setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(user_buffer.len()),
    )
  };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 89);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_allows_synthetic_untracked_line_buffered_null_buffer_after_failed_non_null_fflush() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-non-null-failure-synthetic-untracked-line-buffered-null\n\0";
  let stream = synthetic_untracked_stream();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  write_errno(39);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming failing stream must succeed");
  assert_eq!(read_errno(), 39);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(stream) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let flush_status = unsafe { fflush(failing_stream) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(97);

  // SAFETY: this minimal implementation treats stream pointer as an opaque key.
  let setvbuf_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(32)) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 97);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_allows_synthetic_reconfiguration_after_failed_non_null_fflush_before_io() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-non-null-failure-synthetic-reconfigure\n\0";
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];
  let stream = synthetic_untracked_stream();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  write_errno(23);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming failing stream must succeed");
  assert_eq!(read_errno(), 23);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(stream) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let flush_status = unsafe { fflush(failing_stream) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(29);

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);
  assert_eq!(read_errno(), 29);

  write_errno(31);

  // SAFETY: no I/O occurred on this stream yet, so reconfiguration remains valid.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, 0);
  assert_eq!(read_errno(), 31);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_rejects_synthetic_stream_reconfiguration_after_non_null_fflush_post_failure_isolation() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-non-null-failure-then-synthetic-flush\n\0";
  let stream = synthetic_untracked_stream();
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  write_errno(19);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming failing stream must succeed");
  assert_eq!(read_errno(), 19);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(stream) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let failing_flush_status = unsafe { fflush(failing_stream) };

  assert_eq!(failing_flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(71);

  // SAFETY: non-host synthetic stream is a valid opaque key for `fflush`.
  let synthetic_flush_status = unsafe { fflush(stream) };

  assert_eq!(synthetic_flush_status, 0);
  assert_eq!(read_errno(), 71);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_keeps_other_stream_reconfigurable_after_non_null_fflush_on_other_stream() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-non-null-failure-isolation\0";
  let stream = synthetic_untracked_stream();
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  write_errno(41);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming failing stream must succeed");
  assert_eq!(read_errno(), 41);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to exercise non-null `fflush` host behavior.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let flush_status = unsafe { fflush(failing_stream) };

  assert!(
    flush_status == EOF_STATUS || flush_status == 0,
    "closed-fd host fflush may fail (EOF) or report success (0) depending on host libc behavior",
  );

  if flush_status == EOF_STATUS {
    assert_ne!(read_errno(), 0);
  } else {
    assert_eq!(read_errno(), 0);
  }

  write_errno(59);

  // SAFETY: first stream remains non-IO-active; reconfiguration should still succeed.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, 0);
  assert_eq!(read_errno(), 59);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_allows_other_tracked_stream_after_failed_non_null_fflush() {
  let _guard = test_lock();
  let payload = b"i023-setvbuf-non-null-failure-tracked-isolation\n\0";
  let mut marker = 0_u8;
  let other_stream = as_file_ptr(&mut marker);
  let mut first_other_buffer = [0_u8; 8];
  let mut second_other_buffer = [0_u8; 16];
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: stream and buffer pointers are valid for this call.
  let first_status = unsafe {
    setvbuf(
      other_stream,
      first_other_buffer.as_mut_ptr().cast(),
      _IOFBF,
      as_size_t(first_other_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  write_errno(21);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "priming failing stream must succeed");
  assert_eq!(read_errno(), 21);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(stream) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let flush_status = unsafe { fflush(failing_stream) };

  if flush_status == EOF_STATUS {
    assert_ne!(read_errno(), 0);
  } else {
    assert_eq!(flush_status, 0);
    assert_eq!(read_errno(), 0);
  }

  write_errno(75);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      other_stream,
      second_other_buffer.as_mut_ptr().cast(),
      _IOLBF,
      as_size_t(second_other_buffer.len()),
    )
  };

  assert_eq!(second_status, 0);
  assert_eq!(read_errno(), 75);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_allows_synthetic_untracked_stream_after_failed_fflush_null() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-null-failure-synthetic-untracked\n\0";
  let mut user_buffer = [0_u8; 16];
  let stream = synthetic_untracked_stream();

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(write_status >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(NULL) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(61);

  // SAFETY: this minimal implementation treats stream pointer as an opaque key.
  let setvbuf_status = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 61);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_allows_synthetic_reconfiguration_after_failed_fflush_null_before_io() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-null-failure-synthetic-reconfigure\n\0";
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];
  let stream = synthetic_untracked_stream();

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(write_status >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(NULL) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(37);

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);
  assert_eq!(read_errno(), 37);

  write_errno(43);

  // SAFETY: no I/O occurred on this stream yet, so reconfiguration remains valid.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, 0);
  assert_eq!(read_errno(), 43);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_rejects_synthetic_reconfiguration_after_failed_fflush_null_then_non_null_fflush() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-null-failure-synthetic-then-flush\n\0";
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];
  let stream = synthetic_untracked_stream();

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(write_status >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(NULL) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let null_flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(null_flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(47);

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_status, 0);
  assert_eq!(read_errno(), 47);

  write_errno(53);

  // SAFETY: non-host synthetic stream is a valid opaque key for `fflush`.
  let non_null_flush_status = unsafe { fflush(stream) };

  assert_eq!(non_null_flush_status, 0);
  assert_eq!(read_errno(), 53);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_rejects_synthetic_first_configuration_after_failed_fflush_null_then_non_null_fflush() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-null-failure-synthetic-first-after-flush\n\0";
  let mut user_buffer = [0_u8; 16];
  let stream = synthetic_untracked_stream();

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(write_status >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(NULL) failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let null_flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(null_flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(79);

  // SAFETY: non-host synthetic stream is a valid opaque key for `fflush`.
  let non_null_flush_status = unsafe { fflush(stream) };

  assert_eq!(non_null_flush_status, 0);
  assert_eq!(read_errno(), 79);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let setvbuf_status = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_rejects_synthetic_reconfiguration_after_second_failed_fflush_null() {
  let _guard = test_lock();
  let payload_one = b"i022-setvbuf-null-failure-first-pass\n\0";
  let payload_two = b"i022-setvbuf-null-failure-second-pass\n\0";
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];
  let stream = synthetic_untracked_stream();

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream_one = unsafe { tmpfile() };

  assert!(
    !failing_stream_one.is_null(),
    "first tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status_one = unsafe { fputs(payload_one.as_ptr().cast(), failing_stream_one) };

  assert!(
    write_status_one >= 0,
    "priming first failing stream must succeed"
  );
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd_one = unsafe { fileno(failing_stream_one) };

  assert!(failing_fd_one >= 0, "first failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(NULL) failure.
  let close_status_one = unsafe { close(failing_fd_one) };

  assert_eq!(
    close_status_one, 0,
    "closing first failing stream fd must succeed"
  );

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let first_null_flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(first_null_flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(11);

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_setvbuf_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_setvbuf_status, 0);
  assert_eq!(read_errno(), 11);

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream_two = unsafe { tmpfile() };

  assert!(
    !failing_stream_two.is_null(),
    "second tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status_two = unsafe { fputs(payload_two.as_ptr().cast(), failing_stream_two) };

  assert!(
    write_status_two >= 0,
    "priming second failing stream must succeed"
  );
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd_two = unsafe { fileno(failing_stream_two) };

  assert!(failing_fd_two >= 0, "second failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(NULL) failure.
  let close_status_two = unsafe { close(failing_fd_two) };

  assert_eq!(
    close_status_two, 0,
    "closing second failing stream fd must succeed"
  );

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let second_null_flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(second_null_flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_setvbuf_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream_one) };
  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream_two) };
}

#[test]
fn setvbuf_rejects_synthetic_reconfiguration_after_failed_non_null_then_failed_null_fflush() {
  let _guard = test_lock();
  let payload_non_null = b"i022-setvbuf-non-null-then-null-failure-a\n\0";
  let payload_null = b"i022-setvbuf-non-null-then-null-failure-b\n\0";
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];
  let stream = synthetic_untracked_stream();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream_non_null = unsafe { tmpfile() };

  assert!(
    !failing_stream_non_null.is_null(),
    "tmpfile for non-null failure stream must succeed"
  );

  write_errno(13);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status_non_null = unsafe {
    vfprintf(
      failing_stream_non_null,
      payload_non_null.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(
    write_status_non_null >= 0,
    "priming non-null failure stream must succeed",
  );
  assert_eq!(read_errno(), 13);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd_non_null = unsafe { fileno(failing_stream_non_null) };

  assert!(
    failing_fd_non_null >= 0,
    "non-null failure stream must have an fd"
  );
  // SAFETY: explicit fd close is used to induce host fflush(stream) failure.
  let close_status_non_null = unsafe { close(failing_fd_non_null) };

  assert_eq!(
    close_status_non_null, 0,
    "closing non-null failure stream fd must succeed"
  );

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call and fd was closed above.
  let non_null_flush_status = unsafe { fflush(failing_stream_non_null) };

  assert_eq!(non_null_flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(17);

  // SAFETY: synthetic stream key and buffer pointer are treated as opaque metadata.
  let first_setvbuf_status = unsafe {
    setvbuf(
      stream,
      first_buffer.as_mut_ptr().cast(),
      _IOFBF,
      as_size_t(first_buffer.len()),
    )
  };

  assert_eq!(first_setvbuf_status, 0);
  assert_eq!(read_errno(), 17);

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream_null = unsafe { tmpfile() };

  assert!(
    !failing_stream_null.is_null(),
    "tmpfile for null failure stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status_null = unsafe { fputs(payload_null.as_ptr().cast(), failing_stream_null) };

  assert!(
    write_status_null >= 0,
    "priming null failure stream must succeed"
  );
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd_null = unsafe { fileno(failing_stream_null) };

  assert!(failing_fd_null >= 0, "null failure stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush(NULL) failure.
  let close_status_null = unsafe { close(failing_fd_null) };

  assert_eq!(
    close_status_null, 0,
    "closing null failure stream fd must succeed"
  );

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let null_flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(null_flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_setvbuf_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream_non_null) };
  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream_null) };
}

#[test]
fn setvbuf_rejects_stdin_reconfiguration_after_failed_fflush_null() {
  let _guard = test_lock();
  let payload = b"i022-setvbuf-null-failure-stdin\n\0";
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides a valid stream or null on allocation failure.
  let failing_stream = unsafe { tmpfile() };

  assert!(
    !failing_stream.is_null(),
    "tmpfile for failing stream must succeed"
  );

  // SAFETY: stream and payload pointers are valid for host fputs.
  let write_status = unsafe { fputs(payload.as_ptr().cast(), failing_stream) };

  assert!(write_status >= 0, "priming failing stream must succeed");
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let failing_fd = unsafe { fileno(failing_stream) };

  assert!(failing_fd >= 0, "failing stream must have an fd");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(failing_fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: C contract allows `fflush(NULL)` to flush all process streams.
  let flush_status = unsafe { fflush(ptr::null_mut()) };

  assert_eq!(flush_status, EOF_STATUS);
  assert_ne!(read_errno(), 0);

  // SAFETY: host libc provides `stdin` global stream pointer.
  let stdin_stream = unsafe { host_stdin };

  assert!(
    !stdin_stream.is_null(),
    "host stdin pointer must be available"
  );

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let setvbuf_status = unsafe { setvbuf(stdin_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_rejects_reconfiguration_after_non_null_fflush_attempt() {
  let _guard = test_lock();
  let payload = b"i022\n\0";
  let max_untracked_stream_retries: usize = 64;
  let mut first_buffer = [0_u8; 8];
  let mut second_buffer = [0_u8; 16];
  let mut skipped_streams = Vec::new();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };
  let stream = loop {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    let candidate = unsafe { tmpfile() };

    assert!(
      !candidate.is_null(),
      "tmpfile stream for non-null failure case must succeed"
    );

    write_errno(0);

    // SAFETY: stream and buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        candidate,
        first_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(first_buffer.len()),
      )
    };

    if first_status == 0 {
      break candidate;
    }

    assert_eq!(first_status, EOF_STATUS);
    assert_eq!(read_errno(), EINVAL);
    skipped_streams.push(candidate);
    assert!(
      skipped_streams.len() < max_untracked_stream_retries,
      "failed to obtain an untracked tmpfile stream after repeated attempts",
    );
  };

  write_errno(53);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "host-backed write must succeed");
  assert_eq!(read_errno(), 53);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(fd) };

  assert_eq!(close_status, 0, "closing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stream) };

  assert!(
    flush_status == EOF_STATUS || flush_status == 0,
    "closed-fd host fflush may fail (EOF) or report success (0) depending on host libc behavior",
  );

  if flush_status == EOF_STATUS {
    assert_ne!(read_errno(), 0, "failed fflush(stream) must set errno");
  } else {
    assert_eq!(
      read_errno(),
      0,
      "successful fflush(stream) must preserve errno"
    );
  }

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  for skipped_stream in skipped_streams {
    // SAFETY: each skipped stream came from `tmpfile` and was not fd-closed.
    let close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(
      close_status, 0,
      "closing skipped tmpfile stream must succeed"
    );
  }

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(stream) };
}

#[test]
fn setvbuf_rejects_unbuffered_reconfiguration_after_non_null_fflush_attempt() {
  let _guard = test_lock();
  let payload = b"i022-unbuffered\n\0";
  let max_untracked_stream_retries: usize = 64;
  let mut first_buffer = [0_u8; 8];
  let mut skipped_streams = Vec::new();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };
  let stream = loop {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    let candidate = unsafe { tmpfile() };

    assert!(
      !candidate.is_null(),
      "tmpfile stream for non-null failure case must succeed"
    );

    write_errno(0);

    // SAFETY: stream and buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        candidate,
        first_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(first_buffer.len()),
      )
    };

    if first_status == 0 {
      break candidate;
    }

    assert_eq!(first_status, EOF_STATUS);
    assert_eq!(read_errno(), EINVAL);
    skipped_streams.push(candidate);
    assert!(
      skipped_streams.len() < max_untracked_stream_retries,
      "failed to obtain an untracked tmpfile stream after repeated attempts",
    );
  };

  write_errno(17);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "host-backed write must succeed");
  assert_eq!(read_errno(), 17);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(fd) };

  assert_eq!(close_status, 0, "closing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stream) };

  assert!(
    flush_status == EOF_STATUS || flush_status == 0,
    "closed-fd host fflush may fail (EOF) or report success (0) depending on host libc behavior",
  );

  if flush_status == EOF_STATUS {
    assert_ne!(read_errno(), 0, "failed fflush(stream) must set errno");
  } else {
    assert_eq!(
      read_errno(),
      0,
      "successful fflush(stream) must preserve errno"
    );
  }

  write_errno(0);

  // SAFETY: stream pointer is valid and unbuffered mode does not require user buffer.
  let second_status = unsafe { setvbuf(stream, ptr::null_mut(), _IONBF, 0) };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  for skipped_stream in skipped_streams {
    // SAFETY: each skipped stream came from `tmpfile` and was not fd-closed.
    let close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(
      close_status, 0,
      "closing skipped tmpfile stream must succeed"
    );
  }

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(stream) };
}

#[test]
fn setvbuf_rejects_null_buffer_reconfiguration_after_non_null_fflush_attempt() {
  let _guard = test_lock();
  let payload = b"i022-null-buffer\n\0";
  let max_untracked_stream_retries: usize = 64;
  let mut first_buffer = [0_u8; 8];
  let mut skipped_streams = Vec::new();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };
  let stream = loop {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    let candidate = unsafe { tmpfile() };

    assert!(
      !candidate.is_null(),
      "tmpfile stream for non-null failure case must succeed"
    );

    write_errno(0);

    // SAFETY: stream and buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        candidate,
        first_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(first_buffer.len()),
      )
    };

    if first_status == 0 {
      break candidate;
    }

    assert_eq!(first_status, EOF_STATUS);
    assert_eq!(read_errno(), EINVAL);
    skipped_streams.push(candidate);
    assert!(
      skipped_streams.len() < max_untracked_stream_retries,
      "failed to obtain an untracked tmpfile stream after repeated attempts",
    );
  };

  write_errno(23);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "host-backed write must succeed");
  assert_eq!(read_errno(), 23);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(fd) };

  assert_eq!(close_status, 0, "closing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stream) };

  assert!(
    flush_status == EOF_STATUS || flush_status == 0,
    "closed-fd host fflush may fail (EOF) or report success (0) depending on host libc behavior",
  );

  if flush_status == EOF_STATUS {
    assert_ne!(read_errno(), 0, "failed fflush(stream) must set errno");
  } else {
    assert_eq!(
      read_errno(),
      0,
      "successful fflush(stream) must preserve errno"
    );
  }

  write_errno(97);

  // SAFETY: stream pointer is valid and null buffer is accepted for buffered modes.
  let second_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOFBF, as_size_t(32)) };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  for skipped_stream in skipped_streams {
    // SAFETY: each skipped stream came from `tmpfile` and was not fd-closed.
    let close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(
      close_status, 0,
      "closing skipped tmpfile stream must succeed"
    );
  }

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(stream) };
}

#[test]
fn setvbuf_rejects_reconfiguration_after_failed_host_vfprintf() {
  let _guard = test_lock();
  let payload = b"i022-vfprintf-host-failure\n\0";
  let max_untracked_stream_retries: usize = 64;
  let mut second_buffer = [0_u8; 16];
  let mut skipped_streams = Vec::new();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };
  let stream = loop {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    let candidate = unsafe { tmpfile() };

    assert!(
      !candidate.is_null(),
      "tmpfile stream for non-null failure case must succeed"
    );

    write_errno(0);

    // SAFETY: stream pointer is valid and unbuffered mode accepts null buffer.
    let first_status = unsafe { setvbuf(candidate, ptr::null_mut(), _IONBF, 0) };

    if first_status == 0 {
      break candidate;
    }

    assert_eq!(first_status, EOF_STATUS);
    assert_eq!(read_errno(), EINVAL);
    skipped_streams.push(candidate);
    assert!(
      skipped_streams.len() < max_untracked_stream_retries,
      "failed to obtain an untracked tmpfile stream after repeated attempts",
    );
  };

  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: explicit fd close is used to induce host `vfprintf` failure.
  let close_status = unsafe { close(fd) };

  assert_eq!(close_status, 0, "closing stream fd must succeed");

  write_errno(0);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert_eq!(write_status, EOF_STATUS);
  assert_ne!(read_errno(), 0, "failed vfprintf(stream) must set errno");

  write_errno(0);

  // SAFETY: stream and buffer pointers are valid for this call.
  let second_status = unsafe {
    setvbuf(
      stream,
      second_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(second_buffer.len()),
    )
  };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  for skipped_stream in skipped_streams {
    // SAFETY: each skipped stream came from `tmpfile` and was not fd-closed.
    let close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(
      close_status, 0,
      "closing skipped tmpfile stream must succeed"
    );
  }

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(stream) };
}

#[test]
fn setvbuf_keeps_other_stream_reconfigurable_after_failed_host_vfprintf() {
  let _guard = test_lock();
  let payload = b"i022-vfprintf-host-failure-isolation\n\0";
  let max_untracked_stream_retries: usize = 64;
  let mut unaffected_initial_buffer = [0_u8; 8];
  let mut unaffected_replacement_buffer = [0_u8; 16];
  let unaffected_stream = synthetic_untracked_stream();
  let mut skipped_streams = Vec::new();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };
  let failing_stream = loop {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    let candidate = unsafe { tmpfile() };

    assert!(
      !candidate.is_null(),
      "tmpfile stream for host vfprintf failure case must succeed"
    );

    write_errno(0);

    // SAFETY: stream pointer is valid and unbuffered mode accepts null buffer.
    let first_status = unsafe { setvbuf(candidate, ptr::null_mut(), _IONBF, 0) };

    if first_status == 0 {
      break candidate;
    }

    assert_eq!(first_status, EOF_STATUS);
    assert_eq!(read_errno(), EINVAL);
    skipped_streams.push(candidate);
    assert!(
      skipped_streams.len() < max_untracked_stream_retries,
      "failed to obtain an untracked tmpfile stream after repeated attempts",
    );
  };

  write_errno(17);

  // SAFETY: unaffected synthetic stream key and buffer pointer are valid for metadata tracking.
  let unaffected_first_status = unsafe {
    setvbuf(
      unaffected_stream,
      unaffected_initial_buffer.as_mut_ptr().cast::<c_char>(),
      _IOFBF,
      as_size_t(unaffected_initial_buffer.len()),
    )
  };

  assert_eq!(unaffected_first_status, 0);
  assert_eq!(read_errno(), 17);

  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(failing_stream) };

  assert!(fd >= 0, "failing stream must expose file descriptor");
  // SAFETY: explicit fd close is used to induce host `vfprintf` failure.
  let close_status = unsafe { close(fd) };

  assert_eq!(close_status, 0, "closing failing stream fd must succeed");

  write_errno(0);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      failing_stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert_eq!(write_status, EOF_STATUS);
  assert_ne!(read_errno(), 0, "failed vfprintf(stream) must set errno");

  write_errno(29);

  // SAFETY: unaffected stream and buffer pointers are valid for this call.
  let unaffected_second_status = unsafe {
    setvbuf(
      unaffected_stream,
      unaffected_replacement_buffer.as_mut_ptr().cast::<c_char>(),
      _IOLBF,
      as_size_t(unaffected_replacement_buffer.len()),
    )
  };

  assert_eq!(unaffected_second_status, 0);
  assert_eq!(read_errno(), 29);

  for skipped_stream in skipped_streams {
    // SAFETY: each skipped stream came from `tmpfile` and was not fd-closed.
    let close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(
      close_status, 0,
      "closing skipped tmpfile stream must succeed"
    );
  }

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(failing_stream) };
}

#[test]
fn setvbuf_rejects_null_buffer_line_reconfiguration_after_non_null_fflush_attempt() {
  let _guard = test_lock();
  let payload = b"i022-null-buffer-line\n\0";
  let max_untracked_stream_retries: usize = 64;
  let mut first_buffer = [0_u8; 8];
  let mut skipped_streams = Vec::new();
  let mut empty_ap = SysVVaList {
    gp_offset: 48,
    fp_offset: 0,
    overflow_arg_area: ptr::null_mut(),
    reg_save_area: ptr::null_mut(),
  };
  let stream = loop {
    // SAFETY: host libc provides a valid stream or null on allocation failure.
    let candidate = unsafe { tmpfile() };

    assert!(
      !candidate.is_null(),
      "tmpfile stream for non-null failure case must succeed"
    );

    write_errno(0);

    // SAFETY: stream and buffer pointers are valid for this call.
    let first_status = unsafe {
      setvbuf(
        candidate,
        first_buffer.as_mut_ptr().cast::<c_char>(),
        _IOFBF,
        as_size_t(first_buffer.len()),
      )
    };

    if first_status == 0 {
      break candidate;
    }

    assert_eq!(first_status, EOF_STATUS);
    assert_eq!(read_errno(), EINVAL);
    skipped_streams.push(candidate);
    assert!(
      skipped_streams.len() < max_untracked_stream_retries,
      "failed to obtain an untracked tmpfile stream after repeated attempts",
    );
  };

  write_errno(31);

  // SAFETY: stream, format string, and `va_list` pointer are valid.
  let write_status = unsafe {
    vfprintf(
      stream,
      payload.as_ptr().cast(),
      core::ptr::addr_of_mut!(empty_ap).cast(),
    )
  };

  assert!(write_status >= 0, "host-backed write must succeed");
  assert_eq!(read_errno(), 31);
  // SAFETY: `fileno` expects a valid host FILE pointer.
  let fd = unsafe { fileno(stream) };

  assert!(fd >= 0, "stream must expose file descriptor");
  // SAFETY: explicit fd close is used to induce host fflush failure.
  let close_status = unsafe { close(fd) };

  assert_eq!(close_status, 0, "closing stream fd must succeed");

  write_errno(0);

  // SAFETY: host stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stream) };

  assert!(
    flush_status == EOF_STATUS || flush_status == 0,
    "closed-fd host fflush may fail (EOF) or report success (0) depending on host libc behavior",
  );

  if flush_status == EOF_STATUS {
    assert_ne!(read_errno(), 0, "failed fflush(stream) must set errno");
  } else {
    assert_eq!(
      read_errno(),
      0,
      "successful fflush(stream) must preserve errno"
    );
  }

  write_errno(89);

  // SAFETY: stream pointer is valid and null buffer is accepted for buffered modes.
  let second_status = unsafe { setvbuf(stream, ptr::null_mut(), _IOLBF, as_size_t(32)) };

  assert_eq!(second_status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);

  for skipped_stream in skipped_streams {
    // SAFETY: each skipped stream came from `tmpfile` and was not fd-closed.
    let close_status = unsafe { fclose(skipped_stream) };

    assert_eq!(
      close_status, 0,
      "closing skipped tmpfile stream must succeed"
    );
  }

  // SAFETY: even after injected fd close, fclose is still required to release FILE state.
  let _ = unsafe { fclose(stream) };
}

#[test]
fn setvbuf_rejects_stdin_reconfiguration_after_non_null_fflush() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides `stdin` global stream pointer.
  let stdin_stream = unsafe { host_stdin };

  assert!(
    !stdin_stream.is_null(),
    "host stdin pointer must be available"
  );

  write_errno(41);

  // SAFETY: non-null host stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stdin_stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 41);

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let status = unsafe { setvbuf(stdin_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_stdout_reconfiguration_after_non_null_fflush() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides `stdout` global stream pointer.
  let stdout_stream = unsafe { host_stdout };

  assert!(
    !stdout_stream.is_null(),
    "host stdout pointer must be available"
  );

  write_errno(43);

  // SAFETY: non-null host stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stdout_stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 43);

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let status = unsafe { setvbuf(stdout_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn setvbuf_rejects_stderr_reconfiguration_after_non_null_fflush() {
  let _guard = test_lock();
  let mut user_buffer = [0_u8; 16];

  // SAFETY: host libc provides `stderr` global stream pointer.
  let stderr_stream = unsafe { host_stderr };

  assert!(
    !stderr_stream.is_null(),
    "host stderr pointer must be available"
  );

  write_errno(47);

  // SAFETY: non-null host stream pointer is valid for this call.
  let flush_status = unsafe { fflush(stderr_stream) };

  assert_eq!(flush_status, 0);
  assert_eq!(read_errno(), 47);

  write_errno(0);

  // SAFETY: stream and user buffer pointers are valid for this call.
  let status = unsafe { setvbuf(stderr_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(status, EOF_STATUS);
  assert_eq!(read_errno(), EINVAL);
}
