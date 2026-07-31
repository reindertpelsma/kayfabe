# The compatibility matrix — six axes, and the cells nobody had looked at

> **Read this with `scripts/compat_matrix.py`.** Everything in §2 is derived by that script
> from the tree itself, so the table is wrong only when the tree has moved. A hand-written
> matrix rots green; this one is regenerated. §3–§6 are the reading, which is human work.
>
> ⊘ **No hardware was run for this note.** Every status below is a source read of
> `/workspace/nvkvm-rs` at `ee3c808` plus the vendored driver trees, and is labelled
> `[read]`. Where a claim rests on somebody else's bench run, the run is named and the
> claim is labelled `[their run]`. Nothing here says what a mismatched configuration
> *does* — only what this tree would send it.

## 0. Why six axes and not four

`four_axes_of_variation.md` names four: GPU architecture, guest driver version, host driver
version, guest OS. Two more vary in practice and neither has ever been audited:

- **guest kernel version** — the bench pins `6.8.0-117`/`-136`, and that pin has a reason
  that turns out not to be about the kernel at all (§2.5);
- **multi-GPU** — `MG-1 … MG-7` are built in the core, and the axis stops dead at the core
  boundary (§2.6).

Adding them is not bookkeeping. Four of the six most interesting findings in this note are
in the *cross* terms between the new two and the old four (§3).

## 1. ★★★ The frame that changes every answer: LINKED vs NOT LINKED

The single most useful thing the deriver computes is not an axis. It is the **shipping
closure** — the set of crates a real build output links, walked from the two artifacts that
exist (`kayfabe-qemu-raw`'s `staticlib`, which QEMU links, and `kayfabe-isolate-host`'s
binaries), through `[dependencies]` only:

```
guest edge (QEMU staticlib)  : abi arch completion device gsp isolate linux-raw mmu
                               qemu-raw trace util vmm vmm-qemu
host edge  (isolate binaries): abi arch isolate isolate-host linux-raw util vmm
NEITHER                      : chips core crec fwd mocks rmrpc rt shell vmm-kvm
```

`[read]`, and it reframes three axes at once:

- **`kayfabe-rmrpc` is not linked.** It is the only crate that threads `GuestOs`, so the
  fourth axis' seam is, today, reachable only from the test crate.
- **`kayfabe-chips` is not linked.** `Ad10xGspModel` and `Gh100GspModel` are fixtures the
  test crate builds; no shipping artifact contains them.
- **`kayfabe-core` (and `-fwd`, `-rt`, `-shell`) are not linked.** Every `MG-*` decision
  lives there. Multi-GPU is *built* and *not wired*.

⊘ This is a statement about **linkage**, not about correctness or deadness. A crate on the
last line may be finished, well-tested and about to be wired. What it cannot be is
*reachable by a guest*. Read every "built" below through this line.

## 2. The six axes, per cell

### 2.1 Axis A — guest driver version. **BUILT (table), PINNED (integration)**

| what | status | citation |
|---|---|---|
| version key | full `(major, minor, patch)`, no major-only collapse | `crates/kayfabe-abi/src/lib.rs` `DriverVersion` |
| table registry | **8 rows**, 550.54.04 → 610.43.02, ascending | `crates/kayfabe-abi/src/versions.rs:309` `TABLES` |
| miss behaviour | `NoTableForVersion`, no nearest-neighbour below the floor | `versions.rs:509-517` |
| capability surface | shared base + per-boundary blocks, 8 boundaries, `ALL_BOUNDARIES` checked against `TABLES` | `crates/kayfabe-abi/src/capability.rs:1478`, `:1580-1610` |
| **what the shim can select** | **exactly one**, a compile-time `const` | `crates/kayfabe-qemu-raw/src/shim.rs:602` `GUEST_DRIVER = BENCH_DRIVER` = 580.159.04 |

★★ **The derived cell nobody had stated: only 2 of the 8 rows can answer the FIRST RPC.**

