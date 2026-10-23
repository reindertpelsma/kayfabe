/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * test_xor.c -- the XOR cursor, GPU-free (D2, 2026-10-17).
 *
 * Linked against the SAME nb_xor.c / nb_frame.c / nb_cursor.c objects the
 * broker links.  What it covers:
 *
 *   1. the state machine (nb_xor_decide): the WHOLE input table against an
 *      independently written oracle, named rows, invariants ("never two
 *      cursors"), and the transitions where the old bug lived;
 *   2. placement (nb_xor_place) and the pointer->frame map: the 0.5 logical-px
 *      registration bound, proven over a sweep and over hostile inputs;
 *   3. the pure helpers (exporter name, allowlist, layout);
 *   4. the frame reader on a SEALED MEMFD: open, per-frame check, bounded
 *      reads, a resized frame, an unsealed memfd, a mapping that fails, a
 *      description that disagrees with the mapping (guard page behind it);
 *   5. the same on a real udmabuf where the box has one (SKIP otherwise);
 *   6. the whole driver (nb_xor_eval) on frames read from the memfd, compared
 *      with the retired kernel's formula (compose_reference's XOR branch:
 *      below ^ (w & 0x00ffffff)) for white / black / colour / alpha 0 / all
 *      four edges / off the frame, a resize, a mapping failure -> fallback with
 *      ONE choice change, and a random differential.
 *
 * Every check prints "FAIL <name>" on failure; test/mutate_xor.sh breaks the
 * sources on purpose and demands >= 2 distinct failures per mutant.
 */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdio.h>
#include <sys/prctl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <signal.h>
#include <sys/wait.h>
#include <unistd.h>
#include <linux/udmabuf.h>

#include "nvkvm_broker.h"

/* the broker's logging, unused here (the pure objects never log) */
int nb_verbose;
void nb_log(const char *fmt, ...) { (void)fmt; }
void nb_err(const char *fmt, ...) { (void)fmt; }

#ifndef F_SEAL_SHRINK
#define F_SEAL_SHRINK 0x0002
#endif
#ifndef F_ADD_SEALS
#define F_ADD_SEALS 1033
#endif

static unsigned n_checks, n_failed;

#define CHECK(cond, ...)                                                      \
    do {                                                                      \
        n_checks++;                                                           \
        if (!(cond)) {                                                        \
            n_failed++;                                                       \
            printf("FAIL ");                                                  \
            printf(__VA_ARGS__);                                              \
            printf("\n");                                                     \
        }                                                                     \
    } while (0)

static uint32_t rng_state = 0x12345678u;
static uint32_t rnd(void)
{
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 17;
    rng_state ^= rng_state << 5;
    return rng_state;
}

/* ═════════════════════════════════════════════════════════════════════════
 * 1. THE STATE MACHINE
 * ════════════════════════════════════════════════════════════════════════ */

/*
 * The ORACLE: the specification, written as rules in a different shape than
 * the implementation (rules by situation, not by input).
 */
static struct nb_xor_out oracle(const struct nb_xor_in *in)
{
    struct nb_xor_out o = { NB_XP_UNTOUCHED, NB_XO_OFF, NB_XC_NONE };
    bool ours = in->ptr_inside && in->serial_ok;

    /* the standing choice: what the cursor IS and whether it can be inverted */
    if (in->kind == NB_XK_PLAIN) {
        o.choice = NB_XC_PLAIN;
    }
    if (in->kind == NB_XK_XOR) {
        o.choice = in->frame_readable ? NB_XC_COMPOSED : NB_XC_APPROX;
    }
    if (in->grabbed) {
        /* pointer locked: the host pointer carries nothing; the guest's
         * position (a MOVE) places an overlay */
        if (ours) {
            o.ptr = NB_XP_HIDDEN;
        }
        if (in->kind != NB_XK_NONE && in->placed) {
            o.ov = (in->kind == NB_XK_XOR && in->frame_readable) ? NB_XO_XOR
                                                                 : NB_XO_IMAGE;
        }
        return o;
    }
    if (!ours) {
        return o;
    }
    if (in->kind == NB_XK_NONE) {
        o.ptr = NB_XP_HIDDEN;
    } else if (in->kind == NB_XK_PLAIN) {
        o.ptr = NB_XP_GUEST_IMAGE;
    } else if (in->frame_readable) {
        o.ptr = NB_XP_HIDDEN;
        o.ov = NB_XO_XOR;
    } else {
        o.ptr = NB_XP_GUEST_IMAGE;
    }
    return o;
}

static const char *ptr_name(enum nb_xor_ptr p)
{
    return p == NB_XP_UNTOUCHED ? "untouched" : p == NB_XP_HIDDEN ? "hidden" : "image";
}
static const char *ov_name(enum nb_xor_ov o)
{
    return o == NB_XO_OFF ? "off" : o == NB_XO_XOR ? "xor" : "image";
}

static struct nb_xor_in mk(enum nb_xor_kind k, bool grab, bool placed, bool rd,
                           bool inside, bool serial)
{
    struct nb_xor_in in = { k, grab, placed, rd, inside, serial };

    return in;
}

static void test_state_table(void)
{
    int k, g, p, r, i, s;
    unsigned rows = 0;

    for (k = 0; k < 3; k++)
        for (g = 0; g < 2; g++)
            for (p = 0; p < 2; p++)
                for (r = 0; r < 2; r++)
                    for (i = 0; i < 2; i++)
                        for (s = 0; s < 2; s++) {
                            struct nb_xor_in in = mk((enum nb_xor_kind)k, g, p, r, i, s);
                            struct nb_xor_out a = nb_xor_decide(&in), b = oracle(&in);

                            rows++;
                            CHECK(a.ptr == b.ptr && a.ov == b.ov && a.choice == b.choice,
                                  "table: kind=%d grab=%d placed=%d readable=%d inside=%d "
                                  "serial=%d: got ptr=%s ov=%s choice=%s, want ptr=%s ov=%s "
                                  "choice=%s", k, g, p, r, i, s, ptr_name(a.ptr),
                                  ov_name(a.ov), nb_xor_choice_text(a.choice),
                                  ptr_name(b.ptr), ov_name(b.ov),
                                  nb_xor_choice_text(b.choice));
                        }
    CHECK(rows == 96, "table: all 96 rows walked (%u)", rows);

    /* ── invariants, over the same 96 rows, from the implementation ── */
    for (k = 0; k < 3; k++)
        for (g = 0; g < 2; g++)
            for (p = 0; p < 2; p++)
                for (r = 0; r < 2; r++)
                    for (i = 0; i < 2; i++)
                        for (s = 0; s < 2; s++) {
                            struct nb_xor_in in = mk((enum nb_xor_kind)k, g, p, r, i, s);
                            struct nb_xor_out a = nb_xor_decide(&in);

                            /* never two cursors: a true-inversion overlay in hover
                             * always comes with the host pointer hidden */
                            CHECK(!(!g && a.ov == NB_XO_XOR && a.ptr != NB_XP_HIDDEN),
                                  "invariant: hover overlay XOR must hide the host pointer "
                                  "(k=%d r=%d i=%d s=%d)", k, r, i, s);
                            /* ... and a guest image on the host pointer is never
                             * accompanied by an overlay in hover */
                            CHECK(!(!g && a.ptr == NB_XP_GUEST_IMAGE && a.ov != NB_XO_OFF),
                                  "invariant: hover image + overlay is two cursors "
                                  "(k=%d r=%d i=%d s=%d)", k, r, i, s);
                            /* only an XOR cursor is ever composed */
                            CHECK(!(k != NB_XK_XOR && a.ov == NB_XO_XOR),
                                  "invariant: a non-XOR cursor is never inverted (k=%d)", k);
                            /* the host pointer is never set without the enter serial
                             * and the pointer over the content */
                            CHECK(!((!s || !i) && a.ptr != NB_XP_UNTOUCHED),
                                  "invariant: no set_cursor without serial/inside "
                                  "(k=%d g=%d i=%d s=%d)", k, g, i, s);
                            /* the standing choice does not depend on the pointer, the
                             * serial, or the grab */
                            {
                                struct nb_xor_in o2 = mk((enum nb_xor_kind)k, !g, !p, r, !i, !s);
                                CHECK(nb_xor_decide(&o2).choice == a.choice,
                                      "invariant: the choice is independent of pointer/serial/"
                                      "grab (k=%d r=%d)", k, r);
                            }
                            /* the XOR overlay only ever appears on a readable frame */
                            CHECK(!(a.ov == NB_XO_XOR && !r),
                                  "invariant: no composed overlay on an unreadable frame");
                        }

    /* ── the named rows (what each looks like on the owner's screen) ── */
    {
        struct nb_xor_in in;
        struct nb_xor_out o;

        in = mk(NB_XK_XOR, false, false, true, true, true);
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_HIDDEN && o.ov == NB_XO_XOR && o.choice == NB_XC_COMPOSED,
              "named: hover, I-beam, frame readable: host pointer hidden, patch overlay");
        in = mk(NB_XK_XOR, false, false, false, true, true);
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_GUEST_IMAGE && o.ov == NB_XO_OFF && o.choice == NB_XC_APPROX,
              "named: hover, I-beam, frame NOT readable: the approximation as the host pointer");
        in = mk(NB_XK_PLAIN, false, false, true, true, true);
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_GUEST_IMAGE && o.ov == NB_XO_OFF && o.choice == NB_XC_PLAIN,
              "named: hover, arrow: the guest's image on the host pointer (unchanged)");
        in = mk(NB_XK_XOR, false, false, true, false, true);
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_UNTOUCHED && o.ov == NB_XO_OFF,
              "named: pointer on the title bar: nothing of ours, no overlay");
        in = mk(NB_XK_XOR, false, false, true, true, false);
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_UNTOUCHED && o.ov == NB_XO_OFF,
              "named: no enter serial: the host pointer cannot be hidden, so NO overlay");
        in = mk(NB_XK_XOR, true, true, true, true, true);
        o = nb_xor_decide(&in);
        CHECK(o.ov == NB_XO_XOR && o.ptr == NB_XP_HIDDEN, "named: grab, I-beam, readable: patch at the MOVE");
        in = mk(NB_XK_XOR, true, true, false, true, true);
        o = nb_xor_decide(&in);
        CHECK(o.ov == NB_XO_IMAGE, "named: grab, I-beam, unreadable: the approximation overlay");
        in = mk(NB_XK_XOR, true, false, true, true, true);
        o = nb_xor_decide(&in);
        CHECK(o.ov == NB_XO_OFF, "named: grab, no MOVE yet: no overlay");
        in = mk(NB_XK_NONE, false, false, true, true, true);
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_HIDDEN && o.ov == NB_XO_OFF && o.choice == NB_XC_NONE,
              "named: no cursor: the host pointer is hidden over the guest");
    }

    /* ── the transitions the old bug lived in: the host pointer must come
     *    back the moment the XOR cursor is not what is composed ── */
    {
        struct nb_xor_in in = mk(NB_XK_XOR, false, false, true, true, true);
        struct nb_xor_out o = nb_xor_decide(&in);

        CHECK(o.ptr == NB_XP_HIDDEN, "transition: I-beam composed hides the host pointer");
        in.kind = NB_XK_PLAIN;      /* the guest switched to an arrow */
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_GUEST_IMAGE && o.ov == NB_XO_OFF,
              "transition: I-beam -> arrow restores the host pointer, drops the overlay");
        in.kind = NB_XK_XOR;
        in.frame_readable = false;  /* the frame stopped being readable */
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_GUEST_IMAGE && o.ov == NB_XO_OFF,
              "transition: composed -> unreadable restores the host pointer (approximation)");
        in.frame_readable = true;
        in.kind = NB_XK_NONE;       /* HIDE */
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_HIDDEN && o.ov == NB_XO_OFF,
              "transition: HIDE takes the overlay down");
        in.kind = NB_XK_XOR;
        in.grabbed = true;
        in.placed = true;
        o = nb_xor_decide(&in);
        CHECK(o.ov == NB_XO_XOR, "transition: hover -> grab keeps the I-beam as the patch");
        in.grabbed = false;
        o = nb_xor_decide(&in);
        CHECK(o.ptr == NB_XP_HIDDEN && o.ov == NB_XO_XOR,
              "transition: grab -> hover: the host pointer hidden again, patch kept");
    }
}

