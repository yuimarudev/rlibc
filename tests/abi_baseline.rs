use core::mem::{align_of, size_of};
use rlibc::abi::errno::{
  E2BIG, EACCES, EADDRINUSE, EADDRNOTAVAIL, EADV, EAFNOSUPPORT, EAGAIN, EALREADY, EBADE, EBADF,
  EBADFD, EBADMSG, EBADR, EBADRQC, EBADSLT, EBFONT, EBUSY, ECANCELED, ECHILD, ECHRNG, ECOMM,
  ECONNABORTED, ECONNREFUSED, ECONNRESET, EDEADLK, EDEADLOCK, EDESTADDRREQ, EDOM, EDOTDOT, EDQUOT,
  EEXIST, EFAULT, EFBIG, EHOSTDOWN, EHOSTUNREACH, EHWPOISON, EIDRM, EILSEQ, EINPROGRESS, EINTR,
  EINVAL, EIO, EISCONN, EISDIR, EISNAM, EKEYEXPIRED, EKEYREJECTED, EKEYREVOKED, EL2HLT, EL2NSYNC,
  EL3HLT, EL3RST, ELIBACC, ELIBBAD, ELIBEXEC, ELIBMAX, ELIBSCN, ELNRNG, ELOOP, EMEDIUMTYPE, EMFILE,
  EMLINK, EMSGSIZE, EMULTIHOP, ENAMETOOLONG, ENAVAIL, ENETDOWN, ENETRESET, ENETUNREACH, ENFILE,
  ENOANO, ENOBUFS, ENOCSI, ENODATA, ENODEV, ENOENT, ENOEXEC, ENOKEY, ENOLCK, ENOLINK, ENOMEDIUM,
  ENOMEM, ENOMSG, ENONET, ENOPKG, ENOPROTOOPT, ENOSPC, ENOSR, ENOSTR, ENOSYS, ENOTBLK, ENOTCONN,
  ENOTDIR, ENOTEMPTY, ENOTNAM, ENOTRECOVERABLE, ENOTSOCK, ENOTSUP, ENOTTY, ENOTUNIQ, ENXIO,
  EOPNOTSUPP, EOVERFLOW, EOWNERDEAD, EPERM, EPFNOSUPPORT, EPIPE, EPROTO, EPROTONOSUPPORT,
  EPROTOTYPE, ERANGE, EREMCHG, EREMOTE, EREMOTEIO, ERESTART, ERFKILL, EROFS, ESHUTDOWN,
  ESOCKTNOSUPPORT, ESPIPE, ESRCH, ESRMNT, ESTALE, ESTRPIPE, ETIME, ETIMEDOUT, ETOOMANYREFS,
  ETXTBSY, EUCLEAN, EUNATCH, EUSERS, EWOULDBLOCK, EXDEV, EXFULL,
};
use rlibc::abi::types::{
  c_char, c_double, c_float, c_int, c_long, c_longlong, c_schar, c_short, c_uchar, c_uint, c_ulong,
  c_ulonglong, c_ushort, c_void, size_t, ssize_t,
};

#[test]
fn c_type_aliases_match_x86_64_linux_baseline() {
  assert_eq!(size_of::<c_char>(), 1);
  assert_eq!(size_of::<c_schar>(), 1);
  assert_eq!(size_of::<c_uchar>(), 1);
  assert_eq!(size_of::<c_short>(), 2);
  assert_eq!(size_of::<c_ushort>(), 2);
  assert_eq!(size_of::<c_int>(), 4);
  assert_eq!(size_of::<c_uint>(), 4);
  assert_eq!(size_of::<c_long>(), 8);
  assert_eq!(size_of::<c_ulong>(), 8);
  assert_eq!(size_of::<c_longlong>(), 8);
  assert_eq!(size_of::<c_ulonglong>(), 8);
  assert_eq!(size_of::<c_float>(), 4);
  assert_eq!(size_of::<c_double>(), 8);
  assert_eq!(align_of::<c_float>(), 4);
  assert_eq!(align_of::<c_double>(), 8);
  assert_eq!(size_of::<size_t>(), size_of::<usize>());
  assert_eq!(size_of::<ssize_t>(), size_of::<isize>());
  assert_eq!(align_of::<size_t>(), align_of::<usize>());
  assert_eq!(align_of::<ssize_t>(), align_of::<isize>());
}

