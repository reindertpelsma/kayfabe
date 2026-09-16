# w753 — route K phase 2: pre-registration

> ### STATUS — 2026-09-16 / **LIVE — PREDICTIONS COMMITTED BEFORE ANY BOX EXISTS.**
> Spec: `w750_route_k_phase2_plan.md` (the eight increments) and `THE_CONSTRAINTS.md`
> **constraint 32** (the ruling). ⊘ **No constraint is relaxed here and none is proposed
> relaxed.** Phase 1's four discriminators are ANSWERED (`w750_route_k_prereg.md`) and are
> **not re-measured**.

---

## 0. ★★★★★ THE THING I ESTABLISHED BEFORE WRITING A LINE, AND IT NARROWS THE JOB

The brief's gate is *"the raw client `(P)`, `THREADS 8 of 8` on the `device` arm."* Before
building anything I asked **what the device arm is actually red for**, because *"route K is
the fix"* was an inherited premise and not a measurement. The answer is committed and
byte-identical at **six** revisions (w740, w742, w743, w745, w746, w752):

```
ADOPT-WHY ⊘ (6) "the binding EXISTS but carries NO HOST OBJECT"   17
BIRTH-AT-ALLOC … REFUSED (FwdFault::PassthroughRingNotAdoptable)  11   ⇐ every channel
FwdFault::PassthroughDoorbellBirth                                19
BIRTH-ADOPTING                                                     0
⇒ the CE completion semaphore is never written ⇒ (R) THREADS 0 of 8
```
`[crates/kayfabe-fwd/src/lib.rs:4949-4956, :5499-5520; traces/w752_inplace_repoint/w752_run.log:565-569 (arm 2), :889-893 (arm 3)]`

★★★ **So the redness IS channel birth, and route K is on its causal path — but for ARM 3
ONLY, and through a link that is not the one constraint 32 headlines.** Constraint 32's
subject is the USERD/leg-B ruling; arm 3's *measured* wall is one step earlier:

```
STORE-MAP adopts=0 adopt_refused=4718 first_refusal=[Rm("InsufficientPermissions")]
          maps=0 slices_bound=0 asserted=0
```
`[w752_run.log:866-889; the same wall at w746, THE_CONSTRAINTS.md:199-216]`

RM's gate is `[ogkm-580.159.04 rs_client.c:537-551]`: a **cross-client** `NV_ESC_RM_DUP_OBJECT`
below `RS_PRIV_LEVEL_KERNEL` needs `RS_ACCESS_DUP_OBJECT` granted on the source. We grant
nothing, so the scratchpad cannot dup a per-proc isolate's VA space.

⇒ **Route K dissolves exactly this, by construction and not by a grant**: the dup's
*destination* becomes client **B**, which carries **I's `ProcessID`**, so RM's same-PID DUP
policy (`sharing.c:341-352`) applies and **no grant is needed at all**. Stated in the tree
before this session: `fable_leg_b_solution_space.md:227` — *"(This is also why w746's
`InsufficientPermissions` disappears for any dup whose destination has I's `ProcID`.)"*

### ⊘⊘ AND THE HALF OF THE CHAIN THAT IS STILL UNMEASURED, SAID NOW RATHER THAN AFTER

Getting past the dup does **not** by itself satisfy conjunct (6). Four links downstream of
it have **never run on hardware**, at three consecutive boots (`SINGLE_STORE_PLAN.md:2389-2398`):
`map_store_slice` · the slice binding · **constraint 28's placement assert on hardware** ·
the ring oracle. ⇒ **a green dup is not a green client**, and this pre-registration grades
them separately for exactly that reason.

### ⊘ ARM 2 IS RED BY DESIGN AND STAYS RED

`device + isolate` never reaches the block that binds a vidmem range at all
(`shim.rs:14360`, gated on `store_owns_vas()`). It is the **reproduction control**. A
session that "fixed" arm 2 would have destroyed the thing that makes arm 3's result
attributable.

---

## 1. What is being built — and the two invariants it does NOT cross

`w750_route_k_phase2_plan.md` §1 names four type-level invariants route K crosses. **Only
two of them are on the path to arm 3's measured wall**, and saying which is the difference
between an increment and a rewrite:

| plan § | invariant | crossed by the mapping plane? | why |
|---|---|---|---|
| 1.1 | `Worker::execute`'s foreign-handle gate | **NO** | `StoreMapPort` reaches RM through `SharedIsolate::with_worker` → `Worker::with_rm`, which takes a closure and never builds a `VerbPlan`. The gate is on `execute`. |
| 1.2 | `Staged::check_out` one-plan-one-worker | **NO** | same reason — the store-map path is not a staged plan. |
| 1.3 | `CrossedFd::lend_to` | **YES** | B's descriptor must cross I → VMM → S. |
| 1.4 | the request-direction fd path | **YES** | it must cross **downward**, which the tree says explicitly does not exist. |

⇒ **Increments 1–5 + the arm are the deliverable of this session.** Increments 6–7 of the
plan (the two-phase plan and the foreign-handle successor) are what **birth in B** costs, and
birth in B is only needed if the mapping plane alone does not clear conjunct (6).

