# Diagnosing the cuInit blocker

## Why this exists

As of 2026-05-25 cuInit consistently returns `CUDA_ERROR_NO_DEVICE` (100) in our
guest, even though:

- `test_ioctl_fwd` passes all 48 checks.
- `cuDriverGetVersion` works (returns CUDA 12.9).
- Every RM ioctl the guest issues during cuInit returns `NV_OK` with
  `nvstatus=0x0`.
- The full init flow completes: REGISTER_FD on two nvidia0 fds, subdevice
  alloc, GR/BUS/FB info queries, NV_ESC_RM_MAP_MEMORY, NV_ESC_RM_ALLOC_OS_EVENT.

The behavioural diff from a working host cuInit:

| Step                          | Host (cuInit=0)              | Guest (cuInit=100)   |
|------------------------------|------------------------------|----------------------|
| RM_MAP_MEMORY                | succeeds                     | succeeds (same)      |
| `mmap(/dev/nvidia0)`         | **64 KB MAP_SHARED**         | **never called**     |
| `mmap(/dev/nvidiactl)`       | **4 KB MAP_SHARED**          | **never called**     |
| Remainder of init            | proceeds                     | cleans up, returns 100 |

Hypotheses already eliminated:

- `pLinearAddress == 0` triggering CUDA to skip mmap. We leaked the driver's
  real host VA in p_linear_address; CUDA still didn't call mmap and still
  returned 100.
- `virtualizationMode == 1` from NV0080_CTRL_CMD_GPU_GET_VIRTUALIZATION_MODE.
  Masked to 0; behaviour unchanged.
- Stale `test_ioctl_fwd` binary. Rebuilt; all checks pass.

So one (or more) of the response payloads that look syntactically correct in
our QEMU log is semantically wrong in a way CUDA notices. We need ground truth
from a host run to know which.

## Approach: byte-level ioctl capture, then diff

A ptrace-based tool runs a target binary and, for every `ioctl` syscall on a
`/dev/nvidia*` fd, records:

- `fd` and the device's path (`/proc/<pid>/fd/<fd>` readlink)
- the ioctl `cmd` number
- the syscall return value and `errno`
- for ioctls with an arg pointer: a hex dump of the buffer
  - *pre-call* (so we capture caller-supplied inputs verbatim)
  - *post-call* (so we capture driver-written outputs)
- the `cmd` field of `nvos54_parameters` for RM_CONTROL, and the `hClass`
  for RM_ALLOC, so we can group sub-commands

Run the tool on:

1. **Host** — `tools/diag/nvioctl_trace ./host_cu_test`. Driver succeeds;
   cuInit=0.
2. **Guest VM** — same tool, same binary (rebuilt inside the VM). cuInit=100.

Then diff the two structured logs. The first response field that diverges
between host and guest is, by elimination, what CUDA reads to decide there's
no device.

### Why ptrace and not LD_PRELOAD or eBPF

- **LD_PRELOAD on `ioctl`** doesn't work: NVIDIA's libcuda calls `syscall(SYS_ioctl, …)`
  via inline assembly. Verified earlier today — the preload constructor ran
  but no ioctl was intercepted.
- **eBPF/kprobe** on the nvidia kernel module is fine in principle but
  requires either bpftrace-with-CO-RE (which won't have type info for
  driver-internal structs) or a custom libbpf program. ptrace is portable
  across the host and the guest, doesn't require kernel-side privileges
  beyond what we already have, and is simpler to make robust.
- **Modifying the open-gpu kernel module** to add printk's would also work,
  but rebuilding+reloading the driver risks bricking the GPU on a remote
  instance.

### Output format

One record per ioctl, separated by blank lines, machine-parseable:

```
PID=12345 TID=12349 FD=8 PATH=/dev/nvidiactl CMD=0xc020462a RET=0
PRE  : 00 00 00 00 64 00 00 00 14 02 00 00 00 00 00 00 …
POST : 00 00 00 00 64 00 00 00 14 02 00 00 00 00 00 00 …
DELTA: ..........|......|......|...... …
```

`DELTA` marks bytes that changed; `.` for unchanged, `|` for changed. This
makes diffs immediately readable.

For RM_CONTROL we also annotate the inner cmd at offset 8 of nvos54_parameters
so different sub-commands group cleanly:

```
PID=… RM_CONTROL inner=0x20801201 hClient=0x… hObject=0x… RET=0
PRE  : …
POST : …
```

## After diagnosis

Once the offending field is identified, the fix path is one of:

- A specific inner cmd needs special handling (pointer translation,
  capabilities table generation, value rewrite). Add to QEMU's
  isolate-handlers along the same lines as GET_BUILD_VERSION and the
  InfoList family.
- An ioctl we're not even forwarding is needed. Add it.
- A whole-class problem (e.g. the response struct layout for one ioctl is
  different in driver 575.x than gVisor's vendored definition).

Document the finding in `docs/CUINIT_FIX_NOTES.md` and either fix in place
or open a follow-up task.

## Failure modes the tool itself must handle

- **Multi-threaded tracees.** libcuda spawns worker threads (we saw
  `cuda00001400006` in the strace output). The tracer must `PTRACE_SETOPTIONS
  | PTRACE_O_TRACECLONE` and follow forked/cloned threads. Entry/exit state
  is per-tid.
- **PTRACE_EVENT_CLONE etc.** Don't confuse `SIGTRAP|0x80` (syscall stops)
  with `SIGTRAP` + ptrace event message (clone notifications).
- **Signal injection.** Non-trap signals must be re-injected via the data
  arg of PTRACE_SYSCALL.
- **Reads from tracee memory after the tracee has died.** Guard the
  post-call read; if the tracee was killed by a signal during the ioctl, the
  pre dump alone is what we have.
- **Large arg buffers.** Cap dumps at 256 bytes — enough to cover all the
  RM_CONTROL/ALLOC structs we care about, small enough to keep logs readable.
