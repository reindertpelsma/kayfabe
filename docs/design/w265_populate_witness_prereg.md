# w265 PRE-REGISTRATION — the populate source is BUILT, and `w264` ran with it DISARMED

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTRATION, committed before the boot.**
> Branch `leg-4-populate-witness`, off `leg-4-pushbuffer-pin` = `a464c1e`.
> Predecessor: `traces/boots/w264/RESULT.md`. Authority: `mode2_address_table.md` (C tree).

---

## 0. ★★★★★ LEAD — THE BRIEF'S CENTRAL QUESTION IS ANSWERED, AND BOTH HALVES CONTRADICT IT

The brief asks two questions and offers a reading of the first. **The reading does not survive,
and the second question's answer already exists in this tree and was switched OFF in the boot
that produced the finding.**

### 0.1 ⊘⊘ Q1 — "is filling the table from the descent legitimate?" **NO, and the doc refuses it BY NAME**

The brief's reading is *"a walk is forward (VA → GPGA), and `the table IS the guest's TLB` is a
metaphor in which filling from a walk on a miss is exactly what a TLB does."*

`mode2_address_table.md` **anticipates that exact argument and rejects it, in the same sentence
that introduces the metaphor**:

> §1 — *"A real TLB holds stale/absent entries until an invalidate; it is refreshed from memory on
> invalidate (**and, on real HW, on a miss-walk — which we deliberately do NOT replicate, §6**)."*

> §6 — *"We explicitly do **NOT** do an opportunistic 'walk the PDB one last time' on a miss. Real
> HW's miss-walk is safe only because the driver never *acts* on an uncommitted VA — which, for
> us, means it is already in the table (no miss). **A miss is, by definition, the unsafe-to-walk
> state.**"*

⇒ The metaphor is **explicitly scoped to exclude the miss-walk**. This is not a gap in the doc
that a reading can fill; it is a decision the doc records, with its reason (a torn multi-level
walk resolves to the wrong physical page — a cross-context leak).

★ And the doc names this rung's correct shape in the next sentence:

> §6 — *"A miss is also the signal that we failed to capture a binding at its populate site (§4) —
> **fix it there, not with a fallback.**"*

⊘ **Per `a_rulings_date_is_part_of_the_citation` I checked whether the *why* survives**, not only
the ruling. §5's ★ CORRECTION (2026-07-22) rewrote the *populate sources* but did **not** touch
§6; and §6's reason (uncommitted state is unsafe to read) is **strengthened**, not weakened, by a
compute path on which no invalidate fires at all. **Q1 needs no owner ruling** — the doc already
ruled, the reason still holds, and this rung does the thing the doc says to do instead.

### 0.2 ★★★★★ Q2 — YES. The third source is UNAMBIGUOUSLY FORWARD, ALREADY BUILT, AND WAS OFF

`KAYFABE_PT_WITNESS_EXEC` — `crates/kayfabe-qemu-raw/src/shim.rs:6117`,
`SharedDoorbell::witness_executor_fb_pages`. **`w264` never exported it.** `w264_run.sh` sets
seven `KAYFABE_*` variables and this is not among them, and every one of `w264`'s four arms says
so in its own log:

```
kayfabe: PT-DECODE token=0x00020013 | EXEC-WITNESS DISARMED (KAYFABE_PT_WITNESS_EXEC unset
  or `off`) — the executor's pages are NOT witnessed …
