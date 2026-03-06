//! Minimal pthread APIs.
//!
//! This module provides a Linux `x86_64` baseline for:
//! - thread lifecycle: `pthread_create`, `pthread_join`, `pthread_detach`
//! - mutex + mutex attributes
//! - condition variables
//! - read-write locks
//!
//! Contract notes:
//! - APIs return pthread-style error numbers directly (`0` on success).
//! - APIs in this module do not use `errno` for error delivery.
//! - `PTHREAD_PROCESS_SHARED` attributes are accepted for ABI compatibility,
//!   but synchronization state remains process-local within this runtime.

use crate::abi::errno::{EAGAIN, EBUSY, EDEADLK, EINVAL, EPERM, ESRCH, ETIMEDOUT};
use crate::abi::types::{c_int, c_long, c_ulong, c_void};
use crate::time::{CLOCK_REALTIME, clock_gettime, timespec};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_char;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

/// `PTHREAD_MUTEX_NORMAL` mutex type value.
pub const PTHREAD_MUTEX_NORMAL: c_int = 0;
/// `PTHREAD_MUTEX_RECURSIVE` mutex type value.
pub const PTHREAD_MUTEX_RECURSIVE: c_int = 1;
/// `PTHREAD_MUTEX_ERRORCHECK` mutex type value.
pub const PTHREAD_MUTEX_ERRORCHECK: c_int = 2;
/// `PTHREAD_MUTEX_DEFAULT` mutex type value.
pub const PTHREAD_MUTEX_DEFAULT: c_int = PTHREAD_MUTEX_NORMAL;
/// `PTHREAD_PROCESS_PRIVATE` attribute value.
pub const PTHREAD_PROCESS_PRIVATE: c_int = 0;
/// `PTHREAD_PROCESS_SHARED` attribute value.
pub const PTHREAD_PROCESS_SHARED: c_int = 1;
const NATIVE_DETACHED_CACHE_LIMIT: usize = 1024;
const NATIVE_CONSUMED_CACHE_LIMIT: usize = 1024;
const DESTROYED_COND_SENTINEL: *mut PthreadCondState = ptr::dangling_mut::<PthreadCondState>();
const RTLD_NEXT: *mut c_void = (-1_isize) as *mut c_void;
const PTHREAD_CREATE_NAME: &[u8] = b"pthread_create\0";
const PTHREAD_DETACH_NAME: &[u8] = b"pthread_detach\0";
const PTHREAD_JOIN_NAME: &[u8] = b"pthread_join\0";
const GLIBC_DLSYM_VERSION_CANDIDATES: [&[u8]; 2] = [b"GLIBC_2.34\0", b"GLIBC_2.2.5\0"];
static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);
static REGISTRY: LazyLock<Mutex<PthreadRegistry>> =
  LazyLock::new(|| Mutex::new(PthreadRegistry::default()));
static RWLOCK_REGISTRY: LazyLock<Mutex<HashMap<usize, Arc<PthreadRwlockState>>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
static COND_LAZY_INIT_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static HOST_PTHREAD_CREATE: OnceLock<Option<RealPthreadCreate>> = OnceLock::new();
static HOST_PTHREAD_DETACH: OnceLock<Option<RealPthreadDetach>> = OnceLock::new();
static HOST_PTHREAD_JOIN: OnceLock<Option<RealPthreadJoin>> = OnceLock::new();

/// Opaque pthread handle type for Linux `x86_64`.
pub type pthread_t = c_ulong;

/// Opaque pthread attribute payload for Linux `x86_64`.
///
/// ABI notes:
/// - This layout matches glibc's public contract shape:
///   a 56-byte payload aligned as `long`.
/// - Non-null attributes are accepted for ABI compatibility and treated as an
///   opaque payload.
#[repr(C)]
pub union pthread_attr_t {
  /// Raw opaque storage.
  pub __size: [u8; 56],
  /// Alignment anchor.
  pub __align: c_long,
}

/// POSIX read-write lock object.
///
/// Contract notes:
/// - This payload mirrors glibc's public union shape (56-byte opaque storage
///   aligned as `long`).
/// - The object must be initialized by [`pthread_rwlock_init`] before use.
#[repr(C)]
pub union pthread_rwlock_t {
  /// Raw opaque storage.
  pub __size: [u8; 56],
  /// Alignment anchor.
  pub __align: c_long,
}

/// POSIX read-write lock attribute object.
///
/// ABI notes:
/// - This payload mirrors glibc's 8-byte rwlock-attr union shape on Linux
///   `x86_64`.
/// - [`pthread_rwlock_init`] accepts nullable pointers to this payload and
///   currently treats all provided bytes as default attributes.
#[repr(C)]
pub union pthread_rwlockattr_t {
  /// Raw opaque storage.
  pub __size: [u8; 8],
  /// Alignment anchor.
  pub __align: c_long,
}

type StartRoutine = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

type RealPthreadCreate = unsafe extern "C" fn(
  *mut pthread_t,
  *const pthread_attr_t,
  Option<StartRoutine>,
  *mut c_void,
) -> c_int;

type RealPthreadDetach = unsafe extern "C" fn(pthread_t) -> c_int;

type RealPthreadJoin = unsafe extern "C" fn(pthread_t, *mut *mut c_void) -> c_int;

type ThreadResultWord = usize;

enum JoinTarget {
  Local(JoinHandle<ThreadResultWord>),
  NativeJoinable,
  NativeDetached,
  UnknownNative,
}

enum DetachTarget {
  Local(JoinHandle<ThreadResultWord>),
  NativeJoinable,
  NativeDetached,
  UnknownNative,
}

#[derive(Default)]
struct PthreadRegistry {
  detached: HashSet<pthread_t>,
  finished: HashSet<pthread_t>,
  joinable: HashMap<pthread_t, JoinHandle<ThreadResultWord>>,
  local_pending: HashSet<pthread_t>,
  native_detached: HashSet<pthread_t>,
  native_detached_order: VecDeque<pthread_t>,
  native_consumed: HashSet<pthread_t>,
  native_consumed_order: VecDeque<pthread_t>,
  native_joinable: HashSet<pthread_t>,
}

thread_local! {
  static CURRENT_THREAD_ID: Cell<Option<pthread_t>> = const { Cell::new(None) };
  static INTERNAL_PTHREAD_FALLBACK: Cell<bool> = const { Cell::new(false) };
  static RECENT_NATIVE_CREATES: RefCell<Vec<pthread_t>> = const { RefCell::new(Vec::new()) };
}

