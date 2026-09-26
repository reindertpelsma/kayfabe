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

/* ★★★ `KF_DEVICE_ONLY` — COMPILE JUST THE HALF NVRTC CAN SEE, so the PTX kayfabe
 * ships is built from THIS file rather than from a copy of it.
 *
 * `THE_CONSTRAINTS.md` §20: *"it is not code injection: the PTX is ours, built at
 * build time"*. Building it needs a CUDA front end for the device half and nothing
 * for the host half, and NVRTC is the one front end that runs with **no GPU and no
 * nvcc** — which is what lets the PTX be generated where the rest of the code is
 * generated instead of on a rented box.
 *
 * ⊘ The guard is the ONLY change, and it is a compile-time seam rather than a
 * refactor: with the macro undefined this file is byte-for-byte the program the
 * 58/58 suite grades. ⚠ Slicing the file by LINE NUMBER was the alternative and it
 * would have rotted on the first edit above the seam, silently producing PTX for a
 * different program than the one under test.
 */
#ifndef KF_DEVICE_ONLY
#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#endif

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

    /* ★ The PTE's KIND, which joins RUN IDENTITY. VER2: 63:56. VER3: 11:8. A
     * moved field, so the descriptor carries it and no switch arm is needed. */
    KfField  kind;

    uint8_t  ps_log2[4];        /* page-size code -> log2(bytes) */
};

/* ═══ I2: the ONE expression in this file that dereferences the GPGA buffer ═══ */
/* `len` bounds what we may READ; `span` bounds where a leaf may POINT. §39(c). */
struct KfWin { const uint8_t *base; uint64_t len; uint64_t span; };
#define KF_GPGA_DEREF(win, off) (*(const volatile uint64_t *)((win).base + (off)))

/* ── the kernel's cross-refresh state ──────────────────────────────────────────
 * ⊘ No snapshot of the guest's tables lives here (V3_BUILD.md). What persists
 * across walks is the committed placements (`KfArgs::com`, per slot, written only
 * on the host's ack) and the report counter. */
struct KfDev {
    uint64_t generation;         /* reports emitted */
    uint64_t committed;          /* the last report generation whose ack was committed */
    uint32_t runs_per_pdb;       /* the LARGEST walk_cap/slot_cap a layout may name (KfLayout) */
    uint32_t entry_budget;
    uint32_t run_capacity;       /* report */
    uint32_t pdb_capacity;       /* report */
    uint32_t max_pdbs;           /* walk entries */
    uint32_t max_slots;          /* committed-placement slots */
    uint32_t tbl_run_count[KF_MAX_PDB];   /* the walk's runs, per entry */
    uint32_t diff_count[KF_MAX_PDB];      /* the staged diff's runs, per entry */
    uint32_t diff_vflags[KF_MAX_PDB];     /* KFWR_V_PARTIAL / KFWR_V_OVERFLOW, per entry */
    /* ★ owner ruling 2026-09-25: which refusals fired IN EACH ENTRY'S walk — the host fails
     * that space by name (its refused leaves are absent from the walk, not "unmapped"). */
    uint32_t entry_refuse[KF_MAX_PDB];
    /* per-refresh accumulators, zeroed by kf_begin_kernel */
    unsigned long long entries_visited;
    unsigned int refusals;
    unsigned int refuse_mask;
    unsigned int hdr_flags;
    unsigned int walk_trunc;
    unsigned int walk_abort;   /* the walk itself stopped: budget or frontier cap */
    unsigned int sparse_slots;
    /* ★ w829: per entry, the capacity it NEEDED — the walk's UNCAPPED run count, or, for a
     * diff that could not fit its slot (PARTIAL/OVERFLOW), placements + maps. The host sizes
     * the next walk / grows the slot from it (KfLayout). */
    uint32_t need[KF_MAX_PDB];
};

/* ⚠ Scratch: `scratch` holds 4 * runs_per_pdb runs PER ENTRY (the diff: the
 * class-partitioned walk + up to 3 * runs_per_pdb staged runs; the commit: kept,
 * added, merged) and `iscratch` 3 * runs_per_pdb words per entry. The host sizes
 * both (and reuses the parallel walk's run stage for `scratch`: the walk is done
 * with it before the diff runs, and the commit runs before the walk). */
