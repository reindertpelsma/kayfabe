/*
 * nvkvm validation -- offscreen EGL / OpenGL ES probe.
 *
 * Uses EGL_EXT_platform_device so it needs no X server and no Wayland
 * compositor -- it binds the NVIDIA EGL device directly. dlopen()s
 * libEGL.so.1 and libGLESv2.so.2 and hand-rolls the types, so no EGL/GLES
 * headers are needed to build.
 *
 * NOTE ON THE PIXEL CHECK. It is not enough to render and observe "some
 * non-black pixels" -- this project has already been burned by a checker that
 * reported "100% non-black" while reading a compositor's own background. So
 * this probe renders into an FBO IT ALLOCATED, clears it to an exact colour,
 * draws a triangle covering only the lower-left half in a DIFFERENT exact
 * colour, and then asserts BOTH:
 *     a pixel inside the triangle  == the triangle colour
 *     a pixel outside the triangle == the clear colour
 * Reading anyone else's framebuffer cannot satisfy both.
 */
#define _GNU_SOURCE
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <ctype.h>
#include <unistd.h>
#include <dlfcn.h>

typedef void *EGLDisplay, *EGLConfig, *EGLSurface, *EGLContext, *EGLDeviceEXT;
typedef unsigned int EGLenum, EGLBoolean;
typedef int EGLint;

#define EGL_NO_DISPLAY      ((EGLDisplay)0)
#define EGL_NO_CONTEXT      ((EGLContext)0)
#define EGL_NO_SURFACE      ((EGLSurface)0)
#define EGL_PLATFORM_DEVICE_EXT 0x313F
#define EGL_OPENGL_ES_API   0x30A0
#define EGL_NONE            0x3038
#define EGL_SURFACE_TYPE    0x3033
#define EGL_PBUFFER_BIT     0x0001
#define EGL_RENDERABLE_TYPE 0x3040
#define EGL_OPENGL_ES2_BIT  0x0004
#define EGL_RED_SIZE        0x3024
#define EGL_GREEN_SIZE      0x3023
#define EGL_BLUE_SIZE       0x3022
#define EGL_ALPHA_SIZE      0x3021
#define EGL_CONTEXT_CLIENT_VERSION 0x3098
#define EGL_WIDTH           0x3057
#define EGL_HEIGHT          0x3056

#define GL_RENDERER   0x1F01
#define GL_VENDOR     0x1F00
#define GL_VERSION    0x1F02
#define GL_COLOR_BUFFER_BIT 0x00004000
#define GL_FRAMEBUFFER      0x8D40
#define GL_RENDERBUFFER     0x8D41
#define GL_RGBA8            0x8058
#define GL_TEXTURE_2D       0x0DE1
#define GL_TEXTURE_MIN_FILTER 0x2801
#define GL_TEXTURE_MAG_FILTER 0x2800
#define GL_NEAREST          0x2600
#define GL_COLOR_ATTACHMENT0 0x8CE0
#define GL_FRAMEBUFFER_COMPLETE 0x8CD5
#define GL_RGBA             0x1908
#define GL_UNSIGNED_BYTE    0x1401
#define GL_VERTEX_SHADER    0x8B31
#define GL_FRAGMENT_SHADER  0x8B30
#define GL_COMPILE_STATUS   0x8B81
#define GL_LINK_STATUS      0x8B82
#define GL_FLOAT            0x1406
#define GL_TRIANGLES        0x0004
#define GL_NO_ERROR         0
#define GL_FALSE            0

static const char *ALL_CHECKS[] = { "egl_loader", "egl_context", "gl_renderer",
                                    "gl_renderer_is_nvidia", "gl_draw_pixel_check", NULL };
static int reported[16];
static void emit(const char *name, const char *status, const char *fmt, ...) {
    char buf[1024]; va_list ap; va_start(ap, fmt); vsnprintf(buf, sizeof buf, fmt, ap); va_end(ap);
    int i; for (i = 0; ALL_CHECKS[i]; i++) if (!strcmp(ALL_CHECKS[i], name)) reported[i] = 1;
    printf("CHECK|%s|%s|%s\n", name, status, buf); fflush(stdout);
}
static void finish(const char *reason) {
    int i; for (i = 0; ALL_CHECKS[i]; i++) if (!reported[i]) printf("CHECK|%s|SKIP|%s\n", ALL_CHECKS[i], reason);
    fflush(stdout);
}

