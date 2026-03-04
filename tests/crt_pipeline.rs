#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, fs, thread};

const EXPECTED_OBJECTS: [&str; 4] = ["crt1.o", "Scrt1.o", "crti.o", "crtn.o"];
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
    let path = env::temp_dir().join(format!("rlibc-i007-crt-{timestamp}-{counter}"));

    fs::create_dir_all(&path).expect("failed to create temp directory for crt pipeline test");

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

fn pipeline_binary() -> PathBuf {
  let binary = env::var("CARGO_BIN_EXE_crt_pipeline")
    .expect("cargo must provide CARGO_BIN_EXE_crt_pipeline for integration tests");

  PathBuf::from(binary)
}

fn object_path(output_dir: &Path, name: &str) -> PathBuf {
  output_dir.join(name)
}

fn run_pipeline(args: &[String]) -> Output {
  let mut command = Command::new(pipeline_binary());

  for arg in args {
    command.arg(arg);
  }

  run_pipeline_command(&mut command, "failed to execute crt_pipeline binary")
}

fn run_pipeline_in_dir(args: &[String], working_dir: &Path) -> Output {
  let mut command = Command::new(pipeline_binary());

  command.current_dir(working_dir);

  for arg in args {
    command.arg(arg);
  }

  run_pipeline_command(
    &mut command,
    "failed to execute crt_pipeline binary in custom directory",
  )
}

fn run_pipeline_with_cc_env(args: &[String], cc_value: &str) -> Output {
  let mut command = Command::new(pipeline_binary());

  command.env("CC", cc_value);

  for arg in args {
    command.arg(arg);
  }

  run_pipeline_command(&mut command, "failed to execute crt_pipeline binary")
}

fn run_pipeline_without_cc_env(args: &[String]) -> Output {
  let mut command = Command::new(pipeline_binary());

  command.env_remove("CC");

  for arg in args {
    command.arg(arg);
  }

  run_pipeline_command(&mut command, "failed to execute crt_pipeline binary")
}

fn run_pipeline_command(command: &mut Command, context: &str) -> Output {
  const MAX_NOT_FOUND_RETRIES: u8 = 2;

  for attempt in 0..=MAX_NOT_FOUND_RETRIES {
    match command.output() {
      Ok(output) => return output,
      Err(error) => {
        let should_retry = error.kind() == std::io::ErrorKind::NotFound;

        if should_retry && attempt < MAX_NOT_FOUND_RETRIES {
          thread::sleep(Duration::from_millis(50));
          continue;
        }

        panic!("{context}: {error}");
      }
    }
  }

  unreachable!("retry loop must always return or panic");
}

