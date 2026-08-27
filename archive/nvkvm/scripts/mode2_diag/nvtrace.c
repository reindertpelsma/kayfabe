/*
 * nvtrace.c — ptrace(PTRACE_SYSCALL) NVIDIA ioctl/mmap tracer.
 *
 * Why ptrace (not LD_PRELOAD): catches every syscall at the kernel boundary
 * regardless of how it was issued (libc wrapper, inline asm, raw `syscall`), and
 * automatically includes /dev/nvidia-uvm ioctls. Follows all threads/children
 * (libcuda spawns workers). Reads the NVOS struct AND the params-buffer writeback
 * out of the tracee with process_vm_readv at syscall EXIT. Decoded + diffed by
 * nvdecode.py (semantic field decode, host-vs-guest).
 *
 *   build: gcc -O2 -o nvtrace nvtrace.c
 *   run:   ./nvtrace -o trace.txt -- <prog> [args...]
 *
 * Records:
 *   OPEN  tid fd path
 *   IOCTL tid fd path req nr size ret arg hdr=<hex> [params psz p=<hex>]
 *   MMAP  tid fd path addr len prot flags off ret
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <signal.h>
#include <sys/ptrace.h>
#include <sys/wait.h>
#include <sys/uio.h>
#include <sys/types.h>
#include <sys/syscall.h>
#include <linux/ptrace.h>
#include <stdint.h>

#ifndef PTRACE_GET_SYSCALL_INFO
#define PTRACE_GET_SYSCALL_INFO 0x420e
#endif
#define ull unsigned long long

static FILE *out;
#define MAXFD 8192
static char fdpath[MAXFD][96];

#define MAXTID 4096
typedef struct { pid_t tid; uint64_t nr, a[6]; } entry_t;
static entry_t ent[MAXTID];
static entry_t *entslot(pid_t tid)
{
    for (int k = 0; k < MAXTID; k++) if (ent[k].tid == tid) return &ent[k];
    for (int k = 0; k < MAXTID; k++) if (ent[k].tid == 0) { ent[k].tid = tid; return &ent[k]; }
    return &ent[0];
}

static long pvread(pid_t pid, uint64_t addr, void *buf, size_t n)
{
    struct iovec lo = { buf, n }, ro = { (void *)(uintptr_t)addr, n };
    return process_vm_readv(pid, &lo, 1, &ro, 1, 0);
}
static void hexdump(const unsigned char *b, size_t n){ for(size_t i=0;i<n;i++) fprintf(out,"%02x",b[i]); }

static void handle_ioctl(pid_t tid, uint64_t fd, uint64_t req, uint64_t arg, long ret)
{
    unsigned type = (req >> 8) & 0xff, nr = req & 0xff, size = (req >> 16) & 0x3fff;
    const char *path = (fd < MAXFD && fdpath[fd][0]) ? fdpath[fd] : "?";
    if (!(strstr(path, "nvidia") || type == 0x46)) return;
    fprintf(out, "IOCTL tid=%d fd=%llu path=%s req=0x%llx nr=0x%02x size=%u ret=%ld arg=0x%llx",
            tid, (ull)fd, path, (ull)req, nr, size, ret, (ull)arg);
    unsigned hdrn = size ? (size < 512 ? size : 512) : 64;
    unsigned char hdr[512];
    if (arg && pvread(tid, arg, hdr, hdrn) == (long)hdrn) {
        fprintf(out, " hdr="); hexdump(hdr, hdrn);
        if (type == 0x46 && (nr == 0x2A || nr == 0x2B) && hdrn >= 32) {
            uint64_t pp = *(uint64_t *)(hdr + 16);
            uint32_t psz = *(uint32_t *)(hdr + 24);
            if (pp && psz && psz <= (1u << 20)) {
                unsigned pn = psz < 1024 ? psz : 1024; unsigned char pb[1024];
                if (pvread(tid, pp, pb, pn) == (long)pn) {
                    fprintf(out, " params=0x%llx psz=%u p=", (ull)pp, psz); hexdump(pb, pn);
                }
            }
        }
    }
    fprintf(out, "\n"); fflush(out);
}

static void handle_open(pid_t tid, uint64_t pathaddr, long ret)
{
    if (ret < 0 || ret >= MAXFD) return;
    char p[96] = {0};
    if (pvread(tid, pathaddr, p, sizeof(p) - 1) > 0) {
        p[sizeof(p) - 1] = 0;
        strncpy(fdpath[ret], p, sizeof(fdpath[0]) - 1);
        if (strstr(p, "nvidia") || strstr(p, "/dev/"))
            { fprintf(out, "OPEN tid=%d fd=%ld path=%s\n", tid, ret, p); fflush(out); }
    }
}

static void handle_mmap(pid_t tid, uint64_t *a, long ret)
{
    uint64_t fd = a[4];
    const char *path = (fd < MAXFD && fdpath[fd][0]) ? fdpath[fd] : "?";
    if (!strstr(path, "nvidia")) return;   /* only nvidia-fd mmaps */
    fprintf(out, "MMAP tid=%d fd=%llu path=%s addr=0x%llx len=%llu prot=0x%llx flags=0x%llx off=0x%llx ret=0x%lx\n",
            tid, (ull)fd, path, (ull)a[0], (ull)a[1], (ull)a[2], (ull)a[3], (ull)a[5], ret);
    fflush(out);
}

