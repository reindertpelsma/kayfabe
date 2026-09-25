/* kf_walk.h — the walk kernel's report ABI and host API.
 *
 * Contract: docs/design/the_walk_kernel_report_format.md ("the format doc").
 *
 * ⚠ TWO DEVIATIONS FROM THE FORMAT DOC, both deliberate and both recorded in
 *    cuda/walk/README.md:
 *
 *  1. The doc says `struct ReportHeader { // 64 B }` and then lists fields summing to
 *     **56**. Nothing can be built from a contradiction, so this header adds the two
 *     u32 the doc left implicit — `refuse_mask` and `pad` — reaching the stated 64.
 *     `refuse_mask` is the field that lets a test assert a refusal is THE ONE IT
 *     INTENDED rather than merely "something was refused".
 *
 *  2. The doc's validation rule "every `first_run + run_count <= run_count`" is a typo
 *     for "<= header.run_count". Implemented as the latter; see kf_validate_report.
 *
 * Everything else is the doc's layout, byte for byte: little-endian, naturally
 * aligned, no bitfields.
 */
#ifndef KF_WALK_H
#define KF_WALK_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* 'K','F','W','R' as little-endian bytes. */
#define KFWR_MAGIC   0x5257464Bu
#define KFWR_VERSION 1u

/* ── ReportHeader::flags ─────────────────────────────────────────────────────── */
#define KFWR_HF_TRUNCATED      (1u << 0)  /* run/pdb capacity or walk budget hit   */
#define KFWR_HF_RESYNC         (1u << 1)  /* every run is a MAP; not a delta       */
#define KFWR_HF_REFUSED        (1u << 2)  /* at least one refusal; see refuse_mask */
#define KFWR_HF_BUDGET         (1u << 3)  /* the entry budget stopped a walk       */
#define KFWR_HF_SCOPED         (1u << 4)  /* a scope hint restricted the walk      */
#define KFWR_HF_PDB_TRUNCATED  (1u << 5)  /* more address spaces than pdb_capacity */
#define KFWR_HF_SCOPE_DEGRADED (1u << 6)  /* a hint was unusable ⇒ full walk       */
/* ★ 2026-09-25: the runs are the DIFF of each entry's walk against its slot's
 * committed placements (kf_walk.cu, "THE DIFF AGAINST THE COMMITTED
 * PLACEMENTS"): UNMAPs name whole placements, MAPs the pieces to place. */
#define KFWR_HF_DIFF           (1u << 7)

/* ── ReportHeader::refuse_mask — WHICH refusal, so a test can name it ────────── */
#define KFWR_R_OOB           (1u << 0)  /* a table would leave the GPGA buffer     */
#define KFWR_R_UNALIGNED     (1u << 1)  /* a table pointer is not naturally aligned*/
#define KFWR_R_FOREIGN_AP    (1u << 2)  /* a table page is not in vidmem           */
#define KFWR_R_TOO_DEEP      (1u << 3)  /* ⊘ STRUCTURALLY UNREACHABLE: see README  */
#define KFWR_R_RUN_CAP       (1u << 4)  /* out of run slots                        */
#define KFWR_R_BUDGET        (1u << 5)  /* out of entry budget                     */
#define KFWR_R_PDB_CAP       (1u << 6)  /* out of PdbEntry slots                   */
#define KFWR_R_BAD_SCOPE     (1u << 7)  /* a hint we could not use                 */
#define KFWR_R_PDB_UNSORTED  (1u << 8)  /* caller's pdb list was not ascending     */
#define KFWR_R_DELTA_CAP     (1u << 9)  /* the delta itself overflowed its slice   */
/* ★★★ A leaf whose TARGET is not aligned to its own page size. The VER2 encoding
 * carries a 4 KiB-granular address field at EVERY leaf level, so a hostile guest
 * can spell a 512 MiB page whose physical base is 4 KiB-aligned and nothing else.
 * What the GMMU does with the low bits is not documented in ogkm, so the honest
 * answer is a refusal by name rather than a mapping we cannot stand behind.
 * ⊘ Found by the test suite, not by reading the format: the format doc's
 * validation rule "every len non-zero and page-aligned" has no counterpart for
 * gpga, and the encoding is why it cannot have one. */