#[link(name = "dl")]
unsafe extern "C" {
  #[link_name = "dlvsym"]
  fn host_dlvsym(handle: *mut c_void, symbol: *const c_char, version: *const c_char)
  -> *mut c_void;
  fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[derive(Default)]
struct PthreadMutexLockState {
  owner: Option<ThreadId>,
  recursion_depth: usize,
  waiter_count: usize,
  cond_waiter_count: usize,
}

struct PthreadMutexState {
  mutex_type: c_int,
  lock_state: Mutex<PthreadMutexLockState>,
  wait_cv: Condvar,
}

#[derive(Default)]
struct PthreadCondWaitState {
  waiter_count: usize,
  pending_wakeups: usize,
}

struct PthreadCondState {
  wait_state: Mutex<PthreadCondWaitState>,
  wait_cv: Condvar,
}

#[derive(Default)]
struct PthreadRwlockLockState {
  reader_owners: HashMap<ThreadId, usize>,
  total_readers: usize,
  writer_owner: Option<ThreadId>,
  waiting_readers: usize,
  waiting_writers: usize,
  destroyed: bool,
}

struct PthreadRwlockState {
  lock_state: Mutex<PthreadRwlockLockState>,
  wait_cv: Condvar,
}

/// POSIX mutex object.
///
/// The object must be initialized by [`pthread_mutex_init`] before use and
/// destroyed by [`pthread_mutex_destroy`] when no thread owns it.
#[repr(C)]
pub struct pthread_mutex_t {
  state: *mut PthreadMutexState,
}

/// POSIX mutex attribute object.
///
/// This implementation supports mutex type selection (`NORMAL`, `ERRORCHECK`,
/// `RECURSIVE`) plus both `PTHREAD_PROCESS_PRIVATE` and
/// `PTHREAD_PROCESS_SHARED` attribute values.
///
/// `PTHREAD_PROCESS_SHARED` is accepted for ABI compatibility, but the mutex
/// state remains process-local and does not coordinate across processes.
#[repr(C)]
pub struct pthread_mutexattr_t {
  mutex_type: c_int,
  pshared: c_int,
  initialized: c_int,
}

/// POSIX condition variable object.
///
/// The object is typically initialized by [`pthread_cond_init`] before wait
/// operations and destroyed by [`pthread_cond_destroy`] when no threads are
/// waiting.
///
/// For libc compatibility, [`pthread_cond_signal`] / [`pthread_cond_broadcast`]
/// / [`pthread_cond_destroy`] also accept zero-initialized objects (`state ==
/// null`) as static-initializer states. Wait APIs lazily initialize that state
/// on first use.
#[repr(C)]
pub struct pthread_cond_t {
  state: *mut PthreadCondState,
}

/// POSIX condition-variable attribute object.
///
/// This implementation accepts both `PTHREAD_PROCESS_PRIVATE` and
/// `PTHREAD_PROCESS_SHARED`.
///
/// `PTHREAD_PROCESS_SHARED` is accepted for ABI compatibility, but the
/// condition-variable state remains process-local and does not coordinate
/// across processes.
#[repr(C)]
pub struct pthread_condattr_t {
  pshared: c_int,
  initialized: c_int,
}

impl PthreadRegistry {
  fn handle_local_thread_exit(&mut self, thread: pthread_t) {
    if self.detached.remove(&thread) {
      // Detached threads are tracked only while they may still be running.
    } else if self.joinable.contains_key(&thread) || self.local_pending.contains(&thread) {
      self.finished.insert(thread);
    }
  }

  fn mark_native_detached(&mut self, thread: pthread_t) {
    self.clear_native_consumed(thread);
    self.native_joinable.remove(&thread);

    if self.native_detached.insert(thread) {
      self.native_detached_order.push_back(thread);
    }

    while self.native_detached_order.len() > NATIVE_DETACHED_CACHE_LIMIT {
      let Some(expired) = self.native_detached_order.pop_front() else {
        break;
      };

      self.native_detached.remove(&expired);
    }
  }

  fn clear_native_detached(&mut self, thread: pthread_t) {
    if !self.native_detached.remove(&thread) {
      return;
    }

    if let Some(position) = self
      .native_detached_order
      .iter()
      .position(|candidate| *candidate == thread)
    {
      self.native_detached_order.remove(position);
    }
  }

  fn mark_native_consumed(&mut self, thread: pthread_t) {
    self.native_joinable.remove(&thread);
    self.clear_native_detached(thread);

    if self.native_consumed.insert(thread) {
      self.native_consumed_order.push_back(thread);
    }

    while self.native_consumed_order.len() > NATIVE_CONSUMED_CACHE_LIMIT {
      let Some(expired) = self.native_consumed_order.pop_front() else {
        break;
      };

      self.native_consumed.remove(&expired);
    }
  }

  fn clear_native_consumed(&mut self, thread: pthread_t) {
    if !self.native_consumed.remove(&thread) {
      return;
    }

    if let Some(position) = self
      .native_consumed_order
      .iter()
      .position(|candidate| *candidate == thread)
    {
      self.native_consumed_order.remove(position);
    }
  }

  fn handle_forwarded_native_detach_result(
    &mut self,
    thread: pthread_t,
    detach_result: c_int,
    restore_joinable_on_error: bool,
  ) -> c_int {
    if detach_result == 0 || detach_result == EINVAL {
      self.mark_native_detached(thread);

      return detach_result;
    }

    if detach_result == ESRCH {
      self.mark_native_consumed(thread);

      return ESRCH;
    }

    if restore_joinable_on_error && !self.native_detached.contains(&thread) {
      self.native_joinable.insert(thread);
    }

    detach_result
  }

  fn handle_forwarded_native_join_result(
    &mut self,
    thread: pthread_t,
    join_result: c_int,
    restore_joinable_on_error: bool,
  ) -> c_int {
    let was_detached = self.native_detached.contains(&thread);

    if join_result == 0 {
      if was_detached {
        self.mark_native_detached(thread);

        return 0;
      }

      self.mark_native_consumed(thread);

      return 0;
    }

    if join_result == ESRCH {
      self.mark_native_consumed(thread);

      return ESRCH;
    }

    if join_result == EINVAL {
      self.mark_native_detached(thread);

      return EINVAL;
    }

    if restore_joinable_on_error && !self.native_detached.contains(&thread) {
      self.native_joinable.insert(thread);
    }

    join_result
  }
}

impl PthreadMutexState {
  const fn new(mutex_type: c_int) -> Self {
    Self {
      mutex_type,
      lock_state: Mutex::new(PthreadMutexLockState {
        owner: None,
        recursion_depth: 0,
        waiter_count: 0,
        cond_waiter_count: 0,
      }),
      wait_cv: Condvar::new(),
    }
  }
}

impl PthreadCondState {
  fn new() -> Self {
    Self {
      wait_state: Mutex::new(PthreadCondWaitState::default()),
      wait_cv: Condvar::new(),
    }
  }
}

impl PthreadRwlockState {
  fn new() -> Self {
    Self {
      lock_state: Mutex::new(PthreadRwlockLockState::default()),
      wait_cv: Condvar::new(),
    }
  }
}

impl Default for pthread_condattr_t {
  fn default() -> Self {
    Self {
      pshared: PTHREAD_PROCESS_PRIVATE,
      initialized: 0,
    }
  }
}

impl Default for pthread_mutex_t {
  fn default() -> Self {
    Self {
      state: ptr::null_mut(),
    }
  }
}

impl Default for pthread_mutexattr_t {
  fn default() -> Self {
    Self {
      mutex_type: PTHREAD_MUTEX_DEFAULT,
      pshared: PTHREAD_PROCESS_PRIVATE,
      initialized: 0,
    }
  }
}

impl Default for pthread_cond_t {
  fn default() -> Self {
    Self {
      state: ptr::null_mut(),
    }
  }
}

fn current_thread_id() -> Option<pthread_t> {
  CURRENT_THREAD_ID.with(Cell::get)
}

fn allocate_thread_id(registry: &PthreadRegistry) -> pthread_t {
  loop {
    let raw = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
    let candidate =
      pthread_t::try_from(raw).unwrap_or_else(|_| unreachable!("u64 must fit into pthread_t"));

    if candidate == 0 {
      continue;
    }

    if registry.joinable.contains_key(&candidate)
      || registry.local_pending.contains(&candidate)
      || registry.detached.contains(&candidate)
      || registry.native_joinable.contains(&candidate)
      || registry.native_detached.contains(&candidate)
      || registry.native_consumed.contains(&candidate)
    {
      continue;
    }

    return candidate;
  }
}

fn lock_registry() -> MutexGuard<'static, PthreadRegistry> {
  REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

fn is_zeroed_attr_payload(attr: *const pthread_attr_t) -> bool {
  // SAFETY: callers only pass non-null pointers and `pthread_create` contract
  // requires readable attribute bytes when `attr` is non-null.
  let payload = unsafe {
    core::slice::from_raw_parts(attr.cast::<u8>(), core::mem::size_of::<pthread_attr_t>())
  };

  payload.iter().all(|byte| *byte == 0)
}

fn should_forward_internal_pthread_call() -> bool {
  INTERNAL_PTHREAD_FALLBACK.with(Cell::get)
}

fn remember_recent_native_create(thread: pthread_t) {
  RECENT_NATIVE_CREATES.with(|recent| {
    let mut recent = recent.borrow_mut();

    if recent.contains(&thread) {
      return;
    }

    recent.push(thread);
  });
}

fn clear_recent_native_create(thread: pthread_t) {
  RECENT_NATIVE_CREATES.with(|recent| {
    let mut recent = recent.borrow_mut();

    if let Some(position) = recent.iter().position(|candidate| *candidate == thread) {
      recent.remove(position);
    }
  });
}

fn with_internal_pthread_fallback<T>(f: impl FnOnce() -> T) -> T {
  INTERNAL_PTHREAD_FALLBACK.with(|slot| {
    let previous = slot.replace(true);
    let output = f();

    slot.set(previous);

    output
  })
}

fn resolve_host_symbol(symbol_name: &[u8]) -> Option<*mut c_void> {
  let versioned_symbol_ptr = GLIBC_DLSYM_VERSION_CANDIDATES.iter().find_map(|version| {
    // SAFETY: symbol/version names are static NUL-terminated strings.
    let resolved = unsafe {
      host_dlvsym(
        RTLD_NEXT,
        symbol_name.as_ptr().cast(),
        version.as_ptr().cast(),
      )
    };

    if resolved.is_null() {
      return None;
    }

    Some(resolved)
  });
  let symbol_ptr = versioned_symbol_ptr.unwrap_or_else(|| {
    // SAFETY: symbol name is NUL-terminated and `RTLD_NEXT` is a documented lookup handle.
    unsafe { dlsym(RTLD_NEXT, symbol_name.as_ptr().cast()) }
  });

  if symbol_ptr.is_null() {
    return None;
  }

  Some(symbol_ptr)
}

fn resolve_real_pthread_create() -> Option<RealPthreadCreate> {
  *HOST_PTHREAD_CREATE.get_or_init(|| {
    let symbol = resolve_host_symbol(PTHREAD_CREATE_NAME)?;
    let local_symbol = pthread_create
      as unsafe extern "C" fn(
        *mut pthread_t,
        *const pthread_attr_t,
        Option<StartRoutine>,
        *mut c_void,
      ) -> c_int;

    if symbol == local_symbol as *const () as *mut c_void {
      return None;
    }

    // SAFETY: `symbol` was returned by `dlsym` for `pthread_create` and is
    // invoked using the matching C ABI signature.
    Some(unsafe { core::mem::transmute::<*mut c_void, RealPthreadCreate>(symbol) })
  })
}

fn resolve_real_pthread_detach() -> Option<RealPthreadDetach> {
  *HOST_PTHREAD_DETACH.get_or_init(|| {
    let symbol = resolve_host_symbol(PTHREAD_DETACH_NAME)?;
    let local_symbol = pthread_detach as extern "C" fn(pthread_t) -> c_int;

    if symbol == local_symbol as *const () as *mut c_void {
      return None;
    }

    // SAFETY: `symbol` was returned by `dlsym` for `pthread_detach` and is
    // invoked using the matching C ABI signature.
    Some(unsafe { core::mem::transmute::<*mut c_void, RealPthreadDetach>(symbol) })
  })
}

fn resolve_real_pthread_join() -> Option<RealPthreadJoin> {
  *HOST_PTHREAD_JOIN.get_or_init(|| {
    let symbol = resolve_host_symbol(PTHREAD_JOIN_NAME)?;
    let local_symbol = pthread_join as unsafe extern "C" fn(pthread_t, *mut *mut c_void) -> c_int;

    if symbol == local_symbol as *const () as *mut c_void {
      return None;
    }

    // SAFETY: `symbol` was returned by `dlsym` for `pthread_join` and is
    // invoked using the matching C ABI signature.
    Some(unsafe { core::mem::transmute::<*mut c_void, RealPthreadJoin>(symbol) })
  })
}

unsafe fn forward_pthread_create(
  thread: *mut pthread_t,
  attr: *const pthread_attr_t,
  start_routine: Option<StartRoutine>,
  arg: *mut c_void,
) -> c_int {
  let Some(real_pthread_create) = resolve_real_pthread_create() else {
    return EAGAIN;
  };

  // SAFETY: forwarding preserves libc ABI argument contract and function signature.
  unsafe { real_pthread_create(thread, attr, start_routine, arg) }
}

fn forward_pthread_detach(thread: pthread_t) -> c_int {
  let Some(real_pthread_detach) = resolve_real_pthread_detach() else {
    return ESRCH;
  };

  // SAFETY: forwarding preserves libc ABI argument contract and function signature.
  unsafe { real_pthread_detach(thread) }
}

unsafe fn forward_pthread_join(thread: pthread_t, retval: *mut *mut c_void) -> c_int {
  let Some(real_pthread_join) = resolve_real_pthread_join() else {
    return ESRCH;
  };

  // SAFETY: forwarding preserves libc ABI argument contract and function signature.
  unsafe { real_pthread_join(thread, retval) }
}

fn spawn_local_thread(
  thread: *mut pthread_t,
  start_routine: StartRoutine,
  arg: *mut c_void,
) -> c_int {
  let thread_id = {
    let mut registry = lock_registry();
    let thread_id = allocate_thread_id(&registry);

    registry.local_pending.insert(thread_id);

    thread_id
  };
  let arg_word = arg as usize;
  let spawn_result = with_internal_pthread_fallback(|| {
    thread::Builder::new().spawn(move || {
      CURRENT_THREAD_ID.with(|slot| slot.set(Some(thread_id)));

      let arg_ptr = arg_word as *mut c_void;
      // SAFETY: caller validates start-routine pointer/value and forwards `arg` verbatim.
      let returned = unsafe { start_routine(arg_ptr) };

      CURRENT_THREAD_ID.with(|slot| slot.set(None));

      let mut registry = lock_registry();

      registry.handle_local_thread_exit(thread_id);

      drop(registry);

      returned as usize
    })
  });
  let Ok(join_handle) = spawn_result else {
    lock_registry().local_pending.remove(&thread_id);

    return EAGAIN;
  };

  {
    let mut registry = lock_registry();

    registry.joinable.insert(thread_id, join_handle);
    registry.local_pending.remove(&thread_id);
  }

  // SAFETY: `thread` is validated non-null and points to writable memory by callers.
  unsafe {
    thread.write(thread_id);
  }

  0
}

/// C ABI entry point for `pthread_create`.
///
/// Creates a new joinable thread that executes `start_routine(arg)`.
///
/// Returns:
/// - `0` on success and writes a thread handle to `thread`
/// - `EINVAL` when `thread` is null or `start_routine` is null
/// - zero-initialized non-null attributes are treated as local defaults and
///   use the local runtime thread path
/// - native pthread error codes when non-null `attr` forwarding fails in host
///   libc
/// - `EAGAIN` when runtime thread creation fails
///
/// # Safety
/// - `thread` must point to writable storage for one [`pthread_t`].
/// - `start_routine` must be a valid callable function pointer when present.
/// - `arg` must satisfy `start_routine`'s contract.
/// - When non-null, `attr` must point to readable [`pthread_attr_t`] bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_create(
  thread: *mut pthread_t,
  attr: *const pthread_attr_t,
  start_routine: Option<StartRoutine>,
  arg: *mut c_void,
) -> c_int {
  if should_forward_internal_pthread_call() {
    // SAFETY: internal runtime fallback forwards the C ABI arguments unchanged.
    return unsafe { forward_pthread_create(thread, attr, start_routine, arg) };
  }

  if thread.is_null() {
    return EINVAL;
  }

  let Some(start) = start_routine else {
    return EINVAL;
  };

  if !attr.is_null() {
    if is_zeroed_attr_payload(attr) {
      return spawn_local_thread(thread, start, arg);
    }

    // SAFETY: top-level native attribute path forwards libc ABI arguments unchanged.
    let create_result = unsafe { forward_pthread_create(thread, attr, Some(start), arg) };

    if create_result == 0 {
      // SAFETY: successful create writes a valid native pthread handle into `thread`.
      let native_thread = unsafe { thread.read() };
      let mut registry = lock_registry();

      registry.native_joinable.insert(native_thread);
      registry.clear_native_detached(native_thread);
      registry.clear_native_consumed(native_thread);

      drop(registry);

      remember_recent_native_create(native_thread);
    }

    return create_result;
  }

  spawn_local_thread(thread, start, arg)
}

