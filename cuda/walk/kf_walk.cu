/* kf_walk.cu — a CUDA kernel that walks NVIDIA GA10x (VER2) guest page tables
 * inside one contiguous buffer standing in for GPGA, coalesces the mappings it
 * finds into runs, and emits the delta report of the_walk_kernel_report_format.md.
 *
 * ═══ THE THREE INVARIANTS, structural rather than probable ════════════════════
 *
 * I1. NO LOOP TERMINATES ON GUEST DATA.
 *     The descent is not a stack and not a recursion: it is five LITERALLY NESTED
 *     `for` loops with compile-time trip counts (4, 512, 512, 256, 32×16), one per
 *     format level. There is no `while` anywhere on the guest-data path and no way
 *     to express a sixth level. ⇒ A cycle in the guest's tables is HARMLESS rather
 *     than detected: a PDE pointing at its own page is simply read again at the
 *     next level down and then the nesting runs out. Nothing is unbounded, so
 *     nothing needs to be recognised.
 *     ⊘ This is why KFWR_R_TOO_DEEP can never be set. It is kept in the ABI as the
 *     name of a refusal this design does not need, and a test ASSERTS it is absent
 *     on every cycle case — the assertion is that the depth bound is structural.
 *     The only `while`s in this file are in the diff, over the kernel's OWN table,
 *     bounded by capacities we allocated.
 *
 * I2. EVERY DEREFERENCE IS PRECEDED BY A BOUNDS CHECK.
 *     KF_GPGA_DEREF is the only expression in this file that touches the buffer,
 *     it appears exactly once outside its own definition, and that one use is
 *     inside kf_load64 immediately after the compare. Table-extent and alignment
 *     are additionally checked once per descent by kf_table_ok, so a table that
 *     merely STRADDLES the end of the buffer is refused before its first entry.
 *     `make check-invariants` greps for both facts, and `make check-negative`
 *     is the KNOWN-POSITIVE: it rebuilds with -DKF_BREAK_BOUNDS, which deletes
 *     the two checks and nothing else, and REQUIRES the bounds cases to fail.
 *     A green suite over a check that has never been seen to fire is the
 *     failure mode this repository has paid for most often.
 *
 * I3. OUTPUT IS CAPPED AND TRUNCATION IS LOUD.
 *     Three independent caps — runs per address space, entry budget per walk, runs
 *     in the report — and each one sets KFWR_HF_TRUNCATED plus a distinct bit in
 *     refuse_mask. A walk that truncated does NOT install its table
 *     (have_prev = 0), so the next refresh is a full resync: a short report can
 *     never become the baseline a later delta is computed against.
 *
 * Format reference: ogkm-580 kern_gmmu_fmt_gp10x.c / kern_gmmu_fmt_ga10x.c and
 * NV_MMU_VER2_* in pascal/gp100/dev_mmu.h. The Rust implementation this agrees
 * with is kayfabe-chips/src/ga10x.rs (Ga10xGmmu) + kayfabe-mmu/src/walker.rs.
 */

#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "kf_walk.h"

/* ── VER2 geometry (ga10x.rs, verbatim) ──────────────────────────────────────
 *  PD3      bits 48:47   4 entries × 8 B  =   32 B
 *  PD2      bits 46:38 512 entries × 8 B  = 4096 B
 *  PD1      bits 37:29 512 entries × 8 B  = 4096 B   — and a 512 MiB LEAF on GA10x
 *  PD0      bits 28:21 256 entries × 16 B = 4096 B   — and a 2 MiB LEAF
 *  PT_BIG   bits 20:16  32 entries × 8 B  =  256 B   — 64 KiB leaves
 *  PT_SMALL bits 20:12 512 entries × 8 B  = 4096 B   — 4 KiB leaves
 */
#define KF_PD3_N 4u
#define KF_PD2_N 512u
#define KF_PD1_N 512u
#define KF_PD0_N 256u
#define KF_PTB_N 32u
#define KF_PTS_N 512u

#define KF_PD3_BYTES (KF_PD3_N * 8u)
#define KF_PD2_BYTES (KF_PD2_N * 8u)
#define KF_PD1_BYTES (KF_PD1_N * 8u)
#define KF_PD0_BYTES (KF_PD0_N * 16u)
#define KF_PTB_BYTES (KF_PTB_N * 8u)
#define KF_PTS_BYTES (KF_PTS_N * 8u)

#define KF_SH_PD3 47u
#define KF_SH_PD2 38u
#define KF_SH_PD1 29u
#define KF_SH_PD0 21u
#define KF_SH_PTB 16u
#define KF_SH_PTS 12u

/* ═══ I2: the ONE expression in this file that dereferences the GPGA buffer ═══ */
struct KfWin { const uint8_t *base; uint64_t len; };
#define KF_GPGA_DEREF(win, off) (*(const volatile uint64_t *)((win).base + (off)))

