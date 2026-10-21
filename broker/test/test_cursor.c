/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * test_cursor.c — the CMD_CURSOR record's validator, extent arithmetic, pixel
 * copy and scaler, driven directly.  No socket, no display, no GPU.
 *
 * Linked against the SAME nb_cursor.c the broker links (see the Makefile), so
 * what passes here is the code that runs, not a copy of it.  `make
 * check-sanitize` builds this under ASAN+UBSAN too, which is what turns the
 * fuzz loop's out-of-range reads and writes into failures instead of luck.
 *
 * Three kinds of check:
 *   - every boundary of every rule, from both sides (the value that passes and
 *     the first one that does not);
 *   - arithmetic chosen to wrap 32 bits if anything were computed in 32 bits;
 *   - a deterministic random sweep asserting the invariants the backends rely
 *     on: an accepted record's extent is inside the protocol bounds, a scaled
 *     cursor fits a NB_CURSOR_SCALED_MAX slot, and its hot spot is inside it.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../nvkvm_broker.h"

#define AR24 0x34325241u
#define XR24 0x34325258u

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

static struct nvkvm_broker_cursor_cmd base_set(void)
{
    struct nvkvm_broker_cursor_cmd c;

    memset(&c, 0, sizeof c);
    c.type = NVKVM_BROKER_CMD_CURSOR;
    c.op = NVKVM_BROKER_CURSOR_SET;
    c.width = 32;
    c.height = 32;
    c.stride = 128;
    c.fourcc = AR24;
    c.hot_x = 3;
    c.hot_y = 4;
    return c;
}

static int verdict(const struct nvkvm_broker_cursor_cmd *c, bool fd)
{
    const char *why = NULL;
    int v = nb_cursor_check(c, fd, false, &why);

    /* Every verdict names its rule, and an OK names none. */
    CHECK(why != NULL, "nb_cursor_check left *why unset");
    if (why) {
        CHECK((v == NB_CURSOR_OK) == (why[0] == '\0'),
              "verdict %d with reason '%s'", v, why);
    }
    return v;
}

static void test_framing(void)
{
    struct nvkvm_broker_cursor_cmd c;
    uint32_t ops[] = { 0, 4, 5, 0x80000001u, 0xffffffffu };
    unsigned i, f;

    c = base_set();
    CHECK(verdict(&c, true) == NB_CURSOR_OK, "the base SET is accepted");

    c = base_set(); c.flags = 1;
    CHECK(verdict(&c, true) == NB_CURSOR_VIOLATION, "flags bit 0 (F_SHM)");
    c = base_set(); c.flags = 0x8000;
    CHECK(verdict(&c, true) == NB_CURSOR_VIOLATION, "flags bit 15");
    c = base_set(); c.reserved1 = 1;
    CHECK(verdict(&c, true) == NB_CURSOR_VIOLATION, "reserved1");
    c = base_set(); c.reserved1 = 0x80000000u;
    CHECK(verdict(&c, true) == NB_CURSOR_VIOLATION, "reserved1 top bit");

    for (i = 0; i < sizeof ops / sizeof ops[0]; i++) {
        c = base_set(); c.op = ops[i];
        CHECK(verdict(&c, true) == NB_CURSOR_VIOLATION, "op %u", ops[i]);
        CHECK(verdict(&c, false) == NB_CURSOR_VIOLATION, "op %u, no fd",
              ops[i]);
    }

    c = base_set();
    CHECK(verdict(&c, false) == NB_CURSOR_VIOLATION, "SET without an fd");

    /* HIDE and SHOW: no fd, and every image field zero -- each one alone. */
    for (f = 0; f < 2; f++) {
        uint32_t op = f ? NVKVM_BROKER_CURSOR_SHOW : NVKVM_BROKER_CURSOR_HIDE;

        memset(&c, 0, sizeof c);
        c.type = NVKVM_BROKER_CMD_CURSOR;
        c.op = op;
        CHECK(verdict(&c, false) == NB_CURSOR_OK, "bare op %u", op);
        CHECK(verdict(&c, true) == NB_CURSOR_VIOLATION, "op %u with an fd",
              op);
        for (i = 0; i < 7; i++) {
            struct nvkvm_broker_cursor_cmd d = c;
            uint32_t *field[7] = { &d.width, &d.height, &d.stride, &d.offset,
                                   &d.fourcc, &d.hot_x, &d.hot_y };

            *field[i] = 1;
            CHECK(verdict(&d, false) == NB_CURSOR_VIOLATION,
                  "op %u with image field %u set", op, i);
        }
    }
}

static void test_content(void)
{
    struct nvkvm_broker_cursor_cmd c;

    c = base_set(); c.fourcc = XR24;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "XR24 cursor");
    c = base_set(); c.fourcc = 0;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "fourcc 0");

    /* Size: 1..256 on both axes. */
    c = base_set(); c.width = 0;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "width 0");
    c = base_set(); c.height = 0;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "height 0");
    c = base_set(); c.width = 1; c.height = 1; c.stride = 4;
    c.hot_x = 0; c.hot_y = 0;
    CHECK(verdict(&c, true) == NB_CURSOR_OK, "1x1");
    c = base_set(); c.width = 256; c.height = 256; c.stride = 1024;
    CHECK(verdict(&c, true) == NB_CURSOR_OK, "256x256");
    c = base_set(); c.width = 257; c.stride = 1028;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "width 257");
    c = base_set(); c.height = 257;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "height 257");
    /* A width whose *4 wraps 32 bits to something small. */
    c = base_set(); c.width = 0x40000001u; c.stride = 4;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "width that wraps *4");
    c = base_set(); c.width = 0xffffffffu;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "width UINT32_MAX");

    /* Hot spot: strictly inside. */
    c = base_set(); c.hot_x = 31; c.hot_y = 31;
    CHECK(verdict(&c, true) == NB_CURSOR_OK, "hot spot at the last pixel");
    c = base_set(); c.hot_x = 32;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "hot_x == width");
    c = base_set(); c.hot_y = 32;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "hot_y == height");
    c = base_set(); c.hot_x = 0xffffffffu;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "hot_x UINT32_MAX");

    /* Stride: width*4 .. 1024. */
    c = base_set(); c.stride = 127;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "stride < width*4");
    c = base_set(); c.stride = 0;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "stride 0");
    c = base_set(); c.stride = 1024;
    CHECK(verdict(&c, true) == NB_CURSOR_OK, "a 32-wide image in a 256 pitch");
    c = base_set(); c.stride = 1025;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "stride 1025");
    c = base_set(); c.stride = 0xffffffffu;
    CHECK(verdict(&c, true) == NB_CURSOR_REJECT, "stride UINT32_MAX");

    /* Offset is NOT the record's to judge -- it needs the fd's size. */
    c = base_set(); c.offset = 0xffffffffu;
    CHECK(verdict(&c, true) == NB_CURSOR_OK, "offset is checked by extent");
}

static void test_extent(void)
{
    CHECK(nb_cursor_span(32, 32, 128) == 128u * 31u + 128u, "32x32 span");
    CHECK(nb_cursor_span(1, 1, 4) == 4, "1x1 span");
    CHECK(nb_cursor_span(2, 2, 16) == 24,
          "the last row needs only its pixels, not a whole stride");
    CHECK(nb_cursor_span(256, 256, 1024) == 262144u, "largest span");
    CHECK(nb_cursor_span(0, 5, 4) == 0 && nb_cursor_span(5, 0, 20) == 0,
          "an empty image has no span");

    CHECK(nb_cursor_fits(0, 4096, 4096), "exact fit");
    CHECK(!nb_cursor_fits(1, 4096, 4096), "one byte over");
    CHECK(!nb_cursor_fits(0, 4096, 0), "an empty fd");
    /* In 32 bits 0xffffffff + 262144 wraps to 262143, which "fits" a 256 KiB
     * memfd.  It must not. */
    CHECK(!nb_cursor_fits(0xffffffffu, 262144u, 262144u),
          "offset + span must not wrap");
    CHECK(nb_cursor_fits(0xffffffffu, 1u, 0x100000000ull),
          "the exact 64-bit sum fits");
    CHECK(!nb_cursor_fits(0xffffffffu, 2u, 0x100000000ull),
          "and one past it does not");
}

static struct nb_cursor cur;     /* 256 KiB: static, not on the stack */

static void test_load(void)
{
    /* 2x2, stride 12 (one pixel of padding per row, filled with junk). */
    static const uint8_t src[] = {
        /* row 0: B G R A             padding  */
        0x01, 0x02, 0x03, 0xff,  0xff, 0x10, 0xff, 0x80,  0xde, 0xad, 0xbe, 0xef,
        /* row 1 */
        0x00, 0x00, 0x00, 0x00,  0x40, 0x40, 0x40, 0x40,
    };
    uint64_t gen0 = cur.gen;

    nb_cursor_load(&cur, src, 2, 2, 12, 1, 0);
    CHECK(cur.defined && cur.visible, "a load defines and shows");
    CHECK(cur.gen != gen0, "a load moves gen");
    /*
     * "Never repeats within a process" is a property of the WIDTH: a 32-bit
     * counter wraps to a value a backend may still hold as "already rendered"
     * (a cursor change every 8 ms reaches 2^32 in about a year).  Start one
     * load short of 2^32 and require the next to be 2^32, not 0.
     */
    cur.gen = 0xffffffffull;
    nb_cursor_load(&cur, src, 2, 2, 12, 1, 0);
    CHECK(cur.gen == 0x100000000ull, "gen does not wrap at 32 bits "
          "(0x%llx)", (unsigned long long)cur.gen);
    CHECK(cur.w == 2 && cur.h == 2 && cur.hot_x == 1 && cur.hot_y == 0,
          "geometry copied");
    CHECK(cur.px[0] == 0xff030201u, "opaque pixel unchanged: 0x%08x",
          cur.px[0]);
    /* R and B exceed alpha 0x80: clamped to it.  G (0x10) does not. */
    CHECK(cur.px[1] == 0x80801080u, "colour clamped to alpha: 0x%08x",
          cur.px[1]);
    CHECK(cur.px[2] == 0x00000000u, "transparent pixel");
    CHECK(cur.px[3] == 0x40404040u, "exactly-premultiplied pixel");
    /* Packed: the padding never reached the image. */
    {
        unsigned i;
        bool junk = false;

        for (i = 0; i < 4; i++) {
            junk |= cur.px[i] == 0xefbeadde;
        }
        CHECK(!junk, "stride padding is not image");
    }
}

