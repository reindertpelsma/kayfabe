# w266 — **THE HOST GPU STOPPED FAULTING.** 8 `Xid` → **0**, and the completion still never lands

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Two arms from **one** binary, source revision
> **`f09aba2`**, stamp-gated against the binary before booting
> (`STAMP: [kayfabe-rev:f09aba29…] WANT: [kayfabe-rev:f09aba29…]` → `PASS`), content-checked on
> 10 strings, and **four arming assertions per arm read out of that arm's own log** — the
> variable under test *and* the three carried ones (`GUEST-SEMA`, `EXEC-WITNESS`,
> `GUEST-PUSHBUF`, all **PASS** on both arms).
> Bench `vh`, `NVIDIA GeForce RTX 3060` (GA106), host driver **580.159.04**.
> Graded against `docs/design/w266_completion_pin_prereg.md`, committed at `2cbe7b8`,
> **before the code existed**. `BUILD_RC=0`, both `BOOT … RC=0`, `EXIT rc=0`, `ENOSPC/LLVM = 0`
> from the same invocations. Branch `leg-5-completion-pin`.

---

## 0. ★★★★★ THE HEADLINE — the wall is **no longer a fault**, and it is **still a wall**

**One variable, `KAYFABE_GUEST_SEMA` `off`→`pin`. One page pinned. Everything else identical.**

```
off:  8 × NVRM: Xid 31 … MMU Fault: ENGINE CE3/CE2 HUBCLIENT_CE1/CE0
                faulted @ 0x2_0440f000 … FAULT_PDE ACCESS_TYPE_VIRT_WRITE
on:   (nothing)
```

| field | `off` | `on` |
|---|---|---|
| host `Xid` count | **8** | **0** |
| engine | `CE3` / `CE2` | — |
| client | `HUBCLIENT_CE1` / `CE0` | — |
| distinct fault addresses | 1 (`0x2_0440f000`) | **0** |
| access type | `ACCESS_TYPE_VIRT_WRITE` | — |

⇒ **Three campaigns of host MMU faults end here.** `w263`/`w264`/`w265_off` faulted reading the
pushbuffer; `w265_on` faulted writing the completion; `w266_on` **does not fault at all**.

### 0.1 ★★★★★ AND THE `0` IS MEASURED, NOT AN EMPTY ARTEFACT — this is the trap and it was checked FIRST

`run_w266_on_hostdmesg.log` is **zero bytes**, which is precisely the shape `CLAUDE.md` warns
reads as benign. It is a real absence, and three independent facts say so:

```
[boot_capture:w266_off] host dmesg watermark: 961 lines
[boot_capture:w266_off] host dmesg delta: 8 lines, 8 NVRM, 8 Xid
[boot_capture:w266_on]  host dmesg watermark: 969 lines     ← 961 + off's own 8
[boot_capture:w266_on]  host dmesg delta: 0 lines, 0 NVRM, 0 Xid
```

1. **The watermark advanced by exactly 8** between the arms — the `off` arm's own faults — so
   `dmesg` was read successfully on both.
2. **The success branch RAN.** `boot_capture.sh:310-320` prints `host dmesg delta:` only from
   the arm that read `dmesg`; the failure arm prints `the HOST's dmesg was UNREADABLE`. The
   `on` boot printed the former.
3. **The probe log states the number either way**: `HOST_DMESG_LINES=0 HOST_DMESG_NVRM=0
   HOST_DMESG_XID=0`, beside `off`'s `=8 =8 =8`.

⊘ The harness's own comment anticipated this exact reading and refuses to assert non-emptiness,
*"because an emptiness assertion would fail a good boot and pressure the next reader into fixing
a harness that was telling the truth."* This is that boot.

### 0.2 ⊘⊘ AND THE COMPLETION STILL DOES NOT LAND — **byte-identically to the control**

