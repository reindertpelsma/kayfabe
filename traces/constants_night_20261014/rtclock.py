# cudart probe for the constants lane (2026-10-14): one device count plus the two clock attributes.
# Run unprivileged under the nvdiff shim; prints only status lines (no board data).
import ctypes, sys
lib = sys.argv[1]
rt = ctypes.CDLL(lib)
rt.cudaGetErrorString.restype = ctypes.c_char_p
n = ctypes.c_int(-1)
rc = rt.cudaGetDeviceCount(ctypes.byref(n))
print("RT_cudaGetDeviceCount=%d n=%d msg=%s" % (rc, n.value, rt.cudaGetErrorString(rc).decode()))
for name, attr in (("ClockRate", 13), ("MemoryClockRate", 36)):
    v = ctypes.c_int(-1)
    rc = rt.cudaDeviceGetAttribute(ctypes.byref(v), attr, 0)
    print("RT_attr %s=%d rc=%d" % (name, v.value, rc))
print("RT_PROBE_END")
