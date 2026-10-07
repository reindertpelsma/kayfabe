// rm_mini.h — the smallest RM + UVM userspace client the b3 tests need (C, header-only).
// Built against the pinned sources: ogkm 580.159.04 src/common/sdk/nvidia/inc (RM ABI) and
// the patched nvidia-uvm tree (UVM ABI + uvm_efs_ioctl.h). Every call returns the RM status
// (or -errno for a failed ioctl(2)), never aborts: the tests print what RM/UVM answered.
#ifndef RM_MINI_H
#define RM_MINI_H

#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "nvtypes.h"
#include "nvos.h"
#include "class/cl0000.h"
#include "class/cl0080.h"
#include "class/cl2080.h"
#include "class/cl90f1.h"
#include "ctrl/ctrl2080/ctrl2080gpu.h"
#include "rs_access.h"

#include "uvm_linux_ioctl.h"
#include "uvm_ioctl.h"
#include "uvm_efs_ioctl.h"

#define RMM_ESC_RM_FREE      0x29
#define RMM_ESC_RM_CONTROL   0x2A
#define RMM_ESC_RM_ALLOC     0x2B
#define RMM_ESC_RM_SHARE     0x35
#define RMM_IOWR(nr, sz)     _IOC(_IOC_READ | _IOC_WRITE, 'F', (nr), (sz))

#ifndef NV01_MEMORY_LOCAL_USER
#define NV01_MEMORY_LOCAL_USER 0x40
#endif

// All raw ioctls go straight to the kernel: a test binary may also define ioctl().
static inline int rmm_sys_ioctl(int fd, unsigned long req, void *arg)
{
    long r = syscall(SYS_ioctl, fd, req, arg);
    return r < 0 ? -errno : (int)r;
}

typedef struct {
    int ctl;            // /dev/nvidiactl
    NvHandle client;
    NvHandle device;
    NvHandle subdev;
    NvHandle next;
    NvProcessorUuid uuid;
} rmm_t;

static inline int rmm_alloc(rmm_t *r, NvHandle parent, NvHandle h, NvU32 cls, void *p, NvU32 psz)
{
    NVOS21_PARAMETERS a;
    memset(&a, 0, sizeof(a));
    a.hRoot = r->client;
    a.hObjectParent = parent;
    a.hObjectNew = h;
    a.hClass = cls;
    a.pAllocParms = (NvP64)(uintptr_t)p;
    a.paramsSize = psz;
    int e = rmm_sys_ioctl(r->ctl, RMM_IOWR(RMM_ESC_RM_ALLOC, sizeof(a)), &a);
    return e < 0 ? e : (int)a.status;
}

static inline int rmm_free(rmm_t *r, NvHandle parent, NvHandle h)
{
    NVOS00_PARAMETERS a;
    memset(&a, 0, sizeof(a));
    a.hRoot = r->client;
    a.hObjectParent = parent;
    a.hObjectOld = h;
    int e = rmm_sys_ioctl(r->ctl, RMM_IOWR(RMM_ESC_RM_FREE, sizeof(a)), &a);
    return e < 0 ? e : (int)a.status;
}

static inline int rmm_ctrl(rmm_t *r, NvHandle obj, NvU32 cmd, void *p, NvU32 psz)
{
    NVOS54_PARAMETERS a;
    memset(&a, 0, sizeof(a));
    a.hClient = r->client;
    a.hObject = obj;
    a.cmd = cmd;
    a.params = (NvP64)(uintptr_t)p;
    a.paramsSize = psz;
    int e = rmm_sys_ioctl(r->ctl, RMM_IOWR(RMM_ESC_RM_CONTROL, sizeof(a)), &a);
    return e < 0 ? e : (int)a.status;
}

// Share hObject with a policy (type RS_SHARE_TYPE_*, target = pid for PID, access DUP_OBJECT).
static inline int rmm_share(rmm_t *r, NvHandle obj, NvU16 type, NvU32 target)
{
    NVOS57_PARAMETERS a;
    memset(&a, 0, sizeof(a));
    a.hClient = r->client;
    a.hObject = obj;
    a.sharePolicy.type = type;
    a.sharePolicy.target = target;
    RS_ACCESS_MASK_ADD(&a.sharePolicy.accessMask, RS_ACCESS_DUP_OBJECT);
    int e = rmm_sys_ioctl(r->ctl, RMM_IOWR(RMM_ESC_RM_SHARE, sizeof(a)), &a);
    return e < 0 ? e : (int)a.status;
}

