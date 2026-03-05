use core::ffi::{CStr, c_char, c_int};
use core::ptr;
use rlibc::errno::__errno_location;
use rlibc::locale::{
  LC_ALL, LC_COLLATE, LC_CTYPE, LC_MESSAGES, LC_MONETARY, LC_NUMERIC, LC_TIME, setlocale,
};
use std::env;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::sync::{Mutex, MutexGuard, OnceLock};

const INVALID_CATEGORY: c_int = -1;
const UNSUPPORTED_CATEGORY: c_int = 99;
const LOCALE_ENV_KEYS: [&str; 8] = [
  "LC_ALL",
  "LC_CTYPE",
  "LC_NUMERIC",
  "LC_TIME",
  "LC_COLLATE",
  "LC_MONETARY",
  "LC_MESSAGES",
  "LANG",
];
const SUPPORTED_CATEGORIES: [c_int; 7] = [
  LC_CTYPE,
  LC_NUMERIC,
  LC_TIME,
  LC_COLLATE,
  LC_MONETARY,
  LC_MESSAGES,
  LC_ALL,
];
const CATEGORY_VARIABLES: [(c_int, &str); 6] = [
  (LC_CTYPE, "LC_CTYPE"),
  (LC_NUMERIC, "LC_NUMERIC"),
  (LC_TIME, "LC_TIME"),
  (LC_COLLATE, "LC_COLLATE"),
  (LC_MONETARY, "LC_MONETARY"),
  (LC_MESSAGES, "LC_MESSAGES"),
];
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvironmentSnapshot {
  entries: Vec<(&'static str, Option<OsString>)>,
}

impl EnvironmentSnapshot {
  fn capture(keys: &[&'static str]) -> Self {
    let entries = keys
      .iter()
      .map(|&key| (key, env::var_os(key)))
      .collect::<Vec<_>>();

    Self { entries }
  }
}

impl Drop for EnvironmentSnapshot {
  fn drop(&mut self) {
    for &(key, ref value) in &self.entries {
      // SAFETY: all tests mutate environment only while holding `ENV_LOCK`,
      // preventing concurrent test-driven mutation races in this process.
      unsafe {
        match value {
          Some(saved) => env::set_var(key, saved),
          None => env::remove_var(key),
        }
      }
    }
  }
}

const fn as_c_ptr(bytes: &[u8]) -> *const c_char {
  bytes.as_ptr().cast::<c_char>()
}

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns writable TLS errno storage for the
  // current thread. Reading a single `c_int` is valid.
  unsafe { __errno_location().read() }
}

fn locale_env_lock() -> &'static Mutex<()> {
  ENV_LOCK.get_or_init(|| Mutex::new(()))
}

fn locale_name(locale_ptr: *mut c_char) -> Vec<u8> {
  // SAFETY: callers only pass pointers returned by `setlocale` for non-null results.
  unsafe { CStr::from_ptr(locale_ptr.cast_const()) }
    .to_bytes()
    .to_vec()
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable TLS errno storage for the
  // current thread. Writing a single `c_int` is valid.
  unsafe {
    __errno_location().write(value);
  }
}

fn lock_locale_environment() -> MutexGuard<'static, ()> {
  match locale_env_lock().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn clear_locale_environment() {
  for &key in &LOCALE_ENV_KEYS {
    // SAFETY: all tests mutate environment only while holding `ENV_LOCK`.
    unsafe {
      env::remove_var(key);
    }
  }
}

fn set_locale_environment(key: &str, value: &str) {
  // SAFETY: all tests mutate environment only while holding `ENV_LOCK`.
  unsafe {
    env::set_var(key, value);
  }
}

fn set_locale_environment_raw_bytes(key: &str, bytes: &[u8]) {
  // SAFETY: all tests mutate environment only while holding `ENV_LOCK`.
  unsafe {
    env::set_var(key, OsString::from_vec(bytes.to_vec()));
  }
}

