// rmlaunch.c -- E6' N4 raw-RM userspace compute launcher (no libcuda).
// Stages (each prints [Sx] status; a partial run is still reportable):
//   S0  open devices; alloc client, NV01_DEVICE_0, NV20_SUBDEVICE_0
//   S1  alloc VAS (normal) AND a fault-capable VAS (ENABLE_PAGE_FAULTING|IS_EXTERNALLY_OWNED)
//   S2  alloc sysmem buffers (gpfifo,pushbuf,program,qmd,cbank,param,out,sem); map GPU VA + CPU
//   S3  alloc TSG + AMPERE_CHANNEL_GPFIFO_A + AMPERE_COMPUTE_B; USERMODE doorbell; work-submit token
//   S4  channel semaphore release (proves gpfifo+doorbell+pushbuffer end-to-end)
//   S5  build QMD, SET_OBJECT + SEND_PCAS, doorbell, poll out==7  (M1)
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <stdint.h>

#include "nvtypes.h"
#include "nvos.h"
#include "class/cl0000.h"
#include "class/cl0080.h"
#include "class/cl2080.h"
#include "class/cl90f1.h"          // FERMI_VASPACE_A params (NV_VASPACE_ALLOCATION_PARAMETERS in nvos.h)
#include "class/clc56f.h"          // AMPERE_CHANNEL_GPFIFO_A methods
#include "class/clc7c0.h"          // AMPERE_COMPUTE_B methods
#include "alloc/alloc_channel.h"   // NV_CHANNEL_ALLOC_PARAMS
#include "ctrl/ctrlc36f.h"         // GET_WORK_SUBMIT_TOKEN
#include "ctrl/ctrl2080/ctrl2080gpu.h"  // NV2080_CTRL_CMD_GPU_GET_ID
#include "ctrl/ctrl0080/ctrl0080fifo.h"

#ifndef NV_IOCTL_MAGIC
#define NV_IOCTL_MAGIC 'F'
#endif
#define NV_ESC_RM_ALLOC_MEMORY   0x27
#define NV_ESC_RM_FREE           0x29
#define NV_ESC_RM_CONTROL        0x2A
#define NV_ESC_RM_ALLOC          0x2B
#define NV_ESC_RM_VID_HEAP_CTRL  0x4A
#define NV_ESC_RM_MAP_MEMORY     0x4E
#define NV_ESC_RM_MAP_MEMORY_DMA 0x57
#define NV_ESC_REGISTER_FD       0xC9   /* NV_IOCTL_BASE(200)+1 */
#define NV_ESC_ATTACH_GPUS_TO_FD 0xD4   /* NV_IOCTL_BASE(200)+12 */
struct kf_register_fd { int ctl_fd; };

#define IOWR(nr,sz) _IOC(_IOC_READ|_IOC_WRITE, NV_IOCTL_MAGIC, (nr), (sz))

static int ctlfd, devfd;
#define HCLIENT 0xc1d00001u
static uint32_t hClient, hDevice=0xca000001, hSubdev=0xca000002, hVas=0xca000003, hVasF=0xca000004;

static int rm_alloc(uint32_t hParent, uint32_t hNew, uint32_t cls, void *p, uint32_t psz){
    NVOS21_PARAMETERS a; memset(&a,0,sizeof(a));
    a.hRoot=hClient; a.hObjectParent=hParent; a.hObjectNew=hNew; a.hClass=cls;
    a.pAllocParms=(NvP64)(uintptr_t)p; a.paramsSize=psz;
    int r=ioctl(ctlfd, IOWR(NV_ESC_RM_ALLOC, sizeof(a)), &a);
    if(r<0){ printf("   ioctl RM_ALLOC cls=0x%x errno=%d(%s)\n",cls,errno,strerror(errno)); return -1; }
    return (int)a.status;
}
static int rm_ctrl(uint32_t hObj, uint32_t cmd, void *p, uint32_t psz){
    NVOS54_PARAMETERS a; memset(&a,0,sizeof(a));
    a.hClient=hClient; a.hObject=hObj; a.cmd=cmd; a.params=(NvP64)(uintptr_t)p; a.paramsSize=psz;
    int r=ioctl(ctlfd, IOWR(NV_ESC_RM_CONTROL, sizeof(a)), &a);
    if(r<0){ printf("   ioctl RM_CONTROL cmd=0x%x errno=%d(%s)\n",cmd,errno,strerror(errno)); return -1; }
    return (int)a.status;
}

