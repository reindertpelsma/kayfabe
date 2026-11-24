/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * kf_gldual.c -- GL dual-context soak of the open Windows rows (STATUS: DESIGN-ONLY, 2026-11-17; written against the WGL / OpenGL 4.x APIs, compiled
 * only by the CPU-only stub test, never by MSVC and never run on a Windows guest).
 *
 * What it reproduces (INFERRED target): DaVinci Resolve died 3 of 3 times in nvoglv64.dll (0x10816ad) 3 to 4 minutes after load on a kf3 guest
 * (RESOLVE_GPU); it is an application with several GL contexts in ONE process. So: one process, two windows, two OpenGL core contexts (optionally
 * sharing objects, --share), two threads, each looping for --seconds over the paths of the GL CTS groups the row also runs:
 *   compute shader writing a 64K-uint SSBO (checksum compared with the closed form in kf_gldual_logic.h),
 *   tessellation (VS, TCS, TES, FS) into a depth-tested RGBA8 FBO, blit into a second FBO, flat-colour centre pixel compared exactly,
 *   a fence wait (thread B) or glFinish (thread A), and a SwapBuffers of the window.
 * Output `KFGL <key> <value>`; the last line is `KFGL RESULT OK|FAIL <why>`, exit 0 iff OK (the protocol of kf_glgears.c). Needs an interactive
 * desktop session (it creates windows). Build: cl /nologo /O2 /W3 kf_gldual.c user32.lib gdi32.lib opengl32.lib   (gl_soak.py does it).
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <GL/gl.h>
#include <stddef.h>
#include "kf_gldual_logic.h"

#define GLC(n, v) enum { n = v }
GLC(KF_COMPUTE_SHADER, 0x91B9); GLC(KF_SSBO, 0x90D2); GLC(KF_TCS, 0x8E88); GLC(KF_TES, 0x8E87); GLC(KF_PATCHES, 0x000E); GLC(KF_PATCH_VERTICES, 0x8E72);
GLC(KF_FRAMEBUFFER, 0x8D40); GLC(KF_READ_FB, 0x8CA8); GLC(KF_DRAW_FB, 0x8CA9); GLC(KF_COLOR0, 0x8CE0); GLC(KF_DEPTH_ATT, 0x8D00);
GLC(KF_FB_COMPLETE, 0x8CD5); GLC(KF_RGBA8, 0x8058); GLC(KF_DEPTH24, 0x81A6); GLC(KF_RENDERBUFFER, 0x8D41); GLC(KF_VERTEX_SHADER, 0x8B31);
GLC(KF_FRAGMENT_SHADER, 0x8B30); GLC(KF_COMPILE_STATUS, 0x8B81); GLC(KF_LINK_STATUS, 0x8B82); GLC(KF_SSBO_BARRIER, 0x2000);
GLC(KF_DYNAMIC_DRAW, 0x88E8); GLC(KF_ARRAY_BUFFER, 0x8892); GLC(KF_SYNC_GPU, 0x9117); GLC(KF_SYNC_FLUSH, 1); GLC(KF_ALREADY, 0x911A);
GLC(KF_CONDITION, 0x911C); GLC(KF_COLOR_ATTACHMENT_TEX, 0x8CE0); GLC(KF_LINEAR, 0x2601);
GLC(WGL_CTX_MAJOR, 0x2091); GLC(WGL_CTX_MINOR, 0x2092); GLC(WGL_CTX_PROFILE, 0x9126); GLC(WGL_CORE_BIT, 1);