#[test]
fn setlocale_accepts_c_for_lc_all() {
  // SAFETY: argument points to a valid NUL-terminated locale string.
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_accepts_posix_alias_and_normalizes_to_c() {
  // SAFETY: argument points to a valid NUL-terminated locale string.
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"POSIX\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_null_locale_queries_current_category_locale() {
  // SAFETY: argument points to a valid NUL-terminated locale string.
  let set_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!set_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_CTYPE, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_rejects_unsupported_locale_and_preserves_previous_state() {
  // SAFETY: argument points to a valid NUL-terminated locale string.
  let set_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!set_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"en_US.UTF-8\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_rejects_invalid_categories() {
  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let invalid_ptr = unsafe { setlocale(INVALID_CATEGORY, as_c_ptr(b"C\0")) };
  // SAFETY: argument points to a valid NUL-terminated locale string.
  let unsupported_ptr = unsafe { setlocale(UNSUPPORTED_CATEGORY, as_c_ptr(b"C\0")) };

  assert!(invalid_ptr.is_null());
  assert!(unsupported_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_repeated_calls_keep_consistent_state() {
  // SAFETY: arguments are valid NUL-terminated locale names.
  let first = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };
  // SAFETY: arguments are valid NUL-terminated locale names.
  let second = unsafe { setlocale(LC_ALL, as_c_ptr(b"POSIX\0")) };
  // SAFETY: arguments are valid NUL-terminated locale names.
  let third = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };
  // SAFETY: null query is valid per `setlocale` contract.
  let query = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!first.is_null());
  assert!(!second.is_null());
  assert!(!third.is_null());
  assert!(!query.is_null());
  assert_eq!(locale_name(first), b"C");
  assert_eq!(locale_name(second), b"C");
  assert_eq!(locale_name(third), b"C");
  assert_eq!(locale_name(query), b"C");
}

#[test]
fn setlocale_accepts_c_and_posix_for_lc_ctype() {
  // SAFETY: arguments are valid NUL-terminated locale names.
  let c_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"C\0")) };
  // SAFETY: arguments are valid NUL-terminated locale names.
  let posix_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"POSIX\0")) };
  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_CTYPE, ptr::null()) };

  assert!(!c_ptr.is_null());
  assert!(!posix_ptr.is_null());
  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(c_ptr), b"C");
  assert_eq!(locale_name(posix_ptr), b"C");
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_locale_uses_lc_all_environment_value() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "POSIX");
  set_locale_environment("LANG", "en_US.UTF-8");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_prefers_lc_all_over_category_variables() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "POSIX");
  set_locale_environment("LC_TIME", "en_US.UTF-8");
  set_locale_environment("LANG", "en_US.UTF-8");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_uses_lang_when_category_variables_are_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "");
  }

  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_falls_back_to_c_when_all_environment_values_are_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "");

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "");
  }

  set_locale_environment("LANG", "");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_rejects_unsupported_category_variable_with_supported_lang() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_NUMERIC", "en_US.UTF-8");
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_rejects_unsupported_category_variables_for_all_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(_, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "LC_ALL empty locale should reject unsupported {variable}"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_lc_all_locale_rejects_non_utf8_category_variables_for_all_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(_, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "LC_ALL empty locale should reject non-UTF-8 {variable}"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_lc_all_locale_ignores_non_utf8_lang_when_all_category_variables_supported() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment_raw_bytes("LANG", &[0xFF]);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_ignores_unsupported_lang_when_all_category_variables_supported() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment("LANG", "en_US.UTF-8");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_rejects_unsupported_lang_when_any_category_variable_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment("LC_TIME", "");
  set_locale_environment("LANG", "en_US.UTF-8");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_rejects_non_utf8_lang_when_any_category_variable_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment("LC_TIME", "");
  set_locale_environment_raw_bytes("LANG", &[0xFF]);

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_rejects_unsupported_lc_all_even_if_others_supported() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "en_US.UTF-8");
  set_locale_environment("LC_CTYPE", "POSIX");
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_locale_rejects_unsupported_environment_and_preserves_state() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "en_US.UTF-8");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_locale_rejects_non_utf8_lc_all_and_preserves_state() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment_raw_bytes("LC_ALL", &[0xFF]);

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_lc_all_locale_rejects_non_utf8_lc_all_even_if_others_supported() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment_raw_bytes("LC_ALL", &[0xFF]);
  set_locale_environment("LC_CTYPE", "POSIX");
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_locale_rejects_non_utf8_lang_when_all_overrides_are_unset() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment_raw_bytes("LANG", &[0xFF]);

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_uses_category_variable_before_lang() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_CTYPE", "POSIX");
  set_locale_environment("LANG", "en_US.UTF-8");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_uses_lang_when_category_variable_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_CTYPE", "");
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_falls_back_to_c_when_lc_all_category_and_lang_are_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment(variable, "");
    set_locale_environment("LANG", "");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should fallback to C with empty environment values"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_treats_empty_lc_all_as_unset() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "");
  set_locale_environment("LC_CTYPE", "POSIX");
  set_locale_environment("LANG", "en_US.UTF-8");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_prefers_category_when_lc_all_empty_and_lang_non_utf8() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "");
  set_locale_environment("LC_CTYPE", "POSIX");
  set_locale_environment_raw_bytes("LANG", &[0xFF]);

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_prefers_each_category_when_lc_all_empty_and_lang_unsupported() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment(variable, "POSIX");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer {variable} when LC_ALL is empty and LANG unsupported"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_each_category_when_lc_all_empty_and_lang_unset() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment(variable, "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer {variable} when LC_ALL is empty and LANG is unset"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_each_category_when_lc_all_empty_and_lang_non_utf8() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment(variable, "POSIX");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer {variable} when LC_ALL is empty and LANG non-UTF-8"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_category_and_lang() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "POSIX");
  set_locale_environment("LC_CTYPE", "en_US.UTF-8");
  set_locale_environment("LANG", "en_US.UTF-8");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_category_and_lang_for_all_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over unsupported {variable} and LANG"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_non_utf8_category_and_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over non-UTF-8 {variable} and LANG"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_non_utf8_category_with_supported_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over non-UTF-8 {variable} even when LANG is supported"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_non_utf8_category_with_empty_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over non-UTF-8 {variable} when LANG is empty"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_non_utf8_category_with_unset_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment_raw_bytes(variable, &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over non-UTF-8 {variable} when LANG is unset"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_non_utf8_category_with_unsupported_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over non-UTF-8 {variable} when LANG is unsupported"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_empty_category_and_non_utf8_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment(variable, "");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over empty {variable} and non-UTF-8 LANG"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_empty_category_and_unsupported_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment(variable, "");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over empty {variable} and unsupported LANG"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_empty_category_and_supported_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment(variable, "");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over empty {variable} even when LANG is supported"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_unsupported_category_with_supported_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over unsupported {variable} even when LANG is supported"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_unsupported_category_with_non_utf8_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over unsupported {variable} and non-UTF-8 LANG"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_unsupported_category_with_empty_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over unsupported {variable} when LANG is empty"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_lc_all_over_unsupported_category_with_unset_lang_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");
    set_locale_environment(variable, "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer LC_ALL over unsupported {variable} when LANG is unset"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_lc_all_even_when_category_supported() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "en_US.UTF-8");
  set_locale_environment("LC_CTYPE", "POSIX");
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_CTYPE, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_lc_all_even_when_category_supported() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment_raw_bytes("LC_ALL", &[0xFF]);
  set_locale_environment("LC_CTYPE", "POSIX");
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_CTYPE, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_lc_all_when_category_unset_for_all_categories()
{
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes("LC_ALL", &[0xFF]);
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 LC_ALL fallback when category variable is unset"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_lc_all_when_category_unset_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "en_US.UTF-8");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported LC_ALL fallback when category variable is unset"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_lc_all_when_each_category_variable_is_empty()
{
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "en_US.UTF-8");
    set_locale_environment(variable, "");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported LC_ALL fallback when {variable} is empty"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_lc_all_when_each_category_variable_is_supported()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "en_US.UTF-8");
    set_locale_environment(variable, "POSIX");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported LC_ALL even when {variable} is supported"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_lc_all_when_each_category_variable_is_supported()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes("LC_ALL", &[0xFF]);
    set_locale_environment(variable, "POSIX");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 LC_ALL even when {variable} is supported"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_lc_all_when_each_category_variable_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes("LC_ALL", &[0xFF]);
    set_locale_environment(variable, "");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 LC_ALL fallback when {variable} is empty"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_uses_lang_when_category_unset() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_uses_matching_category_variable_for_all_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "POSIX");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should resolve from {variable}"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_prefers_each_category_variable_when_lang_is_non_utf8() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "POSIX");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer {variable} when LANG is non-UTF-8"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_uses_lang_when_each_category_variable_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should fall back to LANG when {variable} is empty"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_falls_back_to_c_when_each_category_variable_and_lang_are_empty()
{
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "");
    set_locale_environment("LANG", "");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should fall back to C when {variable} and LANG are empty"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_uses_lang_when_each_category_variable_is_unset_and_lc_all_empty()
{
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should fall back to LANG when category variable is unset and LC_ALL is empty"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_falls_back_to_c_when_each_category_variable_is_unset_and_lc_all_and_lang_are_empty()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment("LANG", "");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should fall back to C when LC_ALL is empty and category/LANG are unset or empty"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_lang_when_each_category_variable_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 LANG fallback when {variable} is empty"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_lang_when_each_category_variable_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported LANG fallback when {variable} is empty"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_lang_when_each_category_variable_is_unset_and_lc_all_empty()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 LANG fallback when category variable is unset and LC_ALL is empty"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_lang_when_each_category_variable_is_unset_and_lc_all_empty()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported LANG fallback when category variable is unset and LC_ALL is empty"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_category_value() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_CTYPE", "en_US.UTF-8");
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_CTYPE, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_category_values_for_all_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported {variable}"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_category_values_when_lang_is_unset_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported {variable} even when LANG is unset"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_category_values_when_lang_is_empty_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported {variable} even when LANG is empty"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_category_values_when_lc_all_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported {variable} with empty LC_ALL"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_category_value() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment_raw_bytes("LC_CTYPE", &[0xFF]);
  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_CTYPE, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_category_values_when_lc_all_is_empty() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 {variable} with empty LC_ALL"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_category_values_for_all_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 {variable}"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_category_values_when_lang_is_unset_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes(variable, &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 {variable} even when LANG is unset"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_category_values_when_lang_is_empty_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 {variable} even when LANG is empty"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_lang_when_category_unset() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment_raw_bytes("LANG", &[0xFF]);

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"C\0")) };

  assert!(!baseline_ptr.is_null());

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_CTYPE, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_CTYPE, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
}

