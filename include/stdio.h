#ifndef RLIBC_STDIO_H
#define RLIBC_STDIO_H

#include <stdarg.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define EOF (-1)
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2

typedef struct FILE FILE;

int setvbuf(FILE *stream, char *buffer, int mode, size_t size);
int fflush(FILE *stream);
int vsnprintf(char *s, size_t n, const char *format, va_list ap);
int vfprintf(FILE *stream, const char *format, va_list ap);
int vprintf(const char *format, va_list ap);
int fprintf(FILE *stream, const char *format, ...);
int printf(const char *format, ...);

#ifdef __cplusplus
}
#endif

#endif
