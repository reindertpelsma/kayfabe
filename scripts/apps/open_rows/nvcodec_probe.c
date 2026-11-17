// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
/*
 * nvcodec_probe: ask the DRIVER what NVENC and NVDEC can do on device 0, and print it as TSV.
 *
 * STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md; GPU-free tests parse the output format,
 * scripts/ci/test_open_rows.py; the program itself runs only on a machine with an NVIDIA driver).
 *
 * Nothing here is a table of what a die can do: every value is what NvEncGetEncodeCaps, cuvidGetDecoderCaps and
 * NvEncOpenEncodeSessionEx answer on the machine the probe runs on (the host, or a kayfabe guest). The expected outcome
 * of each ffmpeg case is computed from this output (scripts/apps/open_rows/nvcodec_lib.py), never from a committed
 * per-die table.
 *
 * Headers: nv-codec-headers (MIT, FFmpeg project), fetched at a pinned upstream commit by provision.sh and found with
 * -I. The loader in them dlopens libcuda, libnvidia-encode and libnvcuvid, so this binary has no link-time driver
 * dependency.   Build: gcc -O2 -I<headers>/include nvcodec_probe.c -o nvcodec_probe -ldl
 *
 * Usage: nvcodec_probe [--max-sessions N]      (default 16; the session discovery opens at most N at once)
 * Output lines: kind TAB key TAB value.   kinds: meta, enc_codec, enc_cap, enc_profile, enc_input, dec_cap, sessions, error.
 * Exit status: 0 when the CUDA device was reached (single NVENC/NVDEC failures are `error` lines), 2 when not even
 * libcuda could be used. The device name is printed; the UUID, the serial and the PCI address never are.
 */
#include <ffnvcodec/dynlink_loader.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static CudaFunctions *cu;
static NvencFunctions *nv;
static CuvidFunctions *cvd;
static NV_ENCODE_API_FUNCTION_LIST api;

static void kv(const char *kind, const char *key, const char *fmt, ...)
{
    va_list ap;
    printf("%s\t%s\t", kind, key);
    va_start(ap, fmt);
    vprintf(fmt, ap);
    va_end(ap);
    printf("\n");
}

#define STATUS_NAMES(X) \
    X(NV_ENC_SUCCESS) X(NV_ENC_ERR_NO_ENCODE_DEVICE) X(NV_ENC_ERR_UNSUPPORTED_DEVICE) X(NV_ENC_ERR_INVALID_ENCODERDEVICE) \
    X(NV_ENC_ERR_INVALID_DEVICE) X(NV_ENC_ERR_DEVICE_NOT_EXIST) X(NV_ENC_ERR_INVALID_PTR) X(NV_ENC_ERR_INVALID_EVENT) \
    X(NV_ENC_ERR_INVALID_PARAM) X(NV_ENC_ERR_INVALID_CALL) X(NV_ENC_ERR_OUT_OF_MEMORY) \
    X(NV_ENC_ERR_ENCODER_NOT_INITIALIZED) X(NV_ENC_ERR_UNSUPPORTED_PARAM) X(NV_ENC_ERR_LOCK_BUSY) \
    X(NV_ENC_ERR_NOT_ENOUGH_BUFFER) X(NV_ENC_ERR_INVALID_VERSION) X(NV_ENC_ERR_MAP_FAILED) \
    X(NV_ENC_ERR_NEED_MORE_INPUT) X(NV_ENC_ERR_ENCODER_BUSY) X(NV_ENC_ERR_EVENT_NOT_REGISTERD) X(NV_ENC_ERR_GENERIC) \
    X(NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY) X(NV_ENC_ERR_UNIMPLEMENTED) X(NV_ENC_ERR_RESOURCE_REGISTER_FAILED) \
    X(NV_ENC_ERR_RESOURCE_NOT_REGISTERED) X(NV_ENC_ERR_RESOURCE_NOT_MAPPED)

