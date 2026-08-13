# w268 — ★★★★★ **THE GR ENGINE FETCHED, THE GR WORK RAN, AND ALL EIGHT COMPLETIONS LANDED.** `CUP2_RC` is still `124`

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Two arms from **one** binary, source revision
> **`70463ae`**, stamp-gated against the binary before booting
> (`STAMP: [kayfabe-rev:70463ae3…] WANT: [kayfabe-rev:70463ae3…]` → `PASS`), content-checked on
> **21** strings including this rung's own eight, and **five** arming assertions per arm read out
> of that arm's own log (`GUEST-SEMA`, **`GR-ROUTE`**, `EXEC-WITNESS`, `GUEST-PUSHBUF`,
> `GR-CURSOR`) — all **PASS** on both. Bench `vh`, `NVIDIA GeForce RTX 3060` (GA106), host driver
> **580.159.04**. Graded against `docs/design/w268_the_cursor_and_the_arm_prereg.md`, committed
> at **`87c0d0f`, before either instrument existed** (instruments landed at `65fe5ca`, harness at
> `c14075f`). `BUILD_RC=0`, both `BOOT … RC=0`, `EXIT rc=0`, `ENOSPC/LLVM = 0` from the same
> invocations. Branch `w268-the-cursor-and-the-arm`.

---

## 0. ★★★★★ THE HEADLINE — the owner's question is answered, and the answer moved the wall

**The owner asked for a three-way discriminator: *"if the GPU even tried running, `GP_GET`
should advance."* It was built as a call site, not a capability, and it fired on both arms.**

### 0.1 The `refuse` arm — the shipping configuration. **U1: the engine NEVER fetched.**

```
GR-CURSOR token=0x00000007 proc=2 chan=0 engine=GrCompute why=doorbell LATCHED  GET=0 PUT=1
GR-CURSOR token=0x00000007 proc=2 chan=0 engine=GrCompute why=first  t=+71006ms GET=0 PUT=1
GR-CURSOR token=0x00000007 proc=2 chan=0 engine=GrCompute why=CHANGED t=+238095ms GET=0 PUT=0
```

**All eight `GrCompute` channels, no exceptions**: `PUT = 1`, `GET = 0`, and `GET` **never
becomes non-zero at any tick** (`U2 = 0`). The `PUT = 1 → PUT = 0` transition at `t=+238 s` is
the guest tearing down after `cup2` gave up, **167 seconds** after it submitted.

⇒ ★★★ **U1, pre-registered at `p = 0.70`.** The guest submitted (`PUT > 0`) and the host engine
never fetched (`GET == 0`) for 167 seconds. The zero completion is downstream of a cause
upstream of it.

★★ **And the reading has an internal known-positive.** `[measured, same arm, same reader
function]` `fbuserd@0x1026088 GET=1 PUT=1` — a **copy-engine** channel, which *did* fetch. ⇒
`GET = 0` on the eight GR channels is a fact about those channels, not about the instrument. A
census zero with a known-positive beside it, in one arm, from one function.

### 0.2 The `pass` arm — one variable. **The engine fetched within 32 ms.**

```
GR-CURSOR proc=2 chan=0 why=first   t=+70857ms  GET=0 PUT=1
GR-CURSOR proc=2 chan=0 why=CHANGED t=+70889ms  GET=1 PUT=1     ← 32 ms later
```

**`GET` caught `PUT` on all eight GR channels** (`U2 = 8`). ⇒ **A3 fired**, pre-registered at
`p = 0.35`.

### 0.3 ★★★★★ AND THE THING `cuCtxCreate` POLLS WAS WRITTEN — for the first time in this campaign

`w267` measured the GR half of the completion page (`+0xf80…+0xff0`) as **zero on every dump of
both arms**. On this arm it is written, all eight slots, each with `payload = 1` and a **distinct
GPU timestamp**:

```
SEMA-PAGE-SLOT va=0x20440ff80 proc=2 chan=7 kind=GR-REPORT-SEMAPHORE report16=[0x1,0x0,0xfb910000,0x18cb08c1]
SEMA-PAGE-SLOT va=0x20440ff90 proc=2 chan=6 kind=GR-REPORT-SEMAPHORE report16=[0x1,0x0,0xf97ce400,0x18cb08c1]
SEMA-PAGE-SLOT va=0x20440ffa0 proc=2 chan=5 kind=GR-REPORT-SEMAPHORE report16=[0x1,0x0,0xf7d98400,0x18cb08c1]
…all eight, +0xf80 … +0xff0
```

and the observer says so in its own vocabulary: **`COMPLETION-WATCH → OBSERVED` = 8** against
`NOT-OBSERVED = 8` on the control. The page fills in real time as the eight reports land:

