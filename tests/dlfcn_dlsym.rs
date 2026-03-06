use core::ffi::{c_char, c_void};
use core::ptr;
use rlibc::abi::types::{c_int, c_long, c_longlong, c_uint, c_ulong, c_ulonglong, size_t, ssize_t};
use rlibc::ctype::{
  isalnum as rlibc_isalnum, isalpha as rlibc_isalpha, isblank as rlibc_isblank,
  iscntrl as rlibc_iscntrl, isdigit as rlibc_isdigit, isgraph as rlibc_isgraph,
  islower as rlibc_islower, isprint as rlibc_isprint, ispunct as rlibc_ispunct,
  isspace as rlibc_isspace, isupper as rlibc_isupper, isxdigit as rlibc_isxdigit,
  tolower as rlibc_tolower, toupper as rlibc_toupper,
};
use rlibc::dirent::{
  Dir as RlibcDir, Dirent as RlibcDirent, closedir as rlibc_closedir, opendir as rlibc_opendir,
  readdir as rlibc_readdir, rewinddir as rlibc_rewinddir,
};
use rlibc::dlfcn::{RTLD_NOW, dlclose, dlerror, dlopen, dlsym};
use rlibc::errno::__errno_location;
use rlibc::fcntl::fcntl as rlibc_fcntl;
use rlibc::fenv::{
  feclearexcept as rlibc_feclearexcept, fegetenv as rlibc_fegetenv,
  fegetexceptflag as rlibc_fegetexceptflag, fegetround as rlibc_fegetround,
  feholdexcept as rlibc_feholdexcept, fenv_t as RlibcFenvT, feraiseexcept as rlibc_feraiseexcept,
  fesetenv as rlibc_fesetenv, fesetexceptflag as rlibc_fesetexceptflag,
  fesetround as rlibc_fesetround, fetestexcept as rlibc_fetestexcept,
  feupdateenv as rlibc_feupdateenv, fexcept_t as RlibcFexceptT,
};
use rlibc::fs::{
  Stat as RlibcStat, fstat as rlibc_fstat, fstatat as rlibc_fstatat, lstat as rlibc_lstat,
  stat as rlibc_stat,
};
use rlibc::glob::{
  Glob as RlibcGlob, GlobErrorFn as RlibcGlobErrorFn, glob as rlibc_glob,
  globfree as rlibc_globfree,
};
use rlibc::locale::setlocale as rlibc_setlocale;
use rlibc::math::{exp as rlibc_exp, log as rlibc_log, sqrt as rlibc_sqrt};
use rlibc::memory::{
  memcmp as rlibc_memcmp, memcpy as rlibc_memcpy, memmove as rlibc_memmove, memset as rlibc_memset,
};
use rlibc::netdb::{
  addrinfo as RlibcAddrInfo, freeaddrinfo as rlibc_freeaddrinfo,
  gai_strerror as rlibc_gai_strerror, getaddrinfo as rlibc_getaddrinfo,
  getnameinfo as rlibc_getnameinfo, sockaddr as RlibcSockAddr, socklen_t as rlibc_socklen_t,
};
use rlibc::pthread::{
  pthread_attr_t, pthread_cond_broadcast as rlibc_pthread_cond_broadcast,
  pthread_cond_destroy as rlibc_pthread_cond_destroy, pthread_cond_init as rlibc_pthread_cond_init,
  pthread_cond_signal as rlibc_pthread_cond_signal, pthread_cond_t,
  pthread_cond_timedwait as rlibc_pthread_cond_timedwait,
  pthread_cond_wait as rlibc_pthread_cond_wait,
  pthread_condattr_destroy as rlibc_pthread_condattr_destroy,
  pthread_condattr_getpshared as rlibc_pthread_condattr_getpshared,
  pthread_condattr_init as rlibc_pthread_condattr_init,
  pthread_condattr_setpshared as rlibc_pthread_condattr_setpshared, pthread_condattr_t,
  pthread_create as rlibc_pthread_create, pthread_detach as rlibc_pthread_detach,
  pthread_join as rlibc_pthread_join, pthread_mutex_destroy as rlibc_pthread_mutex_destroy,
  pthread_mutex_init as rlibc_pthread_mutex_init, pthread_mutex_lock as rlibc_pthread_mutex_lock,
  pthread_mutex_t, pthread_mutex_trylock as rlibc_pthread_mutex_trylock,
  pthread_mutex_unlock as rlibc_pthread_mutex_unlock,
  pthread_mutexattr_destroy as rlibc_pthread_mutexattr_destroy,
  pthread_mutexattr_getpshared as rlibc_pthread_mutexattr_getpshared,
  pthread_mutexattr_gettype as rlibc_pthread_mutexattr_gettype,
  pthread_mutexattr_init as rlibc_pthread_mutexattr_init,
  pthread_mutexattr_setpshared as rlibc_pthread_mutexattr_setpshared,
  pthread_mutexattr_settype as rlibc_pthread_mutexattr_settype, pthread_mutexattr_t,
  pthread_rwlock_destroy as rlibc_pthread_rwlock_destroy,
  pthread_rwlock_init as rlibc_pthread_rwlock_init,
  pthread_rwlock_rdlock as rlibc_pthread_rwlock_rdlock, pthread_rwlock_t,
  pthread_rwlock_tryrdlock as rlibc_pthread_rwlock_tryrdlock,
  pthread_rwlock_trywrlock as rlibc_pthread_rwlock_trywrlock,
  pthread_rwlock_unlock as rlibc_pthread_rwlock_unlock,
  pthread_rwlock_wrlock as rlibc_pthread_rwlock_wrlock, pthread_rwlockattr_t, pthread_t,
};
use rlibc::resource::{
  RLimit as RlibcRLimit, getrlimit as rlibc_getrlimit, prlimit64 as rlibc_prlimit64,
  setrlimit as rlibc_setrlimit,
};
use rlibc::setjmp::{jmp_buf as rlibc_jmp_buf, longjmp as rlibc_longjmp, setjmp as rlibc_setjmp};
use rlibc::signal::{
  SigAction as RlibcSigAction, SigSet as RlibcSigSet, kill as rlibc_kill, raise as rlibc_raise,
  sigaction as rlibc_sigaction, sigaddset as rlibc_sigaddset, sigdelset as rlibc_sigdelset,
  sigemptyset as rlibc_sigemptyset, sigfillset as rlibc_sigfillset,
  sigismember as rlibc_sigismember, sigprocmask as rlibc_sigprocmask,
};
use rlibc::socket::{
  Sockaddr as RlibcSockaddrCore, SocklenT as RlibcSocklenTCore, accept as rlibc_accept,
  bind as rlibc_bind, connect as rlibc_connect, listen as rlibc_listen, socket as rlibc_socket,
};
use rlibc::startup::{__libc_start_main as rlibc_libc_start_main, StartMainFn as RlibcStartMainFn};
use rlibc::stdio::{
  FILE, fflush as rlibc_fflush, fileno as rlibc_fileno, fileno_unlocked as rlibc_fileno_unlocked,
  fopen as rlibc_fopen, fprintf as rlibc_fprintf, fputs as rlibc_fputs, fread as rlibc_fread,
  printf as rlibc_printf, setbuf as rlibc_setbuf, setbuffer as rlibc_setbuffer,
  setlinebuf as rlibc_setlinebuf, setvbuf as rlibc_setvbuf, tmpfile as rlibc_tmpfile,
  vfprintf as rlibc_vfprintf, vprintf as rlibc_vprintf, vsnprintf as rlibc_vsnprintf,
};
use rlibc::stdlib::alloc::{
  aligned_alloc_c_abi as rlibc_aligned_alloc, malloc_c_abi as rlibc_malloc,
  memalign_c_abi as rlibc_memalign, posix_memalign_c_abi as rlibc_posix_memalign,
  pvalloc_c_abi as rlibc_pvalloc, valloc_c_abi as rlibc_valloc,
};
use rlibc::stdlib::atoi::{atoi as rlibc_atoi, atol as rlibc_atol, atoll as rlibc_atoll};
use rlibc::stdlib::conv::{
  strtol as rlibc_strtol, strtoll as rlibc_strtoll, strtoul as rlibc_strtoul,
  strtoull as rlibc_strtoull,
};
use rlibc::stdlib::env::core::getenv as rlibc_getenv;
use rlibc::stdlib::env::mutating::setenv as rlibc_setenv;
use rlibc::stdlib::{
  _Exit as rlibc_underscore_exit, abort as rlibc_abort, atexit as rlibc_atexit,
  environ as rlibc_environ, exit as rlibc_exit,
};
use rlibc::string::{strlen as rlibc_strlen, strnlen as rlibc_strnlen};
use rlibc::system::{
  SysInfo as RlibcSysInfo, UtsName as RlibcUtsName, gethostname as rlibc_gethostname,
  getpagesize as rlibc_getpagesize, sysconf as rlibc_sysconf, sysinfo as rlibc_sysinfo,
  uname as rlibc_uname,
};
use rlibc::time::{
  clock_gettime as rlibc_clock_gettime, clockid_t as rlibc_clockid_t,
  gettimeofday as rlibc_gettimeofday, gmtime as rlibc_gmtime, gmtime_r as rlibc_gmtime_r,
  localtime as rlibc_localtime, localtime_r as rlibc_localtime_r, mktime as rlibc_mktime,
  strftime as rlibc_strftime, time_t as rlibc_time_t, timegm as rlibc_timegm,
  timespec as RlibcTimespec, timeval as RlibcTimeval, timezone as RlibcTimezone, tm as RlibcTm,
};
use rlibc::unistd::{
  access as rlibc_access, close as rlibc_close, dup as rlibc_dup, dup2 as rlibc_dup2,
  dup3 as rlibc_dup3, fdatasync as rlibc_fdatasync, fsync as rlibc_fsync, getegid as rlibc_getegid,
  geteuid as rlibc_geteuid, getgid as rlibc_getgid, getpgid as rlibc_getpgid,
  getpgrp as rlibc_getpgrp, getpid as rlibc_getpid, getppid as rlibc_getppid,
  getsid as rlibc_getsid, gettid as rlibc_gettid, getuid as rlibc_getuid, isatty as rlibc_isatty,
  lseek as rlibc_lseek, open as rlibc_open, openat as rlibc_openat, pipe as rlibc_pipe,
  pipe2 as rlibc_pipe2, read as rlibc_read, recv as rlibc_recv, send as rlibc_send,
  sync as rlibc_sync, syncfs as rlibc_syncfs, unlink as rlibc_unlink, write as rlibc_write,
};
use rlibc::wchar::{
  mblen as rlibc_mblen, mbrlen as rlibc_mbrlen, mbrtowc as rlibc_mbrtowc, mbsinit as rlibc_mbsinit,
  mbsrtowcs as rlibc_mbsrtowcs, mbstate_t as RlibcMbStateT, mbstowcs as rlibc_mbstowcs,
  mbtowc as rlibc_mbtowc, wchar_t as rlibc_wchar_t, wcrtomb as rlibc_wcrtomb,
  wcsrtombs as rlibc_wcsrtombs, wcstombs as rlibc_wcstombs, wctomb as rlibc_wctomb,
};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::{fs, thread};

const RTLD_DEFAULT: *mut c_void = ptr::null_mut();
const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
const MISSING_SYMBOL_DETAIL: &[u8] = b"rlibc_i055_missing_symbol_detail\0";

fn dlfcn_test_lock() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

  LOCK
    .get_or_init(|| Mutex::new(()))
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const fn symbol_ptr(bytes: &'static [u8]) -> *const c_char {
  bytes.as_ptr().cast()
}

