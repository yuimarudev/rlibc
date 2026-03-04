use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::{env, fs};

const DEFAULT_OUTPUT_DIR: &str = "target/release/crt";
const DEFAULT_COMPILER: &str = "cc";
const CRT_OBJECTS: [CrtObjectSpec; 4] = [
  CrtObjectSpec {
    source: "crt1.S",
    output: "crt1.o",
  },
  CrtObjectSpec {
    source: "Scrt1.S",
    output: "Scrt1.o",
  },
  CrtObjectSpec {
    source: "crti.S",
    output: "crti.o",
  },
  CrtObjectSpec {
    source: "crtn.S",
    output: "crtn.o",
  },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
  compiler: String,
  out_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CrtObjectSpec {
  source: &'static str,
  output: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
  Build(Config),
  Help,
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
  match parse_args(args)? {
    Action::Build(config) => build_crt_objects(&config),
    Action::Help => {
      print_usage();
      Ok(())
    }
  }
}

fn default_compiler_from_env() -> String {
  resolve_default_compiler(env::var("CC").ok())
}

fn resolve_default_compiler(env_cc_value: Option<String>) -> String {
  match env_cc_value {
    Some(value) if !is_blank_value(&value) => value,
    _ => DEFAULT_COMPILER.to_string(),
  }
}

fn parse_args(args: &[String]) -> Result<Action, String> {
  let compiler = default_compiler_from_env();
  let mut config = Config {
    compiler,
    out_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
  };
  let mut index = 0;

  while index < args.len() {
    match args[index].as_str() {
      "-h" | "--help" => return Ok(Action::Help),
      "--" => {
        let trailing_positionals = &args[(index + 1)..];

        if !trailing_positionals.is_empty() {
          return Err(format_unexpected_positionals(trailing_positionals));
        }

        break;
      }
      arg if arg.starts_with("--out-dir=") => {
        let value = parse_inline_option_value(arg, "--out-dir=")?;

        config.out_dir = PathBuf::from(value);
      }
      "--out-dir" => {
        let output = parse_option_value(args, index, "--out-dir")?;
        let next_index = index + 1;

        config.out_dir = PathBuf::from(output);
        index = next_index;
      }
      arg if arg.starts_with("--cc=") => {
        let value = parse_inline_option_value(arg, "--cc=")?;

        config.compiler = value.to_string();
      }
      "--cc" => {
        let compiler = parse_option_value(args, index, "--cc")?;
        let next_index = index + 1;

        config.compiler = compiler.to_string();
        index = next_index;
      }
      unknown => {
        if is_option_token(unknown) {
          return Err(format!("unknown argument: {unknown}"));
        }

        let positionals = trailing_non_option_positionals(args, index);

        return Err(format_unexpected_positionals(positionals));
      }
    }

    index += 1;
  }

  Ok(Action::Build(config))
}

fn parse_inline_option_value<'arg>(arg: &'arg str, prefix: &str) -> Result<&'arg str, String> {
  let option_name = prefix.trim_end_matches('=');
  let value = arg
    .strip_prefix(prefix)
    .ok_or_else(|| format!("missing value for {option_name}"))?;

  if is_blank_value(value) {
    return Err(format!("missing value for {option_name}"));
  }

  Ok(value)
}

fn parse_option_value<'args>(
  args: &'args [String],
  index: usize,
  option_name: &str,
) -> Result<&'args str, String> {
  let value = args
    .get(index + 1)
    .ok_or_else(|| format!("missing value for {option_name}"))?;

  if is_blank_value(value) || is_option_token(value) {
    return Err(format!("missing value for {option_name}"));
  }

  Ok(value)
}

fn is_blank_value(value: &str) -> bool {
  value.trim().is_empty()
}

fn is_option_token(value: &str) -> bool {
  value.starts_with('-')
}

fn trailing_non_option_positionals(args: &[String], start_index: usize) -> &[String] {
  let mut end = start_index;

  while end < args.len() && !is_option_token(&args[end]) {
    end += 1;
  }

  &args[start_index..end]
}

