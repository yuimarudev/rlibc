use core::mem::{align_of, size_of};
use rlibc::abi::errno::{EBADF, EFAULT, EINVAL, ENOENT, ENOTDIR};
use rlibc::dirent::{Dirent, closedir, opendir, readdir};
use rlibc::errno::__errno_location;
use rlibc::fs::{
  AT_EMPTY_PATH, AT_FDCWD, AT_SYMLINK_NOFOLLOW, Stat, Timespec, fstat, fstatat, lstat, stat,
};
use std::ffi::{CStr, CString, OsStr};
use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const S_IFMT: u32 = 0o170_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir {
  path: PathBuf,
}

impl TempDir {
  fn new() -> Self {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let now_millis = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time must be after unix epoch")
      .as_millis();
    let path = std::env::temp_dir().join(format!("rlibc_i020_{pid}_{now_millis}_{counter}"));

    fs::create_dir_all(&path).expect("failed to create temporary test directory");

    Self { path }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TempDir {
  fn drop(&mut self) {
    if !self.path.exists() {
      return;
    }

    if let Err(error) = remove_dir_tree(self.path()) {
      eprintln!(
        "failed to clean temporary directory {}: {error}",
        self.path.display()
      );
    }
  }
}

fn dirent_name_bytes(entry: *const Dirent) -> Vec<u8> {
  // SAFETY: `entry` comes from `readdir` and remains valid until the next `readdir` call.
  let entry_ref = unsafe { &*entry };
  // SAFETY: `readdir` zero-terminates `d_name`.
  let name = unsafe { CStr::from_ptr(entry_ref.d_name.as_ptr()) };

  name.to_bytes().to_vec()
}

fn remove_dir_tree(path: &Path) -> io::Result<()> {
  let path_c = path_to_c_string(path);
  // SAFETY: `path_c` is NUL-terminated and lives until this call returns.
  let dir = unsafe { opendir(path_c.as_ptr()) };

  if dir.is_null() {
    return Err(io::Error::other(format!(
      "opendir failed with errno={}",
      read_errno()
    )));
  }

  let mut traversal_result: io::Result<()> = Ok(());

  loop {
    write_errno(0);
    // SAFETY: `dir` is a live directory handle from `opendir`.
    let entry = unsafe { readdir(dir) };

    if entry.is_null() {
      let errno_value = read_errno();

      if errno_value != 0 {
        traversal_result = Err(io::Error::other(format!(
          "readdir failed with errno={errno_value}"
        )));
      }

      break;
    }

    let entry_name = dirent_name_bytes(entry);

    if entry_name == b"." || entry_name == b".." {
      continue;
    }

    let child_path = path.join(OsStr::from_bytes(&entry_name));
    let file_type = fs::symlink_metadata(&child_path)?.file_type();

    if file_type.is_dir() {
      remove_dir_tree(&child_path)?;
      continue;
    }

    fs::remove_file(&child_path)?;
  }

  // SAFETY: `dir` is still a live handle from `opendir`.
  let close_rc = unsafe { closedir(dir) };

  if close_rc != 0 {
    let errno_value = read_errno();

    if traversal_result.is_ok() {
      traversal_result = Err(io::Error::other(format!(
        "closedir failed with errno={errno_value}"
      )));
    }
  }

  traversal_result?;
  fs::remove_dir(path)
}

fn path_to_c_string(path: &Path) -> CString {
  CString::new(path.as_os_str().as_bytes()).expect("path must not contain interior NUL")
}

fn read_errno() -> i32 {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns a valid thread-local pointer for this thread.
  unsafe { errno_ptr.read() }
}

fn write_errno(value: i32) {
  let errno_ptr = __errno_location();

  // SAFETY: `__errno_location` returns a valid thread-local pointer for this thread.
  unsafe {
    errno_ptr.write(value);
  }
}

#[test]
fn stat_layout_matches_linux_x86_64_abi_shape() {
  assert_eq!(size_of::<Timespec>(), 16);
  assert_eq!(align_of::<Timespec>(), 8);
  assert_eq!(size_of::<Stat>(), 144);
  assert_eq!(align_of::<Stat>(), 8);
}

#[test]
fn stat_reports_regular_file_mode_and_size() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("regular.txt");
  let file_path_c = path_to_c_string(&file_path);
  let mut stat_buf = Stat::default();

  fs::write(&file_path, b"hello").expect("failed to write regular file");
  write_errno(0);

  // SAFETY: `file_path_c` and `stat_buf` pointers are valid for this call.
  let rc = unsafe { stat(file_path_c.as_ptr(), &raw mut stat_buf) };

  assert_eq!(rc, 0);
  assert_eq!(read_errno(), 0);
  assert_eq!(stat_buf.st_mode & S_IFMT, S_IFREG);
  assert_eq!(stat_buf.st_size, 5);
}

#[test]
fn lstat_keeps_symlink_type_while_stat_follows_target() {
  let temp_dir = TempDir::new();
  let target_path = temp_dir.path().join("target.txt");
  let link_path = temp_dir.path().join("target.link");
  let link_path_c = path_to_c_string(&link_path);
  let mut stat_buf = Stat::default();
  let mut lstat_buf = Stat::default();

  fs::write(&target_path, b"payload").expect("failed to create symlink target");
  symlink(&target_path, &link_path).expect("failed to create symlink");

  write_errno(0);

  // SAFETY: `link_path_c` and out pointers are valid for the call.
  let stat_rc = unsafe { stat(link_path_c.as_ptr(), &raw mut stat_buf) };
  // SAFETY: `link_path_c` and out pointers are valid for the call.
  let lstat_rc = unsafe { lstat(link_path_c.as_ptr(), &raw mut lstat_buf) };

  assert_eq!(stat_rc, 0);
  assert_eq!(lstat_rc, 0);
  assert_eq!(read_errno(), 0);
  assert_eq!(stat_buf.st_mode & S_IFMT, S_IFREG);
  assert_eq!(lstat_buf.st_mode & S_IFMT, S_IFLNK);
}

#[test]
fn lstat_dangling_symlink_succeeds_while_stat_sets_enoent() {
  let temp_dir = TempDir::new();
  let missing_target_path = temp_dir.path().join("dangling_target.txt");
  let link_path = temp_dir.path().join("dangling_target.link");
  let link_path_c = path_to_c_string(&link_path);
  let mut stat_buf = Stat::default();
  let mut lstat_buf = Stat::default();

  symlink(&missing_target_path, &link_path).expect("failed to create dangling symlink");

  write_errno(EINVAL);

  // SAFETY: pointers are valid and point to an existing dangling symlink path.
  let stat_rc = unsafe { stat(link_path_c.as_ptr(), &raw mut stat_buf) };

  assert_eq!(stat_rc, -1);
  assert_eq!(read_errno(), ENOENT);

  write_errno(EBADF);

  // SAFETY: pointers are valid and point to an existing dangling symlink path.
  let lstat_rc = unsafe { lstat(link_path_c.as_ptr(), &raw mut lstat_buf) };

  assert_eq!(lstat_rc, 0);
  assert_eq!(read_errno(), EBADF);
  assert_eq!(lstat_buf.st_mode & S_IFMT, S_IFLNK);
}

#[test]
fn fstat_matches_stat_for_open_file() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("open.txt");
  let file_path_c = path_to_c_string(&file_path);
  let mut stat_buf = Stat::default();
  let mut fstat_buf = Stat::default();

