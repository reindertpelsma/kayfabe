/* Guest-only explicit KMS TMO experiment. Loaded into Sway, never the host/VMM.
 * A fixed zero-intensity 1024-entry UNORM16 curve is requested on its active
 * primary plane. The stock NVIDIA driver owns conversion/allocation/submission.
 * Missing NV_PLANE_TMO_LUT is an error, never a request to use a shader substitute.
 * Control file: /home/ubuntu/color/tmo-mode = off / on / restore.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <xf86drm.h>
#include <xf86drmMode.h>

#define MAX_OBJECTS 64
#define MAX_PROPERTIES 1024
static int (*next_ioctl)(int, unsigned long, ...);
static unsigned logs;

static void note(const char *kind, uint32_t plane, uint32_t prop,
                 uint32_t blob, uint32_t flags, int rc, int error)
{
    if (logs < 32) {
        ++logs;
        fprintf(stderr, "TMO_TEST %s plane=%u prop=%u blob=%u flags=%u test_only=%u rc=%d errno=%d\n",
                kind, plane, prop, blob, flags, !!(flags & DRM_MODE_ATOMIC_TEST_ONLY), rc, error);
    }
}

__attribute__((constructor)) static void arm(void)
{
    next_ioctl = dlsym(RTLD_NEXT, "ioctl");
    fprintf(stderr, "TMO_TEST armed curve=zero_unorm16 entries=1024 guest_only=1\n");
}

static int mode(void)
{
    char text[9] = {0};
    int fd = open("/home/ubuntu/color/tmo-mode", O_RDONLY | O_CLOEXEC);
    if (fd < 0) return -1;
    ssize_t n = read(fd, text, sizeof(text) - 1);
    close(fd);
    if (n == 4 && !memcmp(text, "off\n", 4)) return 0;
    if (n == 3 && !memcmp(text, "on\n", 3)) return 1;
    if (n == 8 && !memcmp(text, "restore\n", 8)) return 2;
    return -1;
}

/* Only source properties exposed by this guest's stock KMS driver are used. */
static int primary(int fd, uint32_t object, uint32_t *tmo, uint32_t *fb,
                   uint64_t *current_fb)
{
    int yes = 0;
    drmModeObjectPropertiesPtr ps = drmModeObjectGetProperties(fd, object, DRM_MODE_OBJECT_PLANE);
    if (!ps) return 0;
    if (ps->count_props > 64) {
        drmModeFreeObjectProperties(ps);
        return -1;
    }
    for (uint32_t i = 0; i < ps->count_props; ++i) {
        drmModePropertyPtr p = drmModeGetProperty(fd, ps->props[i]);
        if (!p) continue;
        if (!strcmp(p->name, "type") && ps->prop_values[i] == DRM_PLANE_TYPE_PRIMARY) yes = 1;
        if (!strcmp(p->name, "NV_PLANE_TMO_LUT")) *tmo = p->prop_id;
        if (!strcmp(p->name, "FB_ID")) { *fb = p->prop_id; *current_fb = ps->prop_values[i]; }
        drmModeFreeProperty(p);
    }
    drmModeFreeObjectProperties(ps);
    return yes;
}

int ioctl(int fd, unsigned long request, ...)
{
    va_list ap;
    va_start(ap, request);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    if (!next_ioctl) { errno = ENOSYS; return -1; }
    if (request != DRM_IOCTL_MODE_ATOMIC || !arg) return next_ioctl(fd, request, arg);
    int m = mode();
    if (!m) return next_ioctl(fd, request, arg);
    const struct drm_mode_atomic *a = arg;
    if (m < 0 || !a->count_objs || a->count_objs > MAX_OBJECTS ||
        !a->objs_ptr || !a->count_props_ptr || !a->props_ptr || !a->prop_values_ptr) {
        note("INVALID_REQUEST", 0, 0, 0, a->flags, -1, EINVAL);
        errno = EINVAL;
        return -1;
    }
    const uint32_t *objects = (const void *)(uintptr_t)a->objs_ptr;
    const uint32_t *counts = (const void *)(uintptr_t)a->count_props_ptr;
    const uint32_t *props = (const void *)(uintptr_t)a->props_ptr;
    const uint64_t *values = (const void *)(uintptr_t)a->prop_values_ptr;
    uint32_t total = 0, chosen = MAX_OBJECTS, property = 0;
    for (uint32_t i = 0; i < a->count_objs; ++i) {
        if (counts[i] > MAX_PROPERTIES - total - 1) { errno = E2BIG; return -1; }
        uint32_t tmo = 0, fb = 0;
        uint64_t fb_value = 0;
        int is_primary = primary(fd, objects[i], &tmo, &fb, &fb_value);
        if (is_primary < 0) { errno = E2BIG; return -1; }
        for (uint32_t j = 0; j < counts[i]; ++j)
            if (fb && props[total + j] == fb) fb_value = values[total + j];
        if (is_primary && fb_value && chosen == MAX_OBJECTS) {
            if (!tmo && m == 1) {
                note("MISSING_TMO_LUT", objects[i], 0, 0, a->flags, -1, EOPNOTSUPP);
                errno = EOPNOTSUPP;
                return -1;
            }
            if (tmo) { chosen = i; property = tmo; }
        }
        total += counts[i];
    }
    if (chosen == MAX_OBJECTS) return next_ioctl(fd, request, arg);

    uint32_t blob = 0;
    if (m == 1) {
        const struct drm_color_lut curve[1024] = {{0}};
        if (drmModeCreatePropertyBlob(fd, curve, sizeof(curve), &blob)) {
            int error = errno;
            note("BLOB_FAILED", objects[chosen], property, 0, a->flags, -1, error);
            errno = error;
            return -1;
        }
    }
    uint32_t new_counts[MAX_OBJECTS], new_props[MAX_PROPERTIES];
    uint64_t new_values[MAX_PROPERTIES];
    uint32_t src = 0, dst = 0;
    for (uint32_t i = 0; i < a->count_objs; ++i) {
        int replaced = 0;
        new_counts[i] = counts[i];
        for (uint32_t j = 0; j < counts[i]; ++j, ++src, ++dst) {
            new_props[dst] = props[src];
            new_values[dst] = values[src];
            if (i == chosen && props[src] == property) {
                new_values[dst] = blob;
                replaced = 1;
            }
        }
        if (i == chosen && !replaced) {
            new_props[dst] = property;
            new_values[dst++] = blob;
            ++new_counts[i];
        }
    }
    struct drm_mode_atomic changed = *a;
    changed.count_props_ptr = (uintptr_t)new_counts;
    changed.props_ptr = (uintptr_t)new_props;
    changed.prop_values_ptr = (uintptr_t)new_values;
    int rc = next_ioctl(fd, request, &changed);
    int error = rc ? errno : 0;
    note(m == 1 ? "REQUEST" : "RESTORE", objects[chosen], property, blob, a->flags, rc, error);
    if (blob) drmModeDestroyPropertyBlob(fd, blob);
    errno = error;
    return rc;
}