/* ═════════════════════════════════════════════════════════════════════════
 * 2. PLACEMENT AND THE POINTER MAP
 * ════════════════════════════════════════════════════════════════════════ */

static void test_place(void)
{
    static const struct { uint32_t nx, dx; } fits[] = {
        { 1, 1 }, { 1915, 2048 }, { 2446, 2048 }, { 1000, 1001 }, { 3, 2 },
        { 4096, 2048 }, { 1920, 1920 }, { 7, 13 }, { 0, 0 }, { 640, 2048 },
    };
    static const uint32_t scales[] = { 0, 60, 120, 150, 180, 240, 360, 480, 9999 };
    unsigned fi, si, n = 0;
    struct nb_xor_place pl;

    for (fi = 0; fi < sizeof fits / sizeof fits[0]; fi++) {
        for (si = 0; si < sizeof scales / sizeof scales[0]; si++) {
            int tl;

            for (tl = -300; tl <= 2400; tl += 7) {
                uint32_t pw = 1u + (uint32_t)(tl & 0xff) % 40u, ph = 16;
                uint64_t nx = fits[fi].nx ? fits[fi].nx : 1, dx = fits[fi].dx ? fits[fi].dx : 1;
                int64_t e0, e1;
                bool ok;

                nb_xor_place(tl, tl / 2, pw, ph, fits[fi].nx, fits[fi].dx, fits[fi].nx,
                             fits[fi].dx, scales[si], &pl);
                n++;
                /* each edge within 0.5 logical px of tl * nx/dx: |x*dx - tl*nx| * 2 <= dx */
                e0 = ((int64_t)pl.x * (int64_t)dx - (int64_t)tl * (int64_t)nx) * 2;
                e1 = (((int64_t)pl.x + pl.gw) * (int64_t)dx -
                      ((int64_t)tl + pw) * (int64_t)nx) * 2;
                if (e0 < 0) { e0 = -e0; }
                if (e1 < 0) { e1 = -e1; }
                /* gw is clamped to >= 1, which can only move the right edge */
                ok = e0 <= (int64_t)dx &&
                     (e1 <= (int64_t)dx || ((int64_t)pl.gw == 1 && (int64_t)pw * (int64_t)nx < (int64_t)dx));
                CHECK(ok, "place: edges within 0.5 logical px (fit %u/%u tl %d pw %u)",
                      fits[fi].nx, fits[fi].dx, tl, pw);
                CHECK(pl.gw >= 1 && pl.gh >= 1 && pl.scale >= 1 && pl.scale <= 4 &&
                      pl.bw == pl.gw * pl.scale && pl.bh == pl.gh * pl.scale &&
                      pl.bw <= NB_CURSOR_SURFACE_MAX_BUF && pl.bh <= NB_CURSOR_SURFACE_MAX_BUF,
                      "place: sizes bounded and buffer = logical x scale (fit %u/%u scale %u)",
                      fits[fi].nx, fits[fi].dx, scales[si]);
                if (fits[fi].nx == fits[fi].dx || fits[fi].nx == 0) {
                    CHECK(pl.x == tl && pl.gw == pw, "place: fit 1 is exact");
                }
            }
        }
    }
    CHECK(n > 3000, "place: swept %u cases", n);
    /* rounding, not truncation: 5 * 1915/2048 = 4.67 -> 5; a floor would say 4 */
    nb_xor_place(5, 0, 1, 1, 1915, 2048, 1915, 2048, 120, &pl);
    CHECK(pl.x == 5, "place: rounds to nearest (4.67 -> 5, got %d)", pl.x);
    nb_xor_place(-5, 0, 1, 1, 1915, 2048, 1915, 2048, 120, &pl);
    CHECK(pl.x == -5, "place: negative rounds to nearest (-4.67 -> -5, got %d)", pl.x);
    /* the owner's two screens */
    nb_xor_place(821, 1097, 7, 16, 1915, 2048, 1915, 2048, 180, &pl);
    CHECK(pl.scale == 2 && pl.bw == pl.gw * 2, "place: scale 1.5 -> buffer_scale 2");
    /* hostile */
    nb_xor_place(INT32_MAX, INT32_MIN, 1u << 30, 1u << 30, 0xffffffffu, 1, 1, 0xffffffffu,
                 0xffffffffu, &pl);
    CHECK(pl.bw <= NB_CURSOR_SURFACE_MAX_BUF && pl.bh <= NB_CURSOR_SURFACE_MAX_BUF && pl.gw >= 1,
          "place: hostile inputs stay bounded");

    /* the pointer -> guest pixel map */
    CHECK(nb_xor_ptr_to_frame(0, 1, 1, 2048) == 0, "ptr: origin");
    CHECK(nb_xor_ptr_to_frame(100 * 256, 1, 1, 2048) == 100, "ptr: 1:1");
    CHECK(nb_xor_ptr_to_frame(100 * 256 + 255, 1, 1, 2048) == 100, "ptr: floor within a pixel");
    CHECK(nb_xor_ptr_to_frame(1915 * 256, 1915, 2048, 2048) == 2047, "ptr: right edge clamps");
    CHECK(nb_xor_ptr_to_frame(-5 * 256, 1, 1, 2048) == 0, "ptr: negative clamps to 0");
    CHECK(nb_xor_ptr_to_frame(INT32_MAX, 1, 1, 2048) == 2047, "ptr: huge clamps");
    CHECK(nb_xor_ptr_to_frame(1000 * 256, 1915, 2048, 2048) == 1069, "ptr: 1000 logical px at fit 1915/2048 -> guest px 1069 (got %d)",
          nb_xor_ptr_to_frame(1000 * 256, 1915, 2048, 2048));
    {
        int32_t prev = -1, f;
        bool mono = true;

        for (f = 0; f < 2000 * 256; f += 37) {
            int32_t v = nb_xor_ptr_to_frame(f, 1915, 2048, 2048);

            if (v < prev || v < 0 || v > 2047) { mono = false; }
            prev = v;
        }
        CHECK(mono, "ptr: monotonic and in range");
    }
}

