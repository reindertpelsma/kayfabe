> ## ★★★ SUPERSEDED IN PART — 2026-07-29 (owner decision)
>
> **§5.4's memory-plane mechanism is replaced.** Do not express the guest-visible map
> through QEMU's `MemoryRegion` tree. QEMU reserves the GPA window and does not back it;
> we install our own memslot via `KVM_SET_USER_MEMORY_REGION` and populate by `mmap`,
> with `KVM_MEM_READONLY` giving the read-native tier directly. See
> **`host_execution_plane.md` §1** for the decision, its three arguments, the C citations
> (`nvkvm_mmap_host.c:482`, `nvkvm_isolate_handlers.c:1792`) and the named risks.
>
> Consequences for this document:
> * ★ **§4.3 vs §5.4 no longer conflict** — the contradiction dissolves rather than being
>   worked around, because nothing touches QEMU's tree after realize. `f0053ef`'s
>   latch-and-claim workaround becomes unnecessary.
> * ★ **§5.1's pointer hand-over is not needed**, so Q2 does **not** owe
>   `kayfabe-linux-raw` a relaxation — the unwrap stays inside `kvm_unsafe.rs` and the
>   ratchet stays at 37.
> * §5.4's "classify the overlay `Device`" is dropped; it contradicted
>   `Vmm::map_read_native`'s own contract and made Q1's "identical operation logs"
>   acceptance unachievable.
>
> The rest of this document — the crate split, the lock ladder, §9.3's three-part gate,
> the doorbell decision and every QEMU-facility citation — **stands unchanged**.
>
> ## ★★★ SECOND AMENDMENT — 2026-07-29, stage Q2–Q5 BUILT
>
> `host_execution_plane.md` **§1.6** records what the build found. The four that amend *this*
> document:
>
> * ★★★ **§4.3's realize-only confinement no longer applies to the memory plane at all**, and
>   §5.4's realize-time overlay class is gone with it. There is no topology transaction left:
>   installing a window is a call to the **kernel**. `Vmm::map_read_native` creates a window at
>   runtime, exactly as the sibling backend does, and its four "you may only claim what realize
>   made" refusals are deleted. `TOPOLOGY_AFTER_REALIZE` lost its subject.
> * ★★★ **§8.5's balloon argument is VOID and decision Q4 stands on a new one** (task #97). Our
>   reservation is not a hypervisor RAM block, so the balloon skips it trivially; the live
>   hazard is guest RAM **exported to isolates** — a shared `memfd` whose backing pages a
>   hole-punching discard destroys for *every* mapping of the file. The `-EBUSY` arm must name
>   the conflicting device, not a class.
> * ★★ **Decision Q8 is superseded.** `map_read_native` is not a ROM-device overlay with
>   hypervisor-owned backing; it is a memslot with the kernel's read-only flag over **our own**
>   reservation. So §5.4's "the backing is not ours and nothing may be placed into it" is gone,
>   and a caller-supplied backing is honoured rather than refused.
> * ★ **§12 item 7's pointer hand-over never happens**, so `kayfabe-linux-raw` still has no
>   base-address accessor. The one relaxation stage Q2 did cost is elsewhere and is argued in
>   `ci.yml`: a `BorrowedFd` over a number read out of `/proc/self/fd`, to duplicate the
>   hypervisor's own VM descriptor.
>
> **Not built:** the C QOM shim. `kayfabe-qemu-raw` is still empty — it needs a hypervisor
> source tree, and this machine has none. Stage Q2's *Rust* half, Q3's GPA plane, Q4's
> read-native tier and Q5's teardown are built and gated against a real kernel; the C half and
> its `-S`/QMP acceptances are not.

# L2-Q — the QEMU adapter: the device, the threads, the doorbell, and the lifecycle

**What this doc is.** The design for the last unbuilt layer between `kayfabe` and a running
guest: a real `kayfabe_vmm::Vmm` backend on QEMU, plus the device that carries it. It is the
milestone `l1_os_shell.md` §10 calls **L2-Q**, and it is written *after* the two source rounds
that were scheduled to precede it (`../reference/qemu_bql_spike.md`,
`../reference/qemu_102_facilities.md`) and *after* the first real backend
(`kayfabe-vmm-kvm`) existed to be copied from.

**What it is not.** Not a port of the C's `nvkvm_gpu_emul.c`. Not a performance plan — every
absolute number quoted here is inherited and every one of them is inherited *with its
caveat*. Not a claim that any of this has run: **nothing in this document was executed.** The
bench is offline at the provider and no QEMU tree exists on this machine.

**Ground truth, and how to read a claim.**

| tag | means |
|---|---|
| **[src]** | read from the named file at the named tag. **Every QEMU citation in this file was resolved against `v10.2.0` by fetching the file**, not inherited from another doc. Where an inherited citation was re-checked and moved, §12 says so. |
| **[measured]** | a number produced by a run. There are none new here; every one is quoted from `qemu_bql_spike.md` or `guest_memory_lock.md` **with its arm and its caveat**. |
| **[inferred]** | a conclusion drawn from those. |
| **[open]** | not settled, and the experiment is named. |

**Citation convention** (`testing_doctrine.md` §6.1): our own tree is cited by **symbol**
(`kayfabe_rt::SharedDevice::doorbell`), pinned external trees by `file:line` **with the tag**
(`[src] v10.2.0 include/system/memory.h:2354`).

> **★ Reading note — `R1` is overloaded across the design docs, and this file uses only one
> sense.** `l1_concurrency.md` §3.3 defines **R1–R5 as lock invariants** (R1 = no blocking under
> a lock, R3 = lock rank, R5 = revalidate after a lock gap). `l1_os_shell.md` §7.6 independently
> defines **R1–R11 as resource classes** (R1 = host RM objects, R2 = host VAS + mappings, …).
> **Every `R1`/`R3`/`R5` in this document is the lock invariant.** Resource classes are named in
> words here, never by number.

**Where this doc has no authority.** It does not amend `l1_os_shell.md`, `l1_concurrency.md`
or the two QEMU reference files. Where it disagrees with one of them it says so in §12 and
proposes the amendment; it does not make it.

### ★★ Method — how the QEMU citations in this file were obtained, and what that settles

`l1_os_shell.md:64` stamps **every** QEMU and cloud-hypervisor citation in the design docs
`[unverified]`, on the correct grounds that *"there is no QEMU tree and no cloud-hypervisor
checkout on this machine"*. That stamp is the right default and it is why
`memory_region_clear_global_locking()` — dead for five years — survived in a design doc.

**It does not apply to this file.** Every QEMU claim below was resolved by **fetching the file
from `raw.githubusercontent.com/qemu/qemu/v10.2.0/…` and grepping it**, one file at a time. The
following were read at the tag:

`include/system/memory.h`, `include/system/kvm.h`, `include/hw/pci/pci.h`,
`include/hw/pci/pci_device.h`, `include/hw/pci/msix.h`, `include/hw/pci/msi.h`,
`include/hw/qdev-core.h`, `include/hw/resettable.h`, `include/migration/blocker.h`,
`include/qemu/main-loop.h`, `include/qemu/event_notifier.h`, `include/qemu/module.h`,
`include/qemu/osdep.h`, `include/exec/memattrs.h`, `include/block/aio.h`,
`include/system/iothread.h`, `system/memory.c`, `system/physmem.c`, `util/module.c`,
`hw/virtio/virtio-balloon.c`, `hw/vfio-user/proxy.c`, `hw/vfio-user/pci.c`, `meson.build`,
`MAINTAINERS`, `VERSION`.

**What that settles.** Every `[src] v10.2.0 …` citation in this document is a **verified read at
that tag**, not an inherited claim, and §12 item 8 records the two inherited line numbers that
had drifted and the fact that **no inherited symbol was found to be absent**. Where a claim is
about *behaviour under load* rather than the presence or shape of a symbol, it is tagged
`[inferred]` or `[open]` and the experiment is named — because fetching a file proves what the
code says, never what it does.

**What that does NOT settle**, stated so the method is not over-read: nothing was **built**,
nothing was **run**, and each file was read for the specific symbols named here rather than in
full. A signature is settled; a semantics claim that depends on a call graph I did not walk is
still an inference and is tagged as one.

---

## 0. The blocker, and what its resolution changed

`crate_maturity_map.md` records L2-Q as *"Blocked on #57, deliberately"* — the argument being
that building a hypervisor adapter on a memory plane with a known R1 violation gives a later
hang two candidate causes instead of one.

**#57 is closed** (`020790c`). And the *shape* of its resolution is load-bearing for this
design, not merely its closure:

> The violation was not the two-lock split, the lock order, or a syscall inside a critical
> section. It was **`Arc` ownership**: `resolve` hands an accessor an `Arc<GuestWindow>` so the
> memcpy runs outside the view lock, and that same `Arc` is a *release* handle — when a removal
> raced a reader, the **reader's** clone became the last one and `munmap` ran on a thread
> legally holding rank 0.
>
> **R1 is about blocking calls, and a `Drop` is a call site.**

**Three consequences this adapter inherits directly**, because QEMU hands us more
destructor-shaped foreign resources than KVM does:

1. **`memory_region_unref()` is a `Drop`-shaped call site.** It is `object_unref` on the
   region's owner and can run a QOM finalizer. Any place we drop the last reference to a
   foreign `MemoryRegion` is subject to the #57 rule, and the adapter's answer is the same one
   `kayfabe_vmm_kvm::Plane::retire`/`collect_retired` already implements: the *plane* keeps a
   reference no accessor can take away, and collection happens at a door already proven
   lock-free.
2. **`leafwitness` must be adopted on day one.** `kayfabe_vmm_kvm::leaf`'s own rustdoc predicts
   *"the `leaf` witness will be forgotten by the QEMU adapter"*. It now lives in
   `kayfabe_util::leafwitness`; `kayfabe-vmm-qemu` depends on it directly and every lock
   accessor returns `(MutexGuard, leafwitness::Held)` as `kayfabe_vmm_kvm::Plane::view` does.
3. **The ranked witness is blind here too.** `lockwitness` sees only *ranked* locks. The BQL has
   no rank, our leaf locks have no rank, and a foreign destructor has no rank. Three blind
   spots, one discipline.

**So the blocker is gone and the reason it existed was worth the wait**: had L2-Q started
before `020790c`, the first QEMU hang would have had the KVM backend's defect as a live
candidate, and the actual cause — ownership — would have been the *last* thing looked at,
exactly as it was in `kayfabe-vmm-kvm`.

---

## 1. Inherited law, restated as L2-Q obligations

These are decided. This document builds on them and does not reopen them.