```
COMPLETION-WATCH proc=2 chan=0 va=0x20440fff0 payload=0x00000001
  → NOT-OBSERVED samples=88 last_seen=0x00000000        ← IDENTICAL on both arms
```

Eight watches, `samples` **88/87/86/85/84/83/82/81** on **both** arms, `last_seen=0x00000000` on
both, `CUP2_RC = 124` on both. **Zero verdicts moved.**

★★★ **And the observer is watching the page that was pinned — this is not a mismatch.** The
declares resolve `va=0x20440fff0 → GuestRam { gpa: 563605488 }` = **`0x2197eff0`**, and the pin
placed `va=0x20440f000 gpa=0x2197e000 len=4096`. Same page, offset `0xff0`. ⇒ Had anything
written payload `1` at the watched address, the observer would have read it — it read `0`
**88 times**. ⊘ *(Note in passing: the descent and the address table **agree** about this page's
GPA on this boot. `w264` measured them disagreeing; the witness arm is what closed that.)*

⇒ ★★★★★ **THE PRE-REGISTERED SEPARATION, MEASURED IN ITS SHARPEST FORM: the page became
writable and nothing observable was written to it.** §2.3 of the prereg asked for exactly this
distinction and predicted exactly this pair. *"Writable"* and *"the guest's wait was satisfied"*
are not the same fact, and this boot is the cleanest demonstration the campaign has: **0 faults
and 0 completions, simultaneously.**

---

## 1. THE PRE-REGISTERED SCORECARD

★ Both arms differ in **exactly one** variable, and each arm's arming is asserted out of its own
log by the harness rather than by the shell.

