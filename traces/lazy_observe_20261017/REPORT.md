# Lazy-observe review — 2026-10-17

STATUS: STOPPED BEFORE HARDWARE. Scratch branch, never merge to master.
Branch: `codex/lazy-observe-20261017`, based on fetched gap head
`890c6643c6022df22a5bc02804ab1dad5449e9a7` (see git ancestry for full identity).
The pre-registration was committed and pushed at `b5549cf8c` before tests or
hardware work. No GPU workload, QEMU, display broker or Vast instance was started.

**Outcome:** 17/17 lazy-observe tests pass and 10/10 functional source mutants
are killed. The required full release suite is **308 passed / 1 failed**, so it
is not green. Review also found an inherited idle GP_PUT polling path that
conflicts with the owner's no-polling rule. Followed the instruction to stop and
report rather than change the design. There are no new FPS measurements.

## Stop findings

### 1. Required release suite fails on the inherited capacity contract

MEASURED: `nice -n 19 cargo test --release -j2 -p kf-chan` exits 101 in
`user_consumption_model::capacity_is_trusted_bounded_and_keeps_one_empty_slot`.
Continuing the suite with `--no-fail-fast` gives 308 passed, 1 failed, 0 ignored.
The isolated failing test also exits 101 after restoring the original gap-head
`user_lazy_model.rs`; the capacity source and test are unchanged from that head.

CODE READ: `crates/kf-chan/src/user_consumption.rs:17` sets scratch
`MAX_ENTRIES = 65536` (comment: was 512). `Consumption::new` accepts power-of-two
capacities through that limit. `crates/kf-chan/tests/user_consumption_model.rs:104`
still includes 1024 among capacities that must be refused. The assertion at
line 105 fails. This is a source/test contract mismatch; it is not a failed
lazy-observe predicate. Neither the capacity policy nor that test was changed.

### 2. The idle backstop polls GP_PUT and schedules work

CODE READ, not a hardware observation:

- `crates/kf-chan/src/user_loop.rs:30`: `BACKSTOP_MS = 50`.
- `crates/kf-chan/src/worker.rs:252`: an ordinary park timeout invokes
  `relay.peek(Peek::Backstop)`.
- `crates/kf-qemu/src/userpump.rs:2925`: Backstop bypasses the early return
  even when there is neither pending demand nor a waiting record.
- `userpump.rs:3001`: when the pump has no wait and no open segment, the
  backstop reads guest `USERD_GP_PUT`.
- `userpump.rs:3015`: PUT != cursor increments `backstop_put_ahead` and
  executes `return ring(self)`. The ring closure at line 2931 rings the
  internal token and signals the worker.

Thus the comment "Information only" does not describe an observation-only
path: it also schedules relay work. There is no trace/lifecycle flag guarding
this branch. This is an inherited GP_PUT poll, independently of A or B.
The stated "no GP_PUT polling" requirement is contradicted by this code path.
Its existence does **not** establish that a legitimate last-doorbell wake is
lost, or that a timer is necessary for correct guests; those remain unproven.
No redesign or removal was attempted under the explicit stop instruction.

## GPU-free review and tests

MEASURED on the trusted host, idle, under `flock -n -o
/tmp/kayfabe-fastguest.lock`, with nice 19 and two Cargo jobs. Source was copied
into `/data/codex-lazy-observe/src`; `/data/claude-gap` was read-only.
All 928 Cargo manifests/lockfile/Rust input files matched the local checkout,
and matched again after all source mutations were restored. Toolchain and
manifest digest: [input_verification.txt](input_verification.txt).

| Check | Result |
| --- | --- |
| Required release package suite | FAIL, exit 101 |
| Release suite with `--no-fail-fast` | 308 passed / 1 failed / 0 ignored |
| Lazy model alone after source restoration | 17 passed / 0 failed |
| Cached-PUT exhaustive exploration | 2,461 reachable states |
| Existing detailed worker-loop model | 37 passed |
| Existing SYNC_WAIT/parser rule tests | 9 passed |
| Functional source mutation campaign | 10 killed / 0 survived / 0 compile-error kills |

Full screened target counts: [tests_summary.txt](tests_summary.txt).
The original 12 lazy tests were retained. Five tests were added, and the
existing exhaustive model now permits a doorbell even when PUT has not changed
and the worker is asleep, including an empty-ring doorbell.

| Required property | Coverage and review |
| --- | --- |
| Cache unused at cursor == cached PUT | Existing direct predicate test, exhaustive model, advancing-PUT test |
| New generation invalidates, including lower PUT | Existing generation and explicit-invalidation test; model has up to two migrations |
| PUT advances before cache is exhausted; last doorbell drains all | Exhaustive guest Write/SetPut/Ring/Turn interleavings and directed last-doorbell test |
| Entries ordered, no loss/duplicates, wrap and I8 | New physical-slot identities across three ring laps; loaded PUT bounded by forward modular distance; exhaustive bounded drain from every reachable state |
| Idle/end_batch only after fresh PUT | Existing model rejects cached idle; new directed test forces fresh idle after consuming the last published identity |
| Stale GET cannot over-credit | Existing small exhaustive check plus 100,000 actions each on 4-, 8- and 128-slot rings, with exact receipt identities across repeated wraps |
| Full ring / credits < 64 force observation | New full 128-slot ring, every credit 0..63, floor boundary, wrapped refill and drain with periodic observation disabled |
| Waiting / open segment read every turn | Existing predicate check plus full-ring recovery assertions; separate mutants remove both reasons |
| Stale REFERENCE cannot release early; progress eventually releases | Existing monotonicity, unissued-word and bounded gate exploration; long lazy gate schedules below |
| N=1 matches eager behavior | New independent FIFO and absolute hardware-reference oracle, 200,000 randomized turns, GET and 16-bit REFERENCE wrap; retains original PUT test with A disabled |
| SYNC_WAIT ordering with laziness | New 48 randomized 2,048-entry schedules, intervals 4/16/64/never-periodic, checking every read against actual marker progress |

