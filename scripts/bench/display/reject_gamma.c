/* Guest-only fault injection: refuse nonzero GAMMA_LUT atomic requests.
 * Startup/reset with a null LUT and every other ioctl go to the stock driver.
 * This probes compositor behavior when KMS reports a missing color operation;
 * it implements no color transform and is never loaded in the VMM or host. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdint.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <xf86drm.h>
#include <xf86drmMode.h>

static uint32_t gamma_property(int fd)
{
    uint32_t id = 0;
    drmModeResPtr res = drmModeGetResources(fd);
    if (!res) return 0;
    for (int i = 0; i < res->count_crtcs && !id; i++) {
        drmModeObjectPropertiesPtr ps = drmModeObjectGetProperties(fd, res->crtcs[i], DRM_MODE_OBJECT_CRTC);
        for (uint32_t j = 0; ps && j < ps->count_props; j++) {
            drmModePropertyPtr p = drmModeGetProperty(fd, ps->props[j]);
            if (p && !strcmp(p->name, "GAMMA_LUT")) id = p->prop_id;
            if (p) drmModeFreeProperty(p);
        }
        if (ps) drmModeFreeObjectProperties(ps);
    }
    drmModeFreeResources(res);
    return id;
}

static int (*next_ioctl)(int, unsigned long, ...);

__attribute__((constructor)) static void arm(void)
{
    next_ioctl = dlsym(RTLD_NEXT, "ioctl");
    fprintf(stderr, "COLOR_FAULT armed guest-only ioctl control\n");
}

int ioctl(int fd, unsigned long request, ...)
{
    va_list ap;
    va_start(ap, request);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    static unsigned logs;
    if (!next_ioctl) { errno = ENOSYS; return -1; }
    if (request == DRM_IOCTL_MODE_ATOMIC && arg) {
        const struct drm_mode_atomic *a = arg;
        if (a->count_objs <= 256 && a->count_props_ptr && a->props_ptr && a->prop_values_ptr) {
            const uint32_t *counts = (const void *)(uintptr_t)a->count_props_ptr;
            const uint32_t *props = (const void *)(uintptr_t)a->props_ptr;
            const uint64_t *values = (const void *)(uintptr_t)a->prop_values_ptr;
            uint32_t gamma = gamma_property(fd), k = 0;
            for (uint32_t i = 0; i < a->count_objs; i++) {
                if (counts[i] > 256 || k + counts[i] > 4096) break;
                for (uint32_t j = 0; j < counts[i]; j++, k++) {
                    if (gamma && props[k] == gamma && values[k]) {
                        if (logs < 8) {
                            logs++;
                            fprintf(stderr, "COLOR_FAULT reject GAMMA_LUT prop=%u flags=%u\n", gamma, a->flags);
                        }
                        errno = EOPNOTSUPP;
                        return -1;
                    }
                }
            }
        }
    }
    return next_ioctl(fd, request, arg);
}
