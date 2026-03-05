use core::ffi::{c_char, c_int, c_void};
use rlibc::abi::errno::EINVAL;
use rlibc::abi::types::size_t;
use rlibc::errno::__errno_location;
use rlibc::stdio::vsnprintf;

#[repr(C)]
struct SysVVaList {
  gp_offset: u32,
  fp_offset: u32,
  overflow_arg_area: *mut c_void,
  reg_save_area: *mut c_void,
}

struct OwnedVaList {
  _overflow_args: Vec<u64>,
  _gp_save_area: Option<Box<[u64; 6]>>,
  raw: SysVVaList,
}

impl OwnedVaList {
  const fn from_u64_slots(mut args: Vec<u64>) -> Self {
    let overflow_arg_area = if args.is_empty() {
      core::ptr::null_mut()
    } else {
      args.as_mut_ptr().cast::<c_void>()
    };
    let raw = SysVVaList {
      gp_offset: 48,
      fp_offset: 0,
      overflow_arg_area,
      reg_save_area: core::ptr::null_mut(),
    };

    Self {
      _overflow_args: args,
      _gp_save_area: None,
      raw,
    }
  }

  fn from_register_slots(gp_offset: u32, gp_slots: [u64; 6], mut overflow_args: Vec<u64>) -> Self {
    let overflow_arg_area = if overflow_args.is_empty() {
      core::ptr::null_mut()
    } else {
      overflow_args.as_mut_ptr().cast::<c_void>()
    };
    let mut gp_save_area = Box::new(gp_slots);
    let reg_save_area = gp_save_area.as_mut_ptr().cast::<c_void>();
    let raw = SysVVaList {
      gp_offset,
      fp_offset: 0,
      overflow_arg_area,
      reg_save_area,
    };

    Self {
      _overflow_args: overflow_args,
      _gp_save_area: Some(gp_save_area),
      raw,
    }
  }

  const fn as_mut_ptr(&mut self) -> *mut c_void {
    core::ptr::addr_of_mut!(self.raw).cast::<c_void>()
  }
}

const fn as_format_ptr(bytes: &[u8]) -> *const c_char {
  bytes.as_ptr().cast()
}

fn int_slot(value: c_int) -> u64 {
  u64::from(u32::from_ne_bytes(value.to_ne_bytes()))
}

fn u32_slot(value: u32) -> u64 {
  u64::from(value)
}

const fn u64_slot(value: u64) -> u64 {
  value
}

fn usize_slot(value: usize) -> u64 {
  u64::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize must fit in u64 on x86_64-unknown-linux-gnu"))
}

fn ptr_slot<T>(pointer: *const T) -> u64 {
  u64::try_from(pointer.addr())
    .unwrap_or_else(|_| unreachable!("pointer address must fit in u64 on this target"))
}

fn set_errno(value: c_int) {
  // SAFETY: `__errno_location` returns a valid thread-local pointer.
  unsafe {
    __errno_location().write(value);
  }
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns a valid thread-local pointer.
  unsafe { __errno_location().read() }
}

fn as_size_t(value: usize) -> size_t {
  size_t::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize must fit in size_t on x86_64-unknown-linux-gnu"))
}

#[test]
fn null_buffer_with_zero_size_returns_required_length() {
  set_errno(0);
  // SAFETY: `format` is a valid NUL-terminated string; `n == 0` allows null `s`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"abc\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(read_errno(), 0);
}

#[test]
fn null_format_returns_error_and_sets_einval_without_writing() {
  let mut buffer = [b'Z'; 4];

  set_errno(0);
  // SAFETY: non-null buffer is writable; null `format` is rejected by contract.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      core::ptr::null(),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(buffer, [b'Z'; 4]);
}

#[test]
fn empty_format_writes_only_terminator_and_returns_zero() {
  let mut buffer = [b'X'; 4];

  set_errno(0);
  // SAFETY: buffer is writable and `format` is a valid empty C string.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, 0);
  assert_eq!(buffer, [0, b'X', b'X', b'X']);
  assert_eq!(read_errno(), 0);
}

#[test]
fn size_one_buffer_is_always_nul_terminated() {
  let mut buffer = [b'X'];

  set_errno(0);
  // SAFETY: buffer is writable for one byte and `format` is valid.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abc\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(buffer[0], 0);
  assert_eq!(read_errno(), 0);
}

