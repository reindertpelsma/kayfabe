#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <stdio.h>
#include <string.h>
static Window find(Display *d, Window w, const char *name, int depth) {
  char *n = NULL; if (XFetchName(d, w, &n) && n) { int hit = strstr(n, name) != NULL; XFree(n); if (hit) return w; }
  Window r, p, *kids; unsigned nk;
  if (XQueryTree(d, w, &r, &p, &kids, &nk)) { for (unsigned i = 0; i < nk; i++) { Window f = depth < 4 ? find(d, kids[i], name, depth + 1) : 0; if (f) { XFree(kids); return f; } } if (kids) XFree(kids); }
  return 0;
}
int main(int argc, char **argv) {
  Display *d = XOpenDisplay(NULL); if (!d) return 2;
  Window w = find(d, DefaultRootWindow(d), argc > 1 ? argv[1] : "Unigine Heaven", 0);
  int rw = DisplayWidth(d, 0), rh = DisplayHeight(d, 0);
  if (!w) { printf("none %d %d\n", rw, rh); return 3; }
  XWindowAttributes a; XGetWindowAttributes(d, w, &a); Window c; int x, y; XTranslateCoordinates(d, w, DefaultRootWindow(d), 0, 0, &x, &y, &c);
  printf("%d %d %d %d %d %d\n", x, y, a.width, a.height, rw, rh); return 0;
}
