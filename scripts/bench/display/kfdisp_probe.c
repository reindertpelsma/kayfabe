/*
 * kfdisp_probe — the display lane's KMS-level probe (docs/design/V3_DISPLAY.md §7).
 *
 * Runs IN THE GUEST against the stock nvidia-drm card node, and ON THE HOST with no DRM at all
 * (the `ppm` mode) to produce the reference image the host's QEMU screendump is compared with.
 *
 *   kfdisp_probe list  [card]                 connectors / CRTCs / planes, one line each
 *   kfdisp_probe show  [card] [hold_s] [flips] set the preferred mode on the first connected
 *                                              connector, scan out PATTERN A from a dumb buffer,
 *                                              then page-flip A<->B `flips` times (events counted
 *                                              and timed), end on A and hold `hold_s` seconds, then
 *                                              RESTORE the CRTC it found (fbcon's framebuffer)
 *   kfdisp_probe show  [card] [hold_s] [flips] [async] [gap_ms]
 *                                              (display-max-fps, V3_DISPLAY.md sec. 8.16): `async`
 *                                              flips with DRM_MODE_PAGE_FLIP_ASYNC (nvidia-drm's
 *                                              tearing flips, the D1 gate's input; `vsync` = the
 *                                              default), `gap_ms` waits that long after each flip
 *                                              event before the next flip (a client paced slower than
 *                                              the cap)
 *   kfdisp_probe ppm   <w> <h> [a|b]           write the pattern as a binary PPM to stdout
 *
 * The pattern is a pure function of (x, y, w, h): the guest's dumb buffer and the host's
 * reference PPM are the same bytes by construction, so a screendump that matches is pixel-exact
 * evidence that the scanout path read the right surface, in the right format, at the right pitch.
 *
 * Output lines are KEY=VALUE for the harness (grep-able); every failure prints KFDISP_FAIL=<why>
 * and exits non-zero. ⊘ A flip that never completes is a timeout (KFDISP_FAIL=flip-timeout),
 * never a hang: every wait has a deadline.
 *
 * ⊘ Why `show` restores the CRTC before it exits (m1c, 2026-09-30): a client that exits with its
 * framebuffer still on the primary plane makes the kernel remove that framebuffer in a BLOCKING
 * commit that DISABLES the plane (drm_fb_release -> atomic_remove_fb). nvidia-drm counts one flip
 * event for it (old CRTC active, old fb non-NULL: nvidia-drm-modeset.c __will_generate_flip_event)
 * but NVKMS programs the disabled window with NO completion notifier, so no event can come, and
 * the commit logs "Flip event timeout" after 3 s. That is nvidia-drm's own behaviour on real
 * hardware (NVIDIA/open-gpu-kernel-modules#1361: "framebuffer removal on DRM file close ... no
 * flip-complete event arrives"), not a display-engine defect — so the probe does what a
 * well-behaved client does and puts back what it found; the teardown path is not what it grades.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#ifndef KFDISP_NO_DRM
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <drm_fourcc.h>
#endif

static uint32_t pattern_px(uint32_t x, uint32_t y, uint32_t w, uint32_t h, int b)
{
    /* XRGB8888: 0x00RRGGBB. R ramps across, G ramps down, B is a 64-px checkerboard. */
    uint32_t r = w > 1 ? (x * 255u) / (w - 1) : 0;
    uint32_t g = h > 1 ? (y * 255u) / (h - 1) : 0;
    uint32_t bl = (((x >> 6) ^ (y >> 6)) & 1u) ? 255u : 0u;
    /* a 1-px white frame, so a cropped or shifted scanout is visible at a glance */
    if (x == 0 || y == 0 || x == w - 1 || y == h - 1)
        r = g = bl = 255u;
    if (b) { r ^= 0xffu; g ^= 0xffu; bl ^= 0xffu; }
    return (r << 16) | (g << 8) | bl;
}

/* FNV-1a over the visible pixels as R,G,B bytes (row by row, pitch padding excluded) — the same
 * byte order a binary PPM carries, so the guest's number and the host's are comparable. */
static __attribute__((unused)) uint64_t fnv_rgb(const uint8_t *base, uint32_t pitch, uint32_t w, uint32_t h)
{
    uint64_t hsh = 1469598103934665603ull;
    for (uint32_t y = 0; y < h; y++) {
        const uint32_t *row = (const uint32_t *)(base + (size_t)y * pitch);
        for (uint32_t x = 0; x < w; x++) {
            uint32_t p = row[x];
            uint8_t rgb[3] = { (uint8_t)(p >> 16), (uint8_t)(p >> 8), (uint8_t)p };
            for (int i = 0; i < 3; i++) { hsh ^= rgb[i]; hsh *= 1099511628211ull; }
        }
    }
    return hsh;
}

