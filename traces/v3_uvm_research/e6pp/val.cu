#include <cstdio>
#include <cuda_runtime.h>
extern "C" __global__ void k(int *out);
int main(){
    int *d=nullptr; int h=0;
    cudaError_t e;
    e=cudaMalloc(&d,sizeof(int)); if(e){printf("cudaMalloc %s\n",cudaGetErrorString(e));return 1;}
    e=cudaMemset(d,0,sizeof(int)); if(e){printf("memset %s\n",cudaGetErrorString(e));return 1;}
    k<<<1,1>>>(d);
    e=cudaGetLastError(); if(e){printf("launch %s\n",cudaGetErrorString(e));return 1;}
    e=cudaDeviceSynchronize(); if(e){printf("sync %s\n",cudaGetErrorString(e));return 1;}
    e=cudaMemcpy(&h,d,sizeof(int),cudaMemcpyDeviceToHost); if(e){printf("copy %s\n",cudaGetErrorString(e));return 1;}
    printf("VALIDATOR out=%d expected=7 %s\n",h,h==7?"PASS":"FAIL");
    return h==7?0:2;
}
