use core::ffi::{c_char, c_int};
use rlibc::abi::errno::{EINVAL, ENOENT, ENOEXEC};
use rlibc::dlfcn::{RTLD_GLOBAL, RTLD_LAZY, RTLD_LOCAL, RTLD_NOW, dlclose, dlopen};
use rlibc::errno::__errno_location;
use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn set_errno(value: c_int) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage.
  unsafe {
    errno_ptr.write(value);
  }
}

fn errno_value() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage.
  unsafe { errno_ptr.read() }
}

fn c_string_from_path(path: &Path) -> CString {
  CString::new(path.to_string_lossy().as_bytes()).expect("path must not contain interior NUL")
}

fn unique_temp_path(stem: &str) -> PathBuf {
  let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);

  std::env::temp_dir().join(format!(
    "rlibc_i056_{stem}_{}_{}",
    std::process::id(),
    unique
  ))
}

fn first_loaded_shared_object() -> Option<PathBuf> {
  let maps = fs::read_to_string("/proc/self/maps").ok()?;

  for line in maps.lines() {
    let path = line.split_ascii_whitespace().last()?;

    if !path.starts_with('/') || !path.contains(".so") {
      continue;
    }

    let candidate = PathBuf::from(path);

    if candidate.is_file() {
      return Some(candidate);
    }
  }

  None
}

#[test]
fn dlopen_rejects_unsupported_flag_bits_with_einval() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056.so")
    .expect("static path must not contain interior NUL");
  let invalid_flags = RTLD_NOW | RTLD_LOCAL | 0x4000;

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), invalid_flags) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_unsupported_lazy_flag_bits_with_einval() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_lazy.so")
    .expect("static path must not contain interior NUL");
  let invalid_flags = RTLD_LAZY | RTLD_GLOBAL | 0x2000;

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), invalid_flags) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_unsupported_flag_bits_for_valid_shared_object_with_einval() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);
  let invalid_flags = RTLD_NOW | RTLD_GLOBAL | 0x4000;

  set_errno(ENOEXEC);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), invalid_flags) };

  assert!(
    handle.is_null(),
    "dlopen should reject unsupported flag bits even for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_unsupported_lazy_flag_bits_for_valid_shared_object_with_einval() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);
  let invalid_flags = RTLD_LAZY | RTLD_GLOBAL | 0x2000;

  set_errno(ENOENT);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), invalid_flags) };

  assert!(
    handle.is_null(),
    "dlopen should reject unsupported lazy-mode flag bits even for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_unsupported_lazy_flag_bits_without_visibility_for_valid_shared_object_with_einval()
 {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);
  let invalid_flags = RTLD_LAZY | 0x2000;

  set_errno(ENOENT);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), invalid_flags) };

  assert!(
    handle.is_null(),
    "dlopen should reject unsupported lazy-mode bits without visibility flags for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_global_without_binding_mode_with_einval() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_global_only.so")
    .expect("static path must not contain interior NUL");

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), RTLD_GLOBAL) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_global_without_binding_mode_for_valid_shared_object_with_einval() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(ENOEXEC);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_GLOBAL) };

  assert!(
    handle.is_null(),
    "dlopen should reject RTLD_GLOBAL without binding mode even for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_local_without_binding_mode_with_einval() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_local_only.so")
    .expect("static path must not contain interior NUL");

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), RTLD_LOCAL) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_local_without_binding_mode_for_valid_shared_object_with_einval() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(ENOENT);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LOCAL) };

  assert!(
    handle.is_null(),
    "dlopen should reject RTLD_LOCAL without binding mode even for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_conflicting_binding_modes_with_einval() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_conflicting_modes.so")
    .expect("static path must not contain interior NUL");
  let conflicting_flags = RTLD_LAZY | RTLD_NOW;

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), conflicting_flags) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_conflicting_binding_modes_with_global_visibility_and_einval() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_conflicting_global.so")
    .expect("static path must not contain interior NUL");
  let conflicting_flags = RTLD_LAZY | RTLD_NOW | RTLD_GLOBAL;

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), conflicting_flags) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_conflicting_binding_modes_with_global_visibility_for_valid_shared_object_with_einval()
 {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);
  let conflicting_flags = RTLD_LAZY | RTLD_NOW | RTLD_GLOBAL;

  set_errno(ENOEXEC);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), conflicting_flags) };

  assert!(
    handle.is_null(),
    "dlopen should reject conflicting binding flags with RTLD_GLOBAL even for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_rejects_conflicting_binding_modes_for_valid_shared_object_with_einval() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);
  let conflicting_flags = RTLD_LAZY | RTLD_NOW;

  set_errno(ENOENT);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), conflicting_flags) };

  assert!(
    handle.is_null(),
    "dlopen should reject conflicting binding flags even for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_null_filename_returns_null_and_einval() {
  set_errno(0);
  // SAFETY: null pointer is intentional to validate input checking.
  let handle = unsafe { dlopen(core::ptr::null(), RTLD_NOW) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), EINVAL);
}

