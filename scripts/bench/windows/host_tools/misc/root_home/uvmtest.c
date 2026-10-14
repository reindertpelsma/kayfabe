/*
 * Exercise UVM managed memory through the CUDA driver API.
 *
 * This is the path the 32->40 byte UVM_REGISTER_GPU change sits on: creating a
 * context registers the GPU with UVM, and at 32 bytes the guest read rmCtrlFd
 * (an input fd) as rmStatus and dropped hClient/hSmcPartRef entirely.  If the
 * new layout were wrong, this is where it shows -- not in a unit test.
 *
 * Driver API only, declared by hand: the guest has libcuda but no cuda.h.
 */
#include <stdio.h>
#include <string.h>
#include <dlfcn.h>

typedef int CUresult;
typedef int CUdevice;
typedef void *CUcontext;
typedef unsigned long long CUdeviceptr;

int main(void)
{
    void *h = dlopen("libcuda.so.1", RTLD_NOW);
    if (!h) { printf("FAIL dlopen: %s\n", dlerror()); return 1; }

    CUresult (*cuInit)(unsigned) = dlsym(h, "cuInit");
    CUresult (*cuDeviceGetCount)(int *) = dlsym(h, "cuDeviceGetCount");
    CUresult (*cuDeviceGet)(CUdevice *, int) = dlsym(h, "cuDeviceGet");
    CUresult (*cuDeviceGetName)(char *, int, CUdevice) = dlsym(h, "cuDeviceGetName");
    CUresult (*cuCtxCreate)(CUcontext *, unsigned, CUdevice) = dlsym(h, "cuCtxCreate_v2");
    CUresult (*cuMemAllocManaged)(CUdeviceptr *, size_t, unsigned) = dlsym(h, "cuMemAllocManaged");
    CUresult (*cuMemFree)(CUdeviceptr) = dlsym(h, "cuMemFree_v2");
    CUresult (*cuCtxSynchronize)(void) = dlsym(h, "cuCtxSynchronize");
    CUresult (*cuCtxDestroy)(CUcontext) = dlsym(h, "cuCtxDestroy_v2");

    if (!cuInit || !cuMemAllocManaged) { printf("FAIL dlsym\n"); return 1; }

    CUresult r = cuInit(0);
    if (r) { printf("FAIL cuInit rc=%d\n", r); return 1; }
    printf("ok   cuInit\n");

    int n = 0;
    if (cuDeviceGetCount(&n) || n < 1) { printf("FAIL no devices (n=%d)\n", n); return 1; }
    CUdevice dev; char name[128] = {0};
    cuDeviceGet(&dev, 0); cuDeviceGetName(name, sizeof name, dev);
    printf("ok   device 0: %s\n", name);

    CUcontext ctx = NULL;
    r = cuCtxCreate(&ctx, 0, dev);
    if (r) { printf("FAIL cuCtxCreate rc=%d  <-- UVM registration happens here\n", r); return 1; }
    printf("ok   cuCtxCreate (the GPU is now registered with UVM)\n");

    /* Managed memory: allocated once, touched by the CPU, and only valid if the
     * UVM registration above actually took. */
    CUdeviceptr p = 0;
    const size_t SZ = 4u << 20;              /* 4 MiB */
    r = cuMemAllocManaged(&p, SZ, 0x01 /* CU_MEM_ATTACH_GLOBAL */);
    if (r) { printf("FAIL cuMemAllocManaged rc=%d\n", r); return 1; }
    printf("ok   cuMemAllocManaged %zu bytes at 0x%llx\n", SZ, (unsigned long long)p);

    unsigned char *cpu = (unsigned char *)p;
    memset(cpu, 0xA5, SZ);
    if (cuCtxSynchronize()) { printf("FAIL cuCtxSynchronize\n"); return 1; }
    for (size_t i = 0; i < SZ; i += 4093) {
        if (cpu[i] != 0xA5) { printf("FAIL readback at %zu: 0x%02x\n", i, cpu[i]); return 1; }
    }
    printf("ok   4 MiB written and verified through the managed mapping\n");

    cuMemFree(p);
    cuCtxDestroy(ctx);
    printf("PASS uvm managed memory works over the 40-byte REGISTER_GPU layout\n");
    return 0;
}
