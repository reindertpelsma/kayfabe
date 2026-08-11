# The four-kind GPGA taxonomy — is a region's kind DECLARED or DERIVED?

**STATUS: LIVE, 2026-08-11 — ★ §1.3's recommendation is now BUILT.** The analysis half
(§0–§7) was written on branch `fb-memfd-join` at `b3ecda4` and is unchanged below except
where a correction is folded in above the text it corrects. The build is on branch
`gpga-region-kind-decision`, and §8 (added at the bottom) records what landed, what it
refuted, and the hole it does **not** close.

Supersedes nothing. **Corrects** `fb_leaf_crossing.md` §1 and §3 (folded in there, above the
text they correct).

★★★ **READ §8 BEFORE §0–§7.** Three statements in the analysis are now out of date as
*descriptions of the code*, and each is corrected in place below:
- §1.1's table of two unguarded arms — the first arm is **gone** (§8.1).
- §1.3's *"the mechanical change is well-shaped"* — it was taken, **but not by the mechanism
  it proposed** (§8.2 — a `Kind` parameter beside a `Binding` would have been a second source
  of truth, which is the thing the rule it cites forbids).
- §7's *"R32 … built, gated, UNRUN"* — still true, and ⊘ **it is NOT a prerequisite for
  anything here**; the brief that commissioned §8 asserted the opposite (§8.5).

---

## 0. ★★★★★ LEAD: THE TAXONOMY HAS NO DECISION POINT, BECAUSE WE ARE NOT PRESENT AT ALLOCATION

The model says a region's kind is **"DECIDED WHEN IT IS ALLOCATED"**, and names the guest's
surfaces as *"the RM map/alloc ioctls, `OS_DESCRIPTOR`, the UVM unified-VA ioctls, and the
one doorbell page."*

⊘ **In Mode 2, three of those four are not our surface at all.** The guest runs the **stock
kernel driver**; `/dev/nvidiactl` and `/dev/nvidia-uvm` are the *guest's own*. Its RM
allocates video memory out of **its own heap over the framebuffer we advertise**, and never
asks us anything.

| the model's surface | reality in this tree |
|---|---|
| RM alloc ioctls | ⊘ **no transport.** `NV_ESC_RM_ALLOC` has no guest-side decoder. `capability.rs:143-150` says the C's 23-row frontend allowlist was deliberately **not ported** because *"neither its ioctls nor the frontend escapes ever reach us"* |
| RM map ioctls | ⊘ **no producer.** `RmEvent::MapMemoryDma` is unreachable: `rpcMapMemoryDma` is a **HAL stub** on GA106, and the C artifact that booted to CUDA had no fn-14/15 hook (`gsp_core_bridge.md` §2.7) |
| UVM unified-VA ioctls | ⊘ **zero handlers.** The guest's `nvidia-uvm` talks to the guest's `nvidia.ko` |
| the doorbell page | ★ **ours, and real** — the one surface the model gets right |

### 0.1 ★★★ MEASURED: the two classes that decide a backing never cross to us

Our only guest ingress for object creation is the **GSP RPC** wire (`GSP_RM_ALLOC`, fn 103,
`kayfabe-rmrpc/src/lib.rs:1073-1148`). Two disjoint populations, both from committed traces:

```
GSP wire — every hClass our boots ever saw (traces/guest_boots/*dmesg*.log):
   112 × 0xc36f    112 × 0x402c    112 × 0x0070    111 × 0x208f    1 × 0xc076

the guest's OWN /dev/nvidiactl, real hardware (run_w217_2f616e2_grpush_probe.log):
    19 × 0x003e  NV01_MEMORY_SYSTEM        ← allocates backing
     5 × 0x0040  NV01_MEMORY_LOCAL_USER    ← allocates backing
    16 × 0xc7b5   16 × 0xc574   16 × 0xc56f   8 × 0xc7c0   …
```

⇒ **24 backing-allocating calls per CUDA run, and zero of them reach us.**

★★ **And the absence is a measurement, not a silence** — which is the half that matters,
because *"an absent line means 'not asked', not 'asked and got nothing'"*. Here **arrival
would be necessarily loud**: `versions.rs::alloc_params` has **no arm** for `0x003e`,
`0x0040` or `0x50a0` (it falls through `_ => None`, `versions.rs:1262`), and
`translate_alloc` turns a `None` shape into `BridgeRefusal::UnmappedAllocClass` — which is
exactly what makes the guest print `rpcRmApiAlloc_GSP: GspRmAlloc failed … hClass=…`. Those
lines are the census above. A memory-class alloc that arrived **could not** have gone
unrecorded. ⇒ It does not arrive.

