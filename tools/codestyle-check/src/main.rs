use codestyle_lib::codestyle::cli::{CHECK_HELP_TEXT, ParseOutcome, parse_check_args};
use codestyle_lib::codestyle::files::resolve_rust_files;
use codestyle_lib::codestyle::layout::check_layout_files;
use codestyle_lib::codestyle::tool::{run_clippy_release, run_rustfmt};
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
  let parsed = parse_check_args()?;
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

  if config.common.run_rustfmt {
    run_rustfmt(&files, true, config.common.verbose)
      .map_err(|error| format!("rustfmt check failed: {error}"))?;
  }

  let violation_count = check_layout_files(&files)?;

  if violation_count > 0 {
    return Err(format!("found {violation_count} codestyle violation(s)"));
  }

  if config.run_clippy {
    run_clippy_release(config.common.verbose).map_err(|error| format!("clippy failed: {error}"))?;
  }

  Ok(())
}

fn print_help() {
  println!("{CHECK_HELP_TEXT}");
}
