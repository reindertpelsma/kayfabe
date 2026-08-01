/*
 * gmmu_fmt_oracle — a TEST-ONLY differential oracle built out of NVIDIA's OWN GMMU
 * page-table format encoder.
 *
 * ============================================================================
 * WHY THIS EXISTS
 * ============================================================================
 *
 * `kayfabe_chips::Ga10xGmmu` decodes GA10x page-table entries, and its unit tests
 * construct the entries it decodes. Both were written by reading the same C. That is
 * exactly the hole `vbios_oracle.c` opens with: *"a transcribed parser cannot detect a
 * shared misreading, by construction."*
 *
 * It is not hypothetical here. The predecessor C artifact was bitten by one specific
 * misreading of this very format — on GA10x, `PD0`'s entry is a SIXTEEN-byte dual entry
 * naming two sub-tables, and `PD1` is itself a 512 MiB leaf level
 * (`kern_gmmu_fmt_ga10x.c:46-53`, whose entire generation delta is
 * `pLevels[2].bPageTable = NV_TRUE`). That was bug `#13` and it cost weeks.
 *
 * This program removes the hole rather than shrinking it. It compiles the ACTUAL format
 * encoder out of the vendored open kernel modules —
 *
 *   src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_fmt_ga10x.c
 *   src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_fmt_gp10x.c
 *   src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_fmt_gm10x.c
 *   src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_fmt_gm20x.c
 *   src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_gm200.c
 *   src/nvidia/src/libraries/mmu/gmmu_fmt.c
 *   src/nvidia/src/libraries/mmu/mmu_fmt.c
 *
 * — UNMODIFIED, against those trees' OWN headers, and drives them through the same
 * sequence `kgmmuConstructEngine_IMPL` does (`kern_gmmu.c:77-99`). Encoding and decoding
 * both go through the driver's own `gmmuFieldSetAperture` / `gmmuFieldSetAddress` /
 * `nvFieldSetBool` and their getters, and the PDE-vs-PTE question is answered by the
 * driver's own `gmmuFmtEntryIsPte`. There is no transcription of a bit position anywhere
 * in the path, so there is nothing left for a misreading to be shared with.
 *
 * ============================================================================
 * LICENSING — read before copying anything into this repository
 * ============================================================================
 *
 * Those sources are NVIDIA's, dual-licensed MIT / GPL-2.0 (each carries
 * `SPDX-License-Identifier: MIT` in its header). Compiling a slice of them for testing is
 * within the MIT grant. Deliberately, NOTHING from those trees is vendored into this
 * repository: `tests/build.rs` hands the C compiler their ABSOLUTE PATHS out of the
 * checkout that already exists beside it, and refuses (loudly) rather than substituting a
 * copy when that checkout is absent. If a copy is ever wanted, the MIT notice has to come
 * with it. This is `vbios_oracle.c`'s arrangement, unchanged.
 *
 * ============================================================================
 * ★★ SEPARATE TRANSLATION UNITS, NOT `#include`
 * ============================================================================
 *
 * `vbios_oracle.c` `#include`s its parser sources because it needs their `static`
 * helpers. This one must NOT: `kern_gmmu_fmt_gm10x.c` pulls `published/maxwell/gm107/
 * dev_mmu.h` (the VERSION_1 field definitions) and `kern_gmmu_fmt_gp10x.c` pulls
 * `published/pascal/gp100/dev_mmu.h` (VERSION_2), and the two headers define the SAME
 * macro names — `NV_MMU_PTE_COMPTAGLINE` among them — at different bit positions. Pulled
 * into one translation unit, whichever came second would silently re-point the other's
 * fields. So each driver file is compiled as its own translation unit and LINKED, which
 * is how the driver itself builds them; every function this harness calls is an external
 * symbol declared by the tree's own `g_kern_gmmu_nvoc.h`.
 *
 * ============================================================================
 * ★★ THE HAL BINDING IS DERIVED, NOT CHOSEN
 * ============================================================================
 *
 * Which `_GA10X` / `_GP10X` / `_GM10X` implementation a GA106 gets is a per-chip decision
 * the driver makes in `__nvoc_init_funcTable_KernelGmmu_1` (`src/nvidia/generated/
 * g_kern_gmmu_nvoc.c`). Picking one here by hand would be one more transcription — of the
 * sort that is right today and wrong at the next chip. Instead `tests/build.rs` PARSES
 * that dispatch table, finds the branch whose own `ChipHal:` comment names the chip, and
 * `-D`s the resulting symbol name into the macros below. If the driver rebinds a
 * function, this harness follows it or fails to build.
 */

