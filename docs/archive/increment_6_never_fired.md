> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# Increment 6 never fired — the birth is in the wrong process, and my gate pinned it there

**STATUS: ✔ ANSWERED 2026-09-17 (w755u) — the delegation was built, the doorbell was made to
follow the channel, and the gate MOVED. See §6 at the end for the result. The diagnosis below
is kept verbatim because it is what the fix was measured against.**

**Originally: LIVE — measured 2026-09-17 (w755q), on vast box 51304517, RTX 3090, host driver
580.159.04, tree `eb3fc08f`, three arms, `ARM3_VAS_OWNER=k`.**

---

## 0. The headline

**Route K increment 6 did not execute.** The boot is therefore **not** evidence that birthing a
store-slice USERD in client B fails — it is evidence that the code path was never reached.

⚠ And the reason is my own: **the route is in a function this path does not call, and the gate I
wrote asserted its position in that same unreached function.** A green gate held the wall in
place — `a_green_test_can_hold_a_wall_in_place`, exactly.

---

## 1. What the boot measured

Route K itself is healthy and that part is real:

| row | value | prior |
|---|---|---|
| `CONSTRAINT-32 BIRTH-CLIENT MINTED / ADOPTED / HANDED` | **2 / 2 / 2** | — |
| `W745-BIRTH-REFUSED` | **0** | 11, 11, 11 |
| `W745-BIRTH-ADOPTING` | **11** | 0 on all three prior boots |
| `STORE-MAP ... adopts=4 adopt_refused=0 maps=45 map_refused=0` | — | — |
| `W746-SCRATCHPAD-BIRTH-REFUSED` / `W746-C30-BIRTH-REFUSED` | 0 / 0 | — |

And the gate did not move: **`P1 rm-invalidate` still `NEVER RETIRED`**, 8/8 lanes, `P2` the
same, `P3 UNEXERCISED`, `STALE RACE` refused. 16 thread faults, all *"the completion semaphore
never reached `0x6d000001`"*.

## 2. ⊘ The decisive counters — increment 6 is ABSENT, not failing

```
CHANNEL-BIRTH IN B                 : 0      ← birth_in_b's SUCCESS print
"NO BIRTH CLIENT holds this range" : 0      ← birth_in_b's first refusal
"B holds no dup of the store"      : 0      ← birth_in_b's second refusal
```

`birth_in_b` prints on **all three** of its exits. All three are zero ⇒ **it was never entered.**

⊘ Note how nearly this was misread: `grep CHANNEL-BIRTH` against the *harness stdout* returns 0,
which looks like the same answer for the wrong reason — the isolate's stderr is in the **QEMU
log**, where `CHANNEL-BIRTH` appears **22** times. The right check was *"are `kayfabe-isolate:`
lines captured here at all?"* — they are, 35 of them. A count of zero from a stream that carries
none of the producer's output is not a measurement.

## 3. ★★★ What actually refused, and it names its own fix

```
kayfabe-isolate: GR-BIRTH ⊘⊘ REFUSED USERD_IN_STORE_NEEDS_BIRTH_IN_B — the guest's USERD is a
slice of the ONE reserved store, which this per-proc isolate may not name (constraint 26).
The birth belongs in the birth client B, which holds a dup.          × 11
```

- raised at `rm.rs:8945`, inside **`alloc_channel_lowered`**
- printed by **`iso2`** — a **per-proc isolate**
- reached via `alloc_channel` / `alloc_channel_declared` → `alloc_channel_lowered`

**My increment-6 route is at the head of `alloc_channel_in`** (`rm.rs:9906`), reachable from
`alloc_channel_over_guest_ring` — and `alloc_channel_in` takes a **range**, which is what
`adopt_space` returns *in B*. So the route is in **the scratchpad's** verb family.

⇒ The two families never meet:

| family | entry | who runs it | has `birth_ranges`? |
|---|---|---|---|
| `alloc_channel` · `alloc_channel_declared` → **`alloc_channel_lowered`** | the guest's birth-at-alloc | **per-proc isolate** | **no** |
| `alloc_channel_over_guest_ring` → **`alloc_channel_in`** → `birth_in_b` | the scratchpad's range verb | **scratchpad** | **yes** |

★★★ **So this is not a misplaced line — it is a MISSING DELEGATION, across a process boundary.**
`birth_in_b` resolves B through `self.conn.birth_for_range`, and `birth_ranges` is populated at
`rm.rs:7575` inside the **scratchpad's** `adopt_space` (`CONSTRAINT-32 ADOPT-IN-B`). A per-proc
isolate has no B and structurally cannot birth in one. Its refusal is **correct**; what is
missing is that nobody acts on it.