static int cmd_ppm(int argc, char **argv)
{
    if (argc < 4) { fprintf(stderr, "usage: ppm <w> <h> [a|b]\n"); return 2; }
    uint32_t w = (uint32_t)strtoul(argv[2], NULL, 0), h = (uint32_t)strtoul(argv[3], NULL, 0);
    int b = argc > 4 && argv[4][0] == 'b';
    if (!w || !h || w > 16384 || h > 16384) { fprintf(stderr, "bad size\n"); return 2; }
    printf("P6\n%u %u\n255\n", w, h);
    for (uint32_t y = 0; y < h; y++)
        for (uint32_t x = 0; x < w; x++) {
            uint32_t p = pattern_px(x, y, w, h, b);
            putchar((p >> 16) & 0xff); putchar((p >> 8) & 0xff); putchar(p & 0xff);
        }
    return 0;
}

#ifdef KFDISP_NO_DRM
int main(int argc, char **argv)
{
    if (argc >= 2 && !strcmp(argv[1], "ppm")) return cmd_ppm(argc, argv);
    fprintf(stderr, "host build: only `ppm` is available\n");
    return 2;
}
#else

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static const char *conn_type(uint32_t t)
{
    switch (t) {
    case DRM_MODE_CONNECTOR_VGA: return "VGA";
    case DRM_MODE_CONNECTOR_DVII: return "DVI-I";
    case DRM_MODE_CONNECTOR_DVID: return "DVI-D";
    case DRM_MODE_CONNECTOR_DisplayPort: return "DP";
    case DRM_MODE_CONNECTOR_HDMIA: return "HDMI-A";
    case DRM_MODE_CONNECTOR_HDMIB: return "HDMI-B";
    case DRM_MODE_CONNECTOR_eDP: return "eDP";
    case DRM_MODE_CONNECTOR_VIRTUAL: return "Virtual";
    case DRM_MODE_CONNECTOR_LVDS: return "LVDS";
    default: return "other";
    }
}

static int open_card(const char *path)
{
    int fd = open(path, O_RDWR | O_CLOEXEC);
    if (fd < 0) { printf("KFDISP_FAIL=open %s: %s\n", path, strerror(errno)); return -1; }
    drmVersionPtr v = drmGetVersion(fd);
    if (v) { printf("KFDISP_DRIVER=%s\n", v->name); drmFreeVersion(v); }
    return fd;
}

static int cmd_list(const char *card)
{
    int fd = open_card(card);
    if (fd < 0) return 1;
    drmSetClientCap(fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1);
    drmModeResPtr res = drmModeGetResources(fd);
    if (!res) { printf("KFDISP_FAIL=no-kms-resources (%s)\n", strerror(errno)); return 1; }
    printf("KFDISP_COUNTS connectors=%d crtcs=%d encoders=%d fbs=%d\n",
           res->count_connectors, res->count_crtcs, res->count_encoders, res->count_fbs);
    int connected = 0;
    for (int i = 0; i < res->count_connectors; i++) {
        drmModeConnectorPtr c = drmModeGetConnector(fd, res->connectors[i]);
        if (!c) continue;
        const char *st = c->connection == DRM_MODE_CONNECTED ? "connected" :
                         c->connection == DRM_MODE_DISCONNECTED ? "disconnected" : "unknown";
        if (c->connection == DRM_MODE_CONNECTED) connected++;
        printf("KFDISP_CONNECTOR id=%u type=%s-%u status=%s modes=%d",
               c->connector_id, conn_type(c->connector_type), c->connector_type_id, st, c->count_modes);
        for (int m = 0; m < c->count_modes; m++)
            if (c->modes[m].type & DRM_MODE_TYPE_PREFERRED)
                printf(" preferred=%ux%u@%u", c->modes[m].hdisplay, c->modes[m].vdisplay, c->modes[m].vrefresh);
        /* display-max-fps (sec. 8.16): the fastest mode the guest was offered (a cap below 60 must
         * leave no 60 Hz mode in the list) */
        uint32_t maxv = 0;
        for (int m = 0; m < c->count_modes; m++)
            if (c->modes[m].vrefresh > maxv) maxv = c->modes[m].vrefresh;
        printf(" max_vrefresh=%u", maxv);
        printf("\n");
        drmModeFreeConnector(c);
    }
    drmModePlaneResPtr pr = drmModeGetPlaneResources(fd);
    int ncursor = 0, nprimary = 0, noverlay = 0;
    for (uint32_t i = 0; pr && i < pr->count_planes; i++) {
        drmModePlanePtr p = drmModeGetPlane(fd, pr->planes[i]);
        if (!p) continue;
        drmModeObjectPropertiesPtr props = drmModeObjectGetProperties(fd, p->plane_id, DRM_MODE_OBJECT_PLANE);
        const char *ty = "?";
        for (uint32_t k = 0; props && k < props->count_props; k++) {
            drmModePropertyPtr pp = drmModeGetProperty(fd, props->props[k]);
            if (pp && !strcmp(pp->name, "type")) {
                uint64_t v = props->prop_values[k];
                ty = v == DRM_PLANE_TYPE_PRIMARY ? "primary" : v == DRM_PLANE_TYPE_CURSOR ? "cursor" : "overlay";
            }
            if (pp) drmModeFreeProperty(pp);
        }
        if (!strcmp(ty, "primary")) nprimary++; else if (!strcmp(ty, "cursor")) ncursor++; else noverlay++;
        printf("KFDISP_PLANE id=%u type=%s crtcs=0x%x formats=%u\n", p->plane_id, ty, p->possible_crtcs, p->count_formats);
        if (props) drmModeFreeObjectProperties(props);
        drmModeFreePlane(p);
    }
    printf("KFDISP_SUMMARY connected=%d primary=%d overlay=%d cursor=%d\n", connected, nprimary, noverlay, ncursor);
    return connected > 0 ? 0 : 3;
}

