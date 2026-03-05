#![cfg(unix)]

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::fs::symlink;
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
  repository_root().join("scripts/conformance/ltp-openposix-adapter.sh")
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

fn run_adapter(arguments: &[String]) -> Output {
  Command::new("bash")
    .arg(adapter_script_path())
    .args(arguments)
    .output()
    .expect("failed to execute I061 adapter script")
}

fn stderr_text(output: &Output) -> String {
  String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_text(output: &Output) -> String {
  String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn adapter_script_is_executable() {
  let metadata = fs::metadata(adapter_script_path()).expect("adapter script must exist");
  let mode = metadata.permissions().mode();

  assert_ne!(mode & 0o111, 0, "adapter script must be executable");
}

#[test]
fn adapter_rejects_missing_suite_root() {
  let temp_dir = TempDirGuard::new("i061-missing-root");
  let missing_root = temp_dir.path().join("missing-suite-root");
  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    missing_root.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("suite root does not exist"));
}

#[test]
fn adapter_rejects_missing_suite_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-missing-suite");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-missing-suite.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite is required"));
  assert!(
    !marker.exists(),
    "suite command must not run when --suite is missing"
  );
}

#[test]
fn adapter_rejects_missing_suite_root_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-missing-suite-root-required");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-missing-suite-root.marker");
  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite-root is required"));
  assert!(
    !marker.exists(),
    "suite command must not run when --suite-root is missing"
  );
}

#[test]
fn adapter_rejects_missing_suite_root_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-missing-suite-root-override");
  let missing_root = temp_dir.path().join("missing-suite-root");
  let valid_root = temp_dir.path().join("suite-root-valid");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-missing-suite-root-override.marker");

  fs::create_dir_all(&valid_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      valid_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    missing_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    valid_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("suite root does not exist"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --suite-root value is missing"
  );
}

#[test]
fn adapter_rejects_non_directory_suite_root_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-suite-root-non-directory");
  let suite_root_file = temp_dir.path().join("suite-root-file");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-suite-root-file.marker");

  write_text(&suite_root_file, "not a directory\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root_file.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("suite root must be a directory"));
  assert!(
    !marker.exists(),
    "suite command must not run when --suite-root points to a file"
  );
}

#[test]
fn adapter_rejects_non_directory_suite_root_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-suite-root-non-directory-override");
  let invalid_suite_root = temp_dir.path().join("suite-root-file");
  let valid_suite_root = temp_dir.path().join("suite-root-valid");
  let valid_results_file = valid_suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-suite-root-file-override.marker");

  write_text(&invalid_suite_root, "not a directory\n");
  write_text(&valid_results_file, "PASS override.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    invalid_suite_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    valid_suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("suite root must be a directory"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --suite-root value points to a file"
  );
}

#[test]
fn adapter_rejects_uncanonicalizable_suite_root_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-suite-root-uncanonicalizable-override");
  let invalid_suite_root = temp_dir.path().join("suite-root-no-exec");
  let valid_suite_root = temp_dir.path().join("suite-root-valid");
  let valid_results_file = valid_suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-suite-root-no-exec-override.marker");

  fs::create_dir_all(&invalid_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create invalid suite root {}: {error}",
      invalid_suite_root.display()
    )
  });

  let original_mode = fs::metadata(&invalid_suite_root)
    .unwrap_or_else(|error| {
      panic!(
        "failed to stat invalid suite root {}: {error}",
        invalid_suite_root.display()
      )
    })
    .permissions()
    .mode();
  let mut no_exec_permissions = fs::metadata(&invalid_suite_root)
    .unwrap_or_else(|error| {
      panic!(
        "failed to re-stat invalid suite root {}: {error}",
        invalid_suite_root.display()
      )
    })
    .permissions();

  no_exec_permissions.set_mode(0o000);
  fs::set_permissions(&invalid_suite_root, no_exec_permissions).unwrap_or_else(|error| {
    panic!(
      "failed to remove execute bits from invalid suite root {}: {error}",
      invalid_suite_root.display()
    )
  });
  write_text(&valid_results_file, "PASS override.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    invalid_suite_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    valid_suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);
  let mut restore_permissions = fs::metadata(&invalid_suite_root)
    .unwrap_or_else(|error| {
      panic!(
        "failed to stat invalid suite root for restore {}: {error}",
        invalid_suite_root.display()
      )
    })
    .permissions();

  restore_permissions.set_mode(original_mode);
  fs::set_permissions(&invalid_suite_root, restore_permissions).unwrap_or_else(|error| {
    panic!(
      "failed to restore permissions on invalid suite root {}: {error}",
      invalid_suite_root.display()
    )
  });

  assert!(!output.status.success());
  assert!(stderr.contains("failed to canonicalize suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --suite-root value cannot be canonicalized"
  );
}

#[test]
fn adapter_rejects_uncanonicalizable_suite_root_before_later_unknown_argument() {
  let temp_dir = TempDirGuard::new("i061-suite-root-uncanonicalizable-before-unknown");
  let invalid_suite_root = temp_dir.path().join("suite-root-no-exec");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-suite-root-no-exec-before-unknown.marker");

  fs::create_dir_all(&invalid_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create invalid suite root {}: {error}",
      invalid_suite_root.display()
    )
  });

  let original_mode = fs::metadata(&invalid_suite_root)
    .unwrap_or_else(|error| {
      panic!(
        "failed to stat invalid suite root {}: {error}",
        invalid_suite_root.display()
      )
    })
    .permissions()
    .mode();
  let mut no_exec_permissions = fs::metadata(&invalid_suite_root)
    .unwrap_or_else(|error| {
      panic!(
        "failed to re-stat invalid suite root {}: {error}",
        invalid_suite_root.display()
      )
    })
    .permissions();

  no_exec_permissions.set_mode(0o000);
  fs::set_permissions(&invalid_suite_root, no_exec_permissions).unwrap_or_else(|error| {
    panic!(
      "failed to remove execute bits from invalid suite root {}: {error}",
      invalid_suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    invalid_suite_root.to_string_lossy().into_owned(),
    "--unknown-after-invalid-suite-root".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);
  let mut restore_permissions = fs::metadata(&invalid_suite_root)
    .unwrap_or_else(|error| {
      panic!(
        "failed to stat invalid suite root for restore {}: {error}",
        invalid_suite_root.display()
      )
    })
    .permissions();

  restore_permissions.set_mode(original_mode);
  fs::set_permissions(&invalid_suite_root, restore_permissions).unwrap_or_else(|error| {
    panic!(
      "failed to restore permissions on invalid suite root {}: {error}",
      invalid_suite_root.display()
    )
  });

  assert!(!output.status.success());
  assert!(stderr.contains("failed to canonicalize suite root"));
  assert!(
    !stderr.contains("unknown argument"),
    "suite-root canonicalization error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when suite-root canonicalization fails"
  );
}

#[test]
fn adapter_rejects_unsupported_suite_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-unsupported-suite");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-unsupported-suite.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "unsupported-suite".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite must be one of: ltp, open_posix_testsuite"));
  assert!(
    !marker.exists(),
    "suite command must not run when --suite value is unsupported"
  );
}

