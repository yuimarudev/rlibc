#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::mem::size_of;
use rlibc::netdb::{
  AF_INET, AF_INET6, EAI_BADFLAGS, EAI_FAMILY, EAI_NONAME, EAI_OVERFLOW, NI_DGRAM, NI_NAMEREQD,
  NI_NOFQDN, NI_NUMERICHOST, NI_NUMERICSERV, getnameinfo, in_addr, in6_addr, sockaddr, sockaddr_in,
  sockaddr_in6, socklen_t,
};
use std::ffi::CStr;

fn slen(value: usize) -> socklen_t {
  socklen_t::try_from(value)
    .unwrap_or_else(|_| unreachable!("usize does not fit into socklen_t on this target"))
}

fn af_inet_family() -> u16 {
  u16::try_from(AF_INET)
    .unwrap_or_else(|_| unreachable!("AF_INET must fit sockaddr_in::sin_family"))
}

fn af_inet6_family() -> u16 {
  u16::try_from(AF_INET6)
    .unwrap_or_else(|_| unreachable!("AF_INET6 must fit sockaddr_in6::sin6_family"))
}

#[test]
fn getnameinfo_ipv4_returns_numeric_host_and_service() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 8080_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0_i8; 64];
  let mut serv = [0_i8; 32];

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      serv.as_mut_ptr(),
      slen(serv.len()),
      0,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to both outputs.
  let host_text = unsafe { CStr::from_ptr(host.as_ptr()) };
  // SAFETY: successful getnameinfo writes a terminating NUL to both outputs.
  let serv_text = unsafe { CStr::from_ptr(serv.as_ptr()) };

  assert_eq!(
    host_text.to_str().expect("host output must be UTF-8"),
    "127.0.0.1"
  );
  assert_eq!(
    serv_text.to_str().expect("service output must be UTF-8"),
    "8080"
  );
}

#[test]
fn getnameinfo_ipv6_returns_numeric_host_and_service() {
  let address = sockaddr_in6 {
    sin6_family: af_inet6_family(),
    sin6_port: 443_u16.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr {
      s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    },
    sin6_scope_id: 0,
  };
  let mut host = [0_i8; 96];
  let mut serv = [0_i8; 32];

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in6>()),
      host.as_mut_ptr(),
      slen(host.len()),
      serv.as_mut_ptr(),
      slen(serv.len()),
      0,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to both outputs.
  let host_text = unsafe { CStr::from_ptr(host.as_ptr()) };
  // SAFETY: successful getnameinfo writes a terminating NUL to both outputs.
  let serv_text = unsafe { CStr::from_ptr(serv.as_ptr()) };

  assert_eq!(
    host_text.to_str().expect("host output must be UTF-8"),
    "::1"
  );
  assert_eq!(
    serv_text.to_str().expect("service output must be UTF-8"),
    "443"
  );
}

#[test]
fn getnameinfo_rejects_unknown_flags() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 53_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on EAI_BADFLAGS",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on EAI_BADFLAGS",
  );
}

#[test]
fn getnameinfo_rejects_unknown_flags_for_service_only_name_required_request() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 53_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut service = [0x58_i8; 32];

  // SAFETY: service output is requested and writable; unsupported bits must be
  // rejected before any name-required or service formatting path is taken.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(0),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD | 0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on early EAI_BADFLAGS",
  );
}