#[test]
fn truncation_returns_full_required_length() {
  let mut buffer = [b'X'; 4];

  set_errno(0);
  // SAFETY: buffer is writable for four bytes and `format` is valid.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(buffer, [b'a', b'b', b'c', 0]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn percent_escape_is_rendered_as_single_percent() {
  let mut buffer = [0_u8; 8];

  set_errno(0);
  // SAFETY: buffer is writable and `format` is valid.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"A%%B\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(&buffer[..4], b"A%B\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn repeated_percent_escapes_report_full_length_under_truncation() {
  let mut buffer = [b'X'; 3];

  set_errno(0);
  // SAFETY: buffer is writable and `format` is valid.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%%%%%%\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(buffer, [b'%', b'%', 0]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn unsupported_specifier_returns_error_and_sets_einval() {
  let mut buffer = [b'X'; 8];

  set_errno(0);
  // SAFETY: buffer is writable and `format` is valid.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"x%fy\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..2], b"x\0");
}

#[test]
fn nonzero_size_requires_non_null_buffer() {
  set_errno(0);
  // SAFETY: null `s` with non-zero `n` is rejected by this implementation.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(3),
      as_format_ptr(b"abc\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn successful_call_does_not_clobber_existing_errno() {
  let mut buffer = [0_u8; 8];

  set_errno(EINVAL);
  // SAFETY: buffer is writable and `format` is valid.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ok\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(&buffer[..3], b"ok\0");
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn zero_capacity_leaves_non_null_buffer_untouched() {
  let mut buffer = [b'Q'; 3];

  set_errno(0);
  // SAFETY: `format` is valid; `n == 0` means no output bytes are written.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(buffer, [b'Q'; 3]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn dangling_percent_returns_error_and_nul_terminates_prefix() {
  let mut buffer = [b'X'; 5];

  set_errno(0);
  // SAFETY: buffer is writable and `format` is valid.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(buffer, [b'a', b'b', 0, b'X', b'X']);
}

#[test]
fn unsupported_specifier_at_start_nul_terminates_buffer() {
  let mut buffer = [b'Y'; 4];

  set_errno(0);
  // SAFETY: buffer is writable and `format` is valid.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%q\0"),
      core::ptr::null_mut::<c_void>(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(buffer, [0, b'Y', b'Y', b'Y']);
}

#[test]
fn percent_s_supports_width_and_precision() {
  let mut buffer = [0_u8; 16];
  let source = b"abcdef\0";
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(as_format_ptr(source))]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%5.3s\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 5);
  assert_eq!(&buffer[..6], b"  abc\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn percent_s_null_pointer_respects_precision_and_width() {
  let mut buffer = [0_u8; 16];
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::null::<c_char>())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%5.3s\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 5);
  assert_eq!(&buffer[..6], b"  (nu\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn percent_s_supports_dynamic_precision() {
  let mut buffer = [0_u8; 16];
  let source = b"abcdef\0";
  let mut ap = OwnedVaList::from_u64_slots(vec![int_slot(4), ptr_slot(as_format_ptr(source))]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%.*s\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 4);
  assert_eq!(&buffer[..5], b"abcd\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn percent_c_supports_width_and_left_adjust() {
  let mut buffer = [0_u8; 16];
  let mut ap = OwnedVaList::from_u64_slots(vec![int_slot('A' as c_int), int_slot('B' as c_int)]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"|%3c|%-3c|\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 9);
  assert_eq!(&buffer[..10], b"|  A|B  |\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn percent_s_reads_argument_from_register_save_area() {
  let mut buffer = [0_u8; 32];
  let source = b"register-source\0";
  let mut ap =
    OwnedVaList::from_register_slots(0, [ptr_slot(as_format_ptr(source)), 0, 0, 0, 0, 0], vec![]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%s\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 15);
  assert_eq!(&buffer[..16], b"register-source\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn dynamic_precision_transitions_from_register_to_overflow_slots() {
  let mut buffer = [0_u8; 16];
  let source = b"abcdef\0";
  let mut ap = OwnedVaList::from_register_slots(
    40,
    [0, 0, 0, 0, 0, int_slot(4)],
    vec![ptr_slot(as_format_ptr(source))],
  );

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%.*s\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 4);
  assert_eq!(&buffer[..5], b"abcd\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn integer_conversions_support_core_flags_and_precision() {
  let mut buffer = [0_u8; 32];
  let mut ap = OwnedVaList::from_u64_slots(vec![int_slot(-12), u32_slot(0), u32_slot(42)]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%+05d|%.0u|%#x\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 11);
  assert_eq!(&buffer[..12], b"-0012||0x2a\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn pointer_conversion_supports_width_and_zero_padding() {
  let mut buffer = [0_u8; 64];
  let pointer = 0x2a_usize as *const c_void;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(pointer),
    ptr_slot(pointer),
    ptr_slot(pointer),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%p|%12p|%012p\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 30);
  assert_eq!(&buffer[..31], b"0x2a|        0x2a|0x000000002a\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn pointer_conversion_accepts_sign_space_and_alternate_flags() {
  let mut buffer = [0_u8; 64];
  let pointer = 0x2a_usize as *const c_void;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(pointer),
    ptr_slot(pointer),
    ptr_slot(pointer),
    ptr_slot(pointer),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%#p|%+p|% p|%+012p\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 29);
  assert_eq!(&buffer[..30], b"0x2a|+0x2a| 0x2a|+0x00000002a\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn pointer_conversion_sign_flags_respect_left_align_and_precision() {
  let mut buffer = [0_u8; 64];
  let pointer = 0x2a_usize as *const c_void;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(pointer),
    ptr_slot(pointer),
    ptr_slot(pointer),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%-+12p|%+#12.6p|%#-12p\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 38);
  assert_eq!(&buffer[..39], b"+0x2a       |   +0x00002a|0x2a        \0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn pointer_conversion_supports_precision_and_width() {
  let mut buffer = [0_u8; 64];
  let pointer = 0x2a_usize as *const c_void;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(pointer),
    ptr_slot(pointer),
    ptr_slot(pointer),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%p|%12.6p|%012.6p\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 30);
  assert_eq!(&buffer[..31], b"0x2a|    0x00002a|    0x00002a\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn pointer_conversion_null_precision_zero_keeps_single_zero_digit() {
  let mut buffer = [0_u8; 64];
  let null_pointer = core::ptr::null::<c_void>();
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(null_pointer),
    ptr_slot(null_pointer),
    ptr_slot(null_pointer),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%p|%.0p|%5.0p\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 13);
  assert_eq!(&buffer[..14], b"0x0|0x0|  0x0\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn integer_length_modifiers_match_vararg_contract() {
  let mut buffer = [0_u8; 96];
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(260),
    int_slot(65_540),
    u32_slot(u32::MAX),
    usize_slot(12_345),
    u64_slot((-1_234_567_890_123_i64).cast_unsigned()),
    u64_slot(0xfeed_face_cafe_beef),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%hhu %hu %u %zu %lld %llx\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 52);
  assert_eq!(
    &buffer[..53],
    b"4 4 4294967295 12345 -1234567890123 feedfacecafebeef\0"
  );
  assert_eq!(read_errno(), 0);
}

#[test]
fn integer_length_modifiers_support_j_and_t() {
  let mut buffer = [0_u8; 96];
  let mut ap = OwnedVaList::from_u64_slots(vec![
    u64_slot(u64::from_ne_bytes((-42_i64).to_ne_bytes())),
    u64_slot(0x1ff_u64),
    usize_slot(33),
    u64_slot(u64::from_ne_bytes((-7_i64).to_ne_bytes())),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%jd %jx %tu %td\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 13);
  assert_eq!(&buffer[..14], b"-42 1ff 33 -7\0");
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_writes_emitted_length_without_output_bytes() {
  let mut buffer = [0_u8; 32];
  let mut count: c_int = -1;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abc%nXYZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer[..7], b"abcXYZ\0");
  assert_eq!(count, 3);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_writes_emitted_length_with_zero_capacity_non_null_buffer() {
  let mut buffer = [b'Q'; 4];
  let mut count: c_int = -1;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc%n\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(count, 3);
  assert_eq!(buffer, [b'Q'; 4]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_lln_writes_emitted_length_with_zero_capacity_non_null_buffer() {
  let mut buffer = [b'Q'; 4];
  let mut count_ll: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_ll).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc%lln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(count_ll, 3);
  assert_eq!(buffer, [b'Q'; 4]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_zn_writes_emitted_length_with_zero_capacity_non_null_buffer() {
  let mut buffer = [b'Q'; 4];
  let mut count_z: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_z).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(count_z, 3);
  assert_eq!(buffer, [b'Q'; 4]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_hn_writes_emitted_length_with_zero_capacity_non_null_buffer() {
  let mut buffer = [b'Q'; 4];
  let mut count_h: i16 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_h).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc%hn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(count_h, 3);
  assert_eq!(buffer, [b'Q'; 4]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_hhn_writes_emitted_length_with_zero_capacity_non_null_buffer() {
  let mut buffer = [b'Q'; 4];
  let mut count_hh: i8 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_hh).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc%hhn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(count_hh, 3);
  assert_eq!(buffer, [b'Q'; 4]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_jn_writes_emitted_length_with_zero_capacity_non_null_buffer() {
  let mut buffer = [b'Q'; 4];
  let mut count_j: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_j).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc%jn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(count_j, 3);
  assert_eq!(buffer, [b'Q'; 4]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_tn_writes_emitted_length_with_zero_capacity_non_null_buffer() {
  let mut buffer = [b'Q'; 4];
  let mut count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_t).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(count_t, 3);
  assert_eq!(buffer, [b'Q'; 4]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_ln_writes_emitted_length_with_zero_capacity_non_null_buffer() {
  let mut buffer = [b'Q'; 4];
  let mut count_l: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_l).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(0),
      as_format_ptr(b"abc%ln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(count_l, 3);
  assert_eq!(buffer, [b'Q'; 4]);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_writes_required_length_with_null_buffer_and_zero_capacity() {
  let mut count: c_int = -1;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: `n == 0` allows null output buffer; pointers in `ap` follow SysV `va_list`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"ab%n\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(count, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_zn_writes_required_length_with_null_buffer_and_zero_capacity() {
  let mut count_z: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_z).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `n == 0` allows null output buffer; pointers in `ap` follow SysV `va_list`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"ab%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(count_z, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_lln_writes_required_length_with_null_buffer_and_zero_capacity() {
  let mut count_ll: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_ll).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `n == 0` allows null output buffer; pointers in `ap` follow SysV `va_list`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"ab%lln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(count_ll, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_ln_writes_required_length_with_null_buffer_and_zero_capacity() {
  let mut count_l: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_l).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `n == 0` allows null output buffer; pointers in `ap` follow SysV `va_list`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"ab%ln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(count_l, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_hhn_writes_required_length_with_null_buffer_and_zero_capacity() {
  let mut count_hh: i8 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_hh).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `n == 0` allows null output buffer; pointers in `ap` follow SysV `va_list`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"ab%hhn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(count_hh, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_hn_writes_required_length_with_null_buffer_and_zero_capacity() {
  let mut count_h: i16 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_h).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `n == 0` allows null output buffer; pointers in `ap` follow SysV `va_list`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"ab%hn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(count_h, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_jn_writes_required_length_with_null_buffer_and_zero_capacity() {
  let mut count_j: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_j).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `n == 0` allows null output buffer; pointers in `ap` follow SysV `va_list`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"ab%jn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(count_j, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_tn_writes_required_length_with_null_buffer_and_zero_capacity() {
  let mut count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_t).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `n == 0` allows null output buffer; pointers in `ap` follow SysV `va_list`.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"ab%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 2);
  assert_eq!(count_t, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_uses_required_length_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut count: c_int = -1;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef%n\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer, b"abc\0");
  assert_eq!(count, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_hhn_uses_required_length_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut count_hh: i8 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_hh).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef%hhn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer, b"abc\0");
  assert_eq!(count_hh, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_hn_uses_required_length_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut count_h: i16 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_h).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef%hn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer, b"abc\0");
  assert_eq!(count_h, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_jn_uses_required_length_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut count_j: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_j).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef%jn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer, b"abc\0");
  assert_eq!(count_j, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_tn_uses_required_length_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_t).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer, b"abc\0");
  assert_eq!(count_t, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_ln_uses_required_length_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut count_l: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_l).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef%ln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer, b"abc\0");
  assert_eq!(count_l, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_lln_uses_required_length_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut count_ll: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_ll).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef%lln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer, b"abc\0");
  assert_eq!(count_ll, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_zn_uses_required_length_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut count_z: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_z).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"abcdef%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer, b"abc\0");
  assert_eq!(count_z, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_jn_and_zn_include_escaped_percent_lengths_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut intmax_written: i64 = -1;
  let mut size_written: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(intmax_written).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(size_written).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"A%%BC%%D%jnE%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 7);
  assert_eq!(&buffer, b"A%B\0");
  assert_eq!(intmax_written, 6);
  assert_eq!(size_written, 7);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_ln_and_lln_include_escaped_percent_lengths_under_truncation() {
  let mut buffer = [0_u8; 4];
  let mut long_written: i64 = -1;
  let mut long_long_written: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(long_written).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(long_long_written).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"A%%BC%%D%lnE%lln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 7);
  assert_eq!(&buffer, b"A%B\0");
  assert_eq!(long_written, 6);
  assert_eq!(long_long_written, 7);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_rejects_width_directive() {
  let mut buffer = [b'Q'; 8];
  let mut count: c_int = 123;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"z%2n!\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..2], b"z\0");
  assert_eq!(count, 123);
}

#[test]
fn count_conversion_rejects_dynamic_width_directive() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 64;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(5),
    ptr_slot(core::ptr::addr_of_mut!(count).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%*nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 64);
}

#[test]
fn count_conversion_rejects_dynamic_precision_directive() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 64;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(0),
    ptr_slot(core::ptr::addr_of_mut!(count).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%.*nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 64);
}

#[test]
fn count_conversion_rejects_left_align_flag() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 55;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%-nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 55);
}

#[test]
fn count_conversion_rejects_zero_pad_flag() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 55;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%0nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 55);
}

#[test]
fn count_conversion_rejects_alternate_flag() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 55;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%#nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 55);
}

#[test]
fn count_conversion_rejects_force_sign_flag() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 55;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%+nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 55);
}