#[test]
fn dlopen_missing_path_returns_null_and_enoent() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_missing.so")
    .expect("static path must not contain interior NUL");

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn dlopen_missing_path_with_lazy_binding_returns_null_and_enoent() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_missing_lazy.so")
    .expect("static path must not contain interior NUL");

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(missing_path.as_ptr().cast::<c_char>(), RTLD_LAZY) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn dlopen_missing_path_with_global_visibility_returns_null_and_enoent() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_missing_global.so")
    .expect("static path must not contain interior NUL");

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe {
    dlopen(
      missing_path.as_ptr().cast::<c_char>(),
      RTLD_NOW | RTLD_GLOBAL,
    )
  };

  assert!(handle.is_null());
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn dlopen_missing_path_with_lazy_global_visibility_returns_null_and_enoent() {
  let missing_path = CString::new("/definitely/missing/rlibc_i056_missing_lazy_global.so")
    .expect("static path must not contain interior NUL");

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe {
    dlopen(
      missing_path.as_ptr().cast::<c_char>(),
      RTLD_LAZY | RTLD_GLOBAL,
    )
  };

  assert!(handle.is_null());
  assert_eq!(errno_value(), ENOENT);
}

#[test]
fn dlopen_non_elf_file_returns_null_and_enoexec() {
  let non_elf_path = unique_temp_path("not_elf");

  fs::write(&non_elf_path, b"not an elf image").expect("failed to write non-ELF fixture");

  let path_cstr = c_string_from_path(&non_elf_path);

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), ENOEXEC);

  fs::remove_file(&non_elf_path).expect("failed to cleanup non-ELF fixture");
}

#[test]
fn dlopen_non_elf_file_with_lazy_binding_returns_null_and_enoexec() {
  let non_elf_path = unique_temp_path("not_elf_lazy");

  fs::write(&non_elf_path, b"not an elf image").expect("failed to write non-ELF fixture");

  let path_cstr = c_string_from_path(&non_elf_path);

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LAZY) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), ENOEXEC);

  fs::remove_file(&non_elf_path).expect("failed to cleanup non-ELF fixture");
}

#[test]
fn dlopen_empty_file_returns_null_and_enoexec() {
  let empty_file_path = unique_temp_path("empty_file");

  fs::write(&empty_file_path, b"").expect("failed to create empty fixture file");

  let path_cstr = c_string_from_path(&empty_file_path);

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), ENOEXEC);

  fs::remove_file(&empty_file_path).expect("failed to cleanup empty fixture file");
}

#[test]
fn dlopen_empty_file_with_lazy_binding_returns_null_and_enoexec() {
  let empty_file_path = unique_temp_path("empty_file_lazy");

  fs::write(&empty_file_path, b"").expect("failed to create empty fixture file");

  let path_cstr = c_string_from_path(&empty_file_path);

  set_errno(0);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LAZY) };

  assert!(handle.is_null());
  assert_eq!(errno_value(), ENOEXEC);

  fs::remove_file(&empty_file_path).expect("failed to cleanup empty fixture file");
}

#[test]
fn dlopen_valid_shared_object_returns_non_null_handle_and_preserves_errno() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(EINVAL);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW | RTLD_LOCAL) };

  assert!(
    !handle.is_null(),
    "dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);

  assert_eq!(dlclose(handle), 0);
}

#[test]
fn dlopen_valid_shared_object_with_global_visibility_preserves_errno() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(ENOEXEC);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW | RTLD_GLOBAL) };

  assert!(
    !handle.is_null(),
    "dlopen should accept RTLD_GLOBAL for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), ENOEXEC);

  assert_eq!(dlclose(handle), 0);
}

#[test]
fn dlopen_valid_shared_object_with_now_binding_only_preserves_errno() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(ENOENT);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should accept RTLD_NOW without explicit visibility flag for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), ENOENT);

  assert_eq!(dlclose(handle), 0);
}

#[test]
fn dlopen_valid_shared_object_with_lazy_binding_only_preserves_errno() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(ENOEXEC);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LAZY) };

  assert!(
    !handle.is_null(),
    "dlopen should accept RTLD_LAZY without explicit visibility flag for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), ENOEXEC);

  assert_eq!(dlclose(handle), 0);
}

#[test]
fn dlopen_valid_shared_object_with_lazy_local_visibility_preserves_errno() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(ENOENT);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LAZY | RTLD_LOCAL) };

  assert!(
    !handle.is_null(),
    "dlopen should accept RTLD_LAZY | RTLD_LOCAL for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), ENOENT);

  assert_eq!(dlclose(handle), 0);
}

#[test]
fn dlopen_valid_shared_object_with_lazy_global_visibility_preserves_errno() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(EINVAL);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LAZY | RTLD_GLOBAL) };

  assert!(
    !handle.is_null(),
    "dlopen should accept RTLD_LAZY | RTLD_GLOBAL for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);

  assert_eq!(dlclose(handle), 0);
}

#[test]
fn dlopen_repeated_loads_are_accepted_by_rlibc_dlclose() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(ENOEXEC);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let first_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !first_handle.is_null(),
    "first dlopen should succeed for {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), ENOEXEC);

  set_errno(ENOENT);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let second_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LAZY) };

  assert!(
    !second_handle.is_null(),
    "second dlopen should succeed for {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), ENOENT);

  assert_eq!(dlclose(second_handle), 0);
  assert_eq!(dlclose(first_handle), 0);
}

#[test]
fn dlopen_repeated_loads_with_mixed_visibility_modes_preserve_errno() {
  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  set_errno(ENOEXEC);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let first_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW | RTLD_GLOBAL) };

  assert!(
    !first_handle.is_null(),
    "first dlopen should succeed for {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), ENOEXEC);

  set_errno(EINVAL);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let second_handle =
    unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_LAZY | RTLD_LOCAL) };

  assert!(
    !second_handle.is_null(),
    "second dlopen should succeed for {}",
    shared_object_path.display(),
  );
  assert_eq!(errno_value(), EINVAL);

  assert_eq!(dlclose(second_handle), 0);
  assert_eq!(dlclose(first_handle), 0);
}
