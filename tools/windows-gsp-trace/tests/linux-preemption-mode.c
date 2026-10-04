/* Native Linux experiment, NOT a Kayfabe host verb or a production workaround.
 * After a successful application SET_CTXSW_PREEMPTION_MODE, issue an additional
 * graphics-mode request on that application's own channel. Preserve the original
 * call/result. No registry changes; scope ends when the process destroys channels.
 *
 * cc -shared -fPIC -O2 -Wall -Wextra -Werror linux-preemption-mode.c -o mode.so -ldl
 * KF_GFX_MODE=0|1|2 LD_PRELOAD=./mode.so application
 * JSON diagnostics on stderr distinguish a refused request from an enabled mode.
 * ABI: OGKM 580.65.06, ctrl2080gr.h and nvos.h; NVOS54 is 32 bytes on x86-64.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/uio.h>
#include <unistd.h>

struct control {
    uint32_t client, object, command, flags;
    uint64_t params;
    uint32_t size, status;
};
struct mode {
    uint32_t flags, channel, graphics, compute;
    uint64_t route[2];
};
_Static_assert(sizeof(struct control) == 32, "NVOS54");
_Static_assert(sizeof(struct mode) == 32, "preemption mode");
static int (*next_ioctl)(int, unsigned long, void *);
static int requested_mode = -1;
static unsigned calls;

__attribute__((constructor)) static void init(void)
{
    *(void **)(&next_ioctl) = dlsym(RTLD_NEXT, "ioctl");
    if (!next_ioctl) _exit(125);
    const char *s = getenv("KF_GFX_MODE");
    if (s && s[0] >= '0' && s[0] <= '2' && s[1] == '\0')
        requested_mode = s[0] - '0';
}

static int read_self(void *dst, const void *src, size_t n)
{
    struct iovec local = {dst, n}, remote = {(void *)src, n};
    return process_vm_readv(getpid(), &local, 1, &remote, 1, 0) == (ssize_t)n;
}

int ioctl(int fd, unsigned long req, ...)
{
    va_list ap;
    va_start(ap, req);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    int rc = next_ioctl(fd, req, arg);
    int saved_errno = errno;
    struct control original;
    struct mode before;
    if (requested_mode < 0 || rc != 0 || req != 0xc020462aUL ||
        !read_self(&original, arg, sizeof original) || original.status != 0 ||
        original.command != 0x20801210 || original.size != sizeof before ||
        !read_self(&before, (void *)(uintptr_t)original.params, sizeof before))
        goto done;
    char link[64], path[128];
    snprintf(link, sizeof link, "/proc/self/fd/%d", fd);
    ssize_t len = readlink(link, path, sizeof path - 1);
    if (len < 0) goto done;
    path[len] = 0;
    if (strcmp(path, "/dev/nvidiactl") != 0) goto done;
    unsigned index = __atomic_fetch_add(&calls, 1, __ATOMIC_RELAXED);
    if (index >= 32) goto done;
    /* Same owned handles, but a fresh, pointer-free payload and zero MIG route. */
    if (before.route[0] || before.route[1]) goto done;
    struct mode p = {.flags = 2, .channel = before.channel,
                     .graphics = (uint32_t)requested_mode};
    struct control c = {.client = original.client, .object = original.object,
        .command = 0x20801210, .params = (uintptr_t)&p, .size = sizeof p};
    errno = 0;
    int extra_rc = next_ioctl(fd, req, &c);
    int extra_errno = errno;
    char line[512];
    int n = snprintf(line, sizeof line,
        "{\"experiment\":\"linux-gfx-mode\",\"uid\":%u,\"index\":%u,"
        "\"client\":%u,\"object\":%u,\"channel\":%u,\"original_flags\":%u,"
        "\"original_graphics\":%u,\"original_compute\":%u,"
        "\"requested_graphics\":%d,\"rc\":%d,\"errno\":%d,\"status\":%u}\n",
        (unsigned)geteuid(), index, original.client, original.object, p.channel,
        before.flags, before.graphics, before.compute, requested_mode,
        extra_rc, extra_errno, c.status);
    if (n > 0 && (size_t)n < sizeof line) {
        ssize_t written = write(STDERR_FILENO, line, (size_t)n);
        (void)written;
    }
done:
    errno = saved_errno;
    return rc;
}
