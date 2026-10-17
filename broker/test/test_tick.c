/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * test_tick.c -- the display tick's decisions (nb_tick.c), driven directly:
 * no socket, no display.  Linked against the SAME nb_tick.c the broker links.
 *
 * Time is a number the test passes in, so every boundary (exactly 100 ms,
 * exactly half a period, the sequence wrap) is hit to the nanosecond.  Every
 * check prints a DISTINCT message per rule, because test/mutate_tick.py counts
 * distinct failing messages: one failing line is a coincidence, two is a net.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "nvkvm_broker.h"

static unsigned n_checks, n_fail;

#define CHECK(cond, ...) do {                                           \
        n_checks++;                                                     \
        if (!(cond)) {                                                  \
            n_fail++;                                                   \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);        \
            fprintf(stderr, __VA_ARGS__);                               \
            fprintf(stderr, "\n");                                      \
        }                                                               \
    } while (0)

#define MS 1000000ull
#define P60 16666667ull         /* 60 Hz nominal period, as a compositor says it */

static struct nb_tick T;

static void fresh(unsigned cap)
{
    nb_tick_reset(&T, true, cap);
}

/* One feedback; returns whether it ticked. */
static bool fb(uint64_t now, uint64_t period, struct nb_tick_out *o)
{
    return nb_tick_refresh(&T, now, now, period, 0, NB_HZ_AUTO, 0, o);
}

/* The same with a receipt-time stamp (a frame-callback tick). */
static bool fbc(uint64_t now, uint64_t period, struct nb_tick_out *o)
{
    return nb_tick_refresh(&T, now, now, period, NVKVM_BROKER_TICK_F_CLOCK_ONLY,
                           NB_HZ_AUTO, 0, o);
}

/* --- the divisor ---------------------------------------------------------- */
static void test_div(void)
{
    static const struct { double hz; unsigned cap; unsigned want; } t[] = {
        { 24, 75, 1 },  { 30, 75, 1 },  { 50, 75, 1 },   { 59.94, 75, 1 },
        { 60, 75, 1 },  { 72, 75, 1 },  { 75, 75, 1 },   { 76, 75, 2 },
        { 100, 75, 2 }, { 120, 75, 2 }, { 144, 75, 2 },  { 150, 75, 2 },
        { 165, 75, 3 }, { 200, 75, 3 }, { 240, 75, 4 },  { 360, 75, 5 },
        { 60, 60, 1 },  { 144, 60, 3 }, { 120, 60, 2 },  { 75, 60, 2 },
        { 75.2, 75, 1 }, { 75.45, 75, 1 }, { 75.6, 75, 2 },
        { 144, 0, 2 },  { 144, 200, 2 },    /* 0 and too-big caps mean 75 */
        { 144, 10, 6 },                     /* a too-small cap is 24 */
    };

    for (unsigned i = 0; i < sizeof(t) / sizeof(t[0]); i++) {
        uint64_t period = (uint64_t)(1e9 / t[i].hz);
        unsigned d = nb_tick_div(period, t[i].cap, NB_HZ_AUTO, 0);

        CHECK(d == t[i].want, "div: %g Hz cap %u gave %u, want %u",
              t[i].hz, t[i].cap, d, t[i].want);
    }
    /* the nominal period of a 75 Hz output is 13333333 ns = 75.0000019 Hz:
     * the slack keeps it undivided */
    CHECK(nb_tick_div(13333333, 75, NB_HZ_AUTO, 0) == 1, "div: 75 Hz nominal period");
    CHECK(nb_tick_div(13297872 /* 75.2 Hz */, 75, NB_HZ_AUTO, 0) == 1, "div: slack above the cap");
    CHECK(nb_tick_div(13333333, 0, NB_HZ_AUTO, 0) == 1, "div: 75 Hz cap 0");
    /* unknown host rate: undivided */
    CHECK(nb_tick_div(0, 75, NB_HZ_AUTO, 0) == 1, "div: unknown period");
    /* fixed and none */
    CHECK(nb_tick_div(6944444, 75, NB_HZ_FIXED, 72000) == 2, "div: fixed 72 of 144");
    CHECK(nb_tick_div(6944444, 75, NB_HZ_FIXED, 144000) == 1, "div: fixed 144 of 144");
    CHECK(nb_tick_div(16666667, 75, NB_HZ_FIXED, 60000) == 1, "div: fixed 60 of 60");
    CHECK(nb_tick_div(6944444, 75, NB_HZ_FIXED, 0) == 2, "div: fixed 0 is 60");
    CHECK(nb_tick_div(6944444, 75, NB_HZ_NONE, 0) == 2, "div: none 144");
    CHECK(nb_tick_div(16666667, 75, NB_HZ_NONE, 0) == 1, "div: none 60");
    CHECK(nb_tick_div(1000000, 75, NB_HZ_AUTO, 0) <= 64, "div: bounded");
    CHECK(nb_tick_div(100, 75, NB_HZ_AUTO, 0) == 64, "div: 10 MHz is bounded to 64");
    /* the advertised mode rate is host/div */
    CHECK(nb_tick_mode_mhz(144000, 2) == 72000, "mode: 144/2");
    CHECK(nb_tick_mode_mhz(165000, 3) == 55000, "mode: 165/3");
    CHECK(nb_tick_mode_mhz(60000, 1) == 60000, "mode: 60/1");
    CHECK(nb_tick_mode_mhz(0, 1) == 0, "mode: unknown stays unknown");
}

