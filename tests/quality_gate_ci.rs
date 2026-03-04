use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn repository_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_repository_file(relative_path: &str) -> String {
  let path = repository_root().join(relative_path);

  fs::read_to_string(&path)
    .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn extract_function_body_lines(script: &str, signature_line: &str) -> Vec<String> {
  let mut in_function = false;
  let mut body_lines = Vec::new();

  for raw_line in script.lines() {
    let line = raw_line.trim();

    if !in_function {
      if line == signature_line {
        in_function = true;
      }

      continue;
    }

    if line == "}" {
      break;
    }

    if line.is_empty() {
      continue;
    }

    body_lines.push(line.to_string());
  }

  assert!(
    in_function,
    "failed to find function signature `{signature_line}` in quality gate script"
  );

  body_lines
}

fn extract_top_level_job_block(workflow: &str, job_name: &str) -> String {
  let job_header = format!("  {job_name}:");
  let mut in_job = false;
  let mut lines = Vec::new();

  for line in workflow.lines() {
    if !in_job {
      if line == job_header {
        in_job = true;
        lines.push(line.to_string());
      }

      continue;
    }

    if line.starts_with("  ") && !line.starts_with("    ") && line.ends_with(':') {
      break;
    }

    lines.push(line.to_string());
  }

  assert!(in_job, "failed to find top-level workflow job `{job_name}`");

  lines.join("\n")
}

fn extract_job_step_block(job_block: &str, step_name: &str) -> String {
  let step_header = format!("      - name: {step_name}");
  let mut in_step = false;
  let mut lines = Vec::new();

  for line in job_block.lines() {
    if !in_step {
      if line == step_header {
        in_step = true;
        lines.push(line.to_string());
      }

      continue;
    }

    if line.starts_with("      - name:") && line != step_header {
      break;
    }

    lines.push(line.to_string());
  }

  assert!(
    in_step,
    "failed to find workflow step `{step_name}` in selected job block"
  );

  lines.join("\n")
}

#[test]
fn quality_gate_script_defines_pr_nightly_full_profiles() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "PROFILE=\"pr\"",
    "Usage: bash scripts/quality-gate.sh [--profile <pr|nightly|full>|--profile=<pr|nightly|full>] [--continue-on-fail]",
    "run_pr_profile()",
    "run_nightly_profile()",
    "run_full_profile()",
    "case \"${PROFILE}\" in",
    "pr)",
    "nightly)",
    "full)",
  ] {
    assert!(
      script.contains(required_snippet),
      "quality gate script must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_parses_args_before_profile_dispatch() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let parse_call = "parse_args \"$@\"";
  let dispatch_case = "case \"${PROFILE}\" in";
  let parse_call_count = script.matches(parse_call).count();

  assert_eq!(
    parse_call_count, 1,
    "quality gate script must invoke parse_args exactly once before dispatch"
  );

  let parse_index = script
    .find(parse_call)
    .unwrap_or_else(|| panic!("quality gate script must contain `{parse_call}`"));
  let dispatch_index = script
    .find(dispatch_case)
    .unwrap_or_else(|| panic!("quality gate script must contain `{dispatch_case}`"));

  assert!(
    parse_index < dispatch_index,
    "quality gate script must parse cli args before profile dispatch case"
  );
}

#[test]
fn quality_gate_script_pr_profile_runs_release_commands() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "run_step 300 cargo run --release -p codestyle-check --bin codestyle_check -- --all --no-clippy",
    "run_step 300 cargo check --release --workspace",
    "run_step 300 cargo test --release --workspace --test quality_gate_ci",
    "run_step 300 cargo test --release --workspace --test stdlib_process abort_ignored_sigabrt_still_terminates_with_signal_and_skips_atexit_handlers -- --exact --test-threads=1",
    "run_step 300 cargo test --release --workspace --test stdlib_process abort_caught_sigabrt_runs_handler_then_terminates_with_signal -- --exact --test-threads=1",
    "run_step 300 cargo test --release --workspace --test library_shape",
    "run_step 300 cargo test --release --workspace --test abi_baseline",
  ] {
    assert!(
      script.contains(required_snippet),
      "pr profile must timeout-wrap command: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_pr_profile_prioritizes_i059_contract_checks() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let pr_body = extract_function_body_lines(&script, "run_pr_profile() {");
  let observed_steps: Vec<String> = pr_body
    .into_iter()
    .filter(|line| line.starts_with("run_"))
    .collect();
  let expected_steps = vec![
    "run_step 300 cargo test --release --workspace --test quality_gate_ci".to_string(),
    "run_step 300 cargo test --release --workspace --test stdlib_process abort_ignored_sigabrt_still_terminates_with_signal_and_skips_atexit_handlers -- --exact --test-threads=1".to_string(),
    "run_step 300 cargo test --release --workspace --test stdlib_process abort_caught_sigabrt_runs_handler_then_terminates_with_signal -- --exact --test-threads=1".to_string(),
    "run_step 300 cargo run --release -p codestyle-check --bin codestyle_check -- --all --no-clippy"
      .to_string(),
    "run_step 300 cargo check --release --workspace".to_string(),
    "run_step 300 cargo test --release --workspace --test library_shape".to_string(),
    "run_step 300 cargo test --release --workspace --test abi_baseline".to_string(),
  ];

  assert_eq!(
    observed_steps, expected_steps,
    "pr profile must execute I059 quality_gate_ci contract checks before broader workspace gates"
  );
}

#[test]
fn quality_gate_script_pr_profile_runs_abort_signal_regression_guard() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let pr_body = extract_function_body_lines(&script, "run_pr_profile() {");
  let quality_gate_step = "run_step 300 cargo test --release --workspace --test quality_gate_ci";
  let abort_ignored_guard_step = "run_step 300 cargo test --release --workspace --test stdlib_process abort_ignored_sigabrt_still_terminates_with_signal_and_skips_atexit_handlers -- --exact --test-threads=1";
  let abort_caught_guard_step = "run_step 300 cargo test --release --workspace --test stdlib_process abort_caught_sigabrt_runs_handler_then_terminates_with_signal -- --exact --test-threads=1";
  let codestyle_step = "run_step 300 cargo run --release -p codestyle-check --bin codestyle_check -- --all --no-clippy";
  let quality_gate_index = pr_body
    .iter()
    .position(|line| line == quality_gate_step)
    .unwrap_or_else(|| panic!("pr profile must contain step: {quality_gate_step}"));
  let abort_ignored_guard_index = pr_body
    .iter()
    .position(|line| line == abort_ignored_guard_step)
    .unwrap_or_else(|| panic!("pr profile must contain step: {abort_ignored_guard_step}"));
  let abort_caught_guard_index = pr_body
    .iter()
    .position(|line| line == abort_caught_guard_step)
    .unwrap_or_else(|| panic!("pr profile must contain step: {abort_caught_guard_step}"));
  let codestyle_index = pr_body
    .iter()
    .position(|line| line == codestyle_step)
    .unwrap_or_else(|| panic!("pr profile must contain step: {codestyle_step}"));

  assert!(
    quality_gate_index < abort_ignored_guard_index,
    "pr profile must run quality_gate_ci contract tests before abort_ignored regression guard"
  );
  assert!(
    abort_ignored_guard_index < abort_caught_guard_index,
    "pr profile must execute abort_ignored guard before abort_caught guard for deterministic signal contract checks"
  );
  assert!(
    abort_caught_guard_index < codestyle_index,
    "pr profile must execute abort regression guards before codestyle/check gates"
  );
}

#[test]
fn quality_gate_script_pr_profile_codestyle_step_disables_clippy() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let pr_body = extract_function_body_lines(&script, "run_pr_profile() {");
  let pr_codestyle_with_no_clippy = "run_step 300 cargo run --release -p codestyle-check --bin codestyle_check -- --all --no-clippy";
  let pr_codestyle_with_clippy =
    "run_step 300 cargo run --release -p codestyle-check --bin codestyle_check -- --all";

  assert!(
    pr_body
      .iter()
      .any(|line| line == pr_codestyle_with_no_clippy),
    "pr profile must keep codestyle step as --no-clippy to preserve profile split behavior"
  );
  assert!(
    !pr_body.iter().any(|line| line == pr_codestyle_with_clippy),
    "pr profile must not re-enable clippy through codestyle_check; clippy belongs to nightly/full"
  );
}

#[test]
fn quality_gate_script_pr_profile_excludes_workspace_level_clippy_and_tests() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let pr_body = extract_function_body_lines(&script, "run_pr_profile() {");

  for forbidden_step in [
    "run_step 300 cargo clippy --release --workspace",
    "run_step 300 cargo test --release --workspace",
  ] {
    assert!(
      !pr_body.iter().any(|line| line == forbidden_step),
      "pr profile must stay focused on I059 contract checks and must not include workspace-wide nightly/full step: {forbidden_step}"
    );
  }
}

#[test]
fn quality_gate_script_pr_profile_timeout_wraps_all_commands() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let pr_body = extract_function_body_lines(&script, "run_pr_profile() {");

  assert!(
    !pr_body.is_empty(),
    "pr profile must keep at least one run_step command"
  );

  for line in &pr_body {
    assert!(
      line.starts_with("run_step ") || line.starts_with('#'),
      "pr profile commands must stay timeout-wrapped through run_step: {line}"
    );
  }
}

