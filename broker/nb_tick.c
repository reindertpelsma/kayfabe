/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nb_tick.c -- the display tick's DECISIONS, and nothing else.
 *
 * No I/O, no clock, no backend: every function takes the time it should judge
 * by.  That is what lets test/test_tick.c pin each rule to the nanosecond and
 * test/mutate_tick.py break each one and watch it get caught.  The core
 * (nvkvm_broker.c) owns the wire; the backends only report "the display
 * refreshed" and "the display showed the frame".
 *
 * The rules (docs/design/V3_DISPLAY_PACING_20261017.md section 11):
 *
 *   - one tick per ACCEPTED, DECIMATED display feedback; a duplicate feedback,
 *     or one closer than half a host period to the last accepted, is not a
 *     refresh and ticks nothing (a backlog never emits twice);
 *   - WANT_FRAME opens a request, and a request opens only when none is open
 *     and the frame answered before it has been shown -- or both have waited
 *     NVKVM_BROKER_TICK_REQ_TIMEOUT_NS;
 *   - a reply is judged against what was ISSUED, not against what is open.
 */
#include <stdio.h>
#include <string.h>

#include "nvkvm_broker.h"

uint32_t nb_tick_next_seq(uint32_t seq)
{
    seq++;
    return seq == 0 ? 1u : seq;
}

bool nb_tick_seq_newer(uint32_t a, uint32_t b)
{
    return (int32_t)(a - b) > 0;
}

void nb_tick_reset(struct nb_tick *t, bool on, unsigned cap_hz)
{
    memset(t, 0, sizeof(*t));
    t->on = on;
    t->cap_hz = cap_hz;
    t->div = 1;
}

static unsigned tick_cap(unsigned cap_hz)
{
    if (cap_hz == 0 || cap_hz > NVKVM_BROKER_GUEST_MAX_HZ) {
        return NVKVM_BROKER_GUEST_MAX_HZ;
    }
    if (cap_hz < NVKVM_BROKER_GUEST_MIN_HZ) {
        return NVKVM_BROKER_GUEST_MIN_HZ;
    }
    return cap_hz;
}

/* The host rate in millihertz from its nominal period, rounded; 0 = unknown. */
static uint64_t host_mhz_of(uint64_t period_ns)
{
    if (period_ns == 0) {
        return 0;
    }
    return (1000000000000ull + period_ns / 2) / period_ns;
}

unsigned nb_tick_div(uint64_t host_period_ns, unsigned cap_hz, int hz_mode,
                     unsigned fixed_mhz)
{
    uint64_t host = host_mhz_of(host_period_ns);
    uint64_t mode, d;

    if (host == 0) {
        return 1;
    }
    switch (hz_mode) {
    case NB_HZ_FIXED:
        mode = fixed_mhz ? fixed_mhz : 60000u;
        d = (host + mode / 2) / mode;
        break;
    case NB_HZ_NONE:
        d = (host + 30000u) / 60000u;
        break;
    default: {
        uint64_t lim = (uint64_t)tick_cap(cap_hz) * 1000u + NB_TICK_HZ_SLACK_MHZ;

        d = (host + lim - 1u) / lim;
        break;
    }
    }
    if (d < 1) {
        d = 1;
    }
    if (d > 64) {
        d = 64;     /* a rate this far off the cap is a bug upstream; bound it */
    }
    return (unsigned)d;
}

unsigned nb_tick_mode_mhz(unsigned host_mhz, unsigned div)
{
    return div > 1 ? host_mhz / div : host_mhz;
}

bool nb_tick_want(const struct nb_tick *t, uint64_t now_ns)
{
    if (t->unpresented &&
        now_ns - t->unpresented_since_ns < NVKVM_BROKER_TICK_REQ_TIMEOUT_NS) {
        return false;
    }
    if (t->open_req == 0) {
        return true;
    }
    return now_ns - t->open_since_ns >= NVKVM_BROKER_TICK_REQ_TIMEOUT_NS;
}

