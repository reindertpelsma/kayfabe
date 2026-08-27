/* SPDX-License-Identifier: MIT */
/*
 * ioctl_jitter.c — LD_PRELOAD wrapper that adds random 0..1ms sleep
 * before AND after every ioctl() on a /dev/nvidia* fd.
 *
 * Purpose: falsify the hypothesis that our multi-layer roundtrip
 * (guest userspace → guest kernel → virtio → QEMU → stub → host
 *  kernel) breaks something due to its added per-syscall latency.
 *
 * If the host bare-metal cumemalloc_test STILL succeeds with up to
 * 2ms jitter per ioctl, timing is not the bug. If it fails, libcuda
 * has a latency-sensitive code path we need to identify.
 *
 * Build:
 *   cc -O2 -shared -fPIC -o ioctl_jitter.so ioctl_jitter.c -ldl
 *
 * Run:
 *   LD_PRELOAD=./ioctl_jitter.so ./host_cumemalloc_test
 *   NVKVM_JITTER_MAX_NS=2000000 LD_PRELOAD=./ioctl_jitter.so ./host_cumemalloc_test
 *
 * Env vars:
 *   NVKVM_JITTER_MAX_NS  — max nanoseconds of jitter (default 1_000_000 = 1ms)
 *   NVKVM_JITTER_LOG     — set to "1" to log each jittered ioctl to stderr
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <time.h>
#include <sys/ioctl.h>
#include <sys/syscall.h>
#include <sys/stat.h>
#include <unistd.h>
#include <fcntl.h>
#include <stdint.h>
#include <errno.h>

static int    (*real_ioctl)(int fd, unsigned long request, ...) = NULL;
static long   jitter_max_ns = 1000000;  /* 1ms default */
static int    jitter_log    = 0;

static void __attribute__((constructor)) jitter_init(void)
{
    real_ioctl = dlsym(RTLD_NEXT, "ioctl");
    if (getenv("NVKVM_JITTER_MAX_NS"))
        jitter_max_ns = atol(getenv("NVKVM_JITTER_MAX_NS"));
    if (getenv("NVKVM_JITTER_LOG"))
        jitter_log = 1;
    srand((unsigned)(time(NULL) ^ (uintptr_t)&jitter_init));
    fprintf(stderr, "[ioctl_jitter] active: max=%ldns log=%d\n",
            jitter_max_ns, jitter_log);
}

static int is_nvidia_fd(int fd)
{
    /* Resolve /proc/self/fd/N to see if it points at /dev/nvidia* */
    char path[64], link[256];
    snprintf(path, sizeof(path), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(path, link, sizeof(link) - 1);
    if (n <= 0) return 0;
    link[n] = 0;
    return strncmp(link, "/dev/nvidia", 11) == 0;
}

static void jitter(void)
{
    if (jitter_max_ns <= 0) return;
    long ns = rand() % jitter_max_ns;
    struct timespec ts = { .tv_sec = 0, .tv_nsec = ns };
    nanosleep(&ts, NULL);
}

int ioctl(int fd, unsigned long request, ...)
{
    void *arg;
    va_list ap;
    va_start(ap, request);
    arg = va_arg(ap, void *);
    va_end(ap);

    int relevant = is_nvidia_fd(fd);
    if (relevant)
        jitter();

    int ret = real_ioctl(fd, request, arg);
    int saved_errno = errno;

    if (relevant) {
        if (jitter_log)
            fprintf(stderr, "[ioctl_jitter] fd=%d req=0x%lx -> %d errno=%d\n",
                    fd, request, ret, saved_errno);
        jitter();
    }

    errno = saved_errno;
    return ret;
}
