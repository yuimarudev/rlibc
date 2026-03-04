use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn map_path() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rlibc.map")
}

fn golden_snapshot_path() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("abi/golden/x86_64-unknown-linux-gnu.abi")
}

fn ctype_header_path() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/ctype.h")
}

fn build_script_path() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build.rs")
}

fn parse_global_symbols(version_script: &str) -> Vec<String> {
  let mut in_global_block = false;
  let mut symbols = Vec::new();

  for raw_line in version_script.lines() {
    let line = raw_line.trim();

    if line.starts_with("global:") {
      in_global_block = true;

      continue;
    }

    if line.starts_with("local:") {
      in_global_block = false;

      continue;
    }

    if !in_global_block || line.is_empty() || !line.ends_with(';') {
      continue;
    }

    let name = line.trim_end_matches(';').trim();

    if !name.is_empty() {
      symbols.push(name.to_string());
    }
  }

  symbols
}

fn parse_golden_symbols(snapshot: &str) -> Vec<String> {
  let mut in_symbols = false;
  let mut symbols = Vec::new();

  for raw_line in snapshot.lines() {
    let line = raw_line.trim();

    if line == "SYMBOLS:" {
      in_symbols = true;

      continue;
    }

    if !in_symbols || line.is_empty() {
      continue;
    }

    symbols.push(line.to_string());
  }

  symbols
}

fn parse_c_header_int_function_names(header: &str) -> BTreeSet<String> {
  header
    .lines()
    .filter_map(|raw_line| {
      let line = raw_line.trim();

      if !line.starts_with("int ") || !line.ends_with(';') {
        return None;
      }

      let signature = line.strip_prefix("int ")?.trim_end_matches(';').trim();
      let (name, _params) = signature.split_once('(')?;
      let symbol = name.trim();

      if symbol.is_empty() {
        return None;
      }

      Some(symbol.to_string())
    })
    .collect()
}

#[test]
fn symbol_map_lists_current_c_abi_exports() {
  let version_script = fs::read_to_string(map_path()).expect("failed to read rlibc.map");
  let golden_snapshot =
    fs::read_to_string(golden_snapshot_path()).expect("failed to read golden abi snapshot");
  let expected = parse_golden_symbols(&golden_snapshot);
  let actual = parse_global_symbols(&version_script);
  let expected_set: BTreeSet<String> = expected.iter().cloned().collect();
  let actual_set: BTreeSet<String> = actual.iter().cloned().collect();

  assert!(
    !actual.is_empty(),
    "symbol map must export at least one symbol"
  );
  assert_eq!(
    actual.len(),
    actual_set.len(),
    "version-script global exports must not contain duplicate symbol entries"
  );
  assert_eq!(
    expected.len(),
    expected_set.len(),
    "golden ABI snapshot symbols must remain unique"
  );

  assert_eq!(
    actual_set, expected_set,
    "version-script global exports must stay in sync with the golden ABI symbol list"
  );
}

#[test]
fn ctype_header_declarations_match_version_script_exports() {
  let version_script = fs::read_to_string(map_path()).expect("failed to read rlibc.map");
  let ctype_header =
    fs::read_to_string(ctype_header_path()).expect("failed to read include/ctype.h");
  let exported: BTreeSet<String> = parse_global_symbols(&version_script).into_iter().collect();
  let declared = parse_c_header_int_function_names(&ctype_header);
  let legacy_non_header_symbols: BTreeSet<String> = ["isascii", "toascii"]
    .into_iter()
    .map(str::to_string)
    .collect();
  let ctype_exports_in_map: BTreeSet<String> = exported
    .iter()
    .filter(|symbol| declared.contains(*symbol) || legacy_non_header_symbols.contains(*symbol))
    .cloned()
    .collect();

  assert!(
    !declared.is_empty(),
    "include/ctype.h must declare ctype exports"
  );
  assert_eq!(
    ctype_exports_in_map, declared,
    "version script ctype exports must match include/ctype.h declarations"
  );

  for symbol in legacy_non_header_symbols {
    assert!(
      !exported.contains(&symbol),
      "version script must not export legacy non-header ctype symbol `{symbol}`",
    );
  }
}

#[test]
fn symbol_map_includes_pthread_cond_exports() {
  let version_script = fs::read_to_string(map_path()).expect("failed to read rlibc.map");
  let exported: BTreeSet<String> = parse_global_symbols(&version_script).into_iter().collect();
  let required_cond_symbols = [
    "pthread_condattr_init",
    "pthread_condattr_destroy",
    "pthread_cond_init",
    "pthread_cond_destroy",
    "pthread_cond_wait",
    "pthread_cond_timedwait",
    "pthread_cond_signal",
    "pthread_cond_broadcast",
    "pthread_condattr_getpshared",
    "pthread_condattr_setpshared",
  ];

  for symbol in required_cond_symbols {
    assert!(
      exported.contains(symbol),
      "version script must export required condvar symbol `{symbol}`",
    );
  }
}

#[test]
fn symbol_map_has_catch_all_local_visibility_rule() {
  let version_script = fs::read_to_string(map_path()).expect("failed to read rlibc.map");

  assert!(
    version_script.contains("local:"),
    "version script must define a local block",
  );
  assert!(
    version_script.contains("*;"),
    "version script must hide unspecified symbols with '*'",
  );
}

#[test]
fn build_script_links_cdylib_with_version_script() {
  let build_script = fs::read_to_string(build_script_path()).expect("failed to read build.rs");

  assert!(
    build_script.contains("rustc-cdylib-link-arg"),
    "build script must pass cdylib linker args",
  );
  assert!(
    build_script.contains("--version-script"),
    "build script must enable linker version script",
  );
  assert!(
    build_script.contains("rerun-if-changed=rlibc.map"),
    "build script must rerun when map changes",
  );
}