⇒ The fix: when a guest channel's USERD is `UserdObject::TheStore`, the birth must be **executed
by the scratchpad**, not by the proc's own isolate. That is a change in *which process runs the
verb*, which is why moving a line inside `rm.rs` cannot fix it.

## 4. ⊘ A SECOND, INDEPENDENT GAP — the oracle is never in hand when it matters

```
oracle_installed=true  : 0
oracle_installed=false : 4      ← every RING-NOT-A-SLICE
```

`adopted_guest_ring` has **two** consult sites:
- `lib.rs:4853`, inside `plan_engine_object` — passes **`None`** on purpose. Its comment says the
  latch path *"has no route to the composition root's store-map port … the channel is born at its
  own alloc, which is where the oracle is in hand."*
- `lib.rs:5512`, inside `plan_channel_birth` — passes the real `ring_slice`.

**The second one never saw a store-slice ring: `oracle_installed=true` is zero.** So the
intended hand-off — *"decline here, adopt at the birth"* — did not happen either. Whether that is
the same defect as §3 or a second one is **not yet established**, and this document does not
claim it is.

⚠ `RING-NOT-A-SLICE=4` must therefore **not** be read as *"4 rings were not store slices."* It
means *"4 times nobody could answer."* The refusal is fail-closed and right; the number is not
about slices.

## 5. What to fix, in order

1. **Route the birth to the scratchpad when the USERD is `TheStore`.** A port verb, the way
   `StoreMapPort` already carries store-mapping escapes to the scratchpad's worker.
2. **Re-aim the gate.** `a_store_slice_userd_is_routed_to_b_before_anything_is_allocated`
   currently proves a property of `alloc_channel_in`. It must instead prove that the path the
   **guest's** birth takes reaches B — and its known-positive must be *"the per-proc isolate
   births it locally"*, which is the state measured here.
3. **Then re-boot**, and only then is `NEVER RETIRED` evidence about increment 6.

⊘ Nothing here says increment 6 is wrong. It says it is **unmeasured**, and that the harness
reported a failure of something that never ran.


---

## 6. ✔ THE ANSWER — w755u, and the gate moved

`[measured w755u — same box, tree `b4b72f54`]`

    STORE-BIRTH    asked=12 refused=0 born=12
    STORE-DOORBELL asked=13 refused=0 rung=13
    W755-ROW[P1 rm-invalidate]  ✔ VERIFIED over 8 round(s)
    W755-ROW[P2 uvm-memop]      ✔ VERIFIED over 8 round(s)
    W755-ROW[P3 rpc-bind]       ✔ VERIFIED over 8 round(s)
    MEAN_FALSIFIER=PASS
    ForeignHandle 20 → 1

**Every one of those three rows was `⊘ REFUSED … NEVER RETIRED` on every previous boot of this
campaign.** The guest's own copies now execute on the host engine, out of the guest's own ring,
against the guest's own USERD, on channels born in the per-proc birth client **B**.

### It took TWO crossings, not one, and the second was this document's own defect repeated

§3 named the missing delegation for the **birth**. Building it (w755r/w755s) gave `born=11` —
and **every doorbell on those channels was then refused**:

    ForeignHandle { handle: HostHandle(iso4294967295/gpu0:0xb1470006), worker_isolate: iso2 }

⊘ The same shape, one verb later: the channel now belonged to the scratchpad, while the
doorbell verb still ran on the per-proc worker. ⇒ **A delegation is not done when the thing it
creates is created; it is done when everything that NAMES that thing follows it.** This
document diagnosed the birth and did not ask what else held the handle.

⚠ And the order inside the doorbell fix is a ruling: **schedule, then ring**. The host-side
runlist submit is lazy, and a doorbell rung on a channel that is not on the runlist has its
submission **dropped silently** — no fault, guest waits forever.

### What remains, and §4 called it

`THREADS 0 of 8 ⊘ A WORKER CAME BACK DIRTY`. The eight worker lanes still report `NEVER
RETIRED`; `STALE RACE` now reports `★★★ CONTENT MISMATCH`, which is **progress** — the copy ran
and produced bytes, and a stale mapping was caught red-handed.

★ The cause is **§4 of this document, unchanged**: `RING-NOT-A-SLICE=4`, every one carrying
`oracle_installed=false`, and `oracle_installed=true` still **0**. The intended hand-off —
*"decline at the latch, adopt at the birth, which is where the oracle is in hand"* — has still
never happened, so those rings are refused, those channels are not born in B, and their threads
never retire. ⇒ **That is the next increment**, and it was named before the first boot of this
route.