fn assert_pipeline_success(output: &Output) {
  assert!(
    output.status.success(),
    "crt_pipeline failed with status {:?}, stdout={:?}, stderr={:?}",
    output.status,
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

fn assert_pipeline_failure_contains(output: &Output, message: &str) {
  assert!(
    !output.status.success(),
    "crt_pipeline unexpectedly succeeded: stdout={:?}, stderr={:?}",
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );

  let stderr_text = String::from_utf8_lossy(&output.stderr);

  assert!(
    stderr_text.contains(message),
    "expected stderr to contain {message:?}, got {stderr_text:?}",
  );
}

fn assert_pipeline_stdout_contains(output: &Output, message: &str) {
  let stdout_text = String::from_utf8_lossy(&output.stdout);

  assert!(
    stdout_text.contains(message),
    "expected stdout to contain {message:?}, got {stdout_text:?}",
  );
}

fn assert_expected_objects(output_dir: &Path) {
  for object_name in EXPECTED_OBJECTS {
    let path = object_path(output_dir, object_name);
    let metadata = fs::metadata(&path)
      .unwrap_or_else(|error| panic!("missing expected crt object at {}: {error}", path.display()));

    assert!(
      metadata.is_file(),
      "crt object path must be a file: {}",
      path.display()
    );
    assert!(
      metadata.len() > 0,
      "crt object must not be empty: {}",
      path.display()
    );
  }
}

fn assert_no_expected_objects(output_dir: &Path) {
  for object_name in EXPECTED_OBJECTS {
    let path = object_path(output_dir, object_name);

    assert!(
      fs::metadata(&path).is_err(),
      "crt object should not be generated when build is skipped: {}",
      path.display()
    );
  }
}

#[test]
fn crt_pipeline_emits_all_required_crt_objects() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_emits_all_required_crt_objects_with_equals_out_dir_option() {
  let output_dir = TempDirectory::create();
  let args = vec![format!("--out-dir={}", output_dir.path().display())];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_builds_from_non_repo_working_directory_with_explicit_out_dir() {
  let external_working_dir = TempDirectory::create();
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
  ];
  let output = run_pipeline_in_dir(&args, external_working_dir.path());

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_builds_default_output_dir_from_non_repo_working_directory() {
  let external_working_dir = TempDirectory::create();
  let args: Vec<String> = Vec::new();
  let output = run_pipeline_in_dir(&args, external_working_dir.path());
  let default_output_dir = external_working_dir.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_expected_objects(&default_output_dir);
}

#[test]
fn crt_pipeline_rejects_blank_out_dir_equals_value() {
  let args = vec!["--out-dir=   ".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "missing value for --out-dir");
}

#[test]
fn crt_pipeline_rejects_blank_cc_split_value() {
  let args = vec!["--cc".to_string(), "   ".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "missing value for --cc");
}

#[test]
fn crt_pipeline_rejects_blank_cc_equals_value() {
  let args = vec!["--cc=   ".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "missing value for --cc");
}

#[test]
fn crt_pipeline_rejects_unknown_argument() {
  let args = vec!["--unknown-option".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unknown argument: --unknown-option");
}

#[test]
fn crt_pipeline_rejects_bare_positional_argument() {
  let args = vec!["positional".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unexpected positional argument: positional");
}

#[test]
fn crt_pipeline_positional_before_long_help_still_fails_as_positional() {
  let args = vec!["positional".to_string(), "--help".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unexpected positional argument: positional");
}

#[test]
fn crt_pipeline_positional_before_short_help_still_fails_as_positional() {
  let args = vec!["positional".to_string(), "-h".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unexpected positional argument: positional");
}

#[test]
fn crt_pipeline_rejects_multiple_bare_positional_arguments() {
  let args = vec!["first".to_string(), "second".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unexpected positional arguments: first, second");
}

#[test]
fn crt_pipeline_rejects_three_bare_positional_arguments_without_summary_suffix() {
  let args = vec![
    "first".to_string(),
    "second".to_string(),
    "third".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    "unexpected positional arguments: first, second, third",
  );
}

#[test]
fn crt_pipeline_summarizes_many_bare_positional_arguments() {
  let args = vec![
    "first".to_string(),
    "second".to_string(),
    "third".to_string(),
    "fourth".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    "unexpected positional arguments: first, second, third (+1 more)",
  );
}

#[test]
fn crt_pipeline_uses_default_compiler_when_cc_env_is_blank() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
  ];
  let output = run_pipeline_with_cc_env(&args, "   ");

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_default_compiler_when_cc_env_is_unset() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
  ];
  let output = run_pipeline_without_cc_env(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_reports_execution_error_for_invalid_cc_env() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
  ];
  let output = run_pipeline_with_cc_env(&args, "/definitely/not/a/compiler/rlibc-i007");

  assert_pipeline_failure_contains(&output, "failed to execute");
}

#[test]
fn crt_pipeline_prefers_cli_cc_over_invalid_cc_env() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc".to_string(),
    "cc".to_string(),
  ];
  let output = run_pipeline_with_cc_env(&args, "/definitely/not/a/compiler/rlibc-i007");

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_reports_execution_error_for_invalid_cli_cc() {
  let output_dir = TempDirectory::create();
  let invalid_compiler = "/definitely/not/a/compiler/rlibc-i007-cli";
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc".to_string(),
    invalid_compiler.to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    &format!("failed to execute {invalid_compiler} for"),
  );
}

#[test]
fn crt_pipeline_rejects_out_dir_when_path_is_a_file() {
  let sandbox = TempDirectory::create();
  let out_file = sandbox.path().join("not-a-directory");

  fs::write(&out_file, "x").expect("failed to create file used as invalid out-dir target");

  let args = vec!["--out-dir".to_string(), out_file.display().to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "failed to create output directory");
}

#[test]
fn crt_pipeline_reports_execution_error_for_invalid_equals_style_cli_cc() {
  let output_dir = TempDirectory::create();
  let invalid_compiler = "/definitely/not/a/compiler/rlibc-i007-equals";
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    format!("--cc={invalid_compiler}"),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    &format!("failed to execute {invalid_compiler} for"),
  );
}

#[test]
fn crt_pipeline_prefers_equals_style_cli_cc_over_invalid_cc_env() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline_with_cc_env(&args, "/definitely/not/a/compiler/rlibc-i007-env");

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_argument_when_repeated() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-first".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_argument_even_when_invalid() {
  let output_dir = TempDirectory::create();
  let invalid_compiler = "/definitely/not/a/compiler/rlibc-i007-last";
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=cc".to_string(),
    format!("--cc={invalid_compiler}"),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    &format!("failed to execute {invalid_compiler} for"),
  );
}

