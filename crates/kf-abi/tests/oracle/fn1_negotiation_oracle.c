/*
 * fn1_negotiation_oracle -- the GUEST half of the first vGPU/GSP RPC, SET_GUEST_SYSTEM_INFO (fn 1),
 * compiled out of NVIDIA's own open-gpu-kernel-modules source (595.84) and driven as a state machine.
 *
 * ============================================================================
 * WHAT IT IS, AND WHY
 * ============================================================================
 *
 * The guest sends its VGX pair in fn 1, expects `rpc_result` OK, and then reads `rpc_result_private`
 * and the pair in the REPLY BODY. A private status != OK with a different pair in the body is the
 * protocol's down-negotiation: the guest retries once at the host's pair (when that is >= the guest's
 * minimum) or gives up with a named log. kayfabe is the HOST side of that exchange, so this program is
 * the GUEST: it feeds a host reply in, and prints what the real guest code decides.
 *
 * Nothing about the decision is transcribed. Two pieces of ogkm are compiled BYTE-FOR-BYTE:
 *
 *   slice A  RmRpcSetGuestSystemInfo        src/nvidia/src/kernel/vgpu/rpc.c
 *            (the whole function: request build, the rpc_result_private branch, the retry rule,
 *             the min-supported check, the stored pair, the rpcSetIpVersion call, the fn-64 tail call)
 *   slice B  the tail of _issueRpcAndWait   src/nvidia/src/kernel/vgpu/rpc.c
 *            (from "Now check if RPC really succeeded": what the guest does with `rpc_result`)
 *
 * The slices are cut out by tests/oracle/build_fn1_oracle.sh, anchored on the file's own text and
 * checked for the markers they must contain, so a tree whose function moved is a hard build error
 * rather than a weaker oracle. The real headers supply: rpc_set_guest_system_info_v (the wire struct),
 * rpc_message_header_v (the envelope, both result fields), RPC_VERSION_FROM_VGX_VERSION and the
 * VGX_* / MIN_SUPPORTED_INTERNAL constants (vgpu_version.h, g_rpc-structures.h), the
 * NV_VGPU_MSG_RESULT_* codes (rpc_headers.h), DRF_NUM/DRF_BASE (nvmisc.h), NV_ROUNDUP.
 *
 * ============================================================================
 * WHAT IS SHIMMED (everything else is the real source)
 * ============================================================================
 *
 *   1. OBJGPU / OBJRPC / OBJGPUMGR / OBJSYS are minimal structs; IS_GSP_CLIENT / IS_VIRTUAL /
 *      IS_VGPU_GSP_PLUGIN_OFFLOAD_ENABLED are constants; the machine has ONE gpu (numGpuHandles = 1), so
 *      the multi-GPU "skip the handshake on the second GPU" branch is NOT exercised.
 *   2. The guest's identity macros are RUNTIME values instead of compile-time constants, so one binary
 *      can play every guest: VGX_MAJOR/MINOR_VERSION_NUMBER, NV_VERSION_STRING,
 *      NV_BUILD_BRANCH_VERSION, NV_DISPLAY_DRIVER_TITLE, NV_BUILD_CHANGELIST_NUM. The pair a given tree
 *      would send is still reported by `consts` (it is vgpu_version.h's, not ours).
 *   3. The transport: _issueRpcAndWait is a wrapper that (a) puts the request on stdout, (b) reads a
 *      host reply from stdin (or asks a built-in host MODEL in grid mode), writes it over the shared
 *      message buffer exactly as a shared-memory plugin would, and (c) runs slice B on the result.
 *      rpcWriteCommonHeader is a 6-line stand-in for rpc_common.c:rpcWriteCommonHeader (it zeroes the
 *      message and writes function/length; the real one also stamps header_version/signature/sequence,
 *      which no code in slice A or B reads).
 *   4. rpcSetIpVersion records its argument instead of re-running the 191-range function table.
 *      (The agreed pair -> table mapping is NOT modelled: see the doc.)
 *   5. NV_PRINTF / NV_RM_RPC_LOG record the message into a buffer; portStringCopy is strncpy-with-NUL.
 *   6. The fn-64 tail call (NV_RM_RPC_SET_GUEST_SYSTEM_INFO_EXT) is recorded and returns a status the
 *      driver script supplies (default NV_OK).
 *
 * ============================================================================
 * LICENSING
 * ============================================================================
 *
 * The sliced source is NVIDIA's (MIT / GPL-2.0, SPDX MIT in its header). Nothing from the ogkm tree is
 * vendored: the build script reads the tree where it already is and refuses to build without it.
 *
 * ============================================================================
 * PROTOCOL
 * ============================================================================
 *
 *   fn1_negotiation_oracle consts
 *       key=value lines: the constants this ogkm tree defines (version, VGX pair, min supported pair,
 *       result codes, the internal->external mapping rows).
 *   fn1_negotiation_oracle grid
 *       one `GRID ...` line per (guest pair, host pair, host model) over a pair universe derived from
 *       the tree's own mapping table.
 *   fn1_negotiation_oracle cell <model> <gmaj> <gmin> <hmaj> <hmin>
 *       one `GRID ...` line for a single cell (pairs outside the universe).
 *   fn1_negotiation_oracle drive <guest-major> <guest-minor> <driver> <branch> <cl> [ext-status]
 *       stdout `REQ <hex>` (the request body, hex), stdin `REPLY <rpc_result> <rpc_result_private> <hex>`
 *       (decimal or 0x hex), repeated, then stdout `DONE ...` (the decision).
 *       A host that closes stdin ends the run with `DONE ... status=<transport-error>`.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <stdint.h>

#include <nvtypes.h>
#include <nvmisc.h>
#include <nvstatus.h>

#define RPC_STRUCTURES
#include "g_rpc-structures.h"
#undef RPC_STRUCTURES
#define RPC_GENERIC_UNION
#include "g_rpc-structures.h"
#undef RPC_GENERIC_UNION
#define RPC_MESSAGE_STRUCTURES
#include "g_rpc-message-header.h"
#undef RPC_MESSAGE_STRUCTURES
#include "vgpu/rpc_headers.h"
#include "vgpu/vgpu_version.h"
#include "nvUnixVersion.h"

/* What this tree itself would send (reported by `consts`; the code below sends the runtime guest's). */
static const char *const OGKM_TREE_VERSION = NV_VERSION_STRING;
static const NvU32 OGKM_VGX_MAJOR = VGX_MAJOR_VERSION_NUMBER;
static const NvU32 OGKM_VGX_MINOR = VGX_MINOR_VERSION_NUMBER;

