// wl_scene — a DETERMINISTIC Wayland EGL client for the headless-compositor items of the test set.
//
// It maps one xdg toplevel (app_id "gset"), draws a fixed scene with GLES2 on NVIDIA's EGL Wayland
// platform — three interpenetrating triangles, depth test LESS, colour scaled by window depth (the
// scene of gfx/refcheck.h) — reads its own back buffer and prints WL_SCENE_HASH (FNV-1a 64), commits
// the frame, waits for the compositor's frame callback (the frame was USED), prints WL_SCENE_READY,
// and then stays mapped, drawing nothing new, until SIGTERM or <hold-seconds>. The caller captures the
// compositor's output (weston-screenshooter / grim / wlr-screencopy) while it is mapped, and digests it:
// the composite of a static client over a solid background is a deterministic image, so it is graded
// against bare metal byte for byte.
//   usage: wl_scene [hold-seconds]      needs WAYLAND_DISPLAY; prints WL_SCENE_FAIL <why> on failure
//   build: wayland-scanner client-header/private-code xdg-shell.xml, then
//          gcc wl_scene.c xdg-shell-protocol.c -lwayland-client -lwayland-egl -lEGL -lGLESv2
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <wayland-client.h>
#include <wayland-egl.h>
#include "xdg-shell-client-protocol.h"

#define FAIL(...) do { printf("WL_SCENE_FAIL " __VA_ARGS__); printf("\n"); fflush(stdout); exit(3); } while (0)
static struct wl_compositor *comp; static struct xdg_wm_base *wm;
static int cw, ch, configured, frame_done, stop;
static void on_sig(int s) { (void)s; stop = 1; }

static void reg_global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t v) {
    (void)d; (void)v;
    if (!strcmp(iface, wl_compositor_interface.name)) comp = wl_registry_bind(r, name, &wl_compositor_interface, 4);
    else if (!strcmp(iface, xdg_wm_base_interface.name)) wm = wl_registry_bind(r, name, &xdg_wm_base_interface, 1);
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t n) { (void)d; (void)r; (void)n; }
static const struct wl_registry_listener reg_l = { reg_global, reg_remove };
static void wm_ping(void *d, struct xdg_wm_base *b, uint32_t s) { (void)d; xdg_wm_base_pong(b, s); }
static const struct xdg_wm_base_listener wm_l = { wm_ping };
static void xs_conf(void *d, struct xdg_surface *s, uint32_t serial) { (void)d; xdg_surface_ack_configure(s, serial); configured = 1; }
static const struct xdg_surface_listener xs_l = { xs_conf };
static void tl_conf(void *d, struct xdg_toplevel *t, int32_t w, int32_t h, struct wl_array *st) {
    (void)d; (void)t; (void)st; if (w > 0 && h > 0) { cw = w; ch = h; }
}
static void tl_close(void *d, struct xdg_toplevel *t) { (void)d; (void)t; stop = 1; }
static const struct xdg_toplevel_listener tl_l = { tl_conf, tl_close };
static void fr_done(void *d, struct wl_callback *cb, uint32_t t) { (void)d; (void)t; wl_callback_destroy(cb); frame_done = 1; }
static const struct wl_callback_listener fr_l = { fr_done };

static const float P[9][3] = {
    {-0.9f, -0.8f, 0.2f}, { 0.9f, -0.6f, 0.8f}, { 0.0f,  0.9f, 0.5f},
    {-0.9f, -0.6f, 0.8f}, { 0.9f, -0.8f, 0.2f}, { 0.0f,  0.8f, 0.5f},
    {-0.3f, -0.3f, 0.4f}, { 0.3f, -0.3f, 0.6f}, { 0.0f,  0.4f, 0.1f}};
static const float C[3][3] = {{1.0f, 0.2f, 0.1f}, {0.1f, 1.0f, 0.2f}, {0.2f, 0.3f, 1.0f}};
static const char *VS = "attribute vec3 p; attribute vec3 c; varying vec3 v;\n"
                        "void main(){ v = c; gl_Position = vec4(p.xy, 2.0 * p.z - 1.0, 1.0); }\n";
static const char *FS = "precision highp float; varying vec3 v;\n"
                        "void main(){ gl_FragColor = vec4(v * (0.6 + 0.4 * gl_FragCoord.z), 1.0); }\n";
static GLuint sh(GLenum t, const char *s) {
    GLuint o = glCreateShader(t); glShaderSource(o, 1, &s, 0); glCompileShader(o);
    GLint ok = 0; glGetShaderiv(o, GL_COMPILE_STATUS, &ok); if (!ok) FAIL("shader compile"); return o;
}

