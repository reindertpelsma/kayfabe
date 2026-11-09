# Release claims ledger

**STATUS: LIVE, 2026-11-08 for the ledger, validator, two GPU-free release checks and advisory PR workflow; DESIGN-ONLY for remaining `NEW:` stages and release-tag enforcement.** Windows apps and SPEC stage code exists, disabled pending the first Worker run; neither new stage is deployed. Owner rule of 2026-11-07: every claim `README.md` makes about what works is
re-checked before a release, by an automated stage where one can exist and by a written manual checklist where
not. This page explains `docs/RELEASE_CLAIMS.tsv`, `scripts/ci/release_claims.py`, the implemented checks and the manual
checklists. The advisory workflow is separate from required CI; release-tag enforcement is not implemented.
The historical audit below describes `a9dee7b2c`, not today's README. The DX1 section gives the current counts.
Hardware statuses come from named committed records; CODE READ rows describe repository facts. DX1 ran GPU-free
checks, not hardware.


## The contract

`README.md` is the contract. The ledger holds one row per verifiable claim. A README line is a claim line unless
it is blank, a heading, a table header or a table separator; there are 47 of them at `a9dee7b2c`. Each claim
line is bound to its rows by the hash of its whitespace-normalised text (`src_hash`). Consequences, enforced by
`release_claims.py validate`:

- A README edit without a ledger update FAILS: the new text is an uncovered line, and the old hash is a stale row.
- A row whose evidence pointer does not exist FAILS. A pointer is a path in the tree, or `git:<rev>:<path>` for
  a record that lives only on a branch (resolved with `--check-git`).
- A row whose status is `verified` must name a revision, a date and an evidence pointer (as in `a9dee7b2c`, 2026-11-07). Without all three the row FAILS.
- A `wording` row can never be `verified`: no check can establish it; it must be reworded, marked as a goal, or
  dropped.
- Local README links and anchors must resolve.

Pages other than the README (`site/content/works-today.md`, `docs/howto/requirements.md`, `docs/howto/goals.md`,
`docs/FAQ.md`, `docs/howto/windows-guest.md`, `docs/howto/troubleshooting.md`) are derived. Rows for them have
`src_hash` `-` and only exist where they repeat or contradict a README claim. The README is fixed first.

## Columns

