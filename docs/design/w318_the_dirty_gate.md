# w318 — THE LAUNCH FLOOR WAS REPETITION, AND GATING IT IS 16.9× ON THE TRAP AND 19.6× ON THE GUEST'S SUBMIT

**STATUS: LIVE — measured 2026-08-14 on bench `vh2` (RTX 3060 GA106, driver 580.159.04),
source revision `c6301a57`, stamp gate passed on every boot.** Artifacts:
`traces/w318_gate/`.
⊘ `vh2` is itself a KVM guest — our guest is at **L2**. w315 §4 measured that tax at
**≤ 2.2 ms per launch** and it is not what this rung moved.

> ★★★★★ **PRE-REGISTERED OUTCOME: (A) — the gate fires, the trap drops, both workloads green
> at n ≥ 3.**
>
> | | control (gates off) | gated | |
> |---|---|---|---|
> | host doorbell trap, per launch | **77.367 ms** | **4.583 ms** | **16.9×** |
> | guest `submit_med_ms` (`cuLaunchKernel`) | **78.127 ms** | **3.990 ms** | **19.6×** |
> | guest `launch_med_ms` (submit + sync) | 101.767 ms | 27.960 ms | 3.64× |
> | `batch_gflops` | 4.133 | 22.221 | 5.38× |
> | `bad` / `maxerr` | 0 / 0 | 0 / 0 | — |
>
> ★★★ **And the strongest half is not the speed.** The two arms' `bound=` and `published=`
> histograms are **identical row for row on every non-zero row**. The gate removed **87
> doorbells that published nothing and bound nothing**, and removed nothing else. §4.

---

## 0. The one-line answer

**The 91.5 % w315 attributed to page-table and publication work was not page-table work — it
was the same page-table work, done again.** Two consecutive launch doorbells in w315's trace
printed byte-identical `PT-DECODE` lines and byte-identical publication censuses; this rung
armed each pass on the state it actually reads, and 96.4 % of publication passes and 95.6 %
of executor-witness passes stopped running. Nothing that ever bound or published stopped
happening.

---

## 1. What was changed — three arming edges, and why they are not the C's

The C artifact gates exactly this shape (`C: src/qemu/nvkvm_gpu_emul.c:580-583`, `:1399-1400`,
`:284`) and its precedent is what licensed trying. ⊘ **Its design was not transcribed.** The C
arms on *"a tracked page-table page was written"*. Each gate here arms on **the thing the
skipped pass actually reads**, which is what makes the skip provable rather than plausible:

| pass | what it reads | arming edge added |
|---|---|---|
| `publish_vas_rows` | a `Vas`'s table rows and its guest-RAM pins | `Vas::publish_epoch` = `(AddressTable::generation, guest_ram_pins.len())` |
| …and its **host** outcome | whether the range is already joined | `RegPlane::joined_fb_ranges().len()` |
| `witness_executor_fb_pages` | the bytes of every executor-created FB page | `FbStore::writes_by(FbWriter::Executor)` |

`AddressTable::generation` is bumped at the table's **only two write sites**
(`bind` on success, `unbind` when a row left) and nowhere else — total, because every other
`self.map.*` in that impl is a read. ⊘ A **refused** bind and an **empty** unbind are not
changes: both are guest-reachable, and a generation that moved on them would let the guest arm
the gate at will, on exactly the input that produces no new mapping to publish.

`FbStore::writes_by` exists because `FbStore::page_origin` **cannot** answer this question.
Origin is deliberately *first*-writer and its sequence bumps only on page **creation**, so the
executor rewriting a page it already created is invisible to it — and that is the only thing a
re-queue can newly teach a decode. ⇒ **the gate could not have been built on the counter the
tree already had**, and a rung that tried would have shipped a blind spot.

### 1.1 ⚠ Three refusals to skip, enforced rather than argued

1. **`None` is UNMEASURED and ARMS**, everywhere: a store that does not count, a `Vas` that is
   not there, a plane that is gone. There is no default-skip anywhere in `DirtyGate`.
