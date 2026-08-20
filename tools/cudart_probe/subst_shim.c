/* ★★★★★ Substitute OUR reply values into the WORKING host stack, after the real call.
 *
 * The refusal interposer answered "is control X necessary?". It could not answer "does our
 * WRONG VALUE for X break anything?" — and the host<->guest body differential says that is
 * where the defect lives (27 records status-equal, body-different).
 *
 * SUBST="cmd:word:value[,cmd:word:value...]"  — after the real ioctl returns, overwrite
 * params[word] with value, so the caller sees exactly what our guest would have seen.
 * This tests SUFFICIENCY of a wrong value, on a stack that is otherwise known-good.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/uio.h>

#define NV_ESC_RM_CONTROL 0x2a
struct nvos54 { uint32_t hc, ho, cmd, flags; uint64_t params; uint32_t psize, status; };

struct rule { uint32_t cmd, word, val; };
static struct rule g_r[1024]; static int g_n = -1; static long g_hits;

static void init(void) {
    g_n = 0;
    const char *s = getenv("SUBST");
    if (!s || !*s) return;
    size_t blen = strlen(s) + 1;
    char *buf = malloc(blen); if (!buf) return; memcpy(buf, s, blen);
    for (char *t = strtok(buf, ","); t && g_n < 1024; t = strtok(NULL, ",")) {
        unsigned long c, w, v;
        if (sscanf(t, "%li:%li:%li", &c, &w, &v) == 3) {
            g_r[g_n].cmd = (uint32_t)c; g_r[g_n].word = (uint32_t)w; g_r[g_n].val = (uint32_t)v; g_n++;
        }
    }
    fprintf(stderr, "[subst] %d rule(s) parsed\n", g_n);
    if (g_n >= 1024) fprintf(stderr, "[subst] ⚠ RULE TABLE FULL — SET WAS TRUNCATED\n");
}

int ioctl(int fd, unsigned long req, ...) {
    static int (*real)(int, unsigned long, ...);
    if (!real) real = dlsym(RTLD_NEXT, "ioctl");
    va_list ap; va_start(ap, req); void *argp = va_arg(ap, void *); va_end(ap);
    int rc = real(fd, req, argp);
    if (g_n < 0) init();
    if (g_n > 0 && ((req >> 8) & 0xff) == 70 && (req & 0xff) == NV_ESC_RM_CONTROL && argp) {
        struct nvos54 *p = argp;
        for (int i = 0; i < g_n; i++) {
            if (p->cmd != g_r[i].cmd) continue;
            if ((g_r[i].word + 1) * 4 > p->psize) continue;   /* never write past the struct */
            uint32_t v = g_r[i].val;
            struct iovec l = { &v, 4 };
            struct iovec r2 = { (char *)(uintptr_t)p->params + g_r[i].word * 4, 4 };
            if (process_vm_writev(getpid(), &l, 1, &r2, 1, 0) == 4) {
                g_hits++;
                fprintf(stderr, "[subst] cmd=%#010x word=%u <- %#x\n", p->cmd, g_r[i].word, v);
            }
        }
    }
    return rc;
}
