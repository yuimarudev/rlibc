//! stdlib allocation C ABI functions.
//!
//! This module provides allocator-family entry points required by libc users:
//! - `malloc`
//! - `calloc`
//! - `realloc`
//! - `reallocarray`
//! - `free`
//! - `cfree`
//! - `malloc_usable_size`
//! - `aligned_alloc`
//! - `posix_memalign`
//! - `memalign`
//! - `valloc`
//! - `pvalloc`
//!
//! The implementation is split into two layers:
//! - Rust-callable implementation functions (`*_impl`) used by internal/tests.
//! - C-export wrappers with explicit symbol names (`malloc`, `calloc`, ...).
//!
//! This split keeps Rust-side calls deterministic while preserving exported
//! symbol compatibility for C ABI consumers.

use crate::abi::errno::{EINVAL, ENOMEM};
use crate::abi::types::{c_int, c_long};
use crate::errno::set_errno;
use crate::syscall::{decode_raw, syscall2, syscall6};
use core::ffi::c_void;
use core::hint::spin_loop;
use core::mem::{MaybeUninit, size_of};
use core::ptr;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

const MAP_ANONYMOUS: c_long = 0x20;
const MAP_PRIVATE: c_long = 0x02;
const MAX_REQUEST_SIZE: usize = isize::MAX as usize;
const MIN_NONZERO_ALLOCATION: usize = 1;
const PAGE_SIZE: usize = 4096;
const POINTER_SIZE: usize = size_of::<*const c_void>();
const PROT_READ: c_long = 0x1;
const PROT_WRITE: c_long = 0x2;
const SYS_MMAP: c_long = 9;
const SYS_MUNMAP: c_long = 11;
const HEADER_MAGIC: usize = 0x524C_4942_435F_414C;
static HEADER_COOKIE_ANCHOR: u8 = 0;
static ACTIVE_ALLOCATIONS_HEAD: AtomicPtr<AllocationRecord> = AtomicPtr::new(ptr::null_mut());
static ACTIVE_ALLOCATIONS_LOCK: AtomicBool = AtomicBool::new(false);
const HEADER_SIZE: usize = size_of::<AllocationHeader>();
const MALLOC_ALIGNMENT: usize = 16;
const RECORD_SIZE: usize = size_of::<AllocationRecord>();

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(C)]
struct AllocationHeader {
  magic: usize,
  mapping_base: usize,
  mapping_len: usize,
  payload_addr: usize,
  requested_size: usize,
  alignment: usize,
  cookie: usize,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct AllocationRecord {
  next: *mut Self,
  header: AllocationHeader,
}

struct AllocationListGuard;

impl Drop for AllocationListGuard {
  fn drop(&mut self) {
    ACTIVE_ALLOCATIONS_LOCK.store(false, Ordering::Release);
  }
}

const fn request_too_large(size: usize) -> bool {
  size > MAX_REQUEST_SIZE
}

fn checked_calloc_size(nmemb: usize, size: usize) -> Option<usize> {
  let total = nmemb.checked_mul(size)?;

  if request_too_large(total) {
    return None;
  }

  Some(total)
}

fn c_long_from_usize(value: usize) -> Option<c_long> {
  c_long::try_from(value).ok()
}

const fn payload_size_for(requested_size: usize) -> usize {
  if requested_size == 0 {
    return MIN_NONZERO_ALLOCATION;
  }

  requested_size
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
  let alignment_mask = alignment.checked_sub(1)?;
  let padded = value.checked_add(alignment_mask)?;

  Some(padded & !alignment_mask)
}

fn mapping_len_for(requested_size: usize, alignment: usize) -> Option<usize> {
  let payload_size = payload_size_for(requested_size);
  let alignment_padding = alignment.checked_sub(1)?;
  let total_size = HEADER_SIZE
    .checked_add(RECORD_SIZE)?
    .checked_add(payload_size)?
    .checked_add(alignment_padding)?;

  align_up(total_size, PAGE_SIZE)
}

fn allocation_cookie_seed() -> usize {
  ptr::addr_of!(HEADER_COOKIE_ANCHOR).addr().rotate_left(17) ^ HEADER_MAGIC.rotate_right(7)
}

fn allocation_cookie(
  mapping_base: usize,
  mapping_len: usize,
  payload_addr: usize,
  requested_size: usize,
  alignment: usize,
) -> usize {
  mapping_base.rotate_left(5)
    ^ mapping_len.rotate_left(11)
    ^ payload_addr.rotate_left(17)
    ^ requested_size.rotate_left(23)
    ^ alignment.rotate_left(29)
    ^ allocation_cookie_seed()
}

const fn is_page_aligned(value: usize) -> bool {
  value.is_multiple_of(PAGE_SIZE)
}

fn payload_lower_bound_for(mapping_base: usize) -> Option<usize> {
  mapping_base
    .checked_add(RECORD_SIZE)?
    .checked_add(HEADER_SIZE)
}

fn validate_header_for_payload(
  header: AllocationHeader,
  payload_addr: usize,
) -> Option<AllocationHeader> {
  if header.magic != HEADER_MAGIC || header.payload_addr != payload_addr {
    return None;
  }

  if !has_allocator_alignment_shape(header.alignment)
    || !payload_addr.is_multiple_of(header.alignment)
  {
    return None;
  }

  if header.mapping_base == 0
    || !is_page_aligned(header.mapping_base)
    || header.mapping_len < RECORD_SIZE.checked_add(HEADER_SIZE)?
    || !is_page_aligned(header.mapping_len)
  {
    return None;
  }

  let payload_size = payload_size_for(header.requested_size);
  let payload_lower_bound = payload_lower_bound_for(header.mapping_base)?;
  let mapping_end = header.mapping_base.checked_add(header.mapping_len)?;
  let payload_end = payload_addr.checked_add(payload_size)?;

  if payload_addr < payload_lower_bound || payload_end > mapping_end {
    return None;
  }

  if header.cookie
    != allocation_cookie(
      header.mapping_base,
      header.mapping_len,
      header.payload_addr,
      header.requested_size,
      header.alignment,
    )
  {
    return None;
  }

  Some(header)
}

fn lock_allocation_list() -> AllocationListGuard {
  while ACTIVE_ALLOCATIONS_LOCK
    .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
    .is_err()
  {
    spin_loop();
  }

  AllocationListGuard
}

fn register_allocation(record: NonNull<AllocationRecord>) {
  let _guard = lock_allocation_list();
  let head = ACTIVE_ALLOCATIONS_HEAD.load(Ordering::Relaxed);

  // SAFETY: allocation list lock serializes all list mutations.
  unsafe {
    (*record.as_ptr()).next = head;
  }
  ACTIVE_ALLOCATIONS_HEAD.store(record.as_ptr(), Ordering::Release);
}

fn lookup_allocation_header(payload_ptr: *mut c_void) -> Option<AllocationHeader> {
  let _guard = lock_allocation_list();
  let payload_addr = payload_ptr.addr();
  let mut cursor = ACTIVE_ALLOCATIONS_HEAD.load(Ordering::Acquire);

  while let Some(record_ptr) = NonNull::new(cursor) {
    // SAFETY: the list lock guarantees `record_ptr` stays live during traversal.
    let record = unsafe { record_ptr.as_ref() };

    if record.header.payload_addr == payload_addr {
      return Some(record.header);
    }

    cursor = record.next;
  }

  None
}

fn unregister_allocation(payload_ptr: *mut c_void) -> Option<AllocationHeader> {
  let _guard = lock_allocation_list();
  let payload_addr = payload_ptr.addr();
  let mut previous = ptr::null_mut::<AllocationRecord>();
  let mut cursor = ACTIVE_ALLOCATIONS_HEAD.load(Ordering::Acquire);

  while let Some(record_ptr) = NonNull::new(cursor) {
    // SAFETY: the list lock guarantees `record_ptr` stays live during traversal.
    let record = unsafe { &mut *record_ptr.as_ptr() };

    if record.header.payload_addr == payload_addr {
      let next = record.next;

      if previous.is_null() {
        ACTIVE_ALLOCATIONS_HEAD.store(next, Ordering::Release);
      } else {
        // SAFETY: `previous` was previously observed in the locked list.
        unsafe {
          (*previous).next = next;
        }
      }

      record.next = ptr::null_mut();

      return Some(record.header);
    }

    previous = cursor;
    cursor = record.next;
  }

  None
}

fn decode_syscall_result(raw: c_long) -> Result<usize, i32> {
  let raw_isize = isize::try_from(raw)
    .unwrap_or_else(|_| unreachable!("c_long must fit into isize on this target"));

  decode_raw(raw_isize)
}

unsafe fn mmap_allocation(mapping_len: usize) -> Result<NonNull<u8>, i32> {
  let Some(length_arg) = c_long_from_usize(mapping_len) else {
    return Err(ENOMEM);
  };

  // SAFETY: Linux x86_64 syscall arguments follow `mmap` contract.
  let raw = unsafe {
    syscall6(
      SYS_MMAP,
      0,
      length_arg,
      PROT_READ | PROT_WRITE,
      MAP_PRIVATE | MAP_ANONYMOUS,
      -1,
      0,
    )
  };
  let mapped_addr = decode_syscall_result(raw)?;
  let mapped_ptr = mapped_addr as *mut u8;

  NonNull::new(mapped_ptr).ok_or(ENOMEM)
}

unsafe fn munmap_allocation(mapping_base: *mut c_void, mapping_len: usize) {
  let Some(base_arg) = c_long_from_usize(mapping_base.addr()) else {
    return;
  };
  let Some(length_arg) = c_long_from_usize(mapping_len) else {
    return;
  };

  // SAFETY: Linux x86_64 syscall arguments follow `munmap` contract.
  let _ = unsafe { syscall2(SYS_MUNMAP, base_arg, length_arg) };
}

const unsafe fn payload_header_bytes_ptr(payload_ptr: *mut c_void) -> *mut u8 {
  // SAFETY: caller guarantees `payload_ptr` originated from this allocator.
  unsafe { payload_ptr.cast::<u8>().sub(HEADER_SIZE) }
}

unsafe fn read_payload_header(payload_ptr: *mut c_void) -> Option<AllocationHeader> {
  let mut header = MaybeUninit::<AllocationHeader>::uninit();
  let payload_addr = payload_ptr.addr();

  // SAFETY: caller guarantees `payload_ptr` originated from this allocator.
  unsafe {
    ptr::copy_nonoverlapping(
      payload_header_bytes_ptr(payload_ptr).cast_const(),
      header.as_mut_ptr().cast::<u8>(),
      HEADER_SIZE,
    );
  }
  // SAFETY: bytes were initialized by the copy above.
  let header = unsafe { header.assume_init() };

  validate_header_for_payload(header, payload_addr)
}

fn build_allocation_header(
  payload_ptr: NonNull<u8>,
  mapping_base: NonNull<u8>,
  mapping_len: usize,
  requested_size: usize,
  alignment: usize,
) -> AllocationHeader {
  let payload_addr = payload_ptr.as_ptr().addr();
  let mapping_base_addr = mapping_base.as_ptr().addr();

  AllocationHeader {
    magic: HEADER_MAGIC,
    mapping_base: mapping_base_addr,
    mapping_len,
    payload_addr,
    requested_size,
    alignment,
    cookie: allocation_cookie(
      mapping_base_addr,
      mapping_len,
      payload_addr,
      requested_size,
      alignment,
    ),
  }
}

fn write_allocation_record(
  mapping_base: NonNull<u8>,
  header: AllocationHeader,
) -> NonNull<AllocationRecord> {
  let _ = ACTIVE_ALLOCATIONS_LOCK.load(Ordering::Relaxed);
  let record_ptr = mapping_base.cast::<AllocationRecord>();

  // SAFETY: allocation record lives at the start of a writable allocation mapping.
  unsafe {
    record_ptr.as_ptr().write(AllocationRecord {
      next: ptr::null_mut(),
      header,
    });
  }

  record_ptr
}

fn write_payload_header(payload_ptr: NonNull<u8>, header: AllocationHeader) -> *mut c_void {
  let _ = ACTIVE_ALLOCATIONS_HEAD.load(Ordering::Relaxed);
  // SAFETY: `payload_ptr` was derived from the allocation mapping and has room
  // for one leading header.
  let header_bytes_ptr = unsafe { payload_ptr.as_ptr().sub(HEADER_SIZE) };

  // SAFETY: header destination points inside writable mapping and is sized.
  unsafe {
    ptr::copy_nonoverlapping(
      (&raw const header).cast::<u8>(),
      header_bytes_ptr,
      HEADER_SIZE,
    );
  }

  payload_ptr.as_ptr().cast::<c_void>()
}

fn validate_live_allocation(payload_ptr: *mut c_void) -> Option<AllocationHeader> {
  let authoritative_header = lookup_allocation_header(payload_ptr)?;
  // SAFETY: the allocation registry matched `payload_ptr` to a live allocation
  // created by this allocator, so the adjacent header range is mapped.
  let payload_header = unsafe { read_payload_header(payload_ptr) }?;

  if payload_header != authoritative_header {
    return None;
  }

  Some(authoritative_header)
}

const fn has_allocator_alignment_shape(alignment: usize) -> bool {
  alignment.is_power_of_two() && alignment != 0
}

const fn has_valid_alignment_shape(alignment: usize) -> bool {
  has_allocator_alignment_shape(alignment) && alignment >= POINTER_SIZE
}

const fn has_glibc_alignment_shape(alignment: usize) -> bool {
  has_allocator_alignment_shape(alignment)
}

const fn effective_allocation_alignment(alignment: usize) -> usize {
  if alignment < MALLOC_ALIGNMENT {
    return MALLOC_ALIGNMENT;
  }

  alignment
}

const unsafe fn zero_allocation_bytes(pointer: *mut c_void, size: usize) {
  if size == 0 {
    return;
  }

  // SAFETY: caller guarantees `pointer` is writable for `size` bytes.
  unsafe {
    ptr::write_bytes(pointer.cast::<u8>(), 0, size);
  }
}

unsafe fn allocate_with_alignment(
  requested_size: usize,
  alignment: usize,
) -> Result<*mut c_void, i32> {
  if request_too_large(requested_size) || !has_allocator_alignment_shape(alignment) {
    return Err(ENOMEM);
  }

  let effective_alignment = effective_allocation_alignment(alignment);
  let Some(mapping_len) = mapping_len_for(requested_size, effective_alignment) else {
    return Err(ENOMEM);
  };
  // SAFETY: validated mapping length is passed to Linux `mmap`.
  let mapping_base = unsafe { mmap_allocation(mapping_len)? };
  let base_addr = mapping_base.as_ptr().addr();
  let Some(payload_lower_bound) = payload_lower_bound_for(base_addr) else {
    // SAFETY: mapping was created successfully and can be reclaimed.
    unsafe {
      munmap_allocation(mapping_base.as_ptr().cast::<c_void>(), mapping_len);
    }

    return Err(ENOMEM);
  };
  let Some(payload_addr) = align_up(payload_lower_bound, effective_alignment) else {
    // SAFETY: mapping was created successfully and can be reclaimed.
    unsafe {
      munmap_allocation(mapping_base.as_ptr().cast::<c_void>(), mapping_len);
    }

    return Err(ENOMEM);
  };
  let payload_size = requested_size.max(MIN_NONZERO_ALLOCATION);
  let Some(payload_end_addr) = payload_addr.checked_add(payload_size) else {
    // SAFETY: mapping was created successfully and can be reclaimed.
    unsafe {
      munmap_allocation(mapping_base.as_ptr().cast::<c_void>(), mapping_len);
    }

    return Err(ENOMEM);
  };
  let Some(mapping_end_addr) = base_addr.checked_add(mapping_len) else {
    // SAFETY: mapping was created successfully and can be reclaimed.
    unsafe {
      munmap_allocation(mapping_base.as_ptr().cast::<c_void>(), mapping_len);
    }

    return Err(ENOMEM);
  };

  if payload_end_addr > mapping_end_addr {
    // SAFETY: mapping was created successfully and can be reclaimed.
    unsafe {
      munmap_allocation(mapping_base.as_ptr().cast::<c_void>(), mapping_len);
    }

    return Err(ENOMEM);
  }

  let Some(payload_ptr) = NonNull::new(payload_addr as *mut u8) else {
    // SAFETY: mapping was created successfully and can be reclaimed.
    unsafe {
      munmap_allocation(mapping_base.as_ptr().cast::<c_void>(), mapping_len);
    }

    return Err(ENOMEM);
  };
  let header = build_allocation_header(
    payload_ptr,
    mapping_base,
    mapping_len,
    requested_size,
    effective_alignment,
  );
  let record = write_allocation_record(mapping_base, header);
  let pointer = write_payload_header(payload_ptr, header);

  register_allocation(record);

  Ok(pointer)
}

/// Rust-callable implementation of `malloc`.
///
/// Returns a writable memory block of `size` bytes, or null on failure.
///
/// Contract notes:
/// - Requests larger than `isize::MAX` are rejected with `errno = ENOMEM`.
/// - Allocations are serviced by anonymous private memory mappings.
///
/// # Safety
/// This has C allocator semantics. The returned pointer must be passed to
/// [`free_impl`] or [`realloc_impl`] and must not be dereferenced out of bounds.
#[must_use]
pub unsafe extern "C" fn malloc_impl(size: usize) -> *mut c_void {
  // SAFETY: internal allocator requires no additional caller guarantees.
  let result = unsafe { allocate_with_alignment(size, MALLOC_ALIGNMENT) };

  match result {
    Ok(pointer) => pointer,
    Err(errno_value) => {
      set_errno(errno_value);

      ptr::null_mut()
    }
  }
}

/// Rust-callable implementation of `malloc_usable_size`.
///
/// Returns the currently usable byte capacity for a live allocation created by
/// this allocator.
///
/// Contract notes:
/// - Returns `0` for null pointers.
/// - Returns `0` for pointers that are not recognized as live allocations from
///   this allocator.
/// - Preserves `errno`.
///
/// # Safety
/// The pointer must either be null or denote memory that can be inspected by
/// this allocator's allocation registry. Unknown pointers are treated as
/// non-live and return `0`.
#[must_use]
pub unsafe extern "C" fn malloc_usable_size_impl(ptr: *mut c_void) -> usize {
  if ptr.is_null() {
    return 0;
  }

  let Some(header) = validate_live_allocation(ptr) else {
    return 0;
  };

  payload_size_for(header.requested_size)
}

/// Rust-callable implementation of `calloc`.
///
/// Returns a zero-initialized block of `nmemb * size` bytes, or null on
/// failure.
///
/// Contract notes:
/// - Multiplication overflow is detected and reported as `ENOMEM`.
/// - Requests larger than `isize::MAX` are rejected with `ENOMEM`.
///
/// # Safety
/// This has C allocator semantics. The returned pointer must be passed to
/// [`free_impl`] or [`realloc_impl`] and must not be dereferenced out of bounds.
#[must_use]
pub unsafe extern "C" fn calloc_impl(nmemb: usize, size: usize) -> *mut c_void {
  let Some(total_size) = checked_calloc_size(nmemb, size) else {
    set_errno(ENOMEM);

    return ptr::null_mut();
  };

  // SAFETY: delegation preserves `calloc` contract after overflow validation.
  let pointer = unsafe { malloc_impl(total_size) };

  if pointer.is_null() {
    return ptr::null_mut();
  }

  // SAFETY: newly allocated `pointer` is writable for `total_size` bytes.
  unsafe {
    zero_allocation_bytes(pointer, total_size);
  }

  pointer
}

/// Rust-callable implementation of `aligned_alloc`.
///
/// Returns an allocation aligned to `alignment` and sized `size`, or null on
/// failure.
///
/// Contract notes:
/// - `alignment` must be a power of two and at least pointer size.
/// - glibc-compatible behavior accepts any non-zero power-of-two alignment.
/// - Invalid arguments fail with `errno = EINVAL`.
///
/// # Safety
/// Returned pointer must be freed through [`free_impl`] and not dereferenced
/// out of bounds.
#[must_use]
pub unsafe extern "C" fn aligned_alloc_impl(alignment: usize, size: usize) -> *mut c_void {
  if !has_glibc_alignment_shape(alignment) {
    set_errno(EINVAL);

    return ptr::null_mut();
  }

  // SAFETY: internal allocator requires no additional caller guarantees.
  let result = unsafe { allocate_with_alignment(size, alignment) };

  match result {
    Ok(pointer) => pointer,
    Err(errno_value) => {
      set_errno(errno_value);

      ptr::null_mut()
    }
  }
}

/// Rust-callable implementation of `posix_memalign`.
///
/// Writes an aligned allocation pointer into `memptr` and returns status code.
///
/// Contract notes:
/// - Returns `0` on success.
/// - Returns `EINVAL` for invalid `memptr` or invalid `alignment`.
/// - Returns `ENOMEM` on allocation failure.
/// - Does not modify thread-local `errno`; caller should use return code.
///
/// # Safety
/// - `memptr` must be writable for one pointer when non-null.
/// - successful output pointer must be released with [`free_impl`].
pub unsafe extern "C" fn posix_memalign_impl(
  memptr: *mut *mut c_void,
  alignment: usize,
  size: usize,
) -> c_int {
  if memptr.is_null() || !has_valid_alignment_shape(alignment) {
    return EINVAL;
  }

  // SAFETY: internal allocator requires no additional caller guarantees.
  let result = unsafe { allocate_with_alignment(size, alignment) };

  match result {
    Ok(pointer) => {
      // SAFETY: `memptr` was validated non-null and caller guarantees writeable location.
      unsafe {
        memptr.write(pointer);
      }

      0
    }
    Err(errno_value) => errno_value,
  }
}

/// Rust-callable implementation of `memalign`.
///
/// Returns an allocation aligned to `alignment`, or null on failure.
///
/// Contract notes:
/// - `alignment` must be a power of two and at least pointer size.
/// - Invalid alignment fails with `errno = EINVAL`.
///
/// # Safety
/// Returned pointer must be freed through [`free_impl`] and not dereferenced
/// out of bounds.
#[must_use]
pub unsafe extern "C" fn memalign_impl(alignment: usize, size: usize) -> *mut c_void {
  if !has_glibc_alignment_shape(alignment) {
    set_errno(EINVAL);

    return ptr::null_mut();
  }

  // SAFETY: internal allocator requires no additional caller guarantees.
  let result = unsafe { allocate_with_alignment(size, alignment) };

  match result {
    Ok(pointer) => pointer,
    Err(errno_value) => {
      set_errno(errno_value);

      ptr::null_mut()
    }
  }
}

/// Rust-callable implementation of `valloc`.
///
/// Returns a page-aligned allocation, or null on failure.
///
/// # Safety
/// Returned pointer must be freed through [`free_impl`] and not dereferenced
/// out of bounds.
#[must_use]
pub unsafe extern "C" fn valloc_impl(size: usize) -> *mut c_void {
  // SAFETY: internal allocator requires no additional caller guarantees.
  let result = unsafe { allocate_with_alignment(size, PAGE_SIZE) };

  match result {
    Ok(pointer) => pointer,
    Err(errno_value) => {
      set_errno(errno_value);

      ptr::null_mut()
    }
  }
}

/// Rust-callable implementation of `pvalloc`.
///
/// Returns a page-aligned allocation sized up to the next page multiple.
///
/// Contract notes:
/// - `size` is rounded up to page size before allocation.
/// - Overflow during rounding fails with `errno = ENOMEM`.
///
/// # Safety
/// Returned pointer must be freed through [`free_impl`] and not dereferenced
/// out of bounds.
#[must_use]
pub unsafe extern "C" fn pvalloc_impl(size: usize) -> *mut c_void {
  let rounded_size = if size == 0 {
    PAGE_SIZE
  } else if let Some(value) = align_up(size, PAGE_SIZE) {
    value
  } else {
    set_errno(ENOMEM);

    return ptr::null_mut();
  };

  // SAFETY: delegation preserves `pvalloc` contract after rounding.
  unsafe { valloc_impl(rounded_size) }
}

/// Rust-callable implementation of `realloc`.
///
/// Resizes an existing allocation and returns a pointer to the new block, or
/// null on failure.
///
/// Contract notes:
/// - `realloc(NULL, size)` behaves like [`malloc_impl`].
/// - `realloc(ptr, 0)` frees `ptr` and returns null.
/// - Requests larger than `isize::MAX` are rejected with `ENOMEM`.
/// - Successful reallocation preserves the recorded alignment of allocator-
///   family pointers produced by this module.
///
/// # Safety
/// `ptr` must be null or a pointer previously obtained from this allocator
/// family and not yet freed.
#[must_use]
pub unsafe extern "C" fn realloc_impl(ptr: *mut c_void, size: usize) -> *mut c_void {
  if ptr.is_null() {
    // SAFETY: forwarding `realloc(NULL, size)` semantics to `malloc`.
    return unsafe { malloc_impl(size) };
  }

  if size == 0 {
    // SAFETY: `ptr` was allocated by this allocator family per contract.
    unsafe {
      free_impl(ptr);
    }

    return ptr::null_mut();
  }

  if request_too_large(size) {
    set_errno(ENOMEM);

    return ptr::null_mut();
  }

  let Some(old_header) = validate_live_allocation(ptr) else {
    return ptr::null_mut();
  };
  // SAFETY: old header validation preserves allocator metadata contract.
  let new_ptr_result = unsafe { allocate_with_alignment(size, old_header.alignment) };
  let new_ptr = match new_ptr_result {
    Ok(pointer) => pointer,
    Err(errno_value) => {
      set_errno(errno_value);

      return ptr::null_mut();
    }
  };
  let bytes_to_copy = old_header.requested_size.min(size);

  if bytes_to_copy > 0 {
    // SAFETY: source and destination are valid non-overlapping allocations.
    unsafe {
      ptr::copy_nonoverlapping(ptr.cast::<u8>(), new_ptr.cast::<u8>(), bytes_to_copy);
    }
  }

  // SAFETY: old pointer belongs to this allocator and is no longer needed.
  unsafe {
    free_impl(ptr);
  }

  new_ptr
}

/// Rust-callable implementation of `reallocarray`.
///
/// Resizes an existing allocation to hold `nmemb * size` bytes, detecting
/// multiplication overflow before delegating to [`realloc_impl`].
///
/// Contract notes:
/// - Overflow fails with `errno = ENOMEM` and returns null.
/// - On overflow failure, the original allocation remains valid and unchanged.
///
/// # Safety
/// `ptr` must be null or a pointer previously obtained from this allocator
/// family and not yet freed.
#[must_use]
pub unsafe extern "C" fn reallocarray_impl(
  ptr: *mut c_void,
  nmemb: usize,
  size: usize,
) -> *mut c_void {
  let Some(total_size) = checked_calloc_size(nmemb, size) else {
    set_errno(ENOMEM);

    return ptr::null_mut();
  };

  // SAFETY: delegation preserves `reallocarray` contract after overflow checks.
  unsafe { realloc_impl(ptr, total_size) }
}

/// Rust-callable implementation of `free`.
///
/// Releases a pointer allocated by this allocator family.
///
/// # Safety
/// `ptr` must be null or a pointer returned by this allocator family and not
/// yet freed.
pub unsafe extern "C" fn free_impl(ptr: *mut c_void) {
  if ptr.is_null() {
    return;
  }

  if validate_live_allocation(ptr).is_none() {
    return;
  }

  let Some(header) = unregister_allocation(ptr) else {
    return;
  };
  let mapping_base = header.mapping_base as *mut c_void;

  if mapping_base.is_null() || header.mapping_len < RECORD_SIZE + HEADER_SIZE {
    return;
  }

  // SAFETY: header metadata originated from a successful allocation mapping.
  unsafe {
    munmap_allocation(mapping_base, header.mapping_len);
  }
}

/// Rust-callable implementation of `cfree`.
///
/// Alias of [`free_impl`] provided for libc compatibility.
///
/// # Safety
/// Same as [`free_impl`].
pub unsafe extern "C" fn cfree_impl(ptr: *mut c_void) {
  // SAFETY: wrapper preserves `free_impl` contract.
  unsafe {
    free_impl(ptr);
  }
}

/// Exported C ABI wrapper for `malloc`.
///
/// # Safety
/// Same as [`malloc_impl`].
#[unsafe(export_name = "malloc")]
#[must_use]
pub unsafe extern "C" fn malloc_c_abi(size: usize) -> *mut c_void {
  // SAFETY: wrapper preserves `malloc_impl` contract.
  unsafe { malloc_impl(size) }
}

/// Exported C ABI wrapper for `malloc_usable_size`.
///
/// # Safety
/// Same as [`malloc_usable_size_impl`].
#[unsafe(export_name = "malloc_usable_size")]
#[must_use]
pub unsafe extern "C" fn malloc_usable_size_c_abi(ptr: *mut c_void) -> usize {
  // SAFETY: wrapper preserves `malloc_usable_size_impl` contract.
  unsafe { malloc_usable_size_impl(ptr) }
}

/// Exported C ABI wrapper for `calloc`.
///
/// # Safety
/// Same as [`calloc_impl`].
#[unsafe(export_name = "calloc")]
#[must_use]
pub unsafe extern "C" fn calloc_c_abi(nmemb: usize, size: usize) -> *mut c_void {
  // SAFETY: wrapper preserves `calloc_impl` contract.
  unsafe { calloc_impl(nmemb, size) }
}

/// Exported C ABI wrapper for `aligned_alloc`.
///
/// # Safety
/// Same as [`aligned_alloc_impl`].
#[unsafe(export_name = "aligned_alloc")]
#[must_use]
pub unsafe extern "C" fn aligned_alloc_c_abi(alignment: usize, size: usize) -> *mut c_void {
  // SAFETY: wrapper preserves `aligned_alloc_impl` contract.
  unsafe { aligned_alloc_impl(alignment, size) }
}

/// Exported C ABI wrapper for `posix_memalign`.
///
/// # Safety
/// Same as [`posix_memalign_impl`].
#[unsafe(export_name = "posix_memalign")]
pub unsafe extern "C" fn posix_memalign_c_abi(
  memptr: *mut *mut c_void,
  alignment: usize,
  size: usize,
) -> c_int {
  // SAFETY: wrapper preserves `posix_memalign_impl` contract.
  unsafe { posix_memalign_impl(memptr, alignment, size) }
}

/// Exported C ABI wrapper for `memalign`.
///
/// # Safety
/// Same as [`memalign_impl`].
#[unsafe(export_name = "memalign")]
#[must_use]
pub unsafe extern "C" fn memalign_c_abi(alignment: usize, size: usize) -> *mut c_void {
  // SAFETY: wrapper preserves `memalign_impl` contract.
  unsafe { memalign_impl(alignment, size) }
}

/// Exported C ABI wrapper for `valloc`.
///
/// # Safety
/// Same as [`valloc_impl`].
#[unsafe(export_name = "valloc")]
#[must_use]
pub unsafe extern "C" fn valloc_c_abi(size: usize) -> *mut c_void {
  // SAFETY: wrapper preserves `valloc_impl` contract.
  unsafe { valloc_impl(size) }
}

/// Exported C ABI wrapper for `pvalloc`.
///
/// # Safety
/// Same as [`pvalloc_impl`].
#[unsafe(export_name = "pvalloc")]
#[must_use]
pub unsafe extern "C" fn pvalloc_c_abi(size: usize) -> *mut c_void {
  // SAFETY: wrapper preserves `pvalloc_impl` contract.
  unsafe { pvalloc_impl(size) }
}

/// Exported C ABI wrapper for `realloc`.
///
/// # Safety
/// Same as [`realloc_impl`].
#[unsafe(export_name = "realloc")]
#[must_use]
pub unsafe extern "C" fn realloc_c_abi(ptr: *mut c_void, size: usize) -> *mut c_void {
  // SAFETY: wrapper preserves `realloc_impl` contract.
  unsafe { realloc_impl(ptr, size) }
}

/// Exported C ABI wrapper for `reallocarray`.
///
/// # Safety
/// Same as [`reallocarray_impl`].
#[unsafe(export_name = "reallocarray")]
#[must_use]
pub unsafe extern "C" fn reallocarray_c_abi(
  ptr: *mut c_void,
  nmemb: usize,
  size: usize,
) -> *mut c_void {
  // SAFETY: wrapper preserves `reallocarray_impl` contract.
  unsafe { reallocarray_impl(ptr, nmemb, size) }
}

/// Exported C ABI wrapper for `free`.
///
/// # Safety
/// Same as [`free_impl`].
#[unsafe(export_name = "free")]
pub unsafe extern "C" fn free_c_abi(ptr: *mut c_void) {
  // SAFETY: wrapper preserves `free_impl` contract.
  unsafe {
    free_impl(ptr);
  }
}

/// Exported C ABI wrapper for `cfree`.
///
/// # Safety
/// Same as [`cfree_impl`].
#[unsafe(export_name = "cfree")]
pub unsafe extern "C" fn cfree_c_abi(ptr: *mut c_void) {
  // SAFETY: wrapper preserves `cfree_impl` contract.
  unsafe {
    cfree_impl(ptr);
  }
}

#[cfg(test)]
mod tests {
  use core::ffi::c_void;
  use core::ptr;