#[test]
fn adapter_rejects_unsupported_suite_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-unsupported-suite-override");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-unsupported-suite-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "unsupported-suite".to_string(),
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite must be one of: ltp, open_posix_testsuite"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --suite value is unsupported"
  );
}

#[test]
fn adapter_rejects_empty_suite_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-empty-suite");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir.path().join("suite-command-ran-empty-suite.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    String::new(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite must not be empty"));
  assert!(
    !marker.exists(),
    "suite command must not run when --suite is empty"
  );
}

#[test]
fn adapter_rejects_empty_suite_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-empty-suite-override");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-empty-suite-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    String::new(),
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite must not be empty"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --suite value is empty"
  );
}

#[test]
fn adapter_rejects_suite_separator_token_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-suite-separator-token");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-suite-separator.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "--".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite requires a value"));
  assert!(
    !marker.exists(),
    "suite command must not run when --suite value is missing"
  );
}

#[test]
fn adapter_rejects_suite_separator_token_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-suite-separator-override");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-suite-separator-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "--".to_string(),
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite requires a value"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --suite value is missing"
  );
}

#[test]
fn adapter_rejects_suite_root_separator_token_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-suite-root-separator-token");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-suite-root-separator.marker");
  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    "--".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite-root requires a value"));
  assert!(
    !marker.exists(),
    "suite command must not run when --suite-root value is missing"
  );
}

#[test]
fn adapter_rejects_suite_root_separator_token_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-suite-root-separator-override");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-suite-root-separator-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    "--".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite-root requires a value"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --suite-root value is separator token"
  );
}

#[test]
fn adapter_rejects_empty_suite_root_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-empty-suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-empty-suite-root.marker");
  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    String::new(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite-root must not be empty"));
  assert!(
    !marker.exists(),
    "suite command must not run when --suite-root is empty"
  );
}

#[test]
fn adapter_rejects_empty_suite_root_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-empty-suite-root-override");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-empty-suite-root-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    String::new(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--suite-root must not be empty"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --suite-root value is empty"
  );
}

#[test]
fn adapter_rejects_unknown_argument_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-unknown-argument");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-unknown-argument.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--unknown-option".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("unknown argument: --unknown-option"));
  assert!(
    !marker.exists(),
    "suite command must not run when an unknown option is provided"
  );
}

#[test]
fn adapter_rejects_empty_results_file_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-empty-results-file");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-empty-results.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    String::new(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--results-file must not be empty"));
  assert!(
    !marker.exists(),
    "suite command must not run when --results-file is empty"
  );
}

#[test]
fn adapter_rejects_empty_results_file_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-empty-results-file-override");
  let suite_root = temp_dir.path().join("suite-root");
  let results_file = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-empty-results-override.marker");

  write_text(&results_file, "PASS override.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    String::new(),
    "--results-file".to_string(),
    results_file.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--results-file must not be empty"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --results-file value is empty"
  );
}

#[test]
fn adapter_rejects_results_file_separator_token_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-file-separator-token");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-separator-token.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "--".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--results-file requires a value"));
  assert!(
    !marker.exists(),
    "suite command must not run when --results-file value is missing"
  );
}

#[test]
fn adapter_rejects_results_file_separator_token_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-results-file-separator-token-override");
  let suite_root = temp_dir.path().join("suite-root");
  let results_file = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-separator-token-override.marker");

  write_text(&results_file, "PASS separator.override.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "--".to_string(),
    "--results-file".to_string(),
    results_file.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--results-file requires a value"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --results-file value is missing"
  );
}

#[test]
fn adapter_rejects_results_file_separator_token_before_suite_root_even_when_later_value_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new(
    "i061-results-file-separator-token-before-suite-root-even-when-later-valid-before-unknown",
  );
  let suite_root = temp_dir.path().join("suite-root");
  let results_file = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir.path().join(
    "suite-command-ran-separator-token-before-suite-root-even-when-later-valid-before-unknown.marker",
  );

  write_text(&results_file, "PASS separator.override.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    "--".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    results_file.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--results-file requires a value"));
  assert!(
    !stderr.contains("unknown argument"),
    "missing results-file value error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any --results-file value is missing"
  );
}

#[test]
fn adapter_rejects_results_file_outside_suite_root() {
  let temp_dir = TempDirGuard::new("i061-results-outside-root");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results = temp_dir.path().join("external-results.txt");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    external_results.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
}

