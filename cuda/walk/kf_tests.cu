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

    int refresh(const std::vector<uint64_t> &roots,
                const std::vector<KfScope> &sc = std::vector<KfScope>())
    {
        return kf_refresh(w, dev, g.size(), roots.data(), (uint32_t)roots.size(),
                          sc.empty() ? NULL : sc.data(), (uint32_t)sc.size(),
                          &hdr, pe.data(), rn.data());
    }
    void ack() { kf_ack(w, hdr.generation); }
};

static KfWalkCfg cfg_default(void)
{
    KfWalkCfg c;
    c.runs_per_pdb = 4096;
    c.run_capacity = 8192;
    c.pdb_capacity = 8;
    c.entry_budget = 4000000u;
    c.max_pdbs = 8;
    c.table_version = KF_TBL_VER2;
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
    int rc = kf_validate_report(&f.hdr, f.pe.data(), f.rn.data(), &why);
    CHECK_M(rc == 0, why);
    /* ⚠ The REPORT's order is (page-size class ascending, VA ascending within a
     * class) -- NOT the walk's (va asc, size desc). The diff is computed per
     * class, because a 4 KiB and a 64 KiB leaf can name the same VA. ⊘ And the
     * order is no longer load-bearing: the UNMAP set and the MAP/REMAP set are
     * disjoint in (va, class) by construction, so a host may apply the runs in
     * any order. The round-trip test asserts exactly that. */
#if !defined(KF_OLD_MERGE) && !defined(KF_BREAK_ORDER)
    for (uint32_t p = 0; p < f.hdr.pdb_count; p++) {
        for (uint32_t i = 1; i < f.pe[p].run_count; i++) {
            const KfMapRun &a = f.rn[f.pe[p].first_run + i - 1];
            const KfMapRun &b = f.rn[f.pe[p].first_run + i];
            uint32_t pa = (a.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
            uint32_t pb = (b.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
            bool ok = (pa < pb) || (pa == pb && a.va < b.va);
            CHECK_M(ok, "runs not in (page-size class asc, va asc) order");
        }
    }
#endif
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

static void t_single_4k(void)
{
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x300000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0x300000ull, 4096ull, F4K, KFWR_OP_MAP}});
    CHECK(f.hdr.flags & KFWR_HF_RESYNC);
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
        {VBASE + 6 * 4096ull, 0x360000ull, 4096ull, F4K | KFWR_AP_PEER,           KFWR_OP_MAP},
    });
}

static void t_dual_pde_both_halves(void)
{
    /* Both sub-tables of one PD0 slot populated. Both must come back, and the
     * order must be (va asc, page size desc). */
    Fix f(8u << 20, cfg_default());
    Tree t(f.g);
    t.map64k(VBASE, 0x400000ull);
    t.map4k(VBASE, 0x500000ull);
    t.map4k(VBASE + 0x1000ull, 0x501000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,             0x500000ull, 2ull * 4096ull, F4K, KFWR_OP_MAP},
        {VBASE,             0x400000ull, 64ull << 10, F64K, KFWR_OP_MAP},
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

/* Every delta test starts from a settled baseline: walk, ack, then mutate. */
static void baseline(Fix &f, Tree &t, uint32_t want_runs)
{
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK(f.hdr.flags & KFWR_HF_RESYNC);
    CHECK_EQ(f.hdr.run_count, want_runs);
    f.ack();
}

static void t_delta_nothing_changed(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 8; i++) t.map4k(VBASE + i * 4096ull, 0x800000ull + i * 4096ull);
    baseline(f, t, 1);
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK(!(f.hdr.flags & KFWR_HF_RESYNC));
    CHECK_EQ(f.hdr.run_count, 0);
    CHECK_EQ(f.hdr.pdb_count, 1);
    CHECK_EQ(f.pe[0].vas_flags, 0);
    if (g_fails_here) dump(f);
}

static void t_delta_add(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x800000ull);
    baseline(f, t, 1);
    t.map4k(VBASE + (1ull << 21), 0x900000ull);   /* a separate 2 MiB region */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + (1ull << 21), 0x900000ull, 4096ull, F4K, KFWR_OP_MAP}});
}

static void t_delta_delete(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x800000ull);
    t.map4k(VBASE + (1ull << 21), 0x900000ull);
    baseline(f, t, 2);
    t.unmap4k(VBASE + (1ull << 21));
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + (1ull << 21), 0x900000ull, 4096ull, F4K, KFWR_OP_UNMAP}});
}

static void t_delta_edit_gpga(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x800000ull);
    baseline(f, t, 1);
    t.map4k(VBASE, 0xA00000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0xA00000ull, 4096ull, F4K, KFWR_OP_REMAP}});
}

static void t_delta_edit_flags(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x800000ull);
    baseline(f, t, 1);
    t.map4k(VBASE, 0x800000ull, AP_PTE_VID, PTE_READ_ONLY);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE, 0x800000ull, 4096ull, F4K | KFWR_RF_READ_ONLY, KFWR_OP_REMAP}});
}

static void t_delta_extend_and_shrink(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 4; i++) t.map4k(VBASE + i * 4096ull, 0x800000ull + i * 4096ull);
    baseline(f, t, 1);
    /* extend: a 5th adjacent page. One REMAP over the WHOLE new extent — never
     * an UNMAP+MAP pair, which would leave a window with the VA unmapped. */
    t.map4k(VBASE + 4 * 4096ull, 0x804000ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    /* ★ The unchanged prefix is NOT re-pointed. The delta is exactly the new
     * page -- the per-class segment diff compares coverage, not whole runs. */
    expect(f, {{VBASE + 4 * 4096ull, 0x804000ull, 4096ull, F4K, KFWR_OP_MAP}});
    f.ack();
    /* shrink: drop the last two. REMAP the survivor + UNMAP the tail. */
    t.unmap4k(VBASE + 4 * 4096ull);
    t.unmap4k(VBASE + 3 * 4096ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + 3 * 4096ull, 0x803000ull, 2 * 4096ull, F4K, KFWR_OP_UNMAP}});
}

