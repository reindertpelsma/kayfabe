#!/bin/bash
echo "=== inside the kata guest container ==="
echo "-- nvidia-smi --"
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1 | head -2
echo "-- libcuda present? --"
ldconfig -p 2>/dev/null | grep -m2 libcuda
echo "-- cuInit(0) --"
python3 - <<'PY' 2>&1 | tail -3
import ctypes
try:
    l = ctypes.CDLL("libcuda.so.1")
except OSError as e:
    print("libcuda load FAILED:", e); raise SystemExit
rc = l.cuInit(0)
print("cuInit rc =", rc, "(0=OK, 999=CUDA_ERROR_UNKNOWN)")
if rc == 0:
    n = ctypes.c_int()
    print("cuDeviceGetCount rc =", l.cuDeviceGetCount(ctypes.byref(n)), "devices =", n.value)
PY
