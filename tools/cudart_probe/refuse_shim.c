/* Refuse a named set of RM controls in-band, exactly as our device does to the guest:
 * ioctl() returns 0, and NVOS54_PARAMETERS.status is set to NV_ERR_NOT_SUPPORTED (0x56).
 * This is a CAUSALITY probe on bare metal — no guest, no emulator. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>

#define NV_ESC_RM_CONTROL 0x2a
#define NV_ERR_NOT_SUPPORTED 0x56

/* NVOS54_PARAMETERS: hClient,hObject,cmd,flags @0,4,8,12; params@16; paramsSize@24; status@28 */
struct nvos54 { uint32_t hc, ho, cmd, flags; uint64_t params; uint32_t psize, status; };

static uint32_t g_set[64]; static int g_n = -1; static long g_hits = 0;

static void init_set(void) {
    g_n = 0;
    const char *s = getenv("REFUSE_CMDS");
    if (!s) return;
    char buf[512]; snprintf(buf, sizeof buf, "%s", s);
    for (char *t = strtok(buf, ","); t && g_n < 64; t = strtok(NULL, ","))
        g_set[g_n++] = (uint32_t)strtoul(t, NULL, 0);
}

int ioctl(int fd, unsigned long req, ...) {
    static int (*real)(int, unsigned long, ...);
    if (!real) real = dlsym(RTLD_NEXT, "ioctl");
    va_list ap; va_start(ap, req); void *argp = va_arg(ap, void *); va_end(ap);
    if (g_n < 0) init_set();
    if (g_n > 0 && ((req >> 8) & 0xff) == 70 && (req & 0xff) == NV_ESC_RM_CONTROL && argp) {
        struct nvos54 *p = argp;
        for (int i = 0; i < g_n; i++)
            if (p->cmd == g_set[i]) {          /* refuse WITHOUT calling through */
                p->status = NV_ERR_NOT_SUPPORTED;
                g_hits++;
                fprintf(stderr, "[refuse] cmd=%#010x -> status=0x56 (hit %ld)\n", p->cmd, g_hits);
                return 0;
            }
    }
    return real(fd, req, argp);
}
