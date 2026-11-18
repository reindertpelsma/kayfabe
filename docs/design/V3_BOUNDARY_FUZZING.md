# V3 — boundary fuzzing: GPU-free property tests at the guest-to-host boundary

**STATUS: LIVE, 2026-11-18 (first pass: four entry points fuzzed, two findings; BF-1 fixed (span cap, PR stacked on this note's PR), BF-2 fixed in #495).**
Owner, 2026-11-18: finding a bug is worth more than shipping with bugs nobody knows about. This document lists the entry
points that take guest-controlled bytes, the properties each campaign checks, how to run the campaigns, and the results.
**Measured** means a test or campaign run on 2026-11-18 at `origin/next` `55194e22e` plus this branch. **Inferred** means read
from code and not run. It answers S1-83 ("v3's decoders have no fuzz or property target", `V3_SECURITY_MODEL.md` §8) for the
four targets below. Every other target is still open.

## 1. Entry points, ranked

Reach is who can choose the bytes: guest userspace (U), which is unprivileged, or guest kernel/root (K). Damage is the worst
outcome if a check is wrong. Rank is reach × damage. "Existing tests" means what was there before this pass.

| # | entry point | crate | what the guest controls | reach | damage if wrong | existing validation | existing tests | this pass |
|---|---|---|---|---|---|---|---|---|
| 1 | `served_policy(..).respond(cmd)`: every link of the served chain | kf-rm | `GSP_RM_CONTROL` id, `paramsSize`, flags, param bytes; `GSP_RM_ALLOC` class/params; FREE/DUP handles; any function code | U (guest RM forwards user controls and allocs) | drainer panic (device dead); reply over-read; emits to the channel/memory planes; host log volume | per-link size/flag/handle checks; `ReasmLimits` | per-control unit tests; `rmgraph` xorshift (graph only) | `tests/boundary_fuzz_chain.rs` |
| 2 | `GspFsm::service_command_queue` (element framing, checksum, `elemCount`, `writePtr`, large-RPC joining, held replies) | kf-gsp | the whole shared command queue: headers, elements, pointers | K | drainer panic or hang; OOB read of the region; held-reply growth | `MsgLen`, `max_elements`, `available_elements`, `MAX_HELD` | unit tests; the 2026-08-01 fuzz targets were retired at the v3 cutover | `boot.rs` `hostile_queue` |
| 3 | `Engine::step` / `vblank` / `cursor_write` / `alloc` (display pushbuffers) | kf-disp | every pushbuffer word, `PUT`, channel instance and offset, cursor PIO | K | display-thread panic; unbounded effects; loop on a JUMP | `pushbuf::decode` bounds, the class's offset widths | lock-step soak (pacing, not hostile words) | `engine/boundary_fuzz.rs` |
| 4 | `tmode::bind` / `resolve_spans` (Translated CE launches) | kf-chan | launch registers (pitch, line count, offsets), and the rows behind them through its own page tables | K | operand outside the window (host memory); work or allocation proportional to the footprint | `TWindows`, `MAX_PIECES`, `COPY_SLICE_BYTES` (single-line only) | `tests/tmode.rs` random streams (window and allowlist properties) | `tests/boundary_bind_cost.rs`: **BF-1** |
| 5 | doorbell token writes | kf-trap | any token word | U | wrong channel serviced | design: what a token can reach (§2.1) | `doorbell_contract.rs` | not repeated |
| 6 | GPFIFO entries (`ring.rs`, `MAX_SEGMENT_BYTES`) | kf-chan | GP entries, segment lengths | K | over-allocation | `MAX_SEGMENT_BYTES` check before the allocation | `tests/ring.rs` | inferred bounded (read only) |
| 7 | kf-qemu guest-RAM readers (USERD, user rings, display pushbuffer copy) | kf-qemu | GPAs, lengths | K/U | OOB guest-RAM read | `block_for` range checks | unit tests | **not GPU-free as a whole**; see §5 |

## 2. Properties (each campaign checks them after every call)

- **No panic.** The call runs under `catch_unwind`; a panic is reported with its seed and step.
- **Bounded time.** Each call takes less than 2 s, a generous figure for debug builds on shared runners.
- **Bounded output.** kf-rm: reply body ≤ 1 MiB and ≤ 16 statements to the channel or memory plane per command (a statement is
  an authored host act). kf-disp: ≤ 4096 effects plus GETs per call. kf-gsp: `held_len() ≤ MAX_HELD`.
- **Determinism.** A replay of the same seed gives a byte-identical transcript (replies, effects, or the device snapshot plus RAM).
- **Structure-aware inputs.** Headers that look valid with hostile counts. `paramsSize` honest, ±1, 0, `u32::MAX` or random.
  Truncated bodies, zero lengths, sizes at every boundary the discovery step found. kf-gsp mutations re-seal the checksum so
  they reach the decoders behind it. Display method offsets come from the class table at indices 0..9, which is one past the
  last head and window.
- **Coverage signal without instrumentation.** In campaign mode, kf-rm first probes every dictionary id and class (hex literals
  read from `kf-rm/src` and `kf-abi/src`) at every candidate size. It keeps the ids that some link claims and the sizes at
  which the answer changes. Measured 2026-11-18 by `campaign_served_chain` (`crates/kf-rm/tests/boundary_fuzz_chain.rs`): 119 of 716 ids claimed, 48 of 638 classes, 167 sized. Without discovery the fuzz had
  NV_OK answers for 53 distinct ids; with it, 119.

## 3. Commands

Each crate has a fast default test (well under 60 s) and a campaign behind `#[ignore]`. `KF_FUZZ_SECONDS=<s>` keeps running
batches until the deadline, so `KF_FUZZ_SECONDS=1800` is a 30-minute run per target.

```
cargo test -p kf-rm  --test boundary_fuzz_chain                 # 48 sessions x 120 commands
cargo test -p kf-gsp hostile_queue                              # 8 scripts x 80 actions
cargo test -p kf-disp --lib boundary_fuzz                       # 32 scripts x 150 calls
cargo test -p kf-chan --test boundary_bind_cost                 # BF-1 pin (+ the ignored invariant)

KF_FUZZ_SESSIONS=20000 [KF_FUZZ_SECONDS=1800] cargo test --release -p kf-rm --test boundary_fuzz_chain -- --ignored --nocapture
KF_FUZZ_GSP_SEEDS=3000 KF_FUZZ_GSP_ROUNDS=400 [KF_FUZZ_SECONDS=1800] cargo test --release -p kf-gsp campaign_hostile_queue -- --ignored --nocapture
KF_FUZZ_DISP_SEEDS=20000 [KF_FUZZ_SECONDS=1800] cargo test --release -p kf-disp --lib campaign_display_engine -- --ignored --nocapture
```

`KF_FUZZ_BASE=<seed>` moves the seed range. `KF_FUZZ_STATS=1` (kf-rm) prints the result histogram. A failure prints its seed
and step, and replaying that seed reproduces it.

## 4. Results (MEASURED, 2026-11-18, one x86_64 host, release builds)

| target | inputs | panics | over-bound | nondeterminism | notes |
|---|---|---|---|---|---|
| kf-rm served chain | 4.0 M commands (20 000 sessions × 200, Linux and declared-Windows arms, discovery on) + 0.8 M without discovery | 0 | 0 | 0 (every 16th session replayed) | in a 2 000-session stats run, 63 % of controls got a reply from a link and 37 % went unanswered to the ledger |
| kf-gsp command queue | 1.2 M guest actions (3 000 scripts × 400), each followed by a doorbell service | 0 | 0 | 0 (every 8th replayed) | one false alarm was in the test's own fake RAM (unwritten pages refused), fixed in the test |
| kf-disp engine | 8.0 M calls (20 000 scripts × 400) | 0 | 0 | 0 (every 8th replayed) | 93 % of calls produce effects (500-script sample: 140 137 of 150 082) |
| kf-chan tmode bind | targeted test | 0 | **BF-1** | — | see below |

**What a clean result does not show (INFERRED).** These are random and dictionary-guided searches, not proofs. No coverage
instrumentation was used. The kf-rm figure counts commands, not distinct paths. Links whose state needs a long valid
prologue (channel birth through `ChanSink`, the Windows software-runlist paths) are reached only shallowly.

### Findings

| id | crate | input class | impact | severity | test | status |
|---|---|---|---|---|---|---|
| BF-1 | kf-chan | a multi-line CE launch (guest kernel) whose operand covers many discontiguous rows | `bind` resolves one span per row of the whole footprint before it refuses (`MultiLineDiscontiguous` / `TooManyPieces`). Measured: 262 144 spans and 262 144 row lookups for a refused launch of 64 × `MAX_PIECES` lines, about 38 ms in a debug build. The work and the transient allocation grow with `LINE_COUNT` and are bounded only by the rows the guest has mapped. Pump thread, not a vCPU. | minor (guest-root DoS of its own channel's pump; host memory transient) | `tests/boundary_bind_cost.rs` (bounded spans; the cap sits exactly at `MAX_PIECES + 1`; a large bind that needs a walk is still waited) + `crates/kf-chan/mutants/bf1_mutants.py` (5 mutants, 2 or more named killers each) | **fixed** 2026-11-18: `resolve_spans` builds at most `MAX_SPANS = MAX_PIECES + 1` spans and then returns `Unresolved::TooManySpans`; `Rows::resolve` carries `Unresolved` (`Uncovered(at)`, the gap a walk may fill, waited on as before; `TooManySpans`, refused by name as `Refusal::TooManySpans`). Nothing a bind could accept is cut off: a side with more than `MAX_PIECES + 1` spans needs more than `MAX_PIECES` pieces. |
| BF-2 | kf-rm | any guest-repeatable refusal: GSS-legacy controls (`W343GSSCENSUS`), wrong size or FINN-serialized init-table controls (`W349REFUSE`), `GET_GSP_STATIC_INFO` of the wrong size, a FINN-serialized display control, `UPDATE_BAR_PDE`, video caps, `FIFO_GET_LATENCY_BUFFER_SIZE` (NON_PRIVILEGED), fn 1 | one host stderr line per guest command, unbounded (measured: 1 024 lines for 1 024 commands in each of 9 classes; 64 replays of a fixed hostile session wrote 6 492 lines against 113 for one) | minor (S1-86 class; host log/disk growth driven by unprivileged guest userspace, plus stderr latency on the drainer) | `crates/kf-rm/tests/guest_log_budget.rs` (red before the fix, green after) + `crates/kf-rm/mutants/logbudget_mutants.py` (9 mutants, 2 named killers each) | **fixed** in its own PR: every `eprintln!` on the served-chain respond path is `kf_arch::rate_eprintln!`. Lines are bounded per site (first 8, then powers of two) and each line carries the site's running count. Census counters are unchanged. |

## 5. Not GPU-free today, and the seam that would make it so (INFERRED)

- **kf-qemu guest-RAM readers** (`userstore.rs`, `display.rs` `pushbuffer()`, `userread.rs`): they read through
  `dev.ram.block_for(gpa, len)` on a realized device. The seam would be the `block_for` lookup as a trait over a `Vec`-backed
  RAM. `userstore.rs` already has a fake for its cache. The display copy path's bound comes from the engine's `PbLoc` (bounded,
  §1 row 3), so the remaining risk is in GPA arithmetic.
- **The register drainer end to end** (kf-qemu device + kf-gsp + kf-rm): `kf-harness` gates need a GPU. Rows 1 and 2 cover both
  halves separately. What is not covered is the drainer's own ordering.
- **Log sites outside kf-rm** (BF-2's class, counted 2026-11-18 by grep, not all guest-reachable): plain `eprintln!` sites
  number kf-qemu 364, kf-mem 59, kf-disp 28, kf-chan 8, kf-host 7, kf-cuda 5, kf-gsp 2. Many kf-qemu sites have their own
  per-site counters (e.g. `refusals_logged < 64`). The S1-86 row of `V3_SECURITY_MODEL.md` §7 stays open for them.

## 6. Sources

`docs/audits/2026-10-03-v3-stage1.md` (S1-83, S1-86); `docs/design/V3_SECURITY_MODEL.md` §7–§8; `docs/design/V3_REPLAY_TESTS.md`
(the red-then-pin pattern BF-1 follows); `crates/kf-arch/src/ratelog.rs` (the rate limit BF-2 applies).
