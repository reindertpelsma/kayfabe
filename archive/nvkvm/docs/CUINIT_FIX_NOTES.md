# cuInit fix notes — what diagnosis turned up

Captured by `tools/diag/nvioctl_trace` and `tools/diag/diff_traces.py` on
2026-05-25. Both host and guest run the same `cu_test` (`dlopen libcuda` +
`cuInit(0)`) on driver 575.51.03.

## Bugs found and fixed

### 1. nvos54 `params` pointer not round-tripping

CUDA reads the input pointer back from the response struct after RM_CONTROL
ioctls. The host driver does not modify it. Our sanitizer cleared it
on the way to the host (no leakage of guest VAs to the driver) and our
stub re-cleared it post-ioctl (no leakage of stub VAs back to the guest),
leaving CUDA with `params=0` in the response — CUDA treated this as the
struct never made it to the driver and aborted.

Fix: in the guest `nvkvm_main.c` ioctl handler, snapshot the caller's
`nvos54.params` before `nvkvm_sanitize_ioctl_params`, restore it into
`params_buf` before `copy_to_user`. (Symmetric for nvos21
`p_alloc_parms` and nvos64 `p_alloc_parms` / `p_rights_requested`.)

### 2. nvos64 `alloc_parms_size` not round-tripping

For RM_ALLOC on classes like `NV01_DEVICE_0` CUDA leaves
`alloc_parms_size = 0` and the driver sizes the buffer by `hClass`.
Our sanitizer (correctly) infers the size by class and writes it into
`alloc_parms_size` so QEMU/stub see a consistent `(params, size)`
pair. But that write made it back to the user too, where CUDA — which
had set 0 — saw 56 and bailed.

Fix: snapshot the original `alloc_parms_size` before the sanitizer
touches it; restore in the response.

## Bug still pending (architectural)

### 3. `RM_MAP_MEMORY` returns different response data on guest vs host

After 1 and 2, the first 60-ish ioctls of cuInit produce byte-identical
responses (modulo handles and user-space VAs, which are inherently
per-process). The divergence is at the `NV_ESC_RM_MAP_MEMORY` (NR=0x4e,
56-byte struct).

Bytes 32-55 of the response, aligned to the `nv_ioctl_nvos33_parameters_with_fd`
layout:

```
                            offset:  32 33 34 35 | 36 37 38 39 | 40 41 42 43 | 44 45 46 47 | 48 49 50 51 | 52 53 54 55
field (per our nvgpu.h):           [   pLinearAddress (8 bytes)   ] [ status  ] [  flags  ] [    fd    ] [  pad    ]
HOST  POST (first call):    00 00 00 00 | 00 00 00 00 | 00 00 00 00 | 00 00 02 00 | 08 03 0f 00 | 00 00 00 00
GUEST POST (first call):    00 00 00 00 | bb c0 00 00 | 00 00 1f 00 | 00 00 02 00 | 08 03 ee 01 | 00 00 00 00
                                              ^^^^^^^^^^^                ^^^^^^^^^
                                          differs                       differs
```

The driver returns a different `status` (and partial `pLinearAddress`)
when called from QEMU's process vs CUDA's process. CUDA reads the
response, sees non-zero status, treats the map as failed, and switches
to the cleanup path (RM_FREE × n, UVM_DEINIT) → cuInit returns
CUDA_ERROR_NO_DEVICE (100).

This is the same class of issue as UVM_MM_INITIALIZE returning
NV_WARN_MORE_PROCESSING_REQUIRED — the host driver's behaviour depends
on the calling process's `mm`, and our forwarder (QEMU) is not a
CUDA-like process. The architectural fix is `install_isolate_mapping`:
intercept the mapping-creating ioctls (including RM_MAP_MEMORY) in
either the stub or the guest kernel, and own the mapping bookkeeping
end-to-end so the GPA window the guest sees is consistent with what
CUDA expects.

(Quick experiment ruled out: zeroing vs leaking `p_linear_address` does
not change CUDA's behaviour — confirmed by trace tool. CUDA does NOT
read pLinearAddress to decide whether the map succeeded; it reads
`status` (and probably other fields) elsewhere.)

## How the diagnostic tool was decisive

`nvioctl_trace` runs the target binary under ptrace, intercepts every
`ioctl` syscall on a `/dev/nvidia*` fd, and records:

- the cmd number (decoded into RM_CONTROL inner cmd / RM_ALLOC hClass)
- pre-call and post-call hex dumps of the arg buffer
- following the `params` pointer for RM_CONTROL/ALLOC, pre+post dumps
  of the inner struct too
- a DELTA mask highlighting only the bytes the driver changed

Side-by-side diff (`diff_traces.py`) groups by `(cmd, inner_cmd /
hClass)` and red-highlights byte positions that differ between host
and guest while ignoring expected differences (random handles,
per-process pointer VAs). Without that, "ioctls all return NV_OK" was
the only signal — too coarse to find the actual blocker. With it the
bugs above were obvious within minutes of capturing the first traces.

## How to reproduce

```
# host:
cd tools/diag && make
./nvioctl_trace -o /tmp/host.log -- ./cu_test

# guest VM (build inside, glibc abi):
cp -r tools/diag /mnt/nvkvm/diag
cd /tmp && cp /mnt/nvkvm/diag/* . && make
./nvioctl_trace -o /tmp/guest.log -- ./cu_test

# compare:
python3 tools/diag/diff_traces.py /tmp/host.log /tmp/guest.log
```