static void test_geom(void)
{
    struct nb_cursor_geom g;
    static const uint8_t white[256 * 1024];

    nb_cursor_load(&cur, white, 32, 32, 128, 3, 4);

    nb_cursor_scaled_geom(&cur, 1, 1, 1, 1, 512, &g);
    CHECK(g.w == 32 && g.h == 32 && g.hot_x == 3 && g.hot_y == 4, "1:1");
    nb_cursor_scaled_geom(&cur, 0, 0, 7, 0, 512, &g);
    CHECK(g.w == 32 && g.h == 32, "a zero factor means 1:1");
    nb_cursor_scaled_geom(&cur, 3840, 1920, 2160, 1080, 512, &g);
    CHECK(g.w == 64 && g.h == 64 && g.hot_x == 6 && g.hot_y == 8,
          "a 2x-scaled frame doubles the cursor and its hot spot");
    /* Halved, hot 3 is a source column the scaler skips (it samples 0, 2,
     * 4...); the hot spot is the output pixel showing the next one, 4. */
    nb_cursor_scaled_geom(&cur, 960, 1920, 540, 1080, 512, &g);
    CHECK(g.w == 16 && g.h == 16 && g.hot_x == 2 && g.hot_y == 2,
          "a half-size frame halves it (hot %u,%u)", g.hot_x, g.hot_y);
    nb_cursor_scaled_geom(&cur, 3, 1, 1, 1, 512, &g);
    CHECK(g.w == 96 && g.h == 32, "stretch scales the axes separately");
    nb_cursor_scaled_geom(&cur, 1, 1000, 1, 1000, 512, &g);
    CHECK(g.w == 1 && g.h == 1 && g.hot_x == 0 && g.hot_y == 0,
          "never below one pixel, hot spot still inside");
    nb_cursor_scaled_geom(&cur, 0xffffffffu, 1, 0xffffffffu, 1, 512, &g);
    CHECK(g.w == 512 && g.h == 512 && g.hot_x < 512 && g.hot_y < 512,
          "an absurd factor is clamped, not wrapped");
    nb_cursor_scaled_geom(&cur, 1, 1, 1, 1, 0, &g);
    CHECK(g.w == 1 && g.h == 1, "max 0 is treated as 1");

    nb_cursor_load(&cur, white, 256, 64, 1024, 255, 63);
    nb_cursor_scaled_geom(&cur, 4, 1, 4, 1, 512, &g);
    CHECK(g.w == 512 && g.h == 128, "clamping keeps the aspect (%ux%u)",
          g.w, g.h);
    CHECK(g.hot_x < g.w && g.hot_y < g.h, "clamped hot spot inside");
}

static void test_scale_pixels(void)
{
    /* A 2x2 checker scaled 2x must be four 2x2 blocks, nearest-neighbour. */
    static const uint8_t src[] = {
        0x00, 0x00, 0x00, 0xff,  0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff,  0x00, 0x00, 0x00, 0xff,
    };
    uint32_t out[4 * 4 + 1];
    unsigned x, y;
    bool ok = true;

    nb_cursor_load(&cur, src, 2, 2, 8, 0, 0);
    out[16] = 0x5a5a5a5au;          /* canary past the end */
    nb_cursor_scale(&cur, out, 4, 4);
    for (y = 0; y < 4; y++) {
        for (x = 0; x < 4; x++) {
            uint32_t want = ((x / 2) ^ (y / 2)) ? 0xffffffffu : 0xff000000u;

            ok &= out[y * 4 + x] == want;
        }
    }
    CHECK(ok, "nearest-neighbour 2x");
    CHECK(out[16] == 0x5a5a5a5au, "scale wrote past dw*dh");
}

/* The scaler's own sampling rule, restated: output index x shows source
 * floor(x * in / out). */
static uint32_t sample_of(uint32_t x, uint32_t in, uint32_t out)
{
    return (uint32_t)((uint64_t)x * in / out);
}

/*
 * THE HOT SPOT LANDS ON THE GUEST'S HOT PIXEL (finding 6, 2026-10-03).
 *
 * First the case the review measured: 32 px at 1.5x with hot spot 3.  An image
 * whose ONLY opaque pixel is the hot pixel is scaled exactly as a backend
 * scales it, and the pixel under the scaled hot spot must be that pixel.
 * floor(hot * out / in) -- the old rule -- put it on output 4, which shows
 * source 2: transparent.
 */
static void test_hot_pixel(void)
{
    static uint8_t src[32 * 4 * 32];
    static uint32_t out[NB_CURSOR_SCALED_MAX * NB_CURSOR_SCALED_MAX];
    struct nb_cursor_geom g;

    memset(src, 0, sizeof src);
    src[3 * 128 + 3 * 4 + 3] = 0xff;            /* (3,3): alpha 255 */
    nb_cursor_load(&cur, src, 32, 32, 128, 3, 3);
    nb_cursor_scaled_geom(&cur, 3, 2, 3, 2, NB_CURSOR_SCALED_MAX, &g);
    CHECK(g.w == 48 && g.h == 48, "1.5x of 32 is 48 (%ux%u)", g.w, g.h);
    CHECK(g.hot_x == 5 && g.hot_y == 5, "1.5x: hot 3,3 -> 5,5 (got %u,%u)",
          g.hot_x, g.hot_y);
    nb_cursor_scale(&cur, out, g.w, g.h);
    CHECK((out[(size_t)g.hot_y * g.w + g.hot_x] >> 24) == 0xff,
          "1.5x: the pixel under the hot spot is the guest's hot pixel");
}

/*
 * ...and then every scale a window can plausibly produce, every width up to
 * 64 and the protocol's largest, and every hot spot in each: the hot spot must
 * be EXACTLY the first output index whose sample reaches the hot pixel
 * (clamped to the last), and whenever the cursor is not shrunk that index must
 * sample the hot pixel itself -- checked on real scaled pixels, not just on
 * the arithmetic.
 */
static void test_hot_sweep(void)
{
    static const uint32_t f[][2] = {
        {1, 1}, {3, 2}, {2, 1}, {5, 4}, {7, 5}, {4, 3}, {9, 4}, {3, 1},
        {5, 3}, {1, 2}, {2, 3}, {4, 5}, {1, 3}, {7, 3}, {11, 8}, {2560, 1920},
        {1920, 1280}, {3840, 2560}, {1366, 1024}, {1000, 999}, {999, 1000},
    };
    static uint8_t row[256 * 4];
    static uint32_t out[NB_CURSOR_SCALED_MAX * NB_CURSOR_SCALED_MAX];
    unsigned fi, bad = 0, exact = 0, pixel_bad = 0;
    uint32_t w, hot;

    for (fi = 0; fi < sizeof f / sizeof f[0]; fi++) {
        /* 1..64 one by one, then 128 and 256 (the protocol maximum). */
        for (w = 1; w <= 256; w = (w < 64) ? w + 1 : w * 2) {
            const uint32_t ww = w;

            for (hot = 0; hot < ww; hot++) {
                struct nb_cursor_geom g;
                uint32_t want, x;

                /* A ww x 1 image, opaque only at the hot pixel: the scaler is
                 * separable, so one row tests the x rule completely; the same
                 * call with the axes swapped tests y. */
                memset(row, 0, (size_t)ww * 4);
                row[hot * 4 + 3] = 0xff;
                nb_cursor_load(&cur, row, ww, 1, ww * 4, hot, 0);
                nb_cursor_scaled_geom(&cur, f[fi][0], f[fi][1], 1, 1,
                                      NB_CURSOR_SCALED_MAX, &g);
                want = (uint32_t)(((uint64_t)hot * g.w + ww - 1) / ww);
                if (want >= g.w) {
                    want = g.w - 1;
                }
                if (g.hot_x != want) {
                    bad++;
                }
                /* The first index at or past the hot pixel, by the scaler's
                 * own rule: no earlier index reaches it. */
                for (x = 0; x < g.hot_x; x++) {
                    if (sample_of(x, ww, g.w) >= hot) {
                        bad++;
                        break;
                    }
                }
                /* And, independently of the two arithmetic checks above, on
                 * the scaled pixels themselves. */
                if (g.w >= ww) {
                    exact++;
                    nb_cursor_scale(&cur, out, g.w, g.h);
                    if (sample_of(g.hot_x, ww, g.w) != hot ||
                        (out[g.hot_x] >> 24) != 0xff) {
                        pixel_bad++;
                    }
                }
                /* The y axis: the same image as a column. */
                nb_cursor_load(&cur, row, 1, ww, 4, 0, hot);
                nb_cursor_scaled_geom(&cur, 1, 1, f[fi][0], f[fi][1],
                                      NB_CURSOR_SCALED_MAX, &g);
                want = (uint32_t)(((uint64_t)hot * g.h + ww - 1) / ww);
                if (want >= g.h) {
                    want = g.h - 1;
                }
                if (g.hot_y != want) {
                    bad++;
                }
                if (g.h >= ww) {
                    nb_cursor_scale(&cur, out, g.w, g.h);
                    if ((out[(size_t)g.hot_y * g.w] >> 24) != 0xff) {
                        pixel_bad++;
                    }
                }
            }
        }
    }
    CHECK(exact > 10000, "the sweep reached the not-shrunk case (%u)", exact);
    CHECK(bad == 0, "%u hot spots off the first index that shows the hot "
          "pixel", bad);
    CHECK(pixel_bad == 0, "%u unshrunk cursors whose hot spot is not on the "
          "guest's hot pixel", pixel_bad);
}

