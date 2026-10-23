# BW3 round B1 preparation, 2026-10-23

1. Lane and requested work

BW3, branch `codex/bw-native-edge-probe-20261022`. The controller18:30 instruction authorizes one RTX50 lease, round stop2.50USD/cap3.00USD/reserve8.00USD, hourly price<=2.50USD, denied-country exclusions, timer armed before create, then B hold/collection and optional B1. Code head ccdb9c82206982c3f56bf8549d60ec46a2d424dd contains the prepared budget support. Origin remains160dc031ad51cda74ac3848f33386237fcd8c2d4. No merge or push was performed. No market query or rental has occurred in this round.

2. Result (UNVERIFIED-HOST grant; no current host)

- DONE — CODE READ `ROUND_B1.md`: new one-lease stop2.50/cap3/reserve8 instructions read; previous phase estimate5.759571019USD remains historical, not a new-round allowance.
- DONE — MEASURED preparation2026-10-23, `helper_tests.txt`: baseline255 helper cases, then292 pass after18 budget-planning and19 watchdog cases. Tests-only commits precede each implementation/correction.
- DONE — MEASURED preparation2026-10-23, `budget_mutations.json` and `watchdog_mutations.json`:20 new mutants each have>=2 distinct test killers; formatting, source100-column bound, whitespace and staged-secret checks pass.
- DONE — CODE READ `scripts/bench/blackwell/round_budget.py`: fixed pre-allocation deadline,150-minute ceiling, price bound,0.50USD uncertainty,60s teardown margin and account reserve. `precreate_watchdog.py` signals armed before contract publication, binds one recorded id, checks actual rate/credit and verifies exact-id APIabsence after destroy. The deadline is rechecked after a provider response. Neither a watchdog nor a rental was launched; rental-path integration remains pending.
- DONE — MEASURED preparation2026-10-23, `preparation.json`: four official cached media files match original size/SHA pins; shared credit28.804882486USD at preflight; all19 prior ownids API-absent. New-round createdids=[], destroyedids=[], runtimecost0USD.
- WAITING — CODE READ `ROUND_B1.md`: its exactchannel+VA+faulttype+CE-client instruction cannot match the saved Xid32 records, which have only code/PID/channel/intr. `../UNVERIFIED-HOST_1345/lease_metadata/kernel_xids.json` records the previous lease57366202 pair0x14/0x1000000b, intr00200000. No VA, faulttype or client was recorded for either32.
- DONE — CODE READ `scripts/bench/blackwell/phase_xid_guard.py`: the source-marked native Xid31 controls already enforce exact channel/VA/fault/access/PID/device/recovery and operation-appropriate engine/client family. They are distinct from the QEMU Xid32 observer. Native controls and the Xid32 guard were not changed.
- NOT DONE — preparation2026-10-23: no new host, native suite, Windows installation, B/B1 ON arm,10-minute hold, guest events, dump header or TDR verdict. No page/inheritance/containment outcome exists for this round.

| Arm | Die | OS | Guest driver | Rev | Flags | Verdict | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| B | unassigned | LTSC2024 planned | 580.88 planned | 48b027267 | existing p19, one enable planned | UNRUN | preparation.json |
| B1 | unassigned | LTSC2024 planned | 580.88 planned | cb80cbd29 | existing p19, one enable planned | UNRUN | preparation.json |

Xid list for this round: none (no lease). The prior pair is quoted only to identify the unresolved admission scope. No record is silently excluded and no VA/type/client metadata is invented.

3. Tests and mutations

MEASURED preparation2026-10-23: budget EXPECTED RED baf0164b4 had14 failures before implementation c039a073c; final18 cases pass. Watchdog EXPECTED RED07c9d0a62 had12 failures before implementation f029a6545; slow-response EXPECTED RED f4ff871f0 had2 failures before ccdb9c822 rechecked the deadline after the provider response; final19 cases pass. Full helper suite255 before/292 after. No hardware sample (n=0), spread or performance result.

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

4. Required clarification

1. Does the owner intend exact owned-QEMU Xid32 signatures0x14/0x1000000b with intr00200000, max8 per arm and all other guards retained, or require VA/type/CE-client proof before admitting them? Recommendation: explicitly scope the32 observation signature; do not infer missing MMU/client fields or broaden the guard from a Xid31 rule. The asynchronous question remains pending.

The controlling COMMON.md instruction is: "If something contradicts the code or these instructions, STOP and write down options; never improvise a design change." ROUND_B1.md says "exact channel + exact VA + exact fault type admitted; client only required to be in the copy-engine family". The preserved32 records lack those fields, so rental waits for this clarification; the one lease remains unused. Publication checks did not prevent hardware work.

5. Unverified items and falsifier

All new hardware observations remain unrun. FALSIFIER recorded before any later rental: guest TDR or bugcheck within the10-minute hold after an admitted pair refutes survival/containment for that actual die. No admitted fault means containment remains untested. Exact32 channel identity and CE-family attribution remain unresolved from the previous trace. The cached media pins do not establish a current host identity or readiness.

6. Cleanup and publication

MEASURED preparation2026-10-23: no new instance or active timer, no own workload/build/SSH process, no local Cargo target created, no GPU lock taken. Prior19 own instances remain absent. All work is local, append-only on the lane branch; native/observation rules remain unchanged. Current support is not wired into an allocation while the admission decision is pending. No claims gate was run to stop a live experiment; the inherited814-diagnostic blocker was documented by the previous closeout and remains unchanged. Before any eventual push, the full global claims new=0 requirement still applies; neither checker nor debt baseline was modified.