/// C ABI entry point for `pthread_detach`.
///
/// Marks `thread` as detached. Detached threads cannot be joined.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when the thread is detached and the host runtime still keeps the
///   detached handle alive
/// - `ESRCH` when a previously detached native handle has already been released
/// - `ESRCH` when no known pthread target exists for the handle
#[unsafe(no_mangle)]
pub extern "C" fn pthread_detach(thread: pthread_t) -> c_int {
  if should_forward_internal_pthread_call() {
    return forward_pthread_detach(thread);
  }

  let detach_target = {
    let mut registry = lock_registry();

    if registry.detached.contains(&thread) {
      drop(registry);

      return EINVAL;
    }

    if registry.native_consumed.contains(&thread) {
      drop(registry);

      return ESRCH;
    }

    if let Some(join_handle) = registry.joinable.remove(&thread) {
      let was_finished = registry.finished.remove(&thread);

      if !was_finished {
        registry.detached.insert(thread);
      }

      drop(registry);

      DetachTarget::Local(join_handle)
    } else if registry.native_detached.contains(&thread) {
      drop(registry);
      clear_recent_native_create(thread);

      DetachTarget::NativeDetached
    } else if registry.native_joinable.remove(&thread) {
      registry.clear_native_detached(thread);
      drop(registry);
      clear_recent_native_create(thread);

      DetachTarget::NativeJoinable
    } else {
      drop(registry);
      clear_recent_native_create(thread);

      DetachTarget::UnknownNative
    }
  };

  match detach_target {
    DetachTarget::Local(join_handle) => {
      with_internal_pthread_fallback(|| drop(join_handle));

      0
    }
    DetachTarget::NativeJoinable => {
      let detach_result = forward_pthread_detach(thread);
      let mut registry = lock_registry();

      registry.handle_forwarded_native_detach_result(thread, detach_result, true)
    }
    DetachTarget::NativeDetached => EINVAL,
    DetachTarget::UnknownNative => {
      lock_registry().mark_native_consumed(thread);

      ESRCH
    }
  }
}

/// C ABI entry point for `pthread_join`.
///
/// Waits for the target `thread` to terminate and optionally stores its return
/// value into `retval`.
///
/// Returns:
/// - `0` on success
/// - `EDEADLK` when joining the current thread
/// - `EINVAL` when the target is detached and the host runtime still keeps the
///   detached handle alive
/// - `ESRCH` when a detached native handle has already been released
/// - `ESRCH` when no known pthread target exists for the handle
///
/// # Safety
/// - When non-null, `retval` must point to writable storage for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_join(thread: pthread_t, retval: *mut *mut c_void) -> c_int {
  if should_forward_internal_pthread_call() {
    // SAFETY: internal runtime fallback forwards the C ABI arguments unchanged.
    return unsafe { forward_pthread_join(thread, retval) };
  }

  if current_thread_id() == Some(thread) {
    return EDEADLK;
  }

  let join_target = {
    let mut registry = lock_registry();

    if registry.detached.contains(&thread) {
      drop(registry);

      return EINVAL;
    }

    if registry.native_consumed.contains(&thread) {
      drop(registry);

      return ESRCH;
    }

    if let Some(join_handle) = registry.joinable.remove(&thread) {
      registry.finished.remove(&thread);

      drop(registry);

      JoinTarget::Local(join_handle)
    } else if registry.native_detached.contains(&thread) {
      drop(registry);
      clear_recent_native_create(thread);

      JoinTarget::NativeDetached
    } else if registry.native_joinable.remove(&thread) {
      registry.clear_native_detached(thread);
      drop(registry);
      clear_recent_native_create(thread);

      JoinTarget::NativeJoinable
    } else {
      drop(registry);
      clear_recent_native_create(thread);

      JoinTarget::UnknownNative
    }
  };

  match join_target {
    JoinTarget::Local(join_handle) => {
      let joined_result = with_internal_pthread_fallback(|| join_handle.join());
      let Ok(joined) = joined_result else {
        return EINVAL;
      };

      if !retval.is_null() {
        // SAFETY: `retval` is caller-provided writable storage when non-null.
        unsafe {
          retval.write(joined as *mut c_void);
        }
      }

      0
    }
    JoinTarget::NativeJoinable => {
      // SAFETY: forwarding preserves libc ABI argument contract and function signature.
      let join_result = unsafe { forward_pthread_join(thread, retval) };
      let mut registry = lock_registry();

      registry.handle_forwarded_native_join_result(thread, join_result, true)
    }
    JoinTarget::NativeDetached => EINVAL,
    JoinTarget::UnknownNative => {
      lock_registry().mark_native_consumed(thread);

      ESRCH
    }
  }
}

fn lock_mutex_lock_state(mutex_state: &PthreadMutexState) -> MutexGuard<'_, PthreadMutexLockState> {
  mutex_state
    .lock_state
    .lock()
    .unwrap_or_else(PoisonError::into_inner)
}

fn lock_cond_wait_state(cond_state: &PthreadCondState) -> MutexGuard<'_, PthreadCondWaitState> {
  cond_state
    .wait_state
    .lock()
    .unwrap_or_else(PoisonError::into_inner)
}

fn lock_cond_lazy_init() -> MutexGuard<'static, ()> {
  COND_LAZY_INIT_LOCK
    .lock()
    .unwrap_or_else(PoisonError::into_inner)
}

fn lock_rwlock_registry() -> MutexGuard<'static, HashMap<usize, Arc<PthreadRwlockState>>> {
  RWLOCK_REGISTRY
    .lock()
    .unwrap_or_else(PoisonError::into_inner)
}

fn lock_rwlock_lock_state(
  rwlock_state: &PthreadRwlockState,
) -> MutexGuard<'_, PthreadRwlockLockState> {
  rwlock_state
    .lock_state
    .lock()
    .unwrap_or_else(PoisonError::into_inner)
}

fn rwlock_key(rwlock: *mut pthread_rwlock_t) -> Result<usize, c_int> {
  if rwlock.is_null() {
    return Err(EINVAL);
  }

  Ok(rwlock.addr())
}

fn rwlock_state_from_raw(rwlock: *mut pthread_rwlock_t) -> Result<Arc<PthreadRwlockState>, c_int> {
  let key = rwlock_key(rwlock)?;
  let registry = lock_rwlock_registry();

  registry.get(&key).map(Arc::clone).ok_or(EINVAL)
}

unsafe fn mutex_state_from_raw<'a>(
  mutex: *mut pthread_mutex_t,
) -> Result<&'a PthreadMutexState, c_int> {
  if mutex.is_null() {
    return Err(EINVAL);
  }

  // SAFETY: `mutex` was validated as non-null and points to caller-managed memory.
  let state_ptr = unsafe { (*mutex).state };

  if state_ptr.is_null() {
    return Err(EINVAL);
  }

  // SAFETY: `state_ptr` originates from `Box::into_raw` during init and remains
  // valid until successful destroy.
  Ok(unsafe { &*state_ptr })
}

unsafe fn cond_state_from_raw_or_lazy_init_locked<'a>(
  cond: *mut pthread_cond_t,
) -> Result<&'a PthreadCondState, c_int> {
  if cond.is_null() {
    return Err(EINVAL);
  }

  // SAFETY: `cond` was validated as non-null and points to caller-managed memory.
  let mut state_ptr = unsafe { (*cond).state };

  if state_ptr == DESTROYED_COND_SENTINEL {
    return Err(EINVAL);
  }

  if state_ptr.is_null() {
    // SAFETY: callers serialize lazy-init transitions with `lock_cond_lazy_init`.
    unsafe {
      (*cond).state = Box::into_raw(Box::new(PthreadCondState::new()));
      state_ptr = (*cond).state;
    }
  }

  // SAFETY: `state_ptr` is guaranteed non-null and non-sentinel in this branch.
  Ok(unsafe { &*state_ptr })
}

