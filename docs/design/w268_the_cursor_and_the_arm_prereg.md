# w268 PRE-REGISTRATION — **READ `GP_GET`.** Committed before one line of the instrument exists

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTRATION.** Branch `w268-the-cursor-and-the-arm`,
> off `w267-read-the-page` = `3ba9e38`. Predecessor: `traces/boots/w267/RESULT.md` (rev
> `b129770`, 2 arms, real GA106). Bench `vh` = `NVIDIA GeForce RTX 3060` (GA106), host driver
> `580.159.04`.

---

## 0. ⊘⊘ WHAT CONTRADICTS THE BRIEF — three things, all at the code, before any boot

### 0.1 ★★★★★ THE STATED REASON THE GR ROUTE IS DISARMED WAS TRUE ON 2026-08-11 AND IS FALSE ON 2026-08-12

`docs/design/gr_doorbell_passthrough.md` §0.3 is the document that keeps
`KAYFABE_GR_ROUTE` at `refuse`. It gives **two** reasons, and calls them *"both in the code,
neither is a guess"*:

> 1. **The ring is OURS.** … The verb that *would* name [the guest's ring] —
>    `alloc_channel_over_guest_ring` … — exists and has **exactly one caller: the
>    `kayfabe-rm-ladder` probe.**
> 2. **The cursor is OURS.** `GP_PUT` lives in the USERD *we* handed RM … Nothing writes the
>    guest's `GP_PUT` into it.

⊘⊘ **Both are refuted by `w267`'s own committed log**, which is the boot the brief hands me.
`[measured, traces/boots/w267/run_w267_on_qemu.log, 16 `GR-BIRTH iso2` lines, all of them]`:

```
8 × engine=Ce        adopt=GUEST-RING userd=GUEST-USERD  → alloc_channel_over_guest_ring
8 × engine=GrCompute adopt=GUEST-RING userd=GUEST-USERD  → alloc_channel_over_guest_ring
```

⇒ `alloc_channel_over_guest_ring` has **sixteen production callers per boot**, eight of them
`GrCompute`; the host GR channel's ring **is** the guest's (`gp_fifo_va=0x200200000`,
`entries=1024`, `joined=YES`) and its USERD **is** the guest's page
(`userd_memory=0xcafe0006 userd_offset=0x2000`, the guest's own kernel's
`NV_CHANNEL_ALLOC_PARAMS.userdMem`, inside the joined leaf). Legs A2 and B landed at `w261`/
`w262` and §0.3 has not been re-read since.

★★★ **This is `a_rulings_date_is_part_of_the_citation` exactly**, and it is load-bearing rather
than tidy: §0.3's conclusion — *"`GP_PUT == GP_GET` forever ⇒ the engine fetches nothing and
reports no error"* — is the **prediction this rung measures**, and its two premises are gone.
The same sentence is duplicated as a comment at `shim.rs:5154-5158` and at `shim.rs:4752`, so a
reader of the code meets it three times.

⊘ **What survives** is the *posture*, not the reason: re-opening the route must still be an
armed, printed choice with a control arm. That is what §2 does.

### 0.2 ★★★★★ THE GR PUSHBUFFER PAGES ARE NEVER PINNED, AND NOT BECAUSE THE PIN IS SHORT OF A SOURCE

The brief's item-2 Q1 asks *"do the GR channels' pushbuffer pages get pinned at all?"*
⊘ **No — and the reason is structural, not a missing source.** All three pin passes
(`pin_ring_guest_ram`, `pin_pushbuffer_guest_ram`, `pin_completion_guest_ram`) live in
`DoorbellPort::ring`, **below** `try_ce_submission`. On the shipping arm a `GrCompute` doorbell
is terminated *inside* `try_ce_submission` by `ShellDisposition::RefuseByRoute` →
`return Some(refused(Route::NotACopyEngineChannel))`, so it never reaches any of them.

`[measured, w267_on]` the census says so with no interpretation: **16** `PB-PIN token=` lines
and **12** `SEMA-PIN token=` lines, and every single one names one of the **eight `Ce` tokens**
(`0x0001000f…0x00020016`, twice each). **Zero** name any of the eight `GrCompute` tokens
(`0x00000007…0x0000000e`).

⇒ ★ **Arming the route IS giving the pins their GR source.** The two are not separate rungs:
the pins are downstream of the gate.

### 0.3 ★★★★★ `CE-SUBMIT → RETIRED = 0/0` IS NOT THE ANSWER TO "DOES ANYTHING RUN A CHANNEL"

The brief's item-2 Q2 offers `CE-SUBMIT → RETIRED = 0/0` as possible evidence that *"the GR path
has no executor and never did."* ⊘ `CE-SUBMIT` is the **isolate's own emulated copy**
(`kayfabe-isolate-host/src/rm.rs`, `ce_copy`); it is not what ran the CE work at `w267`.

What ran it is `SharedDevice::doorbell` → `kayfabe_fwd`'s `VerbPlan::Doorbell` arm
(`kayfabe-isolate/src/lib.rs:2787-2790`), which calls **`rm.schedule(chan)` then
`rm.ring_doorbell(host_token)`** — the tree's only `ring_doorbell` call site — and it runs
**inside `verb_op`, before `forward_ring`**. So at `w267` every `Ce` doorbell scheduled and rang
its host channel, and *then* `forward_ring` returned `PushbufferAperture` and the shim printed
`DOORBELL-REFUSED`.

⇒ ⊘⊘ **`DOORBELL-REFUSED [FwdFault::PushbufferAperture]` is a POST-HOC refusal.** The host
doorbell had already been rung. That is why the hardware executed, and it retires the standing
puzzle `w266` recorded as *"the eight doorbells are REFUSED and the hardware executes anyway"*.

⇒ And it names the asymmetry exactly: `ring_content_is_forwardable(engine)` is
`matches!(route_of_engine(engine), DoorbellRoute::CpuCe)`, so a `GrCompute` doorbell that
reached `SharedDevice::doorbell` would **skip `forward_ring` entirely** and return `Ok(Served)`
— after scheduling and ringing. **The executor for GR exists, is the same one, and is reached by
one `if`.**

⇒ ★ The answer to item 2 Q3 is therefore **a missing ARM**, and the arm supplies the missing
source (§0.2). It is not a missing mechanism and not a refusal we cannot name.

---

## 1. ⊘ THE OWNER'S DIAGNOSTIC IS A ONE-CALL-SITE ARMING, NOT A BUILD — verified before writing it

The owner's mid-lane correction: *"if the GPU even tried running, `GP_GET` should advance —
read `GP_GET`/`GP_PUT` for the GR channels."* ⚠ *"Check that before building anything."*

`[verified 2026-08-12, at the code]` the capability **exists and is engine-agnostic**:
`fb_userd_cursors(plane, Option<DeclaredUserd>)` (`shim.rs:11203`) reads
`USERD_GP_GET`/`USERD_GP_PUT` out of the framebuffer store, checks the join **before**
residency (the correction `w266` paid for), and formats
`fbuserd@0x… GET=n PUT=m JOINED-one-memory`. It takes a `DeclaredUserd`, never an engine.

⊘ **What is missing is a caller.** It is reached only through `addressing_probe_facts`, which is
printed only on the forwarding fall-through (`RING-PROJ`) and the three CE refusal sites.
`[measured, w267_on, `grep -c "GET="` = **9**]` — eight `RING-PROJ` lines, all `Ce`, plus one in
the first-refusal summary. **No boot in this campaign has ever printed a `GrCompute` channel's
`GP_GET`.**

⚠ **And a doorbell-time sample cannot answer the owner's question.** `[measured, w267_on]` each
GR channel is rung **once** (`DOORBELL-REFUSED #5…#12`, one per token), so a cursor read on the
doorbell path is taken microseconds after the guest wrote `GP_PUT` — far too early for `GP_GET`
to have moved, on either arm. A sample that reads `GET=0` at `t+0µs` is not evidence the engine
never fetched; it is evidence nobody waited.

⇒ **The instrument must be LATE and REPEATED.** `w268` puts it on the completion observer's
thread, beside `SEMA-PAGE`, which already ticks every 250 ms for ~174 s.

---

## 2. WHAT IS BUILT — two instruments and one fix, and the arms they are read under

**I1 — `GR-CURSOR`, the owner's three-way discriminator, sampled late.** Every `GrCompute`
channel that declares a completion latches `(proc, chan, token, DeclaredUserd)`; the observer
thread reads each one's `GP_GET`/`GP_PUT` on every tick and prints one line per channel **on
first sight and on every change**, with `t=+Nms`. ⊘ A pure read of eight bytes through the same
`RegPlane` store the descent uses — no second resolver, no new address arithmetic.
⚠ It also prints a bounded `GR-CURSOR … why=doorbell` at the declare, so *"the latch never
happened"* and *"the cursor never moved"* are separable.

**I2 — the ordering fix (the brief's item 1): a SECOND SOURCE for the completion pin.**
`pin_completion_guest_ram`'s only source is `WatchList::declared_sites`, which the **GR**
doorbells populate, while the pass is triggered by a **CE** doorbell — nothing orders those, and
`w267` measured 4 of 8 CE channels arriving first and printing `NO PAGE TO PIN`.
The second source is **this channel's own pushbuffer**: `read_submission_methods` +
`PushbufferAbi::decode_run` (the chip's own CE codec, `ga10x.rs`'s `ce_completion`), whose
`PushMethod::CeLaunchDma { completion } / CeRelease` carries the `SET_SEMAPHORE_A/B` VA the
guest wrote in **this** doorbell's own bytes.
⊘⊘ **NOT a remembered `0x2_0440f000`** — that is the `cap2b` class and the brief is right that it
is the one way this can go badly wrong. Every address comes from the guest's bytes, read at this
doorbell, decoded by the existing class-gated codec, and every page is still asked of the
address table with `miss = fault`.
⊘⊘ **AND IT IS NOT A WIDENING OF THE WATCH.** `completion_watch.rs` is untouched; the CE VA
enters the **pin** and never `WatchList`. The brief's ⊘ stands and this rung honours it: an
`OBSERVED` row on a CE slot would mean nothing and read as everything.

**A — the arm.** `KAYFABE_GR_ROUTE`, `refuse` → `passthrough`, the **only** variable between the
two boots. Both instruments and the fix are on **both** arms; they are instruments, not the
variable. ⚠ This makes `w268_refuse` ↔ `w267_on` **not** byte-comparable, and that is stated
here rather than discovered in the grading.

| arm | FB_JOIN | GUEST_RING | GUEST_PUSHBUF | PT_WITNESS_EXEC | GUEST_SEMA | **GR_ROUTE** |
|---|---|---|---|---|---|---|
| `w268_refuse` | shared | ring | pin | on | pin | *(unset ⇒ refuse)* |
| `w268_pass` | shared | ring | pin | on | pin | **passthrough** |

---

## 3. THE PREDICTIONS — every reading named before it exists

### 3.1 ★★★★★ THE OWNER'S THREE-WAY DISCRIMINATOR — `w268_refuse`, the eight `GrCompute` channels

| # | reading | what it means | p |
|---|---|---|---|
| **U1** | `PUT > 0`, `GET == 0`, all 8 | **the engine never fetched GR's ring.** The work was never started; the zero semaphore is downstream of a cause upstream of it | **0.70** |
| **U2** | `GET == PUT != 0` on ≥1 channel | **GR's work RAN.** The zero slot is then a *separate*, downstream failure and the diagnosis changes completely | **0.09** |
| **U3** | `PUT == 0` on all 8 | the guest never submitted GR work on these channels — relocates the question to the guest side entirely | **0.05** |
| **U4** | `0 < GET < PUT` on ≥1 | partial fetch — the engine started and stopped mid-ring | **0.03** |
| **U5** | no `GR-CURSOR` line, or `=REFUSED(...)`, or `framebuffer_base() == None` | ⊘ a statement about the **instrument**, not the plane. Named so an absence can never be read as `GET=0` | **0.13** |

★ I name **U1**. The mechanism §0.3 establishes predicts it without needing the measurement:
`rm.schedule` + `rm.ring_doorbell` are only reached through `SharedDevice::doorbell`, and no GR
doorbell reaches it on this arm. An unscheduled channel is not on a runlist and cannot fetch.
⊘ U5's 0.13 is deliberately the second-largest mass: this instrument has never run on a `GrCompute`
channel in any boot, and *"it printed nothing"* is its most likely first failure.

### 3.2 THE ARM — `w268_pass` minus `w268_refuse`, one variable

| # | reading | p |
|---|---|---|
| **A1** | `DOORBELL-REFUSED [Route::NotACopyEngineChannel]` **8 → 0**, and 8 GR doorbells report `Served` | **0.85** |
| **A2** | `PB-PIN token=` **16 → 32** and `SEMA-PIN token=` grows by 8+ — the pins acquire their GR source **for free**, §0.2 | **0.80** |
| **A3** | GR `GP_GET` **advances** on ≥1 channel ⇒ the host engine fetched the guest's own ring | **0.35** |
| **A4** | a **new** host `Xid` whose address is a GR **pushbuffer** VA (`0x2_00400000`, `0x2_00800000`, …), `ACCESS_TYPE_VIRT_READ` | **0.40** |
| **A5** | GR slots `+0xf80…+0xff0` become non-zero ⇒ `COMPLETION-WATCH → OBSERVED` | **0.07** |
| **A6** | **`CUP2_RC` moves off `124`** | **0.05** |
| **A7** | the arm is *worse*: QEMU dies, the host GPU wedges, or an `Xid` storm | **0.15** |

⚠ **A4 is why A3 firing is not good news on its own.** The GR pushbuffer VAs
(`0x200400000`, `0x200800000`, …) are in **neither** joined leaf: `GR-RING-JOIN` joins the ring's
leaf (`0x200200000`) and `GR-FB-JOIN` joins the *operand* census's leaves (`0x200000000`,
`0x10000000000`, `0x10002000000`). ⊘ **A pushbuffer is not an operand of the methods it carries**
— `resolve_leaf_of`'s own doc says exactly this about the ring, one plane over. So a fetching GR
engine should fault reading its own methods, and **that is leg 4 repeating one plane on**: for CE
the pushbuffer was in guest RAM and the *pin* reached it; for GR it is in the emulated framebuffer
and only a *join* can.

⇒ ★ **A3 + A4 together is the most useful outcome available and I expect it more than A3 alone.**

### 3.3 THE ORDERING FIX — both arms

| # | reading | p |
|---|---|---|
| **O1** | `NO PAGE TO PIN` **4 → 0**; all 8 CE channels pin | **0.80** |
| **O2** | `SEMA-SOURCE=CE` fires on 8 channels, each naming a page **derived from that channel's own `m0x240` operand** | **0.85** |
| **O3** | the CE half of the page carries **8** complete reports ⇒ `nonzero = 24/1024` (3 non-zero words per 16-byte report; word 1 is the zero half of the payload) | **0.55** |
| **O4** | host `Xid` **4 → 0**, graded as **identity**: no `ENGINE CE3`, no `HUBCLIENT_CE1`, zero distinct addresses | **0.60** |
| **O5** | the second source and the declared source **agree** on the page wherever both fire | **0.90** |

⊘ **O3 is weaker than O1**, deliberately: pinning is our act and writing is the engine's, and
`w267` §4 states in as many words that *"the four unpinned channels would have written had they
been pinned"* is **implied and not measured**. O3 is where that gets measured.

### 3.4 THE CARRIED ROWS — predicted at zero movement, for the eighth consecutive rung

| observable | prediction | why |
|---|---|---|
| **`CUP2_RC`** | **`124` on BOTH arms. Movement predicted at ZERO — not "small"** — `p = 0.88` | ★ The brief says to say so plainly if zero is the right prediction, and it is. Item 1 moves a CE plane the guest does not poll. The arm's *most likely* good outcome (A3+A4) is a **fault**, not a completion. The residual `0.12` is A6 plus *"`cup2` changed for an unrelated reason"*, which would make the number **uninterpretable**, not good |
| `COMPLETION-WATCH → OBSERVED` / `NOT-OBSERVED` | `0` / `8`, both arms | §3.2 A5 |
| `CE-SUBMIT` / `RETIRED` | `0` / `0`, both arms — and it is **not** the GR question (§0.3) | eighth boot pre-registered at zero |
| `refusals=` | `255` both | untouched, carried debt |
| `by-executor` | `39` both | untouched, carried debt |
| host-channel VAS | `[NOT MEASURED]` | untouched, carried debt |
| `PT-DECODE` first pass `bound` | ⚠ **I predict it MOVES again** (`≠ 19618`) and stays identical **between arms**, `p = 0.55` | `w267` §3.4 measured `19615 → 19618` and bounded rather than explained it; the semaphore page landed at three different GPAs in three boots. A cross-revision guard that has already drifted once is not a guard |
| `PAGE-READER ASSERTION` | **FAIL on both arms**, `p = 0.85` | ⚠ inherited from `w267` §3.2 and **not fixed here**: `close()` needs `detach_ram`, and QEMU is powered off without it. Named so its recurrence is a prediction rather than a surprise |

---

## 4. ⊘ WHAT THIS RUN WILL NOT BE ABLE TO PROVE — stated now

1. ⊘ **That `GET == 0` means the channel was never scheduled.** It is consistent with that and
   with *scheduled but never rung*, and with *rung but faulted before the first fetch*. Nothing
   here reads a runlist.
2. ⊘ **That the guest's CPU sees what the FB store sees.** Carried unpaid from `w266` §4 /
   `w267` §3.5.
3. ⊘ **Anything finer than 250 ms**, and nothing about writes that did not change bytes. The
   dump and the cursor sample both report *state*, never *events*.
4. ⊘ **That a moving `GP_GET` means correct work.** The engine fetching bytes is not the engine
   executing them correctly, and this rung has no oracle for the second.
5. ⊘ **255 `StraddlesLiveBinding`**, **`by-executor = 39`**, **host-channel VAS** — untouched.
6. ⊘ **That the arm is safe to default.** Two boots is not a posture change; `refuse` stays the
   default whatever this measures.
