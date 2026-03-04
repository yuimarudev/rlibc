#ifndef RLIBC_SYS_STAT_H
#define RLIBC_SYS_STAT_H

#include <fcntl.h>
#include <time.h>

#ifdef __cplusplus
extern "C" {
#endif

struct stat {
  unsigned long st_dev;
  unsigned long st_ino;
  unsigned long st_nlink;
  unsigned int st_mode;
  unsigned int st_uid;
  unsigned int st_gid;
  int __pad0;
  unsigned long st_rdev;
  long st_size;
  long st_blksize;
  long st_blocks;
  struct timespec st_atim;
  struct timespec st_mtim;
  struct timespec st_ctim;
  long __glibc_reserved[3];
};

int stat(const char *path, struct stat *stat_buf);
int fstat(int fd, struct stat *stat_buf);
int lstat(const char *path, struct stat *stat_buf);
int fstatat(int fd, const char *path, struct stat *stat_buf, int flag);

#ifndef AT_SYMLINK_NOFOLLOW
#define AT_SYMLINK_NOFOLLOW 0x100
#endif

#ifndef AT_EMPTY_PATH
#define AT_EMPTY_PATH 0x1000
#endif

#ifdef __cplusplus
}
#endif

#endif
