#ifndef RLIBC_SYS_UTSNAME_H
#define RLIBC_SYS_UTSNAME_H

#ifdef __cplusplus
extern "C" {
#endif

#define __OLD_UTS_LEN 8
#define __NEW_UTS_LEN 64
#define _UTSNAME_LENGTH (__NEW_UTS_LEN + 1)
#define _UTSNAME_SYSNAME_LENGTH _UTSNAME_LENGTH
#define _UTSNAME_NODENAME_LENGTH _UTSNAME_LENGTH
#define _UTSNAME_RELEASE_LENGTH _UTSNAME_LENGTH
#define _UTSNAME_VERSION_LENGTH _UTSNAME_LENGTH
#define _UTSNAME_MACHINE_LENGTH _UTSNAME_LENGTH
#define _UTSNAME_DOMAIN_LENGTH _UTSNAME_LENGTH
#define SYS_NMLN _UTSNAME_LENGTH

struct utsname {
  char sysname[_UTSNAME_SYSNAME_LENGTH];
  char nodename[_UTSNAME_NODENAME_LENGTH];
  char release[_UTSNAME_RELEASE_LENGTH];
  char version[_UTSNAME_VERSION_LENGTH];
  char machine[_UTSNAME_MACHINE_LENGTH];
  char domainname[_UTSNAME_DOMAIN_LENGTH];
};

int uname(struct utsname *buf);

#ifdef __cplusplus
}
#endif

#endif
