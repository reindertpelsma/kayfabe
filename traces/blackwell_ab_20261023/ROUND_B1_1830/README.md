# BW3 round B1 closeout — OBSERVATION, 2026-10-23

1. Lane and requested work

BW3, branch `codex/bw-native-edge-probe-20261022`. The controller18:30 round grant allowed one RTX50 lease, stop2.50USD/cap3.00USD/reserve8.00USD, timer armed before create, B hold/collection and optional B1. The controller17:00 decision resolved the admission scope: exact owned-QEMU Xid32 channels0x14/0x1000000b, intr00200000, max8 per arm, in a separate lease-only OBSERVATION mode; it never grants PASS or attestation. Native/production/CI exact-VA/type/client rules remain unchanged. Evidence is in this directory. Origin remains160dc031ad51cda74ac3848f33386237fcd8c2d4; later lane commits are held locally pending the unchanged claims gate. No merge, force-push or second lease.

2. Result (<=15 lines)

- DONE — MEASURED lease57388945 on 2026-10-23, `market.json`/`owned_instances.json`: one market snapshot selected provider-verified offer50694293/machine45428, RTX5060Ti advertisedGB206, QuebecCA. Bad-listed machines and denied countries were excluded. The conservative UNVERIFIED-HOST artifact label denotes untrusted experiment evidence; the provider's verified status is retained separately. One lease, zero provider retries.
- DONE — MEASURED lease57388945 on 2026-10-23, `precreate_watch_ready.json`: watchdog1519909 was ARMED at15:25:27.347817UTC before create15:25:28.607553UTC. Backup1519916 then signalled ready. Fixeddeadline17:55:27.347817UTC was not extended. Quote0.297981481USD/h, actual0.320666667USD/h; guarded reservation1.494953704USD.
- DONE — MEASURED lease57388945 on 2026-10-23, `rehearsal/receipt.json`/`ssh_identity.json`: final operator receipt replay accepted / accepted after20s / refused before SSH. New-round spend fixture0USD replaced the older phase fixture5.2USD. Console identity matched at15:34:02.730649UTC; disk167GB free and KVM checks passed; all four media size/SHA checks passed before controller4566 started.
- DONE — CODE READ `scripts/bench/blackwell/saved_xid32_observation.py`: separate named mode enforces the exact pair/intr/code, owned PID life/device, expiry, one-enable arms and8-record ceiling. Records include untruncated raw lines and OBSERVATION/no-PASS/no-attestation fields; surrounding kernel rows and full dmesg snapshots are retained. Existing observation/default/native guards were not relaxed.
- DONE — MEASURED lease57388945 on 2026-10-23, `python310_compatibility.json`: hostPython3.10.12 rejected compact kernel timezone+0000. Two regression cases and a two-killer mutant cover the correction to the existing guard's strptime syntax. Two of87 source hashes changed after SSH, before controller/observation-watcher/Windows-ON start; this supersedes the earlier source-freeze statement for that codec correction. Actual-host CPU fixtures passed2; admission policy and production guards were unchanged. These CPU fixtures are not live Xid observations.
- DONE — MEASURED lease57388945 on 2026-10-23, `lease_metadata/sweep_host_die.log`/`sweep_gates.log`: unprivileged probeGB206, driver595.91.07 open; gates9/9 plus gate10 and12/12 USER births passed.
- FAIL — MEASURED lease57388945 on 2026-10-23, `native_failure/capc_native595_concurrent-fuzz.log`: actual concurrent-fuzz childrc1, one ALIAS_REVOKED invariant failure in the9-worker crowd arm pinned to3 cores. Unpinned3-worker, per-core3-worker and same-width unpinned9-worker controls passed. This is one invocation, not a repeatability or causal finding.
- STOPPED — MEASURED lease57388945 on 2026-10-23, `lease_metadata/XID_STOP`/`native_failure/sweep_bare595.log`: the native wrapper recorded the real failure and returned7. Two later cells (cross-client-leak/rpc-mixed-allocs) were blocked before launching their clients. The suite reports27PASS/3FAIL/0CRASH/30 arms, representing one executed failure and two blocked cells, not three independent failures.
- DONE — MEASURED lease57388945 on 2026-10-23, `native_failure/kernel_full.txt`/`dmesg_full.txt`: full saved kernel/dmesg context exported. Exactly five intentional native Xid31 controls were recorded; no Xid32 occurred and no saved-pair observation was admitted. There was no GPU query or guest boot after fatal.
- UNRUN — MEASURED lease57388945 on 2026-10-23, `outcome.json`: CUDA/GR-tier validation, Windows install/bind, B/B1, page row, Code0 hold, guest153/4101/41/1001 events, dump header, TDR verdict and Xid32 channel decode. The native precondition stopped the run before any Windows arm.
- DONE — MEASURED lease57388945 on 2026-10-23, `closure.json`/`COST_FINAL.json`: exact-id APIabsence16:02:27.179152UTC preceded timer cancellation; both timerPIDs gone, all20 known ownids absent. Runtime estimate0.197617214USD<stop2.50/cap3; observed shared credit28.334037805USD>reserve8. Credit movement is not exclusive lane billing. Historical prior-round estimate5.759571019USD is separate from this new round.

