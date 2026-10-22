/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nvkvm_broker.h — internals of the privileged display broker.
 *
 * Two abstractions, and only two:
 *
 *   struct nb_session   a *session backend*: it owns one window on one display
 *                       stack (Wayland or X11), knows how to turn a dma-buf
 *                       into that stack's native buffer object and put it on
 *                       screen, and knows how to read input.  Backends are
 *                       deliberately dumb: they translate native events into
 *                       sink calls and do what set_grab() says.
 *
 *   struct nb_sink      the policy core: hotkeys, grab state, focus gating,
 *                       stuck-key release, command validation and packet
 *                       emission.  Every security-relevant rule lives here
 *                       ONCE so it can be audited once, rather than twice in
 *                       two backends that will drift.
 *
 * Nothing in the broker allocates on behalf of a client.  Every buffer is
 * fixed size and statically sized at compile time, and no size, count or index
 * is ever taken from the wire.
 */
#ifndef NVKVM_BROKER_H
#define NVKVM_BROKER_H

#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>
#include <poll.h>
#include <sys/types.h>

#include "nvkvm_broker_proto.h"

#define NB_MAX_SESSION_FDS   8
#define NB_MAX_ALLOW_IDS     16
/*
 * How many imported buffers a backend keeps alive at once.  A guest compositor
 * cycles a handful of scanout bos — measured at 3, which is why
 * src/qemu/nvkvm_present_egl.c carries an 8-entry import cache — so 8 covers
 * the real workload with headroom.  It is also the FD INTAKE BOUND: the client
 * cannot make the privileged process hold more than this many buffers, because
 * importing the ninth evicts and closes the least recently used.
 */
#define NB_MAX_BUFS          8
/* Outbound packet ring.  ~12 KB.  Overflow means the client is not draining
 * its socket, which is either a dead client or a hostile one trying to make
 * the privileged side buffer without bound: we disconnect instead of growing. */
#define NB_TXRING            512

