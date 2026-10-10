# V3 — the host patch list: every optional host-kernel/driver enhancement, in one plan

**STATUS: DESIGN-ONLY, 2026-10-10 (H5 since written and build-verified, same day: see §4.5 and `V3_H5_USERD_DMA_PATCH.md`).**
Nothing in this document is built, apart from what each entry says is already built (the b3
nvidia-uvm patch, the ioeventfd fast path, the USERD relay, the stock-path fallbacks, and the H5
patch file, which has been compiled but never loaded). It consolidates the candidates scattered over `V3_COOPERATIVE_TIERS.md`,
`V3_UVM_DEMAND_PAGING.md`, `V3_UVM_B3_IMPLEMENTATION.md`, `V3_VFIO_USER_FRONTEND.md`,
`V3_BATCHED_MAP.md`, `V3_USERD_RELAY.md`, `V3_DOORBELL_IOEVENTFD.md`, `OWNER_RULINGS.md` and the
Windows records under `traces/`. It does not supersede them; where one of them disagrees with this
page about a detail, that document's dated STATUS wins and this page is fixed.

Labels used throughout: **[read]** seen in source (ogkm, Linux, or this repo), **[measured]** in a
committed trace or in the output of a command run for this page, **[reasoned]** derived from numbers
that were measured elsewhere, **[inferred]** a belief that no one has checked, **[proposed]** a design
choice made by this page. An entry is never promoted from one label to a stronger one without a run.

## 1. The owner policy this page serves (binding, 2026-10-10)

1. **The stock install works with no kernel patch.** A stock host driver, an unprivileged VMM, no
   extra module. That is the acceptance bar for every release and every gate
   (`CLAUDE.md`, *Verification*; `V3_COOPERATIVE_TIERS.md` §2).
2. **A patched host kernel/driver is an optional, better tier** with a significantly better
   experience: the quirks of the stock path are removed, not worked around.
3. **The mdev shim** (a software vfio device, possible only as a kernel module) lets stock
   hypervisors (Proxmox, libvirt, OpenStack) use kayfabe without kayfabe's QEMU overlay.
4. **Secure Boot is no concern on Linux** (DKMS users sign with their own keys). **Windows driver
   signing is the hard one**, and the host tier avoids it: a host patch gives stock guest OSes the
   benefit with no guest-side driver (§9.4).
5. **With host patches, passthrough doorbells need not leave the kernel** (H1), which is where most of
   the performance for stock guest OSes is expected to come from.
6. The owner now plans to implement these enhancements, as a host kernel module patch. §7 is the
   order proposed for that.

Nothing in the tier may weaken the hostile-guest rules (`CLAUDE.md`, *Rules*): host actions are
authored from unprivileged RM verbs, never forwarded from guest bytes; nothing blocks on a vCPU;
no CPU executor for GPU work; no forged completions.

## 2. The candidates at a glance

| id | candidate | where the kernel change lives | stock fallback (correct without it) | effort | state |
|---|---|---|---|---|---|
| H0 | tier infrastructure: runtime probe, DKMS shape, manifest, fallback gating | kayfabe side + packaging | not applicable (it is what makes the others optional) | S | **not started**; ships with the first patch |
| H1 | in-kernel doorbell for Passthrough channels | new kayfabe module (variant B) or `nvidia.ko` (variant A) | ioeventfd + register drainer, else trapped write | M | design only; fast path it replaces is built |
| H2 | mdev shim: software vfio device for stock hypervisors | new GPL module (`kayfabe_mdev`) | kayfabe's QEMU overlay `kf3-gpu` | L | design only; four prove-first questions open |
| H3 | b3: external fault service in host `nvidia-uvm` (managed memory) | `nvidia-uvm.ko` (patch, ~1.3 kLoC) | no GPU demand faults; loud failure | L overall (patch itself S-M to port) | **host-only proof passes on hardware (580.159.04)**; guest side unbuilt |
| H4 | host-owned UVM pool (module-owned backing, CPU VA != GPU VA) | new module + `nvidia-uvm.ko` change | H3, else the fault-free pool in the guest (stage 2) or loud failure | L | design only; first experiment not run |
| H5 | USERD-in-sysmem address-size fix (Windows; Turing/Ampere/Ada hosts only) | `nvidia.ko` kernel-interface layer (`kernel-open/nvidia/nv.c`; applies to the packaged DKMS tree) | the USERD relay (built, measured) | S | scope ruled (D7, 2026-10-10); **patch written, build-only verified at 595.91.07, never loaded**; the kayfabe behaviour that uses it is not enabled (`V3_H5_USERD_DMA_PATCH.md`) |
| H6 | exact partial unmap of `NV01_MEMORY_VIRTUAL` mappings | `nvidia.ko` RM core (open source build) | per-leaf mappings, micro reservation if it passes, else 4 KiB grain; never re-make | M | candidate from source reading; **not owner-decided, not hardware-measured** |
| H7 | scatter-map verb (no stitched views) | `nvidia.ko` RM core | stitched OS descriptor, batched (built, measured nested) | M | candidate derived by this page; lowest priority |

Considered and **not** host-patch items (carve-out refusal, MMU-invalidate refusals, the arm64
mapping flag, IOMMU identity mode, the vfio-pci/QEMU tracer, vfio-user mmap changes, guest-side
stages): §6, so that no one re-opens them as patch candidates by mistake.

## 3. Rules common to every candidate

### 3.1 Detection is a runtime probe, never a build-time assumption

- **No `cfg`, no feature flag that the user must set to match the host, no reading of the driver
  version string to decide a feature.** A patched build reports the same version string as a stock
  one (`NV_ESC_CHECK_VERSION_STR`, `kernel-open/nvidia/nv.c:2694`), so the string cannot tell them
  apart. The version gate that exists (`kf_host::host_abi_gate`, `crates/kf-host/src/lib.rs`) decides
  whether the *stock* path is allowed, and stays as it is.
- **Each feature has its own probe**, with three outcomes: `Present(abi)`, `Absent`, `Broken`.
  `Broken` (unexpected errno, unexpected abi, a probe that hangs past a deadline) is treated as
  `Absent` and logged by name. Fail closed to stock.
- **The probe is made at VMM start, per GPU, and its result is one line in the status line** (`host
  tier: stock` or `host tier: patched [h1 h3 ...]`), so a run's evidence carries which tier it ran on.
  A bench claim that does not carry the tier is not a patched-tier claim.
- **A use-time refusal degrades one object, not the process.** The model is the ioeventfd
  registration: a registration the kernel refuses leaves that token on the trapped path, named and
  counted (`V3_DOORBELL_IOEVENTFD.md` §0, §5). Every patched feature needs the same test: exhaust it,
  and show that everything still works on the stock path.
- **Probes that cost a fault are forbidden.** A behavioural probe of H6 on a stock host would
  deliberately fault the probe's own channel (host Xid 31, `gpu_vaspace.c:1639` assert at exit). The
  patched components therefore answer a cheap query instead (below).

The probes proposed per component:

| component | probe | stock answer | patched answer |
|---|---|---|---|
| `nvidia-uvm.ko` (H3) | `UVM_INITIALIZE` with `UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE`, then `UVM_EFS_QUERY` | `NV_ERR_INVALID_ARGUMENT` (unknown flag) | `NV_ERR_NOT_SUPPORTED` when the module parameter `uvm_efs_enable` is 0 (patched, disabled); OK when 1, and `abiVersion` (1 today) **[read: `tools/uvm_efs/patch/uvm_efs_ioctl.h`]** |
| `nvidia.ko` (H5, H6, H7, H1-A) | one new escape in `nvidia_ioctl`'s switch (`nv.c:2598...`), `NV_ESC_KF_QUERY` (`NV_IOCTL_BASE+40`), returning `{abi, feature mask, per-feature abi}` **[built into the H5 patch, compiled; not run]**. Kayfabe side: `crates/kf-host/src/tier.rs` (probe, three outcomes, tests) | an unknown escape returns `-EINVAL` from `nv_validate_ioctls()` **before** the switch, and logs `NVRM:unknown NVRM ioctl command` once per probe **[read, 595.91.07 `nv.c:2404-2466`; corrected 2026-10-10, the old text cited the `default:` branch]** | `0`, abi and a mask with one bit per feature; each feature also has its own abi byte |
| new kayfabe module (H1-B, H2) | `open("/dev/kayfabe-host")` then `KF_HOST_QUERY` **[proposed]** | `ENOENT` (no node) | abi and a feature mask |

