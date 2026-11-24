/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * kf_gldual_logic.h -- the pure part of kf_gldual.c (argument parsing, expected values, result line). No Windows or GL include, so the CPU-only
 * test (tests/test_gldual_logic.py) compiles it with the host compiler. STATUS: DESIGN-ONLY, 2026-11-17; never ran on a Windows guest.
 */
#ifndef KF_GLDUAL_LOGIC_H
#define KF_GLDUAL_LOGIC_H
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define KF_COMPUTE_N 65536u          /* uints written by the compute shader per iteration */
#define KF_FBO_SIZE 128              /* edge of the FBO a patch is drawn into */

typedef struct kf_cfg {
    int seconds;                      /* soak length; default 600 (the Resolve crash came 3 to 4 minutes after load) */
    int share;                        /* 1: the second context shares objects with the first */
    int any_renderer;                 /* 1: do not require GL_RENDERER to name NVIDIA */
    int bad_args;
} kf_cfg;

static void kf_parse_args(int argc, char **argv, kf_cfg *c) {
    int i;
    c->seconds = 600; c->share = 0; c->any_renderer = 0; c->bad_args = 0;
    for (i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--seconds") && i + 1 < argc) {
            c->seconds = atoi(argv[++i]);
            if (c->seconds < 1 || c->seconds > 86400) c->bad_args = 1;
        } else if (!strcmp(argv[i], "--share")) c->share = 1;
        else if (!strcmp(argv[i], "--any-renderer")) c->any_renderer = 1;
        else c->bad_args = 1;
    }
}

/* sum over i < n of (i * 2654435761 + seed), modulo 2^32: what the compute shader's buffer must add up to */
static uint32_t kf_expected_sum(uint32_t n, uint32_t seed) {
    uint64_t tri = ((uint64_t)n * (uint64_t)(n - 1u)) / 2u;
    return (uint32_t)(2654435761u * (uint32_t)tri + seed * n);
}

/* the flat colour channel a patch draw must produce at the FBO centre in iteration `iter` for thread `tid` (0 or 1) */
static unsigned kf_channel(unsigned iter, unsigned tid, unsigned ch) {
    return (iter * (7u + 2u * tid) + 31u * ch + 17u * tid) & 255u;
}

typedef struct kf_stats {
    unsigned long iters;
    unsigned long bad;                /* wrong checksum, wrong pixel or an unexpected glGetError */
    char first_error[160];
} kf_stats;

static void kf_note_bad(kf_stats *s, const char *what, unsigned long iter) {
    if (!s->bad) snprintf(s->first_error, sizeof s->first_error, "%s at iteration %lu", what, iter);
    s->bad++;
}

/* the last line of the program: `KFGL RESULT OK|FAIL <why>`, the format of kf_glgears.c */
static int kf_result(const kf_stats *a, const kf_stats *b, int context_failed, char *line, size_t n) {
    if (context_failed) { snprintf(line, n, "KFGL RESULT FAIL context-setup-failed"); return 1; }
    if (a->iters == 0 || b->iters == 0) { snprintf(line, n, "KFGL RESULT FAIL a-thread-did-no-iterations a=%lu b=%lu", a->iters, b->iters); return 1; }
    if (a->bad || b->bad) { snprintf(line, n, "KFGL RESULT FAIL bad=%lu/%lu first=%s", a->bad, b->bad, a->bad ? a->first_error : b->first_error); return 1; }
    snprintf(line, n, "KFGL RESULT OK iters=%lu/%lu", a->iters, b->iters);
    return 0;
}
#endif