static void test_scale_box(void)
{
    uint32_t src[11 * 7], dst[64 * 64], ref[64 * 64];
    uint32_t i, x, y;
    bool all;

    /* an I-beam-like patch: opaque white where it inverts, 0 elsewhere */
    for (i = 0; i < 77; i++) { src[i] = 0; }
    for (y = 0; y < 11; y++) { src[y * 7 + 3] = 0xffffffffu; }
    for (x = 1; x < 6; x++) { src[0 * 7 + x] = 0xffffffffu; src[10 * 7 + x] = 0xffffffffu; }
    /* identity */
    nb_px_scale_box(src, 7, 11, dst, 7, 11);
    CHECK(memcmp(src, dst, sizeof src) == 0, "box: same size is the identity");
    /* growing is nearest, exactly */
    nb_px_scale_box(src, 7, 11, dst, 21, 33);
    nb_px_scale(src, 7, 11, ref, 21, 33);
    CHECK(memcmp(dst, ref, 21 * 33 * 4) == 0, "box: an integer upscale equals nearest (crisp)");
    nb_px_scale_box(src, 7, 11, dst, 11, 17);
    nb_px_scale(src, 7, 11, ref, 11, 17);
    CHECK(memcmp(dst, ref, 11 * 17 * 4) == 0, "box: a fractional upscale equals nearest (crisp)");
    /* shrinking keeps thin features as coverage: a one-row serif the nearest
     * sampler skips (row 10 of 11 -> 6 rows samples rows 0,1,3,5,7,9) ... */
    for (i = 0; i < 77; i++) { src[i] = 0; }
    for (x = 1; x < 6; x++) { src[10 * 7 + x] = 0xffffffffu; }
    nb_px_scale_box(src, 7, 11, dst, 4, 6);
    {
        bool bottom = false;

        for (x = 0; x < 4; x++) { bottom |= (dst[5 * 4 + x] >> 24) != 0; }
        CHECK(bottom, "box: a one-row serif survives a vertical shrink");
    }
    /* ... and a one-column stem the nearest sampler skips (column 2 of 7 -> 4
     * columns samples 0,1,3,5) */
    for (i = 0; i < 77; i++) { src[i] = 0; }
    for (y = 0; y < 11; y++) { src[y * 7 + 2] = 0xffffffffu; }
    nb_px_scale_box(src, 7, 11, dst, 4, 6);
    {
        bool stem = false;

        for (y = 0; y < 6; y++) { stem |= (dst[y * 4 + 1] >> 24) != 0; }
        CHECK(stem, "box: a one-column stem survives a horizontal shrink");
    }
    /* uniform stays uniform; premultiplied ordering (channel <= alpha) holds */
    for (i = 0; i < 77; i++) { src[i] = 0xff336699u; }
    nb_px_scale_box(src, 7, 11, dst, 3, 5);
    all = true;
    for (i = 0; i < 15; i++) { if (dst[i] != 0xff336699u) { all = false; } }
    CHECK(all, "box: a uniform patch stays uniform when shrunk");
    for (i = 0; i < 77; i++) { src[i] = (i % 3) ? 0 : 0xff00ff80u; }
    nb_px_scale_box(src, 7, 11, dst, 3, 4);
    all = true;
    for (i = 0; i < 12; i++) {
        uint32_t a = dst[i] >> 24, r = (dst[i] >> 16) & 0xff, g = (dst[i] >> 8) & 0xff, b = dst[i] & 0xff;

        if (r > a || g > a || b > a) { all = false; }
    }
    CHECK(all, "box: averaged premultiplied pixels keep every colour channel <= alpha");
    /* 2:1 of a checkerboard of opaque white / clear = half coverage */
    {
        uint32_t cb[4 * 4], out[2 * 2];

        for (i = 0; i < 16; i++) { cb[i] = ((i % 4) + (i / 4)) % 2 ? 0xffffffffu : 0; }
        nb_px_scale_box(cb, 4, 4, out, 2, 2);
        CHECK(out[0] == 0x80808080u && out[3] == 0x80808080u, "box: 2:1 of a checkerboard is half coverage (0x%08x)", out[0]);
    }
    {
        uint32_t st[4 * 4], out[2 * 2];

        for (i = 0; i < 16; i++) { st[i] = (i / 4) % 2 ? 0xffffffffu : 0; }     /* row stripes */
        nb_px_scale_box(st, 4, 4, out, 2, 2);
        CHECK(out[0] == 0x80808080u && out[1] == 0x80808080u && out[2] == 0x80808080u,
              "box: 2:1 of row stripes is half coverage in every pixel (0x%08x)", out[0]);
        for (i = 0; i < 16; i++) { st[i] = (i % 4) % 2 ? 0xffffffffu : 0; }     /* column stripes */
        nb_px_scale_box(st, 4, 4, out, 2, 2);
        CHECK(out[0] == 0x80808080u && out[1] == 0x80808080u && out[3] == 0x80808080u,
              "box: 2:1 of column stripes is half coverage in every pixel (0x%08x)", out[0]);
    }
    /* hostile sizes: bounded, no overrun (ASan watches) */
    for (i = 0; i < 2000; i++) {
        uint32_t sw = 1 + rnd() % 256, sh = 1 + rnd() % 256, dw = 1 + rnd() % 64, dh = 1 + rnd() % 64;
        static uint32_t big[256 * 256], small[64 * 64];

        nb_px_scale_box(big, sw, sh, small, dw, dh);
    }
    nb_px_scale_box(NULL, 1, 1, dst, 1, 1);     /* ignored, not a crash */
    nb_px_scale_box(src, 0, 1, dst, 1, 1);
}

/* ═════════════════════════════════════════════════════════════════════════
 * 3. PURE HELPERS
 * ════════════════════════════════════════════════════════════════════════ */

static void test_helpers(void)
{
    char name[24];
    const char *t1 = "pos:\t0\nflags:\t02\nmnt_id:\t17\nino:\t4242\nsize:\t9461760\n"
                     "count:\t2\nexp_name:\tudmabuf\n";
    const char *t2 = "pos:\t0\nexp_name:\tamdgpu\nqueue:\t1\n";
    const char *t3 = "pos:\t0\nflags:\t02\n";
    const char *t4 = "pos:\t0\nexp_name:\t\n";
    const char *t5 = "exp_name:\tudmabuf\n";     /* the first line is never exp_name */
    const char *t6 = "pos:\t0\nexp_name:\tverylongexportername_overflowing_the_buffer\n";

    CHECK(nb_fdinfo_exp_name(t1, strlen(t1), name, sizeof name) && !strcmp(name, "udmabuf"),
          "fdinfo: udmabuf found");
    CHECK(nb_fdinfo_exp_name(t2, strlen(t2), name, sizeof name) && !strcmp(name, "amdgpu"),
          "fdinfo: amdgpu found");
    CHECK(!nb_fdinfo_exp_name(t3, strlen(t3), name, sizeof name), "fdinfo: absent");
    CHECK(!nb_fdinfo_exp_name(t4, strlen(t4), name, sizeof name), "fdinfo: empty name refused");
    CHECK(!nb_fdinfo_exp_name(t5, strlen(t5), name, sizeof name), "fdinfo: not the first line");
    CHECK(nb_fdinfo_exp_name(t6, strlen(t6), name, sizeof name) && strlen(name) == sizeof name - 1,
          "fdinfo: a long name is truncated, never overflowed");
    CHECK(!nb_fdinfo_exp_name(NULL, 0, name, sizeof name) && !nb_fdinfo_exp_name(t1, 0, name, sizeof name),
          "fdinfo: empty input");
    CHECK(nb_frame_exporter_ok("udmabuf") && nb_frame_exporter_ok("system"),
          "exporter: system memory accepted");
    CHECK(!nb_frame_exporter_ok("amdgpu") && !nb_frame_exporter_ok("nvidia") &&
          !nb_frame_exporter_ok("i915") && !nb_frame_exporter_ok("") &&
          !nb_frame_exporter_ok(NULL) && !nb_frame_exporter_ok("udmabuf2") &&
          !nb_frame_exporter_ok("UDMABUF"),
          "exporter: anything not known to be RAM refused");
    CHECK(nb_frame_layout_ok(NB_FRAME_MOD_LINEAR, "udmabuf"), "layout: LINEAR on a system exporter");
    CHECK(nb_frame_layout_ok(NB_FRAME_MOD_INVALID, "udmabuf"), "layout: the implicit modifier on a system exporter");
    CHECK(!nb_frame_layout_ok(0x0300000000606014ULL, "udmabuf"),
          "layout: an NVIDIA block-linear modifier is refused even on a system exporter");
    CHECK(!nb_frame_layout_ok(0x0200000000401b03ULL, "system"),
          "layout: an AMD tiled modifier is refused even on a system exporter");
    CHECK(!nb_frame_layout_ok(0x0100000000000001ULL, "udmabuf"),
          "layout: any other vendor modifier is refused");
    CHECK(!nb_frame_layout_ok(NB_FRAME_MOD_LINEAR, "amdgpu"),
          "layout: LINEAR on a GPU exporter is refused (VRAM reads are uncached)");
    CHECK(!nb_frame_layout_ok(NB_FRAME_MOD_INVALID, "nvidia"),
          "layout: the implicit modifier on a GPU exporter is refused");
}

/* ═════════════════════════════════════════════════════════════════════════
 * 4. THE FRAME READER ON A SEALED MEMFD
 * ════════════════════════════════════════════════════════════════════════ */

#define FW 200
#define FH 120
#define FSTRIDE (FW * 4 + 64)       /* padded rows, like a real pitch */
#define FOFFSET 128