/* --- sequence numbers ------------------------------------------------------ */
static void test_seq(void)
{
    CHECK(nb_tick_next_seq(0) == 1, "seq: 0 -> 1");
    CHECK(nb_tick_next_seq(1) == 2, "seq: 1 -> 2");
    CHECK(nb_tick_next_seq(0xfffffffeu) == 0xffffffffu, "seq: max-1 -> max");
    CHECK(nb_tick_next_seq(0xffffffffu) == 1, "seq: wraps to 1, never 0");
    CHECK(nb_tick_seq_newer(2, 1), "newer: 2 > 1");
    CHECK(!nb_tick_seq_newer(1, 1), "newer: equal is not newer");
    CHECK(!nb_tick_seq_newer(1, 2), "newer: 1 < 2");
    CHECK(nb_tick_seq_newer(1, 0xffffffffu), "newer: wrapped 1 > max");
    CHECK(!nb_tick_seq_newer(0xffffffffu, 1), "newer: max < wrapped 1");
    CHECK(nb_tick_seq_newer(0x7fffffffu, 0), "newer: half range edge");
    CHECK(!nb_tick_seq_newer(0x80000000u, 0), "newer: past half range is older");
}

/* --- one tick per accepted, decimated feedback ------------------------------ */
static void test_refresh(void)
{
    struct nb_tick_out o;
    uint64_t now = 1000 * MS;

    /* off: nothing ever */
    nb_tick_reset(&T, false, 0);
    CHECK(!fb(now, P60, &o), "refresh: ticks while not negotiated");

    /* 60 Hz, cap 75: every feedback ticks; the first has seq 1, then +1 */
    fresh(0);
    for (unsigned i = 0; i < 10; i++, now += P60) {
        bool r = fb(now, P60, &o);

        CHECK(r, "refresh: a 60 Hz feedback must tick");
        CHECK(r && o.tick_seq == i + 1, "refresh: seq counts +1 per tick (%u)", o.tick_seq);
        CHECK(r && (o.refresh_word & NVKVM_BROKER_TICK_REFRESH_MASK) == P60,
              "refresh: the period is the host's (div 1)");
        CHECK(r && o.present_ns == now, "refresh: present_ns is the feedback's");
    }
    CHECK(T.n_ticks == 10 && T.n_feedback == 10 && T.n_decimated == 0 && T.n_fast == 0,
          "refresh: counters 10/10/0/0");

    /* 144 Hz, cap 75: every SECOND feedback ticks, the first one first, and the
     * period on the wire is the decimated one */
    fresh(0);
    now = 5000 * MS;
    {
        uint64_t p144 = 6944444;
        unsigned ticks = 0;

        for (unsigned i = 0; i < 100; i++, now += p144) {
            bool r = fb(now, p144, &o);

            CHECK(r == (i % 2 == 0), "decimate: 144 Hz ticks on feedbacks 0,2,4.. (i=%u)", i);
            if (r) {
                ticks++;
                CHECK((o.refresh_word & NVKVM_BROKER_TICK_REFRESH_MASK) == 2 * p144,
                      "decimate: the period on the wire is host x div");
            }
        }
        CHECK(ticks == 50, "decimate: 100 feedbacks at 144 Hz -> 50 ticks, got %u", ticks);
    }
    /* 240 Hz: every fourth */
    fresh(0);
    now = 9000 * MS;
    {
        uint64_t p240 = 4166667;
        unsigned ticks = 0;

        for (unsigned i = 0; i < 400; i++, now += p240) {
            ticks += fb(now, p240, &o);
        }
        CHECK(ticks == 100, "decimate: 400 feedbacks at 240 Hz -> 100 ticks, got %u", ticks);
    }
    /* the client's cap divides further: 144 Hz, cap 60 -> div 3 */
    fresh(60);
    now = 12000 * MS;
    {
        uint64_t p144 = 6944444;
        unsigned ticks = 0;

        for (unsigned i = 0; i < 90; i++, now += p144) {
            ticks += fb(now, p144, &o);
        }
        CHECK(ticks == 30, "decimate: cap 60 at 144 Hz -> every third, got %u", ticks);
    }

    /* A refresh-rate change mid-stream restarts the count: the first accepted
     * feedback at the new rate ticks. */
    fresh(0);
    now = 20000 * MS;
    CHECK(fb(now, P60, &o), "rate change: 60 Hz ticks");
    now += P60;
    CHECK(fb(now, 6944444, &o), "rate change: the first 144 Hz feedback ticks at once");
    now += 6944444;
    CHECK(!fb(now, 6944444, &o), "rate change: then every second");

    /* a changed word: an out-of-range period is put on the wire as 0 */
    fresh(0);
    now = 30000 * MS;
    CHECK(nb_tick_refresh(&T, now, now, 100, 0, NB_HZ_AUTO, 0, &o) &&
          (o.refresh_word & NVKVM_BROKER_TICK_REFRESH_MASK) == 0,
          "word: a sub-millisecond period is sent as unknown");
    fresh(0);
    CHECK(nb_tick_refresh(&T, now, now, 0, 0, NB_HZ_AUTO, 0, &o) &&
          (o.refresh_word & NVKVM_BROKER_TICK_REFRESH_MASK) == 0,
          "word: an unknown period is sent as unknown");
    fresh(0);
    CHECK(nb_tick_refresh(&T, now, now, 300 * MS, 0, NB_HZ_AUTO, 0, &o) &&
          (o.refresh_word & NVKVM_BROKER_TICK_REFRESH_MASK) == 0,
          "word: a period past the mask is sent as unknown");
    fresh(0);
    CHECK(nb_tick_refresh(&T, now, now, NVKVM_BROKER_TICK_REFRESH_MIN_NS, 0,
                          NB_HZ_AUTO, 0, &o) &&
          (o.refresh_word & NVKVM_BROKER_TICK_REFRESH_MASK) ==
              NVKVM_BROKER_TICK_REFRESH_MIN_NS * T.div,
          "word: exactly 1 ms is kept");
    /* the flag bits never leak into the period and vice versa */
    fresh(0);
    CHECK(nb_tick_refresh(&T, now, now, P60, NVKVM_BROKER_TICK_F_CLOCK_ONLY,
                          NB_HZ_AUTO, 0, &o) &&
          (o.refresh_word & NVKVM_BROKER_TICK_F_CLOCK_ONLY) &&
          !(o.refresh_word & NVKVM_BROKER_TICK_F_RESERVED),
          "word: CLOCK_ONLY passes through, reserved stays clear");
    fresh(0);
    CHECK(nb_tick_refresh(&T, now, now, P60, 0xffffffffu, NB_HZ_AUTO, 0, &o) &&
          !(o.refresh_word & NVKVM_BROKER_TICK_F_RESERVED),
          "word: a caller's junk flags cannot set reserved bits");

    /* present_ns: 0 means now; never decreasing */
    fresh(0);
    now = 40000 * MS;
    CHECK(nb_tick_refresh(&T, now, 0, P60, 0, NB_HZ_AUTO, 0, &o) && o.present_ns == now,
          "present: 0 means the feedback's own time");
    now += P60;
    CHECK(nb_tick_refresh(&T, now, now - 10 * MS, P60, 0, NB_HZ_AUTO, 0, &o) &&
          o.present_ns >= now - 10 * MS && o.present_ns >= T.last_present_ns,
          "present: ok");
    {
        uint64_t before = T.last_present_ns;

        now += P60;
        CHECK(nb_tick_refresh(&T, now, 1 * MS, P60, NVKVM_BROKER_TICK_F_CLOCK_ONLY,
                              NB_HZ_AUTO, 0, &o) &&
              o.present_ns == before,
              "present: a timestamp that goes backwards is clamped, never decreasing");
        now += P60;
        CHECK(nb_tick_refresh(&T, now, now, P60, 0, NB_HZ_AUTO, 0, &o) &&
              o.present_ns == now && T.last_present_ns >= before,
              "present: the tick after a clamped one carries its own stamp");
    }
    fresh(0);
    CHECK(nb_tick_refresh(&T, now, now, P60, 0xffffffffu, NB_HZ_AUTO, 0, &o) &&
          (o.refresh_word & NVKVM_BROKER_TICK_REFRESH_MASK) == P60,
          "word: junk flags cannot corrupt the period");
}

