use std::path::PathBuf;

fn read_manifest() -> (PathBuf, String) {
  let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  let manifest_path = manifest_dir.join("Cargo.toml");
  let manifest = std::fs::read_to_string(&manifest_path)
    .unwrap_or_else(|error| panic!("failed to read {}: {error}", manifest_path.display()));

  (manifest_path, manifest)
}

fn package_name_from_manifest(manifest: &str) -> Option<&str> {
  let mut in_package_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "name" {
      continue;
    }

    let value = value.trim();

    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
      return Some(&value[1..value.len() - 1]);
    }
  }

  None
}

fn package_autolib_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_package_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "autolib" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn package_default_run_from_manifest(manifest: &str) -> Option<String> {
  let mut in_package_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "default-run" {
      continue;
    }

    let value = value.trim();

    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
      return Some(value[1..value.len() - 1].to_string());
    }
  }

  None
}

fn package_autobins_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_package_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "autobins" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn package_autoexamples_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_package_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "autoexamples" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn package_autotests_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_package_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "autotests" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn package_autobenches_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_package_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "autobenches" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn lib_proc_macro_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "proc-macro" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn lib_harness_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "harness" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn lib_test_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "test" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn lib_doctest_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "doctest" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn lib_bench_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "bench" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn lib_doc_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "doc" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn lib_plugin_from_manifest(manifest: &str) -> Option<bool> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "plugin" {
      continue;
    }

    return match value.trim() {
      "true" => Some(true),
      "false" => Some(false),
      _ => None,
    };
  }

  None
}

fn crate_types_from_manifest(manifest: &str) -> Vec<String> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "crate-type" {
      continue;
    }

    let value = value.trim();
    let Some(stripped) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
      return Vec::new();
    };

    return stripped
      .split(',')
      .map(str::trim)
      .filter(|entry| entry.len() >= 2 && entry.starts_with('"') && entry.ends_with('"'))
      .map(|entry| entry[1..entry.len() - 1].to_string())
      .collect();
  }

  Vec::new()
}

fn lib_path_from_manifest(manifest: &str) -> Option<String> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "path" {
      continue;
    }

    let value = value.trim();

    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
      return Some(value[1..value.len() - 1].to_string());
    }
  }

  None
}

fn lib_name_from_manifest(manifest: &str) -> Option<String> {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() != "name" {
      continue;
    }

    let value = value.trim();

    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
      return Some(value[1..value.len() - 1].to_string());
    }
  }

  None
}

fn manifest_has_bin_targets(manifest: &str) -> bool {
  manifest.lines().any(|line| line.trim() == "[[bin]]")
}

fn section_count(manifest: &str, section_name: &str) -> usize {
  manifest
    .lines()
    .map(str::trim)
    .filter(|line| *line == section_name)
    .count()
}

fn lib_section_has_key(manifest: &str, key_name: &str) -> bool {
  let mut in_lib_section = false;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, _value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() == key_name {
      return true;
    }
  }

  false
}

fn lib_section_keys(manifest: &str) -> Vec<String> {
  let mut in_lib_section = false;
  let mut keys = Vec::new();

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, _value)) = trimmed.split_once('=') else {
      continue;
    };

    keys.push(key.trim().to_string());
  }

  keys
}

fn lib_section_key_occurrences(manifest: &str, key_name: &str) -> usize {
  let mut in_lib_section = false;
  let mut occurrences = 0;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_lib_section = trimmed == "[lib]";
      continue;
    }

    if !in_lib_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, _value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() == key_name {
      occurrences += 1;
    }
  }

  occurrences
}

fn package_section_keys(manifest: &str) -> Vec<String> {
  let mut in_package_section = false;
  let mut keys = Vec::new();

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, _value)) = trimmed.split_once('=') else {
      continue;
    };

    keys.push(key.trim().to_string());
  }

  keys
}

fn package_section_key_occurrences(manifest: &str, key_name: &str) -> usize {
  let mut in_package_section = false;
  let mut occurrences = 0;

  for line in manifest.lines() {
    let trimmed = line.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
      in_package_section = trimmed == "[package]";
      continue;
    }

    if !in_package_section || trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }

    let Some((key, _value)) = trimmed.split_once('=') else {
      continue;
    };

    if key.trim() == key_name {
      occurrences += 1;
    }
  }

  occurrences
}

