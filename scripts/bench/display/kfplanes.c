/*
 * kfplanes — the display-PLANE oracle (docs/design/V3_DISPLAY_PLANES_ORACLE.md).
 *
 * Runs IN A LINUX GUEST against the stock nvidia-drm card node (nvidia-drm.modeset=1, no other DRM
 * master: stop the display manager first). It drives the KMS features a Windows desktop also uses —
 * overlay planes (RGB and YUV), plane-only flips, enable/disable of a plane, scaling, cursor, colour
 * LUT/CTM updates, async flips, DPMS cycles, vblank waits, out-fences — through ATOMIC commits with
 * PAGE_FLIP_EVENT, and times every completion. nvidia-drm turns each commit into NVKMS window/core
 * UPDATEs (nvkms-evo3.c) and delivers the event only after NVKMS's per-layer FLIP_OCCURRED (an RM
 * event on the window channel, raised by the window's AWAKEN notifier: nvkms-evo.c
 * CompletionNotifierEvent; nvidia-drm-modeset.c __will_generate_flip_event counts one per plane that
 * had a framebuffer before the commit). So an event that never comes, or comes a frame late, is the
 * Linux twin of the Windows "plane 1 never completes" TDR (V3_FEATURE_GAPS.md G01).
 *
 *   kfplanes list  <card>                 connectors / CRTCs / planes / formats / properties
 *   kfplanes all   <card> [n]             every test, n flips each (default 120); one result line each
 *   kfplanes show  <card> <what> <hold_s> scan out a known primary + overlay (what = rgb|nv12|p010|
 *                                         yuyv|nv12-scaled) and hold it, for a host screendump
 *   kfplanes ppm   <w> <h> <what>         (built with -DKFPLANES_NO_DRM) the expected composed frame of
 *                                         `show <what>` as a binary PPM, BT.709 limited range
 *
 * Output: KEY=VALUE lines (KFPLANES_*). Every wait has a deadline; a lost event is a counted timeout
 * and the test recovers with a blocking commit (which nvidia-drm bounds at 3 s, "Flip event timeout").
 * Result line per test:
 *   KFPLANES_RESULT test=<t> commits= events= timeouts= errors= ebusy= lat_avg_us= lat_p95_us=
 *     lat_max_us= over1f= over2f= seq0= seq1= seq2= seqN= frame_us= verdict=PASS|SLOW|FAIL|SKIP why=
 * (seq0 counts events whose vblank sequence did not move: nvidia-drm on kernels with
 * NV_DRM_CRTC_STATE_HAS_NO_VBLANK has no vblank counter, every event carries 0; then only latency grades)
 * PASS: every commit got its event, at most 5% later than one frame + 2 ms after the commit, none later
 * than two frames, and
 * consecutive flips advanced the vblank sequence by exactly one (a flip per vblank, as on hardware).
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <math.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#ifndef KFPLANES_NO_DRM
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <drm_fourcc.h>
#else
#define DRM_FORMAT_XRGB8888 0x34325258u
#define DRM_FORMAT_ARGB8888 0x34325241u
#define DRM_FORMAT_NV12 0x3231564eu
#define DRM_FORMAT_P010 0x30313050u
#define DRM_FORMAT_YUYV 0x56595559u
#endif

/* ---------------------------------------------------------------- patterns (pure functions) */

/* primary: a gradient with a grid, opaque XRGB */
static uint32_t prim_px(uint32_t x, uint32_t y, uint32_t w, uint32_t h, int b)
{
    uint32_t r = (x * 255) / (w ? w : 1), g = (y * 255) / (h ? h : 1), bl = b ? 0xC0 : 0x40;
    if ((x % 64) == 0 || (y % 64) == 0) r = g = bl = 0xFF;
    return 0xFF000000u | (r << 16) | (g << 8) | bl;
}

/* overlay YUV bars: 8 vertical bars, BT.709 limited-range code values (8-bit) */
static const uint8_t BAR_Y[8] = { 235, 219, 188, 173, 78, 63, 32, 16 };
static const uint8_t BAR_U[8] = { 128, 16, 154, 42, 214, 102, 240, 128 };
static const uint8_t BAR_V[8] = { 128, 138, 16, 26, 230, 240, 118, 128 };
static int bar_of(uint32_t x, uint32_t w) { return (int)((x * 8) / (w ? w : 1)); }

/* overlay RGB: solid-coloured quadrants, alternating per buffer */
static uint32_t ovl_rgb_px(uint32_t x, uint32_t y, uint32_t w, uint32_t h, int b)
{
    int q = (x >= w / 2) + 2 * (y >= h / 2);
    static const uint32_t A[4] = { 0xFFFF0000, 0xFF00FF00, 0xFF0000FF, 0xFFFFFFFF };
    static const uint32_t B[4] = { 0xFF00FFFF, 0xFFFF00FF, 0xFFFFFF00, 0xFF202020 };
    return b ? B[q] : A[q];
}

static uint8_t clamp8(double v) { return v < 0 ? 0 : v > 255 ? 255 : (uint8_t)lrint(v); }
/* BT.709 limited -> full-range RGB (the reference, in double precision) */
static __attribute__((unused)) uint32_t yuv709_to_rgb(int Y, int U, int V)
{
    double y = (Y - 16) * 255.0 / 219.0, u = (U - 128) * 255.0 / 224.0, v = (V - 128) * 255.0 / 224.0;
    double r = y + 1.5748 * v, g = y - 0.1873 * u - 0.4681 * v, b = y + 1.8556 * u;
    return 0xFF000000u | ((uint32_t)clamp8(r) << 16) | ((uint32_t)clamp8(g) << 8) | clamp8(b);
}

/* the overlay rectangle used by `show` and by the flip tests (destination on the CRTC) */
struct rect { uint32_t x, y, w, h; };
static struct rect ovl_dst(uint32_t W, uint32_t H, int scaled)
{
    struct rect r = { W / 8, H / 8, 640, 360 };
    if (scaled) { r.w = 960; r.h = 540; }
    if (r.x + r.w > W) r.w = W - r.x;
    if (r.y + r.h > H) r.h = H - r.y;
    return r;
}
#define OVL_SRC_W 640u
#define OVL_SRC_H 360u

static __attribute__((unused)) double now_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e6 + t.tv_nsec / 1e3;
}

