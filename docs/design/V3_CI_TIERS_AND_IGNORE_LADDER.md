# CI tiers, the admin start route, the tree-hash key and the ignore ladder

**STATUS: DESIGN-ONLY for the release workload contract, updated 2026-11-09.**
The tier intent below includes the owner's 2026-11-09 clarification. CODE READ: signed tier
selection, the ignore/tree-key controls and the admin routes now exist; the original
2026-11-05 sections below describe their design history, not a current implementation census.
Middle applications are implemented in the pending change described by
[V3_MIDDLE_REAL_APPS.md](V3_MIDDLE_REAL_APPS.md); matching Worker deployment and a combined
hardware run remain unverified. Large-tier SPEC plus real LLM execution remains design intent.
2026-11-12 (draft PR `claude/release-rehearsal-20261112`, not merged, not deployed): a large leg also
requests the middle apps pair ("large includes middle", owner), and the owner's Start may name one die
with `rehearsal: true` (a named scope that never clears a matrix blocker and ends at most
`REHEARSAL_PASS`); see [V3_RELEASE_REHEARSAL.md](V3_RELEASE_REHEARSAL.md). Everything in this paragraph
is GPU-free tested only.
Parent document: [V3_PLATFORM_TESTING.md](V3_PLATFORM_TESTING.md), whose §2 (tiers), §1 and §4
(fall-through key) this document corrects. Numbers are design targets unless a run is cited.

**Correction, 2026-11-09 (section 4 is built; the sentence "Nothing here is implemented" above is no
longer true of it):** `.workerignore`, `.dockerignore`, the ladder lint (`scripts/ci/ignore_ladder.py
check`) and the tree key (`ci/vast-broker/src/treekey.js`) exist. New with this correction: the strip
step (`ignore_ladder.py strip --in-place`, called by `scripts/bench/box/merge_check.sh` for the box
checkout and by the `stripped` job of `.github/workflows/ci.yml`), a cross-check that the Python matcher
and `treekey.js` compute the same key (`ignore_ladder.py keycheck`,
`ci/vast-broker/test/ladder-parity.test.js`), `KF_REQUIRE_ORACLES=1` with
`scripts/ci/fetch_oracles.sh`, and a strip floor that refuses to delete code. The strip has not run on
hardware yet. The gate-skip decision of `gpu-vast.yml` still uses its own hard-coded list on purpose;
see "Stripped tree" in `docs/CI_V3.md` for why it does not read `.workerignore`. Sections 1, 2, 3
(the Worker half) and 5 to 8 remain design.

Labels: **OWNER DECISION** is binding text from the owner. **AGREED RECOMMENDATION** was proposed by
the coordinator and the owner agreed; it is binding as intent and can still change when built.
**OPEN** is undecided. All of them are intent, not facts.

## 1. Tiers

**OWNER DECISION** unless marked.

| | LARGEST (release) | MIDDLE (PR to master) | SMALL |
|---|---|---|---|
| Event | release tag | PR to master (the `next` -> master batch) | push, and manual |
| Coverage | middle plus SPECviewperf and a real LLM workload; at least 1 driver and 2 dies per family; Windows and Linux required on Turing, Ampere, Ada and Blackwell; at least 2 Windows runs per die | today's check plus all 66 supported Linux apps and Windows `mid`'s 10 apps (including all 8 small apps) | today's Linux bar and required Windows gate; no apps request |
| Parallelism | up to 8 Vast boxes at once | 1 box | rides on the box the middle run already rented (AGREED RECOMMENDATION) |
| Time and cost | hours and several USD are fine; real workload execution is DESIGN-ONLY | 300 min hard limit: reserve 245 min for Linux apps (which build their own media on the box, 2026-11-10) + Windows gate + Windows mid apps, 45 min for earlier work and 10 min soft margin; RETRY on insufficient time (the reserve was 205 min with 85 min for earlier work until 2026-11-10; see `V3_APPS_IMAGE_RECIPE.md`, INTERIM) | existing check budget; no new apps allocation |
| Excluded | nothing by tier intent; LLM slot is refused pending its adapter; a rehearsal also excludes the driver sub-run matrix (stated in every report row) | SPECviewperf and real LLM workload | SPECviewperf, real LLM workload and apps |
| Retries | none: **RETRY never passes** | none | none |
| Die choice | every die in the release matrix | **rotation** across families per PR, with fallback; the family is recorded on the run (AGREED RECOMMENDATION) | the middle run's die |
| VRAM floor | no offer filter (see §5) | a tier setting here only (AGREED RECOMMENDATION) | inherited |

- **Tier is chosen from the event** (AGREED RECOMMENDATION): PR to master = middle; release tag =
  largest; push or schedule = small. The request carries no tier field a caller could narrow.
- **Scheduled runs were considered and dropped for now** (OWNER DECISION). Manual admin runs (§2) and
  the release gate cover that need.
- A retry is never a pass. A FAIL stays a FAIL; the release tier does not re-rent to turn it green.
- **OWNER DECISION, 2026-11-09:** middle includes every Linux middle-list app and Windows
  `mid`, which is a superset of `small`. The old approximate app/time target is superseded by
  the exact lists and guarded budget above. No app is silently dropped to fit. Large inherits
  middle and adds SPEC plus a real LLM workload; `llm_parity` is reserved and refused until its
  CI adapter exists. See the linked middle-apps design for historical timing sources.
- **OPEN:** the exact middle-tier rotation order and the fallback die when the rotation's die has no
  offer.

## 2. Admin start route

