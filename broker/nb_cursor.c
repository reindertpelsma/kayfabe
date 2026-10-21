/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nb_cursor.c — the pure half of CMD_CURSOR: classify a record, bound its
 * extent, copy its pixels, and scale them.
 *
 * Nothing here touches an fd, a socket or a display connection, and nothing
 * logs.  That is the point of the split: every rule a hostile VMM's cursor
 * record has to get past lives in functions test/test_cursor.c can drive
 * through each boundary, and through a few hundred thousand random records,
 * without a broker, a compositor or a GPU.  The fd half -- proving the fd is
 * shmem, measuring it, reading it -- is nb_cmd_cursor() in nvkvm_broker.c, and
 * it calls these for every decision it makes.
 *
 * Every size here is bounded by a compile-time constant before it is used in
 * arithmetic, and every product is formed in 64 bits, so no client value can
 * wrap a bound into a small number.
 */
#include <string.h>

#include "nvkvm_broker.h"

/* DRM_FORMAT_ARGB8888, spelled locally: this file links into a unit test
 * that should not need libdrm either. */
#define NB_CURSOR_FOURCC_AR24 0x34325241u

int nb_cursor_check(const struct nvkvm_broker_cursor_cmd *c, bool has_fd,
                    bool stream_ok, const char **why)
{
    /*
     * FRAMING FIRST, then content -- the same order the clipboard case learned
     * the hard way: a malformed record is a violation however the image inside
     * it would have been judged, so the verdict must not depend on which
     * content check happens to run first.
     */
    /*
     * The one flag there is, NVKVM_BROKER_CURSOR_F_XOR, is a framing matter:
     * only on a SET, and only for a client that was told in HELLO that this
     * broker takes it (CAP_CURSOR_STREAM).  Anything else stays "flags are
     * zero", exactly as before the flag existed.
     */
    if (c->flags != 0 &&
        !(c->flags == NVKVM_BROKER_CURSOR_F_XOR && stream_ok &&
          c->op == NVKVM_BROKER_CURSOR_SET) &&
        !(c->flags == NVKVM_BROKER_CURSOR_F_ABS && stream_ok &&
          c->op == NVKVM_BROKER_CURSOR_MOVE)) {
        *why = (c->flags == NVKVM_BROKER_CURSOR_F_XOR && !stream_ok)
                   ? "CURSOR_F_XOR without CAP_CURSOR_STREAM"
                   : "CURSOR flags are not zero (or F_XOR on a HIDE/SHOW)";
        return NB_CURSOR_VIOLATION;
    }
    if (c->reserved1 != 0) {
        *why = "CURSOR reserved1 is not zero";
        return NB_CURSOR_VIOLATION;
    }
    switch (c->op) {
    case NVKVM_BROKER_CURSOR_HIDE:
    case NVKVM_BROKER_CURSOR_SHOW:
        if (has_fd) {
            *why = "CURSOR HIDE/SHOW carried an fd";
            return NB_CURSOR_VIOLATION;
        }
        if (c->width || c->height || c->stride || c->offset || c->fourcc ||
            c->hot_x || c->hot_y) {
            *why = "CURSOR HIDE/SHOW carried image fields";
            return NB_CURSOR_VIOLATION;
        }
        *why = "";
        return NB_CURSOR_OK;
    case NVKVM_BROKER_CURSOR_MOVE:
        if (!stream_ok) {
            *why = "CURSOR MOVE without CAP_CURSOR_STREAM";
            return NB_CURSOR_VIOLATION;
        }
        if (has_fd) {
            *why = "CURSOR MOVE carried an fd";
            return NB_CURSOR_VIOLATION;
        }
        if (c->stride || c->offset || c->fourcc) {
            *why = "CURSOR MOVE carried image fields";
            return NB_CURSOR_VIOLATION;
        }
        if (c->flags != NVKVM_BROKER_CURSOR_F_ABS && (c->width || c->height)) {
            *why = "CURSOR MOVE carried an absolute position without F_ABS";
            return NB_CURSOR_VIOLATION;
        }
        *why = "";
        return NB_CURSOR_OK;
    case NVKVM_BROKER_CURSOR_SET:
        if (!has_fd) {
            *why = "CURSOR SET without an SCM_RIGHTS fd";
            return NB_CURSOR_VIOLATION;
        }
        break;
    default:
        *why = "CURSOR op is not SET, HIDE or SHOW";
        return NB_CURSOR_VIOLATION;
    }

    /*
     * CONTENT.  Everything from here on describes the guest's image, which the
     * VMM forwards rather than invents, so a bad value is a refused cursor and
     * not a disconnect -- the same rule as a rejected ATTACH.
     */
    if (c->fourcc != NB_CURSOR_FOURCC_AR24) {
        *why = "CURSOR SET fourcc is not ARGB8888 (the only cursor format)";
        return NB_CURSOR_REJECT;
    }
    if (c->width == 0 || c->height == 0 ||
        c->width > NVKVM_BROKER_CURSOR_MAX_DIM ||
        c->height > NVKVM_BROKER_CURSOR_MAX_DIM) {
        *why = "CURSOR SET size is outside 1..256";
        return NB_CURSOR_REJECT;
    }
    /* Checked against the SIZE, which is already known to be <= 256, so
     * neither comparison can be satisfied by a wrapped value. */
    if (c->hot_x >= c->width || c->hot_y >= c->height) {
        *why = "CURSOR SET hot spot is outside the image";
        return NB_CURSOR_REJECT;
    }
    if ((uint64_t)c->stride < (uint64_t)c->width * 4u ||
        c->stride > NVKVM_BROKER_CURSOR_MAX_STRIDE) {
        *why = "CURSOR SET stride is not between width*4 and 1024";
        return NB_CURSOR_REJECT;
    }
    *why = "";
    return NB_CURSOR_OK;
}