fn c_string_from_path(path: &Path) -> CString {
  CString::new(path.to_string_lossy().as_bytes()).expect("path must not contain interior NUL")
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

fn loaded_libc_path() -> Option<PathBuf> {
  let maps = fs::read_to_string("/proc/self/maps").ok()?;

  for line in maps.lines() {
    let path = line.split_ascii_whitespace().last()?;

    if !path.starts_with('/') || !path.contains("libc.so") {
      continue;
    }

    let candidate = PathBuf::from(path);

    if candidate.is_file() {
      return Some(candidate);
    }
  }

  None
}

fn loaded_shared_object_paths() -> Vec<PathBuf> {
  let Ok(maps) = fs::read_to_string("/proc/self/maps") else {
    return Vec::new();
  };
  let mut paths = Vec::new();

  for line in maps.lines() {
    let Some(path) = line.split_ascii_whitespace().last() else {
      continue;
    };

    if !path.starts_with('/') || !path.contains(".so") {
      continue;
    }

    let candidate = PathBuf::from(path);

    if !candidate.is_file() || paths.iter().any(|seen| seen == &candidate) {
      continue;
    }

    paths.push(candidate);
  }

  paths
}

fn open_host_handle_and_resolve_symbol(
  symbol: &'static [u8],
) -> Option<(*mut c_void, *mut c_void, PathBuf)> {
  clear_pending_dlerror();

  for path in loaded_shared_object_paths() {
    let shared_object = c_string_from_path(&path);

    // SAFETY: shared-object path is NUL-terminated and points to a loadable object.
    let handle = unsafe { dlopen(shared_object.as_ptr().cast::<c_char>(), RTLD_NOW) };

    if handle.is_null() {
      clear_pending_dlerror();
      continue;
    }

    // SAFETY: external handle and NUL-terminated symbol satisfy the dlsym C ABI contract.
    let resolved = unsafe { dlsym(handle, symbol_ptr(symbol)) };

    if !resolved.is_null() {
      clear_pending_dlerror();

      return Some((handle, resolved, path));
    }

    clear_pending_dlerror();
    assert_eq!(dlclose(handle), 0, "host handle should be closable");
  }

  None
}

fn assert_rtld_next_prefers_host_symbol(
  symbol: &'static [u8],
  label: &str,
  rlibc_symbol: Option<*mut c_void>,
  errno_value: c_int,
) {
  clear_pending_dlerror();
  write_errno(errno_value);

  let (host_handle, host_resolved, host_path) = open_host_handle_and_resolve_symbol(symbol)
    .unwrap_or_else(|| panic!("expected loaded host shared object exporting {label}"));

  // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy the dlsym ABI contract.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(symbol)) };

  assert!(!resolved.is_null(), "{label} lookup should succeed");
  assert_eq!(
    resolved,
    host_resolved,
    "RTLD_NEXT {label} should match host symbol resolution from {}",
    host_path.display(),
  );

  if let Some(rlibc_resolved) = rlibc_symbol {
    assert_ne!(
      resolved, rlibc_resolved,
      "RTLD_NEXT {label} should not stay pinned to rlibc when the host exports it",
    );
  }

  assert_eq!(
    read_errno(),
    errno_value,
    "successful dlsym must preserve errno"
  );
  assert!(
    take_dlerror_message().is_none(),
    "successful RTLD_NEXT lookup from clean state must not create dlerror",
  );
  assert_eq!(dlclose(host_handle), 0, "host handle should be closable");
}

fn take_dlerror_message() -> Option<String> {
  let message_ptr = dlerror();

  if message_ptr.is_null() {
    return None;
  }

  // SAFETY: `dlerror` returns either null or a valid NUL-terminated C string.
  let message = unsafe { CStr::from_ptr(message_ptr.cast_const()) };

  Some(message.to_string_lossy().into_owned())
}

fn clear_pending_dlerror() {
  while take_dlerror_message().is_some() {}
}

fn read_errno() -> c_int {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage for `errno`.
  unsafe { errno_ptr.read() }
}

fn write_errno(value: c_int) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns valid thread-local storage for `errno`.
  unsafe { errno_ptr.write(value) };
}

#[test]
fn dlsym_resolves_next_getenv_symbol() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  // SAFETY: `RTLD_NEXT` is a valid loader sentinel and the symbol string is NUL-terminated.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"getenv\0")) };

  assert!(
    !resolved.is_null(),
    "dlsym(RTLD_NEXT, getenv) must resolve on Linux/glibc",
  );
}

#[test]
fn dlsym_rtld_next_prefers_host_strlen_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  assert_rtld_next_prefers_host_symbol(
    b"strlen\0",
    "strlen",
    Some(rlibc_strlen as *const () as *mut c_void),
    3160,
  );
}

#[test]
fn dlsym_rtld_next_prefers_host_setenv_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  assert_rtld_next_prefers_host_symbol(
    b"setenv\0",
    "setenv",
    Some(rlibc_setenv as *const () as *mut c_void),
    3161,
  );
}

#[test]
fn dlsym_rtld_next_prefers_host_vfprintf_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  assert_rtld_next_prefers_host_symbol(
    b"vfprintf\0",
    "vfprintf",
    Some(
      (rlibc_vfprintf as unsafe extern "C" fn(*mut FILE, *const c_char, *mut c_void) -> c_int)
        as *const () as *mut c_void,
    ),
    3162,
  );
}

#[test]
fn dlsym_rtld_next_prefers_host_fflush_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  assert_rtld_next_prefers_host_symbol(
    b"fflush\0",
    "fflush",
    Some((rlibc_fflush as unsafe extern "C" fn(*mut FILE) -> c_int) as *const () as *mut c_void),
    3165,
  );
}

#[test]
fn dlsym_rtld_next_prefers_host_fileno_symbol_pointers() {
  let _serial = dlfcn_test_lock();
  let checks = [
    (
      b"fileno\0".as_slice(),
      (rlibc_fileno as unsafe extern "C" fn(*mut FILE) -> c_int) as *const () as *mut c_void,
      "fileno",
    ),
    (
      b"fileno_unlocked\0".as_slice(),
      (rlibc_fileno_unlocked as unsafe extern "C" fn(*mut FILE) -> c_int) as *const ()
        as *mut c_void,
      "fileno_unlocked",
    ),
  ];

  for (index, (symbol, expected_ptr, label)) in checks.into_iter().enumerate() {
    let errno_offset = c_int::try_from(index).unwrap_or_else(|_| unreachable!("index fits c_int"));

    assert_rtld_next_prefers_host_symbol(symbol, label, Some(expected_ptr), 3166 + errno_offset);
  }
}

#[test]
fn dlsym_rtld_next_prefers_host_file_stream_io_symbol_pointers() {
  let _serial = dlfcn_test_lock();
  let checks = [
    (
      b"tmpfile\0".as_slice(),
      (rlibc_tmpfile as unsafe extern "C" fn() -> *mut FILE) as *const () as *mut c_void,
      "tmpfile",
    ),
    (
      b"fopen\0".as_slice(),
      (rlibc_fopen as unsafe extern "C" fn(*const c_char, *const c_char) -> *mut FILE) as *const ()
        as *mut c_void,
      "fopen",
    ),
    (
      b"fread\0".as_slice(),
      (rlibc_fread as unsafe extern "C" fn(*mut c_void, size_t, size_t, *mut FILE) -> size_t)
        as *const () as *mut c_void,
      "fread",
    ),
    (
      b"fputs\0".as_slice(),
      (rlibc_fputs as unsafe extern "C" fn(*const c_char, *mut FILE) -> c_int) as *const ()
        as *mut c_void,
      "fputs",
    ),
  ];

  for (index, (symbol, expected_ptr, label)) in checks.into_iter().enumerate() {
    let errno_offset = c_int::try_from(index).unwrap_or_else(|_| unreachable!("index fits c_int"));

    assert_rtld_next_prefers_host_symbol(symbol, label, Some(expected_ptr), 3168 + errno_offset);
  }
}

#[test]
fn dlsym_rtld_next_prefers_host_buffering_wrapper_symbol_pointers() {
  let _serial = dlfcn_test_lock();
  let checks = [
    (
      b"setbuffer\0".as_slice(),
      (rlibc_setbuffer as unsafe extern "C" fn(*mut FILE, *mut c_char, size_t)) as *const ()
        as *mut c_void,
      "setbuffer",
    ),
    (
      b"setbuf\0".as_slice(),
      (rlibc_setbuf as unsafe extern "C" fn(*mut FILE, *mut c_char)) as *const () as *mut c_void,
      "setbuf",
    ),
    (
      b"setlinebuf\0".as_slice(),
      (rlibc_setlinebuf as unsafe extern "C" fn(*mut FILE)) as *const () as *mut c_void,
      "setlinebuf",
    ),
    (
      b"setvbuf\0".as_slice(),
      (rlibc_setvbuf as unsafe extern "C" fn(*mut FILE, *mut c_char, c_int, size_t) -> c_int)
        as *const () as *mut c_void,
      "setvbuf",
    ),
  ];

  for (index, (symbol, expected_ptr, label)) in checks.into_iter().enumerate() {
    let errno_offset = c_int::try_from(index).unwrap_or_else(|_| unreachable!("index fits c_int"));

    assert_rtld_next_prefers_host_symbol(symbol, label, Some(expected_ptr), 3166 + errno_offset);
  }
}

#[test]
fn dlsym_rtld_next_prefers_host_file_locking_symbol_pointers() {
  let _serial = dlfcn_test_lock();
  let checks = [
    (
      b"flockfile\0".as_slice(),
      (rlibc::stdio::flockfile as unsafe extern "C" fn(*mut FILE)) as *const () as *mut c_void,
      "flockfile",
    ),
    (
      b"ftrylockfile\0".as_slice(),
      (rlibc::stdio::ftrylockfile as unsafe extern "C" fn(*mut FILE) -> c_int) as *const ()
        as *mut c_void,
      "ftrylockfile",
    ),
    (
      b"funlockfile\0".as_slice(),
      (rlibc::stdio::funlockfile as unsafe extern "C" fn(*mut FILE)) as *const () as *mut c_void,
      "funlockfile",
    ),
  ];

  for (index, (symbol, rlibc_symbol, label)) in checks.into_iter().enumerate() {
    let Ok(errno_offset) = c_int::try_from(index) else {
      panic!("test errno offset should fit in c_int");
    };

    assert_rtld_next_prefers_host_symbol(symbol, label, Some(rlibc_symbol), 3168 + errno_offset);
  }
}

#[test]
fn dlsym_rtld_next_prefers_host_pthread_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  assert_rtld_next_prefers_host_symbol(
    b"pthread_create\0",
    "pthread_create",
    Some(
      (rlibc_pthread_create
        as unsafe extern "C" fn(
          *mut pthread_t,
          *const pthread_attr_t,
          Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
          *mut c_void,
        ) -> c_int) as *const () as *mut c_void,
    ),
    3163,
  );
  assert_rtld_next_prefers_host_symbol(
    b"pthread_join\0",
    "pthread_join",
    Some(
      (rlibc_pthread_join as unsafe extern "C" fn(pthread_t, *mut *mut c_void) -> c_int)
        as *const () as *mut c_void,
    ),
    3164,
  );
  assert_rtld_next_prefers_host_symbol(
    b"pthread_detach\0",
    "pthread_detach",
    Some((rlibc_pthread_detach as extern "C" fn(pthread_t) -> c_int) as *const () as *mut c_void),
    3165,
  );
}