#define KFWR_R_MISALIGNED_LEAF (1u << 10)
/* The format descriptor the host built was refused: an unknown `abi_version`, an
 * unknown `table_version`, or geometry the kernel's compile-time bounds cannot
 * hold. ⚠ A Rust/PTX skew must fail LOUDLY at launch rather than decode garbage
 * field offsets and look like a page-table bug (THE_CONSTRAINTS.md §21). */
#define KFWR_R_BAD_FORMAT      (1u << 11)
/* The parallel walk's frontier or task array overflowed. A legitimate tree
 * cannot reach it (the 12 GiB worst case needs 6 144 tasks); a cycle that makes
 * many directory entries point at one table can. Loud, and it truncates. */
#define KFWR_R_FRONTIER_CAP    (1u << 12)
/* ★★★★★ A LEAF whose [gpga, gpga+len) LEAVES THE GPGA WINDOW (§39). `KFWR_R_OOB`
 * is about a TABLE we would READ; this is about a MAPPING we would MAKE, and the
 * two are different stakes. A guest that points a leaf at its own wrong page has
 * corrupted only itself and we do not care (§39(b)); a guest that points one
 * PAST THE STORE is asking us to hand it memory that is not its own, which is
 * the escalation this walker exists to refuse. ⊘ Refused at the two emit
 * chokepoints, so the run never reaches the report — the host's `map()` bound is
 * defence in depth BEHIND this, not instead of it. */
#define KFWR_R_LEAF_OOB        (1u << 13)
/* An entry named a slot outside the committed-placement table. */
#define KFWR_R_BAD_SLOT        (1u << 14)

/* ── PdbEntry::vas_flags ─────────────────────────────────────────────────────── */
#define KFWR_V_NEW    (1u << 0)
#define KFWR_V_GONE   (1u << 1)
#define KFWR_V_RESYNC (1u << 2)
/* The entry's MAPs were WITHHELD — committing them on top of every current
 * placement could overflow the slot — and only its UNMAPs are in the report.
 * The host re-walks once they are applied. */
#define KFWR_V_PARTIAL  (1u << 3)
/* The slot is full and nothing can be retired: the entry carries no runs. */
#define KFWR_V_OVERFLOW (1u << 4)
/* ★ The entry's WALK refused something (`reserved2` = which KFWR_R_* bits): its
 * refused leaves are ABSENT from the walk, so the host fails that space by name
 * rather than treating the diff as the whole truth (owner ruling 2026-09-25). */
#define KFWR_V_REFUSED  (1u << 5)

/* ── MapRun::op ──────────────────────────────────────────────────────────────── */
#define KFWR_OP_MAP   1u
#define KFWR_OP_UNMAP 2u
#define KFWR_OP_REMAP 3u

/* ── MapRun::flags — the DECODED fields, never the raw entry ─────────────────── */
#define KFWR_RF_AP_SHIFT   0u
#define KFWR_RF_AP_MASK    0x7u   /* 0 vidmem, 1 peer, 2 sys-coherent, 3 sys-noncoh */
#define KFWR_AP_VIDMEM     0u
#define KFWR_AP_PEER       1u
#define KFWR_AP_SYSCOH     2u
#define KFWR_AP_SYSNONCOH  3u
#define KFWR_RF_READ_ONLY      (1u << 3)
#define KFWR_RF_ATOMIC_DISABLE (1u << 4)
#define KFWR_RF_VOLATILE       (1u << 5)
#define KFWR_RF_PRIVILEGE      (1u << 6)
#define KFWR_RF_PS_SHIFT   8u
#define KFWR_RF_PS_MASK    0xFu
/* ★★★★★ KIND IS PART OF RUN IDENTITY (the_walk_kernel_report_format.md §w725b).
 *
 * A run is the unit the host publishes as ONE mapping, and one mapping carries
 * one kind: a PTE's KIND tells the GPU how to INTERPRET the memory — tiling,
 * compression. A host mapping with the wrong kind makes an engine misread a
 * surface the guest wrote correctly: no fault, no refusal, wrong data.
 *
 * ⊘ It holds even though the publish path may not propagate kind yet: NEVER
 * COALESCE ACROSS A FIELD YOU DO NOT PROPAGATE. Merging destroys the boundary
 * irreversibly; keeping it costs runs. You can always merge later; you cannot
 * unmerge. `[measured w725]` on the real 12 GiB guest address space it costs
 * 3 runs instead of 1, against a 64K cap.
 *
 * ⊘ COMPTAGLINE is deliberately NOT here: it is a per-surface compression-tag
 * index, so two runs differing only in it are the same mapping shape — and the
 * two dev_mmu.h copies in the reference tree disagree about its width (53:36 vs
 * 55:36), which is its own reason not to key anything on it.
 *
 * ⊘ It lives in spare bits of the EXISTING flags word rather than in a new
 * field, so the report's byte layout — pinned by crates/kayfabe-mmu's seam test
 * against the C compiler's own offsetof — does not move. Run identity is
 * `flags` equality, so kind joins it by construction rather than by a rule
 * somebody has to remember at the coalescer. */
