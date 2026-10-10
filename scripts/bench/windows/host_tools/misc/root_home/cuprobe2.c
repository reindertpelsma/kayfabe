#include <stdio.h>
#include <dlfcn.h>
/* Sequenced properly: the earlier version read `n` inside the same printf that
 * called cuDeviceGetCount(&n). Argument evaluation order is UNSPECIFIED in C,
 * so `n` could be (and was) read before the call wrote it -- reporting 0
 * devices no matter what CUDA actually said. */
int main(void) {
    void *H = dlopen("libcuda.so.1", RTLD_NOW);
    if (!H) { printf("libcuda dlopen FAILED: %s\n", dlerror()); return 2; }
    int (*cuInit)(unsigned) = dlsym(H, "cuInit");
    int (*cuDriverGetVersion)(int *) = dlsym(H, "cuDriverGetVersion");
    int (*cuDeviceGetCount)(int *) = dlsym(H, "cuDeviceGetCount");
    int (*cuDeviceGetName)(char *, int, int) = dlsym(H, "cuDeviceGetName");
    int (*cuDeviceGet)(int *, int) = dlsym(H, "cuDeviceGet");
    int rc = cuInit(0);
    printf("cuInit rc = %d\n", rc);
    if (rc) return 1;
    int v = 0; if (cuDriverGetVersion) { cuDriverGetVersion(&v); printf("driver API = %d\n", v); }
    int n = -1;
    int rc2 = cuDeviceGetCount(&n);          /* call FIRST */
    printf("cuDeviceGetCount rc = %d\n", rc2);   /* then read */
    printf("devices = %d\n", n);
    if (rc2 == 0 && n > 0 && cuDeviceGet && cuDeviceGetName) {
        int dev = 0; char nm[256] = {0};
        if (cuDeviceGet(&dev, 0) == 0 && cuDeviceGetName(nm, sizeof nm, dev) == 0)
            printf("device 0 = %s\n", nm);
    }
    return 0;
}
