/*
 * nvd_prog.c — the differential workload. ONE program, staged, so host and guest
 * captures are byte-comparable and so a future question can re-run a shorter prefix.
 *
 *   nvd_prog init    cuInit only
 *   nvd_prog dev     + cuDeviceGet / attributes / totalMem
 *   nvd_prog ctx     + cuCtxCreate + cuCtxDestroy          <- the wall
 *   nvd_prog alloc   + cuMemAlloc / cuMemFree
 *   nvd_prog ce      + HtoD/DtoH round trip (this is what cup2 does: ZERO launches)
 *   nvd_prog launch  + one trivial kernel launch from embedded PTX
 *
 * Default stage is "ce" — i.e. the cup2 shape, no kernel launches, because that is
 * what our guest runs. "launch" is the first stage the CE emulator cannot forge.
 *
 * ## ★ THE FAULT STAGES (added 2026-08-12) — a KNOWN-POSITIVE for fault reporting
 *
 *   nvd_prog faultce   CE copy to a device VA that is NOT mapped in the context's VAS
 *   nvd_prog faultgr   compute-class global store to an unmapped VA (GR engine)
 *   nvd_prog bystander a long benign loop, to be run CONCURRENTLY with either fault
 *
 * Every claim this project holds about fault reporting rests on ABSENCE (no host event
 * registered, RcTriggered has no runtime producer, NV2080_NOTIFIERS_RC_ERROR armed zero
 * times). An absence is not evidence without a known-positive. These stages MAKE the GPU
 * fault on purpose so we can measure, on real hardware and then in our guest:
 *   (1) what the process is told, and on WHICH call (the offending one, or a later one);
 *   (2) BY WHAT MECHANISM — answered by the ioctl/mmap trace the shim records around the
 *       fault, not by narration;
 *   (3) what host dmesg says (the Xid identity: engine / client / access / fault type);
 *   (4) whether the CHANNEL survives — the question that discriminates "cuCtxCreate spins
 *       because the channel is DEAD (fault -> RC)" from "completions stopped for another
 *       reason". Measured by what every subsequent call on the victim context returns.
 *   (5) whether a BYSTANDER context keeps working — both in-process (a second CUcontext,
 *       exercised after the fault) and cross-process (the `bystander` stage).
 *
 * ⚠ A fault stage is EXPECTED to end with a failing CUDA call. Its exit status is
 * therefore NOT a pass/fail signal; read the FAULT= / VERDICT lines. The program still
 * exits 0 when it successfully provoked what it set out to provoke, and prints an
 * explicit NOFAULT verdict when it did not — because "the program errored" and "the GPU
 * faulted" are different facts and must not arrive as the same word.
 *
 * Build: cc -O0 -o nvd_prog nvd_prog.c -lcuda      (no nvcc, no cudart, no cuBLAS)
 */
/* ⊘ -DNVD_NO_CUDA_H selects the bundled stand-in (nvd_capture.sh sets it when the box has
 * libcuda but no toolkit). ⚠ It must be set on BOTH sides of a differential or neither:
 * the two binaries are supposed to be the same program. */
#ifdef NVD_NO_CUDA_H
#include "nvd_cuda_min.h"
#else
#include <cuda.h>
#endif
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <time.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>

/* ---------------------------------------------------------------------------
 * ★★★ PHASE MARKERS — why an ioctl trace alone could not answer the question.
 *
 * The mechanism question is "does the process learn about the fault through an
 * ioctl?", and the honest form of that is "are there ANY ioctls between the launch
 * and the cuCtxSynchronize that returns 700?". A flat 881-record trace cannot say:
 * teardown and workload look alike, and guessing the boundary is exactly the kind of
 * inference this project keeps having to retract.
 *
 * So the program writes its own labels INTO the trace. `_IOW('F', 0x7f, char[32])` on
 * /dev/nvidiactl is not a real NVIDIA escape (the RM escapes in use here are all well
 * below 0x7f); the driver returns EINVAL and does nothing, but the LD_PRELOAD recorder
 * captures the call with our 32 ASCII bytes as the header, so every record can be
 * attributed to a named phase. The marker is in-band, ordered by the same atomic
 * sequence number as everything else, and costs one failing syscall.
 *
 * ⊘ If /dev/nvidiactl cannot be opened the markers are silently absent — so the
 * analysis must REFUSE to phase-split an unmarked trace rather than assume one phase.
 * ------------------------------------------------------------------------ */