```
seq=1 t=+70856ms nonzero=24/1024      ← the 8 CE reports (the ordering fix, §2)
seq=3 t=+70913ms nonzero=30/1024
seq=5 t=+70960ms nonzero=36/1024
seq=8 t=+71034ms nonzero=45/1024
seq=9 t=+71286ms nonzero=48/1024      ← 16 reports × 3 non-zero words. EVERYTHING wrote.
```

⇒ ★★★★★ **A5 fired, and it was pre-registered at `p = 0.07`.** The completion plane
`cuCtxCreate` waits on is **closed**.

### 0.4 ⊘ AND `CUP2_RC` IS STILL `124`, ON BOTH ARMS — pre-registered at ZERO movement, `p = 0.88`

**Eighth consecutive rung to predict zero movement and measure zero.** `cup2`'s own output is
**byte-identical** between the two arms (`diff` of the hook sections: only the timestamp and the
host-dmesg delta differ). It still reaches `cuDeviceTotalMem` and hangs.

⇒ ★★★ **This is the single most informative negative this campaign has produced**, because it
is the first one taken *behind* a satisfied completion. *"The guest is waiting for a semaphore
nobody writes"* is **retired as an explanation**. Eight semaphores were written, at the guest's
own declared addresses, with payloads the guest itself specified, and the guest did not proceed.

---

## 1. ⊘⊘ WHAT CONTRADICTS THE BRIEF — and the largest one is what made this rung possible

### 1.1 ★★★★★ The reason the GR route was disarmed was TRUE ON 08-11 AND FALSE ON 08-12

`gr_doorbell_passthrough.md` §0.3 keeps `KAYFABE_GR_ROUTE=refuse` on two reasons it calls *"both
in the code, neither is a guess"*: the host GR channel's **ring** is ours, and its **`GP_PUT`
cursor** is ours — therefore *"`GP_PUT == GP_GET` forever ⇒ the engine fetches nothing"*.

⊘⊘ Both premises are refuted by **`w267`'s own committed log**, the boot the brief hands me.
`[measured, all 16 `GR-BIRTH iso2` lines]` `8 × engine=Ce` **and `8 × engine=GrCompute`**, every
one `adopt=GUEST-RING userd=GUEST-USERD → alloc_channel_over_guest_ring`. Legs A2 and B landed
at `w261`/`w262` and §0.3 had not been re-read since. The same sentence was duplicated as a
comment in **two** places in `shim.rs`, so a reader met it three times.

★★★ **A ruling's DATE is part of the citation.** Every sentence in §0.3 carries a file, a
function and a measured boot. It is impeccably sourced and it was out of date, and it is the one
document that would have stopped this rung. All three copies are corrected **in place, above the
text they correct**, at `65fe5ca`.

### 1.2 ★★★★★ The brief's item-2 Q1 — "do the GR pushbuffer pages get pinned?" — No, and not for want of a source

All three pin passes live in `DoorbellPort::ring` **below** `try_ce_submission`, and on the
shipping arm a `GrCompute` doorbell is terminated *inside* it by `RefuseByRoute`.
`[measured, w267_on and w268_refuse]` **every** `PB-PIN`/`SEMA-PIN` line names a `Ce` token;
**zero** name any of the eight `GrCompute` tokens.

⇒ ★ **Arming the route IS giving the pins their GR source**, and the boot shows it with no extra
work: `PB-PIN` distinct tokens **8 → 16**, `SEMA-PIN` distinct tokens **8 → 16**. They were never
two rungs.

### 1.3 ★★★★★ The brief's item-2 Q2 — `CE-SUBMIT → RETIRED = 0/0` is not the answer

`CE-SUBMIT` is the **isolate's own emulated copy** and is not what runs a channel. What runs one
is `SharedDevice::doorbell` → `VerbPlan::Doorbell` → **`rm.schedule` then `rm.ring_doorbell`** —
the tree's only `ring_doorbell` call site — and it executes **inside `verb_op`, before
`forward_ring`**.

⇒ ⊘⊘ **`DOORBELL-REFUSED [FwdFault::PushbufferAperture]` is a POST-HOC refusal**: the host
doorbell had already been rung. That retires `w266`'s standing puzzle (*"the eight doorbells are
REFUSED and the hardware executes anyway"*) — it executed because it was rung.
And `ring_content_is_forwardable` is `CpuCe`-only, so a GR doorbell reaching the same function
**skips `forward_ring` entirely** and returns `Served`. Measured: `Route::NotACopyEngineChannel`
**9 → 0**, `DOORBELL-REFUSED` **16 → 9**, and the eight GR tokens now appear in `RING-PROJ`.