#[test]
fn setlocale_empty_category_locale_rejects_non_utf8_lang_when_category_unset_for_all_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject non-UTF-8 LANG fallback when category variable is unset"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_uses_lang_when_category_unset_for_all_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should use LANG fallback when category variable is unset"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_category_locale_rejects_unsupported_lang_when_category_unset_for_all_categories()
{
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, _) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!baseline_ptr.is_null());

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported LANG fallback when category variable is unset"
    );

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(!query_ptr.is_null());
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_empty_locale_preserves_errno_on_successful_environment_resolution() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "POSIX");
  write_errno(2718);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
  assert_eq!(read_errno(), 2718);
}

#[test]
fn setlocale_empty_locale_preserves_errno_on_unsupported_environment_rejection() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "en_US.UTF-8");
  write_errno(3141);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 3141);
}

#[test]
fn setlocale_empty_locale_success_preserves_errno_for_all_supported_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &category in &SUPPORTED_CATEGORIES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "POSIX");

    let expected_errno = 3200 + category;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should resolve empty locale to C from LC_ALL=POSIX",
    );
    assert_eq!(locale_name(locale_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_empty_locale_rejection_then_query_preserves_errno_for_all_supported_categories() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &category in &SUPPORTED_CATEGORIES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before empty-locale rejection checks",
    );

    let rejection_errno = 3300 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from unsupported LC_ALL",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 3400 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after empty-locale rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_locale_non_utf8_lc_all_rejection_then_query_preserves_errno_for_all_supported_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &category in &SUPPORTED_CATEGORIES {
    clear_locale_environment();
    set_locale_environment_raw_bytes("LC_ALL", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before empty-locale non-UTF-8 rejection checks",
    );

    let rejection_errno = 3350 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from non-UTF-8 LC_ALL",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 3450 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after empty-locale non-UTF-8 rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_and_unsupported_lang_rejection_then_query_preserves_errno_for_all_supported_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &category in &SUPPORTED_CATEGORIES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before empty-LC_ALL unsupported LANG rejection checks",
    );

    let rejection_errno = 3475 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from unsupported LANG when LC_ALL is empty",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 3490 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after empty-LC_ALL unsupported LANG rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_and_non_utf8_lang_rejection_then_query_preserves_errno_for_all_supported_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &category in &SUPPORTED_CATEGORIES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before empty-LC_ALL non-UTF-8 LANG rejection checks",
    );

    let rejection_errno = 3495 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from non-UTF-8 LANG when LC_ALL is empty",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 3510 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after empty-LC_ALL non-UTF-8 LANG rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_category_locale_non_utf8_category_rejection_then_query_preserves_errno_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before non-UTF-8 {variable} rejection checks",
    );

    let rejection_errno = 3550 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from non-UTF-8 {variable}",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 3650 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after non-UTF-8 {variable} rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_category_locale_unsupported_category_rejection_then_query_preserves_errno_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before unsupported {variable} rejection checks",
    );

    let rejection_errno = 3750 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from unsupported {variable}",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 3850 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after unsupported {variable} rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_category_locale_with_empty_lc_all_unsupported_category_rejection_then_query_preserves_errno_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before empty-LC_ALL unsupported {variable} rejection checks",
    );

    let rejection_errno = 3900 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from unsupported {variable} when LC_ALL is empty",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 3950 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after empty-LC_ALL unsupported {variable} rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_category_locale_with_empty_lc_all_non_utf8_category_rejection_then_query_preserves_errno_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before empty-LC_ALL non-UTF-8 {variable} rejection checks",
    );

    let rejection_errno = 4000 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from non-UTF-8 {variable} when LC_ALL is empty",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 4050 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after empty-LC_ALL non-UTF-8 {variable} rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_category_locale_with_empty_lc_all_non_utf8_lang_rejection_then_query_preserves_errno_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before empty-LC_ALL non-UTF-8 LANG rejection checks for {variable}",
    );

    let rejection_errno = 4100 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from non-UTF-8 LANG when LC_ALL is empty and {variable} is unset",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 4150 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after empty-LC_ALL non-UTF-8 LANG rejection with {variable} unset",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_category_locale_with_empty_lc_all_unsupported_lang_rejection_then_query_preserves_errno_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before empty-LC_ALL unsupported LANG rejection checks for {variable}",
    );

    let rejection_errno = 4400 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject empty locale resolution from unsupported LANG when LC_ALL is empty and {variable} is unset",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 4450 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after empty-LC_ALL unsupported LANG rejection with {variable} unset",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_unsupported_category_variable_rejection_then_query_preserves_errno()
{
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for (index, &(_, variable)) in CATEGORY_VARIABLES.iter().enumerate() {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");

    for &(_, category_variable) in &CATEGORY_VARIABLES {
      set_locale_environment(category_variable, "POSIX");
    }

    set_locale_environment(variable, "en_US.UTF-8");
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "LC_ALL should accept baseline C locale before unsupported {variable} rejection checks",
    );

    let rejection_errno = 4500 + index as c_int;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "LC_ALL empty locale should reject unsupported {variable}",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 4550 + index as c_int;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "LC_ALL query should stay available after unsupported {variable} rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_non_utf8_category_variable_rejection_then_query_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for (index, &(_, variable)) in CATEGORY_VARIABLES.iter().enumerate() {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");

    for &(_, category_variable) in &CATEGORY_VARIABLES {
      set_locale_environment(category_variable, "POSIX");
    }

    set_locale_environment_raw_bytes(variable, &[0xFF]);
    set_locale_environment("LANG", "POSIX");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "LC_ALL should accept baseline C locale before non-UTF-8 {variable} rejection checks",
    );

    let rejection_errno = 4600 + index as c_int;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "LC_ALL empty locale should reject non-UTF-8 {variable}",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 4650 + index as c_int;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "LC_ALL query should stay available after non-UTF-8 {variable} rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_empty_category_variable_and_unsupported_lang_rejection_then_query_preserves_errno()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for (index, &(_, variable)) in CATEGORY_VARIABLES.iter().enumerate() {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");

    for &(_, category_variable) in &CATEGORY_VARIABLES {
      set_locale_environment(category_variable, "POSIX");
    }

    set_locale_environment(variable, "");
    set_locale_environment("LANG", "en_US.UTF-8");

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "LC_ALL should accept baseline C locale before empty {variable} + unsupported LANG rejection checks",
    );

    let rejection_errno = 4700 + index as c_int;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "LC_ALL empty locale should reject when {variable} is empty and LANG is unsupported",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 4750 + index as c_int;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "LC_ALL query should stay available after empty {variable} + unsupported LANG rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_empty_category_variable_and_non_utf8_lang_rejection_then_query_preserves_errno()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for (index, &(_, variable)) in CATEGORY_VARIABLES.iter().enumerate() {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");

    for &(_, category_variable) in &CATEGORY_VARIABLES {
      set_locale_environment(category_variable, "POSIX");
    }

    set_locale_environment(variable, "");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "LC_ALL should accept baseline C locale before empty {variable} + non-UTF-8 LANG rejection checks",
    );

    let rejection_errno = 4800 + index as c_int;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      rejected_ptr.is_null(),
      "LC_ALL empty locale should reject when {variable} is empty and LANG is non-UTF-8",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 4850 + index as c_int;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "LC_ALL query should stay available after empty {variable} + non-UTF-8 LANG rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_empty_category_variable_and_supported_lang_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for (index, &(_, variable)) in CATEGORY_VARIABLES.iter().enumerate() {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");

    for &(_, category_variable) in &CATEGORY_VARIABLES {
      set_locale_environment(category_variable, "POSIX");
    }

    set_locale_environment(variable, "");
    set_locale_environment("LANG", "POSIX");

    let expected_errno = 4900 + index as c_int;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "LC_ALL empty locale should resolve when {variable} is empty and LANG is supported",
    );
    assert_eq!(locale_name(locale_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_empty_category_variable_and_unset_lang_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for (index, &(_, variable)) in CATEGORY_VARIABLES.iter().enumerate() {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");

    for &(_, category_variable) in &CATEGORY_VARIABLES {
      set_locale_environment(category_variable, "POSIX");
    }

    set_locale_environment(variable, "");

    let expected_errno = 5000 + index as c_int;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "LC_ALL empty locale should resolve when {variable} is empty and LANG is unset",
    );
    assert_eq!(locale_name(locale_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_empty_category_variable_and_empty_lang_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for (index, &(_, variable)) in CATEGORY_VARIABLES.iter().enumerate() {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");

    for &(_, category_variable) in &CATEGORY_VARIABLES {
      set_locale_environment(category_variable, "POSIX");
    }

    set_locale_environment(variable, "");
    set_locale_environment("LANG", "");

    let expected_errno = 5100 + index as c_int;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "LC_ALL empty locale should resolve when {variable} and LANG are empty",
    );
    assert_eq!(locale_name(locale_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_all_category_variables_empty_and_supported_lang_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "");

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "");
  }

  set_locale_environment("LANG", "POSIX");

  write_errno(5206);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
  assert_eq!(read_errno(), 5206);
}

#[test]
fn setlocale_empty_lc_all_with_unsupported_lc_all_and_supported_category_variables_rejection_then_query_preserves_errno()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "en_US.UTF-8");

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(
    !baseline_ptr.is_null(),
    "LC_ALL should accept baseline C locale before unsupported LC_ALL rejection checks",
  );

  write_errno(5306);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 5306);

  write_errno(5316);

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
  assert_eq!(read_errno(), 5316);
}

