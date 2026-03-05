//! Minimal `netdb` address-resolution interfaces.
//!
//! This module provides minimal address-resolution interfaces for Linux
//! `x86_64`:
//! - `getaddrinfo`
//! - `freeaddrinfo`
//! - `gai_strerror`
//! - `getnameinfo`
//!
//! Scope notes:
//! - numeric host lookup is handled in-module
//! - non-numeric host lookup uses `/etc/hosts` and localhost fallback
//! - service lookup supports numeric ports and `/etc/services` names
//! - DNS lookup remains scaffold-only and currently returns `EAI_NONAME`

use crate::abi::types::{c_char, c_int, c_uint, c_ushort};
use core::ffi::CStr;
use core::mem::{align_of, size_of};
use core::ptr;
use std::alloc::{Layout, dealloc};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

/// Generic address-family value.
pub const AF_UNSPEC: c_int = 0;
/// IPv4 address family.
pub const AF_INET: c_int = 2;
/// IPv6 address family.
pub const AF_INET6: c_int = 10;
/// Stream socket type.
pub const SOCK_STREAM: c_int = 1;
/// Datagram socket type.
pub const SOCK_DGRAM: c_int = 2;
/// TCP protocol number.
pub const IPPROTO_TCP: c_int = 6;
/// UDP protocol number.
pub const IPPROTO_UDP: c_int = 17;
/// `getaddrinfo` flag: wildcard result for null host.
pub const AI_PASSIVE: c_int = 0x0001;
/// `getaddrinfo` flag: require numeric host literal.
pub const AI_NUMERICHOST: c_int = 0x0004;
/// `getaddrinfo` flag: require numeric service literal.
pub const AI_NUMERICSERV: c_int = 0x0400;
/// `getnameinfo` flag: force numeric host string.
pub const NI_NUMERICHOST: c_int = 0x01;
/// `getnameinfo` flag: force numeric service string.
pub const NI_NUMERICSERV: c_int = 0x02;
/// `getnameinfo` flag: omit domain name for local hosts (accepted as no-op).
pub const NI_NOFQDN: c_int = 0x04;
/// `getnameinfo` flag: require hostname resolution.
pub const NI_NAMEREQD: c_int = 0x08;
/// `getnameinfo` flag: request datagram service semantics.
pub const NI_DGRAM: c_int = 0x10;
/// `getaddrinfo` error: invalid flags.
pub const EAI_BADFLAGS: c_int = -1;
/// `getaddrinfo` error: name/service not known.
pub const EAI_NONAME: c_int = -2;
/// `getaddrinfo`/`getnameinfo` error: temporary failure in name resolution.
pub const EAI_AGAIN: c_int = -3;
/// `getaddrinfo` error: internal failure.
pub const EAI_FAIL: c_int = -4;
/// `getaddrinfo` error: unsupported address family.
pub const EAI_FAMILY: c_int = -6;
/// `getaddrinfo` error: unsupported socket type.
pub const EAI_SOCKTYPE: c_int = -7;
/// `getaddrinfo` error: unsupported service/protocol combination.
pub const EAI_SERVICE: c_int = -8;
/// `getaddrinfo` error: memory-allocation failure.
pub const EAI_MEMORY: c_int = -10;
/// `getaddrinfo` error: generic system error (paired with `errno`).
pub const EAI_SYSTEM: c_int = -11;
/// `getnameinfo` error: output buffer too small.
pub const EAI_OVERFLOW: c_int = -12;
const ALLOWED_AI_FLAGS: c_int = AI_PASSIVE | AI_NUMERICHOST | AI_NUMERICSERV;
const ALLOWED_NI_FLAGS: c_int =
  NI_NUMERICHOST | NI_NUMERICSERV | NI_NOFQDN | NI_NAMEREQD | NI_DGRAM;
const INADDR_ANY_BYTES: [u8; 4] = [0, 0, 0, 0];
const INADDR_LOOPBACK_BYTES: [u8; 4] = [127, 0, 0, 1];
const IN6ADDR_ANY_BYTES: [u8; 16] = [0; 16];
const IN6ADDR_LOOPBACK_BYTES: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
const GAI_BADFLAGS: &[u8] = b"Bad value for ai_flags\0";
const GAI_NONAME: &[u8] = b"Name or service not known\0";
const GAI_AGAIN: &[u8] = b"Temporary failure in name resolution\0";
const GAI_FAMILY: &[u8] = b"Address family not supported\0";
const GAI_SOCKTYPE: &[u8] = b"Socket type not supported\0";
const GAI_SERVICE: &[u8] = b"Service not supported for socket type\0";
const GAI_MEMORY: &[u8] = b"Memory allocation failure\0";
const GAI_SYSTEM: &[u8] = b"System error\0";
const GAI_OVERFLOW: &[u8] = b"Argument buffer overflow\0";
const GAI_FAIL: &[u8] = b"Non-recoverable failure\0";
const GAI_UNKNOWN: &[u8] = b"Unknown error\0";

/// Socket-address length type (`socklen_t`) on Linux `x86_64`.
pub type socklen_t = c_uint;

/// Socket-address family type (`sa_family_t`) on Linux `x86_64`.
pub type sa_family_t = c_ushort;

/// Internet-port type (`in_port_t`) on Linux `x86_64`.
pub type in_port_t = c_ushort;

/// Camel-case alias for [`socklen_t`] used by existing tests.
pub type SockLenT = socklen_t;

/// Camel-case alias for [`sa_family_t`] used by existing tests.
pub type SaFamilyT = sa_family_t;

/// Camel-case alias for [`in_port_t`] used by existing tests.
pub type InPortT = in_port_t;

/// IPv4 address payload for C ABI interoperability.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct in_addr {
  /// IPv4 address in network byte order.
  pub s_addr: u32,
}

/// Camel-case alias for [`in_addr`] used by existing tests.
pub type InAddr = in_addr;

/// IPv6 address payload for C ABI interoperability.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct in6_addr {
  /// IPv6 address bytes in network order.
  pub s6_addr: [u8; 16],
}

/// Camel-case alias for [`in6_addr`] used by existing tests.
pub type In6Addr = in6_addr;

/// Generic socket-address structure.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct sockaddr {
  /// Address family discriminator.
  pub sa_family: sa_family_t,
  /// Family-specific raw address bytes.
  pub sa_data: [c_char; 14],
}

/// Camel-case alias for [`sockaddr`] used by existing tests.
pub type SockAddr = sockaddr;

/// IPv4 socket-address structure.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct sockaddr_in {
  /// Address family (`AF_INET`).
  pub sin_family: sa_family_t,
  /// Port in network byte order.
  pub sin_port: in_port_t,
  /// IPv4 address payload.
  pub sin_addr: in_addr,
  /// ABI padding bytes.
  pub sin_zero: [u8; 8],
}

/// Camel-case alias for [`sockaddr_in`] used by existing tests.
pub type SockAddrIn = sockaddr_in;

/// IPv6 socket-address structure.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct sockaddr_in6 {
  /// Address family (`AF_INET6`).
  pub sin6_family: sa_family_t,
  /// Port in network byte order.
  pub sin6_port: in_port_t,
  /// Flow-label field.
  pub sin6_flowinfo: u32,
  /// IPv6 address payload.
  pub sin6_addr: in6_addr,
  /// Scope ID for scoped addresses.
  pub sin6_scope_id: u32,
}

/// Camel-case alias for [`sockaddr_in6`] used by existing tests.
pub type SockAddrIn6 = sockaddr_in6;