typedef char kfchar;
typedef ptrdiff_t kfsizeiptr;
typedef HGLRC(WINAPI *PFN_CTXATTRIBS)(HDC, HGLRC, const int *);
#define FN(ret, name, args) typedef ret(APIENTRY *PFN_##name) args;
FN(GLuint, glCreateShader, (GLenum)) FN(void, glShaderSource, (GLuint, GLsizei, const kfchar *const *, const GLint *)) FN(void, glCompileShader, (GLuint))
FN(void, glGetShaderiv, (GLuint, GLenum, GLint *)) FN(GLuint, glCreateProgram, (void)) FN(void, glAttachShader, (GLuint, GLuint))
FN(void, glLinkProgram, (GLuint)) FN(void, glGetProgramiv, (GLuint, GLenum, GLint *)) FN(void, glUseProgram, (GLuint))
FN(GLint, glGetUniformLocation, (GLuint, const kfchar *)) FN(void, glUniform1ui, (GLint, GLuint)) FN(void, glUniform3f, (GLint, GLfloat, GLfloat, GLfloat))
FN(void, glGenBuffers, (GLsizei, GLuint *)) FN(void, glBindBuffer, (GLenum, GLuint)) FN(void, glBufferData, (GLenum, kfsizeiptr, const void *, GLenum))
FN(void, glBindBufferBase, (GLenum, GLuint, GLuint)) FN(void, glGetBufferSubData, (GLenum, kfsizeiptr, kfsizeiptr, void *))
FN(void, glDispatchCompute, (GLuint, GLuint, GLuint)) FN(void, glMemoryBarrier, (GLbitfield))
FN(void, glGenVertexArrays, (GLsizei, GLuint *)) FN(void, glBindVertexArray, (GLuint)) FN(void, glVertexAttribPointer, (GLuint, GLint, GLenum, GLboolean, GLsizei, const void *))
FN(void, glEnableVertexAttribArray, (GLuint)) FN(void, glPatchParameteri, (GLenum, GLint))
FN(void, glGenFramebuffers, (GLsizei, GLuint *)) FN(void, glBindFramebuffer, (GLenum, GLuint)) FN(void, glFramebufferTexture2D, (GLenum, GLenum, GLenum, GLuint, GLint))
FN(void, glFramebufferRenderbuffer, (GLenum, GLenum, GLenum, GLuint)) FN(GLenum, glCheckFramebufferStatus, (GLenum))
FN(void, glGenRenderbuffers, (GLsizei, GLuint *)) FN(void, glBindRenderbuffer, (GLenum, GLuint)) FN(void, glRenderbufferStorage, (GLenum, GLenum, GLsizei, GLsizei))
FN(void, glBlitFramebuffer, (GLint, GLint, GLint, GLint, GLint, GLint, GLint, GLint, GLbitfield, GLenum))
FN(void *, glFenceSync, (GLenum, GLbitfield)) FN(GLenum, glClientWaitSync, (void *, GLbitfield, unsigned long long)) FN(void, glDeleteSync, (void *))
FN(const GLubyte *, glGetStringi, (GLenum, GLuint))

typedef struct gl_fns {
    PFN_glCreateShader CreateShader; PFN_glShaderSource ShaderSource; PFN_glCompileShader CompileShader; PFN_glGetShaderiv GetShaderiv;
    PFN_glCreateProgram CreateProgram; PFN_glAttachShader AttachShader; PFN_glLinkProgram LinkProgram; PFN_glGetProgramiv GetProgramiv;
    PFN_glUseProgram UseProgram; PFN_glGetUniformLocation GetUniformLocation; PFN_glUniform1ui Uniform1ui; PFN_glUniform3f Uniform3f;
    PFN_glGenBuffers GenBuffers; PFN_glBindBuffer BindBuffer; PFN_glBufferData BufferData; PFN_glBindBufferBase BindBufferBase;
    PFN_glGetBufferSubData GetBufferSubData; PFN_glDispatchCompute DispatchCompute; PFN_glMemoryBarrier MemoryBarrier;
    PFN_glGenVertexArrays GenVertexArrays; PFN_glBindVertexArray BindVertexArray; PFN_glVertexAttribPointer VertexAttribPointer;
    PFN_glEnableVertexAttribArray EnableVertexAttribArray; PFN_glPatchParameteri PatchParameteri;
    PFN_glGenFramebuffers GenFramebuffers; PFN_glBindFramebuffer BindFramebuffer; PFN_glFramebufferTexture2D FramebufferTexture2D;
    PFN_glFramebufferRenderbuffer FramebufferRenderbuffer; PFN_glCheckFramebufferStatus CheckFramebufferStatus;
    PFN_glGenRenderbuffers GenRenderbuffers; PFN_glBindRenderbuffer BindRenderbuffer; PFN_glRenderbufferStorage RenderbufferStorage;
    PFN_glBlitFramebuffer BlitFramebuffer; PFN_glFenceSync FenceSync; PFN_glClientWaitSync ClientWaitSync; PFN_glDeleteSync DeleteSync;
} gl_fns;