static const char *status_name(NVENCSTATUS s)
{
#define CASE(n) if (s == n) return #n;
    STATUS_NAMES(CASE)
#undef CASE
    return "NV_ENC_STATUS_UNKNOWN";
}

/* the caps whose NAMES the case table uses; every other id is still printed as capNN so a host/guest diff is complete */
#define NAMED_CAPS(X) \
    X(NUM_MAX_BFRAMES) X(WIDTH_MAX) X(HEIGHT_MAX) X(WIDTH_MIN) X(HEIGHT_MIN) X(SUPPORT_YUV444_ENCODE) \
    X(SUPPORT_10BIT_ENCODE) X(SUPPORT_LOOKAHEAD) X(SUPPORT_TEMPORAL_AQ) X(SUPPORT_LOSSLESS_ENCODE) \
    X(SUPPORT_BFRAME_REF_MODE) X(NUM_ENCODER_ENGINES) X(MB_NUM_MAX) X(MB_PER_SEC_MAX) X(PREPROC_SUPPORT) \
    X(SUPPORT_MULTIPLE_REF_FRAMES) X(SUPPORT_LOOKAHEAD_LEVEL) X(SUPPORT_UNIDIRECTIONAL_B) X(SUPPORT_TEMPORAL_FILTER)

static const char *cap_name(int id)
{
#define CASE(n) if (id == (int)NV_ENC_CAPS_##n) return #n;
    NAMED_CAPS(CASE)
#undef CASE
    return NULL;
}

#define PROFILES(X) \
    X(h264, BASELINE, NV_ENC_H264_PROFILE_BASELINE_GUID) X(h264, MAIN, NV_ENC_H264_PROFILE_MAIN_GUID) \
    X(h264, HIGH, NV_ENC_H264_PROFILE_HIGH_GUID) X(h264, HIGH_444, NV_ENC_H264_PROFILE_HIGH_444_GUID) \
    X(h264, STEREO, NV_ENC_H264_PROFILE_STEREO_GUID) X(h264, PROGRESSIVE_HIGH, NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID) \
    X(h264, CONSTRAINED_HIGH, NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID) X(hevc, MAIN, NV_ENC_HEVC_PROFILE_MAIN_GUID) \
    X(hevc, MAIN10, NV_ENC_HEVC_PROFILE_MAIN10_GUID) X(hevc, FREXT, NV_ENC_HEVC_PROFILE_FREXT_GUID) \
    X(av1, MAIN, NV_ENC_AV1_PROFILE_MAIN_GUID)

static int guid_eq(const GUID *a, const GUID *b) { return memcmp(a, b, sizeof(GUID)) == 0; }

static void *open_session(CUcontext ctx, NVENCSTATUS *st)
{
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS p;
    void *enc = NULL;
    memset(&p, 0, sizeof(p));
    p.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
    p.deviceType = NV_ENC_DEVICE_TYPE_CUDA;
    p.device = ctx;
    p.apiVersion = NVENCAPI_VERSION;
    *st = api.nvEncOpenEncodeSessionEx(&p, &enc);
    return *st == NV_ENC_SUCCESS ? enc : NULL;
}

