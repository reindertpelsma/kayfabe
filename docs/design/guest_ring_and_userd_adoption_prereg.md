# PRE-REGISTRATION — legs A and B: birthing the production GR host channel over the guest's ring and the guest's USERD

> ### STATUS — 2026-08-11 / **LIVE — PRE-REGISTRATION.** Written and committed **before any
> code on this rung**, amended in place only by appending measured outcomes below the line
> each prediction is registered on. Successor to `gr_doorbell_passthrough.md` (leg C, landed
> `b734995`); folds nothing, supersedes nothing.
>
> Branch `legs-a-and-b`, off `origin/hostgr-passthrough-server` = `b734995`.

---

## 0. The three-legged stool, and which legs this rung is

`RESUME_HERE_2026_08_11.md` §1 + `gr_doorbell_passthrough.md` §0.3 establish, **at the code**,
that making the host GPU execute the guest's GR work needs three independent things:

| leg | what it is | state entering this rung |
|---|---|---|
| **A — the RING** | the host GR channel is born over the **guest's** GPFIFO | verb `alloc_channel_over_guest_ring` exists (`w230`); **one caller, the R31 probe**. Production birth is `commit_engine_object → alloc_channel → alloc_channel_on → alloc_channel_at(.., None) → alloc_channel_in(.., RingSource::Ours(None))`. **NOT WIRED** |
| **B — the CURSOR** | `GP_PUT` is a word the **guest** advances, i.e. the guest's USERD handed to RM at channel creation | USERD is ours on every channel (`h_userd_memory_0: userd`, `rm.rs:4304`, allocated at `rm.rs:4174`). **NOT BUILT** |
| **C — the DOORBELL** | trap, translate guest token → host token, ring | **BUILT**, `b734995`, `KAYFABE_GR_ROUTE=passthrough` |

⇒ With only C, `GP_PUT == GP_GET` forever. That is not a prediction; it is
`alloc_channel_over_guest_ring`'s own doc comment (`rm.rs:4085-4089`).

---

## 1. ★★★ THE ORDERING FACT THIS RUNG IS SHAPED BY — read before the predictions

`GuestRing::memory` is a `HostHandle`. `alloc_channel_in`'s `RingSource::Guest` arm calls
`self.narrow(g.memory)?` (`rm.rs:4162`) — a validation that the handle was minted **by this
isolate's RM connection**. ⇒ **The host memory object over the guest's ring must EXIST before
the host channel is born.**

⚠ This does **not** contradict `guest_ring_adoption.md` §3.3, and the distinction is the whole
shape of this rung. §3.3 refuted the claim that the channel's birth must move because the ring
must be **bound** (measured: R31 arm C, `gpFifoOffset = 0xB_0000_0000` at an address nothing
was ever mapped at — **accepted**). It says nothing about the object needing to **exist**, and
`narrow` requires exactly that. ⇒ **Binding may be late. Minting may not.**

★ The same is true of leg B, one step harder: `h_userd_memory_0` is a handle too, and
`[measured 2026-08-11, R32/w233, real GA106 / 580.159.04]` **RM ZEROES all 512 bytes of a
caller-supplied USERD**. So the guest's USERD object must exist *and* be handed in at
**creation**, when zeroing is harmless because the guest has not yet written `GP_PUT`. Adopting
at the first doorbell wipes the cursor that caused the doorbell.

### 1.1 ⊘ WHERE THE OBJECT COMES FROM, and why the census cannot supply it

