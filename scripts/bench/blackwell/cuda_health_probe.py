"""Stock CUDA initialization/device-count health only; no context or GPU work is created."""

import ctypes
import json
import sys


def main():
    result = {}
    try:
        cuda = ctypes.CDLL("libcuda.so.1")
        cuda.cuInit.argtypes = [ctypes.c_uint]
        cuda.cuInit.restype = ctypes.c_int
        cuda.cuDeviceGetCount.argtypes = [ctypes.POINTER(ctypes.c_int)]
        cuda.cuDeviceGetCount.restype = ctypes.c_int
        result["cuInit"] = cuda.cuInit(0)
        count = ctypes.c_int()
        result["cuDeviceGetCount"] = cuda.cuDeviceGetCount(ctypes.byref(count))
        result["devices"] = count.value
        print(json.dumps(result))
        return int(result["cuInit"] != 0 or result["cuDeviceGetCount"] != 0 or count.value != 1)
    except (OSError, AttributeError) as error:
        print(json.dumps({"health_error": type(error).__name__}))
        return 1


if __name__ == "__main__":
    sys.exit(main())
