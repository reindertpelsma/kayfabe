/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * kf_glvsync.c -- the vsync reproducer for the Win11 OpenGL hang (traces/win11_opengl_20261014): a
 * WGL context on the NVIDIA ICD, wglSwapIntervalEXT(--swap N), a clear+triangle+SwapBuffers loop for
 * --seconds, windowed or --fullscreen (a borderless popup covering the primary monitor, the case the
 * driver may flip). A watchdog thread reports a stall (no frame for 2 s) and, if no frame arrives for
 * --hang-seconds, prints HANG and exits non-zero, so a hang is a result and not a stuck runner.
 *   kf_glvsync.exe [--swap N] [--seconds N] [--fullscreen] [--hang-seconds N]
 * Output: `KFVS <key> <value>` lines, a per-second `KFVS sec` line, last line `KFVS RESULT OK|FAIL <why>`.
 * Build: x86_64-w64-mingw32-gcc -O2 -o kf_glvsync.exe kf_glvsync.c -lopengl32 -lgdi32 -luser32 -lwinmm
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <GL/gl.h>
#include <mmsystem.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef BOOL(WINAPI *PFNWGLSWAPINTERVALEXT)(int);
typedef int(WINAPI *PFNWGLGETSWAPINTERVALEXT)(void);
static volatile LONG frames = 0;
static volatile LONGLONG last_frame_ms = 0;
static int hang_s = 30;

static LONGLONG now_ms(void) { LARGE_INTEGER f, c; QueryPerformanceFrequency(&f); QueryPerformanceCounter(&c); return c.QuadPart * 1000 / f.QuadPart; }
static LRESULT CALLBACK wp(HWND h, UINT m, WPARAM w, LPARAM l) { return DefWindowProcA(h, m, w, l); }

static DWORD WINAPI watchdog(LPVOID p) {
  (void)p; int stalled = 0;
  for (;;) {
    Sleep(500);
    LONGLONG since = now_ms() - last_frame_ms;
    if (last_frame_ms && since > 2000 && !stalled) { printf("KFVS STALL frame=%ld no_frame_for_ms=%lld\n", frames, since); fflush(stdout); stalled = 1; }
    if (since < 2000) stalled = 0;
    if (last_frame_ms && since > (LONGLONG)hang_s * 1000) { printf("KFVS HANG frame=%ld no_frame_for_ms=%lld\nKFVS RESULT FAIL hang\n", frames, since); fflush(stdout); ExitProcess(3); }
  }
  return 0;
}

int main(int argc, char **argv) {
  int seconds = 90, swap = 1, full = 0;
  for (int i = 1; i < argc; i++) {
    if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atoi(argv[++i]);
    else if (!strcmp(argv[i], "--swap") && i + 1 < argc) swap = atoi(argv[++i]);
    else if (!strcmp(argv[i], "--hang-seconds") && i + 1 < argc) hang_s = atoi(argv[++i]);
    else if (!strcmp(argv[i], "--fullscreen")) full = 1;
  }
  timeBeginPeriod(1);
  WNDCLASSA c = {0}; c.lpfnWndProc = wp; c.hInstance = GetModuleHandleA(0); c.lpszClassName = "kfvs";
  RegisterClassA(&c);
  int sw_ = GetSystemMetrics(SM_CXSCREEN), sh_ = GetSystemMetrics(SM_CYSCREEN);
  HWND w = full ? CreateWindowA("kfvs", "kf_glvsync", WS_POPUP | WS_VISIBLE, 0, 0, sw_, sh_, 0, 0, c.hInstance, 0)
                : CreateWindowA("kfvs", "kf_glvsync", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 40, 800, 600, 0, 0, c.hInstance, 0);
  if (!w) { printf("KFVS RESULT FAIL CreateWindow\n"); return 1; }
  HDC dc = GetDC(w);
  PIXELFORMATDESCRIPTOR pfd = {0}; pfd.nSize = sizeof pfd; pfd.nVersion = 1;
  pfd.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER; pfd.iPixelType = PFD_TYPE_RGBA; pfd.cColorBits = 32; pfd.cDepthBits = 24;
  int pf = ChoosePixelFormat(dc, &pfd);
  if (!pf || !SetPixelFormat(dc, pf, &pfd)) { printf("KFVS RESULT FAIL SetPixelFormat\n"); return 1; }
  HGLRC rc = wglCreateContext(dc);
  if (!rc || !wglMakeCurrent(dc, rc)) { printf("KFVS RESULT FAIL context\n"); return 1; }
  printf("KFVS renderer %s\nKFVS version %s\nKFVS mode %s %dx%d\n", (const char *)glGetString(GL_RENDERER), (const char *)glGetString(GL_VERSION), full ? "fullscreen" : "windowed", full ? sw_ : 800, full ? sh_ : 600);
  PFNWGLSWAPINTERVALEXT si = (PFNWGLSWAPINTERVALEXT)wglGetProcAddress("wglSwapIntervalEXT");
  PFNWGLGETSWAPINTERVALEXT gi = (PFNWGLGETSWAPINTERVALEXT)wglGetProcAddress("wglGetSwapIntervalEXT");
  BOOL ok = si ? si(swap) : FALSE;
  printf("KFVS swap_interval set=%d ok=%d get=%d\n", swap, ok, gi ? gi() : -1); fflush(stdout);
  last_frame_ms = now_ms();
  CreateThread(0, 0, watchdog, 0, 0, 0);
  LONGLONG t0 = now_ms(), tsec = t0, max_gap = 0, prev = t0; long fsec = 0;
  while (now_ms() - t0 < (LONGLONG)seconds * 1000) {
    MSG m; while (PeekMessageA(&m, 0, 0, 0, PM_REMOVE)) DispatchMessageA(&m);
    float k = (float)(frames % 120) / 120.0f;
    glClearColor(k, 0.2f, 1.0f - k, 1); glClear(GL_COLOR_BUFFER_BIT);
    glBegin(GL_TRIANGLES); glColor3f(1, 1, 1); glVertex2f(-0.5f + k, -0.5f); glVertex2f(0.5f, -0.5f); glVertex2f(0, 0.5f); glEnd();
    SwapBuffers(dc);
    LONGLONG t = now_ms(); if (t - prev > max_gap) max_gap = t - prev; prev = t;
    InterlockedIncrement(&frames); last_frame_ms = t; fsec++;
    if (t - tsec >= 1000) { printf("KFVS sec t=%lld frames=%ld fps=%ld max_gap_ms=%lld\n", (t - t0) / 1000, frames, fsec, max_gap); fflush(stdout); tsec = t; fsec = 0; max_gap = 0; }
  }
  GLenum e = glGetError();
  double s = (now_ms() - t0) / 1000.0;
  printf("KFVS done frames=%ld secs=%.1f fps=%.1f glerr=0x%x\n", frames, s, frames / s, e);
  printf("KFVS RESULT %s\n", e ? "FAIL glerror" : "OK"); fflush(stdout);
  wglMakeCurrent(0, 0); wglDeleteContext(rc);
  return e ? 2 : 0;
}
