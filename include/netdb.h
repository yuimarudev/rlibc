#ifndef RLIBC_NETDB_H
#define RLIBC_NETDB_H

#include <sys/socket.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AF_UNSPEC 0
#define AF_INET 2
#define AF_INET6 10

#define AI_PASSIVE 0x0001
#define AI_NUMERICHOST 0x0004
#define AI_NUMERICSERV 0x0400

#define NI_NUMERICHOST 0x01
#define NI_NUMERICSERV 0x02
#define NI_NOFQDN 0x04
#define NI_NAMEREQD 0x08
#define NI_DGRAM 0x10

#define EAI_BADFLAGS (-1)
#define EAI_NONAME (-2)
#define EAI_AGAIN (-3)
#define EAI_FAIL (-4)
#define EAI_FAMILY (-6)
#define EAI_SOCKTYPE (-7)
#define EAI_SERVICE (-8)
#define EAI_MEMORY (-10)
#define EAI_SYSTEM (-11)
#define EAI_OVERFLOW (-12)

struct addrinfo {
  int ai_flags;
  int ai_family;
  int ai_socktype;
  int ai_protocol;
  socklen_t ai_addrlen;
  struct sockaddr *ai_addr;
  char *ai_canonname;
  struct addrinfo *ai_next;
};

int getaddrinfo(
  const char *node,
  const char *service,
  const struct addrinfo *hints,
  struct addrinfo **res
);
void freeaddrinfo(struct addrinfo *res);
const char *gai_strerror(int errcode);

int getnameinfo(
  const struct sockaddr *addr,
  socklen_t addrlen,
  char *host,
  socklen_t hostlen,
  char *serv,
  socklen_t servlen,
  int flags
);

#ifdef __cplusplus
}
#endif

#endif