  fs::write(&file_path, b"abcdef").expect("failed to write open file");

  let file = File::open(&file_path).expect("failed to open test file");

  write_errno(0);

  // SAFETY: arguments are valid for this call.
  let stat_rc = unsafe { stat(file_path_c.as_ptr(), &raw mut stat_buf) };
  // SAFETY: `file` provides a live file descriptor and out pointer is valid.
  let fstat_rc = unsafe { fstat(file.as_raw_fd(), &raw mut fstat_buf) };

  assert_eq!(stat_rc, 0);
  assert_eq!(fstat_rc, 0);
  assert_eq!(read_errno(), 0);
  assert_eq!(fstat_buf.st_dev, stat_buf.st_dev);
  assert_eq!(fstat_buf.st_ino, stat_buf.st_ino);
  assert_eq!(fstat_buf.st_size, stat_buf.st_size);
}

#[test]
fn fstatat_empty_path_with_empty_path_flag_uses_fd_metadata_and_preserves_errno() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("empty_path_flag.txt");
  let empty_path = CString::new("").expect("CString::new failed");
  let mut fstat_buf = Stat::default();
  let mut fstatat_buf = Stat::default();

  fs::write(&file_path, b"payload").expect("failed to create empty-path flag test file");

  let file = File::open(&file_path).expect("failed to open empty-path flag test file");

  write_errno(EINVAL);

  // SAFETY: file descriptor is valid and output pointer is writable.
  let fstat_rc = unsafe { fstat(file.as_raw_fd(), &raw mut fstat_buf) };

  assert_eq!(fstat_rc, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ENOENT);

  // SAFETY: `fd` is valid, empty path is NUL-terminated, output pointer is writable.
  let fstatat_rc = unsafe {
    fstatat(
      file.as_raw_fd(),
      empty_path.as_ptr(),
      &raw mut fstatat_buf,
      AT_EMPTY_PATH,
    )
  };

  assert_eq!(fstatat_rc, 0);
  assert_eq!(read_errno(), ENOENT);
  assert_eq!(fstatat_buf.st_mode & S_IFMT, S_IFREG);
  assert_eq!(fstatat_buf.st_ino, fstat_buf.st_ino);
  assert_eq!(fstatat_buf.st_dev, fstat_buf.st_dev);
}

#[test]
fn fstatat_empty_path_with_at_fdcwd_uses_cwd_metadata_and_preserves_errno() {
  let empty_path = CString::new("").expect("CString::new failed");
  let current_dir_path = CString::new(".").expect("CString::new failed");
  let mut by_empty_path = Stat::default();
  let mut by_stat = Stat::default();

  write_errno(EINVAL);

  // SAFETY: empty path is NUL-terminated and output pointer is writable.
  let by_empty_path_rc = unsafe {
    fstatat(
      AT_FDCWD,
      empty_path.as_ptr(),
      &raw mut by_empty_path,
      AT_EMPTY_PATH,
    )
  };

  assert_eq!(by_empty_path_rc, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ENOENT);

  // SAFETY: current-directory path is NUL-terminated and output pointer is writable.
  let by_stat_rc = unsafe { stat(current_dir_path.as_ptr(), &raw mut by_stat) };

  assert_eq!(by_stat_rc, 0);
  assert_eq!(by_empty_path.st_dev, by_stat.st_dev);
  assert_eq!(by_empty_path.st_ino, by_stat.st_ino);
}

