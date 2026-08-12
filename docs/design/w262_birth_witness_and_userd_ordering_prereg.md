# PRE-REGISTRATION w262 — witness the birth, and measure the leg-B ordering

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTRATION.** Written and committed **before any
> code on this rung and before any boot**. Amended only by appending measured outcomes below
> the prediction they are graded against.
>
> Branch `witness-birth-and-userd-ordering`, off `origin/legs-a-and-b` = `00c3e28`.
>
> Successor to `guest_ring_and_userd_adoption_prereg.md` (legs A+B) and
> `leg_b_userd_adoption_blocker.md`. Supersedes neither; ⊘ but §1.2 below **contradicts** the
> second one's §2.2, and §1.3 contradicts the first one's §4, and both are recorded here rather
> than only in a report.

---

## 0. What this rung is, in one line each

1. **ITEM 1 — the birth witness.** `w261` booted leg A and its own `RESULT.md` leads with the
   hole: *"nothing prints that a channel was born with `RingSource::Guest`"*. Zero
   `RING_NOT_A_JOINED_WINDOW` refusals is consistent with `adopt: Some` succeeding **and** with
   `adopt: None` never being asked. ⇒ We do not know whether A2 fired.
2. **ITEM 2 — the leg-B ordering.** `leg_b_userd_adoption_blocker.md` §2.3 registers as
   unmeasured: *does the guest write `GP_PUT` before or after its first engine-object alloc?*
   If before, adoption-at-creation is a wipe and leg B has **no safe moment** under the current
   birth ordering.

⊘ **Neither leg-B route is built on this rung**, and the predecessor's refusal to add a
`UserdSource::Guest(handle)` arm stands — see §1.2 for the one place I think its *reasoning* is
stale, which is not the same as thinking the *decision* was wrong.

---

## 1. ★★★ THREE THINGS IN THE BRIEF AND ITS SOURCE DOCS THAT ARE WRONG, MEASURED BEFORE ANY CODE

### 1.1 ★★★★★ ITEM 2 CANNOT BE MEASURED FROM ANY EXISTING LOG, AND A NAIVE READING GIVES A CONFIDENT WRONG ANSWER

