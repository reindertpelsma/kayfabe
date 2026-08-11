# The GR doorbell passthrough route — what it is, and the two things it cannot be

> ### STATUS — 2026-08-11 / **LIVE**. Written as a PRE-REGISTRATION, before any code on this
> rung, and amended in place as the code landed. Supersedes nothing; folds two earlier
> findings in below (`guest_ring_adoption.md` §4, `gr_execution_boundary.md` §4.1) rather
> than restating them beside their parents.

---

## 0. ★★★★★ LEAD — three of the commissioning brief's claims are REFUTED, at the code

The rung was commissioned as *"`DoorbellRoute::HostGr` has zero consumers; give it a
passthrough server: trap the doorbell write, look up the channel by guest token, ring the
host token, return to VM entry — and the host GPU then fetches the guest's own ring."*

Three parts of that are false, and the third is the expensive one.

### 0.1 ⊘ The server EXISTS and is the SAME ONE the CE path uses. What was missing is a
### route arm, not a server.

`kayfabe_rt::device::SharedDevice::doorbell` (`crates/kayfabe-rt/src/device.rs`) already
*is* the passthrough server: it routes the guest token, plans, materializes/schedules the
host channel, and calls `RmBackend::ring_doorbell(host_token)` — the tree's **only** call
site (`crates/kayfabe-isolate/src/lib.rs`, `VerbPlan::Doorbell` arm). Nothing in that chain
is copy-engine-specific; `VerbPlan::gated_doorbell` takes the channel's `EngineKind` and
passes it to `alloc_channel`.

★ It is **already demonstrably reachable by a `GrCompute` channel**: the opacity pin
(`tests/tests/doorbell_is_forwarded_without_reading_the_ring.rs`, landed at `b9025b4`)
drives exactly that and asserts `ring_doorbell` is reached once.

⇒ The whole of the missing production wiring is **one arm in one `if`** at
`crates/kayfabe-qemu-raw/src/shim.rs` (`SharedDoorbell::try_ce_submission`), where
`if route != DoorbellRoute::CpuCe` refuses `HostGr` identically to `Unserved`.

### 0.2 ⊘ That arm was not an omission. It was CLOSED ON PURPOSE, at §16.65, with the
### reason recorded at the site.

The comment above the refusal says it, and it is the same sentence
`execution_plane_increments.md` §15.5 uses:

> *"With a real forwarding plane (`KAYFABE_ISOLATES` set) a `GrCompute` channel with a
> `vas_pdb` used to return `None` here and fall through to `SharedDevice::doorbell`. It is
> now refused by name instead. That is deliberate: §15.5's own words for what that
> fall-through achieved are* **"we rang a doorbell on a host channel into which the guest's
> methods were never copied"**."

So the GR doorbell **used to be forwarded**, was measured to be a no-op, and was replaced by
a true refusal on the principle *"a true refusal outranks a forwarded no-op."*

⇒ ★★★ **This rung re-opens a path that was closed on evidence.** That does not make the
owner's ruling wrong — a refusal is also a wall, and the standing debt in
`RESUME_HERE_2026_08_11.md` §3 asks for a boot where the doorbell **is** routed. But it does
mean the re-opening must be a **deliberate, armed, printed choice with a control arm**, not
a silent flip, and that whoever reads the boot must already know what §0.3 says.

### 0.3 ★★★★★ THE ONE THAT MATTERS — forwarding the doorbell CANNOT make the host GPU
### fetch the guest's ring. Two independent reasons, both in the code, neither is a guess.

The host GR channel that the doorbell rings is born on the engine-object path through
`RmBackend::alloc_channel` → `HostRmBackend::alloc_channel_on` → `alloc_channel_at(vas, ty,
None)` → `alloc_channel_in(range, ty, RingSource::Ours(None))`
(`crates/kayfabe-isolate-host/src/rm.rs`). In that arm:

1. **The ring is OURS.** `gp_fifo_offset` is `layout.gp_fifo_va` = `va + GPFIFO_OFFSET`,
   where `va` is a **host** ring object RM placed in the host VAS. The guest's ring
   (`0x2_0020_0000`, 1024 entries) is not named anywhere in that channel's declaration.
   The verb that *would* name it — `alloc_channel_over_guest_ring`
   (`RingSource::Guest`) — exists, is hardware-verified (R31/`w230`), and has **exactly one
   caller: the `kayfabe-rm-ladder` probe.**
