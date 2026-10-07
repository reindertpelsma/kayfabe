# cuInit "no device" blocker — host vs gVisor vs nvkvm comparison

## RESOLVED 2026-05-26

cuInit now returns 0 and reports 1 GPU.  See [[cuinit-first-success]].
The remaining work is on cuCtxCreate / cuMemAlloc, summarised at the
end of this document.

This is a forensic breakdown of where each implementation diverges on the
UVM call sequence that cuInit issues during device probe.

## Current status (2026-05-26 — late)

cuInit still returns 100, but the *reason* now is several layers deeper
than at the start of the day.  Today's commits, in order, each unblocked
one concrete failure mode:

1. **hClass=0xDE size (RM_USER_SHARED_DATA)** — driver wants 8 bytes
   (NV00DE_ALLOC_PARAMETERS_V545); we were sending 0.  cuInit moved from
   "no device" (100) → 999 (unknown error).
2. **IOCTL NR collision** — sanitize/dispatch matched `_IOC_NR(cmd)` for
   frontend ioctls but UVM_PAGEABLE_MEM_ACCESS (0x27) ==
   NV_ESC_RM_ALLOC_MEMORY (0x27).  The 8-byte UVM struct was read as a
   48-byte nvos02-with-fd, garbage interpreted as fd → EBADF.  Fix: gate
   the NR switches on `_IOC_TYPE == 'F'`.  cuInit reached UVM init and
   the first RM_MAP_MEMORY.
3. **hClass=0x90F1 size (FERMI_VASPACE_A)** — same shape as 0xDE: missing
   from the size fallback table.  Added NV_VASPACE_ALLOCATION_PARAMETERS
   (48 bytes, pre-V580 layout).  cuInit reached an even deeper class.

## Next concrete failure (today's stopping point)

Two issues blocking the next step, both surfaced by the new RM_ALLOC
hex-dump diagnostic:

### a) NV50_MEMORY_VIRTUAL (hClass=0x50A0) — embedded pointer

Adding the size (128 bytes, NV_MEMORY_ALLOCATION_PARAMS_V545) lets the
alloc reach the driver, which then dereferences the embedded `address`
field (NvP64 at offset 104) in the *isolate* process and hangs the
calling thread (kernel hung-task warning in dmesg).  CUDA fills this
with a libcuda-process VA that means nothing to the isolate's mm.

**Fix needed:** per-class aux sanitizer that zeros `address` before
forwarding, then writes it back on return (or installs it via the
GPA window so the value the driver writes is actually visible to
libcuda).  Same story for the other classes that share this struct.

### b) RM_MAP_MEMORY embedded fd not translated in stub

Earlier "this worked" was luck — `fd_token` happened to match an unused
isolate-side fd number.  Now that more handles are opened first,
`fd_token=25` is passed into the kernel and rejected with
NV_ERR_INVALID_ARGUMENT (0x1F).  The legacy QEMU dispatch translated
fd_token → host fd; the isolate path doesn't.

**Fix needed:** stub-side translation of the embedded fd in
`nv_ioctl_nvos33_parameters_with_fd` (and the same for
`nv_ioctl_nvos02_parameters_with_fd` once 0x50A0 is wired in).

Both are tractable now that the diagnostic dumps the struct
contents.  RM_MAP_MEMORY's eventual install path (host VA →
GPA-window region) is the broader dual-mmap architectural work
described in `docs/ARCHITECTURE.md`.

---

## Original analysis below (historical)

After stripping the install_isolate_mapping path and the
UVM_MM_INITIALIZE mask, cuInit still fails with code 100
(CUDA_ERROR_NO_DEVICE). All ioctls succeed individually:

  UVM_INITIALIZE      rm_status=0x0
  UVM_MM_INITIALIZE   rm_status=0x10006  (NV_WARN_NOTHING_TO_DO,
                                          expected on this driver build,
                                          libcuda handles as success)
  UVM_PAGEABLE_MEM_ACCESS  rm_status=0x0
  ...followed by ~60 successful RM_CONTROL / RM_ALLOC / RM_FREE...
  RM_MAP_MEMORY (0xc038464e)  ret=0  nvstatus=0x0
  UVM_DEINITIALIZE  rm_status=0x0
  (libcuda gives up)

The host strace from the same libcuda version, on the same driver, does
**not** stop after RM_MAP_MEMORY — it goes on to UVM_REGISTER_GPU_VASPACE,
UVM_REGISTER_GPU, then `mmap(/dev/nvidia0, 65536, PROT_WRITE, MAP_SHARED, fd, 0)`
to actually map the GPU memory into the calling process's mm.