  use super::{
    AllocationHeader, HEADER_MAGIC, HEADER_SIZE, MALLOC_ALIGNMENT, PAGE_SIZE, aligned_alloc_impl,
    allocation_cookie, free_impl, malloc_impl, memalign_impl, payload_header_bytes_ptr,
    read_payload_header, realloc_impl, reallocarray_impl,
  };

  #[repr(align(16))]
  struct ForgedAllocation {
    bytes: [u8; HEADER_SIZE + 64],
  }

  #[test]
  fn realloc_preserves_overaligned_alignment() {
    // SAFETY: allocator entry points are called with valid arguments, and all
    // reads/writes stay within allocated ranges.
    unsafe {
      let initial = aligned_alloc_impl(256, 256).cast::<u8>();

      assert!(
        !initial.is_null(),
        "aligned_alloc should provide a starting allocation for realloc",
      );
      assert_eq!(
        initial.addr() % 256,
        0,
        "initial allocation must satisfy requested over-alignment",
      );

      for index in 0_u8..64 {
        initial
          .add(usize::from(index))
          .write(index.wrapping_mul(9).wrapping_add(5));
      }

      let grown = realloc_impl(initial.cast::<c_void>(), 513).cast::<u8>();

      assert!(
        !grown.is_null(),
        "realloc should succeed for a modest growth request",
      );
      assert_eq!(
        grown.addr() % 256,
        0,
        "realloc must preserve the original allocation alignment",
      );

      for index in 0_u8..64 {
        assert_eq!(
          grown.add(usize::from(index)).read(),
          index.wrapping_mul(9).wrapping_add(5),
          "realloc must preserve the existing prefix bytes",
        );
      }

      free_impl(grown.cast::<c_void>());
    }
  }