#[test]
fn adapter_rejects_out_of_root_results_file_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-outside-root-preflight");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results = temp_dir.path().join("external-results.txt");
  let marker = temp_dir.path().join("suite-command-ran.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    external_results.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when results file preflight is invalid"
  );
}

#[test]
fn adapter_rejects_out_of_root_results_file_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-results-outside-root-override");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results = temp_dir.path().join("external-results.txt");
  let valid_results = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-outside-root-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");
  write_text(&valid_results, "PASS valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    external_results.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --results-file value escapes suite root"
  );
}

#[test]
fn adapter_rejects_out_of_root_results_file_before_later_unknown_argument() {
  let temp_dir = TempDirGuard::new("i061-results-outside-root-before-unknown");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results = temp_dir.path().join("external-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-outside-root-before-unknown.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    external_results.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !stderr.contains("unknown argument"),
    "results-file validation error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when results-file preflight fails"
  );
}

#[test]
fn adapter_rejects_out_of_root_results_file_before_suite_root_and_before_later_unknown_argument() {
  let temp_dir = TempDirGuard::new("i061-results-outside-root-before-suite-root-before-unknown");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results = temp_dir.path().join("external-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-outside-root-before-suite-root-before-unknown.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    external_results.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !stderr.contains("unknown argument"),
    "results-file validation error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when out-of-root results-file is specified before suite-root"
  );
}

#[test]
fn adapter_rejects_results_file_that_becomes_out_of_root_after_suite_root_override_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new("i061-results-becomes-out-of-root-after-suite-root-override");
  let first_suite_root = temp_dir.path().join("suite-root-a");
  let second_suite_root = temp_dir.path().join("suite-root-b");
  let results_under_first_root = first_suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-results-becomes-out-of-root.marker");

  fs::create_dir_all(&first_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create first suite root {}: {error}",
      first_suite_root.display()
    )
  });
  fs::create_dir_all(&second_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create second suite root {}: {error}",
      second_suite_root.display()
    )
  });
  write_text(&results_under_first_root, "PASS pre.override.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    results_under_first_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    first_suite_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    second_suite_root.to_string_lossy().into_owned(),
    "--unknown-after-suite-root-override".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !stderr.contains("unknown argument"),
    "results-file/suite-root validation error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when suite-root override invalidates prior --results-file"
  );
}

#[test]
fn adapter_rejects_results_file_invalidated_by_suite_root_override_even_when_later_results_file_is_valid()
 {
  let temp_dir =
    TempDirGuard::new("i061-results-invalidated-by-suite-root-override-with-later-valid-file");
  let first_suite_root = temp_dir.path().join("suite-root-a");
  let second_suite_root = temp_dir.path().join("suite-root-b");
  let results_under_first_root = first_suite_root.join("results/ltp-results-a.txt");
  let results_under_second_root = second_suite_root.join("results/ltp-results-b.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-results-invalidated-by-suite-root-override.marker");

  fs::create_dir_all(&first_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create first suite root {}: {error}",
      first_suite_root.display()
    )
  });
  fs::create_dir_all(&second_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create second suite root {}: {error}",
      second_suite_root.display()
    )
  });
  write_text(&results_under_first_root, "PASS root.a.case\n");
  write_text(&results_under_second_root, "PASS root.b.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    results_under_first_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    first_suite_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    second_suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    results_under_second_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when any earlier --results-file is invalidated by suite-root override"
  );
}

#[test]
fn adapter_rejects_symlinked_ancestor_results_file_invalidated_by_suite_root_override_even_when_later_results_file_is_valid()
 {
  let temp_dir =
    TempDirGuard::new("i061-symlink-ancestor-results-invalidated-by-suite-root-override");
  let first_suite_root = temp_dir.path().join("suite-root-a");
  let second_suite_root = temp_dir.path().join("suite-root-b");
  let first_results_real = first_suite_root.join("results-real");
  let first_results_link = first_suite_root.join("results-link");
  let first_results_file = first_results_link.join("ltp-results-a.txt");
  let second_results_file = second_suite_root.join("results/ltp-results-b.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-symlink-ancestor-results-invalidated-by-suite-root-override.marker");

  fs::create_dir_all(&first_results_real).unwrap_or_else(|error| {
    panic!(
      "failed to create first results real directory {}: {error}",
      first_results_real.display()
    )
  });
  fs::create_dir_all(&second_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create second suite root {}: {error}",
      second_suite_root.display()
    )
  });
  symlink(&first_results_real, &first_results_link).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      first_results_link.display(),
      first_results_real.display()
    )
  });
  write_text(
    &first_results_real.join("ltp-results-a.txt"),
    "PASS root.a.symlink.case\n",
  );
  write_text(&second_results_file, "PASS root.b.valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    first_results_file.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    first_suite_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    second_suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    second_results_file.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when an earlier symlinked-ancestor results file becomes out-of-root after suite-root override"
  );
}

#[test]
fn adapter_rejects_symlinked_ancestor_results_file_invalidated_by_suite_root_override_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new(
    "i061-symlink-ancestor-results-invalidated-by-suite-root-override-before-unknown",
  );
  let first_suite_root = temp_dir.path().join("suite-root-a");
  let second_suite_root = temp_dir.path().join("suite-root-b");
  let first_results_real = first_suite_root.join("results-real");
  let first_results_link = first_suite_root.join("results-link");
  let first_results_file = first_results_link.join("ltp-results-a.txt");
  let second_results_file = second_suite_root.join("results/ltp-results-b.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-symlink-ancestor-results-invalidated-before-unknown.marker");

  fs::create_dir_all(&first_results_real).unwrap_or_else(|error| {
    panic!(
      "failed to create first results real directory {}: {error}",
      first_results_real.display()
    )
  });
  fs::create_dir_all(&second_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create second suite root {}: {error}",
      second_suite_root.display()
    )
  });
  symlink(&first_results_real, &first_results_link).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      first_results_link.display(),
      first_results_real.display()
    )
  });
  write_text(
    &first_results_real.join("ltp-results-a.txt"),
    "PASS root.a.symlink.case\n",
  );
  write_text(&second_results_file, "PASS root.b.valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    first_results_file.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    first_suite_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    second_suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    second_results_file.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !stderr.contains("unknown argument"),
    "suite-root override invalidation should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any earlier symlinked-ancestor results file is invalidated by suite-root override"
  );
}

