# w265 — THE POPULATE SIDE CLOSES, THE PIN FIRES, AND **THE WALL MOVES TO THE COMPLETION PLANE**

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Two arms from **one** binary, source revision
> **`2f02621`**, stamp-gated against the binary before booting
> (`STAMP: [kayfabe-rev:2f026212…] WANT: [kayfabe-rev:2f026212…]` → `PASS`), content-checked, and
> **each arm's arming ASSERTED out of its own log** (`WITNESS-ARM ASSERTION: PASS`, both).
> Bench `vh`, `NVIDIA GeForce RTX 3060` (GA106), host driver **580.159.04**.
> Graded against `docs/design/w265_populate_witness_prereg.md`, committed at `b0d6559`,
> **before the code existed**. `BUILD_RC=0`, both `BOOT … RC=0`, `EXIT rc=0`, `ENOSPC/LLVM = 0`
> from the same invocations. Branch `leg-4-populate-witness`.

---

## 0. ★★★★★ THE HEADLINE — FOUR THINGS, AND THE FOURTH IS THE ONE THAT MATTERS

**No device code was written.** The populate source was already built and `w264` ran with it off.
Arming `KAYFABE_PT_WITNESS_EXEC=on`, one variable, moved the wall a full layer.

1. **The table learned the leaves.** `pdb=0x201000` went from **5 rows** to **13 348 rows**;
   `wit=0` → **`wit=37`**; and `wit_sample` went from `[]` to
   **`[0x201000, 0x202000, 0x203000, 0x204000]`** — ★★★ *exactly* the four page-table pages the
   descent's own `walk:` line names as `byEXEC#104…#107`. The witness now covers precisely the
   pages that were invisible.
2. **`PB-PIN` MISS 8 → 0, resolved-in-guest-RAM 0 → 8.** The two resolvers that `w264` found
   disagreeing about existence now **agree**.
