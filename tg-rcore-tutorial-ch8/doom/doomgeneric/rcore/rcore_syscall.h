// rcore_syscall.h — Raw rCore system call wrappers for RISC-V ecall
//
// This file provides C wrappers around the rCore kernel's ecall interface.
// Syscall numbers match tg-rcore-tutorial-games-syscall/src/syscall.h.in
// (standard Linux RISC-V numbering).

#ifndef RCORE_SYSCALL_H
#define RCORE_SYSCALL_H

#include <stddef.h>
#include <stdint.h>

// ===== Syscall numbers =====
#define SYS_IOCTL       29
#define SYS_OPENAT      56
#define SYS_CLOSE       57
#define SYS_LSEEK       62
#define SYS_READ        63
#define SYS_WRITE       64
#define SYS_FSTAT       80
#define SYS_EXIT        93
#define SYS_NANOSLEEP   101
#define SYS_CLOCK_GETTIME 113
#define SYS_SCHED_YIELD 124
#define SYS_KILL        129
#define SYS_GETPID      172
#define SYS_BRK         214
#define SYS_MMAP        222

// ===== Open flags (matches OpenFlags in user.rs) =====
#define RCORE_O_RDONLY   0
#define RCORE_O_WRONLY   (1 << 0)
#define RCORE_O_RDWR     (1 << 1)
#define RCORE_O_CREATE   (1 << 9)
#define RCORE_O_TRUNC    (1 << 10)

// ===== mmap constants =====
#define RCORE_PROT_READ  1
#define RCORE_PROT_WRITE 2
#define RCORE_MAP_PRIVATE 0x1

// ===== ioctl constants for framebuffer =====
#define RCORE_FB_FLUSH          1
#define RCORE_FB_GET_RESOLUTION 2

// ===== TimeSpec (matches rCore TimeSpec: two usize fields) =====
struct rcore_timespec {
    unsigned long tv_sec;
    unsigned long tv_nsec;
};

// ===== InputEvent (matches breakout.rs InputEvent) =====
struct rcore_input_event {
    uint64_t timestamp_usec;
    uint16_t event_type;
    uint16_t code;
    uint32_t value;
};

#define RCORE_EV_KEY 1

// ===== Raw ecall wrappers =====
long rcore_syscall0(long id);
long rcore_syscall1(long id, long a0);
long rcore_syscall2(long id, long a0, long a1);
long rcore_syscall3(long id, long a0, long a1, long a2);
long rcore_syscall6(long id, long a0, long a1, long a2, long a3, long a4, long a5);

// ===== High-level wrappers =====

// Note: rCore's open takes (path_ptr, path_len, flags) - NOT null-terminated!
int    rcore_open(const char *path, int flags);
int    rcore_close(int fd);
long   rcore_read(int fd, void *buf, size_t count);
long   rcore_write(int fd, const void *buf, size_t count);
long   rcore_lseek(int fd, long offset, int whence);
int    rcore_ioctl(int fd, unsigned long request, unsigned long argp);
long   rcore_mmap(unsigned long start, size_t len, int prot, int flags, int fd, long offset);
void   rcore_exit(int status);
long   rcore_sbrk(int incr);
int    rcore_clock_gettime(int clockid, struct rcore_timespec *tp);
int    rcore_nanosleep(const struct rcore_timespec *req);
int    rcore_sched_yield(void);
int    rcore_getpid(void);

#endif // RCORE_SYSCALL_H
