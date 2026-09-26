/* kf_tests.cu — the attacking test suite for the walk kernel.
 *
 * Build the tables here, mutate them, assert what the kernel reports. Every
 * hostile case asserts WHICH refusal fired (refuse_mask), not merely that
 * something was refused: a test that fails for the wrong reason is worse than
 * no test.
 */
#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <string>
#include <vector>
#include <thread>
#include <atomic>
#include <chrono>
#include <algorithm>
#include <map>
#include <set>

#include "kf_walk.h"
#include "kf_tables.h"

/* ── a very small assertion framework ────────────────────────────────────────── */
static const char *g_name = "";
static int g_fails_here = 0;
static int g_pass = 0, g_fail = 0;

static void failf(int line, const char *expr, const char *extra)
{
    g_fails_here++;
    printf("    ! %s:%d  %s%s%s\n", g_name, line, expr, extra ? "  -- " : "", extra ? extra : "");
}
#define CHECK(c)         do { if (!(c)) failf(__LINE__, #c, NULL); } while (0)
#define CHECK_M(c, m)    do { if (!(c)) failf(__LINE__, #c, (m)); } while (0)
#define CHECK_EQ(a, b)   do { unsigned long long _x = (unsigned long long)(a), _y = (unsigned long long)(b); \
    if (_x != _y) { char _b[128]; snprintf(_b, sizeof(_b), "%llu != %llu (0x%llx != 0x%llx)", _x, _y, _x, _y); \
    failf(__LINE__, #a " == " #b, _b); } } while (0)

#define CUDA_OK(x) do { cudaError_t _e = (x); if (_e != cudaSuccess) { \
    char _b[160]; snprintf(_b, sizeof(_b), "%s", cudaGetErrorString(_e)); failf(__LINE__, #x, _b); } } while (0)

/* ── the fixture ─────────────────────────────────────────────────────────────── */
struct Fix {
    Gpga g;
    void *dev;
    KfWalk *w;
    KfWalkCfg cfg;
    KfReportHeader hdr;
    std::vector<KfPdbEntry> pe;
    std::vector<KfMapRun> rn;
    bool host_mapped;

    Fix(size_t bytes, KfWalkCfg c, bool mapped = false)
        : g(bytes), dev(NULL), w(NULL), cfg(c), host_mapped(mapped)
    {
        memset(&hdr, 0, sizeof(hdr));
        if (mapped) {
            void *h = NULL;
            cudaHostAlloc(&h, bytes, cudaHostAllocMapped);
            host_ptr = (uint8_t *)h;
            cudaHostGetDevicePointer(&dev, h, 0);
        } else {
            cudaMalloc(&dev, bytes);
            host_ptr = NULL;
        }
        w = kf_create(&cfg);
        pe.resize(cfg.pdb_capacity + 4);
        rn.resize(cfg.run_capacity + 4);
    }
    ~Fix()
    {
        kf_destroy(w);
        if (host_mapped) cudaFreeHost(host_ptr); else cudaFree(dev);
    }
    uint8_t *host_ptr;

    void upload()
    {
        if (host_mapped) memcpy(host_ptr, g.mem.data(), g.size());
        else cudaMemcpy(dev, g.mem.data(), g.size(), cudaMemcpyHostToDevice);
    }
    /* push one edited 8-byte entry without re-uploading the whole buffer */
    void poke(uint64_t off)
    {
        if (host_mapped) memcpy(host_ptr + off, g.mem.data() + off, 8);
        else cudaMemcpy((uint8_t *)dev + off, g.mem.data() + off, 8, cudaMemcpyHostToDevice);
    }

    /* Entry i is diffed against slot i. With no verdict in between, every
     * refresh diffs against the same (initially empty) slot, so a slot that was
     * never acknowledged reports the FULL walk as MAPs. */
    int refresh(const std::vector<uint64_t> &roots)
    {
        std::vector<uint32_t> sl(roots.size());
        for (size_t i = 0; i < sl.size(); i++) sl[i] = (uint32_t)i;
        return refresh_slots(roots, sl);
    }
    int refresh_slots(const std::vector<uint64_t> &roots, const std::vector<uint32_t> &sl)
    {
        return kf_refresh(w, dev, g.size(), roots.data(), sl.data(), (uint32_t)roots.size(),
                          &hdr, pe.data(), rn.data());
    }
    /* The host's verdict on the last report: everything applied. */
    void ack()
    {
        std::vector<uint8_t> c(hdr.run_count ? hdr.run_count : 1, KFWR_ACK_APPLIED);
        kf_ack(w, hdr.generation, c.data(), hdr.run_count, NULL, 0);
    }
    /* A chosen verdict (one code per run) and slots to release. */
    void ack_codes(const std::vector<uint8_t> &c, const std::vector<uint32_t> &resets = std::vector<uint32_t>())
    {
        kf_ack(w, hdr.generation, c.empty() ? NULL : c.data(), (uint32_t)c.size(),
               resets.empty() ? NULL : resets.data(), (uint32_t)resets.size());
    }
};

static KfWalkCfg cfg_default(void)
{
    KfWalkCfg c;
    c.runs_per_pdb = 4096;
    c.run_capacity = 8192;
    c.pdb_capacity = 8;
    c.entry_budget = 4000000u;
    c.max_pdbs = 8;
    c.max_slots = 8;
    c.table_version = KF_TBL_VER2;
    /* ⊘ §39(c) UNBOUNDED BY DEFAULT, and the reason is not convenience: a Fix's
     * buffer holds TABLE PAGES, not a framebuffer. `Fix f(8u << 20, ...)` then
     * maps leaves at 0x20000000 on purpose, modelling a guest whose GPGA space
     * is far larger than the bytes this test needs to make readable. Bounding
     * leaves by the buffer would refuse those legitimately -- it refused 263 of
     * them on the real-GA106 corpus before this field existed. The four
     * hostile/leaf_* cases set a TIGHT span and are where containment is pinned. */
    c.gpga_span = KF_GPGA_SPAN_UNBOUNDED;
    return c;
}

/* ── report helpers ──────────────────────────────────────────────────────────── */
static void dump(const Fix &f)
{
    printf("      hdr gen=%llu ack=%llu flags=0x%04x pdb=%u/%u run=%u/%u visited=%llu ref=%u mask=0x%x\n",
           (unsigned long long)f.hdr.generation, (unsigned long long)f.hdr.acked_generation,
           f.hdr.flags, f.hdr.pdb_count, f.hdr.pdb_capacity, f.hdr.run_count, f.hdr.run_capacity,
           (unsigned long long)f.hdr.entries_visited, f.hdr.refusals, f.hdr.refuse_mask);
    for (uint32_t i = 0; i < f.hdr.pdb_count && i < 8; i++)
        printf("      pdb[%u] 0x%llx first=%u n=%u vflags=0x%x\n", i,
               (unsigned long long)f.pe[i].pdb, f.pe[i].first_run, f.pe[i].run_count, f.pe[i].vas_flags);
    for (uint32_t i = 0; i < f.hdr.run_count && i < 24; i++)
        printf("      run[%u] op=%u va=0x%llx gpga=0x%llx len=0x%llx flags=0x%x pdb=%u\n", i,
               f.rn[i].op, (unsigned long long)f.rn[i].va, (unsigned long long)f.rn[i].gpga,
               (unsigned long long)f.rn[i].len, f.rn[i].flags, f.rn[i].pdb_index);
}

/* the format doc's property 3, plus the ordering the diff's merge join needs */
static void validate(Fix &f)
{
    const char *why = NULL;
    /* §39(c): the REAL window, so every caller of validate() -- the racers
     * included -- asserts that no run escapes the store. */
    int rc = kf_validate_report(&f.hdr, f.pe.data(), f.rn.data(), f.cfg.gpga_span, &why);
    CHECK_M(rc == 0, why);
    /* ★ The REPORT's order, per entry: page-size class ascending; within a
     * class every UNMAP (whole committed placements) before every MAP (pieces in
     * the gaps), each group VA-ascending. The UNMAP set and the MAP set never
     * overlap a KEPT placement, and a host applies all unmaps first. */
    for (uint32_t p = 0; p < f.hdr.pdb_count; p++) {
        for (uint32_t i = 1; i < f.pe[p].run_count; i++) {
            const KfMapRun &a = f.rn[f.pe[p].first_run + i - 1];
            const KfMapRun &b = f.rn[f.pe[p].first_run + i];
            uint32_t pa = (a.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
            uint32_t pb = (b.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
            bool ok = (pa < pb) ||
                      (pa == pb && a.op == KFWR_OP_UNMAP && b.op == KFWR_OP_MAP) ||
                      (pa == pb && a.op == b.op && a.va < b.va);
            CHECK_M(ok, "runs not in (class asc, UNMAPs then MAPs, va asc) order");
        }
        for (uint32_t i = 0; i < f.pe[p].run_count; i++) {
            const KfMapRun &r = f.rn[f.pe[p].first_run + i];
            CHECK_M(r.op == KFWR_OP_MAP || r.op == KFWR_OP_UNMAP, "a diff carries only MAP and UNMAP");
            CHECK_M(r.op == KFWR_OP_UNMAP || !(r.flags & KFWR_RF_HELD), "a MAP never carries HELD");
        }
    }
    if (f.hdr.magic == KFWR_MAGIC) CHECK_M(f.hdr.flags & KFWR_HF_DIFF, "every report is a DIFF");
}

struct ER { uint64_t va, gpga, len; uint32_t flags; uint32_t op; };

static void expect(Fix &f, const std::vector<ER> &want)
{
    validate(f);
    CHECK_EQ(f.hdr.run_count, want.size());
    if (f.hdr.run_count != want.size()) { dump(f); return; }
    for (size_t i = 0; i < want.size(); i++) {
        CHECK_EQ(f.rn[i].va, want[i].va);
        CHECK_EQ(f.rn[i].gpga, want[i].gpga);
        CHECK_EQ(f.rn[i].len, want[i].len);
        CHECK_EQ(f.rn[i].flags, want[i].flags);
        CHECK_EQ(f.rn[i].op, want[i].op);
    }
    if (g_fails_here) dump(f);
}

#define F4K   ((uint32_t)(KFWR_PS_4K   << KFWR_RF_PS_SHIFT))
#define F64K  ((uint32_t)(KFWR_PS_64K  << KFWR_RF_PS_SHIFT))
#define F2M   ((uint32_t)(KFWR_PS_2M   << KFWR_RF_PS_SHIFT))
#define F512M ((uint32_t)(KFWR_PS_512M << KFWR_RF_PS_SHIFT))

/* A base VA that exercises every level's index being non-zero. */
static const uint64_t VBASE = ((uint64_t)1 << 47) | ((uint64_t)3 << 38) | ((uint64_t)7 << 29) | ((uint64_t)5 << 21);

/* ══ CORRECTNESS ═════════════════════════════════════════════════════════════ */

/* =====================================================================================
 * w768 — AN INVALIDATE MAY ONLY DISTURB THE ADDRESS SPACES IT NAMES
 * =====================================================================================
 *
 * > Owner, 2026-09-19: *"during invalidate you may only create undefined behaviour on the
 * > vas that change, never on the vas the invalidate didn't touch. Invalidate is not freeze
 * > the guest, I can now reconstruct VA."*
 *
 * ★★★★★ The guest is NOT stopped while we answer an invalidate. Another thread may be
 * running against another address space the whole time, and that address space has no reason
 * to notice. ⇒ A refresh driven by a change in VAS A must emit NOTHING for VAS B.
 *
 * ⊘ This is not a perf claim dressed as correctness. `[measured w766]` the host's
 * `refresh_page_tables` does `for pid in pids` — it sweeps EVERY process's tables on an
 * invalidate that names exactly ONE pdb (`all_pdb=0`, `distinct_pdbs=6`). Doing more than the
 * invalidate asked is how an untouched VAS acquires an op it never earned.
 *
 * ⚠ A test that only asserts "B emitted nothing" passes trivially if the walker emits nothing
 * for anybody, so the KNOWN-POSITIVE below changes B and requires that it DOES emit. Without
 * it this is a test of an empty report.
 */
static void t_invalidate_only_disturbs_the_vas_it_names(void)
{
    Fix f(16u << 20, cfg_default());
    Tree a(f.g), b(f.g);
    /* ⊘ The SAME VA in both spaces, at different memory: if the walker ever confused the two
     * this fixture reports it as a wrong gpga rather than as a silent absence. */
    a.map4k(VBASE, 0x300000ull);
    b.map4k(VBASE, 0x500000ull);
    f.upload();
    CHECK_EQ(f.refresh({a.root, b.root}), 0);
    CHECK_EQ(f.hdr.pdb_count, 2);
    CHECK_EQ(f.hdr.run_count, 2);
    f.ack();

    /* ---- the invalidate: A changes, B is not touched at all. */
    a.map4k(VBASE + 4096ull, 0x301000ull);
    f.upload();
    CHECK_EQ(f.refresh({a.root, b.root}), 0);
    f.ack();

    int b_runs = -1, a_runs = -1;
    for (uint32_t i = 0; i < f.hdr.pdb_count; i++) {
        if (f.pe[i].pdb == b.root) b_runs = (int)f.pe[i].run_count;
        if (f.pe[i].pdb == a.root) a_runs = (int)f.pe[i].run_count;
    }
    CHECK(b_runs >= 0);
    CHECK(a_runs >= 0);
    if (b_runs != 0) dump(f);
    CHECK_EQ(b_runs, 0);   /* ★ the whole point: B changed nothing, so B owes nothing */
    CHECK(a_runs > 0);     /* ⊘ and A must still report, or the delta is just broken */

    /* ---- THE KNOWN-POSITIVE. Change B and it MUST speak, or the assertion above is
     * satisfied by a walker that has stopped emitting. */
    b.map4k(VBASE + 4096ull, 0x501000ull);
    f.upload();
    CHECK_EQ(f.refresh({a.root, b.root}), 0);
    int b_runs2 = -1;
    for (uint32_t i = 0; i < f.hdr.pdb_count; i++)
        if (f.pe[i].pdb == b.root) b_runs2 = (int)f.pe[i].run_count;
    if (b_runs2 <= 0) dump(f);
    CHECK(b_runs2 > 0);
}

static void t_single_4k(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0x300000ull, 4096ull, F4K, KFWR_OP_MAP}});
    CHECK(f.hdr.flags & KFWR_HF_DIFF);
    CHECK_EQ(f.hdr.refusals, 0);
    CHECK_EQ(f.hdr.pdb_count, 1);
    CHECK_EQ(f.pe[0].pdb, t.root);
}

static void t_large_pages(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map64k(VBASE, 0x400000ull);
    t.map2m(VBASE + (2ull << 21), 0x600000ull);
    t.map512m(VBASE + (1ull << 29), 0x20000000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    /* ⚠ A leaf is reported at ITS OWN LEVEL'S base VA, not at the VA the builder
     * was handed: a 512 MiB page lives in a PD1 slot, so its VA is 512 MiB
     * aligned however the caller spelled it. */
    const uint64_t va512 = (VBASE + (1ull << 29)) & ~((1ull << 29) - 1ull);
    expect(f, {
        {VBASE,                    0x400000ull,   64ull << 10,  F64K,  KFWR_OP_MAP},
        {VBASE + (2ull << 21),     0x600000ull,   2ull << 20,   F2M,   KFWR_OP_MAP},
        {va512,                    0x20000000ull, 512ull << 20, F512M, KFWR_OP_MAP},
    });
}

static void t_coalesce_one_run(void)
{
    /* 1024 consecutive 4 KiB pages = 4 MiB, spanning TWO PT_SMALL tables and two
     * PD0 slots. One run. */
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 1024; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 4096ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0x800000ull, 1024ull * 4096ull, F4K, KFWR_OP_MAP}});
}

