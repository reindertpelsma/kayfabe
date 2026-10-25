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

## Authorized continuation — 2026-10-25

STATUS: PARKED after the continuation admission rule rejected FAULT_PDE.
This section supersedes the prior current-status counts. Subject remains revision
`6564dd597`, binary `kf3-bins/6564dd59/qemu-system-x86_64`, unchanged hash.

CODE READ coordinator continuation ruling (2026-10-25): intentional negative
controls may emit own-QEMU-PID Xid31 CE0/HUBCLIENT_CE1 FAULT_PTE virtual read/write
at declared primed VAs. Count distinct primed VAs, so a repeat is not an extra prime.
Other types, PIDs, VAs, noninducing arms and excess primes still stop the lane.

MEASURED HM1 admission tests at commit `53ef1c854`: 36 cases, eight failures
against the unchanged blanket guard, committed EXPECTED RED before the fix.
MEASURED HM1 admission tests at commit `3003ba3c1`: 39/39 PASS on dev box and host;
14 policy mutants each killed by two tests or more, as recorded in
[admission-mutations.json](admission-mutations.json). The original stop-reason
regression also passes. This changes only the HM1 harness, not the candidate.
The continuation controller uses a fresh kernel journal cursor for each owned VM.

MEASURED HM1 continuation (rev `6564dd597`, `continuation/corrected-fast-result.json`):
`defer-liveness`, `blockage-coverage`, `alias-two-vas`, `alias-unmap-observe`,
`map-propagation` and `late-map-race` passed. The first admitted four own-PID
CE/PTE records at three declared primed VAs. Combined with the six retained
passes, the complete suite has **12 accepted passes, one admission stop,
17 unrun arms**; there is no 30/30 claim.

MEASURED HM1 (rev `6564dd597`), `hm1c_missing-page-fault/fault-admission.json`: the stopped arm
emitted one own-PID Xid31 CE0/HUBCLIENT_CE1 **FAULT_PDE** VIRT_WRITE at its declared
victim VA. Admission rejected the fault kind because the ruling permits PTE only.
MEASURED HM1 stopped-arm serial/exit receipts: raw containment assertions PASS,
client rc=0 and DONE; QMP system_powerdown; QEMU exited 0. This is a HOST_FAULT
admission stop at 6.780 seconds, with no timeout misclassification.
CODE READ candidate `crates/kayfabe-rm-ladder/src/main.rs`, `missing_page_fault`:
the fixture deliberately accesses a never-mapped VA and checks bystander survival.
No mirror regression or wider fault admission follows from the passing raw assertion.

MEASURED HM1 final continuation audit (`continuation/final-audit.json`,
2026-10-25 17:54:46 UTC): no QEMU/controller, lock released, GPU idle, GDM active,
no OOM, target remains deleted, source clean and candidate hash unchanged.
The continuation journal contains four admitted PTE records and the one rejected
PDE record. All task 2 Linux lanes and Windows tasks 3–6 remain UNRUN.
Recommendation remains **no promotion on incomplete evidence**. No admission
expansion or product fix was attempted after the PDE stop.

## Requested controls — 2026-10-25

STATUS: CONTROL COMPLETE; broader campaign remains parked.

CODE READ HM1_CONTROL instruction (2026-10-25): run `defer-liveness` n=1
with the mirror binary OFF, then HP1 plain 6d, without changing the PTE-only guard.
The prior rejected PDE victim write belongs to `missing-page-fault`.
A fixture clarification was requested; these runs follow the explicitly named
`defer-liveness` fixture and do not substitute it for the missing-page fixture.

MEASURED HM1 control (rev `6564dd597`, `control/results.json`):
all three same-fixture outcomes follow. Mirror-capable rows use
`kf3-bins/6564dd59/qemu-system-x86_64`; the plain row uses
`kf3-bins/714ec29d/qemu-system-x86_64`, revision `714ec29d3`.

| defer-liveness arm (n=1) | Revision | Result | Kernel records |
|---|---|---|---|
| mirror-ON reference | 6564dd597 | PASS | 4 PTE reads, 3 VAs, 0 PDE |
| mirror-OFF control | 6564dd597 | PASS | 4 PTE reads, 3 VAs, 0 PDE |
| plain-6d control | 714ec29d3 | PASS | 4 PTE reads, 3 VAs, 0 PDE |

MEASURED HM1 control (`control/plans.json`, rev `6564dd597`): preregistration
preceded each run. The mirror switch is the only behavior delta for OFF;
the executable is the only further delta for plain 6d, with the inert OFF
environment retained. Kernel/initrd, budget, policy and guest arguments match;
argv parity is asserted before launch, allowing owned output identities to differ.
The guard remains byte-identical; 39 admission tests and two targeted guard tests
passed before the controls. No product edit or rebuild.

INFERRED from HM1 `control/results.json`: the stated absence-in-both-controls
falsifier is observed for the requested defer-liveness fixture. None of its
ON/OFF/plain arms emitted PDE. This does not discriminate the different
missing-page-fault fixture: its ON arm has one rejected PDE victim write,
and its OFF/plain arms remain UNRUN. No mirror-specific PDE finding follows.

MEASURED HM1 control cleanup (`control/final-audit.json`, 2026-10-25
18:21:11 UTC): no QEMU/controller, lease released, GPU idle, GDM active,
zero OOM, unchanged binary/policy/kernel/initrd identities, target still deleted.
The control journal contains eight admitted own-PID PTE read-fault records only.
No other campaign task ran. Recommendation remains no promotion on incomplete
evidence; the prior PDE admission question remains unresolved.
