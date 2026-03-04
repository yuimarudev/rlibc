use rlibc::abi::types::size_t;
use rlibc::memory::{memcmp, memcpy, memmove, memset};

fn sz(len: usize) -> size_t {
  size_t::try_from(len)
    .unwrap_or_else(|_| unreachable!("usize does not fit into size_t on this target"))
}

#[test]
fn memmove_zero_length_keeps_bytes_and_returns_destination() {
  let mut buffer = [10_u8, 20, 30, 40];
  let before = buffer;
  let destination = buffer.as_mut_ptr().wrapping_add(1);
  let source = buffer.as_ptr().wrapping_add(2);
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, before);
}

#[test]
fn memmove_zero_length_allows_null_pointers() {
  // SAFETY: `n == 0`, so no memory access is performed.
  let returned = unsafe { memmove(core::ptr::null_mut(), core::ptr::null(), sz(0)) };

  assert!(returned.is_null());
}

#[test]
fn memmove_zero_length_allows_mixed_null_and_non_null_pointers() {
  let mut buffer = [7_u8, 8, 9];
  let valid_destination = buffer.as_mut_ptr();
  let valid_source = buffer.as_ptr();

  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let returned_with_null_source =
    unsafe { memmove(valid_destination.cast(), core::ptr::null(), sz(0)) }.cast();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let returned_with_null_destination =
    unsafe { memmove(core::ptr::null_mut(), valid_source.cast(), sz(0)) };

  assert_eq!(returned_with_null_source, valid_destination);
  assert!(returned_with_null_destination.is_null());
  assert_eq!(buffer, [7_u8, 8, 9]);
}

#[test]
fn memmove_zero_length_allows_one_past_end_pointers() {
  let mut buffer = [1_u8, 2, 3, 4];
  let one_past_end = buffer.as_mut_ptr().wrapping_add(buffer.len());
  let before = buffer;
  // SAFETY: `n == 0`, so one-past-end pointers are never dereferenced.
  let returned = unsafe { memmove(one_past_end.cast(), one_past_end.cast(), sz(0)) }.cast();

  assert_eq!(returned, one_past_end);
  assert_eq!(buffer, before);
}

#[test]
fn memmove_zero_length_allows_same_one_past_end_pointer_for_non_byte_array() {
  let mut buffer = [271_u32, 272, 273];
  let one_past_end = buffer.as_mut_ptr().wrapping_add(buffer.len()).cast::<u8>();
  let before = buffer;
  // SAFETY: `n == 0`, so one-past-end pointers are never dereferenced.
  let returned =
    unsafe { memmove(one_past_end.cast(), one_past_end.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, one_past_end);
  assert_eq!(buffer, before);
}

#[test]
fn memmove_zero_length_allows_null_destination_with_one_past_end_source() {
  let buffer = [11_u8, 12, 13, 14];
  let one_past_end_source = buffer.as_ptr().wrapping_add(buffer.len());
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(core::ptr::null_mut(), one_past_end_source.cast(), sz(0)) };

  assert!(returned.is_null());
  assert_eq!(buffer, [11_u8, 12, 13, 14]);
}

#[test]
fn memmove_zero_length_allows_null_destination_with_non_byte_one_past_end_source() {
  let source_buffer = [281_u32, 282, 283];
  let one_past_end_source = source_buffer
    .as_ptr()
    .wrapping_add(source_buffer.len())
    .cast::<u8>();
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(core::ptr::null_mut(), one_past_end_source.cast(), sz(0)) };

  assert!(returned.is_null());
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_one_past_end_destination_with_null_source() {
  let mut buffer = [21_u8, 22, 23, 24];
  let one_past_end_destination = buffer.as_mut_ptr().wrapping_add(buffer.len());
  let before = buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned =
    unsafe { memmove(one_past_end_destination.cast(), core::ptr::null(), sz(0)) }.cast();

  assert_eq!(returned, one_past_end_destination);
  assert_eq!(buffer, before);
}

#[test]
fn memmove_zero_length_allows_live_destination_with_one_past_end_source() {
  let mut buffer = [31_u8, 32, 33, 34];
  let destination = buffer.as_mut_ptr().wrapping_add(1);
  let one_past_end_source = buffer.as_ptr().wrapping_add(buffer.len());
  let before = buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), one_past_end_source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, before);
}

#[test]
fn memmove_zero_length_allows_one_past_end_destination_with_live_source() {
  let mut buffer = [41_u8, 42, 43, 44];
  let one_past_end_destination = buffer.as_mut_ptr().wrapping_add(buffer.len());
  let source = buffer.as_ptr().wrapping_add(1);
  let before = buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(one_past_end_destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, one_past_end_destination);
  assert_eq!(buffer, before);
}

#[test]
fn memmove_zero_length_allows_distinct_one_past_end_pointers() {
  let mut destination_buffer = [61_u8, 62, 63, 64];
  let source_buffer = [71_u8, 72, 73, 74];
  let destination = destination_buffer
    .as_mut_ptr()
    .wrapping_add(destination_buffer.len());
  let source = source_buffer.as_ptr().wrapping_add(source_buffer.len());
  let destination_before = destination_buffer;
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so one-past-end pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_null_destination_with_distinct_live_source() {
  let source_buffer = [81_u8, 82, 83, 84];
  let source = source_buffer.as_ptr().wrapping_add(2);
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(core::ptr::null_mut(), source.cast(), sz(0)) };

  assert!(returned.is_null());
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_distinct_live_destination_with_one_past_end_source() {
  let mut destination_buffer = [91_u8, 92, 93, 94];
  let source_buffer = [101_u8, 102, 103, 104];
  let destination = destination_buffer.as_mut_ptr().wrapping_add(2);
  let source = source_buffer.as_ptr().wrapping_add(source_buffer.len());
  let destination_before = destination_buffer;
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_distinct_live_pointers_without_mutation() {
  let mut destination_buffer = [111_u8, 112, 113, 114];
  let source_buffer = [121_u8, 122, 123, 124];
  let destination = destination_buffer.as_mut_ptr().wrapping_add(1);
  let source = source_buffer.as_ptr().wrapping_add(2);
  let destination_before = destination_buffer;
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_distinct_live_destination_with_null_source() {
  let mut destination_buffer = [131_u8, 132, 133, 134];
  let destination = destination_buffer.as_mut_ptr().wrapping_add(3);
  let destination_before = destination_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), core::ptr::null(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
}

#[test]
fn memmove_zero_length_allows_distinct_one_past_end_destination_with_live_source() {
  let mut destination_buffer = [141_u8, 142, 143, 144];
  let source_buffer = [151_u8, 152, 153, 154];
  let destination = destination_buffer
    .as_mut_ptr()
    .wrapping_add(destination_buffer.len());
  let source = source_buffer.as_ptr().wrapping_add(1);
  let destination_before = destination_buffer;
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_null_destination_with_distinct_one_past_end_source() {
  let source_buffer = [161_u8, 162, 163, 164];
  let source = source_buffer.as_ptr().wrapping_add(source_buffer.len());
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(core::ptr::null_mut(), source.cast(), sz(0)) };

  assert!(returned.is_null());
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_live_destination_with_empty_array_source() {
  let mut destination_buffer = [171_u8, 172, 173, 174];
  let source_buffer: [u8; 0] = [];
  let destination = destination_buffer.as_mut_ptr().wrapping_add(2);
  let source = source_buffer.as_ptr();
  let destination_before = destination_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
  assert_eq!(source_buffer, []);
}

#[test]
fn memmove_zero_length_allows_null_destination_with_empty_array_source() {
  let source_buffer: [u8; 0] = [];
  let source = source_buffer.as_ptr();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(core::ptr::null_mut(), source.cast(), sz(0)) };

  assert!(returned.is_null());
  assert_eq!(source_buffer, []);
}

#[test]
fn memmove_zero_length_allows_empty_array_destination_with_live_source() {
  let mut destination_buffer: [u8; 0] = [];
  let source_buffer = [181_u8, 182, 183, 184];
  let destination = destination_buffer.as_mut_ptr();
  let source = source_buffer.as_ptr().wrapping_add(1);
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, []);
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_empty_array_pointers() {
  let mut destination_buffer: [u8; 0] = [];
  let source_buffer: [u8; 0] = [];
  let destination = destination_buffer.as_mut_ptr();
  let source = source_buffer.as_ptr();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, []);
  assert_eq!(source_buffer, []);
}

#[test]
fn memmove_zero_length_allows_empty_array_pointers_with_distinct_alignments() {
  let mut destination_buffer: [u8; 0] = [];
  let source_buffer: [u16; 0] = [];
  let destination = destination_buffer.as_mut_ptr();
  let source = source_buffer.as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, []);
  assert_eq!(source_buffer, []);
}

#[test]
fn memmove_zero_length_allows_same_empty_array_pointer() {
  let mut buffer: [u8; 0] = [];
  let pointer = buffer.as_mut_ptr();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(pointer.cast(), pointer.cast_const().cast(), sz(0)) }.cast();

  assert_eq!(returned, pointer);
  assert_eq!(buffer, []);
}

#[test]
fn memmove_zero_length_allows_empty_array_destination_with_null_source() {
  let mut destination_buffer: [u8; 0] = [];
  let destination = destination_buffer.as_mut_ptr();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), core::ptr::null(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, []);
}

#[test]
fn memmove_zero_length_allows_empty_array_destination_with_one_past_end_source() {
  let mut destination_buffer: [u8; 0] = [];
  let source_buffer = [191_u8, 192, 193, 194];
  let destination = destination_buffer.as_mut_ptr();
  let source = source_buffer.as_ptr().wrapping_add(source_buffer.len());
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, []);
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_one_past_end_destination_with_empty_array_source() {
  let mut destination_buffer = [201_u8, 202, 203, 204];
  let source_buffer: [u8; 0] = [];
  let destination = destination_buffer
    .as_mut_ptr()
    .wrapping_add(destination_buffer.len());
  let source = source_buffer.as_ptr();
  let destination_before = destination_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
  assert_eq!(source_buffer, []);
}

