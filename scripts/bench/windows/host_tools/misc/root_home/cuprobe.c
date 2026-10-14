#include <stdio.h>
#include <dlfcn.h>
int main(void) {
    void *H = dlopen("libcuda.so.1", RTLD_NOW);
    if (!H) { printf("libcuda dlopen FAILED: %s\n", dlerror()); return 2; }
    int (*cuInit)(unsigned) = dlsym(H, "cuInit");
    int (*cuDriverGetVersion)(int *) = dlsym(H, "cuDriverGetVersion");
    int (*cuDeviceGetCount)(int *) = dlsym(H, "cuDeviceGetCount");
    if (!cuInit) { printf("no cuInit symbol\n"); return 2; }
    int rc = cuInit(0);
    printf("cuInit rc = %d   (0=OK, 999=CUDA_ERROR_UNKNOWN)\n", rc);
    if (rc == 0) {
        int v = 0, n = 0;
        if (cuDriverGetVersion) { cuDriverGetVersion(&v); printf("driver API version = %d\n", v); }
        if (cuDeviceGetCount) { printf("cuDeviceGetCount rc = %d  devices = %d\n", cuDeviceGetCount(&n), n); }
    }
    return rc == 0 ? 0 : 1;
}