static void t_coalesce_split_gpga(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 64; i++) {
        uint64_t gp = 0x800000ull + (uint64_t)i * 4096ull;
        if (i >= 32) gp += 0x100000ull;             /* a deliberate discontinuity */
        t.map4k(VBASE + (uint64_t)i * 4096ull, gp);
    }
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,                   0x800000ull,            32ull * 4096ull, F4K, KFWR_OP_MAP},
        {VBASE + 32ull * 4096ull, 0x900000ull + 0x20000ull, 32ull * 4096ull, F4K, KFWR_OP_MAP},
    });
}

static void t_coalesce_split_flags(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 64; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 4096ull,
                AP_PTE_VID, (i >= 32 && i < 48) ? PTE_READ_ONLY : 0);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,                   0x800000ull,  32ull * 4096ull, F4K, KFWR_OP_MAP},
        {VBASE + 32ull * 4096ull, 0x820000ull,  16ull * 4096ull, F4K | KFWR_RF_READ_ONLY, KFWR_OP_MAP},
        {VBASE + 48ull * 4096ull, 0x830000ull,  16ull * 4096ull, F4K, KFWR_OP_MAP},
    });
}

static void t_coalesce_never_across_page_size(void)
{
    /* VA- and GPGA-contiguous, but 4 KiB then 64 KiB. Page size is part of flags,
     * so they must NOT merge. */
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 16; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 4096ull);
    t.map64k(VBASE + 0x10000ull, 0x810000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,             0x800000ull, 64ull << 10, F4K,  KFWR_OP_MAP},
        {VBASE + 0x10000ull, 0x810000ull, 64ull << 10, F64K, KFWR_OP_MAP},
    });
}

static void t_sparse_and_invalid_skipped(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    uint64_t pts = t.pts(VBASE);
    f.g.u64(pts + 1 * 8) = kfb_sparse_pte();      /* sparse: VOL set, VALID clear */
    f.g.u64(pts + 2 * 8) = 0ull;                  /* invalid                      */
    f.g.u64(pts + 3 * 8) = kfb_pte(0x310000ull);  /* a real one after the holes    */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,            0x300000ull, 4096ull, F4K, KFWR_OP_MAP},
        {VBASE + 3 * 4096ull, 0x310000ull, 4096ull, F4K, KFWR_OP_MAP},
    });
    /* ★ SPARSE and INVALID are different facts and the report now says which.
     * One slot was DECLARED empty; the other was simply never written. */
    CHECK_EQ(f.hdr.sparse_slots, 1);
}

static void t_flags_decoded(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE + 0 * 4096ull, 0x300000ull, AP_PTE_VID,  PTE_READ_ONLY);
    t.map4k(VBASE + 1 * 4096ull, 0x310000ull, AP_PTE_VID,  PTE_ATOMIC_DISABLE);
    t.map4k(VBASE + 2 * 4096ull, 0x320000ull, AP_PTE_VID,  PTE_VOL);
    t.map4k(VBASE + 3 * 4096ull, 0x330000ull, AP_PTE_VID,  PTE_PRIVILEGE);
    t.map4k(VBASE + 4 * 4096ull, 0x340000ull, AP_PTE_SCOH, 0);
    t.map4k(VBASE + 5 * 4096ull, 0x350000ull, AP_PTE_SNC,  0);
    t.map4k(VBASE + 6 * 4096ull, 0x360000ull, AP_PTE_PEER, 0);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE + 0 * 4096ull, 0x300000ull, 4096ull, F4K | KFWR_RF_READ_ONLY,      KFWR_OP_MAP},
        {VBASE + 1 * 4096ull, 0x310000ull, 4096ull, F4K | KFWR_RF_ATOMIC_DISABLE, KFWR_OP_MAP},
        {VBASE + 2 * 4096ull, 0x320000ull, 4096ull, F4K | KFWR_RF_VOLATILE,       KFWR_OP_MAP},
        {VBASE + 3 * 4096ull, 0x330000ull, 4096ull, F4K | KFWR_RF_PRIVILEGE,      KFWR_OP_MAP},
        {VBASE + 4 * 4096ull, 0x340000ull, 4096ull, F4K | KFWR_AP_SYSCOH,         KFWR_OP_MAP},
        {VBASE + 5 * 4096ull, 0x350000ull, 4096ull, F4K | KFWR_AP_SYSNONCOH,      KFWR_OP_MAP},
    });
    /* w825 A1: a PEER leaf is REFUSED at the emit chokepoint — this device backs no peer
     * aperture — and the refusal is loud, never a silent drop. */
    CHECK_M(f.hdr.refuse_mask & KFWR_R_LEAF_OOB, "the peer leaf must be refused, loudly");
}

static void t_dual_pde_both_halves(void)
{
    /* Both sub-tables of one PD0 slot populated, in DIFFERENT 64 KiB slots. Both must come
     * back, in VA order.
     * ⊘ v3-mapfix: this test used to put the 64 KiB page and the 4 KiB pages in the SAME
     * slot and demand both — encoding the defect t_valid_big_pte_hides_stale_4k refutes (a
     * valid big PTE owns its slot; the MMU never reads the 4 KiB PTEs under it). */
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map64k(VBASE + 0x10000ull, 0x400000ull);
    t.map4k(VBASE, 0x500000ull);
    t.map4k(VBASE + 0x1000ull, 0x501000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,             0x500000ull, 2ull * 4096ull, F4K, KFWR_OP_MAP},
        {VBASE + 0x10000ull, 0x400000ull, 64ull << 10, F64K, KFWR_OP_MAP},
    });
}

static void t_valid_big_pte_hides_stale_4k(void)
{
    /* ★ v3-mapfix — UVM's 4 KiB -> 64 KiB merge (uvm_va_block.c:6444-6512): UNMAPPED big
     * PTE, invalidate, then a VALID big PTE, with the sixteen 4 KiB PTEs under it never
     * rewritten. The MMU uses the big PTE and never reads them (mmu_trace.c: sublevel 0
     * first, done on the first valid translation). `[measured 670bd310 UnifiedMemoryStreams]`
     * reporting both put two host mappings over one VA: the host refused the second
     * (NV_ERR_INVALID_ARGUMENT, gpu_vaspace.c:4761). Slot 1 is the control: a 4 KiB leaf
     * under an INVALID (zero) big PTE is live and must survive. */
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 16u; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x300000ull + (uint64_t)i * 4096ull);
    t.map4k(VBASE + 0x10000ull, 0x310000ull);
    t.map64k(VBASE, 0x600000ull);               /* the merge's last write: slot 0 VALID big */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    /* Exactly two leaves, whatever the per-class order: the big page, and the control. */
    CHECK_EQ(f.hdr.run_count, 2u);
    bool big = false, ctl = false;
    for (uint32_t i = 0; i < f.hdr.run_count && i < 2u; i++) {
        const KfMapRun &r = f.rn[i];
        CHECK_M(r.op == KFWR_OP_MAP, "every leaf is a MAP");
        if (r.va == VBASE && r.gpga == 0x600000ull && r.len == (64ull << 10) && r.flags == F64K) big = true;
        if (r.va == VBASE + 0x10000ull && r.gpga == 0x310000ull && r.len == 4096ull && r.flags == F4K) ctl = true;
    }
    CHECK_M(big, "the VALID big PTE must be reported");
    CHECK_M(ctl, "the control (4 KiB under an INVALID big PTE) must survive");
    if (g_fails_here) dump(f);
}

/* ★★★ v3-roperm × v3-mapfix — A BIG PTE THAT OWNS ITS SLOT CARRIES *ITS* PERMISSIONS.
 * UVM's read duplication maps the GPU duplicate read-only at whatever page size the block
 * uses, and the merge leaves stale RW 4 KiB PTEs under a VALID big PTE. The run the host
 * places is the big leaf's, with the big leaf's READ_ONLY — never the stale smalls' RW —
 * and revoking write on the big PTE in place (same backing) re-maps it read-only. */
static void t_owning_big_pte_carries_its_permissions(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 16u; i++)                          /* stale RW smalls */
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x300000ull + (uint64_t)i * 4096ull);
    t.map64k(VBASE, 0x600000ull, AP_PTE_VID, PTE_READ_ONLY);    /* the owning big PTE, RO */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0x600000ull, 64ull << 10, F64K | KFWR_RF_READ_ONLY, KFWR_OP_MAP}});
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK_EQ(f.hdr.run_count, 0);
    t.map64k(VBASE, 0x600000ull);                               /* collapse: write re-granted */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0x600000ull, 64ull << 10, F64K | KFWR_RF_READ_ONLY, KFWR_OP_UNMAP},
               {VBASE, 0x600000ull, 64ull << 10, F64K, KFWR_OP_MAP}});
    f.ack();
    t.map64k(VBASE, 0x600000ull, AP_PTE_VID, PTE_READ_ONLY);    /* in-place revoke again */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0x600000ull, 64ull << 10, F64K, KFWR_OP_UNMAP},
               {VBASE, 0x600000ull, 64ull << 10, F64K | KFWR_RF_READ_ONLY, KFWR_OP_MAP}});
}

static void t_dual_pde_mixed_slots_live_w826(void)
{
    /* `[measured w826 ct10]` the live GA106 shape that lost two 4 KiB leaves: the small
     * table exists first (slots 0 and 1 mapped), THEN RM adds a big table at a 256-byte
     * aligned address whose bit 9 is set (0x11300 -> dual low word 0x1132) and maps slot 2
     * as a 64 KiB page. Big PTEs 0 and 1 are ZERO, so hardware uses the small PTEs there. */
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x490000ull);
    t.map4k(VBASE + 0x10000ull, 0x4a0000ull);
    f.g.alloc(0x300, 256);                      /* push the big table to ...300 */
    t.map64k(VBASE + 0x20000ull, 0x4b0000ull);
    CHECK_M((t.ptb(VBASE, false) & 0x200ull) != 0ull, "fixture: big table must have bit 9 set");
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,             0x490000ull, 4096ull, F4K, KFWR_OP_MAP},
        {VBASE + 0x10000ull, 0x4a0000ull, 4096ull, F4K, KFWR_OP_MAP},
        {VBASE + 0x20000ull, 0x4b0000ull, 64ull << 10, F64K, KFWR_OP_MAP},
    });
}

static void t_multiple_pdbs(void)
{
    Fix f(16u << 20, cfg_default());
    Tree a(f.g), b(f.g);
    a.map4k(VBASE, 0x800000ull);
    b.map4k(VBASE + 0x2000ull, 0x900000ull);
    b.map4k(VBASE + 0x3000ull, 0x901000ull);
    f.upload();
    std::vector<uint64_t> roots = { a.root, b.root };
    if (roots[0] > roots[1]) std::swap(roots[0], roots[1]);
    CHECK_EQ(f.refresh(roots), 0);
    validate(f);
    CHECK_EQ(f.hdr.pdb_count, 2);
    CHECK_EQ(f.hdr.run_count, 2);
    for (uint32_t i = 0; i < f.hdr.pdb_count; i++) {
        CHECK_EQ(f.pe[i].pdb, roots[i]);
        uint32_t want = (roots[i] == a.root) ? 1u : 1u;
        CHECK_EQ(f.pe[i].run_count, want);
        for (uint32_t k = 0; k < f.pe[i].run_count; k++)
            CHECK_EQ(f.rn[f.pe[i].first_run + k].pdb_index, i);
    }
    if (g_fails_here) dump(f);
}

/* ══ CHANGE DETECTION ════════════════════════════════════════════════════════ */

/* ══ THE DIFF, COMMITTED ON ACK ═══════════════════════════════════════════════
 *
 * ★★★★★ Owner design + COMMIT-ON-ACK ruling (2026-09-25); the spec is
 * crates/kf-cuda/src/diffmodel.rs. Per VA-space object the kernel holds a SLOT
 * of the placements the host CONFIRMED it made. Every refresh reports, per
 * walked entry, the diff of the guest's live tables against that slot:
 *   - UNMAP of every WHOLE placement the walk no longer backs byte for byte
 *     (same host ground truth: vidmem vs guest RAM; same linear offset);
 *   - MAP of every piece of the walk in a gap between KEPT placements.
 * The host answers one verdict per run (kf_ack); the NEXT refresh's first
 * kernel commits exactly the acknowledged runs. A refused run stays a
 * difference. No verdict ⇒ nothing is committed ⇒ the same diff again.
 *
 * ⊘ These replace the old delta/, roundtrip/ and scope/ cases, which tested the
 * superseded walk-vs-previous-walk snapshot (V3_BUILD.md's amended rule).
 */

static const uint64_t PG = 4096ull;
static const uint64_t GB0 = 0x800000ull;
#define VP(i) (VBASE + (uint64_t)(i) * PG)
#define GP(i) (GB0 + (uint64_t)(i) * PG)

/* The runs of report entry `e`. */
static std::vector<KfMapRun> entry_runs(const Fix &f, uint32_t e)
{
    std::vector<KfMapRun> v;
    if (e >= f.hdr.pdb_count) return v;
    for (uint32_t i = 0; i < f.pe[e].run_count; i++) v.push_back(f.rn[f.pe[e].first_run + i]);
    return v;
}

/* Walk once and acknowledge every run: the slot now holds the walk. */
static void settle(Fix &f, Tree &t, uint32_t want_runs)
{
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK(f.hdr.flags & KFWR_HF_DIFF);
    CHECK_EQ(f.hdr.run_count, want_runs);
    for (uint32_t i = 0; i < f.hdr.run_count; i++) CHECK_EQ(f.rn[i].op, KFWR_OP_MAP);
    f.ack();
}