#[test]
fn quality_gate_script_nightly_profile_extends_pr_commands() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "run_pr_profile",
    "run_step 300 cargo clippy --release --workspace",
    "run_step 300 cargo test --release --workspace",
    "run_step 300 cargo rustc --release --lib --crate-type cdylib",
    "run_step 120 cargo run --release --bin abi_check -- --golden abi/golden/x86_64-unknown-linux-gnu.abi",
  ] {
    assert!(
      script.contains(required_snippet),
      "nightly profile must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_builds_cdylib_before_abi_golden_diff() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let build_step = "run_step 300 cargo rustc --release --lib --crate-type cdylib";
  let abi_step = "run_step 120 cargo run --release --bin abi_check -- --golden abi/golden/x86_64-unknown-linux-gnu.abi";
  let build_index = script
    .find(build_step)
    .unwrap_or_else(|| panic!("nightly profile must contain: {build_step}"));
  let abi_index = script
    .find(abi_step)
    .unwrap_or_else(|| panic!("nightly profile must contain: {abi_step}"));

  assert!(
    build_index < abi_index,
    "nightly profile must build cdylib before abi golden diff"
  );
}

#[test]
fn quality_gate_script_nightly_profile_step_order_is_stable() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let nightly_body = extract_function_body_lines(&script, "run_nightly_profile() {");
  let observed_steps: Vec<String> = nightly_body
    .into_iter()
    .filter(|line| line.starts_with("run_"))
    .collect();
  let expected_steps = vec![
    "run_pr_profile".to_string(),
    "run_step 300 cargo clippy --release --workspace".to_string(),
    "run_step 300 cargo test --release --workspace".to_string(),
    "run_step 300 cargo rustc --release --lib --crate-type cdylib".to_string(),
    "run_step 120 cargo run --release --bin abi_check -- --golden abi/golden/x86_64-unknown-linux-gnu.abi".to_string(),
  ];

  assert_eq!(
    observed_steps, expected_steps,
    "nightly profile release gate steps must remain ordered for deterministic ABI checks"
  );
}