2. **An INCOMPLETE pass never stamps clean.** A publication that hit `VAS_PUBLISH_WALL_BUDGET`
   left candidates unattempted; stamping it would strand them until something else happened to
   move the epoch — a publication silently never performed.
3. **The epoch is RE-READ after the joins.** A successful join binds into the table and moves
   the epoch *during* the pass; stamping the pre-pass value would make the next doorbell see a
   mismatch and re-run, i.e. a gate that can never go clean on any VAS that ever published.

★ Both gates are **off by default**, separately armed (`KAYFABE_DIRTY_GATE_PUBLISH` /
`_WITNESS`), and an unparseable value reads as `off`. Every other arm in this shim is
off-by-default so an *instrument* cannot fire unasked; this one is off-by-default because the
armed direction **removes work**, and `VAS_PUBLISH` ablated **red**.

---

## 2. ★★★★★ THE BREAKDOWN — same instrument, same arms, one variable

Measured with **w315's own scripts, unmodified** (`w315_floor.sh full` → `w315_align.py`), so
the numbers are comparable to w315's table by construction rather than by claim. Twelve
matched launch doorbells per arm.

```
                        CONTROL (gates off)              GATED
segment            ms/launch    share            ms/launch    share      ratio
vas_publish          41.163     53.2%              0.207       4.5%      199×
pt_decode            21.166     27.4%              0.031       0.7%      683×
pt_sweep              6.525      8.4%              0.013       0.3%      502×
ringproj              3.137      4.1%              1.321      28.8%      2.4×
core     (host RM)    2.720      3.5%              0.422       9.2%      6.4×
pt_vascensus          2.105      2.7%              2.196      47.9%      1.0×  ⊘ UNGATED
UNMARKED              0.009      0.0%              0.008       0.2%       —

page-table + publn   71.012     91.8%              2.451      53.5%       29×
Σ trap               77.367                        4.583                 16.9×
```

- ★★ **`UNMARKED` is 0.008 ms of 4.583 (0.2 %)** on the gated arm. The breakdown still closes;
  the saving is not hiding in an unattributed remainder.
