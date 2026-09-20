> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# w750 — route K, phase 2: what the integration actually is, and why it is not one sitting

> ### ⊘⊘⊘ CORRECTED 2026-09-16 (w753) — **§1 IS RIGHT THAT ROUTE K CROSSES FOUR INVARIANTS,
> ### AND WRONG THAT THE MAPPING PLANE CROSSES ALL FOUR.** Read this before §1.
> Measured by reading the call graph, not the counter: `StoreMapPort` reaches RM through
> `SharedIsolate::with_worker` → `Worker::with_rm`, which takes a **closure** and never
> builds a `VerbPlan`. ⇒ **§1.1's foreign-handle gate and §1.2's `Staged::check_out` are NOT
> on the path to arm 3's measured wall** — they are on the path to *birth in B*, which is a
> different question. Only §1.3 (`lend_to`) and §1.4 (the request-direction fd) are crossed
> by the mapping plane.
> ⇒ **Increments 1–5 + 7 are one session's work, not three**, and per constraint 29 nothing
> that is not crossed is retired: 1.1 and 1.2 stand un-restated **on purpose**. Retiring an
> assert *"because the design moved"* when the design did not move past it is the deletion
> constraint 29 exists to refuse.
>
> ### ⊘⊘ AND §0's FRAMING UNDERSELLS WHAT ARM 3 NEEDS FROM K, WHILE §3 STEP 8 OVERSELLS IT
> §0 says the RM sequence is done and what is missing is the four restatements. True. But
> the reason arm 3 is **red** is not named anywhere in this file: its measured wall is
> `STORE-MAP adopts=0 adopt_refused=4718 first_refusal=[Rm("InsufficientPermissions")]`, and
> route K dissolves it **by construction** (the dup's destination carries I's `ProcessID`, so
> it is a same-PID dup needing no grant — `sharing.c:341-352`). That consequence is stated
> once in the tree, at `fable_leg_b_solution_space.md:227`, and nowhere in the plan that
> costs the work.
> ⚠ ⊘ **AND FOUR LINKS DOWNSTREAM OF THAT DUP HAVE NEVER RUN ON HARDWARE** at three
> consecutive boots (`SINGLE_STORE_PLAN.md:2389-2398`): `map_store_slice`, the slice binding,
> constraint 28's placement assert, and the ring oracle. ⇒ **step 8's gate is not a
> prediction this plan is entitled to make.** A green dup is not a green client.
> Pre-registration, with the refuting value for each row: `w753_route_k_phase2_prereg.md`.
>
> ### STATUS — 2026-09-16 / **SUPERSEDED IN PART — increments 1-5 and 7 are BUILT (w753).**
> ⊘ The text below is the plan as written before any code existed. Increments **1, 2, 3, 4, 5
> and 7 landed** on branch `w753-route-k-phase2`; **6 did not**, for the reason in the
> correction above. Read §1.1 and §1.2 as *"what birth in B will cost"*, not as *"what this
> increment owes"*.
>
> ### STATUS — 2026-09-16 / **LIVE — PLAN ONLY. NO PRODUCTION CODE WRITTEN.**
> Phase 1 passed (`w750_route_k_prereg.md` §RESULTS; constraint 32 updated in place), so
> phase 2 is unblocked **and deliberately not started in the same session.** This file says
> what it is, at what cost, with every claim carrying a `file:line` I re-read myself rather
> than inherited. ⊘ **No constraint is relaxed here and none is proposed relaxed.** Four are
> proposed *restated*, and each restatement is named with the question it must keep asking.

---

## 0. The one-paragraph answer

The **RM sequence is done**: `kayfabe-rm-ladder --route-k` performs the whole of constraint
32's step list against a real GA106 and all four discriminators hold. What is missing is not
driver work — it is that route K makes a channel's `HostHandle` belong to the **scratchpad**
while the guest-side plan that births it belongs to the **per-guest-process isolate**, and this
tree asserts *"one plan, one worker, one isolate"* **in the type system, on purpose, in four
separate places.** Each of those four is a safety property with its own argument; none may be
deleted, and constraint 29 says each may only be retired by re-asking its question in the new
shape, fail-closed, with the successor named. ⇒ phase 2 is a **multi-session increment whose
cost is the four restatements**, not the ioctls.

---

