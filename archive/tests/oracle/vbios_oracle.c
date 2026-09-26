/*
 * vbios_oracle — a TEST-ONLY differential oracle built out of NVIDIA's OWN VBIOS parser.
 *
 * ============================================================================
 * WHY THIS EXISTS
 * ============================================================================
 *
 * `kayfabe_abi::vbios` builds a synthetic VBIOS image, and its unit tests parse that
 * image back with a Rust reader. Both were written by reading the same C. The author of
 * the generator said so plainly in the module doc: *"the test oracle is a transcription,
 * not independent. If I misread the C, builder and parser are wrong the same way and stay
 * green."* A transcribed parser cannot detect a shared misreading, by construction — only
 * a real guest boot could, and that costs a VM, a GPU and a whole system.
 *
 * This program removes the hole rather than shrinking it. It compiles the ACTUAL parser
 * out of the vendored open kernel modules —
 *
 *   src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c
 *   src/nvidia/src/kernel/gpu/gsp/kernel_gsp_fwsec.c
 *
 * — UNMODIFIED, against those trees' OWN headers, and runs our generated image through
 * `kgspExtractVbiosFromRom_TU102` -> `kgspParseFwsecUcodeFromVbiosImg_IMPL`. There is no
 * transcription anywhere in the path, so there is nothing left for a misreading to be
 * shared with.
 *
 * ============================================================================
 * LICENSING — read before copying anything into this repository
 * ============================================================================
 *
 * The two parser sources above are NVIDIA's, dual-licensed MIT / GPL-2.0 (each carries
 * `SPDX-License-Identifier: MIT` in its header). Compiling a slice of them for testing is
 * within the MIT grant. Deliberately, NOTHING from those trees is vendored into this
 * repository: this file `#include`s the .c sources BY ABSOLUTE PATH out of the checkout
 * that already exists beside it, and `tests/build.rs` refuses (loudly) rather than
 * substituting a copy when that checkout is absent. If a copy is ever wanted, the MIT
 * notice has to come with it.
 *
 * ============================================================================
 * HOW IT IS FAITHFUL
 * ============================================================================
 *
 * The image is served through a stubbed `GPU_REG_RD32`/`RD08` at the PROM window
 * (`NV_PROM_DATA`), which is EXACTLY the path a real device is read over — the driver
 * does not get a buffer handed to it, it does dword reads of a register window and
 * assembles the image itself. So the harness mirrors the real data path rather than
 * approximating it, including the unaligned-read assembly in `s_romImgReadGeneric`.
 *
 * Everything else the two parser files touch is a stub, and there are only twelve:
 * malloc / memset / memcpy, an alloc-and-return-a-pointer for the memdescs, two log
 * sinks, and `kgspIsDebugModeEnabled`, which is a per-object FUNCTION POINTER on
 * `KernelGsp` — so both FWSEC selection paths (debug-fused and prod-fused) are drivable
 * from a command-line flag rather than guessed at.
 *
 * `portSafeAddU32` — the bounds check every offset in this path goes through — is NOT
 * stubbed: it is `PORT_SAFE_INLINE` in `nvport/safe.h` and is compiled in from the real
 * header.
 *
 * ============================================================================
 * PROTOCOL
 * ============================================================================
 *
 *   argv[1] = path to the VBIOS image file
 *   argv[2] = "debug" | "prod"   (drives kgspIsDebugModeEnabled)
 *
 * stdout: `key=value` lines, one per line, ASCII, stable order. `log=` lines are the
 * driver's OWN diagnostics, in the order it emitted them, with the source file and line
 * it emitted them from. Exit status 0 means the harness ran to completion; the parser's
 * verdict is in `extract_status` / `parse_status`, never in the exit status. A crash in
 * the parser therefore shows up as a SIGNAL on this process, which is a verdict the
 * caller can see rather than a hang.
 */

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* The vendored tree's own headers. OGKM_SRC_NVIDIA is -D'd by tests/build.rs. */
#include "gpu/gsp/kernel_gsp.h"
#include "gpu/gpu.h"
#include "gpu/mem_mgr/mem_desc.h"
#include "published/turing/tu102/dev_ext_devices.h"

