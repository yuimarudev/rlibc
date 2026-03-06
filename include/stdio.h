#ifndef RLIBC_STDIO_H
#define RLIBC_STDIO_H

#include <stdarg.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define EOF (-1)
#define BUFSIZ 8192
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2

typedef struct FILE FILE;

FILE *tmpfile(void);
FILE *fopen(const char *path, const char *mode);
size_t fread(void *ptr, size_t size, size_t nmemb, FILE *stream);
int fputs(const char *s, FILE *stream);
int fileno(FILE *stream);
int fileno_unlocked(FILE *stream);
void flockfile(FILE *stream);
int ftrylockfile(FILE *stream);
void funlockfile(FILE *stream);
int fclose(FILE *stream);
void setbuffer(FILE *stream, char *buffer, size_t size);
void setbuf(FILE *stream, char *buffer);
void setlinebuf(FILE *stream);
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
