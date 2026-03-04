#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use core::ptr;
use rlibc::netdb::{
  AF_INET, AF_INET6, AF_UNSPEC, AI_NUMERICHOST, AI_NUMERICSERV, AI_PASSIVE, EAI_BADFLAGS,
  EAI_FAMILY, EAI_NONAME, EAI_SERVICE, IPPROTO_TCP, IPPROTO_UDP, SOCK_DGRAM, SOCK_STREAM, addrinfo,
  freeaddrinfo, getaddrinfo, sockaddr, sockaddr_in, sockaddr_in6,
};
use std::ffi::CString;
use std::net::{Ipv4Addr, Ipv6Addr};

const fn empty_hints() -> addrinfo {
  addrinfo {
    ai_flags: 0,
    ai_family: AF_UNSPEC,
    ai_socktype: 0,
    ai_protocol: 0,
    ai_addrlen: 0,
    ai_addr: ptr::null_mut(),
    ai_canonname: ptr::null_mut(),
    ai_next: ptr::null_mut(),
  }
}

fn sockaddr_ptr_as_in(addr: *mut sockaddr) -> *const sockaddr_in {
  addr.addr() as *const sockaddr_in
}

fn sockaddr_ptr_as_in6(addr: *mut sockaddr) -> *const sockaddr_in6 {
  addr.addr() as *const sockaddr_in6
}