static void t_diff_first_walk_is_all_maps(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 8; i++) t.map4k(VP(i), GP(i));
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, 8 * PG, F4K, KFWR_OP_MAP}});
    CHECK(f.hdr.flags & KFWR_HF_DIFF);
    /* No verdict ⇒ nothing committed ⇒ the same diff again. */
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, 8 * PG, F4K, KFWR_OP_MAP}});
}

static void t_diff_acked_is_quiet(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 8; i++) t.map4k(VP(i), GP(i));
    settle(f, t, 1);
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK_EQ(f.hdr.run_count, 0);
    CHECK_EQ(f.hdr.pdb_count, 1);
    CHECK_EQ(f.hdr.acked_generation, f.hdr.generation - 1);
    if (g_fails_here) dump(f);
}

static void t_diff_add(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    settle(f, t, 1);
    t.map4k(VBASE + (1ull << 21), 0x900000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + (1ull << 21), 0x900000ull, PG, F4K, KFWR_OP_MAP}});
}

static void t_diff_delete(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    t.map4k(VBASE + (1ull << 21), 0x900000ull);
    settle(f, t, 2);
    t.unmap4k(VBASE + (1ull << 21));
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + (1ull << 21), 0x900000ull, PG, F4K, KFWR_OP_UNMAP}});
}

static void t_diff_edit_gpga(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    settle(f, t, 1);
    t.map4k(VBASE, 0xA00000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K, KFWR_OP_UNMAP}, {VBASE, 0xA00000ull, PG, F4K, KFWR_OP_MAP}});
}

/* ★★★★★ v3-roperm — A PERMISSION EDIT OVER THE SAME BACKING IS A CHANGE.
 * ⊘ This case was "flags_only_edit_is_quiet": READ_ONLY was not part of the diff key, so a
 * guest RW→RO downgrade (UVM read duplication's in-place revoke, uvm_va_block.c:9010-9060)
 * owed the host nothing, the host twin stayed READ-WRITE, and a GPU write to the duplicate
 * landed silently in a stale copy (V3_UVM_DEMAND_PAGING.md §6). The host map now carries
 * READ_ONLY / ATOMIC_DISABLE / VOLATILE (KFWR_RF_HOST_PERM), so each is part of a placement:
 * the RW placement is UNMAPPED and the same bytes MAPPED read-only. PRIVILEGE is not carried
 * (no unprivileged host verb places it) and stays quiet. */
static void t_diff_permission_edit_remaps(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    settle(f, t, 1);
    t.map4k(VBASE, GB0, AP_PTE_VID, PTE_READ_ONLY);            /* RW -> RO, same page */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K, KFWR_OP_UNMAP},
               {VBASE, GB0, PG, F4K | KFWR_RF_READ_ONLY, KFWR_OP_MAP}});
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK_EQ(f.hdr.run_count, 0);                              /* landed: quiet */
    t.map4k(VBASE, GB0, AP_PTE_VID, PTE_READ_ONLY | PTE_ATOMIC_DISABLE);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K | KFWR_RF_READ_ONLY, KFWR_OP_UNMAP},
               {VBASE, GB0, PG, F4K | KFWR_RF_READ_ONLY | KFWR_RF_ATOMIC_DISABLE, KFWR_OP_MAP}});
    f.ack();
    t.map4k(VBASE, GB0, AP_PTE_VID, PTE_READ_ONLY | PTE_ATOMIC_DISABLE | PTE_PRIVILEGE);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK_EQ(f.hdr.run_count, 0);                              /* PRIVILEGE: not placed */
    t.map4k(VBASE, GB0);                                       /* RO -> RW: the upgrade */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K | KFWR_RF_READ_ONLY | KFWR_RF_ATOMIC_DISABLE, KFWR_OP_UNMAP},
               {VBASE, GB0, PG, F4K, KFWR_OP_MAP}});
    f.ack();
    /* The ground truth IS part of it: the same offset in guest RAM is a new map. */
    t.map4k(VBASE, GB0, AP_PTE_SCOH);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K, KFWR_OP_UNMAP}, {VBASE, GB0, PG, F4K | KFWR_AP_SYSCOH, KFWR_OP_MAP}});
}

/* Grow: ONE map of the new page, the old placement kept. Shrink: the placements
 * the walk no longer backs WHOLE are unmapped, and what remains is mapped. */
static void t_diff_grow_then_shrink(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 4; i++) t.map4k(VP(i), GP(i));
    settle(f, t, 1);
    t.map4k(VP(4), GP(4));
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VP(4), GP(4), PG, F4K, KFWR_OP_MAP}});
    f.ack();                            /* placements: [0,4) and [4,5) */
    t.unmap4k(VP(4));
    t.unmap4k(VP(3));
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VP(0), GP(0), 4 * PG, F4K, KFWR_OP_UNMAP},
               {VP(4), GP(4), PG, F4K, KFWR_OP_UNMAP},
               {VP(0), GP(0), 3 * PG, F4K, KFWR_OP_MAP}});
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_EQ(f.hdr.run_count, 0);
}

static void t_diff_failed_map_is_retried(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    t.map4k(VBASE + (1ull << 21), 0x900000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_EQ(f.hdr.run_count, 2);
    f.ack_codes({KFWR_ACK_APPLIED, KFWR_ACK_FAILED});
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + (1ull << 21), 0x900000ull, PG, F4K, KFWR_OP_MAP}});
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_EQ(f.hdr.run_count, 0);
}

static void t_diff_failed_unmap_is_retried(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    t.map4k(VBASE + (1ull << 21), 0x900000ull);
    settle(f, t, 2);
    t.unmap4k(VBASE + (1ull << 21));
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + (1ull << 21), 0x900000ull, PG, F4K, KFWR_OP_UNMAP}});
    f.ack_codes({KFWR_ACK_FAILED});
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + (1ull << 21), 0x900000ull, PG, F4K, KFWR_OP_UNMAP}});
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_EQ(f.hdr.run_count, 0);
}

static void t_diff_held_is_committed_held(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    f.ack_codes({KFWR_ACK_HELD});
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_EQ(f.hdr.run_count, 0);         /* held and still backed: kept */
    t.unmap4k(VBASE);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K | KFWR_RF_HELD, KFWR_OP_UNMAP}});
}

static void t_diff_stale_or_absent_verdict_commits_nothing(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    uint64_t g1 = f.hdr.generation;
    CHECK_EQ(f.refresh({t.root}), 0);     /* no verdict: gen g1+1, same diff */
    expect(f, {{VBASE, GB0, PG, F4K, KFWR_OP_MAP}});
    std::vector<uint8_t> c(2, KFWR_ACK_APPLIED);
    kf_ack(f.w, g1, c.data(), 1, NULL, 0); /* names an OLDER report */
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K, KFWR_OP_MAP}});
    kf_ack(f.w, f.hdr.generation, c.data(), 2, NULL, 0); /* the wrong run count */
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K, KFWR_OP_MAP}});
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_EQ(f.hdr.run_count, 0);
}

/* A walk that ran out of slice is TRUNCATED; the host refuses it, and even an
 * ack of it commits nothing. */
static void t_diff_truncated_is_never_committed(void)
{
    KfWalkCfg c = cfg_default();
    c.runs_per_pdb = 64;
    Fix f(16u << 20, c);
    Tree t(f.g);
    for (uint32_t i = 0; i < 400; i++) t.map4k(VP(i), GB0 + (uint64_t)i * 8192ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK(f.hdr.flags & KFWR_HF_TRUNCATED);
    uint32_t n = f.hdr.run_count;
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK(f.hdr.flags & KFWR_HF_TRUNCATED);
    CHECK_EQ(f.hdr.run_count, n);
    for (uint32_t i = 0; i < f.hdr.run_count; i++) CHECK_EQ(f.rn[i].op, KFWR_OP_MAP);
}

/* ★ The slot is the OBJECT's: its new root is diffed against what was placed
 * under the old one. Two objects may also share one root (each its own slot). */
static void t_diff_root_move_keeps_the_slot(void)
{
    Fix f(16u << 20, cfg_default());
    Tree a(f.g), b(f.g);
    a.map4k(VBASE, GB0);
    a.map4k(VBASE + (1ull << 21), 0x900000ull);
    b.map4k(VBASE, GB0);
    b.map4k(VBASE + (2ull << 21), 0xA00000ull);
    settle(f, a, 2);
    CHECK_EQ(f.refresh({b.root}), 0);
    expect(f, {{VBASE + (1ull << 21), 0x900000ull, PG, F4K, KFWR_OP_UNMAP},
               {VBASE + (2ull << 21), 0xA00000ull, PG, F4K, KFWR_OP_MAP}});
    CHECK_EQ(f.pe[0].pdb, b.root);
    CHECK_EQ(f.pe[0].reserved, 0);
    /* Two objects, one root: slot 1 has placed nothing, so it owes the whole walk. */
    f.ack();
    CHECK_EQ(f.refresh_slots({b.root, b.root}, {0, 1}), 0);
    validate(f);
    CHECK_EQ(f.pe[0].run_count, 0);
    CHECK_EQ(f.pe[1].run_count, 2);
    CHECK_EQ(f.pe[1].reserved, 1);
}

static void t_diff_reset_empties_the_slot(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    /* The verdict AND a release of the same slot: the release wins (never commit
     * into a slot whose object is gone). */
    std::vector<uint8_t> c(f.hdr.run_count, KFWR_ACK_APPLIED);
    uint32_t r0 = 0;
    kf_ack(f.w, f.hdr.generation, c.data(), (uint32_t)c.size(), &r0, 1);
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K, KFWR_OP_MAP}});
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_EQ(f.hdr.run_count, 0);
    f.ack_codes({}, {0});                 /* release a populated slot */
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, GB0, PG, F4K, KFWR_OP_MAP}});
}

/* A commit may never overflow a slot: a diff whose maps could, withholds them
 * (PARTIAL) and emits its unmaps; with nothing to retire it is OVERFLOW. */
static void t_diff_partial_and_overflow(void)
{
    KfWalkCfg c = cfg_default();
    c.runs_per_pdb = 2;
    Fix f(16u << 20, c);
    Tree t(f.g);
    t.map4k(VBASE, GB0);
    t.map4k(VBASE + (1ull << 21), 0x900000ull);
    settle(f, t, 2);
    t.map4k(VBASE, 0xA00000ull);
    t.map4k(VBASE + (1ull << 21), 0xB00000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK(f.pe[0].vas_flags & KFWR_V_PARTIAL);
    CHECK_EQ(f.hdr.run_count, 2);
    for (uint32_t i = 0; i < f.hdr.run_count; i++) CHECK_EQ(f.rn[i].op, KFWR_OP_UNMAP);
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK(!(f.pe[0].vas_flags & KFWR_V_PARTIAL));
    expect(f, {{VBASE, 0xA00000ull, PG, F4K, KFWR_OP_MAP}, {VBASE + (1ull << 21), 0xB00000ull, PG, F4K, KFWR_OP_MAP}});

    Fix g(16u << 20, c);
    Tree u(g.g);
    u.map4k(VP(0), GP(0));
    settle(g, u, 1);
    u.map4k(VP(1), GP(1));               /* grown: a second placement */
    g.upload();
    CHECK_EQ(g.refresh({u.root}), 0);
    CHECK_EQ(g.hdr.run_count, 1);
    g.ack();
    u.map4k(VP(2), GP(2));               /* one walk run, two kept, one piece: 3 > 2 */
    g.upload();
    CHECK_EQ(g.refresh({u.root}), 0);
    CHECK(g.pe[0].vas_flags & KFWR_V_OVERFLOW);
    CHECK_EQ(g.hdr.run_count, 0);
    CHECK(g.hdr.refuse_mask & KFWR_R_RUN_CAP);
    if (g_fails_here) { dump(f); dump(g); }
}

static void t_diff_move_page_table_is_quiet(void)
{
    /* Repoint the parent PDE at a byte-identical COPY of the leaf table. */
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 8; i++) t.map4k(VP(i), GP(i));
    settle(f, t, 1);
    uint64_t old_pts = t.pts(VBASE);
    uint64_t nw = f.g.alloc(4096, 4096);
    memcpy(f.g.mem.data() + nw, f.g.mem.data() + old_pts, 4096);
    memset(f.g.mem.data() + old_pts, 0, 4096);
    f.g.u64(t.pd0(VBASE) + (uint64_t)vi0(VBASE) * 16 + 8) = kfb_pde(nw);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

/* One run spanning two leaf tables (the task-boundary join): the full walk is
 * ONE run. `make check-coalesce-negative` breaks the join and requires this to fail. */
static void t_one_run_across_page_tables(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    uint64_t base = (VBASE | ((1ull << 21) - 1ull)) + 1ull - 4 * PG;   /* 4 pages below a 2 MiB line */
    for (uint32_t i = 0; i < 8; i++) t.map4k(base + i * PG, GB0 + i * PG);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{base, GB0, 8 * PG, F4K, KFWR_OP_MAP}});
}

/* ══ THE ROUND TRIP, AGAINST A PORT OF THE RUST MODEL ═════════════════════════
 * Entry 0 (slot 0) is diffed and acknowledged with random verdicts; entry 1
 * (slot 1) walks the SAME root and is never acknowledged, so its report IS the
 * full walk — the model's input. Every slot-0 report must equal the model's diff
 * of that walk against the model's slot; the model then commits the same
 * verdict. At the end, acknowledging everything must leave the slot expressing
 * exactly the walk (closure) and the next diff empty.
 * ⚠ The host rule is kept: a map over a refused unmap is never acknowledged. */
typedef std::vector<KfMapRun> Runs;
struct MSlot { Runs cls[4]; };

static uint32_t m_cls(uint32_t f) { return (f >> KFWR_RF_PS_SHIFT) & 3u; }
/* Mirrors kf_hkey / diffmodel::host_key: ground truth, KIND, and the carried permissions. */
static uint32_t m_key(uint32_t f)
{
    const uint32_t a = f & 7u;
    return (a == 0u ? 0u : (a == 2u || a == 3u) ? 1u : 2u)
         | (((f >> KFWR_RF_KIND_SHIFT) & KFWR_RF_KIND_MASK) << 2)
         | (((f & KFWR_RF_HOST_PERM) >> 3) << 10);
}

