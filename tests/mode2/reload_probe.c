// reload_probe.c — minimal driver-API adapter probe used by the Q2 driver-reload
// reproducer (scripts/mode2_diag/m554_reload_host.sh). Pure CUDA Driver API: no cudart,
// no UVM-managed, no events — so a PASS/FAIL isolates the emulated-GSP adapter init from
// all higher-level subsystems. Run it once per "init cycle"; the harness runs it before
// and after a close+reopen and an rmmod+insmod to prove a SECOND GSP init in one QEMU
// lifetime works (currently it does not: first init cuInit=0, any later init cuInit=999
// "WPR2 still up"). Build: gcc -O0 -o reload_probe reload_probe.c -lcuda
#include <stdio.h>
#include <cuda.h>
int main(void){
  CUresult r=cuInit(0); printf("cuInit=%d\n",(int)r);
  if(r){const char*s=0;cuGetErrorString(r,&s);printf("  err=%s\n",s?s:"?");return 0;}
  int n=0; r=cuDeviceGetCount(&n); printf("cuDeviceGetCount=%d n=%d\n",(int)r,n);
  if(n>0){CUdevice d;cuDeviceGet(&d,0);char nm[128]={0};cuDeviceGetName(nm,127,d);printf("dev0=%s\n",nm);
    CUcontext c; r=cuCtxCreate(&c,0,d); printf("cuCtxCreate=%d\n",(int)r);
    if(!r){size_t fr=0,to=0; r=cuMemGetInfo(&fr,&to); printf("cuMemGetInfo=%d free=%zuMB tot=%zuMB\n",(int)r,fr>>20,to>>20);} }
  return 0;
}
