use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{fs, process};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

fn repository_file(path: &str) -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn unique_temp_snapshot_path(label: &str) -> PathBuf {
  let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);

  std::env::temp_dir().join(format!(
    "rlibc-abi-golden-{label}-{}-{id}.abi",
    process::id()
  ))
}

fn build_release_cdylib() {
  let output = Command::new("cargo")
    .arg("rustc")
    .arg("--release")
    .arg("--lib")
    .arg("--crate-type")
    .arg("cdylib")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to run cargo rustc for release cdylib");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "release cdylib build must succeed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
}

fn parse_global_symbols(version_script: &str) -> BTreeSet<String> {
  let mut in_global_block = false;
  let mut symbols = BTreeSet::new();

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

    let symbol = line.trim_end_matches(';').trim();

    if !symbol.is_empty() {
      symbols.insert(symbol.to_string());
    }
  }

  symbols
}

fn parse_golden_snapshot(snapshot: &str) -> (String, String, BTreeSet<String>) {
  let mut class = None;
  let mut machine = None;
  let mut symbols = BTreeSet::new();
  let mut in_symbols = false;

  for (index, raw_line) in snapshot.lines().enumerate() {
    let line = raw_line.trim();

    if index == 0 {
      assert_eq!(line, "ABI_SNAPSHOT_V1", "golden snapshot magic mismatch");

      continue;
    }

    if line.is_empty() {
      continue;
    }

    if line == "SYMBOLS:" {
      in_symbols = true;

      continue;
    }

    if in_symbols {
      symbols.insert(line.to_string());

      continue;
    }

    if let Some(value) = line.strip_prefix("ELF_CLASS=") {
      class = Some(value.trim().to_string());

      continue;
    }

    if let Some(value) = line.strip_prefix("ELF_MACHINE=") {
      machine = Some(value.trim().to_string());

      continue;
    }

    panic!("unexpected line in golden snapshot: {line}");
  }

  let class = class.expect("golden snapshot must define ELF_CLASS");
  let machine = machine.expect("golden snapshot must define ELF_MACHINE");

  (class, machine, symbols)
}

