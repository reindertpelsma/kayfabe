/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nvioctl_trace — ptrace-based capture of every ioctl on /dev/nvidia*.
 *
 *   nvioctl_trace [-o FILE] [-l DUMP_LEN] -- CMD [ARGS…]
 *
 * For each ioctl(fd, cmd, arg) where fd refers to /dev/nvidia*:
 *
 *   PID=… TID=… FD=… PATH=… CMD=0x… RM_CONTROL inner=0x… RET=… ERRNO=…
 *   PRE  : <hex bytes from *arg before the syscall>
 *   POST : <hex bytes from *arg after the syscall>
 *   DELTA: < '|' where bytes changed, '.' elsewhere >
 *
 * Designed to be diffed mechanically between a host run and a guest run to
 * identify which response payload differs in a CUDA cuInit failure.
 *
 * See docs/DIAGNOSIS_CUINIT.md.
 *
 * Build:  cc -O2 -g -Wall -Wextra -o nvioctl_trace nvioctl_trace.c
 * Run:    ./nvioctl_trace -o /tmp/host.log -- ./host_cu_test
 */

#define _GNU_SOURCE
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <inttypes.h>
#include <linux/elf.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ptrace.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <unistd.h>

/* ── Config ────────────────────────────────────────────────────────────────── */

static FILE   *g_log;
static size_t  g_dump_len = 256;

/* IOC encoding (x86-64): bits 0-7 NR, 8-15 TYPE, 16-29 SIZE, 30-31 DIR. */
static unsigned ioc_nr(unsigned long cmd)   { return cmd & 0xff; }
static unsigned ioc_size(unsigned long cmd) { return (cmd >> 16) & 0x3fff; }

/* ── Per-thread tracker for syscall-entry vs syscall-exit ──────────────────── */

/*
 * Linux delivers two syscall-stops per syscall: one on entry, one on exit.
 * We flip an "expecting-exit" bit per tid each time we see a stop with
 * orig_rax == SYS_ioctl. Hash keyed on tid.
 */
#define TIDTAB_BUCKETS 257

struct tid_state {
    pid_t           tid;
    bool            in_ioctl;       /* next stop on this tid is the exit */
    int             fd;
    unsigned long   cmd;
    unsigned long   arg;
    unsigned long   arg_size;       /* min(ioc_size, g_dump_len) */
    unsigned long   inner_cmd;      /* RM_CONTROL inner cmd at offset 8     */
    unsigned long   hclass;         /* RM_ALLOC hClass at offset 12         */
    unsigned char  *pre;
    bool            pre_valid;
    /* Inner params buffer snapshot (taken at syscall entry) */
    unsigned long   inner_pre_addr;
    unsigned long   inner_pre_size;
    unsigned char  *inner_pre;
    bool            inner_pre_valid;
    char            path[128];      /* /proc/pid/fd/N readlink result       */
    struct tid_state *next;
};

static struct tid_state *g_tidtab[TIDTAB_BUCKETS];

static struct tid_state *tid_get(pid_t tid)
{
    unsigned h = ((unsigned)tid) % TIDTAB_BUCKETS;
    for (struct tid_state *s = g_tidtab[h]; s; s = s->next)
        if (s->tid == tid) return s;
    struct tid_state *s = calloc(1, sizeof(*s));
    if (!s) return NULL;
    s->tid  = tid;
    s->fd   = -1;
    s->next = g_tidtab[h];
    g_tidtab[h] = s;
    return s;
}

static void tid_reset(struct tid_state *s)
{
    free(s->pre);
    free(s->inner_pre);
    s->pre        = NULL;
    s->inner_pre  = NULL;
    s->pre_valid  = false;
    s->inner_pre_valid = false;
    s->inner_pre_addr  = 0;
    s->inner_pre_size  = 0;
    s->in_ioctl   = false;
    s->fd         = -1;
    s->cmd        = 0;
    s->arg        = 0;
    s->arg_size   = 0;
    s->inner_cmd  = 0;
    s->hclass     = 0;
    s->path[0]    = 0;
}

