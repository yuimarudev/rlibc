use rlibc::abi::types::c_int;
use rlibc::ctype::{
  isalnum, isalpha, isascii, isblank, iscntrl, isdigit, isgraph, islower, isprint, ispunct,
  isspace, isupper, isxdigit, toascii, tolower, toupper,
};
use rlibc::errno::__errno_location;

const EOF_C_INT: c_int = -1;

const fn as_c_int(byte: u8) -> c_int {
  byte as c_int
}

fn errno_ptr() -> *mut c_int {
  // `__errno_location` returns thread-local errno storage for the calling thread.
  let pointer = __errno_location();

  assert!(!pointer.is_null(), "__errno_location returned null");

  pointer
}

fn set_errno(value: c_int) {
  let pointer = errno_ptr();

  // SAFETY: `errno_ptr` returns writable thread-local storage.
  unsafe {
    pointer.write(value);
  }
}

fn errno_value() -> c_int {
  let pointer = errno_ptr();

  // SAFETY: `errno_ptr` returns readable thread-local storage.
  unsafe { pointer.read() }
}

fn assert_is_c_bool(value: c_int, context: &str) {
  assert!(
    matches!(value, 0 | 1),
    "predicate result must be 0 or 1: {context}, got={value}"
  );
}

fn assert_all_ctype_predicates_are_c_bool(probe: c_int) {
  assert_is_c_bool(isalpha(probe), "isalpha");
  assert_is_c_bool(isdigit(probe), "isdigit");
  assert_is_c_bool(isalnum(probe), "isalnum");
  assert_is_c_bool(islower(probe), "islower");
  assert_is_c_bool(isupper(probe), "isupper");
  assert_is_c_bool(isxdigit(probe), "isxdigit");
  assert_is_c_bool(isblank(probe), "isblank");
  assert_is_c_bool(isspace(probe), "isspace");
  assert_is_c_bool(iscntrl(probe), "iscntrl");
  assert_is_c_bool(isprint(probe), "isprint");
  assert_is_c_bool(isgraph(probe), "isgraph");
  assert_is_c_bool(ispunct(probe), "ispunct");
}

fn assert_errno_unchanged_after_ctype_calls(probe: c_int, sentinel: c_int) {
  set_errno(sentinel);

  isalnum(probe);
  isalpha(probe);
  isblank(probe);
  iscntrl(probe);
  isdigit(probe);
  isgraph(probe);
  islower(probe);
  isprint(probe);
  ispunct(probe);
  isspace(probe);
  isupper(probe);
  isxdigit(probe);
  tolower(probe);
  toupper(probe);

  assert_eq!(
    errno_value(),
    sentinel,
    "ctype call must not modify errno (probe={probe})"
  );
}

fn assert_each_ctype_call_preserves_errno(probe: c_int, sentinel: c_int) {
  set_errno(sentinel);
  isalnum(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isalnum changed errno (probe={probe})"
  );

  set_errno(sentinel);
  isalpha(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isalpha changed errno (probe={probe})"
  );

  set_errno(sentinel);
  isblank(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isblank changed errno (probe={probe})"
  );

  set_errno(sentinel);
  iscntrl(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "iscntrl changed errno (probe={probe})"
  );

  set_errno(sentinel);
  isdigit(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isdigit changed errno (probe={probe})"
  );

  set_errno(sentinel);
  isgraph(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isgraph changed errno (probe={probe})"
  );

  set_errno(sentinel);
  islower(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "islower changed errno (probe={probe})"
  );

  set_errno(sentinel);
  isprint(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isprint changed errno (probe={probe})"
  );

  set_errno(sentinel);
  ispunct(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "ispunct changed errno (probe={probe})"
  );

  set_errno(sentinel);
  isspace(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isspace changed errno (probe={probe})"
  );

  set_errno(sentinel);
  isupper(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isupper changed errno (probe={probe})"
  );

  set_errno(sentinel);
  isxdigit(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "isxdigit changed errno (probe={probe})"
  );

  set_errno(sentinel);
  tolower(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "tolower changed errno (probe={probe})"
  );

  set_errno(sentinel);
  toupper(probe);
  assert_eq!(
    errno_value(),
    sentinel,
    "toupper changed errno (probe={probe})"
  );
}

fn ctype_results_snapshot(probe: c_int) -> [c_int; 14] {
  [
    isalnum(probe),
    isalpha(probe),
    isblank(probe),
    iscntrl(probe),
    isdigit(probe),
    isgraph(probe),
    islower(probe),
    isprint(probe),
    ispunct(probe),
    isspace(probe),
    isupper(probe),
    isxdigit(probe),
    tolower(probe),
    toupper(probe),
  ]
}

fn case_insensitive_class_snapshot(probe: c_int) -> [c_int; 10] {
  [
    isalnum(probe),
    isalpha(probe),
    isblank(probe),
    iscntrl(probe),
    isdigit(probe),
    isgraph(probe),
    isprint(probe),
    ispunct(probe),
    isspace(probe),
    isxdigit(probe),
  ]
}

#[test]
fn ctype_predicates_accept_expected_ascii_examples() {
  assert_ne!(isalpha(as_c_int(b'A')), 0);
  assert_ne!(isalpha(as_c_int(b'z')), 0);
  assert_ne!(isdigit(as_c_int(b'0')), 0);
  assert_ne!(isalnum(as_c_int(b'7')), 0);
  assert_ne!(islower(as_c_int(b'q')), 0);
  assert_ne!(isupper(as_c_int(b'Q')), 0);
  assert_ne!(isxdigit(as_c_int(b'f')), 0);
  assert_ne!(isblank(as_c_int(b' ')), 0);
  assert_ne!(isblank(as_c_int(b'\t')), 0);
  assert_ne!(isspace(as_c_int(b'\n')), 0);
  assert_ne!(isspace(as_c_int(0x0B)), 0);
  assert_ne!(isspace(as_c_int(0x0C)), 0);
  assert_ne!(isspace(as_c_int(b'\r')), 0);
  assert_ne!(iscntrl(as_c_int(0x1F)), 0);
  assert_ne!(isprint(as_c_int(b'~')), 0);
  assert_ne!(isgraph(as_c_int(b'!')), 0);
  assert_ne!(ispunct(as_c_int(b'?')), 0);
}

#[test]
fn ctype_predicates_reject_non_matching_ascii_examples() {
  assert_eq!(isalpha(as_c_int(b'7')), 0);
  assert_eq!(isdigit(as_c_int(b'G')), 0);
  assert_eq!(isalnum(as_c_int(b'_')), 0);
  assert_eq!(islower(as_c_int(b'Q')), 0);
  assert_eq!(isupper(as_c_int(b'q')), 0);
  assert_eq!(isxdigit(as_c_int(b'g')), 0);
  assert_eq!(isblank(as_c_int(b'\n')), 0);
  assert_eq!(isspace(as_c_int(b'X')), 0);
  assert_eq!(iscntrl(as_c_int(b'A')), 0);
  assert_eq!(isprint(as_c_int(0x1F)), 0);
  assert_eq!(isgraph(as_c_int(b' ')), 0);
  assert_eq!(ispunct(as_c_int(b'A')), 0);
}

#[test]
fn ctype_predicates_return_strict_c_boolean_values() {
  let probe_values = [
    c_int::MIN,
    -1024,
    -2,
    EOF_C_INT,
    0,
    1,
    127,
    255,
    256,
    1024,
    c_int::MAX,
  ];

  for probe in probe_values {
    assert_all_ctype_predicates_are_c_bool(probe);
  }
}