/* ── logging ─────────────────────────────────────────────────────────────── */
void nb_log(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
void nb_err(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
extern int nb_verbose;

/*
 * --trace-frames.  Logs one line per COMMIT: the client's frame counter, the
 * dma-buf inode, the cache slot, and whether the compositor had released that
 * buffer before it was handed back.  This exists because an intermittent
 * "stale frame" is the one bug class you cannot fix by reading the code: the
 * only way to tell a reorder from a buffer overwritten in flight is to watch
 * buffer IDENTITY per frame, and identity is only knowable here.
 */
extern int nb_trace_frames;

/* ── configuration (command line only; never anything a client sent) ─────── */
/*
 * CLIPBOARD MODES, NAMED BY DIRECTION.
 *
 * "copy-only" and similar are ambiguous about which way copying goes, and on a
 * security setting a misread default is the whole problem.  The ladder is
 * ordered by how much crosses, and `consent` is the recommended rung.
 */
#define NB_CLIP_OFF      0  /* nothing crosses.  The DEFAULT.               */
#define NB_CLIP_G2H      1  /* guest may write the host clipboard; the host
                             * clipboard is never readable by the guest      */
#define NB_CLIP_CONSENT  2  /* G2H, plus host->guest ONLY on an explicit
                             * paste trigger.  RECOMMENDED.                  */

/* Keys that mean "paste" and so trigger a host->guest send. */
#define NB_CLIP_MAX_TRIGGERS 8
struct nb_clip_trigger {
    unsigned code;              /* evdev key code                           */
    bool     need_ctrl;
    bool     need_shift;
};

struct nb_config {
    const char *socket_path;
    const char *backend;        /* "auto" | "x11" | "wayland" | "test"        */
    const char *title;
    unsigned    win_w, win_h;   /* initial window size                        */
    const char *drop_user;      /* user to become after setup, or NULL        */
    /*
     * SOCKET ACCESS.  Until these existed the socket was created 0600 and
     * owned by the broker's user, so no other user could connect() at all --
     * which made --allow-group a flag that could never fire: SO_PEERCRED never
     * got the chance to say yes, because the filesystem had already said no.
     * The two controls are complementary and both apply: the mode decides who
     * may reach the socket, SO_PEERCRED decides who may be served.
     */
    unsigned    socket_mode;    /* octal; default 0600                        */
    const char *socket_group;   /* chgrp the socket to this group, or NULL    */
    bool        no_peercred;    /* opt OUT of the credential allow-list       */
    int         socket_fd;      /* pre-bound listening fd, or -1              */

    /*
     * Clipboard.  DEFAULT OFF, deliberately.  Clipboard already needs opting
     * in twice (QEMU's -chardev qemu-vdagent, and spice-vdagent in the guest),
     * so a third gate costs little -- and it buys the property that matters:
     * with `consent` as a default, what a system actually does would depend on
     * whether vdagent happens to be installed, so you could not tell your
     * clipboard posture by reading the broker's configuration.  With `off`,
     * the flag IS the answer.
     *
     * The cost of a restrictive default is that friction pushes people to the
     * least safe setting, so every dead end here names `consent` explicitly
     * rather than leaving the reader to find the flag list.
     */
    int         clip_mode;
    struct nb_clip_trigger clip_trigger[NB_CLIP_MAX_TRIGGERS];
    int         n_clip_trigger;
    uid_t  allow_uid[NB_MAX_ALLOW_IDS];
    int    n_allow_uid;
    gid_t  allow_gid[NB_MAX_ALLOW_IDS];
    int    n_allow_gid;
    /* Start the window fullscreen, rather than waiting for CTRL+ALT+F.
     * Direct scanout is only possible fullscreen, so a measurement of it
     * should not depend on somebody pressing a key. */
    bool fullscreen;
    /* Keep the window and wait for another client when one disconnects.  The
     * default is to exit: a broker window outliving its VM is a window showing
     * nothing, and the socket is the VMM's to reconnect on. */
    bool persist;
    /* ★ MEASUREMENT (hitch investigation): print one `stats t=<CLOCK_MONOTONIC ms> ...` line per second of per-second
     * deltas of the tick counters (--stats-line).  Off by default, never required, no behaviour change. */
    bool stats_line;
    /* 0 = never scale the guest frame, 1 = always scale it to the window,
     * 2 = auto: 1:1 windowed, scaled to fit in fullscreen.  Default 2. */
    /*
     * How the guest's frame is mapped into the window.  "fit" is deliberately
     * NOT one of the names: in video players it conventionally means preserve
     * aspect and letterbox, while the user meant fill -- so the word would be
     * read backwards by half the people who saw it.
     */
    int  scale_mode;
};

/* ── a validated buffer description ──────────────────────────────────────── */

/*
 * What reaches a backend's attach().  Every field here has already been
 * checked by nb_cmd_attach() in nvkvm_broker.c against the REAL fd:
 *
 *   - `fd` is a dma-buf (fstatfs f_type == DMA_BUF_MAGIC), not merely an fd —
 *     or, when `is_shm` is set, a memfd (TMPFS_MAGIC) carrying F_SEAL_SHRINK,
 *     which is the only other fd type any backend may ever be handed;
 *   - width/height are non-zero and <= NVKVM_BROKER_MAX_DIM;
 *   - fourcc is one the backend's format_ok() said the GPU advertises,
 *     paired with this exact modifier;
 *   - stride >= width * bpp, and offset + stride*height <= the fd's real size
 *     measured with lseek(fd, 0, SEEK_END).
 *
 * A backend may therefore import without re-deriving any of that.  It must
 * still check its own import calls for failure, because the compositor or X
 * server can refuse for reasons the broker cannot see.
 */
struct nb_buf_desc {
    int      fd;                /* borrowed: attach() must not close it       */
    uint64_t id;                /* the dma-buf inode — stable, not reused
                                 * while the buffer is live, and derived by
                                 * the BROKER, never supplied by the client   */
    uint64_t size;              /* real extent, from lseek(SEEK_END)          */
    uint32_t width, height, stride, offset, fourcc;
    uint64_t modifier;
    uint32_t bpp;               /* bytes per pixel implied by fourcc          */
    bool     is_shm;            /* the fd is a memfd, not a dma-buf: the only
                                 * buffer type a Wayland compositor CANNOT
                                 * refuse, so it is the universal last tier
                                 * when even LINEAR dma-buf import fails      */
    uint32_t seq;               /* the client's own frame counter (cmd.seq).
                                 * ADVISORY — it comes from the untrusted VMM
                                 * and is used for nothing but log lines, so a
                                 * lie in it costs a confusing trace and
                                 * nothing else.                              */
};

/* ── the guest's cursor (CMD_CURSOR) ─────────────────────────────────────── */

/*
 * The core's copy of the guest's pointer image.  Filled ONLY by nb_cursor_load()
 * from rows the core read out of a validated memfd, so every invariant below
 * holds whatever the client sent:
 *
 *   - when `defined`: 1 <= w,h <= NVKVM_BROKER_CURSOR_MAX_DIM, hot_x < w,
 *     hot_y < h, and px[0 .. w*h) is the image, packed (row pitch == w);
 *   - every pixel is premultiplied ARGB with each colour channel <= alpha.
 *
 * Backends read it (never write it) and must copy what they render.  What a
 * backend is handed is the core's PUBLISHED snapshot (nb_sink.cursor_pub), which
 * changes only when the pacing interval allows -- never the copy a SET writes
 * into -- so a backend re-rendering on its own schedule (a frame commit, a
 * resize, a buffer release, the grab ending) re-renders what was last
 * published, not something newer the pacing has not let through yet.
 *
 * `gen` changes on every change of the image and never repeats within a
 * process -- 64 bits, incremented by one, so it would take longer than the
 * machine will run to wrap -- and "have I already rendered this?" is one
 * comparison, across reconnects too.
 */
struct nb_cursor {
    uint64_t gen;
    bool     defined;           /* an image was SET on this connection        */
    bool     visible;           /* SET/SHOW since the last HIDE               */
    uint32_t w, h, hot_x, hot_y;
    int32_t  x, y;              /* the image's top-left on the guest frame,   */
    bool     placed;            /* once a MOVE set it (CAP_CURSOR_STREAM)     */
    int32_t  abs_x, abs_y;      /* the last absolute pointer position (MOVE), */
    bool     abs_valid;         /* valid only if the VMM's last input was one */
    /*
     * XOR (NVKVM_BROKER_CURSOR_F_XOR, D2 2026-10-17).  When `xor` is set, `px`
     * is the APPROXIMATION (nb_cursor_xor_approx(): what every backend that
     * cannot invert draws, unchanged) and `mask` is the INVERSION MASK the VMM
     * sent -- the true cursor, composed by nb_xor_eval() where the frame can be
     * read.  mask[0 .. w*h) is packed like px; (bx, by, bw, bh) bounds its
     * non-clear pixels (bw = 0: it inverts nothing).  Only nb_cursor_set_xor()
     * sets these; nb_cursor_load() clears `xor`.
     */
    bool     xor;
    uint32_t bx, by, bw, bh;
    uint32_t px[NVKVM_BROKER_CURSOR_MAX_DIM * NVKVM_BROKER_CURSOR_MAX_DIM];
    uint32_t mask[NVKVM_BROKER_CURSOR_MAX_DIM * NVKVM_BROKER_CURSOR_MAX_DIM];
};

/*
 * The most a backend scales a cursor to, per edge.  A guest cursor of 256 over
 * a 2x-scaled frame is 512; anything larger is shrunk with its aspect kept,
 * which bounds what the broker renders per cursor change at 1 MiB whatever
 * window, mode or scale the user picked.
 */
#define NB_CURSOR_SCALED_MAX 512u

/* What a scaled cursor measures: image size and hot spot, all in OUTPUT pixels.
 * Guaranteed by nb_cursor_scaled_geom(): 1 <= w,h <= the max asked for,
 * hot_x < w, hot_y < h. */
struct nb_cursor_geom {
    uint32_t w, h, hot_x, hot_y;
};

/* nb_cursor_check() verdicts. */
#define NB_CURSOR_OK        0
#define NB_CURSOR_REJECT    1   /* well-formed, but this image is refused   */
#define NB_CURSOR_VIOLATION 2   /* framing error: hang up                   */

/*
 * Classify one CURSOR record from its fields alone, before any fd is touched.
 * `*why` names the rule that failed.  Pure, so test/test_cursor.c can drive it
 * through every boundary without a socket (nb_cursor.c).
 */
int  nb_cursor_check(const struct nvkvm_broker_cursor_cmd *c, bool has_fd,
                     bool stream_ok, const char **why);
/*
 * An XOR SET (NVKVM_BROKER_CURSOR_F_XOR) loaded into `c` -> the cursor the user
 * sees: every pixel whose alpha is non-zero (the guest's XOR cursor inverts the
 * screen there) becomes opaque BLACK, every other pixel touching one (the
 * 8-neighbourhood, clipped at the image's edge) opaque WHITE, the rest
 * transparent.  Size, hot spot, gen, position and visibility are untouched;
 * bounded (<= 256x256, two passes), in place, deterministic.  Always applied to
 * an XOR SET: a viewer cannot invert.  Returns false for an all-clear mask (the
 * guest's cursor inverts nothing: nothing to show; the caller hides it). */
bool nb_cursor_xor_approx(struct nb_cursor *c);
/*
 * The whole XOR SET step on a freshly loaded image: keep the mask (c->mask,
 * c->xor, its bounding box c->bx.. ), then make c->px the approximation.
 * Returns false for an all-clear mask (nothing to show; the caller hides it).
 * The mask's colour channels are exactly what nb_cursor_load() left (colour
 * clamped to alpha: a mask pixel is alpha 0xff or 0, so nothing is changed).
 */
bool nb_cursor_set_xor(struct nb_cursor *c);
/*
 * MOVE: refuse (-1) a position outside +-NVKVM_BROKER_CURSOR_MOVE_LIMIT, else
 * clamp it to -NVKVM_BROKER_CURSOR_MAX_DIM .. NVKVM_BROKER_MAX_DIM (outside of
 * which the image cannot show) and return 0. */
int  nb_cursor_move_clamp(int32_t x, int32_t y, int32_t *ox, int32_t *oy);
/*
 * The hot spot derived from the pointer (hover).  The Windows driver programs
 * hot 0,0 while the real spot is inside the image; each MOVE carries the image's
 * top-left and the last absolute pointer position, and hot = pointer - top-left.
 * Rules ported from the VMM's HotTracker (crates/kf-broker/src/cursor.rs), with
 * the owner's latch: the guest moves its cursor a little after it receives an
 * injected event, so a derived spot is LATCHED only when the SAME difference
 * appears in two consecutive MOVEs AT THE SAME POINTER (event-counted, no timer):
 * a cursor trailing a pointer that moves at constant speed shows one constant,
 * WRONG difference, so equal differences alone are not stationarity.  Until then, and
 * after every new image (nb_hot_reset), the SET hot spot stands.
 */
struct nb_hot {
    bool    latched;            /* a derived hot spot replaced the SET one    */
    bool    cand_valid;         /* the previous MOVE's plausible difference   */
    int32_t cand_x, cand_y;     /* ... that difference                         */
    int32_t cand_ax, cand_ay;   /* ... and the pointer it was taken at         */
};
void nb_hot_reset(struct nb_hot *h);
/* One MOVE: `c` holds the image (size, SET/latched hot spot) and its new top-left
 * c->x,c->y.  Returns true when c->hot_x/hot_y changed (the backend re-renders). */
bool nb_hot_move(struct nb_hot *h, struct nb_cursor *c, bool abs_valid,
                 int32_t abs_x, int32_t abs_y);

/* What a backend draws for the held cursor: nothing, the HOST POINTER image
 * (hover: the host pointer is the guest's), or an OVERLAY at c->x,c->y (grab,
 * where the broker has no pointer of its own; needs CAP_CURSOR_STREAM, a
 * MOVE received, and the image visible). */
enum nb_cursor_draw { NB_CURSOR_NONE, NB_CURSOR_POINTER, NB_CURSOR_OVERLAY };
enum nb_cursor_draw nb_cursor_policy(const struct nb_cursor *c, bool grabbed,
                                     bool stream);
/* Bytes a valid SET covers, from row 0's first byte to the last row's last
 * pixel byte: stride*(h-1) + w*4.  Only meaningful for a record
 * nb_cursor_check() accepted; at most MAX_STRIDE * MAX_DIM. */
uint32_t nb_cursor_span(uint32_t w, uint32_t h, uint32_t stride);
/* offset + span <= size, in 64 bits so no operand can wrap the sum. */
bool nb_cursor_fits(uint32_t offset, uint32_t span, uint64_t size);
/* Copy a validated image out of `src` (span bytes, row 0 first) into `dst`,
 * clamping colour to alpha, and bump dst->gen.  Sets defined and visible. */
void nb_cursor_load(struct nb_cursor *dst, const uint8_t *src, uint32_t w,
                    uint32_t h, uint32_t stride, uint32_t hot_x,
                    uint32_t hot_y);
/* Make `dst` a copy of `src`: every field, and the w*h packed pixels when an
 * image is defined (the rest of px is never read).  How the core publishes. */
void nb_cursor_copy(struct nb_cursor *dst, const struct nb_cursor *src);

/*
 * The image the HOST pointer is given: the tight box around the cursor's
 * non-clear pixels (plus one clear pixel of margin and always including the hot
 * spot), not the whole 256x256 hardware-cursor buffer a guest draws a 32x32
 * arrow into.  A pointer surface that large is outside what compositors and
 * remote views are built for (owner 2026-10-21: the hover pointer is invisible
 * with the large one).  Returns true and fills `dst` (w, h, hot_x, hot_y and
 * the packed pixels; gen/defined/visible copied; no XOR mask) when that box is
 * smaller than the image; false when there is nothing to crop (undefined, fully
 * clear, or already tight) and the caller keeps using `src`.
 */
bool nb_cursor_crop(const struct nb_cursor *src, struct nb_cursor *dst);
/*
 * The cursor's size and hot spot after scaling by num_x/den_x horizontally and
 * num_y/den_y vertically -- the same factors the frame is scaled by.
 *
 * SIZE: rounded to nearest, each edge at least 1 and at most `max` (past it,
 * both edges shrink by the same factor, aspect kept).
 *
 * HOT SPOT: the output pixel nb_cursor_scale() fills FROM the guest's hot
 * pixel -- ceil(hot * out / in), the first output index whose sample
 * floor(x * in / out) reaches it -- so the scaled image's hot spot is the
 * guest's hot pixel, not its neighbour.  Exact whenever the cursor is not
 * shrunk (every source pixel is then sampled at least once); when it is
 * shrunk and the hot pixel is one the scaler skips, it is the nearest sampled
 * pixel after it, clamped to the last.  test/test_cursor.c sweeps this.
 *
 * A zero numerator or denominator means 1:1.  `c` must be defined.
 */
void nb_cursor_scaled_geom(const struct nb_cursor *c, uint32_t num_x,
                           uint32_t den_x, uint32_t num_y, uint32_t den_y,
                           uint32_t max, struct nb_cursor_geom *out);
/*
 * The same cursor in DEVICE pixels on an output whose scale is scale_120/120
 * (wp_fractional_scale_v1's unit): nb_cursor_scaled_geom() with the output
 * scale folded into the factors, so a backend can render a buffer with the
 * detail the output can show and let a viewport bring it to the logical size.
 * A scale_120 below 120 (0 = unknown) is taken as 1, and one above 16x as 16x.
 */
void nb_cursor_device_geom(const struct nb_cursor *c, uint32_t num_x,
                           uint32_t den_x, uint32_t num_y, uint32_t den_y,
                           uint32_t scale_120, uint32_t max,
                           struct nb_cursor_geom *out);
/*
 * THE SIZE OF A CURSOR SURFACE (Wayland hover pointer and grab overlay both).
 *
 *   g       the cursor in SURFACE-LOCAL (logical) pixels: image_dim * fit, where
 *           fit = num/den is the frame's own scale (window logical px per guest
 *           px).  This is the unit wl_pointer.set_cursor's hot spot and a
 *           subsurface's position are in.  The OUTPUT scale is not a factor.
 *   scale   wl_surface.set_buffer_scale: ceil(scale_120 / 120), 1..4.
 *   bw, bh  the buffer: exactly g * scale, so the surface is buffer / scale = g
 *           with NO viewport (a compositor need not honour wp_viewport on a
 *           cursor-role surface; buffer_scale is the universal path).
 *
 * A scale_120 below 120 (0 = unknown) is 1; bw, bh <= max_buf.
 */
#define NB_CURSOR_SURFACE_MAX_BUF 1024u
struct nb_cursor_surface {
    struct nb_cursor_geom g;
    uint32_t scale, bw, bh;
};
void nb_cursor_surface_geom(const struct nb_cursor *c, uint32_t num_x,
                            uint32_t den_x, uint32_t num_y, uint32_t den_y,
                            uint32_t scale_120, uint32_t max_buf,
                            struct nb_cursor_surface *out);
/*
 * THE TRUE XOR (D2, 2026-10-17): the pure compose of an XOR cursor's inversion
 * mask onto frame pixels.  The mask is what kayfabe sends under CURSOR_F_XOR
 * (premultiplied ARGB8888, alpha 0xff and the XOR colour where the cursor
 * inverts, all zero elsewhere): a pixel with alpha != 0 selects, its colour is
 * px & 0x00ffffff, and  out = below ^ colour  per channel -- the semantics of
 * the retired kayfabe compose kernel (kf_disp compose_reference: `below ^
 * (w & 0x00ffffff)`).  A transparent mask pixel never changes anything; the
 * frame's top byte is untouched.  The mask's top-left sits at (x, y) on the
 * frame (signed; hot spot irrelevant); everything past the frame is clipped.
 *
 * `frame` is fw x fh XRGB/ARGB pixels, `stride_px` pixels per row.
 *   _apply  inverts in place (a broker-owned copy of the frame).
 *   _patch  reads the frame and writes the mask-sized `out` (c->w x c->h, tight):
 *           0xff000000 | (below ^ colour) where the cursor inverts AND the pixel
 *           is on the frame, 0 elsewhere -- the opaque patch a subsurface shows
 *           over the unchanged frame.  Never writes `frame`.
 * Returns the number of frame pixels inverted / patch pixels set.
 *
 * NOTE: these two take `c->px` as the mask.  For a cursor with `xor` set, px is
 * the APPROXIMATION and the mask is `c->mask` (cropped to c->bx.. by nb_xor_eval,
 * which uses nb_xor_patch_rect()); these remain the whole-cursor reference the
 * tests compare against.
 */
size_t nb_cursor_xor_apply(const struct nb_cursor *c, uint32_t *frame,
                           uint32_t fw, uint32_t fh, size_t stride_px,
                           int32_t x, int32_t y);
size_t nb_cursor_xor_patch(const struct nb_cursor *c, const uint32_t *frame,
                           uint32_t fw, uint32_t fh, size_t stride_px,
                           int32_t x, int32_t y, uint32_t *out);
/* Nearest-neighbour scale of the defined image into dst[0 .. dw*dh), packed.
 * dw, dh must be >= 1. */
void nb_cursor_scale(const struct nb_cursor *c, uint32_t *dst, uint32_t dw,
                     uint32_t dh);
/* The same scaler for any packed sw x sh pixel array. */
void nb_px_scale(const uint32_t *src, uint32_t sw, uint32_t sh, uint32_t *dst,
                 uint32_t dw, uint32_t dh);
/*
 * The XOR patch's scaler: nearest where an axis grows (crisp, like the guest's
 * own cursor image), the mean of the covered source pixels where it shrinks --
 * premultiplied ARGB averaged per channel, so a thin stem that nearest would
 * skip (a 7x11 I-beam at fit 0.6 lost its bottom serif: MEASURED on a real
 * mutter) comes out as partial coverage instead of vanishing.  Pure, bounded.
 */
void nb_px_scale_box(const uint32_t *src, uint32_t sw, uint32_t sh,
                     uint32_t *dst, uint32_t dw, uint32_t dh);

/* ══ THE XOR CURSOR, COMPOSED BY THE BROKER (nb_frame.c, nb_xor.c) ═══════════
 *
 * Design and live-check list: docs/design/V3_BROKER_XOR_CURSOR_20261017.md.
 * Everything a backend decides about an XOR cursor is below, behind pure
 * functions test/test_xor.c drives GPU-free; a backend gathers its state,
 * calls nb_xor_eval(), and does what it returns. */

/*
 * A READ-ONLY WINDOW ONTO ONE FRAME BUFFER, opened when the broker accepts the
 * buffer, closed with it.  Two carriers are readable:
 *   - a sealed (F_SEAL_SHRINK) memfd, the wl_shm tier;
 *   - a LINEAR dma-buf whose exporter is system memory (udmabuf, the system
 *     dma-heap): mmap(PROT_READ), every read bracketed by DMA_BUF_IOCTL_SYNC.
 * Anything else is "not readable" with a reason, and the caller falls back to
 * the approximation.  Only the cursor's own rows are ever read.
 */
enum nb_frame_kind { NB_FRAME_NONE = 0, NB_FRAME_SHM, NB_FRAME_DMABUF };
enum nb_frame_why {
    NB_FW_OK = 0,
    NB_FW_NONE,         /* no frame on screen yet / not opened               */
    NB_FW_FORMAT,       /* not a 32-bit XR24 / AR24 layout                   */
    NB_FW_GEOMETRY,     /* the described extent does not fit the buffer      */
    NB_FW_NOSEAL,       /* a memfd without F_SEAL_SHRINK (SIGBUS risk)       */
    NB_FW_MODIFIER,     /* a dma-buf that is not LINEAR (tiled: unreadable)  */
    NB_FW_EXPORTER,     /* a dma-buf whose memory is not known to be system  */
    NB_FW_FDINFO,       /* cannot learn a dma-buf's exporter                 */
    NB_FW_MMAP,         /* mmap / dup failed                                 */
    NB_FW_STALE,        /* the buffer on screen is not the one opened        */
    NB_FW_RANGE,        /* a read outside the mapping was asked for          */
    NB_FW_SYNC          /* DMA_BUF_IOCTL_SYNC failed                         */
};
struct nb_frame {
    enum nb_frame_kind kind;    /* NB_FRAME_NONE: unreadable, see `why`      */
    enum nb_frame_why  why;
    const uint8_t     *map;     /* exactly what mmap returned               */
    size_t             len;     /* ... and its length                       */
    int                sync_fd; /* dup of the dma-buf (DMABUF only)         */
    uint32_t           w, h, stride, offset, fourcc;
    char               exporter[24];
};
/* The geometry the caller says is on screen, checked against `f` per frame. */
struct nb_frame_geom {
    uint32_t w, h, stride, offset, fourcc;
};
/* DRM_FORMAT_MOD_LINEAR / _INVALID (implicit), spelled locally. */
#define NB_FRAME_MOD_LINEAR  0ULL
#define NB_FRAME_MOD_INVALID 0x00ffffffffffffffULL
/*
 * Open the buffer behind `fd` (borrowed; a dma-buf is dup'ed).  `size` is the
 * extent the caller measured (lseek SEEK_END).  Always fills *f; returns true
 * iff the frame is readable (kind != NONE).  Re-proves what it relies on:
 * seals, geometry against `size`, exporter, modifier.
 */
bool nb_frame_open(struct nb_frame *f, int fd, bool is_shm, uint64_t size,
                   const struct nb_frame_geom *g, uint64_t modifier);
void nb_frame_close(struct nb_frame *f);
/* Per-frame check: `f` is open and describes exactly `g` (and its mapping
 * still covers it).  Returns NB_FW_OK or the reason. */
enum nb_frame_why nb_frame_check(const struct nb_frame *f,
                                 const struct nb_frame_geom *g);
/*
 * Copy the rw x rh pixel rectangle at (rx, ry) into `out` (tight, rw*rh):
 * rows only, each a bounded copy inside the mapping; the DMA-BUF sync pair
 * brackets the whole read.  NB_FW_OK or the reason; on failure `out` is not
 * to be trusted.  rw, rh <= NVKVM_BROKER_CURSOR_MAX_DIM.
 */
enum nb_frame_why nb_frame_read_rect(const struct nb_frame *f, uint32_t rx,
                                     uint32_t ry, uint32_t rw, uint32_t rh,
                                     uint32_t *out);
/* Words a reason is logged with. */
const char *nb_frame_why_text(enum nb_frame_why why);
/* Pure: pick `exp_name` out of /proc/self/fdinfo text.  False when absent. */
bool nb_fdinfo_exp_name(const char *text, size_t len, char *out, size_t n);
/* Pure: the exporters whose memory is system RAM (a CPU read is cheap). */
bool nb_frame_exporter_ok(const char *exp_name);
/* Pure: which modifiers a CPU read can interpret (LINEAR; the implicit
 * modifier only for a system-memory exporter, whose layout is linear). */
bool nb_frame_layout_ok(uint64_t modifier, const char *exp_name);

/* ── the decision (PURE) ─────────────────────────────────────────────────── */
enum nb_xor_kind   { NB_XK_NONE, NB_XK_PLAIN, NB_XK_XOR };
enum nb_xor_ptr    { NB_XP_UNTOUCHED, NB_XP_GUEST_IMAGE, NB_XP_HIDDEN };
enum nb_xor_ov     { NB_XO_OFF, NB_XO_IMAGE, NB_XO_XOR };
enum nb_xor_choice { NB_XC_NONE, NB_XC_PLAIN, NB_XC_COMPOSED, NB_XC_APPROX };
struct nb_xor_in {
    enum nb_xor_kind kind;  /* the cursor: none (undefined/hidden), plain, XOR */
    bool grabbed;
    bool placed;            /* a MOVE gave it a position (grab overlay)        */
    bool frame_readable;    /* the frame on screen can be read right now       */
    bool ptr_inside;        /* hover: the host pointer is over the content     */
    bool serial_ok;         /* the wl_pointer enter serial is present          */
};
struct nb_xor_out {
    enum nb_xor_ptr    ptr; /* what to do with the HOST pointer                */
    enum nb_xor_ov     ov;  /* what the overlay subsurface shows               */
    enum nb_xor_choice choice; /* the standing choice (one log line a change)  */
};
struct nb_xor_out nb_xor_decide(const struct nb_xor_in *in);
const char *nb_xor_choice_text(enum nb_xor_choice c);

/* ── placement (PURE) ────────────────────────────────────────────────────── */
/*
 * Where the patch lands in the surface's LOGICAL pixels, from the patch's
 * top-left (tlx, tly) and size on the guest frame, the frame's fit (nx/dx,
 * ny/dy logical px per guest px) and the output scale.  Both edges are rounded
 * to the nearest logical pixel on their own, so each edge is within 0.5 logical
 * px of where the frame's own pixel boundary falls (0 at fit 1); the size is
 * the difference of the edges (>= 1).  scale = ceil(scale_120/120), 1..4;
 * the buffer is logical x scale, bounded by NB_CURSOR_SURFACE_MAX_BUF.
 */
struct nb_xor_place {
    int32_t  x, y;          /* logical position of the patch's top-left       */
    uint32_t gw, gh;        /* logical size                                   */
    uint32_t scale, bw, bh; /* buffer scale and buffer size                   */
};
void nb_xor_place(int32_t tlx, int32_t tly, uint32_t pw, uint32_t ph,
                  uint32_t nx, uint32_t dx, uint32_t ny, uint32_t dy,
                  uint32_t scale_120, struct nb_xor_place *o);
/* The guest pixel under a surface-local position in wl_fixed units (1/256
 * logical px), clamped to the frame.  nx/dx is the frame's fit (0 = 1:1). */
int32_t nb_xor_ptr_to_frame(int32_t fixed, uint32_t nx, uint32_t dx,
                            uint32_t frame_dim);

/* ── mask-based compose ──────────────────────────────────────────────────── */
/* nb_cursor_xor_patch() for a mask given as a pointer and a row pitch (so a
 * sub-rectangle of a cursor image works): out is mw x mh, tight. */
size_t nb_xor_patch_rect(const uint32_t *mask, uint32_t mstride, uint32_t mw,
                         uint32_t mh, const uint32_t *frame, uint32_t fw,
                         uint32_t fh, size_t stride_px, int32_t x, int32_t y,
                         uint32_t *out);

/* ── the driver every backend shares ─────────────────────────────────────── */
struct nb_xor_view {
    const struct nb_cursor *cur;    /* may be NULL                            */
    bool shown_ok;                  /* no dialog, a guest frame is on screen  */
    bool grabbed, ptr_inside, serial_ok;
    bool ptr_known;
    int32_t ptr_x, ptr_y;           /* hover: the pointer on the GUEST frame  */
    const struct nb_frame *frame;   /* the buffer being shown (may be NULL)   */
    struct nb_frame_geom geom;      /* ... and what the caller says it is     */
};
struct nb_xor_ctx {
    enum nb_xor_choice last_choice;
    enum nb_frame_why  last_why;
    bool        choice_valid;
    /* The patch the DISPLAY holds (nb_xor_attached()), to tell a patch that
     * needs a new buffer from one that is already up -- including when an
     * earlier one could not be attached (no free buffer) and must be retried. */
    bool        att_valid;
    uint32_t    att_pw, att_ph;
    uint64_t    att_gen;
    uint32_t    att[NVKVM_BROKER_CURSOR_MAX_DIM * NVKVM_BROKER_CURSOR_MAX_DIM];
    uint32_t    patch[NVKVM_BROKER_CURSOR_MAX_DIM * NVKVM_BROKER_CURSOR_MAX_DIM];
    uint32_t    rect[NVKVM_BROKER_CURSOR_MAX_DIM * NVKVM_BROKER_CURSOR_MAX_DIM];
    uint64_t    n_composed, n_fallback;     /* evals, for the test backend    */
};
struct nb_xor_res {
    struct nb_xor_out act;
    bool     patch;                 /* act.ov == XOR: px/pw/ph/tl are valid   */
    int32_t  tlx, tly;              /* patch top-left on the guest frame      */
    uint32_t pw, ph;
    const uint32_t *px;             /* pw*ph, tight; 0 = nothing inverted     */
    uint32_t npix;                  /* pixels that invert                     */
    bool     patch_changed;         /* the pixels or the size differ from the
                                     * patch the display holds (or none held);
                                     * a position change alone is NOT this    */
    enum nb_frame_why why;          /* kind XOR but not composed: why not     */
    bool     choice_changed;        /* the standing choice changed: LOG NOW   */
};
void nb_xor_eval(struct nb_xor_ctx *ctx, const struct nb_xor_view *v,
                 struct nb_xor_res *r);
/* The backend attached the result's patch to the display / took it down. */
void nb_xor_attached(struct nb_xor_ctx *ctx, const struct nb_xor_res *r,
                     uint64_t cursor_gen);
void nb_xor_detached(struct nb_xor_ctx *ctx);
/* The ONE line for a change of choice ("cursor: XOR composed by the broker
 * ..." / "... approximated (frame not readable: ...)").  Fills buf. */
void nb_xor_choice_line(const struct nb_xor_res *r, const struct nb_frame *f,
                        char *buf, size_t n);
/* Whether the host pointer over the guest's picture should BE the guest's
 * image: one is defined and visible, and the input is not grabbed. */
bool nb_cursor_wanted(const struct nb_cursor *c, bool grabbed);
/* FNV-1a over the packed pixels -- the test backend's way of saying exactly
 * which image it was handed. */
uint32_t nb_cursor_hash(const struct nb_cursor *c);

/*
 * PAIRS THE DISPLAY REFUSED TO IMPORT after advertising them -- the core's
 * memory (nb_sink.fmt_refused), filled through nb_sink_format_refused().  The
 * X11 backend reports DRI3 refusals here; the Wayland backend keeps the same
 * fact in its `proven` table, which also tracks probes.  Fixed capacity: a
 * client chooses which pairs to try, so it must not choose how much this
 * remembers; when full, the oldest entry is forgotten, which costs at most
 * one more refused import of it.  8, because a DRI3 refusal fills two -- it
 * is a refusal of both alpha twins -- so this holds four refused modifiers.
 */
#define NB_REFUSED_SLOTS 8
struct nb_refused {
    struct { uint32_t fourcc; uint64_t modifier; } e[NB_REFUSED_SLOTS];
    unsigned n, next;
};
void nb_refused_add(struct nb_refused *r, uint32_t fourcc, uint64_t modifier);
bool nb_refused_has(const struct nb_refused *r, uint32_t fourcc,
                    uint64_t modifier);

/* ── the display tick: EV_TICK and the lock-step frame requests ──────────────
 *
 * PURE decisions (nb_tick.c, test/test_tick.c): nothing here does I/O, reads a
 * clock or knows a backend.  Time is passed in; the core feeds it the time of
 * the event (CLOCK_MONOTONIC ns) and the test backend a virtual clock, so every
 * rule below is table-testable to the nanosecond.  See
 * docs/design/V3_DISPLAY_PACING_20261017.md section 11.
 */

/* The slack, in millihertz, a host rate may exceed the VMM's cap by and still
 * be undivided: an output whose nominal rate is 75.2 Hz (a 75 Hz mode, as the
 * display reports it) must not become 37.6 Hz. */
#define NB_TICK_HZ_SLACK_MHZ 500u

/* ★ MEASUREMENT: the per-second WINDOW of the stats line (reset at every line). */
struct nb_tick_win {
    uint64_t gap_max_ns, gap33;          /* max gap between accepted refreshes; gaps > 33 ms             */
    uint64_t ans_max_ns, ans_sum_ns, ans_n;   /* a request's tick sent -> COMMIT/NO_CHANGE arrived       */
    uint64_t pres_max_ns, pres_sum_ns, pres_n; /* a COMMIT arrived -> the display showed/discarded it     */
    uint64_t discarded;                  /* presentation feedbacks the compositor DISCARDED              */
    uint64_t loop_late_max_ns;           /* poll() slept this much past its timeout                      */
    uint64_t loop_work_max_ns;           /* the longest time between poll() returning and the next poll()*/
    uint64_t gaps50, gaps100;            /* accepted-refresh gaps over 50 / 100 ms                       */
    uint64_t new_n, repeat_n, merged;    /* presents that carried NEW content / repeats / commits merged into a later present */
    uint64_t cgap_max_ns, cg33, cg50, cg100; /* gap between consecutive NEW-content presents             */
    uint64_t pump_discarded;             /* feedbacks of the broker's own pump commit the compositor discarded (not a frame's) */
};

struct nb_tick {
    bool     on;             /* negotiated: the VMM sent CLIENT_TICK          */
    unsigned cap_hz;         /* the VMM's cap, 0 = NVKVM_BROKER_GUEST_MAX_HZ  */
    uint32_t last_seq;       /* the last tick_seq emitted, 0 = none yet       */
    uint32_t open_req;       /* the open request (a tick_seq), 0 = none       */
    uint64_t open_since_ns;
    bool     unpresented;    /* an answered frame the display has not shown   */
    uint64_t unpresented_since_ns;
    unsigned div, phase;     /* decimation: every div-th feedback ticks       */
    bool     have_accept;
    uint64_t last_accept_ns; /* the last feedback that was accepted           */
    uint64_t last_present_ns;/* the last present_ns put on the wire           */
    bool     have_real;
    uint64_t last_real_ns;   /* the last compositor timestamp accepted        */
    uint64_t now_ns;         /* the time of the latest event the core saw     */
    /* counters, for the one line at detach and the test backend's stats */
    uint64_t n_feedback, n_ticks, n_want, n_fast, n_decimated;
    uint64_t n_req_timeouts, n_present_timeouts;
    uint64_t n_answered, n_no_change, n_late, n_unsolicited;
    uint64_t n_coalesced;
    struct nb_tick_win win;      /* ★ MEASUREMENT: see nb_tick_stats_format */
    uint64_t commits_since_present; /* ★ MEASUREMENT: COMMITs answered since the last present   */
    bool     have_content;       /* ★ MEASUREMENT: a new-content present was seen        */
    uint64_t last_content_ns;    /* its present_ns (never part of the window)            */
};

/*
 * ★ MEASUREMENT: the per-second stats line.  Pure: the time and the counters are arguments.
 *   nb_tick_snap_take   copies the cumulative counters
 *   nb_tick_stats_due   true (and advances *next_ns by one period, never in a burst) when `now_ns` reached *next_ns;
 *                       the first call arms it and returns false
 *   nb_tick_stats_format writes `stats t=<ms> feedbacks=<d> ticks=<d> want_ticks=<d> commit=<d> no_change=<d>
 *                       timeouts=<d> late=<d> unsolicited=<d> present_timeouts=<d> coalesced=<d>` (deltas against
 *                       `prev`) into buf; returns the length, 0 when buf is too small
 */
struct nb_tick_snap {
    uint64_t feedback, ticks, want, answered, no_change, req_timeouts, late, unsolicited, present_timeouts, coalesced;
};
void nb_tick_snap_take(const struct nb_tick *t, struct nb_tick_snap *s);
bool nb_tick_stats_due(uint64_t now_ns, uint64_t *next_ns, uint64_t period_ns);
int nb_tick_stats_format(char *buf, size_t n, uint64_t t_ms, const struct nb_tick_snap *prev,
                         const struct nb_tick_snap *now, const struct nb_tick_win *w);
/* window updaters (pure) */
void nb_tick_win_gap(struct nb_tick *t, uint64_t gap_ns);
/* A PRESENT happened (a `presented` feedback, the compositor's time `present_ns`).  It carried NEW content when a COMMIT was
 * answered since the previous present, else it repeated the previous frame; commits beyond the first merged into it.  (The
 * compositor discards the feedback of a COMMIT that a later commit replaces, so the content flag is derived from the commits,
 * not from which feedback arrived.)  Tracks the gap between consecutive NEW-content presents. */
void nb_tick_content_present(struct nb_tick *t, uint64_t present_ns);
void nb_tick_win_loop(struct nb_tick *t, uint64_t late_ns, uint64_t work_ns);
void nb_tick_presented_at(struct nb_tick *t, uint64_t now_ns, bool discarded);

#define NB_TICK_STATS_PERIOD_NS 1000000000ull
/* The buffer for one line.  The worst line (every counter 2^64-1, 30 numbers of 20 digits and their names) is under 1000 bytes;
 * test_tick.c formats that worst case into exactly this size. */
#define NB_TICK_STATS_LINE_MAX 1536
/* One step of the per-second line: the first call ARMS (and takes the baseline once: what happened before is not in the first line);
 * a call that is not due writes nothing; a due call formats into buf and, only when the line fitted, moves the baseline and clears
 * the window.  Returns the line's length, 0 when nothing was written (a dropped line resets nothing). */
int nb_tick_stats_step(struct nb_tick *t, struct nb_tick_snap *prev, uint64_t *next_ns, uint64_t now_ns, char *buf, size_t n);
/* The broker's own pump commit's feedback was DISCARDED: counted, and nothing else -- the request book is untouched (a frame's
 * un-presented flag gates WANT, and a pump discard says nothing about a frame). */
void nb_tick_pump_discarded(struct nb_tick *t);

/* What nb_tick_refresh() decided to put on the wire. */
struct nb_tick_out {
    uint32_t tick_seq;
    uint32_t refresh_word;   /* period | NVKVM_BROKER_TICK_F_*                */
    uint64_t present_ns;
};

/* How a reply (COMMIT.seq, or NO_CHANGE) relates to the requests. */
enum nb_tick_ans {
    NB_TICK_ANS_OPEN,        /* it answers the open request: that is closed   */
    NB_TICK_ANS_LATE,        /* issued, but not open (timed out, answered)    */
    NB_TICK_ANS_UNSOLICITED, /* seq 0 on a COMMIT                             */
    NB_TICK_ANS_VIOLATION    /* never issued / 0 on NO_CHANGE                 */
};

void nb_tick_reset(struct nb_tick *t, bool on, unsigned cap_hz);
/* The tick_seq after `seq`: +1, wrapping 0xffffffff to 1 (0 is never a tick). */
uint32_t nb_tick_next_seq(uint32_t seq);
/* Is `a` newer than `b` in wrapping 32-bit order? */
bool nb_tick_seq_newer(uint32_t a, uint32_t b);
/* The divisor: ceil(host rate / the cap) with NB_TICK_HZ_SLACK_MHZ of slack
 * (--refresh auto); round(host / mode) for a fixed rate (--refresh <hz>); and
 * 60 Hz for --refresh none.  1 when the host period is unknown. */
unsigned nb_tick_div(uint64_t host_period_ns, unsigned cap_hz, int hz_mode,
                     unsigned fixed_mhz);
/* The mode rate to advertise in EV_SURFACE under --refresh auto: host/div. */
unsigned nb_tick_mode_mhz(unsigned host_mhz, unsigned div);
/* WANT_FRAME at `now`?  No request open and no answered frame un-presented;
 * or the open request / the un-presented frame is NVKVM_BROKER_TICK_REQ_TIMEOUT_NS
 * old.  (A pure read.) */
bool nb_tick_want(const struct nb_tick *t, uint64_t now_ns);
/* ONE display refresh feedback.  Returns true and fills *o when a tick is to be
 * sent: never more than one per call, never for a duplicate or too-fast
 * feedback (closer than half a host period to the last accepted one), only
 * every div-th accepted feedback.  `flags` are NVKVM_BROKER_TICK_F_CLOCK_ONLY. */
bool nb_tick_refresh(struct nb_tick *t, uint64_t now_ns, uint64_t present_ns,
                     uint64_t host_period_ns, uint32_t flags, int hz_mode,
                     unsigned fixed_mhz, struct nb_tick_out *o);
/* The tick sent when the VMM opts in: WANT_FRAME, CLOCK_ONLY. */
void nb_tick_prime(struct nb_tick *t, uint64_t now_ns, struct nb_tick_out *o);
/* A COMMIT (is_commit) or NO_CHANGE with request number `req`. */
enum nb_tick_ans nb_tick_answer(struct nb_tick *t, uint32_t req, bool is_commit,
                                uint64_t now_ns);
/* The display showed (or discarded) the answered frame. */
void nb_tick_presented(struct nb_tick *t);
/* The TX queue's rule: a queued, unsent tick is REPLACED by a newer one -- unless
 * it carries WANT_FRAME and the newer does not: that is the live request, and
 * the only copy of it.  (A newer WANT_FRAME replaces it: its request is dead,
 * the broker has already moved on.)  So the queue holds at most one live WANT
 * tick and one newer tick behind it, however long the peer does not read. */
bool nb_tick_may_replace(uint32_t queued_refresh_word, uint32_t new_refresh_word);

/* ── the policy core ─────────────────────────────────────────────────────── */
struct nb_session;

struct nb_sink {
    struct nb_session *sess;

    int      client_fd;         /* -1 when nobody is connected                */
    pid_t    client_pid;        /* from SO_PEERCRED, for log lines only       */
    uint64_t client_generation; /* changes across every attach/detach; async
                                 * backend work must match before delivery    */
    uint32_t seq;
    /* ★ MEASUREMENT: the per-second stats line's state */
    struct nb_tick_snap stats_prev;
    uint64_t            stats_next_ns;

    /*
     * The window has been sized once from the guest's own resolution.  After
     * that a guest re-mode changes the SOURCE, not the window -- see the
     * WINDOW case in nb_handle_cmd().
     */
    bool     window_established;
    /* Command-rate limiter (audit B-1b).  A legitimate VMM sends about three
     * commands per frame; anything orders of magnitude above that is a client
     * trying to burn the broker's CPU, not one trying to display. */
    uint64_t rate_ms;           /* start of the current 1s window          */
    unsigned rate_count;
    /*
     * PER-WAKEUP WORK BUDGET (audit 2026-08-29 S-1, S-2).
     *
     * B-1's budget counts COMMANDS, which is only a bound if every command
     * costs about the same.  Two do not: an X11 shm COMMIT copies the whole
     * frame (up to 8192*8192*4 = 256 MB) and an X11 ATTACH of a buffer the
     * cache missed makes a BLOCKING round trip to the X server.  Sixty-four of
     * either is not a bounded amount of work on the thread that also has to
     * dispatch focus-loss and CTRL+ALT+G while the keyboard is grabbed.
     *
     * So the backends say what a command actually cost.  Bytes stop the DRAIN
     * LOOP -- the work has already happened, so the charge bounds what follows
     * it; round trips are refused by the BACKEND before it makes one, because
     * a trip already begun cannot be cut short.  Both are reset at the top of
     * every nb_sink_readable(): a wakeup that ends early loses nothing, since
     * poll() reports POLLIN again immediately and the next call resumes.
     */
    uint64_t work_bytes;        /* copied/pushed so far this wakeup        */
    unsigned work_trips;        /* blocking display round trips this wakeup*/
    /*
     * Rejection-log throttle (audit 2026-08-27 S-4).  A rejected frame is
     * deliberately NOT a disconnect, so the reject path is the one thing a
     * hostile client can drive forever — and every line through nb_err() is a
     * blocking, fflush()ed write.  Log a few per second and count the rest.
     */
    uint64_t rej_log_ms;
    unsigned rej_log_count;
    unsigned rej_log_hidden;

    /*
     * CLIPBOARD, guest -> host.  Reassembled here rather than in a backend so
     * the bounds live next to every other client-input bound.  `n` is a chunk
     * count we keep ourselves; nothing the client sends decides how much we
     * store.
     */
    char     clip_in[NVKVM_BROKER_CLIP_MAX_BYTES + 1];
    unsigned clip_in_len;
    unsigned clip_in_chunks;
    uint32_t clip_in_next_chunk;
    bool     clip_in_active;
    bool     clip_in_bad;       /* this transfer already failed; drain it   */
    uint64_t clip_rate_ms;      /* separate 1s window for clipboard         */
    unsigned clip_rate_count;
    uint64_t clip_last_ms;      /* for the "guest changed your clipboard"
                                 * notice, so a burst says it once          */
    /* Each dead end is said ONCE per connection: a line per keystroke is
     * noise, and noise is why nobody reads the line that mattered. */
    bool     clip_told_off;
    bool     clip_told_noclient;
    bool     clip_told_noagent;
    unsigned clip_held_key;     /* paste key held until content is queued   */
    bool     clip_held_ctrl;
    bool     clip_held_shift;
    uint64_t clip_held_generation;
    unsigned caps_seen;         /* what the client told us it can do        */
    const struct nb_config *cfg;
    bool     focused;
    bool     pointer_in;
    bool     grabbed;
    bool     fullscreen;

    /* Modifier latch for the broker-owned hotkeys.  Tracked from the same key
     * stream the client sees, so the two cannot disagree about reality. */
    bool     ctrl_down, alt_down;
    /* Latched like ctrl/alt: a paste chord needs it, and it must track reality
     * even while unfocused, or the first chord after a focus change is missed. */
    bool     shift_down;

    /* Which keys the CLIENT currently believes are down.  On focus loss we
     * synthesise a release for each: a grab that ends with keys latched down
     * in the guest is both a bug and, for modifiers, a security surprise. */
    unsigned char key_down[768 / 8];
    unsigned char consumed[768 / 8]; /* broker-owned chord key-ups           */

    struct nvkvm_broker_pkt tx[NB_TXRING];
    unsigned tx_head, tx_tail;  /* head==tail ⇒ empty                         */
    size_t   tx_partial;        /* bytes of tx[tx_tail] already written       */

    /* Inbound command accumulator.  EXACTLY one command wide: nothing here is
     * length-driven, so there is no buffer for a client to overrun. */
    uint8_t  rx[NVKVM_BROKER_CMD_SIZE];
    size_t   rxlen;
    int      rxfd;              /* fd banked from a partial ATTACH read       */

    /* Counters, for the one-line status the broker logs on detach. */
    uint64_t n_attach, n_commit, n_reject;

    /*
     * THE GUEST'S CURSOR.  Connection state: forgotten on every attach and
     * detach, like everything else a VMM supplied.
     *
     * Applying it is PACED, not per command: a changed cursor reaches the
     * backend at most once per NB_CURSOR_MIN_MS, latest wins, via
     * nb_sink_tick().  So however fast a client (or a guest animating its
     * pointer behind an honest client) sends SETs, the display server sees a
     * bounded number of cursor uploads per second -- and the last one sent is
     * always the one that ends up on screen.
     *
     * TWO COPIES, and the split is what makes "paced" true.  `cursor` is where
     * SET/HIDE/SHOW land, at once; `cursor_pub` is what the backend is handed
     * and keeps a pointer to, and it changes ONLY in nb_sink_tick() (and on
     * attach/detach, see nb_client_state_reset()).  Handing backends the
     * first one -- as the first version did -- paced the core's ->cursor()
     * calls and nothing else: every backend also re-renders on its own
     * schedule (a frame commit, a configure, a buffer release, the grab
     * ending), and each of those re-read the newest image straight past the
     * interval.  test/test_cursor.py measures the interval across frame
     * commits.
     */
    struct nb_cursor cursor;
    struct nb_hot    cursor_hot;    /* hover: the hot spot derived from MOVEs   */
    struct nb_cursor cursor_pub;
    bool     cursor_dirty;      /* changed since the backend last saw it   */
    uint64_t cursor_applied_ms; /* when the backend last saw it            */
    uint64_t n_cursor, n_cursor_reject;
    /* Which carrier the accepted frames arrive in (0 none yet, 1 dma-buf,
     * 2 shared memory): a change is said ONCE, in the core, because the broker
     * is the one place that can say what it was sent -- see nb_note_tier(). */
    int      last_tier;
    uint64_t n_tier_dmabuf, n_tier_shm;

    /*
     * PAIRS THE DISPLAY REFUSED TO IMPORT after advertising them, as the
     * backend reported them through nb_sink_format_refused().  Consulted by
     * the one resolver ATTACH and QUERY_FORMAT share, so neither can say yes
     * to a pair the other has seen fail.
     *
     * CONNECTION STATE, like the cursor: forgotten on attach and detach.  The
     * refusal is triggered by a buffer the guest chose, so on a --persist
     * broker a memory that outlived the client would let one VM decide the
     * next VM's present path.  The Wayland backend's `proven` table is reset
     * at the same moment for the same reason (wl_client_detach()), so the two
     * backends still say the same thing on the wire: within a connection a
     * refused pair stays refused, and a new connection starts from what the
     * display advertises.  The cost is re-learning: one dropped probe frame on
     * Wayland, one bounded blocking round trip on X11, per refused pair per
     * connection.
     */
    struct nb_refused fmt_refused;
    /*
     * Pairs the client has been TOLD x=0 for on this connection, so the core
     * can tell it whenever ATTACH's format gate refuses a pair it sent --
     * once per pair, not once per frame.  Same lifetime as fmt_refused.
     */
    struct nb_refused fmt_told;

    /* The display tick (EV_TICK / lock-step requests): connection state. */
    struct nb_tick tick;
    unsigned surf_mhz;      /* the mode rate last advertised in EV_SURFACE   */
    unsigned host_mhz;      /* the host rate the last refresh feedback gave  */
    uint64_t host_period_ns;
};

void nb_sink_init(struct nb_sink *s, struct nb_session *sess);
/* Attach the single client.  Performs the handshake.  <0 ⇒ nothing attached. */
int  nb_sink_attach(struct nb_sink *s, int fd);
void nb_sink_detach(struct nb_sink *s, const char *why);
/* Flush queued packets; returns <0 if the client must be dropped. */
int  nb_sink_flush(struct nb_sink *s);
bool nb_sink_want_write(const struct nb_sink *s);
/* Drain and act on whatever the client sent.  Disconnects on any violation. */
void nb_sink_readable(struct nb_sink *s);
/*
 * Clock-driven core work: today, handing a changed guest cursor to the backend
 * once its pacing interval has passed.  Called at the top of every main-loop
 * iteration, BEFORE the backend computes its poll fds, so anything it queues
 * on the display connection is flushed by that same iteration.  Returns the
 * milliseconds until it next has work, or -1 for none.
 */
int  nb_sink_tick(struct nb_sink *s);
/* The backend learned a different display device (sess->dev_*).  Re-sends
 * EV_DEVICE to a connected client; a no-op with nobody connected, since the
 * handshake carries the current value anyway. */
void nb_sink_device_changed(struct nb_sink *s);

/*
 * WHAT THAT COMMAND COST, reported by the backend that paid it.  Both of these
 * exist because the drain loop cannot see inside ->attach()/->commit(), and
 * one command there can be worth thousands of ordinary ones.
 *
 * nb_sink_charge_work(): `bytes` the backend copied or pushed through the
 * display connection for this command.  Purely additive — the work has already
 * happened; the charge is what makes the loop yield before the NEXT one.
 *
 * nb_sink_take_roundtrip(): ask permission for one BLOCKING round trip to the
 * display server.  False means the wakeup's allowance is gone and the backend
 * must fail the command instead (a dropped frame, not a stalled loop).  Only a
 * backend that has no non-blocking way to hear "the server refused this" needs
 * this; the Wayland backend deliberately has none to ask about.
 */
void nb_sink_charge_work(struct nb_sink *s, uint64_t bytes);
bool nb_sink_take_roundtrip(struct nb_sink *s);

/* Called by session backends.  `code` values are Linux evdev codes on every
 * backend — that is what makes the wire format backend-independent. */
void nb_sink_key(struct nb_sink *s, unsigned code, bool down);
void nb_sink_btn(struct nb_sink *s, unsigned code, bool down);
void nb_sink_abs(struct nb_sink *s, int x, int y, unsigned w, unsigned h);
void nb_sink_rel(struct nb_sink *s, int dx, int dy);
void nb_sink_wheel(struct nb_sink *s, int v, int h);
void nb_sink_focus(struct nb_sink *s, bool active);
void nb_sink_pointer(struct nb_sink *s, bool inside);
/*
 * Tell the client a (fourcc, modifier) turned out to be unusable AFTER we said
 * it was advertised.  A compositor can advertise a modifier its own driver then
 * refuses to bind -- MEASURED on GNOME/Mutter with NVIDIA 595.84, which
 * advertises XR24+LINEAR and answers the import with "Could not bind the given
 * EGLImage to a CoglTexture2D".  Without this the VMM would keep sending a pair
 * we have proven does not work.
 */
void nb_sink_format_verdict(struct nb_sink *s, uint32_t fourcc, uint64_t mod,
                            bool usable);
/*
 * The display REFUSED an import of a pair it advertised, and the backend wants
 * the core to remember it: from now until the client detaches, ATTACH and
 * QUERY_FORMAT both say no to the pair, and the client is told so with an
 * unsolicited EV_FORMAT x=0 for EVERY fourcc this refuses.
 *
 * `both_twins`: the import path cannot see the fourcc at all -- DRI3 imports
 * by depth and bpp, so XR24 and AR24 are one buffer description to it -- and
 * a refusal of one is a refusal of the other.  Both are then remembered AND
 * both announced: a relay holding x=1 for the twin it did not use would
 * otherwise keep sending it into a refusal it was never told about.
 */
void nb_sink_format_refused(struct nb_sink *s, uint32_t fourcc, uint64_t mod,
                            bool both_twins);
void nb_sink_surface(struct nb_sink *s, unsigned w, unsigned h,
                     unsigned refresh_mhz);
void nb_sink_frame(struct nb_sink *s);
/*
 * The display refreshed (a wp_presentation `presented`, or the frame callback
 * that stands in for it): `present_ns` is CLOCK_MONOTONIC ns, `host_period_ns`
 * the output's nominal period (0 = unknown), `flags` NVKVM_BROKER_TICK_F_CLOCK_ONLY
 * when present_ns is not the compositor's timestamp.  The CORE decides what goes
 * on the wire (decimation, WANT_FRAME, the rate rules): a backend just reports
 * every feedback it gets, once.  A no-op with no client or one that did not opt
 * in.  Returns whether a tick was queued.
 */
bool nb_sink_refresh(struct nb_sink *s, uint64_t now_ns, uint64_t present_ns,
                     uint64_t host_period_ns, uint32_t flags);
/* The display showed (or discarded) the frame the VMM last COMMITted. */
void nb_sink_presented(struct nb_sink *s);
/* Did the VMM opt in to ticks?  The mode rate the broker advertises follows. */
bool nb_sink_tick_on(const struct nb_sink *s);
/* ★ MEASUREMENT: emit the per-second stats line if --stats-line is on and one is due (event-driven: called from the tick
 * and reply paths and from the backend's existing pacing callback; no timer of its own). */
void nb_sink_stats_poll(struct nb_sink *s);
/* ★ MEASUREMENT: the compositor DISCARDED a presentation feedback; the event loop's lateness / body time. */
void nb_sink_note_discarded(struct nb_sink *s);
void nb_sink_note_loop(struct nb_sink *s, uint64_t late_ns, uint64_t work_ns);
void nb_sink_note_pump_discarded(struct nb_sink *s);
void nb_sink_note_present(struct nb_sink *s, uint64_t present_ns);
/* The EV_SURFACE rate for a host rate of `host_mhz`, after the tick divisor. */
unsigned nb_sink_mode_mhz(const struct nb_sink *s, unsigned host_mhz,
                          uint64_t host_period_ns);
void nb_sink_release(struct nb_sink *s, uint64_t buf_id);
/* The user closed the display.  Reports it to the client and returns; the
 * client decides what that means for the VM.  Returns false when nobody is
 * connected, in which case the caller should just quit -- there is no policy
 * to defer to. */
bool nb_sink_close_request(struct nb_sink *s, int action);
/* The BACKEND reporting what the display server actually did.  Fullscreen can
 * be entered and left without our hotkey -- a compositor binding, --fullscreen
 * at startup -- and the F_FULLSCREEN flag on every packet has to be the truth,
 * not our last request, because the client keys a guest mode switch off it. */
void nb_sink_set_fullscreen(struct nb_sink *s, bool on);
/* Is a VMM actually connected?  A backend asking "is there anything here to
 * shut down" -- with no client there is no policy to defer to. */
bool nb_sink_has_client(const struct nb_sink *s);
/* Drop the grab because the broker is about to show host UI.  A host dialog
 * while the guest holds the keyboard is incoherent, and it is the state a
 * stuck grab is least recoverable from. */
void nb_sink_force_ungrab(struct nb_sink *s);
/* Host clipboard text on its way to the guest, already size-checked.  The
 * generation captured when the fetch began must still name the current
 * connection and held paste operation.  Returns false for stale completion or
 * ring pressure, so neither can disclose or paste the wrong content. */
bool nb_sink_send_clipboard(struct nb_sink *s, uint64_t generation,
                            const char *text, size_t len);
/* A backend telling the core a paste trigger fired.  The core owns the policy:
 * which mode we are in, whether anyone is connected, and what to say when the
 * answer is no. */
void nb_sink_paste_trigger(struct nb_sink *s);
/* Complete an asynchronous host-selection fetch.  `generation` is the value
 * captured when it began; stale completion from an old persistent client is
 * ignored.  When `paste` is true a coherent, balanced paste chord is replayed
 * after the clipboard packets.  False cancels it visibly without pasting stale
 * guest content. */
void nb_sink_clip_finish(struct nb_sink *s, uint64_t generation, bool paste);
void nb_sink_bye(struct nb_sink *s, int reason);

/* ── session backends ────────────────────────────────────────────────────── */
struct nb_session_ops {
    const char *name;
    /* Create the window and the input source.  Fills the public fields below.
     * Returns 0, or -errno with a human explanation already logged. */
    int  (*open)(struct nb_session *s, const struct nb_config *cfg);
    void (*close)(struct nb_session *s);
    /* Fds to poll, WITH their events already set — a backend that has queued
     * output the display server has not taken yet asks for POLLOUT itself, so
     * flushing the display can never sit inside the input path.  Returns how
     * many were written to `out`, or <0. */
    int  (*pollfds)(struct nb_session *s, struct pollfd *out, int max);
    /* Drain whatever those fds have, pushing into `sink`.  Returns <0 on a
     * fatal session error (the display went away). */
    int  (*dispatch)(struct nb_session *s, struct nb_sink *sink);
    int  (*set_grab)(struct nb_session *s, bool on);
    int  (*set_fullscreen)(struct nb_session *s, bool on);

    /* Is this (fourcc, modifier) pair one the GPU actually advertises?  NOT a
     * hardcoded list: each backend answers from the set the compositor or X
     * server enumerated at startup. */
    bool (*format_ok)(struct nb_session *s, uint32_t fourcc, uint64_t mod);
    /* Import an already-validated dma-buf and make it the pending content.
     * Does NOT take ownership of desc->fd.  Returns 0 or -errno. */
    int  (*attach)(struct nb_session *s, const struct nb_buf_desc *desc);
    /* Put the pending content on screen.  Returns 0 or -errno. */
    int  (*commit)(struct nb_session *s, struct nb_sink *sink);
    /* Ask for a window of this size.  Advisory; may be ignored. */
    int  (*resize)(struct nb_session *s, unsigned w, unsigned h);

    /* Cancel connection-scoped asynchronous work before the sink generation
     * changes.  Optional on backends with no such work. */
    void (*client_detach)(struct nb_session *s, uint64_t generation);

    /*
     * Show the "no VM attached" placeholder.  Called once before the first
     * client can connect and again whenever one goes away, so that an idle
     * broker is visibly idle rather than an unmapped window.  Optional: a
     * backend that leaves this NULL simply shows nothing until a real frame
     * arrives, which is what every backend did before.
     */
    int  (*show_idle)(struct nb_session *s);
    /*
     * Cancel any modal UI the backend is showing.  OPTIONAL -- may be NULL,
     * and is on the X11 backend, which draws no dialog.  Called when the
     * broker takes a state that contradicts one (grab on, fullscreen on):
     * the later action wins, rather than leaving two modes fighting.
     */
    void (*dismiss_dialog)(struct nb_session *s);
    /*
     * Re-assert input state to a client that has just attached.  OPTIONAL.
     *
     * Focus and pointer-inside are EDGE-triggered: the backend learns them
     * from events, and a client that connects afterwards never sees the edge.
     * MEASURED on X11 under openbox, which focuses a window when it maps: the
     * broker window took focus before the VM connected, no FocusIn ever
     * followed, and the guest therefore believed it was unfocused forever --
     * so no key and no pointer motion reached it, while CTRL+ALT+G still
     * worked because the hotkey is handled before the focus gate.  Wayland
     * hid the same bug behind a compositor that re-sends enter on click.
     */
    void (*resync)(struct nb_session *s);
    /* Clipboard.  OPTIONAL -- NULL on a backend that has no selection, in
     * which case clipboard is simply unavailable rather than half-working. */
    /*
     * Put the guest's text on the host clipboard.
     *   0   applied -- it IS the host clipboard now
     *   1   HELD -- accepted, but not applied, because the window is not
     *       focused.  The backend owns it and will apply it on focus-in, and
     *       is responsible for the notice AT THAT MOMENT: a title-bar notice
     *       raised while the window is unfocused is one nobody can see.
     *   <0  -errno
     */
    int  (*set_clipboard)(struct nb_session *s, const char *text, size_t len);
    /*
     * Do any work that has come DUE ON A CLOCK rather than on an event, and
     * return the milliseconds until the next such deadline, or -1 for none.
     * OPTIONAL.
     *
     * The main loop polls with an infinite timeout, which is right: it should
     * sleep until something happens.  But a title-bar notice that expires
     * after four seconds is something happening with no fd behind it, so
     * without this it stayed on screen until an unrelated event redrew the
     * bar -- a transient message that was not, in practice, transient.
     */
    int  (*tick)(struct nb_session *s);
    void (*notify_clipboard)(struct nb_session *s);
    /*
     * The clock the display tick's rules judge COMMIT / NO_CHANGE / CAPS by,
     * CLOCK_MONOTONIC nanoseconds.  OPTIONAL: NULL means the real clock.  Only
     * the test backend has one (a virtual clock driven by its stdin, so a tick
     * test is deterministic to the nanosecond).
     */
    uint64_t (*now_ns)(struct nb_session *s);
    /* Begin an asynchronous read of the host selection.  The backend captures
     * `generation`, queues content with nb_sink_send_clipboard(), then calls
     * nb_sink_clip_finish() with the same generation. */
    int  (*fetch_clipboard)(struct nb_session *s, struct nb_sink *sink,
                            uint64_t generation);
    /*
     * The guest's cursor changed: a new image, HIDE/SHOW, or forgotten on a
     * detach.  OPTIONAL, and paired with NVKVM_BROKER_CAP_CURSOR: a backend
     * leaves this NULL exactly when it does not set that bit, and the core
     * then treats CMD_CURSOR as the violation an older broker would.
     *
     * `cur` is the core's PUBLISHED snapshot (nb_sink.cursor_pub) and stays
     * valid for the life of the process; the backend may keep the pointer and
     * re-read it whenever it needs to re-render (a resize changing the frame's
     * scale, the grab ending, a frame commit), and what it finds there is
     * only ever what the pacing has let through -- never a SET the interval
     * is still holding back.  It must not block, and it decides what is
     * actually shown:
     * nb_cursor_wanted() with the backend's OWN grab state, which is the
     * truth earlier than the core's during a grab transition.
     */
    void (*cursor)(struct nb_session *s, const struct nb_cursor *cur);
};

#define NB_SESSION_CLIP_G2H (1u << 0)
#define NB_SESSION_CLIP_H2G (1u << 1)

struct nb_sink;

struct nb_session {
    const struct nb_session_ops *ops;
    /*
     * The connected client's sink, set at ATTACH -- before any event has been
     * dispatched.  A backend that only learns its sink inside dispatch cannot
     * re-emit anything during ->resync(), which is the one moment resync
     * exists for: MEASURED on X11, x11_resync() returned at its first guard on
     * every first attach, so focus and pointer state that predated the client
     * were never re-sent and the backend could not tell "attached" from "no
     * client" at all.
     */
    struct nb_sink *sink;
    void    *priv;

    uint32_t width, height;     /* current window size, in pixels             */
    uint32_t caps;              /* NVKVM_BROKER_CAP_*                         */
    uint32_t clipboard_caps;    /* NB_SESSION_CLIP_* actually implemented     */
    /*
     * Backends whose import path cannot tell a dma-buf from any other fd are
     * not a thing — but the TEST backend has no import path at all and must be
     * able to exercise the accept side of the validator without a GPU.  It is
     * the only backend that sets this, it is never reachable from
     * --backend auto, and it prints a banner saying it drives nothing.
     *
     * IT MEANS EXACTLY ONE THING: a memfd may stand in for a dma-buf on the
     * ORDINARY import path.  It is NOT the shm tier's gate — see accept_shm.
     * Audit 2026-08-27 S-1: the real backends set this to make the shm tier
     * work, which silently deleted the "the fd is proved to be a dma-buf"
     * rule (design §3 rule 4) for every ordinary frame as well.  One flag
     * cannot answer two questions.
     */
    bool     accept_memfd;
    /*
     * This backend can PRESENT a memfd declared with NVKVM_BROKER_CMD_F_SHM.
     * Distinct from accept_memfd because the shm tier authenticates the fd
     * itself (TMPFS_MAGIC + F_SEAL_SHRINK) rather than relaxing the dma-buf
     * proof for everything else.
     */
    bool     accept_shm;
    /* One line, printed at startup and again at connect, naming exactly what
     * this stack cannot capture.  Empty string means "everything". */
    char     grab_caveat[256];
    /*
     * The DRM device the display server renders/imports on, for EV_DEVICE.
     * Filled by the backend from the display server's own answer (never from
     * anything a client sent); all zero means "unknown", which is a valid
     * answer and is sent as such.
     */
    uint32_t dev_flags;         /* NVKVM_BROKER_DEVICE_F_*                    */
    uint32_t dev_major, dev_minor;
};

/* Backend constructors.  Each returns NULL (after logging) when its stack is
 * not present or not usable; nb_session_open() tries them in order. */
struct nb_session *nb_session_x11(const struct nb_config *cfg);
struct nb_session *nb_session_wayland(const struct nb_config *cfg);
/* No display behind it; see nb_session_test.c.  Never selected by
 * --backend auto — it must be asked for by name. */
struct nb_session *nb_session_test(const struct nb_config *cfg);

/* Pick and open a backend per cfg->backend. */
struct nb_session *nb_session_open(const struct nb_config *cfg);

/* ── shared helpers ──────────────────────────────────────────────────────── */

/* Bytes per pixel for the single-plane 32-bit formats the nvkvm head can
 * flip, or 0 for anything else.  0 means "reject": a format whose pitch we
 * cannot compute is a format whose bounds we cannot check. */
/* Presentation tiers, worst-to-best fallback order.  See nb_common.c. */
enum { NB_TIER_AUTO = 0, NB_TIER_NATIVE, NB_TIER_LINEAR, NB_TIER_SHM };
/* What refresh the broker tells the guest.  See nb_common.c. */
enum { NB_HZ_AUTO = 0, NB_HZ_NONE, NB_HZ_FIXED };
extern int      nb_hz_mode;
extern unsigned nb_hz_fixed;

/* Which resolution the broker SUGGESTS to the guest.  See nb_common.c. */
enum { NB_RES_AUTO = 0, NB_RES_NONE, NB_RES_FIXED };
extern int      nb_res_mode;
extern unsigned nb_res_w, nb_res_h;

extern int  nb_tier;
extern bool nb_linear_only;   /* deprecated alias for NB_TIER_LINEAR */

/*
 * nb_geom.c.  The width the broker SUGGESTS to the guest is a multiple of
 * NB_HINT_W_ALIGN pixels: at 32 bits per pixel the guest's frame then has a row
 * that is a multiple of 256 bytes, the pitch every GPU's dma-buf import accepts
 * (MEASURED 2026-10-15: a 2080-wide frame, row 8320 = 65 x 128, was refused by
 * an AMD-iGPU compositor; 1920 and 2048 were not).  Below one unit the CVT
 * granularity of 8 is all that is asked.  Heights need no rule.
 */
#define NB_HINT_W_ALIGN 64u
unsigned nb_hint_width(unsigned w);

/*
 * The buffer geometries a display has been PROVEN to import.  Whether an import
 * succeeds depends on the buffer, not only on its (fourcc, modifier): the
 * pair was proven at 1920x1080 and refused at 2080x1171.  A small ring; a
 * forgotten geometry costs one probe, never the connection.
 */
#define NB_GEOM_SLOTS 8
struct nb_geom {
    bool     used;
    uint32_t fourcc;
    uint64_t modifier;
    uint32_t width, height, stride, offset;
};
struct nb_geom_set {
    struct nb_geom e[NB_GEOM_SLOTS];
    unsigned       next;
};
bool nb_geom_has(const struct nb_geom_set *s, uint32_t fourcc, uint64_t mod,
                 uint32_t w, uint32_t h, uint32_t stride, uint32_t offset);
void nb_geom_add(struct nb_geom_set *s, uint32_t fourcc, uint64_t mod,
                 uint32_t w, uint32_t h, uint32_t stride, uint32_t offset);
void nb_geom_clear(struct nb_geom_set *s);

uint32_t nb_fourcc_bpp(uint32_t fourcc);
/* The opaque twin of an alpha format (AR24->XR24), or 0 if there is none.
 * Same byte layout; only the alpha channel's meaning differs. */
uint32_t nb_fourcc_opaque_twin(uint32_t fourcc);
/* Human name for a modifier's vendor half — INTEL/NVIDIA/AMD/LINEAR/... */
const char *nb_modifier_vendor(uint64_t modifier);
const char *nb_fourcc_name(uint32_t fourcc, char buf[8]);

/*
 * A backend's advertised (fourcc, modifier) set.  Filled from what the display
 * server enumerated; consulted by the generic format_ok helper.  Fixed
 * capacity, because the number of entries a compositor sends is not something
 * the broker should let determine an allocation.
 */
#define NB_MAX_FORMATS 256
struct nb_formats {
    struct { uint32_t fourcc; uint64_t modifier; } e[NB_MAX_FORMATS];
    unsigned n;
    bool     overflowed;
};
void nb_formats_add(struct nb_formats *f, uint32_t fourcc, uint64_t modifier);
bool nb_formats_has(const struct nb_formats *f, uint32_t fourcc, uint64_t mod);
void nb_formats_log(const struct nb_formats *f, const char *what);

/*
 * Resolve a DRM character device to the RENDER node of the same GPU through
 * sysfs (/sys/dev/char/M:m/device/drm/renderD*), so EV_DEVICE names the node
 * a relay compares against whichever node the display server happened to
 * report.  Fills *flags with NVKVM_BROKER_DEVICE_F_* and the resolved (or, if
 * resolution failed, the original) numbers.  Reads only the host's sysfs.
 */
void nb_drm_device_resolve(uint32_t major, uint32_t minor, uint32_t *flags,
                           uint32_t *out_major, uint32_t *out_minor);

/*
 * Paint the idle placeholder into a 32-bit XRGB/ARGB buffer (nb_placeholder.c).
 * `stride_px` is in pixels.  CPU only — no GL, no dma-buf, deliberately.
 */
void nb_placeholder_paint(uint32_t *px, unsigned w, unsigned h,
                          unsigned stride_px, const char *line1,
                          const char *line2);
/* What a client reports it supports.  Today only the clipboard bit exists;
 * a VMM with no clipboard agent must not be told to prepare a paste. */
#define NB_CLIENT_HAS_CLIPBOARD (1u << 0)

/* nb_config.scale_mode */
#define NB_SCALE_NONE    0      /* 1:1, centred, no scaling at all           */
#define NB_SCALE_STRETCH 1      /* fill the window, ignore aspect, distort   */
#define NB_SCALE_ASPECT  2      /* as large as fits, aspect kept, black bars */

/* The same 5x7 font, for the client-side title bar.  Uppercase-only. */
unsigned nb_placeholder_text_w(const char *s, unsigned scale);
void nb_placeholder_fill(uint32_t *px, unsigned w, unsigned h,
                         unsigned stride_px, uint32_t argb);
void nb_placeholder_text(uint32_t *px, unsigned w, unsigned h,
                         unsigned stride_px, unsigned x, unsigned y,
                         const char *s, unsigned scale, uint32_t argb);

#endif /* NVKVM_BROKER_H */
