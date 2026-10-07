#include <stddef.h>
#include <stdio.h>
#include "nvos.h"
int main(void) {
    printf("%zu %zu %zu %zu\n", sizeof(NV_SWRUNLIST_ALLOCATION_PARAMS),
           offsetof(NV_SWRUNLIST_ALLOCATION_PARAMS, engineId),
           offsetof(NV_SWRUNLIST_ALLOCATION_PARAMS, maxTSGs),
           offsetof(NV_SWRUNLIST_ALLOCATION_PARAMS, qosIntrEnableMask));
}
