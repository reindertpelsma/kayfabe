/* LD_PRELOAD ioctl tracer for nvidia 'F'-type escapes — decodes RM_CONTROL
 * (NVOS54) and RM_ALLOC (NVOS64) in+out so we can see the exact control whose
 * returned params libcuda derefs at the cuCtxCreate crash. Each line is flushed
 * immediately (write(2)) so the trace survives the SIGSEGV. Build:
 *   gcc -shared -fPIC -ldl -o ioctl_trace.so ioctl_trace.c
 * Run: NVKVM_TRACE=/tmp/ioctl_trace.log LD_PRELOAD=./ioctl_trace.so ./cup2 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <sys/syscall.h>

#define IOC_NR(c)   ((c) & 0xff)
#define IOC_TYPE(c) (((c) >> 8) & 0xff)
#define IOC_SIZE(c) (((c) >> 16) & 0x3fff)

static int (*real_ioctl)(int, unsigned long, ...);
static long (*real_syscall)(long, ...);
static int logfd = -1;

static void initlog(void)
{
    if (logfd >= 0) return;
    const char *p = getenv("NVKVM_TRACE");
    if (!p) p = "/tmp/ioctl_trace.log";
    logfd = open(p, O_WRONLY | O_CREAT | O_APPEND, 0644);
    if (!real_ioctl)   real_ioctl   = dlsym(RTLD_NEXT, "ioctl");
    if (!real_syscall) real_syscall = dlsym(RTLD_NEXT, "syscall");
    static int marked;
    if (!marked) { marked = 1;
        const char *m = "[ioctl_trace loaded]\n"; (void)write(2, m, 20);
        if (logfd >= 0) (void)write(logfd, m, 20);
    }
}

static void emit(const char *buf, int n) { if (logfd >= 0) (void)write(logfd, buf, n); }

/* hex-dump up to n bytes of a param buffer into the line */
static int hexcat(char *o, int cap, const uint8_t *p, int n)
{
    int k = 0;
    for (int i = 0; i < n && k < cap - 4; i++) k += snprintf(o + k, cap - k, "%02x", p[i]);
    return k;
}

/* Log the IN side of an nvidia 'F' escape. RM_CONTROL=0x2a (NVOS54: hClient@0,
 * hObject@4,cmd@8,flags@12,params@16(u64),paramsSize@24,status@28). */
static void trace_in(unsigned nr, unsigned sz, void *arg)
{
    char line[8192];
    if (nr == 0x2a && arg) {
        uint8_t *a = arg;
        uint64_t pp = *(uint64_t *)(a + 16);
        const uint8_t *params = (const uint8_t *)(uintptr_t)pp;
        uint32_t psize = *(uint32_t *)(a + 24);
        int n = snprintf(line, sizeof line,
            "CTRL  cmd=0x%08x hClient=0x%08x hObject=0x%08x psize=%u IN[",
            *(uint32_t *)(a + 8), *(uint32_t *)(a + 0), *(uint32_t *)(a + 4), psize);
        if (params && psize) n += hexcat(line + n, sizeof line - n, params, psize < 1024 ? psize : 1024);
        n += snprintf(line + n, sizeof line - n, "]\n");
        emit(line, n);
    }
}

/* Log the OUT side after the real call. */
static void trace_out(unsigned nr, unsigned sz, void *arg, long rc)
{
    char line[8192];
    if (nr == 0x2a && arg) {
        uint8_t *a = arg;
        uint64_t pp = *(uint64_t *)(a + 16);
        const uint8_t *params = (const uint8_t *)(uintptr_t)pp;
        uint32_t psize = *(uint32_t *)(a + 24);
        int n = snprintf(line, sizeof line, "CTRL= cmd=0x%08x rc=%ld status=0x%x OUT[",
            *(uint32_t *)(a + 8), rc, *(uint32_t *)(a + 28));
        if (params && psize) n += hexcat(line + n, sizeof line - n, params, psize < 1024 ? psize : 1024);
        n += snprintf(line + n, sizeof line - n, "]\n");
        emit(line, n);
    } else if (nr == 0x2b && arg) {
        uint8_t *a = arg;     /* NVOS64: hRoot@0,hParent@4,hNew@8,hClass@12,
                               * pAllocParms@16(u64),pRightsReq@24,paramsSize@32,flags@36 */
        uint32_t hClass = *(uint32_t *)(a + 12);
        uint64_t pap = *(uint64_t *)(a + 16);
        uint32_t paps = *(uint32_t *)(a + 32);
        int n = snprintf(line, sizeof line,
            "ALLOC hClass=0x%04x hNew=0x%08x rc=%ld NVOS64[",
            hClass, *(uint32_t *)(a + 8), rc);
        n += hexcat(line + n, sizeof line - n, a, 48);
        n += snprintf(line + n, sizeof line - n, "] parms(sz=%u)[", paps);
        if (pap && paps) n += hexcat(line + n, sizeof line - n,
                                     (const uint8_t *)(uintptr_t)pap, paps < 64 ? paps : 64);
        n += snprintf(line + n, sizeof line - n, "]\n");
        emit(line, n);
    } else {
        int n = snprintf(line, sizeof line, "ESC   nr=0x%02x size=%u rc=%ld\n", nr, sz, rc);
        emit(line, n);
    }
}

int ioctl(int fd, unsigned long request, ...)
{
    initlog();
    va_list ap; va_start(ap, request); void *arg = va_arg(ap, void *); va_end(ap);
    unsigned nr = IOC_NR(request), ty = IOC_TYPE(request), sz = IOC_SIZE(request);
    if (ty == 'F') trace_in(nr, sz, arg);
    int rc = real_ioctl(fd, request, arg);
    if (ty == 'F') trace_out(nr, sz, arg, rc);
    return rc;
}

/* libcuda often issues ioctls via syscall(SYS_ioctl,...) to dodge libc interposition. */
long syscall(long n, ...)
{
    initlog();
    va_list ap; va_start(ap, n);
    long a1 = va_arg(ap, long), a2 = va_arg(ap, long), a3 = va_arg(ap, long),
         a4 = va_arg(ap, long), a5 = va_arg(ap, long), a6 = va_arg(ap, long);
    va_end(ap);
    if (n == SYS_ioctl) {
        unsigned long request = (unsigned long)a2; void *arg = (void *)a3;
        unsigned nr = IOC_NR(request), ty = IOC_TYPE(request), sz = IOC_SIZE(request);
        if (ty == 'F') trace_in(nr, sz, arg);
        long rc = real_syscall(n, a1, a2, a3, a4, a5, a6);
        if (ty == 'F') trace_out(nr, sz, arg, rc);
        return rc;
    }
    return real_syscall(n, a1, a2, a3, a4, a5, a6);
}