  #[test]
  fn reallocarray_preserves_recorded_alignment() {
    // SAFETY: allocator entry points are called with valid arguments, and all
    // reads/writes stay within allocated ranges.
    unsafe {
      let initial = memalign_impl(128, 48).cast::<u8>();

      assert!(
        !initial.is_null(),
        "memalign should provide a starting allocation for reallocarray",
      );
      assert_eq!(
        initial.addr() % 128,
        0,
        "initial allocation must satisfy requested alignment",
      );

      for index in 0_u8..48 {
        initial
          .add(usize::from(index))
          .write(index.wrapping_mul(13).wrapping_add(1));
      }

      let grown = reallocarray_impl(initial.cast::<c_void>(), 5, 21).cast::<u8>();

      assert!(
        !grown.is_null(),
        "reallocarray should succeed for a modest growth request",
      );
      assert_eq!(
        grown.addr() % 128,
        0,
        "reallocarray must preserve the original allocation alignment",
      );

      for index in 0_u8..48 {
        assert_eq!(
          grown.add(usize::from(index)).read(),
          index.wrapping_mul(13).wrapping_add(1),
          "reallocarray must preserve the existing prefix bytes",
        );
      }

      free_impl(grown.cast::<c_void>());
    }
  }

  #[test]
  fn realloc_rejects_forged_header_metadata() {
    let mut forged = ForgedAllocation {
      bytes: [0; HEADER_SIZE + 64],
    };
    let payload_ptr = forged
      .bytes
      .as_mut_ptr()
      .wrapping_add(HEADER_SIZE)
      .cast::<c_void>();
    let forged_header = AllocationHeader {
      magic: HEADER_MAGIC,
      mapping_base: 1,
      mapping_len: HEADER_SIZE,
      payload_addr: payload_ptr.addr(),
      requested_size: 8,
      alignment: MALLOC_ALIGNMENT,
      cookie: 0,
    };

    // SAFETY: destination buffer is large enough to hold one header.
    unsafe {
      ptr::copy_nonoverlapping(
        (&raw const forged_header).cast::<u8>(),
        forged.bytes.as_mut_ptr(),
        HEADER_SIZE,
      );
      payload_ptr.cast::<u8>().write(0xA5);
    }

    // SAFETY: the forged pointer targets writable stack storage with an
    // adjacent fake header so the allocator can inspect and reject it.
    let grown = unsafe { realloc_impl(payload_ptr, 32) };

    assert!(
      grown.is_null(),
      "realloc must reject pointers whose adjacent header metadata is forged",
    );
    // SAFETY: payload pointer still refers to stack storage owned by this test.
    let preserved_byte = unsafe { payload_ptr.cast::<u8>().read() };

    assert_eq!(
      preserved_byte, 0xA5,
      "rejecting a forged header must leave caller-owned bytes untouched",
    );
  }

