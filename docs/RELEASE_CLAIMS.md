# Release claims ledger

**STATUS: DESIGN-ONLY, 2026-11-07.** The ledger, the validator and its tests exist; the jobs and stages named below are design intent, none is wired. Owner rule of 2026-11-07: every claim `README.md` makes about what works is
re-checked before a release, by an automated stage where one can exist and by a written manual checklist where
not. This page explains the ledger `docs/RELEASE_CLAIMS.tsv`, the validator `scripts/ci/release_claims.py`, the
stages each claim runs on, and the manual checklists. Nothing here is wired into required CI. The counts below
are COMPUTED from the ledger at `a9dee7b2c` (next, 2026-11-07) with `release_claims.py summary`; the status of
each row is INFERRED from the repository's own records, never from a new run: this lane ran no hardware.

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

## Today's ledger (COMPUTED, `a9dee7b2c`, 2026-11-07)

COMPUTED at `a9dee7b2c` on 2026-11-07, 133 rows: 77 auto, 29 manual, 27 wording. Status: 59 verified, 9 outdated, 27 contradicted, 34 never-verified,
4 unknown. Action: 53 keep, 47 update-wording, 32 add-test, 1 drop. Where: 14 CI GPU-free, 8 master gate middle,
66 release large, 18 owner hardware, 27 not verifiable.

The status `verified` (at `a9dee7b2c`, 2026-11-07) only says a record exists. At release every row is re-checked, and `release_claims.py release
--rev <tag revision>` fails until each row names that revision (see Proposed jobs).

### Top gaps

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
   the Windows NVDEC soak, the relay-forced suite, the no-patch check and the clean-clone build are not built or
   not wired.

### README sentences to reword or remove now

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
| `scripts/apps/apps_matrix.sh` | the fat guest image; 71 rows; failures re-run alone |
| `scripts/bench/windows/appmatrix/run_windows_apps.sh` | the Windows base image, an app disk (`build_appdisk.sh`, pinned `manifest.json`), a guest agent; no committed run of it exists |
| `scripts/bench/gfx_suite.sh`, `video_lane.sh`, `llm_parity.sh`, `dbfast_lane.sh`, `cuda_ladder.sh`, `perf_compare.sh` | a provisioned box; host and guest on the same box |
| `scripts/bench/display/desktop_smoke.sh`, `planes_oracle.sh`, `desktop_s3.sh` | Linux desktop guest; the broker lane (`broker_lane.sh`, `interactive.sh`) needs a host desktop session and cannot run on a headless rental |
| `scripts/bench/gpu_fault_containment.sh`, `gpu_vram_denial.sh`, `gpu_wedge_containment.sh` | a GPU with a driver; host-side probes, no guest |

SPECviewperf is not a claim stage: the README makes no SPECviewperf claim; its design is release tier only
(`docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md` section 6).

### Planned stages (the `NEW:` slugs; design intent, none built)

| slug | design | tier |
|---|---|---|
| `perf-report` | the README table is generated from the run database's `report.json` perf rows, each row citing revision and date; perf never gates | release large |
| `fault-containment` | two guests plus a host CUDA loop; one guest faults; the host and the other guest must keep correct results and forward progress | release large |
| `windows-nvdec-soak` | 15 to 20 min of 4K60 playback from a local file in Edge; at least 3 passing runs across dies confirm or retire "Edge can reset after 7 to 16 minutes" | release large |
| `windows-sleep-wake` | S3 sleep and wake of the Windows guest; expected FAIL today; the README sentence flips the day it passes; master gate only if it takes about 2 min | release large |
| `linux-display-desktop` | desktop and display planes; manual (MC-10) unless a Vast display lane exists | release large or owner hardware |
| `clean-clone-build` | lean clone of the tag, `cargo build --workspace --release --locked`, GPU-free `cargo test` | CI GPU-free |
| `no-patch-check` | lane scripts only run vendor installers; no default lane applies `docs/kernel_patches` or the UVM host patch | CI GPU-free |
| `release-asset-policy` | compares the release assets with the "no public binary" sentence | release large |
| `host-device-files-census` | the QEMU process opens only `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm`, `/dev/kvm` | master gate middle |
| `fast-suite-relay-forced` | `fast_suite.sh` with `KF3_DEBUG=1 KF3_DEBUG_FORCE_MIGRATABLE=1`; env passthrough to be confirmed | release large |
| `windows-gate-nvidia-smi-cuda-version`, `windows-setup-doc-vs-gate`, `app-gpu-attribution`, `relay-stall-pairs-linux`, `closed-module-refusal-check`, `license-spdx-check`, `external-link-check`, `ghsa-enabled-check` | each a small check named in its rows | per row |

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

## Proposed jobs (not wired)

1. **Advisory PR job** `release-claims` (non-required): `validate`, on PRs that touch `README.md`,
   `docs/RELEASE_CLAIMS.*` or `scripts/ci/release_claims*`. It makes a README edit visible with the missing
   ledger rows, without blocking.
2. **Release-tag check**: on a tag, `validate --check-git` then `release --rev <tag commit>`. It fails until
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