static uint32_t frame_px(uint32_t x, uint32_t y)
{
    /* distinct, non-trivial, with a non-zero top byte (X is not special) */
    return 0x11000000u | ((x * 7u + y * 13u) & 0xffu) << 16 | ((x * 3u) & 0xffu) << 8 |
           ((y * 5u + 1u) & 0xffu);
}

static int make_frame_memfd(uint32_t w, uint32_t h, uint32_t stride, uint32_t offset,
                            bool seal, size_t *size_out)
{
    size_t size = (size_t)offset + (size_t)stride * h;
    int fd = memfd_create("test-frame", MFD_ALLOW_SEALING | MFD_CLOEXEC);
    uint8_t *m;
    uint32_t x, y;

    if (fd < 0 || ftruncate(fd, (off_t)size) < 0) {
        return -1;
    }
    m = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (m == MAP_FAILED) {
        return -1;
    }
    memset(m, 0xA5, size);
    for (y = 0; y < h; y++) {
        for (x = 0; x < w; x++) {
            uint32_t v = frame_px(x, y);
            uint8_t *p = m + offset + (size_t)y * stride + (size_t)x * 4u;

            p[0] = (uint8_t)v;
            p[1] = (uint8_t)(v >> 8);
            p[2] = (uint8_t)(v >> 16);
            p[3] = (uint8_t)(v >> 24);
        }
    }
    munmap(m, size);
    if (seal && fcntl(fd, F_ADD_SEALS, F_SEAL_SHRINK) < 0) {
        return -1;
    }
    *size_out = size;
    return fd;
}

#define XR24 0x34325258u
#define AR24 0x34325241u

static void test_frame_memfd(void)
{
    size_t size;
    struct nb_frame_geom g = { FW, FH, FSTRIDE, FOFFSET, XR24 };
    struct nb_frame f;
    uint32_t out[64 * 64];
    int fd = make_frame_memfd(FW, FH, FSTRIDE, FOFFSET, true, &size);
    uint32_t x, y;
    bool all;

    CHECK(fd >= 0, "memfd frame created");
    CHECK(nb_frame_open(&f, fd, true, size, &g, 0) && f.kind == NB_FRAME_SHM &&
          f.why == NB_FW_OK && f.sync_fd == -1, "memfd: sealed frame opens");
    CHECK(nb_frame_check(&f, &g) == NB_FW_OK, "memfd: check passes for the same description");
    /* read the whole rectangle and compare to the pattern */
    CHECK(nb_frame_read_rect(&f, 10, 20, 40, 30, out) == NB_FW_OK, "memfd: read a rectangle");
    all = true;
    for (y = 0; y < 30; y++) {
        for (x = 0; x < 40; x++) {
            if (out[y * 40 + x] != frame_px(10 + x, 20 + y)) { all = false; }
        }
    }
    CHECK(all, "memfd: every pixel read is the pixel written (padding, offset honoured)");
    /* the last pixel of the last row, and the first */
    CHECK(nb_frame_read_rect(&f, FW - 1, FH - 1, 1, 1, out) == NB_FW_OK &&
          out[0] == frame_px(FW - 1, FH - 1), "memfd: bottom-right pixel");
    CHECK(nb_frame_read_rect(&f, 0, 0, 1, 1, out) == NB_FW_OK && out[0] == frame_px(0, 0),
          "memfd: top-left pixel");
    /* out of range */
    CHECK(nb_frame_read_rect(&f, FW - 1, 0, 2, 1, out) == NB_FW_RANGE, "memfd: one column past the right edge refused");
    CHECK(nb_frame_read_rect(&f, 0, FH - 1, 1, 2, out) == NB_FW_RANGE, "memfd: one row past the bottom refused");
    CHECK(nb_frame_read_rect(&f, 0, 0, 0, 1, out) == NB_FW_RANGE &&
          nb_frame_read_rect(&f, 0, 0, 1, 0, out) == NB_FW_RANGE, "memfd: an empty rectangle refused");
    CHECK(nb_frame_read_rect(&f, 0, 0, NVKVM_BROKER_CURSOR_MAX_DIM + 1, 1, out) == NB_FW_RANGE,
          "memfd: a rectangle wider than any cursor refused");
    CHECK(nb_frame_read_rect(&f, UINT32_MAX, 0, 2, 1, out) == NB_FW_RANGE &&
          nb_frame_read_rect(&f, 0, UINT32_MAX - 1, 1, 4, out) == NB_FW_RANGE,
          "memfd: coordinates that would wrap are refused");
    /* the frame was replaced / resized: the description no longer matches */
    {
        struct nb_frame_geom bigger = { FW + 8, FH, FSTRIDE, FOFFSET, XR24 };
        struct nb_frame_geom taller = { FW, FH + 8, FSTRIDE, FOFFSET, XR24 };
        struct nb_frame_geom other = { FW, FH, FSTRIDE + 4, FOFFSET, XR24 };
        struct nb_frame_geom ofs = { FW, FH, FSTRIDE, FOFFSET + 4, XR24 };
        struct nb_frame_geom fmt = { FW, FH, FSTRIDE, FOFFSET, AR24 };

        CHECK(nb_frame_check(&f, &bigger) == NB_FW_STALE, "memfd: wider description is STALE");
        CHECK(nb_frame_check(&f, &taller) == NB_FW_STALE, "memfd: taller description is STALE");
        CHECK(nb_frame_check(&f, &other) == NB_FW_STALE, "memfd: other stride is STALE");
        CHECK(nb_frame_check(&f, &ofs) == NB_FW_STALE, "memfd: other offset is STALE");
        CHECK(nb_frame_check(&f, &fmt) == NB_FW_STALE, "memfd: other format is STALE");
    }
    /* a sealed memfd cannot be shrunk under the mapping (why a read cannot SIGBUS) */
    CHECK(ftruncate(fd, 0) < 0 && errno == EPERM, "memfd: the seal forbids shrinking (no SIGBUS)");
    nb_frame_close(&f);
    CHECK(f.kind == NB_FRAME_NONE && f.map == NULL, "memfd: close clears");
    nb_frame_close(&f);     /* twice is harmless */
    CHECK(nb_frame_read_rect(&f, 0, 0, 1, 1, out) != NB_FW_OK, "memfd: a closed frame reads nothing");

    /* AR24 opens too (same bytes) */
    g.fourcc = AR24;
    CHECK(nb_frame_open(&f, fd, true, size, &g, 0), "memfd: AR24 opens");
    nb_frame_close(&f);
    g.fourcc = XR24;

    /* hostile descriptions, refused before anything is mapped */
    {
        struct nb_frame_geom bad;

        bad = g; bad.fourcc = 0x3231564e;       /* NV12 */
        CHECK(!nb_frame_open(&f, fd, true, size, &bad, 0) && f.why == NB_FW_FORMAT, "open: NV12 refused");
        bad = g; bad.h = FH + 1;
        CHECK(!nb_frame_open(&f, fd, true, size, &bad, 0) && f.why == NB_FW_GEOMETRY,
              "open: a description larger than the buffer refused");
        bad = g; bad.offset = (uint32_t)size;
        CHECK(!nb_frame_open(&f, fd, true, size, &bad, 0) && f.why == NB_FW_GEOMETRY, "open: offset past the end");
        bad = g; bad.stride = UINT32_MAX;
        CHECK(!nb_frame_open(&f, fd, true, size, &bad, 0) && f.why == NB_FW_GEOMETRY, "open: huge stride (no wrap)");
        bad = g; bad.w = UINT32_MAX; bad.stride = UINT32_MAX;
        CHECK(!nb_frame_open(&f, fd, true, size, &bad, 0) && f.why == NB_FW_GEOMETRY, "open: huge width");
        bad = g; bad.w = 0;
        CHECK(!nb_frame_open(&f, fd, true, size, &bad, 0) && f.why == NB_FW_GEOMETRY, "open: zero width");
        bad = g; bad.stride = FW * 4 - 4;
        CHECK(!nb_frame_open(&f, fd, true, size, &bad, 0) && f.why == NB_FW_GEOMETRY, "open: stride < w*4");
        bad = g; bad.w = NVKVM_BROKER_MAX_DIM + 1;
        CHECK(!nb_frame_open(&f, fd, true, size, &bad, 0) && f.why == NB_FW_GEOMETRY, "open: wider than any frame");
        CHECK(!nb_frame_open(&f, fd, true, 0, &g, 0) && f.why == NB_FW_GEOMETRY, "open: a size of 0");
    }
    close(fd);

    /* an UNSEALED memfd: could be truncated under the reader -> refused */
    fd = make_frame_memfd(FW, FH, FSTRIDE, FOFFSET, false, &size);
    CHECK(!nb_frame_open(&f, fd, true, size, &g, 0) && f.why == NB_FW_NOSEAL && f.kind == NB_FRAME_NONE,
          "open: an unsealed memfd is not readable (NOSEAL)");
    CHECK(nb_frame_check(&f, &g) == NB_FW_NOSEAL, "check: an unreadable frame reports why");
    CHECK(nb_frame_read_rect(&f, 0, 0, 1, 1, out) == NB_FW_NOSEAL, "read: an unreadable frame reads nothing");
    close(fd);

    /* a plain file / pipe passed as shm */
    {
        int p[2];

        CHECK(pipe(p) == 0, "pipe");
        CHECK(!nb_frame_open(&f, p[0], true, 1u << 20, &g, 0) && f.why == NB_FW_NOSEAL,
              "open: a pipe is not a sealed memfd");
        close(p[0]);
        close(p[1]);
    }
    /* a memfd presented as a dma-buf (what the other tier would do) */
    fd = make_frame_memfd(FW, FH, FSTRIDE, FOFFSET, true, &size);
    CHECK(!nb_frame_open(&f, fd, false, size, &g, 0) && (f.why == NB_FW_FDINFO || f.why == NB_FW_EXPORTER),
          "open: a memfd claiming to be a dma-buf is refused (no exp_name)");
    CHECK(!nb_frame_open(&f, fd, false, size, &g, 0x0300000000606014ULL) && f.why == NB_FW_MODIFIER,
          "open: a tiled modifier is refused before anything is read");
    close(fd);

    /* mapping failure: the address space is exhausted -> NB_FW_MMAP, in a child */
    fd = make_frame_memfd(FW, FH, FSTRIDE, FOFFSET, true, &size);
    {
        pid_t pid = fork();

        if (pid == 0) {
            struct rlimit rl;
            struct nb_frame c;
            struct nb_frame_geom big = { 8192, 8192, 8192 * 4, 0, XR24 };
            int fd2;
            size_t sz2;

            /* a sealed memfd large enough that mapping it needs 256 MiB ... */
            fd2 = make_frame_memfd(8192, 8192, 8192 * 4, 0, true, &sz2);
            if (fd2 < 0) { _exit(2); }
            getrlimit(RLIMIT_AS, &rl);
            /* ... with ~16 MiB of headroom left */
            {
                struct rlimit lim;
                FILE *st = fopen("/proc/self/statm", "r");
                unsigned long pages = 0;

                if (st) { if (fscanf(st, "%lu", &pages) != 1) { pages = 0; } fclose(st); }
                lim.rlim_cur = lim.rlim_max = (rlim_t)pages * 4096 + (16u << 20);
                if (setrlimit(RLIMIT_AS, &lim) < 0) { _exit(3); }
            }
            _exit(nb_frame_open(&c, fd2, true, sz2, &big, 0) ? 1
                  : c.why == NB_FW_MMAP && c.kind == NB_FRAME_NONE ? 0 : 4);
        }
        {
            int st = 0;

            waitpid(pid, &st, 0);
            CHECK(WIFEXITED(st) && WEXITSTATUS(st) == 0,
                  "open: a mapping that fails -> not readable, NB_FW_MMAP (child status %d)", st);
        }
    }
    close(fd);

    /* the description and the mapping disagree: a hand-built frame whose
     * mapping is ONE page followed by a PROT_NONE guard page.  A read that
     * trusts the description instead of the mapping faults; ours refuses.
     * Each probe runs in its own child, so a fault is a FAIL line, not the end
     * of the run. */
    {
        static const struct { uint32_t x, y, w, h; const char *what; } probes[] = {
            { 0, 50, 8, 8, "rows past the mapping" },
            { 90, 99, 8, 1, "the far corner" },
            { 0, 12, 100, 1, "the first row beyond the mapped page" },
        };
        unsigned k;

        for (k = 0; k < sizeof probes / sizeof probes[0]; k++) {
            pid_t pid = fork();
            int st = 0;

            if (pid == 0) {
                uint8_t *region = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
                struct nb_frame h;
                uint32_t o[128 * 8];

                mprotect(region + 4096, 4096, PROT_NONE);
                memset(&h, 0, sizeof h);
                h.kind = NB_FRAME_SHM;
                h.sync_fd = -1;
                h.map = region;
                h.len = 4096;
                h.w = 100; h.h = 100; h.stride = 400; h.offset = 0; h.fourcc = XR24;   /* claims 40000 bytes */
                _exit(nb_frame_read_rect(&h, probes[k].x, probes[k].y, probes[k].w, probes[k].h, o) == NB_FW_RANGE ? 0 : 1);
            }
            waitpid(pid, &st, 0);
            CHECK(WIFEXITED(st) && WEXITSTATUS(st) == 0,
                  "guard: a description that outruns its mapping is refused, not read (%s)", probes[k].what);
        }
        {
            uint8_t *region = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            struct nb_frame h;
            struct nb_frame_geom hg = { 100, 100, 400, 0, XR24 };
            uint32_t o[16 * 16];

            mprotect(region + 4096, 4096, PROT_NONE);
            memset(&h, 0, sizeof h);
            h.kind = NB_FRAME_SHM; h.sync_fd = -1; h.map = region; h.len = 4096;
            h.w = 100; h.h = 100; h.stride = 400; h.offset = 0; h.fourcc = XR24;
            CHECK(nb_frame_read_rect(&h, 0, 0, 8, 8, o) == NB_FW_OK, "guard: rows inside the mapping are fine");
            CHECK(nb_frame_check(&h, &hg) == NB_FW_GEOMETRY, "guard: check sees the description exceeds the mapping");
            munmap(region, 8192);
        }
    }
    /* a wide rectangle runs into the next row / the padding: refused by the frame's width */
    {
        struct nb_frame f2;
        int fd3 = make_frame_memfd(FW, FH, FSTRIDE, FOFFSET, true, &size);
        struct nb_frame_geom g3 = { FW, FH, FSTRIDE, FOFFSET, XR24 };
        uint32_t big[(FW + 16) * 4];

        (void)nb_frame_open(&f2, fd3, true, size, &g3, 0);
        CHECK(nb_frame_read_rect(&f2, 0, 5, FW + 10, 1, big) == NB_FW_RANGE,
              "memfd: a rectangle wider than the frame (runs into the padding) refused");
        CHECK(nb_frame_read_rect(&f2, FW - 5, 5, 6, 1, big) == NB_FW_RANGE,
              "memfd: a rectangle that starts inside and ends one pixel outside refused");
        CHECK(nb_frame_read_rect(&f2, FW - 5, 5, 5, 3, big) == NB_FW_OK,
              "memfd: ... exactly to the edge is fine");
        /* read-only: the mapping is r--s, and a store faults */
        {
            char needle[64], line[512];
            FILE *m = fopen("/proc/self/maps", "r");
            bool found = false, ro = false;

            snprintf(needle, sizeof needle, "%lx-", (unsigned long)(uintptr_t)f2.map);
            while (m && fgets(line, sizeof line, m)) {
                if (strncmp(line, needle, strlen(needle)) == 0) {
                    found = true;
                    ro = strstr(line, " r--s ") != NULL;
                }
            }
            if (m) { fclose(m); }
            CHECK(found && ro, "memfd: the mapping is read-only and shared (r--s) [%s]", found ? "found" : "not found");
        }
        {
            pid_t pid = fork();
            int st = 0;

            if (pid == 0) {
                signal(SIGSEGV, SIG_DFL);   /* a sanitizer's handler would exit instead */
                *(volatile uint8_t *)(uintptr_t)f2.map = 1;
                _exit(0);
            }
            waitpid(pid, &st, 0);
            CHECK(WIFSIGNALED(st) && WTERMSIG(st) == SIGSEGV, "memfd: a store through the mapping faults (read-only)");
        }
        nb_frame_close(&f2);
        close(fd3);
    }
}