fn mutex_lock_internal(mutex_state: &PthreadMutexState, try_only: bool) -> c_int {
  let current = thread::current().id();
  let mut lock_state = lock_mutex_lock_state(mutex_state);

  loop {
    match lock_state.owner {
      None => {
        lock_state.owner = Some(current);
        lock_state.recursion_depth = 1;

        return 0;
      }
      Some(owner) if owner == current => {
        if mutex_state.mutex_type == PTHREAD_MUTEX_RECURSIVE {
          lock_state.recursion_depth = lock_state.recursion_depth.saturating_add(1);

          return 0;
        }

        if mutex_state.mutex_type == PTHREAD_MUTEX_ERRORCHECK {
          return if try_only { EBUSY } else { EDEADLK };
        }

        if try_only {
          return EBUSY;
        }

        lock_state.waiter_count = lock_state.waiter_count.saturating_add(1);
        lock_state = mutex_state
          .wait_cv
          .wait(lock_state)
          .unwrap_or_else(PoisonError::into_inner);
        lock_state.waiter_count = lock_state.waiter_count.saturating_sub(1);
      }
      Some(_) => {
        if try_only {
          return EBUSY;
        }

        lock_state.waiter_count = lock_state.waiter_count.saturating_add(1);
        lock_state = mutex_state
          .wait_cv
          .wait(lock_state)
          .unwrap_or_else(PoisonError::into_inner);
        lock_state.waiter_count = lock_state.waiter_count.saturating_sub(1);
      }
    }
  }
}

fn mutex_unlock_internal(mutex_state: &PthreadMutexState, for_cond_wait: bool) -> c_int {
  let current = thread::current().id();
  let mut lock_state = lock_mutex_lock_state(mutex_state);

  if lock_state.owner != Some(current) {
    return EPERM;
  }

  if mutex_state.mutex_type == PTHREAD_MUTEX_RECURSIVE
    && lock_state.recursion_depth > 1
    && !for_cond_wait
  {
    lock_state.recursion_depth -= 1;

    return 0;
  }

  lock_state.owner = None;
  lock_state.recursion_depth = 0;
  drop(lock_state);
  mutex_state.wait_cv.notify_one();

  0
}

fn mutex_add_cond_waiter_reference(mutex_state: &PthreadMutexState) -> Result<usize, c_int> {
  let current = thread::current().id();
  let mut lock_state = lock_mutex_lock_state(mutex_state);

  if lock_state.owner != Some(current) {
    return Err(EPERM);
  }

  let recursive_depth = lock_state.recursion_depth;

  lock_state.cond_waiter_count = lock_state.cond_waiter_count.saturating_add(1);
  drop(lock_state);

  Ok(recursive_depth)
}

fn mutex_remove_cond_waiter_reference(mutex_state: &PthreadMutexState) {
  let mut lock_state = lock_mutex_lock_state(mutex_state);

  lock_state.cond_waiter_count = lock_state.cond_waiter_count.saturating_sub(1);
}

fn mutex_restore_cond_wait_lock_depth(
  mutex_state: &PthreadMutexState,
  recursive_depth: usize,
) -> c_int {
  let relock_result = mutex_lock_internal(mutex_state, false);

  if relock_result != 0 {
    return relock_result;
  }

  if mutex_state.mutex_type != PTHREAD_MUTEX_RECURSIVE || recursive_depth <= 1 {
    return 0;
  }

  for _ in 1..recursive_depth {
    let recursive_relock_result = mutex_lock_internal(mutex_state, false);

    if recursive_relock_result != 0 {
      return recursive_relock_result;
    }
  }

  0
}

fn rwlock_rdlock_internal(rwlock_state: &PthreadRwlockState, try_only: bool) -> c_int {
  let current = thread::current().id();
  let mut lock_state = lock_rwlock_lock_state(rwlock_state);
  let mut registered_waiter = false;

  loop {
    if lock_state.destroyed {
      if registered_waiter {
        lock_state.waiting_readers = lock_state.waiting_readers.saturating_sub(1);
      }

      return EINVAL;
    }

    if lock_state.writer_owner.is_none()
      && (lock_state.waiting_writers == 0 || lock_state.reader_owners.contains_key(&current))
    {
      if registered_waiter {
        lock_state.waiting_readers = lock_state.waiting_readers.saturating_sub(1);
      }

      let Some(next_reader_depth) = lock_state
        .reader_owners
        .get(&current)
        .copied()
        .unwrap_or(0)
        .checked_add(1)
      else {
        return EAGAIN;
      };
      let Some(next_total_readers) = lock_state.total_readers.checked_add(1) else {
        return EAGAIN;
      };

      lock_state.reader_owners.insert(current, next_reader_depth);
      lock_state.total_readers = next_total_readers;

      return 0;
    }

    if lock_state.writer_owner == Some(current) {
      if registered_waiter {
        lock_state.waiting_readers = lock_state.waiting_readers.saturating_sub(1);
      }

      return if try_only { EBUSY } else { EDEADLK };
    }

    if try_only {
      return EBUSY;
    }

    if !registered_waiter {
      lock_state.waiting_readers = lock_state.waiting_readers.saturating_add(1);
      registered_waiter = true;
    }

    lock_state = rwlock_state
      .wait_cv
      .wait(lock_state)
      .unwrap_or_else(PoisonError::into_inner);
  }
}

fn rwlock_wrlock_internal(rwlock_state: &PthreadRwlockState, try_only: bool) -> c_int {
  let current = thread::current().id();
  let mut lock_state = lock_rwlock_lock_state(rwlock_state);
  let mut registered_waiter = false;

  loop {
    if lock_state.destroyed {
      if registered_waiter {
        lock_state.waiting_writers = lock_state.waiting_writers.saturating_sub(1);
      }

      return EINVAL;
    }

    if lock_state.writer_owner == Some(current) {
      if registered_waiter {
        lock_state.waiting_writers = lock_state.waiting_writers.saturating_sub(1);
      }

      return if try_only { EBUSY } else { EDEADLK };
    }

    if lock_state.writer_owner.is_none() && lock_state.total_readers == 0 {
      if registered_waiter {
        lock_state.waiting_writers = lock_state.waiting_writers.saturating_sub(1);
      }

      lock_state.writer_owner = Some(current);

      return 0;
    }

    if try_only {
      return EBUSY;
    }

    if !registered_waiter {
      lock_state.waiting_writers = lock_state.waiting_writers.saturating_add(1);
      registered_waiter = true;
    }

    lock_state = rwlock_state
      .wait_cv
      .wait(lock_state)
      .unwrap_or_else(PoisonError::into_inner);
  }
}

fn rwlock_unlock_internal(rwlock_state: &PthreadRwlockState) -> c_int {
  let current = thread::current().id();
  let mut lock_state = lock_rwlock_lock_state(rwlock_state);

  if lock_state.destroyed {
    return EINVAL;
  }

  if lock_state.writer_owner == Some(current) {
    lock_state.writer_owner = None;
    drop(lock_state);
    rwlock_state.wait_cv.notify_all();

    return 0;
  }

  let should_notify = {
    let remove_owner = {
      let Some(reader_depth) = lock_state.reader_owners.get_mut(&current) else {
        return EINVAL;
      };

      *reader_depth -= 1;
      *reader_depth == 0
    };

    lock_state.total_readers -= 1;

    if remove_owner {
      lock_state.reader_owners.remove(&current);
    }

    lock_state.total_readers == 0
  };

  drop(lock_state);

  if should_notify {
    rwlock_state.wait_cv.notify_all();
  }

  0
}

fn timeout_from_abstime(abstime: *const timespec) -> Result<Duration, c_int> {
  if abstime.is_null() {
    return Err(EINVAL);
  }

  // SAFETY: caller must provide readable `timespec` storage.
  let deadline = unsafe { *abstime };

  if !(0..1_000_000_000).contains(&deadline.tv_nsec) {
    return Err(EINVAL);
  }

  let mut now = timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };

  if clock_gettime(CLOCK_REALTIME, &raw mut now) != 0 {
    return Err(EINVAL);
  }

  let mut sec_delta = i128::from(deadline.tv_sec) - i128::from(now.tv_sec);
  let mut nsec_delta = i128::from(deadline.tv_nsec) - i128::from(now.tv_nsec);

  if nsec_delta < 0 {
    sec_delta -= 1;
    nsec_delta += 1_000_000_000;
  }

  if sec_delta < 0 || (sec_delta == 0 && nsec_delta == 0) {
    return Ok(Duration::ZERO);
  }

  let seconds = u64::try_from(sec_delta).map_err(|_| EINVAL)?;
  let nanos = u32::try_from(nsec_delta).map_err(|_| EINVAL)?;

  Ok(Duration::new(seconds, nanos))
}

const fn validate_mutex_type(mutex_type: c_int) -> bool {
  matches!(
    mutex_type,
    PTHREAD_MUTEX_NORMAL | PTHREAD_MUTEX_RECURSIVE | PTHREAD_MUTEX_ERRORCHECK
  )
}

fn rwlock_mark_destroyed(rwlock_state: &PthreadRwlockState) -> c_int {
  let mut lock_state = lock_rwlock_lock_state(rwlock_state);

  if lock_state.destroyed {
    return EINVAL;
  }

  if lock_state.writer_owner.is_some()
    || lock_state.total_readers != 0
    || lock_state.waiting_readers != 0
    || lock_state.waiting_writers != 0
  {
    return EBUSY;
  }

  lock_state.destroyed = true;
  drop(lock_state);
  rwlock_state.wait_cv.notify_all();

  0
}

/// C ABI entry point for `pthread_mutexattr_init`.
///
/// Initializes `attr` with default values (`PTHREAD_MUTEX_DEFAULT`,
/// `PTHREAD_PROCESS_PRIVATE`). Returns `0` on success or `EINVAL` when `attr`
/// is null.
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutexattr_init(attr: *mut pthread_mutexattr_t) -> c_int {
  pthread_mutexattr_init_impl(attr)
}

fn pthread_mutexattr_init_impl(attr: *mut pthread_mutexattr_t) -> c_int {
  if attr.is_null() {
    return EINVAL;
  }

  // SAFETY: `attr` is validated non-null and points to writable storage.
  unsafe {
    (*attr).mutex_type = PTHREAD_MUTEX_DEFAULT;
    (*attr).pshared = PTHREAD_PROCESS_PRIVATE;
    (*attr).initialized = 1;
  }

  0
}

/// C ABI entry point for `pthread_mutexattr_destroy`.
///
/// Marks `attr` as uninitialized. Returns `0` on success or `EINVAL` when
/// `attr` is null.
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutexattr_destroy(attr: *mut pthread_mutexattr_t) -> c_int {
  pthread_mutexattr_destroy_impl(attr)
}

fn pthread_mutexattr_destroy_impl(attr: *mut pthread_mutexattr_t) -> c_int {
  if attr.is_null() {
    return EINVAL;
  }

  // SAFETY: `attr` is validated non-null and points to writable storage.
  unsafe {
    (*attr).initialized = 0;
  }

  0
}

/// C ABI entry point for `pthread_mutexattr_gettype`.
///
/// Writes the configured mutex type to `mutex_type`.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when pointers are null or `attr` is uninitialized
#[unsafe(no_mangle)]
pub const extern "C" fn pthread_mutexattr_gettype(
  attr: *const pthread_mutexattr_t,
  mutex_type: *mut c_int,
) -> c_int {
  pthread_mutexattr_gettype_impl(attr, mutex_type)
}

