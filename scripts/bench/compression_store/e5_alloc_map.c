/* SPDX-License-Identifier: MIT
 * E5(a,b): unprivileged, bare-metal RM allocation/PTE observation only.
 * ABI and constants come from the open 595.91.07 headers supplied at build time.
 * No PTE writes, physical GPU operands, kernel module, CUDA or guest code.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include "nvos.h"
#include "nvmisc.h"
#include "nv_escape.h"
#include "class/cl0000.h"
#include "class/cl0040.h"
#include "class/cl0080.h"
#include "class/cl2080.h"
#include "class/cl50a0.h"
#include "ctrl/ctrl0000/ctrl0000gpu.h"
#include "ctrl/ctrl0041.h"
#include "ctrl/ctrl0080/ctrl0080dma.h"
#include "ctrl/ctrl2080/ctrl2080fb.h"
#include "dev_mmu.h"

static int ctl_fd = -1;
static NvHandle client, device, subdevice;
static unsigned failures;

static NvHandle alloc_obj(const char *label, NvHandle parent, NvU32 cls,
                          void *params, size_t size)
{
    NVOS21_PARAMETERS a = {
        .hRoot = client, .hObjectParent = parent, .hClass = cls,
        .pAllocParms = params, .paramsSize = size,
    };
    int rc = ioctl(ctl_fd, _IOWR('F', NV_ESC_RM_ALLOC, NVOS21_PARAMETERS), &a);
    printf("{\"event\":\"alloc\",\"label\":\"%s\",\"class\":%u,\"handle\":%u,"
           "\"status\":%u,\"rc\":%d,\"errno\":%d}\n",
           label, cls, a.hObjectNew, a.status, rc, rc ? errno : 0);
    if (rc || a.status || !a.hObjectNew) {
        failures++;
        return 0;
    }
    return a.hObjectNew;
}

static bool control(const char *label, NvHandle obj, NvU32 cmd, void *params, size_t size)
{
    NVOS54_PARAMETERS c = {
        .hClient = client, .hObject = obj, .cmd = cmd,
        .params = params, .paramsSize = size,
    };
    int rc = ioctl(ctl_fd, _IOWR('F', NV_ESC_RM_CONTROL, NVOS54_PARAMETERS), &c);
    printf("{\"event\":\"control\",\"label\":\"%s\",\"command\":%u,"
           "\"status\":%u,\"rc\":%d,\"errno\":%d}\n",
           label, cmd, c.status, rc, rc ? errno : 0);
    if (rc || c.status) {
        failures++;
        return false;
    }
    return true;
}

static void free_obj(NvHandle parent, NvHandle handle)
{
    if (!handle) return;
    NVOS00_PARAMETERS f = {
        .hRoot = client, .hObjectParent = parent, .hObjectOld = handle,
    };
    int rc = ioctl(ctl_fd, _IOWR('F', NV_ESC_RM_FREE, NVOS00_PARAMETERS), &f);
    printf("{\"event\":\"free\",\"handle\":%u,\"status\":%u,\"rc\":%d}\n",
           handle, f.status, rc);
    if (rc || f.status) failures++;
}

static void fb_info(const char *phase)
{
    NV2080_CTRL_FB_GET_INFO_V2_PARAMS p = {.fbInfoListSize = 3};
    p.fbInfoList[0].index = NV2080_CTRL_FB_INFO_INDEX_TOTAL_RAM_SIZE;
    p.fbInfoList[1].index = NV2080_CTRL_FB_INFO_INDEX_HEAP_FREE;
    p.fbInfoList[2].index = NV2080_CTRL_FB_INFO_INDEX_LARGEST_FREE_REGION_SIZE_KB;
    if (!control(phase, subdevice, NV2080_CTRL_CMD_FB_GET_INFO_V2, &p, sizeof(p))) return;
    for (unsigned i = 0; i < p.fbInfoListSize; ++i)
        printf("{\"event\":\"fb\",\"phase\":\"%s\",\"index\":%u,\"value_kib\":%u}\n",
               phase, p.fbInfoList[i].index, p.fbInfoList[i].data);
}

static void allocation_info(const char *phase, const NV_MEMORY_ALLOCATION_PARAMS *p)
{
    printf("{\"event\":\"allocation_params\",\"phase\":\"%s\",\"attr\":%u,"
           "\"attr2\":%u,\"format\":%u,\"compr\":%u,\"zbc\":%u,\"flags\":%u,"
           "\"size\":%" PRIu64 ",\"alignment\":%" PRIu64 ",\"offset\":%" PRIu64 ","
           "\"compr_covg\":%u,\"ctag_offset\":%u}\n",
           phase, p->attr, p->attr2, p->format,
           DRF_VAL(OS32, _ATTR, _COMPR, p->attr),
           DRF_VAL(OS32, _ATTR2, _ZBC, p->attr2), p->flags,
           (uint64_t)p->size, (uint64_t)p->alignment, (uint64_t)p->offset,
           p->comprCovg, p->ctagOffset);
}

static void surface_info(NvHandle mem)
{
    NV0041_CTRL_SURFACE_INFO entries[] = {
        {.index = NV0041_CTRL_SURFACE_INFO_INDEX_ATTRS},
        {.index = NV0041_CTRL_SURFACE_INFO_INDEX_COMPR_COVERAGE},
        {.index = NV0041_CTRL_SURFACE_INFO_INDEX_PHYS_ATTR},
    };
    NV0041_CTRL_GET_SURFACE_INFO_PARAMS p = {
        .surfaceInfoListSize = 3, .surfaceInfoList = entries,
    };
    if (control("surface_info", mem, NV0041_CTRL_CMD_GET_SURFACE_INFO, &p, sizeof(p)))
        for (unsigned i = 0; i < 3; ++i)
            printf("{\"event\":\"surface_info\",\"index\":%u,\"value\":%u}\n",
                   entries[i].index, entries[i].data);
    NV0041_CTRL_GET_SURFACE_COMPRESSION_COVERAGE_PARAMS c = {.hSubDevice = subdevice};
    if (control("coverage", mem, NV0041_CTRL_CMD_GET_SURFACE_COMPRESSION_COVERAGE,
                &c, sizeof(c)))
        printf("{\"event\":\"coverage\",\"format\":%u,\"line_min\":%u,\"line_max\":%u}\n",
               c.format, c.lineMin, c.lineMax);
}

static void map_observe(NvHandle mem, NvU32 page_flag, NvU64 page_size, NvU32 kind)
{
    NV_MEMORY_ALLOCATION_PARAMS v = {
        .type = NVOS32_TYPE_IMAGE,
        .flags = NVOS32_ALLOC_FLAGS_VIRTUAL | NVOS32_ALLOC_FLAGS_LAZY,
        .size = 65536,
    };
    NvHandle virt = alloc_obj("virtual", device, NV50_MEMORY_VIRTUAL, &v, sizeof(v));
    if (!virt) return;
    NVOS46_PARAMETERS m = {
        .hClient = client, .hDevice = device, .hDma = virt, .hMemory = mem,
        .length = 65536, .kindOverride = kind,
        .flags = DRF_DEF(OS46, _FLAGS, _PAGE_KIND_OVERRIDE, _YES)
               | DRF_NUM(OS46, _FLAGS, _PAGE_SIZE, page_flag),
    };
    int rc = ioctl(ctl_fd, _IOWR('F', NV_ESC_RM_MAP_MEMORY_DMA, NVOS46_PARAMETERS), &m);
    printf("{\"event\":\"map\",\"requested_page\":%" PRIu64 ",\"requested_kind\":%u,"
           "\"va\":%" PRIu64 ",\"status\":%u,\"rc\":%d,\"errno\":%d}\n",
           (uint64_t)page_size, kind, (uint64_t)m.dmaOffset, m.status, rc, rc ? errno : 0);
    if (rc || m.status) {
        failures++;
    } else {
        NV0080_CTRL_DMA_GET_PTE_INFO_PARAMS p = {.gpuAddr = m.dmaOffset};
        bool found = false;
        if (control("pte", device, NV0080_CTRL_CMD_DMA_GET_PTE_INFO, &p, sizeof(p))) {
            for (unsigned i = 0; i < NV0080_CTRL_DMA_GET_PTE_INFO_PTE_BLOCKS; ++i) {
                NV0080_CTRL_DMA_PTE_INFO_PTE_BLOCK *b = &p.pteBlocks[i];
                printf("{\"event\":\"pte\",\"requested_page\":%" PRIu64 ","
                       "\"requested_kind\":%u,\"block\":%u,\"page\":%" PRIu64 ","
                       "\"kind\":%u,\"flags\":%u,\"comptag_line\":%u}\n",
                       (uint64_t)page_size, kind, i, (uint64_t)b->pageSize,
                       b->kind, b->pteFlags, b->comptagLine);
                if (b->pageSize == page_size && (b->pteFlags & 1) && b->kind == kind)
                    found = true;
            }
        }
        if (!found) failures++;
    }
    /* Freeing the virtual allocation tears down its mapping before the physical object. */
    free_obj(device, virt);
}

