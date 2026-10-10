/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * ce_blocksync.c — the Linux oracle for "a completion the guest waits for by INTERRUPT"
 * (2026-10-08, Windows TDR investigation; coordinator: iterate until Linux passes, then Windows).
 *
 * A CUDA context in BLOCKING-SYNC mode (CU_CTX_SCHED_BLOCKING_SYNC) and an event created with
 * CU_EVENT_BLOCKING_SYNC: cuEventSynchronize sleeps until the RM wakes it from a NON-STALL interrupt
 * (no spin). If the interrupt never reaches the waiter, libcuda wakes itself after a ~1 s slice
 * (and asks the RM to service interrupts, V3_REFUSAL_AUDIT.md row MC_SERVICE_INTERRUPTS), so a lost
 * interrupt shows as LATENCY, not as a failure.
 *
 * ⊘ Tiny work (a 4 KiB copy) is finished before libcuda decides to sleep — measured on bare metal
 * 2026-10-08: ~10 us waits — so it never exercises the interrupt. The phases that matter are LONG:
 *   ce_h2d_big  — cuMemcpyHtoDAsync of 64 MiB from pinned memory on its own stream (a copy engine)
 *   ce_d2d_big  — cuMemcpyDtoDAsync of 64 MiB (a copy engine on current drivers)
 *   ce_d2h_big  — cuMemcpyDtoHAsync of 64 MiB into pinned memory, data checked
 *   gr_spin     — a %globaltimer spin kernel of 5 ms (graphics/compute engine), the control
 * plus the tiny ones (ce_h2d, gr_noop) for reference. Each line reports the wall time per wait and
 * the CPU time the process burnt per wait (`cpu_pct`): a blocking wait sleeps, so cpu_pct well
 * below 100 shows the wait really slept on an interrupt (a spin would be ~100).
 *   `CEBS <phase> n=<n> med_us=<> p90_us=<> max_us=<> slow=<waits over 200 ms> cpu_pct=<> ok|FAIL`
 * then `CEBS data ok|FAIL` and `CEBS_DONE ok|FAIL`.
 *
 * FALSIFIER (stated before the first guest run): the interrupt path is broken for a phase iff its
 * median wait is >= 200 ms (slice-bound) while bare metal's is the work's own duration (a few ms).
 *
 * Build: gcc -O2 -o ce_blocksync ce_blocksync.c -ldl     (no CUDA toolkit needed: dlopen libcuda)
 * Use:   ./ce_blocksync [iterations, default 30]
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>

typedef int CUresult;
typedef int CUdevice;
typedef void *CUcontext;
typedef void *CUmodule;
typedef void *CUfunction;
typedef void *CUstream;
typedef void *CUevent;
typedef unsigned long long CUdeviceptr;

static CUresult (*cuInit)(unsigned);
static CUresult (*cuDeviceGet)(CUdevice *, int);
static CUresult (*cuCtxCreate)(CUcontext *, unsigned, CUdevice);
static CUresult (*cuMemAlloc)(CUdeviceptr *, size_t);
static CUresult (*cuMemAllocHost)(void **, size_t);
static CUresult (*cuMemcpyHtoDAsync)(CUdeviceptr, const void *, size_t, CUstream);
static CUresult (*cuMemcpyDtoHAsync)(void *, CUdeviceptr, size_t, CUstream);
static CUresult (*cuMemcpyDtoDAsync)(CUdeviceptr, CUdeviceptr, size_t, CUstream);
static CUresult (*cuStreamCreate)(CUstream *, unsigned);
static CUresult (*cuEventCreate)(CUevent *, unsigned);
static CUresult (*cuEventRecord)(CUevent, CUstream);
static CUresult (*cuEventSynchronize)(CUevent);
static CUresult (*cuModuleLoadData)(CUmodule *, const void *);
static CUresult (*cuModuleGetFunction)(CUfunction *, CUmodule, const char *);
static CUresult (*cuLaunchKernel)(CUfunction, unsigned, unsigned, unsigned, unsigned, unsigned,
				  unsigned, unsigned, CUstream, void **, void **);

