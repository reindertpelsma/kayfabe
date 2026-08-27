/* Minimal CUDA driver-API probe for Mode-2 bring-up: cuInit -> device -> ctx ->
 * CE memcpy round-trip.  Prints where it stops.  Build: nvcc -o cup cup.c -lcuda */
#include <cuda.h>
#include <stdio.h>
#define CK(x) do{ CUresult r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} else { printf("ok   %s\n",#x); fflush(stdout);} }while(0)
int main(void){
    CK(cuInit(0));
    int n=0; CK(cuDeviceGetCount(&n)); printf("devices=%d\n",n); if(n<1) return 1;
    CUdevice d; CK(cuDeviceGet(&d,0));
    char nm[256]={0}; CK(cuDeviceGetName(nm,256,d)); printf("name=%s\n",nm);
    int maj=0,min=0;
    cuDeviceGetAttribute(&maj,CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR,d);
    cuDeviceGetAttribute(&min,CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR,d);
    printf("compute=%d.%d\n",maj,min);
    CUcontext ctx; CK(cuCtxCreate(&ctx,0,d));
    CUdeviceptr dp; CK(cuMemAlloc(&dp,4096));
    unsigned hv=0xabcd1234, rv=0;
    CK(cuMemcpyHtoD(dp,&hv,4));
    CK(cuMemcpyDtoH(&rv,dp,4));
    printf("CE-roundtrip rv=0x%x want=0x%x -> %s\n", rv, hv, rv==hv?"PASS":"MISMATCH");
    printf("DONE\n");
    return 0;
}