⊘ `0x0070` on the wire is `NV01_MEMORY_VIRTUAL` — an *address space*, not a backing store,
and it is refused too.

### 0.2 ⇒ What this does and does not do to the model

★ **The taxonomy is not wrong. It is unwitnessed.** As a statement of what a region's kind
*ought* to be, all four kinds are coherent and kind-4 (`OS_DESCRIPTOR`) is measured working
on this GA106 (§7). What it lacks is an **event at which to decide**. The first time a GPGA
address exists for us is when a **byte moves through it** — a trapped BAR store, or a
page-table walk during a submission — and by then the guest has already chosen.

⇒ **The fix is not "add a decision point"; it is "manufacture a witness".** Only three
candidates exist, and two are already refuted:

| candidate witness | verdict |
|---|---|
| the GSP RPC wire | ⊘ **refuted, §0.1** — the classes are not on it |
| the guest's page tables (we already decode them) | ⚠ available, but it is a **classification of what the guest already did**, arriving after the fact. It can *label*; it cannot *decide* |
| ★ **advertise less framebuffer** | the only lever that exists today, and it is real — see §5 |

---

## 1. Q1 — is the taxonomy implementable as a DECLARED property?

**As a type: yes, and the seed is already in the tree. As a decision: not today (§0).**

### 1.1 Confirmed — `Representability` is derived, and `Fabricated` is the fall-through

> ⊘ ★ **CORRECTION (2026-08-11, folded in above the text it corrects — branch
> `gpga-region-kind-decision`).** Everything below was true when it was written and **the
> first row of the table is now false**: `kayfabe_mmu::Binding` carries a
> `RegionKind`, decided by one of its two constructors, and `representability_of` reads it.
> The `Binding::host == None ⇒ Fabricated` fall-through **no longer exists**.
> ⚠ **The SECOND row survives unchanged and is the standing hazard**: a range with no row at
> all is still `Untracked` and still routes to the real host GPU. See §8.4.

No struct anywhere carries a kind field. `Representability` is recomputed per operand by a
four-arm match, `kayfabe-fwd/src/lib.rs:4780-4832`. **Two arms are defaults, and they point
in opposite directions:**

| arm | line | reached because | result | routes to |
|---|---|---|---|---|
| `Some(b) if host.is_some_and(is_sole_backing)` | 4802 | guarded | `HostBacked` | host CE |
| `Some(b) if host.is_some()` | 4816 | guarded | `Err(BackingNotGuestVisible)` | refused by name |
| **`Some(b)`** — no guard | **4820** | ★ **`Binding::host == None`, i.e. nothing decided** | **`Fabricated`** | our CPU executor |
| **`None`** | **4830** | ★ **no row at all** | **`Untracked`** | ⚠ **real hardware** |

The enum's own header says it outright: *"it is a property of the address alone."*

⚠ **The second default is the dangerous one and is easy to miss.** A range we know *nothing*
about goes to the **host GPU**; the same range once given a row with no host object becomes
`Fabricated` and goes to our CPU executor. ⇒ **Populating the address table moves work OFF
the hardware arm.** Only the #14 ring gate stands behind that, and see §1.3.

### 1.2 ⊘ CORRECTION to the brief: `BackingBytes` is not the thing to delete

The brief says `BackingBytes::{SoleBacking, ShadowsGuestMemory}` *"is a runtime check for a
state the owner's model makes unrepresentable."* **Half right, and the other half is the
point.**

★ `BackingBytes` is the **only declared kind in the system** — *"a constructor argument with
no default, deliberately: the fact is knowable only at the instant the backing is created,
by the chain that created it, and is unrecoverable afterwards"* (`kayfabe-mmu/src/lib.rs:236-239`).
It exists *because* this exact bit used to be derived from `host.is_some()` and was measured
**backwards** on 2026-08-11.

⇒ Under the four-kind model the *variant* `ShadowsGuestMemory` does become unrepresentable —
but the *mechanism* is precisely what the model asks for, and it is the only thing in the
tree already doing it. **`BackingBytes` is the seed of `Kind`. Generalize it; do not delete
it.**

### 1.3 Every site that derives, and would instead consult

