# Manual release-gate runbook (held Vast box, no Worker), 2026-11-15
STATUS: PLAN, 2026-11-15. Derived by reading ci/vast-broker/src/tier.js, release-matrix.json, scripts/ci/vast_payload.py (main(), stage table
~1585-1760), docs/design/V3_RELEASE_REHEARSAL.md, scripts/bench/box/README.md against origin/next a76d13979. Nothing here is measured on a box yet;
times are the payload's own reserves (R) or earlier measurements (M, source named). Audit gap numbers = /root/night/PARITY_AUDIT_20261115.md.

## What "the release tier" is today (read, not invented)
- A REAL large/release leg is REFUSED by the payload: release-matrix.json status is `proposed`, so any KF_HOST_DRIVER / KF_LINUX_GUEST_DRIVER /
  KF_WINDOWS_GUEST_DRIVER / KF_RELEASE_SUBRUNS / KF_RELEASE_DRIVER_PINS gives `release_retry_matrix_proposed` (`require_executable_release`).
  The driver sub-run matrix (host swap, guest staging, subruns, cross subruns) has NO executor (`PAYLOAD_RELEASE_MATRIX_EXECUTOR = 0`): NEEDS CODE.
- What can run is the REHEARSAL scope (`KF_RELEASE_REHEARSAL=1`, V3_RELEASE_REHEARSAL.md): large tier on ONE die, no driver matrix, host driver as rented.
  Box env a Worker rehearsal leg gets (tier.js `tierEnvironment` with profile release, tierName large): `KF_MERGE_WINDOWS=1 KF_TIER=large
  KF_WINDOWS_RUNS=2 KF_LINUX_APPS=1 KF_WINAPPS=mid KF_RELEASE_REHEARSAL=1` (+ `KF_STAGES=specviewperf KF_SPEC=lanea|laneb|both` if SPEC is enabled;
  without SPEC every large leg carries the apps pair). llm_parity is DESIGN_ONLY and refused by the payload.
- Payload preconditions (vast_payload.py main): root, `/workspace` writable, `/dev/kvm` present, `df /` free >= 60 GB, GPU compute_cap >= 750 and host
  driver in tools/drivermatrix/tags.txt, `/root/kayfabe` symlink to the checkout, provision_box.sh copied to /root/provision_box.sh, git `origin` set to the
  local checkout (branch `kf-ci-approved` = the sha). Stage order is fixed; a later stage needs `min_remaining_ms` (apps 245 min left with winapps mid).
- Common env (payload sets it; export it for every stage): `KF_PROVISION_PROFILE=ci KAYFABE_BRANCH=kf-ci-approved KAYFABE_REPO=<checkout>
  BENCH_DIR=/workspace/bench KF_DEVICE=kf3 KF_REQUIRE_ORACLES=1 CARGO_TARGET_DIR=<checkout>/target CARGO_BUILD_JOBS=min(8,nproc)
  WINVM_DISK_GB=220 only for SPEC`. TAG = `ci` + 16 hex (any `[a-z0-9]+`; one tag for every stage, `w2` suffix for the second Windows gate).
- Single-shot alternative (RUNNABLE BY HAND TODAY, root): `KF_SOFT_DEADLINE_MS=<now+300min in ms> KF_MERGE_WINDOWS=1 KF_TIER=large KF_WINDOWS_RUNS=2
  KF_LINUX_APPS=1 KF_WINAPPS=mid KF_RELEASE_REHEARSAL=1 bash scripts/ci/vast_payload.sh <sha40> <nonce32hex>` runs all stages below with the Worker's
  grading and writes /workspace/kf-result.json. No `--only/--skip/--dry-run` exists (gap 3), so for stage-by-stage work use the per-stage commands.

