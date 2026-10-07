#!/usr/bin/env python3
"""Host RM privilege probe — which knob decides whether an RM client acts as admin?

Read-only with respect to the GPU: it allocates and frees RM root clients on
/dev/nvidiactl and issues one NON_PRIVILEGED control on each,
NV0000_CTRL_CMD_SYSTEM_GET_PRIVILEGED_STATUS (0x135, ogkm-580:
src/nvidia/src/kernel/rmapi/client_resource.c:4520-4551), which reports
  PRIV_USER_FLAG   (0x1) = this CALL runs at RS_PRIV_LEVEL_USER_ROOT
                           (escape.c:304: osIsAdministrator() = capable(CAP_SYS_ADMIN))
  PRIV_HANDLE_FLAG (0x4) = rmclientIsAdmin(client, privLevel)
                           (client.c:384-394: USER_ROOT && !bIsRootNonPriv)
rmclientIsAdmin is the exact predicate kernel_channel.c:283 uses to stamp a new
channel PRIVILEGE_ADMIN + NVOS04_FLAGS_PRIVILEGED_CHANNEL.

Arms:
  class   — root allocated as NV01_ROOT (0x0), NV01_ROOT_NON_PRIV (0x1), NV01_ROOT_CLIENT (0x41)
  capeff  — CAP_SYS_ADMIN in this thread's effective set: kept / cleared (capset, permitted kept)
"""
import ctypes
import fcntl
import os
import struct
import sys

NV_ESC_RM_FREE, NV_ESC_RM_CONTROL, NV_ESC_RM_ALLOC = 0x29, 0x2A, 0x2B
CMD_GET_PRIVILEGED_STATUS = 0x135
CLASSES = {"NV01_ROOT": 0x0, "NV01_ROOT_NON_PRIV": 0x1, "NV01_ROOT_CLIENT": 0x41}
CAP_SYS_ADMIN = 21
SYS_capget, SYS_capset = 125, 126  # x86_64
CAP_V3 = 0x20080522

libc = ctypes.CDLL(None, use_errno=True)


def iowr(nr, size):
    return (3 << 30) | (size << 16) | (ord("F") << 8) | nr


class CapHdr(ctypes.Structure):
    _fields_ = [("version", ctypes.c_uint32), ("pid", ctypes.c_int)]


class CapData(ctypes.Structure):
    _fields_ = [("effective", ctypes.c_uint32), ("permitted", ctypes.c_uint32),
                ("inheritable", ctypes.c_uint32)]


def capget():
    h = CapHdr(CAP_V3, 0)
    d = (CapData * 2)()
    if libc.syscall(SYS_capget, ctypes.byref(h), d) != 0:
        raise OSError(ctypes.get_errno(), "capget")
    return h, d


def set_sys_admin_effective(on):
    h, d = capget()
    if on:
        d[0].effective |= 1 << CAP_SYS_ADMIN
    else:
        d[0].effective &= ~(1 << CAP_SYS_ADMIN)
    if libc.syscall(SYS_capset, ctypes.byref(h), d) != 0:
        raise OSError(ctypes.get_errno(), "capset")


def eff_has_sys_admin():
    _, d = capget()
    return bool(d[0].effective >> CAP_SYS_ADMIN & 1), bool(d[0].permitted >> CAP_SYS_ADMIN & 1)


def alloc_root(fd, cls):
    buf = bytearray(struct.pack("<IIIIQII", 0, 0, 0, cls, 0, 0, 0))
    fcntl.ioctl(fd, iowr(NV_ESC_RM_ALLOC, 32), buf, True)
    h_root, _, h_new, h_class, _, _, status = struct.unpack("<IIIIQII", buf)
    return h_new, h_class, status


def priv_status(fd, h_client):
    out = ctypes.create_string_buffer(1)
    buf = bytearray(struct.pack("<IIIIQII", h_client, h_client, CMD_GET_PRIVILEGED_STATUS, 0,
                                ctypes.addressof(out), 1, 0))
    fcntl.ioctl(fd, iowr(NV_ESC_RM_CONTROL, 32), buf, True)
    status = struct.unpack("<IIIIQII", buf)[6]
    return out.raw[0], status


def free(fd, h):
    buf = bytearray(struct.pack("<IIII", h, 0, h, 0))
    fcntl.ioctl(fd, iowr(NV_ESC_RM_FREE, 16), buf, True)
    return struct.unpack("<IIII", buf)[3]


def decode(f):
    return "PRIV_USER=%d PRIV_HANDLE=%d (admin per rmclientIsAdmin)" % (f & 1, (f >> 2) & 1)


def main():
    print("== privprobe euid=%d pid=%d" % (os.geteuid(), os.getpid()))
    with open("/proc/self/status") as s:
        for line in s:
            if line.startswith(("CapPrm", "CapEff")):
                print("== " + line.strip())
    with open("/proc/driver/nvidia/version") as v:
        print("== " + v.readline().strip())
    fd = os.open("/dev/nvidiactl", os.O_RDWR)
    held = []
    for capeff in (True, False):
        set_sys_admin_effective(capeff)
        eff, prm = eff_has_sys_admin()
        for name, cls in CLASSES.items():
            h, cls_out, st = alloc_root(fd, cls)
            if st != 0:
                print("ARM capeff_sys_admin=%d class=%s alloc status=%#x" % (eff, name, st))
                continue
            flags, cst = priv_status(fd, h)
            print("ARM capeff_sys_admin=%d capprm_sys_admin=%d class=%s(%#x) reply_hClass=%#x "
                  "client=%#x ctrl_status=%#x privStatusFlags=%#x %s"
                  % (eff, prm, name, cls, cls_out, h, cst, flags, decode(flags)))
            held.append((h, name))
    # Per-call, not per-client: re-query the clients born with CAP_SYS_ADMIN effective,
    # now from a thread without it, then with it back.
    for capeff in (False, True):
        set_sys_admin_effective(capeff)
        eff, _ = eff_has_sys_admin()
        for h, name in held[:3]:
            flags, cst = priv_status(fd, h)
            print("REQUERY born_with_capeff=1 now_capeff_sys_admin=%d class=%s client=%#x "
                  "privStatusFlags=%#x %s" % (eff, name, h, flags, decode(flags)))
    for h, _ in held:
        free(fd, h)
    os.close(fd)
    print("== PRIVPROBE_EXIT rc=0")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # noqa: BLE001 — the exit line must always be written
        print("== PRIVPROBE_EXIT rc=1 err=%r" % (e,))
        sys.exit(1)
