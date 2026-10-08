# Owner rulings — the decisions that govern kayfabe v3 work

**STATUS: LIVE, 2026-10-07 (§S added, then the BAR0-trace exception and the `PERF_GET_POWERSTATE` confirmation; earlier rulings dated in place); §T added 2026-10-08 (filtered recovered directives); §X added 2026-10-08 (non-stall interrupts); §Y added 2026-10-09 (diagnostic BAR0 trace mode, numbered §X on its branch, the VFIO tracer).** Every ruling the owner made in the 2026-09-25 … 09-30 working sessions,
with its date, so work can resume from the repository alone. The architecture itself is in
`docs/design/THE_V3_PLAN.md` and `THE_CONSTRAINTS.md`; this file records *decisions* on top of it.
Where a ruling was later refined, the refinement is listed under it. A ruling's date is part of its
citation: ask whether its reason still holds before relying on it.

## A. Standing principles

1. **v3 is a rewrite; never mix planes.** No CPU data plane; no mixing of CPU- and GPU-plane paths.
2. **Host verbs are authored, never forwarded.** kayfabe issues its own unprivileged host RM calls;
   it never replays a guest's privileged control raw.
3. **Completions are host events on an fd — never forged.** ★ (09-26) *A refusal the guest reads as
   "wait over" or "done" IS a forged completion* (`MC_SERVICE_INTERRUPTS`, the sysmembar flush): audit
   non-OK statuses on wait/flush paths first. An entry retired without its work having executed is the
   same defect.
4. **No blocking on a vCPU and none under a lock another thread blocks on.** The one exception: a PRAMIN
   window move = one host map + one mmap, synchronously in the vCPU.
5. **VMM addresses are never guest-chosen or guest-visible.**
6. **§13 unsafe discipline:** no raw VMM pointers in safe code; lengths/offsets validated inside the
   unsafe crates. `kf-cuda` may be a third unsafe crate (09-25).
7. **All GPU families are first-class** (Turing … Blackwell, incl. Hopper). **Derive per die, maintain
   per family:** per-architecture values come from the open guest driver source (ogkm) — generated
   tables, not hand rows; per-die values from unprivileged host controls at VM start.
8. **Compliance principle (09-26):** kayfabe only needs guest *userspace* to work. The guest kernel
   driver is open source, so read what it checks and supply values that satisfy it; prefer deriving
   from source over measuring.
9. **Hostile-guest isolation is the value proposition.** Guest root may only affect isolation *inside*
   the guest, never the host. In-guest isolation matters too: unprivileged guest userspace must not
   reach what the guest kernel protects (09-25 Q7; 09-26 PRIV leaves are withheld from user twins).
10. **Bare metal passes + guest fails ⇒ kayfabe bug.** (09-26) *Always confirm on bare metal first*
    before blaming a box or host.
11. **Don't silently change a constraint — tell the owner.** For open design choices, don't wait:
    run the best-bet **experiment on a sub-branch** (never master), report evidence, let the owner decide
    what lands (09-26).

## B. Specific decisions (2026-09-25)

- **Q7 kernel identity** = RM's `internalFlags` PRIVILEGE stamp (+ the RM-internal handle range).
  Guest-root (ADMIN) channels stay Passthrough.
- **Translated** rings/pushbuffer/USERD live outside the GPGA but inside the VA space; operands still
  reference the GPGA and are never copied.
- **`GPU_PROMOTE_CTX`: stub** — the host twin already holds the real context; golden context is
  guest-kernel-only.
- **Q8 memory plane:** the GPU walker sends only a **diff** against its last snapshot, kept **in vidmem**;
  no checksums; the host kernel is the ledger; **commit-on-ack** (only host-confirmed placements commit).
- **Non-GSP guests:** a future target, not v1 (Windows can force GSP on).
- **Q11:** narrow the ring-collision refusal to the offending leaf — self-harm only, never inducible
  across isolation by unprivileged guest userspace.
- **Per-twin RC** is fine iff unobservable to guest userspace and leaving no state the guest RM doesn't
  expect.
- **Hopper stays supported** (e.g. MEM_OP_D MMU_OPERATION is Hopper's invalidate: serve it, don't refuse).
- **Suite budget** may be raised when measured variance justifies it (it is 180 s since 09-26).

## C. Roadmap (2026-09-26), in order

1. Apps working (the nvkvm-pv CUDA app set; works/fails, not parity).
2. All headless-graphics tests nvkvm-pv passed.
3. Display (scanout), then display apps / a desktop (Mint) that nvkvm-pv ran.
4. In parallel: doorbell performance — non-nested baseline and host-side exits first, optional
   Linux guest helper afterward (refined 2026-09-28, §D); Blackwell (done: RTX 5080 30/30).
5. Driver matrix — the same driver range nvkvm-pv supports (535 → 610), both driver axes.
   ★ **Refined 2026-09-28 (owner): keep the GPU-architecture axis — the sweep covers every family
   Turing and newer** (host driver × guest driver × GPU arch), not one bench die. Vast VM offers seen on
   2026-09-28: TU116 (GTX 1660 S/Ti) and TU106 (RTX 2060 S), GA10x, AD10x and GB20x in number; **no
   GA100, GH100 or GB10x as VMs** — those rows stay source-derived until such a host is available.
6. Windows guest last. ★ **Refined 2026-10-01 (owner):** before Windows, the display must work with a
   **stock guest and no guest-side tweaks**, and frames should leave through a **VMM-agnostic broker**
   rather than QEMU's SDL/GTK UI (`design/V3_DISPLAY.md`, the NEXT note at the top).
   ★ **Demo milestone (owner, 2026-10-01):** a **game running in a Windows guest**, plus **SolidWorks**,
   shown in a window on the **Linux host desktop** (through the broker) — then a new r/VFIO post. The
   point to show: one GPU shared with the host (no VFIO passthrough, host desktop stays alive), the stock
   NVIDIA driver in the guest, hostile-guest isolation. Pick a game whose anti-cheat tolerates VMs.
- ⊘ *Superseded 2026-09-28 by the refinement of item 5 (Turing is on the arch axis, so it runs on
  hardware before the sweep):* Later, if time: **Turing** on hardware (its GSP model is source-derived only).
- **All working, verified work lands on master.** Nothing verified may be stranded on a branch.

## D. Guest doorbell helper module (2026-09-26)

- **Approved for design; security posture accepted:** giving guest *root* write access to the real host
  doorbell page (it can ring any host token) is accepted risk, as in shared CUDA containers — a ring only
  makes the GPU re-read that channel's own GP_PUT. Stock guests keep the trapped path.
- Design specifics (table = one atomic u64 per guest token; always opportunistic; virtio discovery;
  inert on bare metal; BAR1 map immediate / unmap waits for the module's ack; free load/unload order;
  BAR0 offset supplied per family; multi-GPU per BDF): `docs/design/V3_GUEST_DOORBELL_MODULE.md`.
- **Windows:** a legitimate Windows equivalent is not possible today (WDDM rings from the kernel
  driver); possibly not needed — measure doorbells/token on a Windows guest first
  (`docs/design/V3_WINDOWS_DOORBELL_RESEARCH.md`).
- **DECIDED 2026-09-28: baseline first, then the helper.** Measure on non-nested hardware and
  improve host-side exit handling first; **ioeventfd remains in the pipeline**. An optional
  paravirtual interface in a modified guest NVIDIA driver stays open for later, not selected now.
  Measure vCPU return separately from the eventual GPU doorbell: asynchronous dispatch can help
  a deep queue while hurting an idle/synchronous launch. Coalescing must preserve progress and
  ordering. The owner's ~70% non-nested throughput expectation and possible Windows batching
  advantage are hypotheses, not measured results. A Windows kernel transition is not itself a
  hardware VM exit; collect submission/doorbell counts before comparing OSes.
- **Refined 2026-09-28: no deliberate coalescing delay.** Ring as soon as the asynchronous worker
  can act. No batching timers or extra waiting to accumulate kicks; CUDA already batches work.
  Coalescing is only a benign side effect when several notifications are already pending, not a
  performance objective in itself. Notification counts need not match host MMIO ring counts;
  eventual GPU notification or translated/emulated queue inspection must never be lost.
- **BAR1 doorbell (Hopper+):** must follow where RM places it — built (`V3_BAR1_DOORBELL.md`).
- **Refined 2026-09-30 (owner, in the session):**
  - *ioeventfd is for every doorbell, not only passthrough:* "doorbells always need to be forwarded as
    quickly as possible", so the non-blocking register drainer is the right place; it rings the host twin
    inline for Passthrough and hands Translated/Emulated tokens on. (This is what `v3-mc22` built:
    `design/V3_DOORBELL_IOEVENTFD.md`; still default **off** pending a non-nested measurement.)
  - *Nested is a real deployment, not an edge case* ("think microVM containers"). The owner expects the
    guest helper module to bring a nested doorbell to about **1–2 µs**, which they consider sufficient
    for parity. ⚠ Hypothesis: the helper is still design-only and unmeasured.
  - *Why the C reached host parity despite trapping every doorbell:* its 1.05× (llama.cpp 49.9 vs
    47.5 tok/s) was measured on **bare metal**, an RTX 3050 kiosk PC (`docs/archive/STATUS_DETAIL_pre_v3.md`
    "Hardware"), not on a nested Vast box. ★ (owner, 2026-09-30) That C is **nvkvm Mode 2 — the same shape
    as kayfabe** (stock guest driver, emulated GPU + fake GSP, trapped doorbells), **not nvkvm-pv**; that is
    why it is the relevant baseline. Its copy-engine work ran on the CPU, so it speaks to the doorbell cost,
    not the copy path. On bare metal a KVM exit costs a few µs, not the ~15–50 µs of
    a nested box, so ioeventfd alone may reach parity there. Bare metal is also the most useful case for
    Windows guests. ⚠ Hypothesis until a non-nested host is measured (§D "baseline first").

