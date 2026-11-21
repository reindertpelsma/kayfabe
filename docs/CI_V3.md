# CI scope after the v3 migration

**STATUS: LIVE, 2026-11-20 (the rootless payload run, section below, 2026-11-20; the `windows-guest-setup` job and its mutants 2026-11-20; the environment-read and comment-hygiene gates 2026-11-20; the standalone trust gate and its mutants and the `open-rows-mutants` job of the standalone union 2026-11-17; the plain-log ratchet was added 2026-11-18; the job layout, the concurrency rule and the timing audit 2026-11-17; the VA-apply profiling mutants and the profile-table self-test 2026-11-16; the host unmap-call cap mutants 2026-11-15; the rest is as of 2026-11-09).** This describes the CI repair, not an expansion of the
hardware support matrix or a claim of complete unsafe-code soundness.

## Job layout, concurrency and caches (2026-11-17)

**STATUS of this section: LIVE, 2026-11-17.** The layout and the rules are CODE READ (`.github/workflows/ci.yml`, tests named
below). Every number marked MEASURED names the run it comes from (GitHub job and step records, fetched with
`scripts/ci/ci_run_timings.py`). The expected times are INFERENCES from those records until the first runs on this
layout are compared (*Test plan*, last subsection of this section). Nothing here changes what a gate checks.

### Why: where the time and the queue went (MEASURED 2026-11-17)

- The `stable` job took 210.5 min (run 44088759756), 252.2 min (44084630157) and 216.0 min (44079124968); the median of its
  step "Vast broker mocks and workflow boundary" over the 17 recent successful runs was 148.6 min (min 136.2, max 162.7). In
  run 44088759756 that step was 141.0 of the 210.5 min: 99 min were `middle_apps_mutants.py` (118 mutants, one after another, about 54 s
  each), 13 min `evidence-mutants.mjs`, 7 min `report-mutants.mjs`, the rest the other Worker harnesses. The next three steps were
  "v3 dependency isolation and CI guard self-tests" 32.9 min (17.4 min of it `unittest discover -s scripts/ci`, 5660 tests; the rest
  the script mutants), "Windows support profile mutations" 19.1 min and "Host unmap-call cap mutations" 8.2 min. Everything else in `stable`
  (clippy, fmt, the boundary and claim gates: 5.2 min; the three smaller Rust mutant steps: 4.1 min) added up to 9.3 min. One runner ran all of it in sequence.
- The jobs wait for a runner far longer than they run. In 44088759756 `stable` queued 33.3 min, `stripped` 95.0 min, `platform` 157.8 min,
  `ptx` 173.0 min; the `CI` gate job, which needs every other job and runs for under a second, was created when `stable` finished and then queued
  139.6 min for a runner, so the run took 383.5 min from first job created to gate completed. At 20:25 UTC on 2026-11-17 there were 38 queued and 13 running
  `CI` runs, and 32 of those 51 belonged to a branch that had a newer run (one branch, `claude/open-ffmpeg-dcgm-20261117`, had 17).
  The runner pool is the bottleneck, so the first fix is no work for commits nobody will merge, and the second is a run that is as long as its longest job.
- In 8 sampled runs that ended `cancelled`, the `CI` job (`if: always()`) still ran and ended `failure`, for example runs 43807153613 and 43805699116.

### Concurrency

`ci.yml` has a `concurrency:` block (tested by `scripts/ci/test_ci_concurrency.py`, which evaluates both expressions for every kind of
trigger): a push to a branch other than `next` and `master`, and a `pull_request` run, share a group per branch or per pull request and
`cancel-in-progress` is true there, so the newer run cancels the one it supersedes, queued or running. Pushes to `next` and `master`, tags,
`schedule`, `workflow_dispatch` and `merge_group` get a group that is only their own run id and are never cancelled. The `CI` gate keeps
`if: always()`: a cancelled run's gate fails, which reds the superseded commit and nothing else, because a pull request is judged by the
checks of its head commit and the newer run decides. A gate that skipped itself after a cancel would report a required check as passed for a
run that never finished, so the gate is not allowed to. Other workflows keep their own settings (`vast-box-scripts.yml` and `gpu-vast.yml`
already cancel in progress; `gpu-vast.yml` is deliberately not touched).

### Jobs

`stable` keeps its name and the repository-wide gates: clippy and its debt ratchet, rustfmt, the full-tree Rust checks, the boundary, ABI,
unsafe, claim-ledger, GPU-UUID, private-reference, caller-contract and section gates, the dependency and ignore-ladder checks and `workflow_lint.py`.
Everything that was a long harness moved into a job of its own; each command is the same and in exactly one job
(`scripts/ci/test_ci_jobs.py` pins the list of 65). Expected run time = the MEASURED step times of run 44088759756 (2026-11-17) summed per job plus
about a minute of set-up; the two values that are not sums of measured steps are marked.

| job | what it runs | measured before (min) | expected (min) |
| --- | --- | --- | --- |
| `stable` | the gates above | 210.5 for the whole old job | about 8 |
| `worker-tests` | prettier, black, `node --test` of the Worker and the admin, tier mutants | about 6.5 | about 7 |
| `worker-mutants-a` | evidence-upload, provider-tolerance, denied-countries mutants | about 14.7 | about 15 |
| `worker-mutants-b` | report, release-*, rv2, driver-pins, simulation, profile, proprietary-image mutants | about 13.5 | about 14 |
| `middle-apps` (4 slices) | `middle_apps_mutants.py --shard I/4`: every fourth of the 118 mutants from the I-th | 99.0 (one worker) | about 27 each (INFERRED: a quarter of 99 plus a baseline run and set-up) |
| `mutants-a` | OIDC retry, propimage, clock rule, bandwidth, image size, kfci, instruction, and the mutants of the sharder and of the slicing | about 9 | about 10 |
| `mutants-b` | kfroot, box downloader, runner stages, doc steps, command-line reference, fault names, evidence and bench self-tests | about 14 | about 15 |
| `selftests` (3 shards) | every `test_*.py` of `scripts/ci`, a partition by module | 17.4 (one process) | about 6 each |
| `rust-mutants-a` | Windows support profile mutants | 19.1 | about 20 |
| `rust-mutants-b` | oracle gate, video classes, offer filter, unmap cap, VA profiling mutants | about 12.3 | about 13 |
| `windows-guest-setup` | `scripts/ci/windows_setup_check.ps1` on `windows-latest`: `scripts/windows/kayfabe-windows-setup.*` (Windows PowerShell 5.1) against the runner's real registry with a faked adapter class key: values and types, idempotency, an unelevated run refused (a fresh standard local user), cleanup. A gate-carrying job. The table, the docs and the logic against an in-memory registry are in `selftests` (`test_windows_guest_setup.py`) and `mutants-a` (`windows_guest_setup_mutants.py`, needs `pwsh`) | not measured yet | INFERRED: about 3 |
| `stripped` | unchanged: `cargo build` and `cargo test` on the stripped tree | 24.7 (44088759756), 44.5 (44084630157) | unchanged, now the longest job |