static int load_fns(gl_fns *g) {
#define L(field, name) do { g->field = (PFN_##name)wglGetProcAddress(#name); if (!g->field) return 0; } while (0)
    L(CreateShader, glCreateShader); L(ShaderSource, glShaderSource); L(CompileShader, glCompileShader); L(GetShaderiv, glGetShaderiv);
    L(CreateProgram, glCreateProgram); L(AttachShader, glAttachShader); L(LinkProgram, glLinkProgram); L(GetProgramiv, glGetProgramiv);
    L(UseProgram, glUseProgram); L(GetUniformLocation, glGetUniformLocation); L(Uniform1ui, glUniform1ui); L(Uniform3f, glUniform3f);
    L(GenBuffers, glGenBuffers); L(BindBuffer, glBindBuffer); L(BufferData, glBufferData); L(BindBufferBase, glBindBufferBase);
    L(GetBufferSubData, glGetBufferSubData); L(DispatchCompute, glDispatchCompute); L(MemoryBarrier, glMemoryBarrier);
    L(GenVertexArrays, glGenVertexArrays); L(BindVertexArray, glBindVertexArray); L(VertexAttribPointer, glVertexAttribPointer);
    L(EnableVertexAttribArray, glEnableVertexAttribArray); L(PatchParameteri, glPatchParameteri);
    L(GenFramebuffers, glGenFramebuffers); L(BindFramebuffer, glBindFramebuffer); L(FramebufferTexture2D, glFramebufferTexture2D);
    L(FramebufferRenderbuffer, glFramebufferRenderbuffer); L(CheckFramebufferStatus, glCheckFramebufferStatus);
    L(GenRenderbuffers, glGenRenderbuffers); L(BindRenderbuffer, glBindRenderbuffer); L(RenderbufferStorage, glRenderbufferStorage);
    L(BlitFramebuffer, glBlitFramebuffer); L(FenceSync, glFenceSync); L(ClientWaitSync, glClientWaitSync); L(DeleteSync, glDeleteSync);
    return 1;
}

static const char *CS_SRC = "#version 430\nlayout(local_size_x = 64) in;\nlayout(std430, binding = 0) buffer B { uint v[]; };\nuniform uint seed;\n"
    "void main() { uint i = gl_GlobalInvocationID.x; v[i] = i * 2654435761u + seed; }\n";
static const char *VS_SRC = "#version 430\nlayout(location = 0) in vec2 pos;\nvoid main() { gl_Position = vec4(pos, 0.0, 1.0); }\n";
static const char *TCS_SRC = "#version 430\nlayout(vertices = 3) out;\nvoid main() {\n gl_out[gl_InvocationID].gl_Position = gl_in[gl_InvocationID].gl_Position;\n"
    " if (gl_InvocationID == 0) { gl_TessLevelOuter[0] = 4.0; gl_TessLevelOuter[1] = 4.0; gl_TessLevelOuter[2] = 4.0; gl_TessLevelInner[0] = 4.0; }\n}\n";
static const char *TES_SRC = "#version 430\nlayout(triangles, equal_spacing, ccw) in;\nvoid main() {\n"
    " gl_Position = gl_TessCoord.x * gl_in[0].gl_Position + gl_TessCoord.y * gl_in[1].gl_Position + gl_TessCoord.z * gl_in[2].gl_Position;\n}\n";
static const char *FS_SRC = "#version 430\nlayout(location = 0) out vec4 c;\nuniform vec3 col;\nvoid main() { c = vec4(col, 1.0); }\n";

static volatile LONG g_stop = 0, g_setup_failed = 0;
static kf_cfg g_cfg;
static const char *g_fail_text = "";

typedef struct worker {
    int tid;
    HDC dc;
    HGLRC rc;
    kf_stats st;
    int ready;
} worker;

static GLuint compile_stage(gl_fns *g, GLenum type, const char *src) {
    GLuint s = g->CreateShader(type);
    GLint ok = 0;
    g->ShaderSource(s, 1, (const kfchar *const *)&src, NULL);
    g->CompileShader(s);
    g->GetShaderiv(s, KF_COMPILE_STATUS, &ok);
    return ok ? s : 0;
}

