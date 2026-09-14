#!/usr/bin/env python3
"""★★★★★ BUILD THE PTX KAYFABE SHIPS, LOCALLY, WITH NO GPU AND NO nvcc.

`THE_CONSTRAINTS.md` §20: *"it is not code injection: the PTX is ours, built at build time"*.
Making that literally true of the shipped artifact means generating it where the rest of the
tree is generated -- not on a rented box, whose outputs this project does not copy back.

NVRTC is the one CUDA front end that satisfies that: a library, no GPU, no toolkit, no `nvcc`
driver. It is reached through `ctypes` so this script has no build dependency at all beyond a
`libnvrtc.so.12` on disk (the `nvidia-cuda-nvrtc-cu12` wheel is enough; a CUDA install works
too).

  usage: python3 make_ptx.py kf_walk.cu kf_walk.ptx compute_75
         NVRTC_SO=/path/to/libnvrtc.so.12 python3 make_ptx.py ...

Two things it has to supply that NVRTC does not:

  1. `-DKF_DEVICE_ONLY=1`, which skips the half of `kf_walk.cu` that drives the CUDA RUNTIME
     API. NVRTC cannot compile that half, and kayfabe does not use it -- the isolate drives the
     DRIVER API from Rust. The alternative was slicing the file by line number, which rots on
     the first edit above the seam and would silently emit PTX for a different program.
  2. `stdint.h` and `stddef.h`, as named in-memory headers. NVRTC ships no libc headers, and
     `kf_walk.h` asks for the fixed-width integer types. Supplying them here rather than
     editing the source keeps the source the file the 58/58 suite compiles.
"""
import ctypes, sys, os
# ⊘ Overridable, and the default is the plain soname so a machine with CUDA installed needs
# no configuration. A hard-coded path would make this script work on exactly one box.
_SO = os.environ.get("NVRTC_SO", "libnvrtc.so.12")
try:
    lib = ctypes.CDLL(_SO)
except OSError as e:
    sys.stderr.write(
        f"could not load {_SO}: {e}\n"
        "NVRTC is how this tree builds the walk kernel's PTX without a GPU or nvcc.\n"
        "  pip download nvidia-cuda-nvrtc-cu12 --no-deps -d /tmp/w && unzip -o /tmp/w/*.whl -d /tmp/nvrtc\n"
        "  NVRTC_SO=/tmp/nvrtc/nvidia/cuda_nvrtc/lib/libnvrtc.so.12 python3 make_ptx.py ...\n"
    )
    raise SystemExit(2)
src_path, out_path, arch = sys.argv[1], sys.argv[2], sys.argv[3]
src = open(src_path, "rb").read()
# ⊘ NVRTC ships no libc headers. These are the ONLY declarations kf_walk.h asks for,
# supplied as named headers rather than by editing the source — the source must stay
# the file the 58/58 suite compiles.
STDINT = b"""#pragma once
typedef signed char int8_t;   typedef unsigned char uint8_t;
typedef short int16_t;        typedef unsigned short uint16_t;
typedef int int32_t;          typedef unsigned int uint32_t;
typedef long long int64_t;    typedef unsigned long long uint64_t;
typedef unsigned long uintptr_t; typedef long intptr_t;
"""
STDDEF = b"#pragma once\ntypedef unsigned long size_t;\ntypedef long ptrdiff_t;\n#define NULL 0\n"
names = [b"stdint.h", b"stddef.h"]
bodies = [STDINT, STDDEF]
na = (ctypes.c_char_p * len(names))(*names)
ba = (ctypes.c_char_p * len(bodies))(*bodies)
prog = ctypes.c_void_p()
r = lib.nvrtcCreateProgram(ctypes.byref(prog), src, src_path.encode(), len(names), ba, na)
assert r == 0, ("create", r)
inc = os.path.dirname(os.path.abspath(src_path)).encode()
opts = [b"--gpu-architecture=" + arch.encode(), b"-DKF_DEVICE_ONLY=1",
        b"-I" + inc, b"-default-device", b"--std=c++14"]
arr = (ctypes.c_char_p * len(opts))(*opts)
r = lib.nvrtcCompileProgram(prog, len(opts), arr)
n = ctypes.c_size_t()
lib.nvrtcGetProgramLogSize(prog, ctypes.byref(n))
log = ctypes.create_string_buffer(n.value)
lib.nvrtcGetProgramLog(prog, log)
sys.stderr.write(log.value.decode(errors="replace"))
if r != 0:
    sys.stderr.write(f"\nNVRTC_COMPILE_FAILED rc={r}\n"); sys.exit(1)
lib.nvrtcGetPTXSize(prog, ctypes.byref(n))
buf = ctypes.create_string_buffer(n.value)
lib.nvrtcGetPTX(prog, buf)
open(out_path, "wb").write(buf.value)
print(f"PTX_BYTES={len(buf.value)} -> {out_path}")
