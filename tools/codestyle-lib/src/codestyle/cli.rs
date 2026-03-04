use super::files::FileSelection;
use std::env;
use std::path::PathBuf;

pub const CHECK_HELP_TEXT: &str =
  "codestyle_check [--all | --staged | --path <PATH> ...] [--no-rustfmt] [--no-clippy] [--verbose]";
pub const FORMAT_HELP_TEXT: &str =
  "codestyle_fmt [--all | --staged | --path <PATH> ...] [--check] [--no-rustfmt] [--verbose]";

#[derive(Clone, Debug)]
pub struct CommonArgs {
  pub selection: FileSelection,
  pub run_rustfmt: bool,
  pub verbose: bool,
}

#[derive(Clone, Debug)]
pub struct CheckArgs {
  pub common: CommonArgs,
  pub run_clippy: bool,
}

#[derive(Clone, Debug)]
pub struct FormatArgs {
  pub common: CommonArgs,
  pub check: bool,
}

pub enum ParseOutcome<T> {
  Config(T),
  Help,
}

#[derive(Debug)]
enum SelectionBuilder {
  Unset,
  All,
  Staged,
  Paths(Vec<PathBuf>),
}

#[derive(Debug)]
struct ParseState {
  selection: SelectionBuilder,
  run_rustfmt: bool,
  verbose: bool,
}

/// Parses CLI arguments for `codestyle_check`.
///
/// # Errors
///
/// Returns an error when an unsupported flag is supplied or required values are
/// missing.
pub fn parse_check_args() -> Result<ParseOutcome<CheckArgs>, String> {
  let mut state = ParseState {
    selection: SelectionBuilder::Unset,
    run_rustfmt: true,
    verbose: false,
  };
  let mut run_clippy = true;
  let mut args = env::args().skip(1);

  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--help" | "-h" => return Ok(ParseOutcome::Help),
      "--all" | "--staged" | "--path" => {
        apply_selection_argument(arg.as_str(), &mut args, &mut state.selection)?;
      }
      "--no-rustfmt" => {
        state.run_rustfmt = false;
      }
      "--no-clippy" => {
        run_clippy = false;
      }
      "--verbose" => {
        state.verbose = true;
      }
      _ => {
        return Err(format!("unknown argument: {arg}"));
      }
    }
  }

  let selection = finalize_selection(state.selection)?;

  Ok(ParseOutcome::Config(CheckArgs {
    common: CommonArgs {
      selection,
      run_rustfmt: state.run_rustfmt,
      verbose: state.verbose,
    },
    run_clippy,
  }))
}

/// Parses CLI arguments for `codestyle_fmt`.
///
/// # Errors
///
/// Returns an error when an unsupported flag is supplied or required values are
/// missing.
pub fn parse_format_args() -> Result<ParseOutcome<FormatArgs>, String> {
  let mut state = ParseState {
    selection: SelectionBuilder::Unset,
    run_rustfmt: true,
    verbose: false,
  };
  let mut check = false;
  let mut args = env::args().skip(1);

  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--help" | "-h" => return Ok(ParseOutcome::Help),
      "--all" | "--staged" | "--path" => {
        apply_selection_argument(arg.as_str(), &mut args, &mut state.selection)?;
      }
      "--check" => {
        check = true;
      }
      "--no-rustfmt" => {
        state.run_rustfmt = false;
      }
      "--verbose" => {
        state.verbose = true;
      }
      _ => {
        return Err(format!("unknown argument: {arg}"));
      }
    }
  }

  let selection = finalize_selection(state.selection)?;

  Ok(ParseOutcome::Config(FormatArgs {
    common: CommonArgs {
      selection,
      run_rustfmt: state.run_rustfmt,
      verbose: state.verbose,
    },
    check,
  }))
}

fn apply_selection_argument(
  arg: &str,
  args: &mut impl Iterator<Item = String>,
  selection: &mut SelectionBuilder,
) -> Result<(), String> {
  match arg {
    "--all" => {
      ensure_selection_switchable(selection, "--all")?;
      *selection = SelectionBuilder::All;
      Ok(())
    }
    "--staged" => {
      ensure_selection_switchable(selection, "--staged")?;
      *selection = SelectionBuilder::Staged;
      Ok(())
    }
    "--path" => {
      let Some(value) = args.next() else {
        return Err("--path requires one argument".to_owned());
      };

      match selection {
        SelectionBuilder::Unset => {
          *selection = SelectionBuilder::Paths(vec![PathBuf::from(value)]);
        }
        SelectionBuilder::Paths(paths) => {
          paths.push(PathBuf::from(value));
        }
        SelectionBuilder::All | SelectionBuilder::Staged => {
          return Err("--path cannot be combined with --all or --staged".to_owned());
        }
      }

      Ok(())
    }
    _ => Err(format!("unknown selection argument: {arg}")),
  }
}

fn finalize_selection(selection: SelectionBuilder) -> Result<FileSelection, String> {
  match selection {
    SelectionBuilder::Unset | SelectionBuilder::All => Ok(FileSelection::All),
    SelectionBuilder::Staged => Ok(FileSelection::Staged),
    SelectionBuilder::Paths(paths) => {
      if paths.is_empty() {
        Err("--path requires at least one value".to_owned())
      } else {
        Ok(FileSelection::Paths(paths))
      }
    }
  }
}

fn ensure_selection_switchable(selection: &SelectionBuilder, incoming: &str) -> Result<(), String> {
  match selection {
    SelectionBuilder::Unset => Ok(()),
    SelectionBuilder::All if incoming == "--all" => Ok(()),
    SelectionBuilder::Staged if incoming == "--staged" => Ok(()),
    SelectionBuilder::Paths(_) => Err(format!("{incoming} cannot be combined with --path")),
    SelectionBuilder::All | SelectionBuilder::Staged => Err(format!(
      "selection argument {incoming} conflicts with previous selection"
    )),
  }
}
