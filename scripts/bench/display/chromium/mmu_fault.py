#!/usr/bin/env python3
"""mmu_fault.py [n] — deliberately fault the GPU MMU from a CUDA kernel (store to an unmapped address), n times,
each in a fresh context: the CUDA error is 700 (illegal address) and the RM logs Xid 31 and resets the channel
(NV906F_CTRL_CMD_RESET_CHANNEL, reason MMU_FLT). Used to see what kayfabe does with that control (fn76 0x906f0102)."""
import ctypes, sys, time
PTX = rb""".version 7.0
.target sm_50
.address_size 64
.visible .entry bad(.param .u64 p) { .reg .u64 %a; .reg .u32 %v; mov.u64 %a, 0xdead00000000; mov.u32 %v, 1; st.global.u32 [%a], %v; ret; }
"""
cu = ctypes.CDLL('libcuda.so.1')
def chk(r, what):
    if r: print(f'{what}: CUresult {r}', flush=True)
    return r
n = int(sys.argv[1]) if len(sys.argv) > 1 else 1
chk(cu.cuInit(0), 'cuInit')
for i in range(n):
    dev = ctypes.c_int(); ctx = ctypes.c_void_p(); mod = ctypes.c_void_p(); fn = ctypes.c_void_p()
    chk(cu.cuDeviceGet(ctypes.byref(dev), 0), 'cuDeviceGet')
    chk(cu.cuCtxCreate_v2(ctypes.byref(ctx), 0, dev), 'cuCtxCreate')
    chk(cu.cuModuleLoadData(ctypes.byref(mod), PTX), 'cuModuleLoadData')
    chk(cu.cuModuleGetFunction(ctypes.byref(fn), mod, b'bad'), 'cuModuleGetFunction')
    arg = ctypes.c_uint64(0); args = (ctypes.c_void_p * 1)(ctypes.cast(ctypes.byref(arg), ctypes.c_void_p))
    chk(cu.cuLaunchKernel(fn, 1, 1, 1, 1, 1, 1, 0, None, args, None), 'cuLaunchKernel')
    t = time.time(); r = cu.cuCtxSynchronize()
    print(f'iteration {i}: cuCtxSynchronize -> {r} ({time.time()-t:.2f}s)', flush=True)
    cu.cuCtxDestroy_v2(ctx)
    time.sleep(2)
print('MMU_FAULT_DONE')