#[test]
fn quality_gate_script_full_profile_references_optional_adapters() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "run_nightly_profile",
    "scripts/conformance/libc-test-adapter.sh",
    "RLIBC_LIBC_TEST_ROOT",
    "--dry-run --profile docs/conformance/libc-test-smoke.txt",
    "scripts/conformance/ltp-openposix-adapter.sh",
    "RLIBC_LTP_SUITE_ROOT",
    "RLIBC_LTP_SUITE:-ltp",
    "docs/conformance/xfail-ledger.csv",
    "libc-test-smoke.txt",
    "awk -F','",
    "skipping optional adapter",
  ] {
    assert!(
      script.contains(required_snippet),
      "full profile must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_full_profile_gates_ltp_adapter_on_env_and_executable() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    concat!(
      "if [[ -x scripts/conformance/ltp-openposix-adapter.sh ]] && [[ -n \"${",
      "RLIBC_LTP_SUITE_ROOT:-}\" ]]; then"
    ),
    concat!(
      "run_step 1800 bash scripts/conformance/ltp-openposix-adapter.sh --suite \"${",
      "RLIBC_LTP_SUITE:-ltp}\" --suite-root \"${",
      "RLIBC_LTP_SUITE_ROOT}\""
    ),
    "skipping optional adapter: scripts/conformance/ltp-openposix-adapter.sh",
  ] {
    assert!(
      script.contains(required_snippet),
      "ltp adapter gating contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_full_profile_timeout_wraps_xfail_ledger_check() {
  let script = read_repository_file("scripts/quality-gate.sh");

  assert!(
    script.contains("run_step 60 awk -F','"),
    "full profile must timeout-wrap xfail-ledger validation through run_step"
  );
}

#[test]
fn quality_gate_script_resolves_timeout_command() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "command -v timeout",
    "command -v gtimeout",
    "TIMEOUT_BIN",
    "\"${TIMEOUT_BIN}\"",
  ] {
    assert!(
      script.contains(required_snippet),
      "quality gate script must resolve timeout command with snippet: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_fails_with_127_when_timeout_binary_is_missing() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "echo \"[quality-gate] failed: timeout command not found (tried: timeout, gtimeout)\" >&2",
    "exit 127",
  ] {
    assert!(
      script.contains(required_snippet),
      "timeout resolution failure branch must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_defines_failed_tests_logging_contract() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "FAILED_TESTS_PATH",
    "FAILURE_ISSUE_ID",
    "RLIBC_FAILURE_ISSUE_ID",
    "FAILURE_LOG_ENABLED",
    "RLIBC_FAILURE_LOG_ENABLED",
    "append_failure_log()",
    "docs/failed-tests.md",
  ] {
    assert!(
      script.contains(required_snippet),
      "quality gate script must define failed-tests logging contract snippet: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_defaults_failure_issue_id_to_ci() {
  let script = read_repository_file("scripts/quality-gate.sh");

  assert!(
    script.contains("FAILURE_ISSUE_ID=\"${RLIBC_FAILURE_ISSUE_ID:-CI}\""),
    "quality gate script must default failure issue id routing to CI when RLIBC_FAILURE_ISSUE_ID is unset"
  );
}

#[test]
fn quality_gate_script_defaults_failure_log_enabled_to_one() {
  let script = read_repository_file("scripts/quality-gate.sh");

  assert!(
    script.contains("FAILURE_LOG_ENABLED=\"${RLIBC_FAILURE_LOG_ENABLED:-1}\""),
    "quality gate script must default failure log writes to enabled when RLIBC_FAILURE_LOG_ENABLED is unset"
  );
}

#[test]
fn quality_gate_script_append_failure_log_is_guarded() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "if [[ \"${FAILURE_LOG_ENABLED}\" != \"1\" ]]; then",
    "if [[ ! -f \"${FAILED_TESTS_PATH}\" ]]; then",
    "printf '%s|%s|%s\\n' \"${FAILURE_ISSUE_ID}\" \"${command}\" \"${reason}\" >> \"${FAILED_TESTS_PATH}\" || true",
  ] {
    assert!(
      script.contains(required_snippet),
      "append_failure_log guard contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_run_step_logs_failures_and_timeouts() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "command_status=0",
    "command_string=\"${TIMEOUT_BIN} ${timeout_seconds}\"",
    "for argument in \"$@\"; do",
    "command_string+=\" $(printf '%q' \"${argument}\")\"",
    "set +e",
    "set -e",
    "if [[ \"${command_status}\" -eq 124 ]]; then",
    "append_failure_log \"${command_string}\" \"stuck (timeout exit 124)\"",
    "append_failure_log \"${command_string}\" \"exit code ${command_status}\"",
  ] {
    assert!(
      script.contains(required_snippet),
      "run_step failure logging contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_run_step_echoes_escaped_command_string() {
  let script = read_repository_file("scripts/quality-gate.sh");

  assert!(
    script.contains("echo \"[quality-gate:${PROFILE}] running: ${command_string}\""),
    "run_step progress output must use escaped command_string for consistent diagnostics"
  );
  assert!(
    !script.contains("echo \"[quality-gate:${PROFILE}] running: $*\""),
    "run_step progress output must not regress to unescaped $* display"
  );
}

#[test]
fn quality_gate_script_run_step_avoids_unescaped_command_string_regression() {
  let script = read_repository_file("scripts/quality-gate.sh");

  assert!(
    script.contains("local command_string=\"${TIMEOUT_BIN} ${timeout_seconds}\""),
    "run_step must start failure log command text from timeout prefix only"
  );
  assert!(
    script.contains("for argument in \"$@\"; do"),
    "run_step must build command text from escaped argv entries"
  );
  assert!(
    script.contains("command_string+=\" $(printf '%q' \"${argument}\")\""),
    "run_step must escape each argv entry when composing failure log command text"
  );
  assert!(
    !script.contains("local command_string=\"${TIMEOUT_BIN} ${timeout_seconds} $*\""),
    "run_step must not regress to unescaped $* command logging"
  );
}

#[test]
fn quality_gate_script_supports_continue_on_fail_mode() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "CONTINUE_ON_FAIL=\"0\"",
    "--continue-on-fail",
    "(flag only; no value)",
    "CONTINUE_ON_FAIL=\"1\"",
    "OVERALL_STATUS=0",
    "if [[ \"${CONTINUE_ON_FAIL}\" == \"1\" ]]; then",
    "if [[ \"${OVERALL_STATUS}\" -ne 0 ]]; then",
  ] {
    assert!(
      script.contains(required_snippet),
      "quality gate script must define continue-on-fail contract snippet: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_continue_on_fail_preserves_first_failure_status() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "if [[ \"${CONTINUE_ON_FAIL}\" == \"1\" ]]; then",
    "if [[ \"${OVERALL_STATUS}\" -eq 0 ]]; then",
    "OVERALL_STATUS=\"${command_status}\"",
    "return 0",
    "if [[ \"${OVERALL_STATUS}\" -ne 0 ]]; then",
    "exit \"${OVERALL_STATUS}\"",
  ] {
    assert!(
      script.contains(required_snippet),
      "continue-on-fail first-failure-status contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_rejects_unsupported_profile_values() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "echo \"[quality-gate] failed: unsupported profile '${PROFILE}'\" >&2",
    "print_usage",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "unsupported profile branch must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_rejects_unknown_arguments() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "echo \"[quality-gate] failed: unknown argument: $1\" >&2",
    "print_usage",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "unknown-argument guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_separates_option_and_positional_argument_errors() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "-*)",
    "echo \"[quality-gate] failed: unknown argument: $1\" >&2",
    "*)",
    "echo \"[quality-gate] failed: unexpected positional argument: $1\" >&2",
    "print_usage",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "option-vs-positional parse guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_checks_option_branch_before_positional_branch() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let parse_args_body = extract_function_body_lines(&script, "parse_args() {");
  let option_branch_index = parse_args_body
    .iter()
    .position(|line| line == "-*)")
    .unwrap_or_else(|| panic!("parse_args must define unknown-option branch '-*)'"));
  let positional_branch_index = parse_args_body
    .iter()
    .position(|line| line == "*)")
    .unwrap_or_else(|| panic!("parse_args must define positional-argument branch '*)'"));

  assert!(
    option_branch_index < positional_branch_index,
    "parse_args must evaluate unknown-option branch before positional-argument branch"
  );
}

#[test]
fn quality_gate_script_checks_end_of_options_before_unknown_option_branch() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let parse_args_body = extract_function_body_lines(&script, "parse_args() {");
  let end_of_options_branch_index = parse_args_body
    .iter()
    .position(|line| line == "--)")
    .unwrap_or_else(|| panic!("parse_args must define end-of-options branch '--)'"));
  let unknown_option_branch_index = parse_args_body
    .iter()
    .position(|line| line == "-*)")
    .unwrap_or_else(|| panic!("parse_args must define unknown-option branch '-*)'"));

  assert!(
    end_of_options_branch_index < unknown_option_branch_index,
    "parse_args must evaluate '--' branch before unknown-option fallback so '--' is not treated as an unknown option"
  );
}

#[test]
fn quality_gate_script_handles_end_of_options_separator() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "--)",
    "if [[ $# -gt 0 ]]; then",
    "echo \"[quality-gate] failed: unexpected positional argument: $1\" >&2",
    "print_usage",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "end-of-options contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_rejects_duplicate_profile_argument() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "PROFILE_SET=\"0\"",
    "if [[ \"${PROFILE_SET}\" == \"1\" ]]; then",
    "echo \"[quality-gate] failed: --profile specified multiple times\" >&2",
    "PROFILE_SET=\"1\"",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "duplicate --profile guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_rejects_empty_profile_value() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "if [[ -z \"${PROFILE}\" ]]; then",
    "echo \"[quality-gate] failed: --profile requires a non-empty value\" >&2",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "empty --profile value guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_rejects_missing_profile_value_after_flag() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "if [[ $# -eq 0 ]]; then",
    "echo \"[quality-gate] failed: --profile requires a value\" >&2",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "missing --profile value guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_rejects_option_like_profile_token_after_flag_as_missing_value() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "--profile)",
    "if [[ \"$1\" == -* ]]; then",
    "echo \"[quality-gate] failed: --profile requires a value\" >&2",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "option-like profile token after --profile must be handled as missing-value contract: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_checks_option_like_profile_token_before_setting_value() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let option_like_guard = "if [[ \"$1\" == -* ]]; then";
  let set_profile_call = "set_profile_value \"$1\"";
  let option_like_guard_index = script
    .find(option_like_guard)
    .unwrap_or_else(|| panic!("parse_args must contain option-like profile token guard"));
  let set_profile_call_index = script
    .find(set_profile_call)
    .unwrap_or_else(|| panic!("parse_args must contain set_profile_value call for --profile"));

  assert!(
    option_like_guard_index < set_profile_call_index,
    "parse_args must reject option-like --profile token before calling set_profile_value"
  );
}

#[test]
fn quality_gate_script_rejects_option_like_profile_value() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "if [[ \"${PROFILE}\" == -* ]]; then",
    "echo \"[quality-gate] failed: --profile value must not start with '-': ${PROFILE}\" >&2",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "option-like --profile value guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_supports_equals_style_profile_argument() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "set_profile_value() {",
    "--profile=*)",
    "set_profile_value \"${1#--profile=}\"",
    "--profile)",
    "set_profile_value \"$1\"",
  ] {
    assert!(
      script.contains(required_snippet),
      "equals-style --profile guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_centralizes_profile_validation_in_helper() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "set_profile_value() {",
    "local candidate=\"$1\"",
    "if [[ \"${PROFILE_SET}\" == \"1\" ]]; then",
    "echo \"[quality-gate] failed: --profile specified multiple times\" >&2",
    "PROFILE=\"${candidate}\"",
    "if [[ -z \"${PROFILE}\" ]]; then",
    "echo \"[quality-gate] failed: --profile requires a non-empty value\" >&2",
    "if [[ \"${PROFILE}\" == -* ]]; then",
    "echo \"[quality-gate] failed: --profile value must not start with '-': ${PROFILE}\" >&2",
    "PROFILE_SET=\"1\"",
    "set_profile_value \"$1\"",
    "set_profile_value \"${1#--profile=}\"",
  ] {
    assert!(
      script.contains(required_snippet),
      "profile helper contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_rejects_duplicate_continue_on_fail_argument() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "CONTINUE_ON_FAIL_SET=\"0\"",
    "if [[ \"${CONTINUE_ON_FAIL_SET}\" == \"1\" ]]; then",
    "echo \"[quality-gate] failed: --continue-on-fail specified multiple times\" >&2",
    "CONTINUE_ON_FAIL=\"1\"",
    "CONTINUE_ON_FAIL_SET=\"1\"",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "duplicate --continue-on-fail guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_rejects_equals_style_continue_on_fail_argument() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "--continue-on-fail=*)",
    "echo \"[quality-gate] failed: --continue-on-fail does not take a value: ${1}\" >&2",
    "exit 2",
  ] {
    assert!(
      script.contains(required_snippet),
      "equals-style --continue-on-fail guard must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_checks_help_branch_before_unknown_option_branch() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let parse_args_body = extract_function_body_lines(&script, "parse_args() {");
  let help_branch_index = parse_args_body
    .iter()
    .position(|line| line == "-h | --help)")
    .unwrap_or_else(|| panic!("parse_args must define help branch '-h | --help)'"));
  let unknown_option_branch_index = parse_args_body
    .iter()
    .position(|line| line == "-*)")
    .unwrap_or_else(|| panic!("parse_args must define unknown-option branch '-*)'"));

  assert!(
    help_branch_index < unknown_option_branch_index,
    "parse_args must evaluate help branch before unknown-option fallback"
  );
}

#[test]
fn quality_gate_script_checks_end_of_options_before_help_branch() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let parse_args_body = extract_function_body_lines(&script, "parse_args() {");
  let end_of_options_branch_index = parse_args_body
    .iter()
    .position(|line| line == "--)")
    .unwrap_or_else(|| panic!("parse_args must define end-of-options branch '--)'"));
  let help_branch_index = parse_args_body
    .iter()
    .position(|line| line == "-h | --help)")
    .unwrap_or_else(|| panic!("parse_args must define help branch '-h | --help)'"));

  assert!(
    end_of_options_branch_index < help_branch_index,
    "parse_args must evaluate '--' branch before help-flag branch"
  );
}

#[test]
fn quality_gate_script_supports_help_flags() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in ["-h | --help)", "print_usage", "exit 0"] {
    assert!(
      script.contains(required_snippet),
      "help-flag branch must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_usage_describes_each_profile_mode() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "Profiles:",
    "pr       fast checks for pull requests and pushes",
    "nightly  extended checks for scheduled runs",
    "full     nightly checks + optional conformance adapters",
    "Options:",
    "-h, --help         show this help and exit",
  ] {
    assert!(
      script.contains(required_snippet),
      "usage contract must describe profile mode behavior: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_usage_mentions_end_of_options_separator() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "--)",
    "if [[ $# -gt 0 ]]; then",
    "--                end option parsing (positional args are rejected)",
  ] {
    assert!(
      script.contains(required_snippet),
      "usage/end-of-options contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_usage_mentions_default_profile() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in ["PROFILE=\"pr\"", "default: pr when omitted"] {
    assert!(
      script.contains(required_snippet),
      "usage/default-profile contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_usage_mentions_default_profile_once() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let occurrence_count = script.matches("default: pr when omitted").count();

  assert_eq!(
    occurrence_count, 1,
    "usage/default-profile contract text must appear exactly once to avoid duplicate guidance drift"
  );
}

#[test]
fn quality_gate_script_usage_mentions_continue_on_fail_default_behavior() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "--continue-on-fail  keep running remaining profile steps and exit non-zero at end if any step failed (flag only; no value)",
    "default: stop on first failed step",
  ] {
    assert!(
      script.contains(required_snippet),
      "usage/continue-on-fail contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_usage_mentions_continue_on_fail_equals_style_rejection() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let required_snippet = "equals-style values are rejected (for example: --continue-on-fail=1)";

  assert!(
    script.contains(required_snippet),
    "usage/continue-on-fail contract must mention equals-style rejection: {required_snippet}"
  );
}

#[test]
fn quality_gate_script_usage_mentions_duplicate_option_rejection() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let required_snippet =
    "duplicate options are rejected (for example: repeated --profile or --continue-on-fail)";

  assert!(
    script.contains(required_snippet),
    "usage/options contract must mention duplicate-option rejection: {required_snippet}"
  );
}

#[test]
fn quality_gate_script_usage_mentions_duplicate_profile_rejection_for_both_forms() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let required_snippet =
    "duplicate --profile is rejected for both --profile <value> and --profile=<value> forms";

  assert!(
    script.contains(required_snippet),
    "usage/profile contract must mention duplicate-profile rejection for both forms: {required_snippet}"
  );
}

#[test]
fn quality_gate_script_usage_mentions_duplicate_profile_rejection_once() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let occurrence_count = script
    .matches(
      "duplicate --profile is rejected for both --profile <value> and --profile=<value> forms",
    )
    .count();

  assert_eq!(
    occurrence_count, 1,
    "usage/profile duplicate-profile rejection text must appear exactly once to avoid duplicate guidance drift"
  );
}

#[test]
fn quality_gate_script_usage_lists_duplicate_profile_rejection_between_profile_and_continue_sections()
 {
  let script = read_repository_file("scripts/quality-gate.sh");
  let profile_equals_style_support = "supports --profile <value> and --profile=<value> forms";
  let duplicate_profile_rejection =
    "duplicate --profile is rejected for both --profile <value> and --profile=<value> forms";
  let continue_on_fail_option = "--continue-on-fail  keep running remaining profile steps and exit non-zero at end if any step failed (flag only; no value)";
  let profile_equals_style_support_index = script
    .find(profile_equals_style_support)
    .unwrap_or_else(|| {
      panic!(
        "usage must contain profile equals-style support contract line: {profile_equals_style_support}"
      )
    });
  let duplicate_profile_rejection_index = script
    .find(duplicate_profile_rejection)
    .unwrap_or_else(|| {
      panic!(
        "usage must contain duplicate-profile rejection contract line: {duplicate_profile_rejection}"
      )
    });
  let continue_on_fail_index = script.find(continue_on_fail_option).unwrap_or_else(|| {
    panic!("usage must contain continue-on-fail option contract line: {continue_on_fail_option}")
  });

  assert!(
    profile_equals_style_support_index < duplicate_profile_rejection_index,
    "usage must list duplicate-profile rejection guidance after profile equals-style support"
  );
  assert!(
    duplicate_profile_rejection_index < continue_on_fail_index,
    "usage must keep duplicate-profile rejection guidance in the profile section before continue-on-fail option"
  );
}

#[test]
fn quality_gate_script_usage_mentions_duplicate_option_rejection_once() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let occurrence_count = script
    .matches(
      "duplicate options are rejected (for example: repeated --profile or --continue-on-fail)",
    )
    .count();

  assert_eq!(
    occurrence_count, 1,
    "usage/options duplicate-option rejection text must appear exactly once to avoid duplicate guidance drift"
  );
}

#[test]
fn quality_gate_script_usage_mentions_continue_on_fail_equals_style_rejection_once() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let occurrence_count = script
    .matches("equals-style values are rejected (for example: --continue-on-fail=1)")
    .count();

  assert_eq!(
    occurrence_count, 1,
    "usage/continue-on-fail equals-style rejection text must appear exactly once to avoid duplicate guidance drift"
  );
}

#[test]
fn quality_gate_script_usage_lists_duplicate_option_rejection_before_default_behavior() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let duplicate_rejection_line =
    "duplicate options are rejected (for example: repeated --profile or --continue-on-fail)";
  let default_line = "default: stop on first failed step";
  let duplicate_rejection_index = script.find(duplicate_rejection_line).unwrap_or_else(|| {
    panic!("usage must contain duplicate-option rejection line: {duplicate_rejection_line}")
  });
  let default_index = script
    .find(default_line)
    .unwrap_or_else(|| panic!("usage must contain default behavior line: {default_line}"));

  assert!(
    duplicate_rejection_index < default_index,
    "usage must list duplicate-option rejection guidance before continue-on-fail default behavior"
  );
}

#[test]
fn quality_gate_script_usage_lists_duplicate_option_rejection_after_equals_style_rejection() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let equals_style_rejection_line =
    "equals-style values are rejected (for example: --continue-on-fail=1)";
  let duplicate_rejection_line =
    "duplicate options are rejected (for example: repeated --profile or --continue-on-fail)";
  let equals_style_rejection_index = script.find(equals_style_rejection_line).unwrap_or_else(|| {
    panic!(
      "usage must contain continue-on-fail equals-style rejection line: {equals_style_rejection_line}"
    )
  });
  let duplicate_rejection_index = script.find(duplicate_rejection_line).unwrap_or_else(|| {
    panic!("usage must contain duplicate-option rejection line: {duplicate_rejection_line}")
  });

  assert!(
    equals_style_rejection_index < duplicate_rejection_index,
    "usage must list duplicate-option rejection guidance after equals-style rejection guidance"
  );
}

#[test]
fn quality_gate_script_usage_lists_continue_on_fail_rejection_after_option_line() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let option_line = "--continue-on-fail  keep running remaining profile steps and exit non-zero at end if any step failed (flag only; no value)";
  let rejection_line = "equals-style values are rejected (for example: --continue-on-fail=1)";
  let option_index = script.find(option_line).unwrap_or_else(|| {
    panic!("usage must contain continue-on-fail option contract line: {option_line}")
  });
  let rejection_index = script.find(rejection_line).unwrap_or_else(|| {
    panic!("usage must contain continue-on-fail equals-style rejection line: {rejection_line}")
  });

  assert!(
    option_index < rejection_index,
    "usage must list continue-on-fail equals-style rejection after the option line for readable grouping"
  );
}

#[test]
fn quality_gate_script_usage_lists_continue_on_fail_default_after_rejection() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let option_line = "--continue-on-fail  keep running remaining profile steps and exit non-zero at end if any step failed (flag only; no value)";
  let rejection_line = "equals-style values are rejected (for example: --continue-on-fail=1)";
  let default_line = "default: stop on first failed step";
  let option_index = script.find(option_line).unwrap_or_else(|| {
    panic!("usage must contain continue-on-fail option contract line: {option_line}")
  });
  let rejection_index = script.find(rejection_line).unwrap_or_else(|| {
    panic!("usage must contain continue-on-fail equals-style rejection line: {rejection_line}")
  });
  let default_index = script.find(default_line).unwrap_or_else(|| {
    panic!("usage must contain continue-on-fail default behavior line: {default_line}")
  });

  assert!(
    rejection_index < default_index,
    "usage must list continue-on-fail default behavior after equals-style rejection guidance"
  );
  assert!(
    option_index < default_index,
    "usage must list continue-on-fail default behavior after the option line"
  );
}

#[test]
fn quality_gate_script_usage_mentions_continue_on_fail_default_behavior_once() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let occurrence_count = script.matches("default: stop on first failed step").count();

  assert_eq!(
    occurrence_count, 1,
    "usage/continue-on-fail default behavior text must appear exactly once to avoid duplicate guidance drift"
  );
}

#[test]
fn quality_gate_script_usage_describes_profile_option_contract() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in [
    "--profile <pr|nightly|full>  select quality-gate profile",
    "default: pr when omitted",
  ] {
    assert!(
      script.contains(required_snippet),
      "usage/profile-option contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_usage_lists_profile_option_before_continue_on_fail_option() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let profile_option = "--profile <pr|nightly|full>  select quality-gate profile";
  let profile_equals_style_support = "supports --profile <value> and --profile=<value> forms";
  let continue_on_fail_option = "--continue-on-fail  keep running remaining profile steps and exit non-zero at end if any step failed (flag only; no value)";
  let profile_index = script
    .find(profile_option)
    .unwrap_or_else(|| panic!("usage must contain profile option contract line: {profile_option}"));
  let profile_equals_style_support_index = script
    .find(profile_equals_style_support)
    .unwrap_or_else(|| {
      panic!(
        "usage must contain profile equals-style support contract line: {profile_equals_style_support}"
      )
    });
  let continue_on_fail_index = script.find(continue_on_fail_option).unwrap_or_else(|| {
    panic!("usage must contain continue-on-fail option contract line: {continue_on_fail_option}")
  });

  assert!(
    profile_index < continue_on_fail_index,
    "usage must list profile selector before continue-on-fail option to keep option help scan order stable"
  );
  assert!(
    profile_index < profile_equals_style_support_index,
    "usage must list profile equals-style support after the profile option line for readable grouping"
  );
  assert!(
    profile_equals_style_support_index < continue_on_fail_index,
    "usage must keep profile guidance grouped before continue-on-fail option line"
  );
}

#[test]
fn quality_gate_script_usage_mentions_profile_equals_style_support() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let required_snippet = "supports --profile <value> and --profile=<value> forms";

  assert!(
    script.contains(required_snippet),
    "usage/profile-option contract must mention equals-style support: {required_snippet}"
  );
}

#[test]
fn quality_gate_script_usage_mentions_profile_equals_style_support_once() {
  let script = read_repository_file("scripts/quality-gate.sh");
  let occurrence_count = script
    .matches("supports --profile <value> and --profile=<value> forms")
    .count();

  assert_eq!(
    occurrence_count, 1,
    "usage/profile-option equals-style support text must appear exactly once to avoid duplicate guidance drift"
  );
}

#[test]
fn quality_gate_script_enters_repository_root() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in ["BASH_SOURCE[0]", "REPO_ROOT", "cd \"${REPO_ROOT}\""] {
    assert!(
      script.contains(required_snippet),
      "quality gate script must execute from repository root with snippet: {required_snippet}"
    );
  }
}

#[test]
fn quality_gate_script_enables_strict_shell_mode() {
  let script = read_repository_file("scripts/quality-gate.sh");

  for required_snippet in ["#!/usr/bin/env bash", "set -euo pipefail"] {
    assert!(
      script.contains(required_snippet),
      "quality gate script must keep strict shell mode snippet: {required_snippet}"
    );
  }
}

#[test]
fn libc_test_adapter_script_defines_manifest_and_mode_contract() {
  let script = read_repository_file("scripts/conformance/libc-test-adapter.sh");

  for required_snippet in [
    "Usage: bash scripts/conformance/libc-test-adapter.sh [--dry-run] [--profile <manifest-path>]",
    "RLIBC_LIBC_TEST_ROOT",
    "RLIBC_LIBC_TEST_SMOKE_MANIFEST",
    "RLIBC_LIBC_TEST_RUNTEST",
    "docs/conformance/libc-test-smoke.txt",
    "cargo rustc --release --lib --crate-type cdylib",
    "cargo rustc --release --lib --crate-type staticlib",
    "expected <case-id>|<command>",
    "resolve_runtest_command",
    "rewrite_runtest_prefix",
    "command_uses_runtest_prefix",
    "runtest_command_has_arguments",
    "runtest_command_uses_unsupported_shell_operators",
    "trim_ascii_whitespace",
    "./bin/runtest",
    "bin/runtest",
    "invalid smoke entry at line",
    "invalid smoke case id",
    "duplicate smoke case id",
    "smoke case ids must be sorted",
    "runtest smoke command requires explicit arguments",
    "runtest smoke command contains unsupported shell operators",
    "RLIBC_LIBC_TEST_RUNTEST must not be empty when set",
    "RLIBC_LIBC_TEST_RUNTEST must not contain whitespace",
    "RLIBC_LIBC_TEST_RUNTEST must be a path-like token without shell metacharacters",
    "set RLIBC_LIBC_TEST_RUNTEST to override runtest location",
    "if [[ \"${DRY_RUN}\" -eq 1 ]]; then",
  ] {
    assert!(
      script.contains(required_snippet),
      "libc-test adapter script must contain: {required_snippet}"
    );
  }
}

#[test]
fn libc_test_smoke_manifest_has_case_id_command_entries() {
  let manifest = read_repository_file("docs/conformance/libc-test-smoke.txt");
  let entries: Vec<&str> = manifest
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty() && !line.starts_with('#'))
    .collect();

  assert!(
    !entries.is_empty(),
    "smoke manifest must contain at least one case"
  );

  let mut case_ids = Vec::new();

  for entry in &entries {
    let mut parts = entry.splitn(2, '|');
    let case_id = parts.next().unwrap_or_default().trim();
    let command = parts.next().unwrap_or_default().trim();

    assert!(!case_id.is_empty(), "case id must not be empty: {entry}");
    assert!(!command.is_empty(), "command must not be empty: {entry}");
    assert!(
      case_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-')),
      "case id must stay machine-parseable: {case_id}",
    );
    assert!(
      command_uses_supported_runtest_prefix(command),
      "smoke manifest command must use a supported runtest prefix: {command}",
    );

    case_ids.push(case_id.to_string());
  }

  let mut sorted_case_ids = case_ids.clone();

  sorted_case_ids.sort();
  assert_eq!(
    case_ids, sorted_case_ids,
    "smoke case ids must stay sorted for deterministic conformance diffs"
  );

  let unique_case_id_count = case_ids
    .iter()
    .collect::<std::collections::BTreeSet<_>>()
    .len();

  assert_eq!(
    case_ids.len(),
    unique_case_id_count,
    "smoke case ids must stay unique"
  );
}

fn command_uses_supported_runtest_prefix(command: &str) -> bool {
  let Some((prefix, suffix)) = command.split_once(char::is_whitespace) else {
    return false;
  };
  let trimmed_suffix = suffix.trim();

  if trimmed_suffix.is_empty() || trimmed_suffix.starts_with('#') {
    return false;
  }

  if trimmed_suffix.split_whitespace().last() == Some("-w") {
    return false;
  }

  if trimmed_suffix.contains(';')
    || trimmed_suffix.contains("&&")
    || trimmed_suffix.contains("||")
    || trimmed_suffix.contains('&')
    || trimmed_suffix.contains('|')
    || trimmed_suffix.contains('!')
    || trimmed_suffix.contains('`')
    || trimmed_suffix.contains('$')
    || trimmed_suffix.contains('\\')
    || trimmed_suffix.contains("$(")
    || trimmed_suffix.contains('<')
    || trimmed_suffix.contains('>')
    || trimmed_suffix.contains('(')
    || trimmed_suffix.contains(')')
    || trimmed_suffix.contains('\'')
    || trimmed_suffix.contains('"')
    || trimmed_suffix.contains('#')
    || trimmed_suffix.contains('*')
    || trimmed_suffix.contains('?')
    || trimmed_suffix.contains('[')
    || trimmed_suffix.contains(']')
    || trimmed_suffix.contains('{')
    || trimmed_suffix.contains('}')
  {
    return false;
  }

  let tokens: Vec<&str> = trimmed_suffix.split_whitespace().collect();

  if tokens.is_empty() {
    return false;
  }

  let mut saw_workload_flag = false;
  let mut token_index = 0usize;
  let token_count = tokens.len();

  while token_index < token_count {
    let token = tokens[token_index];

    if token.starts_with("-w=") {
      return false;
    }

    if token.starts_with("-w") && token != "-w" {
      return false;
    }

    if token != "-w" {
      return false;
    }

    saw_workload_flag = true;

    let Some(workload) = tokens.get(token_index + 1) else {
      return false;
    };

    if workload.starts_with('-') {
      return false;
    }

    if workload.starts_with('/') {
      return false;
    }

    if workload.ends_with('/') {
      return false;
    }

    if workload.contains("//") {
      return false;
    }

    if workload == &"."
      || workload.starts_with("./")
      || workload.ends_with("/.")
      || workload.contains("/./")
    {
      return false;
    }

    if workload == &".."
      || workload.starts_with("../")
      || workload.ends_with("/..")
      || workload.contains("/../")
    {
      return false;
    }

    token_index += 2;
  }

  matches!(
    prefix,
    "runtest" | "./runtest" | "bin/runtest" | "./bin/runtest"
  ) && saw_workload_flag
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_missing_workload_after_w_flag() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w"),
    "runtest -w without workload must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_absolute_workload_path() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w /functional/argv"),
    "absolute workload path must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_dotdot_workload_segment() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/../argv"),
    "dotdot workload segment must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_dot_workload_segment() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/./argv"),
    "dot workload segment must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_empty_workload_path_segment() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional//argv"),
    "empty workload path segment must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_trailing_workload_path_separator() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/"),
    "trailing workload path separator must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_invalid_workload_paths_for_bin_runtest_prefixes() {
  let invalid_workloads = [
    "/functional/argv",
    "./functional/argv",
    "../functional/argv",
    ".",
    "..",
    "functional/./argv",
    "functional/.",
    "functional/",
    "functional//argv",
    "functional/../argv",
    "functional/..",
  ];

  for prefix in ["bin/runtest", "./bin/runtest"] {
    for workload in invalid_workloads {
      let command = format!("{prefix} -w {workload}");

      assert!(
        !command_uses_supported_runtest_prefix(&command),
        "invalid workload path for bin-runtest prefix must be rejected by manifest contract helper: {command}"
      );
    }
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_invalid_workload_paths_for_dot_runtest_prefix() {
  let invalid_workloads = [
    "/functional/argv",
    "./functional/argv",
    "../functional/argv",
    ".",
    "..",
    "functional/./argv",
    "functional/.",
    "functional/",
    "functional//argv",
    "functional/../argv",
    "functional/..",
  ];

  for workload in invalid_workloads {
    let command = format!("./runtest -w {workload}");

    assert!(
      !command_uses_supported_runtest_prefix(&command),
      "invalid workload path for ./runtest prefix must be rejected by manifest contract helper: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_invalid_workload_paths_for_bare_runtest_prefix() {
  let invalid_workloads = [
    "/functional/argv",
    "./functional/argv",
    "../functional/argv",
    ".",
    "..",
    "functional/./argv",
    "functional/.",
    "functional/",
    "functional//argv",
    "functional/../argv",
    "functional/..",
  ];

  for workload in invalid_workloads {
    let command = format!("runtest -w {workload}");

    assert!(
      !command_uses_supported_runtest_prefix(&command),
      "invalid workload path for runtest prefix must be rejected by manifest contract helper: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_suffix_without_w_workload_selector() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest functional/argv"),
    "runtest suffix without -w workload selector must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_bin_runtest_prefix_without_arguments() {
  for command in ["bin/runtest", "./bin/runtest"] {
    assert!(
      !command_uses_supported_runtest_prefix(command),
      "bin-runtest prefix without args must be rejected by manifest contract helper: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_option_like_workload_after_w_flag() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w --all"),
    "runtest -w followed by an option-like token must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_trailing_option_like_token_outside_w_pair() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv --all"),
    "option-like tokens outside -w pairs must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_trailing_positional_token_outside_w_pair() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv functional/ctype"),
    "positional tokens outside -w pairs must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_empty_workload_equals_syntax() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w="),
    "runtest -w= without workload must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_non_empty_workload_equals_syntax() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w=functional/argv"),
    "runtest -w=<workload> attached syntax must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_non_empty_workload_equals_syntax_after_valid_pair()
{
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv -w=functional/errno"),
    "runtest -w=<workload> syntax after valid -w pair must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_attached_w_workload_token() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -wfunctional/argv"),
    "attached -w workload token must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_accepts_split_w_workload_for_allowed_prefixes() {
  for command in [
    "runtest -w functional/argv",
    "./runtest -w math/sin",
    "bin/runtest -w string/memcpy",
    "./bin/runtest -w stdio/fprintf",
  ] {
    assert!(
      command_uses_supported_runtest_prefix(command),
      "space-separated -w workload token must remain valid for supported runtest prefixes: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_accepts_multiple_split_w_workload_pairs() {
  assert!(
    command_uses_supported_runtest_prefix("runtest -w functional/argv -w functional/ctype"),
    "multiple split -w workload pairs must remain valid in manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_accepts_multiple_split_w_workload_pairs_for_allowed_prefixes()
 {
  for command in [
    "runtest -w functional/argv -w functional/ctype",
    "./runtest -w math/sin -w math/cos",
    "bin/runtest -w string/memcpy -w string/memmove",
    "./bin/runtest -w stdio/fprintf -w stdio/snprintf",
  ] {
    assert!(
      command_uses_supported_runtest_prefix(command),
      "multiple split -w workload pairs must remain valid for supported runtest prefixes: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_accepts_nested_workload_path_for_supported_prefixes() {
  for command in [
    "runtest -w functional/stdio/vfprintf",
    "./runtest -w functional/stdio/vfprintf",
    "bin/runtest -w functional/stdio/vfprintf",
    "./bin/runtest -w functional/stdio/vfprintf",
  ] {
    assert!(
      command_uses_supported_runtest_prefix(command),
      "nested workload path must remain valid for supported runtest prefixes: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_attached_w_workload_token_after_valid_pair() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv -wfunctional/errno"),
    "attached -w workload token after a valid -w pair must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_logical_or_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv || echo unexpected"),
    "logical-or operator in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_positional_token_and_logical_or_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix(
      "runtest -w functional/argv functional/ctype || echo unexpected",
    ),
    "positional token plus logical-or operator in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_logical_and_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv && echo unexpected"),
    "logical-and operator in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_background_operator_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv & echo unexpected"),
    "background operator in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_output_redirection_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv > /tmp/rlibc-i060-out"),
    "output redirection in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_input_redirection_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv < /tmp/rlibc-i060-in"),
    "input redirection in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_trailing_inline_comment_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/argv # comment"),
    "trailing inline comment in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_shell_suffixes_for_bin_runtest_prefixes() {
  for command in [
    "bin/runtest -w functional/argv # comment",
    "./bin/runtest -w functional/*",
    "bin/runtest -w functional/argv || echo unexpected",
  ] {
    assert!(
      !command_uses_supported_runtest_prefix(command),
      "bin-runtest variant with shell suffix must be rejected by manifest contract helper: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_operator_suffixes_for_bin_runtest_prefixes() {
  for command in [
    "bin/runtest -w functional/argv && echo unexpected",
    "bin/runtest -w functional/argv | cat",
    "bin/runtest -w functional/argv > /tmp/rlibc-i060-bin-runtest-out",
    "./bin/runtest -w functional/argv && echo unexpected",
    "./bin/runtest -w functional/argv | cat",
    "./bin/runtest -w functional/argv > /tmp/rlibc-i060-dot-bin-runtest-out",
  ] {
    assert!(
      !command_uses_supported_runtest_prefix(command),
      "bin-runtest operator suffix must be rejected by manifest contract helper: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_shell_suffixes_for_dot_runtest_prefix() {
  for command in [
    "./runtest -w functional/argv || echo unexpected",
    "./runtest -w functional/argv && echo unexpected",
    "./runtest -w functional/argv | cat",
    "./runtest -w functional/argv > /tmp/rlibc-i060-dot-runtest-out",
  ] {
    assert!(
      !command_uses_supported_runtest_prefix(command),
      "dot-runtest variant with shell suffix must be rejected by manifest contract helper: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_comment_only_suffix_for_bin_runtest_prefixes() {
  for command in ["bin/runtest # comment", "./bin/runtest # comment"] {
    assert!(
      !command_uses_supported_runtest_prefix(command),
      "comment-only suffix for bin-runtest prefix must be rejected by manifest contract helper: {command}"
    );
  }
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_question_glob_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/arg?"),
    "question-mark glob tokens in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_backtick_substitution_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w `printf functional/argv`"),
    "backtick substitution in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_brace_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/{argv}"),
    "brace tokens in runtest suffix must be rejected by manifest contract helper"
  );
}

#[test]
fn command_uses_supported_runtest_prefix_rejects_bracket_suffix() {
  assert!(
    !command_uses_supported_runtest_prefix("runtest -w functional/[argv]"),
    "bracket tokens in runtest suffix must be rejected by manifest contract helper"
  );
}

#[cfg(unix)]
#[test]
fn quality_gate_script_is_executable() {
  let metadata = fs::metadata(repository_root().join("scripts/quality-gate.sh"))
    .expect("quality gate script metadata must be readable");
  let mode = metadata.permissions().mode();

  assert_ne!(
    mode & 0o111,
    0,
    "quality gate script must have at least one executable permission bit"
  );
}

#[cfg(unix)]
#[test]
fn libc_test_adapter_script_is_executable() {
  let metadata = fs::metadata(repository_root().join("scripts/conformance/libc-test-adapter.sh"))
    .expect("libc-test adapter script metadata must be readable");
  let mode = metadata.permissions().mode();

  assert_ne!(
    mode & 0o111,
    0,
    "libc-test adapter script must have at least one executable permission bit"
  );
}

#[test]
fn ci_workflow_routes_events_to_profiled_quality_gate_jobs() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for required_snippet in [
    "pull_request",
    "push",
    "schedule",
    "workflow_dispatch",
    "quality-gate-pr",
    "quality-gate-nightly",
    "quality-gate-full",
    "--profile pr",
    "--profile nightly",
    "--profile full",
  ] {
    assert!(
      workflow.contains(required_snippet),
      "ci workflow must contain: {required_snippet}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_jobs_bind_expected_event_conditions() {
  let workflow = read_repository_file(".github/workflows/ci.yml");
  let all_if_conditions = [
    "if: github.event_name == 'pull_request' || github.event_name == 'push'",
    "if: github.event_name == 'schedule'",
    "if: github.event_name == 'workflow_dispatch'",
  ];

  for (job_name, expected_if) in [
    (
      "quality-gate-pr",
      "if: github.event_name == 'pull_request' || github.event_name == 'push'",
    ),
    (
      "quality-gate-nightly",
      "if: github.event_name == 'schedule'",
    ),
    (
      "quality-gate-full",
      "if: github.event_name == 'workflow_dispatch'",
    ),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      job_block.contains(expected_if),
      "quality-gate job must keep expected event condition: {job_name}"
    );

    for if_condition in all_if_conditions {
      if if_condition == expected_if {
        continue;
      }

      assert!(
        !job_block.contains(if_condition),
        "quality-gate job must not include event condition from other profile variants: {job_name}"
      );
    }
  }
}

#[test]
fn ci_workflow_quality_gate_jobs_pin_runner_label() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "quality-gate-pr",
    "quality-gate-nightly",
    "quality-gate-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      job_block.contains("runs-on: ubuntu-latest"),
      "quality-gate job must pin ubuntu-latest runner label: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_jobs_keep_bootstrap_step_order() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, profile_step_name) in [
    ("quality-gate-pr", "Run PR Profile"),
    ("quality-gate-nightly", "Run Nightly Profile"),
    ("quality-gate-full", "Run Full Profile"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let checkout_index = job_block
      .find("      - name: Checkout")
      .unwrap_or_else(|| panic!("quality-gate job must include checkout step: {job_name}"));
    let install_index = job_block
      .find("      - name: Install Rust")
      .unwrap_or_else(|| panic!("quality-gate job must include rust install step: {job_name}"));
    let cache_index = job_block
      .find("      - name: Cache Cargo Artifacts")
      .unwrap_or_else(|| panic!("quality-gate job must include cargo cache step: {job_name}"));
    let profile_index = job_block
      .find(&format!("      - name: {profile_step_name}"))
      .unwrap_or_else(|| panic!("quality-gate job must include expected profile step: {job_name}"));

    assert!(
      checkout_index < install_index,
      "quality-gate job must run checkout before rust install: {job_name}"
    );
    assert!(
      install_index < cache_index,
      "quality-gate job must run rust install before cargo cache: {job_name}"
    );
    assert!(
      cache_index < profile_index,
      "quality-gate job must bootstrap cache before profile run step: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_jobs_define_timeouts() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for required_snippet in [
    "quality-gate-pr:",
    "timeout-minutes: 30",
    "quality-gate-nightly:",
    "timeout-minutes: 60",
    "quality-gate-full:",
    "timeout-minutes: 120",
  ] {
    assert!(
      workflow.contains(required_snippet),
      "ci workflow must define job timeout snippet: {required_snippet}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_jobs_bind_expected_timeout_windows() {
  let workflow = read_repository_file(".github/workflows/ci.yml");
  let all_timeout_snippets = [
    "timeout-minutes: 30",
    "timeout-minutes: 60",
    "timeout-minutes: 120",
  ];

  for (job_name, expected_timeout) in [
    ("quality-gate-pr", "timeout-minutes: 30"),
    ("quality-gate-nightly", "timeout-minutes: 60"),
    ("quality-gate-full", "timeout-minutes: 120"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      job_block.contains(expected_timeout),
      "quality-gate job must keep expected timeout window: {job_name}"
    );

    for timeout_snippet in all_timeout_snippets {
      if timeout_snippet == expected_timeout {
        continue;
      }

      assert!(
        !job_block.contains(timeout_snippet),
        "quality-gate job must not include timeout from a different profile variant: {job_name}"
      );
    }
  }
}

#[test]
fn ci_workflow_pr_profile_uses_continue_on_fail_for_full_signal() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  assert!(
    workflow.contains("run: bash scripts/quality-gate.sh --profile pr --continue-on-fail"),
    "ci workflow pr profile must use --continue-on-fail to execute all timeout-wrapped gates before final exit status"
  );
}

#[test]
fn ci_workflow_nightly_and_full_profiles_use_continue_on_fail_for_full_signal() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for required_snippet in [
    "run: bash scripts/quality-gate.sh --profile nightly --continue-on-fail",
    "run: bash scripts/quality-gate.sh --profile full --continue-on-fail",
  ] {
    assert!(
      workflow.contains(required_snippet),
      "nightly/full profile jobs must use --continue-on-fail for complete diagnostics: {required_snippet}"
    );
  }
}

#[test]
fn ci_workflow_profile_jobs_pin_continue_on_fail_run_count() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  assert_eq!(
    workflow
      .matches("run: bash scripts/quality-gate.sh --profile ")
      .count(),
    3,
    "ci workflow must define exactly three quality-gate profile run steps"
  );

  assert_eq!(
    workflow
      .matches("run: bash scripts/quality-gate.sh --profile pr --continue-on-fail")
      .count(),
    1,
    "ci workflow must define one PR profile run step with --continue-on-fail"
  );
  assert_eq!(
    workflow
      .matches("run: bash scripts/quality-gate.sh --profile nightly --continue-on-fail")
      .count(),
    1,
    "ci workflow must define one nightly profile run step with --continue-on-fail"
  );
  assert_eq!(
    workflow
      .matches("run: bash scripts/quality-gate.sh --profile full --continue-on-fail")
      .count(),
    1,
    "ci workflow must define one full profile run step with --continue-on-fail"
  );
}

#[test]
fn ci_workflow_profile_jobs_disable_failed_tests_file_writes() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for required_snippet in [
    "quality-gate-pr:",
    "quality-gate-nightly:",
    "quality-gate-full:",
  ] {
    assert!(
      workflow.contains(required_snippet),
      "ci workflow must contain profile job snippet: {required_snippet}"
    );
  }

  assert_eq!(
    workflow.matches("RLIBC_FAILURE_LOG_ENABLED: \"0\"").count(),
    3,
    "each quality-gate profile job must disable failed-tests file appends in CI"
  );
}

#[test]
fn ci_workflow_profile_steps_set_failure_log_env_on_run_step() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name) in [
    ("quality-gate-pr", "Run PR Profile"),
    ("quality-gate-nightly", "Run Nightly Profile"),
    ("quality-gate-full", "Run Full Profile"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);

    assert!(
      profile_step.contains("env:"),
      "quality-gate profile run step must define env block: {job_name}/{step_name}"
    );
    assert!(
      profile_step.contains("RLIBC_FAILURE_LOG_ENABLED: \"0\""),
      "quality-gate profile run step must disable failed-tests file appends: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_steps_place_failure_log_env_before_profile_run() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name, run_line) in [
    (
      "quality-gate-pr",
      "Run PR Profile",
      "run: bash scripts/quality-gate.sh --profile pr --continue-on-fail",
    ),
    (
      "quality-gate-nightly",
      "Run Nightly Profile",
      "run: bash scripts/quality-gate.sh --profile nightly --continue-on-fail",
    ),
    (
      "quality-gate-full",
      "Run Full Profile",
      "run: bash scripts/quality-gate.sh --profile full --continue-on-fail",
    ),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);
    let env_index = profile_step
      .find("RLIBC_FAILURE_LOG_ENABLED: \"0\"")
      .unwrap_or_else(|| panic!("profile step must set failure-log env: {job_name}/{step_name}"));
    let run_index = profile_step.find(run_line).unwrap_or_else(|| {
      panic!("profile step must execute expected run line: {job_name}/{step_name}")
    });

    assert!(
      env_index < run_index,
      "failure-log env must be declared before run command: {job_name}/{step_name}"
    );
    assert_eq!(
      profile_step.matches(run_line).count(),
      1,
      "profile step must contain expected run command exactly once: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_steps_define_single_failure_log_toggle() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name) in [
    ("quality-gate-pr", "Run PR Profile"),
    ("quality-gate-nightly", "Run Nightly Profile"),
    ("quality-gate-full", "Run Full Profile"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);

    assert_eq!(
      profile_step
        .matches("RLIBC_FAILURE_LOG_ENABLED: \"0\"")
        .count(),
      1,
      "profile step must define exactly one failure-log disable toggle: {job_name}/{step_name}"
    );
    assert!(
      !profile_step.contains("RLIBC_FAILURE_LOG_ENABLED: \"1\""),
      "profile step must not re-enable failed-tests file appends: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_run_steps_do_not_pin_optional_adapter_envs() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name) in [
    ("quality-gate-pr", "Run PR Profile"),
    ("quality-gate-nightly", "Run Nightly Profile"),
    ("quality-gate-full", "Run Full Profile"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);

    assert!(
      !profile_step.contains("RLIBC_LIBC_TEST_ROOT"),
      "profile run step must not pin libc-test adapter root directly: {job_name}/{step_name}"
    );
    assert!(
      !profile_step.contains("RLIBC_LTP_SUITE_ROOT"),
      "profile run step must not pin ltp adapter suite root directly: {job_name}/{step_name}"
    );
    assert!(
      !profile_step.contains("RLIBC_LTP_SUITE:"),
      "profile run step must not pin ltp suite selector directly: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_run_steps_do_not_override_failure_issue_id() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name) in [
    ("quality-gate-pr", "Run PR Profile"),
    ("quality-gate-nightly", "Run Nightly Profile"),
    ("quality-gate-full", "Run Full Profile"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);

    assert!(
      !profile_step.contains("RLIBC_FAILURE_ISSUE_ID"),
      "profile run step must not override failure issue id routing: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_run_steps_do_not_invoke_adapters_directly() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name) in [
    ("quality-gate-pr", "Run PR Profile"),
    ("quality-gate-nightly", "Run Nightly Profile"),
    ("quality-gate-full", "Run Full Profile"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);

    assert!(
      !profile_step.contains("scripts/conformance/libc-test-adapter.sh"),
      "profile run step must not call libc-test adapter directly; it should delegate through quality-gate profile flow: {job_name}/{step_name}"
    );
    assert!(
      !profile_step.contains("scripts/conformance/ltp-openposix-adapter.sh"),
      "profile run step must not call ltp/openposix adapter directly; it should delegate through quality-gate profile flow: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_run_steps_do_not_include_dry_run_flag() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name) in [
    ("quality-gate-pr", "Run PR Profile"),
    ("quality-gate-nightly", "Run Nightly Profile"),
    ("quality-gate-full", "Run Full Profile"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);

    assert!(
      !profile_step.contains("--dry-run"),
      "profile run step must not include --dry-run; dry-run is reserved for dedicated adapter prep steps: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_run_steps_pin_single_profile_flag() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name, profile_name) in [
    ("quality-gate-pr", "Run PR Profile", "pr"),
    ("quality-gate-nightly", "Run Nightly Profile", "nightly"),
    ("quality-gate-full", "Run Full Profile", "full"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);
    let expected_flag_pair = format!("--profile {profile_name} --continue-on-fail");

    assert_eq!(
      profile_step.matches("--profile ").count(),
      1,
      "profile run step must include exactly one --profile flag: {job_name}/{step_name}"
    );
    assert_eq!(
      profile_step.matches("--continue-on-fail").count(),
      1,
      "profile run step must include --continue-on-fail exactly once: {job_name}/{step_name}"
    );
    assert!(
      profile_step.contains(&expected_flag_pair),
      "profile run step must pin expected profile+flag pair: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_run_steps_do_not_use_equals_style_profile_flag() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name) in [
    ("quality-gate-pr", "Run PR Profile"),
    ("quality-gate-nightly", "Run Nightly Profile"),
    ("quality-gate-full", "Run Full Profile"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);

    assert!(
      !profile_step.contains("--profile="),
      "profile run step must use explicit space-separated --profile value, not equals style: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_profile_run_step_names_are_unique() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  assert_eq!(
    workflow.matches("      - name: Run PR Profile").count(),
    1,
    "ci workflow must define exactly one Run PR Profile step"
  );
  assert_eq!(
    workflow
      .matches("      - name: Run Nightly Profile")
      .count(),
    1,
    "ci workflow must define exactly one Run Nightly Profile step"
  );
  assert_eq!(
    workflow.matches("      - name: Run Full Profile").count(),
    1,
    "ci workflow must define exactly one Run Full Profile step"
  );
}

#[test]
fn ci_workflow_profile_jobs_pair_continue_on_fail_with_failure_log_disable() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for required_snippet in [
    "run: bash scripts/quality-gate.sh --profile pr --continue-on-fail",
    "run: bash scripts/quality-gate.sh --profile nightly --continue-on-fail",
    "run: bash scripts/quality-gate.sh --profile full --continue-on-fail",
    "RLIBC_FAILURE_LOG_ENABLED: \"0\"",
  ] {
    assert!(
      workflow.contains(required_snippet),
      "profile job contract must contain: {required_snippet}"
    );
  }
}

#[test]
fn ci_workflow_declares_libc_test_smoke_adapter_jobs() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for required_snippet in [
    "libc-test-smoke-nightly:",
    "libc-test-smoke-full:",
    "continue-on-error: true",
    "vars.RLIBC_LIBC_TEST_ROOT",
    "scripts/conformance/libc-test-adapter.sh --dry-run --profile docs/conformance/libc-test-smoke.txt",
    "run: bash scripts/conformance/libc-test-adapter.sh",
  ] {
    assert!(
      workflow.contains(required_snippet),
      "ci workflow must contain libc-test smoke snippet: {required_snippet}"
    );
  }
}

#[test]
fn ci_workflow_libc_test_smoke_jobs_use_explicit_manifest_profile() {
  let workflow = read_repository_file(".github/workflows/ci.yml");
  let explicit_profile_run = "run: bash scripts/conformance/libc-test-adapter.sh --profile docs/conformance/libc-test-smoke.txt";
  let explicit_profile_count = workflow.matches(explicit_profile_run).count();

  assert_eq!(
    explicit_profile_count, 2,
    "libc-test smoke jobs must pin the explicit smoke manifest profile in both jobs"
  );
}

#[test]
fn ci_workflow_declares_ltp_openposix_smoke_adapter_jobs() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for required_snippet in [
    "ltp-openposix-smoke-nightly:",
    "ltp-openposix-smoke-full:",
    "vars.RLIBC_LTP_SUITE_ROOT",
    "scripts/conformance/ltp-openposix-adapter.sh",
    "--suite \"${RLIBC_LTP_SUITE:-ltp}\"",
    "--suite-root \"${RLIBC_LTP_SUITE_ROOT}\"",
  ] {
    assert!(
      workflow.contains(required_snippet),
      "ci workflow must contain ltp/open_posix smoke snippet: {required_snippet}"
    );
  }
}

#[test]
fn ci_workflow_ltp_openposix_smoke_jobs_pin_explicit_suite_root_contract() {
  let workflow = read_repository_file(".github/workflows/ci.yml");
  let explicit_run = concat!(
    "run: bash scripts/conformance/ltp-openposix-adapter.sh --suite \"${",
    "RLIBC_LTP_SUITE:-ltp}\" --suite-root \"${",
    "RLIBC_LTP_SUITE_ROOT}\""
  );
  let explicit_root_env = "RLIBC_LTP_SUITE_ROOT: ${{ vars.RLIBC_LTP_SUITE_ROOT }}";

  assert_eq!(
    workflow.matches(explicit_run).count(),
    2,
    "ltp/open_posix smoke jobs must pin the same explicit suite-root command in nightly and full jobs"
  );
  assert_eq!(
    workflow.matches(explicit_root_env).count(),
    2,
    "ltp/open_posix smoke jobs must inject RLIBC_LTP_SUITE_ROOT from CI vars in both jobs"
  );
}

#[test]
fn ci_workflow_ltp_openposix_smoke_jobs_source_suite_name_from_repo_vars() {
  let workflow = read_repository_file(".github/workflows/ci.yml");
  let explicit_suite_env = "RLIBC_LTP_SUITE: ${{ vars.RLIBC_LTP_SUITE }}";
  let explicit_run = concat!(
    "run: bash scripts/conformance/ltp-openposix-adapter.sh --suite \"${",
    "RLIBC_LTP_SUITE:-ltp}\" --suite-root \"${",
    "RLIBC_LTP_SUITE_ROOT}\""
  );

  for job_name in ["ltp-openposix-smoke-nightly", "ltp-openposix-smoke-full"] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      job_block.contains(explicit_suite_env),
      "ltp/open_posix smoke job must source suite name from repository vars: {job_name}"
    );
    assert!(
      job_block.contains(explicit_run),
      "ltp/open_posix smoke job must keep suite fallback command contract: {job_name}"
    );
    assert!(
      !job_block.contains("--suite \"ltp\""),
      "ltp/open_posix smoke job must not hardcode suite name and bypass fallback env contract: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_smoke_jobs_depend_on_matching_quality_gate_profiles() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for required_snippet in [
    "libc-test-smoke-nightly:",
    "libc-test-smoke-full:",
    "ltp-openposix-smoke-nightly:",
    "ltp-openposix-smoke-full:",
  ] {
    assert!(
      workflow.contains(required_snippet),
      "ci workflow must contain smoke job snippet: {required_snippet}"
    );
  }

  assert_eq!(
    workflow.matches("needs: quality-gate-nightly").count(),
    2,
    "nightly smoke jobs must depend on quality-gate-nightly"
  );
  assert_eq!(
    workflow.matches("needs: quality-gate-full").count(),
    2,
    "full smoke jobs must depend on quality-gate-full"
  );
}

#[test]
fn ci_workflow_smoke_jobs_bind_to_correct_quality_gate_variant() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, expected_dependency) in [
    ("libc-test-smoke-nightly", "needs: quality-gate-nightly"),
    ("ltp-openposix-smoke-nightly", "needs: quality-gate-nightly"),
    ("libc-test-smoke-full", "needs: quality-gate-full"),
    ("ltp-openposix-smoke-full", "needs: quality-gate-full"),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      job_block.contains(expected_dependency),
      "smoke job must declare matching quality-gate dependency: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_profile_jobs_do_not_use_continue_on_error() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, step_name, profile_run_line) in [
    (
      "quality-gate-pr",
      "Run PR Profile",
      "run: bash scripts/quality-gate.sh --profile pr --continue-on-fail",
    ),
    (
      "quality-gate-nightly",
      "Run Nightly Profile",
      "run: bash scripts/quality-gate.sh --profile nightly --continue-on-fail",
    ),
    (
      "quality-gate-full",
      "Run Full Profile",
      "run: bash scripts/quality-gate.sh --profile full --continue-on-fail",
    ),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let profile_step = extract_job_step_block(&job_block, step_name);

    assert!(
      !profile_step.contains("continue-on-error: true"),
      "quality-gate profile run step must not use continue-on-error: {job_name}/{step_name}"
    );
    assert!(
      profile_step.contains(profile_run_line),
      "quality-gate profile run step must execute expected command: {job_name}/{step_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_jobs_remain_blocking_without_continue_on_error() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "quality-gate-pr",
    "quality-gate-nightly",
    "quality-gate-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      !job_block.contains("continue-on-error: true"),
      "quality-gate job must remain blocking and must not set continue-on-error: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_jobs_do_not_depend_on_other_jobs() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "quality-gate-pr",
    "quality-gate-nightly",
    "quality-gate-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      !job_block
        .lines()
        .any(|line| line.trim_start().starts_with("needs:")),
      "quality-gate job must stay dependency-free and start independently: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_pr_job_does_not_run_libc_test_dry_run_adapter() {
  let workflow = read_repository_file(".github/workflows/ci.yml");
  let pr_job_block = extract_top_level_job_block(&workflow, "quality-gate-pr");

  assert!(
    !pr_job_block.contains("Dry-Run libc-test Smoke Adapter"),
    "quality-gate-pr must not include dry-run libc-test adapter step"
  );
  assert!(
    !pr_job_block
      .contains("run: bash scripts/conformance/libc-test-adapter.sh --dry-run --profile docs/conformance/libc-test-smoke.txt"),
    "quality-gate-pr must stay independent from dry-run libc-test adapter invocation"
  );
  assert!(
    !pr_job_block.contains("RLIBC_LIBC_TEST_ROOT: /tmp/libc-test"),
    "quality-gate-pr must not pin libc-test root because dry-run adapter is nightly/full-only"
  );
}

#[test]
fn ci_workflow_nightly_and_full_jobs_run_dry_run_before_profile() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for (job_name, dry_run_step_name, profile_step_name) in [
    (
      "quality-gate-nightly",
      "Dry-Run libc-test Smoke Adapter",
      "Run Nightly Profile",
    ),
    (
      "quality-gate-full",
      "Dry-Run libc-test Smoke Adapter",
      "Run Full Profile",
    ),
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let dry_run_header = format!("      - name: {dry_run_step_name}");
    let profile_header = format!("      - name: {profile_step_name}");
    let dry_run_index = job_block
      .find(&dry_run_header)
      .unwrap_or_else(|| panic!("job `{job_name}` must contain dry-run step: {dry_run_step_name}"));
    let profile_index = job_block
      .find(&profile_header)
      .unwrap_or_else(|| panic!("job `{job_name}` must contain profile step: {profile_step_name}"));

    assert!(
      dry_run_index < profile_index,
      "job `{job_name}` must run dry-run adapter step before profile execution"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_dry_run_steps_remain_blocking() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in ["quality-gate-nightly", "quality-gate-full"] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let dry_run_step = extract_job_step_block(&job_block, "Dry-Run libc-test Smoke Adapter");

    assert!(
      dry_run_step.contains(
        "run: bash scripts/conformance/libc-test-adapter.sh --dry-run --profile docs/conformance/libc-test-smoke.txt"
      ),
      "dry-run smoke adapter step must use the explicit smoke profile command: {job_name}"
    );
    assert!(
      !dry_run_step.contains("continue-on-error: true"),
      "dry-run smoke adapter step must not mask failures in quality-gate jobs: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_dry_run_steps_pin_temp_libc_test_root() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in ["quality-gate-nightly", "quality-gate-full"] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let dry_run_step = extract_job_step_block(&job_block, "Dry-Run libc-test Smoke Adapter");

    assert!(
      dry_run_step.contains("env:"),
      "dry-run step must define env block for deterministic adapter context: {job_name}"
    );
    assert!(
      dry_run_step.contains("RLIBC_LIBC_TEST_ROOT: /tmp/libc-test"),
      "dry-run step must pin RLIBC_LIBC_TEST_ROOT to /tmp/libc-test: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_dry_run_steps_do_not_toggle_failure_log_env() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in ["quality-gate-nightly", "quality-gate-full"] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let dry_run_step = extract_job_step_block(&job_block, "Dry-Run libc-test Smoke Adapter");

    assert!(
      !dry_run_step.contains("RLIBC_FAILURE_LOG_ENABLED"),
      "dry-run step must not toggle failure-log env; it should stay scoped to profile run steps only: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_dry_run_steps_do_not_depend_on_repo_vars() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in ["quality-gate-nightly", "quality-gate-full"] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let dry_run_step = extract_job_step_block(&job_block, "Dry-Run libc-test Smoke Adapter");

    assert!(
      !dry_run_step.contains("${{ vars.RLIBC_LIBC_TEST_ROOT }}"),
      "dry-run step must use fixed local root path and must not depend on repo vars: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_dry_run_steps_do_not_include_continue_on_fail_flag() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in ["quality-gate-nightly", "quality-gate-full"] {
    let job_block = extract_top_level_job_block(&workflow, job_name);
    let dry_run_step = extract_job_step_block(&job_block, "Dry-Run libc-test Smoke Adapter");

    assert!(
      !dry_run_step.contains("--continue-on-fail"),
      "dry-run step must not include --continue-on-fail; that flag is only for quality-gate profile runs: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_quality_gate_dry_run_contract_occurs_exactly_twice() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  assert_eq!(
    workflow
      .matches("      - name: Dry-Run libc-test Smoke Adapter")
      .count(),
    2,
    "dry-run libc-test adapter step must exist exactly once per quality-gate nightly/full job"
  );
  assert_eq!(
    workflow
      .matches(
        "run: bash scripts/conformance/libc-test-adapter.sh --dry-run --profile docs/conformance/libc-test-smoke.txt",
      )
      .count(),
    2,
    "dry-run libc-test adapter command must appear exactly twice in ci workflow"
  );
  assert_eq!(
    workflow
      .matches("RLIBC_LIBC_TEST_ROOT: /tmp/libc-test")
      .count(),
    2,
    "dry-run libc-test root pin must appear exactly twice in ci workflow"
  );
}

#[test]
fn ci_workflow_smoke_jobs_are_non_blocking_with_continue_on_error() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "libc-test-smoke-nightly",
    "libc-test-smoke-full",
    "ltp-openposix-smoke-nightly",
    "ltp-openposix-smoke-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      job_block.contains("continue-on-error: true"),
      "smoke adapter job must remain non-blocking in CI: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_smoke_jobs_do_not_invoke_quality_gate_script_directly() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "libc-test-smoke-nightly",
    "libc-test-smoke-full",
    "ltp-openposix-smoke-nightly",
    "ltp-openposix-smoke-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      !job_block.contains("scripts/quality-gate.sh"),
      "smoke job must execute adapter commands only and must not invoke quality-gate.sh directly: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_smoke_jobs_do_not_toggle_failure_log_env() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "libc-test-smoke-nightly",
    "libc-test-smoke-full",
    "ltp-openposix-smoke-nightly",
    "ltp-openposix-smoke-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      !job_block.contains("RLIBC_FAILURE_LOG_ENABLED"),
      "smoke jobs must not toggle failure-log env; this contract is scoped to quality-gate profile run steps: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_smoke_jobs_do_not_override_failure_issue_id() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "libc-test-smoke-nightly",
    "libc-test-smoke-full",
    "ltp-openposix-smoke-nightly",
    "ltp-openposix-smoke-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      !job_block.contains("RLIBC_FAILURE_ISSUE_ID"),
      "smoke jobs must not override failure issue id routing; this belongs to local runner context: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_smoke_jobs_do_not_include_dry_run_flag() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "libc-test-smoke-nightly",
    "libc-test-smoke-full",
    "ltp-openposix-smoke-nightly",
    "ltp-openposix-smoke-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      !job_block.contains("--dry-run"),
      "smoke jobs must execute real adapter runs and must not include dry-run flag: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_smoke_jobs_do_not_include_continue_on_fail_flag() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "libc-test-smoke-nightly",
    "libc-test-smoke-full",
    "ltp-openposix-smoke-nightly",
    "ltp-openposix-smoke-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      !job_block.contains("--continue-on-fail"),
      "smoke jobs must not include --continue-on-fail; that flag belongs to quality-gate profile runs: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_smoke_jobs_do_not_use_quality_gate_profile_values() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "libc-test-smoke-nightly",
    "libc-test-smoke-full",
    "ltp-openposix-smoke-nightly",
    "ltp-openposix-smoke-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      !job_block.contains("--profile pr"),
      "smoke jobs must not use quality-gate pr profile flag: {job_name}"
    );
    assert!(
      !job_block.contains("--profile nightly"),
      "smoke jobs must not use quality-gate nightly profile flag: {job_name}"
    );
    assert!(
      !job_block.contains("--profile full"),
      "smoke jobs must not use quality-gate full profile flag: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_smoke_jobs_keep_adapter_timeout_window() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in [
    "libc-test-smoke-nightly",
    "libc-test-smoke-full",
    "ltp-openposix-smoke-nightly",
    "ltp-openposix-smoke-full",
  ] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      job_block.contains("timeout-minutes: 30"),
      "smoke adapter jobs must keep timeout-minutes: 30 for bounded CI runtime: {job_name}"
    );
  }
}

#[test]
fn ci_workflow_libc_test_smoke_jobs_source_root_from_repo_vars() {
  let workflow = read_repository_file(".github/workflows/ci.yml");

  for job_name in ["libc-test-smoke-nightly", "libc-test-smoke-full"] {
    let job_block = extract_top_level_job_block(&workflow, job_name);

    assert!(
      job_block.contains("RLIBC_LIBC_TEST_ROOT: ${{ vars.RLIBC_LIBC_TEST_ROOT }}"),
      "libc-test smoke job must source RLIBC_LIBC_TEST_ROOT from repository vars: {job_name}"
    );
    assert!(
      !job_block.contains("RLIBC_LIBC_TEST_ROOT: /tmp/libc-test"),
      "libc-test smoke job must not pin local /tmp root; that contract is dry-run-only: {job_name}"
    );
  }
}
