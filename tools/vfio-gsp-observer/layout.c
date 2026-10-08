/* SPDX-License-Identifier: GPL-2.0-or-later */
#include <stddef.h>
#include <stdio.h>
#include "nvtypes.h"
#include "libos_init_args.h"
#include "gsp_init_args.h"
#include "dev_gsp.h"
#include "dev_falcon_v4.h"
#define RANGE_BASE(range) (0 ? range)
int main(void) {
    printf("{\"libos_stride\":%zu,\"libos_pa\":%zu,\"libos_size\":%zu,"
           "\"libos_kind\":%zu,\"libos_location\":%zu,\"sysmem\":%u,\"contiguous\":%u,"
           "\"mq_bytes\":%zu,\"mq_pa\":%zu,\"mq_pages\":%zu,\"mq_cmd\":%zu,\"mq_stat\":%zu,"
           "\"mailbox0\":%u,\"mailbox1\":%u,\"doorbell0\":%u,\"doorbell_stride\":%u,\"doorbells\":%u,"
           "\"irq_clear\":%u,\"irq_status\":%u}\n",
           sizeof(LibosMemoryRegionInitArgument),offsetof(LibosMemoryRegionInitArgument,pa),
           offsetof(LibosMemoryRegionInitArgument,size),offsetof(LibosMemoryRegionInitArgument,kind),
           offsetof(LibosMemoryRegionInitArgument,loc),LIBOS_MEMORY_REGION_LOC_SYSMEM,
           LIBOS_MEMORY_REGION_CONTIGUOUS,sizeof(MESSAGE_QUEUE_INIT_ARGUMENTS),
           offsetof(MESSAGE_QUEUE_INIT_ARGUMENTS,sharedMemPhysAddr),
           offsetof(MESSAGE_QUEUE_INIT_ARGUMENTS,pageTableEntryCount),
           offsetof(MESSAGE_QUEUE_INIT_ARGUMENTS,cmdQueueOffset),
           offsetof(MESSAGE_QUEUE_INIT_ARGUMENTS,statQueueOffset),
           NV_PGSP_FALCON_MAILBOX0,NV_PGSP_FALCON_MAILBOX1,NV_PGSP_QUEUE_HEAD(0),
           NV_PGSP_QUEUE_HEAD(1)-NV_PGSP_QUEUE_HEAD(0),NV_PGSP_QUEUE_HEAD__SIZE_1,
           RANGE_BASE(NV_PGSP)+NV_PFALCON_FALCON_IRQSCLR,
           RANGE_BASE(NV_PGSP)+NV_PFALCON_FALCON_IRQSTAT);
    return 0;
}