#[test]
fn getnameinfo_rejects_unknown_flags_for_service_only_short_addrlen_request() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 53_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut service = [0x58_i8; 32];

  // SAFETY: service output is writable and requested; unsupported flags must
  // be rejected before short-addrlen validation.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>() - 1),
      core::ptr::null_mut(),
      slen(0),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD | 0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on early EAI_BADFLAGS",
  );
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_address_validation() {
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: output buffers are writable; `addr` is intentionally null to
  // verify unknown-flag validation short-circuits before address checks.
  let status = unsafe {
    getnameinfo(
      core::ptr::null(),
      slen(0),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on early EAI_BADFLAGS",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on early EAI_BADFLAGS",
  );
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_overflow_checks() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0x58_i8; 1];
  let mut service = [0x58_i8; 1];

  // SAFETY: buffers are writable; zero lengths would trigger overflow if
  // reached, but unknown flags must be rejected first.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(0),
      service.as_mut_ptr(),
      slen(0),
      0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert_eq!(host[0], 0x58_i8, "host output must remain untouched");
  assert_eq!(service[0], 0x58_i8, "service output must remain untouched");
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_no_output_noname() {
  // SAFETY: address/output pointers are intentionally null to verify validation
  // order for unsupported flags.
  let status = unsafe {
    getnameinfo(
      core::ptr::null(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_no_output_noname_with_name_required() {
  // SAFETY: address/output pointers are intentionally null. Unsupported flags
  // must be rejected before no-output and name-required branches.
  let status = unsafe {
    getnameinfo(
      core::ptr::null(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      NI_NAMEREQD | 0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_no_output_length_handling() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };

  // SAFETY: outputs are intentionally disabled with non-zero lengths and a
  // short `addrlen`; bad-flags validation must still run first.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr>() - 1),
      core::ptr::null_mut(),
      slen(64),
      core::ptr::null_mut(),
      slen(32),
      0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_name_required_error() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: with `NI_NAMEREQD` plus an unsupported bit set, bad-flag
  // validation must win before name-required checks.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD | 0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on early EAI_BADFLAGS",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on early EAI_BADFLAGS",
  );
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_family_checks() {
  let address = sockaddr {
    sa_family: 1234_u16,
    sa_data: [0; 14],
  };
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: output buffers are writable and address is readable; unsupported
  // flags must be rejected before family validation.
  let status = unsafe {
    getnameinfo(
      &raw const address,
      slen(size_of::<sockaddr>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on early EAI_BADFLAGS",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on early EAI_BADFLAGS",
  );
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_length_checks() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: `addrlen` is intentionally short (would be `EAI_FAMILY`), but
  // unsupported flags must be rejected first.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>() - 1),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on early EAI_BADFLAGS",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on early EAI_BADFLAGS",
  );
}

#[test]
fn getnameinfo_rejects_unknown_flags_before_ipv6_length_checks() {
  let address = sockaddr_in6 {
    sin6_family: af_inet6_family(),
    sin6_port: 80_u16.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr {
      s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    },
    sin6_scope_id: 0,
  };
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: `addrlen` is intentionally short (would be `EAI_FAMILY`), but
  // unsupported flags must still be rejected first for IPv6.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in6>() - 1),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0x4000,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on early EAI_BADFLAGS",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on early EAI_BADFLAGS",
  );
}

#[test]
fn getnameinfo_rejects_unsupported_family() {
  let address = sockaddr {
    sa_family: 1234_u16,
    sa_data: [0; 14],
  };
  let mut host = [0_i8; 32];

  // SAFETY: pointers refer to valid writable buffers and a valid generic sockaddr.
  let status = unsafe {
    getnameinfo(
      &raw const address,
      slen(size_of::<sockaddr>()),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(status, EAI_FAMILY);
}

#[test]
fn getnameinfo_rejects_short_sockaddr_lengths_for_ip_families() {
  let ipv4 = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let ipv6 = sockaddr_in6 {
    sin6_family: af_inet6_family(),
    sin6_port: 80_u16.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr {
      s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    },
    sin6_scope_id: 0,
  };
  let mut host = [0_i8; 64];

  // SAFETY: pointer references a valid `sockaddr_in`; `addrlen` intentionally
  // truncates by one byte to verify IPv4 length validation.
  let ipv4_status = unsafe {
    getnameinfo(
      (&raw const ipv4).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>() - 1),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(ipv4_status, EAI_FAMILY);

  // SAFETY: pointer references a valid `sockaddr_in6`; `addrlen` intentionally
  // truncates by one byte to verify IPv6 length checks.
  let ipv6_status = unsafe {
    getnameinfo(
      (&raw const ipv6).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in6>() - 1),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(ipv6_status, EAI_FAMILY);
}

#[test]
fn getnameinfo_returns_eai_overflow_when_output_buffers_are_too_small() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 65535_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host_too_small = [0_i8; 9];
  let mut service_too_small = [0_i8; 5];

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let host_status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host_too_small.as_mut_ptr(),
      slen(host_too_small.len()),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(host_status, EAI_OVERFLOW);

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let service_status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(0),
      service_too_small.as_mut_ptr(),
      slen(service_too_small.len()),
      0,
    )
  };

  assert_eq!(service_status, EAI_OVERFLOW);
}

#[test]
fn getnameinfo_returns_eai_overflow_for_zero_length_output_buffers() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0_i8; 1];
  let mut service = [0_i8; 1];

  // SAFETY: pointer arguments are valid; the zero `hostlen` requests an
  // overflow path for a non-null host buffer.
  let host_status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(host_status, EAI_OVERFLOW);

  // SAFETY: pointer arguments are valid; the zero `servlen` requests an
  // overflow path for a non-null service buffer.
  let service_status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(0),
      service.as_mut_ptr(),
      slen(0),
      0,
    )
  };

  assert_eq!(service_status, EAI_OVERFLOW);
}

#[test]
fn getnameinfo_returns_eai_noname_when_no_output_is_requested() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };

  // SAFETY: address pointer is valid; both output pointers are null by design
  // to verify `EAI_NONAME`.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(status, EAI_NONAME);
}

#[test]
fn getnameinfo_returns_eai_noname_when_no_output_is_requested_with_nonzero_lengths() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };

  // SAFETY: output pointers are null by design; non-zero lengths must be
  // ignored because no outputs are requested.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(64),
      core::ptr::null_mut(),
      slen(32),
      0,
    )
  };

  assert_eq!(status, EAI_NONAME);
}