/* --- too fast / duplicates: the flood rules --------------------------------- */
static void test_fast(void)
{
    struct nb_tick_out o;
    uint64_t now = 100 * MS;

    fresh(0);
    CHECK(fb(now, P60, &o), "fast: first feedback ticks");
    /* the SAME instant again: a duplicate */
    CHECK(!fb(now, P60, &o), "fast: a duplicate feedback never ticks twice");
    CHECK(T.n_fast == 1, "fast: the duplicate is counted");
    /* one nanosecond before half a period: too fast; at half: accepted */
    CHECK(!fb(now + P60 / 2 - 1, P60, &o), "fast: just under half a period is dropped");
    CHECK(fb(now + P60 / 2, P60, &o), "fast: exactly half a period is a refresh");
    /* time going backwards */
    CHECK(!fb(now, P60, &o), "fast: a backwards clock never ticks");

    /* a flood at 10x, 100x, 1000x the refresh rate for one simulated second:
     * the ticks are the refreshes, not the flood */
    static const unsigned mult[] = { 10, 100, 1000 };

    for (unsigned m = 0; m < 3; m++) {
        uint64_t step = P60 / mult[m];
        unsigned ticks = 0;
        uint64_t t0 = 1000 * MS, t;

        fresh(0);
        for (t = t0; t < t0 + 1000 * MS; t += step) {
            ticks += fb(t, P60, &o);
        }
        /* a second at 60 Hz is 60 refreshes (+1 for the edge) */
        CHECK(ticks <= 121, "flood %ux: at most 2 ticks per period (the half-period rule), got %u",
              mult[m], ticks);
        CHECK(ticks >= 59, "flood %ux: the refreshes still tick (got %u)", mult[m], ticks);
        CHECK(T.n_fast > 0, "flood %ux: the excess is counted", mult[m]);
        CHECK(T.n_feedback == T.n_ticks + T.n_fast + T.n_decimated,
              "flood %ux: every feedback is accounted for", mult[m]);
    }
    /* the compositor's own timestamps: two feedbacks whose present_ns are the
     * same vsync, however late the events were dispatched, are one refresh */
    fresh(0);
    CHECK(nb_tick_refresh(&T, 100 * MS, 90 * MS, P60, 0, NB_HZ_AUTO, 0, &o),
          "present: first compositor timestamp ticks");
    CHECK(!nb_tick_refresh(&T, 130 * MS, 90 * MS, P60, 0, NB_HZ_AUTO, 0, &o),
          "present: the same vsync reported again is dropped");
    CHECK(!nb_tick_refresh(&T, 140 * MS, 90 * MS + P60 / 2 - 1, P60, 0, NB_HZ_AUTO, 0, &o),
          "present: a vsync under half a period later is dropped");
    CHECK(nb_tick_refresh(&T, 150 * MS, 90 * MS + P60, P60, 0, NB_HZ_AUTO, 0, &o),
          "present: the next vsync ticks however the dispatch is spaced");
    CHECK(!nb_tick_refresh(&T, 170 * MS, 50 * MS, P60, 0, NB_HZ_AUTO, 0, &o),
          "present: a compositor timestamp going backwards is dropped");
    /* CLOCK_ONLY timestamps are receipt times and are not judged this way */
    fresh(0);
    CHECK(nb_tick_refresh(&T, 100 * MS, 100 * MS, P60, NVKVM_BROKER_TICK_F_CLOCK_ONLY, NB_HZ_AUTO, 0, &o) &&
          nb_tick_refresh(&T, 100 * MS + P60, 100 * MS + P60, P60, NVKVM_BROKER_TICK_F_CLOCK_ONLY, NB_HZ_AUTO, 0, &o),
          "present: CLOCK_ONLY ticks by receipt time");

    /* the same rules for receipt-time (frame callback) ticks */
    fresh(0);
    CHECK(fbc(now, P60, &o), "fast/clock: first ticks");
    CHECK(!fbc(now, P60, &o), "fast/clock: a duplicate never ticks twice");
    CHECK(!fbc(now + P60 / 2 - 1, P60, &o), "fast/clock: just under half a period is dropped");
    CHECK(fbc(now + P60 / 2, P60, &o), "fast/clock: exactly half a period is a refresh");
    CHECK(!fbc(now, P60, &o), "fast/clock: a backwards clock never ticks");
    for (unsigned m = 0; m < 3; m++) {
        uint64_t step = P60 / mult[m];
        unsigned ticks = 0;
        uint64_t t0 = 1000 * MS, t;

        fresh(0);
        for (t = t0; t < t0 + 1000 * MS; t += step) {
            ticks += fbc(t, P60, &o);
        }
        CHECK(ticks <= 121, "flood/clock %ux: at most 2 ticks per period, got %u", mult[m], ticks);
        CHECK(ticks >= 59, "flood/clock %ux: the refreshes still tick (got %u)", mult[m], ticks);
        CHECK(T.n_fast > 0, "flood/clock %ux: the excess is counted", mult[m]);
    }

    /* a burst after silence: N feedbacks at the same instant tick once */
    fresh(0);
    {
        unsigned ticks = 0;

        for (unsigned i = 0; i < 1000; i++) {
            ticks += fb(5000 * MS, P60, &o);
        }
        CHECK(ticks == 1, "burst: 1000 feedbacks at one instant tick once, got %u", ticks);
    }
}

