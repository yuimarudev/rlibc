//! C standard library related APIs.
//!
//! This module groups stdlib functionality implemented for the current phase:
//! - allocation (`malloc`, `calloc`, `realloc`, `reallocarray`, `free`, `cfree`, `malloc_usable_size`)
//! - numeric conversion (`strto*`, `atoi*`)
//! - environment access/mutation (`getenv`, `setenv`, ...)
//! - process termination (`atexit`, `exit`, `_Exit`, `abort`)

pub mod alloc;
pub mod atoi;
pub mod conv;
mod env_core;
mod env_mut;
pub mod process;

#[cfg(test)]
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Environment-related namespaces.
pub mod env {
  /// Read-only environment interfaces.
  pub mod core {
    pub use crate::stdlib::env_core::{environ, getenv};
  }

  /// Environment mutation interfaces.
  pub mod mutating {
    pub use crate::stdlib::env_mut::{clearenv, putenv, setenv, unsetenv};
  }
}

pub use alloc::{
  aligned_alloc_impl as aligned_alloc, calloc_impl as calloc, cfree_impl as cfree,
  free_impl as free, malloc_impl as malloc, malloc_usable_size_impl as malloc_usable_size,
  memalign_impl as memalign, posix_memalign_impl as posix_memalign, pvalloc_impl as pvalloc,
  realloc_impl as realloc, reallocarray_impl as reallocarray, valloc_impl as valloc,
};
pub use atoi::{atoi, atol, atoll};
pub use conv::{strtol, strtoll, strtoul, strtoull};
pub use env::core::{environ, getenv};
pub use env::mutating::{clearenv, putenv, setenv, unsetenv};
pub use process::{_Exit, abort, atexit, exit};

#[cfg(test)]
static ENVIRON_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Acquires the shared test-only lock for operations that mutate/read `environ`.
///
/// This lock is used to serialize tests across modules that touch process-global
/// environment state, preventing flaky assertions caused by concurrent updates.
#[cfg(test)]
pub(crate) fn lock_environ_for_test() -> MutexGuard<'static, ()> {
  match ENVIRON_TEST_LOCK.get_or_init(|| Mutex::new(())).lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}