#[test]
fn getnameinfo_returns_eai_noname_with_name_required_when_no_output_and_nonzero_lengths() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };

  // SAFETY: output pointers are null by design; lengths are intentionally
  // non-zero and must still be ignored for no-output requests.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(64),
      core::ptr::null_mut(),
      slen(32),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, EAI_NONAME);
}

#[test]
fn getnameinfo_returns_eai_noname_with_name_required_when_no_output_is_requested() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };

  // SAFETY: address pointer is valid; both outputs are intentionally not
  // requested and must return `EAI_NONAME` even with `NI_NAMEREQD`.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, EAI_NONAME);
}

#[test]
fn getnameinfo_returns_eai_noname_before_address_validation_when_no_output_is_requested() {
  // SAFETY: both output pointers are null by design and `addr` is also null to
  // verify `EAI_NONAME` is returned before address validation.
  let status = unsafe {
    getnameinfo(
      core::ptr::null(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(status, EAI_NONAME);
}

#[test]
fn getnameinfo_returns_eai_noname_before_length_validation_when_no_output_is_requested() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };

  // SAFETY: both outputs are intentionally disabled and `addrlen` is
  // intentionally too short; `EAI_NONAME` must win before length validation.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr>() - 1),
      core::ptr::null_mut(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(status, EAI_NONAME);
}

#[test]
fn getnameinfo_returns_eai_noname_before_family_validation_when_no_output_is_requested() {
  let address = sockaddr {
    sa_family: 1234_u16,
    sa_data: [0; 14],
  };

  // SAFETY: both outputs are intentionally disabled and family is unsupported;
  // `EAI_NONAME` must win before family validation.
  let status = unsafe {
    getnameinfo(
      &raw const address,
      slen(size_of::<sockaddr>()),
      core::ptr::null_mut(),
      slen(0),
      core::ptr::null_mut(),
      slen(0),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, EAI_NONAME);
}

#[test]
fn getnameinfo_returns_eai_noname_for_name_required_without_reverse_lookup() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0_i8; 64];

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(0),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, EAI_NONAME);
}

#[test]
fn getnameinfo_allows_numeric_host_when_name_required_and_numerichost_set() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0_i8; 64];

  // SAFETY: pointers are valid; NI_NUMERICHOST should allow numeric host
  // formatting even when NI_NAMEREQD is also set.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(0),
      NI_NAMEREQD | NI_NUMERICHOST,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to host.
  let host_text = unsafe { CStr::from_ptr(host.as_ptr()) };

  assert_eq!(
    host_text.to_str().expect("host output must be UTF-8"),
    "127.0.0.1"
  );
}

#[test]
fn getnameinfo_allows_numeric_ipv6_host_when_name_required_and_numerichost_set() {
  let address = sockaddr_in6 {
    sin6_family: af_inet6_family(),
    sin6_port: 443_u16.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr {
      s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    },
    sin6_scope_id: 0,
  };
  let mut host = [0_i8; 96];

  // SAFETY: pointers are valid; NI_NUMERICHOST should allow numeric host
  // formatting even when NI_NAMEREQD is also set.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in6>()),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(0),
      NI_NAMEREQD | NI_NUMERICHOST,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to host.
  let host_text = unsafe { CStr::from_ptr(host.as_ptr()) };

  assert_eq!(
    host_text.to_str().expect("host output must be UTF-8"),
    "::1"
  );
}