/* --- WANT_FRAME ------------------------------------------------------------- */
static void test_want(void)
{
    struct nb_tick_out o;
    uint64_t now = 1000 * MS;

    fresh(0);
    /* nothing open, nothing unpresented */
    CHECK(nb_tick_want(&T, now), "want: a fresh connection wants a frame");
    CHECK(fb(now, P60, &o) && (o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME),
          "want: the first tick carries WANT_FRAME");
    CHECK(T.open_req == o.tick_seq && T.open_req == 1, "want: it opens request = tick_seq");
    uint32_t r1 = o.tick_seq;

    /* open and unanswered: the following ticks do not want */
    for (unsigned i = 1; i < 6; i++) {
        now += P60;
        CHECK(fb(now, P60, &o) && !(o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME),
              "want: no second request while one is open");
        CHECK(T.open_req == r1, "want: the open request is unchanged");
    }
    /* the timeout edge: 100 ms - 1 ns: still open; exactly 100 ms: reopen */
    CHECK(!nb_tick_want(&T, T.open_since_ns + 100 * MS - 1), "want: 1 ns before the timeout");
    CHECK(nb_tick_want(&T, T.open_since_ns + 100 * MS), "want: exactly at the timeout");
    CHECK(nb_tick_want(&T, T.open_since_ns + 100 * MS + 1), "want: past the timeout");
    now = T.open_since_ns + 100 * MS;
    CHECK(fb(now, P60, &o) && (o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME),
          "want: a timed-out request is replaced by a new one");
    CHECK(T.open_req == o.tick_seq && T.open_req != r1 && T.n_req_timeouts == 1,
          "want: replaced, not stacked, counted");

    /* answer with a frame: closed, but NOT wanted again until presented */
    uint32_t open = T.open_req;

    CHECK(nb_tick_answer(&T, open, true, now) == NB_TICK_ANS_OPEN, "answer: the open request");
    CHECK(T.open_req == 0 && T.unpresented, "answer: closed, frame un-presented");
    CHECK(!nb_tick_want(&T, now + P60), "want: not while the answered frame is un-presented");
    now += P60;
    CHECK(fb(now, P60, &o) && !(o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME),
          "want: a tick between answer and presentation does not want");
    /* the unpresented timeout edge */
    CHECK(!nb_tick_want(&T, T.unpresented_since_ns + 100 * MS - 1), "want: unpresented 1 ns short");
    CHECK(nb_tick_want(&T, T.unpresented_since_ns + 100 * MS), "want: unpresented at the edge");
    /* presented: wanted at the next tick */
    nb_tick_presented(&T);
    CHECK(!T.unpresented, "presented clears it");
    now += P60;
    CHECK(fb(now, P60, &o) && (o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME),
          "want: after presentation the next tick wants");
    CHECK(T.n_want == 3, "want: three requests so far (%llu)", (unsigned long long)T.n_want);

    /* NO_CHANGE: closes without waiting for any presentation */
    open = T.open_req;
    CHECK(nb_tick_answer(&T, open, false, now) == NB_TICK_ANS_OPEN, "no_change: answers the open request");
    CHECK(!T.unpresented && T.open_req == 0, "no_change: nothing to present");
    now += P60;
    CHECK(fb(now, P60, &o) && (o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME),
          "no_change: the next tick wants at once");
    CHECK(T.n_no_change == 1, "no_change: counted");

    /* a timed-out un-presented frame counts as presented when the request
     * that follows it opens */
    fresh(0);
    now = 3000 * MS;
    fb(now, P60, &o);
    nb_tick_answer(&T, T.open_req, true, now);
    now += 100 * MS;
    CHECK(fb(now, P60, &o) && (o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME),
          "want: an un-presented frame stops blocking after the timeout");
    CHECK(T.n_present_timeouts == 1 && !T.unpresented, "want: the presentation timeout is counted");

    /* the prime */
    fresh(0);
    nb_tick_prime(&T, 7 * MS, &o);
    CHECK((o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME) &&
          (o.refresh_word & NVKVM_BROKER_TICK_F_CLOCK_ONLY) && o.tick_seq == 1 &&
          o.present_ns == 7 * MS, "prime: WANT, CLOCK_ONLY, seq 1, now");
    CHECK(T.open_req == 1, "prime: opens request 1");
}

