"""M2 gate: CUDA Driver API, four streams, exact full CPU readback; no toolkit.

Expected: each uint32 output is (index * 3) XOR (seed + iteration).
Falsifier: any CUDA error, differing element, new Xid/TDR, or hang.
Run two processes together for distinct live guest CUDA contexts.
"""
import array
import ctypes as c
import os
import sys
import time

seconds = int(sys.argv[1])
seed = int(sys.argv[2])
os.environ['CUDA_CACHE_PATH'] = rf'C:\kf-m2\cache-{os.getpid()}'
api = c.WinDLL('nvcuda.dll')
P = c.c_void_p
U = c.c_uint
D = c.c_uint64
Z = c.c_size_t

def bind(name, args):
    fn = getattr(api, name)
    fn.argtypes = args
    fn.restype = c.c_int
    def call(*values):
        rc = fn(*values)
        if rc:
            raise RuntimeError(f'{name} rc={rc}')
    return call

init = bind('cuInit', [U])
getdev = bind('cuDeviceGet', [c.POINTER(c.c_int), c.c_int])
name = bind('cuDeviceGetName', [P, c.c_int, c.c_int])
create = bind('cuCtxCreate_v2', [c.POINTER(P), U, c.c_int])
destroy = bind('cuCtxDestroy_v2', [P])
module_load = bind('cuModuleLoadData', [c.POINTER(P), P])
module_unload = bind('cuModuleUnload', [P])
getfn = bind('cuModuleGetFunction', [c.POINTER(P), P, c.c_char_p])
alloc = bind('cuMemAlloc_v2', [c.POINTER(D), Z])
free = bind('cuMemFree_v2', [D])
stream_create = bind('cuStreamCreate', [c.POINTER(P), U])
stream_sync = bind('cuStreamSynchronize', [P])
stream_destroy = bind('cuStreamDestroy_v2', [P])
launch = bind('cuLaunchKernel', [P, U, U, U, U, U, U, U, P, c.POINTER(P), P])
read = bind('cuMemcpyDtoH_v2', [P, D, Z])
ptx = b'''.version 7.8
.target sm_86
.address_size 64
.visible .entry fill(.param .u64 dst, .param .u32 n, .param .u32 key) {
.reg .pred p;
.reg .b32 r<8>;
.reg .b64 d<4>;
ld.param.u64 d0,[dst];
ld.param.u32 r0,[n];
ld.param.u32 r1,[key];
mov.u32 r2,%ctaid.x;
mov.u32 r3,%ntid.x;
mov.u32 r4,%tid.x;
mad.lo.u32 r5,r2,r3,r4;
setp.ge.u32 p,r5,r0;
@p bra done;
mul.lo.u32 r6,r5,3;
xor.b32 r7,r6,r1;
mul.wide.u32 d1,r5,4;
add.u64 d2,d0,d1;
st.global.u32 [d2],r7;
done: ret;
}
'''
init(0)
dev = c.c_int()
getdev(c.byref(dev), 0)
devname = c.create_string_buffer(256)
name(devname, 256, dev)
ctx = P()
create(c.byref(ctx), 0, dev)
mod = P()
module_load(c.byref(mod), c.cast(c.c_char_p(ptx), P))
fn = P()
getfn(c.byref(fn), mod, b'fill')
n = 1 << 20
streams = []
buffers = []
for _ in range(4):
    st = P()
    stream_create(c.byref(st), 1)  # CU_STREAM_NON_BLOCKING
    streams.append(st)
    buf = D()
    alloc(c.byref(buf), n * 4)
    buffers.append(buf)
print(f'CUDA_CONTEXT_READY pid={os.getpid()} device={devname.value.decode()} streams=4 elements_per_stream={n} seed={seed}', flush=True)
start = time.monotonic()
print(f'CUDA_START wall_ns={time.time_ns()}', flush=True)
iteration = 0
checks = 0
last_print = start
while True:
    key = U(seed + iteration)
    count = U(n)
    for stream, buf in zip(streams, buffers):
        args = (P * 3)(c.cast(c.byref(buf), P), c.cast(c.byref(count), P), c.cast(c.byref(key), P))
        for _ in range(32):
            launch(fn, (n + 255) // 256, 1, 1, 256, 1, 1, 0, stream, args, None)
    for stream in streams:
        stream_sync(stream)
    final = time.monotonic() - start >= seconds
    if iteration == 0 or final:
        expected = array.array('I', ((i * 3) ^ key.value for i in range(n)))
        for idx, buf in enumerate(buffers):
            actual = array.array('I', [0]) * n
            view = (U * n).from_buffer(actual)
            read(c.cast(view, P), buf, n * 4)
            if actual != expected:
                first = next(i for i in range(n) if actual[i] != expected[i])
                raise RuntimeError(f'CPU_MISMATCH stream={idx} index={first} actual={actual[first]} expected={expected[first]}')
            checks += 1
        print(f'CPU_CHECK PASS iteration={iteration} streams=4 elements={4*n}', flush=True)
    iteration += 1
    now = time.monotonic()
    if now - last_print >= 15:
        print(f'CUDA_PROGRESS elapsed_s={now-start:.3f} batches={iteration}', flush=True)
        last_print = now
    if final:
        break
end = time.monotonic()
for stream in streams:
    stream_destroy(stream)
for buf in buffers:
    free(buf)
module_unload(mod)
destroy(ctx)
print(f'CUDA_PASS pid={os.getpid()} wall_ns={time.time_ns()} elapsed_s={end-start:.3f} batches={iteration} kernels={iteration*128} full_buffer_cpu_checks={checks}', flush=True)