#[test]
fn count_conversion_rejects_leading_space_flag() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 55;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab% nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 55);
}

#[test]
fn count_conversion_supports_length_modifiers() {
  let mut buffer = [0_u8; 32];
  let mut count_hh: i8 = -1;
  let mut count_h: i16 = -1;
  let mut count_l: i64 = -1;
  let mut count_ll: i64 = -1;
  let mut count_z: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_hh).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(count_h).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(count_l).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(count_ll).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(count_z).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"a%hhnbc%hnDEF%lnGHIJ%llnKLMNO%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 15);
  assert_eq!(&buffer[..16], b"abcDEFGHIJKLMNO\0");
  assert_eq!(count_hh, 1);
  assert_eq!(count_h, 3);
  assert_eq!(count_l, 6);
  assert_eq!(count_ll, 10);
  assert_eq!(count_z, 15);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_multiple_writes_track_progress_per_conversion() {
  let mut buffer = [0_u8; 8];
  let mut short_count: i16 = -1;
  let mut intmax_count: i64 = -1;
  let mut ptrdiff_count: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot((&raw mut short_count).cast_const()),
    ptr_slot((&raw mut intmax_count).cast_const()),
    ptr_slot((&raw mut ptrdiff_count).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"A%hnB%jnC%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(&buffer[..4], b"ABC\0");
  assert_eq!(short_count, 1);
  assert_eq!(intmax_count, 2);
  assert_eq!(ptrdiff_count, 3);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_includes_escaped_percent_in_written_lengths() {
  let mut buffer = [0_u8; 16];
  let mut first_count_n: c_int = -1;
  let mut second_count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(first_count_n).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(second_count_t).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"A%%B%nC%%D%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 6);
  assert_eq!(&buffer[..7], b"A%BC%D\0");
  assert_eq!(first_count_n, 3);
  assert_eq!(second_count_t, 6);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_later_null_pointer_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count: c_int = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(first_count).cast_const()),
    0,
  ]);

  set_errno(0);
  // SAFETY: first pointer is valid, second pointer is intentionally null for error-path coverage.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%nC%jnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..4], b"abC\0");
  assert_eq!(first_count, 2);
}

