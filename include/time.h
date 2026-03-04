#ifndef RLIBC_TIME_H
#define RLIBC_TIME_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef int clockid_t;
typedef long time_t;

#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#define CLOCK_PROCESS_CPUTIME_ID 2
#define CLOCK_THREAD_CPUTIME_ID 3
#define CLOCK_MONOTONIC_RAW 4
#define CLOCK_REALTIME_COARSE 5
#define CLOCK_MONOTONIC_COARSE 6
#define CLOCK_BOOTTIME 7
#define CLOCK_REALTIME_ALARM 8
#define CLOCK_BOOTTIME_ALARM 9
#define CLOCK_SGI_CYCLE 10
#define CLOCK_TAI 11
#define CLOCKFD 3
#define FD_TO_CLOCKID(fd) ((~(clockid_t)(fd) << 3) | CLOCKFD)
#define CLOCKID_TO_FD(clk) (~((clockid_t)(clk) >> 3))

struct timespec {
  long tv_sec;
  long tv_nsec;
};

struct timeval {
  long tv_sec;
  long tv_usec;
};

struct timezone {
  int tz_minuteswest;
  int tz_dsttime;
};

struct tm {
  int tm_sec;
  int tm_min;
  int tm_hour;
  int tm_mday;
  int tm_mon;
  int tm_year;
  int tm_wday;
  int tm_yday;
  int tm_isdst;
  long tm_gmtoff;
  const char *tm_zone;
};

int clock_gettime(clockid_t clock_id, struct timespec *tp);
int gettimeofday(struct timeval *tv, struct timezone *tz);
struct tm *gmtime_r(const time_t *timer, struct tm *result);
struct tm *gmtime(const time_t *timer);
struct tm *localtime_r(const time_t *timer, struct tm *result);
struct tm *localtime(const time_t *timer);
time_t timegm(struct tm *time_parts);
time_t mktime(struct tm *time_parts);
size_t strftime(char *s, size_t max, const char *format, const struct tm *time_ptr);

#ifdef __cplusplus
}
#endif

#endif