struct KfArgs {
    KfWin win;
    KfFormat fmt;                /* SETUP DATA: immutable for the VM's lifetime */
    KfDev *dev;
    KfMapRun *walk;              /* the walk's runs: runs_per_pdb per entry */
    KfMapRun *com;               /* committed placements: runs_per_pdb per slot */
    KfSlot *slot;                /* per slot: committed counts per class */
    const uint64_t *pdbs;        /* per entry: the root walked */
    const uint32_t *slots;       /* per entry: the slot it is diffed against */
    uint32_t npdb;
    const KfAck *ack;            /* the host's verdict on the PREVIOUS report */
    const uint8_t *ack_code;     /* one KFWR_ACK_* per previous report run */
    KfMapRun *scratch;
    uint32_t *iscratch;
    KfReportHeader *hdr;
    KfPdbEntry *rpdb;
    KfMapRun *rrun;
    const KfLayout *lay;         /* ★ w829: per-entry and per-slot capacity (host-managed) */
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
    /* KIND joins run identity: see kf_walk.h. Folding it into `flags` here is
     * what makes the coalescer honour it without a rule of its own. */
    f |= ((uint32_t)kf_fld(raw, F.kind) & KFWR_RF_KIND_MASK) << KFWR_RF_KIND_SHIFT;
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
        /* ★ VER3 — THE APERTURE IS THE VALIDITY HERE TOO. `[measured w826 kf-gate7]` the
         * sketch this replaced required bit 0 (`leaf_capable || VALID`) and found ZERO runs:
         * RM's own VER3 PDE format has NO valid field at any level — only aperture, PCF
         * and address (`ogkm-580 kern_gmmu_fmt_gh10x.c:132-158`; `fldValid` is PTE-only,
         * `:169`) — and the dual PDE's SMALL half has no bit 0 at all (its fields are
         * `66:65`, `69:67`, `115:76`). Bit 0 of a PDE is `IS_PTE`, and a leaf-capable
         * level's PTEs are taken BEFORE this test (`kf_valid` on the leaf branch). */
        (void)leaf_capable;
        return apc != F.pde_ap_invalid;
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

/* ★★★★★ THE BIG HALF CAN VETO THE SMALL ONE (w760h, from ogkm's own checks).
 * Two encodings of a dual PDE's LOW word, both meaning "there is nothing below
 * me -- do not look at the 4 KiB table", and we honoured NEITHER:
 *
 *   SPARSE    VALID=0 VOL=1.  ogkm `_gmmuIsInvalidPdeOk` (gmmu_trace.c:429-466)
 *             returns NV_FALSE for sublevel 0 -- THE BIG HALF -- when sparse, and
 *             its caller (mmu_trace.c:552-558) turns that into
 *             NV_ERR_INVALID_XLATE and LEAVES the walk; not `continue`. So a
 *             sparse big half aborts translation for the whole 2 MiB WITHOUT
 *             EVER READING sublevel 1.
 *
 *   UNMAPPED  VALID=0 VOL=0 PRIVILEGE=1 (`0x20` on GA10x). uvm_mmu.h:203-212:
 *             "Unmapped big PTEs indicate that there are no 4k PTEs below the
 *             unmapped big entry, so MMU should stop its walk and not cache any
 *             4k entries which may be in memory". ⊘⊘⊘ AND THIS ONE IS NOT
 *             HOSTILE INPUT: uvm_va_block.c:6484-6492 unmaps 64 KiB of VA by
 *             writing exactly this and DELIBERATELY LEAVING THE 4 KiB PTEs
 *             STALE -- "we only need to invalidate the 4k PTEs without actually
 *             writing them". An honest, stock guest produces it on every such
 *             unmap, and we were reporting the stale 4 KiB leaves as live
 *             mappings AFTER the guest asked for them to be gone.
 *
 * ⚠ VER2 only: VER3 spells both in PCF and its decode is a sketch that has never
 * run, so claiming to implement this there would be a lie (see the format seam).
 * ⚠ `lo16` is known VALID-clear here -- a valid low word is a 2 MiB leaf and is
 * handled before this is reached. */
/* ⊘⊘⊘ w826 — CORRECTED: THE VETO IS PER 64 KiB SLOT, AND IT IS A BIG **PTE**, NOT THE PDE.
 * The citations above describe a big PTE (an entry INSIDE the big-page table): uvm_mmu.h
 * "unmapped big PTEs indicate that there are no 4k PTEs below the unmapped big ENTRY", and
 * uvm_va_block.c unmaps ONE 64 KiB by writing it. The old code applied the test to the
 * DUAL PDE's low word, where bit 5 is not PRIVILEGE but part of ADDRESS_BIG
 * (ogkm-580 pascal/gp100/dev_mmu.h:102, `(35-3):(8-4)`), and dropped the WHOLE 2 MiB small
 * table. `[measured w826 ct4]` a guest 4 KiB operand leaf present on one walk was gone on
 * the next beside a big-page region, reconcile unmapped it, and the copy did nothing.
 * Now: an UNMAPPED big PTE (VALID=0 VOL=0 PRIV=1) hides only the small PTEs of its slot. */
__device__ __forceinline__ bool kf_big_pte_unmapped(const KfFormat &F, uint64_t e)
{
    if (F.table_version != KF_TBL_VER2) return false;
    if (kf_valid(F, e) || kf_slot_sparse(F, e)) return false;
    return ((e >> F.bit_privilege) & 1ull) != 0ull;
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
    uint16_t pdb_index;
};

/* I2: the bounds check, and the single dereference it guards. */
/* ★★★ THE PRIMITIVE. Every path — serial and parallel — reaches GPGA through
 * exactly this function, so I2 stays a property of the source text however many
 * kernels there are. ⊘ The load stays `volatile`: the race/* cases depend on each
 * dereference being a real read of current memory, and the parallel walk gets its
 * memory-level parallelism from THREADS rather than from letting the compiler
 * batch loads, so nothing here had to be given up for speed. */
__device__ __forceinline__ bool kf_win_load(const KfWin &w, uint64_t off, uint64_t *v)
{
#ifndef KF_BREAK_BOUNDS
    if (off > w.len || 8ull > w.len - off) return false;
#endif
    *v = KF_GPGA_DEREF(w, off);
    return true;
}

__device__ __forceinline__ bool kf_win_table_ok(const KfWin &w, uint64_t phys, uint64_t bytes, uint64_t align)
{
#ifndef KF_BREAK_BOUNDS
    if (phys & (align - 1ull)) return false;
    if (phys > w.len || bytes > w.len - phys) return false;
#endif
    return true;
}

__device__ __forceinline__ bool kf_load64(KfCtx &c, uint64_t off, uint64_t *v)
{
    if (!kf_win_load(c.w, off, v)) { c.refuse |= KFWR_R_OOB; c.refusals++; return false; }
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
#ifndef KF_BREAK_BOUNDS
    /* §39(c) THE CONTAINMENT CHECK. `gpga` and `len` are already COPIES in
     * registers -- `gpga` came out of kf_win_load into a local and `len` is a
     * format constant -- so the guest cannot change either between this test and
     * the emit below (§39(a)). Overflow-safe, and BEFORE the coalesce branch so
     * an extension inherits a checked base. */
    /* ★ w825 — THE BOUND IS PER APERTURE. `span` bounds VIDMEM leaves (GPGA offsets). A
     * system-memory leaf names a guest-PHYSICAL address in guest RAM, which the host bounds
     * against the ONE guest-RAM object and the VMM's own layout before anything is mapped;
     * bounding it by the vidmem span refused every sysmem leaf above ~span. PEER has no
     * meaning for a single-GPU guest and is refused here. */
    {
        uint32_t ap = (flags >> KFWR_RF_AP_SHIFT) & KFWR_RF_AP_MASK;
        if (ap == KFWR_AP_PEER) { c.refuse |= KFWR_R_LEAF_OOB; c.refusals++; return; }
        if (ap == KFWR_AP_VIDMEM && (gpga > c.w.span || len > c.w.span - gpga)) { c.refuse |= KFWR_R_LEAF_OOB; c.refusals++; return; }
    }
#endif
#ifndef KF_BREAK_COALESCE
    if (c.have && c.run.flags == flags &&
        c.run.va + c.run.len == va && c.run.gpga + c.run.len == gpga) {
        c.run.len += len;
        return;
    }
#endif
    kf_flush(c);
    if (c.stop) return;
    c.run.va = va; c.run.gpga = gpga; c.run.len = len;
    c.run.flags = flags; c.run.op = KFWR_OP_MAP; c.run.pdb_index = c.pdb_index;
    c.have = 1;
}

/* ⊘ Scope hints are gone (2026-09-25): a scoped walk CARRIED the previous walk's
 * runs across the regions it skipped, and there is no previous walk any more —
 * the walk is diffed against the committed placements, not against itself. */

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

__device__ void kf_walk_one(KfCtx &c, uint64_t pdb)
{
    const KfFormat &F = *c.fmt;
    const uint64_t root_bytes = (uint64_t)F.dir[F.first_dir].entries * F.dir[F.first_dir].entry_bytes;
    if (!kf_table_ok(c, pdb, root_bytes, F.root_align)) return;

    const uint32_t n0 = F.dir[0].active ? F.dir[0].entries : 1u;
    for (uint32_t i0 = 0; i0 < KF_MAX_ENT && i0 < n0 && !c.stop; i0++) {
        uint64_t t1 = pdb, va0 = 0ull;
        if (F.dir[0].active) {
            va0 = (uint64_t)i0 << F.dir[0].va_lo;
            uint32_t r = kf_dir_step(c, 0, pdb, i0, va0, &t1);
            if (r == KF_STEP_STOP) break;
            if (r != KF_STEP_DESCEND) continue;
        }

        const uint32_t n1 = F.dir[1].active ? F.dir[1].entries : 1u;
        for (uint32_t i1 = 0; i1 < KF_MAX_ENT && i1 < n1 && !c.stop; i1++) {
            uint64_t t2 = t1, va1 = va0;
            if (F.dir[1].active) {
                va1 = va0 | ((uint64_t)i1 << F.dir[1].va_lo);
                uint32_t r = kf_dir_step(c, 1, t1, i1, va1, &t2);
                if (r == KF_STEP_STOP) break;
                if (r != KF_STEP_DESCEND) continue;
            }

            const uint32_t n2 = F.dir[2].active ? F.dir[2].entries : 1u;
            for (uint32_t i2 = 0; i2 < KF_MAX_ENT && i2 < n2 && !c.stop; i2++) {
                uint64_t t3 = t2, va2 = va1;
                if (F.dir[2].active) {
                    va2 = va1 | ((uint64_t)i2 << F.dir[2].va_lo);
                    uint32_t r = kf_dir_step(c, 2, t2, i2, va2, &t3);
                    if (r == KF_STEP_STOP) break;
                    if (r != KF_STEP_DESCEND) continue;
                }

                const uint32_t n3 = F.dir[3].active ? F.dir[3].entries : 1u;
                for (uint32_t i3 = 0; i3 < KF_MAX_ENT && i3 < n3 && !c.stop; i3++) {
                    uint64_t t4 = t3, va3 = va2;
                    if (F.dir[3].active) {
                        va3 = va2 | ((uint64_t)i3 << F.dir[3].va_lo);
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
                        } else if (kf_slot_sparse(F, lo16)) {
                            /* ogkm `_gmmuIsInvalidPdeOk`: an INVALID big half with VOL set
                             * is sparse and aborts the whole 2 MiB — sublevel 1 is never
                             * read. ⊘ Only when the big half is NOT present: VOL on a
                             * present big table is a cache attribute, not a veto. */
                            c.sparse++;
                            has_s = false;
                        }

                        /* Both sub-tables cover the SAME VA range at different page
                         * sizes, so they are interleaved at the BIG page's
                         * granularity to keep each page-size class ascending in VA
                         * — which is what the per-class diff needs. `ratio` is
                         * derived, not hardcoded. */
                        const uint32_t ratio = 1u << (F.big_va_lo - F.small_va_lo);
                        for (uint32_t b = 0; b < KF_MAX_ENT && b < F.big_entries && !c.stop; b++) {
                            bool slot_unmapped = false;
                            if (has_b) {
                                uint64_t e;
                                /* ⊘ KF_BREAK_ORDER walks the big table DOWNWARDS, so the
                                 * 64 KiB class comes out descending in VA. Compiled in
                                 * only by `make check-closure-negative`: the per-class
                                 * diff needs each class ascending, and this is the
                                 * known-positive that the round-trip can see it when
                                 * it is not.
                                 * ⚠ This injection site lived in the code the format
                                 * seam replaced, and the control went VACUOUS for one
                                 * run — caught only because check-closure-negative
                                 * REQUIRES it to fail. A control that merely reports
                                 * would have gone on passing. */
#ifdef KF_BREAK_ORDER
                                const uint32_t bb = (uint32_t)F.big_entries - 1u - b;
#else
                                const uint32_t bb = b;
#endif
                                if (!kf_charge(c, 1)) break;
                                if (kf_load64(c, ptb + (uint64_t)bb * F.big_entry_bytes, &e)) {
                                    slot_unmapped = kf_big_pte_unmapped(F, e);
                                    if (kf_valid(F, e))
                                        kf_emit(c, va4 | ((uint64_t)bb << F.big_va_lo),
                                                kf_addr(F, e, kf_ap_raw(F, e)),
                                                1ull << F.ps_log2[F.big_ps],
                                                kf_leaf_flags(F, e, F.big_ps));
                                    else if (kf_slot_sparse(F, e)) c.sparse++;
                                }
                            }
                            if (has_s && !slot_unmapped) {
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
__global__ void kf_begin_kernel(KfArgs a)
{
    KfDev *d = a.dev;
    d->entries_visited = 0ull;
    d->refusals = 0u;
    d->refuse_mask = 0u;
    d->hdr_flags = 0u;
    d->walk_trunc = 0u;
    d->walk_abort = 0u;
    d->sparse_slots = 0u;
    for (uint32_t i = 0u; i < KF_MAX_PDB; i++) d->entry_refuse[i] = 0u;
    if (a.ack == NULL) return;
    /* After kf_commit_kernel (a kernel boundary orders them): record what was
     * committed, then empty the slots the host released. A released slot is never
     * committed into (kf_commit_kernel skips it), so the order is not a race. */
    const KfReportHeader *h = a.hdr;
    if (h->magic == KFWR_MAGIC && a.ack->generation != 0ull && a.ack->generation == h->generation &&
        !(h->flags & KFWR_HF_TRUNCATED) && a.ack->nrun == h->run_count)
        d->committed = a.ack->generation;
    const uint32_t nres = a.ack->nreset < KF_MAX_RESET ? a.ack->nreset : KF_MAX_RESET;
    for (uint32_t i = 0u; i < nres; i++) {
        const uint32_t s = a.ack->reset[i];
        if (s < d->max_slots)
            for (uint32_t c = 0u; c < 4u; c++) a.slot[s].n[c] = 0u;
    }
}

/* The serial walk: one thread per entry. Kept for the harness's A/B and the
 * deliberately-malformed launch probe; production runs the parallel walk. Both
 * write the same table (`a.walk`, `tbl_run_count`). */
__global__ void kf_walk_kernel(KfArgs a)
{
    const uint32_t t = blockIdx.x * blockDim.x + threadIdx.x;
    if (t >= a.npdb) return;
    KfDev *d = a.dev;
    const uint64_t pdb = a.pdbs[t];

    KfCtx c;
    c.w = a.win;
    c.fmt = &a.fmt;
    c.visited = 0ull;
    c.sparse = 0u;
    c.budget = (uint64_t)d->entry_budget;
    c.refusals = 0u; c.refuse = 0u; c.stop = 0u;
    c.out = a.walk + a.lay->walk_off[t];
    c.cap = a.lay->walk_cap[t];
    c.n = 0u; c.have = 0u;
    c.pdb_index = (uint16_t)t;
    memset(&c.run, 0, sizeof(c.run));

    kf_walk_one(c, pdb);

    d->tbl_run_count[t] = c.n;
    d->need[t] = c.stop ? c.n + 1u : c.n;   /* serial: "more than it holds" */

    atomicAdd(&d->entries_visited, (unsigned long long)c.visited);
    if (c.refusals) atomicAdd(&d->refusals, c.refusals);
    if (c.refuse) { atomicOr(&d->refuse_mask, c.refuse); if (t < KF_MAX_PDB) atomicOr(&d->entry_refuse[t], c.refuse); }
    if (c.sparse) atomicAdd(&d->sparse_slots, c.sparse);
    if (c.stop) { atomicOr(&d->hdr_flags, KFWR_HF_TRUNCATED); atomicOr(&d->walk_trunc, 1u); }
    if (c.refuse & KFWR_R_BUDGET) atomicOr(&d->hdr_flags, KFWR_HF_BUDGET);
}

/* ═══ THE DIFF AGAINST THE COMMITTED PLACEMENTS, AND THE COMMIT ON ACK ═════════
 *
 * ★★★★★ Owner design 2026-09-25: *"The GPU only sends a diff … the copy the PTX
 * holds, the last snapshot, is in vidmem, maintained by the PTX for compare"*;
 * ruling COMMIT-ON-ACK: the walk is diffed against what the host CONFIRMED it
 * placed, the host applies the diff, and only the entries it acknowledges are
 * committed. The Rust statement of this protocol — the spec these kernels are
 * held to, report for report, by `kf-gate9` — is `crates/kf-cuda/src/diffmodel.rs`.
 *
 * ⊘ What is committed is NOT the previous walk. The old snapshot (w826 and
 * before) was the last walk's output, a copy of guest table content committed
 * whether or not the host acted on it — the shadow `V3_BUILD.md` ruled out. A
 * slot here holds ONE ENTRY PER HOST MAP CALL WE MADE, written only when the
 * host says it landed: a record of our own actions (`THE_ARCHITECTURE_v3.md`
 * §4.2 w825). It is keyed by VA-space OBJECT (the host's slot index), never by
 * PDB, so a root move diffs the new root against what we placed under the old.
 *
 * The diff, per walked entry t, per page-size class:
 *   kept(p)  ⇔ the walk backs placement p byte for byte (same host ground truth,
 *              same linear offset, no hole);
 *   UNMAP p  for every placement not kept — WHOLE placements, because the host
 *              unmaps by the VA a map was placed at and nothing else;
 *   MAP      for every piece of the walk in a GAP between kept placements.
 * ⇒ Every kept placement is disjoint from every emitted piece, and the work is
 *   parallel over placements and gaps: O(n / threads + log n) per class plus
 *   O(pieces) — not the single-thread O(n) emission `[measured e3]` made
 *   quadratic (~0.53 µs/row at every invalidate).
 *
 * ⊘ Every loop below runs over OUR tables (the walk's output and the slot),
 * bounded by counts we wrote and capacities we allocated — I1 is about guest
 * data, and no guest byte is read here.
 */
#define KF_DIFF_BLOCK 512u
#define KF_DIFF_WARPS (KF_DIFF_BLOCK / 32u)
#define KF_CLASSES 4u

__device__ __forceinline__ uint32_t kf_pcls(uint32_t flags) { return (flags >> KFWR_RF_PS_SHIFT) & 3u; }
/* The ground truth the HOST maps: store, guest RAM, or nothing it will match — and, v3-gfx, the
 * PTE KIND the host mapping carries (a re-kinded page must be re-mapped: the host PTE's kind is
 * part of what we place). Mirrored by kf_cuda::diffmodel::host_key. */
__device__ __forceinline__ uint32_t kf_hkey(uint32_t flags)
{
    const uint32_t ap = flags & KFWR_RF_AP_MASK;
    const uint32_t k = ap == KFWR_AP_VIDMEM ? 0u : (ap == KFWR_AP_SYSCOH || ap == KFWR_AP_SYSNONCOH) ? 1u : 2u;
    return k | (((flags >> KFWR_RF_KIND_SHIFT) & KFWR_RF_KIND_MASK) << 2);
}
__device__ __forceinline__ uint64_t kf_end(const KfMapRun &r) { return r.va + r.len; }

/* Exclusive block-wide scan. Every thread of the block must call it (it syncs). */
template <typename T>
__device__ T kf_bscan(T v, T *total, T *sh)
{
    const uint32_t lane = threadIdx.x & 31u, wid = threadIdx.x >> 5;
    T x = v;
    for (uint32_t o = 1u; o < 32u; o <<= 1) {
        T y = __shfl_up_sync(0xffffffffu, x, o);
        if (lane >= o) x += y;
    }
    if (lane == 31u) sh[wid] = x;
    __syncthreads();
    if (wid == 0u) {
        const uint32_t nw = blockDim.x >> 5;
        T s = (lane < nw) ? sh[lane] : (T)0;
        for (uint32_t o = 1u; o < 32u; o <<= 1) {
            T y = __shfl_up_sync(0xffffffffu, s, o);
            if (lane >= o) s += y;
        }
        if (lane < nw) sh[lane] = s;
    }
    __syncthreads();
    const T excl = x - v + (wid ? sh[wid - 1u] : (T)0);
    *total = sh[(blockDim.x >> 5) - 1u];
    __syncthreads();
    return excl;
}

/* How many runs start at or before `va` (the Rust `partition_point(|r| r.va <= va)`). */
__device__ __forceinline__ uint32_t kf_n_le(const KfMapRun *w, uint32_t n, uint64_t va)
{
    uint32_t lo = 0u, hi = n;
    while (lo < hi) { const uint32_t m = (lo + hi) >> 1; if (w[m].va <= va) lo = m + 1u; else hi = m; }
    return lo;
}
/* How many runs start strictly before `va`. */
__device__ __forceinline__ uint32_t kf_n_lt(const KfMapRun *w, uint32_t n, uint64_t va)
{
    uint32_t lo = 0u, hi = n;
    while (lo < hi) { const uint32_t m = (lo + hi) >> 1; if (w[m].va < va) lo = m + 1u; else hi = m; }
    return lo;
}
/* How many runs END at or before `x` (sorted, disjoint ⇒ ends ascend). */
__device__ __forceinline__ uint32_t kf_n_end_le(const KfMapRun *w, uint32_t n, uint64_t x)
{
    uint32_t lo = 0u, hi = n;
    while (lo < hi) { const uint32_t m = (lo + hi) >> 1; if (kf_end(w[m]) <= x) lo = m + 1u; else hi = m; }
    return lo;
}

/* Whether run `r` holds VA `va`. */
__device__ __forceinline__ bool kf_holds(const KfMapRun &r, uint64_t va) { return r.va <= va && va < kf_end(r); }

/* Whether the walk runs `w` (one class, sorted, disjoint) back placement `p`
 * byte for byte; on success `*last` is the index of the run holding p's last
 * byte. `hint` is where the run holding p.va probably is — the SAME index as
 * p's in the slot when the slot and the walk agree up to it (the common case:
 * a few pages added or dropped) — tried with its two neighbours before the
 * binary search, so an unchanged space costs one coalesced load per placement.
 * The loop advances one run per step and stops at `n`. */
__device__ bool kf_covered(const KfMapRun &p, const KfMapRun *w, uint32_t n, uint32_t hint, uint32_t *last)
{
    const uint64_t end = p.va + p.len;
    if (end < p.va || n == 0u) return false;
    uint32_t i = n;
    if (hint < n && kf_holds(w[hint], p.va)) i = hint;
    else if (hint + 1u < n && kf_holds(w[hint + 1u], p.va)) i = hint + 1u;
    else if (hint >= 1u && hint - 1u < n && kf_holds(w[hint - 1u], p.va)) i = hint - 1u;
    else {
        const uint32_t j = kf_n_le(w, n, p.va);
        if (j == 0u) return false;
        i = j - 1u;
    }
    uint64_t at = p.va;
    const uint32_t key = kf_hkey(p.flags);
    for (; i < n; i++) {
        const KfMapRun r = w[i];
        const uint64_t rend = kf_end(r);
        if (!(r.va <= at && at < rend) || kf_hkey(r.flags) != key) return false;
        if (r.gpga + (at - r.va) != p.gpga + (at - p.va)) return false;
        at = rend;
        if (at >= end) { *last = i; return true; }
    }
    return false;
}

/* The first walk run ending after `lo`, the start of gap g. After a kept
 * placement it is the run that held that placement's last byte, or the next one
 * (`KW`, recorded by the kept pass) — no search; gap 0 searches. */
__device__ __forceinline__ uint32_t kf_gap_first(const KfMapRun *w, uint32_t n, const uint32_t *KW, uint32_t g, uint64_t lo)
{
    if (g == 0u) return kf_n_end_le(w, n, lo);
    uint32_t i = KW[g - 1u];
    if (i < n && kf_end(w[i]) <= lo) i++;
    return i;
}

/* The bounds of gap g between kept placements K[g-1] and K[g] of class list P. */
__device__ __forceinline__ void kf_gap(const KfMapRun *P, const uint32_t *K, uint32_t nk, uint32_t g,
                                       uint64_t *lo, uint64_t *hi)
{
    *lo = g ? kf_end(P[K[g - 1u]]) : 0ull;
    *hi = (g < nk) ? P[K[g]].va : ~0ull;
}

/* Where each walked entry's scratch lives (see KfArgs::scratch). */
/* ★ w829: carved from the entry's walk region (KfLayout): 4 runs and 3 words per walk-run slot. */
__device__ __forceinline__ KfMapRun *kf_scr(const KfArgs &a, uint32_t off)
{
    return a.scratch + (size_t)off * 4u;
}
__device__ __forceinline__ uint32_t *kf_iscr(const KfArgs &a, uint32_t off)
{
    return a.iscratch + (size_t)off * 3u;
}

/* ★★★★★ ONE BLOCK PER WALKED ENTRY: the diff of its walk against its slot, staged
 * in scratch as [UNMAPs of class 0][MAPs of class 0] … [class 3]. */
__global__ void __launch_bounds__(KF_DIFF_BLOCK) kf_diff_slots(KfArgs a)
{
    const uint32_t t = blockIdx.x;
    if (t >= a.npdb || t >= KF_MAX_PDB) return;
    KfDev *d = a.dev;
    const uint32_t rpp = a.lay->walk_cap[t];   /* this entry's walk region (and staging bound) */
    __shared__ unsigned long long sh64[KF_DIFF_WARPS];
    __shared__ uint32_t sh32[KF_DIFF_WARPS];
    const uint32_t s = a.slots[t];
    if (s >= d->max_slots) {
        if (threadIdx.x == 0u) {
            d->diff_count[t] = 0u;
            d->diff_vflags[t] = KFWR_V_OVERFLOW;
            atomicOr(&d->refuse_mask, KFWR_R_BAD_SLOT);
            atomicAdd(&d->refusals, 1u);
        }
        return;
    }
    const uint32_t scap = a.lay->slot_cap[s];
    /* ⊘ The staging bound (3 * walk_cap) holds only while the slot fits the walk region:
     * refused by name with the need, never written past. The host re-sizes and re-walks. */
    if (scap > rpp) {
        if (threadIdx.x == 0u) {
            d->diff_count[t] = 0u;
            d->diff_vflags[t] = KFWR_V_OVERFLOW;
            d->need[t] = scap;
            atomicOr(&d->refuse_mask, KFWR_R_RUN_CAP);
            atomicAdd(&d->refusals, 1u);
        }
        return;
    }
    const uint32_t nw = d->tbl_run_count[t] < rpp ? d->tbl_run_count[t] : rpp;
    const KfMapRun *W = a.walk + a.lay->walk_off[t];
    KfMapRun *X = kf_scr(a, a.lay->walk_off[t]);   /* the walk, partitioned by class */
    KfMapRun *stg = X + rpp;                /* the staged diff (<= 3 * rpp) */
    uint32_t *K = kf_iscr(a, a.lay->walk_off[t]);  /* kept placement indices, all classes */
    uint32_t *kf = K + rpp;                 /* kept flag per placement */
    uint32_t *KW = kf + rpp;                /* per kept placement: the walk run holding its last byte */
    const KfMapRun *com = a.com + a.lay->slot_off[s];
    uint32_t pn[KF_CLASSES], po[KF_CLASSES];
    {
        uint32_t o = 0u;
        for (uint32_t c = 0u; c < KF_CLASSES; c++) {
            uint32_t n = a.slot[s].n[c];
            if (n > scap - o) n = scap - o;   /* a corrupt count can only shrink the view */
            pn[c] = n; po[c] = o; o += n;
        }
    }

    /* ── 1. partition the walk by class (a packed 4 × 16-bit scan) ─────────── */
    uint32_t wn[KF_CLASSES] = {0u, 0u, 0u, 0u}, wo[KF_CLASSES];
    for (uint32_t base = 0u; base < nw; base += blockDim.x) {
        const uint32_t i = base + threadIdx.x;
        const unsigned long long v = (i < nw) ? (1ull << (16u * kf_pcls(W[i].flags))) : 0ull;
        unsigned long long tot;
        (void)kf_bscan<unsigned long long>(v, &tot, sh64);
        for (uint32_t c = 0u; c < KF_CLASSES; c++) wn[c] += (uint32_t)((tot >> (16u * c)) & 0xFFFFull);
    }
    wo[0] = 0u;
    for (uint32_t c = 1u; c < KF_CLASSES; c++) wo[c] = wo[c - 1u] + wn[c - 1u];
    {
        uint32_t run[KF_CLASSES] = {0u, 0u, 0u, 0u};
        for (uint32_t base = 0u; base < nw; base += blockDim.x) {
            const uint32_t i = base + threadIdx.x;
            const uint32_t c = (i < nw) ? kf_pcls(W[i].flags) : 0u;
            const unsigned long long v = (i < nw) ? (1ull << (16u * c)) : 0ull;
            unsigned long long tot;
            const unsigned long long ex = kf_bscan<unsigned long long>(v, &tot, sh64);
            if (i < nw) X[wo[c] + run[c] + (uint32_t)((ex >> (16u * c)) & 0xFFFFull)] = W[i];
            for (uint32_t k = 0u; k < KF_CLASSES; k++) run[k] += (uint32_t)((tot >> (16u * k)) & 0xFFFFull);
        }
    }
    __syncthreads();

    /* ── 2. kept placements (parallel over placements) ─────────────────────── */
    uint32_t nk[KF_CLASSES], ko[KF_CLASSES], nu[KF_CLASSES], nm[KF_CLASSES];
    {
        uint32_t kc = 0u;
        for (uint32_t c = 0u; c < KF_CLASSES; c++) {
            const KfMapRun *P = com + po[c];
            const KfMapRun *Wc = X + wo[c];
            ko[c] = kc; nk[c] = 0u; nu[c] = 0u;
            for (uint32_t base = 0u; base < pn[c]; base += blockDim.x) {
                const uint32_t i = base + threadIdx.x;
                const bool valid = i < pn[c];
                uint32_t lastw = 0u;
                const bool kept = valid && kf_covered(P[i], Wc, wn[c], i, &lastw);
                if (valid) kf[po[c] + i] = kept ? 1u : 0u;
                uint32_t tot;
                const uint32_t ex = kf_bscan<uint32_t>(kept ? 1u : 0u, &tot, sh32);
                if (kept) { K[kc + ex] = i; KW[kc + ex] = lastw; }
                kc += tot; nk[c] += tot;
            }
            nu[c] = pn[c] - nk[c];
        }
    }
    __syncthreads();

    /* ── 3. count the pieces in the gaps (parallel over gaps) ─────────────── */
    for (uint32_t c = 0u; c < KF_CLASSES; c++) {
        const KfMapRun *P = com + po[c];
        const KfMapRun *Wc = X + wo[c];
        const uint32_t *Kc = K + ko[c];
        const uint32_t *KWc = KW + ko[c];
        nm[c] = 0u;
        const uint32_t ng = nk[c] + 1u;
        for (uint32_t base = 0u; base < ng; base += blockDim.x) {
            const uint32_t g = base + threadIdx.x;
            uint32_t cnt = 0u;
            if (g < ng && wn[c]) {
                uint64_t lo, hi;
                kf_gap(P, Kc, nk[c], g, &lo, &hi);
                if (lo < hi)
                    for (uint32_t i = kf_gap_first(Wc, wn[c], KWc, g, lo); i < wn[c] && Wc[i].va < hi; i++) cnt++;
            }
            uint32_t tot;
            (void)kf_bscan<uint32_t>(cnt, &tot, sh32);
            nm[c] += tot;
        }
    }

    /* ── 4. capacity: a commit may never overflow the slot ─────────────────── */
    uint32_t np_all = 0u, nu_all = 0u, nm_all = 0u;
    for (uint32_t c = 0u; c < KF_CLASSES; c++) { np_all += pn[c]; nu_all += nu[c]; nm_all += nm[c]; }
    uint32_t vflags = 0u;
    bool emit_maps = true;
    if ((uint64_t)np_all + nm_all > scap) {
        /* ★ w829: say how much it needed, unless the walk itself was cut (its need wins). */
        if (threadIdx.x == 0u && !(d->entry_refuse[t] & KFWR_R_RUN_CAP)) d->need[t] = np_all + nm_all;
        if (nu_all) { vflags |= KFWR_V_PARTIAL; emit_maps = false; }
        else {
            if (threadIdx.x == 0u) {
                d->diff_count[t] = 0u;
                d->diff_vflags[t] = KFWR_V_OVERFLOW;
                atomicOr(&d->refuse_mask, KFWR_R_RUN_CAP);
                atomicAdd(&d->refusals, 1u);
            }
            return;
        }
    }

    /* ── 5. emit: per class, its UNMAPs then its MAPs ──────────────────────── */
    uint32_t out = 0u;
    for (uint32_t c = 0u; c < KF_CLASSES; c++) {
        const KfMapRun *P = com + po[c];
        const KfMapRun *Wc = X + wo[c];
        const uint32_t *Kc = K + ko[c];
        const uint32_t *KWc = KW + ko[c];
        for (uint32_t base = 0u; base < pn[c]; base += blockDim.x) {
            const uint32_t i = base + threadIdx.x;
            const bool un = i < pn[c] && kf[po[c] + i] == 0u;
            uint32_t tot;
            const uint32_t ex = kf_bscan<uint32_t>(un ? 1u : 0u, &tot, sh32);
            if (un) {
                KfMapRun r = P[i];
                r.op = KFWR_OP_UNMAP;
                r.pdb_index = (uint16_t)t;
                stg[out + ex] = r;
            }
            out += tot;
        }
        if (!emit_maps) continue;
        const uint32_t ng = nk[c] + 1u;
        for (uint32_t base = 0u; base < ng; base += blockDim.x) {
            const uint32_t g = base + threadIdx.x;
            uint64_t lo = 0ull, hi = 0ull;
            uint32_t first = 0u, cnt = 0u;
            if (g < ng && wn[c]) {
                kf_gap(P, Kc, nk[c], g, &lo, &hi);
                if (lo < hi) {
                    first = kf_gap_first(Wc, wn[c], KWc, g, lo);
                    for (uint32_t i = first; i < wn[c] && Wc[i].va < hi; i++) cnt++;
                }
            }
            uint32_t tot;
            const uint32_t ex = kf_bscan<uint32_t>(cnt, &tot, sh32);
            for (uint32_t k = 0u; k < cnt; k++) {
                const KfMapRun r = Wc[first + k];
                const uint64_t s0 = r.va > lo ? r.va : lo;
                const uint64_t e0 = kf_end(r) < hi ? kf_end(r) : hi;
                KfMapRun m;
                m.va = s0;
                m.gpga = r.gpga + (s0 - r.va);
                m.len = e0 - s0;
                m.flags = r.flags & ~KFWR_RF_HELD;
                m.op = KFWR_OP_MAP;
                m.pdb_index = (uint16_t)t;
                stg[out + ex + k] = m;
            }
            out += tot;
        }
    }
    if (threadIdx.x == 0u) {
        d->diff_count[t] = out;
        d->diff_vflags[t] = vflags;
    }
}

/* ★ The report: every entry's staged diff, in entry order, dense. One block per
 * entry copies its own slice; block 0 writes the header. Each block derives its
 * base from the counts, so no extra launch orders them. */
__global__ void kf_diff_emit(KfArgs a)
{
    const uint32_t t = blockIdx.x;
    KfDev *d = a.dev;
    const uint32_t cap_pdb = d->pdb_capacity < KF_MAX_PDB ? d->pdb_capacity : KF_MAX_PDB;
    const uint32_t np = a.npdb < cap_pdb ? a.npdb : cap_pdb;
    uint32_t base = 0u, total = 0u;
    for (uint32_t u = 0u; u < np; u++) {
        if (u < t) base += d->diff_count[u];
        total += d->diff_count[u];
    }
    const bool trunc = total > d->run_capacity;
    if (t == 0u && threadIdx.x == 0u) {
        d->generation += 1ull;
        uint32_t flags = d->hdr_flags | KFWR_HF_DIFF;
        if (trunc) flags |= KFWR_HF_TRUNCATED;
        if (a.npdb > cap_pdb) {
            flags |= (KFWR_HF_TRUNCATED | KFWR_HF_PDB_TRUNCATED);
            atomicOr(&d->refuse_mask, KFWR_R_PDB_CAP);
            atomicAdd(&d->refusals, 1u);
        }
        if (trunc) { atomicOr(&d->refuse_mask, KFWR_R_RUN_CAP); atomicAdd(&d->refusals, 1u); }
        if (d->refusals) flags |= KFWR_HF_REFUSED;
        KfReportHeader h;
        h.magic = KFWR_MAGIC;
        h.version = (uint16_t)KFWR_VERSION;
        h.flags = (uint16_t)flags;
        h.generation = d->generation;
        h.acked_generation = d->committed;
        h.pdb_count = np;
        h.pdb_capacity = d->pdb_capacity;
        h.run_count = trunc ? 0u : total;
        h.run_capacity = d->run_capacity;
        h.entries_visited = d->entries_visited;
        h.refusals = d->refusals;
        h.refuse_mask = d->refuse_mask;
        h.sparse_slots = d->sparse_slots;
        for (uint32_t i = 0; i < 4u; i++) h.ps_log2[i] = a.fmt.ps_log2[i];
        *a.hdr = h;
    }
    if (t >= np) return;
    const uint32_t n = trunc ? 0u : d->diff_count[t];
    const KfMapRun *stg = kf_scr(a, a.lay->walk_off[t]) + a.lay->walk_cap[t];
    for (uint32_t k = threadIdx.x; k < n; k += blockDim.x) a.rrun[base + k] = stg[k];
    if (threadIdx.x == 0u) {
        KfPdbEntry e;
        e.pdb = a.pdbs[t];
        e.first_run = trunc ? 0u : base;
        e.run_count = n;
        e.vas_flags = d->diff_vflags[t] | (d->entry_refuse[t] ? KFWR_V_REFUSED : 0u);
        e.reserved = a.slots[t];
        /* low 32: which refusals, for the host to name; high 32: the capacity it needed (w829) */
        e.reserved2 = (uint64_t)d->entry_refuse[t] | ((uint64_t)d->need[t] << 32);
        a.rpdb[t] = e;
    }
}

/* ★★★★★ COMMIT ON ACK — the first node of the NEXT walk. `a.hdr`/`a.rpdb`/`a.rrun`
 * still hold the previous report; `a.ack` is the host's verdict on it, written
 * into pinned memory before the walk was submitted. Only a verdict naming THAT
 * report's generation, over all of its runs, is committed; anything else (no
 * verdict, a stale one, a truncated report) commits nothing — the next diff is
 * then simply computed against the same placements again. */
__global__ void __launch_bounds__(KF_DIFF_BLOCK) kf_commit_kernel(KfArgs a)
{
    const uint32_t t = blockIdx.x;
    KfDev *d = a.dev;
    const KfAck *k = a.ack;
    const KfReportHeader *h = a.hdr;
    if (k == NULL) return;
    if (h->magic != KFWR_MAGIC || k->generation == 0ull || k->generation != h->generation) return;
    if (h->flags & KFWR_HF_TRUNCATED) return;
    if (k->nrun != h->run_count) return;
    if (t >= h->pdb_count || t >= KF_MAX_PDB) return;
    const KfPdbEntry e = a.rpdb[t];
    const uint32_t s = e.reserved;
    if (s >= d->max_slots) return;
    const uint32_t nres = k->nreset < KF_MAX_RESET ? k->nreset : KF_MAX_RESET;
    for (uint32_t i = 0u; i < nres; i++) if (k->reset[i] == s) return;
    /* ★ w829: the slot's region, and the PREVIOUS walk's region for this entry as scratch
     * (the report being committed was that walk's). */
    const uint32_t rpp = a.lay->slot_cap[s];
    const uint32_t pcap = a.lay->prev_cap[t];
    if ((uint64_t)e.first_run + e.run_count > h->run_count) return;
    const KfMapRun *R = a.rrun + e.first_run;
    const uint8_t *code = a.ack_code + e.first_run;
    const uint32_t nr = e.run_count;
    KfMapRun *com = a.com + a.lay->slot_off[s];
    KfMapRun *A = kf_scr(a, a.lay->prev_off[t]), *B = A + pcap, *O = A + 2u * pcap;
    uint32_t *rm = kf_iscr(a, a.lay->prev_off[t]);
    __shared__ uint32_t sh32[KF_DIFF_WARPS];
    uint32_t pn[KF_CLASSES], po[KF_CLASSES], nn[KF_CLASSES];
    {
        uint32_t o = 0u;
        for (uint32_t c = 0u; c < KF_CLASSES; c++) {
            uint32_t n = a.slot[s].n[c];
            if (n > rpp - o) n = rpp - o;
            pn[c] = n; po[c] = o; o += n;
        }
        /* ⊘ The scratch is the previous walk's region: a slot that no longer fits it (the host
         * grew it since) is refused loudly BEFORE any write — the next diff is taken against the
         * same placements again, which is always safe. */
        if (o > pcap) {
            if (threadIdx.x == 0u) { atomicOr(&d->refuse_mask, KFWR_R_RUN_CAP); atomicAdd(&d->refusals, 1u); }
            return;
        }
    }
    uint32_t oc = 0u;
    for (uint32_t c = 0u; c < KF_CLASSES; c++) {
        const KfMapRun *P = com + po[c];
        for (uint32_t i = threadIdx.x; i < pn[c]; i += blockDim.x) rm[i] = 0u;
        __syncthreads();
        for (uint32_t i = threadIdx.x; i < nr; i += blockDim.x) {
            const KfMapRun r = R[i];
            if (kf_pcls(r.flags) != c || r.op != KFWR_OP_UNMAP || code[i] == 0u) continue;
            const uint32_t j = kf_n_lt(P, pn[c], r.va);
            if (j < pn[c] && P[j].va == r.va) rm[j] = 1u;
        }
        __syncthreads();
        uint32_t na = 0u, nb = 0u;
        for (uint32_t base = 0u; base < pn[c]; base += blockDim.x) {
            const uint32_t i = base + threadIdx.x;
            const bool keep = i < pn[c] && rm[i] == 0u;
            uint32_t tot;
            const uint32_t ex = kf_bscan<uint32_t>(keep ? 1u : 0u, &tot, sh32);
            if (keep) A[na + ex] = P[i];
            na += tot;
        }
        for (uint32_t base = 0u; base < nr; base += blockDim.x) {
            const uint32_t i = base + threadIdx.x;
            bool take = false;
            KfMapRun r;
            if (i < nr) {
                r = R[i];
                take = kf_pcls(r.flags) == c && r.op == KFWR_OP_MAP && code[i] != 0u;
            }
            uint32_t tot;
            const uint32_t ex = kf_bscan<uint32_t>(take ? 1u : 0u, &tot, sh32);
            if (take) {
                r.op = KFWR_OP_MAP;
                r.pdb_index = 0u;
                r.flags = (r.flags & ~KFWR_RF_HELD) | (code[i] == 2u ? KFWR_RF_HELD : 0u);
                B[nb + ex] = r;
            }
            nb += tot;
        }
        __syncthreads();
        if ((uint64_t)oc + na + nb > rpp || (uint64_t)oc + na + nb > pcap) {
            /* ⊘ Unreachable by the diff's capacity rule; refused loudly rather than
             * written past the slot. The host then sees the next diff unchanged. */
            if (threadIdx.x == 0u) { atomicOr(&d->refuse_mask, KFWR_R_RUN_CAP); atomicAdd(&d->refusals, 1u); }
            return;
        }
        for (uint32_t i = threadIdx.x; i < na; i += blockDim.x) O[oc + i + kf_n_lt(B, nb, A[i].va)] = A[i];
        for (uint32_t j = threadIdx.x; j < nb; j += blockDim.x) O[oc + j + kf_n_le(A, na, B[j].va)] = B[j];
        nn[c] = na + nb;
        oc += na + nb;
        __syncthreads();
    }
    for (uint32_t i = threadIdx.x; i < oc; i += blockDim.x) com[i] = O[i];
    if (threadIdx.x == 0u)
        for (uint32_t c = 0u; c < KF_CLASSES; c++) a.slot[s].n[c] = nn[c];
}


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
    F.kind = kf_f(56, 8, 0);             /* NV_MMU_VER2_PTE_KIND 63:56 */
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
    F.kind = kf_f(8, 4, 0);              /* NV_MMU_VER3_PTE_KIND 11:8 */
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

/* ═══ THE PARALLEL WALK — level-synchronous, one warp per table ═══════════════
 *
 * `[w725]` the serial walk cost **450 ms** for the measured working set: one
 * thread per address space, 961 540 entries, 468 ns each ≈ one dependent memory
 * round-trip per 8-byte entry. The descent was serialised on dependent loads and
 * nothing else — a single thread cannot have two loads in flight.
 *
 * ⇒ The fix is BREADTH. Only the DEPTH of a page-table walk is serial: five
 * levels, PD3 → PD2 → PD1 → PD0 → leaves. At each level the fan-out is large,
 * and the leaf level — where essentially every entry lives — is embarrassingly
 * parallel once the leaf-table addresses are known. So the walk is done
 * **level-synchronously**: one kernel launch per level, each launch reading that
 * whole level's tables at once, with one warp per table and 32 loads in flight
 * per warp.
 *
 * ## The shape
 *
 *   seed      → a frontier of one entry per address space
 *   per level → COUNT (how many children does each frontier entry have?)
 *             → SCAN  (exclusive prefix sum over the counts)
 *             → WRITE (each entry writes its children at its scanned offset)
 *   dual      → the same, but the children are TASKS: one per PD0 slot
 *   leaves    → one warp per task: the warp bulk-loads the leaf table into
 *               shared memory, then forms runs from shared memory
 *   join      → runs that meet across a task boundary are re-joined
 *
 * ★★★ THE SCAN IS WHAT KEEPS THE REPORT SORTED, and it is why a scan is used
 * rather than the obvious `atomicAdd` reservation. Children are written at
 * `scan[parent] + rank_within_parent`, so the frontier stays in **exactly** the
 * order a depth-first walk would have produced: parents in VA order, children in
 * slot order. An atomic reservation would have been simpler and would have
 * scrambled the order that the per-class diff and `walkdiff` both depend on.
 *
 * ★★★ AND IT IS WHY RUN IDENTITY IS PRESERVED. Because the task array is in
 * DFS order, concatenating each task's runs in task order reproduces the serial
 * emission stream exactly — including the big/small interleave inside a dual
 * slot. The only runs the serial walk would have joined and the concatenation
 * would not are those meeting at a task boundary, and `kf_par_join` re-joins
 * precisely those: `head[t]` is false when task t's first run continues task
 * t-1's last, and then t's first run's length is added to t-1's last run instead
 * of being written. Chains across many tasks fall out of the same rule.
 *
 * ⊘ ONE KNOWN DIFFERENCE, and it is in the hostile direction only: the serial
 * walk carries its open run across a task that emitted NOTHING, so a mapping
 * either side of an all-refused task could join. A task emits nothing either
 * because its VA range holds no mapping — in which case the gap breaks
 * contiguity anyway and there is nothing to join — or because every leaf in it
 * was REFUSED (only possible for a misaligned large leaf). In that one case the
 * parallel walk reports one more run than the serial walk would. The mapping SET
 * is identical either way, which is what every consumer and every test asserts.
 *
 * ## The invariants
 *
 * I1 is STRONGER here, not weaker. The depth is now the host's launch loop,
 * bounded by `KF_DIRS`, a compile-time constant — a cycle in the guest's tables
 * cannot recurse at all, because there is no recursion: it simply produces
 * frontier entries at the next level, and there is no next level after the
 * fifth. Every device loop is bounded by `KF_MAX_ENT` or by one of our own
 * capacities.
 * I2 is unchanged: every load goes through `kf_win_load`, which is the only
 * caller of the one dereference.
 * I3 gains caps — the frontier, the task array — and both are loud.
 */

#define KF_MAX_FRONTIER 131072u
#define KF_MAX_SCRATCH  (4u << 20)    /* runs the leaf phase may stage */
#define KF_WARP         32u
#define KF_PAR_BLOCK    128u          /* 4 warps */
#define KF_SCAN_BLOCK   1024u
#define KF_SHWORDS      (KF_MAX_ENT + 64u)   /* u64 of shared memory per warp */
/* ⊘ A FIXED grid, walked with a stride, rather than one warp per frontier entry.
 * The frontier's size is a device value, so sizing the grid to it would need a
 * device-to-host synchronisation per level; sizing it to the host's UPPER BOUND
 * instead launched 32 768 blocks of which ~470 did anything, and the empty ones
 * cost more than the walk. `[measured]` the fixed cost fell from ~700 us to the
 * launch overhead of the launches themselves. */
#ifndef KF_PAR_GRID
#define KF_PAR_GRID     128u
#endif

#define KF_ENT_DEAD  0u   /* a root that was refused: contributes nothing      */
#define KF_ENT_TABLE 1u   /* a page-directory page to expand                   */
#define KF_ENT_LEAF  2u   /* a leaf found at a directory level, passed through */
#define KF_ENT_DUAL  3u   /* a TASK: the two leaf tables under one PD0 slot    */

struct KfEnt {
    uint64_t va;      /* VA base of this subtree / of this leaf */
    uint64_t addr;    /* table address, or a leaf's GPGA        */
    uint64_t addr2;   /* the BIG leaf table (KF_ENT_DUAL only)  */
    uint32_t flags;   /* a leaf's decoded flags                 */
    uint32_t len_log2;/* a leaf's size, log2                    */
    uint16_t pdb;
    uint8_t  kind;
    uint8_t  has;     /* bit0 small table present, bit1 big     */
};

/* What one task contributed, so the boundary join can be decided without
 * re-walking, and where its runs were staged. */
struct KfSum {
    uint64_t fva, fgpga, flen;
    uint64_t lva, lgpga, llen;
    uint32_t fflags, lflags;
    uint32_t n;
    uint32_t start;   /* where this task's runs sit in the staging buffer */
};

struct KfPar {
    KfEnt *fr[2];
    KfEnt *stage;          /* children written at atomically reserved offsets */
    KfEnt *task;
    KfMapRun *runstage;
    uint32_t *cnt, *off, *start, *nfr, *ntask, *pdbbase, *used;
    KfSum *sum;
    unsigned char *head;
};

/* ── the run accumulator ─────────────────────────────────────────────────────
 * ⊘ `out == NULL` means COUNT ONLY. The counting walk and the writing walk read
 * the SAME shared-memory snapshot of the table, so they cannot disagree about
 * how many runs there are — which is what makes the walk correct while the guest
 * is mutating the tables underneath it. */
struct KfRunAcc {
    const KfFormat *fmt;
    uint64_t span;                 /* §39(c): the bound every emitted leaf must lie inside */
    KfMapRun *out;
    uint32_t cap, n, have, overflow;
    KfMapRun run, first, last;
    uint32_t got_first, skip;
    uint16_t pdb_index;
    uint32_t refuse, refusals;
};

__device__ __forceinline__ void kf_acc_init(KfRunAcc &c, const KfFormat *f, uint64_t span,
                                            KfMapRun *out, uint32_t cap, uint16_t pi)
{
    c.fmt = f; c.span = span; c.out = out; c.cap = cap; c.n = 0u; c.have = 0u; c.overflow = 0u;
    c.got_first = 0u; c.skip = 0u; c.pdb_index = pi; c.refuse = 0u; c.refusals = 0u;
    memset(&c.run, 0, sizeof(c.run));
    memset(&c.first, 0, sizeof(c.first));
    memset(&c.last, 0, sizeof(c.last));
}

__device__ __forceinline__ void kf_acc_flush(KfRunAcc &c)
{
    if (!c.have) return;
    if (!c.got_first) { c.first = c.run; c.got_first = 1u; }
    c.last = c.run;
    c.have = 0u;
    if (c.skip) { c.skip = 0u; return; }
    if (c.out) {
        if (c.n < c.cap) c.out[c.n] = c.run;
        else { c.overflow = 1u; return; }
    }
    c.n++;
}

__device__ __forceinline__ void kf_acc_emit(KfRunAcc &c, uint64_t va, uint64_t gpga,
                                            uint64_t len, uint32_t flags)
{
    if (gpga & (kf_ps_bytes_of(*c.fmt, flags) - 1ull)) {
        c.refuse |= KFWR_R_MISALIGNED_LEAF; c.refusals++; return;
    }
#ifndef KF_BREAK_BOUNDS
    /* §39(c), the parallel half of the same chokepoint. */
    {   /* w825 — per aperture; see kf_emit. */
        uint32_t ap = (flags >> KFWR_RF_AP_SHIFT) & KFWR_RF_AP_MASK;
        if (ap == KFWR_AP_PEER ||
            (ap == KFWR_AP_VIDMEM && (gpga > c.span || len > c.span - gpga))) {
            c.refuse |= KFWR_R_LEAF_OOB; c.refusals++; return;
        }
    }
#endif
#ifndef KF_BREAK_COALESCE
    if (c.have && c.run.flags == flags &&
        c.run.va + c.run.len == va && c.run.gpga + c.run.len == gpga) {
        c.run.len += len;
        return;
    }
#endif
    kf_acc_flush(c);
    c.run.va = va; c.run.gpga = gpga; c.run.len = len; c.run.flags = flags;
    c.run.op = KFWR_OP_MAP; c.run.pdb_index = c.pdb_index; c.have = 1u;
}

__device__ __forceinline__ void kf_par_refuse(KfDev *d, unsigned int bit)
{
    atomicOr(&d->refuse_mask, bit);
    atomicAdd(&d->refusals, 1u);
}
/* The same, charged to walk entry `entry` as well (the per-space refusal the host names). */
__device__ __forceinline__ void kf_par_refuse_in(KfDev *d, unsigned int bit, uint32_t entry)
{
    kf_par_refuse(d, bit);
    if (entry < KF_MAX_PDB) atomicOr(&d->entry_refuse[entry], bit);
}

/* ⊘ ABORT, not TRUNCATED. Running out of RUN slots truncates the report but the
 * walk must still write what fits; running out of BUDGET or FRONTIER stops the
 * walk itself. Conflating them is how a run-cap truncation silently produced a
 * report of uninitialised runs. */
__device__ __forceinline__ void kf_par_abort(KfDev *d, unsigned int bit)
{
    atomicOr(&d->hdr_flags, KFWR_HF_TRUNCATED);
    atomicOr(&d->walk_trunc, 1u);
    atomicOr(&d->walk_abort, 1u);
    if (bit == KFWR_R_BUDGET) atomicOr(&d->hdr_flags, KFWR_HF_BUDGET);
    kf_par_refuse(d, bit);
}
__device__ __forceinline__ bool kf_par_aborted(const KfDev *d) { return d->walk_abort != 0u; }

/* ── seed: one frontier entry per address space, IN ORDER ──────────────────── */
__global__ void kf_par_seed(KfArgs a, KfEnt *fr, uint32_t *nfr, uint32_t *used)
{
    const uint32_t t = blockIdx.x * blockDim.x + threadIdx.x;
    if (t >= a.npdb) return;
    KfDev *d = a.dev;
    const KfFormat &F = a.fmt;
    const uint64_t pdb = a.pdbs[t];

    d->tbl_run_count[t] = 0u;
    if (t == 0u) { *nfr = a.npdb; used[0] = 0u; used[1] = 0u; }

    KfEnt e;
    memset(&e, 0, sizeof(e));
    e.pdb = (uint16_t)t;
    e.kind = KF_ENT_DEAD;
    const uint64_t rb = (uint64_t)F.dir[F.first_dir].entries * F.dir[F.first_dir].entry_bytes;
    if (pdb & (F.root_align - 1ull))                    kf_par_refuse_in(d, KFWR_R_UNALIGNED, t);
    else if (pdb > a.win.len || rb > a.win.len - pdb)   kf_par_refuse_in(d, KFWR_R_OOB, t);
    else { e.kind = KF_ENT_TABLE; e.addr = pdb; }
    fr[t] = e;
}

/* ── expand ONE level ────────────────────────────────────────────────────────
 *
 * One warp per frontier entry. The warp bulk-loads the whole table into shared
 * memory — coalesced, 32 loads in flight, ONE pass over global memory — and then
 * lane 0 decodes it. Counting and writing both read that one snapshot, so a
 * guest mutating the table cannot make them disagree.
 *
 * Children are staged at an ATOMICALLY reserved offset and each parent records
 * `(start, count)`; `kf_par_compact` then copies them into frontier order. ⊘ The
 * reservation is unordered and the compaction is what restores order — the
 * report's sortedness is not an accident of scheduling.
 *
 * `level == KF_DIRS-1` is the dual level, whose children are TASKS.
 */
/* Decode slot `i` of the table now in shared memory. Returns true and fills `ch`
 * when the slot yields a child; the census (refusals, sparse, foreign aperture)
 * is taken only when `census` is set, so the counting and writing sweeps do not
 * double-count. */
__device__ __forceinline__ bool kf_par_decode_slot(const KfArgs &a, uint32_t level, bool dual,
                                                   const KfEnt &e, const uint64_t *sh,
                                                   uint32_t i, uint32_t census, KfEnt &ch)
{
    KfDev *d = a.dev;
    const KfFormat &F = a.fmt;
    const KfDir &L = F.dir[level];
    ch.va = 0ull; ch.addr = 0ull; ch.addr2 = 0ull; ch.flags = 0u;
    ch.len_log2 = 0u; ch.pdb = e.pdb; ch.kind = 0u; ch.has = 0u;

    const uint64_t lo16 = dual ? sh[2u * i] : sh[i];
    if (L.leaf_ps != KF_PS_NONE && kf_valid(F, lo16)) {
        ch.va = e.va | ((uint64_t)i << L.va_lo);
        ch.addr = kf_addr(F, lo16, kf_ap_raw(F, lo16));
        ch.flags = kf_leaf_flags(F, lo16, L.leaf_ps);
        ch.len_log2 = F.ps_log2[L.leaf_ps];
        ch.kind = KF_ENT_LEAF;
        return true;
    }
    if (!dual) {
        const uint32_t apc = kf_ap_raw(F, lo16);
        if (!kf_dir_present(F, lo16, apc, L.leaf_ps != KF_PS_NONE)) {
            if (census && kf_slot_sparse(F, lo16)) atomicAdd(&d->sparse_slots, 1u);
            return false;
        }
        if (F.pde_ap_map[apc] != KFWR_AP_VIDMEM) {
            if (census) kf_par_refuse_in(d, KFWR_R_FOREIGN_AP, e.pdb);
            return false;
        }
        const uint64_t cb = (uint64_t)F.dir[level + 1u].entries * F.dir[level + 1u].entry_bytes;
        const uint64_t nx = kf_addr(F, lo16, apc);
        if (nx == 0ull) return false;          /* a null sub-table pointer is not a sub-table */
        if (!kf_win_table_ok(a.win, nx, cb, cb)) {
            if (census) kf_par_refuse_in(d, (nx & (cb - 1ull)) ? KFWR_R_UNALIGNED : KFWR_R_OOB, e.pdb);
            return false;
        }
        ch.va = e.va | ((uint64_t)i << L.va_lo);
        ch.addr = nx;
        ch.kind = KF_ENT_TABLE;
        return true;
    }

    const uint64_t hi16 = sh[2u * i + 1u];
    const uint64_t sb = (uint64_t)F.small_entries * F.small_entry_bytes;
    const uint64_t bb = (uint64_t)F.big_entries * F.big_entry_bytes;
    const uint32_t aps = kf_ap_raw(F, hi16), apb = kf_ap_raw(F, lo16);
    uint8_t has = 0u;
    uint64_t pts = 0ull, ptb = 0ull;
    if (kf_dir_present(F, hi16, aps, false)) {
        if (F.pde_ap_map[aps] != KFWR_AP_VIDMEM) { if (census) kf_par_refuse_in(d, KFWR_R_FOREIGN_AP, e.pdb); }
        else {
            pts = kf_addr(F, hi16, aps);
            if (pts != 0ull) {
                if (kf_win_table_ok(a.win, pts, sb, sb)) has |= 1u;
                else if (census) kf_par_refuse_in(d, (pts & (sb - 1ull)) ? KFWR_R_UNALIGNED : KFWR_R_OOB, e.pdb);
            }
        }
    } else if (census && kf_slot_sparse(F, hi16)) atomicAdd(&d->sparse_slots, 1u);
    if (kf_dir_present(F, lo16, apb, L.leaf_ps != KF_PS_NONE)) {
        if (F.pde_ap_map[apb] != KFWR_AP_VIDMEM) { if (census) kf_par_refuse_in(d, KFWR_R_FOREIGN_AP, e.pdb); }
        else {
            ptb = kf_big_addr(F, lo16, apb);
            if (ptb != 0ull) {
                if (kf_win_table_ok(a.win, ptb, bb, bb)) has |= 2u;
                else if (census) kf_par_refuse_in(d, (ptb & (bb - 1ull)) ? KFWR_R_UNALIGNED : KFWR_R_OOB, e.pdb);
            }
        }
    } else if (kf_slot_sparse(F, lo16)) {
        /* An INVALID, sparse big half vetoes the small table (see the serial walk). */
        if (census) atomicAdd(&d->sparse_slots, 1u);
        has &= ~1u;
    }
    if (!has) return false;
    ch.va = e.va | ((uint64_t)i << L.va_lo);
    ch.addr = pts; ch.addr2 = ptb; ch.has = has; ch.kind = KF_ENT_DUAL;
    return true;
}

/* ── expand ONE frontier entry ───────────────────────────────────────────────
 *
 * ★★★ ALL THIRTY-TWO LANES DECODE. `[measured w726]` the first version of this
 * had lane 0 walk the table alone after the warp had staged it, and cost **275
 * cycles per entry** — not spilling (ptxas says 0 bytes of spill), just the
 * throughput one thread gets. Thirty-one idle lanes were the whole cost, and
 * `__ballot_sync` is what gives them something to do while keeping the output in
 * slot order: `popc(ballot & lanemask_lt)` is the child's rank within its
 * parent, so a lane writes exactly where a serial walk would have put it.
 *
 * The table is read ONCE, into shared memory, and both sweeps read that one
 * snapshot — which is what makes the counting and the writing agree while the
 * guest is mutating the tables underneath.
 */
__device__ __forceinline__ void kf_par_expand_one(const KfArgs &a, uint32_t level, const KfEnt *in,
                                                  uint32_t gw, uint32_t lane, uint64_t *sh,
                                                  KfEnt *stage, uint32_t stagecap, uint32_t *used,
                                                  uint32_t *start, uint32_t *cnt)
{
    KfDev *d = a.dev;
    const KfFormat &F = a.fmt;
    const bool dual = (level == KF_DIRS - 1u);
    const KfDir &L = F.dir[level];
    const KfEnt e = in[gw];
    const uint32_t full = 0xFFFFFFFFu;

    if (e.kind == KF_ENT_DEAD) { if (lane == 0u) { cnt[gw] = 0u; start[gw] = 0u; } return; }
    if (e.kind == KF_ENT_LEAF) {          /* a leaf found higher up, passed through */
        if (lane == 0u) {
            const uint32_t st = atomicAdd(used, 1u);
            if (st < stagecap) stage[st] = e; else kf_par_abort(d, KFWR_R_FRONTIER_CAP);
            start[gw] = st; cnt[gw] = 1u;
        }
        return;
    }

    const uint32_t nslots = L.entries;
    const uint32_t nwords = nslots * (L.entry_bytes / 8u);
    if (lane == 0u) {
        const unsigned long long was = atomicAdd(&d->entries_visited, (unsigned long long)nslots);
        if (was + nslots > (unsigned long long)d->entry_budget) kf_par_abort(d, KFWR_R_BUDGET);
    }
    /* THE ONE PASS OVER GLOBAL MEMORY. */
    for (uint32_t i = lane; i < KF_SHWORDS && i < nwords; i += KF_WARP)
        if (!kf_win_load(a.win, e.addr + (uint64_t)i * 8u, &sh[i])) sh[i] = 0ull;
    __syncwarp();

    KfEnt ch;
    uint32_t n = 0u;
    for (uint32_t base = 0u; base < KF_MAX_ENT && base < nslots; base += KF_WARP) {
        const uint32_t i = base + lane;
        const uint32_t good = (i < nslots && kf_par_decode_slot(a, level, dual, e, sh, i, 1u, ch)) ? 1u : 0u;
        n += (uint32_t)__popc(__ballot_sync(full, good));
    }

    uint32_t st = 0u;
    if (lane == 0u) {
        st = atomicAdd(used, n);
        start[gw] = st;
        cnt[gw] = n;
    }
    st = __shfl_sync(full, st, 0);
    if (st + n > stagecap) { if (lane == 0u) kf_par_abort(d, KFWR_R_FRONTIER_CAP); return; }

    uint32_t w = 0u;
    for (uint32_t base = 0u; base < KF_MAX_ENT && base < nslots; base += KF_WARP) {
        const uint32_t i = base + lane;
        const uint32_t good = (i < nslots && kf_par_decode_slot(a, level, dual, e, sh, i, 0u, ch)) ? 1u : 0u;
        const uint32_t bal = __ballot_sync(full, good);
        if (good) stage[st + w + (uint32_t)__popc(bal & ((1u << lane) - 1u))] = ch;
        w += (uint32_t)__popc(bal);
    }
}

__global__ void kf_par_expand(KfArgs a, uint32_t level, const KfEnt *in, const uint32_t *nin,
                              KfEnt *stage, uint32_t stagecap, uint32_t *used,
                              uint32_t *start, uint32_t *cnt)
{
    extern __shared__ uint64_t shmem[];
    const uint32_t lane = threadIdx.x & (KF_WARP - 1u);
    const uint32_t wib = threadIdx.x / KF_WARP;
    uint64_t *sh = shmem + (size_t)wib * KF_SHWORDS;
    const uint32_t w0 = (blockIdx.x * blockDim.x + threadIdx.x) / KF_WARP;
    const uint32_t stride = (gridDim.x * blockDim.x) / KF_WARP;
    const uint32_t n = *nin;
    for (uint32_t gw = w0; gw < KF_MAX_FRONTIER && gw < n; gw += stride)
        kf_par_expand_one(a, level, in, gw, lane, sh, stage, stagecap, used, start, cnt);
}

/* ── the exclusive prefix sum ────────────────────────────────────────────────
 * One block. Each thread serially sums a contiguous chunk, one Hillis-Steele
 * scan over the KF_SCAN_BLOCK partials, then each thread writes its chunk's
 * offsets. ⊘ Both loops are bounded by KF_MAX_FRONTIER at compile time and by
 * OUR OWN count at run time — never by anything the guest wrote. */
__global__ void kf_par_scan(const uint32_t *in, const uint32_t *nin, uint32_t *out, uint32_t *total)
{
    __shared__ uint32_t s[KF_SCAN_BLOCK];
    const uint32_t n = *nin;
    const uint32_t chunk = (n + KF_SCAN_BLOCK - 1u) / KF_SCAN_BLOCK;
    const uint32_t lo = threadIdx.x * chunk;
    uint32_t sum = 0u;
    for (uint32_t i = lo; i < KF_MAX_FRONTIER && i < lo + chunk && i < n; i++) sum += in[i];
    s[threadIdx.x] = sum;
    __syncthreads();
    for (uint32_t dd = 1u; dd < KF_SCAN_BLOCK; dd <<= 1) {
        const uint32_t t = (threadIdx.x >= dd) ? s[threadIdx.x - dd] : 0u;
        __syncthreads();
        s[threadIdx.x] += t;
        __syncthreads();
    }
    uint32_t base = s[threadIdx.x] - sum;
    for (uint32_t i = lo; i < KF_MAX_FRONTIER && i < lo + chunk && i < n; i++) { out[i] = base; base += in[i]; }
    if (threadIdx.x == KF_SCAN_BLOCK - 1u) *total = s[threadIdx.x];
}

/* ★★★ ORDER IS RESTORED HERE. Children were staged wherever an atomic put them;
 * this copies them to `scan[parent] + j`, which is exactly where a depth-first
 * walk would have produced them. */
__global__ void kf_par_compact(KfArgs a, const KfEnt *stage, const uint32_t *start,
                               const uint32_t *cnt, const uint32_t *off, const uint32_t *nin,
                               KfEnt *dst, uint32_t cap)
{
    const uint32_t lane = threadIdx.x & (KF_WARP - 1u);
    const uint32_t w0 = (blockIdx.x * blockDim.x + threadIdx.x) / KF_WARP;
    const uint32_t stride = (gridDim.x * blockDim.x) / KF_WARP;
    if (kf_par_aborted(a.dev)) return;
    const uint32_t nn = *nin;
    for (uint32_t gw = w0; gw < KF_MAX_FRONTIER && gw < nn; gw += stride) {
        const uint32_t n = cnt[gw], st = start[gw], o = off[gw];
        for (uint32_t j = lane; j < KF_MAX_ENT && j < n; j += KF_WARP)
            if (o + j < cap) dst[o + j] = stage[st + j];
    }
}

/* ── the leaf phase ──────────────────────────────────────────────────────────
 * One warp per task. The warp bulk-loads the leaf tables into shared memory,
 * lane 0 forms the runs twice from that one snapshot — once to count, once to
 * write — and the runs are staged at an atomically reserved offset. */
/* Form the runs for big-page chunks [b0,b1) of one task, into `c`. ⊘ A CHUNK is
 * one big-page slot plus the small-page slots it spans, which is the unit the
 * interleave is built from — so a contiguous range of chunks is a contiguous
 * range of VA, which is what lets the warp split the task across lanes and join
 * the pieces back. */
__device__ __forceinline__ void kf_par_chunks(const KfArgs &a, const KfEnt &t,
                                              const uint64_t *ssmall, const uint64_t *sbig,
                                              KfRunAcc &c, uint32_t b0, uint32_t b1,
                                              uint32_t census)
{
    const KfFormat &F = a.fmt;
    const uint32_t ns = F.small_entries, nb = F.big_entries;
    const uint32_t ratio = 1u << (F.big_va_lo - F.small_va_lo);
    for (uint32_t b = b0; b < KF_MAX_ENT && b < b1; b++) {
#ifdef KF_BREAK_ORDER
        const uint32_t bb = nb - 1u - b;
#else
        const uint32_t bb = b;
#endif
        bool slot_unmapped = false;
        if (t.has & 2u) {
            const uint64_t e = sbig[bb];
            slot_unmapped = kf_big_pte_unmapped(F, e);
            if (kf_valid(F, e))
                kf_acc_emit(c, t.va | ((uint64_t)bb << F.big_va_lo), kf_addr(F, e, kf_ap_raw(F, e)),
                            1ull << F.ps_log2[F.big_ps], kf_leaf_flags(F, e, F.big_ps));
            else if (census && kf_slot_sparse(F, e)) atomicAdd(&a.dev->sparse_slots, 1u);
        }
        if ((t.has & 1u) && !slot_unmapped) {
            for (uint32_t j = 0u; j < 16u && j < ratio; j++) {
                const uint32_t si = bb * ratio + j;
                if (si >= ns) break;
                const uint64_t e = ssmall[si];
                if (kf_valid(F, e))
                    kf_acc_emit(c, t.va | ((uint64_t)si << F.small_va_lo), kf_addr(F, e, kf_ap_raw(F, e)),
                                1ull << F.ps_log2[F.small_ps], kf_leaf_flags(F, e, F.small_ps));
                else if (census && kf_slot_sparse(F, e)) atomicAdd(&a.dev->sparse_slots, 1u);
            }
        }
    }
    kf_acc_flush(c);
}

/* ── one task, across the whole warp ─────────────────────────────────────────
 *
 * ★★★ THE COALESCER IS THE PART THAT DOES NOT PARALLELISE, so it is split and
 * re-joined rather than left on one lane. `[measured w726]` leaving it on lane 0
 * cost 1 741 us for the working set — 31 idle lanes.
 *
 * Each lane takes a contiguous range of big-page chunks, which is a contiguous
 * range of VA, and forms runs within it. The pieces are then joined at lane
 * boundaries by exactly the rule the task boundary uses one level up: lane l's
 * first run continues lane l-1's last when VA, GPGA and flags all line up, and
 * then lane l contributes one run fewer and adds its first run's LENGTH to the
 * run lane l-1 already wrote. A warp-wide exclusive scan of the contributions
 * gives each lane its slot.
 *
 * ⊘ The writes and the length additions are separated by a `__syncwarp()`, for
 * the same reason the task-level join is a separate KERNEL: a plain store of
 * `len` and an atomic add to it would otherwise race, and the store would win.
 *
 * ⊘ An empty lane breaks a chain, and that is correct rather than convenient: a
 * lane produces nothing only when its VA range holds no mapping, and a hole in
 * VA breaks contiguity anyway.
 */
__device__ __forceinline__ void kf_par_leaf_one(const KfArgs &a, const KfEnt *task, uint32_t gw,
                                                uint32_t lane, uint64_t *ssmall, uint64_t *sbig,
                                                KfMapRun *runstage, uint32_t stagecap,
                                                uint32_t *used, KfSum *sum)
{
    KfDev *d = a.dev;
    const KfFormat &F = a.fmt;
    const uint32_t full = 0xFFFFFFFFu;
    KfSum sm;
    memset(&sm, 0, sizeof(sm));
    sm.lflags = 0xFFFFFFFFu;     /* a sentinel no real run carries: nothing joins to it */
    if (kf_par_aborted(d)) { if (lane == 0u) sum[gw] = sm; return; }

    const KfEnt t = task[gw];

    if (t.kind != KF_ENT_DUAL) {                 /* a leaf found at a directory level */
        if (lane == 0u) {
            KfRunAcc c;
            kf_acc_init(c, &F, a.win.span, NULL, 0u, t.pdb);
            kf_acc_emit(c, t.va, t.addr, 1ull << t.len_log2, t.flags);
            kf_acc_flush(c);
            /* §39(e): above the stagecap return below. That return would drop this
             * leaf's refusal on the floor and report only FRONTIER_CAP. */
            if (c.refuse) { atomicOr(&d->refuse_mask, c.refuse); atomicAdd(&d->refusals, c.refusals); if (c.pdb_index < KF_MAX_PDB) atomicOr(&d->entry_refuse[c.pdb_index], c.refuse); }
            if (c.n) {
                const uint32_t st = atomicAdd(used, c.n);
                if (st + c.n > stagecap) { kf_par_abort(d, KFWR_R_FRONTIER_CAP); sum[gw] = sm; return; }
                runstage[st] = c.last;
                sm.n = 1u; sm.start = st;
                sm.fva = c.last.va; sm.fgpga = c.last.gpga; sm.flen = c.last.len; sm.fflags = c.last.flags;
                sm.lva = c.last.va; sm.lgpga = c.last.gpga; sm.llen = c.last.len; sm.lflags = c.last.flags;
            }
            sum[gw] = sm;
        }
        return;
    }

    if (lane == 0u) {
        uint32_t charge = 0u;
        if (t.has & 1u) charge += F.small_entries;
        if (t.has & 2u) charge += F.big_entries;
        const unsigned long long was = atomicAdd(&d->entries_visited, (unsigned long long)charge);
        if (was + charge > (unsigned long long)d->entry_budget) kf_par_abort(d, KFWR_R_BUDGET);
    }
    if (t.has & 1u)
        for (uint32_t i = lane; i < KF_MAX_ENT && i < F.small_entries; i += KF_WARP)
            if (!kf_win_load(a.win, t.addr + (uint64_t)i * F.small_entry_bytes, &ssmall[i])) ssmall[i] = 0ull;
    if (t.has & 2u)
        for (uint32_t i = lane; i < KF_MAX_ENT && i < F.big_entries; i += KF_WARP)
            if (!kf_win_load(a.win, t.addr2 + (uint64_t)i * F.big_entry_bytes, &sbig[i])) sbig[i] = 0ull;
    __syncwarp();

    const uint32_t nb = F.big_entries;
    const uint32_t cpl = (nb + KF_WARP - 1u) / KF_WARP;   /* chunks per lane */
    const uint32_t b0 = lane * cpl;
    const uint32_t b1 = (b0 + cpl < nb) ? (b0 + cpl) : nb;

    KfRunAcc c;
    kf_acc_init(c, &F, a.win.span, NULL, 0u, t.pdb);
    if (b0 < nb) kf_par_chunks(a, t, ssmall, sbig, c, b0, b1, 1u);

    /* Does this lane's first run continue the previous lane's last? */
    const uint32_t pk    = __shfl_up_sync(full, c.n, 1);
    const uint64_t plva  = __shfl_up_sync(full, c.last.va, 1);
    const uint64_t plgp  = __shfl_up_sync(full, c.last.gpga, 1);
    const uint64_t pllen = __shfl_up_sync(full, c.last.len, 1);
    const uint32_t plfl  = __shfl_up_sync(full, c.last.flags, 1);
    uint32_t head = 1u;
    if (lane && c.n && pk && plfl == c.first.flags &&
        plva + pllen == c.first.va && plgp + pllen == c.first.gpga) head = 0u;
    const uint32_t contrib = (c.n && !head) ? (c.n - 1u) : c.n;

    uint32_t x = contrib;
    for (uint32_t dd = 1u; dd < KF_WARP; dd <<= 1) {
        const uint32_t y = __shfl_up_sync(full, x, dd);
        if (lane >= dd) x += y;
    }
    const uint32_t excl = x - contrib;
    const uint32_t total = __shfl_sync(full, x, KF_WARP - 1u);

    /* ★★★★★ PROPAGATE REFUSALS BEFORE ANY EARLY RETURN. ⊘ This used to sit at the
     * bottom of the function, below BOTH returns below -- so a warp whose leaves
     * were ALL refused produced `total == 0`, took the `!total` exit, and reported
     * nothing at all: no run, no flag, no refusal count. The report then said
     * "this address space is empty" when what happened was "every mapping in it
     * was rejected", which are opposite facts. Found by hostile/leaf_past_end,
     * whose single leaf is refused and which therefore hits exactly that path;
     * `leaf_oob_no_neighbour_extend` passed throughout because its surviving
     * legal run kept `total` non-zero. ⚠ Pre-existing and NOT specific to
     * LEAF_OOB -- MISALIGNED_LEAF was lost the same way, and a warp that is
     * entirely misaligned is not exotic. Same class as
     * `a_refusal_counter_read_as_absent_demand`. */
    if (c.refuse) { atomicOr(&d->refuse_mask, c.refuse); atomicAdd(&d->refusals, c.refusals); if (c.pdb_index < KF_MAX_PDB) atomicOr(&d->entry_refuse[c.pdb_index], c.refuse); }

    uint32_t st = 0u;
    if (lane == 0u && total) st = atomicAdd(used, total);
    st = __shfl_sync(full, st, 0);
    if (!total) { if (lane == 0u) sum[gw] = sm; return; }
    if (st + total > stagecap) { if (lane == 0u) { kf_par_abort(d, KFWR_R_FRONTIER_CAP); sum[gw] = sm; } return; }

    KfRunAcc wacc;
    kf_acc_init(wacc, &F, a.win.span, runstage + st + excl, contrib, t.pdb);
    wacc.skip = head ? 0u : 1u;
    if (b0 < nb) kf_par_chunks(a, t, ssmall, sbig, wacc, b0, b1, 0u);
    __syncwarp();
    /* ⊘ AFTER the stores, never interleaved with them. */
    if (!head && c.n)
        atomicAdd((unsigned long long *)&runstage[st + excl - 1u].len, (unsigned long long)c.first.len);
    __syncwarp();

    if (lane == 0u) {
        sm.n = total;
        sm.start = st;
        const KfMapRun f = runstage[st], l = runstage[st + total - 1u];
        sm.fva = f.va; sm.fgpga = f.gpga; sm.flen = f.len; sm.fflags = f.flags;
        sm.lva = l.va; sm.lgpga = l.gpga; sm.llen = l.len; sm.lflags = l.flags;
        sum[gw] = sm;
    }
}

__global__ void kf_par_leaf(KfArgs a, const KfEnt *task, const uint32_t *nt,
                            KfMapRun *runstage, uint32_t stagecap, uint32_t *used, KfSum *sum)
{
    extern __shared__ uint64_t shmem[];
    const uint32_t lane = threadIdx.x & (KF_WARP - 1u);
    const uint32_t wib = threadIdx.x / KF_WARP;
    uint64_t *ssmall = shmem + (size_t)wib * KF_SHWORDS;
    uint64_t *sbig = ssmall + KF_MAX_ENT;
    const uint32_t w0 = (blockIdx.x * blockDim.x + threadIdx.x) / KF_WARP;
    const uint32_t stride = (gridDim.x * blockDim.x) / KF_WARP;
    const uint32_t n = *nt;
    for (uint32_t gw = w0; gw < KF_MAX_FRONTIER && gw < n; gw += stride)
        kf_par_leaf_one(a, task, gw, lane, ssmall, sbig, runstage, stagecap, used, sum);
}

/* head[t] is false exactly when task t's first run continues task t-1's last.
 * The whole boundary-joining rule is these six comparisons. */
__global__ void kf_par_heads(KfArgs a, const KfEnt *task, const KfSum *sum, const uint32_t *nt,
                             uint32_t *contrib, unsigned char *head)
{
    const uint32_t nn = *nt;
    const uint32_t stride0 = gridDim.x * blockDim.x;
    for (uint32_t i = blockIdx.x * blockDim.x + threadIdx.x; i < KF_MAX_FRONTIER && i < nn; i += stride0) {
    const uint32_t n = sum[i].n;
    unsigned char h = 1u;
#ifndef KF_BREAK_JOIN
    if (n && i) {
        const KfSum p = sum[i - 1u];
        if (p.n && task[i - 1u].pdb == task[i].pdb &&
            p.lflags == sum[i].fflags &&
            p.lva + p.llen == sum[i].fva &&
            p.lgpga + p.llen == sum[i].fgpga) h = 0u;
    }
#endif
    head[i] = h;
    contrib[i] = (n && !h) ? (n - 1u) : n;
    if (contrib[i]) atomicAdd(&a.dev->tbl_run_count[task[i].pdb], contrib[i]);
    }
}

__global__ void kf_par_bases(KfArgs a, uint32_t *pdbbase)
{
    if (threadIdx.x || blockIdx.x) return;
    KfDev *d = a.dev;
    uint32_t run = 0u;
    for (uint32_t p = 0u; p < KF_MAX_PDB && p < a.npdb; p++) {
        pdbbase[p] = run;
        uint32_t n = d->tbl_run_count[p];
        d->need[p] = n;   /* ★ w829: UNCAPPED — what the host sizes the next walk by */
        if (n > a.lay->walk_cap[p]) {
            n = a.lay->walk_cap[p];
            d->tbl_run_count[p] = n;
            atomicOr(&d->hdr_flags, KFWR_HF_TRUNCATED);
            atomicOr(&d->walk_trunc, 1u);
            kf_par_refuse_in(d, KFWR_R_RUN_CAP, p);   /* the host names WHICH space overflowed */
        }
        run += n;
    }
}

/* Copy each task's staged runs into the table, in task order — which is DFS
 * order — and drop the first one when the boundary join has given it to the
 * previous task. */
__global__ void kf_par_emit(KfArgs a, const KfEnt *task, const KfSum *sum, const uint32_t *nt,
                            const uint32_t *off, const unsigned char *head,
                            const uint32_t *pdbbase, const KfMapRun *runstage)
{
    const uint32_t lane = threadIdx.x & (KF_WARP - 1u);
    const uint32_t w0 = (blockIdx.x * blockDim.x + threadIdx.x) / KF_WARP;
    const uint32_t stride = (gridDim.x * blockDim.x) / KF_WARP;
    KfDev *d = a.dev;
    if (kf_par_aborted(d)) return;
    const uint32_t nn = *nt;
    for (uint32_t gw = w0; gw < KF_MAX_FRONTIER && gw < nn; gw += stride) {
    const KfSum sm = sum[gw];
    if (!sm.n) continue;
    const uint16_t p = task[gw].pdb;
    const uint32_t skip = head[gw] ? 0u : 1u;
    const uint32_t cap = d->tbl_run_count[p];
    const uint32_t local = off[gw] - pdbbase[p];
    KfMapRun *dst = a.walk + a.lay->walk_off[p];
    for (uint32_t j = lane + skip; j < KF_MAX_ENT * 2u && j < sm.n; j += KF_WARP) {
        const uint32_t o = local + j - skip;
        if (o < cap) dst[o] = runstage[sm.start + j];
    }
    }
}

/* The other half of the boundary join: a task whose first run continued the
 * previous one adds its length to that run rather than writing a run of its own.
 * ⊘ A SEPARATE LAUNCH, and that is the whole reason it exists: the head run's
 * plain store of `len` and a follower's atomic add to it would otherwise race,
 * and the loser would be the store. A kernel boundary is the cheapest barrier
 * that orders them. */
__global__ void kf_par_join(KfArgs a, const KfEnt *task, const KfSum *sum, const uint32_t *nt,
                            const uint32_t *off, const unsigned char *head, const uint32_t *pdbbase)
{
    KfDev *d = a.dev;
    if (kf_par_aborted(d)) return;
    const uint32_t nn = *nt;
    const uint32_t stride0 = gridDim.x * blockDim.x;
    for (uint32_t i = blockIdx.x * blockDim.x + threadIdx.x; i < KF_MAX_FRONTIER && i < nn; i += stride0) {
    if (head[i] || !sum[i].n) continue;
    const uint16_t p = task[i].pdb;
    const uint32_t local = off[i] - pdbbase[p];
    if (local == 0u || local - 1u >= d->tbl_run_count[p]) continue;
    KfMapRun *dst = a.walk + a.lay->walk_off[p];
    atomicAdd((unsigned long long *)&dst[local - 1u].len, (unsigned long long)sum[i].flen);
    }
}

/* ── host side ───────────────────────────────────────────────────────────────── */
/* ⊘ Everything below drives the CUDA **runtime** API. It is the half kayfabe does
 * NOT use — the isolate drives the **driver** API from Rust — and the half NVRTC
 * cannot compile. See the `KF_DEVICE_ONLY` note at the top of this file. */
#ifndef KF_DEVICE_ONLY
struct KfWalk {
    KfDev *dev;
    KfFormat fmt;
    KfPar par;
    KfMapRun *walk;
    KfMapRun *com;
    KfSlot *slot;
    uint64_t *pdbs;
    uint32_t *slots;
    KfAck *ack;
    uint8_t *ack_code;
    uint32_t *iscratch;
    KfReportHeader *hdr;
    KfPdbEntry *rpdb;
    KfMapRun *rrun;
    KfLayout *lay;     /* w829: this runtime half keeps the UNIFORM layout (runs_per_pdb each) */
    KfWalkCfg cfg;
    /* the verdict kf_ack stages for the next kf_refresh */
    KfAck ack_h;
    uint8_t *ack_code_h;
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
    if (w->cfg.max_slots == 0u) w->cfg.max_slots = KF_MAX_PDB;
    if (w->cfg.max_slots > KF_MAX_SLOTS) {
        fprintf(stderr, "kf_create: max_slots %u > KF_MAX_SLOTS %u\n", w->cfg.max_slots, KF_MAX_SLOTS);
        free(w);
        return NULL;
    }
    /* The diff's scratch reuses the walk's run stage: 4 * runs_per_pdb per entry. */
    if ((uint64_t)KF_MAX_PDB * 4u * w->cfg.runs_per_pdb > KF_MAX_SCRATCH || w->cfg.runs_per_pdb > 0xFFFFu) {
        fprintf(stderr, "kf_create: runs_per_pdb %u does not fit the diff's scratch\n", w->cfg.runs_per_pdb);
        free(w);
        return NULL;
    }

    size_t tbl_bytes = (size_t)KF_MAX_PDB * w->cfg.runs_per_pdb * sizeof(KfMapRun);
    size_t com_bytes = (size_t)w->cfg.max_slots * w->cfg.runs_per_pdb * sizeof(KfMapRun);
    KF_CU(cudaMalloc(&w->dev, sizeof(KfDev)));
    KF_CU(cudaMalloc(&w->walk, tbl_bytes));
    KF_CU(cudaMalloc(&w->com, com_bytes));
    KF_CU(cudaMalloc(&w->slot, (size_t)w->cfg.max_slots * sizeof(KfSlot)));
    KF_CU(cudaMemset(w->slot, 0, (size_t)w->cfg.max_slots * sizeof(KfSlot)));
    KF_CU(cudaMalloc(&w->pdbs, KF_MAX_PDB * sizeof(uint64_t)));
    KF_CU(cudaMalloc(&w->slots, KF_MAX_PDB * sizeof(uint32_t)));
    KF_CU(cudaMalloc(&w->ack, sizeof(KfAck)));
    KF_CU(cudaMemset(w->ack, 0, sizeof(KfAck)));
    KF_CU(cudaMalloc(&w->ack_code, (size_t)w->cfg.run_capacity));
    KF_CU(cudaMalloc(&w->iscratch, (size_t)KF_MAX_PDB * 3u * w->cfg.runs_per_pdb * sizeof(uint32_t)));
    w->ack_code_h = (uint8_t *)calloc(w->cfg.run_capacity ? w->cfg.run_capacity : 1u, 1);
    KF_CU(cudaMalloc(&w->hdr, sizeof(KfReportHeader)));
    KF_CU(cudaMemset(w->hdr, 0, sizeof(KfReportHeader)));
    KF_CU(cudaMalloc(&w->rpdb, (size_t)w->cfg.pdb_capacity * sizeof(KfPdbEntry)));
    KF_CU(cudaMalloc(&w->rrun, (size_t)w->cfg.run_capacity * sizeof(KfMapRun)));

    KfDev h;
    memset(&h, 0, sizeof(h));
    h.runs_per_pdb = w->cfg.runs_per_pdb;
    h.entry_budget = w->cfg.entry_budget;
    h.run_capacity = w->cfg.run_capacity;
    h.pdb_capacity = w->cfg.pdb_capacity;
    h.max_pdbs = w->cfg.max_pdbs;
    h.max_slots = w->cfg.max_slots;
    KF_CU(cudaMemcpy(w->dev, &h, sizeof(h), cudaMemcpyHostToDevice));
    {
        KfLayout L;
        memset(&L, 0, sizeof(L));
        for (uint32_t i = 0u; i < KF_MAX_PDB; i++) {
            L.walk_off[i] = L.prev_off[i] = i * w->cfg.runs_per_pdb;
            L.walk_cap[i] = L.prev_cap[i] = w->cfg.runs_per_pdb;
        }
        for (uint32_t i = 0u; i < KF_MAX_SLOTS; i++) {
            L.slot_off[i] = i < w->cfg.max_slots ? i * w->cfg.runs_per_pdb : 0u;
            L.slot_cap[i] = i < w->cfg.max_slots ? w->cfg.runs_per_pdb : 0u;
        }
        KF_CU(cudaMalloc(&w->lay, sizeof(KfLayout)));
        KF_CU(cudaMemcpy(w->lay, &L, sizeof(L), cudaMemcpyHostToDevice));
    }

    /* ── the parallel walk's working set ──
     * ⊘ Sized by the FORMAT's worst legitimate case, not by hope: 12 GiB mapped
     * at 4 KiB is 6 144 PD0-slot tasks. KF_MAX_FRONTIER is twenty times that, so
     * reaching it means a tree whose directory entries alias one table — which
     * truncates, loudly. */
    KF_CU(cudaMalloc(&w->par.fr[0], (size_t)KF_MAX_FRONTIER * sizeof(KfEnt)));
    KF_CU(cudaMalloc(&w->par.fr[1], (size_t)KF_MAX_FRONTIER * sizeof(KfEnt)));
    KF_CU(cudaMalloc(&w->par.task, (size_t)KF_MAX_FRONTIER * sizeof(KfEnt)));
    KF_CU(cudaMalloc(&w->par.stage, (size_t)KF_MAX_FRONTIER * sizeof(KfEnt)));
    KF_CU(cudaMalloc(&w->par.start, (size_t)KF_MAX_FRONTIER * sizeof(uint32_t)));
    KF_CU(cudaMalloc(&w->par.runstage, (size_t)KF_MAX_SCRATCH * sizeof(KfMapRun)));
    KF_CU(cudaMalloc(&w->par.used, 4u * sizeof(uint32_t)));
    KF_CU(cudaMalloc(&w->par.cnt, (size_t)KF_MAX_FRONTIER * sizeof(uint32_t)));
    KF_CU(cudaMalloc(&w->par.off, (size_t)KF_MAX_FRONTIER * sizeof(uint32_t)));
    KF_CU(cudaMalloc(&w->par.sum, (size_t)KF_MAX_FRONTIER * sizeof(KfSum)));
    KF_CU(cudaMalloc(&w->par.head, (size_t)KF_MAX_FRONTIER));
    KF_CU(cudaMalloc(&w->par.nfr, 4u * sizeof(uint32_t)));
    KF_CU(cudaMalloc(&w->par.pdbbase, (size_t)KF_MAX_PDB * sizeof(uint32_t)));
    w->par.ntask = w->par.nfr + 2;
    return w;
}

extern "C" void kf_destroy(KfWalk *w)
{
    if (!w) return;
    cudaFree(w->par.fr[0]); cudaFree(w->par.fr[1]); cudaFree(w->par.task);
    cudaFree(w->par.stage); cudaFree(w->par.start); cudaFree(w->par.runstage);
    cudaFree(w->par.used);
    cudaFree(w->par.cnt); cudaFree(w->par.off); cudaFree(w->par.sum);
    cudaFree(w->par.head); cudaFree(w->par.nfr); cudaFree(w->par.pdbbase);
    cudaFree(w->dev); cudaFree(w->walk); cudaFree(w->com); cudaFree(w->slot);
    cudaFree(w->pdbs); cudaFree(w->slots); cudaFree(w->ack); cudaFree(w->ack_code);
    cudaFree(w->iscratch); cudaFree(w->lay); free(w->ack_code_h);
    cudaFree(w->hdr); cudaFree(w->rpdb); cudaFree(w->rrun);
    free(w);
}

extern "C" void kf_ack(KfWalk *w, uint64_t g, const uint8_t *codes, uint32_t nrun,
                       const uint32_t *resets, uint32_t nreset)
{
    memset(&w->ack_h, 0, sizeof(w->ack_h));
    w->ack_h.generation = g;
    w->ack_h.nrun = nrun;
    if (nrun > w->cfg.run_capacity) nrun = w->cfg.run_capacity;
    if (nrun && codes) memcpy(w->ack_code_h, codes, nrun);
    if (nreset > KF_MAX_RESET) nreset = KF_MAX_RESET;
    w->ack_h.nreset = nreset;
    for (uint32_t i = 0; i < nreset; i++) w->ack_h.reset[i] = resets[i];
}


/* ── the parallel walk's launch sequence ─────────────────────────────────────
 *
 * `[w725]` the serial walk cost 450 ms for the measured working set: one thread
 * per address space, 961 540 entries, 468 ns each — one dependent memory
 * round-trip per 8-byte entry, because a single thread cannot have two loads in
 * flight. Only the DEPTH of a page-table walk is serial; the breadth at every
 * level is large, and the leaf level, where essentially all the entries live, is
 * embarrassingly parallel once the leaf-table addresses are known.
 *
 * ★★★ THE DEPTH OF THE WALK IS THIS LOOP, and it is bounded by KF_DIRS, a
 * compile-time constant. That is I1 stated more strongly than the serial walk
 * could state it: there is no recursion to bound, because a level is a KERNEL
 * LAUNCH. A cycle in the guest's tables produces frontier entries at the next
 * level, and there is no level after the last one.
 *
 * ⊘ Grid sizes come from a HOST-side upper bound on the frontier
 * (`bound * entries`, clamped to the cap) rather than from the device's actual
 * count, so no phase needs a device-to-host synchronisation. Warps past the real
 * frontier read one word and retire; synchronising would cost more than they do.
 */

#ifdef KF_PHASES
/* ⊘ A measurement, not a feature. `[w726]` the fixed cost of a parallel refresh
 * was ~610 us on a 2 116-entry walk and two guesses at the cause (empty blocks,
 * block dispatch) were both WRONG — changing the grid from 512 to 128 blocks
 * moved it by nothing. This is what settled it. */
static int kf_ph_n = 0;
#define KF_PH_MAX 24
static cudaEvent_t kf_ph_e[KF_PH_MAX];
static const char *kf_ph_name[KF_PH_MAX];
static int kf_ph_i = 0;
static void kf_ph(const char *nm)
{
    if (kf_ph_i >= KF_PH_MAX) return;
    if (!kf_ph_e[kf_ph_i]) cudaEventCreate(&kf_ph_e[kf_ph_i]);
    cudaEventRecord(kf_ph_e[kf_ph_i]);
    kf_ph_name[kf_ph_i] = nm;
    kf_ph_i++;
}
static void kf_ph_dump(void)
{
    if (++kf_ph_n != 6) { kf_ph_i = 0; return; }
    cudaEventSynchronize(kf_ph_e[kf_ph_i - 1]);
    fprintf(stderr, "PHASES:");
    for (int i = 1; i < kf_ph_i; i++) {
        float ms = 0.f;
        cudaEventElapsedTime(&ms, kf_ph_e[i - 1], kf_ph_e[i]);
        fprintf(stderr, " %s=%.0fus", kf_ph_name[i], ms * 1000.f);
    }
    fprintf(stderr, "\n");
    kf_ph_i = 0;
}
#else
#define kf_ph(x) ((void)0)
#define kf_ph_dump() ((void)0)
#endif

static void kf_run_parallel(KfWalk *w, const KfArgs &a, uint32_t npdb)
{
    KfPar &P = w->par;
    const KfFormat &F = w->fmt;
    const uint32_t shm = (KF_PAR_BLOCK / KF_WARP) * KF_SHWORDS * 8u;

    kf_ph("start");
    kf_par_seed<<<(npdb + 127u) / 128u, 128>>>(a, P.fr[0], P.nfr, P.used);
    kf_ph("seed");

    uint32_t src = 0u;
    for (uint32_t k = F.first_dir; k < KF_DIRS; k++) {
        const uint32_t blocks = KF_PAR_GRID;
        uint32_t *nin = P.nfr + src, *nout = P.nfr + (src ^ 1u);
        KfEnt *dst = (k + 1u < KF_DIRS) ? P.fr[src ^ 1u] : P.task;
        kf_par_expand<<<blocks, KF_PAR_BLOCK, shm>>>(a, k, P.fr[src], nin, P.stage,
                                                     KF_MAX_FRONTIER, P.used, P.start, P.cnt);
        kf_ph("expand");
        kf_par_scan<<<1, KF_SCAN_BLOCK>>>(P.cnt, nin, P.off, nout);
        kf_ph("scan");
        kf_par_compact<<<blocks, KF_PAR_BLOCK>>>(a, P.stage, P.start, P.cnt, P.off, nin,
                                                 dst, KF_MAX_FRONTIER);
        kf_ph("compact");
        if (k + 1u < KF_DIRS) src ^= 1u; else cudaMemcpyAsync(P.ntask, nout, 4, cudaMemcpyDeviceToDevice);
        /* `used` is the staging cursor and is reset for the next level by the
         * compaction having already copied everything out of it. */
        cudaMemsetAsync(P.used, 0, 4);
        kf_ph("reset");
    }

    const uint32_t tblocks = KF_PAR_GRID, lblocks = KF_PAR_GRID;
    kf_par_leaf<<<tblocks, KF_PAR_BLOCK, shm>>>(a, P.task, P.ntask, P.runstage,
                                                KF_MAX_SCRATCH, P.used, P.sum);
    kf_ph("leaf");
    kf_par_heads<<<lblocks, 128>>>(a, P.task, P.sum, P.ntask, P.cnt, P.head);
    kf_par_scan<<<1, KF_SCAN_BLOCK>>>(P.cnt, P.ntask, P.off, P.nfr + 3);
    kf_par_bases<<<1, 1>>>(a, P.pdbbase);
    kf_par_emit<<<tblocks, KF_PAR_BLOCK>>>(a, P.task, P.sum, P.ntask, P.off, P.head,
                                           P.pdbbase, P.runstage);
    kf_par_join<<<lblocks, 128>>>(a, P.task, P.sum, P.ntask, P.off, P.head, P.pdbbase);
    kf_ph("emit+join");
    kf_ph_dump();
}

extern "C" int kf_refresh(KfWalk *w,
                          const void *gpga_dev, uint64_t gpga_len,
                          const uint64_t *pdbs, const uint32_t *slots, uint32_t npdb,
                          KfReportHeader *hdr_out, KfPdbEntry *pdb_out, KfMapRun *run_out)
{
    if (!w) { fprintf(stderr, "kf_refresh: no walker (the format descriptor was refused)\n"); return -3; }
    if (npdb == 0u) { fprintf(stderr, "kf_refresh: npdb == 0\n"); return -2; }
    if (npdb > w->cfg.max_pdbs) { fprintf(stderr, "kf_refresh: npdb %u > max_pdbs %u\n", npdb, w->cfg.max_pdbs); return -2; }

    KF_CU_I(cudaMemcpy(w->pdbs, pdbs, (size_t)npdb * sizeof(uint64_t), cudaMemcpyHostToDevice));
    KF_CU_I(cudaMemcpy(w->slots, slots, (size_t)npdb * sizeof(uint32_t), cudaMemcpyHostToDevice));
    KF_CU_I(cudaMemcpy(w->ack, &w->ack_h, sizeof(KfAck), cudaMemcpyHostToDevice));
    if (w->ack_h.nrun)
        KF_CU_I(cudaMemcpy(w->ack_code, w->ack_code_h,
                           w->ack_h.nrun < w->cfg.run_capacity ? w->ack_h.nrun : w->cfg.run_capacity,
                           cudaMemcpyHostToDevice));
    /* A verdict is consumed by exactly one refresh. */
    memset(&w->ack_h, 0, sizeof(w->ack_h));

    KfArgs a;
    memset(&a, 0, sizeof(a));
    a.win.base = (const uint8_t *)gpga_dev;
    a.win.len = gpga_len;
    a.win.span = w->cfg.gpga_span;
    a.fmt = w->fmt;
    a.dev = w->dev;
    a.walk = w->walk; a.com = w->com; a.slot = w->slot;
    a.pdbs = w->pdbs; a.slots = w->slots; a.npdb = npdb;
    a.ack = w->ack; a.ack_code = w->ack_code;
    a.scratch = w->par.runstage; a.iscratch = w->iscratch;
    a.hdr = w->hdr; a.rpdb = w->rpdb; a.rrun = w->rrun; a.lay = w->lay;

    kf_commit_kernel<<<KF_MAX_PDB, KF_DIFF_BLOCK>>>(a);
    kf_begin_kernel<<<1, 1>>>(a);
    if (getenv("KF_WALK_SERIAL")) kf_walk_kernel<<<(npdb + 31u) / 32u, 32>>>(a);
    else kf_run_parallel(w, a, npdb);
    kf_diff_slots<<<KF_MAX_PDB, KF_DIFF_BLOCK>>>(a);
    kf_diff_emit<<<KF_MAX_PDB, 256>>>(a);
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
                                  const KfMapRun *r, uint64_t gpga_span, const char **why)
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
        /* ★★★★★ §39(c) CONTAINMENT -- the one that is about ESCALATION rather than
         * well-formedness. An UNMAP names a VA we are retiring and carries no
         * gpga, so it is exempt; everything else becomes a MAPPING, and a mapping
         * outside the store hands the guest memory that is not its own. ⊘ This is
         * a SECOND implementation of the kernel's own emit-time check on purpose:
         * if the two ever disagree, the report is the thing that was wrong. */
        if (r[i].op != KFWR_OP_UNMAP &&
            ((r[i].flags & KFWR_RF_AP_MASK) == KFWR_AP_VIDMEM) &&
            (r[i].gpga > gpga_span || r[i].len > gpga_span - r[i].gpga))
                                                  { msg = "run leaves the GPGA window"; rc = -16; goto out; }
    }
    if ((h->flags & KFWR_HF_TRUNCATED) && h->run_count > h->run_capacity) { msg = "trunc"; rc = -14; goto out; }
out:
    if (why) *why = msg ? msg : "ok";
    return rc;
}
#endif /* KF_DEVICE_ONLY */