#define NVOC_KERN_GMMU_H_PRIVATE_ACCESS_ALLOWED

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* The vendored tree's own headers. The include path is -I'd by tests/build.rs. */
#include "gpu/mmu/kern_gmmu.h"
#include "mmu/gmmu_fmt.h"
#include "mmu/mmu_fmt.h"

/*
 * The per-chip HAL bindings, -D'd by tests/build.rs out of the driver's own generated
 * dispatch table. Named here only so a build without them fails at compile time rather
 * than silently falling back to a guess.
 */
#ifndef OGKM_HAL_FMT_INIT_LEVELS
#error "OGKM_HAL_FMT_INIT_LEVELS not defined: tests/build.rs derives it from g_kern_gmmu_nvoc.c"
#endif
#ifndef OGKM_HAL_FMT_INIT_PDE
#error "OGKM_HAL_FMT_INIT_PDE not defined"
#endif
#ifndef OGKM_HAL_FMT_INIT_PDE_MULTI
#error "OGKM_HAL_FMT_INIT_PDE_MULTI not defined"
#endif
#ifndef OGKM_HAL_FMT_INIT_PTE
#error "OGKM_HAL_FMT_INIT_PTE not defined"
#endif
#ifndef OGKM_HAL_FMT_INIT_PDE_APERTURES
#error "OGKM_HAL_FMT_INIT_PDE_APERTURES not defined"
#endif
#ifndef OGKM_HAL_FMT_INIT_PTE_APERTURES
#error "OGKM_HAL_FMT_INIT_PTE_APERTURES not defined"
#endif
#ifndef OGKM_HAL_FMT_INIT_PTE_COMPTAGLINE
#error "OGKM_HAL_FMT_INIT_PTE_COMPTAGLINE not defined"
#endif
#ifndef OGKM_HAL_FMT_INIT_CAPS
#error "OGKM_HAL_FMT_INIT_CAPS not defined"
#endif
#ifndef OGKM_HAL_FMT_FAMILIES_INIT
#error "OGKM_HAL_FMT_FAMILIES_INIT not defined"
#endif
#ifndef OGKM_CHIP_NAME
#error "OGKM_CHIP_NAME not defined"
#endif

/* Report the derived symbol names back, so the Rust side can assert WHICH implementation
 * it was judged against rather than trusting the build script's word for it. */
#define OGKM_STR_(x) #x
#define OGKM_STR(x)  OGKM_STR_(x)

/* ------------------------------------------------------------------------- */
/* The assertion sink — every NV_ASSERT the driver fires is a VERDICT          */
/* ------------------------------------------------------------------------- */

/*
 * ★ Counted, not silenced. `kgmmuFmtInitLevels_GP10X` opens with three
 * `NV_ASSERT_OR_RETURN_VOID`s (version, level count, big page shift) and every
 * `kgmmuFmtInit*` asserts its version. A harness that fed the encoder something it
 * rejects would otherwise get a zeroed structure back and report it as the format.
 * `asserts=` is printed by every mode; a caller that sees a non-zero count knows the
 * answer below it is not the driver's considered opinion.
 *
 * The signature is `NV_ASSERT_FAILED_FUNC_TYPE`, which `NV_ASSERT_FAILED_USES_STRINGS=1`
 * (the tree's own Makefile default, and what tests/build.rs passes) makes
 * `const char *`.
 */
static int  g_asserts;
static char g_first_assert[512];

/* ★ The signatures come from the tree's OWN `NV_ASSERT_FAILED_FUNC_TYPE`, so a driver
 * that changes them breaks the build instead of quietly linking against the wrong one. */
static void oracle_note_assert(const char *expr, const char *file, NvU32 line)
{
    if (g_asserts == 0)
    {
        const char *base = file ? strrchr(file, '/') : NULL;
        snprintf(g_first_assert, sizeof g_first_assert, "%s @ %s:%u",
                 expr ? expr : "?", base ? base + 1 : (file ? file : "?"), (unsigned)line);
    }
    g_asserts++;
}

void nvAssertFailed(NV_ASSERT_FAILED_FUNC_TYPE)
{
    oracle_note_assert(pszExpr, pszFileName, lineNum);
}