#[test]
fn c_type_aliases_match_ffi_pointer_and_float_contracts() {
  assert_eq!(size_of::<*const c_void>(), size_of::<*const u8>());
  assert_eq!(size_of::<*mut c_void>(), size_of::<*mut u8>());
  assert_eq!(align_of::<*const c_void>(), align_of::<*const u8>());
  assert_eq!(align_of::<*mut c_void>(), align_of::<*mut u8>());
  assert_eq!(size_of::<c_float>(), size_of::<f32>());
  assert_eq!(align_of::<c_float>(), align_of::<f32>());
  assert_eq!(size_of::<c_double>(), size_of::<f64>());
  assert_eq!(align_of::<c_double>(), align_of::<f64>());
}

#[test]
fn c_float_and_c_double_round_trip_as_rust_primitives() {
  let float_value: c_float = 1.25_f32;
  let double_value: c_double = 1.25_f64;
  let float_as_f32: f32 = float_value;
  let double_as_f64: f64 = double_value;

  assert_eq!(float_as_f32.to_bits(), 1.25_f32.to_bits());
  assert_eq!(double_as_f64.to_bits(), 1.25_f64.to_bits());
}

#[test]
fn c_void_pointer_cast_round_trip_preserves_address() {
  let mut value: usize = 0x1234_5678_9abc_def0;
  let value_ptr: *mut usize = &raw mut value;
  let as_void = value_ptr.cast::<c_void>();
  let round_trip = as_void.cast::<usize>();

  assert_eq!(round_trip, value_ptr);
}

#[test]
fn c_void_const_pointer_cast_round_trip_preserves_address() {
  let value: usize = 0x0fed_cba9_8765_4321;
  let value_ptr: *const usize = &raw const value;
  let as_void = value_ptr.cast::<c_void>();
  let round_trip = as_void.cast::<usize>();

  assert_eq!(round_trip, value_ptr);
}

#[test]
fn size_types_follow_lp64_word_contract() {
  assert_eq!(size_of::<size_t>(), size_of::<c_ulong>());
  assert_eq!(align_of::<size_t>(), align_of::<c_ulong>());
  assert_eq!(size_of::<ssize_t>(), size_of::<c_long>());
  assert_eq!(align_of::<ssize_t>(), align_of::<c_long>());

  let unsigned_word_max: size_t = usize::MAX as size_t;
  let signed_word_max: ssize_t = isize::MAX as ssize_t;

  assert_eq!(unsigned_word_max, c_ulong::MAX);
  assert_eq!(signed_word_max, c_long::MAX);
}

#[test]
fn size_types_match_pointer_word_width() {
  assert_eq!(size_of::<size_t>(), size_of::<*const c_void>());
  assert_eq!(align_of::<size_t>(), align_of::<*const c_void>());
  assert_eq!(size_of::<ssize_t>(), size_of::<*const c_void>());
  assert_eq!(align_of::<ssize_t>(), align_of::<*const c_void>());
}

#[test]
fn size_types_round_trip_with_machine_words() {
  let machine_unsigned: usize = 0x1234_5678_9abc_def0;
  let abi_unsigned: size_t = machine_unsigned as size_t;
  let round_trip_unsigned: usize = abi_unsigned as usize;
  let machine_signed: isize = -0x1234_5678;
  let abi_signed: ssize_t = machine_signed as ssize_t;
  let round_trip_signed: isize = abi_signed as isize;

  assert_eq!(round_trip_unsigned, machine_unsigned);
  assert_eq!(round_trip_signed, machine_signed);
}

