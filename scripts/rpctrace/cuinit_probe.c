/*
 * cuinit_probe.c — drive libcuda far enough to make it ask the questions
 * `nvidia-smi` never asks, and mark the trace so each question is attributable
 * to the CUDA call that provoked it.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * ★ THE POINT IS THE MARKERS.
 *
 * A flat ioctl trace of a CUDA process answers "was `0x20810108` requested?".
 * It does not answer "by which call", and that second question is the one that
 * decides what our port has to serve before `cuInit` can return 0. So every
 * stage writes a `MARK` line into the same trace file the interposer appends
 * to, in the same order, through the same append-mode fd semantics.
 *
 * ⊘ No CUDA toolkit is required and none is installed on the bench: the
 * entry points are taken by `dlsym` out of `libcuda.so.1` (the driver's own
 * library, version-matched by construction) and the prototypes are declared
 * here. Depending on a toolkit would make the instrument un-runnable on
 * exactly the machine that has the GPU.
 *
 * Stages, each stopping the ladder if it fails, so a failure localises:
 *
 *   1  cuInit(0)            — the rung `cup2` fails in the guest with 100
 *   2  cuDeviceGetCount     — the first question that needs a device
 *   3  cuDeviceGet(0)
 *   4  cuCtxCreate(0, dev)  — `cupctx2_min`'s rung
 *   5  cuCtxDestroy
 *
 * `NVPROBE_STOP=<n>` stops after stage n (default: all). Stage 1 alone is the
 * attributable capture for the `cuInit` window.
 *
 * BUILD:  gcc -O2 -o cuinit_probe cuinit_probe.c -ldl
 */

#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef int CUresult;
typedef int CUdevice;
typedef void *CUcontext;

static int mark_fd = -1;

/*
 * ⚠ The marker MUST go into the same file the interposer writes, through a
 * separate O_APPEND fd. `O_APPEND` makes each `write` atomic with respect to
 * the interposer's writes at the kernel level, so the interleaving in the file
 * is the real chronological order. A separate marker file would have to be
 * merged by timestamp afterwards, and there are no timestamps.
 */
static void mark_open(void) {
  const char *path = getenv("NVTRACE_OUT");
  if (path != NULL && path[0] != '\0') {
    mark_fd = open(path, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0644);
  }
}

static void mark(const char *what) {
  char line[256];
  int n = snprintf(line, sizeof(line), "MARK %s\n", what);
  if (mark_fd >= 0 && n > 0) {
    ssize_t ignored = write(mark_fd, line, (size_t)n);
    (void)ignored;
  }
  fprintf(stderr, "== %s\n", what);
}

int main(void) {
  void *lib;
  CUresult (*cu_init)(unsigned int);
  CUresult (*cu_device_get_count)(int *);
  CUresult (*cu_device_get)(CUdevice *, int);
  CUresult (*cu_ctx_create)(CUcontext *, unsigned int, CUdevice);
  CUresult (*cu_ctx_destroy)(CUcontext);
  const char *stop_env = getenv("NVPROBE_STOP");
  int stop = (stop_env != NULL && stop_env[0] != '\0') ? atoi(stop_env) : 99;
  int count = -1;
  CUdevice dev = -1;
  CUcontext ctx = NULL;
  CUresult r;

  mark_open();

  lib = dlopen("libcuda.so.1", RTLD_NOW);
  if (lib == NULL) {
    fprintf(stderr, "FAIL dlopen(libcuda.so.1): %s\n", dlerror());
    return 2;
  }
  cu_init = dlsym(lib, "cuInit");
  cu_device_get_count = dlsym(lib, "cuDeviceGetCount");
  cu_device_get = dlsym(lib, "cuDeviceGet");
  /* `_v2` is the ABI-versioned symbol libcuda actually exports; the unadorned
   * name is a macro in the toolkit header, not a symbol in the library. */
  cu_ctx_create = dlsym(lib, "cuCtxCreate_v2");
  cu_ctx_destroy = dlsym(lib, "cuCtxDestroy_v2");
  if (cu_init == NULL) {
    fprintf(stderr, "FAIL dlsym(cuInit): %s\n", dlerror());
    return 2;
  }

  mark("stage1 cuInit BEGIN");
  r = cu_init(0);
  mark("stage1 cuInit END");
  printf("cuInit(0) -> %d\n", (int)r);
  if (r != 0) {
    printf("PROBE_RC=1 (stopped at cuInit)\n");
    return 1;
  }
  if (stop < 2) {
    printf("PROBE_RC=0 (stopped by NVPROBE_STOP=%d)\n", stop);
    return 0;
  }

  mark("stage2 cuDeviceGetCount BEGIN");
  r = cu_device_get_count != NULL ? cu_device_get_count(&count) : -1;
  mark("stage2 cuDeviceGetCount END");
  printf("cuDeviceGetCount -> %d, count=%d\n", (int)r, count);
  if (r != 0 || count <= 0) {
    printf("PROBE_RC=1 (stopped at cuDeviceGetCount)\n");
    return 1;
  }
  if (stop < 3) {
    printf("PROBE_RC=0 (stopped by NVPROBE_STOP=%d)\n", stop);
    return 0;
  }

  mark("stage3 cuDeviceGet BEGIN");
  r = cu_device_get != NULL ? cu_device_get(&dev, 0) : -1;
  mark("stage3 cuDeviceGet END");
  printf("cuDeviceGet(0) -> %d, dev=%d\n", (int)r, (int)dev);
  if (r != 0) {
    printf("PROBE_RC=1 (stopped at cuDeviceGet)\n");
    return 1;
  }
  if (stop < 4) {
    printf("PROBE_RC=0 (stopped by NVPROBE_STOP=%d)\n", stop);
    return 0;
  }

  mark("stage4 cuCtxCreate BEGIN");
  r = cu_ctx_create != NULL ? cu_ctx_create(&ctx, 0, dev) : -1;
  mark("stage4 cuCtxCreate END");
  printf("cuCtxCreate(0, %d) -> %d\n", (int)dev, (int)r);
  if (r != 0) {
    printf("PROBE_RC=1 (stopped at cuCtxCreate)\n");
    return 1;
  }
  if (stop < 5) {
    printf("PROBE_RC=0 (stopped by NVPROBE_STOP=%d)\n", stop);
    return 0;
  }

  mark("stage5 cuCtxDestroy BEGIN");
  r = cu_ctx_destroy != NULL ? cu_ctx_destroy(ctx) : -1;
  mark("stage5 cuCtxDestroy END");
  printf("cuCtxDestroy -> %d\n", (int)r);
  printf("PROBE_RC=%d\n", r == 0 ? 0 : 1);
  return r == 0 ? 0 : 1;
}