#[test]
fn isascii_and_toascii_follow_ascii7bit_contract() {
  let boundary_probes = [c_int::MIN, -4096, -2, EOF_C_INT, 256, 4096, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let expected_ascii = byte <= 0x7F;

    assert_eq!(
      isascii(value) != 0,
      expected_ascii,
      "isascii mismatch for byte={byte:#04X}"
    );
    assert_eq!(
      toascii(value),
      value & 0x7F,
      "toascii must clear high bits for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let expected_ascii = (0..=0x7F).contains(&probe);

    assert_eq!(
      isascii(probe) != 0,
      expected_ascii,
      "isascii mismatch for probe={probe}"
    );

    let projected = toascii(probe);

    assert_eq!(
      projected,
      probe & 0x7F,
      "toascii must clear high bits for probe={probe}"
    );
    assert_eq!(
      toascii(projected),
      projected,
      "toascii must be idempotent for probe={probe}"
    );
  }
}

#[test]
fn isascii_matches_toascii_fixed_point_relation() {
  let boundary_probes = [c_int::MIN, -4096, -2, EOF_C_INT, 256, 4096, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let is_ascii7 = isascii(value) != 0;
    let is_toascii_fixed_point = toascii(value) == value;

    assert_eq!(
      is_ascii7, is_toascii_fixed_point,
      "isascii/toascii relation mismatch for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let is_ascii7 = isascii(probe) != 0;
    let is_toascii_fixed_point = toascii(probe) == probe;

    assert_eq!(
      is_ascii7, is_toascii_fixed_point,
      "isascii/toascii relation mismatch for probe={probe}"
    );
  }
}

#[test]
fn isascii_rejects_negative_byte_sized_values() {
  for probe in -128..=-1 {
    assert_eq!(isascii(probe), 0, "isascii must reject probe={probe}");
  }
}

#[test]
fn toascii_projection_lands_in_ascii7bit_domain() {
  let boundary_probes = [c_int::MIN, -4096, -2, EOF_C_INT, 256, 4096, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let projected = toascii(as_c_int(byte));

    assert!(
      (0..=0x7F).contains(&projected),
      "toascii projection out of ASCII7 domain for byte={byte:#04X}, projected={projected}"
    );
    assert_eq!(
      isascii(projected),
      1,
      "toascii projection must satisfy isascii for byte={byte:#04X}, projected={projected}"
    );
  }

  for probe in boundary_probes {
    let projected = toascii(probe);

    assert!(
      (0..=0x7F).contains(&projected),
      "toascii projection out of ASCII7 domain for probe={probe}, projected={projected}"
    );
    assert_eq!(
      isascii(projected),
      1,
      "toascii projection must satisfy isascii for probe={probe}, projected={projected}"
    );
  }
}

#[test]
fn toascii_matches_bitmask_for_dense_negative_range() {
  for probe in -1024..=-1 {
    let projected = toascii(probe);

    assert_eq!(
      projected,
      probe & 0x7F,
      "toascii mask mismatch for probe={probe}"
    );
    assert_eq!(
      isascii(probe),
      0,
      "isascii must reject negative probe={probe}"
    );
    assert_eq!(
      isascii(projected),
      1,
      "toascii projection must satisfy isascii for probe={probe}, projected={projected}"
    );
  }
}

#[test]
fn toascii_matches_bitmask_for_dense_positive_out_of_ascii_range() {
  for probe in 128..=4096 {
    let projected = toascii(probe);

    assert_eq!(
      projected,
      probe & 0x7F,
      "toascii mask mismatch for probe={probe}"
    );
    assert_eq!(
      isascii(probe),
      0,
      "isascii must reject non-ASCII positive probe={probe}"
    );
    assert_eq!(
      isascii(projected),
      1,
      "toascii projection must satisfy isascii for probe={probe}, projected={projected}"
    );
  }
}

#[test]
fn toascii_is_periodic_across_signed_128_step_offsets() {
  for base in 0..=0x7F {
    let base_value = base as c_int;

    for offset in -16..=16 {
      let shifted = base_value + (offset * 128);

      assert_eq!(
        toascii(shifted),
        base_value,
        "toascii periodicity mismatch for base={base_value}, offset={offset}, shifted={shifted}"
      );
      assert_eq!(
        isascii(shifted),
        c_int::from(offset == 0),
        "isascii periodicity mismatch for base={base_value}, offset={offset}, shifted={shifted}"
      );
    }
  }
}

#[test]
fn toascii_periodicity_holds_near_c_int_extremes_without_overflow() {
  let probes = [
    c_int::MIN,
    c_int::MIN + 1,
    -4096,
    -129,
    -128,
    -1,
    0,
    1,
    127,
    128,
    255,
    256,
    4096,
    c_int::MAX - 1,
    c_int::MAX,
  ];

  for probe in probes {
    if let Some(shifted_up) = probe.checked_add(128) {
      assert_eq!(
        toascii(shifted_up),
        toascii(probe),
        "toascii periodicity (+128) mismatch for probe={probe}, shifted={shifted_up}"
      );
    }

    if let Some(shifted_down) = probe.checked_sub(128) {
      assert_eq!(
        toascii(shifted_down),
        toascii(probe),
        "toascii periodicity (-128) mismatch for probe={probe}, shifted={shifted_down}"
      );
    }
  }
}

#[test]
fn toascii_is_idempotent_across_dense_signed_range() {
  for probe in -4096..=4096 {
    let projected = toascii(probe);

    assert_eq!(
      toascii(projected),
      projected,
      "toascii must be idempotent for probe={probe}, projected={projected}"
    );
    assert!(
      (0..=0x7F).contains(&projected),
      "toascii projection out of ASCII7 domain for probe={probe}, projected={projected}"
    );
  }
}

#[test]
fn isascii_returns_strict_c_boolean_values_across_domains() {
  let boundary_probes = [c_int::MIN, -4096, -2, EOF_C_INT, 256, 4096, c_int::MAX];

  for probe in boundary_probes {
    assert_is_c_bool(isascii(probe), "isascii");
  }

  for byte in 0_u8..=u8::MAX {
    assert_is_c_bool(isascii(as_c_int(byte)), "isascii");
  }
}

#[test]
fn isascii_matches_closed_range_predicate_for_dense_signed_range() {
  for probe in -4096..=4096 {
    assert_eq!(
      isascii(probe),
      c_int::from((0..=0x7F).contains(&probe)),
      "isascii range contract mismatch for probe={probe}"
    );
  }
}

#[test]
fn isascii_and_toascii_do_not_modify_errno() {
  let sentinels = [0, 17, 913, c_int::MAX];
  let boundary_probes = [c_int::MIN, -4096, -2, EOF_C_INT, 256, 4096, c_int::MAX];

  for sentinel in sentinels {
    for probe in boundary_probes {
      set_errno(sentinel);

      let _ = isascii(probe);

      assert_eq!(
        errno_value(),
        sentinel,
        "isascii changed errno for probe={probe}, sentinel={sentinel}"
      );

      set_errno(sentinel);

      let _ = toascii(probe);

      assert_eq!(
        errno_value(),
        sentinel,
        "toascii changed errno for probe={probe}, sentinel={sentinel}"
      );
    }

    for byte in 0_u8..=u8::MAX {
      let probe = as_c_int(byte);

      set_errno(sentinel);

      let _ = isascii(probe);

      assert_eq!(
        errno_value(),
        sentinel,
        "isascii changed errno for byte={byte:#04X}, sentinel={sentinel}"
      );

      set_errno(sentinel);

      let _ = toascii(probe);

      assert_eq!(
        errno_value(),
        sentinel,
        "toascii changed errno for byte={byte:#04X}, sentinel={sentinel}"
      );
    }
  }
}

#[test]
fn ctype_predicates_return_strict_c_boolean_values_for_full_byte_domain() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for probe in boundary_probes {
    assert_all_ctype_predicates_are_c_bool(probe);
  }

  for byte in 0_u8..=u8::MAX {
    assert_all_ctype_predicates_are_c_bool(as_c_int(byte));
  }
}

#[test]
fn ctype_functions_do_not_modify_errno() {
  let probes = [
    c_int::MIN,
    EOF_C_INT,
    as_c_int(0),
    as_c_int(b'A'),
    as_c_int(b' '),
    as_c_int(0x80),
    c_int::MAX,
  ];
  let sentinel = 321;

  for probe in probes {
    assert_errno_unchanged_after_ctype_calls(probe, sentinel);
  }
}

#[test]
fn ctype_functions_do_not_modify_errno_for_full_byte_domain() {
  let sentinel = 913;
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for probe in boundary_probes {
    assert_errno_unchanged_after_ctype_calls(probe, sentinel);
  }

  for byte in 0_u8..=u8::MAX {
    assert_errno_unchanged_after_ctype_calls(as_c_int(byte), sentinel);
  }
}

#[test]
fn ctype_functions_preserve_distinct_errno_sentinels() {
  let sentinels = [0, 1, 13, 321, c_int::MAX];
  let probes = [EOF_C_INT, as_c_int(b'A'), as_c_int(0x80), c_int::MAX];

  for sentinel in sentinels {
    for probe in probes {
      assert_errno_unchanged_after_ctype_calls(probe, sentinel);
    }
  }
}

#[test]
fn ctype_functions_do_not_modify_errno_near_domain_boundaries() {
  let sentinel = 777;

  for probe in -16..=272 {
    assert_errno_unchanged_after_ctype_calls(probe, sentinel);
  }
}

#[test]
fn ctype_each_call_preserves_errno_individually() {
  let sentinels = [2, 17, 1024];
  let probes = [
    c_int::MIN,
    EOF_C_INT,
    as_c_int(b'A'),
    as_c_int(0x80),
    c_int::MAX,
  ];

  for sentinel in sentinels {
    for probe in probes {
      assert_each_ctype_call_preserves_errno(probe, sentinel);
    }
  }
}

#[test]
fn ctype_results_are_independent_from_errno_value() {
  let probes = [
    c_int::MIN,
    EOF_C_INT,
    -2,
    as_c_int(0),
    as_c_int(b'A'),
    as_c_int(0x80),
    as_c_int(255),
    256,
    c_int::MAX,
  ];
  let sentinels = [0, 1, 17, 1024, c_int::MAX];

  for probe in probes {
    let baseline = ctype_results_snapshot(probe);

    for sentinel in sentinels {
      set_errno(sentinel);

      let with_sentinel = ctype_results_snapshot(probe);

      assert_eq!(
        with_sentinel, baseline,
        "ctype output changed with errno state (probe={probe}, sentinel={sentinel})"
      );
    }
  }
}

#[test]
fn ctype_results_near_domain_boundaries_are_independent_from_errno() {
  let sentinels = [0, 37, c_int::MAX];

  for probe in -16..=272 {
    let baseline = ctype_results_snapshot(probe);

    for sentinel in sentinels {
      set_errno(sentinel);

      let with_sentinel = ctype_results_snapshot(probe);

      assert_eq!(
        with_sentinel, baseline,
        "ctype output changed with errno state near boundary (probe={probe}, sentinel={sentinel})"
      );
    }
  }
}

#[test]
fn ctype_results_are_independent_from_errno_for_full_byte_domain() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];
  let sentinels = [0, 1, 37, 1024, c_int::MAX];

  for probe in boundary_probes {
    let baseline = ctype_results_snapshot(probe);

    for sentinel in sentinels {
      set_errno(sentinel);

      assert_eq!(
        ctype_results_snapshot(probe),
        baseline,
        "ctype output changed with errno state for boundary probe={probe}, sentinel={sentinel}"
      );
    }
  }

  for byte in 0_u8..=u8::MAX {
    let probe = as_c_int(byte);
    let baseline = ctype_results_snapshot(probe);

    for sentinel in sentinels {
      set_errno(sentinel);

      assert_eq!(
        ctype_results_snapshot(probe),
        baseline,
        "ctype output changed with errno state for byte={byte:#04X}, sentinel={sentinel}"
      );
    }
  }
}

