/*
 * drmcap.c -- can this process READ the host's scanout framebuffer?
 *
 * The interesting path is not modesetting (which is master-gated) but CAPTURE:
 *   GETRESOURCES -> CRTC -> fb_id -> GETFB -> GEM handle -> mmap / PRIME export
 * A GEM handle to the scanout buffer is a read of whatever is on the screen.
 *
 * Reports exactly which step refuses, and with what errno, so the answer is
 * "gated by X" rather than "seems to not work".
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <sys/mman.h>

struct dcard_res { uint64_t fb_id_ptr, crtc_id_ptr, connector_id_ptr, encoder_id_ptr;
                   uint32_t count_fbs, count_crtcs, count_connectors, count_encoders;
                   uint32_t min_width, max_width, min_height, max_height; };
struct dcrtc { uint64_t set_connectors_ptr; uint32_t count_connectors, crtc_id, fb_id,
               x, y, gamma_size, mode_valid; uint8_t mode[68]; };
struct dfb_cmd { uint32_t fb_id, width, height, pitch, bpp, depth, handle; };
struct dprime { uint32_t handle, flags; int32_t fd; };

#define IOC_RES   _IOWR('d', 0xA0, struct dcard_res)
#define IOC_CRTC  _IOWR('d', 0xA1, struct dcrtc)
#define IOC_GETFB _IOWR('d', 0xAD, struct dfb_cmd)
#define IOC_H2FD  _IOWR('d', 0x2D, struct dprime)
#define IOC_SETMASTER _IO('d', 0x1E)

int main(int argc, char **argv) {
    const char *node = argc > 1 ? argv[1] : "/dev/dri/card0";
    int fd = open(node, O_RDWR | O_CLOEXEC);
    if (fd < 0) { printf("open(%s): %s\n", node, strerror(errno)); return 1; }
    printf("open(%s): OK\n", node);

    printf("SET_MASTER: %s\n",
           ioctl(fd, IOC_SETMASTER, 0) == 0 ? "GRANTED (we are master!)" : strerror(errno));

    struct dcard_res r; memset(&r, 0, sizeof r);
    if (ioctl(fd, IOC_RES, &r)) { printf("GETRESOURCES: %s\n", strerror(errno)); return 1; }
    printf("GETRESOURCES: OK  crtcs=%u fbs=%u connectors=%u\n",
           r.count_crtcs, r.count_fbs, r.count_connectors);

    uint32_t crtcs[16] = {0};
    if (r.count_crtcs > 16) r.count_crtcs = 16;
    r.crtc_id_ptr = (uint64_t)(uintptr_t)crtcs;
    r.fb_id_ptr = r.connector_id_ptr = r.encoder_id_ptr = 0;
    r.count_fbs = r.count_connectors = r.count_encoders = 0;
    if (ioctl(fd, IOC_RES, &r)) { printf("GETRESOURCES(2): %s\n", strerror(errno)); return 1; }

    int found = 0;
    for (unsigned i = 0; i < r.count_crtcs; i++) {
        struct dcrtc c; memset(&c, 0, sizeof c); c.crtc_id = crtcs[i];
        if (ioctl(fd, IOC_CRTC, &c)) continue;
        if (!c.fb_id) { printf("  crtc %u: no active framebuffer\n", crtcs[i]); continue; }
        found = 1;
        printf("  crtc %u: ACTIVE fb_id=%u  (something is on screen)\n", crtcs[i], c.fb_id);

        struct dfb_cmd f; memset(&f, 0, sizeof f); f.fb_id = c.fb_id;
        if (ioctl(fd, IOC_GETFB, &f)) {
            printf("    GETFB -> REFUSED: %s   <-- capture blocked here\n", strerror(errno));
            continue;
        }
        printf("    GETFB -> handle=%u %ux%u pitch=%u bpp=%u  <-- GOT A HANDLE\n",
               f.handle, f.width, f.height, f.pitch, f.bpp);
        if (!f.handle) { printf("    (handle 0 = kernel withheld it)\n"); continue; }

        struct dprime p; memset(&p, 0, sizeof p); p.handle = f.handle; p.flags = O_RDONLY;
        if (ioctl(fd, IOC_H2FD, &p))
            printf("    PRIME export -> REFUSED: %s\n", strerror(errno));
        else
            printf("    PRIME export -> fd=%d  <-- SCREEN CONTENTS READABLE\n", p.fd);
    }
    if (!found) printf("  (no CRTC has an active framebuffer on this node)\n");
    close(fd);
    return 0;
}