void nvAssertFailedNoLog(NV_ASSERT_FAILED_FUNC_TYPE)
{
    oracle_note_assert(pszExpr, pszFileName, lineNum);
}

void nvAssertOkFailed(NvU32 status NV_ASSERT_FAILED_FUNC_COMMA_TYPE)
{
    (void)status;
    oracle_note_assert(pszExpr, pszFileName, lineNum);
}

void nvAssertOkFailedNoLog(NvU32 status NV_ASSERT_FAILED_FUNC_COMMA_TYPE)
{
    (void)status;
    oracle_note_assert(pszExpr, pszFileName, lineNum);
}

/* ------------------------------------------------------------------------- */
/* The format, assembled exactly as `kgmmuConstructEngine_IMPL` assembles it   */
/* ------------------------------------------------------------------------- */

/*
 * ⊘ THE ONE THING HERE THAT IS NOT THE DRIVER'S OWN CODE, named so it is not mistaken
 * for oracle.
 *
 * `kgmmuConstructEngine_IMPL` and `kgmmuFmtInit_IMPL` live in `kern_gmmu.c`, which also
 * contains the fault-buffer, invalidate and RMAPI paths — it takes an `OBJGPU *`, calls
 * `pRmApi->Control` and reaches half the object model. Compiling it would mean building
 * a mock maze, and a larger fake oracle is worth less than a smaller true one.
 *
 * So `build_fmt` below re-states those two functions' WIRING — which structure is passed
 * to which initialiser, in which order — and nothing else. It contains no bit position,
 * no field width, no shift, no aperture value and no level geometry: every one of those
 * still comes from the driver's own code, which is the half that can be misread. The
 * wiring itself is checked by its own citations and by the fact that a mis-wired format
 * would fail every single differential rather than passing a subset.
 *
 * Reference: `ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:77-99` (apertures, then
 * pdeMulti / pde / pte / comptagline per version) and `:616-650` (`pRoot`, `pPde`,
 * `pPdeMulti`, `pPte`, then levels, then caps).
 */
typedef struct {
    KernelGmmu         *pGmmu;
    GMMU_FMT_FAMILY     fam;
    GMMU_FMT            fmt;
    MMU_FMT_LEVEL       levels[GMMU_FMT_MAX_LEVELS + 1];
    NV_FIELD_ENUM_ENTRY pdeApertures[GMMU_APERTURE__COUNT];
    NV_FIELD_ENUM_ENTRY pteApertures[GMMU_APERTURE__COUNT];
    NvU32               numLevels;
} ORACLE_FMT;

static ORACLE_FMT g_o;

