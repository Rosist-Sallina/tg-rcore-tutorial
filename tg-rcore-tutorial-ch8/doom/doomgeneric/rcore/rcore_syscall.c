// rcore_syscall.c — Raw rCore system call wrappers via RISC-V ecall
//
// Provides the actual inline-asm ecall implementations and
// high-level C wrappers for use by syscall_stubs.c and doomgeneric_rcore.c.

#include "rcore_syscall.h"
#include <string.h>

// ===== Raw ecall wrappers =====

long rcore_syscall0(long id)
{
    register long a7 __asm__("a7") = id;
    register long a0 __asm__("a0");
    __asm__ volatile("ecall"
                     : "=r"(a0)
                     : "r"(a7)
                     : "memory");
    return a0;
}

long rcore_syscall1(long id, long a0_val)
{
    register long a7 __asm__("a7") = id;
    register long a0 __asm__("a0") = a0_val;
    __asm__ volatile("ecall"
                     : "+r"(a0)
                     : "r"(a7)
                     : "memory");
    return a0;
}

long rcore_syscall2(long id, long a0_val, long a1_val)
{
    register long a7 __asm__("a7") = id;
    register long a0 __asm__("a0") = a0_val;
    register long a1 __asm__("a1") = a1_val;
    __asm__ volatile("ecall"
                     : "+r"(a0)
                     : "r"(a7), "r"(a1)
                     : "memory");
    return a0;
}

long rcore_syscall3(long id, long a0_val, long a1_val, long a2_val)
{
    register long a7 __asm__("a7") = id;
    register long a0 __asm__("a0") = a0_val;
    register long a1 __asm__("a1") = a1_val;
    register long a2 __asm__("a2") = a2_val;
    __asm__ volatile("ecall"
                     : "+r"(a0)
                     : "r"(a7), "r"(a1), "r"(a2)
                     : "memory");
    return a0;
}

long rcore_syscall6(long id, long a0_val, long a1_val, long a2_val,
                    long a3_val, long a4_val, long a5_val)
{
    register long a7 __asm__("a7") = id;
    register long a0 __asm__("a0") = a0_val;
    register long a1 __asm__("a1") = a1_val;
    register long a2 __asm__("a2") = a2_val;
    register long a3 __asm__("a3") = a3_val;
    register long a4 __asm__("a4") = a4_val;
    register long a5 __asm__("a5") = a5_val;
    __asm__ volatile("ecall"
                     : "+r"(a0)
                     : "r"(a7), "r"(a1), "r"(a2), "r"(a3), "r"(a4), "r"(a5)
                     : "memory");
    return a0;
}

// ===== High-level wrappers =====

// rCore open takes (path_ptr, path_len, flags) — NOT null-terminated
int rcore_open(const char *path, int flags)
{
    size_t len = strlen(path);
    return (int)rcore_syscall3(SYS_OPENAT,
                               (long)path,
                               (long)len,
                               (long)flags);
}

int rcore_close(int fd)
{
    return (int)rcore_syscall1(SYS_CLOSE, (long)fd);
}

long rcore_read(int fd, void *buf, size_t count)
{
    return rcore_syscall3(SYS_READ,
                          (long)fd,
                          (long)buf,
                          (long)count);
}

long rcore_write(int fd, const void *buf, size_t count)
{
    return rcore_syscall3(SYS_WRITE,
                          (long)fd,
                          (long)buf,
                          (long)count);
}

long rcore_lseek(int fd, long offset, int whence)
{
    return rcore_syscall3(SYS_LSEEK,
                          (long)fd,
                          offset,
                          (long)whence);
}

int rcore_ioctl(int fd, unsigned long request, unsigned long argp)
{
    return (int)rcore_syscall3(SYS_IOCTL,
                               (long)fd,
                               (long)request,
                               (long)argp);
}

long rcore_mmap(unsigned long start, size_t len, int prot, int flags,
                int fd, long offset)
{
    return rcore_syscall6(SYS_MMAP,
                          (long)start,
                          (long)len,
                          (long)prot,
                          (long)flags,
                          (long)fd,
                          offset);
}

void rcore_exit(int status)
{
    rcore_syscall1(SYS_EXIT, (long)status);
    __builtin_unreachable();
}

long rcore_sbrk(int incr)
{
    return rcore_syscall1(SYS_BRK, (long)incr);
}

int rcore_clock_gettime(int clockid, struct rcore_timespec *tp)
{
    return (int)rcore_syscall2(SYS_CLOCK_GETTIME,
                               (long)clockid,
                               (long)tp);
}

// int rcore_nanosleep(const struct rcore_timespec *req)
// {
//     return (int)rcore_syscall2(SYS_NANOSLEEP,
//                                (long)req,
//                                0); // rem = NULL
// }

int rcore_sched_yield(void)
{
    return (int)rcore_syscall0(SYS_SCHED_YIELD);
}

int rcore_getpid(void)
{
    return (int)rcore_syscall0(SYS_GETPID);
}