⇒ The answer to item 2 Q3 is **a missing ARM**, and the arm supplied the missing source.

---

## 2. THE ORDERING FIX (brief item 1) — closed, on both arms, exactly as pre-registered

| # | prediction | `refuse` | `pass` | |
|---|---|---|---|---|
| **O1** | `NO PAGE TO PIN` 4 → 0, `p=.80` | **0** | **0** | ★★★ **FIRED** |
| **O2** | `SEMA-SOURCE-CE` on 8 channels, `p=.85` | **8** | **17** (8 CE + 9 doorbells) | ★★★ **FIRED** |
| **O3** | `nonzero = 24/1024`, `p=.55` | **24/1024** | **24/1024** at seq 1 | ★★★★★ **FIRED, exactly** |
| **O4** | `Xid` 4 → 0 by identity, `p=.60` | **0** — no `Xid` line at all | 1, and it is a **different** fault (§3.2) | ★★★ **FIRED on the control** |
| **O5** | the two sources agree, `p=.90` | ✔ | ✔ | ★ |

Each channel recovers **its own** offset, from **its own** bytes:

```
SEMA-SOURCE-CE token=0x0001000f chan=8  → methods=3 launches=1 opaque=1 release_target(s)=1 [0x20440ff70]
SEMA-SOURCE-CE token=0x00010010 chan=9  → …[0x20440ff60]   … ff50, ff40, ff30, ff20, ff10, ff00
SEMA-SOURCES: 1 page(s) to pin in total, of which 1 came ONLY from this channel's own pushbuffer
```

★★ **`of which 1 came ONLY from this channel's own pushbuffer` fires on all 8** — i.e. on every
CE doorbell of this boot the declared source was still empty and the new source was the *only*
one. The race `w267` hit 4 times out of 8 hit **8 times out of 8** here, and was invisible
because the fix covers it.
⊘ Eight distinct offsets and no shared constant: nothing was recalled, which is the `cap2b`
hazard the brief named as the one way this could go badly wrong.
⊘ `release_target(s)=0 []` on the `pass` arm's **GR** doorbells — the CE codec is class-gated and
correctly finds nothing in a compute pushbuffer. The negative control is in the same column.

---

## 3. ⊘ WHERE THIS RUN DEVIATES FROM ITS PREDICTIONS

### 3.1 ⊘⊘ A4 DID NOT FIRE — I predicted the wrong fault, and the right one is more interesting