The run's wall time without queueing is therefore the `stripped` job (INFERRED: 25 to 45 min) plus the gate, instead of 210 min plus the gate. Runner
minutes of a whole run stay about where they were: 255.4 (MEASURED, run 44088759756 of 2026-11-17) and an INFERRED 270 to 280 afterwards, because every new job adds a set-up of
about a minute and `middle-apps` runs its baseline four times. The split buys wall time; what buys queue is the cancellation above (63 % of the live runs in the snapshot) and `stripped`
staying the longest job.

`middle-apps` is four slices on four runners, not several workers on one. MEASURED on a developer box on 2026-11-17: the harness with six workers on one machine, against the
sequential harness on nine of the mutants, gave one extra killer in each of two runs (`test_apps_pins.SetupSideRows.test_every_expected_setup_row_is_printed_ok_on_a_good_run`, then
`...test_a_silent_model_load_is_not_a_success`). The cause is shared state, not load: `scripts/apps/setup_side.sh` writes `/tmp/setup_hf.out` and reads it back, so two copies on
one machine disturb each other. A spurious failure counts as a killer and the rule is a minimum of two, so a pool of workers was not used. Fixing the script's fixed `/tmp` paths (or giving each
worker a private `/tmp`) would make a pool possible and would save about 75 runner minutes per run; it changes a script the boxes run, so it was not done here.

The `selftests` shards come from `scripts/ci/unittest_shard.py`: every test module is in exactly one shard (a partition by file size, deterministic, no
state shared between jobs), the exit status is the plain runner's, and an empty shard fails. `scripts/ci/test_unittest_shard.py` proves the partition
and the status, and `unittest_shard_mutants.py` needs two named killers for each way of getting either wrong. `middle_apps_mutants.py --shard I/N` runs
every N-th mutant from the I-th (default `1/1`, the whole list as before); the slices are tested the same way (`test_middle_apps_mutants_shards.py`, `middle_apps_shards_mutants.py`).
A harness that changes the checkout (the Rust mutants rewrite one file, run `cargo test` and restore it) runs in a job of its own, one after another.

### The gate

`gate` (`name: CI`, the required check) needs every job of the workflow except itself and the nightly `slow`; `scripts/ci/workflow_lint.py`
(rule `gate-needs`) fails the lint for a job it does not need, so a new shard cannot be left out. `stripped`, `stable` and every job split out of `stable`
must have succeeded: a skipped result is a failure for them (`if: false` on a shard would otherwise read as a pass). The other jobs keep the old rule (a
skip by their own `if:` is accepted). `scripts/ci/test_ci_gate.py` runs the gate script for every result of every job. A cancelled or failed need fails
the gate. `scripts/ci_gates.sh` reads the split jobs too (it expands the `selftests` matrix), so a local run still runs every harness.

### Adding a harness

Put its command in a `run:` step of the job whose time it fits (the table), or add a job (copy `mutants-a`, give it `timeout-minutes`) and put its id in the
`gate` job's `needs` and `MUST_RUN`, and in `scripts/ci_gates.sh` `SPLIT_JOBS` and `SPLIT` of `test_ci_jobs.py`; the lint and the tests fail until you do.
A new `scripts/ci/test_*.py` needs no edit: it falls into the lightest `selftests` shard. A harness can be sliced over machines like `middle_apps_mutants.py --shard I/N`; do not run several copies of tests on one machine
without checking them for fixed `/tmp` paths (see above), and never run a harness that edits the checkout beside another reader of it. To balance, read the per-step times of a run (`python3 scripts/ci/ci_run_timings.py --steps 25 RUN`) and
move the longest command of the slowest job to the fastest. Keep every job under the longest one (`stripped`) so the run is not made slower by its own shards.

### Caches and downloads

Measured in the same run: the Rust build is not the cost (`stripped` "Build" 0.5 min, "Clippy" 0.5 min with a 73 MB cache), and the three cache entries are keyed by the
lockfile, so they already hit; the new Rust jobs use `shared-key: stable` to start from the entry `stable` had. The downloads are small: `npm ci` for the Worker
(`jose` only) 0.3 s, `pip install black` about 3 s, `npx prettier` about 5 s, the toolchain about 11 s. Caching them would cost about what it saves and add an action to pin, so none
was added. Not changed, on purpose: path filters (the required check `CI` must always be reported, and a docs-only change still needs `stripped` to prove the stripped tree is
the same, which needs a result store, not a filter), the `pull_request` trigger (only the batch pull request to `master` uses it), the `stripped` test step (one `cargo test`, 23 to 45 min:
the next cut is splitting its heavy test binaries into legs, which needs the reached-count steps to read all of their logs) and the `Vast box scripts` workflow (a `gpu-free` job of 50 to
62 min on every push that touches `scripts/**`, not a required check). A gate on a lighter runner class is a further option, not done: whether the label exists for this account is not known from the repository.

### Test plan and rollback

Compare after the merge, with `python3 scripts/ci/ci_run_timings.py --steps 25 RUN...`: (1) three push runs on `next` after the merge against runs 44088759756 and 44084630157:
expected `stable` under 12 min, every other new job under 25 min, `stripped` unchanged, no job above the `stripped` time, the four `middle-apps` slices each under 35 min, the gate beginning within
its own queue wait of the last job; (2) a branch pushed twice in a minute: the first run must end `cancelled` and its `CI` job `failure`, the second run decides; (3) the killer lists of
`middle-apps` against a sequential run of the same tree (`python3 scripts/ci/middle_apps_mutants.py` with one worker, on the same tree): equal. Roll back with one
revert of the merge commit of the pull request (the old single `stable` job, no concurrency block); to keep the layout and drop only the cancel, revert the commit that added the `concurrency:` block.

## Shipped GPU programs: the `ptx` job (2026-11-13)

**STATUS of this section: LIVE, 2026-11-13.** Security review 3 F-5/F-8 (S1-68): before this job nothing
that runs tied the committed PTX to its source. CODE READ, and MEASURED locally in the job's pinned
container on 2026-11-13 (all steps pass on `next` at `dba284320`; the committed `kf_walk.ptx` and the three
generated display files reproduce byte for byte):

- `scripts/ci/walk_ptx_repro.py walk` rebuilds `cuda/walk/kf_walk.ptx` with `cuda/walk/make_ptx.py` and
  NVRTC 12.2.140 (the official `nvidia-cuda-nvrtc-cu12` wheel, sha256-pinned in
  `scripts/ci/nvrtc-12.2.140.requirements.txt`, downloaded in the job, never committed) and compares the bytes.