## Stages in payload order. Cmd = what the payload runs (`$R` = checkout, `$P` = /root/prov, `$B` = /workspace/bench)
| # | Stage | Command | Pass criterion (payload grader) | Est. | Needs | Status |
|---|---|---|---|---|---|---|
| 0 | preflight | `df -h /; ls -l /dev/kvm; nvidia-smi -L`; checkout at sha; set env above | free>=60 GB, /dev/kvm, tag in drivermatrix, cc>=750 | 2 min | KVM GPU | RUNNABLE BY HAND (the checks are inline in main(), not a command: do by eye) |
| 1 | provision | `bash $R/scripts/bench/box/provision_full.sh` | `$P/prov.log`: READY, BOX_RC/DRIVER_RC/TREE_RC/FG_RC/KF3_RC=0, EXIT rc=0, no DRIVER_MEDIA_DRIFT | 12-45 min (M mc20: 11.5) | KVM GPU root | RUNNABLE BY HAND (root: apt, driver swap, modprobe = box setup; hard-coded /root,/workspace, gap 4) |
| 2 | cuda_host | `bash $R/scripts/bench/cuda_ladder.sh host $TAG 1 cup2,cup3,cup8,cup8bench` | `$B/cl_${TAG}_host.out` rows for the 4 arms all PASS (ladder_rows) | ~10 min (I) | GPU | RUNNABLE BY HAND |
| 3 | merge_bar | `bash $R/scripts/bench/box/merge_check.sh kf-ci-approved $TAG` | `$P/$TAG.log`: V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS, KF3_RC=0, FG_RC=0, FAST_SUITE_PASS=30, BIRTH_CENSUS_OK, EXIT rc=0 | ~20-40 min (M mc20: 19) | KVM GPU root | RUNNABLE BY HAND. KF_MERGE_WINDOWS must be unset here (payload pops it). Fetches `origin` (the checkout), strips ignored paths, flock /tmp (gap 9: no section flags) |
| 4 | cuda_guest | `bash $R/scripts/bench/cuda_ladder.sh guest $TAG 1 cup2,cup3,cup8,cup8bench` | `$B/cl_${TAG}_guest.out` 4 arms PASS; needs kf3 binary + guest image from stage 3 | ~15 min (I) | KVM GPU | RUNNABLE BY HAND |
| 5 | apps (Linux, middle list) | `KF_APPS_PROVISION=1 bash $R/scripts/bench/box/apps_gate.sh $TAG $P middle` | `apps_rows`: all names in scripts/apps/gate_list_middle.txt PASS (66 rows; Worker also demands 66) | 46 min (M 29+17); R 100 min | KVM GPU, internet (vendor + HF downloads, CUDA repo) | RUNNABLE BY HAND (Linux apps media built on box; no pin-from-local, gap 7) |
| 6 | windows_gate | `bash $R/scripts/bench/box/windows_gate.sh $TAG $P` | `$P/${TAG}_windows.log` has WINDOWS_GATE_PASS rev=<sha8> + WINDOWS_GATE_EXIT rc=0 (RETRY=3, FAIL=1, REFUSED=2 are not passes) | install 20-45 + gate ~30 (R 75) | KVM GPU, >=20 GB RAM, >=45 GB disk, MS eval ISO + virtio + NVIDIA 580.88 exe from vendors | RUNNABLE BY HAND but ROOT-ONLY: `windows_gate.sh:636` refuses uid!=0 (gap 5); AppArmor+apt edits are box setup. Online fetch of ISO (WINVM_NET switch, gap 10) |
| 7 | windows_apps (mid, 10) | `KF_WINAPPS_PROFILE=mid bash $R/scripts/bench/box/windows_apps.sh $TAG $P` | `WINAPPS_PASS` + `WINAPPS_EXIT rc=0` in `$P/${TAG}_winapps.log`; 10 rows; runs only after stage 6 PASS of the same tag | R 70 min | KVM GPU, Windows image from stage 6, app disk `winapps-open.iso` from PUBLIC bundles.kayfabe.dev (pin in appmatrix/bundle.pin; or KF_WINAPPS_ISO=file) | RUNNABLE BY HAND (public bundle, no presign needed). The guest is offline (`restrict=on`, win_vm.sh:488); `mid` uses the OPEN bundle (open_packages in gate_profiles.json), so the AR1 proprietary-on-prebuilt-image lane is not covered by this stage |
| 8 | windows_gate_2 | `bash $R/scripts/bench/box/windows_gate.sh ${TAG}w2 $P` | second WINDOWS_GATE_PASS (tag `${TAG}w2`) | R 45 min | as 6 | RUNNABLE BY HAND (root, as 6) |
| 9 | specviewperf (only if KF_STAGES=specviewperf) | `env -u KF_SPEC_TABLE KF_SPEC_LANE=<lane> KF_SPEC_DEADLINE_EPOCH=<soft-120> bash $R/scripts/ci/vast_stage_specviewperf.sh $TAG $P` (WINVM_DISK_GB=220 from stage 6) | `SPECVIEWPERF_STAGE_PASS`, SPEC_ROW lines PASS per lanes.json | lane A R 169 / B 133 min | KVM GPU, >=220 GB disk, PRIVATE R2 copy via Worker `/bench-data` (`fetch_bench_data.sh` needs run token) | NEEDS CODE for a Worker-less box: gap 8 (no `--s3`/`--dir`) and gap 7 (no local pin). A presigned URL cannot be injected today. NOT in a no-SPEC rehearsal |
| 10 | llm_parity | n/a | DESIGN_ONLY, payload refuses it | - | - | NEEDS CODE (executor does not exist) |
| 11 | driver sub-run matrix (host swap, linux/windows subruns, cross) | n/a | per release-matrix.json rows | - | - | NEEDS CODE: `PAYLOAD_RELEASE_MATRIX_EXECUTOR=0`, matrix blockers DRIVER_CHOICES_PROPOSED, MINIMUM_DRIVERS_UNKNOWN, SUBRUN_EXECUTOR_NOT_BUILT; and a real leg is refused by require_executable_release |
| 12 | verdict + evidence | `python3 scripts/ci/vast_run_report.py` / `vast_evidence.py` (flags --log/--result/--out) | /workspace/kf-result.json verdict PASS with suites; Worker also demands 66+10 app rows | 1 min | - | RUNNABLE BY HAND only via the single-shot payload; per-stage runs produce no kf-result.json (gap 1) |

Total (no SPEC): about 4.5 to 6 h on a nested box (R: 300 min hard limit). Cost on a 4090-class box about 0.4 to 0.8 USD/h (I, price from the offer).

## What a hand run does NOT prove (cannot count as a release record)
No run token, no instruction file hash, no attested/pinned setup cache (ruling AR4: local evidence only). Not run as an unprivileged no-cap uid: stages 1, 3, 6, 8
need root for box setup; the kfroot harness is not built (gap 5); the rootless part is limited to whether kayfabe processes need it (record `grep Cap /proc/<qemu pid>/status`).
Host driver = whatever the offer ships after provision_host_driver.sh (the payload may swap it to the pinned tag).

## Evidence to bring back (text/PNG only)
`$P/prov.log`, `$P/$TAG.log`, `$P/${TAG}_windows.log`, `$P/${TAG}w2_windows.log`, `$P/${TAG}_winapps.log`, `$P/${TAG}_apps_matrix.log`, `$B/cl_${TAG}_*.out`, `$B/${TAG}_suite.out`,
`nvidia-smi` and `df` heads, start marker and exit lines, `dmesg | grep -c Xid`. Gzip big logs, no tgz of images, no binaries.