const fn pthread_mutexattr_gettype_impl(
  attr: *const pthread_mutexattr_t,
  mutex_type: *mut c_int,
) -> c_int {
  if attr.is_null() || mutex_type.is_null() {
    return EINVAL;
  }

  // SAFETY: pointers are validated non-null and expected readable/writable by
  // caller contract.
  unsafe {
    if (*attr).initialized == 0 {
      return EINVAL;
    }

    mutex_type.write((*attr).mutex_type);
  }

  0
}

/// C ABI entry point for `pthread_mutexattr_settype`.
///
/// Sets the mutex type for subsequent [`pthread_mutex_init`] calls.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when `attr` is null/uninitialized or `mutex_type` is unsupported
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutexattr_settype(
  attr: *mut pthread_mutexattr_t,
  mutex_type: c_int,
) -> c_int {
  pthread_mutexattr_settype_impl(attr, mutex_type)
}

fn pthread_mutexattr_settype_impl(attr: *mut pthread_mutexattr_t, mutex_type: c_int) -> c_int {
  if attr.is_null() || !validate_mutex_type(mutex_type) {
    return EINVAL;
  }

  // SAFETY: `attr` is validated non-null and points to writable storage.
  unsafe {
    if (*attr).initialized == 0 {
      return EINVAL;
    }

    (*attr).mutex_type = mutex_type;
  }

  0
}

/// C ABI entry point for `pthread_mutexattr_getpshared`.
///
/// Writes the configured process-shared selector to `pshared`.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when pointers are null or `attr` is uninitialized
#[unsafe(no_mangle)]
pub const extern "C" fn pthread_mutexattr_getpshared(
  attr: *const pthread_mutexattr_t,
  pshared: *mut c_int,
) -> c_int {
  pthread_mutexattr_getpshared_impl(attr, pshared)
}

const fn pthread_mutexattr_getpshared_impl(
  attr: *const pthread_mutexattr_t,
  pshared: *mut c_int,
) -> c_int {
  if attr.is_null() || pshared.is_null() {
    return EINVAL;
  }

  // SAFETY: pointers are validated non-null and expected readable/writable by
  // caller contract.
  unsafe {
    if (*attr).initialized == 0 {
      return EINVAL;
    }

    pshared.write((*attr).pshared);
  }

  0
}

/// C ABI entry point for `pthread_mutexattr_setpshared`.
///
/// Returns:
/// - `0` when setting `PTHREAD_PROCESS_PRIVATE` or `PTHREAD_PROCESS_SHARED`
/// - `EINVAL` for null/uninitialized attr or invalid `pshared` value
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutexattr_setpshared(
  attr: *mut pthread_mutexattr_t,
  pshared: c_int,
) -> c_int {
  pthread_mutexattr_setpshared_impl(attr, pshared)
}

fn pthread_mutexattr_setpshared_impl(attr: *mut pthread_mutexattr_t, pshared: c_int) -> c_int {
  if attr.is_null() {
    return EINVAL;
  }

  // SAFETY: `attr` is validated non-null and points to writable storage.
  unsafe {
    if (*attr).initialized == 0 {
      return EINVAL;
    }

    if pshared != PTHREAD_PROCESS_PRIVATE && pshared != PTHREAD_PROCESS_SHARED {
      return EINVAL;
    }

    (*attr).pshared = pshared;
  }

  0
}

/// C ABI entry point for `pthread_mutex_init`.
///
/// Initializes `mutex` using optional `attr` configuration.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` for null `mutex`, uninitialized attributes, or invalid types
/// - same-process behavior is preserved even when process-shared attributes are requested
/// - `EBUSY` when `mutex` was already initialized
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutex_init(
  mutex: *mut pthread_mutex_t,
  attr: *const pthread_mutexattr_t,
) -> c_int {
  pthread_mutex_init_impl(mutex, attr)
}

fn pthread_mutex_init_impl(mutex: *mut pthread_mutex_t, attr: *const pthread_mutexattr_t) -> c_int {
  if mutex.is_null() {
    return EINVAL;
  }

  let (mutex_type, pshared) = if attr.is_null() {
    (PTHREAD_MUTEX_DEFAULT, PTHREAD_PROCESS_PRIVATE)
  } else {
    // SAFETY: `attr` was validated non-null.
    let attr_ref = unsafe { &*attr };

    if attr_ref.initialized == 0 || !validate_mutex_type(attr_ref.mutex_type) {
      return EINVAL;
    }

    (attr_ref.mutex_type, attr_ref.pshared)
  };

  if pshared != PTHREAD_PROCESS_PRIVATE && pshared != PTHREAD_PROCESS_SHARED {
    return EINVAL;
  }

  // SAFETY: `mutex` is validated non-null and points to writable storage.
  unsafe {
    if !(*mutex).state.is_null() {
      return EBUSY;
    }

    (*mutex).state = Box::into_raw(Box::new(PthreadMutexState::new(mutex_type)));
  }

  0
}

/// C ABI entry point for `pthread_mutex_destroy`.
///
/// Destroys an initialized mutex that is not currently owned.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` for null/uninitialized mutex
/// - `EBUSY` when mutex is currently locked, has blocked lock waiters, or is
///   still referenced by blocked condition-variable waits
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutex_destroy(mutex: *mut pthread_mutex_t) -> c_int {
  pthread_mutex_destroy_impl(mutex)
}

fn pthread_mutex_destroy_impl(mutex: *mut pthread_mutex_t) -> c_int {
  if mutex.is_null() {
    return EINVAL;
  }

  // SAFETY: `mutex` is validated non-null and points to caller storage.
  let state_ptr = unsafe { (*mutex).state };

  if state_ptr.is_null() {
    return EINVAL;
  }

  // SAFETY: `state_ptr` originates from `Box::into_raw` during init.
  let state = unsafe { &*state_ptr };
  let lock_state = lock_mutex_lock_state(state);

  if lock_state.owner.is_some() || lock_state.waiter_count != 0 || lock_state.cond_waiter_count != 0
  {
    return EBUSY;
  }

  drop(lock_state);

  // SAFETY: `state_ptr` was allocated by `Box::into_raw` and is freed exactly
  // once during successful destroy.
  unsafe {
    drop(Box::from_raw(state_ptr));
    (*mutex).state = ptr::null_mut();
  }

  0
}

/// C ABI entry point for `pthread_mutex_lock`.
///
/// Blocks until the mutex can be acquired.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` for null/uninitialized mutex
/// - `EDEADLK` when re-locking an error-check mutex from the same thread
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutex_lock(mutex: *mut pthread_mutex_t) -> c_int {
  pthread_mutex_lock_impl(mutex)
}

fn pthread_mutex_lock_impl(mutex: *mut pthread_mutex_t) -> c_int {
  // SAFETY: function validates non-null and initialized state.
  let mutex_state = match unsafe { mutex_state_from_raw(mutex) } {
    Ok(mutex_state) => mutex_state,
    Err(errno) => return errno,
  };

  mutex_lock_internal(mutex_state, false)
}

/// C ABI entry point for `pthread_mutex_trylock`.
///
/// Attempts to acquire the mutex without blocking.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` for null/uninitialized mutex
/// - `EBUSY` when mutex cannot be acquired immediately
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutex_trylock(mutex: *mut pthread_mutex_t) -> c_int {
  pthread_mutex_trylock_impl(mutex)
}

fn pthread_mutex_trylock_impl(mutex: *mut pthread_mutex_t) -> c_int {
  // SAFETY: function validates non-null and initialized state.
  let mutex_state = match unsafe { mutex_state_from_raw(mutex) } {
    Ok(mutex_state) => mutex_state,
    Err(errno) => return errno,
  };

  mutex_lock_internal(mutex_state, true)
}

/// C ABI entry point for `pthread_mutex_unlock`.
///
/// Releases one level of ownership for recursive mutexes, or fully releases
/// normal/error-check mutexes.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` for null/uninitialized mutex
/// - `EPERM` when called by a non-owner thread
#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutex_unlock(mutex: *mut pthread_mutex_t) -> c_int {
  pthread_mutex_unlock_impl(mutex)
}

fn pthread_mutex_unlock_impl(mutex: *mut pthread_mutex_t) -> c_int {
  // SAFETY: function validates non-null and initialized state.
  let mutex_state = match unsafe { mutex_state_from_raw(mutex) } {
    Ok(mutex_state) => mutex_state,
    Err(errno) => return errno,
  };

  mutex_unlock_internal(mutex_state, false)
}

/// C ABI entry point for `pthread_condattr_init`.
///
/// Initializes `attr` with `PTHREAD_PROCESS_PRIVATE`.
#[unsafe(no_mangle)]
pub extern "C" fn pthread_condattr_init(attr: *mut pthread_condattr_t) -> c_int {
  pthread_condattr_init_impl(attr)
}

fn pthread_condattr_init_impl(attr: *mut pthread_condattr_t) -> c_int {
  if attr.is_null() {
    return EINVAL;
  }

  // SAFETY: `attr` is validated non-null and points to writable storage.
  unsafe {
    (*attr).pshared = PTHREAD_PROCESS_PRIVATE;
    (*attr).initialized = 1;
  }

  0
}

/// C ABI entry point for `pthread_condattr_destroy`.
///
/// Finalizes `attr`.
///
/// For libc compatibility this operation is a no-op for non-null pointers:
/// callers may still reuse `attr` with `pthread_condattr_getpshared` /
/// `pthread_condattr_setpshared` or pass it to [`pthread_cond_init`].
#[unsafe(no_mangle)]
pub const extern "C" fn pthread_condattr_destroy(attr: *mut pthread_condattr_t) -> c_int {
  pthread_condattr_destroy_impl(attr)
}

const fn pthread_condattr_destroy_impl(attr: *mut pthread_condattr_t) -> c_int {
  if attr.is_null() {
    return EINVAL;
  }

  0
}

/// C ABI entry point for `pthread_condattr_getpshared`.
///
/// Writes the configured process-shared selector into `pshared`.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when pointers are null or `attr` is uninitialized
#[unsafe(no_mangle)]
pub const extern "C" fn pthread_condattr_getpshared(
  attr: *const pthread_condattr_t,
  pshared: *mut c_int,
) -> c_int {
  pthread_condattr_getpshared_impl(attr, pshared)
}

const fn pthread_condattr_getpshared_impl(
  attr: *const pthread_condattr_t,
  pshared: *mut c_int,
) -> c_int {
  if attr.is_null() || pshared.is_null() {
    return EINVAL;
  }

  // SAFETY: pointers are validated non-null and expected readable/writable by
  // caller contract.
  unsafe {
    if (*attr).initialized == 0 {
      return EINVAL;
    }

    pshared.write((*attr).pshared);
  }

  0
}

