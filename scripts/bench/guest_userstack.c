/* ★★★★★ §16.78 — WHAT IS THE HUNG PROCESS ACTUALLY SPINNING ON?
 *
 * ## ⊘⊘ WHY `/proc/<pid>/stack` IS THE WRONG INSTRUMENT, MEASURED
 *
 * `scripts/bench/guest_cuinit_wall.sh` samples `/proc/<pid>/stack` when its deadline fires,
 * and its own header says why: *"when `cuInit` HANGS it spins in `uvm_spin_loop` — a busy
 * spin in the kernel that takes no signal"*.  `[measured 2026-08-10, boot `w214_9b65664_ctl`,
 * committed at `traces/guest_boots/run_w214_9b65664_ctl_probe.log`]` that premise is FALSE
 * for the wall this campaign is actually standing at.  The three samples that boot printed
 * were:
 *
 *     pid=1811 state=Rl
 *     --- stack sample 1 ---            (empty)
 *     --- stack sample 2 ---            [<0>] ktime_get_raw_ts64+0x41/0xd0
 *     --- stack sample 3 ---            [<0>] __x64_sys_clock_gettime+0xb4/0x110
 *
 * `state=R` and a kernel stack that is nothing but `clock_gettime` is a **userspace** spin:
 * the thread is in `libcuda`, not in `uvm_spin_loop`, and `/proc/<pid>/stack` can only ever
 * show the two instructions of whatever `vDSO`-adjacent syscall it happened to be inside.
 * ⊘ The instrument RAN, printed a POSITIVE result, and that result is about the wrong plane
 * — the campaign's own `a_correct_capture_can_answer_the_wrong_question` shape.
 *
 * ## ★ What this program does instead
 *
 * `PTRACE_SEIZE` + `PTRACE_INTERRUPT` every thread of the target, read `RIP`/`RSP` with
 * `PTRACE_GETREGSET`, and resolve both through the target's own `/proc/<pid>/maps` so the
 * answer is `libcuda.so.1+0x…` and not a bare number.  Then a conservative stack scan: walk
 * `[RSP, RSP+SCAN_WORDS*8)` and print every word that lands inside an executable mapping.
 * ⊘ That is a SUPERSET of the call chain, not the call chain — dead frames survive on the
 * stack and are indistinguishable from live ones.  It is labelled `CANDIDATE` in the output
 * for exactly that reason; a reader who takes it for a backtrace is reading a claim this
 * program does not make.
 *
 * ★ Sampled `N` times with a gap, because the question is *which loop*, and one sample of a
 * spin cannot tell a loop from a coincidence.  If the same `RIP±` region recurs across
 * samples, that is the loop.
 *
 * ## ⊘ What it deliberately does NOT do
 *
 * No symbolisation beyond module+offset.  `libcuda` exports only the CUDA API, so a
 * "nearest preceding dynamic symbol" would name whichever `cu*` entry point happens to sit
 * below an internal function and would read as a much stronger claim than it is.  The
 * offset is exact and joins to `nm -D`/`objdump` offline; a wrong name does not.
 *
 * usage: guest_userstack <pid> [samples] [gap_ms]
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ptrace.h>
#include <sys/uio.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define MAX_MAPS 512
#define MAX_TIDS 64
#define SCAN_WORDS 256
#define MAX_CANDIDATES 24

struct maprow {
    uint64_t lo, hi;
    int exec;
    char name[256];
};

static struct maprow g_maps[MAX_MAPS];
static int g_nmaps;

static void load_maps(pid_t pid)
{
    char path[64];
    char line[512];
    FILE *f;

    g_nmaps = 0;
    snprintf(path, sizeof(path), "/proc/%d/maps", (int)pid);
    f = fopen(path, "r");
    if (!f) {
        fprintf(stderr, "★ cannot open %s: %s\n", path, strerror(errno));
        return;
    }
    while (fgets(line, sizeof(line), f) && g_nmaps < MAX_MAPS) {
        unsigned long lo, hi;
        char perms[8];
        char name[256];
        int n;

        name[0] = '\0';
        n = sscanf(line, "%lx-%lx %7s %*s %*s %*s %255[^\n]", &lo, &hi, perms, name);
        if (n < 3) {
            continue;
        }
        g_maps[g_nmaps].lo = lo;
        g_maps[g_nmaps].hi = hi;
        g_maps[g_nmaps].exec = (perms[2] == 'x');
        /* Strip the leading run of spaces sscanf's %[^\n] keeps. */
        {
            const char *p = name;
            while (*p == ' ') {
                p++;
            }
            snprintf(g_maps[g_nmaps].name, sizeof(g_maps[g_nmaps].name), "%s",
                     *p ? p : "[anon]");
        }
        g_nmaps++;
    }
    fclose(f);
}

/* Resolve `a` to "module+offset", or "?" when no mapping owns it. */
static const char *resolve(uint64_t a, char *buf, size_t len, int *is_exec)
{
    int i;

    if (is_exec) {
        *is_exec = 0;
    }
    for (i = 0; i < g_nmaps; i++) {
        if (a >= g_maps[i].lo && a < g_maps[i].hi) {
            if (is_exec) {
                *is_exec = g_maps[i].exec;
            }
            snprintf(buf, len, "%s+0x%llx", g_maps[i].name,
                     (unsigned long long)(a - g_maps[i].lo));
            return buf;
        }
    }
    snprintf(buf, len, "?");
    return buf;
}

