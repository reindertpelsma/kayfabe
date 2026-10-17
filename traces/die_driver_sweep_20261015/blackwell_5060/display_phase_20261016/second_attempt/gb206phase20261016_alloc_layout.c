#include <stddef.h>
#include <stdio.h>
#include "nvtypes.h"
#include "nvos.h"
#include "class/clc67a.h"
#include "class/clc67b.h"
#include "class/clc67d.h"
#include "class/clc67e.h"
#include "class/clca7a.h"
#include "class/clca7b.h"
#include "class/clca7d.h"
#include "class/clca7e.h"
#define FIELD(T, M) printf("\"%s\":[%zu,%zu]", #M, offsetof(T,M), sizeof(((T*)0)->M))
int main(void) {
 printf("{\"pio\":{\"size\":%zu,\"classes\":[%u,%u],\"fields\":{",
        sizeof(NV50VAIO_CHANNELPIO_ALLOCATION_PARAMETERS),
        NVC67A_CURSOR_IMM_CHANNEL_PIO, NVCA7A_CURSOR_IMM_CHANNEL_PIO);
 FIELD(NV50VAIO_CHANNELPIO_ALLOCATION_PARAMETERS, channelInstance);puts(",");
 FIELD(NV50VAIO_CHANNELPIO_ALLOCATION_PARAMETERS, hObjectNotify);puts(",");
 FIELD(NV50VAIO_CHANNELPIO_ALLOCATION_PARAMETERS, pControl);puts("}},");
 printf("\"dma\":{\"size\":%zu,\"classes\":[%u,%u,%u,%u,%u,%u],\"fields\":{",
        sizeof(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS),
        NVC67B_WINDOW_IMM_CHANNEL_DMA,NVC67D_CORE_CHANNEL_DMA,NVC67E_WINDOW_CHANNEL_DMA,
        NVCA7B_WINDOW_IMM_CHANNEL_DMA,NVCA7D_CORE_CHANNEL_DMA,NVCA7E_WINDOW_CHANNEL_DMA);
 FIELD(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS, channelInstance);puts(",");
 FIELD(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS, hObjectBuffer);puts(",");
 FIELD(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS, hObjectNotify);puts(",");
 FIELD(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS, offset);puts(",");
 FIELD(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS, pControl);puts(",");
 FIELD(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS, flags);puts(",");
 FIELD(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS, channelPBSize);puts(",");
 FIELD(NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS, subDeviceId);puts("}}}");
 return 0;
}
