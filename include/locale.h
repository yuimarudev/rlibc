#ifndef RLIBC_LOCALE_H
#define RLIBC_LOCALE_H

#ifdef __cplusplus
extern "C" {
#endif

#define LC_CTYPE 0
#define LC_NUMERIC 1
#define LC_TIME 2
#define LC_COLLATE 3
#define LC_MONETARY 4
#define LC_MESSAGES 5
#define LC_ALL 6

char *setlocale(int category, const char *locale);

#ifdef __cplusplus
}
#endif

#endif