| site | file:line | what it does today |
|---|---|---|
| `representability_of` | `fwd/src/lib.rs:4780` | **the** derivation. One call site (`:4935`) |
| `TableOperands::new` | `fwd/src/lib.rs:4920-4924` | ⚠ `_ => Untracked` on a missing table **or** a missing PDB ⇒ **one `None` PDB makes an entire channel's operands hardware-bound** |
| `WalkOperands::resolve_runs` | `rt/src/ceutils.rs:371,378` | ★ **a SECOND, independent producer of `Fabricated`** — hard-coded for every run, never consulting a `Binding`. Justified by a boot measurement, not structurally |
| `partition_ce` | `fwd/src/lib.rs:5119`, `:5133` | `Untracked` for a clipped source; `_ => Ours` when the two ends disagree (conservative) |
| `host_published` — the **#14 ring gate** | `fwd/src/lib.rs:2480-2482` | ⚠ **still on the REFUTED bare `host.is_some()`** |
| `AddressTable::bind` | `mmu/src/lib.rs:521-565` | ★ **has no opinion.** Both its laws are `if let Some(h) = binding.host`, so `host: None` sails through unexamined |

★★ **A live defect found on the way, independent of this rung:** the ring gate and the
classifier now **disagree**. A `ShadowsGuestMemory` publication **passes** `host_published`
and is **refused** by `representability_of`. Neither doc cross-references the other. The gate
was not updated when `BackingBytes` landed.

**Three of five `bind` sites insert with no decided kind** — `walker.rs:909,924` (guest PTE
decode), `promote.rs:1160` (GR context promotion — ⚠ its own comment says *the host allocated
and mapped it for itself*, yet it records `host: None`, so it classifies as fiction),
`gpu.rs:3002` (RPC-declared mappings).

> ⊘ ★★ **CORRECTION (2026-08-11).** The recommendation below was **taken in substance and
> refused in mechanism**, and the refusal is on this doc's own grounds. A `kind` parameter
> *beside* a fully-formed `Binding` would be a **second source of truth** that `bind` could
> only reconcile with a runtime check — which is exactly what §1.2 says `BackingBytes` exists
> to avoid. The kind went **on `Binding`**, whose fields are now private with two
> constructors; the struct literal stopped compiling at every site, so the compiler named
> them anyway, and the forbidden combinations became unwritable rather than checked. See §8.2
> for the five sites and what each decided.

⇒ **The mechanical change is well-shaped: put the kind in `AddressTable::bind`'s signature.**
That is the one funnel every row passes through, it currently requires a fully-formed
`Binding` and inspects nothing, and making `Kind` a parameter with no default deletes both
fall-throughs at once — the compiler then names all three under-declared sites. ★ This is the
`BackingBytes` move applied one level out, and it is the part of the model that **is**
implementable today.

⊘ **And it closes a hole the taxonomy would fix by construction:** `VerbPlan::PinGuestRam` —
the one chain that genuinely shares memory with the guest, i.e. the model's kind-4 — **never
produces a `Binding` at all**. It records into `Vas::guest_ram_pins`. The classifier has
never seen it.

---

## 2. Q2 — what happens when the guest maps an unallocated GPGA region?

**Established, not inferred: there is no "unallocated" state to be in.**

`SparseFb` fabricates a zero page **on the spot**, on first write, for any address below the
advertised `fb_length` (`fbwin.rs:708-718`). The module says so:

> *"And within the framebuffer this chip **advertises** there is no refusal to have:
> `SparseFb` allocates a page on first write, so every address below `ChipProfile::fb_length`
> accepts bytes."* — `kayfabe-device/src/fbwin.rs:65-68`

⇒ **The brief's guess is right — "it becomes fabricated by default" — and the proposed fix
does not follow.** A decision point needs an event, and §0 shows there is none. What exists
instead is a *bound*: `fb_length`. Everything under it is fiction by default; everything over
it refuses by name (`OUTSIDE_FRAMEBUFFER`).

⊘ **`Site::Framebuffer` is a classification of a page-table walk, not a decision.** The chain
is: `ceresolve.rs:519` descends the **guest's own** directory and returns the leaf PTE's own
`(phys, aperture)` → `DeclaredResidency::residency_of_aperture` maps `Vidmem → CpuPlane::Fb`
by a **pure static table that does not even read the address** (`fwd/src/lib.rs:4730-4738`,
`_addr: u64`) → `ceutils.rs:983-990` names it `Site::Framebuffer`. So
`Framebuffer { phys: 0x800000 }` means exactly: *the guest's own leaf PTE says
`GMMU_APERTURE_VIDEO` at `0x800000`*. Nobody decided that. The guest's RM did, in its own
heap, in memory we never see.

---

## 3. Q3 — does anything require the fake framebuffer on a USERSPACE path?

## ★★★★ YES. BAR1. And it is worse than a fifth kind — the fiction is the userspace path's ADDRESS MODEL.

All three windows are served from the one `SparseFb` (`shim.rs:6385`, the single production
installation):