- `... display` rebuilds `kf_yuv.ptx`, `kf_color.ptx`, `kf_bl_pack.ptx` with their `make_*_ptx.sh` and the
  clang their headers record (Ubuntu 26.04 `clang-21 1:21.1.8-6ubuntu1`, hence the digest-pinned container);
  any other clang is refused, not compared.
- `... static` pins the two hand-written display PTX files (`kf_scanout.ptx`, `kf_sum.ptx`: no source exists,
  so they are **accepted unreviewed blobs** until one does) by sha256, and requires the constants the walk
  kernel and its Rust launcher share (`KF_SHWORDS`, frontier/scratch capacities, grid, block, warp) to agree.
- `make -C cuda/walk check-invariants check-cpu` runs the walk kernel's own source (its device half under
  `cuda/walk/kf_cpu_shim.h`) on a CPU over the committed corpus. The warp-parallel walk is not run there; it
  needs a GPU (`kf_tests`).
- The checker has unit tests (`test_walk_ptx_repro.py`) and mutants, each killed by two named tests.
- `scripts/ci/walk_kernel_mutants.py` mutates the kernel source's CPU-reachable checks (the VER2/VER3 4 KiB
  vetoes, review 3 F-1) and requires each mutant to fail at least two distinct `kf_cpu_walk` checks.
- `scripts/ci/va_apply_bounds_mutants.py` (Rust job, 2026-11-15) mutates the host unmap-call cap in `crates/kf-mem/src/batch.rs`
  (`UNMAP_CALL_MAX_ENTRIES`) and requires each mutant to fail at least two named `kf-mem` tests.
- `scripts/ci/util_flag_mutants.py` (Rust job, 2026-11-17) mutates GPU-utilisation reporting (`report-utilization`; `kf-abi` `utilization.rs`, the
  `kf-rm` link, sampler and RUSD publisher, the `kf-qemu` host read and the property's use sites) and requires each mutant to fail at least two named
  tests (`crates/kf-rm/tests/utilization_chain.rs`, `utilization_replay.rs`, the unit tests they sit beside, and `crates/kf-qemu/tests/utilization_use_sites.rs`,
  which pins its sites as source text). `crates/kf-abi/tests/utilization_layout_is_current.rs` recompiles `tools/util-probe/probe.c` against the two pinned
  oracles (`KF_REQUIRE_ORACLES=1` makes a missing tree a failure). Design and hostile-guest analysis: `docs/design/V3_UTILIZATION_FLAG.md`.
- `scripts/ci/windows_profile_mutants.py` (Rust job, 2026-11-15) mutates the Windows support profile's identity gates (`kf-rm` `winprofile.rs`, the
  chain and channel links, `kf-qemu` channel birth, the mirror-space start and the display colour path) and requires each mutant to fail at least two
  named tests. The tests are the proof that a Linux guest, an undeclared guest and a `windows-support=off` VM reach none of the profile
  (`crates/kf-rm/tests/windows_profile_chain.rs` and the unit tests it names). Review of PR #438 (2026-11-15) added mutants at the USE sites of the gates (where the
  gate is read, not only its truth table): behavioural tests with the real profile where a site is reachable GPU-free, and `crates/kf-qemu/tests/windows_use_sites.rs`
  (source pins, two independent tests) for the sites inside `ChanPlane::birth`, the drainer loop and `Device::realize`. **Honest count (delta review, 2026-11-15): 9 of the 23 use-site mutants are killed ONLY by those text-matching pins, which prove a line exists, not that it works; 5 more have a single behavioural killer.** The reviewer's own 15 re-spelled use-site hardwires (birth and pump, memory-plane and display construction, the display worker's `sdr()` reads, the drainer's caps sync, `HostRm::alloc_vaspace_at`, and one that makes the device ignore `windows-support=off`) mostly survive; that is a known coverage gap (`V3_WINDOWS_SUPPORT_PROFILE.md`, *Known coverage gap*), not a claim of full coverage.
- `scripts/ci/va_profile_mutants.py` (Rust job, 2026-11-16) mutates the observation-only VA-apply profiling
  (`docs/design/V3_VA_PROFILE.md`: `kf-mem` vaprof/vasmgr/batch, `kf-qemu` split delivery, `kf-cuda` ceiling geometry)
  and requires each mutant to fail at least two named tests; `scripts/bench/va_profile_table.py --self-test` runs in the
  `mutants-b` job.
- `static` also ties the VER3 PCF enumerants the kernel names (`KF_VER3_*`) to
  `crates/kf-chip/data/hwref-580.159.04.tsv`.

## Stripped tree (2026-11-09)

**STATUS of this section: LIVE, 2026-11-09.** Owner ruling (cited as section AQ of `OWNER_RULINGS.md` in the
lane brief; that section is not yet in this tree's `docs/OWNER_RULINGS.md`): the tests of
production code, and the tests a Vast box depends on, run on the tree that is left after everything
`.workerignore` matches is deleted. The site, the docs, the Worker, the repository-structure and the
claim checks keep their own filters and run on the full tree.

What exists (CODE READ unless marked):

- **One matcher.** `matches` in `scripts/ci/ignore_ladder.py`. `ignore_ladder.py strip --in-place`
  deletes the tracked files the commit's own `.workerignore` matches (read with `git show REV:.workerignore`,
  never from the working copy; never `.git`; symlinks are removed, not followed; every path is checked
  before the first deletion); `strip --dest DIR` exports the same tree. The in-place strip also marks the
  removed paths `skip-worktree`, so since 2026-11-10 the `-dirty` suffix of `kf3-bins/<rev8>[-dirty]` means
  "differs from the commit minus its own `.workerignore`" (without it every stripped box checkout was
  `-dirty` and the guest guard refused the binary, run 42712356582). A later `git reset --hard`, `stash` or
  `checkout` on the box would silently un-strip the tree (none runs today); `ignore_ladder.py strip-verify`,
  called by `merge_check.sh` right after the strip, fails with exit 2 if a stripped path is on disk, not
  flagged, or `git status` is not clean.
- **Fail closed (exit 3, nothing deleted):** a missing or non-UTF-8 file, a line outside the restricted
  syntax, a tree with nothing left, and an ignore set that would delete code: any non-prose file under
  `crates/ qemu/ cuda/ firmware/ tools/ .cargo/ third_party/ogkm third_party/ogkm-580`, `Cargo.toml`,
  `Cargo.lock`, `rust-toolchain.toml`, `clippy.toml`, `.gitmodules`, `.dockerignore`, `Dockerfile*`,
  `docker-compose*.yml`. `ignore_ladder.py check` reports the same refusals on every push.