/*
 * HiDPI (finding 7): the buffer a backend renders is the cursor in DEVICE
 * pixels.  A 32x32 guest cursor over a 3840-wide guest frame shown in a
 * 1920-logical window on a scale-2 output is 16 logical pixels -- and 32
 * device pixels, i.e. the guest's own, so nothing is lost to magnification.
 */
static void test_device_geom(void)
{
    static const uint8_t white[256 * 1024];
    struct nb_cursor_geom g, b;
    unsigned i, bad = 0;

    memset(&b, 0, sizeof b);
    nb_cursor_load(&cur, white, 32, 32, 128, 3, 4);
    nb_cursor_scaled_geom(&cur, 1920, 3840, 1080, 2160, 512, &g);
    nb_cursor_device_geom(&cur, 1920, 3840, 1080, 2160, 240, 512, &b);
    CHECK(g.w == 16 && g.h == 16, "logical: 16x16 (%ux%u)", g.w, g.h);
    CHECK(b.w == 32 && b.h == 32 && b.hot_x == 3 && b.hot_y == 4,
          "scale 2: the buffer is the guest's own 32x32, hot 3,4 "
          "(%ux%u hot %u,%u)", b.w, b.h, b.hot_x, b.hot_y);
    nb_cursor_device_geom(&cur, 1920, 3840, 1080, 2160, 180, 512, &b);
    CHECK(b.w == 24 && b.h == 24, "scale 1.5: 24x24 (%ux%u)", b.w, b.h);
    nb_cursor_device_geom(&cur, 1920, 3840, 1080, 2160, 120, 512, &b);
    CHECK(b.w == g.w && b.h == g.h && b.hot_x == g.hot_x &&
          b.hot_y == g.hot_y, "scale 1: the buffer is the logical cursor");
    nb_cursor_device_geom(&cur, 1920, 3840, 1080, 2160, 0, 512, &b);
    CHECK(b.w == g.w && b.h == g.h, "scale unknown (0): 1:1");
    nb_cursor_device_geom(&cur, 1920, 3840, 1080, 2160, 60, 512, &b);
    CHECK(b.w == g.w && b.h == g.h, "scale below 1: never fewer pixels "
          "than logical");
    nb_cursor_device_geom(&cur, 1, 1, 1, 1, 0xffffffffu, 512, &b);
    CHECK(b.w == 512 && b.h == 512 && b.hot_x < 512 && b.hot_y < 512,
          "an absurd scale is clamped (16x of 32 = 512)");
    nb_cursor_device_geom(&cur, 0xffffffffu, 1, 0xffffffffu, 1, 0xffffffffu,
                          512, &b);
    CHECK(b.w == 512 && b.h == 512 && b.hot_x < 512 && b.hot_y < 512,
          "absurd factor AND scale: clamped, not wrapped");
    /* Whatever the inputs, the buffer fits a slot and its hot spot is in it. */
    for (i = 0; i < 20000; i++) {
        uint32_t sw = 1 + (i * 2654435761u) % 256u;
        uint32_t sh = 1 + (i * 40503u) % 256u;

        nb_cursor_load(&cur, white, sw, sh, sw * 4, (i * 7u) % sw,
                       (i * 13u) % sh);
        nb_cursor_device_geom(&cur, 1 + (i * 977u) % 8192u,
                              1 + (i * 331u) % 8192u, 1 + (i * 619u) % 8192u,
                              1 + (i * 113u) % 8192u, (i * 37u) % 4000u,
                              NB_CURSOR_SCALED_MAX, &b);
        if (b.w < 1 || b.w > NB_CURSOR_SCALED_MAX || b.h < 1 ||
            b.h > NB_CURSOR_SCALED_MAX || b.hot_x >= b.w || b.hot_y >= b.h) {
            bad++;
        }
    }
    CHECK(bad == 0, "%u device geometries outside a slot", bad);
}

/*
 * THE SIZE RULE of a cursor surface (hover pointer and grab overlay alike).
 *
 * A guest cursor pixel is one GUEST FRAME pixel.  The frame is shown at
 * `fit` logical pixels per guest pixel (fit_w / buf_w), so the cursor's
 * SURFACE-LOCAL size -- the unit wl_pointer.set_cursor's hot spot and a
 * subsurface's position use -- is  image_dim * fit,  and nothing else: the
 * output scale is NOT a factor of it.  The output scale only decides how many
 * BUFFER pixels carry those logical pixels: wl_surface.set_buffer_scale is an
 * integer k and the surface is buffer / k, so the buffer is  logical * k  with
 * k = ceil(scale), and on screen the cursor measures logical * scale PHYSICAL
 * pixels  (= image_dim * fit * scale: the frame's own magnification).
 */
struct surf_case {
    const char *name;
    uint32_t w, h;                  /* the guest image */
    uint32_t fit_num, fit_den;      /* fit_w / buf_w (both axes) */
    uint32_t scale_120;
    uint32_t want_lw, want_lh;      /* surface-local (logical) size */
    uint32_t want_k;
};

static void check_surface(const struct surf_case *t)
{
    static const uint8_t px[256 * 1024];
    struct nb_cursor_surface s;
    double phys, want_phys;
    double sc = (t->scale_120 < 120 ? 120 : t->scale_120) / 120.0;

    nb_cursor_load(&cur, px, t->w, t->h, t->w * 4, 0, 0);
    memset(&s, 0xa5, sizeof s);
    nb_cursor_surface_geom(&cur, t->fit_num, t->fit_den, t->fit_num,
                           t->fit_den, t->scale_120, NB_CURSOR_SURFACE_MAX_BUF,
                           &s);
    CHECK(s.g.w == t->want_lw && s.g.h == t->want_lh,
          "%s: logical %ux%u, want %ux%u", t->name, s.g.w, s.g.h, t->want_lw,
          t->want_lh);
    CHECK(s.scale == t->want_k, "%s: buffer_scale %u, want %u", t->name,
          s.scale, t->want_k);
    CHECK(s.bw == s.g.w * s.scale && s.bh == s.g.h * s.scale,
          "%s: the buffer %ux%u is not logical x scale (wl_surface would "
          "raise invalid_size)", t->name, s.bw, s.bh);
    /* On screen, in PHYSICAL pixels: the frame's magnification of the image,
     * within the rounding of one logical pixel. */
    phys = (double)s.g.w * sc;
    want_phys = (double)t->w * t->fit_num / t->fit_den * sc;
    CHECK(phys > want_phys - 1.5 * sc && phys < want_phys + 1.5 * sc,
          "%s: on screen %.1f physical px, the frame magnifies the image to "
          "%.1f", t->name, phys, want_phys);
}

static void test_surface_geom(void)
{
    static const struct surf_case cases[] = {
        /* THE OWNER'S SCREEN (broker log, run 3001): a 256x256 cursor image,
         * guest frame 2048 wide shown in 1915 logical pixels, output 1.5:
         * logical 239 (= 256*1915/2048), buffer 478 at buffer_scale 2.
         * 239 logical = 358.5 physical = 256 guest px * 1.4004. */
        {"owner: fit 1915/2048 scale 1.5", 256, 256, 1915, 2048, 180, 239, 239, 2},
        {"owner: window grown, fit 2446/2048", 256, 256, 2446, 2048, 180, 306, 306, 2},
        /* fit < 1 / fit == 1 / fit > 1, every output scale */
        {"fit 0.5 scale 1.0", 32, 32, 1, 2, 120, 16, 16, 1},
        {"fit 0.5 scale 1.25", 32, 32, 1, 2, 150, 16, 16, 2},
        {"fit 0.5 scale 1.5", 32, 32, 1, 2, 180, 16, 16, 2},
        {"fit 0.5 scale 2.0", 32, 32, 1, 2, 240, 16, 16, 2},
        {"fit 1 scale 1.0", 32, 32, 1, 1, 120, 32, 32, 1},
        {"fit 1 scale 1.25", 32, 32, 1, 1, 150, 32, 32, 2},
        {"fit 1 scale 1.5", 32, 32, 1, 1, 180, 32, 32, 2},
        {"fit 1 scale 2.0", 32, 32, 1, 1, 240, 32, 32, 2},
        {"fit 1.5 scale 1.0", 32, 32, 3, 2, 120, 48, 48, 1},
        {"fit 1.5 scale 1.5", 32, 32, 3, 2, 180, 48, 48, 2},
        {"fit 2 scale 2.0", 32, 32, 2, 1, 240, 64, 64, 2},
        /* the hover pointer of a 2x HiDPI guest shown 1:2 (finding 7) */
        {"HiDPI: 32px cursor, frame 3840 in 1920 logical, scale 2", 32, 32, 1920,
         3840, 240, 16, 16, 2},
        /* a non-square image keeps its aspect */
        {"I-beam-like 12x24", 12, 24, 3, 2, 180, 18, 36, 2},
        /* unknown / below-1 scale is 1 */
        {"scale unknown (0)", 32, 32, 1, 1, 0, 32, 32, 1},
        {"scale below 1", 32, 32, 1, 1, 60, 32, 32, 1},
    };
    struct nb_cursor_surface a, b;
    static const uint8_t px[256 * 1024];
    unsigned i, bad = 0;

    for (i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        check_surface(&cases[i]);
    }

    /* The output scale is NOT a factor of the logical size: the same frame fit
     * at 1.0 and at 2.0 gives the same surface-local cursor (a double-scaling
     * regression multiplies it). */
    nb_cursor_load(&cur, px, 256, 256, 1024, 14, 15);
    nb_cursor_surface_geom(&cur, 1915, 2048, 1155, 1155, 120,
                           NB_CURSOR_SURFACE_MAX_BUF, &a);
    nb_cursor_surface_geom(&cur, 1915, 2048, 1155, 1155, 240,
                           NB_CURSOR_SURFACE_MAX_BUF, &b);
    CHECK(a.g.w == b.g.w && a.g.h == b.g.h && a.g.hot_x == b.g.hot_x &&
          a.g.hot_y == b.g.hot_y,
          "the output scale changed the logical size (%u -> %u): double "
          "scaling", a.g.w, b.g.w);
    CHECK(b.bw == 2 * a.bw, "scale 2 carries it in twice the buffer pixels");

    /* The hot spot is logical and inside the surface; it follows the fit. */
    nb_cursor_surface_geom(&cur, 1915, 2048, 1915, 2048, 180,
                           NB_CURSOR_SURFACE_MAX_BUF, &a);
    CHECK(a.g.hot_x == 14 * 1915 / 2048 + 1 || a.g.hot_x == 14 * 1915 / 2048,
          "hot x %u follows the fit", a.g.hot_x);

    /* RESIZE: a window resize is a new fit and a new size, nothing sticky. */
    nb_cursor_surface_geom(&cur, 1000, 2048, 1000, 2048, 180,
                           NB_CURSOR_SURFACE_MAX_BUF, &a);
    nb_cursor_surface_geom(&cur, 2000, 2048, 2000, 2048, 180,
                           NB_CURSOR_SURFACE_MAX_BUF, &b);
    CHECK(a.g.w == 125 && b.g.w == 250, "resize: %u -> %u (want 125 -> 250)",
          a.g.w, b.g.w);

    /* Bounds: whatever comes in, the buffer fits a slot, is exactly logical x
     * scale, and the hot spot is inside the surface. */
    for (i = 0; i < 40000; i++) {
        uint32_t sw = 1 + (i * 2654435761u) % 256u;
        uint32_t sh = 1 + (i * 40503u) % 256u;

        nb_cursor_load(&cur, px, sw, sh, sw * 4, (i * 7u) % sw, (i * 13u) % sh);
        nb_cursor_surface_geom(&cur, 1 + (i * 977u) % 8192u,
                               1 + (i * 331u) % 8192u, 1 + (i * 619u) % 8192u,
                               1 + (i * 113u) % 8192u, (i * 37u) % 4000u,
                               NB_CURSOR_SURFACE_MAX_BUF, &a);
        if (a.g.w < 1 || a.g.h < 1 || a.scale < 1 || a.scale > 4 ||
            a.bw != a.g.w * a.scale || a.bh != a.g.h * a.scale ||
            a.bw > NB_CURSOR_SURFACE_MAX_BUF ||
            a.bh > NB_CURSOR_SURFACE_MAX_BUF || a.g.hot_x >= a.g.w ||
            a.g.hot_y >= a.g.h) {
            bad++;
        }
    }
    CHECK(bad == 0, "%u cursor surfaces outside the bounds", bad);
    nb_cursor_surface_geom(&cur, 0xffffffffu, 1, 0xffffffffu, 1, 0xffffffffu,
                           NB_CURSOR_SURFACE_MAX_BUF, &a);
    CHECK(a.bw <= NB_CURSOR_SURFACE_MAX_BUF && a.scale == 4 &&
          a.bw == a.g.w * 4, "absurd factor and scale: clamped, not wrapped");
}

