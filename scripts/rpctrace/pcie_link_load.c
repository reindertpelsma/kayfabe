/*
 * pcie_link_load.c — put enough traffic on the host GPU's PCIe link to make it
 * TRAIN UP, so that a value suspected of describing the LINK can be caught
 * changing on a single box.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * ★★★ WHY THIS EXISTS
 *
 * `rmladder --bus-info-sweep` (R22) asks a real GA106 for
 * `NV2080_CTRL_BUS_INFO_INDEX_PCIE_GEN_INFO` (`0x2d`) sixteen times and gets
 * sixteen identical words. ⊘ **That is not evidence the value is constant.**
 * The link on an idle GPU sits parked at 2.5 GT/s and stays there, so an idle
 * sampler measures a constant link, not a constant control. Sixteen identical
 * reads of a variable that is not varying say nothing at all.
 *
 * The decode says `0x2d` carries a `CURR_LEVEL` field equal to the *current*
 * negotiated generation. To turn that reading into a MEASUREMENT the link has
 * to move while somebody is watching — and the only lever an unprivileged
 * process has on link speed is traffic.
 *
 * ⇒ This program allocates a device buffer and pinned host memory and copies
 * between them in a loop for `NVLOAD_SECONDS` (default 20), which is what RM's
 * own power management watches when it decides to raise the link.
 *
 * ⊘ **No CUDA toolkit is required and none is installed on the bench**: the
 * entry points come out of `libcuda.so.1` by `dlsym`, the same way
 * `cuinit_probe.c` does it, and for the same reason — a dependency on a
 * toolkit would make the instrument un-runnable on the one machine that has
 * the GPU.
 *
 * ⚠ It prints `pcie.link.gen.current` from sysfs before and after so a run
 * that FAILED to move the link is distinguishable from one that moved it: a
 * loader that quietly did nothing and exited 0 would make a negative result
 * unreadable, which is the `boot_capture.sh` lesson in a smaller place.
 *
 *   cc -O2 -o pcie_link_load pcie_link_load.c -ldl
 *   NVLOAD_SECONDS=20 ./pcie_link_load
 *
 * Exit codes: 0 = the loop ran; 2 = libcuda unusable; 3 = a CUDA call failed.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef int CUresult_t;
typedef int CUdevice_t;
typedef void *CUcontext_t;
typedef unsigned long long CUdeviceptr_t;

static void *L;
#define SYM(name) dlsym(L, name)

static double now_s(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

/* The PCI address is a parameter, not a constant: the bench's GPU is at
 * 0000:00:07.0 but that is a fact about one rented box. */
static void report_link(const char *when, const char *bdf) {
    char path[256];
    char buf[64] = "";
    FILE *f;
    snprintf(path, sizeof path, "/sys/bus/pci/devices/%s/current_link_speed", bdf);
    f = fopen(path, "r");
    if (f) {
        if (!fgets(buf, sizeof buf, f)) buf[0] = 0;
        fclose(f);
    }
    buf[strcspn(buf, "\n")] = 0;
    printf("[link %-6s] %s current_link_speed = %s\n", when, bdf, buf[0] ? buf : "(unreadable)");
    fflush(stdout);
}

int main(void) {
    const char *bdf = getenv("NVLOAD_BDF");
    const char *secs_s = getenv("NVLOAD_SECONDS");
    double secs = secs_s ? atof(secs_s) : 20.0;
    size_t bytes = 256u << 20; /* 256 MiB — big enough that the copy, not the
                                * launch overhead, is what occupies the link. */
    if (!bdf) bdf = "0000:00:07.0";

    L = dlopen("libcuda.so.1", RTLD_NOW);
    if (!L) {
        fprintf(stderr, "★ dlopen(libcuda.so.1): %s\n", dlerror());
        return 2;
    }
    CUresult_t (*cuInit)(unsigned) = SYM("cuInit");
    CUresult_t (*cuDeviceGet)(CUdevice_t *, int) = SYM("cuDeviceGet");
    CUresult_t (*cuCtxCreate)(CUcontext_t *, unsigned, CUdevice_t) = SYM("cuCtxCreate_v2");
    CUresult_t (*cuCtxDestroy)(CUcontext_t) = SYM("cuCtxDestroy_v2");
    CUresult_t (*cuMemAlloc)(CUdeviceptr_t *, size_t) = SYM("cuMemAlloc_v2");
    CUresult_t (*cuMemFree)(CUdeviceptr_t) = SYM("cuMemFree_v2");
    CUresult_t (*cuMemAllocHost)(void **, size_t) = SYM("cuMemAllocHost_v2");
    CUresult_t (*cuMemcpyHtoD)(CUdeviceptr_t, const void *, size_t) = SYM("cuMemcpyHtoD_v2");
    CUresult_t (*cuMemcpyDtoH)(void *, CUdeviceptr_t, size_t) = SYM("cuMemcpyDtoH_v2");
    CUresult_t (*cuCtxSynchronize)(void) = SYM("cuCtxSynchronize");
    if (!cuInit || !cuDeviceGet || !cuCtxCreate || !cuMemAlloc || !cuMemAllocHost ||
        !cuMemcpyHtoD || !cuMemcpyDtoH || !cuCtxSynchronize) {
        fprintf(stderr, "★ libcuda is missing an entry point this loader needs\n");
        return 2;
    }

    report_link("before", bdf);

    CUdevice_t dev = 0;
    CUcontext_t ctx = NULL;
    CUdeviceptr_t dptr = 0;
    void *hptr = NULL;
    int rc;
#define CHECK(call)                                                                      \
    do {                                                                                 \
        rc = (call);                                                                     \
        if (rc != 0) {                                                                   \
            fprintf(stderr, "★ %s -> %d\n", #call, rc);                                   \
            return 3;                                                                    \
        }                                                                                \
    } while (0)
    CHECK(cuInit(0));
    CHECK(cuDeviceGet(&dev, 0));
    CHECK(cuCtxCreate(&ctx, 0, dev));
    CHECK(cuMemAlloc(&dptr, bytes));
    CHECK(cuMemAllocHost(&hptr, bytes));
    memset(hptr, 0xA5, bytes);

    double t0 = now_s();
    unsigned long long moved = 0;
    unsigned long iters = 0;
    while (now_s() - t0 < secs) {
        CHECK(cuMemcpyHtoD(dptr, hptr, bytes));
        CHECK(cuMemcpyDtoH(hptr, dptr, bytes));
        CHECK(cuCtxSynchronize());
        moved += 2ull * bytes;
        if (++iters % 4 == 0) report_link("during", bdf);
    }
    double dt = now_s() - t0;
    printf("[load] %lu iterations, %.2f GiB moved in %.1f s (%.2f GiB/s)\n", iters,
           (double)moved / (1024.0 * 1024.0 * 1024.0), dt,
           (double)moved / (1024.0 * 1024.0 * 1024.0) / dt);
    report_link("after", bdf);
    fflush(stdout);

    if (cuMemFree) cuMemFree(dptr);
    if (cuCtxDestroy) cuCtxDestroy(ctx);
    return 0;
}
