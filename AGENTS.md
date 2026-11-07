# AGENTS.md — kayfabe

A map, plus the rules that are not discoverable from the file they constrain. It is kept short on
purpose; the design lives in `docs/design/`. **STATUS: LIVE, 2026-11-07 (v3; layout, CI and merge flow refreshed).** ⊘ The pre-v3
version of this file described the `kayfabe-core`/`-fwd`/`-rt` hexagonal tree and
`scripts/run_full_suite.sh`. It is in git history before this date, and the tree it describes
is frozen (see *Layout*).

## Read first

0. ★ **Resuming work? `docs/STATUS_AND_HANDOFF.md`** — one page: the resume checklist, the newest
   dated UPDATEs, how work is run, and the map from old section numbers to `docs/history/`.
   **`docs/OWNER_RULINGS.md`** — every owner decision with its date (binding).
   **`scripts/bench/box/README.md`** — renting, using and retiring vast boxes (untrusted, no
   secrets, evidence in git, teardown by own ids only). **`docs/README.md`** — which documents are
   current and which are history.
1. `README.md` — what kayfabe is, and the status in one screen.
2. `ARCHITECTURE.md` — v3 on one page: planes, channel kinds, crates, rules.
3. `docs/STATUS_DETAIL.md` — every status claim, with its revision, box and evidence.
4. `docs/design/THE_ARCHITECTURE_v3.md` (the design), `THE_V3_PLAN.md` (the phased plan),
   `V3_BUILD.md` (execution rules: what may be copied from the old tree, and what never may),
   `THE_CONSTRAINTS.md` (the owner's constraints: no BAR1/BAR2 traps, no blocking on a vCPU, …).
5. `docs/design/V3_*.md` — one document per subsystem or result.
6. CI and the Worker: `docs/CI_V3.md` (what the gates are), `ci/vast-broker/README.md` (the Worker that
   rents Vast boxes; `DEPLOY.md` beside it says what is and is not deployed),
   `docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md` (release tiers and the ignore ladder),
   `docs/design/V3_PLATFORM_TESTING.md` (the platform status pages, `site/status/`).

## Layout

- `crates/kf-*` — **v3, the product.** The table is in `ARCHITECTURE.md`.
- `crates/kayfabe-*` — the **pre-v3 tree, frozen**: reference, plus the grader. `kayfabe-rm-ladder`
  (the 30-arm raw client used by `scripts/fastguest/` and the bare-metal suite) and its
  dependency closure must keep building. Do not add features there, and never make a `kf-*`
  crate depend on it. Code *may* be copied from it when the code's shape fits v3
  (`V3_BUILD.md`, *Rules*). 2026-10-31: the other pre-v3 crates (`core`, `fwd`, `rmrpc`, `rt`, `shell`,
  `crec`, `qemu-raw`, `vmm-*`, and `device`) are under `archive/crates/` and do not build; a citation of
  `crates/kayfabe-X` for one of them resolves to `archive/crates/kayfabe-X` (`archive/README.md`).
- `qemu/hw/misc/kf3/` — the C QOM device. `cuda/walk/` — the walk kernel (`.cu` plus committed
  PTX). `broker/` — the C display broker. `firmware/` — the boot-display GOP driver. `tools/` — small
  host tools and patches.
- `scripts/bench/`, `scripts/fastguest/`, `scripts/apps/` — provisioning, gates, lanes and the app
  matrix. `scripts/ci/` — the CI gates and ratchets (doc freshness, claim ledger, clippy debt, GPU
  UUIDs, private references, ignore ladder, Vast payload tests); `scripts/site/` feeds the site.
- `ci/` — the Cloudflare Worker side of CI (`ci/vast-broker`, `ci/vast-reaper`, `ci/vast-admin`,
  `ci/platform-api`). `site/` — the public site (`site/README.md`); only `docs/howto/` and
  `docs/FAQ.md` are published, the rest of `docs/` is internal (`docs/README.md`).
- `traces/` — committed evidence, one directory per run, each with a README that names the run, the
  revision and the date. **Files that tests or the Vast box payload read live under `traces/`** (the
  list is the SPARSE set in `ci/vast-broker/src/onstart.js` and the paths in `crates/*/data`); never
  move or rename one without updating its reader. A claim that cites `traces/<dir>/<file>` must name a
  file that exists in the tree or in its directory's `MANIFEST.tsv`: since 2026-11-07 the bulk files (PNG,
  large logs, tarballs) of 61 directories live in per-directory archives, not in git
  (`traces/EVIDENCE_ARCHIVE.md`: public URL, sha256, `scripts/evidence/fetch_evidence_archive.py`).
- `third_party/` — pinned reference sources (submodules; not built). `archive/` — frozen.

## Rules

- **Hostile guest.** Guest userspace and guest root are both untrusted. Host actions are
  **authored** from unprivileged RM verbs, never forwarded from guest bytes. A guest-reachable
  unbounded read or emit is a security bug.
- **Derive, never capture.** Per-die facts come from the host (unprivileged controls). Register
  offsets and class sets are generated from ogkm. Only family rows are maintained by hand. A
  captured per-die table is the defect that v3 exists to end. **All families are first-class.**
- **Only BAR0 writes trap; nothing blocks on a vCPU or under a lock that a vCPU takes.** Mapping
  runs on the VA-manager side, never in a trap.