CODE READ: the scratch call site selects the cache with the current generation
before reading PUT; equality forces a fresh read before `end_batch`. Migration
updates the generation and rebases the cursor. B passes previous wait state,
private credits and open-segment state into `must_observe`. The two switches are
independent and default disabled; `OBSERVE_INTERVAL` remains the named constant
16 in the pure module. No relay/product source was changed for this review.

Limits: these are bounded models and randomized deterministic schedules, not
hardware evidence or a full concurrent proof of `Pump`. The lazy model treats a
relay turn as atomic; the separate 37-test worker model covers token/park races
but does not integrate these new lazy predicates. Conservative GET staleness
assumes ordered samples from this live private USERD; arbitrary old samples
replayed after a newer sample are not supported. Cached PUT assumes monotone
publication within a generation and no producer overwrite of outstanding slots;
"below PUT" means forward modular order, not numeric less-than across a wrap.
Gate proofs use REFERENCE, never GP_GET-as-processing. No claim of production
readiness follows from these predicate tests.

## Mutations

Reproducible runner: `scripts/bench/d3d11_gap/lazy_mutations.py CHECKOUT OUTPUT_DIR`,
under the fastguest lock on a disposable checkout. Each mutant runs the entire
17-test lazy target in release mode; compilation errors do not count as kills.
All files are restored between mutants and in the runner's `finally` block.
The original seven-mutation handoff table remains intact on the base branch;
this campaign reruns its functional rules and adds explicit invalidation,
open-segment observation and N=1 sensitivity.

[mutation_table.md](mutation_table.md) lists every mutation, exact failing tests,
and restored-source SHA256 digests. Final results:

| Broken rule | Failed lazy tests |
| --- | ---: |
| Ignore generation key | 2 |
| Allow cached idle at cursor == PUT | 4 |
| Omit explicit cache invalidation | 1 |
| Omit waiting observation | 5 |
| Omit open-segment observation | 2 |
| Omit low-credit observation | 2 |
| Over-count one private credit | 4 |
| Trust REFERENCE one marker early | 4 |
| Trust an unissued REFERENCE word | 1 |
| Skip observation at N=1 | 3 |

An initial trial changed `interval <= 1` to `interval == 0`; it correctly
survived because `turn_no % 1 == 0` still observes every turn. This equivalent
mutant was excluded, replaced with a functional N=1 bypass, and the entire
10-mutant campaign rerun. No surviving functional mutant was concealed.

## Four-arm D3D11 table

The committed [pre-registration](PREREGISTRATION.md) defines configuration,
interleaving, 260-second benchmarks and falsifiers. Correctness review stopped
the campaign before building the split binary or staging a guest.

| Arm | A | B | Run 1 FPS | Run 2 FPS | Carrier verdict |
| --- | --- | --- | --- | --- | --- |
| None | off | off | NOT RUN | NOT RUN | Control unavailable |
| A only | on | off | NOT RUN | NOT RUN | Untested; <= ~220 would falsify |
| B only | off | N=16 | NOT RUN | NOT RUN | Untested; <= ~220 would falsify |
| A+B | on | N=16 | NOT RUN | NOT RUN | Untested |

## N ladder and OpenGL

| Arm / API | N=4 | N=16 | N=64 |
| --- | --- | --- | --- |
| Conditional carrier ladder, D3D11 | NOT RUN | NOT RUN | NOT RUN |

| OpenGL check | Result |
| --- | --- |
| Best arm, fresh boot 1 | NOT RUN; no best arm selected |
| Best arm, fresh boot 2 | NOT RUN; no best arm selected |

Per-run sanity, screenshots/statistics, assert files, `/proc/QEMU/environ`,
guest nvlddmkm 153 / Display 4101 events and kernel-journal Xid/PID attribution
are **N/A: no VM run exists**. No dmesg-count delta was substituted for journal
sanity. Existing handoff FPS values (A+B ~299, fast-segment ~305, native
330/342, OpenGL scan-off 338–359) are inherited, not remeasured or independently
validated here. They cannot identify whether A or B carries the gain.

## Inferred, not measured

A and B preserve the tested pure invariants under the stated model assumptions.
Their individual performance contribution and best interval remain unknown.
The backstop could hide a wakeup defect in a hardware-only test; this is a risk
inferred from its scheduling path, not an observed lost wakeup. Resolving the
source/test capacity contract and the prohibited idle-PUT path is required
before resuming this campaign. The owner did not authorize a replacement design
in this task; no change was improvised.

## Not run and handoff

No split-arm binary build, guest overlay, Heaven staging, D3D11 arm, N ladder,
OpenGL arm, native Windows reference, new Vast rental or production integration.
No master merge; no edits to `kayfabe-*`, Linux, Translated or Emulated behavior.
All transient source mutations were confined to our disposable copy and restored.

Resume from this branch and report. Address the two stop findings explicitly,
then rerun the package suite before any GPU measurement. Review the old harness
before reuse: `hv2.sh` counts dmesg Xids rather than kernel-journal records,
`chain7.sh` lacks a health-stop gate and uses broad `pkill` patterns, and its
old `split.txt` lacks both second control and second A+B runs. These are code
review observations, not attempted runs. Use the committed eight-boot order and
capture journal records/PID maps and exact environments if the campaign resumes.

No ownership claim to instances 56302930 or 56313924; neither was queried or
modified. Owned instances: none. Host was idle at entry; final observed state
is recorded in [host_state.txt](host_state.txt), well before 06:30 CEST.