#define NV01_MEMORY_SYSTEM 0x3e
#define ATTR_SYSMEM ((1u<<25)|(2u<<27)|(5u<<29))   // LOCATION_PCI | PHYSICALITY_CONTIGUOUS | COHERENCY_WRITE_BACK
typedef struct { uint32_t h; uint64_t gpuva; void* cpu; uint64_t moff; size_t size; } Buf;
static uint32_t next_h=0xb0000000;

static int alloc_sysmem(Buf *b, size_t size){
    b->h=++next_h; b->size=size;
    NV_MEMORY_ALLOCATION_PARAMS p; memset(&p,0,sizeof(p));
    p.owner=0x6e766b6d; p.type=NVOS32_TYPE_IMAGE; p.flags=0;
    p.attr=ATTR_SYSMEM; p.attr2=0; p.size=size; p.alignment=4096;
    int s=rm_alloc(hDevice,b->h,NV01_MEMORY_SYSTEM,&p,sizeof(p));
    printf("   alloc_sysmem req=0x%zx status=0x%x  ret: size=0x%llx attr=0x%x attr2=0x%x addr=0x%llx\n",
           size,s,(unsigned long long)p.size,p.attr,p.attr2,(unsigned long long)(uintptr_t)p.address);
    if(s) return s;
    return 0;
}
struct nvos33_with_fd { NVOS33_PARAMETERS params; int fd; };
static int cpu_map(Buf *b){
    uint32_t hdev_try[2]={hDevice,hSubdev}; int r=0,ok=0; struct nvos33_with_fd a;
    for(int i=0;i<2;i++){
      memset(&a,0,sizeof(a));
      a.params.hClient=hClient; a.params.hDevice=hdev_try[i]; a.params.hMemory=b->h;
      a.params.length=b->size; a.params.flags=0; a.fd=devfd;
      r=ioctl(ctlfd, IOWR(NV_ESC_RM_MAP_MEMORY,sizeof(a)), &a);   // CTL_DEVICE_ONLY
      printf("   RM_MAP_MEMORY hDev=0x%x r=%d status=0x%x\n",hdev_try[i],r,a.params.status);
      if(r==0 && a.params.status==0){ok=1;break;}
    }
    if(!ok) return -1;
    b->moff=(uint64_t)a.params.pLinearAddress;
    void *m=mmap(NULL,b->size,PROT_READ|PROT_WRITE,MAP_SHARED,devfd,(off_t)b->moff);
    if(m==MAP_FAILED){printf("   mmap off=0x%llx errno=%d(%s)\n",(unsigned long long)b->moff,errno,strerror(errno)); return -1;}
    b->cpu=m; return 0;
}
static int gpu_map(Buf *b, uint32_t vas){
    NVOS46_PARAMETERS a; memset(&a,0,sizeof(a));
    a.hClient=hClient; a.hDevice=hDevice; a.hDma=vas; a.hMemory=b->h;
    a.offset=0; a.length=b->size; a.flags=0;
    int r=ioctl(ctlfd, IOWR(NV_ESC_RM_MAP_MEMORY_DMA,sizeof(a)), &a);
    if(r<0||a.status){printf("   RM_MAP_MEMORY_DMA r=%d status=0x%x errno=%d\n",r,a.status,errno); return -1;}
    b->gpuva=a.dmaOffset; return 0;
}

