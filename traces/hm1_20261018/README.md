# HM1 candidate mirror validation — parked 2026-10-25

STATUS: PARTIAL. Subject revision `6564dd5977bbcd6aa947359b55b4aa3d9a5e92f4`;
all candidate observations below use `kf3-bins/6564dd59/qemu-system-x86_64`.
Lane branch `codex/hm1-20261025`; reports and scripts only, no product change.
Run identifiers and evidence names refer to the owner's private HM1 evidence directory.
No benchmark scores or workload imagery are published.

MEASURED HM1 build (2026-10-25, `build.log`, `binary.sha256`): host-only
`build_kf3.sh` completed with a clean pinned source tree. Binary SHA256:
`82a806eeacbfc3df5f42e1575fd4b242d7fda7d3f4781f8fe0f708398c0a658a`.
MEASURED HM1 native gates (`v3-gates.log`): 9/9 PASS, gate10 PASS,
12/12 USER births, zero refused births; zero host Xid/OOM through those gates.

MEASURED HM1 fast suite (rev `6564dd597`, `corrected-fast-result.json`): six accepted arm passes,
one controller host-fault stop at `hm1-raw_defer-liveness`, 23 remaining arms unrun.
The stopped arm's guest client returned zero and printed
`RUNG_defer_liveness=PASS` and `FASTGUEST: DONE`; QMP powerdown was requested
on the controller's host-fault observation, so this is not an unaffected seventh pass.
The controller recorded 15.824 seconds, below the unchanged 180-second budget.
Its original TIMEOUT label is incorrect: it used exit 124 for a host-fault stop.
The original scoreboard remains preserved, alongside the explicit correction.

CODE READ at revision `6564dd597`,
`crates/kayfabe-rm-ladder/src/main.rs` (`defer_liveness_prime`, step 5):
the test deliberately accesses invalid mappings and explicitly predicts host
Xid 31. MEASURED HM1 `kernel-stop-preserved.log`: four Xid 31 CE0 read-fault
records name the stopped QEMU PID. MEASURED HM1 stopped-arm QEMU log:
`mirror=[on ...]`, groups=0, ctx_created=0, ctx_unknown=0, mode_refused=0,
flag_refused=0, unwind_failed=0. The explicit mirror path was unexercised.
INFERRED from this source and HM1 evidence: these observations establish neither
a mirror regression nor host instability. The blanket zero-Xid controller
criterion conflicted with a fault-inducing raw test; it was too broad.

MEASURED HM1 stop-reason replay (rev `6564dd597`, `stop-reason-expected-red.log`):
[test_stop_reason_expected_red.py](../../scripts/bench/hm1/test_stop_reason_expected_red.py)
fails with assertion exit 1 against the deployed controller. It replays the
host-fault exit expression without GPU access. A separate harness repair should
preserve a distinct HOST_FAULT reason. No repair or new hardware run occurred.
Before a new campaign, define expected intentional-fault treatment explicitly;
do not silently change the stopped run's preregistration.

MEASURED HM1 final audit (`final-audit.json`, 2026-10-25 17:25:28 UTC):
no remaining QEMU or owned controller, GPU idle, GDM active, lock released,
owned Cargo target deleted, zero OOM records. QEMU exited 0 after QMP powerdown.

| Required check | Verdict |
|---|---|
| Host build and binary identity | PASS |
| Native gates 9/9 plus gate10 | PASS |
| Fast suite 30/30 | UNRUN in full; controller gate FAIL, partial results above |
| Linux CUDA, graphics, video, broker, interactive, UVM | UNRUN |
| Windows Steel mirror ON/OFF | UNRUN |
| Matched M2 ON/OFF/6d arms | UNRUN |
| Nine viewsets and selected Enscape | UNRUN |
| D3D12 hold, unload/reload, reset with live explicit groups | UNRUN |

Recommendation: **do not promote on HM1 evidence**. Required validation is
incomplete. The standing common lane rule requires stopping after an agent's
own safety check blocks a step; the lane was parked without changing that rule.
