#ifdef RLIBC_NORETURN
#undef RLIBC_NORETURN
#endif

#ifndef RLIBC_SETJMP_H
#define RLIBC_SETJMP_H

#if defined(__cplusplus) && __cplusplus >= 201103L
#define RLIBC_NORETURN [[noreturn]]
#elif defined(_MSC_VER)
#define RLIBC_NORETURN __declspec(noreturn)
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
#define RLIBC_NORETURN _Noreturn
#elif defined(__GNUC__) || defined(__clang__)
#define RLIBC_NORETURN __attribute__((__noreturn__))
#else
#define RLIBC_NORETURN
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef long jmp_buf[8];

int setjmp(jmp_buf env);
RLIBC_NORETURN void longjmp(jmp_buf env, int value);

#ifdef __cplusplus
}
#endif

#undef RLIBC_NORETURN

#endif