#[test]
fn ctype_results_are_stable_across_repeated_calls() {
  let probes = [
    c_int::MIN,
    EOF_C_INT,
    -2,
    as_c_int(b'A'),
    as_c_int(0x80),
    256,
    c_int::MAX,
  ];

  for probe in probes {
    let first = ctype_results_snapshot(probe);
    let second = ctype_results_snapshot(probe);
    let third = ctype_results_snapshot(probe);

    assert_eq!(second, first, "second call changed outputs (probe={probe})");
    assert_eq!(third, first, "third call changed outputs (probe={probe})");
  }
}

#[test]
fn ctype_results_are_stable_across_repeated_calls_for_full_byte_domain() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for probe in boundary_probes {
    let baseline = ctype_results_snapshot(probe);

    for _ in 0..3 {
      assert_eq!(
        ctype_results_snapshot(probe),
        baseline,
        "repeated ctype outputs changed at boundary probe={probe}"
      );
    }
  }

  for byte in 0_u8..=u8::MAX {
    let probe = as_c_int(byte);
    let baseline = ctype_results_snapshot(probe);

    for _ in 0..3 {
      assert_eq!(
        ctype_results_snapshot(probe),
        baseline,
        "repeated ctype outputs changed for byte={byte:#04X}"
      );
    }
  }
}

#[test]
fn case_conversion_composition_is_consistent_across_domains() {
  let boundary_probes = [c_int::MIN, -1024, -2, EOF_C_INT, 256, 1024, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let probe = as_c_int(byte);

    assert_eq!(
      toupper(tolower(probe)),
      toupper(probe),
      "toupper(tolower(x)) mismatch for byte={byte:#04X}"
    );
    assert_eq!(
      tolower(toupper(probe)),
      tolower(probe),
      "tolower(toupper(x)) mismatch for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    assert_eq!(
      toupper(tolower(probe)),
      toupper(probe),
      "toupper(tolower(x)) mismatch for boundary probe={probe}"
    );
    assert_eq!(
      tolower(toupper(probe)),
      tolower(probe),
      "tolower(toupper(x)) mismatch for boundary probe={probe}"
    );
  }
}

#[test]
fn isspace_treats_vertical_tab_as_whitespace() {
  assert_ne!(isspace(as_c_int(0x0B)), 0);
}