- **No CPU executor for GPU work** and no forged completion for work that reached the GPU. Only
  emulated channels, which have no GPU work behind them, complete in the VMM.
- **Unsafe** code only in `kf-linux-raw`, `kf-qemu` and `kf-cuda`, and only in files named
  `*_unsafe.rs`.
- User-visible behaviour change updates docs/howto/ in the same PR; never put design notes into docs/howto/.
- **Doc hygiene** (paid for repeatedly):
  - An unmarked sentence is a claim of fact. Mark design intent and anything not measured.
  - Every design doc opens with a dated `STATUS` line: LIVE / SUPERSEDED / ANSWERED /
    DESIGN-ONLY / RESEARCH.
  - A correction is folded into its parent, **above** the text it corrects.
  - A supersession is recorded **in** the superseded text, not only in its successor.
  - Every active Markdown document opens with a dated STATUS; `scripts/ci/doc_freshness.py`
    enforces it and reports old LIVE documents with dead references (docs/CI_V3.md). A SUPERSEDED
    document links its successor. Fix a finding; never re-record the baseline to bypass it. The
    published manual (`docs/howto/`, `docs/FAQ.md`) is exempt.

## Branches and merging

- **Open PRs against `next`**, not `master`. `next` requires only `CI`. Master requires `CI` and
  `gpu-vast`, so the real-GPU gate runs once, on the batch PR `next` -> master (merge commit), on the
  exact final tree. Merge commits (or squash for docs/site PRs that cite no revision). We do not use rebase merges.
  Read master as one line per PR/batch with `git log --first-parent master`.
- After a batch lands, **sync `master` back into `next`** (merge `master` into `next`) so the next
  batch PR does not carry a divergence. Never push to `next` or `master` directly.
- **Branch names:** `claude/<topic>-YYYYMMDD` or `codex/<topic>-YYYYMMDD`, one worktree per
  parallel agent (a shared checkout contaminates commits; `git stash` is shared by all worktrees).
- **The ignore ladder:** `.gitignore` is a subset of `.workerignore`, which is a subset of
  `.dockerignore`; add an ignored path to all three, in that order (`scripts/ci/ignore_ladder.py`
  checks it; design in `docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md`).
- Break-glass: the owner may bypass the required `gpu-vast` check on one PR (rare, e.g. a broken gate
  runner); no other exceptions.

## Working rules — hypotheses, models, sessions

- **Correctness before cost.** A wrong hypothesis or a week spent on something irrelevant costs far more
  than tokens. Never downgrade a model for work whose wrong answer would redirect the effort.
- **Models by risk (updated 2026-10-10: Sonnet 5.5 for everything; Opus 5.5 ONLY when Sonnet fails or is stuck
  after a stated time-box, to save weekly usage; Haiku 5.5 is to be tried on mechanical lanes after the CLI
  upgrade).** Earlier ruling, owner, 2026-10-08: the gap between Sonnet 5.5 and Opus 5.5 is small, so **Sonnet 5.5
  is the default for agents and Opus 5.5 is the strongest tier** (what Fable was). Opus 5.5 runs hypothesis
  work, decoding unknown semantics, root-cause hunts, security review and anything that feeds an owner
  decision, and whatever a wrong answer would redirect for a week. Sonnet 5.5 runs the rest, including
  mechanical work with checkable output (evidence collection, formatting, running a script, grep sweeps,
  Windows iteration loops once the question is stated).
- **Every hypothesis states its falsifier before the run:** the result that would prove it wrong. Report the
  result either way. Keep *measured* and *inferred* apart; an inference is never written as evidence.
  (Precedent: "the 24 watchdog assertions never change, so the walls are irrelevant" was an inference. The
  journal holds exactly 24 records, so it could not change; the walls were the abort points.)
- **Time-box open-ended wall-clearing.** Batch the known walls per run, then stop and compare against the VFIO
  reference before continuing; never walk one wall per boot indefinitely.
- **Fresh session from `docs/STATUS_AND_HANDOFF.md`** instead of dragging a long chat: past ~150-200k
  tokens of context, resume in a new session. Filter big logs on the host; do not print them; read big
  files by range.

## Verification — what "green" means

- GPU-free: `cargo test -p kf-<crate>` for the `kf-*` crates.
- Hardware, strictly serial, one box: `scripts/bench/v3_gates.sh` (9/9) → `build_kf3.sh` →
  rebuild the raw client and the fast guest → `KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh
  <tag> 180` (30/30). For fat-guest claims, add the lane (`cuda_ladder.sh`, the app matrix,
  `llm_parity.sh`, `gfx_suite.sh`, `video_lane.sh`).
- **Bare metal first.** If bare metal passes and the guest fails, the defect is kayfabe's.
- **A bench claim carries its source revision.** `build_kf3.sh` installs each revision's binary
  under `kf3-bins/<rev>/`, and the lanes run that binary.
- GitHub CI is opportunistic, not the definition of green.

## Bench traps worth knowing before a run

- Vast boxes are nested KVM guests. A VM exit costs about 10–40× bare metal, so budgets are
  generous and ratios are per box.
- Rent with `vms_enabled=true` and the KVM image. Check `df -h /` right after the box comes up,
  because the disk request can be ignored.
- A killed job and a running job look the same if you only check for missing output. Write a
  start marker and an exit line, and check that the process is alive.
- The guest driver's `dmesg` is not in the serial log unless the harness captures it.