/* ------------------------------------------------------------------------- */
/* The image, served at the PROM window                                       */
/* ------------------------------------------------------------------------- */

static const unsigned char *g_image;
static unsigned int g_image_len;
static unsigned long g_prom_rd32;
static unsigned long g_prom_rd08;

/*
 * A byte outside the image. A real PROM window answers SOMETHING for every offset in it;
 * what it answers past the end of the ROM is not ours to decide, so this reads back 0xFF
 * (an erased flash cell) rather than 0x00, which would be indistinguishable from a
 * legitimately zero image byte.
 */
#define PROM_UNMAPPED_BYTE 0xFF

static unsigned char prom_byte(unsigned int off)
{
    if (off < g_image_len)
        return g_image[off];
    return PROM_UNMAPPED_BYTE;
}

/* ------------------------------------------------------------------------- */
/* The diagnostic sink — the driver's own messages, verbatim                   */
/* ------------------------------------------------------------------------- */

#define LOG_CAP 128
#define LOG_LINE 512
static char g_log[LOG_CAP][LOG_LINE];
static int g_log_n;

static void log_add(const char *file, int line, const char *kind, const char *fmt, va_list ap)
{
    char body[LOG_LINE - 128]; /* leaves room for the "KIND file:line " prefix */
    const char *base;

    if (g_log_n >= LOG_CAP)
        return;

    vsnprintf(body, sizeof(body), fmt, ap);

    /* Trim the trailing newline the driver's format strings carry. */
    {
        size_t n = strlen(body);
        while (n > 0 && (body[n - 1] == '\n' || body[n - 1] == '\r'))
            body[--n] = '\0';
    }

    base = strrchr(file, '/');
    base = base ? base + 1 : file;

    snprintf(g_log[g_log_n], LOG_LINE, "%s %s:%d %s", kind, base, line, body);
    g_log_n++;
}

/* ------------------------------------------------------------------------- */
/* The twelve stubs                                                           */
/* ------------------------------------------------------------------------- */

void *portMemAllocNonPaged(NvLength length)
{
    /* The driver calls this with 0 in at least one reachable path (a descriptor that
     * declares SignatureCount == 0); malloc(0) may return NULL, which the driver would
     * read as NV_ERR_NO_MEMORY and mask the real behaviour. Round up to 1. */
    return malloc(length ? (size_t) length : 1u);
}

void *portMemSet(void *p, NvU8 v, NvLength n)
{
    return memset(p, v, (size_t) n);
}

void *portMemCopy(void *dst, NvLength dstSize, const void *src, NvLength srcSize)
{
    if (srcSize > dstSize)
        return NULL; /* nvport's own contract */
    return memcpy(dst, src, (size_t) srcSize);
}

void nvDbg_Printf(const char *file, int line, const char *function, int level,
                  const char *s, ...)
{
    va_list ap;
    (void) function;
    (void) level;
    va_start(ap, s);
    log_add(file, line, "PRINTF", s, ap);
    va_end(ap);
}

static void log_plain(const char *file, int line, const char *kind, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    log_add(file, line, kind, fmt, ap);
    va_end(ap);
}

void nvAssertFailedNoLog(const char *pszExpr, const char *pszFileName, NvU32 lineNum)
{
    log_plain(pszFileName, (int) lineNum, "ASSERT", "%s", pszExpr);
}

void nvAssertOkFailedNoLog(NvU32 status, const char *pszExpr, const char *pszFileName,
                           NvU32 lineNum)
{
    log_plain(pszFileName, (int) lineNum, "ASSERT_OK", "0x%08x from %s", status, pszExpr);
}

