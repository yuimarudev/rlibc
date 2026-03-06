#ifndef RLIBC_STDLIB_H
#define RLIBC_STDLIB_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

long strtol(const char *nptr, char **endptr, int base);
long long strtoll(const char *nptr, char **endptr, int base);
unsigned long strtoul(const char *nptr, char **endptr, int base);
unsigned long long strtoull(const char *nptr, char **endptr, int base);

void *malloc(size_t size);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t size);
void *reallocarray(void *ptr, size_t nmemb, size_t size);
size_t malloc_usable_size(void *ptr);
void free(void *ptr);
void cfree(void *ptr);
void *aligned_alloc(size_t alignment, size_t size);
int posix_memalign(void **memptr, size_t alignment, size_t size);
void *memalign(size_t alignment, size_t size);
void *valloc(size_t size);
void *pvalloc(size_t size);

int mblen(const char *s, size_t n);
int mbtowc(wchar_t *pwc, const char *s, size_t n);
int wctomb(char *s, wchar_t wc);
size_t mbstowcs(wchar_t *dst, const char *src, size_t len);
size_t wcstombs(char *dst, const wchar_t *src, size_t len);

int atoi(const char *nptr);
long atol(const char *nptr);
long long atoll(const char *nptr);

extern char **environ;

char *getenv(const char *name);
int setenv(const char *name, const char *value, int overwrite);
int unsetenv(const char *name);
int putenv(char *string);
int clearenv(void);

int atexit(void (*function)(void));
void __libc_start_main(int (*main)(int, char **, char **), int argc, char **argv, char **envp);
void exit(int status);
void _Exit(int status);
void abort(void);

#ifdef __cplusplus
}
#endif

#endif