/*
 * THE TRUE XOR COMPOSE (D2).  ORACLE: the retired kayfabe compose kernel,
 * kf_disp::scanout::compose_reference, XOR branch:  out = below ^ (w &
 * 0x00ffffff), the top byte kept; its tests: a white opaque pixel inverts, a
 * clear pixel never flips anything, an all-zero image composes to nothing
 * (docs/design/V3_DISPLAY_CURSOR_FORMATS.md section 3/4).
 */
static struct nb_cursor xcur;
static uint32_t xframe[16 * 16], xout[16 * 16], xsave[16 * 16];

static void xmask(uint32_t w, uint32_t h)
{
    memset(&xcur, 0, sizeof xcur);
    xcur.w = w;
    xcur.h = h;
    xcur.defined = true;
    xcur.visible = true;
    xcur.hot_x = w / 2;     /* the hot spot is irrelevant to the compose */
    xcur.hot_y = h / 2;
}

static uint32_t kernel_ref(uint32_t below, uint32_t mask_px)
{
    return below ^ (mask_px & 0x00ffffffu);     /* compose_reference, XOR */
}

static void test_xor_compose(void)
{
    unsigned i, bad = 0;
    size_t n;

    /* inverts under white, black and a colour; the colour is per channel */
    xmask(4, 1);
    xcur.px[0] = 0xffffffffu;           /* invert everything */
    xcur.px[1] = 0xff00ff00u;           /* invert green only */
    xcur.px[2] = 0x00ffffffu;           /* alpha 0: transparent, colour ignored */
    xcur.px[3] = 0;                     /* nothing */
    xframe[0] = 0x00ffffffu; xframe[1] = 0x00000000u;
    xframe[2] = 0x00123456u; xframe[3] = 0x00abcdefu;
    n = nb_cursor_xor_apply(&xcur, xframe, 4, 1, 4, 0, 0);
    CHECK(n == 2, "two pixels inverted (%zu)", n);
    CHECK(xframe[0] == 0x00000000u, "white under white-mask -> black (%08x)", xframe[0]);
    CHECK(xframe[1] == 0x0000ff00u, "black under green-mask -> green only (%08x)", xframe[1]);
    CHECK(xframe[2] == 0x00123456u, "alpha 0: untouched (%08x)", xframe[2]);
    CHECK(xframe[3] == 0x00abcdefu, "empty mask pixel: untouched (%08x)", xframe[3]);

    /* black -> white, colour -> complement, the top byte is kept */
    xmask(2, 1);
    xcur.px[0] = xcur.px[1] = 0xffffffffu;
    xframe[0] = 0x00000000u;
    xframe[1] = 0xab123456u;
    nb_cursor_xor_apply(&xcur, xframe, 2, 1, 2, 0, 0);
    CHECK(xframe[0] == 0x00ffffffu, "black -> white (%08x)", xframe[0]);
    CHECK(xframe[1] == 0xabedcba9u, "colour -> complement, X/A byte kept (%08x)",
          xframe[1]);

    /* involution: the same mask twice is the identity */
    xmask(3, 3);
    for (i = 0; i < 9; i++) {
        xcur.px[i] = (i & 1) ? 0xffffffffu : 0;
        xframe[i] = 0x01010101u * (i + 1);
        xsave[i] = xframe[i];
    }
    nb_cursor_xor_apply(&xcur, xframe, 3, 3, 3, 0, 0);
    CHECK(memcmp(xframe, xsave, 9 * 4) != 0, "the compose changed something");
    nb_cursor_xor_apply(&xcur, xframe, 3, 3, 3, 0, 0);
    CHECK(memcmp(xframe, xsave, 9 * 4) == 0, "XOR twice = identity");

    /* an all-zero mask composes to nothing; so does an undefined cursor */
    xmask(4, 4);
    memset(xframe, 0x5a, sizeof xframe);
    memcpy(xsave, xframe, sizeof xsave);
    CHECK(nb_cursor_xor_apply(&xcur, xframe, 4, 4, 4, 0, 0) == 0 &&
          memcmp(xframe, xsave, sizeof xframe) == 0, "an empty mask is nothing");
    xcur.defined = false;
    xcur.px[0] = 0xffffffffu;
    CHECK(nb_cursor_xor_apply(&xcur, xframe, 4, 4, 4, 0, 0) == 0 &&
          memcmp(xframe, xsave, sizeof xframe) == 0, "an undefined cursor is nothing");

    /* edges: clipped on all four sides, hot spot irrelevant, stride honoured */
    xmask(4, 4);
    for (i = 0; i < 16; i++) {
        xcur.px[i] = 0xffffffffu;
    }
    memset(xframe, 0, sizeof xframe);
    n = nb_cursor_xor_apply(&xcur, xframe, 8, 8, 16, -2, -3);
    CHECK(n == 2, "top-left clip: 2x1 pixels remain (%zu)", n);
    CHECK(xframe[0] == 0x00ffffffu && xframe[1] == 0x00ffffffu &&
          xframe[16] == 0, "top-left clip lands at 0,0 and 1,0 only");
    memset(xframe, 0, sizeof xframe);
    n = nb_cursor_xor_apply(&xcur, xframe, 8, 8, 16, 6, 7);
    CHECK(n == 2, "bottom-right clip: 2x1 pixels remain (%zu)", n);
    CHECK(xframe[7 * 16 + 6] == 0x00ffffffu && xframe[7 * 16 + 7] == 0x00ffffffu &&
          xframe[6 * 16 + 6] == 0, "bottom-right clip");
    CHECK(nb_cursor_xor_apply(&xcur, xframe, 8, 8, 16, 8, 0) == 0 &&
          nb_cursor_xor_apply(&xcur, xframe, 8, 8, 16, 0, 8) == 0 &&
          nb_cursor_xor_apply(&xcur, xframe, 8, 8, 16, -4, 0) == 0 &&
          nb_cursor_xor_apply(&xcur, xframe, 8, 8, 16, 0, -4) == 0 &&
          nb_cursor_xor_apply(&xcur, xframe, 8, 8, 16, INT32_MIN, INT32_MAX) == 0,
          "fully outside (incl. extreme positions): nothing");
    CHECK(nb_cursor_xor_apply(&xcur, xframe, 8, 8, 4, 0, 0) == 0,
          "a stride shorter than the width is refused");

    /* the patch: opaque inverted pixels where the cursor inverts AND on the
     * frame, zero elsewhere; the frame is never written */
    xmask(4, 2);
    xcur.px[0] = 0xffffffffu;
    xcur.px[1] = 0xff0000ffu;           /* blue only */
    xcur.px[5] = 0xffffffffu;
    for (i = 0; i < 16; i++) {
        xframe[i] = 0x00102030u + i;
    }
    memcpy(xsave, xframe, sizeof xsave);
    memset(xout, 0xee, sizeof xout);
    n = nb_cursor_xor_patch(&xcur, xframe, 4, 2, 4, 0, 0, xout);
    CHECK(n == 3, "three patch pixels (%zu)", n);
    CHECK(xout[0] == (0xff000000u | (0x00102030u ^ 0x00ffffffu)), "patch 0 %08x", xout[0]);
    CHECK(xout[1] == (0xff000000u | (0x00102031u ^ 0x000000ffu)), "patch 1 %08x", xout[1]);
    CHECK(xout[2] == 0 && xout[3] == 0 && xout[4] == 0 && xout[6] == 0 &&
          xout[7] == 0, "the rest of the patch is transparent");
    CHECK(xout[5] == (0xff000000u | (0x00102035u ^ 0x00ffffffu)), "patch 5 %08x", xout[5]);
    CHECK(memcmp(xframe, xsave, sizeof xframe) == 0, "the patch never writes the frame");
    memset(xout, 0xee, sizeof xout);
    n = nb_cursor_xor_patch(&xcur, xframe, 4, 2, 4, -1, 0, xout);
    CHECK(n == 2 && xout[0] == 0 && xout[1] == (0xff000000u | (0x00102030u ^ 0x000000ffu)),
          "patch clipped at the left edge: %zu, %08x", n, xout[1]);

    /* DIFFERENTIAL vs the kernel formula over random masks, frames, positions:
     * apply == kernel_ref on every covered pixel, the patch == apply's result */
    for (i = 0; i < 30000; i++) {
        uint32_t w = 1 + (i * 7u) % 6, h = 1 + (i * 5u) % 6, k, fx, fy;
        int32_t px = (int32_t)((i * 2654435761u) % 15u) - 4;
        int32_t py = (int32_t)((i * 40503u) % 15u) - 4;
        uint32_t a[16 * 16];

        xmask(w, h);
        for (k = 0; k < w * h; k++) {
            uint32_t r = (i + 1) * 2246822519u + k * 3266489917u;

            /* alpha 0xff + colour, or fully transparent: what kayfabe sends */
            xcur.px[k] = (r & 4u) ? (0xff000000u | ((r >> 8) & 0xffffffu)) : 0;
        }
        for (k = 0; k < 16 * 16; k++) {
            xframe[k] = (i + k) * 2654435761u;
        }
        memcpy(a, xframe, sizeof a);
        nb_cursor_xor_apply(&xcur, a, 16, 16, 16, px, py);
        nb_cursor_xor_patch(&xcur, xframe, 16, 16, 16, px, py, xout);
        for (fy = 0; fy < 16; fy++) {
            for (fx = 0; fx < 16; fx++) {
                int64_t mx = (int64_t)fx - px, my = (int64_t)fy - py;
                uint32_t want = xframe[fy * 16 + fx];

                if (mx >= 0 && my >= 0 && mx < (int64_t)w && my < (int64_t)h) {
                    want = kernel_ref(want, xcur.px[my * w + mx]);
                    if (xcur.px[my * w + mx] &&
                        xout[my * w + mx] != (0xff000000u | (want & 0xffffffu))) {
                        bad++;
                    }
                }
                if (a[fy * 16 + fx] != want) {
                    bad++;
                }
            }
        }
    }
    CHECK(bad == 0, "%u pixels differ from the compose kernel's formula", bad);

    /* a SCALED cursor: composing the 2x nearest-scaled mask onto the 2x frame
     * is the 2x nearest-scale of composing at 1x (the compositor scaling the
     * result is the same picture) */
    {
        static uint32_t big[4 * 4], small_[2 * 2], sm_ref[2 * 2], scaled[4 * 4];
        struct nb_cursor m2;
        unsigned fx, fy;

        xmask(2, 2);
        xcur.px[0] = 0xffffffffu;
        xcur.px[3] = 0xff808080u;
        small_[0] = 0x00112233u; small_[1] = 0x00445566u;
        small_[2] = 0x00778899u; small_[3] = 0x00aabbccu;
        memcpy(sm_ref, small_, sizeof small_);
        nb_cursor_xor_apply(&xcur, sm_ref, 2, 2, 2, 0, 0);
        nb_cursor_scale(&xcur, scaled, 4, 4);
        memset(&m2, 0, sizeof m2);
        m2.w = m2.h = 4;
        m2.defined = true;
        memcpy(m2.px, scaled, sizeof scaled);
        for (fy = 0; fy < 4; fy++) {
            for (fx = 0; fx < 4; fx++) {
                big[fy * 4 + fx] = small_[(fy / 2) * 2 + fx / 2];
            }
        }
        nb_cursor_xor_apply(&m2, big, 4, 4, 4, 0, 0);
        for (fy = 0; fy < 4; fy++) {
            for (fx = 0; fx < 4; fx++) {
                CHECK(big[fy * 4 + fx] == sm_ref[(fy / 2) * 2 + fx / 2],
                      "scaled cursor, pixel %u,%u: %08x vs %08x", fx, fy,
                      big[fy * 4 + fx], sm_ref[(fy / 2) * 2 + fx / 2]);
            }
        }
    }
}

