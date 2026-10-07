# kf3 host channels are born USER while QEMU runs as root (2026-10-03)

**STATUS: LIVE, 2026-10-03.** Branch `v3-sec-nonpriv`. This is a security-policy change, so the
owner reviews it before merge (OWNER_RULINGS §F).

## The property

Every host channel kf3 creates runs guest-authored GPU work: passthrough twins run guest
pushbuffers as written, and Translated rings run rewritten guest-kernel work. So each one must
be a `PRIVILEGE_USER` channel. A channel that host RM stamps `ADMIN` gets more than a user
channel does. For example, its GR context is given the *unrestricted* privileged-register
access map (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_context.c:3293-3297`), and
the only thing between it and physical-mode copy-engine operands is a flag enforced in closed
GSP firmware.

Host RM fixes a channel's privilege once, at the channel-alloc ioctl. It reads it from the
calling thread's `capable(CAP_SYS_ADMIN)`:

- `ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:277-291` decides the level;
- `src/nvidia/arch/nvalloc/unix/src/escape.c:304` sets the call's privilege from
  `osIsAdministrator()`;
- `kernel-open/common/inc/nv-linux.h:537` defines that as `capable(CAP_SYS_ADMIN)`.

The bench runs QEMU as root, so before this change every channel kf3 created was `ADMIN`.

## The mechanism and the tripwire

- **The mechanism: clear one bit for one call.** kf-host's `birth_member` makes the
  channel-alloc ioctl inside `kf_linux_raw::capability::with_effective_cap_cleared` (since the
  review: through `kf_host::birth::born_user`, the only path a channel class may take). That clears
  `CAP_SYS_ADMIN` from the calling thread's *effective* set, makes the call, and restores the
  set. The bit stays in the permitted set. The VMM's privileges, user and threads are otherwise
  unchanged; sandboxing the VMM is not kayfabe's job (owner, 2026-10-03). If the bit cannot be
  cleared, the birth is refused before any host call (`CAP_BRACKET_REFUSED`, `0x4B74`).
- **The tripwire: read RM's verdict.** `kf_host::channel::birth_privilege` reads the alloc
  reply. It refuses the birth by name and frees the channel (`PRIVILEGED_CHANNEL_REFUSED`,
  `0x4B73`) if either of these holds:
  - `NVOS04_FLAGS_PRIVILEGED_CHANNEL` (bit 5) is set;
  - the reply's `internalFlags` privilege is not `USER`.

  kf3 never asks for bit 5, and RM sets it for `ADMIN` and `KERNEL` channels only. So a set
  bit can only be RM's own verdict. Every birth logs one line with the reply flags.
- **Not the client class.** `NV01_ROOT_NON_PRIV` does nothing for a Linux userspace client.
  `escape.c:394-403` rewrites every userspace root allocation to `NV01_ROOT_CLIENT` before RM
  sees it, so `bIsRootNonPriv` (`rmapi/client.c:88`) is never set. `privprobe_root.log` shows
  this on the box.

## Box and revisions

- Box: vast 54050499, RTX 3060 (GA106).
- Host driver: open kernel module 580.159.04.
- QEMU: 10.2.4 with kf3, started by `scripts/fastguest/run_fast_guest.sh` as the bench does
  today, as **root**. Each run log records the QEMU process's `Uid: 0` and
  `CapEff: 000001ffffffffff`.

## Files

| file | revision | what it shows |
|---|---|---|
| `privprobe.py`, `privprobe_root.log` | (no kf3) | As root, root clients of class `NV01_ROOT`, `NV01_ROOT_NON_PRIV` and `NV01_ROOT_CLIENT` all come back as class `0x41`. All three report `GET_PRIVILEGED_STATUS` (`0x135`) = `0x5`: admin under `rmclientIsAdmin`. With `CAP_SYS_ADMIN` cleared from the calling thread's effective set, every client, including ones created as admin, reports `0x0`. Restoring the bit brings back `0x5`. The decision is per call and per thread. |
| `before_measure_root_run.log`, `before_measure_root_qemu_sec_p0.log` | `3e0f6dee` (branch `v3-sec-p0`) | **BEFORE.** Copied from `traces/v3_security/p0_20261003/` in that branch's worktree (`measure_root_*`). `--probe-launch-dma` with QEMU as root: 6 channel births, all with reply flags `0x004000a0` and `PRIVILEGED_CHANNEL=1`. That branch also asked for bit 22 and ran with its own diagnostic override; only its readback is reused here. |
| `after_run.sh` | — | Wrapper for the two runs below. It records the QEMU process's uid and capabilities, then runs `run_fast_guest.sh` with `KF_ARMS=--probe-launch-dma`, budget 180 s. |
| `after_root_run.log`, `after_root_qemu.log` | `dc64b22b` | **AFTER.** The same arm, QEMU as root. ⊘ CORRECTED 2026-10-03 (adversarial review): "same arm" hid a difference. `after_run.sh` sets `KF_ARMS=--probe-launch-dma`, and `run_fast_guest.sh` appends the probe token again, so this run's arm list is `--probe-launch-dma,--probe-launch-dma` (the log's `== arms:` line), where BEFORE ran it once. The births match: the same 6 (engines `0xb, 0xb, 0x9, 0x1, 0x9, 0x9`). Every reply reads `0x00000080 PRIVILEGED_CHANNEL=0 privilege=USER cap_sys_admin=cleared-for-call`. `FAST_VERDICT=PASS`. |
| `negctl_root_run.log`, `negctl_root_qemu.log` | `dc64b22b` | **Negative control** (arm list `--probe-launch-dma,--probe-launch-dma`, as AFTER). `KF3_NEGCTL_SKIP_CAP_BRACKET=1` skips the bracket. RM's reply to the first birth is `0x000000a0` (bit 5 set). The tripwire refuses it by name and frees the channel, and the guest's client then fails (`FAST_VERDICT=FAIL`). This is the expected result: it shows the check reports a set bit from a live reply. The knob can only make births fail. |

**Merge bar** at `55743ecd` (`merge_bar/README.md`), run as root with QEMU started the bench's
usual way:

- every kf-* crate test: 1764 passed, 0 failed;
- v3 gates: 9/9;
- bare-metal suite: 30/30;
- 30-arm thin-guest suite: 30/30.

All 161 channel births in the suite and all 11 in the gates read `PRIVILEGED_CHANNEL=0`; none
was refused. (That count was a manual grep. Since the review, `merge_check.sh` gates on it; the bar
at `1d71f3db`, with the census, is `../merge_bar_1d71f3db/`.)

## What this does not cover

- ⊘ **CORRECTED later on 2026-10-03: libcuda-owned channels are now covered** (see
  `../libcuda_20261003/`). This entry read: *"The walker (C2) and display (C3) libcuda contexts
  create their own channels on threads that hold the VMM's capabilities. This bracket and
  tripwire do not cover them."* kf-cuda now clears `CAP_SYS_ADMIN` from each thread's effective
  set for its life before the thread's first CUDA call (`kf_cuda::posture`), and kf3 runs
  realize's CUDA calls on threads of their own.
- **Other open findings from the same 2026-10-03 audit.** These are separate tasks and are not
  addressed here:
  - the identity windows mapped into user twin address spaces;
  - Translated rings sharing the guest kernel's twin address space.