#[test]
fn count_conversion_later_unsupported_specifier_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count: c_int = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(first_count).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `%n` pointer is valid; `%q` is intentionally unsupported for error-path coverage.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%nC%qZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..4], b"abC\0");
  assert_eq!(first_count, 2);
}

#[test]
fn count_conversion_later_null_pointer_with_tn_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count_j: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(first_count_j).cast_const()),
    0,
  ]);

  set_errno(0);
  // SAFETY: first pointer is valid, second pointer is intentionally null for error-path coverage.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%jnC%tnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..4], b"abC\0");
  assert_eq!(first_count_j, 2);
}

#[test]
fn count_conversion_later_null_pointer_with_zn_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(first_count_t).cast_const()),
    0,
  ]);

  set_errno(0);
  // SAFETY: first pointer is valid, second pointer is intentionally null for error-path coverage.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%tnC%znZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..4], b"abC\0");
  assert_eq!(first_count_t, 2);
}

#[test]
fn count_conversion_later_unsupported_specifier_with_tn_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(first_count_t).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `%tn` pointer is valid; `%q` is intentionally unsupported for error-path coverage.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%tnC%qZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..4], b"abC\0");
  assert_eq!(first_count_t, 2);
}

#[test]
fn count_conversion_later_unsupported_specifier_with_zn_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count_z: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(first_count_z).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `%zn` pointer is valid; `%q` is intentionally unsupported for error-path coverage.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%znC%qZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..4], b"abC\0");
  assert_eq!(first_count_z, 2);
}

