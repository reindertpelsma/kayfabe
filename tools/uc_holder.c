/* Persistent pinned-buffer holder that stamps a unique magic at the start of
 * each page, so the host can locate the backing pages in QEMU by content. */
#include <stdio.h>
#include <dlfcn.h>
#include <unistd.h>
#include <stdint.h>
#include <string.h>
typedef int (*pfn_init)(unsigned);
typedef int (*pfn_dget)(int*,int);
typedef int (*pfn_ctx)(void**,unsigned,int);
typedef int (*pfn_alloc)(void**,size_t,unsigned);
#define MAGIC 0x5544C0DE5544C0DEULL  /* "UDCODE" */
int main(){
  void*h=dlopen("/usr/local/nvidia-guest/lib/libcuda.so.1",RTLD_NOW);
  if(!h){fprintf(stderr,"dlopen %s\n",dlerror());return 1;}
  pfn_init cuInit=dlsym(h,"cuInit");
  pfn_dget cuDeviceGet=dlsym(h,"cuDeviceGet");
  pfn_ctx cuCtxCreate=dlsym(h,"cuCtxCreate_v2");
  pfn_alloc cuMemAllocHost=dlsym(h,"cuMemAllocHost_v2");
  if(cuInit(0)){fprintf(stderr,"cuInit fail\n");return 2;}
  int dev=0; if(cuDeviceGet(&dev,0)){fprintf(stderr,"devget\n");return 3;}
  void*ctx=0; if(cuCtxCreate(&ctx,0,dev)){fprintf(stderr,"ctx\n");return 4;}
  size_t sz=8UL<<20; void*p=0; int r=cuMemAllocHost(&p,sz,0);
  if(r){fprintf(stderr,"allochost=%d\n",r);return 5;}
  volatile uint64_t*q=(uint64_t*)p;
  for(size_t off=0; off<sz; off+=4096){
    q[off/8] = MAGIC;
    q[off/8+1] = off;        /* page offset as second word */
  }
  FILE*st=fopen("/tmp/h.status","w");
  if(st){ fprintf(st,"HOLDER_OK pid=%d pinned=%p size=%zu magic=0x%llx\n",
                  getpid(),p,sz,(unsigned long long)MAGIC); fflush(st); fclose(st); }
  printf("HOLDER_OK pinned=%p size=%zu magic=0x%llx\n",p,sz,(unsigned long long)MAGIC);
  fflush(stdout);
  sleep(600);
  return 0;
}