/*
 * PUBLISHING (finding 3) is a copy: the published cursor must be the image as
 * it was, unaffected by the next load into the pending one -- which is the
 * whole point of having two.
 */
static void test_copy(void)
{
    static struct nb_cursor pub;
    static const uint8_t red[] = { 0, 0, 0xff, 0xff,  0, 0, 0xff, 0xff };
    static const uint8_t blue[] = { 0xff, 0, 0, 0xff,  0xff, 0, 0, 0xff };

    nb_cursor_load(&cur, red, 2, 1, 8, 1, 0);
    nb_cursor_copy(&pub, &cur);
    CHECK(pub.defined && pub.visible && pub.gen == cur.gen && pub.w == 2 &&
          pub.h == 1 && pub.hot_x == 1 && pub.px[0] == 0xffff0000u &&
          pub.px[1] == 0xffff0000u, "a copy is the image, field for field");
    nb_cursor_load(&cur, blue, 2, 1, 8, 0, 0);
    CHECK(pub.px[0] == 0xffff0000u && pub.hot_x == 1 && pub.gen != cur.gen,
          "a later load does not reach the published copy");
    cur.defined = false;
    cur.gen++;
    nb_cursor_copy(&pub, &cur);
    CHECK(!pub.defined && pub.gen == cur.gen, "forgetting is published too");
}

static void test_wanted(void)
{
    struct nb_cursor *c = &cur;

    CHECK(!nb_cursor_wanted(NULL, false), "no cursor state");
    c->defined = true; c->visible = true;
    CHECK(nb_cursor_wanted(c, false), "defined, visible, ungrabbed");
    CHECK(!nb_cursor_wanted(c, true), "the grab hides it");
    c->visible = false;
    CHECK(!nb_cursor_wanted(c, false), "HIDE hides it");
    c->visible = true; c->defined = false;
    CHECK(!nb_cursor_wanted(c, false), "SHOW without an image shows none");
}

/* Deterministic: a failure here reproduces on every run and every machine. */
static uint64_t rng = 0x9e3779b97f4a7c15ull;
static uint32_t rnd(void)
{
    rng ^= rng << 13;
    rng ^= rng >> 7;
    rng ^= rng << 17;
    return (uint32_t)(rng >> 16);
}

/* Mostly values near the boundaries, sometimes anything at all. */
static uint32_t edgy(uint32_t bound)
{
    switch (rnd() % 6) {
    case 0: return 0;
    case 1: return bound;
    case 2: return bound + 1;
    case 3: return bound ? bound - 1 : 0;
    case 4: return rnd() % (bound + 2);
    default: return rnd() | (rnd() << 16);
    }
}

static void test_fuzz(void)
{
    static uint8_t src[NVKVM_BROKER_CURSOR_MAX_STRIDE *
                       NVKVM_BROKER_CURSOR_MAX_DIM];
    static uint32_t slot[NB_CURSOR_SCALED_MAX * NB_CURSOR_SCALED_MAX + 1];
    unsigned i, accepted = 0, bad = 0;

    for (i = 0; i < sizeof src; i++) {
        src[i] = (uint8_t)rnd();
    }
    for (i = 0; i < 300000; i++) {
        struct nvkvm_broker_cursor_cmd c;
        const char *why;
        int v;

        memset(&c, 0, sizeof c);
        c.type = NVKVM_BROKER_CMD_CURSOR;
        c.op = (rnd() % 8) ? NVKVM_BROKER_CURSOR_SET : edgy(3);
        c.flags = (rnd() % 16) ? 0 : (uint16_t)rnd();
        c.reserved1 = (rnd() % 16) ? 0 : rnd();
        c.fourcc = (rnd() % 8) ? AR24 : rnd();
        c.width = edgy(NVKVM_BROKER_CURSOR_MAX_DIM);
        c.height = edgy(NVKVM_BROKER_CURSOR_MAX_DIM);
        c.stride = (rnd() % 2) ? c.width * 4u : edgy(NVKVM_BROKER_CURSOR_MAX_STRIDE);
        c.hot_x = edgy(c.width);
        c.hot_y = edgy(c.height);
        c.offset = edgy(4096);
        v = nb_cursor_check(&c, true, (i & 1) != 0, &why);
        if (v != NB_CURSOR_OK || c.op != NVKVM_BROKER_CURSOR_SET) {
            continue;
        }
        accepted++;
        /* What every caller of an accepted SET relies on. */
        if (c.width < 1 || c.width > NVKVM_BROKER_CURSOR_MAX_DIM ||
            c.height < 1 || c.height > NVKVM_BROKER_CURSOR_MAX_DIM ||
            c.hot_x >= c.width || c.hot_y >= c.height ||
            c.stride < c.width * 4u ||
            c.stride > NVKVM_BROKER_CURSOR_MAX_STRIDE ||
            nb_cursor_span(c.width, c.height, c.stride) > sizeof src ||
            nb_cursor_span(c.width, c.height, c.stride) < c.width * 4u) {
            bad++;
            continue;
        }
        /* Exercise the copy and the scaler on a subset: under ASAN an
         * out-of-range index in either is a hard failure. */
        if (i % 16 == 0) {
            struct nb_cursor_geom g;
            uint32_t nx = edgy(8192), dx = edgy(8192);
            uint32_t ny = edgy(8192), dy = edgy(8192);

            nb_cursor_load(&cur, src, c.width, c.height, c.stride, c.hot_x,
                           c.hot_y);
            nb_cursor_scaled_geom(&cur, nx, dx, ny, dy, NB_CURSOR_SCALED_MAX,
                                  &g);
            if (g.w < 1 || g.w > NB_CURSOR_SCALED_MAX || g.h < 1 ||
                g.h > NB_CURSOR_SCALED_MAX || g.hot_x >= g.w ||
                g.hot_y >= g.h) {
                bad++;
                continue;
            }
            slot[(size_t)g.w * g.h] = 0xa5a5a5a5u;
            nb_cursor_scale(&cur, slot, g.w, g.h);
            if (slot[(size_t)g.w * g.h] != 0xa5a5a5a5u) {
                bad++;
            }
            /* Premultiplied after the copy, whatever the source held. */
            {
                size_t k, n = (size_t)cur.w * cur.h;

                for (k = 0; k < n; k++) {
                    uint32_t p = cur.px[k], a = p >> 24;

                    if (((p >> 16) & 0xff) > a || ((p >> 8) & 0xff) > a ||
                        (p & 0xff) > a) {
                        bad++;
                        break;
                    }
                }
            }
        }
    }
    CHECK(accepted > 1000, "the sweep reached the accept path (%u)", accepted);
    CHECK(bad == 0, "%u accepted records broke an invariant", bad);
}