2. **The cursor is OURS.** `GP_PUT` lives in the USERD *we* handed RM, at
   `USERD_GP_PUT`, and the **only** writer in the tree is
   `HostRmBackend::submit_entry`, which refuses outright when the ring is not ours
   (`RING_NOT_OURS`). Nothing writes the guest's `GP_PUT` into it. That is exactly the
   "cursor bridge (G8)" `guest_ring_adoption.md` §4 names as unbuilt.

`alloc_channel_over_guest_ring`'s own doc already states the end state, and it is the end
state a routed GR doorbell reaches today:

> *"RM will have accepted it, `RmBackend::schedule` will make it eligible, and it will still
> execute **nothing**, because the engine reads `GP_PUT` out of the USERD *we* gave it and
> nothing on this rung writes the guest's cursor into that word."*

⇒ **`GP_PUT == GP_GET` forever ⇒ the engine fetches nothing and reports no error.**

---

## 1. ★★★ PRE-REGISTERED PREDICTION — read this BEFORE the boot that follows this rung

`RESUME_HERE_2026_08_11.md` §3 records a standing debt in these words:

> *"the passthrough model now owes a boot where the doorbell **is** routed. **If that one
> also moves by one step, the model itself is what to doubt.**"*

★★★★★ **Registered here, before the boot: it will move by ZERO steps, and that must not be
read as doubting the model.** `CUP2_RC` will still be `124` and
`SET_REPORT_SEMAPHORE → 0x2_0440fff0` will still be `NOT-OBSERVED`, for §0.3's reason —
which is a fact about *which ring the host channel was born over*, not about whether
doorbell passthrough is the right architecture.

What the boot **can** move, and what to grade it on:

| observable | prediction | why it is worth the boot |
|---|---|---|
| `Route::NotACopyEngineChannel` refusals | **→ 0** on the armed arm | the wall named in the handoff is gone |
| `ring_doorbell` calls with a **GR** host token | **> 0**, first time ever | the passthrough transport is live end to end |
| `RmInitAdapter` failures | **0** (unchanged) | re-opening the route must not regress `nvidia-smi` |
| CE doorbells / `ServedLocally` | **unchanged vs the control arm** | the change is additive to one route |
| `CUP2_RC` | **124**, unchanged | §0.3 |
| host `Xid` | **none** | a channel whose `GP_PUT` never moves fetches nothing; it does not fault |

⇒ **The discriminator that would genuinely doubt the model is a boot in which the doorbell
is routed AND the ring source AND the cursor are both the guest's, and it still does not
move.** That boot is three rungs away, not one.

---

## 2. What is actually built here

**One route arm, one flag, and a refusal that stops being wrong for the whole class.**

- `kayfabe_rt::device::ShellDisposition` + `shell_disposition(route, gr_passthrough)` — the
  **pure half** of the decision, beside `route_of_engine` and for its stated reason
  (*"the half that can be quantified over should be"*). Exhaustive over `DoorbellRoute`, so
  a new route variant fails the build until somebody says which executor owns it.
- `KAYFABE_GR_ROUTE` — `refuse` (**default**, byte-identical to today) / `passthrough`.
  A value naming no arm is **refused, not defaulted**, for `KAYFABE_FB_JOIN`'s reason: a typo
  that silently disarmed the route would make an evidence run and its own control
  indistinguishable. Printed on **every** arm, including the default.
- `kayfabe_fwd::DoorbellOutcome::engine` — carried out of `commit_doorbell`, so
  `SharedDevice::doorbell` can decide whether the **CE content-forward** applies without a
  second lock and without a second resolution of one fact.

### 2.1 ⚠ `forward_ring` MUST NOT run on the GR arm, and that is the ruling, not an
### optimisation

`SharedDevice::doorbell` ends with `forward_ring`, which parses the guest's ring with the
**copy-engine** codec and plans `ce_copy`s. On a GR channel every span decodes `Opaque`
(the codec is class-gated), so it forwards nothing — **but it can return `Err`, and
`doorbell` propagates that**. A GR doorbell that rang the host channel successfully would
then be reported `Refused`.

That is precisely the failure mode the rung brief forbids:

> *"Ring resolution / pushbuffer reads / method decode are DEBUG: flag-gated, non-fatal, and
> they must never gate whether the doorbell is forwarded."*

⇒ The content-forward is asked for by **engine**, through the same authority as the route:
`ring_content_is_forwardable(engine)` is `route_of_engine(engine) == CpuCe`. One rule, two
callers, no second table to drift. ⊘ **Not** by passing `vmm: None` from the shim — that
parameter's own doc names that as `a_fallback_keyed_on_our_own_ignorance` waiting to happen.