- **Key cross-check.** `ignore_ladder.py keycheck` computes the `kf-tree-v2` key twice, in Python and with
  `ci/vast-broker/src/treekey.js`, and fails unless the bytes are equal; `ci/vast-broker/test/ladder-parity.test.js`
  compares the two matchers on the repository's three ignore files, on 400 seeded random pattern sets and on
  a set of lines only one side accepts (the strict side refuses them).
- **Where it runs.** The `stripped` job of `.github/workflows/ci.yml` (GPU-free `cargo build`, `cargo test
  --workspace`, the reached-count gates and `cargo clippy` with errors fatal) and the box's checkout in
  `scripts/bench/box/merge_check.sh`. The `stable` job keeps clippy with its warning-debt ratchet, rustfmt,
  and every structure, boundary, claim and documentation gate. **Not run on hardware:** the box strip has
  never run on a rented box; the first `gpu-vast` run of a batch PR is its test.
- **merge_check.sh options (2026-11-15, OWNER_RULINGS section AR).** `--no-fetch` (resolve the branch locally), `--tree PATH` (use a
  checkout as is: no fetch, no worktree, no strip, so it is no merge claim) and `--gates --kf3 --host --fast --census` (run only the
  named sections, in bar order). No option is the full bar as before. Tests: `scripts/ci/test_vast_merge_bar.py`.
- **Oracles must run.** `crates/kf-chip/tests/hwref_is_current.rs` skipped (a printed line that cargo counts
  as a pass) when `third_party/ogkm-580` or `gcc` was absent. With `KF_REQUIRE_ORACLES` set to anything but
  empty or `0` an absence is a failure. The stripped job sets it (`merge_check.sh` exports it too, but the box merge bar fetches no oracle and runs no cargo test except the opt-in KVM stage, so no oracle test runs on a box today); a developer may leave it
  unset and skip. `scripts/ci/fetch_oracles.sh` checks the oracle out shallow at the pinned commit and fails
  unless the commit recorded in the tree and the id fetched both equal the pin written in the script.
  The `stripped` job also fails unless the log carries `ORACLE-GATE: RAN hwref_is_current required=true`.
  The other oracle test that skips when its tree is absent, the fn-1 negotiation oracle
  (ogkm 595.84, not a submodule), now runs: `fetch_oracles.sh` checks the pinned tag out into `third_party/ogkm-595`, and under `KF_REQUIRE_ORACLES` an absent tree is a failure (its `FN1-ORACLE-GATE` lines go straight to stderr, so the job counts them). Two families still skip loudly everywhere, the full tree included, and are listed with that reason in `scripts/ci/full_tree_tests.tsv`: the nouveau citation tests (they need the 2 GB `third_party/linux`) and the `derive_classes` test (it needs an ogkm clone outside the repository).

**Why `gpu-vast.yml` does not read `.workerignore` (decision: left out).** Its two policy jobs run the
BASE workflow with a hard-coded skip list, so a pull request cannot change the skip set of its own run.
Reading the PR's own `.workerignore` would hand the author the skip set. Reading the BASE copy would be
safe against that but still weaker than today: `.workerignore` ignores `/.github` and `/ci/vast-broker`,
whose changes run the gate today (`trigger-policy.mjs` never skips `ci/vast-broker/`), so using it as the
skip set runs the gate less often. Making the result provably no weaker means intersecting it with the
existing list, which is the list. A test asserts the property that makes a skip sound instead: every
path the gate skips is also deleted by the strip (`ladder-parity.test.js`, "skip set inside strip set").

## CI repair of 2026-10-21 (branch `claude/ci-green-20261021`)

The Clippy step had been red on master since 2026-10-15, so every step after it had stopped running and
drift accumulated behind it. What changed, step by step:

- **Clippy errors (93 on the first local run, one more revealed once those were fixed).** `kf-qemu` denies `clippy::arithmetic_side_effects` and
  `clippy::indexing_slicing` for the guest-facing plane (`chan.rs` and the `user*.rs` modules it includes).
  Every flagged site now uses a checked or saturating form or `.get(..)` and fails closed; no `#[allow]` was added.
- **Format check.** 50 files were reformatted (`cargo fmt --all`); no token changed.
- **Clippy debt gate (`clippy-debt.json`).** 119 warning sites that predate this repair (new code since the last
  record) were recorded as reviewed migration debt: kf-disp 29, kf-qemu 26, kf-chan 24, kf-core 17, kf-abi 8,
  kf-broker 7, kf-harness 3, kf-host 2, kf-cuda 1, kf-mem 1, kf-trap 1. Largest classes: `items_after_test_module` 17,
  `dead_code` 17, `manual_is_multiple_of` 13, `collapsible_if` 13, `missing_docs` 11, `type_complexity` 7. Nine
  recorded sites no longer fire. No repair-introduced warning is among them.
- **Claim ledger (`claims-debt.json`).** 616 sites were recorded (607 `unattributed`, 7 `bare-hardware`,
  2 `conflated`): docs 327, traces 154, kf-qemu 51, kf-chan 40, kf-disp 11, kf-broker 8, kf-rm 7, kf-mem 5, kf-abi 4,
  and 9 elsewhere. They are prose written since the last record; none was rewritten.
- **Plain-log ratchet (`logs-debt.json`, 2026-11-18).** `python3 scripts/ci/debt.py logs` pins every plain
  `eprintln!` in the non-test code of the product crates (`LOG_CRATES` in `debt.py`) by file, enclosing fn and the
  macro up to its format string; a second identical site in one fn is numbered, so it is new too. A new plain site
  fails; guest-repeatable sites use `kf_arch::rate_eprintln!` (S1-86, `docs/design/V3_BOUNDARY_FUZZING.md`). 436 sites
  were recorded as the first baseline: kf-qemu 261, kf-rm 98, kf-mem 44, kf-chan 8, kf-linux-raw 8, kf-host 7,
  kf-cuda 5, kf-util 3, kf-broker 1, kf-gsp 1. Sites kept on purpose carry their reason in the baseline value
  (`LOG_KEEP`: the kf-host channel-birth line, which the merge-bar census reads in full).
- **Dependency isolation.** `kf-trap -> loom` and `kf-disp -> kf-broker, kf-linux-raw` are dev-dependencies of pure
  crates that the rule forbids. They are named, exactly and as dev-dependencies only, in `DEV_ONLY_EXCEPTIONS` of
  `scripts/ci/dependencies.py`. **Owner decision pending:** keep the exceptions, or move those tests to `kf-harness`.
- **ABI-quarantine gate.** It aborted on `SIGPIPE` under `pipefail` once a file's remainder outgrew the 64 KiB pipe
  buffer; the type lookup now tolerates an early-exiting `grep`.