#[test]
fn count_conversion_later_dangling_percent_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count_n: c_int = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(first_count_n).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `%n` pointer is valid; trailing `%` intentionally triggers format error after `%n`.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%n%\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(first_count_n, 2);
}

#[test]
fn count_conversion_later_dangling_percent_with_tn_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(first_count_t).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `%tn` pointer is valid; trailing `%` intentionally triggers format error after `%tn`.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%tn%\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(first_count_t, 2);
}

#[test]
fn count_conversion_later_dangling_percent_with_zn_keeps_prior_successful_write() {
  let mut buffer = [b'Q'; 16];
  let mut first_count_z: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(first_count_z).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: `%zn` pointer is valid; trailing `%` intentionally triggers format error after `%zn`.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%zn%\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(first_count_z, 2);
}

#[test]
fn count_conversion_later_dangling_percent_with_ln_and_lln_keeps_prior_successful_writes() {
  let mut buffer = [b'Q'; 16];
  let mut first_count_l: i64 = -1;
  let mut second_count_ll: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(first_count_l).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(second_count_ll).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: `%ln`/`%lln` pointers are valid; trailing `%` intentionally triggers format error after both writes.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%lnC%lln%\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..4], b"abC\0");
  assert_eq!(first_count_l, 2);
  assert_eq!(second_count_ll, 3);
}