```
550.54.04  550.90.07  555.42.02  560.28.03  570.86.15  575.51.02   →  vgx: None
580.65.06  610.43.02                                               →  vgx: Some(..)
```

`SET_GUEST_SYSTEM_INFO` (fn 1) is the first thing RM sends once its GSP is up, and the
policy answers it out of `DriverAbiTable::vgx_version()`; `None` short-circuits to
`NV_ERR_NOT_SUPPORTED` (`crates/kayfabe-device/src/guestsysinfo.rs:77-82,110-118`). So the
six older rows carry wire layouts and a capability set for a guest that **cannot get past
message one**. That is not a defect — a refusal is the right answer where we have no
citation — but it means "the guest-driver axis supports 550→610" and "a 570 guest boots"
are very different statements, and only the deriver tells them apart.

★ **And the match is EXACT, not an interval.** `agreed_version` refuses on
`VersionMismatch` whenever the guest's declared pair differs from ours
(`guestsysinfo.rs:77-87`). The pairs are `0x2B/0x13` at 580 and `0x2E/0x0D` at 610
(`ogkm-580:`/`ogkm-610: src/nvidia/inc/kernel/vgpu/vgpu_version.h:33-34`, cross-checked
against 610's own `VGPU_19_0` row at `:41-42`). ⇒ **a 590 or 595 guest resolves to the
580.65.06 row by `table_for`'s newest-≤ rule, is answered `0x2B/0x13`, declares something
else, and is refused at fn 1** — even though `four_axes_of_variation.md` §3 correctly notes
that 580/590/595 share the 48-byte GSP element form. The *layout* axis and the *handshake*
axis do not agree about which guests are supported, and the handshake is the binding one.

⊘ **The protocol offers a graceful path and we do not take it.** `rpc.c:8765-8801` defines
down-negotiation: a GSP that speaks a different version replies non-`NV_OK` **with its own
pair in the body**, and the guest retries at that pair or reports "host too old". Not built,
deliberately (`crates/kayfabe-abi/src/guestsysinfo.rs`, "⊘ The down-negotiation branch is
NOT built"). This is the single cheapest thing that would turn Axis A from one point into a
range.

### 2.2 Axis B — GPU architecture. **SEAM BUILT, ONE REACHABLE CHIP**

Three lists, and their disagreement is the finding:

| list | contents | linked? |
|---|---|---|
| `kayfabe_device::CHIPS` — the chips a PCI device id can resolve to | **`GA106` only** | LINKED |
| `kayfabe_abi::vbios::VBIOS_PROFILES` | `GA106`, **`AD106`** | LINKED |
| GSP models in `kayfabe-chips` | `ad10x`, `gh100` | **NOT LINKED** |
| GSP model in `kayfabe-device` | `ga10x` | LINKED |

`[read]` `crates/kayfabe-device/src/lib.rs:334`; `crates/kayfabe-abi/src/vbios.rs:451`;
`crates/kayfabe-chips/src/lib.rs`.

★ **`AD106` has a VBIOS profile and no `ChipProfile`.** `chip_for_device_id` can never
return it, so the row is unreachable — and the QEMU device *does* expose
`chip-device-id` as a settable property (`qemu/hw/misc/nvkvm/nvkvm.c:1474`), so an operator
can ask for `0x2803` today and will get `NoChipForDevice` rather than an Ada boot. The
refusal is correct; the asymmetry between the two tables is what nobody had noticed. The
chip-table rustdoc states the three-part rule ("a row here, plus the module its `gsp_model`
names, plus the `VBIOS_PROFILES` row its `pci_device_id` keys") — AD106 has one part of
three.

★★ The seam itself is real and that is `1292f80`'s established result — adding GH100
changed zero lines in the Turing path, `BootSequence` is per-generation and
`GspModel::boot_sequence()` is deliberately not defaulted. What §1 adds is that **the
crate holding both new generations is linked by nothing**, so "the seam exists" and "a
second generation is reachable" are separate facts and only the first is true.

### 2.3 Axis C — host driver version. **PINNED, LOUDLY** (established `2a49589`)

`[read]` `crates/kayfabe-abi/src/host_driver.rs:91,101,108` — `ENCODED_FOR_MAJOR = 580`,
`ENCODED_FOR_FLOOR = (65, 6)`, `CHANNEL_PARAMS_SHIFT_MAJOR = 610` ⇒ the encoders are pinned
to **`[580.65.06, 581.00.00)`**, checked per `RmConnection::open`, two refusal arms
(`Unparsable`, `NotEncodedFor`), no override. Nothing further to add except its cross terms
(§3.4).

### 2.4 Axis D — guest OS. **SEAM BUILT, NO CONFIGURATION DOOR, NOT ON THE LIVE PATH**

The seam is genuinely good and its own docs are the best in the tree
(`crates/kayfabe-abi/src/guest_os.rs`): OS is **configured, never detected** (it is a
`#define` in the guest driver's build and appears in no wire field); `Linux` carries
`ClientKindRule::KernelPidSentinel`; `Windows` carries `None`, which is a typed refusal at
every call site; a mistyped `--guest-os=` is `UnknownGuestOsName` rather than a fallback.

★★★ **Three cells nobody had checked, all derived:**

1. **`GuestOs::from_config_name` has no caller outside its own module and its own tests.**
   `[read]`, and the deriver checks it. So there is no flag, no QEMU property, no realize
   plumbing — the value an operator would set is unsettable. The only way a `GuestOs` is
   produced in a non-test build is `Default` = `Linux`. The rustdoc says as much ("the
   realize-time plumbing that will call it is not built yet"); the derived form is stronger:
   **the axis has a home and no front door.**
2. **The only crate that threads `GuestOs` is `kayfabe-rmrpc`, which is not linked** (§1).
   `GraphPolicy` is constructed nowhere outside `tests/`, and `translate(abi, guest_os,
   cmd)` is reached only from inside it. So the fourth-axis refusal cannot fire in a
   shipping build — not because it is wrong, but because the RM-object plane it guards is
   not yet on the guest's path. The live path today is `kayfabe-device`'s boot policies,
   and `GuestOs` appears in that crate **zero** times.
3. **`guest_os_axis_gate.rs` is a LEXICAL gate over 15 tokens, and it is honest about being
   one.** It scopes 14 crates, exempts 9 with a written reason each, and — the part worth
   copying elsewhere — proves it can fire, per token, against a synthetic violation
   (`the_gate_bites_every_token_it_claims_to_cover`), and forbids a token that contains its
   own escape word (`no_token_can_name_its_own_way_out`, which deleted
   `RMCFG_FEATURE_PLATFORM_UNIX` from the list because `UNIX` is itself a naming word).
   What it therefore covers is *vocabulary*: `mm_struct`, `fork`, `pid_t`, `KERNEL_PID`,
   `/proc/`, `/sys/`, `/dev/`, `ELF`, `PAGE_SHIFT`, `sysconf`, … What it cannot cover is a
   Linux assumption expressed in NVIDIA vocabulary or in silence. §3.2 is one such.

★ **Windows guest: nothing else in the tree is Windows-hostile that I could find** `[read]`.
The GSP register plane, the FWSEC/VBIOS extraction, the msgq, the boot FSM and the RPC
envelope are all driver-side and OS-independent, which is exactly the bet
`four_axes_of_variation.md` §1.1 makes. The one measured OS-conditional field in the whole
protocol is `NV0000_ALLOC_PARAMETERS.processID`, and it is the one that has a home.

### 2.5 Axis E — guest kernel version. **NO HOME, AND THE PIN WAS NEVER ABOUT THE KERNEL**

`[read]` — the deriver's negative sweep: `GuestKernelVersion`, `LINUX_VERSION_CODE`,
`KERNEL_VERSION(`, `vermagic`, `utsname`, `osrelease` have **zero** hits in tracked Rust.
There is no constant, no probe, no typed error, no log line. A guest-kernel mismatch is
**not detected and cannot be**, and that stands out precisely because every neighbouring
axis has a refusal (`NoTableForVersion`, `UnknownGuestOsName`, `NotEncodedFor`,
`NoChipForDevice`).

★★★ **The crux, and it inverts the received wisdom.** The C bench's `6.8.0-117` pin is a
**Mode-1 patched-module vermagic** artifact, not a property of the emulated device:

- the coupling is one line, `KDIR ?= /lib/modules/$(shell uname -r)/build` in the *Mode-1
  guest module's* Makefile (`/workspace/nvidia-gpu-passthrough/src/guest/Makefile:1`);
- the Mode-2 emulator has no kernel-version awareness at all — `LINUX_VERSION_CODE` /
  `KERNEL_VERSION(` do not appear in `nvkvm_gpu_emul.c` `[read]`;
- and the falsification: the two Mode-2 runs of record used **different** guest kernels
  with the **stock, unpatched** module — `6.8.0-117` for the C reference capture
  (`traces/README.md:15-17`) and `6.8.0-136` for the Rust run of record
  (`crates/kayfabe-qemu-raw/src/shim.rs:486-487`) `[their runs]`. Bring-up progressed the
  same on both.

⇒ **`[read]`, INFERRED: the emulated device is insensitive to the guest kernel release
across at least `6.8.0-117 ↔ -136`, and the `-117` pin does not transfer to the Rust port.**
Both are the same Ubuntu 6.8 ABI family; this says nothing about a 5.x or a 7.x guest.

★★ **A provenance defect found on the way.** `traces/README.md:16` asserts the capture's
guest was "kernel 6.8.0-117-generic", but the artifact's own provenance block does not
substantiate it: the field literally named `guest-kernel-pin:` holds a *driver installer
filename*, and the only kernel release in the block is the **host's**
(`bench-host: ubuntu Linux 6.8.0-59-generic`) — despite the recorder's stated rule R4 that
the block carries the guest kernel vermagic
(`/workspace/nvidia-gpu-passthrough/src/qemu/nvkvm_m2_rec.h:34`). And
`crates/kayfabe-crec/tests/decoder_matches_reference.rs:66-89` asserts the provenance block
contains chip, hermeticity, driver version and four md5s — **but not the kernel**, so the
gate cannot catch it. The oracle's guest kernel is therefore an **UNKNOWN**, not a 117.

**Two unnamed guest-kernel dependencies**, neither detected (see §3.2 for why they are
Axis D as much as Axis E).

### 2.6 Axis F — multi-GPU. **BUILT IN THE CORE, STOPS AT THE CORE BOUNDARY**

MG-1 … MG-7 are built and densely exercised: `GpuId` as a routable target; routing keys
`(GpuId, Pdb)` / `(GpuId, VChid)`; one isolate **and** one GPA arena per `(Proc, GpuId)`;
per-target `GpuTarget { gpa, delivery, arch_name }`. **12 files instantiate more than one
GPU**, and a 2-GPU world is the *default* harness in `l1_mean.rs`, `rt_shell.rs`,
`reactor.rs` and `reactor_os.rs` — the axis is exercised incidentally by most of the suite,
not only by `multi_gpu.rs`. `[read]`

Two cells the deriver and the audit turn up:

★ **`MG-6 HeterogeneousArch` is an unreachable, untested refusal.** `GpuTarget.arch_name` is
private and only ever minted from `self.arch.name()`; `Spine.arch` is private with no
setter, and the code says so in as many words ("within this composition
`t.arch_name != self.arch.name()` cannot hold", `crates/kayfabe-core/src/gpu.rs:864-873`).
No test constructs or asserts `GpuError::HeterogeneousArch`;
`homogeneous_arch_all_targets_share_the_device_arch` asserts the *invariant* and then checks
an unrelated `UnknownPdb` miss. Mark the cell **policy refusal, unexercised** — the exact
shape of `a gate never SEEN to fail is not evidence`.

★★ **Multi-GPU × interrupt delivery is a shaped gap nobody has written down.** The
completion *state* is per-target (MG-6's drain gate), but the wire that signals the guest is
not: `IrqSpec` has no target field (`crates/kayfabe-vmm/src/lib.rs:475-488`),
`COMPLETION_VECTOR = IrqSpec::Msix(0)` is a single const
(`crates/kayfabe-fwd/src/lib.rs:101`), and both raise sites take a `GpuId` for the batch and
then raise a target-free vector on one `&mut dyn Vmm` (`fwd/src/lib.rs:1566-1590`).
Symmetrically the inbound `Device::mmio_read/mmio_write` carries no target, though
`gr_multigpu_seam_audit.md:95` assumed the adapter would supply one. The **same class of
bug one layer up was found and fixed** — `CoreEventKind::CompletionRedeliver(GpuId)`, pinned
by `tests/tests/rt_shell.rs:938` — which is the best argument that this one is real. `[read]`

**MIG** is designed-only and deliberately so (`docs/design/multi_gpu_and_mig.md`): MIG is
datacenter silicon, a slice is not a `/dev/nvidiaX`, and the whole accommodation is that
`GpuId` is a *target* rather than a device node.

## 3. The cross terms — reachable, structurally excluded, or empty

★ The useful question is not "does arch work" but "is *this combination* even expressible".

### 3.1 What is REACHABLE today, at the guest edge

**One point.** Not a subspace:

| axis | reachable values | why not more |
|---|---|---|
| guest driver | `580.159.04` | `GUEST_DRIVER` is a build `const`; a second value is a source edit |
| arch / chip | `GA106` | `CHIPS` has one row; `AD106` has a VBIOS row and no chip row |
| guest OS | `Linux` (by `Default`) | `from_config_name` has no caller |
| guest kernel | **anything** | unmodelled; nothing constrains or observes it |
| host driver | `[580.65.06, 581.00.00)` | enforced, per connection, two refusal arms |
| multi-GPU | `N = 1` at the guest edge | nothing in the tree says how N `GpuId`s become N PCI functions a guest enumerates; every adapter crate is `GpuId`-free |

⇒ **the reachable cross-product is `1 × 1 × 1 × (unconstrained) × (pinned) × 1`.** Every
"supported" claim on any other axis is a claim about a table, not about a configuration a
guest can be booted in. That is the honest headline of this whole audit.

### 3.2 ★★★ guest OS × guest kernel — the two unnamed Linux assumptions

Both would be invisible to `guest_os_axis_gate.rs`, because neither is spelled with any of
its tokens. Both are as much Axis D as Axis E: a Windows guest makes the same choices for
different reasons.

1. **MSI-X only, with no fallback and no detection.** The device models message-signalled
   interrupts and refuses legacy INTx by name
   (`crates/kayfabe-vmm-qemu/src/lib.rs:190-191`, raised at `:2088`); QEMU registers the
   MSI-X table/PBA at BAR 3 and `msix_vector_use`s every vector at realize
   (`qemu/hw/misc/nvkvm/nvkvm.c:1322-1340`). ★ **The refusal fires when *we* try to raise
   INTx — not when the guest never enables MSI-X.** A guest booted `pci=nomsi`, or a guest
   OS whose driver falls back to line interrupts, gets a device with no interrupt path and
   **no refusal at all**. `[read]`
2. **Guest-physical addresses are consumed raw; nothing translates.** `iommu` / `swiotlb` /
   `vIOMMU` appear **zero** times in code across `*.rs`, `*.c`, `*.h` `[read]`. Every
   address the guest driver hands the device — GPFIFO entries, GSP mailbox pointers — goes
   straight to `Vmm::gpa_read` (`crates/kayfabe-rt/src/device.rs:1236-1240` via
   `crates/kayfabe-fwd/src/lib.rs:2329-2335`). A guest behind a vIOMMU, or bouncing through
   SWIOTLB, supplies bus addresses that are not GPAs. The failure surfaces as
   `BadGpa`/`NonRamGpa` — a refusal, but one attributed to the wrong cause, which is the
   expensive kind.

The one guest-kernel behaviour that **is** anticipated: BAR reassignment. `nvkvm_config_write`
intercepts writes overlapping `PCI_BASE_ADDRESS_0..5` and refuses a base-address move, with
`nvkvm_after_bar_update` as the detector (`qemu/hw/misc/nvkvm/nvkvm.c:1032-1091`), and the
comment records that the C artifact has neither. Credit where due — this is the shape the
other two want.

### 3.3 ★★★ multi-GPU × guest driver version — the empty cell

**Not built, not refused, not represented, and not mentioned.** `[read]`

- `kayfabe-abi` — home of `DriverVersion`, `DriverAbiTable`, `CapabilityTable` — contains
  **zero** occurrences of `GpuId`. Same for `kayfabe-gsp`, `kayfabe-rmrpc`, `kayfabe-chips`.
- The guest-edge decoder holds **one** table for the whole device:
  `GraphPolicy { abi: &DriverAbiTable, guest_os, gpu: &mut Gpu }`
  (`crates/kayfabe-rmrpc/src/policy.rs:128-155`) — and that one `&mut Gpu` contains *all*
  `GpuTarget`s. The register plane likewise: `RegPlane::new(chip, abi, clock)`
  (`crates/kayfabe-device/src/plane.rs:396-429`).
- `four_axes_of_variation.md` never says "multi-GPU", "GpuId" or "per-GPU" once.

⇒ **structurally excluded, but by a hold one level too high — not by a global.** A statics
audit across `kayfabe-core`, `-fwd`, `-arch` finds no `OnceLock`/`lazy_static`/singleton
holding any axis value; every exclusion is one field on the wrong struct. That is the good
news: lifting it is threading `GpuId` into `GraphPolicy`/`RegPlane`, not a teardown. It is
also exactly the bolt-on `gr_multigpu_seam_audit.md:8-13` exists to prevent, and nobody
applied that bar to Axis A.

### 3.4 multi-GPU × host driver, × guest OS — N/A, and say *why*

- **× host driver: N/A by physics.** One kernel module ⇒ one RM ⇒ one version across every
  `/dev/nvidiaN`. The pin is nonetheless re-checked per `RmConnection::open`, i.e. per
  `(Proc, GpuId)` isolate — N identical answers, harmless, and nothing compares across.
  ★ **Incidental finding:** the guest's `GpuId` is used *raw* as the host device-node index
  (`format!("nvidia{}", gpu.0)`, `crates/kayfabe-isolate-host/src/rm.rs:499,995`). There is
  no guest-`GpuId` → host-GPU translation table, and whether "the entitlement roster is the
  host roster" is intended is stated nowhere.
- **× guest OS: N/A by semantics.** One VM has one kernel; two adapters are enumerated by
  that one kernel's one driver. `GuestOs` is correctly held once on `GraphPolicy`, not per
  target. Not a gap — worth writing down so nobody later "fixes" it.

### 3.5 ★★ multi-GPU × chip, same arch — the *silent-wrong* cell

`GpuTarget` has exactly three fields and **none of them is a chip**
(`crates/kayfabe-core/src/gpu.rs:1140-1148`); the chip lives on `RegPlane`, of which there
is one. `MG-6`'s guard compares `arch.name()`, so **GA106 + GA102 is same-arch, is not
refused, and is not representable**: two parts with different device-info tables, BAR
geometry and VBIOS would share one `RegPlane`'s answers. `[read]`

This is the sharpest of the four multi-GPU cross-gaps because it is the only one that is
**legal, common (a 3060 next to a 3090), and silent** rather than loudly refused.

### 3.6 The named example, answered

> *"does a 575 guest on a GH100-shaped adapter with two GPUs work?"*

**Structurally unreachable, three times over, and the three refusals arrive in this order:**
1. two GPUs — the guest edge cannot present a second PCI function at all (§3.1);
2. GH100 — `CHIPS` has no Hopper row, so `chip_for_device_id` refuses `NoChipForDevice`
   before a register is answered;
3. 575 — `GUEST_DRIVER` is a build const, and even after a source edit the 575 row is
   `vgx: None`, so fn 1 refuses.

Every one of the three is a **loud, named refusal**. That is the property worth keeping.

## 4. ⊘ What I could not determine

These are UNKNOWN. Each is a real finding; none is padded into a guess.

1. **Whether `VGX_MAJOR/MINOR` is constant across 580.x.** The pair `0x2B/0x13` is read at
   `ogkm-580` = **580.159.04**, and attributed to a table row keyed **580.65.06**. Only
   580.159.04 and 610.43.02 are vendored, so the 65.06 end of that row is an assumption. Low
   stakes (the failure is a refusal), but it is an unread tag stated as a row key.
2. **What the C ever ran mismatched.** `four_axes_of_variation.md` §3 says it is not known
   whether the C artifact ever ran guest and host at different versions. I could not
   establish it either.
3. **The reference capture's guest kernel.** §2.5 — `traces/README.md` says `6.8.0-117`;
   the artifact's provenance block does not carry a guest kernel at all. I cannot tell
   whether the README is right, and the decoder gate cannot either.
4. **Whether a `GuestOs::Windows` boot would reach the refusal.** The refusal lives in an
   unlinked crate on a plane the boot has not reached. Whether a WDDM guest would fail
   earlier (in the boot FSM, on something OS-shaped I did not find) is unknown — I found no
   Windows-hostile code in the boot plane, which is weaker than knowing there is none.
5. **Whether the entitlement roster is meant to be the host GPU roster.** §3.4. The raw
   `GpuId → nvidia{N}` mapping is either the intent or an unexamined coincidence; nothing
   in the tree says which.
6. **What a guest kernel outside the 6.8 family does.** The 117↔136 evidence is within one
   Ubuntu ABI family. Nothing is known about 5.x or 7.x, and nothing would report it.

## 5. ★ The one cell to measure on hardware first

**A 575.51.02 guest against the current device, on the existing GA106 bench, host unchanged.**

Not because 575 matters, but because it is the cheapest experiment that discriminates
between two readings of this entire matrix:

- it needs **no new hardware** (same box, same host driver, same chip) — only a different
  guest driver install and a one-line `GUEST_DRIVER` edit;
- ★ **it is answerable at the rung the ladder is already on.** Almost every other question
  in this matrix needs bring-up to get further than it has; this one does not. fn 1 is
  *behind* the current rung (`3946897`: chip info, fn 76, answered), so the whole
  experiment lives inside territory the boot already crosses — the ladder simply falls
  back to rung 1, visibly, in the `unserviced` ledger;
- it makes the §2.1 prediction **falsifiable**: the boot must stop at RPC fn 1 with our own
  `NV_ERR_NOT_SUPPORTED`, and the `unserviced` ledger must name fn 1 and nothing before it.
  If it stops *earlier*, something in the pre-RPC boot plane is version-dependent in a way
  no table records — which would be the most valuable single fact this audit could turn up;
- it is the first step of the experiment `four_axes_of_variation.md` §3 already calls for
  ("walk the guest driver back through 575/570 until something refuses — and record **what**
  refuses"), and it converts the estimated drift range into a measured one;
- and it is the only axis where a negative result is cheap and a positive result is a
  product claim.

**Runner-up, if hardware is plentiful rather than scarce:** two GA106 in one guest — because
§2.6's interrupt cell and §3.5's chip cell are both invisible to every test in the tree, and
a 2-GPU guest is the only thing that can see them.

## 6. Regenerating this

```
scripts/compat_matrix.py            # the derived matrix
scripts/compat_matrix.py --check    # non-zero if a derived fact moved
```

⊘ Three things the deriver cannot see, so a green is not over-read: it reads
**declarations**, not behaviour (a `TABLES` row means a table exists, never that a guest
boots); reachability proves a crate is **linked**, not that any path calls into it; and the
negative sweeps are token searches, blind to an assumption written without the token —
exactly as `claim_ledger.py` is blind to a claim made without a claim word.