Our guest never gets to that mmap.

**Hypothesis updated 2026-05-26.** Added pLinearAddress logging to QEMU
dispatch (commit pending). Output:

  nvkvm: RM_MAP_MEMORY response: pLinearAddress=0xc0bb0000 status=0x0

That's a *low 32-bit* value, **not** a stub-mm VA (which would be 0x7f…
range). So the initial "stub VA leaks out" framing was wrong.

What 0xc0bb0000 actually is: likely a GPU-side BAR offset / handle the
kernel writes back, intended as the `offset` argument to a subsequent
`mmap(/dev/nvidia0, …, offset=0xc0bb0000)` that materialises the
mapping in the calling-process mm.

Strace shows libcuda **does not call mmap** — it `close(14)` (the
nvidia0 fd it was about to mmap on) immediately after RM_MAP_MEMORY
returns, then proceeds through cleanup RM_FREEs and exits. So libcuda
is reading something in the response (or in a prior RM_CONTROL response)
that tells it the device isn't usable.

Candidates worth diagnosing next:
- Compare the full nvos33_parameters response byte-by-byte to host
  (status, flags, fd, reserved fields, length).
- Earlier RM_CONTROL responses for capability/topology queries that
  may have returned values libcuda treats as "device not viable" —
  NV0080_CTRL_CMD_GPU_GET_CLASSLIST, NV2080_CTRL_CMD_GPU_GET_INFO,
  NV2080_CTRL_CMD_BUS_GET_INFO, NV2080_CTRL_CMD_FB_GET_INFO.
- Whether RM_ALLOC_MEMORY (NR=0x27 frontend, not UVM_PAGEABLE_MEM_ACCESS
  which is also 0x27 in UVM-space) returned correctly — we may be
  showing libcuda a "0 memory size" GPU.

## Pointer #2: RM_ALLOC with hClass=0xde fails

Added a second diagnostic (commit pending) that prints hClient /
hParent / hObjNew / hClass for any RM_ALLOC that returns non-zero
nvstatus.  Observed output:

  nvkvm: RM_ALLOC failed: hClient=0xc1d00a2a hParent=0x5c000003
         hObjNew=0x5c000006 hClass=0xde nvstatus=0x1f

Class **0xde is `RM_USER_SHARED_DATA`** (per
`gvisor/pkg/abi/nvgpu/classes.go:61`), a newer-driver class for an
RM↔userspace shared-data region.  The driver returns
`NV_ERR_INVALID_ARGUMENT` (0x1f) — almost certainly because the
allocation params (nvos21 or nvos64 alloc_params extension) for this
class are wrong / missing / have the wrong size.

**This is the concrete fix-target.**  Investigation:
1. Find what `NV00DE_ALLOC_PARAMETERS` (or similar) the driver expects
   for hClass=0xde.  Open-source `kernel-open/nvidia/src/kernel/rmapi/`
   in the driver source.
2. Add the param struct to `src/abi/nvgpu.h`.
3. Make sure the guest module's sanitizer + alloc_params_size table
   handles this class (the [[nvos64-abi-fix]] pattern: when CUDA leaves
   alloc_parms_size at 0, we must fall back to the right per-class
   size).
4. Re-run cuinit_test.

If libcuda treats RM_USER_SHARED_DATA allocation as fatal (likely —
the post-CUDA-12 driver pretty much requires this region for any
context), this single ABI bug could be the root cause of "no device."

The dual-mmap architectural refactor in `docs/ARCHITECTURE.md` is still
needed to actually expose RM_MAP_MEMORY's region to the guest after this
ABI question is settled.

## The architectural fix needed

This is exactly the problem the original `install_isolate_mapping`
design was trying to solve, before we discovered KVM strict-mm made the
stub-side syscall impossible. The right design, now understood:

1. Stub does the nvidia ioctls (including RM_MAP_MEMORY)
2. When RM_MAP_MEMORY returns, stub does its own `mmap(nvidia0, ..., offset)`
   to materialize the mapping in stub's mm
3. Stub sends "I mapped this fd at offset O, size S" to QEMU via the
   existing socket
4. QEMU mmaps the **same** nvidia0 fd (received from stub via SCM_RIGHTS)
   at any QEMU VA — both processes share the underlying physical pages
   because it's the same struct file
