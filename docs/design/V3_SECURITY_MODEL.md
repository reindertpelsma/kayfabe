# V3 security model

**STATUS: LIVE, 2026-10-03.** The security model of kayfabe v3, by role. Written in stage 1 of
`V3_SECURITY_AUDIT_PLAN.md` (area §2.5) and refined by the other four stage-1 areas (§2.1, §2.6,
§2.7, §2.10). It expands the plan's §1 table; §9 below says which of that table's rows hold today.
Findings named here (`S1-xx`) are recorded with their checks in
`docs/audits/2026-10-03-v3-stage1.md`.

This is a source audit. Everything below was read, not run, unless a line names a run and its date.
Revisions read: `origin/v3-owner-questions` `12a526df` (its code is identical to `origin/master`
`7c9234d2`), `v3-sec-nonpriv` `55743ecd`, `v3-sec-p0` `ea5890e9`, `v3-broker` `be58f7c9`;
nvkvm-pv `368d2db`; ogkm-580.159.04. Master has since merged `v3-gop` (`24762a7b`); that branch is
stage-2 scope and is not covered here.

## 0. What changed from nvkvm-pv and kayfabe v2

- **One process.** kf3 is linked into the VMM. There are no isolate children, no scratchpad process
  and no IPC (`THE_ARCHITECTURE_v3.md` §1). Sandboxing that process is the deployer's job (owner,
  2026-10-03; `OWNER_RULINGS.md` §P).
- **Intent is constructed, not forwarded.** Every host RM parameter block is built by kf-host from
  named scalars; host verbs take no guest flags word (`crates/kf-host/src/lib.rs:8`; rule A.2).
  Guest RM calls are answered by kayfabe's fake GSP (`kf-gsp`, `kf-rm`).
- **Guest GPU work still runs verbatim on the host GPU**, on passthrough twins, in a host VA space
  mirrored from the guest's page tables by a GPU walker that runs in a CUDA context inside the VMM.
- **Guest memory is live under every read.** There is no trap-and-lock layer any more;
  copy-then-check-then-use is the rule (`THE_CONSTRAINTS.md` §39).

## 1. Assets

| asset | why it matters |
|---|---|
| host memory: the VMM process, the host kernel, other host processes | breakout target |
| the VMM's own address space as seen from its in-process CUDA contexts | under unified addressing or HMM a GPU address can be a VMM address (S1-05), so kayfabe's own GPU kernels guard host memory |
| host VRAM outside this VM's store; other tenants' GPU contexts | cross-tenant target |
| kayfabe-owned VRAM: Translated rings, walker pools, display staging, broker frames, the firmware carve-out holding kayfabe's BAR1/BAR2 roots | a write there defeats a kayfabe boundary |
| the guest kernel's memory and other guest processes' memory, seen from guest userspace | in-guest isolation (owner rule A.9), the value proposition |
| host GPU availability: engines, channel IDs, NVENC sessions, BAR1, RM/GSP throughput, the walker | shared by every tenant, or by every process in the guest |
| the host desktop, through the broker | the broker's own model (nvkvm-pv) |

## 2. Roles

### R1. Guest userspace (untrusted; includes containers inside the guest)

**Can reach.**
- The doorbell and usermode pages: writes trap; one host page is readable without an exit.
- Passthrough pushbuffers, executed on the host GPU in its own twin VA space.
- Kayfabe's control decoders. An honest guest kernel forwards many user-issued controls to "GSP"
  with the user's parameter bytes (`ogkm-580: src/nvidia/src/kernel/rmapi/resource.c:250-293`), so
  `kf-rm` parses data chosen by unprivileged guest userspace (S1-24).
- Store pages the guest kernel maps for it (BAR1).
- The walker, indirectly: its own VA layout decides how much walk work a refresh does (S1-61).

**Must hold.**
- **R1.1** It reaches no memory the guest kernel did not map for it: not another guest process's,
  not the guest kernel's (A.9). This includes translations the guest kernel has since removed: no
  stale translation may outlive the guest's unmap.