static void t_delta_move_page_table(void)
{
    /* Repoint the parent PDE at a byte-identical COPY of the leaf table. The
     * mappings did not change, so the report must be EMPTY. */
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    for (uint32_t i = 0; i < 8; i++) t.map4k(VBASE + i * 4096ull, 0x800000ull + i * 4096ull);
    baseline(f, t, 1);
    uint64_t old_pts = t.pts(VBASE);
    uint64_t nw = f.g.alloc(4096, 4096);
    memcpy(f.g.mem.data() + nw, f.g.mem.data() + old_pts, 4096);
    memset(f.g.mem.data() + old_pts, 0, 4096);
    f.g.u64(t.pd0(VBASE) + (uint64_t)vi0(VBASE) * 16 + 8) = kfb_pde(nw);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    validate(f);
    CHECK_EQ(f.hdr.run_count, 0);
    CHECK(!(f.hdr.flags & KFWR_HF_RESYNC));
    if (g_fails_here) dump(f);
}

static void t_delta_change_root(void)
{
    Fix f(16u << 20, cfg_default());
    Tree a(f.g);
    a.map4k(VBASE, 0x800000ull);
    baseline(f, a, 1);
    Tree b(f.g);
    b.map4k(VBASE + 0x1000ull, 0x900000ull);
    f.upload();
    CHECK_EQ(f.refresh({b.root}), 0);
    validate(f);
    CHECK_EQ(f.hdr.pdb_count, 2);
    CHECK_EQ(f.hdr.run_count, 1);
    int gone = -1, fresh = -1;
    for (uint32_t i = 0; i < f.hdr.pdb_count; i++) {
        if (f.pe[i].vas_flags & KFWR_V_GONE) gone = (int)i;
        if (f.pe[i].vas_flags & KFWR_V_NEW)  fresh = (int)i;
    }
    CHECK(gone >= 0 && fresh >= 0);
    if (gone >= 0) { CHECK_EQ(f.pe[gone].pdb, a.root); CHECK_EQ(f.pe[gone].run_count, 0); }
    if (fresh >= 0) { CHECK_EQ(f.pe[fresh].pdb, b.root); CHECK_EQ(f.pe[fresh].run_count, 1); }
    if (g_fails_here) dump(f);
}

static void t_delta_free_subtree(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x800000ull);
    t.map4k(VBASE + (1ull << 29), 0x900000ull);   /* a different PD1 slot */
    baseline(f, t, 2);
    f.g.u64(t.pd1(VBASE) + (uint64_t)vi1(VBASE + (1ull << 29)) * 8) = 0ull;  /* free it */
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {{VBASE + (1ull << 29), 0x900000ull, 4096ull, F4K, KFWR_OP_UNMAP}});
}

