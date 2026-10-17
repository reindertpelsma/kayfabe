# V3 display pacing — the guest's flips are never parked; copies are bounded; who ticks

**STATUS: LIVE, 2026-10-17.** Phase 1 (§4–§6) is BUILT on `claude/display-pacing-20261017` (cut from `claude/tear-gate-exp-20261017`
`ec42327f0` = `integration/prod-candidate-3-20261017` + the `KF3_TEAR_GATE` measurement flag), GPU-free tested, **NOT RUN ON A BOX**.
Phases 2 and 3 (§8) are DESIGN-ONLY. The owner's decisions are `OWNER_RULINGS.md` §AK (they amend §M D1). The mechanism this
replaces is `V3_DISPLAY.md` §8.16 (corrected in place).

Labels: **MEASURED** = a test run of this branch (the model, §2) or a number the owner quoted from his live run; **CODE READ** = read in
the tree; **INFERRED** = reasoning, never written as evidence. Every hypothesis states its falsifier before the result.

## 0. Short answer

1. **The D1 gate does hold a fullscreen present loop at one flip per tick (MEASURED, model): 60 flips/s, whatever the render time up to
   ~14 ms.** It does **not**, by itself, produce 30 (§2). The owner's counters (`kms=59.9 tear=29.9 copies=59.9 tick=59.9`, 30 fps) are
   reproduced exactly by a loop that makes **two flips per frame, one immediate and one not** (INFERRED structure; falsifier in §2.4);
   they are not reproduced by one immediate flip per frame, and `held=+60/s` is reproduced by neither (§2.5).
2. Decision (a) removes the gate: an immediate flip latches and completes at once. On the model, one immediate flip per frame at
   R = 4 ms then runs at **210 flips/s with two buffers, 250 with three, while the display ticks at 60 and the copies stay at 60**.
   If the structure of finding 3 is real, the ceiling becomes the non-tearing flip of each frame, which waits for its tick by decision (a):
   about **60 frames/s, not 200** — the owner's hardware run decides (§9).
3. The rate is bounded where the guest does not wait: **the newest latched frame is copied at most once per tick** (`pace::Mailbox`).
   The correctness condition of completing a flip at once is the **buffer-reuse rule** (§6.1): a flip's completion waits for the copy in
   flight at its latch and for nothing else.
4. Who ticks is `tick::TickSource` (§5): the free-running clock (headless, and the fallback) or the broker's events; **50 ms of broker
   silence** hands the head back to the clock; the guest's vblank is never gated by the broker. In phase 1 nothing feeds a broker event, so
   the clock ticks exactly as before.

## 1. What the code did (CODE READ, `ec42327f0`)