/* ---- shim 2: the guest identity is a runtime value ------------------------------------------------ */
static NvU32 g_vgx_major, g_vgx_minor, g_cl;
static const char *g_driver, *g_branch, *g_title = "NVIDIA kayfabe-oracle";
#undef VGX_MAJOR_VERSION_NUMBER
#undef VGX_MINOR_VERSION_NUMBER
#undef NV_VERSION_STRING
#define VGX_MAJOR_VERSION_NUMBER g_vgx_major
#define VGX_MINOR_VERSION_NUMBER g_vgx_minor
#define NV_VERSION_STRING g_driver
#define NV_BUILD_BRANCH_VERSION g_branch
#define NV_DISPLAY_DRIVER_TITLE g_title
#define NV_BUILD_CHANGELIST_NUM g_cl

/* ---- shim 1: the objects slice A touches ---------------------------------------------------------- */
typedef struct OBJGPU OBJGPU;
typedef struct OBJRPC {
    NvU32 maxRpcSize;
    void *message_buffer;
    NvBool bQuietPrints;
} OBJRPC;
typedef struct { NvU32 numGpuHandles; } OBJGPUMGR;
typedef struct { OBJGPUMGR gpumgr; } OBJSYS;
struct OBJGPU {
    int (*getProperty)(OBJGPU *, int);
};
static OBJSYS g_sys = { { 1 } };
#define SYS_GET_INSTANCE() (&g_sys)
#define SYS_GET_GPUMGR(s) (&(s)->gpumgr)
#define IS_GSP_CLIENT(g) 1
#define IS_VIRTUAL(g) 1
#define IS_VGPU_GSP_PLUGIN_OFFLOAD_ENABLED(g) 1
#define PDB_PROP_GPU_IN_PM_RESUME_CODEPATH 1
#define gpuGetInstance(g) 0
#define NV_ROUNDUP(a, b) ((NV_CEIL(a, b)) * (b))
#include "ctrl/ctrl0000/ctrl0000system.h" /* NV0000_CTRL_CMD_SYSTEM_GET_VGX_SYSTEM_INFO_BUFFER_SIZE */
/* slice B's profiler line is dead code (bProfileRPC is a driver global that is off), kept compiling: */
static NvBool bProfileRPC = NV_FALSE;
static struct { struct { NvU64 endTimeInNs; } rpcData; } g_meter, *pNewEntry = &g_meter;
static void osGetPerformanceCounter(NvU64 *p) { *p = 0; }

