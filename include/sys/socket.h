#ifndef RLIBC_SYS_SOCKET_H
#define RLIBC_SYS_SOCKET_H

#ifdef __cplusplus
extern "C" {
#endif

typedef unsigned short sa_family_t;
typedef unsigned int socklen_t;

struct sockaddr {
  sa_family_t sa_family;
  char sa_data[14];
};

struct sockaddr_un {
  sa_family_t sun_family;
  char sun_path[108];
};

#define AF_UNIX 1

#define SOCK_STREAM 1
#define SOCK_NONBLOCK 04000
#define SOCK_CLOEXEC 02000000

int socket(int domain, int type, int protocol);
int connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen);
int bind(int sockfd, const struct sockaddr *addr, socklen_t addrlen);
int listen(int sockfd, int backlog);
int accept(int sockfd, struct sockaddr *addr, socklen_t *addrlen);

#ifdef __cplusplus
}
#endif

#endif
