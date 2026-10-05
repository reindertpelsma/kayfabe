/* Research probe: compile the public declaration, never restate its layout. */
#include <stddef.h>
#include <stdio.h>
#include "nvos.h"

int main(void)
{
    printf("{\"NV_SWRUNLIST_ALLOCATION_PARAMS\": {\"size\": %zu, "
           "\"engineId\": %zu, \"maxTSGs\": %zu, \"qosIntrEnableMask\": %zu}}\n",
           sizeof(NV_SWRUNLIST_ALLOCATION_PARAMS),
           offsetof(NV_SWRUNLIST_ALLOCATION_PARAMS, engineId),
           offsetof(NV_SWRUNLIST_ALLOCATION_PARAMS, maxTSGs),
           offsetof(NV_SWRUNLIST_ALLOCATION_PARAMS, qosIntrEnableMask));
    return 0;
}
