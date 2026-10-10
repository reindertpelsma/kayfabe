/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * kf_glgears.c -- the Windows counterpart of glxgears for the app matrix: an OpenGL (WGL) context on
 * the real display driver, GL_RENDERER must be NVIDIA (not "GDI Generic" / Microsoft / llvmpipe), a
 * known quad is drawn and read back (colour check), then spinning gears-like geometry is rendered
 * for --seconds with vsync off and the frame rate is printed.
 *   kf_glgears.exe [--seconds N] [--any-renderer]
 * Output: `KFGL <key> <value>`, last line `KFGL RESULT OK|FAIL <why>`, exit 0 iff OK.
 * Needs an interactive desktop session (it creates a window). Build: build_tools.sh (mingw-w64).
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <GL/gl.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef BOOL(WINAPI *PFNWGLSWAPINTERVALEXT)(int);
static int fail = 0;
static char why[200] = "";
static void say(const char *k, const char *v) { printf("KFGL %s %s\n", k, v); fflush(stdout); }
static void bad(const char *w) { if (!fail) { fail = 1; snprintf(why, sizeof why, "%s", w); } printf("KFGL FAILED %s\n", w); fflush(stdout); }

static LRESULT CALLBACK wp(HWND h, UINT m, WPARAM w, LPARAM l) { return DefWindowProcA(h, m, w, l); }

static void gear(float inner, float outer, int teeth, float width) {
  float da = 2.0f * 3.14159265f / teeth / 4.0f;
  glBegin(GL_QUAD_STRIP);
  for (int i = 0; i <= teeth; i++) {
    float a = i * 2.0f * 3.14159265f / teeth;
    glVertex3f(inner * cosf(a), inner * sinf(a), width * 0.5f);
    glVertex3f(inner * cosf(a), inner * sinf(a), -width * 0.5f);
    glVertex3f(outer * cosf(a + da), outer * sinf(a + da), width * 0.5f);
    glVertex3f(outer * cosf(a + da), outer * sinf(a + da), -width * 0.5f);
  }
  glEnd();
}

int main(int argc, char **argv) {
  int seconds = 8, any = 0;
  for (int i = 1; i < argc; i++) {
    if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atoi(argv[++i]);
    else if (!strcmp(argv[i], "--any-renderer")) any = 1;
  }
  WNDCLASSA c = {0}; c.lpfnWndProc = wp; c.hInstance = GetModuleHandleA(0); c.lpszClassName = "kfgl";
  RegisterClassA(&c);
  HWND w = CreateWindowA("kfgl", "kf_glgears", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 40, 640, 480, 0, 0, c.hInstance, 0);
  if (!w) { bad("CreateWindow"); goto done; }
  HDC dc = GetDC(w);
  PIXELFORMATDESCRIPTOR pfd = {0}; pfd.nSize = sizeof pfd; pfd.nVersion = 1;
  pfd.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER; pfd.iPixelType = PFD_TYPE_RGBA; pfd.cColorBits = 32; pfd.cDepthBits = 24;
  int pf = ChoosePixelFormat(dc, &pfd);
  if (!pf || !SetPixelFormat(dc, pf, &pfd)) { bad("SetPixelFormat"); goto done; }
  HGLRC rc = wglCreateContext(dc);
  if (!rc || !wglMakeCurrent(dc, rc)) { bad("wglCreateContext/MakeCurrent"); goto done; }
  const char *ven = (const char *)glGetString(GL_VENDOR), *ren = (const char *)glGetString(GL_RENDERER), *ver = (const char *)glGetString(GL_VERSION);
  printf("KFGL vendor %s\nKFGL renderer %s\nKFGL version %s\n", ven ? ven : "?", ren ? ren : "?", ver ? ver : "?"); fflush(stdout);
  if (!any && !(ven && strstr(ven, "NVIDIA") && ren && strstr(ren, "NVIDIA"))) { bad("GL renderer is not NVIDIA (software or wrong adapter)"); goto done; }
  PFNWGLSWAPINTERVALEXT sw = (PFNWGLSWAPINTERVALEXT)wglGetProcAddress("wglSwapIntervalEXT");
  if (sw) sw(0);
  /* colour check: a known quad, read back from the back buffer */
  glViewport(0, 0, 640, 480);
  glClearColor(0, 0, 0, 1); glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
  glMatrixMode(GL_PROJECTION); glLoadIdentity(); glOrtho(-1, 1, -1, 1, -1, 1);
  glMatrixMode(GL_MODELVIEW); glLoadIdentity();
  glColor3ub(64, 128, 191);
  glBegin(GL_QUADS); glVertex2f(-0.5f, -0.5f); glVertex2f(0.5f, -0.5f); glVertex2f(0.5f, 0.5f); glVertex2f(-0.5f, 0.5f); glEnd();
  unsigned char px[4] = {0};
  glReadBuffer(GL_BACK); glReadPixels(320, 240, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
  { char b[80]; snprintf(b, sizeof b, "%u,%u,%u,%u", px[0], px[1], px[2], px[3]); say("pixel", b); }
  if (abs(px[0] - 64) > 2 || abs(px[1] - 128) > 2 || abs(px[2] - 191) > 2) bad("quad readback colour mismatch");
  GLenum e = glGetError();
  if (e) { char b[40]; snprintf(b, sizeof b, "glGetError 0x%x after quad", e); bad(b); }
  /* gears-like loop */
  DWORD t0 = GetTickCount(); long frames = 0; float ang = 0;
  glEnable(GL_DEPTH_TEST);
  while (GetTickCount() - t0 < (DWORD)seconds * 1000) {
    MSG m; while (PeekMessageA(&m, 0, 0, 0, PM_REMOVE)) DispatchMessageA(&m);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    glMatrixMode(GL_PROJECTION); glLoadIdentity(); glFrustum(-1.3, 1.3, -1.0, 1.0, 5, 60);
    glMatrixMode(GL_MODELVIEW); glLoadIdentity(); glTranslatef(0, 0, -20); glRotatef(40, 1, 0, 0); glRotatef(ang, 0, 1, 0);
    glColor3f(0.8f, 0.1f, 0.0f); gear(1.0f, 4.0f, 20, 1.0f);
    glPushMatrix(); glTranslatef(5.1f, 0, 0); glRotatef(-2 * ang - 9, 0, 0, 1); glColor3f(0.0f, 0.8f, 0.2f); gear(0.5f, 2.0f, 10, 2.0f); glPopMatrix();
    glPushMatrix(); glTranslatef(-3.1f, 4.2f, 0); glRotatef(-2 * ang - 25, 0, 0, 1); glColor3f(0.2f, 0.2f, 1.0f); gear(1.3f, 2.0f, 10, 0.5f); glPopMatrix();
    SwapBuffers(dc);
    ang += 1.0f; frames++;
  }
  e = glGetError();
  { char b[96]; double s = (GetTickCount() - t0) / 1000.0; snprintf(b, sizeof b, "frames=%ld secs=%.1f fps=%.1f glerr=0x%x", frames, s, frames / (s > 0 ? s : 1), e); say("gears", b); }
  if (frames < 10) bad("fewer than 10 frames rendered");
  if (e) bad("GL error during gears");
  wglMakeCurrent(0, 0); wglDeleteContext(rc);
done:
  if (!fail) say("RESULT", "OK"); else { char b[240]; snprintf(b, sizeof b, "FAIL %s", why); say("RESULT", b); }
  return fail;
}