5. QEMU calls `KVM_SET_USER_MEMORY_REGION(QEMU_VA → GPA)`
6. QEMU returns the **GPA** to the stub
7. Stub returns the GPA (disguised as a VA, since libcuda will just
   `mmap(nvidia0, MAP_FIXED, ..., offset=ret_VA)` on it) to libcuda via
   the ioctl response
8. Guest module's `mmap(nvidia0)` then maps that GVA → GPA in the guest
   userspace mm

That collapses the install_isolate_mapping idea into the regular mmap
path, with QEMU doing the KVM region call (which is correct re mm
strict-equality), and the stub still owning the ioctl path.

This is a multi-day refactor — see `docs/ARCHITECTURE.md` "Known wrong"
items 1–4 for the full delta.

## Original analysis (preserved for reference)

## The sequence libcuda issues (verified via strace)

For driver 575.51.03 + CUDA 12.9 libcuda, the relevant tail of the cuInit
syscall trace is identical on every implementation up to UVM:

```
openat("/dev/nvidia-uvm", O_RDWR|O_CLOEXEC) = 9     # primary UVM fd
openat("/dev/nvidia-uvm", O_RDWR|O_CLOEXEC) = 10    # secondary (MM tracker)
ioctl(9,  _IO(0, 0x01), &p_init)                    # UVM_INITIALIZE
ioctl(10, _IO(0, 0x4b), &p_mm_init)                 # UVM_MM_INITIALIZE
ioctl(9,  _IO(0, 0x27), &p_pageable)                # UVM_PAGEABLE_MEM_ACCESS
ioctl(9,  _IO(0, 0x25), ...)                        # UVM_REGISTER_GPU_VASPACE
ioctl(9,  _IO(0, 0x17), ...)                        # UVM_REGISTER_GPU
```

cuInit gates `cuDeviceGetCount > 0` on `UVM_REGISTER_GPU` succeeding,
which requires `UVM_PAGEABLE_MEM_ACCESS` to have run on a properly
initialised primary UVM fd.

## What the kernel UVM driver actually checks

From `/usr/src/nvidia-575.51.03/nvidia-uvm/uvm.c::uvm_api_mm_initialize`:

```c
uvm_file = fget(params->uvmFd);                     // primary fd
if (!uvm_file_is_nvidia_uvm(uvm_file)) → INVALID_ARGUMENT
if (uvm_fd_type(uvm_file, ...) != UVM_FD_VA_SPACE)  → INVALID_ARGUMENT
if (!uvm_va_space_mm_enabled(va_space))             → WARN_NOTHING_TO_DO  (0x00010006)
```

`uvm_va_space_mm_enabled` returns false if:
- the va_space was opened with `UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE`, **or**
- `UVM_CAN_USE_MMU_NOTIFIERS() && uvm_enable_va_space_mm` is false at the
  system level (controlled by a kernel conftest at module build).

`UVM_PAGEABLE_MEM_ACCESS` is wrapped by `UVM_ROUTE_CMD_STACK_INIT_CHECK`,
which fails the call to NV_ERR_ILLEGAL_ACTION (in `params.rmStatus`, not
as syscall errno) if `uvm_fd_va_space(filp)` returns NULL — i.e. if the
filp wasn't UVM_INITIALIZE'd on this exact open.

The kernel UVM ioctl handler never returns `-EBADF` itself; the only
syscall errnos it emits are `-EFAULT` (copy-from/to-user) and `-EAGAIN`
(power-management trylock).

## Host (vanilla)

- libcuda's process opens both UVM fds, calls all three ioctls.
- File ownership: same process, same mm — passes `current->mm == file->mm` checks.
- `UVM_INITIALIZE` succeeds → va_space attached to fd 9.
- `UVM_MM_INITIALIZE` returns NV_WARN_NOTHING_TO_DO (0x10006) because
  this driver build has `UVM_CAN_USE_MMU_NOTIFIERS() = 0`. libcuda
  treats the warning as "MM FD not needed" and continues per the
  driver's documented contract.
- `UVM_PAGEABLE_MEM_ACCESS` succeeds because the same filp (fd 9)
  has a va_space; `params.pageableMemAccess` is set to false (no MM
  tracking) but rmStatus is NV_OK.
- cuInit proceeds; device is enumerated.

## gVisor nvproxy (`pkg/sentry/devices/nvproxy/uvm.go`)

- Everything runs in the sentry process. The application's "fd 9" is a
  gVisor virtual fd that wraps a host fd that the sentry opens at
  `/dev/nvidia-uvm`.
