/*
 * userd_chid_oracle — a TEST-ONLY differential oracle built out of NVIDIA's OWN
 * USERD-index writer, its OWN reader, and its OWN recombination.
 *
 * ============================================================================
 * WHY THIS EXISTS — and why it compiles FOUR spans instead of one
 * ============================================================================
 *
 * `kayfabe_arch::Arch::vchid_from_userd_flags` turns the `NVOS04_FLAGS_*` word CPU-RM
 * puts on the `ALLOC_CHANNEL` RPC into the channel the guest means. On the Mode-2 path we
 * ARE physical RM, so a wrong recovery does not fail — it routes a guest's channel to
 * another channel, and there is no second party to notice. That is the same silence
 * `worksubmit_token_oracle.c` was built for, and this file is built the same way.
 *
 * ★★★ The reason it is not a three-line decode with a three-line oracle: **the writer's
 * divisor and the reader's multiplier are two different numbers, arrived at by two
 * unrelated routes.**
 *
 *   - The WRITER (`kernel_channel.c`) splits the chid with
 *     `numChannelsPerUserd = NVBIT(DRF_SIZE(NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE))` —
 *     a number that comes from the WIDTH OF A FLAG FIELD.
 *   - The READER (`kchannelAllocHwID_*`) extracts the two subfields and passes them down
 *     SEPARATELY; it never combines them.
 *   - The RECOMBINATION (`kfifoChidMgrAllocChid_IMPL`) multiplies by
 *     `pChidMgr->pGlobalChIDHeap->ownerGranularity`, which was set from
 *     `RM_PAGE_SIZE / userdBar1Size` — a number that comes from A PAGE SIZE DIVIDED BY
 *     THE SIZE OF A USERD, sized by the halified `kfifoGetUserdSizeAlign` out of
 *     `dev_ram.h`.
 *
 * They happen to be equal on GA106. **An oracle that assumed that equality would be
 * asserting the thing most likely to be wrong**, so this harness computes neither: it
 * runs the writer, runs the reader, runs the recombination, and PRINTS the granularity
 * the driver's own eheap ended up holding. If a release ever moves one and not the other,
 * the printed round trip stops closing and the test says so.
 *
 * ⊘ **If you find `8` written anywhere in this file, the oracle has been defeated.**
 *
 * ============================================================================
 * WHAT IS COMPILED, AND WHAT IS OURS
 * ============================================================================
 *
 * The only NVIDIA code here is:
 *
 *   1. The WRITER span — a contiguous run of `kernel_channel.c`'s own bytes, from the
 *      `numChannelsPerUserd` declaration through the `_PAGE_VALUE` `FLD_SET_DRF_NUM`. The
 *      flags word this oracle decodes is therefore bit-for-bit the one CPU-RM puts on our
 *      wire, including the `_FIXED=_FALSE` / `_PAGE_FIXED=_TRUE` it sets alongside.
 *
 *   2. The READER — `kchannelAllocHwID_<CHIP>` itself, sliced whole out of the file the
 *      driver's OWN dispatch table binds for the chip (`tests/build.rs` parses
 *      `g_kernel_channel_nvoc.c`; on GA106 that is `_GM107`). With it come RM's own
 *      validity asserts, which is what makes the two malformed shapes below refusals
 *      rather than guesses. `kfifoChidMgrAllocChid` is stubbed to RECORD its arguments
 *      rather than allocate, so the eheap allocation path is never entered — but the
 *      extraction and the asserts are the driver's.
 *
 *   3. The RECOMBINATION span — the `if (bForceUserdPage) … else if (bForceInternalIdx) …`
 *      block from `kfifoChidMgrAllocChid_IMPL`'s **non-VF** arm, byte for byte. The
 *      multiply is RM's, and so is the `NV_ASSERT_OR_RETURN(!bForceInternalIdx)` in it.
 *
 *   4. The GRANULARITY span — the three statements in `kfifoChidMgrConstructChidMgr` that
 *      size a USERD (`kfifoGetUserdSizeAlign_HAL`) and hand `RM_PAGE_SIZE / userdBar1Size`
 *      to `eheapSetOwnerIsolation`. The HAL pointer is set to the symbol the DRIVER'S own
 *      `g_kernel_fifo_nvoc.c` binds for this chip, and the eheap is the driver's real
 *      `eheap_old.c`, compiled whole — so `ownerGranularity` is stored by NVIDIA's own
 *      setter and read back by NVIDIA's own recombination. Nothing in between is ours.
 *
 *   5. Everything those read: `nvmisc.h`'s `DRF_*` family, `alloc_channel.h`'s field
 *      extents, `dev_ram.h`'s `NV_RAMUSERD_BASE_SHIFT`, `rm_page_size.h`'s
 *      `RM_PAGE_SIZE` — all reached through the sliced files' OWN `#include` lines.
 *      NO BIT POSITION, FIELD WIDTH, PAGE SIZE OR DIVISOR IS TRANSCRIBED IN THIS FILE.
 *
 * Everything else is a stub. They stand in for the object model, the client database and
 * the diagnostic sink — never for arithmetic.
 *
 * ============================================================================
 * LICENSING — read before copying anything into this repository
 * ============================================================================
 *
 * Those sources are NVIDIA's, dual-licensed MIT / GPL-2.0. Compiling a slice of them for
 * testing is within the MIT grant. Deliberately, NOTHING from those trees is vendored
 * here: `tests/build.rs` hands the compiler their ABSOLUTE PATHS out of a checkout that
 * already exists beside this repository, and refuses loudly rather than substituting a
 * copy when it is absent. That is the arrangement of all four sibling oracles, unchanged.
 */