#[test]
fn getaddrinfo_ipv4_numeric_host_and_service_populates_sockaddr_in() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("8080").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;

    assert_eq!(entry.ai_family, AF_INET);
    assert_eq!(entry.ai_socktype, SOCK_STREAM);
    assert_eq!(entry.ai_protocol, IPPROTO_TCP);
    assert!(entry.ai_next.is_null());

    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 8080);
    assert_eq!(socket_addr.sin_addr.s_addr.to_be_bytes(), [127, 0, 0, 1]);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_accepts_ipv4_shorthand_host_with_ai_numerichost() {
  let host = CString::new("127.1").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 80);
    assert_eq!(socket_addr.sin_addr.s_addr.to_be_bytes(), [127, 0, 0, 1]);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_ipv6_numeric_host_and_service_populates_sockaddr_in6() {
  let host = CString::new("::1").expect("host literal must be NUL-free");
  let service = CString::new("443").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET6;
  hints.ai_socktype = SOCK_DGRAM;
  hints.ai_protocol = IPPROTO_UDP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;

    assert_eq!(entry.ai_family, AF_INET6);
    assert_eq!(entry.ai_socktype, SOCK_DGRAM);
    assert_eq!(entry.ai_protocol, IPPROTO_UDP);

    let socket_addr = &*sockaddr_ptr_as_in6(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin6_port), 443);
    assert_eq!(
      socket_addr.sin6_addr.s6_addr,
      [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_hints_uses_default_stream_tcp_profile() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("8081").expect("service literal must be NUL-free");
  let mut result = ptr::null_mut();

  // SAFETY: pointers are valid for the duration of this call; null hints is supported.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      ptr::null(),
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;

    assert_eq!(first.ai_family, AF_INET);
    assert_eq!(first.ai_socktype, SOCK_STREAM);
    assert_eq!(first.ai_protocol, IPPROTO_TCP);
    assert!(first.ai_next.is_null());

    let socket_addr = &*sockaddr_ptr_as_in(first.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 8081);
    assert_eq!(socket_addr.sin_addr.s_addr.to_be_bytes(), [127, 0, 0, 1]);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_hints_uses_default_stream_tcp_profile_for_ipv6() {
  let host = CString::new("::1").expect("host literal must be NUL-free");
  let service = CString::new("8082").expect("service literal must be NUL-free");
  let mut result = ptr::null_mut();

  // SAFETY: pointers are valid for the duration of this call; null hints is supported.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      ptr::null(),
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(first.ai_socktype, SOCK_STREAM);
    assert_eq!(first.ai_protocol, IPPROTO_TCP);
    assert!(first.ai_next.is_null());

    let socket_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin6_port), 8082);
    assert_eq!(
      Ipv6Addr::from(socket_addr.sin6_addr.s6_addr),
      Ipv6Addr::LOCALHOST
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_hints_with_null_service_uses_zero_port_for_ipv6() {
  let host = CString::new("::1").expect("host literal must be NUL-free");
  let mut result = ptr::null_mut();

  // SAFETY: pointers are valid for the duration of this call; null hints and service are supported.
  let status = unsafe { getaddrinfo(host.as_ptr(), ptr::null(), ptr::null(), &raw mut result) };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(first.ai_socktype, SOCK_STREAM);
    assert_eq!(first.ai_protocol, IPPROTO_TCP);
    assert!(first.ai_next.is_null());

    let socket_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin6_port), 0);
    assert_eq!(
      Ipv6Addr::from(socket_addr.sin6_addr.s6_addr),
      Ipv6Addr::LOCALHOST
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_hints_with_null_service_uses_zero_port_for_ipv4() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let mut result = ptr::null_mut();

  // SAFETY: pointers are valid for the duration of this call; null hints and service are supported.
  let status = unsafe { getaddrinfo(host.as_ptr(), ptr::null(), ptr::null(), &raw mut result) };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;

    assert_eq!(first.ai_family, AF_INET);
    assert_eq!(first.ai_socktype, SOCK_STREAM);
    assert_eq!(first.ai_protocol, IPPROTO_TCP);
    assert!(first.ai_next.is_null());

    let socket_addr = &*sockaddr_ptr_as_in(first.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 0);
    assert_eq!(
      Ipv4Addr::from(socket_addr.sin_addr.s_addr.to_be_bytes()),
      Ipv4Addr::LOCALHOST
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_hints_with_null_node_uses_loopback_addresses_with_service_port() {
  let service = CString::new("8083").expect("service literal must be NUL-free");
  let mut result = ptr::null_mut();

  // SAFETY: pointers are valid for the duration of this call; null hints and node are supported.
  let status = unsafe { getaddrinfo(ptr::null(), service.as_ptr(), ptr::null(), &raw mut result) };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(first.ai_socktype, SOCK_STREAM);
    assert_eq!(first.ai_protocol, IPPROTO_TCP);

    let first_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(u16::from_be(first_addr.sin6_port), 8083);
    assert_eq!(
      Ipv6Addr::from(first_addr.sin6_addr.s6_addr),
      Ipv6Addr::LOCALHOST
    );
    assert!(!first.ai_next.is_null());

    let second = &*first.ai_next;

    assert_eq!(second.ai_family, AF_INET);
    assert_eq!(second.ai_socktype, SOCK_STREAM);
    assert_eq!(second.ai_protocol, IPPROTO_TCP);
    assert!(second.ai_next.is_null());

    let second_addr = &*sockaddr_ptr_as_in(second.ai_addr);

    assert_eq!(u16::from_be(second_addr.sin_port), 8083);
    assert_eq!(
      Ipv4Addr::from(second_addr.sin_addr.s_addr.to_be_bytes()),
      Ipv4Addr::LOCALHOST
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_hints_with_null_node_accepts_zero_service_port() {
  let service = CString::new("0").expect("service literal must be NUL-free");
  let mut result = ptr::null_mut();

  // SAFETY: pointers are valid for the duration of this call; null hints and node are supported.
  let status = unsafe { getaddrinfo(ptr::null(), service.as_ptr(), ptr::null(), &raw mut result) };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;
    let first_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(first.ai_socktype, SOCK_STREAM);
    assert_eq!(first.ai_protocol, IPPROTO_TCP);
    assert_eq!(u16::from_be(first_addr.sin6_port), 0);
    assert_eq!(
      Ipv6Addr::from(first_addr.sin6_addr.s6_addr),
      Ipv6Addr::LOCALHOST
    );
    assert!(!first.ai_next.is_null());

    let second = &*first.ai_next;
    let second_addr = &*sockaddr_ptr_as_in(second.ai_addr);

    assert_eq!(second.ai_family, AF_INET);
    assert_eq!(second.ai_socktype, SOCK_STREAM);
    assert_eq!(second.ai_protocol, IPPROTO_TCP);
    assert_eq!(u16::from_be(second_addr.sin_port), 0);
    assert_eq!(
      Ipv4Addr::from(second_addr.sin_addr.s_addr.to_be_bytes()),
      Ipv4Addr::LOCALHOST
    );
    assert!(second.ai_next.is_null());
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_hints_rejects_when_node_and_service_are_both_null() {
  let mut result = ptr::null_mut();

  // SAFETY: pointers are valid for the duration of this call.
  let status = unsafe { getaddrinfo(ptr::null(), ptr::null(), ptr::null(), &raw mut result) };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_invalid_numeric_host_literal() {
  let host = CString::new("not-an-ip").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_plus_prefixed_ipv4_shorthand_host_with_ai_numerichost() {
  let host = CString::new("+127.1").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_ipv6_numeric_host_with_ipv4_family_hint() {
  let host = CString::new("::1").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_FAMILY);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_ipv4_numeric_host_with_ipv6_family_hint() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET6;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_FAMILY);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_whitespace_wrapped_numeric_host_with_ai_numerichost() {
  let host = CString::new(" 127.0.0.1 ").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_newline_terminated_numeric_host_with_ai_numerichost() {
  let host = CString::new("127.0.0.1\n").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_tab_terminated_numeric_host_with_ai_numerichost() {
  let host = CString::new("127.0.0.1\t").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_scoped_ipv6_host_with_ai_numerichost() {
  let host = CString::new("fe80::1%eth0").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_ipv4_literal_with_trailing_dot_with_ai_numerichost() {
  let host = CString::new("127.0.0.1.").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_ipv6_literal_with_trailing_dot_with_ai_numerichost() {
  let host = CString::new("::1.").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_bracketed_ipv6_host_with_ai_numerichost() {
  let host = CString::new("[::1]").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_bracketed_scoped_ipv6_host_with_ai_numerichost() {
  let host = CString::new("[fe80::1%eth0]").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_bracketed_ipv4_host_with_ai_numerichost() {
  let host = CString::new("[127.0.0.1]").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_bracketed_ipv4_host_with_port_with_ai_numerichost() {
  let host = CString::new("[127.0.0.1]:80").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_bracketed_ipv6_host_with_port_with_ai_numerichost() {
  let host = CString::new("[::1]:80").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_bracketed_scoped_ipv6_with_port_with_ai_numerichost() {
  let host = CString::new("[fe80::1%eth0]:80").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_non_utf8_hostname() {
  let host = CString::new(vec![0xff]).expect("hostname bytes must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_empty_hostname_without_ai_numerichost() {
  let host = CString::new("").expect("hostname bytes must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_empty_hostname_with_ai_numerichost() {
  let host = CString::new("").expect("hostname bytes must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_resolves_localhost_hostname_without_ai_numerichost() {
  let host = CString::new("localhost").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;

    assert_eq!(entry.ai_family, AF_INET);
    assert_eq!(entry.ai_socktype, SOCK_STREAM);
    assert_eq!(entry.ai_protocol, IPPROTO_TCP);

    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 80);
    assert!(Ipv4Addr::from(socket_addr.sin_addr.s_addr.to_be_bytes()).is_loopback());
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_resolves_localhost_hostname_with_ipv6_hint() {
  let host = CString::new("localhost").expect("host literal must be NUL-free");
  let service = CString::new("443").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_INET6;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;

    assert_eq!(entry.ai_family, AF_INET6);
    assert_eq!(entry.ai_socktype, SOCK_STREAM);
    assert_eq!(entry.ai_protocol, IPPROTO_TCP);

    let socket_addr = &*sockaddr_ptr_as_in6(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin6_port), 443);
    assert!(Ipv6Addr::from(socket_addr.sin6_addr.s6_addr).is_loopback());
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_localhost_unspec_returns_ipv6_loopback_first() {
  let host = CString::new("localhost").expect("host literal must be NUL-free");
  let service = CString::new("53").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_DGRAM;
  hints.ai_protocol = IPPROTO_UDP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(first.ai_socktype, SOCK_DGRAM);
    assert_eq!(first.ai_protocol, IPPROTO_UDP);

    let first_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(u16::from_be(first_addr.sin6_port), 53);
    assert!(Ipv6Addr::from(first_addr.sin6_addr.s6_addr).is_loopback());
    assert!(!first.ai_next.is_null());

    let second = &*first.ai_next;

    assert_eq!(second.ai_family, AF_INET);
    assert_eq!(second.ai_socktype, SOCK_DGRAM);
    assert_eq!(second.ai_protocol, IPPROTO_UDP);

    let second_addr = &*sockaddr_ptr_as_in(second.ai_addr);

    assert_eq!(u16::from_be(second_addr.sin_port), 53);
    assert!(Ipv4Addr::from(second_addr.sin_addr.s_addr.to_be_bytes()).is_loopback());
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_localhost_trailing_dot_unspec_returns_ipv6_loopback_first() {
  let host = CString::new("LOCALHOST.").expect("host literal must be NUL-free");
  let service = CString::new("1234").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_DGRAM;
  hints.ai_protocol = IPPROTO_UDP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;
    let first_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(first.ai_socktype, SOCK_DGRAM);
    assert_eq!(first.ai_protocol, IPPROTO_UDP);
    assert_eq!(u16::from_be(first_addr.sin6_port), 1234);
    assert!(Ipv6Addr::from(first_addr.sin6_addr.s6_addr).is_loopback());
    assert!(!first.ai_next.is_null());

    let second = &*first.ai_next;

    assert_eq!(second.ai_family, AF_INET);
    assert_eq!(second.ai_socktype, SOCK_DGRAM);
    assert_eq!(second.ai_protocol, IPPROTO_UDP);

    let second_addr = &*sockaddr_ptr_as_in(second.ai_addr);

    assert_eq!(u16::from_be(second_addr.sin_port), 1234);
    assert!(Ipv4Addr::from(second_addr.sin_addr.s_addr.to_be_bytes()).is_loopback());
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_resolves_localhost_hostname_with_trailing_dot() {
  let host = CString::new("localhost.").expect("host literal must be NUL-free");
  let service = CString::new("8080").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(entry.ai_family, AF_INET);
    assert_eq!(u16::from_be(socket_addr.sin_port), 8080);
    assert!(Ipv4Addr::from(socket_addr.sin_addr.s_addr.to_be_bytes()).is_loopback());
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_rejects_localhost_hostname_when_ai_numerichost_is_set() {
  let host = CString::new("localhost").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_non_numeric_service_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("http").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_resolves_http_service_name_without_ai_numericserv() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("http").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 80);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_resolves_www_service_name_without_ai_numericserv() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("www").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 80);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_resolves_www_http_service_name_without_ai_numericserv() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("www-http").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 80);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_resolves_www_https_service_name_without_ai_numericserv() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("www-https").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 443);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_rejects_http_service_name_with_surrounding_whitespace_without_ai_numericserv() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new(" \thttp\n").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_SERVICE);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_numeric_service_with_surrounding_whitespace_without_ai_numericserv() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new(" \t80\n").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_SERVICE);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_unknown_service_name_without_ai_numericserv_with_eai_service() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("definitely-not-a-service").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_SERVICE);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_non_utf8_service_without_ai_numericserv_with_eai_service() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new(vec![0xff]).expect("service bytes must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_SERVICE);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_non_utf8_service_with_ai_numericserv_with_eai_noname() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new(vec![0xff]).expect("service bytes must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_non_utf8_service_without_ai_numericserv_with_null_node_and_eai_service() {
  let service = CString::new(vec![0xff]).expect("service bytes must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      ptr::null(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_SERVICE);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_non_utf8_service_with_ai_numericserv_with_null_node_and_eai_noname() {
  let service = CString::new(vec![0xff]).expect("service bytes must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      ptr::null(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_empty_service_string_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_accepts_service_with_plus_prefix_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("+80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 80);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_accepts_service_with_leading_space_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new(" 80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 80);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_rejects_service_with_minus_prefix_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("-80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_service_with_whitespace_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new(" 80 ").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_service_with_newline_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("80\n").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_service_with_non_ascii_digits_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("８０").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_accepts_service_with_leading_zeros_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("00080").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 80);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_rejects_overflow_service_with_leading_zeros_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("00065536").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_accepts_zero_service_when_ai_numericserv_is_set() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("0").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 0);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_accepts_null_service_and_sets_zero_port() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      ptr::null(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), 0);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_rejects_when_node_and_service_are_both_null() {
  let hints = empty_hints();
  let mut result = ptr::null_mut();

  // SAFETY: pointers are valid for the duration of this call.
  let status = unsafe { getaddrinfo(ptr::null(), ptr::null(), &raw const hints, &raw mut result) };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_null_node_with_ai_passive_returns_wildcard_addresses_with_service_port() {
  let service = CString::new("9090").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_PASSIVE | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      ptr::null(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(first.ai_socktype, SOCK_STREAM);
    assert_eq!(first.ai_protocol, IPPROTO_TCP);

    let first_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(u16::from_be(first_addr.sin6_port), 9090);
    assert_eq!(
      Ipv6Addr::from(first_addr.sin6_addr.s6_addr),
      Ipv6Addr::UNSPECIFIED
    );
    assert!(!first.ai_next.is_null());

    let second = &*first.ai_next;

    assert_eq!(second.ai_family, AF_INET);
    assert_eq!(second.ai_socktype, SOCK_STREAM);
    assert_eq!(second.ai_protocol, IPPROTO_TCP);

    let second_addr = &*sockaddr_ptr_as_in(second.ai_addr);

    assert_eq!(u16::from_be(second_addr.sin_port), 9090);
    assert_eq!(
      Ipv4Addr::from(second_addr.sin_addr.s_addr.to_be_bytes()),
      Ipv4Addr::UNSPECIFIED,
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_node_with_ai_passive_accepts_zero_service_port() {
  let service = CString::new("0").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_PASSIVE | AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      ptr::null(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;
    let first_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(u16::from_be(first_addr.sin6_port), 0);
    assert_eq!(
      Ipv6Addr::from(first_addr.sin6_addr.s6_addr),
      Ipv6Addr::UNSPECIFIED
    );
    assert!(!first.ai_next.is_null());

    let second = &*first.ai_next;
    let second_addr = &*sockaddr_ptr_as_in(second.ai_addr);

    assert_eq!(second.ai_family, AF_INET);
    assert_eq!(u16::from_be(second_addr.sin_port), 0);
    assert_eq!(
      Ipv4Addr::from(second_addr.sin_addr.s_addr.to_be_bytes()),
      Ipv4Addr::UNSPECIFIED
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_node_without_ai_passive_returns_loopback_addresses_with_service_port() {
  let service = CString::new("7070").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      ptr::null(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(first.ai_socktype, SOCK_STREAM);
    assert_eq!(first.ai_protocol, IPPROTO_TCP);

    let first_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(u16::from_be(first_addr.sin6_port), 7070);
    assert_eq!(
      Ipv6Addr::from(first_addr.sin6_addr.s6_addr),
      Ipv6Addr::LOCALHOST,
    );
    assert!(!first.ai_next.is_null());

    let second = &*first.ai_next;

    assert_eq!(second.ai_family, AF_INET);
    assert_eq!(second.ai_socktype, SOCK_STREAM);
    assert_eq!(second.ai_protocol, IPPROTO_TCP);

    let second_addr = &*sockaddr_ptr_as_in(second.ai_addr);

    assert_eq!(u16::from_be(second_addr.sin_port), 7070);
    assert_eq!(
      Ipv4Addr::from(second_addr.sin_addr.s_addr.to_be_bytes()),
      Ipv4Addr::LOCALHOST,
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_null_node_without_ai_passive_accepts_zero_service_port() {
  let service = CString::new("0").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      ptr::null(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let first = &*result;
    let first_addr = &*sockaddr_ptr_as_in6(first.ai_addr);

    assert_eq!(first.ai_family, AF_INET6);
    assert_eq!(u16::from_be(first_addr.sin6_port), 0);
    assert_eq!(
      Ipv6Addr::from(first_addr.sin6_addr.s6_addr),
      Ipv6Addr::LOCALHOST
    );
    assert!(!first.ai_next.is_null());

    let second = &*first.ai_next;
    let second_addr = &*sockaddr_ptr_as_in(second.ai_addr);

    assert_eq!(second.ai_family, AF_INET);
    assert_eq!(u16::from_be(second_addr.sin_port), 0);
    assert_eq!(
      Ipv4Addr::from(second_addr.sin_addr.s_addr.to_be_bytes()),
      Ipv4Addr::LOCALHOST
    );
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_accepts_u16_max_service_port() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("65535").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, 0);
  assert!(!result.is_null());

  // SAFETY: successful `getaddrinfo` returns a valid linked-list head.
  unsafe {
    let entry = &*result;
    let socket_addr = &*sockaddr_ptr_as_in(entry.ai_addr);

    assert_eq!(u16::from_be(socket_addr.sin_port), u16::MAX);
  }

  // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
  unsafe { freeaddrinfo(result) };
}

#[test]
fn getaddrinfo_rejects_service_port_above_u16_max() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("65536").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_unsupported_address_family_hint() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = 12345;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_FAMILY);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_mismatched_stream_udp_hints() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_UDP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_SERVICE);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_mismatched_dgram_tcp_hints() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_DGRAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_SERVICE);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_rejects_unknown_flag_bits() {
  let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV | 0x8000;
  hints.ai_family = AF_INET;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_BADFLAGS);
  assert!(result.is_null());
}

#[test]
fn freeaddrinfo_accepts_null_and_repeated_allocation_cycles() {
  // SAFETY: null pointer is explicitly allowed by `freeaddrinfo` contract.
  unsafe { freeaddrinfo(ptr::null_mut()) };

  for _ in 0..4 {
    let host = CString::new("127.0.0.1").expect("host literal must be NUL-free");
    let service = CString::new("53").expect("service literal must be NUL-free");
    let mut hints = empty_hints();
    let mut result = ptr::null_mut();

    hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV;
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = IPPROTO_UDP;

    // SAFETY: all pointers are valid for the duration of this call.
    let status = unsafe {
      getaddrinfo(
        host.as_ptr(),
        service.as_ptr(),
        &raw const hints,
        &raw mut result,
      )
    };

    assert_eq!(status, 0);
    assert!(!result.is_null());

    // SAFETY: `result` is owned by this test after successful `getaddrinfo`.
    unsafe { freeaddrinfo(result) };
  }
}

#[test]
fn getaddrinfo_rejects_non_numeric_host_without_ai_numerichost() {
  let host = CString::new("example.invalid").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}

#[test]
fn getaddrinfo_unknown_hostname_returns_error_and_null_result() {
  let host = CString::new("definitely-missing.invalid").expect("host literal must be NUL-free");
  let service = CString::new("80").expect("service literal must be NUL-free");
  let mut hints = empty_hints();
  let mut result = ptr::null_mut();

  hints.ai_flags = AI_NUMERICSERV;
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_protocol = IPPROTO_TCP;

  // SAFETY: all pointers are valid for the duration of this call.
  let status = unsafe {
    getaddrinfo(
      host.as_ptr(),
      service.as_ptr(),
      &raw const hints,
      &raw mut result,
    )
  };

  assert_eq!(status, EAI_NONAME);
  assert!(result.is_null());
}