/* --- answers ----------------------------------------------------------------- */
static void test_answer(void)
{
    struct nb_tick_out o;
    uint64_t now = 1000 * MS;

    /* nothing issued yet: any non-zero id is from the future */
    fresh(0);
    CHECK(nb_tick_answer(&T, 1, true, now) == NB_TICK_ANS_VIOLATION, "answer: id 1 before any tick");
    CHECK(nb_tick_answer(&T, 1, false, now) == NB_TICK_ANS_VIOLATION, "answer: NO_CHANGE 1 before any tick");
    /* zero */
    CHECK(nb_tick_answer(&T, 0, true, now) == NB_TICK_ANS_UNSOLICITED, "answer: COMMIT seq 0 is unsolicited");
    CHECK(T.n_unsolicited == 1 && T.open_req == 0, "answer: unsolicited is counted");
    CHECK(nb_tick_answer(&T, 0, false, now) == NB_TICK_ANS_VIOLATION, "answer: NO_CHANGE 0 is a violation");
    CHECK(T.n_unsolicited == 1, "answer: NO_CHANGE 0 is not counted as unsolicited");
    CHECK(nb_tick_answer(&T, 0, false, now) != NB_TICK_ANS_UNSOLICITED,
          "answer: NO_CHANGE 0 is never unsolicited");

    /* ticks 1 (WANT), 2, 3 */
    fb(now, P60, &o);
    now += P60;
    fb(now, P60, &o);
    now += P60;
    fb(now, P60, &o);
    CHECK(T.last_seq == 3 && T.open_req == 1, "answer: setup");
    CHECK(nb_tick_answer(&T, 4, true, now) == NB_TICK_ANS_VIOLATION, "answer: the next, un-issued id");
    CHECK(nb_tick_answer(&T, 0x80000000u, true, now) == NB_TICK_ANS_VIOLATION, "answer: a far-future id");
    CHECK(nb_tick_answer(&T, 0xffffffffu, false, now) == NB_TICK_ANS_LATE,
          "answer: an id behind the last tick (wrapped) is late, not future");
    CHECK(T.open_req == 1, "answer: violations leave the open request alone");
    /* an issued but never-requested tick */
    CHECK(nb_tick_answer(&T, 2, true, now) == NB_TICK_ANS_LATE, "answer: an issued non-WANT tick is late");
    CHECK(T.open_req == 1 && !T.unpresented, "answer: late does not close or block");
    CHECK(T.n_late >= 1, "answer: late is counted");
    /* the open one */
    CHECK(nb_tick_answer(&T, 1, true, now) == NB_TICK_ANS_OPEN, "answer: the open request");
    /* answering it again is late */
    CHECK(nb_tick_answer(&T, 1, true, now) == NB_TICK_ANS_LATE, "answer: a duplicate answer is late");
    CHECK(nb_tick_answer(&T, 1, false, now) == NB_TICK_ANS_LATE, "answer: a duplicate NO_CHANGE is late");

    /* a timed-out request answered late: LATE, the replacement stays open */
    fresh(0);
    now = 5000 * MS;
    fb(now, P60, &o);                   /* request 1 */
    now += 100 * MS;
    fb(now, P60, &o);                   /* request 2 replaces it */
    CHECK(T.open_req == 2, "late: replacement setup");
    CHECK(nb_tick_answer(&T, 1, true, now) == NB_TICK_ANS_LATE, "late: the dead request's answer");
    CHECK(T.open_req == 2 && !T.unpresented,
          "late: the replacement stays open and the late frame does not block it");

    /* across the sequence wrap */
    fresh(0);
    T.last_seq = 0xfffffffeu;
    now = 8000 * MS;
    fb(now, P60, &o);                   /* 0xffffffff, WANT */
    CHECK(o.tick_seq == 0xffffffffu && T.open_req == 0xffffffffu, "wrap: max opens");
    CHECK(nb_tick_answer(&T, 0xffffffffu, true, now) == NB_TICK_ANS_OPEN, "wrap: max answers");
    nb_tick_presented(&T);
    now += P60;
    fb(now, P60, &o);
    CHECK(o.tick_seq == 1 && (o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME),
          "wrap: the tick after max is 1, never 0");
    CHECK(nb_tick_answer(&T, 1, true, now) == NB_TICK_ANS_OPEN, "wrap: 1 answers after the wrap");
    CHECK(nb_tick_answer(&T, 2, true, now) == NB_TICK_ANS_VIOLATION, "wrap: 2 was never issued");
}