/* ★ A committed placement the host answered "already held": satisfied, but NOT
 * ours, so its UNMAP carries this bit and the host retires it without asking the
 * host RM to take down what it never placed for us (P6b ruling (a)). */
#define KFWR_RF_HELD (1u << 31)
#define KFWR_RF_KIND_SHIFT 16u
#define KFWR_RF_KIND_MASK  0xFFu
#define KFWR_PS_4K    0u
#define KFWR_PS_64K   1u
#define KFWR_PS_2M    2u
#define KFWR_PS_512M  3u

typedef struct KfReportHeader {
    uint32_t magic;
    uint16_t version;
    uint16_t flags;
    uint64_t generation;
    uint64_t acked_generation;
    uint32_t pdb_count;
    uint32_t pdb_capacity;
    uint32_t run_count;
    uint32_t run_capacity;
    uint64_t entries_visited;
    uint32_t refusals;
    uint32_t refuse_mask;   /* the doc's `reserved`, given a job (deviation 1) */
    /* Slots the guest DECLARED empty, as opposed to never having written them.
     * Counted because the two are different facts and the encoding that
     * distinguishes them is the one thing a field descriptor cannot carry. */
    uint32_t sparse_slots;
    /* ★ The report is SELF-DESCRIBING about page sizes: code -> log2(bytes).
     * ⇒ The host's parser needs no format-version knowledge at all, which is what
     * the format doc asks for ("the format-version knowledge stays in the kernel
     * and does not leak into the host's parser"). */
    uint8_t  ps_log2[4];
} KfReportHeader;

typedef struct KfPdbEntry {
    uint64_t pdb;
    uint32_t first_run;
    uint32_t run_count;
    uint32_t vas_flags;
    uint32_t reserved;
    uint64_t reserved2;
} KfPdbEntry;

typedef struct KfMapRun {
    uint64_t va;
    uint64_t gpga;
    uint64_t len;
    uint32_t flags;
    uint16_t op;
    uint16_t pdb_index;
} KfMapRun;

/* ★ One slot of committed placements: its count per page-size class. The
 * placements themselves are `runs_per_pdb` KfMapRuns at `com + slot * runs_per_pdb`,
 * class 0's first, each class VA-sorted and disjoint. */
typedef struct KfSlot {
    uint32_t n[4];
} KfSlot;

/* ★★★★★ THE HOST'S VERDICT on one report (COMMIT-ON-ACK). Written by the host
 * before the next walk is submitted; the next walk's first node commits it.
 * `code[i]` (a separate array, one byte per report run) is one of: */
#define KFWR_ACK_FAILED  0u   /* not applied: commit nothing              */
#define KFWR_ACK_APPLIED 1u   /* applied                                  */
#define KFWR_ACK_HELD    2u   /* a MAP the host already held: not ours    */
#define KF_MAX_RESET 64u
typedef struct KfAck {
    uint64_t generation;      /* the report this answers; 0 = no verdict  */
    uint32_t nrun;            /* must equal that report's run_count       */
    uint32_t nreset;          /* slots to EMPTY (their object is gone)    */
    uint32_t reset[KF_MAX_RESET];
} KfAck;

/* {pdb, va_base, va_len} — va_len == 0 means "walk this whole PDB" (the doc's
 * sentinel). Scope is a HINT: it can only make a walk faster, never wrong. */
typedef struct KfScope {
    uint64_t pdb;
    uint64_t va_base;
    uint64_t va_len;
} KfScope;

/* Bumped whenever the format descriptor's layout changes. A host/PTX skew must
 * fail LOUDLY at launch rather than decode garbage field offsets and look like a
 * page-table bug (THE_CONSTRAINTS.md §21). */
#define KF_ABI_VERSION 3u   /* 3: the diff/ack protocol (KfArgs, KfDev, KfSlot, KfAck) */