#ifdef KFPLANES_NO_DRM
int main(int argc, char **argv)
{
    if (argc < 5 || strcmp(argv[1], "ppm")) { fprintf(stderr, "usage: kfplanes ppm <w> <h> <what>\n"); return 2; }
    uint32_t W = atoi(argv[2]), H = atoi(argv[3]);
    const char *what = argv[4];
    int scaled = !strcmp(what, "nv12-scaled");
    struct rect d = ovl_dst(W, H, scaled);
    printf("P6\n%u %u\n255\n", W, H);
    for (uint32_t y = 0; y < H; y++)
        for (uint32_t x = 0; x < W; x++) {
            uint32_t p = prim_px(x, y, W, H, 0);
            if (x >= d.x && x < d.x + d.w && y >= d.y && y < d.y + d.h) {
                uint32_t sx = (x - d.x) * OVL_SRC_W / d.w, sy = (y - d.y) * OVL_SRC_H / d.h;
                if (!strcmp(what, "rgb")) p = ovl_rgb_px(sx, sy, OVL_SRC_W, OVL_SRC_H, 0);
                else { int b = bar_of(sx, OVL_SRC_W); p = yuv709_to_rgb(BAR_Y[b], BAR_U[b], BAR_V[b]); }
            }
            unsigned char px[3] = { (p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF };
            fwrite(px, 1, 3, stdout);
        }
    return 0;
}
#else

/* ---------------------------------------------------------------- DRM state */

static int fd = -1;
static uint32_t crtc_id, crtc_idx, conn_id, mode_blob;
static drmModeModeInfo mode;
static double frame_us = 16667;

enum { P_FB, P_CRTC, P_SX, P_SY, P_SW, P_SH, P_CX, P_CY, P_CW, P_CH, P_ZPOS, P_ALPHA, P_BLEND, P_ROT,
       P_ENC, P_RANGE, P_INFENCE, P_N };
static const char *PNAMES[P_N] = { "FB_ID", "CRTC_ID", "SRC_X", "SRC_Y", "SRC_W", "SRC_H", "CRTC_X",
    "CRTC_Y", "CRTC_W", "CRTC_H", "zpos", "alpha", "pixel blend mode", "rotation", "COLOR_ENCODING",
    "COLOR_RANGE", "IN_FENCE_FD" };

struct plane {
    uint32_t id, type, possible, nfmt;
    uint32_t *fmts;
    uint32_t prop[P_N];
};
static struct plane planes[32];
static int nplanes;
static struct plane *primary, *overlay[8], *cursor;
static int noverlay;
static uint32_t c_active, c_mode, c_gamma, c_degamma, c_ctm, c_outfence, c_gamma_size, c_degamma_size,
    c_vrr, k_crtc;

static uint32_t prop_lookup(uint32_t obj, uint32_t type, const char *name, uint64_t *val)
{
    drmModeObjectPropertiesPtr ps = drmModeObjectGetProperties(fd, obj, type);
    uint32_t id = 0;
    if (!ps) return 0;
    for (uint32_t i = 0; i < ps->count_props && !id; i++) {
        drmModePropertyPtr p = drmModeGetProperty(fd, ps->props[i]);
        if (p && !strcmp(p->name, name)) { id = p->prop_id; if (val) *val = ps->prop_values[i]; }
        drmModeFreeProperty(p);
    }
    drmModeFreeObjectProperties(ps);
    return id;
}

static const char *ptype(uint32_t t)
{
    return t == DRM_PLANE_TYPE_PRIMARY ? "primary" : t == DRM_PLANE_TYPE_OVERLAY ? "overlay" :
           t == DRM_PLANE_TYPE_CURSOR ? "cursor" : "?";
}

static int has_fmt(const struct plane *p, uint32_t f)
{
    for (uint32_t i = 0; p && i < p->nfmt; i++) if (p->fmts[i] == f) return 1;
    return 0;
}

static int setup(const char *card, int verbose)
{
    fd = open(card, O_RDWR | O_CLOEXEC);
    if (fd < 0) { printf("KFPLANES_FAIL=open %s %s\n", card, strerror(errno)); return -1; }
    if (drmSetClientCap(fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1) || drmSetClientCap(fd, DRM_CLIENT_CAP_ATOMIC, 1)) {
        printf("KFPLANES_FAIL=atomic-cap %s\n", strerror(errno)); return -1;
    }
    drmSetMaster(fd);
    drmModeResPtr res = drmModeGetResources(fd);
    if (!res) { printf("KFPLANES_FAIL=resources %s\n", strerror(errno)); return -1; }
    drmModeConnectorPtr conn = NULL;
    for (int i = 0; i < res->count_connectors; i++) {
        drmModeConnectorPtr c = drmModeGetConnector(fd, res->connectors[i]);
        if (verbose && c)
            printf("KFPLANES_CONNECTOR id=%u type=%u status=%d modes=%d encoders=%d\n", c->connector_id,
                   c->connector_type, c->connection, c->count_modes, c->count_encoders);
        if (c && c->connection == DRM_MODE_CONNECTED && c->count_modes > 0 && !conn) conn = c;
        else drmModeFreeConnector(c);
    }
    if (!conn) { printf("KFPLANES_FAIL=no-connected-connector\n"); return -1; }
    conn_id = conn->connector_id;
    mode = conn->modes[0];
    for (int i = 0; i < conn->count_modes; i++) if (conn->modes[i].type & DRM_MODE_TYPE_PREFERRED) { mode = conn->modes[i]; break; }
    /* the CRTC: the one the encoder names, else the first one the encoder can drive */
    drmModeEncoderPtr enc = conn->encoder_id ? drmModeGetEncoder(fd, conn->encoder_id) : NULL;
    if (!enc && conn->count_encoders) enc = drmModeGetEncoder(fd, conn->encoders[0]);
    crtc_idx = 0;
    if (enc && enc->crtc_id) {
        for (int i = 0; i < res->count_crtcs; i++) if (res->crtcs[i] == enc->crtc_id) crtc_idx = i;
    } else if (enc) {
        for (int i = 0; i < res->count_crtcs; i++) if (enc->possible_crtcs & (1u << i)) { crtc_idx = i; break; }
    }
    crtc_id = res->crtcs[crtc_idx];
    if (mode.htotal && mode.vtotal) frame_us = 1e9 / ((double)mode.clock * 1000.0 / mode.htotal / mode.vtotal) / 1e3;
    printf("KFPLANES_SETUP card=%s connector=%u crtc=%u crtc_index=%u mode=%ux%u@%u clock=%u frame_us=%.0f crtcs=%d\n",
           card, conn_id, crtc_id, crtc_idx, mode.hdisplay, mode.vdisplay, mode.vrefresh, mode.clock, frame_us,
           res->count_crtcs);
    if (verbose) {
        drmModePropertyBlobPtr edid = NULL;
        uint64_t eid = 0;
        if (prop_lookup(conn_id, DRM_MODE_OBJECT_CONNECTOR, "EDID", &eid) && eid) edid = drmModeGetPropertyBlob(fd, eid);
        if (edid) {
            uint8_t sum = 0; const uint8_t *e = edid->data;
            for (uint32_t i = 0; i < 128 && i < edid->length; i++) sum += e[i];
            printf("KFPLANES_EDID bytes=%u header_ok=%d checksum_ok=%d ext=%u\n", edid->length,
                   edid->length >= 8 && e[0] == 0 && e[1] == 0xFF && e[7] == 0, sum == 0,
                   edid->length >= 127 ? e[126] : 0);
            drmModeFreePropertyBlob(edid);
        } else printf("KFPLANES_EDID none\n");
        for (int i = 0; i < conn->count_modes && i < 6; i++)
            printf("KFPLANES_MODE %s %u Hz clock %u%s\n", conn->modes[i].name, conn->modes[i].vrefresh,
                   conn->modes[i].clock, conn->modes[i].type & DRM_MODE_TYPE_PREFERRED ? " preferred" : "");
    }
    c_active = prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "ACTIVE", NULL);
    c_mode = prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "MODE_ID", NULL);
    c_gamma = prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "GAMMA_LUT", NULL);
    c_degamma = prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "DEGAMMA_LUT", NULL);
    c_ctm = prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "CTM", NULL);
    c_outfence = prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "OUT_FENCE_PTR", NULL);
    c_vrr = prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "VRR_ENABLED", NULL);
    uint64_t gs = 0, ds = 0;
    prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "GAMMA_LUT_SIZE", &gs);
    prop_lookup(crtc_id, DRM_MODE_OBJECT_CRTC, "DEGAMMA_LUT_SIZE", &ds);
    c_gamma_size = (uint32_t)gs; c_degamma_size = (uint32_t)ds;
    k_crtc = prop_lookup(conn_id, DRM_MODE_OBJECT_CONNECTOR, "CRTC_ID", NULL);
    printf("KFPLANES_CRTC_PROPS active=%u mode=%u gamma=%u(size %u) degamma=%u(size %u) ctm=%u out_fence=%u vrr=%u\n",
           c_active, c_mode, c_gamma, c_gamma_size, c_degamma, c_degamma_size, c_ctm, c_outfence, c_vrr);
    if (verbose) {
        static const char *cprops[] = { "HDR_OUTPUT_METADATA", "Colorspace", "max bpc", "content type",
                                        "link-status", "non-desktop", "vrr_capable" };
        for (unsigned i = 0; i < sizeof cprops / sizeof *cprops; i++) {
            uint64_t v = 0;
            uint32_t id = prop_lookup(conn_id, DRM_MODE_OBJECT_CONNECTOR, cprops[i], &v);
            printf("KFPLANES_CONNECTOR_PROP name=\"%s\" id=%u value=%" PRIu64 "\n", cprops[i], id, v);
        }
    }
    drmModePlaneResPtr pr = drmModeGetPlaneResources(fd);
    for (uint32_t i = 0; pr && i < pr->count_planes && nplanes < 32; i++) {
        drmModePlanePtr p = drmModeGetPlane(fd, pr->planes[i]);
        if (!p) continue;
        struct plane *q = &planes[nplanes++];
        q->id = p->plane_id; q->possible = p->possible_crtcs; q->nfmt = p->count_formats;
        q->fmts = calloc(p->count_formats + 1, 4);
        memcpy(q->fmts, p->formats, 4 * p->count_formats);
        uint64_t t = 0;
        prop_lookup(q->id, DRM_MODE_OBJECT_PLANE, "type", &t);
        q->type = (uint32_t)t;
        for (int k = 0; k < P_N; k++) q->prop[k] = prop_lookup(q->id, DRM_MODE_OBJECT_PLANE, PNAMES[k], NULL);
        int mine = (p->possible_crtcs >> crtc_idx) & 1;
        if (verbose) {
            printf("KFPLANES_PLANE id=%u type=%s possible=0x%x mine=%d formats=%u:", q->id, ptype(q->type),
                   q->possible, mine, q->nfmt);
            for (uint32_t f = 0; f < q->nfmt; f++) printf(" %.4s", (char *)&q->fmts[f]);
            printf(" props:");
            for (int k = 0; k < P_N; k++) if (q->prop[k]) printf(" [%s]", PNAMES[k]);
            printf("\n");
        }
        if (mine) {
            if (q->type == DRM_PLANE_TYPE_PRIMARY && !primary) primary = q;
            else if (q->type == DRM_PLANE_TYPE_OVERLAY && noverlay < 8) overlay[noverlay++] = q;
            else if (q->type == DRM_PLANE_TYPE_CURSOR && !cursor) cursor = q;
        }
        drmModeFreePlane(p);
    }
    printf("KFPLANES_SUMMARY planes=%d primary=%u overlays=%d cursor=%u ovl_nv12=%d ovl_p010=%d ovl_yuyv=%d ovl_argb=%d\n",
           nplanes, primary ? primary->id : 0, noverlay, cursor ? cursor->id : 0,
           noverlay && has_fmt(overlay[0], DRM_FORMAT_NV12), noverlay && has_fmt(overlay[0], DRM_FORMAT_P010),
           noverlay && has_fmt(overlay[0], DRM_FORMAT_YUYV), noverlay && has_fmt(overlay[0], DRM_FORMAT_ARGB8888));
    if (drmModeCreatePropertyBlob(fd, &mode, sizeof mode, &mode_blob)) { printf("KFPLANES_FAIL=mode-blob\n"); return -1; }
    return primary ? 0 : -1;
}