/* --- the memdesc registry ------------------------------------------------- */
/*
 * The parser allocates memdescs, maps them, copies the ucode into the mapping and
 * unmaps. The mapping is what the falcon would be handed, so it is the single most
 * interesting artefact this oracle produces — the registry keeps each buffer alive past
 * the unmap so the caller can inspect what the REAL parser decided to deliver.
 */
#define MEMDESC_CAP 8
static struct {
    MEMORY_DESCRIPTOR *desc;
    unsigned char *buf;
    unsigned long size;
} g_memdesc[MEMDESC_CAP];
static int g_memdesc_n;

NV_STATUS memdescCreate(MEMORY_DESCRIPTOR **ppMemDesc, OBJGPU *pGpu, NvU64 Size,
                        NvU64 alignment, NvBool contiguous, NV_ADDRESS_SPACE addrSpace,
                        NvU32 cacheAttrib, NvU64 flags)
{
    MEMORY_DESCRIPTOR *d;

    (void) pGpu; (void) alignment; (void) contiguous;
    (void) addrSpace; (void) cacheAttrib; (void) flags;

    if (g_memdesc_n >= MEMDESC_CAP)
        return NV_ERR_INSUFFICIENT_RESOURCES;

    /*
     * `memdescTagAlloc` writes `pMemdesc->allocTag`, so this must be a real
     * MEMORY_DESCRIPTOR of the tree's own size — not an opaque handle.
     */
    d = calloc(1, sizeof(*d));
    if (d == NULL)
        return NV_ERR_NO_MEMORY;

    g_memdesc[g_memdesc_n].desc = d;
    g_memdesc[g_memdesc_n].buf = NULL;
    g_memdesc[g_memdesc_n].size = (unsigned long) Size;
    g_memdesc_n++;

    *ppMemDesc = d;
    return NV_OK;
}

static int memdesc_slot(const MEMORY_DESCRIPTOR *d)
{
    int i;
    for (i = 0; i < g_memdesc_n; i++)
        if (g_memdesc[i].desc == d)
            return i;
    return -1;
}

NV_STATUS memdescAlloc(MEMORY_DESCRIPTOR *pMemDesc)
{
    int i = memdesc_slot(pMemDesc);
    if (i < 0)
        return NV_ERR_INVALID_ARGUMENT;
    /*
     * Poisoned, not zeroed. The parser is expected to write EVERY byte it promises to
     * deliver; a zero-filled arena would make a short copy indistinguishable from a
     * correctly-copied run of zeros — which is precisely the class of bug this whole
     * oracle exists to catch.
     */
    g_memdesc[i].buf = malloc(g_memdesc[i].size ? g_memdesc[i].size : 1u);
    if (g_memdesc[i].buf == NULL)
        return NV_ERR_NO_MEMORY;
    memset(g_memdesc[i].buf, 0xDB, g_memdesc[i].size);
    return NV_OK;
}

void *memdescMapInternal(OBJGPU *pGpu, MEMORY_DESCRIPTOR *pMemDesc, NvU32 flags)
{
    int i = memdesc_slot(pMemDesc);
    (void) pGpu; (void) flags;
    return (i < 0) ? NULL : g_memdesc[i].buf;
}

void memdescUnmapInternal(OBJGPU *pGpu, MEMORY_DESCRIPTOR *pMemDesc, NvU32 flags)
{
    (void) pGpu; (void) pMemDesc; (void) flags;
    /* The buffer deliberately outlives the mapping; see the registry comment. */
}

/* --- the register window -------------------------------------------------- */

DEVICE_MAPPING *gpuGetDeviceMapping_IMPL(struct OBJGPU *pGpu, DEVICE_INDEX idx, NvU32 inst)
{
    static DEVICE_MAPPING mapping;
    (void) pGpu; (void) idx; (void) inst;
    return &mapping;
}