- **R1.2** It reaches no kayfabe or host memory.
- **R1.3** A write to a user-mappable page other than the doorbell does nothing
  (`crates/kf-trap/src/trap.rs:106`).
- **R1.4** It cannot make the VMM block or do unbounded work (no queues or degradation paths on
  user-writable pages, `THE_ARCHITECTURE_v3.md` §2.1).
- **R1.5** It cannot deny GPU service to another process in the same guest beyond what a local
  process on bare metal could.

**Status today.**
- ★ **2026-10-04 (P1+P2, `V3_P1P2_TSPACE.md`):** the fix is built on `v3-p1p2` behind `KF3_TSPACE`
  (default OFF): with the flag no mirror (twin) carries a window or a ring, and Translated work runs
  in the per-VM T-space. R1.1 holds for window reach only once the flag is default-on (inc E) after
  the REGRESSION A/B and T-WINDOW-USER box steps; the channel-privilege half rests on P0 (S1-20).
  Guest leaves into the firmware carve-out are counted on twins (`carve_gpu=`) and refused at inc A2.
  ⊘ CORRECTED 2026-10-04 (implementation review): with `KF3_TSPACE=1` they are already REFUSED in
  every twin a guest non-kernel channel may run in (only counted in a guest-KERNEL space, where no
  channel runs under T-mode); on the default path they are counted only, until inc A2.
- R1.1 does **not hold**: the identity windows are mapped in every twin (S1-21, client audit P1).
  On VER3 page-table formats (Hopper, Blackwell) the walker also publishes stale 4 KiB translations
  under an unmapped big PTE (S1-60). Once P1 is fixed, a late invalidate with no teardown (S1-81)
  becomes the leading in-guest exposure.
- R1.2 rests on USER host channels refusing physical-mode and privileged work, which is
  UNVERIFIED in v3 (S1-20).
- R1.3 holds by design.
- R1.4 holds on the trap path; guest-driven log volume is uncapped (S1-86).
- R1.5 does **not hold**: walk resources are shared per refresh, so one space can fail the walk of
  every space batched with it (S1-61).

### R2. Guest kernel / guest root (untrusted to the host; trusted by the guest's own processes)

**Can reach.**
- BAR0 privileged registers, through the ordered ring, which poisons when full
  (`crates/kf-trap/src/ring.rs:21-35`).
- The fake GSP: init arguments, message-queue geometry, RPC elements and their reassembly (bounded:
  `crates/kf-rm/src/rmrpc/reasm.rs:90`, `:106`).
- Guest page tables, read by the walker.
- Translated copy-engine pushbuffers, which kayfabe rewrites.
- Emulated display channels (`kf-disp`).
- PRAMIN and BAR window placement; USERD and notifier offsets; object and channel counts.
- Device removal, on a hotplug-capable PCI topology (S1-80).

