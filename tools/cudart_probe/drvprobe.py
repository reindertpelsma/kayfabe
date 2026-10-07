# driver-API-only probe — mirrors the calls the GUEST already passes
import ctypes
d = ctypes.CDLL("libcuda.so.1")
n = ctypes.c_int(-1); dev = ctypes.c_int(-1); ctx = ctypes.c_void_p()
print("DRV_cuInit=%d" % d.cuInit(0))
print("DRV_cuDeviceGetCount=%d n=%d" % (d.cuDeviceGetCount(ctypes.byref(n)), n.value))
print("DRV_cuDeviceGet=%d dev=%d" % (d.cuDeviceGet(ctypes.byref(dev), 0), dev.value))
buf = ctypes.create_string_buffer(256)
print("DRV_cuDeviceGetName=%d name=%s" % (d.cuDeviceGetName(buf, 256, dev), buf.value))
print("DRV_PrimaryCtxRetain=%d" % d.cuDevicePrimaryCtxRetain(ctypes.byref(ctx), dev))
print("DRV_CtxSetCurrent=%d" % d.cuCtxSetCurrent(ctx))
print("DRV_PROBE_END")
