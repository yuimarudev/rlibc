#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirGuard {
  path: PathBuf,
}

impl TempDirGuard {
  fn new(prefix: &str) -> Self {
    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system clock must be after unix epoch")
      .as_nanos();
    let path = std::env::temp_dir().join(format!("rlibc-{prefix}-{unique}"));

    fs::create_dir_all(&path)
      .unwrap_or_else(|error| panic!("failed to create temp dir {}: {error}", path.display()));

    Self { path }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TempDirGuard {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.path);
  }
}

fn repository_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn adapter_script_path() -> PathBuf {
  repository_root().join("scripts/conformance/libc-test-adapter.sh")
}

fn write_text(path: &Path, content: &str) {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).unwrap_or_else(|error| {
      panic!(
        "failed to create parent directory {}: {error}",
        parent.display()
      )
    });
  }

  fs::write(path, content)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

fn make_executable(path: &Path) {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
      .unwrap_or_else(|error| panic!("failed to read metadata {}: {error}", path.display()))
      .permissions();

    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap_or_else(|error| {
      panic!(
        "failed to set executable bit on {}: {error}",
        path.display()
      )
    });
  }
}

fn run_adapter(arguments: &[String]) -> Output {
  run_adapter_with_env(arguments, &[])
}

fn run_adapter_with_env(arguments: &[String], env_pairs: &[(&str, &str)]) -> Output {
  let mut command = Command::new("bash");

  command.arg(adapter_script_path()).args(arguments);

  for (key, value) in env_pairs {
    command.env(key, value);
  }

  command
    .output()
    .expect("failed to execute I060 adapter script")
}

fn stderr_text(output: &Output) -> String {
  String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_text(output: &Output) -> String {
  String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn adapter_rejects_duplicate_case_ids() {
  let temp_dir = TempDirGuard::new("i060-duplicate-case-id");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-dup|echo first\ncase-dup|echo second\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success(), "duplicate case ids must fail");
  assert!(stderr.contains("duplicate smoke case id: case-dup"));
}

#[test]
fn adapter_rejects_unsorted_case_ids() {
  let temp_dir = TempDirGuard::new("i060-unsorted-case-id");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-b|echo second\ncase-a|echo first\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success(), "unsorted case ids must fail");
  assert!(stderr.contains("smoke case ids must be sorted"));
  assert!(stderr.contains("line 2"));
}

#[test]
fn adapter_trims_case_id_and_command_in_dry_run_output() {
  let temp_dir = TempDirGuard::new("i060-trim-whitespace");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "  case-whitespace  |   echo ok   \n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "single dry-run case must pass");
  assert!(stdout.contains("smoke case case-whitespace"));
  assert!(stdout.contains("dry-run command: echo ok"));
}

#[test]
fn adapter_rejects_entries_without_separator() {
  let temp_dir = TempDirGuard::new("i060-missing-separator");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "# comment line\nbroken-entry-without-separator\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "invalid manifest entries must fail"
  );
  assert!(stderr.contains("expected <case-id>|<command>"));
  assert!(stderr.contains("line 2"));
}

#[test]
fn adapter_rejects_case_id_with_internal_whitespace() {
  let temp_dir = TempDirGuard::new("i060-invalid-case-id");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "bad case|echo ok\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success(), "invalid case id must fail");
  assert!(stderr.contains("invalid smoke case id"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_reports_summary_for_unique_manifest() {
  let temp_dir = TempDirGuard::new("i060-summary");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|echo one\ncase-b|echo two\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "unique dry-run cases must pass");
  assert!(stdout.contains("smoke summary: total=2 failed=0"));
}

#[test]
fn adapter_rewrites_runtest_prefix_using_override_in_dry_run() {
  let temp_dir = TempDirGuard::new("i060-runtest-override");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|./runtest -w functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc-test/bin/runtest")],
  );
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "dry-run manifest must pass");
  assert!(stdout.contains("dry-run command: /opt/libc-test/bin/runtest -w functional/argv"));
}

#[test]
fn adapter_rejects_empty_runtest_override_when_set() {
  let temp_dir = TempDirGuard::new("i060-runtest-empty-override");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|./runtest -w functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(&arguments, &[("RLIBC_LIBC_TEST_RUNTEST", "   ")]);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "whitespace-only runtest override must fail"
  );
  assert!(stderr.contains("RLIBC_LIBC_TEST_RUNTEST must not be empty when set"));
}