/// Linked-list node returned by `getaddrinfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct addrinfo {
  /// Input/result flags.
  pub ai_flags: c_int,
  /// Address family (`AF_*`).
  pub ai_family: c_int,
  /// Socket type (`SOCK_*`).
  pub ai_socktype: c_int,
  /// Protocol (`IPPROTO_*`).
  pub ai_protocol: c_int,
  /// Byte size of `ai_addr`.
  pub ai_addrlen: socklen_t,
  /// Pointer to family-specific socket-address payload.
  pub ai_addr: *mut sockaddr,
  /// Canonical name pointer (unused in this phase; null).
  pub ai_canonname: *mut c_char,
  /// Next node in result list, or null.
  pub ai_next: *mut Self,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AddressCandidate {
  Ipv4([u8; 4]),
  Ipv6([u8; 16]),
}

const fn resolve_family(family: c_int) -> Result<c_int, c_int> {
  match family {
    AF_UNSPEC | AF_INET | AF_INET6 => Ok(family),
    _ => Err(EAI_FAMILY),
  }
}

const fn resolve_socket_profile(socktype: c_int, protocol: c_int) -> Result<(c_int, c_int), c_int> {
  if !matches!(socktype, 0 | SOCK_STREAM | SOCK_DGRAM) {
    return Err(EAI_SOCKTYPE);
  }

  if !matches!(protocol, 0 | IPPROTO_TCP | IPPROTO_UDP) {
    return Err(EAI_SERVICE);
  }

  match (socktype, protocol) {
    (0 | SOCK_STREAM, 0 | IPPROTO_TCP) => Ok((SOCK_STREAM, IPPROTO_TCP)),
    (0 | SOCK_DGRAM, 0 | IPPROTO_UDP) => Ok((SOCK_DGRAM, IPPROTO_UDP)),
    _ => Err(EAI_SERVICE),
  }
}

fn parse_numeric_service(service: *const c_char) -> Result<u16, c_int> {
  if service.is_null() {
    return Ok(0);
  }

  // SAFETY: caller guarantees `service` is a readable NUL-terminated C string when non-null.
  let raw_bytes = unsafe { CStr::from_ptr(service) }.to_bytes();
  let start = raw_bytes
    .iter()
    .position(|byte| !byte.is_ascii_whitespace())
    .unwrap_or(raw_bytes.len());

  if start == raw_bytes.len() {
    return Err(EAI_NONAME);
  }

  let bytes = &raw_bytes[start..];
  let digits = if bytes.first() == Some(&b'+') {
    &bytes[1..]
  } else {
    bytes
  };

  parse_numeric_service_digits(digits)
}

fn parse_numeric_service_digits(digits: &[u8]) -> Result<u16, c_int> {
  if digits.is_empty() {
    return Err(EAI_NONAME);
  }

  let mut value = 0_u16;

  for byte in digits {
    if !byte.is_ascii_digit() {
      return Err(EAI_NONAME);
    }

    let digit = u16::from(*byte - b'0');

    value = value
      .checked_mul(10)
      .and_then(|scaled| scaled.checked_add(digit))
      .ok_or(EAI_NONAME)?;
  }

  Ok(value)
}

const fn service_protocol_name(protocol: c_int) -> Option<&'static str> {
  match protocol {
    IPPROTO_TCP => Some("tcp"),
    IPPROTO_UDP => Some("udp"),
    _ => None,
  }
}

fn resolve_service_name_from_services_text(
  services_text: &str,
  service_name: &str,
  protocol: c_int,
) -> Option<u16> {
  let protocol_name = service_protocol_name(protocol)?;

  for raw_line in services_text.lines() {
    let line = raw_line.split('#').next().unwrap_or_default().trim();

    if line.is_empty() {
      continue;
    }

    let mut columns = line.split_whitespace();
    let Some(primary_name) = columns.next() else {
      continue;
    };
    let Some(port_and_protocol) = columns.next() else {
      continue;
    };
    let Some((port_text, entry_protocol)) = port_and_protocol.split_once('/') else {
      continue;
    };

    if !entry_protocol.eq_ignore_ascii_case(protocol_name) {
      continue;
    }

    let Ok(port) = port_text.parse::<u16>() else {
      continue;
    };
    let matches_primary = primary_name.eq_ignore_ascii_case(service_name);
    let matches_alias = columns.any(|alias| alias.eq_ignore_ascii_case(service_name));

    if matches_primary || matches_alias {
      return Some(port);
    }
  }

  None
}

fn resolve_service_name_from_services_file(service_name: &str, protocol: c_int) -> Option<u16> {
  let services_file = std::fs::read_to_string("/etc/services").ok()?;

  resolve_service_name_from_services_text(&services_file, service_name, protocol)
}

const fn resolve_service_name_from_builtin(service_name: &str, protocol: c_int) -> Option<u16> {
  match (protocol, service_name) {
    (IPPROTO_TCP, name) if name.eq_ignore_ascii_case("http") => Some(80),
    (IPPROTO_TCP, name) if name.eq_ignore_ascii_case("www") => Some(80),
    (IPPROTO_TCP, name) if name.eq_ignore_ascii_case("www-http") => Some(80),
    (IPPROTO_TCP, name) if name.eq_ignore_ascii_case("https") => Some(443),
    (IPPROTO_TCP, name) if name.eq_ignore_ascii_case("www-https") => Some(443),
    (IPPROTO_TCP, name) if name.eq_ignore_ascii_case("ssh") => Some(22),
    (IPPROTO_TCP | IPPROTO_UDP, name) if name.eq_ignore_ascii_case("domain") => Some(53),
    _ => None,
  }
}

fn resolve_service_name(service_name: &str, protocol: c_int) -> Option<u16> {
  resolve_service_name_from_services_file(service_name, protocol)
    .or_else(|| resolve_service_name_from_builtin(service_name, protocol))
}

fn parse_service_port(service: *const c_char, flags: c_int, protocol: c_int) -> Result<u16, c_int> {
  if service.is_null() {
    return Ok(0);
  }

  if let Ok(port) = parse_numeric_service(service) {
    return Ok(port);
  }

  if (flags & AI_NUMERICSERV) != 0 {
    return Err(EAI_NONAME);
  }

  // SAFETY: caller guarantees `service` is a readable NUL-terminated C string.
  let service_text = unsafe { CStr::from_ptr(service) }
    .to_str()
    .map_err(|_| EAI_NONAME)?;

  resolve_service_name(service_text, protocol).ok_or(EAI_SERVICE)
}

const fn map_getaddrinfo_service_error(
  service: *const c_char,
  flags: c_int,
  error: c_int,
) -> c_int {
  if error != EAI_NONAME {
    return error;
  }

  if service.is_null() || (flags & AI_NUMERICSERV) != 0 {
    return error;
  }

  EAI_SERVICE
}

fn parse_numeric_host_text(node_text: &str) -> Option<AddressCandidate> {
  Ipv4Addr::from_str(node_text)
    .map(|v4| AddressCandidate::Ipv4(v4.octets()))
    .ok()
    .or_else(|| parse_ipv4_legacy_text(node_text).map(AddressCandidate::Ipv4))
    .or_else(|| {
      Ipv6Addr::from_str(node_text)
        .map(|v6| AddressCandidate::Ipv6(v6.octets()))
        .ok()
    })
}

fn parse_ipv4_legacy_component(component: &str) -> Option<u32> {
  if component.is_empty() || component.starts_with('+') {
    return None;
  }

  if let Some(hex_digits) = component
    .strip_prefix("0x")
    .or_else(|| component.strip_prefix("0X"))
  {
    if hex_digits.is_empty() {
      return None;
    }

    return u32::from_str_radix(hex_digits, 16).ok();
  }

  if component.len() > 1 && component.starts_with('0') {
    return u32::from_str_radix(component, 8).ok();
  }

  component.parse::<u32>().ok()
}