/* ═════════════════════════════════════════════════════════════════════════
 * 5. A REAL UDMABUF
 * ════════════════════════════════════════════════════════════════════════ */

static void test_frame_udmabuf(void)
{
    size_t size;
    int ud = open("/dev/udmabuf", O_RDWR | O_CLOEXEC);
    int mfd, dfd;
    struct udmabuf_create c;
    struct nb_frame f;
    struct nb_frame_geom g = { FW, FH, 4096, 0, XR24 };
    uint32_t out[32 * 32];
    uint32_t x, y;
    bool all;

    if (ud < 0) {
        printf("SKIP udmabuf: /dev/udmabuf unavailable (%s)\n", strerror(errno));
        return;
    }
    /* udmabuf wants page-multiple memfd extents */
    mfd = make_frame_memfd(FW, FH, 4096, 0, false, &size);
    CHECK(mfd >= 0, "udmabuf: backing memfd");
    size = (size_t)4096 * FH;
    if (fcntl(mfd, F_ADD_SEALS, F_SEAL_SHRINK) < 0) {
        printf("SKIP udmabuf: cannot seal (%s)\n", strerror(errno));
        return;
    }
    memset(&c, 0, sizeof c);
    c.memfd = (uint32_t)mfd;
    c.flags = UDMABUF_FLAGS_CLOEXEC;
    c.size = size;
    dfd = ioctl(ud, UDMABUF_CREATE, &c);
    if (dfd < 0) {
        printf("SKIP udmabuf: UDMABUF_CREATE: %s\n", strerror(errno));
        return;
    }
    CHECK(nb_frame_open(&f, dfd, false, size, &g, NB_FRAME_MOD_LINEAR) &&
          f.kind == NB_FRAME_DMABUF && f.sync_fd >= 0 && f.sync_fd != dfd &&
          !strcmp(f.exporter, "udmabuf"), "udmabuf: LINEAR dma-buf opens (exporter %s)", f.exporter);
    CHECK(nb_frame_read_rect(&f, 30, 40, 20, 10, out) == NB_FW_OK, "udmabuf: read a rectangle (sync START/END)");
    all = true;
    for (y = 0; y < 10; y++) {
        for (x = 0; x < 20; x++) {
            if (out[y * 20 + x] != frame_px(30 + x, 40 + y)) { all = false; }
        }
    }
    CHECK(all, "udmabuf: the pixels read are the pixels the writer put in the memfd");
    /* a write through the memfd after the map is seen by the next read (coherent) */
    {
        uint8_t *w = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, mfd, 0);

        if (w != MAP_FAILED) {
            w[(size_t)40 * 4096 + 30 * 4] = 0x77;
            munmap(w, size);
            CHECK(nb_frame_read_rect(&f, 30, 40, 1, 1, out) == NB_FW_OK && (out[0] & 0xff) == 0x77,
                  "udmabuf: a later write by the producer is visible to the next read");
        }
    }
    CHECK(nb_frame_check(&f, &g) == NB_FW_OK, "udmabuf: check");
    {
        pid_t pid = fork();
        int st = 0;

        if (pid == 0) {
            signal(SIGSEGV, SIG_DFL);
            *(volatile uint8_t *)(uintptr_t)f.map = 1;
            _exit(0);
        }
        waitpid(pid, &st, 0);
        CHECK(WIFSIGNALED(st) && WTERMSIG(st) == SIGSEGV, "udmabuf: a store through the mapping faults (read-only)");
    }
    nb_frame_close(&f);
    CHECK(fcntl(dfd, F_GETFD) >= 0, "udmabuf: closing the frame leaves the caller's fd");
    CHECK(nb_frame_open(&f, dfd, false, size, &g, NB_FRAME_MOD_INVALID), "udmabuf: the implicit modifier opens");
    nb_frame_close(&f);
    CHECK(!nb_frame_open(&f, dfd, false, size, &g, 0x0300000000606014ULL) && f.why == NB_FW_MODIFIER,
          "udmabuf: an NVIDIA block-linear modifier is refused");
    {
        struct nb_frame_geom too_big = { FW, FH + 50, 4096, 0, XR24 };

        CHECK(!nb_frame_open(&f, dfd, false, size, &too_big, 0) && f.why == NB_FW_GEOMETRY,
              "udmabuf: a description past the buffer is refused");
    }
    close(dfd);
    close(mfd);
    close(ud);
}