#[test]
fn setlocale_empty_lc_all_with_non_utf8_lc_all_and_supported_category_variables_rejection_then_query_preserves_errno()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment_raw_bytes("LC_ALL", &[0xFF]);

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment("LANG", "POSIX");

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let baseline_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"C\0")) };

  assert!(
    !baseline_ptr.is_null(),
    "LC_ALL should accept baseline C locale before non-UTF-8 LC_ALL rejection checks",
  );

  write_errno(5406);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 5406);

  write_errno(5416);

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
  assert_eq!(read_errno(), 5416);
}

#[test]
fn setlocale_empty_category_locale_prefers_category_variable_and_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment(variable, "POSIX");
    set_locale_environment("LANG", "en_US.UTF-8");

    let expected_errno = 3500 + category;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer {variable}=POSIX over unsupported LANG fallback",
    );
    assert_eq!(locale_name(locale_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_uses_supported_category_variables_and_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment("LANG", "en_US.UTF-8");

  write_errno(3606);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
  assert_eq!(read_errno(), 3606);
}

#[test]
fn setlocale_empty_category_locale_with_empty_lc_all_supported_category_and_non_utf8_lang_preserves_errno_for_all_categories()
 {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment(variable, "POSIX");
    set_locale_environment_raw_bytes("LANG", &[0xFF]);

    let expected_errno = 4200 + category;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer {variable}=POSIX over non-UTF-8 LANG fallback when LC_ALL is empty",
    );
    assert_eq!(locale_name(locale_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_supported_category_variables_and_non_utf8_lang_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "");

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment_raw_bytes("LANG", &[0xFF]);

  write_errno(4306);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
  assert_eq!(read_errno(), 4306);
}

#[test]
fn setlocale_empty_category_locale_with_empty_lc_all_prefers_category_variable_and_preserves_errno()
{
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  for &(category, variable) in &CATEGORY_VARIABLES {
    clear_locale_environment();
    set_locale_environment("LC_ALL", "");
    set_locale_environment(variable, "POSIX");
    set_locale_environment("LANG", "en_US.UTF-8");

    let expected_errno = 3700 + category;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should prefer {variable}=POSIX when LC_ALL is empty and LANG is unsupported",
    );
    assert_eq!(locale_name(locale_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_empty_lc_all_with_supported_category_variables_preserves_errno() {
  let _env_lock = lock_locale_environment();
  let _snapshot = EnvironmentSnapshot::capture(&LOCALE_ENV_KEYS);

  clear_locale_environment();
  set_locale_environment("LC_ALL", "");

  for &(_, variable) in &CATEGORY_VARIABLES {
    set_locale_environment(variable, "POSIX");
  }

  set_locale_environment("LANG", "en_US.UTF-8");

  write_errno(3806);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let locale_ptr = unsafe { setlocale(LC_ALL, as_c_ptr(b"\0")) };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
  assert_eq!(read_errno(), 3806);
}

#[test]
fn setlocale_query_preserves_errno() {
  write_errno(1122);

  // SAFETY: null query is valid per `setlocale` contract.
  let query_ptr = unsafe { setlocale(LC_ALL, ptr::null()) };

  assert!(!query_ptr.is_null());
  assert_eq!(locale_name(query_ptr), b"C");
  assert_eq!(read_errno(), 1122);
}

#[test]
fn setlocale_query_preserves_errno_for_all_supported_categories() {
  for &category in &SUPPORTED_CATEGORIES {
    let expected_errno = 1130 + category;

    write_errno(expected_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should return current locale",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_invalid_category_preserves_errno() {
  write_errno(3344);

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let rejected_ptr = unsafe { setlocale(INVALID_CATEGORY, as_c_ptr(b"C\0")) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 3344);
}

#[test]
fn setlocale_unsupported_category_preserves_errno() {
  write_errno(3355);

  // SAFETY: argument points to a valid NUL-terminated locale string.
  let rejected_ptr = unsafe { setlocale(UNSUPPORTED_CATEGORY, as_c_ptr(b"C\0")) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 3355);
}

#[test]
fn setlocale_invalid_category_query_preserves_errno() {
  write_errno(3366);

  // SAFETY: null query is valid per `setlocale` contract.
  let rejected_ptr = unsafe { setlocale(INVALID_CATEGORY, ptr::null()) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 3366);
}

#[test]
fn setlocale_invalid_category_empty_locale_preserves_errno() {
  write_errno(3367);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(INVALID_CATEGORY, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 3367);
}

#[test]
fn setlocale_unsupported_category_query_preserves_errno() {
  write_errno(3377);

  // SAFETY: null query is valid per `setlocale` contract.
  let rejected_ptr = unsafe { setlocale(UNSUPPORTED_CATEGORY, ptr::null()) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 3377);
}

#[test]
fn setlocale_unsupported_category_empty_locale_preserves_errno() {
  write_errno(3378);

  // SAFETY: argument points to a valid NUL-terminated locale string (`""`).
  let rejected_ptr = unsafe { setlocale(UNSUPPORTED_CATEGORY, as_c_ptr(b"\0")) };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 3378);
}

#[test]
fn setlocale_explicit_locale_success_preserves_errno() {
  write_errno(4455);

  let locale_ptr = {
    // SAFETY: argument points to a valid NUL-terminated locale string.
    unsafe { setlocale(LC_CTYPE, as_c_ptr(b"POSIX\0")) }
  };

  assert!(!locale_ptr.is_null());
  assert_eq!(locale_name(locale_ptr), b"C");
  assert_eq!(read_errno(), 4455);
}

#[test]
fn setlocale_explicit_locale_rejection_preserves_errno() {
  write_errno(5566);

  let rejected_ptr = {
    // SAFETY: argument points to a valid NUL-terminated locale string.
    unsafe { setlocale(LC_CTYPE, as_c_ptr(b"en_US.UTF-8\0")) }
  };

  assert!(rejected_ptr.is_null());
  assert_eq!(read_errno(), 5566);
}

#[test]
fn setlocale_explicit_locale_success_preserves_errno_for_all_categories() {
  for &category in &SUPPORTED_CATEGORIES {
    let expected_errno = 6000 + category;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"POSIX\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should accept explicit POSIX locale",
    );
    assert_eq!(locale_name(locale_ptr), b"C");
    assert_eq!(read_errno(), expected_errno);
  }
}

#[test]
fn setlocale_explicit_locale_rejection_preserves_errno_for_all_categories() {
  for &category in &SUPPORTED_CATEGORIES {
    let expected_errno = 7000 + category;

    write_errno(expected_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"en_US.UTF-8\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject explicit unsupported locale",
    );
    assert_eq!(read_errno(), expected_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
  }
}

#[test]
fn setlocale_explicit_non_utf8_locale_rejection_preserves_errno_for_all_categories() {
  for &category in &SUPPORTED_CATEGORIES {
    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before explicit non-UTF-8 rejection checks",
    );

    let rejection_errno = 7100 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"\xFF\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject explicit non-UTF-8 locale name",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 7200 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should stay available after explicit non-UTF-8 rejection",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_rejection_then_query_preserves_errno_for_all_supported_categories() {
  for &category in &SUPPORTED_CATEGORIES {
    // SAFETY: argument points to a valid NUL-terminated locale string.
    let baseline_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(
      !baseline_ptr.is_null(),
      "category {category} should accept baseline C locale before rejection/query checks",
    );

    let rejection_errno = 8000 + category;

    write_errno(rejection_errno);

    // SAFETY: argument points to a valid NUL-terminated locale string.
    let rejected_ptr = unsafe { setlocale(category, as_c_ptr(b"en_US.UTF-8\0")) };

    assert!(
      rejected_ptr.is_null(),
      "category {category} should reject unsupported explicit locale",
    );
    assert_eq!(read_errno(), rejection_errno);

    let query_errno = 8100 + category;

    write_errno(query_errno);

    // SAFETY: null query is valid per `setlocale` contract.
    let query_ptr = unsafe { setlocale(category, ptr::null()) };

    assert!(
      !query_ptr.is_null(),
      "category {category} query should remain available after rejected explicit locale",
    );
    assert_eq!(locale_name(query_ptr), b"C");
    assert_eq!(read_errno(), query_errno);
  }
}

#[test]
fn setlocale_accepts_c_for_all_supported_categories() {
  for &category in &SUPPORTED_CATEGORIES {
    // SAFETY: argument points to a valid NUL-terminated locale string.
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"C\0")) };

    assert!(!locale_ptr.is_null(), "category {category} should accept C");
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}

#[test]
fn setlocale_accepts_posix_for_all_supported_categories() {
  for &category in &SUPPORTED_CATEGORIES {
    // SAFETY: argument points to a valid NUL-terminated locale string.
    let locale_ptr = unsafe { setlocale(category, as_c_ptr(b"POSIX\0")) };

    assert!(
      !locale_ptr.is_null(),
      "category {category} should accept POSIX alias"
    );
    assert_eq!(locale_name(locale_ptr), b"C");
  }
}