fn parse_ipv4_legacy_text(node_text: &str) -> Option<[u8; 4]> {
  let mut components = [0_u32; 4];
  let mut count = 0_usize;

  for component in node_text.split('.') {
    if count >= components.len() {
      return None;
    }

    components[count] = parse_ipv4_legacy_component(component)?;
    count += 1;
  }

  if count == 0 {
    return None;
  }

  let address = match count {
    1 => components[0],
    2 => {
      if components[0] > 0xff || components[1] > 0x00ff_ffff {
        return None;
      }

      (components[0] << 24) | components[1]
    }
    3 => {
      if components[0] > 0xff || components[1] > 0xff || components[2] > 0xffff {
        return None;
      }

      (components[0] << 24) | (components[1] << 16) | components[2]
    }
    4 => {
      if components[0] > 0xff
        || components[1] > 0xff
        || components[2] > 0xff
        || components[3] > 0xff
      {
        return None;
      }

      (components[0] << 24) | (components[1] << 16) | (components[2] << 8) | components[3]
    }
    _ => return None,
  };

  Some(address.to_be_bytes())
}

fn normalize_hostname(name: &str) -> &str {
  name.trim_end_matches('.')
}

fn is_localhost_name(node_text: &str) -> bool {
  normalize_hostname(node_text).eq_ignore_ascii_case("localhost")
}

const fn family_accepts_candidate(family: c_int, candidate: AddressCandidate) -> bool {
  matches!(
    (family, candidate),
    (AF_UNSPEC, _) | (AF_INET, AddressCandidate::Ipv4(_)) | (AF_INET6, AddressCandidate::Ipv6(_))
  )
}

fn push_unique_candidate(
  candidates: &mut Vec<AddressCandidate>,
  family: c_int,
  candidate: AddressCandidate,
) {
  if !family_accepts_candidate(family, candidate) {
    return;
  }

  if !candidates.contains(&candidate) {
    candidates.push(candidate);
  }
}

fn resolve_hosts_file_candidates(node_text: &str, family: c_int) -> Vec<AddressCandidate> {
  let mut candidates = Vec::new();
  let lookup_name = normalize_hostname(node_text);

  if lookup_name.is_empty() {
    return candidates;
  }

  let Ok(hosts_file) = std::fs::read_to_string("/etc/hosts") else {
    return candidates;
  };

  for raw_line in hosts_file.lines() {
    let line = raw_line.split('#').next().unwrap_or_default().trim();

    if line.is_empty() {
      continue;
    }

    let mut columns = line.split_whitespace();
    let Some(address_text) = columns.next() else {
      continue;
    };
    let has_match =
      columns.any(|alias| normalize_hostname(alias).eq_ignore_ascii_case(lookup_name));

    if !has_match {
      continue;
    }

    if let Some(candidate) = parse_numeric_host_text(address_text) {
      push_unique_candidate(&mut candidates, family, candidate);
    }
  }

  candidates
}

fn append_localhost_fallback(
  node_text: &str,
  family: c_int,
  candidates: &mut Vec<AddressCandidate>,
) {
  if !is_localhost_name(node_text) {
    return;
  }

  push_unique_candidate(
    candidates,
    family,
    AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES),
  );
  push_unique_candidate(
    candidates,
    family,
    AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES),
  );
}

fn reorder_localhost_candidates(
  node_text: &str,
  family: c_int,
  candidates: &mut Vec<AddressCandidate>,
) {
  if !is_localhost_name(node_text) || candidates.len() < 2 {
    return;
  }

  let mut reordered = Vec::with_capacity(candidates.len());
  let preferred = match family {
    AF_INET => [Some(AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES)), None],
    AF_INET6 => [Some(AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES)), None],
    AF_UNSPEC => [
      Some(AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES)),
      Some(AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES)),
    ],
    _ => return,
  };

  for candidate in preferred.into_iter().flatten() {
    if candidates.contains(&candidate) {
      reordered.push(candidate);
    }
  }

  for candidate in candidates.iter().copied() {
    if !reordered.contains(&candidate) {
      reordered.push(candidate);
    }
  }

  *candidates = reordered;
}

#[cfg(test)]
fn resolve_dns_candidates_with_lookup<F>(
  node_text: &str,
  family: c_int,
  lookup: F,
) -> Result<Vec<AddressCandidate>, c_int>
where
  F: FnOnce(&str) -> Result<Vec<AddressCandidate>, c_int>,
{
  let mut candidates = Vec::new();
  let lookup_candidates = lookup(node_text)?;

  for candidate in lookup_candidates {
    push_unique_candidate(&mut candidates, family, candidate);
  }

  if candidates.is_empty() {
    return Err(EAI_NONAME);
  }

  Ok(candidates)
}

const fn resolve_dns_candidates(
  node_text: &str,
  family: c_int,
) -> Result<Vec<AddressCandidate>, c_int> {
  let _ = (node_text, family);

  Err(EAI_NONAME)
}

fn resolve_hostname_candidates_with<F>(
  node_text: &str,
  family: c_int,
  mut candidates: Vec<AddressCandidate>,
  resolve_dns: F,
) -> Result<Vec<AddressCandidate>, c_int>
where
  F: FnOnce(&str, c_int) -> Result<Vec<AddressCandidate>, c_int>,
{
  append_localhost_fallback(node_text, family, &mut candidates);
  reorder_localhost_candidates(node_text, family, &mut candidates);

  if !candidates.is_empty() {
    return Ok(candidates);
  }

  resolve_dns(node_text, family)
}

fn resolve_hostname_candidates(
  node_text: &str,
  family: c_int,
) -> Result<Vec<AddressCandidate>, c_int> {
  let hosts_candidates = resolve_hosts_file_candidates(node_text, family);

  resolve_hostname_candidates_with(node_text, family, hosts_candidates, resolve_dns_candidates)
}

fn resolve_candidates(
  node: Option<AddressCandidate>,
  family: c_int,
  flags: c_int,
) -> Result<Vec<AddressCandidate>, c_int> {
  match node {
    Some(AddressCandidate::Ipv4(address)) => match family {
      AF_UNSPEC | AF_INET => Ok(vec![AddressCandidate::Ipv4(address)]),
      _ => Err(EAI_FAMILY),
    },
    Some(AddressCandidate::Ipv6(address)) => match family {
      AF_UNSPEC | AF_INET6 => Ok(vec![AddressCandidate::Ipv6(address)]),
      _ => Err(EAI_FAMILY),
    },
    None => {
      let passive = (flags & AI_PASSIVE) != 0;

      match family {
        AF_INET => {
          let bytes = if passive {
            INADDR_ANY_BYTES
          } else {
            INADDR_LOOPBACK_BYTES
          };

          Ok(vec![AddressCandidate::Ipv4(bytes)])
        }
        AF_INET6 => {
          let bytes = if passive {
            IN6ADDR_ANY_BYTES
          } else {
            IN6ADDR_LOOPBACK_BYTES
          };

          Ok(vec![AddressCandidate::Ipv6(bytes)])
        }
        AF_UNSPEC => {
          let ipv6 = if passive {
            IN6ADDR_ANY_BYTES
          } else {
            IN6ADDR_LOOPBACK_BYTES
          };
          let ipv4 = if passive {
            INADDR_ANY_BYTES
          } else {
            INADDR_LOOPBACK_BYTES
          };

          Ok(vec![
            AddressCandidate::Ipv6(ipv6),
            AddressCandidate::Ipv4(ipv4),
          ])
        }
        _ => Err(EAI_FAMILY),
      }
    }
  }
}