/* The record check with the XOR capability advertised or not. */
static int verdict_xor(const struct nvkvm_broker_cursor_cmd *c, bool fd, bool xor_ok)
{
    const char *why = NULL;
    int v = nb_cursor_check(c, fd, xor_ok, &why);

    CHECK(why != NULL, "nb_cursor_check left *why unset");
    if (why) {
        CHECK((v == NB_CURSOR_OK) == (why[0] == '\0'),
              "xor verdict %d with reason '%s'", v, why);
    }
    return v;
}

/*
 * NVKVM_BROKER_CURSOR_F_XOR: allowed only on a SET and only when the broker
 * advertised NVKVM_BROKER_CAP_CURSOR_STREAM; any other flag bit is a violation
 * either way; HIDE/SHOW never carry it.  Without the capability the flag is
 * exactly what it was before (flags must be 0).
 */
static void test_xor_flag(void)
{
    struct nvkvm_broker_cursor_cmd c = base_set();
    uint16_t f;

    CHECK(verdict_xor(&c, true, false) == NB_CURSOR_OK, "plain SET, no cap");
    CHECK(verdict_xor(&c, true, true) == NB_CURSOR_OK, "plain SET, cap");
    c.flags = NVKVM_BROKER_CURSOR_F_XOR;
    CHECK(verdict_xor(&c, true, false) == NB_CURSOR_VIOLATION,
          "F_XOR without the capability is a framing violation");
    CHECK(verdict_xor(&c, true, true) == NB_CURSOR_OK, "F_XOR with the capability");
    /* the content rules are the same for an XOR SET */
    c.width = 257;
    CHECK(verdict_xor(&c, true, true) == NB_CURSOR_REJECT, "257 wide XOR SET");
    c = base_set();
    c.flags = NVKVM_BROKER_CURSOR_F_XOR;
    c.fourcc = 0x1234;
    CHECK(verdict_xor(&c, true, true) == NB_CURSOR_REJECT, "XOR SET, wrong fourcc");
    c = base_set();
    c.flags = NVKVM_BROKER_CURSOR_F_XOR;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "XOR SET without an fd");
    for (f = 2; f != 0; f = (uint16_t)(f << 1)) {
        c = base_set();
        c.flags = f;
        CHECK(verdict_xor(&c, true, true) == NB_CURSOR_VIOLATION, "flag bit %#x", f);
        c.flags = (uint16_t)(f | NVKVM_BROKER_CURSOR_F_XOR);
        CHECK(verdict_xor(&c, true, true) == NB_CURSOR_VIOLATION, "flag bits %#x|1", f);
    }
    c = base_set();
    c.op = NVKVM_BROKER_CURSOR_HIDE;
    c.width = c.height = c.stride = c.fourcc = c.hot_x = c.hot_y = 0;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_OK, "HIDE");
    c.flags = NVKVM_BROKER_CURSOR_F_XOR;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "HIDE with F_XOR");
    c.op = NVKVM_BROKER_CURSOR_SHOW;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "SHOW with F_XOR");
}

static struct nb_cursor xor_cur; /* 256 KiB: not on the stack */

static void xor_set(uint32_t x, uint32_t y)
{
    xor_cur.px[y * xor_cur.w + x] = 0xff000000u | 0x00ffffffu; /* mask: inverts here */
}

/*
 * nb_cursor_xor_approx(): an XOR SET's mask becomes a visible cursor -- every
 * inverting pixel opaque BLACK, every other pixel touching one (8-neighbourhood,
 * inside the image) opaque WHITE, the rest transparent; the hot spot and size
 * are kept.  Shape: the measured Windows I-beam (run 2990, a 256x256 image,
 * 26 inverting pixels: serifs on rows 2 and 17 at x 7..9 and 11..13, a stem on
 * x 10 over rows 3..16).
 */
static void test_xor_approx(void)
{
    static const uint32_t BLACK = 0xff000000u, WHITE = 0xffffffffu;
    uint32_t x, y, fill = 0, outline = 0, other = 0;

    memset(&xor_cur, 0, sizeof xor_cur);
    xor_cur.w = xor_cur.h = 256;
    xor_cur.hot_x = 5;
    xor_cur.hot_y = 6;
    xor_cur.defined = true;
    for (y = 2; y <= 17; y += 15) {
        for (x = 7; x <= 13; x++) {
            if (x != 10) {
                xor_set(x, y);
            }
        }
    }
    for (y = 3; y <= 16; y++) {
        xor_set(10, y);
    }
    nb_cursor_xor_approx(&xor_cur);
    CHECK(xor_cur.w == 256 && xor_cur.h == 256, "size kept");
    CHECK(xor_cur.hot_x == 5 && xor_cur.hot_y == 6, "hot spot kept");
    CHECK(xor_cur.defined, "still defined");
    for (y = 0; y < 256; y++) {
        for (x = 0; x < 256; x++) {
            uint32_t p = xor_cur.px[y * 256 + x];
            int is_fill = (y == 2 || y == 17) ? (x >= 7 && x <= 13 && x != 10)
                                              : (y >= 3 && y <= 16 && x == 10);
            int near = 0, dy, dx;

            for (dy = -1; dy <= 1; dy++) {
                for (dx = -1; dx <= 1; dx++) {
                    int yy = (int)y + dy, xx = (int)x + dx;

                    if (yy >= 0 && yy < 256 && xx >= 0 && xx < 256 &&
                        ((yy == 2 || yy == 17) ? (xx >= 7 && xx <= 13 && xx != 10)
                                               : (yy >= 3 && yy <= 16 && xx == 10))) {
                        near = 1;
                    }
                }
            }
            if (is_fill) {
                CHECK(p == BLACK, "fill %u,%u is %08x", x, y, p);
                fill++;
            } else if (near) {
                CHECK(p == WHITE, "outline %u,%u is %08x", x, y, p);
                outline++;
            } else {
                CHECK(p == 0, "%u,%u should be transparent, is %08x", x, y, p);
                other += p != 0;
            }
        }
    }
    CHECK(fill == 26, "26 inverting pixels, %u filled", fill);
    CHECK(outline > 0 && other == 0, "outline %u, stray %u", outline, other);
    /* the diagonal corner, the gap in the top serif, two away */
    CHECK(xor_cur.px[1 * 256 + 6] == WHITE, "diagonal of the serif's corner");
    CHECK(xor_cur.px[2 * 256 + 10] == WHITE, "the gap in the serif");
    CHECK(xor_cur.px[2 * 256 + 5] == 0, "two left of the serif");
    CHECK(xor_cur.px[19 * 256 + 10] == 0, "two below the lower serif");
    /* deterministic: the same mask gives the same cursor */
    {
        static struct nb_cursor a, b;

        memset(&a, 0, sizeof a);
        a.w = a.h = 64;
        a.defined = true;
        a.px[5 * 64 + 5] = 0xff123456u;
        a.px[6 * 64 + 9] = 0xff000000u;
        memcpy(&b, &a, sizeof b);
        nb_cursor_xor_approx(&a);
        nb_cursor_xor_approx(&b);
        CHECK(memcmp(&a, &b, sizeof a) == 0, "same input, same output");
        CHECK(a.px[5 * 64 + 5] == BLACK && a.px[6 * 64 + 9] == BLACK,
              "the colour channels do not matter, the alpha does");
    }
    /* an empty mask is a transparent image (nothing to show) */
    memset(xor_cur.px, 0, sizeof xor_cur.px);
    CHECK(!nb_cursor_xor_approx(&xor_cur), "an empty mask draws nothing (false)");
    for (x = 0; x < 256 * 256; x++) {
        if (xor_cur.px[x]) {
            CHECK(0, "an empty XOR mask drew pixel %u = %08x", x, xor_cur.px[x]);
            break;
        }
    }
    /* the outline is clipped at every edge and corner, never wrapped */
    memset(&xor_cur, 0, sizeof xor_cur);
    xor_cur.w = 40;
    xor_cur.h = 24;
    xor_cur.defined = true;
    xor_set(0, 0);
    xor_set(39, 23);
    CHECK(nb_cursor_xor_approx(&xor_cur), "something drawn (true)");
    CHECK(xor_cur.px[0] == BLACK && xor_cur.px[1] == WHITE && xor_cur.px[40] == WHITE &&
          xor_cur.px[41] == WHITE, "top-left corner: fill + 3 outline");
    CHECK(xor_cur.px[39 + 23 * 40] == BLACK && xor_cur.px[38 + 23 * 40] == WHITE &&
          xor_cur.px[39 + 22 * 40] == WHITE && xor_cur.px[38 + 22 * 40] == WHITE,
          "bottom-right corner: fill + 3 outline");
    for (x = 0; x < 40 * 24; x++) {
        fill += xor_cur.px[x] != 0;
    }
    CHECK(fill == 26 + 8, "only 8 pixels drawn (%u counted incl. the earlier 26)", fill);
    CHECK(xor_cur.px[23 * 40] == 0 && xor_cur.px[39] == 0, "no wrap to the other edge");
    /* a mask pixel carrying colour but alpha 0 does not invert */
    memset(&xor_cur, 0, sizeof xor_cur);
    xor_cur.w = xor_cur.h = 8;
    xor_cur.defined = true;
    xor_cur.px[9] = 0x00ffffffu;
    nb_cursor_xor_approx(&xor_cur);
    CHECK(xor_cur.px[9] == 0, "alpha is the mask");
    /* a 1x1 image works */
    memset(&xor_cur, 0, sizeof xor_cur);
    xor_cur.w = xor_cur.h = 1;
    xor_cur.defined = true;
    xor_cur.px[0] = 0xff000001u;
    nb_cursor_xor_approx(&xor_cur);
    CHECK(xor_cur.px[0] == BLACK, "1x1");
}