uint32_t nb_cursor_span(uint32_t w, uint32_t h, uint32_t stride)
{
    /*
     * The LAST row is only w*4 bytes long -- a cursor packed tightly at the
     * end of its memfd is legitimate, and demanding a full stride for it would
     * refuse an honest client for padding it never needed.  For a record
     * nb_cursor_check() accepted this is at most 1024*255 + 1024.
     */
    uint64_t span;

    if (w == 0 || h == 0) {
        return 0;
    }
    span = (uint64_t)stride * (uint64_t)(h - 1u) + (uint64_t)w * 4u;
    return span > UINT32_MAX ? UINT32_MAX : (uint32_t)span;
}

bool nb_cursor_fits(uint32_t offset, uint32_t span, uint64_t size)
{
    /* Two uint32 operands summed in 64 bits cannot wrap. */
    return (uint64_t)offset + (uint64_t)span <= size;
}

void nb_cursor_load(struct nb_cursor *dst, const uint8_t *src, uint32_t w,
                    uint32_t h, uint32_t stride, uint32_t hot_x,
                    uint32_t hot_y)
{
    uint32_t x, y;

    for (y = 0; y < h; y++) {
        const uint8_t *row = src + (size_t)y * stride;
        uint32_t *out = dst->px + (size_t)y * w;

        for (x = 0; x < w; x++) {
            /*
             * Bytes, not a uint32 load: B,G,R,A in memory is the DEFINITION
             * of DRM's little-endian ARGB8888, and assembling it explicitly
             * keeps that true on any host and needs no alignment from the
             * client's offset or stride.
             */
            uint32_t b = row[x * 4u + 0], g = row[x * 4u + 1];
            uint32_t r = row[x * 4u + 2], a = row[x * 4u + 3];

            /*
             * PREMULTIPLIED means no colour channel exceeds alpha.  Clamp
             * rather than refuse: an image that breaks it is the sender's
             * colour bug, and handing it on would make what the compositor
             * does with an out-of-range premultiplied value -- additive
             * blending, in practice -- a property of our input validation.
             */
            if (r > a) { r = a; }
            if (g > a) { g = a; }
            if (b > a) { b = a; }
            out[x] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }
    dst->w = w;
    dst->h = h;
    dst->hot_x = hot_x;
    dst->hot_y = hot_y;
    dst->defined = true;
    dst->visible = true;
    dst->xor = false;       /* nb_cursor_set_xor() says otherwise, after */
    dst->bx = dst->by = dst->bw = dst->bh = 0;
    dst->gen++;
}

int nb_cursor_move_clamp(int32_t x, int32_t y, int32_t *ox, int32_t *oy)
{
    const int64_t lo = -(int64_t)NVKVM_BROKER_CURSOR_MAX_DIM;
    const int64_t hi = (int64_t)NVKVM_BROKER_MAX_DIM;
    int64_t X = x, Y = y;

    if (X > NVKVM_BROKER_CURSOR_MOVE_LIMIT || X < -NVKVM_BROKER_CURSOR_MOVE_LIMIT ||
        Y > NVKVM_BROKER_CURSOR_MOVE_LIMIT || Y < -NVKVM_BROKER_CURSOR_MOVE_LIMIT) {
        return -1;
    }
    *ox = (int32_t)(X < lo ? lo : X > hi ? hi : X);
    *oy = (int32_t)(Y < lo ? lo : Y > hi ? hi : Y);
    return 0;
}

void nb_hot_reset(struct nb_hot *h)
{
    memset(h, 0, sizeof *h);
}

bool nb_hot_move(struct nb_hot *h, struct nb_cursor *c, bool abs_valid,
                 int32_t abs_x, int32_t abs_y)
{
    int64_t dx, dy;

    if (!c->defined || c->w == 0 || c->h == 0 || !abs_valid) {
        /* relative input (or no image): the pair says nothing; a candidate from
         * before it would be bridged over input the guest moved on through */
        h->cand_valid = false;
        return false;
    }
    dx = (int64_t)abs_x - (int64_t)c->x;
    dy = (int64_t)abs_y - (int64_t)c->y;
    if (dx < 0 || dy < 0 || dx >= (int64_t)c->w || dy >= (int64_t)c->h) {
        /* the pointer and the cursor belong to different moments, or the
         * pointer is not over this image: implausible, and it breaks the run */
        h->cand_valid = false;
        return false;
    }
    if (h->cand_valid && h->cand_x == (int32_t)dx && h->cand_y == (int32_t)dy &&
        h->cand_ax == abs_x && h->cand_ay == abs_y) {
        /* the same difference in two consecutive MOVEs: the pointer is still */
        int64_t ddx = dx - (int64_t)c->hot_x, ddy = dy - (int64_t)c->hot_y;
        bool far = ddx > 1 || ddx < -1 || ddy > 1 || ddy < -1;

        if (!h->latched || far) {
            bool changed = (uint32_t)dx != c->hot_x || (uint32_t)dy != c->hot_y;

            h->latched = true;
            c->hot_x = (uint32_t)dx;
            c->hot_y = (uint32_t)dy;
            return changed;
        }
        return false;
    }
    h->cand_valid = true;
    h->cand_x = (int32_t)dx;
    h->cand_y = (int32_t)dy;
    h->cand_ax = abs_x;
    h->cand_ay = abs_y;
    return false;
}

enum nb_cursor_draw nb_cursor_policy(const struct nb_cursor *c, bool grabbed,
                                     bool stream)
{
    if (!c || !c->defined || !c->visible) {
        return NB_CURSOR_NONE;
    }
    if (!grabbed) {
        return NB_CURSOR_POINTER;
    }
    return (stream && c->placed) ? NB_CURSOR_OVERLAY : NB_CURSOR_NONE;
}

bool nb_cursor_xor_approx(struct nb_cursor *c)
{
    /* Black fill, white outline: both opaque, so distinct from each other and
     * from the transparent rest -- which lets this run in place.  The outline
     * pass reads only BLACK neighbours, and writes only WHITE onto pixels that
     * were transparent, so the order of the writes cannot matter. */
    const uint32_t BLACK = 0xff000000u, WHITE = 0xffffffffu;
    size_t n, x, y, w, h;

    bool any = false;

    if (!c->defined || c->w == 0 || c->h == 0 ||
        c->w > NVKVM_BROKER_CURSOR_MAX_DIM || c->h > NVKVM_BROKER_CURSOR_MAX_DIM) {
        return false;
    }
    w = c->w;
    h = c->h;
    n = w * h;
    for (x = 0; x < n; x++) {
        /* the alpha byte is the mask: set where the XOR cursor inverts */
        c->px[x] = (c->px[x] >> 24) ? BLACK : 0;
        any |= c->px[x] != 0;
    }
    if (!any) {
        return false;
    }
    for (y = 0; y < h; y++) {
        for (x = 0; x < w; x++) {
            size_t y0 = y ? y - 1 : 0, y1 = y + 1 < h ? y + 1 : h - 1;
            size_t x0 = x ? x - 1 : 0, x1 = x + 1 < w ? x + 1 : w - 1;
            size_t j, i;

            if (c->px[y * w + x] != 0) {
                continue;
            }
            for (j = y0; j <= y1; j++) {
                for (i = x0; i <= x1; i++) {
                    if (c->px[j * w + i] == BLACK) {
                        c->px[y * w + x] = WHITE;
                        goto next;
                    }
                }
            }
next:       ;
        }
    }
    return true;
}

bool nb_cursor_set_xor(struct nb_cursor *c)
{
    uint32_t x, y, x0, y0, x1, y1;
    size_t n;
    bool any = false;

    c->xor = false;
    c->bx = c->by = c->bw = c->bh = 0;
    if (!c->defined || c->w == 0 || c->h == 0 ||
        c->w > NVKVM_BROKER_CURSOR_MAX_DIM || c->h > NVKVM_BROKER_CURSOR_MAX_DIM) {
        return false;
    }
    n = (size_t)c->w * c->h;
    memcpy(c->mask, c->px, n * sizeof c->px[0]);
    x0 = c->w;
    y0 = c->h;
    x1 = y1 = 0;
    for (y = 0; y < c->h; y++) {
        for (x = 0; x < c->w; x++) {
            if ((c->mask[(size_t)y * c->w + x] >> 24) != 0) {
                any = true;
                if (x < x0) { x0 = x; }
                if (y < y0) { y0 = y; }
                if (x > x1) { x1 = x; }
                if (y > y1) { y1 = y; }
            }
        }
    }
    c->xor = true;
    if (any) {
        c->bx = x0;
        c->by = y0;
        c->bw = x1 - x0 + 1u;
        c->bh = y1 - y0 + 1u;
    }
    /* the approximation replaces px (backends that cannot invert draw it) */
    return nb_cursor_xor_approx(c) && any;
}

void nb_cursor_copy(struct nb_cursor *dst, const struct nb_cursor *src)
{
    size_t n = 0;

    if (dst == src) {
        return;
    }
    dst->gen = src->gen;
    dst->defined = src->defined;
    dst->visible = src->visible;
    dst->w = src->w;
    dst->h = src->h;
    dst->hot_x = src->hot_x;
    dst->hot_y = src->hot_y;
    dst->x = src->x;
    dst->y = src->y;
    dst->placed = src->placed;
    dst->abs_x = src->abs_x;
    dst->abs_y = src->abs_y;
    dst->abs_valid = src->abs_valid;
    dst->xor = src->xor;
    dst->bx = src->bx;
    dst->by = src->by;
    dst->bw = src->bw;
    dst->bh = src->bh;
    /*
     * Only the pixels the image has.  Bounded by the array, not by w*h alone:
     * a struct nobody loaded through nb_cursor_load() must not make this a
     * copy past the end of either buffer.
     */
    if (src->defined) {
        n = (size_t)src->w * src->h;
        if (n > sizeof src->px / sizeof src->px[0]) {
            n = sizeof src->px / sizeof src->px[0];
        }
        memcpy(dst->px, src->px, n * sizeof src->px[0]);
        if (src->xor) {
            memcpy(dst->mask, src->mask, n * sizeof src->mask[0]);
        }
    }
}

/*
 * The scaled cursor's geometry from 64-bit factors.  Every caller bounds them
 * (see the two wrappers below) so that w * num stays under 2^63: w <= 256 is
 * 2^8, and the largest numerator either wrapper can form is under 2^44.
 */
static void nb_cursor_geom64(const struct nb_cursor *c, uint64_t num_x,
                             uint64_t den_x, uint64_t num_y, uint64_t den_y,
                             uint32_t max, struct nb_cursor_geom *out)
{
    uint64_t w = c->w ? c->w : 1u, h = c->h ? c->h : 1u;
    uint64_t dw, dh, hx, hy;

    if (num_x == 0 || den_x == 0) {
        num_x = den_x = 1;
    }
    if (num_y == 0 || den_y == 0) {
        num_y = den_y = 1;
    }
    if (max == 0) {
        max = 1;
    }
    dw = (w * num_x + den_x / 2u) / den_x;
    dh = (h * num_y + den_y / 2u) / den_y;
    if (dw < 1) { dw = 1; }
    if (dh < 1) { dh = 1; }
    /*
     * Past the bound, shrink BOTH edges by the same factor, so a clamped
     * cursor is a smaller cursor rather than a distorted one.  The product
     * below is formed from edges first brought under 2^24 (a ratio of two
     * numbers that large keeps far more precision than a <= max result can
     * show), so it stays under 2^56 whatever max is.
     */
    if (dw > max || dh > max) {
        while (dw > (1ull << 24) || dh > (1ull << 24)) {
            dw >>= 1;
            dh >>= 1;
        }
        if (dw < 1) { dw = 1; }
        if (dh < 1) { dh = 1; }
        if (dw >= dh) {
            dh = dh * max / dw;
            dw = max;
        } else {
            dw = dw * max / dh;
            dh = max;
        }
        if (dw < 1) { dw = 1; }
        if (dh < 1) { dh = 1; }
    }
    out->w = (uint32_t)dw;
    out->h = (uint32_t)dh;
    /*
     * THE HOT SPOT IS THE OUTPUT PIXEL THAT SHOWS THE GUEST'S HOT PIXEL.
     *
     * nb_cursor_scale() fills output x from source floor(x * w / dw).  The
     * first x whose source reaches hot is ceil(hot * dw / w): x * w / dw >=
     * hot exactly when x >= hot * dw / w.  When the cursor is not shrunk
     * (dw >= w) that x samples hot itself, every time; when it is shrunk and
     * hot is a pixel the scaler skips, it samples the nearest pixel after it.
     *
     * This used to be floor(hot * dw / w) -- "the same scale as the image" --
     * which at 1.5x puts a 32-pixel cursor's hot spot 3 on output pixel 4,
     * and output pixel 4 shows source pixel 2: a pointer that clicks one
     * pixel left of where its arrow's tip is drawn, or on a transparent
     * pixel beside it.  Computed in 64 bits: hot < 256, dw <= max.
     *
     * The clamp is reached only by a shrunk cursor whose hot pixel lies past
     * the last sampled one -- and makes "inside the image" hold even for an
     * nb_cursor nobody loaded through nb_cursor_load().
     */
    hx = ((uint64_t)c->hot_x * dw + w - 1u) / w;
    hy = ((uint64_t)c->hot_y * dh + h - 1u) / h;
    out->hot_x = hx >= dw ? (uint32_t)dw - 1u : (uint32_t)hx;
    out->hot_y = hy >= dh ? (uint32_t)dh - 1u : (uint32_t)hy;
}

void nb_cursor_scaled_geom(const struct nb_cursor *c, uint32_t num_x,
                           uint32_t den_x, uint32_t num_y, uint32_t den_y,
                           uint32_t max, struct nb_cursor_geom *out)
{
    /* w <= 256 and num <= 2^32-1: the product is < 2^41. */
    nb_cursor_geom64(c, num_x, den_x, num_y, den_y, max, out);
}

/* The largest output scale nb_cursor_device_geom() honours: 16x, which is
 * past any real display and keeps every product below 2^44. */
#define NB_CURSOR_MAX_SCALE_120 (16u * 120u)

void nb_cursor_device_geom(const struct nb_cursor *c, uint32_t num_x,
                           uint32_t den_x, uint32_t num_y, uint32_t den_y,
                           uint32_t scale_120, uint32_t max,
                           struct nb_cursor_geom *out)
{
    uint64_t s = scale_120;

    /*
     * Below 1 is not a reason to render FEWER pixels than the logical size --
     * the compositor can shrink -- and 0 is "unknown".  Both are 1:1.
     */
    if (s < 120u) {
        s = 120u;
    }
    if (s > NB_CURSOR_MAX_SCALE_120) {
        s = NB_CURSOR_MAX_SCALE_120;
    }
    if (num_x == 0 || den_x == 0) {
        num_x = den_x = 1;
    }
    if (num_y == 0 || den_y == 0) {
        num_y = den_y = 1;
    }
    /* num * s < 2^32 * 2^11 = 2^43; den * 120 < 2^39.  Times w <= 2^8: no
     * product reaches 2^52. */
    nb_cursor_geom64(c, (uint64_t)num_x * s, (uint64_t)den_x * 120u,
                     (uint64_t)num_y * s, (uint64_t)den_y * 120u, max, out);
}

void nb_cursor_surface_geom(const struct nb_cursor *c, uint32_t num_x,
                            uint32_t den_x, uint32_t num_y, uint32_t den_y,
                            uint32_t scale_120, uint32_t max_buf,
                            struct nb_cursor_surface *out)
{
    uint32_t k, max_logical;

    /* ceil(scale/1.0), the smallest integer buffer_scale that loses no detail
     * the output can show; below 1 (or unknown) is 1; 4 is past any display. */
    if (scale_120 > 4u * 120u) {
        scale_120 = 4u * 120u;
    }
    k = (scale_120 + 119u) / 120u;
    if (k < 1u) {
        k = 1u;
    }
    if (max_buf == 0) {
        max_buf = 1;
    }
    max_logical = max_buf / k;
    if (max_logical < 1u) {
        max_logical = 1u;
    }
    /* The logical size is the frame fit alone: image_dim * num / den. */
    nb_cursor_scaled_geom(c, num_x, den_x, num_y, den_y, max_logical, &out->g);
    out->scale = k;
    out->bw = out->g.w * k;
    out->bh = out->g.h * k;
}

/* Whether mask pixel (ix, iy) lands on the frame at (x, y) + (ix, iy), and where. */
static bool xor_on_frame(int32_t x, int32_t y, uint32_t ix, uint32_t iy,
                         uint32_t fw, uint32_t fh, int64_t *fx, int64_t *fy)
{
    *fx = (int64_t)x + (int64_t)ix;
    *fy = (int64_t)y + (int64_t)iy;
    return *fx >= 0 && *fy >= 0 && *fx < (int64_t)fw && *fy < (int64_t)fh;
}

size_t nb_cursor_xor_apply(const struct nb_cursor *c, uint32_t *frame,
                           uint32_t fw, uint32_t fh, size_t stride_px,
                           int32_t x, int32_t y)
{
    size_t n = 0;
    uint32_t ix, iy;

    if (!c || !c->defined || c->w == 0 || c->h == 0 ||
        c->w > NVKVM_BROKER_CURSOR_MAX_DIM || c->h > NVKVM_BROKER_CURSOR_MAX_DIM ||
        !frame || stride_px < fw) {
        return 0;
    }
    for (iy = 0; iy < c->h; iy++) {
        for (ix = 0; ix < c->w; ix++) {
            uint32_t m = c->px[(size_t)iy * c->w + ix];
            int64_t fx, fy;

            if ((m >> 24) == 0 || !xor_on_frame(x, y, ix, iy, fw, fh, &fx, &fy)) {
                continue;
            }
            frame[(size_t)fy * stride_px + (size_t)fx] ^= m & 0x00ffffffu;
            n++;
        }
    }
    return n;
}

size_t nb_xor_patch_rect(const uint32_t *mask, uint32_t mstride, uint32_t mw,
                         uint32_t mh, const uint32_t *frame, uint32_t fw,
                         uint32_t fh, size_t stride_px, int32_t x, int32_t y,
                         uint32_t *out)
{
    size_t n = 0;
    uint32_t ix, iy;

    if (!mask || mw == 0 || mh == 0 || mw > NVKVM_BROKER_CURSOR_MAX_DIM ||
        mh > NVKVM_BROKER_CURSOR_MAX_DIM || mstride < mw || !frame || !out ||
        stride_px < fw) {
        return 0;
    }
    for (iy = 0; iy < mh; iy++) {
        for (ix = 0; ix < mw; ix++) {
            uint32_t m = mask[(size_t)iy * mstride + ix];
            uint32_t *o = out + (size_t)iy * mw + ix;
            int64_t fx, fy;

            if ((m >> 24) == 0 || !xor_on_frame(x, y, ix, iy, fw, fh, &fx, &fy)) {
                *o = 0;
                continue;
            }
            *o = 0xff000000u |
                 ((frame[(size_t)fy * stride_px + (size_t)fx] ^ m) & 0x00ffffffu);
            n++;
        }
    }
    return n;
}

size_t nb_cursor_xor_patch(const struct nb_cursor *c, const uint32_t *frame,
                           uint32_t fw, uint32_t fh, size_t stride_px,
                           int32_t x, int32_t y, uint32_t *out)
{
    if (!c || !c->defined) {
        return 0;
    }
    return nb_xor_patch_rect(c->px, c->w, c->w, c->h, frame, fw, fh, stride_px,
                             x, y, out);
}

void nb_px_scale(const uint32_t *src, uint32_t sw, uint32_t sh, uint32_t *dst,
                 uint32_t dw, uint32_t dh)
{
    uint32_t x, y;

    if (!src || !dst || sw == 0 || sh == 0 || dw == 0 || dh == 0) {
        return;
    }
    for (y = 0; y < dh; y++) {
        /* y < dh  ⇒  y*h/dh < h: every source index is in the image. */
        uint32_t sy = (uint32_t)((uint64_t)y * sh / dh);
        const uint32_t *srow = src + (size_t)sy * sw;
        uint32_t *drow = dst + (size_t)y * dw;

        for (x = 0; x < dw; x++) {
            drow[x] = srow[(uint64_t)x * sw / dw];
        }
    }
}

void nb_px_scale_box(const uint32_t *src, uint32_t sw, uint32_t sh,
                     uint32_t *dst, uint32_t dw, uint32_t dh)
{
    uint32_t x, y;

    if (!src || !dst || sw == 0 || sh == 0 || dw == 0 || dh == 0) {
        return;
    }
    for (y = 0; y < dh; y++) {
        uint32_t sy0 = (uint32_t)((uint64_t)y * sh / dh);
        uint32_t sy1 = dh >= sh ? sy0 + 1u
                                : (uint32_t)(((uint64_t)(y + 1u) * sh + dh - 1u) / dh);

        if (sy1 > sh) { sy1 = sh; }
        if (sy1 <= sy0) { sy1 = sy0 + 1u; }
        for (x = 0; x < dw; x++) {
            uint32_t sx0 = (uint32_t)((uint64_t)x * sw / dw);
            uint32_t sx1 = dw >= sw ? sx0 + 1u
                                    : (uint32_t)(((uint64_t)(x + 1u) * sw + dw - 1u) / dw);
            uint64_t a = 0, r = 0, g = 0, b = 0, n;
            uint32_t i, j;

            if (sx1 > sw) { sx1 = sw; }
            if (sx1 <= sx0) { sx1 = sx0 + 1u; }
            for (j = sy0; j < sy1; j++) {
                for (i = sx0; i < sx1; i++) {
                    uint32_t v = src[(size_t)j * sw + i];

                    a += v >> 24;
                    r += (v >> 16) & 0xffu;
                    g += (v >> 8) & 0xffu;
                    b += v & 0xffu;
                }
            }
            n = (uint64_t)(sx1 - sx0) * (sy1 - sy0);
            dst[(size_t)y * dw + x] =
                (uint32_t)(((a + n / 2) / n) << 24 | ((r + n / 2) / n) << 16 |
                           ((g + n / 2) / n) << 8 | ((b + n / 2) / n));
        }
    }
}

void nb_cursor_scale(const struct nb_cursor *c, uint32_t *dst, uint32_t dw,
                     uint32_t dh)
{
    if (!c->defined || c->w == 0 || c->h == 0) {
        return;
    }
    nb_px_scale(c->px, c->w, c->h, dst, dw, dh);
}

bool nb_cursor_wanted(const struct nb_cursor *c, bool grabbed)
{
    return c && c->defined && c->visible && !grabbed;
}

uint32_t nb_cursor_hash(const struct nb_cursor *c)
{
    uint32_t h = 2166136261u;
    size_t i, n;

    if (!c->defined) {
        return 0;
    }
    n = (size_t)c->w * c->h;
    for (i = 0; i < n; i++) {
        uint32_t v = c->px[i];
        int k;

        for (k = 0; k < 4; k++) {
            h ^= (v >> (8 * k)) & 0xffu;
            h *= 16777619u;
        }
    }
    return h;
}

bool nb_cursor_crop(const struct nb_cursor *src, struct nb_cursor *dst)
{
    uint32_t x0, y0, x1, y1, x, y, w, h;
    bool any = false;

    if (!src || !dst || !src->defined || src->w == 0 || src->h == 0 ||
        src->w > NVKVM_BROKER_CURSOR_MAX_DIM || src->h > NVKVM_BROKER_CURSOR_MAX_DIM ||
        src->hot_x >= src->w || src->hot_y >= src->h) {
        return false;
    }
    /* the box starts as the hot spot, so the crop always contains it */
    x0 = x1 = src->hot_x;
    y0 = y1 = src->hot_y;
    for (y = 0; y < src->h; y++) {
        for (x = 0; x < src->w; x++) {
            if (src->px[(size_t)y * src->w + x] != 0) {
                any = true;
                if (x < x0) {
                    x0 = x;
                }
                if (x > x1) {
                    x1 = x;
                }
                if (y < y0) {
                    y0 = y;
                }
                if (y > y1) {
                    y1 = y;
                }
            }
        }
    }
    if (!any) {
        return false;       /* nothing drawn: nothing to crop to */
    }
    /* one clear pixel of margin, inside the image */
    x0 = x0 > 0 ? x0 - 1 : 0;
    y0 = y0 > 0 ? y0 - 1 : 0;
    x1 = x1 + 1 < src->w ? x1 + 1 : src->w - 1;
    y1 = y1 + 1 < src->h ? y1 + 1 : src->h - 1;
    w = x1 - x0 + 1;
    h = y1 - y0 + 1;
    if (w == src->w && h == src->h) {
        return false;       /* already tight */
    }
    dst->gen = src->gen;
    dst->defined = true;
    dst->visible = src->visible;
    dst->w = w;
    dst->h = h;
    dst->hot_x = src->hot_x - x0;
    dst->hot_y = src->hot_y - y0;
    dst->x = src->x + (int32_t)x0;
    dst->y = src->y + (int32_t)y0;
    dst->placed = src->placed;
    dst->abs_x = src->abs_x;
    dst->abs_y = src->abs_y;
    dst->abs_valid = src->abs_valid;
    dst->xor = false;       /* the host pointer shows the approximation: no mask */
    dst->bx = dst->by = dst->bw = dst->bh = 0;
    for (y = 0; y < h; y++) {
        memcpy(&dst->px[(size_t)y * w], &src->px[(size_t)(y0 + y) * src->w + x0],
               (size_t)w * sizeof dst->px[0]);
    }
    return true;
}