- **Untagged ogkm citations.** `kf-abi/chanbudget.rs` is tagged `ogkm-580:` with corrected lines (the old
  `:317-334` was not where those cases are at 580.159.04); `kf-census` no longer spells the notation in identifiers
  (module `oracle`, field `ogkm_answer`); the 52 bare citations of `V3_VGPU_COMPAT_RESEARCH_20261013.md` are tagged
  `ogkm-595:`, the one tree that document names, without being re-read. The docs ratchet stays at 172.
- **Section numbers.** `V3_COMPLETION_AUDIT.md` finding headings are `F1..F12`.
- **Unsafe-surface gate.** A test message in `kf-mem/src/sim/tests.rs` contained the keyword; reworded.

## CI host prerequisites (2026-09-29)

The local unprivileged reproduction of GitHub's `execve/EACCES` failure produced
an AppArmor denial: the generic `unprivileged_userns` profile refused the embedded
memfd executable as a disconnected path. The identical test passed under a named
profile restricted to its executable; the probe profile was then removed.
`runner_namespaces.py` applies that prerequisite **only** to this GitHub checkout's
`target/**` executables and removes it after tests. It does not disable AppArmor,
change system sysctls, grant Linux capabilities or change product sandbox code.
All namespace/isolate containment assertions remain active.

Unprivileged `/proc/iomem` can redact RAM to `00000000-00000000`. The memory-type
test now separates its always-run fail-closed assertion from the host-RAM positive
control, which emits an explicit skipped marker if no complete RAM page is visible.
Both raw-OS crates additionally test redacted, valid and short-range parser inputs.

## What remains mandatory

`cargo test --workspace` still runs **all** retained and v3 workspace tests. No
failing test is allowlisted. The OS-free job forces KVM absent, with reached-count
guards; the scheduled/manual slow job exercises the longer tests. The aarch64
cross-check remains. Hardware acceptance is separate: exact-revision GPU gates,
QEMU build, fresh guest, and the 30-arm thin suite (see `CLAUDE.md`).

The repair keeps the legacy tests' safety assertions, updates paths for the
grader's move, adds v3 ABI reliance metadata to the capture guard, and teaches
the frozen doorbell model's generated-class parser to explicitly exclude optical
flow (which that old model never implemented). Unknown class kinds still fail;
the current `kf-chip` coverage is retained. An actual error-code collision in
`kf-host` was fixed. The grader's `--cuda-store-probe` feature check now names
its real `cuda-window` feature instead of a nonexistent local feature.

V3's QEMU wire header now has a compiled C/Rust differential test for size,
alignment, member offsets and ABI version. Foreign CUDA API layouts stay in the
named raw adapter; the CUDA walk wire structs retain their existing differential
proof. Archived QEMU headers are not live ABI declarations.

## Architecture gates

- Every `kf-*` member is checked for old-tree dependencies, including renamed,
  workspace-inherited, build, development and target-specific dependencies.
- Pure v3 algorithm/vocabulary crates have an explicit dependency boundary:
  they cannot import the host/OS adapters or an arbitrary external crate.
  Adapter comments may explain OS mechanisms; legacy comment-vocabulary guards
  are not misapplied to the new architecture.
- V3 unsafe quarantines are the already documented `kf-linux-raw`, `kf-cuda`
  and `kf-qemu`. The retained grader keeps its legacy raw-OS and CUDA adapters.
  Forbid inheritance, raw-pointer naming and relaxation-count gates cover both.
  Counts record the existing surfaces (127/43 and 73/73/36 respectively), not a
  new permission to put unsafe code in other crates.
- `kayfabe-doorbell` is a frozen prototype, not the v3 trap plane. The retained
  `kayfabe-chips` consumes its generated class table. That one legacy dependency
  remains; new consumers are rejected. Its pre-existing naming/lint exceptions
  are bound to SHA-256 hashes of **every Rust source and its manifest**. Added,
  removed or changed files fail that guard. Its tests still execute. This
  exception does not authorize new unsafe surfaces in the prototype.
- **Caller contracts (2026-11-14).** `scripts/ci/caller_contracts.py` requires every `CALLER-CONTRACT:`
  definition comment (owner rule, 2026-11-13) in `crates/ cuda/ qemu/hw/misc/kf3/ broker/ firmware/ tools/` to have a
  reviewed row in `scripts/ci/caller-contracts.tsv`; the list is exact (missing row, wrong count, stale row or empty
  reason fails). A citation in running prose is not a definition. Tests and mutants: `test_caller_contracts.py`,
  `caller_contracts_mutants.py`.

## Firmware (2026-10-03)