static void build_fmt(NvU32 bigPageShift, NvBool bUnifiedAperture)
{
    /*
     * A zeroed `KernelGmmu`. Every function called below takes one and NOT ONE OF THEM
     * DEREFERENCES IT — the fmt-init family is pure bit-packing, which is exactly why it
     * is the cheap target — except `kgmmuFmtFamiliesInit_GM200`, which reads
     * `pFmtFamilies[]` and nothing else. So the object exists to be that array's home.
     */
    g_o.pGmmu = calloc(1, sizeof(KernelGmmu));
    if (g_o.pGmmu == NULL)
    {
        fprintf(stderr, "gmmu_fmt_oracle: out of memory\n");
        exit(3);
    }

    memset(&g_o.fam, 0, sizeof g_o.fam);
    memset(&g_o.fmt, 0, sizeof g_o.fmt);
    memset(g_o.levels, 0, sizeof g_o.levels);
    memset(g_o.pdeApertures, 0, sizeof g_o.pdeApertures);
    memset(g_o.pteApertures, 0, sizeof g_o.pteApertures);

    /* kern_gmmu.c:77-78 */
    OGKM_HAL_FMT_INIT_PDE_APERTURES(g_o.pGmmu, g_o.pdeApertures);
    OGKM_HAL_FMT_INIT_PTE_APERTURES(g_o.pGmmu, g_o.pteApertures);

    /* kern_gmmu.c:94-99, for GMMU_FMT_VERSION_2 (the one GA10x walks). */
    OGKM_HAL_FMT_INIT_PDE_MULTI(g_o.pGmmu, &g_o.fam.pdeMulti, GMMU_FMT_VERSION_2,
                                g_o.pdeApertures);
    OGKM_HAL_FMT_INIT_PDE(g_o.pGmmu, &g_o.fam.pde, GMMU_FMT_VERSION_2, g_o.pdeApertures);
    OGKM_HAL_FMT_INIT_PTE(g_o.pGmmu, &g_o.fam.pte, GMMU_FMT_VERSION_2, g_o.pteApertures,
                          bUnifiedAperture);
    OGKM_HAL_FMT_INIT_PTE_COMPTAGLINE(g_o.pGmmu, &g_o.fam.pte, GMMU_FMT_VERSION_2);

    /* kern_gmmu.c:640-647 */
    g_o.numLevels          = GMMU_FMT_MAX_LEVELS + 1;
    g_o.fmt.version        = GMMU_FMT_VERSION_2;
    g_o.fmt.pRoot          = g_o.levels;
    g_o.fmt.pPdeMulti      = &g_o.fam.pdeMulti;
    g_o.fmt.pPde           = &g_o.fam.pde;
    g_o.fmt.pPte           = &g_o.fam.pte;
    OGKM_HAL_FMT_INIT_LEVELS(g_o.pGmmu, g_o.levels, g_o.numLevels, GMMU_FMT_VERSION_2,
                             bigPageShift);
    OGKM_HAL_FMT_INIT_CAPS(g_o.pGmmu, &g_o.fmt);

    /*
     * The sparse / NV4K templates. `kgmmuFmtFamiliesInit_*` is where the driver states
     * what "sparse" IS on this regime, and it is the only source for that fact: the
     * encoding is *valid clear, volatile set*, which nothing in the format description
     * itself says.
     */
    g_o.pGmmu->pFmtFamilies[1] = &g_o.fam; /* index of GMMU_FMT_VERSION_2 in g_gmmuFmtVersions */
    (void)OGKM_HAL_FMT_FAMILIES_INIT(NULL, g_o.pGmmu);
}

/* ------------------------------------------------------------------------- */
/* Small helpers — hex in, hex out                                            */
/* ------------------------------------------------------------------------- */

/* An entry buffer is always GMMU_FMT_MAX_ENTRY_SIZE (16) bytes; short levels use a prefix. */
static void put_hex(const NvU8 *v)
{
    int i;
    for (i = 0; i < GMMU_FMT_MAX_ENTRY_SIZE; ++i)
        printf("%02x", v[i]);
}

static int get_hex(const char *s, NvU8 *out)
{
    int i;
    memset(out, 0, GMMU_FMT_MAX_ENTRY_SIZE);
    for (i = 0; i < GMMU_FMT_MAX_ENTRY_SIZE * 2; ++i)
    {
        int hi, lo;
        char a = s[i];
        if (a == '\0')
            return -1;
        hi = (a >= '0' && a <= '9') ? a - '0'
           : (a >= 'a' && a <= 'f') ? a - 'a' + 10
           : (a >= 'A' && a <= 'F') ? a - 'A' + 10
           : -1;
        if (hi < 0)
            return -1;
        lo = i & 1;
        if (lo)
            out[i / 2] |= (NvU8)hi;
        else
            out[i / 2] = (NvU8)(hi << 4);
    }
    /* Bytes are printed most-significant-nibble first per byte, byte 0 first (little
     * endian entry order), which is how the Rust side lays out a u128's LE bytes. */
    return 0;
}

static const char *aperture_name(GMMU_APERTURE a)
{
    switch (a)
    {
        case GMMU_APERTURE_INVALID:     return "INVALID";
        case GMMU_APERTURE_VIDEO:       return "VIDEO";
        case GMMU_APERTURE_PEER:        return "PEER";
        case GMMU_APERTURE_SYS_COH:     return "SYS_COH";
        case GMMU_APERTURE_SYS_NONCOH:  return "SYS_NONCOH";
        default:                        return "?";
    }
}

static int aperture_of_name(const char *s, GMMU_APERTURE *out)
{
    if (!strcmp(s, "INVALID"))    { *out = GMMU_APERTURE_INVALID;    return 0; }
    if (!strcmp(s, "VIDEO"))      { *out = GMMU_APERTURE_VIDEO;      return 0; }
    if (!strcmp(s, "PEER"))       { *out = GMMU_APERTURE_PEER;       return 0; }
    if (!strcmp(s, "SYS_COH"))    { *out = GMMU_APERTURE_SYS_COH;    return 0; }
    if (!strcmp(s, "SYS_NONCOH")) { *out = GMMU_APERTURE_SYS_NONCOH; return 0; }
    return -1;
}

