// CODE READ: public ogkm 580.65.06, commit 307159f2623d3bf45feb9177bd2da52ffbc5ddf9.
// Compile against that tag's src/common/sdk/nvidia/inc. No firmware is read.
#include <stdio.h>
#include <stddef.h>
#include "class/cl00de.h"
int main(void) {
    printf("sizeof=%zu bar1=%zu pma=%zu perf=%zu\n", sizeof(NV00DE_SHARED_DATA),
        offsetof(NV00DE_SHARED_DATA, bar1MemoryInfo),
        offsetof(NV00DE_SHARED_DATA, pmaMemoryInfo),
        offsetof(NV00DE_SHARED_DATA, perfDevUtil));
    return 0;
}
