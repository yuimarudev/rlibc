#ifndef RLIBC_SIGNAL_H
#define RLIBC_SIGNAL_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
  unsigned long __val[16];
} sigset_t;

struct sigaction {
  unsigned long sa_handler;
  unsigned long sa_flags;
  unsigned long sa_restorer;
  sigset_t sa_mask;
};

#define SIG_BLOCK 0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

#define SIGABRT 6
#define SIGKILL 9
#define SIGUSR1 10
#define SIGUSR2 12
#define SIGSTOP 19

#define SIG_DFL ((unsigned long)0)
#define SIG_IGN ((unsigned long)1)

#define SA_SIGINFO 0x00000004UL
#define SA_RESTORER 0x04000000UL
#define SA_RESTART 0x10000000UL

int sigemptyset(sigset_t *set);
int sigfillset(sigset_t *set);
int sigaddset(sigset_t *set, int signum);
int sigdelset(sigset_t *set, int signum);
int sigismember(const sigset_t *set, int signum);
int sigaction(int signum, const struct sigaction *act, struct sigaction *oldact);

int raise(int sig);
int kill(int pid, int sig);
int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);

#ifdef __cplusplus
}
#endif

#endif