fn format_unexpected_positionals(positionals: &[String]) -> String {
  match positionals {
    [single] => format!("unexpected positional argument: {single}"),
    [first, second] => format!("unexpected positional arguments: {first}, {second}"),
    [first, second, third] => {
      format!("unexpected positional arguments: {first}, {second}, {third}")
    }
    _ => {
      let displayed = positionals[..3].join(", ");
      let remaining = positionals.len() - 3;

      format!("unexpected positional arguments: {displayed} (+{remaining} more)")
    }
  }
}

fn build_crt_objects(config: &Config) -> Result<(), String> {
  fs::create_dir_all(&config.out_dir).map_err(|error| {
    format!(
      "failed to create output directory {}: {error}",
      config.out_dir.display()
    )
  })?;

  for spec in CRT_OBJECTS {
    build_one_object(config, &spec)?;
  }

  Ok(())
}

fn build_one_object(config: &Config, spec: &CrtObjectSpec) -> Result<(), String> {
  let source_path = crt_source_root().join(spec.source);

  ensure_source_file(&source_path)?;

  let output_path = config.out_dir.join(spec.output);
  let output = Command::new(&config.compiler)
    .arg("-c")
    .arg("-nostdlib")
    .arg("-ffreestanding")
    .arg("-fno-stack-protector")
    .arg("-o")
    .arg(&output_path)
    .arg(&source_path)
    .output()
    .map_err(|error| {
      format!(
        "failed to execute {} for {}: {error}",
        config.compiler,
        source_path.display(),
      )
    })?;

  if !output.status.success() {
    return Err(format!(
      "{} failed for {} (status {}): {}",
      config.compiler,
      source_path.display(),
      output.status,
      String::from_utf8_lossy(&output.stderr),
    ));
  }

  println!("generated {}", output_path.display());

  Ok(())
}

fn crt_source_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/crt/asm")
}

fn ensure_source_file(path: &Path) -> Result<(), String> {
  let metadata = fs::metadata(path)
    .map_err(|error| format!("crt source file is missing at {}: {error}", path.display()))?;

  if !metadata.is_file() {
    return Err(format!("crt source path is not a file: {}", path.display()));
  }

  if metadata.len() == 0 {
    return Err(format!("crt source file is empty: {}", path.display()));
  }

  Ok(())
}

fn print_usage() {
  println!("Usage: cargo run --release --bin crt_pipeline -- [--out-dir PATH] [--cc PATH]");
  println!("  --out-dir PATH  Output directory for crt objects (default: {DEFAULT_OUTPUT_DIR})");
  println!("  --cc PATH       C compiler executable (default: $CC or {DEFAULT_COMPILER})");
}

#[cfg(test)]
mod tests {
  use super::{
    Action, Config, DEFAULT_COMPILER, DEFAULT_OUTPUT_DIR, crt_source_root, ensure_source_file,
    parse_args, resolve_default_compiler,
  };
  use std::path::{Path, PathBuf};
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::time::{SystemTime, UNIX_EPOCH};
  use std::{env, fs};

  static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

  struct TempDirectory {
    path: PathBuf,
  }

  impl TempDirectory {
    fn create() -> Self {
      let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos();
      let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
      let path = env::temp_dir().join(format!("rlibc-i007-unit-{timestamp}-{counter}"));

      fs::create_dir_all(&path).expect("failed to create temporary unit-test directory");

      Self { path }
    }

    fn path(&self) -> &Path {
      &self.path
    }
  }

  impl Drop for TempDirectory {
    fn drop(&mut self) {
      let _result = fs::remove_dir_all(&self.path);
    }
  }

  #[test]
  fn parse_args_uses_defaults() {
    let action = parse_args(&[]).expect("default args should parse");

    match action {
      Action::Build(Config { compiler, out_dir }) => {
        assert!(!compiler.is_empty(), "compiler should not be empty");
        assert_eq!(out_dir, PathBuf::from(DEFAULT_OUTPUT_DIR));
      }
      Action::Help => panic!("defaults should not resolve to help action"),
    }
  }