Detection by name, in this repo, does not exist yet: `crates/` has no EFS client and no host-tier
type **[measured: grep, 2026-10-10]**. H0 adds it.

### 3.2 Security: what each patch must prove

For every entry the question is the same: *what can an unprivileged VMM process, or a hostile guest
through it, do that it could not do on the stock host?* The rules the answers must satisfy:

- The new privilege is **named and bounded** (a count, a size, a set of objects the caller already
  owns). An unbounded read or emit reachable from the guest is a security bug (`CLAUDE.md`).
- Values the kernel acts on are **authored by kayfabe at registration time**, never taken from guest
  bytes at fire time.
- The patch is **off or absent unless the admin installed it**, and an installed-but-disabled patch
  answers the probe as such.
- **A patched component never trusts the VMM more than the stock one does.** `V3_UVM_DEMAND_PAGING.md`
  §0.1 states it for UVM (the module must not trust the VMM); this page extends it to every entry.
- A bug in a patch is a **host kernel bug**; each entry names what limits the blast radius.

### 3.3 Validation: A/B on one revision

- A patched-tier claim needs the **stock result and the patched result of the same kayfabe binary**
  (`kf3-bins/<rev>/`) on the same box, with the tier in the status line (§3.1).
- GPU-free first: every behaviour the kernel change relies on gets a model switch in the existing
  GPU-free models (`kf_mem::sim` for H5-H7) so that the property tests run under both the stock and
  the patched RM model.
- Then hardware, strictly serial, one box (`CLAUDE.md`, *Verification*): `v3_gates.sh` 9/9, the fast
  suite 30/30, with the patched component loaded and **also with it loaded but not probed-in**
  (probe forced absent), and the entry's own lane.
- Anything that reloads `nvidia.ko` stops the desktop on the trusted host. RM-core entries (H5-H7)
  are therefore first proven on a rented vast KVM-image box (`scripts/bench/box/README.md`: a
  container box "cannot load kernel modules").

## 4. The candidates

### 4.1 H1 — the in-kernel doorbell for Passthrough channels

**Fixes.** On the stock path a guest doorbell store is either a trapped BAR0 write, or (with
`doorbell-ioeventfd=on`, default off) a KVM ioeventfd match whose eventfd wakes the **register
drainer**, which then rings the host twin's token (`crates/kf-chan/src/dbfast.rs`;
`V3_DOORBELL_IOEVENTFD.md` §1). The expensive part that remains is the wake-up of a user thread
between the guest's write and the GPU, plus the CPU that thread uses. What the guest vCPU pays,
**[measured, trusted host, not nested, `V3_LINUX_PERF_BASELINE_6692e621.md` §4]**:

| store | p50 |
|---|---|
| matched doorbell, fast path off (trapped) | 6.1 us |
| matched doorbell, fast path on | 1.8 us |
| plain ioeventfd with no memslot (the floor) | 1.285 us |
| no-op MMIO handled in QEMU | 5.2 us |

Nested vast boxes, for scale: 20.4 us trapped, 15.7 us ioeventfd; the drainer's wake-up adds about
46 us when idle; LLM decode 0.29x to 0.32x of the host with the fast path on, and +16-18 % with the
default-off bounded-spin experiment (`KF3_DBFAST_SPIN_US`) which burns a whole core to remove that
wake-up **[measured, nested]**. A bare-metal wake-to-ring figure for the drainer has **not** been
measured; the owner's "much performance is won" is a hypothesis until it is.

**What the kernel change is.** On a doorbell eventfd signal, the kernel itself writes the host twin's
token to the host doorbell page, in the signalling context, with no thread in between. The binding
is what the ioeventfd registration already is (one per passthrough guest token, matched by value)
plus a host token and the host doorbell page. Two variants:

- **Variant B (recommended first), a new kayfabe module** `kayfabe_host.ko`, no `nvidia.ko` rebuild.
  A registration ioctl takes `{eventfd, host_token, pfn-source}` and parks a wait-queue entry on the
  eventfd (the shape of vfio's virqfd: it needs no KVM export). The wake function does one
  `iowrite32(token)` to a pre-mapped notify register and nothing else (no sleeping lock, no
  allocation). **The page the module may write comes from the caller:** the VMM passes the user
  address of the USERMODE page it already mapped through `nvidia.ko` for its own RM client; the module
  accepts only a writable `VM_PFNMAP|VM_IO` mapping of exactly one 4 KiB page, takes the PFN, and
  registers an MMU notifier on the caller's mm so that an `unmap_mapping_range` by `nvidia.ko` (object
  freed, GPU reset or unbind) revokes the binding synchronously. That revocation is the hard part:
  a stale PFN after a GPU hot-unplug would be a write into whatever owns that address next
  **[inferred]**; it is the first thing the review must attack. The notify offset in the page is the
  offset the stock path already uses (BAR0 usermode page `+0x90`, `V3_DOORBELL_IOEVENTFD.md` §1;
  Hopper+ BAR1 view per `V3_BAR1_DOORBELL.md`); it must be derived per family, never captured.
- **Variant A, inside `nvidia.ko`** (open-flavour source build only). A new RM entry takes
  `{hClient, hChannel, eventfd}`; RM checks the client owns the channel, resolves the token
  (`kchannelCtrlCmdGpfifoGetWorkSubmitToken_IMPL`, `src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3200`
  in 595.84) and the mapped page (`usermode_api.c`), and parks the eventfd entry. Lifetime and
  ownership are then RM's own; the cost is a full `nvidia.ko` build and a stack reload (§9.5).
  Line numbers are from the 595.84 tree; the trusted host runs 595.91.07, so re-locate them.
- Neither variant can add a KVM MMIO handler: `kvm_io_bus_register_dev` is not exported on kernel 7.0
  (`V3_COOPERATIVE_TIERS.md` §3.2, read from `/proc/kallsyms` 2026-10-03). The module rides on the
  ioeventfd that kayfabe's in-process device already registers with KVM.
- **Translated and Emulated tokens never use it.** Kayfabe must see those writes
  (`V3_DOORBELL_IOEVENTFD.md` §0); only Passthrough tokens (a host twin ring, no kayfabe code behind
  it) bind in the kernel.

**Interface and detection.** §3.1: `/dev/kayfabe-host` plus `KF_HOST_QUERY` (variant B) or
`NV_ESC_KF_QUERY` (variant A). Registration per token sits behind the dbfast registry: if the
module refuses a binding (limit, `ENODEV`, revoked), that token stays on the drainer path, named and
counted; if the module is absent, the whole fast path is the drainer's. The guest, QEMU, and the
trap path are not told.

