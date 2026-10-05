/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Native Linux oracle: allocate/free 28 KiB of ordinary contiguous system memory,
 * optionally requesting GSP registration. Compile against the running driver's
 * pinned public headers. Never takes a physical address or privileged list class.
 * Run without privileges; an external observer may trace the named RPC wrapper.
 */
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include "nvtypes.h"
#include "nvmisc.h"
#include "nvos.h"
#include "nv-ioctl.h"
#include "nv_escape.h"
#include "class/cl003e.h"
#include "class/cl0080.h"

static int alloc(int fd, NvHandle client, NvHandle parent, NvHandle *handle,
                 NvU32 class_id, void *params, NvU32 size)
{
    NVOS21_PARAMETERS request = {0};
    request.hRoot = client;
    request.hObjectParent = parent;
    request.hObjectNew = *handle;
    request.hClass = class_id;
    request.pAllocParms = NV_PTR_TO_NvP64(params);
    request.paramsSize = size;
    if (ioctl(fd, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC, NVOS21_PARAMETERS),
              &request) < 0) {
        perror("RM_ALLOC");
        return -1;
    }
    printf("ALLOC class=%#x bytes=%u status=%#x\n", class_id, size, request.status);
    *handle = request.hObjectNew;
    return request.status == 0 ? 0 : -1;
}

static int release(int fd, NvHandle client, NvHandle parent, NvHandle handle)
{
    NVOS00_PARAMETERS request = {0};
    request.hRoot = client;
    request.hObjectParent = parent;
    request.hObjectOld = handle;
    if (ioctl(fd, _IOWR(NV_IOCTL_MAGIC, NV_ESC_RM_FREE, NVOS00_PARAMETERS),
              &request) < 0) {
        perror("RM_FREE");
        return -1;
    }
    printf("FREE status=%#x\n", request.status);
    return request.status == 0 ? 0 : -1;
}

int main(int argc, char **argv)
{
    if (argc != 2 || (strcmp(argv[1], "plain") && strcmp(argv[1], "registered"))) {
        fprintf(stderr, "usage: linux-memory-registration plain|registered\n");
        return 2;
    }
    if (getuid() == 0 || geteuid() == 0) {
        fprintf(stderr, "run as an unprivileged user with all capabilities dropped\n");
        return 2;
    }
    setvbuf(stdout, NULL, _IONBF, 0);
    const int registered = strcmp(argv[1], "registered") == 0;
    printf("MEMORY_REGISTRATION pid=%ld uid=%ld euid=%ld mode=%s\n",
           (long)getpid(), (long)getuid(), (long)geteuid(), argv[1]);
    FILE *status = fopen("/proc/self/status", "r");
    if (!status) { perror("proc status"); return 1; }
    char line[256];
    while (fgets(line, sizeof(line), status))
        if (!strncmp(line, "Cap", 3) || !strncmp(line, "NoNewPrivs:", 11))
            fputs(line, stdout);
    fclose(status);
    int ctl = open("/dev/nvidiactl", O_RDWR | O_CLOEXEC);
    int gpu = open("/dev/nvidia0", O_RDWR | O_CLOEXEC);
    if (ctl < 0 || gpu < 0) { perror("open NVIDIA device"); return 1; }
    int result = 1;
    NvHandle client = 0, device = 0xcaf00001, memory = 0xcaf00002;
    nv_ioctl_rm_api_version_t version = {.cmd = NV_RM_API_VERSION_CMD_QUERY};
    if (ioctl(ctl, _IOWR(NV_IOCTL_MAGIC, NV_ESC_CHECK_VERSION_STR,
                        nv_ioctl_rm_api_version_t), &version) < 0) {
        perror("CHECK_VERSION"); goto done;
    }
    printf("DRIVER %.*s\n", (int)sizeof(version.versionString), version.versionString);
    nv_ioctl_register_fd_t fd_registration = {.ctl_fd = ctl};
    if (ioctl(gpu, _IOWR(NV_IOCTL_MAGIC, NV_ESC_REGISTER_FD,
                        nv_ioctl_register_fd_t), &fd_registration) < 0) {
        perror("REGISTER_FD"); goto done;
    }
    if (alloc(ctl, 0, 0, &client, NV01_ROOT_CLIENT, NULL, 0)) goto done;
    NV0080_ALLOC_PARAMETERS device_params = {0};
    if (alloc(ctl, client, client, &device, NV01_DEVICE_0,
              &device_params, sizeof(device_params))) goto free_client;
    NV_MEMORY_ALLOCATION_PARAMS params = {0};
    params.type = NVOS32_TYPE_IMAGE;
    params.size = 0x7000;
    params.attr = DRF_DEF(OS32, _ATTR, _LOCATION, _PCI) |
                  DRF_DEF(OS32, _ATTR, _PHYSICALITY, _CONTIGUOUS) |
                  DRF_DEF(OS32, _ATTR, _COHERENCY, _WRITE_COMBINE) |
                  DRF_DEF(OS32, _ATTR, _PAGE_SIZE, _4KB);
    if (registered)
        params.attr2 = DRF_DEF(OS32, _ATTR2, _REGISTER_MEMDESC_TO_PHYS_RM, _TRUE);
    printf("MEMORY_REQUEST class=%#x bytes=%llu attr=%#x attr2=%#x\n",
           NV01_MEMORY_SYSTEM, (unsigned long long)params.size, params.attr, params.attr2);
    if (alloc(ctl, client, device, &memory, NV01_MEMORY_SYSTEM,
              &params, sizeof(params))) goto free_client;
    printf("MEMORY_RESULT bytes=%llu attr=%#x attr2=%#x\n",
           (unsigned long long)params.size, params.attr, params.attr2);
    result = release(ctl, client, device, memory) ? 1 : 0;
free_client:
    if (release(ctl, client, 0, client)) result = 1;
done:
    close(gpu);
    close(ctl);
    printf("PROBE_EXIT=%d\n", result);
    return result;
}