**OWNER DECISION (direction).** The CI Worker's admin page and API start a run for a die, a commit
and a profile, and post the result to D1. The `gpu-vast` check then only asks "did requirements R
pass for commit X?" and the Worker answers lazily (parent §1 and §4): it joins in-flight runs, caches
FAIL, applies the caps, and forks never trigger rentals. **Not built.**

**Stop-gap in use today:** a trigger PR to master (as #249 and #255), because `gpu-vast` rents only
for PRs to master or pushes to master. The stop-gap goes away when the admin route exists.

## 3. Result key: the tree hash

**AGREED RECOMMENDATION.** The key of a result is the **tree hash**: a hash of the sorted list of
unignored files, each as (path, mode, blob sha), **after applying the commit's own `.workerignore`**.
Commits with an identical filtered tree share every result: reverts, merges, squashes and edits that
touch only ignored files, including `.workerignore` itself when the filtered tree does not change.

This replaces the parent's `ci/test_boundary.toml` boundary hash read from the BASE branch. Reading
the ignore file from the commit under test is safe only because the key is the filtered tree, not the
changed paths:

- A PR that newly ignores existing code changes the filtered tree (the file leaves the list), so the
  key changes and the result does **not** fall through. It cannot hide changed code that way.
- A PR that adds a new file together with its exclusion falls through, which is right: the file was
  never in any tested tree, and the box deletes it after cloning (§4).
- **Do not implement it as "all changed paths match the new ignore".** That test is the one a PR can
  game, because the PR chooses the ignore.

**Fail closed.** A truncated tree listing, a pattern the matcher does not understand, or a
disagreement between the matcher and `git check-ignore` means **run**, never fall through.
`ci/requirements.toml` still names the requirement sets and is still read from the BASE branch.

## 4. The ignore ladder

**AGREED RECOMMENDATION.**

```
.gitignore  is a subset of  .workerignore  is a subset of  .dockerignore
```

- A **line-based superset** at each step, asserted by a CI lint (every line of the narrower file is a
  line of the wider one).
- **One syntax subset**, understood identically by git, docker and our matcher: no negations. CI
  asserts the matcher against `git check-ignore` on the repository's own paths.
- **`.workerignore`** excludes itself, `.gitignore`, `.git/`, `archive/`, `traces/`, docs, site,
  `*.md`, the GitHub CI files, the Worker's JavaScript, and local test scripts the Worker does not use.
- **`.dockerignore`** additionally excludes `.dockerignore`, `docker-compose.yml`, the box scripts,
  tests, and scripts not meant for Docker.
- **The box deletes everything matching `.workerignore` after cloning**, so a test cannot depend on
  an ignored file.
- **GPU-free Rust CI jobs should also run on the stripped tree**: it proves the dependency set.
  Known example: a test that reads `traces/` evidence breaks on the stripped tree. The data must move
  or that path must stay unignored. Using the stripped tree is a per-job choice; the docs, site and
  other jobs have their own filters.

**Staging.** First: `.workerignore`, the strip step, and the tree-hash key with ingest and the admin
route. Later: `.dockerignore` and the lint.

## 5. Pillar stages: two guests, and the TRIO

**AGREED RECOMMENDATION.** Middle and largest tiers only.

- **(a) Two-guest stage:** an installed Windows guest and the Linux fast guest at once, each passing
  its own suite.
- **(b) The TRIO:** a host CUDA known-answer loop plus the two guests. **Correctness and forward
  progress are the gate.** Throughput must stay above a loose floor, a starting idea being 10 to 20
  percent of solo throughput (OPEN: the number), to catch starvation. Ratios are recorded and **never
  gate** on a tight performance bound. No TDR, Xid or bugcheck is allowed.
- **Risk:** a saturating host workload may push Windows past its roughly 2 s timeout (TDR). That is a
  real finding, not a harness fault. Expect flakiness at first.
- **Rollout in 3 steps:** (1) a manual run; (2) a non-gating stage that records metrics; (3) required
  after N consecutive passes per family (OPEN: N).
- **VRAM.** The release tier has **no offer filter**: the stage sizes the host slice from free VRAM and
  reports `TRIO_SKIPPED_VRAM`, which is **never a pass**, and the trio must have run on at least one
  die per family before a release. A VRAM floor exists only in the middle tier and is set from
  footprints taken in the first manual run: floor **TBD**, host slice **TBD**, guest footprints **TBD**. The first manual
  run belongs to the `trio` agent. Vast offers have at least 32 GB of RAM (the Worker floor is
  `ramMb` 34360 with 8 cores, `ci/vast-broker/defaults.json`).
- This document reports no two-VM or trio result; informal two-VM runs on one host exist but are not
  evidence and are not cited.

## 6. SPECviewperf

**OWNER DECISION.** Release tier only. The 42,002,015,727 B tarball (size as stated by the owner) is
fetched from the private R2 bucket through a short-lived presigned URL (the only secret-like value a
box may hold). Scores stay hidden on public pages under [SPEC_RESULTS.md](../SPEC_RESULTS.md); the
public API already hides them (parent §8).

## 7. Parent-document points this corrects

The parent now carries these corrections above the text they correct: the tier table (§1 here), the
fall-through key (§3 here) and the open question on tiers.

## 8. First result through the Worker path

**MEASURED:** the Windows stage ran through the Worker path on a GA102 box and passed, n=1: gpu-vast
run 41936606326, revision `d74f6063`, 2026-11-05. One run is not a rate; it proves the path can pass,
not that it passes reliably. The broker logs of the earlier gate runs of 2026-11-04 (one of which reached Windows) are in
`traces/gate_windows_ci_20261104/`.