#define NVOC_KERNEL_CHANNEL_H_PRIVATE_ACCESS_ALLOWED
#define NVOC_KERNEL_FIFO_H_PRIVATE_ACCESS_ALLOWED

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* The tree's own headers. The include path is -I'd by tests/build.rs. */
#include "kernel/gpu/fifo/kernel_fifo.h"
#include "kernel/gpu/fifo/kernel_channel.h"
#include "kernel/gpu/fifo/kernel_channel_group_api.h"
#include "kernel/gpu/fifo/kernel_channel_group.h"
#include "containers/eheap_old.h"

/*
 * ★ The `#include` lines of the two .c files whose STATEMENT spans are compiled below,
 * carried across byte for byte by tests/build.rs rather than chosen here. `RM_PAGE_SIZE`,
 * `NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE` and `NVOS04_FLAGS_CHANNEL_USERD_INDEX_*`
 * reach the compiler ONLY through them. Naming the headers here instead would put the
 * choice back in our hands, which is the transcription this oracle exists to remove.
 */
#include OGKM_KERNEL_CHANNEL_INCLUDES
#include OGKM_KERNEL_FIFO_INCLUDES

/* ------------------------------------------------------------------------- */
/* The stubs                                                                  */
/* ------------------------------------------------------------------------- */

/* The diagnostic sink: the driver's own messages, verbatim, so a refusal is readable. */
#define LOG_CAP 64
#define LOG_LINE 512
static char g_log[LOG_CAP][LOG_LINE];
static int g_log_n;

