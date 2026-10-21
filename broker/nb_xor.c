/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nb_xor.c -- everything the broker decides about an XOR cursor, as pure
 * functions plus one driver, so a backend (Wayland, test) keeps only the part
 * that talks to its display.
 *
 *   nb_xor_decide()        THE STATE MACHINE: cursor kind x hover/grab x frame
 *                          readable x pointer inside x enter serial -> what the
 *                          host pointer does, what the overlay shows, and which
 *                          choice (plain / composed / approximated) stands.
 *                          This is the place the old bug lived (an XOR cursor
 *                          and an ARGB one fighting over the host pointer,
 *                          across two processes); it is a table here, and
 *                          test/test_xor.c walks the whole table and mutates it.
 *   nb_xor_place()         where the patch lands, to within 0.5 logical px.
 *   nb_xor_ptr_to_frame()  the guest pixel under the host pointer.
 *   nb_xor_eval()          gather -> decide -> read ONLY the cursor's pixels ->
 *                          compose -> re-decide if the read failed.
 *
 * Nothing here touches a display connection or logs; the one log line a change
 * of choice earns is built by nb_xor_choice_line() and printed by the caller.
 */
#include <stdio.h>
#include <string.h>

#include "nvkvm_broker.h"

const char *nb_xor_choice_text(enum nb_xor_choice c)
{
    switch (c) {
    case NB_XC_NONE:     return "none";
    case NB_XC_PLAIN:    return "plain";
    case NB_XC_COMPOSED: return "composed";
    case NB_XC_APPROX:   return "approximated";
    }
    return "?";
}

struct nb_xor_out nb_xor_decide(const struct nb_xor_in *in)
{
    struct nb_xor_out o;
    /* the host pointer is ours to set only while it is over the content AND
     * the enter serial (the only valid one for set_cursor) is present */
    bool pa = in->ptr_inside && in->serial_ok;

    o.ptr = NB_XP_UNTOUCHED;
    o.ov = NB_XO_OFF;
    o.choice = in->kind == NB_XK_NONE  ? NB_XC_NONE
             : in->kind == NB_XK_PLAIN ? NB_XC_PLAIN
             : in->frame_readable      ? NB_XC_COMPOSED : NB_XC_APPROX;

    if (in->grabbed) {
        /* pointer locked: never a host image; the overlay carries the cursor
         * at the guest's own position */
        o.ptr = pa ? NB_XP_HIDDEN : NB_XP_UNTOUCHED;
        if (in->kind == NB_XK_NONE || !in->placed) {
            o.ov = NB_XO_OFF;
        } else if (in->kind == NB_XK_PLAIN) {
            o.ov = NB_XO_IMAGE;
        } else {
            o.ov = in->frame_readable ? NB_XO_XOR : NB_XO_IMAGE;
        }
        return o;
    }
    /* hover */
    if (!pa) {
        /* not our pointer to set (chrome, outside) or no serial to set it
         * with: leave it; and never draw an overlay beside a host pointer
         * we could not hide */
        return o;
    }
    switch (in->kind) {
    case NB_XK_NONE:
        o.ptr = NB_XP_HIDDEN;
        break;
    case NB_XK_PLAIN:
        o.ptr = NB_XP_GUEST_IMAGE;
        break;
    case NB_XK_XOR:
        if (in->frame_readable) {
            o.ptr = NB_XP_HIDDEN;
            o.ov = NB_XO_XOR;
        } else {
            o.ptr = NB_XP_GUEST_IMAGE;     /* the approximation, as before */
        }
        break;
    }
    return o;
}

static int64_t floor_div(int64_t a, int64_t b)
{
    int64_t q = a / b;

    if ((a % b != 0) && ((a < 0) != (b < 0))) {
        q--;
    }
    return q;
}

/* a * n / d rounded to nearest (ties up), exact in 64 bits for |a| < 2^21,
 * n, d < 2^32 */
static int64_t round_ratio(int64_t a, uint64_t n, uint64_t d)
{
    return floor_div(2 * a * (int64_t)n + (int64_t)d, 2 * (int64_t)d);
}

