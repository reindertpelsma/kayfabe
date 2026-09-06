# ★★★★★ WHAT ONE DOORBELL COSTS THE SUBMITTING THREAD — a five-second gate for `w383-doorbell-async`

**STATUS — 2026-09-06 — LIVE.** Adds one rung to the raw client (`--doorbell-latency`) and its
differential harness. It does not supersede `w381_the_guest_servable_probe.md`; it is built on
that lane's `LAUNCH_DMA` primitive and inherits its scope caveats verbatim. Mechanism sections
§1–§3 are read off this tree's own source and are checkable without a GPU. §4 carries the
measurements; every number in it names the arm it came from.

---

## §0 THE ONE-LINE PROBLEM

`[measured, LLM boot, w383 lane]` publication runs **60–71 ms per doorbell, inline on the vCPU
thread**:

```
TRAPWITNESS off_trap_claims=0 inline_exceptions=61865 worst_trap=1750538us
            (target: inline_exceptions=0)
```

Thousands of doorbells at that price is why the LLM rung is killed by a harness timeout with
doorbells still being served — and **nothing in this tree measured it in under twenty minutes.**
`LLM_TOKENS`, the async lane's only feedback, needs a full boot plus a model load and can come
back `UNMEASURED` for reasons that have nothing to do with the change under test. A lane whose
only instrument costs twenty minutes and can answer *"no result"* is a lane iterating blind.

## §1 WHY A NEW RUNG AND NOT ONE OF THE SEVEN

Owner, 2026-09-06: *"the most valuable raw client is one that passes on host and fails in guest,
then its just iterate unless there is a blocker."*

All seven w379/w381 rungs pass natively; six of seven pass in the guest (`w381` §4.1). By that
criterion they discriminate almost nothing. `missing_page` is the one with the valuable shape,
and it is about the **error notifier** — not about anything on the critical path the async lane
is moving. So the shape had to be built, not found.

## §2 THE MEASURED REGION, STATED EXACTLY

For the graded arm the region is **one `HostRmBackend::submit_copy_at` call and nothing else**.

| inside the region | outside it |
|---|---|
| handle narrow; one `HashMap` lookup for the channel's parts | the store that seeds the source word |
| the pushbuffer encode (**allocates one small `Vec`**) | the every-16 drain |
| ~12 stores into the ring object; 2 GPFIFO words; the `GP_PUT` store | every `println!` |
| two release fences and **the doorbell store** | all statistics |

★ **The `Vec` and the lookup are inside the region on BOTH arms, so they cancel in the ratio the
gate is taken on.** That is the reason the gate is a *multiple of a measured native median* and
not an absolute number: any cost that is arm-independent divides out.

### ★★★ THE INSTRUMENT THAT WOULD HAVE BEEN THE MEASUREMENT

`RmConnection::doorbell` prints **two `eprintln!` lines per store for its first 512 stores**
(`DOORBELL_WITNESS_MAX`), and those lines are *inside* the region. On bare metal a doorbell is
microseconds, so a formatted write to stderr is not a rounding error on it — it is a large
fraction of the number. ⇒ every gate multiplied out of that floor would have been **uniformly,
quietly too loose**.

`kayfabe_isolate_host::rm::mute_doorbell_witness()` **exhausts** the witness (advances its
counter past the cap) rather than disabling it: the periodic tally still prints, and **every
refusal still prints in full**, by `doorbell`'s own argument that suppressing the rare event to
make room for the common one inverts the witness's purpose. The rung **prints that it muted**,
because a quiet log a reader takes for a complete one is a failure class this tree has paid for.

## §3 THE GATE — a multiple of a measurement, and the multiple is argued

⊘ A hard-coded millisecond figure would be a standard nobody agreed to, on a box nobody
characterised, and it would expire as the box changed with no one noticing — the shape
`a_capture_derived_table_expires_as_a_vendor_regression` records. So:

```
gate = native_p50 × DBL_GATE_MULTIPLE          DBL_GATE_MULTIPLE = 1000
```

`native_p50` is measured **minutes earlier, on the same box, by the same binary** (md5 printed on
both arms) and is fed to the guest arm by the harness as `--doorbell-latency-native-us`. Nothing
is baked into the binary.

**1000× is the only decade that separates the two things the rung must tell apart**, and both
sides of the band are measured rather than assumed:

| | what it is | order |
|---|---|---|
| the **floor a correct emulation cannot go below** | a doorbell in a Mode-2 guest is a trapped MMIO store: one VM exit, one VMM dispatch, one return — a cost an async design still pays | **10–100×** |
| the **defect** | 60–71 ms inline on the vCPU thread | **~10⁴×** |