## E. UVM demand paging (2026-09-26)

- Unprivileged-only is believed impossible (host replayable faults need a host-UVM-owned VA space; the
  fault buffer is kernel-privileged; the GPU reaches memory at the guest's VA).
- **A privileged host piece is allowed, for UVM only, and it must not trust the VMM**; the rest stays
  unprivileged and untrusted.
- **DECIDED 2026-09-28: full host CUDA must coexist.** A narrowly scoped patch to the host's open
  **nvidia-uvm (b3)** for guest UVM fault handling is acceptable and is the selected route. Keep
  ordinary host CUDA behavior, the CUDA-based walker, and non-opted-in address spaces working.
  **Supersedes the 09-26 preference** for a separate replacement module: N4 takeover prevents
  stock UVM coexistence in the same host kernel (the 580.159.04 callback registration is global,
  not per GPU). N4 remains historical research, not the next implementation experiment.
- **RULED 2026-10-02 (owner) on the `v3-uvm-guest` questions — both NO:**
  - *No MMIO/BAR CPU reads of guest page tables* (the branch's §3.8a experiment reads them through
    CPU views of the store). Prefer the GPU walker; if a CPU decode is unavoidable on the fault path,
    the words must come from a **copy-engine snapshot** into host memory, not MMIO reads.
  - *A ~4 s stall of every tenant's GR work per parked fault is not acceptable.* Cause, measured on
    the RTX 3060: a GR context stalled on a parked replayable fault cannot be context-switched out, so
    no other GR work runs (other VMs, host CUDA, kf3's own walker) until the fault is replayed; the
    bound is the host's ~4.3 s ctxsw watchdog (Xid 109), not a kayfabe timeout. ⇒ The fault path must
    be GR-free end to end and fast, the park time bounded far below the watchdog, and UVM is **not
    merged** until the per-fault GR hold time is measured and acceptable (two-VM test, one hostile).
- Stock UVM read-duplication (`cudaMemAdviseSetReadMostly`) is not a kayfabe mode — kayfabe must *handle*
  it correctly (read-only PTEs, collapse on write): permission bits are now carried.
- **Next: bounded b3 host-only proof before guest integration.** Require real fault delivery,
  repair/replay with correct data, scoped cancellation, ownership-negative tests, bounded teardown,
  and concurrent ordinary host CUDA. Existing `DupAddressSpace` / `RetainChannel` helpers do not
  provide caller ownership authentication for free; the new boundary must enforce it. No production
  support or coexistence result is implied by this direction. Historical research continues through
  `v3-uvm-e6pp` at `c6765f5c`; its phase 0 did not prove replay with a replacement module present.

## F. Work practice (2026-09-26)

- **Agents:** each on its own worktree + sub-branch and its own vast box(es); builds on the box; local
  cargo only under a global flock with 2 jobs and a deleted-after target dir (the dev host is shared).
- **Boxes:** untrusted, not guaranteed to persist, no secrets, no executables copied back, evidence
  pushed to git after each run, keep only boxes in use, teardown only by ids you created
  (`scripts/bench/box/README.md`).
- **Security-policy changes need explicit owner review before merge.** **APPROVED 2026-09-28:**
  the 535/545 capability extension (`a50265f8`, formerly `ee35ca4a`, on `v3-drivers`). Independently
  audit the shared groups omitted by the header sweep, compare complete resolved existing 550+
  policies (not just counts), and pass the required exact-revision tests before promotion. This
  approval does not claim end-to-end 535/545 application support or relax other policy rules.
- **Refined 2026-09-30 (owner):** *"a chat is no durable artifact, neither this host — keep GitHub as
  backup for all valuable work."* The dev host is not durable storage either: push branches (a
  `backup/<host>-<date>/` namespace for unreviewed local work, after a secret scan) and record the resume
  point in `docs/STATUS_AND_HANDOFF.md` §0 before any pause.
- **Pacing (owner, 2026-09-30):** the owner expected check-ins before long unattended stretches ("thought
  you would stop earlier and ask") while welcoming the results; weekly-usage aim ≈ 14 % per day. Check in
  before launching a new wave of agents or boxes; wrap up (push, stop notes, destroy boxes) when asked.
- **Storage and execution reaffirmed 2026-09-28:** Vast is untrusted, replaceable compute, never the
  only copy of unique work. Commit/push source and useful text evidence from trusted local storage;
  do not send account credentials or forward the SSH agent. The owner authorized taking over all
  instances in the recovery inventory and retiring those no longer needed, after preserving work.
  **Superseded 2026-09-29:** the owner authorized retiring the Paguro Windows box after saving
  any unique work; retention is no longer required. The /dev/sdb SSD is
  spare workspace; regenerate/download caches, builds and VM images rather than lose unique work.
- **Public repository: which addresses may be committed (owner, 2026-10-07, relayed by the
  coordinator).** Private LAN addresses, Vast instance ids and Vast IP addresses may appear in commits.
  The owner's home public IP and any Scaleway public IP of the controller must NEVER be committed.
  Scan every diff for non-private IP addresses before pushing.

## G. Licence (2026-10-02)

- **Ruling:** dual-license the whole repository as **`Apache-2.0 OR GPL-2.0-or-later`**. The owner's
  words: *"Same as ogkm. We satisfy both, user may chose either"*. The owner confirmed the exact
  expression on 2026-10-02.
- **Done on branch `v3-dual-license`:**
  - `LICENSE` states the grant and its exceptions. `LICENSE-APACHE` and `LICENSE-GPL` hold the full
    texts.
  - The `Cargo.toml` `license` fields are updated: the workspace and both detached `gen` crates.
- **Exceptions, as `LICENSE` lists them:**
  - the `third_party/` submodules;
  - files derived from gVisor's nvproxy, which stay Apache-2.0 only;
  - files that carry their own notice;
  - third-party data inside recorded traces.
- **Still open, under task I7 (`docs/design/V3_SWEEP_AND_INSTALL.md`):**
  - Re-derive the nvproxy-derived allowlists (`kf-abi`/`kayfabe-abi` `capability.rs`) from ogkm
    headers. `kf-qemu` links `kf-abi`, so until then a QEMU binary built from this tree is not
    distributable under the GPL.
  - Per-file SPDX headers, starting with the three kf3 overlay files.
  - `LICENSES/` in the release tarball.
- **For the owner to decide:** whether the archived C reference traces stay in the public tree.
  - `archive/nvkvm/traces/mode2_c_reference/` records PROM reads as served values.
  - The C emulator served those reads from a real GA106 VBIOS dump: its `vbios=` property; md5 in
    that directory's README.
  - The grant does not cover that data (`LICENSE`, exception 4).

## H. Feature scope after a working product (2026-10-02)

- **Stub what workloads do not need** (owner). Stub a refusal when it only blocks a feature
  workloads do not need, especially one an unprivileged host process cannot issue.
  - Named: debugger, profiler, NVLink, ECC.
  - Confidential compute is impossible. Its attestation is rooted in the physical GPU's keys and is
    never faked.
  - A stub answers the way a GPU without the feature answers: capability absent, or not permitted.
    It never answers `OK` to an action; that is the forged-completion trap of 2026-09-26.
    ⊘ *Refined 2026-10-07 by §S:* a privileged host-management action with no compute or
    display effect on the guest (RC-recovery policy, notifier arming for thermal or power
    events, for example) may be accepted silently, with a coherent "feature absent" state.
    GPU work that reached hardware is still never forged.
  - Read in ogkm-580 the same day:
    - The **debugger** is open to unprivileged users for their own processes
      (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_sm_debugger_session.c:270-301`). kayfabe
      could forward it later.
    - The **profiler** needs root or `CAP_PERFMON` unless the host admin sets
      `RestrictProfilingToAdminUsers=0`. The default is 1 (`ogkm-580: kernel-open/nvidia/nv-reg.h:965`;
      `ogkm-580: src/nvidia/src/kernel/gpu/hwpm/profiler_v2/kern_profiler_v2.c:31-63`), so a stub
      matches what a non-root user gets on bare metal. If it is ever offered, make it opt-in for
      single-tenant hosts only: profiler counters are a side channel across tenants.
    - The debugger does **not** service GPU faults. cuda-gdb's own table lists the memory
      exceptions as "Not precise" and says continuing after them "can lead to undefined behavior"
      (NVIDIA's cuda-gdb documentation, read 2026-10-02). Resumable fault service exists only below
      the debugger, in UVM, and in kayfabe's EFS patch (`tools/uvm_efs/patch/uvm_efs.c`).
- **Guest-side NVIDIA vGPU: crossed off** (owner).
  - Running NVIDIA's licensed vGPU guest stack on kayfabe needs its license check faked, the way
    vgpu_unlock setups do. That is piracy, however little reverse engineering it takes.
  - A user who holds a vGPU license should run NVIDIA vGPU and does not need kayfabe.
- **Ideas for later, not rulings.** The full plan, with the stock tier and the opt-in stages, is in
  `design/V3_COOPERATIVE_TIERS.md` (2026-10-03).
  - Host-side vGPU-style sharing, one device per VM, through Linux mediated devices, so Proxmox,
    libvirt and OpenStack attach kayfabe unmodified. Not NVIDIA's vGPU software, and no licensing.
    See `design/V3_VFIO_USER_FRONTEND.md` §2, option 0.
  - MIG: map host MIG instances into guests, on MIG-capable cards.

## I. Release scope and priorities (2026-10-03)

- **Post-release:** the cooperation stages in `design/V3_COOPERATIVE_TIERS.md` (owner). These are
  the patched guest driver, the patched host driver and the host helper module.
- **Release focus, in order (owner):**
  1. the display path (`design/V3_DISPLAY.md`, NEXT block of 2026-10-03);
  2. the install path and the sweep (`design/V3_SWEEP_AND_INSTALL.md`);
  3. Windows.
- **Non-GSP (pre-Turing) was raised as possibly more useful.** The review's view:
  - It is the largest new emulation surface: the GSP-side three quarters of RM, running against
    emulated registers.
  - It targets a frozen platform. R580 is NVIDIA's last driver branch for Maxwell, Pascal and
    Volta, and CUDA 13 dropped them.
  - §B ("Non-GSP guests: a future target, not v1") stands. Decide after Windows, on demand.
- **Guest doorbell helper (stage 1): a release candidate,** decided after two steps:
  - a per-token breakdown on nested boxes, where doorbells explain about 22 of PyTorch eager's
    ~58 ms per-token gap (0.29× of host) and the rest is unattributed;
  - a non-nested baseline (§D, 2026-09-28).
  - llama.cpp already decodes at 0.92× of host there.
- **Managed memory (UVM demand paging): not a release target (owner).** PyTorch, TensorFlow, JAX,
  vLLM, llama.cpp by default, games and Vulkan, GL and desktop apps do not use it.
  - Exceptions to document:
    - opt-in switches such as llama.cpp's `GGML_CUDA_ENABLE_UNIFIED_MEMORY`;
    - RAPIDS cudf.pandas, which, as far as the review knows, defaults to managed memory when the
      GPU reports concurrent managed access.
  - **Release item: unsupported must fail loudly.** The four managed-memory apps fail on master,
    and `conjugateGradientUM` prints a wrong answer with `result = SUCCESS`
    (`design/V3_APP_MATRIX.md`). Every such fault must reach the app as an error.
  - A stock Linux guest cannot use the Windows-style mode: guest UVM hard-codes fault support
    (`ogkm-580: kernel-open/nvidia-uvm/uvm_ampere.c:81`). Windows guests should get managed memory
    without a fault plane; that is unverified until a Windows guest runs CUDA.
- **Pinned DMA stays supported.** `bandwidthTest` and `simpleZeroCopy` pass on master
  (`design/V3_APP_MATRIX.md`).
  - Coverage gap: no row uses the CUDA virtual memory API (`cuMemCreate`/`cuMemMap`), which PyTorch's
    expandable segments and vLLM rely on. Add a sample such as `vectorAddMMAP` to the sweep.

## J. Kernel-module flavours (2026-10-03)

- **Closed (proprietary) host modules are supported, never refused (owner).** nvkvm-pv forwards the
  same RM ioctls on both flavours and met few compatibility differences between them. kayfabe's host
  side uses only that unprivileged ioctl API and nvidia-uvm, whose source ships in both packages.
  - Preflight reports the flavour and does not refuse it. Until a sweep covers a closed-host cell,
    `SUPPORT.md` shows it as "accepted, untested".
  - NVIDIA's own limit: the closed module cannot drive Blackwell (`nvkvm-pv:docs/howto/sweep.md:303`),
    so closed-host cells exist for Turing, Ampere and Ada.
  - This supersedes the review's recommendation of 2026-10-03 to refuse closed hosts
    (`OWNER_QUESTIONS_2026-10-03.md`, Q5).
- **Closed guest modules are a sweep axis, and Windows makes them a must (owner).** Windows has only
  NVIDIA's proprietary driver.
  - At 580 both flavours use the GSP firmware by default on every Turing-and-later GPU (`README.txt`
    of `NVIDIA-Linux-x86_64-580.159.04.run`, chapter 44C, read 2026-10-03). A closed Linux guest
    therefore reaches kayfabe's fake GSP with no module option. Older branches are to be checked per
    tag.
  - The open question is whether the closed guest's kernel RM issues RPCs or controls the open one
    does not. One closed-guest cell per family in the band tier answers it.
  - As far as the review knows, the closed Linux RM and the Windows RM come from the same NVIDIA
    sources, so the closed-guest cells are also the earliest Linux-side signal for Windows
    (unverified).
- Answers `design/V3_SWEEP_AND_INSTALL.md` §4 Q5.

## K. The boot display's option ROM is one embedded blob plus generated config (2026-10-03)

- **Owner:** *"generate the uefi data in kayfabe and give it as blob in the rom. So there is no rom
  per gpu or similar, all is given as config data, just like cuda."*
- The GOP driver is one constant `.efi`, embedded in kayfabe with `include_bytes!`. It is not a
  separate firmware file, there is no `romfile=`, and users install nothing per GPU.
- **No compiled binary is committed** (owner, the same day: *"We aren't going to put compiled stuff
  in the repo right? … the efi driver is compiled when building the repo."*). build.rs compiles
  `firmware/kf-gop` during the normal cargo build, with the nested-cargo pattern that already embeds
  the musl isolate (`crates/kayfabe-isolate-host/build.rs`). `rust-toolchain.toml` lists
  `x86_64-unknown-uefi` beside the musl target, so rustup installs it wherever the repo builds. The
  PTX kernels the owner compared it to are hand-written source, not build output
  (`crates/kf-cuda/src/display.rs:21`).
- kf3 generates everything per device when it starts: the PCI ROM header and PCIR (the identity kf3
  already presents) and the `KFGP` config blob (BAR, offset, size, mode, pitch, format, EDID). It
  wraps the constant `.efi` with them in memory and serves the result as its ROM BAR.
- The driver is byte-identical on every host, so one future Secure Boot signature covers all of
  them.
- Design: `traces/v3_design_review_20261003/` (gop), and `design/V3_DISPLAY.md` once `v3-gop-rom` and
  `v3-gop-kf3` land.
- **aarch64 later (owner, the same day: *"Also later needs aarch64 as well."*).** x86_64 ships first,
  but the ROM stays arch-neutral:
  - the ROM header's EFI machine type comes from the built driver (0x8664 or 0xAA64), never a
    constant;
  - x86 port I/O, used for test-build debug output only, is behind `cfg(target_arch = "x86_64")`;
  - build.rs builds the driver for the arch kayfabe itself is built for;
  - CI's aarch64 job also builds the driver for `aarch64-unknown-uefi`, so x86-isms cannot creep in.
  - Unchecked: whether AAVMF (OVMF's Arm build) runs PCI option ROMs.
  - ⊘ **CORRECTED the same day — the note below overstated it; there is no regression against bare
    metal.** nvidia.ko itself maps the framebuffer BAR as **Device-nGnRE** on arm64 when write
    combining is asked for (`ogkm-580: kernel-open/common/inc/nv-pgprot.h:76-80`, used for
    `NV_MEMORY_TYPE_FRAMEBUFFER` by `kernel-open/nvidia/nv-mmap.c:355-363`, `:587-597`; WC is allowed
    on aarch64, `nv-linux.h:307-308`). So the host's own CUDA gets Device memory for BAR1 too. KVM's
    Device stage-2 for these views equals what bare metal uses, and the guest's own nvidia.ko asks for
    the same type. What remains:
    - code that maps the boot framebuffer expecting Normal-NC must use aligned accesses (the GOP
      driver's `Blt`: aligned volatile stores, no `DC ZVA`-style memset; the guest kernel's I/O
      accessors already align);
    - the `VM_ALLOW_ANY_UNCACHED` patch below would give *more* than bare metal (Normal-NC), which
      NVIDIA chose not to use on arm64, and ARM does not guarantee Normal-NC is safe on every MMIO
      region (`drivers/vfio/pci/vfio_pci_core.c:1815-1830`). Not planned.
    - The same holds for BAR2 (PCI BAR3) and the PRAMIN window. Under the single store
      (`design/THE_CONSTRAINTS.md:1011-1020`) every mapped page of all three windows is a view of
      the one reserved host-VRAM object (`crates/kf-qemu/src/mem.rs:1-35`). Only unmapped pages show
      the per-window scratch memfd, which exists because a memslot hole kills the guest. Since
      2026-10-03 that scratch is one small tile per window, repeated, so a guest touching every
      unmapped page costs the host at most one tile per window (`design/V3_P4_PORT_MAP.md` Q3).
  - (superseded, kept as written) ⚠ **Found 2026-10-03, and it concerns all of kf3 on arm64, not only the ROM.** arm64 KVM maps a
    non-cacheable PFNMAP memslot as Normal-NC only when the host VMA carries `VM_ALLOW_ANY_UNCACHED`,
    and as Device memory otherwise (Linux 7.1 `arch/arm64/kvm/mmu.c:1966-1968`). vfio-pci sets that
    flag (`drivers/vfio/pci/vfio_pci_core.c:1831`); nvidia.ko never does (no occurrence in
    `ogkm-580: kernel-open`). kf3's BAR1 views are nvidia.ko mmaps, so on an arm64 host the guest
    would see the framebuffer and every BAR1 view as Device memory: no write combining, and an
    unaligned access faults. The likely fix is the one-flag host patch in nvidia.ko's mmap path,
    since a host patch is already required for UVM (`design/V3_COOPERATIVE_TIERS.md`).

- **Secure Boot (owner, 2026-10-04, verbatim):** *"In qemu we can just enable 'secure boot' for a
  windows vm and self sign the rom, I mean windows vms just work normally without complaint"*.
  - The ROM's EFI driver is signed with a kayfabe key, and the VM's OVMF variables enroll
    Microsoft's standard keys plus that certificate in `db`. Windows' boot manager and the ROM then
    both verify, and the guest runs with Secure Boot on.
  - The private key is never committed (the repo is public). It is generated per installation or
    per build on the user's machine, and that installation's variables file enrolls its
    certificate.
  - A guest that seals BitLocker to TPM measurements may ask for its recovery key once after the
    ROM changes, because the measurement of the option ROM changes.
- **The vTPM (swtpm), from the owner's question the same day** (*"For swtpm a secure seed must be
  provided probably?"*):
  - ⊘ *Corrected the same day; the first wording ("no seed is supplied") was too loose.* Owner,
    verbatim: *"Yes but tpm should persist reboot though. And a hash of non secret values isn't
    secure. So some secure seed must be stored right."* Right.
  - **A secret seed is stored: the state file is the secret.** `swtpm_setup --tpm2` manufactures the
    TPM once per VM. The primary seeds (endorsement, storage, platform) are random secrets from the
    host's CSPRNG, never derived from non-secret values such as the VM's name or UUID. swtpm keeps
    them in its state file. Everything the guest seals to the TPM, BitLocker and Windows Hello
    included, depends on them.
  - **It persists.** The same state is reused on every guest boot, every QEMU restart (kayfabe
    restarts QEMU on each guest reboot) and every kayfabe update. Manufacture happens once, and the
    tooling refuses to re-run it on an existing state. Losing or regenerating the state is like
    replacing the TPM: BitLocker asks for its recovery key. The VM's OVMF variables file is per-VM
    persistent state too.
  - **It is protected like a key:** one state per VM; owner-only permissions; kept with the VM and
    backed up with its disk, as a secret; never copied into an image, a template or another VM
    (a copy duplicates the seeds and the endorsement key); never committed. If it is encrypted at
    rest (`--key`/`--pwdfile`), that key is a random host-held secret.
  - A self-signed EK certificate is enough for Windows 11 and BitLocker.
- **Bench Windows guests run without BitLocker** (owner, 2026-10-04, verbatim: *"Also disable bitlocker
  in the windows guest, its useless for our vm."*). The unattended install prevents BitLocker and
  Windows' automatic device encryption from the first boot, and the lane checks the volume is fully
  decrypted with protection off, after install and again after the NVIDIA driver install. This is a
  bench setting: a user's own Windows guest may still use BitLocker, so the TPM-persistence rules
  above still apply to it.

## L. Broker frames: a GPU copy into kayfabe's own frames, never guest memory (2026-10-03)

- **Owner:** *"exact zero copy isn't needed though, what we do need is that we can avoid a GPU-CPU copy.
  So if you need an object to export to the screen that contains a frame you can also copy directly.
  Is maybe better for security because a shared RM object lets the guest change the bytes
  underneath."*
- No guest surface and no slice of the store is ever exported to the broker or any other process.
- For a compositor on the same NVIDIA GPU, kf-disp copies each finished frame GPU→GPU into a frame
  object kayfabe allocated itself in host VRAM. That object is exported as a dma-buf with a format and
  modifier the compositor imports. The frame never crosses to the CPU, and the guest cannot change it
  after the copy.
- The existing rungs stay as fallbacks: a linear dma-buf copy in host RAM for compositors on another GPU
  or vendor, and F_SHM.
- Design pass of 2026-10-03; research in `traces/` once the broker branch lands.

## M. The display frame-rate bound is configurable (2026-10-03)

- **Owner:** *"I think the fps bound must be configurable."* Context: with `GF100_DISP_SW` twinned on
  the host (`x11-dispsw`, still an experiment), host RM fires vblank releases at the host head's vblank,
  and immediately on a headless host. Vsync'd X11 clients would then run uncapped.
- One device property (working name `display-max-fps`), defaulting to the virtual monitor's refresh.
  - **KMS flips** (Wayland, fullscreen X, nvidia-drm) are paced by kf-disp's own vblank timer, so the
    bound is exact there. The EDID's preferred mode follows the same rate.
  - **X11 display-SW paths** (windowed GLX, X11 Vulkan FIFO): ⊘ corrected after the required
    D4 hardware run, 2026-10-04. On candidate 2 `9d82f259`, NVIDIA 580.159.04 on GA106,
    these clients follow the guest driver's consumption of kayfabe's emulated vblank. Cap 30
    gives GLX about 30 FPS (also with a forced 60 Hz raster) and actual Vulkan FIFO presents
    at 29.976857 FPS; the IMMEDIATE control reaches 1557.8 FPS. Thus the clamp bounds the
    measured vsync paths. The earlier proposal for extra notify/submission pacing levers is
    superseded for these paths; this is not a promise to throttle non-vsync rendering.
    Evidence: `traces/v3_candidates/cand2_20261004/{x1130,present_timing}/`.
  - The status line reports the achieved rate per path, so a bound that does not hold is visible.

- **Design decisions (2026-10-04).** The design is reviewed and kept outside the repo until the
  `v3-maxfps` branch carries it. Owner, verbatim: *"Why copying when not flipping. Yes a tearing copy
  seems not great though. The rest seems good to md"*.
  - **Mechanism:** the limit clamps kf-disp's own emulated vblank tick (the display thread's
    deadline). Nothing blocks a vCPU, and no lock is held while waiting.
  - **D1, tearing flips are gated (adopted).** An async/tearing flip that arrives sooner than the
    limit allows waits for the next tick. In kayfabe a flip copies a finished buffer, so the gate is
    about rate, not image tearing.
  - **D2, copies made without a flip (proposed by Claude in answer to the owner's question; adopted
    unless the owner objects).** These exist for front-buffer rendering: the boot console, X11
    without a compositor, front-buffer applications. kayfabe has no physical scanout, so without a
    copy the host never sees those writes.
    - Instead of a fixed 30 Hz timer, a copy is made at the head's emulated (clamped) vblank. That is
      real scanout's cadence and phase, so tearing is no worse than bare metal.
    - A copy is sent only when a GPU-side checksum of the surface changed.
    - Nothing is copied while nobody watches. A `screendump` asks for a fresh copy on demand.
  - **D3 (adopted):** values above 75 Hz are refused. The virtual monitor is single-link DVI
    (165 MHz), so 1080p tops out near 71 Hz.
  - **D4 (adopted, verified 2026-10-04):** the required `display-max-fps=30`, `x11-dispsw=on`
    run supports pacing through the guest's vblank consumer. The correction is folded above;
    §N's separate conditions for enabling x11-dispsw by default remain outstanding.
  - **D5 (adopted):** unset means a cap of 75 Hz, and the EDID stays byte-identical to today.

## N. X11 desktops: GF100_DISP_SW option A is the design (2026-10-03)

- **Owner, after the design was laid out:** *"I think this is the best design i intended."* This answers
  `OWNER_QUESTIONS_2026-10-03.md` item 2.
- **The design:**
  - A guest `GF100_DISP_SW` (`0x9072`) allocation gets a real host twin under its channel's host
    twin. Its parameters are authored by kayfabe (head 0, displayMask 0, caps 0); the guest's
    parameters are never forwarded.
  - The guest's display-SW software methods trap on the host GPU to host RM. Host RM writes the
    vblank release directly into the guest's semaphore memory. That memory is a guest-RAM page or a
    store slice, mapped with a host kernel mapping (`NVOS46_FLAGS_KERNEL_MAPPING_ENABLE`) for exactly
    that memory. kayfabe acts only at allocation, mapping and teardown, not per vblank.
  - Pacing follows §M.
- **It stays inside the single store (owner, the same day: "the guest cannot get an object outside
  its allocated vram").** The object is not memory the guest can address, and releases land only
  in the guest's own mappings. The host client that the address check uses must hold only the
  guest's store, guest RAM and twins; kayfabe's own allocations, such as the broker frame slots,
  live in separate clients. This is pending the client audit of 2026-10-03.
- **Default-on once all of these hold, on hosts whose GPU has a display engine** (refusal by name
  stays the fallback elsewhere):
  - a box run shows releases landing: Cinnamon X11 up, X11 vkcube presenting, vsync clients
    advancing;
  - per-VM caps on display-SW objects and kernel-mapped bytes exist, and refuse by name;
  - the client split is built;
  - a release aimed outside the guest's own memory lands nowhere outside the store and guest RAM.

## O. The guest cursor (2026-10-03)

- **Owner:** *"For hover broker receives the cursor image of the guest, and sets using wl/x api that as
  cursor … if the guest doesn't draw a cursor then it remains hidden in hover/absolute. For grab mode …
  we follow the cursor as is."* And: *"frame copy in gpu is cheap. Best is copy to a nvidia buf on the
  gpu … draw cursor there, then copy to linear over dma as dmabuf (in case of host ram)."*
- The host's hardware cursor plane is never used. The guest programs kf-disp's emulated cursor channel,
  and kf-disp knows the cursor's image (from the store), hot spot, position and visibility per head.
- **Hover / absolute pointer:**
  - The broker receives the guest's cursor image and hot spot, and sets it as the host cursor
    (Wayland `wl_pointer.set_cursor`; X11 an ARGB cursor on the window).
  - The guest's cursor position is ignored, because the absolute device already makes the host
    pointer the guest pointer. A cursor move produces no frame.
  - If the guest hides its cursor, the host cursor is hidden.
  - The image is sent only when it changes, detected by a hash.
  - kf-disp does not compose the cursor into frames in this mode, so there is never a second,
    lagging cursor.
  - If the broker scales the guest frame, it scales the cursor image and hot spot by the same factor.
- **Grab / relative pointer:** the guest owns the position. kf-disp composes the cursor as the top layer
  of the frame (display step 3d), and frames are coalesced to the display rate (§M).
- **XOR / monochrome cursors** cannot be expressed as a host ARGB cursor, so they are composed into the
  frame in either mode. The compose kernel gains an XOR blend.
- **Where composition happens:** always on the GPU, into kayfabe's VRAM staging frame. The GPU-copy rung
  packs that frame into the exported VRAM slot (§L). The host-RAM rungs (linear dma-buf, F_SHM) take
  one copy-engine DMA from staging into the CUDA-registered host frame; the CPU never copies a frame.
- **Protocol:** the broker protocol gains a cursor message (image fd, size, hot spot, hide) behind a
  capability bit. A broker without the bit gets composed cursors. The change goes into nvkvm-pv's
  broker too, so both projects get it.

## P. A security audit before the first public binary (2026-10-03)

- **Owner:** *"Maybe worth doing a security audit. Meanwhile continue on display."* The owner named ten
  areas: no host pointers in safe code; unsafe code bounds-checks every input; races and
  interleavings, with formal checkers where useful; guest DoS points; the security model by role;
  every RM call and ioctl we make; the PTX page-table walker against hostile tables; integer, cast
  and compiled-out checks; prove what can be proved and record what is uncertain; and bug classes
  from history.
- **Plan and timing:** `design/V3_SECURITY_AUDIT_PLAN.md`. Stage 1 (read-only inventory areas) runs
  now, alongside the display work. Stage 2 runs after the display and security branches merge.
- **Gate:** no public binary ships before every stage-2 blocker is closed or accepted by the owner.
- **Out of scope:** sandboxing the VMM process. The owner, the same day: *"Sandboxing vmm is not our
  job."*
- **kf3's host channels when the VMM is root** (the audit's P0, the same day). The owner's full
  sentence: *"I dont think you should refuse cap sys admin if clearing a bit fixes it. Sandboxing vmm
  is not our job though."* ⇒ kf3 does not refuse to run with `CAP_SYS_ADMIN`. It clears the bit from
  a thread's effective set where host RM reads it: for each channel-alloc call kf-host makes, and
  for the life of each thread that calls libcuda. `design/THE_CONSTRAINTS.md` §30 has the rule and
  its checks (`v3-sec-nonpriv`, pending the owner's review).

## Q. The address model: what each kind of address may reach (2026-10-03)

- **Owner, verbatim:** *"Gpga offsets can't, outside guest vram is invalid. Gpa offset can't either,
  only reference guest ram or its bar or an error. Same for bar offsets. Passthrough uses a sandboxed
  channel, that strictly only maps based on pte/pdb and nothing else. Translated uses a channel only
  gpga and/or gpa is mapped."* And, on host-process addresses as GPU addresses (HMM): *"only for the
  cuda channel (vmm ptx refresher), and this offset is never supplied or given by guest."*
- **Rules that follow:**
  - A guest-VRAM offset (GPGA) outside guest VRAM is invalid. A guest-physical address (GPA) resolves
    only to guest RAM or the device's own BAR, or it is an error. BAR offsets are validated the same way.
  - A passthrough twin's host VA space maps only rows derived from the guest's own page tables. It maps
    nothing else: no windows, no kayfabe memory. (Audit S1-21 records where the code departs from this today.)
  - A Translated channel's host VA space maps guest VRAM and/or guest RAM, and nothing else the guest's
    work can address. **This answers `OWNER_QUESTIONS_2026-10-03.md` 6c: a guest-RAM window in the
    Translated space is allowed.** The one mapping the engine needs beyond that, the Translated
    channel's own command ring, is kayfabe-authored, mapped read-only except its fence, and outside
    every address the rewriter will emit.
  - Host-process addresses reach a GPU only in kayfabe's own CUDA contexts (the walker and the display
    compose), and no guest value is ever used as one. Guest data only steers offsets inside those
    kernels, so the kernels' own bounds (the walker's store bound; the compose layer check) are
    host-memory-safety obligations. They stay in the audited set and need tests that can fail. HMM is
    not refused. (Audit S1-05 is narrowed accordingly.)
- **The rule is keyed on the channel's privilege, which kayfabe always knows.** Owner, verbatim: *"what
  you do know is if the channel is privileged or not. channel guest says is unprivileged cannot contain
  a full vram map ever, forbidden. real hardware must be told, so ogkm indicates it, as on bare metal
  channel userd and the ring and push buffers are mapped in an unprivileged cpu process, so the gpu is
  told any data at those addresses must be confined without ogkm checking it as it doesn't inspect in
  the first place. right? so then we should too. this is a critical flaw worth fixing right?"*
  - On bare metal, RM never inspects a user pushbuffer. Confinement is the hardware's job: the
    channel is unprivileged, and its VA space holds only what the kernel mapped for that process.
    kayfabe gives the same guarantee the same way. The host VA space that runs work from a channel
    the guest created unprivileged holds only rows derived from the guest's page tables: never a
    window, never kayfabe memory. The guest's RM sets each channel's privilege, and kayfabe services
    the channel allocation, so the decision needs no knowledge of what the space will later hold.
  - Audit S1-21 (the windows mapped in every mirrored space) breaks this rule and is **critical**. The
    fix is P1 + P2: no window in any space an unprivileged channel uses, and Translated work in its
    own space as above.
- **Also the same day:** *"We must check iova addresses are supported in kayfabe for guests requiring
  iommu protection. Not that this becomes a hard retrofit later."* A guest DMA address (an IOVA under a
  guest vIOMMU) is a fourth kind. It must be translated to a GPA at one validated boundary before any of
  the rules above apply. The readiness check is on `v3-viommu` (`design/V3_VIOMMU.md`).

## R. Memory safety: the `_unsafe` file is the audit perimeter (2026-10-03)

- **Owner, verbatim:**
  - *"safe code cannoy hold a raw pointer, either get rid of it or declare it as unsafe code and
    ensures it validates if safe code callers use it, do whats best"*;
  - *"we need to limit the amount of unsafe code, just logic may not need unsafe code"*;
  - *"the main invariant to hold, to protect against memory bugs, is that the vast majority of safe
    code that contains a bug and calls into any unsafe code is bound checked into that function. not at
    every call site. its a protection at us, a few validation sites is easy to audit, at every call its
    not then its basically unsafe code declared as safe."*;
  - *"if you would export an abritary read memory at vmm offset + length address as normal rust
    function, unchecked, and then expect safe code to call it correctly, you basically have a function
    that can dereference pointers without unsafe { ... }. so it breaks the compile time security rust
    provides. ideally by only auditing unsafe code we can conclude that with rust + that audit (assume
    the audit was perfect) that memory bugw cannot occur, without an audit on safe code. the same way a
    memory bug should not be possible in java/python/js/C# code ... i would say if the file ends with
    _unsafe, unvalidated length+offset or raw ptr is allowed, incl validation functions, even though you
    didn't need rust's unsafe block. then if safe code calls it, its validated, and that file gets a lot
    ot audit attention. so unsafe block is used as I needed this otherwise rust doesn't compile, if its
    not needed dont use it even in an unsafe file. unsafe file means this file can violate memory
    safety. so unsafe rust block not allowed in safe files is one gate, but not the whole actual
    security promise for memory bugs."*
- **Rules:**
  - A file named `*_unsafe.rs` is the audit perimeter: "this file can violate memory safety". It may
    hold raw pointers, unvalidated offsets and lengths, and the validation code itself, including code
    that needs no `unsafe` block. Code outside the perimeter must be memory-safe for every input by
    construction, so it needs a logic review only, never a memory-safety audit.
  - Every function a perimeter file exports to safe code validates all of its own inputs: overflow,
    range in the real allocation, alignment and lifetime. A buggy safe caller cannot cause a memory
    error. A precondition left to callers is forbidden; it is unsafe code declared safe.
  - `unsafe {}` is used only where Rust does not compile without it, inside the perimeter too.
  - No raw or disguised host or device address outside the perimeter. The console frame's address
    (S1-03) and kf-cuda's device-memory functions (S1-04) are being converted on `v3-sec-rawaddr`.
    The owner keeps the console display feature.
- **Proposed by Claude the same day, and ADOPTED by the owner the same day** (*"Do your
  suggestions/you think is best as well"*):
  - (a) Inside the perimeter, mark a function with an unchecked precondition `unsafe fn` even when
    its body needs no `unsafe` block, so every caller states why its arguments are valid.
  - (b) Code that produces an address hardware will dereference (GPU page-table entries, RM ioctl
    structs, kernel launch arguments, `mmap` targets) is perimeter material even though it compiles
    as safe Rust.
  - (c) Ratchet the perimeter's size (lines and exported items), not only `unsafe` blocks.
  - (d) `kf3.c` is entirely inside the perimeter and must compile in CI.
  - Also adopted, from the same proposal: the CI gates that enforce this model.
    1. No `unsafe` outside the perimeter, in a form that cannot be bypassed (audit S1-01).
    2. No raw or disguised address outside the perimeter (S1-02; built on `v3-sec-rawaddr`).
    3. A reviewed table of every function the perimeter exports to safe code, each with the checks it
       performs and the test that shows each check.
    4. Private fields on handle types that carry addresses.
    5. No `Copy` or `Clone` on handles that own memory.
- **Standing merge approval** (owner, the same day, verbatim): *"you have full approval to merge only
  it ci passed, code is good and it passed on real hardware with a real boot and apps. you are free to
  merge in a candidate merge branch and then test that, or check per branch, what you prefer, as long as
  the end state as a whole in main is tested"* and *"then if thats true, you dont have to ask to merge"*.
  The bar for master, for code and security-policy changes alike: CI green; the code reviewed; and the
  exact commit master will point to has passed, on a box with a real NVIDIA GPU, a real guest boot, the
  merge bar (tests, gates, bare metal, guest suite) and the apps. A candidate branch that merges several
  lanes is tested as a whole, at its own head.
- **Testing before master** (owner, the same day): *"before you merge to master, test it works"*, *"on
  real hardware"*. Code reaches master only after CI and a merge bar on a box with a real NVIDIA GPU,
  at the exact commit, plus that change's own hardware tests. Docs-only commits are tested by CI.

## S. Privileged host management: stub; unprivileged features: implement; GPU work: never forge (2026-10-07)

**STATUS: LIVE, 2026-10-07.** Binding owner ruling, relayed by the coordinator during the Windows
Code43 work (`traces/windows_code43_walls_20261007/README.md`, runs 25-27). It refines §H.

1. **Stub.** A privileged host-management action that is not compute and has no display or compute
   effect on the guest may be accepted or armed silently. It keeps a coherent "feature absent"
   state and never touches the host. Examples: RC-recovery policy (`SET/GET_RC_RECOVERY`), and
   arming notifiers for thermal or cooler diag zones, the power connector, platform power mode, aux
   power, GPU RC reset and ucode reset. Guest queries in the same area stay coherent with the stub:
   refused or reported absent, never filled with invented values.
2. **Implement for real.** Anything kayfabe can do with unprivileged host access is implemented,
   not stubbed: LUTs, timeslice, display colour, and the virtual monitor including hotplug and
   resize. An event about such a feature is armed only if kayfabe can post it from its own
   state. Otherwise the gap is reported.
3. **Never forge.** GPU work that reached the hardware is never forged (§A.3, unchanged). An
   event about real GPU work (copy-engine or graphics completions, runlist preemption, P-state or
   power events) is accepted only if it is derived from real host events.


**§S applied to specific notifiers (owner, 2026-10-07, after the VFIO event census).** These
are the four indices the real GSP does post in vfio-8/9/10:
- **PSTATE_CHANGE (33):** a no-op. The host does power management. Arm silently, never post.
- **HDCP_STATUS_CHANGE (34) and AUDIO_HDCP_REQUEST (45):** there is no HDCP, and none can be
  forwarded. Arm silently and never post. Every HDCP-related answer reports it as absent or
  unsupported, consistently.
- **RUNLIST_PREEMPT_COMPLETE (139):** the host driver does real preemption. Arm silently for
  now. If Windows issues the preempt control (in the VFIO boots: FIFO_DISABLE_CHANNELS with a
  `pRunlistPreemptEvent`, first at RPC index 3307), serve it only as a real unprivileged
  preempt of the VM's own channel group, and post the completion only from the host's real
  completion. Never invent one.

**§S applied to guest-kernel GR work (owner, 2026-10-07, relayed by the coordinator after runs
29-31; `traces/windows_code43_walls_20261007/README.md`, run31 and the GR-tier sections).**
Windows' first kernel-channel work is FERMI_TWOD_A and KEPLER_INLINE_TO_MEMORY_B methods on its
kernel GR channel, plus a software-subchannel bind on its kernel CE channel.
1. **Unprivileged host channel only.** Kernel-GR work may run for real only on an UNPRIVILEGED
   USER host channel with access to the VM's mirrored guest memory and VA space. No privileged host
   channel, no privileged RM verb, no host action taken from guest bytes. At birth the host channel
   is verified and asserted USER, and the birth is refused otherwise (tested).
2. **Re-author from an allowlist.** The rewriter re-authors host methods from a per-class allowlist
   (starting with exactly the observed 2D and I2M methods; one class/method set at a time). It
   never copies guest pushbuffer words. Every address operand is validated as a virtual address
   inside the VM's space. A method outside the allowlist, a privileged-state method, and any
   unbounded read or emit is refused by name and kills only that channel. Hostile guest, guest root
   included. Default-off flag until proven.
3. **Native first.** Each new class/method set is validated on the borrowed host with the native
   oracle (real GPU completion, resources freed, host display healthy) before any Windows run.
   Strictly serial, one VM or oracle at a time.
4. **Software subchannel bound to a non-class value** (5-7): accept the bind silently and refuse any
   later software method on that subchannel by name (logged). The hardware behaviour this matches
   is inferred, not tested.
5. **No forged completion.** Completions come only from real GPU work on the host channel. No CPU
   executor for GPU work.
6. **Standing rules.** Power, thermal, process, preempt and P-state stay host-owned stubs; no HDCP;
   faults depend on the UVM plan (stop and report if reached); FIFO_DISABLE_CHANNELS preempt, when
   reached, only as a real unprivileged preempt of the VM's own channel group; no new emulated
   kernel channel without telling the owner.
7. **Completion data and interrupts (added the same day).** GPU completion data must come from the
   real GPU writing GP_GET and semaphores into guest-visible memory (kayfabe is not in the
   completion path). Completion interrupts are relayed host non-stall event → guest vector (the
   interrupt plane maps GR0 and the other engines). For the GR tier, verify and report both, include
   them in the native validation and in the README of the first run that executes GR work, and fix
   the relay before relying on it if it is missing.

⊘ *Confirmed by the owner 2026-10-07 (relayed by the coordinator), above the text it settles:* the
`PERF_GET_POWERSTATE` `AC` stub is fine as it is. The entry below is no longer an assumption; its
"owner to confirm" and "not an owner decision" are answered by this line.

**§S applied to `PERF_GET_POWERSTATE` (2026-10-07) — ASSUMED from the power ruling, owner to
confirm.** `NV2080_CTRL_CMD_PERF_GET_POWERSTATE` (`0x2080205a`) answers `NV2080_CTRL_PERF_POWER_SOURCE_AC`
(`kf_rm::vfguest`). This is treated as a §S.1 stub under item 6's "power … stays host-owned": the
answer is the vGPU-guest body's constant (`subdeviceCtrlCmdPerfGetPowerstate_VF`,
`ogkm-580.65.06: src/nvidia/src/kernel/gpu/perf/kern_perf_ctrl.c:293-308`), never a host reading,
and nothing reaches the host. §S.1 also says queries in a stubbed area are "refused or reported
absent", which an `AC` answer is not; the classification is the coordinator's reading of the
ruling (compliance audit `2026-10-07-code43-gr-derive-compliance.md` S6), **not an owner
decision**. If the owner rules otherwise, the control is refused instead.

**§S exception: a diagnostic BAR0-read trap (owner APPROVED 2026-10-07, relayed by the coordinator
after the run35 stop; `traces/windows_code43_walls_20261007/README.md`, "Stop: two runs after A and
B").** An explicit, scoped exception to "only BAR0 writes trap" (`AGENTS.md`, Rules;
`design/THE_CONSTRAINTS.md`):
1. **Default off.** Behind a flag (`KF3_BAR0_TRACE=1`). With the flag off, production behaviour is
   unchanged: BAR0 reads never exit.
2. **Scoped.** Reads trap only from a guest-kernel channel's `GPFIFO_SCHEDULE` (the last one before
   the teardown is the one that matters) to the first following `Free`.
3. **Bounded.** At most 4096 accesses recorded per run.
4. **What it logs.** Every BAR0 read (with the value served) and write in that window, doorbell
   writes included, plus a one-time dump of 64 words of the new channel's USERD and its error
   notifier.
5. **Nothing else changes.** A trapped read is answered with the value the untrapped read would
   have returned; nothing is forwarded to the host.
Implemented as `crates/kf-qemu/src/bar0trace.rs` (ABI 21's `kf3_set_read_trap`). A window there
closes at the next RPC's queue-head write, so each window lies inside the approved interval.


**§S applied to `0x2081010d` (2026-10-07) — ASSUMED from the owner's 2026-10-07 statement that
privileged non-compute actions can be stubbed; owner to confirm.** Owner, 2026-10-07 (relayed by the
coordinator): *"Most actions that are not compute, privileged actions can be stubbed."* and *"All about
power control, thermal and process, preempt management is host"*.
- **What is stubbed.** Exactly one control: `0x2081010d`, answered `NV_OK` for a Windows 580.88 guest
  (`kf_rm::hoststub`, default on; the default-off `KF3_DIAG_ZERO_OK` diagnostic is removed). Its id is
  generated (`kf_abi::hoststub`, `tools/windows-ctrl-export/derive.py`): the interface from OGKM
  580.65.06 `FINN_NV2081_BINAPI_INTERFACE_ID` (`g_finn_rm_api.h:425`), the message number 0x0d from the
  bisect, `paramSize 0` and flags from the control's export row in the pinned retail driver.
- **Why only this one.** Runs 36-41 (2026-10-07, `traces/windows_code43_walls_20261007/README.md`)
  answered 13 refused power/thermal/perf/clock-area queries with zero-filled `NV_OK` and bisected
  them: `0x2081010d` alone is sufficient for StartDevice to pass VFIO 2861; the other twelve are not
  needed and stay refused. The four with public layouts (`0x2080205b`, `0x20802068`, `0x20802801`,
  `0x00800106`) are among the twelve, so no invented or zero-filled reply is shipped for them.
- **What the answer claims.** Nothing but the status: the control has zero params, so "zero-filled"
  invents no value. Nine of the thirteen had no public layout at all; for this one even the meaning is
  not public. A web search (2026-10-07) found no public layout. vfio-10's reply bytes exist and are
  not shipped as a captured table (derive-never-capture).
- ⚠ **For the owner.** The retail export row says the control is `NON_PRIVILEGED |
  ROUTE_TO_VGPU_HOST | GSP_PLUGIN_FOR_VGPU_GSP` (flags `0x10208`), and its area (`NV2081` binary API)
  is not shown to be power, thermal or P-state. So "privileged host management" is an assumption about
  its purpose, not a reading of it. Options: (a) keep the stub; (b) issue it on kayfabe's own host
  objects for real (it is non-privileged, but its effect on the host GPU is unknown); (c) refuse it,
  which returns Windows to Code43 at VFIO 2861.

## T. Directives recovered from earlier sessions (filtered 2026-10-08)

**STATUS: LIVE, 2026-10-08.** Recovered from earlier sessions (an audit of the owner's messages,
2026-10-07; branch `docs/owner-directives-20261007`, commits `d548a7a8` and `49b99d1e`), FILTERED
2026-10-08 under the owner's rule (§W: later rulings win; v1/v2 directives excluded). Unmeasured claims
stay as the original states them and are marked as recovered. Of 13 recovered entries, 5 are kept
(T.1, T.2, T.3, T.11, T.12); the numbers are the original ones, because `traces/windows_code43_walls_20261007/README.md`
cites T.1. Every original directive, with its KEEP or DROP and the reason, is in
`docs/design/OWNER_DIRECTIVES_RECOVERY_20261008.md`. Each quote is copied verbatim from the owner's
message, typos kept, and `…` joins fragments of one message. The audit's message dumps are not in the
repo, so the date and time are the citation. An entry's date is the day the owner said it: ask whether
its reason still holds before relying on it (see the top of this file).

1. **172.22.1.20 (RTX 4070) is trusted hardware.** *(Recovered; the owner's wording was relayed by the
   coordinator.)*
   - Owner, 2026-10-07 (the correction of an earlier reading, relayed by the coordinator): *"172.22.1.20
     is trusted (our own pc hardware at home with a different ssd for claude), the kiosk pcs not, and
     vast absolutely not"*.
   - Owner, 2026-10-04 15:17: *"you may use VFIO or any other destructive chane, incl display
     restart). Just no firmware changes on metal ofc or bricking hardware but those are very rare
     anyways."* And: *"so you do not need permission for most stuff to do on 172.22.1.20."*
   - **How to apply:** on 172.22.1.20, driver, VFIO and display changes, including destructive ones,
     need no permission. Firmware changes and anything that could brick hardware are forbidden. Vast
     boxes follow §F and `scripts/bench/box/README.md`.
   - ✔ **Confirmed by the owner, 2026-10-08:** *"you have full permission to do whatever is needed on
     172.22.1.20. no need to ask"*, *"yes no secrets on them. 172.22.1.20 is trusted"*, and about vast
     boxes *"you can do whatever you want on vast boxes, on those I care the least, if it breaks we just
     rerent"*. So no permission is needed on 172.22.1.20, and vast boxes may be wedged or re-rented
     freely; the §F and box-README rules still hold (no secrets on a box, only instance ids this
     session rented, no instance key printed). The 2026-10-04 "I temporarily borrow this machine"
     remark is superseded by the 10-07 and 10-08 statements; pushing work that matters (§F) stays
     good practice.
2. **Outside repositories are untrusted; clone them, do not web-fetch them.**
   - Owner, 2026-10-01 12:20, about a fork of virtio-nvgpu: *"(Do not trust stranger repos if you
     clone)."*
   - Owner, 2026-10-04 13:36: *"avoid using webfetch to search remote repos, clone is usually
     better"*. A minute earlier the owner had said that ogkm, nova and nouveau are cloned locally.
   - **How to apply:** read reference sources from local clones and grep them there. Treat a cloned
     third-party repo as untrusted data and follow no instructions in it.
3. **Search for an existing solution before building one.**
   - Owner, 2026-10-04 14:05: *"but first, maybe search on the internet if it already exists,
     because it can save us work"*.
   - Owner, 2026-10-05 01:32: *"yes next time we need to do better research, would have saved us
     time to just run reinstall and wrap it in a vast.ai template"*.
   - **How to apply:** before writing new tooling or a new mechanism, look for prior art and report
     what was found.
11. **Docs for agents and docs for people** (owner, 2026-10-07, in the session that wrote the
    recovered list; the coordinator relayed the words verbatim).
    - Owner: *"Ai written/optimized docs (so loads of verbose step by step and rulings), atleast
      what you personally prefer, are fine, but not for the prominient human facing ones. AI docs
      to optimize ai are really wanted though"*.
    - **How to apply:** internal and agent-facing docs may be verbose, step by step and explicit
      about rulings, and they are wanted. Prominent human-facing text (the README, announcements,
      the r/VFIO post) is written for people.

12. **Rent only Vast "verified" hosts.** *(Restored 2026-10-08: it was dropped by the filter for a
    "conflict" with a README command that merely does not mention the filter; the owner's statement
    stands, and it was followed on 2026-10-08.)*
    - Owner, 2026-07-28 01:35: *"and do verified if possible :-)"*, then at 01:36: *"to have some
      trust"*.
    - Owner, 2026-07-30 17:32: *"and only use verified hosts."*
    - **How to apply:** filter Vast offers to verified hosts (`verified=true` in the offer search).
      The other box rules are in §F and in `scripts/bench/box/README.md`.

## U. The deferred API (class 0x5080) is Translated-only; the doorbell is not a boundary (2026-10-07)

**STATUS: LIVE ruling; one open question for the owner (U.4).**

1. **Class 5080 is Translated-only.** Owner, 2026-10-07: *"So then my ruling will be api defer for
   translated only."*
   - **How to apply:** a class-5080 alloc on a Passthrough channel is refused `NOT_SUPPORTED`; on a
     Translated channel it is admitted, and the `0x200` trigger (`DeferredApiV2`, data = the
     `hApiHandle` registered with `NV5080_CTRL_CMD_DEFERRED_API[_V2]`) is serviced by a
     host-authored equivalent, never forwarded as guest bytes. There is no channel-creation flag that
     announces the class (ogkm `alloc_channel.h` `NVOS04_FLAGS_*`; the class is a `KernelChannel`
     child, `RS_FLAGS_ALLOC_NON_PRIVILEGED`, `resource_list.h:1525`), and a Passthrough ring cannot be
     converted mid-life, so the alloc RPC is the control point.
2. **The doorbell is not a synchronization boundary.** Owner, 2026-10-07: *"No diff at doorbell,
   doorbell doesn't provide any safe boundary for you other than translate and ring on host, it only
   provides "everything before this rung is now scheduled to run after it", it doesn't even provide
   a boundary to you to run code before a channel advances, as this can be done without doorbell or
   worse a doorbell of another process just sending the same token (everyone can write), plus perf
   wise its bad, doorbells are the most optimized piece of code. Nothing can be injected there."*
   - **How to apply:** ordering against a channel's progress exists only in streams kayfabe writes.
     A deferred `DMA_INVALIDATE_TLB` is therefore a host-written gate: an acquire on a host-owned
     semaphore at the `0x200`, released by the VA-manager thread after its diff is committed, then the
     host invalidate. Nothing blocks on a vCPU and no completion is forged. (Whether the Translated
     plane can emit that acquire is unverified; it is the first step of the implementation.)
3. **An unprivileged-origin deferred-API channel**, if one is ever needed, must not accept physical
   operands and must not map VRAM beyond what is assigned to the channel, as for Passthrough. Owner,
   2026-10-07: *"Such channels should absolutely not accept phys operands or map the entire vram
   beyond whats assigned to the channel much like passthrough."* Today every host channel is born
   with `DENY_PHYSICAL_MODE_CE` (`kf-host/src/channel.rs:234`); the VAS restriction (the channel's
   own VAS, not the GPGA VAS) is not built, and no creation-time signal exists to choose it, so such
   a use is refused until it is needed.
4. **Open for the owner.** The 2026-10-07 measurement (`traces/deferred_api_falsify_20261007/`,
   driver 595.91.07, not the 580 interval) shows a host client registers the guest's chosen
   `hApiHandle` verbatim (only `0`, the client handle, the firmware-reserved range and its own live
   handles are refused) and that two clients have independent namespaces. That is the owner's own
   condition for "match the handle, no translated handling". Ruling U.1 predates the result and is
   unchanged. Matching would still not cover entries whose side effects kayfabe owns (the TLB
   invalidate), and nothing in the scanned userspace needs the class (below), so there is no demand
   for the Passthrough route. **Owner to say whether U.1 stays as written.**

**Evidence behind U (measured unless marked).**
- No Linux guest run allocated the class: 250 trace files carry `AllocClassNotPermitted` lines
  (class 5080 was not on the allowlist before 2026-10-07); class 20608 (`0x5080`) appears only in
  Windows runs 16 and 17.
- A byte scan of the NVIDIA userspace (595.91.07 on the host; 610.43.02 CUDA compat) finds none of
  the four control ids `0x50800101..104` in libcuda, OpenCL, NVENC, NVML, NVCUVID, the GL/EGL/GLX
  cores, Vulkan SC, `nvidia-smi` or the MPS server; positive controls (`GPU_GET_GID_INFO`,
  `GR_GET_INFO`, `BUS_GET_INFO_V2`) hit. 580.x Linux libraries were not scanned.
- The same scan on the Windows 580.88 driver package (`DriverVer 32.0.15.8088`, `nv_dispi.inf_amd64_fe5f369669db2f36`
  in the baseline Windows image, 103 files; read-only, static presence only): `nvcuda64.dll`,
  `nvcuda32.dll`, `nvopencl*.dll`, `nvml.dll`, `nvcuvid*.dll` contain **none** of the four ids, so the
  Windows CUDA user-mode path does not register deferred entries. **But the Windows graphics user-mode
  driver does contain them:** `nvwgf2umx.dll` has `DEFERRED_API_V2` (`0x50800103`) x10 and
  `nvdxdlkernels.dll` has all four plus `DMA_INVALIDATE_TLB` and `GPU_PROMOTE_CTX`. `nvlddmkm.sys`
  (the positive control for this scan) has all four, `DMA_INVALIDATE_TLB` x7, `GPU_PROMOTE_CTX` x9
  and `GR_CTXSW_ZCULL_BIND` x6. `REMOVE_API` (`0x50800102`) alone also appears in `NvPresent64.dll` x26,
  `nvoglv64.dll`, `nvcudadebugger.dll` and others with no registration id beside it; that is not
  taken as use of the class (inferred: an unrelated constant). The earlier statement that Windows
  uses the class only from its kernel driver is therefore wrong for D3D; how the UMD's ids reach the
  RM (through the kernel driver) and on which channel kind is not measured.
- Of the eight commands the trigger can run, `DMA_INVALIDATE_TLB`, `GR_CTXSW_ZCULL_BIND`,
  `GR_CTXSW_PM_BIND`, `GR_CTXSW_PREEMPTION_BIND` are `NON_PRIVILEGED`; `GPU_PROMOTE_CTX`,
  `GPU_INITIALIZE_CTX`, `FIFO_UPDATE_CHANNEL_INFO` are `PRIVILEGED`; `GPU_EVICT_CTX` is kernel-only
  (ogkm flag words). Registration does not check the inner command; the check runs at trigger with
  the registrant's privilege (`deferred_api.c`). Execution on GSP GPUs is in physical RM (inferred).
- A stray `0x200` with no 5080 object raises Xid 32 on the firing channel only and the channel is
  RC'd; no MMU fault reaches nvidia-uvm. Not measured: cross-object, cross-channel and cross-client
  isolation, and rate (the probe's CE channel could not bind a software object).

## V. GPU UUID per VM and GPU; a VMM-neutral input trait; Translated channels forward unknown entries (2026-10-08)

- **GPU UUID** (owner, 2026-10-08): the guest-visible UUID is a hash of the host GPU's UUID and the VM
  id, one value per VM and GPU: stable enough across restarts, never the host's own UUID. The user may
  supply an explicit UUID per GPU (`gpu-uuid=`). Purpose as before: orchestrators must not see two VMs
  with the same GPU id (`docs/design/V3_GPU_UUID.md`). Of the four sub-decisions A-D there, the owner
  answered "a hash of host GPU UUID and VM id, per VM x GPU": that keeps the implemented identity
  source (`vm-id=`, else QEMU `-uuid`), the `slot` (PCI `devfn`) byte, and `auto` needing the host
  UUID. ⊘ Two points are the implementer's defaults, not stated by the owner: with no VM identity a
  random UUID for that boot with a named warning (B), and a refusal to realize when the host UUID is
  unreadable (D). Launchers in `scripts/` must pass `-uuid` so a VM keeps its GPU UUID.
- **Display broker and input** (owner, 2026-10-08): the broker's keyboard, pointer and cursor logic
  hooks onto the VMM through a full VMM-neutral trait. Policy (bounds, grab, absolute/relative choice,
  button and wheel routing, re-sync) lives in Rust in `kf-broker`; only a thin shim names the VMM.
- **Translated channels, unknown entries** (owner, 2026-10-08, "ok go ahead"; the oracle's evidence is in
  `traces/phys_operand_oracle_20261008/`, branch `claude/phys-operand-oracle-20261008`; still a DRAFT ruling): a known push-buffer entry in a Translated channel is
  inspected and its physical operands translated and checked; an unknown entry is forwarded as
  virtual-address-only. Conditions: the host twin is unprivileged with an address space that holds only
  that VM's memory; hardware refuses physical operands on an unprivileged channel for that engine class
  (measured per class, by the oracle; a class it does not clear stays allowlist-only); entry framing and
  lengths are bounds-checked from the push-buffer header for every entry; guest-controlled integer
  arithmetic panics rather than wraps (`overflow-checks = true` in the release profile); every
  forwarded unknown method is logged and counted. Isolation inside one VM is the guest kernel's;
  VM-to-VM and VM-to-host isolation is the host channel's privilege and address space.
  - **Evidence, 2026-10-08 (RTX 4070, driver 595.91.07, oracle rev `8c084ab7`):** [measured] on a
    `USER`-privilege channel the copy engine (class `0xC7B5`) refuses a PHYSICAL operand, source and
    destination, local FB and coherent sysmem, with or without the extra deny setting: Xid 32, channel
    reset, nothing delivered, while the VIRTUAL control is delivered. [OGKM reading, not measured] the 3D
    (`0xC997`), compute (`0xC9C0`/`0xC6C0`), video (NVDEC/NVENC/OFA) and host/FIFO methods have no
    physical-aperture operand at all, so a forwarded unknown entry there cannot encode a physical
    address and containment is the VA space. Not done: a privileged-channel positive control (no safe
    way to create one on that host; "honoured on a privileged channel" stays inferred).

## W. Answers to the 2026-10-08 open decisions

All owner statements of 2026-10-08, in the order the open-decision list was given.

- **nvkvm-pv broker diffs: apply** (owner: "yes apply"): keep the sub-pixel remainder per axis in
  `relptr_motion` (a relative delta below one pixel must not truncate to nothing), and hide the host
  cursor in `wl_set_grab` wherever the pointer is when the grab starts (one cursor, not two).
- **Colour: "No color correction" (610 SAT_MODE 3) versus mode 2:** *test and decide: we should prevent
  incorrect results.* Until the hardware test says it is the same, mode 3 stays refused.
- **Arbitrary chroma correction: keep it out** while apps do not use it and the screen renders correctly
  (it would not apply to screen sharing either); revisit only if a real workload needs it.
- **Segmented capability bits (`LOGNR`) for Windows: the implementer decides** (decision: not declared
  until a Windows run shows it asks).
- **`EVICT_CTX` under ruling B (§U):** a control kayfabe authors on the VM's own host twin, safe by
  construction; the owner's reading is that Windows shares OGKM's behaviour and does not depend on the
  eviction result in guest memory. Accepted as the direct act; "Windows never reads the context buffer
  back" stays an inference until a D3D run shows otherwise.
- **RUSD (`0x20800afe`/`0x20800aff`) and `utilization.gpu`:** the owner asks for GPU utilization to be
  served read-only (`nvidia-smi` is unprivileged on bare metal); power is not important. Direction: the
  values the guest sees are this VM's own, never host-wide quantities (§S exposure rule); the design
  and its measurements are open work (`docs/STATUS_AND_HANDOFF.md`).
- **Software-runlist flag (`KF3_SW_RUNLIST_HOST_OWNED`):** stays off and undecided.
  - ⊘ *Later idea, not for now (owner, 2026-10-08):* "for B ... kayfabe can quota VMs
    scheduling/fairness, is related." If kayfabe owns the scheduling of the host twins (B), a per-VM
    share, timeslice or fairness quota has a natural place; a guest-built runlist, which B ignores,
    could then never raise a VM's share. Whether the host exposes the needed controls to an
    unprivileged client is not checked (inferred open question).
- **`raw_control_native`:** under review, see the handoff; production code must not call it.
- **§T (recovered directives):** later rulings always win over recovered ones; keep only what is useful
  and does not conflict; **avoid directions from the v1/v2 kayfabe architecture** altogether.
- **Disks:** the 13 GB `base.qcow2` original under `/data/paguro-work.old` is deleted ("fully
  regeneratable"; the archive copy remains); `/workspace/nvidia-gpu-passthrough` is backed up to
  `/mnt/windows-work/archive/`.
- **Models:** Sonnet 5.5 by default, Opus 5.5 as the strongest tier (`CLAUDE.md`, *Models by risk*).

## Y. A diagnostic BAR0 trace mode with the VFIO reference's own tracer (2026-10-09)

⊘ Numbered **§X** on `claude/kf3-read-trace-20261008` (and in its code comments and `V3_BAR0_TRACE_MODE.md`); renumbered
§Y at the merge into `claude/windows-reset-20261009` (2026-10-09), because §X is the non-stall ruling of 2026-10-08.

**STATUS: LIVE, 2026-10-09.** Owner rulings of 2026-10-09, relayed by the coordinator. A second,
broader exception to "only BAR0 writes trap" (`AGENTS.md`, Rules; `design/THE_CONSTRAINTS.md`) than
§S's scoped window; it does not replace §S.
1. **What is allowed.** For diagnostics, kf3's BAR0 **reads may exit** (trap) to the VMM and be
   logged; boot slowness does not matter.
2. **Default off.** In every default, production and performance configuration the BAR0 mapping, the
   exit behaviour and the code path stay exactly as before: "only BAR0 writes trap; nothing blocks on
   a vCPU" stays the default. The mode is an explicit, loudly reported diagnostic exception, like the
   existing probe flags.
3. **The same tracer as the VFIO reference** (owner emphasis, same day: "using THE SAME tracer is the
   most valuable thing"; "inject the same PCIe tracer used for VFIO into kayfabe's device, so both
   give the same responses in the same formats"). Not a parallel look-alike: the observer is made a
   device-agnostic helper that both the patched vfio-pci and kf3 call, with the same properties and
   output, so the `scripts/bench/windows` harnesses and analysers work unchanged for both.
4. **Hostile guest bytes.** The trace has a hard cap on log volume (bytes and records), a drop counter
   reported at the end, and never grows memory or disk without bound per guest action.

Implemented (branch `claude/kf3-read-trace-20261008`, `docs/design/V3_BAR0_TRACE_MODE.md`): the
environment switch `KF3_BAR0_READ_TRACE=1` (ranges, caps: `crates/kf-qemu/src/readtrace.rs`), QEMU's
own `vfio_region_read`/`vfio_region_write`/`vfio_msi_interrupt` trace events called by `kf3.c`, and the
shared GSP observer (`tools/vfio-gsp-observer`, `gsp_observer_*`) behind kf3's `x-gsp-observer`
property.
## X. Non-stall interrupts wake every VM that armed the event; an edge is never dropped (2026-10-08)

Two owner statements on the same day; the second supersedes the gate/bucket part of the first.

- **First (2026-10-08):** remove the doorbell requirement from the Passthrough non-stall relay
  (`kf_chan::ptnsi`, branch `claude/passthrough-interrupt-20261008`): doorbells are guest-controlled and a
  weak boundary (the guest can ring to open the gate; the doorbell hook is slated to be replaced), and the
  1 s afterglow could lose completions. The host notifiers are GPU-wide anyway: RM delivers an engine's
  non-stall edge to every client registered on it, whoever's work it was. ⊘ *Superseded the same day:*
  this statement also asked for a live-twin condition and a per-vector token bucket; see the next item.
- **Second, binding (2026-10-08): follow NVIDIA — an interrupt wakes everyone.** Losing an interrupt for
  relevant work is a correctness bug; a cross-tenant wake is only a minor denial of service. So:
  - forward every host `FIFO_EVENT_MTHD` edge and every engine-notifier edge to every VM whose guest has
    **armed** that event (its own non-stall subscription, a host-recorded fact), not "has a live twin",
    not a doorbell, not outstanding work;
  - **invariant: an edge may be delayed, never dropped.** No dropping token bucket. Default: no pacing.
    Any pacing knob is env-tunable, default off, and loss-free (a pending flag per VM and vector, and a
    guaranteed trailing raise by a timer even if no further edge arrives);
  - keep the hostile-index refusal and the armed-event check, and `KF3_PT_NSI_RELAY=0` as the falsifier
    mode; counters: raised, coalesced-and-later-raised, no-armed-event;
  - the accepted residuals are written down: a tenant can make other guests wake more often (minor DoS,
    accepted, it is RM's own semantics) and a guest learns, by timing, that some tenant used an engine
    class it armed. Where: `docs/design/the_three_channel_kinds.md` §1.2 and `docs/FAQ.md`.
