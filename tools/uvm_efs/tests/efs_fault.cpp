// efs_fault.cpp — b3 host-only proof: a real compute kernel faults in an EXTERNAL FAULT SERVICE
// VA space, the fault reaches THIS process instead of being cancelled, this process maps the page,
// a replay completes the kernel. Driver API only; the kernels are efs_kernels.cubin.
//
// Launch/ownership setup (docs/design/V3_UVM_B3_IMPLEMENTATION.md, "which UVM file owns the CUDA
// VA space"): libcuda opens this process's UVM file and creates, registers and maps everything in
// it. This process opts THAT file into EFS at UVM_INITIALIZE — before any registration — by OR-ing
// UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE | UVM_INIT_FLAGS_DISABLE_HMM into libcuda's own
// UVM_INITIALIZE (the ioctl() below is this executable's symbol, which libcuda's call binds to).
// No second file is involved and no other process's file is touched. `stock` mode leaves the flag
// out: same kernel, same unmapped access, stock UVM.
//
// Modes (argv[1]):
//   negative N      EFS on, all N pages mapped before launch: expect 0 records, correct data
//   service N [raw] EFS on, N unmapped pages, each fault serviced: map page, REPLAY. Latencies.
//                   `raw`: map with this process's own RM vidmem through UVM ioctls (kayfabe's
//                   shape) instead of cuMemMap.
//   read N          like service, but the kernel LOADS pre-filled pages (read faults)
//   cancel          first fault answered with CANCEL: the kernel must fail, nothing else
//   park MS         first fault parked MS milliseconds before map+replay (coexistence probe)
//   timeout         never answer: the kernel's own deadline (uvm_efs_timeout_ms) must cancel
//   crash [MS]      on the first fault, SIGKILL self (after MS ms) with the fault parked
//   ctxdestroy      on the first fault, destroy the CUDA context while the fault is parked
//   stock           EFS NOT requested: the unmapped access must fail as on any stock host
// Every line of output that matters starts with a tag the orchestrator greps.
#include <cuda.h>

#include <atomic>
#include <cinttypes>
#include <csignal>
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <algorithm>
#include <pthread.h>
#include <sys/types.h>
#include <time.h>
#include <vector>

#include "rm_mini.h"

// ------------------------------------------------------------------------------------------
// In-process opt-in: this executable's ioctl() is the one libcuda's calls bind to.
// ------------------------------------------------------------------------------------------
static std::atomic<int> g_want_efs{0};
static int g_uvm_fd = -1;
static NV_STATUS g_init_status = 0xffffffffu;
static NvU64 g_init_flags;

