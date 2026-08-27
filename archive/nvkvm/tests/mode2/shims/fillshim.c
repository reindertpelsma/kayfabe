#define _GNU_SOURCE
#include <stdio.h>
#include <stdarg.h>
#include <dlfcn.h>
#include <stdint.h>
#include <string.h>
#include <sys/mman.h>
static int (*r_ioctl)(int,unsigned long,void*);
static void* (*r_mmap)(void*,size_t,int,int,int,off_t);
#define N 128
static struct { void*a; size_t l; int filled; uint32_t hm; } reg[N]; static int nreg;
static uint32_t L_hm,L_hc; static uint64_t L_len;
static int nonzero(void*a,size_t l){ size_t n=l<4096?l:4096; volatile unsigned char*p=a; for(size_t i=0;i<n;i++) if(p[i]) return 1; return 0; }
static void scan(const char*tag){
  for(int i=0;i<nreg;i++) if(!reg[i].filled && reg[i].a && nonzero(reg[i].a,reg[i].l)){
    reg[i].filled=1;
    fprintf(stderr,"[FILL] addr=%p len=0x%zx hMem=0x%x became NON-ZERO after %s\n",reg[i].a,reg[i].l,reg[i].hm,tag);
  }
}
int ioctl(int fd, unsigned long req, ...){
  va_list ap; va_start(ap,req); void*arg=va_arg(ap,void*); va_end(ap);
  if(!r_ioctl) r_ioctl=dlsym(RTLD_NEXT,"ioctl");
  int r=r_ioctl(fd,req,arg);
  unsigned nr=req&0xff, ty=(req>>8)&0xff;
  if(ty==0x46 && nr==0x4e && arg){ uint32_t*p=arg; L_hc=p[0];L_hm=p[2]; L_len=*(uint64_t*)((char*)arg+24); }
  if(ty==0x46 && arg){ char tag[120]; uint32_t*p=arg;
    /* dump first 4 dwords: alloc class@12, control cmd@8 */
    snprintf(tag,sizeof(tag),"nr=0x%02x d[0..3]=%08x,%08x,%08x,%08x",nr,p[0],p[1],p[2],p[3]);
    scan(tag);
  }
  return r;
}
void* mmap(void*a,size_t l,int pr,int fl,int fd,off_t off){
  if(!r_mmap) r_mmap=dlsym(RTLD_NEXT,"mmap");
  void*r=r_mmap(a,l,pr,fl,fd,off);
  if((fl&MAP_SHARED)&&fd>=0&&r!=MAP_FAILED&&nreg<N){ reg[nreg].a=r; reg[nreg].l=l; reg[nreg].filled=nonzero(r,l); reg[nreg].hm=L_hm;
    fprintf(stderr,"[MMAP] addr=%p len=0x%zx fd=%d hMem=0x%x initnz=%d\n",r,l,fd,L_hm,reg[nreg].filled); nreg++; }
  return r;
}
