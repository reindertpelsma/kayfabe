import ctypes, sys, os
lib = ctypes.CDLL("/opt/nvcc/pkg/nvidia/cuda_nvrtc/lib/libnvrtc.so.12")
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