#[test]
fn adapter_rejects_missing_parent_out_of_root_results_file_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-outside-root-preflight-missing-parent");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results = temp_dir.path().join("outside-dir/results.txt");
  let marker = temp_dir.path().join("suite-command-ran.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let suite_command = format!(
    "mkdir -p '{}' && printf 'PASS generated.case\\n' > '{}' && touch '{}'",
    external_results
      .parent()
      .expect("external results path must have a parent")
      .display(),
    external_results.display(),
    marker.display(),
  );
  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    external_results.to_string_lossy().into_owned(),
    "--".to_string(),
    "bash".to_string(),
    "-lc".to_string(),
    suite_command,
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when out-of-root results_file parent is absent"
  );
  assert!(
    !external_results.exists(),
    "out-of-root results file must not be created by suite command"
  );
}

#[test]
fn adapter_rejects_missing_parent_out_of_root_results_file_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-results-outside-root-missing-parent-override");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results = temp_dir.path().join("outside-dir/results.txt");
  let valid_results = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-outside-root-missing-parent-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });
  write_text(&valid_results, "PASS valid.case\n");

  let suite_command = format!(
    "mkdir -p '{}' && printf 'PASS generated.case\\n' > '{}' && touch '{}'",
    external_results
      .parent()
      .expect("external results path must have a parent")
      .display(),
    external_results.display(),
    marker.display(),
  );
  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    external_results.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--".to_string(),
    "bash".to_string(),
    "-lc".to_string(),
    suite_command,
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --results-file value escapes suite root"
  );
  assert!(
    !external_results.exists(),
    "out-of-root results file must not be created by suite command"
  );
}

#[test]
fn adapter_rejects_missing_parent_out_of_root_results_file_before_suite_root_and_before_later_unknown_argument()
 {
  let temp_dir =
    TempDirGuard::new("i061-results-outside-root-missing-parent-before-suite-root-before-unknown");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results = temp_dir.path().join("outside-dir/results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-outside-root-missing-parent-before-suite-root-before-unknown.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let suite_command = format!(
    "mkdir -p '{}' && printf 'PASS generated.case\\n' > '{}' && touch '{}'",
    external_results
      .parent()
      .expect("external results path must have a parent")
      .display(),
    external_results.display(),
    marker.display(),
  );
  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    external_results.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "bash".to_string(),
    "-lc".to_string(),
    suite_command,
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !stderr.contains("unknown argument"),
    "results-file preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when missing-parent out-of-root results-file is detected"
  );
  assert!(
    !external_results.exists(),
    "out-of-root missing-parent results file must not be created by suite command"
  );
}

#[test]
fn adapter_rejects_missing_parent_results_file_invalidated_by_suite_root_override_even_when_later_results_file_is_valid()
 {
  let temp_dir =
    TempDirGuard::new("i061-missing-parent-results-invalidated-by-suite-root-override");
  let first_suite_root = temp_dir.path().join("suite-root-a");
  let second_suite_root = temp_dir.path().join("suite-root-b");
  let missing_parent_results_under_first =
    first_suite_root.join("missing-parent/results/ltp-results-a.txt");
  let valid_results_under_second = second_suite_root.join("results/ltp-results-b.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-missing-parent-results-invalidated-by-suite-root-override.marker");

  fs::create_dir_all(&first_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create first suite root {}: {error}",
      first_suite_root.display()
    )
  });
  fs::create_dir_all(&second_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create second suite root {}: {error}",
      second_suite_root.display()
    )
  });
  write_text(&valid_results_under_second, "PASS root.b.valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    missing_parent_results_under_first
      .to_string_lossy()
      .into_owned(),
    "--suite-root".to_string(),
    first_suite_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    second_suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results_under_second.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when earlier missing-parent results-file is invalidated by suite-root override"
  );
  assert!(
    !missing_parent_results_under_first.exists(),
    "invalidated missing-parent results-file path must not be created by suite command"
  );
}

#[test]
fn adapter_rejects_missing_parent_results_file_invalidated_by_suite_root_override_before_later_unknown_argument()
 {
  let temp_dir =
    TempDirGuard::new("i061-missing-parent-results-invalidated-by-suite-root-before-unknown");
  let first_suite_root = temp_dir.path().join("suite-root-a");
  let second_suite_root = temp_dir.path().join("suite-root-b");
  let missing_parent_results_under_first =
    first_suite_root.join("missing-parent/results/ltp-results-a.txt");
  let valid_results_under_second = second_suite_root.join("results/ltp-results-b.txt");
  let marker = temp_dir.path().join(
    "suite-command-ran-missing-parent-results-invalidated-by-suite-root-before-unknown.marker",
  );

  fs::create_dir_all(&first_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create first suite root {}: {error}",
      first_suite_root.display()
    )
  });
  fs::create_dir_all(&second_suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create second suite root {}: {error}",
      second_suite_root.display()
    )
  });
  write_text(&valid_results_under_second, "PASS root.b.valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    missing_parent_results_under_first
      .to_string_lossy()
      .into_owned(),
    "--suite-root".to_string(),
    first_suite_root.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    second_suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results_under_second.to_string_lossy().into_owned(),
    "--unknown-after-suite-root-override".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !stderr.contains("unknown argument"),
    "missing-parent invalidation error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when missing-parent results-file is invalidated by suite-root override"
  );
  assert!(
    !missing_parent_results_under_first.exists(),
    "invalidated missing-parent results-file path must not be created by suite command"
  );
}