/* ---------------------------------------------------------------- buffers */

struct buf { uint32_t handle, fb, w, h, fmt; uint32_t pitches[4], offsets[4]; uint64_t size; uint8_t *map; };

static int dumb(struct buf *b, uint32_t w, uint32_t h, uint32_t bpp)
{
    struct drm_mode_create_dumb cr = { .width = w, .height = h, .bpp = bpp };
    if (drmIoctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &cr)) return -errno;
    struct drm_mode_map_dumb mp = { .handle = cr.handle };
    if (drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mp)) return -errno;
    b->map = mmap(NULL, cr.size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, mp.offset);
    if (b->map == MAP_FAILED) return -errno;
    b->handle = cr.handle; b->size = cr.size; b->pitches[0] = cr.pitch;
    return 0;
}

/* variant: 0 = A, 1 = B. what: the overlay's content. */
static int make_buf(struct buf *b, uint32_t w, uint32_t h, uint32_t fmt, int variant, int is_primary)
{
    memset(b, 0, sizeof *b);
    b->w = w; b->h = h; b->fmt = fmt;
    uint32_t handles[4] = { 0 };
    int r;
    if (fmt == DRM_FORMAT_XRGB8888 || fmt == DRM_FORMAT_ARGB8888) {
        if ((r = dumb(b, w, h, 32))) return r;
        for (uint32_t y = 0; y < h; y++) {
            uint32_t *row = (uint32_t *)(b->map + (size_t)y * b->pitches[0]);
            for (uint32_t x = 0; x < w; x++)
                row[x] = is_primary ? prim_px(x, y, w, h, variant) : ovl_rgb_px(x, y, w, h, variant);
        }
        handles[0] = b->handle;
    } else if (fmt == DRM_FORMAT_NV12 || fmt == DRM_FORMAT_P010) {
        int wide = fmt == DRM_FORMAT_P010;
        if ((r = dumb(b, w, h + h / 2, wide ? 16 : 8))) return r;
        uint32_t p = b->pitches[0];
        b->pitches[1] = p; b->offsets[1] = p * h;
        for (uint32_t y = 0; y < h; y++)
            for (uint32_t x = 0; x < w; x++) {
                int bar = bar_of(variant ? w - 1 - x : x, w);
                if (wide) ((uint16_t *)(b->map + (size_t)y * p))[x] = (uint16_t)(BAR_Y[bar] << 8);
                else b->map[(size_t)y * p + x] = BAR_Y[bar];
            }
        for (uint32_t y = 0; y < h / 2; y++)
            for (uint32_t x = 0; x < w / 2; x++) {
                int bar = bar_of(variant ? w - 1 - 2 * x : 2 * x, w);
                if (wide) {
                    uint16_t *c = (uint16_t *)(b->map + b->offsets[1] + (size_t)y * p);
                    c[2 * x] = (uint16_t)(BAR_U[bar] << 8); c[2 * x + 1] = (uint16_t)(BAR_V[bar] << 8);
                } else {
                    uint8_t *c = b->map + b->offsets[1] + (size_t)y * p;
                    c[2 * x] = BAR_U[bar]; c[2 * x + 1] = BAR_V[bar];
                }
            }
        handles[0] = handles[1] = b->handle;
    } else if (fmt == DRM_FORMAT_YUYV) {
        if ((r = dumb(b, w, h, 16))) return r;
        for (uint32_t y = 0; y < h; y++)
            for (uint32_t x = 0; x < w; x += 2) {
                int bar = bar_of(variant ? w - 1 - x : x, w);
                uint8_t *px = b->map + (size_t)y * b->pitches[0] + 2 * x;
                px[0] = BAR_Y[bar]; px[1] = BAR_U[bar]; px[2] = BAR_Y[bar]; px[3] = BAR_V[bar];
            }
        handles[0] = b->handle;
    } else return -EINVAL;
    if (drmModeAddFB2(fd, w, h, fmt, handles, b->pitches, b->offsets, &b->fb, 0)) return -errno;
    return 0;
}