fn parse_golden_symbol_lines(snapshot: &str) -> Vec<String> {
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

fn parse_nm_dynamic_symbols(output: &str) -> BTreeSet<String> {
  output
    .lines()
    .filter_map(|line| {
      let symbol = line.split_whitespace().next()?;

      if symbol.ends_with(':') {
        return None;
      }

      let base = symbol.split('@').next().unwrap_or(symbol);

      if base.is_empty() {
        return None;
      }

      Some(base.to_string())
    })
    .collect()
}

fn read_cdylib_dynamic_symbols(library_path: &PathBuf) -> BTreeSet<String> {
  let output = Command::new("nm")
    .arg("--dynamic")
    .arg("--defined-only")
    .arg("--extern-only")
    .arg("--format=posix")
    .arg(library_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to run nm for release cdylib");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "nm must succeed for release cdylib\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );

  parse_nm_dynamic_symbols(&stdout)
}

#[test]
fn golden_snapshot_matches_target_header_baseline() {
  let snapshot = fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
    .expect("failed to read golden abi snapshot");
  let (class, machine, _symbols) = parse_golden_snapshot(&snapshot);

  assert_eq!(class, "ELF64");
  assert_eq!(machine, "Advanced Micro Devices X86-64");
}

#[test]
fn golden_snapshot_symbols_match_version_script_global_exports() {
  let snapshot = fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
    .expect("failed to read golden abi snapshot");
  let (_class, _machine, golden_symbols) = parse_golden_snapshot(&snapshot);
  let version_script =
    fs::read_to_string(repository_file("rlibc.map")).expect("failed to read version script");
  let map_symbols = parse_global_symbols(&version_script);

  assert_eq!(golden_symbols, map_symbols);
}

#[test]
fn release_cdylib_dynamic_exports_match_golden_snapshot() {
  let snapshot = fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
    .expect("failed to read golden abi snapshot");
  let (_class, _machine, golden_symbols) = parse_golden_snapshot(&snapshot);
  let library_path = repository_file("target/release/librlibc.so");

  build_release_cdylib();

  let exported_symbols = read_cdylib_dynamic_symbols(&library_path);

  assert_eq!(
    exported_symbols, golden_symbols,
    "release cdylib dynamic exports must match ABI golden snapshot"
  );
}

#[test]
fn golden_snapshot_symbols_are_sorted_and_unique() {
  let snapshot = fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
    .expect("failed to read golden abi snapshot");
  let symbols_in_file = parse_golden_symbol_lines(&snapshot);
  let mut sorted_symbols = symbols_in_file.clone();
  let mut dedup_symbols = symbols_in_file.clone();

  sorted_symbols.sort_unstable();
  dedup_symbols.sort_unstable();
  dedup_symbols.dedup();

  assert_eq!(
    symbols_in_file, sorted_symbols,
    "golden snapshot symbols must be sorted for stable ABI review diffs"
  );
  assert_eq!(
    symbols_in_file, dedup_symbols,
    "golden snapshot symbols must not contain duplicates"
  );
}

#[test]
fn abi_check_binary_matches_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let library_path = repository_file("target/release/librlibc.so");

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg(&library_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must match golden snapshot\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_symbol_entries_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("duplicate-symbols");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("duplicate-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate-symbol golden snapshots\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate symbol entry"),
    "abi_check stderr must explain duplicate-symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbol_entries_with_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("whitespace-symbol");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy alias
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("whitespace-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject symbol entries containing whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid symbol entry with whitespace"),
    "abi_check stderr must explain whitespace-symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbol_entries_with_tab_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("tab-symbol");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy\talias
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("tab-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject symbol entries containing tab characters\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid symbol entry with whitespace"),
    "abi_check stderr must explain tab-symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbol_entries_with_surrounding_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("surrounding-whitespace-symbol");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
 memcpy
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("surrounding-whitespace-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject symbol entries with surrounding whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid symbol entry with surrounding whitespace"),
    "abi_check stderr must explain surrounding-whitespace symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbol_entries_with_trailing_space_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("trailing-space-symbol");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy 
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("trailing-space-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject symbol entries with trailing spaces\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid symbol entry with surrounding whitespace"),
    "abi_check stderr must explain trailing-space symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbol_entries_with_trailing_tab_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("trailing-tab-symbol");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy\t
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("trailing-tab-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject symbol entries with trailing tabs\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid symbol entry with surrounding whitespace"),
    "abi_check stderr must explain trailing-tab symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_empty_line_inside_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("empty-line-inside-symbols");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy

memmove
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("empty-line-inside-symbols golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject empty lines inside SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("empty line inside `SYMBOLS:` block"),
    "abi_check stderr must explain empty-line rejection inside SYMBOLS block\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_leading_empty_line_inside_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("leading-empty-line-inside-symbols");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:

memcpy
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("leading-empty-line-inside-symbols golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject leading empty lines inside SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("empty line inside `SYMBOLS:` block"),
    "abi_check stderr must explain leading empty-line rejection inside SYMBOLS block\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_trailing_whitespace_only_line_inside_symbols_block_in_golden_snapshot()
{
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("trailing-whitespace-only-line-inside-symbols");
  let snapshot_contents = concat!(
    "ABI_SNAPSHOT_V1\n",
    "ELF_CLASS=ELF64\n",
    "ELF_MACHINE=Advanced Micro Devices X86-64\n",
    "SYMBOLS:\n",
    "abort\n",
    " \t \n",
  );

  fs::write(&snapshot_path, snapshot_contents)
    .expect("trailing-whitespace-only-line-inside-symbols fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject trailing whitespace-only lines inside SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid trailing whitespace-only line inside `SYMBOLS:` block"),
    "abi_check stderr must explain trailing whitespace-only line rejection inside SYMBOLS block\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_accepts_trailing_empty_line_after_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let library_path = repository_file("target/release/librlibc.so");
  let snapshot_path = unique_temp_snapshot_path("trailing-empty-line-after-symbols");
  let mut snapshot_contents =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("failed to read golden snapshot fixture");

  snapshot_contents.push('\n');

  fs::write(&snapshot_path, snapshot_contents)
    .expect("trailing-empty-line-after-symbols golden snapshot fixture write must succeed");

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .arg(&library_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    output.status.success(),
    "abi_check must accept a trailing empty line after SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report golden snapshot match for trailing empty line case\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_multiple_trailing_empty_lines_after_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let library_path = repository_file("target/release/librlibc.so");
  let snapshot_path = unique_temp_snapshot_path("multiple-trailing-empty-lines-after-symbols");
  let mut snapshot_contents =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("failed to read golden snapshot fixture");

  snapshot_contents.push_str("\n\n");

  fs::write(&snapshot_path, snapshot_contents).expect(
    "multiple-trailing-empty-lines-after-symbols golden snapshot fixture write must succeed",
  );

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .arg(&library_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    output.status.success(),
    "abi_check must accept multiple trailing empty lines after SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report golden snapshot match for multiple trailing empty line case\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_rejects_symbol_entries_with_trailing_semicolon_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("semicolon-symbol");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy;
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("semicolon-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject symbol entries with trailing semicolons\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid symbol entry"),
    "abi_check stderr must explain semicolon-symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_version_suffixed_symbol_entries_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("versioned-symbol");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy@@GLIBC_2.2.5
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("versioned-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject version-suffixed symbol entries\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid symbol entry"),
    "abi_check stderr must explain version-suffixed symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbol_entries_starting_with_digit_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("digit-prefixed-symbol");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
9memcpy
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("digit-prefixed-symbol golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject symbol entries starting with digits\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid symbol entry"),
    "abi_check stderr must explain digit-prefixed symbol rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_metadata_lines_inside_symbol_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("metadata-in-symbols");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
ELF_MACHINE=AArch64
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("metadata-in-symbols golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject metadata lines inside SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("metadata field inside `SYMBOLS:` block"),
    "abi_check stderr must explain SYMBOLS-block metadata rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("duplicate-symbols-block");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
SYMBOLS:
memcpy
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("duplicate-symbols-block golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate SYMBOLS blocks\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate `SYMBOLS:` block"),
    "abi_check stderr must explain duplicate SYMBOLS block rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbols_line_with_leading_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("symbols-line-leading-whitespace");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
 SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("symbols-line-leading-whitespace golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject SYMBOLS line with leading whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid `SYMBOLS:` line with surrounding whitespace"),
    "abi_check stderr must explain leading-whitespace SYMBOLS line rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbols_line_with_trailing_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("symbols-line-trailing-whitespace");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS: 
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("symbols-line-trailing-whitespace golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject SYMBOLS line with trailing whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid `SYMBOLS:` line with surrounding whitespace"),
    "abi_check stderr must explain trailing-whitespace SYMBOLS line rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbols_line_with_leading_tab_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("symbols-line-leading-tab");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
\tSYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("symbols-line-leading-tab golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject SYMBOLS line with leading tab whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid `SYMBOLS:` line with surrounding whitespace"),
    "abi_check stderr must explain leading-tab SYMBOLS line rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_symbols_line_with_trailing_tab_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("symbols-line-trailing-tab");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:\t
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("symbols-line-trailing-tab golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject SYMBOLS line with trailing tab whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid `SYMBOLS:` line with surrounding whitespace"),
    "abi_check stderr must explain trailing-tab SYMBOLS line rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_accepts_symbols_line_with_carriage_return_suffix_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("symbols-line-carriage-return-suffix");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let snapshot_contents = base_snapshot.replacen("SYMBOLS:\n", "SYMBOLS:\r\n", 1);

  fs::write(&snapshot_path, snapshot_contents)
    .expect("symbols-line-carriage-return-suffix golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    output.status.success(),
    "abi_check must accept snapshots with CRLF-style SYMBOLS line endings\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching for CRLF SYMBOLS line endings\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_rejects_trailing_whitespace_only_line_after_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("symbols-trailing-whitespace-only-line");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let snapshot_contents = format!("{base_snapshot} \t \n");

  fs::write(&snapshot_path, snapshot_contents)
    .expect("symbols-trailing-whitespace-only-line golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject trailing whitespace-only lines inside SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("invalid trailing whitespace-only line inside `SYMBOLS:` block"),
    "abi_check stderr must explain trailing whitespace-only line rejection inside SYMBOLS block\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_missing_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("missing-symbols-block");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("missing-symbols-block golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject missing SYMBOLS block snapshots\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot missing `SYMBOLS:` block"),
    "abi_check stderr must explain missing SYMBOLS block rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_empty_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("empty-symbols-block");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("empty-symbols-block golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject empty SYMBOLS block snapshots\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has empty `SYMBOLS:` block"),
    "abi_check stderr must explain empty SYMBOLS block rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_class_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("duplicate-class-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("duplicate-class-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate ELF_CLASS field\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has duplicate `ELF_CLASS` field"),
    "abi_check stderr must explain duplicate ELF_CLASS rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_machine_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("duplicate-machine-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("duplicate-machine-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate ELF_MACHINE field\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has duplicate `ELF_MACHINE` field"),
    "abi_check stderr must explain duplicate ELF_MACHINE rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_missing_machine_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("missing-machine-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("missing-machine-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots missing ELF_MACHINE\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot missing `ELF_MACHINE` field"),
    "abi_check stderr must explain missing ELF_MACHINE rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_missing_class_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("missing-class-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("missing-class-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots missing ELF_CLASS\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot missing `ELF_CLASS` field"),
    "abi_check stderr must explain missing ELF_CLASS rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_empty_machine_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("empty-machine-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("empty-machine-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with empty ELF_MACHINE value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has empty `ELF_MACHINE` field"),
    "abi_check stderr must explain empty ELF_MACHINE rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_empty_class_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("empty-class-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("empty-class-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with empty ELF_CLASS value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has empty `ELF_CLASS` field"),
    "abi_check stderr must explain empty ELF_CLASS rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_only_machine_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("whitespace-only-machine-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=   \t
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("whitespace-only-machine-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with whitespace-only ELF_MACHINE value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has empty `ELF_MACHINE` field"),
    "abi_check stderr must explain whitespace-only ELF_MACHINE rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_tab_only_machine_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("tab-only-machine-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=\t
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("tab-only-machine-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with tab-only ELF_MACHINE value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has empty `ELF_MACHINE` field"),
    "abi_check stderr must explain tab-only ELF_MACHINE rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_machine_field_with_tab_character_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("machine-field-with-tab-character");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced\tMicro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("machine-field-with-tab-character golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with tab characters inside ELF_MACHINE value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid `ELF_MACHINE` field with non-space whitespace"),
    "abi_check stderr must explain tab character in ELF_MACHINE rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_only_class_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("whitespace-only-class-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=   \t
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("whitespace-only-class-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with whitespace-only ELF_CLASS value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has empty `ELF_CLASS` field"),
    "abi_check stderr must explain whitespace-only ELF_CLASS rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_class_field_with_leading_space_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("class-field-leading-space");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS= ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("class-field-leading-space golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with leading-space ELF_CLASS value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid `ELF_CLASS` field with leading whitespace"),
    "abi_check stderr must explain leading-space ELF_CLASS rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_class_line_with_leading_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("class-line-leading-whitespace");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
 ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("class-line-leading-whitespace golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with leading whitespace before ELF_CLASS key\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid `ELF_CLASS` line with leading whitespace"),
    "abi_check stderr must explain leading-whitespace ELF_CLASS line rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_class_field_with_trailing_space_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("class-field-trailing-space");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64 
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("class-field-trailing-space golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with trailing-space ELF_CLASS value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid `ELF_CLASS` field with trailing whitespace"),
    "abi_check stderr must explain trailing-space ELF_CLASS rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_machine_field_with_leading_space_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("machine-field-leading-space");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE= Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("machine-field-leading-space golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with leading-space ELF_MACHINE value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid `ELF_MACHINE` field with leading whitespace"),
    "abi_check stderr must explain leading-space ELF_MACHINE rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_machine_line_with_leading_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("machine-line-leading-whitespace");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
 ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("machine-line-leading-whitespace golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with leading whitespace before ELF_MACHINE key\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid `ELF_MACHINE` line with leading whitespace"),
    "abi_check stderr must explain leading-whitespace ELF_MACHINE line rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_machine_field_with_trailing_space_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("machine-field-trailing-space");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64 
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("machine-field-trailing-space golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with trailing-space ELF_MACHINE value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid `ELF_MACHINE` field with trailing whitespace"),
    "abi_check stderr must explain trailing-space ELF_MACHINE rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_class_field_with_internal_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("class-field-with-internal-whitespace");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF 64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("class-field-with-internal-whitespace golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with internal whitespace in ELF_CLASS value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid `ELF_CLASS` field with whitespace"),
    "abi_check stderr must explain internal whitespace in ELF_CLASS rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_class_field_with_space_before_equals_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("class-field-space-before-equals");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS =ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("class-field-space-before-equals golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject ELF_CLASS with space before '='\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has malformed `ELF_CLASS` field"),
    "abi_check stderr must explain malformed ELF_CLASS field rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_tab_only_class_field_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("tab-only-class-field");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=\t
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("tab-only-class-field golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots with tab-only ELF_CLASS value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has empty `ELF_CLASS` field"),
    "abi_check stderr must explain tab-only ELF_CLASS rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_machine_field_with_space_before_equals_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("machine-field-space-before-equals");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE =Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("machine-field-space-before-equals golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject ELF_MACHINE with space before '='\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has malformed `ELF_MACHINE` field"),
    "abi_check stderr must explain malformed ELF_MACHINE field rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_golden_flags() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg("--golden")
    .arg(&golden_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate --golden flags\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate --golden arguments are not supported"),
    "abi_check stderr must mention duplicate --golden rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_golden_mixed_forms() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let golden_equals_arg = format!("--golden={}", golden_path.display());
  let output = Command::new(&abi_check_path)
    .arg(golden_equals_arg)
    .arg("--golden")
    .arg(&golden_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate --golden arguments in mixed forms\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate --golden arguments are not supported"),
    "abi_check stderr must mention duplicate --golden rejection for mixed forms\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_golden_mixed_forms_reverse_order() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let golden_equals_arg = format!("--golden={}", golden_path.display());
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg(golden_equals_arg)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate --golden arguments in mixed reverse order\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate --golden arguments are not supported"),
    "abi_check stderr must mention duplicate --golden rejection for mixed reverse order\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_golden_equals_forms() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let first_golden_equals_arg = format!("--golden={}", golden_path.display());
  let output = Command::new(&abi_check_path)
    .arg(first_golden_equals_arg)
    .arg("--golden=--bogus")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate --golden=<...> arguments\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate --golden arguments are not supported"),
    "abi_check stderr must mention duplicate --golden rejection for equals forms\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_before_empty_equals_value_validation() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let first_golden_equals_arg = format!("--golden={}", golden_path.display());
  let output = Command::new(&abi_check_path)
    .arg(first_golden_equals_arg)
    .arg("--golden=")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate --golden before validating second empty equals value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate --golden arguments are not supported"),
    "abi_check stderr must prioritize duplicate --golden rejection over empty value validation\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_before_missing_value_validation_for_help_token() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg("--golden")
    .arg("--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate --golden before validating second missing value token\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate --golden arguments are not supported"),
    "abi_check stderr must prioritize duplicate --golden rejection over second --golden value validation\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_before_missing_value_validation_for_option_like_token() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg("--golden")
    .arg("--bogus")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate --golden before validating second option-like token\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate --golden arguments are not supported"),
    "abi_check stderr must prioritize duplicate --golden rejection over second option-like value validation\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_duplicate_before_whitespace_value_validation() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg("--golden")
    .arg("   ")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject duplicate --golden before validating second whitespace-only token\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("duplicate --golden arguments are not supported"),
    "abi_check stderr must prioritize duplicate --golden rejection over second whitespace-only value validation\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_unexpected_snapshot_line_before_symbols_block() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("unexpected-pre-symbol-line");
  let snapshot_contents = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
ELF_DATA=2's complement, little endian
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("unexpected-pre-symbol-line golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject unexpected metadata lines before SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("unexpected snapshot line before symbol block"),
    "abi_check stderr must explain unexpected pre-symbol metadata rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_missing_magic_header_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("missing-magic-header");
  let snapshot_contents = "\
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
abort
";

  fs::write(&snapshot_path, snapshot_contents)
    .expect("missing-magic-header golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots missing magic header\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot missing magic header"),
    "abi_check stderr must explain missing magic header rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_magic_header_with_leading_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("magic-header-leading-whitespace");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let suffix = base_snapshot
    .strip_prefix("ABI_SNAPSHOT_V1")
    .expect("reference golden snapshot must start with ABI_SNAPSHOT_V1");
  let snapshot_contents = format!(" ABI_SNAPSHOT_V1{suffix}");

  fs::write(&snapshot_path, snapshot_contents)
    .expect("magic-header-leading-whitespace golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots whose magic header has leading whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid magic header with surrounding whitespace"),
    "abi_check stderr must explain leading-whitespace magic header rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_magic_header_with_trailing_whitespace_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("magic-header-trailing-whitespace");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let suffix = base_snapshot
    .strip_prefix("ABI_SNAPSHOT_V1")
    .expect("reference golden snapshot must start with ABI_SNAPSHOT_V1");
  let snapshot_contents = format!("ABI_SNAPSHOT_V1 {suffix}");

  fs::write(&snapshot_path, snapshot_contents)
    .expect("magic-header-trailing-whitespace golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots whose magic header has trailing whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid magic header with surrounding whitespace"),
    "abi_check stderr must explain trailing-whitespace magic header rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_magic_header_with_leading_tab_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("magic-header-leading-tab");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let suffix = base_snapshot
    .strip_prefix("ABI_SNAPSHOT_V1")
    .expect("reference golden snapshot must start with ABI_SNAPSHOT_V1");
  let snapshot_contents = format!("\tABI_SNAPSHOT_V1{suffix}");

  fs::write(&snapshot_path, snapshot_contents)
    .expect("magic-header-leading-tab golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots whose magic header has leading tab whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid magic header with surrounding whitespace"),
    "abi_check stderr must explain leading-tab magic header rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_magic_header_with_trailing_tab_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("magic-header-trailing-tab");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let suffix = base_snapshot
    .strip_prefix("ABI_SNAPSHOT_V1")
    .expect("reference golden snapshot must start with ABI_SNAPSHOT_V1");
  let snapshot_contents = format!("ABI_SNAPSHOT_V1\t{suffix}");

  fs::write(&snapshot_path, snapshot_contents)
    .expect("magic-header-trailing-tab golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject snapshots whose magic header has trailing tab whitespace\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("snapshot has invalid magic header with surrounding whitespace"),
    "abi_check stderr must explain trailing-tab magic header rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_accepts_magic_header_with_carriage_return_suffix_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("magic-header-carriage-return-suffix");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let suffix = base_snapshot
    .strip_prefix("ABI_SNAPSHOT_V1")
    .expect("reference golden snapshot must start with ABI_SNAPSHOT_V1");
  let snapshot_contents = format!("ABI_SNAPSHOT_V1\r{suffix}");

  fs::write(&snapshot_path, snapshot_contents)
    .expect("magic-header-carriage-return-suffix golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    output.status.success(),
    "abi_check must accept snapshots with CRLF-style magic-header line endings\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching for CRLF header line endings\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_snapshot_with_all_crlf_line_endings() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("full-crlf-line-endings");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let snapshot_contents = base_snapshot.replace('\n', "\r\n");

  fs::write(&snapshot_path, snapshot_contents)
    .expect("full-crlf-line-endings golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    output.status.success(),
    "abi_check must accept snapshots with full CRLF line endings\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching for full CRLF snapshot\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_class_line_with_crlf_line_endings_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("class-line-crlf-line-endings");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let snapshot_contents = base_snapshot.replacen("ELF_CLASS=ELF64\n", "ELF_CLASS=ELF64\r\n", 1);

  fs::write(&snapshot_path, snapshot_contents)
    .expect("class-line-crlf-line-endings golden snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    output.status.success(),
    "abi_check must accept snapshots with CRLF ELF_CLASS metadata line endings\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching for CRLF ELF_CLASS metadata line ending\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_trailing_crlf_empty_line_after_symbols_block_in_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("trailing-crlf-empty-line-after-symbols-block");
  let base_snapshot =
    fs::read_to_string(repository_file("abi/golden/x86_64-unknown-linux-gnu.abi"))
      .expect("reference golden snapshot must be readable");
  let mut snapshot_contents = base_snapshot.replace('\n', "\r\n");

  snapshot_contents.push_str("\r\n");

  fs::write(&snapshot_path, snapshot_contents).expect(
    "trailing-crlf-empty-line-after-symbols-block golden snapshot fixture write must succeed",
  );

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    output.status.success(),
    "abi_check must accept trailing CRLF empty line after SYMBOLS block\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching for trailing CRLF empty line fixture\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_rejects_empty_golden_snapshot() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let snapshot_path = unique_temp_snapshot_path("empty-golden-snapshot");

  fs::write(&snapshot_path, "").expect("empty-golden-snapshot fixture write must succeed");

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&snapshot_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  let _ = fs::remove_file(&snapshot_path);

  assert!(
    !output.status.success(),
    "abi_check must reject empty golden snapshots\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("golden snapshot is empty"),
    "abi_check stderr must explain empty golden snapshot rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_unknown_cli_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--bogus")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject unknown CLI arguments\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("unknown argument: --bogus"),
    "abi_check stderr must explain unknown argument rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_missing_golden_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden without a path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain missing --golden path rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_help_token_used_as_golden_path_value() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --help used as a --golden value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain --golden value rejection for --help\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_short_help_token_used_as_golden_path_value() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("-h")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject -h used as a --golden value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain --golden value rejection for -h\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_option_like_token_used_as_golden_path_value() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("--bogus")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject option-like tokens used as --golden values\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain --golden value rejection for option-like tokens\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_prefixed_option_like_golden_path_value() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("   --bogus")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject whitespace-prefixed option-like token used as --golden value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain rejection for whitespace-prefixed option-like --golden values\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_treats_whitespace_prefixed_option_like_golden_equals_value_as_path() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=   --bogus")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must treat whitespace-prefixed option-like --golden=<...> value as a golden path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("failed to read golden snapshot"),
    "abi_check stderr must explain file-read failure for whitespace-prefixed option-like --golden=<...> values\nstderr:\n{stderr}",
  );
  assert!(
    stderr.contains("--bogus"),
    "abi_check stderr must include the unresolved whitespace-prefixed option-like path token\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_prefixed_option_like_golden_path_value_before_help() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("   --bogus")
    .arg("--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject whitespace-prefixed option-like token used as --golden value before --help\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain rejection for whitespace-prefixed option-like --golden values\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_prefixed_option_like_golden_path_value_before_short_help() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("   --bogus")
    .arg("-h")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject whitespace-prefixed option-like token used as --golden value before -h\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain rejection for whitespace-prefixed option-like --golden values before -h\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_treats_whitespace_prefixed_option_like_golden_equals_value_as_golden_path_before_help()
 {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=   --bogus")
    .arg("--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must not treat --help as help when whitespace-prefixed option-like --golden=<...> is present\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("unknown argument: --help"),
    "abi_check stderr must report --help as an unknown positional argument in this parse mode\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_treats_whitespace_prefixed_option_like_golden_equals_value_as_golden_path_before_short_help()
 {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=   --bogus")
    .arg("-h")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must not treat -h as help when whitespace-prefixed option-like --golden=<...> is present\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("unknown argument: -h"),
    "abi_check stderr must report -h as an unknown positional argument in this parse mode\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_treats_dash_prefixed_token_as_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("--bogus")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must treat dash-prefixed token after -- as a library path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path does not exist"),
    "abi_check stderr must report missing library path after -- positional parsing\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_treats_help_token_as_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must treat --help after -- as a library path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path does not exist"),
    "abi_check stderr must report missing library path for --help after --\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_treats_short_help_token_as_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("-h")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must treat -h after -- as a library path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path does not exist"),
    "abi_check stderr must report missing library path for -h after --\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_treats_golden_equals_token_as_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("--golden=abi/golden/x86_64-unknown-linux-gnu.abi")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must treat --golden=<path> after -- as a library path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path does not exist"),
    "abi_check stderr must report missing library path for --golden=<path> after --\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_treats_golden_flag_token_as_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("--golden")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must treat --golden after -- as a library path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path does not exist"),
    "abi_check stderr must report missing library path for --golden after --\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_accepts_double_dash_without_library_path() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg("--")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must accept `--` without an explicit library path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("ABI check passed."),
    "abi_check stdout must report successful ABI check for `--` only invocation\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_rejects_empty_library_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject empty library path arguments\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain empty library path rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_only_library_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("   ")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject whitespace-only library path arguments\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain whitespace-only library path rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_tab_only_library_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("\t\t")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject tab-only library path arguments\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain tab-only library path rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_newline_only_library_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("\n")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject newline-only library path arguments\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain newline-only library path rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_carriage_return_only_library_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("\r")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject carriage-return-only library path arguments\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain carriage-return-only library path rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_only_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("   ")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject whitespace-only library path arguments after --\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain whitespace-only library path rejection after --\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_tab_only_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("\t\t")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject tab-only library path arguments after --\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain tab-only library path rejection after --\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_newline_only_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("\n")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject newline-only library path arguments after --\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain newline-only library path rejection after --\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_carriage_return_only_library_path_after_double_dash() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--")
    .arg("\r")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject carriage-return-only library path arguments after --\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("library path must not be empty"),
    "abi_check stderr must explain carriage-return-only library path rejection after --\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_accepts_golden_equals_syntax() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let golden_arg = format!("--golden={}", golden_path.display());

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg(golden_arg)
    .arg(repository_file("target/release/librlibc.so"))
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must accept --golden=<path> syntax\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_library_path_before_golden_flag() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let library_path = repository_file("target/release/librlibc.so");

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg(&library_path)
    .arg("--golden")
    .arg(&golden_path)
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must accept <library> before --golden <path>\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching when library path precedes --golden\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_golden_flag_before_double_dash_library_path() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg("--")
    .arg(repository_file("target/release/librlibc.so"))
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must accept --golden <path> followed by -- <library>\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching when -- is used\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_golden_equals_before_double_dash_library_path() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let golden_arg = format!("--golden={}", golden_path.display());

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg(golden_arg)
    .arg("--")
    .arg(repository_file("target/release/librlibc.so"))
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must accept --golden=<path> followed by -- <library>\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching for --golden=<path> + -- usage\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_golden_flag_before_double_dash_without_library_path() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg("--")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must accept --golden <path> followed by trailing --\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching when trailing -- is used\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_accepts_golden_equals_before_double_dash_without_library_path() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let golden_arg = format!("--golden={}", golden_path.display());

  build_release_cdylib();

  let output = Command::new(&abi_check_path)
    .arg(golden_arg)
    .arg("--")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must accept --golden=<path> followed by trailing --\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Golden ABI snapshot matched"),
    "abi_check stdout must report successful golden snapshot matching for --golden=<path> + trailing --\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_rejects_empty_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden= without a path\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain empty --golden= rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_short_help_after_empty_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=")
    .arg("-h")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject -h after empty --golden=\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must prioritize empty --golden= rejection over later -h\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_long_help_after_empty_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=")
    .arg("--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --help after empty --golden=\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must prioritize empty --golden= rejection over later --help\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_option_like_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=--bogus")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden= with option-like value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("failed to read golden snapshot"),
    "abi_check stderr must explain option-like --golden= file-read failure\nstderr:\n{stderr}",
  );
  assert!(
    stderr.contains("/--bogus"),
    "abi_check stderr must include the resolved --golden=--bogus path\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_help_token_used_as_golden_equals_value() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden=--help\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("failed to read golden snapshot"),
    "abi_check stderr must explain --golden=--help file-read failure\nstderr:\n{stderr}",
  );
  assert!(
    stderr.contains("/--help"),
    "abi_check stderr must include the resolved --golden=--help path\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_short_help_token_used_as_golden_equals_value() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=-h")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden=-h\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("failed to read golden snapshot"),
    "abi_check stderr must explain --golden=-h file-read failure\nstderr:\n{stderr}",
  );
  assert!(
    stderr.contains("/-h"),
    "abi_check stderr must include the resolved --golden=-h path\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_shows_usage_for_help_after_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let golden_arg = format!("--golden={}", golden_path.display());
  let output = Command::new(&abi_check_path)
    .arg(golden_arg)
    .arg("--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must return success for --help even when --golden=<path> is present\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Usage: cargo run --release --bin abi_check --"),
    "abi_check stdout must include usage text for --help with --golden=<path>\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_shows_usage_for_short_help_after_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let golden_arg = format!("--golden={}", golden_path.display());
  let output = Command::new(&abi_check_path)
    .arg(golden_arg)
    .arg("-h")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must return success for -h even when --golden=<path> is present\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Usage: cargo run --release --bin abi_check --"),
    "abi_check stdout must include usage text for -h with --golden=<path>\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_shows_usage_for_short_help_after_golden_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg("-h")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must return success for -h even when --golden <path> is present\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Usage: cargo run --release --bin abi_check --"),
    "abi_check stdout must include usage text for -h with --golden <path>\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_shows_usage_for_help_after_golden_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let golden_path = repository_file("abi/golden/x86_64-unknown-linux-gnu.abi");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg(&golden_path)
    .arg("--help")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    output.status.success(),
    "abi_check must return success for --help even when --golden <path> is present\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stdout.contains("Usage: cargo run --release --bin abi_check --"),
    "abi_check stdout must include usage text for --help with --golden <path>\nstdout:\n{stdout}",
  );
}

#[test]
fn abi_check_binary_rejects_double_dash_used_as_golden_equals_value() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=--")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden=--\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("failed to read golden snapshot"),
    "abi_check stderr must explain --golden=-- file-read failure\nstderr:\n{stderr}",
  );
  assert!(
    stderr.contains("/--"),
    "abi_check stderr must include the resolved --golden=-- path\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_only_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=   ")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden= with whitespace-only value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain whitespace-only --golden= rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_tab_only_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=\t\t")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden= with tab-only value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain tab-only --golden= rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_newline_only_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=\n")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden= with newline-only value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain newline-only --golden= rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_carriage_return_only_golden_equals_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden=\r")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden= with carriage-return-only value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain carriage-return-only --golden= rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_whitespace_only_golden_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("   ")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden with whitespace-only value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain whitespace-only --golden value rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_tab_only_golden_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("\t\t")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden with tab-only value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain tab-only --golden value rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_newline_only_golden_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("\n")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden with newline-only value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain newline-only --golden value rejection\nstderr:\n{stderr}",
  );
}

#[test]
fn abi_check_binary_rejects_carriage_return_only_golden_path_argument() {
  let abi_check_path = std::env::var_os("CARGO_BIN_EXE_abi_check")
    .map(PathBuf::from)
    .expect("cargo must provide CARGO_BIN_EXE_abi_check for integration tests");
  let output = Command::new(&abi_check_path)
    .arg("--golden")
    .arg("\r")
    .current_dir(repository_file("."))
    .output()
    .expect("failed to execute abi_check binary");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(
    !output.status.success(),
    "abi_check must reject --golden with carriage-return-only value\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
    output.status,
    stdout,
    stderr,
  );
  assert!(
    stderr.contains("missing value for --golden"),
    "abi_check stderr must explain carriage-return-only --golden value rejection\nstderr:\n{stderr}",
  );
}
