/* kf_report_emit.c — THE C HALF OF THE C↔RUST SEAM TEST.
 *
 * ⊘⊘⊘ WHY THIS FILE EXISTS, w725
 *
 * `grep -rn "ReportHeader\|kf_report" crates/ --include=*.rs` returned NOTHING until now:
 * there was no Rust reader for the walk kernel's report at all. The differential oracle
 * compares through a *corpus file* written by a third program, so the report ABI — the
 * thing increment 6 consumes — was carried by two struct declarations in two languages
 * that had never met.
 *
 * A Rust encode/decode round trip would not have closed that gap. `NVOS34` is the proof:
 * a field at +12 instead of +16 encodes, decodes and round-trips AGAINST ITSELF while
 * every byte is in the wrong place. ⇒ The pins must be checked against the **C compiler's
 * own `offsetof`**, on the real `kf_walk.h`.
 *
 * This program therefore does two things and they are independent:
 *
 *   1. prints a LAYOUT manifest — `offsetof` and `sizeof` for every field of every report
 *      struct, straight from the compiler; and
 *   2. writes a REPORT image (header ‖ PdbEntry[pdb_count] ‖ MapRun[run_count], which is
 *      exactly the three `cudaMemcpy`s of kf_walk.cu:1286-1290 concatenated) and prints a
 *      VALUE manifest saying, in its own words, what it put in each field.
 *
 * The Rust side compares (1) against its pinned offsets and (2) against its parse. Neither
 * comparison is against something Rust itself produced.
 *
 * ⚠ HOST C ONLY — no CUDA, no GPU, no nvcc. It includes `kf_walk.h` for the structs and
 * nothing else, so this test runs anywhere `cargo test` runs.
 *
 * ★ KNOWN-POSITIVE: build with -DKF_SEAM_BREAK_LAYOUT to insert a 4-byte hole before
 * `pLinearAddress`'s analogue here (`MapRun::flags`), reproducing the NVOS34 shape exactly.
 * The seam test compiles this program BOTH ways and requires the broken one to be caught.
 *
 *   cc -O1 -std=c99 -Wall -Wextra -o kf_report_emit kf_report_emit.c
 *   ./kf_report_emit <report.bin>       # manifests on stdout
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stddef.h>

#include "kf_walk.h"

#ifdef KF_SEAM_BREAK_LAYOUT
/* ★ The known-positive. A struct that is byte-for-byte the real one EXCEPT that `flags`
 * has been pushed 4 bytes later — which is the NVOS34 defect, transplanted. It still
 * encodes, still decodes, and still round-trips against itself. Only the offsets differ. */
typedef struct KfMapRunBroken {
    uint64_t va;
    uint64_t gpga;
    uint64_t len;
    uint32_t hole;      /* <-- the injected 4-byte hole */
    uint32_t flags;
    uint16_t op;
    uint16_t pdb_index;
} KfMapRunEmit;
#define EMIT_FLAGS_OFF   offsetof(KfMapRunEmit, flags)
#define EMIT_OP_OFF      offsetof(KfMapRunEmit, op)
#define EMIT_PDBIDX_OFF  offsetof(KfMapRunEmit, pdb_index)
#else
typedef KfMapRun KfMapRunEmit;
#define EMIT_FLAGS_OFF   offsetof(KfMapRun, flags)
#define EMIT_OP_OFF      offsetof(KfMapRun, op)
#define EMIT_PDBIDX_OFF  offsetof(KfMapRun, pdb_index)
#endif

