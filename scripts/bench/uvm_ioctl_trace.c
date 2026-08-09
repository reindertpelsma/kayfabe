// LD_PRELOAD interposer for the UVM ioctl plane — the one plane every other instrument
// this port owns is STRUCTURALLY BLIND to.
//
// ★★★ Why this file exists. `scripts/rpctrace/cuda_ioctl_trace.c:493` gates on
// `_IOC_TYPE(request) == NV_IOCTL_MAGIC` ('F' = 0x46) and `scripts/mode2_diag/
// nvioctl_trace.c` (C artifact) filters the same way. **UVM's ioctl magic is 0**, so
// `/dev/nvidia-uvm` traffic passes through both untouched. §14.39 measured that `cuInit`'s
// last productive syscall is `ioctl(9, UVM_IOCTL_BASE(37))` = `UVM_REGISTER_GPU` — i.e. the
// deciding call sits exactly in the filtered-out band. ⊘ The absence of UVM from every trace
// this project holds is an artefact of those filters, never evidence.
//
// ★★ And a return code cannot settle it. UVM's ioctl handler returns 0 at the syscall
// boundary for every call it dispatched at all and carries the real verdict in
// `params.rmStatus` (`ogkm-580: kernel-open/nvidia-uvm/uvm_ioctl.h:534-543` for
// `UVM_REGISTER_GPU_PARAMS`). `strace` prints `= 0` and says nothing. This interposer reads
// the struct AFTER the call and prints that field — which is the measurement.
//
// build:  gcc -O1 -fPIC -shared -o uvm_ioctl_trace.so uvm_ioctl_trace.c -ldl
// use:    UVMTRACE_OUT=/tmp/uvm.txt LD_PRELOAD=./uvm_ioctl_trace.so ./cup2
//
// ⚠ Safety of the dump. The ioctl argument is a caller pointer of a length this file cannot
// derive: `UVM_IOCTL_BASE(i)` is `_IOC(_IOC_NONE, 0, i, 0)`, so the size field is ZERO for
// every UVM command and the usual `_IOC_SIZE` trick yields nothing. Rather than guess a
// length and risk reading off the end of a stack object, every dump is probed first by
// `write()`ing the range to /dev/null: the kernel returns EFAULT instead of faulting us if
// any byte is unreadable. A range that does not survive the probe is reported as such and
// not read. ⊘ Never widen the dump without keeping the probe in front of it.

#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/ioctl.h>

static int (*real_ioctl)(int, unsigned long, ...);
static FILE *lf;
static int nullfd = -1;

// ★ The known UVM parameter blocks, from `ogkm-580: kernel-open/nvidia-uvm/uvm_ioctl.h` and
// `uvm_linux_ioctl.h`. `off_status` is the byte offset of the `NV_STATUS rmStatus` OUT field,
// which is the whole point of the file; -1 means the command has no such field. Offsets are
// laid out by hand for x86-64 against the header's declarations: `NvBool` is `NvU8`
// (`nvtypes.h:272`) so it does NOT imply four bytes, and members carrying
// `NV_ALIGN_BYTES(8)` force padding that a naive field count misses.
//
// ⊘ Every row below is transcribed from a struct read in full. A guessed size here would be
// silently wrong in exactly the way that reads a neighbouring field and calls it a status.
//
// ⚠ `key` is the FULL request word when the command does not come from `UVM_IOCTL_BASE`.
// On Linux `UVM_IOCTL_BASE(i)` is literally `i` (`uvm_ioctl.h:41`) — no direction, no type,
// no size — but `UVM_INITIALIZE`/`UVM_DEINITIALIZE` are the raw constants 0x30000001 /
// 0x30000002 (`uvm_linux_ioctl.h:32,40`), whose low byte COLLIDES with `UVM_RESERVE_VA` (1)
// and `UVM_RELEASE_VA` (2). Matching on `nr` alone would mislabel them and dump the wrong
// struct; the table is therefore keyed on the whole word.
struct uvm_cmd { unsigned long key; const char *name; unsigned size; int off_status; };
static const struct uvm_cmd CMDS[] = {
    // uuid[16] flags@16(8-aligned) ... — raw-constant commands first.
    { 0x30000001UL, "UVM_INITIALIZE",             16,  8 },  // NvU64 flags; NV_STATUS
    { 0x30000002UL, "UVM_DEINITIALIZE",            0, -1 },  // no argument at all
    // UVM_IOCTL_BASE(i) == i on Linux.
    {          1UL, "UVM_RESERVE_VA",             24, 16 },  // NvU64 base; NvU64 len; status
    {          2UL, "UVM_RELEASE_VA",             24, 16 },
    {         25UL, "UVM_REGISTER_GPU_VASPACE",   32, 28 },  // uuid16 fd20 hCl24 hVas.. st28
    {         26UL, "UVM_UNREGISTER_GPU_VASPACE", 20, 16 },
    {         27UL, "UVM_REGISTER_CHANNEL",       56, 48 },  // pad 28..31 before NvU64 base
    {         28UL, "UVM_UNREGISTER_CHANNEL",     28, 24 },
    {         34UL, "UVM_FREE",                   24, 16 },
    {         35UL, "UVM_MEM_MAP",                24, 16 },
    // ★★★ THE ONE THAT MATTERS: uuid[16] numaEnabled@16 (NvU8) pad numaNodeId@20 rmCtrlFd@24
    // hClient@28 hSmcPartRef@32 rmStatus@36 (`uvm_ioctl.h:534-543`).
    {         37UL, "UVM_REGISTER_GPU",           40, 36 },
    {         38UL, "UVM_UNREGISTER_GPU",         20, 16 },
    {         39UL, "UVM_PAGEABLE_MEM_ACCESS",     8,  4 },
};