int main(void)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("{\"event\":\"start\",\"uid\":%u,\"gid\":%u}\n", getuid(), getgid());
    if (getuid() == 0) {
        fprintf(stderr, "Run with an unprivileged uid and an empty capability set.\n");
        return 3;
    }
    ctl_fd = open("/dev/nvidiactl", O_RDWR | O_CLOEXEC);
    int gpu_fd = open("/dev/nvidia0", O_RDWR | O_CLOEXEC);
    if (ctl_fd < 0 || gpu_fd < 0) { perror("open NVIDIA nodes"); return 3; }
    client = alloc_obj("client", 0, NV01_ROOT, NULL, 0);
    if (!client) goto done;
    NV0000_CTRL_GPU_GET_ATTACHED_IDS_PARAMS ids = {0};
    if (!control("attached", client, NV0000_CTRL_CMD_GPU_GET_ATTACHED_IDS,
                 &ids, sizeof(ids))) goto done;
    NvU32 id = NV0000_CTRL_GPU_INVALID_ID;
    for (unsigned i = 0; i < 32; ++i)
        if (ids.gpuIds[i] != NV0000_CTRL_GPU_INVALID_ID) { id = ids.gpuIds[i]; break; }
    if (id == NV0000_CTRL_GPU_INVALID_ID) { failures++; goto done; }
    NV0000_CTRL_GPU_GET_ID_INFO_PARAMS info = {.gpuId = id};
    if (!control("gpu_id", client, NV0000_CTRL_CMD_GPU_GET_ID_INFO,
                 &info, sizeof(info))) goto done;
    NV0080_ALLOC_PARAMETERS dp = {.deviceId = info.deviceInstance};
    device = alloc_obj("device", client, NV01_DEVICE_0, &dp, sizeof(dp));
    if (!device) goto done;
    NV2080_ALLOC_PARAMETERS sp = {.subDeviceId = info.subDeviceInstance};
    subdevice = alloc_obj("subdevice", device, NV20_SUBDEVICE_0, &sp, sizeof(sp));
    if (!subdevice) goto done;
    fb_info("before");
    NV_MEMORY_ALLOCATION_PARAMS p = {
        .type = NVOS32_TYPE_IMAGE,
        .attr = DRF_DEF(OS32, _ATTR, _PHYSICALITY, _CONTIGUOUS)
              | DRF_DEF(OS32, _ATTR, _COMPR, _ANY)
              | DRF_DEF(OS32, _ATTR, _FORMAT, _BLOCK_LINEAR),
        .attr2 = DRF_DEF(OS32, _ATTR2, _ZBC, _PREFER_NO_ZBC),
        .format = NV_MMU_PTE_KIND_GENERIC_MEMORY_COMPRESSIBLE,
        .size = UINT64_C(1) << 30,
    };
    allocation_info("request", &p);
    NvHandle mem = alloc_obj("compressed_store", device, NV01_MEMORY_LOCAL_USER,
                             &p, sizeof(p));
    allocation_info("returned", &p);
    if (!mem) goto done;
    fb_info("allocated");
    surface_info(mem);
    map_observe(mem, NVOS46_FLAGS_PAGE_SIZE_BIG, 65536,
                NV_MMU_PTE_KIND_GENERIC_MEMORY_COMPRESSIBLE);
    map_observe(mem, NVOS46_FLAGS_PAGE_SIZE_BIG, 65536, NV_MMU_PTE_KIND_PITCH);
    map_observe(mem, NVOS46_FLAGS_PAGE_SIZE_4KB, 4096,
                NV_MMU_PTE_KIND_GENERIC_MEMORY_COMPRESSIBLE);
    map_observe(mem, NVOS46_FLAGS_PAGE_SIZE_4KB, 4096, NV_MMU_PTE_KIND_PITCH);
    free_obj(device, mem);
    fb_info("freed");
done:
    free_obj(0, client);
    close(gpu_fd);
    close(ctl_fd);
    printf("{\"event\":\"finish\",\"failures\":%u}\n", failures);
    return failures ? 2 : 0;
}