/* ---- shim 5: log, string copy ---------------------------------------------------------------------- */
static char g_log[8192];
static void oracle_log(const char *fmt, ...)
{
    size_t n = strlen(g_log);
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(g_log + n, sizeof g_log - n, fmt, ap);
    va_end(ap);
}
#define NV_PRINTF(level, fmt, ...) oracle_log(fmt, ##__VA_ARGS__)
#define NV_PRINTF_COND(cond, lt, lf, fmt, ...) oracle_log(fmt, ##__VA_ARGS__)
static int g_rpc_log_count;
#define NV_RM_RPC_LOG(pgpu, logstr, level) (g_rpc_log_count++)
static NvLength portStringCopy(char *dest, NvLength destSize, const char *src, NvLength srcSize)
{
    NvLength n = 0;
    while (n + 1 < destSize && n < srcSize && src[n]) { dest[n] = src[n]; n++; }
    if (destSize) dest[n] = 0;
    return n;
}

/* ---- shim 4 and 6: the version table and the fn-64 tail ------------------------------------------- */
static int g_set_ip_version_calls;
static NvU32 g_ip_version;
static void rpcSetIpVersion(OBJGPU *pGpu, OBJRPC *pRpc, NvU32 v) { g_set_ip_version_calls++; g_ip_version = v; }
static int g_ext_calls;
static NV_STATUS g_ext_status = NV_OK;
#define NV_RM_RPC_SET_GUEST_SYSTEM_INFO_EXT(pGpu, status) do { g_ext_calls++; (status) = g_ext_status; } while (0)

/* The process-global slice A reads/writes (a `static` in rpc.c, declared just above the function). */
typedef struct rpc_vgx_version { NvU32 majorNum; NvU32 minorNum; } RPC_VGX_VERSION;
static RPC_VGX_VERSION rpcVgxVersion;
static NvBool bSkipRpcVersionHandshake = NV_FALSE;

#define vgpu_rpc_message_header_v ((rpc_message_header_v *)(pRpc->message_buffer))
#define rpc_message (vgpu_rpc_message_header_v->rpc_message_data)

/* ---- shim 3: the transport ------------------------------------------------------------------------- */
static NV_STATUS rpcWriteCommonHeader(OBJGPU *pGpu, OBJRPC *pRpc, NvU32 func, NvU32 paramLength)
{
    memset(pRpc->message_buffer, 0, pRpc->maxRpcSize);
    vgpu_rpc_message_header_v->function = func;
    vgpu_rpc_message_header_v->length = (NvU32)(sizeof(rpc_message_header_v) + paramLength);
    return NV_OK;
}

/* host callback: given the request body, fill result / private / body (in place, 792 bytes). */
typedef int (*host_fn)(void *ctx, const unsigned char *req, NvU32 *result, NvU32 *priv, unsigned char *body);
static host_fn g_host;
static void *g_host_ctx;
static int g_issued;
static int g_host_eof;
static NvU32 g_first_reply_result, g_first_reply_private;

#define BODY_LEN ((int)sizeof(rpc_set_guest_system_info_v))

