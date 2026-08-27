// rtp.c — minimal CUDA RUNTIME (cudart) adapter probe, the cudart counterpart of
// reload_probe.c (dvp, driver-API). Links libcudart (CUDA 12, the llama.cpp stack),
// NOT libcuda directly. cudaGetDeviceCount triggers the cudart lazy full init that
// llama.cpp's ggml-cuda backend performs; on Mode-2 it fails fast with
// cudaErrorInitializationError(3) while the driver API (dvp) succeeds on the SAME
// pinned adapter (see memory mode2_execfwd_layer2 CORRECTION 3). This isolates the
// cudart-only blocker: an RM control / UVM-init reply whose in-payload NvStatus
// libcudart rejects (silent — no errno, no dmesg). No cudart headers needed; the few
// symbols are declared by hand so it builds against just libcudart.so.12.
#include <stdio.h>
typedef int cudaError_t;
extern cudaError_t cudaGetDeviceCount(int *);
extern cudaError_t cudaDriverGetVersion(int *);
extern cudaError_t cudaRuntimeGetVersion(int *);
extern const char *cudaGetErrorString(cudaError_t);
extern const char *cudaGetErrorName(cudaError_t);
int main(void)
{
    int drv = 0, rt = 0;
    cudaDriverGetVersion(&drv);
    cudaRuntimeGetVersion(&rt);
    printf("RTP versions: cudaDriverGetVersion=%d cudaRuntimeGetVersion=%d\n", drv, rt);
    fflush(stdout);
    int n = -1;
    cudaError_t e = cudaGetDeviceCount(&n);
    printf("RTP cudaGetDeviceCount rc=%d name=%s str=%s n=%d\n",
           (int)e, cudaGetErrorName(e), cudaGetErrorString(e), n);
    fflush(stdout);
    return (e == 0 && n > 0) ? 0 : 1;
}
