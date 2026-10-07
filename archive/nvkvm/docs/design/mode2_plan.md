# Mode-2 "Reverse Driver" — Implementation Plan (Phase 1)

Status: PLAN. Kicked off 2026-06-03 after the attestation spike returned **GO**
([[mode2_attestation_spike_GO]], `docs/research/mode2_attestation_spike.md`).

## Why Mode-2 (and why now)

Mode-1 forwards guest NVIDIA ioctls to the real driver running on the **host**;
the guest has only a fake DRM head. That split-brain is the root of every
display headache we hit (wrong-card rendering, scanout freezes, nvidia-modeset
coupling, no early-boot/text-VT target) and caps us at "Linux guest with our
guest module."

Mode-2 inverts it: the guest runs the **real, stock NVIDIA driver** against an
**emulated NVIDIA GPU** we present from QEMU. We "fake the boot" (never run a
real GSP), translate the addresses the driver programs, and forward real compute
to the host GPU via the existing Mode-1 core. Consequences:

- **Any stock OS** (Linux open *or* closed driver; Windows later) — no guest agent.
- **Display is free**: the emulated GPU exposes a normal framebuffer/VGA, so
  BIOS/grub/plymouth/text-VT/desktop all scan out trivially at every stage.
- The spike proved the **one** existential risk (silicon attestation) is a
  non-issue for GeForce in default mode: the driver verifies no silicon secret;
  all bring-up gates are software-mirrorable register/mailbox/RPC values.

The Mode-1 present/console/readback work (commit `eaf90fc`) is **mode-agnostic**
and reused as Mode-2's host-side display sink — not stranded.

## Definition of done (north star, user 2026-06-03)

Mode-2 is "done" when it passes the **same** acceptance suite Mode-1 already
passes, at **host parity**, with the stock driver and no guest agent:
- the 22-app real-application matrix (`tests/perf/run_matrix.sh`: PyTorch
  CNN/ViT/BERT, HPC GEMM/FFT/nbody, crypto, gpu-burn, 7B LLM, Vulkan compute,
  OpenGL render) — [[realapp_matrix_done]];
- the host-vs-guest parity harness (`tests/perf/run_parity.sh`: GEMM/LLM/DMA
  within a few % + byte-exact) — [[parity_harness_next]];
- graphics (`run_graphics.sh`) and the desktop present path — [[present_path_b_done]].
These already exist for Mode-1, so Mode-2 reuses them verbatim; the only
difference is the device front end. Each milestone below is a step toward
re-running that suite green. M5 = first app passes; the full matrix at parity is
the finish line.

## Architecture

```
   guest (stock NVIDIA driver, unmodified)
        │  MMIO / config / DMA to emulated GPU
        ▼
   QEMU: emulated NVIDIA GPU PCI device  ── nvkvm_gpu_emul.c (NEW)
     ├─ PCI config: vendor 0x10DE, real device ID, class 0x030000, MSI-X
     ├─ BAR0  register aperture (MMIO)  → boot-register state machine (checks #1–#7)
     ├─ BAR1  FB/aperture window        → address-virtualization layer
     ├─ GSP-RPC endpoint                → consumes NV_VGPU_MSG_* from sysmem rings,
     │                                     posts responses (GSP_INIT_DONE first)
     └─ address-virtualization layer    → record every GPA/GPU-phys/bus addr the
                                           driver programs; translate → host/stub
        │  shim RPCs + translated memory ops
        ▼
   Mode-1 core (stub + ioctl forwarding)  → REAL host GPU (compute executes here)
```

Reference chip: **GA106 (RTX 3060)** to match the host GPU, so the HAL the
driver selects, the register family (ampere/ga102 `swref`), and the downstream
forwarding all describe the same silicon. Register defs live in
`research_clones/ogkm/src/common/inc/swref/published/ampere/ga102/`.

Driver to target first: the **open** kernel modules (instrumentable, register
defs public). Closed Linux driver and Windows are later (same trust model per
the spike; different exact poll/RPC set).

## Phases & milestones

`fake-the-boot` (M0–M3) needs **NO real GPU** — pure emulation. So it escapes the
singleton GPU-host serialization ([[orchestration_model]]) and iterates fast on
any KVM box.