static void enc_codec_block(void *enc, const char *codec, GUID guid)
{
    char key[96];
    uint32_t n = 0, got = 0, i;
    int val, id;
    NV_ENC_CAPS_PARAM p;
    GUID pg[32];
    for (id = 0; id < (int)NV_ENC_CAPS_EXPOSED_COUNT; id++) {
        memset(&p, 0, sizeof(p));
        p.version = NV_ENC_CAPS_PARAM_VER;
        p.capsToQuery = (NV_ENC_CAPS)id;
        val = 0;
        NVENCSTATUS st = api.nvEncGetEncodeCaps(enc, guid, &p, &val);
        snprintf(key, sizeof(key), "%s.cap%d", codec, id);
        if (st == NV_ENC_SUCCESS) kv("enc_cap", key, "%d", val);
        else kv("enc_cap", key, "ERR:%s", status_name(st));
        if (st == NV_ENC_SUCCESS && cap_name(id)) {
            snprintf(key, sizeof(key), "%s.%s", codec, cap_name(id));
            kv("enc_cap", key, "%d", val);
        }
    }
    if (api.nvEncGetEncodeProfileGUIDCount(enc, guid, &n) == NV_ENC_SUCCESS && n > 0 && n <= 32 &&
        api.nvEncGetEncodeProfileGUIDs(enc, guid, pg, n, &got) == NV_ENC_SUCCESS) {
        for (i = 0; i < got; i++) {
            const char *pname = "UNKNOWN";
#define MATCH(c, nm, g) if (!strcmp(codec, #c) && guid_eq(&pg[i], &g)) pname = #nm;
            PROFILES(MATCH)
#undef MATCH
            kv("enc_profile", codec, "%s", pname);
        }
    }
    n = got = 0;
    if (api.nvEncGetInputFormatCount(enc, guid, &n) == NV_ENC_SUCCESS && n > 0 && n <= 64) {
        NV_ENC_BUFFER_FORMAT f[64];
        if (api.nvEncGetInputFormats(enc, guid, f, n, &got) == NV_ENC_SUCCESS) {
            for (i = 0; i < got; i++) kv("enc_input", codec, "0x%x", (unsigned)f[i]);
        }
    }
}

static void dec_caps(void)
{
    static const struct { const char *name; cudaVideoCodec c; } CODECS[] = {
        {"mpeg1", cudaVideoCodec_MPEG1}, {"mpeg2", cudaVideoCodec_MPEG2}, {"mpeg4", cudaVideoCodec_MPEG4},
        {"vc1", cudaVideoCodec_VC1}, {"h264", cudaVideoCodec_H264}, {"jpeg", cudaVideoCodec_JPEG},
        {"hevc", cudaVideoCodec_HEVC}, {"vp8", cudaVideoCodec_VP8}, {"vp9", cudaVideoCodec_VP9}, {"av1", cudaVideoCodec_AV1},
    };
    static const char *CHROMA[] = {"400", "420", "422", "444"};
    unsigned ci, ch, bd;
    char key[96];
    if (!cvd->cuvidGetDecoderCaps) { kv("error", "dec_caps", "cuvidGetDecoderCaps not exported by libnvcuvid"); return; }
    for (ci = 0; ci < sizeof(CODECS) / sizeof(CODECS[0]); ci++)
        for (ch = 0; ch < 4; ch++)
            for (bd = 0; bd <= 4; bd += 2) {
                CUVIDDECODECAPS d;
                memset(&d, 0, sizeof(d));
                d.eCodecType = CODECS[ci].c;
                d.eChromaFormat = (cudaVideoChromaFormat)ch;
                d.nBitDepthMinus8 = bd;
                CUresult r = cvd->cuvidGetDecoderCaps(&d);
                snprintf(key, sizeof(key), "%s.%s.%u", CODECS[ci].name, CHROMA[ch], 8 + bd);
                if (r != CUDA_SUCCESS) { kv("dec_cap", key, "ERR:cuda%d", (int)r); continue; }
                kv("dec_cap", key, "supported=%u max=%ux%u min=%ux%u mbmax=%u nvdecs=%u fmtmask=0x%x", d.bIsSupported,
                   d.nMaxWidth, d.nMaxHeight, d.nMinWidth, d.nMinHeight, d.nMaxMBCount, d.nNumNVDECs, d.nOutputFormatMask);
            }
}