#[test]
fn ctype_predicates_return_zero_for_eof_and_out_of_range_values() {
  let probes = [EOF_C_INT, -2, 256, 1024];

  for probe in probes {
    assert_eq!(isalnum(probe), 0);
    assert_eq!(isalpha(probe), 0);
    assert_eq!(isblank(probe), 0);
    assert_eq!(iscntrl(probe), 0);
    assert_eq!(isdigit(probe), 0);
    assert_eq!(isgraph(probe), 0);
    assert_eq!(islower(probe), 0);
    assert_eq!(isprint(probe), 0);
    assert_eq!(ispunct(probe), 0);
    assert_eq!(isspace(probe), 0);
    assert_eq!(isupper(probe), 0);
    assert_eq!(isxdigit(probe), 0);
  }
}

#[test]
fn ctype_functions_reject_extreme_c_int_values_without_truncation() {
  let probes = [c_int::MIN, c_int::MIN + 1, c_int::MAX - 1, c_int::MAX];

  for probe in probes {
    assert_eq!(isalnum(probe), 0);
    assert_eq!(isalpha(probe), 0);
    assert_eq!(isblank(probe), 0);
    assert_eq!(iscntrl(probe), 0);
    assert_eq!(isdigit(probe), 0);
    assert_eq!(isgraph(probe), 0);
    assert_eq!(islower(probe), 0);
    assert_eq!(isprint(probe), 0);
    assert_eq!(ispunct(probe), 0);
    assert_eq!(isspace(probe), 0);
    assert_eq!(isupper(probe), 0);
    assert_eq!(isxdigit(probe), 0);
    assert_eq!(tolower(probe), probe);
    assert_eq!(toupper(probe), probe);
  }
}

#[test]
fn ctype_functions_handle_wide_out_of_domain_values() {
  for probe in -1024..=1024 {
    if probe == EOF_C_INT || (0..=255).contains(&probe) {
      continue;
    }

    assert_eq!(isalnum(probe), 0);
    assert_eq!(isalpha(probe), 0);
    assert_eq!(isblank(probe), 0);
    assert_eq!(iscntrl(probe), 0);
    assert_eq!(isdigit(probe), 0);
    assert_eq!(isgraph(probe), 0);
    assert_eq!(islower(probe), 0);
    assert_eq!(isprint(probe), 0);
    assert_eq!(ispunct(probe), 0);
    assert_eq!(isspace(probe), 0);
    assert_eq!(isupper(probe), 0);
    assert_eq!(isxdigit(probe), 0);
    assert_eq!(tolower(probe), probe);
    assert_eq!(toupper(probe), probe);
  }
}

#[test]
fn tolower_converts_only_ascii_uppercase() {
  assert_eq!(tolower(as_c_int(b'A')), as_c_int(b'a'));
  assert_eq!(tolower(as_c_int(b'Z')), as_c_int(b'z'));
  assert_eq!(tolower(as_c_int(b'a')), as_c_int(b'a'));
  assert_eq!(tolower(as_c_int(b'7')), as_c_int(b'7'));
}

#[test]
fn toupper_converts_only_ascii_lowercase() {
  assert_eq!(toupper(as_c_int(b'a')), as_c_int(b'A'));
  assert_eq!(toupper(as_c_int(b'z')), as_c_int(b'Z'));
  assert_eq!(toupper(as_c_int(b'Z')), as_c_int(b'Z'));
  assert_eq!(toupper(as_c_int(b'7')), as_c_int(b'7'));
}

#[test]
fn to_case_functions_leave_eof_and_out_of_range_values_unchanged() {
  let probes = [EOF_C_INT, -2, 256, 1024];

  for probe in probes {
    assert_eq!(tolower(probe), probe);
    assert_eq!(toupper(probe), probe);
  }
}

#[test]
fn to_case_functions_are_stable_for_boundary_probes_outside_byte_domain() {
  let probes = [c_int::MIN, -4096, -2, EOF_C_INT, 256, 4096, c_int::MAX];

  for probe in probes {
    let lower = tolower(probe);
    let upper = toupper(probe);

    assert_eq!(
      lower, probe,
      "tolower must preserve out-of-domain probe={probe}"
    );
    assert_eq!(
      upper, probe,
      "toupper must preserve out-of-domain probe={probe}"
    );
    assert_eq!(
      tolower(lower),
      lower,
      "tolower must be idempotent for out-of-domain probe={probe}"
    );
    assert_eq!(
      toupper(upper),
      upper,
      "toupper must be idempotent for out-of-domain probe={probe}"
    );
  }
}

#[test]
fn isblank_and_isspace_have_expected_relationship() {
  assert_ne!(isblank(as_c_int(b' ')), 0);
  assert_ne!(isspace(as_c_int(b' ')), 0);
  assert_ne!(isblank(as_c_int(b'\t')), 0);
  assert_ne!(isspace(as_c_int(b'\t')), 0);
  assert_eq!(isblank(as_c_int(0x0B)), 0);
  assert_ne!(isspace(as_c_int(0x0B)), 0);
  assert_eq!(isblank(as_c_int(0x0C)), 0);
  assert_ne!(isspace(as_c_int(0x0C)), 0);
}

#[test]
fn isspace_accepts_only_c_locale_whitespace_bytes() {
  for byte in 0_u8..=0x7F {
    let expected = matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r');
    let actual = isspace(as_c_int(byte)) != 0;

    assert_eq!(actual, expected, "byte={byte:#04X}");
  }
}

#[test]
fn isspace_rejects_non_whitespace_control_bytes() {
  let controls = [0x00_u8, 0x07, 0x1C, 0x1D, 0x1E, 0x1F, 0x7F];

  for byte in controls {
    assert_eq!(isspace(as_c_int(byte)), 0, "byte={byte:#04X}");
  }
}

#[test]
fn del_byte_has_expected_ctype_membership() {
  let del = as_c_int(0x7F);

  assert_ne!(iscntrl(del), 0);
  assert_eq!(isspace(del), 0);
  assert_eq!(isprint(del), 0);
  assert_eq!(isgraph(del), 0);
  assert_eq!(ispunct(del), 0);
}

#[test]
fn iscntrl_respects_ascii_boundaries() {
  assert_ne!(iscntrl(as_c_int(0x00)), 0);
  assert_ne!(iscntrl(as_c_int(0x1F)), 0);
  assert_eq!(iscntrl(as_c_int(0x20)), 0);
  assert_eq!(iscntrl(as_c_int(0x7E)), 0);
  assert_ne!(iscntrl(as_c_int(0x7F)), 0);
  assert_eq!(iscntrl(as_c_int(0x80)), 0);
}

#[test]
fn isblank_respects_ascii_boundaries() {
  assert_eq!(isblank(as_c_int(0x08)), 0);
  assert_ne!(isblank(as_c_int(0x09)), 0);
  assert_eq!(isblank(as_c_int(0x0A)), 0);
  assert_ne!(isblank(as_c_int(0x20)), 0);
  assert_eq!(isblank(as_c_int(0x21)), 0);
  assert_eq!(isblank(as_c_int(0x7F)), 0);
}

#[test]
fn isblank_accepts_only_space_and_tab_over_full_byte_domain() {
  for byte in 0_u8..=u8::MAX {
    let expected = byte == b' ' || byte == b'\t';
    let actual = isblank(as_c_int(byte)) != 0;

    assert_eq!(actual, expected, "byte={byte:#04X}");
  }

  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for probe in boundary_probes {
    assert_eq!(isblank(probe), 0, "probe={probe}");
  }
}

