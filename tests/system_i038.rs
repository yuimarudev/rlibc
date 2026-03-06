#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ffi::c_char;
use core::mem;
use rlibc::abi::errno::{EFAULT, ENAMETOOLONG};
use rlibc::abi::types::{c_int, c_long, size_t};
use rlibc::errno::__errno_location;
use rlibc::system::{SysInfo, UtsName, gethostname, getpagesize, sysinfo, uname};

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns valid TLS storage for this thread.
  unsafe { __errno_location().read() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns valid writable TLS storage.
  unsafe {
    __errno_location().write(value);
  }
}

fn c_field_len(field: &[c_char]) -> usize {
  field.iter().position(|&ch| ch == 0).unwrap_or(field.len())
}

fn c_field_bytes(field: &[c_char]) -> Vec<u8> {
  let len = c_field_len(field);

  field[..len].iter().map(|&ch| ch.cast_unsigned()).collect()
}

fn nodename_from_uname() -> Vec<u8> {
  let mut info = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let result = unsafe { uname(&raw mut info) };

  assert_eq!(result, 0, "uname must succeed to derive nodename");

  c_field_bytes(&info.nodename)
}

#[test]
fn utsname_layout_matches_linux_x86_64_abi() {
  assert_eq!(mem::size_of::<UtsName>(), 65 * 6);
  assert_eq!(mem::align_of::<UtsName>(), mem::align_of::<c_char>());
  assert_eq!(mem::offset_of!(UtsName, sysname), 0);
  assert_eq!(mem::offset_of!(UtsName, nodename), 65);
  assert_eq!(mem::offset_of!(UtsName, release), 130);
  assert_eq!(mem::offset_of!(UtsName, version), 195);
  assert_eq!(mem::offset_of!(UtsName, machine), 260);
  assert_eq!(mem::offset_of!(UtsName, domainname), 325);
}

#[test]
fn sysinfo_layout_matches_linux_x86_64_abi() {
  assert_eq!(mem::align_of::<SysInfo>(), mem::align_of::<c_long>());
  assert_eq!(mem::size_of::<SysInfo>(), 112);
  assert_eq!(mem::offset_of!(SysInfo, uptime), 0);
  assert_eq!(mem::offset_of!(SysInfo, loads), 8);
  assert_eq!(mem::offset_of!(SysInfo, totalram), 32);
  assert_eq!(mem::offset_of!(SysInfo, freeram), 40);
  assert_eq!(mem::offset_of!(SysInfo, sharedram), 48);
  assert_eq!(mem::offset_of!(SysInfo, bufferram), 56);
  assert_eq!(mem::offset_of!(SysInfo, totalswap), 64);
  assert_eq!(mem::offset_of!(SysInfo, freeswap), 72);
  assert_eq!(mem::offset_of!(SysInfo, procs), 80);
  assert_eq!(mem::offset_of!(SysInfo, pad), 82);
  assert_eq!(mem::offset_of!(SysInfo, totalhigh), 88);
  assert_eq!(mem::offset_of!(SysInfo, freehigh), 96);
  assert_eq!(mem::offset_of!(SysInfo, mem_unit), 104);
}

#[test]
fn uname_populates_nul_terminated_core_fields() {
  let mut info = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };

  write_errno(0);

  let result = unsafe { uname(&raw mut info) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 0);
  assert!(c_field_len(&info.sysname) < info.sysname.len());
  assert!(c_field_len(&info.nodename) < info.nodename.len());
  assert!(c_field_len(&info.release) < info.release.len());
  assert!(c_field_len(&info.version) < info.version.len());
  assert!(c_field_len(&info.machine) < info.machine.len());
  assert!(c_field_len(&info.domainname) < info.domainname.len());
  assert!(!c_field_bytes(&info.sysname).is_empty());
  assert!(!c_field_bytes(&info.nodename).is_empty());
  assert!(!c_field_bytes(&info.machine).is_empty());
}

#[test]
fn uname_success_preserves_errno_sentinel() {
  let mut info = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };

  write_errno(444);

  let result = unsafe { uname(&raw mut info) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 444);
}