struct dumb { uint32_t handle, pitch, fb; uint64_t size; uint8_t *map; };

static int dumb_make(int fd, uint32_t w, uint32_t h, int b, struct dumb *d)
{
    struct drm_mode_create_dumb cr = { .width = w, .height = h, .bpp = 32 };
    if (drmIoctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &cr)) { printf("KFDISP_FAIL=create-dumb %s\n", strerror(errno)); return -1; }
    d->handle = cr.handle; d->pitch = cr.pitch; d->size = cr.size;
    struct drm_mode_map_dumb mp = { .handle = cr.handle };
    if (drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mp)) { printf("KFDISP_FAIL=map-dumb %s\n", strerror(errno)); return -1; }
    d->map = mmap(NULL, d->size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, (off_t)mp.offset);
    if (d->map == MAP_FAILED) { printf("KFDISP_FAIL=mmap-dumb %s\n", strerror(errno)); return -1; }
    for (uint32_t y = 0; y < h; y++) {
        uint32_t *row = (uint32_t *)(d->map + (size_t)y * d->pitch);
        for (uint32_t x = 0; x < w; x++) row[x] = pattern_px(x, y, w, h, b);
    }
    uint32_t handles[4] = { d->handle }, pitches[4] = { d->pitch }, offsets[4] = { 0 };
    if (drmModeAddFB2(fd, w, h, DRM_FORMAT_XRGB8888, handles, pitches, offsets, &d->fb, 0)) {
        printf("KFDISP_FAIL=addfb2 %s\n", strerror(errno)); return -1;
    }
    return 0;
}

static volatile int g_flip_done;
static double g_last_flip;
static void on_flip(int fd, unsigned int seq, unsigned int sec, unsigned int usec, void *data)
{
    (void)fd; (void)seq; (void)data;
    g_flip_done = 1;
    g_last_flip = (double)sec + (double)usec / 1e6;
}

static int wait_flip(int fd, double deadline_s)
{
    drmEventContext ev = { .version = 2, .page_flip_handler = on_flip };
    g_flip_done = 0;
    double end = now_s() + deadline_s;
    while (!g_flip_done) {
        double left = end - now_s();
        if (left <= 0) return -1;
        struct pollfd p = { .fd = fd, .events = POLLIN };
        int r = poll(&p, 1, (int)(left * 1000) + 1);
        if (r > 0) drmHandleEvent(fd, &ev);
        else if (r < 0 && errno != EINTR) return -1;
    }
    return 0;
}

