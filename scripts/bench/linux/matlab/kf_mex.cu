#include "mex.h"
#include "gpu/mxGPUArray.h"
#include <string.h>

/* kf_mex('hist', uint8 gpuArray) -> uint32 256-bin histogram (shared-memory bins, global atomics);
   kf_mex('sum', double gpuArray) -> scalar sum (shared-memory tree reduction, one atomicAdd of a double per block). */

__global__ void kf_hist(const unsigned char *in, unsigned int *bins, int n)
{
    __shared__ unsigned int sh[256];
    for (int i = threadIdx.x; i < 256; i += blockDim.x) sh[i] = 0u;
    __syncthreads();
    for (int i = blockIdx.x * blockDim.x + threadIdx.x; i < n; i += gridDim.x * blockDim.x)
        atomicAdd(&sh[in[i]], 1u);
    __syncthreads();
    for (int i = threadIdx.x; i < 256; i += blockDim.x) atomicAdd(&bins[i], sh[i]);
}

__global__ void kf_sum(const double *in, double *out, int n)
{
    __shared__ double sh[256];
    double acc = 0.0;
    for (int i = blockIdx.x * blockDim.x + threadIdx.x; i < n; i += gridDim.x * blockDim.x) acc += in[i];
    sh[threadIdx.x] = acc;
    __syncthreads();
    for (int s = blockDim.x / 2; s > 0; s >>= 1) {
        if (threadIdx.x < s) sh[threadIdx.x] += sh[threadIdx.x + s];
        __syncthreads();
    }
    if (threadIdx.x == 0) atomicAdd(out, sh[0]);
}

void mexFunction(int nlhs, mxArray *plhs[], int nrhs, mxArray const *prhs[])
{
    mxInitGPU();
    if (nrhs != 2 || !mxIsChar(prhs[0])) mexErrMsgIdAndTxt("kf:mex:args", "two arguments: an operation name and a gpuArray");
    char op[8];
    mxGetString(prhs[0], op, sizeof(op));
    mxGPUArray const *in = mxGPUCreateFromMxArray(prhs[1]);
    int n = (int)mxGPUGetNumberOfElements(in);
    const int threads = 256;
    int blocks = (n + threads - 1) / threads;
    if (blocks > 1024) blocks = 1024;
    if (blocks < 1) blocks = 1;
    mxGPUArray *out = NULL;
    if (strcmp(op, "hist") == 0 && mxGPUGetClassID(in) == mxUINT8_CLASS) {
        mwSize dims[2] = {256, 1};
        out = mxGPUCreateGPUArray(2, dims, mxUINT32_CLASS, mxREAL, MX_GPU_INITIALIZE_VALUES);
        kf_hist<<<blocks, threads>>>((const unsigned char *)mxGPUGetDataReadOnly(in), (unsigned int *)mxGPUGetData(out), n);
    } else if (strcmp(op, "sum") == 0 && mxGPUGetClassID(in) == mxDOUBLE_CLASS) {
        mwSize dims[2] = {1, 1};
        out = mxGPUCreateGPUArray(2, dims, mxDOUBLE_CLASS, mxREAL, MX_GPU_INITIALIZE_VALUES);
        kf_sum<<<blocks, threads>>>((const double *)mxGPUGetDataReadOnly(in), (double *)mxGPUGetData(out), n);
    } else {
        mxGPUDestroyGPUArray(in);
        mexErrMsgIdAndTxt("kf:mex:op", "unknown operation or wrong class");
    }
    cudaError_t e = cudaGetLastError();
    if (e != cudaSuccess) {
        mxGPUDestroyGPUArray(out);
        mxGPUDestroyGPUArray(in);
        mexErrMsgIdAndTxt("kf:mex:launch", "kernel launch failed");
    }
    plhs[0] = mxGPUCreateMxArrayOnGPU(out);
    mxGPUDestroyGPUArray(out);
    mxGPUDestroyGPUArray(in);
}