static void tick_emit(struct nb_tick *t, uint64_t now_ns, uint64_t present_ns,
                      uint64_t period_ns, uint32_t flags, bool want,
                      struct nb_tick_out *o)
{
    uint32_t word = 0;

    t->last_seq = nb_tick_next_seq(t->last_seq);
    if (want) {
        /* a request that timed out is replaced, not stacked */
        if (t->open_req != 0) {
            t->n_req_timeouts++;
        }
        if (t->unpresented) {
            t->n_present_timeouts++;
            t->unpresented = false;
        }
        t->open_req = t->last_seq;
        t->open_since_ns = now_ns;
        t->n_want++;
    }
    if (period_ns >= NVKVM_BROKER_TICK_REFRESH_MIN_NS &&
        period_ns <= NVKVM_BROKER_TICK_REFRESH_MASK) {
        word = (uint32_t)period_ns;
    }
    word |= flags & NVKVM_BROKER_TICK_F_CLOCK_ONLY;
    if (want) {
        word |= NVKVM_BROKER_TICK_F_WANT_FRAME;
    }
    if (present_ns == 0) {
        present_ns = now_ns;
    }
    if (present_ns < t->last_present_ns) {
        present_ns = t->last_present_ns;        /* never decreasing */
    }
    t->last_present_ns = present_ns;
    t->n_ticks++;
    o->tick_seq = t->last_seq;
    o->refresh_word = word;
    o->present_ns = present_ns;
}

bool nb_tick_refresh(struct nb_tick *t, uint64_t now_ns, uint64_t present_ns,
                     uint64_t host_period_ns, uint32_t flags, int hz_mode,
                     unsigned fixed_mhz, struct nb_tick_out *o)
{
    unsigned div;

    if (!t->on) {
        return false;
    }
    t->now_ns = now_ns;
    t->n_feedback++;
    /* a duplicate, a reordering, or faster than the display can refresh */
    if (t->have_accept &&
        (now_ns <= t->last_accept_ns ||
         now_ns - t->last_accept_ns < host_period_ns / 2)) {
        t->n_fast++;
        return false;
    }
    /* the compositor's own timestamps are exact vsync instants: two closer
     * than half a period are the same refresh (or a flood) however late the
     * events were dispatched */
    if (present_ns != 0 && !(flags & NVKVM_BROKER_TICK_F_CLOCK_ONLY)) {
        if (t->have_real && (present_ns <= t->last_real_ns ||
                             present_ns - t->last_real_ns < host_period_ns / 2)) {
            t->n_fast++;
            return false;
        }
        t->have_real = true;
        t->last_real_ns = present_ns;
    }
    /* ★ MEASUREMENT: the spacing of ACCEPTED refreshes by arrival time - one clock domain (the pump's ticks carry the broker's own
     * clock, a frame's carry the compositor's, so present_ns deltas across the two are not a spacing) */
    if (t->have_accept) {
        nb_tick_win_gap(t, now_ns - t->last_accept_ns);
    }
    t->have_accept = true;
    t->last_accept_ns = now_ns;
    div = nb_tick_div(host_period_ns, t->cap_hz, hz_mode, fixed_mhz);
    if (div != t->div) {
        t->div = div;
        t->phase = div - 1u;        /* a changed rate restarts the count: the
                                     * next accepted feedback ticks */
    }
    t->phase = (t->phase + 1u) % t->div;
    if (t->phase != 0) {
        t->n_decimated++;
        return false;
    }
    tick_emit(t, now_ns, present_ns, host_period_ns * t->div, flags,
              nb_tick_want(t, now_ns), o);
    return true;
}

void nb_tick_prime(struct nb_tick *t, uint64_t now_ns, struct nb_tick_out *o)
{
    t->now_ns = now_ns;
    tick_emit(t, now_ns, now_ns, 0, NVKVM_BROKER_TICK_F_CLOCK_ONLY, true, o);
}