static bool fd_is(int fd, const char *path)
{
    char p[64], t[256];
    snprintf(p, sizeof(p), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(p, t, sizeof(t) - 1);
    if (n <= 0)
        return false;
    t[n] = 0;
    return strcmp(t, path) == 0;
}

extern "C" int ioctl(int fd, unsigned long req, ...)
{
    va_list ap;
    va_start(ap, req);
    void *arg = va_arg(ap, void *);
    va_end(ap);

    if (req == UVM_INITIALIZE && fd_is(fd, "/dev/nvidia-uvm")) {
        UVM_INITIALIZE_PARAMS *p = (UVM_INITIALIZE_PARAMS *)arg;
        if (g_want_efs.load())
            p->flags |= UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE | UVM_INIT_FLAGS_DISABLE_HMM;
        long r = syscall(SYS_ioctl, fd, req, arg);
        g_uvm_fd = fd;
        g_init_status = p->rmStatus;
        g_init_flags = p->flags;
        return (int)r;
    }

    return (int)syscall(SYS_ioctl, fd, req, arg);
}

// ------------------------------------------------------------------------------------------
#define CK(x) do { CUresult _e = (x); if (_e != CUDA_SUCCESS) { const char *_s = "?"; cuGetErrorName(_e, &_s); \
    printf("CUDA_FAIL %s -> %d %s (line %d)\n", #x, (int)_e, _s, __LINE__); exit(3); } } while (0)

static uint64_t now_real(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static const char *err_name(CUresult e)
{
    const char *s = "?";
    cuGetErrorName(e, &s);
    return s;
}

struct FaultTiming {
    unsigned page;
    uint64_t F;      // packet timestamp (GPU clock)
    uint64_t D;      // parked in the bottom half (CPU realtime)
    uint64_t R;      // received by this process (CPU realtime)
    uint64_t M;      // page mapped (CPU realtime)
    uint64_t Y;      // replay ioctl returned (CPU realtime)
    unsigned access, ftype, ninst;
};

static CUcontext g_ctx;
static CUmodule g_mod;
static CUfunction g_touch, g_spin;
static CUdeviceptr g_base, g_alias, g_out;
static size_t g_gran;
static unsigned g_npages, g_nchunks;
static std::vector<CUmemGenericAllocationHandle> g_chunks;
static std::vector<NvHandle> g_raw_mem;
static rmm_t g_rm;
static bool g_raw_map;
static std::atomic<int> g_done{0};
static std::vector<FaultTiming> g_timings;
static unsigned g_unexpected, g_extra_records;
static char g_mode[32];
static long g_mode_arg;
static std::atomic<int> g_first_fault_seen{0};
static std::atomic<int> g_ctx_destroyed{0};
static CUresult g_ctx_destroy_result = CUDA_SUCCESS;
static uint64_t g_first_fault_R, g_cancel_Y;

static void map_page_cuda(unsigned p)
{
    CUdeviceptr va = g_base + (CUdeviceptr)p * g_gran;
    CK(cuMemMap(va, g_gran, 0, g_chunks[p % g_nchunks], 0));
    CUmemAccessDesc acc;
    memset(&acc, 0, sizeof(acc));
    acc.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
    acc.location.id = 0;
    acc.flags = CU_MEM_ACCESS_FLAGS_PROT_READWRITE;
    CK(cuMemSetAccess(va, g_gran, &acc, 1));
}

// kayfabe's shape: this process's own RM vidmem, mapped with the stock UVM external-mapping
// ioctls on the EFS file. cuMemAddressReserve'd VAs have no UVM range of their own, so the
// range is created here.
static void map_page_raw(unsigned p)
{
    NvU64 va = g_base + (NvU64)p * g_gran;
    NV_STATUS s = uvm_create_ext_range(g_uvm_fd, va, g_gran);
    if (s != NV_OK)
        printf("RAW_MAP create_external_range page=%u status=0x%x\n", p, s);
    s = uvm_map_ext(g_uvm_fd, &g_rm.uuid, va, g_gran, g_rm.ctl, g_rm.client, g_raw_mem[p % g_nchunks], 0);
    if (s != NV_OK) {
        printf("RAW_MAP map_external page=%u status=0x%x\n", p, s);
        exit(4);
    }
}

static void *servicer(void *)
{
    CK(cuCtxSetCurrent(g_ctx));
    UvmEfsFaultRecord recs[64];

    while (!g_done.load()) {
        int n = efs_wait(g_uvm_fd, recs, 64, 100000);
        if (n < 0) {
            printf("EFS_WAIT_ERROR %d\n", n);
            break;
        }
        for (int i = 0; i < n; i++) {
            const UvmEfsFaultRecord &r = recs[i];
            FaultTiming t;
            memset(&t, 0, sizeof(t));
            t.R = now_real();
            t.F = r.gpuTimestampNs;
            t.D = r.divertTimeNs;
            t.access = r.accessType;
            t.ftype = r.faultType;
            t.ninst = r.numInstances;

            if (r.faultAddress < g_base || r.faultAddress >= g_base + (NvU64)g_npages * g_gran) {
                ++g_unexpected;
                printf("UNEXPECTED_RECORD va=0x%llx id=0x%llx access=%u type=%u -> CANCEL\n",
                       (unsigned long long)r.faultAddress, (unsigned long long)r.recordId, r.accessType, r.faultType);
                unsigned a, st;
                efs_resolve(g_uvm_fd, &r.recordId, 1, UVM_EFS_ACTION_CANCEL, &a, &st);
                continue;
            }
            t.page = (unsigned)((r.faultAddress - g_base) / g_gran);

            if (!g_first_fault_seen.exchange(1)) {
                g_first_fault_R = t.R;
                printf("FIRST_RECORD id=0x%llx va=0x%llx page=%u access=%u fault_type=%u client_type=%u "
                       "gpc=%u utlb=%u ve=%u mmu_engine=%u instances=%u\n",
                       (unsigned long long)r.recordId, (unsigned long long)r.faultAddress, t.page, r.accessType,
                       r.faultType, r.clientType, r.gpcId, r.utlbId, r.veId, r.mmuEngineId, r.numInstances);
                fflush(stdout);
            }

            if (!strcmp(g_mode, "cancel")) {
                unsigned a = 0, st = 0;
                NV_STATUS s = efs_resolve(g_uvm_fd, &r.recordId, 1, UVM_EFS_ACTION_CANCEL, &a, &st);
                g_cancel_Y = now_real();
                printf("CANCEL_ISSUED status=0x%x resolved=%u stale=%u ioctl_us=%.1f\n", s, a, st,
                       (g_cancel_Y - t.R) / 1e3);
                fflush(stdout);
                continue;
            }
            if (!strcmp(g_mode, "timeout"))
                continue;   // never answer
            if (!strcmp(g_mode, "crash")) {
                printf("CRASH_NOW after_ms=%ld (fault parked, SIGKILL self)\n", g_mode_arg);
                fflush(stdout);
                if (g_mode_arg > 0)
                    usleep(g_mode_arg * 1000);
                kill(getpid(), SIGKILL);
                pause();
            }
            if (!strcmp(g_mode, "ctxdestroy")) {
                printf("CTXDESTROY_NOW (fault parked)\n");
                fflush(stdout);
                uint64_t t0 = now_real();
                g_ctx_destroy_result = cuCtxDestroy(g_ctx);
                uint64_t t1 = now_real();
                printf("CTXDESTROY_DONE result=%d %s ms=%.1f\n", (int)g_ctx_destroy_result,
                       err_name(g_ctx_destroy_result), (t1 - t0) / 1e6);
                fflush(stdout);
                g_ctx_destroyed.store(1);
                return NULL;
            }
            if (!strcmp(g_mode, "park") && g_timings.empty()) {
                printf("PARK_BEGIN ms=%ld real_ns=%" PRIu64 "\n", g_mode_arg, now_real());
                fflush(stdout);
                usleep(g_mode_arg * 1000);
                printf("PARK_END real_ns=%" PRIu64 "\n", now_real());
                fflush(stdout);
            }

            if (g_raw_map)
                map_page_raw(t.page);
            else
                map_page_cuda(t.page);
            t.M = now_real();
            unsigned a = 0, st = 0;
            NV_STATUS s = efs_resolve(g_uvm_fd, &r.recordId, 1, UVM_EFS_ACTION_REPLAY, &a, &st);
            t.Y = now_real();
            if (s != NV_OK || a != 1)
                printf("REPLAY_PROBLEM page=%u status=0x%x resolved=%u stale=%u\n", t.page, s, a, st);
            if (t.page < g_timings.size() + 1 || true)
                g_timings.push_back(t);
        }
    }
    return NULL;
}

static double pct(std::vector<double> v, double q)
{
    if (v.empty())
        return 0;
    std::sort(v.begin(), v.end());
    size_t i = (size_t)(q * (v.size() - 1) + 0.5);
    return v[std::min(i, v.size() - 1)];
}

static void stat_line(const char *name, const std::vector<double> &v)
{
    if (v.empty()) {
        printf("LAT %-26s n=0\n", name);
        return;
    }
    printf("LAT %-26s n=%zu min=%.1f p50=%.1f p90=%.1f p99=%.1f max=%.1f us\n", name, v.size(), pct(v, 0),
           pct(v, 0.5), pct(v, 0.9), pct(v, 0.99), pct(v, 1.0));
}

// GPU PTIMER -> CPU realtime offset: min over samples of (t_read - gpu_value).
static int64_t calibrate(CUstream stream, volatile unsigned long long *slot, volatile unsigned int *stop,
                         CUdeviceptr dslot, CUdeviceptr dstop)
{
    *slot = 0;
    *stop = 0;
    void *args[] = {&dslot, &dstop};
    CK(cuLaunchKernel(g_spin, 1, 1, 1, 1, 1, 1, 0, stream, args, NULL));
    int64_t best = INT64_MAX;
    unsigned long long last = 0;
    uint64_t start = now_real();
    int samples = 0;
    while (now_real() - start < 200000000ull) {
        unsigned long long v = *slot;
        uint64_t t = now_real();
        if (v && v != last) {
            int64_t off = (int64_t)t - (int64_t)v;
            if (off < best)
                best = off;
            last = v;
            ++samples;
        }
    }
    *stop = 1;
    CK(cuStreamSynchronize(stream));
    printf("CALIB samples=%d offset_ns=%" PRId64 "\n", samples, best);
    return best;
}

static void print_query(const char *tag)
{
    UVM_EFS_QUERY_PARAMS q;
    int e = efs_query(g_uvm_fd, &q);
    if (e < 0) {
        printf("%s QUERY ioctl=%d\n", tag, e);
        return;
    }
    printf("%s QUERY rm=0x%x abi=%u enabled=%u active=%u max=%u timeout_ms=%u skip=%u parked=%u undelivered=%u "
           "diverted=%llu deduped=%llu refused_full=%llu refused_closing=%llu delivered=%llu replayed=%llu "
           "cancelled=%llu timed_out=%llu teardown=%llu stale=%llu gpu_gone=%llu hw_replays=%llu | "
           "g_skipped_batch_replays=%llu g_safety_replays=%llu g_efs_va_spaces=%llu\n",
           tag, q.rmStatus, q.abiVersion, q.moduleEnabled, q.active, q.maxRecords, q.timeoutMs,
           q.skipDivertedReplays, q.numParked, q.numUndelivered,
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_DIVERTED],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_DEDUPED],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_REFUSED_FULL],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_REFUSED_CLOSING],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_DELIVERED],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_RESOLVED_REPLAY],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_RESOLVED_CANCEL],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_TIMED_OUT],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_TEARDOWN_CANCELLED],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_STALE],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_DROPPED_GPU_GONE],
           (unsigned long long)q.vaSpaceCounters[UVM_EFS_CTR_HW_REPLAYS],
           (unsigned long long)q.globalCounters[UVM_EFS_GCTR_BATCH_REPLAYS_SKIPPED],
           (unsigned long long)q.globalCounters[UVM_EFS_GCTR_SAFETY_REPLAYS],
           (unsigned long long)q.globalCounters[UVM_EFS_GCTR_EFS_VA_SPACES]);
    fflush(stdout);
}

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "usage: efs_fault <mode> [arg] [raw]  (see header)\n");
        return 2;
    }
    snprintf(g_mode, sizeof(g_mode), "%s", argv[1]);
    g_mode_arg = argc > 2 ? atol(argv[2]) : 0;
    g_raw_map = argc > 3 && !strcmp(argv[3], "raw");
    const char *cubin = getenv("EFS_CUBIN") ? getenv("EFS_CUBIN") : "efs_kernels.cubin";

    bool stock = !strcmp(g_mode, "stock");
    g_want_efs.store(stock ? 0 : 1);

    unsigned n = 1;
    if (!strcmp(g_mode, "negative") || !strcmp(g_mode, "service") || !strcmp(g_mode, "read"))
        n = g_mode_arg > 0 ? (unsigned)g_mode_arg : 1;
    g_npages = n;
    int op = !strcmp(g_mode, "read") ? 1 : 0;

    printf("MODE %s arg=%ld pages=%u op=%s map=%s pid=%d\n", g_mode, g_mode_arg, n, op ? "load" : "store",
           g_raw_map ? "raw-rm+uvm-ioctl" : "cuMemMap", (int)getpid());

    CK(cuInit(0));
    printf("UVM_INIT fd=%d rmStatus=0x%x flags=0x%llx efs_requested=%d\n", g_uvm_fd, g_init_status,
           (unsigned long long)g_init_flags, g_want_efs.load());
    if (!stock && g_init_status != NV_OK) {
        printf("RESULT FAIL uvm_initialize_refused\n");
        return 1;
    }
    CUdevice dev;
    CK(cuDeviceGet(&dev, 0));
    char name[128];
    CK(cuDeviceGetName(name, sizeof(name), dev));
    CK(cuCtxCreate(&g_ctx, 0, dev));
    printf("DEVICE %s\n", name);
    print_query("START");

    CK(cuModuleLoad(&g_mod, cubin));
    CK(cuModuleGetFunction(&g_touch, g_mod, "touch_seq"));
    CK(cuModuleGetFunction(&g_spin, g_mod, "clock_spin"));
    CUstream stream;
    CK(cuStreamCreate(&stream, CU_STREAM_NON_BLOCKING));

    // host-mapped stamps + calibration slots
    volatile unsigned long long *h_stamps;
    CK(cuMemHostAlloc((void **)&h_stamps, sizeof(unsigned long long) * 2 * n + 64, CU_MEMHOSTALLOC_DEVICEMAP));
    memset((void *)h_stamps, 0, sizeof(unsigned long long) * 2 * n + 64);
    CUdeviceptr d_stamps;
    CK(cuMemHostGetDevicePointer(&d_stamps, (void *)h_stamps, 0));
    volatile unsigned long long *h_slot = h_stamps + 2 * n + 1;
    volatile unsigned int *h_stop = (volatile unsigned int *)(h_stamps + 2 * n + 3);
    CUdeviceptr d_slot = d_stamps + sizeof(unsigned long long) * (2 * n + 1);
    CUdeviceptr d_stop = d_stamps + sizeof(unsigned long long) * (2 * n + 3);
    CK(cuMemAlloc(&g_out, 64));
    CK(cuMemsetD32(g_out, 0, 16));

    int64_t off = calibrate(stream, h_slot, h_stop, d_slot, d_stop);

    // physical backing pool + an alias view of it (for pre-fill and for verification)
    CUmemAllocationProp prop;
    memset(&prop, 0, sizeof(prop));
    prop.type = CU_MEM_ALLOCATION_TYPE_PINNED;
    prop.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
    prop.location.id = 0;
    CK(cuMemGetAllocationGranularity(&g_gran, &prop, CU_MEM_ALLOC_GRANULARITY_MINIMUM));
    g_nchunks = std::min(n, 8u);
    CK(cuMemAddressReserve(&g_base, (size_t)n * g_gran, g_gran, 0, 0));
    CK(cuMemAddressReserve(&g_alias, (size_t)g_nchunks * g_gran, g_gran, 0, 0));
    CUmemAccessDesc acc;
    memset(&acc, 0, sizeof(acc));
    acc.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
    acc.location.id = 0;
    acc.flags = CU_MEM_ACCESS_FLAGS_PROT_READWRITE;
    if (g_raw_map) {
        int s = rmm_open(&g_rm);
        if (s) {
            printf("RESULT FAIL raw_rm_open=0x%x\n", s);
            return 1;
        }
        g_raw_mem.resize(g_nchunks);
        for (unsigned c = 0; c < g_nchunks; c++) {
            s = rmm_alloc_vidmem(&g_rm, g_gran, &g_raw_mem[c]);
            if (s) {
                printf("RESULT FAIL raw_vidmem=0x%x\n", s);
                return 1;
            }
        }
        printf("RAW_RM client=0x%x chunks=%u\n", g_rm.client, g_nchunks);
    }
    else {
        g_chunks.resize(g_nchunks);
        for (unsigned c = 0; c < g_nchunks; c++) {
            CK(cuMemCreate(&g_chunks[c], g_gran, &prop, 0));
            CK(cuMemMap(g_alias + (CUdeviceptr)c * g_gran, g_gran, 0, g_chunks[c], 0));
        }
        CK(cuMemSetAccess(g_alias, (size_t)g_nchunks * g_gran, &acc, 1));
        // pre-fill word 0 of each chunk with a per-chunk value the read mode sums
        for (unsigned c = 0; c < g_nchunks; c++)
            CK(cuMemsetD32(g_alias + (CUdeviceptr)c * g_gran, 0x100u + c, g_gran / 4));
        CK(cuCtxSynchronize());
    }
    printf("LAYOUT base=0x%llx alias=0x%llx gran=0x%zx pages=%u chunks=%u\n", (unsigned long long)g_base,
           (unsigned long long)g_alias, g_gran, n, g_nchunks);

    if (!strcmp(g_mode, "negative")) {
        for (unsigned p = 0; p < n; p++) {
            if (g_raw_map)
                map_page_raw(p);
            else
                map_page_cuda(p);
        }
        printf("NEGATIVE all %u pages mapped before launch\n", n);
    }

    print_query("PRELAUNCH");

    pthread_t th;
    bool need_servicer = !stock;
    if (need_servicer)
        pthread_create(&th, NULL, servicer, NULL);

    unsigned long long page_words = g_gran / 4;
    CUdeviceptr base = g_base;
    void *args[] = {&base, &n, &page_words, &d_stamps, &op, &g_out};
    uint64_t t_launch = now_real();
    CK(cuLaunchKernel(g_touch, 1, 1, 1, 1, 1, 1, 0, stream, args, NULL));
    printf("LAUNCHED real_ns=%" PRIu64 "\n", t_launch);
    fflush(stdout);

    CUresult kr;
    if (!strcmp(g_mode, "ctxdestroy")) {
        // Do not touch the context from this thread: the servicer destroys it mid-fault.
        uint64_t t0 = now_real();
        while (!g_ctx_destroyed.load() && now_real() - t0 < 60000000000ull)
            usleep(1000);
        g_done.store(1);
        pthread_join(th, NULL);
        printf("RESULT %s ctx_destroy=%s\n", g_ctx_destroyed.load() ? "CTXDESTROY_COMPLETED" : "CTXDESTROY_HUNG",
               err_name(g_ctx_destroy_result));
        return g_ctx_destroyed.load() ? 0 : 1;
    }

    kr = cuStreamSynchronize(stream);
    uint64_t t_end = now_real();
    g_done.store(1);
    if (need_servicer)
        pthread_join(th, NULL);

    printf("KERNEL_RESULT %d %s elapsed_ms=%.3f\n", (int)kr, err_name(kr), (t_end - t_launch) / 1e6);
    if (!strcmp(g_mode, "timeout") || !strcmp(g_mode, "cancel")) {
        uint64_t ref = !strcmp(g_mode, "cancel") ? g_cancel_Y : g_first_fault_R;
        if (ref)
            printf("ERROR_AFTER_%s_MS %.3f\n", !strcmp(g_mode, "cancel") ? "CANCEL" : "FIRST_RECORD",
                   (t_end - ref) / 1e6);
    }

    if (kr != CUDA_SUCCESS) {
        // The context is dead; only the host-side counters are left to read.
        print_query("END");
        bool expected = !strcmp(g_mode, "cancel") || !strcmp(g_mode, "timeout") || stock;
        printf("RESULT %s kernel_error=%s\n", expected ? "EXPECTED_KERNEL_ERROR" : "FAIL", err_name(kr));
        return expected ? 0 : 1;
    }

    // verify
    unsigned out[4];
    CK(cuMemcpyDtoH(out, g_out, sizeof(out)));
    unsigned bad = 0;
    if (op == 0) {
        for (unsigned p = 0; p < n; p++) {
            unsigned v = 0;
            // read back through the faulting VA itself: it is mapped now
            CK(cuMemcpyDtoH(&v, g_base + (CUdeviceptr)p * g_gran, sizeof(v)));
            if (v != (0xC0DE0000u | p))
                ++bad;
        }
    }
    else if (!g_raw_map) {
        unsigned expect = 0;
        for (unsigned p = 0; p < n; p++)
            expect += 0x100u + (p % g_nchunks);
        if (out[0] != expect)
            ++bad;
        printf("READ_SUM got=0x%x expect=0x%x\n", out[0], expect);
    }
    if (out[1] != 0xD0D0D0D0u)
        ++bad;

    print_query("END");

    // timings
    std::vector<double> hw, isr, wake, deliver, deliver_acc, service, map, complete, total;
    for (const FaultTiming &t : g_timings) {
        unsigned p = t.page;
        uint64_t g0 = h_stamps[2 * p], g1 = h_stamps[2 * p + 1];
        int64_t F = (int64_t)t.F + off, G0 = (int64_t)g0 + off, G1 = (int64_t)g1 + off;
        hw.push_back(((int64_t)t.F - (int64_t)g0) / 1e3);
        isr.push_back(((int64_t)t.D - F) / 1e3);
        wake.push_back(((int64_t)t.R - (int64_t)t.D) / 1e3);
        deliver.push_back(((int64_t)t.R - F) / 1e3);
        deliver_acc.push_back(((int64_t)t.R - G0) / 1e3);
        map.push_back(((int64_t)t.M - (int64_t)t.R) / 1e3);
        service.push_back(((int64_t)t.Y - (int64_t)t.R) / 1e3);
        complete.push_back((G1 - (int64_t)t.Y) / 1e3);
        total.push_back(((int64_t)g1 - (int64_t)g0) / 1e3);
    }
    std::vector<double> nofault;
    if (g_timings.empty())
        for (unsigned p = 0; p < n; p++)
            nofault.push_back(((int64_t)h_stamps[2 * p + 1] - (int64_t)h_stamps[2 * p]) / 1e3);

    printf("RECORDS serviced=%zu unexpected=%u pages=%u\n", g_timings.size(), g_unexpected, n);
    stat_line("hw_access_to_packet", hw);
    stat_line("packet_to_parked", isr);
    stat_line("parked_to_user", wake);
    stat_line("DELIVERY_packet_to_user", deliver);
    stat_line("access_to_user", deliver_acc);
    stat_line("map_(cuMemMap|uvm_map)", map);
    stat_line("service_user_to_replay", service);
    stat_line("replay_to_access_done", complete);
    stat_line("total_access_to_done", total);
    stat_line("no_fault_access", nofault);
    if (!g_timings.empty()) {
        const FaultTiming &t = g_timings[0];
        printf("FIRST_FAULT_RAW page=%u F=%" PRIu64 " D=%" PRIu64 " R=%" PRIu64 " M=%" PRIu64 " Y=%" PRIu64
               " g0=%llu g1=%llu off=%" PRId64 " access=%u type=%u inst=%u\n",
               t.page, t.F, t.D, t.R, t.M, t.Y, h_stamps[2 * t.page], h_stamps[2 * t.page + 1], off, t.access,
               t.ftype, t.ninst);
    }

    bool expect_records = !strcmp(g_mode, "service") || !strcmp(g_mode, "read") || !strcmp(g_mode, "park");
    bool ok = bad == 0 && g_unexpected == 0 &&
              (expect_records ? g_timings.size() == n : g_timings.empty());
    printf("DATA bad=%u\n", bad);
    printf("RESULT %s\n", ok ? "PASS" : "FAIL");
    return ok ? 0 : 1;
}