#define KF_TBL_VER2 2u   /* Pascal…Ada  — GA10x is the tested one               */
#define KF_TBL_VER3 3u   /* Hopper/Blackwell — SKETCHED, NEVER RUN, and refused
                          * unless KF_ALLOW_UNTESTED_VER3 is defined.           */

#define KF_MAX_PDB   64u   /* address spaces the kernel's table can hold */
#define KF_MAX_SCOPE 256u

typedef struct KfWalkCfg {
    uint32_t runs_per_pdb;  /* slice size of the kernel's own table, per VAS */
    uint32_t run_capacity;  /* report run array capacity                     */
    uint32_t pdb_capacity;  /* report PdbEntry capacity                      */
    uint32_t entry_budget;  /* entries one VAS's walk may examine            */
    /* KF_TBL_VER2 (0 means VER2) or KF_TBL_VER3. ⚠ VER3 is a SKETCH that has
     * never run and `kf_create` refuses it unless KF_ALLOW_UNTESTED_VER3 is
     * defined. See kf_walk.cu's format-seam block. */
    uint32_t table_version;
    /* Address spaces the kernel's OWN table can hold. Distinct from
     * pdb_capacity so that "the report ran out of PdbEntry slots" is testable
     * without also shrinking the table. <= KF_MAX_PDB; 0 means KF_MAX_PDB. */
    uint32_t max_pdbs;
    /* Committed-placement slots (one per VA-space object). 0 means KF_MAX_PDB. */
    uint32_t max_slots;
    /* ★★★★★ §39(c) THE GPGA SPAN -- how large the guest's GPGA address space is,
     * which is NOT the same question as `gpga_len` (how many bytes of it are
     * mapped for the kernel to READ). In production they coincide, because the
     * single store IS the whole of guest vidmem and all of it is mapped. They
     * come apart everywhere else: a captured corpus image holds the guest's
     * TABLE PAGES and not its framebuffer, so its leaves legitimately point far
     * outside the bytes the image contains.
     * ⊘ Conflating the two refused 263 legitimate leaves on real GA106 tables.
     * A leaf is contained against THIS; a table read is bounded by `gpga_len`.
     * 0 refuses every leaf -- loudly, which is the right failure for a field
     * someone forgot. Use KF_GPGA_SPAN_UNBOUNDED to opt out on purpose. */
    uint64_t gpga_span;
} KfWalkCfg;

/* "This caller has no framebuffer, only tables" -- corpora and differentials. */
#define KF_GPGA_SPAN_UNBOUNDED (~0ull)

typedef struct KfWalk KfWalk;

KfWalk *kf_create(const KfWalkCfg *cfg);
void    kf_destroy(KfWalk *w);

/* One refresh: commit the pending verdict (kf_ack), walk `pdbs[i]` for slot
 * `slots[i]`, and report each entry's diff against its slot. `gpga_dev` is a
 * DEVICE pointer to the buffer standing in for GPGA. Slots must be distinct and
 * < max_slots. Returns 0 on success, <0 on a CUDA error. */
int kf_refresh(KfWalk *w,
               const void *gpga_dev, uint64_t gpga_len,
               const uint64_t *pdbs, const uint32_t *slots, uint32_t npdb,
               KfReportHeader *hdr_out, KfPdbEntry *pdb_out, KfMapRun *run_out);

/* The host's verdict on the LAST report: one KFWR_ACK_* per run. Held until the
 * next kf_refresh, whose first kernel commits it. `nrun` must be that report's
 * run_count. `resets` empties slots (their object is gone). */
void kf_ack(KfWalk *w, uint64_t generation, const uint8_t *codes, uint32_t nrun,
            const uint32_t *resets, uint32_t nreset);

/* Property 3 of the format doc, on the host, over a buffer we already have.
 * Returns 0 if the report is well formed, else a negative code; `why` (may be NULL)
 * receives a static string. */
/* ⚠ §39(c): `gpga_span` is how large the guest's GPGA space is -- NOT how many
 * bytes were mapped to walk it. Every non-UNMAP run must lie wholly inside it.
 * Pass KF_GPGA_SPAN_UNBOUNDED when the caller has no framebuffer, only tables. */
int kf_validate_report(const KfReportHeader *h, const KfPdbEntry *p,
                       const KfMapRun *r, uint64_t gpga_span, const char **why);

#ifdef __cplusplus
}
#endif
#endif /* KF_WALK_H */
