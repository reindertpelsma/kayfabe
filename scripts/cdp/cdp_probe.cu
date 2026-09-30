// ★ Recovered 2026-09-30 unchanged (sha256 077050de…) from the vast 53004208 recovery bundle
//   (`traces/recovery_20260928/53004208.json.gz` on `recovery/resume-2026-09-28`, path
//   /root/cdp_probe/cdp_probe.cu). Build: nvcc -O2 -arch=sm_86 -rdc=true -o cdp_probe cdp_probe.cu -lcudadevrt
//   (`cdp_host.sh` / `cdp_guest.sh`). `docs/design/V3_CDP.md`.
// cdp_probe.cu — where does a CUDA dynamic-parallelism launch stop completing?
//   cdp_probe <mode> [auto|spin|yield|block]
//   mode 0: parent only (no device launch; module still linked with the device runtime)
//        1: child on the parent's implicit (NULL) device stream
//        2: child on cudaStreamFireAndForget
//        3: child on cudaStreamTailLaunch
//        4: child on a device-created cudaStreamNonBlocking stream (cdpSimpleQuicksort's shape)
// The kernels write progress flags into MAPPED HOST memory (zero-copy): [0] parent started,
// [1] child ran, [2] parent reached its end. The host first polls those flags and
// cudaStreamQuery / cudaEventQuery WITHOUT blocking (5 s), then blocks in cudaDeviceSynchronize
// under a 20 s watchdog.
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <chrono>
#include <thread>
#include <unistd.h>
#include <cuda_runtime.h>

__global__ void child(volatile unsigned *flags, unsigned *out) {
  out[1] = 0xC0FFEEu;
  flags[1] = 1u;
  __threadfence_system();
}

__global__ void parent(volatile unsigned *flags, unsigned *out, int mode) {
  flags[0] = 1u;
  __threadfence_system();
  if (mode == 1) {
    child<<<1, 1>>>(flags, out);
  } else if (mode == 2) {
    child<<<1, 1, 0, cudaStreamFireAndForget>>>(flags, out);
  } else if (mode == 3) {
    child<<<1, 1, 0, cudaStreamTailLaunch>>>(flags, out);
  } else if (mode == 4) {
    cudaStream_t s;
    cudaStreamCreateWithFlags(&s, cudaStreamNonBlocking);
    child<<<1, 1, 0, s>>>(flags, out);
    cudaStreamDestroy(s);
  }
  out[0] = 0xBEEFu;
  flags[2] = 1u;
  __threadfence_system();
}

static double now_s() {
  return std::chrono::duration<double>(std::chrono::steady_clock::now().time_since_epoch()).count();
}

int main(int argc, char **argv) {
  int mode = argc > 1 ? atoi(argv[1]) : 4;
  const char *sched = argc > 2 ? argv[2] : "auto";
  unsigned fl = cudaDeviceScheduleAuto;
  if (!strcmp(sched, "spin")) fl = cudaDeviceScheduleSpin;
  else if (!strcmp(sched, "yield")) fl = cudaDeviceScheduleYield;
  else if (!strcmp(sched, "block")) fl = cudaDeviceScheduleBlockingSync;
  cudaError_t e = cudaSetDeviceFlags(fl | cudaDeviceMapHost);
  printf("CDPP mode=%d sched=%s setflags=%d\n", mode, sched, (int)e);
  volatile unsigned *hflags = nullptr;
  e = cudaHostAlloc((void **)&hflags, 4096, cudaHostAllocMapped);
  if (e) { printf("CDPP hostalloc err=%d\n", (int)e); return 2; }
  memset((void *)hflags, 0, 4096);
  unsigned *dflags = nullptr;
  cudaHostGetDevicePointer((void **)&dflags, (void *)hflags, 0);
  unsigned *out = nullptr;
  cudaMalloc(&out, 4096);
  cudaMemset(out, 0, 4096);
  e = cudaDeviceSynchronize();
  printf("CDPP setup sync=%d\n", (int)e);
  cudaEvent_t ev;
  cudaEventCreateWithFlags(&ev, cudaEventDisableTiming);
  std::thread([] {
    sleep(20);
    printf("CDPP WATCHDOG: cudaDeviceSynchronize did not return in 20 s\n");
    fflush(stdout);
    _exit(3);
  }).detach();
  double t0 = now_s();
  parent<<<1, 1>>>((volatile unsigned *)dflags, out, mode);
  cudaError_t le = cudaGetLastError();
  cudaEventRecord(ev, 0);
  printf("CDPP launched err=%d\n", (int)le);
  fflush(stdout);
  double tp = -1, tc = -1, te = -1, tq = -1, tev = -1;
  while (now_s() - t0 < 5.0) {
    double t = now_s() - t0;
    if (tp < 0 && hflags[2]) tp = t;
    if (tc < 0 && hflags[1]) tc = t;
    if (tq < 0 && cudaStreamQuery(0) == cudaSuccess) tq = t;
    if (tev < 0 && cudaEventQuery(ev) == cudaSuccess) tev = t;
    if (tp >= 0 && (mode == 0 || tc >= 0) && tq >= 0 && tev >= 0) break;
    usleep(2000);
  }
  (void)te;
  printf("CDPP flags parent_started=%u child_ran=%u parent_end=%u | t_parent_end=%.3f t_child=%.3f t_streamquery_ok=%.3f t_eventquery_ok=%.3f (-1 = not within 5 s)\n",
         hflags[0], hflags[1], hflags[2], tp, tc, tq, tev);
  fflush(stdout);
  double ts = now_s();
  e = cudaDeviceSynchronize();
  printf("CDPP sync err=%d took=%.3f s\n", (int)e, now_s() - ts);
  unsigned o[2] = {0, 0};
  e = cudaMemcpy(o, out, 8, cudaMemcpyDeviceToHost);
  printf("CDPP out[0]=%#x out[1]=%#x memcpy=%d\n", o[0], o[1], (int)e);
  printf("CDPP RESULT mode=%d sched=%s %s\n", mode, sched,
         (o[0] == 0xBEEFu && (mode == 0 || o[1] == 0xC0FFEEu)) ? "OK" : "BAD");
  return 0;
}