3. **`PINNED` 0 → 8 — the guest-RAM pin has placed bytes on a live guest for the first time.**
   Eight runs, `placed_as_asked=true` on all eight, `4 → 12` (+8, and only a rise above 4 is this
   rung's). `NOT-IN-GUEST-RAM = 0`, so the `miss = fault` falsifier held.
4. ★★★★★ **THE EIGHT `Xid` AT THE EIGHT PUSHBUFFER VAs ARE GONE — AND THE COUNT DID NOT MOVE.**

```
off: MMU Fault: ENGINE CE3_PBDMA0 HUBCLIENT_ESC faulted @ 0x2_02c00000 … type FAULT_PDE ACCESS_TYPE_VIRT_READ
     (8 faults, at the 8 distinct pushbuffer VAs 0x2_02400000 … 0x2_03200000)

on:  MMU Fault: ENGINE CE3      HUBCLIENT_CE1 faulted @ 0x2_0440f000  … type FAULT_PDE ACCESS_TYPE_VIRT_WRITE
     (8 faults, ALL at ONE address, which is not a pushbuffer VA)
```

**All four fields changed, and every one changed toward progress:**

| field | `off` | `on` | what the change means |
|---|---|---|---|
| engine | `CE3_PBDMA0` | **`CE3`** | the **front-end** stopped faulting; the **copy engine** started |
| client | `HUBCLIENT_ESC` | **`HUBCLIENT_CE1`/`CE0`** | `ESC` is the PBDMA's method-fetch client; `CE0/CE1` are the engine's **data** clients |
| address | the 8 pushbuffer VAs | **one new page** | the pages we pinned are no longer faulting |
| access | **`VIRT_READ`** | **`VIRT_WRITE`** | it stopped failing to **read the pushbuffer** and started failing to **write a destination** |

⇒ **The PBDMA fetched the pushbuffer, parsed its methods, and the copy engine began executing
them.** That is the pushbuffer being *consumed*, not merely *fetched* — the thing `w263` could
only show was *attempted*.

### 0.1 ★★★★★ AND THE NEW ADDRESS IS THE COMPLETION SEMAPHORE — LEG 5, ARRIVING AS A HARDWARE FAULT

`0x2_0440f000` is not an unknown. Our own log names every one of its occupants:

```
COMPLETION-DECLARE token=0x0000000e proc=2 chan=7 engine=GrCompute
  → DECLARED va=0x20440ff80 payload=0x00000001 … class=0xc7c0 site=GuestRam { gpa: 93990784 }
SET_REPORT_SEMAPHORE  m=0x1b00 sub=1 va=0x20440ff80 → GuestRam { gpa: 93990784 }
```

**Eight channels, eight `SET_REPORT_SEMAPHORE` targets, `0x20440ff80 … 0x20440fff0` at a 16-byte
stride — all inside the one 4 KiB page the hardware faults on, and all `site=GuestRam`** at
contiguous gpas `0x59a0f80 … 0x59a0ff0`.

⇒ **The eight `Xid` on `on` are eight channels each faulting while trying to WRITE its own
completion semaphore.** `CE-SUBMIT → RETIRED` is still `0` — but for the first time in ~128 logs
the hardware got far enough to *attempt* a completion, and it named the page it needs.

★★★ **The next rung's shape is already proven, because it is the shape that just worked.** This
is isomorphic to the rung that closed:

| | `w264` / `w265_off` | `w265_on` |
|---|---|---|
| who faults | PBDMA, `VIRT_READ` | copy engine, `VIRT_WRITE` |
| on what | 8 pushbuffer pages | **1 semaphore page** |
| where it lives | guest RAM (`pb=S:`) | **guest RAM** (`site=GuestRam`) |
| the fix | present those pages to `pin_guest_ram` | **present that page to `pin_guest_ram`** |

⊘ **It is ONE page for all eight channels**, so it is smaller than the rung just completed.
⚠ And the same caveat the pin prints about itself still applies: *"This says NOTHING about
whether the host channel is bound to a VA space in which those VAs resolve."*

---

## 1. ⊘⊘ WHAT THIS RUN REFUTED IN ITS OWN PRE-REGISTRATION

### 1.1 ★★★★★ R12 — I PREDICTED NO MOVEMENT AT `p=.55`, AND I WAS WRONG IN THE FAVOURABLE DIRECTION

The brief named this *"the falsifiable question this rung owns"* — *"if the table resolves and the
pin places, **do the eight `Xid` go away?**"* — and I answered **no, they stay at 8**, giving 0.25
to a drop and 0.20 to "some other non-zero".

**The eight go away.** The answer is the 0.20 branch, and it is better than the 0.25 branch would
have been: a drop to zero would have said the engine stopped trying, whereas a *move* says it
**advanced**. I gave the outcome that actually happened the **lowest** of my three weights.

⚠⚠ **AND MY OWN GRADER COULD NOT SEE IT.** `R12` is `grep -c Xid` — **8 on both arms.** The row
I pre-registered as the rung's central question reported *"no change"* about the run's biggest
result. Only the **address/engine/client/access-type** projection shows it.
⇒ ★★★ **A COUNT CANNOT SEE A SUBSTITUTION.** Eight faults became eight *different* faults, from a
*different engine*, via a *different client*, in a *different direction*, at a *different address*
— and every one of those five facts is invisible to a count that was, by construction, only ever
going to report a magnitude. This is `a_small_count_is_not_a_small_event` composed with
`our_census_counts_intent_the_driver_counts_attempts`: **when the fix is expected to move a wall
rather than remove it, the count is the wrong instrument and the identity is the right one.**
⊘ The `w264` grader had the address row too; I inherited it. **The row that saved this result was
inherited, not designed** — I should have promoted it to the scorecard and did not.

### 1.2 ⊘ R2 — `by-executor = 39`, NOT the `≈ 2522` I PREDICTED AT `p=.8`

`EXEC-WITNESS ARMED resident=156 by-executor=39 refused-at-cap=0`, identically on every doorbell.
The `FIRST-WRITER census` reads **`EXEC 2522 / resident 2640` on BOTH arms** — unchanged by the
flag, as it should be, since the census is about *who wrote*, not *who witnessed*.

⇒ **The two numbers are measured at different TIMES**, not of different things: the witness runs
per-doorbell (156 frames resident then); the census prints later, after `cup2` has run (2640).
I read a teardown number as if it described the doorbell instant — the same class as *"a recorder
that dumps at teardown reports ORDER, not TIME"*, one instrument over.

★ **39 was enough**, and the reason is not luck: page tables are written *before* the doorbell
that uses them, so the pages that matter are exactly the ones already resident. ⚠ But it **bounds
the fix**: a page-table page first written *after* the doorbell that needs it would still be
missed, and nothing here tests that.

### 1.3 ⊘ R5 — `unwitnessed` went UP, not to 0

Predicted `6275 → 0` at `p=.7`. Measured, on the first (only decoding) `PT-DECODE` pass:

```
off: drained=121 latched=42 rounds=2 → bound=6275  unwitnessed=6275  refusals=0
on:  drained=174 latched=79 rounds=2 → bound=19615 unwitnessed=19874 refusals=255
```

`bound` **6275 → 19615** (+13 340, and R6 predicted `≈12 550` — right direction, larger). But
`unwitnessed` **6275 → 19874**. ⇒ **The gate opened partway.** Witnessing 39 pages made a much
larger tree *reachable*, and a bigger tree contains more still-unwitnessed leaves. **A rising
`unwitnessed` beside a rising `bound` is the shadow seeing more, not binding less** — but it is
**not** the clean zero I predicted, and the table is **not** complete.

⚠ **My grader's R5/R6 summary rows are artifacts and must not be read.** They use `last`, and the
last `PT-DECODE` line of every boot reads `bound=0 unwitnessed=0` because only the first pass
decodes. The numbers above come from the per-line **dump**, which is why the dump exists.

### 1.4 ⊘ R6b — 255 refusals, all one kind, and it is a real cost

`refusals=255 … first=StraddlesLiveBinding { va: GpuVa(8655536128) }` — `0x203e00000`, on every
`on` pass, `0` on every `off` pass. The newly-decoded leaves **straddle a range already bound by
another populate source**, and the table refuses rather than overwrite. ⊘ `faults=0`,
`reach_faults=0`, so nothing is torn — but **255 bindings the guest's page tables declare are not
in the table**, and this run does not say whether any of them matter. It is the price of the fix
and it is unpaid.

---

## 2. THE PRE-REGISTERED SCORECARD

★ Both arms differ in **exactly one** variable, and the arming is read out of **each boot's own
log** (`EXEC-WITNESS DISARMED`/`ARMED`), asserted by the harness, not by the shell.

| # | observable | pred `on` | **off** | **on** | |
|---|---|---|---|---|---|
| R1 | `EXEC-WITNESS` arm | `ARMED` | `DISARMED` | **`ARMED`** | ✔ assertion PASS both |
| R2 | `by-executor=` | ≈2522 | n/a | **39** | ⊘ **REFUTED** — §1.2 |
| R2b | `resident=` | — | n/a | 156 | ⊘ vs census 2640 |
| R3 | `refused-at-cap=` | 0 | n/a | **0** | ✔ |
| R4 | `VAS-BIND-CENSUS wit=` | >0 | **0** | **37** | ✔★★★ and `rows` **5 → 13 348**, `published` **0 → 13 344** |
| R4b | `wit_sample` | non-empty | `[]` | **`[0x201000,0x202000,0x203000,0x204000]`** | ✔★★★★★ the four `byEXEC` pages exactly |
| R5 | `unwitnessed` (1st pass) | 0 | 6275 | **19 874** | ⊘⊘ **REFUTED** — §1.3 |
| R6 | `bound` (1st pass) | ≈12 550 | 6275 | **19 615** | ✔ direction, larger |
| R6b | `refusals` | — | **0** | **255** | ⊘ predicted risk **materialised** — §1.4 |
| R6c/d | `faults` / `reach_faults` | 0 | 0/0 | **0/0** | ✔ nothing torn |
| R7 | `PB-PIN … MISS` | **0** | **8** | **0** | ✔★★★★★ **the rung's own claim** |
| R8 | `… resolved in guest RAM` | 8 | **0** | **8** | ✔★★★★★ |
| R9 | `… NOT-IN-GUEST-RAM` | **0** | 0 | **0** | ✔★★★ the `miss = fault` falsifier **held** |
| R10 | `PINNED` runs | ≥1 | **0** | **8** | ✔★★★★★ first pin ever on a live guest |
| R10b/c | `CAPPED` / `SystemDataPlane` | 0 | 0/0 | **0/0** | ⊘ green **not exercised** (1 page/doorbell) |
| R11 | `placed_as_asked=true` | ≥4 | 4 | **12** | ✔ +8 = this rung's |
| R11b | `placed_as_asked=false` | 0 | 0 | **0** | ✔ |
| R12 | host `Xid` **count** | 8 | **8** | **8** | ⊘⊘ **the row is BLIND** — §1.1 |
| R12′ | host `Xid` **identity** | — | `CE3_PBDMA0`/`ESC`/8 pushbuffer VAs/`READ` | **`CE3`/`CE1`/1 semaphore page/`WRITE`** | ★★★★★ **THE RESULT** |
| R13 | **`CUP2_RC`** | **124** | 124 | **124** | ✔★ movement predicted **0**, movement **0** |
| R14 | `CE-SUBMIT` / `RETIRED` | 0/0 | 0/0 | **0/0** | ✔ leg 5 unbuilt |
| R15 | `RmInitAdapter failed` | 0 | 0 | **0** | ✔ guest alive on both |
| R16 | guest `NVRM` / `GR-BIRTH` | 31/24 | 31/24 | **31/24** | ✔ guard |
| R17 | `ENGINE-OBJECT seen/fwd/ref` | 34/32/2 | 34/32/2 | **34/32/2** | ✔ guard (⚠ grader's `first` row is an artifact; the census's **last** line is the number) |
| R18 | `BAR1 GP_PUT` | ~equal | 66 | **66** | ✔ guard |
| — | doorbells `REFUSED` / heartbeat | — | 16 / 160 | **16 / 160** | ✔ identical |

`RING-PROJ` 8, `adopt=GUEST-RING` 16, `userd=GUEST-USERD` 16, ring-pin `NOT IN GUEST RAM` 8 —
identical on both arms.

### 2.1 ★ `off` REPRODUCES `w264`'s `pin` ARM, which is what licenses the comparison

`MISS` 8, `NOT ONE PAGE RESOLVED` 8, `PINNED` 0, `placed_as_asked=true` 4, `Xid` 8 at the same
eight VAs, `bound=6275 unwitnessed=6275`, `CUP2_RC=124`, `ENGINE-OBJECT 34/32/2`, `NVRM` 31,
`GR-BIRTH` 24. ⇒ Two boot campaigns, two revisions, same numbers.
⚠ **The GPAs moved again, for the fourth time** — `w264` `0x41539000…`, `w265_off`
`0x422e7000…`, `w265_on` `0x7215000/0x3515000/0x3ee15000…` (and on `on` they are **not even
monotonic across channels**). ⊘ Any rung that hard-codes one is right on exactly one boot.

---

## 3. ⊘ WHERE THIS RUN IS WEAKER THAN IT LOOKS

- ⊘ **The arm is not the pin.** `on` changed the **witness**, which changed **13 343 bindings**,
  which changed the pin *and* everything else that resolves. The `Xid` move is attributable to
  **the arm**, not to the pin specifically. Pre-registered as §2.1 of the prereg, and it stands.
- ⊘ **`refusals=255` is a real, unpaid cost** (§1.4) and no row here says whether it matters.
- ⊘ **`unwitnessed` rose** (§1.3): the table is **more** complete, not complete.
- ⊘ **R10b `CAPPED = 0` is not exercised** — `pages.len() == 1` per doorbell, so the cap was never
  approached. Its correctness still rests on `pushbuffer_pin_tests`, not on this boot.
- ⊘ **The `stride` row still reads `n/a (fewer than two extents)`** on every line. Unexercised.
- ⊘ **`by-executor=39` bounds the fix to pages resident at doorbell time** (§1.2). A page-table
  page first written after its doorbell would still be missed; untested.
- ⊘ **Nothing measures which VAS the host channel is bound to.** Still `[NOT MEASURED]`, exactly
  as `w264` §5 recorded — and the pin says so in its own output.
- ⊘ **The `on` arm's 8 `Xid` are 8 channels on ONE page**, so they are 8 *observations* of one
  missing PDE, not 8 independent facts.

---

## 4. ⊘ WHAT THIS RUN CANNOT PROVE

- ⊘ **That pinning the semaphore page would retire a completion.** `CE-SUBMIT → RETIRED` is `0`,
  as in ~128 logs. §0.1 names the **next** address, not a finished plane.
- ⊘ **Anything about `CUP2_RC`.** 124 on both, pre-registered at zero movement, **fifth
  consecutive lane to predict zero and measure zero.** ★ That remains the right prediction: no
  address-table fix can retire a semaphore nothing submits.
- ⊘ **Leg B vs leg A2** — the brief's ITEM 2. **Not done, and deliberately**: both arms carry
  both, and separating them needs a boolean on `plan_engine_object`'s public signature. Said
  plainly, as the brief asked.
- ⊘ **That the 255 `StraddlesLiveBinding` refusals are benign.**
- ⊘ **Whether the fix survives a workload with more than one pushbuffer page per doorbell** — the
  `39de9b9` sample-vs-count fix is in this binary, but the population that would exercise it is
  still absent.
