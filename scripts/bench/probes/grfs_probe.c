// grfs_probe.c — ask the HOST RM, as a plain unprivileged client, every GR / FB floorsweeping
// control the guest's RM or libcuda can see, and log each call's request and reply bytes.
//
//   gcc -O1 -o grfs_probe scripts/bench/probes/grfs_probe.c && ./grfs_probe [minor] [deviceInstance]
//
// ★ Why it exists (v3-gpcmask, 2026-09-26): `kf_rm::hostquery::query_gr_geometry` rebuilds the
// GSP's `INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS` reply from these per-index controls, and a
// floor-swept die (non-contiguous GPC mask, logical GPC order != physical) is only checkable
// against the die's OWN answers. Every `CTRL` line below is replayable: the kf-rm test
// `gr_static_floorswept.rs` answers `query_gr_geometry` from them by exact request bytes, so the
// requests issued here are EXACTLY the shapes kayfabe issues (zeroed buffers, inputs set), plus
// an exploratory sweep past the GPC count that records the out-of-range statuses. Run it as the
// VMM would run (ideally without CAP_SYS_ADMIN): a control it cannot ask is a finding.
//
// ⚠ The traces under `traces/real_ga104/` and `traces/real_ad104/` were taken at 25edb757 /
// 0376ddb6, when `GR_GET_PHYS_GPC_MASK` still sat in the kayfabe-shaped section; it moved to the
// exploratory one when realize stopped asking it (it is PRIVILEGED).
//
// Output: human-readable lines, and one `CTRL cmd=0x… status=0x… size=N in=<hex> out=<hex>` line
// per control (`out` is the buffer after the call, whatever the status).
//
// ⊘ Diagnostic only: allocates a root client, a device and a subdevice, issues read-only
// controls, and exits (the fd close frees the client). No GPU work, no mappings.
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

#define NV_IOCTL_MAGIC 'F'
#define NV_ESC_REGISTER_FD 201      /* NV_IOCTL_BASE + 1  (nv-ioctl-numbers.h) */
#define NV_ESC_CHECK_VERSION_STR 210 /* NV_IOCTL_BASE + 10 */
#define NV_ESC_RM_CONTROL 0x2A      /* nv_escape.h */
#define NV_ESC_RM_ALLOC 0x2B

typedef struct { uint32_t cmd, reply; char v[64]; } ver_t;                       /* nv_ioctl_rm_api_version_t */
typedef struct { uint32_t hRoot, hParent, hNew, hClass; uint64_t pParams; uint32_t size, status; } os21_t;
typedef struct { uint32_t hClient, hObject, cmd, flags; uint64_t params; uint32_t size, status; } os54_t;
typedef struct { uint32_t deviceId, hClientShare, hTargetClient, hTargetDevice, flags, pad;
                 uint64_t vaSpaceSize, vaStartInternal, vaLimitInternal; uint32_t vaMode, pad2; } dev_alloc_t;

static int ctl = -1;
static uint32_t hClient, hDevice = 0xcaf00001, hSub = 0xcaf00002;

static int rm_alloc(uint32_t parent, uint32_t hnew, uint32_t cls, void *p, uint32_t sz, uint32_t *out) {
    os21_t a = {hClient, parent, hnew, cls, (uint64_t)(uintptr_t)p, sz, 0};
    if (cls == 0x41) { a.hRoot = 0; a.hParent = 0; }
    if (ioctl(ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC, os21_t), &a) < 0) { perror("RM_ALLOC"); return -1; }
    if (a.status) { fprintf(stderr, "alloc class %#x: status %#x\n", cls, a.status); return -1; }
    if (out) *out = a.hNew;
    return 0;
}

static void hex(const char *tag, const uint8_t *b, uint32_t n) {
    printf(" %s=", tag);
    for (uint32_t i = 0; i < n; i++) printf("%02x", b[i]);
}