#[test]
fn memmove_zero_length_allows_one_past_end_and_empty_array_with_distinct_alignments() {
  let mut destination_buffer = [261_u16, 262, 263, 264];
  let source_buffer: [u32; 0] = [];
  let destination = destination_buffer
    .as_mut_ptr()
    .wrapping_add(destination_buffer.len())
    .cast::<u8>();
  let source = source_buffer.as_ptr().cast::<u8>();
  let destination_before = destination_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
  assert_eq!(source_buffer, []);
}

#[test]
fn memmove_zero_length_allows_distinct_dangling_pointers() {
  let destination = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let source = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned =
    unsafe { memmove(destination.cast(), source.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination);
}

#[test]
fn memmove_zero_length_allows_same_dangling_pointer() {
  let pointer = core::ptr::NonNull::<u64>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned =
    unsafe { memmove(pointer.cast(), pointer.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, pointer);
}

#[test]
fn memmove_zero_length_allows_mixed_null_and_dangling_pointers() {
  let destination = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let source = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();

  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_null_source =
    unsafe { memmove(destination.cast(), core::ptr::null(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_null_destination =
    unsafe { memmove(core::ptr::null_mut(), source.cast_const().cast(), sz(0)) };

  assert_eq!(returned_with_null_source, destination);
  assert!(returned_with_null_destination.is_null());
}

#[test]
fn memmove_zero_length_allows_one_past_end_destination_with_dangling_source() {
  let mut destination_buffer = [211_u8, 212, 213, 214];
  let destination = destination_buffer
    .as_mut_ptr()
    .wrapping_add(destination_buffer.len());
  let source = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let destination_before = destination_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned =
    unsafe { memmove(destination.cast(), source.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(destination_buffer, destination_before);
}

#[test]
fn memmove_zero_length_allows_dangling_destination_with_one_past_end_source() {
  let source_buffer = [221_u8, 222, 223, 224];
  let destination = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let source = source_buffer.as_ptr().wrapping_add(source_buffer.len());
  let source_before = source_buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_mixed_empty_array_and_dangling_pointers() {
  let mut destination_buffer: [u8; 0] = [];
  let source_buffer: [u8; 0] = [];
  let destination_empty = destination_buffer.as_mut_ptr();
  let source_empty = source_buffer.as_ptr();
  let dangling_source = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let dangling_destination = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();

  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_dangling_source = unsafe {
    memmove(
      destination_empty.cast(),
      dangling_source.cast_const().cast(),
      sz(0),
    )
  }
  .cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_dangling_destination =
    unsafe { memmove(dangling_destination.cast(), source_empty.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_with_dangling_source, destination_empty);
  assert_eq!(returned_with_dangling_destination, dangling_destination);
  assert_eq!(destination_buffer, []);
  assert_eq!(source_buffer, []);
}

#[test]
fn memmove_zero_length_allows_mixed_live_and_dangling_pointers() {
  let mut destination_buffer = [231_u8, 232, 233, 234];
  let source_buffer = [241_u8, 242, 243, 244];
  let live_destination = destination_buffer.as_mut_ptr().wrapping_add(1);
  let live_source = source_buffer.as_ptr().wrapping_add(2);
  let dangling_source = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let dangling_destination = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  let destination_before = destination_buffer;
  let source_before = source_buffer;

  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_dangling_source = unsafe {
    memmove(
      live_destination.cast(),
      dangling_source.cast_const().cast(),
      sz(0),
    )
  }
  .cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_dangling_destination =
    unsafe { memmove(dangling_destination.cast(), live_source.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_with_dangling_source, live_destination);
  assert_eq!(returned_with_dangling_destination, dangling_destination);
  assert_eq!(destination_buffer, destination_before);
  assert_eq!(source_buffer, source_before);
}

#[test]
fn memmove_zero_length_allows_same_live_pointer() {
  let mut buffer = [251_u8, 252, 253, 254];
  let pointer = buffer.as_mut_ptr().wrapping_add(2);
  let before = buffer;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned =
    unsafe { memmove(pointer.cast(), pointer.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, pointer);
  assert_eq!(buffer, before);
}

#[test]
fn memmove_overlap_forward_matches_expected_layout() {
  let mut buffer = [0_u8, 1, 2, 3, 4, 5];
  let source = buffer.as_ptr();
  let destination = buffer.as_mut_ptr().wrapping_add(1);
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(5)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [0_u8, 0, 1, 2, 3, 4]);
}

#[test]
fn memmove_overlap_forward_non_byte_array_matches_expected_layout() {
  let mut buffer = [0_u32, 1, 2, 3];
  let source = buffer.as_ptr().cast::<u8>();
  let destination = buffer.as_mut_ptr().wrapping_add(1).cast::<u8>();
  let copy_len = core::mem::size_of::<u32>() * 3;
  // SAFETY: pointers originate from a live array and length is in-bounds.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [0_u32, 0, 1, 2]);
}

#[test]
fn memmove_overlap_backward_matches_expected_layout() {
  let mut buffer = [0_u8, 1, 2, 3, 4, 5];
  let source = buffer.as_ptr().wrapping_add(1);
  let destination = buffer.as_mut_ptr();
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(5)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [1_u8, 2, 3, 4, 5, 5]);
}

#[test]
fn memmove_overlap_backward_non_byte_array_matches_expected_layout() {
  let mut buffer = [0_u32, 1, 2, 3];
  let source = buffer.as_ptr().wrapping_add(1).cast::<u8>();
  let destination = buffer.as_mut_ptr().cast::<u8>();
  let copy_len = core::mem::size_of::<u32>() * 3;
  // SAFETY: pointers originate from a live array and length is in-bounds.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [1_u32, 2, 3, 3]);
}

#[test]
fn memmove_overlap_with_unaligned_non_byte_offsets_matches_copy_within() {
  let mut actual = [0x1122_3344_u32, 0x5566_7788, 0x99aa_bbcc, 0xddee_ff00];
  let mut expected = actual;
  let source_offset = 1usize;
  let destination_offset = 3usize;
  let copy_len = 9usize;

  // SAFETY: Arrays are live and converted to byte slices of exact in-bounds length.
  unsafe {
    let expected_bytes = core::slice::from_raw_parts_mut(
      expected.as_mut_ptr().cast::<u8>(),
      core::mem::size_of_val(&expected),
    );

    expected_bytes.copy_within(source_offset..source_offset + copy_len, destination_offset);
  }

  let destination = actual
    .as_mut_ptr()
    .cast::<u8>()
    .wrapping_add(destination_offset);
  let source = actual.as_ptr().cast::<u8>().wrapping_add(source_offset);
  // SAFETY: pointers are derived from a live array and the byte range is in-bounds.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(actual, expected);
}

#[test]
fn memmove_matches_copy_within_for_unaligned_non_byte_byte_ranges() {
  let original = [0x1122_3344_u32, 0x5566_7788, 0x99aa_bbcc, 0xddee_ff00];
  let total_bytes = core::mem::size_of_val(&original);

  for source_offset in 0..=total_bytes {
    for destination_offset in 0..=total_bytes {
      for copy_len in 0..=total_bytes {
        if source_offset + copy_len > total_bytes || destination_offset + copy_len > total_bytes {
          continue;
        }

        let mut actual = original;
        let mut expected = original;

        // SAFETY: Arrays are live and converted to byte slices of exact in-bounds length.
        unsafe {
          let expected_bytes = core::slice::from_raw_parts_mut(
            expected.as_mut_ptr().cast::<u8>(),
            core::mem::size_of_val(&expected),
          );

          if copy_len != 0 {
            expected_bytes.copy_within(source_offset..source_offset + copy_len, destination_offset);
          }
        }

        let destination = actual
          .as_mut_ptr()
          .cast::<u8>()
          .wrapping_add(destination_offset);
        let source = actual.as_ptr().cast::<u8>().wrapping_add(source_offset);
        // SAFETY: For `copy_len > 0`, range checks above guarantee in-bounds access.
        // For `copy_len == 0`, pointers are never dereferenced.
        let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(
          actual, expected,
          "unexpected u32 bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
      }
    }
  }
}

#[test]
fn memmove_unaligned_non_byte_word_boundary_lengths_match_copy_within() {
  let original = [
    0x0001_0203_0405_0607_u64,
    0x1011_1213_1415_1617,
    0x2021_2223_2425_2627,
    0x3031_3233_3435_3637,
    0x4041_4243_4445_4647,
    0x5051_5253_5455_5657,
  ];
  let total_bytes = core::mem::size_of_val(&original);
  let cases = [
    (1usize, 9usize, 7usize),
    (1, 9, 8),
    (1, 9, 9),
    (9, 1, 7),
    (9, 1, 8),
    (9, 1, 9),
    (3, 19, 15),
    (3, 19, 16),
    (3, 19, 17),
    (19, 3, 15),
    (19, 3, 16),
    (19, 3, 17),
    (5, 13, 31),
    (13, 5, 31),
  ];

  for (source_offset, destination_offset, copy_len) in cases {
    assert!(source_offset + copy_len <= total_bytes);
    assert!(destination_offset + copy_len <= total_bytes);

    let mut actual = original;
    let mut expected = original;

    // SAFETY: Arrays are live and converted to byte slices of exact in-bounds length.
    unsafe {
      let expected_bytes = core::slice::from_raw_parts_mut(
        expected.as_mut_ptr().cast::<u8>(),
        core::mem::size_of_val(&expected),
      );

      expected_bytes.copy_within(source_offset..source_offset + copy_len, destination_offset);
    }

    let destination = actual
      .as_mut_ptr()
      .cast::<u8>()
      .wrapping_add(destination_offset);
    let source = actual.as_ptr().cast::<u8>().wrapping_add(source_offset);
    // SAFETY: case table and assertions above guarantee in-bounds access.
    let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast();

    assert_eq!(
      returned, destination,
      "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
    );
    assert_eq!(
      actual, expected,
      "unexpected u64 bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
    );
  }
}

#[test]
fn memmove_matches_copy_within_for_u64_all_in_bounds_byte_ranges() {
  let original = [
    0x0001_0203_0405_0607_u64,
    0x1011_1213_1415_1617,
    0x2021_2223_2425_2627,
    0x3031_3233_3435_3637,
    0x4041_4243_4445_4647,
    0x5051_5253_5455_5657,
  ];
  let total_bytes = core::mem::size_of_val(&original);

  for source_offset in 0..=total_bytes {
    for destination_offset in 0..=total_bytes {
      for copy_len in 0..=total_bytes {
        if source_offset + copy_len > total_bytes || destination_offset + copy_len > total_bytes {
          continue;
        }

        let mut actual = original;
        let mut expected = original;

        // SAFETY: Arrays are live and converted to byte slices of exact in-bounds length.
        unsafe {
          let expected_bytes = core::slice::from_raw_parts_mut(
            expected.as_mut_ptr().cast::<u8>(),
            core::mem::size_of_val(&expected),
          );

          if copy_len != 0 {
            expected_bytes.copy_within(source_offset..source_offset + copy_len, destination_offset);
          }
        }

        let destination = actual
          .as_mut_ptr()
          .cast::<u8>()
          .wrapping_add(destination_offset);
        let source = actual.as_ptr().cast::<u8>().wrapping_add(source_offset);
        // SAFETY: For `copy_len > 0`, range checks above guarantee in-bounds access.
        // For `copy_len == 0`, pointers are never dereferenced.
        let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(
          actual, expected,
          "unexpected u64 bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
      }
    }
  }
}

#[test]
fn memmove_matches_copy_within_for_u128_all_in_bounds_byte_ranges() {
  let original = [
    0x0001_0203_0405_0607_0809_0a0b_0c0d_0e0f_u128,
    0x1011_1213_1415_1617_1819_1a1b_1c1d_1e1f,
    0x2021_2223_2425_2627_2829_2a2b_2c2d_2e2f,
  ];
  let total_bytes = core::mem::size_of_val(&original);

  for source_offset in 0..=total_bytes {
    for destination_offset in 0..=total_bytes {
      for copy_len in 0..=total_bytes {
        if source_offset + copy_len > total_bytes || destination_offset + copy_len > total_bytes {
          continue;
        }

        let mut actual = original;
        let mut expected = original;

        // SAFETY: Arrays are live and converted to byte slices of exact in-bounds length.
        unsafe {
          let expected_bytes = core::slice::from_raw_parts_mut(
            expected.as_mut_ptr().cast::<u8>(),
            core::mem::size_of_val(&expected),
          );
          if copy_len != 0 {
            expected_bytes.copy_within(source_offset..source_offset + copy_len, destination_offset);
          }
        }

        let destination = actual
          .as_mut_ptr()
          .cast::<u8>()
          .wrapping_add(destination_offset);
        let source = actual.as_ptr().cast::<u8>().wrapping_add(source_offset);
        // SAFETY: For `copy_len > 0`, range checks above guarantee in-bounds access.
        // For `copy_len == 0`, pointers are never dereferenced.
        let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(
          actual, expected,
          "unexpected u128 bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
      }
    }
  }
}

#[test]
fn memmove_unaligned_u128_word_boundary_lengths_match_copy_within() {
  let original = [
    0x0001_0203_0405_0607_0809_0a0b_0c0d_0e0f_u128,
    0x1011_1213_1415_1617_1819_1a1b_1c1d_1e1f,
    0x2021_2223_2425_2627_2829_2a2b_2c2d_2e2f,
    0x3031_3233_3435_3637_3839_3a3b_3c3d_3e3f,
    0x4041_4243_4445_4647_4849_4a4b_4c4d_4e4f,
    0x5051_5253_5455_5657_5859_5a5b_5c5d_5e5f,
  ];
  let total_bytes = core::mem::size_of_val(&original);
  let cases = [
    (1usize, 17usize, 15usize),
    (1, 17, 16),
    (1, 17, 17),
    (17, 1, 15),
    (17, 1, 16),
    (17, 1, 17),
    (3, 35, 31),
    (3, 35, 32),
    (3, 35, 33),
    (35, 3, 31),
    (35, 3, 32),
    (35, 3, 33),
  ];

  for (source_offset, destination_offset, copy_len) in cases {
    assert!(source_offset + copy_len <= total_bytes);
    assert!(destination_offset + copy_len <= total_bytes);

    let mut actual = original;
    let mut expected = original;

    // SAFETY: Arrays are live and converted to byte slices of exact in-bounds length.
    unsafe {
      let expected_bytes = core::slice::from_raw_parts_mut(
        expected.as_mut_ptr().cast::<u8>(),
        core::mem::size_of_val(&expected),
      );
      expected_bytes.copy_within(source_offset..source_offset + copy_len, destination_offset);
    }

    let destination = actual
      .as_mut_ptr()
      .cast::<u8>()
      .wrapping_add(destination_offset);
    let source = actual.as_ptr().cast::<u8>().wrapping_add(source_offset);
    // SAFETY: case table and assertions above guarantee in-bounds access.
    let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast();

    assert_eq!(
      returned, destination,
      "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
    );
    assert_eq!(
      actual, expected,
      "unexpected u128 bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
    );
  }
}

#[test]
fn memmove_non_overlapping_unaligned_non_byte_ranges_match_copy_from_slice() {
  let cases = [
    (0usize, 0usize, 24usize),
    (1, 2, 17),
    (3, 5, 12),
    (7, 1, 9),
    (5, 8, 8),
  ];

  for (source_offset, destination_offset, copy_len) in cases {
    let source = [
      0x1122_3344_u32,
      0x5566_7788,
      0x99aa_bbcc,
      0xddee_ff00,
      0x1357_9bdf,
      0x2468_ace0,
    ];
    let source_before = source;
    let mut actual_destination = [
      0xa1a2_a3a4_u32,
      0xb1b2_b3b4,
      0xc1c2_c3c4,
      0xd1d2_d3d4,
      0xe1e2_e3e4,
      0xf1f2_f3f4,
    ];
    let mut expected_destination = actual_destination;

    let source_total_bytes = core::mem::size_of_val(&source);
    let destination_total_bytes = core::mem::size_of_val(&actual_destination);
    assert!(source_offset + copy_len <= source_total_bytes);
    assert!(destination_offset + copy_len <= destination_total_bytes);

    // SAFETY: source/destination arrays are live and range checks above ensure in-bounds slices.
    unsafe {
      let source_bytes =
        core::slice::from_raw_parts(source.as_ptr().cast::<u8>().add(source_offset), copy_len);
      let expected_destination_bytes = core::slice::from_raw_parts_mut(
        expected_destination
          .as_mut_ptr()
          .cast::<u8>()
          .add(destination_offset),
        copy_len,
      );
      expected_destination_bytes.copy_from_slice(source_bytes);
    }

    let destination = actual_destination
      .as_mut_ptr()
      .cast::<u8>()
      .wrapping_add(destination_offset);
    let source_ptr = source.as_ptr().cast::<u8>().wrapping_add(source_offset);
    // SAFETY: pointers are derived from live arrays and bounded by the checks above.
    let returned = unsafe { memmove(destination.cast(), source_ptr.cast(), sz(copy_len)) }.cast();

    assert_eq!(
      returned, destination,
      "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
    );
    assert_eq!(
      actual_destination, expected_destination,
      "unexpected destination bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
    );
    assert_eq!(source, source_before, "source buffer mutated unexpectedly");
  }
}

#[test]
fn memmove_non_overlapping_u64_buffers_match_copy_from_slice_for_all_ranges() {
  let source = [
    0x0102_0304_0506_0708_u64,
    0x1112_1314_1516_1718,
    0x2122_2324_2526_2728,
    0x3132_3334_3536_3738,
  ];
  let source_before = source;
  let initial_destination = [
    0xa1a2_a3a4_a5a6_a7a8_u64,
    0xb1b2_b3b4_b5b6_b7b8,
    0xc1c2_c3c4_c5c6_c7c8,
    0xd1d2_d3d4_d5d6_d7d8,
  ];
  let total_bytes = core::mem::size_of_val(&source);

  for source_offset in 0..=total_bytes {
    for destination_offset in 0..=total_bytes {
      for copy_len in 0..=total_bytes {
        if source_offset + copy_len > total_bytes || destination_offset + copy_len > total_bytes {
          continue;
        }

        let mut actual_destination = initial_destination;
        let mut expected_destination = initial_destination;

        // SAFETY: Arrays are live and range checks above ensure in-bounds slices.
        unsafe {
          let source_bytes =
            core::slice::from_raw_parts(source.as_ptr().cast::<u8>().add(source_offset), copy_len);
          let expected_destination_bytes = core::slice::from_raw_parts_mut(
            expected_destination
              .as_mut_ptr()
              .cast::<u8>()
              .add(destination_offset),
            copy_len,
          );
          expected_destination_bytes.copy_from_slice(source_bytes);
        }

        let destination = actual_destination
          .as_mut_ptr()
          .cast::<u8>()
          .wrapping_add(destination_offset);
        let source_ptr = source.as_ptr().cast::<u8>().wrapping_add(source_offset);
        // SAFETY: pointers are derived from live arrays and bounded by checks above.
        let returned =
          unsafe { memmove(destination.cast(), source_ptr.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(
          actual_destination, expected_destination,
          "unexpected destination bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(source, source_before, "source buffer mutated unexpectedly");
      }
    }
  }
}

#[test]
fn memmove_non_overlapping_u128_buffers_match_copy_from_slice_for_all_ranges() {
  let source = [
    0x0001_0203_0405_0607_0809_0a0b_0c0d_0e0f_u128,
    0x1011_1213_1415_1617_1819_1a1b_1c1d_1e1f,
    0x2021_2223_2425_2627_2829_2a2b_2c2d_2e2f,
  ];
  let source_before = source;
  let initial_destination = [
    0xa0a1_a2a3_a4a5_a6a7_a8a9_aaab_acad_aeaf_u128,
    0xb0b1_b2b3_b4b5_b6b7_b8b9_babb_bcbd_bebf,
    0xc0c1_c2c3_c4c5_c6c7_c8c9_cacb_cccd_cecf,
  ];
  let total_bytes = core::mem::size_of_val(&source);

  for source_offset in 0..=total_bytes {
    for destination_offset in 0..=total_bytes {
      for copy_len in 0..=total_bytes {
        if source_offset + copy_len > total_bytes || destination_offset + copy_len > total_bytes {
          continue;
        }

        let mut actual_destination = initial_destination;
        let mut expected_destination = initial_destination;

        // SAFETY: Arrays are live and range checks above ensure in-bounds slices.
        unsafe {
          let source_bytes =
            core::slice::from_raw_parts(source.as_ptr().cast::<u8>().add(source_offset), copy_len);
          let expected_destination_bytes = core::slice::from_raw_parts_mut(
            expected_destination
              .as_mut_ptr()
              .cast::<u8>()
              .add(destination_offset),
            copy_len,
          );
          expected_destination_bytes.copy_from_slice(source_bytes);
        }

        let destination = actual_destination
          .as_mut_ptr()
          .cast::<u8>()
          .wrapping_add(destination_offset);
        let source_ptr = source.as_ptr().cast::<u8>().wrapping_add(source_offset);
        // SAFETY: pointers are derived from live arrays and bounded by checks above.
        let returned =
          unsafe { memmove(destination.cast(), source_ptr.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(
          actual_destination, expected_destination,
          "unexpected destination bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(source, source_before, "source buffer mutated unexpectedly");
      }
    }
  }
}

#[test]
fn memmove_non_overlapping_distinct_size_non_byte_buffers_match_copy_from_slice() {
  let source = [0x1122_3344_u32, 0x5566_7788, 0x99aa_bbcc, 0xddee_ff00];
  let source_before = source;
  let initial_destination = [
    0xa1a2_a3a4_a5a6_a7a8_u64,
    0xb1b2_b3b4_b5b6_b7b8,
    0xc1c2_c3c4_c5c6_c7c8,
    0xd1d2_d3d4_d5d6_d7d8,
    0xe1e2_e3e4_e5e6_e7e8,
  ];
  let source_total_bytes = core::mem::size_of_val(&source);
  let destination_total_bytes = core::mem::size_of_val(&initial_destination);

  for source_offset in 0..=source_total_bytes {
    for destination_offset in 0..=destination_total_bytes {
      let max_copy_len = core::cmp::min(
        source_total_bytes - source_offset,
        destination_total_bytes - destination_offset,
      );

      for copy_len in 0..=max_copy_len {
        let mut actual_destination = initial_destination;
        let mut expected_destination = initial_destination;

        // SAFETY: arrays are live and range bounds above ensure in-bounds slices.
        unsafe {
          let source_bytes =
            core::slice::from_raw_parts(source.as_ptr().cast::<u8>().add(source_offset), copy_len);
          let expected_destination_bytes = core::slice::from_raw_parts_mut(
            expected_destination
              .as_mut_ptr()
              .cast::<u8>()
              .add(destination_offset),
            copy_len,
          );
          expected_destination_bytes.copy_from_slice(source_bytes);
        }

        let destination = actual_destination
          .as_mut_ptr()
          .cast::<u8>()
          .wrapping_add(destination_offset);
        let source_ptr = source.as_ptr().cast::<u8>().wrapping_add(source_offset);
        // SAFETY: pointers are derived from live arrays and bounded by checks above.
        let returned =
          unsafe { memmove(destination.cast(), source_ptr.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(
          actual_destination, expected_destination,
          "unexpected destination bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(source, source_before, "source buffer mutated unexpectedly");
      }
    }
  }
}

#[test]
fn memmove_non_overlapping_u16_to_u128_buffers_match_copy_from_slice_for_all_ranges() {
  let source = [
    0x1020_u16, 0x3040, 0x5060, 0x7080, 0x90a0, 0xb0c0, 0xd0e0, 0xf001,
  ];
  let source_before = source;
  let initial_destination = [
    0x0001_0203_0405_0607_0809_0a0b_0c0d_0e0f_u128,
    0x1011_1213_1415_1617_1819_1a1b_1c1d_1e1f,
  ];
  let source_total_bytes = core::mem::size_of_val(&source);
  let destination_total_bytes = core::mem::size_of_val(&initial_destination);

  for source_offset in 0..=source_total_bytes {
    for destination_offset in 0..=destination_total_bytes {
      let max_copy_len = core::cmp::min(
        source_total_bytes - source_offset,
        destination_total_bytes - destination_offset,
      );

      for copy_len in 0..=max_copy_len {
        let mut actual_destination = initial_destination;
        let mut expected_destination = initial_destination;

        // SAFETY: arrays are live and range checks above ensure in-bounds slices.
        unsafe {
          let source_bytes =
            core::slice::from_raw_parts(source.as_ptr().cast::<u8>().add(source_offset), copy_len);
          let expected_destination_bytes = core::slice::from_raw_parts_mut(
            expected_destination
              .as_mut_ptr()
              .cast::<u8>()
              .add(destination_offset),
            copy_len,
          );
          expected_destination_bytes.copy_from_slice(source_bytes);
        }

        let destination = actual_destination
          .as_mut_ptr()
          .cast::<u8>()
          .wrapping_add(destination_offset);
        let source_ptr = source.as_ptr().cast::<u8>().wrapping_add(source_offset);
        // SAFETY: pointers are derived from live arrays and bounded by checks above.
        let returned =
          unsafe { memmove(destination.cast(), source_ptr.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(
          actual_destination, expected_destination,
          "unexpected destination bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(source, source_before, "source buffer mutated unexpectedly");
      }
    }
  }
}

#[test]
fn memmove_adjacent_forward_boundary_copies_as_non_overlap() {
  let mut buffer = [0_u8, 1, 2, 3, 4, 5];
  let source = buffer.as_ptr();
  let destination = buffer.as_mut_ptr().wrapping_add(3);
  // SAFETY: source/destination are in-bounds, adjacent, and length is valid.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(3)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [0_u8, 1, 2, 0, 1, 2]);
}

#[test]
fn memmove_adjacent_forward_boundary_non_byte_array_copies_as_non_overlap() {
  let mut buffer = [0_u32, 1, 2, 3];
  let source = buffer.as_ptr().cast::<u8>();
  let destination = buffer.as_mut_ptr().wrapping_add(2).cast::<u8>();
  let copy_len = core::mem::size_of::<u32>() * 2;
  // SAFETY: source/destination are in-bounds, adjacent, and length is valid.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [0_u32, 1, 0, 1]);
}

#[test]
fn memmove_adjacent_backward_boundary_copies_as_non_overlap() {
  let mut buffer = [0_u8, 1, 2, 3, 4, 5];
  let source = buffer.as_ptr().wrapping_add(3);
  let destination = buffer.as_mut_ptr();
  // SAFETY: source/destination are in-bounds, adjacent, and length is valid.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(3)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [3_u8, 4, 5, 3, 4, 5]);
}

#[test]
fn memmove_adjacent_backward_boundary_non_byte_array_copies_as_non_overlap() {
  let mut buffer = [0_u32, 1, 2, 3];
  let source = buffer.as_ptr().wrapping_add(2).cast::<u8>();
  let destination = buffer.as_mut_ptr().cast::<u8>();
  let copy_len = core::mem::size_of::<u32>() * 2;
  // SAFETY: source/destination are in-bounds, adjacent, and length is valid.
  let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast::<u8>();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [2_u32, 3, 2, 3]);
}

#[test]
fn memmove_same_source_and_destination_is_noop() {
  let mut buffer = [3_u8, 4, 5, 6];
  let pointer = buffer.as_mut_ptr();
  let before = buffer;
  // SAFETY: source and destination are the same in-bounds pointer.
  let returned = unsafe {
    memmove(
      pointer.cast(),
      pointer.cast_const().cast(),
      sz(buffer.len()),
    )
  }
  .cast();

  assert_eq!(returned, pointer);
  assert_eq!(buffer, before);
}

#[test]
fn memmove_non_overlapping_distinct_buffers_copies_all_bytes() {
  let mut destination = [0_u8; 5];
  let source = [9_u8, 8, 7, 6, 5];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe {
    memmove(
      destination.as_mut_ptr().cast(),
      source.as_ptr().cast(),
      sz(source.len()),
    )
  }
  .cast();

  assert_eq!(returned, destination.as_mut_ptr());
  assert_eq!(destination, source);
}

#[test]
fn memmove_matches_copy_within_for_all_in_bounds_ranges() {
  const BUFFER_LEN: usize = 8;
  let original = [0_u8, 1, 2, 3, 4, 5, 6, 7];

  for source_offset in 0..=BUFFER_LEN {
    for destination_offset in 0..=BUFFER_LEN {
      for copy_len in 0..=BUFFER_LEN {
        if source_offset + copy_len > BUFFER_LEN || destination_offset + copy_len > BUFFER_LEN {
          continue;
        }

        let mut actual = original;
        let mut expected = original;

        if copy_len != 0 {
          expected.copy_within(source_offset..source_offset + copy_len, destination_offset);
        }

        let destination = actual.as_mut_ptr().wrapping_add(destination_offset);
        let source = actual.as_ptr().wrapping_add(source_offset);
        // SAFETY: When `copy_len > 0`, pointers are derived from a live array and in-bounds.
        // For `copy_len == 0`, one-past-the-end pointers are allowed and never dereferenced.
        let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
        assert_eq!(
          actual, expected,
          "unexpected bytes for src={source_offset}, dst={destination_offset}, len={copy_len}",
        );
      }
    }
  }
}

#[test]
fn memmove_matches_copy_within_for_non_byte_ranges() {
  const ELEMENTS: usize = 6;
  const BYTES_PER_ELEMENT: usize = core::mem::size_of::<u32>();
  let original = [10_u32, 20, 30, 40, 50, 60];

  for source_offset in 0..=ELEMENTS {
    for destination_offset in 0..=ELEMENTS {
      for copy_elements in 0..=ELEMENTS {
        if source_offset + copy_elements > ELEMENTS || destination_offset + copy_elements > ELEMENTS
        {
          continue;
        }

        let mut actual = original;
        let mut expected = original;

        if copy_elements != 0 {
          expected.copy_within(
            source_offset..source_offset + copy_elements,
            destination_offset,
          );
        }

        let destination = actual
          .as_mut_ptr()
          .cast::<u8>()
          .wrapping_add(destination_offset * BYTES_PER_ELEMENT);
        let source = actual
          .as_ptr()
          .cast::<u8>()
          .wrapping_add(source_offset * BYTES_PER_ELEMENT);
        let copy_len = copy_elements * BYTES_PER_ELEMENT;
        // SAFETY: pointers are derived from a live array and range checks above
        // ensure in-bounds access for non-zero lengths.
        let returned = unsafe { memmove(destination.cast(), source.cast(), sz(copy_len)) }.cast();

        assert_eq!(
          returned, destination,
          "unexpected return pointer for src={source_offset}, dst={destination_offset}, elems={copy_elements}",
        );
        assert_eq!(
          actual, expected,
          "unexpected u32 elements for src={source_offset}, dst={destination_offset}, elems={copy_elements}",
        );
      }
    }
  }
}

#[test]
fn memcpy_zero_length_keeps_bytes_and_returns_destination() {
  let mut destination = [1_u8, 2, 3, 4];
  let source = [9_u8, 8, 7, 6];
  let before = destination;
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe {
    memcpy(
      destination.as_mut_ptr().cast(),
      source.as_ptr().cast(),
      sz(0),
    )
  }
  .cast();

  assert_eq!(returned, destination.as_mut_ptr());
  assert_eq!(destination, before);
}

#[test]
fn memcpy_full_copy_updates_all_bytes_and_returns_destination() {
  let mut destination = [0_u8; 6];
  let source = [9_u8, 8, 7, 6, 5, 4];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe {
    memcpy(
      destination.as_mut_ptr().cast(),
      source.as_ptr().cast(),
      sz(source.len()),
    )
  }
  .cast();

  assert_eq!(returned, destination.as_mut_ptr());
  assert_eq!(destination, source);
}

#[test]
fn memcpy_partial_copy_only_updates_requested_prefix() {
  let mut destination = [0_u8, 1, 2, 3, 4];
  let source = [9_u8, 8, 7, 6, 5];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe {
    memcpy(
      destination.as_mut_ptr().cast(),
      source.as_ptr().cast(),
      sz(3),
    )
  }
  .cast();

  assert_eq!(returned, destination.as_mut_ptr());
  assert_eq!(destination, [9_u8, 8, 7, 3, 4]);
}

#[test]
fn memcpy_with_offset_pointers_copies_requested_subrange() {
  let mut destination = [0_u8, 0, 0, 0, 0];
  let source = [9_u8, 8, 7, 6, 5];
  let destination_ptr = destination.as_mut_ptr().wrapping_add(1);
  let source_ptr = source.as_ptr().wrapping_add(2);
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe { memcpy(destination_ptr.cast(), source_ptr.cast(), sz(2)) }.cast();

  assert_eq!(returned, destination_ptr);
  assert_eq!(destination, [0_u8, 7, 6, 0, 0]);
}

#[test]
fn memcpy_zero_length_allows_null_pointers() {
  let destination = core::ptr::null_mut::<u8>();
  let source = core::ptr::null::<u8>();
  // SAFETY: C ABI allows null pointers when length is zero.
  let returned = unsafe { memcpy(destination.cast(), source.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination);
}

#[test]
fn memcpy_zero_length_allows_empty_array_pointers() {
  let mut destination = [0_u8; 0];
  let source = [1_u8; 0];
  let destination_ptr = destination.as_mut_ptr();
  let source_ptr = source.as_ptr();
  // SAFETY: `n == 0`, so no memory access is performed.
  let returned = unsafe { memcpy(destination_ptr.cast(), source_ptr.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination_ptr);
  assert_eq!(destination, []);
  assert_eq!(source, []);
}

#[test]
fn memcpy_zero_length_allows_same_empty_array_pointer() {
  let mut buffer = [0_u8; 0];
  let pointer = buffer.as_mut_ptr();
  // SAFETY: `n == 0`, so no memory access is performed.
  let returned = unsafe { memcpy(pointer.cast(), pointer.cast_const().cast(), sz(0)) }.cast();

  assert_eq!(returned, pointer);
  assert_eq!(buffer, []);
}

#[test]
fn memcpy_zero_length_allows_empty_array_and_null_pointers() {
  let mut destination = [0_u8; 0];
  let source = [1_u8; 0];
  let destination_ptr = destination.as_mut_ptr();
  let source_ptr = source.as_ptr();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_empty_destination =
    unsafe { memcpy(destination_ptr.cast(), core::ptr::null(), sz(0)) }.cast();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_null_destination =
    unsafe { memcpy(core::ptr::null_mut(), source_ptr.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_empty_destination, destination_ptr);
  assert_eq!(returned_null_destination, core::ptr::null_mut());
  assert_eq!(destination, []);
  assert_eq!(source, []);
}

#[test]
fn memcpy_zero_length_allows_null_and_empty_array_with_distinct_alignments() {
  let mut destination: [u16; 0] = [];
  let source: [u32; 0] = [];
  let destination_ptr = destination.as_mut_ptr().cast::<u8>();
  let source_ptr = source.as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_empty_destination =
    unsafe { memcpy(destination_ptr.cast(), core::ptr::null(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_null_destination =
    unsafe { memcpy(core::ptr::null_mut(), source_ptr.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_empty_destination, destination_ptr);
  assert_eq!(returned_null_destination, core::ptr::null_mut());
  assert_eq!(destination, []);
  assert_eq!(source, []);
}

#[test]
fn memcpy_zero_length_allows_empty_array_pointers_with_distinct_alignments() {
  let mut destination: [u16; 0] = [];
  let source: [u32; 0] = [];
  let destination_ptr = destination.as_mut_ptr().cast::<u8>();
  let source_ptr = source.as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memcpy(destination_ptr.cast(), source_ptr.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination_ptr);
  assert_eq!(destination, []);
  assert_eq!(source, []);
}

#[test]
fn memcpy_zero_length_allows_empty_array_and_one_past_end_pointers() {
  let mut destination = [0_u8; 0];
  let source = [1_u8, 2, 3];
  let destination_ptr = destination.as_mut_ptr();
  let source_one_past_end = source.as_ptr().wrapping_add(source.len());
  let one_past_end_destination = source.as_ptr().wrapping_add(source.len()).cast_mut();
  let source_ptr = source.as_ptr();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_empty_destination =
    unsafe { memcpy(destination_ptr.cast(), source_one_past_end.cast(), sz(0)) }.cast();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_one_past_end_destination =
    unsafe { memcpy(one_past_end_destination.cast(), source_ptr.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_empty_destination, destination_ptr);
  assert_eq!(returned_one_past_end_destination, one_past_end_destination);
  assert_eq!(destination, []);
  assert_eq!(source, [1_u8, 2, 3]);
}

#[test]
fn memcpy_zero_length_allows_live_destination_with_empty_array_source() {
  let mut destination = [31_u8, 32, 33, 34];
  let source: [u8; 0] = [];
  let destination_ptr = destination.as_mut_ptr().wrapping_add(1);
  let source_ptr = source.as_ptr();
  let before = destination;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memcpy(destination_ptr.cast(), source_ptr.cast(), sz(0)) }.cast();

  assert_eq!(returned, destination_ptr);
  assert_eq!(destination, before);
  assert_eq!(source, []);
}

#[test]
fn memcpy_zero_length_allows_empty_array_destination_with_live_source() {
  let mut destination: [u8; 0] = [];
  let source = [31_u8, 32, 33, 34];
  let destination_ptr = destination.as_mut_ptr();
  let source_ptr = source.as_ptr().wrapping_add(1);
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memcpy(destination_ptr.cast(), source_ptr.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination_ptr);
  assert_eq!(destination, []);
  assert_eq!(source, [31_u8, 32, 33, 34]);
}

#[test]
fn memcpy_zero_length_allows_mixed_live_and_empty_array_with_distinct_alignments() {
  let mut live_destination_words = [31_u16, 32, 33, 34];
  let empty_source_words: [u32; 0] = [];
  let live_destination = live_destination_words
    .as_mut_ptr()
    .wrapping_add(1)
    .cast::<u8>();
  let empty_source = empty_source_words.as_ptr().cast::<u8>();
  let live_destination_before = live_destination_words;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_live_destination =
    unsafe { memcpy(live_destination.cast(), empty_source.cast(), sz(0)) }.cast::<u8>();

  let mut empty_destination_words: [u16; 0] = [];
  let live_source_words = [41_u32, 42, 43, 44];
  let empty_destination = empty_destination_words.as_mut_ptr().cast::<u8>();
  let live_source = live_source_words.as_ptr().wrapping_add(1).cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_empty_destination =
    unsafe { memcpy(empty_destination.cast(), live_source.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_live_destination, live_destination);
  assert_eq!(returned_empty_destination, empty_destination);
  assert_eq!(live_destination_words, live_destination_before);
  assert_eq!(empty_destination_words, []);
  assert_eq!(empty_source_words, []);
  assert_eq!(live_source_words, [41_u32, 42, 43, 44]);
}

#[test]
fn memcpy_zero_length_allows_distinct_live_pointers_without_mutation() {
  let mut destination_words = [71_u16, 72, 73, 74];
  let source_words = [81_u32, 82, 83, 84];
  let destination_ptr = destination_words.as_mut_ptr().wrapping_add(1).cast::<u8>();
  let source_ptr = source_words.as_ptr().wrapping_add(1).cast::<u8>();
  let destination_before = destination_words;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned = unsafe { memcpy(destination_ptr.cast(), source_ptr.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination_ptr);
  assert_eq!(destination_words, destination_before);
  assert_eq!(source_words, [81_u32, 82, 83, 84]);
}

#[test]
fn memcpy_zero_length_allows_same_live_pointer_with_distinct_alignment_origin() {
  let mut buffer_words = [91_u32, 92, 93];
  let pointer = buffer_words.as_mut_ptr().wrapping_add(1).cast::<u8>();
  let before = buffer_words;
  // SAFETY: `n == 0`, so the pointer is never dereferenced.
  let returned = unsafe { memcpy(pointer.cast(), pointer.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, pointer);
  assert_eq!(buffer_words, before);
}

#[test]
fn memcpy_zero_length_allows_mixed_null_and_non_null_pointers() {
  let mut destination = [7_u8];
  let destination_ptr = destination.as_mut_ptr();
  let source_ptr = destination.as_ptr();
  // SAFETY: `n == 0`, so source is never dereferenced.
  let returned_non_null_destination =
    unsafe { memcpy(destination_ptr.cast(), core::ptr::null(), sz(0)) }.cast();
  // SAFETY: `n == 0`, so destination is never dereferenced.
  let returned_null_destination =
    unsafe { memcpy(core::ptr::null_mut(), source_ptr.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_non_null_destination, destination_ptr);
  assert_eq!(returned_null_destination, core::ptr::null_mut());
  assert_eq!(destination, [7_u8]);
}

#[test]
fn memcpy_zero_length_allows_mixed_null_and_live_with_distinct_alignments() {
  let mut destination_words = [7_u16, 8, 9];
  let source_words = [1_u32, 2, 3];
  let destination_ptr = destination_words.as_mut_ptr().wrapping_add(1).cast::<u8>();
  let source_ptr = source_words.as_ptr().wrapping_add(1).cast::<u8>();
  let destination_before = destination_words;
  // SAFETY: `n == 0`, so source is never dereferenced.
  let returned_non_null_destination =
    unsafe { memcpy(destination_ptr.cast(), core::ptr::null(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so destination is never dereferenced.
  let returned_null_destination =
    unsafe { memcpy(core::ptr::null_mut(), source_ptr.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_non_null_destination, destination_ptr);
  assert_eq!(returned_null_destination, core::ptr::null_mut());
  assert_eq!(destination_words, destination_before);
  assert_eq!(source_words, [1_u32, 2, 3]);
}

#[test]
fn memcpy_zero_length_allows_one_past_end_pointers() {
  let mut destination = [7_u8, 8, 9];
  let source = [1_u8, 2, 3];
  let destination_one_past_end = destination.as_mut_ptr().wrapping_add(destination.len());
  let source_one_past_end = source.as_ptr().wrapping_add(source.len());
  let before = destination;
  // SAFETY: `n == 0`, so one-past-end pointers are never dereferenced.
  let returned = unsafe {
    memcpy(
      destination_one_past_end.cast(),
      source_one_past_end.cast(),
      sz(0),
    )
  }
  .cast();

  assert_eq!(returned, destination_one_past_end);
  assert_eq!(destination, before);
}

#[test]
fn memcpy_zero_length_allows_same_one_past_end_pointer() {
  let mut buffer = [7_u8, 8, 9];
  let one_past_end = buffer.as_mut_ptr().wrapping_add(buffer.len());
  let before = buffer;
  // SAFETY: `n == 0`, so one-past-end pointers are never dereferenced.
  let returned =
    unsafe { memcpy(one_past_end.cast(), one_past_end.cast_const().cast(), sz(0)) }.cast();

  assert_eq!(returned, one_past_end);
  assert_eq!(buffer, before);
}

#[test]
fn memcpy_zero_length_allows_same_one_past_end_pointer_with_distinct_alignment_origin() {
  let mut buffer_words = [17_u16, 18, 19];
  let one_past_end = buffer_words
    .as_mut_ptr()
    .wrapping_add(buffer_words.len())
    .cast::<u8>();
  let before = buffer_words;
  // SAFETY: `n == 0`, so one-past-end pointers are never dereferenced.
  let returned =
    unsafe { memcpy(one_past_end.cast(), one_past_end.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, one_past_end);
  assert_eq!(buffer_words, before);
}

#[test]
fn memcpy_zero_length_allows_same_dangling_pointer() {
  let dangling = core::ptr::NonNull::<u8>::dangling();
  // SAFETY: `n == 0`, so dangling pointers are never dereferenced.
  let returned = unsafe {
    memcpy(
      dangling.as_ptr().cast(),
      dangling.as_ptr().cast_const().cast(),
      sz(0),
    )
  }
  .cast::<u8>();

  assert_eq!(returned, dangling.as_ptr());
}

#[test]
fn memcpy_zero_length_allows_same_dangling_pointer_with_distinct_alignment_origin() {
  let dangling = core::ptr::NonNull::<u16>::dangling();
  // SAFETY: `n == 0`, so dangling pointers are never dereferenced.
  let returned = unsafe {
    memcpy(
      dangling.as_ptr().cast::<u8>().cast(),
      dangling.as_ptr().cast::<u8>().cast_const().cast(),
      sz(0),
    )
  }
  .cast::<u8>();

  assert_eq!(returned, dangling.as_ptr().cast::<u8>());
}

#[test]
fn memcpy_zero_length_allows_distinct_dangling_pointers() {
  let destination = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let source = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so dangling pointers are never dereferenced.
  let returned =
    unsafe { memcpy(destination.cast(), source.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned, destination);
}

#[test]
fn memcpy_zero_length_allows_mixed_null_and_dangling_pointers() {
  let destination = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let source = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_null_source =
    unsafe { memcpy(destination.cast(), core::ptr::null(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_null_destination =
    unsafe { memcpy(core::ptr::null_mut(), source.cast_const().cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_with_null_source, destination);
  assert_eq!(returned_with_null_destination, core::ptr::null_mut());
}

#[test]
fn memcpy_zero_length_allows_mixed_empty_array_and_dangling_pointers() {
  let empty: [u8; 0] = [];
  let empty_pointer = empty.as_ptr();
  let empty_mut_pointer = empty_pointer.cast_mut();
  let dangling = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_dangling_destination =
    unsafe { memcpy(dangling.cast(), empty_pointer.cast(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_empty_destination = unsafe {
    memcpy(
      empty_mut_pointer.cast(),
      dangling.cast_const().cast(),
      sz(0),
    )
  }
  .cast::<u8>();

  assert_eq!(returned_with_dangling_destination, dangling);
  assert_eq!(returned_with_empty_destination, empty_mut_pointer);
}

#[test]
fn memcpy_zero_length_allows_null_and_one_past_end_pointers() {
  let mut destination = [7_u8, 8, 9];
  let source = [1_u8, 2, 3];
  let destination_one_past_end = destination.as_mut_ptr().wrapping_add(destination.len());
  let source_one_past_end = source.as_ptr().wrapping_add(source.len());
  let before = destination;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_null_source =
    unsafe { memcpy(destination_one_past_end.cast(), core::ptr::null(), sz(0)) }.cast();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_null_destination =
    unsafe { memcpy(core::ptr::null_mut(), source_one_past_end.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_with_null_source, destination_one_past_end);
  assert_eq!(returned_with_null_destination, core::ptr::null_mut());
  assert_eq!(destination, before);
}

#[test]
fn memcpy_zero_length_allows_null_and_one_past_end_with_distinct_alignments() {
  let mut destination_words = [7_u16, 8, 9];
  let source_words = [1_u32, 2, 3];
  let destination_one_past_end = destination_words
    .as_mut_ptr()
    .wrapping_add(destination_words.len())
    .cast::<u8>();
  let source_one_past_end = source_words
    .as_ptr()
    .wrapping_add(source_words.len())
    .cast::<u8>();
  let destination_before = destination_words;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_null_source =
    unsafe { memcpy(destination_one_past_end.cast(), core::ptr::null(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_null_destination =
    unsafe { memcpy(core::ptr::null_mut(), source_one_past_end.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_with_null_source, destination_one_past_end);
  assert_eq!(returned_with_null_destination, core::ptr::null_mut());
  assert_eq!(destination_words, destination_before);
  assert_eq!(source_words, [1_u32, 2, 3]);
}

#[test]
fn memcpy_zero_length_allows_one_past_end_and_empty_array_with_distinct_alignments() {
  let mut destination_words = [31_u16, 32, 33];
  let source_words = [41_u32, 42, 43];
  let empty_destination_words: [u32; 0] = [];
  let empty_source_words: [u16; 0] = [];
  let destination_one_past_end = destination_words
    .as_mut_ptr()
    .wrapping_add(destination_words.len())
    .cast::<u8>();
  let source_one_past_end = source_words
    .as_ptr()
    .wrapping_add(source_words.len())
    .cast::<u8>();
  let empty_destination = empty_destination_words.as_ptr().cast_mut().cast::<u8>();
  let empty_source = empty_source_words.as_ptr().cast::<u8>();
  let destination_before = destination_words;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_one_past_end_destination =
    unsafe { memcpy(destination_one_past_end.cast(), empty_source.cast(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_empty_destination =
    unsafe { memcpy(empty_destination.cast(), source_one_past_end.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_one_past_end_destination, destination_one_past_end);
  assert_eq!(returned_empty_destination, empty_destination);
  assert_eq!(destination_words, destination_before);
  assert_eq!(source_words, [41_u32, 42, 43]);
}

#[test]
fn memcpy_zero_length_allows_mixed_one_past_end_and_dangling_pointers() {
  let mut destination_words = [51_u16, 52, 53];
  let source_words = [61_u32, 62, 63];
  let destination_one_past_end = destination_words
    .as_mut_ptr()
    .wrapping_add(destination_words.len())
    .cast::<u8>();
  let source_one_past_end = source_words
    .as_ptr()
    .wrapping_add(source_words.len())
    .cast::<u8>();
  let dangling_destination = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  let dangling_source = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let destination_before = destination_words;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_one_past_end_destination = unsafe {
    memcpy(
      destination_one_past_end.cast(),
      dangling_source.cast_const().cast(),
      sz(0),
    )
  }
  .cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_dangling_destination = unsafe {
    memcpy(
      dangling_destination.cast(),
      source_one_past_end.cast(),
      sz(0),
    )
  }
  .cast::<u8>();

  assert_eq!(returned_one_past_end_destination, destination_one_past_end);
  assert_eq!(returned_dangling_destination, dangling_destination);
  assert_eq!(destination_words, destination_before);
  assert_eq!(source_words, [61_u32, 62, 63]);
}

#[test]
fn memcpy_zero_length_allows_live_and_one_past_end_pointers() {
  let mut destination = [17_u8, 18, 19, 20];
  let source = [21_u8, 22, 23, 24];
  let live_destination = destination.as_mut_ptr().wrapping_add(1);
  let one_past_end_destination = destination.as_mut_ptr().wrapping_add(destination.len());
  let live_source = source.as_ptr().wrapping_add(1);
  let one_past_end_source = source.as_ptr().wrapping_add(source.len());
  let before = destination;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_live_destination =
    unsafe { memcpy(live_destination.cast(), one_past_end_source.cast(), sz(0)) }.cast();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_one_past_end_destination =
    unsafe { memcpy(one_past_end_destination.cast(), live_source.cast(), sz(0)) }.cast();

  assert_eq!(returned_live_destination, live_destination);
  assert_eq!(returned_one_past_end_destination, one_past_end_destination);
  assert_eq!(destination, before);
}

#[test]
fn memcpy_zero_length_allows_live_and_one_past_end_with_distinct_alignments() {
  let mut destination_words = [17_u16, 18, 19, 20];
  let source_words = [21_u32, 22, 23, 24];
  let live_destination = destination_words.as_mut_ptr().wrapping_add(1).cast::<u8>();
  let one_past_end_destination = destination_words
    .as_mut_ptr()
    .wrapping_add(destination_words.len())
    .cast::<u8>();
  let live_source = source_words.as_ptr().wrapping_add(1).cast::<u8>();
  let one_past_end_source = source_words
    .as_ptr()
    .wrapping_add(source_words.len())
    .cast::<u8>();
  let destination_before = destination_words;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_live_destination =
    unsafe { memcpy(live_destination.cast(), one_past_end_source.cast(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_one_past_end_destination =
    unsafe { memcpy(one_past_end_destination.cast(), live_source.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_live_destination, live_destination);
  assert_eq!(returned_one_past_end_destination, one_past_end_destination);
  assert_eq!(destination_words, destination_before);
  assert_eq!(source_words, [21_u32, 22, 23, 24]);
}

#[test]
fn memcpy_zero_length_allows_mixed_live_and_dangling_pointers() {
  let mut destination = [31_u8, 32, 33];
  let live_destination = destination.as_mut_ptr().wrapping_add(1);
  let live_source = destination.as_ptr().wrapping_add(1);
  let dangling = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let before = destination;
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_dangling_source =
    unsafe { memcpy(live_destination.cast(), dangling.cast_const().cast(), sz(0)) }.cast::<u8>();
  // SAFETY: `n == 0`, so pointers are never dereferenced.
  let returned_with_dangling_destination =
    unsafe { memcpy(dangling.cast(), live_source.cast(), sz(0)) }.cast::<u8>();

  assert_eq!(returned_with_dangling_source, live_destination);
  assert_eq!(returned_with_dangling_destination, dangling);
  assert_eq!(destination, before);
}

#[test]
fn memcpy_same_source_and_destination_is_noop() {
  let mut buffer = [3_u8, 4, 5, 6];
  let pointer = buffer.as_mut_ptr();
  let before = buffer;
  // SAFETY: source and destination are the same in-bounds pointer.
  let returned = unsafe {
    memcpy(
      pointer.cast(),
      pointer.cast_const().cast(),
      sz(buffer.len()),
    )
  }
  .cast();

  assert_eq!(returned, pointer);
  assert_eq!(buffer, before);
}

#[test]
fn memset_zero_length_keeps_bytes_and_returns_destination() {
  let mut buffer = [1_u8, 2, 3, 4];
  let before = buffer;
  let destination = buffer.as_mut_ptr().wrapping_add(2);
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe { memset(destination.cast(), 0xAA, sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, before);
}

#[test]
fn memset_zero_length_allows_null_pointers() {
  let destination = core::ptr::null_mut::<u8>();
  // SAFETY: `n == 0`, so no memory access is performed and null is allowed.
  let returned = unsafe { memset(destination.cast(), 0xAB, sz(0)) }.cast();

  assert_eq!(returned, destination);
}

#[test]
fn memset_zero_length_allows_empty_array_pointer() {
  let mut buffer = [0_u8; 0];
  let destination = buffer.as_mut_ptr();
  // SAFETY: `n == 0`, so no memory access is performed.
  let returned = unsafe { memset(destination.cast(), 0xCD, sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, []);
}

#[test]
fn memset_zero_length_allows_one_past_end_pointer() {
  let mut buffer = [1_u8, 2, 3];
  let destination = buffer.as_mut_ptr().wrapping_add(buffer.len());
  let before = buffer;
  // SAFETY: `n == 0`, so one-past-end pointer is never dereferenced.
  let returned = unsafe { memset(destination.cast(), 0xCD, sz(0)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, before);
}

#[test]
fn memset_truncates_value_to_unsigned_byte_and_returns_destination() {
  let mut buffer = [0_u8, 0, 0, 0];
  let destination = buffer.as_mut_ptr().wrapping_add(1);
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let returned = unsafe { memset(destination.cast(), 0x01FF, sz(2)) }.cast();

  assert_eq!(returned, destination);
  assert_eq!(buffer, [0_u8, 0xFF, 0xFF, 0]);
}

#[test]
fn memset_negative_value_uses_low_unsigned_byte() {
  let mut buffer = [0_u8; 3];
  // SAFETY: pointer originates from a live array and length is in-bounds.
  let returned = unsafe { memset(buffer.as_mut_ptr().cast(), -1, sz(buffer.len())) }.cast();

  assert_eq!(returned, buffer.as_mut_ptr());
  assert_eq!(buffer, [0xFF_u8, 0xFF, 0xFF]);
}

#[test]
fn memcmp_zero_length_returns_zero_for_different_buffers() {
  let left = [1_u8, 2, 3];
  let right = [9_u8, 8, 7];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let result = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_null_pointers() {
  // SAFETY: `n == 0`, so no memory is accessed and null pointers are valid.
  let result = unsafe { memcmp(core::ptr::null(), core::ptr::null(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_same_live_pointer() {
  let buffer = [1_u8, 2, 3];
  let pointer = buffer.as_ptr();
  // SAFETY: `n == 0`, so no memory is accessed.
  let result = unsafe { memcmp(pointer.cast(), pointer.cast(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_same_dangling_pointer() {
  let pointer = core::ptr::NonNull::<u8>::dangling().as_ptr();
  // SAFETY: `n == 0`, so the pointer is never dereferenced.
  let result = unsafe { memcmp(pointer.cast(), pointer.cast(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_distinct_dangling_pointers() {
  let left = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let right = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let left_vs_right = unsafe { memcmp(left.cast(), right.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let right_vs_left = unsafe { memcmp(right.cast(), left.cast(), sz(0)) };

  assert_eq!(left_vs_right, 0);
  assert_eq!(right_vs_left, 0);
}

#[test]
fn memcmp_zero_length_allows_empty_array_pointers() {
  let left = [0_u8; 0];
  let right = [1_u8; 0];
  let left_ptr = left.as_ptr();
  let right_ptr = right.as_ptr();
  // SAFETY: `n == 0`, so no memory is accessed.
  let result = unsafe { memcmp(left_ptr.cast(), right_ptr.cast(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_same_empty_array_pointer() {
  let buffer = [0_u8; 0];
  let pointer = buffer.as_ptr();
  // SAFETY: `n == 0`, so no memory is accessed.
  let result = unsafe { memcmp(pointer.cast(), pointer.cast(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_empty_array_and_null_pointers() {
  let buffer = [0_u8; 0];
  let empty_pointer = buffer.as_ptr();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let left_empty = unsafe { memcmp(empty_pointer.cast(), core::ptr::null(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let right_empty = unsafe { memcmp(core::ptr::null(), empty_pointer.cast(), sz(0)) };

  assert_eq!(left_empty, 0);
  assert_eq!(right_empty, 0);
}

#[test]
fn memcmp_zero_length_allows_null_and_empty_array_with_distinct_alignments() {
  let left_empty: [u16; 0] = [];
  let right_empty: [u32; 0] = [];
  let left_empty_pointer = left_empty.as_ptr().cast::<u8>();
  let right_empty_pointer = right_empty.as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let null_vs_left_empty = unsafe { memcmp(core::ptr::null(), left_empty_pointer.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let right_empty_vs_null = unsafe { memcmp(right_empty_pointer.cast(), core::ptr::null(), sz(0)) };

  assert_eq!(null_vs_left_empty, 0);
  assert_eq!(right_empty_vs_null, 0);
  assert_eq!(left_empty, []);
  assert_eq!(right_empty, []);
}

#[test]
fn memcmp_zero_length_allows_empty_array_and_one_past_end_pointers() {
  let empty_buffer: [u8; 0] = [];
  let end_buffer = [1_u8, 2, 3];
  let empty_pointer = empty_buffer.as_ptr();
  let one_past_end_pointer = end_buffer.as_ptr().wrapping_add(end_buffer.len());
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let empty_vs_end = unsafe { memcmp(empty_pointer.cast(), one_past_end_pointer.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let end_vs_empty = unsafe { memcmp(one_past_end_pointer.cast(), empty_pointer.cast(), sz(0)) };

  assert_eq!(empty_vs_end, 0);
  assert_eq!(end_vs_empty, 0);
}

#[test]
fn memcmp_zero_length_allows_one_past_end_and_empty_array_with_distinct_alignments() {
  let left_words = [1_u16, 2, 3];
  let right_words = [4_u32, 5, 6];
  let left_one_past_end = left_words
    .as_ptr()
    .wrapping_add(left_words.len())
    .cast::<u8>();
  let right_one_past_end = right_words
    .as_ptr()
    .wrapping_add(right_words.len())
    .cast::<u8>();
  let left_empty: [u16; 0] = [];
  let right_empty: [u32; 0] = [];
  let left_empty_pointer = left_empty.as_ptr().cast::<u8>();
  let right_empty_pointer = right_empty.as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let left_end_vs_empty =
    unsafe { memcmp(left_one_past_end.cast(), right_empty_pointer.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let left_empty_vs_end =
    unsafe { memcmp(left_empty_pointer.cast(), right_one_past_end.cast(), sz(0)) };

  assert_eq!(left_end_vs_empty, 0);
  assert_eq!(left_empty_vs_end, 0);
  assert_eq!(left_words, [1_u16, 2, 3]);
  assert_eq!(right_words, [4_u32, 5, 6]);
  assert_eq!(left_empty, []);
  assert_eq!(right_empty, []);
}

#[test]
fn memcmp_zero_length_allows_mixed_empty_array_and_dangling_pointers() {
  let left_empty: [u8; 0] = [];
  let right_empty: [u8; 0] = [];
  let left_empty_pointer = left_empty.as_ptr();
  let right_empty_pointer = right_empty.as_ptr();
  let left_dangling = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let right_dangling = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let empty_vs_dangling =
    unsafe { memcmp(left_empty_pointer.cast(), right_dangling.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let dangling_vs_empty =
    unsafe { memcmp(left_dangling.cast(), right_empty_pointer.cast(), sz(0)) };

  assert_eq!(empty_vs_dangling, 0);
  assert_eq!(dangling_vs_empty, 0);
  assert_eq!(left_empty, []);
  assert_eq!(right_empty, []);
}

#[test]
fn memcmp_zero_length_allows_mixed_live_and_empty_array_pointers() {
  let left_live = [11_u8, 12, 13];
  let right_live = [21_u8, 22, 23];
  let left_empty: [u8; 0] = [];
  let right_empty: [u8; 0] = [];
  let left_live_pointer = left_live.as_ptr().wrapping_add(1);
  let right_live_pointer = right_live.as_ptr();
  let left_empty_pointer = left_empty.as_ptr();
  let right_empty_pointer = right_empty.as_ptr();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let live_vs_empty =
    unsafe { memcmp(left_live_pointer.cast(), right_empty_pointer.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let empty_vs_live =
    unsafe { memcmp(left_empty_pointer.cast(), right_live_pointer.cast(), sz(0)) };

  assert_eq!(live_vs_empty, 0);
  assert_eq!(empty_vs_live, 0);
  assert_eq!(left_live, [11_u8, 12, 13]);
  assert_eq!(right_live, [21_u8, 22, 23]);
  assert_eq!(left_empty, []);
  assert_eq!(right_empty, []);
}

#[test]
fn memcmp_zero_length_allows_mixed_null_and_non_null_pointers() {
  let right = [1_u8, 2, 3];
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let result = unsafe { memcmp(core::ptr::null(), right.as_ptr().cast(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_mixed_null_and_dangling_pointers() {
  let dangling = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let null_vs_dangling = unsafe { memcmp(core::ptr::null(), dangling.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let dangling_vs_null = unsafe { memcmp(dangling.cast(), core::ptr::null(), sz(0)) };

  assert_eq!(null_vs_dangling, 0);
  assert_eq!(dangling_vs_null, 0);
}

#[test]
fn memcmp_zero_length_allows_mixed_live_and_dangling_pointers() {
  let left_live = [1_u8, 2, 3];
  let right_live = [4_u8, 5, 6];
  let left_live_pointer = left_live.as_ptr().wrapping_add(1);
  let right_live_pointer = right_live.as_ptr();
  let left_dangling = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let right_dangling = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let live_vs_dangling = unsafe { memcmp(left_live_pointer.cast(), right_dangling.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let dangling_vs_live = unsafe { memcmp(left_dangling.cast(), right_live_pointer.cast(), sz(0)) };

  assert_eq!(live_vs_dangling, 0);
  assert_eq!(dangling_vs_live, 0);
  assert_eq!(left_live, [1_u8, 2, 3]);
  assert_eq!(right_live, [4_u8, 5, 6]);
}

#[test]
fn memcmp_zero_length_allows_mixed_one_past_end_and_dangling_pointers() {
  let left_live = [31_u8, 32, 33];
  let right_live = [41_u8, 42, 43];
  let left_one_past_end = left_live.as_ptr().wrapping_add(left_live.len());
  let right_one_past_end = right_live.as_ptr().wrapping_add(right_live.len());
  let left_dangling = core::ptr::NonNull::<u16>::dangling().as_ptr().cast::<u8>();
  let right_dangling = core::ptr::NonNull::<u32>::dangling().as_ptr().cast::<u8>();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let one_past_end_vs_dangling =
    unsafe { memcmp(left_one_past_end.cast(), right_dangling.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let dangling_vs_one_past_end =
    unsafe { memcmp(left_dangling.cast(), right_one_past_end.cast(), sz(0)) };

  assert_eq!(one_past_end_vs_dangling, 0);
  assert_eq!(dangling_vs_one_past_end, 0);
  assert_eq!(left_live, [31_u8, 32, 33]);
  assert_eq!(right_live, [41_u8, 42, 43]);
}

#[test]
fn memcmp_zero_length_allows_non_null_and_null_pointers() {
  let left = [1_u8, 2, 3];
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let result = unsafe { memcmp(left.as_ptr().cast(), core::ptr::null(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_one_past_end_pointers() {
  let left = [1_u8, 2, 3];
  let right = [4_u8, 5, 6];
  let left_one_past_end = left.as_ptr().wrapping_add(left.len());
  let right_one_past_end = right.as_ptr().wrapping_add(right.len());
  // SAFETY: `n == 0`, so the pointers are not dereferenced.
  let result = unsafe { memcmp(left_one_past_end.cast(), right_one_past_end.cast(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_same_one_past_end_pointer() {
  let buffer = [1_u8, 2, 3];
  let one_past_end = buffer.as_ptr().wrapping_add(buffer.len());
  // SAFETY: `n == 0`, so the pointer is not dereferenced.
  let result = unsafe { memcmp(one_past_end.cast(), one_past_end.cast(), sz(0)) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_zero_length_allows_null_and_one_past_end_pointers() {
  let buffer = [1_u8, 2, 3];
  let one_past_end = buffer.as_ptr().wrapping_add(buffer.len());
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let left_null = unsafe { memcmp(core::ptr::null(), one_past_end.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let right_null = unsafe { memcmp(one_past_end.cast(), core::ptr::null(), sz(0)) };

  assert_eq!(left_null, 0);
  assert_eq!(right_null, 0);
}

#[test]
fn memcmp_zero_length_allows_null_and_one_past_end_with_distinct_alignments() {
  let left_words = [11_u16, 12, 13];
  let right_words = [21_u32, 22, 23];
  let left_one_past_end = left_words
    .as_ptr()
    .wrapping_add(left_words.len())
    .cast::<u8>();
  let right_one_past_end = right_words
    .as_ptr()
    .wrapping_add(right_words.len())
    .cast::<u8>();
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let null_vs_left_end = unsafe { memcmp(core::ptr::null(), left_one_past_end.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let right_end_vs_null = unsafe { memcmp(right_one_past_end.cast(), core::ptr::null(), sz(0)) };

  assert_eq!(null_vs_left_end, 0);
  assert_eq!(right_end_vs_null, 0);
  assert_eq!(left_words, [11_u16, 12, 13]);
  assert_eq!(right_words, [21_u32, 22, 23]);
}

#[test]
fn memcmp_zero_length_allows_live_and_one_past_end_pointers() {
  let live_buffer = [1_u8, 2, 3];
  let end_buffer = [4_u8, 5, 6];
  let live_pointer = live_buffer.as_ptr();
  let one_past_end_pointer = end_buffer.as_ptr().wrapping_add(end_buffer.len());
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let live_vs_end = unsafe { memcmp(live_pointer.cast(), one_past_end_pointer.cast(), sz(0)) };
  // SAFETY: `n == 0`, so neither pointer is dereferenced.
  let end_vs_live = unsafe { memcmp(one_past_end_pointer.cast(), live_pointer.cast(), sz(0)) };

  assert_eq!(live_vs_end, 0);
  assert_eq!(end_vs_live, 0);
}

#[test]
fn memcmp_returns_exact_positive_byte_delta_for_first_difference() {
  let left = [0x31_u8];
  let right = [0x10_u8];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let result = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(1)) };

  assert_eq!(result, 0x21);
}

#[test]
fn memcmp_returns_exact_negative_byte_delta_for_first_difference() {
  let left = [0x10_u8];
  let right = [0x31_u8];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let result = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(1)) };

  assert_eq!(result, -0x21);
}

#[test]
fn memcmp_returns_full_unsigned_byte_delta_for_extreme_difference() {
  let left = [0x00_u8];
  let right = [0xFF_u8];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let negative = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(1)) };
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let positive = unsafe { memcmp(right.as_ptr().cast(), left.as_ptr().cast(), sz(1)) };

  assert_eq!(negative, -255);
  assert_eq!(positive, 255);
}

#[test]
fn memcmp_equal_ranges_returns_zero() {
  let left = [4_u8, 5, 6, 7];
  let right = [4_u8, 5, 6, 7];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let result = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(left.len())) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_same_pointer_returns_zero_for_nonzero_length() {
  let buffer = [4_u8, 5, 6, 7];
  let pointer = buffer.as_ptr();
  // SAFETY: pointer originates from a live array and length is in-bounds.
  let result = unsafe { memcmp(pointer.cast(), pointer.cast(), sz(buffer.len())) };

  assert_eq!(result, 0);
}

#[test]
fn memcmp_returns_negative_when_first_difference_is_smaller() {
  let left = [0_u8];
  let right = [0xFF_u8];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let result = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(1)) };

  assert!(result.is_negative());
}

#[test]
fn memcmp_returns_positive_when_first_difference_is_greater_unsigned() {
  let left = [0x80_u8];
  let right = [0x00_u8];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let result = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(1)) };

  assert!(result.is_positive());
}

#[test]
fn memcmp_uses_first_differing_byte() {
  let left = [5_u8, 1, 255];
  let right = [5_u8, 2, 0];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let result = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(left.len())) };

  assert!(result.is_negative());
}

#[test]
fn memcmp_ignores_differences_past_requested_count() {
  let left = [7_u8, 3, 1];
  let right = [7_u8, 3, 9];
  // SAFETY: pointers originate from live arrays and length is in-bounds.
  let result = unsafe { memcmp(left.as_ptr().cast(), right.as_ptr().cast(), sz(2)) };

  assert_eq!(result, 0);
}
