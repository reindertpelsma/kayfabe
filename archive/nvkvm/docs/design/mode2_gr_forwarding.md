# Mode-2 Phase B: forward GR/compute to a real host GPU (host parity)

> **Rewrite note (2026-07-24):** the Rust rewrite's execution/forwarding plane — the
> `Engine`/GR-context/pushbuffer/completion/forward abstractions designed *into* the core
> — is specified in the rewrite repo at `nvkvm-rs/docs/design/execution_plane.md` (which
> mines this doc + `mode2_compute_forwarding.md`/`mode2_forwarding_model.md`/
> `mode2_address_table.md`). The rewrite repo owns its design docs going forward; this
> doc remains the C-era ground truth for the forward-not-emulate approach.

Decision (user, 2026-06-03): **do B.** Pure-emulation Mode-2 is complete through
boot + GSP-RPC + BAR2/GMMU + CE + interrupts; it stalls at the GR (compute) engine
golden-context bootstrap because GR state is produced by FECS/GPCCS microcode on
real silicon. Phase B runs GR/compute on a real host GPU while the guest keeps
running its stock NVIDIA driver against the emulated device. Goal: CUDA at host
parity. QEMU stays UNPRIVILEGED.

## Premise & the lucky alignment
- Host (vast `vh`) has a real **GA106** (RTX 3060) + nvidia 580 driver.
- We advertise the guest as **GA106** too → same chip, same context/golden format,
  same class IDs. Driver versions must match (guest 580.159.04 ↔ host 580.x).
- The address-translation core (nvkvm_walk_pdb: guest GR-VA → GPA/FB, aperture-aware,
  2M/64K/4K) and channel tracking (gpfifo/USERD/instblk/doorbell) are already built.

## Architecture: DIRECT-MAP FIRST (user steer 2026-06-03), not trap-and-replay
Decision principle (user): the production hot path is **userspace-mmap-only** — a
CUDA app mmaps the channel USERD + work-submit doorbell and writes them directly
(no kernel, no trap). So **trap-and-emulate of submissions is throwaway AND
non-performant**; do NOT build it. Test for every GPU range: "would the guest
kernel mmap this into userspace?" If yes (USERD, doorbell, semaphore surfaces) →
**direct-map it to the real host-GPU MMIO/memory from the start.**

The real axis is EMULATE vs FORWARD, not trap vs mmap:
- FORBIDDEN — a GR/compute METHOD *emulator* (execute GR pushbuffers in QEMU).
  THIS is the throwaway + non-performant thing. Never build it.
- FINE & CHEAP — trap-to-FORWARD: trap the doorbell write and forward the token to
  a real host channel. ~20 lines, NOT throwaway: it's a thin shim over the SAME
  host-channel infra that direct-map also needs. Good correctness-first step.
- PARITY FINISH — direct-map: forwarded-mmap the host doorbell/USERD MMIO into the
  guest BAR so the guest writes it directly (no per-submit trap). Removes the only
  cost of trap-to-forward (the per-submit trap+RTT latency).
- BRING-UP ONLY — fake a one-time KERNEL-channel completion (CE scrubber, golden).
  Not the hot path, not mmap'd; a ~10-line stepping stone to get the driver loaded.

So the THROWAWAY risk is EMULATION, not trapping. The investment order:
  1. Shared infra (the real bulk, needed either way): a real host channel whose
     GPFIFO/pushbuffer = the guest's (guest RAM mapped into the host VAS via
     OS-descriptor), so the host GPU runs the guest's actual work.
  2. trap-to-forward the doorbell → CORRECTNESS (cheap, reuses step 1).
  3. direct-map doorbell/USERD → PARITY (removes per-submit trap).
  4. NEVER emulate GR methods.
Step 1 dominates the effort and is identical for trap-forward and direct-map;
2→3 is a small late swap, so we are not throwing work away by doing 2 first.

Model: "host-backed channel, guest-driven, direct-mapped":
- QEMU (emulated device) ↔ UNPRIVILEGED host helper (reuse the Mode-1 stub: holds
  real RM client/device/GR-context fds, issues host nvidia ioctls; QEMU never
  touches /dev/nvidia*).
- At the guest's channel/context ALLOC (GSP_RM_ALLOC), allocate a REAL host-GPU
  channel/context and arrange that the regions the guest RM will mmap to userspace
  (USERD, doorbell/work-submit, semaphore) are **backed by the host channel's real
  MMIO/memory** — so the guest userspace mmap resolves to host HW (forwarded mmap,
  Mode-1 style). Guest doorbell write → real host doorbell → real HW. No trap, no
  per-submit replay, parity.
- Data stays in **guest RAM (GPAs)**; map guest RAM into the host context VAS
  (RM OS-descriptor of the guest-RAM HVA — Mode-1, unprivileged). guest GR-VA →
  GPA → host-GPU-VA; host GPU reads kernels/inputs + writes outputs into guest RAM.
