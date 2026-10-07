/* nvioctl_snoop.c — LD_PRELOAD shim to capture the NVIDIA frontend allocs that
 * matter for Mode-2 os-event delivery, straight from libcuda's ioctls (no guest
 * kernel rebuild).  Decodes:
 *   NV_ESC_RM_ALLOC (NR 0x2B): NVOS21/64 {hRoot@0, hObjectParent@4, hObjectNew@8,
 *       hClass@12} — prints when hClass==NV01_EVENT_OS_EVENT(0x0079) or any
 *       event class (0x0005/0x0079/0x007e) so we learn libcuda's real
 *       (hClient,hEvent) for the blocking-sync wait.
 *   NV_ESC_ALLOC_OS_EVENT (NR 206): {hClient@0, hDevice@4, fd@8} — the os-event
 *       fd libcuda's poll() blocks on.
 *   NV_ESC_RM_CONTROL (NR 0x2A): NVOS54 {hClient@0, hObject@4, cmd@8} — prints
 *       EVENT_SET_NOTIFICATION (0x20800301) to see which notifier is armed.
 * Build: gcc -shared -fPIC -O2 -o nvioctl_snoop.so nvioctl_snoop.c -ldl */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <stdarg.h>
#include <poll.h>
#include <dlfcn.h>
#include <sys/ioctl.h>

#ifndef _IOC_TYPE
#include <asm/ioctl.h>
#endif

static int (*real_ioctl)(int, unsigned long, ...);
static int (*real_poll)(struct pollfd *, nfds_t, int);

/* track os-event fds returned by NV_ESC_ALLOC_OS_EVENT */
static int osevent_fds[64]; static int osevent_n;
static int is_osevent_fd(int fd){ for(int i=0;i<osevent_n;i++) if(osevent_fds[i]==fd) return 1; return 0; }

int poll(struct pollfd *fds, nfds_t nfds, int timeout)
{
    if (!real_poll) real_poll = dlsym(RTLD_NEXT, "poll");
    /* log blocking polls that wait on a nvidia os-event fd */
    if (fds && (timeout < 0 || timeout > 50)) {
        for (nfds_t i = 0; i < nfds; i++) {
            if (is_osevent_fd(fds[i].fd)) {
                char buf[256]; int o = 0;
                o += snprintf(buf+o, sizeof(buf)-o, "[SNOOP] POLL(timeout=%d) os-event fds:", timeout);
                for (nfds_t j = 0; j < nfds && o < 200; j++)
                    o += snprintf(buf+o, sizeof(buf)-o, " fd%d%s", fds[j].fd,
                                  is_osevent_fd(fds[j].fd) ? "*" : "");
                fprintf(stderr, "%s\n", buf); fflush(stderr);
                break;
            }
        }
    }
    return real_poll(fds, nfds, timeout);
}

int ioctl(int fd, unsigned long req, ...)
{
    va_list ap; void *arg;
    va_start(ap, req); arg = va_arg(ap, void *); va_end(ap);
    if (!real_ioctl) real_ioctl = dlsym(RTLD_NEXT, "ioctl");

    unsigned type = _IOC_TYPE(req), nr = _IOC_NR(req), sz = _IOC_SIZE(req);
    int is_evt_alloc = 0; uint32_t e_hClass = 0, e_hRoot = 0, e_hNew = 0;
    void *darg = arg;  /* buffer to DECODE (real call always uses the original arg) */
    /* NV_ESC_IOCTL_XFER_CMD (211): {cmd@0, size@4, ptr@8} wraps an inner ioctl.
     * Unwrap so RM_ALLOC/ALLOC_OS_EVENT issued via XFER are still decoded. */
    if (type == 'F' && nr == 211 && arg) {
        uint32_t inner_cmd = ((uint32_t *)arg)[0];
        uint32_t inner_sz  = ((uint32_t *)arg)[1];
        uint64_t iptr = *(uint64_t *)((char *)arg + 8);
        if (iptr) { nr = inner_cmd; darg = (void *)(uintptr_t)iptr; sz = inner_sz; }
    }
    if (type == 'F' && darg) {
        uint32_t *p = (uint32_t *)darg;
        if (nr == 0x2B) {                 /* NV_ESC_RM_ALLOC (NVOS21/64) */
            uint32_t hRoot = p[0], hParent = p[1], hNew = p[2], hClass = p[3];
            if (hClass == 0x0079u || hClass == 0x0005u || hClass == 0x007eu) {
                /* pAllocParms @16 (NvP64) -> NV0005_ALLOC_PARAMETERS
                 * {hParentClient@0, hSrcResource@4, hClass@8, notifyIndex@12, data@16(NvP64)} */
                uint64_t pAllocParms = *(uint64_t *)((char *)darg + 16);
                uint32_t notifyIdx = 0; uint64_t data = 0;
                if (pAllocParms) {
                    uint32_t *ap = (uint32_t *)(uintptr_t)pAllocParms;
                    notifyIdx = ap[3];
                    data = *(uint64_t *)((char *)ap + 16);
                }
                fprintf(stderr, "[SNOOP] RM_ALLOC EVENT hClass=0x%04x hRoot(client)=0x%08x "
                        "hParent=0x%08x event=0x%08x notifyIdx=0x%x data(osevent)=0x%llx\n",
                        hClass, hRoot, hParent, hNew, notifyIdx, (unsigned long long)data);
                is_evt_alloc = 1; e_hClass = hClass; e_hRoot = hRoot; e_hNew = hNew;
            }
        } else if (nr == 206) {           /* NV_ESC_ALLOC_OS_EVENT */
            fprintf(stderr, "[SNOOP] ALLOC_OS_EVENT hClient=0x%08x hDevice=0x%08x fd=%d\n",
                    p[0], p[1], (int)p[2]);
            if (osevent_n < 64) osevent_fds[osevent_n++] = (int)p[2];
        } else if (nr == 0x29) {          /* NV_ESC_RM_FREE (NVOS00) */
            fprintf(stderr, "[SNOOP] RM_FREE hRoot=0x%08x hParent=0x%08x hObjectOld=0x%08x\n",
                    p[0], p[1], p[2]);
        } else if (nr == 0x2A) {          /* NV_ESC_RM_CONTROL (NVOS54) */
            uint32_t hClient = p[0], hObject = p[1], cmd = p[2];
            if (cmd == 0x20800301u || cmd == 0x20800302u || cmd == 0x730190u) {
                fprintf(stderr, "[SNOOP] RM_CONTROL cmd=0x%08x hClient=0x%08x hObject=0x%08x\n",
                        cmd, hClient, hObject);
            }
        }
        fflush(stderr);
    }
    int rc = real_ioctl(fd, req, arg);
    if (is_evt_alloc) {                   /* status: NVOS64(48)@40, NVOS21(32)@28 */
        uint32_t *p = (uint32_t *)darg;
        uint32_t status = (sz >= 48) ? p[10] : p[7];
        fprintf(stderr, "[SNOOP]  -> EVENT alloc 0x%08x class=0x%04x status=0x%x rc=%d\n",
                e_hNew, e_hClass, status, rc);
        fflush(stderr);
    }
    return rc;
}
