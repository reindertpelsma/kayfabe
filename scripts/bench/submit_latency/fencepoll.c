// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// fencepoll.c PID FD HI32 LO32 SECONDS OUT: external observer of the guest's fence word (host side, read-only).
// Maps the guest-RAM memfd of the running QEMU read-only (/proc/PID/fd/FD), scans it for the 8-byte word
// (HI32<<32 | LO32), then busy-polls every match and logs each change with CLOCK_MONOTONIC (the clock bpftrace's
// nsecs uses). Reads only; writes nothing into the guest; no ptrace, no change to the VM.
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <fcntl.h>
#include <time.h>
#include <sched.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
static inline uint64_t now(void){ struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return (uint64_t)t.tv_sec*1000000000ull+t.tv_nsec; }
int main(int argc,char**argv){
  if(argc<7){fprintf(stderr,"usage: fencepoll PID FD HI32 LO32 SECONDS OUT [CPU]\n");return 2;}
  char p[128]; snprintf(p,sizeof p,"/proc/%s/fd/%s",argv[1],argv[2]);
  uint32_t hi=strtoul(argv[3],0,0), lo=strtoul(argv[4],0,0); int secs=atoi(argv[5]);
  if(argc>7){cpu_set_t s;CPU_ZERO(&s);CPU_SET(atoi(argv[7]),&s);sched_setaffinity(0,sizeof s,&s);}
  int fd=open(p,O_RDONLY); if(fd<0){perror(p);return 1;}
  struct stat st; fstat(fd,&st); size_t len=st.st_size;
  const volatile uint32_t*m=mmap(0,len,PROT_READ,MAP_SHARED,fd,0); if(m==MAP_FAILED){perror("mmap");return 1;}
  FILE*o=fopen(argv[6],"w"); setvbuf(o,0,_IOFBF,1<<20);
  const volatile uint64_t*cand[16]; uint64_t last[16]; int nc=0;
  uint64_t t_scan0=now();
  for(;;){
    nc=0;
    for(size_t i=0;i+1<len/4 && nc<16;i+=2){           // aligned qwords: lo at even dword, hi next
      if(m[i+1]==hi && m[i]==lo){ cand[nc]=(const volatile uint64_t*)(m+i); nc++; }
    }
    if(nc>0)break;
    if(now()-t_scan0>(uint64_t)120e9){fprintf(o,"# no match\n");return 3;}
  }
  fprintf(o,"# scan_done %llu matches=%d scan_ms=%llu\n",(unsigned long long)now(),nc,(unsigned long long)((now()-t_scan0)/1000000));
  for(int c=0;c<nc;c++){ last[c]=*cand[c]; fprintf(o,"# cand %d off=0x%zx val=%llx\n",c,(size_t)((const volatile uint32_t*)cand[c]-m)*4,(unsigned long long)last[c]); }
  uint64_t end=now()+(uint64_t)secs*1000000000ull;
  size_t nev=0; unsigned long spin=0;
  while(1){
    for(int c=0;c<nc;c++){
      uint64_t v=*cand[c];
      if(v!=last[c]){ uint64_t t=now(); fprintf(o,"F %llu %d %llx\n",(unsigned long long)t,c,(unsigned long long)v); last[c]=v; nev++; }
    }
    if(((++spin)&0xFFFF)==0 && now()>end)break;
  }
  fprintf(o,"# end %llu events=%zu\n",(unsigned long long)now(),nev); fclose(o); return 0;
}