**Security.** *New privilege:* a kernel-context 4-byte store of a kayfabe-authored token to a page the
caller already maps writable. Bounded because (a) the value is the registered host token, not guest
bytes; the guest only chooses *when* (by writing the matching value); (b) a ring only makes the GPU
re-read that channel's own `GP_PUT`, the posture the owner accepted (`OWNER_RULINGS.md` §D; that any
unprivileged CUDA process can already ring any token on the GPU it has open is **[inferred]**, not checked); (c) per-fd registration
cap, page and offset fixed, revocation on unmap/release. *Residual risks to review:* the wake function
runs in the signaller's context, which is **[inferred, verify on kernel 7.0]** the guest's vCPU thread
(KVM's ioeventfd write calls `eventfd_signal` from the vCPU, the way irqfd works), so the MMIO store
executes on a vCPU; a posted store does not wait,
but a GPU or link in a bad state could stall the bus. This sits against the rule "nothing blocks on a
vCPU" and needs an explicit owner judgement; the alternative (a work item) brings the wake-up back
**[reasoned]**. A guest firing at full rate costs host CPU per event; bounded by the vCPU's own
rate.

**Performance.** *Reasoned:* removes the drainer wake-up and its CPU; leaves the vCPU store cost at
the ioeventfd floor (1.3-1.8 us measured non-nested). The measured proxy for "no wake-up" is the spin
experiment's +16-18 % LLM decode (nested, one burned core); the module is expected to approach it
without burning a core **[reasoned, not measured]**. On a nested box the exit dominates and only a
guest-side stage removes it (`V3_COOPERATIVE_TIERS.md` §3.2).

**Validation.** *Step 0, before any module code (falsifier stated):* on the trusted host, measure the
drainer's wake-to-ring p50/p99 with the existing instruments (`V3_DOORBELL_IOEVENTFD.md` §7,
`scripts/bench/dbfast_lane.sh`, `dbfast_llm.sh`) under idle and deep-queue load. **If the wake-to-ring
time is within noise of the floor, H1 has no performance to give and is dropped to a small
robustness item.** Then: A/B on one binary, `ioeventfd+drainer` vs `module`: doorbell wake-to-ring,
LLM decode tok/s (`llm_parity.sh`), CUDA ladder, `dbfast_lane.sh`. Correctness: `v3_gates.sh` 9/9,
fast suite 30/30 with the module on, then with it loaded and probed out. Negative controls: `rmmod`
refused while a VM runs; SIGKILL of the VMM (registrations vanish, no Xid); GPU reset or
`nvidia` unbind during a run (revocation, no stray write: watch the host log and a guard page);
guest hammering the doorbell at full rate (bounded host CPU); hostile value writes (only matched
values fire).

**Effort.** M (module a few hundred lines; the larger share is the revocation proof, kayfabe's
probe/registration/fallback, and the tests).
**Dependencies.** H0. The ioeventfd path (built). Not on any other entry.

### 4.2 H2 — the mdev shim (software vfio) for stock hypervisors

**Fixes.** Today kayfabe is reachable only as `-device kf3-gpu`, i.e. through kayfabe's QEMU overlay
(`qemu/hw/misc/kf3/`). A stock QEMU under Proxmox, libvirt or OpenStack cannot use it. vfio-user does
not close the gap: the protocol has no server-initiated change of a BAR's mmap areas, so the BAR1
view churn would have to trap (functional, "far too slow") **[read: `V3_VFIO_USER_FRONTEND.md` §1, as
of 2026-10-01, to re-verify against the current spec]**. Fallback that stays correct: the QEMU
overlay (today's shipped path).

**What the kernel change is.** A new GPL module `kayfabe_mdev.ko` that registers mediated devices and
forwards accesses to an unprivileged kayfabe daemon. Only the shim is in the kernel; kayfabe's
guest-facing parsers stay in userspace. BAR mmaps are served from a page-fault handler, so pages can
change behind one static mapping (zap, refault; KVM follows through its MMU notifiers). Starting
points: `samples/vfio-mdev/`, and Nutanix's MUSER (find out why vfio-user replaced it before
building). It touches no NVIDIA file. The mdev parent can be a virtual device; `nvidia.ko` owns the
GPU's PCI function, so the shim cannot be that function's driver **[inferred]**.

**Interface and detection.** The daemon looks for the module's node and `KF_HOST_QUERY` at start. The
hypervisor side needs no detection: the admin creates an mdev of a type the module advertises
(`mdev_supported_types`) and the stock tools attach it.

**Security.** *New privilege:* the daemon gets pinned guest pages and serves faults into the guest's
BAR mappings. Bounds that must hold: (1) the shim's map call accepts only memory the daemon already
owns, else it hands an unprivileged process a way to map arbitrary physical memory
(`V3_VFIO_USER_FRONTEND.md` §2 option 0, item 4); (2) the fault handler **always resolves**, with a
scratch tile when the daemon is slow or dead, because a fault KVM cannot resolve fails `KVM_RUN` and
kills the guest; (3) the shim parses nothing a guest sends; (4) daemon death resets the device.

**Performance.** Unmeasured. "May cost less than kf3's memslot churn" is unverified. Doorbells: VFIO
has `VFIO_DEVICE_IOEVENTFD`; whether an unmodified QEMU arms it for an mdev region is open. If it does
not, every doorbell crosses QEMU, and H1 does not help because kayfabe's daemon holds no KVM VM fd
(`V3_COOPERATIVE_TIERS.md` §3.2, "Open for the mdev path").

**Validation.** First the four prove-first questions of `V3_VFIO_USER_FRONTEND.md` §2 option 0 as
kernel-side prototypes (IOVA pages importable by an unprivileged RM client; the always-resolving
fault handler; the doorbell ioeventfd; the ownership check on the map call). Then a new lane: stock
QEMU 10.x with `-device vfio-pci,sysfsdev=...`, the Linux fast suite 30/30, CUDA ladder, LLM, and a
Windows guest; A/B against the `kf3-gpu` overlay on the same revision; negative: kill the daemon mid
run, a guest touching every unmapped BAR page.

**Effort.** L (the largest entry). **Dependencies.** H0; the prove-first answers; independent of H1
except for the doorbell question.

### 4.3 H3 — b3: external fault service in host `nvidia-uvm` (managed memory)

