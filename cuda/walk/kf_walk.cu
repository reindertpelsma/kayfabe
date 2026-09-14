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

/* ═══ THE FORMAT SEAM — the layout is DATA, the algorithm is code ═════════════
 *
 * `THE_CONSTRAINTS.md` §21: *"our ptx must be Turing+ compatible … you need to
 * support both the turing/ada page tables as blackwell table, in same kayfabe …
 * I would avoid shipping two cuda program"*, and *"that kind of config you
 * already derived from ABI on host … I would call this setup data alongside
 * table version."*
 *
 * ⇒ **No bit position lives below this block.** Everything the decode needs —
 * level geometry, entry widths, aperture nibbles, address fields and their
 * shifts, the permission bits — is a field of [KfFormat], built on the host and
 * handed to the kernel once. A new die is a new descriptor. A new FORMAT is a
 * descriptor plus one arm in the two switches below.
 *
 * ★★★ AND I1 IS UNHARMED. The nesting is still literal and the trip counts are
 * still compile-time: every loop reads `i < KF_MAX_ENT && i < <descriptor>`.
 * A descriptor can only make a loop SHORTER. It cannot make one unbounded and it
 * cannot add a level — the nesting depth is a property of the source text, not of
 * the data. That is what makes "format as data" safe here at all.
 */

/* Nesting slots for page DIRECTORIES. Five, because VER3 has one more directory
 * level than VER2 (PD4[56] on top of PD3[55:47]); a format with fewer marks the
 * leading slots inactive and they cost one pass-through iteration each. */
#define KF_DIRS 5u
/* The compile-time cap on any level's fan-out. This is the number that keeps I1
 * structural: no descriptor can make a loop run longer than this. */
#define KF_MAX_ENT 512u
#define KF_PS_NONE 0xFFu

/* value = ((raw >> lo) & ((1<<bits)-1)) << shift */
struct KfField { uint8_t lo, bits, shift, pad; };

struct KfDir {
    uint8_t  active;        /* 0 ⇒ a pass-through: this format is shallower  */
    uint8_t  va_lo;         /* first VA bit this level indexes               */
    uint8_t  entry_bytes;   /* 8, or 16 for a dual entry                     */
    uint8_t  leaf_ps;       /* page-size CODE a VALID entry here means, or KF_PS_NONE */
    uint16_t entries;       /* 1 << (va_hi - va_lo + 1); <= KF_MAX_ENT       */
    uint16_t pad;
};

struct KfFormat {
    uint32_t abi_version;
    uint32_t table_version;

    /* ── geometry ── */
    KfDir    dir[KF_DIRS];      /* dir[KF_DIRS-1] is the DUAL level in both formats */
    uint8_t  big_va_lo, small_va_lo;
    uint8_t  big_entry_bytes, small_entry_bytes;
    uint16_t big_entries, small_entries;
    uint8_t  big_ps, small_ps;
    uint32_t root_align;        /* a page-directory base is a PAGE address */
    uint8_t  first_dir;         /* the first active slot: where the root table sits */
    uint8_t  pad0[3];

    /* ── entry fields ── */
    uint8_t  valid_bit;
    uint8_t  ap_lo, ap_bits;    /* the aperture nibble, in whichever word holds it */
    uint8_t  pde_ap_invalid;    /* the PDE aperture VALUE meaning "no sub-level"   */
    uint8_t  pte_ap_map[4];     /* raw nibble -> KFWR_AP_*, for a LEAF             */
    uint8_t  pde_ap_map[4];     /* raw nibble -> KFWR_AP_*, for a DIRECTORY        */
    uint8_t  addr_sel[4];       /* raw nibble -> 0 = the local spec, 1 = the sys spec */
    KfField  addr_local, addr_sys;          /* a normal PDE/PTE target, and the
                                             * dual entry's SMALL half — the same
                                             * spec in BOTH formats, checked */
    KfField  big_addr_local, big_addr_sys;  /* the dual entry's BIG half (shift 8) */

    /* ── permissions of a VALID leaf ──
     * ⊘ Bit POSITIONS, in both formats. VER3's PCF is documented as an enum, but
     * for these four the enumerants are a bit-field: PCF[0]=uncached,
     * PCF[1]=privilege, PCF[2]=read-only, PCF[3]=no-atomic
     * (`gh100/dev_mmu.h:498-530`, read off REGULAR_RW_ATOMIC_CACHED=0 →
     * _UNCACHED=1 → PRIVILEGE_=2 → _RO_=4 → _NO_ATOMIC_=8). So these MOVED; they
     * were not re-encoded, and they need no switch arm. */
    uint8_t  bit_volatile, bit_privilege, bit_read_only, bit_atomic_disable;

    /* ── the part that is NOT a moved field ──
     * SPARSE. VER2 spells it "valid clear, VOLATILE set"; VER3 spells it as a
     * VALUE of PCF. A bit test and an equality test against a multi-bit value are
     * different operations, so this is the one thing the descriptor cannot carry
     * and the switches below exist for. */
    KfField  pcf;
    uint8_t  pcf_sparse;        /* the PCF value meaning SPARSE (VER3)            */
    uint8_t  pad1[3];

    uint8_t  ps_log2[4];        /* page-size code -> log2(bytes) */
};

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
    unsigned int sparse_slots;
};