NvU32 osDevReadReg032(OBJGPU *pGpu, DEVICE_MAPPING *pMapping, NvU32 addr)
{
    NvU32 off = addr - NV_PROM_DATA(0);
    (void) pGpu; (void) pMapping;
    g_prom_rd32++;
    return (NvU32) prom_byte(off)
         | ((NvU32) prom_byte(off + 1) << 8)
         | ((NvU32) prom_byte(off + 2) << 16)
         | ((NvU32) prom_byte(off + 3) << 24);
}

NvU8 osDevReadReg008(OBJGPU *pGpu, DEVICE_MAPPING *pMapping, NvU32 addr)
{
    (void) pGpu; (void) pMapping;
    g_prom_rd08++;
    return prom_byte(addr - NV_PROM_DATA(0));
}

/* --- the fuse read, as a parameter ---------------------------------------- */
/*
 * ★ `kgspIsDebugModeEnabled` is a per-object function POINTER on `KernelGsp`
 * (`__kgspIsDebugModeEnabled__`), not a compiled-in decision — so the harness can drive
 * BOTH FWSEC selection arms without emulating a fuse register. The generator claims its
 * image is found under debug AND prod fusing; that claim is checkable only because this
 * is a parameter.
 */
static NvBool oracle_debug_mode_on(struct OBJGPU *pGpu, struct KernelGsp *pKernelGsp)
{
    (void) pGpu; (void) pKernelGsp;
    return NV_TRUE;
}

static NvBool oracle_debug_mode_off(struct OBJGPU *pGpu, struct KernelGsp *pKernelGsp)
{
    (void) pGpu; (void) pKernelGsp;
    return NV_FALSE;
}

/* --- the two free helpers the parser calls on its own error paths ---------- */

void kgspFreeVbiosImg(KernelGspVbiosImg *pVbiosImg)
{
    if (pVbiosImg == NULL)
        return;
    free(pVbiosImg->pImage);
    free(pVbiosImg);
}

void kgspFreeFlcnUcode(KernelGspFlcnUcode *pFlcnUcode)
{
    if (pFlcnUcode == NULL)
        return;
    if (pFlcnUcode->bootType == KGSP_FLCN_UCODE_BOOT_FROM_HS)
        free(pFlcnUcode->ucodeBootFromHs.pSignatures);
    free(pFlcnUcode);
}

/* ------------------------------------------------------------------------- */
/* The parser itself — NVIDIA's sources, unmodified, compiled in              */
/* ------------------------------------------------------------------------- */
/*
 * Included rather than compiled separately so that this whole oracle is ONE translation
 * unit per vendored tag: two tags then coexist as two EXECUTABLES with no symbol
 * renaming anywhere, and nothing in the build has to know which symbols the parser
 * happens to export.
 */
#include OGKM_VBIOS_TU102_C
#include OGKM_FWSEC_C

/*
 * ★ AND ONE STAGE FURTHER: the FWSEC INTERFACE WALK.
 *
 * `kgspParseFwsecUcodeFromVbiosImg` above reads `InterfaceOffset` as an opaque number
 * and never follows it. What follows it is `s_vbiosPatchInterfaceData`, in
 * kernel_gsp_frts_tu102.c — and that is where a stock guest with a perfectly parseable
 * ROM stops, with "failed to find required interface entry for FWSEC cmd 0x15". A ROM
 * oracle that ends at the parser cannot see that plane at all.
 *
 * That file cannot be included whole: it also holds `s_prepareForFwsec_TU102` and the
 * FRTS entry points, which need HAL dispatch, KernelFalcon and memdescMapInternal. So
 * `tests/build.rs` cuts a BYTE-FOR-BYTE RANGE of it — the three interface typedefs, the
 * FWSECLIC command structures, and `s_vbiosPatchInterfaceData` — into OUT_DIR and points
 * OGKM_FRTS_INTERFACE_SLICE at the result. The cut is delimited by the file's own text
 * (not by line number) and build.rs asserts the anchors the slice must and must not
 * contain, so a slice that caught the wrong region is a build failure rather than a
 * quieter oracle. Nothing in it is rewritten.
 */
#include OGKM_FRTS_INTERFACE_SLICE