**Fixes.** On a stock host a GPU demand fault in a guest's managed memory cannot be serviced: the
fault arrives in a host VA space where no UVM range can service it, and stock UVM cancels it
(`service_fault_batch_dispatch`, `NV_ERR_INVALID_ADDRESS`). Measured symptom: the four UVM failures of
the app matrix, 61/65 apps + 6/6 probes at `6692e621` (`UnifiedMemoryStreams`, `UnifiedMemoryPerf`,
`conjugateGradientUM`, `attach_verify`; `STATUS_AND_HANDOFF.md`, "Apps and CUDA ladder at
`6692e621`"). Owner-selected 2026-09-28: b3, with full ordinary host CUDA coexistence
(`V3_UVM_DEMAND_PAGING.md` §0).

**The stock fallback (correct without it).** Managed memory that the app prefetches or advises still
works (those calls never raise a GPU fault); a GPU fault fails **loudly** as a channel error and must
not corrupt silently (`V3_COOPERATIVE_TIERS.md` §2). *Known gap in that guarantee:* `conjugateGradientUM`
fails silently today (`STATUS_AND_HANDOFF.md`: "managed-memory faults must fail loudly"); this is a
stock-path defect to fix independently of the patch. Read-only duplicates fail loudly since `v3-roperm`
(719 + host Xid 31).

**What the kernel change is.** `nvidia-uvm.ko`, not `nvidia.ko`. Built and measured: a per-VA-space
mode (EFS) in one new file `uvm_efs.c` plus hook lines in stock files (`tools/uvm_efs/patch/`, 7 files,
30 hunks, ~1.3 kLoC). Hooks: the fault batch dispatch's `else` branch
(`uvm_gpu_replayable_faults.c`), `uvm_api_initialize` (`uvm.c`), and the teardown paths
(`uvm_va_space.c`, `uvm_va_space_mm.c`). A fault the stock code would cancel is copied into a bounded
per-file table and parked in hardware; the owner reads records (`UVM_EFS_WAIT`) and answers REPLAY or
CANCEL (`UVM_EFS_RESOLVE`). Everything else is stock code. It works on a closed-flavour host as well:
`nvidia-uvm` source ships in both packages (`OWNER_RULINGS.md` §J).

**Version state.** Written and proven against **580.159.04** **[read]**. *New for this page,*
**[measured, `patch -p1 --dry-run` in `kernel-open/` of an ogkm 595.84 tree, 2026-10-10]:** all hunks
apply to 595.84, with offsets of -2 to -56 lines and one hunk at fuzz 2 (`uvm_gpu_replayable_faults.c`
hunk 5). That is a textual result only: the semantics of the fault path in 595 must be re-read, and the
hardware proof re-run on 595.91.07, before the port counts.

**Interface and detection.** §3.1: the `UVM_INITIALIZE` flag probe and `UVM_EFS_QUERY`
(`abiVersion`). Off unless the admin loads with `uvm_efs_enable=1` **and** a file opts in at
`UVM_INITIALIZE` (which then requires HMM off for that file). Kayfabe-side client: none yet.

**Security.** *New privilege:* the owning thread group of an opted-in UVM file reads fault records
and issues replay/cancel. Bounded **[read + measured, `V3_UVM_B3_IMPLEMENTATION.md` §0.2]:** records
carry no pointer, PDB or host address, only an opaque kernel-issued id; every hardware action uses
state the kernel saved; the table is bounded (`uvm_efs_max_records`); a record is cancelled by the
kernel at a deadline (`uvm_efs_timeout_ms`) and at teardown; stale ids do nothing; a SIGKILLed VMM
with a parked fault tears down cleanly (`parked_at_shutdown=1 hw_cancelled=1`). Not yet covered: the
**production** guest fault plane (forged/stale handles from a guest through kayfabe), which is the
next audit surface. The per-fault GR hold is owner-flagged: the patch stays unmerged until that hold is
bounded (`OWNER_RULINGS.md` §E; `V3_SWEEP_AND_INSTALL.md` §2.7).

**Performance.** *Measured, host only:* a concurrent unrelated host CUDA process kept 96 % of its
throughput (203.6 vs 211.9 iters/s) with `managed_bad=0`. *Reasoned only:* 0.3-1 ms per fault batch
through a guest (`V3_UVM_DEMAND_PAGING.md` §4.4), 10-20x bare-metal UVM. Fault-scoped walks are an
open owner decision.

**Validation.** Per-tag `patch --dry-run` in CI (early warning; `V3_SWEEP_AND_INSTALL.md` task I8);
re-run `tools/uvm_efs/` on a vast KVM-image box at 595.91.07 (and the 580 box); then the guest side
(`V3_UVM_DEMAND_PAGING.md` §5, unbuilt steps 5b-5d). A/B: the app matrix with and without; the claim
"4 of 4 now pass" is a hypothesis until run.

**Effort.** Port and per-tag upkeep S-M; the whole feature L, because the guest fault plane and the
UVM-owned twin VAS are unbuilt. **Dependencies.** H0; the guest fault plane.

### 4.4 H4 — host-owned UVM residency (module-owned pool, CPU VA != GPU VA)

**Fixes.** Even with H3 the guest services its own faults; the owner's longer-term design has the
host do every migration in both directions, so the guest never holds a GPU fault
(`V3_COOPERATIVE_TIERS.md` §5.1). Stock fallback: H3 if present, else the fault-free managed memory of
the patched *guest* driver (§5.3 there), else loud failure.

**Kernel change.** (a) A host module that owns the backing pages of a dedicated pool, served to stock
QEMU as a second guest-RAM block (`memory-backend-file`); its fault handler is the guest CPU trap
(memfd cannot do that: no hook short of userfaultfd, ruled out 2026-09-14). (b) A real UVM change to
decouple CPU and GPU addresses for registered ranges ("a UVM software restriction, not a hardware
one", owner). Files: **to locate** in `nvidia-uvm` (the CPU-VA-equals-GPU-VA assumption sits in the
va_range/va_space code; `V3_UVM_STATE_MACHINE.md` has the map).

**Detection / security / performance.** As H3 plus the module's own: the handler must never fail (a
fault KVM cannot resolve kills the guest); no guest page-table reads. Performance unmeasured, no
reasoned number beyond H3's. **Validation.** First experiment (`V3_COOPERATIVE_TIERS.md` §5.4 item 1):
does KVM fault cleanly through the module's file (get_user_pages, MMU notifier zap, refault)?
**Effort** L. **Dependencies** H3 and the stage-2 guest driver; post-release by owner ruling
(`OWNER_RULINGS.md` §I).

### 4.5 H5 — the USERD-in-sysmem address-size fix (Windows)

⊘ **Corrections and status (2026-10-10, from reading 595.91.07 and 595.84 and building the patch; they sit above the text they correct;
full reasoning, security review and test plan: `V3_H5_USERD_DMA_PATCH.md`).**

1. **Scope ruled and implemented as option (a), in a narrower sense than written.** `DELEGATED_DECISIONS_20261010.md` D7 answers the
   "owner ruling to obtain first" below: the §AB.2 exception covers only a DMA address-size constraint on the memory a guest declares
   as USERD; nothing is mapped into any GPU VA; relaxing the size check is forbidden. **[read]** The IOVA of an OS descriptor is
   assigned *inside* the `NV_ESC_RM_ALLOC_MEMORY` call that creates it (`osmemdesc.c:296` -> `memdescMapIommu` -> `osIovaMap` ->
   `nv_dma_map_alloc` -> `dma_map_page_attrs`/`dma_map_sg`), long before a channel names the memory as USERD; the RM core has no way
   to re-map a part of it (`io_vaspace.c:258-340`, sub-mappings are subsets of the root mapping). "A constraint for USERD memory
   only" is therefore implemented as **a per-allocation constraint requested by the creator of the OS descriptor**
   (`NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW`), and kayfabe chooses which descriptors (the guest's USERD pages; or, simpler and wider, the
   guest-RAM object). Option (b) is not shipped. The patch is 4 files and maps nothing.
2. **[read] The wall exists on Turing, Ampere and Ada only.** The USERD pointer is 40 bits there (`_GV100`, `_GA100` checks); on Hopper
   and Blackwell it is 52 bits (`dev_ram.h:28` gh100/gb10b) and the 47-bit IOVA fits. The width to request is derived from that field.
   The trusted host's GPU is an RTX 4070 (AD104, Ada) **[measured, read-only, 2026-10-10]**.
3. **[read, replaces "[inferred]"] The IOVA allocator.** `iommu_dma_alloc_iova` takes `dma_get_mask(dev)` at every mapping and allocates the
   highest free range at or below it (`dma-iommu.c` v7.0). RM already lowers the mask for one allocation once, for the flush buffer
   (`kern_mem_sys_gm107.c:300-322`); H5 is that technique, per call, under a lock.
4. **[measured] No full-source rebuild tier is needed for H5.** The change is in `kernel-open/nvidia/`. The host's packaged
   `/usr/src/nvidia-595.91.07` is identical to the tag's `kernel-open/` apart from the prebuilt RM-core objects, the patch applies to it
   (dry run) and builds against it (`BUILD_RC=0`, kernel 7.0.0-34), and the patched `nvidia.ko` has byte-identical export CRCs. (Still
   true for H6/H7.) §5 and §9.1 are corrected accordingly.
5. **[measured, read-only ssh] The GPU's IOMMU group (11) reads `identity` on the trusted host now**, where the patch answers
   `NOT_TRANSLATED` and the wall is not present. The A/B needs the group at `DMA-FQ` (`V3_H5_USERD_DMA_PATCH.md` §6.0).