static void free_buf(struct buf *b)
{
    if (b->fb) drmModeRmFB(fd, b->fb);
    if (b->map && b->map != MAP_FAILED) munmap(b->map, b->size);
    if (b->handle) { struct drm_mode_destroy_dumb d = { .handle = b->handle }; drmIoctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &d); }
    memset(b, 0, sizeof *b);
}

/* ---------------------------------------------------------------- atomic helpers and events */

static void add(drmModeAtomicReq *r, uint32_t obj, uint32_t prop, uint64_t v) { if (prop) drmModeAtomicAddProperty(r, obj, prop, v); }

static void plane_on(drmModeAtomicReq *r, const struct plane *p, uint32_t fb, uint32_t sw, uint32_t sh,
                     uint32_t cx, uint32_t cy, uint32_t cw, uint32_t ch)
{
    add(r, p->id, p->prop[P_FB], fb); add(r, p->id, p->prop[P_CRTC], crtc_id);
    add(r, p->id, p->prop[P_SX], 0); add(r, p->id, p->prop[P_SY], 0);
    add(r, p->id, p->prop[P_SW], (uint64_t)sw << 16); add(r, p->id, p->prop[P_SH], (uint64_t)sh << 16);
    add(r, p->id, p->prop[P_CX], cx); add(r, p->id, p->prop[P_CY], cy);
    add(r, p->id, p->prop[P_CW], cw); add(r, p->id, p->prop[P_CH], ch);
}
static void plane_off(drmModeAtomicReq *r, const struct plane *p)
{
    add(r, p->id, p->prop[P_FB], 0); add(r, p->id, p->prop[P_CRTC], 0);
}

static int ev_count, vbl_count;
static unsigned ev_seq, vbl_seq;
static double ev_tv_us, ev_recv_us, vbl_tv_us;
static void on_flip2(int f, unsigned seq, unsigned s, unsigned us, unsigned crtc, void *d)
{
    (void)f; (void)crtc; (void)d;
    ev_count++; ev_seq = seq; ev_tv_us = s * 1e6 + us; ev_recv_us = now_us();
}
static void on_vbl(int f, unsigned seq, unsigned s, unsigned us, void *d)
{
    (void)f; (void)d; vbl_count++; vbl_seq = seq; vbl_tv_us = s * 1e6 + us;
}
static drmEventContext evctx = { .version = 3, .vblank_handler = on_vbl, .page_flip_handler2 = on_flip2 };

/* wait until ev_count > c0 or the deadline (us from now) passes */
static int wait_ev(int c0, double budget_us)
{
    double end = now_us() + budget_us;
    while (ev_count <= c0) {
        double left = end - now_us();
        if (left <= 0) return 0;
        struct pollfd p = { .fd = fd, .events = POLLIN };
        int r = poll(&p, 1, (int)(left / 1000) + 1);
        if (r > 0) drmHandleEvent(fd, &evctx);
    }
    return 1;
}

static int commit(drmModeAtomicReq *r, uint32_t flags)
{
    int ret = drmModeAtomicCommit(fd, r, flags, NULL);
    int e = errno;
    drmModeAtomicFree(r);
    return ret ? -e : 0;
}

struct stats {
    const char *name; int n, events, timeouts, errors, ebusy, seq0, seq1, seq2, seqn, lat_n, last_err;
    double lat_sum, lat_max, lat[4096]; int over1, over2, have_seq; unsigned last_seq; int stuck;
};

static int cmpd(const void *a, const void *b) { double x = *(const double *)a, y = *(const double *)b; return x < y ? -1 : x > y; }

static void report(struct stats *s, const char *skip)
{
    if (skip) {
        printf("KFPLANES_RESULT test=%s verdict=SKIP why=\"%s\"\n", s->name, skip);
        return;
    }
    double p95 = 0;
    if (s->lat_n) { qsort(s->lat, s->lat_n, sizeof(double), cmpd); p95 = s->lat[(int)(s->lat_n * 0.95) < s->lat_n ? (int)(s->lat_n * 0.95) : s->lat_n - 1]; }
    const char *v = "PASS";
    if (s->timeouts || s->errors || s->stuck || s->events < s->n) v = "FAIL";
    /* the falsifier stated before the runs: SLOW when more than 5% of the events come later than
     * one frame + 2 ms, any later than two frames, or vblank steps above one (when counted) */
    else if (s->over2 || s->over1 * 20 > s->events || s->seq2 || s->seqn) v = "SLOW";
    printf("KFPLANES_RESULT test=%s commits=%d events=%d timeouts=%d errors=%d last_err=%d ebusy=%d lat_avg_us=%.0f "
           "lat_p95_us=%.0f lat_max_us=%.0f over1f=%d over2f=%d seq0=%d seq1=%d seq2=%d seqN=%d frame_us=%.0f verdict=%s\n",
           s->name, s->n, s->events, s->timeouts, s->errors, s->last_err, s->ebusy, s->lat_n ? s->lat_sum / s->lat_n : 0,
           p95, s->lat_max, s->over1, s->over2, s->seq0, s->seq1, s->seq2, s->seqn, frame_us, v);
    fflush(stdout);
}

typedef void (*build_fn)(drmModeAtomicReq *r, int i, void *ctx);

/* recover from a lost event: a BLOCKING commit with nothing in it drains nvidia-drm's flip list
 * (bounded at 3 s by nvidia-drm itself) */
