/*
 * Hardware-step helper for the H5 patch. COMPILED ONLY by the author (no GPU was available or
 * allowed); the first run is the owner's/coordinator's, per V3_H5_USERD_DMA_PATCH.md section 6.
 *
 *   kf_h5_hwtest probe                 NV_ESC_KF_QUERY on /dev/nvidiactl; prints what the module says.
 *                                       On a stock module: ioctl fails with EINVAL and the kernel
 *                                       logs "NVRM:unknown NVRM ioctl command: 0xf0" once.
 *   kf_h5_hwtest window [N [BITS]]     RM client + device on GPU N (RM device instance N, default 0),
 *                                       then (a) a stock OS-descriptor allocation of 64 MiB of this
 *                                       process's memory and (b) the same through
 *                                       NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW with BITS (default 40).
 *                                       Prints the escape's verdict/mask width. The I/O virtual
 *                                       addresses are read from the kernel's dma tracepoints:
 *                                       see the test plan (T4). The objects are freed on exit.
 *
 * Build: gcc -O1 -Wall -Wextra -I ../../include -I ../stub kf_h5_hwtest.c -o kf_h5_hwtest
 *        (the stub dir supplies nvtypes.h / nv-ioctl-numbers.h exactly as the unit test does)
 * Uses only unprivileged RM verbs on a root client of its own; maps nothing into any GPU VA.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>
#include "nv-kf-host-patch.h"

#define ESC_REGISTER_FD   (NV_IOCTL_BASE + 1)
#define ESC_RM_ALLOC      0x2B
#define ESC_RM_FREE       0x29
#define CLASS_ROOT        0x41
#define CLASS_DEVICE_0    0x80
#define CLASS_OSDESC      0x71
#define OSDESC_FLAGS      0x40001010u /* LOCATION_PCI | PHYSICALITY_NONCONTIGUOUS | COHERENCY_CACHED | MAPPING_NO_MAP */

#define IOWR(nr, size) _IOC(_IOC_READ | _IOC_WRITE, NV_IOCTL_MAGIC, (nr), (size))

struct nvos21 { uint32_t root, parent, new_, cls; uint64_t parms; uint32_t psize, status; };           /* 32 */
struct nv0080 { uint32_t device_id, share, target_client, target_device, flags, pad; uint64_t va_size, va_start, va_limit; uint32_t va_mode, pad2; }; /* 56 */
struct nvos02fd { uint32_t root, parent, new_, cls, flags, pad0; uint64_t pmem, limit; uint32_t status, pad1; int32_t fd, pad2; };  /* 56 */
struct nvos00 { uint32_t root, parent, object, status; };                                               /* 16 */

static int rm_alloc(int fd, uint32_t root, uint32_t parent, uint32_t obj, uint32_t cls, void *parms, uint32_t psize)
{
    struct nvos21 a = { root, parent, obj, cls, (uint64_t)(uintptr_t)parms, psize, 0 };
    if (ioctl(fd, IOWR(ESC_RM_ALLOC, sizeof a), &a) != 0) { perror("RM_ALLOC"); return -1; }
    if (a.status != 0) { fprintf(stderr, "RM_ALLOC class 0x%x: status 0x%x\n", cls, a.status); return -1; }
    return 0;
}

static void fill_osdesc(struct nvos02fd *p, uint32_t client, uint32_t dev, uint32_t obj, void *mem, size_t len)
{
    memset(p, 0, sizeof *p);
    p->root = client; p->parent = dev; p->new_ = obj; p->cls = CLASS_OSDESC; p->flags = OSDESC_FLAGS;
    p->pmem = (uint64_t)(uintptr_t)mem; p->limit = len - 1; p->fd = -1;
}

int main(int argc, char **argv)
{
    const char *mode = argc > 1 ? argv[1] : "probe";
    int ctl = open("/dev/nvidiactl", O_RDWR | O_CLOEXEC);
    if (ctl < 0) { perror("open nvidiactl"); return 2; }

    if (!strcmp(mode, "probe")) {
        nv_ioctl_kf_query_t q; memset(&q, 0, sizeof q);
        if (ioctl(ctl, IOWR(NV_ESC_KF_QUERY, sizeof q), &q) != 0) {
            printf("PROBE absent (errno %d %s) -> stock module\n", errno, strerror(errno));
            return 0;
        }
        printf("PROBE present abi=%u features=0x%x h5_abi=%u h5_usable=%d\n", q.abi, q.features,
               q.feature_abi[NV_KF_FEATURE_H5_DMA_WINDOW_BIT],
               (q.features & NV_KF_FEATURE_H5_DMA_WINDOW) && q.feature_abi[0] == NV_KF_H5_DMA_WINDOW_ABI);
        return 0;
    }

    int n = argc > 2 ? atoi(argv[2]) : 0;
    unsigned bits = argc > 3 ? (unsigned)atoi(argv[3]) : 40;
    char path[64]; snprintf(path, sizeof path, "/dev/nvidia%d", n);
    int gpu = open(path, O_RDWR | O_CLOEXEC);
    if (gpu < 0) { perror("open gpu"); return 2; }
    int cfd = ctl;
    if (ioctl(gpu, IOWR(ESC_REGISTER_FD, sizeof cfd), &cfd) != 0) { perror("REGISTER_FD"); return 2; }

    const uint32_t client = 0xCAFE0000, dev = 0xCAFE0001, o_stock = 0xCAFE0010, o_win = 0xCAFE0011;
    if (rm_alloc(ctl, 0, 0, client, CLASS_ROOT, NULL, 0)) return 3;
    struct nv0080 dp; memset(&dp, 0, sizeof dp); dp.device_id = (uint32_t)n;
    if (rm_alloc(ctl, client, client, dev, CLASS_DEVICE_0, &dp, sizeof dp)) return 3;

    const size_t len = 64u << 20;
    char *mem = mmap(NULL, 2 * len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (mem == MAP_FAILED) { perror("mmap"); return 3; }
    memset(mem, 0xa5, 2 * len);

    struct nvos02fd s; fill_osdesc(&s, client, dev, o_stock, mem, len);
    int rc = ioctl(gpu, IOWR(0x27, sizeof s), &s);
    printf("STOCK  osdesc: ioctl=%d status=0x%x\n", rc, s.status);

    struct nvos02fd w; fill_osdesc(&w, client, dev, o_win, mem + len, len);
    nv_ioctl_kf_alloc_mem_dma_window_t k; memset(&k, 0, sizeof k);
    k.abi = NV_KF_H5_DMA_WINDOW_ABI; k.dma_bits = bits; k.inner_size = sizeof w; k.inner_ptr = (uint64_t)(uintptr_t)&w;
    rc = ioctl(gpu, IOWR(NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW, sizeof k), &k);
    printf("WINDOW osdesc: ioctl=%d errno=%d inner_status=0x%x applied=%u verdict=%u mask_bits=%u\n",
           rc, rc ? errno : 0, w.status, k.applied, k.verdict, k.mask_bits);

    /* free both objects, then the client (frees the device) */
    for (int i = 0; i < 2; i++) {
        struct nvos00 f = { client, dev, i ? o_win : o_stock, 0 };
        ioctl(ctl, IOWR(ESC_RM_FREE, sizeof f), &f);
    }
    struct nvos00 f = { client, client, client, 0 };
    ioctl(ctl, IOWR(ESC_RM_FREE, sizeof f), &f);
    printf("DONE\n");
    return 0;
}
