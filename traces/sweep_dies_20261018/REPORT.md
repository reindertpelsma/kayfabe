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

ROUND 2 — 2026-10-22

1. CODE READ (2026-10-22): ROUND 2, laneS4, branch `codex/sweep-dies-20261022`; EXPECTED RED commit `3de4e17b02095869175cb6ed979a02b86450b580`; final publication head is the containing commit, verified on origin at closeout.
2. CODE READ (`round2/DERIVATION_BLOCKER.md`): asked to derive the fault client from public OGKM plus host engine facts, then resume Ampere and Turing; result BLOCKED at the public derivation, before any rental.
3. CODE READ (`scripts/bench/native_xid_guard.py:66,70,327`): CE1 literals found in native and guest admission; COPY0 defer controls intentionally provoke four read faults, plus one separate GR missing-page fault.
4. MEASURED (lease57209908, `native_controls/pce-mask-probe.log`): COPY0/LCE0 host mask0x10; four journal faults reportCE0; this observation cannot independently justify admitting that same journal.
5. CODE READ (`round2/sources.json`): public OGKM580.126.09,595.91.07,615.78.08 inspected, with immutable commits, archive hashes and CE/FIFO/UVM source hashes retained.
6. CODE READ (`round2/*.search.json`): C/H searches covered3694/3708/3914 files; no independent PCE-to-HUB-client rule was established; regex absence is not proof of global absence.
7. CODE READ (`round2/DERIVATION_BLOCKER.md`): missing fact is the public generation-specific bridge from GA102 backingPCE4 to its MMU HUB client; fault-name enums and LCE/PCE masks describe separate identifiers.
8. CODE READ (`round2/facts_not_admission.json`): evidence classes separated; no inferred admission table, CE0/CE1 allowlist, guard change, product change, timer change or baseline expansion.
9. MEASURED (`round2/round2_expected_red.log`):8tests,6PASS,2EXPECTED RED; authentic AD102 exact-five replayPASS; authentic GA102 rejected, while a CE1-substituted journal is admitted by the existing guard.
10. MEASURED (`round2/round2_test_*xid*.log`): existing native18/18,guest61/61,abort5/5PASS; extra-record, wrong-channel, wrong-VA and unrelated-Xid refusals retained; after-fix proofNOTDONE.
11. CODE READ (`round2/DERIVATION_BLOCKER.md`): AD104 paired replayNOTDONE, not relabelled from AD106; derivation tests/fix/mutantsNOTDONE; Rust changes0, added Rust lint sites0, clippyNOTRUN.
12. CODE READ (`round2/DERIVATION_BLOCKER.md`): Ampere Windows/bind/ON/appmatrix/Unigine/600-second hold and TuringNOTRUN; ROUND2 explicitly requires stopping when public derivation cannot be established.
13. INFERRED decision1: recommend obtaining the missing published PCE/client mapping before another lease; no further permission question or speculative mapping was substituted.
14. MEASURED (`round2/cleanup.json`): own ids57193877/57209908 API-absent; new leases0, Round2 rental cost$0; INFERRED conservative lane accounting$3.80362 remains below the Round2$8 stop.
15. MEASURED (`round2/cleanup.json`): owned helper jobs0,target absent, GPU/cargo locks acquired0; claims scoped to lane changes are checked before publication; no private RE or credentials used.

ROUND 3 — LIVE 2026-10-22 (supersedes preliminary coverage counts)

1. CODE READ (`round3/TIER_PLAN.md`): laneS4, branch `codex/sweep-dies-20261022`; execute tiers in order, cover20 consumer/workstation dies, no harvested-route table.
2. CODE READ (`round3/tier12_proof.json`): tier1 engine MMU fault ID and HUB CLIENT are independent packet fields; proposed equality contradicted.
3. CODE READ (`round3/tier12_proof.json`): negative control binds COPY0; CE1 originated with the AD106 fixture, not selection of a second engine.
4. CODE READ (`round3/tier12_proof.json`): tier2 authored3212-byte FIFO query is kernel-only; root userspace is USER_ROOT, not kernel; public table has no CLIENT field.
5. CODE READ (`round3/coverage.json`): independently derived runtime CLIENT admission0/20; canonical negative-control journal observations3/20 before the current TU106 probe.
6. CODE READ (`round3/coverage.json`): existing any-item constants14/20 (70%), inherited0; missingTU104/TU117/GA103/GA107/AD107/GB2076/20 (30%).
7. MEASURED (`round3/offers_audited.json`):12 current eligible-query offers/9 model classes; no eligible candidate for any of the six missing dies, price unavailable.
8. CODE READ (`round3/coverage.json`): explicit workstation/consumer/mobile PCI planning map; branch/vBIOS/ECC unverified; mutable masks/counts/routes reacquired from host.
9. CODE READ (`round3/reuse_provenance.json`): old nvkvm-pv planning/availability/known-bad inventory reviewed; reuse v3 provision_full/sweep/matrix_table; no old ABI oracle or PSK.
10. MEASURED (`round3/final_guard_green.log`, `round3/matrix_table_selftest.log`):117/117 guarded harness tests and9/9 reporter checks pass.
11. MEASURED (`round3/collector_mutants.json`, `round3/sweep_proof_mutants.json`, `round3/native_sweep_mutants.json`):9 new mutants each killed by at least2 tests; tests-first red commits precede implementations.
12. MEASURED (lease57262021, `lease_57262021/START.json`): own RTX2060S at$0.163333/h; auto-destroy170min armed first, pinned SSH, one lease; no new duplicate rentals planned.
13. MEASURED (lease57262021, `lease_57262021/native_deployment.json`): stock595.91.07 open module prepared; public full provisioning currently live; native engine/Xid observation pending.
14. MEASURED (`round3/pending_runtime_admission_red.log`): Round2 replay remains6/8 with2 expected nonpasses; Ampere/Turing Windows acceptance not resumed because derived-and-green admission remains unmet.
15. INFERRED (`round3/TIER4_RESIDUE.md`): owner decides separate narrow private investigation after legal review; recommend current-host routing mechanism, no incomplete production table and no RE in S4.
