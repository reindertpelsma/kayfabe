#define _GNU_SOURCE
#include <stdio.h>
#include <stdarg.h>
#include <dlfcn.h>
#include <stdint.h>
#include <unistd.h>
static int (*r_ioctl)(int,unsigned long,void*);
int ioctl(int fd, unsigned long req, ...){
  va_list ap; va_start(ap,req); void*arg=va_arg(ap,void*); va_end(ap);
  if(!r_ioctl) r_ioctl=dlsym(RTLD_NEXT,"ioctl");
  int rc=r_ioctl(fd,req,arg);
  unsigned nr=req&0xff, ty=(req>>8)&0xff;
  if(ty==0x46){ char path[128]={0},l[64]; snprintf(l,sizeof(l),"/proc/self/fd/%d",fd); if(readlink(l,path,127)<0){} 
    unsigned cls=0; if(arg && (nr==0x2b||nr==0x27)) cls=((uint32_t*)arg)[3];
    if(nr==0x2b||nr==0x27||nr==0xc9||nr==0x4e||nr==0x2a)
      fprintf(stderr,"[SEQ] nr=0x%02x fd=%d(%s) class=0x%x ret=%d\n",nr,fd,path,cls,rc);
  }
  return rc;
}
