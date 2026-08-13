# Property 2, adjudicated against ruling 2 (kernel CE VA spaces need not exist)

> ### STATUS — 2026-08-11 / **LIVE — ADJUDICATION. Read-only; nothing here was built.**
> Adjudicates the `[NOT MEASURED]` prediction in
> `C: docs/design/gpga_kind_is_decided_at_bind.md` §3 item 1 (*"it may DISSOLVE Property 2"*)
> against `C: docs/design/property_2_the_subtraction.md`, at `kayfabe` HEAD `b3ecda4`.
>
> ⚠ **This file is in the WRONG PLACE by this tree's own doc-hygiene rule.** A correction
> belongs folded into its parent, above the thing it corrects; the parent
> (`property_2_the_subtraction.md` §2) lives in the C repo and this lane was read-only there.
> ⇒ **§1 below must be folded into `property_2_the_subtraction.md` above its §2**, and this
> file deleted, by whoever next writes to that tree.
>
> **Verdict: ALREADY STALE** in the object it names — and **UNTOUCHED** on the exposure that
> actually survives.

---

## 1. ⊘ LEAD WITH THE REFUTATION — Property 2 §2's central claim is FALSE at HEAD

> *"★★ And there is exactly ONE host address space per guest address space today, holding all
> of: the guest's channel; the host-backed framebuffer leaves at guest-chosen addresses; and
> our own ring, cursor block and completion semaphore."* — `property_2_the_subtraction.md` §2

Three separate things in that sentence are wrong at `b3ecda4`, all readable in the code:

- **There are TWO host address spaces per guest `Vas`, not one.** `Vas::host_vas`
  (`kayfabe-core/src/gpu.rs:171`) and an `ExecutorVas` minted lazily beside it
  (`kayfabe-isolate-host/src/rm.rs:2975`, one per guest range, held on the `RmConnection`).
  `RmBackend::free` disposes of both, in that order (`rm.rs:3285`).
- **The isolate's ring, USERD and completion semaphore are NOT in the guest's space.**
  `ce_copy_outcome` → `executor_vas(key)` → `ce_channel(key, exec)` →
  `alloc_channel_for_isolate(vas: ExecutorVas)` → `alloc_channel_in(vas.range, …)`
  (`rm.rs:4442,4443,3812,3817`). The ring object is mapped through `exec.range` and through
  nothing else. `ExecutorVas` cannot even be *spelled* by a caller holding a guest `Vas`
  (private field, one mint site, `tests/ui/name_an_executor_vas.rs`).
- **The "cursor block" is in no GPU address space at all.** USERD is handed to RM as
  `hUserdMemory[0]` and is only ever CPU-mapped (`rm.rs:4090` region); the free path unmaps a
  GPU VA for the *ring* only (`rm.rs:3320`-ish, `RingOwner::Ours` arm). A guest VA cannot name
  it under any topology.

### ⊘ And the exploit citation is a PRE-FIX measurement, quoted without its revision