★★★ **Per constraint 29 I therefore retire NOTHING I do not cross.** 1.1 and 1.2 are left
standing and un-restated on purpose: an assert retired *"because the design moved"* when the
design did not move past it is the deletion constraint 29 exists to refuse.

---

## 2. THE PREDICTIONS — each row with the value that REFUTES it

⚠ Rows 1–4 are **offline** and are settled before any box is rented. Rows 5–9 need the boot.

| # | prediction | refuted by |
|---|---|---|
| 1 | `HandedClient` is unspellable outside the scratchpad: `handed_client_is_unforgeable` passes, and breaking `ScratchpadRole::of` makes it RED | the test passing with the role neutered ⇒ it tests nothing |
| 2 | a birth-client fd lent to **any** per-proc isolate, its own minter included, is `ForeignDescriptor` | any of the four targets in the known-positive receiving it |
| 3 | the request-direction fd path carries a descriptor **down** and the child adopts it as a `CharDevice` | the child receiving `fds.len() == 0` ⇒ the kernel dropped it and the whole route is a no-op that returns `Ok` |
| 4 | the failing-test **name set** is identical to the baseline set collected on the same target dir with my changes stashed | any name added or removed |
| 5 | `KAYFABE_VAS_OWNER=k` **refuses rather than defaults** on a typo, exactly as `vas_owner_from` already does for its two values | a typo booting the `isolate` arm silently |
| 6 | ★★★ arm 3 (`device+K`): `adopt_refused` **0** and `adopts > 0` — the dup that RM refused 4718 times succeeds with **no grant issued** | `adopt_refused > 0`; especially a *new* first_refusal, which would mean K moved the wall rather than dissolving it |
| 7 | `maps > 0` and `slices_bound > 0` — the first time in four boots | `maps=0` with `adopts>0` ⇒ the dup was never the only blocker |
| 8 | constraint 28's placement assert **is asked** on hardware: `asserted > 0` | `asserted=0` with `maps>0` ⇒ the gate is not wired to the path it guards, i.e. another `census_zero_its_own_plumbing_guarantees` |
| 9 | ⚠ **NOT PREDICTED GREEN.** The client's verdict is genuinely open. `ADOPT-WHY (6)` **falls below 17** is the honest prediction; `(P) 8/8` is the hope | `(6)` still exactly 17 with `maps>0` ⇒ the binding is not what conjunct (6) reads, and the model of the chain is wrong |

★★★ **Row 9 is stated weakly ON PURPOSE.** `w743` drove `CpuCeFb` 63 → 0 and the client did
not move one thread; the brief names that as a trap already paid for. A pre-registration
that predicted `(P)` because `(P)` is the goal would be the same shape one rung along.

## 2b. ⊘⊘⊘ WHAT THIS SESSION DID **NOT** BUILD, AND WHY — said before the boot, not after

Three gaps. Each is named with the condition under which it stops being acceptable.

| gap | why it is not on this session's path | when it stops being acceptable |
|---|---|---|
| **Increment 6 — birth in B** (`Worker::execute`'s foreign-handle gate, `Staged::check_out`) | Neither is crossed by the mapping plane: `StoreMapPort` reaches RM through `Worker::with_rm`, a **closure**, never a `VerbPlan`. Per constraint 29, an assert that is not crossed is not retired. | the moment conjunct (6) still refuses **after** `maps > 0`. That is the measurement that says the channel must be born in B too. |
| **Birth-client teardown** — nothing removes a `BirthConn`; two descriptors per guest process stay open and **client B outlives isolate I**, because RM frees a client when its last `struct file` closes and we hold that file | the grading boot is one guest process and a few minutes | the LLM lane, or any run with many short-lived guest processes. Counter: `birth_clients_outstanding`. ⚠ The reap must run **after** every range placed through the connection is gone — the descriptors are what keep those objects reachable for the unmap constraint 27's barrier waits on. |
| **A known-positive for `ProxyRmBackend::adopt_birth_client`'s scratchpad refusal** | it lives behind a live proxy with no offline harness | never blocked: the **same question** is asked at the receiver, where it *is* testable (`a_per_proc_isolate_refuses_a_birth_client_offered_to_it`), and the two are not redundant — one protects against a VMM bug, the other against the socket. |

⚠ **The second row is the one to watch.** It is the shape this tree calls *"correct by accident
under a temporary condition"*: it is right for the boot that grades it and wrong for the
workload that follows.

## 3. Non-regression, pre-registered as a list rather than as a habit

BAR1/BAR2 `TRAP_FILLS=0` **and** `misses=0` · `named` ≈ 312k · `RmInitAdapter failed!`=0
**asserted against a non-empty dmesg** · `SMI_RC=0` · control arm `(P)` 8/8 · F11 green **on
its merits** (not by the scan's universe shrinking) · failing-test name set == baseline ·
`VCPU-BLOCKING doors=6 total=102` not worsened.

⊘ **A reduced refusal count is not a pass**, and §2 row 6 is deliberately not the gate.