- `uvmMMInitialize` translates `params.UvmFD` from the sentry's
  fd-table token to the corresponding host fd, then invokes the host
  ioctl on the same sentry process.
- Owning mm is the sentry process's mm for both fds; same task calls
  all UVM ioctls.
- Same NV_WARN_NOTHING_TO_DO observed; libcuda handles it.

## Our nvkvm setup (before today)

- QEMU opens `/dev/nvidia-uvm` and passes the fd to the isolate via
  `SCM_RIGHTS`. The file's owning mm is **QEMU's mm**, not the
  isolate's.
- `UVM_INITIALIZE` succeeded (the macro is `NO_INIT_CHECK`, no mm check).
- `UVM_MM_INITIALIZE`: kernel does `fget(params->uvmFd)` in the
  isolate's task → gets QEMU-owned file → `current->mm` mismatch
  internally → returns **0x1f (NV_ERR_INVALID_ARGUMENT)**.
- We masked 0x1f to 0 in QEMU dispatch so libcuda would continue, but
  the kernel UVM driver's internal va_space state for the primary fd
  is still incomplete because MM_INITIALIZE didn't fully establish
  the mm binding.
- Subsequent `UVM_PAGEABLE_MEM_ACCESS` would set
  `params.rmStatus = NV_ERR_ILLEGAL_ACTION` (still ret 0), and libcuda
  fails downstream.

## Our nvkvm setup (after stub-local UVM opens)

- The stub now `openat("/dev/nvidia-uvm")` itself, before seccomp, so
  the file's owning mm matches the stub task.
- `UVM_INITIALIZE` succeeds: va_space attached.
- `UVM_MM_INITIALIZE` returns **0x10006 (NV_WARN_NOTHING_TO_DO)** —
  same as host! The kernel sees both fds in the same mm and just says
  "MM FD not needed."
- We short-circuit `UVM_MM_INITIALIZE` entirely in QEMU now (don't
  forward to the kernel) so the driver state isn't perturbed.
- **`UVM_PAGEABLE_MEM_ACCESS` returns intermittently 0 OR -1 EBADF.**

## Where the intermittent EBADF comes from

This is the open puzzle. Verified by strace + dmesg side-by-side on
the same build:

- The strace ioctl(9, 0x27) returns -1 EBADF, but `nvkvm_ioctl_unlocked`
  (our guest kernel module's `unlocked_ioctl` handler) is **never
  entered** for that call — its `pr_warn` doesn't fire even though it
  does fire for `UVM_INITIALIZE` and `UVM_DEINITIALIZE` on the same fd
  in the same run.
- fd 9 is **not closed** between the calls (no `close(9)` in strace,
  and the subsequent `UVM_DEINITIALIZE` on fd 9 succeeds).
- The error must therefore be `filp->private_data == NULL` at the
  start of `nvkvm_ioctl_unlocked` (line 389 returns `-EBADF` exactly
  there), reached without our `pr_warn` because that's placed after
  the null check.
- Why `private_data` would intermittently be NULL after a successful
  `open()` (which sets it on the `done:` label and always returns 0)
  is the next thing to verify. Suspects, in order:
    1. A use-after-free where `release()` runs and clears
       `private_data` while another fd still holds the file. The two
       UVM opens share an inode; if our `release()` is wired to the
       inode rather than the file, closing the secondary fd would
       free the primary's ctx. (Most likely.)
    2. The isolate's `nvkvm_session` is torn down by a concurrent
       isolate-kill path, which frees ctx through the session.
    3. A virtio response from QEMU racing the UVM call's setup
       overwrites the slot.

## Symbols that confirm the diagnosis above

Run sequence (multiple back-to-back invocations):

```
run 1: cuInit FAILED: 100 (no device — UVM_PAGEABLE_MEM_ACCESS = 0)
run 2: cuInit FAILED: 999 (UVM_PAGEABLE_MEM_ACCESS = -EBADF)
run 3: cuInit FAILED: 999
run 4: cuInit FAILED: 100
run 5: cuInit FAILED: 100
```

dmesg for the 999 runs has UVM_INITIALIZE + UVM_MM_INITIALIZE + UVM_DEINITIALIZE traces only. UVM_PAGEABLE_MEM_ACCESS never reaches our module.

dmesg for the 100 runs has all four UVM ioctls traced.

## Next step

Add a check at the top of `nvkvm_ioctl_unlocked` that logs `filp` /
`ctx` (and the calling tgid) when ctx is NULL, plus instrument
`nvkvm_release` to log which fd it's releasing. That should
positively identify the use-after-free path.