  #[test]
  fn realloc_rejects_forged_interior_pointer_metadata() {
    // SAFETY: allocator entry points are called with valid arguments, and the
    // forged metadata is written only into the live allocation owned by this test.
    unsafe {
      let original = aligned_alloc_impl(PAGE_SIZE, PAGE_SIZE * 3).cast::<u8>();

      assert!(
        !original.is_null(),
        "aligned_alloc should provide a large backing allocation for interior-pointer hardening checks",
      );

      for index in 0_u8..64 {
        original
          .add(usize::from(index))
          .write(index.wrapping_mul(5).wrapping_add(7));
      }

      let forged_ptr = original.add(PAGE_SIZE + 256).cast::<c_void>();
      let forged_mapping_base = forged_ptr.addr() & !(PAGE_SIZE - 1);
      let forged_header = AllocationHeader {
        magic: HEADER_MAGIC,
        mapping_base: forged_mapping_base,
        mapping_len: PAGE_SIZE,
        payload_addr: forged_ptr.addr(),
        requested_size: 48,
        alignment: MALLOC_ALIGNMENT,
        cookie: allocation_cookie(
          forged_mapping_base,
          PAGE_SIZE,
          forged_ptr.addr(),
          48,
          MALLOC_ALIGNMENT,
        ),
      };

      ptr::copy_nonoverlapping(
        (&raw const forged_header).cast::<u8>(),
        payload_header_bytes_ptr(forged_ptr),
        HEADER_SIZE,
      );

      let grown = realloc_impl(forged_ptr, 96);

      assert!(
        grown.is_null(),
        "realloc must reject interior pointers even when nearby bytes spoof a self-consistent allocator header",
      );

      for index in 0_u8..64 {
        assert_eq!(
          original.add(usize::from(index)).read(),
          index.wrapping_mul(5).wrapping_add(7),
          "rejecting a forged interior pointer must preserve the live allocation contents",
        );
      }

      free_impl(original.cast::<c_void>());
    }
  }