/* ═════════════════════════════════════════════════════════════════════════
 * 6. THE DRIVER ON FRAMES READ FROM A MEMFD, AGAINST THE KERNEL'S FORMULA
 * ════════════════════════════════════════════════════════════════════════ */

/* compose_reference's XOR branch, per pixel, then the patch's opaque wrap */
static uint32_t ref_pixel(uint32_t below, uint32_t m)
{
    return (m >> 24) == 0 ? 0 : 0xff000000u | ((below ^ (m & 0x00ffffffu)) & 0x00ffffffu);
}

static bool px_eq(const uint32_t *a, const uint32_t *b, size_t n_px)
{
    return a && b && memcmp(a, b, n_px * 4u) == 0;
}

static struct nb_cursor *new_cursor(uint32_t w, uint32_t h, uint32_t hx, uint32_t hy)
{
    struct nb_cursor *c = calloc(1, sizeof *c);

    c->defined = true;
    c->visible = true;
    c->w = w;
    c->h = h;
    c->hot_x = hx;
    c->hot_y = hy;
    c->gen = 1;
    return c;
}

/* set the mask from `on` pixels with colour `col` (alpha 0xff), the rest clear,
 * then do exactly what the SET path does */
static void set_mask(struct nb_cursor *c, const uint32_t *px)
{
    memcpy(c->px, px, (size_t)c->w * c->h * 4u);
    (void)nb_cursor_set_xor(c);
}

struct fx {
    struct nb_xor_ctx *ctx;
    struct nb_frame f;
    struct nb_frame_geom g;
    int fd;
    size_t size;
};

static void fx_open(struct fx *x, uint32_t w, uint32_t h)
{
    x->ctx = calloc(1, sizeof *x->ctx);
    x->g = (struct nb_frame_geom){ w, h, w * 4 + 64, FOFFSET, XR24 };
    x->fd = make_frame_memfd(w, h, x->g.stride, x->g.offset, true, &x->size);
    (void)nb_frame_open(&x->f, x->fd, true, x->size, &x->g, 0);
}

static void fx_close(struct fx *x)
{
    nb_frame_close(&x->f);
    close(x->fd);
    free(x->ctx);
}

static void eval_hover(struct fx *x, const struct nb_cursor *c, int px, int py,
                       struct nb_xor_res *r)
{
    struct nb_xor_view v;

    memset(&v, 0, sizeof v);
    v.cur = c;
    v.shown_ok = true;
    v.ptr_inside = true;
    v.serial_ok = true;
    v.ptr_known = true;
    v.ptr_x = px;
    v.ptr_y = py;
    v.frame = &x->f;
    v.geom = x->g;
    nb_xor_eval(x->ctx, &v, r);
}

static void eval_grab(struct fx *x, const struct nb_cursor *c, struct nb_xor_res *r)
{
    struct nb_xor_view v;

    memset(&v, 0, sizeof v);
    v.cur = c;
    v.shown_ok = true;
    v.grabbed = true;
    v.ptr_inside = true;
    v.serial_ok = true;
    v.frame = &x->f;
    v.geom = x->g;
    nb_xor_eval(x->ctx, &v, r);
}

/* the full-frame reference patch of `c` at top-left (tlx,tly), cropped to its bbox */
static size_t ref_bbox_patch(const struct nb_cursor *c, int tlx, int tly, uint32_t fw,
                             uint32_t fh, uint32_t *out)
{
    uint32_t ix, iy;
    size_t n = 0;

    for (iy = 0; iy < c->bh; iy++) {
        for (ix = 0; ix < c->bw; ix++) {
            uint32_t m = c->mask[(size_t)(c->by + iy) * c->w + (c->bx + ix)];
            int fx = tlx + (int)c->bx + (int)ix, fy = tly + (int)c->by + (int)iy;
            uint32_t v = 0;

            if ((m >> 24) != 0 && fx >= 0 && fy >= 0 && fx < (int)fw && fy < (int)fh) {
                v = ref_pixel(frame_px((uint32_t)fx, (uint32_t)fy), m);
                n++;
            }
            out[(size_t)iy * c->bw + ix] = v;
        }
    }
    return n;
}

