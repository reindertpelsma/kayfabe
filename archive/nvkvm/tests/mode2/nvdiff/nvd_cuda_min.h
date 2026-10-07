/*
 * nvd_cuda_min.h — a MINIMAL stand-in for <cuda.h>, for hosts that have libcuda
 * but no CUDA toolkit.
 *
 * ## Why this exists
 *
 * `nvd_prog.c` needs `cuda.h` only for prototypes and two enum values. The bench box
 * `vh` ships `libcuda.so.580.159.04` and **no toolkit**, and the only `cuda.h` on the
 * filesystem is the PowerMac ADB driver header — the exact decoy
 * `cup2_hook_deadline.sh` warns about ("3 of 5 hits in this guest are the PowerMac ADB").
 * Without this file the differential cannot be captured on the box whose GPU and driver
 * actually match the guest's.
 *
 * ## ⚠⚠ THE ONLY THING THAT CAN GO WRONG HERE, AND IT IS SILENT
 *
 * Real `cuda.h` does not declare `cuCtxCreate`; it declares `cuCtxCreate_v2` and
 * `#define`s the short name onto it. **Seven of the entry points this workload uses are
 * versioned that way.** A hand-written header that declares the SHORT names binds the
 * v1 symbols, which are different functions with different ABIs — the program still
 * builds, still links, still runs, and emits a DIFFERENT ioctl stream. A differential
 * built on that is comparing two things neither of which is what CUDA does.
 *
 * ⇒ The `#define`s below are the load-bearing half of this file, not the prototypes.
 * ⇒ And they are not trusted: `nvd_capture.sh` greps the built binary's dynamic
 *   relocations and REFUSES to run unless every one of the seven `_v2` names is bound.
 *   A header cannot check itself; the linker's output can.
 *
 * Versioning read off the CUDA driver API (unchanged since CUDA 3.2):
 *   cuDeviceTotalMem, cuCtxCreate, cuCtxDestroy, cuMemAlloc, cuMemFree,
 *   cuMemcpyHtoD, cuMemcpyDtoH   -> _v2
 *   cuInit, cuGetErrorString, cuDeviceGetCount, cuDeviceGet, cuDeviceGetName,
 *   cuDeviceGetAttribute, cuModuleLoadData, cuModuleGetFunction, cuLaunchKernel,
 *   cuCtxSynchronize             -> unversioned
 */
#ifndef NVD_CUDA_MIN_H
#define NVD_CUDA_MIN_H

#include <stddef.h>

typedef int                 CUresult;
typedef int                 CUdevice;
typedef unsigned long long  CUdeviceptr;
typedef struct CUctx_st    *CUcontext;
typedef struct CUmod_st    *CUmodule;
typedef struct CUfunc_st   *CUfunction;
typedef struct CUstream_st *CUstream;

#define CUDA_SUCCESS 0
#define CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR 75
#define CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR 76

/* ★ the versioned seven — see the header comment; getting these wrong is silent */
#define cuDeviceTotalMem cuDeviceTotalMem_v2
#define cuCtxCreate      cuCtxCreate_v2
#define cuCtxDestroy     cuCtxDestroy_v2
#define cuMemAlloc       cuMemAlloc_v2
#define cuMemFree        cuMemFree_v2
#define cuMemcpyHtoD     cuMemcpyHtoD_v2
#define cuMemcpyDtoH     cuMemcpyDtoH_v2
/* ★ an EIGHTH, added for the fault stages: cuMemcpyDtoD is versioned too. The other
 * fault-stage entry points (cuCtxSetCurrent, cuMemAddressReserve/Free) are NOT — they
 * postdate the CUDA 3.2 renaming and carry no _v2 alias. Checked against libcuda's
 * dynamic symbol table on the bench, not from memory. */
#define cuMemcpyDtoD     cuMemcpyDtoD_v2