#[test]
fn isxdigit_respects_ascii_boundaries() {
  assert_eq!(isxdigit(as_c_int(b'/')), 0);
  assert_ne!(isxdigit(as_c_int(b'0')), 0);
  assert_ne!(isxdigit(as_c_int(b'9')), 0);
  assert_eq!(isxdigit(as_c_int(b':')), 0);

  assert_eq!(isxdigit(as_c_int(b'@')), 0);
  assert_ne!(isxdigit(as_c_int(b'A')), 0);
  assert_ne!(isxdigit(as_c_int(b'F')), 0);
  assert_eq!(isxdigit(as_c_int(b'G')), 0);

  assert_eq!(isxdigit(as_c_int(b'`')), 0);
  assert_ne!(isxdigit(as_c_int(b'a')), 0);
  assert_ne!(isxdigit(as_c_int(b'f')), 0);
  assert_eq!(isxdigit(as_c_int(b'g')), 0);
}

#[test]
fn isalpha_respects_ascii_boundaries() {
  assert_eq!(isalpha(as_c_int(b'@')), 0);
  assert_ne!(isalpha(as_c_int(b'A')), 0);
  assert_ne!(isalpha(as_c_int(b'Z')), 0);
  assert_eq!(isalpha(as_c_int(b'[')), 0);

  assert_eq!(isalpha(as_c_int(b'`')), 0);
  assert_ne!(isalpha(as_c_int(b'a')), 0);
  assert_ne!(isalpha(as_c_int(b'z')), 0);
  assert_eq!(isalpha(as_c_int(b'{')), 0);
}

#[test]
fn isdigit_respects_ascii_boundaries() {
  assert_eq!(isdigit(as_c_int(b'/')), 0);
  assert_ne!(isdigit(as_c_int(b'0')), 0);
  assert_ne!(isdigit(as_c_int(b'9')), 0);
  assert_eq!(isdigit(as_c_int(b':')), 0);
}

#[test]
fn isalnum_respects_ascii_boundaries() {
  assert_eq!(isalnum(as_c_int(b'/')), 0);
  assert_ne!(isalnum(as_c_int(b'0')), 0);
  assert_ne!(isalnum(as_c_int(b'9')), 0);
  assert_eq!(isalnum(as_c_int(b':')), 0);

  assert_eq!(isalnum(as_c_int(b'@')), 0);
  assert_ne!(isalnum(as_c_int(b'A')), 0);
  assert_ne!(isalnum(as_c_int(b'Z')), 0);
  assert_eq!(isalnum(as_c_int(b'[')), 0);

  assert_eq!(isalnum(as_c_int(b'`')), 0);
  assert_ne!(isalnum(as_c_int(b'a')), 0);
  assert_ne!(isalnum(as_c_int(b'z')), 0);
  assert_eq!(isalnum(as_c_int(b'{')), 0);
}

#[test]
fn islower_and_isupper_respect_ascii_boundaries() {
  assert_eq!(islower(as_c_int(b'`')), 0);
  assert_ne!(islower(as_c_int(b'a')), 0);
  assert_ne!(islower(as_c_int(b'z')), 0);
  assert_eq!(islower(as_c_int(b'{')), 0);

  assert_eq!(isupper(as_c_int(b'@')), 0);
  assert_ne!(isupper(as_c_int(b'A')), 0);
  assert_ne!(isupper(as_c_int(b'Z')), 0);
  assert_eq!(isupper(as_c_int(b'[')), 0);
}

#[test]
fn ispunct_respects_ascii_boundaries() {
  assert_eq!(ispunct(as_c_int(b'/')), 1);
  assert_eq!(ispunct(as_c_int(b'0')), 0);
  assert_eq!(ispunct(as_c_int(b':')), 1);
  assert_eq!(ispunct(as_c_int(b'@')), 1);
  assert_eq!(ispunct(as_c_int(b'A')), 0);
  assert_eq!(ispunct(as_c_int(b'[')), 1);
  assert_eq!(ispunct(as_c_int(b'`')), 1);
  assert_eq!(ispunct(as_c_int(b'a')), 0);
  assert_eq!(ispunct(as_c_int(b'{')), 1);
}

#[test]
fn to_case_functions_respect_ascii_letter_boundaries() {
  assert_eq!(tolower(as_c_int(b'@')), as_c_int(b'@'));
  assert_eq!(tolower(as_c_int(b'A')), as_c_int(b'a'));
  assert_eq!(tolower(as_c_int(b'Z')), as_c_int(b'z'));
  assert_eq!(tolower(as_c_int(b'[')), as_c_int(b'['));

  assert_eq!(toupper(as_c_int(b'`')), as_c_int(b'`'));
  assert_eq!(toupper(as_c_int(b'a')), as_c_int(b'A'));
  assert_eq!(toupper(as_c_int(b'z')), as_c_int(b'Z'));
  assert_eq!(toupper(as_c_int(b'{')), as_c_int(b'{'));
}

#[test]
fn ascii_control_and_print_partition_ascii_domain() {
  for byte in 0_u8..=0x7F {
    let value = as_c_int(byte);
    let control = iscntrl(value) != 0;
    let printable = isprint(value) != 0;

    assert_ne!(control, printable, "byte={byte:#04X}");
  }
}

#[test]
fn ascii_ctype_class_cardinality_matches_c_locale_baseline() {
  let mut alpha_count = 0_usize;
  let mut digit_count = 0_usize;
  let mut alnum_count = 0_usize;
  let mut lower_count = 0_usize;
  let mut upper_count = 0_usize;
  let mut xdigit_count = 0_usize;
  let mut blank_count = 0_usize;
  let mut space_count = 0_usize;
  let mut control_count = 0_usize;
  let mut print_count = 0_usize;
  let mut graph_count = 0_usize;
  let mut punct_count = 0_usize;

  for byte in 0_u8..=0x7F {
    let value = as_c_int(byte);

    alpha_count += usize::from(isalpha(value) != 0);
    digit_count += usize::from(isdigit(value) != 0);
    alnum_count += usize::from(isalnum(value) != 0);
    lower_count += usize::from(islower(value) != 0);
    upper_count += usize::from(isupper(value) != 0);
    xdigit_count += usize::from(isxdigit(value) != 0);
    blank_count += usize::from(isblank(value) != 0);
    space_count += usize::from(isspace(value) != 0);
    control_count += usize::from(iscntrl(value) != 0);
    print_count += usize::from(isprint(value) != 0);
    graph_count += usize::from(isgraph(value) != 0);
    punct_count += usize::from(ispunct(value) != 0);
  }

  assert_eq!(alpha_count, 52, "alpha count in ASCII C locale");
  assert_eq!(digit_count, 10, "digit count in ASCII C locale");
  assert_eq!(alnum_count, 62, "alnum count in ASCII C locale");
  assert_eq!(lower_count, 26, "lower count in ASCII C locale");
  assert_eq!(upper_count, 26, "upper count in ASCII C locale");
  assert_eq!(xdigit_count, 22, "xdigit count in ASCII C locale");
  assert_eq!(blank_count, 2, "blank count in ASCII C locale");
  assert_eq!(space_count, 6, "space count in ASCII C locale");
  assert_eq!(control_count, 33, "control count in ASCII C locale");
  assert_eq!(print_count, 95, "print count in ASCII C locale");
  assert_eq!(graph_count, 94, "graph count in ASCII C locale");
  assert_eq!(punct_count, 32, "punct count in ASCII C locale");
}