static const struct uvm_cmd *lookup(unsigned long key)
{
    for (unsigned i = 0; i < sizeof(CMDS) / sizeof(CMDS[0]); i++)
        if (CMDS[i].key == key) return &CMDS[i];
    return NULL;
}

static void setup(void)
{
    if (!real_ioctl) real_ioctl = (int (*)(int, unsigned long, ...))dlsym(RTLD_NEXT, "ioctl");
    if (nullfd < 0)  nullfd = open("/dev/null", O_WRONLY | O_CLOEXEC);
    if (!lf) {
        const char *p = getenv("UVMTRACE_OUT");
        lf = fopen(p ? p : "/tmp/uvm_ioctl_trace.txt", "a");
        if (lf) setvbuf(lf, NULL, _IOLBF, 0);
    }
}

// Is `fd` open on a UVM node? Decided from /proc, not from a remembered open(), so an fd the
// process inherited or reopened is still classified correctly.
static int is_uvm_fd(int fd)
{
    char p[64], t[512];
    snprintf(p, sizeof p, "/proc/self/fd/%d", fd);
    ssize_t n = readlink(p, t, sizeof t - 1);
    if (n < 0) return 0;
    t[n] = 0;
    return strstr(t, "nvidia-uvm") != NULL;
}

// Probe readability WITHOUT reading: write() reports EFAULT rather than delivering SIGSEGV.
static int readable(const void *p, unsigned n)
{
    if (!p || !n || nullfd < 0) return 0;
    return write(nullfd, p, n) == (ssize_t)n;
}

static void dump(const char *tag, const unsigned char *p, unsigned n)
{
    fprintf(lf, "    %s", tag);
    for (unsigned i = 0; i < n; i++) {
        if (i % 16 == 0) fprintf(lf, "\n      +%02u:", i);
        fprintf(lf, " %02x", p[i]);
    }
    fprintf(lf, "\n");
}

int ioctl(int fd, unsigned long request, ...)
{
    va_list ap;
    void *arg;
    va_start(ap, request);
    arg = va_arg(ap, void *);
    va_end(ap);

    setup();
    if (!real_ioctl) { errno = ENOSYS; return -1; }

    unsigned type = (unsigned)((request >> 8) & 0xff);
    unsigned nr   = (unsigned)(request & 0xff);

    // ⊘ Type is the ONLY discriminator that is cheap; the fd check is what makes it sound.
    // A type-0 ioctl on a non-UVM fd is somebody else's and must pass through untouched.
    if (type != 0 || !lf || !is_uvm_fd(fd))
        return real_ioctl(fd, request, arg);

    const struct uvm_cmd *c = lookup(request);
    fprintf(lf, "UVM ioctl fd=%d cmd=0x%08lx nr=%u (%s) arg=%p\n",
            fd, request, nr, c ? c->name : "UNKNOWN — not in the table, size unknown", arg);

    if (c && c->size && readable(arg, c->size))
        dump("IN ", (const unsigned char *)arg, c->size);
    else if (c && c->size)
        fprintf(lf, "    IN  <%u bytes NOT READABLE at %p — not dumped>\n", c->size, arg);

    int rc = real_ioctl(fd, request, arg);
    int e = errno;

    fprintf(lf, "  -> rc=%d errno=%d\n", rc, rc < 0 ? e : 0);
    if (c && c->size && readable(arg, c->size)) {
        dump("OUT", (const unsigned char *)arg, c->size);
        if (c->off_status >= 0 && (unsigned)c->off_status + 4 <= c->size) {
            uint32_t st;
            memcpy(&st, (const unsigned char *)arg + c->off_status, 4);
            // ★★★ THE MEASUREMENT. rc=0 above says only that UVM dispatched the call.
            fprintf(lf, "  ★ %s rmStatus = 0x%08x %s\n", c->name, st,
                    st == 0 ? "(NV_OK)" : "<-- THE FAILURE, invisible to a return-code check");
        }
    }
    errno = e;
    return rc;
}
