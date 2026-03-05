use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};
use std::{env, fs};

const DEFAULT_LIBRARY_PATH: &str = "target/release/librlibc.so";
const EXPECTED_ELF_CLASS: &str = "ELF64";
const EXPECTED_MACHINE: &str = "Advanced Micro Devices X86-64";
const SNAPSHOT_MAGIC: &str = "ABI_SNAPSHOT_V1";
const GOLDEN_FLAG: &str = "--golden";
const REQUIRED_SYMBOLS: &[&str] = &[
  "__errno_location",
  "_Exit",
  "abort",
  "atexit",
  "atoi",
  "atol",
  "atoll",
  "clearenv",
  "dlopen",
  "dlsym",
  "environ",
  "exit",
  "getenv",
  "memcmp",
  "memcpy",
  "memmove",
  "memset",
  "putenv",
  "setenv",
  "strlen",
  "strnlen",
  "strtol",
  "strtoll",
  "strtoul",
  "strtoull",
  "unsetenv",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ElfHeader {
  class: String,
  machine: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliOptions {
  library_path: Option<String>,
  golden_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AbiSnapshot {
  class: String,
  machine: String,
  symbols: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotDiff {
  class_mismatch: Option<(String, String)>,
  machine_mismatch: Option<(String, String)>,
  missing_symbols: Vec<String>,
  unexpected_symbols: Vec<String>,
}

impl SnapshotDiff {
  const fn is_empty(&self) -> bool {
    self.class_mismatch.is_none()
      && self.machine_mismatch.is_none()
      && self.missing_symbols.is_empty()
      && self.unexpected_symbols.is_empty()
  }
}

fn main() -> ExitCode {
  let args: Vec<String> = env::args().skip(1).collect();

  match run(&args) {
    Ok(()) => ExitCode::SUCCESS,
    Err(message) => {
      eprintln!("{message}");
      ExitCode::FAILURE
    }
  }
}

fn run(args: &[String]) -> Result<(), String> {
  if has_help_flag(args) {
    print_usage();

    return Ok(());
  }

  let options = parse_cli_options(args)?;
  let library_args = options
    .library_path
    .as_ref()
    .map_or_else(Vec::new, |path| vec![path.clone()]);
  let library_path = resolve_library_path(&library_args)?;
  let uses_default_library_path =
    should_rebuild_default_library_path(library_path, library_args.is_empty());

  ensure_library_exists(library_path, uses_default_library_path)?;

  let absolute_path = normalize_library_path(library_path);
  let header_output = run_command("readelf", &["--file-header", library_path])?;
  let nm_output = run_command(
    "nm",
    &[
      "--dynamic",
      "--defined-only",
      "--extern-only",
      "--format=posix",
      library_path,
    ],
  )?;
  let header = parse_readelf_header(&header_output)?;

  validate_elf_header(&header)?;

  let symbols = parse_nm_symbols(&nm_output);
  let missing_symbols = missing_required_symbols(&symbols);
  let snapshot = build_snapshot(&header, &symbols);

  println!("ABI check target: {absolute_path}");
  println!("ELF class: {}", header.class);
  println!("Machine: {}", header.machine);
  println!("Exported symbols: {}", symbols.len());

  if !missing_symbols.is_empty() {
    println!("Missing required symbols:");

    for symbol in &missing_symbols {
      println!("  - {symbol}");
    }

    return Err("ABI check failed".to_string());
  }

  if let Some(golden_path) = options.golden_path.as_deref() {
    let golden_contents = fs::read_to_string(golden_path).map_err(|error| {
      format!(
        "failed to read golden snapshot {}: {error}",
        normalize_library_path(golden_path),
      )
    })?;
    let golden_snapshot = parse_snapshot(&golden_contents)?;
    let diff = diff_against_golden(&golden_snapshot, &snapshot);

    if !diff.is_empty() {
      println!(
        "ABI golden diff detected against {}:",
        normalize_library_path(golden_path)
      );
      print_snapshot_diff(&diff);

      return Err("ABI check failed".to_string());
    }

    println!(
      "Golden ABI snapshot matched: {}",
      normalize_library_path(golden_path)
    );
  }

  println!("ABI check passed.");

  Ok(())
}

fn has_help_flag(args: &[String]) -> bool {
  let mut index = 0;

  while index < args.len() {
    let arg = args
      .get(index)
      .expect("index is bounds-checked by while condition");

    if arg == "--" {
      return false;
    }

    if arg == "-h" || arg == "--help" {
      return true;
    }

    if let Some(value) = arg.strip_prefix("--golden=") {
      if is_missing_option_value(value) || is_option_like_value(value) {
        return false;
      }

      index += 1;

      continue;
    }

    if arg == GOLDEN_FLAG {
      // `--golden` consumes the next token as its value; it must not be
      // interpreted as an unrelated help flag.
      let value_index = index + 1;
      let Some(value) = args.get(value_index) else {
        return false;
      };

      if is_missing_option_value(value) || is_option_like_value(value) {
        return false;
      }

      index = value_index + 1;

      continue;
    }

    index += 1;
  }

  false
}

fn print_usage() {
  println!(
    "Usage: cargo run --release --bin abi_check -- [{GOLDEN_FLAG} PATH_TO_GOLDEN] [PATH_TO_LIB]"
  );
  println!("If no library path is given, {DEFAULT_LIBRARY_PATH} is checked.");
}

fn parse_cli_options(args: &[String]) -> Result<CliOptions, String> {
  let mut options = CliOptions {
    library_path: None,
    golden_path: None,
  };
  let mut positional_only = false;
  let mut index = 0;

  while index < args.len() {
    let arg = args
      .get(index)
      .ok_or_else(|| "failed to read argument".to_string())?;

    if !positional_only {
      if arg == "--" {
        positional_only = true;
        index += 1;

        continue;
      }

      if let Some(path) = arg.strip_prefix("--golden=") {
        assign_golden_path(&mut options, path, true)?;
        index += 1;

        continue;
      }

      if arg == GOLDEN_FLAG {
        let value_index = index + 1;
        let path = args
          .get(value_index)
          .ok_or_else(|| format!("missing value for {GOLDEN_FLAG}"))?;

        assign_golden_path(&mut options, path, false)?;
        index = value_index + 1;

        continue;
      }

      if arg.starts_with('-') {
        return Err(format!("unknown argument: {arg}"));
      }
    }

    if arg.trim().is_empty() {
      return Err("library path must not be empty".to_string());
    }

    if options.library_path.is_some() {
      return Err("expected at most one positional argument for library path".to_string());
    }

    options.library_path = Some(arg.clone());
    index += 1;
  }

  Ok(options)
}

fn is_missing_option_value(value: &str) -> bool {
  value.trim().is_empty()
}

fn is_option_like_value(value: &str) -> bool {
  value.trim_start().starts_with('-')
}

fn has_leading_whitespace(value: &str) -> bool {
  value.chars().next().is_some_and(char::is_whitespace)
}

fn assign_golden_path(
  options: &mut CliOptions,
  path: &str,
  allow_dash_prefix: bool,
) -> Result<(), String> {
  if options.golden_path.is_some() {
    return Err(format!(
      "duplicate {GOLDEN_FLAG} arguments are not supported"
    ));
  }

  let option_like = is_option_like_value(path);
  let invalid_dash_prefixed_path = (!allow_dash_prefix && option_like)
    || (allow_dash_prefix && option_like && has_leading_whitespace(path));

  if is_missing_option_value(path) || invalid_dash_prefixed_path {
    return Err(format!("missing value for {GOLDEN_FLAG}"));
  }

  options.golden_path = Some(path.to_string());

  Ok(())
}

fn resolve_library_path(args: &[String]) -> Result<&str, String> {
  match args {
    [] => Ok(DEFAULT_LIBRARY_PATH),
    [path] => Ok(path),
    _ => Err("expected at most one positional argument for library path".to_string()),
  }
}

fn should_rebuild_default_library_path(
  resolved_library_path: &str,
  path_argument_is_omitted: bool,
) -> bool {
  if path_argument_is_omitted || resolved_library_path == DEFAULT_LIBRARY_PATH {
    return true;
  }

  normalize_library_path_for_comparison(resolved_library_path)
    == normalize_library_path_for_comparison(DEFAULT_LIBRARY_PATH)
}

fn ensure_library_exists(path: &str, uses_default_path: bool) -> Result<(), String> {
  ensure_library_exists_with(path, uses_default_path, build_default_cdylib)
}

fn ensure_library_exists_with<F>(
  path: &str,
  uses_default_path: bool,
  mut build_cdylib: F,
) -> Result<(), String>
where
  F: FnMut() -> Result<(), String>,
{
  let library_path = Path::new(path);

  if library_path.is_file() {
    return Ok(());
  }

  if uses_default_path {
    // Build only when the default path is expected and the library is currently missing.
    build_cdylib()?;

    if library_path.is_file() {
      return Ok(());
    }

    if library_path.exists() {
      return Err(format!(
        "default library path is not a regular file after cdylib build: {}",
        normalize_library_path(path)
      ));
    }

    return Err(format!(
      "default library path missing after cdylib build: {}",
      normalize_library_path(path)
    ));
  }

  if library_path.exists() {
    return Err(format!(
      "library path is not a regular file: {}",
      normalize_library_path(path)
    ));
  }

  Err(format!(
    "library path does not exist: {}",
    normalize_library_path(path)
  ))
}

fn build_default_cdylib() -> Result<(), String> {
  let args = ["rustc", "--release", "--lib", "--crate-type", "cdylib"];
  let output = Command::new("cargo")
    .args(args)
    .output()
    .map_err(|error| format!("failed to execute `cargo` with args {args:?}: {error}"))?;

  if output.status.success() {
    return Ok(());
  }

  Err(format!(
    "`cargo` failed with status {} while building {}: {}",
    output.status,
    DEFAULT_LIBRARY_PATH,
    String::from_utf8_lossy(&output.stderr),
  ))
}

fn run_command(program: &str, args: &[&str]) -> Result<String, String> {
  let output = Command::new(program)
    .args(args)
    .output()
    .map_err(|error| format!("failed to execute `{program}` with args {args:?}: {error}"))?;

  if !output.status.success() {
    return Err(format!(
      "`{program}` failed with status {}: {}",
      output.status,
      String::from_utf8_lossy(&output.stderr),
    ));
  }

  String::from_utf8(output.stdout)
    .map_err(|error| format!("`{program}` produced non-UTF8 output: {error}"))
}

fn parse_readelf_header(output: &str) -> Result<ElfHeader, String> {
  let class = find_readelf_field(output, "Class")
    .ok_or_else(|| "readelf output missing `Class` field".to_string())?;
  let machine = find_readelf_field(output, "Machine")
    .ok_or_else(|| "readelf output missing `Machine` field".to_string())?;

  Ok(ElfHeader { class, machine })
}

fn parse_nm_symbols(output: &str) -> BTreeSet<String> {
  output
    .lines()
    .filter_map(|line| {
      line.split_whitespace().next().and_then(|symbol| {
        if symbol.ends_with(':') {
          return None;
        }

        let base = symbol.split('@').next().unwrap_or(symbol);

        if base.is_empty() {
          return None;
        }

        Some(base.to_string())
      })
    })
    .collect()
}

fn find_readelf_field(output: &str, field: &str) -> Option<String> {
  let prefix = format!("{field}:");

  output.lines().find_map(|line| {
    line
      .trim_start()
      .strip_prefix(&prefix)
      .map(str::trim)
      .map(str::to_string)
  })
}

fn validate_elf_header(header: &ElfHeader) -> Result<(), String> {
  if header.class != EXPECTED_ELF_CLASS {
    return Err(format!(
      "unexpected ELF class: expected {EXPECTED_ELF_CLASS}, found {}",
      header.class,
    ));
  }

  if header.machine != EXPECTED_MACHINE {
    return Err(format!(
      "unexpected ELF machine: expected {EXPECTED_MACHINE}, found {}",
      header.machine,
    ));
  }

  Ok(())
}

fn missing_required_symbols(symbols: &BTreeSet<String>) -> Vec<&'static str> {
  REQUIRED_SYMBOLS
    .iter()
    .copied()
    .filter(|symbol| !symbols.contains(*symbol))
    .collect()
}

fn build_snapshot(header: &ElfHeader, symbols: &BTreeSet<String>) -> AbiSnapshot {
  AbiSnapshot {
    class: header.class.clone(),
    machine: header.machine.clone(),
    symbols: symbols.clone(),
  }
}

#[cfg(test)]
fn format_snapshot(snapshot: &AbiSnapshot) -> String {
  use std::fmt::Write as _;

  let mut formatted = String::new();

  formatted.push_str(SNAPSHOT_MAGIC);
  formatted.push('\n');
  writeln!(formatted, "ELF_CLASS={}", snapshot.class).expect("writing to String must not fail");
  writeln!(formatted, "ELF_MACHINE={}", snapshot.machine).expect("writing to String must not fail");
  formatted.push_str("SYMBOLS:\n");

  for symbol in &snapshot.symbols {
    formatted.push_str(symbol);
    formatted.push('\n');
  }

  formatted
}

fn parse_snapshot(contents: &str) -> Result<AbiSnapshot, String> {
  let mut lines = contents.lines();
  let Some(first_line) = lines.next() else {
    return Err("golden snapshot is empty".to_string());
  };

  if first_line.trim() == SNAPSHOT_MAGIC && first_line != SNAPSHOT_MAGIC {
    return Err("snapshot has invalid magic header with surrounding whitespace".to_string());
  }

  if first_line != SNAPSHOT_MAGIC {
    return Err(format!("snapshot missing magic header `{SNAPSHOT_MAGIC}`"));
  }

  let mut class: Option<String> = None;
  let mut machine: Option<String> = None;
  let mut symbols = BTreeSet::new();
  let mut in_symbol_block = false;

  while let Some(raw_line) = lines.next() {
    let line = raw_line.trim();

    if in_symbol_block && line.is_empty() {
      let remaining_lines_are_blank = lines
        .clone()
        .all(|remaining_line| remaining_line.trim().is_empty());

      if remaining_lines_are_blank && raw_line.is_empty() && lines.clone().all(str::is_empty) {
        break;
      }

      if remaining_lines_are_blank {
        return Err(
          "snapshot has invalid trailing whitespace-only line inside `SYMBOLS:` block".to_string(),
        );
      }

      return Err("snapshot has empty line inside `SYMBOLS:` block".to_string());
    }

    if line.is_empty() {
      continue;
    }

    if line == "SYMBOLS:" {
      if in_symbol_block {
        return Err("snapshot has duplicate `SYMBOLS:` block".to_string());
      }

      if raw_line != line {
        return Err("snapshot has invalid `SYMBOLS:` line with surrounding whitespace".to_string());
      }

      in_symbol_block = true;

      continue;
    }

    if in_symbol_block {
      if line.starts_with("ELF_CLASS=") || line.starts_with("ELF_MACHINE=") {
        return Err(format!(
          "snapshot has metadata field inside `SYMBOLS:` block: {line}"
        ));
      }

      if raw_line != line {
        return Err(format!(
          "snapshot has invalid symbol entry with surrounding whitespace: {raw_line}"
        ));
      }

      if line.chars().any(char::is_whitespace) {
        return Err(format!(
          "snapshot has invalid symbol entry with whitespace: {line}"
        ));
      }

      if !is_valid_snapshot_symbol(line) {
        return Err(format!("snapshot has invalid symbol entry: {line}"));
      }

      if !symbols.insert(line.to_string()) {
        return Err(format!("snapshot has duplicate symbol entry: {line}"));
      }

      continue;
    }

    if let Some(raw_value) = line.strip_prefix("ELF_CLASS=") {
      if class.is_some() {
        return Err("snapshot has duplicate `ELF_CLASS` field".to_string());
      }

      if raw_line.chars().next().is_some_and(char::is_whitespace) {
        return Err("snapshot has invalid `ELF_CLASS` line with leading whitespace".to_string());
      }

      let raw_line_value = raw_line.strip_prefix("ELF_CLASS=").unwrap_or(raw_value);
      let value = raw_line_value.trim();

      if value.is_empty() {
        return Err("snapshot has empty `ELF_CLASS` field".to_string());
      }

      if raw_line_value
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
      {
        return Err("snapshot has invalid `ELF_CLASS` field with leading whitespace".to_string());
      }

      if raw_line_value
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
      {
        return Err("snapshot has invalid `ELF_CLASS` field with trailing whitespace".to_string());
      }

      if value.chars().any(char::is_whitespace) {
        return Err("snapshot has invalid `ELF_CLASS` field with whitespace".to_string());
      }

      class = Some(value.to_string());

      continue;
    }

    if let Some(raw_value) = line.strip_prefix("ELF_MACHINE=") {
      if machine.is_some() {
        return Err("snapshot has duplicate `ELF_MACHINE` field".to_string());
      }

      if raw_line.chars().next().is_some_and(char::is_whitespace) {
        return Err("snapshot has invalid `ELF_MACHINE` line with leading whitespace".to_string());
      }

      let raw_line_value = raw_line.strip_prefix("ELF_MACHINE=").unwrap_or(raw_value);
      let value = raw_line_value.trim();

      if value.is_empty() {
        return Err("snapshot has empty `ELF_MACHINE` field".to_string());
      }

      if raw_line_value
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
      {
        return Err("snapshot has invalid `ELF_MACHINE` field with leading whitespace".to_string());
      }

      if raw_line_value
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
      {
        return Err(
          "snapshot has invalid `ELF_MACHINE` field with trailing whitespace".to_string(),
        );
      }

      if value.chars().any(|ch| ch.is_whitespace() && ch != ' ') {
        return Err(
          "snapshot has invalid `ELF_MACHINE` field with non-space whitespace".to_string(),
        );
      }

      machine = Some(value.to_string());

      continue;
    }

    if line.starts_with("ELF_CLASS") {
      return Err(format!("snapshot has malformed `ELF_CLASS` field: {line}"));
    }

    if line.starts_with("ELF_MACHINE") {
      return Err(format!(
        "snapshot has malformed `ELF_MACHINE` field: {line}"
      ));
    }

    return Err(format!(
      "unexpected snapshot line before symbol block: {line}"
    ));
  }

  if !in_symbol_block {
    return Err("snapshot missing `SYMBOLS:` block".to_string());
  }

  if symbols.is_empty() {
    return Err("snapshot has empty `SYMBOLS:` block".to_string());
  }

  let class = class.ok_or_else(|| "snapshot missing `ELF_CLASS` field".to_string())?;
  let machine = machine.ok_or_else(|| "snapshot missing `ELF_MACHINE` field".to_string())?;

  Ok(AbiSnapshot {
    class,
    machine,
    symbols,
  })
}

fn is_valid_snapshot_symbol(symbol: &str) -> bool {
  let mut chars = symbol.chars();
  let Some(first) = chars.next() else {
    return false;
  };

  if !(first.is_ascii_alphabetic() || first == '_') {
    return false;
  }

  chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn diff_against_golden(expected: &AbiSnapshot, actual: &AbiSnapshot) -> SnapshotDiff {
  let class_mismatch = if expected.class == actual.class {
    None
  } else {
    Some((expected.class.clone(), actual.class.clone()))
  };
  let machine_mismatch = if expected.machine == actual.machine {
    None
  } else {
    Some((expected.machine.clone(), actual.machine.clone()))
  };
  let missing_symbols = expected
    .symbols
    .difference(&actual.symbols)
    .cloned()
    .collect();
  let unexpected_symbols = actual
    .symbols
    .difference(&expected.symbols)
    .cloned()
    .collect();

  SnapshotDiff {
    class_mismatch,
    machine_mismatch,
    missing_symbols,
    unexpected_symbols,
  }
}

fn print_snapshot_diff(diff: &SnapshotDiff) {
  if let Some((expected, actual)) = &diff.class_mismatch {
    println!("ELF class mismatch: expected `{expected}`, found `{actual}`");
  }

  if let Some((expected, actual)) = &diff.machine_mismatch {
    println!("ELF machine mismatch: expected `{expected}`, found `{actual}`");
  }

  if !diff.missing_symbols.is_empty() {
    println!("Missing symbols compared to golden:");

    for symbol in &diff.missing_symbols {
      println!("  - {symbol}");
    }
  }

  if !diff.unexpected_symbols.is_empty() {
    println!("Unexpected symbols compared to golden:");

    for symbol in &diff.unexpected_symbols {
      println!("  - {symbol}");
    }
  }
}

fn normalize_library_path(path: &str) -> String {
  normalize_library_path_for_comparison(path)
    .display()
    .to_string()
}

fn normalize_library_path_for_comparison(path: &str) -> PathBuf {
  let normalized = Path::new(path);
  let absolute = if normalized.is_absolute() {
    PathBuf::from(normalized)
  } else {
    env::current_dir().map_or_else(|_| normalized.to_path_buf(), |cwd| cwd.join(normalized))
  };
  let collapsed = collapse_path_components(&absolute);

  fs::canonicalize(&collapsed).unwrap_or(collapsed)
}

fn collapse_path_components(path: &Path) -> PathBuf {
  let mut collapsed = PathBuf::new();

  for component in path.components() {
    match component {
      Component::CurDir => {}
      Component::ParentDir => {
        if collapsed.file_name().is_some() {
          let _ = collapsed.pop();
        } else if !collapsed.has_root() {
          collapsed.push(component.as_os_str());
        }
      }
      _ => collapsed.push(component.as_os_str()),
    }
  }

  collapsed
}

#[cfg(test)]
mod tests {
  use super::{
    AbiSnapshot, CliOptions, DEFAULT_LIBRARY_PATH, EXPECTED_ELF_CLASS, EXPECTED_MACHINE, ElfHeader,
    REQUIRED_SYMBOLS, SnapshotDiff, assign_golden_path, collapse_path_components,
    diff_against_golden, ensure_library_exists, ensure_library_exists_with, find_readelf_field,
    format_snapshot, missing_required_symbols, normalize_library_path,
    normalize_library_path_for_comparison, parse_cli_options, parse_nm_symbols,
    parse_readelf_header, parse_snapshot, resolve_library_path,
    should_rebuild_default_library_path, validate_elf_header,
  };
  use std::collections::BTreeSet;
  use std::path::PathBuf;
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::{env, fs, process};

  static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

  fn unique_temp_dir(label: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);

    env::temp_dir().join(format!("rlibc-abi-check-{label}-{}-{id}", process::id()))
  }

  #[test]
  fn parse_cli_options_accepts_golden_and_library_path() {
    let args = vec![
      "--golden".to_string(),
      "abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "target/release/librlibc.so".to_string(),
    ];
    let actual = parse_cli_options(&args).expect("valid arguments must parse");

    assert_eq!(
      actual,
      CliOptions {
        golden_path: Some("abi/golden/x86_64-unknown-linux-gnu.abi".to_string()),
        library_path: Some("target/release/librlibc.so".to_string()),
      }
    );
  }

  #[test]
  fn parse_cli_options_rejects_missing_golden_path() {
    let args = vec!["--golden".to_string()];
    let error = parse_cli_options(&args).expect_err("missing value must fail");

    assert!(error.contains("--golden"));
  }

  #[test]
  fn parse_cli_options_accepts_golden_equals_syntax() {
    let args = vec![
      "--golden=abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "target/release/librlibc.so".to_string(),
    ];
    let actual = parse_cli_options(&args).expect("equals-style --golden must parse");

    assert_eq!(
      actual,
      CliOptions {
        golden_path: Some("abi/golden/x86_64-unknown-linux-gnu.abi".to_string()),
        library_path: Some("target/release/librlibc.so".to_string()),
      }
    );
  }

  #[test]
  fn parse_cli_options_rejects_empty_golden_equals_value() {
    let args = vec!["--golden=".to_string()];
    let error = parse_cli_options(&args).expect_err("empty equals-style value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_whitespace_only_golden_equals_value() {
    let args = vec!["--golden=   ".to_string()];
    let error = parse_cli_options(&args).expect_err("whitespace-only equals-style value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_tab_only_golden_equals_value() {
    let args = vec!["--golden=\t\t".to_string()];
    let error = parse_cli_options(&args).expect_err("tab-only equals-style value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_newline_only_golden_equals_value() {
    let args = vec!["--golden=\n".to_string()];
    let error = parse_cli_options(&args).expect_err("newline-only equals-style value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_carriage_return_only_golden_equals_value() {
    let args = vec!["--golden=\r".to_string()];
    let error =
      parse_cli_options(&args).expect_err("carriage-return-only equals-style value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_accepts_option_like_golden_equals_value() {
    let args = vec!["--golden=--bogus".to_string()];
    let actual = parse_cli_options(&args).expect("option-like equals-style value must parse");

    assert_eq!(
      actual,
      CliOptions {
        library_path: None,
        golden_path: Some("--bogus".to_string()),
      }
    );
  }

  #[test]
  fn parse_cli_options_rejects_whitespace_prefixed_option_like_golden_equals_value() {
    let args = vec!["--golden=   --bogus".to_string()];
    let error = parse_cli_options(&args)
      .expect_err("whitespace-prefixed option-like equals-style value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_accepts_help_like_golden_equals_value() {
    let args = vec!["--golden=--help".to_string()];
    let actual = parse_cli_options(&args).expect("help-like equals-style value must parse");

    assert_eq!(
      actual,
      CliOptions {
        library_path: None,
        golden_path: Some("--help".to_string()),
      }
    );
  }

  #[test]
  fn parse_cli_options_rejects_option_like_golden_path() {
    let args = vec!["--golden".to_string(), "--bogus".to_string()];
    let error = parse_cli_options(&args).expect_err("option-like golden path must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_whitespace_prefixed_option_like_golden_path() {
    let args = vec!["--golden".to_string(), "   --bogus".to_string()];
    let error =
      parse_cli_options(&args).expect_err("whitespace-prefixed option-like golden path must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_empty_golden_path() {
    let args = vec!["--golden".to_string(), String::new()];
    let error = parse_cli_options(&args).expect_err("empty golden path must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_whitespace_only_golden_path() {
    let args = vec!["--golden".to_string(), "   ".to_string()];
    let error = parse_cli_options(&args).expect_err("whitespace-only separated value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_tab_only_golden_path() {
    let args = vec!["--golden".to_string(), "\t\t".to_string()];
    let error = parse_cli_options(&args).expect_err("tab-only separated value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_newline_only_golden_path() {
    let args = vec!["--golden".to_string(), "\n".to_string()];
    let error = parse_cli_options(&args).expect_err("newline-only separated value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_carriage_return_only_golden_path() {
    let args = vec!["--golden".to_string(), "\r".to_string()];
    let error =
      parse_cli_options(&args).expect_err("carriage-return-only separated value must fail");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn parse_cli_options_rejects_duplicate_golden_flags() {
    let args = vec![
      "--golden".to_string(),
      "one.abi".to_string(),
      "--golden".to_string(),
      "two.abi".to_string(),
    ];
    let error = parse_cli_options(&args).expect_err("duplicate --golden must fail");

    assert!(error.contains("duplicate"));
  }

  #[test]
  fn parse_cli_options_rejects_duplicate_golden_mixed_forms() {
    let args = vec![
      "--golden=one.abi".to_string(),
      "--golden".to_string(),
      "two.abi".to_string(),
    ];
    let error = parse_cli_options(&args).expect_err("duplicate mixed --golden forms must fail");

    assert!(error.contains("duplicate"));
  }

  #[test]
  fn parse_cli_options_reports_duplicate_before_option_like_value_validation() {
    let args = vec![
      "--golden".to_string(),
      "one.abi".to_string(),
      "--golden".to_string(),
      "--bogus".to_string(),
    ];
    let error = parse_cli_options(&args)
      .expect_err("duplicate golden flag should fail before value-shape validation");

    assert!(error.contains("duplicate --golden arguments are not supported"));
  }

  #[test]
  fn parse_cli_options_reports_duplicate_before_empty_equals_value_validation() {
    let args = vec!["--golden=one.abi".to_string(), "--golden=".to_string()];
    let error = parse_cli_options(&args)
      .expect_err("duplicate golden flag should fail before empty-value validation");

    assert!(error.contains("duplicate --golden arguments are not supported"));
  }

  #[test]
  fn parse_cli_options_reports_duplicate_before_whitespace_separated_value_validation() {
    let args = vec![
      "--golden".to_string(),
      "one.abi".to_string(),
      "--golden".to_string(),
      "   ".to_string(),
    ];
    let error = parse_cli_options(&args)
      .expect_err("duplicate golden flag should fail before whitespace-value validation");

    assert!(error.contains("duplicate --golden arguments are not supported"));
  }

  #[test]
  fn assign_golden_path_rejects_duplicate_assignment() {
    let mut options = CliOptions {
      library_path: None,
      golden_path: Some("one.abi".to_string()),
    };
    let error = assign_golden_path(&mut options, "two.abi", false)
      .expect_err("second golden assignment must fail");

    assert!(error.contains("duplicate --golden arguments are not supported"));
  }

  #[test]
  fn assign_golden_path_reports_duplicate_before_value_validation() {
    let mut options = CliOptions {
      library_path: None,
      golden_path: Some("one.abi".to_string()),
    };
    let error = assign_golden_path(&mut options, "--bogus", false)
      .expect_err("duplicate assignment must fail before path-shape validation");

    assert!(error.contains("duplicate --golden arguments are not supported"));
  }

  #[test]
  fn assign_golden_path_accepts_dash_prefixed_value_when_allowed() {
    let mut options = CliOptions {
      library_path: None,
      golden_path: None,
    };

    assign_golden_path(&mut options, "--bogus", true)
      .expect("dash-prefixed value should be accepted for equals form");

    assert_eq!(options.golden_path, Some("--bogus".to_string()));
  }

  #[test]
  fn assign_golden_path_rejects_dash_prefixed_value_when_not_allowed() {
    let mut options = CliOptions {
      library_path: None,
      golden_path: None,
    };
    let error = assign_golden_path(&mut options, "--bogus", false)
      .expect_err("dash-prefixed value must be rejected for separated form");

    assert!(error.contains("missing value for --golden"));
  }

  #[test]
  fn assign_golden_path_rejects_whitespace_only_value_even_when_allowed() {
    let mut options = CliOptions {
      library_path: None,
      golden_path: None,
    };
    let error = assign_golden_path(&mut options, "   ", true)
      .expect_err("whitespace-only value must be rejected");

    assert!(error.contains("missing value for --golden"));
    assert_eq!(options.golden_path, None);
  }

  #[test]
  fn parse_cli_options_rejects_multiple_positional_library_paths() {
    let args = vec!["one.so".to_string(), "two.so".to_string()];
    let error = parse_cli_options(&args).expect_err("multiple positional paths must fail");

    assert!(error.contains("at most one positional"));
  }

  #[test]
  fn parse_cli_options_rejects_empty_library_path() {
    let args = vec![String::new()];
    let error = parse_cli_options(&args).expect_err("empty library path must fail");

    assert!(error.contains("library path must not be empty"));
  }

  #[test]
  fn parse_cli_options_rejects_whitespace_only_library_path() {
    let args = vec!["   ".to_string()];
    let error = parse_cli_options(&args).expect_err("whitespace-only library path must fail");

    assert!(error.contains("library path must not be empty"));
  }

  #[test]
  fn parse_cli_options_rejects_whitespace_only_library_path_after_double_dash() {
    let args = vec!["--".to_string(), "   ".to_string()];
    let error =
      parse_cli_options(&args).expect_err("whitespace-only path after `--` must still fail");

    assert!(error.contains("library path must not be empty"));
  }

  #[test]
  fn parse_cli_options_rejects_tab_only_library_path_after_double_dash() {
    let args = vec!["--".to_string(), "\t\t".to_string()];
    let error = parse_cli_options(&args).expect_err("tab-only path after `--` must still fail");

    assert!(error.contains("library path must not be empty"));
  }

  #[test]
  fn parse_cli_options_rejects_newline_only_library_path_after_double_dash() {
    let args = vec!["--".to_string(), "\n".to_string()];
    let error = parse_cli_options(&args).expect_err("newline-only path after `--` must still fail");

    assert!(error.contains("library path must not be empty"));
  }

  #[test]
  fn parse_cli_options_rejects_carriage_return_only_library_path_after_double_dash() {
    let args = vec!["--".to_string(), "\r".to_string()];
    let error =
      parse_cli_options(&args).expect_err("carriage-return-only path after `--` must still fail");

    assert!(error.contains("library path must not be empty"));
  }

  #[test]
  fn parse_cli_options_accepts_dash_prefixed_library_path_after_double_dash() {
    let args = vec!["--".to_string(), "--custom-librlibc.so".to_string()];
    let actual = parse_cli_options(&args).expect("double-dash terminator should allow path");

    assert_eq!(
      actual,
      CliOptions {
        library_path: Some("--custom-librlibc.so".to_string()),
        golden_path: None,
      }
    );
  }

  #[test]
  fn parse_cli_options_accepts_golden_then_double_dash_library_path() {
    let args = vec![
      "--golden".to_string(),
      "abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "--".to_string(),
      "--custom-librlibc.so".to_string(),
    ];
    let actual = parse_cli_options(&args).expect("golden + double-dash path should parse");

    assert_eq!(
      actual,
      CliOptions {
        library_path: Some("--custom-librlibc.so".to_string()),
        golden_path: Some("abi/golden/x86_64-unknown-linux-gnu.abi".to_string()),
      }
    );
  }

  #[test]
  fn parse_cli_options_accepts_golden_equals_then_double_dash_library_path() {
    let args = vec![
      "--golden=abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "--".to_string(),
      "--custom-librlibc.so".to_string(),
    ];
    let actual = parse_cli_options(&args).expect("golden= + double-dash path should parse");

    assert_eq!(
      actual,
      CliOptions {
        library_path: Some("--custom-librlibc.so".to_string()),
        golden_path: Some("abi/golden/x86_64-unknown-linux-gnu.abi".to_string()),
      }
    );
  }

  #[test]
  fn parse_cli_options_accepts_golden_then_double_dash_without_library_path() {
    let args = vec![
      "--golden".to_string(),
      "abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "--".to_string(),
    ];
    let actual =
      parse_cli_options(&args).expect("golden + trailing double-dash without path should parse");

    assert_eq!(
      actual,
      CliOptions {
        library_path: None,
        golden_path: Some("abi/golden/x86_64-unknown-linux-gnu.abi".to_string()),
      }
    );
  }

  #[test]
  fn parse_cli_options_accepts_golden_equals_then_double_dash_without_library_path() {
    let args = vec![
      "--golden=abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "--".to_string(),
    ];
    let actual =
      parse_cli_options(&args).expect("golden= + trailing double-dash without path should parse");

    assert_eq!(
      actual,
      CliOptions {
        library_path: None,
        golden_path: Some("abi/golden/x86_64-unknown-linux-gnu.abi".to_string()),
      }
    );
  }

  #[test]
  fn parse_cli_options_treats_golden_equals_as_literal_after_double_dash() {
    let args = vec![
      "--".to_string(),
      "--golden=abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
    ];
    let actual = parse_cli_options(&args).expect("double-dash should disable --golden parsing");

    assert_eq!(
      actual,
      CliOptions {
        library_path: Some("--golden=abi/golden/x86_64-unknown-linux-gnu.abi".to_string()),
        golden_path: None,
      }
    );
  }

  #[test]
  fn parse_cli_options_accepts_double_dash_without_library_path() {
    let args = vec!["--".to_string()];
    let actual =
      parse_cli_options(&args).expect("double-dash without positional path should parse");

    assert_eq!(
      actual,
      CliOptions {
        library_path: None,
        golden_path: None,
      }
    );
  }

  #[test]
  fn has_help_flag_detects_help_before_double_dash() {
    let args = vec![
      "--golden".to_string(),
      "abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "--help".to_string(),
      "--".to_string(),
      "--not-a-flag.so".to_string(),
    ];

    assert!(super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_detects_help_after_golden_argument() {
    let args = vec![
      "--golden".to_string(),
      "abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "--help".to_string(),
    ];

    assert!(super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_detects_short_help_after_golden_equals_argument() {
    let args = vec![
      "--golden=abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "-h".to_string(),
    ];

    assert!(super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_detects_long_help_after_golden_equals_argument() {
    let args = vec![
      "--golden=abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "--help".to_string(),
    ];

    assert!(super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_long_help_after_empty_golden_equals_argument() {
    let args = vec!["--golden=".to_string(), "--help".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_help_after_whitespace_only_golden_equals_argument() {
    let args = vec!["--golden=   ".to_string(), "--help".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_short_help_after_whitespace_only_golden_equals_argument() {
    let args = vec!["--golden=   ".to_string(), "-h".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_help_after_newline_only_golden_equals_argument() {
    let args = vec!["--golden=\n".to_string(), "--help".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_help_after_option_like_golden_equals_argument() {
    let args = vec![
      "--golden=--option-like-path.abi".to_string(),
      "--help".to_string(),
    ];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_help_after_whitespace_prefixed_option_like_golden_equals_argument() {
    let args = vec![
      "--golden=   --option-like-path.abi".to_string(),
      "--help".to_string(),
    ];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_short_help_after_whitespace_prefixed_option_like_golden_equals_argument()
  {
    let args = vec![
      "--golden=   --option-like-path.abi".to_string(),
      "-h".to_string(),
    ];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_help_after_whitespace_prefixed_option_like_golden_argument() {
    let args = vec![
      "--golden".to_string(),
      "   --option-like-path.abi".to_string(),
      "--help".to_string(),
    ];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_short_help_after_whitespace_prefixed_option_like_golden_argument() {
    let args = vec![
      "--golden".to_string(),
      "   --option-like-path.abi".to_string(),
      "-h".to_string(),
    ];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_help_after_double_dash_consumed_by_invalid_golden_value() {
    let args = vec![
      "--golden".to_string(),
      "--".to_string(),
      "--help".to_string(),
    ];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_short_help_after_empty_golden_equals_argument() {
    let args = vec!["--golden=".to_string(), "-h".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_detects_short_help_after_golden_argument() {
    let args = vec![
      "--golden".to_string(),
      "abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
      "-h".to_string(),
    ];

    assert!(super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_help_after_double_dash() {
    let args = vec![
      "--".to_string(),
      "--help".to_string(),
      "--custom-librlibc.so".to_string(),
    ];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_long_help_token_consumed_by_golden_value() {
    let args = vec!["--golden".to_string(), "--help".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_short_help_token_consumed_by_golden_value() {
    let args = vec!["--golden".to_string(), "-h".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_long_help_in_golden_equals_value() {
    let args = vec!["--golden=--help".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn has_help_flag_ignores_short_help_in_golden_equals_value() {
    let args = vec!["--golden=-h".to_string()];

    assert!(!super::has_help_flag(&args));
  }

  #[test]
  fn parse_nm_symbols_collects_dynamic_defined_symbols() {
    let output = "\
abort T 0000000000001100 30
strlen T 0000000000001200 40
__errno_location T 0000000000001300 20
";
    let symbols = parse_nm_symbols(output);
    let expected = BTreeSet::from([
      "__errno_location".to_string(),
      "abort".to_string(),
      "strlen".to_string(),
    ]);

    assert_eq!(symbols, expected);
  }

  #[test]
  fn parse_nm_symbols_skips_non_symbol_lines() {
    let output = "\
nm: target/release/librlibc.so: no symbols
memcpy T 0000000000001234 16
";
    let symbols = parse_nm_symbols(output);

    assert_eq!(symbols, BTreeSet::from(["memcpy".to_string()]));
  }

  #[test]
  fn parse_nm_symbols_strips_symbol_versions() {
    let output = "strlen@@RLIBC_0.1 T 0000000000001750 1c";
    let symbols = parse_nm_symbols(output);

    assert_eq!(symbols, BTreeSet::from(["strlen".to_string()]));
  }

  #[test]
  fn parse_readelf_header_extracts_class_and_machine() {
    let output = "\
ELF Header:
  Class:                             ELF64
  Machine:                           Advanced Micro Devices X86-64
";
    let header = parse_readelf_header(output).expect("header should parse");

    assert_eq!(
      header,
      ElfHeader {
        class: "ELF64".to_string(),
        machine: "Advanced Micro Devices X86-64".to_string(),
      },
    );
  }

  #[test]
  fn parse_readelf_header_fails_when_machine_is_missing() {
    let output = "\
ELF Header:
  Class:                             ELF64
";
    let error = parse_readelf_header(output).expect_err("missing machine must fail");

    assert!(error.contains("Machine"));
  }

  #[test]
  fn find_readelf_field_extracts_trimmed_value() {
    let output = "  Class:                             ELF64";
    let value = find_readelf_field(output, "Class").expect("field should parse");

    assert_eq!(value, "ELF64");
  }

  #[test]
  fn validate_elf_header_accepts_expected_target() {
    let header = ElfHeader {
      class: EXPECTED_ELF_CLASS.to_string(),
      machine: EXPECTED_MACHINE.to_string(),
    };

    assert!(validate_elf_header(&header).is_ok());
  }

  #[test]
  fn validate_elf_header_rejects_unexpected_machine() {
    let header = ElfHeader {
      class: EXPECTED_ELF_CLASS.to_string(),
      machine: "AArch64".to_string(),
    };
    let error = validate_elf_header(&header).expect_err("unexpected machine must fail");

    assert!(error.contains("unexpected ELF machine"));
  }

  #[test]
  fn missing_required_symbols_reports_absent_entries() {
    let exported = BTreeSet::from([
      "__errno_location".to_string(),
      "abort".to_string(),
      "atexit".to_string(),
      "memcpy".to_string(),
      "memmove".to_string(),
      "memset".to_string(),
      "strlen".to_string(),
      "strnlen".to_string(),
    ]);
    let missing = missing_required_symbols(&exported);

    assert!(missing.contains(&"memcmp"));
    assert!(missing.contains(&"getenv"));
    assert!(!missing.contains(&"memcpy"));
  }

  #[test]
  fn resolve_library_path_uses_default_when_no_argument() {
    let args: Vec<String> = vec![];
    let actual = resolve_library_path(&args).expect("default path should resolve");

    assert_eq!(actual, "target/release/librlibc.so");
  }

  #[test]
  fn resolve_library_path_rejects_multiple_arguments() {
    let args = vec!["one".to_string(), "two".to_string()];
    let error = resolve_library_path(&args).expect_err("multiple args must fail");

    assert!(error.contains("at most one"));
  }

  #[test]
  fn should_rebuild_default_library_path_accepts_explicit_default_path() {
    assert!(should_rebuild_default_library_path(
      DEFAULT_LIBRARY_PATH,
      false
    ));

    let absolute_default_path = normalize_library_path(DEFAULT_LIBRARY_PATH);

    assert!(should_rebuild_default_library_path(
      &absolute_default_path,
      false
    ));
    assert!(!should_rebuild_default_library_path(
      "target/release/custom-librlibc.so",
      false
    ));
  }

  #[test]
  fn should_rebuild_default_library_path_accepts_equivalent_relative_path() {
    assert!(should_rebuild_default_library_path(
      "./target/release/librlibc.so",
      false
    ));
    assert!(should_rebuild_default_library_path(
      "target/release/../release/librlibc.so",
      false
    ));
  }

  #[test]
  fn normalize_library_path_for_comparison_collapses_relative_components() {
    let default_path = normalize_library_path_for_comparison(DEFAULT_LIBRARY_PATH);
    let dotted_path = normalize_library_path_for_comparison("./target/release/librlibc.so");
    let parent_path =
      normalize_library_path_for_comparison("target/release/../release/librlibc.so");

    assert_eq!(default_path, dotted_path);
    assert_eq!(default_path, parent_path);
  }

  #[test]
  fn collapse_path_components_preserves_leading_parent_components_for_relative_paths() {
    let collapsed = collapse_path_components(std::path::Path::new("../target/release/librlibc.so"));

    assert_eq!(collapsed, PathBuf::from("../target/release/librlibc.so"));
  }

  #[test]
  #[cfg(unix)]
  fn collapse_path_components_keeps_root_when_parent_reaches_root() {
    let collapsed = collapse_path_components(std::path::Path::new("/../tmp/librlibc.so"));

    assert_eq!(collapsed, PathBuf::from("/tmp/librlibc.so"));
  }

  #[test]
  #[cfg(unix)]
  fn normalize_library_path_for_comparison_resolves_existing_symlink() {
    use std::os::unix::fs::symlink;

    let temp_dir = unique_temp_dir("normalize-existing-symlink");
    let target_path = temp_dir.join("librlibc-target.so");
    let symlink_path = temp_dir.join("librlibc-symlink.so");
    let target_path_string = target_path.to_string_lossy().to_string();
    let symlink_path_string = symlink_path.to_string_lossy().to_string();
    let _ = fs::remove_dir_all(&temp_dir);

    fs::create_dir_all(&temp_dir).expect("temp dir creation must succeed");
    fs::write(&target_path, b"target").expect("target file write must succeed");
    symlink(&target_path, &symlink_path).expect("symlink creation must succeed");

    let normalized_target = normalize_library_path_for_comparison(&target_path_string);
    let normalized_symlink = normalize_library_path_for_comparison(&symlink_path_string);

    assert_eq!(normalized_target, normalized_symlink);

    let _ = fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn ensure_library_exists_rejects_missing_explicit_path() {
    let error = ensure_library_exists("target/release/missing-lib.so", false)
      .expect_err("missing explicit library path must fail");

    assert!(error.contains("library path does not exist"));
  }

  #[test]
  fn ensure_library_exists_keeps_existing_explicit_path_without_rebuild() {
    let temp_dir = unique_temp_dir("explicit-existing-no-rebuild");
    let library_path = temp_dir.join("librlibc.so");
    let library_path_string = library_path.to_string_lossy().to_string();
    let mut build_invocations = 0_u32;
    let _ = fs::remove_dir_all(&temp_dir);

    fs::create_dir_all(&temp_dir).expect("temp dir creation must succeed");
    fs::write(&library_path, b"explicit").expect("library stub write must succeed");

    let result = ensure_library_exists_with(&library_path_string, false, || {
      build_invocations += 1;

      Ok(())
    });

    assert!(result.is_ok());
    assert_eq!(build_invocations, 0);

    let _ = fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn ensure_library_exists_missing_explicit_path_does_not_attempt_rebuild() {
    let temp_dir = unique_temp_dir("explicit-missing-no-rebuild");
    let library_path = temp_dir.join("librlibc.so");
    let library_path_string = library_path.to_string_lossy().to_string();
    let mut build_invocations = 0_u32;
    let _ = fs::remove_dir_all(&temp_dir);
    let error = ensure_library_exists_with(&library_path_string, false, || {
      build_invocations += 1;
      Ok(())
    })
    .expect_err("missing explicit path must fail without rebuild");

    assert!(error.contains("library path does not exist"));
    assert_eq!(build_invocations, 0);
  }

  #[test]
  fn ensure_library_exists_rejects_explicit_directory_path_without_rebuild() {
    let temp_dir = unique_temp_dir("explicit-directory-no-rebuild");
    let directory_path = temp_dir.join("librlibc.so");
    let directory_path_string = directory_path.to_string_lossy().to_string();
    let mut build_invocations = 0_u32;
    let _ = fs::remove_dir_all(&temp_dir);

    fs::create_dir_all(&directory_path).expect("directory path creation must succeed");

    let error = ensure_library_exists_with(&directory_path_string, false, || {
      build_invocations += 1;

      Ok(())
    })
    .expect_err("explicit directory path must fail");

    assert!(error.contains("library path is not a regular file"));
    assert_eq!(build_invocations, 0);

    let _ = fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn ensure_library_exists_builds_missing_default_path() {
    let temp_dir = unique_temp_dir("builds-missing-default");
    let library_path = temp_dir.join("librlibc.so");
    let library_path_string = library_path.to_string_lossy().to_string();
    let mut build_invocations = 0_u32;
    let _ = fs::remove_dir_all(&temp_dir);
    let result = ensure_library_exists_with(&library_path_string, true, || {
      build_invocations += 1;
      fs::create_dir_all(&temp_dir).expect("temp dir creation must succeed");
      fs::write(&library_path, b"stub").expect("library stub write must succeed");

      Ok(())
    });

    assert!(result.is_ok());
    assert_eq!(build_invocations, 1);

    let _ = fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn ensure_library_exists_fails_when_build_leaves_default_path_missing() {
    let temp_dir = unique_temp_dir("missing-after-build");
    let library_path = temp_dir.join("librlibc.so");
    let library_path_string = library_path.to_string_lossy().to_string();
    let _ = fs::remove_dir_all(&temp_dir);
    let error = ensure_library_exists_with(&library_path_string, true, || Ok(()))
      .expect_err("missing default path after build must fail");

    assert!(error.contains("default library path missing after cdylib build"));
  }

  #[test]
  fn ensure_library_exists_rebuilds_when_default_path_is_directory() {
    let temp_dir = unique_temp_dir("default-directory-rebuild");
    let directory_path = temp_dir.join("librlibc.so");
    let directory_path_string = directory_path.to_string_lossy().to_string();
    let mut build_invocations = 0_u32;
    let _ = fs::remove_dir_all(&temp_dir);

    fs::create_dir_all(&directory_path).expect("directory path creation must succeed");

    let result = ensure_library_exists_with(&directory_path_string, true, || {
      build_invocations += 1;
      fs::remove_dir_all(&directory_path).expect("directory cleanup must succeed");
      fs::write(&directory_path, b"rebuilt").expect("library file write must succeed");

      Ok(())
    });

    assert!(result.is_ok());
    assert_eq!(build_invocations, 1);

    let _ = fs::remove_file(&directory_path);
    let _ = fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn ensure_library_exists_fails_when_default_path_stays_directory_after_build() {
    let temp_dir = unique_temp_dir("default-directory-stays-directory");
    let directory_path = temp_dir.join("librlibc.so");
    let directory_path_string = directory_path.to_string_lossy().to_string();
    let mut build_invocations = 0_u32;
    let _ = fs::remove_dir_all(&temp_dir);

    fs::create_dir_all(&directory_path).expect("directory path creation must succeed");

    let error = ensure_library_exists_with(&directory_path_string, true, || {
      build_invocations += 1;

      Ok(())
    })
    .expect_err("default path that stays directory must fail");

    assert!(error.contains("default library path is not a regular file"));
    assert_eq!(build_invocations, 1);

    let _ = fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn ensure_library_exists_skips_rebuild_when_default_path_exists() {
    let temp_dir = unique_temp_dir("skips-rebuild-existing-default");
    let library_path = temp_dir.join("librlibc.so");
    let library_path_string = library_path.to_string_lossy().to_string();
    let mut build_invocations = 0_u32;
    let _ = fs::remove_dir_all(&temp_dir);

    fs::create_dir_all(&temp_dir).expect("temp dir creation must succeed");
    fs::write(&library_path, b"stub").expect("library stub write must succeed");

    let result = ensure_library_exists_with(&library_path_string, true, || {
      build_invocations += 1;

      Ok(())
    });

    assert!(result.is_ok());
    assert_eq!(build_invocations, 0);

    let _ = fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn ensure_library_exists_skips_build_failure_when_default_path_exists() {
    let temp_dir = unique_temp_dir("build-failure-existing-default");
    let library_path = temp_dir.join("librlibc.so");
    let library_path_string = library_path.to_string_lossy().to_string();
    let mut build_invocations = 0_u32;
    let _ = fs::remove_dir_all(&temp_dir);

    fs::create_dir_all(&temp_dir).expect("temp dir creation must succeed");
    fs::write(&library_path, b"stale").expect("library stub write must succeed");

    let result = ensure_library_exists_with(&library_path_string, true, || {
      build_invocations += 1;
      Err("forced build failure".to_string())
    });

    assert!(result.is_ok());
    assert_eq!(build_invocations, 0);

    let _ = fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn normalize_library_path_returns_absolute_path() {
    let actual = normalize_library_path("target/release/librlibc.so");

    assert!(actual.starts_with('/'));
    assert!(actual.ends_with("target/release/librlibc.so"));
  }

  #[test]
  fn parse_snapshot_round_trips_symbols_and_header() {
    let snapshot = AbiSnapshot {
      class: "ELF64".to_string(),
      machine: "Advanced Micro Devices X86-64".to_string(),
      symbols: BTreeSet::from(["abort".to_string(), "memcpy".to_string()]),
    };
    let serialized = format_snapshot(&snapshot);
    let parsed = parse_snapshot(&serialized).expect("snapshot should parse");

    assert_eq!(parsed, snapshot);
  }

  #[test]
  fn parse_snapshot_rejects_missing_fields() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("snapshot must reject missing machine");

    assert!(error.contains("ELF_MACHINE"));
  }

  #[test]
  fn parse_snapshot_rejects_magic_header_with_leading_whitespace() {
    let snapshot = " ABI_SNAPSHOT_V1\nELF_CLASS=ELF64\nELF_MACHINE=Advanced Micro Devices X86-64\nSYMBOLS:\nmemcpy\n";
    let error = parse_snapshot(snapshot).expect_err("magic header must be matched exactly");

    assert!(error.contains("magic header with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_reports_specific_error_for_magic_header_with_trailing_whitespace() {
    let snapshot = "ABI_SNAPSHOT_V1 \nELF_CLASS=ELF64\nELF_MACHINE=Advanced Micro Devices X86-64\nSYMBOLS:\nmemcpy\n";
    let error = parse_snapshot(snapshot).expect_err("magic header with trailing space must fail");

    assert!(error.contains("magic header with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_magic_header_with_leading_tab() {
    let snapshot = "\tABI_SNAPSHOT_V1\nELF_CLASS=ELF64\nELF_MACHINE=Advanced Micro Devices X86-64\nSYMBOLS:\nmemcpy\n";
    let error = parse_snapshot(snapshot).expect_err("magic header with leading tab must fail");

    assert!(error.contains("magic header with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_magic_header_with_trailing_tab() {
    let snapshot = "ABI_SNAPSHOT_V1\t\nELF_CLASS=ELF64\nELF_MACHINE=Advanced Micro Devices X86-64\nSYMBOLS:\nmemcpy\n";
    let error = parse_snapshot(snapshot).expect_err("magic header with trailing tab must fail");

    assert!(error.contains("magic header with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_accepts_magic_header_with_crlf_line_endings() {
    let snapshot = "ABI_SNAPSHOT_V1\r\nELF_CLASS=ELF64\nELF_MACHINE=Advanced Micro Devices X86-64\nSYMBOLS:\nmemcpy\n";
    let parsed = parse_snapshot(snapshot)
      .expect("CRLF line endings in snapshot header should be accepted for portability");

    assert_eq!(parsed.class, "ELF64");
    assert_eq!(parsed.machine, "Advanced Micro Devices X86-64");
    assert!(parsed.symbols.contains("memcpy"));
  }

  #[test]
  fn parse_snapshot_accepts_snapshot_with_all_crlf_line_endings() {
    let snapshot = concat!(
      "ABI_SNAPSHOT_V1\r\n",
      "ELF_CLASS=ELF64\r\n",
      "ELF_MACHINE=Advanced Micro Devices X86-64\r\n",
      "SYMBOLS:\r\n",
      "memcpy\r\n",
    );
    let parsed =
      parse_snapshot(snapshot).expect("snapshot parser should accept full CRLF line endings");

    assert_eq!(parsed.class, "ELF64");
    assert_eq!(parsed.machine, "Advanced Micro Devices X86-64");
    assert!(parsed.symbols.contains("memcpy"));
  }

  #[test]
  fn parse_snapshot_accepts_class_line_with_crlf_line_endings() {
    let snapshot = concat!(
      "ABI_SNAPSHOT_V1\n",
      "ELF_CLASS=ELF64\r\n",
      "ELF_MACHINE=Advanced Micro Devices X86-64\n",
      "SYMBOLS:\n",
      "memcpy\n",
    );
    let parsed = parse_snapshot(snapshot)
      .expect("parser should accept CRLF line endings on ELF_CLASS metadata line");

    assert_eq!(parsed.class, "ELF64");
    assert_eq!(parsed.machine, "Advanced Micro Devices X86-64");
    assert!(parsed.symbols.contains("memcpy"));
  }

  #[test]
  fn parse_snapshot_rejects_empty_symbols_block() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
";
    let error = parse_snapshot(snapshot).expect_err("empty SYMBOLS block must fail");

    assert!(error.contains("empty `SYMBOLS:` block"));
  }

  #[test]
  fn parse_snapshot_rejects_empty_class_field() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("empty ELF_CLASS must fail");

    assert!(error.contains("empty `ELF_CLASS`"));
  }

  #[test]
  fn parse_snapshot_rejects_empty_machine_field() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("empty ELF_MACHINE must fail");

    assert!(error.contains("empty `ELF_MACHINE`"));
  }

  #[test]
  fn parse_snapshot_rejects_whitespace_only_class_field() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS= \t
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("whitespace-only ELF_CLASS must fail");

    assert!(error.contains("empty `ELF_CLASS`"));
  }

  #[test]
  fn parse_snapshot_rejects_class_field_with_leading_space() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS= ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("ELF_CLASS with leading space must fail");

    assert!(error.contains("invalid `ELF_CLASS` field with leading whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_class_line_with_leading_whitespace() {
    let snapshot = "\
ABI_SNAPSHOT_V1
 ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error =
      parse_snapshot(snapshot).expect_err("ELF_CLASS line with leading whitespace must fail");

    assert!(error.contains("invalid `ELF_CLASS` line with leading whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_class_field_with_trailing_space() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64 
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("ELF_CLASS with trailing space must fail");

    assert!(error.contains("invalid `ELF_CLASS` field with trailing whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_whitespace_only_machine_field() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE= \t
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("whitespace-only ELF_MACHINE must fail");

    assert!(error.contains("empty `ELF_MACHINE`"));
  }

  #[test]
  fn parse_snapshot_rejects_machine_field_with_leading_space() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE= Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("ELF_MACHINE with leading space must fail");

    assert!(error.contains("invalid `ELF_MACHINE` field with leading whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_machine_line_with_leading_whitespace() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
 ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error =
      parse_snapshot(snapshot).expect_err("ELF_MACHINE line with leading whitespace must fail");

    assert!(error.contains("invalid `ELF_MACHINE` line with leading whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_machine_field_with_trailing_space() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64 
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("ELF_MACHINE with trailing space must fail");

    assert!(error.contains("invalid `ELF_MACHINE` field with trailing whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_class_field_with_internal_whitespace() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF 64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("ELF_CLASS with internal whitespace must fail");

    assert!(error.contains("invalid `ELF_CLASS` field with whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_class_field_with_space_before_equals() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS =ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error =
      parse_snapshot(snapshot).expect_err("ELF_CLASS with a space before '=' must be rejected");

    assert!(error.contains("malformed `ELF_CLASS` field"));
  }

  #[test]
  fn parse_snapshot_rejects_machine_field_with_tab_character() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced\tMicro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("ELF_MACHINE with tab character must fail");

    assert!(error.contains("invalid `ELF_MACHINE` field with non-space whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_machine_field_with_space_before_equals() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE =Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error =
      parse_snapshot(snapshot).expect_err("ELF_MACHINE with a space before '=' must be rejected");

    assert!(error.contains("malformed `ELF_MACHINE` field"));
  }

  #[test]
  fn parse_snapshot_rejects_duplicate_class_field() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("duplicate ELF_CLASS must fail");

    assert!(error.contains("duplicate `ELF_CLASS`"));
  }

  #[test]
  fn parse_snapshot_rejects_duplicate_machine_field() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("duplicate ELF_MACHINE must fail");

    assert!(error.contains("duplicate `ELF_MACHINE`"));
  }

  #[test]
  fn parse_snapshot_rejects_duplicate_symbols_block() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
SYMBOLS:
memmove
";
    let error = parse_snapshot(snapshot).expect_err("duplicate SYMBOLS block must fail");

    assert!(error.contains("duplicate `SYMBOLS:`"));
  }

  #[test]
  fn parse_snapshot_rejects_symbols_line_with_leading_whitespace() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
 SYMBOLS:
memcpy
";
    let error = parse_snapshot(snapshot)
      .expect_err("SYMBOLS line with leading whitespace must not be normalized");

    assert!(error.contains("invalid `SYMBOLS:` line with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_symbols_line_with_trailing_whitespace() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS: 
memcpy
";
    let error = parse_snapshot(snapshot)
      .expect_err("SYMBOLS line with trailing whitespace must not be normalized");

    assert!(error.contains("invalid `SYMBOLS:` line with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_symbols_line_with_leading_tab() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
\tSYMBOLS:
memcpy
";
    let error =
      parse_snapshot(snapshot).expect_err("SYMBOLS line with leading tab must not be normalized");

    assert!(error.contains("invalid `SYMBOLS:` line with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_symbols_line_with_trailing_tab() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:\t
memcpy
";
    let error =
      parse_snapshot(snapshot).expect_err("SYMBOLS line with trailing tab must not be normalized");

    assert!(error.contains("invalid `SYMBOLS:` line with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_accepts_symbols_line_with_crlf_line_endings() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:\r
memcpy
";
    let parsed = parse_snapshot(snapshot)
      .expect("CRLF line endings in SYMBOLS line should be accepted for portability");

    assert_eq!(parsed.class, "ELF64");
    assert_eq!(parsed.machine, "Advanced Micro Devices X86-64");
    assert!(parsed.symbols.contains("memcpy"));
  }

  #[test]
  fn parse_snapshot_rejects_duplicate_symbol_entries() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
memcpy
";
    let error = parse_snapshot(snapshot).expect_err("duplicate symbol entries must fail");

    assert!(error.contains("duplicate symbol entry"));
  }

  #[test]
  fn parse_snapshot_rejects_symbol_entries_with_whitespace() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy alias
";
    let error =
      parse_snapshot(snapshot).expect_err("symbol entries containing whitespace must be rejected");

    assert!(error.contains("invalid symbol entry with whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_symbol_entries_with_tab_character() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy\talias
";
    let error = parse_snapshot(snapshot)
      .expect_err("symbol entries containing tab characters must be rejected");

    assert!(error.contains("invalid symbol entry with whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_symbol_entries_with_surrounding_whitespace() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
 memcpy
";
    let error = parse_snapshot(snapshot)
      .expect_err("symbol entries with surrounding whitespace must not be normalized silently");

    assert!(error.contains("invalid symbol entry with surrounding whitespace"));
  }

  #[test]
  fn parse_snapshot_rejects_empty_line_inside_symbols_block() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy

memmove
";
    let error = parse_snapshot(snapshot)
      .expect_err("empty lines inside `SYMBOLS:` must be rejected to avoid silent normalization");

    assert!(error.contains("empty line inside `SYMBOLS:` block"));
  }

  #[test]
  fn parse_snapshot_rejects_leading_empty_line_inside_symbols_block() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:

memcpy
";
    let error =
      parse_snapshot(snapshot).expect_err("leading empty line inside `SYMBOLS:` must be rejected");

    assert!(error.contains("empty line inside `SYMBOLS:` block"));
  }

  #[test]
  fn parse_snapshot_accepts_trailing_empty_line_after_symbols_block() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy

";
    let parsed =
      parse_snapshot(snapshot).expect("trailing empty line after symbol list should be accepted");

    assert_eq!(parsed.symbols, BTreeSet::from(["memcpy".to_string()]));
  }

  #[test]
  fn parse_snapshot_accepts_trailing_crlf_empty_line_after_symbols_block() {
    let snapshot = concat!(
      "ABI_SNAPSHOT_V1\r\n",
      "ELF_CLASS=ELF64\r\n",
      "ELF_MACHINE=Advanced Micro Devices X86-64\r\n",
      "SYMBOLS:\r\n",
      "memcpy\r\n",
      "\r\n",
    );
    let parsed = parse_snapshot(snapshot)
      .expect("trailing CRLF empty line after symbol list should be accepted");

    assert_eq!(parsed.class, "ELF64");
    assert_eq!(parsed.machine, "Advanced Micro Devices X86-64");
    assert_eq!(parsed.symbols, BTreeSet::from(["memcpy".to_string()]));
  }

  #[test]
  fn parse_snapshot_rejects_trailing_whitespace_only_line_after_symbols_block() {
    let snapshot = concat!(
      "ABI_SNAPSHOT_V1\n",
      "ELF_CLASS=ELF64\n",
      "ELF_MACHINE=Advanced Micro Devices X86-64\n",
      "SYMBOLS:\n",
      "memcpy\n",
      " \t \n",
    );
    let error = parse_snapshot(snapshot).expect_err(
      "trailing whitespace-only line after symbol list must not be normalized as empty",
    );

    assert!(error.contains("invalid trailing whitespace-only line inside `SYMBOLS:` block"));
  }

  #[test]
  fn parse_snapshot_rejects_symbol_entries_with_trailing_semicolon() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy;
";
    let error = parse_snapshot(snapshot)
      .expect_err("symbol entries with trailing semicolons must be rejected");

    assert!(error.contains("invalid symbol entry"));
  }

  #[test]
  fn parse_snapshot_rejects_symbol_entries_starting_with_digit() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
9memcpy
";
    let error =
      parse_snapshot(snapshot).expect_err("symbol entries starting with digits must be rejected");

    assert!(error.contains("invalid symbol entry"));
  }

  #[test]
  fn parse_snapshot_rejects_version_suffixed_symbol_entries() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy@@GLIBC_2.2.5
";
    let error =
      parse_snapshot(snapshot).expect_err("symbol entries with version suffixes must be rejected");

    assert!(error.contains("invalid symbol entry"));
  }

  #[test]
  fn parse_snapshot_rejects_metadata_lines_inside_symbol_block() {
    let snapshot = "\
ABI_SNAPSHOT_V1
ELF_CLASS=ELF64
ELF_MACHINE=Advanced Micro Devices X86-64
SYMBOLS:
memcpy
ELF_CLASS=ELF32
";
    let error = parse_snapshot(snapshot).expect_err("metadata line in SYMBOLS block must fail");

    assert!(error.contains("metadata field inside `SYMBOLS:` block"));
  }

  #[test]
  fn diff_against_golden_reports_all_regressions() {
    let expected = AbiSnapshot {
      class: "ELF64".to_string(),
      machine: "Advanced Micro Devices X86-64".to_string(),
      symbols: BTreeSet::from([
        "__errno_location".to_string(),
        "abort".to_string(),
        "memcpy".to_string(),
      ]),
    };
    let actual = AbiSnapshot {
      class: "ELF64".to_string(),
      machine: "AArch64".to_string(),
      symbols: BTreeSet::from(["abort".to_string(), "puts".to_string()]),
    };
    let diff = diff_against_golden(&expected, &actual);

    assert_eq!(
      diff,
      SnapshotDiff {
        class_mismatch: None,
        machine_mismatch: Some((
          "Advanced Micro Devices X86-64".to_string(),
          "AArch64".to_string()
        )),
        missing_symbols: vec!["__errno_location".to_string(), "memcpy".to_string()],
        unexpected_symbols: vec!["puts".to_string()],
      },
    );
  }

  #[test]
  fn required_symbols_are_covered_by_golden_snapshot() {
    let golden = include_str!("../../abi/golden/x86_64-unknown-linux-gnu.abi");
    let snapshot = parse_snapshot(golden).expect("golden snapshot must parse");

    for required in REQUIRED_SYMBOLS {
      assert!(
        snapshot.symbols.contains(*required),
        "golden snapshot must contain required symbol `{required}`",
      );
    }
  }
}