#[test]
fn uname_repeated_success_keeps_errno_sentinel() {
  let mut first = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let mut second = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };

  write_errno(512);

  let first_result = unsafe { uname(&raw mut first) };
  let first_errno = read_errno();
  let second_result = unsafe { uname(&raw mut second) };
  let second_errno = read_errno();

  assert_eq!(first_result, 0);
  assert_eq!(second_result, 0);
  assert_eq!(first_errno, 512);
  assert_eq!(second_errno, 512);
  assert!(!c_field_bytes(&first.sysname).is_empty());
  assert!(!c_field_bytes(&second.sysname).is_empty());
}

#[test]
fn uname_invalid_pointer_sets_efault() {
  write_errno(0);

  let result = unsafe { uname(std::ptr::dangling_mut::<UtsName>()) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn uname_failure_overwrites_previous_errno_with_efault() {
  write_errno(ENAMETOOLONG);

  let result = unsafe { uname(core::ptr::null_mut()) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn uname_success_does_not_clear_errno_after_prior_failure() {
  write_errno(0);

  let failed = unsafe { uname(std::ptr::dangling_mut::<UtsName>()) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let mut info = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let succeeded = unsafe { uname(&raw mut info) };

  assert_eq!(succeeded, 0);
  assert_eq!(
    read_errno(),
    EFAULT,
    "successful uname must leave errno unchanged after earlier failure",
  );
}

#[test]
fn uname_success_preserves_enametoolong_from_gethostname_failure() {
  let nodename = nodename_from_uname();
  let mut short_buffer = vec![0x6a as c_char; nodename.len()];

  write_errno(0);

  let failed = unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let mut info = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let succeeded = unsafe { uname(&raw mut info) };

  assert_eq!(succeeded, 0);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn uname_success_preserves_enametoolong_from_gethostname_zero_length_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let mut info = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let succeeded = unsafe { uname(&raw mut info) };

  assert_eq!(succeeded, 0);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn uname_repeated_success_preserves_enametoolong_from_gethostname_zero_length_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let mut first = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let mut second = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let first_result = unsafe { uname(&raw mut first) };
  let first_errno = read_errno();
  let second_result = unsafe { uname(&raw mut second) };
  let second_errno = read_errno();

  assert_eq!(first_result, 0);
  assert_eq!(second_result, 0);
  assert_eq!(first_errno, ENAMETOOLONG);
  assert_eq!(second_errno, ENAMETOOLONG);
}

#[test]
fn uname_success_preserves_efault_from_gethostname_null_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let mut info = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let succeeded = unsafe { uname(&raw mut info) };

  assert_eq!(succeeded, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn uname_success_preserves_efault_from_sysinfo_failure() {
  write_errno(0);

  let failed = unsafe { sysinfo(core::ptr::null_mut()) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let mut info = UtsName {
    sysname: [0; 65],
    nodename: [0; 65],
    release: [0; 65],
    version: [0; 65],
    machine: [0; 65],
    domainname: [0; 65],
  };
  let succeeded = unsafe { uname(&raw mut info) };

  assert_eq!(succeeded, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gethostname_matches_uname_nodename_with_large_buffer() {
  let expected = nodename_from_uname();
  let mut buffer = [0x7f as c_char; 256];

  write_errno(777);

  let result = unsafe { gethostname(buffer.as_mut_ptr(), buffer.len() as size_t) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 777);
  assert_eq!(c_field_bytes(&buffer), expected);
}

#[test]
fn gethostname_repeated_success_keeps_errno_sentinel() {
  let expected = nodename_from_uname();
  let mut first = [0x24 as c_char; 256];
  let mut second = [0x57 as c_char; 256];

  write_errno(271);

  let first_result = unsafe { gethostname(first.as_mut_ptr(), first.len() as size_t) };
  let first_errno = read_errno();
  let second_result = unsafe { gethostname(second.as_mut_ptr(), second.len() as size_t) };
  let second_errno = read_errno();

  assert_eq!(first_result, 0);
  assert_eq!(second_result, 0);
  assert_eq!(first_errno, 271);
  assert_eq!(second_errno, 271);
  assert_eq!(c_field_bytes(&first), expected);
  assert_eq!(c_field_bytes(&second), expected);
}

#[test]
fn gethostname_success_only_writes_nodename_and_nul() {
  let expected = nodename_from_uname();
  let required = expected.len() + 1;
  let mut buffer = vec![0x33 as c_char; required + 8];

  write_errno(222);

  let result = unsafe { gethostname(buffer.as_mut_ptr(), buffer.len() as size_t) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 222);
  assert_eq!(c_field_bytes(&buffer), expected);
  assert_eq!(buffer[expected.len()], 0);
  assert!(
    buffer[required..]
      .iter()
      .all(|&byte| byte == 0x33 as c_char)
  );
}

#[test]
fn gethostname_zero_length_sets_enametoolong_without_dereference() {
  write_errno(0);

  let result = unsafe { gethostname(core::ptr::null_mut(), 0) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn gethostname_short_buffer_returns_enametoolong() {
  let nodename = nodename_from_uname();

  assert!(!nodename.is_empty(), "kernel nodename must not be empty");

  let mut buffer = [0 as c_char; 1];

  write_errno(0);

  let result = unsafe { gethostname(buffer.as_mut_ptr(), buffer.len() as size_t) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn gethostname_short_buffer_copies_prefix_before_enametoolong() {
  let nodename = nodename_from_uname();

  assert!(!nodename.is_empty(), "kernel nodename must not be empty");

  let mut buffer = [0x5a as c_char; 1];

  write_errno(0);

  let result = unsafe { gethostname(buffer.as_mut_ptr(), buffer.len() as size_t) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);
  assert_eq!(buffer[0], nodename[0].cast_signed());
}

#[test]
fn gethostname_payload_length_buffer_copies_full_name_without_nul() {
  let nodename = nodename_from_uname();

  assert!(!nodename.is_empty(), "kernel nodename must not be empty");

  let mut buffer = vec![0x41 as c_char; nodename.len()];

  write_errno(0);

  let result = unsafe { gethostname(buffer.as_mut_ptr(), buffer.len() as size_t) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);
  assert_eq!(
    buffer
      .iter()
      .map(|&byte| byte.cast_unsigned())
      .collect::<Vec<_>>(),
    nodename,
  );
}

#[test]
fn gethostname_null_pointer_sets_efault() {
  write_errno(0);

  let result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gethostname_null_pointer_overwrites_previous_errno_with_efault() {
  write_errno(ENAMETOOLONG);

  let result = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gethostname_zero_length_buffer_returns_enametoolong() {
  let mut byte = 0x42 as c_char;

  write_errno(0);

  let result = unsafe { gethostname(core::ptr::addr_of_mut!(byte), 0 as size_t) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);
  assert_eq!(byte, 0x42 as c_char);
}

#[test]
fn gethostname_zero_length_overwrites_previous_errno_with_enametoolong() {
  write_errno(EFAULT);

  let result = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn gethostname_failure_overwrites_errno_with_enametoolong() {
  let nodename = nodename_from_uname();
  let mut buffer = vec![0x66 as c_char; nodename.len()];

  write_errno(EFAULT);

  let result = unsafe { gethostname(buffer.as_mut_ptr(), buffer.len() as size_t) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);
  assert_eq!(
    buffer
      .iter()
      .map(|&byte| byte.cast_unsigned())
      .collect::<Vec<_>>(),
    nodename,
  );
}

#[test]
fn gethostname_success_does_not_clear_errno_after_prior_failure() {
  let nodename = nodename_from_uname();
  let mut short_buffer = vec![0x77 as c_char; nodename.len()];

  write_errno(0);

  let failed = unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let mut large_buffer = [0x22 as c_char; 256];
  let succeeded = unsafe { gethostname(large_buffer.as_mut_ptr(), large_buffer.len() as size_t) };

  assert_eq!(succeeded, 0);
  assert_eq!(
    read_errno(),
    ENAMETOOLONG,
    "successful gethostname must leave errno unchanged after earlier failure",
  );
}

#[test]
fn gethostname_success_preserves_enametoolong_from_gethostname_zero_length_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let mut large_buffer = [0x52 as c_char; 256];
  let succeeded = unsafe { gethostname(large_buffer.as_mut_ptr(), large_buffer.len() as size_t) };

  assert_eq!(succeeded, 0);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn gethostname_success_preserves_efault_from_sysinfo_failure() {
  write_errno(0);

  let failed = unsafe { sysinfo(core::ptr::null_mut()) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let mut large_buffer = [0x31 as c_char; 256];
  let succeeded = unsafe { gethostname(large_buffer.as_mut_ptr(), large_buffer.len() as size_t) };

  assert_eq!(succeeded, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gethostname_success_preserves_efault_from_uname_failure() {
  write_errno(0);

  let failed = unsafe { uname(core::ptr::null_mut()) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let mut large_buffer = [0x21 as c_char; 256];
  let succeeded = unsafe { gethostname(large_buffer.as_mut_ptr(), large_buffer.len() as size_t) };

  assert_eq!(succeeded, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gethostname_success_preserves_efault_from_gethostname_null_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let mut large_buffer = [0x11 as c_char; 256];
  let succeeded = unsafe { gethostname(large_buffer.as_mut_ptr(), large_buffer.len() as size_t) };

  assert_eq!(succeeded, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn gethostname_exact_length_buffer_is_nul_terminated() {
  let expected = nodename_from_uname();
  let required = expected.len() + 1;
  let mut buffer = vec![0x55 as c_char; required];

  write_errno(888);

  let result = unsafe { gethostname(buffer.as_mut_ptr(), required as size_t) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 888);
  assert_eq!(c_field_bytes(&buffer), expected);
  assert_eq!(buffer[expected.len()], 0);
}

#[test]
fn getpagesize_returns_positive_power_of_two() {
  let page_size = getpagesize();

  assert!(page_size > 0);
  assert_eq!(page_size & (page_size - 1), 0);
}

#[test]
fn getpagesize_matches_i038_contract_constant() {
  assert_eq!(getpagesize(), 4096);
}

#[test]
fn getpagesize_repeated_success_keeps_errno_sentinel() {
  write_errno(313);

  let first = getpagesize();
  let first_errno = read_errno();
  let second = getpagesize();
  let second_errno = read_errno();

  assert_eq!(first, 4096);
  assert_eq!(second, 4096);
  assert_eq!(first_errno, 313);
  assert_eq!(second_errno, 313);
}

#[test]
fn getpagesize_success_preserves_errno_after_prior_failure() {
  write_errno(0);

  let failed = unsafe { uname(core::ptr::null_mut()) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let page_size = getpagesize();

  assert_eq!(page_size, 4096);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn getpagesize_preserves_enametoolong_from_gethostname_failure() {
  let nodename = nodename_from_uname();
  let mut short_buffer = vec![0x3a as c_char; nodename.len()];

  write_errno(0);

  let failed = unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let page_size = getpagesize();

  assert_eq!(page_size, 4096);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn getpagesize_preserves_enametoolong_from_gethostname_zero_length_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let page_size = getpagesize();

  assert_eq!(page_size, 4096);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn getpagesize_repeated_success_preserves_enametoolong_from_gethostname_zero_length_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  let first_page_size = getpagesize();
  let first_errno = read_errno();
  let second_page_size = getpagesize();
  let second_errno = read_errno();

  assert_eq!(first_page_size, 4096);
  assert_eq!(second_page_size, 4096);
  assert_eq!(first_errno, ENAMETOOLONG);
  assert_eq!(second_errno, ENAMETOOLONG);
}

#[test]
fn getpagesize_preserves_efault_from_gethostname_null_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let page_size = getpagesize();

  assert_eq!(page_size, 4096);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn getpagesize_preserves_efault_from_sysinfo_failure() {
  write_errno(0);

  let failed = unsafe { sysinfo(core::ptr::null_mut()) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  let page_size = getpagesize();

  assert_eq!(page_size, 4096);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn sysinfo_success_preserves_enametoolong_from_gethostname_failure() {
  let nodename = nodename_from_uname();
  let mut short_buffer = vec![0x4b as c_char; nodename.len()];

  write_errno(0);

  let failed = unsafe { gethostname(short_buffer.as_mut_ptr(), short_buffer.len() as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut info: SysInfo = unsafe { mem::zeroed() };
  let result = unsafe { sysinfo(&raw mut info) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn sysinfo_success_preserves_enametoolong_from_gethostname_zero_length_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut info: SysInfo = unsafe { mem::zeroed() };
  let result = unsafe { sysinfo(&raw mut info) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), ENAMETOOLONG);
}

#[test]
fn sysinfo_repeated_success_preserves_enametoolong_from_gethostname_zero_length_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 0 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), ENAMETOOLONG);

  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut first_info: SysInfo = unsafe { mem::zeroed() };
  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut second_info: SysInfo = unsafe { mem::zeroed() };
  let first = unsafe { sysinfo(&raw mut first_info) };
  let first_errno = read_errno();
  let second = unsafe { sysinfo(&raw mut second_info) };
  let second_errno = read_errno();

  assert_eq!(first, 0);
  assert_eq!(second, 0);
  assert_eq!(first_errno, ENAMETOOLONG);
  assert_eq!(second_errno, ENAMETOOLONG);
  assert!(first_info.mem_unit > 0);
  assert!(second_info.mem_unit > 0);
}

#[test]
fn sysinfo_success_preserves_efault_from_uname_failure() {
  write_errno(0);

  let failed = unsafe { uname(core::ptr::null_mut()) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut info: SysInfo = unsafe { mem::zeroed() };
  let result = unsafe { sysinfo(&raw mut info) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn sysinfo_success_preserves_efault_from_gethostname_null_failure() {
  write_errno(0);

  let failed = unsafe { gethostname(core::ptr::null_mut(), 8 as size_t) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut info: SysInfo = unsafe { mem::zeroed() };
  let result = unsafe { sysinfo(&raw mut info) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn sysinfo_writes_snapshot_and_preserves_errno_on_success() {
  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut info: SysInfo = unsafe { mem::zeroed() };

  write_errno(999);

  let result = unsafe { sysinfo(&raw mut info) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), 999);
  assert!(info.mem_unit > 0);
  assert!(info.totalram > 0);
  assert!(info.uptime >= 0);
}

#[test]
fn sysinfo_repeated_success_keeps_errno_sentinel() {
  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut first_info: SysInfo = unsafe { mem::zeroed() };
  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut second_info: SysInfo = unsafe { mem::zeroed() };

  write_errno(271);

  let first = unsafe { sysinfo(&raw mut first_info) };
  let first_errno = read_errno();
  let second = unsafe { sysinfo(&raw mut second_info) };
  let second_errno = read_errno();

  assert_eq!(first, 0);
  assert_eq!(second, 0);
  assert_eq!(first_errno, 271);
  assert_eq!(second_errno, 271);
  assert!(first_info.mem_unit > 0);
  assert!(second_info.mem_unit > 0);
}

#[test]
fn sysinfo_success_does_not_clear_errno_after_prior_failure() {
  write_errno(0);

  let failed = unsafe { sysinfo(core::ptr::null_mut()) };

  assert_eq!(failed, -1);
  assert_eq!(read_errno(), EFAULT);

  // SAFETY: zero is valid for this C struct because all fields are integer scalars.
  let mut info: SysInfo = unsafe { mem::zeroed() };
  let result = unsafe { sysinfo(&raw mut info) };

  assert_eq!(result, 0);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn sysinfo_invalid_pointer_sets_efault() {
  write_errno(ENAMETOOLONG);

  let result = unsafe { sysinfo(std::ptr::dangling_mut::<SysInfo>()) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn sysinfo_null_pointer_overwrites_previous_errno_with_efault() {
  write_errno(ENAMETOOLONG);

  let result = unsafe { sysinfo(core::ptr::null_mut()) };

  assert_eq!(result, -1);
  assert_eq!(read_errno(), EFAULT);
}
