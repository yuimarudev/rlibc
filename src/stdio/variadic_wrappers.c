#include <stdarg.h>
#include <stdio.h>

int fprintf(FILE *stream, const char *format, ...) {
  va_list args;
  int result;

  va_start(args, format);
  result = vfprintf(stream, format, args);
  va_end(args);

  return result;
}

int printf(const char *format, ...) {
  va_list args;
  int result;

  va_start(args, format);
  result = vprintf(format, args);
  va_end(args);

  return result;
}