⊘ **CORRECTED 2026-10-03 (late, branch `v3-gop`; `OWNER_RULINGS.md` §K) — the paragraph below
describes a committed blob that no longer exists.** No compiled binary is committed. The driver kf3
serves is built from `firmware/kf-gop` by `crates/kf-gop-image/build.rs` in every workspace build
(`rust-toolchain.toml` lists `x86_64-unknown-uefi`; the `stable` and `slow` jobs install it, and a
missing target is a named build error). The `firmware` job now: fails if any tracked file is a compiled
binary (`.efi` or an `MZ` header, outside `archive/`); builds both flavours; checks that the driver
`kf-gop-image` embeds equals the standalone build byte for byte (the property one future signature rests
on — it replaced the committed-blob comparison); runs the host tests, Clippy with `-D warnings`, rustfmt,
kf3's PE check, the no-port-I/O check, the size record and the stand-in. The `aarch64` job's
workspace cross-check now embeds a real `aarch64-unknown-uefi` driver, and a step of its own builds the
driver for that target, lints it, checks it is an AArch64 boot-service driver, and checks that the two
x86-only pieces (`debugcon`, the test app) refuse to build there by name. The firmware ratchet entry is
`firmware/kf-gop:78` (the framebuffer's one volatile word store).

`firmware/kf-gop` is guest-side UEFI code (kf3's boot-display GOP, `docs/design/V3_DISPLAY.md` §4.11),
outside the cargo workspace and built by its own job, `firmware`: `rustup target add
x86_64-unknown-uefi --toolchain 1.99.0` (`rust-toolchain.toml` unchanged), both build flavours, the
safe library's host tests, Clippy with `-D warnings`, rustfmt, **a byte-for-byte comparison of the
committed `firmware/kf-gop/kf-gop.efi` (the blob `kf-oprom` embeds) with this source's build**, kf3's
PE acceptance check, a check that the release driver has no port I/O, the `.efi` size in the job
summary, and the local stand-in
(`scripts/display/gop_standin.sh`, gating). Its unsafe code is a named exception under the same rules,
checked in the `stable` job: `*_unsafe.rs` naming and the host-pointer gate apply unchanged; gate B and
the ratchet take path entries, and `firmware/kf-gop` is the one such entry; the ABI-quarantine gate's
firmware arm keeps every `#[repr(C)]` there inside a `*_unsafe.rs` file. `crates/kf-oprom` (the ROM
container) is an ordinary pure workspace crate.

## Visible migration debt, not warning-free code

The previously red workflow had hundreds of existing lint diagnostics and stale
numeric prose ceilings after the v3 copy/archive. CI now separates failed tests,
compiler errors and denied Clippy errors (always fatal) from existing warning
and prose debt. `scripts/ci/{clippy,claims}-debt.json` records diagnostic identity
by path, category and normalized source text. **Any new site fails**, even if
another old site was removed. An old item's line may move without buying a new
exception. The claim key covers the full paragraph, not just its display excerpt.

This is a deliberate change from `-D warnings` for the whole historical
workspace; it does **not** claim the recorded warnings were fixed or that old
unsupported prose became evidence. Initial Clippy diagnostics were inventoried
with Rust 1.98.0. Style, documentation and dead-code debt is still visible in the
JSON inventory. Denied undocumented unsafe sites were fixed, overlapping match
ranges made disjoint, and the unexpected feature condition corrected rather than
treated as proof that the warning was harmless. The existing documentation-only
unversioned-citation ceiling was reconciled to 172; source citations still fail
outside the hash-frozen prototype.

`--record` is a deliberate maintenance operation, **never run by CI**. Do not
refresh an inventory merely to silence a regression. Review the added keys and
explain them in a commit. Upgrading the compiler may require reviewing new lints.
The Python self-tests check that removed debt cannot buy unrelated new debt,
truncated/failed Clippy output cannot pass, and dependency aliases cannot bypass
the v3 boundary. The QEMU process-identity shell regression test also runs in CI.

Run `bash scripts/ci_gates.sh` for fast guards, or `--all` for the build/test/lint
steps too. The runner extracts the actual workflow (the `stable` job, the jobs split out of it on 2026-11-17 and the
non-destructive steps of `stripped`, where build and test live; it never runs the
strip, which deletes files, so `--all` builds and tests the full tree) and enforces
minimum step counts, so a shortened or malformed job cannot report a vacuous green.

## Environment and comment gates (2026-11-20)

**STATUS of this section: LIVE, 2026-11-20.** CODE READ: [`env_allowlist.py`](../scripts/ci/env_allowlist.py) and
[`comment_hygiene.py`](../scripts/ci/comment_hygiene.py) run in the `stable` job beside `trust_lint.py`; their mutants run in `mutants-b`
(two named killers per mutant, tests `test_env_allowlist.py` and `test_comment_hygiene.py`). Both are ignored by the ladder like the other CI-only scripts.

- **Environment reads.** No environment variable is required to boot a Windows or Linux guest in production; the product path reads the
  environment only inside an allowlist. The gate scans `qemu/hw/misc/kf3`, `broker`, `cuda/walk`, `firmware` and every `crates/kf-*`
  for `std::env::var`/`var_os`/`vars`/`vars_os`/`env!`/`option_env!` and `getenv`/`secure_getenv` on comment- and string-masked text. It leaves out
  `tests/`, `benches/`, `examples/`, test-named files and `#[cfg(test)]` items (not shipped code), the frozen `crates/kayfabe-*`, `archive/` and
  `third_party/`. Every name must be in [`env_registry.json`](../scripts/ci/env_registry.json) with a class (`debug`, `measurement`, `inject`,
  `stress`, `system`, `product`), the reading file, a one-line why and `changes_guest_served`. A read with a computed name is `<dynamic:file:fn>` and is registered like any
  other. The gate fails on an unregistered read, a stale entry, an entry whose file does not read the name, a malformed entry, and on an
  offender (class `product` or `changes_guest_served: true`) that is on neither the registry's `product_env_allowlist` nor
  [`env-allowlist-debt.json`](../scripts/ci/env-allowlist-debt.json). That baseline is a ratchet: it can only shrink, a name that stops being an
  offender must be deleted from it, and `--record-baseline` never adds a name. `env_allowlist.py report` prints the counts per class and the table of offenders
  with file, line and a proposal (`device-property`, `keep-as-debug` or `delete`).
- **Comment hygiene.** Code comments say what the code does and why; working notes, owner and process commentary and date-stamped annotations belong in
  `docs/design` and the handoff files. The gate extracts the comments (Rust, C, CUDA, JavaScript, Python `#`, shell, PowerShell) of the tracked sources outside
  `docs/`, `archive/`, `third_party/`, `traces/`, `site/` and `crates/kayfabe-*`, and flags a line that matches a pattern (owner rulings and dates,
  `OWNER_RULINGS`, agent and model names, `AGENTS.md`/`CLAUDE.md` pointers, coordinator notes, "as discussed", `Added 2026-..`, `(since 2026-..)`, a comment that opens
  with a date or a marker and a date, `Lane X` labels). Each pattern was counted on the tree first; words with a technical meaning (the owner of a lock, a guest agent, an RM
  session, a coordinator thread) are not matched. A finding is identified by file, pattern and normalised text, so moving a comment does not make it new.
  [`comment-hygiene-debt.json`](../scripts/ci/comment-hygiene-debt.json) is the shrink-only baseline. A finding is fixed by rewriting the comment; a
  baseline entry whose comment is gone must be deleted (`--record-baseline` only removes). No comment was rewritten when the gate landed.

## Standalone trust gate (2026-11-17)

CODE READ: [`trust_lint.py`](../scripts/ci/trust_lint.py) runs in the `stable` job beside `ignore_ladder.py check`; its mutants run in `mutants-b`. It reads
[`trust_manifest.json`](../scripts/ci/trust_manifest.json), the repository's list of what its scripts download and the anchor of each
(an upstream checksum fetched from upstream, or a pin in git of something upstream authored: a commit, a vendor checksum, a registry digest, a
source hash), and fails on an anchor that is "a sha256 in git of an artifact we build or host", an artifact without an anchor, a consumer that
does not go through [`kf_trust.py`](../scripts/ci/kf_trust.py), a pin that drifted away from the code that holds it, an unpinned Dockerfile
`FROM`, and any download primitive in `scripts/` or `tools/` that is neither routed nor on the manifest's `unrouted` ratchet (a count per file
and a reason; it can only go down; a finding is fixed, the list is never re-recorded to pass). `kf_trust_mutants.py` and
`trust_lint_mutants.py` weaken each rule once and need two named killers each. The gate and its mutants are ignored by the ladder like the other
CI-only scripts; `kf_trust.py` and the manifest are not (the box scripts use them). Design: `docs/design/V3_STANDALONE_TRUST.md`.

## QEMU release workflow (2026-11-21)

**STATUS of this section: LIVE as code, 2026-11-21; the workflow itself has NOT RUN on GitHub (it can only be dispatched once it is on `master`).** CODE READ:
[`release-qemu.yml`](../.github/workflows/release-qemu.yml), [`scripts/release/build_qemu.py`](../scripts/release/build_qemu.py) (the build kit), tests
[`test_release_qemu.py`](../scripts/ci/test_release_qemu.py) (selftests shards) and mutants [`release_qemu_mutants.py`](../scripts/ci/release_qemu_mutants.py)
(`mutants-b`, two named killers each).

- **What it builds.** The patched QEMU (the `kf3-gpu` device and the `kf-qemu` Rust archive linked into `qemu-system-x86_64`, x86_64-softmmu only), a
  corresponding-source tarball (kayfabe source, vendored crates, the unmodified signed QEMU tarball) and a manifest (QEMU version and source anchor, kayfabe
  revision, every overlay patch with its sha256, the exact configure line, the toolchain versions, the hash of every output).
- **Trust of the QEMU input.** The official tarball is accepted only if its qemu.org detached signature verifies against a signer fingerprint in
  `scripts/release/qemu-release.pin` (a private gpg keyring built from `qemu-release-keys.asc`; the signature must be a detached one) and the tarball must have the
  size and sha256 pinned in `scripts/bench/qemu.pin`; this holds for the downloaded tarball (also checked by `kf_trust.py`) and for the tarball inside a source tarball, whose
  signature is required. The signature file is fetched through `kf_trust.py` (manifest entry `qemu-source-signature`). A tarball or tree the user supplies is
  recorded as such (anchor `user`).
- **Jobs.** `gate` (master only: `ci/vast-broker/release_gate.py`, the same-SHA GPU-gate check `release-build.yml` uses), `build` (source tarball, reproducibility check,
  `SHA256SUMS`, one run artifact; it runs the third-party build code and holds `contents: read` only, no OIDC), `attest` (master only, needs `gate` and `build` to have
  succeeded; no checkout and no build code: it downloads the artifact, checks every file against the `SHA256SUMS` whose digest the `build` job published as a job output
  and refuses a file the list does not name, then attests the three files and the list) and `draft` (master only: refuses an existing tag or release, runs
  `gh attestation verify` on each of the four files with the signer workflow, the source ref and the hosted-runner pins, then `gh release create --draft` with exactly those
  four files). Off master only `build` runs, as a rehearsal: no OIDC, no attestation, no release. On master a skipped or cancelled `gate` stops `build`.
- **Reproducibility check.** The binary tarball is built from the source tarball and again from the checkout in another directory (both under `$RUNNER_TEMP`, outside the
  checkout); the two must be byte-identical (the manifests differ only in the key `source_tarball` and in `qemu.source.anchors`, which records how the QEMU tarball was
  checked). The kit fixes the clock (`SOURCE_DATE_EPOCH` = the commit time), the locale, the build paths (fixed `src` and `build` directory names, `-ffile-prefix-map`,
  `--remap-path-prefix`) and sorts and normalises the archive. Bit identity is measured for one toolchain on one machine, which is why the manifest records the toolchain;
  it is not claimed across machines.
- **Licence notices.** The binary tarball carries QEMU's `COPYING`, `COPYING.LIB`, `LICENSE` and `pc-bios/README` (the firmware licences) and the Rust toolchain's standard-library notices under `notices/`, besides kayfabe's own licence files.
- **Permissions.** Top level `permissions: {}`. `attest` has `id-token: write` and `attestations: write` and nothing else (the attestation is made in the run that builds and
  uploads, which is what the Worker verifies by the OIDC identity); `draft` has `contents: write` and `attestations: read`; `build` and `gate` have read permissions only. No
  secret but the default token. Every action is pinned by commit. The release is created with `--draft`; nothing in the workflow publishes it.
  `ci/vast-broker/workflow-guard.js` lists the `build` job of `release-build.yml` and the `attest` job of `release-qemu.yml` (exact file and job names) as the places
  `id-token: write` may appear; `ci/vast-broker/test/release-qemu-workflow.test.js` refuses it in every other job and file.
- **Verifying.** Every `gh attestation verify` of this workflow and of the manual pins `--signer-workflow reindertpelsma/kayfabe/.github/workflows/release-qemu.yml`,
  `--source-ref refs/heads/master` and `--deny-self-hosted-runners`: `--repo` alone accepts an attestation from any workflow on any branch.
- **What this does not change.** The standing blocker of a public binary is the owner's ruling (`OWNER_RULINGS.md` section BC item 1 and section P: the security gate and
  the licence review of the QEMU-linked tree). `LICENSE` exception 2 and `scripts/ci/qemu_license_closure.py` describe the licence state of that tree. The draft stays a
  draft until the owner rules.