static void recover(void)
{
    drmModeAtomicReq *r = drmModeAtomicAlloc();
    add(r, crtc_id, c_active, 1);
    commit(r, 0);
    wait_ev(ev_count, 50000);
}

static void series(struct stats *s, int n, build_fn b, void *ctx, uint32_t flags)
{
    s->n = 0;
    for (int i = 0; i < n && i < 4096; i++) {
        drmModeAtomicReq *r = drmModeAtomicAlloc();
        b(r, i, ctx);
        int c0 = ev_count;
        double t0 = now_us();
        int ret = commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT | flags);
        if (ret == -EBUSY) {
            s->ebusy++;
            wait_ev(c0, 3 * frame_us);
            c0 = ev_count;
            r = drmModeAtomicAlloc(); b(r, i, ctx); t0 = now_us();
            ret = commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT | flags);
        }
        s->n++;
        if (ret) { s->errors++; s->last_err = ret; continue; }
        if (!wait_ev(c0, 3 * frame_us + 200000)) {
            s->timeouts++;
            printf("KFPLANES_TIMEOUT test=%s i=%d waited_us=%.0f\n", s->name, i, now_us() - t0);
            if (!wait_ev(c0, 3500000)) { s->stuck++; recover(); }
            if (s->timeouts > 3) break;
            continue;
        }
        s->events++;
        double lat = ev_recv_us - t0;
        s->lat[s->lat_n++] = lat; s->lat_sum += lat; if (lat > s->lat_max) s->lat_max = lat;
        if (lat > frame_us + 2000) s->over1++;
        if (lat > 2 * frame_us + 2000) s->over2++;
        if (s->have_seq) {
            unsigned d = ev_seq - s->last_seq;
            if (d == 0) s->seq0++; else if (d == 1) s->seq1++; else if (d == 2) s->seq2++; else s->seqn++;
        }
        s->have_seq = 1; s->last_seq = ev_seq;
    }
}

/* ---------------------------------------------------------------- the tests */

static struct buf pa, pb;          /* primary A/B (full mode size) */
static uint32_t W, H;

static int modeset_primary(void)
{
    drmModeAtomicReq *r = drmModeAtomicAlloc();
    add(r, conn_id, k_crtc, crtc_id);
    add(r, crtc_id, c_mode, mode_blob);
    add(r, crtc_id, c_active, 1);
    plane_on(r, primary, pa.fb, W, H, 0, 0, W, H);
    for (int i = 0; i < noverlay; i++) plane_off(r, overlay[i]);
    if (cursor) plane_off(r, cursor);
    int ret = commit(r, DRM_MODE_ATOMIC_ALLOW_MODESET);
    printf("KFPLANES_MODESET rc=%d\n", ret);
    return ret;
}

struct ctx { struct buf *a, *b; const struct plane *p; uint32_t sw, sh; struct rect d; int toggle, both; uint32_t blob[2]; uint32_t prop; };

static void b_primary(drmModeAtomicReq *r, int i, void *c)
{
    (void)c; add(r, primary->id, primary->prop[P_FB], (i & 1) ? pa.fb : pb.fb);
}
static void b_overlay(drmModeAtomicReq *r, int i, void *v)
{
    struct ctx *c = v;
    struct buf *b = (i & 1) ? c->a : c->b;
    if (c->toggle && (i & 1)) plane_off(r, c->p);
    else plane_on(r, c->p, b->fb, c->sw, c->sh, c->d.x, c->d.y, c->d.w, c->d.h);
    if (c->both) add(r, primary->id, primary->prop[P_FB], (i & 1) ? pa.fb : pb.fb);
}
static void b_cursor(drmModeAtomicReq *r, int i, void *v)
{
    struct ctx *c = v;
    plane_on(r, cursor, c->a->fb, 64, 64, 100 + (i * 7) % 600, 100 + (i * 5) % 400, 64, 64);
    add(r, primary->id, primary->prop[P_FB], (i & 1) ? pa.fb : pb.fb);
}
static void b_blobprop(drmModeAtomicReq *r, int i, void *v)
{
    struct ctx *c = v;
    add(r, crtc_id, c->prop, c->blob[i & 1]);
    add(r, primary->id, primary->prop[P_FB], (i & 1) ? pa.fb : pb.fb);
}

static uint32_t lut_blob(uint32_t size, int invert)
{
    struct drm_color_lut *l = calloc(size, sizeof *l);
    for (uint32_t i = 0; i < size; i++) {
        uint16_t v = (uint16_t)((uint64_t)i * 0xFFFF / (size > 1 ? size - 1 : 1));
        if (invert) v = 0xFFFF - v;
        l[i].red = l[i].green = l[i].blue = v;
    }
    uint32_t id = 0;
    drmModeCreatePropertyBlob(fd, l, size * sizeof *l, &id);
    free(l);
    return id;
}
static uint32_t ctm_blob(int swap)
{
    struct drm_color_ctm m;
    memset(&m, 0, sizeof m);
    /* S31.32 sign-magnitude; identity or a red<->blue swap */
    const uint64_t one = 1ull << 32;
    if (!swap) { m.matrix[0] = one; m.matrix[4] = one; m.matrix[8] = one; }
    else { m.matrix[2] = one; m.matrix[4] = one; m.matrix[6] = one; }
    uint32_t id = 0;
    drmModeCreatePropertyBlob(fd, &m, sizeof m, &id);
    return id;
}

static void test_vblank(int n)
{
    struct stats s = { .name = "vblank-wait" };
    double prev = 0, sum = 0, mx = 0, mn = 1e12;
    int got = 0;
    unsigned hi = crtc_idx > 1 ? (crtc_idx << DRM_VBLANK_HIGH_CRTC_SHIFT) & DRM_VBLANK_HIGH_CRTC_MASK : (crtc_idx == 1 ? DRM_VBLANK_SECONDARY : 0);
    for (int i = 0; i < n; i++) {
        drmVBlank vb = { .request = { .type = DRM_VBLANK_RELATIVE | hi, .sequence = 1 } };
        if (drmWaitVBlank(fd, &vb)) { s.errors++; s.last_err = -errno; if (s.errors > 3) break; continue; }
        double t = vb.reply.tval_sec * 1e6 + vb.reply.tval_usec;
        if (prev) { double d = t - prev; sum += d; if (d > mx) mx = d; if (d < mn) mn = d; got++; }
        prev = t;
    }
    printf("KFPLANES_RESULT test=vblank-wait waits=%d errors=%d last_err=%d interval_avg_us=%.0f min_us=%.0f max_us=%.0f frame_us=%.0f verdict=%s\n",
           n, s.errors, s.last_err, got ? sum / got : 0, got ? mn : 0, mx, frame_us,
           s.errors ? "FAIL" : (got && fabs(sum / got - frame_us) < 0.05 * frame_us && mx < 2 * frame_us) ? "PASS" : "SLOW");
    /* vblank EVENTS (DRM_VBLANK_EVENT) */
    int c0 = vbl_count, sent = 0;
    for (int i = 0; i < 10; i++) {
        drmVBlank vb = { .request = { .type = DRM_VBLANK_RELATIVE | DRM_VBLANK_EVENT | hi, .sequence = 1 } };
        if (!drmWaitVBlank(fd, &vb)) sent++;
        double end = now_us() + 3 * frame_us + 100000;
        while (vbl_count - c0 < sent && now_us() < end) {
            struct pollfd p = { .fd = fd, .events = POLLIN };
            if (poll(&p, 1, 20) > 0) drmHandleEvent(fd, &evctx);
        }
    }
    printf("KFPLANES_RESULT test=vblank-event requested=%d received=%d verdict=%s\n", sent, vbl_count - c0,
           sent && vbl_count - c0 == sent ? "PASS" : "FAIL");
}

