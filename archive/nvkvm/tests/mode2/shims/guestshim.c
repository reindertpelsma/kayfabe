#define _GNU_SOURCE
#include <stdio.h>
#include <stdarg.h>
#include <dlfcn.h>
#include <stdint.h>
#include <unistd.h>
#include <sys/mman.h>
static int (*r_ioctl)(int,unsigned long,void*);
static void* (*r_mmap)(void*,size_t,int,int,int,off_t);
static uint32_t L_hm;
int ioctl(int fd, unsigned long req, ...){
  va_list ap; va_start(ap,req); void*arg=va_arg(ap,void*); va_end(ap);
  if(!r_ioctl) r_ioctl=dlsym(RTLD_NEXT,"ioctl");
  int rc=r_ioctl(fd,req,arg);
  unsigned nr=req&0xff, ty=(req>>8)&0xff;
  if(ty==0x46 && arg){ uint32_t*p=arg;
    if(nr==0x2b) fprintf(stderr,"[G] ALLOC cls=0x%x hP=0x%x hO=0x%x ret=%d\n",p[3],p[1],p[2],rc);
    else if(nr==0x4e){ L_hm=p[2]; fprintf(stderr,"[G] MAP hMem=0x%x len=0x%lx ret=%d\n",p[2],*(uint64_t*)((char*)arg+24),rc);}
    else if(nr==0x2a) fprintf(stderr,"[G] CTRL cmd=0x%x ret=%d\n",p[2],rc);
    else if(nr==0xc9) fprintf(stderr,"[G] nr=0xc9 ret=%d\n",rc);
    else if(nr==0x4a||nr==0x27) fprintf(stderr,"[G] VIDHEAP/ALLOCMEM nr=0x%x ret=%d\n",nr,rc);
  }
  return rc;
}
void* mmap(void*a,size_t l,int pr,int fl,int fd,off_t off){
  if(!r_mmap) r_mmap=dlsym(RTLD_NEXT,"mmap");
  void*r=r_mmap(a,l,pr,fl,fd,off);
  if((fl&MAP_SHARED)&&fd>=0) fprintf(stderr,"[G] MMAP addr=%p len=0x%zx fd=%d hMem=0x%x\n",r,l,fd,L_hm);
  return r;
}