#[test]
fn c_type_aliases_match_x86_64_linux_signedness_contract() {
  assert!(c_char::MIN < 0 as c_char);
  assert!(c_schar::MIN < 0 as c_schar);
  assert_eq!(c_uchar::MIN, 0 as c_uchar);

  assert!(c_short::MIN < 0 as c_short);
  assert_eq!(c_ushort::MIN, 0 as c_ushort);

  assert!(c_int::MIN < 0 as c_int);
  assert_eq!(c_uint::MIN, 0 as c_uint);

  assert!(c_long::MIN < 0 as c_long);
  assert_eq!(c_ulong::MIN, 0 as c_ulong);

  assert!(c_longlong::MIN < 0 as c_longlong);
  assert_eq!(c_ulonglong::MIN, 0 as c_ulonglong);

  assert_eq!(size_t::MIN, 0 as size_t);
  assert_eq!(size_t::MAX, usize::MAX as size_t);
  assert!(ssize_t::MIN < 0 as ssize_t);
  assert_eq!(ssize_t::MAX, isize::MAX as ssize_t);
}

#[test]
fn errno_constants_match_linux_values() {
  assert_eq!(EINVAL, 22);
  assert_eq!(ENOMEM, 12);
  assert_eq!(ERANGE, 34);
  assert_eq!(EWOULDBLOCK, EAGAIN);
}

#[test]
fn errno_constants_include_low_linux_baseline() {
  assert_eq!(EPERM, 1);
  assert_eq!(ENOENT, 2);
  assert_eq!(ESRCH, 3);
  assert_eq!(EINTR, 4);
  assert_eq!(EIO, 5);
  assert_eq!(ENXIO, 6);
  assert_eq!(E2BIG, 7);
  assert_eq!(ENOEXEC, 8);
  assert_eq!(EBADF, 9);
  assert_eq!(ECHILD, 10);
  assert_eq!(EAGAIN, 11);
  assert_eq!(ENOMEM, 12);
  assert_eq!(EACCES, 13);
  assert_eq!(EFAULT, 14);
  assert_eq!(ENOTBLK, 15);
  assert_eq!(EBUSY, 16);
  assert_eq!(EEXIST, 17);
  assert_eq!(EXDEV, 18);
  assert_eq!(ENODEV, 19);
  assert_eq!(ENOTDIR, 20);
  assert_eq!(EISDIR, 21);
  assert_eq!(EINVAL, 22);
  assert_eq!(ENFILE, 23);
  assert_eq!(EMFILE, 24);
  assert_eq!(ENOTTY, 25);
  assert_eq!(ETXTBSY, 26);
  assert_eq!(EFBIG, 27);
  assert_eq!(ENOSPC, 28);
  assert_eq!(ESPIPE, 29);
  assert_eq!(EROFS, 30);
  assert_eq!(EMLINK, 31);
  assert_eq!(EPIPE, 32);
  assert_eq!(EDOM, 33);
  assert_eq!(ERANGE, 34);
  assert_eq!(EDEADLK, 35);
  assert_eq!(ENAMETOOLONG, 36);
  assert_eq!(ENOLCK, 37);
  assert_eq!(ENOSYS, 38);
  assert_eq!(ENOTEMPTY, 39);
  assert_eq!(ELOOP, 40);
}

#[test]
fn errno_constants_include_extended_linux_baseline() {
  assert_eq!(ENOMSG, 42);
  assert_eq!(EIDRM, 43);
  assert_eq!(ECHRNG, 44);
  assert_eq!(EL2NSYNC, 45);
  assert_eq!(EL3HLT, 46);
  assert_eq!(EL3RST, 47);
  assert_eq!(ELNRNG, 48);
  assert_eq!(EUNATCH, 49);
  assert_eq!(ENOCSI, 50);
  assert_eq!(EL2HLT, 51);
  assert_eq!(EBADE, 52);
  assert_eq!(EBADR, 53);
  assert_eq!(EXFULL, 54);
  assert_eq!(ENOANO, 55);
  assert_eq!(EBADRQC, 56);
  assert_eq!(EBADSLT, 57);
  assert_eq!(EBFONT, 59);
  assert_eq!(ENOSTR, 60);
  assert_eq!(ENODATA, 61);
  assert_eq!(ETIME, 62);
  assert_eq!(ENOSR, 63);
  assert_eq!(ENONET, 64);
  assert_eq!(ENOPKG, 65);
  assert_eq!(EREMOTE, 66);
  assert_eq!(ENOLINK, 67);
  assert_eq!(EADV, 68);
  assert_eq!(ESRMNT, 69);
  assert_eq!(ECOMM, 70);
  assert_eq!(EPROTO, 71);
  assert_eq!(EMULTIHOP, 72);
  assert_eq!(EDOTDOT, 73);
  assert_eq!(EBADMSG, 74);
  assert_eq!(EOVERFLOW, 75);
  assert_eq!(ENOTUNIQ, 76);
  assert_eq!(EBADFD, 77);
  assert_eq!(EREMCHG, 78);
  assert_eq!(ELIBACC, 79);
  assert_eq!(ELIBBAD, 80);
  assert_eq!(ELIBSCN, 81);
  assert_eq!(ELIBMAX, 82);
  assert_eq!(ELIBEXEC, 83);
  assert_eq!(EILSEQ, 84);
  assert_eq!(ERESTART, 85);
  assert_eq!(ESTRPIPE, 86);
  assert_eq!(ETIMEDOUT, 110);
  assert_eq!(ECANCELED, 125);
  assert_eq!(EOWNERDEAD, 130);
  assert_eq!(ENOTRECOVERABLE, 131);
}