| window | BAR | reachable by guest userspace? | evidence |
|---|---|---|---|
| PRAMIN | BAR0 `0x700000`, 1 MiB | ⊘ **kernel only** | `GA106_USER_REGISTER_ACCESS_MAP = NOT_PUBLISHED` (`ga10x.rs:1444`), so `gpuGetUserRegisterAccessPermissions` answers `NV_FALSE` to everything |
| **FB aperture** | **BAR1, 256 MiB** | ★★★ **YES — this is *the* userspace surface** | it is what `NV_ESC_RM_MAP_MEMORY` + `mmap` produces for a vidmem object. BAR1 is `NVKVM_KIND_TRAP` with **no memslot** (`qemu/hw/misc/nvkvm/nvkvm.c:694-713`), so every userspace store traps into `fb_write` → `SparseFb` |
| instance window | BAR2, 32 MiB | ⊘ **kernel only** — RM never hands BAR2 to a client | |

★ So the owner's intuition holds for **two of three** windows, and fails on the one that
matters: BAR1 is the ordinary CPU view of video memory — CPU-mapped buffers, USERD, GPFIFO
rings.

★★ **And the coupling is tighter than "a fifth kind".** `bar1_phys` walks the guest's BAR1
page directory **out of `SparseFb` itself** (`plane.rs:2788`, `FbStoreReader { fb }`). ⇒ A
userspace BAR1 store both **reads its own translation from** and **writes its data into** the
fiction. Removing the fiction from the userspace path is therefore not "route these bytes
elsewhere" — it is replacing BAR1's address model.

### 3.1 ★★★★★ AND THE RULE IS NOT MERELY UNENFORCED — IT IS UNSTATEABLE

*"Unprivileged guest userspace should never see the fake framebuffer"* cannot be written down
in any code that exists, because **the trap path carries no principal.** The signature is four
scalars, unchanged from QEMU's callback to the store:

```
nvkvm_bar1_write(opaque, hwaddr addr, uint64_t val, unsigned size)      nvkvm.c:651
  → kayfabe_shim_regs_write(handle, bar, off, size, val, out)           kayfabe_shim.h:1201
    → RegPlane::write(&self, bar: u32, off: u64, size: u32, val: u64)   shim.rs:6863
      → fb_write(&self, w: FbWindow, off: u64, size: u8, val: u64)      plane.rs:2939
        → SparseFb::write_tagged(phys, bytes, FbWriter::Window(w))      plane.rs:3006
```

No CPL, no CR3, no vCPU id, no guest PID, no `Proc`. A grep for
`cpl|privilege_level|guest_pid|process_id` across `kayfabe-device`, `kayfabe-qemu-raw`,
`kayfabe-vmm-kvm` and `kayfabe-rt` returns **nothing**. The only guest-principal identity in
the system — `ClientKind`, from the client root's `processID` — lives on the **RPC** plane and
never touches MMIO.

⊘ **The `FbWriter` census cannot help**: its five kinds are `Window(Pramin|FbAperture|
InstanceWindow)`, `Executor`, `Unattributed` — a vocabulary of **which aperture**, never
**which principal**. And there is exactly **one** guest-caused `write_tagged` call site
(`plane.rs:3006`); kernel and userspace arrive at the same line, indistinguishably.

★ The tree already documents the identical collision one plane over
(`doorbell.rs:5-11`): guest userspace rings the doorbell through the 64 KiB usermode mapping,
kernel RM rings it through `GPU_VREG_WR32`, *"so there is exactly one offset, and both rings
arrive here."*

⇒ **A rule phrased over "guest userspace" needs a new fact on the trap path that QEMU's
`MemoryRegionOps` signature does not carry.** That is a real cost and it should be priced
before the taxonomy is adopted as a security property rather than a design one.

---

## 4. ⊘ What I did NOT establish

- ⊘ **Whether the guest can be made to stop asking for CPU-visible vidmem.** §5 is a
  direction, not a measurement.
- ⊘ **Whether `Kind` in `bind`'s signature is sufficient** — it is necessary and mechanical;
  what each of the three under-declared sites should *pass* is a separate ruling per site.
- ⊘ **Anything about two processes.** R32 (§7) is two mappings in one process.

---

## 5. ★★ THE ONE LEVER THAT EXISTS — and where it points

`OUTSIDE_FRAMEBUFFER` is the only mechanism by which a framebuffer address can fail today, and
its threshold is one number: `SparseFb::new(chip.fb_length)`, `fb_length = 12 GiB`. If the
fiction is meant to be small — kernel-internal channels only — then **advertising a small
framebuffer is the only way to make "we did not decide this" representable**, because it is
the only way an address can be refused rather than fabricated.

★ Followed to its end that is **`PDB_PROP_GPU_ZERO_FB`**: a part with no framebuffer, where
every allocation is sysmem reached by descriptor. Two things fall out and both are checkable
rather than rhetorical:

1. It is **exactly the owner's model** — kinds 1, 3 and 4, with kind 2 (the fiction) shrunk
   toward nothing.
2. ⊘ **It flips the door the brief called refused.** `nv-dmabuf`'s `can_mmap` is gated on
   `PDB_PROP_GPU_ZERO_FB` — the property is *true* only for three integrated chips. A device
   that claims it would not need the workaround the brief went looking for.

⚠ **This is reasoning, not measurement**, and it has an obvious falsifier: a stock driver may
refuse to bind to a GA106 that claims zero FB, or take an entirely different bring-up path.
That is one boot to find out, and it is a cheaper question than the taxonomy port.

---

## 6. ⊘ Corrections owed to `fb_leaf_crossing.md` (folded in there)

1. ★ **§1's rejection of sysmem inverts under a memfd.** It refuses `alloc_sysmem` because
   *"that verb asks for `MAPPING_NO_MAP` … deliberately un-CPU-mappable … fatal for this one,
   because GEN-2's mature form is a double mapping."* `MAPPING_NO_MAP` says **RM will not give
   you a CPU view**. Over a memfd you never ask RM for one: the CPU view is the **input**, and
   the "double mapping" is two `mmap`s of one fd. ⇒ `OS_DESCRIPTOR` is the one backing whose
   CPU view is guaranteed *by construction*, precisely because of the flag that disqualified it.
2. ⊘ **§3's "there is NO CPU view" is true of that path and false as a capability.**
   `RmConnection::map_cpu` (`NV_ESC_RM_MAP_MEMORY` on the control node + `mmap` on the per-GPU
   node, `rm.rs:1582-1611`) CPU-maps a real `NV01_MEMORY_LOCAL_USER` vidmem object, and R25
   **exercises it on this exact GA106**: the trace's `dst[0] 0xa112fffe -> 0x5eed0001` is two
   CPU reads and one CPU write of host vidmem. ⇒ The two-memories problem never required the
   blocked dma-buf door. The memfd should win on its merits (no BAR1 ceiling, the fd crosses
   to the isolate, sparse), not because the alternative is shut.

---

## 7. R32 — built, gated, **UNRUN**

> ⊘ ★★★ **CORRECTION (2026-08-11): R32 IS NOT A PREREQUISITE FOR THE DECISION POINT, AND A
> BRIEF SAID IT WAS.** A brief commissioning §8 asserted that fake-FB → real-GPU promotion
> *"presupposes a GPU-reachable fake FB, i.e. the memfd-backed framebuffer, which is on
> branch `fb-memfd-join` (`b3ecda4`)"*. Two things wrong, both checkable from `git log`:
> `b3ecda4` is **this document**, not a framebuffer; the memfd work is `2624798`, and it is
> the `rmladder` **probe** below — an instrument, not a framebuffer. Nothing on any branch
> makes `SparseFb` GPU-reachable. See §8.5 for what the real blocker is.


`prove_fb_memfd_join` + `rmladder --fb-memfd-join[-negative]`. It measures the two properties
R25 does not: **J1** two independent mappings of one sealed memfd are one memory across an
`OS_DESCRIPTOR`, and **J2** GPU-write → CPU-read — the direction the stuck completion
semaphore needs, and the direction no `OS_DESCRIPTOR` evidence in this tree has ever run.

★ It survives the reframe and arguably matters *more* under it: the owner's model makes
`OS_DESCRIPTOR` the sanctioned route for **kind 4 and the scratchpad case both**, so J2 is a
precondition of two of the four kinds rather than one.

⚠ **It has not been run on hardware.** `cargo clippy --workspace --all-targets -- -D warnings`
(plus `--features kayfabe-qemu-raw/host-isolates`) and `cargo test --workspace --no-fail-fast`
are green at `2624798`; the bench sync was cut short by a timeout and no measurement exists.
**Predictions in `fb_memfd_join_prereg.md` remain unscored, and must stay that way until a run
produces them.**


---

# 8. ★★★★★ WHAT WAS BUILT — the decision point, and ruling 3 enforced

