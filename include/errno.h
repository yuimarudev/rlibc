#ifndef RLIBC_ERRNO_H
#define RLIBC_ERRNO_H

#ifdef __cplusplus
extern "C" {
#endif

int *__errno_location(void);

#ifdef __cplusplus
}
#endif

#define errno (*__errno_location())

#endif
