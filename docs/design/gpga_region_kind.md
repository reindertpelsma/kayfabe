# The four-kind GPGA taxonomy — is a region's kind DECLARED or DERIVED?

**STATUS: LIVE, 2026-08-11.** Answers the owner's four-kind model as a question about this
code. Branch `fb-memfd-join`, based on `4428b6b`. ⊘ **Nothing here is built.** The one thing
that was built this rung (`R32`, §7) is a *measurement* instrument, is gated but **unrun**,
and is separable.

Supersedes nothing. **Corrects** `fb_leaf_crossing.md` §1 and §3 (folded in there, above the
text they correct).

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
