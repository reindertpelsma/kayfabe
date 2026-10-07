/* Read-only KMS color-state audit. Run while the compositor owns the card.
 * The blob contents are interpreted only as bounded drm_color_lut entries;
 * this does not claim that the display hardware applied them. */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <xf86drm.h>
#include <xf86drmMode.h>

static void inspect(int fd, uint32_t id, uint32_t type, int dump)
{
    drmModeObjectPropertiesPtr props = drmModeObjectGetProperties(fd, id, type);
    if (!props) return;
    for (uint32_t i = 0; i < props->count_props; i++) {
        drmModePropertyPtr p = drmModeGetProperty(fd, props->props[i]);
        if (!p) continue;
        if (strstr(p->name, "LUT") || strstr(p->name, "CTM") ||
            strstr(p->name, "TF") || strstr(p->name, "MULTIPLIER")) {
            uint64_t v = props->prop_values[i];
            printf("COLOR_PROPERTY object=%u type=%u name=%s value=%llu", id, type,
                   p->name, (unsigned long long)v);
            if ((p->flags & DRM_MODE_PROP_BLOB) && v && v <= UINT32_MAX) {
                drmModePropertyBlobPtr b = drmModeGetPropertyBlob(fd, (uint32_t)v);
                if (b && b->data && b->length <= 131072) {
                    printf(" bytes=%u", b->length);
                    if (strstr(p->name, "LUT") && b->length >= 8 && b->length % 8 == 0) {
                        const struct drm_color_lut *lut = b->data;
                        size_t n = b->length / sizeof(*lut);
                        printf(" entries=%zu midpoint=%u,%u,%u endpoint=%u,%u,%u", n,
                               lut[n/2].red, lut[n/2].green, lut[n/2].blue,
                               lut[n-1].red, lut[n-1].green, lut[n-1].blue);
                        if (dump && n <= 1025) {
                            for (size_t j = 0; j < n; j++)
                                printf("\nCOLOR_LUT object=%u name=%s index=%zu rgb=%u,%u,%u", id,
                                       p->name, j, lut[j].red, lut[j].green, lut[j].blue);
                        }
                    }
                }
                if (b) drmModeFreePropertyBlob(b);
            }
            putchar('\n');
        }
        drmModeFreeProperty(p);
    }
    drmModeFreeObjectProperties(props);
}

int main(int argc, char **argv)
{
    if (argc != 2 && (argc != 3 || strcmp(argv[2], "--dump-lut"))) return 2;
    int fd = open(argv[1], O_RDWR | O_CLOEXEC);
    if (fd < 0) { fprintf(stderr, "open: %s\n", strerror(errno)); return 3; }
    drmVersionPtr ver = drmGetVersion(fd);
    printf("COLOR_READER uid=%u driver=%.*s master=%d\n", (unsigned)getuid(),
           ver ? ver->name_len : 0, ver ? ver->name : "", drmIsMaster(fd));
    if (ver) drmFreeVersion(ver);
    drmSetClientCap(fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1);
    drmSetClientCap(fd, DRM_CLIENT_CAP_ATOMIC, 1);
    drmModeResPtr res = drmModeGetResources(fd);
    if (!res) { close(fd); return 4; }
    for (int i = 0; i < res->count_crtcs; i++) {
        drmModeCrtcPtr c = drmModeGetCrtc(fd, res->crtcs[i]);
        printf("COLOR_CRTC id=%u active=%d gamma_size=%d\n", res->crtcs[i],
               c ? c->mode_valid : 0, c ? c->gamma_size : 0);
        if (c) drmModeFreeCrtc(c);
        inspect(fd, res->crtcs[i], DRM_MODE_OBJECT_CRTC, argc == 3);
    }
    drmModePlaneResPtr planes = drmModeGetPlaneResources(fd);
    for (uint32_t i = 0; planes && i < planes->count_planes; i++)
        inspect(fd, planes->planes[i], DRM_MODE_OBJECT_PLANE, argc == 3);
    if (planes) drmModeFreePlaneResources(planes);
    drmModeFreeResources(res);
    close(fd);
    return 0;
}