#[test]
fn crt_pipeline_uses_last_cc_argument_when_repeated_with_mixed_styles() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-mixed-first".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_argument_even_when_invalid_with_mixed_styles() {
  let output_dir = TempDirectory::create();
  let invalid_compiler = "/definitely/not/a/compiler/rlibc-i007-mixed-last";
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=cc".to_string(),
    "--cc".to_string(),
    invalid_compiler.to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    &format!("failed to execute {invalid_compiler} for"),
  );
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_when_repeated_with_mixed_styles() {
  let sandbox = TempDirectory::create();
  let first_out_file = sandbox.path().join("first-not-dir");

  fs::write(&first_out_file, "x").expect("failed to create invalid first out-dir file");

  let final_output_dir = TempDirectory::create();
  let args = vec![
    format!("--out-dir={}", first_out_file.display()),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_even_when_invalid_with_mixed_styles() {
  let first_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let last_out_file = sandbox.path().join("last-not-dir");

  fs::write(&last_out_file, "x").expect("failed to create invalid last out-dir file");

  let args = vec![
    "--out-dir".to_string(),
    first_output_dir.path().display().to_string(),
    format!("--out-dir={}", last_out_file.display()),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "failed to create output directory");
}

#[test]
fn crt_pipeline_uses_last_cc_argument_with_three_mixed_occurrences() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-three-1".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-three-2".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_with_three_mixed_occurrences() {
  let first_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let second_out_file = sandbox.path().join("second-not-dir");

  fs::write(&second_out_file, "x").expect("failed to create invalid second out-dir file");

  let final_output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    first_output_dir.path().display().to_string(),
    format!("--out-dir={}", second_out_file.display()),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_prints_usage_for_long_help_flag() {
  let args = vec!["--help".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
}

#[test]
fn crt_pipeline_prints_usage_for_short_help_flag() {
  let args = vec!["-h".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
}

#[test]
fn crt_pipeline_help_flag_takes_precedence_when_before_unknown_argument() {
  let args = vec!["--help".to_string(), "--unknown-option".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
}

#[test]
fn crt_pipeline_unknown_argument_before_help_still_fails() {
  let args = vec!["--unknown-option".to_string(), "--help".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unknown argument: --unknown-option");
}

#[test]
fn crt_pipeline_short_help_flag_takes_precedence_when_before_unknown_argument() {
  let args = vec!["-h".to_string(), "--unknown-option".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
}

#[test]
fn crt_pipeline_unknown_argument_before_short_help_still_fails() {
  let args = vec!["--unknown-option".to_string(), "-h".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unknown argument: --unknown-option");
}

#[test]
fn crt_pipeline_double_dash_without_trailing_args_builds_default_output_dir() {
  let sandbox = TempDirectory::create();
  let args = vec!["--".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_expected_objects(&default_output_dir);
}

#[test]
fn crt_pipeline_rejects_trailing_positional_after_double_dash() {
  let args = vec!["--".to_string(), "extra".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unexpected positional argument: extra");
}

#[test]
fn crt_pipeline_rejects_multiple_trailing_positionals_after_double_dash() {
  let args = vec!["--".to_string(), "extra".to_string(), "more".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unexpected positional arguments: extra, more");
}

#[test]
fn crt_pipeline_rejects_three_trailing_positionals_after_double_dash_without_summary_suffix() {
  let args = vec![
    "--".to_string(),
    "extra".to_string(),
    "more".to_string(),
    "overflow".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    "unexpected positional arguments: extra, more, overflow",
  );
}

#[test]
fn crt_pipeline_summarizes_many_trailing_positionals_after_double_dash() {
  let args = vec![
    "--".to_string(),
    "extra".to_string(),
    "more".to_string(),
    "overflow".to_string(),
    "tail".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    "unexpected positional arguments: extra, more, overflow (+1 more)",
  );
}

#[test]
fn crt_pipeline_rejects_option_like_token_after_double_dash_as_positional() {
  let args = vec!["--".to_string(), "--help".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unexpected positional argument: --help");
}

#[test]
fn crt_pipeline_rejects_short_help_token_after_double_dash_as_positional() {
  let args = vec!["--".to_string(), "-h".to_string()];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "unexpected positional argument: -h");
}

#[test]
fn crt_pipeline_rejects_three_option_like_tokens_after_double_dash_without_summary_suffix() {
  let args = vec![
    "--".to_string(),
    "--help".to_string(),
    "--cc".to_string(),
    "--out-dir".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    "unexpected positional arguments: --help, --cc, --out-dir",
  );
}

#[test]
fn crt_pipeline_summarizes_many_option_like_positionals_after_double_dash() {
  let args = vec![
    "--".to_string(),
    "--help".to_string(),
    "--cc".to_string(),
    "--out-dir".to_string(),
    "--unknown".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    "unexpected positional arguments: --help, --cc, --out-dir (+1 more)",
  );
}

#[test]
fn crt_pipeline_summarizes_many_mixed_help_tokens_after_double_dash() {
  let args = vec![
    "--".to_string(),
    "-h".to_string(),
    "--help".to_string(),
    "--cc".to_string(),
    "--out-dir".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(
    &output,
    "unexpected positional arguments: -h, --help, --cc (+1 more)",
  );
}

#[test]
fn crt_pipeline_double_dash_after_overrides_builds_requested_output_dir() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc".to_string(),
    "cc".to_string(),
    "--".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_long_help_before_missing_cc_value_still_succeeds() {
  let sandbox = TempDirectory::create();
  let args = vec!["--help".to_string(), "--cc".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when trailing --cc is incomplete: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_short_help_before_missing_out_dir_value_still_succeeds() {
  let sandbox = TempDirectory::create();
  let args = vec!["-h".to_string(), "--out-dir".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when trailing --out-dir is incomplete: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_long_help_before_blank_cc_equals_value_still_succeeds() {
  let sandbox = TempDirectory::create();
  let args = vec!["--help".to_string(), "--cc=".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when trailing --cc= is blank: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_short_help_before_blank_out_dir_equals_value_still_succeeds() {
  let sandbox = TempDirectory::create();
  let args = vec!["-h".to_string(), "--out-dir=".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when trailing --out-dir= is blank: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_long_help_before_bare_positional_still_succeeds() {
  let sandbox = TempDirectory::create();
  let args = vec!["--help".to_string(), "positional".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when trailing positional follows: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_short_help_before_bare_positional_still_succeeds() {
  let sandbox = TempDirectory::create();
  let args = vec!["-h".to_string(), "positional".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when trailing positional follows: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_long_help_before_double_dash_with_trailing_positional_still_succeeds() {
  let sandbox = TempDirectory::create();
  let args = vec![
    "--help".to_string(),
    "--".to_string(),
    "trailing-positional".to_string(),
  ];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when -- and trailing positional follow: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_short_help_before_double_dash_with_trailing_positional_still_succeeds() {
  let sandbox = TempDirectory::create();
  let args = vec!["-h".to_string(), "--".to_string(), "tail".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when -- and trailing positional follow: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_long_help_skips_build_even_with_build_arguments() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--help".to_string(),
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc".to_string(),
    "cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert_no_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_short_help_skips_build_even_with_build_arguments() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "-h".to_string(),
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc".to_string(),
    "cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert_no_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_long_help_does_not_create_default_output_dir() {
  let sandbox = TempDirectory::create();
  let args = vec!["--help".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_short_help_does_not_create_default_output_dir() {
  let sandbox = TempDirectory::create();
  let args = vec!["-h".to_string()];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_long_help_does_not_create_explicit_relative_out_dir() {
  let sandbox = TempDirectory::create();
  let relative_out_dir = "explicit-help-out-long";
  let args = vec![
    "--help".to_string(),
    "--out-dir".to_string(),
    relative_out_dir.to_string(),
    "--cc".to_string(),
    "cc".to_string(),
  ];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let explicit_output_dir = sandbox.path().join(relative_out_dir);
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&explicit_output_dir).is_err(),
    "help mode should not create explicit output directory: {}",
    explicit_output_dir.display()
  );
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_short_help_does_not_create_explicit_relative_out_dir() {
  let sandbox = TempDirectory::create();
  let relative_out_dir = "explicit-help-out-short";
  let args = vec![
    "-h".to_string(),
    "--out-dir".to_string(),
    relative_out_dir.to_string(),
    "--cc".to_string(),
    "cc".to_string(),
  ];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let explicit_output_dir = sandbox.path().join(relative_out_dir);
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&explicit_output_dir).is_err(),
    "help mode should not create explicit output directory: {}",
    explicit_output_dir.display()
  );
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_long_help_after_build_args_does_not_create_explicit_relative_out_dir() {
  let sandbox = TempDirectory::create();
  let relative_out_dir = "explicit-help-out-long-after-build-args";
  let args = vec![
    "--out-dir".to_string(),
    relative_out_dir.to_string(),
    "--cc".to_string(),
    "cc".to_string(),
    "--help".to_string(),
  ];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let explicit_output_dir = sandbox.path().join(relative_out_dir);
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&explicit_output_dir).is_err(),
    "help mode should not create explicit output directory when help is late: {}",
    explicit_output_dir.display()
  );
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when help is late: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_short_help_after_build_args_does_not_create_explicit_relative_out_dir() {
  let sandbox = TempDirectory::create();
  let relative_out_dir = "explicit-help-out-short-after-build-args";
  let args = vec![
    "--out-dir".to_string(),
    relative_out_dir.to_string(),
    "--cc".to_string(),
    "cc".to_string(),
    "-h".to_string(),
  ];
  let output = run_pipeline_in_dir(&args, sandbox.path());
  let explicit_output_dir = sandbox.path().join(relative_out_dir);
  let default_output_dir = sandbox.path().join("target/release/crt");

  assert_pipeline_success(&output);
  assert_pipeline_stdout_contains(&output, "Usage: cargo run --release --bin crt_pipeline --");
  assert!(
    fs::metadata(&explicit_output_dir).is_err(),
    "help mode should not create explicit output directory when help is late: {}",
    explicit_output_dir.display()
  );
  assert!(
    fs::metadata(&default_output_dir).is_err(),
    "help mode should not create default output directory when help is late: {}",
    default_output_dir.display()
  );
}

#[test]
fn crt_pipeline_uses_last_cc_argument_with_four_mixed_occurrences() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-four-1".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-four-2".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-four-3".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_with_four_mixed_occurrences() {
  let first_output_dir = TempDirectory::create();
  let second_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let third_out_file = sandbox.path().join("third-not-dir");

  fs::write(&third_out_file, "x").expect("failed to create invalid third out-dir file");

  let final_output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    first_output_dir.path().display().to_string(),
    format!("--out-dir={}", second_output_dir.path().display()),
    format!("--out-dir={}", third_out_file.display()),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_argument_with_five_mixed_occurrences() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-five-1".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-five-2".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-five-3".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-five-4".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_with_five_mixed_occurrences() {
  let first_output_dir = TempDirectory::create();
  let second_output_dir = TempDirectory::create();
  let final_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let invalid_out_file1 = sandbox.path().join("invalid-1");
  let invalid_out_file2 = sandbox.path().join("invalid-2");

  fs::write(&invalid_out_file1, "x").expect("failed to create invalid first out-dir file");
  fs::write(&invalid_out_file2, "x").expect("failed to create invalid second out-dir file");

  let args = vec![
    "--out-dir".to_string(),
    first_output_dir.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file1.display()),
    "--out-dir".to_string(),
    second_output_dir.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file2.display()),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_argument_with_six_mixed_occurrences() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-six-1".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-six-2".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-six-3".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-six-4".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-six-5".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_with_six_mixed_occurrences() {
  let out1 = TempDirectory::create();
  let out2 = TempDirectory::create();
  let out3 = TempDirectory::create();
  let final_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let invalid_out_file1 = sandbox.path().join("invalid-six-1");
  let invalid_out_file2 = sandbox.path().join("invalid-six-2");

  fs::write(&invalid_out_file1, "x").expect("failed to create invalid out-dir file 1");
  fs::write(&invalid_out_file2, "x").expect("failed to create invalid out-dir file 2");

  let args = vec![
    "--out-dir".to_string(),
    out1.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file1.display()),
    "--out-dir".to_string(),
    out2.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file2.display()),
    "--out-dir".to_string(),
    out3.path().display().to_string(),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_argument_with_seven_mixed_occurrences() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-seven-1".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-seven-2".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-seven-3".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-seven-4".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-seven-5".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-seven-6".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_with_seven_mixed_occurrences() {
  let out1 = TempDirectory::create();
  let out2 = TempDirectory::create();
  let out3 = TempDirectory::create();
  let final_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let invalid_out_file1 = sandbox.path().join("invalid-seven-1");
  let invalid_out_file2 = sandbox.path().join("invalid-seven-2");
  let invalid_out_file3 = sandbox.path().join("invalid-seven-3");

  fs::write(&invalid_out_file1, "x").expect("failed to create invalid out-dir file 1");
  fs::write(&invalid_out_file2, "x").expect("failed to create invalid out-dir file 2");
  fs::write(&invalid_out_file3, "x").expect("failed to create invalid out-dir file 3");

  let args = vec![
    "--out-dir".to_string(),
    out1.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file1.display()),
    "--out-dir".to_string(),
    out2.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file2.display()),
    "--out-dir".to_string(),
    out3.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file3.display()),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_argument_with_eight_mixed_occurrences() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-eight-1".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-eight-2".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-eight-3".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-eight-4".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-eight-5".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-eight-6".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-eight-7".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_with_eight_mixed_occurrences() {
  let out1 = TempDirectory::create();
  let out2 = TempDirectory::create();
  let out3 = TempDirectory::create();
  let out4 = TempDirectory::create();
  let final_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let invalid_out_file1 = sandbox.path().join("invalid-eight-1");
  let invalid_out_file2 = sandbox.path().join("invalid-eight-2");
  let invalid_out_file3 = sandbox.path().join("invalid-eight-3");
  let invalid_out_file4 = sandbox.path().join("invalid-eight-4");

  fs::write(&invalid_out_file1, "x").expect("failed to create invalid out-dir file 1");
  fs::write(&invalid_out_file2, "x").expect("failed to create invalid out-dir file 2");
  fs::write(&invalid_out_file3, "x").expect("failed to create invalid out-dir file 3");
  fs::write(&invalid_out_file4, "x").expect("failed to create invalid out-dir file 4");

  let args = vec![
    "--out-dir".to_string(),
    out1.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file1.display()),
    "--out-dir".to_string(),
    out2.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file2.display()),
    "--out-dir".to_string(),
    out3.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file3.display()),
    "--out-dir".to_string(),
    out4.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file4.display()),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_argument_with_nine_mixed_occurrences() {
  let output_dir = TempDirectory::create();
  let args = vec![
    "--out-dir".to_string(),
    output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-nine-1".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-nine-2".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-nine-3".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-nine-4".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-nine-5".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-nine-6".to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-nine-7".to_string(),
    "--cc".to_string(),
    "/definitely/not/a/compiler/rlibc-i007-nine-8".to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_out_dir_argument_with_nine_mixed_occurrences() {
  let out1 = TempDirectory::create();
  let out2 = TempDirectory::create();
  let out3 = TempDirectory::create();
  let out4 = TempDirectory::create();
  let out5 = TempDirectory::create();
  let final_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let invalid_out_file1 = sandbox.path().join("invalid-nine-1");
  let invalid_out_file2 = sandbox.path().join("invalid-nine-2");
  let invalid_out_file3 = sandbox.path().join("invalid-nine-3");
  let invalid_out_file4 = sandbox.path().join("invalid-nine-4");
  let invalid_out_file5 = sandbox.path().join("invalid-nine-5");

  fs::write(&invalid_out_file1, "x").expect("failed to create invalid out-dir file 1");
  fs::write(&invalid_out_file2, "x").expect("failed to create invalid out-dir file 2");
  fs::write(&invalid_out_file3, "x").expect("failed to create invalid out-dir file 3");
  fs::write(&invalid_out_file4, "x").expect("failed to create invalid out-dir file 4");
  fs::write(&invalid_out_file5, "x").expect("failed to create invalid out-dir file 5");

  let args = vec![
    "--out-dir".to_string(),
    out1.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file1.display()),
    "--out-dir".to_string(),
    out2.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file2.display()),
    "--out-dir".to_string(),
    out3.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file3.display()),
    "--out-dir".to_string(),
    out4.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file4.display()),
    "--out-dir".to_string(),
    out5.path().display().to_string(),
    format!("--out-dir={}", invalid_out_file5.display()),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_uses_last_cc_and_out_dir_arguments_independently() {
  let sandbox = TempDirectory::create();
  let invalid_out_file = sandbox.path().join("independent-invalid-out-dir");

  fs::write(&invalid_out_file, "x").expect("failed to create invalid out-dir file");

  let final_output_dir = TempDirectory::create();
  let args = vec![
    format!("--out-dir={}", invalid_out_file.display()),
    "--cc=/definitely/not/a/compiler/rlibc-i007-independent-first".to_string(),
    "--out-dir".to_string(),
    final_output_dir.path().display().to_string(),
    "--cc=cc".to_string(),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_success(&output);
  assert_expected_objects(final_output_dir.path());
}

#[test]
fn crt_pipeline_fails_when_last_out_dir_is_invalid_even_if_cc_is_valid() {
  let first_output_dir = TempDirectory::create();
  let sandbox = TempDirectory::create();
  let invalid_out_file = sandbox.path().join("independent-invalid-last-out-dir");

  fs::write(&invalid_out_file, "x").expect("failed to create invalid last out-dir file");

  let args = vec![
    "--out-dir".to_string(),
    first_output_dir.path().display().to_string(),
    "--cc=/definitely/not/a/compiler/rlibc-i007-independent-first".to_string(),
    "--cc".to_string(),
    "cc".to_string(),
    format!("--out-dir={}", invalid_out_file.display()),
  ];
  let output = run_pipeline(&args);

  assert_pipeline_failure_contains(&output, "failed to create output directory");
}