/* ── Read tracee memory via /proc/PID/mem ──────────────────────────────────── */

static int peek_remote(pid_t pid, unsigned long addr, void *out, size_t len)
{
    char path[64];
    snprintf(path, sizeof(path), "/proc/%d/mem", pid);
    int fd = open(path, O_RDONLY);
    if (fd < 0) return -1;
    ssize_t n = pread(fd, out, len, (off_t)addr);
    close(fd);
    return (n < 0) ? -1 : (int)n;
}

/* ── /proc/PID/fd/N readlink to know if it's a nvidia device ───────────────── */

static int resolve_fd_path(pid_t pid, int fd, char *out, size_t outsz)
{
    char link[64];
    snprintf(link, sizeof(link), "/proc/%d/fd/%d", pid, fd);
    ssize_t n = readlink(link, out, outsz - 1);
    if (n < 0) { out[0] = 0; return -1; }
    out[n] = 0;
    return 0;
}

static bool is_nvidia_path(const char *path)
{
    return strstr(path, "/dev/nvidia") != NULL;
}

/* ── Hex dump helpers ──────────────────────────────────────────────────────── */

static void dump_hex(FILE *f, const unsigned char *buf, size_t len)
{
    for (size_t i = 0; i < len; i++) {
        fprintf(f, "%02x%s", buf[i], (i + 1 == len) ? "" : " ");
    }
}

static void dump_delta(FILE *f, const unsigned char *a,
                       const unsigned char *b, size_t len)
{
    for (size_t i = 0; i < len; i++) {
        fprintf(f, "%s%s", a[i] == b[i] ? ".." : "||", (i + 1 == len) ? "" : " ");
    }
}

/* ── Decode RM_CONTROL inner / RM_ALLOC hClass ─────────────────────────────── */

static void decode_rm_outer(const unsigned char *pre, size_t prelen,
                            unsigned long cmd,
                            unsigned long *inner_cmd_out,
                            unsigned long *hclass_out)
{
    *inner_cmd_out = 0;
    *hclass_out    = 0;
    if (!pre || prelen < 16) return;
    unsigned nr = ioc_nr(cmd);
    if (nr == 0x2a && prelen >= 12) {           /* NV_ESC_RM_CONTROL */
        uint32_t v;
        memcpy(&v, pre + 8, 4);
        *inner_cmd_out = v;
    } else if (nr == 0x2b && prelen >= 16) {    /* NV_ESC_RM_ALLOC */
        uint32_t v;
        memcpy(&v, pre + 12, 4);
        *hclass_out = v;
    }
}

/* ── Print one syscall record ──────────────────────────────────────────────── */

/* Extract the (inner params pointer, inner params size) for RM_CONTROL /
 * RM_ALLOC, given the outer buffer. */
static void inner_ptr_size(unsigned long cmd, const unsigned char *buf,
                           size_t buflen,
                           unsigned long *inner_ptr,
                           unsigned long *inner_size)
{
    *inner_ptr  = 0;
    *inner_size = 0;
    if (!buf) return;
    unsigned nr = ioc_nr(cmd);
    /* RM_CONTROL: nvos54.params at +16, nvos54.paramsSize at +24. */
    if (nr == 0x2a && buflen >= 28) {
        uint64_t p;
        uint32_t sz;
        memcpy(&p,  buf + 16, 8);
        memcpy(&sz, buf + 24, 4);
        *inner_ptr  = p;
        *inner_size = sz;
        return;
    }
    /* RM_ALLOC nvos21 (32B): pAllocParms at +16 (no explicit size — fixed
     * per class). nvos64 (48B): pAllocParms at +16, paramsSize at +32. */
    if (nr == 0x2b) {
        if (buflen == 32 && buflen >= 24) {
            uint64_t p;
            memcpy(&p, buf + 16, 8);
            *inner_ptr  = p;
            *inner_size = 0; /* caller can't size it without a class table */
            return;
        }
        if (buflen == 48) {
            uint64_t p;
            uint32_t sz;
            memcpy(&p,  buf + 16, 8);
            memcpy(&sz, buf + 32, 4);
            *inner_ptr  = p;
            *inner_size = sz;
            return;
        }
    }
}