#define CU_CTX_SCHED_BLOCKING_SYNC 0x4
#define CU_EVENT_BLOCKING_SYNC 0x1
#define CU_EVENT_DISABLE_TIMING 0x2
#define CU_STREAM_NON_BLOCKING 0x1
#define SLOW_US 200000.0
#define SMALL ((size_t)4096)
#define BIG ((size_t)64 << 20)
#define SPIN_NS 5000000ULL

/* noop(): returns. spin(ns): spins on %globaltimer (GPU wall-clock ns) for ns. */
static const char PTX[] =
	".version 6.0\n.target sm_50\n.address_size 64\n"
	".visible .entry noop()\n{\n\tret;\n}\n"
	".visible .entry spin(.param .u64 ns)\n{\n"
	"\t.reg .u64 %t0, %t1, %d, %n;\n\t.reg .pred %p;\n"
	"\tld.param.u64 %n, [ns];\n\tmov.u64 %t0, %globaltimer;\n"
	"L:\n\tmov.u64 %t1, %globaltimer;\n\tsub.u64 %d, %t1, %t0;\n"
	"\tsetp.lt.u64 %p, %d, %n;\n\t@%p bra L;\n\tret;\n}\n";

static double now_us(void)
{
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	return t.tv_sec * 1e6 + t.tv_nsec / 1e3;
}

static double cpu_us(void)
{
	struct rusage r;
	getrusage(RUSAGE_SELF, &r);
	return (r.ru_utime.tv_sec + r.ru_stime.tv_sec) * 1e6 + r.ru_utime.tv_usec +
	       r.ru_stime.tv_usec;
}

static int cmp(const void *a, const void *b)
{
	double x = *(const double *)a, y = *(const double *)b;
	return (x > y) - (x < y);
}

#define SYM(n, s)                                                                    \
	do {                                                                         \
		*(void **)(&n) = dlsym(h, s);                                        \
		if (!n) {                                                            \
			printf("CEBS_DONE FAIL no symbol %s\n", s);                  \
			return 1;                                                    \
		}                                                                    \
	} while (0)