/* One control: the request as sent and the buffer as returned, logged unless `quiet`. */
static uint32_t ctrl_x(uint32_t cmd, uint8_t *p, uint32_t sz, int quiet) {
    static uint8_t in[16384];
    memcpy(in, p, sz);
    os54_t a = {hClient, hSub, cmd, 0, (uint64_t)(uintptr_t)p, sz, 0};
    uint32_t st = ioctl(ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_CONTROL, os54_t), &a) < 0 ? 0xffffffffu : a.status;
    if (!quiet) {
        printf("CTRL cmd=0x%08x status=0x%08x size=%u", cmd, st, sz);
        hex("in", in, sz);
        hex("out", p, sz);
        printf("\n");
    }
    return st;
}
static uint32_t ctrl(uint32_t cmd, uint8_t *p, uint32_t sz) { return ctrl_x(cmd, p, sz, 0); }

static uint32_t rd32(const uint8_t *b, int o) { uint32_t v; memcpy(&v, b + o, 4); return v; }
static uint16_t rd16(const uint8_t *b, int o) { uint16_t v; memcpy(&v, b + o, 2); return v; }
static void wr32(uint8_t *b, int o, uint32_t v) { memcpy(b + o, &v, 4); }
static void wr16(uint8_t *b, int o, uint16_t v) { memcpy(b + o, &v, 2); }

/* GRMGR_GET_GR_FS_INFO: numQueries (u16) @0, queries @8, stride 20: type u16 @0, status @4, data @8. */
static void grmgr(const uint16_t *types, const uint32_t *inputs, int n, const char *what) {
    uint8_t b[1928];
    memset(b, 0, sizeof b);
    wr16(b, 0, (uint16_t)n);
    for (int i = 0; i < n; i++) { wr16(b, 8 + 20 * i, types[i]); wr32(b, 8 + 20 * i + 8, inputs[i]); }
    uint32_t st = ctrl(0x20803801, b, sizeof b);
    printf("GRMGR %s st=%#x\n", what, st);
    for (int i = 0; i < n; i++) {
        int o = 8 + 20 * i;
        printf("  q%-2d type=%-2u status=%#-4x d0=%#x d1=%#x\n", i, rd16(b, o), rd32(b, o + 4), rd32(b, o + 8), rd32(b, o + 12));
    }
}