⇒ 1000× sits a decade **above** the most expensive honest emulation cost and a decade **below**
the defect. A gate outside that band either reddens a working async design or greens the thing it
exists to catch. ⚠ It is a **discrimination threshold, not a performance target**: passing it is
not a claim that anything is fast.

### ★★ AND THE NATIVE ARM HAS ITS OWN BAR, because a calibration cannot fail a gate derived from itself

`DBL_NATIVE_SANITY_CEILING_US = 1000`. Nothing in `release_fence()` plus a store into a mapped
write-combining window legitimately medians above a millisecond; if it does, the box is
contended or the path is not what we think, and **the number must not be used as anyone's
floor**. A native run above it is a red, printed as one. The native verdict says in words that
it is *not* "native met a gate".

### ★★★★★ THE SAMPLE FLOOR HAS AN ESCAPE HATCH, and without it the rung is backwards on exactly the case it exists for

The wall budget is what makes this a five-second gate, and its consequence is that **the worse
the arm is, the fewer samples it produces**. At `worst_trap=1750538us` a 1.5 s repetition yields
**one** sample. A flat rule of *"under 50 samples ⇒ UNMEASURED"* would report the most
catastrophic possible result as *"we could not tell"* — the one arm the rung could never grade
would be the broken one.

The hatch is narrow and **sound rather than lenient**: it does not lower the bar, it uses a
statistic that needs no sample size. If the **fastest** submission observed is already over the
gate, no median over any number of further samples can be under it — every sample is at least the
minimum, by definition. The rung prints `DBL_ROLE=GRADED_ON_MIN` and says both facts.

## §3.1 THE CONTROLS — the rung is these, not the timing loop

- **Two positive controls, one before the loop and one after.** Each is a four-byte `LAUNCH_DMA`
  into the rung's own mapped object, read back through an **independent** CPU mapping. ★ The
  closing one is what makes the loop attributable: the opening control alone would be passed by a
  loop that killed its own channel on submission 3 and then timed 500 cheap refusals. Either
  control failing ⇒ `RUNG_doorbell_latency=NOTRUN`, ⊘ **not** a red.
- **Refusals are excluded from the samples and counted separately** (`refused=` on every
  distribution line). A refusal is not a fast submission.
- **`truncated=`** is printed on every distribution. A truncated run is still a valid
  distribution; a run that does not *say* it was truncated is not.
- **Five order statistics and a total, never a mean.** The failure being gated is a tail that
  dominates an aggregate, and a mean is exactly the summary that hides it.
- ⊘ **`GP_GET` is not read anywhere in this rung.** It has no writer in the workspace and an
  emulated device has no PBDMA, so `gp_get == 0` is the only value it can hold on every
  configuration. The ring itself is measured instead.

### §3.2 TWO UNGRADED ARMS, and they are what make a green readable

- **`arm=bare`** — `ring_doorbell_only`: one fence and one store, no ring stores, no `GP_PUT`
  advance, no new entry. ⊘ **Ungraded**: a device is entitled to notice `GP_PUT` has not moved
  and do nothing, and *"a no-op is fast"* is a finding about the no-op. ★ What it is for is
  **attribution** — if a real submission is expensive and this is cheap, the cost is in the ring
  stores or the planning; if this is expensive too, the cost is in the trap.
- **`arm=freshmap`** — a **fresh** 64 KiB object at a **fresh** VA before every ring, so there is
  always something unpublished behind it. ★★★ This exists so that a **green on the graded arm is
  interpretable**: if publication is incremental, a loop that rings the same channel at the same
  address N times may pay once and be cheap thereafter, and would pass while a real workload —
  which maps as it goes — does not.

## §3.3 WHAT THIS DOES NOT MEASURE — scope, stated up front

- ⊘ **The ladder builds its OWN `FERMI_VASPACE_A` and its own CE channel inside the guest.** The
  claim it supports is *"a doorbell rung by an unprivileged guest process costs N"* — the
  quantity the async lane is moving. It is **not** *"the LLM's per-doorbell cost is N"*, and a
  reader who takes it for that has this campaign's recorded failure shape: a probe in the wrong
  address space.
- ⊘ **The guest arm runs `KAYFABE_CE_EXECUTOR=local`**, so it measures the CPU copy-engine
  emulator's doorbell path. `=host` is a different measurement and needs its own row.