/* ── the kernel's cross-refresh state ────────────────────────────────────────── */
struct KfDev {
    uint64_t generation;
    uint64_t acked;
    uint32_t have_prev;
    uint32_t cur_buf;            /* which of the two tables is the INSTALLED one */
    uint32_t runs_per_pdb;
    uint32_t entry_budget;
    uint32_t run_capacity;       /* report */
    uint32_t pdb_capacity;       /* report */
    uint32_t max_pdbs;           /* table slots */
    uint32_t tbl_pdb_count[2];
    uint64_t tbl_pdb[2][KF_MAX_PDB];
    uint32_t tbl_run_count[2][KF_MAX_PDB];
    /* per-refresh accumulators, zeroed by kf_begin_kernel */
    unsigned long long entries_visited;
    unsigned int refusals;
    unsigned int refuse_mask;
    unsigned int hdr_flags;
    unsigned int walk_trunc;
};

struct KfArgs {
    KfWin win;
    KfDev *dev;
    KfMapRun *tbl[2];
    const uint64_t *pdbs;
    uint32_t npdb;
    const KfScope *scopes;
    uint32_t nscope;
    KfReportHeader *hdr;
    KfPdbEntry *rpdb;
    KfMapRun *rrun;
};

/* ── decode helpers ──────────────────────────────────────────────────────────── */
__device__ __forceinline__ uint64_t kf_field(uint64_t raw, uint32_t lo, uint32_t bits, uint32_t sh)
{
    return ((raw >> lo) & ((1ull << bits) - 1ull)) << sh;
}
/* PTE aperture nibble: 0 vid, 1 peer, 2 sys-coh, 3 sys-noncoh. */
__device__ __forceinline__ uint64_t kf_pte_addr(uint64_t raw)
{
    uint32_t ap = (uint32_t)((raw >> 1) & 3u);
    return kf_field(raw, 8, (ap <= 1u) ? 25u : 46u, 12u);
}
/* PDE aperture nibble: 0 INVALID, 1 vid, 2 sys-coh, 3 sys-noncoh. */
__device__ __forceinline__ uint64_t kf_pde_addr(uint64_t raw)
{
    uint32_t ap = (uint32_t)((raw >> 1) & 3u);
    return kf_field(raw, 8, (ap == 1u) ? 25u : 46u, 12u);
}
/* The BIG half of a dual PDE: shift 8, not 12 (dev_mmu.h:104). */
__device__ __forceinline__ uint64_t kf_big_pde_addr(uint64_t raw)
{
    uint32_t ap = (uint32_t)((raw >> 1) & 3u);
    return kf_field(raw, 4, (ap == 1u) ? 29u : 50u, 8u);
}
__device__ __forceinline__ uint32_t kf_leaf_flags(uint64_t raw, uint32_t ps)
{
    uint32_t f = (uint32_t)((raw >> 1) & 3u) & KFWR_RF_AP_MASK;
    if (raw & (1ull << 3)) f |= KFWR_RF_VOLATILE;
    if (raw & (1ull << 5)) f |= KFWR_RF_PRIVILEGE;
    if (raw & (1ull << 6)) f |= KFWR_RF_READ_ONLY;
    if (raw & (1ull << 7)) f |= KFWR_RF_ATOMIC_DISABLE;
    return f | ((ps & KFWR_RF_PS_MASK) << KFWR_RF_PS_SHIFT);
}

/* The page size a run's flags name, in bytes. ⊘ Derived from the flags rather
 * than from `len`, because a carried-forward run's length is a MULTIPLE of the
 * page size and not itself a power of two. */