#define CHK(x)                                                                       \
	do {                                                                         \
		CUresult r_ = (x);                                                   \
		if (r_) {                                                            \
			printf("CEBS_DONE FAIL %s = %d (line %d)\n", #x, r_, __LINE__); \
			return 1;                                                    \
		}                                                                    \
	} while (0)

static void report(const char *phase, double *dt, int n, double wall, double cpu, int *all_ok)
{
	int slow = 0;
	for (int i = 0; i < n; i++)
		slow += dt[i] >= SLOW_US;
	qsort(dt, n, sizeof *dt, cmp);
	double med = dt[n / 2], p90 = dt[(n * 9) / 10], max = dt[n - 1];
	int ok = med < SLOW_US;
	*all_ok &= ok;
	printf("CEBS %s n=%d med_us=%.1f p90_us=%.1f max_us=%.1f slow=%d cpu_pct=%.0f %s\n", phase,
	       n, med, p90, max, slow, wall > 0 ? 100.0 * cpu / wall : 0.0, ok ? "ok" : "FAIL");
	fflush(stdout);
}

enum op { H2D, D2D, D2H, NOOP, SPIN };

static CUstream st;
static CUevent ev;
static CUdeviceptr a, b;
static unsigned char *hp;
static CUfunction noop, spin;

static CUresult issue(enum op o, size_t sz)
{
	unsigned long long ns = SPIN_NS;
	void *args[] = { &ns };
	switch (o) {
	case H2D:
		return cuMemcpyHtoDAsync(a, hp, sz, st);
	case D2D:
		return cuMemcpyDtoDAsync(b, a, sz, st);
	case D2H:
		return cuMemcpyDtoHAsync(hp + BIG, b, sz, st);
	case NOOP:
		return cuLaunchKernel(noop, 1, 1, 1, 1, 1, 1, 0, st, NULL, NULL);
	case SPIN:
		return cuLaunchKernel(spin, 1, 1, 1, 1, 1, 1, 0, st, args, NULL);
	}
	return 1;
}

static int phase(const char *name, enum op o, size_t sz, int n, double *dt, int *all_ok)
{
	double w0 = now_us(), c0 = cpu_us();
	for (int i = 0; i < n; i++) {
		double t0 = now_us();
		CHK(issue(o, sz));
		CHK(cuEventRecord(ev, st));
		CHK(cuEventSynchronize(ev));
		dt[i] = now_us() - t0;
	}
	report(name, dt, n, now_us() - w0, cpu_us() - c0, all_ok);
	return 0;
}

int main(int argc, char **argv)
{
	int n = argc > 1 ? atoi(argv[1]) : 30;
	if (n < 2 || n > 100000)
		n = 30;
	void *h = dlopen("libcuda.so.1", RTLD_NOW);
	if (!h) {
		printf("CEBS_DONE FAIL dlopen libcuda.so.1: %s\n", dlerror());
		return 1;
	}
	SYM(cuInit, "cuInit");
	SYM(cuDeviceGet, "cuDeviceGet");
	SYM(cuCtxCreate, "cuCtxCreate_v2");
	SYM(cuMemAlloc, "cuMemAlloc_v2");
	SYM(cuMemAllocHost, "cuMemAllocHost_v2");
	SYM(cuMemcpyHtoDAsync, "cuMemcpyHtoDAsync_v2");
	SYM(cuMemcpyDtoHAsync, "cuMemcpyDtoHAsync_v2");
	SYM(cuMemcpyDtoDAsync, "cuMemcpyDtoDAsync_v2");
	SYM(cuStreamCreate, "cuStreamCreate");
	SYM(cuEventCreate, "cuEventCreate");
	SYM(cuEventRecord, "cuEventRecord");
	SYM(cuEventSynchronize, "cuEventSynchronize");
	SYM(cuModuleLoadData, "cuModuleLoadData");
	SYM(cuModuleGetFunction, "cuModuleGetFunction");
	SYM(cuLaunchKernel, "cuLaunchKernel");

	CUdevice dev;
	CUcontext ctx;
	CUmodule mod;
	CHK(cuInit(0));
	CHK(cuDeviceGet(&dev, 0));
	CHK(cuCtxCreate(&ctx, CU_CTX_SCHED_BLOCKING_SYNC, dev));
	CHK(cuStreamCreate(&st, CU_STREAM_NON_BLOCKING));
	CHK(cuEventCreate(&ev, CU_EVENT_BLOCKING_SYNC | CU_EVENT_DISABLE_TIMING));
	CHK(cuMemAlloc(&a, BIG));
	CHK(cuMemAlloc(&b, BIG));
	CHK(cuMemAllocHost((void **)&hp, 2 * BIG));
	CHK(cuModuleLoadData(&mod, PTX));
	CHK(cuModuleGetFunction(&noop, mod, "noop"));
	CHK(cuModuleGetFunction(&spin, mod, "spin"));

	double *dt = calloc(n, sizeof *dt);
	if (!dt)
		return 1;
	int all_ok = 1;
	/* Warm-up: one of each, so first-use costs are not measured. */
	for (int o = H2D; o <= SPIN; o++)
		CHK(issue((enum op)o, SMALL));
	CHK(cuEventRecord(ev, st));
	CHK(cuEventSynchronize(ev));

	memset(hp, 0x5a, BIG);
	if (phase("ce_h2d", H2D, SMALL, n, dt, &all_ok) || phase("gr_noop", NOOP, 0, n, dt, &all_ok) ||
	    phase("ce_h2d_big", H2D, BIG, n, dt, &all_ok) ||
	    phase("ce_d2d_big", D2D, BIG, n, dt, &all_ok))
		return 1;
	memset(hp + BIG, 0, BIG);
	if (phase("ce_d2h_big", D2H, BIG, n, dt, &all_ok) ||
	    phase("gr_spin", SPIN, 0, n, dt, &all_ok))
		return 1;
	/* b holds the 0x5a pattern; a wait that returned before its copy landed would leave zeros. */
	int data_ok = hp[BIG] == 0x5a && hp[2 * BIG - 1] == 0x5a;
	printf("CEBS data %s\n", data_ok ? "ok" : "FAIL (a wait returned before its copy landed)");
	printf("CEBS_DONE %s\n", all_ok && data_ok ? "ok" : "FAIL");
	return !(all_ok && data_ok);
}