#[test]
fn dlsym_rtld_next_prefers_host_dlfcn_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  assert_rtld_next_prefers_host_symbol(
    b"dlopen\0",
    "dlopen",
    Some(
      (dlopen as unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void) as *const ()
        as *mut c_void,
    ),
    3166,
  );
  assert_rtld_next_prefers_host_symbol(
    b"dlsym\0",
    "dlsym",
    Some(
      (dlsym as unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void) as *const ()
        as *mut c_void,
    ),
    3167,
  );
  assert_rtld_next_prefers_host_symbol(
    b"dlclose\0",
    "dlclose",
    Some((dlclose as unsafe extern "C" fn(*mut c_void) -> c_int) as *const () as *mut c_void),
    3168,
  );
  assert_rtld_next_prefers_host_symbol(
    b"dlerror\0",
    "dlerror",
    Some((dlerror as unsafe extern "C" fn() -> *mut c_char) as *const () as *mut c_void),
    3169,
  );
}

#[test]
fn dlsym_rtld_next_getenv_matches_host_symbol_and_leaves_dlerror_empty() {
  let _serial = dlfcn_test_lock();

  assert_rtld_next_prefers_host_symbol(
    b"getenv\0",
    "getenv",
    Some(
      (rlibc_getenv as unsafe extern "C" fn(*const c_char) -> *mut c_char) as *const ()
        as *mut c_void,
    ),
    3170,
  );
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_getenv_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3131);

  // SAFETY: RTLD_DEFAULT and symbol pointer satisfy dlsym contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"getenv\0")) };

  assert!(!resolved.is_null(), "getenv symbol lookup should succeed");

  let expected = rlibc_getenv as unsafe extern "C" fn(*const c_char) -> *mut c_char;
  let expected_ptr = expected as *const () as *mut c_void;

  assert_eq!(
    resolved, expected_ptr,
    "RTLD_DEFAULT getenv should resolve to rlibc getenv implementation",
  );
  assert_eq!(read_errno(), 3131, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_setenv_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3132);

  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"setenv\0")) };

  assert!(!resolved.is_null(), "setenv symbol lookup should succeed");

  let expected = rlibc_setenv as unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> c_int;
  let expected_ptr = expected as *const () as *mut c_void;

  assert_eq!(
    resolved, expected_ptr,
    "RTLD_DEFAULT setenv should resolve to rlibc setenv implementation",
  );
  assert_eq!(read_errno(), 3132, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_malloc_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3133);

  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"malloc\0")) };

  assert!(!resolved.is_null(), "malloc symbol lookup should succeed");

  let expected = rlibc_malloc as unsafe extern "C" fn(usize) -> *mut c_void;
  let expected_ptr = expected as *const () as *mut c_void;

  assert_eq!(
    resolved, expected_ptr,
    "RTLD_DEFAULT malloc should resolve to rlibc malloc implementation",
  );
  assert_eq!(read_errno(), 3133, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_next_prefers_host_malloc_symbol_pointer_over_rlibc_default() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = c_string_from_path(&libc_path);

  // SAFETY: `libc_cstr` is a valid NUL-terminated shared-object path.
  let host_handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !host_handle.is_null(),
    "dlopen should return a host handle for libc: {}",
    libc_path.display(),
  );

  write_errno(3137);

  // SAFETY: `RTLD_NEXT` and symbol pointer satisfy the dlsym C ABI contract.
  let from_next = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"malloc\0")) };
  // SAFETY: external handle and symbol pointer satisfy the dlsym C ABI contract.
  let from_host = unsafe { dlsym(host_handle, symbol_ptr(b"malloc\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy the dlsym C ABI contract.
  let from_default = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"malloc\0")) };

  assert!(
    !from_next.is_null(),
    "RTLD_NEXT malloc lookup should succeed"
  );
  assert!(
    !from_host.is_null(),
    "host-handle malloc lookup should succeed"
  );
  assert!(
    !from_default.is_null(),
    "RTLD_DEFAULT malloc lookup should succeed"
  );
  assert_eq!(
    from_default,
    (rlibc_malloc as unsafe extern "C" fn(usize) -> *mut c_void) as *const () as *mut c_void,
    "RTLD_DEFAULT malloc should resolve to rlibc malloc implementation",
  );
  assert_eq!(
    from_next, from_host,
    "RTLD_NEXT malloc should prefer the host next-definition when available",
  );
  assert_ne!(
    from_next, from_default,
    "RTLD_NEXT malloc should stay distinct from RTLD_DEFAULT rlibc malloc",
  );
  assert_eq!(read_errno(), 3137, "successful dlsym must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "successful RTLD_NEXT host-first lookup from clean state must not create dlerror",
  );
  assert_eq!(dlclose(host_handle), 0, "host handle should be closable");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_errno_location_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3134);

  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"__errno_location\0")) };

  assert!(
    !resolved.is_null(),
    "__errno_location symbol lookup should succeed"
  );

  let expected = __errno_location as extern "C" fn() -> *mut c_int;
  let expected_ptr = expected as *const () as *mut c_void;

  assert_eq!(
    resolved, expected_ptr,
    "RTLD_DEFAULT __errno_location should resolve to rlibc errno implementation",
  );
  assert_eq!(read_errno(), 3134, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_fflush_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3135);

  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"fflush\0")) };

  assert!(!resolved.is_null(), "fflush symbol lookup should succeed");

  let expected = rlibc_fflush as unsafe extern "C" fn(*mut FILE) -> c_int;
  let expected_ptr = expected as *const () as *mut c_void;

  assert_eq!(
    resolved, expected_ptr,
    "RTLD_DEFAULT fflush should resolve to rlibc fflush implementation",
  );
  assert_eq!(read_errno(), 3135, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_fileno_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3136);

  let checks = [
    (
      b"fileno\0".as_slice(),
      (rlibc_fileno as unsafe extern "C" fn(*mut FILE) -> c_int) as *const () as *mut c_void,
      "fileno",
    ),
    (
      b"fileno_unlocked\0".as_slice(),
      (rlibc_fileno_unlocked as unsafe extern "C" fn(*mut FILE) -> c_int) as *const ()
        as *mut c_void,
      "fileno_unlocked",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} symbol lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3136, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_host_backed_file_stream_io_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3137);

  let checks = [
    (
      b"tmpfile\0".as_slice(),
      (rlibc_tmpfile as unsafe extern "C" fn() -> *mut FILE) as *const () as *mut c_void,
      "tmpfile",
    ),
    (
      b"fopen\0".as_slice(),
      (rlibc_fopen as unsafe extern "C" fn(*const c_char, *const c_char) -> *mut FILE) as *const ()
        as *mut c_void,
      "fopen",
    ),
    (
      b"fread\0".as_slice(),
      (rlibc_fread as unsafe extern "C" fn(*mut c_void, size_t, size_t, *mut FILE) -> size_t)
        as *const () as *mut c_void,
      "fread",
    ),
    (
      b"fputs\0".as_slice(),
      (rlibc_fputs as unsafe extern "C" fn(*const c_char, *mut FILE) -> c_int) as *const ()
        as *mut c_void,
      "fputs",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} symbol lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3137, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_buffering_wrapper_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3137);

  let checks = [
    (
      b"setbuffer\0".as_slice(),
      (rlibc_setbuffer as unsafe extern "C" fn(*mut FILE, *mut c_char, size_t)) as *const ()
        as *mut c_void,
      "setbuffer",
    ),
    (
      b"setbuf\0".as_slice(),
      (rlibc_setbuf as unsafe extern "C" fn(*mut FILE, *mut c_char)) as *const () as *mut c_void,
      "setbuf",
    ),
    (
      b"setlinebuf\0".as_slice(),
      (rlibc_setlinebuf as unsafe extern "C" fn(*mut FILE)) as *const () as *mut c_void,
      "setlinebuf",
    ),
    (
      b"setvbuf\0".as_slice(),
      (rlibc_setvbuf as unsafe extern "C" fn(*mut FILE, *mut c_char, c_int, size_t) -> c_int)
        as *const () as *mut c_void,
      "setvbuf",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} symbol lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3137, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_vsnprintf_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3137);

  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"vsnprintf\0")) };

  assert!(
    !resolved.is_null(),
    "vsnprintf symbol lookup should succeed"
  );

  let expected = rlibc_vsnprintf
    as unsafe extern "C" fn(*mut c_char, size_t, *const c_char, *mut c_void) -> c_int;
  let expected_ptr = expected as *const () as *mut c_void;

  assert_eq!(
    resolved, expected_ptr,
    "RTLD_DEFAULT vsnprintf should resolve to rlibc vsnprintf implementation",
  );
  assert_eq!(read_errno(), 3137, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_vfprintf_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3138);

  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"vfprintf\0")) };

  assert!(!resolved.is_null(), "vfprintf symbol lookup should succeed");

  let expected =
    rlibc_vfprintf as unsafe extern "C" fn(*mut FILE, *const c_char, *mut c_void) -> c_int;
  let expected_ptr = expected as *const () as *mut c_void;

  assert_eq!(
    resolved, expected_ptr,
    "RTLD_DEFAULT vfprintf should resolve to rlibc vfprintf implementation",
  );
  assert_eq!(read_errno(), 3138, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_vprintf_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3139);

  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"vprintf\0")) };

  assert!(!resolved.is_null(), "vprintf symbol lookup should succeed");

  let expected = rlibc_vprintf as unsafe extern "C" fn(*const c_char, *mut c_void) -> c_int;
  let expected_ptr = expected as *const () as *mut c_void;

  assert_eq!(
    resolved, expected_ptr,
    "RTLD_DEFAULT vprintf should resolve to rlibc vprintf implementation",
  );
  assert_eq!(read_errno(), 3139, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_pthread_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3140);

  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let create_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"pthread_create\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let join_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"pthread_join\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let detach_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"pthread_detach\0")) };

  assert!(
    !create_resolved.is_null(),
    "pthread_create lookup should succeed"
  );
  assert!(
    !join_resolved.is_null(),
    "pthread_join lookup should succeed"
  );
  assert!(
    !detach_resolved.is_null(),
    "pthread_detach lookup should succeed"
  );

  let expected_create = rlibc_pthread_create
    as unsafe extern "C" fn(
      *mut pthread_t,
      *const pthread_attr_t,
      Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
      *mut c_void,
    ) -> c_int;
  let expected_join =
    rlibc_pthread_join as unsafe extern "C" fn(pthread_t, *mut *mut c_void) -> c_int;
  let expected_detach = rlibc_pthread_detach as extern "C" fn(pthread_t) -> c_int;

  assert_eq!(
    create_resolved, expected_create as *const () as *mut c_void,
    "RTLD_DEFAULT pthread_create should resolve to rlibc pthread_create implementation",
  );
  assert_eq!(
    join_resolved, expected_join as *const () as *mut c_void,
    "RTLD_DEFAULT pthread_join should resolve to rlibc pthread_join implementation",
  );
  assert_eq!(
    detach_resolved, expected_detach as *const () as *mut c_void,
    "RTLD_DEFAULT pthread_detach should resolve to rlibc pthread_detach implementation",
  );
  assert_eq!(read_errno(), 3140, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_dlfcn_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3141);

  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let dlopen_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"dlopen\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let dlsym_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"dlsym\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let dlclose_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"dlclose\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let dlerror_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"dlerror\0")) };

  assert!(!dlopen_resolved.is_null(), "dlopen lookup should succeed");
  assert!(!dlsym_resolved.is_null(), "dlsym lookup should succeed");
  assert!(!dlclose_resolved.is_null(), "dlclose lookup should succeed");
  assert!(!dlerror_resolved.is_null(), "dlerror lookup should succeed");

  let expected_dlopen = dlopen as unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;
  let expected_dlsym = dlsym as unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
  let expected_dlclose = dlclose as extern "C" fn(*mut c_void) -> c_int;
  let expected_dlerror = dlerror as extern "C" fn() -> *mut c_char;

  assert_eq!(
    dlopen_resolved, expected_dlopen as *const () as *mut c_void,
    "RTLD_DEFAULT dlopen should resolve to rlibc dlopen implementation",
  );
  assert_eq!(
    dlsym_resolved, expected_dlsym as *const () as *mut c_void,
    "RTLD_DEFAULT dlsym should resolve to rlibc dlsym implementation",
  );
  assert_eq!(
    dlclose_resolved, expected_dlclose as *const () as *mut c_void,
    "RTLD_DEFAULT dlclose should resolve to rlibc dlclose implementation",
  );
  assert_eq!(
    dlerror_resolved, expected_dlerror as *const () as *mut c_void,
    "RTLD_DEFAULT dlerror should resolve to rlibc dlerror implementation",
  );
  assert_eq!(read_errno(), 3141, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_string_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3142);

  let checks = [
    (
      b"strlen\0".as_slice(),
      (rlibc_strlen as unsafe extern "C" fn(*const c_char) -> usize) as *const () as *mut c_void,
      "strlen",
    ),
    (
      b"strnlen\0".as_slice(),
      (rlibc_strnlen as unsafe extern "C" fn(*const c_char, usize) -> usize) as *const ()
        as *mut c_void,
      "strnlen",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3142, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_memory_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3143);

  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let memcmp_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"memcmp\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let memcpy_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"memcpy\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let memmove_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"memmove\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
  let memset_resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"memset\0")) };

  assert!(!memcmp_resolved.is_null(), "memcmp lookup should succeed");
  assert!(!memcpy_resolved.is_null(), "memcpy lookup should succeed");
  assert!(!memmove_resolved.is_null(), "memmove lookup should succeed");
  assert!(!memset_resolved.is_null(), "memset lookup should succeed");

  let expected_memcmp =
    rlibc_memcmp as unsafe extern "C" fn(*const c_void, *const c_void, size_t) -> c_int;
  let expected_memcpy =
    rlibc_memcpy as unsafe extern "C" fn(*mut c_void, *const c_void, size_t) -> *mut c_void;
  let expected_memmove =
    rlibc_memmove as unsafe extern "C" fn(*mut c_void, *const c_void, size_t) -> *mut c_void;
  let expected_memset =
    rlibc_memset as unsafe extern "C" fn(*mut c_void, c_int, size_t) -> *mut c_void;

  assert_eq!(
    memcmp_resolved, expected_memcmp as *const () as *mut c_void,
    "RTLD_DEFAULT memcmp should resolve to rlibc memcmp implementation",
  );
  assert_eq!(
    memcpy_resolved, expected_memcpy as *const () as *mut c_void,
    "RTLD_DEFAULT memcpy should resolve to rlibc memcpy implementation",
  );
  assert_eq!(
    memmove_resolved, expected_memmove as *const () as *mut c_void,
    "RTLD_DEFAULT memmove should resolve to rlibc memmove implementation",
  );
  assert_eq!(
    memset_resolved, expected_memset as *const () as *mut c_void,
    "RTLD_DEFAULT memset should resolve to rlibc memset implementation",
  );
  assert_eq!(read_errno(), 3143, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_ctype_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3144);

  let to_symbol_ptr = |function: extern "C" fn(c_int) -> c_int| -> *mut c_void {
    function as *const () as *mut c_void
  };
  let checks = [
    (
      b"isalpha\0".as_slice(),
      to_symbol_ptr(rlibc_isalpha),
      "isalpha",
    ),
    (
      b"isdigit\0".as_slice(),
      to_symbol_ptr(rlibc_isdigit),
      "isdigit",
    ),
    (
      b"isalnum\0".as_slice(),
      to_symbol_ptr(rlibc_isalnum),
      "isalnum",
    ),
    (
      b"islower\0".as_slice(),
      to_symbol_ptr(rlibc_islower),
      "islower",
    ),
    (
      b"isupper\0".as_slice(),
      to_symbol_ptr(rlibc_isupper),
      "isupper",
    ),
    (
      b"isxdigit\0".as_slice(),
      to_symbol_ptr(rlibc_isxdigit),
      "isxdigit",
    ),
    (
      b"isblank\0".as_slice(),
      to_symbol_ptr(rlibc_isblank),
      "isblank",
    ),
    (
      b"isspace\0".as_slice(),
      to_symbol_ptr(rlibc_isspace),
      "isspace",
    ),
    (
      b"iscntrl\0".as_slice(),
      to_symbol_ptr(rlibc_iscntrl),
      "iscntrl",
    ),
    (
      b"isprint\0".as_slice(),
      to_symbol_ptr(rlibc_isprint),
      "isprint",
    ),
    (
      b"isgraph\0".as_slice(),
      to_symbol_ptr(rlibc_isgraph),
      "isgraph",
    ),
    (
      b"ispunct\0".as_slice(),
      to_symbol_ptr(rlibc_ispunct),
      "ispunct",
    ),
    (
      b"tolower\0".as_slice(),
      to_symbol_ptr(rlibc_tolower),
      "tolower",
    ),
    (
      b"toupper\0".as_slice(),
      to_symbol_ptr(rlibc_toupper),
      "toupper",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3144, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_numeric_conversion_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3145);

  let checks = [
    (
      b"atoi\0".as_slice(),
      (rlibc_atoi as unsafe extern "C" fn(*const c_char) -> c_int) as *const () as *mut c_void,
      "atoi",
    ),
    (
      b"atol\0".as_slice(),
      (rlibc_atol as unsafe extern "C" fn(*const c_char) -> c_long) as *const () as *mut c_void,
      "atol",
    ),
    (
      b"atoll\0".as_slice(),
      (rlibc_atoll as unsafe extern "C" fn(*const c_char) -> c_longlong) as *const ()
        as *mut c_void,
      "atoll",
    ),
    (
      b"strtol\0".as_slice(),
      (rlibc_strtol as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_long)
        as *const () as *mut c_void,
      "strtol",
    ),
    (
      b"strtoll\0".as_slice(),
      (rlibc_strtoll as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_longlong)
        as *const () as *mut c_void,
      "strtoll",
    ),
    (
      b"strtoul\0".as_slice(),
      (rlibc_strtoul as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_ulong)
        as *const () as *mut c_void,
      "strtoul",
    ),
    (
      b"strtoull\0".as_slice(),
      (rlibc_strtoull
        as unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> c_ulonglong)
        as *const () as *mut c_void,
      "strtoull",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3145, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_unistd_io_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3146);

  let checks = [
    (
      b"close\0".as_slice(),
      (rlibc_close as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "close",
    ),
    (
      b"access\0".as_slice(),
      (rlibc_access as unsafe extern "C" fn(*const c_char, c_int) -> c_int) as *const ()
        as *mut c_void,
      "access",
    ),
    (
      b"dup\0".as_slice(),
      (rlibc_dup as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "dup",
    ),
    (
      b"dup2\0".as_slice(),
      (rlibc_dup2 as extern "C" fn(c_int, c_int) -> c_int) as *const () as *mut c_void,
      "dup2",
    ),
    (
      b"dup3\0".as_slice(),
      (rlibc_dup3 as extern "C" fn(c_int, c_int, c_int) -> c_int) as *const () as *mut c_void,
      "dup3",
    ),
    (
      b"getpid\0".as_slice(),
      (rlibc_getpid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "getpid",
    ),
    (
      b"getppid\0".as_slice(),
      (rlibc_getppid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "getppid",
    ),
    (
      b"getpgrp\0".as_slice(),
      (rlibc_getpgrp as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "getpgrp",
    ),
    (
      b"getpgid\0".as_slice(),
      (rlibc_getpgid as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "getpgid",
    ),
    (
      b"getsid\0".as_slice(),
      (rlibc_getsid as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "getsid",
    ),
    (
      b"gettid\0".as_slice(),
      (rlibc_gettid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "gettid",
    ),
    (
      b"getuid\0".as_slice(),
      (rlibc_getuid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "getuid",
    ),
    (
      b"geteuid\0".as_slice(),
      (rlibc_geteuid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "geteuid",
    ),
    (
      b"getgid\0".as_slice(),
      (rlibc_getgid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "getgid",
    ),
    (
      b"getegid\0".as_slice(),
      (rlibc_getegid as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "getegid",
    ),
    (
      b"lseek\0".as_slice(),
      (rlibc_lseek as extern "C" fn(c_int, c_long, c_int) -> c_long) as *const () as *mut c_void,
      "lseek",
    ),
    (
      b"isatty\0".as_slice(),
      (rlibc_isatty as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "isatty",
    ),
    (
      b"read\0".as_slice(),
      (rlibc_read as unsafe extern "C" fn(c_int, *mut c_void, size_t) -> ssize_t) as *const ()
        as *mut c_void,
      "read",
    ),
    (
      b"write\0".as_slice(),
      (rlibc_write as unsafe extern "C" fn(c_int, *const c_void, size_t) -> ssize_t) as *const ()
        as *mut c_void,
      "write",
    ),
    (
      b"send\0".as_slice(),
      (rlibc_send as unsafe extern "C" fn(c_int, *const c_void, size_t, c_int) -> ssize_t)
        as *const () as *mut c_void,
      "send",
    ),
    (
      b"recv\0".as_slice(),
      (rlibc_recv as unsafe extern "C" fn(c_int, *mut c_void, size_t, c_int) -> ssize_t)
        as *const () as *mut c_void,
      "recv",
    ),
    (
      b"fdatasync\0".as_slice(),
      (rlibc_fdatasync as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "fdatasync",
    ),
    (
      b"fsync\0".as_slice(),
      (rlibc_fsync as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "fsync",
    ),
    (
      b"sync\0".as_slice(),
      (rlibc_sync as extern "C" fn()) as *const () as *mut c_void,
      "sync",
    ),
    (
      b"syncfs\0".as_slice(),
      (rlibc_syncfs as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "syncfs",
    ),
    (
      b"unlink\0".as_slice(),
      (rlibc_unlink as unsafe extern "C" fn(*const c_char) -> c_int) as *const () as *mut c_void,
      "unlink",
    ),
    (
      b"pipe\0".as_slice(),
      (rlibc_pipe as unsafe extern "C" fn(*mut c_int) -> c_int) as *const () as *mut c_void,
      "pipe",
    ),
    (
      b"pipe2\0".as_slice(),
      (rlibc_pipe2 as unsafe extern "C" fn(*mut c_int, c_int) -> c_int) as *const () as *mut c_void,
      "pipe2",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3146, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_unistd_open_and_fs_stat_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3147);

  let checks = [
    (
      b"open\0".as_slice(),
      (rlibc_open as unsafe extern "C" fn(*const c_char, c_int, c_uint) -> c_int) as *const ()
        as *mut c_void,
      "open",
    ),
    (
      b"openat\0".as_slice(),
      (rlibc_openat as unsafe extern "C" fn(c_int, *const c_char, c_int, c_uint) -> c_int)
        as *const () as *mut c_void,
      "openat",
    ),
    (
      b"fstat\0".as_slice(),
      (rlibc_fstat as unsafe extern "C" fn(c_int, *mut RlibcStat) -> c_int) as *const ()
        as *mut c_void,
      "fstat",
    ),
    (
      b"fstatat\0".as_slice(),
      (rlibc_fstatat as unsafe extern "C" fn(c_int, *const c_char, *mut RlibcStat, c_int) -> c_int)
        as *const () as *mut c_void,
      "fstatat",
    ),
    (
      b"stat\0".as_slice(),
      (rlibc_stat as unsafe extern "C" fn(*const c_char, *mut RlibcStat) -> c_int) as *const ()
        as *mut c_void,
      "stat",
    ),
    (
      b"lstat\0".as_slice(),
      (rlibc_lstat as unsafe extern "C" fn(*const c_char, *mut RlibcStat) -> c_int) as *const ()
        as *mut c_void,
      "lstat",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3147, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_system_resource_time_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3148);

  let checks = [
    (
      b"uname\0".as_slice(),
      (rlibc_uname as unsafe extern "C" fn(*mut RlibcUtsName) -> c_int) as *const () as *mut c_void,
      "uname",
    ),
    (
      b"gethostname\0".as_slice(),
      (rlibc_gethostname as unsafe extern "C" fn(*mut c_char, size_t) -> c_int) as *const ()
        as *mut c_void,
      "gethostname",
    ),
    (
      b"getpagesize\0".as_slice(),
      (rlibc_getpagesize as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "getpagesize",
    ),
    (
      b"sysinfo\0".as_slice(),
      (rlibc_sysinfo as unsafe extern "C" fn(*mut RlibcSysInfo) -> c_int) as *const ()
        as *mut c_void,
      "sysinfo",
    ),
    (
      b"sysconf\0".as_slice(),
      (rlibc_sysconf as extern "C" fn(c_int) -> c_long) as *const () as *mut c_void,
      "sysconf",
    ),
    (
      b"prlimit64\0".as_slice(),
      (rlibc_prlimit64
        as unsafe extern "C" fn(c_int, c_int, *const RlibcRLimit, *mut RlibcRLimit) -> c_int)
        as *const () as *mut c_void,
      "prlimit64",
    ),
    (
      b"getrlimit\0".as_slice(),
      (rlibc_getrlimit as unsafe extern "C" fn(c_int, *mut RlibcRLimit) -> c_int) as *const ()
        as *mut c_void,
      "getrlimit",
    ),
    (
      b"setrlimit\0".as_slice(),
      (rlibc_setrlimit as unsafe extern "C" fn(c_int, *const RlibcRLimit) -> c_int) as *const ()
        as *mut c_void,
      "setrlimit",
    ),
    (
      b"clock_gettime\0".as_slice(),
      (rlibc_clock_gettime as extern "C" fn(rlibc_clockid_t, *mut RlibcTimespec) -> c_int)
        as *const () as *mut c_void,
      "clock_gettime",
    ),
    (
      b"gettimeofday\0".as_slice(),
      (rlibc_gettimeofday as unsafe extern "C" fn(*mut RlibcTimeval, *mut RlibcTimezone) -> c_int)
        as *const () as *mut c_void,
      "gettimeofday",
    ),
    (
      b"strftime\0".as_slice(),
      (rlibc_strftime
        as unsafe extern "C" fn(*mut c_char, size_t, *const c_char, *const RlibcTm) -> size_t)
        as *const () as *mut c_void,
      "strftime",
    ),
    (
      b"gmtime\0".as_slice(),
      (rlibc_gmtime as unsafe extern "C" fn(*const rlibc_time_t) -> *mut RlibcTm) as *const ()
        as *mut c_void,
      "gmtime",
    ),
    (
      b"gmtime_r\0".as_slice(),
      (rlibc_gmtime_r as unsafe extern "C" fn(*const rlibc_time_t, *mut RlibcTm) -> *mut RlibcTm)
        as *const () as *mut c_void,
      "gmtime_r",
    ),
    (
      b"localtime\0".as_slice(),
      (rlibc_localtime as unsafe extern "C" fn(*const rlibc_time_t) -> *mut RlibcTm) as *const ()
        as *mut c_void,
      "localtime",
    ),
    (
      b"localtime_r\0".as_slice(),
      (rlibc_localtime_r as unsafe extern "C" fn(*const rlibc_time_t, *mut RlibcTm) -> *mut RlibcTm)
        as *const () as *mut c_void,
      "localtime_r",
    ),
    (
      b"timegm\0".as_slice(),
      (rlibc_timegm as unsafe extern "C" fn(*mut RlibcTm) -> rlibc_time_t) as *const ()
        as *mut c_void,
      "timegm",
    ),
    (
      b"mktime\0".as_slice(),
      (rlibc_mktime as unsafe extern "C" fn(*mut RlibcTm) -> rlibc_time_t) as *const ()
        as *mut c_void,
      "mktime",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3148, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_netdb_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3149);

  let checks = [
    (
      b"getaddrinfo\0".as_slice(),
      (rlibc_getaddrinfo
        as unsafe extern "C" fn(
          *const c_char,
          *const c_char,
          *const RlibcAddrInfo,
          *mut *mut RlibcAddrInfo,
        ) -> c_int) as *const () as *mut c_void,
      "getaddrinfo",
    ),
    (
      b"getnameinfo\0".as_slice(),
      (rlibc_getnameinfo
        as unsafe extern "C" fn(
          *const RlibcSockAddr,
          rlibc_socklen_t,
          *mut c_char,
          rlibc_socklen_t,
          *mut c_char,
          rlibc_socklen_t,
          c_int,
        ) -> c_int) as *const () as *mut c_void,
      "getnameinfo",
    ),
    (
      b"freeaddrinfo\0".as_slice(),
      (rlibc_freeaddrinfo as unsafe extern "C" fn(*mut RlibcAddrInfo)) as *const () as *mut c_void,
      "freeaddrinfo",
    ),
    (
      b"gai_strerror\0".as_slice(),
      (rlibc_gai_strerror as extern "C" fn(c_int) -> *const c_char) as *const () as *mut c_void,
      "gai_strerror",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3149, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_socket_core_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3150);

  let checks = [
    (
      b"socket\0".as_slice(),
      (rlibc_socket as unsafe extern "C" fn(c_int, c_int, c_int) -> c_int) as *const ()
        as *mut c_void,
      "socket",
    ),
    (
      b"connect\0".as_slice(),
      (rlibc_connect
        as unsafe extern "C" fn(c_int, *const RlibcSockaddrCore, RlibcSocklenTCore) -> c_int)
        as *const () as *mut c_void,
      "connect",
    ),
    (
      b"bind\0".as_slice(),
      (rlibc_bind
        as unsafe extern "C" fn(c_int, *const RlibcSockaddrCore, RlibcSocklenTCore) -> c_int)
        as *const () as *mut c_void,
      "bind",
    ),
    (
      b"listen\0".as_slice(),
      (rlibc_listen as unsafe extern "C" fn(c_int, c_int) -> c_int) as *const () as *mut c_void,
      "listen",
    ),
    (
      b"accept\0".as_slice(),
      (rlibc_accept
        as unsafe extern "C" fn(c_int, *mut RlibcSockaddrCore, *mut RlibcSocklenTCore) -> c_int)
        as *const () as *mut c_void,
      "accept",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3150, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_pthread_sync_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3151);

  let checks = [
    (
      b"pthread_mutexattr_init\0".as_slice(),
      (rlibc_pthread_mutexattr_init as extern "C" fn(*mut pthread_mutexattr_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_mutexattr_init",
    ),
    (
      b"pthread_mutexattr_destroy\0".as_slice(),
      (rlibc_pthread_mutexattr_destroy as extern "C" fn(*mut pthread_mutexattr_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_mutexattr_destroy",
    ),
    (
      b"pthread_mutexattr_gettype\0".as_slice(),
      (rlibc_pthread_mutexattr_gettype
        as extern "C" fn(*const pthread_mutexattr_t, *mut c_int) -> c_int) as *const ()
        as *mut c_void,
      "pthread_mutexattr_gettype",
    ),
    (
      b"pthread_mutexattr_settype\0".as_slice(),
      (rlibc_pthread_mutexattr_settype as extern "C" fn(*mut pthread_mutexattr_t, c_int) -> c_int)
        as *const () as *mut c_void,
      "pthread_mutexattr_settype",
    ),
    (
      b"pthread_mutexattr_getpshared\0".as_slice(),
      (rlibc_pthread_mutexattr_getpshared
        as extern "C" fn(*const pthread_mutexattr_t, *mut c_int) -> c_int) as *const ()
        as *mut c_void,
      "pthread_mutexattr_getpshared",
    ),
    (
      b"pthread_mutexattr_setpshared\0".as_slice(),
      (rlibc_pthread_mutexattr_setpshared
        as extern "C" fn(*mut pthread_mutexattr_t, c_int) -> c_int) as *const ()
        as *mut c_void,
      "pthread_mutexattr_setpshared",
    ),
    (
      b"pthread_mutex_init\0".as_slice(),
      (rlibc_pthread_mutex_init
        as extern "C" fn(*mut pthread_mutex_t, *const pthread_mutexattr_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_mutex_init",
    ),
    (
      b"pthread_mutex_destroy\0".as_slice(),
      (rlibc_pthread_mutex_destroy as extern "C" fn(*mut pthread_mutex_t) -> c_int) as *const ()
        as *mut c_void,
      "pthread_mutex_destroy",
    ),
    (
      b"pthread_mutex_lock\0".as_slice(),
      (rlibc_pthread_mutex_lock as extern "C" fn(*mut pthread_mutex_t) -> c_int) as *const ()
        as *mut c_void,
      "pthread_mutex_lock",
    ),
    (
      b"pthread_mutex_trylock\0".as_slice(),
      (rlibc_pthread_mutex_trylock as extern "C" fn(*mut pthread_mutex_t) -> c_int) as *const ()
        as *mut c_void,
      "pthread_mutex_trylock",
    ),
    (
      b"pthread_mutex_unlock\0".as_slice(),
      (rlibc_pthread_mutex_unlock as extern "C" fn(*mut pthread_mutex_t) -> c_int) as *const ()
        as *mut c_void,
      "pthread_mutex_unlock",
    ),
    (
      b"pthread_condattr_init\0".as_slice(),
      (rlibc_pthread_condattr_init as extern "C" fn(*mut pthread_condattr_t) -> c_int) as *const ()
        as *mut c_void,
      "pthread_condattr_init",
    ),
    (
      b"pthread_condattr_destroy\0".as_slice(),
      (rlibc_pthread_condattr_destroy as extern "C" fn(*mut pthread_condattr_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_condattr_destroy",
    ),
    (
      b"pthread_condattr_getpshared\0".as_slice(),
      (rlibc_pthread_condattr_getpshared
        as extern "C" fn(*const pthread_condattr_t, *mut c_int) -> c_int) as *const ()
        as *mut c_void,
      "pthread_condattr_getpshared",
    ),
    (
      b"pthread_condattr_setpshared\0".as_slice(),
      (rlibc_pthread_condattr_setpshared as extern "C" fn(*mut pthread_condattr_t, c_int) -> c_int)
        as *const () as *mut c_void,
      "pthread_condattr_setpshared",
    ),
    (
      b"pthread_cond_init\0".as_slice(),
      (rlibc_pthread_cond_init
        as extern "C" fn(*mut pthread_cond_t, *const pthread_condattr_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_cond_init",
    ),
    (
      b"pthread_cond_destroy\0".as_slice(),
      (rlibc_pthread_cond_destroy as extern "C" fn(*mut pthread_cond_t) -> c_int) as *const ()
        as *mut c_void,
      "pthread_cond_destroy",
    ),
    (
      b"pthread_cond_wait\0".as_slice(),
      (rlibc_pthread_cond_wait as extern "C" fn(*mut pthread_cond_t, *mut pthread_mutex_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_cond_wait",
    ),
    (
      b"pthread_cond_timedwait\0".as_slice(),
      (rlibc_pthread_cond_timedwait
        as extern "C" fn(*mut pthread_cond_t, *mut pthread_mutex_t, *const RlibcTimespec) -> c_int)
        as *const () as *mut c_void,
      "pthread_cond_timedwait",
    ),
    (
      b"pthread_cond_signal\0".as_slice(),
      (rlibc_pthread_cond_signal as extern "C" fn(*mut pthread_cond_t) -> c_int) as *const ()
        as *mut c_void,
      "pthread_cond_signal",
    ),
    (
      b"pthread_cond_broadcast\0".as_slice(),
      (rlibc_pthread_cond_broadcast as extern "C" fn(*mut pthread_cond_t) -> c_int) as *const ()
        as *mut c_void,
      "pthread_cond_broadcast",
    ),
    (
      b"pthread_rwlock_init\0".as_slice(),
      (rlibc_pthread_rwlock_init
        as unsafe extern "C" fn(*mut pthread_rwlock_t, *const pthread_rwlockattr_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_rwlock_init",
    ),
    (
      b"pthread_rwlock_destroy\0".as_slice(),
      (rlibc_pthread_rwlock_destroy as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_rwlock_destroy",
    ),
    (
      b"pthread_rwlock_rdlock\0".as_slice(),
      (rlibc_pthread_rwlock_rdlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_rwlock_rdlock",
    ),
    (
      b"pthread_rwlock_tryrdlock\0".as_slice(),
      (rlibc_pthread_rwlock_tryrdlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_rwlock_tryrdlock",
    ),
    (
      b"pthread_rwlock_wrlock\0".as_slice(),
      (rlibc_pthread_rwlock_wrlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_rwlock_wrlock",
    ),
    (
      b"pthread_rwlock_trywrlock\0".as_slice(),
      (rlibc_pthread_rwlock_trywrlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_rwlock_trywrlock",
    ),
    (
      b"pthread_rwlock_unlock\0".as_slice(),
      (rlibc_pthread_rwlock_unlock as unsafe extern "C" fn(*mut pthread_rwlock_t) -> c_int)
        as *const () as *mut c_void,
      "pthread_rwlock_unlock",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3151, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_signal_and_misc_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3152);

  let checks = [
    (
      b"sigemptyset\0".as_slice(),
      (rlibc_sigemptyset as unsafe extern "C" fn(*mut RlibcSigSet) -> c_int) as *const ()
        as *mut c_void,
      "sigemptyset",
    ),
    (
      b"sigfillset\0".as_slice(),
      (rlibc_sigfillset as unsafe extern "C" fn(*mut RlibcSigSet) -> c_int) as *const ()
        as *mut c_void,
      "sigfillset",
    ),
    (
      b"sigaddset\0".as_slice(),
      (rlibc_sigaddset as unsafe extern "C" fn(*mut RlibcSigSet, c_int) -> c_int) as *const ()
        as *mut c_void,
      "sigaddset",
    ),
    (
      b"sigdelset\0".as_slice(),
      (rlibc_sigdelset as unsafe extern "C" fn(*mut RlibcSigSet, c_int) -> c_int) as *const ()
        as *mut c_void,
      "sigdelset",
    ),
    (
      b"sigismember\0".as_slice(),
      (rlibc_sigismember as unsafe extern "C" fn(*const RlibcSigSet, c_int) -> c_int) as *const ()
        as *mut c_void,
      "sigismember",
    ),
    (
      b"sigaction\0".as_slice(),
      (rlibc_sigaction
        as unsafe extern "C" fn(c_int, *const RlibcSigAction, *mut RlibcSigAction) -> c_int)
        as *const () as *mut c_void,
      "sigaction",
    ),
    (
      b"kill\0".as_slice(),
      (rlibc_kill as extern "C" fn(c_int, c_int) -> c_int) as *const () as *mut c_void,
      "kill",
    ),
    (
      b"raise\0".as_slice(),
      (rlibc_raise as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "raise",
    ),
    (
      b"sigprocmask\0".as_slice(),
      (rlibc_sigprocmask
        as unsafe extern "C" fn(c_int, *const RlibcSigSet, *mut RlibcSigSet) -> c_int)
        as *const () as *mut c_void,
      "sigprocmask",
    ),
    (
      b"fcntl\0".as_slice(),
      (rlibc_fcntl as unsafe extern "C" fn(c_int, c_int, c_long) -> c_int) as *const ()
        as *mut c_void,
      "fcntl",
    ),
    (
      b"opendir\0".as_slice(),
      (rlibc_opendir as unsafe extern "C" fn(*const c_char) -> *mut RlibcDir) as *const ()
        as *mut c_void,
      "opendir",
    ),
    (
      b"readdir\0".as_slice(),
      (rlibc_readdir as unsafe extern "C" fn(*mut RlibcDir) -> *mut RlibcDirent) as *const ()
        as *mut c_void,
      "readdir",
    ),
    (
      b"closedir\0".as_slice(),
      (rlibc_closedir as unsafe extern "C" fn(*mut RlibcDir) -> c_int) as *const () as *mut c_void,
      "closedir",
    ),
    (
      b"rewinddir\0".as_slice(),
      (rlibc_rewinddir as unsafe extern "C" fn(*mut RlibcDir)) as *const () as *mut c_void,
      "rewinddir",
    ),
    (
      b"glob\0".as_slice(),
      (rlibc_glob
        as unsafe extern "C" fn(*const c_char, c_int, RlibcGlobErrorFn, *mut RlibcGlob) -> c_int)
        as *const () as *mut c_void,
      "glob",
    ),
    (
      b"globfree\0".as_slice(),
      (rlibc_globfree as unsafe extern "C" fn(*mut RlibcGlob)) as *const () as *mut c_void,
      "globfree",
    ),
    (
      b"setlocale\0".as_slice(),
      (rlibc_setlocale as unsafe extern "C" fn(c_int, *const c_char) -> *mut c_char) as *const ()
        as *mut c_void,
      "setlocale",
    ),
    (
      b"sqrt\0".as_slice(),
      (rlibc_sqrt as extern "C" fn(f64) -> f64) as *const () as *mut c_void,
      "sqrt",
    ),
    (
      b"log\0".as_slice(),
      (rlibc_log as extern "C" fn(f64) -> f64) as *const () as *mut c_void,
      "log",
    ),
    (
      b"exp\0".as_slice(),
      (rlibc_exp as extern "C" fn(f64) -> f64) as *const () as *mut c_void,
      "exp",
    ),
    (
      b"setjmp\0".as_slice(),
      (rlibc_setjmp as unsafe extern "C" fn(*mut rlibc_jmp_buf) -> c_int) as *const ()
        as *mut c_void,
      "setjmp",
    ),
    (
      b"longjmp\0".as_slice(),
      (rlibc_longjmp as unsafe extern "C" fn(*const rlibc_jmp_buf, c_int) -> !) as *const ()
        as *mut c_void,
      "longjmp",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3152, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_process_fenv_wchar_startup_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3153);

  let checks = [
    (
      b"atexit\0".as_slice(),
      (rlibc_atexit as extern "C" fn(Option<extern "C" fn()>) -> c_int) as *const () as *mut c_void,
      "atexit",
    ),
    (
      b"exit\0".as_slice(),
      (rlibc_exit as extern "C" fn(c_int) -> !) as *const () as *mut c_void,
      "exit",
    ),
    (
      b"_Exit\0".as_slice(),
      (rlibc_underscore_exit as extern "C" fn(c_int) -> !) as *const () as *mut c_void,
      "_Exit",
    ),
    (
      b"abort\0".as_slice(),
      (rlibc_abort as extern "C" fn() -> !) as *const () as *mut c_void,
      "abort",
    ),
    (
      b"__libc_start_main\0".as_slice(),
      (rlibc_libc_start_main
        as unsafe extern "C" fn(
          Option<RlibcStartMainFn>,
          c_int,
          *mut *mut c_char,
          *mut *mut c_char,
        ) -> !) as *const () as *mut c_void,
      "__libc_start_main",
    ),
    (
      b"feclearexcept\0".as_slice(),
      (rlibc_feclearexcept as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "feclearexcept",
    ),
    (
      b"fegetexceptflag\0".as_slice(),
      (rlibc_fegetexceptflag as unsafe extern "C" fn(*mut RlibcFexceptT, c_int) -> c_int)
        as *const () as *mut c_void,
      "fegetexceptflag",
    ),
    (
      b"feraiseexcept\0".as_slice(),
      (rlibc_feraiseexcept as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "feraiseexcept",
    ),
    (
      b"fesetexceptflag\0".as_slice(),
      (rlibc_fesetexceptflag as unsafe extern "C" fn(*const RlibcFexceptT, c_int) -> c_int)
        as *const () as *mut c_void,
      "fesetexceptflag",
    ),
    (
      b"fetestexcept\0".as_slice(),
      (rlibc_fetestexcept as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "fetestexcept",
    ),
    (
      b"fegetround\0".as_slice(),
      (rlibc_fegetround as extern "C" fn() -> c_int) as *const () as *mut c_void,
      "fegetround",
    ),
    (
      b"fesetround\0".as_slice(),
      (rlibc_fesetround as extern "C" fn(c_int) -> c_int) as *const () as *mut c_void,
      "fesetround",
    ),
    (
      b"fegetenv\0".as_slice(),
      (rlibc_fegetenv as unsafe extern "C" fn(*mut RlibcFenvT) -> c_int) as *const ()
        as *mut c_void,
      "fegetenv",
    ),
    (
      b"feholdexcept\0".as_slice(),
      (rlibc_feholdexcept as unsafe extern "C" fn(*mut RlibcFenvT) -> c_int) as *const ()
        as *mut c_void,
      "feholdexcept",
    ),
    (
      b"fesetenv\0".as_slice(),
      (rlibc_fesetenv as unsafe extern "C" fn(*const RlibcFenvT) -> c_int) as *const ()
        as *mut c_void,
      "fesetenv",
    ),
    (
      b"feupdateenv\0".as_slice(),
      (rlibc_feupdateenv as unsafe extern "C" fn(*const RlibcFenvT) -> c_int) as *const ()
        as *mut c_void,
      "feupdateenv",
    ),
    (
      b"mbrtowc\0".as_slice(),
      (rlibc_mbrtowc
        as unsafe extern "C" fn(
          *mut rlibc_wchar_t,
          *const c_char,
          size_t,
          *mut RlibcMbStateT,
        ) -> size_t) as *const () as *mut c_void,
      "mbrtowc",
    ),
    (
      b"mbrlen\0".as_slice(),
      (rlibc_mbrlen as unsafe extern "C" fn(*const c_char, size_t, *mut RlibcMbStateT) -> size_t)
        as *const () as *mut c_void,
      "mbrlen",
    ),
    (
      b"wcrtomb\0".as_slice(),
      (rlibc_wcrtomb
        as unsafe extern "C" fn(*mut c_char, rlibc_wchar_t, *mut RlibcMbStateT) -> size_t)
        as *const () as *mut c_void,
      "wcrtomb",
    ),
    (
      b"mbsrtowcs\0".as_slice(),
      (rlibc_mbsrtowcs
        as unsafe extern "C" fn(
          *mut rlibc_wchar_t,
          *mut *const c_char,
          size_t,
          *mut RlibcMbStateT,
        ) -> size_t) as *const () as *mut c_void,
      "mbsrtowcs",
    ),
    (
      b"wcsrtombs\0".as_slice(),
      (rlibc_wcsrtombs
        as unsafe extern "C" fn(
          *mut c_char,
          *mut *const rlibc_wchar_t,
          size_t,
          *mut RlibcMbStateT,
        ) -> size_t) as *const () as *mut c_void,
      "wcsrtombs",
    ),
    (
      b"mblen\0".as_slice(),
      (rlibc_mblen as unsafe extern "C" fn(*const c_char, size_t) -> c_int) as *const ()
        as *mut c_void,
      "mblen",
    ),
    (
      b"mbtowc\0".as_slice(),
      (rlibc_mbtowc as unsafe extern "C" fn(*mut rlibc_wchar_t, *const c_char, size_t) -> c_int)
        as *const () as *mut c_void,
      "mbtowc",
    ),
    (
      b"wctomb\0".as_slice(),
      (rlibc_wctomb as unsafe extern "C" fn(*mut c_char, rlibc_wchar_t) -> c_int) as *const ()
        as *mut c_void,
      "wctomb",
    ),
    (
      b"mbstowcs\0".as_slice(),
      (rlibc_mbstowcs as unsafe extern "C" fn(*mut rlibc_wchar_t, *const c_char, size_t) -> size_t)
        as *const () as *mut c_void,
      "mbstowcs",
    ),
    (
      b"wcstombs\0".as_slice(),
      (rlibc_wcstombs as unsafe extern "C" fn(*mut c_char, *const rlibc_wchar_t, size_t) -> size_t)
        as *const () as *mut c_void,
      "wcstombs",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3153, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_alloc_compat_and_environ_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3154);

  let checks = [
    (
      b"aligned_alloc\0".as_slice(),
      (rlibc_aligned_alloc as unsafe extern "C" fn(usize, usize) -> *mut c_void) as *const ()
        as *mut c_void,
      "aligned_alloc",
    ),
    (
      b"posix_memalign\0".as_slice(),
      (rlibc_posix_memalign as unsafe extern "C" fn(*mut *mut c_void, usize, usize) -> c_int)
        as *const () as *mut c_void,
      "posix_memalign",
    ),
    (
      b"memalign\0".as_slice(),
      (rlibc_memalign as unsafe extern "C" fn(usize, usize) -> *mut c_void) as *const ()
        as *mut c_void,
      "memalign",
    ),
    (
      b"valloc\0".as_slice(),
      (rlibc_valloc as unsafe extern "C" fn(usize) -> *mut c_void) as *const () as *mut c_void,
      "valloc",
    ),
    (
      b"pvalloc\0".as_slice(),
      (rlibc_pvalloc as unsafe extern "C" fn(usize) -> *mut c_void) as *const () as *mut c_void,
      "pvalloc",
    ),
    (
      b"mbsinit\0".as_slice(),
      (rlibc_mbsinit as unsafe extern "C" fn(*const RlibcMbStateT) -> c_int) as *const ()
        as *mut c_void,
      "mbsinit",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  let expected_environ = {
    let environ_ptr: *mut *mut *mut c_char = &raw mut rlibc_environ;

    environ_ptr.cast::<c_void>()
  };

  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy dlsym contract.
  let resolved_environ = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"environ\0")) };

  assert!(!resolved_environ.is_null(), "environ lookup should succeed");
  assert_eq!(
    resolved_environ, expected_environ,
    "RTLD_DEFAULT environ should resolve to rlibc environ variable storage",
  );
  assert_eq!(read_errno(), 3154, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_default_resolves_rlibc_printf_wrapper_symbol_pointers() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3155);

  let checks = [
    (
      b"printf\0".as_slice(),
      (rlibc_printf as unsafe extern "C" fn(*const c_char, ...) -> c_int) as *const ()
        as *mut c_void,
      "printf",
    ),
    (
      b"fprintf\0".as_slice(),
      (rlibc_fprintf as unsafe extern "C" fn(*mut FILE, *const c_char, ...) -> c_int) as *const ()
        as *mut c_void,
      "fprintf",
    ),
    (
      b"flockfile\0".as_slice(),
      (rlibc::stdio::flockfile as unsafe extern "C" fn(*mut FILE)) as *const () as *mut c_void,
      "flockfile",
    ),
    (
      b"ftrylockfile\0".as_slice(),
      (rlibc::stdio::ftrylockfile as unsafe extern "C" fn(*mut FILE) -> c_int) as *const ()
        as *mut c_void,
      "ftrylockfile",
    ),
    (
      b"funlockfile\0".as_slice(),
      (rlibc::stdio::funlockfile as unsafe extern "C" fn(*mut FILE)) as *const () as *mut c_void,
      "funlockfile",
    ),
  ];

  for (symbol, expected_ptr, label) in checks {
    // SAFETY: `RTLD_DEFAULT` and symbol pointers satisfy dlsym contract.
    let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(symbol)) };

    assert!(!resolved.is_null(), "{label} lookup should succeed");
    assert_eq!(
      resolved, expected_ptr,
      "RTLD_DEFAULT {label} should resolve to rlibc {label} implementation",
    );
  }

  assert_eq!(read_errno(), 3155, "successful dlsym must preserve errno");
}

#[test]
fn dlsym_returns_null_for_missing_symbol() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  // SAFETY: `RTLD_DEFAULT` and symbol pointer follow the C ABI contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"rlibc_i057_missing_symbol\0")) };

  assert!(resolved.is_null(), "missing symbol should return null");
}

#[test]
fn dlsym_missing_symbol_dlerror_includes_requested_symbol_detail() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  // SAFETY: `RTLD_DEFAULT` and symbol pointer follow the C ABI contract.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(MISSING_SYMBOL_DETAIL)) };

  assert!(resolved.is_null(), "missing symbol should return null");

  let message =
    take_dlerror_message().expect("missing symbol lookup should set detailed dlerror message");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message prefix: {message}",
  );
  assert!(
    message.contains("rlibc_i055_missing_symbol_detail"),
    "dlerror should include the requested missing symbol name: {message}",
  );
  assert!(
    message.contains("rlibc_i055_missing_symbol_detail:"),
    "dlerror should append host detail text after symbol name: {message}",
  );
}

#[test]
fn dlsym_rtld_next_missing_symbol_preserves_errno_and_sets_dlerror() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(2525);

  // SAFETY: `RTLD_NEXT` is a valid loader sentinel and symbol is a valid NUL-terminated string.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"rlibc_i057_next_missing_symbol\0")) };

  assert!(
    resolved.is_null(),
    "missing RTLD_NEXT symbol should return null"
  );

  let message = take_dlerror_message().expect("missing RTLD_NEXT symbol should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 2525, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_next_missing_symbol_dlerror_includes_requested_symbol_detail() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(2727);

  let missing_symbol = b"rlibc_i057_next_missing_symbol_detail\0";
  // SAFETY: `RTLD_NEXT` is a valid loader sentinel and symbol is a valid
  // NUL-terminated string.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(missing_symbol)) };

  assert!(
    resolved.is_null(),
    "missing RTLD_NEXT symbol should return null"
  );

  let message =
    take_dlerror_message().expect("missing RTLD_NEXT symbol should set detailed dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message prefix: {message}",
  );
  assert!(
    message.contains("rlibc_i057_next_missing_symbol_detail"),
    "dlerror should include requested RTLD_NEXT symbol name: {message}",
  );
  assert!(
    message.contains("rlibc_i057_next_missing_symbol_detail:"),
    "dlerror should append host detail text after RTLD_NEXT symbol name: {message}",
  );
  assert_eq!(read_errno(), 2727, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_empty_symbol_dlerror_uses_empty_symbol_placeholder() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(2828);

  // SAFETY: `RTLD_DEFAULT` handle and NUL-only symbol follow the C ABI; empty
  // symbol name is intentional for error-path coverage.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"\0")) };

  assert!(resolved.is_null(), "empty symbol lookup should fail");

  let message = take_dlerror_message().expect("empty symbol lookup should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message prefix: {message}",
  );
  assert!(
    message.contains("requested symbol was not found: <empty symbol>"),
    "dlerror should include explicit empty-symbol placeholder in its base message: {message}",
  );
  assert!(
    message.contains("<empty symbol>"),
    "dlerror should use the empty-symbol placeholder: {message}",
  );
  assert_eq!(read_errno(), 2828, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_rtld_next_empty_symbol_dlerror_uses_empty_symbol_placeholder() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(2929);

  // SAFETY: `RTLD_NEXT` handle and NUL-only symbol follow the C ABI; empty
  // symbol name is intentional for error-path coverage.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"\0")) };

  assert!(
    resolved.is_null(),
    "empty RTLD_NEXT symbol lookup should fail"
  );

  let message = take_dlerror_message().expect("empty RTLD_NEXT symbol lookup should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message prefix: {message}",
  );
  assert!(
    message.contains("requested symbol was not found: <empty symbol>"),
    "dlerror should include explicit empty-symbol placeholder in its base message: {message}",
  );
  assert!(
    message.contains("<empty symbol>"),
    "dlerror should use the empty-symbol placeholder: {message}",
  );
  assert_eq!(read_errno(), 2929, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_does_not_modify_errno_on_success_or_failure() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  write_errno(1234);
  // SAFETY: `RTLD_NEXT` and symbol pointer follow C ABI contract.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"getenv\0")) };

  assert!(
    !resolved.is_null(),
    "getenv should resolve through RTLD_NEXT"
  );
  assert_eq!(read_errno(), 1234, "successful dlsym must preserve errno");

  write_errno(4321);
  // SAFETY: `RTLD_DEFAULT` and symbol pointer follow C ABI contract.
  let missing = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"rlibc_i057_missing_symbol\0")) };

  assert!(missing.is_null(), "missing symbol should return null");

  let message = take_dlerror_message().expect("missing symbol should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 4321, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_main_program_handle_matches_rtld_default_for_dlopen_symbol() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(3170);

  // SAFETY: null filename requests the main-program loader handle.
  let main_handle = unsafe { dlopen(ptr::null(), RTLD_NOW) };

  assert!(!main_handle.is_null(), "main-program handle should resolve");

  // SAFETY: handle and symbol pointer satisfy dlsym C ABI contract.
  let from_main = unsafe { dlsym(main_handle, symbol_ptr(b"dlopen\0")) };
  // SAFETY: RTLD_DEFAULT and symbol pointer satisfy dlsym C ABI contract.
  let from_default = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"dlopen\0")) };

  assert!(
    !from_main.is_null(),
    "dlopen symbol should resolve via main handle"
  );
  assert_eq!(
    from_main, from_default,
    "main-program-handle lookup should match RTLD_DEFAULT lookup",
  );
  assert_eq!(read_errno(), 3170, "successful dlsym must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "successful main-program-handle and RTLD_DEFAULT lookups must not create dlerror",
  );
  assert_eq!(
    dlclose(main_handle),
    0,
    "main-program handle should be closable"
  );
}

#[test]
fn dlsym_main_program_handle_falls_back_to_host_puts_and_leaves_dlerror_empty() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  // SAFETY: null filename requests the main-program loader handle.
  let main_handle = unsafe { dlopen(ptr::null(), RTLD_NOW) };

  assert!(!main_handle.is_null(), "main-program handle should resolve");

  write_errno(3171);

  // SAFETY: handle and symbol pointer satisfy the dlsym C ABI contract.
  let from_main = unsafe { dlsym(main_handle, symbol_ptr(b"puts\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy the dlsym C ABI contract.
  let from_default = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"puts\0")) };

  assert!(
    !from_main.is_null(),
    "main-program handle should fall back to host lookup for puts",
  );
  assert!(
    !from_default.is_null(),
    "RTLD_DEFAULT host lookup should resolve puts",
  );
  assert_eq!(
    from_main, from_default,
    "main-program handle host fallback should match RTLD_DEFAULT for host-only symbols",
  );
  assert_eq!(read_errno(), 3171, "successful dlsym must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "successful main-program-handle host fallback must not create dlerror",
  );
  assert_eq!(
    dlclose(main_handle),
    0,
    "main-program handle should be closable"
  );
}

#[test]
fn dlsym_closed_main_program_handle_is_not_treated_as_rtld_default() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  // SAFETY: null filename requests the main-program loader handle.
  let main_handle = unsafe { dlopen(ptr::null(), RTLD_NOW) };

  assert!(!main_handle.is_null(), "main-program handle should resolve");
  assert_eq!(
    dlclose(main_handle),
    0,
    "main-program handle should be closable"
  );

  write_errno(3181);

  // SAFETY: closed handle and symbol pointer satisfy the C ABI contract.
  let closed_lookup = unsafe { dlsym(main_handle, symbol_ptr(b"dlopen\0")) };

  assert!(
    closed_lookup.is_null(),
    "closed main-program handle lookup must fail",
  );

  let message =
    take_dlerror_message().expect("closed main-program handle lookup should set dlerror");

  assert!(
    message.contains("already closed"),
    "closed main-program handle must not be treated as RTLD_DEFAULT: {message}",
  );
  assert_eq!(read_errno(), 3181, "failed dlsym must preserve errno");

  // SAFETY: RTLD_DEFAULT and symbol pointer satisfy dlsym C ABI contract.
  let default_lookup = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"dlopen\0")) };

  assert!(
    !default_lookup.is_null(),
    "RTLD_DEFAULT lookup should continue to succeed after closing main handle",
  );
  assert_eq!(read_errno(), 3181, "successful dlsym must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "successful RTLD_DEFAULT lookup from clean state must not create dlerror",
  );
}

#[test]
fn dlsym_returns_null_for_null_symbol_pointer() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();
  write_errno(7070);

  // SAFETY: Passing null symbol pointer is invalid by contract and should fail safely.
  let resolved = unsafe { dlsym(RTLD_DEFAULT, ptr::null()) };

  assert!(resolved.is_null(), "null symbol pointer should return null");

  let message = take_dlerror_message().expect("null symbol pointer should set dlerror");

  assert!(
    message.contains("dlsym symbol pointer is null"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 7070, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_returns_null_for_unknown_handle_and_sets_dlerror() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let unknown_handle = 0x00DE_C0DE_usize as *mut c_void;

  write_errno(6060);

  // SAFETY: `symbol` is a valid NUL-terminated string and `handle` is intentionally invalid.
  let resolved = unsafe { dlsym(unknown_handle, symbol_ptr(b"getenv\0")) };
  let message = take_dlerror_message().expect("invalid handle should set dlerror");

  assert!(resolved.is_null(), "unknown handle should return null");
  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 6060, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_unknown_handle_with_null_symbol_reports_invalid_handle_first() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let unknown_handle = 0x00C0_FFEE_usize as *mut c_void;

  write_errno(6262);

  // SAFETY: both arguments are intentionally invalid to assert deterministic
  // validation precedence on non-special handles.
  let resolved = unsafe { dlsym(unknown_handle, ptr::null()) };
  let message =
    take_dlerror_message().expect("unknown-handle lookup with null symbol should set dlerror");

  assert!(
    resolved.is_null(),
    "invalid-handle lookup should return null"
  );
  assert!(
    message.contains("invalid dynamic-loader handle"),
    "unexpected dlerror message: {message}",
  );
  assert!(
    !message.contains("dlsym symbol pointer is null"),
    "non-special handle validation should win over null-symbol validation: {message}",
  );
  assert_eq!(read_errno(), 6262, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_returns_null_for_closed_handle_and_preserves_errno() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  write_errno(2026);
  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(read_errno(), 2026, "successful dlopen must preserve errno");

  let mut close_attempts = 0_usize;

  loop {
    let close_status = dlclose(handle);

    close_attempts = close_attempts.saturating_add(1);

    if close_status != 0 {
      break;
    }

    assert!(
      close_attempts < 64,
      "failed to observe a closed-handle state after repeated dlclose calls",
    );
  }

  clear_pending_dlerror();

  write_errno(3030);
  // SAFETY: symbol pointer is valid and NUL-terminated.
  let resolved = unsafe { dlsym(handle, symbol_ptr(b"getenv\0")) };

  assert!(resolved.is_null(), "closed handle lookup should fail");

  let message = take_dlerror_message().expect("closed handle lookup should set dlerror");

  assert!(
    message.contains("already closed"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 3030, "failed dlsym must preserve errno");
}

#[test]
fn dlsym_resolves_symbol_for_reopened_handle_after_close() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let first_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !first_handle.is_null(),
    "initial dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    dlclose(first_handle),
    0,
    "closing initial handle should succeed",
  );

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let reopened_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !reopened_handle.is_null(),
    "reopened dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );

  write_errno(4545);

  // SAFETY: symbol pointer is valid and NUL-terminated.
  let resolved = unsafe { dlsym(reopened_handle, symbol_ptr(b"getenv\0")) };

  assert!(!resolved.is_null(), "reopened handle lookup should succeed");
  assert_eq!(read_errno(), 4545, "successful dlsym must preserve errno");
  assert_eq!(
    dlclose(reopened_handle),
    0,
    "closing reopened handle should succeed",
  );
}

#[test]
fn dlsym_host_handle_getenv_differs_from_rtld_default_and_leaves_dlerror_empty() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let libc_path = loaded_libc_path().expect("expected libc path in /proc/self/maps");
  let libc_cstr = c_string_from_path(&libc_path);

  // SAFETY: `libc_cstr` is a valid NUL-terminated shared-object path.
  let host_handle = unsafe { dlopen(libc_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !host_handle.is_null(),
    "dlopen should return a host handle for libc: {}",
    libc_path.display(),
  );

  write_errno(4546);

  // SAFETY: handle and symbol pointer satisfy the dlsym C ABI contract.
  let from_host = unsafe { dlsym(host_handle, symbol_ptr(b"getenv\0")) };
  // SAFETY: `RTLD_DEFAULT` and symbol pointer satisfy the dlsym C ABI contract.
  let from_default = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"getenv\0")) };

  assert!(
    !from_host.is_null(),
    "host-handle getenv lookup should succeed"
  );
  assert!(
    !from_default.is_null(),
    "RTLD_DEFAULT getenv lookup should succeed"
  );
  assert_eq!(
    from_default,
    (rlibc_getenv as unsafe extern "C" fn(*const c_char) -> *mut c_char) as *const ()
      as *mut c_void,
    "RTLD_DEFAULT getenv should resolve to rlibc getenv implementation",
  );
  assert_ne!(
    from_host, from_default,
    "host-handle lookup should keep the host getenv symbol distinct from RTLD_DEFAULT",
  );
  assert_eq!(read_errno(), 4546, "successful dlsym must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "successful host-handle lookup from clean state must not create dlerror",
  );
  assert_eq!(dlclose(host_handle), 0, "host handle should be closable");
}

#[test]
fn dlsym_reopened_handle_missing_symbol_preserves_errno_and_reports_not_found() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let first_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !first_handle.is_null(),
    "initial dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(
    dlclose(first_handle),
    0,
    "closing initial handle should succeed",
  );

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let reopened_handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !reopened_handle.is_null(),
    "reopened dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );

  write_errno(5151);

  // SAFETY: symbol pointer is valid and NUL-terminated.
  let resolved = unsafe {
    dlsym(
      reopened_handle,
      symbol_ptr(b"rlibc_i057_reopen_missing_symbol\0"),
    )
  };

  assert!(resolved.is_null(), "missing symbol lookup should fail");

  let message = take_dlerror_message().expect("missing symbol lookup should set dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 5151, "failed dlsym must preserve errno");
  assert_eq!(
    dlclose(reopened_handle),
    0,
    "closing reopened handle should succeed",
  );
}

#[test]
fn dlsym_missing_symbol_replaces_prior_closed_handle_error() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );

  let mut close_attempts = 0_usize;

  loop {
    let close_status = dlclose(handle);

    close_attempts = close_attempts.saturating_add(1);

    if close_status != 0 {
      break;
    }

    assert!(
      close_attempts < 64,
      "failed to observe a closed-handle state after repeated dlclose calls",
    );
  }

  let mut probe_attempts = 0_usize;
  let closed_lookup = loop {
    write_errno(6161);

    // SAFETY: closed-handle path validates the handle before delegating to host resolver.
    let resolved = unsafe { dlsym(handle, symbol_ptr(b"strlen\0")) };

    if resolved.is_null() {
      break resolved;
    }

    let _ = dlclose(handle);

    probe_attempts = probe_attempts.saturating_add(1);
    assert!(
      probe_attempts < 64,
      "closed handle lookup should eventually fail after repeated close attempts",
    );
  };

  assert!(closed_lookup.is_null(), "closed handle lookup should fail");
  assert_eq!(read_errno(), 6161, "failed dlsym must preserve errno");

  write_errno(7171);

  // SAFETY: symbol pointer is valid and NUL-terminated.
  let missing_lookup = unsafe { dlsym(RTLD_DEFAULT, symbol_ptr(b"rlibc_i057_latest_missing\0")) };

  assert!(
    missing_lookup.is_null(),
    "missing symbol lookup should fail"
  );

  let message =
    take_dlerror_message().expect("latest missing-symbol error should replace prior dlerror");

  assert!(
    message.contains("requested symbol was not found"),
    "unexpected dlerror message: {message}",
  );
  assert_eq!(read_errno(), 7171, "failed dlsym must preserve errno");
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlsym_success_does_not_clear_prior_error_message() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let shared_object_path =
    first_loaded_shared_object().expect("expected at least one loaded shared object in process");
  let path_cstr = c_string_from_path(&shared_object_path);

  // SAFETY: path pointer is valid and NUL-terminated for the duration of the call.
  let handle = unsafe { dlopen(path_cstr.as_ptr().cast::<c_char>(), RTLD_NOW) };

  assert!(
    !handle.is_null(),
    "dlopen should return handle for valid shared object path: {}",
    shared_object_path.display(),
  );
  assert_eq!(dlclose(handle), 0, "first close should succeed");

  write_errno(8181);

  // SAFETY: closed-handle path validates the handle before delegating to host resolver.
  let closed_lookup = unsafe { dlsym(handle, symbol_ptr(b"strlen\0")) };

  assert!(closed_lookup.is_null(), "closed handle lookup should fail");
  assert_eq!(read_errno(), 8181, "failed dlsym must preserve errno");

  write_errno(9191);

  // SAFETY: `RTLD_NEXT` sentinel and symbol pointer satisfy C ABI contract.
  let resolved = unsafe { dlsym(RTLD_NEXT, symbol_ptr(b"getenv\0")) };

  assert!(
    !resolved.is_null(),
    "successful dlsym should resolve getenv through RTLD_NEXT",
  );
  assert_eq!(read_errno(), 9191, "successful dlsym must preserve errno");

  let message =
    take_dlerror_message().expect("successful dlsym should not clear prior pending dlerror");

  assert!(
    message.contains("already closed"),
    "unexpected preserved dlerror message: {message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "second dlerror call must clear pending state",
  );
}

#[test]
fn dlsym_error_state_is_thread_local_between_main_and_child_threads() {
  let _serial = dlfcn_test_lock();

  clear_pending_dlerror();

  let child_message = thread::spawn(|| {
    clear_pending_dlerror();

    let unknown_handle = 0x0BAD_5EED_usize as *mut c_void;
    // SAFETY: `symbol` is a valid NUL-terminated string and `handle` is intentionally invalid.
    let resolved = unsafe { dlsym(unknown_handle, symbol_ptr(b"getenv\0")) };

    assert!(resolved.is_null(), "unknown handle should return null");

    take_dlerror_message()
  })
  .join()
  .expect("child thread panicked")
  .expect("child thread should observe dlsym error");

  assert!(
    child_message.contains("invalid dynamic-loader handle"),
    "unexpected child dlerror message: {child_message}",
  );
  assert!(
    take_dlerror_message().is_none(),
    "child-thread dlsym failure must not leak dlerror state into main thread",
  );
}
