#include <stddef.h>
#include <stdio.h>
#include "nvtypes.h"
#include "vgpu/vgpu_version.h"
#include "vgpu/rpc.h"
#define SDK_ALL_CLASSES_INCLUDE_FULL_HEADER
#include "g_allclasses.h"
#undef SDK_ALL_CLASSES_INCLUDE_FULL_HEADER
#include "nverror.h"
#define RPC_STRUCTURES
#define RPC_GENERIC_UNION
#include "g_rpc-structures.h"

#define FIELD(member) printf("%s %zu %zu\n", #member, \
    offsetof(rpc_alloc_memory_v13_01, member), \
    sizeof(((rpc_alloc_memory_v13_01 *)0)->member))
int main(void) {
    printf("size %zu\n", sizeof(rpc_alloc_memory_v13_01));
    FIELD(hClient); FIELD(hDevice); FIELD(hMemory); FIELD(hClass);
    FIELD(flags); FIELD(pteAdjust); FIELD(format); FIELD(length); FIELD(pageCount);
}