static void log_add(const char *file, int line, const char *kind, const char *fmt, va_list ap)
{
    char body[LOG_LINE - 128];
    const char *base;
    size_t n;

    if (g_log_n >= LOG_CAP)
        return;
    vsnprintf(body, sizeof(body), fmt, ap);
    n = strlen(body);
    while (n > 0 && (body[n - 1] == '\n' || body[n - 1] == '\r'))
        body[--n] = '\0';
    base = strrchr(file, '/');
    base = base ? base + 1 : file;
    snprintf(g_log[g_log_n], LOG_LINE, "%s %s:%d %s", kind, base, line, body);
    g_log_n++;
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

void *portMemSet(void *p, NvU8 v, NvLength n)
{
    return memset(p, v, (size_t) n);
}

void *portMemCopy(void *dst, NvLength dstSize, const void *src, NvLength srcSize)
{
    if (srcSize > dstSize)
        return NULL;
    return memcpy(dst, src, (size_t) srcSize);
}

/* `eheap_old.c` is the driver's real allocator and allocates its own block list. */
void *portMemAllocatorAlloc(PORT_MEM_ALLOCATOR *pAlloc, NvLength length)
{
    (void) pAlloc;
    return malloc((size_t) length);
}

void portMemAllocatorFree(PORT_MEM_ALLOCATOR *pAlloc, void *pMem)
{
    (void) pAlloc;
    free(pMem);
}

PORT_MEM_ALLOCATOR *portMemAllocatorGetGlobalNonPaged(void)
{
    static PORT_MEM_ALLOCATOR alloc;
    return &alloc;
}

void *portMemAllocNonPaged(NvLength length)
{
    return malloc((size_t) length);
}

void portMemFree(void *pMem)
{
    free(pMem);
}

void nvCheckFailedNoLog(NvU32 level, const char *pszExpr, const char *pszFileName, NvU32 lineNum)
{
    (void) level;
    log_plain(pszFileName, (int) lineNum, "CHECK", "%s", pszExpr);
}

/* The object graph. One GPU, one KernelFifo, one CHID_MGR — set up by `main`. */
static OBJGPU *g_pGpu;
static KernelFifo *g_pKernelFifo;
static CHID_MGR *g_pChidMgr;

struct KernelFifo *gpuGetKernelFifoShared(OBJGPU *pGpu)
{
    (void) pGpu;
    return g_pKernelFifo;
}

CHID_MGR *kfifoGetChidMgr_IMPL(OBJGPU *pGpu, KernelFifo *pKernelFifo, NvU32 runlistId)
{
    (void) pGpu;
    (void) pKernelFifo;
    (void) runlistId;
    return g_pChidMgr;
}

/*
 * No channel is ever already resident: the reader consults this only on the
 * `IS_VIRTUAL_WITHOUT_SRIOV` arm, which a zeroed `OBJGPU` does not take.
 */
struct KernelChannel *kfifoChidMgrGetKernelChannel_IMPL(OBJGPU *pGpu, KernelFifo *pKernelFifo,
                                                        CHID_MGR *pChidMgr, NvU32 ChID)
{
    (void) pGpu;
    (void) pKernelFifo;
    (void) pChidMgr;
    (void) ChID;
    return NULL;
}

/*
 * ★★ THE RECORDER, and the reason the eheap allocation path is never entered.
 *
 * `kchannelAllocHwID_*` ends by handing `kfifoChidMgrAllocChid` the four values it
 * extracted. Allocating for real would need the client database, the isolation IDs, the
 * GSP channel reservation and a populated heap — none of which say anything about the
 * ENCODING. So this records what the driver's own extraction produced, and the harness
 * then feeds those four values to the driver's own RECOMBINATION span directly.
 */
typedef struct {
    NvBool bCalled;
    NvBool bForceInternalIdx;
    NvU32 internalIdx;
    NvBool bForceUserdPage;
    NvU32 userdPageIdx;
    NvU32 ChID;
    CHANNEL_HW_ID_ALLOC_MODE allocMode;
} ExtractRecord;

static ExtractRecord g_rec;

NV_STATUS kfifoChidMgrAllocChid_IMPL(OBJGPU *pGpu, KernelFifo *pKernelFifo, CHID_MGR *pChidMgr,
                                     NvHandle hClient, CHANNEL_HW_ID_ALLOC_MODE chIdFlag,
                                     NvBool bForceInternalIdx, NvU32 internalIdx,
                                     NvBool bForceUserdPage, NvU32 userdPageIdx, NvU32 ChID,
                                     KernelChannel *pKernelChannel)
{
    (void) pGpu;
    (void) pKernelFifo;
    (void) pChidMgr;
    (void) hClient;
    (void) pKernelChannel;
    g_rec.bCalled = NV_TRUE;
    g_rec.allocMode = chIdFlag;
    g_rec.bForceInternalIdx = bForceInternalIdx;
    g_rec.internalIdx = internalIdx;
    g_rec.bForceUserdPage = bForceUserdPage;
    g_rec.userdPageIdx = userdPageIdx;
    g_rec.ChID = ChID;
    return NV_OK;
}

/* ------------------------------------------------------------------------- */
/* The driver's own code                                                       */
/* ------------------------------------------------------------------------- */

/*
 * Each slice is a contiguous span of the original file's bytes, cut by `tests/build.rs`
 * using the file's OWN text as the delimiters and checked for the anchors it must
 * contain. A slice that landed on the wrong region is a hard BUILD error there, not a
 * quietly weaker oracle here.
 */
#include OGKM_USERD_SIZE_SLICE
#include OGKM_KCHANNEL_ALLOC_HWID_SLICE

/* ------------------------------------------------------------------------- */
/* The three statement spans, each wrapped in the smallest frame that compiles */
/* ------------------------------------------------------------------------- */

/*
 * The WRITER. The frame declares exactly the two names the span reads — `pRpcParams` and
 * `pKernelChannel` — with the driver's own types, and returns the flags word the span
 * built. The span itself is untouched.
 */
static NvU32 ogkm_writer_flags(NvU32 chid)
{
    NV_CHANNEL_ALLOC_PARAMS params;
    NV_CHANNEL_ALLOC_PARAMS *pRpcParams = &params;
    KernelChannel *pKernelChannel = (KernelChannel *) calloc(1, sizeof(KernelChannel));

    memset(&params, 0, sizeof(params));
    pKernelChannel->ChID = chid;
    {
#include OGKM_USERD_WRITER_SLICE
    }
    free(pKernelChannel);
    return pRpcParams->flags;
}

/*
 * The GRANULARITY. `userdBar1Size` and `pChidMgr` are the span's own names; the HAL
 * pointer was set by `main` to the symbol the driver's dispatch table binds for this chip,
 * so `kfifoGetUserdSizeAlign_HAL` dispatches exactly as the driver would.
 */
static NvU32 g_userdBar1Size;

static void ogkm_set_granularity(KernelFifo *pKernelFifo, CHID_MGR *pChidMgr)
{
    NvU32 userdBar1Size = 0;
    {
#include OGKM_USERD_GRANULARITY_SLICE
    }
    g_userdBar1Size = userdBar1Size;
}

/*
 * The RECOMBINATION. `ChID64`, `chFlag` and `offsetAlign` are declared exactly as
 * `kfifoChidMgrAllocChid_IMPL` declares them, so the span's `|=` and its
 * `NV_ASSERT_OR_RETURN` behave as they do in the driver. `*pNamed` is read off the
 * driver's OWN `NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE` rather than off
 * `bForceUserdPage`: the question "do these flags name a channel?" is answered by the
 * flag RM sets, not by our reading of which branch ran.
 */
static NV_STATUS ogkm_recombine(CHID_MGR *pChidMgr, NvBool bForceUserdPage, NvU32 userdPageIdx,
                                NvBool bForceInternalIdx, NvU32 internalIdx, NvU64 *pChID64,
                                NvBool *pNamed)
{
    NvU32 chFlag = 0;
    NvU64 ChID64 = 0;
    NvU32 offsetAlign = 1;

#include OGKM_CHID_RECOMBINE_SLICE

    (void) offsetAlign;
    *pChID64 = ChID64;
    *pNamed = (chFlag & NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE) ? NV_TRUE : NV_FALSE;
    return NV_OK;
}

/* ------------------------------------------------------------------------- */
/* The harness                                                                 */
/* ------------------------------------------------------------------------- */

/*
 * ⊘ The harness never computes an expected chid. It reports what RM's code produced; the
 * Rust side is what compares. Putting an expectation here would put the transcription back
 * in, one file to the left.
 */
static void emit_flags(const char *name, long writer_chid, NvU32 flags)
{
    KernelChannel *pKernelChannel = (KernelChannel *) calloc(1, sizeof(KernelChannel));
    NV_STATUS status;
    NvU64 chid64 = 0;
    NvBool named = NV_FALSE;
    int i;

    memset(&g_rec, 0, sizeof(g_rec));
    g_log_n = 0;

    status = OGKM_KCHANNEL_ALLOC_HWID_FN(g_pGpu, pKernelChannel, /*hClient=*/0x0badc0deu, flags,
                                         /*verifFlags2=*/0, /*ChID=*/0);

    printf("case %s writer_chid=", name);
    if (writer_chid < 0)
        printf("-");
    else
        printf("%ld", writer_chid);
    printf(" flags=0x%08x reader_status=0x%x", (unsigned) flags, (unsigned) status);

    if (status != NV_OK || !g_rec.bCalled) {
        printf(" extracted=- reader_chid=-\n");
    } else {
        NV_STATUS rstatus = ogkm_recombine(g_pChidMgr, g_rec.bForceUserdPage, g_rec.userdPageIdx,
                                           g_rec.bForceInternalIdx, g_rec.internalIdx, &chid64,
                                           &named);
        printf(" extracted=page:%u/%u,internal:%u/%u recomb_status=0x%x",
               (unsigned) g_rec.bForceUserdPage, (unsigned) g_rec.userdPageIdx,
               (unsigned) g_rec.bForceInternalIdx, (unsigned) g_rec.internalIdx,
               (unsigned) rstatus);
        if (rstatus == NV_OK && named)
            printf(" reader_chid=%llu\n", (unsigned long long) chid64);
        else
            printf(" reader_chid=-\n");
    }

    for (i = 0; i < g_log_n; i++)
        printf("  log %s\n", g_log[i]);

    free(pKernelChannel);
}

/* One round trip: chid -> (RM's writer) flags -> (RM's reader + recombination) chid'. */
static void emit_roundtrip(const char *name, NvU32 chid)
{
    emit_flags(name, (long) chid, ogkm_writer_flags(chid));
}

int main(void)
{
    NvU32 i;
    char name[64];
    NvU32 base;
    OBJEHEAP *pHeap;

    g_pGpu = (OBJGPU *) calloc(1, 1u << 20);
    g_pKernelFifo = (KernelFifo *) calloc(1, sizeof(KernelFifo));
    g_pChidMgr = (CHID_MGR *) calloc(1, sizeof(CHID_MGR));
    pHeap = (OBJEHEAP *) calloc(1, sizeof(OBJEHEAP));
    if (!g_pGpu || !g_pKernelFifo || !g_pChidMgr || !pHeap) {
        printf("ALLOC-FAILED\n");
        return 1;
    }

    /*
     * ★ The HAL pointer, set to the symbol the DRIVER'S own dispatch table binds for this
     * chip — `tests/build.rs` reads `g_kernel_fifo_nvoc.c` for it and passes it as
     * `OGKM_USERD_SIZE_FN`. The granularity span then calls it through the driver's own
     * `_HAL` dispatch macro, unchanged.
     */
    g_pKernelFifo->__kfifoGetUserdSizeAlign__ = &OGKM_USERD_SIZE_FN;

    /*
     * ★★ The USERD-isolation predicate, bound only when the driver's OWN header says it is
     * halified — `580.159.04` calls a plain `static inline` here and `610.43.02` calls a
     * halified one, and an unbound halified pointer is `NULL` in a `calloc`'d `KernelFifo`.
     * The 610 build was clean and crashed on its first case before this existed.
     *
     * ⊘ Its VALUE does not enter the granularity: `eheapSetOwnerIsolation` stores
     * `ownerGranularity` on every path it does not reject, so binding this is about not
     * calling `NULL`, not about steering the number.
     */
#if defined(OGKM_USERD_ISOLATION_MEMBER)
    g_pKernelFifo->OGKM_USERD_ISOLATION_MEMBER = &OGKM_USERD_ISOLATION_FN;
#endif

    /*
     * The driver's real eheap, constructed by the driver's own `constructObjEHeap`, so
     * `ownerGranularity` is written by NVIDIA's `eheapSetOwnerIsolation` and read back by
     * NVIDIA's recombination. The size is the harness's (nothing here allocates), the
     * granularity is not.
     */
    constructObjEHeap(pHeap, 0, 1, sizeof(PFIFO_ISOLATIONID), 0);
    g_pChidMgr->pGlobalChIDHeap = pHeap;
    ogkm_set_granularity(g_pKernelFifo, g_pChidMgr);

    printf("oracle userd_chid\n");
    printf("chip %s\n", OGKM_USERD_CHID_CHIP);
    printf("reader %s\n", OGKM_KCHANNEL_ALLOC_HWID_FN_NAME);
    printf("userd_size %u\n", (unsigned) g_userdBar1Size);
    /*
     * ★★★ THE NUMBER THE WHOLE INCREMENT TURNS ON, printed rather than assumed. It is the
     * driver's `RM_PAGE_SIZE / userdBar1Size`, stored by the driver's own eheap setter.
     * The writer's divisor — `NVBIT(DRF_SIZE(_USERD_INDEX_VALUE))` — is a DIFFERENT number
     * from a different place; the round trips below are what say whether they agree.
     */
    printf("granularity %u\n", (unsigned) g_pChidMgr->pGlobalChIDHeap->ownerGranularity);

    /*
     * ★★ A SWEEP, not samples.
     *
     * One case per bit position of the chid: bits 0..2 land in `_USERD_INDEX_VALUE`, bits
     * 3.. in `_USERD_INDEX_PAGE_VALUE` — IF the two multipliers agree. Bits at and past
     * 12 are deliberately PAST what the flag pair can carry (`_PAGE_VALUE` is nine bits
     * wide and the writer divides by 8): the writer's own `FLD_SET_DRF_NUM` drops them, so
     * these are the cases where RM'S OWN ROUND TRIP DOES NOT CLOSE. That is a fact about
     * the driver, reported, not an error here.
     */
    for (i = 0; i < 20; i++) {
        snprintf(name, sizeof(name), "chid_bit%u", i);
        emit_roundtrip(name, 1u << i);
    }
    /* Both subfields saturated, and one past. */
    emit_roundtrip("zero", 0);
    emit_roundtrip("value_max", 7);
    emit_roundtrip("value_max_plus1", 8);
    emit_roundtrip("both_max", 4095);
    emit_roundtrip("both_max_plus1", 4096);
    emit_roundtrip("all_ones", 0xFFFFFFFFu);
    /* A spread of ordinary values, and the chids a real GA106 census produced. */
    for (i = 1; i <= 12; i++) {
        snprintf(name, sizeof(name), "small%u", i);
        emit_roundtrip(name, i);
    }
    emit_roundtrip("census_gr", 4);
    emit_roundtrip("census_ce4", 9);
    emit_roundtrip("mid", 1234);
    emit_roundtrip("mid2", 2047);
    emit_roundtrip("mid3", 3000);

    /*
     * ★ The two MALFORMED shapes, built with the driver's own `FLD_SET_DRF` from a
     * well-formed writer output so that only the named field differs.
     *
     *   - `_PAGE_FIXED=_FALSE`: the reader takes neither the page branch nor (unless
     *     `_FIXED` is set) the internal one, so nothing names a channel.
     *   - `_PAGE_FIXED=_TRUE` with `_FIXED=_TRUE`: RM's own `NV_ASSERT_OR_RETURN` answers
     *     `NV_ERR_INVALID_STATE`.
     *
     * These are the two arms the shipped decode must answer `None` to, and they are the
     * reason its signature is an `Option`.
     */
    base = ogkm_writer_flags(300);
    emit_flags("malformed_page_fixed_false", -1,
               FLD_SET_DRF(OS04, _FLAGS, _CHANNEL_USERD_INDEX_PAGE_FIXED, _FALSE, base));
    emit_flags("malformed_both_fixed", -1,
               FLD_SET_DRF(OS04, _FLAGS, _CHANNEL_USERD_INDEX_FIXED, _TRUE, base));
    emit_flags("malformed_only_internal_fixed", -1,
               FLD_SET_DRF(OS04, _FLAGS, _CHANNEL_USERD_INDEX_FIXED, _TRUE,
                           FLD_SET_DRF(OS04, _FLAGS, _CHANNEL_USERD_INDEX_PAGE_FIXED, _FALSE,
                                       base)));
    emit_flags("zeroed_flags", -1, 0u);

    /*
     * ★ NEIGHBOURING FIELDS SET. A well-formed word (the writer's output for chid 300) with
     * unrelated `NVOS04_FLAGS_*` fields turned on, so a decoder whose mask is one bit too
     * wide in either direction has somewhere to disagree with `DRF_VAL`. The field names
     * are the driver's; no bit position is written here.
     *
     * ⊘ `writer_chid` is `-` for these two even though 300 went into the base word: the
     * flags are no longer what the writer emitted, and a consumer that compared its own
     * encoder against them would be comparing against a word RM never wrote. What they
     * assert is entirely on the READER side.
     */
    emit_flags("neighbours_set", -1,
               FLD_SET_DRF(OS04, _FLAGS, _CHANNEL_TYPE, _PHYSICAL,
                           FLD_SET_DRF(OS04, _FLAGS, _VPR, _TRUE,
                                       FLD_SET_DRF(OS04, _FLAGS, _PRIVILEGED_CHANNEL, _TRUE,
                                                   base))));
    emit_flags("all_other_bits_set", -1, base | ~(NvU32) (
                   DRF_SHIFTMASK(NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE) |
                   DRF_SHIFTMASK(NVOS04_FLAGS_CHANNEL_USERD_INDEX_FIXED) |
                   DRF_SHIFTMASK(NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_VALUE) |
                   DRF_SHIFTMASK(NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_FIXED)));

    printf("end\n");
    return 0;
}
