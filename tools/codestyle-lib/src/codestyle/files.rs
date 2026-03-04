use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileSelection {
  All,
  Staged,
  Paths(Vec<PathBuf>),
}

/// Returns Rust source files selected by `selection`.
///
/// # Errors
///
/// Returns an error when file discovery fails, git command execution fails
/// for staged mode, or one of the requested paths cannot be read.
pub fn resolve_rust_files(selection: &FileSelection) -> io::Result<Vec<PathBuf>> {
  let workspace_root = std::env::current_dir()?;
  let mut paths = BTreeSet::new();

  match selection {
    FileSelection::All => {
      collect_all_rust_files(&workspace_root, &mut paths)?;
    }
    FileSelection::Staged => {
      collect_staged_rust_files(&workspace_root, &mut paths)?;
    }
    FileSelection::Paths(values) => {
      collect_from_paths(&workspace_root, values, &mut paths)?;
    }
  }

  Ok(paths.into_iter().collect())
}

fn collect_all_rust_files(workspace_root: &Path, sink: &mut BTreeSet<PathBuf>) -> io::Result<()> {
  let git_paths = git_paths(
    workspace_root,
    &[
      "ls-files",
      "--cached",
      "--others",
      "--exclude-standard",
      "--",
      "*.rs",
    ],
  );

  match git_paths {
    Ok(paths) => {
      for path in paths {
        if workspace_root.join(&path).exists() {
          insert_if_target(path, sink);
        }
      }

      Ok(())
    }
    Err(_error) => collect_rust_files_in_tree(workspace_root, workspace_root, sink),
  }
}

fn collect_staged_rust_files(
  workspace_root: &Path,
  sink: &mut BTreeSet<PathBuf>,
) -> io::Result<()> {
  let paths = git_paths(
    workspace_root,
    &[
      "diff",
      "--cached",
      "--name-only",
      "--diff-filter=ACMR",
      "--",
      "*.rs",
    ],
  )?;

  for path in paths {
    if workspace_root.join(&path).exists() {
      insert_if_target(path, sink);
    }
  }

  Ok(())
}

fn collect_from_paths(
  workspace_root: &Path,
  values: &[PathBuf],
  sink: &mut BTreeSet<PathBuf>,
) -> io::Result<()> {
  for value in values {
    let absolute_path = if value.is_absolute() {
      value.clone()
    } else {
      workspace_root.join(value)
    };
    let metadata = fs::metadata(&absolute_path).map_err(|error| {
      io::Error::new(
        error.kind(),
        format!("failed to read metadata for {}: {error}", value.display()),
      )
    })?;

    if metadata.is_file() {
      let relative_path = to_workspace_relative(workspace_root, &absolute_path);

      insert_if_target(relative_path, sink);
      continue;
    }

    if metadata.is_dir() {
      collect_rust_files_in_tree(workspace_root, &absolute_path, sink)?;
      continue;
    }

    return Err(io::Error::other(format!(
      "path is neither file nor directory: {}",
      value.display()
    )));
  }

  Ok(())
}

fn collect_rust_files_in_tree(
  workspace_root: &Path,
  dir: &Path,
  sink: &mut BTreeSet<PathBuf>,
) -> io::Result<()> {
  let mut stack = vec![dir.to_path_buf()];

  while let Some(current_dir) = stack.pop() {
    for entry in fs::read_dir(&current_dir)? {
      let entry = entry?;
      let path = entry.path();
      let relative_path = to_workspace_relative(workspace_root, &path);
      let file_type = entry.file_type()?;

      if file_type.is_dir() {
        if should_skip_dir(relative_path.as_path()) {
          continue;
        }

        stack.push(path);
        continue;
      }

      if file_type.is_file() {
        insert_if_target(relative_path, sink);
      }
    }
  }

  Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
  path.starts_with("codex")
    || path.starts_with(".git")
    || path.starts_with("target")
    || path.file_name() == Some(OsStr::new(".idea"))
    || path.file_name() == Some(OsStr::new(".vscode"))
}

fn git_paths(workspace_root: &Path, args: &[&str]) -> io::Result<Vec<PathBuf>> {
  let output = Command::new("git")
    .args(args)
    .current_dir(workspace_root)
    .output()?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);

    return Err(io::Error::other(format!(
      "git command failed ({}): {stderr}",
      args.join(" ")
    )));
  }

  let stdout = String::from_utf8_lossy(&output.stdout);
  let paths = stdout
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(PathBuf::from)
    .collect::<Vec<_>>();

  Ok(paths)
}

fn insert_if_target(path: PathBuf, sink: &mut BTreeSet<PathBuf>) {
  if !is_rust_file(path.as_path()) {
    return;
  }

  if path.starts_with("codex") {
    return;
  }

  sink.insert(path);
}

fn is_rust_file(path: &Path) -> bool {
  path.extension() == Some(OsStr::new("rs"))
}

fn to_workspace_relative(workspace_root: &Path, path: &Path) -> PathBuf {
  path
    .strip_prefix(workspace_root)
    .map_or_else(|_error| path.to_path_buf(), Path::to_path_buf)
}