/* --- the TX queue's rule ------------------------------------------------------ */
static void test_replace(void)
{
    const uint32_t W = NVKVM_BROKER_TICK_F_WANT_FRAME;

    CHECK(nb_tick_may_replace(0, 0), "replace: a bare tick by a bare tick");
    CHECK(nb_tick_may_replace(P60, P60), "replace: a tick with a period");
    CHECK(nb_tick_may_replace(NVKVM_BROKER_TICK_F_CLOCK_ONLY | 1000, 0), "replace: CLOCK_ONLY");
    CHECK(nb_tick_may_replace(0, W), "replace: a bare tick by a WANT tick");
    CHECK(!nb_tick_may_replace(W, 0), "replace: the live WANT tick is not replaced by a bare one");
    CHECK(!nb_tick_may_replace(W | P60, P60), "replace: WANT with a period, by a bare one");
    CHECK(nb_tick_may_replace(W, W), "replace: a dead WANT tick is replaced by a newer WANT");
    CHECK(nb_tick_may_replace(W | P60, W | P60 * 2), "replace: WANT by WANT with periods");
}

/* --- a long lock-step run: the invariants after every step --------------------- */
static void test_invariants(void)
{
    struct nb_tick_out o;
    uint64_t now = 1000 * MS, rng = 0x9e3779b97f4a7c15ull;
    uint32_t last = 0;
    unsigned issued_want = 0, answered = 0;
    uint64_t last_present = 0;

    fresh(0);
    for (unsigned step = 0; step < 200000; step++) {
        rng = rng * 6364136223846793005ull + 1442695040888963407ull;
        unsigned r = (unsigned)(rng >> 33) % 100;

        now += (rng >> 20) % (P60 / 3);
        if (r < 55) {
            uint64_t jitter = (rng >> 40) % (10 * MS);

            if (nb_tick_refresh(&T, now, now - jitter, P60,
                                NVKVM_BROKER_TICK_F_CLOCK_ONLY, NB_HZ_AUTO, 0, &o)) {
                CHECK(nb_tick_seq_newer(o.tick_seq, last), "run: tick_seq strictly increases");
                CHECK(o.tick_seq != 0, "run: tick_seq is never 0");
                CHECK(o.present_ns >= last_present, "run: present_ns never decreases");
                last = o.tick_seq;
                last_present = o.present_ns;
                if (o.refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME) {
                    issued_want++;
                    CHECK(T.open_req == o.tick_seq, "run: WANT opens request = tick_seq");
                }
            }
        } else if (r < 70) {
            if (T.open_req) {
                CHECK(nb_tick_answer(&T, T.open_req, true, now) == NB_TICK_ANS_OPEN, "run: open answers");
                answered++;
            }
        } else if (r < 80) {
            if (T.open_req) {
                CHECK(nb_tick_answer(&T, T.open_req, false, now) == NB_TICK_ANS_OPEN, "run: no_change answers");
                answered++;
            }
        } else if (r < 90) {
            nb_tick_presented(&T);
        } else {
            uint32_t junk = (uint32_t)(rng >> 11);
            enum nb_tick_ans a = nb_tick_answer(&T, junk, (rng & 1) != 0, now);

            if (a == NB_TICK_ANS_OPEN) {
                answered++;
            }
        }
        CHECK(T.open_req == 0 || nb_tick_seq_newer(T.last_seq + 1u, T.open_req),
              "run: the open request was issued");
        CHECK(T.n_feedback == T.n_ticks + T.n_fast + T.n_decimated,
              "run: every feedback is accounted for");
    }
    CHECK(issued_want >= answered, "run: no more answers than requests (%u/%u)", answered, issued_want);
    CHECK(T.n_want == issued_want, "run: n_want counts the WANT ticks");
}