static bool m_covered(const KfMapRun &p, const Runs &w)
{
    uint64_t end = p.va + p.len;
    size_t i = 0;
    while (i < w.size() && w[i].va <= p.va) i++;
    if (i == 0) return false;
    i--;
    uint64_t at = p.va;
    for (; i < w.size(); i++) {
        const KfMapRun &r = w[i];
        uint64_t rend = r.va + r.len;
        if (!(r.va <= at && at < rend) || m_key(r.flags) != m_key(p.flags)) return false;
        if (r.gpga + (at - r.va) != p.gpga + (at - p.va)) return false;
        at = rend;
        if (at >= end) return true;
    }
    return false;
}

static Runs m_diff(const MSlot &s, const Runs &walk)
{
    Runs out;
    for (uint32_t c = 0; c < 4; c++) {
        Runs w;
        for (const KfMapRun &r : walk) if (m_cls(r.flags) == c) w.push_back(r);
        std::vector<const KfMapRun *> kept;
        Runs un;
        for (const KfMapRun &p : s.cls[c]) {
            if (m_covered(p, w)) kept.push_back(&p);
            else { KfMapRun u = p; u.op = KFWR_OP_UNMAP; un.push_back(u); }
        }
        out.insert(out.end(), un.begin(), un.end());
        for (size_t g = 0; g <= kept.size(); g++) {
            uint64_t lo = g ? kept[g - 1]->va + kept[g - 1]->len : 0ull;
            uint64_t hi = g < kept.size() ? kept[g]->va : ~0ull;
            if (lo >= hi) continue;
            for (const KfMapRun &r : w) {
                uint64_t s0 = std::max(r.va, lo), e0 = std::min(r.va + r.len, hi);
                if (s0 >= e0) continue;
                KfMapRun m = r;
                m.va = s0; m.gpga = r.gpga + (s0 - r.va); m.len = e0 - s0;
                m.flags &= ~KFWR_RF_HELD; m.op = KFWR_OP_MAP;
                out.push_back(m);
            }
        }
    }
    return out;
}

static MSlot m_commit(const MSlot &s, const Runs &runs, const std::vector<uint8_t> &codes)
{
    MSlot o;
    for (uint32_t c = 0; c < 4; c++) {
        std::map<uint64_t, KfMapRun> m;
        for (const KfMapRun &p : s.cls[c]) m[p.va] = p;
        for (size_t i = 0; i < runs.size(); i++) {
            if (m_cls(runs[i].flags) != c || codes[i] == KFWR_ACK_FAILED) continue;
            if (runs[i].op == KFWR_OP_UNMAP) m.erase(runs[i].va);
        }
        for (size_t i = 0; i < runs.size(); i++) {
            if (m_cls(runs[i].flags) != c || codes[i] == KFWR_ACK_FAILED || runs[i].op != KFWR_OP_MAP) continue;
            KfMapRun r = runs[i];
            r.flags = (r.flags & ~KFWR_RF_HELD) | (codes[i] == KFWR_ACK_HELD ? KFWR_RF_HELD : 0u);
            m[r.va] = r;
        }
        for (auto &kv : m) o.cls[c].push_back(kv.second);
    }
    return o;
}

/* (va, len, host key, gpga) with linear neighbours merged, per class. */
static std::vector<std::vector<uint64_t> > m_coverage(const Runs &v)
{
    std::vector<std::vector<uint64_t> > out;
    for (uint32_t c = 0; c < 4; c++) {
        std::map<uint64_t, KfMapRun> m;
        for (const KfMapRun &r : v) if (m_cls(r.flags) == c) m[r.va] = r;
        uint64_t cv = 0, cl = 0, cg = 0, ck = 9;
        for (auto &kv : m) {
            const KfMapRun &r = kv.second;
            if (cl && cv + cl == r.va && ck == m_key(r.flags) && cg + cl == r.gpga) { cl += r.len; continue; }
            if (cl) out.push_back({c, cv, cl, ck, cg});
            cv = r.va; cl = r.len; ck = m_key(r.flags); cg = r.gpga;
        }
        if (cl) out.push_back({c, cv, cl, ck, cg});
    }
    return out;
}

static bool same_run(const KfMapRun &a, const KfMapRun &b)
{
    return a.op == b.op && a.va == b.va && a.len == b.len && a.gpga == b.gpga && a.flags == b.flags;
}

static void rt_diff_stream(uint64_t seed, int steps, bool with_root_move)
{
    Fix f(32u << 20, cfg_default());
    Tree t(f.g), t2(f.g);
    uint64_t s = seed;
    auto rnd = [&](uint64_t n) { s = s * 6364136223846793005ull + 1442695040888963407ull; return (s >> 33) % n; };
    const uint64_t R4 = VBASE, R64 = VBASE + (4ull << 21);
    Tree *cur = &t;
    MSlot model;
    int mism = 0, compared = 0, failed = 0, held = 0;
    for (int step = 0; step < steps; step++) {
        Tree *ts[2] = { &t, &t2 };
        for (Tree *x : ts) {
            for (uint64_t k = 0, n = 1 + rnd(10); k < n; k++) {
                uint32_t op = (uint32_t)rnd(8);
                if (op < 5) {
                    uint64_t va = R4 + rnd(256) * PG;
                    if (op < 2) x->unmap4k(va);
                    else if (op == 2) {           /* grow a neighbour contiguously */
                        uint64_t g = GB0 + rnd(1024) * PG;
                        x->map4k(va, g); x->map4k(va + PG, g + PG);
                    } else x->map4k(va, GB0 + rnd(1024) * PG, rnd(4) == 0 ? AP_PTE_SCOH : AP_PTE_VID);
                } else {
                    uint64_t va = R64 + rnd(16) * 65536ull;
                    uint64_t &e = f.g.u64(x->ptb(va) + (uint64_t)vib(va) * 8);
                    if (op == 5) e = 0; else e = kfb_pte(GB0 + rnd(64) * 65536ull);
                }
            }
        }
        if (with_root_move && step == steps / 2) cur = &t2;
        f.upload();
        CHECK_EQ(f.refresh_slots({cur->root, cur->root}, {0, 1}), 0);
        validate(f);
        if (f.hdr.flags & (KFWR_HF_TRUNCATED | KFWR_HF_REFUSED)) { dump(f); CHECK(0); return; }
        Runs got = entry_runs(f, 0), walk = entry_runs(f, 1);
        Runs want = m_diff(model, walk);
        compared += (int)got.size();
        bool same = got.size() == want.size();
        for (size_t i = 0; same && i < got.size(); i++) same = same_run(got[i], want[i]);
        if (!same) {
            if (!mism++) {
                printf("      step %d: gpu %zu runs, model %zu\n", step, got.size(), want.size());
                for (size_t i = 0; i < std::max(got.size(), want.size()) && i < 12; i++)
                    printf("        [%zu] gpu op=%u va=0x%llx len=0x%llx g=0x%llx f=0x%x | model op=%u va=0x%llx len=0x%llx g=0x%llx f=0x%x\n", i,
                           i < got.size() ? got[i].op : 0, i < got.size() ? (unsigned long long)got[i].va : 0ull,
                           i < got.size() ? (unsigned long long)got[i].len : 0ull, i < got.size() ? (unsigned long long)got[i].gpga : 0ull,
                           i < got.size() ? got[i].flags : 0u,
                           i < want.size() ? want[i].op : 0, i < want.size() ? (unsigned long long)want[i].va : 0ull,
                           i < want.size() ? (unsigned long long)want[i].len : 0ull, i < want.size() ? (unsigned long long)want[i].gpga : 0ull,
                           i < want.size() ? want[i].flags : 0u);
            }
        }
        /* Verdicts over the GPU's runs (identical to the model's when `same`). */
        std::vector<uint8_t> c0(got.size());
        std::vector<std::pair<uint64_t, uint64_t> > refused;
        for (size_t i = 0; i < got.size(); i++) {
            const KfMapRun &x = got[i];
            bool blocked = false;
            for (auto &r : refused) if (x.op == KFWR_OP_MAP && x.va < r.second && r.first < x.va + x.len) blocked = true;
            uint8_t v = (blocked || rnd(5) == 0) ? KFWR_ACK_FAILED
                      : (x.op == KFWR_OP_MAP && rnd(25) == 0) ? KFWR_ACK_HELD : KFWR_ACK_APPLIED;
            if (x.op == KFWR_OP_UNMAP && v == KFWR_ACK_FAILED) refused.push_back({x.va, x.va + x.len});
            failed += v == KFWR_ACK_FAILED; held += v == KFWR_ACK_HELD;
            c0[i] = v;
        }
        std::vector<uint8_t> codes(f.hdr.run_count, KFWR_ACK_FAILED);   /* slot 1: never */
        for (size_t i = 0; i < c0.size(); i++) codes[f.pe[0].first_run + i] = c0[i];
        kf_ack(f.w, f.hdr.generation, codes.data(), (uint32_t)codes.size(), NULL, 0);
        model = m_commit(model, got, c0);
    }
    /* closure */
    bool quiet = false;
    Runs walk;
    for (int k = 0; k < 4 && !quiet; k++) {
        CHECK_EQ(f.refresh_slots({cur->root, cur->root}, {0, 1}), 0);
        Runs got = entry_runs(f, 0);
        walk = entry_runs(f, 1);
        if (got.empty()) { quiet = true; break; }
        std::vector<uint8_t> codes(f.hdr.run_count, KFWR_ACK_FAILED);
        for (uint32_t i = 0; i < f.pe[0].run_count; i++) codes[f.pe[0].first_run + i] = KFWR_ACK_APPLIED;
        kf_ack(f.w, f.hdr.generation, codes.data(), (uint32_t)codes.size(), NULL, 0);
        model = m_commit(model, got, std::vector<uint8_t>(got.size(), KFWR_ACK_APPLIED));
    }
    Runs flat;
    for (uint32_t c = 0; c < 4; c++) flat.insert(flat.end(), model.cls[c].begin(), model.cls[c].end());
    char m[160];
    snprintf(m, sizeof(m), "seed 0x%llx: %d mismatching reports of %d steps (%d runs compared)",
             (unsigned long long)seed, mism, steps, compared);
    CHECK_M(mism == 0, m);
    CHECK_M(quiet, "acknowledging everything must make the slot quiet");
    CHECK_M(m_coverage(flat) == m_coverage(walk), "closure: the settled slot must express exactly the walk");
    CHECK_M(failed > 5, "the stream must exercise refusals");
    printf("      seed 0x%llx: %d steps, %d runs compared, verdicts failed=%d held=%d\n",
           (unsigned long long)seed, steps, compared, failed, held);
}

static void t_roundtrip_diff_stream(void)
{
    const uint64_t seeds[] = { 0x1ull, 0xC0FFEEull, 0x5EED5EEDull };
    for (uint64_t sd : seeds) rt_diff_stream(sd, 150, false);
}

static void t_roundtrip_diff_root_move(void)
{
    rt_diff_stream(0xB0B0ull, 150, true);
}


/* ══ HOSTILE ═════════════════════════════════════════════════════════════════ */

/* What must hold for EVERY hostile input, however malformed. */
static void hostile_invariants(Fix &f)
{
    validate(f);
    CHECK_M(!(f.hdr.refuse_mask & KFWR_R_TOO_DEEP),
            "TOO_DEEP must be unreachable: the depth bound is structural, not detected");
}

static void t_hostile_self_cycle(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    uint64_t pd2 = t.pd2(VBASE);
    f.g.u64(pd2 + (uint64_t)vi2(VBASE) * 8) = kfb_pde(pd2);   /* points at its own page */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    if (g_fails_here) dump(f);
}

static void t_hostile_two_cycle(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    uint64_t pd2 = t.pd2(VBASE), pd1 = t.pd1(VBASE);
    f.g.u64(pd1 + (uint64_t)vi1(VBASE) * 8) = kfb_pde(pd2);   /* PD1 -> PD2's page */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    if (g_fails_here) dump(f);
}

static void t_hostile_deep_cycle(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    uint64_t pd0 = t.pd0(VBASE);
    /* the SMALL half of a dual PDE points back at the ROOT page */
    f.g.u64(pd0 + (uint64_t)vi0(VBASE) * 16 + 8) = kfb_pde(t.root);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    if (g_fails_here) dump(f);
}