  #[test]
  fn parse_args_supports_out_dir_and_compiler_overrides() {
    let args = vec![
      "--out-dir".to_string(),
      "/tmp/custom-crt".to_string(),
      "--cc".to_string(),
      "clang".to_string(),
    ];
    let action = parse_args(&args).expect("overrides should parse");

    assert_eq!(
      action,
      Action::Build(Config {
        compiler: "clang".to_string(),
        out_dir: PathBuf::from("/tmp/custom-crt"),
      }),
    );
  }

  #[test]
  fn parse_args_supports_help() {
    let action = parse_args(&["--help".to_string()]).expect("help should parse");

    assert!(matches!(action, Action::Help));
  }

  #[test]
  fn parse_args_long_help_before_bare_positional_returns_help() {
    let action = parse_args(&["--help".to_string(), "positional".to_string()])
      .expect("long help before bare positional must parse as help");

    assert!(matches!(action, Action::Help));
  }

  #[test]
  fn parse_args_short_help_before_bare_positional_returns_help() {
    let action = parse_args(&["-h".to_string(), "positional".to_string()])
      .expect("short help before bare positional must parse as help");

    assert!(matches!(action, Action::Help));
  }

  #[test]
  fn parse_args_long_help_before_double_dash_with_trailing_positional_returns_help() {
    let action = parse_args(&[
      "--help".to_string(),
      "--".to_string(),
      "trailing-positional".to_string(),
    ])
    .expect("long help before double dash must parse as help");

    assert!(matches!(action, Action::Help));
  }

  #[test]
  fn parse_args_short_help_before_double_dash_with_trailing_positional_returns_help() {
    let action = parse_args(&["-h".to_string(), "--".to_string(), "tail".to_string()])
      .expect("short help before double dash must parse as help");

    assert!(matches!(action, Action::Help));
  }

  #[test]
  fn parse_args_requires_values_for_flag_arguments() {
    let error = parse_args(&["--out-dir".to_string()]).expect_err("missing value must fail");

    assert!(error.contains("--out-dir"));
  }

  #[test]
  fn parse_args_rejects_flag_token_as_out_dir_value() {
    let args = vec![
      "--out-dir".to_string(),
      "--cc".to_string(),
      "clang".to_string(),
    ];
    let error = parse_args(&args).expect_err("flag token as out-dir value must fail");

    assert_eq!(error, "missing value for --out-dir");
  }

  #[test]
  fn parse_args_rejects_unknown_option_token_as_out_dir_value() {
    let args = vec!["--out-dir".to_string(), "--bogus-option".to_string()];
    let error = parse_args(&args).expect_err("unknown option token as out-dir value must fail");

    assert_eq!(error, "missing value for --out-dir");
  }

  #[test]
  fn parse_args_rejects_flag_token_as_cc_value() {
    let args = vec![
      "--cc".to_string(),
      "--out-dir".to_string(),
      "/tmp/out".to_string(),
    ];
    let error = parse_args(&args).expect_err("flag token as cc value must fail");

    assert_eq!(error, "missing value for --cc");
  }

  #[test]
  fn parse_args_rejects_unknown_option_token_as_cc_value() {
    let args = vec!["--cc".to_string(), "--bogus-option".to_string()];
    let error = parse_args(&args).expect_err("unknown option token as cc value must fail");

    assert_eq!(error, "missing value for --cc");
  }

  #[test]
  fn parse_args_supports_equals_style_values() {
    let args = vec![
      "--out-dir=/tmp/crt-equals".to_string(),
      "--cc=clang".to_string(),
    ];
    let action = parse_args(&args).expect("equals-style values should parse");

    assert_eq!(
      action,
      Action::Build(Config {
        compiler: "clang".to_string(),
        out_dir: PathBuf::from("/tmp/crt-equals"),
      }),
    );
  }