/* ★ MEASUREMENT: the per-second stats line is pure: deltas, one line per period, never a burst, the format is stable. */
static void test_stats(void)
{
    struct nb_tick ST;
    struct nb_tick_snap a, b;
    uint64_t next = 0;
    char buf[400];
    const uint64_t P = NB_TICK_STATS_PERIOD_NS;

    nb_tick_reset(&ST, true, 60);
    nb_tick_snap_take(&ST, &a);
    CHECK(!nb_tick_stats_due(5 * P, &next, P), "stats: the first call only arms");
    CHECK(next == 6 * P, "stats: armed one period ahead");
    CHECK(!nb_tick_stats_due(6 * P - 1, &next, P), "stats: not due a ns early");
    CHECK(nb_tick_stats_due(6 * P, &next, P), "stats: due at the period");
    CHECK(next == 7 * P, "stats: next is a period later");
    CHECK(nb_tick_stats_due(20 * P, &next, P) && next == 21 * P, "stats: after a gap one line, then a period from now");
    CHECK(!nb_tick_stats_due(20 * P + 1, &next, P), "stats: no catch-up burst");
    ST.n_feedback = 61; ST.n_ticks = 60; ST.n_want = 50; ST.n_answered = 49; ST.n_no_change = 9;
    ST.n_req_timeouts = 2; ST.n_late = 1; ST.n_unsolicited = 3; ST.n_present_timeouts = 0; ST.n_coalesced = 4;
    nb_tick_snap_take(&ST, &b);
    CHECK(nb_tick_stats_format(buf, sizeof(buf), 123456, &a, &b, &ST.win) > 0, "stats: formats");
    CHECK(strstr(buf, "stats t=123456 feedbacks=61 ticks=60 want_ticks=50 commit=40 no_change=9 timeouts=2 late=1 "
                      "unsolicited=3 present_timeouts=0 coalesced=4 gap_max_us=0 ") == buf, "stats: line is '%s'", buf);
    /* the window: gaps, tick->answer and commit->present latencies, discards, loop stalls */
    nb_tick_reset(&ST, true, 60);
    nb_tick_win_gap(&ST, 16000000ull);
    nb_tick_win_gap(&ST, 50000000ull);
    nb_tick_win_gap(&ST, 34000000ull);
    CHECK(ST.win.gap_max_ns == 50000000ull && ST.win.gap33 == 2, "win: max gap and gaps over 33 ms");
    ST.last_seq = 7; ST.open_req = 7; ST.open_since_ns = 1000000000ull;
    CHECK(nb_tick_answer(&ST, 7, true, 1004000000ull) == NB_TICK_ANS_OPEN, "win: answer");
    CHECK(ST.win.ans_n == 1 && ST.win.ans_max_ns == 4000000ull, "win: tick->answer latency");
    nb_tick_presented_at(&ST, 1021000000ull, false);
    CHECK(ST.win.pres_n == 1 && ST.win.pres_max_ns == 17000000ull && !ST.unpresented, "win: commit->present latency");
    ST.unpresented = true; ST.unpresented_since_ns = 1030000000ull;
    nb_tick_presented_at(&ST, 1031000000ull, true);
    CHECK(ST.win.discarded == 1 && ST.win.pres_n == 2, "win: a discard is counted and ends the wait");
    nb_tick_win_loop(&ST, 3000000ull, 9000000ull);
    nb_tick_win_loop(&ST, 1000000ull, 2000000ull);
    CHECK(ST.win.loop_late_max_ns == 3000000ull && ST.win.loop_work_max_ns == 9000000ull, "win: loop maxima");
    /* presents: NEW when a COMMIT was answered since the previous present, else a repeat; extra commits merge */
    ST.commits_since_present = 1;
    nb_tick_content_present(&ST, 1000000000ull);          /* new */
    nb_tick_content_present(&ST, 1016000000ull);          /* repeat */
    nb_tick_content_present(&ST, 1032000000ull);          /* repeat */
    ST.commits_since_present = 1;
    nb_tick_content_present(&ST, 1060000000ull);          /* new, 60 ms after the previous NEW one */
    ST.commits_since_present = 3;
    nb_tick_content_present(&ST, 1070000000ull);          /* new (two commits merged), 10 ms */
    ST.commits_since_present = 1;
    nb_tick_content_present(&ST, 1200000000ull);          /* new, 130 ms */
    CHECK(ST.win.new_n == 4 && ST.win.repeat_n == 2 && ST.win.merged == 2, "content: new / repeat / merged (%llu %llu %llu)",
          (unsigned long long)ST.win.new_n, (unsigned long long)ST.win.repeat_n, (unsigned long long)ST.win.merged);
    CHECK(ST.win.cgap_max_ns == 130000000ull && ST.win.cg33 == 2 && ST.win.cg50 == 2 && ST.win.cg100 == 1,
          "content: gap max and thresholds (%llu %llu %llu %llu)", (unsigned long long)ST.win.cgap_max_ns,
          (unsigned long long)ST.win.cg33, (unsigned long long)ST.win.cg50, (unsigned long long)ST.win.cg100);
    {
        struct nb_tick S2;

        nb_tick_reset(&S2, true, 60);
        S2.last_seq = 3; S2.open_req = 3; S2.open_since_ns = 10;
        CHECK(nb_tick_answer(&S2, 3, true, 20) == NB_TICK_ANS_OPEN && S2.commits_since_present == 1, "content: a COMMIT answer counts");
        S2.last_seq = 4; S2.open_req = 4;
        CHECK(nb_tick_answer(&S2, 4, false, 30) == NB_TICK_ANS_OPEN && S2.commits_since_present == 1, "content: NO_CHANGE is not content");
    }
    nb_tick_win_gap(&ST, 120000000ull);
    CHECK(ST.win.gaps50 == 1 && ST.win.gaps100 == 1, "win: refresh gaps over 50 / 100 ms");
    nb_tick_snap_take(&ST, &a);
    nb_tick_snap_take(&ST, &b);
    CHECK(nb_tick_stats_format(buf, sizeof(buf), 5, &a, &b, &ST.win) > 0 && strstr(buf, " new=4 repeat=2 merged=2 cgap_max_us=130000 cg33=2 cg50=2 cg100=1") != NULL, "content line is '%s'", buf);
    CHECK(nb_tick_stats_format(buf, sizeof(buf), 5, &a, &b, &ST.win) > 0 && strstr(buf, "gap_max_us=120000 gaps33=3 discarded=1 ans_max_us=4000 ans_mean_us=4000 pres_max_us=17000 pres_mean_us=9000 loop_late_max_us=3000 loop_work_max_us=9000") != NULL, "win: line is '%s'", buf);
    CHECK(nb_tick_stats_format(buf, 20, 1, &a, &b, &ST.win) == 0, "stats: a small buffer yields nothing, never a overrun");
    CHECK(nb_tick_stats_format(buf, sizeof(buf), 1, &b, &b, &ST.win) > 0 && strstr(buf, "ticks=0 ") != NULL, "stats: no change, zero deltas");
    CHECK(nb_tick_stats_format(buf, sizeof(buf), 1, &b, &a, &ST.win) > 0 && strstr(buf, "feedbacks=0 ") != NULL, "stats: a reset never underflows");
}

int main(void)
{
    test_div();
    test_seq();
    test_refresh();
    test_fast();
    test_want();
    test_answer();
    test_replace();
    test_invariants();
    test_stats();
    printf("test_tick: %u checks, %u failed\n", n_checks, n_fail);
    return n_fail ? 1 : 0;
}