__device__ __forceinline__ uint64_t kf_ps_bytes(uint32_t flags)
{
    switch ((flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK) {
    case KFWR_PS_4K:  return 4096ull;
    case KFWR_PS_64K: return 65536ull;
    case KFWR_PS_2M:  return 1ull << 21;
    default:          return 1ull << 29;
    }
}

/* ── per-thread walk context ─────────────────────────────────────────────────── */
struct KfCtx {
    KfWin w;
    uint64_t visited;
    uint64_t budget;
    uint32_t refusals;
    uint32_t refuse;
    uint32_t stop;
    KfMapRun *out;
    uint32_t cap;
    uint32_t n;
    uint32_t have;
    KfMapRun run;
    const KfMapRun *prev;
    uint32_t prev_n;
    uint32_t pcur;
    uint16_t pdb_index;
};

/* I2: the bounds check, and the single dereference it guards. */
__device__ __forceinline__ bool kf_load64(KfCtx &c, uint64_t off, uint64_t *v)
{
#ifndef KF_BREAK_BOUNDS
    if (off > c.w.len || 8ull > c.w.len - off) {
        c.refuse |= KFWR_R_OOB; c.refusals++; return false;
    }
#endif
    *v = KF_GPGA_DEREF(c.w, off);
    return true;
}

/* A whole table, checked once per descent: alignment first, then extent.
 * `bytes` and `align` are powers of two chosen by the FORMAT, never by the guest. */
__device__ __forceinline__ bool kf_table_ok(KfCtx &c, uint64_t phys, uint64_t bytes, uint64_t align)
{
#ifndef KF_BREAK_BOUNDS
    if (phys & (align - 1ull)) { c.refuse |= KFWR_R_UNALIGNED; c.refusals++; return false; }
    if (phys > c.w.len || bytes > c.w.len - phys) { c.refuse |= KFWR_R_OOB; c.refusals++; return false; }
#endif
    return true;
}

__device__ __forceinline__ bool kf_charge(KfCtx &c, uint64_t n)
{
    c.visited += n;
    if (c.visited > c.budget) {
        c.refuse |= KFWR_R_BUDGET; c.refusals++; c.stop = 1; return false;
    }
    return true;
}

__device__ __forceinline__ void kf_flush(KfCtx &c)
{
    if (!c.have) return;
    if (c.n < c.cap) {
        c.out[c.n++] = c.run;
    } else {
        c.refuse |= KFWR_R_RUN_CAP; c.refusals++; c.stop = 1;
    }
    c.have = 0;
}

/* THE coalescer: consecutive VA, consecutive GPGA, identical flags ⇒ one run.
 *
 * ★ It also holds the one semantic refusal in the walk: a leaf whose target is
 * not aligned to its own page size. See KFWR_R_MISALIGNED_LEAF. A 4 KiB leaf can
 * never trip it (the field's granularity IS 4 KiB), so this costs nothing on the
 * ordinary path and closes the whole class at one site. */
__device__ __forceinline__ void kf_emit(KfCtx &c, uint64_t va, uint64_t gpga, uint64_t len, uint32_t flags)
{
    if (c.stop) return;
    if (gpga & (kf_ps_bytes(flags) - 1ull)) { c.refuse |= KFWR_R_MISALIGNED_LEAF; c.refusals++; return; }
    if (c.have && c.run.flags == flags &&
        c.run.va + c.run.len == va && c.run.gpga + c.run.len == gpga) {
        c.run.len += len;
        return;
    }
    kf_flush(c);
    if (c.stop) return;
    c.run.va = va; c.run.gpga = gpga; c.run.len = len;
    c.run.flags = flags; c.run.op = KFWR_OP_MAP; c.run.pdb_index = c.pdb_index;
    c.have = 1;
}

/* ── scope ───────────────────────────────────────────────────────────────────── */
#define KF_SCOPES_PER_PDB 4
struct KfScopeSet {
    uint32_t n;
    uint32_t full;                 /* 1 ⇒ walk everything */
    uint64_t lo[KF_SCOPES_PER_PDB];
    uint64_t hi[KF_SCOPES_PER_PDB];
};

__device__ __forceinline__ bool kf_in_scope(const KfScopeSet &s, uint64_t lo, uint64_t hi)
{
    if (s.full) return true;
    bool hit = false;
    for (int i = 0; i < KF_SCOPES_PER_PDB; i++)   /* fixed trip count */
        if ((uint32_t)i < s.n && lo < s.hi[i] && s.lo[i] < hi) hit = true;
    return hit;
}

/* Carry the PREVIOUS walk's runs across a VA region the scope told us to skip.
 * Clipped to [lo,hi) and pushed through the same coalescer, so a run cut by a
 * region boundary is re-joined rather than split in the report.
 * ⊘ `prev` is the KERNEL'S OWN table, bounded by runs_per_pdb — not guest data. */
__device__ void kf_carry(KfCtx &c, uint64_t lo, uint64_t hi)
{
    for (uint32_t k = c.pcur; k < c.prev_n && !c.stop; k++) {
        uint64_t rva = c.prev[k].va, rlen = c.prev[k].len, rend = rva + rlen;
        if (rend <= lo) { c.pcur = k + 1; continue; }
        if (rva >= hi) break;
        uint64_t a = rva > lo ? rva : lo;
        uint64_t b = rend < hi ? rend : hi;
        kf_emit(c, a, c.prev[k].gpga + (a - rva), b - a, c.prev[k].flags);
        if (rend <= hi) c.pcur = k + 1;
    }
}

/* ═══ THE WALK — I1: five literally nested loops, all trip counts constant ════ */
__device__ void kf_walk_one(KfCtx &c, uint64_t pdb, const KfScopeSet &sc)
{
    /* A page-directory base is a PAGE address: 4 KiB, not merely 32 B. */
    if (!kf_table_ok(c, pdb, KF_PD3_BYTES, 4096ull)) return;

    for (uint32_t i3 = 0; i3 < KF_PD3_N && !c.stop; i3++) {
        const uint64_t va3 = (uint64_t)i3 << KF_SH_PD3;
        const uint64_t end3 = va3 + (1ull << KF_SH_PD3);
        if (!kf_in_scope(sc, va3, end3)) { kf_carry(c, va3, end3); continue; }
        uint64_t e3;
        if (!kf_charge(c, 1)) break;
        if (!kf_load64(c, pdb + i3 * 8u, &e3)) continue;
        uint32_t ap3 = (uint32_t)((e3 >> 1) & 3u);
        if (ap3 == 0u) continue;                       /* INVALID or sparse */
        if (ap3 != 1u) { c.refuse |= KFWR_R_FOREIGN_AP; c.refusals++; continue; }
        const uint64_t pd2 = kf_pde_addr(e3);
        if (pd2 == 0ull) continue;
        if (!kf_table_ok(c, pd2, KF_PD2_BYTES, KF_PD2_BYTES)) continue;

        for (uint32_t i2 = 0; i2 < KF_PD2_N && !c.stop; i2++) {
            const uint64_t va2 = va3 | ((uint64_t)i2 << KF_SH_PD2);
            const uint64_t end2 = va2 + (1ull << KF_SH_PD2);
            if (!kf_in_scope(sc, va2, end2)) { kf_carry(c, va2, end2); continue; }
            uint64_t e2;
            if (!kf_charge(c, 1)) break;
            if (!kf_load64(c, pd2 + i2 * 8u, &e2)) continue;
            uint32_t ap2 = (uint32_t)((e2 >> 1) & 3u);
            if (ap2 == 0u) continue;
            if (ap2 != 1u) { c.refuse |= KFWR_R_FOREIGN_AP; c.refusals++; continue; }
            const uint64_t pd1 = kf_pde_addr(e2);
            if (pd1 == 0ull) continue;
            if (!kf_table_ok(c, pd1, KF_PD1_BYTES, KF_PD1_BYTES)) continue;

            for (uint32_t i1 = 0; i1 < KF_PD1_N && !c.stop; i1++) {
                const uint64_t va1 = va2 | ((uint64_t)i1 << KF_SH_PD1);
                const uint64_t end1 = va1 + (1ull << KF_SH_PD1);
                if (!kf_in_scope(sc, va1, end1)) { kf_carry(c, va1, end1); continue; }
                uint64_t e1;
                if (!kf_charge(c, 1)) break;
                if (!kf_load64(c, pd1 + i1 * 8u, &e1)) continue;
                /* ★ GA10x only: a PD1 slot with VALID set is a 512 MiB PAGE. */
                if (e1 & 1ull) {
                    kf_emit(c, va1, kf_pte_addr(e1), 1ull << 29, kf_leaf_flags(e1, KFWR_PS_512M));
                    continue;
                }
                uint32_t ap1 = (uint32_t)((e1 >> 1) & 3u);
                if (ap1 == 0u) continue;
                if (ap1 != 1u) { c.refuse |= KFWR_R_FOREIGN_AP; c.refusals++; continue; }
                const uint64_t pd0 = kf_pde_addr(e1);
                if (pd0 == 0ull) continue;
                if (!kf_table_ok(c, pd0, KF_PD0_BYTES, KF_PD0_BYTES)) continue;

                for (uint32_t i0 = 0; i0 < KF_PD0_N && !c.stop; i0++) {
                    const uint64_t va0 = va1 | ((uint64_t)i0 << KF_SH_PD0);
                    uint64_t lo16, hi16;
                    if (!kf_charge(c, 1)) break;
                    if (!kf_load64(c, pd0 + i0 * 16u, &lo16)) continue;
                    if (!kf_load64(c, pd0 + i0 * 16u + 8u, &hi16)) continue;
                    /* ★ A 2 MiB leaf is spelled in the LOW half's valid bit, and it is
                     * asked FIRST — the order gmmuFmtEntryIsPte asks it in. */
                    if (lo16 & 1ull) {
                        kf_emit(c, va0, kf_pte_addr(lo16), 1ull << 21,
                                kf_leaf_flags(lo16, KFWR_PS_2M));
                        continue;
                    }
                    uint32_t aps = (uint32_t)((hi16 >> 1) & 3u);   /* SMALL half in hi */
                    uint32_t apb = (uint32_t)((lo16 >> 1) & 3u);   /* BIG half in lo   */
                    uint64_t pts = 0ull, ptb = 0ull;
                    bool has_s = false, has_b = false;
                    if (aps != 0u) {
                        if (aps != 1u) { c.refuse |= KFWR_R_FOREIGN_AP; c.refusals++; }
                        else {
                            pts = kf_pde_addr(hi16);
                            has_s = (pts != 0ull) && kf_table_ok(c, pts, KF_PTS_BYTES, KF_PTS_BYTES);
                        }
                    }
                    if (apb != 0u) {
                        if (apb != 1u) { c.refuse |= KFWR_R_FOREIGN_AP; c.refusals++; }
                        else {
                            ptb = kf_big_pde_addr(lo16);
                            has_b = (ptb != 0ull) && kf_table_ok(c, ptb, KF_PTB_BYTES, KF_PTB_BYTES);
                        }
                    }
                    /* Both sub-tables cover the SAME 2 MiB of VA at different page
                     * sizes, so they are interleaved 64 KiB at a time to keep the
                     * report produced already sorted by VA. A table with only one
                     * half populated — which is every real one — degenerates to a
                     * plain ascending scan. */
                    for (uint32_t b = 0; b < KF_PTB_N && !c.stop; b++) {
                        /* BIG before SMALL inside a 64 KiB chunk. The two leaves
                         * can share a VA, and the report's order must be a TOTAL
                         * order the diff can merge-join on: (va ascending, page
                         * size DESCENDING). kf_key_cmp is that order. */
                        if (has_b) {
                            uint64_t e;
                            if (!kf_charge(c, 1)) break;
                            if (kf_load64(c, ptb + b * 8u, &e) && (e & 1ull))
                                kf_emit(c, va0 | ((uint64_t)b << KF_SH_PTB),
                                        kf_pte_addr(e), 1ull << 16,
                                        kf_leaf_flags(e, KFWR_PS_64K));
                        }
                        if (has_s) {
                            for (uint32_t s = b * 16u; s < b * 16u + 16u && !c.stop; s++) {
                                uint64_t e;
                                if (!kf_charge(c, 1)) break;
                                if (!kf_load64(c, pts + s * 8u, &e)) continue;
                                if (e & 1ull)
                                    kf_emit(c, va0 | ((uint64_t)s << KF_SH_PTS),
                                            kf_pte_addr(e), 1ull << 12,
                                            kf_leaf_flags(e, KFWR_PS_4K));
                            }
                        }
                    }
                }
            }
        }
    }
    kf_flush(c);
}

/* ── kernels ─────────────────────────────────────────────────────────────────── */
__global__ void kf_begin_kernel(KfDev *d)
{
    d->entries_visited = 0ull;
    d->refusals = 0u;
    d->refuse_mask = 0u;
    d->hdr_flags = 0u;
    d->walk_trunc = 0u;
}

__global__ void kf_walk_kernel(KfArgs a)
{
    const uint32_t t = blockIdx.x * blockDim.x + threadIdx.x;
    if (t >= a.npdb) return;
    KfDev *d = a.dev;
    const uint32_t cur = d->cur_buf ^ 1u, prv = d->cur_buf;
    const uint32_t resync = (!d->have_prev) || (d->acked != d->generation);
    const uint64_t pdb = a.pdbs[t];

    KfCtx c;
    c.w = a.win;
    c.visited = 0ull;
    c.budget = (uint64_t)d->entry_budget;
    c.refusals = 0u; c.refuse = 0u; c.stop = 0u;
    c.out = a.tbl[cur] + (size_t)t * d->runs_per_pdb;
    c.cap = d->runs_per_pdb;
    c.n = 0u; c.have = 0u;
    c.prev = NULL; c.prev_n = 0u; c.pcur = 0u;
    c.pdb_index = (uint16_t)t;
    memset(&c.run, 0, sizeof(c.run));

    /* The previous table's slice for THIS address space, if it had one. */
    if (!resync) {
        for (uint32_t j = 0; j < d->tbl_pdb_count[prv]; j++) {
            if (d->tbl_pdb[prv][j] == pdb) {
                c.prev = a.tbl[prv] + (size_t)j * d->runs_per_pdb;
                c.prev_n = d->tbl_run_count[prv][j];
                break;
            }
        }
    }

    /* Scope: a HINT. Anything ambiguous, absent or overflowing degrades to a full
     * walk, which can only cost time. A resync ignores hints entirely — there is
     * no previous table to carry the unwalked regions from. */
    KfScopeSet sc;
    sc.n = 0u; sc.full = 1u;
    for (int i = 0; i < KF_SCOPES_PER_PDB; i++) { sc.lo[i] = 0ull; sc.hi[i] = 0ull; }
    unsigned int degraded = 0u;
    if (a.nscope > 0u && !resync && c.prev != NULL) {
        sc.full = 0u;
        uint32_t seen = 0u;
        for (uint32_t i = 0; i < a.nscope; i++) {
            if (a.scopes[i].pdb != pdb) continue;
            seen++;
            uint64_t len = a.scopes[i].va_len;
            if (len == 0ull) { sc.full = 1u; }               /* the doc's sentinel */
            else if (seen <= (uint32_t)KF_SCOPES_PER_PDB) {
                uint64_t lo = a.scopes[i].va_base;
                uint64_t hi = lo + len;
                if (hi < lo) { sc.full = 1u; degraded = 1u; }  /* overflow ⇒ full */
                else {
                    /* Widen to the 512 MiB granule the pruning works at, so every
                     * carry-forward boundary is aligned to a whole PD1 slot and no
                     * clip can ever cut a page in half. A superset is always safe. */
                    sc.lo[sc.n] = lo & ~((1ull << KF_SH_PD1) - 1ull);
                    sc.hi[sc.n] = (hi + ((1ull << KF_SH_PD1) - 1ull)) & ~((1ull << KF_SH_PD1) - 1ull);
                    sc.n++;
                }
            } else { sc.full = 1u; degraded = 1u; }          /* too many ⇒ full */
        }
        if (seen == 0u) { sc.full = 0u; sc.n = 0u; }  /* named by nobody ⇒ carry it all */
    }

    kf_walk_one(c, pdb, sc);

    d->tbl_pdb[cur][t] = pdb;
    d->tbl_run_count[cur][t] = c.n;
    if (t == 0u) d->tbl_pdb_count[cur] = a.npdb;

    atomicAdd(&d->entries_visited, (unsigned long long)c.visited);
    if (c.refusals) atomicAdd(&d->refusals, c.refusals);
    if (c.refuse) atomicOr(&d->refuse_mask, c.refuse);
    if (c.stop) { atomicOr(&d->hdr_flags, KFWR_HF_TRUNCATED); atomicOr(&d->walk_trunc, 1u); }
    if (c.refuse & KFWR_R_BUDGET) atomicOr(&d->hdr_flags, KFWR_HF_BUDGET);
    if (a.nscope > 0u) atomicOr(&d->hdr_flags, KFWR_HF_SCOPED);
    if (degraded) atomicOr(&d->hdr_flags, KFWR_HF_SCOPE_DEGRADED);
}

/* The diff, and the report. ⊘ Single-threaded on purpose: the merge is a linear
 * scan over two SORTED lists and the report must come out dense and in order.
 * The loops below are bounded by capacities this process allocated, never by
 * anything the guest wrote. */
struct KfOut {
    KfMapRun *run;
    uint32_t cap;
    uint32_t n;
    uint32_t trunc;
};

__device__ __forceinline__ void kf_put(KfOut &o, KfDev *d, const KfMapRun &r, uint32_t op, uint16_t pi)
{
    if (o.n >= o.cap) {
        if (!o.trunc) { o.trunc = 1u; d->refuse_mask |= KFWR_R_RUN_CAP; d->refusals++; }
        return;
    }
    KfMapRun x = r;
    x.op = (uint16_t)op;
    x.pdb_index = pi;
    o.run[o.n++] = x;
}

__device__ __forceinline__ int kf_key_cmp(const KfMapRun &x, const KfMapRun &y)
{
    if (x.va != y.va) return x.va < y.va ? -1 : 1;
    uint32_t px = (x.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
    uint32_t py = (y.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
    /* ⚠ page size DESCENDING at an equal VA — the order the walk emits a dual
     * slot's big and small leaves in. A merge join is only correct over the
     * producer's own total order. */
    if (px != py) return px > py ? -1 : 1;
    return 0;
}

__global__ void kf_diff_kernel(KfArgs a)
{
    KfDev *d = a.dev;
    const uint32_t cur = d->cur_buf ^ 1u, prv = d->cur_buf;
    uint32_t resync = (!d->have_prev) || (d->acked != d->generation);

    uint32_t unsorted = 0u;
    for (uint32_t i = 1u; i < a.npdb; i++)
        if (a.pdbs[i] <= a.pdbs[i - 1u]) unsorted = 1u;
    if (unsorted) { d->refuse_mask |= KFWR_R_PDB_UNSORTED; d->refusals++; resync = 1u; }

    KfOut o; o.run = a.rrun; o.cap = d->run_capacity; o.n = 0u; o.trunc = 0u;
    uint32_t np_out = 0u, pdb_trunc = 0u;

    const uint32_t np = resync ? 0u : d->tbl_pdb_count[prv];
    const uint32_t nc = a.npdb;
    uint32_t ip = 0u, ic = 0u;

    while (ip < np || ic < nc) {
        int take;                                  /* -1 prev only, 0 both, 1 cur only */
        if (ip >= np) take = 1;
        else if (ic >= nc) take = -1;
        else if (d->tbl_pdb[prv][ip] < a.pdbs[ic]) take = -1;
        else if (d->tbl_pdb[prv][ip] > a.pdbs[ic]) take = 1;
        else take = 0;

        /* ⊘ The PdbEntry capacity is checked BEFORE this address space's runs are
         * written, never after. Emitting runs whose pdb_index has no PdbEntry
         * produces a report that fails the format doc's own property 3 — which is
         * exactly how this was found. */
        if (np_out >= d->pdb_capacity) {
            pdb_trunc = 1u;
            d->refuse_mask |= KFWR_R_PDB_CAP;
            d->refusals++;
            break;
        }
        uint64_t pdb = (take == -1) ? d->tbl_pdb[prv][ip] : a.pdbs[ic];
        uint32_t first = o.n;
        uint32_t vflags = 0u;
        uint16_t pi = (uint16_t)np_out;

        if (take == 1) {
            /* an address space we did not have */
            vflags |= resync ? (KFWR_V_NEW | KFWR_V_RESYNC) : KFWR_V_NEW;
            const KfMapRun *cr = a.tbl[cur] + (size_t)ic * d->runs_per_pdb;
            uint32_t cn = d->tbl_run_count[cur][ic];
            for (uint32_t k = 0u; k < cn; k++) kf_put(o, d, cr[k], KFWR_OP_MAP, pi);
            ic++;
        } else if (take == -1) {
            /* ⊘ GONE carries NO runs. The format doc leaves the choice open; the
             * whole-VAS teardown is one flag, not N unmaps. */
            vflags |= KFWR_V_GONE;
            ip++;
        } else {
            const KfMapRun *pr = a.tbl[prv] + (size_t)ip * d->runs_per_pdb;
            const KfMapRun *cr = a.tbl[cur] + (size_t)ic * d->runs_per_pdb;
            uint32_t pn = d->tbl_run_count[prv][ip];
            uint32_t cn = d->tbl_run_count[cur][ic];
            uint32_t p = 0u, q = 0u;
            while (p < pn || q < cn) {
                if (q >= cn)      { kf_put(o, d, pr[p], KFWR_OP_UNMAP, pi); p++; continue; }
                if (p >= pn)      { kf_put(o, d, cr[q], KFWR_OP_MAP,   pi); q++; continue; }
                int k = kf_key_cmp(pr[p], cr[q]);
                if (k < 0)        { kf_put(o, d, pr[p], KFWR_OP_UNMAP, pi); p++; continue; }
                if (k > 0)        { kf_put(o, d, cr[q], KFWR_OP_MAP,   pi); q++; continue; }
                KfMapRun P = pr[p], C = cr[q];
                if (P.gpga == C.gpga && P.len == C.len && P.flags == C.flags) {
                    /* unchanged — the whole point of the delta */
                } else if (C.len >= P.len) {
                    /* ★ REMAP over the WHOLE new extent rather than UNMAP+MAP: the
                     * host re-points without a window in which the VA is unmapped. */
                    kf_put(o, d, C, KFWR_OP_REMAP, pi);
                } else {
                    kf_put(o, d, C, KFWR_OP_REMAP, pi);
                    KfMapRun tail = P;
                    tail.va = C.va + C.len;
                    tail.gpga = P.gpga + C.len;
                    tail.len = P.len - C.len;
                    kf_put(o, d, tail, KFWR_OP_UNMAP, pi);
                }
                p++; q++;
            }
            ip++; ic++;
        }

        KfPdbEntry e;
        e.pdb = pdb;
        e.first_run = first;
        e.run_count = o.n - first;
        e.vas_flags = vflags;
        e.reserved = 0u;
        e.reserved2 = 0ull;
        a.rpdb[np_out] = e;
        np_out++;
    }

    d->generation += 1ull;

    /* I3: a walk that truncated must NEVER become the baseline of a later delta. */
    if (!d->walk_trunc && !unsorted) {
        d->cur_buf = cur;
        d->have_prev = 1u;
    } else {
        d->have_prev = 0u;
    }

    uint32_t flags = d->hdr_flags;
    if (resync)     flags |= KFWR_HF_RESYNC;
    if (o.trunc)    flags |= KFWR_HF_TRUNCATED;
    if (pdb_trunc)  flags |= (KFWR_HF_TRUNCATED | KFWR_HF_PDB_TRUNCATED);
    if (d->refusals) flags |= KFWR_HF_REFUSED;

    KfReportHeader h;
    h.magic = KFWR_MAGIC;
    h.version = (uint16_t)KFWR_VERSION;
    h.flags = (uint16_t)flags;
    h.generation = d->generation;
    h.acked_generation = d->acked;
    h.pdb_count = np_out;
    h.pdb_capacity = d->pdb_capacity;
    h.run_count = o.n;
    h.run_capacity = d->run_capacity;
    h.entries_visited = d->entries_visited;
    h.refusals = d->refusals;
    h.refuse_mask = d->refuse_mask;
    h.pad = 0ull;
    *a.hdr = h;
}

__global__ void kf_ack_kernel(KfDev *d, uint64_t g) { d->acked = g; }

/* ── host side ───────────────────────────────────────────────────────────────── */
struct KfWalk {
    KfDev *dev;
    KfMapRun *tbl[2];
    uint64_t *pdbs;
    KfScope *scopes;
    KfReportHeader *hdr;
    KfPdbEntry *rpdb;
    KfMapRun *rrun;
    KfWalkCfg cfg;
};

#define KF_CU(x) do { cudaError_t _e = (x); if (_e != cudaSuccess) { \
    fprintf(stderr, "CUDA %s:%d %s -> %s\n", __FILE__, __LINE__, #x, cudaGetErrorString(_e)); \
    return NULL; } } while (0)
#define KF_CU_I(x) do { cudaError_t _e = (x); if (_e != cudaSuccess) { \
    fprintf(stderr, "CUDA %s:%d %s -> %s\n", __FILE__, __LINE__, #x, cudaGetErrorString(_e)); \
    return -1; } } while (0)

extern "C" KfWalk *kf_create(const KfWalkCfg *cfg)
{
    KfWalk *w = (KfWalk *)calloc(1, sizeof(KfWalk));
    if (!w) return NULL;
    w->cfg = *cfg;
    if (w->cfg.max_pdbs == 0u || w->cfg.max_pdbs > KF_MAX_PDB) w->cfg.max_pdbs = KF_MAX_PDB;

    size_t tbl_bytes = (size_t)w->cfg.max_pdbs * w->cfg.runs_per_pdb * sizeof(KfMapRun);
    KF_CU(cudaMalloc(&w->dev, sizeof(KfDev)));
    KF_CU(cudaMalloc(&w->tbl[0], tbl_bytes));
    KF_CU(cudaMalloc(&w->tbl[1], tbl_bytes));
    KF_CU(cudaMalloc(&w->pdbs, KF_MAX_PDB * sizeof(uint64_t)));
    KF_CU(cudaMalloc(&w->scopes, KF_MAX_SCOPE * sizeof(KfScope)));
    KF_CU(cudaMalloc(&w->hdr, sizeof(KfReportHeader)));
    KF_CU(cudaMalloc(&w->rpdb, (size_t)w->cfg.pdb_capacity * sizeof(KfPdbEntry)));
    KF_CU(cudaMalloc(&w->rrun, (size_t)w->cfg.run_capacity * sizeof(KfMapRun)));

    KfDev h;
    memset(&h, 0, sizeof(h));
    h.runs_per_pdb = w->cfg.runs_per_pdb;
    h.entry_budget = w->cfg.entry_budget;
    h.run_capacity = w->cfg.run_capacity;
    h.pdb_capacity = w->cfg.pdb_capacity;
    h.max_pdbs = w->cfg.max_pdbs;
    KF_CU(cudaMemcpy(w->dev, &h, sizeof(h), cudaMemcpyHostToDevice));
    return w;
}

extern "C" void kf_destroy(KfWalk *w)
{
    if (!w) return;
    cudaFree(w->dev); cudaFree(w->tbl[0]); cudaFree(w->tbl[1]);
    cudaFree(w->pdbs); cudaFree(w->scopes);
    cudaFree(w->hdr); cudaFree(w->rpdb); cudaFree(w->rrun);
    free(w);
}

extern "C" void kf_ack(KfWalk *w, uint64_t g)
{
    kf_ack_kernel<<<1, 1>>>(w->dev, g);
    cudaDeviceSynchronize();
}

extern "C" int kf_refresh(KfWalk *w,
                          const void *gpga_dev, uint64_t gpga_len,
                          const uint64_t *pdbs, uint32_t npdb,
                          const KfScope *scopes, uint32_t nscope,
                          KfReportHeader *hdr_out, KfPdbEntry *pdb_out, KfMapRun *run_out)
{
    if (npdb == 0u) { fprintf(stderr, "kf_refresh: npdb == 0\n"); return -2; }
    if (npdb > w->cfg.max_pdbs) { fprintf(stderr, "kf_refresh: npdb %u > max_pdbs %u\n", npdb, w->cfg.max_pdbs); return -2; }
    if (nscope > KF_MAX_SCOPE)  { fprintf(stderr, "kf_refresh: nscope too large\n"); return -2; }

    KF_CU_I(cudaMemcpy(w->pdbs, pdbs, (size_t)npdb * sizeof(uint64_t), cudaMemcpyHostToDevice));
    if (nscope) KF_CU_I(cudaMemcpy(w->scopes, scopes, (size_t)nscope * sizeof(KfScope), cudaMemcpyHostToDevice));

    KfArgs a;
    a.win.base = (const uint8_t *)gpga_dev;
    a.win.len = gpga_len;
    a.dev = w->dev;
    a.tbl[0] = w->tbl[0]; a.tbl[1] = w->tbl[1];
    a.pdbs = w->pdbs; a.npdb = npdb;
    a.scopes = w->scopes; a.nscope = nscope;
    a.hdr = w->hdr; a.rpdb = w->rpdb; a.rrun = w->rrun;

    kf_begin_kernel<<<1, 1>>>(w->dev);
    kf_walk_kernel<<<(npdb + 31u) / 32u, 32>>>(a);
    kf_diff_kernel<<<1, 1>>>(a);
    KF_CU_I(cudaGetLastError());
    KF_CU_I(cudaDeviceSynchronize());

    KF_CU_I(cudaMemcpy(hdr_out, w->hdr, sizeof(KfReportHeader), cudaMemcpyDeviceToHost));
    if (hdr_out->pdb_count)
        KF_CU_I(cudaMemcpy(pdb_out, w->rpdb, (size_t)hdr_out->pdb_count * sizeof(KfPdbEntry), cudaMemcpyDeviceToHost));
    if (hdr_out->run_count)
        KF_CU_I(cudaMemcpy(run_out, w->rrun, (size_t)hdr_out->run_count * sizeof(KfMapRun), cudaMemcpyDeviceToHost));
    return 0;
}

/* Property 3 of the format doc: the host validates the report even though we wrote
 * the kernel — because its INPUT is guest-authored. ~10 comparisons. */
extern "C" int kf_validate_report(const KfReportHeader *h, const KfPdbEntry *p,
                                  const KfMapRun *r, const char **why)
{
    const char *msg = NULL;
    int rc = 0;
    static const uint64_t ps_bytes[4] = { 4ull << 10, 64ull << 10, 2ull << 20, 512ull << 20 };
    if (h->magic != KFWR_MAGIC)             { msg = "magic"; rc = -1; goto out; }
    if (h->version != KFWR_VERSION)         { msg = "version"; rc = -2; goto out; }
    if (h->run_count > h->run_capacity)     { msg = "run_count > run_capacity"; rc = -3; goto out; }
    if (h->pdb_count > h->pdb_capacity)     { msg = "pdb_count > pdb_capacity"; rc = -4; goto out; }
    for (uint32_t i = 0; i < h->pdb_count; i++) {
        /* the format doc says "first_run + run_count <= run_count"; it means the
         * header's run_count. (Deviation 2.) */
        if ((uint64_t)p[i].first_run + p[i].run_count > h->run_count) { msg = "pdb extent"; rc = -5; goto out; }
        if (i && p[i].first_run < p[i - 1].first_run)                 { msg = "pdb order"; rc = -6; goto out; }
    }
    for (uint32_t i = 0; i < h->run_count; i++) {
        uint32_t ps = (r[i].flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
        if (ps > 3u)                              { msg = "page size code"; rc = -7; goto out; }
        if (r[i].pdb_index >= h->pdb_count)       { msg = "pdb_index"; rc = -8; goto out; }
        if (r[i].len == 0ull)                     { msg = "zero len"; rc = -9; goto out; }
        if (r[i].len % ps_bytes[ps])              { msg = "len not page-aligned"; rc = -10; goto out; }
        if (r[i].va % ps_bytes[ps])               { msg = "va not page-aligned"; rc = -11; goto out; }
        if (r[i].op != KFWR_OP_MAP && r[i].op != KFWR_OP_UNMAP && r[i].op != KFWR_OP_REMAP)
                                                  { msg = "op"; rc = -12; goto out; }
        if (r[i].op != KFWR_OP_UNMAP && r[i].gpga % ps_bytes[ps])
                                                  { msg = "gpga not page-aligned"; rc = -13; goto out; }
    }
    if ((h->flags & KFWR_HF_TRUNCATED) && h->run_count > h->run_capacity) { msg = "trunc"; rc = -14; goto out; }
out:
    if (why) *why = msg ? msg : "ok";
    return rc;
}