fn resolve_node_candidates(
  node: *const c_char,
  family: c_int,
  flags: c_int,
) -> Result<Vec<AddressCandidate>, c_int> {
  if node.is_null() {
    return resolve_candidates(None, family, flags);
  }

  // SAFETY: caller guarantees `node` is a readable NUL-terminated C string.
  let node_text = unsafe { CStr::from_ptr(node) }
    .to_str()
    .map_err(|_| EAI_NONAME)?;

  if node_text.is_empty() {
    return Err(EAI_NONAME);
  }

  let parsed_node = parse_numeric_host_text(node_text);

  if (flags & AI_NUMERICHOST) != 0 {
    return parsed_node.map_or(Err(EAI_NONAME), |candidate| {
      resolve_candidates(Some(candidate), family, flags)
    });
  }

  parsed_node.map_or_else(
    || resolve_hostname_candidates(node_text, family),
    |candidate| resolve_candidates(Some(candidate), family, flags),
  )
}

fn allocate_ipv4_sockaddr(address: [u8; 4], port: u16) -> (*mut sockaddr, socklen_t) {
  let entry = sockaddr_in {
    sin_family: sa_family_t::try_from(AF_INET)
      .unwrap_or_else(|_| unreachable!("AF_INET must fit sa_family_t")),
    sin_port: port.to_be(),
    sin_addr: in_addr {
      s_addr: u32::from_be_bytes(address),
    },
    sin_zero: [0; 8],
  };
  let raw = Box::into_raw(Box::new(entry)).cast::<sockaddr>();
  let len = socklen_t::try_from(size_of::<sockaddr_in>())
    .unwrap_or_else(|_| unreachable!("sockaddr_in size must fit socklen_t"));

  (raw, len)
}

fn allocate_ipv6_sockaddr(address: [u8; 16], port: u16) -> (*mut sockaddr, socklen_t) {
  let entry = sockaddr_in6 {
    sin6_family: sa_family_t::try_from(AF_INET6)
      .unwrap_or_else(|_| unreachable!("AF_INET6 must fit sa_family_t")),
    sin6_port: port.to_be(),
    sin6_flowinfo: 0,
    sin6_addr: in6_addr { s6_addr: address },
    sin6_scope_id: 0,
  };
  let raw = Box::into_raw(Box::new(entry)).cast::<sockaddr>();
  let len = socklen_t::try_from(size_of::<sockaddr_in6>())
    .unwrap_or_else(|_| unreachable!("sockaddr_in6 size must fit socklen_t"));

  (raw, len)
}

fn allocate_addrinfo_node(
  candidate: AddressCandidate,
  port: u16,
  flags: c_int,
  socktype: c_int,
  protocol: c_int,
) -> *mut addrinfo {
  let (family, ai_addr, ai_addrlen) = match candidate {
    AddressCandidate::Ipv4(address) => {
      let (ai_addr, ai_addrlen) = allocate_ipv4_sockaddr(address, port);

      (AF_INET, ai_addr, ai_addrlen)
    }
    AddressCandidate::Ipv6(address) => {
      let (ai_addr, ai_addrlen) = allocate_ipv6_sockaddr(address, port);

      (AF_INET6, ai_addr, ai_addrlen)
    }
  };
  let node = addrinfo {
    ai_flags: flags,
    ai_family: family,
    ai_socktype: socktype,
    ai_protocol: protocol,
    ai_addrlen,
    ai_addr,
    ai_canonname: ptr::null_mut(),
    ai_next: ptr::null_mut(),
  };

  Box::into_raw(Box::new(node))
}

unsafe fn free_node(node: *mut addrinfo) {
  // SAFETY: caller guarantees `node` points to one `addrinfo` allocated by this module.
  let boxed = unsafe { Box::from_raw(node) };

  if !boxed.ai_addr.is_null() {
    let layout = match boxed.ai_family {
      AF_INET => Layout::from_size_align(size_of::<sockaddr_in>(), align_of::<sockaddr_in>())
        .unwrap_or_else(|_| unreachable!("sockaddr_in layout must be valid")),
      AF_INET6 => Layout::from_size_align(size_of::<sockaddr_in6>(), align_of::<sockaddr_in6>())
        .unwrap_or_else(|_| unreachable!("sockaddr_in6 layout must be valid")),
      _ => return,
    };

    // SAFETY: `ai_addr` was allocated from a `Box<sockaddr_in{,6}>` and
    // therefore can be deallocated with the matching layout.
    unsafe {
      dealloc(boxed.ai_addr.cast::<u8>(), layout);
    }
  }
}

fn socklen_to_usize(length: socklen_t) -> usize {
  usize::try_from(length)
    .unwrap_or_else(|_| unreachable!("socklen_t must fit usize on x86_64 Linux"))
}

fn format_numeric_ipv4(address: in_addr) -> String {
  Ipv4Addr::from(address.s_addr.to_be_bytes()).to_string()
}

fn format_numeric_ipv6(address: in6_addr) -> String {
  Ipv6Addr::from(address.s6_addr).to_string()
}

fn format_numeric_service(port: in_port_t) -> String {
  u16::from_be(port).to_string()
}

fn write_output_string(buffer: *mut c_char, buffer_len: usize, output: &str) -> Result<(), c_int> {
  if buffer.is_null() {
    return Ok(());
  }

  if buffer_len <= output.len() {
    return Err(EAI_OVERFLOW);
  }

  // SAFETY: caller provides writable storage for `buffer_len` bytes and this
  // function verifies `output.len() + 1 <= buffer_len`.
  unsafe {
    ptr::copy_nonoverlapping(output.as_ptr(), buffer.cast::<u8>(), output.len());
    buffer.add(output.len()).write(c_char::default());
  }

  Ok(())
}