6. The query escape is built (`NV_ESC_KF_QUERY`); the kayfabe probe is built and logs the tier; **the behaviour that would use it (adopt the
   guest's USERD instead of the relay) is not enabled**, condition in `V3_H5_USERD_DMA_PATCH.md` §7.

*The original text of this entry follows; where it disagrees with the list above, the list wins.*

**Fixes.** A Windows per-process channel declares its USERD as a 512-byte slot in guest system
memory. A Passthrough twin adopts it through kayfabe's guest-RAM OS descriptor; the host IOMMU
(`DMA-FQ` domain) maps that page at an IOVA such as `0x7ff9_f969_b000`, and host RM refuses the
channel: `kchannelCreateUserdMemDesc_GV100: physical addr size ... is incorrect!` →
`NV_ERR_INVALID_ADDRESS` **[measured, run 61 at 883f878e]**, from `kernel_channel_gv100.c:211-219`
(580) via `kchannelIsUserdAddrSizeValid_HAL` **[read]**.

**Important correction to a natural reading.** The check mirrors a **hardware field**: the USERD
pointer in the runlist entry is `USERD_PTR_LO` (bits 31:8) plus `USERD_PTR_HI_HW` (8 bits), 40 address
bits in all (`dev_ram.h:27-28` Ampere; `dev_pbdma.h:26-27` Volta) **[read, 595.84 headers]**. An IOVA of
47 bits cannot be programmed whatever the software check says. *Relaxing the check would truncate the
pointer and point the GPU at the wrong memory; that is not a valid patch.* The fix must make the DMA
address of that memory fit in 40 bits.

**Stock fallback (correct without it).** The USERD relay, built and measured (`V3_USERD_RELAY.md`;
runs 68-72: the compositor's channel runs on an unprivileged twin, 98 doorbells relayed in run 71, the
engine consumed every entry): a kayfabe-owned 4 KiB video-memory USERD, two 4-byte cursors relayed by
a worker, `GP_PUT` bounded to `[0, gpFifoEntries)`, `GP_GET` written back (with `KF3_RELAY_GET_REFRESH`).
It maps nothing into a GPU VA. The other no-patch option is an admin setting, a host IOMMU identity
domain for the GPU's group (`KF3_USERD_RELAY_OFF=1` then adopts the guest slot); it was ruled out on
the dev host because it touches the owner's live desktop GPU. Linux guests never meet this (their
USERD is in video memory).

**What the kernel change is.** `nvidia.ko`, open-flavour source build. The device's DMA mask is set by
RM through `nv_set_dma_address_size` (`kernel-open/nvidia/nv.c:3273-3290`, 595.84), which sets the
device mask to the GPU's physical address bits; the IOMMU allocator then hands out IOVAs down from that
mask ⊘ **[read, see correction 3]**. Two ways, ⊘ **[chosen: (a), see correction 1]**: (a) a per-allocation 40-bit constraint
for memory that will be a USERD (⊘ **located: the OS-descriptor creation call, `osmemdesc.c:296` and `nv-dma.c:594`**); (b) cap the device
mask at 40 bits when the IOMMU translates (IOVAs below 1 TiB are plentiful; without an IOMMU,
physical memory above 1 TiB would bounce). (b) is smaller and wider in effect; (a) is narrower. The
`NV_ASSERT(0)` on the refusal path (`kernel_channel_gv100.c:217`) fires on every refused birth, so
attempt-and-fall-back is not a clean probe; use the query.

**Detection.** §3.1 query bit (⊘ built: `NV_ESC_KF_QUERY` bit 0); if absent, the relay as today.

**Owner ruling to obtain first.** ⊘ *Answered by D7 (correction 1).* `OWNER_RULINGS.md` §AB.2 and the exposure review name this fix as "the
only future exception" to "nothing host-owned in a Passthrough space", and say it must stay **absent
until ruled on**. The text does not say what mapping the exception would add (an address-size fix maps
nothing into the GPU VA space). Ask the owner what the exception covers before writing any kayfabe
code that depends on H5.

**Security.** No new privilege: it narrows an address. A Passthrough twin adopting the guest's USERD
already means the GPU reads `GP_PUT` from guest memory, as for any Passthrough channel; the relay's
bound disappears with the relay, and the engine's own `GP_PUT` handling is the bare-metal one.
Option (b) changes where every DMA of that GPU lands in IOVA space; review for any consumer that
assumed more than 40 bits.

**Performance.** Removes the relay worker hop for Windows user-channel doorbells (doorbell → worker →
copy → ring) **[reasoned]**; no latency figure for the relay exists **[measured: none]**.

**Validation.** Windows only: the D3D lane with `KF3_USERD_RELAY_OFF=1` on a patched host vs the relay
on a stock host, same binary; Linux gates unchanged. ⊘ The hardware test plan is `V3_H5_USERD_DMA_PATCH.md` §6.
**Effort** S-M (the diff is small; ⊘ the rebuild tier is not needed, correction 4). **Dependencies** ⊘ the owner ruling is given (D7);
the kayfabe follow-up and the hardware proof.

### 4.6 H6 — exact partial unmap of `NV01_MEMORY_VIRTUAL` mappings

**Fixes.** A range unmap that splits a mapping placed through the space's `NV01_MEMORY_VIRTUAL`
range frees the mapping's **whole** VA block in host RM, so the remnants lose their PTEs
(`dmaFreeMapping_GM107` else-branch `vaspaceFree(pVAS, vAddr)`, then a block lookup by containment;
`gvaspaceFree_IMPL`). **[measured, runs 242/243]**: host Xid 31 `FAULT_PTE` at `0x4034000` and
`gpu_vaspace.c:1639` assertions at exit; **[read]** source in `V3_BATCHED_MAP.md` §8.1. Inside an
`NV50_MEMORY_VIRTUAL` (reserving) only the unmapped PTEs are invalidated, so there the partial unmap
is exact; Windows process VAs lie in the unreserved `[1 MiB, 4.5 GiB)`, so they take the bad path. A
second, smaller cost: `serverInterUnmapInternal` walks the mapper's whole mapping list per unmap
(`rs_server.c:2419`), **[measured, nested]** 84-123 us per unmap at ~12 k mappings.

**Stock fallback (correct without it)**, as built and model-tested in `V3_BATCHED_MAP.md` §8.3/§8.7
(hardware verdict pending, §8.6): one host mapping per guest leaf outside any reservation; batches
only inside a VA-reserving `hDma`; owned-span range unmaps only; net diff; no remap, ever.
*A decision on big-leaf placement is being implemented in parallel with this page; it is referred
to here as `V3_BATCHED_MAP.md` §8.8, which does not exist at this revision (§8.7.3 holds the open
owner decision it answers):* for a big leaf (64 KiB / 2 MiB) in the `NV01` range the stock rule is **micro
reservation if `kf-micro-reserve-probe reserve` passes, else 4 KiB grain, and never re-make** (§8.7.3
today declares a re-make; that is the case the decision removes). The cost of the stock rule (**[reasoned]**, §8.7.3): 16 map calls per 64 KiB leaf, 512 per 2 MiB, and 4 KiB host PTEs (smaller
GPU TLB reach). Reconcile this entry with §8.8 when it lands.

**What the kernel change is.** `nvidia.ko` RM core, open-source build (the full tree is needed; see
§9.1). In 595.84: `dmaFreeMapping_GM107` (`virt_mem_allocator_gm107.c:1531`) and `virtmemUnmapFrom_IMPL`
(`virtual_mem.c:1636`). For a partial unmap with `bReserveVaOnAlloc == NV_FALSE`: do what the
`NV50` branch does (invalidate only `[vaLo, vaHi]` through `dmaUpdateVASpace`, valid = false) and keep
the VA heap block alive until its last remnant is gone (or split the block), instead of freeing it.
Second, optional: index the mapper's mapping list so an unmap is not O(M) (`rs_server.c:2419`). Add
an **opt-in flag** in the unmap params so CUDA and every other client keep the stock semantics
**[proposed]**.

**Detection.** The `NV_ESC_KF_QUERY` bit (§3.1). Not behavioural: the stock behaviour of the
`kf-micro-reserve-probe nv01-control` is a fault on the probe's own channel.

**Security.** No new reach: the unmap stays inside the caller's own VA space. The risk is a bug in the
VA heap bookkeeping, which corrupts the VA space of that one RM client (its faults, its asserts);
the opt-in flag keeps every other client on the stock code.

**Performance.** *Reasoned:* removes the call amplification and the 4 KiB-grain TLB penalty on the
Windows low range; saves nothing on the Linux/CUDA paths, which already unmap exactly inside big
reservations (gate 4). Reference measurements, nested: per-run map 19-22 us; a 12 288-run space maps
in 3 batched calls and unmaps with one range (3-8 ms vs ~1.26 s, `V3_BATCHED_MAP.md` §7.1). *Value is
conditional:* if `reserve` passes on a stock host (§8.0) the stock rule already gives exact partial
unmaps for batches, and H6 shrinks to "fewer RM calls for big leaves".

