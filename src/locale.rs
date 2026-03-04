//! Minimal locale state for `setlocale`.
//!
//! Phase-0 behavior intentionally supports only the `C`/`POSIX` locale.
//! Empty locale selection (`""`) resolves through environment variables, but
//! resolution still accepts only `C` and `POSIX` locale names.

use crate::abi::types::c_int;
use crate::errno::{__errno_location, set_errno};
use core::ffi::{CStr, c_char};
use core::ptr;
use core::sync::atomic::{AtomicU8, Ordering};
use std::env::VarError;

/// Locale category: character classification and case conversion.
///
/// `setlocale` accepts this category in phase 0.
pub const LC_CTYPE: c_int = 0;
/// Locale category: numeric formatting.
///
/// Phase-0 accepts this category and keeps it fixed to `"C"`.
pub const LC_NUMERIC: c_int = 1;
/// Locale category: date and time formatting.
///
/// Phase-0 accepts this category and keeps it fixed to `"C"`.
pub const LC_TIME: c_int = 2;
/// Locale category: collation behavior.
///
/// Phase-0 accepts this category and keeps it fixed to `"C"`.
pub const LC_COLLATE: c_int = 3;
/// Locale category: monetary formatting.
///
/// Phase-0 accepts this category and keeps it fixed to `"C"`.
pub const LC_MONETARY: c_int = 4;
/// Locale category: message localization.
///
/// Phase-0 accepts this category and keeps it fixed to `"C"`.
pub const LC_MESSAGES: c_int = 5;
/// Locale category selector for all categories.
///
/// Phase-0 treats all categories as fixed to the `C` locale baseline.
pub const LC_ALL: c_int = 6;
const LOCALE_C_STATE: u8 = 0;
const C_LOCALE: &[u8] = b"C\0";
const ENV_LC_ALL: &str = "LC_ALL";
const ENV_LANG: &str = "LANG";
const CATEGORY_VARIABLES: [(c_int, &str); 6] = [
  (LC_CTYPE, "LC_CTYPE"),
  (LC_NUMERIC, "LC_NUMERIC"),
  (LC_TIME, "LC_TIME"),
  (LC_COLLATE, "LC_COLLATE"),
  (LC_MONETARY, "LC_MONETARY"),
  (LC_MESSAGES, "LC_MESSAGES"),
];
static CURRENT_LOCALE: AtomicU8 = AtomicU8::new(LOCALE_C_STATE);

#[derive(Clone, Copy, Eq, PartialEq)]
enum EnvironmentLocale {
  NotSet,
  Supported(u8),
  Unsupported,
}

struct ErrnoGuard(c_int);

impl Drop for ErrnoGuard {
  fn drop(&mut self) {
    set_errno(self.0);
  }
}

fn current_errno() -> c_int {
  let errno_ptr = __errno_location();

  debug_assert!(
    !errno_ptr.is_null(),
    "__errno_location must not return null",
  );

  // SAFETY: `__errno_location` returns writable calling-thread TLS errno.
  unsafe { errno_ptr.read() }
}

const fn is_supported_category(category: c_int) -> bool {
  matches!(
    category,
    LC_CTYPE | LC_NUMERIC | LC_TIME | LC_COLLATE | LC_MONETARY | LC_MESSAGES | LC_ALL
  )
}

fn c_locale_ptr() -> *mut c_char {
  let _state = CURRENT_LOCALE.load(Ordering::Relaxed);

  C_LOCALE.as_ptr().cast_mut().cast::<c_char>()
}

fn parse_locale_name(locale_bytes: &[u8]) -> Option<u8> {
  if locale_bytes == b"C" || locale_bytes == b"POSIX" {
    return Some(LOCALE_C_STATE);
  }

  None
}

fn parse_environment_locale(variable: &str) -> EnvironmentLocale {
  match std::env::var(variable) {
    Ok(locale_name) => {
      if locale_name.is_empty() {
        return EnvironmentLocale::NotSet;
      }

      parse_locale_name(locale_name.as_bytes())
        .map_or(EnvironmentLocale::Unsupported, |locale_state| {
          EnvironmentLocale::Supported(locale_state)
        })
    }
    Err(VarError::NotPresent) => EnvironmentLocale::NotSet,
    Err(VarError::NotUnicode(_)) => EnvironmentLocale::Unsupported,
  }
}