/// C ABI entry point for `getaddrinfo`.
///
/// Contract:
/// - Returns `0` on success and writes the result head to `res`.
/// - Returns non-zero `EAI_*` code on failure and sets `*res = NULL`.
/// - If both `node` and `service` are null, returns `EAI_NONAME`.
/// - `hints == NULL` is supported and defaults to `AF_UNSPEC`, no flags, and
///   default stream/TCP profile.
/// - Mismatched `socktype`/`protocol` hint pairs (for example
///   `SOCK_STREAM` + `IPPROTO_UDP`) are rejected with `EAI_SERVICE`.
///
/// Resolution behavior in this phase:
/// - Numeric IPv4/IPv6 `node` literals are handled by this module directly.
/// - IPv4 parsing accepts canonical dotted-quad literals and legacy
///   `inet_aton`-compatible shorthand forms (`a`, `a.b`, `a.b.c`) with
///   decimal/octal/hex components, but rejects explicit sign prefixes.
/// - Empty `node` strings are rejected with `EAI_NONAME`.
/// - Non-numeric `node` values:
///   - return `EAI_NONAME` when `AI_NUMERICHOST` is requested,
///   - otherwise resolve with `/etc/hosts` first.
/// - Hostname matching for `/etc/hosts` and localhost fallback treats trailing
///   dots as equivalent (`"localhost"` == `"localhost."`).
/// - `localhost` always receives explicit loopback fallback candidates.
/// - `localhost` results are normalized to deterministic loopback ordering:
///   IPv6 loopback first, then IPv4 loopback, when both are available.
/// - DNS lookup remains scaffold-only in this phase and currently returns
///   `EAI_NONAME` when no host-file/localhost candidate exists.
/// - Service parsing accepts:
///   - numeric ports (`"0".."65535"`) with optional leading ASCII whitespace
///     and an optional leading `'+'`,
///   - trailing/embedded whitespace is rejected for numeric ports,
///   - when `AI_NUMERICSERV` is not set, service names are matched as-is
///     (surrounding whitespace is not trimmed),
///   - service names from `/etc/services` when `AI_NUMERICSERV` is not set,
///   - if `/etc/services` lookup does not resolve, a small builtin fallback is
///     used for common names/aliases (`http`, `www`, `www-http`, `https`,
///     `www-https`, `ssh`, `domain`).
///   - unresolved service names without `AI_NUMERICSERV` return `EAI_SERVICE`.
/// - `ai_canonname` is always returned as null.
///
/// # Safety
/// - When non-null, `node` and `service` must point to valid NUL-terminated C strings.
/// - When non-null, `hints` must point to a valid `addrinfo`.
/// - `res` must be non-null and writable for one pointer value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getaddrinfo(
  node: *const c_char,
  service: *const c_char,
  hints: *const addrinfo,
  res: *mut *mut addrinfo,
) -> c_int {
  if res.is_null() {
    return EAI_FAIL;
  }

  // SAFETY: `res` is non-null and writable per function contract.
  unsafe {
    res.write(ptr::null_mut());
  }

  if node.is_null() && service.is_null() {
    return EAI_NONAME;
  }

  let (flags, family_hint, socktype_hint, protocol_hint) = if hints.is_null() {
    (0, AF_UNSPEC, 0, 0)
  } else {
    // SAFETY: caller provided a valid `hints` pointer when non-null.
    let hints_ref = unsafe { &*hints };

    (
      hints_ref.ai_flags,
      hints_ref.ai_family,
      hints_ref.ai_socktype,
      hints_ref.ai_protocol,
    )
  };

  if (flags & !ALLOWED_AI_FLAGS) != 0 {
    return EAI_BADFLAGS;
  }

  let family = match resolve_family(family_hint) {
    Ok(value) => value,
    Err(error) => return error,
  };
  let (socktype, protocol) = match resolve_socket_profile(socktype_hint, protocol_hint) {
    Ok(value) => value,
    Err(error) => return error,
  };
  let port = match parse_service_port(service, flags, protocol) {
    Ok(value) => value,
    Err(error) => return map_getaddrinfo_service_error(service, flags, error),
  };
  let candidates = match resolve_node_candidates(node, family, flags) {
    Ok(value) => value,
    Err(error) => return error,
  };
  let mut head: *mut addrinfo = ptr::null_mut();
  let mut tail: *mut addrinfo = ptr::null_mut();

  for candidate in candidates {
    let node_ptr = allocate_addrinfo_node(candidate, port, flags, socktype, protocol);

    if head.is_null() {
      head = node_ptr;
      tail = node_ptr;
      continue;
    }

    // SAFETY: `tail` is non-null and points to a valid list node built above.
    unsafe {
      (*tail).ai_next = node_ptr;
    }
    tail = node_ptr;
  }

  if head.is_null() {
    return EAI_NONAME;
  }

  // SAFETY: `res` is non-null and writable per function contract.
  unsafe {
    res.write(head);
  }

  0
}

const fn should_reject_name_required_without_numeric_host(flags: c_int) -> bool {
  (flags & NI_NAMEREQD) != 0 && (flags & NI_NUMERICHOST) == 0
}

/// C ABI entry point for `getnameinfo` (numeric-only mode).
///
/// Converts a socket address into host and/or service text.
///
/// Contract:
/// - Returns `0` on success.
/// - Returns non-zero `EAI_*` code on failure.
/// - Supports `AF_INET` and `AF_INET6`.
/// - Host/service values are formatted numerically.
/// - Reverse DNS and service-database lookups are out of scope.
///
/// # Safety
/// - `addr` must point to readable socket address storage of at least
///   `addrlen` bytes.
/// - When non-null, `host` must be writable for `hostlen` bytes.
/// - When non-null, `serv` must be writable for `servlen` bytes.
///
/// # Errors
/// - Returns `EAI_BADFLAGS` for unsupported `flags` bits.
/// - Returns `EAI_FAMILY` for unsupported address families or incompatible
///   address lengths.
/// - Returns `EAI_NONAME` when neither `host` nor `serv` output is requested.
/// - Returns `EAI_NONAME` when `NI_NAMEREQD` is requested without reverse
///   lookup support, `host` output is requested, and `NI_NUMERICHOST` is not
///   set.
/// - `NI_NAMEREQD` does not affect service-only requests (`host == NULL`).
/// - `NI_NUMERICHOST` allows numeric host output even when `NI_NAMEREQD` is
///   set.
/// - Returns `EAI_OVERFLOW` when output buffers are too small.
///   In this case, requested output buffers are left unmodified.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getnameinfo(
  addr: *const sockaddr,
  addrlen: socklen_t,
  host: *mut c_char,
  hostlen: socklen_t,
  serv: *mut c_char,
  servlen: socklen_t,
  flags: c_int,
) -> c_int {
  if (flags & !ALLOWED_NI_FLAGS) != 0 {
    return EAI_BADFLAGS;
  }

  let wants_host = !host.is_null();
  let wants_service = !serv.is_null();
  let host_buffer_len = socklen_to_usize(hostlen);
  let service_buffer_len = socklen_to_usize(servlen);
  let address_len = socklen_to_usize(addrlen);

  if !wants_host && !wants_service {
    return EAI_NONAME;
  }

  if wants_host && host_buffer_len == 0 {
    return EAI_OVERFLOW;
  }

  if wants_service && service_buffer_len == 0 {
    return EAI_OVERFLOW;
  }

  if addr.is_null() || address_len < size_of::<sockaddr>() {
    return EAI_FAMILY;
  }

  // SAFETY: `addr` is non-null and points to readable memory for at least one
  // `sockaddr`.
  let family = unsafe { ptr::addr_of!((*addr).sa_family).read_unaligned() };
  let (host_output, service_output) = match c_int::from(family) {
    AF_INET => {
      if address_len < size_of::<sockaddr_in>() {
        return EAI_FAMILY;
      }

      // SAFETY: length check above guarantees `sockaddr_in` is readable.
      let ipv4 = unsafe { ptr::read_unaligned(addr.cast::<sockaddr_in>()) };
      let host_output = if wants_host {
        if should_reject_name_required_without_numeric_host(flags) {
          return EAI_NONAME;
        }

        Some(format_numeric_ipv4(ipv4.sin_addr))
      } else {
        None
      };
      let service_output = if wants_service {
        Some(format_numeric_service(ipv4.sin_port))
      } else {
        None
      };

      (host_output, service_output)
    }
    AF_INET6 => {
      if address_len < size_of::<sockaddr_in6>() {
        return EAI_FAMILY;
      }

      // SAFETY: length check above guarantees `sockaddr_in6` is readable.
      let ipv6 = unsafe { ptr::read_unaligned(addr.cast::<sockaddr_in6>()) };
      let host_output = if wants_host {
        if should_reject_name_required_without_numeric_host(flags) {
          return EAI_NONAME;
        }

        Some(format_numeric_ipv6(ipv6.sin6_addr))
      } else {
        None
      };
      let service_output = if wants_service {
        Some(format_numeric_service(ipv6.sin6_port))
      } else {
        None
      };

      (host_output, service_output)
    }
    _ => return EAI_FAMILY,
  };

  if let Some(host_value) = &host_output
    && host_buffer_len <= host_value.len()
  {
    return EAI_OVERFLOW;
  }

  if let Some(service_value) = &service_output
    && service_buffer_len <= service_value.len()
  {
    return EAI_OVERFLOW;
  }

  if let Some(host_value) = &host_output
    && write_output_string(host, host_buffer_len, host_value).is_err()
  {
    return EAI_OVERFLOW;
  }

  if let Some(service_value) = &service_output
    && write_output_string(serv, service_buffer_len, service_value).is_err()
  {
    return EAI_OVERFLOW;
  }

  0
}