enum nb_tick_ans nb_tick_answer(struct nb_tick *t, uint32_t req, bool is_commit,
                                uint64_t now_ns)
{
    t->now_ns = now_ns;
    if (req == 0) {
        if (is_commit) {
            t->n_unsolicited++;
            return NB_TICK_ANS_UNSOLICITED;
        }
        return NB_TICK_ANS_VIOLATION;
    }
    /* an id the broker never issued: newer than the last tick, or no tick yet */
    if (t->last_seq == 0 || nb_tick_seq_newer(req, t->last_seq)) {
        return NB_TICK_ANS_VIOLATION;
    }
    if (t->open_req != 0 && req == t->open_req) {
        if (now_ns >= t->open_since_ns) {
            uint64_t l = now_ns - t->open_since_ns;

            t->win.ans_n++;
            t->win.ans_sum_ns += l;
            if (l > t->win.ans_max_ns) {
                t->win.ans_max_ns = l;
            }
        }
        t->open_req = 0;
        t->n_answered++;
        if (is_commit) {
            t->commits_since_present++;
            t->unpresented = true;
            t->unpresented_since_ns = now_ns;
        } else {
            t->n_no_change++;
        }
        return NB_TICK_ANS_OPEN;
    }
    t->n_late++;
    return NB_TICK_ANS_LATE;
}

void nb_tick_presented(struct nb_tick *t)
{
    t->unpresented = false;
}

bool nb_tick_may_replace(uint32_t queued_refresh_word, uint32_t new_refresh_word)
{
    return (queued_refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME) == 0 ||
           (new_refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME) != 0;
}

/* ── ★ MEASUREMENT: the per-second stats line (pure) ───────────────────────── */

void nb_tick_snap_take(const struct nb_tick *t, struct nb_tick_snap *s)
{
    s->feedback = t->n_feedback;
    s->ticks = t->n_ticks;
    s->want = t->n_want;
    s->answered = t->n_answered;
    s->no_change = t->n_no_change;
    s->req_timeouts = t->n_req_timeouts;
    s->late = t->n_late;
    s->unsolicited = t->n_unsolicited;
    s->present_timeouts = t->n_present_timeouts;
    s->coalesced = t->n_coalesced;
}

bool nb_tick_stats_due(uint64_t now_ns, uint64_t *next_ns, uint64_t period_ns)
{
    if (*next_ns == 0) {
        *next_ns = now_ns + period_ns;      /* armed: the first line is one period away */
        return false;
    }
    if (now_ns < *next_ns) {
        return false;
    }
    /* one line per call; after a gap the next one is a period from now, never a burst of catch-up lines */
    *next_ns = now_ns + period_ns;
    return true;
}

static uint64_t delta_u64(uint64_t now, uint64_t prev)
{
    return now >= prev ? now - prev : 0;
}

