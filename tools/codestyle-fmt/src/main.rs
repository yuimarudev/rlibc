use codestyle_lib::codestyle::cli::{FORMAT_HELP_TEXT, ParseOutcome, parse_format_args};
use codestyle_lib::codestyle::files::resolve_rust_files;
use codestyle_lib::codestyle::layout::{check_layout_files, rewrite_layout_files};
use codestyle_lib::codestyle::tool::run_rustfmt;
use std::process::ExitCode;

fn main() -> ExitCode {
  match run() {
    Ok(()) => ExitCode::SUCCESS,
    Err(error) => {
      eprintln!("{error}");
      ExitCode::FAILURE
    }
  }
}

fn run() -> Result<(), String> {
  let parsed = parse_format_args()?;
  let ParseOutcome::Config(config) = parsed else {
    print_help();

    return Ok(());
  };
  let files = resolve_rust_files(&config.common.selection)
    .map_err(|error| format!("failed to collect rust files: {error}"))?;

  if files.is_empty() {
    if config.common.verbose {
      eprintln!("no Rust files selected");
    }

    return Ok(());
  }

  if config.check {
    return run_check_mode(&config, &files);
  }

  run_format_mode(&config, &files)
}

fn run_check_mode(
  config: &codestyle_lib::codestyle::cli::FormatArgs,
  files: &[std::path::PathBuf],
) -> Result<(), String> {
  if config.common.run_rustfmt {
    run_rustfmt(files, true, config.common.verbose)
      .map_err(|error| format!("rustfmt check failed: {error}"))?;
  }

  let violations = check_layout_files(files)?;

  if violations > 0 {
    return Err(format!("found {violations} codestyle violation(s)"));
  }

  Ok(())
}

fn run_format_mode(
  config: &codestyle_lib::codestyle::cli::FormatArgs,
  files: &[std::path::PathBuf],
) -> Result<(), String> {
  if config.common.run_rustfmt {
    run_rustfmt(files, false, config.common.verbose)
      .map_err(|error| format!("rustfmt failed: {error}"))?;
  }

  let changed_count = rewrite_layout_files(files)?;

  if config.common.run_rustfmt {
    run_rustfmt(files, false, config.common.verbose)
      .map_err(|error| format!("rustfmt post-format failed: {error}"))?;
  }

  if config.common.verbose {
    eprintln!("updated {changed_count} file(s)");
  }

  Ok(())
}

fn print_help() {
  println!("{FORMAT_HELP_TEXT}");
}