## 1. The four invariants route K crosses, each verified by re-reading

### 1.1 ★★★★★ `Worker::execute`'s foreign-handle gate — the hard one

`crates/kayfabe-isolate/src/lib.rs:3862`:

```rust
if let Some(&handle) = plan.handles().iter().find(|h| !h.belongs_to(self.isolate)) {
    return Err(VerbFailure::bare(RmError::ForeignHandle { handle, worker_isolate: self.isolate }));
}
```

Every handle in a plan must have been minted by the worker running it. Route K's birth plan
names **I's VA space** and produces **an S-scoped channel**, so it spans two isolates and this
gate refuses it before RM is reached. ⊘ And the gate is right: it is what stops one guest
process's isolate naming another's objects, which is this rewrite's founding problem (`#14`).

⇒ **Successor required.** Not a relaxation: the question becomes *"does every handle in this
plan belong to a party this plan is ENTITLED to name?"*, where the entitlement is a **brokered
pair** `(I, S-for-I)` minted by the VMM, not a bare second identity. Its known-positive must be
the racing one: a plan carrying a handle from isolate **C** must still go red on I's worker
**while** the `(I, S)` pair is live.

### 1.2 `Staged::check_out` — one plan, one worker

`crates/kayfabe-rt/src/device.rs:1184` checks out exactly one worker per plan, from
`proc.isolates` (`crates/kayfabe-core/src/gpu.rs:1736`), keyed `(ProcId, GpuId)`. Route K's
birth is inherently two-phase — I mints B and surrenders the fd; S does everything after — and
`Staged` cannot express it. ⇒ either a **two-phase plan** (`MintBirthClient` then
`BirthInClient`) or a checkout that yields a pair. The two-phase shape is preferable because it
keeps *"one plan, one worker"* true and makes the crossing a **thing that happened**, with its
own counter, rather than a property of a plan's shape.

### 1.3 `CrossedFd::lend_to` — the cross-isolate descriptor refusal

`crates/kayfabe-isolate-host/src/fdcross.rs:138-146`: a descriptor whose origin is
`FdOrigin::Isolate(A)` handed to `B` is `RawError::ForeignDescriptor`. The module's own docs
(`fdcross.rs:24-38`) say this is the policy and cite the C's one exception as *"a separate,
explicitly argued exception and not a reason to relax the default."*

⇒ Route K needs **exactly that exception, argued**: a third origin — call it
`FdOrigin::BirthClient { minted_by: IsolateId }` — that may be lent to **one** target, the
scratchpad, and to nothing else. ⚠ The known-positive is the one that must be built first: an
attempt to lend a birth-client fd to a **second per-proc isolate** must stay red.

### 1.4 The request-direction fd path does not exist

`crates/kayfabe-isolate-host/src/proto.rs:469-472`, in the tree's own words: *"the fd-IN gap on
the request path is **REAL** (the child's reader is `read_frame`, with no control buffer at
all)"*. Descriptors cross child→parent only (`child.rs:603` is the sole
`write_frame_with_fds`). Route K needs I→VMM→S, i.e. **both** directions.
⇒ new: `Request::MintBirthClient` (reply carries the fd **up**, which already works) and
`Request::AdoptBirthClient { fd }` (request carries it **down**, which does not).

---

## 2. What route K does NOT have to touch — measured, so the estimate is not inflated

- **The doorbell fast path.** `crates/kayfabe-qemu-raw/src/shim.rs:6729` rings by raw
  `host_token`, into a usermode window exported by the **system** proc's isolate
  (`crates/kayfabe-rt/src/device.rs:2707-2709`), not by channel handle. Phase 1 confirms the
  token is all hardware checks: S rang a B-owned channel from **S's own** usermode mapping and
  `GP_GET` advanced (`K_DOORBELL_RC=0`, `K_GPGET_AFTER=1`).
- **The latch / peek / drain plumbing** (`device.rs:646, 1876, 1908`; `shim.rs:3123, 3298`) is
  handle-free.
- **The ring's provenance.** `RingProvenance::StoreSlice` already names no handle
  (`crates/kayfabe-isolate-host/src/rm.rs:7080-7084`) and its containment check is already
  VMM-side (`crates/kayfabe-qemu-raw/src/storemap.rs:435`).