int nb_tick_stats_format(char *buf, size_t n, uint64_t t_ms, const struct nb_tick_snap *prev,
                         const struct nb_tick_snap *now, const struct nb_tick_win *w)
{
    uint64_t dans = delta_u64(now->answered, prev->answered);
    uint64_t dnc = delta_u64(now->no_change, prev->no_change);
    uint64_t commit = dans - (dnc <= dans ? dnc : dans);
    int k = snprintf(buf, n,
                     "stats t=%llu feedbacks=%llu ticks=%llu want_ticks=%llu commit=%llu no_change=%llu timeouts=%llu "
                     "late=%llu unsolicited=%llu present_timeouts=%llu coalesced=%llu "
                     "gap_max_us=%llu gaps33=%llu discarded=%llu ans_max_us=%llu ans_mean_us=%llu "
                     "pres_max_us=%llu pres_mean_us=%llu loop_late_max_us=%llu loop_work_max_us=%llu "
                     "gaps50=%llu gaps100=%llu new=%llu repeat=%llu merged=%llu cgap_max_us=%llu cg33=%llu cg50=%llu cg100=%llu",
                     (unsigned long long)t_ms,
                     (unsigned long long)delta_u64(now->feedback, prev->feedback),
                     (unsigned long long)delta_u64(now->ticks, prev->ticks),
                     (unsigned long long)delta_u64(now->want, prev->want),
                     (unsigned long long)commit,
                     (unsigned long long)dnc,
                     (unsigned long long)delta_u64(now->req_timeouts, prev->req_timeouts),
                     (unsigned long long)delta_u64(now->late, prev->late),
                     (unsigned long long)delta_u64(now->unsolicited, prev->unsolicited),
                     (unsigned long long)delta_u64(now->present_timeouts, prev->present_timeouts),
                     (unsigned long long)delta_u64(now->coalesced, prev->coalesced),
                     (unsigned long long)(w->gap_max_ns / 1000), (unsigned long long)w->gap33,
                     (unsigned long long)w->discarded,
                     (unsigned long long)(w->ans_max_ns / 1000),
                     (unsigned long long)(w->ans_n ? w->ans_sum_ns / w->ans_n / 1000 : 0),
                     (unsigned long long)(w->pres_max_ns / 1000),
                     (unsigned long long)(w->pres_n ? w->pres_sum_ns / w->pres_n / 1000 : 0),
                     (unsigned long long)(w->loop_late_max_ns / 1000),
                     (unsigned long long)(w->loop_work_max_ns / 1000),
                     (unsigned long long)w->gaps50, (unsigned long long)w->gaps100,
                     (unsigned long long)w->new_n, (unsigned long long)w->repeat_n, (unsigned long long)w->merged,
                     (unsigned long long)(w->cgap_max_ns / 1000), (unsigned long long)w->cg33,
                     (unsigned long long)w->cg50, (unsigned long long)w->cg100);
    return (k > 0 && (size_t)k < n) ? k : 0;
}

void nb_tick_presented_at(struct nb_tick *t, uint64_t now_ns, bool discarded)
{
    if (t->unpresented && now_ns >= t->unpresented_since_ns) {
        uint64_t l = now_ns - t->unpresented_since_ns;

        t->win.pres_n++;
        t->win.pres_sum_ns += l;
        if (l > t->win.pres_max_ns) {
            t->win.pres_max_ns = l;
        }
    }
    if (discarded) {
        t->win.discarded++;
    }
    nb_tick_presented(t);
}

void nb_tick_win_gap(struct nb_tick *t, uint64_t gap_ns)
{
    if (gap_ns > t->win.gap_max_ns) {
        t->win.gap_max_ns = gap_ns;
    }
    if (gap_ns > 33000000ull) {
        t->win.gap33++;
    }
    if (gap_ns > 50000000ull) {
        t->win.gaps50++;
    }
    if (gap_ns > 100000000ull) {
        t->win.gaps100++;
    }
}

void nb_tick_win_loop(struct nb_tick *t, uint64_t late_ns, uint64_t work_ns)
{
    if (late_ns > t->win.loop_late_max_ns) {
        t->win.loop_late_max_ns = late_ns;
    }
    if (work_ns > t->win.loop_work_max_ns) {
        t->win.loop_work_max_ns = work_ns;
    }
}

void nb_tick_content_present(struct nb_tick *t, uint64_t present_ns)
{
    uint64_t n = t->commits_since_present;

    t->commits_since_present = 0;
    if (n == 0) {
        t->win.repeat_n++;
        return;
    }
    t->win.merged += n - 1;
    t->win.new_n++;
    if (t->have_content && present_ns > t->last_content_ns) {
        uint64_t g = present_ns - t->last_content_ns;

        if (g > t->win.cgap_max_ns) {
            t->win.cgap_max_ns = g;
        }
        t->win.cg33 += g > 33000000ull;
        t->win.cg50 += g > 50000000ull;
        t->win.cg100 += g > 100000000ull;
    }
    t->have_content = true;
    t->last_content_ns = present_ns;
}