#[test]
fn errno_alias_constants_match_linux_contract() {
  assert_eq!(EDEADLK, 35);
  assert_eq!(EDEADLOCK, 35);
  assert_eq!(EDEADLOCK, EDEADLK);
  assert_eq!(EOPNOTSUPP, 95);
  assert_eq!(ENOTSUP, 95);
  assert_eq!(ENOTSUP, EOPNOTSUPP);
}

#[test]
fn errno_constants_include_socket_baseline() {
  assert_eq!(EAFNOSUPPORT, 97);
  assert_eq!(EADDRINUSE, 98);
}

#[test]
fn errno_constants_include_extended_socket_network_baseline() {
  assert_eq!(EDESTADDRREQ, 89);
  assert_eq!(EMSGSIZE, 90);
  assert_eq!(EPROTOTYPE, 91);
  assert_eq!(ENOPROTOOPT, 92);
  assert_eq!(EPROTONOSUPPORT, 93);
  assert_eq!(ESOCKTNOSUPPORT, 94);
  assert_eq!(EADDRNOTAVAIL, 99);
  assert_eq!(ENETDOWN, 100);
  assert_eq!(ENETUNREACH, 101);
  assert_eq!(ENETRESET, 102);
  assert_eq!(ECONNABORTED, 103);
  assert_eq!(ECONNRESET, 104);
  assert_eq!(ENOBUFS, 105);
  assert_eq!(EISCONN, 106);
  assert_eq!(ENOTCONN, 107);
  assert_eq!(ESHUTDOWN, 108);
  assert_eq!(ETOOMANYREFS, 109);
  assert_eq!(EHOSTDOWN, 112);
  assert_eq!(EHOSTUNREACH, 113);
  assert_eq!(EALREADY, 114);
  assert_eq!(EINPROGRESS, 115);
  assert_eq!(ESTALE, 116);
}

#[test]
fn errno_socket_network_band_preserves_linux_step_sequence() {
  assert_eq!(EDESTADDRREQ, ENOTSOCK + 1);
  assert_eq!(EMSGSIZE, EDESTADDRREQ + 1);
  assert_eq!(EPROTOTYPE, EMSGSIZE + 1);
  assert_eq!(ENOPROTOOPT, EPROTOTYPE + 1);
  assert_eq!(EPROTONOSUPPORT, ENOPROTOOPT + 1);
  assert_eq!(ESOCKTNOSUPPORT, EPROTONOSUPPORT + 1);
  assert_eq!(EOPNOTSUPP, ESOCKTNOSUPPORT + 1);
  assert_eq!(EPFNOSUPPORT, EOPNOTSUPP + 1);
  assert_eq!(EAFNOSUPPORT, EPFNOSUPPORT + 1);
  assert_eq!(EADDRINUSE, EAFNOSUPPORT + 1);
  assert_eq!(EADDRNOTAVAIL, EADDRINUSE + 1);
  assert_eq!(ENETDOWN, EADDRNOTAVAIL + 1);
  assert_eq!(ENETUNREACH, ENETDOWN + 1);
  assert_eq!(ENETRESET, ENETUNREACH + 1);
  assert_eq!(ECONNABORTED, ENETRESET + 1);
  assert_eq!(ECONNRESET, ECONNABORTED + 1);
  assert_eq!(ENOBUFS, ECONNRESET + 1);
  assert_eq!(EISCONN, ENOBUFS + 1);
  assert_eq!(ENOTCONN, EISCONN + 1);
  assert_eq!(ESHUTDOWN, ENOTCONN + 1);
  assert_eq!(ETOOMANYREFS, ESHUTDOWN + 1);
  assert_eq!(ETIMEDOUT, ETOOMANYREFS + 1);
  assert_eq!(ECONNREFUSED, ETIMEDOUT + 1);
  assert_eq!(EHOSTDOWN, ECONNREFUSED + 1);
  assert_eq!(EHOSTUNREACH, EHOSTDOWN + 1);
  assert_eq!(EALREADY, EHOSTUNREACH + 1);
  assert_eq!(EINPROGRESS, EALREADY + 1);
  assert_eq!(ESTALE, EINPROGRESS + 1);
}