```

⚠ `w261` and `w262` **did** arm it (`traces/boots/w261/RESULT.md:10`). `w263_run.sh` and
`w264_run.sh` dropped it. This is `a_correct_default_is_not_a_handoff`: the flag defaults off
*deliberately* (it is its own negative control), and two successive harnesses inherited the
default without naming it.

### 0.3 ★★★★★ THE CAUSAL CHAIN IS COMPLETE INSIDE `w264`'s OWN LOG — four lines, no inference

1. **The tree's pages are executor-written.** `run_w264_pin_qemu.log:291`, the descent for
   `pdb=0x201000` — the pdb that `Miss`es:
   ```
   walk: L0@0x201000/byEXEC#104  L1@0x202000/byEXEC#105
         L2@0x203000/byEXEC#106  L3@0x204000/byEXEC#107
   ```
2. **Our executor is invisible to the witness transport.** `shim.rs:6127` — G1 witnesses inside
   the framebuffer *window* write path (PRAMIN/BAR1/BAR2); `FbWriter::Executor` writes the same
   store and is **structurally invisible** to it.
3. **So the VAS's witness set is EMPTY.** `run_w264_pin_qemu.log:293`:
   ```
   VAS-BIND-CENSUS proc=2 pdb=0x201000 … root_wit=N dirty=0 meta=0 shadow=0
                   wit=0 published=0 wit_sample=[]
   ```
   ★ `ReachShadow::witnessed_len`'s own doc says this distinguishes *"a transport that does not
   cover this writer"* from *"an ordering gap"*. `wit=0` is the **first**.
4. **So every leaf is refused, by design.** `reach.rs:454` — *"Reachable-but-unwitnessed: a MISS,
   on purpose."* `PT-DECODE … bound=6275 … unwitnessed=6275`: **one VAS bound all 6275 of its
   leaves, another had all 6275 refused.** That is `shim.rs:6143`'s measured two-population
   contrast — the *system* proc's tree is BAR2-written and binds; the compute proc's is
   executor-written and does not.

★ And `w264`'s own census reproduces the 96.8 % figure on this hardware:
`framebuffer FIRST-WRITER census: PRAMIN 21 / BAR1 9 / BAR2 88 / EXEC 2522 / UNATTRIBUTED 0`
⇒ **2522 of 2640 resident framebuffer pages (95.5 %) are created by a writer the witness cannot
see.**

### 0.4 ⊘ WHY THIS DOES NOT BREAK `miss = fault` — the invariant the brief demands survives

This adds a **writer** to the witness transport. It does **not** add a lookup path, a fallback, or
a walk. After it:

- `AddressTable::resolve` is **unchanged**: still a pure table read, still `MISS = FAULT`
  (`reach.rs:52` — *"It is not a cache `resolve` may consult"*).
- The bind gate is **still REACHABLE ∧ WITNESSED**. A genuine miss — a VA whose leaf is in a page
  **nobody was seen to write** — still faults, because a non-resident frame has **no origin** and
  `fb_page_origin` answers `None` for it, so it is never added.
- What is excluded is **residue**, and residue is exactly what stays excluded: `reach.rs:41` —
  *"A directory entry read out of allocator residue can only make some other page reachable. That
  page is itself unwitnessed, so nothing in it binds."*
- ★ `shim.rs:6151` states the trust argument directly: *"A page our executor wrote is a page the
  guest asked us to write, at an address the guest chose, with bytes the guest supplied — it is
  **more** directly witnessed than a window write, not less."*

⇒ **The genuine-miss case is untouched.** The falsifier for that claim is row R9 below.

### 0.5 ⊘ WHAT I AM THEREFORE **NOT** BUILDING, AND WHY SAYING SO IS THE DELIVERABLE

No new populate source. No device-code change at all. The consumer (`pushbuffer_pin_report`), the
source (`witness_executor_fb_pages`), the gate (`ReachShadow::settle`) and the diagnostic that
names the cause (`VAS-BIND-CENSUS … wit=`) are **all built, all correct, and all already print**.

⊘ I considered adding a "name the cause on the MISS line" diagnostic and **rejected it**: the
cause is already on `VAS-BIND-CENSUS`, one line above the `PB-PIN` line, in the same log, for the
same pdb. A second copy would be `a_second_source_of_truth_beside_a_complete_value`.

⇒ **The rung is a two-arm boot isolating one variable, plus a harness that can no longer drop
it silently.**

---

## 1. THE LADDER — two arms, ONE variable

| arm | `FB_JOIN` | `GUEST_RING` | `GUEST_PUSHBUF` | **`PT_WITNESS_EXEC`** |
|---|---|---|---|---|
| `w265_off` | shared | ring | pin | **(unset)** |
| `w265_on` | shared | ring | pin | **`on`** |

★ `w265_off` is **byte-for-byte `w264`'s `pin` arm**, so it is simultaneously the control *and* a
reproducibility check on `w264`'s headline. ⚠ If `off` does not reproduce `w264`'s numbers, the
comparison is void and I will say so before reading `on`.

★★ Every `KAYFABE_*` the device reads is **explicitly `export`ed or explicitly `unset`** per arm,
and each arm's arming is asserted **out of the boot's own log**, never out of the shell —
including a new assertion for `EXEC-WITNESS ARMED` / `DISARMED`. That is the durable half: the
defect this rung corrects is *a flag that was never named*, and only a harness that names every
flag prevents its recurrence.

---

## 2. ★★ PRE-REGISTERED PREDICTIONS — each number's READING named before the boot

`p` = my confidence. Every row is graded mechanically by `w265_grade.sh`, which reports a missing
row as **NO-FILE** rather than as a zero.

| # | observable | pred `off` | pred `on` | p | ★ the reading, fixed in advance |
|---|---|---|---|---|---|
| R1 | `EXEC-WITNESS` arm line | `DISARMED` | `ARMED` | .97 | ⊘ Anything else ⇒ **the variable did not reach the device** and the whole run is void. Checked first. |
| R2 | `EXEC-WITNESS by-executor=` | n/a | **≈ 2522** | .8 | Reproduces `w264`'s census. A **0** ⇒ `KAYFABE_CE_EXECUTOR=host` means the executor never writes FB, the source is empty, and §0.3's chain is **refuted at step 1**. |
| R3 | `EXEC-WITNESS refused-at-cap=` | n/a | **0** | .9 | Cap is 65536 ≫ 2522. Non-zero ⇒ the witness is **truncated** and every later number is a lower bound. |
| R4 | `VAS-BIND-CENSUS … wit=` for `pdb=0x201000` | **0** | **> 0** | .85 | ★ **The most direct readout of the mechanism.** `wit=0` on `on` ⇒ the executor pages never reached the queue ⇒ refuted at step 2, and R2 says which. |
| R5 | `PT-DECODE … unwitnessed=` | **6275** | **0** | .7 | The gate opening. A residual non-zero ⇒ a *third* population exists that neither transport covers — a **finding**, not a failure. |
| R6 | `PT-DECODE … bound=` | **6275** | **≈ 12550** | .55 | ⚠ Could instead surface `refusals`/`faults` > 0 (overlap with an RPC-populated range). That would be a **real defect this rung exposed**, and is the most likely bad surprise. |
| R7 | `PB-PIN … MISS` | **8** | **0** | .6 | ★ **The rung's own claim.** 8 → 0 = the two resolvers now agree about existence. |
| R8 | `PB-PIN … resolved in guest RAM` | **0** | **8** | .55 | The consumer's first live input. |
| R9 | `PB-PIN … NOT-IN-GUEST-RAM` | **0** | **0** | .6 | ⊘ **The `miss = fault` falsifier.** A non-zero here means the newly-bound leaf says **Vidmem** for a VA the descent calls `S:` — i.e. we bound the *wrong* leaf. That is worse than the miss and would **retract** the rung. |
| R10 | `PB-PIN … PINNED` runs | **0** | **≥ 1** | .45 | ⊘ `w264`'s Q3 died upstream of this; **nothing past the table lookup has ever executed.** `resolve_guest_ram`, `GuestRamGrant`, `OS_DESCRIPTOR` and placement are all first-run here. Lowest confidence on the page, and deliberately so. |
| R11 | `placed_as_asked=true` | 4 | ≥ 4 | .5 | ⊘ `w264`'s 4 are from **other** pin sites. Only a rise **above 4** is this rung's. |
| R12 | ★ host `Xid 31 FAULT_PDE` | **8** | **8** | .55 | ★★ **THE FALSIFIABLE QUESTION THIS RUNG OWNS.** I predict **NO movement**, and the reason is not pessimism: a pin maps the page into **our** isolate at the guest's VA, and `w264` §5 records that **which VAS the host channel is bound to is `[NOT MEASURED]` by any rung**. A drop to **0** would be the strongest single result of the campaign and would make leg 4 complete; I put that at **.25**, and "some other non-zero" at **.20**. |
| R13 | **`CUP2_RC`** | **124** | **124** | **.9** | ★ **Predicted movement: ZERO, and the magnitude of the expected movement is ZERO seconds** — not "small". Leg 5 (completion) is **unbuilt**; `CE-SUBMIT → RETIRED` has never printed in ~127 logs. **No table fix can retire a semaphore that nothing submits.** ⇒ A `124 → 0` here would mean I do not understand the wall, and I would treat it as an instrument fault until a second boot reproduced it. This is the fifth consecutive lane predicting zero. |
| R14 | `CE-SUBMIT` / `RETIRED` | 0 / 0 | 0 / 0 | .9 | As R13. |
| R15 | `RmInitAdapter failed` | 0 | **0** | .8 | ⚠ **The genuine regression risk.** Binding ~6275 extra leaves changes what the forwarding plane will act on. A guest that dies on `on` and lives on `off` ⇒ the new bindings are **wrong**, and that outranks every green row above it. |
| R16 | guard: `GR-BIRTH` / `NVRM` | 24 / 31 | 24 / 31 | .8 | Movement ⇒ the arms differ in more than one variable. |
| R17 | guard: `ENGINE-OBJECT seen/fwd/ref` | 34/32/2 | 34/32/2 | .8 | As R16. |
| R18 | guard: `BAR1 GP_PUT` advances | 26 | 26 | .7 | As R16. |

### 2.1 ⊘ WHAT THIS RUN CANNOT PROVE — written before it runs

- ⊘ **Anything about completions.** Leg 5 is unbuilt. R13/R14 are pre-registered at zero.
- ⊘ **That a successful pin is what silences an `Xid`, if one is silenced.** `on` changes the
  witness, which changes the bindings, which changes the pin *and* anything else that resolves.
  A `12 → 0` on R12 would be attributable to the arm, **not** to the pin specifically.
- ⊘ **Leg B vs leg A2** (the brief's ITEM 2). Both arms carry both. **I am not doing it** — it
  needs a boolean on `plan_engine_object`'s public signature, and `w264` §5 costs it as its own
  rung. Stated plainly, as the brief asks.
- ⊘ **That `unwitnessed = 0` means the table is COMPLETE.** It means every leaf in a *reachable
  and witnessed* page bound. Pages nobody wrote are still — correctly — absent.
- ⊘ **`refused-at-cap = 0` on `off`** is not exercised: nothing is enqueued there.
- ⊘ **Whether the miss was *never learned* or *learned and pruned*** — `w264` §5 named this split.
  R4 answers it: `wit=0` with a **non-empty** `wit_sample` would be pruning; `wit_sample=[]` (what
  `w264` shows) is *never learned*. This run does not otherwise probe lifetimes.