/* Index of a level pointer within the level array, or -1. */
static int level_index(const MMU_FMT_LEVEL *p)
{
    if (p == NULL)
        return -1;
    if (p < g_o.levels || p >= g_o.levels + g_o.numLevels)
        return -1;
    return (int)(p - g_o.levels);
}

/* ------------------------------------------------------------------------- */
/* Mode: levels — the topology, as the driver builds it                        */
/* ------------------------------------------------------------------------- */

static void mode_levels(void)
{
    NvU32 i;

    printf("chip=%s\n", OGKM_CHIP_NAME);
    printf("hal.init_levels=%s\n", OGKM_STR(OGKM_HAL_FMT_INIT_LEVELS));
    printf("hal.init_pde=%s\n", OGKM_STR(OGKM_HAL_FMT_INIT_PDE));
    printf("hal.init_pde_multi=%s\n", OGKM_STR(OGKM_HAL_FMT_INIT_PDE_MULTI));
    printf("hal.init_pte=%s\n", OGKM_STR(OGKM_HAL_FMT_INIT_PTE));
    printf("hal.families_init=%s\n", OGKM_STR(OGKM_HAL_FMT_FAMILIES_INIT));
    printf("fmt.version=%u\n", g_o.fmt.version);
    printf("fmt.sparse_hw=%u\n", (unsigned)(g_o.fmt.bSparseHwSupport ? 1 : 0));
    printf("fmt.max_entry_size=%u\n", (unsigned)GMMU_FMT_MAX_ENTRY_SIZE);
    printf("levels.allocated=%u\n", g_o.numLevels);

    for (i = 0; i < g_o.numLevels; ++i)
    {
        const MMU_FMT_LEVEL *L = &g_o.levels[i];
        NvU32 s;
        /* A level the driver never filled in has entrySize 0; report and stop. */
        if (L->entrySize == 0)
        {
            printf("level.%u.present=0\n", i);
            continue;
        }
        printf("level.%u.present=1\n", i);
        printf("level.%u.virt_hi=%u\n", i, (unsigned)L->virtAddrBitHi);
        printf("level.%u.virt_lo=%u\n", i, (unsigned)L->virtAddrBitLo);
        printf("level.%u.entry_size=%u\n", i, (unsigned)L->entrySize);
        printf("level.%u.num_sub_levels=%u\n", i, (unsigned)L->numSubLevels);
        printf("level.%u.b_page_table=%u\n", i, (unsigned)(L->bPageTable ? 1 : 0));
        printf("level.%u.page_level_id_tag=%u\n", i, (unsigned)L->pageLevelIdTag);
        /* The driver's OWN geometry arithmetic, not ours. */
        printf("level.%u.page_size=0x%llx\n", i,
               (unsigned long long)mmuFmtLevelPageSize(L));
        printf("level.%u.entry_count=%u\n", i, (unsigned)mmuFmtLevelEntryCount(L));
        printf("level.%u.level_size=%u\n", i, (unsigned)mmuFmtLevelSize(L));
        for (s = 0; s < MMU_FMT_MAX_SUB_LEVELS; ++s)
        {
            if (s < L->numSubLevels)
                printf("level.%u.sub.%u=%d\n", i, s, level_index(L->subLevels + s));
            else
                printf("level.%u.sub.%u=-1\n", i, s);
        }
    }

    /* The templates the driver builds for "this range is declared empty". */
    printf("sparse.pte=");       put_hex(g_o.fam.sparsePte.v8);      printf("\n");
    printf("sparse.pde=");       put_hex(g_o.fam.sparsePde.v8);      printf("\n");
    printf("sparse.pde_multi="); put_hex(g_o.fam.sparsePdeMulti.v8); printf("\n");
    printf("nv4k.pte=");         put_hex(g_o.fam.nv4kPte.v8);        printf("\n");
}

/* ------------------------------------------------------------------------- */
/* Mode: decode — the driver's own reading of raw entry bytes                  */
/* ------------------------------------------------------------------------- */

/*
 * ★★★ THE CENTRE OF THE ORACLE. `gmmuFmtEntryIsPte` is the driver's own answer to the
 * question `#13` got wrong: at a level that is BOTH `bPageTable` and a page directory —
 * which on GA10x is `PD1` (the 512 MiB leaf level) and `PD0` (the 2 MiB one) — the PTE's
 * valid field decides, and at a level that is only one of the two the answer is static
 * (`gmmu_fmt.c:40-64`). Nothing here re-states that rule.
 */