| # | observable | pred `on` | **off** | **on** | |
|---|---|---|---|---|---|
| S1 | `GUEST-SEMA arm=` | `pin` | `off` | **`pin`** | ✔ assertion PASS both |
| S2 | `SEMA-PIN` lines | ≥1 / **0** on `off` | **0** | **16** | ✔ the control printed nothing |
| S3 | `SOURCE … declared completion(s)` | **8** | — | **8** | ✔★★ the reader works |
| S4 | pages after de-duplication | **1** | — | **1** | ✔★★★ eight 16-byte targets are ONE page |
| S5 | `SEMA-TABLE … resolved in guest RAM` | **1** | — | **1** | ✔★★★ **§0.2's falsifier held** — the GR-declared VA resolves in the **CE** channel's pdb |
| S6 | `SEMA-TABLE … MISS` | **0** | — | **0** | ✔ |
| S7 | `SEMA-TABLE … NOT-IN-GUEST-RAM` | **0** | — | **0** | ✔ `miss = fault` held |
| S8 | `sema run … PINNED` (fresh) | **1** | 0 | **1** | ✔ |
| S9 | `sema run … ALREADY PINNED` | **7** | 0 | **7** | ✔★★ eight channels, one page, one pin |
| S9b | `sema run … REFUSED` | 0 | 0 | **0** | ✔ |
| S10 | `sema run … placed_as_asked=true` | 8 | 0 | **8** | ✔ `host_va=0x20440f000` = the guest's own VA |
| S11 | negative control `REFUSED BY NAME` | yes | — | **8** (`NoStatedRun` @ `gpa=0x80001000`) | ✔ |
| S11b | `NO PAGE TO PIN` | 0 | 0 | **0** | ✔ the ordering risk did not materialise |
| S11c | `SEMA: NOT ONE PAGE RESOLVED` | 0 | 0 | **0** | ✔ |
| S12 | **host `Xid` COUNT** | 8→? | **8** | **0** | ★★★★★ ⊘ *the row I marked BLIND is the row that fired* — §3.1 |
| S12a | `Xid` ENGINE | — | `CE3`/`CE2` | **(none)** | ★★★★★ |
| S12b | `Xid` CLIENT | — | `HUBCLIENT_CE1`/`CE0` | **(none)** | ★★★★★ |
| S12c | `Xid` DISTINCT ADDRS | `0x2_0440f000` GONE | **1** | **0** | ★★★★★ **the rung's own claim, met** |
| S12d | `Xid` ACCESS TYPE | — | `VIRT_WRITE` | **(none)** | ★★★★★ |
| S13 | `COMPLETION-WATCH … OBSERVED` | **0** | **0** | **0** | ✔ predicted, and it is the wall |
| S13b | `COMPLETION-WATCH … NOT-OBSERVED` | 8 | **8** | **8** | ✔ |
| S13c | `last_seen` | `0x0` | `0x00000000` | **`0x00000000`** | ✔ ⊘ 0 and "never read" are different facts; `samples=88` says it was read |
| S13d | `COMPLETION-DECLARE` | 8 | **8** | **8** | ✔ |
| S14 | **`CUP2_RC`** | **124** (`p=.72`) | 124 | **124** | ✔★ **sixth consecutive predicted zero, sixth measured zero** |
| S15 | `CE-SUBMIT` / `RETIRED` | 0/0 | 0/0 | **0/0** | ✔ |
| S16 | `PB-PIN token=` lines | 16 both | **16** | **16** | ✔ guard — leg 4 unchanged |
| S17 | `PT-DECODE` 1st pass | 19 615 / 19 874 | `bound=19615 unwitnessed=19874` | **identical** | ✔★★ **cross-revision guard PASSES** |
| S18 | `refusals=` | 255 both | **255** | **255** | ✔ the carried debt is unchanged — this rung touched nothing |
| S19 | `RmInitAdapter failed` | 0 | 0 | **0** | ✔ guest alive on both |
| S20 | guest `NVRM` / `GR-BIRTH` | 31/24 | 31/24 | **31/24** | ✔ guard |
| S21 | `ENGINE-OBJECT` (last) | 34/32/2 | 34/32/2 | **34/32/2** | ✔ guard |
| S22 | doorbells `REFUSED` / heartbeat | 16 both | 16 / 5 | **16 / 5** | ✔★ §0.1 of the prereg — the refusal is unchanged |
| — | `EXEC-WITNESS` / `by-executor` / `wit=` | — | ARMED / 39 / 37 | **identical** | ✔ |
| — | `RING-PROJ` / `adopt=GUEST-RING` / `userd=GUEST-USERD` / `BAR1 GP_PUT` | — | 8 / 16 / 16 / 66 | **identical** | ✔ |

★★ **Every guard is byte-identical.** The two boots did the same work, rang the same doorbells,
took the same 88 observer samples — and one of them provoked eight hardware faults and the other
provoked none.

---

## 2. ⊘⊘ WHAT THIS RUN REFUTED, INCLUDING IN ITS OWN PRE-REGISTRATION

### 2.1 ★★★★★ THE BRIEF'S MECHANISM — pre-registered as refuted, and the boot confirms it

The prereg (§0.1) recorded that the eight doorbells carrying the pin are **`DOORBELL-REFUSED …
[FwdFault::PushbufferAperture]`** and that this device rings nothing — the host channels fetch
autonomously because legs A2+B gave them the **guest's own ring and USERD**. `w266` reproduces
that exactly: `DOORBELL-REFUSED = 16` on both arms, `CE-SUBMIT = 0`, and the hardware still
faulted on `off` and still executed on `on`.

⇒ ★ **This is now load-bearing rather than a curiosity: the forwarding plane's refusal is not on
the critical path.** A rung that "fixes" `FwdFault::PushbufferAperture` would be fixing something
the engine is already routing around.

### 2.2 ⊘ MY OWN `p = 0.6` WAS TOO LOW, AND IN THE FAVOURABLE DIRECTION — for the second rung running

I predicted the fault would *move to a new address* at `p=.6`, with the residual mass on *"the
host channel is not bound to a VA space in which those VAs resolve"*. **It did not move; it
vanished.** There was no next address.