**Validation.** GPU-free: give `kf_mem::sim` an RM model switch (stock / exact) and run the property
tests under both, including the invariant "unchanged VAs never transiently unmapped" after every host
call. Hardware, on a vast KVM-image box (the stock control faults on purpose): `kf-micro-reserve-probe
nv01-control` is the A/B (stock: one Xid 31 on the probe's channel and the `:1639` assert; patched:
the remnant reads); then a Windows boot, expecting `remade_unchanged_pages` 0 and `huge_refused` 0,
no Xid 31; `v3_gates.sh` (gate 4), fast suite, CUDA no-PM lane. **Effort** M. **Dependencies** the
rebuild tier; the `reserve` verdict and §8.8; no other entry.

### 4.7 H7 — a scatter-map verb (no stitched views)

**Fixes.** Mapping N scattered guest runs as one GPU mapping needs, on the stock path, a stitched user
VA (N `MAP_FIXED` mmaps of the guest memfd), one OS descriptor over it, and a reaper that `munmap`s the
view. **[measured, nested]:** stitch 6-14 us per piece, descriptor 7-18 ms per 4 096 pages (RM pins),
`munmap` 20-24 us per VMA, sharing `mmap_lock` with the stitch, so the batched map side is slower than
per-run (352 vs 265 ms for 12 288 runs) while the unmap side is 100x faster (`V3_BATCHED_MAP.md` §7).
**[measured: not yet]** on a non-nested host, where these costs are "expected to be several times
smaller" **[inferred]**.

**Stock fallback.** What is built.

**Kernel change.** `nvidia.ko` RM core. `NV0080_CTRL_CMD_DMA_FILL_PTE_MEM` is declared
(`ctrl0080dma.h:205`) with **no dispatch entry** in 580's `g_device_nvoc.c`, and the other page-array
verbs are privileged or kernel-only (`V3_BATCHED_MAP.md` §2). A patch would take an array of
`(fd, offset, length)` pieces of a memfd the caller owns and pin them in the kernel with the existing
pin path (`os_lock_user_pages`, `os-mlock.c`), which is the stitch moved into the kernel. It must not
take physical addresses from userspace. **Not owner-decided; derived by this page from §2.**

**Security.** No physical addresses from the caller; bounded piece count; ownership by file descriptor.
**Performance.** Unmeasured; removes the stitch, the reaper and the `mmap_lock` contention
**[reasoned]**. **Validation.** Do not start before the non-nested stitch cost is measured on the
trusted host (`bm3_diag` per batch); if the stitch is under ~10 % of a Windows boot's map time, drop
this entry. **Effort** M. **Dependencies** the rebuild tier; lowest priority.

## 5. Shared prerequisites and the dependency graph

- **H0 (probe, packaging, manifest, tier line)** is a prerequisite of every entry. It is kayfabe-side plus
  packaging and needs no kernel change of its own.
- **The `nvidia.ko` rebuild tier** (a source build of the matching open-gpu-kernel-modules tag, a
  stop-VMs/stop-desktop/replace/reload/rollback procedure, a vast KVM-image box to prove it on) is a
  prerequisite of H6, H7 and H1 variant A. ⊘ *H5 (2026-10-10): not a prerequisite of the **build**, since the patch applies to the packaged
  DKMS tree; the stack reload and rollback procedure still is, and is written in `V3_H5_USERD_DMA_PATCH.md` §6.* H1 variant B, H2 and H3 do not need it.
- **Owner decisions** gate: H5 (⊘ the scope of the §AB.2 exception: given, D7), H1 (variant, vCPU-store judgement),
  H6 (opt-in flag; and the §8.8 big-leaf decision it must agree with).
- **Hardware verdicts** gate value, not correctness: the `reserve` probe (H6), the bare-metal
  wake-to-ring figure (H1), the non-nested stitch cost (H7).

```text
H0 ──┬─ H1-B (module, no nvidia.ko rebuild)
     ├─ H3 (nvidia-uvm patch) ── guest fault plane (kayfabe side) ── H4
     ├─ H2 (mdev module) ── four prove-first answers
     └─ rebuild tier ──┬─ H6 (after reserve verdict + §8.8)
                       ├─ H5 (ruled, D7; needs the reload procedure, not the source build)
                       ├─ H7 (after non-nested stitch cost)
                       └─ H1-A (hardening of H1-B)
```

## 6. Neighbouring items that touch the host, and the reason each is not a patch

Recorded so that none is re-opened as a candidate without new evidence.

| item | what it is | why it is not a host patch |
|---|---|---|
| **Carve-out refusal** (`kf_mem::apply::carve_reached`; straddle fix `2e10a0c7`; `V3_WINDOW_EXPOSURE_REVIEW.md` row 6; `OWNER_RULINGS.md` §AB.2) | kayfabe refuses a guest leaf that names its firmware carve-out; a run that straddles the carve-out base is clipped at the base | kayfabe's own walker/apply logic over its own store. No host code is involved; the stock path is the only path |
| **MMU invalidate refused** (`OWNER_RULINGS.md` §AA; hang root cause, memory note of 2026-10-09) | an uncleared `MMU_INVALIDATE` held the guest in a poll with dxgkrnl locks held | fixed in kayfabe (invalidate completes over absence). Not host |
| **Host RM's own buffers in a twin's VA space** (exposure review rows 9-10) | GR context, bundle CB, page pool and GSP's split-VAS range `[4 GiB, 4.5 GiB)` sit in every RM VA space | bare-metal parity: NVIDIA places the same buffers in every native process; the privileged ones fault a USER channel. Row 10 is inherent to every RM address space. A patch to move them has no demonstrated need |
| **arm64 `VM_ALLOW_ANY_UNCACHED` one-flag `nvidia.ko` mmap patch** | proposed 2026-10-03 for arm64 BAR1 views | **withdrawn** by the owner-ruling correction: `nvidia.ko` itself maps the BAR as Device-nGnRE on arm64, so there is no regression against bare metal; "not planned" (`OWNER_RULINGS.md` §I) |
| **Host IOMMU identity domain** | an admin setting, an alternative to H5 | no patch; ruled out on the dev host (live desktop GPU) |
| **The VFIO reference tracer / `x-gsp-observer`** | a diagnostic in the QEMU tree (`vfio_region_read` trace events, `tools/vfio-gsp-observer`) | QEMU-side, bench-only (`OWNER_RULINGS.md` §Y), not a host kernel patch. The memory note that lists a "patched vfio-pci tracer" means this |
| **vfio-user server-initiated mmap changes** | `V3_VFIO_USER_FRONTEND.md` §2 option 2 | a vfio-user client/QEMU change, not a host kernel one |
| **Guest-side stages** (doorbell helper module, patched guest driver: doorbell by token, dynamic memory, fault-free managed memory) | `V3_COOPERATIVE_TIERS.md` stages 1-2 | they patch the **guest**; this page is about the host. Windows has no equivalent for the doorbell helper (`V3_WINDOWS_DOORBELL_RESEARCH.md`), which is why H1 matters for Windows |
| **SR-IOV / vGPU / MIG** | datacenter parts | a different product model (`V3_UVM_DEMAND_PAGING.md` §4.5; `OWNER_RULINGS.md` §H) |

## 7. Proposed implementation order

**Recommended first patch: H1, the in-kernel doorbell, as variant B (a new kayfabe module), with H0
built alongside it.** Reasons, in order of weight:

1. **It is the smallest blast radius that still exercises the whole tier.** A new module that
   neither rebuilds nor reloads `nvidia.ko` can be loaded and unloaded on the trusted host without
   stopping the owner's desktop. H6, H5 and H7 each need a full `nvidia.ko` build and stack reload, which
   today is a vast-box-only operation (§3.3).
2. **It forces H0 to exist for real** (probe, DKMS, manifest, fallback gating, tier in the status
   line, unload safety), which every later entry reuses. The first patch pays that cost once.
3. **It is the owner's stated performance lever for stock guest OSes**, including Windows, which has
   no guest-side doorbell alternative.