#[test]
fn isprint_and_isgraph_respect_ascii_boundaries() {
  assert_eq!(isprint(as_c_int(0x1F)), 0);
  assert_ne!(isprint(as_c_int(0x20)), 0);
  assert_eq!(isgraph(as_c_int(0x20)), 0);

  assert_ne!(isprint(as_c_int(0x21)), 0);
  assert_ne!(isgraph(as_c_int(0x21)), 0);

  assert_ne!(isprint(as_c_int(0x7E)), 0);
  assert_ne!(isgraph(as_c_int(0x7E)), 0);

  assert_eq!(isprint(as_c_int(0x7F)), 0);
  assert_eq!(isgraph(as_c_int(0x7F)), 0);

  assert_eq!(isprint(as_c_int(0x80)), 0);
  assert_eq!(isgraph(as_c_int(0x80)), 0);
}

#[test]
fn isprint_and_isgraph_match_exact_ascii_intervals() {
  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let expected_print = (0x20..=0x7E).contains(&byte);
    let expected_graph = (0x21..=0x7E).contains(&byte);

    assert_eq!(
      isprint(value) != 0,
      expected_print,
      "isprint interval mismatch for byte={byte:#04X}"
    );
    assert_eq!(
      isgraph(value) != 0,
      expected_graph,
      "isgraph interval mismatch for byte={byte:#04X}"
    );
  }
}

#[test]
fn ispunct_matches_exact_ascii_intervals() {
  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let expected = matches!(
      byte,
      b'!'..=b'/' | b':'..=b'@' | b'['..=b'`' | b'{'..=b'~'
    );

    assert_eq!(
      ispunct(value) != 0,
      expected,
      "ispunct interval mismatch for byte={byte:#04X}"
    );
  }
}

#[test]
fn isprint_matches_not_iscntrl_within_byte_domain() {
  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let printable = isprint(value) != 0;
    let control = iscntrl(value) != 0;
    let expected_printable = !control && byte <= 0x7E;

    assert_eq!(
      printable, expected_printable,
      "isprint/iscntrl relation mismatch for byte={byte:#04X}"
    );
  }

  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for probe in boundary_probes {
    assert_eq!(isprint(probe), 0, "probe={probe}");
    assert_eq!(iscntrl(probe), 0, "probe={probe}");
  }
}

#[test]
fn isgraph_rejects_ascii_whitespace_set() {
  let whitespace = [b' ', b'\t', b'\n', 0x0B, 0x0C, b'\r'];

  for byte in whitespace {
    assert_eq!(isgraph(as_c_int(byte)), 0, "byte={byte:#04X}");
  }
}

#[test]
fn ctype_treats_extended_bytes_as_non_ascii_in_c_locale() {
  for byte in 0x80_u8..=u8::MAX {
    let value = as_c_int(byte);

    assert_eq!(isalpha(value), 0, "byte={byte:#04X}");
    assert_eq!(isdigit(value), 0, "byte={byte:#04X}");
    assert_eq!(isalnum(value), 0, "byte={byte:#04X}");
    assert_eq!(islower(value), 0, "byte={byte:#04X}");
    assert_eq!(isupper(value), 0, "byte={byte:#04X}");
    assert_eq!(isblank(value), 0, "byte={byte:#04X}");
    assert_eq!(isspace(value), 0, "byte={byte:#04X}");
    assert_eq!(iscntrl(value), 0, "byte={byte:#04X}");
    assert_eq!(isprint(value), 0, "byte={byte:#04X}");
    assert_eq!(isgraph(value), 0, "byte={byte:#04X}");
    assert_eq!(ispunct(value), 0, "byte={byte:#04X}");
    assert_eq!(tolower(value), value, "byte={byte:#04X}");
    assert_eq!(toupper(value), value, "byte={byte:#04X}");
  }
}

#[test]
fn ctype_predicates_match_ascii_contract_for_full_byte_domain() {
  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let expected_alpha = byte.is_ascii_alphabetic();
    let expected_digit = byte.is_ascii_digit();
    let expected_alnum = expected_alpha || expected_digit;
    let expected_lower = byte.is_ascii_lowercase();
    let expected_upper = byte.is_ascii_uppercase();
    let expected_hex =
      expected_digit || (b'A'..=b'F').contains(&byte) || (b'a'..=b'f').contains(&byte);
    let expected_blank = byte == b' ' || byte == b'\t';
    let expected_space = matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r');
    let expected_cntrl = byte <= 0x1F || byte == 0x7F;
    let expected_print = (0x20..=0x7E).contains(&byte);
    let expected_graph = (0x21..=0x7E).contains(&byte);
    let expected_punct = expected_graph && !expected_alnum;

    assert_eq!(isalpha(value) != 0, expected_alpha);
    assert_eq!(isdigit(value) != 0, expected_digit);
    assert_eq!(isalnum(value) != 0, expected_alnum);
    assert_eq!(islower(value) != 0, expected_lower);
    assert_eq!(isupper(value) != 0, expected_upper);
    assert_eq!(isxdigit(value) != 0, expected_hex);
    assert_eq!(isblank(value) != 0, expected_blank);
    assert_eq!(isspace(value) != 0, expected_space, "byte={byte:#04X}");
    assert_eq!(iscntrl(value) != 0, expected_cntrl);
    assert_eq!(isprint(value) != 0, expected_print);
    assert_eq!(isgraph(value) != 0, expected_graph);
    assert_eq!(ispunct(value) != 0, expected_punct);
  }
}

#[test]
fn to_case_functions_match_ascii_mapping_for_full_byte_domain() {
  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let lower_expected = if byte.is_ascii_uppercase() {
      as_c_int(byte + 32)
    } else {
      value
    };
    let upper_expected = if byte.is_ascii_lowercase() {
      as_c_int(byte - 32)
    } else {
      value
    };

    assert_eq!(tolower(value), lower_expected);
    assert_eq!(toupper(value), upper_expected);
  }
}

#[test]
fn case_conversion_images_have_expected_ascii_shape() {
  let mut lower_image = [false; 256];
  let mut upper_image = [false; 256];
  let mut lower_unique = 0_usize;
  let mut upper_unique = 0_usize;

  for byte in 0_u8..=0x7F {
    let value = as_c_int(byte);
    let lower = tolower(value);
    let upper = toupper(value);

    assert!(
      (0..=0x7F).contains(&lower),
      "tolower output must stay in ASCII for ASCII input: byte={byte:#04X}, out={lower}"
    );
    assert!(
      (0..=0x7F).contains(&upper),
      "toupper output must stay in ASCII for ASCII input: byte={byte:#04X}, out={upper}"
    );

    let lower_index = usize::try_from(lower).expect("ASCII lower output must fit usize index");

    if !lower_image[lower_index] {
      lower_image[lower_index] = true;
      lower_unique += 1;
    }

    let upper_index = usize::try_from(upper).expect("ASCII upper output must fit usize index");

    if !upper_image[upper_index] {
      upper_image[upper_index] = true;
      upper_unique += 1;
    }
  }

  for uppercase in b'A'..=b'Z' {
    assert!(
      !lower_image[usize::from(uppercase)],
      "tolower image for ASCII domain must not contain uppercase byte={uppercase:#04X}"
    );
  }

  for lowercase in b'a'..=b'z' {
    assert!(
      !upper_image[usize::from(lowercase)],
      "toupper image for ASCII domain must not contain lowercase byte={lowercase:#04X}"
    );
  }

  assert_eq!(lower_unique, 102, "tolower image size over ASCII domain");
  assert_eq!(upper_unique, 102, "toupper image size over ASCII domain");
}