static void t_hostile_ptr_past_end(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    f.g.u64(t.root + (uint64_t)vi3(VBASE) * 8) = kfb_pde((uint64_t)f.g.size() + (16u << 20));
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_OOB, "a pointer past the end must refuse as OOB");
    CHECK_EQ(f.hdr.refuse_mask & ~(uint32_t)KFWR_R_OOB, 0);
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_hostile_table_straddles_end(void)
{
    /* A buffer whose length is NOT a whole number of 4 KiB pages, so a table can
     * start aligned and still run off the end. Alignment passes; extent must not. */
    Fix f((8u << 20) + 2048u, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    f.g.u64(t.root + (uint64_t)vi3(VBASE) * 8) = kfb_pde((uint64_t)(8u << 20));
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_OOB, "a straddling table must refuse as OOB");
    CHECK_EQ(f.hdr.refuse_mask & (uint32_t)KFWR_R_UNALIGNED, 0);
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

/* ── §39(c): A LEAF THAT LEAVES THE WINDOW ──────────────────────────────────────
 * ★★★★★ These four are a different stake from every bounds case above them.
 * `KFWR_R_OOB` is about a TABLE WE WOULD READ -- getting it wrong reads memory
 * that is not ours. These are about a MAPPING WE WOULD MAKE -- getting it wrong
 * HANDS THE GUEST memory that is not its own, which is the escalation the walker
 * exists to prevent (§39(b): a guest that corrupts only itself is not our
 * problem; one that reaches outside its store is).
 * ⊘ Until w760c the kernel refused neither, and `kf_validate_report` could not
 * have caught it either -- it was never given the window. The host's `map()`
 * bound was the ONLY thing standing here, which made a defence-in-depth layer
 * into a single point of failure.
 * ⊘ A 64-bit WRAP is not among these because it is INEXPRESSIBLE: the VER2 VID
 * aperture carries 25 address bits at shift 12, so the largest encodable leaf
 * address is (2^25-1)<<12 ~ 137 GiB and `gpga + len` cannot overflow. Same shape
 * as `unaligned_is_inexpressible_below_the_root`: the encoding is the bound. */

static KfWalkCfg cfg_span(uint64_t span)
{
    KfWalkCfg c = cfg_default();
    c.gpga_span = span;          /* §39(c): a guest whose GPGA is exactly this big */
    return c;
}

static void t_hostile_leaf_past_end(void)
{
    Fix f(8u << 20, cfg_span(8u << 20));
    Tree t(f.g);
    t.map4k(VBASE, (uint64_t)f.g.size() + (16u << 20));   /* wholly outside the store */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_LEAF_OOB,
            "a leaf pointing past the end of GPGA must refuse as LEAF_OOB");
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_hostile_leaf_straddles_end(void)
{
    /* Starts inside, ends outside. The `gpga < len` half of the test passes and
     * only the EXTENT half refuses -- the case a naive `gpga < win.len` misses. */
    Fix f((8u << 20) + 4096u, cfg_span((8u << 20) + 4096u));
    Tree t(f.g);
    t.map2m(VBASE, 8ull << 20);          /* 2 MiB-aligned, inside; ends 2 MiB past */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_LEAF_OOB,
            "a leaf that STARTS inside and ENDS outside must refuse as LEAF_OOB");
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_hostile_leaf_at_exact_end(void)
{
    /* ★ THE OFF-BY-ONE GUARD, and the reason it is in the hostile block: it is
     * the case an over-strict containment check breaks. `gpga + len == win.len`
     * is the last LEGAL page. A walker that refuses it would silently drop the
     * guest's top page of memory, and no test above would notice. */
    Fix f(8u << 20, cfg_span(8u << 20));
    Tree t(f.g);
    const uint64_t last = (uint64_t)f.g.size() - 4096ull;
    t.map4k(VBASE, last);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_EQ(f.hdr.refuse_mask & (uint32_t)KFWR_R_LEAF_OOB, 0);
    CHECK_EQ(f.hdr.run_count, 1);
    if (f.hdr.run_count == 1) {
        CHECK_EQ(f.rn[0].va, VBASE);
        CHECK_EQ(f.rn[0].gpga, last);
        CHECK_EQ(f.rn[0].len, 4096ull);
    }
    if (g_fails_here) dump(f);
}

static void t_hostile_leaf_oob_does_not_extend_its_neighbour(void)
{
    /* ★★★★★ THE ORDERING TEST, and the only one here that can fail while the
     * other three pass. The refused leaf is EXACTLY coalesce-adjacent to the
     * legal one before it (0x7ff000 + 4096 == 0x800000 == win.len), so if the
     * containment check sat AFTER the coalesce branch instead of before it, the
     * good run would have absorbed the bad one and grown to 8192 bytes -- a run
     * that REACHES OUTSIDE THE STORE while every refusal flag stays clear.
     * ⊘ That is a silent escalation, not a loud refusal, which is why the check
     * is placed above the coalesce branch in both emit chokepoints. */
    Fix f(8u << 20, cfg_span(8u << 20));
    Tree t(f.g);
    const uint64_t last = (uint64_t)f.g.size() - 4096ull;   /* 0x7ff000 */
    t.map4k(VBASE,            last);                        /* legal: ends AT the end */
    t.map4k(VBASE + 4096ull,  (uint64_t)f.g.size());        /* one page too far      */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_LEAF_OOB, "the second leaf must refuse as LEAF_OOB");
    CHECK_EQ(f.hdr.run_count, 1);
    if (f.hdr.run_count == 1) {
        CHECK_EQ(f.rn[0].gpga, last);
        CHECK_M(f.rn[0].len == 4096ull,
                "the legal run ABSORBED the refused one: the containment check is "
                "running after the coalesce branch, not before it");
    }
    if (g_fails_here) dump(f);
}

/* ── §39: WHAT ogkm ITSELF REFUSES ─────────────────────────────────────────────
 * ★★★★★ Mined from the vendor's own checks (docs/design/ogkm_checks_as_a_fuzz_corpus.md).
 * ⊘ Neither of these is hostile input. Both are bit patterns a STOCK driver
 * writes in the ordinary course of business, and we reported WRONG MAPPINGS for
 * both -- which is why they sit here with controls rather than in a fuzz list. */

/* How many leaves does this tree report when nothing vetoes it? The control is
 * the whole point: "0 runs" is also what a tree that never got built looks like. */
static uint32_t dual_control_runs(uint64_t *pd0_out)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 16u; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x300000ull + (uint64_t)i * 4096ull);
    if (pd0_out) *pd0_out = t.pd0(VBASE);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    return f.hdr.run_count;
}

static void t_ogkm_unmapped_big_pte_hides_stale_4k(void)
{
    /* uvm_va_block.c:6484-6492 unmaps 64 KiB by writing ONE unmapped big PTE and
     * DELIBERATELY leaving the 4 KiB PTEs stale: "we only need to invalidate the
     * 4k PTEs without actually writing them". uvm_mmu.h:203-212 says the MMU then
     * "should stop its walk and not cache any 4k entries which may be in memory".
     * ⇒ Before w760h we reported all 16 stale leaves as LIVE MAPPINGS, after an
     * honest guest had asked for them to be gone. The encoding is VALID=0, VOL=0,
     * PRIVILEGE=1 -- `0x20` on GA10x (uvm_page_tree_test.c:1774). */
    CHECK_M(dual_control_runs(NULL) > 0,
            "the CONTROL reported nothing: this tree never had mappings to hide");

    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 16u; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x300000ull + (uint64_t)i * 4096ull);
    /* ⊘ w826 — the sentinel is a big PTE, INSIDE the big table, one per 64 KiB slot. This
     * test used to poke it into the dual PDE's low word, where bit 5 is an ADDRESS bit
     * (ogkm-580 pascal/gp100/dev_mmu.h:102) — and so encoded the defect that dropped live
     * 4 KiB leaves. Slot 1 is the control: its 4 KiB leaf must survive. */
    t.map4k(VBASE + 0x10000ull, 0x310000ull);
    f.g.u64(t.ptb(VBASE) + (uint64_t)vib(VBASE) * 8) = 0x20ull;   /* the UNMAPPED big PTE */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE + 0x10000ull, 0x310000ull, 4096ull, F4K, KFWR_OP_MAP},
    });
}

static void t_ogkm_sparse_big_half_hides_small_table(void)
{
    /* ogkm `_gmmuIsInvalidPdeOk` (gmmu_trace.c:429-466) returns NV_FALSE when
     * sublevel 0 -- THE BIG HALF -- is sparse, and mmu_trace.c:552-558 turns that
     * into NV_ERR_INVALID_XLATE and LEAVES the walk rather than continuing. So a
     * sparse big half aborts the whole 2 MiB WITHOUT EVER READING sublevel 1. */
    CHECK_M(dual_control_runs(NULL) > 0,
            "the CONTROL reported nothing: this tree never had mappings to hide");

    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 16u; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x300000ull + (uint64_t)i * 4096ull);
    f.g.u64(t.pd0(VBASE) + (uint64_t)vi0(VBASE) * 16) = kfb_sparse_pde();
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK_M(f.hdr.run_count == 0,
            "a SPARSE big half must suppress the small table under it -- ogkm aborts "
            "translation for the whole 2 MiB without reading sublevel 1");
    CHECK_M(f.hdr.sparse_slots >= 1u, "the sparse slot must still be COUNTED, not just obeyed");
    if (g_fails_here) dump(f);
}

static void t_hostile_unaligned_root(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root + 8ull}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_UNALIGNED, "an unaligned root must refuse as UNALIGNED");
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_hostile_root_out_of_range(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    f.upload();
    CHECK_EQ(f.refresh({(uint64_t)f.g.size() + (1u << 20)}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_OOB, "a root past the end must refuse as OOB");
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_hostile_all_ones(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    uint64_t pd2 = t.pd2(VBASE), pd1 = t.pd1(VBASE), pd0 = t.pd0(VBASE), pts = t.pts(VBASE);
    f.g.u64(t.root + 0) = ~0ull;   /* slot 0; VBASE lives in slot 1 */
    f.g.u64(pd2 + 8) = ~0ull;
    f.g.u64(pd1 + 8) = ~0ull;
    f.g.u64(pd0 + 16) = ~0ull;
    f.g.u64(pd0 + 16 + 8) = ~0ull;
    f.g.u64(pts + 8) = ~0ull;
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    /* ~0 at PD1 and PD0 has VALID set, so each decodes as a large LEAF whose
     * address field is 4 KiB-granular and therefore NOT aligned to its own page
     * size. That is the one case the encoding permits and the hardware does not
     * define — refused by name, never reported as a mapping. */
    CHECK_M(f.hdr.refuse_mask & KFWR_R_MISALIGNED_LEAF,
            "a large leaf with a 4 KiB-granular target must be refused by name");
    /* the 4 KiB leaf at ~0 IS well formed and must survive, beside the real one */
    CHECK_EQ(f.hdr.run_count, 2);
    if (g_fails_here) dump(f);
}

static void t_hostile_all_ones_vid_pointer(void)
{
    /* aperture = vidmem and every address bit set: the pure out-of-range case. */
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    f.g.u64(t.root + (uint64_t)vi3(VBASE) * 8) = 0x2ull | (0x1FFFFFFull << 8);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK(f.hdr.refuse_mask & KFWR_R_OOB);
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_hostile_type_confusion(void)
{
    /* a PDE pointing at a page of PTEs, and a PTE table pointer aimed at a page
     * of PDEs. Both are nonsense; both must terminate in bounds. */
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    for (uint32_t i = 0; i < 64; i++)
        f.g.u64(t.pts(VBASE) + i * 8) = kfb_pte(0x400000ull + i * 4096ull, AP_PTE_SCOH);
    uint64_t pts = t.pts(VBASE);
    f.g.u64(t.pd2(VBASE) + (uint64_t)vi2(VBASE) * 8) = kfb_pde(pts);   /* PDE -> PTE page */
    f.g.u64(t.pd0(VBASE) + (uint64_t)vi0(VBASE) * 16 + 8) = kfb_pde(t.root); /* PT -> PDE page */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    /* a page of PTEs read as PD1 entries decodes as a row of 512 MiB leaves whose
     * targets are 4 KiB-granular: every one refused, none reported. */
    CHECK_M(f.hdr.refuse_mask & KFWR_R_MISALIGNED_LEAF, "type confusion must refuse, not map");
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_legal_shared_page_table(void)
{
    /* TWO PDEs pointing at ONE page table. This is LEGAL. It must NOT be refused,
     * and both virtual ranges must be reported. */
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    uint64_t pts = t.pts(VBASE);
    uint64_t va2 = VBASE + (4ull << 21);
    f.g.u64(t.pd0(va2) + (uint64_t)vi0(va2) * 16 + 8) = kfb_pde(pts);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE, 0x300000ull, 4096ull, F4K, KFWR_OP_MAP},
        {va2,   0x300000ull, 4096ull, F4K, KFWR_OP_MAP},
    });
    CHECK_EQ(f.hdr.refusals, 0);
    CHECK_EQ(f.hdr.refuse_mask, 0);
}

static void t_legal_pte_maps_own_page_table(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    uint64_t pts = t.pts(VBASE);
    t.map4k(VBASE, pts);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, pts, 4096ull, F4K, KFWR_OP_MAP}});
    CHECK_EQ(f.hdr.refuse_mask, 0);
}

static void t_hostile_foreign_aperture(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    uint64_t pd2 = t.pd2(VBASE);
    f.g.u64(t.root + (uint64_t)vi3(VBASE) * 8) = kfb_pde(pd2, AP_PDE_SCOH);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_FOREIGN_AP, "a sysmem page table must be refused by name");
    CHECK_EQ(f.hdr.refuse_mask & ~(uint32_t)KFWR_R_FOREIGN_AP, 0);
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_hostile_enormous_but_legal(void)
{
    /* The class that passes every other check: every pointer valid, no cycles,
     * more distinct mappings than the run cap. TRUNCATED must be set, and the
     * report must not be usable as a delta. */
    KfWalkCfg c = cfg_default();
    c.runs_per_pdb = 64;
    c.run_capacity = 8192;
    Fix f(16u << 20, c);
    Tree t(f.g);
    for (uint32_t i = 0; i < 400; i++)                 /* stride 8 KiB ⇒ never coalesces */
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 8192ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.flags & KFWR_HF_TRUNCATED, "an over-large tree must set TRUNCATED");
    CHECK_M(f.hdr.refuse_mask & KFWR_R_RUN_CAP, "and say WHICH cap it hit");
    CHECK_EQ(f.hdr.run_count, 64);
    /* The host refuses a TRUNCATED report, so it never acks it — but even if it
     * did, the kernel must not commit it: the next report is the same full set. */
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_EQ(f.hdr.run_count, 64);
    for (uint32_t i = 0; i < f.hdr.run_count; i++) CHECK_M(f.rn[i].op == KFWR_OP_MAP, "a truncated report must never be committed");
    if (g_fails_here) dump(f);
}

