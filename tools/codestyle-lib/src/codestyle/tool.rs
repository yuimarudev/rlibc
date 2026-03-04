use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;
use std::process::Command;

const RUSTFMT_CHUNK_SIZE: usize = 128;

/// Runs `rustfmt` for the given file set.
///
/// # Errors
///
/// Returns an error when invoking `rustfmt` fails or `rustfmt` exits with a
/// non-zero status.
pub fn run_rustfmt(files: &[PathBuf], check: bool, verbose: bool) -> io::Result<()> {
  if files.is_empty() {
    return Ok(());
  }

  for chunk in files.chunks(RUSTFMT_CHUNK_SIZE) {
    let mut command = Command::new("rustfmt");

    command.arg("--edition").arg("2024");

    if check {
      command.arg("--check");
    }

    for path in chunk {
      command.arg(path);
    }

    run_command(command, verbose, "rustfmt")?;
  }

  Ok(())
}

/// Runs `cargo clippy --release --workspace --all-targets`.
///
/// # Errors
///
/// Returns an error when invoking `cargo` fails or clippy exits with a
/// non-zero status.
pub fn run_clippy_release(verbose: bool) -> io::Result<()> {
  let mut command = Command::new("cargo");

  command.args(["clippy", "--release", "--workspace", "--all-targets"]);

  run_command(command, verbose, "cargo clippy --release --workspace")
}

fn run_command(mut command: Command, verbose: bool, label: &str) -> io::Result<()> {
  let rendered = render_command(&command);
  let output = command.output()?;

  if verbose {
    eprintln!("$ {rendered}");
  }

  if output.status.success() {
    return Ok(());
  }

  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  Err(io::Error::other(format!(
    "{label} failed (status: {:?})\nstdout:\n{}\nstderr:\n{}",
    output.status.code(),
    stdout,
    stderr,
  )))
}

fn render_command(command: &Command) -> String {
  let program = command.get_program().to_string_lossy();
  let mut parts = vec![program.into_owned()];
  let args = command.get_args().map(render_arg).collect::<Vec<_>>();

  parts.extend(args);

  parts.join(" ")
}

fn render_arg(arg: &OsStr) -> String {
  let text = arg.to_string_lossy();

  if text.contains(char::is_whitespace) {
    format!("\"{text}\"")
  } else {
    text.into_owned()
  }
}
