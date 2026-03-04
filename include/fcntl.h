#ifndef RLIBC_FCNTL_H
#define RLIBC_FCNTL_H

#ifdef __cplusplus
extern "C" {
#endif

#define F_DUPFD 0
#define F_GETFD 1
#define F_SETFD 2
#define F_DUPFD_CLOEXEC 1030
#define F_GETFL 3
#define F_SETFL 4

#define FD_CLOEXEC 1
#define O_ACCMODE 03
#define O_NONBLOCK 04000
#define O_RDONLY 00
#define AT_FDCWD -100

#ifndef AT_EMPTY_PATH
#define AT_EMPTY_PATH 0x1000
#endif

int fcntl(int fd, int cmd, ...);
int open(const char *pathname, int flags, ...);
int openat(int dirfd, const char *pathname, int flags, ...);

#ifdef __cplusplus
}
#endif

#endif
