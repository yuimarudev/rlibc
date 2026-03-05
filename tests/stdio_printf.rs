#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::{c_char, c_int, c_long, c_longlong, c_void};
use core::sync::atomic::{AtomicUsize, Ordering};
use rlibc::abi::errno::EINVAL;
use rlibc::errno::__errno_location;
use rlibc::stdio::{_IOFBF, _IONBF, EOF, FILE, fprintf, printf, setvbuf, vfprintf, vprintf};
use std::ffi::CString;

type PrintfFn = unsafe extern "C" fn(format: *const c_char, ...) -> c_int;

type FprintfFn = unsafe extern "C" fn(stream: *mut FILE, format: *const c_char, ...) -> c_int;

type VprintfFn = unsafe extern "C" fn(format: *const c_char, ap: *mut c_void) -> c_int;

type VfprintfFn =
  unsafe extern "C" fn(stream: *mut FILE, format: *const c_char, ap: *mut c_void) -> c_int;

unsafe extern "C" {
  fn fclose(stream: *mut FILE) -> c_int;
  fn fopen(pathname: *const c_char, mode: *const c_char) -> *mut FILE;
  fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
  fn rewind(stream: *mut FILE);
  fn tmpfile() -> *mut FILE;
  static mut stdout: *mut FILE;
}

#[repr(C)]
struct SysVVaList {
  gp_offset: u32,
  fp_offset: u32,
  overflow_arg_area: *mut c_void,
  reg_save_area: *mut c_void,
}

struct OwnedVaList {
  _overflow_args: Vec<u64>,
  raw: SysVVaList,
}

impl OwnedVaList {
  const fn from_u64_slots(mut args: Vec<u64>) -> Self {
    let overflow_arg_area = if args.is_empty() {
      core::ptr::null_mut()
    } else {
      args.as_mut_ptr().cast::<c_void>()
    };

    Self {
      _overflow_args: args,
      raw: SysVVaList {
        gp_offset: 48,
        fp_offset: 0,
        overflow_arg_area,
        reg_save_area: core::ptr::null_mut(),
      },
    }
  }

  const fn as_mut_ptr(&mut self) -> *mut c_void {
    core::ptr::addr_of_mut!(self.raw).cast::<c_void>()
  }
}

fn ptr_slot<T>(pointer: *const T) -> u64 {
  u64::try_from(pointer.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit in u64 on this target"))
}

fn c_string(input: &str) -> CString {
  CString::new(input)
    .unwrap_or_else(|_| unreachable!("test literals must not include interior NUL bytes"))
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` points to writable thread-local errno storage.
  unsafe {
    __errno_location().write(value);
  }
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` points to readable thread-local errno storage.
  unsafe { __errno_location().read() }
}

fn synthetic_untracked_stream() -> *mut FILE {
  static NEXT_STREAM_ID: AtomicUsize = AtomicUsize::new(1);
  const BASE_ADDR: usize = 0x2000_0000_0000;
  const STRIDE: usize = 0x1000;
  let stream_id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);
  let stream_addr = BASE_ADDR.saturating_add(stream_id.saturating_mul(STRIDE));

  stream_addr as *mut FILE
}

#[test]
fn stdio_i025_exports_expected_wrapper_signatures() {
  let _: PrintfFn = printf;
  let _: FprintfFn = fprintf;
  let _: VprintfFn = vprintf;
  let _: VfprintfFn = vfprintf;
}

#[test]
fn printf_returns_utf8_payload_byte_count_for_percent_s() {
  let format = c_string("%s");
  let payload = c_string("寿司🍣");

  // SAFETY: `format` and `payload` are valid NUL-terminated C strings.
  let written = unsafe { printf(format.as_ptr(), payload.as_ptr()) };

  assert_eq!(written, 10);
}

#[test]
fn printf_handles_dynamic_width_and_precision_for_string() {
  let format = c_string("%*.*s");
  let payload = c_string("abcdef");

  // SAFETY: format string arguments match `%*.*s` contract.
  let written = unsafe { printf(format.as_ptr(), 5_i32, 3_i32, payload.as_ptr()) };

  assert_eq!(written, 5);
}

#[test]
fn printf_percent_n_records_emitted_byte_count() {
  let format = c_string("xy%nz");
  let mut count = -1_i32;

  // SAFETY: `%n` receives a valid mutable `int*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 3);
  assert_eq!(count, 2);
}

#[test]
fn printf_percent_n_records_zero_for_empty_prefix() {
  let format = c_string("%n");
  let mut count = -1_i32;

  // SAFETY: `%n` receives a valid mutable `int*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn printf_percent_jn_records_zero_for_empty_prefix() {
  let format = c_string("%jn");
  let mut count: i64 = -1;

  // SAFETY: `%jn` receives a valid mutable `intmax_t*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn printf_percent_tn_records_zero_for_empty_prefix() {
  let format = c_string("%tn");
  let mut count: isize = -1;

  // SAFETY: `%tn` receives a valid mutable `ptrdiff_t*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn printf_percent_zn_records_zero_for_empty_prefix() {
  let format = c_string("%zn");
  let mut count = usize::MAX;

  // SAFETY: `%zn` receives a valid mutable `size_t*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn printf_percent_hhn_records_zero_for_empty_prefix() {
  let format = c_string("%hhn");
  let mut count = -1_i8;

  // SAFETY: `%hhn` receives a valid mutable `signed char*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn printf_percent_hn_records_zero_for_empty_prefix() {
  let format = c_string("%hn");
  let mut count = -1_i16;

  // SAFETY: `%hn` receives a valid mutable `short*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn printf_percent_ln_records_zero_for_empty_prefix() {
  let format = c_string("%ln");
  let mut count: c_long = -1;

  // SAFETY: `%ln` receives a valid mutable `long*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn printf_percent_lln_records_zero_for_empty_prefix() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;

  // SAFETY: `%lln` receives a valid mutable `long long*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn printf_percent_n_records_utf8_byte_count() {
  let format = c_string("%s%n");
  let payload = c_string("寿司");
  let mut count = -1_i32;

  // SAFETY: variadic arguments satisfy `%s%n` contract (`char*`, `int*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 6);
  assert_eq!(count, 6);
}