static void decode_one(int seq, NvU32 level, const NvU8 *entry, const char *echo)
{
    const MMU_FMT_LEVEL *L;
    NvBool               isPte;
    int                  before = g_asserts;

    /* ★ `seq` FIRST and the echoed input LAST, after every `key=value`. The echo
     * contains spaces, so a parser that met it first would have to know where the
     * fields end; this way it splits on " in=" and a desync is visible as a seq gap. */
    printf("seq=%d", seq);

    if (level >= g_o.numLevels || g_o.levels[level].entrySize == 0)
    {
        printf(" level_present=0 in=%s\n", echo);
        return;
    }
    L = &g_o.levels[level];
    printf(" level_present=1 entry_size=%u", (unsigned)L->entrySize);

    isPte = gmmuFmtEntryIsPte(&g_o.fmt, L, entry);
    printf(" is_pte=%u", (unsigned)(isPte ? 1 : 0));

    if (isPte)
    {
        const GMMU_FMT_PTE *P = g_o.fmt.pPte;
        GMMU_APERTURE       ap = gmmuFieldGetAperture(&P->fldAperture, entry);
        printf(" pte.valid=%u", (unsigned)(nvFieldGetBool(&P->fldValid, entry) ? 1 : 0));
        printf(" pte.aperture=%s", aperture_name(ap));
        {
            const GMMU_FIELD_ADDRESS *fld = gmmuFmtPtePhysAddrFld(P, ap);
            if (fld != NULL)
                printf(" pte.address=0x%llx",
                       (unsigned long long)gmmuFieldGetAddress(fld, entry));
            else
                printf(" pte.address=none");
        }
        printf(" pte.volatile=%u", (unsigned)(nvFieldGetBool(&P->fldVolatile, entry) ? 1 : 0));
        printf(" pte.read_only=%u", (unsigned)(nvFieldGetBool(&P->fldReadOnly, entry) ? 1 : 0));
        printf(" pte.privilege=%u", (unsigned)(nvFieldGetBool(&P->fldPrivilege, entry) ? 1 : 0));
        printf(" pte.encrypted=%u", (unsigned)(nvFieldGetBool(&P->fldEncrypted, entry) ? 1 : 0));
        printf(" pte.atomic_disable=%u",
               (unsigned)(nvFieldGetBool(&P->fldAtomicDisable, entry) ? 1 : 0));
        printf(" pte.kind=%u", (unsigned)nvFieldGet32(&P->fldKind, entry));
        printf(" pte.peer_index=%u", (unsigned)nvFieldGet32(&P->fldPeerIndex, entry));
        printf(" pte.comptagline=%u", (unsigned)nvFieldGet32(&P->fldCompTagLine, entry));
        printf(" leaf.page_size=0x%llx", (unsigned long long)mmuFmtLevelPageSize(L));
    }
    else
    {
        NvU32 s;
        printf(" pde.sub_levels=%u", (unsigned)L->numSubLevels);
        for (s = 0; s < L->numSubLevels; ++s)
        {
            const GMMU_FMT_PDE *D = gmmuFmtGetPde(&g_o.fmt, L, s);
            GMMU_APERTURE       ap;
            if (D == NULL)
            {
                printf(" pde.%u.aperture=?", s);
                continue;
            }
            ap = gmmuFieldGetAperture(&D->fldAperture, entry);
            printf(" pde.%u.aperture=%s", s, aperture_name(ap));
            if (ap == GMMU_APERTURE_INVALID)
            {
                /* `gmmuFmtPdePhysAddrFld` ASSERTS on INVALID and returns NULL — the
                 * driver's own statement that an absent sub-level has no address. Not
                 * called, so the assert counter stays a signal. */
                printf(" pde.%u.address=none", s);
            }
            else
            {
                const GMMU_FIELD_ADDRESS *fld = gmmuFmtPdePhysAddrFld(D, ap);
                if (fld != NULL)
                    printf(" pde.%u.address=0x%llx", s,
                           (unsigned long long)gmmuFieldGetAddress(fld, entry));
                else
                    printf(" pde.%u.address=none", s);
            }
            printf(" pde.%u.volatile=%u", s,
                   (unsigned)(nvFieldGetBool(&D->fldVolatile, entry) ? 1 : 0));
            printf(" pde.%u.child_level=%d", s, level_index(L->subLevels + s));
        }
    }

    printf(" asserts=%d in=%s\n", g_asserts - before, echo);
}

