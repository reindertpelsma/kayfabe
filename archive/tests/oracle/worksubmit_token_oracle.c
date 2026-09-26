/*
 * worksubmit_token_oracle — a TEST-ONLY differential oracle built out of NVIDIA's OWN
 * work-submit-token encoder.
 *
 * ============================================================================
 * WHY THIS EXISTS — increment E3, and the reason it is the risky one
 * ============================================================================
 *
 * `kayfabe_chips::Ga10xArch::decode_doorbell` turns a 32-bit work-submit token into a
 * `(runlist, chid)` pair. `execution_plane_increments.md` §2.1 argues it is the riskiest
 * increment in the execution plane, for a reason that has nothing to do with difficulty:
 *
 *   > It is the only increment whose wrong answer is SILENT. A wrong decode does not
 *   > fail; it routes a guest's ring to ANOTHER CHANNEL. On the Mode-2 path we are the
 *   > GSP, so there is no second party to notice.
 *
 * and both of the project's standing oracles are structurally blind to it:
 *
 *   - `MockArch::token_for` is the INVERSE OF THE MOCK'S OWN DECODE. A round-trip through
 *     a function and its inverse measures nothing about the world, and this exact shape
 *     let a planted mutation survive on 2026-08-01.
 *   - `c_rust_trace_differential.md` records that the COMPLETION PLANE HAS NO C ORACLE —
 *     the C artifact forges completions — and that forwarding-mode traces are
 *     non-hermetic. A green differential across a doorbell says nothing about where the
 *     ring went.
 *
 * So the encoding is settled by two instruments outside our own beliefs. The first is a
 * real GA106 (`tests/tests/doorbell_token.rs`, over
 * `docs/reference/bench_evidence/doorbell-census-ba74151.out`). This is the second, and
 * it is the one that pins what hardware could not: the field WIDTHS. The census's chids
 * were 4..9 and its upper-field values 0, 1, 2 and 8 — small numbers that a decoder with
 * the wrong mask agrees with everywhere. RM's encoder does not have to be sampled; it can
 * be swept.
 *
 * ============================================================================
 * WHAT IS COMPILED, AND WHAT IS OURS
 * ============================================================================
 *
 * The only NVIDIA code here is:
 *
 *   1. `kfifoGenerateWorkSubmitTokenHal_<CHIP>` — sliced BYTE FOR BYTE out of the driver
 *      file the driver's OWN dispatch table binds for the chip (`tests/build.rs` parses
 *      `g_kernel_fifo_nvoc.c` for it; on GA106 that is `_GA100`). This is the function
 *      that contains the two lines the whole increment turns on:
 *
 *        val = FLD_SET_DRF_NUM(_CTRL, _VF_DOORBELL, _RUNLIST_ID, runlistId, val);
 *        val = FLD_SET_DRF_NUM(_CTRL, _VF_DOORBELL, _VECTOR,     chId,      val);
 *
 *   2. `kchannelIsRunlistSet` / `kchannelSetRunlistSet` — sliced out of
 *      `kernel_channel.c`, so the "is this channel on a runlist yet" gate the encoder
 *      checks is the driver's own predicate over the driver's own state bits, set through
 *      the driver's own setter. Faking that gate would have been faking the one branch
 *      that decides whether a token exists at all.
 *
 *   3. Everything the two above read: `nvmisc.h`'s `FLD_SET_DRF_NUM`, and
 *      `published/ampere/ga100/dev_ctrl.h`'s `NV_CTRL_VF_DOORBELL_VECTOR` /
 *      `_RUNLIST_ID`, pulled in by the slice's own `#include`s from the tree's own
 *      headers. NO BIT POSITION IS TRANSCRIBED ANYWHERE IN THIS FILE.
 *
 * Everything else is a stub, and there are eight. They stand in for the object model and
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
 * copy when it is absent. That is `vbios_oracle.c`'s and `gmmu_fmt_oracle.c`'s
 * arrangement, unchanged.
 */

#define NVOC_KERNEL_CHANNEL_H_PRIVATE_ACCESS_ALLOWED
#define NVOC_KERNEL_FIFO_H_PRIVATE_ACCESS_ALLOWED

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* The vendored tree's own headers. The include path is -I'd by tests/build.rs. */
#include "kernel/gpu/fifo/kernel_fifo.h"
#include "kernel/gpu/fifo/kernel_channel.h"
#include "kernel/gpu/fifo/kernel_channel_group_api.h"
#include "kernel/gpu/fifo/kernel_channel_group.h"

/* ------------------------------------------------------------------------- */
/* The eight stubs                                                            */
/* ------------------------------------------------------------------------- */

/*
 * One subdevice. `kchannelIsRunlistSet` and `kchannelSetRunlistSet` index
 * `pKernelChannel->swState[]` with this, so a constant 0 makes the getter read exactly
 * the slot the setter wrote — which is the property those two slices are here for.
 */
NvU32 gpumgrGetSubDeviceInstanceFromGpu(OBJGPU *pGpu)
{
    (void) pGpu;
    return 0;
}

/*
 * ★ The physical function. `bUsedForHost` is false in the path this oracle drives, so the
 * encoder's `IS_GFID_VF(gfId)` branch — which would rewrite `chId` into a VF-local one —
 * is taken only if this answers a VF gfid. It answers 0, the PF, which is the
 * configuration the bench measured and the only one this port targets
 * (`mode2_vgpu_posture_decision`: default bare-metal). Reported by the harness so the
 * scope is in the output rather than in a comment.
 */
NV_STATUS vgpuGetCallingContextGfid(OBJGPU *pGpu, NvU32 *pGfid)
{
    (void) pGpu;
    *pGfid = 0;
    return NV_OK;
}

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

