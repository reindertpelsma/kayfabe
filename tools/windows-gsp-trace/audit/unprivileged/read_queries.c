/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Build against the exact OGKM version under test. Only read queries on fresh
 * caller-owned objects. Kernel/internal/admin queries are negative controls.
 */
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include <ctrl/ctrl0073/ctrl0073system.h>
#include <ctrl/ctrl0073/ctrl0073specific.h>
#include <ctrl/ctrl0080/ctrl0080gpu.h>
#include <ctrl/ctrl0080/ctrl0080fifo.h>
#include <ctrl/ctrl2080/ctrl2080gr.h>
#include <ctrl/ctrl2080/ctrl2080internal.h>
#include <ctrl/ctrl2080/ctrl2080fb.h>

typedef struct { uint32_t root,parent,object,cls; uint64_t params; uint32_t size,status; } Alloc;
typedef struct { uint32_t root,object,cmd,flags; uint64_t params; uint32_t size,status; } Control;
typedef struct { uint32_t device,share,client,target,flags,pad; uint64_t va_size,va_start,va_limit; uint32_t mode,pad2; } Device;
static int fd;
static uint32_t root;
static int alloc(uint32_t parent,uint32_t handle,uint32_t cls,void *params,uint32_t size) {
    Alloc a={root,parent,handle,cls,(uintptr_t)params,size,0};
    int rc=ioctl(fd,_IOWR('F',0x2b,Alloc),&a);
    printf("ALLOC class=0x%x rc=%d status=0x%x handle=0x%x\n",cls,rc,a.status,a.object);
    if (cls==0x41 && !rc && !a.status) root=a.object;
    return rc || a.status;
}
static uint32_t query(uint32_t object,uint32_t cmd,void *params,uint32_t size) {
    Control c={root,object,cmd,0,(uintptr_t)params,size,0xffffffff};
    int rc=ioctl(fd,_IOWR('F',0x2a,Control),&c);
    printf("QUERY cmd=0x%08x size=%u rc=%d status=0x%x\n",cmd,size,rc,c.status);
    return rc ? 0xffffffff : c.status;
}
int main(void) {
    if(getuid()!=65534 || geteuid()!=65534) return 2;
    fd=open("/dev/nvidiactl",O_RDWR); int gpu=open("/dev/nvidia0",O_RDWR);
    if(fd<0 || gpu<0) {perror("open");return 1;}
    if(ioctl(gpu,_IOWR('F',201,int),&fd)) {perror("register");return 1;}
    if(alloc(0,0,0x41,NULL,0)) return 1;
    Device dev={0}; uint32_t sub=0;
    if(alloc(root,0xcafe0001,0x80,&dev,sizeof dev) || alloc(0xcafe0001,0xcafe0002,0x2080,&sub,sizeof sub)) return 1;
    NV0080_CTRL_GPU_GET_BRAND_CAPS_PARAMS brand={0};
    query(0xcafe0001,NV0080_CTRL_CMD_GPU_GET_BRAND_CAPS,&brand,sizeof brand);
    NV0080_CTRL_FIFO_GET_LATENCY_BUFFER_SIZE_PARAMS latency={0};
    latency.engineID=1; /* NV2080_ENGINE_TYPE_GR */
    query(0xcafe0001,NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE,&latency,sizeof latency);
    NV2080_CTRL_FB_GET_ROW_REMAPPER_HISTOGRAM_PARAMS rows={0};
    query(0xcafe0002,NV2080_CTRL_CMD_FB_GET_ROW_REMAPPER_HISTOGRAM,&rows,sizeof rows);
    NV2080_CTRL_GR_GFX_POOL_QUERY_SIZE_PARAMS pool={0};pool.maxSlots=1;
    uint32_t kernel=query(0xcafe0002,NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE,&pool,sizeof pool);
    NV2080_CTRL_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE_PARAMS fecs={0};
    uint32_t internal=query(0xcafe0002,NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE,&fecs,sizeof fecs);
    uint32_t admin=0xffffffff;
    if(!alloc(0xcafe0001,0xcafe0003,0x73,NULL,0)) {
        NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS heads={0};
        query(0xcafe0003,NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS,&heads,sizeof heads);
        NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS info={0};
        admin=query(0xcafe0003,NV0073_CTRL_CMD_SPECIFIC_OR_GET_INFO,&info,sizeof info);
    }
    printf("NEGATIVE_CONTROLS kernel=0x%x internal=0x%x admin=0x%x\n",kernel,internal,admin);
    close(gpu); close(fd);
    /* Permission results are analyzed by exact status afterwards, not inferred
     * from a nonzero value here (wrong object/shape would also fail). */
    return kernel==0 || internal==0 || admin==0 ? 3 : 0;
}
