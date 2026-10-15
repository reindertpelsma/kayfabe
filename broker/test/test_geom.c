/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * test_geom.c — the suggested-width rule and the proven-geometry set, driven
 * directly.  No socket, no display, no GPU.  Linked against the SAME nb_geom.c
 * the broker links.
 *
 * The rule under test comes from the 2026-10-15 incident: a guest that followed
 * the window to 2080x1171 produced a host frame with a 8320-byte row (a multiple
 * of 128, not of 256), which an AMD-iGPU compositor refused to import.  A
 * suggested width that is a multiple of 64 makes the 32-bit row a multiple of
 * 256 bytes, so the broker's own suggestions never produce the pitch.
 */
#include <stdio.h>
#include <stdlib.h>

#include "../nvkvm_broker.h"

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

static void test_hint(void)
{
    /* the sizes of the incident */
    CHECK(nb_hint_width(2080) == 2048, "2080 -> %u", nb_hint_width(2080));
    CHECK(nb_hint_width(1920) == 1920, "1920 stays");
    CHECK(nb_hint_width(2048) == 2048, "2048 stays");
    CHECK(nb_hint_width(3672) == 3648, "3672 -> %u", nb_hint_width(3672));
    CHECK(nb_hint_width(2880) == 2880, "2880 (1920 at scale 1.5) stays");
    CHECK(nb_hint_width(3840) == 3840, "3840 stays");

    /* the rule, for every width the broker could be asked about */
    for (unsigned w = 0; w <= 16384; w++) {
        unsigned r = nb_hint_width(w);

        CHECK(r <= w, "%u -> %u grew", w, r);
        if (w >= NB_HINT_W_ALIGN) {
            CHECK(r % NB_HINT_W_ALIGN == 0, "%u -> %u not a multiple of %u",
                  w, r, NB_HINT_W_ALIGN);
            CHECK(w - r < NB_HINT_W_ALIGN, "%u -> %u lost a whole unit", w, r);
            /* the 32-bit row of the guest's frame is a multiple of 256 */
            CHECK((r * 4u) % 256u == 0, "%u -> %u row %u", w, r, r * 4u);
        } else {
            CHECK(r % 8 == 0 && w - r < 8, "%u -> %u (small width)", w, r);
        }
    }
    CHECK(nb_hint_width(0) == 0, "0 stays 0 (suggest nothing)");
    CHECK(nb_hint_width(7) == 0, "7 -> 0 (suggest nothing)");
}

static void test_geom(void)
{
    struct nb_geom_set s;

    nb_geom_clear(&s);
    CHECK(!nb_geom_has(&s, XR24, 0, 1920, 1080, 7680, 0), "empty set has nothing");

    /* proof at 1920x1080 is not proof at 2080x1171: the incident */
    nb_geom_add(&s, XR24, 0, 1920, 1080, 7680, 0);
    CHECK(nb_geom_has(&s, XR24, 0, 1920, 1080, 7680, 0), "1920x1080 proven");
    CHECK(!nb_geom_has(&s, XR24, 0, 2080, 1171, 8320, 0), "2080x1171 NOT proven");
    CHECK(!nb_geom_has(&s, XR24, 0, 1920, 1080, 7936, 0), "another stride not proven");
    CHECK(!nb_geom_has(&s, XR24, 0, 1920, 1080, 7680, 4096), "another offset not proven");
    CHECK(!nb_geom_has(&s, XR24, 0x0300000000606014ull, 1920, 1080, 7680, 0),
          "another modifier not proven");
    CHECK(!nb_geom_has(&s, 0x34325241u, 0, 1920, 1080, 7680, 0), "another fourcc not proven");

    /* adding twice does not consume a second slot */
    nb_geom_add(&s, XR24, 0, 1920, 1080, 7680, 0);
    for (unsigned i = 1; i < NB_GEOM_SLOTS; i++) {
        nb_geom_add(&s, XR24, 0, 100 + i, 100, (100 + i) * 4, 0);
    }
    CHECK(nb_geom_has(&s, XR24, 0, 1920, 1080, 7680, 0),
          "the first proof survives NB_GEOM_SLOTS-1 others");

    /* the ring forgets the oldest, never refuses to learn */
    nb_geom_add(&s, XR24, 0, 2048, 1155, 8192, 0);
    CHECK(nb_geom_has(&s, XR24, 0, 2048, 1155, 8192, 0), "newest proof held");
    CHECK(!nb_geom_has(&s, XR24, 0, 1920, 1080, 7680, 0), "oldest proof forgotten");

    nb_geom_clear(&s);
    CHECK(!nb_geom_has(&s, XR24, 0, 2048, 1155, 8192, 0), "cleared with the client");
}

int main(void)
{
    test_hint();
    test_geom();
    if (n_fail) {
        fprintf(stderr, "test-geom: %u of %u checks FAILED\n", n_fail, n_checks);
        return 1;
    }
    printf("test-geom: %u checks passed\n", n_checks);
    return 0;
}
