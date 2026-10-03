# Candidate v3-cand-1 on real hardware — merge bar, apps, display (2026-10-03/04)

**STATUS: IN PROGRESS, 2026-10-03 23:05 UTC.** Stage 1 (merge bar) is done and passed. Stages 2
(apps) and 3 (display) are running on the box; this file is updated after each stage.

**Tested commit (every run below):** `8a682f1b415844801e855ca6b28388f91c244251` (branch `v3-cand-1`,
"merge v3-dispsw-exp"). This directory is evidence only: the commits that add it change no code.

**Box:** vast 54049598 (`vmb`), RTX 3060 (GA106, 12 GB), 19 vCPUs, 49 GB RAM, KVM template, host
driver 580.159.04 (open kernel module), host kernel 6.8.0-59-generic. The box pulled the commit from
GitHub (`/root/kayfabe`, `git checkout --detach 8a682f1b…`, clean). One GPU job at a time.

**CI at the same commit:** GitHub Actions runs 37157600752 (push) and 37157724581
(workflow_dispatch), both `success` at head `8a682f1b`, all four jobs (stable, aarch64, slow,
firmware).

## Stage 1 — the merge bar (`merge_bar/`)

Command, from the checkout, with a target directory of its own (fresh, created by this run, so no
artifact of another checkout could be reused):

```
CARGO_BUILD_JOBS=$(nproc) CARGO_TARGET_DIR=/workspace/bench/kf-target-cand1 \
  bash /root/kayfabe/scripts/bench/box/merge_check.sh v3-cand-1 cand1mb
```

`merge_check.log` is the script's own log: `START 2026-10-03T22:38:04`, `HEAD=8a682f1b4158…`,
`EXIT rc=0 2026-10-03T23:01:02`.

| item | result | log |
|---|---|---|
| revision (the script's own worktree) | `8a682f1b415844801e855ca6b28388f91c244251` | `merge_check.log` |
| every kf-* crate test | **1936 passed, 0 failed** | `tests.log` |
| v3 gates | **9/9**; `V3_GATES_BIRTHS births=11 user=11 refused=0 ok=1` | `gates.log` |
| kf3 built from this revision | `KF3_RC=0`, `kf3-bins/8a682f1b`, GOP driver embedded | `kf3.log` |
| bare-metal raw-client suite | **30/30**, 0 crash | `host.log` |
| fast guest rebuilt | `FG_RC=0` | `fg.log` |
| 30-arm thin-guest suite, kf3 | **30/30, 0 crash, 0 not run** | `suite.out` |
| census self-test (planted logs) | 11 cases, 0 wrong | `census_selftest.log` |
| channel-birth census | `BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=11 all PRIVILEGED_CHANNEL=0 privilege=USER` | `births.log` |

Guest kernel logs: every one of the 30 thin-guest boots left a non-empty kernel console log
containing `NVRM` lines (22–234 per arm) and no `Xid` line — `guest_console_nvrm.txt` (one row per
arm, counted on the box from `fast_cand1mb_<arm>_ttyS0.log`), the 30 logs themselves in
`guest_consoles_ttyS0.tgz`.

## Stage 2 — apps (`apps/`)

Running: app bundle built on the box (`scripts/apps/build_bundle.sh`), host runtime
(`setup_side.sh`), the guest image a copy of `guest.qcow2` provisioned with
`provision_guest_apps.sh`, then `apps_matrix.sh host` and `apps_matrix.sh guest` (one boot, no PM,
`APPS_PER_BOOT=100`, then every non-PASS app alone) with the merge bar's kf3 binary.

## Stage 3 — display (`display/`)

Queued after stage 2: the GOP lane (B0, B1, B5) and the x11-dispsw A/B pair.
