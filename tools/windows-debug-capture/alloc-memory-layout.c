/* Compile public OGKM declarations; do not restate their layouts or bitfields. */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "nvtypes.h"
#include "nvmisc.h"
#include "nvos.h"
#include "vgpu/vgpu_version.h"
#include "vgpu/rpc.h"
#define SDK_ALL_CLASSES_INCLUDE_FULL_HEADER
#include "g_allclasses.h"
#undef SDK_ALL_CLASSES_INCLUDE_FULL_HEADER
#include "nverror.h"
#define RPC_STRUCTURES
#include "g_rpc-structures.h"
#undef RPC_STRUCTURES
#if __has_include("gpu/mem_mgr/rm_page_size.h")
#include "gpu/mem_mgr/rm_page_size.h"
#else
#include "gpu/mem_mgr/virt_mem_allocator_common.h"
#endif
#include "class/cl84a0.h"
/* Common pitch-kind HAL uses this published register definition. */
#include "published/maxwell/gm107/dev_mmu.h"

#define VALUE(name) printf("\"%s\":%llu,", #name, (unsigned long long)(name))
#define OFFSET(type, field) printf("\"%s\":%zu,", #field, offsetof(type, field))
#define FIELD(name) printf("\"%s\":{\"mask\":%llu,\"shift\":%u},", #name, \
    (unsigned long long)DRF_SHIFTMASK(NVOS02_FLAGS_##name), \
    (unsigned)DRF_SHIFT(NVOS02_FLAGS_##name))
#define BITS(field) do { \
    struct pte_desc d = {0}; uint32_t bits = 0; \
    volatile NvU32 all_bits = UINT32_MAX; \
    d.field = all_bits; memcpy(&bits, &d, sizeof(bits)); \
    printf("\"%s\":%u,", #field, bits); \
} while (0)

int main(void)
{
    printf("{\"rpc_alloc_memory\":{");
    OFFSET(rpc_alloc_memory_v, hClient);
    OFFSET(rpc_alloc_memory_v, hDevice);
    OFFSET(rpc_alloc_memory_v, hMemory);
    OFFSET(rpc_alloc_memory_v, hClass);
    OFFSET(rpc_alloc_memory_v, flags);
    OFFSET(rpc_alloc_memory_v, pteAdjust);
    OFFSET(rpc_alloc_memory_v, format);
    OFFSET(rpc_alloc_memory_v, length);
    OFFSET(rpc_alloc_memory_v, pageCount);
    OFFSET(rpc_alloc_memory_v, pteDesc);
    printf("\"size\":%zu},\"pte_desc\":{", sizeof(rpc_alloc_memory_v));
    OFFSET(struct pte_desc, pte_pde);
    BITS(idr); BITS(reserved1); BITS(length);
    printf("\"size\":%zu},\"constants\":{", sizeof(struct pte_desc));
    VALUE(RM_PAGE_SHIFT); VALUE(RM_PAGE_SIZE);
    VALUE(NV_VGPU_PTE_64_SIZE); VALUE(NV_VGPU_PTEDESC_IDR_NONE);
    VALUE(NV_VGPU_PTEDESC_IDR_SINGLE); VALUE(NV_VGPU_PTEDESC_IDR_DOUBLE);
    VALUE(NV_VGPU_PTEDESC_IDR_TRIPLE); VALUE(NV_VGPU_MSG_FUNCTION_ALLOC_MEMORY);
    VALUE(NV01_MEMORY_LIST_SYSTEM); VALUE(NV01_MEMORY_LIST_FBMEM);
    VALUE(NV_MMU_PTE_KIND_PITCH);
    VALUE(NVOS02_FLAGS_PHYSICALITY_CONTIGUOUS);
    VALUE(NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS);
    VALUE(NVOS02_FLAGS_LOCATION_PCI); VALUE(NVOS02_FLAGS_LOCATION_VIDMEM);
    VALUE(NVOS02_FLAGS_COHERENCY_UNCACHED); VALUE(NVOS02_FLAGS_COHERENCY_CACHED);
    VALUE(NVOS02_FLAGS_COHERENCY_WRITE_COMBINE);
    VALUE(NVOS02_FLAGS_COHERENCY_WRITE_THROUGH);
    VALUE(NVOS02_FLAGS_COHERENCY_WRITE_PROTECT);
    VALUE(NVOS02_FLAGS_COHERENCY_WRITE_BACK);
    VALUE(NVOS02_FLAGS_ALLOC_NONE);
    VALUE(NVOS02_FLAGS_MAPPING_DEFAULT); VALUE(NVOS02_FLAGS_MAPPING_NO_MAP);
    VALUE(NVOS02_FLAGS_MAPPING_NEVER_MAP);
#ifdef NVOS02_FLAGS_REGISTER_MEMDESC_TO_PHYS_RM_TRUE
    VALUE(NVOS02_FLAGS_REGISTER_MEMDESC_TO_PHYS_RM_TRUE);
#endif
    printf("\"end\":0},\"flags\":{");
    FIELD(PHYSICALITY); FIELD(LOCATION); FIELD(COHERENCY); FIELD(ALLOC);
    FIELD(GPU_CACHEABLE); FIELD(KERNEL_MAPPING); FIELD(ALLOC_NISO_DISPLAY);
    FIELD(ALLOC_USER_READ_ONLY); FIELD(ALLOC_DEVICE_READ_ONLY);
    FIELD(PEER_MAP_OVERRIDE); FIELD(ALLOC_TYPE_SYNCPOINT);
#ifdef NVOS02_FLAGS_MEMORY_PROTECTION
    FIELD(MEMORY_PROTECTION);
#endif
#ifdef NVOS02_FLAGS_REGISTER_MEMDESC_TO_PHYS_RM
    FIELD(REGISTER_MEMDESC_TO_PHYS_RM);
#endif
    FIELD(MAPPING);
    printf("\"end\":0}}\n");
    return 0;
}