int main(int argc, char **argv)
{
    int max_sessions = 16, i, ndev = 0, ccmaj = 0, ccmin = 0, drv = 0;
    char name[128] = "";
    CUdevice dev;
    CUcontext ctx = NULL;
    NVENCSTATUS st;
    uint32_t maxver = 0, ncodec = 0, got = 0, g;
    GUID guids[32];
    void *enc;
    for (i = 1; i + 1 < argc; i++)
        if (!strcmp(argv[i], "--max-sessions")) max_sessions = atoi(argv[i + 1]);
    if (max_sessions < 1 || max_sessions > 64) max_sessions = 16;
    setvbuf(stdout, NULL, _IOLBF, 0);

    if (cuda_load_functions(&cu, NULL) < 0 || cu->cuInit(0) != CUDA_SUCCESS || cu->cuDeviceGetCount(&ndev) != CUDA_SUCCESS || ndev < 1) {
        kv("error", "cuda", "libcuda not loadable or no device");
        return 2;
    }
    cu->cuDriverGetVersion(&drv);
    cu->cuDeviceGet(&dev, 0);
    cu->cuDeviceGetName(name, sizeof(name), dev);
    cu->cuDeviceGetAttribute(&ccmaj, CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR, dev);
    cu->cuDeviceGetAttribute(&ccmin, CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR, dev);
    kv("meta", "device_name", "%s", name);
    kv("meta", "compute_capability", "%d.%d", ccmaj, ccmin);
    kv("meta", "cuda_driver_version", "%d", drv);
    kv("meta", "header_api", "%d.%d", NVENCAPI_MAJOR_VERSION, NVENCAPI_MINOR_VERSION);
    if (cu->cuCtxCreate(&ctx, 0, dev) != CUDA_SUCCESS) { kv("error", "cuda", "cuCtxCreate failed"); return 2; }

    if (nvenc_load_functions(&nv, NULL) < 0) {
        kv("error", "nvenc", "libnvidia-encode not loadable");
    } else if (nv->NvEncodeAPIGetMaxSupportedVersion(&maxver) != NV_ENC_SUCCESS) {
        kv("error", "nvenc", "NvEncodeAPIGetMaxSupportedVersion failed");
    } else {
        kv("meta", "driver_api_max", "%u.%u", maxver >> 4, maxver & 0xf);
        memset(&api, 0, sizeof(api));
        api.version = NV_ENCODE_API_FUNCTION_LIST_VER;
        st = nv->NvEncodeAPICreateInstance(&api);
        if (st != NV_ENC_SUCCESS) {
            kv("error", "nvenc", "NvEncodeAPICreateInstance %s", status_name(st));
        } else if (!(enc = open_session(ctx, &st))) {
            kv("error", "nvenc", "open session %s", status_name(st));
        } else {
            if (api.nvEncGetEncodeGUIDCount(enc, &ncodec) == NV_ENC_SUCCESS && ncodec <= 32 &&
                api.nvEncGetEncodeGUIDs(enc, guids, ncodec, &got) == NV_ENC_SUCCESS) {
                static const struct { const char *n; const GUID *g; } C[] = {
                    {"h264", &NV_ENC_CODEC_H264_GUID}, {"hevc", &NV_ENC_CODEC_HEVC_GUID}, {"av1", &NV_ENC_CODEC_AV1_GUID}};
                unsigned c;
                for (c = 0; c < 3; c++) {
                    int has = 0;
                    for (g = 0; g < got; g++) if (guid_eq(&guids[g], C[c].g)) has = 1;
                    kv("enc_codec", C[c].n, "%d", has);
                    if (has) enc_codec_block(enc, C[c].n, *C[c].g);
                }
            } else {
                kv("error", "nvenc", "nvEncGetEncodeGUIDs failed");
            }
            api.nvEncDestroyEncoder(enc);
            /* session discovery: how many encode sessions the driver lets this machine hold at once */
            {
                void *h[64];
                int n = 0;
                NVENCSTATUS last = NV_ENC_SUCCESS;
                while (n < max_sessions && (h[n] = open_session(ctx, &last))) n++;
                kv("sessions", "opened", "%d", n);
                kv("sessions", "limit_hit", "%d", n < max_sessions);
                kv("sessions", "stopped_by", "%s", n < max_sessions ? status_name(last) : "probe_max");
                while (n > 0) api.nvEncDestroyEncoder(h[--n]);
            }
        }
    }
    if (cuvid_load_functions(&cvd, NULL) < 0) kv("error", "nvdec", "libnvcuvid not loadable");
    else dec_caps();
    return 0;
}