static void test_compose(void)
{
    struct fx x;
    struct nb_cursor *c = new_cursor(16, 16, 4, 8);
    struct nb_xor_res r;
    uint32_t mask[16 * 16];
    uint32_t want[16 * 16];
    uint32_t i;
    size_t n;

    fx_open(&x, FW, FH);

    /* an I-beam-ish mask: stem + serifs, white */
    memset(mask, 0, sizeof mask);
    for (i = 3; i < 13; i++) { mask[i * 16 + 7] = 0xffffffffu; }
    for (i = 4; i <= 10; i++) { mask[3 * 16 + i] = 0xffffffffu; mask[12 * 16 + i] = 0xffffffffu; }
    set_mask(c, mask);
    CHECK(c->xor && c->bw == 7 && c->bh == 10 && c->bx == 4 && c->by == 3,
          "compose: the mask's bounding box (%u,%u %ux%u)", c->bx, c->by, c->bw, c->bh);

    /* white mask over the pattern: the patch is the inversion */
    eval_hover(&x, c, 100, 60, &r);
    CHECK(r.act.ov == NB_XO_XOR && r.act.ptr == NB_XP_HIDDEN && r.act.choice == NB_XC_COMPOSED && r.patch,
          "compose: hover, readable: composed");
    n = ref_bbox_patch(c, 100 - 4, 60 - 8, FW, FH, want);
    CHECK(r.npix == n && px_eq(r.px, want, (size_t)c->bw * c->bh),
          "compose: white mask: patch == compose_reference below^colour (n %u vs %zu)", r.npix, n);
    CHECK(r.tlx == 100 - 4 + 4 && r.tly == 60 - 8 + 3 && r.pw == 7 && r.ph == 10,
          "compose: patch origin is pointer - hot + bbox (%d,%d %ux%u)", r.tlx, r.tly, r.pw, r.ph);
    CHECK(r.choice_changed, "compose: the first eval announces the choice");
    CHECK(r.patch_changed, "compose: nothing attached yet -> changed");
    nb_xor_attached(x.ctx, &r, c->gen);
    eval_hover(&x, c, 100, 60, &r);
    CHECK(!r.patch_changed && !r.choice_changed, "compose: the same again: nothing changed, nothing announced");
    eval_hover(&x, c, 101, 60, &r);
    CHECK(r.tlx == 101 && !r.choice_changed, "compose: a pointer move moves the patch");
    n = ref_bbox_patch(c, 101 - 4, 60 - 8, FW, FH, want);
    CHECK(r.npix == n && px_eq(r.px, want, (size_t)c->bw * c->bh), "compose: ... and is recomputed there");
    /* the pixels under the patch changed with the move: content changed -> a new buffer is owed */
    CHECK(r.patch_changed, "compose: new pixels under it -> patch_changed");

    /* black mask (alpha set, colour 0): inverts nothing visible but is a present pixel */
    for (i = 0; i < 256; i++) { mask[i] = mask[i] ? 0xff000000u : 0; }
    set_mask(c, mask);
    eval_hover(&x, c, 100, 60, &r);
    n = ref_bbox_patch(c, 100 - 4, 60 - 8, FW, FH, want);
    CHECK(r.act.ov == NB_XO_XOR && px_eq(r.px, want, (size_t)c->bw * c->bh) &&
          r.px && (r.px[0] & 0xffffff) == (frame_px(100 - 4 + 4 + 0, 60 - 8 + 3) & 0xffffff),
          "compose: black mask: colour 0 leaves the frame's pixel (below ^ 0)");

    /* a colour: per-channel XOR */
    for (i = 0; i < 256; i++) { mask[i] = mask[i] ? 0xff2468acu : 0; }
    set_mask(c, mask);
    eval_hover(&x, c, 100, 60, &r);
    n = ref_bbox_patch(c, 100 - 4, 60 - 8, FW, FH, want);
    CHECK(px_eq(r.px, want, (size_t)c->bw * c->bh) && r.npix == n,
          "compose: colour 0x2468ac: per-channel xor");
    {
    }

    /* alpha 0 pixels never change anything, whatever colour they carry (clamped) */
    for (i = 0; i < 256; i++) { mask[i] = 0; }
    mask[5 * 16 + 5] = 0xffffffffu;
    mask[6 * 16 + 6] = 0x00ffffffu;     /* alpha 0 with colour: not a selected pixel */
    set_mask(c, mask);
    eval_hover(&x, c, 100, 60, &r);
    CHECK(c->bw == 1 && c->bh == 1 && r.pw == 1 && r.npix == 1,
          "compose: alpha 0 pixels are not part of the cursor (bbox %ux%u, n %u)", c->bw, c->bh, r.npix);
    CHECK(r.px && r.px[0] == ref_pixel(frame_px(100 - 4 + 5, 60 - 8 + 5), 0xffffffffu), "compose: the single pixel");

    /* the edges: the cursor straddling every side of the frame */
    memset(mask, 0, sizeof mask);
    for (i = 0; i < 256; i++) { mask[i] = 0xffffffffu; }
    set_mask(c, mask);
    {
        static const int pos[][2] = {
            { 0, 0 }, { 4, 8 }, { FW - 1, FH - 1 }, { FW - 1, 0 }, { 0, FH - 1 }, { 2, 60 },
            { FW - 3, 60 }, { 100, 1 }, { 100, FH - 2 }, { -50 + 4, 60 },
        };
        unsigned k;

        for (k = 0; k < sizeof pos / sizeof pos[0]; k++) {
            eval_hover(&x, c, pos[k][0], pos[k][1], &r);
            n = ref_bbox_patch(c, pos[k][0] - 4, pos[k][1] - 8, FW, FH, want);
            CHECK(r.act.ov == NB_XO_XOR && r.npix == n &&
                  px_eq(r.px, want, (size_t)c->bw * c->bh),
                  "compose: edge case pointer (%d,%d): clipped like the kernel (n %u vs %zu)",
                  pos[k][0], pos[k][1], r.npix, n);
        }
    }
    /* grab: placed by the MOVE (c->x, c->y), negative included, wholly off the frame */
    c->placed = true;
    c->x = -6; c->y = 110;
    eval_grab(&x, c, &r);
    n = ref_bbox_patch(c, -6, 110, FW, FH, want);
    CHECK(r.act.ov == NB_XO_XOR && r.npix == n && n > 0 &&
          px_eq(r.px, want, (size_t)c->bw * c->bh), "compose: grab, partly off the frame");
    c->x = FW + 40; c->y = 10;
    eval_grab(&x, c, &r);
    CHECK(r.act.choice == NB_XC_COMPOSED && r.npix == 0 && r.patch,
          "compose: grab, wholly off the frame: composed, nothing to show (npix %u)", r.npix);
    c->x = -400; c->y = -400;
    eval_grab(&x, c, &r);
    CHECK(r.npix == 0 && r.act.choice == NB_XC_COMPOSED, "compose: far off to the top-left: nothing");

    /* a RESIZED frame: the description changes under the same cursor */
    {
        struct nb_xor_view v;
        struct fx y2;
        uint32_t k;

        fx_open(&y2, FW / 2, FH / 2);
        memset(&v, 0, sizeof v);
        v.cur = c; v.shown_ok = true; v.ptr_inside = true; v.serial_ok = true; v.ptr_known = true;
        v.ptr_x = 40; v.ptr_y = 30;
        v.frame = &x.f;             /* the OLD mapping ... */
        v.geom = y2.g;              /* ... described as the NEW size */
        nb_xor_eval(x.ctx, &v, &r);
        CHECK(r.act.choice == NB_XC_APPROX && r.why == NB_FW_STALE && r.act.ov == NB_XO_OFF &&
              r.act.ptr == NB_XP_GUEST_IMAGE,
              "resize: a mapping that no longer matches the frame -> fallback, reason STALE");
        CHECK(r.choice_changed, "resize: the change of choice is announced");
        v.frame = &y2.f;            /* the new frame, its own mapping */
        nb_xor_eval(x.ctx, &v, &r);
        n = ref_bbox_patch(c, 40 - 4, 30 - 8, FW / 2, FH / 2, want);
        CHECK(r.act.choice == NB_XC_COMPOSED && r.npix == n &&
              px_eq(r.px, want, (size_t)c->bw * c->bh),
              "resize: the new frame composes against ITS pixels");
        for (k = 0; k < 3; k++) {
            nb_xor_eval(x.ctx, &v, &r);
            CHECK(!r.choice_changed, "resize: steady state announces nothing (%u)", k);
        }
        fx_close(&y2);
    }

    /* a frame that cannot be mapped (unsealed memfd): fallback, ONE announcement */
    {
        struct fx u;
        struct nb_xor_view v;
        unsigned k, changes = 0;

        u.ctx = calloc(1, sizeof *u.ctx);
        u.g = (struct nb_frame_geom){ FW, FH, FSTRIDE, FOFFSET, XR24 };
        u.fd = make_frame_memfd(FW, FH, FSTRIDE, FOFFSET, false, &u.size);
        (void)nb_frame_open(&u.f, u.fd, true, u.size, &u.g, 0);
        memset(&v, 0, sizeof v);
        v.cur = c; v.shown_ok = true; v.ptr_inside = true; v.serial_ok = true; v.ptr_known = true;
        v.ptr_x = 50; v.ptr_y = 50; v.frame = &u.f; v.geom = u.g;
        for (k = 0; k < 500; k++) {
            nb_xor_eval(u.ctx, &v, &r);
            changes += r.choice_changed;
            CHECK(r.act.choice == NB_XC_APPROX && r.why == NB_FW_NOSEAL && r.act.ov == NB_XO_OFF &&
                  r.act.ptr == NB_XP_GUEST_IMAGE, "unreadable: approximation, reason NOSEAL (iteration %u)", k);
            if (k > 3) { break; }
        }
        for (k = 0; k < 500; k++) {
            nb_xor_eval(u.ctx, &v, &r);
            changes += r.choice_changed;
        }
        CHECK(changes == 1, "unreadable: 505 evaluations, exactly ONE change of choice (%u)", changes);
        CHECK(u.ctx->n_fallback > 0 && u.ctx->n_composed == 0, "unreadable: counters");
        /* the frame becomes readable (a later, sealed buffer): one more change, then quiet */
        {
            struct fx s2;

            fx_open(&s2, FW, FH);
            v.frame = &s2.f;
            v.geom = s2.g;
            nb_xor_eval(u.ctx, &v, &r);
            CHECK(r.act.choice == NB_XC_COMPOSED && r.choice_changed, "unreadable -> readable: composed, announced");
            nb_xor_eval(u.ctx, &v, &r);
            CHECK(!r.choice_changed, "... and said once");
            fx_close(&s2);
        }
        nb_frame_close(&u.f);
        close(u.fd);
        free(u.ctx);
    }
    /* the READ fails after the frame looked fine (DMA_BUF_IOCTL_SYNC refused):
     * the frame is not readable after all -> the approximation, once */
    {
        struct fx s3;
        struct nb_frame hb;
        struct nb_xor_view v;
        struct nb_xor_ctx *ctx = calloc(1, sizeof *ctx);
        unsigned changes = 0, k;

        fx_open(&s3, FW, FH);
        hb = s3.f;
        hb.kind = NB_FRAME_DMABUF;      /* looks like a dma-buf ... */
        hb.sync_fd = -1;                /* ... whose sync ioctl fails (EBADF) */
        memset(&v, 0, sizeof v);
        v.cur = c; v.shown_ok = true; v.ptr_inside = true; v.serial_ok = true; v.ptr_known = true;
        v.ptr_x = 60; v.ptr_y = 60; v.frame = &hb; v.geom = s3.g;
        for (k = 0; k < 50; k++) {
            nb_xor_eval(ctx, &v, &r);
            changes += r.choice_changed;
        }
        CHECK(r.act.choice == NB_XC_APPROX && r.why == NB_FW_SYNC,
              "sync failure: the read fails -> approximation, reason SYNC (got choice %s why %d)",
              nb_xor_choice_text(r.act.choice), r.why);
        CHECK(r.act.ptr == NB_XP_GUEST_IMAGE && r.act.ov == NB_XO_OFF && !r.patch,
              "sync failure: the host pointer is restored and no patch is offered");
        CHECK(changes == 1, "sync failure: one announcement in 50 evaluations (%u)", changes);
        free(ctx);
        fx_close(&s3);
    }
    /* no frame at all (placeholder, first run) */
    {
        struct nb_xor_view v;
        struct nb_xor_ctx *ctx = calloc(1, sizeof *ctx);

        memset(&v, 0, sizeof v);
        v.cur = c; v.shown_ok = true; v.ptr_inside = true; v.serial_ok = true; v.ptr_known = true;
        nb_xor_eval(ctx, &v, &r);
        CHECK(r.act.choice == NB_XC_APPROX && r.why == NB_FW_NONE && r.act.ptr == NB_XP_GUEST_IMAGE,
              "no frame: approximation, reason NONE");
        free(ctx);
    }
    /* the line it earns */
    {
        char line[256];

        eval_hover(&x, c, 100, 60, &r);
        r.act.choice = NB_XC_COMPOSED;
        nb_xor_choice_line(&r, &x.f, line, sizeof line);
        CHECK(strstr(line, "XOR composed by the broker") != NULL, "line: composed text (%s)", line);
        r.act.choice = NB_XC_APPROX;
        r.why = NB_FW_EXPORTER;
        nb_xor_choice_line(&r, &x.f, line, sizeof line);
        CHECK(strstr(line, "XOR approximated") && strstr(line, "frame not readable") &&
              strstr(line, "system memory"), "line: approximated text carries the reason (%s)", line);
    }
    /* starvation: a patch that could not be attached is still owed next time */
    {
        c->placed = true; c->x = 20; c->y = 20;
        eval_grab(&x, c, &r);
        CHECK(r.patch_changed, "owed: not attached -> changed");
        eval_grab(&x, c, &r);
        CHECK(r.patch_changed, "owed: still changed until nb_xor_attached() says it is up");
        nb_xor_attached(x.ctx, &r, c->gen);
        eval_grab(&x, c, &r);
        CHECK(!r.patch_changed, "owed: attached -> quiet");
        c->x = 21;
        eval_grab(&x, c, &r);
        CHECK(r.patch_changed, "owed: one pixel along, different pixels beneath -> a new buffer is owed");
        nb_xor_attached(x.ctx, &r, c->gen);
        eval_grab(&x, c, &r);
        CHECK(!r.patch_changed, "owed: ... and quiet once that is up");
        c->x = 22;
        eval_grab(&x, c, &r);
        CHECK(r.patch_changed && r.tlx == 22 + (int)c->bx, "owed: and again after the next step");
        nb_xor_detached(x.ctx);
        eval_grab(&x, c, &r);
        CHECK(r.patch_changed, "owed: a detached overlay owes its patch again");
    }
    free(c);
    fx_close(&x);
}