`RmBackend::join_fb_leaf` (`w260`, booted on a real GA106: 3 leaves `JOINED`,
`placed_as_asked=true`, both directions agreeing over 1024 words) mints exactly the object
`GuestRing::memory` wants — an `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over the isolate's own
mapping of the leaf's pages, placed FIXED at the leaf's own guest VA. **ONE memory.**

Its only production driver is `back_census_framebuffer_leaves(facts, &observed.census)`
(`shim.rs:4273`), and that call has **two** properties that disqualify it for a ring:

1. ⊘ **It is the OPERAND census** — the addresses the *methods* dereference.
   `[measured 2026-08-11, w260]` it joined FB phys `0x400000 / 0x600000 / 0x800000`; the GR
   ring lives at FB phys `0x1000000` (`guest_ring_adoption.md` §4, five boots, two independent
   resolvers) and is **never presented**, because *a ring is not an operand of the methods it
   carries*.
2. ⊘ **It runs at the DOORBELL**, and inside `if self.ce.watch.stats().declared > before` —
   i.e. behind a successful pushbuffer decode. That is far downstream of `commit_engine_object`,
   where the channel is born.

⇒ Leg A1 is *"give the join a second source: the channel's own `ring_va`"*, **and** it has to
run early enough. Both halves are this rung's, and the second one is the one the brief did not
name.

---

## 2. WHAT IS BUILT — declared before it is written

- **A1 — a second join source, the channel's own ring.** The channel's declared
  `gpFifoOffset` (and, for leg B, its declared USERD VA) are resolved to FB leaves through the
  **same** `WalkOperands::resolve_one` the operand census uses, and presented to the same
  `back_fb_leaf(.., FbLeafBacking::Joined)` verb. ⊘ **This is a page-table walk, not a ring
  parse** — nothing reads a GPFIFO entry, no pushbuffer is decoded, no method is classified.
  Opacity is preserved by construction, not by a check.
- **A2 — the birth path carries the adoption.** `RmBackend::alloc_channel` grows an additive
  `adopt: Option<GuestChannelBytes>` argument, in exactly the shape §16.106's `hosting` took:
  *an adapter that ignored it entirely would be exactly as correct as this trait was before*.
  The `HostRmBackend` lowers `Some(..)` to `alloc_channel_over_guest_ring`.
- **B — `hUserdMemory[0]` hand-in.** `alloc_channel_in` grows a `UserdSource` beside its
  `RingSource`, with the same three properties: no `alloc_device_local`, no `map_cpu`, and
  every access that would have used the mapping refused **by name** (`USERD_NOT_OURS`),
  mirroring `RING_NOT_OURS`.
- **Arming.** A sibling flag, **`KAYFABE_GUEST_RING`**, default `off`, byte-identical to
  `b734995`. ★ **Three arms, not two** (`falsifier_blocker_vs_only_blocker` — use 3 values):
  `off` / `ring` (leg A only, leg B still ours) / `ring-and-userd` (A **and** B). A value
  naming no arm **refuses to realize**; `on`/`1` are not spellings.

### 2.0 ⚠⚠ THE FALSE-GREEN THIS RUNG IS MOST LIKELY TO PRODUCE — two definitions, one name

`back_census_framebuffer_leaves` is defined **twice** in `shim.rs`: the real one at
`shim.rs:4331` under `#[cfg(feature = "host-isolates")]`, and an **empty-bodied** one at
`shim.rs:4742` under `#[cfg(not(feature = "host-isolates"))]`. ⇒ A build without that feature
runs the whole of leg A as a **silent no-op** and prints nothing at all. Anything added beside
the census inherits that by default.

★★★ **Same class as the `dlen=0` oracle rows and the zero-byte bench artefact: an empty
artefact reads as benign, and only inspecting its content distinguishes "nothing happened" from
"nothing was recorded".** The two outcomes this rung most needs to tell apart —
*"leg A never executed"* and *"leg A executed and changed nothing"* — look **identical** by
default, on every observable in §3.

⇒ **Registered as an obligation, not an intention:**
1. The armed path emits a **positive** signal of its own — a `GR-RING-JOIN` line on **every**
   armed pass, including the ones that join nothing, carrying the arm it read, the feature it
   compiled under, and the value of `KAYFABE_ISOLATES` **as actually read**, from the same
   invocation that acts on it (⊘ never from the build command line, never from absence of an
   error, and ⊘ never `$?` after a pipe — a count and a status must come from one invocation).