**Must hold.**
- **R2.1** It harms only its own VM.
- **R2.2** No mapping leaves the store or registered guest RAM (`crates/kf-mem/src/apply.rs:297-310`;
  RM's own object bound, `ogkm-580: src/nvidia/src/kernel/mem_mgr/virtual_mem.c:1273`).
- **R2.3** It cannot reach kayfabe-owned memory.
- **R2.4** Every guest-steered host resource is bounded per VM and refused by name past the bound
  (§7).
- **R2.5** Every RPC is eventually answered, never on a vCPU, and never with a forged OK
  (`THE_CONSTRAINTS.md` §39(g); A.3).
- **R2.6** Every guest-memory read is copied once and decided on the copy (`THE_CONSTRAINTS.md` §39(a)).
- **R2.7** Hostile page tables cannot make the walker loop, read outside the store, or emit a leaf
  outside the guest's own memory; every refusal is named.
- **R2.8** It cannot free or unplug the device while kayfabe's threads still use it.

**Status today.**
- R2.2 holds by construction: `apply_entry` refuses every row outside the store, outside guest RAM
  or over a VMM placement, and the walker contains every vidmem leaf before coalescing, with a
  known-positive build (`KF_BREAK_BOUNDS`, `cuda/walk/Makefile:67-84`).
- ★ **2026-10-04 (P1+P2, `V3_P1P2_TSPACE.md`):** on `v3-p1p2`, USERD and error-notifier offsets
  outside the usable heap are refused before any host call (S1-43, inc A); the methods that carry
  an unauthored address are refused by name and `SubDeviceMask` headers refused (inc A); with
  `KF3_TSPACE=1` every word a Translated channel emits is authored and every address it
  dereferences is a window address computed by kayfabe, with the engine's whole footprint checked
  in the perimeter (inc C/D), the rings live in the T-space outside every emitted address, and the
  store window ends at the carve-out (a 2 MiB-page part and a 4 KiB tail, so no page reaches it).
  R2.3 holds for the ring and carve-out reach once the flag is default-on (inc E) after the
  T-RING-TRANSLATED box step.
  ⊘ CORRECTED 2026-10-04 (implementation review): inc A's refusals are COUNT-ONLY on the default
  path (`inca[strict=no counted= heap_out= rows_inexact=]`): they refuse with `KF3_INCA_REFUSE=1`
  or `KF3_TSPACE=1`, and become the default only after box step 1 shows the counts at 0 on each
  measured family — "every default" above was wrong for this branch's head.
- R2.3 does **not hold**: Translated rings are reachable through forwarded virtual operands
  (S1-23, client audit P2). USERD and error-notifier offsets can also name the firmware carve-out
  (S1-43).
- R2.4 is partial (§7).
- R2.6 is gated for the walker PTX only (`check_s39.py`); the Rust readers rely on API shape
  (S1-27).
- R2.7 holds for termination, load bounds and leaf containment. Open gaps: the budget is a flag
  rather than a work bound (S1-63), stale walk-table runs after a truncation are contained only by
  the host's refusal of the whole report (S1-62), and no hostile corpus reaches the production walk
  path (S1-65).
- R2.8 does **not hold** on a hotplug-capable topology (S1-80).

**Out of scope:** guest root harming its own VM or its own processes (`THE_CONSTRAINTS.md` §39(b)).

### R3. The VMM process: QEMU + kf3 + in-process libcuda (the host trust boundary)

**Must hold.**
- **R3.1** No host CPU address is guest-chosen or guest-visible (A.5).
- **R3.2** Raw pointers live only in `*_unsafe.rs` (A.6; `THE_CONSTRAINTS.md` §13). Host addresses
  cross safe code only as the opaque `kf_linux_raw::HostSpan`, backend fds only as `BackendFd`.
  Exceptions found: the console frame address as a `usize` (S1-03), kf-cuda device addresses in a
  safe public API (S1-04), RM's map cookie (S1-15). The lexical gates that enforce §13 can be
  passed by code that breaks it (S1-01, S1-02).
- **R3.3** Every safe-to-unsafe boundary checks its inputs: no unsafe block has a precondition its
  safe caller is trusted to meet (the axiom at `crates/kf-linux-raw/src/lib.rs:119`). Gaps: the RM
  companion size fields (S1-40), `view_bytes`/`zeroed` (S1-09), the FFI handle helper (S1-08).
- **R3.4** Kayfabe needs no host privilege, and when the VMM holds `CAP_SYS_ADMIN` kayfabe does not
  pass that privilege to guest-driven work. Built for channel births on `v3-sec-nonpriv`
  (unmerged): an effective-capability bracket around each channel allocation plus a
  `PRIVILEGED_CHANNEL` reply tripwire. **Not covered:** every other kf-host RM call, and the
  libcuda channels of the walker and display contexts (S1-22).
- **R3.5** Kayfabe's own GPU kernels (the walker and the display compose kernel) stay inside their
  buffers. Where the in-process contexts can address pageable host memory, this is a host memory
  safety property, not only a GPU one (S1-05).
- **R3.6** No blocking on a vCPU, or under a lock another thread blocks on (A.4).
- **R3.7** Memory safety under hostile input. Any guest-triggered memory-safety bug in kf3 or
  `kf3.c` **is in scope**: it is the breakout.
- **R3.8** Every RM call kf3 issues is authored, from a closed set, non-privileged per ogkm-580, and
  its reply is parsed with bounds (§4). The set is closed today by code review only (S1-41).

**Out of scope:** what an attacker does after owning the VMM, and how the VMM is confined
(`docs/archive/guest_blast_radius.md` §1.1; `OWNER_RULINGS.md` §P).

### R4. The display broker (a separate process; console authority)

**Authority.**
- Authenticated by peer uid: 0, QEMU's effective uid at each connect, or `display-broker-uid`
  (`v3-broker` `docs/design/V3_DISPLAY.md` §8.5).
- It may inject input, resize the guest's mode and power the guest off. This is intended: it is
  the console (S1-33).

**Must hold.**
- **R4.1** It receives only sealed frame copies and the cursor image, never guest RAM or the store
  (`OWNER_RULINGS.md` §L).
- **R4.2** kf3 accepts no file descriptors from it and bounds every field it sends.
- **R4.3** Frame objects never sit in the guest-facing RM client, never in a twin or kernel mirror,
  and are never kernel-mapped (`OWNER_RULINGS.md` §N; client audit).

The broker in turn treats the VMM as untrusted (nvkvm-pv
`docs/internal/audit-broker-security-2026-08-27.md`). `v3-broker` is unmerged and is stage-2 scope.

### R5. Other host-GPU tenants (other VMs, host processes, the host desktop, other kf3 devices in the same process)

**Must hold.**
- **R5.1** No guest-steered operation reaches their memory. Mechanisms: RM client separation per
  process; per-twin VA spaces; USER host channels (UNVERIFIED, S1-20); store-only rows.
- **R5.2** Exhaustion of shared host resources is bounded per VM (§7).
- **R5.3** Residual VRAM is scrubbed between tenants. This is inherited from host RM scrub-on-free
  and is UNVERIFIED end to end (S1-31).

**Shared fate.** kf3 devices in one QEMU process share that process, and share some process-global
state (S1-91). Separate VMs need separate uids, which is a deployment requirement (§11).

### R6. Host RM, nvidia.ko, GSP firmware, libcuda, nvidia-uvm, KVM, host kernel (trusted)

Their behaviour is cited from ogkm-580. Closed firmware and closed libcuda behaviour is marked
UNVERIFIED.

**Property P (from v2, restated).** Every effect a guest can produce on the host GPU is one a local
unprivileged process could produce. P is a comparison, not a safety claim:
- host driver bugs reachable from an unprivileged ioctl are reachable from a guest too;
- wedges and fairness lie inside P (`docs/archive/guest_blast_radius.md` §2, §5.1-§5.3).

In v2, P held because the RM-calling process held no capability. In v3 with a root VMM it holds
only through R3.4 and R3.8 together (S1-22).

## 3. The chain every host-boundary claim rests on

Each link must hold for "the guest cannot reach host memory or another tenant" to hold.

| # | link | where it lives | status |
|---|---|---|---|
| 1 | No guest bytes are forwarded to RM; every host struct is authored | kf-host verbs; the inventory in `docs/audits/2026-10-03-v3-stage1.md` §3.2 | holds by construction; the set is closed by review only (S1-41) |
| 2 | The raw layer re-validates what safe code hands it | `kf-linux-raw` `CharDevice::ioctl` (`chardev_unsafe.rs:565-776`) | holds for `_IOC_SIZE` and pointer placement; the companion size fields are the caller's (S1-40) |
| 3 | Every mapping is a store or guest-RAM row | `kf-mem` `apply_entry` (`apply.rs:297-310`); RM's object bound | holds |
| 4 | Guest work runs in per-twin VA spaces | kf-host twins | holds, but the identity windows (S1-21) and the Translated rings (S1-23) sit inside those spaces |
| 5 | USER host channels refuse physical-mode and privileged methods | closed GSP firmware; `DENY_PHYSICAL_MODE_CE` requested at birth (`crates/kf-host/src/channel.rs:603-611`) | **UNVERIFIED** (S1-20) |
| 6 | Kayfabe's own GPU kernels stay in bounds | walker I2 with a known-positive; compose kernel bounded host-side only | walker gated in its Makefile but not in CI (S1-68); compose unchecked in-kernel (S1-05) |

## 4. The RM call surface

A summary of the stage-1 inventory of area §2.6 (`docs/audits/2026-10-03-v3-stage1.md` §3.2).

- **One funnel.** Every NVIDIA ioctl goes through `kf_linux_raw::CharDevice::ioctl`, and only
  kf-host calls it. The funnel re-derives `_IOC_SIZE` and refuses an overrun, bounds and
  overlap-checks every pointer patch, and scrubs every minted address after the syscall.
- **Twelve escapes:** `CHECK_VERSION_STR`, `REGISTER_FD`, `CARD_INFO`, `RM_ALLOC`, `RM_CONTROL`,
  `RM_FREE`, `RM_MAP_MEMORY_DMA`, `RM_UNMAP_MEMORY_DMA`, `RM_MAP_MEMORY`, `RM_UNMAP_MEMORY`,
  `RM_ALLOC_MEMORY`, and `ALLOC_OS_EVENT` (in `event.rs`).
- **Privilege.** Every control id at a call site carries `RMCTRL_FLAGS_NON_PRIVILEGED` in
  ogkm-580's generated tables, and every allocated class carries `RS_FLAGS_ALLOC_NON_PRIVILEGED`,
  except the root client under a root VMM (client audit P0). The GSS-legacy controls are
  non-privileged under CPU-RM's mask rule
  (`ogkm-580: src/nvidia/interface/deprecated/rmapi_gss_legacy_control.c:55-60`); how GSP treats them is
  UNVERIFIED.
- **Guest-derived inputs are named scalars only:** GPU VAs, store and RAM offsets and lengths from
  walked PTEs, `gpFifoEntries`, an engine type gated to copy/GR/video, an engine class from the
  family's exact set, a PTE kind through an allowlist, RO/volatile bits, and six control scalars.
- **No guest value becomes a CPU pointer.** Two host addresses live in safe code: RM's map cookie
  (a host physical or bus address, S1-15) and the console frame address (S1-03).
- **A second, closed surface:** libcuda's own RM client, for the walker and display contexts. It is
  outside the funnel, the census and any allowlist, and runs with the VMM's capabilities (S1-22).
  `v3-broker` adds a direct `UDMABUF_CREATE`.

## 5. Attacks in scope

- Guest userspace to: another guest process, the guest kernel, kayfabe memory, the VMM, the host,
  other tenants.
- Guest kernel or root to: kayfabe memory, the VMM, the host, other VMs and tenants, host
  resources beyond a documented bound, and device lifetime (unplug).
- Guest to VMM liveness: vCPU stalls, lock stalls, unbounded work, unbounded logs.
- Guest-chosen data steering host RM into writing outside guest memory: error notifiers,
  display-SW releases, timed semaphores (client audit and its verify pass).
- Hostile page tables against the walker; hostile RPC and control bodies against the decoders.
- Broker to kf3: malformed messages; impersonating the broker.

## 6. Out of scope

- A compromised VMM and its confinement (owner, 2026-10-03).
- Confidential computing and attestation (`OWNER_RULINGS.md` §H).
- Host root, the host kernel, and physical attacks.
- Guest root harming its own VM; guest kernel to guest process.
- Timing and contention side channels between tenants: not studied; assume they exist (nvkvm-pv
  `SECURITY.md`).
- NVIDIA driver bugs reachable by any unprivileged local process: reported upstream, inside P.

## 7. Guest-steered shared resources and their bounds (input to plan §2.4)

| resource | who steers it | bound today | status |
|---|---|---|---|
| guest VRAM | guest kernel | the store, fixed at realize (`THE_CONSTRAINTS.md` w720h) | structural |
| RM graph handles | guest kernel | `MAX_LIVE_HANDLES` 2^18 (`crates/kf-rm/src/rmgraph.rs:805`) | bounded |
| privileged register ring | guest kernel | 4096, poisons when full | bounded |
| workers | realize | at most 255, asserted at startup | bounded |
| scratch memfds, mapping growth | guest kernel | `v3-scratch-bound` | fixed on branch (unmerged) |
| host channel IDs / twin count | guest userspace via guest RM | the host's per-runlist count only | **open** |
| twin-private host VRAM (RM-owned context buffers) | guest userspace via guest RM | uncounted (`crates/kf-qemu/src/cardbudget.rs:1-19` covers BAR1 and the store only) | **open, S1-82** |
| host NVENC sessions | guest userspace | none per VM (`crates/kf-qemu/src/chan.rs:2551-2598`) | **open, S1-42** |
| host BAR1 across VMs | guest kernel | this process only (`cardbudget.rs:10-13`) | **open** |
| PRAMIN view retire queue (fds and BAR1 aperture) | guest kernel | unbounded channel (`crates/kf-qemu/src/mem.rs:301`) | **open, S1-45** |
| runlist time (TSG timeslice) | guest kernel | host minimum only | **open, S1-44** |
| walk budget, frontier, report capacity | guest userspace (its VA layout) | per refresh, shared by every space in the batch | **open, S1-61** |
| display-SW objects, kernel-mapped bytes | guest kernel | caps in progress (`OWNER_RULINGS.md` §N) | **open** |
| host RM/GSP call rate | guest userspace via guest RM | none | **open** |
| host log volume | guest userspace and guest kernel | none on several paths | **open, S1-86** |
| GPU time | guest userspace | none; v2 recorded about a 2x slowdown under a spinning kernel (`docs/archive/guest_blast_radius.md` §5) | fairness, inside P |

## 8. Earlier guarantees that no longer apply without the isolate

| earlier guarantee | source | v3 status |
|---|---|---|
| Each guest process has its own host process and RM client | nvkvm-pv `docs/internal/isolate-model.md` | Gone. One guest-facing client per device holds every twin. Separation is per-twin VA space plus kayfabe's handle tables; client-scoped RM lookups must be shown to be VA-space-scoped (client audit verify pass). |
| A compromised stub is confined by seccomp, namespaces and a uid drop | nvkvm-pv isolate model | Gone. A kf3 bug runs as the VMM. Confinement is the deployment's. |
| P holds because the RM-calling process holds no capability | `docs/archive/guest_blast_radius.md` §3.1-§3.4 | Holds only for channel births (bracket plus tripwire, unmerged). Elsewhere P depends on the authored verb set and the GPU-side bounds while the VMM is root (S1-22). |
| The walker runs in an unprivileged isolate and cannot escalate | `THE_CONSTRAINTS.md` §20 | Stale. It runs in the VMM; its invariants are the only barrier (S1-26). |
| Layer 1 (trap and lock) secures guest-shared memory | `docs/archive/core_security_threat_model.md` §2.1 | Gone by design. Replaced by §39 copy-then-check (S1-27). |
| Guest pointers are overwritten at the boundary | nvkvm-pv `docs/internal/forwarding-model.md` | Superseded and stronger: nothing guest-authored is forwarded to RM. Guest GPU VAs still execute verbatim, bounded by the twin VA space. |
| The guest is never given a BAR | nvkvm-pv `SECURITY.md` | Changed. Store views and one read-only host usermode page are mapped into the guest (S1-30, S1-46). |
| The in-guest boundary is enforced by the guest kernel module | nvkvm-pv isolate model | Now the stock driver. Kayfabe must not open a path around it, which the identity windows currently do (S1-21). |
| Hostile-input instruments run nightly (fuzz, tsan, mutants, the adversarial guest kernel) | `docs/archive/fuzz_campaign_2026_08_01.md`; `docs/archive/the_adversarial_guest.md` | Retired at the v3 cutover; v3's decoders have no fuzz or property target (S1-83). |

## 9. Which rows of the plan's §1 table hold today

| plan §1 row | holds? | why not | findings |
|---|---|---|---|
| guest userspace | **no** | identity windows in every twin; stale VER3 translations; host reach rests on an untested channel property | S1-21, S1-60, S1-81, S1-20 |
| guest kernel / guest root | **no** | Translated rings reachable; several shared resources unbounded; device lifetime on hotplug topologies | S1-23, S1-25, S1-80 |
| the VMM process | **partly** | births de-privileged on an unmerged branch; libcuda and the other RM calls are not; pointer gates bypassable | S1-22, S1-01, S1-02 |
| the display broker | by design, unmerged | stage 2 audits `v3-broker` | S1-33 |
| other tenants | **no** | NVENC, twin VRAM, runlist time and BAR1 are not bounded per VM | S1-25, S1-42, S1-82 |
| host RM and driver | trusted | GSP and libcuda behaviour is UNVERIFIED where cited | §12 |

## 10. Accepted or owner-ruled risks

- **Guest doorbell helper** (design only, owner-ruled): guest root may ring any host token; a ring
  only makes the GPU re-read that channel's own `GP_PUT` (`OWNER_RULINGS.md` §D).
- **Wedge and fairness:** inside P. One guest per GPU partitions the effect.
- **Closed host modules:** supported, untested (`OWNER_RULINGS.md` §J).
- **Proposed, awaiting the owner:** the host clock disclosure through the read-only usermode page
  (S1-30): accept for a single tenant, disclose for multi-tenant, with one guest per GPU as the
  answer.

## 11. Deployment assumptions (to state in SUPPORT and SECURITY)

- A sandboxed VMM.
- One uid per VM.
- `CAP_SYS_ADMIN` dropped by the launcher (recommended; not a refusal, per the owner's ruling).
- Host scrub-on-free left enabled.
- Profiling restricted to admin users (the host default).
- No `--privileged` container.
- kf3 on a PCI topology where the guest cannot unplug it, until S1-80 is closed.

## 12. UNVERIFIED, each with its check

| assumption | check |
|---|---|
| USER host channels refuse physical-mode and privileged work | client-audit box test T-PHYS-CE, both privilege arms (S1-20) |
| Which VA space GSP names in the display-SW callback | T-DISPSW-SCOPE (client audit verify pass) |
| RM bounds USERD inside its object | a kayfabe-side bound (S1-43) |
| libcuda channel privilege, and which threads create them | the capability bracket around context creation, plus a privilege probe for every client in the process (S1-22) |
| Whether HMM or ATS is active for kf3's own CUDA contexts on target hosts | query `CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS` at bring-up and log it (S1-05) |
| Contents of the host usermode page beyond the timer | dump it per family (S1-30) |
| Scrub-on-free through kayfabe | the two-VM canary test (S1-31) |
| Host VRAM per twin, per family | count it on a box (S1-82) |
| The VER3 big-PTE chain end to end | a Hopper or Blackwell card; the project has none (S1-60) |
| How often a stock guest issues ALL_PDB invalidates | a census on the bench (S1-61) |
| QEMU 10.2's `memory_region_enable_lockless_io` disables the reentrancy guard | a realize-time assert (S1-90) |

## 13. Sources

nvkvm-pv: `SECURITY.md`, `docs/internal/isolate-model.md`, `docs/internal/forwarding-model.md`,
`docs/audit/README.md`, `docs/internal/audit-boundaries-2026-08-20.md`,
`docs/internal/audit-broker-security-2026-08-27.md`, `docs/internal/audit-guest-pointers.md`.

kayfabe: `docs/archive/core_security_threat_model.md`, `docs/archive/guest_blast_radius.md`,
`docs/archive/fuzz_campaign_2026_08_01.md`, `archive/nvkvm/docs/SECURITY_MODEL.md`,
`docs/design/THE_CONSTRAINTS.md`, `docs/design/THE_ARCHITECTURE_v3.md`, `docs/OWNER_RULINGS.md`,
`docs/design/V3_SECURITY_AUDIT_PLAN.md`, `docs/design/V3_VIOMMU.md`, `v3-broker`'s
`docs/design/V3_DISPLAY.md` §8, `v3-sec-nonpriv`'s `traces/v3_security/nonpriv_20261003/README.md`.

Also: the 2026-10-03 client and single-store audit and its verify pass (findings P0-P2 and the
`T-*` box tests), the stage-1 findings document `docs/audits/2026-10-03-v3-stage1.md`, and
ogkm-580 as cited inline.