#[test]
fn rlibc_is_library_shaped_without_default_main_entrypoint() {
  let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  let lib_entrypoint = manifest_dir.join("src/lib.rs");
  let bin_entrypoint = manifest_dir.join("src/main.rs");

  assert!(
    lib_entrypoint.exists(),
    "library entrypoint must exist at src/lib.rs"
  );
  assert!(
    !bin_entrypoint.exists(),
    "default binary entrypoint must be absent for library-shape issue I001"
  );
  assert_eq!(rlibc::project_name(), "rlibc");
}

#[test]
fn cargo_manifest_declares_required_library_crate_types() {
  let (_, manifest) = read_manifest();
  let mut crate_types = crate_types_from_manifest(&manifest);

  crate_types.sort();

  assert!(
    manifest.contains("[lib]"),
    "Cargo.toml must define a [lib] section"
  );
  assert_eq!(
    crate_types,
    vec![
      "cdylib".to_string(),
      "rlib".to_string(),
      "staticlib".to_string()
    ],
    "Cargo.toml [lib].crate-type must expose rlib/cdylib/staticlib"
  );
}

#[test]
fn cargo_manifest_library_section_exposes_only_crate_type_key() {
  let (_, manifest) = read_manifest();
  let mut keys = lib_section_keys(&manifest);

  keys.sort();
  keys.dedup();

  assert_eq!(
    keys,
    vec!["crate-type".to_string()],
    "Cargo.toml [lib] must only define crate-type for I001 library shape"
  );
}

#[test]
fn cargo_manifest_defines_library_crate_type_once() {
  let (_, manifest) = read_manifest();

  assert_eq!(
    lib_section_key_occurrences(&manifest, "crate-type"),
    1,
    "Cargo.toml [lib].crate-type must be defined exactly once for I001 library shape"
  );
}

#[test]
fn project_name_matches_manifest_package_name() {
  let (manifest_path, manifest) = read_manifest();
  let package_name = package_name_from_manifest(&manifest).unwrap_or_else(|| {
    panic!(
      "failed to locate [package].name in {}",
      manifest_path.display()
    )
  });

  assert_eq!(
    rlibc::project_name(),
    package_name,
    "project_name() should track Cargo.toml [package].name to preserve library identity"
  );
}

#[test]
fn project_name_c_string_accessors_stay_consistent() {
  let c_name = rlibc::project_name_cstr();
  let name_bytes = rlibc::project_name().as_bytes();
  let c_name_bytes = rlibc::project_name_cstr_bytes();

  assert_eq!(
    c_name.to_bytes(),
    name_bytes,
    "project_name_cstr() payload should match project_name() bytes"
  );
  assert_eq!(
    c_name.as_ptr(),
    rlibc::project_name_cstr_ptr(),
    "project_name_cstr() pointer should match project_name_cstr_ptr()"
  );
  assert_eq!(
    rlibc::project_name_cstr_len(),
    name_bytes.len(),
    "project_name_cstr_len() should match project_name() byte length"
  );
  assert_eq!(
    c_name.to_bytes().len(),
    rlibc::project_name_cstr_len(),
    "project_name_cstr() payload length should match project_name_cstr_len()"
  );
  assert_eq!(
    c_name_bytes.len(),
    rlibc::project_name_cstr_len() + 1,
    "project_name_cstr_bytes() should include exactly one trailing NUL byte"
  );
  assert_eq!(
    c_name_bytes.last(),
    Some(&0),
    "project_name_cstr_bytes() should be NUL-terminated"
  );
  assert_eq!(
    c_name.to_bytes_with_nul(),
    c_name_bytes,
    "project_name_cstr().to_bytes_with_nul() should match project_name_cstr_bytes()"
  );
  assert_eq!(
    rlibc::project_name_cstr_ptr().cast::<u8>(),
    c_name_bytes.as_ptr(),
    "project_name_cstr_ptr() should point at the same static bytes as project_name_cstr_bytes()"
  );
  assert!(
    !rlibc::project_name_cstr_ptr().is_null(),
    "project_name_cstr_ptr() should never return a null pointer"
  );
  assert_eq!(
    c_name.to_bytes().as_ptr(),
    c_name_bytes.as_ptr(),
    "project_name_cstr() payload should start at the same address as project_name_cstr_bytes()"
  );
  assert_eq!(
    c_name_bytes[rlibc::project_name_cstr_len()],
    0,
    "project_name_cstr_len() should point to the trailing NUL byte index"
  );
  assert_eq!(
    c_name.to_bytes_with_nul().len(),
    c_name.to_bytes().len() + 1,
    "project_name_cstr() should expose exactly one trailing NUL byte"
  );
  assert_eq!(
    c_name.to_bytes_with_nul().last(),
    Some(&0),
    "project_name_cstr().to_bytes_with_nul() should end with NUL"
  );
  assert_eq!(
    c_name.to_bytes_with_nul()[rlibc::project_name_cstr_len()],
    0,
    "project_name_cstr_len() should index the trailing NUL in to_bytes_with_nul()"
  );
}

