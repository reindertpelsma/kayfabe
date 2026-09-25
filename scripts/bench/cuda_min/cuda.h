/* ★ A MINIMAL <cuda.h> — only what cup2.c / cup3.c / cup8.c use, nothing else.
 *
 * Why it exists: the bench box has libcuda (the driver's userspace) but NO CUDA toolkit, so
 * there is no <cuda.h> to compile the ladder against, and the thin guest has no compiler at
 * all. The ladder is therefore built ONCE, on the host, against this header, and the SAME
 * binaries run in both arms (host bare metal and the kf3 guest). That is what makes the
 * guest/host ratio a ratio: one source, one compiler invocation, one binary, two platforms.
 * (`cup8bench.c` reached the same conclusion and declares its entry points inline.)
 *
 * The CUDA Driver API is a stable C ABI. The `_v2` spellings are the symbols the real header
 * macro-maps these names to; libcuda exports all of them. Values are from the real header
 * (CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR = 75, _MINOR = 76).
 *
 *   build: gcc -O2 -I scripts/bench/cuda_min -o cup3 cup3.c -lcuda
 */
#ifndef KF_CUDA_MIN_H
#define KF_CUDA_MIN_H
#include <stddef.h>

typedef int CUresult;
#define CUDA_SUCCESS 0
typedef int CUdevice;
typedef unsigned long long CUdeviceptr;
typedef struct CUctx_st *CUcontext;
typedef struct CUmod_st *CUmodule;
typedef struct CUfunc_st *CUfunction;
typedef struct CUstream_st *CUstream;
typedef int CUdevice_attribute;
#define CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR 75
#define CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR 76

#define cuCtxCreate     cuCtxCreate_v2
#define cuDeviceTotalMem cuDeviceTotalMem_v2
#define cuMemAlloc      cuMemAlloc_v2
#define cuMemcpyHtoD    cuMemcpyHtoD_v2
#define cuMemcpyDtoH    cuMemcpyDtoH_v2
#define cuMemsetD32     cuMemsetD32_v2

CUresult cuInit(unsigned int flags);
CUresult cuGetErrorString(CUresult r, const char **s);
CUresult cuDeviceGetCount(int *n);
CUresult cuDeviceGet(CUdevice *d, int ordinal);
CUresult cuDeviceGetName(char *name, int len, CUdevice d);
CUresult cuDeviceGetAttribute(int *v, CUdevice_attribute a, CUdevice d);
CUresult cuDeviceTotalMem_v2(size_t *bytes, CUdevice d);
CUresult cuCtxCreate_v2(CUcontext *ctx, unsigned int flags, CUdevice d);
CUresult cuCtxSynchronize(void);
CUresult cuModuleLoadData(CUmodule *m, const void *image);
CUresult cuModuleGetFunction(CUfunction *f, CUmodule m, const char *name);
CUresult cuMemAlloc_v2(CUdeviceptr *p, size_t bytes);
CUresult cuMemcpyHtoD_v2(CUdeviceptr dst, const void *src, size_t bytes);
CUresult cuMemcpyDtoH_v2(void *dst, CUdeviceptr src, size_t bytes);
CUresult cuMemsetD32_v2(CUdeviceptr dst, unsigned int v, size_t n);
CUresult cuLaunchKernel(CUfunction f, unsigned gx, unsigned gy, unsigned gz,
                        unsigned bx, unsigned by, unsigned bz, unsigned shmem,
                        CUstream s, void **params, void **extra);
#endif