#[test]
fn count_conversion_rejects_hhn_overflow() {
  let mut buffer = [b'Q'; 16];
  let mut count_hh: i8 = 88;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(0),
    ptr_slot(core::ptr::addr_of_mut!(count_hh).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%130d%hhn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(count_hh, 88);
  assert_eq!(buffer[buffer.len() - 1], 0);
}

#[test]
fn count_conversion_rejects_hn_overflow() {
  let mut buffer = [b'Q'; 24];
  let mut count_h: i16 = 777;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(0),
    ptr_slot(core::ptr::addr_of_mut!(count_h).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%33000d%hn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(count_h, 777);
  assert_eq!(buffer[buffer.len() - 1], 0);
}

#[test]
fn count_conversion_rejects_precision_directive() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 77;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%.0nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 77);
}

#[test]
fn count_conversion_rejects_null_pointer_argument() {
  let mut buffer = [b'Q'; 12];
  let mut ap = OwnedVaList::from_u64_slots(vec![0]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%nZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
}

#[test]
fn count_conversion_rejects_null_pointer_with_hn_length_modifier() {
  let mut buffer = [b'Q'; 12];
  let mut ap = OwnedVaList::from_u64_slots(vec![0]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%hnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
}

#[test]
fn count_conversion_rejects_null_pointer_with_hhn_length_modifier() {
  let mut buffer = [b'Q'; 12];
  let mut ap = OwnedVaList::from_u64_slots(vec![0]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%hhnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
}

#[test]
fn count_conversion_rejects_null_pointer_with_ln_length_modifier() {
  let mut buffer = [b'Q'; 12];
  let mut ap = OwnedVaList::from_u64_slots(vec![0]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%lnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
}

#[test]
fn count_conversion_rejects_null_pointer_with_lln_length_modifier() {
  let mut buffer = [b'Q'; 12];
  let mut ap = OwnedVaList::from_u64_slots(vec![0]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%llnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
}

#[test]
fn count_conversion_rejects_null_pointer_with_zn_length_modifier() {
  let mut buffer = [b'Q'; 12];
  let mut ap = OwnedVaList::from_u64_slots(vec![0]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%znZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
}

#[test]
fn count_conversion_rejects_null_pointer_with_jn_length_modifier() {
  let mut buffer = [b'Q'; 12];
  let mut ap = OwnedVaList::from_u64_slots(vec![0]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%jnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
}

#[test]
fn count_conversion_rejects_null_pointer_with_tn_length_modifier() {
  let mut buffer = [b'Q'; 12];
  let mut ap = OwnedVaList::from_u64_slots(vec![0]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%tnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
}

#[test]
fn count_conversion_success_does_not_clobber_errno() {
  let mut buffer = [0_u8; 8];
  let mut count: c_int = -1;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"X%n\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 1);
  assert_eq!(&buffer[..2], b"X\0");
  assert_eq!(count, 1);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_hhn_success_does_not_clobber_errno() {
  let mut buffer = [0_u8; 8];
  let mut count_hh: i8 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_hh).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"X%hhn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 1);
  assert_eq!(&buffer[..2], b"X\0");
  assert_eq!(count_hh, 1);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_hn_success_does_not_clobber_errno() {
  let mut buffer = [0_u8; 8];
  let mut count_h: i16 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_h).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"X%hn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 1);
  assert_eq!(&buffer[..2], b"X\0");
  assert_eq!(count_h, 1);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_n_hn_hhn_zero_prefix_success_does_not_clobber_errno() {
  let mut buffer = [b'Q'; 8];
  let mut count_n: c_int = -1;
  let mut count_hn: i16 = -1;
  let mut count_hhn: i8 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_n).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(count_hn).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(count_hhn).cast_const()),
  ]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%n%hn%hhn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 0);
  assert_eq!(buffer[0], 0);
  assert_eq!(count_n, 0);
  assert_eq!(count_hn, 0);
  assert_eq!(count_hhn, 0);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_ln_success_does_not_clobber_errno() {
  let mut buffer = [0_u8; 8];
  let mut count_l: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_l).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"X%ln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 1);
  assert_eq!(&buffer[..2], b"X\0");
  assert_eq!(count_l, 1);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_ln_zero_prefix_success_does_not_clobber_errno() {
  let mut buffer = [b'Q'; 8];
  let mut count_l: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_l).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%ln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 0);
  assert_eq!(buffer[0], 0);
  assert_eq!(count_l, 0);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_zn_success_does_not_clobber_errno() {
  let mut buffer = [0_u8; 8];
  let mut count_z: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_z).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"X%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 1);
  assert_eq!(&buffer[..2], b"X\0");
  assert_eq!(count_z, 1);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_zn_zero_prefix_success_does_not_clobber_errno() {
  let mut buffer = [b'Q'; 8];
  let mut count_z = usize::MAX;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_z).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 0);
  assert_eq!(buffer[0], 0);
  assert_eq!(count_z, 0);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_jn_success_does_not_clobber_errno() {
  let mut buffer = [0_u8; 8];
  let mut count_j: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_j).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"X%jn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 1);
  assert_eq!(&buffer[..2], b"X\0");
  assert_eq!(count_j, 1);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_jn_zero_prefix_success_does_not_clobber_errno() {
  let mut buffer = [b'Q'; 8];
  let mut count_j: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_j).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%jn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 0);
  assert_eq!(buffer[0], 0);
  assert_eq!(count_j, 0);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_tn_success_does_not_clobber_errno() {
  let mut buffer = [0_u8; 8];
  let mut count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_t).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"X%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 1);
  assert_eq!(&buffer[..2], b"X\0");
  assert_eq!(count_t, 1);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_tn_zero_prefix_success_does_not_clobber_errno() {
  let mut buffer = [b'Q'; 8];
  let mut count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_t).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 0);
  assert_eq!(buffer[0], 0);
  assert_eq!(count_t, 0);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_lln_success_does_not_clobber_errno() {
  let mut buffer = [0_u8; 8];
  let mut count_ll: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_ll).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"X%lln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 1);
  assert_eq!(&buffer[..2], b"X\0");
  assert_eq!(count_ll, 1);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_lln_zero_prefix_success_does_not_clobber_errno() {
  let mut buffer = [b'Q'; 8];
  let mut count_ll: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_ll).cast_const(),
  )]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%lln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 0);
  assert_eq!(buffer[0], 0);
  assert_eq!(count_ll, 0);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_jn_tn_zn_zero_prefix_success_does_not_clobber_errno() {
  let mut buffer = [b'Q'; 8];
  let mut count_j: i64 = -1;
  let mut count_t: isize = -1;
  let mut count_z: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    ptr_slot(core::ptr::addr_of_mut!(count_j).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(count_t).cast_const()),
    ptr_slot(core::ptr::addr_of_mut!(count_z).cast_const()),
  ]);

  set_errno(91);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"%jn%tn%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 0);
  assert_eq!(buffer[0], 0);
  assert_eq!(count_j, 0);
  assert_eq!(count_t, 0);
  assert_eq!(count_z, 0);
  assert_eq!(read_errno(), 91);
}