int main(int argc, char **argv) {
    int minor = argc > 1 ? atoi(argv[1]) : 0;
    char path[64];
    snprintf(path, sizeof path, "/dev/nvidia%d", minor);
    ctl = open("/dev/nvidiactl", O_RDWR);
    int gfd = open(path, O_RDWR);
    if (ctl < 0 || gfd < 0) { perror("open"); return 1; }
    ver_t v = {'2', 0, {0}};
    ioctl(ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_CHECK_VERSION_STR, ver_t), &v);
    printf("GRFS_PROBE driver=%s minor=%d\n", v.v, minor);
    int cfd = ctl;
    if (ioctl(gfd, _IOWR(NV_IOCTL_MAGIC, NV_ESC_REGISTER_FD, int), &cfd) < 0) perror("REGISTER_FD");
    if (rm_alloc(0, 0, 0x41, NULL, 0, &hClient)) return 1;
    dev_alloc_t d;
    memset(&d, 0, sizeof d);
    d.deviceId = argc > 2 ? (uint32_t)atoi(argv[2]) : 0;
    if (rm_alloc(hClient, hDevice, 0x80, &d, sizeof d, NULL)) return 1;
    uint32_t sub = 0;
    if (rm_alloc(hDevice, hSub, 0x2080, &sub, 4, NULL)) return 1;

    uint8_t b[16384];
    uint32_t st;

    /* GR_GET_INFO_V2 — the whole table, index i at slot i (kayfabe's info_list_request). */
    memset(b, 0, sizeof b);
    wr32(b, 0, 0x3a);
    for (int i = 0; i < 0x3a; i++) wr32(b, 4 + 8 * i, i);
    st = ctrl(0x20801228, b, 488);
    printf("GR_GET_INFO_V2 st=%#x data=", st);
    for (int i = 0; i < 0x3a; i++) printf("%u%s", rd32(b, 8 + 8 * i), i + 1 < 0x3a ? "," : "\n");
    uint32_t litter_gpcs = rd32(b, 8 + 8 * 0x14);

    memset(b, 0, sizeof b); st = ctrl(0x2080122a, b, 24);
    uint32_t gpcmask = rd32(b, 16);
    printf("GR_GET_GPC_MASK st=%#x gpcMask=%#x contiguous=%d\n", st, gpcmask, (gpcmask & (gpcmask + 1)) == 0);
    int n = __builtin_popcount(gpcmask);

    /* The physical loop, exactly as kayfabe asks it: TPC then ZCULL mask per set bit. */
    for (uint32_t g = 0; g < 32; g++) {
        if (!(gpcmask & (1u << g))) continue;
        memset(b, 0, sizeof b); wr32(b, 16, g); st = ctrl(0x2080122b, b, 24);
        printf("phys gpc %u: TPC_MASK st=%#x %#x\n", g, st, rd32(b, 20));
        memset(b, 0, sizeof b); wr32(b, 0, g); st = ctrl(0x20801237, b, 8);
        printf("phys gpc %u: ZCULL_MASK st=%#x %#x\n", g, st, rd32(b, 4));
    }
    for (int l = 0; l < n; l++) {
        memset(b, 0, sizeof b); wr32(b, 0, l); st = ctrl(0x20801234, b, 8);
        printf("logical gpc %d: NUM_TPCS st=%#x %u\n", l, st, rd32(b, 4));
    }
    /* The map batch, exactly libcuda's cuInit shape: CHIPLET_GPC_MAP for 0..n-1 and nothing else. */
    {
        uint16_t t[96] = {0}; uint32_t in[96] = {0};
        for (int l = 0; l < n; l++) { t[l] = 2; in[l] = l; }
        grmgr(t, in, n, "map batch");
    }
    for (int l = 0; l < n; l++) {
        memset(b, 0, sizeof b); wr32(b, 0, l); st = ctrl(0x20800168, b, 56);
        printf("logical gpc %d: PES_INFO st=%#x numPes=%u activePes=%#x maxTpc=%u t2p=", l, st, rd32(b, 4), rd32(b, 8), rd32(b, 12));
        for (int i = 0; i < 10; i++) printf("%u%s", rd32(b, 16 + 4 * i), i < 9 ? "," : "\n");
    }
    memset(b, 0, sizeof b); st = ctrl(0x20801239, b, 24);
    printf("GR_GET_GFX_GPC_AND_TPC_INFO st=%#x physGfxGpcMask=%#x numGfxTpc=%u\n", st, rd32(b, 16), rd32(b, 20));
    memset(b, 0, sizeof b); st = ctrl(0x2080121b, b, 9240);
    uint16_t nsm = rd16(b, 512 * 18), ntpc = rd16(b, 512 * 18 + 2);
    printf("GR_GET_GLOBAL_SM_ORDER st=%#x numSm=%u numTpc=%u\n", st, nsm, ntpc);
    for (int i = 0; i < nsm && i < 512; i++) {
        printf("  sm%-3d", i);
        for (int k = 0; k < 9; k++) printf(" %u", rd16(b, i * 18 + 2 * k));
        printf("\n");
    }
    memset(b, 0, sizeof b); st = ctrl(0x20801227, b, 48);
    printf("GR_GET_CAPS_V2 st=%#x populated=%u\n", st, b[40]);
    /* The optional batch, exactly kayfabe's: the two syspipe words, then PPC and ROP per logical GPC. */
    {
        uint16_t t[96] = {0}; uint32_t in[96] = {0}; int q = 0;
        t[q] = 6; in[q++] = 0;
        t[q] = 11; in[q++] = 0;
        for (int l = 0; l < n; l++) { t[q] = 4; in[q++] = l; }
        for (int l = 0; l < n; l++) { t[q] = 10; in[q++] = l; }
        grmgr(t, in, q, "extra batch");
    }

    /* ── exploratory: every per-index control past the counts, to record the out-of-range answers ── */
    /* GR_GET_PHYS_GPC_MASK is PRIVILEGED (export flags 0x14) and NOT asked by kayfabe: recorded so
     * the answer to THIS client is on file (0x1b without CAP_SYS_ADMIN, measured on an RTX 4070). */
    memset(b, 0, sizeof b); st = ctrl(0x20801232, b, 8);
    printf("GR_GET_PHYS_GPC_MASK st=%#x physGpcMask=%#x (PRIVILEGED)\n", st, rd32(b, 4));
    printf("litter_num_gpcs=%u\n", litter_gpcs);
    for (uint32_t g = 0; g < 16; g++) {
        uint32_t s1, s2, s3, s4, s5, tpc, ntp, zc, ppc, npes;
        memset(b, 0, sizeof b); wr32(b, 16, g); s1 = ctrl(0x2080122b, b, 24); tpc = rd32(b, 20);
        memset(b, 0, sizeof b); wr32(b, 0, g); s2 = ctrl(0x20801234, b, 8); ntp = rd32(b, 4);
        memset(b, 0, sizeof b); wr32(b, 0, g); s3 = ctrl(0x20801237, b, 8); zc = rd32(b, 4);
        memset(b, 0, sizeof b); wr32(b, 16, g); s4 = ctrl(0x20801233, b, 24); ppc = rd32(b, 20);
        memset(b, 0, sizeof b); wr32(b, 0, g); s5 = ctrl(0x20800168, b, 56); npes = rd32(b, 4);
        printf("index %2u: TPC_MASK st=%#x %#x | NUM_TPCS st=%#x %u | ZCULL_MASK st=%#x %#x | GR_GET_PPC_MASK st=%#x %#x | PES_INFO st=%#x numPes=%u\n",
               g, s1, tpc, s2, ntp, s3, zc, s4, ppc, s5, npes);
    }
    {
        uint16_t t[96] = {0}; uint32_t in[96] = {0}; int q = 0;
        t[q] = 1; in[q++] = 0;
        for (uint16_t ty = 2; ty <= 4; ty++)
            for (uint32_t g = 0; g < 16; g++) { t[q] = ty; in[q++] = g; }
        grmgr(t, in, q, "exploratory batch (GPC_COUNT, then CHIPLET_GPC_MAP/TPC_MASK/PPC_MASK for 0..15)");
        q = 0;
        for (uint32_t g = 0; g < 16; g++) { t[q] = 10; in[q++] = g; }
        grmgr(t, in, q, "exploratory batch (ROP_MASK for 0..15)");
    }
    memset(b, 0, sizeof b); st = ctrl(0x20801213, b, 12);
    printf("GR_GET_ROP_INFO st=%#x ropUnitCount=%u factor=%u count=%u\n", st, rd32(b, 0), rd32(b, 4), rd32(b, 8));
    /* FB_GET_INFO_V2, each index alone (1028-byte struct: count, then {index, data}[128]). */
    for (uint32_t idx = 0; idx <= 0x3b; idx++) {
        memset(b, 0, sizeof b);
        wr32(b, 0, 1); wr32(b, 4, idx);
        st = ctrl_x(0x20801303, b, 1028, 1); /* 60 x 4 KiB of hex would dwarf the rest: values only */
        printf("FB_INFO[%#04x] st=%#x data=%#x (%u)\n", idx, st, rd32(b, 8), rd32(b, 8));
    }
    /* GPU_GET_ENGINES_V2 — libcuda's cuInit asks it once; the engine TYPES the guest is shown. */
    memset(b, 0, sizeof b); st = ctrl(0x20800170, b, 340);
    printf("GPU_GET_ENGINES_V2 st=%#x n=%u:", st, rd32(b, 0));
    for (uint32_t i = 0; i < rd32(b, 0) && i < 84; i++) printf(" %#x", rd32(b, 4 + 4 * i));
    printf("\n");
    printf("GRFS_PROBE_DONE\n");
    return 0;
}