static void t_hostile_report_run_cap(void)
{
    KfWalkCfg c = cfg_default();
    c.runs_per_pdb = 4096;
    c.run_capacity = 16;              /* the REPORT is what overflows this time */
    Fix f(16u << 20, c);
    Tree t(f.g);
    for (uint32_t i = 0; i < 100; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 8192ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK(f.hdr.flags & KFWR_HF_TRUNCATED);
    CHECK(f.hdr.refuse_mask & KFWR_R_RUN_CAP);
    /* ⊘ A diff that does not fit is handed out as NOTHING: a partial diff is not
     * a smaller diff, it is a wrong one (and it would be committed piecemeal). */
    CHECK_EQ(f.hdr.run_count, 0);
    if (g_fails_here) dump(f);
}

static void t_hostile_budget(void)
{
    KfWalkCfg c = cfg_default();
    c.entry_budget = 50;
    Fix f(16u << 20, c);
    Tree t(f.g);
    for (uint32_t i = 0; i < 400; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 8192ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    hostile_invariants(f);
    CHECK_M(f.hdr.flags & KFWR_HF_BUDGET, "the entry budget must be loud");
    CHECK(f.hdr.flags & KFWR_HF_TRUNCATED);
    CHECK(f.hdr.refuse_mask & KFWR_R_BUDGET);
    /* ⊘ NOT `<= budget + 1` any more, and the change is honest rather than
     * convenient. The parallel walk charges a whole TABLE at a time, by one
     * atomic per warp, because charging per entry would serialise the very loads
     * the parallelism exists to overlap. So the count overshoots by at most one
     * level's fan-out per warp already in flight. What the test still pins is the
     * MEANING: the budget stopped the walk long before the end -- this tree has
     * ~200 000 entries -- and said so by name. */
    CHECK_M(f.hdr.entries_visited < 4000,
            "the budget must stop the walk EARLY, not merely eventually");
    if (g_fails_here) dump(f);
}

static void t_hostile_pdb_capacity(void)
{
    KfWalkCfg c = cfg_default();
    c.pdb_capacity = 2;
    c.max_pdbs = 8;
    Fix f(16u << 20, c);
    std::vector<uint64_t> roots;
    for (int i = 0; i < 4; i++) { Tree t(f.g); t.map4k(VBASE, 0x800000ull); roots.push_back(t.root); }
    std::sort(roots.begin(), roots.end());
    f.upload();
    CHECK_EQ(f.refresh(roots), 0);
    hostile_invariants(f);
    CHECK(f.hdr.flags & KFWR_HF_PDB_TRUNCATED);
    CHECK(f.hdr.flags & KFWR_HF_TRUNCATED);
    CHECK(f.hdr.refuse_mask & KFWR_R_PDB_CAP);
    CHECK_EQ(f.hdr.pdb_count, 2);
    /* ⊘ AND NO RUNS BEYOND THEM. A run whose pdb_index has no PdbEntry is a
     * report that fails the format doc's own property 3 — which is how this bug
     * was found, by the validator rather than by an expectation. */
    CHECK_EQ(f.hdr.run_count, 2);
    for (uint32_t i = 0; i < f.hdr.run_count; i++) CHECK(f.rn[i].pdb_index < f.hdr.pdb_count);
    if (g_fails_here) dump(f);
}

static void t_hostile_unsorted_pdbs(void)
{
    Fix f(16u << 20, cfg_default());
    Tree a(f.g), b(f.g);
    a.map4k(VBASE, 0x800000ull);
    b.map4k(VBASE, 0x900000ull);
    std::vector<uint64_t> roots = { a.root, b.root };
    if (roots[0] < roots[1]) std::swap(roots[0], roots[1]);   /* deliberately descending */
    f.upload();
    CHECK_EQ(f.refresh(roots), 0);
    hostile_invariants(f);
    /* ★ The entry order is the caller's and means nothing: each entry reports
     * its OWN walk against its own slot, and nothing is refused. */
    CHECK_EQ(f.hdr.refusals, 0);
    CHECK_EQ(f.hdr.pdb_count, 2);
    for (uint32_t i = 0; i < 2; i++) {
        CHECK_EQ(f.pe[i].pdb, roots[i]);
        CHECK_EQ(f.pe[i].run_count, 1);
        CHECK_EQ(f.rn[f.pe[i].first_run].gpga, roots[i] == a.root ? 0x800000ull : 0x900000ull);
    }
    if (g_fails_here) dump(f);
}

static void t_bounds_window_is_respected(void)
{
    /* The buffer is 16 MiB; the kernel is TOLD it is 8 MiB. Real, walkable tables
     * live beyond the 8 MiB line. If a single dereference escaped the window
     * those mappings would appear in the report. */
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    /* a complete second hierarchy, entirely above 8 MiB */
    f.g.bump = (10u << 20);
    Tree beyond(f.g);
    beyond.map4k(VBASE + 0x8000ull, 0xDEAD000ull);
    f.g.u64(t.root + (uint64_t)vi3(VBASE) * 8) = kfb_pde(beyond.pd2(VBASE + 0x8000ull));
    f.upload();
    uint32_t slot0 = 0;
    int rc = kf_refresh(f.w, f.dev, 8u << 20, &t.root, &slot0, 1, &f.hdr, f.pe.data(), f.rn.data());
    CHECK_EQ(rc, 0);
    hostile_invariants(f);
    CHECK_EQ(f.hdr.run_count, 0);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_OOB, "the window, not the allocation, is the bound");
    if (g_fails_here) dump(f);
}

/* ══ RACING ══════════════════════════════════════════════════════════════════
 * Under concurrent mutation any interleaving is acceptable. The assertion is
 * that the kernel TERMINATES, stays in bounds, and emits a well-formed report.
 */
__global__ void kf_mut_kernel(uint8_t *base, uint64_t lo, uint64_t span, uint64_t seed, uint32_t iters)
{
    uint64_t s = seed + (uint64_t)(blockIdx.x * blockDim.x + threadIdx.x) * 0x9E3779B97F4A7C15ull;
    for (uint32_t k = 0; k < iters; k++) {
        s = s * 6364136223846793005ull + 1442695040888963407ull;
        uint64_t off = lo + ((s >> 17) % span);
        *(volatile uint64_t *)(base + (off & ~7ull)) = s;
    }
}

static const uint32_t RACE_ALLOWED =
    KFWR_R_OOB | KFWR_R_UNALIGNED | KFWR_R_FOREIGN_AP | KFWR_R_RUN_CAP | KFWR_R_BUDGET |
    KFWR_R_MISALIGNED_LEAF | KFWR_R_LEAF_OOB;

static void race_check(Fix &f, int round)
{
    validate(f);
    if (f.hdr.refuse_mask & ~RACE_ALLOWED) {
        char b[96];
        snprintf(b, sizeof(b), "round %d mask=0x%x", round, f.hdr.refuse_mask);
        failf(__LINE__, "refuse_mask outside the racing-allowed set", b);
    }
    CHECK(!(f.hdr.refuse_mask & KFWR_R_TOO_DEEP));
}

static void build_race_tree(Tree &t)
{
    for (uint32_t i = 0; i < 256; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 4096ull);
    for (uint32_t i = 0; i < 16; i++)
        t.map64k(VBASE + (2ull << 21) + (uint64_t)i * 65536ull, 0x900000ull + (uint64_t)i * 65536ull);
    t.map2m(VBASE + (4ull << 21), 0xA00000ull);
    t.map512m(VBASE + (1ull << 29), 0x20000000ull);
}

/* ── the LIVE-EDIT racers ───────────────────────────────────────────────────────
 * ⊘ A racer that writes pure garbage destroys the tree in microseconds and never
 * restores it, so every later walk is a walk over noise. `0 runs total` over 200
 * rounds is what that looks like, and it PASSES every termination assertion —
 * which is why the garbage racer is kept as its own case and this one exists
 * beside it. Here the mutations are all VALID encodings of the same tree, so the
 * walk keeps finding mappings while the entries change under it.
 */
struct MutSet {
    static const int N = 96;
    uint64_t off[N];
    uint64_t alt[N][4];
    int n;
};

static void build_mut_set(Tree &t, MutSet &m)
{
    m.n = 0;
    uint64_t pts = t.pts(VBASE);
    for (int i = 0; i < 48 && m.n < MutSet::N; i++, m.n++) {
        m.off[m.n] = pts + (uint64_t)i * 8;
        uint64_t gp = 0x800000ull + (uint64_t)i * 4096ull;
        m.alt[m.n][0] = kfb_pte(gp);                               /* as built   */
        m.alt[m.n][1] = kfb_pte(gp + 0x400000ull);                 /* re-pointed */
        m.alt[m.n][2] = kfb_pte(gp, AP_PTE_VID, PTE_READ_ONLY);    /* re-flagged */
        m.alt[m.n][3] = 0ull;                                      /* unmapped   */
    }
    /* a byte-identical copy of the leaf table, so the PARENT can be flipped
     * between two valid targets while the walk is descending through it */
    uint64_t copy = t.g->alloc(4096, 4096);
    memcpy(t.g->mem.data() + copy, t.g->mem.data() + pts, 4096);
    uint64_t pde_off = t.pd0(VBASE) + (uint64_t)vi0(VBASE) * 16 + 8;
    m.off[m.n] = pde_off;
    m.alt[m.n][0] = kfb_pde(pts);
    m.alt[m.n][1] = kfb_pde(copy);
    m.alt[m.n][2] = kfb_pde(pts);
    m.alt[m.n][3] = kfb_pde(copy);
    m.n++;
    /* and the grandparent, flipped between the live PD0 and a zeroed one */
    uint64_t pd0z = t.g->alloc(4096, 4096);
    uint64_t pd1_off = t.pd1(VBASE) + (uint64_t)vi1(VBASE) * 8;
    m.off[m.n] = pd1_off;
    m.alt[m.n][0] = kfb_pde(t.pd0(VBASE));
    m.alt[m.n][1] = kfb_pde(t.pd0(VBASE));
    m.alt[m.n][2] = kfb_pde(pd0z);
    m.alt[m.n][3] = kfb_pde(t.pd0(VBASE));
    m.n++;
}

__global__ void kf_live_mut_kernel(uint8_t *base, const uint64_t *off, const uint64_t *alt,
                                   int n, uint64_t seed, uint32_t iters)
{
    uint64_t s = seed + (uint64_t)(blockIdx.x * blockDim.x + threadIdx.x) * 0x9E3779B97F4A7C15ull;
    for (uint32_t k = 0; k < iters; k++) {
        s = s * 6364136223846793005ull + 1442695040888963407ull;
        int i = (int)((s >> 20) % (uint64_t)n);
        int a = (int)((s >> 13) & 3ull);
        *(volatile uint64_t *)(base + off[i]) = alt[i * 4 + a];
    }
}

static void t_race_cpu_live_edits(void)
{
    KfWalkCfg c = cfg_default();
    c.entry_budget = 20000u;
    Fix f(8u << 20, c, /*mapped=*/true);
    Tree t(f.g);
    build_race_tree(t);
    MutSet m;
    build_mut_set(t, m);
    f.upload();

    std::atomic<bool> stop(false);
    std::atomic<unsigned long long> writes(0);
    uint8_t *hp = f.host_ptr;
    std::thread racer([&]() {
        uint64_t s = 0x9E3779B9ull;
        while (!stop.load(std::memory_order_relaxed)) {
            for (int k = 0; k < 2048; k++) {
                s = s * 6364136223846793005ull + 1442695040888963407ull;
                int i = (int)((s >> 20) % (uint64_t)m.n);
                int a = (int)((s >> 13) & 3ull);
                *(volatile uint64_t *)(hp + m.off[i]) = m.alt[i][a];
            }
            writes.fetch_add(2048, std::memory_order_relaxed);
        }
    });

    unsigned long long total_runs = 0, deepest = 0;
    auto t0 = std::chrono::steady_clock::now();
    for (int r = 0; r < 200; r++) {
        CHECK_EQ(f.refresh({t.root}), 0);
        race_check(f, r);
        total_runs += f.hdr.run_count;
        if (f.hdr.entries_visited > deepest) deepest = f.hdr.entries_visited;
        if (g_fails_here > 8) break;
    }
    auto t1 = std::chrono::steady_clock::now();
    stop.store(true);
    racer.join();
    printf("      [cpu live] 200 walks in %.0f ms, %llu edits interleaved, "
           "deepest %llu entries, %llu runs total\n",
           std::chrono::duration<double, std::milli>(t1 - t0).count(),
           (unsigned long long)writes.load(), deepest, total_runs);
    CHECK_M(deepest > 1500, "the walk never reached the leaves: vacuous");
    CHECK_M(total_runs > 0, "no mapping was ever reported: the tree did not survive the racer");
}

static void t_race_gpu_live_edits(void)
{
    KfWalkCfg c = cfg_default();
    Fix f(8u << 20, c);
    Tree t(f.g);
    build_race_tree(t);
    MutSet m;
    build_mut_set(t, m);
    f.upload();

    uint64_t *d_off = NULL, *d_alt = NULL;
    CUDA_OK(cudaMalloc(&d_off, sizeof(uint64_t) * MutSet::N));
    CUDA_OK(cudaMalloc(&d_alt, sizeof(uint64_t) * MutSet::N * 4));
    CUDA_OK(cudaMemcpy(d_off, m.off, sizeof(uint64_t) * m.n, cudaMemcpyHostToDevice));
    CUDA_OK(cudaMemcpy(d_alt, m.alt, sizeof(uint64_t) * MutSet::N * 4, cudaMemcpyHostToDevice));

    cudaStream_t s2;
    CUDA_OK(cudaStreamCreateWithFlags(&s2, cudaStreamNonBlocking));
    unsigned long long total_runs = 0, deepest = 0;
    auto t0 = std::chrono::steady_clock::now();
    for (int r = 0; r < 200; r++) {
        uint32_t iters = 500u + (uint32_t)(r * 911) % 40000u;   /* varied timing */
        kf_live_mut_kernel<<<4, 32, 0, s2>>>((uint8_t *)f.dev, d_off, d_alt, m.n,
                                             0xFEEDull + r, iters);
        CHECK_EQ(f.refresh({t.root}), 0);
        race_check(f, r);
        total_runs += f.hdr.run_count;
        if (f.hdr.entries_visited > deepest) deepest = f.hdr.entries_visited;
        if (g_fails_here > 8) break;
    }
    auto t1 = std::chrono::steady_clock::now();
    CUDA_OK(cudaStreamSynchronize(s2));
    CUDA_OK(cudaStreamDestroy(s2));
    cudaFree(d_off); cudaFree(d_alt);
    CUDA_OK(cudaGetLastError());
    printf("      [gpu live] 200 walks in %.0f ms, deepest %llu entries, %llu runs total\n",
           std::chrono::duration<double, std::milli>(t1 - t0).count(), deepest, total_runs);
    CHECK_M(deepest > 1500, "the walk never reached the leaves: vacuous");
    CHECK_M(total_runs > 0, "no mapping was ever reported: the tree did not survive the racer");
}

static void t_race_cpu_mutator(void)
{
    KfWalkCfg c = cfg_default();
    /* ⊘ Zero-copy host memory: every entry read crosses PCIe, so the budget here
     * is a WALL-CLOCK bound, not a semantic one. Large enough that a clean tree
     * (≈1800 entries) completes untruncated; small enough that a walk over
     * random garbage cannot run for minutes. */
    c.entry_budget = 20000u;
    Fix f(8u << 20, c, /*mapped=*/true);
    Tree t(f.g);
    build_race_tree(t);
    f.upload();
    /* ⊘ The ROOT page is deliberately left out of the mutated window. Scribbling
     * it makes every walk bail at the first entry, and the test then measures
     * nothing while still passing — the `deepest` assertion below is what makes
     * that state detectable. */
    uint64_t lo = 8192ull, span = f.g.bump - 8192ull - 8ull;

    std::atomic<bool> stop(false);
    std::atomic<unsigned long long> writes(0);
    uint8_t *hp = f.host_ptr;
    std::thread racer([&]() {
        uint64_t s = 0x12345678ull;
        while (!stop.load(std::memory_order_relaxed)) {
            for (int k = 0; k < 4096; k++) {
                s = s * 6364136223846793005ull + 1442695040888963407ull;
                uint64_t off = lo + ((s >> 17) % span);
                *(volatile uint64_t *)(hp + (off & ~7ull)) = s;
            }
            writes.fetch_add(4096, std::memory_order_relaxed);
        }
    });

    auto t0 = std::chrono::steady_clock::now();
    unsigned long long deepest = 0, total_runs = 0;
    for (int r = 0; r < 200; r++) {
        CHECK_EQ(f.refresh({t.root}), 0);
        race_check(f, r);
        if (f.hdr.entries_visited > deepest) deepest = f.hdr.entries_visited;
        total_runs += f.hdr.run_count;
        if (g_fails_here > 8) break;
    }
    auto t1 = std::chrono::steady_clock::now();
    stop.store(true);
    racer.join();
    double ms = std::chrono::duration<double, std::milli>(t1 - t0).count();
    printf("      [cpu racer] 200 walks in %.0f ms, %llu host writes interleaved, "
           "deepest walk %llu entries, %llu runs total\n",
           ms, (unsigned long long)writes.load(), deepest, total_runs);
    CHECK_M(ms < 120000.0, "the walk must terminate, not merely be bounded in theory");
    CHECK_M(deepest > 500, "the racer never let a walk get anywhere: the test would be vacuous");
    /* ⊘ NOT asserting runs > 0: over pure garbage a walk finding nothing is the
     * correct outcome. race/{cpu,gpu}_live_edits is where that is asserted. */
}

static void t_race_gpu_mutator(void)
{
    KfWalkCfg c = cfg_default();
    c.entry_budget = 2000000u;
    Fix f(8u << 20, c);
    Tree t(f.g);
    build_race_tree(t);
    f.upload();
    uint64_t lo = 8192ull, span = f.g.bump - 8192ull - 8ull;

    cudaStream_t s2;
    CUDA_OK(cudaStreamCreateWithFlags(&s2, cudaStreamNonBlocking));
    auto t0 = std::chrono::steady_clock::now();
    unsigned long long deepest = 0;
    for (int r = 0; r < 120; r++) {
        /* varied timing: the mutator's length changes every round, so the walk
         * and the writes overlap differently each time. */
        uint32_t iters = 2000u + (uint32_t)(r * 337) % 60000u;
        kf_mut_kernel<<<8, 64, 0, s2>>>((uint8_t *)f.dev, lo, span, 0xABCDEF00ull + r, iters);
        CHECK_EQ(f.refresh({t.root}), 0);
        race_check(f, r);
        if (f.hdr.entries_visited > deepest) deepest = f.hdr.entries_visited;
        if (g_fails_here > 8) break;
    }
    auto t1 = std::chrono::steady_clock::now();
    CHECK_M(deepest > 500, "the racer never let a walk get anywhere: the test would be vacuous");
    /* ⊘ NOT asserting runs > 0: over pure garbage a walk finding nothing is the
     * correct outcome. race/{cpu,gpu}_live_edits is where that is asserted. */
    CUDA_OK(cudaStreamSynchronize(s2));
    CUDA_OK(cudaStreamDestroy(s2));
    CUDA_OK(cudaGetLastError());
    printf("      [gpu racer] 120 walks in %.0f ms, deepest walk %llu entries\n",
           std::chrono::duration<double, std::milli>(t1 - t0).count(), deepest);
}

static void t_unaligned_is_inexpressible_below_the_root(void)
{
    /* ★★★ A FINDING, kept as a checked claim rather than a comment.
     *
     * The brief asks for "an unaligned pointer". Below the root, VER2 CANNOT
     * SPELL ONE. Every table pointer is a bit-field shifted left by exactly the
     * number of bits its target's size needs:
     *
     *   PDE      field << 12, target 4096 B  ⇒ always 4 KiB aligned
     *   dual BIG field <<  8, target  256 B  ⇒ always  256 B aligned
     *
     * So the alignment half of the bounds check can only ever fire on the ROOT,
     * which arrives from OUTSIDE the format (the host hands it in). This is
     * checked over the whole 64-bit entry space by construction: the assertion
     * holds for every possible entry value, so a loop over random ones is a
     * demonstration, not a sample. It is here so that a future format change
     * that breaks it -- VER3's unified PTE, say -- fails a test instead of
     * silently making the check reachable. */
    uint64_t s = 0x243F6A8885A308D3ull;
    for (int i = 0; i < 200000; i++) {
        s = s * 6364136223846793005ull + 1442695040888963407ull;
        uint32_t ap = (uint32_t)((s >> 1) & 3u);
        uint32_t bits = (ap == 1u) ? 25u : 46u;
        uint64_t pde = ((s >> 8) & kfb_mask(bits)) << 12;
        CHECK((pde & 4095ull) == 0ull);
        uint32_t bbits = (ap == 1u) ? 29u : 50u;
        uint64_t big = ((s >> 4) & kfb_mask(bbits)) << 8;
        CHECK((big & 255ull) == 0ull);
        if (g_fails_here) break;
    }
    /* and the root, which is NOT format-derived, is where it does fire */
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root + 256ull}), 0);
    CHECK_M(f.hdr.refuse_mask & KFWR_R_UNALIGNED, "the root is the only unaligned pointer VER2 admits");
}

