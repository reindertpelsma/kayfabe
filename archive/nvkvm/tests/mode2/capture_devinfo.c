/*
 * capture_devinfo.c — Mode-2 M5 data capture (run on the HOST, real GPU).
 *
 * Opens the real NVIDIA GPU (idle RTX 3060 / GA106), allocates a normal RM
 * client -> device -> subdevice, and issues NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE
 * (0x20801112) — paginated by baseIndex — to capture the REAL engine/device
 * info table the GSP returns.  Dumps it as a C array the QEMU emulator
 * (nvkvm_gpu_emul.c) can replay for the fake GSP in Mode-2.
 *
 * This is NON-DISRUPTIVE: it uses the GPU as a normal userspace client
 * alongside whatever else is running; it does NOT unbind/reload the driver.
 *
 * Build on host:  gcc -I/workspace/nvkvm/src/abi -O2 -o /tmp/capdev capture_devinfo.c
 * Run:            /tmp/capdev > /tmp/devinfo_ga106.h
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <errno.h>
#include <unistd.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <linux/types.h>

#include "abi/nvgpu.h"

#ifndef NV_IOCTL_MAGIC
#define NV_IOCTL_MAGIC 'F'
#endif

#define IOCTL_CHECK_VERSION_STR \
    _IOWR(NV_IOCTL_MAGIC, NV_ESC_CHECK_VERSION_STR, struct nv_ioctl_rm_api_version)
#define IOCTL_REGISTER_FD \
    _IOWR(NV_IOCTL_MAGIC, NV_ESC_REGISTER_FD, struct nv_ioctl_register_fd)
#define IOCTL_RM_ALLOC_NVOS21 \
    _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC, struct nvos21_parameters)
#define IOCTL_RM_CONTROL \
    _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_CONTROL, struct nvos54_parameters)

#define GET_DEVICE_INFO_TABLE 0x20801112u
#define MAX_ENTRIES           32
#define ENGINE_DATA_TYPES     16
#define ENGINE_MAX_PBDMA      2
#define ENGINE_MAX_NAME_LEN   16

typedef struct {
    uint32_t engineData[ENGINE_DATA_TYPES];
    uint32_t pbdmaIds[ENGINE_MAX_PBDMA];
    uint32_t pbdmaFaultIds[ENGINE_MAX_PBDMA];
    uint32_t numPbdmas;
    char     engineName[ENGINE_MAX_NAME_LEN];
} dev_entry_t;  /* 100 bytes */

typedef struct {
    uint32_t   baseIndex;
    uint32_t   numEntries;
    uint8_t    bMore;            /* NvBool */
    uint8_t    _pad[3];
    dev_entry_t entries[MAX_ENTRIES];
} devinfo_params_t;              /* 12 + 32*100 = 3212 */

static nvhandle_t alloc_obj(int fd, nvhandle_t client, nvhandle_t parent,
                            nvhandle_t newh, uint32_t cls, void *parms)
{
    struct nvos21_parameters p;
    memset(&p, 0, sizeof(p));
    p.h_root         = client;
    p.h_object_parent= parent;
    p.h_object_new   = newh;
    p.h_class        = cls;
    p.p_alloc_parms  = (nvp64_t)(uintptr_t)parms;
    int ret = ioctl(fd, IOCTL_RM_ALLOC_NVOS21, &p);
    if (ret < 0 || p.status != 0) {
        fprintf(stderr, "alloc class 0x%x failed: ret=%d errno=%d status=0x%x\n",
                cls, ret, errno, p.status);
        return 0;
    }
    return p.h_object_new;
}