static GLuint link_program(gl_fns *g, GLuint a, GLuint b, GLuint c, GLuint d) {
    GLuint p = g->CreateProgram();
    GLint ok = 0;
    if (!a) return 0;
    g->AttachShader(p, a);
    if (b) g->AttachShader(p, b);
    if (c) g->AttachShader(p, c);
    if (d) g->AttachShader(p, d);
    g->LinkProgram(p);
    g->GetProgramiv(p, KF_LINK_STATUS, &ok);
    return ok ? p : 0;
}

static DWORD WINAPI worker_main(LPVOID arg) {
    worker *w = (worker *)arg;
    gl_fns g;
    GLuint cs, prog_c, prog_t, buf, vbo, vao, fbo[2], tex[2], rbo, seedloc, colloc;
    static const float quad[12] = { -1.f, -1.f, 1.f, -1.f, 1.f, 1.f, -1.f, -1.f, 1.f, 1.f, -1.f, 1.f };
    uint32_t *host = (uint32_t *)malloc(KF_COMPUTE_N * sizeof(uint32_t));
    unsigned iter = 0;
    if (!wglMakeCurrent(w->dc, w->rc) || !load_fns(&g) || !host) { g_fail_text = "context-or-entry-points"; InterlockedExchange(&g_setup_failed, 1); return 1; }
    if (w->tid == 0) {
        const char *r = (const char *)glGetString(GL_RENDERER), *v = (const char *)glGetString(GL_VERSION);
        printf("KFGL renderer %s\nKFGL version %s\n", r ? r : "?", v ? v : "?"); fflush(stdout);
        if (!g_cfg.any_renderer && (!r || !strstr(r, "NVIDIA"))) { g_fail_text = "renderer-not-nvidia"; InterlockedExchange(&g_setup_failed, 1); return 1; }
    }
    cs = compile_stage(&g, KF_COMPUTE_SHADER, CS_SRC);
    prog_c = link_program(&g, cs, 0, 0, 0);
    prog_t = link_program(&g, compile_stage(&g, KF_VERTEX_SHADER, VS_SRC), compile_stage(&g, KF_TCS, TCS_SRC), compile_stage(&g, KF_TES, TES_SRC),
                          compile_stage(&g, KF_FRAGMENT_SHADER, FS_SRC));
    if (!prog_c || !prog_t) { g_fail_text = "shader-build"; InterlockedExchange(&g_setup_failed, 1); return 1; }
    seedloc = (GLuint)g.GetUniformLocation(prog_c, "seed");
    colloc = (GLuint)g.GetUniformLocation(prog_t, "col");
    g.GenBuffers(1, &buf); g.BindBuffer(KF_SSBO, buf); g.BufferData(KF_SSBO, KF_COMPUTE_N * 4, NULL, KF_DYNAMIC_DRAW);
    g.GenVertexArrays(1, &vao); g.BindVertexArray(vao);
    g.GenBuffers(1, &vbo); g.BindBuffer(KF_ARRAY_BUFFER, vbo); g.BufferData(KF_ARRAY_BUFFER, sizeof quad, quad, KF_DYNAMIC_DRAW);
    g.VertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 8, (const void *)0); g.EnableVertexAttribArray(0);
    glGenTextures(2, tex);
    g.GenFramebuffers(2, fbo); g.GenRenderbuffers(1, &rbo);
    g.BindRenderbuffer(KF_RENDERBUFFER, rbo); g.RenderbufferStorage(KF_RENDERBUFFER, KF_DEPTH24, KF_FBO_SIZE, KF_FBO_SIZE);
    for (iter = 0; iter < 2; iter++) {
        glBindTexture(GL_TEXTURE_2D, tex[iter]);
        glTexImage2D(GL_TEXTURE_2D, 0, KF_RGBA8, KF_FBO_SIZE, KF_FBO_SIZE, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, KF_LINEAR);
        g.BindFramebuffer(KF_FRAMEBUFFER, fbo[iter]);
        g.FramebufferTexture2D(KF_FRAMEBUFFER, KF_COLOR0, GL_TEXTURE_2D, tex[iter], 0);
        if (iter == 0) g.FramebufferRenderbuffer(KF_FRAMEBUFFER, KF_DEPTH_ATT, KF_RENDERBUFFER, rbo);
        if (g.CheckFramebufferStatus(KF_FRAMEBUFFER) != KF_FB_COMPLETE) { g_fail_text = "fbo-incomplete"; InterlockedExchange(&g_setup_failed, 1); return 1; }
    }
    w->ready = 1;
    for (iter = 0; !g_stop && !g_setup_failed; iter++) {
        uint32_t seed = (uint32_t)(iter * 40503u + 977u * (unsigned)w->tid), sum = 0;
        unsigned ch[3], px[4] = { 0, 0, 0, 0 };
        unsigned k;
        /* compute: SSBO checksum */
        g.UseProgram(prog_c); g.Uniform1ui((GLint)seedloc, seed);
        g.BindBufferBase(KF_SSBO, 0, buf);
        g.DispatchCompute(KF_COMPUTE_N / 64u, 1, 1);
        g.MemoryBarrier(KF_SSBO_BARRIER);
        g.BindBuffer(KF_SSBO, buf);
        g.GetBufferSubData(KF_SSBO, 0, KF_COMPUTE_N * 4, host);
        for (k = 0; k < KF_COMPUTE_N; k++) sum += host[k];
        if (sum != kf_expected_sum(KF_COMPUTE_N, seed)) kf_note_bad(&w->st, "compute checksum", iter);
        /* tessellation into FBO 0 with depth test, blit to FBO 1, read the centre */
        for (k = 0; k < 3; k++) ch[k] = kf_channel(iter, (unsigned)w->tid, k);
        g.BindFramebuffer(KF_FRAMEBUFFER, fbo[0]);
        glViewport(0, 0, KF_FBO_SIZE, KF_FBO_SIZE);
        glEnable(GL_DEPTH_TEST); glClearColor(0.f, 0.f, 0.f, 1.f); glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        g.UseProgram(prog_t); g.Uniform3f((GLint)colloc, ch[0] / 255.f, ch[1] / 255.f, ch[2] / 255.f);
        g.BindVertexArray(vao); g.PatchParameteri(KF_PATCH_VERTICES, 3);
        glDrawArrays(KF_PATCHES, 0, 6);
        g.BindFramebuffer(KF_READ_FB, fbo[0]); g.BindFramebuffer(KF_DRAW_FB, fbo[1]);
        g.BlitFramebuffer(0, 0, KF_FBO_SIZE, KF_FBO_SIZE, 0, 0, KF_FBO_SIZE, KF_FBO_SIZE, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        g.BindFramebuffer(KF_READ_FB, fbo[1]);
        glReadPixels(KF_FBO_SIZE / 2, KF_FBO_SIZE / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
        if ((unsigned char)(px[0] & 255) != (unsigned char)ch[0] || (unsigned char)((px[0] >> 8) & 255) != (unsigned char)ch[1] ||
            (unsigned char)((px[0] >> 16) & 255) != (unsigned char)ch[2]) kf_note_bad(&w->st, "tessellated FBO pixel", iter);
        /* synchronisation: fence on one thread, glFinish on the other; a present of the window every 4th iteration */
        if (w->tid == 1) {
            void *f = g.FenceSync(KF_SYNC_GPU, 0);
            GLenum r = g.ClientWaitSync(f, KF_SYNC_FLUSH, 5000000000ull);
            g.DeleteSync(f);
            if (r != KF_ALREADY && r != KF_CONDITION) kf_note_bad(&w->st, "fence wait timed out or failed", iter);
        } else glFinish();
        if ((iter & 3u) == 0) { g.BindFramebuffer(KF_FRAMEBUFFER, 0); glClearColor(0.f, 0.f, (float)(iter & 255u) / 255.f, 1.f); glClear(GL_COLOR_BUFFER_BIT); SwapBuffers(w->dc); }
        if (glGetError() != 0) kf_note_bad(&w->st, "unexpected glGetError", iter);
        w->st.iters++;
    }
    wglMakeCurrent(NULL, NULL);
    free(host);
    return 0;
}

static LRESULT CALLBACK wndproc(HWND h, UINT m, WPARAM wp, LPARAM lp) { return DefWindowProcA(h, m, wp, lp); }

static HWND make_window(const char *name, int x) {
    return CreateWindowExA(0, "kf_gldual", name, WS_OVERLAPPEDWINDOW | WS_VISIBLE, x, 40, 320, 240, NULL, NULL, GetModuleHandleA(NULL), NULL);
}

static int set_format(HDC dc) {
    PIXELFORMATDESCRIPTOR pfd;
    int fmt;
    memset(&pfd, 0, sizeof pfd);
    pfd.nSize = sizeof pfd; pfd.nVersion = 1; pfd.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
    pfd.iPixelType = PFD_TYPE_RGBA; pfd.cColorBits = 32; pfd.cDepthBits = 24; pfd.cStencilBits = 8;
    fmt = ChoosePixelFormat(dc, &pfd);
    return fmt && SetPixelFormat(dc, fmt, &pfd);
}

int main(int argc, char **argv) {
    WNDCLASSA wc;
    HWND win[3];
    worker w[2];
    HANDLE th[2];
    HGLRC boot;
    PFN_CTXATTRIBS create_attribs;
    int attrs[] = { WGL_CTX_MAJOR, 4, WGL_CTX_MINOR, 6, WGL_CTX_PROFILE, WGL_CORE_BIT, 0 };
    char line[256];
    DWORD t0, now, last = 0;
    int i, rc;
    kf_parse_args(argc, argv, &g_cfg);
    if (g_cfg.bad_args) { printf("usage: kf_gldual [--seconds N] [--share] [--any-renderer]\nKFGL RESULT FAIL bad-arguments\n"); return 2; }
    memset(&wc, 0, sizeof wc);
    wc.lpfnWndProc = wndproc; wc.hInstance = GetModuleHandleA(NULL); wc.lpszClassName = "kf_gldual"; wc.hCursor = LoadCursorA(NULL, IDC_ARROW);
    RegisterClassA(&wc);
    memset(w, 0, sizeof w);
    win[2] = make_window("kf_gldual boot", 0);
    {   /* a legacy context only to reach wglCreateContextAttribsARB */
        HDC bdc = GetDC(win[2]);
        if (!set_format(bdc) || !(boot = wglCreateContext(bdc)) || !wglMakeCurrent(bdc, boot)) { printf("KFGL RESULT FAIL no-legacy-context\n"); return 1; }
        create_attribs = (PFN_CTXATTRIBS)wglGetProcAddress("wglCreateContextAttribsARB");
        wglMakeCurrent(NULL, NULL); wglDeleteContext(boot);
        if (!create_attribs) { printf("KFGL RESULT FAIL no-wglCreateContextAttribsARB\n"); return 1; }
    }
    for (i = 0; i < 2; i++) {
        win[i] = make_window(i ? "kf_gldual B" : "kf_gldual A", 340 * i);
        w[i].tid = i; w[i].dc = GetDC(win[i]);
        if (!set_format(w[i].dc)) { printf("KFGL RESULT FAIL pixel-format\n"); return 1; }
        w[i].rc = create_attribs(w[i].dc, (i == 1 && g_cfg.share) ? w[0].rc : NULL, attrs);
        if (!w[i].rc) { printf("KFGL RESULT FAIL core-4.6-context-%d\n", i); return 1; }
    }
    printf("KFGL start seconds=%d share=%d\n", g_cfg.seconds, g_cfg.share); fflush(stdout);
    for (i = 0; i < 2; i++) th[i] = CreateThread(NULL, 0, worker_main, &w[i], 0, NULL);
    t0 = GetTickCount();
    for (;;) {
        MSG m;
        while (PeekMessageA(&m, NULL, 0, 0, PM_REMOVE)) { TranslateMessage(&m); DispatchMessageA(&m); }
        now = GetTickCount() - t0;
        if (now / 1000u >= (DWORD)g_cfg.seconds || g_setup_failed) break;
        if (now - last >= 5000u) {
            last = now;
            printf("KFGL tick t=%lu a=%lu b=%lu bad=%lu/%lu\n", (unsigned long)(now / 1000u), w[0].st.iters, w[1].st.iters, w[0].st.bad, w[1].st.bad); fflush(stdout);
        }
        Sleep(20);
    }
    InterlockedExchange(&g_stop, 1);
    WaitForMultipleObjects(2, th, TRUE, 60000);
    rc = kf_result(&w[0].st, &w[1].st, g_setup_failed, line, sizeof line);
    if (g_setup_failed) printf("KFGL FAILED %s\n", g_fail_text);
    printf("%s\n", line); fflush(stdout);
    return rc;
}