| column | meaning |
|---|---|
| `claim_id` | stable slug |
| `source` | `file:line` of the claim |
| `src_hash` | 10-hex hash of the README line (README rows), else `-` |
| `claim` | the claim, shortened |
| `kind` | `auto` (an automated check can establish it), `manual` (a written procedure on named hardware or licensed software), `wording` (no check can; reword, mark as goal, or drop) |
| `verification` | the concrete check or procedure |
| `check_ref` | for `auto`: existing script paths, or `NEW:<slug>` for a stage that must be built; for `manual`: `MC-n` ids defined below; `-` for wording |
| `where` | `CI GPU-free`, `master gate middle` (the one-box PR gate), `release large` (release tag, several boxes, hours), `owner hardware`, or `not verifiable: reword` |
| `status` | `verified` (the row names a revision, a date and evidence, e.g. `0c515b3b3`, 2026-11-04), `outdated` (was true at the named revision, older than the current tree or superseded), `contradicted` (newer evidence disagrees; the row's note says what), `never-verified` (no run record in the tree), `unknown` |
| `evidence`, `rev`, `date` | pointer(s), revision(s) the run used, date of the run |
| `action` | `keep`, `update-wording`, `add-test`, `drop` |

## DX1 reconciliation, 2026-11-08

**CODE READ:** README claims now point to the revisions in the committed die sweep (TU106, TU116, GA104, GA106,
AD102, AD104, GB205 and GB206 at `0c515b3b3`, 2026-11-04; GA102 at `006d845a`, 2026-11-03), the intermittent GB202
record, and the Linux app matrix (70/70 host, 66/70 guest at `144557e43`, 2026-11-07). These are historical measurements,
not current-tree qualification. No hardware was run in DX1. Native SPEC 9/9 is omitted: its PR #289 trace directory is
absent in this checkout. The coordinator's native comparison for issue #305 is labelled separately in `V3_WINDOWS_GATE.md`.

The obsolete blanket parity, 65/61 counts, borrowed-page/Code-43 die summaries, owner-4070 blanket attribution and
unrecorded application stability claims are removed. Manual software claims remain pending MC-2 through MC-9; the README
makes no compatibility promise from them. Windows sleep/wake and Edge stability remain unfinished. The prior audit below
is retained as the dated explanation of what changed, not the status of the rewritten README.

**CODE READ:** all eight overlay entries are `manual: needs real display`, with the MC-14 template. The four Linux UVM
apps and `managed_t` are `known_unsupported`, reported as `unsupported (UVM)` under the owner's GR-hold merge condition.
Every structured report includes policy rows (NOT_RUN, never passing results), even with no app stage; historical raw
verdicts remain in notes and table columns. Supported-app completeness/failure and host-Xid rules remain in force.
CODE READ at `a01a0cb6cb9796b277b125bf58852b3e7cd271ad`, owner decision and RV2 correction 2026-11-09: LiveKernelEvent 193 is reported
as distinct reports in `known_live_dumps_193`, with raw entries recorded separately. The per-die ratchet remains
report-only by default (WARN above its seeded maximum); `KF_WINGATE_LKE_RATCHET=gate` enables ceiling enforcement,
while `KF_WINGATE_STRICT_LKE=1` gates on any non-stale 193 entry. Current seeds are one distinct report for AD102,
TU106 and GA102, with legacy entry maxima 2/3/4 and provenance retained. Unseeded dies give UNKNOWN. Unresolved identity/time gives UNKNOWN unless resolved fresh reports already prove
an overrun (default WARN; explicit gate FAIL). The local report retains count, resolved count, entries, unit, unresolved entries, maximum and ratchet verdict;
the current Worker sanitizer publishes only the existing `windows.live_dumps_193` alias. MEASURED in the supplied
record at `8d495e90`: retained 193-report headers contain code `0x193`, reason `0x815`; this does not establish cause
or harmlessness (issue #305; `design/V3_WINDOWS_GATE.md`, `traces/lke193_phase0_20261109/README.md`). DX1 ran no hardware.

**CODE READ (`scripts/ci/release_claims.py`, `docs/RELEASE_CLAIMS.tsv`):** the ledger uses the README's new hashes.
Rows for removed sentences are deleted instead of reused as positive evidence; derived-page rows name the corrected
claims. Navigation rows check local targets; remote site availability is unverified. Project/deployment statements
remain unverified manual MC-11 claims with the evidence template above/below. The validator and its pass rules are unchanged.

**COMPUTED after the DX1/R2 merge, 2026-11-08:** 55 rows cover 27 README claim lines: 43 auto and 12 manual;
46 verified at their named source/run revisions, 9 never-verified manual statements. No current claim is marked
contradicted or outdated. This is a structural/evidence audit, not a new hardware qualification. Policy behavior is
GPU-free tested at `34ab0a267`; the release-tag recheck requirement remains unchanged.

## Historical audit snapshot (COMPUTED, `a9dee7b2c`, 2026-11-07)

COMPUTED at `a9dee7b2c` on 2026-11-07, 133 rows: 77 auto, 29 manual, 27 wording. Status: 59 verified, 9 outdated, 27 contradicted, 34 never-verified,
4 unknown. Action: 53 keep, 47 update-wording, 32 add-test, 1 drop. Where: 14 CI GPU-free, 8 master gate middle,
66 release large, 18 owner hardware, 27 not verifiable.

The status `verified` (at `a9dee7b2c`, 2026-11-07) only says a record exists. At release every row is re-checked, and `release_claims.py release
--rev <tag revision>` fails until each row names that revision (see Proposed jobs).

### Gaps in the original README at `a9dee7b2c`

1. **The README contradicts newer records on dies.** GB206, GA104, TU116, AD102 and GB205 have
   `WINDOWS_GATE_PASS` at `0c515b3b3` on 2026-11-04 (`traces/die_coverage_20261104/`); the README, the site,
   `requirements.md`, `goals.md`, the FAQ, `windows-guest.md` and `troubleshooting.md` still say open, Code 43,
   enumeration only, or not documented. AD103 and AD106 were not run on Windows in that sweep; the README's
   "borrowed display page" has only older records.
2. **Two-VM sharing is no longer "not measured".** `traces/trio_vast_20261105/` records two guests and the host
   on one GPU (n=2, one RTX 4090 box): correct results, 0 resets, ratios recorded. The README and the site still
   say the fault effect and sharing are not measured. The hostile long-kernel experiment was not run.
3. **Claims with no record in the tree:** Windows multiplayer Minecraft, Minecraft Java and Bedrock separately,
   the 15-minute no-freeze session at `89fc8a222`, the relay figures "0 of 26 against 3 of 12", SolidWorks, OBS,
   Office, Heaven on Direct3D 9 as a pass, "no PCIe/DMA copy of frames", and the guest driver 590 on AD102.
4. **Wording defects found by recomputing:** the app-matrix seconds (1075 and 1193) cover 71 rows, 65 apps plus 6
   stream probes (`traces/v3_candidates/cand2_20261004/`, `9d82f259`); "outputs equal to bare metal" rests on a
   per-app known-answer verdict, and the committed comparison says it is not an all-app correctness claim;
   "all of the above on the owner's RTX 4070" includes numbers taken on rented RTX 3060 boxes; "Written in
   Rust" omits the C QEMU device; "the same set on the home RTX 4070 has not been run" is outdated, a non-nested
   4070 baseline exists (`traces/perf_baseline_6692e621_20261010/`).
5. **A release changes the truth of the status line.** "No public binary" and "master is the current working
   version" must be decided with the release (the `release-asset-policy` row).
6. **Most automated stages exist as scripts but are not release stages.** The trio, fault containment, sleep/wake,
   the Windows NVDEC soak and the relay-forced suite are not built or not wired. The no-patch and clean-clone checks now have scripts and fixture/mutation tests (see below).

### Original README sentences removed or reworded by DX1

| line | sentence | action |
|---|---|---|
| 3 | "Master is the current working version (relay fix `89fc8a222`, 2026-10-21)"; "the Blackwell display work ... live on branches" | update to the release tag; Blackwell display is on master |
| 7 | "Written in Rust" | "Rust, with a C QEMU device" |
| 7, 23 | "nothing in your NVIDIA driver is patched" | add "by default": an opt-in host UVM patch and bring-up guest patches exist |
| 12 | "There is no PCIe/DMA copy of frames for this windows desktop" | drop or measure; the site lists it as not measured |
| 35 | "app matrix, 65 apps" | "65 apps plus 6 probes (71 rows)" |
| 38 | "The same set on the home RTX 4070 ... has not been run yet" | name what ran on the 4070 (non-nested baseline, 2026-10-10) |
| 46 | "multiplayer works fine"; "no freezes in the latest session ... 15+ minutes"; "0 of 26 ... against 3 of 12" | drop, or write "verified manually on <hw> at <rev> on <date>" with a trace directory |
| 47 | "Direct3D 9 ... pass" | add the app-matrix row or drop D3D9 |
| 51, 54, 55 | SolidWorks, OBS and Office rows | manual wording, see below |
| 61 | "outputs equal to bare metal" | "pass their own correctness checks; byte-identical for the graphics and video lanes" |
| 63 | "all of the above on the owner's RTX 4070" | list what ran on the 4070 |
| 64 | "AD106 and AD103 with a borrowed display page", "GA104 only has the GPU", "GB206 ... still open (Code 43)", "(Linux guest driver 590)" | replace with the die coverage record of 2026-11-04 |
| 65 | "Per-die table: docs/STATUS_DETAIL.md" | link `docs/design/V3_FAMILY_COVERAGE.md` (STATUS_DETAIL calls itself history) |
| 68 | "what a guest-triggered GPU fault does ... is not yet measured" | "measured only as the suite's fault arms beside two other workloads, n=2" |

## Stages

Existing scripts a row points at (each exists in the tree at `a9dee7b2c`; the needs are read from the script
headers, not run here):

| script | what it needs |
|---|---|
| `scripts/bench/box/merge_check.sh` | one provisioned GPU box; gates, kf3 build, raw and thin suites, census; about 11 min on a Vast box (`traces/trio_vast_20261105/`) |
| `scripts/bench/box/windows_gate.sh` | the Windows media and an install (16 to 45 min, once per box), then the gate (968 s to 3692 s recorded) |
| `scripts/bench/box/trio_stage.sh` | DRAFT; after `windows_gate.sh` on the same box; about 20 min; not wired |
| `scripts/apps/apps_matrix.sh` | the fat guest image; 70 runnable apps plus one policy skip in the 2026-11-07 record; four UVM apps now known unsupported |
| `scripts/bench/windows/appmatrix/run_windows_apps.sh` | the Windows base image, an app disk (`build_appdisk.sh`, pinned `manifest.json`), guest transport; manual stage runs are recorded in `traces/winapps_20261107/`, not a Worker run |
| `scripts/bench/gfx_suite.sh`, `video_lane.sh`, `llm_parity.sh`, `dbfast_lane.sh`, `cuda_ladder.sh`, `perf_compare.sh` | a provisioned box; host and guest on the same box |
| `scripts/bench/display/desktop_smoke.sh`, `planes_oracle.sh`, `desktop_s3.sh` | Linux desktop guest; the broker lane (`broker_lane.sh`, `interactive.sh`) needs a host desktop session and cannot run on a headless rental |
| `scripts/bench/gpu_fault_containment.sh`, `gpu_vram_denial.sh`, `gpu_wedge_containment.sh` | a GPU with a driver; host-side probes, no guest |

The README makes no native SPECviewperf result claim: that trace directory is absent from this checkout. The SPEC stage code exists, disabled by default; its design is release tier only
(`docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md` section 6).

### Remaining planned stages (the `NEW:` slugs; design intent)


| slug | design | tier |
|---|---|---|
| `perf-report` | the README table is generated from the run database's `report.json` perf rows, each row citing revision and date; perf never gates | release large |
| `fault-containment` | two guests plus a host CUDA loop; one guest faults; the host and the other guest must keep correct results and forward progress | release large |
| `windows-nvdec-soak` | 15 to 20 min of 4K60 playback from a local file in Edge; at least 3 passing runs across dies confirm or retire "Edge can reset after 7 to 16 minutes" | release large |
| `windows-sleep-wake` | S3 sleep and wake of the Windows guest; expected FAIL today; the README sentence flips the day it passes; master gate only if it takes about 2 min | release large |
| `linux-display-desktop` | desktop and display planes; manual (MC-10) unless a Vast display lane exists | release large or owner hardware |
| `release-asset-policy` | compares the release assets with the "no public binary" sentence | release large |
| `host-device-files-census` | the QEMU process opens only `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm`, `/dev/kvm` | master gate middle |
| `fast-suite-relay-forced` | DESIGN-ONLY, unverified: `fast_suite.sh` with `KF3_DEBUG=1 KF3_DEBUG_FORCE_MIGRATABLE=1`; env passthrough is not verified | release large |
| `windows-gate-nvidia-smi-cuda-version`, `windows-setup-doc-vs-gate`, `app-gpu-attribution`, `relay-stall-pairs-linux`, `closed-module-refusal-check`, `license-spdx-check`, `external-link-check`, `ghsa-enabled-check` | each a small check named in its rows | per row |

## GPU-free release checks (implemented)

CODE READ, 2026-11-07: `scripts/ci/clean_clone_check.sh` shallow-clones committed sources
(`--source URL|DIR`, optionally `--ref BRANCH|TAG`) without submodules or evidence restoration.
It checks the README/manual build commands, required workspace/firmware locks, literal Rust
includes and exact references to archived trace members, then runs:

```sh
cargo build --workspace --release --locked -j2
KAYFABE_NO_KVM=1 cargo test --workspace --locked --no-fail-fast -j2
```

CODE READ: this is the documented Rust workspace build, including `kf-qemu`, not a QEMU C
build, driver install or guest boot. `README.md` and `docs/howto/building.md` document the
workspace command; the README also names the cold-clone record with 14 local test failures, so the command is not a
claim of passing qualification; root `CONTRIBUTING.md` contains the sparse-clone alternative. This check uses
a complete shallow tree. A fresh target subdirectory
is removed on success or a reported failure. The script runs at niceness 19, exports two jobs to nested cargo,
requires at least 6 GiB free on `/` before either cargo phase, and does not use image stubs.
On the shared development host, hold the existing global lock:

```sh
CARGO_TARGET_DIR=/workspace/R2-target nice -n 19 \
  flock /tmp/claude-0/kf-local-cargo.lock bash scripts/ci/clean_clone_check.sh
```

CODE READ: `python3 scripts/ci/no_patch_check.py` prints every patch artifact's classification
and explicit opt-in or historical-record pointer from `scripts/ci/no-patch-inventory.json`.
New artifacts, unreviewed application/reference scripts, changed review hashes, missing scope
pointers, missing stock entrypoints, and stock paths reaching a reviewed applicator fail.
The public manual must affirm that stock host and guest NVIDIA drivers are unmodified
“by default” and disclose “opt-in” exceptions. The README now scopes the vendor-driver statement to the stock path
and links to those opt-in requirements; the earlier absolute wording is no longer a current claim. Custom QEMU device integration is documented separately from optional
diagnostic QEMU patching in `docs/howto/installing.md`.

CODE READ, 2026-11-07, J1 integration review: the DirectX VRS patch is an **opt-in
test-program source patch**, applied by the separate offline
`scripts/bench/windows/appmatrix/build_release_bundle.ps1` in a temporary Microsoft
sample worktree. It does not modify the NVIDIA driver, QEMU or guest OS. The
inventory's reviewer note records that scope and points at the explicit builder;
both its PowerShell source and `release_bundle.py` (receipt/hash validation only)
are review-hash pinned. PowerShell `.ps1` files participate in the same application
detection and stock call graph as shell/Python scripts. Every newly discovered
patch still fails until its classification, scope note and opt-in pointer are reviewed.

LIMIT: the call graph follows literal script names/basenames and Python imports, including
shell heredocs. It is a static regression check, not a proof about dynamic dispatch, arbitrary
source rewrites or vendor installer contents. Constructed runtime file paths are exercised by
cargo tests; literal include/archived-input scanning is not a complete Rust parser. No check
fetches an evidence archive to turn a missing input into a pass.

```sh
python3 -m unittest scripts/ci/test_clean_clone_check.py scripts/ci/test_no_patch_check.py \
  scripts/ci/test_release_checks_workflows.py
python3 scripts/ci/release_checks_mutants.py
```

CODE READ: `.github/workflows/release-checks.yml` runs the fixture tests, mutation checks
and no-patch audit on relevant PRs to `next`/`master`. Its clean-clone job is **manual dispatch
only**. MEASURED, 2026-11-07, local source `fe0cd47e3`: the release build completed in 5m 53s,
but the complete cold build/test run exceeded 25 minutes during test compilation at niceness
19 with two jobs; the test run later exceeded three hours. The manual job has a four-hour
budget. This local timing does not predict GitHub runner timing; the final run record is under
`traces/release_checks_20261107/`.

## Claims that can never be automated on Vast

Licensed or accounted software (Minecraft, Office 365, SolidWorks, 3DMark and Steam) and claims bound to the
owner's hardware (the RTX 4070, display planes on a physical monitor, the photo). They are manual. The safe
wording for a manual claim is **"verified manually on <hardware> at <revision> on <date>"** with the trace
directory named, or the claim is dropped. A manual row stays `never-verified` until its evidence template
(next section) is committed.

### Evidence template

Copy into `traces/manual_<id>_<date>/README.md`:

```
STATUS: EVIDENCE, <date>. Checklist MC-<n>, claim ids: <ids>.
Hardware: <GPU, die, host driver, host kernel, CPU>.   Guest: <OS build, guest driver>.
Revision: <git rev of the kf3 binary, from build_kf3.sh>   Run id / VM id: <id>.
Steps: <numbered, exact>.   Result: <what the app itself reported; no score if SPEC_RESULTS forbids it>.
Host Xid delta: <n>.   Guest resets (Display 4101, nvlddmkm 153): <n>.   Duration: <min>.
Files: <screenshots, logs, relative paths>.
```

## Manual checklists

### MC-1 Screenshot and caption
README image and caption. Re-check every sentence of the caption against `traces/20261021_131847.jpg` or a new
photo: run banner, driver, apps, the Linux screen. The cabling sentence cannot be checked from a photo.

### MC-2 Minecraft Java and Bedrock
Per edition: launcher, world, 15 minutes beside Unigine Heaven, then a multiplayer server session. Record freezes.

### MC-3 Edge and YouTube 4K60
The exact YouTube video on owner hardware, hardware decode on, to the end; record any reset. The soak stage
covers the local-file case.

### MC-4 Blender on Windows against native
Cycles CUDA and OptiX on the guest and on a native (VFIO) reference of the same GPU; the large-tile denoise case.

### MC-5 SolidWorks
Install, the built-in Performance Test, sample projects. Student licence.

### MC-6 3DMark Steel Nomad on Steam
Full DX12 run to the end; scores are not committed (`docs/SPEC_RESULTS.md`).

### MC-7 OBS Studio
Record desktop, Heaven and Steel Nomad; keep the OBS log line for the NVENC encoder and the host encoder load.

### MC-8 Office 365
A defined session: Word, Excel, PowerPoint, Teams-style video call off. State what was opened.

### MC-9 Vendor installers and GPU attribution of GUI apps
Installer source and checksum per app; for each GUI app a record that it renders on the NVIDIA GPU (nvidia-smi
process list, the app's own GPU report).

### MC-10 Linux desktop and display planes on owner hardware
Overlay and YUV planes on a physical monitor on the RTX 4070; replaced by `linux-display-desktop` if a lane exists.

### MC-11 Project statements
One maintainer; no audit done or commissioned; no installer; security reporting works; the QEMU GPL sentence
still matches `LICENSE` (with the lawyer's review).

### MC-12 Statements about other products
VFIO, vGPU and nvkvm-pv rows: re-read the linked vendor pages and the nvkvm-pv docs; date the check.

### MC-13 Owner RTX 4070 claims
Which README claims hold on the owner's RTX 4070: the perf set (non-nested), the Linux rows, the Windows rows.

### MC-14 Windows YUV overlays on a real display

All eight entries (`overlay_yuv_probe`, hide, occlude, resize, move, recreate, fullscreen and soak) are manual.
Copy the general evidence template into `traces/manual_overlay_<date>/<app>.md`, adding physical monitor model and
connector, the same-GPU native result with DIRECT overlay support, app version/hash and exact command, per-second
presentation modes, transitions and latency, screenshots, duration, host Xid delta, guest reset counts and 193 counts.
The native Vast simulated-display record is SETUP/refused (code 5): no DIRECT overlay and no scenario frames. No such
real-monitor record is committed. The identical template is in `gate_profiles.json` for every overlay item.

MC-11 also covers the current README's no-binary, no-installer, audit/interface and deployment statements: record
the date and revision, checked release assets, maintainer confirmation, and the deployment/version record without
credentials. A statement supplied by the owner is still manual until that evidence is committed.

## Usage

```
python3 scripts/ci/release_claims.py validate [--check-git]   # structure, README coverage, pointers, links
python3 scripts/ci/release_claims.py summary
python3 scripts/ci/release_claims.py checklist --rev <rev>    # what is left to re-check, by stage
python3 scripts/ci/release_claims.py release --rev <rev>      # exit 1 until every row names <rev>
python3 scripts/ci/release_claims.py lines                    # README lines with hash and covering ids
python3 scripts/ci/release_claims.py renumber                 # fix line numbers after a README edit
python3 -m unittest scripts/ci/test_release_claims.py
python3 scripts/ci/release_claims_mutants.py
```

## CI wiring and remaining proposed jobs

1. **Implemented advisory PR job** `release-claims` in `.github/workflows/release-claims.yml`: `validate`, on PRs touching `README.md`, `docs/RELEASE_CLAIMS.*` or the validator/tests/workflow. CODE READ: job-level `continue-on-error: true`, read-only contents permission, SHA-pinned checkout, no dependency from the required `CI` gate. The workflow command is tested against valid, README-drift and missing-pointer fixtures; existing validator mutants also run.
2. **Release-tag check (design intent; not implemented)**: on a tag, `validate --check-git` then `release --rev <tag commit>`. It fails until
   every non-dropped row names the tag's revision, which forces the stages above (or the manual evidence) to run
   at that revision. The release tier of `V3_CI_TIERS_AND_IGNORE_LADDER.md` (largest, release tag, no retries,
   RETRY never passes) is the place the automated rows run.
3. Later, when the stages exist: each stage writes its row updates (revision, date, evidence pointer) as a
   committed ledger change, so the ledger and the run record move together.

## Limits

- A `verified` row (ledger at `a9dee7b2c`, 2026-11-07) says a record names that revision and date. It does not say the code at HEAD behaves the same.
- The hash covers a README line, not a sentence: a line holding several claims needs a row each, and a changed
  line forces all of them to be read again, by design.
- Rows for pages other than the README are not hash-bound and can go stale without a failure.
- The ledger was written from the tree. Records that exist only on branches are cited as `git:<rev>:<path>` and
  were read, not re-run.