static void emit_record(pid_t pid, pid_t tid, struct tid_state *s,
                        long retval, int saved_errno,
                        const unsigned char *post, size_t postlen)
{
    fprintf(g_log,
            "PID=%d TID=%d FD=%d PATH=%s CMD=0x%lx",
            pid, tid, s->fd, s->path, s->cmd);

    unsigned nr = ioc_nr(s->cmd);
    if (nr == 0x2a) {
        fprintf(g_log, " RM_CONTROL inner=0x%lx", s->inner_cmd);
    } else if (nr == 0x2b) {
        fprintf(g_log, " RM_ALLOC hClass=0x%lx", s->hclass);
    }
    fprintf(g_log, " RET=%ld ERRNO=%d SIZE=%u\n",
            retval, retval < 0 ? saved_errno : 0, ioc_size(s->cmd));

    size_t dump = s->arg_size;
    if (postlen < dump) dump = postlen;
    if (dump == 0) {
        fprintf(g_log, "  (no buffer)\n\n");
        fflush(g_log);
        return;
    }

    fprintf(g_log, "  PRE  : ");
    if (s->pre_valid) {
        dump_hex(g_log, s->pre, dump);
    } else {
        fprintf(g_log, "(read failed)");
    }
    fprintf(g_log, "\n");

    fprintf(g_log, "  POST : ");
    if (post) {
        dump_hex(g_log, post, dump);
    } else {
        fprintf(g_log, "(read failed)");
    }
    fprintf(g_log, "\n");

    if (s->pre_valid && post) {
        fprintf(g_log, "  DELTA: ");
        dump_delta(g_log, s->pre, post, dump);
        fprintf(g_log, "\n");
    }

    /* For RM_CONTROL / RM_ALLOC: follow the inner params pointer and dump
     * its pre/post bytes. That is where the actual driver-written response
     * lives — the outer struct only has handles, status, and a pointer. */
    if ((nr == 0x2a || nr == 0x2b) && post) {
        unsigned long ip_post = 0, isz_post = 0;
        inner_ptr_size(s->cmd, post, dump, &ip_post, &isz_post);
        unsigned long isz = s->inner_pre_size;
        if (isz_post > isz) isz = isz_post;
        if (isz == 0)      isz = 512;
        if (isz > g_dump_len) isz = g_dump_len;

        unsigned char *ibuf_post = NULL;
        if (ip_post) {
            ibuf_post = malloc(isz);
            if (ibuf_post &&
                peek_remote(tid, ip_post, ibuf_post, isz) != (int)isz) {
                free(ibuf_post);
                ibuf_post = NULL;
            }
        }

        if (s->inner_pre_valid || ibuf_post) {
            fprintf(g_log, "  INNER PRE  [%lu bytes @ 0x%lx]: ",
                    isz, s->inner_pre_addr);
            if (s->inner_pre_valid) {
                size_t n = s->inner_pre_size < isz ? s->inner_pre_size : isz;
                dump_hex(g_log, s->inner_pre, n);
            } else {
                fprintf(g_log, "(no pre)");
            }
            fprintf(g_log, "\n");
            fprintf(g_log, "  INNER POST [%lu bytes @ 0x%lx]: ",
                    isz, ip_post);
            if (ibuf_post) {
                dump_hex(g_log, ibuf_post, isz);
            } else {
                fprintf(g_log, "(read failed)");
            }
            fprintf(g_log, "\n");
            /* Delta on the common prefix */
            if (s->inner_pre_valid && ibuf_post) {
                size_t n = s->inner_pre_size < isz ? s->inner_pre_size : isz;
                fprintf(g_log, "  INNER DELTA: ");
                dump_delta(g_log, s->inner_pre, ibuf_post, n);
                fprintf(g_log, "\n");
            }
        }
        free(ibuf_post);
    }

    fprintf(g_log, "\n");
    fflush(g_log);
}