static int list_tids(pid_t pid, pid_t *out, int max)
{
    char path[64];
    DIR *d;
    struct dirent *e;
    int n = 0;

    snprintf(path, sizeof(path), "/proc/%d/task", (int)pid);
    d = opendir(path);
    if (!d) {
        fprintf(stderr, "★ cannot open %s: %s\n", path, strerror(errno));
        return 0;
    }
    while ((e = readdir(d)) && n < max) {
        if (e->d_name[0] >= '0' && e->d_name[0] <= '9') {
            out[n++] = (pid_t)atoi(e->d_name);
        }
    }
    closedir(d);
    return n;
}

/* One thread, one sample. Returns 0 on success. */
static int sample_tid(pid_t tid, pid_t pid, int show_stack)
{
    struct user_regs_struct regs;
    struct iovec iov;
    char sym[224];
    char path[64];
    FILE *mem;
    int status;
    int i, printed = 0;

    if (ptrace(PTRACE_SEIZE, tid, NULL, 0) == -1) {
        printf("    tid %d: PTRACE_SEIZE failed: %s\n", (int)tid, strerror(errno));
        return -1;
    }
    if (ptrace(PTRACE_INTERRUPT, tid, NULL, NULL) == -1) {
        printf("    tid %d: PTRACE_INTERRUPT failed: %s\n", (int)tid, strerror(errno));
        ptrace(PTRACE_DETACH, tid, NULL, 0);
        return -1;
    }
    if (waitpid(tid, &status, __WALL) == -1) {
        printf("    tid %d: waitpid failed: %s\n", (int)tid, strerror(errno));
        ptrace(PTRACE_DETACH, tid, NULL, 0);
        return -1;
    }

    iov.iov_base = &regs;
    iov.iov_len = sizeof(regs);
    if (ptrace(PTRACE_GETREGSET, tid, (void *)1 /* NT_PRSTATUS */, &iov) == -1) {
        printf("    tid %d: PTRACE_GETREGSET failed: %s\n", (int)tid, strerror(errno));
        ptrace(PTRACE_DETACH, tid, NULL, 0);
        return -1;
    }

    printf("    tid %d  RIP=0x%llx %s\n", (int)tid, (unsigned long long)regs.rip,
           resolve((uint64_t)regs.rip, sym, sizeof(sym), NULL));
    printf("    tid %d  RSP=0x%llx  orig_rax=%lld  rax=%lld\n", (int)tid,
           (unsigned long long)regs.rsp, (long long)regs.orig_rax, (long long)regs.rax);

    if (show_stack) {
        snprintf(path, sizeof(path), "/proc/%d/mem", (int)pid);
        mem = fopen(path, "rb");
        if (mem && fseeko(mem, (off_t)regs.rsp, SEEK_SET) == 0) {
            for (i = 0; i < SCAN_WORDS && printed < MAX_CANDIDATES; i++) {
                uint64_t w;
                int isx = 0;

                if (fread(&w, sizeof(w), 1, mem) != 1) {
                    break;
                }
                resolve(w, sym, sizeof(sym), &isx);
                if (isx) {
                    printf("      CANDIDATE [rsp+0x%03x] 0x%llx  %s\n", i * 8,
                           (unsigned long long)w, sym);
                    printed++;
                }
            }
        } else {
            printf("      (no /proc/%d/mem: %s)\n", (int)pid, strerror(errno));
        }
        if (mem) {
            fclose(mem);
        }
    }

    ptrace(PTRACE_DETACH, tid, NULL, 0);
    return 0;
}

int main(int argc, char **argv)
{
    pid_t pid;
    pid_t tids[MAX_TIDS];
    int ntids, s, i;
    int samples = 3;
    long gap_ms = 300;

    if (argc < 2) {
        fprintf(stderr, "usage: %s <pid> [samples] [gap_ms]\n", argv[0]);
        return 2;
    }
    pid = (pid_t)atoi(argv[1]);
    if (argc > 2) {
        samples = atoi(argv[2]);
    }
    if (argc > 3) {
        gap_ms = atol(argv[3]);
    }
    if (pid <= 0) {
        fprintf(stderr, "★ not a pid: %s\n", argv[1]);
        return 2;
    }

    load_maps(pid);
    printf("=== guest_userstack pid=%d maps=%d samples=%d gap=%ldms ===\n", (int)pid,
           g_nmaps, samples, gap_ms);
    if (g_nmaps == 0) {
        fprintf(stderr, "★ no maps read — the target is gone or unreadable; ⊘ every "
                        "address below would be unresolvable, so nothing is printed\n");
        return 1;
    }

    /* ★ The executable mappings, once, so every offset below can be joined offline. */
    printf("--- executable mappings ---\n");
    for (i = 0; i < g_nmaps; i++) {
        if (g_maps[i].exec) {
            printf("    0x%llx-0x%llx  %s\n", (unsigned long long)g_maps[i].lo,
                   (unsigned long long)g_maps[i].hi, g_maps[i].name);
        }
    }

    ntids = list_tids(pid, tids, MAX_TIDS);
    printf("--- %d thread(s) ---\n", ntids);

    for (s = 0; s < samples; s++) {
        struct timespec ts;

        printf("--- sample %d/%d ---\n", s + 1, samples);
        for (i = 0; i < ntids; i++) {
            /* ⊘ The candidate scan only on the FIRST sample: it is ~24 lines per thread
             * and the later samples are asked a different question (does RIP recur?). */
            sample_tid(tids[i], pid, s == 0);
        }
        if (s + 1 < samples) {
            ts.tv_sec = gap_ms / 1000;
            ts.tv_nsec = (gap_ms % 1000) * 1000000L;
            nanosleep(&ts, NULL);
        }
    }
    return 0;
}
