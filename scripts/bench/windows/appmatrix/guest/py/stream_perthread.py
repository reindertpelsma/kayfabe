# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""CUDA per-thread default stream, GPU arithmetic and full CPU-reference readback.

Use the bundled CUDA DLL directory without importing the unrelated Torch runtime.
CuPy14.2 documents Stream.ptds as the per-thread default stream (handle2):
https://docs.cupy.dev/en/stable/reference/generated/cupy.cuda.Stream.html
No CPU execution fallback and no change to the matrix's120s deadline.
"""
import os
import sys
import time

start = time.monotonic()
directory = os.environ.get("KF_TORCH_LIB")
dll_handles = []
if directory and os.path.isdir(directory):
    if hasattr(os, "add_dll_directory"):
        dll_handles.append(os.add_dll_directory(directory))
    os.environ["PATH"] = directory + os.pathsep + os.environ.get("PATH", "")

import cupy as cp
import numpy as np

print(f"STREAM_STAGE cupy_import elapsed_s={time.monotonic() - start:.6f}", flush=True)
properties = cp.cuda.runtime.getDeviceProperties(cp.cuda.runtime.getDevice())
name = properties["name"]
if isinstance(name, bytes):
    name = name.decode("utf-8", errors="replace")
if not any(word in name for word in ("NVIDIA", "GeForce", "RTX")):
    raise RuntimeError(f"NVIDIA CUDA device required, got {name!r}")
print("KFDEV", name, "cupy", cp.__version__, flush=True)

n = 1 << 20
stream = cp.cuda.Stream.ptds
with stream:
    if cp.cuda.get_current_stream().ptr != stream.ptr or stream.ptr == cp.cuda.Stream.null.ptr:
        raise RuntimeError("per-thread default stream was not made current")
    x = cp.arange(n, dtype=cp.float32)
    y = x * cp.float32(2)
    stream.synchronize()
    actual = cp.asnumpy(y)
expected = np.arange(n, dtype=np.float32) * np.float32(2)
ok = bool(np.array_equal(actual, expected))
print(f"CHECK stream_perthread {'ok' if ok else 'FAIL'} elements={n} stream={stream.ptr}", flush=True)
print(f"STREAM_PROBE_DONE perthread rc={0 if ok else 1}", flush=True)
sys.exit(0 if ok else 1)
