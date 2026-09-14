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
    for (uint32_t p = 0; p < f.hdr.pdb_count; p++) {
        for (uint32_t i = 1; i < f.pe[p].run_count; i++) {
            const KfMapRun &a = f.rn[f.pe[p].first_run + i - 1];
            const KfMapRun &b = f.rn[f.pe[p].first_run + i];
            uint32_t pa = (a.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
            uint32_t pb = (b.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
            bool ok = (a.va < b.va) || (a.va == b.va && pa >= pb);
            CHECK_M(ok, "runs not in (va asc, page-size desc) order");
        }
    }
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
        {VBASE,             0x400000ull, 64ull << 10, F64K, KFWR_OP_MAP},
        {VBASE,             0x500000ull, 2ull * 4096ull, F4K, KFWR_OP_MAP},
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
    expect(f, {{VBASE, 0x800000ull, 5 * 4096ull, F4K, KFWR_OP_REMAP}});
    f.ack();
    /* shrink: drop the last two. REMAP the survivor + UNMAP the tail. */
    t.unmap4k(VBASE + 4 * 4096ull);
    t.unmap4k(VBASE + 3 * 4096ull);
    f.upload();
    CHECK_EQ(f.refresh({t.root}), 0);
    expect(f, {
        {VBASE,                  0x800000ull, 3 * 4096ull, F4K, KFWR_OP_REMAP},
        {VBASE + 3 * 4096ull,    0x803000ull, 2 * 4096ull, F4K, KFWR_OP_UNMAP},
    });
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
    expect(f, {{VBASE, 0x800000ull, 2 * 4096ull, F4K, KFWR_OP_REMAP}});
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
    expect(f, {{VBASE, 0x800000ull, 2 * 4096ull, F4K, KFWR_OP_REMAP}});
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
    expect(f, {{VBASE, 0x800000ull, 2 * 4096ull, F4K, KFWR_OP_REMAP}});
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

/* ══ registry ════════════════════════════════════════════════════════════════ */
struct Case { const char *name; void (*fn)(void); };
static const Case CASES[] = {
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
    { "legal/shared_page_table",                t_legal_shared_page_table },
    { "legal/pte_maps_own_page_table",          t_legal_pte_maps_own_page_table },

    { "race/cpu_mutator",                       t_race_cpu_mutator },
    { "race/gpu_mutator",                       t_race_gpu_mutator },

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