int main(int argc, char **argv) {
    int hold = argc > 1 ? atoi(argv[1]) : 60;
    signal(SIGTERM, on_sig); signal(SIGINT, on_sig);
    struct wl_display *dpy = wl_display_connect(NULL);
    if (!dpy) FAIL("wl_display_connect (WAYLAND_DISPLAY=%s)", getenv("WAYLAND_DISPLAY") ? getenv("WAYLAND_DISPLAY") : "-");
    struct wl_registry *reg = wl_display_get_registry(dpy);
    wl_registry_add_listener(reg, &reg_l, NULL); wl_display_roundtrip(dpy);
    if (!comp || !wm) FAIL("compositor lacks wl_compositor/xdg_wm_base");
    xdg_wm_base_add_listener(wm, &wm_l, NULL);
    struct wl_surface *surf = wl_compositor_create_surface(comp);
    struct xdg_surface *xs = xdg_wm_base_get_xdg_surface(wm, surf);
    xdg_surface_add_listener(xs, &xs_l, NULL);
    struct xdg_toplevel *tl = xdg_surface_get_toplevel(xs);
    xdg_toplevel_add_listener(tl, &tl_l, NULL);
    xdg_toplevel_set_app_id(tl, "gset"); xdg_toplevel_set_title(tl, "gset");
    wl_surface_commit(surf);
    while (!configured && wl_display_dispatch(dpy) != -1) {}
    if (cw <= 0 || ch <= 0) { cw = 512; ch = 384; }   // the compositor left the size to us
    printf("WL_SCENE_SIZE %dx%d\n", cw, ch);

    EGLDisplay ed = eglGetPlatformDisplay(EGL_PLATFORM_WAYLAND_KHR, dpy, NULL);
    if (ed == EGL_NO_DISPLAY || !eglInitialize(ed, NULL, NULL)) FAIL("eglInitialize 0x%x", eglGetError());
    printf("WL_SCENE_EGL_VENDOR %s\n", eglQueryString(ed, EGL_VENDOR));
    eglBindAPI(EGL_OPENGL_ES_API);
    EGLint ca[] = { EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_RED_SIZE, 8,
                    EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 0, EGL_DEPTH_SIZE, 24, EGL_NONE };
    EGLConfig cfg; EGLint n = 0;
    if (!eglChooseConfig(ed, ca, &cfg, 1, &n) || n < 1) FAIL("eglChooseConfig");
    EGLint xa[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
    EGLContext ctx = eglCreateContext(ed, cfg, EGL_NO_CONTEXT, xa);
    struct wl_egl_window *ew = wl_egl_window_create(surf, cw, ch);
    EGLSurface es = eglCreateWindowSurface(ed, cfg, (EGLNativeWindowType)ew, NULL);
    if (ctx == EGL_NO_CONTEXT || es == EGL_NO_SURFACE || !eglMakeCurrent(ed, es, es, ctx)) FAIL("context/surface 0x%x", eglGetError());
    printf("WL_SCENE_RENDERER %s\n", glGetString(GL_RENDERER));
    eglSwapInterval(ed, 0);

    GLuint pr = glCreateProgram(); glAttachShader(pr, sh(GL_VERTEX_SHADER, VS)); glAttachShader(pr, sh(GL_FRAGMENT_SHADER, FS));
    glBindAttribLocation(pr, 0, "p"); glBindAttribLocation(pr, 1, "c"); glLinkProgram(pr); glUseProgram(pr);
    float vd[9 * 6];
    for (int i = 0; i < 9; i++) { memcpy(&vd[i * 6], P[i], 12); memcpy(&vd[i * 6 + 3], C[i / 3], 12); }
    glEnableVertexAttribArray(0); glVertexAttribPointer(0, 3, GL_FLOAT, 0, 24, vd);
    glEnableVertexAttribArray(1); glVertexAttribPointer(1, 3, GL_FLOAT, 0, 24, vd + 3);
    glViewport(0, 0, cw, ch);
    glEnable(GL_DEPTH_TEST); glDepthFunc(GL_LESS);
    glClearColor(0.1f, 0.2f, 0.3f, 1.0f); glClearDepthf(1.0f);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    glDrawArrays(GL_TRIANGLES, 0, 9);
    glFinish();
    uint8_t *px = malloc((size_t)cw * ch * 4);
    glReadPixels(0, 0, cw, ch, GL_RGBA, GL_UNSIGNED_BYTE, px);
    uint64_t h = 1469598103934665603ull; int distinct = 0; uint32_t seen[8] = {0};
    for (size_t i = 0; i < (size_t)cw * ch * 4; i++) { h ^= px[i]; h *= 1099511628211ull; }
    for (size_t i = 0; i < (size_t)cw * ch && distinct < 8; i += 97) {
        uint32_t c = px[4 * i] | px[4 * i + 1] << 8 | px[4 * i + 2] << 16; int f = 0;
        for (int k = 0; k < distinct; k++) f |= seen[k] == c;
        if (!f) seen[distinct++] = c;
    }
    printf("WL_SCENE_HASH %016llx\nWL_SCENE_DISTINCT %d\nWL_SCENE_GLERR 0x%x\n", (unsigned long long)h, distinct, glGetError());
    struct wl_callback *cb = wl_surface_frame(surf); wl_callback_add_listener(cb, &fr_l, NULL);
    if (!eglSwapBuffers(ed, es)) FAIL("eglSwapBuffers 0x%x", eglGetError());
    time_t t0 = time(NULL);
    while (!frame_done && !stop && time(NULL) - t0 < 30 && wl_display_dispatch(dpy) != -1) {}
    if (!frame_done) FAIL("no frame callback in 30 s (the compositor never used the frame)");
    printf("WL_SCENE_READY\n"); fflush(stdout);
    // stay mapped (static content) until told to go
    while (!stop && time(NULL) - t0 < hold) {
        struct timespec ts = { 0, 100000000 }; nanosleep(&ts, NULL);
        wl_display_dispatch_pending(dpy); wl_display_flush(dpy);
    }
    printf("WL_SCENE_DONE\n");
    return 0;
}