#[test]
fn adapter_rejects_results_file_with_dotdot_segments_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-dotdot-segments");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir.path().join("suite-command-ran-dotdot.marker");

  fs::create_dir_all(suite_root.join("results")).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory under {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "results/../escaped.txt".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must not contain dot segments"));
  assert!(
    !marker.exists(),
    "suite command must not run when results file path contains dot segments"
  );
}

#[test]
fn adapter_rejects_results_file_dotdot_segments_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-results-dotdot-override");
  let suite_root = temp_dir.path().join("suite-root");
  let results_file = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-dotdot-override.marker");

  write_text(&results_file, "PASS dotdot.override.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "results/../escaped.txt".to_string(),
    "--results-file".to_string(),
    results_file.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must not contain dot segments"));
  assert!(
    !marker.exists(),
    "suite command must not run when any results-file path contains dot segments"
  );
}

#[test]
fn adapter_rejects_results_file_dotdot_segments_even_when_later_value_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new("i061-results-dotdot-override-before-unknown");
  let suite_root = temp_dir.path().join("suite-root");
  let valid_results = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-dotdot-override-before-unknown.marker");

  write_text(&valid_results, "PASS dotdot.override.valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "results/../escaped.txt".to_string(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must not contain dot segments"));
  assert!(
    !stderr.contains("unknown argument"),
    "dot-segment preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any earlier results-file path contains dot segments"
  );
}

#[test]
fn adapter_rejects_absolute_results_file_with_dotdot_segments_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-absolute-dotdot");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-absolute-dotdot.marker");
  let absolute_with_dotdot = suite_root.join("results/../escape.txt");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    absolute_with_dotdot.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must not contain dot segments"));
  assert!(
    !marker.exists(),
    "suite command must not run when absolute results-file path contains dot segments"
  );
}

#[test]
fn adapter_rejects_directory_results_file_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-directory-path");
  let suite_root = temp_dir.path().join("suite-root");
  let results_dir = suite_root.join("results");
  let marker = temp_dir.path().join("suite-command-ran-directory.marker");

  fs::create_dir_all(&results_dir).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory {}: {error}",
      results_dir.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    results_dir.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must reference a regular file"));
  assert!(
    !marker.exists(),
    "suite command must not run when results file path points to a directory"
  );
}

#[test]
fn adapter_rejects_directory_results_file_even_when_later_results_file_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new("i061-results-directory-even-when-later-valid-before-unknown");
  let suite_root = temp_dir.path().join("suite-root");
  let results_dir = suite_root.join("results");
  let valid_results = suite_root.join("results-valid/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-directory-even-when-later-valid-before-unknown.marker");

  fs::create_dir_all(&results_dir).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory {}: {error}",
      results_dir.display()
    )
  });
  write_text(&valid_results, "PASS valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    results_dir.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must reference a regular file"));
  assert!(
    !stderr.contains("unknown argument"),
    "directory-path preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any earlier --results-file points to a directory"
  );
}

#[test]
fn adapter_rejects_directory_results_file_before_suite_root_even_when_later_results_file_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new(
    "i061-results-directory-before-suite-root-even-when-later-valid-before-unknown",
  );
  let suite_root = temp_dir.path().join("suite-root");
  let results_dir = suite_root.join("results");
  let valid_results = suite_root.join("results-valid/ltp-results.txt");
  let marker = temp_dir.path().join(
    "suite-command-ran-directory-before-suite-root-even-when-later-valid-before-unknown.marker",
  );

  fs::create_dir_all(&results_dir).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory {}: {error}",
      results_dir.display()
    )
  });
  write_text(&valid_results, "PASS valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    results_dir.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must reference a regular file"));
  assert!(
    !stderr.contains("unknown argument"),
    "directory-path preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any earlier --results-file points to a directory"
  );
}

#[test]
fn adapter_rejects_results_file_with_trailing_slash_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-trailing-slash");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-trailing-slash.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "results/".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must not end with trailing slash"));
  assert!(
    !marker.exists(),
    "suite command must not run when results file path has trailing slash"
  );
}

#[test]
fn adapter_rejects_results_file_trailing_slash_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-results-trailing-slash-override");
  let suite_root = temp_dir.path().join("suite-root");
  let results_file = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-trailing-slash-override.marker");

  write_text(&results_file, "PASS trailing-slash.override.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "results/".to_string(),
    "--results-file".to_string(),
    results_file.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must not end with trailing slash"));
  assert!(
    !marker.exists(),
    "suite command must not run when any results-file path has trailing slash"
  );
}

#[test]
fn adapter_rejects_results_file_trailing_slash_even_when_later_value_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new("i061-results-trailing-slash-override-before-unknown-argument");
  let suite_root = temp_dir.path().join("suite-root");
  let valid_results = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-trailing-slash-override-before-unknown-argument.marker");

  write_text(&valid_results, "PASS trailing-slash.override.valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "results/".to_string(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must not end with trailing slash"));
  assert!(
    !stderr.contains("unknown argument"),
    "trailing-slash preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any earlier results-file path has trailing slash"
  );
}

#[test]
fn adapter_rejects_results_file_trailing_slash_before_suite_root_even_when_later_value_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new(
    "i061-results-trailing-slash-before-suite-root-even-when-later-valid-before-unknown",
  );
  let suite_root = temp_dir.path().join("suite-root");
  let valid_results = suite_root.join("results/ltp-results.txt");
  let marker = temp_dir.path().join(
    "suite-command-ran-trailing-slash-before-suite-root-even-when-later-valid-before-unknown.marker",
  );

  write_text(&valid_results, "PASS trailing-slash.override.valid.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    "results/".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must not end with trailing slash"));
  assert!(
    !stderr.contains("unknown argument"),
    "trailing-slash preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any earlier results-file path has trailing slash"
  );
}