⚠ That is the second consecutive rung where the outcome I gave the least weight is the one that
happened, and both times in the favourable direction (`w265` §1.1 records the first). ⇒ **My
priors on the supply side are systematically pessimistic.** The correction is not "predict
better outcomes" — it is that I keep modelling *"one more missing thing"* as the default when the
measured pattern is *"the last missing thing on this plane."*

### 2.3 ⊘⊘ THE GRADER I WROTE HAS THREE CONTAMINATED ROWS — found in its own output

`R8`, `R10` and `R11` are inherited leg-4 rows whose regexes match **both** pin passes, so on the
`on` arm they read `16`, `16` and `20` against the `off` arm's `8`, `8` and `12`:

| row | regex, as written | off | on | after the fix (both arms) |
|---|---|---|---|---|
| `R8` | `[1-9][0-9]* page\(s\) asked, [1-9][0-9]* resolved` | 8 | **16** | **8** |
| `R10` | `→ (PINNED\|ALREADY PINNED)` | 8 | **16** | **8** |
| `R11` | `placed_as_asked=true` | 12 | **20** | **8** |

⇒ **Leg 4 did not change; the grader started counting leg 5.** I renamed the sema pass's `TABLE:`
to `SEMA-TABLE:` *before the boot* precisely for this class and then **only fixed the S-rows**,
leaving the R-rows to swallow the new lines. ★ The durable lesson is narrower than "labels
matter": **adding a producer silently re-scopes every pre-existing consumer that was implicitly
scoped by being the only producer.** The `R7b PB-PIN token=` row — `16` on both arms — is what
exonerates leg 4, and it survives only because it greps a **token**, not a shape.

⊘⊘ **And fixing it exposed that `R11` was ALREADY wrong before leg 5 existed.** Its corrected
value is **8**, not 12: the other four `placed_as_asked=true` come from `GR-RING-JOIN` and three
`GR-FB-JOIN` lines — a *different plane*. So at `w265` that row was summing **three** producers,
and its RESULT's *"`4 → 12`, +8, and only a rise above 4 is this rung's"* was arithmetically
right only because a human did the subtraction. ★ **A row that needs a subtraction to be read is
already a row that will be misread**, and the version that needed none (`R7b`) is the one that
survived contact with a new producer.

⚠ One trap inside the fix: the obvious anchor `TABLE:` **does not work**, because `SEMA-TABLE:`
*contains* `TABLE:` — the row would have stayed contaminated while looking repaired. The
corrected regex is `[^-]TABLE:`.

⊘ **Fixed at `w266_grade.sh`; every R-row is now anchored to `pb run` / `PB-PIN`, and the numbers
in the scorecard above are from the corrected rows, re-run against the same logs.**

### 2.4 ⊘ Known-artifact rows, restated so nobody reads them

- `R5`/`R6` (`unwitnessed=0 bound=8`) use `last`, and the final `PT-DECODE` line of every boot
  reads zero because only the first pass decodes. **The real first-pass numbers are in `S17`**,
  and they reproduce `w265` exactly.
- `R17`'s `first` row reads `seen=1 forwarded=0 refused=1`; the census's **last** line is the
  number, and it is `34/32/2` on both. Inherited from `w265`, still an artifact, still stated.

---

## 3. ⊘ WHERE THIS RUN IS WEAKER THAN IT LOOKS

### 3.1 ★★★★★ **NOTHING HERE READS THE PAGE.** "No fault" is not "the write landed"

This is the single most important qualification and it is the next rung.

`0 Xid` is consistent with **two** stories and this boot cannot separate them:

- **(a)** the engine wrote the semaphore page successfully — the mapping resolved, the write
  landed, and it landed at a slot **nobody is watching**;
- **(b)** the engine did not attempt the write at all this boot.

