/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nb_geom.c — two small pure rules of the Wayland backend, kept here so that
 * test/test_geom.c drives them without a compositor, a socket or a GPU:
 *
 *   nb_hint_width()  the width the broker SUGGESTS to the guest;
 *   nb_geom_set      the buffer geometries a display has been PROVEN to import.
 *
 * Both exist because of the 2026-10-15 incident (run 2990, GNOME/mutter on an
 * AMD iGPU, guest on an NVIDIA GPU): the window was maximized and restored, the
 * guest followed it to 2080x1171, and the compositor refused the LINEAR dma-buf
 * import of that size -- "EGL failed to allocate resources" -- as a fatal
 * protocol error on a pair that had been proven at 1920x1080.  Proof was kept
 * per (fourcc, modifier) only; an import depends on the geometry as well.
 */
#include <string.h>

#include "nvkvm_broker.h"

unsigned nb_hint_width(unsigned w)
{
    unsigned a = w & ~(NB_HINT_W_ALIGN - 1u);

    /*
     * A width below one alignment unit still has to be suggestable: fall back
     * to the CVT granularity (8) the guest's mode synthesis applies, which is
     * the rule this replaces.  0 stays 0: "suggest nothing".
     */
    return a ? a : (w & ~7u);
}

bool nb_geom_has(const struct nb_geom_set *s, uint32_t fourcc, uint64_t mod,
                 uint32_t w, uint32_t h, uint32_t stride, uint32_t offset)
{
    for (unsigned i = 0; i < NB_GEOM_SLOTS; i++) {
        const struct nb_geom *g = &s->e[i];

        if (g->used && g->fourcc == fourcc && g->modifier == mod &&
            g->width == w && g->height == h && g->stride == stride &&
            g->offset == offset) {
            return true;
        }
    }
    return false;
}

void nb_geom_add(struct nb_geom_set *s, uint32_t fourcc, uint64_t mod,
                 uint32_t w, uint32_t h, uint32_t stride, uint32_t offset)
{
    if (nb_geom_has(s, fourcc, mod, w, h, stride, offset)) {
        return;
    }
    /* A ring: the oldest proof is the one forgotten, and forgetting costs one
     * probe (one dropped frame), never a connection. */
    s->e[s->next] = (struct nb_geom){
        .used = true, .fourcc = fourcc, .modifier = mod,
        .width = w, .height = h, .stride = stride, .offset = offset,
    };
    s->next = (s->next + 1) % NB_GEOM_SLOTS;
}

void nb_geom_clear(struct nb_geom_set *s)
{
    memset(s, 0, sizeof *s);
}
