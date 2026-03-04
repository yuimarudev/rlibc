#ifndef RLIBC_STRING_H
#define RLIBC_STRING_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

void *memmove(void *dst, const void *src, size_t n);
void *memcpy(void *dst, const void *src, size_t n);
void *memset(void *s, int c, size_t n);
int memcmp(const void *left, const void *right, size_t n);

size_t strlen(const char *s);
size_t strnlen(const char *s, size_t n);

#ifdef __cplusplus
}
#endif

#endif