/*
 * CURSOR_MOVE (CAP_CURSOR_STREAM): framing, the clamp, and what a backend draws.
 */
static struct nvkvm_broker_cursor_cmd move_cmd(int32_t x, int32_t y)
{
    struct nvkvm_broker_cursor_cmd c;

    memset(&c, 0, sizeof c);
    c.type = NVKVM_BROKER_CMD_CURSOR;
    c.op = NVKVM_BROKER_CURSOR_MOVE;
    c.hot_x = (uint32_t)x;
    c.hot_y = (uint32_t)y;
    return c;
}

/* A MOVE carrying the absolute-pointer pair (flag ABS, width/height = x/y). */
static struct nvkvm_broker_cursor_cmd move_abs_cmd(int32_t x, int32_t y, int32_t ax, int32_t ay)
{
    struct nvkvm_broker_cursor_cmd c = move_cmd(x, y);

    c.flags = NVKVM_BROKER_CURSOR_F_ABS;
    c.width = (uint32_t)ax;
    c.height = (uint32_t)ay;
    return c;
}

static void test_move(void)
{
    struct nvkvm_broker_cursor_cmd c = move_cmd(-5, 700);
    int32_t ox = 99, oy = 99;

    CHECK(verdict_xor(&c, false, false) == NB_CURSOR_VIOLATION,
          "MOVE without CAP_CURSOR_STREAM is an unknown-op violation");
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_OK, "MOVE with the capability");
    CHECK(verdict_xor(&c, true, true) == NB_CURSOR_VIOLATION, "MOVE with an fd");
    c.flags = 2;    /* bit 0 is F_ABS on a MOVE (as it is F_XOR on a SET) */
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "MOVE with an unknown flag");
    c = move_cmd(0, 0);
    c.width = 1;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "MOVE with a width");
    /* the absolute-pointer pair: the flag says width/height are it; without the
     * flag they are zero, with it only a MOVE may carry it */
    c = move_abs_cmd(5, 6, 7, 8);
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_OK, "MOVE with the ABS pair");
    CHECK(verdict_xor(&c, false, false) == NB_CURSOR_VIOLATION, "ABS pair without the capability");
    c.stride = 4;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "ABS pair with a stride");
    c = move_abs_cmd(5, 6, 7, 8);
    c.flags = NVKVM_BROKER_CURSOR_F_ABS | 2;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "MOVE with another flag bit");
    c = move_cmd(0, 0);
    c.stride = 4;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "MOVE with a stride");
    c = move_cmd(0, 0);
    c.offset = 4;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "MOVE with an offset");
    c = move_cmd(0, 0);
    c.fourcc = 4;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "MOVE with a fourcc");
    c = move_cmd(0, 0);
    c.reserved1 = 1;
    CHECK(verdict_xor(&c, false, true) == NB_CURSOR_VIOLATION, "MOVE with reserved1");

    /* the clamp: inside is kept, past the frame is pinned, absurd is refused */
    CHECK(nb_cursor_move_clamp(100, 200, &ox, &oy) == 0 && ox == 100 && oy == 200, "inside");
    CHECK(nb_cursor_move_clamp(-256, -256, &ox, &oy) == 0 && ox == -256 && oy == -256,
          "hanging off the top-left by a whole cursor");
    CHECK(nb_cursor_move_clamp(-300, -9000, &ox, &oy) == 0 &&
          ox == -(int32_t)NVKVM_BROKER_CURSOR_MAX_DIM && oy == -(int32_t)NVKVM_BROKER_CURSOR_MAX_DIM,
          "clamped at -MAX_DIM");
    CHECK(nb_cursor_move_clamp(9000, 8193, &ox, &oy) == 0 &&
          ox == (int32_t)NVKVM_BROKER_MAX_DIM && oy == (int32_t)NVKVM_BROKER_MAX_DIM,
          "clamped at MAX_DIM");
    ox = oy = 7;
    CHECK(nb_cursor_move_clamp(NVKVM_BROKER_CURSOR_MOVE_LIMIT + 1, 0, &ox, &oy) == -1 &&
          ox == 7 && oy == 7, "past the limit: refused, untouched");
    CHECK(nb_cursor_move_clamp(0, -NVKVM_BROKER_CURSOR_MOVE_LIMIT - 1, &ox, &oy) == -1, "negative limit");
    CHECK(nb_cursor_move_clamp(INT32_MIN, INT32_MAX, &ox, &oy) == -1, "int32 extremes");
    CHECK(nb_cursor_move_clamp(NVKVM_BROKER_CURSOR_MOVE_LIMIT, -NVKVM_BROKER_CURSOR_MOVE_LIMIT,
                               &ox, &oy) == 0 && ox == (int32_t)NVKVM_BROKER_MAX_DIM,
          "the limit itself is clamped, not refused");

    /*
     * What a backend draws: hover = the host pointer, MOVE ignored; grab = an
     * overlay at the last MOVE, only with the capability and a MOVE received;
     * hidden (HIDE: the guest hid it, or its cursor plane is off and it draws
     * a software cursor into the frame) = nothing in any mode.
     */
    memset(&cur, 0, sizeof cur);
    CHECK(nb_cursor_policy(&cur, false, true) == NB_CURSOR_NONE, "no image: nothing");
    CHECK(nb_cursor_policy(NULL, false, true) == NB_CURSOR_NONE, "no cursor at all");
    cur.w = cur.h = 32;
    cur.defined = cur.visible = true;
    CHECK(nb_cursor_policy(&cur, false, true) == NB_CURSOR_POINTER, "hover: the host pointer");
    CHECK(nb_cursor_policy(&cur, false, false) == NB_CURSOR_POINTER, "hover, no stream: the same");
    CHECK(nb_cursor_policy(&cur, true, true) == NB_CURSOR_NONE, "grab, never MOVEd: nothing yet");
    cur.x = 10;
    cur.y = 20;
    cur.placed = true;
    CHECK(nb_cursor_policy(&cur, false, true) == NB_CURSOR_POINTER, "hover ignores the position");
    CHECK(nb_cursor_policy(&cur, true, true) == NB_CURSOR_OVERLAY, "grab: the overlay at x,y");
    CHECK(nb_cursor_policy(&cur, true, false) == NB_CURSOR_NONE,
          "grab, a VMM that composes (no stream): the broker draws nothing");
    cur.visible = false;
    CHECK(nb_cursor_policy(&cur, true, true) == NB_CURSOR_NONE, "HIDE: nothing under grab");
    CHECK(nb_cursor_policy(&cur, false, true) == NB_CURSOR_NONE, "HIDE: nothing in hover");
    /* the position survives a publish */
    {
        static struct nb_cursor pub;

        cur.visible = true;
        nb_cursor_copy(&pub, &cur);
        CHECK(pub.x == 10 && pub.y == 20 && pub.placed, "the published copy carries the position");
    }
}

/*
 * THE HOT SPOT FROM THE POINTER (hover).  The Windows driver programs hot 0,0 while the real spot
 * is inside the image; the VMM sends, in each MOVE, the image's top-left and the last absolute
 * pointer position it injected (valid flag clear after relative input), the broker derives
 * hot = abs - top-left.  Rules ported from the VMM's Rust HotTracker (which is kept for the
 * composing fallback), with the owner's latch: the guest moves its cursor a little after it gets
 * an injected event, so the pair means something only when the pointer is stationary -- a
 * derived hot spot is LATCHED only when the SAME difference appears in two consecutive MOVEs
 * (event-counted, no timer); until then, and after every new image, the SET hot spot stands.
 */
static struct nb_hot hot_st;
static struct nb_cursor hot_cur;

static void hot_new_image(uint32_t w, uint32_t h, uint32_t hx, uint32_t hy)
{
    memset(&hot_cur, 0, sizeof hot_cur);
    hot_cur.w = w;
    hot_cur.h = h;
    hot_cur.hot_x = hx;
    hot_cur.hot_y = hy;
    hot_cur.defined = hot_cur.visible = true;
    nb_hot_reset(&hot_st);
}

/* One MOVE: top-left (x,y), absolute pointer (ax,ay) valid or not; whether the hot spot changed. */
static bool hot_move(int32_t x, int32_t y, bool valid, int32_t ax, int32_t ay)
{
    hot_cur.x = x;
    hot_cur.y = y;
    hot_cur.placed = true;
    return nb_hot_move(&hot_st, &hot_cur, valid, ax, ay);
}

