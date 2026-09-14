/* ★★★★★ w704 — LOG THE UVM MAP PARAMETERS *BEFORE* THE CALL.
 *
 * `[measured w703]` cup3 makes six `UVM_MAP_EXTERNAL_ALLOCATION` calls that RETURN and a
 * seventh that never does (~10 min, 580 s of kernel CPU, `clear_page_rep` 42 %). The whole
 * question is what distinguishes #7 — and that is in its parameters.
 *
 * ⊘ `strace` cannot answer it: for an UNFINISHED call it prints no argument pointer at all
 * (`ioctl(9, _IOC(_IOC_NONE, 0, 0x21, 0) <unfinished ...>`), and reading the neighbouring
 * completed call's buffer out of `/proc/pid/mem` gave `base=0 length=1 offset=0xc1d0…` — an RM
 * client handle, i.e. the wrong buffer. So the parameters must be captured by the caller,
 * BEFORE the kernel is entered, which is what this does.
 *
 * ⚠ Prints before AND after, so a call that never returns is visible as a `>>>` with no `<<<`.
 * That asymmetry IS the measurement; a logger that only printed on return would be silent for
 * precisely the call under investigation.
 *
 * Same shape as the research tree's `tests/mode2/nvdiff/nvdiff_shim.c`.
 *   build: gcc -shared -fPIC -o uvmparams.so uvmparams_shim.c -ldl
 *   use:   LD_PRELOAD=./uvmparams.so ./cup3
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>

/* nvidia-uvm ioctls are raw integers with no size encoding (`uvm_ioctl.h`: on Linux
 * `UVM_IOCTL_BASE(i)` is `i`). */
#define UVM_CREATE_EXTERNAL_RANGE 73
#define UVM_MAP_EXTERNAL_ALLOCATION 33

static int (*real_ioctl)(int, unsigned long, ...);
static unsigned long n_map;

int ioctl(int fd, unsigned long req, ...) {
    va_list ap;
    void *arg;
    va_start(ap, req);
    arg = va_arg(ap, void *);
    va_end(ap);
    if (!real_ioctl) real_ioctl = dlsym(RTLD_NEXT, "ioctl");

    /* Both structs begin `{ NvU64 base; NvU64 length; ... }`, which is all we need. */
    if ((req == UVM_MAP_EXTERNAL_ALLOCATION || req == UVM_CREATE_EXTERNAL_RANGE) && arg) {
        const uint64_t *p = (const uint64_t *)arg;
        const char *name = (req == UVM_MAP_EXTERNAL_ALLOCATION) ? "MAP_EXTERNAL " : "CREATE_RANGE ";
        unsigned long n = (req == UVM_MAP_EXTERNAL_ALLOCATION) ? ++n_map : n_map + 1;
        fprintf(stderr, "UVMPARAM >>> #%lu %s base=0x%016lx length=0x%lx (%lu KiB)\n",
                n, name, (unsigned long)p[0], (unsigned long)p[1],
                (unsigned long)(p[1] >> 10));
        fflush(stderr);
        int r = real_ioctl(fd, req, arg);
        fprintf(stderr, "UVMPARAM <<< #%lu %s returned %d\n", n, name, r);
        fflush(stderr);
        return r;
    }
    return real_ioctl(fd, req, arg);
}