#[test]
fn adapter_rejects_symlinked_out_of_root_ancestor_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-symlink-ancestor-preflight");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results_root = temp_dir.path().join("external-results-root");
  let suite_results_link = suite_root.join("results");
  let results_file = suite_results_link.join("nested/ltp-results.txt");
  let marker = temp_dir.path().join("suite-command-ran.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });
  fs::create_dir_all(&external_results_root).unwrap_or_else(|error| {
    panic!(
      "failed to create external results root {}: {error}",
      external_results_root.display()
    )
  });
  symlink(&external_results_root, &suite_results_link).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      suite_results_link.display(),
      external_results_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    results_file.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !marker.exists(),
    "suite command must not run when results path escapes via symlinked ancestor"
  );
}

#[test]
fn adapter_rejects_symlinked_out_of_root_ancestor_before_suite_root_and_before_later_unknown_argument()
 {
  let temp_dir =
    TempDirGuard::new("i061-results-symlink-ancestor-before-suite-root-before-unknown");
  let suite_root = temp_dir.path().join("suite-root");
  let external_results_root = temp_dir.path().join("external-results-root");
  let suite_results_link = suite_root.join("results");
  let results_file = suite_results_link.join("nested/ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-symlink-ancestor-before-suite-root-before-unknown.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });
  fs::create_dir_all(&external_results_root).unwrap_or_else(|error| {
    panic!(
      "failed to create external results root {}: {error}",
      external_results_root.display()
    )
  });
  symlink(&external_results_root, &suite_results_link).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      suite_results_link.display(),
      external_results_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    results_file.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must be inside suite root"));
  assert!(
    !stderr.contains("unknown argument"),
    "symlink-escape preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when results path escapes via symlinked ancestor"
  );
}

#[test]
fn adapter_rejects_symlinked_results_file_before_suite_root_and_before_later_unknown_argument() {
  let temp_dir = TempDirGuard::new("i061-results-symlink-file-before-suite-root-before-unknown");
  let suite_root = temp_dir.path().join("suite-root");
  let suite_results = suite_root.join("results");
  let external_results = temp_dir.path().join("external-results/ltp-results.txt");
  let symlink_results = suite_results.join("ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-symlink-file-before-suite-root-before-unknown.marker");

  fs::create_dir_all(&suite_results).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory {}: {error}",
      suite_results.display()
    )
  });
  write_text(&external_results, "PASS symlink.case\n");
  symlink(&external_results, &symlink_results).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      symlink_results.display(),
      external_results.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    symlink_results.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must not be a symlink"));
  assert!(
    !stderr.contains("unknown argument"),
    "results-file symlink preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when results-file is a symlink before suite-root parse completes"
  );
}

#[test]
fn adapter_rejects_symlinked_results_file_even_when_later_results_file_is_valid() {
  let temp_dir = TempDirGuard::new("i061-results-symlink-file-even-when-later-valid");
  let suite_root = temp_dir.path().join("suite-root");
  let suite_results = suite_root.join("results");
  let symlink_results = suite_results.join("ltp-results.txt");
  let external_results = temp_dir.path().join("external-results.txt");
  let valid_results = suite_results.join("valid-ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-symlink-file-even-when-later-valid.marker");

  fs::create_dir_all(&suite_results).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory {}: {error}",
      suite_results.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");
  write_text(&valid_results, "PASS valid.case\n");
  symlink(&external_results, &symlink_results).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      symlink_results.display(),
      external_results.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    symlink_results.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must not be a symlink"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --results-file value is a symlink"
  );
}

#[test]
fn adapter_rejects_symlinked_results_file_even_when_later_results_file_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir =
    TempDirGuard::new("i061-results-symlink-file-even-when-later-valid-before-unknown");
  let suite_root = temp_dir.path().join("suite-root");
  let suite_results = suite_root.join("results");
  let symlink_results = suite_results.join("ltp-results.txt");
  let external_results = temp_dir.path().join("external-results.txt");
  let valid_results = suite_results.join("valid-ltp-results.txt");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-symlink-file-even-when-later-valid-before-unknown.marker");

  fs::create_dir_all(&suite_results).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory {}: {error}",
      suite_results.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");
  write_text(&valid_results, "PASS valid.case\n");
  symlink(&external_results, &symlink_results).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      symlink_results.display(),
      external_results.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    symlink_results.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must not be a symlink"));
  assert!(
    !stderr.contains("unknown argument"),
    "results-file symlink validation should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any --results-file value is a symlink"
  );
}

#[test]
fn adapter_rejects_symlinked_results_file_before_suite_root_even_when_later_results_file_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new(
    "i061-results-symlink-file-before-suite-root-even-when-later-valid-before-unknown",
  );
  let suite_root = temp_dir.path().join("suite-root");
  let suite_results = suite_root.join("results");
  let symlink_results = suite_results.join("ltp-results.txt");
  let external_results = temp_dir.path().join("external-results.txt");
  let valid_results = suite_results.join("valid-ltp-results.txt");
  let marker = temp_dir.path().join(
    "suite-command-ran-symlink-file-before-suite-root-even-when-later-valid-before-unknown.marker",
  );

  fs::create_dir_all(&suite_results).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory {}: {error}",
      suite_results.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");
  write_text(&valid_results, "PASS valid.case\n");
  symlink(&external_results, &symlink_results).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      symlink_results.display(),
      external_results.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--results-file".to_string(),
    symlink_results.to_string_lossy().into_owned(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    valid_results.to_string_lossy().into_owned(),
    "--unknown-after-invalid-results-file".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must not be a symlink"));
  assert!(
    !stderr.contains("unknown argument"),
    "results-file symlink validation should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any --results-file value is a symlink"
  );
}