#[test]
fn adapter_rejects_runtest_override_containing_whitespace() {
  let temp_dir = TempDirGuard::new("i060-runtest-whitespace-override");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|./runtest -w functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc test/bin/runtest")],
  );
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest override containing whitespace must fail"
  );
  assert!(stderr.contains("RLIBC_LIBC_TEST_RUNTEST must not contain whitespace"));
}

#[test]
fn adapter_rejects_runtest_override_with_shell_metacharacters() {
  let temp_dir = TempDirGuard::new("i060-runtest-meta-override");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|./runtest -w functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "./bin/runtest;echo_hacked")],
  );
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest override with shell metacharacters must fail"
  );
  assert!(
    stderr
      .contains("RLIBC_LIBC_TEST_RUNTEST must be a path-like token without shell metacharacters")
  );
}

#[test]
fn adapter_rewrites_bare_runtest_command_using_override_in_dry_run() {
  let temp_dir = TempDirGuard::new("i060-runtest-override-bare");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc-test/bin/runtest")],
  );
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "dry-run manifest must pass");
  assert!(stdout.contains("dry-run command: /opt/libc-test/bin/runtest -w functional/argv"));
}

#[test]
fn adapter_prefers_bin_runtest_when_root_runtest_is_missing() {
  let temp_dir = TempDirGuard::new("i060-bin-runtest");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");
  let libc_test_root = temp_dir.path().join("libc-test");
  let bin_runtest_path = libc_test_root.join("bin/runtest");

  write_text(&manifest_path, "case-a|./runtest -w functional/memcpy\n");
  write_text(&bin_runtest_path, "#!/usr/bin/env bash\nexit 0\n");
  make_executable(&bin_runtest_path);

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_ROOT", &libc_test_root.to_string_lossy())],
  );
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "dry-run manifest must pass");
  assert!(stdout.contains("dry-run command: ./bin/runtest -w functional/memcpy"));
}

#[test]
fn adapter_rewrites_bin_runtest_prefix_using_override_in_dry_run() {
  let temp_dir = TempDirGuard::new("i060-runtest-override-bin-prefix");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|./bin/runtest -w functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc-test/bin/runtest")],
  );
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "dry-run manifest must pass");
  assert!(stdout.contains("dry-run command: /opt/libc-test/bin/runtest -w functional/argv"));
}

#[test]
fn adapter_rewrites_bare_bin_runtest_prefix_using_override_in_dry_run() {
  let temp_dir = TempDirGuard::new("i060-runtest-override-bin-prefix-bare");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|bin/runtest -w functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc-test/bin/runtest")],
  );
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "dry-run manifest must pass");
  assert!(stdout.contains("dry-run command: /opt/libc-test/bin/runtest -w functional/argv"));
}

#[test]
fn adapter_rewrites_runtest_prefix_with_tab_separator_using_override() {
  let temp_dir = TempDirGuard::new("i060-runtest-override-tab");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest\t-w functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc-test/bin/runtest")],
  );
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "dry-run manifest must pass");
  assert!(stdout.contains("dry-run command: /opt/libc-test/bin/runtest -w functional/argv"));
}

#[test]
fn adapter_accepts_multiple_split_w_workload_pairs() {
  let temp_dir = TempDirGuard::new("i060-runtest-multiple-w-pairs");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv -w functional/ctype\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc-test/bin/runtest")],
  );
  let stdout = stdout_text(&output);

  assert!(
    output.status.success(),
    "multiple split -w workload pairs must remain valid"
  );
  assert!(stdout.contains(
    "dry-run command: /opt/libc-test/bin/runtest -w functional/argv -w functional/ctype"
  ));
}

#[test]
fn adapter_accepts_multiple_split_w_workload_pairs_for_bin_runtest_prefix() {
  let temp_dir = TempDirGuard::new("i060-bin-runtest-multiple-w-pairs");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|./bin/runtest -w functional/argv -w functional/ctype\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter_with_env(
    &arguments,
    &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc-test/bin/runtest")],
  );
  let stdout = stdout_text(&output);

  assert!(
    output.status.success(),
    "multiple split -w workload pairs must remain valid for ./bin/runtest prefix"
  );
  assert!(stdout.contains(
    "dry-run command: /opt/libc-test/bin/runtest -w functional/argv -w functional/ctype"
  ));
}

