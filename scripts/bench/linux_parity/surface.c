#define _POSIX_C_SOURCE 200809L
#define GL_GLEXT_PROTOTYPES 1
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GL/gl.h>
#include <GL/glext.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define W 1920
#define H 1080
#define FRAMES 1000
#define LAYERS 64
static double clock_s(clockid_t clock) {
    struct timespec t;
    clock_gettime(clock, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}
static void fail(const char *s) {
    fprintf(stderr, "SURFACE_FAIL %s egl=%x gl=%x\n", s, eglGetError(), glGetError());
    exit(1);
}
static void quad(float z) {
    glBegin(GL_QUADS);
    glTexCoord2f(0, 0); glVertex3f(-1, -1, z);
    glTexCoord2f(1, 0); glVertex3f(1, -1, z);
    glTexCoord2f(1, 1); glVertex3f(1, 1, z);
    glTexCoord2f(0, 1); glVertex3f(-1, 1, z);
    glEnd();
}
int main(void) {
    PFNEGLQUERYDEVICESEXTPROC qd = (void *)eglGetProcAddress("eglQueryDevicesEXT");
    PFNEGLGETPLATFORMDISPLAYEXTPROC pd =
        (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
    if (!qd || !pd) fail("device extension");
    EGLDeviceEXT dev[16];
    EGLint count = 0;
    if (!qd(16, dev, &count)) fail("device enumeration");
    EGLDisplay d = EGL_NO_DISPLAY;
    for (int i = 0; i < count && d == EGL_NO_DISPLAY; i++) {
        EGLDisplay t = pd(EGL_PLATFORM_DEVICE_EXT, dev[i], NULL);
        EGLint a, b;
        if (t != EGL_NO_DISPLAY && eglInitialize(t, &a, &b)) {
            const char *v = eglQueryString(t, EGL_VENDOR);
            if (v && strstr(v, "NVIDIA")) d = t;
            else eglTerminate(t);
        }
    }
    if (d == EGL_NO_DISPLAY) fail("NVIDIA EGL display");
    const EGLint attr[] = {
        EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8,
        EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE
    };
    EGLConfig config;
    EGLint n;
    if (!eglChooseConfig(d, attr, &config, 1, &n) || n != 1) fail("EGL config");
    const EGLint pa[] = {EGL_WIDTH, W, EGL_HEIGHT, H, EGL_NONE};
    EGLSurface s = eglCreatePbufferSurface(d, config, pa);
    if (s == EGL_NO_SURFACE || !eglBindAPI(EGL_OPENGL_API)) fail("pbuffer/API");
    EGLContext c = eglCreateContext(d, config, EGL_NO_CONTEXT, NULL);
    if (c == EGL_NO_CONTEXT || !eglMakeCurrent(d, s, s, c)) fail("context");
    printf("SURFACE_RENDERER=%s VERSION=%s\n", glGetString(GL_RENDERER),
           glGetString(GL_VERSION));
    if (!strstr((const char *)glGetString(GL_VENDOR), "NVIDIA")) fail("renderer");
    const char *extensions = (const char *)glGetString(GL_EXTENSIONS);
    if (!extensions || !strstr(extensions, "GL_ARB_timer_query")) fail("timer queries");
    GLuint tex[2], fbo[2], depth;
    glGenTextures(2, tex);
    glGenFramebuffers(2, fbo);
    glGenRenderbuffers(1, &depth);
    glBindRenderbuffer(GL_RENDERBUFFER, depth);
    glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT24, W, H);
    for (int i = 0; i < 2; i++) {
        glBindTexture(GL_TEXTURE_2D, tex[i]);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, W, H, 0, GL_RGBA,
                     GL_UNSIGNED_BYTE, NULL);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo[i]);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
                               GL_TEXTURE_2D, tex[i], 0);
        if (i == 0) glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT,
                                             GL_RENDERBUFFER, depth);
        if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
            fail("FBO complete");
    }
    glViewport(0, 0, W, H);
    glDisable(GL_DITHER);
    glDisable(GL_BLEND);
    glDepthFunc(GL_LESS);
    GLuint query[3];
    glGenQueries(3, query);
    double cpu[FRAMES], render[FRAMES], sample[FRAMES], start[FRAMES];
    double mono = clock_s(CLOCK_MONOTONIC), real = clock_s(CLOCK_REALTIME);
    printf("SURFACE_CLOCK mono=%.9f real=%.9f\n", mono, real);
    for (int frame = -20; frame < FRAMES; frame++) {
        double t0 = clock_s(CLOCK_MONOTONIC);
        glQueryCounter(query[0], GL_TIMESTAMP);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo[0]);
        glDisable(GL_TEXTURE_2D);
        glEnable(GL_DEPTH_TEST);
        glClearColor(0, 0, 0, 1);
        glClearDepth(1);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        for (int layer = 0; layer < LAYERS; layer++) {
            glColor4f(0.25f, 0.5f, 0.75f, 1);
            quad(0.9f - 1.8f * layer / (LAYERS - 1));
        }
        glQueryCounter(query[1], GL_TIMESTAMP);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo[1]);
        glDisable(GL_DEPTH_TEST);
        glEnable(GL_TEXTURE_2D);
        glBindTexture(GL_TEXTURE_2D, tex[0]);
        glColor4f(1, 1, 1, 1);
        quad(0);
        glQueryCounter(query[2], GL_TIMESTAMP);
        glFinish();
        double t1 = clock_s(CLOCK_MONOTONIC);
        GLuint64 ts[3];
        for (int i = 0; i < 3; i++)
            glGetQueryObjectui64v(query[i], GL_QUERY_RESULT, &ts[i]);
        if (ts[1] < ts[0] || ts[2] < ts[1]) fail("timer ordering");
        if (frame >= 0) {
            start[frame] = t0;
            cpu[frame] = (t1 - t0) * 1000;
            render[frame] = (ts[1] - ts[0]) * 1e-6;
            sample[frame] = (ts[2] - ts[1]) * 1e-6;
        }
        if (glGetError()) fail("render GL error");
    }
    double end_mono = clock_s(CLOCK_MONOTONIC), end_real = clock_s(CLOCK_REALTIME);
    unsigned char pixel[4];
    glReadPixels(W / 2, H / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, pixel);
    int expected[4] = {64, 128, 191, 255};
    for (int i = 0; i < 4; i++)
        if (abs((int)pixel[i] - expected[i]) > 1) fail("sampled color");
    glBindFramebuffer(GL_FRAMEBUFFER, fbo[0]);
    float z;
    glReadPixels(W / 2, H / 2, 1, 1, GL_DEPTH_COMPONENT, GL_FLOAT, &z);
    if (fabsf(z - 0.05f) > 0.0001f) fail("depth value");
    if (glGetError()) fail("readback GL error");
    printf("SURFACE_END_CLOCK mono=%.9f real=%.9f\n", end_mono, end_real);
    puts("SURFACE_COLUMNS frame,mono_start,cpu_ms,render_gpu_ms,sample_gpu_ms");
    for (int i = 0; i < FRAMES; i++)
        printf("SURFACE_FRAME %d,%.9f,%.6f,%.6f,%.6f\n", i, start[i], cpu[i],
               render[i], sample[i]);
    printf("SURFACE_OK frames=%d depth=%.8f rgba=%u,%u,%u,%u\n", FRAMES, z,
           pixel[0], pixel[1], pixel[2], pixel[3]);
    glDeleteQueries(3, query);
    eglMakeCurrent(d, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    eglDestroyContext(d, c);
    eglDestroySurface(d, s);
    eglTerminate(d);
    return 0;
}