/// C ABI entry point for `pthread_condattr_setpshared`.
///
/// Returns:
/// - `0` when setting `PTHREAD_PROCESS_PRIVATE` or `PTHREAD_PROCESS_SHARED`
/// - `EINVAL` for null/uninitialized attr or invalid `pshared` value
#[unsafe(no_mangle)]
pub extern "C" fn pthread_condattr_setpshared(
  attr: *mut pthread_condattr_t,
  pshared: c_int,
) -> c_int {
  pthread_condattr_setpshared_impl(attr, pshared)
}

fn pthread_condattr_setpshared_impl(attr: *mut pthread_condattr_t, pshared: c_int) -> c_int {
  if attr.is_null() {
    return EINVAL;
  }

  // SAFETY: `attr` is validated non-null and points to writable storage.
  unsafe {
    if (*attr).initialized == 0 {
      return EINVAL;
    }

    if pshared != PTHREAD_PROCESS_PRIVATE && pshared != PTHREAD_PROCESS_SHARED {
      return EINVAL;
    }

    (*attr).pshared = pshared;
  }

  0
}

/// C ABI entry point for `pthread_cond_init`.
///
/// Initializes `cond` for use with [`pthread_cond_wait`] and
/// [`pthread_cond_timedwait`].
///
/// Returns:
/// - `0` on success
/// - `EINVAL` for null/uninitialized arguments
/// - same-process behavior is preserved even when process-shared attributes are requested
/// - `EBUSY` when `cond` was already initialized
#[unsafe(no_mangle)]
pub extern "C" fn pthread_cond_init(
  cond: *mut pthread_cond_t,
  attr: *const pthread_condattr_t,
) -> c_int {
  pthread_cond_init_impl(cond, attr)
}

fn pthread_cond_init_impl(cond: *mut pthread_cond_t, attr: *const pthread_condattr_t) -> c_int {
  if cond.is_null() {
    return EINVAL;
  }

  if !attr.is_null() {
    // SAFETY: `attr` is validated non-null in this branch.
    let attr_ref = unsafe { &*attr };

    if attr_ref.initialized == 0 {
      return EINVAL;
    }

    if attr_ref.pshared != PTHREAD_PROCESS_PRIVATE && attr_ref.pshared != PTHREAD_PROCESS_SHARED {
      return EINVAL;
    }
  }

  let _init_guard = lock_cond_lazy_init();

  // SAFETY: `cond` is validated non-null and points to writable storage.
  unsafe {
    if !(*cond).state.is_null() && (*cond).state != DESTROYED_COND_SENTINEL {
      return EBUSY;
    }

    (*cond).state = Box::into_raw(Box::new(PthreadCondState::new()));
  }

  0
}

/// C ABI entry point for `pthread_cond_destroy`.
///
/// Destroys a condition variable that currently has no waiters.
///
/// Returns:
/// - `0` on success (including repeated destroy after a prior successful destroy)
/// - `EINVAL` for null cond pointer
/// - `EBUSY` when waiters are still blocked in wait operations
#[unsafe(no_mangle)]
pub extern "C" fn pthread_cond_destroy(cond: *mut pthread_cond_t) -> c_int {
  pthread_cond_destroy_impl(cond)
}

fn pthread_cond_destroy_impl(cond: *mut pthread_cond_t) -> c_int {
  if cond.is_null() {
    return EINVAL;
  }

  let _init_guard = lock_cond_lazy_init();

  // SAFETY: `cond` is validated non-null and points to caller storage.
  let state_ptr = unsafe { (*cond).state };

  if state_ptr == DESTROYED_COND_SENTINEL {
    return 0;
  }

  if state_ptr.is_null() {
    // Mark zero-initialized/static-initializer objects as destroyed so future
    // operations report invalid lifecycle state consistently.
    unsafe {
      (*cond).state = DESTROYED_COND_SENTINEL;
    }

    return 0;
  }

  // SAFETY: `state_ptr` originates from `Box::into_raw` during init.
  let state = unsafe { &*state_ptr };
  let wait_state = lock_cond_wait_state(state);

  if wait_state.waiter_count != 0 {
    return EBUSY;
  }

  drop(wait_state);

  // SAFETY: `state_ptr` was allocated by `Box::into_raw` and is freed exactly
  // once during successful destroy.
  unsafe {
    drop(Box::from_raw(state_ptr));
    (*cond).state = DESTROYED_COND_SENTINEL;
  }

  0
}

/// C ABI entry point for `pthread_cond_wait`.
///
/// Atomically releases `mutex`, waits for a condition signal/broadcast, then
/// re-acquires `mutex` before returning.
///
/// For `PTHREAD_MUTEX_RECURSIVE`, the wait path temporarily releases the mutex
/// fully and restores the caller's recursive lock depth before return.
///
/// A zero-initialized cond object is treated as a static initializer and is
/// lazily initialized on first wait.
///
/// During the wait/relock phase, `mutex` remains referenced by this waiter and
/// must not be destroyed.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` for null/destroyed cond or null/uninitialized mutex
/// - `EPERM` when `mutex` is not owned by the calling thread
#[unsafe(no_mangle)]
pub extern "C" fn pthread_cond_wait(
  cond: *mut pthread_cond_t,
  mutex: *mut pthread_mutex_t,
) -> c_int {
  pthread_cond_wait_impl(cond, mutex)
}

fn pthread_cond_wait_impl(cond: *mut pthread_cond_t, mutex: *mut pthread_mutex_t) -> c_int {
  let init_guard = lock_cond_lazy_init();
  // SAFETY: function validates non-null and initialized state.
  let Ok(cond_state) = (unsafe { cond_state_from_raw_or_lazy_init_locked(cond) }) else {
    return EINVAL;
  };
  // SAFETY: function validates non-null and initialized state.
  let Ok(mutex_state) = (unsafe { mutex_state_from_raw(mutex) }) else {
    return EINVAL;
  };
  let mut wait_state = lock_cond_wait_state(cond_state);

  wait_state.waiter_count = wait_state.waiter_count.saturating_add(1);

  let recursive_depth = match mutex_add_cond_waiter_reference(mutex_state) {
    Ok(recursive_depth) => recursive_depth,
    Err(errno) => {
      wait_state.waiter_count = wait_state.waiter_count.saturating_sub(1);
      drop(wait_state);

      return errno;
    }
  };
  let unlock_result = mutex_unlock_internal(mutex_state, true);

  if unlock_result != 0 {
    wait_state.waiter_count = wait_state.waiter_count.saturating_sub(1);
    drop(wait_state);
    mutex_remove_cond_waiter_reference(mutex_state);

    return unlock_result;
  }

  drop(init_guard);

  wait_state = cond_state
    .wait_cv
    .wait_while(wait_state, |state| state.pending_wakeups == 0)
    .unwrap_or_else(PoisonError::into_inner);
  wait_state.pending_wakeups = wait_state.pending_wakeups.saturating_sub(1);
  wait_state.waiter_count = wait_state.waiter_count.saturating_sub(1);
  drop(wait_state);

  let relock_result = mutex_restore_cond_wait_lock_depth(mutex_state, recursive_depth);

  mutex_remove_cond_waiter_reference(mutex_state);

  relock_result
}

/// C ABI entry point for `pthread_cond_timedwait`.
///
/// Same as [`pthread_cond_wait`], but additionally returns `ETIMEDOUT` when
/// `abstime` (absolute `CLOCK_REALTIME`) is reached before a wakeup event.
///
/// For `PTHREAD_MUTEX_RECURSIVE`, the wait path temporarily releases the mutex
/// fully and restores the caller's recursive lock depth before return.
///
/// A zero-initialized cond object is treated as a static initializer and is
/// lazily initialized on first wait.
///
/// During the wait/relock phase, `mutex` remains referenced by this waiter and
/// must not be destroyed.
///
/// Returns:
/// - `0` on signal/broadcast wakeup
/// - `ETIMEDOUT` on timeout
/// - `EINVAL` for null/destroyed cond, null/uninitialized mutex, or invalid absolute timeout
/// - `EPERM` when `mutex` is not owned by the calling thread
#[unsafe(no_mangle)]
pub extern "C" fn pthread_cond_timedwait(
  cond: *mut pthread_cond_t,
  mutex: *mut pthread_mutex_t,
  abstime: *const timespec,
) -> c_int {
  pthread_cond_timedwait_impl(cond, mutex, abstime)
}

fn pthread_cond_timedwait_impl(
  cond: *mut pthread_cond_t,
  mutex: *mut pthread_mutex_t,
  abstime: *const timespec,
) -> c_int {
  let Ok(timeout_duration) = timeout_from_abstime(abstime) else {
    return EINVAL;
  };
  let init_guard = lock_cond_lazy_init();
  // SAFETY: function validates non-null and initialized state.
  let Ok(cond_state) = (unsafe { cond_state_from_raw_or_lazy_init_locked(cond) }) else {
    return EINVAL;
  };
  // SAFETY: function validates non-null and initialized state.
  let Ok(mutex_state) = (unsafe { mutex_state_from_raw(mutex) }) else {
    return EINVAL;
  };
  let mut initial_wait_state = lock_cond_wait_state(cond_state);

  initial_wait_state.waiter_count = initial_wait_state.waiter_count.saturating_add(1);

  let recursive_depth = match mutex_add_cond_waiter_reference(mutex_state) {
    Ok(recursive_depth) => recursive_depth,
    Err(errno) => {
      initial_wait_state.waiter_count = initial_wait_state.waiter_count.saturating_sub(1);
      drop(initial_wait_state);

      return errno;
    }
  };
  let unlock_result = mutex_unlock_internal(mutex_state, true);

  if unlock_result != 0 {
    initial_wait_state.waiter_count = initial_wait_state.waiter_count.saturating_sub(1);
    drop(initial_wait_state);
    mutex_remove_cond_waiter_reference(mutex_state);

    return unlock_result;
  }

  drop(init_guard);

  let timeout_result;

  (initial_wait_state, timeout_result) = cond_state
    .wait_cv
    .wait_timeout_while(initial_wait_state, timeout_duration, |state| {
      state.pending_wakeups == 0
    })
    .unwrap_or_else(PoisonError::into_inner);

  let woke_from_signal = initial_wait_state.pending_wakeups != 0;

  if woke_from_signal {
    initial_wait_state.pending_wakeups = initial_wait_state.pending_wakeups.saturating_sub(1);
  }

  let timed_out = timeout_result.timed_out() && !woke_from_signal;

  initial_wait_state.waiter_count = initial_wait_state.waiter_count.saturating_sub(1);
  drop(initial_wait_state);

  let relock_result = mutex_restore_cond_wait_lock_depth(mutex_state, recursive_depth);

  mutex_remove_cond_waiter_reference(mutex_state);

  if relock_result != 0 {
    return relock_result;
  }

  if timed_out {
    return ETIMEDOUT;
  }

  0
}

