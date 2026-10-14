/* Offline measurement decoder layout, compiled from public OGKM 580.65.06.
 * No captured value is an implementation constant. */
#include <stddef.h>
#include <stdio.h>
#include "alloc/alloc_channel.h"
#include "ctrl/ctrl5080.h"
#define FIELD(T, F) printf("\"" #F "\": %zu,\n", offsetof(T, F))
int main(void) {
    puts("{\"source\": \"NVIDIA/open-gpu-kernel-modules tag 580.65.06\",");
    puts("\"channel\": {");
    FIELD(NV_CHANNEL_ALLOC_PARAMS, gpFifoOffset);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, gpFifoEntries);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, flags);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, hContextShare);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, hVASpace);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, hUserdMemory);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, userdOffset);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, engineType);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, internalFlags);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, ProcessID);
    FIELD(NV_CHANNEL_ALLOC_PARAMS, SubProcessID);
    printf("\"size\": %zu},\n", sizeof(NV_CHANNEL_ALLOC_PARAMS));
    puts("\"registration\": {");
    FIELD(NV5080_CTRL_DEFERRED_API_V2_PARAMS, hApiHandle);
    FIELD(NV5080_CTRL_DEFERRED_API_V2_PARAMS, cmd);
    FIELD(NV5080_CTRL_DEFERRED_API_V2_PARAMS, api_bundle);
    printf("\"size\": %zu},\n", sizeof(NV5080_CTRL_DEFERRED_API_V2_PARAMS));
    puts("\"update\": {");
    FIELD(NV2080_CTRL_FIFO_UPDATE_CHANNEL_INFO_PARAMS, hClient);
    FIELD(NV2080_CTRL_FIFO_UPDATE_CHANNEL_INFO_PARAMS, hChannel);
    FIELD(NV2080_CTRL_FIFO_UPDATE_CHANNEL_INFO_PARAMS, hUserdMemory);
    FIELD(NV2080_CTRL_FIFO_UPDATE_CHANNEL_INFO_PARAMS, gpFifoEntries);
    FIELD(NV2080_CTRL_FIFO_UPDATE_CHANNEL_INFO_PARAMS, gpFifoOffset);
    FIELD(NV2080_CTRL_FIFO_UPDATE_CHANNEL_INFO_PARAMS, userdOffset);
    printf("\"size\": %zu}}\n", sizeof(NV2080_CTRL_FIFO_UPDATE_CHANNEL_INFO_PARAMS));
    return 0;
}
