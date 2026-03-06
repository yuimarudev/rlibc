#![doc = "rlibc core library."]
//!
//! This crate hosts libc-compatible building blocks for the primary target:
//! `x86_64-unknown-linux-gnu`.
//!
//! The current implementation includes:
//! - ABI primitive type aliases
//! - thread-local `errno` support
//! - dynamic-loader primitives (`dlopen`, `dlsym`, `dlclose`, `dlerror`)
//! - core memory and string C ABI primitives
//! - directory stream C ABI primitives (`opendir`, `readdir`, `closedir`, `rewinddir`)
//! - file metadata C ABI primitives (`stat` family)
//! - floating-point environment base APIs (`fenv_t`, `fexcept_t`, `fe*`)
//! - minimal pathname expansion (`glob`, `globfree`)
//! - minimal stdio formatting/stream subset (`tmpfile`, `fopen`, `fread`, `fputs`, `fileno`/`fileno_unlocked`, `flockfile` family, `fflush`, `setbuf`/`setbuffer`/`setlinebuf`/`setvbuf`, `vsnprintf`: literals + `%%/%s/%c/%p/%n` + `%d/%i/%u/%x/%X/%o`, integer length modifiers `hh/h/l/ll/j/t/z`)
//! - minimal locale state (`setlocale` for `C`/`POSIX`)
//! - minimal netdb name/service resolution (`getaddrinfo`, `freeaddrinfo`, `gai_strerror`, `getnameinfo`)
//! - non-local jump primitives (`setjmp`, `longjmp`) on `x86_64`
//! - pthread APIs (lifecycle, mutex/condvar, rwlock)
//! - process resource-limit APIs (`getrlimit`, `setrlimit`, `prlimit64`)
//! - signal APIs (`sigset` operations, `sigaction`, `raise`, `kill`, `sigprocmask`)
//! - socket core interfaces (`socket`, `connect`, `bind`, `listen`, `accept`)
//! - math errno/fenv integration primitives (`sqrt`, `log`, `exp`)
//! - stdlib allocation/conversion/environment/process primitives
//! - system-information payload APIs (`uname`, `sysinfo`)
//! - unistd-style interfaces (`access/close/dup/dup2/dup3/getpid/getppid/getpgid/getpgrp/getsid/gettid/getuid/geteuid/getgid/getegid/isatty/lseek/open/openat/pipe/pipe2/fsync/fdatasync/sync/syncfs/read/write/send/recv/unlink/gethostname/getpagesize/sysconf`, plus related `<unistd.h>` constants)
//! - restartable/compatibility multibyte conversion primitives (`mbrtowc`, `mbtowc`, `wcstombs`, ...)
//! - syscall raw return decoding utilities

/// ABI primitive types used by exported C interfaces.
pub mod abi;
/// C locale ASCII classification and case-conversion primitives.
pub mod ctype;
/// Directory stream C ABI functions (`opendir`, `readdir`, `closedir`, `rewinddir`).
pub mod dirent;
/// Dynamic-loader interfaces (`dlopen`, `dlsym`, `dlclose`, `dlerror`).
pub mod dlfcn;
/// Thread-local `errno` storage and exported accessor.
pub mod errno;
/// Minimal `fcntl` interfaces (`F_GETFL`, `F_SETFL`, `F_DUPFD`).
pub mod fcntl;
/// Floating-point environment base APIs (`fenv_t`, `fexcept_t`, `fe*`).
pub mod fenv;
/// File metadata C ABI functions (`stat`, `fstat`, `lstat`, `fstatat`).
pub mod fs;
/// Minimal pathname expansion APIs (`glob`, `globfree`).
pub mod glob;
/// Minimal locale state and `setlocale` for `C`/`POSIX`.
pub mod locale;
/// Math primitives with errno/fenv integration (`sqrt`, `log`, `exp`).
pub mod math;
/// Memory-related C ABI functions (`memmove`, `memcpy`, `memset`, `memcmp`).
pub mod memory;
/// Minimal netdb name/service resolution APIs.
pub mod netdb;
/// Minimal pthread interfaces (thread lifecycle, mutex/condvar, rwlock).
pub mod pthread;
/// Linux process resource-limit APIs (`getrlimit`, `setrlimit`, `prlimit64`).
pub mod resource;
/// Non-local jump primitives (`setjmp`, `longjmp`) for `x86_64`.
pub mod setjmp;
/// Signal APIs (`sigset` operations, `sigaction`, `raise`, `kill`, `sigprocmask`).
pub mod signal;
/// Socket core C ABI interfaces (`socket`, `connect`, `bind`, `listen`, `accept`).
pub mod socket;
/// Startup constructor/destructor array traversal helpers.
pub mod startup;
/// Minimal stdio formatting and stream entry points.
pub mod stdio;
/// C stdlib-related APIs (`strto*`, `atoi*`, env, process termination).
pub mod stdlib;
/// String-related C ABI functions (`strlen`, `strnlen`).
pub mod string;
/// Syscall helper utilities.
pub mod syscall;
/// Linux system-information payload APIs (`uname`, `sysinfo`) and backing
/// implementations for `<unistd.h>` queries re-exported through [`crate::unistd`].
pub mod system;
/// Time-related C ABI interfaces (`clock_gettime`, `gettimeofday`).
pub mod time;
/// Unix file/process I/O C ABI interfaces and `<unistd.h>`-style constants.
///
/// Includes `close`, `dup*`, process identity (`getpid`, `getppid`, `getuid`,
/// `geteuid`, `getgid`, `getegid`), terminal checks (`isatty`),
/// `gethostname`/`getpagesize`/`sysconf`, and open/pipe/sync/read-write/send/recv
/// helpers.
pub mod unistd;
/// Wide/multibyte conversion interfaces and UTF-8 core state helpers.
///
/// This module contains restartable entry points (`mbrtowc`, `mbrlen`,
/// `mbsinit`) and compatibility wrappers (`mblen`, `mbtowc`, `wctomb`,
/// `mbstowcs`, `wcstombs`) for UTF-8 locales.
#[path = "wchar/mod.rs"]
pub mod wchar;