  #[test]
  fn parse_args_rejects_missing_out_dir_equals_value() {
    let args = vec!["--out-dir=".to_string()];
    let error = parse_args(&args).expect_err("missing equals value for out-dir must fail");

    assert_eq!(error, "missing value for --out-dir");
  }

  #[test]
  fn parse_args_rejects_missing_cc_equals_value() {
    let args = vec!["--cc=".to_string()];
    let error = parse_args(&args).expect_err("missing equals value for cc must fail");

    assert_eq!(error, "missing value for --cc");
  }

  #[test]
  fn parse_args_rejects_blank_out_dir_equals_value() {
    let args = vec!["--out-dir=   ".to_string()];
    let error = parse_args(&args).expect_err("blank equals out-dir value must fail");

    assert_eq!(error, "missing value for --out-dir");
  }

  #[test]
  fn parse_args_rejects_blank_cc_equals_value() {
    let args = vec!["--cc=   ".to_string()];
    let error = parse_args(&args).expect_err("blank equals cc value must fail");

    assert_eq!(error, "missing value for --cc");
  }

  #[test]
  fn parse_args_accepts_option_like_out_dir_equals_value() {
    let args = vec!["--out-dir=--bogus-option".to_string()];
    let action = parse_args(&args).expect("option-like out-dir equals value should parse");
    let expected_compiler = std::env::var("CC").unwrap_or_else(|_| DEFAULT_COMPILER.to_string());

    assert_eq!(
      action,
      Action::Build(Config {
        compiler: expected_compiler,
        out_dir: PathBuf::from("--bogus-option"),
      }),
    );
  }

  #[test]
  fn parse_args_accepts_option_like_cc_equals_value() {
    let args = vec!["--cc=--bogus-option".to_string()];
    let action = parse_args(&args).expect("option-like cc equals value should parse");

    assert_eq!(
      action,
      Action::Build(Config {
        compiler: "--bogus-option".to_string(),
        out_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
      }),
    );
  }

  #[test]
  fn parse_args_rejects_empty_out_dir_value() {
    let args = vec!["--out-dir".to_string(), String::new()];
    let error = parse_args(&args).expect_err("empty out-dir value must fail");

    assert_eq!(error, "missing value for --out-dir");
  }

  #[test]
  fn parse_args_rejects_blank_out_dir_value() {
    let args = vec!["--out-dir".to_string(), "   ".to_string()];
    let error = parse_args(&args).expect_err("blank out-dir value must fail");

    assert_eq!(error, "missing value for --out-dir");
  }

  #[test]
  fn parse_args_rejects_empty_cc_value() {
    let args = vec!["--cc".to_string(), String::new()];
    let error = parse_args(&args).expect_err("empty compiler value must fail");

    assert_eq!(error, "missing value for --cc");
  }

  #[test]
  fn parse_args_rejects_blank_cc_value() {
    let args = vec!["--cc".to_string(), "   ".to_string()];
    let error = parse_args(&args).expect_err("blank compiler value must fail");

    assert_eq!(error, "missing value for --cc");
  }

  #[test]
  fn parse_args_rejects_unknown_flags() {
    let error = parse_args(&["--target".to_string()]).expect_err("unknown flag must fail");

    assert!(error.contains("unknown argument"));
  }

  #[test]
  fn parse_args_rejects_bare_positional_argument() {
    let error =
      parse_args(&["positional".to_string()]).expect_err("bare positional argument must fail");

    assert_eq!(error, "unexpected positional argument: positional");
  }

  #[test]
  fn parse_args_positional_before_long_help_still_fails_as_positional() {
    let error = parse_args(&["positional".to_string(), "--help".to_string()])
      .expect_err("positional before long help must fail as positional");

    assert_eq!(error, "unexpected positional argument: positional");
  }

  #[test]
  fn parse_args_positional_before_short_help_still_fails_as_positional() {
    let error = parse_args(&["positional".to_string(), "-h".to_string()])
      .expect_err("positional before short help must fail as positional");

    assert_eq!(error, "unexpected positional argument: positional");
  }