/// C ABI entry point for `pthread_cond_signal`.
///
/// Wakes one waiter, if present.
///
/// A zero-initialized cond object (`pthread_cond_t::default()` /
/// `PTHREAD_COND_INITIALIZER`) is accepted as a no-op and returns `0`.
#[unsafe(no_mangle)]
pub extern "C" fn pthread_cond_signal(cond: *mut pthread_cond_t) -> c_int {
  pthread_cond_signal_impl(cond)
}

fn pthread_cond_signal_impl(cond: *mut pthread_cond_t) -> c_int {
  if cond.is_null() {
    return EINVAL;
  }

  let _init_guard = lock_cond_lazy_init();

  // SAFETY: `cond` is validated non-null and points to caller storage.
  let state_ptr = unsafe { (*cond).state };

  if state_ptr == DESTROYED_COND_SENTINEL {
    return EINVAL;
  }

  if state_ptr.is_null() {
    return 0;
  }

  // SAFETY: non-null non-sentinel pointer comes from `Box::into_raw`.
  let cond_state = unsafe { &*state_ptr };
  let should_notify = {
    let mut wait_state = lock_cond_wait_state(cond_state);

    if wait_state.waiter_count == 0 {
      false
    } else {
      wait_state.pending_wakeups = wait_state
        .pending_wakeups
        .saturating_add(1)
        .min(wait_state.waiter_count);

      true
    }
  };

  if should_notify {
    cond_state.wait_cv.notify_one();
  }

  0
}

/// C ABI entry point for `pthread_cond_broadcast`.
///
/// Wakes all current waiters, if any.
///
/// A zero-initialized cond object (`pthread_cond_t::default()` /
/// `PTHREAD_COND_INITIALIZER`) is accepted as a no-op and returns `0`.
#[unsafe(no_mangle)]
pub extern "C" fn pthread_cond_broadcast(cond: *mut pthread_cond_t) -> c_int {
  pthread_cond_broadcast_impl(cond)
}

fn pthread_cond_broadcast_impl(cond: *mut pthread_cond_t) -> c_int {
  if cond.is_null() {
    return EINVAL;
  }

  let _init_guard = lock_cond_lazy_init();

  // SAFETY: `cond` is validated non-null and points to caller storage.
  let state_ptr = unsafe { (*cond).state };

  if state_ptr == DESTROYED_COND_SENTINEL {
    return EINVAL;
  }

  if state_ptr.is_null() {
    return 0;
  }

  // SAFETY: non-null non-sentinel pointer comes from `Box::into_raw`.
  let cond_state = unsafe { &*state_ptr };
  let should_notify = {
    let mut wait_state = lock_cond_wait_state(cond_state);

    if wait_state.waiter_count == 0 {
      false
    } else {
      wait_state.pending_wakeups = wait_state.waiter_count;

      true
    }
  };

  if should_notify {
    cond_state.wait_cv.notify_all();
  }

  0
}

/// C ABI entry point for `pthread_rwlock_init`.
///
/// Initializes `rwlock` with default attributes.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when `rwlock` is null (this validation takes precedence)
/// - `EBUSY` when this lock storage was already initialized
///
/// # Safety
/// - `rwlock` must point to storage for one [`pthread_rwlock_t`].
/// - `attr` may be null. When non-null, it must point to readable storage for
///   one [`pthread_rwlockattr_t`]. This implementation currently ignores attr
///   bytes and applies default semantics.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_init(
  rwlock: *mut pthread_rwlock_t,
  _attr: *const pthread_rwlockattr_t,
) -> c_int {
  let Ok(key) = rwlock_key(rwlock) else {
    return EINVAL;
  };
  let mut registry = lock_rwlock_registry();

  match registry.entry(key) {
    Entry::Vacant(entry) => {
      entry.insert(Arc::new(PthreadRwlockState::new()));

      0
    }
    Entry::Occupied(_) => EBUSY,
  }
}

/// C ABI entry point for `pthread_rwlock_destroy`.
///
/// Destroys an initialized read-write lock that is not currently held.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when `rwlock` is null/uninitialized
/// - `EBUSY` when lock is currently held by reader/writer threads or still has
///   blocked waiters
///
/// # Safety
/// - `rwlock` must point to storage for one [`pthread_rwlock_t`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_destroy(rwlock: *mut pthread_rwlock_t) -> c_int {
  let Ok(key) = rwlock_key(rwlock) else {
    return EINVAL;
  };
  let Ok(rwlock_state) = rwlock_state_from_raw(rwlock) else {
    return EINVAL;
  };
  let destroy_result = rwlock_mark_destroyed(&rwlock_state);

  if destroy_result != 0 {
    return destroy_result;
  }

  let mut registry = lock_rwlock_registry();

  if let Some(current_entry) = registry.get(&key)
    && Arc::ptr_eq(current_entry, &rwlock_state)
  {
    registry.remove(&key);
  }

  0
}

/// C ABI entry point for `pthread_rwlock_rdlock`.
///
/// Acquires a read lock, blocking while a writer holds the lock.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when `rwlock` is null/uninitialized
/// - `EDEADLK` when the calling thread already holds write ownership
/// - `EAGAIN` when internal recursion/read-count tracking would overflow
///
/// # Safety
/// - `rwlock` must point to storage for one initialized [`pthread_rwlock_t`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_rdlock(rwlock: *mut pthread_rwlock_t) -> c_int {
  let Ok(rwlock_state) = rwlock_state_from_raw(rwlock) else {
    return EINVAL;
  };

  rwlock_rdlock_internal(&rwlock_state, false)
}

/// C ABI entry point for `pthread_rwlock_tryrdlock`.
///
/// Attempts to acquire a read lock without blocking.
///
/// Returns:
/// - `0` on success
/// - `EBUSY` when writer ownership prevents immediate read lock acquisition,
///   including when the caller already holds write ownership
/// - `EINVAL` when `rwlock` is null/uninitialized
/// - `EAGAIN` when internal recursion/read-count tracking would overflow
///
/// # Safety
/// - `rwlock` must point to storage for one initialized [`pthread_rwlock_t`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_tryrdlock(rwlock: *mut pthread_rwlock_t) -> c_int {
  let Ok(rwlock_state) = rwlock_state_from_raw(rwlock) else {
    return EINVAL;
  };

  rwlock_rdlock_internal(&rwlock_state, true)
}

/// C ABI entry point for `pthread_rwlock_wrlock`.
///
/// Acquires exclusive write ownership, blocking while readers or another writer
/// hold the lock.
///
/// Returns:
/// - `0` on success
/// - `EBUSY` when contention exists for `try` mode only (see `trywrlock`)
/// - `EINVAL` when `rwlock` is null/uninitialized
/// - `EDEADLK` when the calling thread already holds write ownership
///
/// # Safety
/// - `rwlock` must point to storage for one initialized [`pthread_rwlock_t`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_wrlock(rwlock: *mut pthread_rwlock_t) -> c_int {
  let Ok(rwlock_state) = rwlock_state_from_raw(rwlock) else {
    return EINVAL;
  };

  rwlock_wrlock_internal(&rwlock_state, false)
}

/// C ABI entry point for `pthread_rwlock_trywrlock`.
///
/// Attempts to acquire exclusive write ownership without blocking.
///
/// Returns:
/// - `0` on success
/// - `EBUSY` when readers or write ownership currently hold the lock,
///   including when the caller already holds write ownership
/// - `EINVAL` when `rwlock` is null/uninitialized
///
/// # Safety
/// - `rwlock` must point to storage for one initialized [`pthread_rwlock_t`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_trywrlock(rwlock: *mut pthread_rwlock_t) -> c_int {
  let Ok(rwlock_state) = rwlock_state_from_raw(rwlock) else {
    return EINVAL;
  };

  rwlock_wrlock_internal(&rwlock_state, true)
}

/// C ABI entry point for `pthread_rwlock_unlock`.
///
/// Releases read or write ownership held by the calling thread.
///
/// Returns:
/// - `0` on success
/// - `EINVAL` when `rwlock` is null/uninitialized or caller holds no ownership
///
/// # Safety
/// - `rwlock` must point to storage for one initialized [`pthread_rwlock_t`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_unlock(rwlock: *mut pthread_rwlock_t) -> c_int {
  let Ok(rwlock_state) = rwlock_state_from_raw(rwlock) else {
    return EINVAL;
  };

  rwlock_unlock_internal(&rwlock_state)
}

#[cfg(test)]
mod tests {
  use super::{
    NATIVE_CONSUMED_CACHE_LIMIT, NATIVE_DETACHED_CACHE_LIMIT, NEXT_THREAD_ID, Ordering,
    PthreadCondState, PthreadRegistry, PthreadRwlockState, allocate_thread_id, lock_cond_lazy_init,
    lock_cond_wait_state, lock_rwlock_lock_state, pthread_cond_signal, pthread_cond_t, pthread_t,
    rwlock_mark_destroyed, rwlock_rdlock_internal,
  };
  use crate::abi::errno::{EBUSY, EINVAL, ESRCH};
  use std::sync::mpsc;
  use std::thread;
  use std::time::Duration;

  #[test]
  fn mark_native_detached_removes_joinable_state() {
    let mut registry = PthreadRegistry::default();
    let thread = 11 as pthread_t;

    registry.native_joinable.insert(thread);
    registry.mark_native_detached(thread);

    assert!(!registry.native_joinable.contains(&thread));
    assert!(registry.native_detached.contains(&thread));
  }

  #[test]
  fn mark_native_detached_evicts_oldest_entries_at_capacity() {
    let mut registry = PthreadRegistry::default();
    let total = NATIVE_DETACHED_CACHE_LIMIT + 1;

    for raw in 1..=total {
      let thread =
        pthread_t::try_from(raw).unwrap_or_else(|_| unreachable!("usize must fit into pthread_t"));

      registry.mark_native_detached(thread);
    }

    assert_eq!(
      registry.native_detached_order.len(),
      NATIVE_DETACHED_CACHE_LIMIT
    );
    assert_eq!(registry.native_detached.len(), NATIVE_DETACHED_CACHE_LIMIT);
    assert!(
      !registry.native_detached.contains(&(1 as pthread_t)),
      "oldest native detached handle must be evicted once capacity is exceeded",
    );
    assert!(registry.native_detached.contains(&(total as pthread_t)));
  }

