use std::env;
use std::path::PathBuf;

fn main() {
  println!("cargo:rerun-if-changed=rlibc.map");
  println!("cargo:rerun-if-changed=src/stdio/variadic_wrappers.c");

  let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
  let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

  if target_os != "linux" || target_env != "gnu" {
    return;
  }

  let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is unset"));

  cc::Build::new()
    .cargo_metadata(false)
    .file("src/stdio/variadic_wrappers.c")
    .compile("rlibc_stdio_variadic_wrappers");
  println!("cargo:rustc-link-search=native={}", out_dir.display());
  println!("cargo:rustc-link-lib=static:+whole-archive=rlibc_stdio_variadic_wrappers");

  let manifest_dir =
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is unset"));
  let map_path = manifest_dir.join("rlibc.map");

  println!(
    "cargo:rustc-cdylib-link-arg=-Wl,--version-script={}",
    map_path.display(),
  );
}