#[test]
fn errno_constants_include_linux_legacy_tail_band() {
  assert_eq!(EUCLEAN, 117);
  assert_eq!(ENOTNAM, 118);
  assert_eq!(ENAVAIL, 119);
  assert_eq!(EISNAM, 120);
  assert_eq!(EREMOTEIO, 121);
  assert_eq!(EDQUOT, 122);
  assert_eq!(ENOMEDIUM, 123);
  assert_eq!(EMEDIUMTYPE, 124);
}

#[test]
fn errno_linux_legacy_tail_band_preserves_step_sequence() {
  assert_eq!(EUCLEAN, ESTALE + 1);
  assert_eq!(ENOTNAM, EUCLEAN + 1);
  assert_eq!(ENAVAIL, ENOTNAM + 1);
  assert_eq!(EISNAM, ENAVAIL + 1);
  assert_eq!(EREMOTEIO, EISNAM + 1);
  assert_eq!(EDQUOT, EREMOTEIO + 1);
  assert_eq!(ENOMEDIUM, EDQUOT + 1);
  assert_eq!(EMEDIUMTYPE, ENOMEDIUM + 1);
  assert_eq!(ECANCELED, EMEDIUMTYPE + 1);
}

#[test]
fn errno_constants_cover_socket_error_band_edges() {
  assert_eq!(EUSERS, 87);
  assert_eq!(ENOTSOCK, 88);
  assert_eq!(EOPNOTSUPP, 95);
  assert_eq!(EPFNOSUPPORT, 96);
  assert_eq!(ECONNREFUSED, 111);
}

#[test]
fn errno_constants_include_key_and_hw_fault_tail() {
  assert_eq!(ENOKEY, 126);
  assert_eq!(EKEYEXPIRED, 127);
  assert_eq!(EKEYREVOKED, 128);
  assert_eq!(EKEYREJECTED, 129);
  assert_eq!(ERFKILL, 132);
  assert_eq!(EHWPOISON, 133);
}

#[test]
fn errno_key_and_recovery_tail_preserves_linux_step_sequence() {
  assert_eq!(ENOKEY, ECANCELED + 1);
  assert_eq!(EKEYEXPIRED, ENOKEY + 1);
  assert_eq!(EKEYREVOKED, EKEYEXPIRED + 1);
  assert_eq!(EKEYREJECTED, EKEYREVOKED + 1);
  assert_eq!(EOWNERDEAD, EKEYREJECTED + 1);
  assert_eq!(ENOTRECOVERABLE, EOWNERDEAD + 1);
  assert_eq!(ERFKILL, ENOTRECOVERABLE + 1);
  assert_eq!(EHWPOISON, ERFKILL + 1);
}

#[test]
fn errno_linux_reserved_gaps_match_baseline_layout() {
  assert_eq!(ENOMSG, ELOOP + 2);
  assert_eq!(EBFONT, EBADSLT + 2);
  assert_eq!(EDEADLK, ERANGE + 1);
  assert_eq!(ENAMETOOLONG, EDEADLK + 1);
}