#[test]
fn adapter_rejects_directory_results_path_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-results-directory-path-preflight");
  let suite_root = temp_dir.path().join("suite-root");
  let marker = temp_dir.path().join("suite-command-ran-directory.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file path must reference a regular file"));
  assert!(
    !marker.exists(),
    "suite command must not run when results file path points to a directory"
  );
}

#[test]
fn adapter_resolves_relative_results_file_against_suite_root() {
  let temp_dir = TempDirGuard::new("i061-relative-results");
  let suite_root = temp_dir.path().join("suite-root");
  let results_file = suite_root.join("results/relative-results.txt");

  write_text(&results_file, "PASS relative.case\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--results-file".to_string(),
    "results/relative-results.txt".to_string(),
  ];
  let output = run_adapter(&arguments);
  let stdout = stdout_text(&output);
  let expected_results = fs::canonicalize(&results_file)
    .unwrap_or_else(|error| panic!("failed to canonicalize {}: {error}", results_file.display()));
  let expected_results = expected_results.to_string_lossy();

  assert!(
    output.status.success(),
    "relative results file should parse"
  );
  assert!(stdout.contains("pass=1"));
  assert!(stdout.contains("fail=0"));
  assert!(stdout.contains("error=0"));
  assert!(stdout.contains(&format!("results_file={expected_results}")));
}

#[test]
fn adapter_rejects_symlinked_results_file_pointing_outside_suite_root() {
  let temp_dir = TempDirGuard::new("i061-results-symlink-outside");
  let suite_root = temp_dir.path().join("suite-root");
  let suite_results = suite_root.join("results");
  let symlink_results = suite_results.join("ltp-results.txt");
  let external_results = temp_dir.path().join("external-results.txt");

  fs::create_dir_all(&suite_results).unwrap_or_else(|error| {
    panic!(
      "failed to create suite results directory {}: {error}",
      suite_results.display()
    )
  });
  write_text(&external_results, "PASS external.case\n");
  symlink(&external_results, &symlink_results).unwrap_or_else(|error| {
    panic!(
      "failed to create symlink {} -> {}: {error}",
      symlink_results.display(),
      external_results.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("results file must not be a symlink"));
}

#[test]
fn adapter_normalizes_ltp_statuses_and_reports_failed_case_ids() {
  let temp_dir = TempDirGuard::new("i061-ltp-normalize");
  let suite_root = temp_dir.path().join("ltp");
  let results_file = suite_root.join("results/ltp-results.txt");

  write_text(
    &results_file,
    "TPASS case.pass\nTFAIL case.fail\nTSKIP case.skip\nXFAIL case.xfail\nXPASS case.xpass\nTTIME case.timeout\n",
  );

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stdout = stdout_text(&output);

  assert!(
    !output.status.success(),
    "hard failures must return non-zero"
  );
  assert!(stdout.contains("suite=ltp"));
  assert!(stdout.contains("pass=1"));
  assert!(stdout.contains("fail=1"));
  assert!(stdout.contains("skip=1"));
  assert!(stdout.contains("xfail=1"));
  assert!(stdout.contains("xpass=1"));
  assert!(stdout.contains("timeout=1"));
  assert!(stdout.contains("failed_cases=case.fail,case.xpass,case.timeout"));
}

#[test]
fn adapter_reports_unknown_status_as_explicit_error() {
  let temp_dir = TempDirGuard::new("i061-unknown-status");
  let suite_root = temp_dir.path().join("ltp");
  let results_file = suite_root.join("results/ltp-results.txt");

  write_text(&results_file, "MYSTERY case.unknown\n");

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);
  let stdout = stdout_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("unknown status 'MYSTERY'"));
  assert!(stdout.contains("error=1"));
  assert!(stdout.contains("failed_cases=case.unknown"));
}

#[test]
fn adapter_enforces_timeout_for_suite_command() {
  let temp_dir = TempDirGuard::new("i061-timeout");
  let suite_root = temp_dir.path().join("ltp");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "1".to_string(),
    "--".to_string(),
    "bash".to_string(),
    "-lc".to_string(),
    "sleep 2".to_string(),
  ];
  let output = run_adapter(&arguments);
  let stdout = stdout_text(&output);

  assert!(!output.status.success());
  assert!(stdout.contains("timeout=1"));
  assert!(stdout.contains("failed_cases=__adapter_timeout__:1"));
}

#[test]
fn adapter_timeout_summary_reflects_configured_timeout_seconds() {
  let temp_dir = TempDirGuard::new("i061-timeout-summary-seconds");
  let suite_root = temp_dir.path().join("ltp");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "2".to_string(),
    "--".to_string(),
    "bash".to_string(),
    "-lc".to_string(),
    "sleep 3".to_string(),
  ];
  let output = run_adapter(&arguments);
  let stdout = stdout_text(&output);

  assert!(!output.status.success());
  assert!(stdout.contains("timeout=1"));
  assert!(stdout.contains("failed_cases=__adapter_timeout__:2"));
}

#[test]
fn adapter_rejects_non_positive_timeout_seconds() {
  let temp_dir = TempDirGuard::new("i061-timeout-validate");
  let suite_root = temp_dir.path().join("ltp");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "0".to_string(),
    "--".to_string(),
    "bash".to_string(),
    "-lc".to_string(),
    "sleep 1".to_string(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds must be a positive integer"));
}

#[test]
fn adapter_rejects_non_positive_timeout_seconds_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-timeout-non-positive-override");
  let suite_root = temp_dir.path().join("ltp");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-timeout-non-positive-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "0".to_string(),
    "--timeout-seconds".to_string(),
    "3".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds must be a positive integer"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --timeout-seconds value is non-positive"
  );
}