#[test]
fn ctype_predicates_preserve_expected_relationships() {
  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let alpha = isalpha(value) != 0;
    let digit = isdigit(value) != 0;
    let alnum = isalnum(value) != 0;
    let lower = islower(value) != 0;
    let upper = isupper(value) != 0;
    let blank = isblank(value) != 0;
    let space = isspace(value) != 0;
    let graph = isgraph(value) != 0;
    let print = isprint(value) != 0;
    let punct = ispunct(value) != 0;

    assert_eq!(alnum, alpha || digit);
    assert_eq!(alpha, lower || upper);
    assert!(
      !(lower && upper),
      "byte cannot be both lower and upper case: {byte:#04X}"
    );
    assert!(!lower || alpha);
    assert!(!upper || alpha);
    assert!(!blank || space);
    assert_eq!(graph, print && !space);
    assert_eq!(punct, graph && !alnum);
  }
}

#[test]
fn ctype_predicate_relationships_hold_for_boundary_probes() {
  let probes = [c_int::MIN, -1024, -2, EOF_C_INT, 256, 1024, c_int::MAX];

  for probe in probes {
    let alpha = isalpha(probe) != 0;
    let digit = isdigit(probe) != 0;
    let alnum = isalnum(probe) != 0;
    let lower = islower(probe) != 0;
    let upper = isupper(probe) != 0;
    let blank = isblank(probe) != 0;
    let space = isspace(probe) != 0;
    let graph = isgraph(probe) != 0;
    let print = isprint(probe) != 0;
    let punct = ispunct(probe) != 0;

    assert_eq!(alnum, alpha || digit, "probe={probe}");
    assert_eq!(alpha, lower || upper, "probe={probe}");
    assert!(!(lower && upper), "probe={probe}");
    assert!(!blank || space, "probe={probe}");
    assert_eq!(graph, print && !space, "probe={probe}");
    assert_eq!(punct, graph && !alnum, "probe={probe}");
  }
}

#[test]
fn isgraph_matches_union_of_alnum_and_punct() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let graph = isgraph(value) != 0;
    let alnum = isalnum(value) != 0;
    let punct = ispunct(value) != 0;

    assert_eq!(
      graph,
      alnum || punct,
      "isgraph must equal (isalnum || ispunct) for byte={byte:#04X}"
    );
    assert!(
      !(alnum && punct),
      "isalnum and ispunct must be disjoint for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let graph = isgraph(probe) != 0;
    let alnum = isalnum(probe) != 0;
    let punct = ispunct(probe) != 0;

    assert_eq!(
      graph,
      alnum || punct,
      "isgraph must equal (isalnum || ispunct) for probe={probe}"
    );
    assert!(
      !(alnum && punct),
      "isalnum and ispunct must be disjoint for probe={probe}"
    );
  }
}

#[test]
fn isprint_matches_isgraph_or_space() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let printable = isprint(value) != 0;
    let graph = isgraph(value) != 0;
    let space = byte == b' ';

    assert_eq!(
      printable,
      graph || space,
      "isprint must equal (isgraph || ' ') for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let printable = isprint(probe) != 0;
    let graph = isgraph(probe) != 0;
    let space = probe == as_c_int(b' ');

    assert_eq!(
      printable,
      graph || space,
      "isprint must equal (isgraph || ' ') for probe={probe}"
    );
  }
}

#[test]
fn isxdigit_is_subset_of_alnum_and_excludes_punct() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let xdigit = isxdigit(value) != 0;
    let alnum = isalnum(value) != 0;
    let punct = ispunct(value) != 0;

    assert!(
      !xdigit || alnum,
      "isxdigit implies isalnum for byte={byte:#04X}"
    );
    assert!(
      !(xdigit && punct),
      "isxdigit and ispunct must be disjoint for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let xdigit = isxdigit(probe) != 0;
    let alnum = isalnum(probe) != 0;
    let punct = ispunct(probe) != 0;

    assert!(
      !xdigit || alnum,
      "isxdigit implies isalnum for probe={probe}"
    );
    assert!(
      !(xdigit && punct),
      "isxdigit and ispunct must be disjoint for probe={probe}"
    );
  }
}

#[test]
fn isxdigit_class_is_stable_under_ascii_case_conversion() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let xdigit = isxdigit(value) != 0;

    assert_eq!(
      isxdigit(tolower(value)) != 0,
      xdigit,
      "tolower must preserve xdigit membership for byte={byte:#04X}"
    );
    assert_eq!(
      isxdigit(toupper(value)) != 0,
      xdigit,
      "toupper must preserve xdigit membership for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let xdigit = isxdigit(probe) != 0;

    assert_eq!(
      isxdigit(tolower(probe)) != 0,
      xdigit,
      "tolower must preserve xdigit membership for probe={probe}"
    );
    assert_eq!(
      isxdigit(toupper(probe)) != 0,
      xdigit,
      "toupper must preserve xdigit membership for probe={probe}"
    );
  }
}

#[test]
fn isalpha_class_is_stable_under_ascii_case_conversion() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let alpha = isalpha(value) != 0;

    assert_eq!(
      isalpha(tolower(value)) != 0,
      alpha,
      "tolower must preserve alpha membership for byte={byte:#04X}"
    );
    assert_eq!(
      isalpha(toupper(value)) != 0,
      alpha,
      "toupper must preserve alpha membership for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let alpha = isalpha(probe) != 0;

    assert_eq!(
      isalpha(tolower(probe)) != 0,
      alpha,
      "tolower must preserve alpha membership for probe={probe}"
    );
    assert_eq!(
      isalpha(toupper(probe)) != 0,
      alpha,
      "toupper must preserve alpha membership for probe={probe}"
    );
  }
}

#[test]
fn isdigit_class_is_stable_under_ascii_case_conversion() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let digit = isdigit(value) != 0;

    assert_eq!(
      isdigit(tolower(value)) != 0,
      digit,
      "tolower must preserve digit membership for byte={byte:#04X}"
    );
    assert_eq!(
      isdigit(toupper(value)) != 0,
      digit,
      "toupper must preserve digit membership for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let digit = isdigit(probe) != 0;

    assert_eq!(
      isdigit(tolower(probe)) != 0,
      digit,
      "tolower must preserve digit membership for probe={probe}"
    );
    assert_eq!(
      isdigit(toupper(probe)) != 0,
      digit,
      "toupper must preserve digit membership for probe={probe}"
    );
  }
}

#[test]
fn isalnum_class_is_stable_under_ascii_case_conversion() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let alnum = isalnum(value) != 0;

    assert_eq!(
      isalnum(tolower(value)) != 0,
      alnum,
      "tolower must preserve alnum membership for byte={byte:#04X}"
    );
    assert_eq!(
      isalnum(toupper(value)) != 0,
      alnum,
      "toupper must preserve alnum membership for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let alnum = isalnum(probe) != 0;

    assert_eq!(
      isalnum(tolower(probe)) != 0,
      alnum,
      "tolower must preserve alnum membership for probe={probe}"
    );
    assert_eq!(
      isalnum(toupper(probe)) != 0,
      alnum,
      "toupper must preserve alnum membership for probe={probe}"
    );
  }
}