#ifdef __cplusplus
extern "C" {
#endif

CUresult cuInit(unsigned int);
CUresult cuGetErrorString(CUresult, const char **);
CUresult cuDeviceGetCount(int *);
CUresult cuDeviceGet(CUdevice *, int);
CUresult cuDeviceGetName(char *, int, CUdevice);
CUresult cuDeviceGetAttribute(int *, int, CUdevice);
CUresult cuDeviceTotalMem(size_t *, CUdevice);
CUresult cuCtxCreate(CUcontext *, unsigned int, CUdevice);
CUresult cuCtxDestroy(CUcontext);
CUresult cuCtxSynchronize(void);
CUresult cuMemAlloc(CUdeviceptr *, size_t);
CUresult cuMemFree(CUdeviceptr);
CUresult cuMemcpyHtoD(CUdeviceptr, const void *, size_t);
CUresult cuMemcpyDtoH(void *, CUdeviceptr, size_t);
CUresult cuModuleLoadData(CUmodule *, const void *);
CUresult cuModuleGetFunction(CUfunction *, CUmodule, const char *);
CUresult cuLaunchKernel(CUfunction, unsigned, unsigned, unsigned,
                        unsigned, unsigned, unsigned, unsigned,
                        CUstream, void **, void **);
/* fault stages */
CUresult cuMemcpyDtoD(CUdeviceptr, CUdeviceptr, size_t);
CUresult cuCtxSetCurrent(CUcontext);
CUresult cuMemAddressReserve(CUdeviceptr *, size_t, size_t, CUdeviceptr,
                             unsigned long long);
CUresult cuMemAddressFree(CUdeviceptr, size_t);

/* ★ The virtual-memory-management (VMM) API. It is the ONLY way found to hand CUDA a
 * pointer it accepts as a device allocation while the GPU page tables hold no mapping
 * for it — i.e. an "invalid CE address" that survives libcuda's own bounds check.
 * ⚠ These structs are ABI, and getting them wrong is the silent-failure shape this
 * header already exists to warn about. Layout is the CUDA 11+ definition; every call's
 * return code is printed by the caller, so a mismatch shows up as a refusal at
 * cuMemCreate rather than as a wrong measurement. */
typedef unsigned long long CUmemGenericAllocationHandle;
#define CU_MEM_ALLOCATION_TYPE_PINNED      0x1
#define CU_MEM_HANDLE_TYPE_NONE            0x0
#define CU_MEM_LOCATION_TYPE_DEVICE        0x1
#define CU_MEM_ACCESS_FLAGS_PROT_READWRITE 0x3
#define CU_MEM_ALLOC_GRANULARITY_MINIMUM   0x0

typedef struct CUmemLocation_st { int type; int id; } CUmemLocation;
typedef struct CUmemAllocationProp_st {
    int          type;                  /* CUmemAllocationType       */
    int          requestedHandleTypes;  /* CUmemAllocationHandleType */
    CUmemLocation location;
    void        *win32HandleMetaData;
    struct {
        unsigned char  compressionType;
        unsigned char  gpuDirectRDMACapable;
        unsigned short usage;
        unsigned char  reserved[4];
    } allocFlags;
} CUmemAllocationProp;
typedef struct CUmemAccessDesc_st {
    CUmemLocation location;
    int           flags;                /* CUmemAccess_flags */
} CUmemAccessDesc;

CUresult cuMemGetAllocationGranularity(size_t *, const CUmemAllocationProp *, int);
CUresult cuMemCreate(CUmemGenericAllocationHandle *, size_t,
                     const CUmemAllocationProp *, unsigned long long);
CUresult cuMemMap(CUdeviceptr, size_t, size_t, CUmemGenericAllocationHandle,
                  unsigned long long);
CUresult cuMemSetAccess(CUdeviceptr, size_t, const CUmemAccessDesc *, size_t);
CUresult cuMemUnmap(CUdeviceptr, size_t);
CUresult cuMemRelease(CUmemGenericAllocationHandle);

#ifdef __cplusplus
}
#endif
#endif /* NVD_CUDA_MIN_H */