- The hard part (and the core of Phase B): Mode-2's guest RM CHOOSES its own
  channel layout against the emulated device, so we must INTERCEPT the channel /
  USERD / context-buffer allocs and substitute host-backed memory, so the
  addresses the guest mmaps line up with host resources. This replaces the
  submission-time replay layer the earlier draft leaned on.

## Golden context: content doesn't matter for BOOT
Critical simplification: the guest's golden-context buffer content is only USED at
context-RESTORE time (when a context actually runs on GR). We forward all real GR
execution to the host (whose own golden context is valid), so **the guest never
runs its own GR engine** — its golden buffer can be garbage. For BOOT we only need
to SIGNAL the golden-capture COMPLETION the driver's 4s poll waits on (same shape
as the CE semaphore fix). So B1 = signal the GR ctxsw/golden completion; we do NOT
need to produce a real golden image.

## Incremental phases (each testable)
- **B0 — design + transport** (this doc). Transport decision: reuse the Mode-1
  unprivileged stub as the host RM proxy (it already forwards RM ioctls); add a
  QEMU↔stub control path for Mode-2 GR ops. (Confirm the stub can be driven from
  the QEMU device process unprivileged.)
- **B1 — golden completion → RmInitAdapter SUCCEEDS** (immediate blocker): find the
  exact signal the golden-context capture's 4s `_threadNodeCheckTimeout` polls
  (after the GR-object GSP_RM_ALLOC on the golden channel) and provide it. Likely a
  GSP-RPC reply field, a notifier/semaphore beyond the channel sema, or a FECS
  status the driver reads. Then the stock driver fully loads + `nvidia-smi`
  enumerates. (No host GPU needed yet — boot-only.)
- **B2 — address bridge**: map guest RAM (a GR context's GPAs) into a host GR
  context's VAS via the stub (OS-descriptor). Prove the host GPU can read a guest
  buffer and write a result back into guest RAM (a host-side memcpy via CE on the
  host channel, verified by the guest reading the result).
- **B3 — first forwarded compute**: replay a real compute pushbuffer (cuLaunchKernel
  → SET_OBJECT(compute) + kernel launch methods) from the guest's GR channel onto
  the host channel; verify kernel output in guest RAM. First real CUDA kernel.
- **B4 — parity hot path**: direct-map the GR channel's USERD/doorbell (host GPU
  MMIO via stub forwarded mmap into guest GPA) so submissions hit real HW with no
  trap (per docs/design/mode2_memory_model.md). Removes the per-submit replay tax.
- **B5 — matrix**: run the 20-app compute/graphics matrix at host parity.

## Open questions to resolve as we go
- Exact GR golden-capture completion signal (B1 — investigate next).
- Stub interface for Mode-2 (raw RM alloc/control/map/submit) vs a new helper.
- Context-buffer/VAS ownership: does the host context use the guest's context
  buffers (mapped) or its own (then we bridge only data buffers)? Start with host
  owning its context, guest data buffers mapped in.
- Driver-version exact match (guest vs host) for class/ABI parity.

## Reuses
- nvkvm_walk_pdb / nvkvm_chan_translate (VA→phys), channel tracking, doorbell,
  the GSP-RPC shim, the CE method parser (extend for GR/compute classes), and the
  Mode-1 stub (unprivileged host RM proxy + OS-descriptor guest-RAM mapping).

## Security: host-side calls reuse the Mode-1 hardened stack (user requirement, 2026-06-03)
No new ioctls are introduced by Mode-2. The guest issues its own ioctls to its own
stock nvidia.ko inside the guest (a real driver — nothing to allowlist there). The
reverse driver's HOST-side calls (allocating the real host GR channel/context,
OS-descriptor of guest RAM, RM controls whose response data we replay, forwarded
mmaps of USERD/doorbell) are the SAME RM ioctls Mode-1 already forwards. They MUST
go through the identical hardened path Mode-1 uses — never a new unvalidated route:

    emulated device (nvkvm_gpu_emul.c)
      -> nvkvm_dispatch.c        (size / _IOC_SIZE / fd-presence validation)
      -> nvkvm_isolate_handlers.c (the sanitizer: fd->handle rewrite, struct field
                                   validation, prot/flag allowlist, alloc-class
                                   allowlist nvkvm_fe_alloc_allowlist.h, OOB-read
                                   kill-switch max(param_size,_IOC_SIZE))
      -> unprivileged stub (nvkvm_stub.c) -> host /dev/nvidia* ioctl

QEMU stays unprivileged; the stub stays in its rootless/namespaced/cap-less
sandbox (nvkvm_isolate.c). Concretely: when nvkvm_gpu_emul needs a host RM op it
hands a request to the dispatch/isolate layer (same struct path as a guest-
originated ioctl), so every host call inherits Mode-1's arg/size/struct/fd
validation for free. Implementation rule: do NOT add a side-channel that calls the
stub or /dev/nvidia* directly from the emulated-device code — always go via
nvkvm_dispatch so the sanitizer runs. This keeps the cross-VM/host boundary exactly
as hardened as Mode-1 (see security audits) while adding compute forwarding.