## Rootless payload run (2026-11-20)

**STATUS of this section: LIVE as code, 2026-11-20; GPU-free tests and mutants only, NOT RUN ON A BOX.** The rootless run is a test that kayfabe's
QEMU needs no root (OWNER_RULINGS section AR item 6): every stage that starts a kayfabe QEMU runs as an unprivileged user with no capability, and
each root step the run still needed is a recorded `kfroot` step. The audit of every step is
[V3_KFROOT_AUDIT](design/V3_KFROOT_AUDIT.md); the mechanism is [V3_KFROOT_ROOTLESS](design/V3_KFROOT_ROOTLESS.md).

Flags and variables of `scripts/ci/vast_payload.py` (the payload is still started as root, as Vast starts it; these are not instruction fields and the
Worker does not send them, so a Worker run is unchanged and a rootless run is started by hand on the box):

| Flag | Variable | Meaning |
|---|---|---|
| `--rootless require` | `KF_PAYLOAD_ROOTLESS=require` | every stage but `provision` starts as the unprivileged user (`kfroot.py run`: every capability set empty, `NoNewPrivs=1`); default `off`; any other value is refused, never read as `off`; the flag beats the variable |
| `--rootless-user USER` | `KF_PAYLOAD_ROOTLESS_USER` | the account (default `ci`; created by the bootstrap; never `root`) |
| `--rootless-root-stage STAGE` (repeatable) | `KF_PAYLOAD_ROOTLESS_ROOT_STAGES=a,b` | the explicit approval of one stage the audit has not converted (`merge_bar`, `apps`, `propimage_apps`, `propimage_apps_b`, `matlab`, `specviewperf`); it runs as root, is recorded as the `VM-RUN` step `root_stage` and makes the verdict `ROOT_STAGES_APPROVED`; approving any other stage is refused |
| `--rootless-memlock` | `KF_PAYLOAD_ROOTLESS_MEMLOCK=1` | raise RLIMIT_MEMLOCK for the user (`limits.d` and `prlimit`); off by default because the audit found no need |