static void test_overlay_fmt(const char *name, uint32_t fmt, int n, uint32_t sw, uint32_t sh, int scaled, int toggle, int both)
{
    struct stats s = { .name = name };
    if (!noverlay) { report(&s, "no overlay plane on this CRTC"); return; }
    const struct plane *p = overlay[0];
    if (!has_fmt(p, fmt)) { report(&s, "overlay plane does not list the format"); return; }
    struct buf a, b;
    int ra = make_buf(&a, sw, sh, fmt, 0, 0), rb = make_buf(&b, sw, sh, fmt, 1, 0);
    if (ra || rb) {
        char why[96]; snprintf(why, sizeof why, "fb create failed %d/%d", ra, rb);
        report(&s, why); free_buf(&a); free_buf(&b); return;
    }
    struct ctx c = { .a = &a, .b = &b, .p = p, .sw = sw, .sh = sh, .d = ovl_dst(W, H, scaled), .toggle = toggle, .both = both };
    /* enable the overlay first: a commit whose planes had no fb before generates no hardware event */
    drmModeAtomicReq *r = drmModeAtomicAlloc();
    plane_on(r, p, b.fb, sw, sh, c.d.x, c.d.y, c.d.w, c.d.h);
    int c0 = ev_count;
    int en = commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT);
    int en_ev = en ? 0 : wait_ev(c0, 3 * frame_us + 200000);
    printf("KFPLANES_ENABLE test=%s rc=%d event=%d\n", name, en, en_ev);
    if (en) { char why[64]; snprintf(why, sizeof why, "enable commit refused rc=%d", en); report(&s, why); }
    else { series(&s, n, b_overlay, &c, 0); report(&s, NULL); }
    /* disable the overlay (a commit that disables a plane that had a fb: nvidia-drm counts one event
     * for it — the issue-#1361 shape; measured separately, never graded by the series above) */
    r = drmModeAtomicAlloc();
    plane_off(r, p);
    c0 = ev_count;
    double t0 = now_us();
    int dr = commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT);
    int dev = dr ? 0 : wait_ev(c0, 3 * frame_us + 200000);
    printf("KFPLANES_DISABLE test=%s rc=%d event=%d lat_us=%.0f\n", name, dr, dev, dev ? ev_recv_us - t0 : -1.0);
    if (!dr && !dev) { wait_ev(c0, 3500000); recover(); }
    free_buf(&a); free_buf(&b);
}

/* the MPO life cycle a desktop compositor drives (and Windows' plane 1 around presents 5212-5215):
 * enable the overlay, flip BOTH planes `flips` times, disable it — `cycles` times. Every flip is
 * graded; the disables are counted apart (the issue-#1361 shape: may carry no event). */
static void test_overlay_cycle(const char *name, uint32_t fmt, int cycles, int flips)
{
    struct stats s = { .name = name };
    if (!noverlay || !has_fmt(overlay[0], fmt)) { report(&s, "no overlay plane with the format"); return; }
    const struct plane *p = overlay[0];
    struct buf a, b;
    if (make_buf(&a, OVL_SRC_W, OVL_SRC_H, fmt, 0, 0) || make_buf(&b, OVL_SRC_W, OVL_SRC_H, fmt, 1, 0)) {
        report(&s, "fb create failed"); free_buf(&a); free_buf(&b); return;
    }
    struct ctx c = { .a = &a, .b = &b, .p = p, .sw = OVL_SRC_W, .sh = OVL_SRC_H, .d = ovl_dst(W, H, 0), .both = 1 };
    int dis_events = 0, en_fail = 0;
    for (int k = 0; k < cycles && !s.stuck; k++) {
        drmModeAtomicReq *r = drmModeAtomicAlloc();
        plane_on(r, p, b.fb, OVL_SRC_W, OVL_SRC_H, c.d.x, c.d.y, c.d.w, c.d.h);
        int c0 = ev_count;
        if (commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT) || !wait_ev(c0, 3 * frame_us + 200000)) en_fail++;
        struct stats one = { .name = name };
        series(&one, flips, b_overlay, &c, 0);
        s.n += one.n; s.events += one.events; s.timeouts += one.timeouts; s.errors += one.errors; s.stuck += one.stuck;
        s.over1 += one.over1; s.over2 += one.over2;
        for (int i = 0; i < one.lat_n && s.lat_n < 4096; i++) { s.lat[s.lat_n++] = one.lat[i]; s.lat_sum += one.lat[i]; if (one.lat[i] > s.lat_max) s.lat_max = one.lat[i]; }
        r = drmModeAtomicAlloc();
        plane_off(r, p);
        add(r, primary->id, primary->prop[P_FB], pa.fb);
        c0 = ev_count;
        if (!commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT)) {
            if (wait_ev(c0, 3 * frame_us + 200000)) dis_events++;
            else { wait_ev(c0, 3500000); recover(); }
        }
    }
    printf("KFPLANES_CYCLE test=%s cycles=%d enable_failures=%d disable_events=%d\n", name, cycles, en_fail, dis_events);
    report(&s, NULL);
    free_buf(&a); free_buf(&b);
}

static void test_blob(const char *name, uint32_t prop, uint32_t b0, uint32_t b1, int n)
{
    struct stats s = { .name = name };
    if (!prop || !b0 || !b1) { report(&s, "property absent"); return; }
    struct ctx c = { .prop = prop, .blob = { b0, b1 } };
    series(&s, n, b_blobprop, &c, 0);
    report(&s, NULL);
    /* restore: no LUT/CTM */
    drmModeAtomicReq *r = drmModeAtomicAlloc();
    add(r, crtc_id, prop, 0);
    add(r, primary->id, primary->prop[P_FB], pa.fb);
    int c0 = ev_count;
    if (!commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT)) wait_ev(c0, 3 * frame_us + 200000);
}

