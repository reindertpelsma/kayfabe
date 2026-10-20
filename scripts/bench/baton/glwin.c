/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * glwin.c - a windowed OpenGL (GLX) client for the desktop baton test: one X11 window, a fill-rate
 * bound fixed-function scene (N alpha-blended full-window layers per frame), and a per-frame log in
 * baton.c's format, so baton_parse.py / desktop_baton_parse.py read it unchanged.
 *
 * No X11 / GL development headers needed: libX11.so.6 and libGL.so.1 are dlopen()ed and the few
 * structs are declared here (the guest image has gcc, libX11 and libGL, but maybe not -dev).
 *
 *   gcc -O2 -Wall -o glwin glwin.c -ldl -lm
 *   ./glwin --label A --duration 60 --x 10 --y 40 --w 940 --h 700 --layers 400 --vsync 0 \
 *           --start-at <epoch> --dump-all A.csv > A.log
 *   ./glwin --calibrate-only --target-ms 5 [--w 940 --h 700]     # prints LAYERS=n
 *
 * One iteration = draw, glXSwapBuffers, glFinish (the frame is complete on the GPU when the
 * iteration ends: the analogue of baton.c's launch + cuStreamSynchronize).  Times are seconds
 * since the timed loop started; t0_epoch is CLOCK_REALTIME at that moment.
 * --canary: a small vsynced window with a trivial scene; its frame cadence is what the
 * compositor lets a client present.  --require-renderer STR: exit 3 unless GL_RENDERER has STR
 * (so a software fallback never produces a "GPU" number).
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

/* ---- minimal Xlib / GLX declarations (ABI of libX11 / libGL on LP64 Linux) ---- */
typedef struct _XDisplay Display;
typedef unsigned long XID, Window, Colormap, VisualID, Pixmap, Cursor;
typedef struct { void *ext_data; VisualID visualid; int c_class; unsigned long red_mask, green_mask, blue_mask;
                 int bits_per_rgb, map_entries; } Visual;
typedef struct { Visual *visual; VisualID visualid; int screen, depth, c_class; unsigned long red_mask, green_mask,
                 blue_mask; int colormap_size, bits_per_rgb; } XVisualInfo;
typedef struct { Pixmap background_pixmap; unsigned long background_pixel; Pixmap border_pixmap;
                 unsigned long border_pixel; int bit_gravity, win_gravity, backing_store;
                 unsigned long backing_planes, backing_pixel; int save_under; long event_mask,
                 do_not_propagate_mask; int override_redirect; Colormap colormap; Cursor cursor; } XSetWindowAttributes;
typedef struct { long flags; int x, y, width, height, min_width, min_height, max_width, max_height, width_inc,
                 height_inc; struct { int x, y; } min_aspect, max_aspect; int base_width, base_height, win_gravity; } XSizeHints;
typedef union { int type; long pad[24]; } XEvent;
typedef struct __GLXcontextRec *GLXContext;
typedef unsigned long GLXDrawable;

#define CWBorderPixel (1L << 3)
#define CWEventMask (1L << 11)
#define CWColormap (1L << 13)
#define StructureNotifyMask (1L << 17)
#define ExposureMask (1L << 15)
#define USPosition 1L
#define USSize 2L
#define GLX_RGBA 4
#define GLX_DOUBLEBUFFER 5
#define GLX_RED_SIZE 8
#define GLX_GREEN_SIZE 9
#define GLX_BLUE_SIZE 10
#define GL_BLEND 0x0BE2
#define GL_SRC_ALPHA 0x0302
#define GL_ONE_MINUS_SRC_ALPHA 0x0303
#define GL_COLOR_BUFFER_BIT 0x4000
#define GL_QUADS 7
#define GL_RENDERER 0x1F01
#define GL_VERSION 0x1F02

#define FN(ret, name, args) static ret (*name) args
FN(Display *, XOpenDisplay, (const char *));
FN(int, XDefaultScreen, (Display *));
FN(Window, XRootWindow, (Display *, int));
FN(Colormap, XCreateColormap, (Display *, Window, Visual *, int));
FN(Window, XCreateWindow, (Display *, Window, int, int, unsigned, unsigned, unsigned, int, unsigned, Visual *,
                           unsigned long, XSetWindowAttributes *));
FN(int, XStoreName, (Display *, Window, const char *));
FN(void, XSetWMNormalHints, (Display *, Window, XSizeHints *));
FN(int, XMapWindow, (Display *, Window));
FN(int, XMoveWindow, (Display *, Window, int, int));
FN(int, XPending, (Display *));
FN(int, XNextEvent, (Display *, XEvent *));
FN(int, XSync, (Display *, int));
FN(int, XFree, (void *));
FN(XVisualInfo *, glXChooseVisual, (Display *, int, int *));
FN(GLXContext, glXCreateContext, (Display *, XVisualInfo *, GLXContext, int));
FN(int, glXMakeCurrent, (Display *, GLXDrawable, GLXContext));
FN(void, glXSwapBuffers, (Display *, GLXDrawable));
FN(void *, glXGetProcAddressARB, (const unsigned char *));
FN(const unsigned char *, glGetString, (unsigned));
FN(void, glViewport, (int, int, int, int));
FN(void, glClearColor, (float, float, float, float));
FN(void, glClear, (unsigned));
FN(void, glEnable, (unsigned));
FN(void, glBlendFunc, (unsigned, unsigned));
FN(void, glBegin, (unsigned));
FN(void, glEnd, (void));
FN(void, glColor4f, (float, float, float, float));
FN(void, glVertex2f, (float, float));
FN(void, glFinish, (void));

static void *lx, *lg;
static void *need(void *h, const char *n)
{
    void *p = dlsym(h, n);
    if (!p) { fprintf(stderr, "glwin: missing symbol %s\n", n); exit(2); }
    return p;
}
#define X(n) n = need(lx, #n)
#define G(n) n = need(lg, #n)

static volatile sig_atomic_t g_stop;
static void on_term(int s) { (void)s; g_stop = 1; }
static double mono(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec * 1e-9; }
static double real(void) { struct timespec t; clock_gettime(CLOCK_REALTIME, &t); return t.tv_sec + t.tv_nsec * 1e-9; }

typedef struct { double start, dur_ms, done; } Iter;

static Display *dpy;
static Window win;
static int W = 940, H = 700;

static void draw(int layers, unsigned frame)
{
    glViewport(0, 0, W, H);
    glClearColor(0.f, 0.f, 0.f, 1.f);
    glClear(GL_COLOR_BUFFER_BIT);
    glBegin(GL_QUADS);
    for (int i = 0; i < layers; i++) {
        float k = (float)((i * 37u + frame) % 255u) / 255.f, j = (float)i * 1e-4f;
        glColor4f(k, 1.f - k, 0.5f + 0.5f * sinf(j + frame * 0.01f), 0.02f);
        glVertex2f(-1.f + j, -1.f); glVertex2f(1.f, -1.f + j); glVertex2f(1.f - j, 1.f); glVertex2f(-1.f, 1.f - j);
    }
    glEnd();
}

static void pump(void)
{
    XEvent e;
    while (XPending(dpy)) XNextEvent(dpy, &e);
}

static void frame(int layers, unsigned n)
{
    draw(layers, n);
    glXSwapBuffers(dpy, win);
    glFinish();
    pump();
}

static int usage(void)
{
    fprintf(stderr, "usage: glwin [--label L] [--duration S] [--x X --y Y --w W --h H] [--layers N] [--vsync 0|1]\n"
                    "             [--start-at EPOCH] [--dump-all FILE] [--slow-ms MS] [--title T] [--canary]\n"
                    "             [--calibrate-only --target-ms MS] [--require-renderer STR]\n");
    exit(1);
}

int main(int argc, char **argv)
{
    const char *label = "A", *dump_path = NULL, *title = NULL, *req = NULL;
    double duration = 60, start_at = 0, slow_ms = 50, target_ms = 5;
    int x = 10, y = 40, layers = 100, vsync = 0, calibrate_only = 0, canary = 0;
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
#define ARG(s) (!strcmp(a, s) && i + 1 < argc)
        if (ARG("--label")) label = argv[++i];
        else if (ARG("--duration")) duration = atof(argv[++i]);
        else if (ARG("--x")) x = atoi(argv[++i]);
        else if (ARG("--y")) y = atoi(argv[++i]);
        else if (ARG("--w")) W = atoi(argv[++i]);
        else if (ARG("--h")) H = atoi(argv[++i]);
        else if (ARG("--layers")) layers = atoi(argv[++i]);
        else if (ARG("--vsync")) vsync = atoi(argv[++i]);
        else if (ARG("--start-at")) start_at = atof(argv[++i]);
        else if (ARG("--dump-all")) dump_path = argv[++i];
        else if (ARG("--slow-ms")) slow_ms = atof(argv[++i]);
        else if (ARG("--title")) title = argv[++i];
        else if (ARG("--target-ms")) target_ms = atof(argv[++i]);
        else if (ARG("--require-renderer")) req = argv[++i];
        else if (!strcmp(a, "--calibrate-only")) calibrate_only = 1;
        else if (!strcmp(a, "--canary")) canary = 1;
        else usage();
    }
    if (duration <= 0 || W < 16 || H < 16) usage();
    if (canary) { vsync = 1; layers = 1; }

    lx = dlopen("libX11.so.6", RTLD_NOW);
    lg = dlopen("libGL.so.1", RTLD_NOW);
    if (!lx || !lg) { fprintf(stderr, "glwin: dlopen libX11/libGL: %s\n", dlerror()); return 2; }
    X(XOpenDisplay); X(XDefaultScreen); X(XRootWindow); X(XCreateColormap); X(XCreateWindow); X(XStoreName);
    X(XSetWMNormalHints); X(XMapWindow); X(XMoveWindow); X(XPending); X(XNextEvent); X(XSync); X(XFree);
    G(glXChooseVisual); G(glXCreateContext); G(glXMakeCurrent); G(glXSwapBuffers); G(glXGetProcAddressARB);
    G(glGetString); G(glViewport); G(glClearColor); G(glClear); G(glEnable); G(glBlendFunc); G(glBegin); G(glEnd);
    G(glColor4f); G(glVertex2f); G(glFinish);

    dpy = XOpenDisplay(NULL);
    if (!dpy) { fprintf(stderr, "glwin: cannot open the X display (DISPLAY/XAUTHORITY)\n"); return 2; }
    int scr = XDefaultScreen(dpy);
    int attr[] = { GLX_RGBA, GLX_DOUBLEBUFFER, GLX_RED_SIZE, 8, GLX_GREEN_SIZE, 8, GLX_BLUE_SIZE, 8, 0 };
    XVisualInfo *vi = glXChooseVisual(dpy, scr, attr);
    if (!vi) { fprintf(stderr, "glwin: no double-buffered RGBA visual\n"); return 2; }
    Window root = XRootWindow(dpy, scr);
    XSetWindowAttributes swa;
    memset(&swa, 0, sizeof swa);
    swa.colormap = XCreateColormap(dpy, root, vi->visual, 0);
    swa.event_mask = StructureNotifyMask | ExposureMask;
    win = XCreateWindow(dpy, root, x, y, (unsigned)W, (unsigned)H, 0, vi->depth, 1, vi->visual,
                        CWColormap | CWEventMask | CWBorderPixel, &swa);
    char tbuf[64];
    snprintf(tbuf, sizeof tbuf, "glwin-%s", label);
    XStoreName(dpy, win, title ? title : tbuf);
    XSizeHints sh;
    memset(&sh, 0, sizeof sh);
    sh.flags = USPosition | USSize; sh.x = x; sh.y = y; sh.width = W; sh.height = H;
    XSetWMNormalHints(dpy, win, &sh);
    GLXContext ctx = glXCreateContext(dpy, vi, NULL, 1);
    if (!ctx) { fprintf(stderr, "glwin: glXCreateContext failed\n"); return 2; }
    XMapWindow(dpy, win);
    XSync(dpy, 0);
    if (!glXMakeCurrent(dpy, win, ctx)) { fprintf(stderr, "glwin: glXMakeCurrent failed\n"); return 2; }
    XMoveWindow(dpy, win, x, y);
    const char *rend = (const char *)glGetString(GL_RENDERER), *ver = (const char *)glGetString(GL_VERSION);
    if (!rend) rend = "?";
    if (!ver) ver = "?";
    if (req && !strstr(rend, req)) {
        fprintf(stderr, "glwin: GL_RENDERER \"%s\" does not contain \"%s\"\n", rend, req);
        return 3;
    }
    void (*swapint)(Display *, GLXDrawable, int) = (void (*)(Display *, GLXDrawable, int))
        glXGetProcAddressARB((const unsigned char *)"glXSwapIntervalEXT");
    int (*swapsgi)(int) = (int (*)(int))glXGetProcAddressARB((const unsigned char *)"glXSwapIntervalSGI");
    const char *sw = "none";
    if (swapint) { swapint(dpy, win, vsync); sw = "EXT"; }
    else if (swapsgi) { swapsgi(vsync); sw = "SGI"; }
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

    if (calibrate_only) {
        /* double the layer count until a frame takes >= target_ms, then scale linearly */
        int n = 16; double ms = 0;
        for (int round = 0; round < 14; round++) {
            for (int w = 0; w < 5; w++) frame(n, (unsigned)w);
            double t = mono();
            for (int f = 0; f < 10; f++) frame(n, (unsigned)f);
            ms = (mono() - t) * 100.0;
            if (ms >= target_ms) break;
            n *= 2;
        }
        int want = ms > 0 ? (int)(n * target_ms / ms + 0.5) : n;
        if (want < 1) want = 1;
        fprintf(stderr, "glwin: calibrate %d layers = %.3f ms/frame (renderer %s, swap control %s)\n", n, ms, rend, sw);
        printf("LAYERS=%d\n", want);
        return 0;
    }

    signal(SIGTERM, on_term);
    signal(SIGINT, on_term);
    unsigned n0 = 0;
    for (int w = 0; w < 5; w++) frame(layers, n0++);   /* warm-up, untimed */
    if (start_at > 0)
        while (!g_stop && real() < start_at) {
            frame(1 > layers ? 1 : (layers > 8 ? 8 : layers), n0++);   /* keep the window alive and exposed */
            usleep(20000);
        }

    size_t cap = 1 << 16, n = 0;
    Iter *it = malloc(cap * sizeof *it);
    FILE *dump = dump_path ? fopen(dump_path, "w") : NULL;
    if (dump) fprintf(dump, "idx,start_s,dur_ms,done_s\n");
    double m0 = mono(), r0 = real();
    printf("# baton v1\n");
    printf("# label=%s pid=%d k_ms=0.000 cycles=%llu grid=%dx%d duration_s=%g kernel=glwin layers=%d vsync=%d "
           "canary=%d swapctl=%s device=\"%s\" driver=\"%s\"\n",
           label, (int)getpid(), (unsigned long long)layers * (unsigned long long)W * (unsigned long long)H, W, H,
           duration, layers, vsync, canary, sw, rend, ver);
    printf("# t0_epoch=%.6f\n", r0);
    printf("sec iterations max_iter_ms\n");
    fflush(stdout);

    int cur_sec = 0; long cur_cnt = 0; double cur_max = 0, busy = 0, gap = 0, gap_end = 0, prev_done = 0, first_done = -1;
    long zero_sec = 0;
    int nsec_window = (int)duration + ((double)(int)duration < duration ? 1 : 0);
    for (;;) {
        double s = mono() - m0;
        if (g_stop || s >= duration) break;
        frame(layers, n0++);
        double d = mono() - m0, dur = (d - s) * 1e3;
        if (n == cap) { cap *= 2; it = realloc(it, cap * sizeof *it); if (!it) return 3; }
        it[n].start = s; it[n].dur_ms = dur; it[n].done = d; n++;
        if (dump) fprintf(dump, "%zu,%.6f,%.3f,%.6f\n", n - 1, s, dur, d);
        busy += dur;
        if (first_done < 0) first_done = d * 1e3;
        if (d - prev_done > gap) { gap = d - prev_done; gap_end = d; }
        prev_done = d;
        int sec = (int)d;
        while (cur_sec < sec) {
            printf("%d %ld %.3f\n", cur_sec, cur_cnt, cur_max);
            if (cur_cnt == 0 && cur_sec < nsec_window) zero_sec++;
            cur_sec++; cur_cnt = 0; cur_max = 0;
            if (dump) fflush(dump);
            fflush(stdout);
        }
        cur_cnt++;
        if (dur > cur_max) cur_max = dur;
    }
    printf("%d %ld %.3f\n", cur_sec, cur_cnt, cur_max);
    if (cur_cnt == 0 && cur_sec < nsec_window) zero_sec++;
    for (int s = cur_sec + 1; s < nsec_window; s++) { printf("%d 0 0.000\n", s); zero_sec++; }
    printf("SUMMARY iterations=%zu busy_ms=%.3f work_cycles=%llu longest_gap_ms=%.3f longest_gap_end_s=%.3f "
           "first_done_ms=%.3f zero_sec=%ld mean_iter_ms=%.3f iters_per_s=%.3f aborted=%d\n",
           n, busy, (unsigned long long)n * (unsigned long long)layers, gap * 1e3, gap_end,
           first_done < 0 ? 0.0 : first_done, zero_sec, n ? busy / (double)n : 0.0, (double)n / duration, (int)g_stop);
    printf("SLOW_CSV_BEGIN idx,start_s,dur_ms,done_s\n");
    for (size_t i = 0; i < n; i++)
        if (it[i].dur_ms > slow_ms) printf("%zu,%.3f,%.3f,%.3f\n", i, it[i].start, it[i].dur_ms, it[i].done);
    printf("SLOW_CSV_END\n");
    fflush(stdout);
    if (dump) fclose(dump);
    return 0;
}
