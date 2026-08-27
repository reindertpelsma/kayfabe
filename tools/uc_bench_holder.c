/* Alloc a pinned buffer, then loop forever: memcpy READ+WRITE bench, append
 * GB/s to /tmp/rate.txt each iter. Lets an external module flip the PTE memtype
 * mid-run so we observe the before/after effect. */
#include <stdio.h>
#include <dlfcn.h>
#include <unistd.h>
#include <stdint.h>
#include <string.h>
#include <time.h>
#include <stdlib.h>
typedef int (*pfn_init)(unsigned);
typedef int (*pfn_dget)(int*,int);
typedef int (*pfn_ctx)(void**,unsigned,int);
typedef int (*pfn_alloc)(void**,size_t,unsigned);
static double now(){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec+t.tv_nsec*1e-9;}
int main(){
  void*h=dlopen("/usr/local/nvidia-guest/lib/libcuda.so.1",RTLD_NOW);
  if(!h){fprintf(stderr,"dlopen %s\n",dlerror());return 1;}
  pfn_init cuInit=dlsym(h,"cuInit"); pfn_dget cuDeviceGet=dlsym(h,"cuDeviceGet");
  pfn_ctx cuCtxCreate=dlsym(h,"cuCtxCreate_v2"); pfn_alloc cuMemAllocHost=dlsym(h,"cuMemAllocHost_v2");
  if(cuInit(0)){fprintf(stderr,"cuInit\n");return 2;}
  int dev=0; if(cuDeviceGet(&dev,0))return 3; void*ctx=0; if(cuCtxCreate(&ctx,0,dev))return 4;
  size_t sz=8UL<<20; void*p=0; if(cuMemAllocHost(&p,sz,0))return 5;
  void*tmp=malloc(sz); memset(tmp,1,sz); memcpy(p,tmp,sz);
  FILE*st=fopen("/tmp/h.status","w");
  if(st){fprintf(st,"HOLDER_OK pid=%d pinned=%p size=%zu\n",getpid(),p,sz);fflush(st);fclose(st);}
  for(;;){
    int iters=40; double t0=now();
    for(int i=0;i<iters;i++) memcpy(p,tmp,sz);
    double wr=(double)iters*sz/(now()-t0)/1e9;
    t0=now();
    for(int i=0;i<iters;i++) memcpy(tmp,p,sz);
    double rd=(double)iters*sz/(now()-t0)/1e9;
    FILE*r=fopen("/tmp/rate.txt","w");
    if(r){fprintf(r,"WRITE %.2f GB/s  READ %.2f GB/s\n",wr,rd);fclose(r);}
  }
}
