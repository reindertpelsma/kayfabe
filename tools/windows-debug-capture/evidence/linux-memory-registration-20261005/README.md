# Linux can request GSP registration through an unprivileged memory allocation

**STATUS: RESEARCH, 2026-10-05.** Native RTX 4070 AD104, NVIDIA open 595.91.07.
This tests a Linux caller for the source-defined path behind Windows I's refused
`ALLOC_MEMORY` RPC. It is not a Windows/Kayfabe workload result or forwarding approval.

Both an ordinary allocation and the same allocation requesting GSP registration
succeeded as UID/EUID 65534, with all five capability sets zero and
`NoNewPrivs: 1`. Each allocated 28 KiB of contiguous, write-combined system memory;
every allocation and explicit free returned `NV_OK`. The registration flag was
preserved in the returned attributes. See [native.log](native.log) and
[context.json](context.json) for the exact source/build/header identities.

| Statement | Evidence and limit |
| --- | --- |
| An unprivileged Linux process can successfully request this registration | Measured `NV01_MEMORY_SYSTEM` allocation with `NVOS32_ATTR2_REGISTER_MEMDESC_TO_PHYS_RM`, actual non-root identity and zero capabilities |
| That attribute leads to memory-registration RPC function 4 on a GSP client | The matching open-driver source calls `memRegisterWithGsp` from `memConstructCommon`; it constructs `NV01_MEMORY_LIST_SYSTEM` and calls `NV_RM_RPC_ALLOC_MEMORY` |
| The exact RPC bytes and PFN list in this run | **Not captured.** The kernel refused installation of the named kprobe with `EINVAL`, before the probe client ran; a later direct invocation performed the measured allocation |
| A guest may forward a raw privileged memory-list request to host RM | **Not authorized.** Direct `NV01_MEMORY_LIST_SYSTEM` allocation is marked privileged; guest PFNs and handles cannot become host PFNs or handles |

The source oracle is OGKM 595.91.07, commit
[`9f087a6d`](https://github.com/NVIDIA/open-gpu-kernel-modules/tree/9f087a6d4e86d85acc0ce1d354d6276fe5047b29):

- [`resource_list.h:537`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/nvidia/src/kernel/rmapi/resource_list.h#L537)
  admits `NV01_MEMORY_SYSTEM` without privilege. The list class at line 622
  requires privilege. These are different API entry points.
- [`system_mem.c:418`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/nvidia/src/kernel/mem_mgr/system_mem.c#L418)
  passes the requested attributes to `memConstructCommon`.
- [`mem.c:486`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/nvidia/src/kernel/mem_mgr/mem.c#L486)
  tests the registration attribute. `memRegisterWithGsp` checks firmware-client
  mode and object ownership, selects the memory-list class, converts flags and
  invokes the RPC. Its successful registration is paired with an RPC free when
  the memory resource is destroyed.
- [`rpc.c:3482`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/9f087a6d4e86d85acc0ce1d354d6276fe5047b29/src/nvidia/src/kernel/vgpu/rpc.c#L3482)
  provides the function-4 encoder and contiguous descriptor treatment.

This establishes an available Linux test stimulus, not that normal Linux desktop
initialization uses Windows' runlist setup, or that any earlier Linux capture
must contain the request. It also distinguishes unprivileged *origin* from a
safe raw-forwarding contract: Linux RM allocates and owns the native pages before
authoring the internal RPC. Kayfabe must validate and represent guest backing
within its own memory authority instead.

## Reproduce

Use a clean checkout of the pinned public source matching the tested driver.
The probe compiles declarations and field constants from those headers; it does
not copy ioctl structure offsets or use physical addresses supplied by a caller.

```bash
cc -std=gnu11 -O2 -Wall -Wextra -Werror \
  -I "$OGKM/src/common/sdk/nvidia/inc" \
  -I "$OGKM/src/nvidia/arch/nvalloc/unix/include" \
  -I "$OGKM/kernel-open/common/inc" \
  tools/windows-debug-capture/linux-memory-registration.c \
  -o /var/tmp/kf-memory-oracle
setpriv --reuid=65534 --regid=65534 --clear-groups \
  --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs \
  /var/tmp/kf-memory-oracle plain
setpriv --reuid=65534 --regid=65534 --clear-groups \
  --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs \
  /var/tmp/kf-memory-oracle registered
```

The recorded executable was built on the controller with the same options plus
`-static`, then sent to the PC. Only text evidence returned. All code is on GitHub.
The optional [scoped observer](../../trace-linux-memory-registration.py) installs
entry/return probes without changing existing trace instances, filters to the
test PID, and removes its own state. This host rejected the entry probe; the exact
cause is not established. [trace-attempt.log](trace-attempt.log) preserves the failure.

An initial probe omitted the required allocation-owner tag; both variants then
returned `NV_ERR_INVALID_OWNER` before registration. That attempt is preserved in
[initial-invalid-owner.log](initial-invalid-owner.log). The measured successful
revision sets the allocating client's handle as the owner, as existing Kayfabe
host allocation code does. This was a probe correction, not a driver workaround.