/* a random differential against the full-frame reference, hostile geometry included */
static void test_differential(void)
{
    struct fx x;
    struct nb_cursor *c = new_cursor(1, 1, 0, 0);
    struct nb_xor_res r;
    uint32_t want[NVKVM_BROKER_CURSOR_MAX_DIM * NVKVM_BROKER_CURSOR_MAX_DIM];
    unsigned it, bad = 0, composed = 0;

    fx_open(&x, FW, FH);
    for (it = 0; it < 3000; it++) {
        uint32_t w = 1 + rnd() % (it % 50 == 0 ? 256 : 24), h = 1 + rnd() % (it % 50 == 0 ? 256 : 24);
        uint32_t i;
        int px, py, tlx, tly;
        size_t n;

        c->w = w; c->h = h;
        c->hot_x = rnd() % w; c->hot_y = rnd() % h;
        c->gen = it + 2;
        for (i = 0; i < w * h; i++) {
            uint32_t m = rnd();

            c->px[i] = (rnd() & 3) ? 0 : ((m & 0x00ffffffu) | 0xff000000u);
        }
        if (!nb_cursor_set_xor(c)) {
            continue;
        }
        px = (int)(rnd() % (FW + 40)) - 20;
        py = (int)(rnd() % (FH + 40)) - 20;
        if (it & 1) {
            c->placed = true;
            c->x = (int32_t)(rnd() % (FW + 600)) - 300;
            c->y = (int32_t)(rnd() % (FH + 600)) - 300;
            eval_grab(&x, c, &r);
            tlx = c->x; tly = c->y;
        } else {
            eval_hover(&x, c, px < 0 ? 0 : px >= FW ? FW - 1 : px, py < 0 ? 0 : py >= FH ? FH - 1 : py, &r);
            tlx = (px < 0 ? 0 : px >= FW ? FW - 1 : px) - (int)c->hot_x;
            tly = (py < 0 ? 0 : py >= FH ? FH - 1 : py) - (int)c->hot_y;
        }
        n = ref_bbox_patch(c, tlx, tly, FW, FH, want);
        if (r.act.choice != NB_XC_COMPOSED ||
            (r.patch && (r.npix != n || memcmp(r.px, want, (size_t)c->bw * c->bh * 4) != 0)) ||
            (!r.patch && n != 0 && r.act.ov != NB_XO_OFF)) {
            bad++;
        }
        if (r.patch && r.npix) { composed++; }
        /* the full-frame function agrees with the rect-based one */
        if (r.patch && r.npix) {
            static uint32_t full[FW * FH], pat[NVKVM_BROKER_CURSOR_MAX_DIM * NVKVM_BROKER_CURSOR_MAX_DIM];
            uint32_t xx, yy;

            for (yy = 0; yy < FH; yy++) { for (xx = 0; xx < FW; xx++) { full[yy * FW + xx] = frame_px(xx, yy); } }
            (void)nb_xor_patch_rect(c->mask + (size_t)c->by * c->w + c->bx, c->w, c->bw, c->bh, full, FW, FH, FW,
                                    tlx + (int)c->bx, tly + (int)c->by, pat);
            if (!px_eq(pat, r.px, (size_t)c->bw * c->bh)) { bad++; }
        }
    }
    CHECK(bad == 0, "differential: 3000 random cursors/positions/edges match the kernel formula (%u bad, %u composed)",
          bad, composed);
    CHECK(composed > 500, "differential: the sweep actually composed (%u)", composed);
    free(c);
    fx_close(&x);
}

/* ═════════════════════════════════════════════════════════════════════════
 * 7. THE CURSOR RECORD: mask kept beside the approximation
 * ════════════════════════════════════════════════════════════════════════ */

static void test_cursor_record(void)
{
    struct nb_cursor *a = new_cursor(8, 8, 1, 1), *b = calloc(1, sizeof *b);
    uint8_t raw[8 * 8 * 4];
    uint32_t i;

    memset(raw, 0, sizeof raw);
    for (i = 0; i < 8; i++) {      /* one column, white, premultiplied opaque */
        raw[(i * 8 + 3) * 4 + 0] = raw[(i * 8 + 3) * 4 + 1] = raw[(i * 8 + 3) * 4 + 2] = 0xff;
        raw[(i * 8 + 3) * 4 + 3] = 0xff;
    }
    nb_cursor_load(a, raw, 8, 8, 32, 1, 1);
    CHECK(!a->xor, "record: a load is not XOR");
    CHECK(nb_cursor_set_xor(a) && a->xor && a->bx == 3 && a->by == 0 && a->bw == 1 && a->bh == 8,
          "record: set_xor keeps the mask's bbox (%u,%u %ux%u)", a->bx, a->by, a->bw, a->bh);
    CHECK(a->mask[3] == 0xffffffffu && a->mask[0] == 0, "record: the MASK holds the inversion pixels");
    CHECK(a->px[3] == 0xff000000u && a->px[2] == 0xffffffffu,
          "record: px is the approximation (black fill, white outline) as before");
    nb_cursor_copy(b, a);
    CHECK(b->xor && b->bw == 1 && b->mask[3 + 8] == 0xffffffffu && b->px[3] == 0xff000000u,
          "record: a published copy carries mask and bbox");
    nb_cursor_load(a, raw, 8, 8, 32, 1, 1);
    CHECK(!a->xor && a->bw == 0, "record: the next ordinary SET clears the XOR state");
    memset(raw, 0, sizeof raw);
    nb_cursor_load(a, raw, 8, 8, 32, 1, 1);
    CHECK(!nb_cursor_set_xor(a) && a->xor && a->bw == 0, "record: an all-clear mask inverts nothing -> false");
    free(a);
    free(b);
}

int main(void)
{
    /* Several checks crash a child on purpose (a store through a read-only mapping, a read past a guard page).
     * With a piped core_pattern (apport, systemd-coredump) the kernel would dump each of them through the
     * handler, which stalls the dying child for minutes on a runner: no core files from this test. */
    struct rlimit nocore = { 0, 0 };

    setrlimit(RLIMIT_CORE, &nocore);
    prctl(PR_SET_DUMPABLE, 0, 0, 0, 0);   /* a piped core_pattern ignores the limit above; a non-dumpable process is never dumped */
    setvbuf(stdout, NULL, _IOLBF, 0);
    test_state_table();
    test_place();
    test_scale_box();
    test_helpers();
    test_frame_memfd();
    test_frame_udmabuf();
    test_compose();
    test_differential();
    test_cursor_record();
    printf("test_xor: %u checks, %u failed\n", n_checks, n_failed);
    return n_failed ? 1 : 0;
}