const fn resolve_locale_with_fallback(
  preferred_locale: EnvironmentLocale,
  fallback_locale: EnvironmentLocale,
) -> Option<u8> {
  match preferred_locale {
    EnvironmentLocale::Supported(locale_state) => Some(locale_state),
    EnvironmentLocale::Unsupported => None,
    EnvironmentLocale::NotSet => match fallback_locale {
      EnvironmentLocale::Supported(locale_state) => Some(locale_state),
      EnvironmentLocale::Unsupported => None,
      EnvironmentLocale::NotSet => Some(LOCALE_C_STATE),
    },
  }
}

fn category_variable(category: c_int) -> Option<&'static str> {
  CATEGORY_VARIABLES
    .iter()
    .find_map(|&(candidate_category, variable)| {
      (candidate_category == category).then_some(variable)
    })
}

fn resolve_category_locale_from_environment(category: c_int) -> Option<u8> {
  let variable = category_variable(category)?;

  match parse_environment_locale(ENV_LC_ALL) {
    EnvironmentLocale::Supported(locale_state) => Some(locale_state),
    EnvironmentLocale::Unsupported => None,
    EnvironmentLocale::NotSet => resolve_locale_with_fallback(
      parse_environment_locale(variable),
      parse_environment_locale(ENV_LANG),
    ),
  }
}

fn resolve_all_categories_from_environment() -> Option<u8> {
  match parse_environment_locale(ENV_LC_ALL) {
    EnvironmentLocale::Supported(locale_state) => return Some(locale_state),
    EnvironmentLocale::NotSet => {}
    EnvironmentLocale::Unsupported => return None,
  }

  let lang_locale = parse_environment_locale(ENV_LANG);

  for &(_, variable) in &CATEGORY_VARIABLES {
    let category_locale =
      resolve_locale_with_fallback(parse_environment_locale(variable), lang_locale)?;

    if category_locale != LOCALE_C_STATE {
      return None;
    }
  }

  Some(LOCALE_C_STATE)
}

/// C ABI entry point for `setlocale`.
///
/// Contract:
/// - supported categories are [`LC_CTYPE`], [`LC_NUMERIC`], [`LC_TIME`],
///   [`LC_COLLATE`], [`LC_MONETARY`], [`LC_MESSAGES`], and [`LC_ALL`]
/// - query (`locale == NULL`) returns a pointer to the current locale name
/// - `"C"` and `"POSIX"` are accepted and normalized to `"C"`
/// - empty locale string (`""`) resolves from environment variables using
///   `LC_ALL`, category-specific `LC_*`, then `LANG`; when all are unset/empty,
///   it falls back to `"C"`
/// - unsupported categories or locale names return null
/// - unsupported environment locale values (including non-UTF-8) return null
///   and keep the previous locale state unchanged
/// - this function preserves calling-thread `errno` on both success and
///   failure paths
///
/// Output contract:
/// - when non-null, the returned pointer is a stable pointer to static storage
///   containing `"C\\0"`
/// - callers must treat the returned storage as read-only
///
/// # Safety
/// - when non-null, `locale` must point to a readable NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char {
  let _errno_guard = ErrnoGuard(current_errno());

  if !is_supported_category(category) {
    return ptr::null_mut();
  }

  if locale.is_null() {
    return c_locale_ptr();
  }

  // SAFETY: caller provides a valid C string pointer when non-null.
  let locale_bytes = unsafe { CStr::from_ptr(locale).to_bytes() };
  let next_state = if locale_bytes.is_empty() {
    if category == LC_ALL {
      resolve_all_categories_from_environment()
    } else {
      resolve_category_locale_from_environment(category)
    }
  } else {
    parse_locale_name(locale_bytes)
  };
  let Some(locale_state) = next_state else {
    return ptr::null_mut();
  };

  CURRENT_LOCALE.store(locale_state, Ordering::Relaxed);

  c_locale_ptr()
}