static void test_async(int n)
{
    struct stats s = { .name = "primary-async-legacy" };
    s.n = 0;
    for (int i = 0; i < n; i++) {
        int c0 = ev_count;
        double t0 = now_us();
        int ret = drmModePageFlip(fd, crtc_id, (i & 1) ? pa.fb : pb.fb, DRM_MODE_PAGE_FLIP_EVENT | DRM_MODE_PAGE_FLIP_ASYNC, NULL);
        s.n++;
        if (ret) { s.errors++; s.last_err = -errno; if (s.errors > 3) break; continue; }
        if (!wait_ev(c0, 3 * frame_us + 200000)) { s.timeouts++; if (!wait_ev(c0, 3500000)) { s.stuck++; recover(); } if (s.timeouts > 3) break; continue; }
        s.events++;
        double lat = ev_recv_us - t0;
        s.lat[s.lat_n++] = lat; s.lat_sum += lat; if (lat > s.lat_max) s.lat_max = lat;
        if (lat > frame_us + 2000) s.over1++;
        if (lat > 2 * frame_us + 2000) s.over2++;
    }
    if (s.errors && !s.events) {
        char why[64]; snprintf(why, sizeof why, "async flip refused rc=%d (driver may not offer async)", s.last_err);
        report(&s, why);
    } else report(&s, NULL);
    /* the atomic flavour */
    struct stats a = { .name = "primary-async-atomic" };
    series(&a, n, b_primary, NULL, DRM_MODE_PAGE_FLIP_ASYNC);
    if (a.errors && !a.events) { char why[64]; snprintf(why, sizeof why, "atomic async refused rc=%d", a.last_err); report(&a, why); }
    else report(&a, NULL);
}

static void test_dpms(int cycles)
{
    int ok = 0;
    double off_max = 0, on_max = 0;
    for (int i = 0; i < cycles; i++) {
        drmModeAtomicReq *r = drmModeAtomicAlloc();
        add(r, crtc_id, c_active, 0);
        double t0 = now_us();
        int a = commit(r, DRM_MODE_ATOMIC_ALLOW_MODESET);
        double t1 = now_us();
        r = drmModeAtomicAlloc();
        add(r, crtc_id, c_active, 1);
        add(r, crtc_id, c_mode, mode_blob);
        plane_on(r, primary, pa.fb, W, H, 0, 0, W, H);
        int b = commit(r, DRM_MODE_ATOMIC_ALLOW_MODESET);
        double t2 = now_us();
        if (!a && !b) ok++;
        if (t1 - t0 > off_max) off_max = t1 - t0;
        if (t2 - t1 > on_max) on_max = t2 - t1;
        printf("KFPLANES_DPMS cycle=%d off_rc=%d on_rc=%d off_us=%.0f on_us=%.0f\n", i, a, b, t1 - t0, t2 - t1);
    }
    /* a flip still completes after the cycles */
    struct stats s = { .name = "flip-after-dpms" };
    series(&s, 30, b_primary, NULL, 0);
    printf("KFPLANES_RESULT test=dpms-cycle cycles=%d ok=%d off_max_us=%.0f on_max_us=%.0f verdict=%s\n", cycles, ok,
           off_max, on_max, ok == cycles && off_max < 3e6 && on_max < 3e6 ? "PASS" : "FAIL");
    report(&s, NULL);
}

static void test_outfence(int n)
{
    struct stats s = { .name = "out-fence" };
    if (!c_outfence) { report(&s, "OUT_FENCE_PTR absent"); return; }
    int signalled = 0, errs = 0, late = 0;
    double mx = 0;
    for (int i = 0; i < n; i++) {
        int32_t ofd = -1;
        drmModeAtomicReq *r = drmModeAtomicAlloc();
        add(r, crtc_id, c_outfence, (uint64_t)(uintptr_t)&ofd);
        add(r, primary->id, primary->prop[P_FB], (i & 1) ? pa.fb : pb.fb);
        int c0 = ev_count;
        double t0 = now_us();
        int ret = commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT);
        if (ret || ofd < 0) { errs++; s.last_err = ret; continue; }
        struct pollfd p = { .fd = ofd, .events = POLLIN };
        int pr = poll(&p, 1, (int)(3 * frame_us / 1000) + 200);
        double lat = now_us() - t0;
        if (pr > 0) { signalled++; if (lat > mx) mx = lat; if (lat > frame_us + 2000) late++; }
        close(ofd);
        wait_ev(c0, 3 * frame_us + 200000);
    }
    printf("KFPLANES_RESULT test=out-fence commits=%d signalled=%d errors=%d last_err=%d late1f=%d lat_max_us=%.0f frame_us=%.0f verdict=%s\n",
           n, signalled, errs, s.last_err, late, mx, frame_us, errs ? "FAIL" : signalled == n ? (late ? "SLOW" : "PASS") : "FAIL");
}

/* explicit sync: IN_FENCE_FD from a sw_sync timeline (debugfs), signalled 40 ms after the commit */
struct sw_sync_create_fence_data { uint32_t value; char name[32]; int32_t fence; };
#define SW_SYNC_IOC_CREATE_FENCE _IOWR('W', 0, struct sw_sync_create_fence_data)
#define SW_SYNC_IOC_INC _IOW('W', 1, uint32_t)
static void test_infence(int n)
{
    struct stats s = { .name = "in-fence" };
    if (!primary->prop[P_INFENCE]) { report(&s, "IN_FENCE_FD absent"); return; }
    int tl = open("/sys/kernel/debug/sync/sw_sync", O_RDWR);
    if (tl < 0) { report(&s, "no sw_sync (debugfs/CONFIG_SW_SYNC)"); return; }
    int early = 0, ok = 0, errs = 0, tmo = 0;
    double mx = 0;
    for (int i = 0; i < n; i++) {
        struct sw_sync_create_fence_data d = { .value = (uint32_t)i + 1 };
        snprintf(d.name, sizeof d.name, "kfplanes%d", i);
        if (ioctl(tl, SW_SYNC_IOC_CREATE_FENCE, &d)) { errs++; break; }
        drmModeAtomicReq *r = drmModeAtomicAlloc();
        add(r, primary->id, primary->prop[P_FB], (i & 1) ? pa.fb : pb.fb);
        add(r, primary->id, primary->prop[P_INFENCE], (uint64_t)d.fence);
        int c0 = ev_count;
        int ret = commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT);
        close(d.fence);
        if (ret) { errs++; s.last_err = ret; uint32_t one = 1; ioctl(tl, SW_SYNC_IOC_INC, &one); continue; }
        if (wait_ev(c0, 40000)) early++;               /* completed before its fence signalled */
        double ts = now_us();
        uint32_t one = 1;
        ioctl(tl, SW_SYNC_IOC_INC, &one);
        if (ev_count > c0 || wait_ev(c0, 3 * frame_us + 200000)) {
            ok++; double lat = ev_recv_us - ts; if (lat > mx) mx = lat;
        } else { tmo++; if (!wait_ev(c0, 3500000)) recover(); }
    }
    close(tl);
    printf("KFPLANES_RESULT test=in-fence commits=%d events=%d early=%d timeouts=%d errors=%d last_err=%d after_signal_max_us=%.0f frame_us=%.0f verdict=%s\n",
           n, ok, early, tmo, errs, s.last_err, mx, frame_us,
           errs || tmo || early ? "FAIL" : mx > frame_us + 2000 ? "SLOW" : "PASS");
}