#[test]
fn adapter_accepts_nested_workload_path_for_supported_prefixes() {
  let commands = [
    "runtest -w functional/stdio/vfprintf",
    "./runtest -w functional/stdio/vfprintf",
    "bin/runtest -w functional/stdio/vfprintf",
    "./bin/runtest -w functional/stdio/vfprintf",
  ];

  for (index, command) in commands.iter().enumerate() {
    let temp_dir = TempDirGuard::new(&format!("i060-runtest-nested-workload-path-{index}"));
    let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

    write_text(&manifest_path, &format!("case-a|{command}\n"));

    let arguments = vec![
      "--dry-run".to_string(),
      "--profile".to_string(),
      manifest_path.to_string_lossy().into_owned(),
    ];
    let output = run_adapter_with_env(
      &arguments,
      &[("RLIBC_LIBC_TEST_RUNTEST", "/opt/libc-test/bin/runtest")],
    );
    let stdout = stdout_text(&output);

    assert!(
      output.status.success(),
      "nested workload path must remain valid for supported runtest prefixes: {command}"
    );
    assert!(
      stdout.contains("dry-run command: /opt/libc-test/bin/runtest -w functional/stdio/vfprintf")
    );
  }
}

#[test]
fn adapter_rejects_runtest_prefix_without_arguments() {
  let temp_dir = TempDirGuard::new("i060-runtest-no-args");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest prefix without args must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_bin_runtest_prefix_without_arguments() {
  for (name, command) in [
    ("i060-bin-runtest-no-args", "bin/runtest"),
    ("i060-dot-bin-runtest-no-args", "./bin/runtest"),
  ] {
    let temp_dir = TempDirGuard::new(name);
    let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

    write_text(&manifest_path, &format!("case-a|{command}\n"));

    let arguments = vec![
      "--dry-run".to_string(),
      "--profile".to_string(),
      manifest_path.to_string_lossy().into_owned(),
    ];
    let output = run_adapter(&arguments);
    let stderr = stderr_text(&output);

    assert!(
      !output.status.success(),
      "bin runtest variant without args must fail: {command}"
    );
    assert!(stderr.contains("runtest smoke command requires explicit arguments"));
    assert!(stderr.contains("line 1"));
  }
}

#[test]
fn adapter_rejects_runtest_prefix_without_w_workload_selector() {
  let temp_dir = TempDirGuard::new("i060-runtest-no-w-workload-selector");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest suffix without -w workload selector must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_missing_workload_after_w_flag() {
  let temp_dir = TempDirGuard::new("i060-runtest-missing-workload-after-w");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest -w without workload must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_absolute_workload_path() {
  let temp_dir = TempDirGuard::new("i060-runtest-absolute-workload-path");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w /functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success(), "absolute workload path must fail");
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_dotdot_workload_segment() {
  let temp_dir = TempDirGuard::new("i060-runtest-dotdot-workload-segment");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/../argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "dotdot workload segment must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_dot_workload_segment() {
  let temp_dir = TempDirGuard::new("i060-runtest-dot-workload-segment");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/./argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success(), "dot workload segment must fail");
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_empty_workload_path_segment() {
  let temp_dir = TempDirGuard::new("i060-runtest-empty-workload-path-segment");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional//argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "empty workload path segment must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_trailing_workload_path_separator() {
  let temp_dir = TempDirGuard::new("i060-runtest-trailing-workload-path-separator");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "trailing workload path separator must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_bin_runtest_prefix_with_invalid_workload_paths() {
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

  for command in ["bin/runtest", "./bin/runtest"] {
    for (index, workload) in invalid_workloads.iter().enumerate() {
      let temp_dir = TempDirGuard::new(&format!(
        "i060-bin-runtest-invalid-workload-{}-{}",
        command.replace(['/', '.'], "-"),
        index
      ));
      let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

      write_text(&manifest_path, &format!("case-a|{command} -w {workload}\n"));

      let arguments = vec![
        "--dry-run".to_string(),
        "--profile".to_string(),
        manifest_path.to_string_lossy().into_owned(),
      ];
      let output = run_adapter(&arguments);
      let stderr = stderr_text(&output);

      assert!(
        !output.status.success(),
        "invalid workload path for bin-runtest prefix must fail: {command} -w {workload}"
      );
      assert!(stderr.contains("runtest smoke command requires explicit arguments"));
      assert!(stderr.contains("line 1"));
    }
  }
}

#[test]
fn adapter_rejects_dot_runtest_prefix_with_invalid_workload_paths() {
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

  for (index, workload) in invalid_workloads.iter().enumerate() {
    let temp_dir = TempDirGuard::new(&format!("i060-dot-runtest-invalid-workload-{index}"));
    let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

    write_text(&manifest_path, &format!("case-a|./runtest -w {workload}\n"));

    let arguments = vec![
      "--dry-run".to_string(),
      "--profile".to_string(),
      manifest_path.to_string_lossy().into_owned(),
    ];
    let output = run_adapter(&arguments);
    let stderr = stderr_text(&output);

    assert!(
      !output.status.success(),
      "invalid workload path for ./runtest prefix must fail: ./runtest -w {workload}"
    );
    assert!(stderr.contains("runtest smoke command requires explicit arguments"));
    assert!(stderr.contains("line 1"));
  }
}

#[test]
fn adapter_rejects_bare_runtest_prefix_with_invalid_workload_paths() {
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

  for (index, workload) in invalid_workloads.iter().enumerate() {
    let temp_dir = TempDirGuard::new(&format!("i060-bare-runtest-invalid-workload-{index}"));
    let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

    write_text(&manifest_path, &format!("case-a|runtest -w {workload}\n"));

    let arguments = vec![
      "--dry-run".to_string(),
      "--profile".to_string(),
      manifest_path.to_string_lossy().into_owned(),
    ];
    let output = run_adapter(&arguments);
    let stderr = stderr_text(&output);

    assert!(
      !output.status.success(),
      "invalid workload path for runtest prefix must fail: runtest -w {workload}"
    );
    assert!(stderr.contains("runtest smoke command requires explicit arguments"));
    assert!(stderr.contains("line 1"));
  }
}

#[test]
fn adapter_rejects_runtest_prefix_with_option_like_workload_after_w_flag() {
  let temp_dir = TempDirGuard::new("i060-runtest-option-like-workload");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w --all\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest -w followed by option-like token must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_trailing_option_like_token_outside_w_pair() {
  let temp_dir = TempDirGuard::new("i060-runtest-trailing-option-like-token");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/argv --all\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "option-like tokens outside -w pairs must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_trailing_positional_token_outside_w_pair() {
  let temp_dir = TempDirGuard::new("i060-runtest-trailing-positional-token");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv functional/ctype\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "positional tokens outside -w pairs must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_empty_workload_equals_syntax() {
  let temp_dir = TempDirGuard::new("i060-runtest-empty-workload-equals");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w=\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest -w= without workload must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_non_empty_workload_equals_syntax() {
  let temp_dir = TempDirGuard::new("i060-runtest-non-empty-workload-equals");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w=functional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest -w=<workload> attached syntax must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_non_empty_workload_equals_syntax_after_valid_pair() {
  let temp_dir = TempDirGuard::new("i060-runtest-non-empty-workload-equals-after-valid-pair");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv -w=functional/errno\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "runtest -w=<workload> syntax after a valid -w pair must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_attached_w_workload_token() {
  let temp_dir = TempDirGuard::new("i060-runtest-attached-w-workload");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -wfunctional/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "attached -w workload token must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_attached_w_workload_token_after_valid_pair() {
  let temp_dir = TempDirGuard::new("i060-runtest-attached-w-workload-after-valid-pair");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv -wfunctional/errno\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "attached -w workload token after a valid -w pair must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_comment_only_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-comment-only");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest # comment-only suffix\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "comment-only runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command requires explicit arguments"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_bin_runtest_prefix_with_comment_only_suffix() {
  for (name, command) in [
    ("i060-bin-runtest-comment-only", "bin/runtest"),
    ("i060-dot-bin-runtest-comment-only", "./bin/runtest"),
  ] {
    let temp_dir = TempDirGuard::new(name);
    let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

    write_text(
      &manifest_path,
      &format!("case-a|{command} # comment-only suffix\n"),
    );

    let arguments = vec![
      "--dry-run".to_string(),
      "--profile".to_string(),
      manifest_path.to_string_lossy().into_owned(),
    ];
    let output = run_adapter(&arguments);
    let stderr = stderr_text(&output);

    assert!(
      !output.status.success(),
      "comment-only bin-runtest suffix must fail: {command}"
    );
    assert!(stderr.contains("runtest smoke command requires explicit arguments"));
    assert!(stderr.contains("line 1"));
  }
}

#[test]
fn adapter_rejects_runtest_prefix_with_shell_operator_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-shell-operator");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv; echo unexpected\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "shell operators in runtest command suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_logical_and_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-logical-and");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv && echo unexpected\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "logical-and operator in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_logical_or_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-logical-or");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv || echo unexpected\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "logical-or operator in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_positional_token_and_logical_or_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-positional-and-logical-or");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv functional/ctype || echo unexpected\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "positional token plus logical-or suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_dot_runtest_prefix_with_shell_operator_suffixes() {
  for (name, command) in [
    (
      "i060-dot-runtest-logical-or-suffix",
      "./runtest -w functional/argv || echo unexpected",
    ),
    (
      "i060-dot-runtest-logical-and-suffix",
      "./runtest -w functional/argv && echo unexpected",
    ),
    (
      "i060-dot-runtest-pipe-suffix",
      "./runtest -w functional/argv | cat",
    ),
    (
      "i060-dot-runtest-output-redirection-suffix",
      "./runtest -w functional/argv > /tmp/rlibc-i060-dot-runtest-out",
    ),
  ] {
    let temp_dir = TempDirGuard::new(name);
    let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

    write_text(&manifest_path, &format!("case-a|{command}\n"));

    let arguments = vec![
      "--dry-run".to_string(),
      "--profile".to_string(),
      manifest_path.to_string_lossy().into_owned(),
    ];
    let output = run_adapter(&arguments);
    let stderr = stderr_text(&output);

    assert!(
      !output.status.success(),
      "shell-operator suffix for ./runtest prefix must fail: {command}"
    );
    assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
    assert!(stderr.contains("line 1"));
  }
}

#[test]
fn adapter_rejects_runtest_prefix_with_pipe_operator_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-pipe-operator");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/argv | cat\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "pipe operator in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_output_redirection_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-output-redirection");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv > /tmp/rlibc-i060-out\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "output redirection in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_input_redirection_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-input-redirection");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv < /tmp/rlibc-i060-in\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "input redirection in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_trailing_inline_comment() {
  let temp_dir = TempDirGuard::new("i060-runtest-inline-comment");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv # inline comment\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "inline comment in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_bin_runtest_prefix_with_trailing_inline_comment() {
  let temp_dir = TempDirGuard::new("i060-bin-runtest-inline-comment");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|bin/runtest -w functional/argv # inline comment\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "inline comment in bin/runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_background_operator_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-background-operator");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv & echo unexpected\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "background operator in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_glob_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-glob");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/*\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "glob operator in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_question_glob_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-question-glob");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/arg?\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "question-mark glob operator in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_env_variable_expansion_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-env-expansion");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/$TARGET_CASE\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "env-variable expansion in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_command_substitution_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-command-substitution");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w $(printf functional/argv)\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "command substitution in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_backtick_substitution_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-backtick-substitution");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w `printf functional/argv`\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "backtick substitution in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_backslash_escape_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-backslash-escape");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional\\/argv\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "backslash escapes in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_quote_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-quote");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w \"functional/argv\"\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success(), "quoted runtest suffix must fail");
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_bang_operator_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-bang-operator");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(
    &manifest_path,
    "case-a|runtest -w functional/argv ! echo unexpected\n",
  );

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "bang operator in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_single_quote_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-single-quote");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w 'functional/argv'\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "single quotes in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_double_quote_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-double-quote");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w \"functional/argv\"\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "double quotes in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_parenthesis_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-parenthesis");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/(argv)\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "parenthesis tokens in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_brace_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-brace");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/{argv}\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "brace tokens in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}

#[test]
fn adapter_rejects_runtest_prefix_with_bracket_suffix() {
  let temp_dir = TempDirGuard::new("i060-runtest-bracket");
  let manifest_path = temp_dir.path().join("libc-test-smoke.txt");

  write_text(&manifest_path, "case-a|runtest -w functional/[argv]\n");

  let arguments = vec![
    "--dry-run".to_string(),
    "--profile".to_string(),
    manifest_path.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(
    !output.status.success(),
    "bracket tokens in runtest suffix must fail"
  );
  assert!(stderr.contains("runtest smoke command contains unsupported shell operators"));
  assert!(stderr.contains("line 1"));
}