- **CPU mappings of channel objects.** Nothing in the production birth path CPU-maps one — which
  matters, because phase 1 established that S **cannot**: `osapi.c:2378` refuses
  `NV_ESC_RM_MAP_MEMORY` whenever the client's `ProcID` is not the calling task's.

★ And the direction argument route K needs is **already in the tree, already asserted**:
`crates/kayfabe-rt/src/device.rs:6543-6562` — *"the space must be **created by the per-proc
isolate** and lent UP to the scratchpad, never created by the scratchpad and lent DOWN"*, with
the check on the handle's minting isolate rather than on a comment. **Route K is that same
argument applied to the client instead of the VA space**, and the `ScratchpadRole` /
`HandedVaSpace` pair (`rm.rs:390-436`, sole constructor `handed_over`) is the shape its
`HandedClient` successor should copy.

---

## 3. The increment order, smallest first, each landing green on its own

1. **`HandedClient` + `FdOrigin::BirthClient`, types only**, with the two refusals and their
   known-positives, no wire and no caller. (Mirrors how `HandedVaSpace` landed.)
2. **fd-IN on the request path** — `child.rs`'s reader gains a control buffer; one new request
   tag that carries a descriptor **down**, refused unless it is a `BirthClient`.
3. **A second `RmConnection` in the child**, and the ledger question answered *by type*: today
   `channel_parts`, `ce_channels`, `slots`, `fb_joins`, `forget_exec_vas` are all keyed in the
   child's **one** connection (`rm.rs:6036, 6054, 7086`). A second client that silently shares
   them is the defect this step exists to make unexpressible.
4. **`MintBirthClient` in I** — allocate `NV01_ROOT` on a second `nvidiactl`, answer with the
   fd, close the local copy. Census: `birth_clients_outstanding`, whose zero needs the
   `--skip-free`-shaped known-positive phase 1 already demonstrates.
5. **`BirthInClient` in S** — the probe's steps 3-6, lifted from `rmladder.rs`'s `mod route_k`.
6. **The two-phase plan** and the `check_out` pair; the foreign-handle successor (§1.1) lands
   here and nowhere earlier.
7. **The arm** — a third value beside `KAYFABE_VAS_OWNER`'s two
   (`crates/kayfabe-qemu-raw/src/scratchpad.rs:256-311`), refused rather than defaulted, for
   the reason `vas_owner_from` already gives; and a census line on every boot.
8. **The three-arm boot**: `arena+isolate` → `device+isolate` → `device+K`, via
   `scripts/bench/w736_fbstore_run.sh` with arm 3's env changed. The grade is the raw client:
   `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8` (`scripts/bench/w392d_mean_hook.sh:93,113`).

⚠ **Steps 1-5 are each independently green and independently useful. Step 6 is the one that
cannot be half-done**, and it is where the schedule risk lives.

---

## 4. What will go red on purpose, and must be re-argued rather than edited

`crates/kayfabe-isolate-host/tests/own_client_invariant.rs:522-620` asserts by string match that
there is **exactly one** `hClientSrc` site in the crate; `tests/ownership_split_gates.rs` and
`tests/guest_ring_census.rs` assert the ownership split's current shape. Route K adds a second
legitimate `hClientSrc` user and changes what `host_channel` means. ⊘ **These are not obstacles
to route around.** Each is constraint 29's case: the successor must ask the same question in the
new shape, fail closed, and be named in the commit that retires the old one.

---

## 5. Honest sizing

≈10 production files (`kayfabe-isolate/src/lib.rs`;
`kayfabe-isolate-host/src/{proto,child,isolate,rm,fdcross}.rs`; `kayfabe-fwd/src/lib.rs`;
`kayfabe-rt/src/device.rs`; `kayfabe-core/src/gpu.rs`;
`kayfabe-qemu-raw/src/{shim,scratchpad}.rs`), ≈19 call sites that today name the per-proc
client or handles for a channel, one new env arm with its census, and four type-level
invariants restated with a known-positive each. The RM work is **days and already proven**; the
rest is the four restatements and the two-isolate plan shape.

⇒ **This is a multi-session increment.** Starting it at the end of the session that measured
phase 1 would have produced a half-crossed seam in the one place this tree has deliberately
refused to have one — and `w729b` is explicit that a measured result with its evidence is a
full deliverable. Phase 1 is that; this plan is its handoff.