void nb_xor_place(int32_t tlx, int32_t tly, uint32_t pw, uint32_t ph,
                  uint32_t nx, uint32_t dx, uint32_t ny, uint32_t dy,
                  uint32_t scale_120, struct nb_xor_place *o)
{
    const int64_t LIM = 1 << 20;
    int64_t x0, x1, y0, y1, gw, gh;
    uint32_t k, maxl;

    if (nx == 0 || dx == 0) { nx = dx = 1; }
    if (ny == 0 || dy == 0) { ny = dy = 1; }
    if (tlx < -LIM) { tlx = (int32_t)-LIM; }
    if (tlx > LIM)  { tlx = (int32_t)LIM; }
    if (tly < -LIM) { tly = (int32_t)-LIM; }
    if (tly > LIM)  { tly = (int32_t)LIM; }
    if (pw > NVKVM_BROKER_CURSOR_MAX_DIM) { pw = NVKVM_BROKER_CURSOR_MAX_DIM; }
    if (ph > NVKVM_BROKER_CURSOR_MAX_DIM) { ph = NVKVM_BROKER_CURSOR_MAX_DIM; }

    x0 = round_ratio(tlx, nx, dx);
    x1 = round_ratio((int64_t)tlx + pw, nx, dx);
    y0 = round_ratio(tly, ny, dy);
    y1 = round_ratio((int64_t)tly + ph, ny, dy);
    gw = x1 - x0;
    gh = y1 - y0;
    if (gw < 1) { gw = 1; }
    if (gh < 1) { gh = 1; }

    if (scale_120 > 4u * 120u) {
        scale_120 = 4u * 120u;
    }
    k = (scale_120 + 119u) / 120u;
    if (k < 1u) {
        k = 1u;
    }
    maxl = NB_CURSOR_SURFACE_MAX_BUF / k;
    if (gw > maxl) { gw = maxl; }
    if (gh > maxl) { gh = maxl; }

    o->x = (int32_t)x0;
    o->y = (int32_t)y0;
    o->gw = (uint32_t)gw;
    o->gh = (uint32_t)gh;
    o->scale = k;
    o->bw = o->gw * k;
    o->bh = o->gh * k;
}

int32_t nb_xor_ptr_to_frame(int32_t fixed, uint32_t nx, uint32_t dx,
                            uint32_t frame_dim)
{
    int64_t q;

    if (frame_dim == 0) {
        return 0;
    }
    if (nx == 0 || dx == 0) { nx = dx = 1; }
    /* fixed/256 logical px * dx/nx guest px per logical px */
    q = floor_div((int64_t)fixed * (int64_t)dx, (int64_t)nx * 256);
    if (q < 0) { q = 0; }
    if (q > (int64_t)frame_dim - 1) { q = (int64_t)frame_dim - 1; }
    return (int32_t)q;
}

static void nb_xor_note_choice(struct nb_xor_ctx *ctx, struct nb_xor_res *r)
{
    enum nb_xor_choice ch = r->act.choice;
    enum nb_frame_why why = ch == NB_XC_APPROX ? r->why : NB_FW_OK;

    if (!ctx->choice_valid || ctx->last_choice != ch ||
        (ch == NB_XC_APPROX && ctx->last_why != why)) {
        r->choice_changed = true;
        ctx->choice_valid = true;
        ctx->last_choice = ch;
        ctx->last_why = why;
    }
}

void nb_xor_eval(struct nb_xor_ctx *ctx, const struct nb_xor_view *v,
                 struct nb_xor_res *r)
{
    const struct nb_cursor *c = v->cur;
    struct nb_xor_in in;
    bool is_xor;

    memset(r, 0, sizeof *r);
    r->why = NB_FW_OK;
    in.kind = (!c || !v->shown_ok || !c->defined || !c->visible) ? NB_XK_NONE
              : c->xor ? NB_XK_XOR : NB_XK_PLAIN;
    in.grabbed = v->grabbed;
    in.placed = c && c->placed;
    in.ptr_inside = v->ptr_inside;
    in.serial_ok = v->serial_ok;
    in.frame_readable = false;
    is_xor = in.kind == NB_XK_XOR;

    if (is_xor) {
        r->why = v->frame ? nb_frame_check(v->frame, &v->geom) : NB_FW_NONE;
        in.frame_readable = r->why == NB_FW_OK;
    }
    r->act = nb_xor_decide(&in);