#[test]
fn cargo_manifest_uses_default_library_entrypoint_path() {
  let (_, manifest) = read_manifest();
  let lib_path = lib_path_from_manifest(&manifest);

  assert!(
    lib_path.as_deref().is_none_or(|path| path == "src/lib.rs"),
    "Cargo.toml [lib].path must be omitted or set to src/lib.rs for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_path_override() {
  let (_, manifest) = read_manifest();

  assert!(
    !lib_section_has_key(&manifest, "path"),
    "Cargo.toml [lib].path must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_edition_override() {
  let (_, manifest) = read_manifest();

  assert!(
    !lib_section_has_key(&manifest, "edition"),
    "Cargo.toml [lib].edition must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_bin_targets_for_library_shape() {
  let (_, manifest) = read_manifest();

  assert!(
    !manifest_has_bin_targets(&manifest),
    "Cargo.toml must not declare [[bin]] targets for I001 library shape"
  );
}

#[test]
fn cargo_manifest_uses_default_library_name() {
  let (_, manifest) = read_manifest();
  let lib_name = lib_name_from_manifest(&manifest);

  assert!(
    lib_name.as_deref().is_none_or(|name| name == "rlibc"),
    "Cargo.toml [lib].name must be omitted or set to rlibc for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_name_override() {
  let (_, manifest) = read_manifest();

  assert!(
    !lib_section_has_key(&manifest, "name"),
    "Cargo.toml [lib].name must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_library_name_matches_package_name_when_explicit() {
  let (manifest_path, manifest) = read_manifest();
  let package_name = package_name_from_manifest(&manifest).unwrap_or_else(|| {
    panic!(
      "failed to locate [package].name in {}",
      manifest_path.display()
    )
  });
  let lib_name = lib_name_from_manifest(&manifest);

  assert!(
    lib_name.as_deref().is_none_or(|name| name == package_name),
    "Cargo.toml [lib].name must match [package].name when explicitly configured"
  );
}

#[test]
fn cargo_manifest_keeps_library_auto_discovery_enabled() {
  let (_, manifest) = read_manifest();
  let package_autolib = package_autolib_from_manifest(&manifest);

  assert!(
    package_autolib.is_none_or(|enabled| enabled),
    "Cargo.toml [package].autolib must be omitted or true for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_autolib_override() {
  let (_, manifest) = read_manifest();

  assert!(
    package_autolib_from_manifest(&manifest).is_none(),
    "Cargo.toml [package].autolib must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_defines_single_library_section() {
  let (_, manifest) = read_manifest();

  assert_eq!(
    section_count(&manifest, "[lib]"),
    1,
    "Cargo.toml must define exactly one [lib] section for I001 library shape"
  );
}

#[test]
fn cargo_manifest_defines_single_package_section() {
  let (_, manifest) = read_manifest();

  assert_eq!(
    section_count(&manifest, "[package]"),
    1,
    "Cargo.toml must define exactly one [package] section for I001 library identity"
  );
}

#[test]
fn cargo_manifest_package_section_exposes_expected_keys() {
  let (_, manifest) = read_manifest();
  let mut keys = package_section_keys(&manifest);

  keys.sort();
  keys.dedup();

  assert_eq!(
    keys,
    vec![
      "edition.workspace".to_string(),
      "name".to_string(),
      "version.workspace".to_string()
    ],
    "Cargo.toml [package] should keep the minimal key set expected for I001 library shape"
  );
}

#[test]
fn cargo_manifest_defines_package_name_once() {
  let (_, manifest) = read_manifest();

  assert_eq!(
    package_section_key_occurrences(&manifest, "name"),
    1,
    "Cargo.toml [package].name must be defined exactly once for I001 library identity"
  );
}

#[test]
fn cargo_manifest_has_no_default_run_binary() {
  let (_, manifest) = read_manifest();

  assert!(
    package_default_run_from_manifest(&manifest).is_none(),
    "Cargo.toml [package].default-run must be absent for I001 library shape"
  );
}

#[test]
fn cargo_manifest_does_not_enable_automatic_bin_discovery() {
  let (_, manifest) = read_manifest();
  let package_autobins = package_autobins_from_manifest(&manifest);

  assert!(
    package_autobins.is_none_or(|enabled| !enabled),
    "Cargo.toml [package].autobins must be omitted or false for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_autobins_override() {
  let (_, manifest) = read_manifest();

  assert!(
    package_autobins_from_manifest(&manifest).is_none(),
    "Cargo.toml [package].autobins must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_does_not_enable_automatic_example_discovery() {
  let (_, manifest) = read_manifest();
  let package_autoexamples = package_autoexamples_from_manifest(&manifest);

  assert!(
    package_autoexamples.is_none_or(|enabled| !enabled),
    "Cargo.toml [package].autoexamples must be omitted or false for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_autoexamples_override() {
  let (_, manifest) = read_manifest();

  assert!(
    package_autoexamples_from_manifest(&manifest).is_none(),
    "Cargo.toml [package].autoexamples must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_autotests_override() {
  let (_, manifest) = read_manifest();

  assert!(
    package_autotests_from_manifest(&manifest).is_none(),
    "Cargo.toml [package].autotests must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_does_not_enable_automatic_test_discovery() {
  let (_, manifest) = read_manifest();
  let package_autotests = package_autotests_from_manifest(&manifest);

  assert!(
    package_autotests.is_none_or(|enabled| !enabled),
    "Cargo.toml [package].autotests must be omitted or false for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_autobenches_override() {
  let (_, manifest) = read_manifest();

  assert!(
    package_autobenches_from_manifest(&manifest).is_none(),
    "Cargo.toml [package].autobenches must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_does_not_enable_automatic_bench_discovery() {
  let (_, manifest) = read_manifest();
  let package_autobenches = package_autobenches_from_manifest(&manifest);

  assert!(
    package_autobenches.is_none_or(|enabled| !enabled),
    "Cargo.toml [package].autobenches must be omitted or false for I001 library shape"
  );
}

#[test]
fn cargo_manifest_library_is_not_proc_macro() {
  let (_, manifest) = read_manifest();
  let proc_macro = lib_proc_macro_from_manifest(&manifest);

  assert!(
    proc_macro.is_none_or(|enabled| !enabled),
    "Cargo.toml [lib].proc-macro must be omitted or false for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_proc_macro_override() {
  let (_, manifest) = read_manifest();

  assert!(
    !lib_section_has_key(&manifest, "proc-macro"),
    "Cargo.toml [lib].proc-macro must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_harness_override() {
  let (_, manifest) = read_manifest();

  assert!(
    lib_harness_from_manifest(&manifest).is_none(),
    "Cargo.toml [lib].harness must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_test_override() {
  let (_, manifest) = read_manifest();

  assert!(
    lib_test_from_manifest(&manifest).is_none(),
    "Cargo.toml [lib].test must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_doctest_override() {
  let (_, manifest) = read_manifest();

  assert!(
    lib_doctest_from_manifest(&manifest).is_none(),
    "Cargo.toml [lib].doctest must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_bench_override() {
  let (_, manifest) = read_manifest();

  assert!(
    lib_bench_from_manifest(&manifest).is_none(),
    "Cargo.toml [lib].bench must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_doc_override() {
  let (_, manifest) = read_manifest();

  assert!(
    lib_doc_from_manifest(&manifest).is_none(),
    "Cargo.toml [lib].doc must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_plugin_override() {
  let (_, manifest) = read_manifest();

  assert!(
    lib_plugin_from_manifest(&manifest).is_none(),
    "Cargo.toml [lib].plugin must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_library_required_features_gate() {
  let (_, manifest) = read_manifest();

  assert!(
    !lib_section_has_key(&manifest, "required-features"),
    "Cargo.toml [lib].required-features must be omitted for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_example_targets() {
  let (_, manifest) = read_manifest();

  assert_eq!(
    section_count(&manifest, "[[example]]"),
    0,
    "Cargo.toml must not declare [[example]] targets for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_bench_targets() {
  let (_, manifest) = read_manifest();

  assert_eq!(
    section_count(&manifest, "[[bench]]"),
    0,
    "Cargo.toml must not declare [[bench]] targets for I001 library shape"
  );
}

#[test]
fn cargo_manifest_has_no_manifest_test_targets() {
  let (_, manifest) = read_manifest();

  assert_eq!(
    section_count(&manifest, "[[test]]"),
    0,
    "Cargo.toml must not declare [[test]] targets for I001 library shape"
  );
}
