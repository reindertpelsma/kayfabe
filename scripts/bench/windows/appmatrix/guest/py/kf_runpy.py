# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kf_runpy.py SCRIPT [ARGS...] -- run a python script of the app matrix inside the app disk's embeddable Python.
Adds the torch wheel's lib directory to the DLL search path and imports torch first, so that CuPy/NVRTC/cuBLAS
find the CUDA DLLs that only the torch wheel ships. A failed torch import is reported and the script runs anyway
(scripts that do not need torch, e.g. ort_dml.py, are unaffected)."""
import os
import runpy
import sys

tl = os.environ.get("KF_TORCH_LIB")
if tl and os.path.isdir(tl) and hasattr(os, "add_dll_directory"):
    os.add_dll_directory(tl)
    os.environ["PATH"] = tl + os.pathsep + os.environ.get("PATH", "")
try:
    import torch  # noqa: F401  (pre-loads the CUDA runtime DLLs)
except Exception as e:  # pragma: no cover
    print("kf_runpy: torch import failed:", e, file=sys.stderr)
script = sys.argv[1]
sys.argv = sys.argv[1:]
runpy.run_path(script, run_name="__main__")