/* ══ DIFFERENTIAL — two independent decoders over one corpus ═════════════════
 * The corpus and the Rust walker's decode of it are both COMMITTED
 * (cuda/walk/corpus/, written by kf_corpus.cpp and by
 * crates/kayfabe-mmu/tests/walk_kernel_differential.rs). This side reads both
 * and compares; nothing executable travels back from the box.
 *
 * Benign images: the two decoders must agree EXACTLY, as multisets of
 * (va, gpga, size, aperture, read_only).
 * Hostile images: the kernel's answer must be a SUBSET of the Rust walker's.
 * The kernel has refusals the Rust walker does not (KFWR_R_MISALIGNED_LEAF, the
 * alignment check on a table pointer) and a depth bound of 5 against its 16, so
 * it can only ever report FEWER leaves -- and reporting one the other decoder
 * never saw would be a real divergence.
 */
struct DLeaf {
    uint64_t va, gpga, size;
    uint32_t ap, ro;
    bool operator<(const DLeaf &o) const
    {
        if (va != o.va) return va < o.va;
        if (size != o.size) return size < o.size;
        if (gpga != o.gpga) return gpga < o.gpga;
        if (ap != o.ap) return ap < o.ap;
        return ro < o.ro;
    }
    bool operator==(const DLeaf &o) const
    { return va == o.va && gpga == o.gpga && size == o.size && ap == o.ap && ro == o.ro; }
};

struct DImg {
    std::string name;
    bool benign;
    uint64_t gpga_len, root;
    std::vector<uint8_t> mem;
    std::vector<DLeaf> rust;
};

static bool d_load(std::vector<DImg> &out, std::string &err,
                   const char *corpus_path = "corpus/corpus.bin",
                   const char *leaves_path = "corpus/rust_leaves.txt")
{
    FILE *f = fopen(corpus_path, "rb");
    if (!f) { err = std::string(corpus_path) + " missing"; return false; }
    char magic[8];
    if (fread(magic, 1, 8, f) != 8 || memcmp(magic, "KFCORPUS", 8)) { fclose(f); err = "bad magic"; return false; }
    uint32_t n = 0;
    if (fread(&n, 4, 1, f) != 1) { fclose(f); err = "short"; return false; }
    for (uint32_t i = 0; i < n; i++) {
        DImg im;
        uint32_t nl = 0;
        if (fread(&nl, 4, 1, f) != 1) { fclose(f); err = "short name"; return false; }
        im.name.resize(nl);
        if (fread(&im.name[0], 1, nl, f) != nl) { fclose(f); err = "short name"; return false; }
        uint8_t b = 0;
        if (fread(&b, 1, 1, f) != 1) { fclose(f); err = "short"; return false; }
        im.benign = (b != 0);
        if (fread(&im.gpga_len, 8, 1, f) != 1 || fread(&im.root, 8, 1, f) != 1) { fclose(f); err = "short"; return false; }
        uint32_t np = 0;
        if (fread(&np, 4, 1, f) != 1) { fclose(f); err = "short"; return false; }
        im.mem.assign((size_t)im.gpga_len, 0);
        for (uint32_t k = 0; k < np; k++) {
            uint64_t off = 0;
            if (fread(&off, 8, 1, f) != 1) { fclose(f); err = "short page"; return false; }
            if (off + 4096 > im.gpga_len) { fclose(f); err = "page off"; return false; }
            if (fread(im.mem.data() + off, 1, 4096, f) != 4096) { fclose(f); err = "short page"; return false; }
        }
        out.push_back(im);
    }
    fclose(f);

    FILE *e = fopen(leaves_path, "r");
    if (!e) { err = std::string(leaves_path) + " missing"; return false; }
    char line[256];
    int cur = -1;
    while (fgets(line, sizeof(line), e)) {
        if (line[0] == '#') continue;
        if (!strncmp(line, "image ", 6)) {
            char nm[128];
            int bg = 0, cnt = 0;
            if (sscanf(line + 6, "%127s %d %d", nm, &bg, &cnt) != 3) { fclose(e); err = "bad image line"; return false; }
            cur = -1;
            for (size_t i = 0; i < out.size(); i++) if (out[i].name == nm) cur = (int)i;
            if (cur < 0) { fclose(e); err = std::string("unknown image ") + nm; return false; }
        } else if (!strncmp(line, "leaf ", 5)) {
            if (cur < 0) { fclose(e); err = "leaf before image"; return false; }
            DLeaf l;
            unsigned long long a, b2, c2;
            unsigned d2, e2;
            if (sscanf(line + 5, "%llx %llx %llx %u %u", &a, &b2, &c2, &d2, &e2) != 5) { fclose(e); err = "bad leaf line"; return false; }
            l.va = a; l.gpga = b2; l.size = c2; l.ap = d2; l.ro = e2;
            out[(size_t)cur].rust.push_back(l);
        }
    }
    fclose(e);
    for (size_t i = 0; i < out.size(); i++) std::sort(out[i].rust.begin(), out[i].rust.end());
    return true;
}

