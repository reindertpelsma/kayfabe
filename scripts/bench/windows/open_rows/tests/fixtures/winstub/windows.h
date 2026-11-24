/* Declarations-only stand-in for <windows.h>: lets the CPU-only test type-check kf_gldual.c on Linux. It proves the C is well-formed against
 * THESE declarations, not that it builds with the real Windows SDK (INFERRED until the first MSVC build). */
#ifndef KF_WINSTUB_H
#define KF_WINSTUB_H
#include <stddef.h>
#include <stdint.h>
#define WINAPI
#define CALLBACK
#define APIENTRY
typedef void *HANDLE, *HWND, *HDC, *HGLRC, *HINSTANCE, *LPVOID, *HCURSOR;
typedef unsigned long DWORD;
typedef long LONG, LRESULT;
typedef unsigned int UINT;
typedef int BOOL;
typedef uintptr_t WPARAM;
typedef intptr_t LPARAM;
typedef const char *LPCSTR;
typedef DWORD (WINAPI *LPTHREAD_START_ROUTINE)(LPVOID);
typedef LRESULT (CALLBACK *WNDPROC)(HWND, UINT, WPARAM, LPARAM);
typedef struct { UINT style; WNDPROC lpfnWndProc; int cbClsExtra, cbWndExtra; HINSTANCE hInstance; void *hIcon; HCURSOR hCursor; void *hbrBackground; LPCSTR lpszMenuName, lpszClassName; } WNDCLASSA;
typedef struct { HWND hwnd; UINT message; WPARAM wParam; LPARAM lParam; DWORD time; long pt[2]; } MSG;
typedef struct { unsigned short nSize, nVersion; DWORD dwFlags; unsigned char iPixelType, cColorBits, cRedBits, cRedShift, cGreenBits, cGreenShift, cBlueBits, cBlueShift,
    cAlphaBits, cAlphaShift, cAccumBits, cAccumRedBits, cAccumGreenBits, cAccumBlueBits, cAccumAlphaBits, cDepthBits, cStencilBits, cAuxBuffers, iLayerType, bReserved;
    DWORD dwLayerMask, dwVisibleMask, dwDamageMask; } PIXELFORMATDESCRIPTOR;
#define PFD_DRAW_TO_WINDOW 4
#define PFD_SUPPORT_OPENGL 32
#define PFD_DOUBLEBUFFER 1
#define PFD_TYPE_RGBA 0
#define WS_OVERLAPPEDWINDOW 0xCF0000L
#define WS_VISIBLE 0x10000000L
#define PM_REMOVE 1
#define TRUE 1
#define FALSE 0
#define IDC_ARROW ((LPCSTR)32512)
#define INFINITE 0xFFFFFFFFul
HINSTANCE GetModuleHandleA(LPCSTR);
HCURSOR LoadCursorA(HINSTANCE, LPCSTR);
unsigned short RegisterClassA(const WNDCLASSA *);
HWND CreateWindowExA(DWORD, LPCSTR, LPCSTR, DWORD, int, int, int, int, HWND, void *, HINSTANCE, LPVOID);
HDC GetDC(HWND);
int ChoosePixelFormat(HDC, const PIXELFORMATDESCRIPTOR *);
BOOL SetPixelFormat(HDC, int, const PIXELFORMATDESCRIPTOR *);
BOOL SwapBuffers(HDC);
HGLRC wglCreateContext(HDC);
BOOL wglDeleteContext(HGLRC);
BOOL wglMakeCurrent(HDC, HGLRC);
void *wglGetProcAddress(LPCSTR);
HANDLE CreateThread(void *, size_t, LPTHREAD_START_ROUTINE, LPVOID, DWORD, DWORD *);
DWORD WaitForMultipleObjects(DWORD, const HANDLE *, BOOL, DWORD);
LONG InterlockedExchange(volatile LONG *, LONG);
BOOL PeekMessageA(MSG *, HWND, UINT, UINT, UINT);
BOOL TranslateMessage(const MSG *);
LRESULT DispatchMessageA(const MSG *);
LRESULT DefWindowProcA(HWND, UINT, WPARAM, LPARAM);
DWORD GetTickCount(void);
void Sleep(DWORD);
#endif