#[test]
fn whitespace_classes_are_stable_under_ascii_case_conversion() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let blank = isblank(value) != 0;
    let space = isspace(value) != 0;

    assert_eq!(
      isblank(tolower(value)) != 0,
      blank,
      "tolower must preserve blank membership for byte={byte:#04X}"
    );
    assert_eq!(
      isblank(toupper(value)) != 0,
      blank,
      "toupper must preserve blank membership for byte={byte:#04X}"
    );
    assert_eq!(
      isspace(tolower(value)) != 0,
      space,
      "tolower must preserve space membership for byte={byte:#04X}"
    );
    assert_eq!(
      isspace(toupper(value)) != 0,
      space,
      "toupper must preserve space membership for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let blank = isblank(probe) != 0;
    let space = isspace(probe) != 0;

    assert_eq!(
      isblank(tolower(probe)) != 0,
      blank,
      "tolower must preserve blank membership for probe={probe}"
    );
    assert_eq!(
      isblank(toupper(probe)) != 0,
      blank,
      "toupper must preserve blank membership for probe={probe}"
    );
    assert_eq!(
      isspace(tolower(probe)) != 0,
      space,
      "tolower must preserve space membership for probe={probe}"
    );
    assert_eq!(
      isspace(toupper(probe)) != 0,
      space,
      "toupper must preserve space membership for probe={probe}"
    );
  }
}

#[test]
fn visual_and_control_classes_are_stable_under_ascii_case_conversion() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let control = iscntrl(value) != 0;
    let printable = isprint(value) != 0;
    let graph = isgraph(value) != 0;
    let punct = ispunct(value) != 0;

    assert_eq!(
      iscntrl(tolower(value)) != 0,
      control,
      "tolower must preserve control membership for byte={byte:#04X}"
    );
    assert_eq!(
      iscntrl(toupper(value)) != 0,
      control,
      "toupper must preserve control membership for byte={byte:#04X}"
    );
    assert_eq!(
      isprint(tolower(value)) != 0,
      printable,
      "tolower must preserve print membership for byte={byte:#04X}"
    );
    assert_eq!(
      isprint(toupper(value)) != 0,
      printable,
      "toupper must preserve print membership for byte={byte:#04X}"
    );
    assert_eq!(
      isgraph(tolower(value)) != 0,
      graph,
      "tolower must preserve graph membership for byte={byte:#04X}"
    );
    assert_eq!(
      isgraph(toupper(value)) != 0,
      graph,
      "toupper must preserve graph membership for byte={byte:#04X}"
    );
    assert_eq!(
      ispunct(tolower(value)) != 0,
      punct,
      "tolower must preserve punct membership for byte={byte:#04X}"
    );
    assert_eq!(
      ispunct(toupper(value)) != 0,
      punct,
      "toupper must preserve punct membership for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let control = iscntrl(probe) != 0;
    let printable = isprint(probe) != 0;
    let graph = isgraph(probe) != 0;
    let punct = ispunct(probe) != 0;

    assert_eq!(
      iscntrl(tolower(probe)) != 0,
      control,
      "tolower must preserve control membership for probe={probe}"
    );
    assert_eq!(
      iscntrl(toupper(probe)) != 0,
      control,
      "toupper must preserve control membership for probe={probe}"
    );
    assert_eq!(
      isprint(tolower(probe)) != 0,
      printable,
      "tolower must preserve print membership for probe={probe}"
    );
    assert_eq!(
      isprint(toupper(probe)) != 0,
      printable,
      "toupper must preserve print membership for probe={probe}"
    );
    assert_eq!(
      isgraph(tolower(probe)) != 0,
      graph,
      "tolower must preserve graph membership for probe={probe}"
    );
    assert_eq!(
      isgraph(toupper(probe)) != 0,
      graph,
      "toupper must preserve graph membership for probe={probe}"
    );
    assert_eq!(
      ispunct(tolower(probe)) != 0,
      punct,
      "tolower must preserve punct membership for probe={probe}"
    );
    assert_eq!(
      ispunct(toupper(probe)) != 0,
      punct,
      "toupper must preserve punct membership for probe={probe}"
    );
  }
}

#[test]
fn case_normalization_projects_alpha_into_single_case_classes() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let alpha = isalpha(value) != 0;
    let lower = tolower(value);
    let upper = toupper(value);

    assert_eq!(
      islower(lower) != 0,
      alpha,
      "tolower(x) must be lowercase iff x is alpha for byte={byte:#04X}"
    );
    assert_eq!(
      isupper(lower),
      0,
      "tolower(x) must never classify as uppercase for byte={byte:#04X}"
    );
    assert_eq!(
      isupper(upper) != 0,
      alpha,
      "toupper(x) must be uppercase iff x is alpha for byte={byte:#04X}"
    );
    assert_eq!(
      islower(upper),
      0,
      "toupper(x) must never classify as lowercase for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let alpha = isalpha(probe) != 0;
    let lower = tolower(probe);
    let upper = toupper(probe);

    assert_eq!(
      islower(lower) != 0,
      alpha,
      "tolower(x) must be lowercase iff x is alpha for probe={probe}"
    );
    assert_eq!(
      isupper(lower),
      0,
      "tolower(x) must never classify as uppercase for probe={probe}"
    );
    assert_eq!(
      isupper(upper) != 0,
      alpha,
      "toupper(x) must be uppercase iff x is alpha for probe={probe}"
    );
    assert_eq!(
      islower(upper),
      0,
      "toupper(x) must never classify as lowercase for probe={probe}"
    );
  }
}

#[test]
fn case_roundtrip_preserves_case_insensitive_class_membership() {
  let boundary_probes = [c_int::MIN, -2, EOF_C_INT, 256, c_int::MAX];

  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let baseline = case_insensitive_class_snapshot(value);
    let lower_then_upper = toupper(tolower(value));
    let upper_then_lower = tolower(toupper(value));

    assert_eq!(
      case_insensitive_class_snapshot(lower_then_upper),
      baseline,
      "toupper(tolower(x)) changed case-insensitive classes for byte={byte:#04X}"
    );
    assert_eq!(
      case_insensitive_class_snapshot(upper_then_lower),
      baseline,
      "tolower(toupper(x)) changed case-insensitive classes for byte={byte:#04X}"
    );
  }

  for probe in boundary_probes {
    let baseline = case_insensitive_class_snapshot(probe);
    let lower_then_upper = toupper(tolower(probe));
    let upper_then_lower = tolower(toupper(probe));

    assert_eq!(
      case_insensitive_class_snapshot(lower_then_upper),
      baseline,
      "toupper(tolower(x)) changed case-insensitive classes for probe={probe}"
    );
    assert_eq!(
      case_insensitive_class_snapshot(upper_then_lower),
      baseline,
      "tolower(toupper(x)) changed case-insensitive classes for probe={probe}"
    );
  }
}

#[test]
fn ascii_bytes_are_exclusively_control_or_printable() {
  for byte in 0_u8..=0x7F {
    let value = as_c_int(byte);
    let control = iscntrl(value) != 0;
    let printable = isprint(value) != 0;

    assert_eq!(control, !printable, "byte={byte:#04X}");
  }
}

#[test]
fn to_case_functions_are_idempotent_and_round_trip_ascii_letters() {
  for byte in 0_u8..=u8::MAX {
    let value = as_c_int(byte);
    let lower_once = tolower(value);
    let lower_twice = tolower(lower_once);
    let upper_once = toupper(value);
    let upper_twice = toupper(upper_once);

    assert_eq!(lower_twice, lower_once);
    assert_eq!(upper_twice, upper_once);

    if isalpha(value) != 0 {
      assert_eq!(tolower(toupper(value)), lower_once);
      assert_eq!(toupper(tolower(value)), upper_once);
    }
  }
}
