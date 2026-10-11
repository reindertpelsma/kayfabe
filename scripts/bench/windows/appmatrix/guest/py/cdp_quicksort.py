# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU recursive quicksort via documented NVRTC + cudadevrt Driver API linking.
No nvcc/MSVC, CPU sort only as the result oracle. NVIDIA's public NVRTC 12.9
Dynamic Parallelism example documents this Windows runtime-link route:
https://docs.nvidia.com/cuda/archive/12.9.0/nvrtc/index.html#example-dynamic-parallelism
"""
import ctypes as C
import glob
import os
import random
import torch

SOURCE = r'''
__device__ void small_sort(int *a, int lo, int hi) {
  for (int i=lo+1; i<=hi; ++i) {
    int x=a[i], j=i-1;
    while (j>=lo && a[j]>x) { a[j+1]=a[j]; --j; }
    a[j+1]=x;
  }
}
extern "C" __global__ void quicksort(int *a, int lo, int hi, int depth, int *status) {
  if (threadIdx.x || blockIdx.x || lo>=hi) return;
  if (hi-lo<32 || depth>=16) { small_sort(a,lo,hi); return; }
  int i=lo, j=hi, pivot=a[lo+(hi-lo)/2];
  while (i<=j) {
    while (a[i]<pivot) ++i;
    while (a[j]>pivot) --j;
    if (i<=j) { int t=a[i]; a[i]=a[j]; a[j]=t; ++i; --j; }
  }
  if (lo<j) {
    cudaStream_t s;
    cudaError_t e=cudaStreamCreateWithFlags(&s,cudaStreamNonBlocking);
    if (e!=cudaSuccess) { atomicCAS(status+1,0,(int)e); return; }
    quicksort<<<1,1,0,s>>>(a,lo,j,depth+1,status);
    e=cudaGetLastError();
    if (e!=cudaSuccess) atomicCAS(status+1,0,(int)e);
    else atomicAdd(status,1);
    e=cudaStreamDestroy(s);
    if (e!=cudaSuccess) atomicCAS(status+1,0,(int)e);
  }
  if (i<hi) {
    cudaStream_t s;
    cudaError_t e=cudaStreamCreateWithFlags(&s,cudaStreamNonBlocking);
    if (e!=cudaSuccess) { atomicCAS(status+1,0,(int)e); return; }
    quicksort<<<1,1,0,s>>>(a,i,hi,depth+1,status);
    e=cudaGetLastError();
    if (e!=cudaSuccess) atomicCAS(status+1,0,(int)e);
    else atomicAdd(status,1);
    e=cudaStreamDestroy(s);
    if (e!=cudaSuccess) atomicCAS(status+1,0,(int)e);
  }
}
'''


def bind(lib, name, args):
    f = getattr(lib, name)
    f.argtypes, f.restype = args, C.c_int
    return f


def check(rc, name):
    if rc:
        raise RuntimeError(f'{name} returned {rc}')


def main():
    assert torch.cuda.is_available(), 'CUDA GPU required'
    root = os.environ.get('KF_CDP_NVRTC', r'C:\kfapps\cuda_nvrtc')
    dll = [p for p in glob.glob(os.path.join(root, 'bin', 'nvrtc64_*.dll')) if not p.endswith('.alt.dll')]
    assert len(dll) == 1, f'expected one pinned NVRTC DLL under {root}'
    bin_dir = os.path.dirname(dll[0])
    dll_dir = os.add_dll_directory(bin_dir)
    os.environ['PATH'] = bin_dir + os.pathsep + os.environ.get('PATH', '')
    # NVRTC loads its builtins by basename internally. Keep the matching pinned
    # DLL loaded explicitly, including when Torch already loaded another NVRTC.
    builtins = glob.glob(os.path.join(bin_dir, 'nvrtc-builtins64_*.dll'))
    assert len(builtins) == 1, 'expected matching pinned NVRTC builtins'
    builtins_lib = C.WinDLL(builtins[0])
    nv = C.WinDLL(dll[0]); cu = C.WinDLL('nvcuda.dll')
    vp, size = C.c_void_p, C.c_size_t
    create = bind(nv, 'nvrtcCreateProgram', [C.POINTER(vp), C.c_char_p, C.c_char_p, C.c_int, vp, vp])
    compile_ = bind(nv, 'nvrtcCompileProgram', [vp, C.c_int, C.POINTER(C.c_char_p)])
    get_log_size = bind(nv, 'nvrtcGetProgramLogSize', [vp, C.POINTER(size)])
    get_log = bind(nv, 'nvrtcGetProgramLog', [vp, vp])
    get_ptx_size = bind(nv, 'nvrtcGetPTXSize', [vp, C.POINTER(size)])
    get_ptx = bind(nv, 'nvrtcGetPTX', [vp, vp])
    destroy = bind(nv, 'nvrtcDestroyProgram', [C.POINTER(vp)])
    prog = vp()
    check(create(C.byref(prog), SOURCE.encode(), b'kf_cdp_quicksort.cu', 0, None, None), 'nvrtcCreateProgram')
    major, minor = torch.cuda.get_device_capability(0)
    opts = [f'--gpu-architecture=compute_{major}{minor}'.encode(), b'--relocatable-device-code=true', b'--std=c++17']
    try:
        rc = compile_(prog, len(opts), (C.c_char_p * len(opts))(*opts))
        n = size(); check(get_log_size(prog, C.byref(n)), 'nvrtcGetProgramLogSize')
        log = C.create_string_buffer(n.value)
        check(get_log(prog, log), 'nvrtcGetProgramLog')
        if log.value: print(log.value.decode(errors='replace'), flush=True)
        check(rc, 'nvrtcCompileProgram')
        check(get_ptx_size(prog, C.byref(n)), 'nvrtcGetPTXSize')
        ptx = C.create_string_buffer(n.value)
        check(get_ptx(prog, ptx), 'nvrtcGetPTX')
    finally:
        check(destroy(C.byref(prog)), 'nvrtcDestroyProgram')
    # Tensor allocation makes the PyTorch primary context current. Synchronize
    # input initialization before submitting through the Driver API default stream.
    rng = random.Random(0x434450)
    values = [rng.randrange(-100000, 100000) for _ in range(4096)]
    a = torch.tensor(values, dtype=torch.int32, device='cuda:0')
    status = torch.zeros(2, dtype=torch.int32, device='cuda:0')
    torch.cuda.synchronize()
    error_log = C.create_string_buffer(8192)
    options = (C.c_int * 2)(5, 6)  # cuda.h: CU_JIT_ERROR_LOG_BUFFER / BUFFER_SIZE_BYTES
    option_values = (vp * 2)(C.cast(error_log, vp), vp(len(error_log)))
    link = vp()
    link_create = bind(cu, 'cuLinkCreate_v2', [C.c_uint, C.POINTER(C.c_int), C.POINTER(vp), C.POINTER(vp)])
    add_ptx = bind(cu, 'cuLinkAddData_v2', [vp, C.c_int, vp, size, C.c_char_p, C.c_uint, vp, vp])
    add_lib = bind(cu, 'cuLinkAddFile_v2', [vp, C.c_int, C.c_char_p, C.c_uint, vp, vp])
    complete = bind(cu, 'cuLinkComplete', [vp, C.POINTER(vp), C.POINTER(size)])
    link_destroy = bind(cu, 'cuLinkDestroy', [vp])
    check(link_create(2, options, option_values, C.byref(link)), 'cuLinkCreate')
    try:
        check(add_ptx(link, 1, ptx, len(ptx), b'kf_cdp_quicksort.ptx', 0, None, None), 'cuLinkAddData')
        lib = os.environ.get('KF_CDP_DEVRT', r'C:\kfapps\cuda_cudart\lib\x64\cudadevrt.lib')
        check(add_lib(link, 4, os.fsencode(lib), 0, None, None), 'cuLinkAddFile cudadevrt')
        cubin, cubin_size = vp(), size()
        check(complete(link, C.byref(cubin), C.byref(cubin_size)), 'cuLinkComplete')
        module = vp()
        check(bind(cu, 'cuModuleLoadData', [C.POINTER(vp), vp])(C.byref(module), cubin), 'cuModuleLoadData')
    finally:
        if error_log.value: print(error_log.value.decode(errors='replace'), flush=True)
        check(link_destroy(link), 'cuLinkDestroy')
    try:
        kernel = vp()
        check(bind(cu, 'cuModuleGetFunction', [C.POINTER(vp), vp, C.c_char_p])(C.byref(kernel), module, b'quicksort'), 'cuModuleGetFunction')
        arguments = [C.c_uint64(a.data_ptr()), C.c_int(0), C.c_int(len(values)-1), C.c_int(0), C.c_uint64(status.data_ptr())]
        params = (vp * len(arguments))(*(C.cast(C.byref(x), vp) for x in arguments))
        launch = bind(cu, 'cuLaunchKernel', [vp] + [C.c_uint]*7 + [vp, C.POINTER(vp), vp])
        check(launch(kernel, 1, 1, 1, 1, 1, 1, 0, None, params, None), 'cuLaunchKernel')
        check(bind(cu, 'cuCtxSynchronize', [])(), 'cuCtxSynchronize')
        launches, error = status.cpu().tolist()
        result = a.cpu().tolist()
        assert error == 0, f'device-side CUDA error {error}'
        assert launches > 0, 'GPU must launch child kernels'
        assert result == sorted(values), 'GPU quicksort readback differs'
        print('CDP_DEVICE', torch.cuda.get_device_name(0))
        print(f'CDP_CHILD_LAUNCHES {launches} CUDA_ERROR {error}')
        print('Validating results: OK')
    finally:
        check(bind(cu, 'cuModuleUnload', [vp])(module), 'cuModuleUnload')
        dll_dir.close()


if __name__ == '__main__':
    main()