/// C ABI entry point for `freeaddrinfo`.
///
/// Releases a linked list previously returned by [`getaddrinfo`].
///
/// Passing `NULL` is a no-op.
///
/// # Safety
/// - `res` must be null or a pointer returned by this module's `getaddrinfo`.
/// - The list must not be freed more than once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freeaddrinfo(mut res: *mut addrinfo) {
  while !res.is_null() {
    // SAFETY: `res` points to a valid `addrinfo` node from this module.
    let next = unsafe { (*res).ai_next };

    // SAFETY: `res` was allocated by this module and not freed yet.
    unsafe {
      free_node(res);
    }
    res = next;
  }
}

/// C ABI entry point for `gai_strerror`.
///
/// Returns a pointer to a static NUL-terminated error message string for
/// `errcode`. Unknown values map to `"Unknown error"`.
#[must_use]
#[unsafe(no_mangle)]
pub const extern "C" fn gai_strerror(errcode: c_int) -> *const c_char {
  let message = match errcode {
    EAI_BADFLAGS => GAI_BADFLAGS,
    EAI_NONAME => GAI_NONAME,
    EAI_AGAIN => GAI_AGAIN,
    EAI_FAMILY => GAI_FAMILY,
    EAI_SOCKTYPE => GAI_SOCKTYPE,
    EAI_SERVICE => GAI_SERVICE,
    EAI_MEMORY => GAI_MEMORY,
    EAI_SYSTEM => GAI_SYSTEM,
    EAI_OVERFLOW => GAI_OVERFLOW,
    EAI_FAIL => GAI_FAIL,
    _ => GAI_UNKNOWN,
  };

  message.as_ptr().cast::<c_char>()
}

#[cfg(test)]
mod tests {
  use super::{
    AF_INET, AF_INET6, AF_UNSPEC, AI_NUMERICHOST, AI_NUMERICSERV, AddressCandidate, EAI_NONAME,
    EAI_OVERFLOW, EAI_SERVICE, IN6ADDR_LOOPBACK_BYTES, INADDR_LOOPBACK_BYTES, IPPROTO_TCP,
    IPPROTO_UDP, NI_NAMEREQD, NI_NUMERICHOST, getnameinfo, in_addr, in6_addr,
    parse_numeric_host_text, parse_numeric_service, parse_service_port,
    resolve_dns_candidates_with_lookup, resolve_hostname_candidates_with, resolve_node_candidates,
    resolve_service_name_from_builtin, resolve_service_name_from_services_text,
    resolve_socket_profile, sockaddr, sockaddr_in, sockaddr_in6, socklen_t,
  };
  use std::ffi::{CStr, CString};

  fn slen(value: usize) -> socklen_t {
    socklen_t::try_from(value)
      .unwrap_or_else(|_| unreachable!("usize does not fit into socklen_t on this target"))
  }

  #[test]
  fn parse_numeric_service_accepts_u16_max() {
    let service = CString::new("65535").expect("literal must be NUL-free");
    let parsed =
      parse_numeric_service(service.as_ptr()).expect("u16::MAX service should parse successfully");

    assert_eq!(parsed, u16::MAX);
  }

