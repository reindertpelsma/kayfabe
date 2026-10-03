# Owner rulings — the decisions that govern kayfabe v3 work

**STATUS: LIVE, 2026-09-30 (evening).** Every ruling the owner made in the 2026-09-25 … 09-30 working sessions,
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
      the one reserved host-VRAM object (`crates/kf-qemu/src/mem.rs:1-30`). Only unmapped pages show
      the per-window scratch memfd, which exists because a memslot hole kills the guest.
  - (superseded, kept as written) ⚠ **Found 2026-10-03, and it concerns all of kf3 on arm64, not only the ROM.** arm64 KVM maps a
    non-cacheable PFNMAP memslot as Normal-NC only when the host VMA carries `VM_ALLOW_ANY_UNCACHED`,
    and as Device memory otherwise (Linux 7.1 `arch/arm64/kvm/mmu.c:1966-1968`). vfio-pci sets that
    flag (`drivers/vfio/pci/vfio_pci_core.c:1831`); nvidia.ko never does (no occurrence in
    `ogkm-580: kernel-open`). kf3's BAR1 views are nvidia.ko mmaps, so on an arm64 host the guest
    would see the framebuffer and every BAR1 view as Device memory: no write combining, and an
    unaligned access faults. The likely fix is the one-flag host patch in nvidia.ko's mmap path,
    since a host patch is already required for UVM (`design/V3_COOPERATIVE_TIERS.md`).

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
  - **X11 display-SW paths** (windowed GLX, X11 Vulkan FIFO): host RM owns the release timing, so the
    bound needs a kayfabe lever. If clients use `NV9072_CTRL_CMD_NOTIFY_ON_VBLANK` (`0x90720101`), it
    reaches kayfabe and kf-disp services it at the configured rate. If they use software methods,
    kayfabe paces only the channels that own a display-SW object, through the trapped-doorbell path.
    That bound is per submission, not per frame, and is not promised until a box run shows it holds.
  - The status line reports the achieved rate per path, so a bound that does not hold is visible.

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

