// xkey KEYSYMNAME [WINDOWNAME] | list : focus the X window whose name contains WINDOWNAME (XSetInputFocus) and press a key via XTest
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>
#include <X11/extensions/XTest.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
static Window find(Display *d, Window w, const char *name, int depth) {
  char *n = NULL; if (XFetchName(d, w, &n) && n) { int hit = strstr(n, name) != NULL; if (depth <= 3 && getenv("XKEY_LIST")) printf("win %lx '%s'\n", w, n); XFree(n); if (hit) return w; }
  Window r, p, *kids; unsigned nk;
  if (XQueryTree(d, w, &r, &p, &kids, &nk)) { for (unsigned i = 0; i < nk; i++) { Window f = depth < 4 ? find(d, kids[i], name, depth + 1) : 0; if (f) { XFree(kids); return f; } } if (kids) XFree(kids); }
  return 0;
}
int main(int argc, char **argv) {
  Display *d = XOpenDisplay(NULL); if (!d) { fprintf(stderr, "no display\n"); return 2; }
  const char *wn = argc >= 3 ? argv[2] : "Unigine Heaven";
  Window w = find(d, DefaultRootWindow(d), wn, 0);
  if (!strcmp(argv[1], "list")) return 0;
  if (w) { XSetInputFocus(d, w, RevertToParent, CurrentTime); XFlush(d); usleep(100000); fprintf(stderr, "focused %lx\n", w); } else fprintf(stderr, "window '%s' not found\n", wn);
  KeySym ks = XStringToKeysym(argv[1]); KeyCode kc = XKeysymToKeycode(d, ks);
  XTestFakeKeyEvent(d, kc, True, 0); XFlush(d); usleep(80000); XTestFakeKeyEvent(d, kc, False, 0); XFlush(d);
  XSync(d, False); XCloseDisplay(d); return w ? 0 : 3;
}