/// Returns the stable project identifier for this crate.
///
/// This value is intentionally version-independent and used by integration
/// tests to assert the crate keeps its expected top-level library identity.
#[must_use]
pub const fn project_name() -> &'static str {
  env!("CARGO_PKG_NAME")
}

/// Returns a NUL-terminated project identifier as raw bytes.
///
/// The returned slice is suitable for C-ABI call sites that require a stable
/// process-lifetime `char*` string with a trailing `\0`. The bytes before the
/// terminator are guaranteed to match [`project_name`].
#[must_use]
pub const fn project_name_cstr_bytes() -> &'static [u8] {
  concat!(env!("CARGO_PKG_NAME"), "\0").as_bytes()
}

/// Returns a stable C-compatible pointer to the project identifier.
///
/// The returned pointer is non-null, points to immutable static storage, and
/// remains valid for the entire process lifetime. It references the same
/// NUL-terminated bytes returned by [`project_name_cstr_bytes`].
#[must_use]
pub const fn project_name_cstr_ptr() -> *const core::ffi::c_char {
  project_name_cstr_bytes().as_ptr().cast()
}

/// Returns the project identifier as a stable C string view.
///
/// This view borrows the same process-lifetime static bytes used by
/// [`project_name_cstr_ptr`] and [`project_name_cstr_bytes`]. The returned
/// [`core::ffi::CStr`] always matches [`project_name`] and is safe to reuse
/// across repeated calls.
#[must_use]
pub const fn project_name_cstr() -> &'static core::ffi::CStr {
  // SAFETY: project_name_cstr_bytes() is defined by appending exactly one
  // trailing NUL byte to CARGO_PKG_NAME at compile time.
  unsafe { core::ffi::CStr::from_bytes_with_nul_unchecked(project_name_cstr_bytes()) }
}

/// Returns the project identifier byte length excluding the trailing NUL.
///
/// This value equals [`project_name`].`len()` and is provided for C-ABI call
/// sites that need payload length alongside [`project_name_cstr_ptr`].
#[must_use]
pub const fn project_name_cstr_len() -> usize {
  project_name().len()
}

#[cfg(test)]
mod tests {
  use super::project_name;
  use std::ffi::CStr;

  #[test]
  fn project_name_matches_cargo_package_name() {
    assert_eq!(
      project_name(),
      env!("CARGO_PKG_NAME"),
      "project_name() should remain aligned with Cargo package metadata"
    );
  }

  #[test]
  fn project_name_c_string_is_nul_terminated() {
    let c_name = super::project_name_cstr_bytes();

    assert_eq!(
      c_name.last(),
      Some(&0),
      "project name C string bytes should be NUL-terminated"
    );
  }

  #[test]
  fn project_name_c_string_prefix_matches_project_name() {
    let c_name = super::project_name_cstr_bytes();
    let name = project_name().as_bytes();

    assert_eq!(
      &c_name[..name.len()],
      name,
      "C string project-name payload should match project_name() bytes"
    );
  }

  #[test]
  fn project_name_c_string_pointer_roundtrips_to_expected_name() {
    let c_name = {
      let ptr = super::project_name_cstr_ptr();

      assert!(
        !ptr.is_null(),
        "project_name_cstr_ptr() should never return a null pointer"
      );
      // SAFETY: project_name_cstr_ptr() is expected to return a process-lifetime
      // pointer to immutable NUL-terminated static bytes.
      unsafe { CStr::from_ptr(ptr) }
    };

    assert_eq!(
      c_name.to_bytes(),
      project_name().as_bytes(),
      "project_name_cstr_ptr() should expose the same payload as project_name()"
    );
  }

  #[test]
  fn project_name_cstr_view_matches_pointer_and_bytes() {
    let c_name = super::project_name_cstr();

    assert_eq!(
      c_name.to_bytes(),
      project_name().as_bytes(),
      "project_name_cstr() payload should match project_name()"
    );
    assert_eq!(
      c_name.as_ptr(),
      super::project_name_cstr_ptr(),
      "project_name_cstr() pointer should match project_name_cstr_ptr()"
    );
  }

  #[test]
  fn project_name_cstr_len_matches_payload_length() {
    assert_eq!(
      super::project_name_cstr_len(),
      project_name().len(),
      "project_name_cstr_len() should match project_name() byte length"
    );
    assert_eq!(
      super::project_name_cstr_bytes().len(),
      super::project_name_cstr_len() + 1,
      "project_name_cstr_bytes() should add exactly one trailing NUL byte"
    );
  }
}
