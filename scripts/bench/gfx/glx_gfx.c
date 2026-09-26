// glx_gfx — the three-triangle scene through GLX in an X window: the headless-DESKTOP check.
// Run under VirtualGL on Xvfb (`vglrun -d egl glx_gfx`): the GLX calls land on the GPU through
// VGL's EGL back end, so the readback grades the GPU's rendering (refcheck.h + host hash), which a
// frame counter such as glxgears' cannot (measured gfx7: frames>0 while every 3D channel faulted).
// Prints GLX_RENDERER=, GLXR_HASH_{A,Z,B}=, GLXR_REF_FAILS=.
#define GL_GLEXT_PROTOTYPES 1
#include <GL/gl.h>
#include <GL/glext.h>
#include <GL/glx.h>
#include <X11/Xlib.h>
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

int main(void) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("GLX_START\n");
    Display *dpy = XOpenDisplay(NULL);
    if (!dpy) { printf("GLX_FAIL no X display\n"); return 3; }
    int attr[] = { GLX_RGBA, GLX_DOUBLEBUFFER, GLX_RED_SIZE, 8, GLX_GREEN_SIZE, 8, GLX_BLUE_SIZE, 8, GLX_DEPTH_SIZE, 24, None };
    XVisualInfo *vi = glXChooseVisual(dpy, DefaultScreen(dpy), attr);
    if (!vi) { printf("GLX_FAIL glXChooseVisual\n"); return 3; }
    XSetWindowAttributes swa = { .colormap = XCreateColormap(dpy, RootWindow(dpy, vi->screen), vi->visual, AllocNone) };
    Window win = XCreateWindow(dpy, RootWindow(dpy, vi->screen), 0, 0, W, H, 0, vi->depth, InputOutput, vi->visual, CWColormap, &swa);
    XMapWindow(dpy, win);
    XSync(dpy, False);
    GLXContext ctx = glXCreateContext(dpy, vi, NULL, True);
    if (!ctx || !glXMakeCurrent(dpy, win, ctx)) { printf("GLX_FAIL context\n"); return 3; }
    printf("GLX_RENDERER=%s\nGLX_GL_VERSION=%s\n", glGetString(GL_RENDERER), glGetString(GL_VERSION));
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
    if (st != GL_FRAMEBUFFER_COMPLETE) { printf("GLX_FAIL fbo status 0x%x\n", st); return 3; }

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
    glReadBuffer(GL_BACK);
    glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, b);
    GLenum e = glGetError();
    if (e) printf("EGL_GL_ERROR=0x%x\n", e);

    // glReadPixels row 0 is the BOTTOM row = NDC y -1 — the convention refcheck.h assumes.
    int fails = ref_check("GLXR", a, z, b, 0.0f) + (e ? 1 : 0);
    printf("GLXR_HASH_A=%016llx\nGLXR_HASH_Z=%016llx\nGLXR_HASH_B=%016llx\nGLXR_REF_FAILS=%d\n",
           (unsigned long long)fnv(a, sizeof a), (unsigned long long)fnv(z, sizeof z),
           (unsigned long long)fnv(b, sizeof b), fails);
    glXSwapBuffers(dpy, win);
    glXMakeCurrent(dpy, None, NULL);
    printf("GLX_DONE rc=%d\n", fails ? 1 : 0);
    return fails ? 1 : 0;
}
