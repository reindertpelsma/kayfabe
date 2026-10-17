# V3 display pacing — the guest's flips are never parked; copies are bounded; who ticks

**STATUS: LIVE, 2026-10-17.** Phase 1 (§4–§6) is BUILT on `claude/display-pacing-20261017` (cut from `claude/tear-gate-exp-20261017`
`ec42327f0` = `integration/prod-candidate-3-20261017` + the `KF3_TEAR_GATE` measurement flag), GPU-free tested, **NOT RUN ON A BOX**.
Phase 2 (§11: the broker as tick source, lock-step frame requests) is BUILT on `claude/display-tick-20261017`, GPU-free tested, **NOT RUN ON A BOX**. Phase 3 (§8) is DESIGN-ONLY. The owner's decisions are `OWNER_RULINGS.md` §AK (they amend §M D1). The mechanism this
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

**Phase 2 — the broker as tick source.** ⊘ **SUPERSEDED 2026-10-17 by §11** (the wire, the lock step and the enforcement are decided and built there; this paragraph is the earlier sketch, kept for the record). A capability bit in the broker protocol (a new `CAP_*` next to `CAP_RELEASE`, advertised by a
broker that can report its presenter's refresh) and a `TICK` message from the broker at the **presenter's** refresh (the host compositor's
frame callback / `presented` feedback), carrying the presentation timestamp and sequence. The relay turns it into
`Pacer::broker_tick(head, now)` — the entry point and the pure state machine of §5 already exist; the worker needs only the call. The
broker must **commit every refresh** (even an unchanged frame) so compositors keep sending frame callbacks (Wayland stops them for a
surface that does not commit; an occluded or minimised surface goes silent — which is exactly the silence the 50 ms fallback exists for).
The broker decimates to the guest mode's period (§5). Timestamps: the guest's vblank timestamp stays kayfabe's monotonic stamp; the
presentation timestamp is for phase alignment only.

**Phase 2b — credit-based lock step (decision (b)).** ⊘ **SUPERSEDED by §11** (built together with phase 2). The tick credit of §6.3 is the place a broker credit plugs in: the broker sends a
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

## 11. Phase 2 — the broker as tick source, lock-step frame requests (2026-10-17, branch `claude/display-tick-20261017`)

**STATUS: LIVE (design and code), GPU-free tested, NOT RUN ON A BOX.** Owner decisions: `OWNER_RULINGS.md` §AK (a)–(d) and §AL (kayfabe enforces the lock step; the flip system is testable locally).
Labels as above. **What this does NOT claim:** phase 1 could only reproduce 60 flips/s, not the owner's 30 (§2); the two-flips-per-frame cause is INFERRED. Nothing in this phase is
evidence that the broker tick lifts fullscreen from 30 fps. Only the hardware run (§11.9) can show it.

### 11.1 What is decided

1. **One new capability bit, one opt-in, one event, one command.** The broker advertises `CAP_TICK`; the VMM opts in with `CMD_CAPS`; only then does the broker send `EV_TICK`, and only then does
   it expect `CMD_NO_CHANGE` / a sequence on `COMMIT`. Nothing else on the wire changes.
2. **The broker is the tick source when it speaks; the free clock otherwise.** `EV_TICK` is a heartbeat on EVERY (decimated) host refresh while the window is visible; the VMM feeds it to
   `Pacer::broker_tick` (§5) for the head the broker shows. 50 ms without one hands the head back to the clock (`BROKER_SILENCE_NS`); the next one hands it back. The guest's vblank is never gated.
3. **Lock step is a REQUEST per frame.** `TICK.WANT_FRAME` opens request `R = tick_seq`; kayfabe answers it with exactly one `COMMIT` carrying `seq = R`, or one `NO_CHANGE(R)`; the broker opens the next
   request only after the answer AND after the answered frame was presented (or discarded) by the compositor, or after a timeout. A frame desync cannot happen: a frame is either the answer to the
   open request or it is not sent.
4. **Kayfabe enforces; it never trusts the broker's timing (§AL).** §11.4.
5. **Old and new degrade to today's behaviour** (§11.2 matrix); an old broker, an old VMM, a broker without a bit, headless: the free clock and the legacy credit, bit-identical counters (tested).

### 11.2 The wire (the broker's `broker/common/nvkvm_broker_proto.h` is the one copy; `kf-broker/src/wire.rs` mirrors it, `tests/proto_mirror.rs` compares)

Bit 12 is `CAP_CURSOR_STREAM`, 11 `CAP_DEVICE`, 10 `CAP_CURSOR`; **bit 13 is the next free** (checked: no other `1u << 13` in `broker/` or `crates/kf-broker`).

| item | value | meaning |
|---|---|---|
| `NVKVM_BROKER_CAP_TICK` | HELLO `w1` bit 13 | this backend can report the display's refresh (`EV_TICK`) and runs lock-step requests. Set only by a backend that has what it needs: Wayland with `wp_presentation` or frame callbacks, and the test backend. **X11 does not set it** (no refresh feedback it can commit against: clock only) |
| `NVKVM_BROKER_CLIENT_TICK` | `CMD_CAPS.width` bit 1 | the VMM understands `EV_TICK` and will answer requests. Sent only when HELLO had `CAP_TICK` |
| `CMD_CAPS.height` | 0, or 24..=75 | with `CLIENT_TICK` only: the most Hz the VMM's vblank may run at (`display-max-fps`, else 75: D3). 0 = 75. Anything else, or `height != 0` without `CLIENT_TICK`: framing violation |
| `NVKVM_BROKER_EV_TICK` | event type **18** | below |
| `NVKVM_BROKER_CMD_NO_CHANGE` | command type **8** | below |
| `COMMIT.seq` | (was "advisory, logged only") | with `CLIENT_TICK`: the request answered; 0 = unsolicited |

**`EV_TICK`** (24 bytes, an overlay like the clipboard's, `struct nvkvm_broker_tick_pkt`):

```text
 off size field
  0   u16  type        = 18
  2   u16  flags       = the mirrored F_GRABBED|F_FOCUSED|F_FULLSCREEN, exactly as on every packet
  4   u32  seq         = the connection's packet counter (as on every packet; NOT the tick number)
  8   u32  tick_seq    = the tick number: first tick 1, +1 per EMITTED tick, wraps skipping 0
 12   u32  refresh_word= bits 0..27  refresh_ns (the DECIMATED period: host period x div; 0 = unknown, else 1_000_000..=268_435_455)
                         bit 28 TICK_F_WANT_FRAME  (this tick opens request tick_seq)
                         bit 29 TICK_F_CLOCK_ONLY  (present_ns is the broker's CLOCK_MONOTONIC when it LEARNED of the refresh
                                                    (frame callback, or feedback on another clock), not the compositor's timestamp)
                         bits 30..31 reserved, zero (nonzero: framing violation, the VMM disconnects)
 16   u64  present_ns  = CLOCK_MONOTONIC ns of the presentation (w0 = low, w1 = high), non-decreasing
```
(`tick_seq` u32 instead of the brief's u64: 24 bytes cannot hold a u64 sequence, a u64 timestamp, the refresh and the flags. A u32 at 240 Hz wraps in 207 days; every comparison is
wrapping and only needs "newer than"; 0 is never a tick, so 0 can mean "none".)

**`CMD_NO_CHANGE`** (40 bytes, the standard command record): `type = 8`, `width = R` (the request answered, `!= 0`), every other field zero, no fd. **`COMMIT`** with `CLIENT_TICK`: `seq = R`; ATTACH's `seq` stays advisory.

**Bounds and violations (hostile broker, hostile VMM; no length field anywhere, so nothing is read past 24 / 40 bytes).**

| who checks | what | verdict |
|---|---|---|
| VMM (`kf-broker` relay) | `EV_TICK` without having sent `CLIENT_TICK`; reserved flag bits; `tick_seq == 0`; `refresh_ns` set and outside 1 ms..268 ms; a descriptor on an event (already) | violation: the VMM drops the connection (`lost`, reconnect with backoff) |
| VMM | `tick_seq` not newer than the last (duplicate, stale, reordered) | dropped and counted (`ticks_stale`); not a violation (a proxy may reorder; nothing is acted on) |
| VMM | `present_ns` goes backwards | counted (`ts_backwards`), the tick is processed (the timestamp is statistics and phase only) |
| VMM | a gap in `tick_seq` | counted (`ticks_missed` += gap); fine (the broker coalesces, §11.3) |
| broker | `CMD_NO_CHANGE` / `COMMIT.seq != 0` / `CAPS.height != 0` without `CLIENT_TICK` | violation (like an unknown command) |
| broker | `NO_CHANGE.width == 0`, or a request id the broker never issued (newer than the last tick sent) | violation |
| broker | `COMMIT.seq` issued but not the open request (late, a timed-out request, a duplicate) | the frame IS presented (dropping visible content is worse), counted `late_replies`, the open request stays open |
| broker | `COMMIT.seq == 0` in tick mode | presented, counted `unsolicited` (a replay frame, a VMM that ignored the credit); never closes a request |
| broker | `NO_CHANGE` for a stale / duplicate request | ignored, counted `late_replies` |
| broker | a flood of any of the above | the existing command-rate limiter (`rate_count`) and the work budget apply unchanged |

**Degradation matrix.** (broker, VMM) → behaviour:
(old, old) and (old, new): no `CAP_TICK`, the VMM never opts in, no `EV_TICK`, the VMM's free clock and the legacy credit (`EV_FRAME`/`RELEASE`/100 ms backstop) — exactly today.
(new, old): the old VMM ignores bit 13, sends `CAPS` without `CLIENT_TICK`; the broker sends no `EV_TICK`, expects no sequence (it is `seq`-tolerant: `COMMIT.seq` of an old VMM is advisory) — today's exchange, including `EV_FRAME`.
(new, new): tick mode. The broker keeps sending `EV_FRAME` (the VMM ignores it while ticks are live, and uses it again in fallback).
An old broker given `CAPS.width` with an unknown bit would store it (`caps_seen`) harmlessly; the VMM never sends it to one.

**Broker's TX queue.** ⊘ CORRECTED by the C-side build (a "never dropped WANT" rule lets a peer that stops reading accumulate one WANT tick per 100 ms request timeout until the 512-packet ring
overflows from ticks alone): a queued, unsent tick is REPLACED by a newer one unless it carries `WANT_FRAME` and the newer does not (the live request exists only in that packet); a NEWER `WANT_FRAME`
replaces an older queued one (that request is dead: the broker already moved on). The queue therefore holds at most 3 packets however long the peer sleeps (MEASURED, 6 000 000 feedbacks to a peer that never read:
queue 3 packets, 11 977 ticks emitted, 11 975 coalesced, 5 988 023 dropped as too fast). Further as built: `CAPS` with `CLIENT_TICK` on a broker that did not advertise `CAP_TICK` is a violation; `NO_CHANGE` with any
other field set, or without `CLIENT_TICK`, or for request 0, is a violation; `CAPS` without `CLIENT_TICK` after one with it withdraws the opt-in (one log line); a feedback is dropped (counted) when it is the same
instant or earlier than the last accepted, closer than half a HOST period to it, or (compositor timestamps) the same vsync; `EV_SURFACE` is also re-sent on a refresh-only change (before, an equal size returned early and
dropped it); under `--refresh auto` `EV_SURFACE.w0` is `host_mhz / div` (144 Hz → 72000 mHz, 165 → 55000, 240 → 60000). The ring's 512 packets and 4-packet reserve are untouched.

### 11.3 The broker (C): decisions are pure functions, backends are thin

Pure (`broker/nb_tick.c`, table-tested with mutants, `test/test_tick.c`):
- `nb_tick_div(host_period_ns, guest_max_hz)` = `max(1, ceil(host_hz / guest_max_hz))`, `guest_max_hz = min(client cap, 75)`: 60→1, 75→1, 100→2, 120→2, 144→2, 165→3, 240→4. The ticks are every
  `div`-th host refresh and the mode the broker advertises (`EV_SURFACE.w0`, under `--refresh auto`) is **`host_mhz / div`** (144 Hz host → a 72 Hz mode, 2:1). Under `--refresh <fixed>` /
  `none` the advertised rate is not the host's: `div = max(1, round(host_hz / mode_hz))` (`none`: 60). 75 Hz (D3) is NOT lifted.
- `nb_tick_want(state, now)`: `WANT_FRAME` iff no request is open and no answered frame is un-presented; or the open request is `REQ_TIMEOUT_NS` (100 ms) old; or an answered frame has waited
  `REQ_TIMEOUT_NS` for its presentation feedback (counted `present_timeouts`).
- `nb_tick_answer(state, seq)` → ANSWERS_OPEN / LATE / VIOLATION; `nb_tick_presented(state)`.
Core (`nvkvm_broker.c`): `nb_sink_refresh(sink, present_ns, refresh_ns, flags)` is the one entry a backend calls per host refresh: it decimates, decides `WANT_FRAME`, emits `EV_TICK`. A priming `EV_TICK`
(`WANT_FRAME`, `CLOCK_ONLY`) goes out when the VMM opts in, like the priming `EV_FRAME`.

Wayland (`nb_session_wl.c`): `wp_presentation.feedback` + `wl_surface.frame` on every commit; `presented` → `nb_sink_refresh` (compositor timestamp, `clock_id` checked: not `CLOCK_MONOTONIC` ⇒ `CLOCK_ONLY`)
and `nb_sink_presented`; `discarded` → `nb_sink_presented` too (the frame is gone, the request must not wait); with no `wp_presentation`, `frame_done` stands in (`CLOCK_ONLY`, a documented loss: the instant is
when the callback arrived, not when the display scanned out). **Commit every refresh (⊘ CORRECTED by the measurement, §11.8: the broker commits 1x1 damage on the buffer already attached, driven by the `presented` it produced; an EMPTY commit gets a callback at once and a `discarded` at once and cannot be the pump):** the wl_surface protocol text says a frame callback is *requested* by `frame` and "takes effect on the next
`wl_surface.commit`"; it only says a server "should avoid signaling the frame callbacks if the surface is not visible in any way" and does not promise one for a commit without damage, so the
claim "an empty commit still gets a callback" is NOT in the protocol text. §11.8 records what was MEASURED on `mutter --headless` and which variant the broker commits (empty / 1x1 damage on the
current buffer). Occluded or minimised: callbacks stop, `EV_TICK` stops, the VMM's 50 ms fallback runs the clock, the legacy credit sends a frame every 100 ms; the guest sees an uninterrupted
vblank (§5) — nothing freezes. Monitor move / refresh change: the next `presented` carries the new `refresh`; `div` is recomputed, decimation restarts, `EV_SURFACE` is re-sent with the new mode rate (the existing
`wl_report_surface(force)` path), and the VMM `retime`s (§5). Fullscreen: nothing special — the same feedback; `F_FULLSCREEN` stays mirrored.

### 11.4 Kayfabe enforces the lock step and never trusts the broker's timing (§AL)

Two small pure machines, both table-tested, both used by the real code and by the harness (§11.7):

**`kf_broker::ticks::Requests`** (the relay, one per connection; main loop thread) — the wire half:
- R1 a tick whose `tick_seq` is not newer than the last is **dropped and counted**; nothing is acted on.
- R2 a `WANT_FRAME` tick opens request R **only if** none is open; if one is open, the new one REPLACES it (the broker timed out) and `credits_coalesced += 1`: **at most ONE open request**.
- R3 a frame is sent only while a request is open, spends it (exactly one `COMMIT` per request, `seq = R`), and a second ready frame waits. No frame without a credit; no more than one per credit.
- R4 `NO_CHANGE(R)` is sent when R is open at a LATER tick and nothing is ready, held or owed (an idle guest): it closes R. The mailbox's credit on the worker is not touched.
- R5 **in-flight bound:** committed-not-released frames are at most `HELD_CAP` = 2 (the ring's existing, named bound); a third ready frame waits (`blocked`).
- R7 **work per turn:** the relay reads at most `READ_BATCH` = 64 packets per turn (the named per-turn message budget); the rest stays in the socket (level-triggered) and is read next turn, counted (`budget_hits`); a vblank
  or a flip completion never waits behind a flood. **Sending never blocks:** every send is non-blocking; a full socket makes the frame OWED (retried on writability / every 50 ms) and counted (`dropped`, `uncommitted`),
  a `NO_CHANGE` that meets a full socket is retried at the next tick. **Read deadline:** a partial packet (fewer than 24 bytes) that does not complete within `PARTIAL_READ_MS` = 500 ms drops the connection loudly (`the display
  broker stalled mid-packet`, `partial_timeouts`), so partial-read state is one 24-byte buffer and a clock, never more. A whole-packet-aligned silence is not a fault: it is the 50 ms fallback's business.
- R8 **`NO_CHANGE` is paced:** not sooner than half the broker's refresh (≥ 1 ms) after the last answer, so a flood of `WANT_FRAME` ticks cannot make kayfabe answer faster than the display refreshes (the unanswered request is simply
  replaced by the next; `credits_coalesced`).
- R6 **tick-live** = ticks negotiated ∧ a tick arrived within `BROKER_SILENCE_MS` (50). Not live: the legacy credit decides (frames at the `EV_FRAME`/`RELEASE`/100 ms rule) and an open request is
  still answered by the next frame. Live again: the requests decide.

**`kf_disp::lockstep::CreditGate`** (the worker, per shown head) — the vblank/copy half:
- G1 an event with a `tick_seq` not newer than the last is dropped and counted (defense in depth: the relay already did).
- G2 a `WANT_FRAME` whose tick the double-tick guard SWALLOWED (closer than P/2 to the previous tick) is **carried to the NEXT ACCEPTED tick and coalesces into that tick's own credit** — never acted on early, and never in addition
  to the next tick's credit. ⊘ CORRECTED by the exhaustive model's flood (finding F3, §11.7): the first design deferred it to the guard's edge `last_tick + P/2`, which with the next accepted tick's own credit gave TWO credits per
  guard interval and made the COPY rate exceed the vblank rate (66–79 copies against 40 vblanks). Now copies ≤ vblanks. A swallowed tick's credit has no time of its own, so the worker does not wake for it; if the broker goes silent
  the fallback clock's ticks grant credits as in phase 1.
- G3 a credit arriving while a copy is in flight or a credit is already held is **coalesced**: `Mailbox.credit` is a bool (at most ONE pending credit), the extra is counted (`credits_coalesced`) and
  dropped. N ticks while the copy runs → one frame afterwards.
- G4 the copy starts only when `Mailbox::due` (frame waiting ∧ credit ∧ no copy in flight, §6.3): never earlier than the credit, never before the previous copy completed, never without a free ring
  slot (no slot: skipped, counted — the frame stays pending for the next credit).
- G5 **the immediate flip is untouched.** A credit gates the COPY (the host's view) and nothing else: a flip latches and completes at once whether or not a credit exists (§6.1 stays: its
  completion waits for the copy in flight at its latch, and for nothing else). This is asserted by the harness (an immediate flip never parks, never waits for a credit) and by a mutant that makes
  it wait.
- G6 the worker reads ticks through `TickShare` (below), which holds the LATEST tick plus the newest unconsumed `WANT` request: a flood leaves the worker one vblank per pass (no catch-up burst) and
  at most one credit; the dropped ticks are counted (`ticks_coalesced`).
Flood bound, summarised: the vblank rate ≤ 1 per P/2 (the guard), copies ≤ 1 per accepted credit, frames on the wire ≤ 1 per open request and ≤ 2 unreleased, pending credits ≤ 1, the TX queue ≤ 2 ticks.
A hostile or flooding broker costs counted dropped ticks and nothing else.

**The cross-thread share** (`kf_broker::ticks::TickShare`, lock-free, no lock waited on between the relay on QEMU's main loop and the display worker): a single-writer seqlock for the latest
tick `(count, tick_seq, present_ns, refresh_ns, flags)`, one `AtomicU32` for the newest unconsumed `WANT` request (the relay stores, the worker swaps it out), the connection epoch, and the worker's
wake `Notifier` (one eventfd write per tick, ≤ 240/s). The worker feeds `Pacer::broker_tick(shown_head, now)` and `CreditGate`; other heads keep their free clocks.

### 11.5 The handoff, as the guest and the status line see it

§5's state machine, fed by the broker: the first accepted broker tick hands the head to the broker; silence ≥ 50 ms hands it back on the broker's phase grid; a tick closer than P/2 is swallowed.
**Flap damping (added here, flagged for the owner; `kf_disp::tick`):** after a FALLBACK the broker is on probation — it takes the tick back only after `probation_gaps(k)` consecutive gaps between its events, each shorter than
50 ms (`k` = fallbacks in a row without a one-second stable spell: 1, 2, 4, 8 gaps, 8 the cap). The first takeover of a broker that never fell back is still at once (decision (c)). A broker that speaks at 55 ms gaps,
or alternates 40/60 ms, therefore changes the source once or a few times in all, not per tick (tested, the counts in §11.7); events on probation are swallowed (`probation=` in the status) and the clock keeps ticking. **One log line per tick-source change** (the existing `tick source -> BROKER|FREE-RUNNING clock`, ≤ 32) and one relay line per tick-negotiation change (`broker ticks: ON (...)`/`OFF (...)`),
never per tick. Counters (all bounded `u64`, in the status line `broker[...]`/`disp[...]`): `ticks` (accepted from the wire), `ticks_stale`, `ticks_missed`, `ticks_coalesced` (worker took the latest of
several), `want_ticks`, `credits_coalesced`, `credits_deferred` (guard), `frames_sent_on_credit`, `no_change_sent`, `skipped_no_slot` (counted by the worker), `silence_fallbacks` (= `ho=`'s second
number), `handoffs_to_broker` (= `ho=`'s first), `req_open_blocked` (a ready frame waited for a request), `ts_backwards`.

### 11.6 What does not change

Immediate flips (§AK(a)), the buffer-reuse rule (§6.1), the mailbox (§6.3), `over` (§6.4), the XOR/cursor code, D3 (75 Hz), the kayfabe-* crates, Linux / Translated paths, the legacy broker
protocol for old peers. No flag is required to boot; nothing is a measurement flag.

### 11.7 Tests, results, mutants (GPU-free; every number MEASURED on the dev box, `cargo test --release`, 4 cores)

**Counts.** `cargo test --release -p kf-disp -p kf-broker -p kf-qemu`: **640 passed / 8 ignored on the phase-1 base `dd19a9147` → 708 passed / 12 ignored** (kf-disp lib 185 → 229, `kf-disp/tests/lockstep_threads.rs` +7,
kf-broker lib 50 → 51 and `relay_machine` +9, `proto_mirror` +1, kf-qemu lib 290 → 291); 0 failed. The ignored: the two 10 M-flip / deep-exploration runs, `broker_ticks_e2e` (needs the broker binary) and
the earlier ignored loopbacks. Broker C: `make check` 245 → 365 PASS lines (new: `test-tick` 566 195 checks, `mutate_tick.py` 28 mutants all killed by >= 2 distinct failing checks, `test_tick.py` 122 PASS over the real socket);
`make check-sanitize` green (ASan+UBSan, 0 reports); `make check-tick-probe` (real `mutter --headless`) passes. rustfmt is clean on every file this branch wrote; clippy reports nothing on the changed lines.

**The pure units** (table-tested, `kf_broker::ticks`, `kf_disp::{tick,lockstep,pace}`): `Requests` (R1–R4, R8), `frame_gate`/`tick_live` (every input combination), `TickShare` (latest tick, newest request, wake), `CreditGate`
(G1–G3 rows with exact counters), `Mailbox::grant`, `TickSource` flap damping (`probation_gaps` 1, 2, 4, 8; 55 ms gaps → one fallback in all; 40/60 → at most 6 changes in 400 events), the wire (`Pkt::tick`, `Cmd::*`, offsets from the compiled
header, `seq_newer` around the wrap), the relay against a scripted link (`relay_machine`: opt-in only when both sides can, one frame per request with `COMMIT.seq`, `NO_CHANGE` at a later tick and paced, stale/violation rows,
50 ms live / 51 ms silent, read deadline 499/500 ms, a 1000-tick flood read 64 per turn with exact conservation, an old broker gives bit-identical wire records and counters).
**The relay against the REAL broker** (`broker_ticks_e2e`, `--backend test`, real unix socket): 63 ticks, 33 requests, 2 frames on credit, 30 `NO_CHANGE`, 0 stale/late/unsolicited, 0 violations, silence → fallback → resume with no flap.

**Harness (A): the exhaustive bounded model** (`kf_disp::engine::lockstep_exhaustive`, 18 tests + 2 ignored deeper runs; 3 s for the module). Depth-first over every ordering of an alphabet of (time advance to a boundary, action, worker pass),
visited set over time-normalised state. Real components: `Pacer`/`TickSource`, `CreditGate`, `Mailbox`, `flip_completion_need`, `Requests`, `frame_gate`, `tick_live`; modelled around them: the relay's glue, the share, the worker pass, the ring.

| exploration | depth | letters | states | transitions |
|---|---|---|---|---|
| frames (flip imm/vsync, tick W/N, release, EV_FRAME, copy end exactly / -1 ns, guard edge, P) | 8 | 11 | 806 057 | 2 211 902 |
| time_and_silence (silence edge -1 ns/0/+1 ns, 10/200 ms, flap 55 and 40/60) | 6 | 14 | 516 297 | 1 017 114 |
| hostile_and_floods (skip/dup/stale seq, bursts of 10/100/1000 in ONE instant, sustained 10x/100x/1000x, reconnect) | 5 | 17 | 34 630 | 82 620 |
| long_stalls (2 s, 10 s then stale/dup/fresh) | 5 | 10 | 1 520 | 5 250 |
| sequence_wrap | 6 | 10 | 9 514 | 27 920 |
| three_frames (in-flight pipeline) | 10 | 5 | 26 943 | 70 950 |
| oracles: `CreditGate` 1 727 604 operations (36 letters, depth 4); `Requests` 402 233 (13 letters, depth 5) | | | | |

Invariants after every step: no immediate flip parked and none waits for a credit; no frame without a credit and exactly one per request (checked independently of `frame_gate`); one copy at a time and only on an unspent
credit; ≤ 2 frames in flight (literal 2) and a free slot always; no double tick inside the guard, the vblank +1 and never stalled more than 50 ms + P + guard; the buffer-reuse rule; request numbers strictly increasing across the wrap;
a hand-back after a fallback needs ≥ 2 events; counter conservation; `NO_CHANGE` only at a later tick, never with a frame ready, never sooner than the answer gap; a ready frame is never stuck behind a silent broker.

**Harness (B): the seeded soak** (`lockstep_soak`, 22 tests + one `#[ignore]` long soak; real `Engine` for every flip and vblank, real `Pacer`/`CreditGate`/`Mailbox`, the real `Relay` over a real `FrameRing` and `TickShare`, a scripted adversarial fake
broker that validates every command; injected clock, replayable: `KF_SOAK_SEED=<n>`; a failure prints scenario, seed, time, the invariant and the last 192 events). 50 seeds (0..49) per scenario, **≥ 100 000 flips per scenario**, about 10 s wall for the file:

| scenario | flips | note |
|---|---|---|
| 1 guest faster than refresh, immediate | 100 000 | 362–856 flips/s per seed while the display ticks at 60 |
| 2 two flips per frame (imm + vsync) | 100 000 | ≈ 120 flips/s, vsync-bound |
| 3 broker steady at refresh | 100 000 | jittered, exact counters |
| 4 floods 10x/100x/1000x, with/without WANT, steady and burst-32, other kinds interleaved | 100 000 + 6 constant combinations | see flood table |
| 5 early / duplicate / stale / reordered / dropped / backwards | 100 000 | stale counted exactly |
| 6 silence 49–1000 ms and return, copies 6–20 ms in flight | 189 777 | stale burst after |
| 7 headless / broker never connects | bit-identical to phase 1's `pacing_model::run` over 18 configurations | flips, presents, tearing, copies, ticks, skipped/s, worst flip, handoffs all `==` (f64) |
| 8 crash / socket full / SIGSTOP + down | 255 463 | ≥ 5 connections per seed |
| 9 hostile framing (reserved bits, seq 0, refresh < 1 ms, EV_TICK without opt-in, fd on the stream) | 194 664 | 600 violations sent, 600 connections dropped, the guest never stopped |
| adversarial scheduler (everything at once) | 100 000 | 354 connections, 528 fallbacks, 65 285 stale ticks, 111 359 coalesced credits |
| stalls 1–8 (below) | 100 000–333 893 each | |
| **long soak** (`#[ignore]`) | **10 000 000** over 50 seeds (200 000 each), 109 938 simulated s, 139 316 863 steps | **PASSED** on the final code: 32 827 connections, 54 836 fallbacks, 7.0 M stale ticks, 11.6 M coalesced credits, 28.8 M budget hits, 197 175 `NO_CHANGE`, 332 s wall |

**Floods (explicit).** Model/soak, 50 seeds × 12 s of simulated time per combination, the relay loop modelled busy (one turn per 5 ms): TICK at **10x** (600/s offered): 603 accepted/s, vblank 79.4–79.7/s;
**100x** (6000/s): 6003 accepted, vblank 80.8–81.1/s, 949 076 credits coalesced; **1000x** (60 000/s): 17.6 k accepted/s, 1.67–1.70 M socket-full drops at the broker side, vblank 99.4/s, 700 209 credits coalesced; bursts of 32 and 200
in the same runs; per-turn packets never above 64; at most one open request; counters exact (relay `ticks + stale` == pushed). Exhaustive model: a burst of 1000 ticks in ONE instant → 1 vblank, 1000 wants, 999 replaced, the worker took the latest
of 1000, 1 grant, 1 copy. Broker C side (virtual clock, 60 Hz): 600/6000/60 000 feedbacks → 100/118/120 ticks emitted, 500/5882/59 880 dropped as too fast, 98/116/118 coalesced in the TX queue; 20 bursts × 1000 at one instant → 20 ticks;
a peer that NEVER reads, 6 000 000 feedbacks → queue 3 packets, broker still answering; RELEASE/FOCUS/DEVICE/FRAME interleaved with ticks: all arrive, in order. Real threads over a real socket (7 tests, 1000 flips each, 6.1 s): flood bursts of
100/300/1000 per refresh, 54 876 ticks, max relay turn 64 packets; the socket buffer really fills when the broker never reads and no thread blocks. **The vblank under a flood is 79–99/s against a 60 Hz mode: the phase-1 guard (P/2) caps it at 2x;
"never faster" holds up to that cap, not beyond (limitation L2 below).** After the fix of finding F3 below the COPY rate never exceeds the vblank rate at any flood rate (tested).

**Stalls (explicit).** Durations 10, 49, 50, 51, 200 ms, 2 s, 10 s, in the exhaustive model and soak: (1) broker stops READING but still ticks: 770 frames dropped and counted, 2226 full sends, source stays the broker, vblank unaffected; a broker that
never reads from t = 0 or from 1 s: replay fails the attempt / frames owed, no thread blocks; (2) broker stops ticking but reads: fallback at exactly 50 ms (49 ms none), 835 unsolicited frames on the legacy credit, no frame without a credit while live;
(3) broker never RELEASEs a slot (300 ms–6 s): held ≤ 2, 15 219 `blocked`, 234 reclaimed after 1 s, the held slot never overwritten, no deadlock; (4) partial packet then nothing: dropped at 500–505 ms, loudly; one that completes in time is fine;
(5) resume with a burst of 24 stale ticks (WANT set) and stale releases: stale counted exactly, no backlog replay, handoff back once (handoffs exactly (1 + f, f)); (6) flapping 49/51, 55, 40/60, 48–52 ms: ≤ 6 source changes per 10 s run
(limit 14); the relay logs a flap four times and then every 256th; (7) SIGSTOP 10 s and a hung broker for 30 s: `silence_fallbacks == 1`, handoffs (1,1) or (2,1), log line `SILENT for 50 ms`, the guest runs on the clock indefinitely;
(8) stall during a credit wait with a copy 10–25 ms in flight and an immediate flip: it never waits longer than copy + wake + 2 ms. (A real SIGSTOP of a real broker was driven only against mutter: ticks stop, resume at once, §11.8.)

**Mutants** (each = one textual replacement, the named suites run, the file restored; `scripts/mutate/lockstep_mutants.py --suite must`, extras in `lockstep_extra_{a,b}.py`):
| mutant | suites that must kill it | final-head run, in that order |
|---|---|---|
| credit_check_removed | a b relay | KILLED KILLED KILLED |
| request_not_replaced (coalescing) | a b relay | KILLED KILLED KILLED |
| credit_not_consumed (gate) | a b unit | KILLED KILLED KILLED |
| worker_fallback_removed | a b unit | KILLED KILLED KILLED |
| relay_silence_removed | a b relay | KILLED KILLED KILLED |
| buffer_reuse_rule_removed | a b unit | KILLED KILLED KILLED |
| relay_sequence_check_removed | a b relay | KILLED KILLED KILLED |
| gate_sequence_check_removed | a b unit | KILLED KILLED KILLED |
| in_flight_bound_raised | a b relay | KILLED KILLED KILLED |
| double_tick_guard_removed | a b unit | KILLED KILLED KILLED |
| swallowed_credit_acts_at_once (guard defer removed) | a b unit | KILLED KILLED KILLED |
| carried_credit_not_released (the carry removed) | a b unit | KILLED **SURVIVED** KILLED |
| gate_acts_early | a b unit | KILLED KILLED KILLED |
| flap_damping_removed | a b unit | KILLED KILLED KILLED |
| no_change_at_the_opening_tick | a b relay | KILLED KILLED KILLED |
| read_deadline_removed | b threads relay | KILLED KILLED KILLED |
| per_turn_budget_removed | b threads relay | KILLED KILLED KILLED |
| send_blocks_on_full | b threads relay | not re-run on the final head (see the fork runs below) |
| stale_tick_counts_as_alive | a b relay | a **SURVIVED** (structural, F5: the liveness update lives in conn.rs which A re-implements), b KILLED, relay KILLED (after adding `a_stale_tick_does_not_keep_the_ticks_alive`; first run of the relay suite had it SURVIVE) |
| want_lost_on_coalesce (share) | b threads relay | not re-run on the final head (see the fork runs below) |
| a: NO_CHANGE ignores a ready frame | a | KILLED |
| a: request spent twice (spend does not take) | a | not re-run on the final head (see the fork runs below) |
| a: vblank number skips (+2) | a | KILLED |
| a: mailbox ignores the copy in flight | a | not re-run on the final head (see the fork runs below) |
| a: mailbox keeps its credit after a copy | a | not re-run on the final head (see the fork runs below) |
| a: silence is 500 ms | a | not re-run on the final head (see the fork runs below) |
| a: probation never grows (always 1 gap) | a | KILLED |
| a: NO_CHANGE not paced (answer gap 0) | a | not re-run on the final head (see the fork runs below) |
| held_slot_overwritten | b threads | KILLED KILLED |
| no_change_with_a_frame_waiting | b | not re-run on the final head (see the fork runs below) |
| request_spent_twice | b threads | KILLED KILLED |
| legacy_credit_used_while_live | b threads | not re-run on the final head (see the fork runs below) |
| commit_label_dropped | b threads | KILLED KILLED |
| reserved_tick_bits_accepted | b | not re-run on the final head (see the fork runs below) |
| tick_seq_zero_accepted | b | KILLED |

Honest accounting: the runs were stopped at the owner's request after the rows above; the rows marked "not re-run" were KILLED in the harness forks' own runs on the previous head (A: all 8 extras and every common mutant of its list except `stale_tick_counts_as_alive`;
B: 26 of 26 run, 25 killed after two harness fixes, `want_lost_on_coalesce`, `send_blocks_on_full`, `no_change_with_a_frame_waiting`, `legacy_credit_used_while_live`, `reserved_tick_bits_accepted` included) — BEFORE the G2 change of the gate and the F1/F2/F4 fixes; they were not re-run on the final head. Two rows on the final head are not kills:
**`carried_credit_not_released` survives suite b** (killed by A and by the unit tests; the soak's broker sends a WANT on nearly every tick, so a credit lost at a swallowed tick is replaced by the next tick's own and no soak invariant sees it — a soak invariant "every swallowed WANT is followed by a grant before the next WANT" would close it),
and **`stale_tick_counts_as_alive` survives suite a** (structural). Broker C mutants: `mutate_tick.py` 28/28 killed (each by ≥ 2 distinct failing checks), `mutate-tick-core` 14 killed (2 reported equivalent by design).


**Findings the harness made, and what was done** (each is a test that stays):
- **F1 (defect, FIXED).** A new broker connection restarts the tick numbers at 1; the worker's `CreditGate` kept the old last number and called every new tick stale (the lock step silently off for as long as the old connection had been up). Fix: `TickShare`
  epoch bumped at each tick-negotiated connection (`new_connection`), the worker resets the gate when it changes. Found independently by the exhaustive model (depth 3) and the soak (scenario 8); the test that pins the unfixed behaviour is kept as a proof the harness sees it.
- **F2 (FIXED).** An owed frame (ATTACH met a full socket) could be completed later with `COMMIT.seq 0` while ticks were live. Now `flush_owed` waits for an open request while live and the next request flushes it; asserted `owed_unsolicited == 0` in 50 seeds and in the 10 M run.
- **F3 (FIXED).** A tick flood made the COPY rate exceed the vblank rate (66–79 copies against 40 vblanks in 320 ms): a swallowed tick's credit was deferred to the guard edge and the next accepted tick brought its own. Now the swallowed tick's credit is carried to the NEXT ACCEPTED tick and coalesces into its credit
  (§11.4 G2 corrected above): copies ≤ vblanks at every flood rate.
- **F4 (PARTLY).** The relay's live/silent had no damping: 11 726 log lines in 50 runs with 51 ms gaps. The log is now rate-limited (4, then every 256th); the counters count every change; the relay's frame gate itself still toggles request/legacy per
  tick for such a broker (benign: each ready frame goes under exactly one of two bounded rules). The tick SOURCE is damped (flap damping above).
- **F5 (limits, not defects).** The strict "vblank never stalls more than 50 ms + P" has a counterexample: an event swallowed by the guard still proves the broker alive, so the next vblank came 71 ms after the last tick (bound held: 50 ms + P + guard = 74.7 ms).
  A strict bound would measure the silence from the last EMITTED tick (a behaviour change, not made; owner's call).

**Limitations, stated plainly (L1–L4).** L1 *What these harnesses cannot show:* real compositor timing (the fake broker's presentation delays are random numbers; occlusion, monitor change, fullscreen and direct scanout are not modelled in A/B; §11.8 has the headless-mutter
measurements), GPU copy cost and contention (the copy is a duration), what Windows really sends (the guest is a model: one flip outstanding, immediate or alternating), `display.rs` itself (the worker pass in A/B is a copy of its logic, guarded by a source scan;
the thread run has no `Engine`), the lock-free `TickShare` memory ordering under a real model checker, and depth beyond the stated bounds (the exploration is exhaustive over each alphabet to its depth, not a proof). L2 *Known cap:* the vblank under a flood is at most 2x the mode (the P/2 guard);
tightening it toward 1x would swallow legitimately early ticks of a jittery compositor. L3 *The 30 fps question* is untouched by this phase's evidence (see the top of §11). L4 The relay's silence rule is whole milliseconds.

### 11.8 What was MEASURED on a real compositor (headless; the C side)

`mutter 50.1 --headless`, virtual 1024x768 monitor, software renderer (`broker/test/wl_tick_probe.c`, not shipped; 4 s per row): the protocol text does NOT promise a frame callback for a commit without damage, so the claim "an empty commit still gets a callback" was tested.
Variant 0 = no attach, no damage; 1 = `damage_buffer(0,0,1,1)` on the buffer already attached; 2 = re-attach the same buffer with full damage. Driven by `presented`:

| variant | commits | frame_cb | presented | discarded | result |
|---|---|---|---|---|---|
| 0 empty commit | 32 | 32 | 17 | 14 | a callback **at once** (7 000+/s when each callback commits again) and `discarded` **at once**: no timestamp, not paced to the refresh. **Cannot be the pump** |
| 1 **1x1 damage** | 241 | 240 | 240 | 0 | exactly 60.00/s, mean 16.666 ms, max 17.3 ms, 0 discards. **This is what the broker commits** (least intrusive that works) |
| 2 re-attach + damage | 242 | 241 | 241 | 0 | 60.25/s |

Through the real broker over a real socket on mutter: 181 ticks in 3 s (60.3 Hz), spacing 16.67 ms, compositor stamps 16.666 ms apart, not `CLOCK_ONLY` (`clock_id` 1), no `tick_seq` gaps, ≤ 1 request per 100 ms with nobody answering, ticks wanting 100 % when
answered at once, `NO_CHANGE` → a request on nearly every tick. Minimising the window: `presented` stops after the window animation, later feedbacks are `discarded` at once → ticks stop → the VMM's 50 ms fallback; a 250 ms watchdog commit restarts the pump (4 commits/s while hidden);
SIGSTOP/SIGCONT of the compositor: ticks stop, resume at once. `weston --backend=headless`: clock `CLOCK_MONOTONIC_RAW` (stamped `CLOCK_ONLY` on receipt), a real presented interval of 25 ms (40 Hz) against a nominal 16.67 ms: the tick rate follows what the compositor really does. `sway`/wlroots headless:
like mutter, `refresh` 0 (unknown) handled (period word 0, divisor 1). **NOT verified:** a real GNOME session on a real GPU (direct scanout of a re-committed scanned-out buffer is UNMEASURED: `zero_copy` stayed 0 in every software run), occlusion by another window, a monitor move or refresh change
(the re-divide and re-advertise code is exercised with a 144 → 60 Hz switch on the test backend only), fullscreen, fractional scale, `wp_presentation` absent (the frame-callback fallback is compiled and CODE READ only), X11 (does not advertise the bit).

### 11.9 The owner's hardware checklist (nothing here has been run on a box)

Run: fullscreen Unigine Heaven D3D11 on the Windows guest, the prod-candidate flags, the broker built from this branch (`/data/claude-cursor5/`) on the owner's GNOME session.

1. **Handshake.** Broker log: `capabilities 0x...` with bit 13 (0x2000) set; relay log (qemu.log): `broker ticks: ON (...)`. Kayfabe status `broker[... ]` followed by `tick[on=1 live=1 ...]`. If `on=0`: the broker is old, X11, or has no presentation support: the clock alone, i.e. phase-1 behaviour.
2. **Who ticks.** The kf3 per-second line `h0[... src=broker ho=1/0]` while the Heaven window is visible and not occluded; the display line `tick source -> BROKER` once. `ho` second number and `tick[silence_fallbacks=N]` must stay 0 on a steady run; `tick[ticks_stale]`, `ts_backwards` 0.
3. **Counters expected in fullscreen** (to be compared with phase 1, `tick`/`copies` are the host-refresh-bound numbers): `tick` = the host refresh divided by `div` (60 on a 60 Hz output, 72 on a 144 Hz one), `copies` <= `tick`, `over=0`, `lock[... granted ~ copies, credits_coalesced small, mailbox_coalesced small, ticks_coalesced 0 or small]`, `tick[frames_on_credit ~ copies, no_change ...]`, `tick[req_blocked]` small. `tear`/`kms` = the guest's flip rate (the number this phase does NOT promise to change).
4. **The 30 fps question.** If fullscreen still reads ~30: collect `KF3_DISPLAY_TRACE=1` (bounded) as in section 9; this phase is not evidence either way.
5. **Occlude / minimise the broker window for 2 s, then restore:** the guest keeps running (Windows: no display-driver reset); display line `tick source -> FREE-RUNNING clock` then `-> BROKER`; relay lines `broker ticks: SILENT ...` then `LIVE`. Frames continue at the legacy 100 ms backstop while hidden.
6. **SIGSTOP the broker for 5 s and SIGCONT:** same, no guest reset; after SIGCONT the stale burst is dropped (`tick[ticks_stale]` may rise by the burst, never a backlog replay).
7. **Monitor move / refresh change** (60 <-> 144/165 Hz): broker log `host output refresh ...`, new `EV_SURFACE` mode rate = host/div; the guest re-modes once; `ho` may step once (the retime), not per tick.
8. **If the picture stalls:** look at (a) `tick[live=0]` with `src=free`: the broker's ticks stopped (its window hidden/occluded, compositor not committing): frames then go every 100 ms; (b) `tick[req_blocked]` rising with `live=1` and `frames_on_credit` flat: requests are not opening (the broker is not receiving or not presenting: its log line `present_timeouts`); (c) `tick[partial_timeouts]`; (d) `kf3: broker:` lines `PROTOCOL VIOLATION`. Restart only the broker (`/data/claude-cursor5/` binary), never the VM; the relay reconnects by itself.
9. **Regression** (CLAUDE.md): Linux guest with the broker (`broker_lane.sh`, `interactive.sh`): cursor and typing responsive, no `Flip event timeout`; headless (no broker): `src=free ho=0/0`, counters as phase 1.
