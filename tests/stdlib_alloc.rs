use core::ffi::{c_int, c_void};
use core::ptr;
use rlibc::abi::errno::{EINVAL, ENOMEM};
use rlibc::errno::__errno_location;
use rlibc::stdlib::{
  aligned_alloc, calloc, cfree, free, malloc, malloc_usable_size, memalign, posix_memalign,
  pvalloc, realloc, reallocarray, valloc,
};

const PAGE_SIZE: usize = 4096;
const POINTER_SIZE: usize = core::mem::size_of::<*const c_void>();

fn read_errno() -> c_int {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe { __errno_location().read() }
}

fn write_errno(value: c_int) {
  // SAFETY: `__errno_location` returns writable thread-local errno storage.
  unsafe {
    __errno_location().write(value);
  }
}

const fn allocation_pattern(index: usize, seed: u8) -> u8 {
  index.to_le_bytes()[0].wrapping_mul(19).wrapping_add(seed)
}

unsafe fn write_pattern(ptr: *mut u8, len: usize, seed: u8) {
  for index in 0..len {
    // SAFETY: caller guarantees `ptr.add(index)` is within a writable allocation.
    unsafe {
      ptr.add(index).write(allocation_pattern(index, seed));
    }
  }
}

unsafe fn assert_pattern(ptr: *mut u8, len: usize, seed: u8, context: &str) {
  for index in 0..len {
    // SAFETY: caller guarantees `ptr.add(index)` is within a readable allocation.
    let byte = unsafe { ptr.add(index).read() };

    assert_eq!(
      byte,
      allocation_pattern(index, seed),
      "{context} at byte {index}",
    );
  }
}