What it does, in order: `provision` runs as root and is one recorded `SETUP-ONCE` step. Before the first dropped stage, once: `kfroot.py bootstrap
--only-needed` (the account, `/dev/kvm`, a missing device node, a readable kernel log, the workspace, the lock file: only what is not already there),
the checkout, `/workspace/bench`, `/workspace/prov`, the evidence directory and `/workspace/apps` owned by the user, and the Rust toolchain copied
to the user's home. A windows stage first runs `windows_gate.sh --prepare` as root (packages and the swtpm AppArmor rule; its `kfroot` lines are
recorded through `KF_KFROOT_RECORD`). Later stages use `/workspace/prov` instead of `/root/prov`. An approved root stage runs as root and the trees go
back to the user after it.

Refusals, by name, before any stage starts (`KF_PAYLOAD_ROOTLESS_REFUSED <token>`, then `KF_PAYLOAD_FAIL suite=preflight`): `payload_not_root`,
`unclassified:<stage>` (a stage the audit does not know), `root_stage_not_approved:<stage>`, `approval_not_a_root_stage:<name>`,
`unmarked_root_op:<stage>:<file>:<line>:<op>` (a root operation in a dropped stage's entry script that is not a `kfroot` line: `kfroot.py
lint-stage <stage>` runs the same check), `missing_script:...`. `--dry-run --rootless require` prints them without touching the box, and
`KF_PAYLOAD_ROOTLESS mode=require ...` when the plan passes. A root step that fails at run time fails the stage as the suite `rootless_setup`.

The record is `rootless` in `kf-result.json` (read and validated by `scripts/ci/vast_report.py`, schema in `kfroot.py` `Record`): `verdict`
(`ROOTLESS`, `ROOT_STAGES_APPROVED`, or `VIOLATED` when a dropped stage asked for `kfroot` or a `kfroot` step is not in the audit), every stage with
its kind and status (`probed` or `not_yet_run`), every `kfroot` step with its stage, return code and source, and the refused calls. Read it with
`python3 -c "import json;print(json.dumps(json.load(open('/workspace/kf-result.json'))['rootless'],indent=1))"`. A `PASS` of the payload with
`ROOTLESS` as the verdict is the claim; with `ROOT_STAGES_APPROVED` the claim holds for the stages that were not approved only.

First run on a box (not done): [V3_KFROOT_AUDIT](design/V3_KFROOT_AUDIT.md) section 7 lists the smallest check for each open point; the manual run
is `python3 scripts/ci/vast_payload.py <sha> <nonce> --rootless require --only windows_gate` with the plan variables (`KF_MERGE_WINDOWS=1`, ...).
Tests `scripts/ci/test_kfroot_payload.py`, mutants `scripts/ci/kfroot_mutants.py` (two named killers each).

## Documentation freshness policy (2026-10-22)

CODE READ: [`doc_freshness.py`](../scripts/ci/doc_freshness.py) extends C1's
[`doc_status_check.py`](../scripts/ci/doc_status_check.py), introduced in PR #87.
It checks active Markdown under `docs/`, root `ARCHITECTURE.md` and root Markdown
README files. Each opens with a dated STATUS paragraph after an optional title:
LIVE, SUPERSEDED, ANSWERED, DESIGN-ONLY or RESEARCH. SUPERSEDED metadata links an
existing successor file in the same paragraph. Relative Markdown links, images,
reference links and HTML destinations must resolve inside the checkout. Comments,
fenced examples and inline code examples are excluded from link checking.
Markdown fragments resolve against common heading IDs (including duplicates and
setext headings) or explicit HTML anchors. External URLs and fragment semantics
for non-Markdown assets are outside this check.

CODE READ: a LIVE status older than `--max-age-days` (default 60 days) becomes
STALE-SUSPECT when it also has a missing literal reference. The checker searches
tracked source with `git grep`, reads crate package names from manifests, checks
file paths, and enumerates local/remote-tracking branch refs. It recognizes KF
family environment names, generic environment assignments and named environment
variables, and long command-line flags. Source grep excludes documentation,
archives, traces, third-party files, target directories and checker fixtures.
Short source filenames may resolve by suffix within active source; explicit root
paths must resolve at their stated location. Absolute filenames in code examples
describe host paths and are omitted from repository-reference diagnostics and
reports. Markdown destinations beginning with `/` retain repository-root link
semantics and must resolve. Computed names, external tools and
intentionally historical references need human review. A finding is a review
signal; it does not establish that a document is semantically obsolete.

CODE READ: `scripts/ci/doc-freshness-debt.json` records the initial legacy findings
by path, kind, diagnostic text and SHA-256 of the entire document. Each stale
reference has its own identity, so restoring one does not create new debt for
another. An edited document loses
its exemption and must pass all checks; removing one finding cannot buy a new
finding elsewhere. An unchanged document also fails if a newly missing source or
link target introduces a finding. The age calculation uses the current date;
`--today YYYY-MM-DD` makes a review snapshot reproducible. The explicit `--record`
operation is for reviewed migration debt and is never run in CI. Do not refresh
this baseline to conceal new or edited-document failures. The claims and Clippy
debt inventories are separate and remain unchanged by this workflow.

CODE READ: [the documentation workflow](../.github/workflows/doc-freshness.yml)
runs on documentation PRs, weekly and on manual dispatch. It publishes the full
per-file report as `DOC_FRESHNESS.md` in the job summary and a downloadable
artifact, including status, date, age, missing literal references, successor and
proposed owner action. Non-Markdown files are inventoried as ASSET; archive files
are inventoried as ARCHIVED. Neither receives a fabricated prose status. Report
recommendations are inferences, never automatic document edits or archive moves.

**Proposal, not adopted; this paragraph does not authorise any move.** When an entire
document no longer applies, move its contents to `archive/` and
leave a one-line SUPERSEDED pointer at the original path. Link the preserved
archive document, or the authoritative replacement when one exists. This keeps
incoming file links working. Verify the pointer and relative paths during the
move; freeze the archive after that transaction. Both `archive/` and the existing
`docs/archive/` are read-only afterward and excluded from active-document checks.
Partial corrections stay above the corrected text in the active parent document.
An active supersession notice should name the original pointer stub rather than
require a later edit to frozen archive text. Whole-document moves require the
owner's classification decision; the checker does not authorize them.

AGENTS-level rule. The owner approved the gate on 2026-10-27 ("good classification of old docs is
important as agents otherwise use it when it has long been superseded"); AGENTS.md carries the short
form (Doc hygiene). The published manual (`docs/howto/`, `docs/FAQ.md`) is inventoried as MANUAL and is
not dated, because the site versions it per release tag. The longer proposal, kept for the archive-and-
pointer policy, which is not yet adopted:

> Every active Markdown document opens with a dated allowed STATUS. Review
> source references before reaffirming LIVE. If the whole document no longer
> applies, move it to archive/ and leave a dated SUPERSEDED link at its old path;
> preserve history and incoming links. Archive contents are read-only after the
> move and excluded from active checks. Changed documents must clear the
> freshness ratchet; never re-record debt to bypass it.
