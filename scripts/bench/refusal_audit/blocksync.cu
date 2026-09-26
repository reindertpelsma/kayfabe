// blocksync.cu — the class-A probe of the refusal audit (docs/design/V3_REFUSAL_AUDIT.md).
//
// A CUDA waiter in BLOCKING-SYNC mode sleeps on an interrupt instead of spinning; with no
// interrupt for ~1 s the driver issues NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS, and a non-OK answer
// there ended the wait early (v3-mapfix 56032c46: clFinish returned at +1.000 s with the batch
// still running). This program makes every such wait LONGER than the slice and checks that the
// wait did not return before the work did: the kernel writes a value only at its end, and the
// elapsed time must cover the kernel's own measured duration.
//
// Three waits, each on a ~2.5 s single-thread spin kernel: cudaDeviceSynchronize (blocking-sync
// device flag), cudaEventSynchronize on a cudaEventBlockingSync event, cudaStreamSynchronize.
// Prints `BLOCKSYNC <how> dt=<s> kernel=<s> val=<v> ok|FAIL`, then `BLOCKSYNC_DONE ok|FAIL`.
#include <cstdio>
#include <chrono>
#include <cuda_runtime.h>

// ⊘ Timed by %globaltimer (GPU wall-clock ns), not clock64: the SM clock the guest is TOLD
// (cudaDevAttrClockRate) need not be the clock the host GPU runs, and a cycle-count spin would
// then run shorter than intended and fail an honest wait.
__device__ __forceinline__ unsigned long long gtimer() {
    unsigned long long t;
    asm volatile("mov.u64 %0, %%globaltimer;" : "=l"(t));
    return t;
}

__global__ void spin(unsigned long long ns, int *out, unsigned long long *took) {
    unsigned long long t0 = gtimer();
    while (gtimer() - t0 < ns) { }
    *took = gtimer() - t0;
    *out = 42;
}

static double now() {
    return std::chrono::duration<double>(std::chrono::steady_clock::now().time_since_epoch()).count();
}

int main() {
    if (cudaSetDeviceFlags(cudaDeviceScheduleBlockingSync) != cudaSuccess) { printf("BLOCKSYNC_DONE FAIL setflags\n"); return 1; }
    int *d = nullptr; unsigned long long *dt_d = nullptr;
    if (cudaMalloc(&d, sizeof(int)) != cudaSuccess || cudaMalloc(&dt_d, sizeof(unsigned long long)) != cudaSuccess) {
        printf("BLOCKSYNC_DONE FAIL malloc\n"); return 1;
    }
    const double secs = 2.5;
    const unsigned long long ns = (unsigned long long)(secs * 1e9);
    cudaEvent_t ev; cudaEventCreateWithFlags(&ev, cudaEventBlockingSync | cudaEventDisableTiming);
    cudaStream_t st; cudaStreamCreate(&st);
    bool all = true;
    for (int how = 0; how < 3; how++) {
        cudaMemset(d, 0, sizeof(int)); cudaMemset(dt_d, 0, sizeof(unsigned long long)); cudaDeviceSynchronize();
        double t0 = now();
        cudaError_t e;
        const char *name;
        if (how == 0) { spin<<<1, 1>>>(ns, d, dt_d); e = cudaDeviceSynchronize(); name = "device_sync"; }
        else if (how == 1) { spin<<<1, 1>>>(ns, d, dt_d); cudaEventRecord(ev, 0); e = cudaEventSynchronize(ev); name = "event_sync"; }
        else { spin<<<1, 1, 0, st>>>(ns, d, dt_d); e = cudaStreamSynchronize(st); name = "stream_sync"; }
        double dt = now() - t0;
        // ⊘ Read the result WITHOUT another sync first: a wait that returned early must be seen
        // as early. cudaMemcpy on the legacy stream would itself wait for the kernel.
        int val = -1; unsigned long long took = 0;
        cudaMemcpyAsync(&val, d, sizeof(int), cudaMemcpyDeviceToHost, st);
        cudaStreamSynchronize(st);
        cudaMemcpy(&took, dt_d, sizeof(unsigned long long), cudaMemcpyDeviceToHost);
        double ksecs = (double)took / 1e9;
        // the wait must cover the kernel: it cannot have returned before the GPU's own 2.5 s
        bool ok = e == cudaSuccess && val == 42 && dt >= 0.95 * secs && ksecs >= secs;
        all = all && ok;
        printf("BLOCKSYNC %s dt=%.3f kernel>=%.3f err=%d val=%d %s\n", name, dt, ksecs, (int)e, val, ok ? "ok" : "FAIL");
        fflush(stdout);
    }
    printf("BLOCKSYNC_DONE %s\n", all ? "ok" : "FAIL");
    return all ? 0 : 1;
}