| # | Law | Where it was decided | What L2-Q owes it |
|---|---|---|---|
| L1 | **QEMU ≥ 10.2.0 is a hard floor. The backport is CANCELLED** — we run on a **stock** ≥ 10.2 with no patch of ours in it. | `l1_os_shell.md` §10 decision box (`c3ec258`) | A compile-time refusal **and** a realize-time assertion (§3.5), both negative-tested. ★ Five sites in `l1_os_shell.md` still describe a carried patch (`:1299`, `:2879`, `:3211` *"≥ 10.2.0, **or our patched 9.2.0**"*, decision #35, decision #48) and **§6.3.1 / §14.6 read present-tense as if the remedy were a backport**. They are a dated measurement, not a live plan; **nothing in this document designs against them.** |
| L2 | **★ There is no pairing left to maintain.** Upstream's single `memory_region_enable_lockless_io()` sets `disable_reentrancy_guard` itself. The one obligation that is genuinely **ours** is coverage: the guard's *state* is per-**device**, the opt-out is per-**`MemoryRegion`**. | `qemu_102_facilities.md` §2; §9.3's re-specified gate | ★ **Enumerability**, not just "one helper": one function holds the complete region table and applies the marking, so *"did we miss a region?"* is answered by reading one function — plus a realize-time self-check over that table (§3.3, §11). |
| L3 | **Once lockless IO is taken, R1/R3/R5 are CORRECTNESS, not latency.** | `qemu_bql_spike.md` §4; upstream's own doc comment | §4 is written against R1/R3/R5 rather than against latency, and §11 makes the re-audit a deliverable artifact. |
| L4 | **`gpa_read`/`gpa_write` must PROVE RAM**, never merely refuse devices. | `qemu_102_facilities.md` §11.3 + its three corrections; `kayfabe_vmm::GuestRamMap` | §5.2 — a `MemoryListener`-fed positive map, with a classification **narrower than `memory_region_is_ram()`** (§5.3, a finding). |
| L5 | **`migrate_add_blocker()` at realize** — it closes migration *and* CPR, a ninth lifecycle event §7.6 cannot implement. | `qemu_102_facilities.md` §9, §11.2 | §8.4, plus its refusal exercised. |
| L6 | **Regular traps, copy-once.** No userfaultfd, no `mprotect`, no read-only regions on the data path. | `c0f42fc` (governing directive) | §5.4 — and it deletes the RO-memslot fallback that `qemu_102_facilities.md` §11.1 had already shown is unreachable through QEMU anyway. |
| L7 | **One memslot per window, never per object**; a publication is a `MAP_FIXED` placement performing **no** VMM ioctl. | `l1_os_shell.md` §6.7 | §5.4, and the existing memslot-frequency CI gate must hold with the QEMU backend substituted. |
| L8 | **A memory-plane op requested by the guest runs on the calling thread with our locks dropped.** No memory-plane thread, no queue. | `l1_os_shell.md` §6.6 | §4.4 — and it is the reason the doorbell recommendation in §6 does *not* try to get the work off the vCPU. |
| L9 | **A second backend costs exactly one adapter crate: no trait change, no core change.** | `l1_os_shell.md` §6.0, decision #39 | §2 — and §12 records the one place this design came under pressure and did **not** move the trait. |

---

## 2. ★★ Packaging — where the code lives, and the two findings that decide it

This section is first because it constrains everything after it, and because **both of its
findings are new**: neither appears in `l1_os_shell.md`, `qemu_bql_spike.md` or
`qemu_102_facilities.md`.

### 2.1 ★★ FINDING — QEMU v10.2.0 has no supported out-of-tree device mechanism

The floor decision's central economic claim is that *"requiring a version is strictly less work
than patching one"*, and that it deletes *"the whole 'which QEMU are we built into' ambiguity"*.
The first half is true. **The second half is not available at v10.2.0**, and the reason is
mechanical:

**[src]** `v10.2.0 util/module.c:319` — `module_load_qom(type, errp)` resolves a QOM type name
to a module by scanning `module_info`, a table **generated at QEMU build time** by
`scripts/modinfo-generate.py`. A `.so` that is not in that table can never be discovered by
type name. `QEMU_MODULE_DIR` (`:235`) changes only the *directory* searched, not the table.

**[src]** `v10.2.0 util/module.c:176` — even a module found by path must export
`DSO_STAMP_FUN_STR`, and `include/qemu/module.h:18` defines it as
`glue(qemu_stamp, CONFIG_STAMP)`, a per-build stamp. The failure hint upstream prints is
verbatim: *"Only modules from the same build can be loaded."*

**[inferred]** Therefore the device's C half **must be compiled inside a QEMU source tree.**
There is no third option at this tag.

> ### ★ DECISION Q1 — an **additive overlay**, and the distinction from the cancelled backport is real but not free
>
> The shim ships as a directory the user drops into a stock QEMU 10.2.x checkout —
> `hw/misc/nvkvm/` — plus **two hunks**: one `meson.build` line and one `Kconfig` stanza.
>
> **Why this is not the fork the floor decision deleted.** The cancelled backport *modified
> upstream semantics* in `system/physmem.c`: it had to be re-derived against every release, and
> a mis-rebase would silently change how every device in the machine dispatched. An added
> directory changes nothing upstream does; its only conflict surface is the two hunks, and a
> conflict there is a build failure, not a behaviour change.
>
> **Why it is still a real cost, stated rather than argued away.** The floor decision's own
> accounting — *"a tracked patch, a rebase every release, a build script that must reproduce
> it, and a supply-chain claim we make to every user forever"* — still applies to items 2, 3
> and 4 of that list. **We do not get "install QEMU from your distro and run".** The user
> builds QEMU once. That is a materially worse install story than the decision box assumed, and
> it is the single largest unpaid cost in this milestone.
>
> **The escape exists and is named, not taken** — see §9.6 (vfio-user).

### 2.2 ★★ FINDING — the adapter cannot contain `unsafe`, and the CI gate says so on purpose

**[src, our tree]** `.github/workflows/ci.yml`, step *"Unsafe-containment gates"*:

- **Gate A** loops `for m in crates/*/Cargo.toml`, skipping exactly `kayfabe-linux-raw`, and
  requires `[lints]` / `workspace = true`. The workspace sets `unsafe_code = "forbid"`. So **any
  new crate is `forbid(unsafe_code)` from its first commit** or CI is red on the first push.
- **Gate B** requires `find . -name '*_unsafe.rs' -not -path './crates/kayfabe-linux-raw/src/*'`
  to be **empty**.
- The **ratchet** greps `crates/kayfabe-linux-raw/src/*_unsafe.rs` for a single expected count.

Gate A's own failure text is the instruction: *"There is exactly ONE crate allowed to omit it
(kayfabe-linux-raw), and adding a second is a design decision, not a manifest edit."*

A QEMU adapter needs `extern "C"` entry points, raw `MemoryRegion *` handles, and adoption of a
host pointer QEMU owns. All three are `unsafe`. Putting them in `kayfabe-linux-raw` would be
wrong twice over: QEMU is not Linux, and `four_axes_of_variation.md` §1.1 licenses host-side
*OS* specificity in that crate, not host-side *hypervisor* specificity.

> ### ★★ DECISION Q2 — three crates, and `kayfabe-qemu-raw` is the **second** audited unsafe crate
>
> | crate | lints | contains |
> |---|---|---|
> | `hw/misc/nvkvm/` (C, in the QEMU tree) | — | **QOM only**: the type, `class_init`, realize/unrealize, the three reset phases, `MemoryRegionOps` trampolines, the `MemoryListener` struct. Every trampoline body is one call into Rust. No logic. |
> | `kayfabe-qemu-raw` | `unsafe_code = "allow"`, `undocumented_unsafe_blocks = "deny"` | the entire FFI surface, in `*_unsafe.rs` files only: the `extern "C"` entry points, the typed wrappers over the ~20 QEMU functions of §3/§7/§8, and `ForeignMapping` (adoption of a host pointer QEMU owns). |
> | `kayfabe-vmm-qemu` | `workspace = true` ⇒ **`forbid(unsafe_code)`** | `impl Vmm`, the `GuestRamMap`, the two leaf locks, plan/execute/commit, R5 generations, retirement. **All of the logic.** |
>
> **This is not a new architecture; it is the existing one.** `kayfabe-vmm-kvm` is
> `forbid(unsafe_code)` and gets every effect through `kayfabe-linux-raw`. The QEMU column is
> the same split with a different raw crate.
>
> **What it costs, exactly, and it is a CI change L2-Q must land before any code:** gate A's
> skip list, gate B's `-not -path`, and the ratchet all move from a **constant** to a **named
> two-element list with per-crate expected counts**. The acceptance criterion the gate exists
> to protect — *"read one crate in one sitting"* — becomes "read two", and that is the honest
> price. It should be paid deliberately, in its own commit, with the second crate's blocks
> itemised in the ci.yml comment exactly as the first crate's 37 are.

### 2.3 The call direction, and why it is one-way at the seam

```
   QEMU vCPU thread ──trap──▶ C trampoline ──▶ nvkvm_mmio_write(dev, bar, off, size, val)
                                                   │  (kayfabe-qemu-raw, *_unsafe.rs)
                                                   ▼
                                        kayfabe-vmm-qemu :: QemuVmm  (safe)
                                                   │
                                                   ▼
                                        dyn Device  ──▶  kayfabe_rt::SharedDevice
```

The C never calls Rust logic and Rust never calls C logic; both call *primitives*. The C's
entire job is to satisfy QOM, which is a macro system and therefore cannot be Rust
(**[src]** `qemu_102_facilities.md` row 17: `v10.2.0 rust/hw` contains `char`, `core` and
`timer` — **no `pci`** — so a PCI device is not expressible in the in-tree Rust workspace at
this tag; independently re-checked).

---

## 3. The device model

### 3.1 The QOM type

A `TYPE_PCI_DEVICE` subclass. **[src]** `v10.2.0 include/hw/pci/pci_device.h:26` —
`PCIDeviceClass` with `void (*realize)(PCIDevice *, Error **)` at `:29` and
`vendor_id`/`device_id`/`class_id` at `:34`/`:35`/`:37`. Properties via
`device_class_set_props()` (**[src]** `v10.2.0 include/hw/qdev-core.h:953`).

`device_class_set_legacy_reset()` exists at `v10.2.0 include/hw/qdev-core.h:1002` and its
field carries a *"deprecated… TODO: remove"* comment at `:165-172`. **We do not use it** —
`ResettableClass` three-phase, per §8.2.

### 3.2 BARs

| BAR | class | QEMU region kind | trapped? |
|---|---|---|---|
| **BAR0** | register aperture — boot regs, the faked GSP, IRQ status, the doorbell | `memory_region_init_io` (**[src]** `v10.2.0 include/system/memory.h:1363`) | yes, wholly |
| **BAR1** | GMMU-walked aperture — USERD / GPFIFO CPU access | a **container** with RAM subregions from our reservation, plus IO subregions where a page must be observed | mixed — §5.4 |
| **BAR2** | second GMMU-walked aperture | as BAR1 | mixed |
| MSI-X | table + PBA | `msix_init_exclusive_bar` (**[src]** `v10.2.0 include/hw/pci/msix.h:15`) | QEMU's |

Registration is `pci_register_bar(dev, n, attr, mr)` (**[src]** `v10.2.0
include/hw/pci/pci.h:255`).

`MemoryRegionOps` (**[src]** `v10.2.0 include/system/memory.h:293`) is filled with an explicit
`.valid.min_access_size` / `.max_access_size` / `.unaligned` and `.impl` triple. **Do not leave
`impl` at zero.** **[src]** `v10.2.0 system/memory.c:540-542`: a zero `access_size_max` is
silently defaulted to **4**, so an 8-byte guest access to a register we model as 64-bit would be
split into two 4-byte callbacks without a word of warning. The C's register model must be read
for width before this is filled in, per register class.

`rust/system/src/memory.rs`'s `MemoryRegionOpsBuilder` (`v10.2.0`, ~195 lines) is a good model
for a const-buildable ops table — **a design to read, not a dependency to add** (§2.2).

### 3.3 ★ Lockless IO — one helper, every region

**[src]** `v10.2.0 system/memory.c:2567-2580`, in full, because the whole point is that it is
one function:

```c
void memory_region_enable_lockless_io(MemoryRegion *mr)
{
    mr->lockless_io = true;
    /* reentrancy_guard has per device scope … Turn it off for lock-less IO
     * enabled devices, to allow concurrent IO.
     * TODO: remove this when reentrancy_guard becomes per transaction. */
    mr->disable_reentrancy_guard = true;
}
```

Honoured at **[src]** `v10.2.0 system/physmem.c:3196-3209`
(`if (!bql_locked() && !mr->lockless_io) { bql_lock(); … }`).

> ### ★★ NORMATIVE — the obligation is COVERAGE, and coverage must be ENUMERABLE
>
> There is **no pairing for us to maintain** (the quoted body is one function). §9.3's
> re-specified gate has three clauses, and only the third is a real design obligation:
>
> (a) **exactly one** call site for `memory_region_enable_lockless_io`, in one adapter helper;
> (b) **zero** occurrences tree-wide of `disable_reentrancy_guard`, `bql_unlock`, or a
>     hand-rolled unlock/lock around a dispatch — on ≥ 10.2 that construction is the only way
>     left to reproduce the measured 47 % silent drop;
> (c) ★ **every trapped region of the device is marked, not just the hot one.**
>
> (a) and (b) are greps. **(c) is ours, and a grep cannot answer it** — a region is missed by
> *omission*, and omission has no token to match. So (c) is discharged structurally:
>
> 1. **One region table.** `nvkvm_regions[]` in the shim is the complete, literal enumeration of
>    every `MemoryRegion` this device owns, each row carrying its BAR, offset, size, kind
>    (`IO` / `RAM` / `ROM_DEVICE`) and its ops. There is no region constructed outside it.
> 2. **One constructor.** `nvkvm_region_init_io()` initialises an IO region **and** marks it
>    lockless. It is the only caller of `memory_region_init_io` in the tree.
> 3. **One registration loop.** `nvkvm_bars_realize()` walks `nvkvm_regions[]` and is the only
>    place `pci_register_bar` / `memory_region_add_subregion*` appear. *"Did we miss a region?"*
>    is answered by reading that one function against that one table.
> 4. **A realize-time self-check.** After the loop, walk the table again and assert
>    `mr->lockless_io` for every row whose kind is `IO`, and assert the count of IO rows equals
>    the number of `nvkvm_region_init_io` calls made. **[src]** `lockless_io` is a public field
>    at `v10.2.0 include/system/memory.h:836`, so this is readable, not inferred.
>    This is the clause that catches the failure the gate cannot: a region added later, by
>    someone who read (a) as "one call site, therefore done".
>
> **Why the asymmetry makes omission the dangerous case.** **[src]** `v10.2.0
> system/memory.c:545-555`: `engaged_in_io` lives on `mr->dev->mem_reentrancy_guard`
> (per-device) while `disable_reentrancy_guard` lives on the `MemoryRegion`
> (**[src]** `include/system/memory.h:869`). An **unmarked** region still sets the device-wide
> flag and still returns `MEMTX_ACCESS_ERROR` to a concurrent access — so a device with a
> lockless BAR0 and a stock BAR2 keeps **both** hazards on BAR2 (the BQL *and* the silent drop)
> **while passing any per-device check.** A missed region is not a missed optimisation; it is a
> vCPU reading values we never produced.

**Two precision points that survive re-checking, and one narrowing:**

1. **[src]** `v10.2.0 system/memory.c:545-546` — the guard skips
   `mr->ram_device || mr->ram || mr->rom_device || mr->readonly`. So RAM-backed BAR1/BAR2
   subregions were never guarded and never serialised: **R1/R3/R5 were already correctness
   requirements on those paths before lockless IO was taken.** `qemu_bql_spike.md` §4's
   *"any two vCPUs touching any two regions of the same device collide"* is over-general in
   exactly this direction.
2. **The flag suppresses *acquiring* the BQL, never *releasing* one we already hold.** A
   dispatch we originate from a BQL-holding context of our own (a reset phase, a listener
   callback, a monitor command) still runs the handler BQL-held, and the flag will not say so.
   §4.2 is the list that covers it.
3. **A read that returns `MEMTX_ACCESS_ERROR` is a value the guest reads that we never
   produced** — the 47 % measurement's real content. On ≥ 10.2 the only way left to reproduce
   it is a hand-rolled `bql_unlock()`/`bql_lock()` around a dispatch, which §11 forbids
   tree-wide.

### 3.4 ★ TCG is not a supported configuration, and this must be loud

**[src]** `memory_region_enable_lockless_io` is honoured only on the `physmem.c` dispatch path
reached from the KVM MMIO exit; upstream's own note is that TCG ignores the flag. A TCG guest
would therefore run our device **BQL-held on every access**, i.e. in exactly the configuration
`qemu_bql_spike.md` §5 measured as 5.3× degraded and `l1_os_shell.md` §6.6 classifies as the
sharpest I-NOAMP violation in the design.

⇒ **realize refuses unless `kvm_enabled()`** (**[src]** `v10.2.0 include/system/kvm.h:47`). This
is the §9.3 pattern: a deployment fact no type and no grep can observe, answered by a loud
refusal, never a silent slow mode.

### 3.5 The floor assertion — two of them, and neither is the other's substitute

**Compile-time.** **[src]** `v10.2.0 meson.build:2595-2596` sets `QEMU_VERSION_MAJOR` and
`QEMU_VERSION_MINOR` into `config-host.h`, and **[src]** `v10.2.0 include/qemu/osdep.h:34`
includes `config-host.h` in every QEMU translation unit. So the shim opens with an `#error` on
`QEMU_VERSION_MAJOR < 10 || (== 10 && QEMU_VERSION_MINOR < 2)`. A tree that is too old **fails
to build**, naming the floor.

**Realize-time.** The compile-time check is a claim about the *headers*. §9.3's gate row is
about the *binary*, and the two can differ (a header-only mismatch, an ABI-compatible relink).
So realize also asserts, and refuses with the version string in the error. Both are
negative-tested; neither is dropped because the other exists.

> **Why the check is not "does `memory_region_enable_lockless_io` exist".** That was the shape
> when a backport was in play. Under a version floor the symbol is *always* present, so a
> presence check is vacuously green and would tell us nothing — the exact failure mode
> `testing_doctrine.md` §1 catalogues. The version is the thing being asserted.

---

## 4. ★★ The threading story, stated against R1/R3/R5

### 4.1 The lock ladder, with the foreign lock placed

```
   BQL            (foreign, unrankable, QEMU's)     ── outermost, on the paths that have it
     │
   rank 0         device read/write                 ── ours, ranked, lockwitness
     │
   rank 1         per-Proc                          ── ours, ranked, lockwitness
     │
   leaf(view)  ·  leaf(installer)                   ── ours, unranked, leafwitness
```

**The rule is a direction, not an ordering table.** `l1_os_shell.md` §6.3: *no lock the VMM owns
may be acquired **beneath** one of our locks; and our entry paths may arrive with one already
held.* BQL-above-ours is the safe direction and is unavoidable; ours-above-BQL is the ABBA and
is forbidden. The adapter never takes the BQL from a thread holding any lock of ours, and every
leaf critical section is a bounded `BTreeMap` probe plus an `Arc` clone that calls into nothing.

### 4.2 ★ Which callbacks arrive BQL-held — the written list §6.3 enforcement item 3 demands

This table **is** the review artifact. It is not a summary of one.

| entry point | BQL held? | evidence | what may run there |
|---|---|---|---|
| `mmio_read` / `mmio_write` on a **lockless** IO region, from a vCPU | **NO** | **[src]** `v10.2.0 accel/kvm/kvm-all.c` `KVM_EXIT_MMIO` is commented *"Called outside BQL"*; `physmem.c:3200` then declines to take it | the full core path: rank 0 → rank 1 → lock-free isolate round trip (§4.4) |
| the same handler, reached from a BQL-holding context of ours | **YES** | `physmem.c:3200` — the flag suppresses acquiring, not releasing | nothing. There is no such path by construction (§4.3) |
| `MemoryListener::region_add` / `region_del` / `begin` / `commit` | **YES** | **[src]** `v10.2.0 system/memory.c:1143-1148` — `memory_region_transaction_commit` asserts `bql_locked()`, and it is what drives the listeners | a bounded update of our leaf map, and **nothing else** (§5.2) |
| `ResettableClass` `enter` / `hold` / `exit` | **YES** | **[src]** `v10.2.0 include/hw/resettable.h:50` — *"This whole API must only be used when holding the iothread mutex."* | latch a flag, wake the executor, return (§8.2) |
| `realize` / `unrealize` | **YES** | qdev's `realize` runs from the machine-init / device-add path | all of it — these are the one legal BQL acquisition context (§4.3) |
| an `EventNotifier` we registered in **our own** reactor's epoll set | **NO** | **[src]** `v10.2.0 accel/kvm/kvm-all.c:1889-1905` — KVM registration passes only the fd and installs **no** read handler; nothing puts it on the main loop unless the device asks | ★ **hand off, never block.** No BQL — and no licence either: see the box below |
| a QEMU bottom half | **YES** | it is a main-loop context | **we schedule none.** §4.5 |

> ### ★★ The hand-off rule SURVIVES the read-side finding — only its justification changed
>
> `qemu_102_facilities.md` §10.1 item 2 concludes that putting the eventfd in our own reactor
> *"discharges the caveat structurally"* and therefore **deletes** L2-Q task 4's hand-off clause
> and §6.6 item 2's compensating rule. **The premise is right and the conclusion does not
> follow, and this design keeps both rules.**
>
> The premise: QEMU installs no read handler, so the fd is ours and the wake need not arrive on
> the main loop under the BQL. Verified above. What that deletes is the *reason* the spike gave
> — *"because it runs on the main loop under the BQL"* — and nothing else.
>
> **Our reactor is not a place you may block either**, for two reasons that have nothing to do
> with QEMU:
>
> 1. **The reactor is one thread for every source in the device.** `kayfabe_shell::Reactor`
>    resolves a token and pushes a `CoreEvent`; blocking there stalls **every** completion
>    source, every isolate relay and every timer at once. `l1_os_shell.md` §3.7 additionally
>    forbids the loop thread from draining the inbox at all, because law 9 keeps core state off
>    that thread — so the reactor is a hand-off **by construction** and cannot host the work
>    even if someone wanted it to.
> 2. **The executor it hands off to is serialized.** A verb round trip performed on the executor
>    parks every other proc's deferred reap, `pending_release` drain and re-delivery sweep behind
>    one process's host call. That is the **same I-NOAMP amplification the BQL had, relocated
>    into our own runtime** — and unlike the BQL it would be entirely our own doing.
>
> ⇒ **The rule stands: anything reaching us on an eventfd hands off; it does not block.** Its
> justification is now *the shared reactor thread and the serialized executor*, which is a
> stronger and more portable reason than the one it replaces — it holds on a backend with no
> global lock at all.
>
> **And it is the third independent argument against Q-D3** (§6.4): the destination an ioeventfd
> would deliver to is precisely the thread that must not do the work.

### 4.3 ★ Exactly one BQL acquisition site — and it is a *context*, not a call

§6.3 enforcement item 2 says the adapter must have exactly one function that takes the BQL. On
≥ 10.2 the honest form is slightly different and stronger:

> **The adapter contains ZERO calls to `bql_lock`.** Every BQL-requiring operation — every
> `memory_region_*` topology mutation, `pci_register_bar`, `msix_init_exclusive_bar`,
> `memory_region_add_eventfd`, `memory_listener_register`, `migrate_add_blocker` — is performed
> **only from realize/unrealize**, which QEMU already entered BQL-held. Zero sites is a pass,
> and §9.3's gate row says so.

This is why the coarse memory tier is realize-scoped (§5.4) and why the doorbell recommendation
in §6 avoids a mechanism whose registration would have to happen at channel-allocation
frequency: **registration is a memory transaction and therefore a BQL obligation** (**[src]**
`v10.2.0 system/memory.c:2582-2616` → `memory_region_transaction_commit`'s
`assert(bql_locked())` at `:1148`).

### 4.4 R1 on the trap path — the vCPU does the work, and that is the design

With lockless IO a trapped write arrives on the vCPU thread holding **no** foreign lock. It then
runs the ordinary core path: `kayfabe_rt::SharedDevice`'s route phase under rank 0, the act
phase under rank 1, and the isolate round trip **lock-free on that same thread**
(`l1_os_shell.md` §6.6, law L8).

**This blocks the vCPU, on purpose.** §6.6's argument transfers verbatim: the block is
self-limiting, and it limits *precisely the guest process that caused it*. The two things that
would make it unacceptable are both gone — the BQL (deleted by the floor) and a queue of our own
(never built).

**I-NOAMP, restated for this adapter:** process A's doorbell may block A's vCPU for a round trip;
it must not appear in B's latency. Under lockless IO the only remaining shared serialisation is
the memslot (§5.4, frequency-bounded by L7) and KVM's own `slots_lock`. That is the honest claim,
and it is testable as the §5 A/B shape re-run **against our device** — which §11's gate requires
and which has never been done.

**R3.** The BQL is unrankable and stays unranked. What replaces R3 on this path is the direction
rule of §4.1 plus the table of §4.2 — a review artifact, and §6.3 already says there is no
mechanism here and that claiming one would be the failure this project catalogues.

**R5.** Every gap between a plan and a commit is re-validated against a per-window generation,
exactly as `kayfabe_vmm_kvm::Installer` does. The QEMU-specific addition is that a
**`MemoryListener` callback can invalidate a cached foreign pointer between our plan and our
commit** — see §5.5.

### 4.5 Where the executor and the reactor attach

`qemu_102_facilities.md` §7 already retired the assumption that the executor must be QEMU's main
loop. Restated as a decision:

> ### ★ DECISION Q3 — our threads stay ours; we schedule no bottom half
>
> The reactor is `kayfabe_shell::Reactor` on its own thread, epoll, unchanged. The executor is
> `kayfabe_rt::Executor` on its own thread, and `ExecutorWaker` is the same
> `kayfabe_rt::Parker` both backends already use. **No `aio_bh_schedule_oneshot`, no
> `qemu_bh_new_guarded`, no `IOThread`.**
>
> The one thing a bottom half is required for is work that must run BQL-held. §4.3 removes that
> requirement by confining every such operation to realize/unrealize. If a future need arises
> that genuinely cannot be realize-scoped, a BH is the legal spelling — and it becomes the
> single BQL acquisition context, re-opening §9.3's gate row as a "exactly one" rather than a
> "zero".

**`Vmm` is `Send` and not `Sync`** (`kayfabe_vmm::Vmm`), so each thread that can enter the core
carries its own handle over one shared `Arc<Plane>`, exactly as `kayfabe_vmm_kvm::KvmVmm` does.
That is: one handle per vCPU thread, one for the executor, one for the reactor's hand-off, and
one held by the realize-time machine object.

---

## 5. The memory plane

### 5.1 The window

`l1_os_shell.md` §4.4 / §6.7's shape is unchanged: **we** mmap one large
`MAP_ANONYMOUS|MAP_NORESERVE` reservation and hand QEMU the pointer.

**[src]** `v10.2.0 include/system/memory.h:1531` — `memory_region_init_ram_ptr()` →
`qemu_ram_alloc_from_ptr()`, which sets `RAM_PREALLOC` (**[src]** `v10.2.0
system/physmem.c:2569`), and **[src]** `:2494` asserts `!host ^ (ram_flags & RAM_PREALLOC)` —
handing QEMU a pointer and having QEMU manage the memory are the same choice, taken once.

`RAM_PREALLOC` is what makes the shape safe, by three early-outs re-verified at the tag:
`qemu_ram_remap` (`:2684`) and `reclaim_ramblock` (`:2591`) both skip it, and the shared/aux-ram
promotion branch at `:2498` is `!host`-gated. **The direction of that guarantee is the part worth
restating: QEMU will never `munmap` our window, therefore teardown is entirely ours** (§8.5).

### 5.2 The `GuestRamMap`, fed by a `MemoryListener`

`qemu_102_facilities.md` §13 item 8 records that no `MemoryListener` design was done. Here it is.

`kayfabe_vmm::GuestRamMap` is the one place a guest-chosen GPA is proven RAM, and its rustdoc is
explicit that the map is *"maintained by installs we perform … plus whatever topology the adapter
learns"*. On QEMU the learning mechanism is a `MemoryListener`:

**[src]** `v10.2.0 include/system/memory.h:892` — `struct MemoryListener` with
`region_add`/`region_del` at `:922`/`:934`, taking a `MemoryRegionSection *`
(**[src]** `:98-107`: `mr`, `offset_within_region`, `offset_within_address_space`, `size`,
`readonly`, `nonvolatile`, `unmergeable`). Registered with `memory_listener_register(l, filter)`
(**[src]** `:2654`) against `pci_get_address_space(dev)` (**[src]** `v10.2.0
include/hw/pci/pci_device.h:235`).

Per section, on `region_add`:

- classify (§5.3);
- if RAM: `memory_region_ref(mr)`, cache `memory_region_get_ram_ptr(mr) + offset_within_region`
  (**[src]** `v10.2.0 include/system/memory.h:2094`), and `GuestRamMap::declare(.., Ram, ..)`;
- if not RAM: `GuestRamMap::declare(.., Device, ..)` — **declared, not omitted**, so a device GPA
  reports `NonRamGpa` rather than the near-neighbour `BadGpa`, which
  `testing_doctrine.md` §2 rule 3 requires and which
  `memory_plane.rs::a_device_region_is_the_absence_of_a_memslot_and_refuses_by_name` already
  asserts for the KVM backend.

On `region_del`: `GuestRamMap::undeclare` **first**, then drop the cached pointer, then
`memory_region_unref`. That is `kayfabe_vmm_kvm::KvmMachine::remove_window`'s order — undeclare
before release, so there is no instant at which a resolved offset points at a released mapping —
and the `unref` last is #57's rule applied to a foreign destructor (§0). `region_del` arrives
BQL-held, so a QOM finalizer running there is legal.

The listener holds **only** the leaf(view) lock, for a bounded map update. It takes no rank and
calls nothing. Lock order on that path is `BQL → leaf(view)`, which is the safe direction (§4.1);
the reverse never occurs because no leaf critical section calls QEMU.

### 5.3 ★★ FINDING — `memory_region_is_ram()` is the wrong test, and v10.2.0 exposes no complete one

The obvious listener classifies with `memory_region_is_ram(mr)`. That is wrong in **three**
directions, and only the first is recorded in `qemu_102_facilities.md`:

1. **`ram_device`.** **[src]** `v10.2.0 include/system/memory.h:3136-3151` —
   `memory_region_supports_direct_access` excludes `ram_device`, commented *"RAM DEVICE regions
   can be accessed directly using memcpy, but it might be MMIO… So we treat this as IO"* — yet
   `memory_region_is_ram()` returns **true** for one, because `mr->ram` is set. A VFIO-mapped BAR
   is exactly this shape. Predicate available: `memory_region_is_ram_device()` (**[src]** `:1800`).
2. **`rom_device`.** `memory_region_init_rom_device*` produces a region where `mr->ram` is true,
   *reads* are direct, and *writes* go to callbacks. Memcpy-ing a `gpa_write` into it would
   bypass the device's own write path. **[src]** the only public predicates are
   `memory_region_is_romd()` (`:1810-1812`, `mr->rom_device && mr->romd_mode`) and
   `memory_region_is_rom()` (`:2036`, `mr->ram && mr->readonly`) — **neither is the test**: a
   `rom_device` with `romd_mode` off passes both and is still not memcpy-able.
   > **There is no accessor for `mr->rom_device` at v10.2.0.** The field is public
   > (**[src]** `:834`, `romd_mode` at `:829`), so the shim reads it directly. That is a
   > deliberate reach into a struct, and it is recorded here as such rather than hidden behind a
   > helper that would read as an API.
3. **`section->readonly`.** A read-only section is not a write target, and `flatrange_equal`
   compares `readonly` (**[src]** `v10.2.0 system/memory.c:251-260`), so it is a first-class
   property of the section rather than a hint.

> ### ★★★ NORMATIVE — the classification, positive and complete
>
> ```
> Ram  ⟺  memory_region_is_ram(mr)
>          && !memory_region_is_ram_device(mr)
>          && !mr->rom_device
>          && !section->readonly
>          && !section->nonvolatile
> Device otherwise — including "we do not recognise it"
> ```
>
> **The rule is *prove RAM*, not *refuse MMIO*** (`qemu_102_facilities.md` §11.3 correction 1).
> Do not reason from "unassigned memory is harmless": it happens to be lockless at v10.2.0
> (**[src]** `system/physmem.c:3010-3017`, *"Trivially thread-safe since memory accesses are
> rejected"*), and every ordinary device region is not — so a deny-list is simultaneously
> over- and under-inclusive.

### 5.4 The two tiers

**Coarse — realize-scoped.** Our window becomes one `memory_region_init_ram_ptr` region added as
a BAR subregion. **[src]** `v10.2.0 accel/kvm/kvm-all.c:111` — `kvm_max_slot_size` is `~0` and no
x86/arm64 caller moves it, so one region is one memslot; `qemu_102_facilities.md` row 6 verdict
stands unchanged.

**Fine — the data plane.** A publication is a `MAP_FIXED` placement inside the window, performed
by `kayfabe_linux_raw`'s `Reservation`, and it performs **no QEMU call and no KVM ioctl at all**.
An un-publication is `Reservation::restore` — `MAP_FIXED|MAP_ANONYMOUS`, **never `munmap`**,
which would punch a hole in the VMA under a live memslot.

**`map_read_native`.** The KVM backend spells this as a read-only memslot over the rounded write-
trap span. On QEMU that spelling is unavailable (`qemu_102_facilities.md` §11.1: through QEMU a
readonly change is a `region_del` + `region_add`; **[measured, KVM-direct, 2026-07-27]** KVM
refuses the flags-only flip with `-EINVAL` anyway) — and law L6 forbids it regardless. The QEMU
spelling is the **rom-device overlay the port already names** (`kayfabe_vmm::Vmm::map_read_native`
rustdoc: *"the `gsp_falcon` rom-device overlay pattern (lesson L12)"*):
`memory_region_init_rom_device_nomigrate` (**[src]** `v10.2.0 include/system/memory.h:1629`,
*"Writes are handled via callbacks"*) as a higher-priority subregion over the write-trap span,
added with `memory_region_add_subregion_overlap` (**[src]** `:2433`).

> **★ The constraint that comes with it, stated because it changes what `HostRegion` means.**
> There is **no `_ptr` variant** of `memory_region_init_rom_device*` at v10.2.0 — QEMU allocates
> that RAM, we do not. So a read-native overlay's backing is **QEMU-owned**, reachable through
> `memory_region_get_ram_ptr`, and it is *not* inside our reservation and *not* `RAM_PREALLOC`.
> This is acceptable because the class is small and static (faked registers, class iv-a), and
> because nothing is ever `MAP_FIXED` into it. It must **not** be used for the data plane.
> Classified `Device` in the `GuestRamMap` (§5.3), so the core can never `gpa_write` through it.

**The memslot-frequency gate holds unchanged**: installs are `O(procs)`, placements are
`O(publishes)`, and the ratio is asserted by the existing instrument.

### 5.5 ★ The R5 obligation QEMU adds

`kayfabe_vmm_kvm`'s R5 token is `Window::generation` versus `Installer::generation`. QEMU adds a
second invalidator that KVM-direct does not have: **a `MemoryListener` callback can retire a
cached foreign pointer between our plan and our commit**, on another thread, BQL-held. The
adapter therefore versions the *map* as well as the *window*: `region_add`/`region_del` bump a
map generation, and any commit that resolved through a foreign region re-validates it.

**A named residual, `[open]`.** `qemu_ram_remap` (**[src]** `v10.2.0 system/physmem.c:2684-2685`)
skips `RAM_PREALLOC` blocks — our window — but **machine RAM is not PREALLOC**, so a
memory-error recovery can remap it under a pointer we cached, and it emits no
`region_del`/`region_add`. **The experiment that settles it:** inject a hwpoison on a guest RAM
page we have cached and observe whether any listener callback fires. If none does, the mitigation
is a `RAMBlockNotifier` (`include/system/ramblock.h`) or refusing to cache pointers into
non-PREALLOC blocks and re-resolving per access. This is an MCE-only path; it is recorded, not
solved.

---

## 6. ★★ The doorbell — the options, and the recommendation

### 6.1 The premise that has changed, and it is decisive

`l1_os_shell.md` §10 task 4 and `qemu_bql_spike.md` §6 both rest on one fact about the **C**:

> **[src]** `C: nvkvm_gpu_emul.c:8644` — `nvkvm_m2_exec_doorbell(s)` takes **neither offset nor
> value**; the written value feeds only an off-by-default debug log, and the handler re-derives
> which channel advanced by polling **every** channel's `GP_PUT` against its shadow.

That is why an ioeventfd without datamatch was acceptable there: the C discards the token. The
framing of this milestone inherits that and asks whether to import the O(live channels) scan.

**Our core is not the C.** **[src, our tree]**:

- `kayfabe_rt::SharedDevice::doorbell(target_gpu, token, working_set)` — the token is a
  **parameter**.
- `kayfabe_fwd::route_doorbell` — `spine.arch().decode_doorbell(token)` → `target.vchid` → a
  `by_vchid` lookup; a token that does not decode is `FwdFault::MalformedToken`.
- `kayfabe_arch::Arch::decode_doorbell(token) -> Option<DoorbellTarget>`, and
  `kayfabe_arch::DoorbellTarget` carries `vchid`.
- `kayfabe_vmm`'s own crate docs, first paragraph: *"doorbell demux is vChid-keyed (GPU-side
  identity)"* — experiment E0.

> ### ★★★ The written value **is** the routing key. There is no rescan to import, because there
> is no rescan.
>
> The open decision was framed as *"should we inherit the C's O(live channels) `GP_PUT` scan?"*.
> The answer is that inheriting it would mean **building it** — a mechanism the core does not
> have, does not want, and whose whole purpose would be to reconstruct information the guest
> already handed us and QEMU would have thrown away. `l1_concurrency.md`'s F1 wake-count gate
> exists to forbid exactly an O(live-things) sweep per wake.

### 6.2 The options

| | mechanism | verdict |
|---|---|---|
| **Q-D1** | **Regular trap** on the lockless-IO BAR0 doorbell page; the write callback calls `SharedDevice::doorbell(gpu, val, ws)` on the vCPU thread | ★ **RECOMMENDED** |
| **Q-D2** | `memory_region_add_eventfd`, **no datamatch**, + a rescan to recover the token | **REJECTED** — §6.3 |
| **Q-D3** | `memory_region_add_eventfd` **with datamatch**, one `EventNotifier` per live token | **DEFERRED**, with a named experiment — §6.4 |
| **Q-D4** | no-datamatch eventfd + reading the token back from a shadow register | **REJECTED** — two rings of *different* tokens between two wakes are indistinguishable; it is Q-D2 with a race added |

### 6.3 Why Q-D2 is rejected — and the spike's own rule is the reason

`qemu_bql_spike.md` §6 states the condition under which its coalescing argument holds, and then
states its own limit:

> *"The handler is **level-triggered over `[gp_get, GP_PUT)` and idempotent** … a merged wake
> loses nothing. **This is a property of this handler, not of ioeventfd.** An edge-triggered or
> value-consuming handler behind an ioeventfd would be a bug."*

**Our handler is value-consuming.** `route_doorbell` consumes the token. So the spike's own
sentence disqualifies Q-D2 for our core, and no new argument is needed. Beyond that:

- an eventfd is a **counter**: two writes of *different* tokens can coalesce into one wake with
  no way to recover either;
- recovering them would require the O(live channels) sweep — F1's testable form is *"the loop
  wakes more than the signals it was given"*, and a per-wake sweep is the unbounded-poll shape
  that gate exists to catch.

### 6.4 Why Q-D3 is deferred rather than rejected, and what would settle it

Datamatch preserves the token exactly — one fd per exact `(addr, size, data)` triple — so it does
not have Q-D2's defect. It is deferred for four reasons, in descending order:

1. **Registration is a BQL memory transaction.** **[src]** `v10.2.0 system/memory.c:2582-2616` →
   `memory_region_transaction_commit`'s `assert(bql_locked())` at `:1148`. A per-token
   registration puts a BQL-taking topology transaction on the **channel-allocation** path, at
   channel-lifecycle frequency — and §4.3's whole discipline is that the adapter takes the BQL in
   no context but realize.
2. **The registration count is `O(live tokens)`**, which is the same order the rescan was
   rejected for, moved from per-wake to per-lifecycle. Better, but not free, and it needs a cap
   and a refusal path (`l1_os_shell.md` §3.8's source-cap shape) that does not exist yet.
3. **★ Its measured advantage was measured against the wrong baseline.** `qemu_bql_spike.md` §6's
   50 µs → **29 µs** and 79 k → **143 k** ops were taken on **stock 9.2 with BQL-held dispatch**.
   The floor decision deletes the term that gap is mostly made of. The comparison that matters —
   *ioeventfd vs a **lockless** trap* — **has never been run**, and the spike's §9 item 2 already
   says its numbers *"do not establish an end-to-end speedup"*.
4. **★ It buys nothing our design wants, and the place it would deliver to is the one place the
   work must not run.** Its remaining benefit is getting the *service* off the vCPU. Law L8 says
   the service **should** be on the calling thread: that is what makes backpressure
   self-limiting and bills the cost to the process that caused it.

   `qemu_bql_spike.md` §7's caveat — *"ioeventfd frees the vCPU, not the SERVICE"* — was
   narrowed by `qemu_102_facilities.md` §3 (the read side is ours, so the wake can arrive on our
   reactor instead of the main loop). **That narrowing does not make the caveat vacuous**, and
   §4.2's box is the argument: our reactor is one thread for every source in the device and is
   forbidden from touching core state, so it must hand off; and the executor it hands off to is
   **serialized**, so a verb round trip there parks every other proc's reclamation. The
   amplification is not removed, it is **relocated from a lock we do not own into a runtime we
   do** — which is worse, because it would be ours.

   So Q-D3's benefit reduces to: move the block off the vCPU and onto a thread that may not
   block. That is not an improvement; it is I-NOAMP pointing the other way, with the cost taken
   off the process that incurred it.

> **The experiment that would reopen Q-D3**, cheap and adapter-local (no port change, no core
> change): the `qemu_bql_spike.md` §5 A/B harness on a **stock ≥ 10.2** build, with both arms
> **lockless**, doorbell p50/p99 and throughput for (a) trap and (b) datamatch ioeventfd, at
> realistic doorbell rates from a Mode-2 workload. Quote the *ratio*, and only if it is large.

### 6.5 Q-D1 in detail

The doorbell page is one `nvkvm_region_init_io` region inside BAR0. The write callback:

```
mmio_write(bar=Bar0, off=DOORBELL, size=4|8, val=token)
  → QemuVmm handle for this vCPU thread
  → Device::mmio_write  (kayfabe_rt::SharedDevice, &self)
  → SharedDevice::doorbell(target_gpu, token, working_set)
       plan   : rank 0 route → rank 1 plan  (the #14 ring-gate runs here)
       execute: locks dropped, isolate round trip on THIS thread
       commit : re-locked, R5 re-resolve
```

The vCPU blocks for the round trip and no other vCPU is affected — §4.4. Reads of the doorbell
page return `u64::MAX`, following `kayfabe_vmm_kvm`'s rule that all-ones is the universal
*"nothing answered"* and zero is a plausible register value.

**Write it idempotent-over-outstanding anyway.** Even though Q-D1 has no coalescing, the handler
should be level-triggered over the channel's outstanding range rather than assuming one wake per
ring. That property costs nothing to have and is the only thing that would make Q-D3 adoptable
later; writing the handler edge-consuming would foreclose the option silently.

---

## 7. Interrupts

`raise_irq` is in-lock **legal** (§6.1) and is the single named exception, so it must be a
descriptor write and never a BQL-taking `msix_notify()`.

**Realize (BQL-held):**
`msix_init_exclusive_bar` (**[src]** `v10.2.0 include/hw/pci/msix.h:15`) → `msix_vector_use`
(`:37`) → `event_notifier_init` (**[src]** `v10.2.0 include/qemu/event_notifier.h:33`) →
`kvm_irqchip_begin_route_changes` / `kvm_irqchip_add_msi_route(c, vector, dev)` (**[src]**
`v10.2.0 include/system/kvm.h:483,474`) → `kvm_irqchip_commit_route_changes` →
`kvm_irqchip_add_irqfd_notifier_gsi(s, n, NULL, virq)` (`:498`).

Mask/unmask and MSI-X reprogramming are tracked with `msix_set_vector_notifiers` (**[src]**
`v10.2.0 include/hw/pci/msix.h:46`), whose callbacks arrive BQL-held and are latch-and-defer like
every other such callback.

**Hot path:** `event_notifier_set(&n)` — **[src]** `v10.2.0 util/event_notifier-posix.c` is one
`write(2)`. That is the whole of `Vmm::raise_irq`, and it is the same shape as
`kayfabe_vmm_kvm`'s `Notifier::signal_under_lock`, which **declares the ranks it permits in its
own signature** rather than asserting lock-freedom. Reuse that type, do not re-invent it.

**`IrqSpec::IntxLevel`** returns `VmmError::Unsupported`. The core never emits it
(`kayfabe_vmm::IrqSpec` rustdoc), and a backend-conditional variant is a contract, not a bug.

---

## 8. Lifecycle — realize / reset / unrealize onto T0–T7

### 8.1 realize

BQL-held, and therefore the one place the whole coarse tier happens (§4.3), in this order:

1. `#error`-checked floor (§3.5) already passed at build; assert the runtime version.
2. Refuse unless `kvm_enabled()` (§3.4).
3. **`migrate_add_blocker(&reason, errp)`** — §8.4. Before anything is mapped.
4. **`ram_block_discard_disable(true)`** — §8.5.
5. Reserve the window (`kayfabe_linux_raw::Reservation`), `memory_region_init_ram_ptr`.
6. Create BAR regions through `nvkvm_region_init_io` (lockless, every one), `pci_register_bar`.
7. MSI-X + irqfd (§7).
8. `memory_listener_register` against `pci_get_address_space(dev)` (§5.2).
9. Refuse **loudly at realize**, never at first guest DMA, if guest RAM was not launched with a
   shareable backing — `kayfabe_vmm::Vmm::export_ram`'s stated precondition.
10. Start the reactor and executor threads.

**Every failure arm unwinds what it created before recording it anywhere** — the shape
`kayfabe_vmm_kvm::KvmMachine::install_window` uses, and the property
`memory_plane.rs::a_repeated_partial_failure_returns_the_host_address_space` measures on the host
address space rather than trusting a ledger that only increments on success.

### 8.2 reset → **T4**, latch-and-defer

`ResettableClass` three-phase (**[src]** `v10.2.0 include/hw/resettable.h:111-120`,
`resettable_class_set_parent_phases` at `:232`). **[src]** `:50`: *"This whole API must only be
used when holding the iothread mutex."*

So T4 arrives BQL-held, and the flag does not help (§3.3 point 2). Therefore:

- **`enter`**: latch a reset epoch. Nothing else. Upstream's own contract for this phase is that
  it *"must not do anything that has a side-effect on other objects"* (**[src]** `:80-83`).
- **`hold`**: stop accepting traps (T7 step 1's mechanism, reused), wake the executor.
- **`exit`**: nothing. The reclamation is the executor's, per §6.6's *"background work with no
  caller to bill it to runs on the executor"*.

`l1_os_shell.md` §7.6 T4's two flavours and its five C-derived rules are unchanged by this
document; only the **thread** T4's entry runs on is pinned, and the eight-trigger property (§7.7)
is untouched.

### 8.3 unrealize → **T7**

`l1_os_shell.md` §7.6 T7's nine ordered steps run in `unrealize`, which is BQL-held. Two
QEMU-specific notes:

- Step 7 **joins** the reactor and executor threads. That join happens with the BQL held, which
  is legal (we hold no lock of ours) but means a wedged worker stalls the machine. §7.5's bounded
  escalation and abandon-with-condemnation escape is what keeps it bounded, and the bound must be
  asserted here rather than assumed.
- Step 8 unmaps the window. **QEMU frees nothing** — `reclaim_ramblock` skips `RAM_PREALLOC`
  (**[src]** `v10.2.0 system/physmem.c:2591`) — so this step is not optional and there is no
  VMM-side backstop for it.

**`Drop` stays a tripwire, not a teardown**, exactly as §7.6 specifies.

### 8.4 The ninth event: migration and CPR

**[src]** `v10.2.0 include/migration/blocker.h:32` — `int migrate_add_blocker(Error **reasonp,
Error **errp)`, documented at `:20` as *"prevent **all** modes of migration from proceeding"*.
Paired with `migrate_del_blocker` (`:61`) at unrealize.

**Why the blocker and not `VMStateDescription.unmigratable`:** the blocker names *our* reason at
the moment migration is attempted, and it is greppable as a deliberate act.

**Why it is mandatory rather than tidy.** **[src]** `v10.2.0 system/physmem.c:2498` — the
CPR/shared-fd branch is `!host`-gated, and our window has `host != NULL`. So `cpr-transfer` would
**silently not preserve our window** rather than fail: a new QEMU binary, our isolates' parent
gone, our `mm` gone, and a device that appears to have survived. Silent is the worse mode.

It also closes, for free, the uffd-composition hazard `qemu_102_facilities.md` §6 names (a VMA
range can carry one userfaultfd registration, and QEMU registers for postcopy) — even though
law L6 means we are not registering one today.

### 8.5 ★ CLOSING an `[open]` — the balloon can discard our window, and the fix is one line

`qemu_102_facilities.md` §10 left this open, saying the balloon /
`qemu_ram_block_from_host` path had not been read. It has now been read.

**[src]** `v10.2.0 hw/virtio/virtio-balloon.c:425-446` — `virtio_balloon_handle_output` does
`memory_region_find(get_system_memory(), pa, 1)` and accepts any section for which
`memory_region_is_ram(section.mr) && !memory_region_is_rom(...) && !memory_region_is_romd(...)`,
then calls `balloon_inflate_page(s, section.mr, section.offset_within_region)`.

**[src]** `:79-97` — that resolves the host pointer with `memory_region_get_ram_ptr(mr)`, finds
the `RAMBlock` with `qemu_ram_block_from_host`, and calls `ram_block_discard_range(rb, …)`.

**[src]** `v10.2.0 system/physmem.c:4094-4119` — `ram_block_discard_range` gates on alignment,
`rb->fd` and page size. **It has no `RAM_PREALLOC` check.**

Our window is created by `memory_region_init_ram_ptr`, so `mr->ram` is true, it is not a ROM and
not ROMD. **[inferred]** ⇒ **a guest that hands the balloon a GPA inside our window reaches
`ram_block_discard_range` on our own `RAMBlock`.** That is a `madvise`-shaped zeroing of backing
underneath live `MAP_FIXED` placements, at an arbitrary later time — the silent-data-loss shape
`qemu_102_facilities.md` §10 predicted.

**[src]** `v10.2.0 hw/virtio/virtio-balloon.c:75` — the balloon consults
`ram_block_discard_is_disabled()`.

> ### ★ DECISION Q4 — `ram_block_discard_disable(true)` at realize, exactly as vfio does
>
> **[src]** `v10.2.0 include/system/memory.h:3290`. One call, checked for `-EBUSY` (it fails if a
> discard *requirer* such as virtio-mem is already present, in which case realize must refuse and
> name the conflict rather than proceed).
>
> **This is now a source read, not an experiment.** The bench experiment §10 proposed —
> *"balloon a page inside our window and see"* — is downgraded from *"settles the question"* to
> *"confirms the fix"*, and belongs in stage **Q3** as a negative test.

---

## 9. ★★ What is NOT built, and why

Every milestone in this project states its omissions. An honest absence is a design statement.

**9.1 No userfaultfd, no `mprotect`, no read-only regions on the data path.** Law L6.
`guest_memory_lock.md`'s decision #49 (uffd-WP) is not implemented by this adapter, and
`CoreEvent::LockedRegionFault` has no producer here. It stays in the port because the *delivery*
half is genuinely cross-seam; nothing on QEMU emits it today.

**9.2 No `IOThread`, no `AioContext`, no bottom half, no `io_uring`.** §4.5, and B7 unchanged —
`aio_add_sqe`/`aio_has_io_uring` are new at 10.2 (**[src]** `v10.2.0 include/block/aio.h:870`,
`:846`) and live inside `AioContext`, which we do not use.

**9.3 No use of the in-tree Rust workspace.** `qemu_102_facilities.md` §4's three disqualifiers,
re-checked: it is in-tree only (a fork by another name), its `BqlCell`/`BqlRefCell` idiom's
soundness argument *is* the BQL and our device runs with none, and `v10.2.0 rust/hw` has no
`pci`.

**9.4 No `VMStateDescription`, no `msix_save`/`msix_load`, no migration support of any kind.**
§8.4 blocks migration outright; writing serialisation we then refuse to use would be a maintained
lie.

**9.5 No display / `Present` implementation.** `kayfabe_vmm::Present` stays mocked. The
QEMU/PRIME sink is a separate concrete impl (`execution_plane.md` §2.6) and pulling it into L2-Q
would put a dma-buf export path in the same diff as the first BQL contact — the misattribution
shape M2-c's "one variable, not two" box exists to prevent.

**9.6 ★ No vfio-user server — and this is the closest thing to a road not taken.**

**[src]** `v10.2.0` ships `hw/vfio-user/` (Kconfig, container.c, device.c, pci.c, protocol.h,
proxy.c) with `docs/interop/vfio-user.rst`, `docs/system/devices/vfio-user.rst` and
`subprojects/libvfio-user`; **[src]** `v10.2.0 MAINTAINERS:4374-4384` lists VFIO-USER as
`S: Supported`.

A vfio-user *server* would make our device a separate process speaking a documented socket
protocol to a **stock, unmodified, distro-packaged QEMU**. That deletes §2.1 entirely — no QEMU
tree, no overlay, no build instruction — and it deletes §2.2's second unsafe crate too, since the
protocol is a socket protocol and guest RAM arrives as fds we `mmap` ourselves with the
`GuestWindow` machinery that already exists. It would also delete the entire foreign-lock class
from our process, which has no BQL.

**Why it is not the v1, on one source read and one honest limit:**

- **[src]** `v10.2.0 hw/vfio-user/pci.c:289` sets `vbasedev->io_ops = &vfio_user_device_io_ops_sock`
  and otherwise reuses the generic `hw/vfio` region ops; **nothing in `hw/vfio-user` calls
  `memory_region_enable_lockless_io`**. So a trapped BAR access becomes a **socket round trip
  taken with the BQL held** — reintroducing, and amplifying, exactly the I-NOAMP violation
  `qemu_bql_spike.md` §5 measured at 5.3× and the floor decision was taken to remove.
- It is a different architecture, not a different adapter: the `Vmm` port survives, but the
  device model, the trap path and the memory plane are all re-derived.

> **The experiment that would make it a real contender**, and it is small: measure a trapped BAR
> read/write round trip through `vfio-user-pci` against the same access through an in-tree
> lockless device, and check whether marking the client's regions lockless is a patch upstream
> would take. If the answer to the second question is yes, **the packaging cost of §2.1
> disappears and this becomes the better design.** It is recorded now because it will not be
> obvious later.

**9.7 No `MemoryListener` beyond RAM classification.** We do not observe a guest re-programming
our own BARs (`qemu_102_facilities.md` §13 item 8). The BAR base is QEMU's business and our
offsets are BAR-relative; if that assumption is ever wrong it is a `region_add` we are already
receiving and ignoring, which is a cheap place to add the check later.

**9.8 No performance work.** Not one number in this design is a target. The acceptance
measurement of §11 is a *correctness* gate about where a block lands, not a speed gate.

---

## 10. The staged build order

Eight stages. **Seven need no GPU. Two need no machine at all. Two more need QEMU but no guest
OS.** `kayfabe-vmm-kvm` was built and tested with no GPU whatsoever, and this ordering keeps that
property for as long as it can be kept honestly — which, with the bench offline at the provider,
is the difference between a milestone that proceeds and one that waits.

| stage | what | QEMU? | **guest OS?** | **GPU?** | independently testable by |
|---|---|---|---|---|---|
| **Q0** | **The gate amendment and the empty crates.** `kayfabe-vmm-qemu` + `kayfabe-qemu-raw` in the workspace; unsafe gates A/B/ratchet grow to a named two-crate list with per-crate counts (§2.2); the three vocabulary gates' hard-coded lists reviewed and left unchanged **deliberately**, with a test asserting the adapter crates are out of their scope by design. | no | no | **no** | CI alone. `KAYFABE_NO_KVM=1 cargo test --workspace` green before **and** after — Q0 must not be the commit that makes the OS-free configuration need an OS. |
| **Q1** | **★ The pure half — the biggest stage, and it needs nothing.** All of `impl Vmm` against a `QemuHost` trait with a mock: the `GuestRamMap` and its §5.3 classification, the two leaf locks with `leafwitness`, plan/execute/commit, the window/map generations, retirement (§0 item 1). Ported from `kayfabe_vmm_kvm::Plane` shape-for-shape. | no | no | **no** | Unit + the existing `rt_shell` / `l1_mean` suites re-run against the mock-hosted `QemuVmm`, **in both lock modes**. Zero OS, zero hypervisor. The differential that matters: the KVM and QEMU backends must produce **identical** operation logs for the same core run (`testing_doctrine.md` §7's two-mode discipline, one axis over). |
| **Q2** | **The shim and the raw crate: realize and its refusals.** The QOM device, BARs, `nvkvm_regions[]` + `nvkvm_bars_realize` + the coverage self-check, both floor assertions, the `kvm_enabled()` refusal, `migrate_add_blocker`, `ram_block_discard_disable`, MSI-X + irqfd. | **yes** | **no** — `-S` + QMP, no OS image | no | ★ Every acceptance here is a **realize-time refusal or a build failure**, so none of it needs a guest: a 10.1 tree fails to **compile**; a TCG invocation refuses at realize; `migrate` refuses **naming our reason**; `cpr-transfer` refuses; a `virtio-mem` present makes `ram_block_discard_disable` return `-EBUSY` and realize refuse naming the conflict; a deliberately bypassed region row makes the §3.3 self-check fail. Plus `info qtree` / `info mtree` over QMP showing the region table. |
| **Q3** | **The listener and the GPA plane.** `memory_listener_register`, the §5.3 classification, cached pointers + refcounts, `gpa_read`/`gpa_write` end to end. | **yes** | **no** — a 200-line freestanding blob is enough | no | The blob writes a pattern into ordinary RAM and the core reads it back via `gpa_read`. A GPA aimed at another emulated device's BAR returns **`NonRamGpa`**; a hole returns **`BadGpa`**; the two are asserted **apart**, because they are near neighbours. A straddling range reports the **boundary** byte, not its own start. A `ram_device` region (add a `vfio`-shaped stub or a second device with `memory_region_init_ram_device_ptr`) and a `rom_device` region are **both** classified `Device` — §5.3's three directions each get a positive case, or the classification is untested in the direction that matters. The balloon negative test (§8.5): `virtio-balloon` present, inflate a page inside our window, assert the page survives. |
| **Q4** | **★ Trap dispatch, the doorbell, and the acceptance measurement.** `Device::mmio_read/write` wired; Q-D1; the F1 wake-count assert; the read-native rom-device overlay. | **yes** | **no** — the synthetic driver is the guest | no | A freestanding guest in the shape of `tests/src/guest.rs::DoorbellDevice` rings a token that must survive `decode_doorbell` → `by_vchid` → **the right `Proc`**; a **malformed** token must produce `MalformedToken`, never a mis-route; two procs' tokens must not cross. `bql_locked() == 0` asserted inside **every** trapped region's handler. **And the `qemu_bql_spike.md` §5 A/B shape re-run against OUR device on a stock ≥ 10.2 build, both arms** — the thing the spike explicitly did not establish and the only reason this stage is a gate rather than a step. |
| **Q5** | **Lifecycle.** The three reset phases as latch-and-defer; `unrealize` = T7's nine steps; the `Drop` tripwire; the bounded join. | **yes** | **no** — the Q4 synthetic guest, driven | no | `system_reset` **under load**: the T4 canary (a verb in flight across a reset never commits into the new life); the conservation ledger balances after every trigger in every interleaving; the `KillPoint` × `Trigger` matrix; the tripwire never fires; the join's bound is **asserted**, not assumed. |
| **Q6** | **★ Hardware bring-up and the BAR1 named unknown.** The real NVIDIA guest driver, real forwarding, and `qemu_bql_spike.md` §8's unmeasured row: data-carrying aperture writes at realistic rates, B's p99 with A blocking, both arms. | **yes** | **yes** | **★ YES — the only stage that does** | The bench, serialized, fresh boot per run. |
| **Q7** | **The exit gate.** §4.2's table finalised as *the* review artifact; the security review of `kayfabe-qemu-raw`'s FFI surface and its second unsafe ratchet; the R1/R3/R5 re-audit **written down** (§10 task 3) rather than asserted; §7.9's QEMU-conditional rows converted from *expected* to *observed*. | — | no | no | Signed off **as part of this milestone**, not as later cleanup. |

> **★ Why "no guest OS" is load-bearing and not a convenience.** Q2–Q5 are the stages that meet
> the foreign lock, and every one of their acceptances is either a refusal, a counter, or a
> latency distribution — none of which needs a distribution image, a driver, or the pinned guest
> kernel. A booted Ubuntu guest in those stages would add a second variable to every failure and
> would make the suite un-runnable on any box without the overlay. The rule the whole ordering
> follows: **a guest is introduced at the first stage that genuinely needs a driver, and that is
> Q6.**

**The ordering argument.** Q1 is deliberately enormous and deliberately OS-free, for the same
reason M2-c built against the KVM harness and not QEMU: **one variable, not two.** If the memory
plane's ownership and R5 discipline are wrong, that must be discovered against a mock, where a
failure is attributable — not in the same diff as first contact with a foreign lock. Q2 is the
first line of C. Q3 is the first guest instruction. Q6 is the first GPU **and** the first guest
OS. **Q0 and Q1 can be built and gated today, on this machine, with the bench offline** — and
between them they contain the entire `Vmm` implementation.

---

## 11. The gates it must satisfy

**Inherited, must keep passing unchanged:**

- All 12 steps of the `stable` job, plus `aarch64`'s `cargo check --workspace`, plus `mutants`
  (whose scope is `crates/*/src/**/*.rs`, so both new crates are mutated automatically and the
  91 % threshold — already flagged *pending re-derivation* — must be re-derived, not lowered).
- **`KAYFABE_NO_KVM=1 cargo test --workspace` green.** This is the property that stops
  Linux/KVM/QEMU semantics leaking into the core, and it is why Q0 and Q1 are OS-free: the
  adapter must never become a *reason* for a test to need a machine. The KVM-gate reached-count
  floor (currently 35) may only rise.
- **The VMM-vocabulary gate.** Its `portable=` list is 11 pure crates plus `kayfabe-rt`, and
  adapter crates are **out of scope by design** — the gate matches API identifiers and
  deliberately never matches the vendor's name, *"so naming the adapter crates stays writable and
  no allowlist is ever needed"*. So `kayfabe-vmm-qemu` and `kayfabe-qemu-raw` may say
  `MemoryRegion` freely, and **no crate in the gated list may gain a QEMU identifier because of
  this milestone.** Q0 asserts that as a test rather than trusting it.
- The hexagonal boundary gate, the generation-name gate, the GPA-accessor gate (exactly one
  classification site, in `kayfabe-fwd`), the memslot-frequency gate, the conservation ledger,
  the kill-point matrix, the reactor wake-count assert, TSan, and the aarch64 cross-check.

**Amended by this milestone (Q0):** unsafe gates A, B and the ratchet, from a one-crate constant
to a named two-crate list with per-crate expected counts (§2.2).

**New, L2-specific:**

| gate | where | fails when |
|---|---|---|
| **lockless-IO (a) + (b)** (§9.3's re-specified row) | push, grep | (a) more than one call site for `memory_region_enable_lockless_io`, or it is not in `nvkvm_region_init_io`; (b) any occurrence tree-wide of `disable_reentrancy_guard`, `bql_unlock`, or a hand-rolled unlock/lock around a dispatch |
| **★ lockless-IO (c) — COVERAGE** (§3.3) | push, **and realize** | a grep **cannot** carry this clause: a missed region is an omission and an omission has no token. Push-side: `memory_region_init_io` or `pci_register_bar`/`memory_region_add_subregion*` appears outside `nvkvm_region_init_io` / `nvkvm_bars_realize`. Realize-side: the self-check walks `nvkvm_regions[]` and fails if any `IO` row has `mr->lockless_io == false`, or if the IO-row count and the constructor-call count disagree. **Negative-tested by adding a row that bypasses the constructor and asserting realize refuses.** |
| **zero BQL acquisition sites** (§4.3) | push | `bql_lock` / `BQL_LOCK_GUARD` / `qemu_mutex_lock_iothread` appears anywhere in the shim or either Rust crate |
| **floor, compile-time** | build | the shim compiles against `QEMU_VERSION_MAJOR/MINOR < 10.2` |
| **floor, realize-time** | runtime | the binary is < 10.2, or `!kvm_enabled()`. Negative-tested both ways |
| **migration + CPR refused** | runtime | `migrate_add_blocker` is not called at realize; a `migrate` or `cpr-transfer` attempt is not refused with our reason |
| **discard refused** (§8.5) | runtime | `ram_block_discard_disable(true)` is not called, or its `-EBUSY` arm is unhandled |
| **prove-RAM classification** (§5.3) | push + runtime | a listener path admits a `ram_device`, a `rom_device`, or a readonly section as `Ram`; or `address_space_rw`/`_read`/`_write` appears anywhere in the adapter |
| **acceptance, against our device** | Q4 | A's block duration appears in an unrelated vCPU's p99 on a stock ≥ 10.2 build. Both arms recorded |

---

## 12. ★ Where this design contradicts the prepared ground

Stated separately because these are the parts most likely to be read past.

1. **★★ The doorbell's premise was inverted.** `l1_os_shell.md` §10 task 4,
   `qemu_bql_spike.md` §6 and this milestone's framing all describe the open decision as *"do we
   import the C's O(live channels) `GP_PUT` scan?"*. **Our core takes the token as a parameter**
   (`kayfabe_rt::SharedDevice::doorbell`, `kayfabe_fwd::route_doorbell`,
   `kayfabe_arch::Arch::decode_doorbell`), so there is no scan to import and a no-datamatch
   ioeventfd is disqualified by `qemu_bql_spike.md` §6's own closing sentence. **Proposed
   amendment:** §10 task 4 should read *"the doorbell is a regular trap; ioeventfd-with-datamatch
   is a measurable later option"*, and `qemu_bql_spike.md` §6's *"the token question, resolved
   favourably"* should carry a note that it was resolved favourably **for the C**.
2. **★★ The floor does not deliver "stock QEMU".** §2.1: there is no out-of-tree device path at
   v10.2.0, so the shim lives in a QEMU tree. The floor decision's *"deletes the whole 'which
   QEMU are we built into' ambiguity"* is **half true** — it deletes the *behavioural* ambiguity
   and keeps the *build* one. This is a real residual (4) on that decision box.
3. **★★ The adapter cannot contain `unsafe`, and adding a second unsafe crate is a design
   decision the gate demands be made explicitly.** §2.2. Nothing in §10 or §6 anticipates this,
   and it must land before any adapter code.
4. **★ `memory_region_is_ram()` is not the RAM test, and no complete accessor exists at
   v10.2.0.** §5.3. `qemu_102_facilities.md` §11.3 correction 2 identified the `ram_device`
   direction; the `rom_device` and `readonly` directions are new, and the third one has no public
   predicate.
5. **★ The balloon `[open]` is closed, against us.** §8.5 — `qemu_102_facilities.md` §10's open
   question is answered by reading `hw/virtio/virtio-balloon.c:425-446`: the path **is**
   reachable, and the one-line fix it predicted is required rather than optional.
6. **`qemu_bql_spike.md` §4's reentrancy claim is over-general.** **[src]** `v10.2.0
   system/memory.c:545-546` skips the guard for `ram || ram_device || rom_device || readonly`, so
   *"any two vCPUs touching any two regions of the same device collide"* was never true of
   RAM-backed BARs. `qemu_102_facilities.md` §2 point 3 already says this; it is repeated because
   §6.3.1's wording still carries the general form.
7. **`map_read_native` has no `_ptr` spelling on QEMU.** §5.4 — the read-native backing is
   QEMU-allocated, which changes what a `HostRegion` means for that one method. Neither §6.7 nor
   the port's rustdoc anticipates it.
8. **`memory_region_clear_global_locking()` remains the standing lesson, and it held.** Every
   QEMU symbol named in this document was fetched from `v10.2.0` and grepped, not inherited. Two
   inherited citations moved by a few lines (the reentrancy guard is at `:545-555` at this tag,
   not `:551-561` as recorded against 9.2; `prepare_mmio_access` is at `:3196`), and **no
   inherited symbol was found to be absent.** ⇒ **Proposed amendment:** `l1_os_shell.md:64`'s
   blanket `[unverified]` stamp should gain an exception naming this file and
   `qemu_102_facilities.md`, listing the 25 files read at the tag (§0.2); the CH half of §6.3
   keeps the stamp, because no cloud-hypervisor checkout was involved here either.
9. **★★ `qemu_102_facilities.md` §10.1 item 2's last sentence is wrong: the hand-off rule must
   NOT be deleted.** It infers from *"the ioeventfd's read side is ours"* that L2-Q task 4's
   hand-off clause and §6.6 item 2's compensating rule can both go. The premise is verified and
   the conclusion does not follow — **our reactor is one thread for every source and may not
   touch core state, and the executor behind it is serialized.** What the finding actually
   deletes is the *justification* (*"because it runs on the main loop under the BQL"*), not the
   rule. §4.2's box restates the rule with a justification that survives on a backend with no
   global lock at all. **This became the third independent argument against Q-D3.**
10. **★ L2-Q task 2 and decision #35's "PACKAGE DEAL" are stale.** Upstream's one function does
    both, so there is no pairing to maintain and clause (b) of the re-specified gate is a grep
    for something we must never write. **The live obligation is clause (c) — coverage — and a
    grep structurally cannot check it, because a missed region is an omission and an omission
    has no token to match.** §3.3 discharges it with one table, one constructor, one
    registration loop and a realize-time self-check over `mr->lockless_io`.
11. **★ Five sites still describe a carried backport.** `l1_os_shell.md:1299`, `:2879`, `:3211`
    (*"≥ 10.2.0, or our patched 9.2.0"*), decision #35 and decision #48, plus the present-tense
    reading of §6.3.1 and §14.6. Nothing here designs against them; recorded so the next reader
    of §10 does not.

**Two things in the framing of this task that turned out to be right and are worth confirming
explicitly**, since the rest of this section is corrections: the "one change, not two" rule for
lockless IO is *exactly* upstream's shape (the function body is quoted in §3.3), and the
per-region/per-device asymmetry is real and is the sharpest practical consequence in the whole
inventory — an unmarked BAR2 keeps both hazards while passing any per-device check.

---

## 12a. ★★★ BUILT, 2026-07-30 (stage Q2) — and what building it settled in §12

The C QOM shim exists. It is `qemu/hw/misc/nvkvm/` (the device, the compatibility header and
the wire header) plus `crates/kayfabe-qemu-raw`, and it was **compiled into two real
hypervisor binaries and run**: 9.2.0 and 10.2.4, the same overlay unchanged, on a machine with
a real accelerator. `scripts/build_qom_shim.sh` is the whole install story, and its length is
the honest measure of §2.1's unpaid cost.

**What was observed, not inferred.** On 10.2.4 the device registers, realizes, survives
firmware's base-address assignment, realizes the memory plane, and installs a **real
accelerator memslot into the hypervisor's own machine** — reported as
`kernel slots live=1 installs=1, regions the hypervisor backs=0`. That last number is §1 of
`host_execution_plane.md` as a single measured quantity. On 9.2.0 the identical binary path
runs and is **refused by name** at the runtime floor. Both were watched.

**Nine things §12 did not have, in the order they cost time:**

1. ★★★ **§3.5's two-floor argument is void, and a better one survives.** §3.5 justifies two
   floors by *"the build-time check is a claim about the headers, this is a claim about the
   binary"*. §2.1 of this same document proves those cannot differ here: there is no
   out-of-tree device mechanism, so the shim is compiled **inside** the binary it runs in. The
   two floors survive because they are about different **subjects** — the compile-time one
   about *symbols* (9.2, where every function the shim names was verified present), the
   realize-time one about *semantics* (10.2, the global-lock opt-out). They are now different
   numbers, and that is what made the shim testable at all.
2. ★★★ **§2.3's seam is a TABLE OF PRIMITIVES, not linked symbols.** §2.3 says both sides call
   primitives and leaves the mechanism implicit. `extern "C"` declarations resolved at link
   time would make the raw crate unbuildable without a hypervisor — deleting §10's whole
   "Q0/Q1 need no machine" property — and would put the vendor's symbol names inside Rust.
   The table is `kayfabe_shim.h`, which names **no hypervisor type at all**.
3. ★★ **The listener must be registered AFTER realize returns, and §5.2 does not say so.**
   `memory_listener_register` replays the entire topology through `region_add` before it
   returns, and the archive calls `register_listener` from *inside* its own realize — so every
   replayed section would arrive with no handle to deliver it to, and be dropped **silently**,
   which is indistinguishable from a machine with no memory in it. The primitive records the
   request; the caller registers the moment a handle exists.
4. ★★ **§8.5's `Busy` class does not survive the port's own error translation.** `HostError::Busy`
   is a named variant *"not an errno"* by its own rustdoc, and `qemu_refused` turns it into
   `HostRefused { errno: Some(EBUSY) }`. The sentence and the number survive; the class does
   not. It is reconstructed at realize only — the one place the reconstruction is exact — and
   the imprecision is pinned by a test rather than left to be found.
5. ★★★ **§3.3's coverage clause needed a structural check it does not name, and the gap
   shipped a bug.** Every region here is a **64-bit** base-address register, and a 64-bit
   register consumes **two** hardware registers. The first build registered three of them at
   0, 1 and 2; PCI accepted it silently and the device came up reporting two registers at the
   same guest-physical base, with a reservation installed over the wrong one. §3.3's four
   structural devices — one table, one constructor, one loop, one self-check — do not catch
   it, because nothing was omitted. The table now carries the port's dense name and the
   hardware's sparse one separately, and the self-check asserts the spacing.
6. ★★ **The unsafe ratchet could not see this crate's dominant form.** Its pattern cannot
   match `unsafe extern "C" fn`, which is what an FFI entry point *is* — so it counted 23 of
   31 relaxations and reported a complete audit. Corrected; `kayfabe-linux-raw` contains no
   occurrence of the form, so its reviewed bar is unchanged, measured rather than assumed.
7. ★★★ **The Axis-A quarantine became a PREDICATE, not a third list entry** — and this is the
   correction worth reading twice, because the first attempt got it wrong in the way this
   repository gets things wrong. An FFI crate needs C layouts, and §11's inherited-gate list
   did not anticipate that. The first fix added `kayfabe-qemu-raw` as a **third exempt crate
   name**; the owner refused it, on grounds that outlive this milestone: *lengthening an
   exemption list weakens a rule with **zero red tests** and licenses a fourth entry.* The
   substance was accepted — these layouts really are ours, and flattening them to scalars is
   **less** safe, because a function-pointer table behind an untyped array turns thirteen call
   sites into transmute-by-position at the exact seam where a mistake is a memory-safety bug.
   So the gate now asks *"is this layout foreign, or own-wire?"* and **own-wire must be
   proved**: a structure of the **same name** in a repository-local header, **and** a
   `tests/wire_mirror.rs` in that crate enforcing the pair in both directions. It **fails
   closed**, so it is strictly stronger than the rule it replaces — a fourth crate needs no CI
   edit and cannot arrive without both proofs, and a logic crate cannot acquire a C layout by
   any route. Negative-tested four ways, including the sharp one: a logic crate declaring a
   type whose name *is* in our header passes proof 1 and is still refused by proof 2.
   `CLAUDE.md` rule 1 was rewritten to match, because the old text said *"live ONLY in
   `kayfabe-abi`"* and a reader finding that next to this code would either revert it or copy
   the violation.
8. ★ **The compatibility surface between two hypervisor releases is four items**, and they
   are the whole of §12's "which QEMU" worry made concrete: the include-path rename, one
   class-initialiser signature, one property-array terminator, and the opt-out itself. Two are
   feature-detected rather than versioned. Nothing in the device branches on a version, and
   nothing in Rust does.
9. ★ **§7 is untouched and says so.** The vector-raising primitive refuses by name. §3.3's
   opt-out is applied and self-checked where the facility exists, but §11's last row — the A/B
   acceptance measurement against *our* device — was not run.

---

## 13. Decision ledger

| # | Decision | §  |
|---|---|---|
| **Q1** | The shim ships as an **additive overlay** in a QEMU 10.2.x tree — two hunks, no upstream semantics changed — and the build instruction is a named, unpaid cost. | 2.1 |
| **Q2** | **Three crates**: a C QOM shim, `kayfabe-qemu-raw` as the **second** audited unsafe crate, and `kayfabe-vmm-qemu` under `forbid(unsafe_code)`. The CI gates grow to a named two-crate list. | 2.2 |
| **Q3** | **Our threads stay ours.** No bottom half, no `IOThread`; the executor and reactor are the same ones both backends already use, and the adapter takes the BQL in **no** context but realize/unrealize. | 4.5, 4.3 |
| **Q4** | `ram_block_discard_disable(true)` at realize, with its `-EBUSY` arm a realize refusal. | 8.5 |
| **Q5** | **The doorbell is a regular trap (Q-D1).** ioeventfd-without-datamatch is rejected on our core's own shape; ioeventfd-with-datamatch is deferred behind a named measurement. | 6 |
| **Q6** | **Refuse TCG at realize.** Lockless IO is KVM-only, so TCG is not a slow mode, it is the measured 5.3× I-NOAMP violation with no way to see it. | 3.4 |
| **Q7** | The `GuestRamMap` classification is **narrower than `memory_region_is_ram()`**, and the shim reads `mr->rom_device` directly because v10.2.0 exposes no complete predicate. | 5.3 |
| **Q8** | `map_read_native` is a **rom-device overlay**, with QEMU-owned backing, restricted to the small static faked-register class and never the data plane. | 5.4 |
| **Q9** | The two floor assertions (compile-time and realize-time) are **both** required; neither substitutes for the other, and a presence check for `memory_region_enable_lockless_io` is vacuous under a version floor. | 3.5 |
| **Q10** | **Lockless-IO coverage is discharged structurally, not by grep**: one region table, one constructor, one registration loop, and a realize-time self-check over `mr->lockless_io`. The pairing discipline is upstream's and is deleted; coverage is ours and is the only live obligation. | 3.3, 11 |
| **Q11** | **The eventfd hand-off rule is kept**, with a new justification: our reactor is one thread for every source and may not touch core state, and the executor behind it is serialized. `qemu_102_facilities.md` §10.1 item 2's deletion of the rule is refused. | 4.2, 6.4 |

---

## 14. What this document does **NOT** establish

1. **Nothing here was run.** No `cargo`, no QEMU build, no guest, no GPU. Every `[src]` is a read
   at `v10.2.0`; every `[measured]` is quoted from an earlier round with its arm and its caveat.
2. **It does not establish that our device passes the lockless-IO acceptance test.** The spike
   measured a throwaway device with trivial handlers. §11's row and stage Q4 still stand.
3. **The BAR1 named unknown is untouched.** `qemu_bql_spike.md` §8's experiment is unchanged and
   unrun, and it is the only reason stage Q6 exists in the shape it does.
4. **No number in §6's doorbell recommendation is a QEMU-with-lockless-IO number**, because none
   exists. The recommendation rests on our core's *shape*, not on a measurement, and §6.4 names
   the measurement that could overturn its deferral.
5. **The vfio-user assessment (§9.6) is a read of a maintainer file, a directory listing and one
   line of `pci.c`** — enough to say it is real and to say why its trap path is currently worse,
   not enough to cost it.
6. **The `qemu_ram_remap` residual (§5.5) is open**, and its experiment is named rather than run.
7. **arm64 is inherited, not checked.** `memory_region_enable_lockless_io` lives in generic
   `system/memory.c` and is honoured in generic `system/physmem.c`, so **[inferred]** it applies
   on arm64/KVM. No arm64 host was involved in this document or in the round it inherits that
   from.
8. **No claim about QEMU > 10.2.** The floor is a minimum; the floor-ratchet residual means that
   is deliberate.
9. **The staged plan's stage sizes are not estimates.** Q1 is called "the biggest stage" on the
   grounds that it contains all the logic, not on the grounds of a measurement.
