# Manual release-gate REHEARSAL on one held Vast box, 2026-11-15

**STATUS: LIVE, 2026-11-15.** MEASURED on one rented box, run by hand (stage commands as the payload would run them), NOT through the Worker.
It is local evidence only (no run token, no instruction file, no attestation; OWNER_RULINGS section AR, item 4) and it is the large-tier
REHEARSAL scope (`KF_RELEASE_REHEARSAL=1`, no driver sub-run matrix, no SPECviewperf), not a release record.

- Tree: `origin/next` = `a76d139790ab2ef126d4abd26b9237e887c5af15` (checkout `/workspace/kf-ci-src`, sparse like the Worker's onstart; kf3 rev `a76d1397`).
- Box: Vast instance 60728222, label `kf-relgate-20261115`, RTX 4090 (AD102), AMD EPYC 7502, 15 vCPU, 98 GB RAM, 194 GB disk, `/dev/kvm` present
  (`crw-rw---- root kvm`), image `vastai/kvm:ubuntu_cli_22.04`, 0.536 USD/h. Rented 02:55Z, destroyed 06:51Z (3.9 h, about 2.1 USD estimated from the
  offer rate, not an invoice). A first instance, 60727399, never left `stopped` ("resources unavailable") and was destroyed after 6 minutes. Teardown
  verified absent with `vastai show instances-v1 --raw` (own ids only). Host driver after provisioning: 580.159.04 (in `tools/drivermatrix/tags.txt`).
- Stage order and commands: `RUNBOOK.md` (derived from `tier.js` and `vast_payload.py`). Env as the payload sets it: `env.sh`; per-stage wrapper with START/EXIT
  lines: `workspace/relgate/stage.sh`, `stages.log`; chain: `chain.sh`. Run as root (box setup and `windows_gate.sh:636` need it).

## Results (verdict lines are in the named files; times from `stages.log`)

| Stage | Result | Time | Evidence |
|---|---|---|---|
| provision | PASS: BOX/DRIVER/TREE/LADDER/FG/KF3_RC=0, READY driver=580.159.04 | 11 min | `prov/prov.log` |
| cuda_host | PASS 4/4 (cup2, cup3, cup8, cup8bench) | 9 s | `workspace/bench/cl_cirelgate0001_host.out` |
| merge_bar | PASS: V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS, KF3_RC=0, BARE_SUITE_PASS=30, FAST_SUITE_PASS=30, BIRTH_CENSUS_OK (births 161 + 12, all USER) | 21.5 min | `prov/cirelgate0001.log`, `workspace/bench/cirelgate0001_suite.out` |
| cuda_guest | PASS 4/4 | 6 min | `workspace/bench/cl_cirelgate0001_guest.out` |
| apps (Linux, middle list) | PASS 66/66 (`APPS_STAGE_COUNTS pass=66 fail=0 not_run=0`; provisioning inside the stage 24 min, matrix 21 min) | 45 min | `workspace/relgate/apps.run`, `prov/cirelgate0001_apps_matrix.log` |
| windows_gate | `WINDOWS_GATE_PASS` rev=a76d1397 (install 33 min, idle 300 s, resets=0, known_live_dumps_193=1 ratchet PASS) | 63 min | `prov/cirelgate0001_windows.log` |
| windows_apps mid | `WINAPPS_PASS` profile=mid pass=10 expected=10 (public bundle) | 30 min | `prov/cirelgate0001_winapps.log`, `.../run/session.log` |
| windows_gate_2 | `WINDOWS_GATE_PASS` (tag `...w2`, install reused) | 28 min | `prov/cirelgate0001w2_windows.log` |
| specviewperf, llm_parity, driver sub-run matrix | NOT RUN | - | RUNBOOK.md rows 9-11 (NEEDS CODE) |

Total from provision start to the last gate: 3.5 h of the 5 h the payload reserves.

## Rootless probe (bonus, after the stages; uid 1002 `kfrun` in group `kvm`)
`setpriv --reuid kfrun --regid kfrun --groups kvm --inh-caps=-all --bounding-set=-all --no-new-privs`; the wrapper prints `/proc/self/status`
(`CapInh/Prm/Eff/Bnd/Amb = 0`, `NoNewPrivs: 1`) before the workload (`workspace/relgate/rootless/`):
- `cuda_ladder.sh host` cup2 and cup8: PASS. `cuda_ladder.sh guest` cup2 (kf3 guest boot): PASS (`guest_cup2.out`, trapped=10588).
- `fast_suite.sh` 30 arms at budget 180: `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0` (`suite.out`, `cirelrl4_suite.out`).
- Device access that sufficed on this image: `/dev/nvidia*` were mode 0666, `/dev/kvm` group `kvm`. RLIMIT_MEMLOCK was 12966280 KiB (not unlimited by design;
  the audit's memlock question is NOT settled for a small default limit). Not probed rootless: the Windows gate and apps, merge_check, provisioning.
- Harness defects met (each cost one retry, both box setup, none in kayfabe): (1) `/tmp/kayfabe-fastguest.lock` was created by root and then refused to a
  non-root user (`Permission denied`, then `flock: Bad file descriptor`; sysctl protected_regular); (2) git `detected dubious ownership` on a root-owned checkout
  (`safe.directory`) made the guest arm fail with boot_rc=2.

## Caveats (said, not hidden)
- The payload (`vast_payload.py`) itself was NOT run, so its grading, `BASH_ENV` xid hooks, `kf-result.json` and the 245-minute start guards did not apply.
  The stage commands are the same ones; the verdict lines above were read by hand from the logs. 15 `NVRM: Xid 31` (CE MMU fault, `qemu-system-x86`) lines were in dmesg at the end. Only 8 were
  printed and saved (`xid_summary.txt`: 03:19-03:22Z in the merge-bar suite window, 06:35-06:38Z in the rootless suite window); the other 7 timestamps were not kept
  (the box is gone). None of the 8 falls in a Windows stage. INFERRED: the suite's deliberate-fault arms; the payload's xid allow-list was not applied to confirm.
- My own error: the first `clone.sh` ended with the Worker's `exec bash scripts/ci/vast_onstart.sh`, which ran once, failed its env check ("missing" KF_*) and exited.
  It did install the box helper tools; nothing else changed.
- One box, one die (AD102), one host driver, one run; nothing here is a claim about another die, driver or revision.