| Arm | Die | OS | Driver | Rev | Flags | Result | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Native raw | GB206 | Ubuntu22.04 | 595.91.07 open | 6e3fd3fd6 | native suite | FAIL | native_failure/capc_native595_concurrent-fuzz.log |
| B | GB206 host | LTSC2024 planned, uninstalled | 580.88 planned | 48b027267 | p19, one enable planned | UNRUN | lease_metadata/XID_STOP |
| B1 | GB206 host | LTSC2024 planned, uninstalled | 580.88 planned | cb80cbd29 | p19, one enable planned | UNRUN | lease_metadata/XID_STOP |

Xid list: five source-marked intentional31s, all retained in `outcome.json` and the full raw kernel/dmesg exports. Four COPY-read controls use CE0/HUBCLIENT_CE1, exact PTE/virtual-read addresses0x60/0x62/0x64_00010000 and source channels2/3; the missing-page GRAPHICS semaphore-write control uses GR_HOST0/HUBCLIENT_ESC0, channel3, PDE/virtual-write0xa_11000000. Xid32 list=[], admitted countB=0/B1=0. These native controls are distinct from the requested saved32 observation mode.

3. Tests and mutations

MEASURED preparation 2026-10-23, `helpers_after_python310.txt`: helper suite255 before this round,337 after. New coverage:18 cost-plan,19 pre-create watchdog,41 saved-signature observation/context/compatibility,4 hold-trigger cases. Final operator rehearsal3 passed separately. Tests-only EXPECTED RED commits precede the cost planner, watchdog, slow-provider correction, new mode, enable-Code0 hold correction and Python3.10 codec correction; the compatibility fixture's helper reference was corrected in a following test commit. Reverting the codec still fails both compatibility regressions. Black100, source line bound, whitespace and staged-secret checks pass. New33 mutants each have>=2 killers. Existing guard tests remain in the337 count; no hardware attestation follows from these CPU tests.

| Mutant | Distinct test killers |
| --- | ---: |
| runtime_ceiling_extended | 6 |
| cost_duration_removed | 4 |
| fee_allowance_removed | 6 |
| account_reserve_removed | 2 |
| uncertainty_margin_removed | 5 |
| teardown_margin_removed | 2 |
| stop_raised | 3 |
| price_limit_raised | 2 |
| readiness_restarts_clock | 2 |
| fixed_clock_validation_removed | 2 |
| ready_after_ledger | 2 |
| provider_call_before_binding | 5 |
| fixed_deadline_changed | 2 |
| slow_query_recheck_removed | 2 |
| rate_bound_removed | 2 |
| reserve_removed | 2 |
| destroys_another_id | 17 |
| destroy_ack_is_absence | 2 |
| closed_row_hides_live_id | 2 |
| teardown_retry_removed | 2 |
| ninth_record_admitted | 2 |
| other_channel_admitted | 4 |
| other_xid_number_admitted | 5 |
| foreign_pid_admitted | 2 |
| different_intr_admitted | 2 |
| observation_flag_ignored | 2 |
| attestation_permission_ignored | 2 |
| raw_line_truncated | 2 |
| records_marked_pass | 2 |
| records_attest | 3 |
| enable_observation_lost | 2 |
| nonzero_status_admitted | 3 |
| compact_offset_iso_parser_restored | 2 |

4. Decisions

1. Investigate/reproduce the native concurrent-fuzz crowd ALIAS_REVOKED failure on declared source6e3fd3fd6 before another Windows observation lease; recommendation: keep the native gate and compare the same-width unpinned control, without changing timers or masking the invariant.
2. If hardware work resumes, issue a fresh lease grant; recommendation: retain exact saved32 OBSERVATION scope, full context capture and non-attestation labels. The one lease for this round is exhausted.
3. Resolve the inherited claims diagnostics before publishing this held lane head; recommendation: preserve the global new=0 gate and repair attribution on the appropriate parent line.

5. Unverified items

INFERRED lease57388945: the pinned crowd result does not establish why the alias probe failed, prove a hardware/provider fault, or establish repeatability. The log directly records VA_A landing before VA_B mapping and a later Lost probe; root cause is unresolved. No Windows boot or Xid32 exists in this round, so page sufficiency/inheritance, channel32 VA/type/client decode, guest survival, fault containment, B1 RC reporting and TDR remain untested. No post-fatal host-health query was performed. The last host-driver validation preceded the native failure.

FALSIFIER recorded before rental in `owner_authority.json`/`run_kit/manifest.json`: a guest TDR or bugcheck within10 minutes after the admitted exact pair refutes survival/containment for the actual die. No admitted fault leaves containment untested. This falsifier was not exercised. No new GB202 or inherited-page result exists.

6. Cleanup and publication

MEASURED lease57388945 on 2026-10-23, `closure.json`: APIabsence verified before cancelling the pre-create/backup timers; both gone, all20 known ownids absent, no active local operator/workload/SSH/build process. Rental QEMU/controller/builds ended with destruction. No local Cargo target was created and no local GPU lock was taken; no foreign resource was changed. Raw private receipts remain in the operator workspace; small text evidence and original/exported source hashes are retained here. Publication checks run after live collection/destruction; see `claims.json`. No checker/debt baseline or admission rule was waived.
