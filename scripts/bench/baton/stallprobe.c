/*
 * stallprobe.c - guest-side controller for the H-ABANDON experiment (see README.md).
 *
 * Watches the baton heartbeat files (--hb-dir, one mmap'd Hb per client).  When a non-ignored client V
 * has completed nothing for --detect-ms while another non-ignored client is alive, it
 *   step 1: SIGSTOPs every other non-ignored client (the GPU is then free, the host worker sees only
 *           the ignored keep-alive traffic) and waits --hold-ms for V to complete an iteration;
 *   step 2: if V is still stuck, SIGSTOPs the ignored clients too (the host worker gets silence) and
 *           waits --hold2-ms;
 * then SIGCONTs everything.  Every transition is logged with CLOCK_MONOTONIC milliseconds.
 *
 *   gcc -O2 -o stallprobe stallprobe.c
 *   stallprobe --hb-dir D --pid A=123 --pid B=124 --pid C=125 --ignore C --detect-ms 2500 \
 *              --hold-ms 1500 --hold2-ms 500 --duration 120 --out FILE
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

typedef struct { volatile double last_real; volatile unsigned long long iters; } Hb;
#define MAXC 8
static struct { char label[16]; pid_t pid; int ignore; int fd; } C[MAXC];
static int nc;

static double mono(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e3 + t.tv_nsec * 1e-6; }
static double real(void) { struct timespec t; clock_gettime(CLOCK_REALTIME, &t); return t.tv_sec + t.tv_nsec * 1e-9; }

static int read_hb(int i, Hb *h) {
    if (C[i].fd < 0) {
        char p[512];
        snprintf(p, sizeof p, "%s/hb.%s", getenv("HBDIR"), C[i].label);
        C[i].fd = open(p, O_RDONLY);
        if (C[i].fd < 0) return 0;
    }
    return pread(C[i].fd, h, sizeof *h, 0) == (ssize_t)sizeof *h;
}

int main(int argc, char **argv) {
    double detect = 2500, hold = 1500, hold2 = 500, duration = 120;
    const char *out = NULL;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--hb-dir") && i + 1 < argc) setenv("HBDIR", argv[++i], 1);
        else if (!strcmp(argv[i], "--pid") && i + 1 < argc && nc < MAXC) {
            char *a = argv[++i], *eq = strchr(a, '=');
            if (!eq) return 2;
            *eq = 0; snprintf(C[nc].label, sizeof C[nc].label, "%s", a); C[nc].pid = atoi(eq + 1); C[nc].fd = -1; nc++;
        } else if (!strcmp(argv[i], "--ignore") && i + 1 < argc) {
            for (int k = 0; k < nc; k++) if (!strcmp(C[k].label, argv[i + 1])) C[k].ignore = 1;
            i++;
        } else if (!strcmp(argv[i], "--detect-ms") && i + 1 < argc) detect = atof(argv[++i]);
        else if (!strcmp(argv[i], "--hold-ms") && i + 1 < argc) hold = atof(argv[++i]);
        else if (!strcmp(argv[i], "--hold2-ms") && i + 1 < argc) hold2 = atof(argv[++i]);
        else if (!strcmp(argv[i], "--duration") && i + 1 < argc) duration = atof(argv[++i]);
        else if (!strcmp(argv[i], "--out") && i + 1 < argc) out = argv[++i];
        else return 2;
    }
    FILE *o = out ? fopen(out, "w") : stdout;
    if (!o || !getenv("HBDIR")) return 2;
    setvbuf(o, NULL, _IOLBF, 0);
    double t0 = mono();
    fprintf(o, "PROBE_BEGIN clients=%d detect_ms=%.0f hold_ms=%.0f hold2_ms=%.0f\n", nc, detect, hold, hold2);
    while (mono() - t0 < duration * 1e3 + 15000) {
        usleep(5000);
        double now = real();
        for (int v = 0; v < nc; v++) {
            if (C[v].ignore) continue;
            Hb hv;
            if (!read_hb(v, &hv) || hv.iters == 0) continue;
            double age = (now - hv.last_real) * 1e3;
            if (age < detect) continue;
            int alive = 0;
            for (int k = 0; k < nc; k++) {
                Hb hk;
                if (k != v && !C[k].ignore && read_hb(k, &hk) && hk.iters > 0 && (now - hk.last_real) * 1e3 < 300) alive = 1;
            }
            if (!alive) continue;
            unsigned long long it0 = hv.iters;
            double t = mono();
            fprintf(o, "STALL t=%.0f victim=%s age_ms=%.0f iters=%llu\n", t - t0, C[v].label, age, it0);
            for (int k = 0; k < nc; k++) if (k != v && !C[k].ignore) kill(C[k].pid, SIGSTOP);
            fprintf(o, "STEP1_STOPPED others (non-ignored) t=%.0f\n", mono() - t0);
            double s = mono(); int resumed = 0;
            while (mono() - s < hold) { usleep(2000); if (read_hb(v, &hv) && hv.iters != it0) { resumed = 1; break; } }
            if (resumed) fprintf(o, "RESUMED_STEP1 after_ms=%.0f\n", mono() - s);
            else {
                fprintf(o, "NOT_RESUMED_STEP1 waited_ms=%.0f\n", mono() - s);
                for (int k = 0; k < nc; k++) if (k != v && C[k].ignore) kill(C[k].pid, SIGSTOP);
                fprintf(o, "STEP2_STOPPED ignored t=%.0f\n", mono() - t0);
                s = mono();
                while (mono() - s < hold2) { usleep(2000); if (read_hb(v, &hv) && hv.iters != it0) { resumed = 1; break; } }
                fprintf(o, resumed ? "RESUMED_STEP2 after_ms=%.0f\n" : "NOT_RESUMED_STEP2 waited_ms=%.0f\n", mono() - s);
            }
            for (int k = 0; k < nc; k++) if (k != v) kill(C[k].pid, SIGCONT);
            fprintf(o, "CONT t=%.0f\n", mono() - t0);
            usleep(500000); /* let everything settle before judging again */
            break;
        }
    }
    fprintf(o, "PROBE_END\n");
    return 0;
}