  #[test]
  fn parse_numeric_service_rejects_overflow() {
    let service = CString::new("65536").expect("literal must be NUL-free");
    let error =
      parse_numeric_service(service.as_ptr()).expect_err("overflowing service must be rejected");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn parse_numeric_service_accepts_plus_prefixed_values() {
    let service = CString::new("+80").expect("literal must be NUL-free");
    let parsed =
      parse_numeric_service(service.as_ptr()).expect("plus-prefixed numeric service should parse");

    assert_eq!(parsed, 80);
  }

  #[test]
  fn parse_numeric_service_accepts_leading_ascii_whitespace() {
    let service = CString::new("\t 80").expect("literal must be NUL-free");
    let parsed = parse_numeric_service(service.as_ptr())
      .expect("leading-whitespace numeric service should parse");

    assert_eq!(parsed, 80);
  }

  #[test]
  fn parse_service_port_resolves_http_without_ai_numericserv() {
    let service = CString::new("http").expect("literal must be NUL-free");
    let parsed = parse_service_port(service.as_ptr(), 0, IPPROTO_TCP)
      .expect("named service should resolve when AI_NUMERICSERV is not set");

    assert_eq!(parsed, 80);
  }

  #[test]
  fn parse_service_port_rejects_http_with_ai_numericserv() {
    let service = CString::new("http").expect("literal must be NUL-free");
    let error = parse_service_port(service.as_ptr(), AI_NUMERICSERV, IPPROTO_TCP)
      .expect_err("named service should be rejected when AI_NUMERICSERV is set");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn parse_service_port_rejects_http_with_surrounding_ascii_whitespace() {
    let service = CString::new(" \thttp\n").expect("literal must be NUL-free");
    let error = parse_service_port(service.as_ptr(), 0, IPPROTO_TCP)
      .expect_err("named service with surrounding whitespace should be rejected");

    assert_eq!(error, EAI_SERVICE);
  }

  #[test]
  fn parse_service_port_rejects_numeric_with_surrounding_ascii_whitespace_without_ai_numericserv() {
    let service = CString::new(" \t80\n").expect("literal must be NUL-free");
    let error = parse_service_port(service.as_ptr(), 0, IPPROTO_TCP)
      .expect_err("numeric service with trailing whitespace should be rejected");

    assert_eq!(error, EAI_SERVICE);
  }

  #[test]
  fn resolve_service_name_from_services_text_skips_malformed_lines() {
    let services_text = "broken_line\nhttp 80/tcp www\n";
    let resolved = resolve_service_name_from_services_text(services_text, "http", IPPROTO_TCP);

    assert_eq!(resolved, Some(80));
  }

  #[test]
  fn resolve_service_name_from_services_text_matches_aliases() {
    let services_text = "www-http 80/tcp http www\n";
    let resolved = resolve_service_name_from_services_text(services_text, "http", IPPROTO_TCP);

    assert_eq!(resolved, Some(80));
  }

  #[test]
  fn resolve_service_name_from_builtin_resolves_known_tcp_services() {
    assert_eq!(
      resolve_service_name_from_builtin("http", IPPROTO_TCP),
      Some(80)
    );
    assert_eq!(
      resolve_service_name_from_builtin("www", IPPROTO_TCP),
      Some(80)
    );
    assert_eq!(
      resolve_service_name_from_builtin("https", IPPROTO_TCP),
      Some(443)
    );
    assert_eq!(
      resolve_service_name_from_builtin("www-http", IPPROTO_TCP),
      Some(80)
    );
    assert_eq!(
      resolve_service_name_from_builtin("www-https", IPPROTO_TCP),
      Some(443)
    );
    assert_eq!(
      resolve_service_name_from_builtin("ssh", IPPROTO_TCP),
      Some(22)
    );
  }

  #[test]
  fn resolve_service_name_from_builtin_resolves_domain_for_tcp_and_udp() {
    assert_eq!(
      resolve_service_name_from_builtin("domain", IPPROTO_TCP),
      Some(53)
    );
    assert_eq!(
      resolve_service_name_from_builtin("domain", IPPROTO_UDP),
      Some(53)
    );
  }

  #[test]
  fn resolve_service_name_from_builtin_matches_domain_case_insensitively() {
    assert_eq!(
      resolve_service_name_from_builtin("DoMaIn", IPPROTO_TCP),
      Some(53)
    );
    assert_eq!(
      resolve_service_name_from_builtin("DoMaIn", IPPROTO_UDP),
      Some(53)
    );
  }

  #[test]
  fn resolve_service_name_from_builtin_rejects_unknown_or_mismatched_protocol() {
    assert_eq!(resolve_service_name_from_builtin("smtp", IPPROTO_TCP), None);
    assert_eq!(resolve_service_name_from_builtin("http", IPPROTO_UDP), None);
  }

  #[test]
  fn parse_numeric_host_text_parses_ipv4_and_ipv6_literals() {
    let parsed_ipv4 =
      parse_numeric_host_text("127.0.0.1").expect("valid IPv4 literal should parse");
    let parsed_ipv6 = parse_numeric_host_text("::1").expect("valid IPv6 literal should parse");

    assert_eq!(parsed_ipv4, AddressCandidate::Ipv4([127, 0, 0, 1]));
    assert_eq!(parsed_ipv6, AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES));
  }

  #[test]
  fn parse_numeric_host_text_parses_ipv4_legacy_literals() {
    let shorthand_two_component =
      parse_numeric_host_text("127.1").expect("two-component IPv4 shorthand should parse");
    let shorthand_three_component =
      parse_numeric_host_text("127.0.1").expect("three-component IPv4 shorthand should parse");
    let shorthand_single_component =
      parse_numeric_host_text("127").expect("single-component IPv4 shorthand should parse");
    let octal_component =
      parse_numeric_host_text("0177.1").expect("octal IPv4 shorthand should parse");
    let hexadecimal_component =
      parse_numeric_host_text("0x7f.1").expect("hexadecimal IPv4 shorthand should parse");

    assert_eq!(
      shorthand_two_component,
      AddressCandidate::Ipv4([127, 0, 0, 1])
    );
    assert_eq!(
      shorthand_three_component,
      AddressCandidate::Ipv4([127, 0, 0, 1])
    );
    assert_eq!(
      shorthand_single_component,
      AddressCandidate::Ipv4([0, 0, 0, 127])
    );
    assert_eq!(octal_component, AddressCandidate::Ipv4([127, 0, 0, 1]));
    assert_eq!(
      hexadecimal_component,
      AddressCandidate::Ipv4([127, 0, 0, 1])
    );
  }

  #[test]
  fn parse_numeric_host_text_rejects_non_numeric_hostname() {
    assert_eq!(parse_numeric_host_text("localhost"), None);
    assert_eq!(parse_numeric_host_text("example.invalid"), None);
    assert_eq!(parse_numeric_host_text("fe80::1%eth0"), None);
    assert_eq!(parse_numeric_host_text("+127.1"), None);
  }

  #[test]
  fn resolve_hostname_candidates_prefers_existing_hosts_entries() {
    let mut dns_called = false;
    let result = resolve_hostname_candidates_with(
      "example.test",
      AF_INET,
      vec![AddressCandidate::Ipv4([192, 0, 2, 7])],
      |_lookup_name, _lookup_family| {
        dns_called = true;
        Err(EAI_NONAME)
      },
    )
    .expect("hosts candidates should be returned without DNS");

    assert_eq!(result, vec![AddressCandidate::Ipv4([192, 0, 2, 7])]);
    assert!(!dns_called);
  }

  #[test]
  fn resolve_hostname_candidates_adds_localhost_loopback_fallback() {
    let result = resolve_hostname_candidates_with(
      "localhost",
      AF_UNSPEC,
      Vec::new(),
      |_lookup_name, _lookup_family| -> Result<Vec<AddressCandidate>, i32> {
        panic!("localhost fallback should short-circuit before DNS lookup");
      },
    )
    .expect("localhost fallback should provide candidates");

    assert_eq!(
      result,
      vec![
        AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES),
        AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES),
      ],
    );
  }

  #[test]
  fn resolve_hostname_candidates_reorders_localhost_to_prefer_ipv6_then_ipv4() {
    let result = resolve_hostname_candidates_with(
      "localhost",
      AF_UNSPEC,
      vec![
        AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES),
        AddressCandidate::Ipv4([127, 0, 1, 1]),
        AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES),
      ],
      |_lookup_name, _lookup_family| -> Result<Vec<AddressCandidate>, i32> {
        panic!("localhost candidates should short-circuit before DNS lookup");
      },
    )
    .expect("localhost candidates should be reordered");

    assert_eq!(
      result,
      vec![
        AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES),
        AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES),
        AddressCandidate::Ipv4([127, 0, 1, 1]),
      ],
    );
  }

  #[test]
  fn resolve_hostname_candidates_reorders_localhost_with_trailing_dot_and_case() {
    let result = resolve_hostname_candidates_with(
      "LOCALHOST.",
      AF_UNSPEC,
      vec![
        AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES),
        AddressCandidate::Ipv4([127, 0, 1, 1]),
        AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES),
      ],
      |_lookup_name, _lookup_family| -> Result<Vec<AddressCandidate>, i32> {
        panic!("localhost candidates should short-circuit before DNS lookup");
      },
    )
    .expect("localhost candidates should be reordered");

    assert_eq!(
      result,
      vec![
        AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES),
        AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES),
        AddressCandidate::Ipv4([127, 0, 1, 1]),
      ],
    );
  }

  #[test]
  fn resolve_dns_candidates_with_lookup_filters_by_requested_family() {
    let candidates = resolve_dns_candidates_with_lookup("dual.example", AF_INET6, |_node_text| {
      Ok(vec![
        AddressCandidate::Ipv4([203, 0, 113, 4]),
        AddressCandidate::Ipv6([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
      ])
    })
    .expect("family-filtered DNS candidates should be returned");

    assert_eq!(
      candidates,
      vec![AddressCandidate::Ipv6([
        0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
      ])],
    );
  }

  #[test]
  fn resolve_node_candidates_rejects_non_numeric_host_when_numerichost_is_set() {
    let node = CString::new("localhost").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST must reject non-numeric hostnames");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn getnameinfo_allows_numerichost_with_name_required() {
    let address = sockaddr_in {
      sin_family: u16::try_from(AF_INET)
        .unwrap_or_else(|_| unreachable!("AF_INET must fit sockaddr_in::sin_family")),
      sin_port: 80_u16.to_be(),
      sin_addr: in_addr {
        s_addr: u32::from_be_bytes([127, 0, 0, 1]),
      },
      sin_zero: [0; 8],
    };
    let mut host = [0_i8; 64];

    // SAFETY: pointers are valid and writable for the declared lengths.
    let status = unsafe {
      getnameinfo(
        (&raw const address).cast::<sockaddr>(),
        slen(core::mem::size_of::<sockaddr_in>()),
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
  fn getnameinfo_allows_numerichost_with_name_required_for_ipv6() {
    let address = sockaddr_in6 {
      sin6_family: u16::try_from(AF_INET6)
        .unwrap_or_else(|_| unreachable!("AF_INET6 must fit sockaddr_in6::sin6_family")),
      sin6_port: 443_u16.to_be(),
      sin6_flowinfo: 0,
      sin6_addr: in6_addr {
        s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
      },
      sin6_scope_id: 0,
    };
    let mut host = [0_i8; 96];

    // SAFETY: pointers are valid and writable for the declared lengths.
    let status = unsafe {
      getnameinfo(
        (&raw const address).cast::<sockaddr>(),
        slen(core::mem::size_of::<sockaddr_in6>()),
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
  fn getnameinfo_allows_numerichost_with_name_required_and_service_output_for_ipv6() {
    let address = sockaddr_in6 {
      sin6_family: u16::try_from(AF_INET6)
        .unwrap_or_else(|_| unreachable!("AF_INET6 must fit sockaddr_in6::sin6_family")),
      sin6_port: 443_u16.to_be(),
      sin6_flowinfo: 0,
      sin6_addr: in6_addr {
        s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
      },
      sin6_scope_id: 0,
    };
    let mut host = [0_i8; 96];
    let mut service = [0_i8; 32];

    // SAFETY: pointers are valid and writable for the declared lengths.
    let status = unsafe {
      getnameinfo(
        (&raw const address).cast::<sockaddr>(),
        slen(core::mem::size_of::<sockaddr_in6>()),
        host.as_mut_ptr(),
        slen(host.len()),
        service.as_mut_ptr(),
        slen(service.len()),
        NI_NAMEREQD | NI_NUMERICHOST,
      )
    };

    assert_eq!(status, 0);

    // SAFETY: successful getnameinfo writes a terminating NUL to host.
    let host_text = unsafe { CStr::from_ptr(host.as_ptr()) };
    // SAFETY: successful getnameinfo writes a terminating NUL to service.
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
  fn getnameinfo_name_required_with_numerichost_and_small_host_buffer_returns_overflow() {
    let address = sockaddr_in {
      sin_family: u16::try_from(AF_INET)
        .unwrap_or_else(|_| unreachable!("AF_INET must fit sockaddr_in::sin_family")),
      sin_port: 80_u16.to_be(),
      sin_addr: in_addr {
        s_addr: u32::from_be_bytes([127, 0, 0, 1]),
      },
      sin_zero: [0; 8],
    };
    let mut host = [0x58_i8; 2];

    // SAFETY: pointers are valid and writable for the declared lengths.
    let status = unsafe {
      getnameinfo(
        (&raw const address).cast::<sockaddr>(),
        slen(core::mem::size_of::<sockaddr_in>()),
        host.as_mut_ptr(),
        slen(host.len()),
        core::ptr::null_mut(),
        slen(0),
        NI_NAMEREQD | NI_NUMERICHOST,
      )
    };

    assert_eq!(status, EAI_OVERFLOW);
    assert!(
      host.iter().all(|&byte| byte == 0x58_i8),
      "host output must remain untouched on EAI_OVERFLOW",
    );
  }

  #[test]
  fn resolve_node_candidates_rejects_ipv6_numeric_host_with_ipv4_family_when_numerichost_is_set() {
    let node = CString::new("::1").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_INET, AI_NUMERICHOST)
      .expect_err("IPv6 numeric host should be rejected when AF_INET is requested");

    assert_eq!(error, super::EAI_FAMILY);
  }

  #[test]
  fn resolve_node_candidates_rejects_ipv4_numeric_host_with_ipv6_family_when_numerichost_is_set() {
    let node = CString::new("127.0.0.1").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_INET6, AI_NUMERICHOST)
      .expect_err("IPv4 numeric host should be rejected when AF_INET6 is requested");

    assert_eq!(error, super::EAI_FAMILY);
  }

  #[test]
  fn resolve_node_candidates_rejects_whitespace_wrapped_numeric_host_when_numerichost_is_set() {
    let node = CString::new(" 127.0.0.1 ").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject host literals with surrounding whitespace");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_newline_terminated_numeric_host_when_numerichost_is_set() {
    let node = CString::new("127.0.0.1\n").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject host literals with trailing newlines");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_tab_terminated_numeric_host_when_numerichost_is_set() {
    let node = CString::new("127.0.0.1\t").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject host literals with trailing tabs");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_scoped_ipv6_host_when_numerichost_is_set() {
    let node = CString::new("fe80::1%eth0").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject scoped IPv6 host text in this phase");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_ipv4_literal_with_trailing_dot_when_numerichost_is_set() {
    let node = CString::new("127.0.0.1.").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject trailing-dot IPv4 host text");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_ipv6_literal_with_trailing_dot_when_numerichost_is_set() {
    let node = CString::new("::1.").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject trailing-dot IPv6 host text");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_bracketed_ipv6_host_when_numerichost_is_set() {
    let node = CString::new("[::1]").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject bracketed IPv6 host text");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_bracketed_scoped_ipv6_host_when_numerichost_is_set() {
    let node = CString::new("[fe80::1%eth0]").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject bracketed scoped IPv6 host text");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_bracketed_ipv4_host_when_numerichost_is_set() {
    let node = CString::new("[127.0.0.1]").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject bracketed IPv4 host text");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_bracketed_ipv4_with_port_when_numerichost_is_set() {
    let node = CString::new("[127.0.0.1]:80").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject bracketed IPv4 host text with port suffix");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_bracketed_ipv6_with_port_when_numerichost_is_set() {
    let node = CString::new("[::1]:80").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject bracketed IPv6 host text with port suffix");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_bracketed_scoped_ipv6_with_port_when_numerichost_is_set() {
    let node = CString::new("[fe80::1%eth0]:80").expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, AI_NUMERICHOST)
      .expect_err("AI_NUMERICHOST should reject bracketed scoped IPv6 host text with port suffix");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_non_utf8_hostnames() {
    let node = CString::new(vec![0xff]).expect("literal must be NUL-free");
    let error = resolve_node_candidates(node.as_ptr(), AF_UNSPEC, 0)
      .expect_err("non-UTF8 hostnames must be rejected");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_node_candidates_rejects_empty_hostname() {
    let node = CString::new("").expect("literal must be NUL-free");
    let error =
      resolve_node_candidates(node.as_ptr(), AF_UNSPEC, 0).expect_err("empty hostnames must fail");

    assert_eq!(error, EAI_NONAME);
  }

  #[test]
  fn resolve_socket_profile_rejects_mismatched_socktype_protocol_with_eai_service() {
    let mismatch_stream_udp = resolve_socket_profile(super::SOCK_STREAM, super::IPPROTO_UDP)
      .expect_err("SOCK_STREAM + UDP must be rejected as invalid service/protocol combination");
    let mismatch_dgram_tcp = resolve_socket_profile(super::SOCK_DGRAM, super::IPPROTO_TCP)
      .expect_err("SOCK_DGRAM + TCP must be rejected as invalid service/protocol combination");

    assert_eq!(mismatch_stream_udp, EAI_SERVICE);
    assert_eq!(mismatch_dgram_tcp, EAI_SERVICE);
  }

  #[test]
  fn resolve_node_candidates_resolves_localhost_without_numerichost() {
    let node = CString::new("localhost").expect("literal must be NUL-free");
    let candidates = resolve_node_candidates(node.as_ptr(), AF_INET, 0)
      .expect("localhost should resolve via hosts/fallback path");

    assert!(!candidates.is_empty());
    assert!(
      candidates
        .iter()
        .all(|candidate| matches!(candidate, AddressCandidate::Ipv4(_)))
    );
    assert!(
      candidates.contains(&AddressCandidate::Ipv4(INADDR_LOOPBACK_BYTES)),
      "localhost resolution should include 127.0.0.1 fallback",
    );
  }

  #[test]
  fn resolve_node_candidates_resolves_localhost_with_trailing_dot() {
    let node = CString::new("localhost.").expect("literal must be NUL-free");
    let candidates = resolve_node_candidates(node.as_ptr(), AF_INET6, 0)
      .expect("localhost with trailing dot should resolve via fallback path");

    assert!(!candidates.is_empty());
    assert!(
      candidates
        .iter()
        .all(|candidate| matches!(candidate, AddressCandidate::Ipv6(_)))
    );
    assert!(
      candidates.contains(&AddressCandidate::Ipv6(IN6ADDR_LOOPBACK_BYTES)),
      "localhost. resolution should include ::1 fallback",
    );
  }
}
