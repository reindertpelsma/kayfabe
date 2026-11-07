# Status and handoff — where kayfabe v3 stands, and how to resume

**STATUS: LIVE, 2026-11-07.** One page to resume from. The newest dated UPDATE below (2026-11-04) is the last resume point recorded in this file; everything older was moved on 2026-11-07, unchanged, to [`history/`](history/) (see *History and the citation map*). Nothing in this file was re-run on 2026-11-07: each entry keeps its own date and evidence pointer, and a lane that finished later records itself in its own design document and `traces/` README, not here.

## Resume checklist

1. [`../AGENTS.md`](../AGENTS.md) (rules and layout), [`../README.md`](../README.md) (status in one screen), [`../ARCHITECTURE.md`](../ARCHITECTURE.md) (v3 on one page).
2. [`OWNER_RULINGS.md`](OWNER_RULINGS.md): every owner decision with its date (binding).
3. [`CI_V3.md`](CI_V3.md) and [`design/V3_CI_TIERS_AND_IGNORE_LADDER.md`](design/V3_CI_TIERS_AND_IGNORE_LADDER.md): what the gates and tiers are.
4. [`STATUS_DETAIL.md`](STATUS_DETAIL.md): the long status ledger (its own STATUS says which parts are history).
5. The state of the branches: `git log --first-parent origin/master` (one line per batch) and `git log --first-parent origin/next`.
6. The documents map: [`README.md`](README.md) (current documents versus history).


## UPDATE 2026-11-04 — Windows is required on the gpu-vast master gate (lane 2)

Branch `claude/gate-windows-required-20261104`, PR #250 against `next` (its parents #242-#247 are in next via #248).
Design: `docs/design/V3_CI_VAST_GPU_20261021.md` (lane 2 section), `docs/design/V3_WINDOWS_GATE.md` §0.
Evidence: `traces/gate_windows_ci_20261104/README.md`.
- CODE READ: the Worker profile `check` sends `KF_MERGE_WINDOWS=1` (`ci/vast-broker/src/tier.js`). The payload runs
  `windows_gate` as the last stage, and only `WINDOWS_GATE_PASS` passes. RETRY is a failing `windows_retry_<class>` row,
  and the check fails with "GPU gate RETRY". Check hard limit 300 min, sweep 180, runner 330/200, job 345/step 340,
  reaper 300.
- MEASURED (runs 41746194587 and 41746861215, 2026-11-04): two deployment-state blockers, both outside this change.
  (1) The deployed Worker (07a0d4eb) differed from master's pin (2828bf41), so every master-PR run was rejected by
  runner.py line 644 after the Worker had rented. The coordinator redeployed 2828bf41.
  (2) Next's `Cargo.lock` and package policy had drifted from `scripts/ci/setup-cache.lock.json` since
  2026-10-31, so every next tree failed `KF_SETUP_CACHE_REFUSED` on a Worker box. A new cache
  (`setup-cache-jammy-amd64-20261104-v1`, now with the Windows packages) is published, and
  `scripts/ci/test_vast_setup_lock_fresh.py` now catches the drift in GPU-free CI.
- Owed by the coordinator after the merge: deploy the reaper (300) first, then Worker bundle
  `d6bd6b363fc04592dc18d7f607178c981bb97983ce36b5c4037bafda1a337b0a`. master's runner must carry the matching pin,
  and the 330-minute budget must be on master before the 300-minute Worker goes live, or master PRs get
  `NOT_ELIGIBLE RUNNER_TIME`.

## UPDATE 2026-11-04 — platform testing and the public status site (lane 1, design + code, not deployed)

DESIGN-ONLY: [V3_PLATFORM_TESTING.md](design/V3_PLATFORM_TESTING.md) records the owner's
2026-11-04 design: the Worker as platform testing infrastructure decoupled from CI, the three
tiers, a read-only public API at `api.kayfabe.dev` over one shared D1 database, public redacted
logs, and the status pages at `kayfabe.dev/status/`. Branch `claude/platform-status-20261104`
builds the public API, the schema and the pages against synthetic data only; deployment steps are
in that document's §12. This entry is scoped to that lane; it is not a resume point for the
product.

## 5. How work was run (so it can be run again)

- **Boxes:** `scripts/bench/box/README.md` — rent, register with the idle watchdog
  (`vast_reaper.sh`), provision (`provision_full.sh`), check (`merge_check.sh`), destroy. From a cloud
  session (no SSH egress) the same scripts are driven through the `vx` kit — execd behind a cloudflared
  quick tunnel (the v3-mc20 bar ran this way: `traces/v3_mc20/README.md`).
