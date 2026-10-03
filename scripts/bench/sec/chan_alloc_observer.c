// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//
// chan_alloc_observer.c — an LD_PRELOAD OBSERVER for docs/design/THE_CONSTRAINTS.md §30: what
// privilege host RM stamped on every GPFIFO channel a process allocates, including the ones
// libcuda allocates inside the process, which kf3's own reply check cannot see.
//
// Host RM writes its verdict into the channel-alloc REPLY: NVOS04_FLAGS_PRIVILEGED_CHANNEL (bit
// 5 of NV_CHANNEL_ALLOC_PARAMS.flags, +20) is set for an ADMIN or KERNEL channel and left as the
// request had it for a USER one (ogkm-580: kernel_channel.c:277-291; internalFlags is zeroed
// before the copy-out, kernel_channel.c:1056-1058, so bit 5 is the reading). The verdict is
// therefore exact only when the REQUEST did not ask for bit 5, so both words are logged.
//
// What it does: wraps ioctl(2). For NV_ESC_RM_ALLOC (type 'F', nr 0x2B) on a /dev/nvidia* fd
// whose class is a GPFIFO channel class (low byte 0x6F, NVIDIA's "NVxx6F" host classes), it
// lets the real call run unchanged and then appends ONE line to $KF_CHANOBS_LOG:
//
//   CHANOBS tid=<tid> thread=<comm> class=0x.. client=0x.. h=0x.. status=0x..
//           request_flags=0x.. reply_flags=0x.. PRIVILEGED_CHANNEL=<0|1|?> capeff_sys_admin=<0|1>
//           nvos=<21|64> params=<set|null> params_size=N nvos_flags=0x..
//
// It never changes an argument or a result. A FINN-serialized NVOS64 block (flags bit 0) is not
// decoded and reads PRIVILEGED_CHANNEL=? — an unmeasured reading, never a zero.
//
// Build (on the box): cc -shared -fPIC -O2 -Wall -o chanobs.so chan_alloc_observer.c -ldl
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <unistd.h>

typedef int (*ioctl_fn)(int, unsigned long, ...);

static int is_nvidia_fd(int fd) {
    char path[64], target[128];
    snprintf(path, sizeof path, "/proc/self/fd/%d", fd);
    ssize_t n = readlink(path, target, sizeof target - 1);
    if (n <= 0) return 0;
    target[n] = 0;
    return strncmp(target, "/dev/nvidia", 11) == 0;
}

static int capeff_sys_admin(void) {
    FILE *f = fopen("/proc/thread-self/status", "r");
    if (!f) return -1;
    char line[256];
    int r = -1;
    while (fgets(line, sizeof line, f)) {
        if (strncmp(line, "CapEff:", 7) == 0) {
            unsigned long long v = strtoull(line + 7, NULL, 16);
            r = (int)((v >> 21) & 1); /* CAP_SYS_ADMIN = 21 */
            break;
        }
    }
    fclose(f);
    return r;
}

static void emit(const char *buf, int len) {
    const char *path = getenv("KF_CHANOBS_LOG");
    if (!path) return;
    int fd = open(path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0644);
    if (fd < 0) return;
    ssize_t w = write(fd, buf, (size_t)len);
    (void)w;
    close(fd);
}

int ioctl(int fd, unsigned long req, ...) {
    static ioctl_fn real;
    if (!real) real = (ioctl_fn)dlsym(RTLD_NEXT, "ioctl");
    va_list ap;
    va_start(ap, req);
    void *arg = va_arg(ap, void *);
    va_end(ap);

    unsigned type = (req >> 8) & 0xff, nr = req & 0xff, size = (req >> 16) & 0x3fff;
    int watch = 0;
    uint32_t cls = 0, req_flags = 0, nvos64_flags = 0;
    uint8_t *p = arg;
    uint64_t parms = 0;
    uint32_t psize = 0;
    if (type == 'F' && nr == 0x2B && p && (size == 32 || size == 48)) {
        memcpy(&cls, p + 12, 4);
        if ((cls & 0xff) == 0x6f && is_nvidia_fd(fd)) {
            memcpy(&parms, p + 16, 8);
            memcpy(&psize, p + (size == 32 ? 24 : 32), 4);
            if (size == 48) memcpy(&nvos64_flags, p + 36, 4);
            if (parms && psize >= 24 && !(nvos64_flags & 1)) {
                memcpy(&req_flags, (uint8_t *)(uintptr_t)parms + 20, 4);
            }
            watch = 1;
        }
    }
    int rc = real(fd, req, arg);
    if (watch) {
        uint32_t client, h, status, reply_flags = 0;
        memcpy(&client, p + 0, 4);
        memcpy(&h, p + 8, 4);
        memcpy(&status, p + (size == 32 ? 28 : 40), 4);
        const char *verdict = "?";
        if (parms && psize >= 24 && !(nvos64_flags & 1)) {
            memcpy(&reply_flags, (uint8_t *)(uintptr_t)parms + 20, 4);
            if (rc == 0 && status == 0 && !(req_flags & 0x20))
                verdict = (reply_flags & 0x20) ? "1" : "0";
        }
        char comm[17] = {0};
        prctl(PR_GET_NAME, comm, 0, 0, 0);
        char buf[512];
        int n = snprintf(buf, sizeof buf,
                         "CHANOBS tid=%ld thread=%s class=%#06x client=%#010x h=%#010x rc=%d "
                         "status=%#x request_flags=%#010x reply_flags=%#010x PRIVILEGED_CHANNEL=%s "
                         "capeff_sys_admin=%d nvos=%u params=%s params_size=%u nvos_flags=%#x\n",
                         (long)syscall(SYS_gettid), comm, cls, client, h, rc, status, req_flags,
                         reply_flags, verdict, capeff_sys_admin(), size == 32 ? 21 : 64,
                         parms ? "set" : "null", psize, nvos64_flags);
        if (n > 0) emit(buf, n < (int)sizeof buf ? n : (int)sizeof buf - 1);
    }
    return rc;
}
