#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_long;
use rlibc::syscall::{
  decode_raw, syscall0, syscall1, syscall2, syscall3, syscall4, syscall5, syscall6,
};
use std::io::Read;
use std::os::fd::{AsRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const SYS_CLOSE: c_long = 3;
const SYS_DUP2: c_long = 33;
const SYS_SENDFILE: c_long = 40;
const SYS_MMAP: c_long = 9;
const SYS_MUNMAP: c_long = 11;
const SYS_PRCTL: c_long = 157;
const SYS_WRITE: c_long = 1;
const SYS_GETPID: c_long = 39;
const EBADF: i32 = 9;
const EINVAL: i32 = 22;
const ENOSYS: i32 = 38;
const PROT_READ: c_long = 0x1;
const PROT_WRITE: c_long = 0x2;
const MAP_PRIVATE: c_long = 0x02;
const MAP_ANONYMOUS: c_long = 0x20;

fn c_long_from_usize(value: usize) -> c_long {
  c_long::try_from(value).expect("usize does not fit into c_long on this target")
}

fn c_long_from_ptr<T>(pointer: *const T) -> c_long {
  c_long_from_usize(pointer.addr())
}

fn decode(raw: c_long) -> Result<usize, i32> {
  let raw_isize = isize::try_from(raw).expect("c_long does not fit into isize on this target");

  decode_raw(raw_isize)
}

fn usize_from_c_long_bits(value: c_long) -> usize {
  usize::try_from(value.cast_unsigned())
    .unwrap_or_else(|_| unreachable!("c_long bit pattern does not fit into usize on this target"))
}

#[test]
fn decode_c_long_min_is_success_value() {
  let raw = c_long::MIN;

  assert_eq!(decode(raw), Ok(usize_from_c_long_bits(raw)));
}

#[test]
fn decode_c_long_max_becomes_ok_usize_value() {
  let expected = usize::try_from(c_long::MAX).expect("c_long::MAX should fit into usize");

  assert_eq!(decode(c_long::MAX), Ok(expected));
}

#[test]
fn decode_c_long_linux_errno_upper_bound_becomes_err_4095() {
  assert_eq!(decode(-4095), Err(4095));
}

#[test]
fn decode_c_long_linux_errno_inner_value_becomes_err_4094() {
  assert_eq!(decode(-4094), Err(4094));
}

#[test]
fn decode_c_long_linux_errno_mid_value_becomes_err_1234() {
  assert_eq!(decode(-1234), Err(1234));
}

#[test]
fn decode_c_long_just_below_linux_errno_upper_bound_is_success_value() {
  let raw = -4096 as c_long;

  assert_eq!(decode(raw), Ok(usize_from_c_long_bits(raw)));
}

#[test]
fn decode_c_long_two_below_linux_errno_upper_bound_is_success_value() {
  let raw = -4097 as c_long;

  assert_eq!(decode(raw), Ok(usize_from_c_long_bits(raw)));
}

#[test]
fn decode_c_long_far_below_linux_errno_range_is_success_value() {
  let raw = -5000 as c_long;

  assert_eq!(decode(raw), Ok(usize_from_c_long_bits(raw)));
}

#[test]
fn decode_c_long_negative_one_becomes_err_one() {
  assert_eq!(decode(-1), Err(1));
}

#[test]
fn decode_c_long_negative_two_becomes_err_two() {
  assert_eq!(decode(-2), Err(2));
}

#[test]
fn decode_c_long_negative_three_becomes_err_three() {
  assert_eq!(decode(-3), Err(3));
}

#[test]
fn decode_c_long_negative_four_becomes_err_four() {
  assert_eq!(decode(-4), Err(4));
}

#[test]
fn decode_c_long_negative_five_becomes_err_five() {
  assert_eq!(decode(-5), Err(5));
}

#[test]
fn decode_c_long_negative_six_becomes_err_six() {
  assert_eq!(decode(-6), Err(6));
}

#[test]
fn decode_c_long_negative_seven_becomes_err_seven() {
  assert_eq!(decode(-7), Err(7));
}

#[test]
fn decode_c_long_i32_min_is_success_value() {
  let raw = c_long::from(i32::MIN);

  assert_eq!(decode(raw), Ok(usize_from_c_long_bits(raw)));
}

#[test]
fn decode_c_long_i32_min_plus_one_is_success_value() {
  let raw = c_long::from(i32::MIN) + 1;

  assert_eq!(decode(raw), Ok(usize_from_c_long_bits(raw)));
}

#[test]
fn decode_c_long_negative_twenty_two_becomes_err_twenty_two() {
  assert_eq!(decode(-22), Err(22));
}

#[test]
fn decode_c_long_i32_max_is_success_value() {
  let raw_errno = c_long::from(i32::MAX);

  assert_eq!(decode(-raw_errno), Ok(usize_from_c_long_bits(-raw_errno)));
}

#[test]
fn decode_c_long_i32_max_minus_one_is_success_value() {
  let raw_errno = c_long::from(i32::MAX - 1);

  assert_eq!(decode(-raw_errno), Ok(usize_from_c_long_bits(-raw_errno)));
}

#[test]
fn decode_c_long_negative_value_just_above_i32_max_is_success_value() {
  let raw_errno = c_long::from(i32::MAX) + 1;

  assert_eq!(decode(-raw_errno), Ok(usize_from_c_long_bits(-raw_errno)));
}

#[test]
fn decode_c_long_zero_becomes_ok_zero() {
  assert_eq!(decode(0), Ok(0));
}

#[test]
fn decode_c_long_positive_value_becomes_ok_value() {
  assert_eq!(decode(12_345), Ok(12_345));
}

#[test]
fn syscall0_getpid_matches_std_process_id() {
  // SAFETY: `SYS_GETPID` has no pointer arguments and is side-effect free.
  let raw_pid = unsafe { syscall0(SYS_GETPID) };
  let decoded_pid = decode(raw_pid).expect("getpid syscall should succeed");
  let expected_pid =
    usize::try_from(std::process::id()).expect("u32 process id does not fit in usize");

  assert_eq!(decoded_pid, expected_pid);
}

#[test]
fn syscall0_invalid_number_returns_enosys() {
  // SAFETY: invalid syscall number does not require any pointer arguments.
  let raw = unsafe { syscall0(c_long::MAX) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall0_negative_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not require pointer arguments.
  let raw = unsafe { syscall0(-1) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall0_large_negative_number_returns_enosys() {
  // SAFETY: invalid large negative syscall number does not require pointer arguments.
  let raw = unsafe { syscall0(-4096) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall0_errno_window_upper_bound_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not require pointer arguments.
  let raw = unsafe { syscall0(-4095) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall0_min_number_currently_returns_zero() {
  // SAFETY: this call uses no pointers and documents current ABI behavior.
  let raw = unsafe { syscall0(c_long::MIN) };

  assert_eq!(decode(raw), Ok(0));
}

#[test]
fn syscall1_invalid_number_returns_enosys() {
  // SAFETY: invalid syscall number does not dereference pointers.
  let raw = unsafe { syscall1(c_long::MAX, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall1_negative_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not dereference pointers.
  let raw = unsafe { syscall1(-1, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall1_large_negative_number_returns_enosys() {
  // SAFETY: invalid large negative syscall number does not dereference pointers.
  let raw = unsafe { syscall1(-4096, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall1_errno_window_upper_bound_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not dereference pointers.
  let raw = unsafe { syscall1(-4095, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall2_invalid_number_returns_enosys() {
  // SAFETY: invalid syscall number does not dereference pointers.
  let raw = unsafe { syscall2(c_long::MAX, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall2_negative_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not dereference pointers.
  let raw = unsafe { syscall2(-1, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall2_large_negative_number_returns_enosys() {
  // SAFETY: invalid large negative syscall number does not dereference pointers.
  let raw = unsafe { syscall2(-4096, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall2_errno_window_upper_bound_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not dereference pointers.
  let raw = unsafe { syscall2(-4095, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall3_invalid_number_returns_enosys() {
  // SAFETY: invalid syscall number does not dereference pointers.
  let raw = unsafe { syscall3(c_long::MAX, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall3_negative_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not dereference pointers.
  let raw = unsafe { syscall3(-1, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall3_large_negative_number_returns_enosys() {
  // SAFETY: invalid large negative syscall number does not dereference pointers.
  let raw = unsafe { syscall3(-4096, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall3_errno_window_upper_bound_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not dereference pointers.
  let raw = unsafe { syscall3(-4095, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall4_invalid_number_returns_enosys() {
  // SAFETY: invalid syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall4(c_long::MAX, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall4_negative_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall4(-1, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall4_large_negative_number_returns_enosys() {
  // SAFETY: invalid large negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall4(-4096, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall4_errno_window_upper_bound_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall4(-4095, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall5_invalid_number_returns_enosys() {
  // SAFETY: invalid syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall5(c_long::MAX, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall5_negative_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall5(-1, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall5_large_negative_number_returns_enosys() {
  // SAFETY: invalid large negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall5(-4096, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall5_errno_window_upper_bound_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall5(-4095, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall6_invalid_number_returns_enosys() {
  // SAFETY: invalid syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall6(c_long::MAX, 0, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall6_negative_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall6(-1, 0, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall6_large_negative_number_returns_enosys() {
  // SAFETY: invalid large negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall6(-4096, 0, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall6_far_negative_number_returns_enosys() {
  // SAFETY: invalid far negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall6(-5000, 0, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall6_errno_window_upper_bound_number_returns_enosys() {
  // SAFETY: invalid negative syscall number does not require valid pointer arguments.
  let raw = unsafe { syscall6(-4095, 0, 0, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(ENOSYS));
}

#[test]
fn syscall1_close_invalid_fd_returns_ebadf() {
  // SAFETY: closing an invalid fd is well-defined and does not require pointers.
  let raw = unsafe { syscall1(SYS_CLOSE, -1) };

  assert_eq!(decode(raw), Err(EBADF));
}

#[test]
fn syscall1_close_valid_fd_returns_ok_zero() {
  let (stream, _) = UnixStream::pair().expect("failed to create unix stream pair");
  let file_descriptor = stream.into_raw_fd();

  // SAFETY: `file_descriptor` is valid and owned by this test at call time.
  let raw = unsafe { syscall1(SYS_CLOSE, c_long::from(file_descriptor)) };

  assert_eq!(decode(raw), Ok(0));
}

#[test]
fn syscall2_dup2_invalid_oldfd_returns_ebadf() {
  // SAFETY: `dup2` with invalid oldfd does not dereference pointers.
  let raw = unsafe { syscall2(SYS_DUP2, -1, 0) };

  assert_eq!(decode(raw), Err(EBADF));
}

#[test]
fn syscall2_dup2_same_fd_returns_same_fd() {
  let (left, _) = UnixStream::pair().expect("failed to create unix stream pair");
  let file_descriptor = c_long::from(left.as_raw_fd());

  // SAFETY: `file_descriptor` is valid and `dup2(fd, fd)` is a defined no-op.
  let raw = unsafe { syscall2(SYS_DUP2, file_descriptor, file_descriptor) };
  let duplicated = decode(raw).expect("dup2(fd, fd) should succeed");
  let expected = usize::try_from(file_descriptor).expect("fd should fit in usize");

  assert_eq!(duplicated, expected);
}

#[test]
fn syscall3_write_sends_bytes_to_socketpair_peer() {
  let payload = b"rlibc-syscall";
  let (mut reader, writer) = UnixStream::pair().expect("failed to create unix stream pair");

  reader
    .set_read_timeout(Some(Duration::from_secs(2)))
    .expect("failed to set read timeout");

  let writer_fd = c_long::from(writer.as_raw_fd());
  let payload_pointer = c_long_from_ptr(payload.as_ptr());
  let payload_length = c_long_from_usize(payload.len());

  // SAFETY: `payload` is valid for reads of `payload.len()` bytes during this call.
  let raw_written = unsafe { syscall3(SYS_WRITE, writer_fd, payload_pointer, payload_length) };
  let written = decode(raw_written).expect("write syscall should succeed");

  assert_eq!(written, payload.len());

  let mut received = [0_u8; 32];
  let bytes_read = reader
    .read(&mut received)
    .expect("failed to read from peer socket");

  assert_eq!(&received[..bytes_read], payload);
}

#[test]
fn syscall4_sendfile_invalid_fds_returns_ebadf() {
  // SAFETY: invalid file descriptor arguments do not involve pointer dereferences.
  let raw = unsafe { syscall4(SYS_SENDFILE, -1, -1, 0, 1) };

  assert_eq!(decode(raw), Err(EBADF));
}

#[test]
fn syscall5_prctl_invalid_option_returns_einval() {
  // SAFETY: this `prctl` call uses only scalar arguments and no pointers.
  let raw = unsafe { syscall5(SYS_PRCTL, -1, 0, 0, 0, 0) };

  assert_eq!(decode(raw), Err(EINVAL));
}

#[test]
fn syscall6_mmap_roundtrip_with_munmap_succeeds() {
  let mapping_length = c_long_from_usize(4096);

  // SAFETY: anonymous mapping with `fd = -1` and `offset = 0` follows Linux mmap contract.
  let raw_mapping_address = unsafe {
    syscall6(
      SYS_MMAP,
      0,
      mapping_length,
      PROT_READ | PROT_WRITE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };
  let mapped_address = decode(raw_mapping_address).expect("mmap syscall should succeed");

  assert_ne!(mapped_address, 0);

  // SAFETY: `mapped_address` and `mapping_length` come from successful `mmap`.
  let raw_unmap = unsafe {
    syscall2(
      SYS_MUNMAP,
      c_long_from_usize(mapped_address),
      mapping_length,
    )
  };

  assert_eq!(decode(raw_unmap), Ok(0));
}