int main(int argc, char **argv)
{
    const char *ofile = NULL; int i = 1;
    for (; i < argc; i++) {
        if (!strcmp(argv[i], "-o") && i + 1 < argc) ofile = argv[++i];
        else if (!strcmp(argv[i], "--")) { i++; break; }
        else break;
    }
    out = ofile ? fopen(ofile, "w") : stderr;
    if (!out) { perror("out"); return 1; }
    if (i >= argc) { fprintf(stderr, "usage: nvtrace -o out -- prog args\n"); return 1; }

    pid_t child = fork();
    if (child == 0) {
        ptrace(PTRACE_TRACEME, 0, 0, 0);
        execvp(argv[i], &argv[i]); perror("execvp"); _exit(127);
    }
    int st; waitpid(child, &st, 0);
    ptrace(PTRACE_SETOPTIONS, child, 0,
           PTRACE_O_TRACESYSGOOD | PTRACE_O_TRACECLONE | PTRACE_O_TRACEFORK |
           PTRACE_O_TRACEVFORK | PTRACE_O_EXITKILL);
    ptrace(PTRACE_SYSCALL, child, 0, 0);

    for (;;) {
        pid_t tid = waitpid(-1, &st, __WALL);
        if (tid < 0) { if (errno == ECHILD) break; continue; }
        if (WIFEXITED(st) || WIFSIGNALED(st)) continue;
        if (!WIFSTOPPED(st)) continue;
        int sig = WSTOPSIG(st); unsigned event = (unsigned)st >> 16;
        if (event == PTRACE_EVENT_CLONE || event == PTRACE_EVENT_FORK || event == PTRACE_EVENT_VFORK) {
            ptrace(PTRACE_SYSCALL, tid, 0, 0); continue;
        }
        if (sig == (SIGTRAP | 0x80)) {
            struct ptrace_syscall_info si; memset(&si, 0, sizeof(si));
            long r = ptrace(PTRACE_GET_SYSCALL_INFO, tid, sizeof(si), &si);
            if (r > 0) {
                if (si.op == PTRACE_SYSCALL_INFO_ENTRY) {
                    entry_t *e = entslot(tid);
                    e->nr = si.entry.nr;
                    for (int j = 0; j < 6; j++) e->a[j] = si.entry.args[j];
                } else if (si.op == PTRACE_SYSCALL_INFO_EXIT) {
                    entry_t *e = entslot(tid);
                    long ret = si.exit.rval;
                    if (e->nr == SYS_ioctl)       handle_ioctl(tid, e->a[0], e->a[1], e->a[2], ret);
                    else if (e->nr == SYS_openat) handle_open(tid, e->a[1], ret);
                    else if (e->nr == SYS_mmap)   handle_mmap(tid, e->a, ret);
                }
            }
            ptrace(PTRACE_SYSCALL, tid, 0, 0); continue;
        }
        ptrace(PTRACE_SYSCALL, tid, 0, (sig == SIGTRAP ? 0 : sig));
    }
    fclose(out); return 0;
}
