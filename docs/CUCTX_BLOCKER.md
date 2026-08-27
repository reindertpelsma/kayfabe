# cuCtxCreate blocker — current state 2026-05-26

cuInit succeeds and reports 1 GPU.  `cuCtxCreate` fails with
`CUDA_ERROR_INVALID_VALUE` (code 1).  All ioctls return without
syscall-level errors — the failure is libcuda interpreting two
specific driver-level statuses as fatal.

## The two driver-level failures

After the V550 + V570 ABI fixes landed, every RM_ALLOC class libcuda
touches during cuCtxCreate is sized correctly and the driver accepts
the allocation parameters.  Two control / frontend ioctls still
return non-zero nvstatus:

```
nvkvm: ioctl_on_isolate: ... cmd=0xc028465e ret=0 nvstatus=0x2 fault=0x0
  ↑ NV_ESC_RM_UPDATE_DEVICE_MAPPING_INFO (NVOS56)

nvkvm: ioctl_on_isolate: ... cmd=0xc020462a inner=0x80170d ret=0 nvstatus=0x1e
  ↑ NV0080_CTRL_CMD_FIFO_GET_CHANNELLIST  → NV_ERR_INVALID_DATA
```

Both share the same root cause: the parameter structs contain CPU
virtual-address pointers that the driver dereferences in the isolate
process's mm:

**NVOS56** has `p_old_cpu_address` and `p_new_cpu_address` — libcuda
passes its own (guest-userspace) VAs telling the driver "the mapping
moved from A to B".  The isolate has no such mapping, so the driver's
internal lookup fails.

**NV0080_CTRL_FIFO_GET_CHANNELLIST_PARAMS** has `PChannelHandleList`
and `PChannelList` (both `NvP64`) — output buffers the driver writes
back into.  gVisor handles this with a dedicated
`ctrlDevFIFOGetChannelList` handler that copies the lists in/out via
the sentry.  Our generic RM_CONTROL path doesn't intercept it, so the
driver page-faults dereferencing guest VAs in the isolate, returns
INVALID_DATA.

## Why this is the dual-mmap wall

Both failures are symptoms of the same architectural gap noted in
`docs/ARCHITECTURE.md`: **libcuda's host VAs are only valid in the
libcuda process; the isolate's mm has no equivalent mapping**.  Every
ioctl whose parameter struct embeds a userspace pointer hits this.
The InfoList family worked around it by detecting specific control
cmds and copying the pointed-to data through an aux-buf extension;
both blocker calls above need similar per-cmd handlers.

The principled fix is the dual-mmap install path (task #1 and #2):
the guest module's `nvkvm_mmap` materialises a real mapping in both
the guest userspace and the isolate at matching offsets so libcuda's
VAs resolve correctly on both sides.  Until that's wired, every new
class of ioctl will need a per-cmd workaround.

## Workarounds available (incremental, in order of effort)

1.  **Stub `NV0080_CTRL_CMD_FIFO_GET_CHANNELLIST`** by extending
    aux_buf with two u32[NumChannels] arrays, zeroing the pointers,
    letting the driver fill them, then copying the arrays back to the
    user buffer on return.  Same shape as the existing InfoList
    family handler in `nvkvm_main.c` around line 464.  Likely 1-2
    hours.
2.  **Skip / suppress `UPDATE_DEVICE_MAPPING_INFO` in the guest** —
    return success without forwarding.  The driver uses this purely
    to update its own bookkeeping; if our mmap path is going to
    replace it anyway, ignoring it might be acceptable.  Risky if
    libcuda relies on a specific return value later.
3.  Full dual-mmap path.  The right answer but multi-day work.

## What to try first

The FIFO_GET_CHANNELLIST handler is the natural next commit — it
follows a pattern we already have in the InfoList family, gVisor has
documented the exact algorithm, and the param struct is small (24
bytes outer + 2*NumChannels*4 inner).
