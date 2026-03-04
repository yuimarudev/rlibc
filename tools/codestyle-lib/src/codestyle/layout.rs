use super::analysis::StyleViolation;
use super::block::analyze_block_layout;
use super::top_level::analyze_top_level_layout;
use std::fs;
use std::path::{Path, PathBuf};

/// Runs all codestyle analyzers and returns collected violations.
///
/// # Errors
///
/// Returns an error when the source cannot be parsed by one of the analyzers.
pub fn collect_layout_violations(source: &str) -> Result<Vec<StyleViolation>, syn::Error> {
  let top_level = analyze_top_level_layout(source)?;
  let block = analyze_block_layout(source)?;
  let mut violations = top_level.violations;

  violations.extend(block.violations);

  Ok(violations)
}

/// Applies all codestyle formatters in a deterministic order.
///
/// # Errors
///
/// Returns an error when the source cannot be parsed by one of the analyzers.
pub fn format_layout_source(source: &str) -> Result<String, syn::Error> {
  let top_level = analyze_top_level_layout(source)?;
  let after_top_level = top_level
    .formatted_source
    .unwrap_or_else(|| source.to_owned());
  let block = analyze_block_layout(after_top_level.as_str())?;

  Ok(block.formatted_source.unwrap_or(after_top_level))
}

/// Reads one file, runs codestyle checks, prints violations, and returns their count.
///
/// # Errors
///
/// Returns an error when file I/O fails or layout parsing fails.
pub fn check_layout_file(path: &Path) -> Result<usize, String> {
  let source = fs::read_to_string(path)
    .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
  let violations = collect_layout_violations(&source)
    .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;

  for violation in &violations {
    println!(
      "{}:{}:{}: {} {}",
      path.display(),
      violation.line,
      violation.column,
      violation.code,
      violation.message,
    );
  }

  Ok(violations.len())
}

/// Rewrites one file in-place with codestyle formatting.
///
/// # Errors
///
/// Returns an error when file I/O fails or layout parsing fails.
pub fn rewrite_layout_file(path: &Path) -> Result<bool, String> {
  let original = fs::read_to_string(path)
    .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
  let updated = format_layout_source(&original)
    .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;

  if updated == original {
    return Ok(false);
  }

  fs::write(path, updated)
    .map_err(|error| format!("failed to write {}: {error}", path.display()))?;

  Ok(true)
}

/// Runs `check_layout_file` on all paths and returns total violations.
///
/// # Errors
///
/// Returns an error when checking any single file fails.
pub fn check_layout_files(paths: &[PathBuf]) -> Result<usize, String> {
  let mut violations = 0usize;

  for path in paths {
    let count = check_layout_file(path.as_path())?;

    violations = violations.saturating_add(count);
  }

  Ok(violations)
}

/// Runs `rewrite_layout_file` on all paths and returns number of changed files.
///
/// # Errors
///
/// Returns an error when formatting any single file fails.
pub fn rewrite_layout_files(paths: &[PathBuf]) -> Result<usize, String> {
  let mut changed_count = 0usize;

  for path in paths {
    if rewrite_layout_file(path.as_path())? {
      changed_count = changed_count.saturating_add(1);
    }
  }

  Ok(changed_count)
}
