import ctypes, re, sys
ptx = open("/workspace/kf-ci-src/cuda/walk/kf_walk.ptx", "rb").read() + b"\0"
names = re.findall(rb"\.visible \.entry (\w+)", ptx) or re.findall(rb"\.entry (\w+)", ptx)
cu = ctypes.CDLL("libcuda.so.1")
def ck(rc, what):
    print(what, "rc=%d" % rc)
    if rc: sys.exit(1)
ck(cu.cuInit(0), "cuInit")
dev = ctypes.c_int(); ck(cu.cuDeviceGet(ctypes.byref(dev), 0), "cuDeviceGet")
ctx = ctypes.c_void_p(); ck(cu.cuCtxCreate_v2(ctypes.byref(ctx), 0, dev), "cuCtxCreate")
mod = ctypes.c_void_p(); ck(cu.cuModuleLoadData(ctypes.byref(mod), ptx), "cuModuleLoadData(committed kf_walk.ptx)")
for n in names:
    f = ctypes.c_void_p(); ck(cu.cuModuleGetFunction(ctypes.byref(f), mod, n), "cuModuleGetFunction " + n.decode())
print("PTX_JIT_OK entries=%d" % len(names))