- ⊘ **Within-process repetitions cannot see a per-boot lottery.** `submit_ms` has been measured
  at **9.1× across three consecutive boots of one build**. The rung repeats three times inside
  one process (within-run stability) and the harness runs the whole binary three times on each
  arm (between-process). Neither substitutes for the other, and **neither sees a per-boot
  effect**, because all three guest processes share a boot.

## §4 THE MEASUREMENTS

### §4.1 NATIVE — the calibration, RTX 3060 (GA106), host driver 580.159.04 open

Bench `kb`, source `261aa977`, binary md5 `7216822ca5744245c3109c9a19ce05aa`, **three separate
processes**, verbatim:

```
--- run 1 ---
DBL_DIST arm=submit   rep=POOLED n=1536 min_us=8.926 p50_us=9.016 p90_us=9.067 max_us=27.952 total_ms=39.6 truncated=false refused=0
DBL_DIST arm=bare     rep=0 ⊘UNGRADED n=512 min_us=0.039 p50_us=0.040 p90_us=0.040 max_us=3.636 total_ms=0.2
DBL_DIST arm=freshmap rep=0 ⊘UNGRADED n=64  min_us=9.107 p50_us=9.317 p90_us=9.467 max_us=25.456 total_ms=23.2
--- run 2 ---
DBL_DIST arm=submit   rep=POOLED n=1536 min_us=8.895 p50_us=8.986 p90_us=9.017 max_us=22.172 total_ms=40.1 truncated=false refused=0
DBL_DIST arm=bare     rep=0 ⊘UNGRADED n=512 min_us=0.039 p50_us=0.040 p90_us=0.040 max_us=3.296
DBL_DIST arm=freshmap rep=0 ⊘UNGRADED n=64  min_us=9.107 p50_us=9.286 p90_us=9.387 max_us=9.607
--- run 3 ---
DBL_DIST arm=submit   rep=POOLED n=1536 min_us=9.236 p50_us=9.267 p90_us=9.278 max_us=34.463 total_ms=40.7 truncated=false refused=0
DBL_DIST arm=bare     rep=0 ⊘UNGRADED n=512 min_us=0.039 p50_us=0.040 p90_us=0.040 max_us=11.370
DBL_DIST arm=freshmap rep=0 ⊘UNGRADED n=64  min_us=9.507 p50_us=9.598 p90_us=9.697 max_us=25.467
```

`DBL_CALIBRATION_NATIVE_P50_US = 9.02 / 8.99 / 9.27`. ⇒ **floor `9.27 us`** (the harness takes
the **worst** of the three, never the best: a floor picked from the fastest run makes the gate
tighter than the host can reliably deliver, and the first thing that reddens is host jitter
wearing the guest's name), so **gate = 9.27 ms**.

★ **The native distribution is extraordinarily tight** — `min 8.93, p50 9.02, p90 9.07` over 1536
samples, and 1.03x spread across three processes. Whatever else this rung is, its instrument is
not noisy, and that is what makes a three-decade gap meaningful.

### §4.1.1 ★★★ THE NATIVE ARMS DISAGREE BY 225x, AND THAT IS THE MOST USEFUL NUMBER HERE

| arm | native p50 | what it contains |
|---|---|---|
| `bare` | **0.040 us** | one `release_fence` + one 32-bit store into the mapped usermode window |
| `submit` | **9.02 us** | the above **plus** ~12 ring stores, 2 GPFIFO words, the `GP_PUT` store |
| `freshmap` | **9.32 us** | `submit`, with a fresh 64 KiB object mapped at a fresh VA first |

⇒ **On bare metal the doorbell store is 40 ns and is 0.4 % of a submission.** The other 99.6 % is
stores into device memory. ⚠ **This is load-bearing for reading the guest arm**, and it is not
what one would guess: the graded number is a *submission* number, not a *doorbell* number, and
the two arms are what separate them. If the guest's `bare` is large, the cost is **the trap**; if
`bare` is small and `submit` is large, it is in the ring stores or in what the VMM does behind
them.

⇒ And **`freshmap` ≈ `submit` natively (9.32 vs 9.02)**: mapping a fresh page before every ring
costs the submission 0.3 us on hardware. That is the control that makes a guest-side gap between
those two arms attributable to **publication** rather than to the arm's own extra work.

### §4.2 GUEST

*(the Mode-2 arm; see `scripts/bench/w384_doorbell_latency.sh`, which feeds the native floor to
the guest arm as `--doorbell-latency-native-us`)*