int main(void){
    printf("=== rmlaunch E6' ===\n");
    ctlfd=open("/dev/nvidiactl",O_RDWR); devfd=open("/dev/nvidia0",O_RDWR);
    printf("[S0] ctlfd=%d devfd=%d\n",ctlfd,devfd);
    if(ctlfd<0||devfd<0){printf("open failed errno=%d\n",errno);return 1;}
    { struct kf_register_fd r; r.ctl_fd=ctlfd;
      int rc=ioctl(devfd, IOWR(NV_ESC_REGISTER_FD,sizeof(r)), &r);
      printf("[S0] register_fd(devfd->ctlfd) rc=%d errno=%d\n",rc,rc?errno:0); }

    // ---- client (RM-assigned handle) ----
    { NV0000_ALLOC_PARAMETERS p; memset(&p,0,sizeof(p));
      p.hClient=0; p.processID=getpid(); strncpy(p.processName,"rmlaunch",NV_PROC_NAME_MAX_LENGTH-1);
      NVOS21_PARAMETERS a; memset(&a,0,sizeof(a));
      a.hRoot=0; a.hObjectParent=NV01_NULL_OBJECT; a.hObjectNew=0; a.hClass=NV01_ROOT;
      a.pAllocParms=(NvP64)(uintptr_t)&p; a.paramsSize=sizeof(p);
      int r=ioctl(ctlfd, IOWR(NV_ESC_RM_ALLOC,sizeof(a)), &a);
      hClient = a.hObjectNew ? a.hObjectNew : p.hClient;
      printf("[S0] client alloc r=%d status=0x%x hClient=0x%x (p.hClient=0x%x)\n",r,a.status,hClient,p.hClient);
      if(r<0||a.status){printf("client FAILED\n");return 1;}
    }
    // ---- device ----
    { NV0080_ALLOC_PARAMETERS p; memset(&p,0,sizeof(p));
      p.deviceId=0; p.hClientShare=hClient;
      int s=rm_alloc(hClient,hDevice,NV01_DEVICE_0,&p,sizeof(p));
      printf("[S0] device  status=0x%x\n",s); if(s)return 1; }
    // ---- subdevice ----
    { NV2080_ALLOC_PARAMETERS p; memset(&p,0,sizeof(p)); p.subDeviceId=0;
      int s=rm_alloc(hDevice,hSubdev,NV20_SUBDEVICE_0,&p,sizeof(p));
      printf("[S0] subdev  status=0x%x\n",s); if(s)return 1; }
    // ---- attach GPU to devfd (libcuda does this; needed for CPU mmap context) ----
    { NV2080_CTRL_GPU_GET_ID_PARAMS g; memset(&g,0,sizeof(g));
      int s=rm_ctrl(hSubdev, NV2080_CTRL_CMD_GPU_GET_ID, &g, sizeof(g));
      printf("[S0] GPU_GET_ID status=0x%x gpuId=0x%x\n",s,g.gpuId);
      NvU32 ids[1]={g.gpuId};
      int rc=ioctl(devfd, IOWR(NV_ESC_ATTACH_GPUS_TO_FD, sizeof(ids)), ids);
      printf("[S0] attach_gpus_to_fd(devfd) rc=%d errno=%d\n",rc,rc?errno:0); }

    // ---- VAS (normal) ----
    { NV_VASPACE_ALLOCATION_PARAMETERS p; memset(&p,0,sizeof(p));
      p.index=NV_VASPACE_ALLOCATION_INDEX_GPU_NEW; p.flags=0;
      int s=rm_alloc(hDevice,hVas,FERMI_VASPACE_A,&p,sizeof(p));
      printf("[S1] VAS normal   status=0x%x\n",s); if(s)return 1; }
    // ---- VAS (fault-capable, externally owned) : the N4 faulting VAS ----
    { NV_VASPACE_ALLOCATION_PARAMETERS p; memset(&p,0,sizeof(p));
      p.index=NV_VASPACE_ALLOCATION_INDEX_GPU_NEW;
      p.flags=NV_VASPACE_ALLOCATION_FLAGS_ENABLE_PAGE_FAULTING|NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED;
      int s=rm_alloc(hDevice,hVasF,FERMI_VASPACE_A,&p,sizeof(p));
      printf("[S1] VAS FAULTING status=0x%x  (flags=0x%x ENABLE_PAGE_FAULTING|IS_EXTERNALLY_OWNED)\n",s,p.flags);
      printf("[S1] ^ this is the feasibility test: can UNPRIV userspace create the N4 fault-capable VAS?\n"); }

    // ---- S2: memory alloc + CPU map + GPU map (on the NORMAL vas) ----
    { Buf t;
      if(alloc_sysmem(&t,0x10000)){printf("[S2] alloc FAILED\n");return 1;}
      printf("[S2] sysmem alloc OK h=0x%x size=0x%zx\n",t.h,t.size);
      int cm=cpu_map(&t);
      if(cm==0) printf("[S2] cpu_map OK cpu=%p moff=0x%llx\n",t.cpu,(unsigned long long)t.moff);
      else       printf("[S2] cpu_map FAILED (continuing to gpu_map)\n");
      int gm=gpu_map(&t,hVas);
      if(gm==0) printf("[S2] gpu_map OK gpuva=0x%llx  (RM_MAP_MEMORY_DMA into normal VAS works)\n",(unsigned long long)t.gpuva);
      else       printf("[S2] gpu_map FAILED\n");
      if(cm==0){ *(volatile uint32_t*)t.cpu=0xdeadbeef;
        printf("[S2] cpu wrote 0x%x reads 0x%x\n",0xdeadbeef,*(volatile uint32_t*)t.cpu); }
      printf("[S2] summary: sysmem_alloc=OK cpu_map=%s gpu_map=%s\n", cm?"FAIL":"OK", gm?"FAIL":"OK");
    }
    printf("[DONE] reached end of implemented stages\n");
    return 0;
}
