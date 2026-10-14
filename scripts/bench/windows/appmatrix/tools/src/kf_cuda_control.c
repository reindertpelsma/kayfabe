/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Windows equivalents of simpleCallback and simpleOccupancy, using the public
 * Driver API and PTX, so nvcc/MSVC/cudart are unnecessary. GPU results are checked
 * against integer arithmetic; the host callback only verifies preceding copies.
 * API contracts:
 * https://docs.nvidia.com/cuda/archive/12.6.1/cuda-driver-api/group__CUDA__EXEC.html
 * https://docs.nvidia.com/cuda/archive/12.6.1/cuda-driver-api/group__CUDA__OCCUPANCY.html
 */
#include <windows.h>
#include <cuda.h>
#include <stdio.h>
#include <string.h>

CUresult cuStreamCreate(CUstream *, unsigned);
CUresult cuStreamSynchronize(CUstream);
CUresult cuStreamDestroy_v2(CUstream);
CUresult cuMemAllocHost_v2(void **, size_t);
CUresult cuMemFreeHost(void *);
CUresult cuMemFree_v2(CUdeviceptr);
CUresult cuMemcpyDtoHAsync_v2(void *, CUdeviceptr, size_t, CUstream);
CUresult cuLaunchHostFunc(CUstream, void (CALLBACK *)(void *), void *);
CUresult cuOccupancyMaxPotentialBlockSize(int *, int *, CUfunction, size_t (CALLBACK *)(int), size_t, int);
CUresult cuOccupancyMaxActiveBlocksPerMultiprocessor(int *, CUfunction, int, size_t);
CUresult cuModuleUnload(CUmodule);
CUresult cuCtxDestroy_v2(CUcontext);
#define CK(call) do { int rc=(call); if(rc) { printf("CHECK cuda_call FAIL %s rc=%d\n",#call,rc); return 1; } } while(0)
#define N 4096
static const char ptx[] =
 ".version 6.3\n.target sm_75\n.address_size 64\n"
 ".visible .entry fill(.param .u64 dst,.param .u32 seed) {\n"
 " .reg .u32 %r<7>; .reg .u64 %d<4>; .reg .pred %p;\n"
 " mov.u32 %r0,%ctaid.x; mov.u32 %r1,%ntid.x; mov.u32 %r2,%tid.x;\n"
 " mad.lo.u32 %r3,%r0,%r1,%r2; setp.ge.u32 %p,%r3,4096; @%p bra done;\n"
 " ld.param.u32 %r4,[seed]; mad.lo.u32 %r5,%r3,3,%r4;\n"
 " ld.param.u64 %d0,[dst]; mul.wide.u32 %d1,%r3,4; add.u64 %d2,%d0,%d1;\n"
 " st.global.u32 [%d2],%r5; done: ret; }\n";

static int matches(const unsigned *v, unsigned seed) {
 for(unsigned i=0;i<N;i++) if(v[i]!=i*3+seed) return 0;
 return 1;
}
struct check { unsigned *values, seed; LONG order; volatile LONG *count, *bad; };
static void CALLBACK verify_callback(void *p) {
 struct check *c=p;
 /* No CUDA API calls or waits here: this runs on the driver's callback thread. */
 if(InterlockedIncrement(c->count)!=c->order || !matches(c->values,c->seed))
  InterlockedExchange(c->bad,1);
}

int main(int argc,char **argv) {
 setvbuf(stdout,NULL,_IONBF,0);
 if(argc!=2 || (strcmp(argv[1],"callback") && strcmp(argv[1],"occupancy"))) {
  puts("usage: kf_cuda_control callback|occupancy"); return 2;
 }
 CUdevice dev; CUcontext ctx; CUmodule mod; CUfunction fn; CUstream stream;
 CUdeviceptr dst; char name[256]={0};
 CK(cuInit(0)); CK(cuDeviceGet(&dev,0)); CK(cuDeviceGetName(name,sizeof(name),dev));
 printf("KFCUDA device %s\n",name);
 CK(cuCtxCreate(&ctx,0,dev)); CK(cuModuleLoadData(&mod,ptx));
 CK(cuModuleGetFunction(&fn,mod,"fill")); CK(cuStreamCreate(&stream,0)); CK(cuMemAlloc(&dst,N*4));
 unsigned *host[2]={0}, seeds[2]={14,41};
 CK(cuMemAllocHost_v2((void **)&host[0],N*4)); CK(cuMemAllocHost_v2((void **)&host[1],N*4));
 memset(host[0],0xee,N*4); memset(host[1],0xee,N*4);
 int block=128, grid=0, active=0, limit=0;
 if(!strcmp(argv[1],"occupancy")) {
  CK(cuOccupancyMaxPotentialBlockSize(&grid,&block,fn,NULL,0,0));
  CK(cuDeviceGetAttribute(&limit,1,dev)); /* MAX_THREADS_PER_BLOCK */
  if(grid<=0 || block<=0 || block>limit) { puts("CHECK occupancy_bounds FAIL"); return 1; }
  CK(cuOccupancyMaxActiveBlocksPerMultiprocessor(&active,fn,block,0));
  if(active<=0) { puts("CHECK occupancy_active_blocks FAIL"); return 1; }
  printf("CHECK occupancy_bounds ok grid=%d block=%d active=%d\n",grid,block,active);
 }
 volatile LONG count=0,bad=0;
 struct check checks[2]={{host[0],14,1,&count,&bad},{host[1],41,2,&count,&bad}};
 int callback=!strcmp(argv[1],"callback");
 for(int i=0;i<2;i++) {
  void *args[]={&dst,&seeds[i]};
  CK(cuLaunchKernel(fn,(N+block-1)/block,1,1,block,1,1,0,stream,args,NULL));
  CK(cuMemcpyDtoHAsync_v2(host[i],dst,N*4,stream));
  if(callback) CK(cuLaunchHostFunc(stream,verify_callback,&checks[i]));
 }
 CK(cuStreamSynchronize(stream));
 int ok=matches(host[0],14) && matches(host[1],41) && (!callback || (count==2 && bad==0));
 printf("CHECK %s %s callbacks=%ld bad=%ld\n",argv[1],ok?"ok":"FAIL",(long)count,(long)bad);
 CK(cuMemFreeHost(host[0])); CK(cuMemFreeHost(host[1])); CK(cuMemFree_v2(dst));
 CK(cuStreamDestroy_v2(stream)); CK(cuModuleUnload(mod)); CK(cuCtxDestroy_v2(ctx));
 if(ok) printf("KFCUDA RESULT OK %s\n",argv[1]);
 return ok?0:1;
}