/* ------------------------------------------------------------------------- */
/* Mode: encode — the driver's own SETTERS, run forwards                       */
/* ------------------------------------------------------------------------- */

static void encode_one(int seq, char *line, const char *echo)
{
    char             *tok;
    GMMU_ENTRY_VALUE  v;
    int               before = g_asserts;

    memset(&v, 0, sizeof v);
    printf("seq=%d", seq);

    tok = strtok(line, " \t");
    if (tok == NULL)
    {
        printf(" error=empty in=%s\n", echo);
        return;
    }

    if (!strcmp(tok, "pte"))
    {
        /* pte <aperture> <addr-hex> <valid> <vol> <ro> <priv> <encrypted> <atomic_disable> <kind> <peer> */
        const GMMU_FMT_PTE *P = g_o.fmt.pPte;
        GMMU_APERTURE ap;
        unsigned long long addr;
        unsigned valid, vol, ro, priv, enc, atom, kind, peer;
        char *a = strtok(NULL, " \t");
        char *b = strtok(NULL, " \t");
        char *rest = strtok(NULL, "");
        if (a == NULL || b == NULL || rest == NULL || aperture_of_name(a, &ap) != 0 ||
            sscanf(b, "%llx", &addr) != 1 ||
            sscanf(rest, "%u %u %u %u %u %u %u %u", &valid, &vol, &ro, &priv, &enc,
                   &atom, &kind, &peer) != 8)
        {
            printf(" error=bad_pte_spec in=%s\n", echo);
            return;
        }
        nvFieldSetBool(&P->fldValid, valid ? NV_TRUE : NV_FALSE, v.v8);
        gmmuFieldSetAperture(&P->fldAperture, ap, v.v8);
        {
            const GMMU_FIELD_ADDRESS *fld = gmmuFmtPtePhysAddrFld(P, ap);
            if (fld != NULL)
                gmmuFieldSetAddress(fld, (NvU64)addr, v.v8);
        }
        nvFieldSetBool(&P->fldVolatile, vol ? NV_TRUE : NV_FALSE, v.v8);
        nvFieldSetBool(&P->fldReadOnly, ro ? NV_TRUE : NV_FALSE, v.v8);
        nvFieldSetBool(&P->fldPrivilege, priv ? NV_TRUE : NV_FALSE, v.v8);
        nvFieldSetBool(&P->fldEncrypted, enc ? NV_TRUE : NV_FALSE, v.v8);
        nvFieldSetBool(&P->fldAtomicDisable, atom ? NV_TRUE : NV_FALSE, v.v8);
        nvFieldSet32(&P->fldKind, (NvU32)kind, v.v8);
        if (ap == GMMU_APERTURE_PEER)
            nvFieldSet32(&P->fldPeerIndex, (NvU32)peer, v.v8);
    }
    else if (!strcmp(tok, "pde"))
    {
        /* pde <aperture> <addr-hex> <vol> */
        const GMMU_FMT_PDE *D = g_o.fmt.pPde;
        GMMU_APERTURE ap;
        unsigned long long addr;
        unsigned vol;
        char *a = strtok(NULL, " \t");
        char *b = strtok(NULL, " \t");
        char *c = strtok(NULL, " \t");
        if (a == NULL || b == NULL || c == NULL || aperture_of_name(a, &ap) != 0 ||
            sscanf(b, "%llx", &addr) != 1 || sscanf(c, "%u", &vol) != 1)
        {
            printf(" error=bad_pde_spec in=%s\n", echo);
            return;
        }
        gmmuFieldSetAperture(&D->fldAperture, ap, v.v8);
        if (ap != GMMU_APERTURE_INVALID)
        {
            const GMMU_FIELD_ADDRESS *fld = gmmuFmtPdePhysAddrFld(D, ap);
            if (fld != NULL)
                gmmuFieldSetAddress(fld, (NvU64)addr, v.v8);
        }
        nvFieldSetBool(&D->fldVolatile, vol ? NV_TRUE : NV_FALSE, v.v8);
    }
    else if (!strcmp(tok, "dual"))
    {
        /* dual <ap0> <addr0-hex> <vol0> <ap1> <addr1-hex> <vol1>
         * Sub-level 0 and 1 are whatever `gmmuFmtGetPde` says they are — the harness
         * does NOT name one "big": which half is which is the driver's statement,
         * reported back by `mode_levels`' sub-level indices. */
        unsigned s;
        char *fields[6];
        for (s = 0; s < 6; ++s)
        {
            fields[s] = strtok(NULL, " \t");
            if (fields[s] == NULL)
            {
                printf(" error=bad_dual_spec in=%s\n", echo);
                return;
            }
        }
        for (s = 0; s < 2; ++s)
        {
            const GMMU_FMT_PDE *D = &g_o.fmt.pPdeMulti->subLevels[s];
            GMMU_APERTURE ap;
            unsigned long long addr;
            unsigned vol;
            if (aperture_of_name(fields[s * 3], &ap) != 0 ||
                sscanf(fields[s * 3 + 1], "%llx", &addr) != 1 ||
                sscanf(fields[s * 3 + 2], "%u", &vol) != 1)
            {
                printf(" error=bad_dual_spec in=%s\n", echo);
                return;
            }
            gmmuFieldSetAperture(&D->fldAperture, ap, v.v8);
            if (ap != GMMU_APERTURE_INVALID)
            {
                const GMMU_FIELD_ADDRESS *fld = gmmuFmtPdePhysAddrFld(D, ap);
                if (fld != NULL)
                    gmmuFieldSetAddress(fld, (NvU64)addr, v.v8);
            }
            nvFieldSetBool(&D->fldVolatile, vol ? NV_TRUE : NV_FALSE, v.v8);
        }
    }
    else
    {
        printf(" error=unknown_kind in=%s\n", echo);
        return;
    }

    printf(" out=");
    put_hex(v.v8);
    printf(" asserts=%d in=%s\n", g_asserts - before, echo);
}

