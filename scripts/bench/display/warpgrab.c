// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// warpgrab.c — what GLFW's disabled-cursor mode does on X11 (docs/design/V3_DISPLAY.md §8.19), so a
// game's mouse-look can be graded without the game: XGrabPointer confined to a window with a blank
// cursor, XWarpPointer to the window centre after every MotionNotify that left it, the camera delta
// taken from the core position minus the last position (GLFW's raw-motion-off path) and, separately,
// summed from XI2 RawMotion (its raw-motion-on path), with the device id of each raw event.
//
//   warpgrab <seconds> [log]   prints one RESULT line; every event goes to <log> when given
//
// Build in the guest: cc -O2 -o warpgrab warpgrab.c -lX11 -lXi
#include <X11/Xlib.h>
#include <X11/extensions/XInput2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/select.h>
#include <time.h>

static double now_s(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (double)t.tv_sec + (double)t.tv_nsec / 1e9;
}

int main(int argc, char **argv)
{
    double secs = argc > 1 ? atof(argv[1]) : 10.0;
    FILE *log = argc > 2 ? fopen(argv[2], "w") : NULL;
    Display *d = XOpenDisplay(NULL);
    if (!d) {
        fprintf(stderr, "no display\n");
        return 2;
    }
    int xi_op, ev, er, maj = 2, min = 0;
    if (!XQueryExtension(d, "XInputExtension", &xi_op, &ev, &er) ||
        XIQueryVersion(d, &maj, &min) != Success) {
        fprintf(stderr, "no XI2\n");
        return 2;
    }
    Window root = DefaultRootWindow(d);
    int W = 800, H = 600, cx = W / 2, cy = H / 2;
    Window w = XCreateSimpleWindow(d, root, 100, 100, W, H, 0, 0, 0);
    XSelectInput(d, w, ExposureMask | PointerMotionMask | ButtonPressMask | ButtonReleaseMask |
                           StructureNotifyMask);
    XMapRaised(d, w);
    XEvent e;
    for (;;) {
        XNextEvent(d, &e);
        if (e.type == MapNotify) {
            break;
        }
    }
    unsigned char m[XIMaskLen(XI_LASTEVENT)] = {0};
    XISetMask(m, XI_RawMotion);
    XIEventMask em = {XIAllMasterDevices, sizeof(m), m};
    XISelectEvents(d, root, &em, 1);
    // a blank cursor, as GLFW's hidden cursor
    char zero = 0;
    Pixmap pm = XCreateBitmapFromData(d, w, &zero, 1, 1);
    XColor black = {0};
    Cursor blank = XCreatePixmapCursor(d, pm, pm, &black, &black, 0, 0);
    int g = XGrabPointer(d, w, True, PointerMotionMask | ButtonPressMask | ButtonReleaseMask,
                         GrabModeAsync, GrabModeAsync, w, blank, CurrentTime);
    XWarpPointer(d, None, w, 0, 0, 0, 0, cx, cy);
    XSync(d, False);
    int lastx = cx, lasty = cy;
    long core_x = 0, core_y = 0, warps = 0, motions = 0, buttons = 0, raws = 0, jumps = 0;
    double raw_x = 0, raw_y = 0;
    int raw_dev[8] = {0}, raw_devs = 0;
    double t0 = now_s();
    int fd = ConnectionNumber(d);
    while (now_s() - t0 < secs) {
        if (!XPending(d)) {
            fd_set s;
            FD_ZERO(&s);
            FD_SET(fd, &s);
            struct timeval tv = {0, 20000};
            select(fd + 1, &s, NULL, NULL, &tv);
            continue;
        }
        XNextEvent(d, &e);
        double t = now_s() - t0;
        if (e.type == MotionNotify) {
            int dx = e.xmotion.x - lastx, dy = e.xmotion.y - lasty;
            core_x += dx;
            core_y += dy;
            motions++;
            if (dx * dx + dy * dy > 200 * 200) {
                jumps++;
            }
            if (log) {
                fprintf(log, "%.3f MOTION at %d,%d delta %d,%d core %ld,%ld\n", t, e.xmotion.x,
                        e.xmotion.y, dx, dy, core_x, core_y);
            }
            lastx = e.xmotion.x;
            lasty = e.xmotion.y;
            if (lastx != cx || lasty != cy) {
                XWarpPointer(d, None, w, 0, 0, 0, 0, cx, cy);
                lastx = cx;
                lasty = cy;
                warps++;
            }
        } else if (e.type == ButtonPress || e.type == ButtonRelease) {
            buttons++;
            if (log) {
                fprintf(log, "%.3f BUTTON %s %u at %d,%d\n", t,
                        e.type == ButtonPress ? "press" : "release", e.xbutton.button,
                        e.xbutton.x, e.xbutton.y);
            }
        } else if (e.type == GenericEvent && e.xcookie.extension == xi_op &&
                   XGetEventData(d, &e.xcookie)) {
            if (e.xcookie.evtype == XI_RawMotion) {
                XIRawEvent *r = e.xcookie.data;
                double v[2] = {0, 0};
                double *val = r->raw_values;
                for (int i = 0; i < r->valuators.mask_len * 8 && i < 2; i++) {
                    if (XIMaskIsSet(r->valuators.mask, i)) {
                        v[i] = *val++;
                    }
                }
                raw_x += v[0];
                raw_y += v[1];
                raws++;
                int k;
                for (k = 0; k < raw_devs && raw_dev[k] != r->sourceid; k++) {
                }
                if (k == raw_devs && raw_devs < 8) {
                    raw_dev[raw_devs++] = r->sourceid;
                }
                if (log) {
                    fprintf(log, "%.3f RAW source %d delta %.0f,%.0f raw %.0f,%.0f\n", t,
                            r->sourceid, v[0], v[1], raw_x, raw_y);
                }
            }
            XFreeEventData(d, &e.xcookie);
        }
    }
    XUngrabPointer(d, CurrentTime);
    printf("RESULT grab=%d core=%ld,%ld raw=%.0f,%.0f motions=%ld warps=%ld raws=%ld buttons=%ld "
           "jumps=%ld raw_sources=",
           g, core_x, core_y, raw_x, raw_y, motions, warps, raws, buttons, jumps);
    for (int k = 0; k < raw_devs; k++) {
        printf("%s%d", k ? "," : "", raw_dev[k]);
    }
    printf("\n");
    if (log) {
        fclose(log);
    }
    return 0;
}