- **M0 — Driver probes our device (observe).** Emulated PCI device with a BAR0
  that logs every register access; boot a guest with the stock open driver bound
  (matches on PCI ID). Deliverable: the exact `RmInitAdapter` register-access
  trace up to first stall + an annotated "what the driver wants" map. No
  responses yet beyond PCI enumeration + `NV_PMC_BOOT_0` chip ID.
- **M1 — Reach GSP bootstrap.** Answer checks #1 (`GFW_BOOT`=COMPLETED + PLM
  lowered) and #2 (`HWCFG2._RISCV`=ENABLE) so the driver identifies the chip and
  enters `kgspBootstrap_TU102`/GA10x. Capture the next stall (FWSEC/booter IMEM/
  DMEM writes + halt/mailbox poll).
- **M2 — Fake the boot to RISCV ACTIVE.** ACK FWSEC/Booter: report falcon halt +
  `MAILBOX0`=NV_OK (#4), `RISCV_STATUS._ACTIVE_STAT`=ACTIVE (#3), `WPR2_ADDR_HI`
  nonzero (#5). Record (don't honor) LibOS-boot-args / WPR-meta GPAs. Driver now
  believes the GSP processor started.
- **M3 — sysmem message queue + `GSP_INIT_DONE` (KEYSTONE).** Implement the
  command/status ring transport in the GPA the driver hands us
  (`GspStatusQueueInit`); consume `kgspSendInitRpcs`; post `GSP_INIT_DONE`
  (`rpc_init_done_v17_00`) with `rpc_result`=NV_OK (#6,#7). `RmInitAdapter`
  returns success → **the stock driver believes it has a live GPU.** This is the
  proof-of-concept gate for the whole approach.
- **M4 — RPC surface triage.** Enumerate the post-INIT_DONE `NV_VGPU_MSG_*`
  stream; classify each: (a) static/emulated answer, (b) shim into a Mode-1 RM
  ioctl, (c) needs real GPU. Build the shim dispatch skeleton. (Mitigation from
  spike §5.1: forward into the Mode-1 core, don't re-model GSP-RM.)
- **M5 — First real compute.** Wire enough RPCs + address translation + memory
  ops into the Mode-1 core to enumerate the GPU (`nvidia-smi`) and run one
  trivial compute op end-to-end through the emulated front + real back.
- **M6 — Display for free.** Expose a generic framebuffer/VGA scanout so early
  boot, text VT, and the desktop all present via the Mode-1 host console
  (`eaf90fc`) with zero compositor gymnastics — closing the loop that motivated
  the pivot.

M0–M3 = "does fake-the-boot actually work against a stock driver" (the risk).
M4–M5 = the large, known RPC/forwarding long-tail. M6 = the display win.

### M5 detailed plan (2026-06-04 — after cuInit+enum landed)

STATUS: M4 done. cuInit + full device enumeration PASS on GA106+open-580
(devices=1, RTX 3060, compute 8.6, 11909 MiB) via two mechanisms now committed:
- the **#2 address-virtualization side-table** from `NV2080_CTRL_CMD_GPU_PROMOTE_CTX`
  (0x2080012b) — resolves GR/compute channel GPU-VA→phys without page-table walks
  ([[mode2-address-virtualization]], [[mode2-promote-ctx-and-uvm-wall]]);
- the **UVM-completion forge** (debug backdoor: guest reports the CE tracking-sema
  GPA, QEMU writes the payload) — breaks the UVM_REGISTER_GPU busy-poll wall.

M5 GATE = **cuCtxCreate**. It currently segfaults *inside libcuda*: the per-context
GPU-ops vtable (`call *0x560([global+0x48])`) has a NULL/garbage method because the
context state was never really initialized — **forging completion ≠ doing the
work.** strace confirms every ioctl/mmap succeeds; the NULL deref is libcuda
reading an un-populated, GPU-owned context buffer. So M5 must execute/forward the
REAL GPU work, not fake it. Sub-steps (each commit-and-test):

- **M5.1 — unprivileged host context.** Stand up a real host-GPU context via the
  Mode-1 stub/isolate (one per guest userspace process, [[isolate-architecture]]),
  unprivileged-ioctl-only ([[access-model-split]]). No new host privilege.
- **M5.2 — forward the compute channel.** CUDA's client DOES use PROMOTE_CTX, so
  its GPFIFO/pushbuffer are already side-table-resolvable. Replace the forge for
  these channels with real submission: on the doorbell, translate the resolved
  pushbuffer → submit on the host channel (reuse Mode-1 chan forwarding).
- **M5.3 — context-buffer backing (chain #2).** Back the guest's GPU buffers (the
  0x200xxxxxxx UVM mmaps libcuda dereferences) with REAL host-GPU memory installed
  at the guest GPA window — the double-mmap ([[gpa-window-design]],
  [[realize-kvm-slot-regression]], [[uvm-in-qemu]]). This populates the context
  state libcuda reads → fixes the cuCtxCreate vtable crash.
- **M5.4 — UVM-internal channels.** Productionize the proof backdoor into a
  *validated* guest→VMM mapping-report (guest reports GPU-VA→GPA bounded to its own
  RAM; QEMU records in the side-table; real chan_exec runs the work). Replaces the
  write-anywhere debug forge.

Reuse the Mode-1 hardened dispatch/sanitizer for all host-side forwarded ioctls.
The cuCtxCreate libcuda RE (guest globals 0x7ffff7dd7bc8/+0x48) is superseded by
M5.3 — don't chase it; provide real backing instead.

## Isolation model (carried over from Mode-1, with a Mode-2 process key)

Mode-1's host security boundary is **one sandboxed isolate per guest userspace
process** ([[isolate_architecture]], [[access_model_split]]): the host-side
execution of a guest process's GPU work runs in a sandbox holding only that
process's GPU resources, so a hostile/buggy guest process cannot reach another's
GPU memory on the host. **Mode-2 MUST preserve this**, even though the guest now
runs one stock driver with no guest agent.

Granularity (user-specified, 2026-06-03):
- **One isolate per guest userspace process.** A process may hold many GPU
  contexts/channels — they all share that process's single isolate.
- **An isolate per context is acceptable** (over-isolation is always safe), but
  the *required* grouping is per process: contexts of the same process coalesce.
- **Process identity = the guest userspace address space**, NOT thread. QEMU
  cannot reliably distinguish guest kernel threads, so "same guest userspace VM
  (address space) ⇒ same process ⇒ same isolate" is the valid criterion.

The Mode-2 signal QEMU keys on (where Mode-1 used a guest-module pid/mm tag, now
unavailable): **the vCPU `CR3` at the trapping MMIO access** — the doorbell /
USERD / submission write (§6/§7 of [[nvidia_gpu_internals|nvidia_gpu_internals.md]]).
Same CR3 ⇒ same guest address space ⇒ same process ⇒ route the forwarded work to
that process's isolate. Properties:
- Userspace submission (the hot path) always traps from the owning process's
  address space, so CR3 is present and correct at exactly the moment we need it.
- Guest **kernel threads** share the kernel CR3 and do not ring the usermode
  doorbell, so they never blur a userspace process key; kernel-initiated RM
  traffic (boot, GSP-RPC setup) maps to a dedicated **system isolate**.
- RM clients / VA-spaces / channel-groups created via GSP-RPC are attributed to
  the CR3 that allocated them, binding each context to its process's isolate at
  creation time (so even non-doorbell control paths route correctly).

Open: confirm CR3 is observable at every relevant MMIO exit (it is, via the vCPU
state at the KVM MMIO exit) and define the CR3→isolate table lifecycle (process
exit = guest frees its mappings → reap the isolate, reusing Mode-1's reaper).

## Address virtualization (the reverse-driver core)

See **docs/design/mode2_address_virtualization.md** — GPU-physical is pure
bookkeeping between the guest kernel module and the QEMU extension; two
translation chains (GPU-VA -> GPU-phys -> BAR / or -> GPA-in-KVM-slot); a
7-state GPU-phys page model with a clear-on-assign simplification; lazy FB.
This is what the current UVM_REGISTER_GPU blocker needs (capture every VAS
root PDB, then walk chain #2 into guest RAM).

## Doorbell trapping: kernel vs userspace (decided 2026-06-03)

A consequence of the privilege model, NOT a perf choice:

- **Kernel doorbells / PRI writes** (interrupts, BAR/PDB binds, GSP control,
  page-map setup) poke *privileged* GPU control registers the host kernel driver
  alone owns; the unprivileged stub **cannot** mmap them. So they **must** be
  trapped and reverse-translated into the equivalent unprivileged operations
  (forwarded RM ioctls the stub performs) + bookkeeping of guest structures.
- **Userspace work-submit doorbells** (USERD / VOLTA_USERMODE_A) are part of the
  normal *unprivileged* RM userspace mapping. **Binding happens at channel
  *creation* time** (the trapped/forwarded kernel-ioctl path already created the
  matching host channel + work-submit token in the right isolate), so at *ring*
  time there is nothing to disambiguate — the doorbell page is 1:1 bound to one
  host channel and the mapping itself encodes isolation. These can be
  **direct-mapped** to the host channel's real USERD/doorbell (HW-direct rings,
  no per-ring trap) exactly as Mode-1 proved at native speed. CR3-keying is the
  **fallback** for the kernel/ambiguous path, not the hot path. Bridge
  requirement: the guest's channel USERD/doorbell GPA (chosen by the guest's own
  RM against the emulated GPU) must be backed by the host channel's real
  USERD/doorbell, wired when we intercept channel-create.

## Input validation & guest-data trust (policy — implement once compute works)

Today the emulator/reverse-driver parses guest-supplied input (GSP-RPC bodies,
page-table/PDE/PTE walks, channel pushbuffers, control params) **without bounds
checking**. Before any multi-tenant use this MUST be hardened. The rule
(user-specified 2026-06-03):

- **Reachable from malicious guest *userspace*** (anything a userspace
  doorbell/USERD submission or a userspace-issued ioctl can drive out of range):
  **never panic** — return an error / clamp, and where sensible **mimic what
  real NVIDIA hardware does on the violation** (e.g. a GMMU fault, an RC error,
  a method-error notifier) so the guest sees hardware-faithful behavior.
- **Reachable only if the guest *kernel module* broke its contract** (a value
  that could never go out of range from a normal, uncompromised guest kernel):
  a QEMU `abort()`/panic is acceptable — it means the guest kernel is
  compromised/buggy, outside the normal trust model. Prefer a logged error
  where cheap.
- All host-side forwarded ioctls reuse the **Mode-1 hardened dispatch/sanitizer
  stack** (size/_IOC_SIZE/struct/fd/alloc-class validation) — no new bypass.
- Ties into the doorbell model above: userspace can ring doorbells and write
  USERD/pushbuffers, so every field the emulator reads from those paths is
  userspace-reachable and must take the graceful-error branch, not panic.

Deferred until the compute path works end-to-end (correctness first), but a
launch blocker for "secure multi-tenant VM". Track alongside the existing
Mode-1 hardening ([[security_audit_2026_05_30]], [[access_model_split]]).

## Language: Rust core, thin C shell

Mode-2 is the right place to introduce Rust (decision 2026-06-03):

- **C — QEMU device shell only.** PCI config, BAR/MMIO read-write traps, MSI-X,
  KVM memory-region wiring. This must integrate with QEMU's C device model, and
  QEMU 9.2 has no first-class Rust device support, so Rust here is FFI pain for
  pure plumbing. The shell stays as small as possible: it traps and hands raw
  buffers/offsets to the core.
- **Rust — the logic core**, built as a `no_std`/static library behind a narrow,
  well-typed C ABI. Owns everything that parses **untrusted guest-supplied
  input on the host**: the GSP-RPC `NV_VGPU_MSG_*` decoder, the address-space
  virtualization + page-table (radix/PDE/PTE) walks, the address translation
  tables, and the RPC→Mode-1-ioctl shim. This is the new host attack surface and
  the keeper logic — memory-safety bugs here are catastrophic, so it is Rust from
  the start (no C→Rust rewrite later).
- **Converges with the stub→Rust rewrite.** Per-process (CR3-keyed) translation
  + compute forwarding runs inside the sandboxed isolate, in Rust — untrusted
  parsing is then *both* memory-safe (Rust) *and* sandbox-contained (isolate).
  System-level pieces (boot-register model, GSP-RPC transport) are the QEMU C
  shell calling the Rust core; per-process channel submission/translation routes
  to the Rust stub isolate ([[mode2_isolation_cr3_key]]).

Boundary rule of thumb: if it touches a QEMU API → C shell; if it interprets a
byte the guest controls → Rust core.

## Key decisions / open questions

- **Emulate vs capture-first:** iterate M0→M3 by booting the open driver against
  the emulated device and adding responses where it stalls (printk + BAR0 log is
  the ground truth). Running the open driver against the *real* GPU with heavy
  printk (spike's suggestion) is a parallel reference if a register trace is
  ambiguous — needs the GPU host.
- **MSI/interrupts:** the driver expects interrupts (e.g. GSP→CPU doorbell).
  Start polled where possible; add MSI-X assertion for the message-queue
  doorbell at M3.
- **Self-consistent identity (spike §5.2):** `NV_PMC_BOOT_0`, HWCFG, PMC boot
  regs, and PCI IDs must all describe GA106. Build a single chip-descriptor.
- **Any open-driver GPU, eventually (user directive 2026-06-03).** GA106 is only
  the *bring-up* reference (it matches the dev host, so the trace and the
  downstream forwarding describe the same silicon). The end goal is to emulate
  **any NVIDIA GPU the open kernel module supports** — every RTX/Ada/Hopper/
  Blackwell part. The architecture is built for this from the start:
  - All silicon-specific identity lives in the `NvkvmGpuChip` descriptor
    (`nvkvm_gpu_emul.c`): PCI IDs, `PMC_BOOT_0/42`, BAR sizes. Adding a chip =
    adding a table row; the device is selected to match (or be told to mimic)
    the host GPU it forwards to.
  - The **register answers** for the boot state machine are mostly arch-stable:
    the GFW/GSP path routes through the shared `_TU102` HAL for everything
    Turing-and-newer, so the GFW_BOOT/PLM/RISCV/mailbox offsets are common.
    Where an arch diverges (Ada/Hopper/Blackwell scratch layouts, CC), the
    descriptor carries per-arch register tables — same dispatch, different data.
  - The **GSP firmware + RPC ABI** are per-driver-version, not per-chip; matching
    the in-guest driver version (we run 580.159.04 to match the host) covers it.
    Converges with Mode-1's `abi_profile` auto-detect ([[multi_driver_validated]]).
  - Multi-GPU (N emulated functions, each bound to a host GPU by BDF) is the
    orthogonal axis already required ([[mode2_perf_dma_multigpu]]); per-instance
    state + per-chip descriptor compose: each function picks its own chip row.
  PoC proceeds on GA106; generalize to a chip table once fake-the-boot →
  GSP_INIT_DONE works on the reference part.
- **Confidential Compute stays OFF** (spike #8/§5.3) — never advertise CC.
- **QEMU is unprivileged at runtime (user directive 2026-06-03).** In prod the
  VMM process is unprivileged and may issue ONLY unprivileged nvidia ioctls
  (the Mode-1 isolate/access model, [[access_model_split]]). Nothing on the
  runtime host-GPU path may need root. The emulated device (BAR traps, register
  answers, PROM/VBIOS serving, MSI-X) is pure userspace QEMU — unprivileged. The
  **VBIOS image is a static provisioned asset**: dumping it from a host card
  (driver unbind + BAR0 PROM mmap) is a one-time root *provisioning/debug* step,
  not a runtime op — at runtime the unprivileged device just `fopen()`s the blob
  and serves bytes. Downstream compute forwarding stays unprivileged-ioctl-only
  inside the sandboxed per-process isolate, exactly as Mode-1.
- **Closed driver / Windows** deferred (spike §5.4): same attestation conclusion,
  unverified poll/RPC set.

## First concrete task (M0)

`src/qemu/nvkvm_gpu_emul.c` — a QEMU PCI device:
1. PCI config: vendor 0x10DE, device 0x2503 (GA106 / RTX 3060), class 0x030000,
   subsystem, capabilities (PM, MSI-X, PCIe).
2. BAR0: 16 MiB MMIO region; read/write handler that logs `(offset, size,
   value)` and, for now, returns the GA106 `NV_PMC_BOOT_0` on offset 0 and 0 for
   everything else.
3. BAR1: 256 MiB prefetchable FB aperture (stub).
4. Boot guest, bind stock open driver, capture the BAR0 access trace.

Scaffolded in this repo; build-wired into the host QEMU tree like the Mode-1
device. The boot-register state machine (M1/M2) is stubbed in the same file
behind a `reg_read` switch keyed on the offsets enumerated in the spike table.