// Opens /dev/nvidiactl and allocates client, device 0, subdevice 0 and reads the GPU UUID.
static inline int rmm_open(rmm_t *r)
{
    memset(r, 0, sizeof(*r));
    r->next = 0xcafe0001;
    r->ctl = open("/dev/nvidiactl", O_RDWR | O_CLOEXEC);
    if (r->ctl < 0)
        return -errno;

    NVOS21_PARAMETERS a;
    memset(&a, 0, sizeof(a));
    a.hClass = NV01_ROOT_CLIENT;
    int e = rmm_sys_ioctl(r->ctl, RMM_IOWR(RMM_ESC_RM_ALLOC, sizeof(a)), &a);
    if (e < 0)
        return e;
    if (a.status)
        return (int)a.status;
    r->client = a.hObjectNew;

    NV0080_ALLOC_PARAMETERS d;
    memset(&d, 0, sizeof(d));
    d.deviceId = 0;
    r->device = r->next++;
    int s = rmm_alloc(r, r->client, r->device, NV01_DEVICE_0, &d, sizeof(d));
    if (s)
        return s;

    NV2080_ALLOC_PARAMETERS sd;
    memset(&sd, 0, sizeof(sd));
    r->subdev = r->next++;
    s = rmm_alloc(r, r->device, r->subdev, NV20_SUBDEVICE_0, &sd, sizeof(sd));
    if (s)
        return s;

    NV2080_CTRL_GPU_GET_GID_INFO_PARAMS g;
    memset(&g, 0, sizeof(g));
    g.flags = NV2080_GPU_CMD_GPU_GET_GID_FLAGS_FORMAT_BINARY; // | TYPE_SHA1 (0)
    s = rmm_ctrl(r, r->subdev, NV2080_CTRL_CMD_GPU_GET_GID_INFO, &g, sizeof(g));
    if (s)
        return s;
    if (g.length != sizeof(r->uuid.uuid))
        return -EPROTO;
    memcpy(r->uuid.uuid, g.data, sizeof(r->uuid.uuid));
    return 0;
}

// A fault-capable VA space as UVM wants it: ENABLE_PAGE_FAULTING | IS_EXTERNALLY_OWNED.
static inline int rmm_alloc_faulting_vas(rmm_t *r, NvHandle *out)
{
    NV_VASPACE_ALLOCATION_PARAMETERS v;
    memset(&v, 0, sizeof(v));
    v.flags = NV_VASPACE_ALLOCATION_FLAGS_ENABLE_PAGE_FAULTING | NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED;
    *out = r->next++;
    return rmm_alloc(r, r->device, *out, FERMI_VASPACE_A, &v, sizeof(v));
}

// Physically contiguous vidmem with 2 MiB pages (maps at the 2 MiB granule).
static inline int rmm_alloc_vidmem(rmm_t *r, NvU64 size, NvHandle *out)
{
    NV_MEMORY_ALLOCATION_PARAMS m;
    memset(&m, 0, sizeof(m));
    m.owner = 0x6b667566; // "kfuf"
    m.type = NVOS32_TYPE_IMAGE;
    m.flags = NVOS32_ALLOC_FLAGS_ALIGNMENT_FORCE;
    m.attr = (NVOS32_ATTR_LOCATION_VIDMEM << 25) | (NVOS32_ATTR_PAGE_SIZE_HUGE << 23) |
             (NVOS32_ATTR_PHYSICALITY_CONTIGUOUS << 27);
    m.attr2 = (NVOS32_ATTR2_PAGE_SIZE_HUGE_2MB << 20);
    m.size = size;
    m.alignment = 2ull << 20;
    *out = r->next++;
    return rmm_alloc(r, r->device, *out, NV01_MEMORY_LOCAL_USER, &m, sizeof(m));
}

// ---- UVM ------------------------------------------------------------------------------

static inline int uvm_open_init(NvU64 flags, int *fd_out, NV_STATUS *st)
{
    int fd = open("/dev/nvidia-uvm", O_RDWR | O_CLOEXEC);
    if (fd < 0)
        return -errno;
    UVM_INITIALIZE_PARAMS p;
    memset(&p, 0, sizeof(p));
    p.flags = flags;
    int e = rmm_sys_ioctl(fd, UVM_INITIALIZE, &p);
    *fd_out = fd;
    *st = p.rmStatus;
    return e;
}

static inline NV_STATUS uvm_register_gpu(int fd, const NvProcessorUuid *u)
{
    UVM_REGISTER_GPU_PARAMS p;
    memset(&p, 0, sizeof(p));
    p.gpu_uuid = *u;
    p.rmCtrlFd = -1;
    int e = rmm_sys_ioctl(fd, UVM_REGISTER_GPU, &p);
    return e < 0 ? (NV_STATUS)0xE0000000u | (NV_STATUS)(-e) : p.rmStatus;
}