#[test]
fn getnameinfo_allows_numeric_ipv6_host_and_service_when_name_required_and_numerichost_set() {
  let address = sockaddr_in6 {
    sin6_family: af_inet6_family(),
    sin6_port: 443_u16.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr {
      s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    },
    sin6_scope_id: 0,
  };
  let mut host = [0_i8; 96];
  let mut service = [0_i8; 32];

  // SAFETY: pointers are valid; NI_NUMERICHOST should allow numeric host
  // formatting even when NI_NAMEREQD is also set.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in6>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD | NI_NUMERICHOST,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to outputs.
  let host_text = unsafe { CStr::from_ptr(host.as_ptr()) };
  // SAFETY: successful getnameinfo writes a terminating NUL to outputs.
  let service_text = unsafe { CStr::from_ptr(service.as_ptr()) };

  assert_eq!(
    host_text.to_str().expect("host output must be UTF-8"),
    "::1"
  );
  assert_eq!(
    service_text.to_str().expect("service output must be UTF-8"),
    "443"
  );
}

#[test]
fn getnameinfo_accepts_numeric_mode_noop_flags_with_name_required_and_numerichost() {
  let address = sockaddr_in6 {
    sin6_family: af_inet6_family(),
    sin6_port: 53_u16.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr {
      s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    },
    sin6_scope_id: 0,
  };
  let mut host = [0_i8; 96];
  let mut service = [0_i8; 32];

  // SAFETY: pointers are valid; NI_NOFQDN/NI_NUMERICSERV/NI_DGRAM are accepted
  // no-op flags in this numeric-only implementation.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in6>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD | NI_NUMERICHOST | NI_NOFQDN | NI_NUMERICSERV | NI_DGRAM,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to outputs.
  let host_text = unsafe { CStr::from_ptr(host.as_ptr()) };
  // SAFETY: successful getnameinfo writes a terminating NUL to outputs.
  let service_text = unsafe { CStr::from_ptr(service.as_ptr()) };

  assert_eq!(
    host_text.to_str().expect("host output must be UTF-8"),
    "::1"
  );
  assert_eq!(
    service_text.to_str().expect("service output must be UTF-8"),
    "53"
  );
}

#[test]
fn getnameinfo_name_required_with_numerichost_and_small_host_buffer_returns_overflow() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host_too_small = [0x58_i8; 2];

  // SAFETY: pointer arguments are valid; the host buffer is intentionally too
  // small for "127.0.0.1" + trailing NUL.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host_too_small.as_mut_ptr(),
      slen(host_too_small.len()),
      core::ptr::null_mut(),
      slen(0),
      NI_NAMEREQD | NI_NUMERICHOST,
    )
  };

  assert_eq!(status, EAI_OVERFLOW);
  assert!(
    host_too_small.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on EAI_OVERFLOW",
  );
}

#[test]
fn getnameinfo_name_required_with_numerichost_and_small_ipv6_host_buffer_keeps_outputs_unmodified()
{
  let address = sockaddr_in6 {
    sin6_family: af_inet6_family(),
    sin6_port: 443_u16.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr {
      s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    },
    sin6_scope_id: 0,
  };
  let mut host_too_small = [0x58_i8; 2];
  let mut service = [0x58_i8; 32];

  // SAFETY: pointer arguments are valid; host buffer is intentionally too
  // small for the IPv6 numeric host text, and outputs should remain unchanged.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in6>()),
      host_too_small.as_mut_ptr(),
      slen(host_too_small.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD | NI_NUMERICHOST,
    )
  };

  assert_eq!(status, EAI_OVERFLOW);
  assert!(
    host_too_small.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on EAI_OVERFLOW",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched when host output overflows",
  );
}

