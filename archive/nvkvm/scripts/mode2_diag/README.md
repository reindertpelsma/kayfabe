# mode2_diag — Mode-2 host-vs-guest ioctl diagnostic

Find where the guest (Mode-2, m2fwd) diverges from a native host run, to pinpoint
cuCtxCreate (and later) bugs without guessing.

## nvioctl_trace.so  (LD_PRELOAD ioctl tracer — no driver changes)
Decodes every NVIDIA RM ioctl: NVOS54 (RM_CONTROL, NR 0x2A) and NVOS21/64 (RM_ALLOC, NR 0x2B),
logging `cmd`/`class`, `paramsSize` (= the buffer libcuda allocated), `status`, and params ptr.
Runs identically on the guest test and a native host test.

    gcc -shared -fPIC -O2 -o nvioctl_trace.so nvioctl_trace.c -ldl
    # host:  LD_PRELOAD=./nvioctl_trace.so NVTRACE=/tmp/host_trace.txt  ./cup2_host
    # guest: LD_PRELOAD=./nvioctl_trace.so NVTRACE=/tmp/guest_trace.txt ./cup2

Then diff (normalize out ASLR params= addrs):
    sed -E 's/ params=0x[0-9a-f]+//' host_trace.txt  | grep -E 'CTRL|ALLOC' > h.norm
    sed -E 's/ params=0x[0-9a-f]+//' guest_trace.txt | grep -E 'CTRL|ALLOC' > g.norm
    diff g.norm h.norm
A control whose `status` differs (guest NV_OK vs host NOT_SUPPORTED) = a faked-success bug:
the host skips the param copyout, the guest copies garbage over libcuda's buffer.

Opt-in patch modes for negative tests:

    NVPATCH_GPUFLAGS=1
    NVCLASSLIST_HEX_FILE=/tmp/host_classlist.hex

`NVPATCH_GPUFLAGS=1` forces `NV0000_CTRL_CMD_GPU_GET_ID_INFO(_V2).gpuFlags |= IN_USE` after the
ioctl. `NVCLASSLIST_HEX_FILE` replaces `NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2` content with a hex
blob captured from the host. Both were used on 2026-06-07 to rule out the remaining early
gpuFlags/classlist content diffs: even with both patched to host values, Mode-2 still crashes after
the `c7c0` alloc.

## report.py  — joins the guest dmesg (instrumented driver) with the QEMU RPC log

## Wins so far (2026-06-06)
- This diff found NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS (0x2080012f) returning fake NV_OK on the
  guest vs NOT_SUPPORTED on the no-ECC GeForce host -> fixed (QEMU returns 0x56).
- And NVLINK_GET_NVLINK_STATUS (0x20803002) likewise (no NVLink on GeForce).

## ptrace tracer (nvtrace.c) + semantic decoder/differ (nvdecode.py)  [2026-06-06]
Full kernel-boundary coverage (catches inline-asm/raw syscalls + /dev/nvidia-uvm that LD_PRELOAD
missed). Field-by-field NVOS struct decode from the SDK headers; host==guest for semantic diff.

    gcc -O2 -o nvtrace nvtrace.c
    # host:  ./nvtrace -o /tmp/host_nvtrace.txt  -- /tmp/cup2_host
    # guest: bash nvtrace_outer.sh   (boots VM, runs cup2 under nvtrace, pulls /tmp/guest_nvtrace.txt)
    python3 nvdecode.py decode /tmp/host_nvtrace.txt          # decoded, named fields
    python3 nvdecode.py diff   /tmp/host_nvtrace.txt /tmp/guest_nvtrace.txt   # field-level host-vs-guest

### First findings (the tool's first run, cuCtxCreate crash):
- NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2 numClasses: host 0x6b(107) vs guest 0x61(97) — guest
  advertises 10 FEWER GPU classes. Later LD_PRELOAD replay proved this is not the current crash.
- NV2080_CTRL_CMD_CE_GET_ALL_CAPS capsTbl: host 0xe3 vs guest 0 — guest reports NO copy-engine caps.
- guest stops right after ALLOC class=0xc7c0 (GR compute object). As of 2026-06-07, the classlist
  divergence is ruled out for this crash; keep using the trace to separate causal reply deltas from
  harmless host/guest shape differences.