#[test]
fn count_conversion_rejects_unrepresentable_emitted_length() {
  let mut count: c_int = -5;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%n\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(count, -5);
}

#[test]
fn count_conversion_lln_rejects_unrepresentable_emitted_length() {
  let mut count_ll: i64 = -7;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_ll).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%lln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(count_ll, -7);
}

#[test]
fn count_conversion_jn_rejects_unrepresentable_emitted_length() {
  let mut count_j: i64 = -7;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_j).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%jn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(count_j, -7);
}

#[test]
fn count_conversion_tn_rejects_unrepresentable_emitted_length() {
  let mut count_t: isize = -7;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_t).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(count_t, -7);
}

#[test]
fn count_conversion_ln_rejects_unrepresentable_emitted_length() {
  let mut count_l: i64 = -7;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_l).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%ln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(count_l, -7);
}

#[test]
fn count_conversion_zn_rejects_unrepresentable_emitted_length() {
  let mut count_z: isize = -7;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_z).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(count_z, -7);
}

#[test]
fn count_conversion_accepts_c_int_max_boundary() {
  let mut count: c_int = -5;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX - 1),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%n\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, c_int::MAX);
  assert_eq!(count, c_int::MAX);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_ln_accepts_c_int_max_boundary() {
  let mut count_l: i64 = -5;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX - 1),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_l).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%ln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, c_int::MAX);
  assert_eq!(count_l, i64::from(c_int::MAX));
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_lln_accepts_c_int_max_boundary() {
  let mut count_ll: i64 = -5;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX - 1),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_ll).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%lln\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, c_int::MAX);
  assert_eq!(count_ll, i64::from(c_int::MAX));
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_zn_accepts_c_int_max_boundary() {
  let mut count_z: isize = -5;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX - 1),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_z).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%zn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, c_int::MAX);
  assert_eq!(count_z, c_int::MAX as isize);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_jn_accepts_c_int_max_boundary() {
  let mut count_j: i64 = -5;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX - 1),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_j).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%jn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, c_int::MAX);
  assert_eq!(count_j, i64::from(c_int::MAX));
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_tn_accepts_c_int_max_boundary() {
  let mut count_t: isize = -5;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(c_int::MAX - 1),
    ptr_slot(as_format_ptr(b"x\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_t).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"A%*s%tn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, c_int::MAX);
  assert_eq!(count_t, c_int::MAX as isize);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_hhn_accepts_i8_max_boundary() {
  let mut count_hh: i8 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(i8::MAX.into()),
    ptr_slot(as_format_ptr(b"\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_hh).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"%*s%hhn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, i8::MAX.into());
  assert_eq!(count_hh, i8::MAX);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_hn_accepts_i16_max_boundary() {
  let mut count_h: i16 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![
    int_slot(i16::MAX.into()),
    ptr_slot(as_format_ptr(b"\0")),
    ptr_slot(core::ptr::addr_of_mut!(count_h).cast_const()),
  ]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      core::ptr::null_mut(),
      as_size_t(0),
      as_format_ptr(b"%*s%hn\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, i16::MAX.into());
  assert_eq!(count_h, i16::MAX);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_supports_j_length_modifier() {
  let mut buffer = [0_u8; 12];
  let mut count_j: i64 = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_j).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%jnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(&buffer[..4], b"abZ\0");
  assert_eq!(count_j, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_supports_t_length_modifier() {
  let mut buffer = [0_u8; 12];
  let mut count_t: isize = -1;
  let mut ap = OwnedVaList::from_u64_slots(vec![ptr_slot(
    core::ptr::addr_of_mut!(count_t).cast_const(),
  )]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%tnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, 3);
  assert_eq!(&buffer[..4], b"abZ\0");
  assert_eq!(count_t, 2);
  assert_eq!(read_errno(), 0);
}

#[test]
fn count_conversion_rejects_capital_l_length_modifier() {
  let mut buffer = [b'Q'; 12];
  let mut count: c_int = 41;
  let mut ap =
    OwnedVaList::from_u64_slots(vec![ptr_slot(core::ptr::addr_of_mut!(count).cast_const())]);

  set_errno(0);
  // SAFETY: pointers are valid and `ap` points to x86_64 SysV `va_list` layout.
  let result = unsafe {
    vsnprintf(
      buffer.as_mut_ptr().cast(),
      as_size_t(buffer.len()),
      as_format_ptr(b"ab%LnZ\0"),
      ap.as_mut_ptr(),
    )
  };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(&buffer[..3], b"ab\0");
  assert_eq!(count, 41);
}