/* ------------------------------------------------------------------------- */
/* Reporting                                                                  */
/* ------------------------------------------------------------------------- */

static unsigned long long fnv1a(const unsigned char *p, unsigned long n)
{
    unsigned long long h = 1469598103934665603ULL;
    unsigned long i;
    for (i = 0; i < n; i++) {
        h ^= p[i];
        h *= 1099511628211ULL;
    }
    return h;
}

static void print_hex(const char *key, const unsigned char *p, unsigned long n)
{
    unsigned long i;
    printf("%s=", key);
    for (i = 0; i < n; i++)
        printf("%02x", p[i]);
    printf("\n");
}

/* ------------------------------------------------------------------------- */
/* Stage two: the real interface walk, over the real parser's own output      */
/* ------------------------------------------------------------------------- */
/*
 * `s_prepareForFwsec_TU102` is NOT compiled in (see the include note above), so the two
 * things it supplies are reproduced here and only here:
 *
 *   - the DMEM base. `pMappedData = memdescMapInternal(...) + pUcode->dataOffset`, and
 *     `mappedDataSize = pUcode->dmemSize`. Both come straight off the KernelGsp*
 *     structure the REAL parser filled in, so neither is a number this harness chose.
 *
 *   - a command buffer. The walk copies `cmdBufferSize` bytes verbatim and never looks
 *     inside them, so the CONTENT is irrelevant and only the LENGTH is load-bearing —
 *     and the length is `sizeof()` of NVIDIA's own typedef, out of the same slice. The
 *     buffer is filled with a recognisable ramp and printed, so the caller checks the
 *     copy landed against the bytes that were actually passed rather than against a
 *     constant both sides agreed on.
 *
 * The DMEM is snapshotted and restored between commands, so FRTS and SB are each run
 * against the pristine image the parser produced.
 */
static void print_walk(const char *prefix, NV_STATUS st, const unsigned char *dmem,
                       unsigned long dmem_len)
{
    char key[64];
    snprintf(key, sizeof(key), "%s_status", prefix);
    printf("%s=0x%08x\n", key, (unsigned) st);
    snprintf(key, sizeof(key), "%s_dmem", prefix);
    print_hex(key, dmem, dmem_len);
}

static void run_interface_walk(const KernelGspFlcnUcodeBootFromHs *u,
                               const unsigned char *ucode, unsigned long ucode_len)
{
    unsigned char *mapped;
    unsigned char *pristine;
    unsigned char *data;
    FWSECLIC_FRTS_CMD frtsCmd;
    FWSECLIC_READ_VBIOS_DESC sbCmd;
    NV_STATUS st;
    unsigned i;

    printf("frts_cmd_size=%lu\n", (unsigned long) sizeof(FWSECLIC_FRTS_CMD));
    printf("read_vbios_desc_size=%lu\n", (unsigned long) sizeof(FWSECLIC_READ_VBIOS_DESC));
    printf("interface_hdr_size=%lu\n",
           (unsigned long) sizeof(FALCON_APPLICATION_INTERFACE_HEADER_V1));
    printf("interface_entry_size=%lu\n",
           (unsigned long) sizeof(FALCON_APPLICATION_INTERFACE_ENTRY_V1));
    printf("dmem_mapper_size=%lu\n",
           (unsigned long) sizeof(FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3));
    printf("dmemmapper_entry_id=%u\n", (unsigned) FALCON_APPLICATION_INTERFACE_ENTRY_ID_DMEMMAPPER);

    /* The walk mutates DMEM in place; work on a copy of the parser's buffer. */
    if (u->dataOffset > ucode_len || u->dmemSize > ucode_len - u->dataOffset) {
        printf("walk_skipped=dmem-out-of-ucode\n");
        return;
    }
    mapped = malloc(ucode_len);
    pristine = malloc(u->dmemSize);
    if (mapped == NULL || pristine == NULL)
        return;
    memcpy(mapped, ucode, ucode_len);
    data = mapped + u->dataOffset;
    memcpy(pristine, data, u->dmemSize);
    print_hex("walk_dmem_before", pristine, u->dmemSize);

    for (i = 0; i < sizeof(frtsCmd); i++)
        ((unsigned char *) &frtsCmd)[i] = (unsigned char) (0x5Au ^ (i % 251u));
    for (i = 0; i < sizeof(sbCmd); i++)
        ((unsigned char *) &sbCmd)[i] = (unsigned char) (0xC3u ^ (i % 251u));
    print_hex("frts_cmd_bytes", (const unsigned char *) &frtsCmd, sizeof(frtsCmd));
    print_hex("sb_cmd_bytes", (const unsigned char *) &sbCmd, sizeof(sbCmd));

    st = s_vbiosPatchInterfaceData(data, u->dmemSize,
                                   FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
                                   &frtsCmd, (NvU32) sizeof(frtsCmd), u->interfaceOffset);
    print_walk("walk_frts", st, data, u->dmemSize);

    memcpy(data, pristine, u->dmemSize);
    st = s_vbiosPatchInterfaceData(data, u->dmemSize,
                                   FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_SB,
                                   &sbCmd, (NvU32) sizeof(sbCmd), u->interfaceOffset);
    print_walk("walk_sb", st, data, u->dmemSize);

    free(pristine);
    free(mapped);
}