static void t_generation_handshake(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    t.map4k(VBASE, 0x800000ull);
    baseline(f, t, 1);                  /* gen 1, acked */
    CHECK_EQ(f.refresh({t.root}), 0);   /* gen 2, delta, NOT acked */
    CHECK(!(f.hdr.flags & KFWR_HF_RESYNC));
    CHECK_EQ(f.hdr.generation, 2);
    CHECK_EQ(f.hdr.acked_generation, 1);
    CHECK_EQ(f.refresh({t.root}), 0);   /* gen 3 — the host never acked 2 */
    validate(f);
    CHECK_M(f.hdr.flags & KFWR_HF_RESYNC, "a lost ack must force a full resync");
    CHECK_EQ(f.hdr.run_count, 1);
    CHECK_EQ(f.rn[0].op, KFWR_OP_MAP);
    if (g_fails_here) dump(f);
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
     * did, the kernel must not use a truncated walk as a delta baseline. */
    f.ack();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_M(f.hdr.flags & KFWR_HF_RESYNC, "a truncated walk must never become a delta baseline");
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
    CHECK_EQ(f.hdr.run_count, 16);
    CHECK_EQ(f.hdr.run_count, f.hdr.run_capacity);   /* independently visible */
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
    CHECK(f.hdr.entries_visited <= 51);
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
    CHECK_M(f.hdr.refuse_mask & KFWR_R_PDB_UNSORTED, "an unsorted pdb list must be named");
    CHECK(f.hdr.flags & KFWR_HF_RESYNC);
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
    int rc = kf_refresh(f.w, f.dev, 8u << 20, &t.root, 1, NULL, 0, &f.hdr, f.pe.data(), f.rn.data());
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
    KFWR_R_MISALIGNED_LEAF;

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

/* ══ SCOPE HINTS ═════════════════════════════════════════════════════════════ */

static void scope_setup(Fix &f, Tree &t)
{
    t.map4k(VBASE, 0x800000ull);                       /* region A */
    t.map4k(VBASE + (1ull << 29), 0x900000ull);        /* region B, a PD1 away */
    baseline(f, t, 2);
}

static void t_scope_covering_the_change(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    scope_setup(f, t);
    t.map4k(VBASE + 0x1000ull, 0x801000ull);
    f.upload();
    KfScope sc = { t.root, VBASE & ~((1ull << 29) - 1ull), 1ull << 29 };
    CHECK_EQ(f.refresh({t.root}, { sc }), 0);
    validate(f);
    CHECK(f.hdr.flags & KFWR_HF_SCOPED);
    /* the new page is adjacent to the old one ⇒ the run extends ⇒ REMAP; and
     * region B, which the hint excluded, must NOT be reported as unmapped. */
    expect(f, {{VBASE + 0x1000ull, 0x801000ull, 4096ull, F4K, KFWR_OP_MAP}});
}

static void t_scope_excluding_a_pdb_carries_it(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    scope_setup(f, t);
    t.map4k(VBASE + 0x1000ull, 0x801000ull);
    f.upload();
    KfScope sc = { t.root + 0x1000ull, 0ull, 1ull << 29 };   /* names a DIFFERENT pdb */
    CHECK_EQ(f.refresh({t.root}, { sc }), 0);
    validate(f);
    CHECK_EQ(f.hdr.run_count, 0);       /* nothing announced ⇒ nothing reported */
    if (g_fails_here) dump(f);
}

static void t_scope_sentinel_is_a_full_walk(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    scope_setup(f, t);
    t.map4k(VBASE + 0x1000ull, 0x801000ull);
    f.upload();
    KfScope sc = { t.root, 0ull, 0ull };     /* va_len == 0: the doc's sentinel */
    CHECK_EQ(f.refresh({t.root}, { sc }), 0);
    expect(f, {{VBASE + 0x1000ull, 0x801000ull, 4096ull, F4K, KFWR_OP_MAP}});
}

static void t_scope_overflow_degrades_to_full(void)
{
    Fix f(16u << 20, cfg_default());
    Tree t(f.g);
    scope_setup(f, t);
    t.map4k(VBASE + 0x1000ull, 0x801000ull);
    f.upload();
    std::vector<KfScope> sc;
    for (int i = 0; i < 5; i++)
        sc.push_back({ t.root, VBASE + (uint64_t)i * (1ull << 29), 1ull << 29 });
    CHECK_EQ(f.refresh({t.root}, sc), 0);
    validate(f);
    CHECK_M(f.hdr.flags & KFWR_HF_SCOPE_DEGRADED, "too many hints must degrade, loudly");
    expect(f, {{VBASE + 0x1000ull, 0x801000ull, 4096ull, F4K, KFWR_OP_MAP}});
}

static void t_scope_cannot_make_the_walk_wrong(void)
{
    /* The asymmetry the doc rests on: a hint that COVERS the change must give the
     * same answer as no hint at all. */
    Fix a(16u << 20, cfg_default());
    Tree ta(a.g);
    scope_setup(a, ta);
    ta.map4k(VBASE + 0x1000ull, 0x801000ull);
    a.upload();
    KfScope sc = { ta.root, VBASE & ~((1ull << 29) - 1ull), 1ull << 29 };
    CHECK_EQ(a.refresh({ta.root}, { sc }), 0);

    Fix b(16u << 20, cfg_default());
    Tree tb(b.g);
    scope_setup(b, tb);
    tb.map4k(VBASE + 0x1000ull, 0x801000ull);
    b.upload();
    CHECK_EQ(b.refresh({tb.root}), 0);

    CHECK_EQ(a.hdr.run_count, b.hdr.run_count);
    for (uint32_t i = 0; i < a.hdr.run_count && i < b.hdr.run_count; i++) {
        CHECK_EQ(a.rn[i].va, b.rn[i].va);
        CHECK_EQ(a.rn[i].len, b.rn[i].len);
        CHECK_EQ(a.rn[i].gpga, b.rn[i].gpga);
        CHECK_EQ(a.rn[i].op, b.rn[i].op);
    }
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

static bool d_load(std::vector<DImg> &out, std::string &err)
{
    FILE *f = fopen("corpus/corpus.bin", "rb");
    if (!f) { err = "corpus/corpus.bin missing"; return false; }
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

    FILE *e = fopen("corpus/rust_leaves.txt", "r");
    if (!e) { err = "corpus/rust_leaves.txt missing"; return false; }
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

static void t_differential_rust_walker(void)
{
    std::vector<DImg> imgs;
    std::string err;
    if (!d_load(imgs, err)) { failf(__LINE__, "loading the differential corpus", err.c_str()); return; }
    CHECK(imgs.size() >= 10);

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
        int rc = kf_refresh(w, dev, im.gpga_len, &im.root, 1, NULL, 0, &h, pe.data(), rn.data());
        CHECK_EQ(rc, 0);
        const char *why = NULL;
        CHECK_M(kf_validate_report(&h, pe.data(), rn.data(), &why) == 0, why);
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
    printf("      [differential] %zu images, %llu benign leaves agreed exactly, "
           "%llu hostile leaves within the rust walker's set\n", imgs.size(), agreed, subset_ok);
    /* ⚠ two decoders that both produced nothing agree perfectly. */
    CHECK_M(agreed > 1200, "the differential decoded almost nothing: it would be vacuous");
}


/* ══ DELTA ROUND-TRIP CLOSURE ════════════════════════════════════════════════
 *
 *      apply(model, deltas_from_walk_N) == full_walk_N      for all N
 *
 * The nine t_delta_* cases each build ONE change and assert ONE expected delta.
 * Nothing accumulates, so nothing checks closure -- and a stream of individually
 * plausible deltas can drift silently. This does the accumulating: start from a
 * model of the mappings, then mutate / walk / APPLY / compare against a fresh
 * full walk of the same tables, for hundreds of seeded random steps.
 *
 * ⊘ DELIBERATELY NOT COUPLED TO THE SHADOW. "A fresh full walk" is obtained from
 * a SECOND walker that is never acked, so every one of its reports is a RESYNC,
 * i.e. the full current state. If the open design question settles on the kernel
 * returning full state instead of deltas, `rt_full_state` is unchanged and
 * `model_apply` becomes the host-side diff-and-apply this same loop tests.
 */

struct MKey { uint64_t pdb, va; uint32_t ps; };
static bool operator<(const MKey &a, const MKey &b)
{
    if (a.pdb != b.pdb) return a.pdb < b.pdb;
    if (a.va != b.va) return a.va < b.va;
    return a.ps < b.ps;
}
struct MVal { uint64_t gpga; uint32_t flags; };
typedef std::map<MKey, MVal> Model;

static const uint64_t RT_PSB[4] = { 4ull << 10, 64ull << 10, 2ull << 20, 512ull << 20 };

struct ApplyStat { unsigned map_over_existing, unmap_of_missing; };

static void model_erase_pdb(Model &m, uint64_t pdb)
{
    MKey lo = { pdb, 0ull, 0u }, hi = { pdb + 1ull, 0ull, 0u };
    m.erase(m.lower_bound(lo), m.lower_bound(hi));
}

static void model_apply_run(Model &m, uint64_t pdb, const KfMapRun &r, ApplyStat &st)
{
    uint32_t cls = (r.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
    uint64_t ps = RT_PSB[cls];
    for (uint64_t o = 0; o < r.len; o += ps) {
        MKey k = { pdb, r.va + o, cls };
        if (r.op == KFWR_OP_UNMAP) {
            if (!m.erase(k)) st.unmap_of_missing++;
        } else {
            if (r.op == KFWR_OP_MAP && m.count(k)) st.map_over_existing++;
            MVal v = { r.gpga + o, r.flags };
            m[k] = v;
        }
    }
}

/* `reverse` applies the runs back to front. ★ Both orders must give the same
 * model: the UNMAP set and the MAP/REMAP set are disjoint in (va, class) by
 * construction, so a host may apply a report in any order. That is the property
 * the per-class segment diff exists to provide, and asserting it here is how a
 * regression to "order matters" gets caught. */
static bool model_apply(Model &m, const KfReportHeader &h, const KfPdbEntry *pe,
                        const KfMapRun *rn, ApplyStat &st, bool reverse = false)
{
    if (h.flags & KFWR_HF_TRUNCATED) return false;     /* never applied as a delta */
    if (h.flags & KFWR_HF_RESYNC) m.clear();
    for (uint32_t p = 0; p < h.pdb_count; p++) {
        uint64_t pdb = pe[p].pdb;
        if (pe[p].vas_flags & KFWR_V_GONE) { model_erase_pdb(m, pdb); continue; }
        if (!(h.flags & KFWR_HF_RESYNC) && (pe[p].vas_flags & KFWR_V_RESYNC))
            model_erase_pdb(m, pdb);
        uint32_t n = pe[p].run_count, f0 = pe[p].first_run;
        for (uint32_t i = 0; i < n; i++) {
            uint32_t k = reverse ? (f0 + n - 1 - i) : (f0 + i);
            model_apply_run(m, pdb, rn[k], st);
        }
    }
    return true;
}

static bool model_eq(const Model &a, const Model &b, std::string &why)
{
    Model::const_iterator ia = a.begin(), ib = b.begin();
    while (ia != a.end() && ib != b.end()) {
        char buf[224];
        if (ia->first < ib->first) {
            snprintf(buf, sizeof(buf), "model has pdb=%llx va=%llx ps=%u that the full walk does not",
                     (unsigned long long)ia->first.pdb, (unsigned long long)ia->first.va, ia->first.ps);
            why = buf; return false;
        }
        if (ib->first < ia->first) {
            snprintf(buf, sizeof(buf), "full walk has pdb=%llx va=%llx ps=%u that the model does not",
                     (unsigned long long)ib->first.pdb, (unsigned long long)ib->first.va, ib->first.ps);
            why = buf; return false;
        }
        if (ia->second.gpga != ib->second.gpga || ia->second.flags != ib->second.flags) {
            snprintf(buf, sizeof(buf),
                     "pdb=%llx va=%llx ps=%u: model gpga=%llx fl=%x, full walk gpga=%llx fl=%x",
                     (unsigned long long)ia->first.pdb, (unsigned long long)ia->first.va, ia->first.ps,
                     (unsigned long long)ia->second.gpga, ia->second.flags,
                     (unsigned long long)ib->second.gpga, ib->second.flags);
            why = buf; return false;
        }
        ++ia; ++ib;
    }
    if (ia != a.end() || ib != b.end()) {
        char buf[128];
        snprintf(buf, sizeof(buf), "sizes differ: model %zu, full walk %zu", a.size(), b.size());
        why = buf; return false;
    }
    return true;
}

/* ── the arena a mutation stream acts on ─────────────────────────────────────
 * Every table is allocated ONCE at construction and remembered, so a mutation is
 * a direct byte write with a known inverse. The ROOT and the PD3/PD2 entries are
 * never touched -- the coordinator's trap: a stream that scribbles the root makes
 * every step bail at entry 1 and pass vacuously. `rt_stats` asserts otherwise. */
struct Vas {
    uint64_t root, pd1, pd0, base, pd1_word;
    uint32_t i1, i0;
    uint64_t pts[4], ptb[4];
};

static void vas_build(Gpga &g, Vas &v, uint64_t base)
{
    Tree t(g);
    v.root = t.root; v.base = base;
    v.i1 = vi1(base); v.i0 = vi0(base);
    v.pd1 = t.pd1(base);
    v.pd0 = t.pd0(base);
    for (int k = 0; k < 4; k++) {
        uint64_t va = base + (uint64_t)k * (1ull << 21);
        v.pts[k] = t.pts(va);
        v.ptb[k] = t.ptb(va);
    }
    v.pd1_word = g.u64(v.pd1 + (uint64_t)v.i1 * 8);
}

struct Rng {
    uint64_t s;
    explicit Rng(uint64_t seed) : s(seed ? seed : 1ull) {}
    uint64_t next() { s = s * 6364136223846793005ull + 1442695040888963407ull; return s >> 17; }
    uint32_t below(uint32_t n) { return (uint32_t)(next() % (uint64_t)n); }
};

static uint64_t &pd0_lo(Gpga &g, Vas &v, int k) { return g.u64(v.pd0 + (uint64_t)(v.i0 + k) * 16); }
static uint64_t &pd0_hi(Gpga &g, Vas &v, int k) { return g.u64(v.pd0 + (uint64_t)(v.i0 + k) * 16 + 8); }

/* One benign edit. Returns false if it chose a no-op. */
static void mutate_benign(Gpga &g, Vas &v, Rng &r)
{
    int k = (int)r.below(4);
    switch (r.below(12)) {
    case 0: {   /* rewrite a small table: a run, a hole and a gpga discontinuity */
        memset(g.mem.data() + v.pts[k], 0, 4096);
        uint32_t st = r.below(300), cnt = 1u + r.below(96);
        uint64_t gb = 0x2000000ull + (uint64_t)r.below(1024) * 4096ull;
        uint32_t hole = st + 1u + r.below(cnt);
        uint32_t jump = st + 1u + r.below(cnt);
        for (uint32_t i = st; i < st + cnt && i < 512; i++) {
            if (i == hole) continue;
            uint64_t gp = gb + (uint64_t)(i - st) * 4096ull + ((i >= jump) ? 0x100000ull : 0ull);
            g.u64(v.pts[k] + (uint64_t)i * 8) = kfb_pte(gp, AP_PTE_VID, (i & 8u) ? PTE_READ_ONLY : 0);
        }
        break;
    }
    case 1:  memset(g.mem.data() + v.pts[k], 0, 4096); break;         /* clear small  */
    case 2: {   /* rewrite a big table -- this is what puts BOTH halves of a dual
                 * PDE in play, i.e. two page-size classes over one VA range */
        memset(g.mem.data() + v.ptb[k], 0, 256);
        uint32_t st = r.below(24), cnt = 1u + r.below(8);
        uint64_t gb = 0x4000000ull + (uint64_t)r.below(256) * 65536ull;
        for (uint32_t i = st; i < st + cnt && i < 32; i++)
            g.u64(v.ptb[k] + (uint64_t)i * 8) = kfb_pte(gb + (uint64_t)(i - st) * 65536ull);
        break;
    }
    case 3:  memset(g.mem.data() + v.ptb[k], 0, 256); break;          /* clear big    */
    case 4:  pd0_hi(g, v, k) = r.below(2) ? kfb_pde(v.pts[k]) : 0ull; break;
    case 5:  if (!(pd0_lo(g, v, k) & PTE_VALID))
                 pd0_lo(g, v, k) = r.below(2) ? kfb_big_pde(v.ptb[k]) : 0ull;
             break;
    case 6:  /* a 2 MiB leaf REPLACES the dual PDE for that slot, and back */
             pd0_lo(g, v, k) = r.below(2)
                 ? kfb_pte(0x6000000ull + (uint64_t)r.below(8) * (1ull << 21))
                 : kfb_big_pde(v.ptb[k]);
             break;
    case 7: {   /* move a whole page table: a byte-identical copy, reparented */
        uint64_t nw = g.alloc(4096, 4096);
        memcpy(g.mem.data() + nw, g.mem.data() + v.pts[k], 4096);
        memset(g.mem.data() + v.pts[k], 0, 4096);
        v.pts[k] = nw;
        pd0_hi(g, v, k) = kfb_pde(nw);
        break;
    }
    case 8: {   /* edit one live PTE: its target, or its flags */
        uint32_t i = r.below(512);
        uint64_t &e = g.u64(v.pts[k] + (uint64_t)i * 8);
        if (e & PTE_VALID) {
            if (r.below(2)) e = kfb_pte(kfb_pte_addr(e) + 0x10000ull, AP_PTE_VID, e & 0xF8ull);
            else e ^= PTE_READ_ONLY;
        }
        break;
    }
    case 9:  g.u64(v.pd1 + (uint64_t)(v.i1 + 1u) * 8) =
                 r.below(2) ? kfb_pte(0x20000000ull * (1ull + r.below(3))) : 0ull;
             break;
    case 10: /* free / restore a whole subtree */
             g.u64(v.pd1 + (uint64_t)v.i1 * 8) = r.below(2) ? v.pd1_word : 0ull;
             break;
    default: {  /* declare a slot sparse, or un-declare it */
        uint32_t i = r.below(512);
        g.u64(v.pts[k] + (uint64_t)i * 8) = r.below(2) ? kfb_sparse_pte() : 0ull;
        break;
    }
    }
}

/* A corruption, and its repair. The root, the PD3 entry and the PD2 entry are
 * out of scope on purpose: corrupting those makes every later step vacuous. */
static void mutate_hostile(Gpga &g, Vas &v, Rng &r)
{
    int k = (int)r.below(4);
    switch (r.below(7)) {
    case 0: pd0_hi(g, v, k) = kfb_pde(v.pd0); break;                       /* cycle      */
    case 1: pd0_hi(g, v, k) = kfb_pde((uint64_t)g.size() + (2u << 20)); break; /* past end  */
    case 2: pd0_hi(g, v, k) = kfb_pde(v.pts[k], AP_PDE_SCOH); break;       /* sysmem     */
    case 3: g.u64(v.pts[k] + (uint64_t)r.below(512) * 8) = ~0ull; break;   /* all ones   */
    case 4: pd0_lo(g, v, k) = kfb_big_pde((uint64_t)g.size() + (2u << 20)); break;
    case 5: pd0_hi(g, v, k) = kfb_pde(v.root); break;                      /* type conf. */
    default:                                                               /* repair     */
        pd0_hi(g, v, k) = kfb_pde(v.pts[k]);
        if (!(pd0_lo(g, v, k) & PTE_VALID)) pd0_lo(g, v, k) = kfb_big_pde(v.ptb[k]);
        break;
    }
}

struct RtStats {
    int steps, compared, trunc, changed, nonempty, dual;
    size_t max_model;
    unsigned long long visited_max;
};

/* ★ COVERAGE, not a result: how many steps had a 4 KiB and a 64 KiB leaf inside
 * ONE 2 MiB region, i.e. both halves of a dual PDE live at once. That is the
 * state the two page-size classes can name the same VA in, and the reason the
 * diff is per class. Without this census, "the stream covers the dual-PDE case"
 * would be an assumption. */
static bool model_has_dual(const Model &m)
{
    std::set<std::pair<uint64_t, uint64_t> > small, big;
    for (Model::const_iterator i = m.begin(); i != m.end(); ++i) {
        std::pair<uint64_t, uint64_t> r(i->first.pdb, i->first.va >> 21);
        if (i->first.ps == KFWR_PS_4K) small.insert(r);
        else if (i->first.ps == KFWR_PS_64K) big.insert(r);
    }
    for (std::set<std::pair<uint64_t, uint64_t> >::const_iterator i = small.begin(); i != small.end(); ++i)
        if (big.count(*i)) return true;
    return false;
}

static void rt_full_state(KfWalk *oracle, void *dev, uint64_t len,
                          const std::vector<uint64_t> &roots, Model &out,
                          KfReportHeader &h, std::vector<KfPdbEntry> &pe,
                          std::vector<KfMapRun> &rn, bool &ok)
{
    ok = false;
    if (kf_refresh(oracle, dev, len, roots.data(), (uint32_t)roots.size(), NULL, 0,
                   &h, pe.data(), rn.data()) != 0) return;
    if (!(h.flags & KFWR_HF_RESYNC)) { failf(__LINE__, "the oracle walker must always RESYNC", NULL); return; }
    if (h.flags & KFWR_HF_TRUNCATED) return;      /* caller skips the comparison */
    out.clear();
    ApplyStat st = { 0u, 0u };
    model_apply(out, h, pe.data(), rn.data(), st);
    ok = true;
}


/* ══ the stream ══════════════════════════════════════════════════════════════ */

static const uint64_t RT_BUF = 64ull << 20;

static void rt_stream(bool hostile, uint64_t seed, int steps, RtStats &sx)
{
    Gpga g(RT_BUF);
    Vas v[3];
    for (int i = 0; i < 3; i++) vas_build(g, v[i], VBASE);

    KfWalkCfg c = cfg_default();
    c.runs_per_pdb = 4096;
    c.run_capacity = 16384;
    c.pdb_capacity = 8;
    c.max_pdbs = 8;
    c.entry_budget = 4000000u;

    void *dev = NULL;
    CUDA_OK(cudaMalloc(&dev, (size_t)RT_BUF));
    /* ⊘ Zeroed once, so a hostile pointer that lands past the arena reads a
     * DEFINED value and the stream is reproducible from its seed alone. */
    CUDA_OK(cudaMemset(dev, 0, (size_t)RT_BUF));
    KfWalk *W = kf_create(&c);        /* the delta walker -- acked after each apply */
    KfWalk *O = kf_create(&c);        /* the oracle -- NEVER acked, so always RESYNC */

    KfReportHeader hw, ho;
    std::vector<KfPdbEntry> pw(c.pdb_capacity + 4), po(c.pdb_capacity + 4);
    std::vector<KfMapRun> rw(c.run_capacity + 4), ro(c.run_capacity + 4);

    Model model, full, prev_full;
    Rng r(seed);
    memset(&sx, 0, sizeof(sx));
    sx.steps = steps;
    bool have_prev_full = false;

    for (int step = 0; step < steps; step++) {
        /* 1. mutate */
        int nmut = 1 + (int)r.below(3);
        for (int i = 0; i < nmut; i++) {
            Vas &vv = v[r.below(3)];
            if (hostile && r.below(4) == 0) mutate_hostile(g, vv, r);
            else mutate_benign(g, vv, r);
        }
        std::vector<uint64_t> roots;
        uint32_t mask = 1u + r.below(7);
        for (int i = 0; i < 3; i++) if (mask & (1u << i)) roots.push_back(v[i].root);
        std::sort(roots.begin(), roots.end());

        uint64_t up = (g.bump + 4095ull) & ~4095ull;
        CUDA_OK(cudaMemcpy(dev, g.mem.data(), (size_t)up, cudaMemcpyHostToDevice));

        /* 2. walk and APPLY */
        CHECK_EQ(kf_refresh(W, dev, RT_BUF, roots.data(), (uint32_t)roots.size(), NULL, 0,
                            &hw, pw.data(), rw.data()), 0);
        const char *why = NULL;
        CHECK_M(kf_validate_report(&hw, pw.data(), rw.data(), &why) == 0, why);
        if (hw.entries_visited > sx.visited_max) sx.visited_max = hw.entries_visited;

        bool applied = false;
        if (hw.flags & KFWR_HF_TRUNCATED) {
            /* ⊘ NEVER applied as a delta, and NOT acked -- so the next report is a
             * full resync. That is the format doc's property 1, as behaviour. */
            model.clear();
            sx.trunc++;
        } else {
            ApplyStat st = { 0u, 0u };
            Model rev = model;
            model_apply(model, hw, pw.data(), rw.data(), st);
            ApplyStat st2 = { 0u, 0u };
            model_apply(rev, hw, pw.data(), rw.data(), st2, /*reverse=*/true);
            std::string w2;
            if (!model_eq(model, rev, w2)) {
                char b[320];
                snprintf(b, sizeof(b), "seed=%llu step=%d: %s",
                         (unsigned long long)seed, step, w2.c_str());
                failf(__LINE__, "applying the report in reverse order gives a different model", b);
            }
            if (st.map_over_existing) {
                char b[128];
                snprintf(b, sizeof(b), "seed=%llu step=%d n=%u", (unsigned long long)seed, step, st.map_over_existing);
                failf(__LINE__, "a MAP landed on a VA the model already had", b);
            }
            if (hw.run_count) sx.nonempty++;
            kf_ack(W, hw.generation);
            applied = true;
        }

        /* 3. the same tables, walked from nothing */
        bool ok = false;
        rt_full_state(O, dev, RT_BUF, roots, full, ho, po, ro, ok);
        if (!ok) continue;
        if (full.size() > sx.max_model) sx.max_model = full.size();
        { std::string wc; if (have_prev_full && !model_eq(full, prev_full, wc)) sx.changed++; }
        if (model_has_dual(full)) sx.dual++;
        prev_full = full;
        have_prev_full = true;

        /* 4. THE PROPERTY */
        if (applied) {
            std::string w;
            if (!model_eq(model, full, w)) {
                char b[384];
                snprintf(b, sizeof(b), "seed=%llu step=%d runs=%u flags=0x%x mask=0x%x :: %s",
                         (unsigned long long)seed, step, hw.run_count, hw.flags, hw.refuse_mask, w.c_str());
                failf(__LINE__, "apply(model, delta) != full walk", b);
                break;
            }
            sx.compared++;
        }
        if (g_fails_here) break;
        /* keep the arena from outgrowing the buffer across long streams */
        if (g.bump > RT_BUF - (8ull << 20)) break;
    }

    kf_destroy(W);
    kf_destroy(O);
    cudaFree(dev);
}

static void rt_run(bool hostile, const char *tag, const uint64_t *seeds, int nseeds, int steps)
{
    RtStats tot;
    memset(&tot, 0, sizeof(tot));
    for (int i = 0; i < nseeds; i++) {
        RtStats sx;
        rt_stream(hostile, seeds[i], steps, sx);
        tot.steps += sx.steps; tot.compared += sx.compared; tot.trunc += sx.trunc;
        tot.changed += sx.changed; tot.nonempty += sx.nonempty; tot.dual += sx.dual;
        if (sx.max_model > tot.max_model) tot.max_model = sx.max_model;
        if (sx.visited_max > tot.visited_max) tot.visited_max = sx.visited_max;
        if (g_fails_here) { printf("      [%s] FAILING SEED %llu\n", tag, (unsigned long long)seeds[i]); break; }
    }
    printf("      [%s] %d steps over %d seeds: %d closures checked, %d non-empty deltas, "
           "%d steps changed the mapping set, %d with BOTH dual-PDE halves live, "
           "%d truncations, max model %zu, deepest walk %llu\n",
           tag, tot.steps, nseeds, tot.compared, tot.nonempty, tot.changed, tot.dual, tot.trunc,
           tot.max_model, tot.visited_max);
    /* ⚠ THE VACUITY GUARDS. A stream that scribbled the root would pass every
     * closure check while the model stayed empty and every walk bailed at entry
     * one. These are what make the green above mean something. */
    CHECK_M(tot.compared > (tot.steps * 3) / 4, "too few steps actually checked closure");
    CHECK_M(tot.max_model > 300, "the mapping set never got large: the stream did nothing");
    CHECK_M(tot.changed > tot.steps / 5, "the mutations barely changed anything");
    CHECK_M(tot.nonempty > tot.steps / 5, "almost every delta was empty");
    CHECK_M(tot.visited_max > 2000, "no walk ever got past the top levels");
    CHECK_M(tot.dual > tot.steps / 20,
            "the stream never had both halves of a dual PDE live: the per-class diff is untested");
}

static void t_roundtrip_benign(void)
{
    static const uint64_t seeds[] = { 1u, 0xC0FFEEull, 0x9E3779B97F4A7C15ull, 42u };
    rt_run(false, "benign", seeds, 4, 250);
}

static void t_roundtrip_hostile(void)
{
    static const uint64_t seeds[] = { 7u, 0xBADC0DEull, 0x243F6A8885A308D3ull, 99u };
    rt_run(true, "hostile", seeds, 4, 250);
}

/* ══ TRUNCATION IS NOT A DELTA -- and the divergence is demonstrated ═════════ */
static void t_truncated_is_never_a_delta(void)
{
    KfWalkCfg c = cfg_default();
    c.runs_per_pdb = 64;             /* the WALK truncates at 64 runs */
    c.run_capacity = 16384;
    Fix f(32u << 20, c);
    Tree t(f.g);
    for (uint32_t i = 0; i < 40; i++)                 /* 40 discontiguous runs */
        t.map4k(VBASE + (uint64_t)i * 8192ull, 0x800000ull + (uint64_t)i * 16384ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK(!(f.hdr.flags & KFWR_HF_TRUNCATED));
    ApplyStat st = { 0u, 0u };
    Model model;
    CHECK(model_apply(model, f.hdr, f.pe.data(), f.rn.data(), st));
    CHECK_EQ(model.size(), 40);
    f.ack();

    /* now push it past the run cap */
    for (uint32_t i = 40; i < 140; i++)
        t.map4k(VBASE + (uint64_t)i * 8192ull, 0x800000ull + (uint64_t)i * 16384ull);
    f.upload();
    Model before = model;
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_M(f.hdr.flags & KFWR_HF_TRUNCATED, "140 runs into a 64-run table must truncate");
    CHECK_M(!model_apply(model, f.hdr, f.pe.data(), f.rn.data(), st),
            "a TRUNCATED report must be refused as a delta by the applier itself");

    /* the full current state, from a walker with room */
    KfWalkCfg oc = cfg_default();
    oc.runs_per_pdb = 4096;
    KfWalk *O = kf_create(&oc);
    KfReportHeader ho;
    std::vector<KfPdbEntry> po(oc.pdb_capacity + 4);
    std::vector<KfMapRun> ro(oc.run_capacity + 4);
    std::vector<uint64_t> roots(1, t.root);
    Model full;
    bool ok = false;
    rt_full_state(O, f.dev, f.g.size(), roots, full, ho, po, ro, ok);
    CHECK(ok);
    CHECK_EQ(full.size(), 140);

    /* ★ THE DEMONSTRATION the property rests on: had the truncated report been
     * applied as a delta anyway, the model would be WRONG. */
    Model bad = before;
    ApplyStat st2 = { 0u, 0u };
    KfReportHeader forced = f.hdr;
    forced.flags = (uint16_t)(forced.flags & ~(uint32_t)KFWR_HF_TRUNCATED);
    model_apply(bad, forced, f.pe.data(), f.rn.data(), st2);
    std::string w;
    CHECK_M(!model_eq(bad, full, w), "applying the truncated report SHOULD have diverged, and did not");

    /* and the honest path reconciles. No ack was given, so the next report is a
     * full RESYNC; once the tree fits the cap again, applying that resync
     * reconstructs the state exactly -- the mapping set is never silently lost,
     * only deferred to a resync. */
    for (uint32_t i = 60; i < 140; i++) t.unmap4k(VBASE + (uint64_t)i * 8192ull);
    f.upload();
    model.clear();
    CHECK_EQ(f.refresh({t.root}), 0);
    CHECK_M(f.hdr.flags & KFWR_HF_RESYNC, "a refused report must be followed by a resync");
    CHECK_M(!(f.hdr.flags & KFWR_HF_TRUNCATED), "60 runs must fit a 64-run table");
    CHECK(model_apply(model, f.hdr, f.pe.data(), f.rn.data(), st));
    Model full2;
    ok = false;
    rt_full_state(O, f.dev, f.g.size(), roots, full2, ho, po, ro, ok);
    CHECK(ok);
    CHECK_EQ(full2.size(), 60);
    CHECK_M(model_eq(model, full2, w), w.c_str());
    kf_destroy(O);
}

/* ══ ROUND-TRIP UNDER THE RACER ══════════════════════════════════════════════
 * ⊘ SCOPED, and the scope is the point. Under concurrent mutation there is no
 * "the tables at step N" to compare a walk against: a second walk reads different
 * bytes. So the per-step assertion is only that the report is WELL FORMED, that
 * both apply orders agree, and that the model stays in bounds. The EXACT claim is
 * made once, at the end: stop the racer, take one more delta, apply it, and the
 * model must equal a fresh full walk of the now-quiesced tables. A racing
 * round-trip that asserted a specific value would be asserting nothing.
 */
static void t_roundtrip_under_racer(void)
{
    KfWalkCfg c = cfg_default();
    c.runs_per_pdb = 4096;
    c.run_capacity = 16384;
    c.entry_budget = 200000u;
    Fix f(16u << 20, c, /*mapped=*/true);
    Vas v;
    vas_build(f.g, v, VBASE);
    Rng r0(0x5EEDull);
    for (int i = 0; i < 24; i++) mutate_benign(f.g, v, r0);
    f.upload();

    /* the racer writes only VALID encodings, to the leaf tables and to the dual
     * PDE -- never the root. */
    MutSet m;
    m.n = 0;
    for (int k = 0; k < 4 && m.n < MutSet::N - 1; k++) {
        for (int i = 0; i < 20 && m.n < MutSet::N - 1; i++, m.n++) {
            m.off[m.n] = v.pts[k] + (uint64_t)i * 8;
            uint64_t gp = 0x2000000ull + (uint64_t)(k * 64 + i) * 4096ull;
            m.alt[m.n][0] = kfb_pte(gp);
            m.alt[m.n][1] = kfb_pte(gp + 0x200000ull);
            m.alt[m.n][2] = kfb_pte(gp, AP_PTE_VID, PTE_READ_ONLY);
            m.alt[m.n][3] = 0ull;
        }
        m.off[m.n] = v.pd0 + (uint64_t)(v.i0 + k) * 16 + 8;
        m.alt[m.n][0] = kfb_pde(v.pts[k]);
        m.alt[m.n][1] = 0ull;
        m.alt[m.n][2] = kfb_pde(v.pts[k]);
        m.alt[m.n][3] = kfb_pde(v.pts[k]);
        m.n++;
    }

    std::atomic<bool> stop(false);
    std::atomic<unsigned long long> writes(0);
    uint8_t *hp = f.host_ptr;
    std::thread racer([&]() {
        uint64_t s = 0xA5A5A5A5ull;
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

    Model model;
    std::vector<uint64_t> roots(1, v.root);
    int applied = 0, trunc = 0;
    unsigned long long deepest = 0;
    size_t max_seen = 0;
    for (int step = 0; step < 150; step++) {
        CHECK_EQ(f.refresh(roots), 0);
        const char *why = NULL;
        CHECK_M(kf_validate_report(&f.hdr, f.pe.data(), f.rn.data(), &why) == 0, why);
        if (f.hdr.entries_visited > deepest) deepest = f.hdr.entries_visited;
        if (f.hdr.refuse_mask & ~RACE_ALLOWED) {
            char b[96];
            snprintf(b, sizeof(b), "step %d mask=0x%x", step, f.hdr.refuse_mask);
            failf(__LINE__, "refuse_mask outside the racing-allowed set", b);
        }
        if (f.hdr.flags & KFWR_HF_TRUNCATED) { model.clear(); trunc++; continue; }
        ApplyStat st = { 0u, 0u };
        Model rev = model;
        model_apply(model, f.hdr, f.pe.data(), f.rn.data(), st);
        model_apply(rev, f.hdr, f.pe.data(), f.rn.data(), st, true);
        std::string w;
        CHECK_M(model_eq(model, rev, w), w.c_str());
        f.ack();
        applied++;
        if (model.size() > max_seen) max_seen = model.size();
        if (g_fails_here > 4) break;
    }
    stop.store(true);
    racer.join();

    /* ── quiesce, DETERMINISTICALLY ──
     * ⊘ The structure is restored to its as-built encoding before the final walk.
     * Without this the final mapping set is whatever the racer's LAST write to
     * each parent PDE happened to be — and `full.size() > 50` was then a vacuity
     * guard that was itself racy: it flaked (~1 run in 250) with every region's
     * PDE cleared last, reporting "the test proves little" about a run that had
     * been fine. A guard that fails at random is worse than none. The PTEs are
     * deliberately NOT restored, so the racer's effect on the CONTENT survives
     * into the reconciliation. */
    for (int k = 0; k < 4; k++) {
        uint64_t off = v.pd0 + (uint64_t)(v.i0 + k) * 16 + 8;
        *(volatile uint64_t *)(f.host_ptr + off) = kfb_pde(v.pts[k]);
    }
    *(volatile uint64_t *)(f.host_ptr + v.pd1 + (uint64_t)v.i1 * 8) = v.pd1_word;

    CHECK_EQ(f.refresh(roots), 0);
    if (f.hdr.flags & KFWR_HF_TRUNCATED) { model.clear(); CHECK_EQ(f.refresh(roots), 0); }
    ApplyStat st = { 0u, 0u };
    CHECK_M(model_apply(model, f.hdr, f.pe.data(), f.rn.data(), st),
            "the final quiesced report must be applicable");
    f.ack();

    KfWalkCfg oc = c;
    KfWalk *O = kf_create(&oc);
    KfReportHeader ho;
    std::vector<KfPdbEntry> po(oc.pdb_capacity + 4);
    std::vector<KfMapRun> ro(oc.run_capacity + 4);
    Model full;
    bool ok = false;
    rt_full_state(O, f.dev, f.g.size(), roots, full, ho, po, ro, ok);
    CHECK(ok);
    std::string w;
    CHECK_M(model_eq(model, full, w), w.c_str());
    kf_destroy(O);

    printf("      [race roundtrip] %d deltas applied, %d truncations, %llu racer writes, "
           "deepest %llu, largest model seen %zu, final model %zu\n",
           applied, trunc, (unsigned long long)writes.load(), deepest, max_seen, full.size());
    CHECK_M(applied > 50, "too few deltas were applied under the racer");
    /* Two guards, neither racy: the racer must have been carrying a real mapping
     * set at some point, and the deterministic quiesced state must be non-trivial. */
    CHECK_M(max_seen > 100, "the model was never large: the racer wrote nothing that mattered");
    CHECK_M(full.size() > 20, "the quiesced mapping set is trivial");
    CHECK_M(deepest > 1500, "the walk never reached the leaves");
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

/* ══ registry ════════════════════════════════════════════════════════════════ */
struct Case { const char *name; void (*fn)(void); };
static const Case CASES[] = {
    { "format/refuses_unknown_table_version",   t_format_refuses_unknown_table_version },
    { "format/refuses_untested_ver3",           t_format_refuses_untested_ver3 },
    { "format/report_is_self_describing",       t_format_report_is_self_describing },
    { "format/sparse_is_counted",               t_format_sparse_is_counted },

    { "correctness/single_4k",                  t_single_4k },
    { "correctness/large_pages_64k_2m_512m",    t_large_pages },
    { "correctness/coalesce_one_run",           t_coalesce_one_run },
    { "correctness/coalesce_split_gpga",        t_coalesce_split_gpga },
    { "correctness/coalesce_split_flags",       t_coalesce_split_flags },
    { "correctness/no_coalesce_across_pagesize",t_coalesce_never_across_page_size },
    { "correctness/sparse_and_invalid_skipped", t_sparse_and_invalid_skipped },
    { "correctness/flags_decoded",              t_flags_decoded },
    { "correctness/dual_pde_both_halves",       t_dual_pde_both_halves },
    { "correctness/multiple_pdbs",              t_multiple_pdbs },

    { "delta/nothing_changed",                  t_delta_nothing_changed },
    { "delta/add",                              t_delta_add },
    { "delta/delete",                           t_delta_delete },
    { "delta/edit_gpga",                        t_delta_edit_gpga },
    { "delta/edit_flags",                       t_delta_edit_flags },
    { "delta/extend_and_shrink",                t_delta_extend_and_shrink },
    { "delta/move_page_table",                  t_delta_move_page_table },
    { "delta/change_root_pdb",                  t_delta_change_root },
    { "delta/free_subtree",                     t_delta_free_subtree },
    { "delta/generation_handshake",             t_generation_handshake },

    { "hostile/self_cycle",                     t_hostile_self_cycle },
    { "hostile/two_cycle",                      t_hostile_two_cycle },
    { "hostile/deep_cycle",                     t_hostile_deep_cycle },
    { "hostile/pointer_past_end",               t_hostile_ptr_past_end },
    { "hostile/table_straddles_end",            t_hostile_table_straddles_end },
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

    { "roundtrip/truncated_is_never_a_delta",   t_truncated_is_never_a_delta },
    { "roundtrip/benign_stream",                t_roundtrip_benign },
    { "roundtrip/hostile_stream",               t_roundtrip_hostile },
    { "roundtrip/under_racer",                  t_roundtrip_under_racer },

    { "race/cpu_live_edits",                    t_race_cpu_live_edits },
    { "race/gpu_live_edits",                    t_race_gpu_live_edits },
    { "race/cpu_garbage",                       t_race_cpu_mutator },
    { "race/gpu_garbage",                       t_race_gpu_mutator },

    { "scope/covering_the_change",              t_scope_covering_the_change },
    { "scope/excluding_a_pdb_carries_it",       t_scope_excluding_a_pdb_carries_it },
    { "scope/sentinel_is_full_walk",            t_scope_sentinel_is_a_full_walk },
    { "scope/overflow_degrades_to_full",        t_scope_overflow_degrades_to_full },
    { "scope/cannot_make_the_walk_wrong",       t_scope_cannot_make_the_walk_wrong },
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
