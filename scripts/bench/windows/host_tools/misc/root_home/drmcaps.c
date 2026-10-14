/* Report the two DRM caps gamescope's DRM backend requires before it will
 * drive a connector, plus the ones it commonly checks alongside them. */
#include <stdio.h>
#include <fcntl.h>
#include <stdint.h>
#include <unistd.h>
#include <xf86drm.h>
#ifndef DRM_CAP_SYNCOBJ
#define DRM_CAP_SYNCOBJ 0x13
#endif
#ifndef DRM_CAP_SYNCOBJ_TIMELINE
#define DRM_CAP_SYNCOBJ_TIMELINE 0x14
#endif
#ifndef DRM_CAP_ASYNC_PAGE_FLIP
#define DRM_CAP_ASYNC_PAGE_FLIP 0x7
#endif
#ifndef DRM_CAP_ATOMIC_ASYNC_PAGE_FLIP
#define DRM_CAP_ATOMIC_ASYNC_PAGE_FLIP 0x15
#endif
static void one(int fd, uint64_t cap, const char *name) {
    uint64_t v = 0;
    int rc = drmGetCap(fd, cap, &v);
    printf("  %-32s %s\n", name, rc ? "UNSUPPORTED (drmGetCap failed)" : (v ? "yes" : "no"));
}
int main(int argc, char **argv) {
    const char *dev = argc > 1 ? argv[1] : "/dev/dri/card0";
    int fd = open(dev, O_RDWR | O_CLOEXEC);
    if (fd < 0) { perror(dev); return 1; }
    printf("device: %s\n", dev);
    drmSetClientCap(fd, DRM_CLIENT_CAP_ATOMIC, 1);
    one(fd, DRM_CAP_SYNCOBJ, "DRM_CAP_SYNCOBJ");
    one(fd, DRM_CAP_SYNCOBJ_TIMELINE, "DRM_CAP_SYNCOBJ_TIMELINE");
    one(fd, DRM_CAP_ASYNC_PAGE_FLIP, "DRM_CAP_ASYNC_PAGE_FLIP");
    one(fd, DRM_CAP_ATOMIC_ASYNC_PAGE_FLIP, "DRM_CAP_ATOMIC_ASYNC_PAGE_FLIP");
    close(fd);
    return 0;
}