static void test_hot_latch(void)
{
    /* a new image keeps the SET (guest-programmed) hot spot */
    hot_new_image(32, 32, 0, 0);
    CHECK(hot_cur.hot_x == 0 && hot_cur.hot_y == 0, "the SET hot spot");
    /* one pair is not enough: the guest may still be moving its cursor */
    CHECK(!hot_move(100, 200, true, 104, 209), "one pair: not latched");
    CHECK(hot_cur.hot_x == 0 && hot_cur.hot_y == 0, "SET hot spot stands");
    /* the same difference again: latched, and the change is reported */
    CHECK(hot_move(100, 200, true, 104, 209), "two equal differences: latched");
    CHECK(hot_cur.hot_x == 4 && hot_cur.hot_y == 9, "hot = abs - top-left");
    CHECK(!hot_move(100, 200, true, 104, 209), "unchanged: nothing to publish");
    /* rounding: one pixel either way is not a change (a moving pointer, one pixel of scaling) */
    CHECK(!hot_move(100, 200, true, 105, 208), "1 px off: candidate");
    CHECK(!hot_move(100, 200, true, 105, 208), "1 px off twice: still the latched spot");
    CHECK(hot_cur.hot_x == 4 && hot_cur.hot_y == 9, "within a pixel: unchanged");
    /* more than a pixel, seen twice: corrected */
    CHECK(!hot_move(100, 200, true, 110, 212), "far: candidate only");
    CHECK(hot_move(100, 200, true, 110, 212), "far twice: re-latched");
    CHECK(hot_cur.hot_x == 10 && hot_cur.hot_y == 12, "the corrected hot spot");

    /* the SAME difference at a DIFFERENT pointer is a cursor trailing a moving pointer, not
     * stationarity: no latch (the live scripted-motion case) */
    hot_new_image(32, 32, 1, 2);
    CHECK(!hot_move(100, 200, true, 105, 205), "p1");
    CHECK(!hot_move(110, 210, true, 115, 215), "same difference 5,5 at another pointer: not a latch");
    CHECK(!hot_move(120, 220, true, 125, 225), "and again");
    CHECK(hot_cur.hot_x == 1 && hot_cur.hot_y == 2, "SET hot spot");
    CHECK(hot_move(120, 220, true, 125, 225), "the pointer holds still: the same pair twice latches");
    CHECK(hot_cur.hot_x == 5 && hot_cur.hot_y == 5, "5,5");
    /* a moving pointer (a new difference every event) never latches */
    hot_new_image(32, 32, 1, 2);
    CHECK(!hot_move(0, 0, true, 3, 3), "a");
    CHECK(!hot_move(0, 0, true, 6, 6), "b: a different difference");
    CHECK(!hot_move(0, 0, true, 9, 9), "c");
    CHECK(hot_cur.hot_x == 1 && hot_cur.hot_y == 2, "a moving pointer keeps the SET hot spot");
    /* ... and a non-consecutive repeat does not latch either */
    CHECK(!hot_move(0, 0, true, 3, 3), "d: 3,3 again but not consecutive");
    CHECK(hot_cur.hot_x == 1 && hot_cur.hot_y == 2, "still the SET hot spot");

    /* implausible (the pointer is outside the image): ignored, and the candidate is dropped */
    hot_new_image(32, 32, 1, 2);
    CHECK(!hot_move(100, 100, true, 100, 140), "y 40 is outside a 32-high image");
    CHECK(!hot_move(100, 100, true, 100, 140), "twice: still ignored");
    CHECK(!hot_move(100, 100, true, 99, 100), "x -1 is outside too");
    CHECK(!hot_move(100, 100, true, 105, 105), "plausible: candidate");
    CHECK(!hot_move(100, 100, true, 100, 140), "an implausible one in between drops it");
    CHECK(!hot_move(100, 100, true, 105, 105), "so this is a candidate again");
    CHECK(hot_move(100, 100, true, 105, 105), "and now it latches");
    CHECK(hot_cur.hot_x == 5 && hot_cur.hot_y == 5, "5,5");
    /* the image's far edge: w-1 is inside, w is not */
    hot_new_image(32, 32, 0, 0);
    CHECK(!hot_move(0, 0, true, 31, 31) && hot_move(0, 0, true, 31, 31) && hot_cur.hot_x == 31, "31,31 inside");
    hot_new_image(32, 32, 0, 0);
    CHECK(!hot_move(0, 0, true, 32, 0) && !hot_move(0, 0, true, 32, 0), "32 is outside");

    /* relative input (valid flag clear): the pair means nothing; the candidate is dropped, the hot
     * spot already latched is kept (hot spot is irrelevant when the image is drawn at the top-left) */
    hot_new_image(32, 32, 0, 0);
    CHECK(!hot_move(0, 0, true, 7, 7), "candidate");
    CHECK(!hot_move(0, 0, false, 0, 0), "relative: dropped");
    CHECK(!hot_move(0, 0, true, 7, 7), "candidate again, not a latch");
    CHECK(hot_move(0, 0, true, 7, 7), "latched");
    CHECK(!hot_move(0, 0, false, 99, 99), "relative with garbage in the pair: ignored");
    CHECK(hot_cur.hot_x == 7 && hot_cur.hot_y == 7, "kept");

    /* a new image resets everything: the SET hot spot again, no latch, no candidate */
    hot_new_image(48, 48, 2, 3);
    CHECK(hot_cur.hot_x == 2 && hot_cur.hot_y == 3, "new image: SET hot spot");
    CHECK(!hot_move(0, 0, true, 7, 7), "a fresh candidate, not a latch carried over");

    /* a hostile guest cannot make it read outside: no image, a zero-size image, extreme values */
    memset(&hot_cur, 0, sizeof hot_cur);
    nb_hot_reset(&hot_st);
    CHECK(!nb_hot_move(&hot_st, &hot_cur, true, 5, 5), "no image: nothing");
    hot_new_image(32, 32, 0, 0);
    CHECK(!hot_move(INT32_MIN, INT32_MAX, true, INT32_MAX, INT32_MIN), "extremes: implausible, no overflow");
    CHECK(!hot_move(INT32_MAX, 0, true, INT32_MIN, 0), "difference overflow is implausible, not a wrap");
}

/*
 * The scripted stream of the live defect (2026-10-15): the Windows I-beam, programmed hot 0,0, whose
 * pointing spot is (4,8) inside the image.  The guest places the image's top-left at pointer - (4,8)
 * but only AFTER it gets the injected event, so during motion the top-left trails the pointer; when
 * the pointer stops it settles.  Placement in hover = the host pointer sits on the image's pointing
 * spot, at window scale 1, 1.5 and 2: the logical hot spot (nb_cursor_scaled_geom, what
 * wl_pointer.set_cursor takes) lands on the scaled pointing spot.
 */
static void test_hot_scripted_stream(void)
{
    static const uint32_t scales[][2] = { {1, 1}, {3, 2}, {2, 1} };   /* num, den */
    unsigned si;

    for (si = 0; si < 3; si++) {
        int32_t px = 300, py = 400, i;
        struct nb_cursor_geom g;
        bool latched_in_motion = false;

        hot_new_image(256, 256, 0, 0);
        /* motion: 12 injected positions 5 px apart; the cursor trails the previous one */
        for (i = 0; i < 12; i++) {
            px += 5;
            py += 3;
            latched_in_motion |= hot_move(px - 5 - 4, py - 3 - 8, true, px, py);
        }
        CHECK(!latched_in_motion, "a trailing cursor never latches (scale %u/%u)", scales[si][0], scales[si][1]);
        CHECK(hot_cur.hot_x == 0 && hot_cur.hot_y == 0, "SET hot spot during motion");
        /* the pointer stops: the cursor settles, and the SAME pair arrives twice */
        CHECK(!hot_move(px - 4, py - 8, true, px, py), "the first settled MOVE is only a candidate");
        CHECK(hot_move(px - 4, py - 8, true, px, py), "the second equal one latches");
        CHECK(hot_cur.hot_x == 4 && hot_cur.hot_y == 8, "hot = pointer - top-left = 4,8");
        nb_cursor_scaled_geom(&hot_cur, scales[si][0], scales[si][1], scales[si][0], scales[si][1],
                              NB_CURSOR_SCALED_MAX, &g);
        /* the hot spot, in logical pixels of the scaled image, is the pointing spot scaled
         * (rounded up where the scaler skips a pixel) */
        CHECK(g.hot_x == (4 * g.w + 255) / 256 && g.hot_y == (8 * g.h + 255) / 256,
              "scale %u/%u: hot %u,%u of %ux%u", scales[si][0], scales[si][1], g.hot_x, g.hot_y, g.w, g.h);
        CHECK(g.hot_x == 4 * scales[si][0] / scales[si][1] && g.hot_y == 8 * scales[si][0] / scales[si][1],
              "scale %u/%u: the pointing spot (4,8) scaled", scales[si][0], scales[si][1]);
        /* a latched hot spot is applied: the published copy carries it (what a backend re-sets) */
        {
            static struct nb_cursor pub;

            nb_cursor_copy(&pub, &hot_cur);
            CHECK(pub.hot_x == 4 && pub.hot_y == 8, "the published copy carries the latched hot spot");
        }
    }
    /* A SINGLE settled MOVE (a VMM that sends an unchanged pair once) never latches: the VMM must
     * confirm a settled pair with a second MOVE -- this is what the live run showed */
    hot_new_image(256, 256, 0, 0);
    hot_move(300, 400, true, 310, 410);        /* the lag */
    CHECK(!hot_move(306, 402, true, 310, 410), "one settled MOVE");
    CHECK(hot_cur.hot_x == 0 && hot_cur.hot_y == 0, "not latched by one settled MOVE");
}

int main(void)
{
    test_hot_scripted_stream();
    test_hot_latch();
    test_move();
    test_xor_flag();
    test_xor_approx();
    test_framing();
    test_content();
    test_extent();
    test_load();
    test_geom();
    test_scale_pixels();
    test_hot_pixel();
    test_hot_sweep();
    test_device_geom();
    test_surface_geom();
    test_xor_compose();
    test_copy();
    test_wanted();
    test_fuzz();
    printf("test_cursor: %u checks, %u failed\n", n_checks, n_fail);
    return n_fail ? 1 : 0;
}