### 2.2 ⚠ The #14 ring gate is VACUOUS, not bypassed — and it already was

The brief asked for this to be made *"a deliberate, documented, tested choice."* ⊘ It was
already all three before this rung: the only production supplier passes `&[]`
(`shim.rs`), `SharedDevice::doorbell`'s own doc states the choice and its reason, and
`doorbell_is_forwarded_without_reading_the_ring.rs` arm 2 pins it **vacuous on an empty
working set and LIVE on a non-empty one**, watched red in both directions. Nothing on this
rung touches the gate; the GR arm inherits the same `&[]` the CE arm passes, from the same
call site.

---

## 3. ★★★ THE TOKEN VERDICT — settled by code: it is a FIELD READ, not even a map lookup

The brief: *"A C-era doc records three guest tokens with one matching no host token. My
inferred reading: that is a symptom of no host GR channel existing at the time."*

**MEASURED (by construction, from the code — no boot needed). The inferred reading is
correct, and the mechanism is narrower than 'a map lookup'.**

Guest token → host token is two hops and neither is arithmetic:

1. `kayfabe_fwd::route_doorbell` — `Arch::decode_doorbell(token)` → `VChid`, then
   `spine.by_vchid: BTreeMap<(GpuId, VChid), (ProcId, ChanId)>`. This is the only map.
2. `kayfabe_fwd::plan_doorbell` — `proc.channels.get(&cid)` then
   **`chan.host_channel.zip(chan.host_token)`**. `Channel::host_token: Option<u64>`
   (`crates/kayfabe-core/src/gpu.rs`) is a **plain field on the channel the guest token
   already routed to.**

`host_token` is `None` at channel construction and is written in exactly two places, both
`commit`s: `commit_doorbell` (lazy materialization on the first doorbell) and
**`commit_engine_object`** (the engine-object path). The latter is the one that matters:
the 8 host GR channels materialised per boot are made there, *upstream in time of the
doorbell*, and they write `chan.host_token = Some(htok)` on the same `Channel` record the
doorbell will read.

⇒ ★★★ **"A guest token that matches no host token" is not a translation defect and never
was. It is the `Option` being `None`** — i.e. *"this channel has not been materialized on
the host yet"* — and `plan_doorbell` handles it by materializing, not by failing. There is
nothing to generalise from the CE path, because **the CE path has no GR-specific
translation to generalise**: `plan_doorbell` never reads the engine to find the token.

★ Pinned as a test rather than as this paragraph:
`tests/tests/gr_doorbell_token_is_the_channels_own_field.rs`.

---

## 4. ⊘ WHAT A GREEN BOOT AFTER THIS RUNG STILL CANNOT PROVE

Stated before the boot, so a green cannot be read as more than it is.

1. **That any guest work executed.** §0.3. The host channel's ring and cursor are both
   ours. `CE-SUBMIT → RETIRED` has never printed and will not print here.
2. **That the guest's ring is reachable by the host engine.** That needs
   `alloc_channel_over_guest_ring` on a guest path, which needs a **host object over the
   guest's ring**. `b9025b4` measured the GR ring to be **in the emulated framebuffer**
   (FB phys `0x1000000`), and at that revision no such object could be minted. ⚠ The
   **w260 FB-leaf JOIN landed after that measurement** (`3159bfb`, 17:13, vs `b9025b4`,
   14:36) and may have dissolved exactly that blocker — **that is INFERRED and not
   measured, and it is the highest-value single question left on this path.**
3. **That the cursor bridge works.** G8. `b9025b4` measured the producer index is already
   trapped (`BAR1[2] WRITE off=0xa008c val=0x1` is `GP_PUT=1` at `USERD+0x8c`); the missing
   piece is the join from a BAR1 offset to a channel.
4. **Anything about ordering, completions, or interrupts.** The completion plane is
   untouched; `AWAKEN_ENABLE = 0` means the guest polls, so no vector is owed.
5. **That the refusal removal is safe under a hostile guest.** The GR arm now reaches
   `plan_doorbell`, whose #14 gate is vacuous on `&[]`. Nothing here widens what a guest can
   address — the channel, its VAS and its host handles are all pre-existing — but it does
   let a guest cause `ring_doorbell` on a GR channel at a rate it chooses. That is the same
   exposure the CE route already has.
