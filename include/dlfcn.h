#ifndef RLIBC_DLFCN_H
#define RLIBC_DLFCN_H

#ifdef __cplusplus
extern "C" {
#endif

#define RTLD_LAZY 0x0001
#define RTLD_NOW 0x0002
#define RTLD_GLOBAL 0x0100
#define RTLD_LOCAL 0
#define RTLD_DEFAULT ((void *)0)
#define RTLD_NEXT ((void *)-1l)

void *dlopen(const char *filename, int flags);
int dlclose(void *handle);
char *dlerror(void);
void *dlsym(void *handle, const char *symbol);

#ifdef __cplusplus
}
#endif

#endif