The brief says *"Both events already print on paths we own — the BAR1 trap (`BAR1[2] WRITE
off=0xa008c` …) and the engine-object alloc"*, and concludes *"one armed boot's log ordering
answers it"* (`leg_b_userd_adoption_blocker.md` §2.3 says the same). **The first half is true and
the second half does not follow.**

`[measured 2026-08-12, source: `qemu/hw/misc/nvkvm/nvkvm.c:604-616` + `:1961-1978`]` the BAR1
access rows are **not** printed when the access happens. `nvkvm_bar1_record` stores the first
**16** accesses into a fixed array (`NVKVM_BAR1_LOG = 16`) and **prints nothing**; the whole
array is dumped once, from the device's teardown report. ⇒ the timestamp on a `BAR1[2] WRITE
off=0xa008c` line is the **dump's** time, and its position in the file is *after everything*.

`[measured, `traces/boots/w261/run_w261_ring_qemu.log`]`:

| event | file line | timestamp |
|---|---|---|
| `ENGINE-OBJECT class=0xc7c0 … materialized_channel=true` (the first GR birth) | 75 | between `05:43:36.825391` and `05:43:36.898985` (nearest stamped neighbours) |
| `BAR1 access log: 16 of 483 …` + `BAR1[2] WRITE off=0xa008c` | 634-637 | **`05:46:34.672864`** |

⇒ Reading the order off the file says *"`GP_PUT` happened 178 seconds after the alloc"*. It says
that on **every** boot, unconditionally, whatever the truth is, because 178 s is the distance to
teardown and not a fact about the guest. ⊘ **This is the `dlen=0` shape again**: the row exists,
is well-formed, carries a plausible timestamp, and is evidence of nothing. Had this rung "just
grepped the logs", it would have reported the *favourable* order — `GP_PUT` after the alloc,
adoption-at-creation is safe — with a citation, and been wrong by construction.

⇒ **Item 2 requires an instrument that does not exist.** It is built here (§2.2).

### 1.2 ★★★★ THE RECORDED `0xa008c` WRITES ARE NOT THE CHANNEL LEG B WOULD ADOPT — and the tree's own comment says so

The brief treats `off=0xa008c` as *"`GP_PUT` at USERD+0x8c"* full stop. `[measured]` the four
recorded write-pairs in every boot are

```text
BAR1[0..2]   off=0x90000 / 0x90004 / 0xa008c     BAR1[3..5]   off=0xb0000 / 0xb0004 / 0xc008c
BAR1[6..8]   off=0xd0000 / 0xd0004 / 0xe008c     BAR1[9..11]  off=0xf0000 / 0xf0004 / 0x10008c
```

— four channels, ring and USERD `0x10000` apart, channels `0x20000` apart. `nvkvm.c:625-632`
attributes them, from the driver's source, to `internal_channel_submit_work`
(`ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:984-1015`) — i.e. **nvidia-uvm's own internal
channel pool**.

The channel leg B is about is the one the campaign walls on: client `0xc1d0000c`, and
`[measured, `w261` log]` its eight GR channels declare `userd=h0x5c000014/off0x2000`, `0x5000`,
`0x8000` … — **one memory object, stride `0x3000`**. The recorded BAR1 GP_PUT writes are stride
`0x20000`. ⇒ **different channels.**

⚠ Stated as an attribution, not a join: `leg_b_userd_adoption_blocker.md` §2.2's *"nothing joins
a BAR1 offset to a CHANNEL"* is exactly why this cannot be closed, and this rung does **not**
build that join (it would be reverse-resolution by address, which `kayfabe-mmu`'s `gpga.rs`
forbids). ⇒ P4 below is registered accordingly: what a live witness can establish is *when the
guest first advances a cursor at all*, not *when that channel does*.

### 1.3 ★★★ THE TRIPWIRE THE BRIEF WARNED ME ABOUT IS WORSE THAN THE BRIEF SAYS — the promised fix was never made

The brief says `guest_ring_census.rs:168` *"reads 'exactly one caller, the R31 probe' and asserts
a **definition** count"*. Correct. What the brief does not say is that
`guest_ring_and_userd_adoption_prereg.md` **§4** already committed to fixing it:

> *"⇒ It is updated to assert **what is now true**: that the verb has exactly two callers … ⊘ Not
> a bumped number."*

`[measured at `00c3e28`]` it was **not** updated. The doc comment still reads *"currently has
exactly one caller, the R31 probe, and that is honest — the rung builds the alloc side and
nothing consumes it"*, on a revision where `rm.rs:3437` **is** the second caller. ⇒ the tripwire
that existed *"so that the day a production caller appears, somebody has to say so out loud"* let
that day pass in silence, and the pre-registration that promised to make it speak was graded
green without that line being delivered. ★ This is `a_correct_default_is_not_a_handoff` and
`a_green_test_can_hold_a_wall_in_place` in one artefact. Fixed on this rung (§2.3).

---

## 2. WHAT IS BUILT — declared before it is written

### 2.1 ITEM 1 — `GR-BIRTH`, at the lowering site, THREE STATES

One line per host-channel birth, from `HostRmBackend::alloc_channel`
(`crates/kayfabe-isolate-host/src/rm.rs`) — the site where `adopt: Some` becomes
`alloc_channel_over_guest_ring`. It is the isolate **child**'s stderr, which is QEMU's stderr,
which `boot_capture.sh` redirects to `run_<tag>_qemu.log` — so the evidence is a file, not a
session transcript (`rm.rs:3683-3686` already documents this for `CE-SUBMIT`).

★★★ **The three states are read off values already at the site, with NO new wire field and no
second source of truth.** The discriminator is the `hosting` argument, which already crosses
(`proto.rs:98`: *"`(class, params)` … or `None` for a doorbell materialization"*):

| `hosting` | `adopt` | state | what it means |
|---|---|---|---|
| any | `Some(r)` | **`GUEST-RING`** | asked, and the plan produced the guest's ring. **This is A2 firing.** |
| `Some(..)` | `None` | **`DECLINED`** | the engine-object birth path ran `adopted_guest_ring` and it produced nothing |
| `None` | `None` | **`NOT-ASKED`** | a doorbell materialization — `kayfabe-isolate/src/lib.rs:2703` passes a literal `None, None` and its own comment says a ring adopted there would be adopted without a join |

⚠ The `DECLINED` reading depends on an invariant — *an engine-object birth always consults* —
that lives in another crate (`kayfabe-fwd`, `plan_engine_object`'s `adopt:` field, consulted
unconditionally on the `channel.is_none()` branch). **A comment cannot hold it.** So the
invariant is pinned by a source census test, in the style of `guest_ring_census.rs`, that fails
if either birth site stops matching the table above.

⊘ **It prints; it decides nothing.** No branch reads it, no refusal is gated on it, no ring byte
is read and no method is decoded. Opacity is unchanged.

★ **The known-positive, because a census zero needs one.** The loopback backend
(`loopback.rs:283`) takes `_adopt` and **discards it silently** today — a real silent no-op on a
path that is selected by `KAYFABE_ISOLATES` and looks identical to a real isolate that adopted
nothing. It is made loud: it prints `GR-BIRTH … ⊘ LOOPBACK-BACKEND: … DISCARDED`. ⇒ *"no
`GR-BIRTH` line at all"* stops being reachable through the plane selector.

### 2.2 ITEM 2 — the live `GP_PUT` witness, in the C

`qemu/hw/misc/nvkvm/nvkvm.c`: at the instant of a BAR1 **write** whose page offset is `0x8c`
(`kayfabe_abi::submit::USERD_GP_PUT`), one `info_report` — so it carries **QEMU's own `-msg`
timestamp**, on a timeline the device under test does not write, and it interleaves with the
`kayfabe: ENGINE-OBJECT …` lines in file order. That is precisely the mechanism
`kayfabe_shim.h:325-341` already argues for the doorbell: *"a doorbell is a property of ONE
WRITE … so the shell logs it as it happens, against QEMU's own `-msg` timestamp … A per-boot
counter cannot be stamped."*

Bounded (first 8 printed) with an **unbounded** total reported at teardown, so a printed count is
never mistakable for a total — `engine_fwd_report_action`'s lesson, one aperture over.

⊘ It reads the offset and the value of a write the trap already handles, and prints them. The
teardown dump already prints exactly these bytes; this only prints them **when they happen**.

### 2.3 The tripwire, actually updated

`guest_ring_census.rs`'s prose and its assertion are brought into agreement: it asserts the
verb's **caller** count (2: the R31 probe and `alloc_channel`'s adoption arm), and asserts that
the production caller sits behind the `RING_NOT_A_JOINED_WINDOW` gate. ⊘ Not a bumped number.

---

## 3. ★★★ THE PREDICTIONS — registered before the boot

Two arms, one boot each, strictly serial. **Arm `off`** = `KAYFABE_GUEST_RING` unset;
**arm `ring`** = `KAYFABE_GUEST_RING=ring`. Everything else exactly `w261`'s environment
(`KAYFABE_GR_ROUTE=passthrough KAYFABE_FB_JOIN=shared KAYFABE_ISOLATES=real
KAYFABE_CE_EXECUTOR=local NVKVM_RAM_BACKEND=memfd KAYFABE_GUEST_RAM=memfd KAYFABE_RING_VIDMEM=1
KAYFABE_PT_WITNESS_EXEC=on KAYFABE_FB_BACKING=on`).

| # | observable | `off` | `ring` |
|---|---|---|---|
| **Q1** | `CUP2_RC` | **124** | **124** |
| **Q2** | `GR-BIRTH` lines, total | **> 0** | **> 0** |
| **Q3** | `GR-BIRTH … adopt=GUEST-RING` | **0** | ★ **≥ 1** — *this is the line that proves A2 fired* |
| **Q4** | `GR-BIRTH … adopt=DECLINED` | **> 0** | ≥ 0 |
| **Q5** | `GR-BIRTH … adopt=NOT-ASKED` | ≥ 0 | ≥ 0 |
| **Q6** | `GR-BIRTH … REFUSED RING_NOT_A_JOINED_WINDOW` | **0** | **0** |
| **Q7** | `GR-RING-JOIN … fb_phys=0x1000000` | **0** | **24** (`w261`'s number, exactly) |
| **Q8** | `BAR1 GP_PUT` live lines | **> 0** | **> 0**, and the same offsets on both arms |
| **Q9** | first `BAR1 GP_PUT` **vs** first `ENGINE-OBJECT … materialized_channel=true` | ★ **`GP_PUT` LATER** | ★ **`GP_PUT` LATER** |
| Q10 | `RmInitAdapter` failures | 0 | 0 |
| Q11 | host `Xid` | 0 | 0 |
| Q12 | guest `NVRM` dmesg lines | 31 | 31 |
| Q13 | `CE-SUBMIT → RETIRED` | 0 | 0 |

### 3.1 ⚠ Q1 IS PREDICTED **124 ON BOTH ARMS**, AND THE EXPECTED MOVEMENT IS **ZERO**

Leg B is not built. The stool has two legs of three (`the_wall_is_a_three_legged_stool`). Nothing
on this rung touches the cursor or the completion plane, and this rung **adds no forward path at
all** — every line of item 1 is a print and every line of item 2 is a print. ⇒ **The predicted
movement in `CUP2_RC` is `0`, i.e. `124 → 124` on both arms, and `124` is the *expected* result,
not a disappointment.** A boot that does not move the number is **not** evidence against the
passthrough model; a boot that *did* move it would mean a print changed behaviour, which would
indict the instrument rather than vindicate the model.

### 3.2 ★ Q9 IS THE BOLD ONE, and it is registered so it can be wrong

**I predict the FAVOURABLE order: the guest's first `GP_PUT` store comes AFTER the first host GR
channel is born.** The reasoning, stated so it is falsifiable: the recorded stores belong to
nvidia-uvm's internal channel pool (§1.2), and `UVM_REGISTER_GPU` builds that pool inside
`cuInit` — *after* RM channel allocation, which is when the engine-object forward fires.

⇒ If Q9 comes back **`GP_PUT` EARLIER**, then adoption-at-creation is a wipe for at least one
channel and leg B has no safe moment under the current birth ordering — which is a **more
valuable** result than the prediction holding, and is the outcome this instrument exists to be
able to report.

⊘ **And Q9 is weaker than the question leg B needs, whichever way it lands.** It orders *the
guest's first cursor advance anywhere* against *the first GR birth*. It does **not** order **the
GR channel's own** `GP_PUT` against **its own** birth, because nothing joins a BAR1 offset to a
channel (§1.2). Registered now so no result can be read as more than it is.

### 3.3 ⊘ WHAT THIS BOOT CANNOT PROVE, WHATEVER IT SAYS

Stated before the run.

1. **That the adopted ring is fetched.** `adopt=GUEST-RING` says RM was **told** the guest's
   ring. `admitted_and_served_are_different_gates`. Nothing here reads `GP_GET`.
2. **That leg B's ordering claim is right.** Even a favourable Q9 is an ordering *observation*,
   not a validation: `a_green_test_can_hold_a_wall_in_place` — a boot in which nothing was
   zeroed at the wrong moment cannot distinguish "the ordering is right" from "the ordering is
   wrong and got lucky". That needs fault injection, which this rung does not do.
3. **That the `DECLINED`/`NOT-ASKED` split is complete.** It is a two-crate invariant pinned by a
   source census. A third birth site added elsewhere would be mis-labelled until the census
   catches it — which is why the census exists, and it is a *source* census, not a runtime one.
4. **Anything about the completion plane.** Unchanged from every prior rung.
5. **That `off` and `ring` differ anywhere but Q3/Q7.** They should not. If any other row moves,
   the instrument is not neutral and the arm comparison is void.

### 3.4 ⚠ The disarmed/armed differential, named in advance

> **The line that proves A2 fired is `kayfabe-isolate: GR-BIRTH #… adopt=GUEST-RING memory=…
> ring_va=… gp_fifo_va=… entries=… → alloc_channel_over_guest_ring`, present on the `ring` arm
> and absent on the `off` arm.**
>
> **The line that proves the witness itself ran on the `off` arm is `… adopt=DECLINED`** — a
> positive statement that the armed path was consulted and produced nothing, which is what makes
> the `off` arm's zero in Q3 a *measured* zero rather than an absence.

⊘ If the `off` arm prints **no `GR-BIRTH` line at all**, the correct reading is *"the witness did
not run"*, not *"nothing was born"* — and Q2 exists to force that reading.