/* ── Per-stop handler ──────────────────────────────────────────────────────── */

static void inner_ptr_size(unsigned long cmd, const unsigned char *buf,
                           size_t buflen,
                           unsigned long *inner_ptr,
                           unsigned long *inner_size);

static int get_regs(pid_t tid, struct user_regs_struct *regs)
{
    struct iovec iov = { regs, sizeof(*regs) };
    return ptrace(PTRACE_GETREGSET, tid, (void *)NT_PRSTATUS, &iov);
}

static void handle_syscall_stop(pid_t tid)
{
    struct user_regs_struct regs;
    if (get_regs(tid, &regs) < 0) return;
    if (regs.orig_rax != SYS_ioctl) return;

    struct tid_state *s = tid_get(tid);
    if (!s) return;

    if (!s->in_ioctl) {
        /* Entry. */
        s->fd       = (int)regs.rdi;
        s->cmd      = regs.rsi;
        s->arg      = regs.rdx;
        s->in_ioctl = true;

        /* Resolve fd path. We need PID, not TID, for /proc but Linux's
         * /proc/<tid>/fd works the same. */
        if (resolve_fd_path(tid, s->fd, s->path, sizeof(s->path)) < 0)
            s->path[0] = 0;

        unsigned size = ioc_size(s->cmd);
        s->arg_size   = (size < g_dump_len) ? size : g_dump_len;
        s->pre_valid  = false;

        if (!is_nvidia_path(s->path) || s->arg_size == 0 || s->arg == 0)
            return;

        s->pre = realloc(s->pre, s->arg_size);
        if (s->pre && peek_remote(tid, s->arg, s->pre, s->arg_size) ==
                              (int)s->arg_size) {
            s->pre_valid = true;
            decode_rm_outer(s->pre, s->arg_size, s->cmd,
                            &s->inner_cmd, &s->hclass);

            /* For RM_CONTROL / RM_ALLOC, also snapshot the inner params
             * buffer the outer struct points to. */
            unsigned nr = ioc_nr(s->cmd);
            if (nr == 0x2a || nr == 0x2b) {
                unsigned long ip = 0, isz = 0;
                inner_ptr_size(s->cmd, s->pre, s->arg_size, &ip, &isz);
                if (isz == 0) isz = 512;
                if (isz > g_dump_len) isz = g_dump_len;
                if (ip) {
                    s->inner_pre_addr = ip;
                    s->inner_pre_size = isz;
                    s->inner_pre = realloc(s->inner_pre, isz);
                    if (s->inner_pre &&
                        peek_remote(tid, ip, s->inner_pre, isz) == (int)isz)
                        s->inner_pre_valid = true;
                }
            }
        }
        return;
    }

    /* Exit. */
    long retval     = (long)regs.rax;
    int  saved_err  = (retval < 0) ? -((int)retval) : 0;

    if (!is_nvidia_path(s->path)) {
        tid_reset(s);
        return;
    }

    unsigned char *post = NULL;
    size_t postlen = 0;
    if (s->arg && s->arg_size) {
        post = malloc(s->arg_size);
        if (post && peek_remote(tid, s->arg, post, s->arg_size) ==
                            (int)s->arg_size) {
            postlen = s->arg_size;
        } else {
            free(post);
            post = NULL;
        }
    }

    emit_record(getpid(), tid, s, retval, saved_err, post, postlen);

    free(post);
    tid_reset(s);
}

/* ── Main loop ─────────────────────────────────────────────────────────────── */