#define NVD_MARK_IOC _IOW('F', 0x7f, char[32])
static int g_markfd = -1;
static void mark(const char *tag)
{
    char buf[32];
    if (g_markfd < 0)
        return;
    memset(buf, 0, sizeof(buf));
    snprintf(buf, sizeof(buf), "NVDMARK:%s", tag);
    ioctl(g_markfd, NVD_MARK_IOC, buf);   /* EINVAL expected; the RECORD is the point */
}

#define CK(x) do{ CUresult r_=(x); const char*s_=0; if(r_!=CUDA_SUCCESS){ \
    cuGetErrorString(r_,&s_); printf("FAIL %s -> %s (%d)\n",#x,s_?s_:"?",r_); \
    fflush(stdout); return 1;} else { printf("ok   %s\n",#x); fflush(stdout);} }while(0)

/* ★ TRY: for the fault stages, where a FAILING call is the measurement, not an abort.
 * Prints the numeric CUresult AND the driver's own string for every call, so the report
 * can quote what the process was told rather than describing it. */
static const char *estr(CUresult r)
{
    const char *s = 0;
    cuGetErrorString(r, &s);
    return s ? s : "(no string)";
}
#define TRY(tag, x) ({ CUresult r__ = (x); \
    printf("  TRY %-22s %-34s rc=%d %s\n", tag, #x, (int)r__, estr(r__)); \
    fflush(stdout); r__; })

/* sm_52 PTX: out[i] = a[i] + b[i]; portable across Turing/Ampere via JIT. */
static const char *k_ptx =
".version 6.0\n.target sm_52\n.address_size 64\n"
".visible .entry vadd(.param .u64 p0, .param .u64 p1, .param .u64 p2)\n"
"{\n"
" .reg .b32 %r<4>; .reg .b64 %rd<10>;\n"
" ld.param.u64 %rd1, [p0]; ld.param.u64 %rd2, [p1]; ld.param.u64 %rd3, [p2];\n"
" cvta.to.global.u64 %rd4, %rd1; cvta.to.global.u64 %rd5, %rd2; cvta.to.global.u64 %rd6, %rd3;\n"
" mov.u32 %r1, %tid.x; mul.wide.u32 %rd7, %r1, 4;\n"
" add.s64 %rd8, %rd4, %rd7; ld.global.u32 %r2, [%rd8];\n"
" add.s64 %rd9, %rd5, %rd7; ld.global.u32 %r3, [%rd9];\n"
" add.s32 %r2, %r2, %r3;\n"
" add.s64 %rd8, %rd6, %rd7; st.global.u32 [%rd8], %r2;\n"
" ret;\n"
"}\n"
/* ★ the deliberate fault, GR side: a GLOBAL store to whatever address it is handed.
 * No cvta — the parameter IS the global address, so the access cannot be re-homed into
 * local/shared space by the generic-window rules and must go to the MMU. */
".visible .entry wildwr(.param .u64 w0)\n"
"{\n"
" .reg .b32 %w<2>; .reg .b64 %wd<2>;\n"
" ld.param.u64 %wd1, [w0];\n"
" mov.u32 %w1, 0xdeadbeef;\n"
" st.global.u32 [%wd1], %w1;\n"
" ret;\n"
"}\n";

/* ⚠ 64 GiB above a REAL allocation. Deliberately chosen, not arbitrary:
 *  - it is far outside anything the context has mapped, so it must miss the page tables;
 *  - it is still inside the 40-bit-ish GPU VA window a GA106 context uses, so the fault is
 *    a PAGE-TABLE miss (FAULT_PDE / FAULT_PTE) and not an out-of-range address rejected
 *    before the walk — the former is the class our guest's Xid 31 reports;
 *  - it is derived from a live pointer, so it is printed and can be matched against the
 *    fault address dmesg reports. A fault address we cannot predict is a fault we cannot
 *    attribute. */
#define WILD_OFF (64ULL << 30)

/* ★★★ THE GRADING RULE, and it exists because the first cut got it wrong (2026-08-12).
 * `cuMemcpyDtoD(big + BIG - 4096, big, BIG)` returned CUDA_ERROR_INVALID_VALUE (1) on the
 * offending call, and a grader that keyed on "a call failed" recorded that as
 * FAULT_PROVOKED. It was the OPPOSITE: libcuda bounds-checked the copy against the
 * allocation and refused it, so the engine never saw an address at all and dmesg was —
 * correctly — silent. ⊘ An API REFUSAL and a HARDWARE FAULT are different facts and they
 * arrive as the same word ("it failed").
 * ⇒ An attempt reached the engine only if the failure carries an EXECUTION status, or if
 * a LATER call failed (asynchrony is the signature of the engine, not of the API). */
#define CU_E_INVALID_VALUE   1
#define CU_E_ILLEGAL_ADDRESS 700
static int is_exec_err(CUresult r)
{
    return r == 700 /* ILLEGAL_ADDRESS */ || r == 715 /* ILLEGAL_INSTRUCTION */ ||
           r == 716 /* MISALIGNED_ADDRESS */ || r == 717 /* INVALID_ADDRESS_SPACE */ ||
           r == 718 /* INVALID_PC */ || r == 719 /* LAUNCH_FAILED */ ||
           r == 702 /* LAUNCH_TIMEOUT */;
}
static int reached_engine(CUresult op, CUresult sync)
{
    return is_exec_err(op) || sync != CUDA_SUCCESS;
}

enum { S_INIT, S_DEV, S_CTX, S_ALLOC, S_CE, S_LAUNCH,
       S_FAULTCE, S_FAULTGR, S_BYSTANDER };

static int stage_of(const char *s)
{
    if (!s || !strcmp(s, "ce"))     return S_CE;
    if (!strcmp(s, "init"))         return S_INIT;
    if (!strcmp(s, "dev"))          return S_DEV;
    if (!strcmp(s, "ctx"))          return S_CTX;
    if (!strcmp(s, "alloc"))        return S_ALLOC;
    if (!strcmp(s, "launch"))       return S_LAUNCH;
    if (!strcmp(s, "faultce"))      return S_FAULTCE;
    if (!strcmp(s, "faultgr"))      return S_FAULTGR;
    if (!strcmp(s, "bystander"))    return S_BYSTANDER;
    fprintf(stderr, "unknown stage '%s'\n", s);
    exit(2);
}

/* ---------------------------------------------------------------------------
 * The post-fault interrogation. Runs identically after a CE fault and after a GR
 * fault, so the two are comparable row by row.
 *
 * ⊘ "The channel is dead" is not inferred from the hang; it is READ OFF the driver's
 * answers to calls we make on purpose after the fault. Each row is one question.
 * ------------------------------------------------------------------------ */
static int interrogate(CUcontext victim, CUcontext bystander,
                       CUdeviceptr da, CUfunction fn_ok,
                       CUdeviceptr db, CUdeviceptr dc)
{
    unsigned hv = 0x5a5a5a5a, rv = 0;
    CUdeviceptr tmp = 0;
    CUresult r_sync, r_dtoh, r_alloc, r_htod, r_destroy;
    int faulted;

    printf("-- POST-FAULT INTERROGATION (victim context) --\n"); fflush(stdout);

    /* Q1: does a synchronisation point report it? */
    mark("q1_sync_begin");
    r_sync = TRY("q1_sync", cuCtxSynchronize());
    /* Q2: does a *fresh* data-plane op on the same context report it, or does it
     *     succeed (which would mean the channel is alive and only that one op died)? */
    mark("q2_dtoh_begin");
    r_dtoh = TRY("q2_dtoh", cuMemcpyDtoH(&rv, da, 4));
    /* Q3: does a CONTROL-plane op (allocation, no channel work) still work? This
     *     separates "the CONTEXT is poisoned" from "the CHANNEL is dead". */
    mark("q3_alloc_begin");
    r_alloc = TRY("q3_alloc", cuMemAlloc(&tmp, 4096));
    /* Q4: a second data-plane op, to show the error is sticky and not one-shot. */
    mark("q4_htod_begin");
    r_htod = TRY("q4_htod", cuMemcpyHtoD(da, &hv, 4));
    mark("q4_htod_end");

    printf("-- BYSTANDER, SAME PROCESS, SAME GPU, DIFFERENT CONTEXT --\n"); fflush(stdout);
    if (bystander) {
        CUresult rb1, rb2, rb3, rb4;
        unsigned bv = 0x1234abcd, brv = 0;
        mark("bystander_begin");
        rb1 = TRY("b1_setcurrent", cuCtxSetCurrent(bystander));
        rb2 = TRY("b2_htod",       cuMemcpyHtoD(db, &bv, 4));
        rb3 = TRY("b3_dtoh",       cuMemcpyDtoH(&brv, db, 4));
        rb4 = TRY("b4_sync",       cuCtxSynchronize());
        printf("BYSTANDER_INPROC rc=%d/%d/%d/%d value=0x%x want=0x%x -> %s\n",
               (int)rb1, (int)rb2, (int)rb3, (int)rb4, brv, bv,
               (rb1 || rb2 || rb3 || rb4 || brv != bv) ? "BROKEN" : "ALIVE");
        fflush(stdout);
        TRY("b5_setcurrent_back", cuCtxSetCurrent(victim));
    } else {
        printf("BYSTANDER_INPROC rc=- value=- -> NOT_CREATED\n"); fflush(stdout);
    }

    mark("ctxdestroy_begin");
    r_destroy = TRY("q5_ctxdestroy", cuCtxDestroy(victim));
    mark("ctxdestroy_end");

    /* ⚠ grade by the KIND of what came back, not by "something failed". Any nonzero on
     * a *later* call means the error is asynchronous and sticky, which is the signature
     * of RM having poisoned the channel rather than of one call being rejected. */
    faulted = (r_sync || r_dtoh || r_htod);
    printf("VICTIM_STATE sync=%d dtoh=%d alloc=%d htod=%d destroy=%d -> %s\n",
           (int)r_sync, (int)r_dtoh, (int)r_alloc, (int)r_htod, (int)r_destroy,
           faulted ? "POISONED" : "STILL_HEALTHY");
    fflush(stdout);
    (void)fn_ok; (void)dc;
    return faulted;
}

/* The cross-process bystander: a benign loop on its own context, meant to be running
 * while another process faults the same GPU. [[gpu_fault_is_contained]] measured
 * 2 675 519 iterations through someone else's fault — this CONFIRMS rather than assumes,
 * because that measurement predates every change to the execution plane since. */
static int run_bystander(CUdevice d, int seconds)
{
    CUcontext ctx;
    CUdeviceptr da = 0;
    unsigned long long iters = 0, fails = 0;
    struct timespec t0, tn;

    CK(cuCtxCreate(&ctx, 0, d));
    CK(cuMemAlloc(&da, 4096));
    printf("BYSTANDER_START seconds=%d\n", seconds); fflush(stdout);
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (;;) {
        unsigned hv = (unsigned)(0x1000 + (iters & 0xffff)), rv = 0;
        CUresult r1 = cuMemcpyHtoD(da, &hv, 4);
        CUresult r2 = cuMemcpyDtoH(&rv, da, 4);
        if (r1 || r2 || rv != hv) {
            if (fails < 5)
                printf("BYSTANDER_FAIL iter=%llu r1=%d(%s) r2=%d(%s) got=0x%x want=0x%x\n",
                       iters, (int)r1, estr(r1), (int)r2, estr(r2), rv, hv);
            fails++;
        }
        iters++;
        if ((iters % 20000) == 0) {
            printf("BYSTANDER_PROGRESS iters=%llu fails=%llu\n", iters, fails);
            fflush(stdout);
        }
        clock_gettime(CLOCK_MONOTONIC, &tn);
        if (tn.tv_sec - t0.tv_sec >= seconds) break;
    }
    printf("BYSTANDER_XPROC iters=%llu fails=%llu -> %s\n",
           iters, fails, fails ? "BROKEN" : "ALIVE");
    fflush(stdout);
    return fails ? 1 : 0;
}

static int run_fault_stage(int stage, CUdevice d)
{
    CUcontext victim = 0, byst = 0;
    CUdeviceptr da = 0, db = 0, dc = 0, resv = 0;
    CUresult r_op = CUDA_SUCCESS, r_sync = CUDA_SUCCESS;
    const char *how = "none";
    unsigned long long faultva = 0;
    int poisoned;

    g_markfd = open("/dev/nvidiactl", O_RDWR);
    printf("MARKFD %d%s\n", g_markfd,
           g_markfd < 0 ? "  ⊘ NO MARKERS — the trace must NOT be phase-split" : "");
    fflush(stdout);
    mark("setup_begin");

    /* victim first, bystander second: cuCtxCreate leaves the NEW context current, so
     * allocating db right after creating byst puts it in byst's VAS, which is the point. */
    CK(cuCtxCreate(&victim, 0, d));
    CK(cuCtxCreate(&byst, 0, d));
    CK(cuMemAlloc(&db, 4096));                 /* bystander's, in bystander's VAS */
    CK(cuCtxSetCurrent(victim));
    CK(cuMemAlloc(&da, 1 << 20));              /* victim's, 1 MiB so a CE copy is real */
    printf("VICTIM da=0x%llx  BYSTANDER db=0x%llx\n",
           (unsigned long long)da, (unsigned long long)db);
    fflush(stdout);

    if (stage == S_FAULTCE) {
        CUdeviceptr big = 0;
        /* ⚠ 256 MiB, not 64. CUDA sub-allocates small buffers out of large VA slabs, so a
         * short overrun can land on a DIFFERENT live mapping — corrupting a neighbour and
         * producing no fault at all, which would read as "the GPU does not report". The
         * overrun has to be big enough to leave the slab. */
        const size_t BIG = 256u << 20;
        printf("-- PROVOKE: COPY ENGINE write to an UNMAPPED device VA --\n");
        fflush(stdout);

        /* ★★★ MEASURED 2026-08-12, and it is the reason this stage looks the way it does.
         * The obvious provocation — cuMemAddressReserve() a VA, leave it UNMAPPED, and
         * cuMemcpyHtoD into it — never reaches the GPU. libcuda does not recognise a
         * reserved-but-unmapped VA as a device allocation, falls into its PAGEABLE-HOST
         * copy path, and executes an SSE store loop against that address ON THE CPU:
         *   nvd_prog[1045501]: segfault at 7ce496600010 ip ...
         *     in libcuda.so.580.159.04 ... error 6
         * i.e. the process dies of a HOST page fault at the reserved VA + 0x10, with no
         * pushbuffer, no engine, and no Xid. ⊘ That is an instrument failure that reads
         * exactly like a provoked fault if you grade on "the program crashed": rc=139 and
         * a segfault line in dmesg. It is kept, behind NVD_TRY_RESERVED=1, because the
         * negative is worth being able to re-run — never as the default path.
         * ⇒ To fault the ENGINE, both pointers must stay INSIDE ranges libcuda knows, so
         * that it dispatches a real copy; only the LENGTH runs off the end. */
        if (getenv("NVD_TRY_RESERVED")) {
            void *hbuf = malloc(1 << 20);
            memset(hbuf, 0x5c, 1 << 20);
            if (TRY("X_reserve", cuMemAddressReserve(&resv, 2u << 20, 0, 0, 0)) == CUDA_SUCCESS) {
                printf("  reserved-unmapped VA = 0x%llx (⚠ expect a HOST segfault, not an Xid)\n",
                       (unsigned long long)resv);
                fflush(stdout);
                r_op = TRY("X_htod_1MiB", cuMemcpyHtoD(resv, hbuf, 1 << 20));
                r_sync = TRY("X_sync", cuCtxSynchronize());
                if (r_op || r_sync) { how = "X_htod_to_reserved_unmapped"; faultva = resv; }
            }
            free(hbuf);
        }

        CK(cuMemAlloc(&big, BIG));
        printf("  big=0x%llx size=%zu  end=0x%llx\n",
               (unsigned long long)big, BIG, (unsigned long long)big + BIG);
        fflush(stdout);

        /* ★ Attempt A — the VMM route, and the ONLY one that reaches the engine.
         * Reserve a VA, back it, map it, let CUDA record it as a device allocation, then
         * UNMAP it while KEEPING the reservation. libcuda's pointer table still calls the
         * range device memory, so its bounds check passes; the GPU page tables hold
         * nothing, so the copy engine meets an unmapped page. That is exactly "a CE copy
         * to an address that is not mapped in the context's VAS". */
        {
            CUmemAllocationProp prop;
            CUmemAccessDesc acc;
            CUmemGenericAllocationHandle h = 0;
            size_t gran = 2u << 20, sz;
            CUdeviceptr va = 0;
            int ok = 1;

            memset(&prop, 0, sizeof(prop));
            prop.type = CU_MEM_ALLOCATION_TYPE_PINNED;
            prop.requestedHandleTypes = CU_MEM_HANDLE_TYPE_NONE;
            prop.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
            prop.location.id = (int)d;
            if (TRY("A_granularity",
                    cuMemGetAllocationGranularity(&gran, &prop,
                                                  CU_MEM_ALLOC_GRANULARITY_MINIMUM)))
                ok = 0;
            sz = gran ? ((4u << 20) + gran - 1) / gran * gran : (4u << 20);
            printf("  vmm granularity=%zu size=%zu\n", gran, sz); fflush(stdout);
            if (ok && TRY("A_reserve", cuMemAddressReserve(&va, sz, 0, 0, 0))) ok = 0;
            if (ok && TRY("A_create",  cuMemCreate(&h, sz, &prop, 0))) ok = 0;
            if (ok && TRY("A_map",     cuMemMap(va, sz, 0, h, 0))) ok = 0;
            if (ok) {
                memset(&acc, 0, sizeof(acc));
                acc.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
                acc.location.id = (int)d;
                acc.flags = CU_MEM_ACCESS_FLAGS_PROT_READWRITE;
                if (TRY("A_setaccess", cuMemSetAccess(va, sz, &acc, 1))) ok = 0;
            }
            /* prove the VA works while MAPPED — otherwise a later failure could be the
             * setup's, and we would book our own bug as the GPU's report. */
            if (ok) {
                CUresult rp = TRY("A_probe_mapped", cuMemcpyDtoD(va, big, 4096));
                CUresult rs = TRY("A_probe_sync", cuCtxSynchronize());
                if (rp || rs) { printf("  ⊘ VMM VA not usable even MAPPED — setup bug, "
                                       "attempt A is VOID\n"); ok = 0; }
            }
            /* ★★★ REVOKE ACCESS, do NOT unmap — and the difference is everything.
             * cuMemUnmap removes the range from libcuda's pointer-attribute table, after
             * which libcuda no longer believes the VA is device memory and CPU-memcpys
             * into it (measured twice: SIGSEGV at va+0x10 inside libcuda, identical
             * instruction both times, no Xid, no engine). cuMemSetAccess(PROT_NONE)
             * leaves the MAPPING in place — so libcuda still dispatches a real copy — and
             * takes the PERMISSION away in the GPU page tables, which is where we want
             * the refusal to happen. */
            if (ok) {
                acc.flags = 0;  /* CU_MEM_ACCESS_FLAGS_PROT_NONE */
                if (TRY("A_revoke", cuMemSetAccess(va, sz, &acc, 1))) ok = 0;
            }
            if (ok) {
                faultva = (unsigned long long)va;
                printf("  MAPPED-but-NO-ACCESS device VA = 0x%llx\n", faultva);
                fflush(stdout);
                r_op = TRY("A_dtod_no_access", cuMemcpyDtoD(va, big, sz));
                r_sync = TRY("A_sync", cuCtxSynchronize());
                if (reached_engine(r_op, r_sync)) how = "A_dtod_to_prot_none_vmm_va";
                else printf("  A: rc=%d refused by the API, engine never saw it\n", (int)r_op);
            }
            /* ⊘ The unmap variant is a MEASURED NEGATIVE, kept re-runnable and OFF: it
             * kills the process with a host segfault before any engine is involved. */
            if (ok && getenv("NVD_TRY_UNMAPPED") && !reached_engine(r_op, r_sync)) {
                if (!TRY("A2_unmap", cuMemUnmap(va, sz))) {
                    r_op = TRY("A2_dtod_unmapped", cuMemcpyDtoD(va, big, sz));
                    r_sync = TRY("A2_sync", cuCtxSynchronize());
                    if (reached_engine(r_op, r_sync)) how = "A2_dtod_to_unmapped_vmm_va";
                }
            }
            if (h) TRY("A_release", cuMemRelease(h));
        }

        /* Attempt B — dst INSIDE a live allocation, length running 256 MiB past its end.
         * ⊘ MEASURED TO BE REFUSED: libcuda bounds-checks cuMemcpyDtoD against the
         * allocation and returns CUDA_ERROR_INVALID_VALUE (1) synchronously. Kept as a
         * documented negative: it is the attempt that taught the grading rule above. */
        if (!reached_engine(r_op, r_sync)) {
            r_op = TRY("B_dtod_overrun", cuMemcpyDtoD(big + (BIG - 4096), big, BIG));
            r_sync = TRY("B_sync", cuCtxSynchronize());
            if (reached_engine(r_op, r_sync)) {
                how = "B_dtod_overrun_past_allocation_end";
                faultva = (unsigned long long)big + BIG;
            } else { r_op = CUDA_SUCCESS; r_sync = CUDA_SUCCESS; }
        }
        /* Attempt C — a pointer allocated in the OTHER context, used from the victim's.
         * The range is known to libcuda but mapped only in the bystander's VAS. */
        if (!reached_engine(r_op, r_sync)) {
            faultva = (unsigned long long)db;
            r_op = TRY("C_dtod_crossctx", cuMemcpyDtoD(db, big, 4096));
            r_sync = TRY("C_sync", cuCtxSynchronize());
            if (reached_engine(r_op, r_sync)) how = "C_dtod_to_other_contexts_va";
            else { r_op = CUDA_SUCCESS; r_sync = CUDA_SUCCESS; }
        }

        /* ★★★ MEASURED RESULT OF THIS WHOLE LADDER (vh2, GA106, 580.159.04, 2026-08-12):
         * NONE of these reaches a copy engine, and the reasons are worth keeping because
         * they are two DIFFERENT defences, not one:
         *   A (PROT_NONE, still mapped)   -> CUDA_ERROR_INVALID_VALUE, synchronous
         *   A2 (unmapped, still reserved) -> host SIGSEGV inside libcuda, no engine
         *   B (length past the end)       -> CUDA_ERROR_INVALID_VALUE, synchronous
         *   C (the other context's VA)    -> SUCCEEDS; UVA makes a same-device peer copy
         *                                    legal, so this is not even an error
         * ⇒ THE PUBLIC CUDA MEMCPY API CANNOT BE MADE TO ISSUE A CE COPY TO AN ADDRESS THE
         * CONTEXT CANNOT ACCESS. libcuda checks the pointer against its own range table
         * first; a pointer that fails the check either is rejected or degrades to a HOST
         * pointer and the CPU takes the fault. A CE-client Xid is therefore not reachable
         * from userspace CUDA — use `faultgr`, which faults from a shader (GPCCLIENT), or
         * drive a channel directly. ⊘ dmesg silence on this stage is CORRECT and must not
         * be read as "the GPU does not report faults". */
        printf("CE_LADDER prot_none/overrun/crossctx all failed to reach an engine "
               "(see the block comment) -> a CE-client Xid needs a raw channel, not CUDA\n");
        fflush(stdout);
    } else {                                    /* S_FAULTGR */
        CUmodule mod; CUfunction fn; void *args[1];
        printf("-- PROVOKE: COMPUTE-CLASS global store to an UNMAPPED device VA --\n");
        fflush(stdout);
        CK(cuModuleLoadData(&mod, k_ptx));
        CK(cuModuleGetFunction(&fn, mod, "wildwr"));
        faultva = (unsigned long long)da + WILD_OFF;
        printf("  wild VA = 0x%llx (da + 64GiB)\n", faultva); fflush(stdout);
        {
            CUdeviceptr wp = (CUdeviceptr)faultva;
            args[0] = &wp;
            mark("launch_begin");
            r_op = TRY("G_launch", cuLaunchKernel(fn, 1, 1, 1, 1, 1, 1, 0, 0, args, 0));
            mark("launch_returned");
            r_sync = TRY("G_sync", cuCtxSynchronize());
            mark("provoke_sync_returned");
        }
        if (r_op || r_sync) how = "G_wild_global_store";
    }

    /* ⊘ The two facts are reported separately and must stay separate: WHICH CALL first
     * returned nonzero (that is the delivery point the process observes) and WHETHER the
     * GPU faulted at all (which only dmesg's Xid can settle). A program that conflates
     * them would report an API-level rejection as a hardware fault. */
    printf("FAULT how=%s va=0x%llx first_op_rc=%d(%s) sync_rc=%d(%s) "
           "delivered_on=%s\n",
           how, faultva, (int)r_op, estr(r_op), (int)r_sync, estr(r_sync),
           is_exec_err(r_op) ? "THE_OFFENDING_CALL"
             : (r_sync ? "A_LATER_SYNC"
             : (r_op ? "API_REFUSAL_NOT_A_FAULT" : "NOTHING")));
    fflush(stdout);

    poisoned = interrogate(victim, byst, da, 0, db, dc);

    if (reached_engine(r_op, r_sync) || poisoned)
        printf("VERDICT FAULT_PROVOKED how=%s\n", how);
    else if (r_op == CU_E_INVALID_VALUE)
        printf("VERDICT API_REFUSED_NO_FAULT — libcuda rejected the address before the "
               "engine saw it; dmesg silence here is CORRECT, not a missing report\n");
    else
        printf("VERDICT NOFAULT — nothing we did produced an error at any level\n");
    fflush(stdout);
    printf("DONE\n"); fflush(stdout);
    return 0;   /* the fault IS the result; see the header note on exit status */
}

int main(int argc, char **argv)
{
    int stage = stage_of(argc > 1 ? argv[1] : NULL);
    CUdevice d;
    CUcontext ctx;
    CUdeviceptr da = 0, db = 0, dc = 0;
    int n = 0, maj = 0, min = 0;
    char nm[256];
    size_t tot = 0;

    printf("STAGE %s\n", argc > 1 ? argv[1] : "ce"); fflush(stdout);

    CK(cuInit(0));
    if (stage == S_INIT) goto done;

    CK(cuDeviceGetCount(&n));
    printf("devices=%d\n", n); fflush(stdout);
    if (n < 1) return 1;
    CK(cuDeviceGet(&d, 0));
    memset(nm, 0, sizeof(nm));
    CK(cuDeviceGetName(nm, sizeof(nm), d));
    printf("name=%s\n", nm); fflush(stdout);
    CK(cuDeviceGetAttribute(&maj, CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR, d));
    CK(cuDeviceGetAttribute(&min, CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR, d));
    printf("compute=%d.%d\n", maj, min); fflush(stdout);
    CK(cuDeviceTotalMem(&tot, d));
    printf("totalMem=%zu MiB\n", (size_t)(tot >> 20)); fflush(stdout);
    if (stage == S_DEV) goto done;

    if (stage == S_BYSTANDER)
        return run_bystander(d, argc > 2 ? atoi(argv[2]) : 30);
    if (stage == S_FAULTCE || stage == S_FAULTGR)
        return run_fault_stage(stage, d);

    CK(cuCtxCreate(&ctx, 0, d));
    printf("CTX OK\n"); fflush(stdout);
    if (stage == S_CTX) { CK(cuCtxDestroy(ctx)); goto done; }

    CK(cuMemAlloc(&da, 4096));
    printf("MEMALLOC OK\n"); fflush(stdout);
    if (stage == S_ALLOC) { CK(cuMemFree(da)); CK(cuCtxDestroy(ctx)); goto done; }

    {
        unsigned hv = 0xabcd1234, rv = 0;
        CK(cuMemcpyHtoD(da, &hv, 4));
        CK(cuMemcpyDtoH(&rv, da, 4));
        printf("CE rv=0x%x want=0x%x -> %s\n", rv, hv, rv == hv ? "PASS" : "MISMATCH");
        fflush(stdout);
        if (rv != hv) return 1;
    }
    if (stage == S_CE) { CK(cuMemFree(da)); CK(cuCtxDestroy(ctx)); goto done; }

    {
        CUmodule mod; CUfunction fn;
        unsigned ha[32], hb[32], hc[32];
        void *args[3];
        int i, bad = 0;
        for (i = 0; i < 32; i++) { ha[i] = (unsigned)i; hb[i] = (unsigned)(100 + i); }
        CK(cuModuleLoadData(&mod, k_ptx));
        CK(cuModuleGetFunction(&fn, mod, "vadd"));
        CK(cuMemAlloc(&db, 128)); CK(cuMemAlloc(&dc, 128));
        CK(cuMemcpyHtoD(da, ha, 128));
        CK(cuMemcpyHtoD(db, hb, 128));
        args[0] = &da; args[1] = &db; args[2] = &dc;
        CK(cuLaunchKernel(fn, 1, 1, 1, 32, 1, 1, 0, 0, args, 0));
        CK(cuCtxSynchronize());
        CK(cuMemcpyDtoH(hc, dc, 128));
        for (i = 0; i < 32; i++) if (hc[i] != ha[i] + hb[i]) bad++;
        printf("LAUNCH bad=%d c[0]=%u c[31]=%u -> %s\n", bad, hc[0], hc[31],
               bad ? "MISMATCH" : "PASS");
        fflush(stdout);
        CK(cuMemFree(dc)); CK(cuMemFree(db)); CK(cuMemFree(da));
        CK(cuCtxDestroy(ctx));
        if (bad) return 1;
    }

done:
    printf("DONE\n"); fflush(stdout);
    return 0;
}