#[test]
fn adapter_rejects_negative_timeout_seconds_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-timeout-negative");
  let suite_root = temp_dir.path().join("ltp");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-timeout-negative.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "-1".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds must be a positive integer"));
  assert!(
    !marker.exists(),
    "suite command must not run when --timeout-seconds is negative"
  );
}

#[test]
fn adapter_rejects_negative_timeout_seconds_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-timeout-negative-override");
  let suite_root = temp_dir.path().join("ltp");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-timeout-negative-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "-1".to_string(),
    "--timeout-seconds".to_string(),
    "3".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds must be a positive integer"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --timeout-seconds value is negative"
  );
}

#[test]
fn adapter_rejects_empty_timeout_seconds_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-timeout-empty");
  let suite_root = temp_dir.path().join("ltp");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-timeout-empty.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    String::new(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds must not be empty"));
  assert!(
    !marker.exists(),
    "suite command must not run when --timeout-seconds is empty"
  );
}

#[test]
fn adapter_rejects_empty_timeout_seconds_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-timeout-empty-override");
  let suite_root = temp_dir.path().join("ltp");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-timeout-empty-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    String::new(),
    "--timeout-seconds".to_string(),
    "3".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds must not be empty"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --timeout-seconds value is empty"
  );
}

#[test]
fn adapter_rejects_timeout_seconds_separator_token_before_running_suite_command() {
  let temp_dir = TempDirGuard::new("i061-timeout-separator-token");
  let suite_root = temp_dir.path().join("ltp");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-timeout-separator.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "--".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds requires a value"));
  assert!(
    !marker.exists(),
    "suite command must not run when --timeout-seconds value is missing"
  );
}

#[test]
fn adapter_rejects_timeout_seconds_separator_token_even_when_later_value_is_valid() {
  let temp_dir = TempDirGuard::new("i061-timeout-separator-override");
  let suite_root = temp_dir.path().join("ltp");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-timeout-separator-override.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "--".to_string(),
    "--timeout-seconds".to_string(),
    "3".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds requires a value"));
  assert!(
    !marker.exists(),
    "suite command must not run when any --timeout-seconds value is missing"
  );
}

#[test]
fn adapter_rejects_timeout_seconds_separator_token_even_when_later_value_is_valid_and_before_later_unknown_argument()
 {
  let temp_dir = TempDirGuard::new("i061-timeout-separator-override-before-unknown");
  let suite_root = temp_dir.path().join("ltp");
  let marker = temp_dir
    .path()
    .join("suite-command-ran-timeout-separator-override-before-unknown.marker");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "--".to_string(),
    "--timeout-seconds".to_string(),
    "3".to_string(),
    "--unknown-after-invalid-timeout".to_string(),
    "--".to_string(),
    "touch".to_string(),
    marker.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("--timeout-seconds requires a value"));
  assert!(
    !stderr.contains("unknown argument"),
    "timeout-seconds preflight error should win over later unknown argument"
  );
  assert!(
    !marker.exists(),
    "suite command must not run when any earlier --timeout-seconds value is missing"
  );
}

#[test]
fn adapter_reports_non_timeout_command_failure_with_summary() {
  let temp_dir = TempDirGuard::new("i061-command-fail");
  let suite_root = temp_dir.path().join("ltp");

  fs::create_dir_all(&suite_root).unwrap_or_else(|error| {
    panic!(
      "failed to create suite root {}: {error}",
      suite_root.display()
    )
  });

  let arguments = vec![
    "--suite".to_string(),
    "ltp".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
    "--timeout-seconds".to_string(),
    "3".to_string(),
    "--".to_string(),
    "bash".to_string(),
    "-lc".to_string(),
    "exit 3".to_string(),
  ];
  let output = run_adapter(&arguments);
  let stderr = stderr_text(&output);
  let stdout = stdout_text(&output);

  assert!(!output.status.success());
  assert!(stderr.contains("suite command failed with exit status 3"));
  assert!(stdout.contains("error=1"));
  assert!(stdout.contains("failed_cases=__adapter_command_failed__:3"));
}

#[test]
fn adapter_supports_open_posix_default_results_path() {
  let temp_dir = TempDirGuard::new("i061-open-posix");
  let suite_root = temp_dir.path().join("open-posix");
  let results_file = suite_root.join("results/open-posix-results.txt");

  write_text(&results_file, "PASS open.case.001\nSKIP open.case.002\n");

  let arguments = vec![
    "--suite".to_string(),
    "open_posix_testsuite".to_string(),
    "--suite-root".to_string(),
    suite_root.to_string_lossy().into_owned(),
  ];
  let output = run_adapter(&arguments);
  let stdout = stdout_text(&output);

  assert!(output.status.success(), "pass/skip only should succeed");
  assert!(stdout.contains("suite=open_posix_testsuite"));
  assert!(stdout.contains("pass=1"));
  assert!(stdout.contains("skip=1"));
  assert!(stdout.contains("fail=0"));
  assert!(stdout.contains("timeout=0"));
  assert!(stdout.contains("failed_cases="));
}

#[test]
fn smoke_manifests_exist_for_both_suites() {
  let ltp_manifest = repository_root().join("docs/conformance/ltp-smoke.txt");
  let open_posix_manifest = repository_root().join("docs/conformance/open-posix-smoke.txt");

  assert!(
    ltp_manifest.exists(),
    "LTP smoke manifest must exist at {}",
    ltp_manifest.display(),
  );
  assert!(
    open_posix_manifest.exists(),
    "open_posix smoke manifest must exist at {}",
    open_posix_manifest.display(),
  );
}