  #[test]
  fn clear_native_detached_updates_set_and_order() {
    let mut registry = PthreadRegistry::default();
    let first = 21 as pthread_t;
    let second = 22 as pthread_t;

    registry.mark_native_detached(first);
    registry.mark_native_detached(second);
    registry.clear_native_detached(first);

    assert!(!registry.native_detached.contains(&first));
    assert!(registry.native_detached.contains(&second));
    assert_eq!(registry.native_detached_order.len(), 1);
    assert_eq!(
      registry.native_detached_order.front().copied(),
      Some(second)
    );
  }

  #[test]
  fn mark_native_consumed_removes_joinable_and_detached_state() {
    let mut registry = PthreadRegistry::default();
    let thread = 31 as pthread_t;

    registry.native_joinable.insert(thread);
    registry.native_detached.insert(thread);
    registry.native_detached_order.push_back(thread);
    registry.mark_native_consumed(thread);

    assert!(!registry.native_joinable.contains(&thread));
    assert!(!registry.native_detached.contains(&thread));
    assert!(registry.native_consumed.contains(&thread));
  }

  #[test]
  fn mark_native_consumed_evicts_oldest_entries_at_capacity() {
    let mut registry = PthreadRegistry::default();
    let total = NATIVE_CONSUMED_CACHE_LIMIT + 1;

    for raw in 1..=total {
      let thread =
        pthread_t::try_from(raw).unwrap_or_else(|_| unreachable!("usize must fit into pthread_t"));

      registry.mark_native_consumed(thread);
    }

    assert_eq!(
      registry.native_consumed_order.len(),
      NATIVE_CONSUMED_CACHE_LIMIT
    );
    assert_eq!(registry.native_consumed.len(), NATIVE_CONSUMED_CACHE_LIMIT);
    assert!(
      !registry.native_consumed.contains(&(1 as pthread_t)),
      "oldest native consumed handle must be evicted once capacity is exceeded",
    );
    assert!(registry.native_consumed.contains(&(total as pthread_t)));
  }

  #[test]
  fn clear_native_consumed_updates_set_and_order() {
    let mut registry = PthreadRegistry::default();
    let first = 41 as pthread_t;
    let second = 42 as pthread_t;

    registry.mark_native_consumed(first);
    registry.mark_native_consumed(second);
    registry.clear_native_consumed(first);

    assert!(!registry.native_consumed.contains(&first));
    assert!(registry.native_consumed.contains(&second));
    assert_eq!(registry.native_consumed_order.len(), 1);
    assert_eq!(
      registry.native_consumed_order.front().copied(),
      Some(second)
    );
  }

  #[test]
  fn allocate_thread_id_skips_native_consumed_candidate() {
    let mut registry = PthreadRegistry::default();
    let first_raw = NEXT_THREAD_ID.load(Ordering::Relaxed);
    let first_candidate = pthread_t::try_from(first_raw)
      .unwrap_or_else(|_| unreachable!("u64 must fit into pthread_t"));

    registry.mark_native_consumed(first_candidate);

    let allocated = allocate_thread_id(&registry);

    assert_ne!(
      allocated, first_candidate,
      "allocator must not hand out handles already marked native-consumed",
    );
  }

  #[test]
  fn allocate_thread_id_skips_local_pending_candidate() {
    let mut registry = PthreadRegistry::default();
    let first_raw = NEXT_THREAD_ID.load(Ordering::Relaxed);
    let first_candidate = pthread_t::try_from(first_raw)
      .unwrap_or_else(|_| unreachable!("u64 must fit into pthread_t"));

    registry.local_pending.insert(first_candidate);

    let allocated = allocate_thread_id(&registry);

    assert_ne!(
      allocated, first_candidate,
      "allocator must not hand out handles still pending local publication",
    );
  }

  #[test]
  fn local_pending_thread_exit_marks_finished() {
    let mut registry = PthreadRegistry::default();
    let thread = 46 as pthread_t;

    registry.local_pending.insert(thread);
    registry.handle_local_thread_exit(thread);

    assert!(
      registry.finished.contains(&thread),
      "local thread exits that win the publication race must still be marked finished",
    );
  }

  #[test]
  fn detaching_finished_local_thread_does_not_leave_detached_state() {
    let mut registry = PthreadRegistry::default();
    let thread = 47 as pthread_t;
    let join_handle = thread::spawn(|| 0_usize);

    registry.local_pending.insert(thread);
    registry.handle_local_thread_exit(thread);
    registry.joinable.insert(thread, join_handle);
    registry.local_pending.remove(&thread);

    let join_handle = registry
      .joinable
      .remove(&thread)
      .unwrap_or_else(|| unreachable!("test published join handle"));
    let was_finished = registry.finished.remove(&thread);

    if !was_finished {
      registry.detached.insert(thread);
    }

    drop(join_handle);

    assert!(!registry.detached.contains(&thread));
    assert!(!registry.finished.contains(&thread));
  }

  #[test]
  fn forwarded_native_detach_einval_marks_detached_state() {
    let mut registry = PthreadRegistry::default();
    let thread = 51 as pthread_t;

    registry.native_joinable.insert(thread);

    let detach_result = registry.handle_forwarded_native_detach_result(thread, EINVAL, true);

    assert_eq!(detach_result, EINVAL);
    assert!(!registry.native_joinable.contains(&thread));
    assert!(registry.native_detached.contains(&thread));
  }

  #[test]
  fn forwarded_unknown_native_join_esrch_marks_consumed_state() {
    let mut registry = PthreadRegistry::default();
    let thread = 61 as pthread_t;
    let join_result = registry.handle_forwarded_native_join_result(thread, ESRCH, false);

    assert_eq!(join_result, ESRCH);
    assert!(!registry.native_joinable.contains(&thread));
    assert!(registry.native_consumed.contains(&thread));
  }

  #[test]
  fn forwarded_unknown_native_detach_einval_marks_detached_state() {
    let mut registry = PthreadRegistry::default();
    let thread = 71 as pthread_t;
    let detach_result = registry.handle_forwarded_native_detach_result(thread, EINVAL, false);

    assert_eq!(detach_result, EINVAL);
    assert!(!registry.native_joinable.contains(&thread));
    assert!(registry.native_detached.contains(&thread));
    assert!(!registry.native_consumed.contains(&thread));
  }

  #[test]
  fn forwarded_native_join_einval_marks_detached_state() {
    let mut registry = PthreadRegistry::default();
    let thread = 81 as pthread_t;

    registry.native_joinable.insert(thread);

    let join_result = registry.handle_forwarded_native_join_result(thread, EINVAL, true);

    assert_eq!(join_result, EINVAL);
    assert!(!registry.native_joinable.contains(&thread));
    assert!(registry.native_detached.contains(&thread));
    assert!(!registry.native_consumed.contains(&thread));
  }

  #[test]
  fn forwarded_detached_native_join_success_keeps_detached_state() {
    let mut registry = PthreadRegistry::default();
    let thread = 82 as pthread_t;

    registry.native_detached.insert(thread);

    let join_result = registry.handle_forwarded_native_join_result(thread, 0, false);

    assert_eq!(join_result, 0);
    assert!(registry.native_detached.contains(&thread));
    assert!(!registry.native_consumed.contains(&thread));
  }

  #[test]
  fn pthread_cond_signal_waits_for_zero_init_waiter_registration() {
    let mut cond = pthread_cond_t::default();
    let init_guard = lock_cond_lazy_init();
    let state_ptr = Box::into_raw(Box::new(PthreadCondState::new()));

    // This simulates a zero-init waiter publishing cond state before
    // waiter-count registration while the lazy-init lock is held.
    cond.state = state_ptr;

    let cond_addr = (&raw mut cond).addr();
    let (signal_result_tx, signal_result_rx) = mpsc::channel();
    let signaler = thread::spawn(move || {
      let cond_ptr = cond_addr as *mut pthread_cond_t;
      let signal_result = pthread_cond_signal(cond_ptr);

      signal_result_tx
        .send(signal_result)
        .expect("failed to send pthread_cond_signal result");
    });

    assert!(
      signal_result_rx
        .recv_timeout(Duration::from_millis(50))
        .is_err(),
      "signal must not run past lazy-init waiter registration",
    );

    let cond_state = unsafe { &*state_ptr };
    let mut wait_state = lock_cond_wait_state(cond_state);

    wait_state.waiter_count = 1;
    drop(wait_state);
    drop(init_guard);

    assert_eq!(
      signal_result_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("signal should complete after waiter registration"),
      0
    );
    signaler.join().expect("signaler thread panicked");

    let wait_state = lock_cond_wait_state(cond_state);

    assert_eq!(wait_state.pending_wakeups, 1);
    drop(wait_state);

    // SAFETY: test-created state is reclaimed exactly once here.
    unsafe { drop(Box::from_raw(state_ptr)) };
  }

  #[test]
  fn pthread_rwlock_tryrdlock_returns_ebusy_while_writer_waits() {
    let rwlock_state = PthreadRwlockState::new();
    let mut lock_state = lock_rwlock_lock_state(&rwlock_state);

    lock_state.waiting_writers = 1;
    drop(lock_state);

    assert_eq!(rwlock_rdlock_internal(&rwlock_state, true), EBUSY);
  }

  #[test]
  fn pthread_rwlock_recursive_reader_reentry_ignores_waiting_writer() {
    let rwlock_state = PthreadRwlockState::new();
    let current = thread::current().id();
    let mut lock_state = lock_rwlock_lock_state(&rwlock_state);

    lock_state.reader_owners.insert(current, 1);
    lock_state.total_readers = 1;
    lock_state.waiting_writers = 1;
    drop(lock_state);

    assert_eq!(rwlock_rdlock_internal(&rwlock_state, true), 0);

    let lock_state = lock_rwlock_lock_state(&rwlock_state);

    assert_eq!(lock_state.reader_owners.get(&current).copied(), Some(2));
    assert_eq!(lock_state.total_readers, 2);

    drop(lock_state);
  }

  #[test]
  fn pthread_rwlock_destroy_returns_ebusy_while_reader_waits() {
    let rwlock_state = PthreadRwlockState::new();
    let mut lock_state = lock_rwlock_lock_state(&rwlock_state);

    lock_state.waiting_readers = 1;
    drop(lock_state);

    assert_eq!(rwlock_mark_destroyed(&rwlock_state), EBUSY);
    assert!(
      !lock_rwlock_lock_state(&rwlock_state).destroyed,
      "destroy must not flip state while reader waiters are still queued",
    );
  }

  #[test]
  fn pthread_rwlock_destroy_returns_ebusy_while_writer_waits() {
    let rwlock_state = PthreadRwlockState::new();
    let mut lock_state = lock_rwlock_lock_state(&rwlock_state);

    lock_state.waiting_writers = 1;
    drop(lock_state);

    assert_eq!(rwlock_mark_destroyed(&rwlock_state), EBUSY);
    assert!(
      !lock_rwlock_lock_state(&rwlock_state).destroyed,
      "destroy must not flip state while writer waiters are still queued",
    );
  }
}