  #[test]
  fn realloc_rejects_live_pointer_with_spoofed_adjacent_header() {
    // SAFETY: allocator entry points are called with valid arguments, and the
    // test restores the original adjacent header before releasing the block.
    unsafe {
      let original = malloc_impl(96).cast::<u8>();

      assert!(
        !original.is_null(),
        "malloc should provide a live allocation before adjacent-header spoof checks",
      );

      for index in 0_u8..64 {
        original
          .add(usize::from(index))
          .write(index.wrapping_mul(11).wrapping_add(9));
      }

      let payload_ptr = original.cast::<c_void>();
      let header_bytes_ptr = payload_header_bytes_ptr(payload_ptr);
      let original_header =
        read_payload_header(payload_ptr).expect("live allocation should expose a valid header");
      let spoofed_requested_size = original_header
        .requested_size
        .checked_add(16)
        .expect("test allocation size should have room for a small spoofed growth");
      let spoofed_header = AllocationHeader {
        requested_size: spoofed_requested_size,
        cookie: allocation_cookie(
          original_header.mapping_base,
          original_header.mapping_len,
          original_header.payload_addr,
          spoofed_requested_size,
          original_header.alignment,
        ),
        ..original_header
      };
      let mut saved_header = [0_u8; HEADER_SIZE];

      ptr::copy_nonoverlapping(
        header_bytes_ptr.cast_const(),
        saved_header.as_mut_ptr(),
        HEADER_SIZE,
      );
      ptr::copy_nonoverlapping(
        (&raw const spoofed_header).cast::<u8>(),
        header_bytes_ptr,
        HEADER_SIZE,
      );

      assert!(
        read_payload_header(payload_ptr) == Some(spoofed_header),
        "the spoofed adjacent header should stay syntactically valid so the authoritative registry is the deciding check",
      );

      let grown = realloc_impl(payload_ptr, 160);

      assert!(
        grown.is_null(),
        "realloc must reject live allocations whose adjacent header no longer matches allocator-owned metadata",
      );

      for index in 0_u8..64 {
        assert_eq!(
          original.add(usize::from(index)).read(),
          index.wrapping_mul(11).wrapping_add(9),
          "rejecting a spoofed adjacent header must preserve the live allocation contents",
        );
      }

      ptr::copy_nonoverlapping(saved_header.as_ptr(), header_bytes_ptr, HEADER_SIZE);
      free_impl(payload_ptr);
    }
  }
}