#[test]
fn getnameinfo_name_required_with_numerichost_and_small_service_buffer_keeps_outputs_unmodified() {
  let address = sockaddr_in6 {
    sin6_family: af_inet6_family(),
    sin6_port: 443_u16.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr {
      s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    },
    sin6_scope_id: 0,
  };
  let mut host = [0x58_i8; 96];
  let mut service_too_small = [0x58_i8; 2];

  // SAFETY: pointer arguments are valid; service buffer is intentionally too
  // small for "443" + trailing NUL, and outputs should remain unchanged.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in6>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service_too_small.as_mut_ptr(),
      slen(service_too_small.len()),
      NI_NAMEREQD | NI_NUMERICHOST,
    )
  };

  assert_eq!(status, EAI_OVERFLOW);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched when service overflows",
  );
  assert!(
    service_too_small.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on EAI_OVERFLOW",
  );
}

#[test]
fn getnameinfo_name_required_with_numerichost_and_zero_host_length_returns_overflow_first() {
  let mut host = [0x58_i8; 16];
  let mut service = [0x58_i8; 32];

  // SAFETY: output buffers are writable. `hostlen=0` must trigger
  // `EAI_OVERFLOW` before address validation and before any output writes.
  let status = unsafe {
    getnameinfo(
      core::ptr::null(),
      slen(0),
      host.as_mut_ptr(),
      slen(0),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD | NI_NUMERICHOST,
    )
  };

  assert_eq!(status, EAI_OVERFLOW);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on zero-length host EAI_OVERFLOW",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched when host length is zero",
  );
}

#[test]
fn getnameinfo_service_only_name_required_with_numerichost_and_zero_service_length_returns_overflow_first()
 {
  let mut service = [0x58_i8; 16];

  // SAFETY: service buffer is writable. `servlen=0` with non-null `serv` must
  // return `EAI_OVERFLOW` before address validation; `hostlen` is ignored
  // because `host == NULL`.
  let status = unsafe {
    getnameinfo(
      core::ptr::null(),
      slen(0),
      core::ptr::null_mut(),
      slen(64),
      service.as_mut_ptr(),
      slen(0),
      NI_NAMEREQD | NI_NUMERICHOST,
    )
  };

  assert_eq!(status, EAI_OVERFLOW);
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on zero-length service EAI_OVERFLOW",
  );
}

#[test]
fn getnameinfo_name_required_host_only_error_leaves_host_unmodified() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0x58_i8; 64];

  // SAFETY: host buffer is writable; `NI_NAMEREQD` is expected to fail before
  // any output write, and `servlen` should be ignored because `serv == NULL`.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(32),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on NI_NAMEREQD EAI_NONAME",
  );
}

#[test]
fn getnameinfo_name_required_error_precedes_output_overflow_checks() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 80_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host_too_small = [0x58_i8; 1];
  let mut service_too_small = [0x58_i8; 1];

  // SAFETY: `NI_NAMEREQD` should fail before any host/service overflow checks.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host_too_small.as_mut_ptr(),
      slen(host_too_small.len()),
      service_too_small.as_mut_ptr(),
      slen(service_too_small.len()),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert_eq!(
    host_too_small[0], 0x58_i8,
    "host output must remain untouched on NI_NAMEREQD EAI_NONAME",
  );
  assert_eq!(
    service_too_small[0], 0x58_i8,
    "service output must remain untouched on NI_NAMEREQD EAI_NONAME",
  );
}

#[test]
fn getnameinfo_returns_eai_family_for_null_addr_pointer() {
  let mut host = [0_i8; 64];

  // SAFETY: host buffer is writable and `addr` is intentionally null to check
  // family/length validation behavior.
  let status = unsafe {
    getnameinfo(
      core::ptr::null(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(0),
      0,
    )
  };

  assert_eq!(status, EAI_FAMILY);
}

#[test]
fn getnameinfo_null_addr_error_leaves_requested_outputs_unmodified() {
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: output buffers are writable; `addr` is intentionally null to
  // verify `EAI_FAMILY` without mutating requested outputs.
  let status = unsafe {
    getnameinfo(
      core::ptr::null(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0,
    )
  };

  assert_eq!(status, EAI_FAMILY);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on null-addr EAI_FAMILY",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on null-addr EAI_FAMILY",
  );
}

#[test]
fn getnameinfo_allows_service_only_request_with_name_required_flag() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 53_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut service = [0_i8; 32];

  // SAFETY: pointers refer to valid address/service buffers; host output is
  // intentionally not requested.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(0),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to service.
  let service_text = unsafe { CStr::from_ptr(service.as_ptr()) };

  assert_eq!(
    service_text.to_str().expect("service output must be UTF-8"),
    "53"
  );
}