- ⊘ **The control reproduces w315.** w315's `full` measured `77.4`-equivalent
  (86.7 ms/launch at a different boot's scatter, `vas_publish` 55.7 %, `pt_decode` 25.7 %);
  this control reads 53.2 % / 27.4 % of a 77.4 ms trap. **The shares reproduce; the absolute
  trap is 11 % lower**, which is inside the boot-to-boot scatter w315 itself reported
  (86.7 vs 93.5 across its own two boots).
- ★ **The guest's `sync_med_ms` did not move** — 22.761 → 23.382. The gate is on the submit
  side and the completion half is untouched, which is what a correct localisation looks like.

### 2.1 ⊘ THE NUMBER I CANNOT ACCOUNT FOR, named rather than distributed

**`core` — the real host RM forward — got 6.4× FASTER** (2.720 → 0.422 ms/launch), and its
nested `core_rm_ipc` with it (2.588 → 0.360). **Nothing in this diff touches that path.**

The mechanism I can name is that the control's publication pass issues **eight
`join_one_fb_leaf` round trips per doorbell** which all end
`⚠ THE INSTALL REFUSED … already joined … ⊘ RELEASED and NOT bound` — i.e. it mints and
releases host objects through the same isolate the forward then has to talk to. Removing them
would leave that isolate idle when the forward arrives.

⚠ **That is a mechanism with one boot behind it, not a measured fact.** A second candidate —
ordinary boot-to-boot variance in the isolate's scheduling — is not excluded by n=1, and it is
exactly the shape w315 §5 got caught by (*"the instrument attributes, it does not isolate"*).
It is reported because a 6.4× move in a segment the diff does not touch is a claim about the
plane either way.

### 2.2 ⊘⊘⊘ THE 48 ms IS **NOT** AN O(VAS) SCAN — REFUTED FROM THE CONTROL ARM'S OWN LOG

A mid-rung redirect proposed that the publication cost is a **whole-table scan** and offered
the arithmetic `48.3 ms ÷ ~16 425 rows ≈ 2.9 µs/row` as *"consistent with an O(VAS) pass"*,
flagged — correctly — as a hypothesis to confirm early. **It is refuted.** Every publication
line carries both its row count and its candidate count, so the two can be separated without a
new instrument:

```
   rows in the biggest VAS   candidates   wall     n
        16 425                    0        0 ms    69
        15 913                    0        0 ms    66
        13 348                    0        0 ms    15
        18 277                    0        1 ms     8
        18 309                    8    38–43 ms   ~87
```

⇒ **Walking 16 425 rows — the exact population the arithmetic used — costs 0 ms.** The four
VASes are walked *in full* on every one of those doorbells (533 + 6 254 + 0 + ~18 300 ≈
**25 100 rows**, and `proc=0 pdb=0x2efa9c000` reports `candidates=6144 capped=2048` on both).
The ~40 ms appears **only, and exactly, when `candidates=8`**.

⇒ **The cost is EIGHT HOST ROUND TRIPS AT ~5 ms EACH, not a scan.** And what those eight do is
the actual defect: each mints a host object, `install_join` refuses it
*"that framebuffer range is **already joined**"*, and it is **`⊘ RELEASED and NOT bound`** —
**87 times, for the same eight leaves, in one boot.**

★★★ **`2.9 µs/row` is a coincidence of magnitudes, and it is the third instance of this
campaign's named failure shape** — an answer that *arrives pre-corroborated*, after w311's
`251/2 = 125.5 ms ≈ C` (which was the observer's own scan period) and w315's overlap-maximising
aligner (which saturated at its own arithmetic ceiling). ⚠ **The tell is the same every time:
the number agreed before anything was measured.**

⇒ **The defect is neither a scan nor a publication-timing error. It is a missing memo of a
PERMANENT refusal.** A dirty gate removes it; publishing at bind removes it; so would one
`if`. What none of those three is, is *"the table is too big to walk"*.

### 2.3 ★★★ THE FLOOR MOVED, AND IT IS NOW SOMEWHERE ELSE — twice over

1. **Inside the trap**, `pt_vascensus` is now the **largest single term at 47.9 %** and its
   absolute cost is *unchanged* (2.105 → 2.196 ms/launch). It is **not gated by this rung**:
   it is an unconditional census w304 made unconditional on purpose, so a boot can tell *"the
   census ran and found nothing"* from *"the sweep was disarmed"*. Gating it needs that
   property preserved, which is a different design from either gate here.
2. **Outside the trap**, the launch is now **85.4 % completion** (`sync_med_ms` 23.382 of
   27.960) where it was 21.9 %. ⇒ **the next binding constraint on the launch is the
   completion plane, not the submit plane**, and that is a different lane's subject.

⊘ Against the product metric: 60 tok/s needs ~64 µs per launch (w311). This rung takes
101.8 ms → 28.0 ms. **That is 3.6× of a required ~1600×** — real, and not the answer.

---

## 3. ★★★★★ THE FIRE/SKIP RATIO — the diagnostic outcome (B) turns on

Pre-registered (B) was *"it fires and the trap does not drop ⇒ the arming edge is wrong"*, and
it is **indistinguishable from a working gate by `trap_ms` alone**. Both gates count
themselves, on every `PT-DECODE` line:

```
control:  DIRTY-GATE publish[fired=1096 skipped=0    0.0% skipped]  witness[fired=275 skipped=0    0.0% skipped]
gated:    DIRTY-GATE publish[fired=39   skipped=1057 96.4% skipped]  witness[fired=12  skipped=263 95.6% skipped]
```

Per doorbell, gated: **258 of 275 doorbells skipped all four VASes**; 7 fired one, 6 fired
three, 3 fired all four, 1 fired two.

⊘ The control's `skipped=0` is the **known-positive for the counter itself**: a census that can
only ever print zeros is not evidence of anything, and the control proves this one moves.

---

## 4. ★★★★★ WHAT THE GATE ACTUALLY REMOVED — and it is not "some publications"

The two arms' whole-boot histograms of the publication pass's own outcome:

| `→ published=N refused=M` | control | gated |
|---|---|---|
| `published=1 refused=0` | 3 | **3** |
| `published=2 refused=0` | 2 | **2** |
| `published=7 refused=0` | 1 | **1** |
| `published=28 refused=0` | 1 | **1** |
| `published=24 refused=8` | 1 | **1** |
| **`published=0 refused=8`** | **87** | **0** |
| `published=0 refused=0` | 180 | 267 |

⇒ **Every publication the control made, the gated arm made — same count, same shape, same
number of passes.** The only class that disappeared is the 87 doorbells that published nothing
and re-refused the *same eight already-joined leaves* at ~40 ms each (≈ 3.5 s of a 2-minute
boot).

The page-table decode says the same thing:

| `→ bound=N …` | control | gated |
|---|---|---|
| `bound=1` | 3 | **3** |
| `bound=512` | 2 | **2** |
| `bound=2050` | 1 | **1** |
| `bound=0` | 264 | 263 |

⇒ **Every bind the control performed, the gated arm performed.** `drained=162 latched=52`
became `drained=109 latched=0 rounds=0` on 249 of 275 doorbells — the decode's own
`latched == 0 ⇒ procs.is_empty() ⇒ break` exit, which already existed and which this rung did
not have to invent. ⊘ The 109 pages still drained are the perpetually-**unattributable** ones
the witness carries by design; they are re-offered every doorbell and cost ~31 µs.

★ **This is the correctness argument in its strongest available form**, and it is measured
rather than reasoned: the gate is behaviour-preserving *on this workload* and only
repetition-removing. ⚠ It is not a proof for workloads not run — see §6.

---

## 5. CORRECTNESS — both workloads, n ≥ 3, and a same-binary control

⊘ **`bad=0` in §2 is UNGUARDED**, exactly as w315's was: those boots ran
`KAYFABE_BENCH_ONLY=measure` with no `BENCH_NOLAUNCH` negative control. Correctness is graded
here instead, in separate boots, on **both planes** — `scripts/bench/relaxation_inert_gate.sh`
exists because a cup3-only grade let a regression through, and cup3 is libcuda+GR and cannot
see a CE-only break.

⚠ **n = 1 is not a grade.** w314 measured a **~20 % false-negative rate** on a single-boot
`^CUP3_VAL=43` grade on these boxes, with the reds identical field for field
(`Xid 31, chan 0x02000015, CE3, HUBCLIENT_CE1, @ 0x2_0440f000, FAULT_PDE, VIRT_WRITE`).

Four runs, eight boots, **one binary at `c6301a57`** differing only in two environment strings:

| run | gate | plane | known-positive | RC | `Xid` | `host_rows` | gate ratio |
|---|---|---|---|---|---|---|---|
| `g1` | **on** | cup3 (GR) | **`CUP3_VAL=43`** | 0 | **0** | **18295** | 95.7 % / 94.8 % |
| `g1` | **on** | R33 arm 1 (raw CE) | **`★ R33 arm 1 COPY`** | 1 | **0** | **3** | 0 % / 0 % |
| `g2` | **on** | cup3 | **`CUP3_VAL=43`** | 0 | **0** | **18295** | 95.7 % / 94.8 % |
| `g2` | **on** | R33 arm 1 | **`★ R33 arm 1 COPY`** | 1 | **0** | **3** | 0 % / 0 % |
| `g3` | **on** | cup3 | **`CUP3_VAL=43`** | 0 | **0** | **18295** | 95.7 % / 94.8 % |
| `g3` | **on** | R33 arm 1 | **`★ R33 arm 1 COPY`** | 1 | **0** | **3** | 0 % / 0 % |
| `c1` | **off** | cup3 | `CUP3_VAL=43` | 0 | 0 | 18295 | 0 % / 0 % |
| `c1` | **off** | R33 arm 1 | `★ R33 arm 1 COPY` | 1 | 0 | 3 | 0 % / 0 % |

**`W313 INERT-GATE VERDICT = INERT-ON-BOTH-PLANES` on all four runs.**

- **`host_rows` is byte-identical to the control and to w297/w314's green**: 18 295 on cup3,
  3 on R33. The gate did not cost the host VAS a single row.
- **Zero Xid on all eight boots.** ⊘ The known intermittent
  (`Xid 31, chan 0x02000015, CE3, HUBCLIENT_CE1, @ 0x2_0440f000, FAULT_PDE, VIRT_WRITE`) did
  **not** appear on any arm. ⚠ At n=3 gated / n=1 control that is **not** evidence its ~20 %
  rate changed in either direction — 3 clean gated boots is what a 20 % rate produces 51 % of
  the time. It is reported as *not observed*, which is what it is.

### 5.1 ⊘⊘ THE R33 PLANE IS A CONTROL, **NOT** EVIDENCE THE GATE IS SAFE WHEN IT FIRES THERE

Read the R33 rows again: **`publish[fired=3 skipped=0] witness[fired=2 skipped=0]`** — on every
run, gated and control alike. The raw CE client is short-lived and rings **three** doorbells, so
there is never a second doorbell over an unchanged epoch and **the gate never skips anything on
that plane.**

⇒ What those four boots establish is that **arming the gate does not break the raw-CE plane** —
the binary, the extra fields and the counter are all inert there. They do **not** establish that
a *skip* is safe on it, because no skip occurred. ★ Said out loud because the identical
`INERT-ON-BOTH-PLANES` banner reads as though both planes were exercised equally, and one of
them was exercised only as a null.

⊘ This is `a census ZERO needs a KNOWN-POSITIVE` in its exact form: the gate's known-positive
on the GR plane is 95.7 % skipped beside an unchanged `host_rows`; on the CE plane there is no
known-positive at all, and the honest word for that is **UNMEASURED**.

---

## 6. ⚠ WHAT THIS RUNG DOES NOT ESTABLISH

- **A skip is only as sound as the epoch is complete.** `Vas::publish_epoch` covers the table
  and the pins; `joined` covers the one host term the refusals in this trace depend on. A
  publication outcome that depends on host state **neither** term sees would be cached. ⊘ The
  refusals measured here are permanent by construction (*"already joined"*), but a **transient**
  host failure would be stamped and not retried until something moved the epoch. That is a real
  residual, it is not exercised by any workload here, and it is the first thing to suspect if a
  publication is ever observed missing on a gated boot.
- **`guest_ram_pins.len()` is an epoch only because pins are insert-only.** If a single-pin
  removal is ever added, that term must become a generation. The type is a tuple so the next
  reader finds the seam.
- **One size (N=512), one guest, one physical GPU.** The timing is **one boot per arm**; what
  makes it readable at n=1 is that the arms are the same binary at the same revision differing
  in two environment strings, and that §4's histograms are exact rather than statistical.
- **Display, NVENC and multi-process are UNMEASURED, not inert.**
- ⊘ **`pt_vascensus` was not gated and `ringproj` was not gated.** §2.2.

---

## 7. What the brief got wrong, and what I got wrong

- ⊘ **The brief's premise that `vas_publish` might be genuinely per-launch work (outcome (D))
  is REFUTED**, and by the strongest available evidence: 87 of 88 non-trivial publication
  passes in the control published **nothing** and refused the **same eight leaves**.
- ⊘ **The brief said the C's gate "is not where w315's numbers say the dominant term is", and
  that was right and load-bearing.** The C's gate sits on the *sweep*; transcribing it would
  have gated `pt_sweep` (8.4 %) and left `vas_publish` (53.2 %) untouched.
- ⊘ **I nearly built the `pt_decode` gate on the wrong signal.** The obvious edge is the
  *page set* the executor witness re-queues — and that set is **stable by construction**,
  because origin is first-writer, so a page joins the population once and never leaves. A gate
  on it would have skipped correctly-and-for-the-wrong-reason until the first in-place PTE
  rewrite, then silently stopped witnessing it. The counter had to be added.
- ⊘ **The gate is on the PRODUCER, not the consumer.** Gating `decode_cpu_pt_writes` would have
  been gating the pass whose cost was measured; the thing keeping it non-empty was
  `witness_executor_fb_pages` re-queueing 53 pages unconditionally one line above it.
- ⚠ **I cannot account for `core` getting 6.4× faster** (§2.1) and did not distribute it.

---

## 7.1 ★★★★★ THE MID-RUNG REDIRECT — what it got right, and the one thing it got wrong

A redirect arrived after the measurement, arguing (owner's framing) that **a GPFIFO doorbell is
asynchronous**, that the trap should be *translate the token, ring the host doorbell, re-enter
the VM*, and that **publication belongs at bind** — the dirty gate being *"the C's mitigation"*
that skips the work instead of removing it.

★★★ **The architectural framing is right, and this rung is evidence FOR it, not against it.**
After the gate the trap is **4.583 ms**, of which the **actual host forward is 0.422 ms** and
the entire remainder is two *diagnostics* — `pt_vascensus` (2.196) and `ringproj` (1.321) —
neither of which is publication. ⇒ **the doorbell is already within ~4 ms of translate-and-ring,
and what is left in it is instrumentation, not the address plane.**

⊘ **Three claims in the redirect are refuted by this rung's own measurements:**

1. **"The scan runs every doorbell over the whole table"** — §2.2. Walking 16 425 rows costs
   **0 ms**; the cost is eight host round trips. `2.9 µs/row` was a coincidence of magnitudes.
2. **"Publish at bind removes the 91.5 %"** — it cannot remove `pt_decode`, and the redirect's
   own text says why: *"the guest writes PTEs via BAR2 CPU stores … blind to RM"*. That segment
   is **27.4 % of the control trap** and has **no bind hook by construction**. This boot's own
   first-writer census puts the ratio the redirect asked for at
   **`PRAMIN 21 / BAR1 9 / BAR2 88 / EXEC 3546 / UNATTRIBUTED 0`** — **3 634 of 3 664 framebuffer
   pages (99.2 %) are created by a CPU transport with no RPC bind hook.** ⇒ publish-at-bind's
   ceiling here is `vas_publish` alone, **53.2 %** of a trap the gate reduced by **94 %**.
3. **"The first doorbell after any PT write still pays the full 86.7 ms"** — refuted at the
   tail. The gated boot's **worst** launch is `max_ms=32.693` against the control's
   `max_ms=124.094`; there is no launch anywhere in the gated boot that pays the ungated cost.

⇒ **Recommendation, offered rather than assumed: land the gate, and take publish-at-bind as the
next rung with its ceiling stated in advance** (≤ 53 % of the *control* trap; ≈ 0.2 ms of the
*gated* one). ⚠ And note what the real defect turned out to be, because it is smaller and more
actionable than either framing: **we re-issue eight host join round trips per doorbell for
framebuffer ranges that are already joined, and release them again — 87 times in one boot.**
That is a missing memo of a permanent refusal. It is not a scan, and it is not a
publication-timing error.

## 8. Reproducing

```
scripts/bench/w318_gate.sh off      # the control — same binary, gates disarmed
scripts/bench/w318_gate.sh on       # the measurement
scripts/bench/w318_gate.sh pub      # only the publication gate
scripts/bench/w318_gate.sh wit      # only the executor-witness gate

scripts/bench/relaxation_inert_gate.sh run <tag> \
    KAYFABE_DIRTY_GATE_PUBLISH=on KAYFABE_DIRTY_GATE_WITNESS=on   # BOTH workloads
```

Knobs: `KAYFABE_DIRTY_GATE_PUBLISH`, `KAYFABE_DIRTY_GATE_WITNESS` (`off` default / `on`).
