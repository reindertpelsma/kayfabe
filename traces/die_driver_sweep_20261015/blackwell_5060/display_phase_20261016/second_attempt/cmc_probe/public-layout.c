#include <stddef.h>
#include <stdio.h>
#include "ctrl/ctrl2080/ctrl2080internal.h"
int main(void) {
 printf("cmd=%#x size=%zu PublicId=%zu logBufferSize=%zu logBufferAddr=%zu\n", NV2080_CTRL_CMD_INTERNAL_SEND_CMC_LIBOS_BUFFER_INFO, sizeof(NV2080_CTRL_INTERNAL_SEND_CMC_LIBOS_BUFFER_INFO_PARAMS), offsetof(NV2080_CTRL_INTERNAL_SEND_CMC_LIBOS_BUFFER_INFO_PARAMS, PublicId), offsetof(NV2080_CTRL_INTERNAL_SEND_CMC_LIBOS_BUFFER_INFO_PARAMS, logBufferSize), offsetof(NV2080_CTRL_INTERNAL_SEND_CMC_LIBOS_BUFFER_INFO_PARAMS, logBufferAddr));
}