I pre-registered (`p = 0.40`) that a fetching GR engine would fault **reading its own pushbuffer**
at `0x2_00400000`, because the GR pushbuffer leaf is joined by neither `GR-RING-JOIN` (the ring's
leaf) nor `GR-FB-JOIN` (the operand census's leaves).

⊘ **No such fault occurred.** The GR work fetched, ran and completed without a single read fault.
⇒ My model of which leaf the GR pushbuffer lives in was wrong, or the leaf is reachable by a
path I did not enumerate. **Unexplained, and it is a debt this rung creates.**

### 3.2 ★★★ THE ONE `Xid` ON THE `pass` ARM IS A NEW WALL AT A NEW ADDRESS

```
Xid 31 … channel 0x01000011, ENGINE CE2 HUBCLIENT_CE0 faulted @ 0x2_04420000
       FAULT_PTE ACCESS_TYPE_VIRT_WRITE
```

Graded as **identity, never as a count** (`w265`'s lesson): engine `CE2`, client `HUBCLIENT_CE0`,
**one** distinct address, `VIRT_WRITE`.
★★ **`0x2_04420000` is a page that appears NOWHERE ELSE in the boot** (`grep -c 4420000` = **0**):
it is not the completion page `0x2_0440f000`, not a declared `SET_REPORT_SEMAPHORE`, not a CE
release target, not a pinned pushbuffer VA. **No source in this device has ever named it.**

⊘ And it arrives **after** real work: the second doorbell on `chan 8` carries `methods=11
launches=3` over **two** GPFIFO entries, where every earlier CE submission of this boot was the
32-byte release-only shape (`methods=3 launches=1`). ⇒ **The guest issued its first substantive
copy-engine work once the GR context init completed, and that work faults writing to a page
nobody pins.** That is the wall, moved one plane on.

### 3.3 ⚠ THE INSTRUMENT'S OWN LIMITS, and one of them cost rows

- ⊘⊘ **`GR-CURSOR-READER stopped` NEVER PRINTS, on either arm** — pre-registered at `p = 0.85`
  and inherited unfixed from `w267` §3.2: `close()` runs only when the observer loop exits, and
  QEMU is powered off without `detach_ram`. **`PAGE-READER ASSERTION: FAIL` on both arms too.**
  ⊘ The finding survives without it: the observer demonstrably ran to **`t=+238599ms, tick=960`**
  (a `CHANGED` row at that instant), so *"`GET` never moved on `refuse`"* is bounded by **167
  seconds of continuous observation**, not by a tally. But the tally is the row that would have
  made it one line instead of an argument, and it is still owed.
- ⚠⚠ **STDERR INTERLEAVING CORRUPTS SOME ROWS.** The observer thread's `eprintln!` and QEMU's own
  `-msg timestamp=on` writer interleave mid-line: on `refuse`, **6 of 8** `why=first` rows are
  spliced with a `DOORBELL-REFUSED` line (`U0c` reads **2**, not 8). The `why=doorbell` rows (8/8)
  and the `why=CHANGED` rows (8/8) are intact and carry the same pairs, so nothing is lost — but
  a grader row that counted only `why=first` would have under-reported by 75 %. ★ New, measured,
  and not previously recorded in this tree.
- ⊘ **`run_w268_refuse_hostdmesg.log` is ZERO BYTES**, which is the artefact this repo's CLAUDE.md
  names as the most expensive trap. It is **correctly** zero: the probe log's own watermark line
  says `981 → 981`, `HOST_DMESG_LINES=0`, `HOST_DMESG_XID=0`. ⇒ *"nothing happened"* is
  distinguished from *"nothing was recorded"* by a second artefact, as designed.
- ⊘ `S3`/`S4` read `SOURCE 0 declared completion` / `0 page(s) after de-duplication` on both arms:
  that is the **first** occurrence, which is the CE doorbell that arrives before any declare —
  the race itself, now harmless. It is not a regression.

### 3.4 THE CARRIED GUARDS

`R6f PT-DECODE bound (1st)` = **19618** on **both** arms — unchanged from `w267`, so the
cross-revision comparison to `w267` holds. (I predicted it would move again at `p = 0.55`;
⊘ it did not.) `unwitnessed=19874`, `refusals=255`, `by-executor=39`, `wit=37`, `resident=156`
all unchanged. ⚠ `R5 PT-DECODE unwitnessed (max)` differs **between arms** (`0` vs `2048`) —
expected, since the `pass` arm runs the populate pass on 8 more doorbells, but **not separately
measured**.

---

## 4. ⊘ WHAT THIS RUN CANNOT PROVE

1. ⊘ **That the GR work was CORRECT.** `GET` advancing and a payload landing say the engine
   fetched and signalled. Nothing here inspects what it computed, and this rung has no oracle
   for that.
2. ⊘ **Why `cup2` still hangs.** The completion it declared was written and observed; what it is
   *now* waiting on is unmeasured. §3.2's new fault is the leading candidate and is not proven
   to be on the guest's critical path.
3. ⊘ **That `GET == 0` on the `refuse` arm means "never scheduled".** It is equally consistent
   with *scheduled but never rung*. Nothing here reads a runlist. (The mechanism at §1.3 says
   neither `schedule` nor `ring_doorbell` is reached on that arm, but that is a code reading, not
   this boot's measurement.)
4. ⊘ **That the arm is safe to default.** Two boots is not a posture change. `refuse` remains the
   default, unchanged by this rung.
5. ⊘ **That the guest's CPU sees the reports.** `gpa_read` reads the **VMM's** view of the memfd.
   Carried unpaid from `w266` §4.
6. ⊘ **Ordering finer than 250 ms**, and nothing about writes that did not change bytes.
7. ⊘ **255 `StraddlesLiveBinding`**, **`by-executor = 39`**, **host-channel VAS `[NOT MEASURED]`**
   — all untouched, all still owed.

---

## 5. THE NEXT RUNG

1. ★★★★★ **FOLLOW `0x2_04420000`.** It is the only fault in the boot, it is at a page no source
   in this device has ever named, and it arrives on the guest's **first substantive CE work**
   (`methods=11 launches=3`, two GPFIFO entries). Find who names it — the second GPFIFO entry's
   own methods are the place to look, and the `pbm` print is bounded to the first extent today.
2. ★★★★ **Ask what `cuCtxCreate` is waiting on NOW.** Its declared completion was satisfied and
   it did not proceed, so the next wait is a different one. ⊘ This is a *new* question: every
   earlier rung's answer was *"the semaphore was never written"*, and that answer is retired.
3. ★★ **Fix the teardown so `close()` runs** (`w267` §3.2, unpaid again) — or delete the `final`
   arm and stop asserting a row nobody can get.
4. ⚠ **Explain §3.1**: the GR engine read a pushbuffer through a leaf I believed was joined by
   nothing. Either the leaf list is wrong or the reachability argument is.
5. ⊘ **Do NOT default the route.** `refuse` stays default until (2) is answered.