int main(void)
{
    int ctl = open("/dev/nvidiactl", O_RDWR);
    if (ctl < 0) { perror("open nvidiactl"); return 1; }

    /* version handshake: query (cmd=0) then confirm (cmd=1) */
    struct nv_ioctl_rm_api_version ver;
    memset(&ver, 0, sizeof(ver));
    ver.cmd = 0;
    ioctl(ctl, IOCTL_CHECK_VERSION_STR, &ver);  /* fills version_string */
    fprintf(stderr, "driver version: %.*s\n", NV_RM_API_VERSION_STRING_LENGTH,
            ver.version_string);
    struct nv_ioctl_rm_api_version confirm;
    memset(&confirm, 0, sizeof(confirm));
    confirm.cmd = 1;
    memcpy(confirm.version_string, ver.version_string,
           NV_RM_API_VERSION_STRING_LENGTH);
    if (ioctl(ctl, IOCTL_CHECK_VERSION_STR, &confirm) != 0)
        fprintf(stderr, "WARN version confirm failed errno=%d\n", errno);

    int dev0 = open("/dev/nvidia0", O_RDWR);
    if (dev0 < 0) { perror("open nvidia0"); return 1; }
    struct nv_ioctl_register_fd reg = { .ctl_fd = ctl };
    if (ioctl(dev0, IOCTL_REGISTER_FD, &reg) != 0)
        fprintf(stderr, "WARN register_fd failed errno=%d\n", errno);

    nvhandle_t hClient = alloc_obj(ctl, 0, 0, 0xc1d00001u, NV01_ROOT_CLIENT, NULL);
    if (!hClient) return 2;

    struct nv0080_alloc_parameters dp;
    memset(&dp, 0, sizeof(dp));
    dp.device_id      = 0;
    dp.h_client_share = hClient;
    nvhandle_t hDevice = alloc_obj(ctl, hClient, hClient, 0xceda0000u,
                                   NV01_DEVICE_0, &dp);
    if (!hDevice) return 3;

    struct nv2080_alloc_parameters sp;
    memset(&sp, 0, sizeof(sp));
    sp.sub_device_id = 0;
    nvhandle_t hSub = alloc_obj(ctl, hClient, hDevice, 0xceda2080u,
                                NV20_SUBDEVICE_0, &sp);
    if (!hSub) return 4;

    fprintf(stderr, "alloc chain ok: client=0x%x device=0x%x sub=0x%x\n",
            hClient, hDevice, hSub);

    /* paginate GET_DEVICE_INFO_TABLE */
    printf("/* GA106 device-info-table captured from real GPU (driver %.*s).\n"
           " * NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE (0x20801112).\n"
           " * Each page = {baseIndex, numEntries, bMore, entries[numEntries]}.\n"
           " */\n", NV_RM_API_VERSION_STRING_LENGTH, ver.version_string);

    uint32_t baseIndex = 0;
    int page = 0, total = 0;
    for (;;) {
        devinfo_params_t params;
        memset(&params, 0, sizeof(params));
        params.baseIndex = baseIndex;
        struct nvos54_parameters c;
        memset(&c, 0, sizeof(c));
        c.h_client     = hClient;
        c.h_object     = hSub;
        c.cmd          = GET_DEVICE_INFO_TABLE;
        c.params       = (nvp64_t)(uintptr_t)&params;
        c.params_size  = sizeof(params);
        int ret = ioctl(ctl, IOCTL_RM_CONTROL, &c);
        if (ret < 0 || c.status != 0) {
            fprintf(stderr, "control 0x20801112 page %d failed: ret=%d errno=%d status=0x%x\n",
                    page, ret, errno, c.status);
            return 5;
        }
        fprintf(stderr, "page %d: baseIndex=%u numEntries=%u bMore=%u\n",
                page, params.baseIndex, params.numEntries, params.bMore);

        printf("\n/* page %d: baseIndex=%u numEntries=%u bMore=%u */\n",
               page, params.baseIndex, params.numEntries, params.bMore);
        printf("static const unsigned char devinfo_page%d[] = {", page);
        const unsigned char *raw = (const unsigned char *)&params;
        /* emit header (12B) + numEntries*sizeof(entry) bytes */
        size_t emit = 12 + (size_t)params.numEntries * sizeof(dev_entry_t);
        for (size_t i = 0; i < emit; i++) {
            if (i % 16 == 0) printf("\n  ");
            printf("0x%02x,", raw[i]);
        }
        printf("\n};\n");

        for (uint32_t i = 0; i < params.numEntries; i++) {
            dev_entry_t *e = &params.entries[i];
            fprintf(stderr, "  [%u] engineData[0..3]=%u,%u,%u,%u name='%.*s' numPbdmas=%u\n",
                    total + i, e->engineData[0], e->engineData[1],
                    e->engineData[2], e->engineData[3],
                    ENGINE_MAX_NAME_LEN, e->engineName, e->numPbdmas);
        }
        total += params.numEntries;
        page++;
        if (!params.bMore) break;
        baseIndex += params.numEntries ? params.numEntries : MAX_ENTRIES;
        if (page > 16) { fprintf(stderr, "too many pages, bail\n"); break; }
    }
    fprintf(stderr, "TOTAL engines: %d across %d page(s)\n", total, page);
    printf("\n#define DEVINFO_GA106_PAGES %d\n", page);
    return 0;
}