struct KfArgs {
    KfWin win;
    KfFormat fmt;                /* SETUP DATA: immutable for the VM's lifetime */
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

/* ── decode, entirely off the descriptor ─────────────────────────────────────── */
__device__ __forceinline__ uint64_t kf_field(uint64_t raw, uint32_t lo, uint32_t bits, uint32_t sh)
{
    return ((raw >> lo) & ((1ull << bits) - 1ull)) << sh;
}
__device__ __forceinline__ uint64_t kf_fld(uint64_t raw, const KfField &f)
{
    return kf_field(raw, f.lo, f.bits, f.shift);
}
/* The aperture nibble of `raw`. The dual entry's SMALL half lives in the HIGH
 * word at the SAME offset in both formats, so the caller passes that word. */
__device__ __forceinline__ uint32_t kf_ap_raw(const KfFormat &F, uint64_t raw)
{
    return (uint32_t)((raw >> F.ap_lo) & ((1ull << F.ap_bits) - 1ull));
}
__device__ __forceinline__ bool kf_valid(const KfFormat &F, uint64_t raw)
{
    return ((raw >> F.valid_bit) & 1ull) != 0ull;
}
__device__ __forceinline__ uint64_t kf_addr(const KfFormat &F, uint64_t raw, uint32_t apc)
{
    return kf_fld(raw, F.addr_sel[apc] ? F.addr_sys : F.addr_local);
}
__device__ __forceinline__ uint64_t kf_big_addr(const KfFormat &F, uint64_t raw, uint32_t apc)
{
    return kf_fld(raw, F.addr_sel[apc] ? F.big_addr_sys : F.big_addr_local);
}
__device__ __forceinline__ uint32_t kf_leaf_flags(const KfFormat &F, uint64_t raw, uint32_t ps)
{
    uint32_t f = F.pte_ap_map[kf_ap_raw(F, raw)] & KFWR_RF_AP_MASK;
    if ((raw >> F.bit_volatile) & 1ull)       f |= KFWR_RF_VOLATILE;
    if ((raw >> F.bit_privilege) & 1ull)      f |= KFWR_RF_PRIVILEGE;
    if ((raw >> F.bit_read_only) & 1ull)      f |= KFWR_RF_READ_ONLY;
    if ((raw >> F.bit_atomic_disable) & 1ull) f |= KFWR_RF_ATOMIC_DISABLE;
    return f | ((ps & KFWR_RF_PS_MASK) << KFWR_RF_PS_SHIFT);
}
__device__ __forceinline__ uint64_t kf_ps_bytes_of(const KfFormat &F, uint32_t flags)
{
    return 1ull << F.ps_log2[(flags >> KFWR_RF_PS_SHIFT) & 3u];
}

/* ═══ THE SWITCH — for what field offsets cannot express ══════════════════════
 *
 * ★★★ It is WARP-UNIFORM. Every thread of a launch walks the same guest's tables
 * in the same format, so `F.table_version` is the same value in every lane and
 * the branch costs a predicate, not a divergence. Divergence is the usual
 * objection to branching in a CUDA kernel and **it does not apply here**. Do not
 * "optimise" this into a template or a second kernel: §21 is explicit that we
 * ship ONE program.
 *
 * ⚠⚠ THE VER3 ARMS HAVE NEVER RUN. There is no Hopper or Blackwell here, and
 * nothing in this tree has ever decoded a VER3 table. They are written from
 * `research_clones/ogkm` 610.43.02 `hopper/gh100/dev_mmu.h` and
 * `kern_gmmu_fmt_gh10x.c`, they are cited line by line, and they are **not**
 * reachable unless a caller passes `table_version = KF_TBL_VER3` — which
 * `kf_create` refuses unless `KF_ALLOW_UNTESTED_VER3` is defined. Treat them as a
 * SKETCH that proves the seam's shape, never as support.
 */

/* Is this directory entry a pointer to a sub-table at all? */
__device__ __forceinline__ bool kf_dir_present(const KfFormat &F, uint64_t raw,
                                               uint32_t apc, bool leaf_capable)
{
    switch (F.table_version) {
    case KF_TBL_VER2:
        /* The aperture IS the validity. `kern_gmmu_fmt_gm10x.c:165-182`. */
        return apc != F.pde_ap_invalid;
    default:
        /* ⚠ UNTESTED. VER3 gives a PDE its own VALID bit (`gh100/dev_mmu.h:417`),
         * but at a level that can also hold a PTE that same bit is IS_PTE
         * (`:414`) and is clear by the time we get here. */
        return (leaf_capable || kf_valid(F, raw)) && apc != F.pde_ap_invalid;
    }
}

/* Did the guest DECLARE this slot empty, as opposed to never having written it? */
__device__ __forceinline__ bool kf_slot_sparse(const KfFormat &F, uint64_t raw)
{
    switch (F.table_version) {
    case KF_TBL_VER2:
        /* "GM20X supports sparse directly in HW by setting the volatile bit when
         * the valid bit is clear" — `kern_gmmu_gm200.c:46-70`. A BIT TEST. */
        return ((raw >> F.bit_volatile) & 1ull) != 0ull;
    default:
        /* ⚠ UNTESTED. `NV_MMU_VER3_PTE_PCF_SPARSE = 1` (`gh100/dev_mmu.h:499`).
         * An EQUALITY against a multi-bit value — which is exactly why a field
         * descriptor cannot carry this and this switch exists. */
        return kf_field(raw, F.pcf.lo, F.pcf.bits, 0) == (uint64_t)F.pcf_sparse;
    }
}

/* ── per-thread walk context ─────────────────────────────────────────────────── */
struct KfCtx {
    KfWin w;
    const KfFormat *fmt;
    uint64_t visited;
    uint64_t budget;
    uint32_t refusals;
    uint32_t refuse;
    uint32_t stop;
    uint32_t sparse;
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
 * `bytes` and `align` are powers of two derived from the FORMAT DESCRIPTOR,
 * never from the guest. */
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
    if (gpga & (kf_ps_bytes_of(*c.fmt, flags) - 1ull)) { c.refuse |= KFWR_R_MISALIGNED_LEAF; c.refusals++; return; }
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

/* ═══ THE WALK ════════════════════════════════════════════════════════════════
 * I1: KF_DIRS + 1 literally nested loops, every trip count bounded at COMPILE
 * TIME by KF_MAX_ENT and only narrowed by the descriptor.
 */
#define KF_STEP_SKIP    0u
#define KF_STEP_DESCEND 1u
#define KF_STEP_STOP    2u

/* One directory slot's body, minus the control flow. Returns where to go next
 * and, on DESCEND, the child table's address. */
__device__ __forceinline__ uint32_t kf_dir_step(KfCtx &c, uint32_t k, uint64_t tbl,
                                                uint32_t idx, uint64_t va, uint64_t *child)
{
    const KfFormat &F = *c.fmt;
    const KfDir &L = F.dir[k];
    uint64_t raw;
    if (!kf_charge(c, 1)) return KF_STEP_STOP;
    if (!kf_load64(c, tbl + (uint64_t)idx * L.entry_bytes, &raw)) return KF_STEP_SKIP;
    if (L.leaf_ps != KF_PS_NONE && kf_valid(F, raw)) {
        kf_emit(c, va, kf_addr(F, raw, kf_ap_raw(F, raw)),
                1ull << F.ps_log2[L.leaf_ps], kf_leaf_flags(F, raw, L.leaf_ps));
        return KF_STEP_SKIP;
    }
    uint32_t apc = kf_ap_raw(F, raw);
    if (!kf_dir_present(F, raw, apc, L.leaf_ps != KF_PS_NONE)) {
        if (kf_slot_sparse(F, raw)) c.sparse++;
        return KF_STEP_SKIP;
    }
    if (F.pde_ap_map[apc] != KFWR_AP_VIDMEM) { c.refuse |= KFWR_R_FOREIGN_AP; c.refusals++; return KF_STEP_SKIP; }
    uint64_t next = kf_addr(F, raw, apc);
    if (next == 0ull) return KF_STEP_SKIP;
    uint64_t cb = (uint64_t)F.dir[k + 1].entries * F.dir[k + 1].entry_bytes;
    if (!kf_table_ok(c, next, cb, cb)) return KF_STEP_SKIP;
    *child = next;
    return KF_STEP_DESCEND;
}

__device__ void kf_walk_one(KfCtx &c, uint64_t pdb, const KfScopeSet &sc)
{
    const KfFormat &F = *c.fmt;
    const uint64_t root_bytes = (uint64_t)F.dir[F.first_dir].entries * F.dir[F.first_dir].entry_bytes;
    if (!kf_table_ok(c, pdb, root_bytes, F.root_align)) return;

    const uint32_t n0 = F.dir[0].active ? F.dir[0].entries : 1u;
    for (uint32_t i0 = 0; i0 < KF_MAX_ENT && i0 < n0 && !c.stop; i0++) {
        uint64_t t1 = pdb, va0 = 0ull;
        if (F.dir[0].active) {
            va0 = (uint64_t)i0 << F.dir[0].va_lo;
            uint64_t e0 = va0 + (1ull << F.dir[0].va_lo);
            if (!kf_in_scope(sc, va0, e0)) { kf_carry(c, va0, e0); continue; }
            uint32_t r = kf_dir_step(c, 0, pdb, i0, va0, &t1);
            if (r == KF_STEP_STOP) break;
            if (r != KF_STEP_DESCEND) continue;
        }

        const uint32_t n1 = F.dir[1].active ? F.dir[1].entries : 1u;
        for (uint32_t i1 = 0; i1 < KF_MAX_ENT && i1 < n1 && !c.stop; i1++) {
            uint64_t t2 = t1, va1 = va0;
            if (F.dir[1].active) {
                va1 = va0 | ((uint64_t)i1 << F.dir[1].va_lo);
                uint64_t e1 = va1 + (1ull << F.dir[1].va_lo);
                if (!kf_in_scope(sc, va1, e1)) { kf_carry(c, va1, e1); continue; }
                uint32_t r = kf_dir_step(c, 1, t1, i1, va1, &t2);
                if (r == KF_STEP_STOP) break;
                if (r != KF_STEP_DESCEND) continue;
            }

            const uint32_t n2 = F.dir[2].active ? F.dir[2].entries : 1u;
            for (uint32_t i2 = 0; i2 < KF_MAX_ENT && i2 < n2 && !c.stop; i2++) {
                uint64_t t3 = t2, va2 = va1;
                if (F.dir[2].active) {
                    va2 = va1 | ((uint64_t)i2 << F.dir[2].va_lo);
                    uint64_t e2 = va2 + (1ull << F.dir[2].va_lo);
                    if (!kf_in_scope(sc, va2, e2)) { kf_carry(c, va2, e2); continue; }
                    uint32_t r = kf_dir_step(c, 2, t2, i2, va2, &t3);
                    if (r == KF_STEP_STOP) break;
                    if (r != KF_STEP_DESCEND) continue;
                }

                const uint32_t n3 = F.dir[3].active ? F.dir[3].entries : 1u;
                for (uint32_t i3 = 0; i3 < KF_MAX_ENT && i3 < n3 && !c.stop; i3++) {
                    uint64_t t4 = t3, va3 = va2;
                    if (F.dir[3].active) {
                        va3 = va2 | ((uint64_t)i3 << F.dir[3].va_lo);
                        uint64_t e3 = va3 + (1ull << F.dir[3].va_lo);
                        if (!kf_in_scope(sc, va3, e3)) { kf_carry(c, va3, e3); continue; }
                        uint32_t r = kf_dir_step(c, 3, t3, i3, va3, &t4);
                        if (r == KF_STEP_STOP) break;
                        if (r != KF_STEP_DESCEND) continue;
                    }

                    /* ── the DUAL level: one entry, two sub-tables, one VA range ── */
                    const KfDir &D = F.dir[KF_DIRS - 1u];
                    for (uint32_t i4 = 0; i4 < KF_MAX_ENT && i4 < D.entries && !c.stop; i4++) {
                        const uint64_t va4 = va3 | ((uint64_t)i4 << D.va_lo);
                        uint64_t lo16, hi16;
                        if (!kf_charge(c, 1)) break;
                        if (!kf_load64(c, t4 + (uint64_t)i4 * D.entry_bytes, &lo16)) continue;
                        if (!kf_load64(c, t4 + (uint64_t)i4 * D.entry_bytes + 8u, &hi16)) continue;
                        /* A leaf at this level is spelled in the LOW half's valid
                         * bit, and it is asked FIRST — the order gmmuFmtEntryIsPte
                         * asks it in. */
                        if (D.leaf_ps != KF_PS_NONE && kf_valid(F, lo16)) {
                            kf_emit(c, va4, kf_addr(F, lo16, kf_ap_raw(F, lo16)),
                                    1ull << F.ps_log2[D.leaf_ps], kf_leaf_flags(F, lo16, D.leaf_ps));
                            continue;
                        }
                        uint32_t aps = kf_ap_raw(F, hi16);   /* SMALL half, HIGH word */
                        uint32_t apb = kf_ap_raw(F, lo16);   /* BIG half,   LOW word  */
                        uint64_t pts = 0ull, ptb = 0ull;
                        bool has_s = false, has_b = false;
                        const uint64_t sb = (uint64_t)F.small_entries * F.small_entry_bytes;
                        const uint64_t bb = (uint64_t)F.big_entries * F.big_entry_bytes;
                        if (kf_dir_present(F, hi16, aps, false)) {
                            if (F.pde_ap_map[aps] != KFWR_AP_VIDMEM) { c.refuse |= KFWR_R_FOREIGN_AP; c.refusals++; }
                            else {
                                pts = kf_addr(F, hi16, aps);
                                has_s = (pts != 0ull) && kf_table_ok(c, pts, sb, sb);
                            }
                        } else if (kf_slot_sparse(F, hi16)) c.sparse++;
                        if (kf_dir_present(F, lo16, apb, D.leaf_ps != KF_PS_NONE)) {
                            if (F.pde_ap_map[apb] != KFWR_AP_VIDMEM) { c.refuse |= KFWR_R_FOREIGN_AP; c.refusals++; }
                            else {
                                ptb = kf_big_addr(F, lo16, apb);
                                has_b = (ptb != 0ull) && kf_table_ok(c, ptb, bb, bb);
                            }
                        } else if (kf_slot_sparse(F, lo16)) c.sparse++;

                        /* Both sub-tables cover the SAME VA range at different page
                         * sizes, so they are interleaved at the BIG page's
                         * granularity to keep each page-size class ascending in VA
                         * — which is what the per-class diff needs. `ratio` is
                         * derived, not hardcoded. */
                        const uint32_t ratio = 1u << (F.big_va_lo - F.small_va_lo);
                        for (uint32_t b = 0; b < KF_MAX_ENT && b < F.big_entries && !c.stop; b++) {
                            if (has_b) {
                                uint64_t e;
                                if (!kf_charge(c, 1)) break;
                                if (kf_load64(c, ptb + (uint64_t)b * F.big_entry_bytes, &e)) {
                                    if (kf_valid(F, e))
                                        kf_emit(c, va4 | ((uint64_t)b << F.big_va_lo),
                                                kf_addr(F, e, kf_ap_raw(F, e)),
                                                1ull << F.ps_log2[F.big_ps],
                                                kf_leaf_flags(F, e, F.big_ps));
                                    else if (kf_slot_sparse(F, e)) c.sparse++;
                                }
                            }
                            if (has_s) {
                                for (uint32_t j = 0; j < 16u && j < ratio && !c.stop; j++) {
                                    uint32_t s = b * ratio + j;
                                    if (s >= F.small_entries) break;
                                    uint64_t e;
                                    if (!kf_charge(c, 1)) break;
                                    if (!kf_load64(c, pts + (uint64_t)s * F.small_entry_bytes, &e)) continue;
                                    if (kf_valid(F, e))
                                        kf_emit(c, va4 | ((uint64_t)s << F.small_va_lo),
                                                kf_addr(F, e, kf_ap_raw(F, e)),
                                                1ull << F.ps_log2[F.small_ps],
                                                kf_leaf_flags(F, e, F.small_ps));
                                    else if (kf_slot_sparse(F, e)) c.sparse++;
                                }
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
    d->sparse_slots = 0u;
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
    c.fmt = &a.fmt;
    c.visited = 0ull;
    c.sparse = 0u;
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
                    /* The granule is the LAST directory level's span — the
                     * deepest level whose subtrees the walk prunes — taken from
                     * the descriptor, not from a VER2 constant. */
                    const uint64_t gsh = a.fmt.dir[KF_DIRS - 2u].va_lo;
                    sc.lo[sc.n] = lo & ~((1ull << gsh) - 1ull);
                    sc.hi[sc.n] = (hi + ((1ull << gsh) - 1ull)) & ~((1ull << gsh) - 1ull);
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
    if (c.sparse) atomicAdd(&d->sparse_slots, c.sparse);
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

/* The page-size class of a run: 0 = 4 KiB … 3 = 512 MiB. */
__device__ __forceinline__ uint32_t kf_cls(const KfMapRun &r)
{
    return (r.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
}

/* The next index at or after `from` whose run is in class `cls`. */
__device__ __forceinline__ uint32_t kf_next_cls(const KfMapRun *a, uint32_t n, uint32_t from, uint32_t cls)
{
    while (from < n && kf_cls(a[from]) != cls) from++;
    return from;
}

/* One output run under construction. Segments are produced in ascending VA
 * within a class, so coalescing them is the same rule the walk uses. */
struct KfSeg {
    uint32_t have, op, flags;
    uint64_t va, gpga, len;
};

__device__ __forceinline__ void kf_seg_flush(KfOut &o, KfDev *d, KfSeg &s, uint16_t pi)
{
    if (!s.have) return;
    KfMapRun r;
    r.va = s.va; r.gpga = s.gpga; r.len = s.len; r.flags = s.flags;
    r.op = (uint16_t)s.op; r.pdb_index = pi;
    kf_put(o, d, r, s.op, pi);
    s.have = 0u;
}

__device__ __forceinline__ void kf_seg_emit(KfOut &o, KfDev *d, KfSeg &s, uint16_t pi,
                                            uint32_t op, uint64_t va, uint64_t gpga,
                                            uint64_t len, uint32_t flags)
{
    if (s.have && s.op == op && s.flags == flags &&
        s.va + s.len == va && s.gpga + s.len == gpga) {
        s.len += len;
        return;
    }
    kf_seg_flush(o, d, s, pi);
    s.have = 1u; s.op = op; s.va = va; s.gpga = gpga; s.len = len; s.flags = flags;
}

/* ⊘ KF_DROP_UNMAP drops every UNMAP and changes nothing else. The MAP/REMAP set
 * is unaffected, so the report stays order-independent -- and closure breaks,
 * because a mapping the guest removed lingers in the model for ever. Compiled in
 * only by `make check-closure-negative`: it is the known-positive for the
 * `apply(model, delta) != full walk` assertion itself, which the other two break
 * flags never reach (they trip the stronger order assertion first). */
#ifdef KF_DROP_UNMAP
#define KF_EMIT_UNMAP(o, d, s, pi, va, gp, len, fl) ((void)0)
#else
#define KF_EMIT_UNMAP(o, d, s, pi, va, gp, len, fl) \
    kf_seg_emit(o, d, s, pi, KFWR_OP_UNMAP, va, gp, len, fl)
#endif

/* ★★★★★ THE DELTA, PER PAGE-SIZE CLASS, AT SEGMENT GRANULARITY.
 *
 * ⊘ The previous shape — one merge join over the combined list, comparing whole
 * runs on the key (va, page size) — produced deltas that were individually
 * plausible and did NOT reconstruct. A cur run covering several prev runs
 * emitted one REMAP followed by UNMAPs of the runs it had just replaced, so
 * applying the report in order LOST those mappings. It was found by the
 * round-trip closure test, not by any of the nine single-step delta cases,
 * because every one of those changes exactly one run.
 *
 * The shape below cannot express that. Within one class, prev and cur are each
 * sorted, disjoint interval sets, and the walk is compared to them SEGMENT by
 * segment:
 *
 *   prev covers, cur does not  ⇒ UNMAP
 *   cur covers, prev does not  ⇒ MAP
 *   both, and they differ      ⇒ REMAP
 *   both, and they agree       ⇒ nothing (and the output run breaks)
 *
 * ⇒ **The UNMAP set and the MAP/REMAP set are disjoint in (va, class) by
 * construction**, so the order a host applies the runs in cannot matter. That is
 * a much stronger property than "apply them in the order given", and it is the
 * one the round-trip test actually checks.
 *
 * ⚠ Classes are processed separately because a 4 KiB and a 64 KiB leaf can
 * describe the SAME virtual address (both halves of a dual PDE). They are
 * different mappings at one VA, so they must be diffed against their own kind;
 * a single interleaved pass would treat one as replacing the other.
 */
__device__ void kf_diff_class(KfOut &o, KfDev *d, uint16_t pi, uint32_t cls,
                              const KfMapRun *pr, uint32_t pn,
                              const KfMapRun *cr, uint32_t cn)
{
    KfSeg s; s.have = 0u; s.op = 0u; s.flags = 0u; s.va = 0ull; s.gpga = 0ull; s.len = 0ull;
    uint32_t p = kf_next_cls(pr, pn, 0u, cls);
    uint32_t q = kf_next_cls(cr, cn, 0u, cls);
    uint64_t pv = 0, pg = 0, pl = 0, cv = 0, cg = 0, cl = 0;
    uint32_t pf = 0, cf = 0, ph = 0, ch = 0;

    /* A fixed trip count over OUR OWN counts: every iteration either finishes a
     * run on one side or splits one, and a split's boundary is the other side's
     * edge, so 3*(pn+cn) bounds it. Tripping the guard is loud. */
    const uint32_t guard_max = 4u * (pn + cn) + 8u;
    uint32_t guard = 0u;
    for (; guard < guard_max; guard++) {
        if (!ph && p < pn) { pv = pr[p].va; pg = pr[p].gpga; pl = pr[p].len; pf = pr[p].flags; ph = 1u; }
        if (!ch && q < cn) { cv = cr[q].va; cg = cr[q].gpga; cl = cr[q].len; cf = cr[q].flags; ch = 1u; }
        if (!ph && !ch) break;
        if (!ch) {
            KF_EMIT_UNMAP(o, d, s, pi, pv, pg, pl, pf);
            ph = 0u; p = kf_next_cls(pr, pn, p + 1u, cls); continue;
        }
        if (!ph) {
            kf_seg_emit(o, d, s, pi, KFWR_OP_MAP, cv, cg, cl, cf);
            ch = 0u; q = kf_next_cls(cr, cn, q + 1u, cls); continue;
        }
        if (pv + pl <= cv) {
            KF_EMIT_UNMAP(o, d, s, pi, pv, pg, pl, pf);
            ph = 0u; p = kf_next_cls(pr, pn, p + 1u, cls); continue;
        }
        if (cv + cl <= pv) {
            kf_seg_emit(o, d, s, pi, KFWR_OP_MAP, cv, cg, cl, cf);
            ch = 0u; q = kf_next_cls(cr, cn, q + 1u, cls); continue;
        }
        if (pv < cv) {                       /* prev-only head */
            uint64_t n = cv - pv;
            KF_EMIT_UNMAP(o, d, s, pi, pv, pg, n, pf);
            pv += n; pg += n; pl -= n; continue;
        }
        if (cv < pv) {                       /* cur-only head */
            uint64_t n = pv - cv;
            kf_seg_emit(o, d, s, pi, KFWR_OP_MAP, cv, cg, n, cf);
            cv += n; cg += n; cl -= n; continue;
        }
        {                                    /* both cover [pv, pv+n) */
            uint64_t n = pl < cl ? pl : cl;
            if (pg != cg || pf != cf) kf_seg_emit(o, d, s, pi, KFWR_OP_REMAP, cv, cg, n, cf);
            else kf_seg_flush(o, d, s, pi);  /* unchanged: nothing, and the run breaks */
            pv += n; pg += n; pl -= n;
            cv += n; cg += n; cl -= n;
            if (!pl) { ph = 0u; p = kf_next_cls(pr, pn, p + 1u, cls); }
            if (!cl) { ch = 0u; q = kf_next_cls(cr, cn, q + 1u, cls); }
            continue;
        }
    }
    if (guard >= guard_max) { d->refuse_mask |= KFWR_R_DELTA_CAP; d->refusals++; o.trunc = 1u; }
    kf_seg_flush(o, d, s, pi);
}

#ifdef KF_OLD_MERGE
/* ⊘⊘ THE SUPERSEDED DIFF, compiled in ONLY by `make check-negative`.
 *
 * It merge-joins WHOLE RUNS on the key (va, page size). Every one of the nine
 * single-step delta cases passes against it, and it does not satisfy closure: a
 * cur run covering several prev runs emits one REMAP and then UNMAPs of the runs
 * it just replaced. It is kept so that "the round-trip test would have caught
 * the old diff" is a MEASUREMENT rather than a claim. */
__device__ __forceinline__ int kf_key_cmp(const KfMapRun &x, const KfMapRun &y)
{
    if (x.va != y.va) return x.va < y.va ? -1 : 1;
    uint32_t px = (x.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
    uint32_t py = (y.flags >> KFWR_RF_PS_SHIFT) & KFWR_RF_PS_MASK;
    if (px != py) return px > py ? -1 : 1;
    return 0;
}

__device__ void kf_diff_old(KfOut &o, KfDev *d, uint16_t pi,
                            const KfMapRun *pr, uint32_t pn,
                            const KfMapRun *cr, uint32_t cn)
{
    uint32_t p = 0u, q = 0u;
    while (p < pn || q < cn) {
        if (q >= cn) { kf_put(o, d, pr[p], KFWR_OP_UNMAP, pi); p++; continue; }
        if (p >= pn) { kf_put(o, d, cr[q], KFWR_OP_MAP,   pi); q++; continue; }
        int k = kf_key_cmp(pr[p], cr[q]);
        if (k < 0)   { kf_put(o, d, pr[p], KFWR_OP_UNMAP, pi); p++; continue; }
        if (k > 0)   { kf_put(o, d, cr[q], KFWR_OP_MAP,   pi); q++; continue; }
        KfMapRun P = pr[p], C = cr[q];
        if (P.gpga == C.gpga && P.len == C.len && P.flags == C.flags) {
            /* unchanged */
        } else if (C.len >= P.len) {
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
}
#endif

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
            /* Per class, so a RESYNC report carries the SAME ordering rule as a
             * delta: grouped by page-size class, ascending within a class. */
            for (uint32_t cls = 0u; cls < 4u; cls++)
                for (uint32_t k = kf_next_cls(cr, cn, 0u, cls); k < cn;
                     k = kf_next_cls(cr, cn, k + 1u, cls))
                    kf_put(o, d, cr[k], KFWR_OP_MAP, pi);
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
#ifdef KF_OLD_MERGE
            kf_diff_old(o, d, pi, pr, pn, cr, cn);
#else
            for (uint32_t cls = 0u; cls < 4u; cls++)
                kf_diff_class(o, d, pi, cls, pr, pn, cr, cn);
#endif
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
    h.sparse_slots = d->sparse_slots;
    for (uint32_t i = 0; i < 4u; i++) h.ps_log2[i] = a.fmt.ps_log2[i];
    *a.hdr = h;
}

__global__ void kf_ack_kernel(KfDev *d, uint64_t g) { d->acked = g; }

/* ── the format descriptors, built on the HOST ───────────────────────────────
 *
 * ⊘ These are the ONLY places in this file where a bit position is written down,
 * and they are host code that runs once. Everything below the seam reads them.
 * In production they come from the Rust `GmmuFmt` impls the tree already
 * maintains (`kayfabe-chips`), uploaded as setup data; here they are literals so
 * the proving ground has no Rust dependency.
 */
static KfField kf_f(uint8_t lo, uint8_t bits, uint8_t shift)
{
    KfField f;
    f.lo = lo; f.bits = bits; f.shift = shift; f.pad = 0;
    return f;
}
static void kf_set_dir(KfDir *d, int active, uint8_t va_lo, uint8_t va_hi,
                       uint8_t entry_bytes, uint8_t leaf_ps)
{
    d->active = (uint8_t)active;
    d->va_lo = va_lo;
    d->entry_bytes = entry_bytes;
    d->leaf_ps = leaf_ps;
    d->entries = (uint16_t)(1u << (va_hi - va_lo + 1u));
    d->pad = 0;
}

/* GA10x / VER2 — `ogkm-580 pascal/gp100/dev_mmu.h` + `kern_gmmu_fmt_ga10x.c`.
 * THE TESTED ONE. Agrees with `kayfabe-chips::ga10x::Ga10xGmmu`. */
static KfFormat kf_format_ver2(void)
{
    KfFormat F;
    memset(&F, 0, sizeof(F));
    F.abi_version = KF_ABI_VERSION;
    F.table_version = KF_TBL_VER2;
    /* PD3 [48:47] → PD2 [46:38] → PD1 [37:29] (or a 512 MiB page)
     *   → PD0 [28:21], a 16-byte DUAL entry (or a 2 MiB page)
     *        ↙ PT_BIG [20:16]        ↘ PT_SMALL [20:12] */
    kf_set_dir(&F.dir[0], 0, 0, 0, 8, KF_PS_NONE);          /* VER2 has no PD4 */
    kf_set_dir(&F.dir[1], 1, 47, 48, 8, KF_PS_NONE);
    kf_set_dir(&F.dir[2], 1, 38, 46, 8, KF_PS_NONE);
    kf_set_dir(&F.dir[3], 1, 29, 37, 8, (uint8_t)KFWR_PS_512M);
    kf_set_dir(&F.dir[4], 1, 21, 28, 16, (uint8_t)KFWR_PS_2M);
    F.big_va_lo = 16;   F.big_entries = 32;    F.big_entry_bytes = 8;  F.big_ps = KFWR_PS_64K;
    F.small_va_lo = 12; F.small_entries = 512; F.small_entry_bytes = 8; F.small_ps = KFWR_PS_4K;
    F.root_align = 4096u;
    F.first_dir = 1;
    F.valid_bit = 0;                     /* NV_MMU_VER2_PTE_VALID 0:0   */
    F.ap_lo = 1; F.ap_bits = 2;          /* _APERTURE 2:1               */
    F.pde_ap_invalid = 0;                /* _PDE_APERTURE_INVALID       */
    F.pte_ap_map[0] = KFWR_AP_VIDMEM;  F.pte_ap_map[1] = KFWR_AP_PEER;
    F.pte_ap_map[2] = KFWR_AP_SYSCOH;  F.pte_ap_map[3] = KFWR_AP_SYSNONCOH;
    F.pde_ap_map[0] = 0xFF;            F.pde_ap_map[1] = KFWR_AP_VIDMEM;
    F.pde_ap_map[2] = KFWR_AP_SYSCOH;  F.pde_ap_map[3] = KFWR_AP_SYSNONCOH;
    F.addr_sel[0] = 0; F.addr_sel[1] = 0; F.addr_sel[2] = 1; F.addr_sel[3] = 1;
    F.addr_local     = kf_f(8, 25, 12);  /* _ADDRESS_VID  32:8,  SHIFT 12 */
    F.addr_sys       = kf_f(8, 46, 12);  /* _ADDRESS_SYS  53:8,  SHIFT 12 */
    F.big_addr_local = kf_f(4, 29, 8);   /* _DUAL_PDE_ADDRESS_BIG_VID 32:4, SHIFT 8 */
    F.big_addr_sys   = kf_f(4, 50, 8);   /* _DUAL_PDE_ADDRESS_BIG_SYS 53:4, SHIFT 8 */
    F.bit_volatile = 3; F.bit_privilege = 5; F.bit_read_only = 6; F.bit_atomic_disable = 7;
    F.pcf = kf_f(0, 0, 0);               /* VER2 has none */
    F.pcf_sparse = 0;
    F.ps_log2[KFWR_PS_4K] = 12; F.ps_log2[KFWR_PS_64K] = 16;
    F.ps_log2[KFWR_PS_2M] = 21; F.ps_log2[KFWR_PS_512M] = 29;
#ifdef KF_BAD_DESCRIPTOR
    F.addr_local.lo = (uint8_t)(F.addr_local.lo + 1u);   /* one bit wrong, on purpose */
#endif
#ifdef KF_BAD_GEOMETRY
    F.dir[3].va_lo = (uint8_t)(F.dir[3].va_lo - 1u);     /* one level mis-strided */
#endif
#ifdef KF_BAD_ABI
    F.abi_version = KF_ABI_VERSION + 1u;                 /* a host/PTX skew */
#endif
    return F;
}

/* ⚠⚠⚠ GH100 / VER3 — A SKETCH THAT HAS NEVER RUN.
 *
 * Read off `research_clones/ogkm` 610.43.02 `hopper/gh100/dev_mmu.h:413-536` and
 * `kern_gmmu_fmt_gh10x.c:33-115`. There is no Hopper or Blackwell in this
 * project; nothing here has ever decoded a VER3 table; no test exercises it. It
 * exists to show that adding a format is **a descriptor plus two switch arms and
 * nothing else**, and `kf_create` REFUSES it unless KF_ALLOW_UNTESTED_VER3 is
 * defined, so it cannot be reached by accident and cannot make anything LOOK
 * supported.
 *
 *  PD4[56] → PD3[55:47] → PD2[46:38] → PD1[37:29] (or 512 MiB)
 *    → PD0[28:21] dual (or 2 MiB) ↙ PT_BIG[20:16]  ↘ PT_SMALL[20:12]
 *
 * ★ Note how little differs: one extra directory on top, ONE address field
 * instead of the vid/sys pair, and the four permission bits MOVED (PCF's low
 * four) rather than re-encoded. Only SPARSE is a different KIND of test.
 */
static KfFormat kf_format_ver3_untested(void)
{
    KfFormat F;
    memset(&F, 0, sizeof(F));
    F.abi_version = KF_ABI_VERSION;
    F.table_version = KF_TBL_VER3;
    kf_set_dir(&F.dir[0], 1, 56, 56, 8, KF_PS_NONE);        /* PD4, two entries */
    kf_set_dir(&F.dir[1], 1, 47, 55, 8, KF_PS_NONE);
    kf_set_dir(&F.dir[2], 1, 38, 46, 8, KF_PS_NONE);
    kf_set_dir(&F.dir[3], 1, 29, 37, 8, (uint8_t)KFWR_PS_512M);
    kf_set_dir(&F.dir[4], 1, 21, 28, 16, (uint8_t)KFWR_PS_2M);
    F.big_va_lo = 16;   F.big_entries = 32;    F.big_entry_bytes = 8;  F.big_ps = KFWR_PS_64K;
    F.small_va_lo = 12; F.small_entries = 512; F.small_entry_bytes = 8; F.small_ps = KFWR_PS_4K;
    F.root_align = 4096u;
    F.first_dir = 0;
    F.valid_bit = 0;                     /* _PTE_VALID / _PDE_VALID / _IS_PTE 0:0 */
    F.ap_lo = 1; F.ap_bits = 2;          /* _APERTURE 2:1, and _SMALL 66:65 = hi 2:1 */
    F.pde_ap_invalid = 0;
    F.pte_ap_map[0] = KFWR_AP_VIDMEM;  F.pte_ap_map[1] = KFWR_AP_PEER;
    F.pte_ap_map[2] = KFWR_AP_SYSCOH;  F.pte_ap_map[3] = KFWR_AP_SYSNONCOH;
    F.pde_ap_map[0] = 0xFF;            F.pde_ap_map[1] = KFWR_AP_VIDMEM;
    F.pde_ap_map[2] = KFWR_AP_SYSCOH;  F.pde_ap_map[3] = KFWR_AP_SYSNONCOH;
    /* ONE address field: no vid/sys split on this regime. */
    F.addr_sel[0] = 0; F.addr_sel[1] = 0; F.addr_sel[2] = 0; F.addr_sel[3] = 0;
    F.addr_local = F.addr_sys = kf_f(12, 40, 12);   /* _PTE_ADDRESS / _PDE_ADDRESS 51:12,
                                                     * and _DUAL_PDE_ADDRESS_SMALL 115:76
                                                     * = hi word 51:12 — the same spec */
    F.big_addr_local = F.big_addr_sys = kf_f(8, 44, 8);  /* _DUAL_PDE_ADDRESS_BIG 51:8, SHIFT 8 */
    /* PCF 7:3, and its low four enumerant bits are the four flags:
     * REGULAR_RW_ATOMIC_CACHED=0, _UNCACHED=1, PRIVILEGE_=2, _RO_=4, _NO_ATOMIC_=8. */
    F.bit_volatile = 3; F.bit_privilege = 4; F.bit_read_only = 5; F.bit_atomic_disable = 6;
    F.pcf = kf_f(3, 5, 0);
    F.pcf_sparse = 1;                    /* NV_MMU_VER3_PTE_PCF_SPARSE */
    F.ps_log2[KFWR_PS_4K] = 12; F.ps_log2[KFWR_PS_64K] = 16;
    F.ps_log2[KFWR_PS_2M] = 21; F.ps_log2[KFWR_PS_512M] = 29;
    return F;
}

static bool kf_pow2(uint64_t x) { return x != 0ull && (x & (x - 1ull)) == 0ull; }

/* ⚠ REFUSED, LOUDLY, AT LAUNCH. A descriptor the kernel's compile-time bounds
 * cannot hold must stop the walker being created, not be discovered halfway down
 * a tree as a page-table bug. */
static const char *kf_format_check(const KfFormat &F)
{
    if (F.abi_version != KF_ABI_VERSION) return "abi_version";
    if (F.table_version != KF_TBL_VER2 && F.table_version != KF_TBL_VER3) return "table_version";
#ifndef KF_ALLOW_UNTESTED_VER3
    if (F.table_version == KF_TBL_VER3) return "VER3 is a sketch that has never run; "
                                               "define KF_ALLOW_UNTESTED_VER3 to reach it";
#endif
    if (F.first_dir >= KF_DIRS) return "first_dir";
    if (!F.dir[KF_DIRS - 1].active) return "the deepest directory slot must be active";
    for (uint32_t k = 0; k < KF_DIRS; k++) {
        if (!F.dir[k].active) {
            if (k >= F.first_dir) return "an inactive slot below first_dir";
            continue;
        }
        if (k < F.first_dir) return "an active slot above first_dir";
        if (F.dir[k].entries == 0 || F.dir[k].entries > KF_MAX_ENT) return "level fan-out exceeds KF_MAX_ENT";
        if (!kf_pow2(F.dir[k].entries)) return "level fan-out is not a power of two";
        if (F.dir[k].entry_bytes != 8 && F.dir[k].entry_bytes != 16) return "entry_bytes";
        if (F.dir[k].va_lo >= 64) return "va_lo";
        if (!kf_pow2((uint64_t)F.dir[k].entries * F.dir[k].entry_bytes)) return "table bytes not a power of two";
        if (F.dir[k].leaf_ps != KF_PS_NONE && F.dir[k].leaf_ps > 3) return "leaf_ps";
    }
    if (F.dir[KF_DIRS - 1].entry_bytes != 16) return "the deepest directory must carry a dual entry";
    if (F.small_entries == 0 || F.small_entries > KF_MAX_ENT) return "small_entries";
    if (F.big_entries == 0 || F.big_entries > KF_MAX_ENT) return "big_entries";
    if (F.big_va_lo <= F.small_va_lo || F.big_va_lo - F.small_va_lo > 4) return "big/small stride";
    if ((uint32_t)F.big_entries << (F.big_va_lo - F.small_va_lo) != F.small_entries)
        return "the big and small tables do not cover the same VA range";
    if (!kf_pow2((uint64_t)F.small_entries * F.small_entry_bytes)) return "small table bytes";
    if (!kf_pow2((uint64_t)F.big_entries * F.big_entry_bytes)) return "big table bytes";
    if (!kf_pow2(F.root_align)) return "root_align";
    if (F.ap_bits == 0 || F.ap_bits > 2) return "ap_bits";
    for (uint32_t i = 0; i < 4; i++)
        if (F.ps_log2[i] < 12 || F.ps_log2[i] > 40) return "ps_log2";
    return NULL;
}

/* ── host side ───────────────────────────────────────────────────────────────── */
struct KfWalk {
    KfDev *dev;
    KfFormat fmt;
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
    uint32_t tv = cfg->table_version ? cfg->table_version : KF_TBL_VER2;
    w->fmt = (tv == KF_TBL_VER3) ? kf_format_ver3_untested() : kf_format_ver2();
    if (tv != KF_TBL_VER2 && tv != KF_TBL_VER3) { w->fmt.table_version = tv; }
    const char *bad = kf_format_check(w->fmt);
    if (bad) {
        fprintf(stderr, "kf_create: format descriptor REFUSED: %s\n", bad);
        free(w);
        return NULL;
    }
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
    if (!w) { fprintf(stderr, "kf_refresh: no walker (the format descriptor was refused)\n"); return -3; }
    if (npdb == 0u) { fprintf(stderr, "kf_refresh: npdb == 0\n"); return -2; }
    if (npdb > w->cfg.max_pdbs) { fprintf(stderr, "kf_refresh: npdb %u > max_pdbs %u\n", npdb, w->cfg.max_pdbs); return -2; }
    if (nscope > KF_MAX_SCOPE)  { fprintf(stderr, "kf_refresh: nscope too large\n"); return -2; }

    KF_CU_I(cudaMemcpy(w->pdbs, pdbs, (size_t)npdb * sizeof(uint64_t), cudaMemcpyHostToDevice));
    if (nscope) KF_CU_I(cudaMemcpy(w->scopes, scopes, (size_t)nscope * sizeof(KfScope), cudaMemcpyHostToDevice));

    KfArgs a;
    a.win.base = (const uint8_t *)gpga_dev;
    a.win.len = gpga_len;
    a.fmt = w->fmt;
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
    /* ★ From the REPORT, not from a constant: the header carries ps_log2 so the
     * host's parser needs no format-version knowledge. */
    uint64_t ps_bytes[4];
    for (uint32_t i = 0; i < 4u; i++) {
        if (h->ps_log2[i] < 12 || h->ps_log2[i] > 40) { msg = "ps_log2"; rc = -15; goto out; }
        ps_bytes[i] = 1ull << h->ps_log2[i];
    }
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
