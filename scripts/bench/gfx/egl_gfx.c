// egl_gfx — headless desktop GL through the EGL device platform (no X, no Wayland, no DRM master).
//
// The same three-triangle scene as vk_gfx render, through the GL driver:
//   FBO (RGBA8 texture + DEPTH24_STENCIL8 renderbuffer), depth test LESS  → colour A, depth Z
//   pbuffer (default framebuffer): fullscreen quad sampling A transposed  → B
// then glReadPixels of A, Z and B, FNV-1a 64 hashes, and the refcheck.h CPU reference.
// Prints EGL_RENDERER=, EGLR_HASH_{A,Z,B}=, EGLR_REF_FAILS=.
//
// Built by build_gfx.sh: gcc egl_gfx.c -lEGL -lOpenGL -lm
#define GL_GLEXT_PROTOTYPES 1
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GL/gl.h>
#include <GL/glext.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "refcheck.h"

#define W RW
#define H RH
static uint64_t fnv(const void *p, size_t n) {
    const uint8_t *b = p; uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 1099511628211ull; }
    return h;
}
#define DIE(...) do { printf("EGL_FAIL " __VA_ARGS__); printf(" (eglGetError=0x%x)\n", eglGetError()); exit(3); } while (0)

int main(void) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("EGL_START\n");
    PFNEGLQUERYDEVICESEXTPROC qd = (void *)eglGetProcAddress("eglQueryDevicesEXT");
    PFNEGLQUERYDEVICESTRINGEXTPROC qs = (void *)eglGetProcAddress("eglQueryDeviceStringEXT");
    PFNEGLGETPLATFORMDISPLAYEXTPROC gpd = (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
    if (!qd || !gpd) DIE("no EGL_EXT_device_enumeration / platform_base");
    EGLDeviceEXT devs[8]; EGLint n = 0;
    if (!qd(8, devs, &n) || n < 1) DIE("eglQueryDevicesEXT found %d devices", n);
    EGLDisplay d = EGL_NO_DISPLAY;
    for (int i = 0; i < n; i++) {
        const char *ext = qs ? qs(devs[i], EGL_EXTENSIONS) : NULL;
        const char *drm = (qs && ext && strstr(ext, "EGL_EXT_device_drm")) ? qs(devs[i], 0x3233 /* EGL_DRM_DEVICE_FILE_EXT */) : NULL;
        printf("EGL_DEVICE[%d] drm=%s ext=%s\n", i, drm ? drm : "-", ext ? ext : "-");
        if (d == EGL_NO_DISPLAY) {
            EGLDisplay t = gpd(EGL_PLATFORM_DEVICE_EXT, devs[i], NULL);
            EGLint ma, mi;
            if (t != EGL_NO_DISPLAY && eglInitialize(t, &ma, &mi)) {
                const char *v = eglQueryString(t, EGL_VENDOR);
                printf("EGL_DISPLAY[%d] vendor=%s version=%d.%d\n", i, v ? v : "?", ma, mi);
                if (v && strstr(v, "NVIDIA")) d = t; else eglTerminate(t);
            }
        }
    }
    if (d == EGL_NO_DISPLAY) DIE("no NVIDIA EGL device display");
    EGLint ca[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
                    EGL_ALPHA_SIZE, 8, EGL_DEPTH_SIZE, 24, EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE };
    EGLConfig cfg; EGLint nc = 0;
    if (!eglChooseConfig(d, ca, &cfg, 1, &nc) || nc < 1) DIE("eglChooseConfig");
    EGLint pa[] = { EGL_WIDTH, W, EGL_HEIGHT, H, EGL_NONE };
    EGLSurface s = eglCreatePbufferSurface(d, cfg, pa);
    if (s == EGL_NO_SURFACE) DIE("eglCreatePbufferSurface");
    if (!eglBindAPI(EGL_OPENGL_API)) DIE("eglBindAPI");
    EGLContext c = eglCreateContext(d, cfg, EGL_NO_CONTEXT, NULL);
    if (c == EGL_NO_CONTEXT) DIE("eglCreateContext");
    if (!eglMakeCurrent(d, s, s, c)) DIE("eglMakeCurrent");
    printf("EGL_RENDERER=%s\nEGL_GL_VERSION=%s\n", glGetString(GL_RENDERER), glGetString(GL_VERSION));
    printf("EGL_STAGE=context_current\n");

    GLuint tex, rb, fbo;
    glGenTextures(1, &tex); glBindTexture(GL_TEXTURE_2D, tex);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glGenRenderbuffers(1, &rb); glBindRenderbuffer(GL_RENDERBUFFER, rb);
    glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, W, H);
    glGenFramebuffers(1, &fbo); glBindFramebuffer(GL_FRAMEBUFFER, fbo);
    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
    GLenum st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
    if (st != GL_FRAMEBUFFER_COMPLETE) { printf("EGL_FAIL fbo status 0x%x\n", st); return 3; }

    // pass 1 — the scene, GL NDC z = 2z-1 so window depth equals refcheck's z.
    glViewport(0, 0, W, H);
    glClearColor(RCLEAR[0], RCLEAR[1], RCLEAR[2], 1.0f); glClearDepth(1.0);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    glEnable(GL_DEPTH_TEST); glDepthFunc(GL_LESS); glDisable(GL_CULL_FACE); glShadeModel(GL_SMOOTH);
    glBegin(GL_TRIANGLES);
    for (int v = 0; v < 9; v++) {
        float k = 0.6f + 0.4f * RP[v][2];
        glColor3f(RC[v / 3][0] * k, RC[v / 3][1] * k, RC[v / 3][2] * k);
        glVertex3f(RP[v][0], RP[v][1], 2.0f * RP[v][2] - 1.0f);
    }
    glEnd();
    glFinish();
    static uint8_t a[W * H * 4], b[W * H * 4]; static float z[W * H];
    glPixelStorei(GL_PACK_ALIGNMENT, 1);
    glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, a);
    glReadPixels(0, 0, W, H, GL_DEPTH_COMPONENT, GL_FLOAT, z);

    // pass 2 — into the pbuffer: B(x,y) samples A at (uy*0.97+0.01, ux*0.97+0.01), green halved.
    glBindFramebuffer(GL_FRAMEBUFFER, 0);
    glDisable(GL_DEPTH_TEST);
    glClearColor(0, 0, 0, 1); glClear(GL_COLOR_BUFFER_BIT);
    glEnable(GL_TEXTURE_2D); glBindTexture(GL_TEXTURE_2D, tex);
    glTexEnvi(GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, GL_MODULATE);
    glColor4f(1.0f, 0.5f, 1.0f, 1.0f);
    glBegin(GL_QUADS);
    glTexCoord2f(0.01f, 0.01f); glVertex2f(-1, -1);
    glTexCoord2f(0.01f, 0.98f); glVertex2f( 1, -1);
    glTexCoord2f(0.98f, 0.98f); glVertex2f( 1,  1);
    glTexCoord2f(0.98f, 0.01f); glVertex2f(-1,  1);
    glEnd();
    glFinish();
    glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, b);
    GLenum e = glGetError();
    if (e) printf("EGL_GL_ERROR=0x%x\n", e);

    // glReadPixels row 0 is the BOTTOM row = NDC y -1 — the convention refcheck.h assumes.
    int fails = ref_check("EGLR", a, z, b, 0.0f) + (e ? 1 : 0);
    printf("EGLR_HASH_A=%016llx\nEGLR_HASH_Z=%016llx\nEGLR_HASH_B=%016llx\nEGLR_REF_FAILS=%d\n",
           (unsigned long long)fnv(a, sizeof a), (unsigned long long)fnv(z, sizeof z),
           (unsigned long long)fnv(b, sizeof b), fails);
    eglMakeCurrent(d, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    eglTerminate(d);
    printf("EGL_DONE rc=%d\n", fails ? 1 : 0);
    return fails ? 1 : 0;
}