4. **The A/B harness exists** (`dbfast_lane.sh`, `dbfast_llm.sh`, the exit hook, `llm_parity.sh`), and
   the stock path it replaces is built and default-off.
5. It has a **cheap falsifier up front** (§4.1, step 0), so the cost of being wrong is one
   measurement, not a module.

**Caveats the owner should weigh.** The performance case is a hypothesis: the non-nested vCPU cost
is already 1.8 us with ioeventfd, and no bare-metal wake-to-ring number exists. If step 0 says the
wake-up is small, the right first patch becomes H3's port (below), which has a proven kernel half.
The wake function runs on the vCPU thread (§4.1, security) and needs an explicit judgement against
"nothing blocks on a vCPU".

**Then, in this order:**

| # | step | why here |
|---|---|---|
| 2 | **H3 port to 595.91.07**: per-tag CI dry-run (applies to 595.84 today), re-run the host-only proof on a vast KVM-image box | independent of 1; the patch exists; cheap early warning against drift. The *feature* waits on the guest fault plane, which is kayfabe-side work |
| 3 | **The `nvidia.ko` rebuild tier** (H0 extension: source build from the matching GitHub tag, replace + reload procedure, §9.5), then **H6** | the rebuild tier is a prerequisite for H5-H7. Start H6 only after the `reserve` hardware verdict and the §8.8 decision: if the stock rule is good enough, H6's value shrinks (§4.6) |
| 4 | **H5**, after the owner ruling on the §AB.2 exception | Windows-only; S-M; shares the rebuild tier |
| 5 | **H2 prove-first prototypes** may start any time in parallel (they are kernel-only experiments); the module ships after | L, with four open questions; the largest adoption lever but the least certain |
| 6 | **H7** (only if the non-nested stitch cost justifies it), **H4** (post-release by owner ruling) | conditional |

## 8. Stock-path guarantees

Each patched feature may change **speed or capability**, and must never change what the stock path
guarantees. These behaviours must keep passing with the patch absent, with it installed and probed
out, and with it failing at use time. Tests run the stock lane first; CI's stock lane never loads a
patch.

| entry | what the stock fallback must keep |
|---|---|
| H1 doorbell | no notification is ever lost, and none is delayed on purpose (no timers, no batching: `OWNER_RULINGS.md` §D, 2026-09-28 refinement). Ordering of one channel's rings is preserved. Translated/Emulated tokens always reach kayfabe's translation path. A refused registration leaves that token on the trapped path, named and counted, and a test exhausts the limit. Unloading or losing the module mid-run moves bound tokens back to the drainer without a lost ring |
| H2 mdev | n/a to stock correctness: stock is the overlay. The shim must not change the overlay's behaviour, and the overlay must not need the module |
| H3 UVM | on a stock host, no opt-in is attempted (probe says Absent). Prefetch/advise work; a GPU fault on a managed twin fails loudly and corrupts nothing (fix the silent `conjugateGradientUM` case); read-only duplicates stay read-only (loud 719 + Xid 31). If the patched module is present but disabled, behaviour is the stock one byte for byte |
| H5 USERD | the relay's bounds stay: `GP_PUT >= gpFifoEntries` is refused, counted, logged by name (the twin keeps its last good value); `GP_GET` write-back is host-derived and bounded to `[0, entries)`, never reads `GP_PUT`; the relay's lock is taken blocking, never `try_lock` (run 77). The relay maps nothing into any GPU VA (§AB.2). ⊘ The scope is ruled (D7); the kayfabe behaviour that adopts the guest USERD stays off until the hardware test passes and the owner confirms (`V3_H5_USERD_DMA_PATCH.md` §7) |
| H6 unmap | **the walker protocol and the owner invariant of 2026-10-10 are unchanged**: unchanged VAs stay accessible at every instant of a refresh; no unmap-then-remap; no overlay (`VA_ALREADY_MAPPED`); commit-on-ack; a refused unmap holds the invalidate as before (§AA). On a stock RM the decision in `V3_BATCHED_MAP.md` §8.7/§8.8 is the rule: **micro reservation if it passes, else 4 KiB grain, never re-make**; the `remade_unchanged_pages` counter stays 0 on the stock path once §8.8 lands. The ownership ledger (`OwnMaps`) stays authoritative with or without the patch. `kf_mem::sim` runs under both RM models |
| H7 scatter | the stitched, batched path (§3 of `V3_BATCHED_MAP.md`) with its fallback to per-run on `ReaperBacklog`/refusal |
| all | no result of a patched run may change a guest-visible value; a patched feature that cannot be used degrades per object (§3.1); no build-time switch selects the tier; the gates (`v3_gates.sh` 9/9, fast suite 30/30) are the stock bar and pass with no patch installed |

## 9. Distribution

### 9.1 Package shape [proposed]

Two source packages, so that the part with no NVIDIA dependency does not carry the part that has.

1. **`kayfabe-host-dkms`**: `kayfabe_host.ko` (H1 variant B and, later, `kayfabe_mdev.ko` for H2). It
   depends on the running kernel (headers, `CONFIG_VFIO_MDEV` for H2), not on the NVIDIA driver
   version. GPL-2.0, as the kernel requires for the GPL-only mdev/vfio symbols; the repository is
   `Apache-2.0 OR GPL-2.0-or-later` (`OWNER_RULINGS.md` §G).
2. **`kayfabe-nvidia-patches`**: a per-driver-tag **patch set** (`patches/<tag>/`) and a `dkms.conf`
   whose `PRE_BUILD` copies the NVIDIA DKMS source tree, applies the patches, and builds the replaced
   modules **together** (`nvidia` and `nvidia-uvm` from one tree, so symbol CRCs agree; this is what
   `tools/uvm_efs/box/build_efs.sh` does). H3 patches only `nvidia-uvm`. The RM-core entries (H5, H6,
   H7, H1-A) need the full `src/nvidia` source.

