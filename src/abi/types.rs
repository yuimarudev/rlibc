//! C ABI primitive type aliases for `x86_64` Linux.
//!
//! The project baseline is LP64, so:
//! - `c_int`/`c_uint` are 32-bit;
//! - `c_long`/`c_ulong` are 64-bit;
//! - `c_float`/`c_double` follow IEEE-754 single/double precision;
//! - `size_t` and `ssize_t` map to unsigned/signed machine word size.

/// C `double` for the primary target ABI.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
pub use core::ffi::c_double;
/// C `float` for the primary target ABI.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
pub use core::ffi::c_float;
/// C `ssize_t` for the primary target ABI.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
pub use core::ffi::c_long as ssize_t;
/// C `size_t` for the primary target ABI.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
pub use core::ffi::c_ulong as size_t;
/// C primitive types for the primary target ABI.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
pub use core::ffi::{
  c_char, c_int, c_long, c_longlong, c_schar, c_short, c_uchar, c_uint, c_ulong, c_ulonglong,
  c_ushort, c_void,
};

#[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
compile_error!("abi::types currently supports only x86_64 Linux");
