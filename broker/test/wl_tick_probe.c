/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * wl_tick_probe.c -- NOT SHIPPED.  A measuring instrument for one question:
 * what must a Wayland client commit, each refresh, to keep getting a frame
 * callback AND a wp_presentation event?
 *
 * The wl_surface protocol text says `frame` "takes effect on the next
 * wl_surface.commit" and that a server "should avoid signaling the frame
 * callbacks if the surface is not visible in any way"; it does NOT say a
 * commit without damage gets a callback.  So it is measured, per compositor:
 *
 *   variant 0  the commit changes nothing: frame request + presentation
 *              feedback + wl_surface.commit, no attach, no damage
 *   variant 1  the same, plus damage_buffer(0,0,1,1) on the buffer already
 *              attached
 *   variant 2  the same, plus attach of the SAME buffer and full damage
 *   variant 9  control: commit once and never again
 *
 * Each callback / presented / discarded event triggers the next commit (what
 * the broker's tick pump does), with a 250 ms watchdog that commits anyway
 * and counts a "stall".  Prints one summary line per run, parsed by
 * test/test_tickpump.py.  `minimize` after N seconds asks the compositor to
 * minimise the window and reports whether events continued.
 *
 *     wl_tick_probe <variant> <seconds> [minimize_after_seconds] [trigger]
 *
 * trigger: `a` (default) every callback / presented / discarded event commits
 * again; `p` only a `presented` does (a callback or a `discarded` does not --
 * what the broker's pump does, since a callback may fire at once and a
 * `discarded` carries no timestamp), with the 250 ms watchdog restarting it.
 */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#include <wayland-client.h>
#include "presentation-time-client-protocol.h"
#include "xdg-shell-client-protocol.h"

#define W 640
#define H 360

static uint64_t now_ns(void)
{
    struct timespec ts;

    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

struct ctx {
    struct wl_display *dpy;
    struct wl_compositor *comp;
    struct wl_shm *shm;
    struct xdg_wm_base *wm;
    struct wp_presentation *pres;
    struct wl_surface *surf;
    struct xdg_surface *xs;
    struct xdg_toplevel *top;
    struct wl_buffer *buf;
    bool configured, running, pending_cb, minimized;
    int variant;
    bool only_presented, only_cb;
    uint32_t clock_id;
    bool have_clock;
    uint64_t t0, last_commit, last_event;
    /* counts, split before / after the minimise request */
    unsigned commits, cb, presented, discarded, stalls;
    unsigned cb_min, presented_min, discarded_min;
    uint64_t first_pres_ns, last_pres_ns;
    uint32_t refresh_ns;
    unsigned pflags_zero_copy, pflags_vsync;
    uint64_t sum_interval, n_interval, max_interval;
    uint64_t cb_first_ns, cb_last_ns;
    unsigned pres_in_order_violations;
};

static void do_commit(struct ctx *c);

static void frame_done(void *d, struct wl_callback *cb, uint32_t t)
{
    struct ctx *c = d;

    (void)t;
    wl_callback_destroy(cb);
    c->pending_cb = false;
    if (c->minimized) {
        c->cb_min++;
    } else {
        c->cb++;
    }
    if (!c->cb_first_ns) {
        c->cb_first_ns = now_ns();
    }
    c->cb_last_ns = now_ns();
    c->last_event = now_ns();
    if (c->running && c->variant != 9 && !c->only_presented) {
        do_commit(c);
    }
}
static const struct wl_callback_listener frame_l = { .done = frame_done };

static void pres_sync(void *d, struct wp_presentation_feedback *f, struct wl_output *o)
{ (void)d; (void)f; (void)o; }
static void pres_presented(void *d, struct wp_presentation_feedback *f,
                           uint32_t hi, uint32_t lo, uint32_t nsec,
                           uint32_t refresh, uint32_t sh, uint32_t sl,
                           uint32_t flags)
{
    struct ctx *c = d;
    uint64_t ts = ((uint64_t)hi << 32 | lo) * 1000000000ull + nsec;

    (void)sh; (void)sl;
    wp_presentation_feedback_destroy(f);
    if (c->minimized) {
        c->presented_min++;
    } else {
        c->presented++;
    }
    c->refresh_ns = refresh;
    if (flags & WP_PRESENTATION_FEEDBACK_KIND_ZERO_COPY) {
        c->pflags_zero_copy++;
    }
    if (flags & WP_PRESENTATION_FEEDBACK_KIND_VSYNC) {
        c->pflags_vsync++;
    }
    if (c->last_pres_ns) {
        if (ts <= c->last_pres_ns) {
            c->pres_in_order_violations++;
        } else {
            uint64_t iv = ts - c->last_pres_ns;

            c->sum_interval += iv;
            c->n_interval++;
            if (iv > c->max_interval) {
                c->max_interval = iv;
            }
        }
    } else {
        c->first_pres_ns = ts;
    }
    c->last_pres_ns = ts;
    c->last_event = now_ns();
    if (c->running && c->variant != 9 && (c->only_presented || (!c->only_cb && !c->pending_cb))) {
        do_commit(c);
    }
}
static void pres_discarded(void *d, struct wp_presentation_feedback *f)
{
    struct ctx *c = d;

    wp_presentation_feedback_destroy(f);
    if (c->minimized) {
        c->discarded_min++;
    } else {
        c->discarded++;
    }
    c->last_event = now_ns();
    if (c->running && c->variant != 9 && !c->only_presented && !c->only_cb && !c->pending_cb) {
        do_commit(c);
    }
}
static const struct wp_presentation_feedback_listener pres_l = {
    .sync_output = pres_sync, .presented = pres_presented, .discarded = pres_discarded,
};

static void pres_clock(void *d, struct wp_presentation *p, uint32_t id)
{
    struct ctx *c = d;

    (void)p;
    c->clock_id = id;
    c->have_clock = true;
}
static const struct wp_presentation_listener presentation_l = { .clock_id = pres_clock };

static void do_commit(struct ctx *c)
{
    struct wl_callback *cb;
    struct wp_presentation_feedback *fb;

    if (c->variant == 2) {
        wl_surface_attach(c->surf, c->buf, 0, 0);
        wl_surface_damage_buffer(c->surf, 0, 0, W, H);
    } else if (c->variant == 1) {
        wl_surface_damage_buffer(c->surf, 0, 0, 1, 1);
    }
    if (c->pres) {
        fb = wp_presentation_feedback(c->pres, c->surf);
        wp_presentation_feedback_add_listener(fb, &pres_l, c);
    }
    cb = wl_surface_frame(c->surf);
    wl_callback_add_listener(cb, &frame_l, c);
    c->pending_cb = true;
    wl_surface_commit(c->surf);
    c->commits++;
    c->last_commit = now_ns();
}

static void reg_global(void *d, struct wl_registry *r, uint32_t name,
                       const char *iface, uint32_t ver)
{
    struct ctx *c = d;

    if (!strcmp(iface, "wl_compositor")) {
        c->comp = wl_registry_bind(r, name, &wl_compositor_interface, 4);
    } else if (!strcmp(iface, "wl_shm")) {
        c->shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
    } else if (!strcmp(iface, "xdg_wm_base")) {
        c->wm = wl_registry_bind(r, name, &xdg_wm_base_interface, 1);
    } else if (!strcmp(iface, "wp_presentation")) {
        c->pres = wl_registry_bind(r, name, &wp_presentation_interface, 1);
        wp_presentation_add_listener(c->pres, &presentation_l, c);
    }
    (void)ver;
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t n)
{ (void)d; (void)r; (void)n; }
static const struct wl_registry_listener reg_l = { reg_global, reg_remove };

static void wm_ping(void *d, struct xdg_wm_base *wm, uint32_t s)
{ (void)d; xdg_wm_base_pong(wm, s); }
static const struct xdg_wm_base_listener wm_l = { wm_ping };

static void xs_conf(void *d, struct xdg_surface *xs, uint32_t serial)
{
    struct ctx *c = d;

    xdg_surface_ack_configure(xs, serial);
    c->configured = true;
}
static const struct xdg_surface_listener xs_l = { xs_conf };
static void top_conf(void *d, struct xdg_toplevel *t, int32_t w, int32_t h,
                     struct wl_array *s)
{ (void)d; (void)t; (void)w; (void)h; (void)s; }
static void top_close(void *d, struct xdg_toplevel *t) { (void)d; (void)t; }
static const struct xdg_toplevel_listener top_l = { .configure = top_conf, .close = top_close };

int main(int argc, char **argv)
{
    struct ctx c;
    double secs, minimize_after = -1;
    int fd;
    void *px;
    struct wl_shm_pool *pool;
    struct wl_registry *reg;
    uint64_t end, min_at = 0;

    memset(&c, 0, sizeof c);
    if (argc < 3) {
        fprintf(stderr, "usage: %s <variant 0|1|2|9> <seconds> [minimize_after]\n", argv[0]);
        return 2;
    }
    c.variant = atoi(argv[1]);
    secs = atof(argv[2]);
    if (argc > 3) {
        minimize_after = atof(argv[3]);
    }
    c.only_presented = argc > 4 && argv[4][0] == 'p';
    c.only_cb = argc > 4 && argv[4][0] == 'c';
    c.dpy = wl_display_connect(NULL);
    if (!c.dpy) {
        fprintf(stderr, "no wayland display\n");
        return 77;
    }
    reg = wl_display_get_registry(c.dpy);
    wl_registry_add_listener(reg, &reg_l, &c);
    wl_display_roundtrip(c.dpy);
    wl_display_roundtrip(c.dpy);
    if (!c.comp || !c.shm || !c.wm) {
        fprintf(stderr, "compositor lacks wl_compositor/wl_shm/xdg_wm_base\n");
        return 77;
    }
    fd = memfd_create("probe", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, W * H * 4) < 0) {
        return 1;
    }
    px = mmap(NULL, W * H * 4, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    memset(px, 0x40, W * H * 4);
    pool = wl_shm_create_pool(c.shm, fd, W * H * 4);
    c.buf = wl_shm_pool_create_buffer(pool, 0, W, H, W * 4, WL_SHM_FORMAT_XRGB8888);
    xdg_wm_base_add_listener(c.wm, &wm_l, &c);
    c.surf = wl_compositor_create_surface(c.comp);
    c.xs = xdg_wm_base_get_xdg_surface(c.wm, c.surf);
    xdg_surface_add_listener(c.xs, &xs_l, &c);
    c.top = xdg_surface_get_toplevel(c.xs);
    xdg_toplevel_add_listener(c.top, &top_l, &c);
    xdg_toplevel_set_title(c.top, "wl_tick_probe");
    wl_surface_commit(c.surf);
    while (!c.configured && wl_display_dispatch(c.dpy) >= 0) {
    }
    wl_surface_attach(c.surf, c.buf, 0, 0);
    wl_surface_damage_buffer(c.surf, 0, 0, W, H);
    c.t0 = now_ns();
    c.running = true;
    if (c.variant == 9) {
        do_commit(&c);          /* once */
    } else {
        do_commit(&c);
    }
    end = c.t0 + (uint64_t)(secs * 1e9);
    if (minimize_after >= 0) {
        min_at = c.t0 + (uint64_t)(minimize_after * 1e9);
    }
    while (now_ns() < end) {
        struct pollfd p = { wl_display_get_fd(c.dpy), POLLIN, 0 };
        uint64_t n;

        wl_display_flush(c.dpy);
        while (wl_display_prepare_read(c.dpy) != 0) {
            wl_display_dispatch_pending(c.dpy);
        }
        wl_display_flush(c.dpy);
        if (poll(&p, 1, 10) > 0) {
            wl_display_read_events(c.dpy);
        } else {
            wl_display_cancel_read(c.dpy);
        }
        wl_display_dispatch_pending(c.dpy);
        n = now_ns();
        if (min_at && n >= min_at && !c.minimized) {
            c.minimized = true;
            xdg_toplevel_set_minimized(c.top);
            wl_display_flush(c.dpy);
            min_at = 0;
        }
        /* the watchdog: a pump that stopped is restarted by a plain commit */
        if (c.variant != 9 && (c.pending_cb || c.only_presented) &&
            n > c.last_event && n > c.last_commit &&
            n - c.last_event > 250000000ull && n - c.last_commit > 250000000ull) {
            c.stalls++;
            c.last_commit = n;
            do_commit(&c);
        }
    }
    {
        double dt = (double)(c.minimized ? (min_at ? 0 : 0) : 0);
        double span = secs;

        (void)dt;
        printf("PROBE trigger=%c variant=%d secs=%.1f commits=%u frame_cb=%u presented=%u "
               "discarded=%u stalls=%u clock_id=%d refresh_ns=%u "
               "mean_interval_ns=%llu max_interval_ns=%llu zero_copy=%u vsync=%u "
               "out_of_order=%u cb_rate_hz=%.2f pres_rate_hz=%.2f "
               "after_minimize_cb=%u after_minimize_presented=%u "
               "after_minimize_discarded=%u\n",
               c.only_presented ? 'p' : c.only_cb ? 'c' : 'a', c.variant, secs, c.commits, c.cb + c.cb_min,
               c.presented + c.presented_min, c.discarded + c.discarded_min,
               c.stalls, c.have_clock ? (int)c.clock_id : -1, c.refresh_ns,
               (unsigned long long)(c.n_interval ? c.sum_interval / c.n_interval : 0),
               (unsigned long long)c.max_interval, c.pflags_zero_copy,
               c.pflags_vsync, c.pres_in_order_violations,
               (c.cb + c.cb_min) / span, (c.presented + c.presented_min) / span,
               c.cb_min, c.presented_min, c.discarded_min);
    }
    return 0;
}