static NV_STATUS _issueRpcAndWait(OBJGPU *pGpu, OBJRPC *pRpc)
{
    NV_STATUS status;
    NvU32 expectedFunc = vgpu_rpc_message_header_v->function;
    unsigned char req[sizeof(rpc_set_guest_system_info_v)];
    NvU32 result = 0, priv = 0;
    memcpy(req, &rpc_message->set_guest_system_info_v, BODY_LEN);
    g_issued++;
    if (!g_host(g_host_ctx, req, &result, &priv, (unsigned char *)&rpc_message->set_guest_system_info_v)) {
        g_host_eof = 1;
        return NV_ERR_TIMEOUT; /* rpcRecvPoll timeout: the host never answered */
    }
    vgpu_rpc_message_header_v->rpc_result = result;
    vgpu_rpc_message_header_v->rpc_result_private = priv;
    if (g_issued == 1) { g_first_reply_result = result; g_first_reply_private = priv; }
    (void)expectedFunc;
    status = NV_OK;
#include "fn1_slice_issue_tail.inc" /* ends with _issueRpcAndWait's own closing brace */

#include "fn1_slice_set_guest_system_info.inc"

/* ---- the driver --------------------------------------------------------------------------------------- */
static unsigned char g_msgbuf[128 * 1024] __attribute__((aligned(16)));
static OBJGPU g_gpu_obj;
static OBJRPC g_rpc_obj;

static void hex_put(const unsigned char *p, int n)
{
    for (int i = 0; i < n; i++) printf("%02x", p[i]);
}

static int hex_get(const char *s, unsigned char *out, int n)
{
    for (int i = 0; i < n; i++) {
        unsigned v;
        if (sscanf(s + 2 * i, "%2x", &v) != 1) return 0;
        out[i] = (unsigned char)v;
    }
    return 1;
}

typedef struct {
    NV_STATUS status;
    int issued, ext_calls, ip_set;
    NvU32 agreed_major, agreed_minor, ip_version;
    int retried_log, too_old_log;
    NvU32 first_result, first_private;
} outcome;

static outcome run_guest(NvU32 vmaj, NvU32 vmin, const char *driver, const char *branch, NvU32 cl,
                         host_fn h, void *ctx)
{
    outcome o;
    g_vgx_major = vmaj; g_vgx_minor = vmin; g_driver = driver; g_branch = branch; g_cl = cl;
    g_host = h; g_host_ctx = ctx; g_issued = 0; g_host_eof = 0; g_set_ip_version_calls = 0;
    g_ext_calls = 0; g_log[0] = 0; g_rpc_log_count = 0;
    g_first_reply_result = g_first_reply_private = 0;
    rpcVgxVersion.majorNum = rpcVgxVersion.minorNum = 0;
    bSkipRpcVersionHandshake = NV_FALSE;
    memset(&g_rpc_obj, 0, sizeof g_rpc_obj);
    g_rpc_obj.message_buffer = g_msgbuf;
    g_rpc_obj.maxRpcSize = sizeof g_msgbuf;
    o.status = RmRpcSetGuestSystemInfo(&g_gpu_obj, &g_rpc_obj);
    o.issued = g_issued;
    o.ext_calls = g_ext_calls;
    o.ip_set = g_set_ip_version_calls;
    o.ip_version = g_ip_version;
    o.agreed_major = rpcVgxVersion.majorNum;
    o.agreed_minor = rpcVgxVersion.minorNum;
    o.retried_log = strstr(g_log, "Retrying with the VGX version") != NULL;
    o.too_old_log = strstr(g_log, "is too old") != NULL;
    o.first_result = g_first_reply_result;
    o.first_private = g_first_reply_private;
    return o;
}

static void print_done(const char *prefix, const outcome *o)
{
    printf("%s status=%#x issued=%d agreed=%02x.%02x ip_version=%#x set_ip_version=%d ext_calls=%d "
           "retry_log=%d too_old_log=%d\n",
           prefix, o->status, o->issued, o->agreed_major, o->agreed_minor,
           o->ip_set ? o->ip_version : 0, o->ip_set, o->ext_calls, o->retried_log, o->too_old_log);
}

/* ---- drive mode: the host is on stdin --------------------------------------------------------------- */
static int host_stdin(void *ctx, const unsigned char *req, NvU32 *result, NvU32 *priv, unsigned char *body)
{
    char line[2 * BODY_LEN + 128];
    char hex[2 * BODY_LEN + 8];
    unsigned long r, p;
    (void)ctx;
    printf("REQ ");
    hex_put(req, BODY_LEN);
    printf("\n");
    fflush(stdout);
    if (!fgets(line, sizeof line, stdin)) return 0;
    if (sscanf(line, "REPLY %lu %lu %s", &r, &p, hex) != 3 &&
        sscanf(line, "REPLY %lx %lx %s", &r, &p, hex) != 3)
        return 0;
    if (strlen(hex) != 2 * (size_t)BODY_LEN || !hex_get(hex, body, BODY_LEN)) return 0;
    *result = (NvU32)r;
    *priv = (NvU32)p;
    return 1;
}

/* ---- grid mode: built-in host models -------------------------------------------------------------- */
enum model { M_ACCEPT_GUEST, M_STEER_TO_HOST, M_STEER_ALWAYS_PRIVATE, M_HARD_FAIL_MISMATCH, M_COUNT };
static const char *const model_name[M_COUNT] = {
    "accept-guest", "steer-to-host", "steer-always-private", "hard-fail-mismatch"
};
typedef struct { enum model m; NvU32 hmaj, hmin; } gridhost;

static int host_model(void *ctx, const unsigned char *req, NvU32 *result, NvU32 *priv, unsigned char *body)
{
    gridhost *g = ctx;
    NvU32 qmaj, qmin;
    memcpy(&qmaj, req, 4);
    memcpy(&qmin, req + 4, 4);
    memcpy(body, req, BODY_LEN); /* an in-place plugin leaves the rest of the shared buffer alone */
    *result = NV_OK;
    *priv = NV_OK;
    int same = (qmaj == g->hmaj && qmin == g->hmin);
    switch (g->m) {
    case M_ACCEPT_GUEST:
        break;
    case M_STEER_TO_HOST:
    case M_STEER_ALWAYS_PRIVATE:
        if (!same || g->m == M_STEER_ALWAYS_PRIVATE) {
            memcpy(body, &g->hmaj, 4);
            memcpy(body + 4, &g->hmin, 4);
            *priv = (!same || g->m == M_STEER_ALWAYS_PRIVATE) ? NV_VGPU_MSG_RESULT_GUEST_HOST_DRIVER_MISMATCH : NV_OK;
        }
        break;
    case M_HARD_FAIL_MISMATCH:
        if (!same) *result = NV_ERR_NOT_SUPPORTED;
        break;
    default:
        break;
    }
    return 1;
}

typedef struct { NvU32 maj, min; } pair;
static pair g_universe[256];
static int g_universe_n;

static void universe_add(NvU32 maj, NvU32 min)
{
    for (int i = 0; i < g_universe_n; i++)
        if (g_universe[i].maj == maj && g_universe[i].min == min) return;
    g_universe[g_universe_n].maj = maj;
    g_universe[g_universe_n].min = min;
    g_universe_n++;
}

static int pair_cmp(const void *a, const void *b)
{
    const pair *x = a, *y = b;
    NvU32 vx = RPC_VERSION_FROM_VGX_VERSION(x->maj, x->min), vy = RPC_VERSION_FROM_VGX_VERSION(y->maj, y->min);
    return vx < vy ? -1 : vx > vy;
}

typedef struct { NvU32 lo[2], hi[2], ext[2]; } map_row;
static const map_row g_map[] = { NV_VGPU_GRIDSW_INTERNAL_TO_EXTERNAL_VERSION_MAPPING };

static void build_universe(void)
{
    /* Every pair is a constant the tree itself states: the ends of each internal->external bucket, the
     * tree's own VGX pair, the minimum supported pair, and the minimum's two neighbours just below it. */
    for (size_t i = 0; i < sizeof g_map / sizeof g_map[0]; i++) {
        universe_add(g_map[i].lo[0], g_map[i].lo[1]);
        universe_add(g_map[i].hi[0], g_map[i].hi[1]);
    }
    universe_add(OGKM_VGX_MAJOR, OGKM_VGX_MINOR);
    universe_add(NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MAJOR, NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MINOR);
    universe_add(NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MAJOR, NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MINOR - 1);
    universe_add(NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MAJOR - 1, NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MINOR);
    qsort(g_universe, g_universe_n, sizeof g_universe[0], pair_cmp);
}

static void cmd_consts(void)
{
    printf("ogkm_version=%s\n", OGKM_TREE_VERSION);
    printf("vgx_major=%#x\nvgx_minor=%#x\n", OGKM_VGX_MAJOR, OGKM_VGX_MINOR);
    printf("min_supported_major=%#x\nmin_supported_minor=%#x\n", NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MAJOR,
           NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MINOR);
    printf("result_guest_host_driver_mismatch=%#x\n", (unsigned)NV_VGPU_MSG_RESULT_GUEST_HOST_DRIVER_MISMATCH);
    printf("result_not_supported=%#x\n", (unsigned)NV_ERR_NOT_SUPPORTED);
    printf("body_len=%d\n", BODY_LEN);
    for (size_t i = 0; i < sizeof g_map / sizeof g_map[0]; i++)
        printf("map=%02x.%02x-%02x.%02x\n", g_map[i].lo[0], g_map[i].lo[1], g_map[i].hi[0], g_map[i].hi[1]);
}

static void cmd_grid(void)
{
    build_universe();
    for (int m = 0; m < M_COUNT; m++)
        for (int gi = 0; gi < g_universe_n; gi++)
            for (int hi = 0; hi < g_universe_n; hi++) {
                gridhost gh = { (enum model)m, g_universe[hi].maj, g_universe[hi].min };
                outcome o = run_guest(g_universe[gi].maj, g_universe[gi].min, "oracle", "oracle", 1, host_model, &gh);
                char prefix[160];
                snprintf(prefix, sizeof prefix, "GRID model=%s guest=%02x.%02x host=%02x.%02x", model_name[m],
                         g_universe[gi].maj, g_universe[gi].min, g_universe[hi].maj, g_universe[hi].min);
                print_done(prefix, &o);
            }
}

/* cell <model> <guest-major> <guest-minor> <host-major> <host-minor>: one GRID row, for pairs outside the
 * universe (a served driver row's own pair, a Windows guest's pair). */
static void cmd_cell(int argc, char **argv)
{
    if (argc < 7) { fprintf(stderr, "cell <model> <gmaj> <gmin> <hmaj> <hmin>\n"); exit(2); }
    int m = -1;
    for (int i = 0; i < M_COUNT; i++)
        if (!strcmp(argv[2], model_name[i])) m = i;
    if (m < 0) { fprintf(stderr, "unknown model %s\n", argv[2]); exit(2); }
    NvU32 gm = (NvU32)strtoul(argv[3], NULL, 0), gn = (NvU32)strtoul(argv[4], NULL, 0);
    gridhost gh = { (enum model)m, (NvU32)strtoul(argv[5], NULL, 0), (NvU32)strtoul(argv[6], NULL, 0) };
    outcome o = run_guest(gm, gn, "oracle", "oracle", 1, host_model, &gh);
    char prefix[160];
    snprintf(prefix, sizeof prefix, "GRID model=%s guest=%02x.%02x host=%02x.%02x", model_name[m], gm, gn, gh.hmaj, gh.hmin);
    print_done(prefix, &o);
}

static void cmd_drive(int argc, char **argv)
{
    if (argc < 7) { fprintf(stderr, "drive <major> <minor> <driver> <branch> <cl> [ext-status]\n"); exit(2); }
    if (argc > 7) g_ext_status = (NV_STATUS)strtoul(argv[7], NULL, 0);
    outcome o = run_guest((NvU32)strtoul(argv[2], NULL, 0), (NvU32)strtoul(argv[3], NULL, 0), argv[4], argv[5],
                          (NvU32)strtoul(argv[6], NULL, 0), host_stdin, NULL);
    print_done("DONE", &o);
}

int main(int argc, char **argv)
{
    if (argc >= 2 && !strcmp(argv[1], "consts")) { cmd_consts(); return 0; }
    if (argc >= 2 && !strcmp(argv[1], "grid")) { cmd_grid(); return 0; }
    if (argc >= 2 && !strcmp(argv[1], "cell")) { cmd_cell(argc, argv); return 0; }
    if (argc >= 2 && !strcmp(argv[1], "drive")) { cmd_drive(argc, argv); return 0; }
    fprintf(stderr, "usage: %s consts | grid | drive ...\n", argv[0]);
    return 2;
}