**The flavour constraint [inferred, verify on the host's `/usr/src/nvidia-*`]:** NVIDIA's packaged
open-module DKMS tree carries the kernel-interface layer, with the RM core as a prebuilt object
(`nv-kernel.o_binary`). If so, RM-core patches cannot be applied to the packaged tree: they need a
build of the matching `NVIDIA/open-gpu-kernel-modules` tag, installed in place of the packaged
modules. Closed-flavour hosts could then take H1-B, H2 and H3 (UVM source ships in both) but not
H5-H7. Check before promising H5-H7 to anyone.

⊘ **Checked for H5 (2026-10-10), [measured]:** the host's packaged tree (`/usr/src/nvidia-595.91.07`, package `nvidia-dkms-595-open`) has
`nvidia/nv-kernel.o_binary` (17.6 MB) and is otherwise identical to the tag's `kernel-open/` (`diff -rq`). The inference above is
right for RM-core patches (H6, H7, H1-A) and **does not bite H5**, which patches only `kernel-open/nvidia/nv.c` and two new headers;
the patch applies to the packaged tree and builds against the prebuilt RM core. Closed-flavour hosts are not checked.

### 9.2 Version pinning and other driver versions

- **The verified pin is the trusted host's driver, 595.91.07 (open module, kernel 7.0.0-34)**
  **[measured: `V3_LINUX_PERF_BASELINE_6692e621.md`]**; the older bench pin is 580.159.04, where H3 was
  proven. A patched-tier claim names its tag.
- A **manifest** lists, per tag, one of `verified` (the entry's hardware proof ran at that tag),
  `applies` (dry-run only), `unknown`. Today: H3 `verified` at 580.159.04, `applies` at 595.84 (note
  that is not 595.91.07: the dry-run above used the 595.84 tree, the nearest available); ⊘ H5 `applies` at 595.91.07 (dry run on the
  packaged tree and on the tag, **plus a build-only compile against the host's kernel 7.0.0-34**) and at 595.84 (dry run, offsets),
  not `verified` (never loaded); everything else unbuilt.
- **Policy for any other driver version** **[proposed, owner to decide]:** the stock tier works with
  every driver version kayfabe accepts, with no patch. The patched tier is built only for manifest
  tags. On any other tag the DKMS `PRE_BUILD` runs `patch --dry-run`; a failure skips the patched build,
  logs the reason, and leaves the system stock (never a half-patched tree). A tag that applies but is
  not `verified` builds only when the admin sets an explicit opt-in. CI runs the dry-run against every
  accepted host tag (`V3_SWEEP_AND_INSTALL.md` task I8).
- **Kayfabe never reads the manifest or the version to decide a feature at runtime** (§3.1). The
  manifest controls what gets built and installed, nothing else.
- Cost grows with each host release for the entries that touch fault handling and RM VA code (H3, H6);
  keep each patch small, in its own file, three hook sites or fewer (`V3_UVM_DEMAND_PAGING.md` §4.4).

### 9.3 Install check

`kayfabe-preflight` (per `V3_SWEEP_AND_INSTALL.md`) prints the tier it can reach: kernel headers,
whether a module can load (Secure Boot state, lockdown), which components are loaded, each probe's
result. An unreachable tier is information, not an error.

### 9.4 Signing and Secure Boot (owner ruling, 2026-10-10)

- **Linux:** source only, built on the user's machine by DKMS and signed there with the machine's own
  key (the standard DKMS/MOK flow). Secure Boot is no concern of ours; if a module does not load
  (unsigned under Secure Boot, lockdown), the probe says Absent and kayfabe runs stock. No prebuilt
  signed binary is shipped for the host tier.
- **Windows:** the host tier has **no Windows component**. Driver signing is the hard problem for any
  guest-side Windows driver (for example a Windows doorbell helper, which is not possible anyway:
  WDDM rings from the kernel driver). The point of H1, H2 and H3 is that a stock Windows guest gets
  the benefit with no guest driver. The boot-ROM signing in `OWNER_RULINGS.md` §K is unrelated.

### 9.5 Unload, reload, rollback

- **`kayfabe_host`** (H1, H2): takes a module reference per open fd and per registration, so `rmmod`
  fails with `EBUSY` while any VM uses it; it is independent of `nvidia`, so it never needs the stack
  reloaded. Closing the fd (a SIGKILLed VMM included) tears down every registration synchronously
  before the page it writes can go away (the revocation design of §4.1). Probes run at each VMM start,
  so a module loaded or unloaded between VMs is picked up with no kayfabe restart.
- **Patched `nvidia-uvm`** (H3): unloads once every UVM file is closed (host CUDA apps stopped). The
  b3 teardown was measured: a SIGKILLed process with a parked fault cancels it in hardware, and
  `nvidia-smi` stayed healthy (`traces/v3_uvm_b3/`, `dmesg_efs_shutdown.txt`).
- **Patched `nvidia`** (H5-H7): replacing it reloads the whole stack (`nvidia_drm`, `nvidia_modeset`,
  `nvidia_uvm`, `nvidia`), which stops the display manager on a desktop GPU. The trusted host already
  has such a procedure for the IOMMU-domain change (`/var/lib/kf-windows-20261005/iommu_type.sh`,
  `traces/windows_code43_walls_20261007/README.md`); a replacement procedure must stop VMs first,
  restore the previous module set on failure, and re-run the probes afterwards.
- **Kernel update:** DKMS rebuilds on the next boot. A failed build leaves the stock modules and the
  system stock. **Rollback:** `dkms remove` plus `modprobe -r`; the next VM start probes Absent and
  runs stock, with no kayfabe change.
- **GPU reset or unbind** with H1 bound: the binding is revoked by the unmap, never left writing to a
  stale address (§4.1). This is a validation item, not an assumption.

### 9.6 Directory layout and packaging, as built for H5 (2026-10-10)

`tools/host_patches/<entry>/` follows `tools/uvm_efs/`: `patch/<name>_<tag>.patch` (applied with `-p1` from `kernel-open/` or a packaged
tree root), `include/` (copies of files the patch adds, checked against it), `box/` (scripts that act on a box: `build_h5.sh` is
**build-only**, refuses live output paths), `tests/` (GPU-free tests plus `tests/hw/` helpers for the later hardware step). One patch file per
driver tag: the same file applies to 595.84 with offsets, so a second tag gets a second file only when a dry run fails.

Packaging [proposed, not built]: the second source package of §9.1 (`kayfabe-nvidia-patches`) holds `patches/<tag>/*.patch` and a
`dkms.conf` that builds the same four modules as the packaged one from a patched copy of `/usr/src/nvidia-<tag>`; `PRE_BUILD` runs
`patch --dry-run` first (a failure leaves the stock modules, §9.2). Two DKMS registrations for the same module names and version
must not coexist: install the patched one with the stock registration removed (`dkms remove nvidia/<tag> --all`), or use the `insmod`
method of `V3_H5_USERD_DMA_PATCH.md` §6.3, which persists nothing. The version string is unchanged, so userspace libraries keep working.
Secure Boot: sign with the machine's key (§9.4).

## 10. Questions for the owner

1. **H1 variant:** B (separate module, no `nvidia.ko` rebuild, caller-supplied page with MMU-notifier
   revocation) first, with A kept as the hardening; or A only? And is a posted MMIO store on the
   vCPU thread acceptable under "nothing blocks on a vCPU", or must it go through a work item?
2. **H5 scope:** ⊘ *answered by `DELEGATED_DECISIONS_20261010.md` D7 (address-size constraint only; option (a) preferred, (b) only if (a)
   cannot be located; never relax the check). (a) was located and built. Still open for the owner: F2 (a descriptor per USERD page range)
   vs F1 (the whole guest-RAM object through the window), `V3_H5_USERD_DMA_PATCH.md` §7.* Original: what does the §AB.2 "sysmem-USERD
   address-size fix" exception cover, and is a 40-bit device-mask cap (option b) acceptable, or only a per-allocation constraint (option a)?
3. **Install policy** for tags that apply but are not hardware-verified (§9.2).
4. **RM-core patches** (H5-H7) are proven on vast KVM-image boxes only until a second GPU host exists
   that can stand a stack reload. Acceptable?
5. **H6 opt-in flag:** per-unmap flag (proposed) or always-exact semantics?

## 11. Sources

`docs/design/V3_COOPERATIVE_TIERS.md` (stages, §3.2, §5); `V3_H5_USERD_DMA_PATCH.md` and `tools/host_patches/h5_userd_dma/` (H5); `V3_UVM_DEMAND_PAGING.md` (§0, §4.4);
`V3_UVM_B3_IMPLEMENTATION.md` (§0); `tools/uvm_efs/` (patch, ioctl header, `box/build_efs.sh`);
`V3_VFIO_USER_FRONTEND.md` (§1, §2); `V3_BATCHED_MAP.md` (§1, §2, §7, §8.0-§8.7); `V3_USERD_RELAY.md`;
`V3_DOORBELL_IOEVENTFD.md`; `V3_BAR1_DOORBELL.md`; `V3_GUEST_DOORBELL_MODULE.md`;
`V3_WINDOWS_DOORBELL_RESEARCH.md`; `V3_LINUX_PERF_BASELINE_6692e621.md` (§4); `V3_WINDOW_EXPOSURE_REVIEW.md`;
`V3_SWEEP_AND_INSTALL.md` (§2.7, I8); `V3_DRIVER_MATRIX.md` (§8.2 ruling 3); `OWNER_RULINGS.md` (§D, §E, §G,
§I, §J, §K, §Y, §AA, §AB); `traces/windows_code43_walls_20261007/README.md` (run 61, runs 242/243);
`STATUS_AND_HANDOFF.md`. Memory notes of the owner sessions: host patch tier (2026-10-10),
passthrough-only-PT-leaves, VA-manager unchanged-VA invariant, pending owner items (2026-10-09).
ogkm trees read: 595.84 (`/workspace/ogkm-tars/open-gpu-kernel-modules-595.84`) for every function and
line cited as 595.84; 580.159.04 for the citations that say 580. The trusted host runs 595.91.07, so
re-locate every line before writing a patch.