#[test]
fn getnameinfo_ignores_hostlen_when_host_output_is_not_requested() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 123_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut service = [0_i8; 32];

  // SAFETY: host output is intentionally not requested (`host == NULL`), so a
  // non-zero host length must be ignored.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      core::ptr::null_mut(),
      slen(64),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to service.
  let service_text = unsafe { CStr::from_ptr(service.as_ptr()) };

  assert_eq!(
    service_text.to_str().expect("service output must be UTF-8"),
    "123"
  );
}

#[test]
fn getnameinfo_ignores_servlen_when_service_output_is_not_requested() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 456_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0_i8; 64];

  // SAFETY: service output is intentionally not requested (`serv == NULL`), so
  // a non-zero service length must be ignored.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      core::ptr::null_mut(),
      slen(64),
      0,
    )
  };

  assert_eq!(status, 0);

  // SAFETY: successful getnameinfo writes a terminating NUL to host.
  let host_text = unsafe { CStr::from_ptr(host.as_ptr()) };

  assert_eq!(
    host_text.to_str().expect("host output must be UTF-8"),
    "127.0.0.1"
  );
}

#[test]
fn getnameinfo_does_not_write_host_when_service_output_overflows() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 65535_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0x58_i8; 64];
  let mut service_too_small = [0x58_i8; 5];

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service_too_small.as_mut_ptr(),
      slen(service_too_small.len()),
      0,
    )
  };

  assert_eq!(status, EAI_OVERFLOW);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched when service output overflows",
  );
  assert!(
    service_too_small.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched when service output overflows",
  );
}

#[test]
fn getnameinfo_does_not_write_service_when_host_output_overflows() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 65535_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host_too_small = [0x58_i8; 9];
  let mut service = [0x58_i8; 32];

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host_too_small.as_mut_ptr(),
      slen(host_too_small.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0,
    )
  };

  assert_eq!(status, EAI_OVERFLOW);
  assert!(
    host_too_small.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched when host output overflows",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched when host output overflows",
  );
}

#[test]
fn getnameinfo_name_required_error_leaves_requested_outputs_unmodified() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 5353_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: pointers refer to valid writable buffers and a valid socket address.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      NI_NAMEREQD,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched when NI_NAMEREQD cannot be satisfied",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched when NI_NAMEREQD cannot be satisfied",
  );
}

#[test]
fn getnameinfo_family_error_leaves_requested_outputs_unmodified() {
  let address = sockaddr {
    sa_family: 1234_u16,
    sa_data: [0; 14],
  };
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: pointers are valid writable buffers; address family is
  // intentionally unsupported to validate error-path non-mutation.
  let status = unsafe {
    getnameinfo(
      &raw const address,
      slen(size_of::<sockaddr>()),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0,
    )
  };

  assert_eq!(status, EAI_FAMILY);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on EAI_FAMILY",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on EAI_FAMILY",
  );
}

#[test]
fn getnameinfo_short_addrlen_error_leaves_requested_outputs_unmodified() {
  let address = sockaddr_in {
    sin_family: af_inet_family(),
    sin_port: 8080_u16.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes([127, 0, 0, 1]),
    },
    sin_zero: [0; 8],
  };
  let mut host = [0x58_i8; 64];
  let mut service = [0x58_i8; 32];

  // SAFETY: address is valid, but `addrlen` is intentionally one byte short
  // to trigger `EAI_FAMILY` without writing outputs.
  let status = unsafe {
    getnameinfo(
      (&raw const address).cast::<sockaddr>(),
      slen(size_of::<sockaddr_in>() - 1),
      host.as_mut_ptr(),
      slen(host.len()),
      service.as_mut_ptr(),
      slen(service.len()),
      0,
    )
  };

  assert_eq!(status, EAI_FAMILY);
  assert!(
    host.iter().all(|&byte| byte == 0x58_i8),
    "host output must remain untouched on short-addrlen EAI_FAMILY",
  );
  assert!(
    service.iter().all(|&byte| byte == 0x58_i8),
    "service output must remain untouched on short-addrlen EAI_FAMILY",
  );
}