static int run(char **argv)
{
    pid_t child = fork();
    if (child < 0) { perror("fork"); return 1; }

    if (child == 0) {
        /* Tracee. Ask to be traced, then SIGSTOP so the tracer can attach
         * options before exec. */
        if (ptrace(PTRACE_TRACEME, 0, 0, 0) < 0) {
            perror("PTRACE_TRACEME");
            _exit(127);
        }
        raise(SIGSTOP);
        execvp(argv[0], argv);
        perror("execvp");
        _exit(127);
    }

    /* Wait for the initial SIGSTOP. */
    int status;
    if (waitpid(child, &status, 0) < 0) { perror("waitpid"); return 1; }

    if (ptrace(PTRACE_SETOPTIONS, child, 0,
               (void *)(uintptr_t)(PTRACE_O_TRACESYSGOOD |
                                   PTRACE_O_TRACECLONE   |
                                   PTRACE_O_TRACEFORK    |
                                   PTRACE_O_TRACEVFORK   |
                                   PTRACE_O_EXITKILL)) < 0) {
        perror("PTRACE_SETOPTIONS");
        return 1;
    }
    if (ptrace(PTRACE_SYSCALL, child, 0, 0) < 0) {
        perror("PTRACE_SYSCALL");
        return 1;
    }

    for (;;) {
        pid_t tid = waitpid(-1, &status, __WALL);
        if (tid < 0) {
            if (errno == ECHILD) break;
            if (errno == EINTR)  continue;
            perror("waitpid");
            break;
        }

        if (WIFEXITED(status) || WIFSIGNALED(status)) {
            /* drop tid state if any */
            unsigned h = ((unsigned)tid) % TIDTAB_BUCKETS;
            struct tid_state **pp = &g_tidtab[h];
            while (*pp) {
                if ((*pp)->tid == tid) {
                    struct tid_state *dead = *pp;
                    *pp = dead->next;
                    free(dead->pre);
                    free(dead);
                    break;
                }
                pp = &(*pp)->next;
            }
            (void)0; /* loop continues until ECHILD */
            continue;
        }

        if (!WIFSTOPPED(status)) continue;

        int sig = WSTOPSIG(status);
        int data_sig = 0;

        if (sig == (SIGTRAP | 0x80)) {
            handle_syscall_stop(tid);
            data_sig = 0;
        } else if (sig == SIGTRAP) {
            /* ptrace event (clone/fork/vfork). Look up event. */
            unsigned event = ((unsigned)status >> 16);
            if (event == PTRACE_EVENT_CLONE ||
                event == PTRACE_EVENT_FORK  ||
                event == PTRACE_EVENT_VFORK) {
                unsigned long new_tid = 0;
                ptrace(PTRACE_GETEVENTMSG, tid, 0, &new_tid);
                /* New thread will appear on its own waitpid; nothing to do
                 * here. */
            }
            data_sig = 0;
        } else if (sig == SIGSTOP) {
            /* Initial stop for a new clone'd thread — eat it. */
            data_sig = 0;
        } else {
            /* Regular signal — re-inject. */
            data_sig = sig;
        }

        if (ptrace(PTRACE_SYSCALL, tid, 0, (void *)(uintptr_t)data_sig) < 0) {
            if (errno == ESRCH) continue;
            perror("PTRACE_SYSCALL");
        }
    }

    return 0;
}

/* ── CLI ───────────────────────────────────────────────────────────────────── */

static void usage(const char *prog)
{
    fprintf(stderr,
            "Usage: %s [-o FILE] [-l DUMP_LEN] -- CMD [ARGS…]\n"
            "  -o FILE       output log file (default: stdout)\n"
            "  -l DUMP_LEN   max bytes to dump per ioctl arg (default 256)\n",
            prog);
}

int main(int argc, char **argv)
{
    const char *out_path = NULL;

    int opt;
    while ((opt = getopt(argc, argv, "+o:l:h")) != -1) {
        switch (opt) {
        case 'o': out_path = optarg; break;
        case 'l': g_dump_len = (size_t)atoi(optarg); break;
        case 'h':
        default:  usage(argv[0]); return opt == 'h' ? 0 : 1;
        }
    }
    if (optind >= argc) { usage(argv[0]); return 1; }

    if (out_path) {
        g_log = fopen(out_path, "w");
        if (!g_log) { perror(out_path); return 1; }
    } else {
        g_log = stdout;
    }

    int rc = run(argv + optind);

    if (g_log != stdout) fclose(g_log);
    return rc;
}