  #[test]
  fn parse_args_rejects_multiple_bare_positional_arguments() {
    let error = parse_args(&["first".to_string(), "second".to_string()])
      .expect_err("multiple bare positional arguments must fail");

    assert_eq!(error, "unexpected positional arguments: first, second");
  }

  #[test]
  fn parse_args_rejects_three_bare_positional_arguments_without_summary_suffix() {
    let error = parse_args(&[
      "first".to_string(),
      "second".to_string(),
      "third".to_string(),
    ])
    .expect_err("three bare positional arguments must fail without summary suffix");

    assert_eq!(
      error,
      "unexpected positional arguments: first, second, third"
    );
  }

  #[test]
  fn parse_args_summarizes_many_bare_positional_arguments() {
    let error = parse_args(&[
      "first".to_string(),
      "second".to_string(),
      "third".to_string(),
      "fourth".to_string(),
    ])
    .expect_err("many bare positional arguments must fail");

    assert_eq!(
      error,
      "unexpected positional arguments: first, second, third (+1 more)"
    );
  }

  #[test]
  fn parse_args_accepts_double_dash_without_trailing_arguments() {
    let action = parse_args(&["--".to_string()]).expect("double dash should parse");
    let Action::Build(config) = action else {
      panic!("double dash without trailing args must produce build action");
    };
    let expected_compiler = std::env::var("CC").unwrap_or_else(|_| DEFAULT_COMPILER.to_string());

    assert_eq!(config.compiler, expected_compiler);
    assert_eq!(config.out_dir, PathBuf::from(DEFAULT_OUTPUT_DIR));
  }

  #[test]
  fn parse_args_rejects_trailing_positional_after_double_dash() {
    let error = parse_args(&["--".to_string(), "extra".to_string()])
      .expect_err("trailing positional after double dash must fail");

    assert_eq!(error, "unexpected positional argument: extra");
  }

  #[test]
  fn parse_args_rejects_multiple_trailing_positionals_after_double_dash() {
    let error = parse_args(&["--".to_string(), "extra".to_string(), "more".to_string()])
      .expect_err("multiple trailing positionals after double dash must fail");

    assert_eq!(error, "unexpected positional arguments: extra, more");
  }

  #[test]
  fn parse_args_rejects_three_trailing_positionals_after_double_dash_without_summary_suffix() {
    let error = parse_args(&[
      "--".to_string(),
      "extra".to_string(),
      "more".to_string(),
      "overflow".to_string(),
    ])
    .expect_err("three trailing positionals after double dash must fail without summary suffix");

    assert_eq!(
      error,
      "unexpected positional arguments: extra, more, overflow"
    );
  }

  #[test]
  fn parse_args_summarizes_many_trailing_positionals_after_double_dash() {
    let error = parse_args(&[
      "--".to_string(),
      "extra".to_string(),
      "more".to_string(),
      "overflow".to_string(),
      "tail".to_string(),
    ])
    .expect_err("many trailing positionals after double dash must fail");

    assert_eq!(
      error,
      "unexpected positional arguments: extra, more, overflow (+1 more)"
    );
  }

  #[test]
  fn parse_args_rejects_option_like_token_after_double_dash_as_positional() {
    let error = parse_args(&["--".to_string(), "--help".to_string()])
      .expect_err("option-like token after double dash must be treated as positional");

    assert_eq!(error, "unexpected positional argument: --help");
  }

  #[test]
  fn parse_args_rejects_short_help_token_after_double_dash_as_positional() {
    let error = parse_args(&["--".to_string(), "-h".to_string()])
      .expect_err("short help token after double dash must be treated as positional");

    assert_eq!(error, "unexpected positional argument: -h");
  }

  #[test]
  fn parse_args_summarizes_many_option_like_positionals_after_double_dash() {
    let error = parse_args(&[
      "--".to_string(),
      "--help".to_string(),
      "--cc".to_string(),
      "--out-dir".to_string(),
      "--unknown".to_string(),
    ])
    .expect_err("many option-like tokens after double dash must be summarized");

    assert_eq!(
      error,
      "unexpected positional arguments: --help, --cc, --out-dir (+1 more)"
    );
  }

