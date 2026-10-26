/* Observe documented RM ioctl payloads; forward every actual ioctl unchanged. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <unistd.h>
#include "nvtypes.h"
#include "nvos.h"

static int logfd = -1;
static int read_self(void *out, const void *in, size_t size) {
    struct iovec dst = {.iov_base = out, .iov_len = size};
    struct iovec src = {.iov_base = (void *)in, .iov_len = size};
    return process_vm_readv(getpid(), &dst, 1, &src, 1, 0) == (ssize_t)size;
}
static void emit(const char *format, ...) {
    char buffer[8192];
    va_list args;
    va_start(args, format);
    int size = vsnprintf(buffer, sizeof buffer, format, args);
    va_end(args);
    int saved = errno;
    if (logfd >= 0 && size > 0 && size < (int)sizeof buffer) {
        ssize_t written = write(logfd, buffer, size);
        if (written != size) fputs("RM_TRACE_WRITE_FAIL\n", stderr);
    }
    errno = saved;
}
__attribute__((constructor)) static void start(void) {
    const char *path = getenv("LX2_RM_OUT");
    if (path) logfd = open(path, O_CREAT | O_EXCL | O_WRONLY | O_CLOEXEC, 0600);
}
static void memory(const char *when, NvU32 cls, const void *pointer, NvU32 size) {
    if (cls == 0xa06c && size == sizeof(NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS)) {
        NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS p;
        if (read_self(&p, pointer, sizeof p))
            emit("RM_TSG %s engine=%x va=%x plugin=%u\n", when,
                 p.engineType, p.hVASpace, p.bIsCallingContextVgpuPlugin);
    } else if (cls == 0x9067 && size == sizeof(NV_CTXSHARE_ALLOCATION_PARAMETERS)) {
        NV_CTXSHARE_ALLOCATION_PARAMETERS p;
        if (read_self(&p, pointer, sizeof p))
            emit("RM_CTXSHARE %s flags=%x va=%x subctx=%x\n", when,
                 p.flags, p.hVASpace, p.subctxId);
    } else if (cls == 0x90f1 && size == sizeof(NV_VASPACE_ALLOCATION_PARAMETERS)) {
        NV_VASPACE_ALLOCATION_PARAMETERS p;
        if (read_self(&p, pointer, sizeof p))
            emit("RM_VASPACE %s flags=%x index=%x size=%llu bigpage=%u base=%llu\n",
                 when, p.flags, p.index, (unsigned long long)p.vaSize,
                 p.bigPageSize, (unsigned long long)p.vaBase);
    } else if ((cls & 0xff) == 0x6f && size == sizeof(NV_CHANNEL_ALLOC_PARAMS)) {
        NV_CHANNEL_ALLOC_PARAMS p;
        if (read_self(&p, pointer, sizeof p))
            emit("RM_CHANNEL %s class=%x flags=%x ctxshare=%x va=%x engine=%x entries=%u\n",
                 when, cls, p.flags, p.hContextShare, p.hVASpace, p.engineType,
                 p.gpFifoEntries);
    }
    if ((cls != 0x40 && cls != 0x3e) || size != sizeof(NV_MEMORY_ALLOCATION_PARAMS)) return;
    NV_MEMORY_ALLOCATION_PARAMS p;
    if (!read_self(&p, pointer, sizeof p)) return;
    emit("RM_MEMORY %s pid=%d class=%x type=%x flags=%x attr=%x attr2=%x format=%x "
         "compression=%u page=%u huge=%u size=%llu alignment=%llu ctag=%u va=%x\n",
         when, getpid(), cls, p.type, p.flags, p.attr, p.attr2, p.format,
         (p.attr >> 12) & 3, (p.attr >> 23) & 3, (p.attr2 >> 20) & 3,
         (unsigned long long)p.size, (unsigned long long)p.alignment,
         p.ctagOffset, p.hVASpace);
}
static void inspect(const char *when, unsigned cmd, const void *pointer, size_t size) {
    emit("RM_IOCTL %s pid=%d tid=%ld command=%x size=%zu\n", when,
         getpid(), syscall(SYS_gettid), cmd, size);
    if (cmd == 0x2b && size == sizeof(NVOS21_PARAMETERS)) {
        NVOS21_PARAMETERS p;
        if (!read_self(&p, pointer, sizeof p)) return;
        emit("RM_ALLOC %s class=%x params=%u root=%x parent=%x object=%x status=%x\n",
             when, p.hClass, p.paramsSize, p.hRoot, p.hObjectParent, p.hObjectNew, p.status);
        memory(when, p.hClass, (const void *)(uintptr_t)p.pAllocParms, p.paramsSize);
    } else if (cmd == 0x2b && size == sizeof(NVOS64_PARAMETERS)) {
        NVOS64_PARAMETERS p;
        if (!read_self(&p, pointer, sizeof p)) return;
        emit("RM_ALLOC64 %s class=%x params=%u flags=%x root=%x parent=%x object=%x "
             "status=%x\n", when, p.hClass, p.paramsSize, p.flags, p.hRoot,
             p.hObjectParent, p.hObjectNew, p.status);
        if (p.flags == 0)
            memory(when, p.hClass, (const void *)(uintptr_t)p.pAllocParms, p.paramsSize);
        else emit("RM_UNDECODED serialized_class=%x flags=%x\n", p.hClass, p.flags);
    } else if (cmd == 0x4a && size == sizeof(NVOS32_PARAMETERS)) {
        NVOS32_PARAMETERS p;
        if (!read_self(&p, pointer, sizeof p)) return;
        emit("RM_HEAP %s function=%u root=%x parent=%x va=%x status=%x\n", when,
             p.function, p.hRoot, p.hObjectParent, p.hVASpace, p.status);
        if (p.function == NVOS32_FUNCTION_ALLOC_SIZE) {
            emit("RM_HEAP_ALLOC %s flags=%x attr=%x attr2=%x format=%x "
                 "compression=%u page=%u huge=%u size=%llu ctag=%u\n", when,
                 p.data.AllocSize.flags, p.data.AllocSize.attr,
                 p.data.AllocSize.attr2, p.data.AllocSize.format,
                 (p.data.AllocSize.attr >> 12) & 3,
                 (p.data.AllocSize.attr >> 23) & 3,
                 (p.data.AllocSize.attr2 >> 20) & 3,
                 (unsigned long long)p.data.AllocSize.size, p.data.AllocSize.ctagOffset);
        }
    } else if (cmd == 0x57 && size == sizeof(NVOS46_PARAMETERS)) {
        NVOS46_PARAMETERS p;
        if (read_self(&p, pointer, sizeof p))
            emit("RM_MAP %s flags=%x flags2=%x kind_request=%x offset=%llu len=%llu "
                 "dmaoffset=%llu status=%x\n", when, p.flags, p.flags2, p.kindOverride,
                 (unsigned long long)p.offset, (unsigned long long)p.length,
                 (unsigned long long)p.dmaOffset, p.status);
    } else if (cmd == 0x2a && size == sizeof(NVOS54_PARAMETERS)) {
        NVOS54_PARAMETERS p;
        if (read_self(&p, pointer, sizeof p))
            emit("RM_CONTROL %s command=%x params=%u status=%x\n",
                 when, p.cmd, p.paramsSize, p.status);
    }
}
int ioctl(int fd, unsigned long request, ...) {
    va_list args;
    va_start(args, request);
    void *argument = va_arg(args, void *);
    va_end(args);
    unsigned cmd = _IOC_NR(request);
    size_t size = _IOC_SIZE(request);
    const void *payload = argument;
    int observed = logfd >= 0 && _IOC_TYPE(request) == 'F';
    if (observed && cmd == 211) {
        struct { uint32_t command, size; uint64_t pointer; } transfer;
        if (read_self(&transfer, argument, sizeof transfer)) {
            cmd = transfer.command;
            size = transfer.size;
            payload = (const void *)(uintptr_t)transfer.pointer;
        } else observed = 0;
    }
    if (observed) inspect("in", cmd, payload, size);
    int result = syscall(SYS_ioctl, fd, request, argument);
    int saved = errno;
    if (observed) {
        inspect("out", cmd, payload, size);
        emit("RM_RETURN command=%x rc=%d errno=%d\n", cmd, result, result < 0 ? saved : 0);
    }
    errno = saved;
    return result;
}