Property 2 §2 cites *"R30 arm C, real GA106: a copy engine bound to the guest's address space
read our own semaphore payload back."* That measurement was taken at `cc5d55c`. The fix landed
at **`254cf38` (2026-08-10)**, `ae73f6b` (the S1 audit Property 2's co-location row rests on) is
an ancestor of it, and the **same arm was re-measured and REFUSED**:

```
ok  R30 spaces = guest range 0xcafe0005, control range 0xcafe0009 — two spaces
★   R30 arm C  = the guest-bound engine did NOT retire a read of 0x1_20022000
NVRM: Xid 31 … CE0 faulted @ 0x1_20022000, FAULT_PDE ACCESS_TYPE_VIRT_READ
```

(`docs/design/executor_vas_separation.md` §2; re-confirmed at `b39f95f` — *"R30 arm C REFUSED"*,
`guest_ring_adoption.md` §1.) ⇒ **Property 2 asks the owner to approve a subtraction that was
performed, and measured, the day before it was written.** This is the
*"a ruling's DATE is part of the citation"* class, applied to a measurement rather than a ruling.

★ Property 2 §3 names `ExecutorVas` as *"the precedent that makes it tractable"*. It is not a
precedent. It is the fix, already applied to the object §2 names.

---

## 2. ✔ The host-VAS census at `b3ecda4`, and what forces each space

**Two production families.** Everything else that calls `alloc_vaspace` is `rmladder`
(diagnostic), `kayfabe-mocks`/`loopback` (test doubles), or the isolate-host wire transport
(`child.rs:528`, `isolate.rs:482`) which only re-issues a caller's request.

| # | site | forced by which GUEST-SIDE fact |
|---|---|---|
| 1 | `kayfabe-isolate/src/lib.rs:2215` — `VerbPlan::Publish` | a guest bind that must be host-backed at the guest's own VA (`#102`) |
| 2 | `:2268` — `VerbPlan::PublishVidmem` | same, in host VRAM (the w228 FB crossing) |
| 3 | `:2317` — `VerbPlan::PinGuestRam` | same, over the guest's own pages |
| 4 | `:2387` — `VerbPlan::Doorbell` | ★ **the guest rang its own channel.** RM will not build a channel group without an `hVASpace` — zeroing it is a measured `0x1F`/`0x33` (`rm.rs:3944` comment) |
| 5 | `:2434` — `VerbPlan::EngineObject` | the guest allocated an engine object on a channel we must materialize |
| 6 | `rm.rs:2975` — `executor_vas` | ⊘ **NO guest fact.** Our own executor needs a home; minted from `map_dma_both` (`rm.rs:3022`) and `ce_copy_outcome` (`:4442`) |

All five of 1–5 write the same field: `Vas::host_vas`, keyed `(ProcId, GpuId, Pdb)`. The guest
kernel is a `Proc` like any other — `Gpu::SYSTEM_PROC = ProcId(0)`, `gpu.rs:4480,4571` — so a
**kernel** PDB acquires its host VAS through exactly these five sites. That is ruling 2's target.

⚠ Site 6 is *not* a guest VA space. Ruling 2 does not reach it, and could not: see §4.

---

## 3. Where the machinery actually lives — verified, not repeated

| object | address space it is mapped in | can a guest channel name it? |
|---|---|---|
| isolate CE ring + its pushbuffer slots + `SEMAPHORE_OFFSET` word | `ExecutorVas` only | **no** — no guest channel is ever bound to that space (measured, R30 arm C `FAULT_PDE`) |
| isolate CE / any channel's USERD (the cursor block) | none — CPU mapping only | **no**, structurally |
| **the materialized guest channel's 64 KiB ring** | `Vas::host_vas`, at an **RM-chosen** VA | ★ **yes** — this is the whole residual |
| fabricated publishes, guest-RAM pins, FB leaves | **both**, at one guest-chosen VA (`map_dma_both`) | yes, by design — they are the guest's operands |

The residual comes from the production verb: `RmBackend::alloc_channel` (`rm.rs:3206`) →
`alloc_channel_on` (`:3713`) → `alloc_channel_at(vas, engine_type, None)` (`:3760`) →
`alloc_channel_in(range, …, RingSource::Ours(None))` — 64 KiB of device-local memory ours,
`raw_map_dma`'d into the **guest's** range. `executor_vas_separation.md` §6 excludes it
explicitly: *"Nothing about the guest's own materialized channel ring. That is isolate-allocated
memory which stays in the guest's space by design — it is the guest's channel."*

⇒ Property 2 §3's *mechanism* sentence ("`alloc_channel_at` must change") is aimed at the right
call. Its §2 *evidence* is aimed at an object that already moved.

---

## 4. ⊘ THE RULING'S PREMISE FAILS FOR KERNEL CE — two code facts

> *"kernel copy engine va spaces dont have to exist on real gpu as for any relevant action you
> translate in vmm anyways."*

1. **Every CE copy this tree can issue is VIRTUAL, by a standing refusal.** `ce_pushbuffer`
   ORs `LAUNCH_SRC_VIRTUAL | LAUNCH_DST_VIRTUAL` (`rm.rs:1930`). `LAUNCH_SRC_PHYSICAL` /
   `LAUNCH_DST_PHYSICAL` exist only as **decode-side** constants, and the encoder's doc refuses
   them by name: *"Virtual, always, for anything an isolate submits. `_PHYSICAL` points the
   engine at physical addresses with no MMU between it and the rest of the machine; nothing in
   this project's threat model permits it"* (`kayfabe-abi/src/submit.rs:2023-2031`). ⇒ Kernel CE
   work needs **a** host VA space. What ruling 2 can delete is the **guest's** space, never *a*
   space — and the space it cannot delete, `ExecutorVas`, is the one that already carries the
   separation.
2. **There is no producer of an executor-space mapping that does not start from the guest
   space.** `map_dma_both` (`rm.rs:3014`) runs `raw_map_dma(guest_range, …)` **first** and feeds
   the address RM reported back into the shadow map. Delete the guest-facing VAS and the
   executor space has no operands, because nothing else maps into it. Nothing in the tree maps a
   GPGA range into an `ExecutorVas` at a host-chosen VA — the VMM-side translation the ruling
   invokes exists in `AddressTable::resolve`, but no *verb* consumes it that way.

⚠ And ruling 2's consequence 2 (*"it deletes work we currently do … the doorbell path calls
`alloc_vaspace`"*) understates the blast radius: with no `host_vas`, `plan_doorbell` returns
`FwdFault::NoVas` (`kayfabe-fwd/src/lib.rs:2729`) and `plan_ce` returns `FwdFault::NoHostVas`
(`:5396`, whose own doc says materializing an empty one *"would turn a refusal into Xid 31"*).
⇒ Applied literally today, ruling 2 turns kernel-channel forwarding **off**; it does not reroute
it. The reroute is unbuilt work, not a deletion.

---

## 5. ⇒ THE ADJUDICATION

- **On the object Property 2 names** (the isolate's ring / cursor block / completion semaphore):
  **ALREADY STALE.** Fixed at `254cf38`, hardened at `b66bd44`, measured green twice. Ruling 2
  has nothing left to dissolve there.
- **On the residual** (the materialized guest channel's ring in `Vas::host_vas`): **UNTOUCHED on
  the graphics path.** A graphics channel belongs to a guest **userspace** RM client
  (`ClientKind::User`), and ruling 2 exempts exactly that case — its own §3 item 3: *"It does NOT
  touch the userspace path."* The residual is dissolved only for `ClientKind::Kernel` PDBs, and
  only by also deleting their forwarding route.
- **What would actually dissolve the residual**, and it needs no new address space: **`w230`'s
  guest-ring adoption.** `alloc_channel_over_guest_ring` (`rm.rs:3791`) builds the channel over
  the guest's *own* pages, so `RingOwner::HandedIn` allocates and maps **nothing** of ours into
  the guest's space — only USERD stays ours, and USERD is in no VA space. It is built and has
  **one caller, the R31 probe** (`tests/guest_ring_census.rs:168`). Promoting it to the doorbell
  path removes the residual as a side effect of the work the execution plane needs anyway.

### ⚠ One NEW exposure, created by the separation, named nowhere — `[NOT MEASURED]`

`map_dma_both` places every guest publish in the `ExecutorVas` at the **guest-chosen** VA, while
our CE ring sits in that same space at an **RM-chosen** VA. A guest fixed publish landing on our
ring's 64 KiB makes the shadow map fail, and the verb then tears the guest-side map down and
returns `PlacementRefused` for a publish that would otherwise succeed. ⇒ A guest can **locate the
isolate's CE ring by binary search over publish refusals**, in a space it can never read. This
is an address disclosure, not a read, and it is an artefact of the fix rather than of the defect
it replaced. Inferred from code (`rm.rs:3029-3040`); no probe has asked hardware.

---

## ✔ Verified by me at `b3ecda4`, in the working tree and via `git show`
`isolate/src/lib.rs:2215,2268,2317,2387,2434` · `rm.rs:2975,3014,3021,3206,3218,3285,3713,3760,
3812,3817,4442,4443,4499` · `submit.rs:2023-2031` · `gpu.rs:171,4480,4571` ·
`fwd/src/lib.rs:2726-2733,5396` · `git merge-base --is-ancestor ae73f6b 254cf38` = yes.
⊘ Not measured by me: any boot. No claim above rests on a boot I ran.
