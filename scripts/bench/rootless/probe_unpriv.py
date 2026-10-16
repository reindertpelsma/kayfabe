import ctypes, os, errno, sys
libc = ctypes.CDLL(None, use_errno=True)
def sc(nr, *a):
    r = libc.syscall(nr, *[ctypes.c_long(x) for x in a]); return r, ctypes.get_errno()
res = {}
hdr = "uid %d groups %s CapEff %s" % (os.getuid(), os.getgroups(), open("/proc/self/status").read().split("CapEff:")[1].split("\n")[0].strip())
# x86_64 syscall numbers; unshare(CLONE_NEWUSER) runs LAST because it changes this process's identity
r,e = sc(323, 0o4000|0)   # userfaultfd(O_NONBLOCK) -> needs CAP_SYS_PTRACE or vm.unprivileged_userfaultfd=1 for kernel faults
res["userfaultfd(plain)"] = "ok" if r>=0 else errno.errorcode[e]
r,e = sc(323, 0o4000|1)   # UFFD_USER_MODE_ONLY = 1
res["userfaultfd(USER_MODE_ONLY)"] = "ok" if r>=0 else errno.errorcode[e]
params = ctypes.create_string_buffer(120)
r,e = libc.syscall(425, 4, params), ctypes.get_errno()  # io_uring_setup
res["io_uring_setup"] = "ok" if r>=0 else errno.errorcode[e]
try:
    fd = os.open("/dev/udmabuf", os.O_RDWR); res["open /dev/udmabuf"] = "ok"; os.close(fd)
except OSError as ex: res["open /dev/udmabuf"] = errno.errorcode[ex.errno]
for p in ("/dev/kvm","/dev/nvidiactl","/dev/nvidia0","/dev/nvidia-uvm","/dev/userfaultfd","/dev/vfio/vfio","/dev/hugepages","/dev/dri/renderD129","/dev/dri/card1","/dev/mem"):
    try:
        fd = os.open(p, os.O_RDWR if not p.endswith("hugepages") else os.O_RDONLY); res["open "+p] = "ok"; os.close(fd)
    except OSError as ex: res["open "+p] = errno.errorcode[ex.errno]
import mmap
try:
    m = mmap.mmap(-1, 1<<20, flags=mmap.MAP_PRIVATE|mmap.MAP_ANONYMOUS|0x2000)  # MAP_LOCKED
    res["mmap(1MiB MAP_LOCKED)"]="ok"
except OSError as ex: res["mmap(1MiB MAP_LOCKED)"] = errno.errorcode[ex.errno]
try:
    m = mmap.mmap(-1, 64<<20, flags=mmap.MAP_PRIVATE|mmap.MAP_ANONYMOUS|0x2000)
    res["mmap(64MiB MAP_LOCKED, memlock=8MiB)"]="ok"
except OSError as ex: res["mmap(64MiB MAP_LOCKED, memlock=8MiB)"] = errno.errorcode[ex.errno]
r,e = sc(319, ctypes.c_char_p(b"x"), 1) if False else (libc.memfd_create(b"x", 0), ctypes.get_errno())
res["memfd_create"] = "ok" if r>=0 else errno.errorcode[e]
r,e = sc(272, 0x10000000)  # unshare(CLONE_NEWUSER)
res["unshare(CLONE_NEWUSER)"] = "ok" if r>=0 else errno.errorcode[e]
for k,v in res.items(): print(f"{k:45s} {v}")
print(hdr)