static inline NV_STATUS uvm_register_gpu_vas(int fd, const NvProcessorUuid *u, int ctl, NvHandle client, NvHandle vas)
{
    UVM_REGISTER_GPU_VASPACE_PARAMS p;
    memset(&p, 0, sizeof(p));
    p.gpuUuid = *u;
    p.rmCtrlFd = ctl;
    p.hClient = client;
    p.hVaSpace = vas;
    int e = rmm_sys_ioctl(fd, UVM_REGISTER_GPU_VASPACE, &p);
    return e < 0 ? (NV_STATUS)0xE0000000u | (NV_STATUS)(-e) : p.rmStatus;
}

static inline NV_STATUS uvm_register_channel(int fd, const NvProcessorUuid *u, int ctl, NvHandle client, NvHandle ch,
                                             NvU64 base, NvU64 length)
{
    UVM_REGISTER_CHANNEL_PARAMS p;
    memset(&p, 0, sizeof(p));
    p.gpuUuid = *u;
    p.rmCtrlFd = ctl;
    p.hClient = client;
    p.hChannel = ch;
    p.base = base;
    p.length = length;
    int e = rmm_sys_ioctl(fd, UVM_REGISTER_CHANNEL, &p);
    return e < 0 ? (NV_STATUS)0xE0000000u | (NV_STATUS)(-e) : p.rmStatus;
}

static inline NV_STATUS uvm_create_ext_range(int fd, NvU64 base, NvU64 len)
{
    UVM_CREATE_EXTERNAL_RANGE_PARAMS p;
    memset(&p, 0, sizeof(p));
    p.base = base;
    p.length = len;
    int e = rmm_sys_ioctl(fd, UVM_CREATE_EXTERNAL_RANGE, &p);
    return e < 0 ? (NV_STATUS)0xE0000000u | (NV_STATUS)(-e) : p.rmStatus;
}

static inline NV_STATUS uvm_map_ext(int fd, const NvProcessorUuid *u, NvU64 base, NvU64 len, int ctl, NvHandle client,
                                    NvHandle mem, NvU32 mapping_type)
{
    UVM_MAP_EXTERNAL_ALLOCATION_PARAMS p;
    memset(&p, 0, sizeof(p));
    p.base = base;
    p.length = len;
    p.offset = 0;
    p.gpuAttributesCount = 1;
    p.perGpuAttributes[0].gpuUuid = *u;
    p.perGpuAttributes[0].gpuMappingType = mapping_type;
    p.rmCtrlFd = ctl;
    p.hClient = client;
    p.hMemory = mem;
    int e = rmm_sys_ioctl(fd, UVM_MAP_EXTERNAL_ALLOCATION, &p);
    return e < 0 ? (NV_STATUS)0xE0000000u | (NV_STATUS)(-e) : p.rmStatus;
}

static inline int efs_query(int fd, UVM_EFS_QUERY_PARAMS *q)
{
    memset(q, 0, sizeof(*q));
    return rmm_sys_ioctl(fd, UVM_EFS_QUERY, q);
}

// Returns the number of records (>= 0), or a negative value: -errno, or -(0x10000 | rmStatus).
static inline int efs_wait(int fd, UvmEfsFaultRecord *recs, unsigned max, unsigned timeout_us)
{
    UVM_EFS_WAIT_PARAMS p;
    memset(&p, 0, sizeof(p));
    p.records = (NvU64)(uintptr_t)recs;
    p.maxRecords = max;
    p.timeoutUs = timeout_us;
    int e = rmm_sys_ioctl(fd, UVM_EFS_WAIT, &p);
    if (e < 0)
        return e;
    if (p.rmStatus)
        return -(int)(0x10000u | p.rmStatus);
    return (int)p.numRecords;
}

static inline NV_STATUS efs_resolve(int fd, const NvU64 *ids, unsigned n, unsigned action, unsigned *resolved,
                                    unsigned *stale)
{
    UVM_EFS_RESOLVE_PARAMS p;
    memset(&p, 0, sizeof(p));
    p.recordIds = (NvU64)(uintptr_t)ids;
    p.count = n;
    p.action = action;
    int e = rmm_sys_ioctl(fd, UVM_EFS_RESOLVE, &p);
    if (resolved)
        *resolved = p.numResolved;
    if (stale)
        *stale = p.numStale;
    return e < 0 ? (NV_STATUS)0xE0000000u | (NV_STATUS)(-e) : p.rmStatus;
}

#endif // RM_MINI_H