★ The evidence leans hard on **(a)**: the fault was `FAULT_PDE` at *exactly* the VA a PDE was
installed for, every other observable is byte-identical, and the CE channels' own pushbuffer is
a semaphore release — `[0] sub4/m0x0 = 0xc7b5` (`SET_OBJECT`, copy engine), `[1] sub4/m0x240
n=3` (`SET_SEMAPHORE_A/B/PAYLOAD`, upper dword `0x2`), `[2] sub4/m0x300 n=1 = 0x14`
(`LAUNCH_DMA`). ⊘ But leaning is not measuring.

⇒ **And (a) has a sting**: the CE's `SET_SEMAPHORE` target and the GR channels'
`SET_REPORT_SEMAPHORE` targets are *different addresses in the same page*. The observer watches
the eight GR slots at `…ff80 … …fff0`; the CE writes wherever its own operand points. A write
that landed at a ninth slot would produce **exactly this boot's log**.
★ **The rung that closes it is small and obvious: dump the 4 KiB page after the run**, and
decode the CE's full `SET_SEMAPHORE_A/B` operand rather than the first argument.

### 3.2 Other limits

- ⊘ **The arm is not the page.** One variable is armed and it has one consumer, which is much
  narrower than `w265`'s 13 343 bindings — but attribution is still to **the arm**.
- ⊘ **`pages.len() == 1`**, so `PUSHBUF_MAX_PAGES`, the run coalescer's multi-run path and the
  `CAPPED` arm are all **unexercised** on this boot and rest on the unit tests.
- ⊘ **The 255 `StraddlesLiveBinding` refusals** are unchanged and still unpaid.
- ⊘ **`by-executor=39`** still bounds the leg-4 fix to pages resident at doorbell time.
- ⊘ **Nothing measures which VAS the host channel is bound to.** Still `[NOT MEASURED]` — though
  the disappearance of a `FAULT_PDE` at a VA we mapped is the strongest *indirect* evidence yet
  that it is the right one.
- ⊘ **Ordering was not stressed.** All eight declares preceded the first CE doorbell, as at
  `w265`. `NO PAGE TO PIN = 0` means the risk did not materialise, not that it cannot.

---

## 4. ⊘ WHAT THIS RUN CANNOT PROVE

- ⊘ **That the semaphore was written.** §3.1. No row reads the page's contents.
- ⊘ **That the completion plane is closer to retiring.** `CE-SUBMIT → RETIRED` is `0`, as in
  ~129 logs, and nothing here submits.
- ⊘ **Anything about `CUP2_RC`.** 124 on both, pre-registered at zero movement with the three
  reasons named in prereg §2.2, all three still standing: the unresolved
  `SET_SHADER_SHARED_MEMORY_WINDOW` operand, the exact-slot question, and the guest's own
  coherent view of the page.
- ⊘ **That the guest's CPU can SEE an engine write to a pinned page.** `COMPLETION-WATCH` reads
  through the **VMM's** view of guest RAM, not through the guest's mapping. Both are the same
  memfd, so this is *likely*, and it is not measured.
- ⊘ **That leg 5 is done.** The page is presented. The completion is not.

---

## 5. THE NEXT RUNG, and it is one page of diagnostic

1. ★★★★★ **DUMP `0x20440f000` after the run** — 4 KiB, from the VMM's own view, at teardown and
   ideally on each `COMPLETION-WATCH` verdict. That single row separates §3.1(a) from §3.1(b),
   and if the page holds a `1` anywhere it also names the slot the engine actually chose.
2. ★★ **Decode the CE channel's own `SET_SEMAPHORE_A/B/PAYLOAD` operand in full.** The
   pushbuffer dump prints only the first of three arguments, so the address the copy engine is
   releasing to is on disk and unread.
3. ⚠ **Widen the watch** to any 4-byte slot in a declared page rather than only the eight
   declared VAs — ⊘ *after* (1), not instead of it: guessing at slots without reading the page
   would be the same class of error as decoding a `dlen=0` row to zeros.
