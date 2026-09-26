/*
 * nvioctl_log.so — LD_PRELOAD logger for E6'' phase 0.
 *
 * Records, for any process (tinygrad, libcuda):
 *   - every ioctl on /dev/nvidia-uvm : name, return, rmStatus, and the first
 *     bytes of the params (enough to see REGISTER_GPU_VASPACE / REGISTER_CHANNEL /
 *     CREATE_EXTERNAL_RANGE / MAP_EXTERNAL_ALLOCATION arguments);
 *   - RM escapes on /dev/nvidiactl and /dev/nvidiaN : RM_ALLOC (class, status),
 *     RM_CONTROL (cmd, status), MAP_MEMORY_DMA (hDma, flags, offset, status),
 *     and the escape number of everything else;
 *   - mmap of any nvidia fd.
 * Output: one line per event to $NVLOG (default stderr). Pure observation: the
 * real call is always made first, and its result returned unchanged.
 *
 * Build: gcc -O2 -shared -fPIC -o nvioctl_log.so nvioctl_log.c -ldl
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include "uvm_table.h"

enum { K_NONE = 0, K_UVM = 1, K_CTL = 2, K_DEV = 3 };
static int (*real_ioctl)(int, unsigned long, ...);
static void *(*real_mmap)(void *, size_t, int, int, int, off_t);
static FILE *out;
static pthread_mutex_t mu = PTHREAD_MUTEX_INITIALIZER;

static void init(void) {
    if (real_ioctl) return;
    real_ioctl = dlsym(RTLD_NEXT, "ioctl");
    real_mmap  = dlsym(RTLD_NEXT, "mmap");
    const char *p = getenv("NVLOG");
    out = p ? fopen(p, "a") : NULL;
    if (!out) out = stderr;
    setvbuf(out, NULL, _IOLBF, 0);
}

static int fd_kind(int fd, char *path, size_t n) {
    char l[64]; snprintf(l, sizeof l, "/proc/self/fd/%d", fd);
    ssize_t r = readlink(l, path, n - 1);
    if (r <= 0) return K_NONE;
    path[r] = 0;
    if (!strcmp(path, "/dev/nvidia-uvm") || !strcmp(path, "/dev/nvidia-uvm-tools")) return K_UVM;
    if (!strcmp(path, "/dev/nvidiactl")) return K_CTL;
    if (!strncmp(path, "/dev/nvidia", 11)) return K_DEV;
    return K_NONE;
}

static double now_ms(void) {
    struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static void hexdump(const unsigned char *p, unsigned n) {
    for (unsigned i = 0; i < n; i++) fprintf(out, "%02x%s", p[i], (i % 4 == 3) ? " " : "");
}

/* RM param layouts, from ogkm-580.159.04 src/common/sdk/nvidia/inc/nvos.h */
typedef struct { uint32_t hRoot, hParent, hNew, hClass; uint64_t pParams; uint32_t paramsSize, status; } os21_t;
typedef struct { uint32_t hClient, hObject, cmd, flags; uint64_t params; uint32_t paramsSize, status; } os54_t;
typedef struct { uint32_t hClient, hDevice, hDma, hMemory; uint64_t offset, length; uint32_t flags, flags2, kind; uint32_t pad; uint64_t dmaOffset; uint32_t status; } os46_t;

int ioctl(int fd, unsigned long req, ...) {
    va_list ap; va_start(ap, req); void *arg = va_arg(ap, void *); va_end(ap);
    init();
    char path[128];
    int kind = fd_kind(fd, path, sizeof path);
    int r = real_ioctl(fd, req, arg);
    if (kind == K_NONE) return r;

    pthread_mutex_lock(&mu);
    fprintf(out, "%.3f pid=%d ", now_ms(), getpid());
    if (kind == K_UVM) {
        unsigned nr = (unsigned)req; const char *name = "UVM_?"; unsigned sz = 0;
        for (unsigned i = 0; i < sizeof uvm_tab / sizeof uvm_tab[0]; i++)
            if (uvm_tab[i].nr == nr) { name = uvm_tab[i].name; sz = uvm_tab[i].sz; break; }
        fprintf(out, "UVM %s(%u) ret=%d", name, nr, r);
        if (arg && sz >= 4) {
            fprintf(out, " rmStatus=0x%x args=", *(uint32_t *)((char *)arg + sz - 4));
            hexdump(arg, sz > 64 ? 64 : sz - 4);
        }
    } else {
        unsigned esc = _IOC_NR(req);
        fprintf(out, "RM %s esc=0x%02x ret=%d", kind == K_CTL ? "ctl" : path, esc, r);
        if (arg && esc == 0x2B) {
            os21_t *p = arg;
            fprintf(out, " ALLOC class=0x%04x parent=0x%x new=0x%x status=0x%x", p->hClass, p->hParent, p->hNew, p->status);
        } else if (arg && esc == 0x2A) {
            os54_t *p = arg;
            fprintf(out, " CONTROL cmd=0x%08x obj=0x%x status=0x%x", p->cmd, p->hObject, p->status);
        } else if (arg && esc == 0x57) {
            os46_t *p = arg;
            fprintf(out, " MAP_DMA hDma=0x%x hMem=0x%x len=0x%llx flags=0x%x off=0x%llx status=0x%x",
                    p->hDma, p->hMemory, (unsigned long long)p->length, p->flags,
                    (unsigned long long)p->dmaOffset, p->status);
        }
    }
    fputc('\n', out);
    pthread_mutex_unlock(&mu);
    return r;
}

void *mmap(void *addr, size_t len, int prot, int flags, int fd, off_t off) {
    init();
    void *p = real_mmap(addr, len, prot, flags, fd, off);
    if (fd >= 0) {
        char path[128];
        int kind = fd_kind(fd, path, sizeof path);
        if (kind != K_NONE) {
            pthread_mutex_lock(&mu);
            fprintf(out, "%.3f pid=%d MMAP %s addr=%p len=0x%zx off=0x%llx -> %p\n",
                    now_ms(), getpid(), path, addr, len, (unsigned long long)off, p);
            pthread_mutex_unlock(&mu);
        }
    }
    return p;
}
