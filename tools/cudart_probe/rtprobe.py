# minimal cudart probe — ONE call, so the two traces are maximally alignable
import ctypes, glob, os, sys
cands = sorted(glob.glob(os.environ.get("RT_GLOB", "/opt/llm/venv/lib/python*/site-packages/nvidia/cuda_runtime/lib/libcudart.so.12")))
if not cands:
    print("RT_ABSENT"); sys.exit(2)
print("RT_LIB=%s" % cands[0])
rt = ctypes.CDLL(cands[0])
rt.cudaGetErrorString.restype = ctypes.c_char_p
n = ctypes.c_int(-1)
rc = rt.cudaGetDeviceCount(ctypes.byref(n))
print("RT_cudaGetDeviceCount=%d n=%d msg=%s" % (rc, n.value, rt.cudaGetErrorString(rc).decode()))
print("RT_PROBE_END")