static void d_run(const char *corpus_path, const char *leaves_path,
                  size_t min_images, unsigned long long min_agreed, const char *label)
{
    std::vector<DImg> imgs;
    std::string err;
    if (!d_load(imgs, err, corpus_path, leaves_path)) { failf(__LINE__, "loading the differential corpus", err.c_str()); return; }
    CHECK(imgs.size() >= min_images);

    KfWalkCfg c = cfg_default();
    c.runs_per_pdb = 8192;
    c.run_capacity = 16384;
    c.entry_budget = 4000000u;

    unsigned long long agreed = 0, subset_ok = 0;
    for (size_t i = 0; i < imgs.size(); i++) {
        DImg &im = imgs[i];
        void *dev = NULL;
        CUDA_OK(cudaMalloc(&dev, (size_t)im.gpga_len));
        CUDA_OK(cudaMemcpy(dev, im.mem.data(), (size_t)im.gpga_len, cudaMemcpyHostToDevice));
        KfWalk *w = kf_create(&c);
        KfReportHeader h;
        std::vector<KfPdbEntry> pe(c.pdb_capacity + 4);
        std::vector<KfMapRun> rn(c.run_capacity + 4);
        uint32_t slot0 = 0;
        int rc = kf_refresh(w, dev, im.gpga_len, &im.root, &slot0, 1, &h, pe.data(), rn.data());
        CHECK_EQ(rc, 0);
        const char *why = NULL;
        CHECK_M(kf_validate_report(&h, pe.data(), rn.data(), c.gpga_span, &why) == 0, why);
        CHECK_M(!(h.flags & KFWR_HF_TRUNCATED), "the corpus must fit: a truncated walk proves nothing here");

        std::vector<DLeaf> mine;
        static const uint64_t psb[4] = { 4ull << 10, 64ull << 10, 2ull << 20, 512ull << 20 };
        for (uint32_t k = 0; k < h.run_count; k++) {
            const KfMapRun &r = rn[k];
            uint64_t ps = psb[(r.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK];
            for (uint64_t o = 0; o < r.len; o += ps) {
                DLeaf l;
                l.va = r.va + o; l.gpga = r.gpga + o; l.size = ps;
                l.ap = r.flags & KFWR_RF_AP_MASK;
                l.ro = (r.flags & KFWR_RF_READ_ONLY) ? 1u : 0u;
                mine.push_back(l);
            }
        }
        std::sort(mine.begin(), mine.end());

        if (im.benign) {
            /* w825 A1: the kernel REFUSES peer leaves (no peer aperture is backed); the Rust
             * decoder still reports them. Compare only what both sides may emit. */
            im.rust.erase(std::remove_if(im.rust.begin(), im.rust.end(),
                                         [](const DLeaf &l) { return l.ap == KFWR_AP_PEER; }),
                          im.rust.end());
            if (mine.size() != im.rust.size()) {
                char b[192];
                snprintf(b, sizeof(b), "%s: kernel %zu leaves, rust %zu", im.name.c_str(), mine.size(), im.rust.size());
                failf(__LINE__, "benign image: leaf counts differ", b);
            } else {
                for (size_t k = 0; k < mine.size(); k++) {
                    if (!(mine[k] == im.rust[k])) {
                        char b[240];
                        snprintf(b, sizeof(b),
                                 "%s[%zu]: kernel va=%llx gpga=%llx sz=%llx ap=%u ro=%u | rust va=%llx gpga=%llx sz=%llx ap=%u ro=%u",
                                 im.name.c_str(), k,
                                 (unsigned long long)mine[k].va, (unsigned long long)mine[k].gpga,
                                 (unsigned long long)mine[k].size, mine[k].ap, mine[k].ro,
                                 (unsigned long long)im.rust[k].va, (unsigned long long)im.rust[k].gpga,
                                 (unsigned long long)im.rust[k].size, im.rust[k].ap, im.rust[k].ro);
                        failf(__LINE__, "benign image: leaf differs", b);
                        break;
                    }
                }
                agreed += mine.size();
            }
        } else {
            for (size_t k = 0; k < mine.size(); k++) {
                if (!std::binary_search(im.rust.begin(), im.rust.end(), mine[k])) {
                    char b[192];
                    snprintf(b, sizeof(b), "%s: kernel reported va=%llx gpga=%llx sz=%llx that the rust walker never saw",
                             im.name.c_str(), (unsigned long long)mine[k].va,
                             (unsigned long long)mine[k].gpga, (unsigned long long)mine[k].size);
                    failf(__LINE__, "hostile image: kernel is not a subset", b);
                    break;
                }
            }
            subset_ok += mine.size();
        }
        kf_destroy(w);
        cudaFree(dev);
    }
    printf("      [%s] %zu images, %llu benign leaves agreed exactly, "
           "%llu hostile leaves within the rust walker's set\n", label, imgs.size(), agreed, subset_ok);
    /* ⚠ two decoders that both produced nothing agree perfectly. */
    CHECK_M(agreed > min_agreed, "the differential decoded almost nothing: it would be vacuous");
}

static void t_differential_rust_walker(void)
{
    d_run("corpus/corpus.bin", "corpus/rust_leaves.txt", 10, 1200, "differential");
}

/* ══ ★★★★★ THE SAME DIFFERENTIAL, OVER TABLES A REAL NVIDIA DRIVER WROTE ═════
 *
 * ⊘⊘⊘ Every other case in this file -- all 58 -- builds its tables with
 * kf_tables.h, OUR OWN builder, encoding OUR OWN understanding of VER2. The
 * differential against the Rust walker does not close that gap, because both
 * decoders share the understanding; only tables written by a real driver can.
 *
 * corpus/real_ga106.bin is five address spaces lifted out of
 * traces/cap1b_coldboot_hermetic_d6.rec -- a capture of a STOCK, UNPATCHED
 * NVIDIA open 580.159.04 guest driver on a real GA106. cuda/walk/kf_real_tables.py
 * documents the extraction, its self-consistency proof (all 177 856 BAR2 writes
 * translate, zero misses) and its limits.
 *
 * ★★★ AND THE FINDING IT CARRIES: 6 986 of 7 008 real leaf PTEs set KIND
 * (bits 63:56) and 6 017 set COMPTAGLINE (55:36). `kfb_pte()` can set NEITHER --
 * it only ever writes bits 0..7 and the address field. So 99.7% of these entries
 * are encodings no case in this suite had ever contained. The decode is
 * unaffected (VER2's vidmem address is 32:8, below both), which is exactly what
 * this case now checks rather than assumes.
 */
static void t_differential_real_driver_tables(void)
{
    d_run("corpus/real_ga106.bin", "corpus/real_leaves.txt", 5, 6000, "real-driver");
}


/* ══ THE FORMAT SEAM ═════════════════════════════════════════════════════════
 * The layout is setup data (THE_CONSTRAINTS.md §21). These assert the gates on
 * it; that the descriptor is actually CONSULTED is proved by
 * `make check-seam-negative`, which perturbs it by one bit and requires the
 * suite to fail.
 */
static void t_format_refuses_unknown_table_version(void)
{
    KfWalkCfg c = cfg_default();
    c.table_version = 99u;
    KfWalk *w = kf_create(&c);
    CHECK_M(w == NULL, "an unknown table_version must be refused at create, loudly");
    if (w) kf_destroy(w);
}

static void t_format_refuses_untested_ver3(void)
{
    KfWalkCfg c = cfg_default();
    c.table_version = KF_TBL_VER3;
    KfWalk *w = kf_create(&c);
#ifdef KF_ALLOW_UNTESTED_VER3
    /* ⚠ THE ONLY CLAIM MADE ABOUT VER3 ANYWHERE, and it is deliberately small:
     * the sketched descriptor is WELL FORMED — its level count, fan-outs, entry
     * widths, root alignment and big/small coverage satisfy kf_format_check. It
     * says NOTHING about whether a VER3 table decodes correctly, because no VER3
     * table has ever existed in this project. It exists so a typo in the sketch
     * fails at build time rather than on Blackwell day one. */
    CHECK_M(w != NULL, "with the gate opened, the VER3 descriptor must pass its own validation");
#else
    /* ⚠ VER3 is a sketch. It must not be reachable by accident: a caller asking
     * for it gets a refusal, not a walk that LOOKS like Hopper support. */
    CHECK_M(w == NULL, "VER3 has never run and must be refused unless deliberately enabled");
#endif
    if (w) kf_destroy(w);
}

static void t_format_report_is_self_describing(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    t.map64k(VBASE + (1ull << 21), 0x400000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    /* The page-size CODE is format-independent; the report carries the mapping
     * from code to bytes, so the host parser needs no format knowledge. */
    CHECK_EQ(f.hdr.ps_log2[KFWR_PS_4K], 12);
    CHECK_EQ(f.hdr.ps_log2[KFWR_PS_64K], 16);
    CHECK_EQ(f.hdr.ps_log2[KFWR_PS_2M], 21);
    CHECK_EQ(f.hdr.ps_log2[KFWR_PS_512M], 29);
}

static void t_format_sparse_is_counted(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    uint64_t pts = t.pts(VBASE);
    for (uint32_t i = 1; i < 9; i++) f.g.u64(pts + (uint64_t)i * 8) = kfb_sparse_pte();
    for (uint32_t i = 9; i < 20; i++) f.g.u64(pts + (uint64_t)i * 8) = 0ull;  /* never written */
    /* a whole 2 MiB region declared sparse at the PD0 slot's small half */
    uint64_t va2 = VBASE + (4ull << 21);
    f.g.u64(t.pd0(va2) + (uint64_t)vi0(va2) * 16 + 8) = kfb_sparse_pde();
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK_EQ(f.hdr.run_count, 1);
    CHECK_M(f.hdr.sparse_slots == 9, "eight sparse PTEs and one sparse PDE, and no invalid slot");
    if (g_fails_here) dump(f);
}


/* ══ KIND IS PART OF RUN IDENTITY ═══════════════════════════════════════════
 *
 * `[found w725, replaying real driver tables]` 6 986 of 7 008 real leaf PTEs set
 * KIND, and `kf_tables.h` had never written one -- so every synthetic run in
 * this suite carried a uniform kind and could not have caught a regression here.
 * The real-table differential is the case that exercises it against a driver;
 * this is the one that pins it in milliseconds.
 *
 * A run is the unit the host publishes as ONE mapping and one mapping carries
 * one kind. Merging across a change of it makes an engine misread a surface the
 * guest wrote correctly: no fault, no refusal, wrong data.
 */
static void t_kind_joins_run_identity(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    /* 64 pages, contiguous in VA and in GPGA, identical permissions -- and a
     * change of KIND at the halfway point and nowhere else. */
    for (uint32_t i = 0; i < 64; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 4096ull,
                AP_PTE_VID, (i >= 32) ? ((uint64_t)9 << 56) : 0ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,                   0x800000ull, 32ull * 4096ull, F4K, KFWR_OP_MAP},
        {VBASE + 32ull * 4096ull, 0x820000ull, 32ull * 4096ull,
         F4K | (9u << KFWR_RF_KIND_SHIFT), KFWR_OP_MAP},
    });
}

static void t_kind_uniform_is_one_run(void)
{
    /* The control. Identical geometry, ONE kind throughout: the two halves must
     * merge, or the case above would be passing because the walk splits runs for
     * some other reason. */
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 64; i++)
        t.map4k(VBASE + (uint64_t)i * 4096ull, 0x800000ull + (uint64_t)i * 4096ull,
                AP_PTE_VID, (uint64_t)9 << 56);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0x800000ull, 64ull * 4096ull,
                F4K | (9u << KFWR_RF_KIND_SHIFT), KFWR_OP_MAP}});
}

/* ══ registry ════════════════════════════════════════════════════════════════ */
struct Case { const char *name; void (*fn)(void); };
static const Case CASES[] = {
    { "format/refuses_unknown_table_version",   t_format_refuses_unknown_table_version },
    { "format/refuses_untested_ver3",           t_format_refuses_untested_ver3 },
    { "format/report_is_self_describing",       t_format_report_is_self_describing },
    { "format/sparse_is_counted",               t_format_sparse_is_counted },

    { "correctness/kind_joins_run_identity",    t_kind_joins_run_identity },
    { "correctness/kind_uniform_is_one_run",    t_kind_uniform_is_one_run },

    { "correctness/single_4k",                  t_single_4k },
    { "correctness/large_pages_64k_2m_512m",    t_large_pages },
    /* ★ w768 — the owner's invalidate-isolation invariant, with its known-positive. */
    { "isolation/invalidate_only_disturbs_named_vas", t_invalidate_only_disturbs_the_vas_it_names },
    { "correctness/coalesce_one_run",           t_coalesce_one_run },
    { "correctness/coalesce_split_gpga",        t_coalesce_split_gpga },
    { "correctness/coalesce_split_flags",       t_coalesce_split_flags },
    { "correctness/no_coalesce_across_pagesize",t_coalesce_never_across_page_size },
    { "correctness/sparse_and_invalid_skipped", t_sparse_and_invalid_skipped },
    { "correctness/flags_decoded",              t_flags_decoded },
    { "correctness/dual_pde_both_halves",       t_dual_pde_both_halves },
    { "correctness/valid_big_pte_hides_stale_4k", t_valid_big_pte_hides_stale_4k },
    { "correctness/owning_big_pte_carries_its_permissions", t_owning_big_pte_carries_its_permissions },
    { "correctness/dual_pde_mixed_slots_live_w826", t_dual_pde_mixed_slots_live_w826 },
    { "correctness/multiple_pdbs",              t_multiple_pdbs },

    { "diff/first_walk_is_all_maps",            t_diff_first_walk_is_all_maps },
    { "diff/acked_is_quiet",                    t_diff_acked_is_quiet },
    { "diff/add",                               t_diff_add },
    { "diff/delete",                            t_diff_delete },
    { "diff/edit_gpga",                         t_diff_edit_gpga },
    { "diff/permission_edit_remaps",            t_diff_permission_edit_remaps },
    { "diff/grow_then_shrink",                  t_diff_grow_then_shrink },
    { "diff/failed_map_is_retried",             t_diff_failed_map_is_retried },
    { "diff/failed_unmap_is_retried",           t_diff_failed_unmap_is_retried },
    { "diff/held_is_committed_held",            t_diff_held_is_committed_held },
    { "diff/stale_or_absent_verdict_commits_nothing", t_diff_stale_or_absent_verdict_commits_nothing },
    { "diff/truncated_is_never_committed",      t_diff_truncated_is_never_committed },
    { "diff/root_move_keeps_the_slot",          t_diff_root_move_keeps_the_slot },
    { "diff/reset_empties_the_slot",            t_diff_reset_empties_the_slot },
    { "diff/partial_and_overflow",              t_diff_partial_and_overflow },
    { "diff/move_page_table_is_quiet",          t_diff_move_page_table_is_quiet },
    { "correctness/one_run_across_page_tables", t_one_run_across_page_tables },

    { "hostile/self_cycle",                     t_hostile_self_cycle },
    { "hostile/two_cycle",                      t_hostile_two_cycle },
    { "hostile/deep_cycle",                     t_hostile_deep_cycle },
    { "hostile/pointer_past_end",               t_hostile_ptr_past_end },
    { "hostile/table_straddles_end",            t_hostile_table_straddles_end },
    { "hostile/leaf_past_end",                  t_hostile_leaf_past_end },
    { "hostile/leaf_straddles_end",             t_hostile_leaf_straddles_end },
    { "hostile/leaf_at_exact_end",              t_hostile_leaf_at_exact_end },
    { "hostile/leaf_oob_no_neighbour_extend",   t_hostile_leaf_oob_does_not_extend_its_neighbour },
    { "ogkm/unmapped_big_pte_hides_4k",         t_ogkm_unmapped_big_pte_hides_stale_4k },
    { "ogkm/sparse_big_half_hides_small",       t_ogkm_sparse_big_half_hides_small_table },
    { "hostile/unaligned_root",                 t_hostile_unaligned_root },
    { "hostile/root_out_of_range",              t_hostile_root_out_of_range },
    { "hostile/all_ones_entries",               t_hostile_all_ones },
    { "hostile/all_ones_vidmem_pointer",        t_hostile_all_ones_vid_pointer },
    { "hostile/type_confusion",                 t_hostile_type_confusion },
    { "hostile/foreign_aperture",               t_hostile_foreign_aperture },
    { "hostile/enormous_but_legal",             t_hostile_enormous_but_legal },
    { "hostile/report_run_cap",                 t_hostile_report_run_cap },
    { "hostile/entry_budget",                   t_hostile_budget },
    { "hostile/pdb_capacity",                   t_hostile_pdb_capacity },
    { "hostile/unsorted_pdb_list",              t_hostile_unsorted_pdbs },
    { "hostile/bounds_window_respected",        t_bounds_window_is_respected },
    { "hostile/unaligned_inexpressible",        t_unaligned_is_inexpressible_below_the_root },
    { "legal/shared_page_table",                t_legal_shared_page_table },
    { "legal/pte_maps_own_page_table",          t_legal_pte_maps_own_page_table },

    { "differential/rust_walker",               t_differential_rust_walker },
    { "differential/real_driver_tables",        t_differential_real_driver_tables },

    { "roundtrip/diff_stream",                  t_roundtrip_diff_stream },
    { "roundtrip/diff_root_move",               t_roundtrip_diff_root_move },

    { "race/cpu_live_edits",                    t_race_cpu_live_edits },
    { "race/gpu_live_edits",                    t_race_gpu_live_edits },
    { "race/cpu_garbage",                       t_race_cpu_mutator },
    { "race/gpu_garbage",                       t_race_gpu_mutator },

};

int main(int argc, char **argv)
{
    const char *filter = (argc > 1) ? argv[1] : NULL;
    int dev = 0;
    cudaDeviceProp prop;
    if (cudaGetDeviceProperties(&prop, dev) != cudaSuccess) {
        printf("no CUDA device\n");
        return 2;
    }
    cudaSetDevice(dev);
    printf("device: %s  sm_%d%d  driver-jit-from-PTX\n", prop.name, prop.major, prop.minor);
    printf("sizeof: header=%zu pdb=%zu run=%zu\n",
           sizeof(KfReportHeader), sizeof(KfPdbEntry), sizeof(KfMapRun));
    if (sizeof(KfReportHeader) != 64 || sizeof(KfPdbEntry) != 32 || sizeof(KfMapRun) != 32) {
        printf("FATAL: report ABI is not the format doc's 64/32/32\n");
        return 2;
    }

    size_t n = sizeof(CASES) / sizeof(CASES[0]);
    for (size_t i = 0; i < n; i++) {
        if (filter && !strstr(CASES[i].name, filter)) continue;
        g_name = CASES[i].name;
        g_fails_here = 0;
        CASES[i].fn();
        cudaError_t e = cudaGetLastError();
        if (e != cudaSuccess) {
            failf(__LINE__, "cudaGetLastError after the case", cudaGetErrorString(e));
            cudaDeviceReset();
            cudaSetDevice(dev);
        }
        if (g_fails_here) { g_fail++; printf("FAIL %s (%d)\n", CASES[i].name, g_fails_here); }
        else { g_pass++; printf("pass %s\n", CASES[i].name); }
        fflush(stdout);
    }
    printf("\n==== %d passed, %d failed ====\n", g_pass, g_fail);
    return g_fail ? 1 : 0;
}
