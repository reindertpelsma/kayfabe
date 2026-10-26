#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include "nvtypes.h"
#include "nvos.h"
int main(void) {
    int fd = open("/dev/null", O_RDONLY);
    NV_MEMORY_ALLOCATION_PARAMS memory = {0};
    memory.type = 2; memory.flags = 0x1234;
    memory.attr = (2u << 12) | (2u << 23); memory.attr2 = 1u << 20;
    memory.size = 1048576; memory.alignment = 65536; memory.ctagOffset = 17;
    NVOS21_PARAMETERS p = {.hClass = 0x40, .pAllocParms = (NvP64)(uintptr_t)&memory,
                          .paramsSize = sizeof memory};
    int rc = ioctl(fd, _IOWR('F', 0x2b, NVOS21_PARAMETERS), &p);
    printf("fixture rc=%d errno=%d\n", rc, errno);
    close(fd);
    return rc == -1 && errno == ENOTTY ? 0 : 1;
}
