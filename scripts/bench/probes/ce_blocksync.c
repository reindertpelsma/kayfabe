/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * ce_blocksync.c — the Linux oracle for "a completion the guest waits for by INTERRUPT"
 * (2026-10-08, Windows TDR investigation; coordinator: iterate until Linux passes, then Windows).
 *
 * A CUDA context in BLOCKING-SYNC mode (CU_CTX_SCHED_BLOCKING_SYNC) and an event created with
 * CU_EVENT_BLOCKING_SYNC: cuEventSynchronize sleeps until the RM wakes it from a NON-STALL interrupt
 * (no spin). If the interrupt never reaches the waiter, libcuda wakes itself after a ~1 s slice
 * (and asks the RM to service interrupts, V3_REFUSAL_AUDIT.md row MC_SERVICE_INTERRUPTS), so a lost
 * interrupt shows as LATENCY, not as a failure: every wait of a 4 KiB copy then takes ~1 s instead
 * of tens of microseconds. This program therefore reports per-wait latency for tiny work on:
 *   ce_h2d  — cuMemcpyHtoDAsync from pinned memory on its own stream (a copy engine)
 *   ce_d2h  — cuMemcpyDtoHAsync into pinned memory (a copy engine), data checked
 *   ce_d2d  — cuMemcpyDtoDAsync (a copy engine on current drivers)
 *   gr_noop — an empty kernel (graphics/compute engine), the control
 * and prints `CEBS <phase> n=<n> med_us=<> p90_us=<> max_us=<> slow=<waits over 200 ms> ok|FAIL`,
 * then `CEBS_DONE ok|FAIL`.
 *
 * FALSIFIER (stated before the first run): the interrupt path is broken for a phase iff its median
 * wait is >= 200 ms (slice-bound) while bare metal's is < 10 ms. A phase whose median is < 10 ms but
 * with some slow waits is reported, not failed (coalescing or scheduling noise).
 *
 * Build: gcc -O2 -o ce_blocksync ce_blocksync.c -ldl     (no CUDA toolkit needed: dlopen libcuda)
 * Use:   ./ce_blocksync [iterations, default 50]
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
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

static const char PTX[] =
	".version 6.0\n.target sm_50\n.address_size 64\n"
	".visible .entry noop()\n{\n\tret;\n}\n";

static double now_us(void)
{
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	return t.tv_sec * 1e6 + t.tv_nsec / 1e3;
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

static int report(const char *phase, double *dt, int n, int *all_ok)
{
	int slow = 0;
	for (int i = 0; i < n; i++)
		slow += dt[i] >= SLOW_US;
	qsort(dt, n, sizeof *dt, cmp);
	double med = dt[n / 2], p90 = dt[(n * 9) / 10], max = dt[n - 1];
	int ok = med < SLOW_US;
	*all_ok &= ok;
	printf("CEBS %s n=%d med_us=%.1f p90_us=%.1f max_us=%.1f slow=%d %s\n", phase, n, med, p90,
	       max, slow, ok ? "ok" : "FAIL");
	fflush(stdout);
	return 0;
}

int main(int argc, char **argv)
{
	int n = argc > 1 ? atoi(argv[1]) : 50;
	if (n < 2 || n > 100000)
		n = 50;
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
	CUstream st;
	CUevent ev;
	CUdeviceptr a, b;
	unsigned char *hp;
	const size_t SZ = 4096;
	CHK(cuInit(0));
	CHK(cuDeviceGet(&dev, 0));
	CHK(cuCtxCreate(&ctx, CU_CTX_SCHED_BLOCKING_SYNC, dev));
	CHK(cuStreamCreate(&st, CU_STREAM_NON_BLOCKING));
	CHK(cuEventCreate(&ev, CU_EVENT_BLOCKING_SYNC | CU_EVENT_DISABLE_TIMING));
	CHK(cuMemAlloc(&a, SZ));
	CHK(cuMemAlloc(&b, SZ));
	CHK(cuMemAllocHost((void **)&hp, 2 * SZ));
	CUmodule mod;
	CUfunction noop;
	CHK(cuModuleLoadData(&mod, PTX));
	CHK(cuModuleGetFunction(&noop, mod, "noop"));

	double *dt = calloc(n, sizeof *dt);
	if (!dt)
		return 1;
	int all_ok = 1, data_ok = 1;
	/* Warm-up: one of each, so first-use costs are not measured. */
	CHK(cuMemcpyHtoDAsync(a, hp, SZ, st));
	CHK(cuMemcpyDtoDAsync(b, a, SZ, st));
	CHK(cuMemcpyDtoHAsync(hp + SZ, b, SZ, st));
	CHK(cuLaunchKernel(noop, 1, 1, 1, 1, 1, 1, 0, st, NULL, NULL));
	CHK(cuEventRecord(ev, st));
	CHK(cuEventSynchronize(ev));

	for (int i = 0; i < n; i++) {
		memset(hp, (i * 7 + 1) & 0xff, SZ);
		double t0 = now_us();
		CHK(cuMemcpyHtoDAsync(a, hp, SZ, st));
		CHK(cuEventRecord(ev, st));
		CHK(cuEventSynchronize(ev));
		dt[i] = now_us() - t0;
	}
	report("ce_h2d", dt, n, &all_ok);
	for (int i = 0; i < n; i++) {
		double t0 = now_us();
		CHK(cuMemcpyDtoDAsync(b, a, SZ, st));
		CHK(cuEventRecord(ev, st));
		CHK(cuEventSynchronize(ev));
		dt[i] = now_us() - t0;
	}
	report("ce_d2d", dt, n, &all_ok);
	for (int i = 0; i < n; i++) {
		memset(hp + SZ, 0, SZ);
		double t0 = now_us();
		CHK(cuMemcpyDtoHAsync(hp + SZ, b, SZ, st));
		CHK(cuEventRecord(ev, st));
		CHK(cuEventSynchronize(ev));
		dt[i] = now_us() - t0;
		/* b holds the last ce_h2d pattern: ((n-1)*7+1) & 0xff. A wait that returned before the
		 * copy landed shows here (the buffer was zeroed first). */
		if (hp[SZ] != (((n - 1) * 7 + 1) & 0xff) || hp[2 * SZ - 1] != hp[SZ])
			data_ok = 0;
	}
	report("ce_d2h", dt, n, &all_ok);
	for (int i = 0; i < n; i++) {
		double t0 = now_us();
		CHK(cuLaunchKernel(noop, 1, 1, 1, 1, 1, 1, 0, st, NULL, NULL));
		CHK(cuEventRecord(ev, st));
		CHK(cuEventSynchronize(ev));
		dt[i] = now_us() - t0;
	}
	report("gr_noop", dt, n, &all_ok);
	printf("CEBS data %s\n", data_ok ? "ok" : "FAIL (a wait returned before its copy landed)");
	printf("CEBS_DONE %s\n", all_ok && data_ok ? "ok" : "FAIL");
	return !(all_ok && data_ok);
}
