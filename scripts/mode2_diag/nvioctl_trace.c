/* nvioctl_trace.so — LD_PRELOAD ioctl tracer for NVIDIA RM ioctls.
 * Decodes NVOS54 (RM_CONTROL, NR 0x2A) and NVOS21/64 (RM_ALLOC, NR 0x2B) so a guest vs
 * host run can be diffed to find the divergent ioctl (status/paramsSize/sequence).
 * No driver changes; runs identically on guest cup2 and host cup2_host.
 *   build:  gcc -shared -fPIC -O2 -o nvioctl_trace.so nvioctl_trace.c -ldl
 *   run:    LD_PRELOAD=./nvioctl_trace.so NVTRACE=/tmp/trace.txt <prog>
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdarg.h>
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
static int (*real_ioctl)(int, unsigned long, ...);
static FILE *lg;
static int hexval(int c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}
static unsigned patch_from_hex_file(void *dst, unsigned max, const char *path)
{
    FILE *f = fopen(path, "r");
    unsigned n = 0;
    int hi = -1;
    if (!f) return 0;
    for (;;) {
        int c = fgetc(f);
        if (c == EOF) break;
        int v = hexval(c);
        if (v < 0) continue;
        if (hi < 0) {
            hi = v;
        } else {
            if (n >= max) break;
            ((unsigned char *)dst)[n++] = (unsigned char)((hi << 4) | v);
            hi = -1;
        }
    }
    fclose(f);
    return n;
}
__attribute__((constructor)) static void init(void){
    real_ioctl = dlsym(RTLD_NEXT, "ioctl");
    const char *p = getenv("NVTRACE");
    lg = p ? fopen(p, "w") : stderr;
}
int ioctl(int fd, unsigned long req, ...){
    va_list ap; va_start(ap, req); void *arg = va_arg(ap, void*); va_end(ap);
    unsigned nr = req & 0xff, type = (req >> 8) & 0xff;
    uint32_t cmd=0, psz=0, hcls=0; uint64_t pptr=0;
    if (type == 0x46 && arg) {
        if (nr == 0x2A) { cmd=*(uint32_t*)((char*)arg+8);  pptr=*(uint64_t*)((char*)arg+16); psz=*(uint32_t*)((char*)arg+24); }
        else if (nr == 0x2B) { hcls=*(uint32_t*)((char*)arg+12); pptr=*(uint64_t*)((char*)arg+16); psz=*(uint32_t*)((char*)arg+24); }
    }
    int r = real_ioctl(fd, req, arg);
    if (type == 0x46 && nr == 0x2A && pptr && r == 0) {
        uint32_t status = *(uint32_t*)((char*)arg+28);
        if (status == 0 && getenv("NVPATCH_GPUFLAGS") &&
            (cmd == 0x00000202u || cmd == 0x00000205u) && psz >= 8) {
            *(uint32_t*)((char*)(uintptr_t)pptr + 4) |= 1u; /* NV0000_CTRL_GPU_ID_INFO_IN_USE */
        }
        const char *classlist = getenv("NVCLASSLIST_HEX_FILE");
        if (status == 0 && classlist && cmd == 0x00800292u && psz) {
            patch_from_hex_file((void *)(uintptr_t)pptr, psz, classlist);
        }
    }
    if (type == 0x46 && lg) {
        if (nr == 0x2A) fprintf(lg, "CTRL  cmd=0x%08x psz=%-6u status=0x%-4x params=0x%llx",
                                cmd, psz, *(uint32_t*)((char*)arg+28), (unsigned long long)pptr);
        else if (nr == 0x2B) fprintf(lg, "ALLOC class=0x%08x psz=%-6u status=0x%-4x params=0x%llx",
                                hcls, psz, *(uint32_t*)((char*)arg+28), (unsigned long long)pptr);
        else { fprintf(lg, "IOCTL nr=0x%02x ret=%d\n", nr, r); fflush(lg); return r; }
        /* dump first N bytes of the params buffer (post-call) so guest vs host CONTENT can be
         * diffed: a control returning real data on the host but NV_OK+zeros on the guest shows up
         * as content=00.. vs nonzero. NVCONTENT env = N (default 48, 0=off). */
        const char *cd = getenv("NVCONTENT"); unsigned cn = cd ? (unsigned)atoi(cd) : 48;
        if (cn > 1024) cn = 1024;
        /* For ALLOC the kernel writes the CLASS param size back even when libcuda passes psz=0
         * (e.g. GR object 0xc7c0 -> 16B caps). Dump a fixed NVALLOC bytes from pAllocParms so the
         * forwarded GR-alloc reply can be compared host-vs-guest. NVALLOC env = N (default 32). */
        if (nr == 0x2B) {
            const char *ad = getenv("NVALLOC"); unsigned an = ad ? (unsigned)atoi(ad) : 32;
            if (an > 256) an = 256;
            if (an && pptr) {
                fprintf(lg, " areply=");
                for (unsigned i = 0; i < an; i++) fprintf(lg, "%02x", ((unsigned char *)(uintptr_t)pptr)[i]);
            }
            /* Also dump the OUTER NVOS64 struct (arg) post-call so the outer-struct layout /
             * copyout can be diffed host-vs-guest — the rbp-clobber is an alloc-reply ABI
             * mismatch, likely in the outer struct, not the params buffer. NVOUTER env = N (default 64). */
            const char *od = getenv("NVOUTER"); unsigned on = od ? (unsigned)atoi(od) : 64;
            if (on > 256) on = 256;
            if (on && arg) {
                fprintf(lg, " outer=");
                for (unsigned i = 0; i < on; i++) fprintf(lg, "%02x", ((unsigned char *)arg)[i]);
            }
        } else if (cn && pptr) {
            unsigned lim = (psz < cn) ? psz : cn;
            fprintf(lg, " content=");
            for (unsigned i = 0; i < lim; i++) fprintf(lg, "%02x", ((unsigned char *)(uintptr_t)pptr)[i]);
        }
        fprintf(lg, "\n"); fflush(lg);
    }
    return r;
}