    if (r->act.ov == NB_XO_XOR && !in.grabbed && !v->ptr_known) {
        r->act.ov = NB_XO_OFF;          /* hover with no pointer position yet */
    }
    if (r->act.ov == NB_XO_XOR) {
        int64_t tlx, tly, x0, y0, x1, y1;
        uint32_t *out = ctx->patch;

        if (in.grabbed) {
            tlx = (int64_t)c->x + c->bx;
            tly = (int64_t)c->y + c->by;
        } else {
            tlx = (int64_t)v->ptr_x - (int64_t)c->hot_x + c->bx;
            tly = (int64_t)v->ptr_y - (int64_t)c->hot_y + c->by;
        }
        r->pw = c->bw;
        r->ph = c->bh;
        /* the rectangle of the frame this patch can touch */
        x0 = tlx < 0 ? 0 : tlx;
        y0 = tly < 0 ? 0 : tly;
        x1 = tlx + c->bw > (int64_t)v->geom.w ? (int64_t)v->geom.w : tlx + c->bw;
        y1 = tly + c->bh > (int64_t)v->geom.h ? (int64_t)v->geom.h : tly + c->bh;
        if (c->bw == 0 || c->bh == 0 || tlx > 1 << 20 || tlx < -(1 << 20) ||
            tly > 1 << 20 || tly < -(1 << 20)) {
            r->act.ov = NB_XO_OFF;      /* nothing to invert */
        } else if (x1 <= x0 || y1 <= y0) {
            /* wholly off the frame: composed, and nothing to show */
            memset(out, 0, (size_t)c->bw * c->bh * sizeof out[0]);
            r->npix = 0;
        } else {
            uint32_t rw = (uint32_t)(x1 - x0), rh = (uint32_t)(y1 - y0);
            enum nb_frame_why w = nb_frame_read_rect(v->frame, (uint32_t)x0,
                                                     (uint32_t)y0, rw, rh,
                                                     ctx->rect);

            if (w != NB_FW_OK) {
                /* the read failed: the frame is NOT readable after all */
                r->why = w;
                in.frame_readable = false;
                r->act = nb_xor_decide(&in);
            } else {
                r->npix = (uint32_t)nb_xor_patch_rect(
                    c->mask + (size_t)c->by * c->w + c->bx, c->w, c->bw, c->bh,
                    ctx->rect, rw, rh, rw, (int32_t)(tlx - x0),
                    (int32_t)(tly - y0), out);
            }
        }
        if (r->act.ov == NB_XO_XOR) {
            size_t n = (size_t)c->bw * c->bh;

            r->patch = true;
            r->tlx = (int32_t)tlx;
            r->tly = (int32_t)tly;
            r->px = out;
            r->patch_changed = !ctx->att_valid || ctx->att_pw != r->pw ||
                               ctx->att_ph != r->ph || ctx->att_gen != c->gen ||
                               memcmp(ctx->att, out, n * sizeof out[0]) != 0;
            ctx->n_composed++;
        }
    }
    if (r->act.ov != NB_XO_XOR) {
        if (is_xor) {
            ctx->n_fallback += r->act.choice == NB_XC_APPROX;
        }
    }
    nb_xor_note_choice(ctx, r);
}

void nb_xor_attached(struct nb_xor_ctx *ctx, const struct nb_xor_res *r,
                     uint64_t cursor_gen)
{
    if (!r->patch || r->pw == 0 || r->ph == 0 ||
        r->pw > NVKVM_BROKER_CURSOR_MAX_DIM || r->ph > NVKVM_BROKER_CURSOR_MAX_DIM) {
        return;
    }
    memcpy(ctx->att, r->px, (size_t)r->pw * r->ph * sizeof ctx->att[0]);
    ctx->att_pw = r->pw;
    ctx->att_ph = r->ph;
    ctx->att_gen = cursor_gen;
    ctx->att_valid = true;
}

void nb_xor_detached(struct nb_xor_ctx *ctx)
{
    ctx->att_valid = false;
}

void nb_xor_choice_line(const struct nb_xor_res *r, const struct nb_frame *f,
                        char *buf, size_t n)
{
    if (r->act.choice == NB_XC_COMPOSED) {
        if (f && f->kind == NB_FRAME_DMABUF) {
            snprintf(buf, n, "cursor: XOR composed by the broker (frame read from "
                     "a LINEAR dma-buf, exporter %s: only the cursor's rows are "
                     "read)", f->exporter);
        } else {
            snprintf(buf, n, "cursor: XOR composed by the broker (frame read from "
                     "sealed shared memory: only the cursor's rows are read)");
        }
    } else if (r->act.choice == NB_XC_APPROX) {
        snprintf(buf, n, "cursor: XOR approximated, black fill and white outline "
                 "(frame not readable: %s)", nb_frame_why_text(r->why));
    } else {
        snprintf(buf, n, "cursor: choice is now %s",
                 nb_xor_choice_text(r->act.choice));
    }
}