/* see the note in the CUDA probe: body + _exit(), never a normal return */
static int probe_main(void) {
    void *E = dlopen("libEGL.so.1", RTLD_NOW);
    if (!E) E = dlopen("libEGL.so", RTLD_NOW);
    if (!E) { emit("egl_loader", "SKIP", "dlopen(libEGL.so.1): %s -- install libegl1 in the guest", dlerror());
              finish("no EGL loader"); return 0; }
    void *G = dlopen("libGLESv2.so.2", RTLD_NOW);
    if (!G) G = dlopen("libGLESv2.so", RTLD_NOW);
    if (!G) { emit("egl_loader", "SKIP", "dlopen(libGLESv2.so.2): %s -- install libgles2 in the guest", dlerror());
              finish("no GLESv2"); return 0; }

    void *(*eglGetProcAddress)(const char *) = dlsym(E, "eglGetProcAddress");
    EGLBoolean (*eglInitialize)(EGLDisplay, EGLint *, EGLint *) = dlsym(E, "eglInitialize");
    EGLBoolean (*eglChooseConfig)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *) = dlsym(E, "eglChooseConfig");
    EGLBoolean (*eglBindAPI)(EGLenum) = dlsym(E, "eglBindAPI");
    EGLContext (*eglCreateContext)(EGLDisplay, EGLConfig, EGLContext, const EGLint *) = dlsym(E, "eglCreateContext");
    EGLSurface (*eglCreatePbufferSurface)(EGLDisplay, EGLConfig, const EGLint *) = dlsym(E, "eglCreatePbufferSurface");
    EGLBoolean (*eglMakeCurrent)(EGLDisplay, EGLSurface, EGLSurface, EGLContext) = dlsym(E, "eglMakeCurrent");
    EGLint (*eglGetError)(void) = dlsym(E, "eglGetError");
    EGLDisplay (*eglGetDisplay)(void *) = dlsym(E, "eglGetDisplay");

    if (!eglInitialize || !eglChooseConfig || !eglCreateContext || !eglMakeCurrent) {
        emit("egl_loader", "FAIL", "libEGL.so.1 loaded but core symbols missing");
        finish("incomplete EGL"); return 1;
    }
    { Dl_info di;
      if (dladdr((void *)eglInitialize, &di) && di.dli_fname) emit("egl_loader", "PASS", "%s", di.dli_fname);
      else emit("egl_loader", "PASS", "libEGL.so.1 loaded"); }

    /* Prefer EGL_EXT_platform_device: binds the NVIDIA GPU with no display
       server at all. Fall back to eglGetDisplay(NULL) only if unavailable. */
    EGLBoolean (*eglQueryDevicesEXT)(EGLint, EGLDeviceEXT *, EGLint *) =
        eglGetProcAddress ? eglGetProcAddress("eglQueryDevicesEXT") : NULL;
    EGLDisplay (*eglGetPlatformDisplayEXT)(EGLenum, void *, const EGLint *) =
        eglGetProcAddress ? eglGetProcAddress("eglGetPlatformDisplayEXT") : NULL;

    EGLDisplay dpy = EGL_NO_DISPLAY;
    char how[128] = "";
    if (eglQueryDevicesEXT && eglGetPlatformDisplayEXT) {
        EGLDeviceEXT devs[16]; EGLint nd = 0;
        if (eglQueryDevicesEXT(16, devs, &nd) && nd > 0) {
            EGLint i;
            for (i = 0; i < nd; i++) {
                EGLDisplay d = eglGetPlatformDisplayEXT(EGL_PLATFORM_DEVICE_EXT, devs[i], NULL);
                if (d != EGL_NO_DISPLAY) {
                    EGLint maj = 0, min = 0;
                    if (eglInitialize(d, &maj, &min)) {
                        dpy = d; snprintf(how, sizeof how, "EGL_EXT_platform_device dev %d/%d, EGL %d.%d", i, nd, maj, min);
                        break;
                    }
                }
            }
        }
    }
    if (dpy == EGL_NO_DISPLAY && eglGetDisplay) {
        EGLDisplay d = eglGetDisplay(NULL);
        EGLint maj = 0, min = 0;
        if (d != EGL_NO_DISPLAY && eglInitialize(d, &maj, &min)) {
            dpy = d; snprintf(how, sizeof how, "eglGetDisplay(EGL_DEFAULT_DISPLAY), EGL %d.%d", maj, min);
        }
    }
    if (dpy == EGL_NO_DISPLAY) {
        emit("egl_context", "FAIL", "no EGL display could be initialised (eglGetError=0x%X)",
             eglGetError ? eglGetError() : 0);
        finish("no EGL display"); return 1;
    }

    eglBindAPI(EGL_OPENGL_ES_API);
    EGLint cfgattr[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
                         EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
                         EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
                         EGL_NONE };
    EGLConfig cfg; EGLint ncfg = 0;
    if (!eglChooseConfig(dpy, cfgattr, &cfg, 1, &ncfg) || ncfg < 1) {
        emit("egl_context", "FAIL", "eglChooseConfig returned %d configs (eglGetError=0x%X)",
             ncfg, eglGetError ? eglGetError() : 0);
        finish("no EGL config"); return 1;
    }
    EGLint ctxattr[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
    EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctxattr);
    if (ctx == EGL_NO_CONTEXT) {
        emit("egl_context", "FAIL", "eglCreateContext failed (eglGetError=0x%X)", eglGetError ? eglGetError() : 0);
        finish("no EGL context"); return 1;
    }
    EGLint pbattr[] = { EGL_WIDTH, 64, EGL_HEIGHT, 64, EGL_NONE };
    EGLSurface surf = eglCreatePbufferSurface ? eglCreatePbufferSurface(dpy, cfg, pbattr) : EGL_NO_SURFACE;
    if (!eglMakeCurrent(dpy, surf, surf, ctx)) {
        /* surfaceless is fine too if the implementation supports it */
        if (!eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, ctx)) {
            emit("egl_context", "FAIL", "eglMakeCurrent failed (eglGetError=0x%X)", eglGetError ? eglGetError() : 0);
            finish("no current context"); return 1;
        }
        snprintf(how + strlen(how), sizeof how - strlen(how), ", surfaceless");
    }
    emit("egl_context", "PASS", "%s", how);

    const unsigned char *(*glGetString)(unsigned) = dlsym(G, "glGetString");
    if (!glGetString) { emit("gl_renderer", "FAIL", "glGetString not found in libGLESv2"); finish("x"); return 1; }
    const char *rend = (const char *)glGetString(GL_RENDERER);
    const char *vend = (const char *)glGetString(GL_VENDOR);
    const char *ver  = (const char *)glGetString(GL_VERSION);
    if (!rend || !*rend) { emit("gl_renderer", "FAIL", "GL_RENDERER is empty"); finish("x"); return 1; }
    emit("gl_renderer", "PASS", "GL_RENDERER='%s' GL_VENDOR='%s' GL_VERSION='%s'",
         rend, vend ? vend : "?", ver ? ver : "?");

    char lower[512]; size_t k;
    for (k = 0; k < sizeof lower - 1 && rend[k]; k++) lower[k] = (char)tolower((unsigned char)rend[k]);
    lower[k] = 0;
    if (strstr(lower, "llvmpipe") || strstr(lower, "softpipe") || strstr(lower, "swrast") ||
        strstr(lower, "software rasterizer")) {
        emit("gl_renderer_is_nvidia", "FAIL", "SOFTWARE RASTERISER: GL_RENDERER='%s'", rend);
        finish("software GL"); return 1;
    }
    /*
     * GL_RENDERER is not a vendor field.  Consumer parts spell it
     * "NVIDIA GeForce RTX 4070/PCIe/SSE2", but DATACENTER parts drop the
     * prefix entirely -- a Tesla T4 reports "Tesla T4/PCIe/SSE2" while
     * GL_VENDOR still reads "NVIDIA Corporation" and GL_VERSION still reads
     * "OpenGL ES 3.2 NVIDIA 580.178.04".  Matching on the renderer alone
     * therefore failed every Tesla/A-series/H-series card as "non-NVIDIA",
     * and took gl_draw_pixel_check down with it as an unexpected SKIP.
     * Measured on 6x Tesla T4 / 580.178.04, 2026-08-22.
     *
     * GL_VENDOR is the field that actually names the vendor; the software
     * rasteriser test above stays on GL_RENDERER, which is where llvmpipe
     * and friends identify themselves.
     */
    char vlower[512]; size_t vk_;
    const char *vsrc = vend ? vend : "";
    for (vk_ = 0; vk_ < sizeof vlower - 1 && vsrc[vk_]; vk_++)
        vlower[vk_] = (char)tolower((unsigned char)vsrc[vk_]);
    vlower[vk_] = 0;
    if (!strstr(lower, "nvidia") && !strstr(vlower, "nvidia")) {
        emit("gl_renderer_is_nvidia", "FAIL",
             "neither GL_RENDERER='%s' nor GL_VENDOR='%s' names NVIDIA",
             rend, vend ? vend : "?");
        finish("non-NVIDIA GL"); return 1;
    }
    emit("gl_renderer_is_nvidia", "PASS", "%s", rend);

    /* ---- render + exact pixel check ------------------------------------- */
    void (*glGenFramebuffers)(int, unsigned *) = dlsym(G, "glGenFramebuffers");
    void (*glBindFramebuffer)(unsigned, unsigned) = dlsym(G, "glBindFramebuffer");
    void (*glGenTextures)(int, unsigned *) = dlsym(G, "glGenTextures");
    void (*glBindTexture)(unsigned, unsigned) = dlsym(G, "glBindTexture");
    void (*glTexImage2D)(unsigned, int, int, int, int, int, unsigned, unsigned, const void *) = dlsym(G, "glTexImage2D");
    void (*glTexParameteri)(unsigned, unsigned, int) = dlsym(G, "glTexParameteri");
    void (*glFramebufferTexture2D)(unsigned, unsigned, unsigned, unsigned, int) = dlsym(G, "glFramebufferTexture2D");
    unsigned (*glCheckFramebufferStatus)(unsigned) = dlsym(G, "glCheckFramebufferStatus");
    void (*glViewport)(int, int, int, int) = dlsym(G, "glViewport");
    void (*glClearColor)(float, float, float, float) = dlsym(G, "glClearColor");
    void (*glClear)(unsigned) = dlsym(G, "glClear");
    unsigned (*glCreateShader)(unsigned) = dlsym(G, "glCreateShader");
    void (*glShaderSource)(unsigned, int, const char *const *, const int *) = dlsym(G, "glShaderSource");
    void (*glCompileShader)(unsigned) = dlsym(G, "glCompileShader");
    void (*glGetShaderiv)(unsigned, unsigned, int *) = dlsym(G, "glGetShaderiv");
    void (*glGetShaderInfoLog)(unsigned, int, int *, char *) = dlsym(G, "glGetShaderInfoLog");
    unsigned (*glCreateProgram)(void) = dlsym(G, "glCreateProgram");
    void (*glAttachShader)(unsigned, unsigned) = dlsym(G, "glAttachShader");
    void (*glLinkProgram)(unsigned) = dlsym(G, "glLinkProgram");
    void (*glGetProgramiv)(unsigned, unsigned, int *) = dlsym(G, "glGetProgramiv");
    void (*glUseProgram)(unsigned) = dlsym(G, "glUseProgram");
    int  (*glGetAttribLocation)(unsigned, const char *) = dlsym(G, "glGetAttribLocation");
    void (*glEnableVertexAttribArray)(unsigned) = dlsym(G, "glEnableVertexAttribArray");
    void (*glVertexAttribPointer)(unsigned, int, unsigned, unsigned char, int, const void *) = dlsym(G, "glVertexAttribPointer");
    void (*glDrawArrays)(unsigned, int, int) = dlsym(G, "glDrawArrays");
    void (*glReadPixels)(int, int, int, int, unsigned, unsigned, void *) = dlsym(G, "glReadPixels");
    void (*glFinish)(void) = dlsym(G, "glFinish");
    unsigned (*glGetError)(void) = dlsym(G, "glGetError");

    if (!glGenFramebuffers || !glDrawArrays || !glReadPixels || !glCreateShader ||
        !glGenTextures || !glTexImage2D || !glFramebufferTexture2D) {
        emit("gl_draw_pixel_check", "SKIP", "libGLESv2 is missing FBO/shader entry points");
        finish("incomplete GLES2"); return 1;
    }

    /* Colour attachment is a TEXTURE, not a renderbuffer: GL_RGBA8
     * renderbuffer storage needs OES_rgb8_rgba8 and is NOT core GLES2, and it
     * really is absent on some stacks -- driver 610 returned
     * GL_FRAMEBUFFER_UNSUPPORTED (0x8CDD) for exactly that. An
     * RGBA/UNSIGNED_BYTE texture is core GLES2 and always colour-renderable. */
    const int W = 64, Hh = 64;
    unsigned fbo = 0, tex = 0;
    glGenFramebuffers(1, &fbo); glBindFramebuffer(GL_FRAMEBUFFER, fbo);
    glGenTextures(1, &tex); glBindTexture(GL_TEXTURE_2D, tex);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, Hh, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    unsigned fbs = glCheckFramebufferStatus(GL_FRAMEBUFFER);
    if (fbs != GL_FRAMEBUFFER_COMPLETE) {
        emit("gl_draw_pixel_check", "FAIL", "FBO incomplete: status=0x%X", fbs);
        finish("x"); return 1;
    }

    static const char *VS = "attribute vec2 pos;\nvoid main(){ gl_Position = vec4(pos,0.0,1.0); }\n";
    static const char *FS = "precision highp float;\nvoid main(){ gl_FragColor = vec4(1.0,0.5,0.0,1.0); }\n";
    unsigned vs = glCreateShader(GL_VERTEX_SHADER), fs = glCreateShader(GL_FRAGMENT_SHADER);
    int ok = 0; char log[512];
    glShaderSource(vs, 1, &VS, NULL); glCompileShader(vs);
    glGetShaderiv(vs, GL_COMPILE_STATUS, &ok);
    if (!ok) { glGetShaderInfoLog(vs, sizeof log, NULL, log);
               emit("gl_draw_pixel_check", "FAIL", "vertex shader compile failed: %s", log); finish("x"); return 1; }
    glShaderSource(fs, 1, &FS, NULL); glCompileShader(fs);
    glGetShaderiv(fs, GL_COMPILE_STATUS, &ok);
    if (!ok) { glGetShaderInfoLog(fs, sizeof log, NULL, log);
               emit("gl_draw_pixel_check", "FAIL", "fragment shader compile failed: %s", log); finish("x"); return 1; }
    unsigned prog = glCreateProgram();
    glAttachShader(prog, vs); glAttachShader(prog, fs); glLinkProgram(prog);
    glGetProgramiv(prog, GL_LINK_STATUS, &ok);
    if (!ok) { emit("gl_draw_pixel_check", "FAIL", "program link failed"); finish("x"); return 1; }
    glUseProgram(prog);

    glViewport(0, 0, W, Hh);
    /* clear colour: exact 0,0,0,255 */
    glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);

    /* triangle covering the LOWER-LEFT half only */
    static const float verts[] = { -1.0f, -1.0f,  1.0f, -1.0f,  -1.0f, 1.0f };
    int loc = glGetAttribLocation(prog, "pos");
    if (loc < 0) { emit("gl_draw_pixel_check", "FAIL", "attribute 'pos' not found"); finish("x"); return 1; }
    glEnableVertexAttribArray((unsigned)loc);
    glVertexAttribPointer((unsigned)loc, 2, GL_FLOAT, 0, 0, verts);
    glDrawArrays(GL_TRIANGLES, 0, 3);
    glFinish();

    unsigned gerr = glGetError();
    if (gerr != GL_NO_ERROR) {
        emit("gl_draw_pixel_check", "FAIL", "glGetError=0x%X after draw", gerr);
        finish("x"); return 1;
    }

    unsigned char *px = malloc((size_t)W * Hh * 4);
    glReadPixels(0, 0, W, Hh, GL_RGBA, GL_UNSIGNED_BYTE, px);
    gerr = glGetError();
    if (gerr != GL_NO_ERROR) {
        emit("gl_draw_pixel_check", "FAIL", "glGetError=0x%X after glReadPixels", gerr);
        finish("x"); return 1;
    }

    /* inside the triangle (lower-left quadrant) and outside it (upper-right) */
    int ix = W / 4,      iy = Hh / 4;
    int ox = (3 * W) / 4, oy = (3 * Hh) / 4;
    unsigned char *pin  = px + ((size_t)iy * W + ix) * 4;
    unsigned char *pout = px + ((size_t)oy * W + ox) * 4;

    /* shader writes (1.0, 0.5, 0.0, 1.0) -> (255, 127|128, 0, 255) */
    int in_ok  = (pin[0] >= 253) && (pin[1] >= 125 && pin[1] <= 130) && (pin[2] <= 2) && (pin[3] >= 253);
    int out_ok = (pout[0] <= 2) && (pout[1] <= 2) && (pout[2] <= 2) && (pout[3] >= 253);

    if (in_ok && out_ok) {
        emit("gl_draw_pixel_check", "PASS",
             "64x64 FBO: inside(%d,%d)=RGBA(%u,%u,%u,%u)==triangle, outside(%d,%d)=RGBA(%u,%u,%u,%u)==clear",
             ix, iy, pin[0], pin[1], pin[2], pin[3], ox, oy, pout[0], pout[1], pout[2], pout[3]);
    } else {
        emit("gl_draw_pixel_check", "FAIL",
             "inside(%d,%d)=RGBA(%u,%u,%u,%u) expected ~(255,128,0,255) [%s]; "
             "outside(%d,%d)=RGBA(%u,%u,%u,%u) expected (0,0,0,255) [%s]",
             ix, iy, pin[0], pin[1], pin[2], pin[3], in_ok ? "ok" : "BAD",
             ox, oy, pout[0], pout[1], pout[2], pout[3], out_ok ? "ok" : "BAD");
    }

    finish("not reached");
    return 0;
}

int main(void) { int rc = probe_main(); fflush(NULL); _exit(rc); }
