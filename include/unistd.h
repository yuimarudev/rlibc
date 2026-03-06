#ifndef RLIBC_UNISTD_H
#define RLIBC_UNISTD_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef __PTRDIFF_TYPE__ ssize_t;
typedef __INT64_TYPE__ off_t;
typedef __UINT32_TYPE__ uid_t;
typedef __UINT32_TYPE__ gid_t;

#define F_OK 0
#define X_OK 1
#define W_OK 2
#define R_OK 4

#define MSG_PEEK 0x2
#define MSG_DONTWAIT 0x40
#define MSG_WAITALL 0x100
#define MSG_NOSIGNAL 0x4000
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#define _SC_CLK_TCK 2
#define _SC_OPEN_MAX 4
#define _SC_PAGESIZE 30
#define _SC_PAGE_SIZE 30
#define _SC_NPROCESSORS_CONF 83
#define _SC_NPROCESSORS_ONLN 84
#define HOST_NAME_MAX 64

int access(const char *pathname, int mode);
int unlink(const char *pathname);
int close(int fd);
int dup(int oldfd);
int dup2(int oldfd, int newfd);
int dup3(int oldfd, int newfd, int flags);
off_t lseek(int fd, off_t offset, int whence);
int pipe(int pipefd[2]);
int pipe2(int pipefd[2], int flags);
int fsync(int fd);
int fdatasync(int fd);
int syncfs(int fd);
void sync(void);
int getpid(void);
int getppid(void);
int getpgid(int pid);
int getpgrp(void);
int getsid(int pid);
int gettid(void);
uid_t getuid(void);
uid_t geteuid(void);
gid_t getgid(void);
gid_t getegid(void);
int isatty(int fd);
ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
ssize_t send(int sockfd, const void *buf, size_t len, int flags);
ssize_t recv(int sockfd, void *buf, size_t len, int flags);
int gethostname(char *name, size_t len);
int getpagesize(void);
long sysconf(int name);

#ifdef __cplusplus
}
#endif

#endif
