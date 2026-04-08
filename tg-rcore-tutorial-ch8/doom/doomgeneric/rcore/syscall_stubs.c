// syscall_stubs.c — picolibc POSIX stubs backed by rCore system calls
//
// picolibc's stdio (fopen, fread, printf, etc.) calls these low-level
// POSIX functions. We implement them by forwarding to rcore_syscall wrappers.

#include <stdio.h>
#include <errno.h>
#include <stdint.h>
#include <sys/types.h>
#include "rcore_syscall.h"

// ===== File I/O =====

int open(const char *pathname, int flags, ...)
{
    return rcore_open(pathname, flags);
}

int close(int fd)
{
    return rcore_close(fd);
}

ssize_t read(int fd, void *buf, size_t count)
{
    return (ssize_t)rcore_read(fd, buf, count);
}

ssize_t write(int fd, const void *buf, size_t count)
{
    return (ssize_t)rcore_write(fd, buf, count);
}

off_t lseek(int fd, off_t offset, int whence)
{
    return (off_t)rcore_lseek(fd, (long)offset, whence);
}

int unlink(const char *pathname)
{
    (void)pathname;
    errno = ENOSYS;
    return -1;
}

int mkdir(const char *pathname, unsigned int mode)
{
    (void)pathname;
    (void)mode;
    errno = ENOSYS;
    return -1;
}

int rename(const char *oldpath, const char *newpath)
{
    (void)oldpath;
    (void)newpath;
    errno = ENOSYS;
    return -1;
}

// ===== stdio streams =====
// picolibc tinystdio needs stdin, stdout, stderr with put/get callbacks.

static int __stdout_put(char c, FILE *f)
{
    (void)f;
    rcore_write(1, &c, 1);
    return c;
}

static int __stdin_get(FILE *f)
{
    (void)f;
    unsigned char c;
    if (rcore_read(0, &c, 1) == 1)
        return c;
    return EOF;
}

static FILE __stdout_file = FDEV_SETUP_STREAM(__stdout_put, NULL, NULL, __SWR);
static FILE __stdin_file  = FDEV_SETUP_STREAM(NULL, __stdin_get, NULL, __SRD);
static FILE __stderr_file = FDEV_SETUP_STREAM(__stdout_put, NULL, NULL, __SWR);

FILE *const stdin  = &__stdin_file;
FILE *const stdout = &__stdout_file;
FILE *const stderr = &__stderr_file;

// ===== Memory allocation =====
// picolibc's malloc calls sbrk() to grow the heap.

void *sbrk(ptrdiff_t incr)
{
    long ret = rcore_sbrk((int)incr);
    if (ret < 0) {
        errno = ENOMEM;
        return (void *)-1;
    }
    return (void *)ret;
}

// ===== Process stubs =====

void _exit(int status)
{
    rcore_exit(status);
    __builtin_unreachable();
}

int getpid(void)
{
    return rcore_getpid();
}

int kill(int pid, int sig)
{
    (void)pid;
    (void)sig;
    errno = ENOSYS;
    return -1;
}

int isatty(int fd)
{
    (void)fd;
    return 0;
}

int fstat(int fd, void *st)
{
    (void)fd;
    (void)st;
    errno = ENOSYS;
    return -1;
}