#[test]
fn fstatat_empty_path_with_invalid_fd_and_empty_path_flag_sets_ebadf() {
  let empty_path = CString::new("").expect("CString::new failed");
  let mut stat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: path is valid and fd is intentionally invalid.
  let rc = unsafe { fstatat(-1, empty_path.as_ptr(), &raw mut stat_buf, AT_EMPTY_PATH) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fstatat_empty_path_with_invalid_fd_and_null_output_sets_ebadf() {
  let empty_path = CString::new("").expect("CString::new failed");

  write_errno(EINVAL);

  // SAFETY: path is valid and fd is intentionally invalid; null output probes errno priority.
  let rc = unsafe { fstatat(-1, empty_path.as_ptr(), std::ptr::null_mut(), AT_EMPTY_PATH) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fstatat_empty_path_with_at_fdcwd_and_null_output_sets_efault() {
  let empty_path = CString::new("").expect("CString::new failed");

  write_errno(EINVAL);

  // SAFETY: empty path is valid; null output pointer intentionally probes EFAULT handling.
  let rc = unsafe {
    fstatat(
      AT_FDCWD,
      empty_path.as_ptr(),
      std::ptr::null_mut(),
      AT_EMPTY_PATH,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_empty_path_with_nofollow_and_empty_path_flag_uses_fd_metadata() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("empty_path_nofollow.txt");
  let empty_path = CString::new("").expect("CString::new failed");
  let mut fstat_buf = Stat::default();
  let mut fstatat_buf = Stat::default();

  fs::write(&file_path, b"payload").expect("failed to create empty-path nofollow test file");

  let file = File::open(&file_path).expect("failed to open empty-path nofollow test file");

  write_errno(EINVAL);

  // SAFETY: file descriptor is valid and output pointer is writable.
  let fstat_rc = unsafe { fstat(file.as_raw_fd(), &raw mut fstat_buf) };

  assert_eq!(fstat_rc, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ENOENT);

  // SAFETY: `fd`/path are valid; supported flags are combined intentionally.
  let fstatat_rc = unsafe {
    fstatat(
      file.as_raw_fd(),
      empty_path.as_ptr(),
      &raw mut fstatat_buf,
      AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(fstatat_rc, 0);
  assert_eq!(read_errno(), ENOENT);
  assert_eq!(fstatat_buf.st_mode & S_IFMT, S_IFREG);
  assert_eq!(fstatat_buf.st_ino, fstat_buf.st_ino);
  assert_eq!(fstatat_buf.st_dev, fstat_buf.st_dev);
}

#[test]
fn fstatat_empty_path_with_at_fdcwd_and_nofollow_uses_cwd_metadata() {
  let empty_path = CString::new("").expect("CString::new failed");
  let current_dir_path = CString::new(".").expect("CString::new failed");
  let mut by_empty_path = Stat::default();
  let mut by_stat = Stat::default();

  write_errno(EINVAL);

  // SAFETY: empty path is NUL-terminated and output pointer is writable.
  let by_empty_path_rc = unsafe {
    fstatat(
      AT_FDCWD,
      empty_path.as_ptr(),
      &raw mut by_empty_path,
      AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(by_empty_path_rc, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ENOENT);

  // SAFETY: current-directory path is NUL-terminated and output pointer is writable.
  let by_stat_rc = unsafe { stat(current_dir_path.as_ptr(), &raw mut by_stat) };

  assert_eq!(by_stat_rc, 0);
  assert_eq!(by_empty_path.st_dev, by_stat.st_dev);
  assert_eq!(by_empty_path.st_ino, by_stat.st_ino);
}

#[test]
fn fstatat_resolves_relative_paths_from_dirfd() {
  let temp_dir = TempDir::new();
  let file_name = format!("rel_{}.txt", TEMP_COUNTER.fetch_add(1, Ordering::Relaxed));
  let file_path = temp_dir.path().join(&file_name);
  let file_name_c = CString::new(file_name).expect("filename must not contain interior NUL");
  let mut by_dirfd = Stat::default();
  let mut by_cwd = Stat::default();

  fs::write(&file_path, b"x").expect("failed to write relative-path file");

  let dir = File::open(temp_dir.path()).expect("failed to open temporary directory");

  write_errno(0);

  // SAFETY: `dir` descriptor is live, and pointers are valid.
  let rc_by_dirfd = unsafe { fstatat(dir.as_raw_fd(), file_name_c.as_ptr(), &raw mut by_dirfd, 0) };
  // SAFETY: pointer is valid; AT_FDCWD resolves against process cwd.
  let rc_by_cwd = unsafe { fstatat(AT_FDCWD, file_name_c.as_ptr(), &raw mut by_cwd, 0) };

  assert_eq!(rc_by_dirfd, 0);
  assert_eq!(by_dirfd.st_size, 1);
  assert_eq!(rc_by_cwd, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_relative_path_with_empty_path_flag_matches_zero_flag_behavior() {
  let temp_dir = TempDir::new();
  let file_name = CString::new("relative_with_empty_flag.txt").expect("CString::new failed");
  let file_path = temp_dir.path().join("relative_with_empty_flag.txt");
  let mut by_zero_flag = Stat::default();
  let mut by_empty_path_flag = Stat::default();

  fs::write(&file_path, b"payload")
    .expect("failed to create relative-path AT_EMPTY_PATH behavior test file");

  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");

  write_errno(EINVAL);

  // SAFETY: `dirfd` and pointers are valid for relative path lookup.
  let by_zero_flag_rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      file_name.as_ptr(),
      &raw mut by_zero_flag,
      0,
    )
  };

  assert_eq!(by_zero_flag_rc, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ENOENT);

  // SAFETY: `AT_EMPTY_PATH` is supported; non-empty relative path should still resolve normally.
  let by_empty_path_flag_rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      file_name.as_ptr(),
      &raw mut by_empty_path_flag,
      AT_EMPTY_PATH,
    )
  };

  assert_eq!(by_empty_path_flag_rc, 0);
  assert_eq!(read_errno(), ENOENT);
  assert_eq!(by_empty_path_flag.st_mode & S_IFMT, S_IFREG);
  assert_eq!(by_empty_path_flag.st_dev, by_zero_flag.st_dev);
  assert_eq!(by_empty_path_flag.st_ino, by_zero_flag.st_ino);
}

#[test]
fn fstatat_symlink_nofollow_and_invalid_fd_set_expected_errno() {
  let temp_dir = TempDir::new();
  let target_path = temp_dir.path().join("nofollow_target.txt");
  let link_path = temp_dir.path().join("nofollow.link");
  let link_path_c = path_to_c_string(&link_path);
  let invalid_name = CString::new("does_not_matter").expect("CString::new failed");
  let mut nofollow_buf = Stat::default();
  let mut follow_buf = Stat::default();
  let mut invalid_fd_buf = Stat::default();

  fs::write(&target_path, b"payload").expect("failed to create fstatat target");
  symlink(&target_path, &link_path).expect("failed to create fstatat symlink");

  write_errno(0);

  // SAFETY: valid pointers and flags for syscall boundary.
  let nofollow_rc = unsafe {
    fstatat(
      AT_FDCWD,
      link_path_c.as_ptr(),
      &raw mut nofollow_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };
  // SAFETY: valid pointers and flags for syscall boundary.
  let follow_rc = unsafe { fstatat(AT_FDCWD, link_path_c.as_ptr(), &raw mut follow_buf, 0) };

  assert_eq!(nofollow_rc, 0);
  assert_eq!(follow_rc, 0);
  assert_eq!(nofollow_buf.st_mode & S_IFMT, S_IFLNK);
  assert_eq!(follow_buf.st_mode & S_IFMT, S_IFREG);

  write_errno(0);

  // SAFETY: descriptor is intentionally invalid; pointer arguments are still valid.
  let invalid_fd_rc = unsafe { fstatat(-1, invalid_name.as_ptr(), &raw mut invalid_fd_buf, 0) };

  assert_eq!(invalid_fd_rc, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fstatat_relative_path_with_invalid_fd_and_nofollow_sets_ebadf() {
  let path = CString::new("relative.txt").expect("CString::new failed");
  let mut stat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: pointer arguments are valid; descriptor is intentionally invalid.
  let rc = unsafe { fstatat(-1, path.as_ptr(), &raw mut stat_buf, AT_SYMLINK_NOFOLLOW) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn stat_and_lstat_missing_path_set_enoent() {
  let temp_dir = TempDir::new();
  let missing_path = temp_dir.path().join("does_not_exist");
  let missing_path_c = path_to_c_string(&missing_path);
  let mut stat_buf = Stat::default();
  let mut lstat_buf = Stat::default();

  write_errno(0);

  // SAFETY: pointers are valid for the syscall boundary.
  let stat_rc = unsafe { stat(missing_path_c.as_ptr(), &raw mut stat_buf) };

  assert_eq!(stat_rc, -1);
  assert_eq!(read_errno(), ENOENT);

  write_errno(0);

  // SAFETY: pointers are valid for the syscall boundary.
  let lstat_rc = unsafe { lstat(missing_path_c.as_ptr(), &raw mut lstat_buf) };

  assert_eq!(lstat_rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn stat_family_empty_path_sets_enoent() {
  let empty_path = CString::new("").expect("CString::new failed");
  let mut stat_buf = Stat::default();

  write_errno(0);

  // SAFETY: pointer references a valid NUL-terminated (empty) path string.
  let stat_rc = unsafe { stat(empty_path.as_ptr(), &raw mut stat_buf) };

  assert_eq!(stat_rc, -1);
  assert_eq!(read_errno(), ENOENT);

  write_errno(0);

  // SAFETY: pointer references a valid NUL-terminated (empty) path string.
  let lstat_rc = unsafe { lstat(empty_path.as_ptr(), &raw mut stat_buf) };

  assert_eq!(lstat_rc, -1);
  assert_eq!(read_errno(), ENOENT);

  write_errno(0);

  // SAFETY: pointer references a valid NUL-terminated (empty) path string.
  let fstatat_rc = unsafe { fstatat(AT_FDCWD, empty_path.as_ptr(), &raw mut stat_buf, 0) };

  assert_eq!(fstatat_rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn stat_family_null_pointer_inputs_set_efault() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("null_pointer.txt");

  fs::write(&file_path, b"data").expect("failed to create file for null-pointer test");

  let file = File::open(&file_path).expect("failed to open file for null-pointer test");
  let file_path_c = path_to_c_string(&file_path);
  let mut stat_buf = Stat::default();

  write_errno(0);

  // SAFETY: null output pointer intentionally probes EFAULT handling.
  let stat_null_buf_rc = unsafe { stat(file_path_c.as_ptr(), std::ptr::null_mut()) };

  assert_eq!(stat_null_buf_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);

  // SAFETY: null path pointer intentionally probes EFAULT handling.
  let stat_null_path_rc = unsafe { stat(std::ptr::null(), &raw mut stat_buf) };

  assert_eq!(stat_null_path_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);

  // SAFETY: null output pointer intentionally probes EFAULT handling.
  let fstat_null_buf_rc = unsafe { fstat(file.as_raw_fd(), std::ptr::null_mut()) };

  assert_eq!(fstat_null_buf_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);

  // SAFETY: null path pointer intentionally probes EFAULT handling.
  let fstatat_null_path_rc = unsafe { fstatat(AT_FDCWD, std::ptr::null(), &raw mut stat_buf, 0) };

  assert_eq!(fstatat_null_path_rc, -1);
  assert_eq!(read_errno(), EFAULT);

  write_errno(0);

  // SAFETY: null output pointer intentionally probes EFAULT handling.
  let fstatat_null_buf_rc =
    unsafe { fstatat(AT_FDCWD, file_path_c.as_ptr(), std::ptr::null_mut(), 0) };

  assert_eq!(fstatat_null_buf_rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_null_path_with_nofollow_sets_efault() {
  let mut stat_buf = Stat::default();

  write_errno(EBADF);

  // SAFETY: null path pointer intentionally probes EFAULT handling with non-zero flags.
  let rc = unsafe {
    fstatat(
      AT_FDCWD,
      std::ptr::null(),
      &raw mut stat_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_null_output_with_nofollow_sets_efault() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("null_output_nofollow.txt");
  let file_path_c = path_to_c_string(&file_path);

  fs::write(&file_path, b"data").expect("failed to create file for null output nofollow test");

  write_errno(EINVAL);

  // SAFETY: null output pointer intentionally probes EFAULT handling with non-zero flags.
  let rc = unsafe {
    fstatat(
      AT_FDCWD,
      file_path_c.as_ptr(),
      std::ptr::null_mut(),
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_empty_path_with_empty_path_flag_and_null_output_sets_efault() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("empty_path_flag_null_output.txt");
  let empty_path = CString::new("").expect("CString::new failed");

  fs::write(&file_path, b"data")
    .expect("failed to create empty-path-flag null-output test input file");

  let file = File::open(&file_path).expect("failed to open empty-path-flag null-output test file");

  write_errno(EINVAL);

  // SAFETY: fd/path are valid; null output pointer intentionally probes EFAULT with AT_EMPTY_PATH.
  let rc = unsafe {
    fstatat(
      file.as_raw_fd(),
      empty_path.as_ptr(),
      std::ptr::null_mut(),
      AT_EMPTY_PATH,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_null_path_with_invalid_fd_and_nofollow_sets_efault() {
  let mut stat_buf = Stat::default();

  write_errno(EBADF);

  // SAFETY: null path pointer intentionally probes EFAULT precedence over invalid fd.
  let rc = unsafe { fstatat(-1, std::ptr::null(), &raw mut stat_buf, AT_SYMLINK_NOFOLLOW) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_absolute_path_with_invalid_fd_and_null_output_nofollow_sets_efault() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("absolute_null_output_nofollow.txt");
  let file_path_c = path_to_c_string(&file_path);

  fs::write(&file_path, b"payload").expect("failed to create absolute-path null-output test file");

  write_errno(EBADF);

  // SAFETY: absolute path is valid and null output pointer intentionally probes EFAULT behavior.
  let rc = unsafe {
    fstatat(
      -1,
      file_path_c.as_ptr(),
      std::ptr::null_mut(),
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_null_path_and_output_with_nofollow_sets_efault() {
  write_errno(EBADF);

  // SAFETY: null path and output pointers intentionally probe EFAULT handling with non-zero flags.
  let rc = unsafe {
    fstatat(
      AT_FDCWD,
      std::ptr::null(),
      std::ptr::null_mut(),
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_relative_path_with_dirfd_and_null_output_nofollow_sets_efault() {
  let temp_dir = TempDir::new();
  let target_name = CString::new("dirfd_null_output_nofollow.txt").expect("CString::new failed");
  let target_path = temp_dir.path().join("dirfd_null_output_nofollow.txt");
  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");

  fs::write(&target_path, b"payload")
    .expect("failed to create relative-path null-output nofollow test file");

  write_errno(EINVAL);

  // SAFETY: `dirfd` and relative path are valid; null output pointer intentionally probes EFAULT.
  let rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      target_name.as_ptr(),
      std::ptr::null_mut(),
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_relative_dirfd_null_path_and_output_nofollow_sets_efault() {
  let temp_dir = TempDir::new();
  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");

  write_errno(EBADF);

  // SAFETY: `dirfd` is valid; null path/output intentionally probe EFAULT handling.
  let rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      std::ptr::null(),
      std::ptr::null_mut(),
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_relative_path_with_invalid_fd_and_null_output_nofollow_sets_ebadf() {
  let relative_path =
    CString::new("invalid_fd_null_output_nofollow.txt").expect("CString::new failed");

  write_errno(EINVAL);

  // SAFETY: path pointer is valid; invalid fd is expected to fail before output buffer use.
  let rc = unsafe {
    fstatat(
      -1,
      relative_path.as_ptr(),
      std::ptr::null_mut(),
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fstatat_absolute_path_with_non_directory_fd_and_null_output_nofollow_sets_efault() {
  let temp_dir = TempDir::new();
  let target_path = temp_dir
    .path()
    .join("absolute_null_output_nofollow_target.txt");
  let fd_source_path = temp_dir
    .path()
    .join("absolute_null_output_nofollow_fd_source.txt");
  let target_path_c = path_to_c_string(&target_path);

  fs::write(&target_path, b"payload").expect("failed to create absolute-path nofollow target");
  fs::write(&fd_source_path, b"fd-source")
    .expect("failed to create non-directory fd source for absolute-path nofollow test");

  let non_directory_fd =
    File::open(&fd_source_path).expect("failed to open non-directory fd source file");

  write_errno(ENOTDIR);

  // SAFETY: absolute path pointer is valid; Linux ignores non-directory `dirfd` for absolute paths.
  let rc = unsafe {
    fstatat(
      non_directory_fd.as_raw_fd(),
      target_path_c.as_ptr(),
      std::ptr::null_mut(),
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EFAULT);
}

#[test]
fn fstatat_relative_path_with_non_directory_fd_and_null_output_nofollow_sets_enotdir() {
  let temp_dir = TempDir::new();
  let target_name =
    CString::new("nondir_fd_null_output_nofollow_target.txt").expect("CString::new failed");
  let target_path = temp_dir
    .path()
    .join("nondir_fd_null_output_nofollow_target.txt");
  let fd_source_path = temp_dir
    .path()
    .join("nondir_fd_null_output_nofollow_fd_source.txt");

  fs::write(&target_path, b"payload").expect("failed to create relative-path nofollow target");
  fs::write(&fd_source_path, b"fd-source")
    .expect("failed to create non-directory fd source for relative-path nofollow test");

  let non_directory_fd =
    File::open(&fd_source_path).expect("failed to open non-directory fd source file");

  write_errno(EFAULT);

  // SAFETY: relative path pointer is valid; non-directory `dirfd` should fail path resolution first.
  let rc = unsafe {
    fstatat(
      non_directory_fd.as_raw_fd(),
      target_name.as_ptr(),
      std::ptr::null_mut(),
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn fstatat_unsupported_flag_sets_einval() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("invalid_flag.txt");

  fs::write(&file_path, b"ok").expect("failed to create invalid-flag test file");

  let file_path_c = path_to_c_string(&file_path);
  let mut stat_buf = Stat::default();

  write_errno(0);

  // SAFETY: pointers are valid; flag intentionally uses unsupported bits.
  let rc = unsafe { fstatat(AT_FDCWD, file_path_c.as_ptr(), &raw mut stat_buf, i32::MIN) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fstatat_absolute_path_with_unsupported_flag_sets_einval_even_with_invalid_dirfd() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("invalid_flag_absolute.txt");
  let file_path_c = path_to_c_string(&file_path);
  let mut stat_buf = Stat::default();

  fs::write(&file_path, b"ok").expect("failed to create invalid-flag absolute-path test file");
  write_errno(EBADF);

  // SAFETY: absolute path pointer/output pointer are valid; flag intentionally uses unsupported bits.
  let rc = unsafe { fstatat(-1, file_path_c.as_ptr(), &raw mut stat_buf, i32::MIN) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn fstatat_relative_path_with_unsupported_flag_sets_einval_with_directory_fd() {
  let temp_dir = TempDir::new();
  let target_name = CString::new("invalid_flag_relative.txt").expect("CString::new failed");
  let target_path = temp_dir.path().join("invalid_flag_relative.txt");
  let mut stat_buf = Stat::default();

  fs::write(&target_path, b"ok").expect("failed to create invalid-flag relative-path test file");

  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");

  write_errno(EBADF);

  // SAFETY: `dirfd` is valid and pointers are live; flag intentionally uses unsupported bits.
  let rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      target_name.as_ptr(),
      &raw mut stat_buf,
      i32::MIN,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EINVAL);
}

#[test]
fn stat_family_success_paths_preserve_errno() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("preserve_errno.txt");
  let file_path_c = path_to_c_string(&file_path);

  fs::write(&file_path, b"xyz").expect("failed to create preserve-errno test file");

  let file = File::open(&file_path).expect("failed to open preserve-errno test file");
  let mut stat_buf = Stat::default();
  let mut lstat_buf = Stat::default();
  let mut fstat_buf = Stat::default();
  let mut fstatat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: pointers are valid and file exists.
  let stat_rc = unsafe { stat(file_path_c.as_ptr(), &raw mut stat_buf) };

  assert_eq!(stat_rc, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ENOENT);

  // SAFETY: pointers are valid and file exists.
  let lstat_rc = unsafe { lstat(file_path_c.as_ptr(), &raw mut lstat_buf) };

  assert_eq!(lstat_rc, 0);
  assert_eq!(read_errno(), ENOENT);

  write_errno(EFAULT);

  // SAFETY: file descriptor is valid and output pointer is writable.
  let fstat_rc = unsafe { fstat(file.as_raw_fd(), &raw mut fstat_buf) };

  assert_eq!(fstat_rc, 0);
  assert_eq!(read_errno(), EFAULT);

  write_errno(EBADF);

  // SAFETY: `AT_FDCWD` with an existing path and writable output pointer is valid.
  let fstatat_rc = unsafe { fstatat(AT_FDCWD, file_path_c.as_ptr(), &raw mut fstatat_buf, 0) };

  assert_eq!(fstatat_rc, 0);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fstatat_relative_dirfd_success_preserves_errno() {
  let temp_dir = TempDir::new();
  let target_name = CString::new("dirfd_errno_target.txt").expect("CString::new failed");
  let link_name = CString::new("dirfd_errno_target.link").expect("CString::new failed");
  let target_path = temp_dir.path().join("dirfd_errno_target.txt");
  let link_path = temp_dir.path().join("dirfd_errno_target.link");
  let mut stat_buf = Stat::default();
  let mut lstat_buf = Stat::default();

  fs::write(&target_path, b"payload").expect("failed to create dirfd errno test target");
  symlink(&target_path, &link_path).expect("failed to create dirfd errno test symlink");

  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");

  write_errno(EINVAL);

  // SAFETY: `dirfd` is valid and pointers reference live C strings/output buffers.
  let stat_rc = unsafe { fstatat(dir.as_raw_fd(), target_name.as_ptr(), &raw mut stat_buf, 0) };

  assert_eq!(stat_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(stat_buf.st_mode & S_IFMT, S_IFREG);

  write_errno(EBADF);

  // SAFETY: `dirfd` is valid and pointers reference live C strings/output buffers.
  let lstat_rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      link_name.as_ptr(),
      &raw mut lstat_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(lstat_rc, 0);
  assert_eq!(read_errno(), EBADF);
  assert_eq!(lstat_buf.st_mode & S_IFMT, S_IFLNK);
}

#[test]
fn fstatat_with_absolute_path_ignores_dirfd_value_and_preserves_errno() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("absolute_path.txt");
  let file_path_c = path_to_c_string(&file_path);
  let mut stat_buf = Stat::default();

  fs::write(&file_path, b"payload").expect("failed to create absolute-path test file");
  write_errno(EINVAL);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let rc = unsafe { fstatat(-1, file_path_c.as_ptr(), &raw mut stat_buf, 0) };

  assert_eq!(rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(stat_buf.st_mode & S_IFMT, S_IFREG);
}

#[test]
fn fstatat_absolute_path_with_empty_path_flag_ignores_dirfd_and_preserves_errno() {
  let temp_dir = TempDir::new();
  let file_path = temp_dir.path().join("absolute_path_with_empty_flag.txt");
  let file_path_c = path_to_c_string(&file_path);
  let mut by_fstatat = Stat::default();
  let mut by_stat = Stat::default();

  fs::write(&file_path, b"payload")
    .expect("failed to create absolute-path test file for AT_EMPTY_PATH");
  write_errno(EINVAL);

  // SAFETY: absolute path/output pointers are valid; Linux ignores `dirfd` for absolute paths.
  let fstatat_rc = unsafe { fstatat(-1, file_path_c.as_ptr(), &raw mut by_fstatat, AT_EMPTY_PATH) };

  assert_eq!(fstatat_rc, 0);
  assert_eq!(read_errno(), EINVAL);

  write_errno(ENOENT);

  // SAFETY: pointer arguments are valid for the syscall boundary.
  let stat_rc = unsafe { stat(file_path_c.as_ptr(), &raw mut by_stat) };

  assert_eq!(stat_rc, 0);
  assert_eq!(by_fstatat.st_mode & S_IFMT, S_IFREG);
  assert_eq!(by_fstatat.st_dev, by_stat.st_dev);
  assert_eq!(by_fstatat.st_ino, by_stat.st_ino);
}

#[test]
fn fstatat_absolute_missing_path_ignores_dirfd_and_sets_enoent() {
  let temp_dir = TempDir::new();
  let missing_path = temp_dir.path().join("missing_absolute_path.txt");
  let missing_path_c = path_to_c_string(&missing_path);
  let mut stat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: absolute path pointer/output pointer are valid; `dirfd` is ignored for absolute paths.
  let rc = unsafe { fstatat(-1, missing_path_c.as_ptr(), &raw mut stat_buf, 0) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_absolute_missing_path_with_empty_path_flag_ignores_dirfd_and_sets_enoent() {
  let temp_dir = TempDir::new();
  let missing_path = temp_dir
    .path()
    .join("missing_absolute_path_with_empty_flag.txt");
  let missing_path_c = path_to_c_string(&missing_path);
  let mut stat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: absolute path pointer/output pointer are valid; `dirfd` is ignored for absolute paths.
  let empty_path_flag_rc = unsafe {
    fstatat(
      -1,
      missing_path_c.as_ptr(),
      &raw mut stat_buf,
      AT_EMPTY_PATH,
    )
  };

  assert_eq!(empty_path_flag_rc, -1);
  assert_eq!(read_errno(), ENOENT);

  write_errno(EBADF);

  // SAFETY: absolute path pointer/output pointer are valid; `dirfd` is ignored for absolute paths.
  let combined_flags_rc = unsafe {
    fstatat(
      -1,
      missing_path_c.as_ptr(),
      &raw mut stat_buf,
      AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(combined_flags_rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_absolute_missing_path_ignores_non_directory_dirfd() {
  let temp_dir = TempDir::new();
  let non_directory_path = temp_dir.path().join("not_a_directory_fd.txt");
  let missing_path = temp_dir.path().join("missing_with_file_dirfd.txt");
  let missing_path_c = path_to_c_string(&missing_path);
  let mut stat_buf = Stat::default();

  fs::write(&non_directory_path, b"payload").expect("failed to create non-directory dirfd file");

  let non_directory =
    File::open(&non_directory_path).expect("failed to open non-directory dirfd file");

  write_errno(EINVAL);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      missing_path_c.as_ptr(),
      &raw mut stat_buf,
      0,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOENT);

  write_errno(EBADF);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let nofollow_rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      missing_path_c.as_ptr(),
      &raw mut stat_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(nofollow_rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_absolute_symlink_nofollow_ignores_dirfd() {
  let temp_dir = TempDir::new();
  let target_path = temp_dir.path().join("absolute_nofollow_target.txt");
  let link_path = temp_dir.path().join("absolute_nofollow_target.link");
  let link_path_c = path_to_c_string(&link_path);
  let mut nofollow_buf = Stat::default();
  let mut follow_buf = Stat::default();

  fs::write(&target_path, b"payload").expect("failed to create absolute nofollow target");
  symlink(&target_path, &link_path).expect("failed to create absolute nofollow symlink");

  write_errno(EINVAL);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let nofollow_rc = unsafe {
    fstatat(
      -1,
      link_path_c.as_ptr(),
      &raw mut nofollow_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(nofollow_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(nofollow_buf.st_mode & S_IFMT, S_IFLNK);

  write_errno(EBADF);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let follow_rc = unsafe { fstatat(-1, link_path_c.as_ptr(), &raw mut follow_buf, 0) };

  assert_eq!(follow_rc, 0);
  assert_eq!(read_errno(), EBADF);
  assert_eq!(follow_buf.st_mode & S_IFMT, S_IFREG);
}

#[test]
fn fstatat_absolute_symlink_nofollow_ignores_non_directory_dirfd() {
  let temp_dir = TempDir::new();
  let non_directory_path = temp_dir.path().join("absolute_nofollow_dirfd_file.txt");
  let target_path = temp_dir.path().join("absolute_nofollow_dirfd_target.txt");
  let link_path = temp_dir.path().join("absolute_nofollow_dirfd_target.link");
  let link_path_c = path_to_c_string(&link_path);
  let mut nofollow_buf = Stat::default();
  let mut follow_buf = Stat::default();

  fs::write(&non_directory_path, b"fd").expect("failed to create non-directory dirfd file");
  fs::write(&target_path, b"payload").expect("failed to create absolute nofollow target");
  symlink(&target_path, &link_path).expect("failed to create absolute nofollow symlink");

  let non_directory =
    File::open(&non_directory_path).expect("failed to open non-directory dirfd file");

  write_errno(EINVAL);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let nofollow_rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      link_path_c.as_ptr(),
      &raw mut nofollow_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(nofollow_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(nofollow_buf.st_mode & S_IFMT, S_IFLNK);

  write_errno(EBADF);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let follow_rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      link_path_c.as_ptr(),
      &raw mut follow_buf,
      0,
    )
  };

  assert_eq!(follow_rc, 0);
  assert_eq!(read_errno(), EBADF);
  assert_eq!(follow_buf.st_mode & S_IFMT, S_IFREG);
}

#[test]
fn fstatat_absolute_dangling_symlink_nofollow_ignores_non_directory_dirfd() {
  let temp_dir = TempDir::new();
  let non_directory_path = temp_dir.path().join("absolute_dangling_dirfd_file.txt");
  let missing_target_path = temp_dir.path().join("absolute_dangling_target.txt");
  let link_path = temp_dir.path().join("absolute_dangling_target.link");
  let link_path_c = path_to_c_string(&link_path);
  let mut nofollow_buf = Stat::default();
  let mut follow_buf = Stat::default();

  fs::write(&non_directory_path, b"fd").expect("failed to create non-directory dirfd file");
  symlink(&missing_target_path, &link_path).expect("failed to create dangling symlink");

  let non_directory =
    File::open(&non_directory_path).expect("failed to open non-directory dirfd file");

  write_errno(EINVAL);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let nofollow_rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      link_path_c.as_ptr(),
      &raw mut nofollow_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(nofollow_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(nofollow_buf.st_mode & S_IFMT, S_IFLNK);

  write_errno(EBADF);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores `dirfd` for absolute paths.
  let follow_rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      link_path_c.as_ptr(),
      &raw mut follow_buf,
      0,
    )
  };

  assert_eq!(follow_rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_absolute_dangling_symlink_nofollow_ignores_invalid_dirfd() {
  let temp_dir = TempDir::new();
  let missing_target_path = temp_dir.path().join("absolute_dangling_invalid_target.txt");
  let link_path = temp_dir
    .path()
    .join("absolute_dangling_invalid_target.link");
  let link_path_c = path_to_c_string(&link_path);
  let mut nofollow_buf = Stat::default();
  let mut follow_buf = Stat::default();

  symlink(&missing_target_path, &link_path).expect("failed to create dangling symlink");

  write_errno(EINVAL);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores invalid `dirfd` for absolute paths.
  let nofollow_rc = unsafe {
    fstatat(
      -1,
      link_path_c.as_ptr(),
      &raw mut nofollow_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(nofollow_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(nofollow_buf.st_mode & S_IFMT, S_IFLNK);

  write_errno(EBADF);

  // SAFETY: absolute path pointer/output pointer are valid; Linux ignores invalid `dirfd` for absolute paths.
  let follow_rc = unsafe { fstatat(-1, link_path_c.as_ptr(), &raw mut follow_buf, 0) };

  assert_eq!(follow_rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstat_invalid_fd_sets_ebadf() {
  let mut stat_buf = Stat::default();

  write_errno(0);

  // SAFETY: output pointer is valid; descriptor is intentionally invalid.
  let rc = unsafe { fstat(-1, &raw mut stat_buf) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), EBADF);
}

#[test]
fn fstatat_relative_path_with_non_directory_fd_sets_enotdir() {
  let temp_dir = TempDir::new();
  let non_directory_path = temp_dir.path().join("not_a_directory.txt");
  let child_name = CString::new("child").expect("CString::new failed");
  let mut stat_buf = Stat::default();

  fs::write(&non_directory_path, b"payload").expect("failed to create non-directory file");

  let non_directory = File::open(&non_directory_path).expect("failed to open non-directory file");

  write_errno(0);

  // SAFETY: output pointer and C string are valid; `dirfd` intentionally references a regular file.
  let rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      child_name.as_ptr(),
      &raw mut stat_buf,
      0,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn fstatat_relative_path_with_non_directory_fd_sets_enotdir_with_nofollow() {
  let temp_dir = TempDir::new();
  let non_directory_path = temp_dir.path().join("not_a_directory_nofollow.txt");
  let child_name = CString::new("child").expect("CString::new failed");
  let mut stat_buf = Stat::default();

  fs::write(&non_directory_path, b"payload").expect("failed to create non-directory file");

  let non_directory = File::open(&non_directory_path).expect("failed to open non-directory file");

  write_errno(EINVAL);

  // SAFETY: output pointer and C string are valid; `dirfd` intentionally references a regular file.
  let rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      child_name.as_ptr(),
      &raw mut stat_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn fstatat_relative_path_with_non_directory_fd_sets_enotdir_with_empty_path_flag() {
  let temp_dir = TempDir::new();
  let non_directory_path = temp_dir.path().join("not_a_directory_empty_flag.txt");
  let child_name = CString::new("child").expect("CString::new failed");
  let mut stat_buf = Stat::default();

  fs::write(&non_directory_path, b"payload").expect("failed to create non-directory file");

  let non_directory = File::open(&non_directory_path).expect("failed to open non-directory file");

  write_errno(EINVAL);

  // SAFETY: output pointer and C string are valid; `dirfd` intentionally references a regular file.
  let empty_path_flag_rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      child_name.as_ptr(),
      &raw mut stat_buf,
      AT_EMPTY_PATH,
    )
  };

  assert_eq!(empty_path_flag_rc, -1);
  assert_eq!(read_errno(), ENOTDIR);

  write_errno(EBADF);

  // SAFETY: output pointer and C string are valid; combined supported flags still require path resolution.
  let combined_flags_rc = unsafe {
    fstatat(
      non_directory.as_raw_fd(),
      child_name.as_ptr(),
      &raw mut stat_buf,
      AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(combined_flags_rc, -1);
  assert_eq!(read_errno(), ENOTDIR);
}

#[test]
fn fstatat_relative_dirfd_missing_path_sets_enoent() {
  let temp_dir = TempDir::new();
  let missing_name = CString::new("missing-entry.txt").expect("CString::new failed");
  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");
  let mut stat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: `dirfd` is valid and pointers reference live C strings/output buffers.
  let rc = unsafe { fstatat(dir.as_raw_fd(), missing_name.as_ptr(), &raw mut stat_buf, 0) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_relative_dirfd_missing_path_with_empty_path_flag_sets_enoent() {
  let temp_dir = TempDir::new();
  let missing_name =
    CString::new("missing-entry-with-empty-flag.txt").expect("CString::new failed");
  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");
  let mut stat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: `dirfd` is valid and pointers reference live C strings/output buffers.
  let rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      missing_name.as_ptr(),
      &raw mut stat_buf,
      AT_EMPTY_PATH,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_relative_dirfd_missing_path_with_empty_path_and_nofollow_sets_enoent() {
  let temp_dir = TempDir::new();
  let missing_name =
    CString::new("missing-entry-with-empty-flag-nofollow.txt").expect("CString::new failed");
  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");
  let mut stat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: `dirfd` is valid and pointers reference live C strings/output buffers.
  let rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      missing_name.as_ptr(),
      &raw mut stat_buf,
      AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_relative_dirfd_empty_path_sets_enoent() {
  let temp_dir = TempDir::new();
  let empty_name = CString::new("").expect("CString::new failed");
  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");
  let mut stat_buf = Stat::default();

  write_errno(EINVAL);

  // SAFETY: `dirfd` is valid and pointers reference a live empty C string/output buffer.
  let rc = unsafe { fstatat(dir.as_raw_fd(), empty_name.as_ptr(), &raw mut stat_buf, 0) };

  assert_eq!(rc, -1);
  assert_eq!(read_errno(), ENOENT);

  write_errno(EBADF);

  // SAFETY: `dirfd` is valid and pointers reference a live empty C string/output buffer.
  let nofollow_rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      empty_name.as_ptr(),
      &raw mut stat_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(nofollow_rc, -1);
  assert_eq!(read_errno(), ENOENT);
}

#[test]
fn fstatat_relative_symlink_nofollow_respects_flag_with_dirfd() {
  let temp_dir = TempDir::new();
  let target_name = CString::new("target.txt").expect("CString::new failed");
  let link_name = CString::new("target.link").expect("CString::new failed");
  let target_path = temp_dir.path().join("target.txt");
  let link_path = temp_dir.path().join("target.link");
  let mut nofollow_buf = Stat::default();
  let mut follow_buf = Stat::default();

  fs::write(&target_path, b"payload").expect("failed to create target file");
  symlink(&target_path, &link_path).expect("failed to create symlink");

  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");

  write_errno(0);

  // SAFETY: `dirfd` is valid and pointers reference live C strings/output buffers.
  let nofollow_rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      link_name.as_ptr(),
      &raw mut nofollow_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  // SAFETY: `dirfd` is valid and pointers reference live C strings/output buffers.
  let follow_rc = unsafe { fstatat(dir.as_raw_fd(), link_name.as_ptr(), &raw mut follow_buf, 0) };

  assert_eq!(nofollow_rc, 0);
  assert_eq!(follow_rc, 0);
  assert_eq!(read_errno(), 0);
  assert_eq!(nofollow_buf.st_mode & S_IFMT, S_IFLNK);
  assert_eq!(follow_buf.st_mode & S_IFMT, S_IFREG);

  write_errno(0);

  // SAFETY: same valid directory fd with a regular file relative path.
  let target_rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      target_name.as_ptr(),
      &raw mut follow_buf,
      0,
    )
  };

  assert_eq!(target_rc, 0);
  assert_eq!(follow_buf.st_mode & S_IFMT, S_IFREG);
}

#[test]
fn fstatat_relative_dangling_symlink_nofollow_succeeds_while_follow_sets_enoent() {
  let temp_dir = TempDir::new();
  let missing_target_path = temp_dir.path().join("dangling_dirfd_target.txt");
  let link_name = CString::new("dangling_dirfd_target.link").expect("CString::new failed");
  let link_path = temp_dir.path().join("dangling_dirfd_target.link");
  let mut nofollow_buf = Stat::default();
  let mut follow_buf = Stat::default();

  symlink(&missing_target_path, &link_path).expect("failed to create dangling symlink");

  let dir = File::open(temp_dir.path()).expect("failed to open directory fd");

  write_errno(EINVAL);

  // SAFETY: `dirfd` is valid and pointers reference a live dangling symlink name/output buffer.
  let nofollow_rc = unsafe {
    fstatat(
      dir.as_raw_fd(),
      link_name.as_ptr(),
      &raw mut nofollow_buf,
      AT_SYMLINK_NOFOLLOW,
    )
  };

  assert_eq!(nofollow_rc, 0);
  assert_eq!(read_errno(), EINVAL);
  assert_eq!(nofollow_buf.st_mode & S_IFMT, S_IFLNK);

  write_errno(EBADF);

  // SAFETY: `dirfd` is valid and pointers reference a live dangling symlink name/output buffer.
  let follow_rc = unsafe { fstatat(dir.as_raw_fd(), link_name.as_ptr(), &raw mut follow_buf, 0) };

  assert_eq!(follow_rc, -1);
  assert_eq!(read_errno(), ENOENT);
}
