#ifndef RLIBC_WCHAR_H
#define RLIBC_WCHAR_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef int wchar_t;

typedef struct {
  unsigned char __bytes[4];
  unsigned char __count;
  unsigned char __expected;
  unsigned char __reserved[2];
} mbstate_t;

size_t mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps);
size_t mbrlen(const char *s, size_t n, mbstate_t *ps);
int mbsinit(const mbstate_t *ps);
size_t wcrtomb(char *s, wchar_t wc, mbstate_t *ps);
size_t mbsrtowcs(wchar_t *dst, const char **src, size_t len, mbstate_t *ps);
size_t wcsrtombs(char *dst, const wchar_t **src, size_t len, mbstate_t *ps);
int mblen(const char *s, size_t n);
int mbtowc(wchar_t *pwc, const char *s, size_t n);
int wctomb(char *s, wchar_t wc);
size_t mbstowcs(wchar_t *dst, const char *src, size_t len);
size_t wcstombs(char *dst, const wchar_t *src, size_t len);

#ifdef __cplusplus
}
#endif

#endif