/* ------------------------------------------------------------------------- */
/* The driver's own code                                                       */
/* ------------------------------------------------------------------------- */

/*
 * Each slice is a contiguous span of the original file's bytes, cut by `tests/build.rs`
 * using the file's OWN text as the delimiters and checked for the anchors it must
 * contain. A slice that landed on the wrong region is a hard BUILD error there, not a
 * quietly weaker oracle here.
 */
#include OGKM_KCHANNEL_RUNLIST_SLICE
#include OGKM_WORKSUBMIT_TOKEN_SLICE

/* ------------------------------------------------------------------------- */
/* The harness                                                                 */
/* ------------------------------------------------------------------------- */

/*
 * Build the minimum object graph the encoder dereferences, ask it for a token, and print
 * one line per case. Output is `case <name> runlist=<r> chid=<c> status=<s> token=<t>`,
 * or `case <name> ... status=<s> token=-` when the encoder refused.
 *
 * ⊘ The harness never computes an expected token. It reports what RM's code produced; the
 * Rust side is what compares. Putting an expectation here would put the transcription
 * back in, one file to the left.
 */
static void emit(const char *name, NvU32 runlistId, NvU32 chId, NvBool bRunlistSet)
{
    OBJGPU *pGpu = calloc(1, 1u << 20);
    KernelFifo *pKernelFifo = calloc(1, sizeof(KernelFifo));
    KernelChannel *pKernelChannel = calloc(1, sizeof(KernelChannel));
    KernelChannelGroupApi *pGrpApi = calloc(1, sizeof(KernelChannelGroupApi));
    KernelChannelGroup *pGrp = calloc(1, sizeof(KernelChannelGroup));
    NvU32 token = 0xDEADBEEFu;
    NV_STATUS status;

    if (!pGpu || !pKernelFifo || !pKernelChannel || !pGrpApi || !pGrp) {
        printf("case %s ALLOC-FAILED\n", name);
        return;
    }

    /*
     * ★ The encoder asserts on this chain (`pKernelChannelGroupApi != NULL &&
     * ...->pKernelChannelGroup != NULL`) before it does anything else, so a channel
     * without a group is a refusal rather than a token — which is itself one of the
     * cases below.
     */
    pGrpApi->pKernelChannelGroup = pGrp;
    pKernelChannel->pKernelChannelGroupApi = pGrpApi;
    pKernelChannel->ChID = chId;
    /* The driver's own setter/getter pair, not a field poke. */
    kchannelSetRunlistId(pKernelChannel, runlistId);
    kchannelSetRunlistSet(pGpu, pKernelChannel, bRunlistSet);

    g_log_n = 0;
    status = OGKM_WORKSUBMIT_TOKEN_FN(pGpu, pKernelFifo, pKernelChannel, &token, NV_FALSE);

    if (status == NV_OK)
        printf("case %s runlist=%u chid=%u status=0x%x token=0x%08x\n", name, runlistId,
               chId, (unsigned) status, (unsigned) token);
    else
        printf("case %s runlist=%u chid=%u status=0x%x token=-\n", name, runlistId, chId,
               (unsigned) status);

    free(pGrp);
    free(pGrpApi);
    free(pKernelChannel);
    free(pKernelFifo);
    free(pGpu);
}

int main(void)
{
    NvU32 i;
    char name[64];

    printf("oracle worksubmit_token\n");
    printf("hal %s\n", OGKM_WORKSUBMIT_TOKEN_FN_NAME);
    printf("chip %s\n", OGKM_WORKSUBMIT_TOKEN_CHIP);
    /* The scope, in the artifact: PF only. See `vgpuGetCallingContextGfid` above. */
    printf("gfid 0\n");

    /*
     * ★★ A SWEEP, not samples. This is the half hardware could not do: every bit
     * position of each field is exercised on its own, so a decoder whose mask is one bit
     * too wide or whose shift is one off has somewhere to disagree. `1 << 12` and
     * `1 << 7` are deliberately PAST each field's documented end — the encoder's own
     * `FLD_SET_DRF_NUM` will drop those bits, and a decoder that reads them back has
     * invented a field.
     */
    for (i = 0; i < 14; i++) {
        snprintf(name, sizeof(name), "chid_bit%u", i);
        emit(name, 0, 1u << i, NV_TRUE);
    }
    for (i = 0; i < 9; i++) {
        snprintf(name, sizeof(name), "runlist_bit%u", i);
        emit(name, 1u << i, 0, NV_TRUE);
    }
    /* Both fields at once, including both saturated. */
    emit("both_max", 0x7Fu, 0xFFFu, NV_TRUE);
    emit("both_overflow", 0xFFFFFFFFu, 0xFFFFFFFFu, NV_TRUE);
    emit("zero", 0, 0, NV_TRUE);
    /* A spread of ordinary values, including the six the RTX 3060 census produced. */
    emit("census_gr", 0, 4, NV_TRUE);
    emit("census_ce0", 0, 5, NV_TRUE);
    emit("census_ce1", 0, 6, NV_TRUE);
    emit("census_ce2", 1, 7, NV_TRUE);
    emit("census_ce3", 2, 8, NV_TRUE);
    emit("census_ce4", 8, 9, NV_TRUE);
    /*
     * ★ The refusal. A channel that was never bound to a runlist has no token, and the
     * driver says so with `NV_ERR_INVALID_STATE` rather than with a zero — which is the
     * fact the whole R13 rung leans on when it calls a token "evidence".
     */
    emit("unbound", 3, 11, NV_FALSE);

    printf("end\n");
    return 0;
}