- **Merging:** agents push sub-branches; a merge branch `v3-mcN` combines verified work on the current
  master; the full bar runs on a box at that exact revision; then master and v3 are fast-forwarded. Only
  docs/traces may be added on top of a verified revision without re-running the bar.
- **Local host:** shared and small; builds go on boxes; local cargo only under
  `flock /tmp/claude-0/kf-local-cargo.lock`, 2 jobs, own target dir deleted after.
## 6. Lessons from this campaign worth keeping

- **A refusal on a wait/flush path is a forged completion.** Symptom: results faster than bare metal
  (clpeak "FP64 450 GFLOPS" vs 218). Audit non-OK statuses against a bare-metal trace (nvdiff) first.
- **An entry retired without its work executing is also a forged completion** (§4.1).
- **Merges combine branches that each passed alone** — two branches added a same-named field with
  different meanings; one branch left another's test red; two bumped the same ABI number. Resolve
  semantically and re-run the whole bar on the merged revision.
- **A test runner without `--no-fail-fast` under-reports** (it stopped at the first red crate).
- **A watchdog must not read "cannot probe" as "idle".**
- **Boxes vanish** (vast destroyed several mid-run); evidence must already be in git.
- **A coexistence test that tolerates a stall reads the stall as a pass** (2026-09-30): the b3 host proof
  logged host CUDA stalling for a whole 3 s fault park as coexistence PASSED; the guest plane then
  deadlocked on exactly that (`design/V3_UVM_GUEST_FAULT_PLANE.md`). Bound the latency, not just the outcome.
- **A cached "not yet" outlives its cause** (2026-09-30, §4.9): a `OnceLock` kept a transient "no guest RAM"
  for a VM's life. And the first plausible cause (a killed QEMU just before) was wrong: read the failed run's
  own log against a passing run of the same arm, and grep the signature's base rate across all archived logs.
- **`git stash` is shared by every worktree of a repo** — another agent's `stash pop` applied someone
  else's stash. Agents working in parallel worktrees save a patch file instead of stashing.

## History and the citation map

The unsplit file is at `a9dee7b2c` (`git show a9dee7b2c:docs/STATUS_AND_HANDOFF.md`). Source comments and design documents cite this file by section number or by line number; the table says where each cited part lives now. A citation of the form `STATUS_AND_HANDOFF.md:NNN` is a line number of the unsplit file at `a9dee7b2c`.

| Cited as | Lines at `a9dee7b2c` | Now in |
|---|---:|---|
| UPDATE 2026-11-04 (two entries) | 6-36 | this file |
| UPDATE 2026-10-22 down to 2026-10-11 22:59 UTC | 37-1104 | [`history/STATUS_AND_HANDOFF_UPDATES_2026-10-11_to_2026-10-22.md`](history/STATUS_AND_HANDOFF_UPDATES_2026-10-11_to_2026-10-22.md) |
| "Decision resolved" (2026-10-12), `§0` "Current resumption" (2026-10-14), "Latest checkpoint" 2026-10-11 | 1105-1601 | [`history/STATUS_AND_HANDOFF_CHECKPOINTS_2026-10-11_to_2026-10-14.md`](history/STATUS_AND_HANDOFF_CHECKPOINTS_2026-10-11_to_2026-10-14.md) |
| `§0.0` "RESUME HERE 2026-10-07" and "Historical resumption 2026-10-05" | 1602-2448 | [`history/STATUS_AND_HANDOFF_RESUME_2026-10-05_to_2026-10-07.md`](history/STATUS_AND_HANDOFF_RESUME_2026-10-05_to_2026-10-07.md) |
| `§1` master and its merge bar, `§2` remaining work, `§3` owner decisions of 2026-09-28 (`§3.1`-`§3.4`), `§4` queued investigations (`§4.1`-`§4.12`) | 2449-2623 | [`history/STATUS_AND_HANDOFF_SECTIONS_1-4_2026-09-28.md`](history/STATUS_AND_HANDOFF_SECTIONS_1-4_2026-09-28.md) |
| `§5` how work was run, `§6` lessons | 2624-2654 | this file |

The references to `§0`, `§4.1` and `§4.9` inside sections 5 and 6 above point into the history files in the table.