**Branch `gpga-region-kind-decision`, based on `d55187a` (`origin/master`).** Owner rulings of
2026-08-11 (1: four kinds decided at allocation/bind; 2: fake FB is for guest-KERNEL channels
we emulate; 3: *"no fake FB ever can be mapped to a real GPU VA of an isolate except the
scratchpad"*; 4: the scratchpad goes through `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`).

## 8.1 The type

⊘⊘ **CORRECTED 2026-08-11 (branch `fb-join-port`) — RULING 4 IS NOW BUILT, AND IT REACHES
THIS TABLE. Read this before §8.1 and §8.2.**

The row *"`real_gpu_memory` … refuses `Vidmem`, or `ShadowsGuestMemory`"* is **no longer the
whole rule**, and `PublishVidmem`'s *"★ REFUSED"* is now true of **one of two chains** rather
than of the path. There is a **third** `BackingBytes`:

| declaration | meaning | `Vidmem` aperture |
|---|---|---|
| `SoleBacking` | we invented the bytes, or they are the guest's own pages mapped through | **refused** |
| `ShadowsGuestMemory` | a SECOND memory at an address the guest reaches another way | **refused, under every aperture** |
| ★ `JoinsGuestWindow` | the guest's own framebuffer WINDOW has been re-pointed at these pages | **admitted** — this is ruling 4 |

★★★ **This is the scratchpad carve-out arriving, not ruling 3 weakening.** The object is an
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over host pages — verbatim ruling 4. What §8.1's text (and
`RegionKind::may_be_host_mapped`'s own doc) got wrong is one clause: *"…and therefore never
asks this question about a `Vidmem` region"*. The object's class is sysmem; the **region's
aperture is whatever the guest declared**, and the guest declares a framebuffer leaf `Vidmem`
because its `phys` is a framebuffer offset. So the carve-out does arrive wearing that
aperture.

⊘ **The repair is NOT "correct the aperture to sysmem"**, and the two are not equivalent —
`residency_of_aperture` sends them to different CPU planes:
1. `Binding::is_guest_ram()` is `matches!(aperture, Sysmem*)` and its contract is *"may a
   consumer hand `phys` to `Vmm::gpa_read`"*. A framebuffer offset may not.
   `[measured 2026-08-11, boot w232c_6fcedac]` the two number spaces collide in one address
   space (`V:0x1024000` / `S:0x41335000`).
2. `DeclaredResidency::residency_of_aperture` sends sysmem to `CpuPlane::GuestRam`. The joined
   bytes are reachable through the **framebuffer store and nowhere else** —
   `FbStore::install_join` puts the shared region in the `FbStore` and *removes the local
   pages*. Routing to guest RAM would read guest RAM at a framebuffer offset.

⇒ **Consequence for `residency_of_aperture`: none.** `Vidmem → CpuPlane::Fb` stays right,
because the join changes the **STORE**, not the row — which is the same reason the mechanism
works at all. ★ The aperture test is **qualified, never deleted**: `Vidmem` + `SoleBacking` is
still refused and `ShadowsGuestMemory` is still refused everywhere, so the admit is bought by a
third distinct word rather than by making the guard aperture-blind. The swept test in
`gpga_region_kind.rs` goes 4×2 → 4×3 cells, `(admitted, refused)` 2/6 → **5/7**, and
`only_the_join_admits_a_vidmem_aperture_…` pins the three cells that separate the two designs.

⚠ **`JoinsGuestWindow` is TRUE ONLY AFTER THE VIEW IS INSTALLED, and no type can check that.**
The chain is therefore ordered install→bind (`kayfabe_fwd::adopt_joined_fb_leaf`), and every
path that does not reach the bind releases instead. ⊘ **Unmeasured**: the mechanism's hardware
result (`[measured 2026-08-11, vh2, GA106, 8eb8dcd]`) ran bind→install, so the ordering has
never booted.

`kayfabe_mmu::RegionKind{FakeFramebuffer, RealGpuMemory, GuestPhysDma}` + `RegionKindFault`.
`Binding`'s fields are **private** and there are exactly **two** constructors:

| constructor | kinds | host object | refuses |
|---|---|---|---|
| `declared_by_guest(phys, aperture)` | 2 (`Vidmem`), 4 (sysmem) | never | `Peer` → `PeerHasNoKind` |
| `real_gpu_memory(phys, aperture, HostBacking)` | 3 | **mandatory** | `Vidmem`, or `ShadowsGuestMemory` → `FakeFbAtRealGpuVa` |

⇒ Two states become **unwritable**, and they are the two the old code got wrong:
*"real GPU memory backed by nothing"* (the `Fabricated` fall-through) and *"fake framebuffer
mapped to a real GPU VA"* (ruling 3).

★ **Kind 1 is the absence of a row**, not a variant. `AddressTable::kind_at` answers `None`,
the same `None` `binding_at` gives. A variant would be a second spelling of one fact.

⊘ **`BackingBytes` was kept, and §1.2 was right about why.** It is not the casualty of
`RegionKind`; it is one of the two independent tests `real_gpu_memory` performs. The two fail
independently — the aperture catches a caller honest about the address and silent about the
shadow, `BackingBytes` catches the reverse — and a single test would let either half be
deleted.

## 8.2 The five production bind sites, each named by the compiler

| site | decided | note |
|---|---|---|
| `mmu/walker.rs:882` — guest PTE decode | `declared_by_guest(leaf.phys, leaf.aperture)` | new `PopulateRefusal::UndecidableKind` for `Peer` |
| `core/promote.rs` — GR context promotion | `declared_by_guest(r.phys, r.aperture)` | new `PromoteFault::UndecidableKind`; ⚠ see §8.3 |
| `core/gpu.rs` — RPC-declared mapping | `declared_by_guest(phys, SysmemCoherent)` | aperture is a literal ⇒ kind 4, unconditionally |
| `fwd/lib.rs` — `VerbPlan::Publish` | `real_gpu_memory(gpa, SysmemCoherent, whole(.., SoleBacking))` | kind 3 |
| `fwd/lib.rs` — `VerbPlan::PublishVidmem` (`FbLeafBacking::Vidmem`) | ★ **REFUSED** — `FwdFault::RegionKindRefused` | §8.4; no production caller |
| `fwd/lib.rs` — `VerbPlan::JoinFbLeaf` (`FbLeafBacking::Joined`) | `real_gpu_memory(phys, Vidmem, whole(.., JoinsGuestWindow))` | kind 3, ruling 4 — see the correction at §8.1. ⊘ Bound by `adopt_joined_fb_leaf`, **after** the install |

Plus ~45 field-read sites (`.phys`→`.phys()` etc.) and ~25 test fixtures.

★ `representability_of` now **reads** `b.kind()`. The #14 ring gate reads the same authority
— ⊘ and that change is **behaviour-equivalent, not a fix**: with the shadow unconstructible,
`host.is_some()` and `kind() == RealGpuMemory` agree on every binding that can exist. Proved
by mutant M6 (revert the gate to `host.is_some()` → the suite stays **green**).

## 8.3 ⚠ A residual disagreement, named rather than papered over

`promote.rs`'s own comment says *the HOST allocated and mapped this GR context buffer for
itself*, which is ruling 1's kind 3 — but **no `HostBacking` reaches that site**, and kind 3
without one is exactly what `real_gpu_memory` refuses to let anyone write. So the truthful
declaration there is the guest-declared one, and the gap is a **supply-side** one:
`promote_ctx` would have to receive the backing. That is a change to what the promotion
carries, not a relabelling, and it was deliberately not made.

## 8.4 ★★★★★ THE HOLE (A) DOES NOT CLOSE — and it is the second derived default

Deciding the kind at bind settles what a **bound** range means. It says nothing about a range
**nobody bound**, and §1.1's second row is unchanged: no row ⇒ `Untracked` ⇒ the **real host
GPU**.

This is visible at the refused FB crossing. `commit_back_fb_leaf` asks
`Binding::real_gpu_memory`, is refused, and hands its host objects back as orphans:

- ✔ **when a row already exists** (the walker forward-populated the leaf), the row is **left
  untouched** — still `FakeFramebuffer`, still no host object — so the range classifies as
  `Fabricated` and goes to our CPU executor over the bytes the guest actually reads. ★ The
  refusal deliberately does **not** unbind: dropping the row would hand the range to
  hardware, which is worse than the state it refused.
- ⚠ **when no row exists**, the range stays `Untracked`. The publish chain does not populate
  the table — the walker does — so refusing here cannot invent the guest's declaration.

Both are pinned as tests (`fb_leaf_backing.rs`), the second one explicitly as **the gap**.

## 8.5 ⊘ PROMOTION — the verdict, and the blocker is NOT what the brief said

**Promotion is not needed to make the decision point correct, and it is not memfd-blocked.**

1. **A region whose kind is decided at bind has nothing to promote.** The guest writes ring
   contents *after* the mapping exists, so at bind time the page is empty; a copy would copy
   nothing. Promotion is only ever a remedy for regions **already** fabricated.
2. ⇒ It is therefore a remedy for **§8.4's hole and for the pre-existing population**, not
   for the mechanism. Nothing in §8 creates a new need for it.
3. ⊘ **The memfd claim is refuted** (§7's correction). Fake FB **is** host memory already —
   it is a `HashMap` on the VMM's heap — and `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` is precisely
   the verb for making already-CPU-mapped host memory GPU-reachable, measured working on this
   GA106 at `CapEff = 0` (R25). **No new mechanism is required.**
4. ✔ **The real obstacle is ALLOCATION ALIGNMENT**, and it is small. `SparseFb`'s page type
   is `Box<[u8; FB_PAGE]>` (`kayfabe-device/src/fbwin.rs:573`); `[u8; 4096]` has **alignment
   1**, so `Box` allocates with align 1 and an allocator that happens to return an aligned
   block is luck, not a guarantee. A `#[repr(align(4096))]` newtype (or an explicit `Layout`)
   makes each fake-FB page individually descriptor-able **today, on master** — 16 descriptors
   for a 64 KiB ring, and one-descriptor-per-page is already the established pattern (R29,
   because the GR ring is not physically contiguous).

   ★★★ **The alignment requirement is CONFIRMED FROM THE DRIVER, not inferred from our own
   notes** (verified 2026-08-11 against both vendored trees; `ogkm` = 610.43.02, and
   `ogkm-580.159.04` is identical on the load-bearing lines):

   ```c
   // ogkm: src/nvidia/arch/nvalloc/unix/src/escape.c:142-147, RmCreateOsDescriptor()
   pDescriptor = NvP64_VALUE(pApi->data.AllocOsDesc.descriptor);
   if (((NvUPtr)pDescriptor & ~os_page_mask) != 0)
   {
       rmStatus = NV_ERR_NOT_SUPPORTED;
       goto done;
   }
   ```

   `os_page_mask = NV_PAGE_MASK` = Linux `PAGE_MASK` (`kernel-open/nvidia/os-interface.c:67`,
   `kernel-open/common/inc/nv-linux.h:743`), so the test is literally
   `if (addr & (PAGE_SIZE - 1)) return NV_ERR_NOT_SUPPORTED`. And on anything but aarch64 RM
   additionally refuses the whole path unless `os_page_size == NV_RM_PAGE_SIZE`
   (`osmemdesc.c:90-94`), and `NV_RM_PAGE_SIZE` is `1 << 12` (`kernel-open/common/inc/nv.h:312`)
   ⇒ **on x86_64 the requirement is exactly 4096.**

   Four consequences worth carrying:
   - ⊘ **Refused by name, before anything is pinned** — `NV_ERR_NOT_SUPPORTED`, raised ahead
     of `os_lock_user_pages`. So an unaligned page would fail loudly, not silently
     misattribute bytes. ⚠ But it fails **at the descriptor**, i.e. per page, at run time.
   - ✔ **LENGTH needs no alignment** — it is rounded UP to a whole page
     (`escape.c:156-157`, `osmemdesc.c:194-200` `NV_ALIGN_UP64(size, os_page_size)`). A
     4 KiB page is exactly one page, so the round-up is a no-op for this use.
   - ⚠ The round-up means **whole pages are pinned and GPU-addressable**. For a
     `#[repr(align(4096))] [u8; 4096]` that is precisely the page and nothing else — which is
     the reason to make the page its own aligned allocation rather than sub-slicing a larger
     buffer.
   - ★ **Our path already goes through the checking shim.** `RmConnection` issues this as
     `NV_ESC_RM_ALLOC_MEMORY` (`kayfabe-isolate-host/src/rm.rs:1805`), which reaches
     `RmCreateOsDescriptor` (`escape.c:401`) and therefore the check above. ⊘ The other
     door — a raw `NV_ESC_RM_ALLOC` of class 0x71 — would be refused anyway, because
     `NVOS32_DESCRIPTOR_TYPE_VIRTUAL_ADDRESS` is `NV_ERR_NOT_SUPPORTED` in
     `osCreateMemFromOsDescriptor` (`osmemdesc.c:135-137`). There is no way to hand RM an
     unaligned user VA and have it accepted.

   ⚠ `SPARSE_FB_RESIDENT_CAP` is `1 GiB`; a per-page alignment change alters the footprint
   per resident page and that budget should be re-checked with it.
   ⊘ `SparseFb` was **not** touched in this rung.
5. ★ **Scoping rule for whenever promotion is built**: it is safe **before** a channel is
   scheduled and unsafe after. Trapping CPU access does not stop an in-flight GPU engine, and
   a mutex over the CPU path does not cover one.

## 8.6 What is deliberately NOT done

- The promotion path (§8.5), and `SparseFb` page alignment.
- Moving the refusal from `commit_back_fb_leaf` into `plan_back_fb_leaf`. It would stop the
  execute phase allocating an object that is then orphaned — a real waste — but it makes the
  commit's fresh-publish arm unreachable, and that arm is where ruling 4's scratchpad case
  arrives once a fake-FB page can be an `OS_DESCRIPTOR`. Deleting it now would delete the
  landing site.
- §3.1's finding is untouched and still stands: *"unprivileged guest userspace must never see
  the fake framebuffer"* remains **unstateable**, because the BAR1 trap path carries `(bar,
  off, size, val)` and no principal at all. `RegionKind` is a design property here, **not** a
  security one, and §8 does not change that.