- `group_ready` parked a group whose window was tearing when `presented[head]` was set; `Engine::vblank` cleared `presented[head]` and then
  latched the groups parked for that head — and a latch sets `presented[head]` again. So after **any** tick that latched something, every
  flip arriving in that tick period parked for the next tick: one latch per tick, and a lock-step guest (it programs flip N after flip N-1
  finished: the Windows driver polls N-1's notifier for FINISHED, `engine.rs` `complete`) is held to one flip per tick whenever its
  render, copy and wake fit inside a period. The "first after a tick latches at once" rule only applies after a tick that latched nothing.
- Every window latch of the shown head set `barrier = started + 1` and `want = true`: the flip's release, notifier and GET waited for
  **its own** copy, which started at once. So the completion's latency was the copy's.
- Copies never waited for a broker slot: the frame ring's cap argument guarantees a free slot, and `none` is a counted fault, not a wait
  (`ConsoleShare::free_slot`, `ScanState::start`). The broker's RELEASE cadence is therefore not a rate limiter of the guest's flips.

## 2. Task 1 — reproducing the fullscreen rate

### 2.1 The hypothesis and its falsifier (stated before the run)

**H1:** the D1 gate plus the completion rule above turn an exclusive-fullscreen loop (render, flip, wait for the completion, render) at a
60 Hz tick into ~30 flips/s. **Falsifier:** a model of that loop against the real `Engine` with the gate on that gives about 60, not 30,
for the render times a 200 fps windowed Heaven implies (R ≈ 5 ms, so R ≤ 14 ms is generous).

### 2.2 The model

`crates/kf-disp/src/engine/pacing_model.rs`. The guest renders a frame into a free buffer (R ms), submits its flip when the previous flip
has completed (one outstanding, as the Windows driver does), and learns of a completion `wake` after the worker publishes it; buffer
reuse follows the swap chain (2 or 3 buffers: flip *j*'s completion releases the buffer of flip *j-1*). The flips go through the **real**
`Engine` (a window ring with the real methods, the real interlock, the core variant), the ticks through the real `Pacer`, and the worker's
copy and completion rule is a small model (`Worker::Legacy` = the rule of §1: the flip's items wait for copy `started + 1`). Copy 2 ms
(the Vast boxes' `scanout_avg_us` was ~1.8 ms), wake 0.5 ms, tick 16.666 ms, 4 s per point after 1 s of warm-up. Variants: 2 vs 3
buffers; immediate, vsync, alternating immediate/vsync, a core-holding flip every 4th; a pipelined driver whose flip is submitted at the
render's start with a semaphore acquire; the gate ON and OFF.

### 2.3 Results (MEASURED, model, gate code of `0780de42e`; flips completed per second against R in ms)

| loop | gate | R 1 | 2 | 4 | 8 | 12 | 14 | 16 | 18 | 20 | 25 | 33 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2 buffers, immediate | ON | 60.0 | 60.0 | **60.0** | 60.0 | 60.0 | 60.0 | 54.0 | 49.0 | 44.5 | 36.2 | 28.0 |
| 2 buffers, immediate | OFF | 285.8 | 222.2 | 153.8 | 95.2 | 68.8 | 60.8 | 54.0 | 49.0 | 44.5 | 36.2 | 28.0 |
| 3 buffers, immediate | ON | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 55.5 | 50.0 | 40.0 | 30.2 |
| 3 buffers, immediate | OFF | 400.0 | 400.0 | 250.0 | 125.0 | 83.2 | 71.2 | 62.5 | 55.5 | 50.0 | 40.0 | 30.2 |
| 2 buffers, vsync | either | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 30.0 | 30.0 | 30.0 | 30.0 | 20.0 |
| 2 buffers, alternating imm/vsync (flips/s; frames = half) | ON | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 40.0 | 40.0 | 40.0 | 30.0 | 24.0 |
| 2 buffers, alternating imm/vsync | OFF | 120.0 | 120.0 | 120.0 | 60.0 | 60.0 | 60.0 | 40.0 | 40.0 | 40.0 | 30.0 | 24.0 |
| 2 buffers, core-holding flip every 4th | ON | 80.0 | 80.0 | 80.0 | 80.0 | 68.5 | 60.8 | 54.0 | 49.0 | 44.5 | 36.2 | 28.0 |
| 2 buffers, immediate, pipelined with an acquire | ON | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 | 30.0 | 30.0 | 30.0 | 30.0 | 20.0 |
| 2 buffers, immediate, pipelined with an acquire | OFF | 221.0 | 221.0 | 153.2 | 95.0 | 69.0 | 60.5 | 54.0 | 48.8 | 44.5 | 35.0 | 27.5 |

Copy-time sweep, 2 buffers, immediate, R = 4, gate ON: copy 1…12 ms → 60.0 flips/s and 60.0 copies/s; 14 ms → 54.0; 18 → 44.5;
25 → 33.8; 33 → 26.8 — **copies = flips throughout**. The counters at R = 4: one immediate flip per frame `kms=60 tear=60 copies=60 held=60
flips=60`; alternating `kms=60 tear=30 copies=60 held=30 flips=60 (30 frames)`.

### 2.4 What this says

1. **H1 is refuted for the plausible render times.** One immediate flip per frame, gate on, runs at 60, not 30, for every R up to 14 ms
   (3 buffers: 16 ms). The first-after-tick rule does not make 30 either (§1). The gate holds the loop at one flip per tick — the
   *mechanism* of the blockage is real, its *size* is 2× smaller than observed.
2. **The slow-copy explanation is refuted by the counters.** A copy long enough to cut the rate cuts the copies with it (copies = flips);
   the owner's line has `copies=59.9` at 30 fps.
3. **Two flips per frame reproduces the counters.** Alternating an immediate and a vsync flip per frame gives, on the gate-on code,
   exactly `kms=60, tear=30, copies=60, tick=60` and 30 frames/s, **independently of R up to 14 ms**: the immediate flip waits for the
   tick (gated), the vsync flip waits for its own, so a frame costs two ticks. This is **INFERRED** structure (a guest that follows an
   immediate flip with a non-tearing one — for example the driver re-flipping to the same surface at the vblank to learn when the frame is
   shown, or a second window; I found nothing in the tree that says which). **Falsifier:** with `KF3_DISPLAY_TRACE=1` on the Windows
   fullscreen run, the engine's `chn N UPDATE` / `group … ready, latch …` lines show **one** window UPDATE per displayed frame — then
   finding 3 is false and the cause is elsewhere (§2.6).
4. A **pipelined driver** (flip queued at the render's start with an acquire) makes the gate cost a whole tick whenever the acquire is not
   yet satisfied at the tick (R ≥ 16 ms: 30 flips/s), because a parked flip's acquire is re-read only at its tick. Gate off, an immediate
   flip's acquire is polled every 2 ms. This produces 30 and `kms = tear = 30` — not the owner's `kms = 60` — and needs R ≥ 16 ms, which a
   200 fps windowed run does not suggest.
5. **`held=+60/s` with `tear=29.9` is reproduced by no variant.** The model gives `held = tear` for an all-immediate loop (60/60) and
   `held = tear = 30` for the alternating one. Counting paths in `group_ready` (one increment per gated group) and `latch_group` (one
   `tearing` increment per latch) show no double count, so the discrepancy is unexplained; a longer trace would decide. It does not change the
   decision: the gate is removed either way.

### 2.5 What else could hold fullscreen at 30 (listed, not modelled)

- The **1016 ms `STALL` line** (`Window 0 (chn 1) update 0x0 parked 1016 ms: ready, waiting for a vblank …`): the line was quoted
  truncated. "ready, waiting for a vblank **or its acquire**" with `head Some(0)` and an `acquire` that did not hold means the guest's GPU had
  not released the flip's semaphore for a second — a stall at the windowed→fullscreen switch, in the guest's rendering, not in the pacing.
  The full line (its `head`, `acquire`, `current` fields) tells.
- `core_imm=2`: two core-holding latches in the whole run (the mode switch at entry and exit) — not a per-frame cost.
- The GPU time-slicing between the guest's contexts and kayfabe's compose context: it would lengthen the copy, which the copy sweep (item 2)
  excludes at `copies=60`.

## 3. The owner's decisions (2026-10-17; `OWNER_RULINGS.md` §AK)

(a) Flips are never parked to bound the rate. Immediate flips latch and complete at once; vsync flips complete at the next vblank; kayfabe
copies only the newest latched scanout frame, at most once per tick (mailbox), into a free ring slot. (b) Flip syncing is lock step between
guest and kayfabe; the broker later sends a frame request only after it received the previous frame (credit based) — not built, must not be
precluded. (c) The free-running clock stays: the tick source without a broker and the fallback when the broker is silent (50 ms, a named
constant); a broker event makes the broker the source again at once; the guest's vblank is never hard-gated by the broker; handoffs are
seamless (one source, monotonic counter and timestamps, no double tick closer than a fraction of a period, phase re-aligned). (d) Later,
design only: the broker as tick source over the broker protocol, and an HDMI-over-TMDS virtual connector past the 75 Hz / 165 MHz limits.

## 4. Three things that were one

| | what the guest sees | who decides | phase 1 |
|---|---|---|---|
| **the vblank** | the frame counter, the vblank interrupt, vsync flips completing, NVKMS's callbacks | `TickSource` (§5): the free clock, later the broker | the free clock, as before |
| **the flips** | `UPDATE` → latch → completion (release semaphore, notifier FINISHED/BEGUN, GET) | the engine: immediate at once, vsync at the head's tick; **never parked for a rate** | immediate flips unparked |
| **frame delivery** | nothing — the host's view | the mailbox: the newest latched frame, one copy per tick, into a free ring slot; lock-step credits later (§8) | one copy per tick |

The guest's view of time (vblank) is a clock; what the guest *does* (flips) is never delayed by the host's view (delivery). Delivery may
skip frames (`skipped`), the guest never loses one.

## 5. The tick source (`kf_disp::tick::TickSource`, pure)

Constants: `BROKER_SILENCE_NS = 50 ms`; the double-tick guard `GUARD_NUM/GUARD_DEN = 1/2` of the tick period (a broker event closer than
half a period after the previous tick, from either source, is swallowed). The cap's slowest tick is 24 Hz = 41.7 ms: a live broker
speaks inside the silence with 8 ms to spare.

**States:** `Free` (kayfabe's clock ticks; the initial state, the only one without a broker), `Broker` (the broker's events tick; the
clock is suspended). **Events:** `due(now)` (time reached), `broker_tick(now)` (a broker event; its source is phase 2),
`retime(now, period)` (the mode changed).

| state | event | tick? | handoff | next state / deadline |
|---|---|---|---|---|
| Free | `due`, before the deadline | no | — | Free, deadline unchanged |
| Free | `due`, at/after it | **yes** (Free; `late` = lateness) | — | Free; deadline + P, or now + P when more than P late (no catch-up burst — exactly the old clock) |
| Free | `broker_tick`, no tick before, or ≥ P/2 after the last | **yes** (Broker) | → Broker | Broker; deadline = now + 50 ms |
| Free | `broker_tick`, < P/2 after the last tick | no (swallowed) | → Broker | Broker; the free deadline is dropped |
| Broker | `due`, before `last event + 50 ms` | no | — | Broker |
| Broker | `due`, at/after it | no | → Free (log line) | Free; deadline = the first point of the grid `last event + k·P` after now (≤ P later) |
| Broker | `broker_tick`, ≥ P/2 after the last tick | **yes** (Broker) | — | Broker; deadline = now + 50 ms |
| Broker | `broker_tick`, < P/2 after the last tick | no (swallowed; the broker is alive) | — | Broker; deadline = now + 50 ms |
| any | `retime`, same period | — | — | unchanged (phase and source kept) |
| any | `retime`, new period | — | — (source reset) | Free; deadline = now + P; the broker's next event takes over again |

**What the guest sees.** One vblank counter per head, +1 per emitted tick, never repeated or skipped (the `Pacer` numbers ticks across
re-arms and idle spells); timestamps non-decreasing; at a handoff Free→Broker the gap to the last free tick is at least P/2 (a closer event
is swallowed: the guest sees the *next* broker tick), and at Broker→Free the gap is the 50 ms silence plus up to one period — well inside
Windows' ~2 s flip-completion watchdog, so a hung, hidden, occluded or crashed broker costs the guest 50–67 ms of vblank, never a reset.
The phase of the clock after a takeover is the broker's, so the guest's tick grid does not jump. **The broker's own decimation is a phase-2
requirement:** a host refresh faster than the guest mode (144 Hz against a 60 Hz mode) must be decimated by the sender, because the guard
is only a safety net against double ticks (accepting events P/2 apart would tick at up to 2× the mode); the cap's `over` meter would show it.

The worker logs each handoff (`kf3: display: +N ms head H tick source -> BROKER|FREE-RUNNING clock (...)`, at most 32 lines) and the
status line carries `src=free|broker ho=<to broker>/<back to free>` per head.

## 6. Flips and copies (phase 1, built)

### 6.1 The buffer-reuse rule (the correctness condition)

Completing a flip writes the release semaphore of the entry it flipped away and the FINISHED notifier of the previous entry
(`engine.rs` `complete`): it **releases the buffer the flip replaced** to the guest, which may render into it the next instant. A scanout
copy that is still reading that buffer would read torn pixels, and the guest's frame would be corrupted *in the host's view*. So:

> **A flip's completion items (and its GET) wait for the copy that was in flight at its latch — and for nothing else.**

(`pace::flip_completion_need(floor, inflight) = max(floor, inflight)`, applied in `ScanState::on_flip_latched`; `floor` is whatever the
queue already waits for, so completions stay in order.) Why this is enough and not more:

- A copy reads the engine's armed state **at its start**. A copy that started before the latch read the replaced buffer: the completion
  waits for it (milliseconds, the copy's duration — never a tick). A copy that starts after the latch reads the *new* buffer, which the
  guest still sees as scanout and is not released by this flip.
- No copy in flight at the latch: nothing read the replaced buffer, the completion publishes in the same pass. The effect loop runs
  *before* the pass's `scan.start`, so a copy cannot start between the latch and the queueing of its items.
- A frame latched and then replaced before any copy (the mailbox's `skipped`) was never read: its release is immediate by the same rule.
- Only the buffer **currently latched as scanout** is ever copied (the copy reads the armed state, which is the newest latch). A flip
  never makes a copy wait for a tick *and* never makes the guest wait for one.
- Verified in the model by counting, for every published latch, whether a copy still in flight reads the buffer it releases:
  0 violations across copy times 0–33 ms, 2 and 3 buffers (`a_flip_never_completes_under_a_copy_of_the_buffer_it_replaces`); the mutant
  worker that ignores the copy in flight is caught (> 50 violations).

**The context-DMA corollary.** The first copy after a latch used to resolve the window's context DMA — and the flip's completion waited for
that copy, so a guest that waits for the channel to idle before unbinding it (NVKMS's teardown: `nvEvoClearSurfaceUsage` →
`nvRMSyncEvoChannel`, then the unbind) found it resolved (`LatchedDmas`). With the copy up to a tick away, the worker now resolves the
window's context DMA **at the latch** (`ScanState::resolve_now`); a refusal is not kept and the copy retries, as before.

### 6.2 What stays

- **Vsync (non-tearing) flips** park for their head's tick as before and complete at it; their frame is copied by the same mailbox (same
  pass, if the credit is there). A group that mixes a tearing and a non-tearing window on one head is a vsync update.
- **Core-holding updates** latch at once (a modeset), count in `core_imm`, and are judged by `over` as before; the copy that shows the new
  mode is the `want` copy (a modeset is not rate-bounded).
- **Front-buffer rendering, the boot console, X11 without a compositor, the screendump** keep their D2 timed copies: a non-flip check at the
  console head's tick, sent when the checksum changed (`NonFlip`). A pending flip frame replaces the check (`flip_pending`).
- **`display-max-fps`** still clamps the tick, so it bounds everything vblank-paced *and* the copies (one per tick). It no longer bounds
  the guest's immediate-flip rate; that is by design.
- **The colour experiment** (`KF3_DISPLAY_SDR_COLOR=1`) resolves LUT context DMAs at copy time: it keeps the old rule (the flip's items
  wait for its own copy, `want`) until it is validated under the mailbox. Off by default.

### 6.3 The mailbox (`pace::Mailbox`)

`latch()` marks a newer frame (a frame already waiting is replaced unseen: `skipped`); `tick()` grants one copy credit (never more than one,
and a tick with nothing to copy keeps it, so an idle head's first flip is copied at once); `due(in_flight)` = a frame waits ∧ a credit ∧ no
copy in flight; `copied()` spends both — **any** flip-class start does, `want`-driven or not, because every copy reads the newest state.
Hence at most `ticks + 1` copies in any window (a test over 10 000 random ticks), which is the bound `over` judges.

### 6.4 `held`, `over`, `core_imm`, `skipped` in the status line

`held` is gone (nothing is held). `tear` is the immediate latches per second and may be any number. `over` counts windows in which
**non-tearing** presents exceeded `floor(L/P) + 2` per head, **or the published frames did** (`over_copies`, against the fastest head's
period); it can fire for one window at a modeset (a `want` copy and the tick's). `core_imm` is unchanged. `skipped` is cumulative frames
replaced before a copy — 150/s at R = 4 ms in the model: the host sees the newest of them every tick.

## 7. Tests

GPU-free: `cargo test --release -p kf-disp -p kf-broker -p kf-qemu` — **614 passed / 8 ignored on the base `ec42327f0` → 640 passed /
8 ignored** (kf-disp lib 162 → 185, kf-qemu lib 287 → 290, kf-broker and the rest unchanged); 0 failed. `rustfmt --check` is clean on every line
this branch wrote (the files carry seven older unformatted hunks, `engine/tests.rs` and `display.rs`, left alone so the diff stays
reviewable); `cargo clippy` reports nothing on the changed lines (`kf-qemu`'s lib has 89 older clippy errors under the workspace's
restriction lints, so clippy does not finish its test target).

**Changed or removed tests, each deliberately:**

| test | change | why |
|---|---|---|
| `engine::tests::a_second_tearing_flip_waits_for_the_vblank` | **removed** | it asserted D1 (§AK removes the gate). Replaced by the next two |
| `engine::tests::an_immediate_flip_latches_at_once_however_many_come_between_ticks` | **new** (replacement) | 50 immediate flips between two ticks all latch at once, none waits, every one a present and a tearing present |
| `engine::tests::a_vsync_flip_waits_for_its_tick_and_an_immediate_one_beside_it_does_not` | **new** (replacement) | the non-tearing flip still parks for its tick; a tearing flip beside it does not; a mixed group is a vsync update |
| `engine::tests::a_head_composes_its_windows_back_to_front` | changed | it relied on the second tearing flip waiting for the tick (`assert!(e.waiting(4)); e.vblank(1)`); now `assert!(!e.waiting(4))` |
| `pace::tests::meter_over` | changed | tearing presents are exempt from `over` (66 presents of which 3 tear are over, 63 of which 3 tear are not), the published frames have their own bound, judged against the fastest head |
| `pace::tests::the_fragment_carries_every_path` | changed | the fragment has no `held`; it has `src=`/`ho=` and `skipped=` |
| `engine/tests.rs` helpers | `pub(super)` | the model (`engine/pacing_model.rs`) reuses `engine()`, `lit()`, `Ring`, … |
| `scripts/bench/display/hook.sh` | comment only | it described the gate's effect on the async probe |

**New:** `engine::pacing_model` (9 tests: the loop on the new code ≥ 200 flips/s at R = 4 ms with the display ticking at 60 and copies ≤ ticks;
vsync flips still at the tick; the alternating loop at 60 frames/s; an acquire costs 2 ms; a core-holding flip changes nothing; **the
buffer-reuse rule** across copy times 0–33 ms with the mutant worker caught; the legacy worker without the gate copies every flip; a dead
broker never freezes the guest; a drifting broker never double-ticks), `pace::tests` (8: mailbox once per tick and newest wins; ≤ ticks + 1
copies over 10 000 random ticks; the completion rule; the pacer without a broker is the free clock; the vblank number survives a re-arm; a
broker event is a numbered tick and the handoffs are reported; the guard edge and a new period; tearing exempt and copies judged),
`tick::tests` (5: the **exhaustive transition table**, hostile event orders, a fuzzed interleaving with a clock that goes back, the
timestamp under a backwards clock, the constants), `kf-qemu display::tests` (3: `on_flip_latched`/`flip_pending`/`reset_life`;
`begin_copy`/`on_console_tick`/`on_window_latched`; a source scan: no gate, no flag, the mailbox wired).

**Mutation table.** Each mutant applied in place, the named suites run (`kf-disp --lib`, and `kf-qemu --lib display::` for those touching
`pace.rs` or the worker), the failing tests counted, the mutant removed (`git status` clean after). Target: ≥ 2 failing tests.

| mutant | failing tests |
|---|---|
| T1 broker double-tick guard removed | 6 |
| T2 guard `<` → `<=` (the edge) | 2 (transition table, guard-edge through the pacer) |
| T3 silence 50 ms → 5 s | 4 |
| T4 a broker event does not make the broker the source | 5 |
| T5 the free deadline is not dropped while the broker speaks | 4 |
| T6 takeover not on the broker's phase grid | 2 |
| T7 timestamps not monotonic (`stamp = now`) | 2 (backwards clock, fuzz) |
| T8 vblank number not advanced | 3 |
| T9 `retime` keeps the broker source on a new period | 3 |
| T10 `retime` re-arms on the same period | 2 (`on_heads_keeps_phase_under_the_clamp`, table) |
| T11 free clock rescheduled from now always | 3 |
| T12 no handoff reported to the caller (the log) | 2 |
| P1 a copy does not spend the credit | 2 |
| P2 `due` ignores a copy in flight | 2 |
| P3 a replaced frame is not counted `skipped` | 2 |
| P4 the completion ignores the copy in flight | 4 |
| P5 the completion waits for its own copy (`inflight + 1`) | 4 |
| P6 a tearing present counts against `over` | 2 |
| P7 published frames never `over` | 2 |
| P8 copies judged against the slowest head | 2 |
| P9 a broker tick is not numbered by the pacer | 5 |
| P10 a tick grants no copy credit | 4 |
| P11 `Mailbox::reset_life` keeps the pending frame | 2 (kf-disp mailbox, kf-qemu worker) |
| E1 the gate back (a tearing flip parks for the vblank) | 10 |
| E2 a tearing flip no longer counted as tearing | 2 |
| W2 `on_flip_latched` leaves the barrier alone | 2 |
| W4 `flip_pending` ignores the mailbox | 2 |
| W5 `begin_copy` does not spend the mailbox | 2 |
| W1 the old `started + 1` barrier back at the worker's call site | **1** (source scan) |
| W3 the shown head's `on_console_tick` call dropped | **1** (source scan) |
| W6 the `KF3_TEAR_GATE` flag back | **1** (source scan) |
| W7 a non-flip check spends the pending frame | **1** (behaviour test) |
| W8 a tick grants a copy for a boot picture | **1** (behaviour test) |
| W9 the colour experiment loses its old rule | **1** (behaviour test) |

The six single-detector mutants are in the worker's loop (a call removed, a flag put back) or in a branch only the display worker
reaches; the worker's loop cannot run without a GPU, so a call-site mutant has exactly one detector by construction (the source scan, with a
planted known-positive), and W7–W9 are each pinned by the one behaviour test of the function they mutate. They are listed, not hidden.

## 8. Later phases (DESIGN-ONLY; nothing here is built)

**Phase 2 — the broker as tick source.** A capability bit in the broker protocol (a new `CAP_*` next to `CAP_RELEASE`, advertised by a
broker that can report its presenter's refresh) and a `TICK` message from the broker at the **presenter's** refresh (the host compositor's
frame callback / `presented` feedback), carrying the presentation timestamp and sequence. The relay turns it into
`Pacer::broker_tick(head, now)` — the entry point and the pure state machine of §5 already exist; the worker needs only the call. The
broker must **commit every refresh** (even an unchanged frame) so compositors keep sending frame callbacks (Wayland stops them for a
surface that does not commit; an occluded or minimised surface goes silent — which is exactly the silence the 50 ms fallback exists for).
The broker decimates to the guest mode's period (§5). Timestamps: the guest's vblank timestamp stays kayfabe's monotonic stamp; the
presentation timestamp is for phase alignment only.

**Phase 2b — credit-based lock step (decision (b)).** The tick credit of §6.3 is the place a broker credit plugs in: the broker sends a
frame request only after it received (and released) the previous frame; the mailbox's `tick()` becomes "a request arrived", `due` stays the
same. The ring's RELEASE already tells kayfabe which frames the broker holds. Nothing in phase 1 prevents this: the copy count is already
bounded by credits (ticks), the rule of §6.1 does not mention the tick, and a stale credit is harmless (bounded to one).

**Phase 3 — HDMI over TMDS (decision (d), D3).** A virtual connector that advertises HDMI (a TMDS link with HDMI-class pixel clocks) instead
of single-link DVI lifts the EDID's 75 Hz range limit and the 165 MHz pixel clock (§M D3: 1080p tops out at ~71 Hz on DVI). Needs the
connector authoring in `kf-rm`'s display model, an EDID with an HDMI vendor block, and the cap's `MAX_HZ`; it changes nothing in §5–§6.

## 9. Hardware checklist (the owner's run; nothing here has been run on a box)

Fullscreen Unigine Heaven D3D11 on the Windows guest, the prod-candidate flags, the kf3 per-second line `kf3: display fps[…]`:

- **The goal:** the app's own fps in exclusive fullscreen is no longer pinned at ~30. Expected on a **single immediate flip per frame**:
  fps follows the render rate (the model: 210 at R = 4 ms with two buffers, 250 with three), `tear` ≈ the app's fps, and the kf3 line shows
  `tick ≈ 60`, `copies ≈ 60` (**never above the tick rate**), `skipped` rising by ≈ (fps − 60)/s, `over=0`, no `held=` field.
- **If fullscreen reads ~60 instead:** the frame is two flips (finding 3) and the vsync one is the ceiling. Then `tear` ≈ fps and
  `kms` ≈ 2 × fps. Run it once with `KF3_DISPLAY_TRACE=1` (bounded, `KF3_DISPLAY_TRACE_CAP`) and count the `chn … UPDATE` /
  `latch` lines per displayed frame and their `BEGIN_MODE`; that is the falsifier of §2.4 item 3 and the input to the next decision.
- **Look at the picture** while it runs: no tearing, no flicker, no stale or half-drawn frames — the buffer-reuse rule's failure would show
  as corruption when the app renders fast (3 buffers, small R). A frozen host image with the guest still flipping would be a lost copy.
- **`src=free ho=0/0`** on every head, always (nothing feeds a broker event yet): any `ho=` other than `0/0`, or a `tick source -> …` log
  line, is a bug.
- **Windowed phases** must read as before: `tear=0`, the vsync presents at 60, `copies` ≤ 60.
- **The STALL line:** if `display: STALL Window 0 …` appears again, save the whole line (head, acquire, current).
- **Regression runs** (CLAUDE.md): `v3_gates.sh`, the fast suite 30/30, and — display code changed — the Linux guest with the broker
  (`broker_lane.sh`, `interactive.sh`): vsync clients stay at the tick, a cursor and typing stay responsive, no `Flip event timeout`.

## 10. What is not verified

- **Everything above on hardware.** The model is the arithmetic of the code, with an assumed guest loop; it cannot say what Windows
  sends (finding 3 is INFERRED) or how the real GPU schedules the copy next to Heaven's rendering.
- **`held=+60/s`** (§2.4 item 5) is not explained.
- The worker's wiring (`display.rs`) is tested through `ScanState::on_flip_latched`/`flip_pending`/`reset_life` and a source scan, not
  against a GPU; the eager context-DMA resolution runs only with a GPU.
- Phase 2/3 are designs; the 24 Hz margin of the 50 ms silence is 8 ms.