2. The `#[cfg(not(host-isolates))]` twin is **not** left empty: it prints that it is the
   stub. An unarmed build must be loud about being unarmed.
3. The boot's grading script asserts that line is present and non-zero **before** reading any
   number in §3.

### 2.1 ⚠ The owner invariant, and how it is made unrepresentable rather than checked

> *no fake framebuffer may ever be mapped to a real GPU VA of an isolate except the scratchpad.*

The guest's ring and USERD are in the **emulated** framebuffer. The only crossing this rung may
use is therefore `FbLeafBacking::Joined` — `JoinsGuestWindow`, ruling 4 — and never
`FbLeafBacking::Vidmem` (`ShadowsGuestMemory`, *"two memories"*, `w228`'s blank twin). ⇒ The
adoption carrier holds the join's own reply type and cannot be constructed from a `Vidmem`
backing; the forbidden state is not reachable through the type, in the style of `Binding`'s
private fields.

---

## 3. ★★★ THE PREDICTIONS — with numbers, registered before any boot

Arms: `off` (control, = every boot since `w260`), `ring`, `ring-and-userd`. All three with
`KAYFABE_GR_ROUTE=passthrough` and `KAYFABE_FB_JOIN=shared` and `KAYFABE_ISOLATES=real`.

| # | observable | `off` | `ring` | `ring-and-userd` |
|---|---|---|---|---|
| P1 | `CUP2_RC` | **124** | **124** | ★ **124** — see §3.1 |
| P2 | `Route::NotACopyEngineChannel` refusals | 0 (leg C already opened it) | 0 | 0 |
| P3 | `ring_doorbell` with a **GR** host token | > 0 | > 0 | > 0 |
| P4 | `GR-RING-JOIN` lines naming FB phys **`0x1000000`** | **0** | **≥ 1** | **≥ 1** |
| P5 | host GR channels born with `RingSource::Guest` | **0** | **≥ 1** | **≥ 1** |
| P6 | `GP_PUT` read out of the host channel's USERD | 0 (ours, never written) | 0 (ours) | ★ **> 0** — the guest's own cursor, if B lands |
| P7 | `GP_GET` on a GR channel advancing to meet `GP_PUT` | 0 | 0 | ★ **the one number that would be new** |
| P8 | `CE-SUBMIT → RETIRED` | 0 | 0 | 0 |
| P9 | `RmInitAdapter` failures | **0** | **0** | **0** |
| P10 | host `Xid` (31 `FAULT_PDE` in particular) | none | none | ⚠ **possible, and it would be GOOD NEWS** — §3.2 |
| P11 | CE doorbells / `ServedLocally` | unchanged | unchanged vs `off` | unchanged vs `off` |
| P12 | guest `dmesg` `NVRM` line count | 31 | 31 | 31 |

### 3.1 ⚠ P1 is predicted **124 on every arm**, and that is deliberate

`cup2` is a **CE** round-trip, not compute (`cup2_is_a_CE_roundtrip_not_compute`). Its wall is
`SET_REPORT_SEMAPHORE → 0x2_0440fff0`, `NOT-OBSERVED` 8× on every arm of `w260`. Nothing in
legs A or B touches the completion plane, and `AWAKEN_ENABLE = 0` means the guest **polls**.
⇒ Even a GR channel that genuinely executes does not, by itself, make `cup2` return 0.

★★★ **So `CUP2_RC` is NOT this rung's grade.** The grade is **P4, P5, P6, P7** — and P7 is the
only one that has never had a value. Registering P1 = 124 in advance is what stops a green-less
boot from being read as a refutation of the model, and what stops me from quietly re-grading
onto whatever moved.

### 3.2 ★ P10 — why an `Xid 31` would be the best available outcome

A host channel whose ring is the guest's, whose cursor is the guest's, and whose ring VA is
**bound in the host VAS** should fetch. If the VA is *not* bound — the failure mode the join
exists to prevent — hardware faults `Xid 31 FAULT_PDE`. ⇒ **A fault means the engine tried.**
Every prior boot's silence means it did not. An `Xid` here discriminates *"the transport is
live and the address plane is short"* from *"nothing ran"*, which is a discrimination no green
has ever supplied on this path.

⊘ It also means a **contained** guest fault, not a host one — `gpu_fault_is_contained`, measured:
a bystander context ran 2 675 519 verified iterations through a fault.

### 3.3 ⊘ WHAT A GREEN ON EVERY ROW STILL COULD NOT PROVE

Stated before the run, so no green can be read as more than it is.

1. **That the guest's methods executed correctly.** P7 (`GP_GET` advancing) says the host unit
   *consumed an entry*. It does not say the entry decoded, that the methods were the guest's,
   or that any operand resolved. `CE-SUBMIT → RETIRED` (P8, predicted 0) is a different plane.
2. **Anything about the completion plane.** The C **forges** completions and has no oracle here
   (`citing_the_c_where_it_forges`). Our observer *watches* and never writes. A green transport
   with a dead completion plane looks exactly like a green transport.
3. **That the ordering is right.** ★★★★★ `a_green_test_can_hold_a_wall_in_place`: when the
   install succeeds, bind-before and bind-after have **identical end states**. Leg B's
   *"adopt at creation, never at first doorbell"* is an **ordering** claim, and a boot in which
   nothing was zeroed at the wrong moment **cannot** distinguish it from a boot in which the
   ordering was wrong and got lucky. ⇒ Validating leg B's ordering needs **fault injection**,
   which this rung does not do.
4. **That the guest's USERD was not corrupted.** RM zeroes it at creation. If the guest wrote
   `GP_PUT` *before* the engine object alloc — unmeasured, and I do not know the answer —
   adoption at creation is *also* a wipe. ⚠ **This is a genuine unmeasured precondition of leg
   B, and I am registering it as unmeasured rather than assuming the safe order.**
5. **That the ring's leaf is joinable at engine-object time.** The GMMU walk needs the guest's
   page tables for the ring VA to be populated at that instant. Every committed measurement of
   that leaf was taken at **doorbell** time. If the walk refuses at engine-object time, P4 and
   P5 are **0** on both armed arms and the rung reports a *timing* wall rather than a plumbing
   one — which is a real finding and is why the refusal is named rather than silent.
6. **That anything is safe under a hostile guest.** Leg A points a host engine at bytes the
   guest writes at a rate it chooses. The boundary is the VA space and nothing else
   (`gr_execution_boundary.md` §2 — `LOAD_MME_INSTRUCTION_RAM` defeats any method allowlist).

### 3.4 ★ The bold prediction, so this pre-registration can be wrong

**P5 ≥ 1 and P7 = 0 on the `ring` arm, and P7 > 0 on `ring-and-userd`.** i.e. **the ring alone
is necessary and not sufficient, and the cursor is what completes it.** If P7 > 0 on the `ring`
arm, my model of who owns `GP_PUT` is wrong. If P7 = 0 on `ring-and-userd` **with** P4/P5/P6
all green, then all three legs are up and the stool still falls over — and *that* is the boot
that would genuinely indict the passthrough model, in the sense
`RESUME_HERE_2026_08_11.md` §3 asks for.

⊘ An all-green pre-registration is a warning that the predictions were not bold enough to be
wrong. P1 = 124 everywhere, P8 = 0 everywhere and P7 = 0 on two of three arms are here so that
this one can fail.

---

## 4. The tripwire that will go red, and how it is being updated

`crates/kayfabe-isolate-host/tests/guest_ring_census.rs:168` asserts
`alloc_channel_over_guest_ring` has exactly **ONE** caller. That is a deliberate tripwire, not a
bug: it exists so that the day a production caller appears, somebody has to say so out loud.

⇒ It is updated to assert **what is now true**: that the verb has exactly two callers, one of
which is the R31 probe and one of which is `HostRmBackend::alloc_channel`'s adoption arm — and
that the adoption arm is reachable **only** through a `GuestChannelBytes` the shell had to arm.
⊘ Not a bumped number.
