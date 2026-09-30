// efs_kernels.cu — device code for the b3 host-only EFS experiment, built to a cubin
// (nvcc -cubin -arch=sm_86) and loaded with cuModuleLoad by efs_fault.
//
// touch_seq: ONE thread walks n_pages pages, one access each, strictly in order, so every
// fault is a separate, individually timed event. stamps[2p] = %globaltimer just before the
// access to page p, stamps[2p+1] = %globaltimer after it completed (a store completes at the
// following __threadfence(); a load when its value is used). %globaltimer is the GPU's
// PTIMER, the clock the fault packet's TIMESTAMP field uses.

__device__ __forceinline__ unsigned long long gtimer(void)
{
    unsigned long long t;
    asm volatile("mov.u64 %0, %%globaltimer;" : "=l"(t));
    return t;
}

// op 0: store 0xC0DE0000|p into word 0 of page p.  op 1: load word 0 of page p and sum.
extern "C" __global__ void touch_seq(unsigned int *base, unsigned int n_pages, unsigned long long page_words,
                                     volatile unsigned long long *stamps, int op, unsigned int *out)
{
    if (threadIdx.x != 0 || blockIdx.x != 0)
        return;

    unsigned int acc = 0;
    for (unsigned int p = 0; p < n_pages; ++p) {
        volatile unsigned int *addr = base + (unsigned long long)p * page_words;
        stamps[2 * p] = gtimer();
        __threadfence_system();
        if (op == 0) {
            *addr = 0xC0DE0000u | p;
            __threadfence();
        }
        else {
            acc += *addr;
        }
        stamps[2 * p + 1] = gtimer();
        __threadfence_system();
    }
    out[0] = acc;
    out[1] = 0xD0D0D0D0u;
}

// Gather word 0 of the first n_chunks pages through the faulting VA, for raw-mode verification
// (the CUDA runtime cannot read those UVM-ioctl-mapped VAs from the host).
extern "C" __global__ void gather_word0(unsigned int *base, unsigned int n_chunks,
                                        unsigned long long page_words, unsigned int *out)
{
    if (threadIdx.x != 0 || blockIdx.x != 0)
        return;
    for (unsigned int c = 0; c < n_chunks; ++c)
        out[c] = base[(unsigned long long)c * page_words];
}

// Clock calibration: write %globaltimer to host memory continuously until the host sets
// stop[0]; the host samples it against CLOCK_REALTIME.
extern "C" __global__ void clock_spin(volatile unsigned long long *slot, volatile unsigned int *stop)
{
    if (threadIdx.x != 0 || blockIdx.x != 0)
        return;
    while (*stop == 0) {
        *slot = gtimer();
        __threadfence_system();
    }
}
