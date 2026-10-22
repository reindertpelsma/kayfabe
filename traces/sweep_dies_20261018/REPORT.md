1. CODE READ (2026-10-22): Lane S4; branch `codex/sweep-dies-20261022`; publication head = containing commit (verified against origin after push). Asked: Ada, Ampere, then Turing Windows580.88 desktop, APP MATRIX, Unigine and600-second zero-TDR hold, with$12 budget. The lane stopped on exact native Xid admission, per the owner safety rule.

2. Result (2026-10-22):

- MEASURED (`lease_57193877/ADA_RESULT.md`): Ada AD102 native gates9+10,raw30/30,CUDA4/4,GRPASS; Windows580.88 Code0/1920x1080/Explorer/DWM and600-second zero-NVIDIA153/Display4101/bugcheck hold PASS; n=1lease,5children,spreadN/A.
- MEASURED (`lease_57193877/final_stopped/matrix_ad102_s4_r4/win.res`): Ada matrix PARTIAL/overallFAIL:18/112 rows recorded,13PASS,4UnigineGPU-proofFAIL,1NVRTCstagingNOTRUN;94unrun. All four Unigine paths survived45s and timed D3D11 frames show scene motion, but the strict GPU oracle did not pass.
- MEASURED (`lease_57193877/first_desktop_event_correlation.json`): first Ada child produced an unclassified graphics LiveKernelEvent193/reason815; later clean hold does not establish zero anomalies across all boots. Blender RADAR_PRE_LEAK_64 WER was retained separately.
- MEASURED (`lease_57209908/AMPERE_RESULT.md`): Ampere GA102 native gates9+10,raw30/30,CUDA4/4,GRPASS; n=1,spreadN/A. AdmissionFAIL before Windows: four source CE faults reportHUBCLIENT_CE0 while the guard expectsHUBCLIENT_CE1; five totalXid31records, no extra records.
- MEASURED (`lease_57209908/XID_SAFETY_STOP.json`): first unmatched event12:53:05.310303UTC, channel4/VA0x6000010000, nativeDOORBELL-STORE#1; relayRPCordinalN/A because no VM started. Baseline was not accepted or enlarged.
- CODE READ: Ampere Windows/bind/ON/matrix/Unigine/hold NOTRUN; Turing NOTDONE, no lease created. Further GPU work stopped rather than relaxing the exact route proof.
- CODE READ: harness changes cover sign-in attribution/diagnostics, Kernel-Boot153 classification, media delivery/integrity, zombie census, Python3.10 hashing, optional pinned NVRTC supplement, and honest budget/hold handling. Product crates/qemu/tools/fastguest trees remain unchanged from rev170054339; no audit-fix branch was added.

3. Tests and mutation evidence:

MEASURED (2026-10-22, `final_harness_green.log`):80/80 S4 tests pass,0skips. MEASURED (`appmatrix_tests.log`): existing unchanged app-matrix76/76 pass. Hardware counts are separate from these fixtures; no broker frame-rate benchmark or percentile claim is made.

MEASURED (2026-10-22, the named expected-red/green logs): representative before/after counts below. Suites overlap; the final80 is the distinct combined count.

| Proof | Before fix | After fix | Evidence |
|---|---:|---:|---|
| Provider classification | 2/13 failing |13/13 pass|provider_expected_red.log / provider_green.log|
| Attempt admission |2/4 failing|4/4 pass|admission_expected_red.log / admission_green.log|
| Process census |2/4 failing|6/6 pass after added live-refusal cases|process_expected_red.log / process_green.log|
| Safe sign-in diagnostics |2/2 failing|2/2 pass|signin_diag_expected_red.log / signin_diag_green.log|
| Reserved hold |3/3 nonpassing|3/3 pass;55 desktop tests green|budget_expected_red.log / budget_green.log|
| Affordable later rows |6/6 nonpassing|6/6 pass;9 budget tests green|budget_selection_expected_red.log / budget_selection_green.log|
| Pinned NVRTC stage |5/5 nonpassing|5/5 pass|nvrtc_expected_red.log / nvrtc_green.log|
| NVRTC bootstrap |4/20 nonpassing|20/20 pass|nvrtc_bootstrap_expected_red.log / nvrtc_bootstrap_green.log|

MEASURED (2026-10-22, `mutation_summary.json` and category JSON files):26 accepted deliberately broken variants each have at least2 distinct killing tests. The initial one-kill development proof is retained as rejected and superseded; the earlier boot-epoch rule was superseded by provider-aware classification.

| Mutation group | Accepted variants | Minimum distinct killers | Evidence |
|---|---:|---:|---|
| Health/sign-in/provider |7|2|health_mutations.json, provider_mutations.json, signin_diag_mutations.json|
| App plan |5|2|plan_mutations.json|
| Controller |2|2|controller_mutations.json|
| Mount/integrity |3|2|mount_mutations.json|
| Process census |2|2|process_mutations.json|
| USB controller |1|3|usb_mutations.json|
| Python3.10 asset guard |1|2|asset_mutations.json|
| Attempt admission |1|2|admission_mutations.json|
| Budget/hold |2|2|budget_mutations.json, budget_selection_mutations.json|
| NVRTC |2|2|nvrtc_mutations.json|

4. Owner decisions:

1. INFERRED recommendation: establish an independently sourced LCE/PCE-to-HUB-client route proof before accepting the GA102 records; retain fail-closed admission and avoid a blanket CE0/CE1 allowlist.
2. INFERRED recommendation: resume Ampere Windows and Turing only after that safety proof is resolved; retain the original budget and price guards.

5. CODE READ: unverified work includes full Ada112-row acceptance, an accepted Unigine hardware-execution oracle, Ampere Windows, all Turing tests, and real-Windows execution of the new NVRTC/budget repairs. Linux guest-suite results were not rerun in S4; native per-family results are the measurements above. Long mixed-workload stability and broker new-content frame metrics remain unmeasured. Large rental logs were ephemeral; bounded text/PNG, native control transcripts, raw-source hashes and event/RPC brackets are preserved.

6. MEASURED (2026-10-22, `CLEANUP.json`, both `TEARDOWN.json` files): owned leases57193877 and57209908 are API-absent; timers cancelled only after absence. No owned background transfer/controller jobs remain. No dev cargo target or GPU/cargo lock was used; target path is absent and both large owned asset caches were removed. INFERRED: rate-times-runtime estimate$1.80362; conservative lane accounting$3.80362 including transfer allowances, below$12; billed cost was not independently verified.