#[test]
fn printf_mixed_count_conversions_track_progress_per_conversion() {
  let format = c_string("A%nB%jnC%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;

  // SAFETY: variadic arguments match `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe {
    printf(
      format.as_ptr(),
      core::ptr::addr_of_mut!(count_n),
      core::ptr::addr_of_mut!(count_j),
      core::ptr::addr_of_mut!(count_t),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count_n, 1);
  assert_eq!(count_j, 2);
  assert_eq!(count_t, 3);
}

#[test]
fn printf_mixed_count_conversions_success_does_not_clobber_errno() {
  let format = c_string("A%nB%jnC%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe {
    printf(
      format.as_ptr(),
      core::ptr::addr_of_mut!(count_n),
      core::ptr::addr_of_mut!(count_j),
      core::ptr::addr_of_mut!(count_t),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count_n, 1);
  assert_eq!(count_j, 2);
  assert_eq!(count_t, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_mixed_zero_prefix_conversions_success_does_not_clobber_errno() {
  let format = c_string("%n%jn%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe {
    printf(
      format.as_ptr(),
      core::ptr::addr_of_mut!(count_n),
      core::ptr::addr_of_mut!(count_j),
      core::ptr::addr_of_mut!(count_t),
    )
  };

  assert_eq!(written, 0);
  assert_eq!(count_n, 0);
  assert_eq!(count_j, 0);
  assert_eq!(count_t, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_n_success_does_not_clobber_errno() {
  let format = c_string("%s%n");
  let payload = c_string("abc");
  let mut count = -1_i32;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments satisfy `%s%n` contract (`char*`, `int*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_zn_success_does_not_clobber_errno() {
  let format = c_string("%s%zn");
  let payload = c_string("abc");
  let mut count = usize::MAX;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments satisfy `%s%zn` contract (`char*`, `size_t*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_zn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%zn");
  let mut count = usize::MAX;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: `%zn` receives a valid mutable `size_t*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_jn_success_does_not_clobber_errno() {
  let format = c_string("%s%jn");
  let payload = c_string("abc");
  let mut count: i64 = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments satisfy `%s%jn` contract (`char*`, `intmax_t*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_tn_success_does_not_clobber_errno() {
  let format = c_string("%s%tn");
  let payload = c_string("abc");
  let mut count: isize = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments satisfy `%s%tn` contract (`char*`, `ptrdiff_t*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_hhn_success_does_not_clobber_errno() {
  let format = c_string("%s%hhn");
  let payload = c_string("abc");
  let mut count: i8 = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments satisfy `%s%hhn` contract (`char*`, `signed char*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_hhn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%hhn");
  let mut count: i8 = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: `%hhn` receives a valid mutable `signed char*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_hn_success_does_not_clobber_errno() {
  let format = c_string("%s%hn");
  let payload = c_string("abc");
  let mut count: i16 = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments satisfy `%s%hn` contract (`char*`, `short*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_hn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%hn");
  let mut count: i16 = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: `%hn` receives a valid mutable `short*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_ln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: `%ln` receives a valid mutable `long*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_lln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: `%lln` receives a valid mutable `long long*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_ln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: `%ln` receives a valid mutable `long*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_lln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: `%lln` receives a valid mutable `long long*`.
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_ln_success_does_not_clobber_errno() {
  let format = c_string("%s%ln");
  let payload = c_string("abc");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments satisfy `%s%ln` contract (`char*`, `long*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_lln_success_does_not_clobber_errno() {
  let format = c_string("%s%lln");
  let payload = c_string("abc");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments satisfy `%s%lln` contract (`char*`, `long long*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn printf_percent_hhn_records_emitted_byte_count() {
  let format = c_string("abcd%hhn");
  let mut count = -1_i8;

  // SAFETY: variadic arguments satisfy `%hhn` contract (`signed char*`).
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 4);
  assert_eq!(count, 4);
}

#[test]
fn printf_percent_hhn_accepts_i8_max_boundary() {
  let format = c_string("%*s%hhn");
  let payload = c_string("");
  let mut count = -1_i8;
  let width = i32::from(i8::MAX);

  // SAFETY: variadic arguments satisfy `%*s%hhn` contract (`int`, `char*`, `signed char*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      width,
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, width);
  assert_eq!(count, i8::MAX);
}

#[test]
fn printf_percent_hn_accepts_i16_max_boundary() {
  let format = c_string("%*s%hn");
  let payload = c_string("");
  let mut count = -1_i16;
  let width = i32::from(i16::MAX);

  // SAFETY: variadic arguments satisfy `%*s%hn` contract (`int`, `char*`, `short*`).
  let written = unsafe {
    printf(
      format.as_ptr(),
      width,
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, width);
  assert_eq!(count, i16::MAX);
}

#[test]
fn printf_percent_hn_records_emitted_byte_count() {
  let format = c_string("abcde%hn");
  let mut count = -1_i16;

  // SAFETY: variadic arguments satisfy `%hn` contract (`short*`).
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 5);
  assert_eq!(count, 5);
}

#[test]
fn printf_percent_ln_records_emitted_byte_count() {
  let format = c_string("abcdef%ln");
  let mut count: c_long = -1;

  // SAFETY: variadic arguments satisfy `%ln` contract (`long*`).
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 6);
  assert_eq!(count, 6);
}

#[test]
fn printf_percent_lln_records_emitted_byte_count() {
  let format = c_string("abcdefg%lln");
  let mut count: c_longlong = -1;

  // SAFETY: variadic arguments satisfy `%lln` contract (`long long*`).
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 7);
  assert_eq!(count, 7);
}

#[test]
fn printf_percent_zn_records_emitted_byte_count() {
  let format = c_string("abcdefgh%zn");
  let mut count = usize::MAX;

  // SAFETY: variadic arguments satisfy `%zn` contract (`size_t*`).
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 8);
  assert_eq!(count, 8);
}

#[test]
fn printf_percent_jn_records_emitted_byte_count() {
  let format = c_string("abcdefghi%jn");
  let mut count: i64 = -1;

  // SAFETY: variadic arguments satisfy `%jn` contract (`intmax_t*`).
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 9);
  assert_eq!(count, 9);
}

#[test]
fn printf_percent_tn_records_emitted_byte_count() {
  let format = c_string("abcdefghij%tn");
  let mut count: isize = -1;

  // SAFETY: variadic arguments satisfy `%tn` contract (`ptrdiff_t*`).
  let written = unsafe { printf(format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 10);
  assert_eq!(count, 10);
}

#[test]
fn printf_success_marks_stdout_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut user_buffer = [0_u8; 8];

  // SAFETY: reading host libc stdout stream pointer for API call boundary.
  let stdout_stream = unsafe { stdout };

  assert!(!stdout_stream.is_null());

  write_errno(0);

  // SAFETY: pointers satisfy C ABI contracts for `printf`/`setvbuf`.
  let (written, setvbuf_status) = unsafe {
    (
      printf(format.as_ptr(), payload.as_ptr()),
      setvbuf(stdout_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn vprintf_returns_payload_byte_count_for_percent_s() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(payload.as_ptr())]);

  // SAFETY: `format` and `args` satisfy the C ABI and `%s` argument contract.
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
}

#[test]
fn vprintf_percent_n_records_emitted_byte_count() {
  let format = c_string("xy%nz");
  let mut count = -1_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%n` contract (`int*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 2);
}

#[test]
fn vprintf_percent_n_records_zero_for_empty_prefix() {
  let format = c_string("%n");
  let mut count = -1_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%n` contract (`int*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vprintf_percent_jn_records_zero_for_empty_prefix() {
  let format = c_string("%jn");
  let mut count: i64 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%jn` contract (`intmax_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vprintf_percent_tn_records_zero_for_empty_prefix() {
  let format = c_string("%tn");
  let mut count: isize = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%tn` contract (`ptrdiff_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vprintf_percent_zn_records_zero_for_empty_prefix() {
  let format = c_string("%zn");
  let mut count = usize::MAX;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%zn` contract (`size_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vprintf_percent_hhn_records_zero_for_empty_prefix() {
  let format = c_string("%hhn");
  let mut count = -1_i8;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%hhn` contract (`signed char*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vprintf_percent_hn_records_zero_for_empty_prefix() {
  let format = c_string("%hn");
  let mut count = -1_i16;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%hn` contract (`short*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vprintf_percent_ln_records_zero_for_empty_prefix() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%ln` contract (`long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vprintf_percent_lln_records_zero_for_empty_prefix() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%lln` contract (`long long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vprintf_percent_n_records_utf8_byte_count() {
  let format = c_string("%s%n");
  let payload = c_string("寿司");
  let mut count = -1_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  // SAFETY: va_list slots satisfy `%s%n` contract (`char*`, `int*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 6);
  assert_eq!(count, 6);
}

#[test]
fn vprintf_mixed_count_conversions_track_progress_per_conversion() {
  let format = c_string("A%nB%jnC%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_n)),
    ptr_slot(core::ptr::addr_of_mut!(count_j)),
    ptr_slot(core::ptr::addr_of_mut!(count_t)),
  ]);

  // SAFETY: va_list slots satisfy `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count_n, 1);
  assert_eq!(count_j, 2);
  assert_eq!(count_t, 3);
}

#[test]
fn vprintf_mixed_count_conversions_success_does_not_clobber_errno() {
  let format = c_string("A%nB%jnC%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_n)),
    ptr_slot(core::ptr::addr_of_mut!(count_j)),
    ptr_slot(core::ptr::addr_of_mut!(count_t)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count_n, 1);
  assert_eq!(count_j, 2);
  assert_eq!(count_t, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_mixed_zero_prefix_conversions_success_does_not_clobber_errno() {
  let format = c_string("%n%jn%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_n)),
    ptr_slot(core::ptr::addr_of_mut!(count_j)),
    ptr_slot(core::ptr::addr_of_mut!(count_t)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count_n, 0);
  assert_eq!(count_j, 0);
  assert_eq!(count_t, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_n_success_does_not_clobber_errno() {
  let format = c_string("%s%n");
  let payload = c_string("abc");
  let mut count = -1_i32;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%s%n` contract (`char*`, `int*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_zn_success_does_not_clobber_errno() {
  let format = c_string("%s%zn");
  let payload = c_string("abc");
  let mut count = usize::MAX;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%s%zn` contract (`char*`, `size_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_zn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%zn");
  let mut count = usize::MAX;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%zn` contract (`size_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_jn_success_does_not_clobber_errno() {
  let format = c_string("%s%jn");
  let payload = c_string("abc");
  let mut count: i64 = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%s%jn` contract (`char*`, `intmax_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_tn_success_does_not_clobber_errno() {
  let format = c_string("%s%tn");
  let payload = c_string("abc");
  let mut count: isize = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%s%tn` contract (`char*`, `ptrdiff_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_hhn_success_does_not_clobber_errno() {
  let format = c_string("%s%hhn");
  let payload = c_string("abc");
  let mut count: i8 = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%s%hhn` contract (`char*`, `signed char*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_hhn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%hhn");
  let mut count: i8 = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%hhn` contract (`signed char*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_hn_success_does_not_clobber_errno() {
  let format = c_string("%s%hn");
  let payload = c_string("abc");
  let mut count: i16 = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%s%hn` contract (`char*`, `short*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_hn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%hn");
  let mut count: i16 = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%hn` contract (`short*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_ln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%ln` contract (`long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_lln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%lln` contract (`long long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_ln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%ln` contract (`long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_lln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%lln` contract (`long long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_ln_success_does_not_clobber_errno() {
  let format = c_string("%s%ln");
  let payload = c_string("abc");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%s%ln` contract (`char*`, `long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_lln_success_does_not_clobber_errno() {
  let format = c_string("%s%lln");
  let payload = c_string("abc");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  write_errno(sentinel_errno);

  // SAFETY: va_list slots satisfy `%s%lln` contract (`char*`, `long long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vprintf_percent_hhn_records_emitted_byte_count() {
  let format = c_string("abcd%hhn");
  let mut count = -1_i8;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%hhn` contract (`signed char*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 4);
  assert_eq!(count, 4);
}

#[test]
fn vprintf_percent_hhn_accepts_i8_max_boundary() {
  let format = c_string("%*s%hhn");
  let payload = c_string("");
  let mut count = -1_i8;
  let mut args = OwnedVaList::from_u64_slots(vec![
    u64::from(127_u32),
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  // SAFETY: va_list slots satisfy `%*s%hhn` contract (`int`, `char*`, `signed char*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, i32::from(i8::MAX));
  assert_eq!(count, i8::MAX);
}

#[test]
fn vprintf_percent_hn_accepts_i16_max_boundary() {
  let format = c_string("%*s%hn");
  let payload = c_string("");
  let mut count = -1_i16;
  let mut args = OwnedVaList::from_u64_slots(vec![
    u64::from(32_767_u32),
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  // SAFETY: va_list slots satisfy `%*s%hn` contract (`int`, `char*`, `short*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, i32::from(i16::MAX));
  assert_eq!(count, i16::MAX);
}

#[test]
fn vprintf_percent_hn_records_emitted_byte_count() {
  let format = c_string("abcde%hn");
  let mut count = -1_i16;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%hn` contract (`short*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 5);
  assert_eq!(count, 5);
}

#[test]
fn vprintf_percent_ln_records_emitted_byte_count() {
  let format = c_string("abcdef%ln");
  let mut count: c_long = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%ln` contract (`long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 6);
  assert_eq!(count, 6);
}

#[test]
fn vprintf_percent_lln_records_emitted_byte_count() {
  let format = c_string("abcdefg%lln");
  let mut count: c_longlong = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%lln` contract (`long long*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 7);
  assert_eq!(count, 7);
}

#[test]
fn vprintf_percent_zn_records_emitted_byte_count() {
  let format = c_string("abcdefgh%zn");
  let mut count = usize::MAX;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%zn` contract (`size_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 8);
  assert_eq!(count, 8);
}

#[test]
fn vprintf_percent_jn_records_emitted_byte_count() {
  let format = c_string("abcdefghi%jn");
  let mut count: i64 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%jn` contract (`intmax_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 9);
  assert_eq!(count, 9);
}

#[test]
fn vprintf_percent_tn_records_emitted_byte_count() {
  let format = c_string("abcdefghij%tn");
  let mut count: isize = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: va_list slots satisfy `%tn` contract (`ptrdiff_t*`).
  let written = unsafe { vprintf(format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 10);
  assert_eq!(count, 10);
}

#[test]
fn vprintf_success_marks_stdout_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(payload.as_ptr())]);
  let mut user_buffer = [0_u8; 8];

  // SAFETY: reading host libc stdout stream pointer for API call boundary.
  let stdout_stream = unsafe { stdout };

  assert!(!stdout_stream.is_null());

  write_errno(83);

  // SAFETY: pointers and varargs satisfy C ABI contracts for `vprintf`/`setvbuf`.
  let (written, setvbuf_status) = unsafe {
    (
      vprintf(format.as_ptr(), args.as_mut_ptr()),
      setvbuf(stdout_stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn vfprintf_writes_to_target_stream_and_reports_byte_count() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(payload.as_ptr())]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: `stream`, `format`, and `args` follow C ABI contracts.
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };
  let mut output = [0_u8; 8];
  // SAFETY: valid stream from `tmpfile`.
  unsafe { rewind(stream) };
  // SAFETY: output buffer is writable and stream is readable.
  let bytes_read = unsafe {
    fread(
      output.as_mut_ptr().cast::<c_void>(),
      1,
      output.len(),
      stream,
    )
  };
  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 3);
  assert_eq!(bytes_read, 3);
  assert_eq!(&output[..3], b"abc");
}

#[test]
fn vfprintf_handles_dynamic_width_and_precision_for_string() {
  let format = c_string("%*.*s");
  let payload = c_string("abcdef");
  let mut args = OwnedVaList::from_u64_slots(vec![5, 3, ptr_slot(payload.as_ptr())]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: argument slots match `%*.*s` contract (`int`, `int`, `char*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };
  let mut output = [0_u8; 8];
  // SAFETY: valid stream from `tmpfile`.
  unsafe { rewind(stream) };
  // SAFETY: output buffer is writable and stream is readable.
  let bytes_read = unsafe {
    fread(
      output.as_mut_ptr().cast::<c_void>(),
      1,
      output.len(),
      stream,
    )
  };
  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 5);
  assert_eq!(bytes_read, 5);
  assert_eq!(&output[..5], b"  abc");
}

#[test]
fn vfprintf_percent_n_records_emitted_byte_count() {
  let format = c_string("xy%nz");
  let mut count = -1_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%n` contract (`int*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 3);
  assert_eq!(count, 2);
}

#[test]
fn vfprintf_percent_hhn_records_emitted_byte_count() {
  let format = c_string("abcd%hhn");
  let mut count = -1_i8;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%hhn` contract (`signed char*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 4);
  assert_eq!(count, 4);
}

#[test]
fn vfprintf_percent_hhn_accepts_i8_max_boundary() {
  let format = c_string("%*s%hhn");
  let payload = c_string("");
  let mut count = -1_i8;
  let mut args = OwnedVaList::from_u64_slots(vec![
    u64::from(127_u32),
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%*s%hhn` contract (`int`, `char*`, `signed char*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, i32::from(i8::MAX));
  assert_eq!(count, i8::MAX);
}

#[test]
fn vfprintf_percent_hn_accepts_i16_max_boundary() {
  let format = c_string("%*s%hn");
  let payload = c_string("");
  let mut count = -1_i16;
  let mut args = OwnedVaList::from_u64_slots(vec![
    u64::from(32_767_u32),
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%*s%hn` contract (`int`, `char*`, `short*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, i32::from(i16::MAX));
  assert_eq!(count, i16::MAX);
}

#[test]
fn vfprintf_percent_hn_records_emitted_byte_count() {
  let format = c_string("abcde%hn");
  let mut count = -1_i16;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%hn` contract (`short*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 5);
  assert_eq!(count, 5);
}

#[test]
fn vfprintf_percent_ln_records_emitted_byte_count() {
  let format = c_string("abcdef%ln");
  let mut count: c_long = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%ln` contract (`long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 6);
  assert_eq!(count, 6);
}

#[test]
fn vfprintf_percent_lln_records_emitted_byte_count() {
  let format = c_string("abcdefg%lln");
  let mut count: c_longlong = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%lln` contract (`long long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 7);
  assert_eq!(count, 7);
}

#[test]
fn vfprintf_percent_zn_records_emitted_byte_count() {
  let format = c_string("abcdefgh%zn");
  let mut count = usize::MAX;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%zn` contract (`size_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 8);
  assert_eq!(count, 8);
}

#[test]
fn vfprintf_percent_jn_records_emitted_byte_count() {
  let format = c_string("abcdefghi%jn");
  let mut count: i64 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%jn` contract (`intmax_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 9);
  assert_eq!(count, 9);
}

#[test]
fn vfprintf_percent_tn_records_emitted_byte_count() {
  let format = c_string("abcdefghij%tn");
  let mut count: isize = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%tn` contract (`ptrdiff_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 10);
  assert_eq!(count, 10);
}

#[test]
fn vfprintf_percent_n_records_zero_for_empty_prefix() {
  let format = c_string("%n");
  let mut count = -1_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%n` contract (`int*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vfprintf_percent_jn_records_zero_for_empty_prefix() {
  let format = c_string("%jn");
  let mut count: i64 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%jn` contract (`intmax_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vfprintf_percent_tn_records_zero_for_empty_prefix() {
  let format = c_string("%tn");
  let mut count: isize = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%tn` contract (`ptrdiff_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vfprintf_percent_zn_records_zero_for_empty_prefix() {
  let format = c_string("%zn");
  let mut count = usize::MAX;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%zn` contract (`size_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vfprintf_percent_hhn_records_zero_for_empty_prefix() {
  let format = c_string("%hhn");
  let mut count = -1_i8;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%hhn` contract (`signed char*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vfprintf_percent_hn_records_zero_for_empty_prefix() {
  let format = c_string("%hn");
  let mut count = -1_i16;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%hn` contract (`short*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vfprintf_percent_ln_records_zero_for_empty_prefix() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%ln` contract (`long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vfprintf_percent_lln_records_zero_for_empty_prefix() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: va_list slots satisfy `%lln` contract (`long long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn vfprintf_percent_n_records_utf8_byte_count() {
  let format = c_string("%s%n");
  let payload = c_string("寿司");
  let mut count = -1_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: argument slots satisfy `%s%n` contract (`char*`, `int*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 6);
  assert_eq!(count, 6);
}

#[test]
fn vfprintf_mixed_count_conversions_track_progress_per_conversion() {
  let format = c_string("A%nB%jnC%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_n)),
    ptr_slot(core::ptr::addr_of_mut!(count_j)),
    ptr_slot(core::ptr::addr_of_mut!(count_t)),
  ]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: argument slots satisfy `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 3);
  assert_eq!(count_n, 1);
  assert_eq!(count_j, 2);
  assert_eq!(count_t, 3);
}

#[test]
fn vfprintf_mixed_count_conversions_success_does_not_clobber_errno() {
  let format = c_string("A%nB%jnC%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_n)),
    ptr_slot(core::ptr::addr_of_mut!(count_j)),
    ptr_slot(core::ptr::addr_of_mut!(count_t)),
  ]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count_n, 1);
  assert_eq!(count_j, 2);
  assert_eq!(count_t, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_mixed_zero_prefix_conversions_success_does_not_clobber_errno() {
  let format = c_string("%n%jn%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let sentinel_errno = 1234_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_n)),
    ptr_slot(core::ptr::addr_of_mut!(count_j)),
    ptr_slot(core::ptr::addr_of_mut!(count_t)),
  ]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count_n, 0);
  assert_eq!(count_j, 0);
  assert_eq!(count_t, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn vfprintf_percent_n_success_does_not_clobber_errno() {
  let format = c_string("%s%n");
  let payload = c_string("abc");
  let mut count = -1_i32;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%s%n` contract (`char*`, `int*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_zn_success_does_not_clobber_errno() {
  let format = c_string("%s%zn");
  let payload = c_string("abc");
  let mut count = usize::MAX;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%s%zn` contract (`char*`, `size_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_zn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%zn");
  let mut count = usize::MAX;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%zn` contract (`size_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_jn_success_does_not_clobber_errno() {
  let format = c_string("%s%jn");
  let payload = c_string("abc");
  let mut count: i64 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%s%jn` contract (`char*`, `intmax_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_tn_success_does_not_clobber_errno() {
  let format = c_string("%s%tn");
  let payload = c_string("abc");
  let mut count: isize = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%s%tn` contract (`char*`, `ptrdiff_t*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_hhn_success_does_not_clobber_errno() {
  let format = c_string("%s%hhn");
  let payload = c_string("abc");
  let mut count: i8 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%s%hhn` contract (`char*`, `signed char*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_hhn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%hhn");
  let mut count: i8 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%hhn` contract (`signed char*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_hn_success_does_not_clobber_errno() {
  let format = c_string("%s%hn");
  let payload = c_string("abc");
  let mut count: i16 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%s%hn` contract (`char*`, `short*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_hn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%hn");
  let mut count: i16 = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%hn` contract (`short*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_ln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%ln` contract (`long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_lln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%lln` contract (`long long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_ln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%ln` contract (`long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_lln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count))]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%lln` contract (`long long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_ln_success_does_not_clobber_errno() {
  let format = c_string("%s%ln");
  let payload = c_string("abc");
  let mut count: c_long = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%s%ln` contract (`char*`, `long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_percent_lln_success_does_not_clobber_errno() {
  let format = c_string("%s%lln");
  let payload = c_string("abc");
  let mut count: c_longlong = -1;
  let mut args = OwnedVaList::from_u64_slots(vec![
    ptr_slot(payload.as_ptr()),
    ptr_slot(core::ptr::addr_of_mut!(count)),
  ]);
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: argument slots satisfy `%s%lln` contract (`char*`, `long long*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_zero_precision_suppresses_string_payload() {
  let format = c_string("%.0s");
  let payload = c_string("abcdef");
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(payload.as_ptr())]);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: argument slot matches `%.0s` contract (`char*`).
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };
  let mut output = [0_u8; 8];
  // SAFETY: valid stream from `tmpfile`.
  unsafe { rewind(stream) };
  // SAFETY: output buffer is writable and stream is readable.
  let bytes_read = unsafe {
    fread(
      output.as_mut_ptr().cast::<c_void>(),
      1,
      output.len(),
      stream,
    )
  };
  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(bytes_read, 0);
}

#[test]
fn vfprintf_null_stream_returns_einval() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(payload.as_ptr())]);

  write_errno(79);

  // SAFETY: null stream pointer intentionally exercises API error contract.
  let written = unsafe { vfprintf(core::ptr::null_mut(), format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn vfprintf_null_stream_error_does_not_mark_unrelated_stream_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(payload.as_ptr())]);
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(79);

  // SAFETY: `setvbuf` treats this synthetic pointer as an opaque stream key.
  let initial_setvbuf_status = unsafe { setvbuf(stream, core::ptr::null_mut(), _IONBF, 0) };

  assert_eq!(initial_setvbuf_status, 0);
  assert_eq!(read_errno(), 79);

  write_errno(83);

  // SAFETY: null stream pointer intentionally exercises API error contract.
  let written = unsafe { vfprintf(core::ptr::null_mut(), format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(79);

  // SAFETY: stream and buffer satisfy `setvbuf` contract.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(second_setvbuf_status, 0);
  assert_eq!(read_errno(), 79);
}

#[test]
fn vfprintf_null_format_returns_einval() {
  let mut args = OwnedVaList::from_u64_slots(Vec::new());

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { vfprintf(stream, core::ptr::null(), args.as_mut_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_null_format_error_does_not_mark_unrelated_stream_as_io_active_for_setvbuf() {
  let mut args = OwnedVaList::from_u64_slots(Vec::new());
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: `setvbuf` treats this synthetic pointer as an opaque stream key.
  let initial_setvbuf_status = unsafe { setvbuf(stream, core::ptr::null_mut(), _IONBF, 0) };

  assert_eq!(initial_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let host_stream = unsafe { tmpfile() };

  assert!(!host_stream.is_null());

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { vfprintf(host_stream, core::ptr::null(), args.as_mut_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(83);

  // SAFETY: stream and buffer satisfy `setvbuf` contract.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(second_setvbuf_status, 0);
  assert_eq!(read_errno(), 83);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(host_stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_error_does_not_mark_stream_as_io_active_for_setvbuf() {
  let mut args = OwnedVaList::from_u64_slots(Vec::new());
  let mut user_buffer = [0_u8; 8];
  let mut skipped_streams = Vec::new();
  let stream = loop {
    // SAFETY: `tmpfile` returns a stream managed by host libc.
    let candidate = unsafe { tmpfile() };

    assert!(!candidate.is_null());

    write_errno(0);

    // SAFETY: stream pointer is valid and unbuffered mode accepts null buffer.
    let initial_setvbuf_status = unsafe { setvbuf(candidate, core::ptr::null_mut(), _IONBF, 0) };

    if initial_setvbuf_status == 0 {
      assert_eq!(read_errno(), 0);
      break candidate;
    }

    assert_eq!(initial_setvbuf_status, EOF);
    assert_eq!(read_errno(), EINVAL);
    skipped_streams.push(candidate);
    assert!(
      skipped_streams.len() < 16,
      "failed to acquire an untracked stream for vfprintf-error path test",
    );
  };

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { vfprintf(stream, core::ptr::null(), args.as_mut_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(83);

  // SAFETY: stream and user buffer satisfy `setvbuf` contract.
  let setvbuf_status = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 83);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);

  for skipped_stream in skipped_streams {
    // SAFETY: each skipped stream came from `tmpfile` and must be closed.
    let skipped_close_result = unsafe { fclose(skipped_stream) };

    assert_eq!(skipped_close_result, 0);
  }
}

#[test]
fn vfprintf_null_ap_returns_einval() {
  let format = c_string("%s");

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(0);

  // SAFETY: null va_list pointer intentionally exercises API error contract.
  let written = unsafe { vfprintf(stream, format.as_ptr(), core::ptr::null_mut()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_null_ap_does_not_mark_stream_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: null va_list pointer intentionally exercises early validation path;
  // the function returns before touching stream internals.
  let written = unsafe { vfprintf(stream, format.as_ptr(), core::ptr::null_mut()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  // SAFETY: `setvbuf` treats stream handle as opaque key in this phase.
  let setvbuf_status = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 0);
}

#[test]
fn vfprintf_null_ap_does_not_mark_stream_as_io_active_for_buffered_setvbuf() {
  let format = c_string("%s");
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: null va_list pointer intentionally exercises early validation path;
  // the function returns before touching stream internals.
  let written = unsafe { vfprintf(stream, format.as_ptr(), core::ptr::null_mut()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  // SAFETY: synthetic stream key and buffer are valid metadata for this call.
  let setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 0);
}

#[test]
fn vfprintf_null_ap_error_does_not_mark_unrelated_stream_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: `setvbuf` treats this synthetic pointer as an opaque stream key.
  let initial_setvbuf_status = unsafe { setvbuf(stream, core::ptr::null_mut(), _IONBF, 0) };

  assert_eq!(initial_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let host_stream = unsafe { tmpfile() };

  assert!(!host_stream.is_null());

  write_errno(0);

  // SAFETY: null va_list pointer intentionally exercises API error contract.
  let written = unsafe { vfprintf(host_stream, format.as_ptr(), core::ptr::null_mut()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  // SAFETY: synthetic stream and buffer satisfy `setvbuf` tracking contract.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(second_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(host_stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_success_marks_stream_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(payload.as_ptr())]);
  let mut user_buffer = [0_u8; 8];

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(0);

  // SAFETY: stream pointer, format string, va_list, and buffer all satisfy C ABI contracts.
  let (written, setvbuf_status) = unsafe {
    (
      vfprintf(stream, format.as_ptr(), args.as_mut_ptr()),
      setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: release host stream state allocated by `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn vfprintf_host_write_error_marks_stream_as_io_active_for_setvbuf() {
  let path = c_string("/dev/null");
  let mode = c_string("r");
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut args = OwnedVaList::from_u64_slots(vec![ptr_slot(payload.as_ptr())]);
  let mut user_buffer = [0_u8; 8];
  let mut skipped_stream_count = 0_usize;
  let stream = loop {
    // SAFETY: path/mode are valid NUL-terminated strings.
    let candidate = unsafe { fopen(path.as_ptr(), mode.as_ptr()) };

    assert!(!candidate.is_null());

    write_errno(0);

    // SAFETY: stream pointer is valid and unbuffered mode accepts null buffer.
    let initial_setvbuf_status = unsafe { setvbuf(candidate, core::ptr::null_mut(), _IONBF, 0) };

    if initial_setvbuf_status == 0 {
      assert_eq!(read_errno(), 0);
      break candidate;
    }

    assert_eq!(initial_setvbuf_status, EOF);
    assert_eq!(read_errno(), EINVAL);
    // SAFETY: skipped candidate came from `fopen`.
    let close_result = unsafe { fclose(candidate) };

    assert_eq!(close_result, 0);
    skipped_stream_count += 1;
    assert!(
      skipped_stream_count < 16,
      "failed to acquire an untracked stream for vfprintf-host-write-error test",
    );
  };

  write_errno(0);

  // SAFETY: stream, format, and va_list satisfy C ABI contracts.
  let written = unsafe { vfprintf(stream, format.as_ptr(), args.as_mut_ptr()) };

  assert_eq!(written, -1);

  write_errno(0);

  // SAFETY: stream and buffer pointers satisfy `setvbuf` contract.
  let setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: stream came from `fopen`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_success_marks_stream_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut user_buffer = [0_u8; 8];

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(0);

  // SAFETY: stream pointer, format string, payload string, and buffer satisfy C ABI contracts.
  let (written, setvbuf_status) = unsafe {
    (
      fprintf(stream, format.as_ptr(), payload.as_ptr()),
      setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(setvbuf_status, EOF);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: release host stream state allocated by `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_handles_dynamic_width_and_precision_for_string() {
  let format = c_string("%*.*s");
  let payload = c_string("abcdef");

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%*.*s` contract (`int`, `int`, `char*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), 5_i32, 3_i32, payload.as_ptr()) };
  let mut output = [0_u8; 8];
  // SAFETY: valid stream from `tmpfile`.
  unsafe { rewind(stream) };
  // SAFETY: output buffer is writable and stream is readable.
  let bytes_read = unsafe {
    fread(
      output.as_mut_ptr().cast::<c_void>(),
      1,
      output.len(),
      stream,
    )
  };
  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 5);
  assert_eq!(bytes_read, 5);
  assert_eq!(&output[..5], b"  abc");
}

#[test]
fn fprintf_percent_n_records_emitted_byte_count() {
  let format = c_string("xy%nz");
  let mut count = -1_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%n` contract (`int*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 3);
  assert_eq!(count, 2);
}

#[test]
fn fprintf_percent_hhn_records_emitted_byte_count() {
  let format = c_string("abcd%hhn");
  let mut count = -1_i8;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%hhn` contract (`signed char*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 4);
  assert_eq!(count, 4);
}

#[test]
fn fprintf_percent_hhn_accepts_i8_max_boundary() {
  let format = c_string("%*s%hhn");
  let payload = c_string("");
  let mut count = -1_i8;
  let width = i32::from(i8::MAX);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%*s%hhn` contract (`int`, `char*`, `signed char*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      width,
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, width);
  assert_eq!(count, i8::MAX);
}

#[test]
fn fprintf_percent_hn_accepts_i16_max_boundary() {
  let format = c_string("%*s%hn");
  let payload = c_string("");
  let mut count = -1_i16;
  let width = i32::from(i16::MAX);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%*s%hn` contract (`int`, `char*`, `short*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      width,
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, width);
  assert_eq!(count, i16::MAX);
}

#[test]
fn fprintf_percent_hn_records_emitted_byte_count() {
  let format = c_string("abcde%hn");
  let mut count = -1_i16;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%hn` contract (`short*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 5);
  assert_eq!(count, 5);
}

#[test]
fn fprintf_percent_ln_records_emitted_byte_count() {
  let format = c_string("abcdef%ln");
  let mut count: c_long = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%ln` contract (`long*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 6);
  assert_eq!(count, 6);
}

#[test]
fn fprintf_percent_lln_records_emitted_byte_count() {
  let format = c_string("abcdefg%lln");
  let mut count: c_longlong = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%lln` contract (`long long*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 7);
  assert_eq!(count, 7);
}

#[test]
fn fprintf_percent_zn_records_emitted_byte_count() {
  let format = c_string("abcdefgh%zn");
  let mut count = usize::MAX;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%zn` contract (`size_t*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 8);
  assert_eq!(count, 8);
}

#[test]
fn fprintf_percent_jn_records_emitted_byte_count() {
  let format = c_string("abcdefghi%jn");
  let mut count: i64 = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%jn` contract (`intmax_t*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 9);
  assert_eq!(count, 9);
}

#[test]
fn fprintf_percent_tn_records_emitted_byte_count() {
  let format = c_string("abcdefghij%tn");
  let mut count: isize = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%tn` contract (`ptrdiff_t*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 10);
  assert_eq!(count, 10);
}

#[test]
fn fprintf_percent_n_records_utf8_byte_count() {
  let format = c_string("%s%n");
  let payload = c_string("寿司");
  let mut count = -1_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%s%n` contract (`char*`, `int*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 6);
  assert_eq!(count, 6);
}

#[test]
fn fprintf_mixed_count_conversions_track_progress_per_conversion() {
  let format = c_string("A%nB%jnC%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      core::ptr::addr_of_mut!(count_n),
      core::ptr::addr_of_mut!(count_j),
      core::ptr::addr_of_mut!(count_t),
    )
  };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 3);
  assert_eq!(count_n, 1);
  assert_eq!(count_j, 2);
  assert_eq!(count_t, 3);
}

#[test]
fn fprintf_mixed_count_conversions_success_does_not_clobber_errno() {
  let format = c_string("A%nB%jnC%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      core::ptr::addr_of_mut!(count_n),
      core::ptr::addr_of_mut!(count_j),
      core::ptr::addr_of_mut!(count_t),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count_n, 1);
  assert_eq!(count_j, 2);
  assert_eq!(count_t, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_mixed_zero_prefix_conversions_success_does_not_clobber_errno() {
  let format = c_string("%n%jn%tn");
  let mut count_n: c_int = -1;
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%n`/`%jn`/`%tn` pointer contracts in order.
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      core::ptr::addr_of_mut!(count_n),
      core::ptr::addr_of_mut!(count_j),
      core::ptr::addr_of_mut!(count_t),
    )
  };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count_n, 0);
  assert_eq!(count_j, 0);
  assert_eq!(count_t, 0);
  assert_eq!(read_errno(), sentinel_errno);
}

#[test]
fn fprintf_percent_n_success_does_not_clobber_errno() {
  let format = c_string("%s%n");
  let payload = c_string("abc");
  let mut count = -1_i32;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%s%n` contract (`char*`, `int*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_zn_success_does_not_clobber_errno() {
  let format = c_string("%s%zn");
  let payload = c_string("abc");
  let mut count = usize::MAX;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%s%zn` contract (`char*`, `size_t*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_zn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%zn");
  let mut count = usize::MAX;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%zn` contract (`size_t*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_jn_success_does_not_clobber_errno() {
  let format = c_string("%s%jn");
  let payload = c_string("abc");
  let mut count: i64 = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%s%jn` contract (`char*`, `intmax_t*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_tn_success_does_not_clobber_errno() {
  let format = c_string("%s%tn");
  let payload = c_string("abc");
  let mut count: isize = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%s%tn` contract (`char*`, `ptrdiff_t*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_hhn_success_does_not_clobber_errno() {
  let format = c_string("%s%hhn");
  let payload = c_string("abc");
  let mut count: i8 = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%s%hhn` contract (`char*`, `signed char*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_hhn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%hhn");
  let mut count: i8 = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%hhn` contract (`signed char*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_hn_success_does_not_clobber_errno() {
  let format = c_string("%s%hn");
  let payload = c_string("abc");
  let mut count: i16 = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%s%hn` contract (`char*`, `short*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_hn_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%hn");
  let mut count: i16 = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%hn` contract (`short*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_ln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%ln` contract (`long*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_lln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%lln` contract (`long long*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_ln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%ln");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%ln` contract (`long*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_lln_zero_prefix_success_does_not_clobber_errno() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%lln` contract (`long long*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  assert_eq!(written, 0);
  assert_eq!(count, 0);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_ln_success_does_not_clobber_errno() {
  let format = c_string("%s%ln");
  let payload = c_string("abc");
  let mut count: c_long = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%s%ln` contract (`char*`, `long*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_lln_success_does_not_clobber_errno() {
  let format = c_string("%s%lln");
  let payload = c_string("abc");
  let mut count: c_longlong = -1;
  let sentinel_errno = 1234_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(sentinel_errno);

  // SAFETY: variadic arguments match `%s%lln` contract (`char*`, `long long*`).
  let written = unsafe {
    fprintf(
      stream,
      format.as_ptr(),
      payload.as_ptr(),
      core::ptr::addr_of_mut!(count),
    )
  };

  assert_eq!(written, 3);
  assert_eq!(count, 3);
  assert_eq!(read_errno(), sentinel_errno);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_percent_n_records_zero_for_empty_prefix() {
  let format = c_string("%n");
  let mut count = -1_i32;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%n` contract (`int*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn fprintf_percent_jn_records_zero_for_empty_prefix() {
  let format = c_string("%jn");
  let mut count: i64 = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%jn` contract (`intmax_t*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn fprintf_percent_tn_records_zero_for_empty_prefix() {
  let format = c_string("%tn");
  let mut count: isize = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%tn` contract (`ptrdiff_t*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn fprintf_percent_zn_records_zero_for_empty_prefix() {
  let format = c_string("%zn");
  let mut count = usize::MAX;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%zn` contract (`size_t*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn fprintf_percent_hhn_records_zero_for_empty_prefix() {
  let format = c_string("%hhn");
  let mut count = -1_i8;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%hhn` contract (`signed char*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn fprintf_percent_hn_records_zero_for_empty_prefix() {
  let format = c_string("%hn");
  let mut count = -1_i16;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%hn` contract (`short*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn fprintf_percent_ln_records_zero_for_empty_prefix() {
  let format = c_string("%ln");
  let mut count: c_long = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%ln` contract (`long*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn fprintf_percent_lln_records_zero_for_empty_prefix() {
  let format = c_string("%lln");
  let mut count: c_longlong = -1;

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%lln` contract (`long long*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), core::ptr::addr_of_mut!(count)) };

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(count, 0);
}

#[test]
fn fprintf_zero_precision_suppresses_string_payload() {
  let format = c_string("%.0s");
  let payload = c_string("abcdef");

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  // SAFETY: variadic arguments match `%.0s` contract (`char*`).
  let written = unsafe { fprintf(stream, format.as_ptr(), payload.as_ptr()) };
  let mut output = [0_u8; 8];
  // SAFETY: valid stream from `tmpfile`.
  unsafe { rewind(stream) };
  // SAFETY: output buffer is writable and stream is readable.
  let bytes_read = unsafe {
    fread(
      output.as_mut_ptr().cast::<c_void>(),
      1,
      output.len(),
      stream,
    )
  };
  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
  assert_eq!(written, 0);
  assert_eq!(bytes_read, 0);
}

#[test]
fn fprintf_null_stream_returns_einval() {
  let format = c_string("%s");
  let payload = c_string("abc");

  write_errno(0);

  // SAFETY: null stream pointer intentionally exercises API error contract.
  let written = unsafe { fprintf(core::ptr::null_mut(), format.as_ptr(), payload.as_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fprintf_null_stream_error_does_not_mark_unrelated_stream_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let payload = c_string("abc");
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: `setvbuf` treats this synthetic pointer as an opaque stream key.
  let initial_setvbuf_status = unsafe { setvbuf(stream, core::ptr::null_mut(), _IONBF, 0) };

  assert_eq!(initial_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  write_errno(0);

  // SAFETY: null stream pointer intentionally exercises API error contract.
  let written = unsafe { fprintf(core::ptr::null_mut(), format.as_ptr(), payload.as_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(79);

  // SAFETY: stream and buffer satisfy `setvbuf` contract.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(second_setvbuf_status, 0);
  assert_eq!(read_errno(), 79);
}

#[test]
fn fprintf_null_format_returns_einval() {
  let payload = c_string("abc");

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let stream = unsafe { tmpfile() };

  assert!(!stream.is_null());

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { fprintf(stream, core::ptr::null(), payload.as_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_null_format_error_does_not_mark_unrelated_stream_as_io_active_for_setvbuf() {
  let payload = c_string("abc");
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: `setvbuf` treats this synthetic pointer as an opaque stream key.
  let initial_setvbuf_status = unsafe { setvbuf(stream, core::ptr::null_mut(), _IONBF, 0) };

  assert_eq!(initial_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  // SAFETY: `tmpfile` returns a stream managed by host libc.
  let host_stream = unsafe { tmpfile() };

  assert!(!host_stream.is_null());

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { fprintf(host_stream, core::ptr::null(), payload.as_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  // SAFETY: synthetic stream and buffer satisfy `setvbuf` tracking contract.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(second_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(host_stream) };

  assert_eq!(close_result, 0);
}

#[test]
fn fprintf_error_does_not_mark_stream_as_io_active_for_setvbuf() {
  let payload = c_string("abc");
  let mut user_buffer = [0_u8; 8];
  let mut skipped_streams = Vec::new();
  let stream = loop {
    // SAFETY: `tmpfile` returns a stream managed by host libc.
    let candidate = unsafe { tmpfile() };

    assert!(!candidate.is_null());

    write_errno(0);

    // SAFETY: stream pointer is valid and unbuffered mode accepts null buffer.
    let initial_setvbuf_status = unsafe { setvbuf(candidate, core::ptr::null_mut(), _IONBF, 0) };

    if initial_setvbuf_status == 0 {
      assert_eq!(read_errno(), 0);
      break candidate;
    }

    assert_eq!(initial_setvbuf_status, EOF);
    assert_eq!(read_errno(), EINVAL);
    skipped_streams.push(candidate);
    assert!(
      skipped_streams.len() < 16,
      "failed to acquire an untracked stream for fprintf-error path test",
    );
  };

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { fprintf(stream, core::ptr::null(), payload.as_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(0);

  // SAFETY: stream and user buffer satisfy `setvbuf` contract.
  let setvbuf_status = unsafe { setvbuf(stream, user_buffer.as_mut_ptr().cast(), _IONBF, 0) };

  assert_eq!(setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  // SAFETY: stream came from `tmpfile`.
  let close_result = unsafe { fclose(stream) };

  assert_eq!(close_result, 0);

  for skipped_stream in skipped_streams {
    // SAFETY: each skipped stream came from `tmpfile` and must be closed.
    let skipped_close_result = unsafe { fclose(skipped_stream) };

    assert_eq!(skipped_close_result, 0);
  }
}

#[test]
fn printf_null_format_returns_einval() {
  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { printf(core::ptr::null()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn printf_null_format_error_does_not_mark_unrelated_stream_as_io_active_for_setvbuf() {
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: `setvbuf` treats this synthetic pointer as an opaque stream key.
  let initial_setvbuf_status = unsafe { setvbuf(stream, core::ptr::null_mut(), _IONBF, 0) };

  assert_eq!(initial_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { printf(core::ptr::null()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(83);

  // SAFETY: synthetic stream and buffer satisfy `setvbuf` tracking contract.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(second_setvbuf_status, 0);
  assert_eq!(read_errno(), 83);
}

#[test]
fn vprintf_null_format_returns_einval() {
  let mut args = OwnedVaList::from_u64_slots(Vec::new());

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { vprintf(core::ptr::null(), args.as_mut_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn vprintf_null_format_error_does_not_mark_unrelated_stream_as_io_active_for_setvbuf() {
  let mut args = OwnedVaList::from_u64_slots(Vec::new());
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: `setvbuf` treats this synthetic pointer as an opaque stream key.
  let initial_setvbuf_status = unsafe { setvbuf(stream, core::ptr::null_mut(), _IONBF, 0) };

  assert_eq!(initial_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  write_errno(0);

  // SAFETY: null format pointer intentionally exercises API error contract.
  let written = unsafe { vprintf(core::ptr::null(), args.as_mut_ptr()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(83);

  // SAFETY: synthetic stream and buffer satisfy `setvbuf` tracking contract.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(second_setvbuf_status, 0);
  assert_eq!(read_errno(), 83);
}

#[test]
fn vprintf_null_ap_error_does_not_mark_unrelated_stream_as_io_active_for_setvbuf() {
  let format = c_string("%s");
  let mut user_buffer = [0_u8; 8];
  let stream = synthetic_untracked_stream();

  write_errno(0);

  // SAFETY: `setvbuf` treats this synthetic pointer as an opaque stream key.
  let initial_setvbuf_status = unsafe { setvbuf(stream, core::ptr::null_mut(), _IONBF, 0) };

  assert_eq!(initial_setvbuf_status, 0);
  assert_eq!(read_errno(), 0);

  write_errno(0);

  // SAFETY: null va_list pointer intentionally exercises API error contract.
  let written = unsafe { vprintf(format.as_ptr(), core::ptr::null_mut()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);

  write_errno(73);

  // SAFETY: synthetic stream and buffer satisfy `setvbuf` tracking contract.
  let second_setvbuf_status = unsafe {
    setvbuf(
      stream,
      user_buffer.as_mut_ptr().cast(),
      _IOFBF,
      u64::try_from(user_buffer.len())
        .unwrap_or_else(|_| unreachable!("buffer length must fit into size_t")),
    )
  };

  assert_eq!(second_setvbuf_status, 0);
  assert_eq!(read_errno(), 73);
}

#[test]
fn vprintf_null_ap_returns_einval() {
  let format = c_string("%s");

  write_errno(0);

  // SAFETY: null va_list pointer intentionally exercises API error contract.
  let written = unsafe { vprintf(format.as_ptr(), core::ptr::null_mut()) };

  assert_eq!(written, -1);
  assert_eq!(read_errno(), EINVAL);
}