#[test]
fn malloc_and_free_round_trip_nonzero_size() {
  // SAFETY: C allocator entry points are called with valid arguments, and the
  // returned pointer is used only within the allocated range.
  unsafe {
    let ptr = malloc(32).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "malloc(32) should return a usable non-null pointer in normal conditions",
    );

    for index in 0_u8..32 {
      ptr.add(usize::from(index)).write(index.wrapping_mul(3));
    }

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn calloc_zero_initializes_full_region() {
  // SAFETY: C allocator entry points are called with valid arguments, and the
  // returned pointer is read within the requested region.
  unsafe {
    let ptr = calloc(16, 4).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "calloc should return non-null for modest sizes"
    );

    for index in 0..64 {
      assert_eq!(
        ptr.add(index).read(),
        0,
        "calloc must zero-initialize each requested byte",
      );
    }

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn calloc_zeroes_same_sized_region_after_nonzero_malloc_contents() {
  // SAFETY: allocator entry points are called with valid arguments, and all
  // reads/writes stay within the requested region size.
  unsafe {
    let len = 96;
    let dirty = malloc(len).cast::<u8>();

    assert!(
      !dirty.is_null(),
      "malloc should allocate block before calloc reuse check",
    );

    write_pattern(dirty, len, 0x41);
    free(dirty.cast::<c_void>());

    let zeroed = calloc(24, 4).cast::<u8>();

    assert!(
      !zeroed.is_null(),
      "calloc should allocate same-sized block after prior malloc/free cycle",
    );

    for index in 0..len {
      assert_eq!(
        zeroed.add(index).read(),
        0,
        "calloc must provide zeroed bytes even when a same-sized allocation previously held non-zero data at byte {index}",
      );
    }

    free(zeroed.cast::<c_void>());
  }
}

#[test]
fn realloc_grow_preserves_existing_prefix_bytes() {
  // SAFETY: C allocator entry points are called with valid arguments, and
  // pointer arithmetic stays within allocated ranges.
  unsafe {
    let initial = malloc(8).cast::<u8>();

    assert!(!initial.is_null(), "malloc should allocate initial block");

    for index in 0_u8..8 {
      initial
        .add(usize::from(index))
        .write(index.wrapping_add(11));
    }

    let grown = realloc(initial.cast::<c_void>(), 40).cast::<u8>();

    assert!(
      !grown.is_null(),
      "realloc should grow block for modest sizes"
    );

    for index in 0_u8..8 {
      assert_eq!(
        grown.add(usize::from(index)).read(),
        index.wrapping_add(11),
        "realloc must preserve bytes from the old allocation prefix",
      );
    }

    free(grown.cast::<c_void>());
  }
}

#[test]
fn realloc_growth_exposes_writable_tail_without_clobbering_prefix() {
  // SAFETY: allocator entry points are called with valid arguments, and all
  // pointer arithmetic remains within allocated bounds.
  unsafe {
    let old_len = 13;
    let new_len = 61;
    let initial = malloc(old_len).cast::<u8>();

    assert!(
      !initial.is_null(),
      "malloc should allocate block before realloc growth tail check",
    );

    write_pattern(initial, old_len, 0x23);

    let grown = realloc(initial.cast::<c_void>(), new_len).cast::<u8>();

    assert!(
      !grown.is_null(),
      "realloc should grow block before tail usability check",
    );

    assert_pattern(
      grown,
      old_len,
      0x23,
      "realloc must preserve existing prefix before tail writes",
    );

    for index in old_len..new_len {
      grown
        .add(index)
        .write(allocation_pattern(index - old_len, 0x6D));
    }

    assert_pattern(
      grown,
      old_len,
      0x23,
      "realloc tail writes must not clobber preserved prefix",
    );

    for index in old_len..new_len {
      assert_eq!(
        grown.add(index).read(),
        allocation_pattern(index - old_len, 0x6D),
        "realloc-grown tail must remain readable and writable at byte {index}",
      );
    }

    free(grown.cast::<c_void>());
  }
}

#[test]
fn realloc_with_null_pointer_behaves_like_malloc() {
  // SAFETY: C allocator entry points are called with valid arguments.
  unsafe {
    let ptr = realloc(ptr::null_mut(), 24).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "realloc(NULL, n) should behave like malloc(n) for modest n",
    );

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn realloc_zero_size_frees_and_returns_null() {
  // SAFETY: C allocator entry points are called with valid arguments.
  unsafe {
    let ptr = malloc(24);

    assert!(
      !ptr.is_null(),
      "malloc should allocate block before realloc"
    );

    let shrunk = realloc(ptr, 0);

    assert!(
      shrunk.is_null(),
      "realloc(ptr, 0) should return null after releasing allocation",
    );
  }
}

#[test]
fn realloc_failed_growth_keeps_original_allocation_usable() {
  // SAFETY: C allocator entry points are called with valid arguments, and
  // pointer accesses remain within the allocated range.
  unsafe {
    let ptr = malloc(16).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "malloc should allocate block before realloc failure-path check",
    );

    for index in 0_u8..16 {
      ptr
        .add(usize::from(index))
        .write(index.wrapping_mul(7).wrapping_add(1));
    }

    write_errno(0);

    let grown = realloc(ptr.cast::<c_void>(), usize::MAX).cast::<u8>();

    assert!(
      grown.is_null(),
      "realloc overflow must fail and return null pointer",
    );
    assert_eq!(
      read_errno(),
      ENOMEM,
      "realloc overflow should set errno to ENOMEM",
    );

    for index in 0_u8..16 {
      assert_eq!(
        ptr.add(usize::from(index)).read(),
        index.wrapping_mul(7).wrapping_add(1),
        "failed realloc must keep original allocation contents intact",
      );
    }

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn reallocarray_grow_preserves_existing_prefix_bytes() {
  // SAFETY: C allocator entry points are called with valid arguments, and
  // pointer arithmetic stays within allocated ranges.
  unsafe {
    let initial = malloc(10).cast::<u8>();

    assert!(!initial.is_null(), "malloc should allocate initial block");

    for index in 0_u8..10 {
      initial
        .add(usize::from(index))
        .write(index.wrapping_mul(5).wrapping_add(3));
    }

    let grown = reallocarray(initial.cast::<c_void>(), 8, 8).cast::<u8>();

    assert!(
      !grown.is_null(),
      "reallocarray should grow block for modest element counts",
    );

    for index in 0_u8..10 {
      assert_eq!(
        grown.add(usize::from(index)).read(),
        index.wrapping_mul(5).wrapping_add(3),
        "reallocarray must preserve bytes from old allocation prefix",
      );
    }

    free(grown.cast::<c_void>());
  }
}

#[test]
fn reallocarray_overflow_sets_errno_enomem_and_keeps_original_allocation_usable() {
  // SAFETY: C allocator entry points are called with valid arguments, and
  // pointer accesses remain within the allocated range.
  unsafe {
    let ptr = malloc(16).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "malloc should allocate block before reallocarray overflow check",
    );

    for index in 0_u8..16 {
      ptr
        .add(usize::from(index))
        .write(index.wrapping_mul(11).wrapping_add(7));
    }

    write_errno(0);

    let grown = reallocarray(ptr.cast::<c_void>(), usize::MAX, 2).cast::<u8>();

    assert!(
      grown.is_null(),
      "reallocarray overflow must fail and return null pointer",
    );
    assert_eq!(
      read_errno(),
      ENOMEM,
      "reallocarray overflow should set errno to ENOMEM",
    );

    for index in 0_u8..16 {
      assert_eq!(
        ptr.add(usize::from(index)).read(),
        index.wrapping_mul(11).wrapping_add(7),
        "failed reallocarray must keep original allocation contents intact",
      );
    }

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn calloc_overflow_sets_errno_enomem() {
  write_errno(0);

  // SAFETY: C allocator entry point is called with value arguments only.
  let ptr = unsafe { calloc(usize::MAX, 2) };

  assert!(
    ptr.is_null(),
    "calloc overflow should fail and return null pointer",
  );
  assert_eq!(
    read_errno(),
    ENOMEM,
    "calloc overflow should set errno to ENOMEM",
  );
}

#[test]
fn malloc_layout_overflow_sets_errno_enomem() {
  write_errno(0);

  // SAFETY: C allocator entry point is called with value arguments only.
  let ptr = unsafe { malloc(usize::MAX) };

  assert!(
    ptr.is_null(),
    "malloc layout overflow should fail and return null pointer",
  );
  assert_eq!(
    read_errno(),
    ENOMEM,
    "malloc layout overflow should set errno to ENOMEM",
  );
}

#[test]
fn zero_size_allocator_entry_points_return_freeable_non_null_pointers() {
  // SAFETY: zero-size allocator entry points either return null on failure or
  // a pointer that remains valid to free.
  unsafe {
    let allocations = [
      malloc(0),
      calloc(0, 32),
      calloc(32, 0),
      realloc(ptr::null_mut(), 0),
      reallocarray(ptr::null_mut(), 0, 32),
      reallocarray(ptr::null_mut(), 32, 0),
    ];

    for (index, allocation) in allocations.into_iter().enumerate() {
      assert!(
        !allocation.is_null(),
        "zero-size allocator case {index} should return a freeable non-null pointer",
      );

      free(allocation);
    }
  }
}

#[test]
fn malloc_usable_size_null_pointer_returns_zero_without_touching_errno() {
  write_errno(7331);

  let usable = unsafe { malloc_usable_size(ptr::null_mut()) };

  assert_eq!(usable, 0, "malloc_usable_size(NULL) should report zero");
  assert_eq!(
    read_errno(),
    7331,
    "malloc_usable_size(NULL) should not modify errno",
  );
}

#[test]
fn malloc_usable_size_reports_live_capacity_for_zero_size_allocations() {
  // SAFETY: each returned pointer is either null on failure or a live allocation
  // that remains valid for `malloc_usable_size` and `free`.
  unsafe {
    let allocations = [
      malloc(0),
      calloc(0, 32),
      calloc(32, 0),
      realloc(ptr::null_mut(), 0),
      reallocarray(ptr::null_mut(), 0, 32),
      reallocarray(ptr::null_mut(), 32, 0),
    ];

    for (index, allocation) in allocations.into_iter().enumerate() {
      assert!(
        !allocation.is_null(),
        "zero-size allocator case {index} should return a live allocation",
      );

      let usable = malloc_usable_size(allocation);

      assert!(
        usable >= 1,
        "zero-size allocator case {index} should expose non-zero usable capacity",
      );

      free(allocation);
    }
  }
}

#[test]
fn malloc_usable_size_reports_at_least_requested_size_for_live_allocations() {
  // SAFETY: each allocator call uses valid arguments, and all returned pointers
  // remain live until released with `free`.
  unsafe {
    let malloc_ptr = malloc(1);
    let calloc_ptr = calloc(4, 8);
    let aligned_ptr = aligned_alloc(64, 65);

    for (label, allocation, requested_size) in [
      ("malloc", malloc_ptr, 1_usize),
      ("calloc", calloc_ptr, 32_usize),
      ("aligned_alloc", aligned_ptr, 65_usize),
    ] {
      assert!(
        !allocation.is_null(),
        "{label} should provide a live allocation before usable-size inspection",
      );

      let usable = malloc_usable_size(allocation);

      assert!(
        usable >= requested_size,
        "{label} usable size should cover the requested size",
      );

      free(allocation);
    }
  }
}

#[test]
fn zero_size_aligned_allocator_entry_points_preserve_alignment() {
  // SAFETY: successful zero-size aligned allocations return freeable pointers.
  unsafe {
    let aligned = aligned_alloc(64, 0).cast::<u8>();

    assert!(
      !aligned.is_null(),
      "aligned_alloc(64, 0) should return a freeable pointer",
    );
    assert_eq!(
      aligned.addr() % 64,
      0,
      "aligned_alloc(64, 0) must preserve requested alignment",
    );
    free(aligned.cast::<c_void>());

    let memaligned = memalign(64, 0).cast::<u8>();

    assert!(
      !memaligned.is_null(),
      "memalign(64, 0) should return a freeable pointer",
    );
    assert_eq!(
      memaligned.addr() % 64,
      0,
      "memalign(64, 0) must preserve requested alignment",
    );
    free(memaligned.cast::<c_void>());

    let page_aligned = valloc(0).cast::<u8>();

    assert!(
      !page_aligned.is_null(),
      "valloc(0) should return a freeable pointer",
    );
    assert_eq!(
      page_aligned.addr() % PAGE_SIZE,
      0,
      "valloc(0) must preserve page alignment",
    );
    free(page_aligned.cast::<c_void>());

    let page_rounded = pvalloc(0).cast::<u8>();

    assert!(
      !page_rounded.is_null(),
      "pvalloc(0) should return a freeable pointer",
    );
    assert_eq!(
      page_rounded.addr() % PAGE_SIZE,
      0,
      "pvalloc(0) must preserve page alignment",
    );
    free(page_rounded.cast::<c_void>());
  }
}

#[test]
fn posix_memalign_zero_size_returns_aligned_freeable_pointer() {
  let mut out = ptr::null_mut::<c_void>();

  // SAFETY: `out` pointer is valid for one pointer-sized write.
  let status = unsafe { posix_memalign(ptr::addr_of_mut!(out), 128, 0) };

  assert_eq!(
    status, 0,
    "posix_memalign should succeed for zero-size allocation requests",
  );
  assert!(
    !out.is_null(),
    "posix_memalign should return a freeable pointer for zero-size requests",
  );
  assert_eq!(
    out.addr() % 128,
    0,
    "posix_memalign zero-size result must preserve requested alignment",
  );

  // SAFETY: successful `posix_memalign` output is released with `free`.
  unsafe {
    free(out);
  }
}

#[test]
fn aligned_alloc_returns_pointer_aligned_to_requested_boundary() {
  // SAFETY: C allocator entry point is called with valid arguments and freed afterwards.
  unsafe {
    let ptr = aligned_alloc(64, 128).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "aligned_alloc should succeed for valid alignment and size multiple",
    );
    assert_eq!(
      ptr.addr() % 64,
      0,
      "aligned_alloc pointer must satisfy requested alignment",
    );

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn aligned_alloc_accepts_pointer_sized_alignment_and_full_requested_size() {
  // SAFETY: allocator entry point is called with valid arguments, and writes
  // remain within the requested size before freeing the returned block.
  unsafe {
    let size = POINTER_SIZE * 3;
    let ptr = aligned_alloc(POINTER_SIZE, size).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "aligned_alloc should accept pointer-sized alignment with size multiple",
    );
    assert_eq!(
      ptr.addr() % POINTER_SIZE,
      0,
      "aligned_alloc pointer must satisfy the minimum valid alignment",
    );

    write_pattern(ptr, size, 0x52);
    assert_pattern(
      ptr,
      size,
      0x52,
      "aligned_alloc allocation must expose the full requested size",
    );

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn aligned_alloc_alignment_smaller_than_pointer_size_is_still_accepted() {
  // SAFETY: glibc-compatible aligned_alloc accepts any non-zero power-of-two alignment.
  unsafe {
    let ptr = aligned_alloc(POINTER_SIZE / 2, POINTER_SIZE).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "aligned_alloc should accept power-of-two alignments smaller than pointer size",
    );
    assert_eq!(
      ptr.addr() % (POINTER_SIZE / 2),
      0,
      "aligned_alloc pointer must satisfy the requested smaller power-of-two alignment",
    );

    write_pattern(ptr, POINTER_SIZE, 0x61);
    assert_pattern(
      ptr,
      POINTER_SIZE,
      0x61,
      "aligned_alloc smaller alignment must still expose the full requested size",
    );

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn aligned_alloc_non_multiple_size_is_still_usable() {
  // SAFETY: glibc-compatible aligned_alloc accepts non-multiple sizes.
  unsafe {
    let ptr = aligned_alloc(64, 65).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "aligned_alloc should accept non-multiple sizes under glibc-compatible semantics",
    );
    assert_eq!(
      ptr.addr() % 64,
      0,
      "aligned_alloc pointer must satisfy requested alignment"
    );

    write_pattern(ptr, 65, 0x6B);
    assert_pattern(
      ptr,
      65,
      0x6B,
      "aligned_alloc non-multiple size must expose the full requested size",
    );

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn posix_memalign_writes_aligned_pointer_and_returns_zero() {
  let mut out = ptr::null_mut::<c_void>();

  // SAFETY: `out` pointer is valid for one pointer-sized write.
  let status = unsafe { posix_memalign(ptr::addr_of_mut!(out), 128, 256) };

  assert_eq!(status, 0, "posix_memalign should return zero on success");
  assert!(
    !out.is_null(),
    "posix_memalign must write non-null pointer on success"
  );
  assert_eq!(
    out.addr() % 128,
    0,
    "posix_memalign output pointer must satisfy requested alignment",
  );

  // SAFETY: `out` was allocated by allocator API and can be released with free.
  unsafe {
    free(out);
  }
}

#[test]
fn posix_memalign_accepts_pointer_sized_alignment_for_small_size() {
  let mut out = ptr::null_mut::<c_void>();

  // SAFETY: `out` pointer is valid for one pointer-sized write.
  let status = unsafe { posix_memalign(ptr::addr_of_mut!(out), POINTER_SIZE, 1) };

  assert_eq!(
    status, 0,
    "posix_memalign should accept the minimum valid alignment",
  );
  assert!(
    !out.is_null(),
    "posix_memalign should return a usable pointer on minimum valid alignment",
  );
  assert_eq!(
    out.addr() % POINTER_SIZE,
    0,
    "posix_memalign output must satisfy the minimum valid alignment",
  );

  // SAFETY: `out` was allocated by allocator API and exposes at least one byte.
  unsafe {
    out.cast::<u8>().write(0xA7);
    free(out);
  }
}

#[test]
fn posix_memalign_invalid_alignment_returns_einval_without_overwriting_output() {
  let sentinel = ptr::dangling_mut::<c_void>();
  let mut out = sentinel;

  write_errno(55);

  // SAFETY: `out` pointer is valid for one pointer-sized write.
  let status = unsafe { posix_memalign(ptr::addr_of_mut!(out), 24, 128) };

  assert_eq!(
    status, EINVAL,
    "posix_memalign should return EINVAL for non-power-of-two alignment",
  );
  assert_eq!(
    out, sentinel,
    "posix_memalign must not overwrite output pointer on invalid alignment",
  );
  assert_eq!(
    read_errno(),
    55,
    "posix_memalign should not touch errno and should report via return code",
  );
}

#[test]
fn posix_memalign_too_small_alignment_returns_einval_without_overwriting_output() {
  let sentinel = ptr::dangling_mut::<c_void>();
  let mut out = sentinel;

  write_errno(91);

  // SAFETY: `out` pointer is valid for one pointer-sized write.
  let status = unsafe { posix_memalign(ptr::addr_of_mut!(out), POINTER_SIZE / 2, 64) };

  assert_eq!(
    status, EINVAL,
    "posix_memalign should reject alignments smaller than pointer size",
  );
  assert_eq!(
    out, sentinel,
    "posix_memalign must preserve output pointer for too-small alignment",
  );
  assert_eq!(
    read_errno(),
    91,
    "posix_memalign should continue reporting invalid alignment via return code only",
  );
}

#[test]
fn posix_memalign_reports_enomem_and_keeps_output_on_allocation_failure() {
  let sentinel = ptr::dangling_mut::<c_void>();
  let mut out = sentinel;

  write_errno(73);

  // SAFETY: `out` pointer is valid for one pointer-sized write.
  let status = unsafe { posix_memalign(ptr::addr_of_mut!(out), 64, usize::MAX) };

  assert_eq!(
    status, ENOMEM,
    "posix_memalign should return ENOMEM when allocation cannot be satisfied",
  );
  assert_eq!(
    out, sentinel,
    "posix_memalign must preserve output pointer when allocation fails",
  );
  assert_eq!(
    read_errno(),
    73,
    "posix_memalign should not update errno on allocation failure",
  );
}

#[test]
fn memalign_returns_requested_alignment() {
  // SAFETY: C allocator entry point is called with valid arguments and pointer is freed.
  unsafe {
    let ptr = memalign(256, 64).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "memalign should succeed for valid alignment and size",
    );
    assert_eq!(
      ptr.addr() % 256,
      0,
      "memalign pointer must satisfy requested alignment",
    );

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn memalign_invalid_alignment_sets_errno_einval() {
  write_errno(0);

  // SAFETY: C allocator entry point is called with value arguments only.
  let ptr = unsafe { memalign(24, 64) };

  assert!(
    ptr.is_null(),
    "memalign must fail for non-power-of-two alignment",
  );
  assert_eq!(
    read_errno(),
    EINVAL,
    "memalign invalid alignment should set errno to EINVAL",
  );
}

#[test]
fn memalign_too_small_alignment_is_still_accepted() {
  // SAFETY: glibc-compatible memalign accepts any non-zero power-of-two alignment.
  unsafe {
    let ptr = memalign(POINTER_SIZE / 2, 64).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "memalign should accept power-of-two alignments smaller than pointer size",
    );
    assert_eq!(
      ptr.addr() % (POINTER_SIZE / 2),
      0,
      "memalign pointer must satisfy the requested smaller power-of-two alignment",
    );

    write_pattern(ptr, 64, 0x71);
    assert_pattern(
      ptr,
      64,
      0x71,
      "memalign smaller alignment must still expose the full requested size",
    );

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn valloc_returns_page_aligned_pointer() {
  // SAFETY: C allocator entry point is called with valid argument and pointer is freed.
  unsafe {
    let ptr = valloc(128).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "valloc should succeed for modest allocation"
    );
    assert_eq!(ptr.addr() % 4096, 0, "valloc pointer must be page aligned");

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn pvalloc_returns_page_aligned_pointer() {
  // SAFETY: C allocator entry point is called with valid argument and pointer is freed.
  unsafe {
    let ptr = pvalloc(129).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "pvalloc should succeed for modest allocation"
    );
    assert_eq!(ptr.addr() % 4096, 0, "pvalloc pointer must be page aligned");

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn pvalloc_rounds_up_size_to_next_page_and_exposes_tail_byte() {
  // SAFETY: allocator entry point is called with valid arguments, and writes
  // remain within the rounded page-sized allocation before freeing it.
  unsafe {
    let ptr = pvalloc(PAGE_SIZE + 1).cast::<u8>();

    assert!(
      !ptr.is_null(),
      "pvalloc should allocate block before rounded-size coverage check",
    );
    assert_eq!(
      ptr.addr() % PAGE_SIZE,
      0,
      "pvalloc pointer must stay page aligned"
    );

    ptr.add((PAGE_SIZE * 2) - 1).write(0x5C);
    assert_eq!(
      ptr.add((PAGE_SIZE * 2) - 1).read(),
      0x5C,
      "pvalloc should expose the full rounded-up page multiple",
    );

    free(ptr.cast::<c_void>());
  }
}

#[test]
fn pvalloc_overflow_sets_errno_enomem() {
  write_errno(0);

  // SAFETY: C allocator entry point is called with value arguments only.
  let ptr = unsafe { pvalloc(usize::MAX) };

  assert!(ptr.is_null(), "pvalloc overflow should return null pointer");
  assert_eq!(
    read_errno(),
    ENOMEM,
    "pvalloc overflow should set errno to ENOMEM",
  );
}

#[test]
fn cfree_releases_malloc_block_like_free() {
  // SAFETY: C allocator entry points are called with valid arguments.
  unsafe {
    let ptr = malloc(48);

    assert!(!ptr.is_null(), "malloc should allocate block before cfree");
    cfree(ptr);
  }
}