#define F(st, fld) \
    printf("field %s %s %zu %zu\n", #st, #fld, offsetof(st, fld), sizeof(((st *)0)->fld))

static void layout_manifest(void)
{
    printf("struct KfReportHeader %zu %zu\n", sizeof(KfReportHeader), _Alignof(KfReportHeader));
    F(KfReportHeader, magic);
    F(KfReportHeader, version);
    F(KfReportHeader, flags);
    F(KfReportHeader, generation);
    F(KfReportHeader, acked_generation);
    F(KfReportHeader, pdb_count);
    F(KfReportHeader, pdb_capacity);
    F(KfReportHeader, run_count);
    F(KfReportHeader, run_capacity);
    F(KfReportHeader, entries_visited);
    F(KfReportHeader, refusals);
    F(KfReportHeader, refuse_mask);
    F(KfReportHeader, sparse_slots);
    F(KfReportHeader, ps_log2);

    printf("struct KfPdbEntry %zu %zu\n", sizeof(KfPdbEntry), _Alignof(KfPdbEntry));
    F(KfPdbEntry, pdb);
    F(KfPdbEntry, first_run);
    F(KfPdbEntry, run_count);
    F(KfPdbEntry, vas_flags);
    F(KfPdbEntry, reserved);
    F(KfPdbEntry, reserved2);

    printf("struct KfMapRun %zu %zu\n", sizeof(KfMapRunEmit), _Alignof(KfMapRunEmit));
    printf("field KfMapRun va %zu %zu\n", offsetof(KfMapRunEmit, va), sizeof(uint64_t));
    printf("field KfMapRun gpga %zu %zu\n", offsetof(KfMapRunEmit, gpga), sizeof(uint64_t));
    printf("field KfMapRun len %zu %zu\n", offsetof(KfMapRunEmit, len), sizeof(uint64_t));
    printf("field KfMapRun flags %zu %zu\n", (size_t)EMIT_FLAGS_OFF, sizeof(uint32_t));
    printf("field KfMapRun op %zu %zu\n", (size_t)EMIT_OP_OFF, sizeof(uint16_t));
    printf("field KfMapRun pdb_index %zu %zu\n", (size_t)EMIT_PDBIDX_OFF, sizeof(uint16_t));

    /* The constants a Rust reader must agree with, straight from the header rather than
     * retyped. A drift in any of these renames every hostile assertion silently. */
    printf("const MAGIC %u\n", (unsigned)KFWR_MAGIC);
    printf("const VERSION %u\n", (unsigned)KFWR_VERSION);
    printf("const HF_TRUNCATED %u\n", (unsigned)KFWR_HF_TRUNCATED);
    printf("const HF_RESYNC %u\n", (unsigned)KFWR_HF_RESYNC);
    printf("const HF_REFUSED %u\n", (unsigned)KFWR_HF_REFUSED);
    printf("const HF_BUDGET %u\n", (unsigned)KFWR_HF_BUDGET);
    printf("const HF_SCOPED %u\n", (unsigned)KFWR_HF_SCOPED);
    printf("const HF_PDB_TRUNCATED %u\n", (unsigned)KFWR_HF_PDB_TRUNCATED);
    printf("const HF_SCOPE_DEGRADED %u\n", (unsigned)KFWR_HF_SCOPE_DEGRADED);
    printf("const R_OOB %u\n", (unsigned)KFWR_R_OOB);
    printf("const R_UNALIGNED %u\n", (unsigned)KFWR_R_UNALIGNED);
    printf("const R_FOREIGN_AP %u\n", (unsigned)KFWR_R_FOREIGN_AP);
    printf("const R_TOO_DEEP %u\n", (unsigned)KFWR_R_TOO_DEEP);
    printf("const R_RUN_CAP %u\n", (unsigned)KFWR_R_RUN_CAP);
    printf("const R_BUDGET %u\n", (unsigned)KFWR_R_BUDGET);
    printf("const R_PDB_CAP %u\n", (unsigned)KFWR_R_PDB_CAP);
    printf("const R_BAD_SCOPE %u\n", (unsigned)KFWR_R_BAD_SCOPE);
    printf("const R_PDB_UNSORTED %u\n", (unsigned)KFWR_R_PDB_UNSORTED);
    printf("const R_DELTA_CAP %u\n", (unsigned)KFWR_R_DELTA_CAP);
    printf("const R_MISALIGNED_LEAF %u\n", (unsigned)KFWR_R_MISALIGNED_LEAF);
    printf("const R_BAD_FORMAT %u\n", (unsigned)KFWR_R_BAD_FORMAT);
    printf("const V_NEW %u\n", (unsigned)KFWR_V_NEW);
    printf("const V_GONE %u\n", (unsigned)KFWR_V_GONE);
    printf("const V_RESYNC %u\n", (unsigned)KFWR_V_RESYNC);
    printf("const OP_MAP %u\n", (unsigned)KFWR_OP_MAP);
    printf("const OP_UNMAP %u\n", (unsigned)KFWR_OP_UNMAP);
    printf("const OP_REMAP %u\n", (unsigned)KFWR_OP_REMAP);
    printf("const RF_AP_SHIFT %u\n", (unsigned)KFWR_RF_AP_SHIFT);
    printf("const RF_AP_MASK %u\n", (unsigned)KFWR_RF_AP_MASK);
    printf("const RF_READ_ONLY %u\n", (unsigned)KFWR_RF_READ_ONLY);
    printf("const RF_ATOMIC_DISABLE %u\n", (unsigned)KFWR_RF_ATOMIC_DISABLE);
    printf("const RF_VOLATILE %u\n", (unsigned)KFWR_RF_VOLATILE);
    printf("const RF_PRIVILEGE %u\n", (unsigned)KFWR_RF_PRIVILEGE);
    printf("const RF_PS_SHIFT %u\n", (unsigned)KFWR_RF_PS_SHIFT);
    printf("const RF_PS_MASK %u\n", (unsigned)KFWR_RF_PS_MASK);
    printf("const AP_VIDMEM %u\n", (unsigned)KFWR_AP_VIDMEM);
    printf("const AP_PEER %u\n", (unsigned)KFWR_AP_PEER);
    printf("const AP_SYSCOH %u\n", (unsigned)KFWR_AP_SYSCOH);
    printf("const AP_SYSNONCOH %u\n", (unsigned)KFWR_AP_SYSNONCOH);
    printf("const PS_4K %u\n", (unsigned)KFWR_PS_4K);
    printf("const PS_64K %u\n", (unsigned)KFWR_PS_64K);
    printf("const PS_2M %u\n", (unsigned)KFWR_PS_2M);
    printf("const PS_512M %u\n", (unsigned)KFWR_PS_512M);
}

#define NPDB 3u
#define NRUN 9u

/* Every field gets a DISTINCT value, so a swap of two same-width neighbours is caught as
 * well as a shift. ⊘ A generator that writes 0,1,2,... would let `op` and `pdb_index`
 * trade places unnoticed. */
static const uint8_t PS_LOG2[4] = { 12, 16, 21, 29 };

int main(int argc, char **argv)
{
    const char *out = (argc > 1) ? argv[1] : "kf_report.bin";

    layout_manifest();

    KfReportHeader h;
    memset(&h, 0, sizeof h);
    h.magic            = KFWR_MAGIC;
    h.version          = KFWR_VERSION;
    h.flags            = (uint16_t)(KFWR_HF_RESYNC | KFWR_HF_SCOPED | KFWR_HF_REFUSED);
    h.generation       = 0x0123456789ABCDEFull;
    h.acked_generation = 0x00FEDCBA98765432ull;
    h.pdb_count        = NPDB;
    h.pdb_capacity     = 64u;
    h.run_count        = NRUN;
    h.run_capacity     = 4096u;
    h.entries_visited  = 0x000000BADC0FFEE1ull;
    h.refusals         = 7u;
    h.refuse_mask      = KFWR_R_MISALIGNED_LEAF | KFWR_R_BUDGET;
    h.sparse_slots     = 1234u;
    memcpy(h.ps_log2, PS_LOG2, 4);

    KfPdbEntry p[NPDB];
    memset(p, 0, sizeof p);
    /* Ascending pdbs; the last one is GONE and therefore carries no runs. */
    p[0].pdb = 0x00000000A0000000ull; p[0].first_run = 0; p[0].run_count = 5; p[0].vas_flags = KFWR_V_NEW | KFWR_V_RESYNC;
    p[1].pdb = 0x00000001B0000000ull; p[1].first_run = 5; p[1].run_count = 4; p[1].vas_flags = KFWR_V_RESYNC;
    p[2].pdb = 0x00000002C0000000ull; p[2].first_run = 9; p[2].run_count = 0; p[2].vas_flags = KFWR_V_GONE;

    KfMapRunEmit r[NRUN];
    memset(r, 0, sizeof r);
    /* VAS 0 — one run of each page-size class, plus one carrying every permission bit. */
    r[0].va = 0x0000100000000000ull; r[0].gpga = 0x0000000000030000ull; r[0].len = 4096ull * 7;
    r[0].flags = (KFWR_PS_4K << KFWR_RF_PS_SHIFT) | (KFWR_AP_VIDMEM << KFWR_RF_AP_SHIFT);
    r[0].op = KFWR_OP_MAP; r[0].pdb_index = 0;

    r[1].va = 0x0000100000100000ull; r[1].gpga = 0x0000000000140000ull; r[1].len = 65536ull * 3;
    r[1].flags = (KFWR_PS_64K << KFWR_RF_PS_SHIFT) | (KFWR_AP_SYSCOH << KFWR_RF_AP_SHIFT) | KFWR_RF_READ_ONLY;
    r[1].op = KFWR_OP_MAP; r[1].pdb_index = 0;

    r[2].va = 0x0000100000400000ull; r[2].gpga = 0x0000000000600000ull; r[2].len = 0x200000ull * 2;
    r[2].flags = (KFWR_PS_2M << KFWR_RF_PS_SHIFT) | (KFWR_AP_SYSNONCOH << KFWR_RF_AP_SHIFT) | KFWR_RF_VOLATILE;
    r[2].op = KFWR_OP_MAP; r[2].pdb_index = 0;

    /* ★★★ A 512 MiB run whose TARGET is only 4 KiB-aligned. VER2 can spell it, so a host
     * parser must PARSE it rather than assert an alignment the encoding cannot carry. */
    r[3].va = 0x0000100020000000ull; r[3].gpga = 0x0000000000003000ull; r[3].len = 0x20000000ull;
    r[3].flags = (KFWR_PS_512M << KFWR_RF_PS_SHIFT) | (KFWR_AP_PEER << KFWR_RF_AP_SHIFT) | KFWR_RF_ATOMIC_DISABLE;
    r[3].op = KFWR_OP_MAP; r[3].pdb_index = 0;

    r[4].va = 0x0000100040000000ull; r[4].gpga = 0x0000000000900000ull; r[4].len = 4096ull;
    r[4].flags = (KFWR_PS_4K << KFWR_RF_PS_SHIFT) | KFWR_RF_PRIVILEGE | KFWR_RF_READ_ONLY
               | KFWR_RF_VOLATILE | KFWR_RF_ATOMIC_DISABLE;
    r[4].op = KFWR_OP_MAP; r[4].pdb_index = 0;

    /* VAS 1 — the three ops, so `op` is exercised across its whole domain. */
    r[5].va = 0x0000200000000000ull; r[5].gpga = 0x0000000001000000ull; r[5].len = 4096ull * 2;
    r[5].flags = (KFWR_PS_4K << KFWR_RF_PS_SHIFT); r[5].op = KFWR_OP_MAP;   r[5].pdb_index = 1;

    r[6].va = 0x0000200000002000ull; r[6].gpga = 0xDEADBEEFDEADBEEFull; r[6].len = 4096ull;
    r[6].flags = (KFWR_PS_4K << KFWR_RF_PS_SHIFT); r[6].op = KFWR_OP_UNMAP; r[6].pdb_index = 1;

    r[7].va = 0x0000200000003000ull; r[7].gpga = 0x0000000002000000ull; r[7].len = 4096ull * 4;
    r[7].flags = (KFWR_PS_4K << KFWR_RF_PS_SHIFT); r[7].op = KFWR_OP_REMAP; r[7].pdb_index = 1;

    r[8].va = 0x0000200000800000ull; r[8].gpga = 0x0000000003000000ull; r[8].len = 65536ull;
    r[8].flags = (KFWR_PS_64K << KFWR_RF_PS_SHIFT); r[8].op = KFWR_OP_MAP;  r[8].pdb_index = 1;

    /* ── the VALUE manifest, printed from the C side's own variables ─────────────── */
    printf("hdr magic %u\n", (unsigned)h.magic);
    printf("hdr version %u\n", (unsigned)h.version);
    printf("hdr flags %u\n", (unsigned)h.flags);
    printf("hdr generation %llu\n", (unsigned long long)h.generation);
    printf("hdr acked_generation %llu\n", (unsigned long long)h.acked_generation);
    printf("hdr pdb_count %u\n", (unsigned)h.pdb_count);
    printf("hdr pdb_capacity %u\n", (unsigned)h.pdb_capacity);
    printf("hdr run_count %u\n", (unsigned)h.run_count);
    printf("hdr run_capacity %u\n", (unsigned)h.run_capacity);
    printf("hdr entries_visited %llu\n", (unsigned long long)h.entries_visited);
    printf("hdr refusals %u\n", (unsigned)h.refusals);
    printf("hdr refuse_mask %u\n", (unsigned)h.refuse_mask);
    printf("hdr sparse_slots %u\n", (unsigned)h.sparse_slots);
    printf("hdr ps_log2 %u %u %u %u\n", h.ps_log2[0], h.ps_log2[1], h.ps_log2[2], h.ps_log2[3]);
    for (unsigned i = 0; i < NPDB; i++)
        printf("pdb %u %llu %u %u %u %u %llu\n", i, (unsigned long long)p[i].pdb,
               (unsigned)p[i].first_run, (unsigned)p[i].run_count, (unsigned)p[i].vas_flags,
               (unsigned)p[i].reserved, (unsigned long long)p[i].reserved2);
    for (unsigned i = 0; i < NRUN; i++)
        printf("run %u %llu %llu %llu %u %u %u\n", i, (unsigned long long)r[i].va,
               (unsigned long long)r[i].gpga, (unsigned long long)r[i].len,
               (unsigned)r[i].flags, (unsigned)r[i].op, (unsigned)r[i].pdb_index);

    /* ── the image: the three cudaMemcpy's of kf_walk.cu:1286-1290, concatenated ─── */
    FILE *f = fopen(out, "wb");
    if (!f) { perror(out); return 1; }
    if (fwrite(&h, sizeof h, 1, f) != 1) { perror("hdr"); return 1; }
    if (fwrite(p, sizeof p[0], NPDB, f) != NPDB) { perror("pdb"); return 1; }
    if (fwrite(r, sizeof r[0], NRUN, f) != NRUN) { perror("run"); return 1; }
    fclose(f);
    printf("image %s %zu\n", out, sizeof h + sizeof p + sizeof r);
    printf("EMIT_OK\n");
    return 0;
}