  #[test]
  fn parse_args_summarizes_many_mixed_help_tokens_after_double_dash() {
    let error = parse_args(&[
      "--".to_string(),
      "-h".to_string(),
      "--help".to_string(),
      "--cc".to_string(),
      "--out-dir".to_string(),
    ])
    .expect_err("many mixed help tokens after double dash must be summarized");

    assert_eq!(
      error,
      "unexpected positional arguments: -h, --help, --cc (+1 more)"
    );
  }

  #[test]
  fn parse_args_accepts_double_dash_after_overrides() {
    let args = vec![
      "--out-dir".to_string(),
      "/tmp/crt-double-dash-overrides".to_string(),
      "--cc".to_string(),
      "clang".to_string(),
      "--".to_string(),
    ];
    let action = parse_args(&args).expect("double dash after overrides should parse");

    assert_eq!(
      action,
      Action::Build(Config {
        compiler: "clang".to_string(),
        out_dir: PathBuf::from("/tmp/crt-double-dash-overrides"),
      }),
    );
  }

  #[test]
  fn parse_args_falls_back_to_default_compiler_when_cc_is_unset() {
    let action = parse_args(&[]).expect("default args should parse");
    let Action::Build(config) = action else {
      panic!("default args must produce build action");
    };

    if std::env::var("CC").is_err() {
      assert_eq!(config.compiler, DEFAULT_COMPILER);
    }
  }

  #[test]
  fn resolve_default_compiler_prefers_non_blank_env_value() {
    let resolved = resolve_default_compiler(Some("clang".to_string()));

    assert_eq!(resolved, "clang");
  }

  #[test]
  fn resolve_default_compiler_falls_back_for_blank_or_missing_env_value() {
    assert_eq!(resolve_default_compiler(None), DEFAULT_COMPILER);
    assert_eq!(
      resolve_default_compiler(Some(String::new())),
      DEFAULT_COMPILER
    );
    assert_eq!(
      resolve_default_compiler(Some("   ".to_string())),
      DEFAULT_COMPILER
    );
  }

  #[test]
  fn crt_source_root_points_to_existing_assembly_directory() {
    let source_root = crt_source_root();

    assert!(
      source_root.is_absolute(),
      "crt source root should be absolute for cwd-independent invocation: {}",
      source_root.display()
    );
    assert!(
      source_root.is_dir(),
      "crt source root should exist as a directory"
    );
    assert!(
      source_root.join("crt1.S").is_file(),
      "crt1.S should exist under source root"
    );
    assert!(
      source_root.join("Scrt1.S").is_file(),
      "Scrt1.S should exist under source root"
    );
    assert!(
      source_root.join("crti.S").is_file(),
      "crti.S should exist under source root"
    );
    assert!(
      source_root.join("crtn.S").is_file(),
      "crtn.S should exist under source root"
    );
  }

  #[test]
  fn ensure_source_file_accepts_non_empty_regular_file() {
    let dir = TempDirectory::create();
    let source_path = dir.path().join("crt1.S");

    fs::write(&source_path, ".text\n.globl _start\n_start:\n  ret\n")
      .expect("failed to write sample source file");

    assert!(ensure_source_file(&source_path).is_ok());
  }

  #[test]
  fn ensure_source_file_rejects_missing_source_file() {
    let dir = TempDirectory::create();
    let missing_source = dir.path().join("missing.S");
    let error = ensure_source_file(&missing_source).expect_err("missing source must fail");

    assert!(error.contains("missing"));
  }

  #[test]
  fn ensure_source_file_rejects_empty_source_file() {
    let dir = TempDirectory::create();
    let source_path = dir.path().join("empty.S");

    fs::write(&source_path, "").expect("failed to create empty source file");

    let error = ensure_source_file(&source_path).expect_err("empty source must fail");

    assert!(error.contains("empty"));
  }
}