/* ------------------------------------------------------------------------- */
/* main                                                                        */
/* ------------------------------------------------------------------------- */

static void usage(void)
{
    fprintf(stderr,
            "usage: gmmu_fmt_oracle <levels|decode|encode> <bigPageShift> <unifiedAperture 0|1>\n"
            "  levels : print the topology + sparse templates on stdout\n"
            "  decode : read `<level> <32 hex chars>` lines on stdin, one result line each\n"
            "  encode : read `pte|pde|dual …` spec lines on stdin, one result line each\n");
}

int main(int argc, char **argv)
{
    char line[1024];
    unsigned shift, unified;

    if (argc != 4)
    {
        usage();
        return 2;
    }
    if (sscanf(argv[2], "%u", &shift) != 1 || sscanf(argv[3], "%u", &unified) != 1)
    {
        usage();
        return 2;
    }

    build_fmt(shift, unified ? NV_TRUE : NV_FALSE);

    if (!strcmp(argv[1], "levels"))
    {
        mode_levels();
    }
    else if (!strcmp(argv[1], "decode"))
    {
        int seq = 0;
        while (fgets(line, sizeof line, stdin) != NULL)
        {
            char  echo[1024];
            char *nl = strchr(line, '\n');
            unsigned level;
            NvU8 entry[GMMU_FMT_MAX_ENTRY_SIZE];
            char hex[128];
            if (nl != NULL)
                *nl = '\0';
            if (line[0] == '\0' || line[0] == '#')
                continue;
            snprintf(echo, sizeof echo, "%s", line);
            if (sscanf(line, "%u %127s", &level, hex) != 2 || get_hex(hex, entry) != 0)
            {
                printf("seq=%d error=bad_decode_line in=%s\n", seq++, echo);
                continue;
            }
            decode_one(seq++, level, entry, echo);
        }
    }
    else if (!strcmp(argv[1], "encode"))
    {
        int seq = 0;
        while (fgets(line, sizeof line, stdin) != NULL)
        {
            char  echo[1024];
            char *nl = strchr(line, '\n');
            if (nl != NULL)
                *nl = '\0';
            if (line[0] == '\0' || line[0] == '#')
                continue;
            snprintf(echo, sizeof echo, "%s", line);
            encode_one(seq++, line, echo);
        }
    }
    else
    {
        usage();
        return 2;
    }

    printf("asserts.total=%d\n", g_asserts);
    if (g_asserts > 0)
        printf("asserts.first=%s\n", g_first_assert);
    printf("done=1\n");
    return 0;
}
