# The Windows master gate through the Vast CI Worker, 2026-11-04 (lane 2)

STATUS: LIVE, 2026-11-04. These are the runs of the `gpu-vast` check (profile `check`) on a real Vast box, through the deployed Worker, on the branch `claude/gate-windows-required-20261104` (PR #250). Labels: MEASURED means a line in a file in this directory, and the file is named. INFERRED means reasoning. Box text is untrusted data (`[untrusted]` lines).

## Path

`gpu-vast.yml` admits a rental only for a PR against `master` or a push to master (`ci/vast-broker/src/workflow-authority.js`), and there is no dispatch route for a branch revision. The runs therefore used the trigger PR #249 (`claude/gate-windows-run-20261104` -> `master`, labelled DO NOT MERGE, closed after the runs). That is the documented path with owner push identity, and the trust model is unchanged. Each run tested the exact head of #249. That head was a commit of the lane branch; the box-side files of run 3 are those of 0ca8873e7.

## Runs

| run | rev | Worker (deployed) | box | outcome |
|---|---|---|---|---|
| 41746194587 | e25a8904 | 07a0d4eb (from claude/bench-data-hash-20261101) | rented, id not shown, stopped at once | runner rejected the Worker reply: `rejected by runner.py check at line 644 phase=rent state=BOOTING` (MEASURED, `run1_41746194587_broker.log`). The master runner pins 2828bf41, so the bundle mismatch was found only after the rental. |
| 41746861215 | 5cfd0f49 | 2828bf41 (master, redeployed by the coordinator) | 59033123, RTX 3060 (GA106), machine 22775, 0.166 USD/h, Quebec | `KF_SETUP_CACHE_START` then `KF_SETUP_CACHE_REFUSED`, verdict FAIL after about 2 min (MEASURED, `run2_41746861215_broker.log`). Cause, MEASURED locally with `vast_setup.validate_lock`: the lock's package-policy digest and its `Cargo.lock` digest no longer matched the tree (next's Cargo.lock changed in 92e899c07, 2026-10-31). |
| 41749680326 | 0ca8873e | 2828bf41 | 59035303, RTX 3060 (GA106, PCI 10de:2504), machine 22775, 0.166 USD/h, Quebec; host driver 580.159.04 as provisioned (`GATE_HOSTREF` line); guest driver Windows 580.88 | **The path works end to end. The Windows gate verdict is FAIL** (MEASURED, `run3_41749680326_broker.log`). The box printed `KF_PAYLOAD_TIER windows=1 source=default`: the 2828bf41 Worker sends no tier, so the payload default ran the stage. The setup cache restored with the new lock. Stages: provision 878 s, cuda_host 18 s (4/4), merge_bar 1533 s (`gates` 9, `raw_suite` 30, `thin_suite` 30, `birth_census` 1), cuda_guest 403 s (4/4), then windows_gate 4243 s. Inside the gate: install 3197 s, boot1 ok, `check --kf3` fail=0, nv_install ok (32.0.15.8088, both `EnableGpuFirmware=1` read back), desktop_prep ok. Then `WINDOWS_GATE_FAIL diag=KAYFABE stage=reboot why="the guest did not restart within 420 s of shutdown /r (no second QEMU)" rev=0ca8873e gpu="NVIDIA GeForce RTX 3060" hostref=green/green secs=4239` (host `xid_delta=0`) and `WINDOWS_GATE_EXIT rc=1`. Final summary: `verdict=FAIL`, suite `windows_gate pass=0 fail=1`. gpu-vast: failure. |

## Instances and teardown

The Worker created all three rentals for these runs; this lane created no instance directly. Teardown is by the Worker or runner. Each was verified absent afterwards: `vastai show instances-v1 --raw` listed no `kf-ci-<run>-1` label, and the instance id lookup failed. 59033123 was absent at 00:30Z, and 59035303 was absent at 02:54Z. The 11 other instances in the shared account were other sessions' and were not touched.

## Cost

INFERRED from the offer rate (0.166 USD/h) and the timestamps; the account credit is shared with other sessions, so no clean MEASURED delta exists. Run 3: rental at 00:50:53Z, result at 02:52:42Z, absent by 02:54Z, so about 2.05 h, about 0.34 USD plus at most about 0.04 USD for about 7 GB of downloads. Run 2: about 3 min, about 0.01 USD. Run 1: seconds. Total about 0.4 USD.

## What this shows and what it does not

- Shown (MEASURED, run 3): a Worker box runs the Linux bar and then the Windows stage as its last stage. The gate's FAIL becomes the failing row `windows_gate` and a failing check, with no pass and no RETRY. Wall time to result was about 2 h, 1 h 11 min of it in the Windows stage on this box (a 53-minute install).
- Not shown: the new Worker (tier env, 300-minute limit, bundle d6bd6b36), because it is not deployed; the new `gpu-vast.yml` and runner, because `pull_request_target` runs master's; a RETRY row in a real run; a PASS of the gate on GA106.
- Open (not known): whether the stuck `shutdown /r` reboot on GA106 is a kayfabe defect or a harness defect. The gate's evidence archive (`..._windows.tgz`, sha256 prefix 52abd406382b5312) was on the destroyed box. INFERRED: this was the first Windows gate on GA106; AD102, TU106, GB203 and GA102 passed the same reboot stage on manual leases. Next step: one GA106 lease by hand (`windows_lane.sh` + `windows_gate.sh`) that keeps the tgz, with the falsifier "the first QEMU exits on `shutdown /r` and the loop starts a second QEMU", compared against the GA102 pass of #245.