static int run_all(int n)
{
    W = mode.hdisplay; H = mode.vdisplay;
    if (make_buf(&pa, W, H, DRM_FORMAT_XRGB8888, 0, 1) || make_buf(&pb, W, H, DRM_FORMAT_XRGB8888, 1, 1)) {
        printf("KFPLANES_FAIL=primary-fb %s\n", strerror(errno)); return 1;
    }
    if (modeset_primary()) return 1;
    wait_ev(ev_count, 100000);
    test_vblank(n);
    { struct stats s = { .name = "primary-flip" }; series(&s, n, b_primary, NULL, 0); report(&s, NULL); }
    /* overlay, RGB: plane-only flips (the Windows plane-1 shape), then both planes per commit */
    test_overlay_fmt("overlay-argb-only", DRM_FORMAT_ARGB8888, n, OVL_SRC_W, OVL_SRC_H, 0, 0, 0);
    test_overlay_fmt("overlay-argb-both", DRM_FORMAT_ARGB8888, n, OVL_SRC_W, OVL_SRC_H, 0, 0, 1);
    test_overlay_fmt("overlay-xrgb-only", DRM_FORMAT_XRGB8888, n, OVL_SRC_W, OVL_SRC_H, 0, 0, 0);
    /* overlay, YUV */
    test_overlay_fmt("overlay-nv12-only", DRM_FORMAT_NV12, n, OVL_SRC_W, OVL_SRC_H, 0, 0, 0);
    test_overlay_fmt("overlay-nv12-both", DRM_FORMAT_NV12, n, OVL_SRC_W, OVL_SRC_H, 0, 0, 1);
    test_overlay_fmt("overlay-p010-only", DRM_FORMAT_P010, n, OVL_SRC_W, OVL_SRC_H, 0, 0, 0);
    test_overlay_fmt("overlay-yuyv-only", DRM_FORMAT_YUYV, n, OVL_SRC_W, OVL_SRC_H, 0, 0, 0);
    /* scaling: 640x360 shown at 960x540 (up) */
    test_overlay_fmt("overlay-nv12-scaled", DRM_FORMAT_NV12, n, OVL_SRC_W, OVL_SRC_H, 1, 0, 0);
    test_overlay_fmt("overlay-argb-scaled", DRM_FORMAT_ARGB8888, n, OVL_SRC_W, OVL_SRC_H, 1, 0, 0);
    /* enable/disable on alternate commits (Windows flipped plane 1 to NO surface and back) */
    test_overlay_fmt("overlay-nv12-toggle", DRM_FORMAT_NV12, n / 2, OVL_SRC_W, OVL_SRC_H, 0, 1, 1);
    test_overlay_fmt("overlay-argb-toggle", DRM_FORMAT_ARGB8888, n / 2, OVL_SRC_W, OVL_SRC_H, 0, 1, 1);
    /* the MPO life cycle: enable, flip both, disable */
    test_overlay_cycle("overlay-argb-cycle", DRM_FORMAT_ARGB8888, 8, 10);
    test_overlay_cycle("overlay-nv12-cycle", DRM_FORMAT_NV12, 8, 10);
    /* cursor plane (atomic) moving each flip */
    if (cursor && has_fmt(cursor, DRM_FORMAT_ARGB8888)) {
        struct buf cb; struct stats s = { .name = "cursor-atomic" };
        if (!make_buf(&cb, 64, 64, DRM_FORMAT_ARGB8888, 0, 0)) {
            struct ctx c = { .a = &cb };
            series(&s, n, b_cursor, &c, 0); report(&s, NULL);
            drmModeAtomicReq *r = drmModeAtomicAlloc(); plane_off(r, cursor); add(r, primary->id, primary->prop[P_FB], pa.fb);
            int c0 = ev_count; if (!commit(r, DRM_MODE_ATOMIC_NONBLOCK | DRM_MODE_PAGE_FLIP_EVENT)) wait_ev(c0, 3 * frame_us + 200000);
            free_buf(&cb);
        } else report(&s, "cursor fb create failed");
    } else { struct stats s = { .name = "cursor-atomic" }; report(&s, "no ARGB cursor plane"); }
    /* colour: LUTs and CTM, each with a flip */
    test_blob("gamma-lut", c_gamma, lut_blob(c_gamma_size ? c_gamma_size : 256, 0), lut_blob(c_gamma_size ? c_gamma_size : 256, 1), n / 2);
    test_blob("degamma-lut", c_degamma, lut_blob(c_degamma_size ? c_degamma_size : 256, 0), lut_blob(c_degamma_size ? c_degamma_size : 256, 1), n / 2);
    test_blob("ctm", c_ctm, ctm_blob(0), ctm_blob(1), n / 2);
    test_async(n);
    test_outfence(n / 2);
    test_infence(n / 4);
    test_dpms(5);
    printf("KFPLANES_DONE\n");
    return 0;
}

static int show(const char *what, int hold)
{
    W = mode.hdisplay; H = mode.vdisplay;
    if (make_buf(&pa, W, H, DRM_FORMAT_XRGB8888, 0, 1)) { printf("KFPLANES_FAIL=primary-fb\n"); return 1; }
    if (modeset_primary()) return 1;
    uint32_t fmt = !strcmp(what, "rgb") ? DRM_FORMAT_ARGB8888 : !strcmp(what, "p010") ? DRM_FORMAT_P010 :
                   !strcmp(what, "yuyv") ? DRM_FORMAT_YUYV : DRM_FORMAT_NV12;
    int scaled = !strcmp(what, "nv12-scaled");
    struct buf o;
    if (!noverlay || !has_fmt(overlay[0], fmt) || make_buf(&o, OVL_SRC_W, OVL_SRC_H, fmt, 0, 0)) {
        printf("KFPLANES_SHOW_FAIL what=%s\n", what); return 1;
    }
    struct rect d = ovl_dst(W, H, scaled);
    drmModeAtomicReq *r = drmModeAtomicAlloc();
    plane_on(r, overlay[0], o.fb, OVL_SRC_W, OVL_SRC_H, d.x, d.y, d.w, d.h);
    int rc = commit(r, 0);
    printf("KFPLANES_SHOWING what=%s rc=%d mode=%ux%u dst=%u,%u,%ux%u\n", what, rc, W, H, d.x, d.y, d.w, d.h);
    fflush(stdout);
    sleep(hold);
    r = drmModeAtomicAlloc();
    plane_off(r, overlay[0]);
    commit(r, 0);
    free_buf(&o);
    return rc ? 1 : 0;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    if (argc < 3) { fprintf(stderr, "usage: kfplanes list|all|show <card> ...\n"); return 2; }
    if (setup(argv[2], !strcmp(argv[1], "list") || !strcmp(argv[1], "all"))) return 1;
    if (!strcmp(argv[1], "list")) return 0;
    if (!strcmp(argv[1], "all")) return run_all(argc > 3 ? atoi(argv[3]) : 120);
    if (!strcmp(argv[1], "show") && argc > 4) return show(argv[3], atoi(argv[4]));
    fprintf(stderr, "unknown command\n");
    return 2;
}
#endif
