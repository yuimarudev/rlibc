#ifndef RLIBC_DIRENT_H
#define RLIBC_DIRENT_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct rlibc_dir DIR;

struct dirent {
  unsigned long d_ino;
  long d_off;
  unsigned short d_reclen;
  unsigned char d_type;
  char d_name[256];
};

DIR *opendir(const char *path);
struct dirent *readdir(DIR *dirp);
int closedir(DIR *dirp);
void rewinddir(DIR *dirp);

#ifdef __cplusplus
}
#endif

#endif
