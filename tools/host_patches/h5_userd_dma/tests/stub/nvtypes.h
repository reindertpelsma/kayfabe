/* Test stub: just the NVIDIA basic types the H5 headers use. Not the real nvtypes.h. */
#ifndef KF_TEST_NVTYPES_H
#define KF_TEST_NVTYPES_H
#include <stdint.h>
typedef uint8_t  NvU8;
typedef uint32_t NvU32;
typedef uint64_t NvU64;
typedef uintptr_t NvUPtr;
#define NV_ALIGN_BYTES(n) __attribute__((aligned(n)))
#endif