static int cmd_show(const char *card, int hold_s, int flips, int async, int gap_ms)
{
    int fd = open_card(card);
    if (fd < 0) return 1;
    drmModeResPtr res = drmModeGetResources(fd);
    if (!res) { printf("KFDISP_FAIL=no-kms-resources\n"); return 1; }
    drmModeConnectorPtr conn = NULL;
    for (int i = 0; i < res->count_connectors && !conn; i++) {
        drmModeConnectorPtr c = drmModeGetConnector(fd, res->connectors[i]);
        if (c && c->connection == DRM_MODE_CONNECTED && c->count_modes > 0) conn = c;
        else if (c) drmModeFreeConnector(c);
    }
    if (!conn) { printf("KFDISP_FAIL=no-connected-connector\n"); return 3; }
    drmModeModeInfo mode = conn->modes[0];
    for (int m = 0; m < conn->count_modes; m++)
        if (conn->modes[m].type & DRM_MODE_TYPE_PREFERRED) { mode = conn->modes[m]; break; }
    uint32_t crtc = 0;
    for (int e = 0; e < conn->count_encoders && !crtc; e++) {
        drmModeEncoderPtr enc = drmModeGetEncoder(fd, conn->encoders[e]);
        for (int i = 0; enc && i < res->count_crtcs; i++)
            if (enc->possible_crtcs & (1u << i)) { crtc = res->crtcs[i]; break; }
        if (enc) drmModeFreeEncoder(enc);
    }
    if (!crtc) { printf("KFDISP_FAIL=no-crtc\n"); return 3; }
    uint32_t w = mode.hdisplay, h = mode.vdisplay;
    printf("KFDISP_MODE %ux%u@%u clock=%u connector=%u crtc=%u\n", w, h, mode.vrefresh, mode.clock, conn->connector_id, crtc);
    struct dumb a = { 0 }, b = { 0 };
    if (dumb_make(fd, w, h, 0, &a) || dumb_make(fd, w, h, 1, &b)) return 4;
    printf("KFDISP_PATTERN_A_FNV=%016" PRIx64 " pitch=%u\n", fnv_rgb(a.map, a.pitch, w, h), a.pitch);
    printf("KFDISP_PATTERN_B_FNV=%016" PRIx64 "\n", fnv_rgb(b.map, b.pitch, w, h));
    /* what was there before us (fbcon's framebuffer), put back before we exit */
    drmModeCrtcPtr saved = drmModeGetCrtc(fd, crtc);
    double t0 = now_s();
    if (drmModeSetCrtc(fd, crtc, a.fb, 0, 0, &conn->connector_id, 1, &mode)) {
        printf("KFDISP_FAIL=setcrtc %s\n", strerror(errno)); return 5;
    }
    printf("KFDISP_SETCRTC_OK ms=%.1f\n", (now_s() - t0) * 1e3);
    int done = 0;
    double first = 0, last = 0;
    uint32_t fl = DRM_MODE_PAGE_FLIP_EVENT | (async ? DRM_MODE_PAGE_FLIP_ASYNC : 0);
    uint64_t cap_async = 0;
    drmGetCap(fd, DRM_CAP_ASYNC_PAGE_FLIP, &cap_async);
    printf("KFDISP_FLIP_MODE=%s gap_ms=%d cap_async_page_flip=%" PRIu64 "\n",
           async ? "async" : "vsync", gap_ms, cap_async);
    for (int i = 0; i < flips; i++) {
        struct dumb *nx = (i & 1) ? &a : &b;
        if (drmModePageFlip(fd, crtc, nx->fb, fl, NULL)) {
            printf("KFDISP_FAIL=pageflip#%d %s\n", i, strerror(errno)); break;
        }
        if (wait_flip(fd, 2.0)) { printf("KFDISP_FAIL=flip-timeout#%d\n", i); break; }
        if (!done) first = g_last_flip;
        last = g_last_flip;
        done++;
        if (gap_ms > 0) usleep((useconds_t)gap_ms * 1000);
    }
    /* end on A so the screendump grades pattern A */
    if (done & 1) {
        if (!drmModePageFlip(fd, crtc, a.fb, DRM_MODE_PAGE_FLIP_EVENT, NULL)) wait_flip(fd, 2.0);
    }
    double hz = (done > 1 && last > first) ? (double)(done - 1) / (last - first) : 0.0;
    printf("KFDISP_FLIPS=%d/%d flip_hz=%.2f\n", done, flips, hz);
    printf("KFDISP_SHOWING=A hold_s=%d\n", hold_s);
    fflush(stdout);
    if (hold_s > 0) sleep((unsigned)hold_s);
    if (saved && saved->mode_valid && saved->buffer_id) {
        int rc = drmModeSetCrtc(fd, crtc, saved->buffer_id, saved->x, saved->y,
                                &conn->connector_id, 1, &saved->mode);
        printf("KFDISP_RESTORED=%s fb=%u\n", rc ? strerror(errno) : "ok", saved->buffer_id);
    } else {
        printf("KFDISP_RESTORED=nothing-to-restore\n");
    }
    if (saved) drmModeFreeCrtc(saved);
    fflush(stdout);
    return done == flips ? 0 : 6;
}

int main(int argc, char **argv)
{
    if (argc < 2) { fprintf(stderr, "usage: kfdisp_probe list|show|ppm ...\n"); return 2; }
    const char *card = "/dev/dri/card0";
    if (!strcmp(argv[1], "ppm")) return cmd_ppm(argc, argv);
    if (!strcmp(argv[1], "list")) return cmd_list(argc > 2 ? argv[2] : card);
    if (!strcmp(argv[1], "show"))
        return cmd_show(argc > 2 ? argv[2] : card, argc > 3 ? atoi(argv[3]) : 10, argc > 4 ? atoi(argv[4]) : 120,
                        argc > 5 && !strcmp(argv[5], "async"), argc > 6 ? atoi(argv[6]) : 0);
    fprintf(stderr, "unknown mode %s\n", argv[1]);
    return 2;
}
#endif