int main(int argc, char **argv)
{
    FILE *f;
    long len;
    unsigned char *img;
    int use_debug;
    OBJGPU *pGpu;
    KernelGsp *pKernelGsp;
    KernelGspVbiosImg *pVbiosImg = NULL;
    KernelGspFlcnUcode *pFwsec = NULL;
    NvU64 vbiosVersionCombined = 0;
    NV_STATUS extract_status, parse_status;
    int i;

    if (argc < 3) {
        fprintf(stderr, "usage: %s <image> <debug|prod>\n", argv[0]);
        return 2;
    }
    use_debug = (strcmp(argv[2], "debug") == 0);

    f = fopen(argv[1], "rb");
    if (f == NULL) {
        fprintf(stderr, "cannot open %s\n", argv[1]);
        return 2;
    }
    fseek(f, 0, SEEK_END);
    len = ftell(f);
    fseek(f, 0, SEEK_SET);
    img = malloc((size_t) (len > 0 ? len : 1));
    if (img == NULL || fread(img, 1, (size_t) len, f) != (size_t) len) {
        fprintf(stderr, "cannot read %s\n", argv[1]);
        return 2;
    }
    fclose(f);
    g_image = img;
    g_image_len = (unsigned int) len;

    /*
     * A zeroed OBJGPU with exactly the two predicates the parser gates on. `pKernelBif`
     * stays NULL, which makes `kgspExtractVbiosFromRom_TU102` skip the EEPROM
     * request/grant handshake — a real bare-metal GA10x has no ERoT either.
     */
    pGpu = calloc(1, sizeof(*pGpu));
    pKernelGsp = calloc(1, sizeof(*pKernelGsp));
    if (pGpu == NULL || pKernelGsp == NULL)
        return 2;
    pGpu->isVirtual = NV_FALSE;
    pGpu->isGspClient = NV_TRUE;
    pKernelGsp->__kgspIsDebugModeEnabled__ =
        use_debug ? oracle_debug_mode_on : oracle_debug_mode_off;

    printf("image_len=%u\n", g_image_len);
    printf("debug_mode=%d\n", use_debug);

    extract_status = kgspExtractVbiosFromRom_TU102(pGpu, pKernelGsp, &pVbiosImg);
    printf("extract_status=0x%08x\n", (unsigned) extract_status);
    printf("vbios_img_present=%d\n", pVbiosImg != NULL ? 1 : 0);
    if (extract_status == NV_OK && pVbiosImg != NULL) {
        printf("bios_size=%u\n", (unsigned) pVbiosImg->biosSize);
        printf("expansion_rom_offset=%u\n", (unsigned) pVbiosImg->expansionRomOffset);

        parse_status = kgspParseFwsecUcodeFromVbiosImg_IMPL(pGpu, pKernelGsp, pVbiosImg,
                                                            &pFwsec, &vbiosVersionCombined);
        printf("parse_status=0x%08x\n", (unsigned) parse_status);
        /*
         * ★ NOT redundant with parse_status. `s_vbiosNewFlcnUcodeFromDesc` frees the
         * ucode and returns NV_OK when the V2/V3 fill fails, so the ONLY signal that the
         * descriptor was rejected is this pointer being NULL. Reported separately for
         * exactly that reason.
         */
        printf("fwsec_present=%d\n", pFwsec != NULL ? 1 : 0);
        printf("vbios_version_combined=0x%llx\n", (unsigned long long) vbiosVersionCombined);

        if (pFwsec != NULL) {
            printf("boot_type=%d\n", (int) pFwsec->bootType);
            if (pFwsec->bootType == KGSP_FLCN_UCODE_BOOT_FROM_HS) {
                KernelGspFlcnUcodeBootFromHs *u = &pFwsec->ucodeBootFromHs;
                int slot = memdesc_slot(u->pUcodeMemDesc);

                printf("ucode_size=%u\n", (unsigned) u->size);
                printf("code_offset=%u\n", (unsigned) u->codeOffset);
                printf("imem_size=%u\n", (unsigned) u->imemSize);
                printf("imem_pa=%u\n", (unsigned) u->imemPa);
                printf("imem_va=%u\n", (unsigned) u->imemVa);
                printf("data_offset=%u\n", (unsigned) u->dataOffset);
                printf("dmem_size=%u\n", (unsigned) u->dmemSize);
                printf("dmem_pa=%u\n", (unsigned) u->dmemPa);
                printf("dmem_va=0x%x\n", (unsigned) u->dmemVa);
                printf("hs_sig_dmem_addr=%u\n", (unsigned) u->hsSigDmemAddr);
                printf("ucode_id=%u\n", (unsigned) u->ucodeId);
                printf("engine_id_mask=%u\n", (unsigned) u->engineIdMask);
                printf("sig_count=%u\n", (unsigned) u->sigCount);
                printf("sig_size=%u\n", (unsigned) u->sigSize);
                printf("signatures_total_size=%u\n", (unsigned) u->signaturesTotalSize);
                printf("vbios_sig_versions=%u\n", (unsigned) u->vbiosSigVersions);
                printf("interface_offset=%u\n", (unsigned) u->interfaceOffset);

                if (u->pSignatures != NULL && u->signaturesTotalSize > 0) {
                    const unsigned char *s = (const unsigned char *) u->pSignatures;
                    printf("sig_fnv1a=0x%016llx\n", fnv1a(s, u->signaturesTotalSize));
                    print_hex("sig_head", s,
                              u->signaturesTotalSize < 32 ? u->signaturesTotalSize : 32);
                }
                if (slot >= 0 && g_memdesc[slot].buf != NULL) {
                    const unsigned char *b = g_memdesc[slot].buf;
                    unsigned long n = g_memdesc[slot].size;
                    printf("ucode_mem_size=%lu\n", n);
                    printf("ucode_fnv1a=0x%016llx\n", fnv1a(b, n));
                    print_hex("ucode_head", b, n < 32 ? n : 32);
                    print_hex("ucode_tail", n < 32 ? b : b + n - 32, n < 32 ? n : 32);

                    run_interface_walk(u, b, n);
                }
            }
        }
    }

    printf("prom_rd32=%lu\n", g_prom_rd32);
    printf("prom_rd08=%lu\n", g_prom_rd08);
    printf("log_lines=%d\n", g_log_n);
    for (i = 0; i < g_log_n; i++)
        printf("log=%s\n", g_log[i]);

    fflush(stdout);
    return 0;
}
