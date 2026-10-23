MEASURED (2026-10-23, lease57296386, `lease_57296386/TEARDOWN.json`): exact-ID API absence verified02:04:33Z; owned auto-destroy cancelled afterward. INFERRED (same receipt): runtime estimate$0.348273, conservative$1.348273; lane total$8.071121<12, Round3$4.267503<8; invoice unverified.

MEASURED (2026-10-23, `adaptive/cleanup/cleanup_red.log`, `cleanup_green.log`): cleanup proof4tests,2expected failures before fix then4PASS. Interrupted graceful stop now still reaches bounded QMP quit for the same owned PID/socket; original arm failure remains. MEASURED (`adaptive/cleanup/cleanup_mutants.json`): removing fallback kills2tests; existing desktop suite74/74PASS including4new tests. CODE READ: product Rust/QEMU unchanged; added Rust lint sites0, fresh clippy not run.

ADAPTIVE GA102 — PARTIAL, 2026-10-23; supersedes earlier LIVE entries.

1. MEASURED (lease57296386, `START.json`): RTX3090/GA102, host595.91.07 OPEN, sourceac21aef93; quote$0.223333/h, auto170min armed first.
2. MEASURED (lease57296386, `native_complete/`): native raw30/30,CUDA4/4,GRtier,gates9+10,USER12/12 PASS; n1,spreadN/A.
3. MEASURED (lease57296386, `stopped_complete/s4_manual_stop_receipt.json`): final host journal matches exact frozen five native controls; no extra Xid.
4. MEASURED (lease57296386, `baseline_calibration/box_readiness.json`): stdVGA verification boot48s, actual OS query15.4196s, once-per-box factor8.
5. MEASURED (lease57296386, `bind_complete/bwr_bind.log`): adaptive SSH reply55.35s; signed580.88/32.0.15.8088, Code22, both GSPkeys1; frozen parent sealed01:39:44Z.
6. MEASURED (lease57296386, `stopped_complete/arm_ampere_s4_enable.txt`): single ON enable reached actual PnP/CIM Code0/OK at01:46:12.1289843Z.
7. MEASURED (lease57296386, `stopped_complete/arm_ampere_s4_diag_min_readiness.json`): next readonly actual-state query timed out twice240s,01:46:12.636–01:54:12.657Z; READINESS_INCONCLUSIVE.
8. MEASURED (lease57296386, `stopped_complete/s4_shell_pulse.txt`): plain shell pulse replied after21.36s; diagnostic timeout alone does not establish driver failure.
9. MEASURED (lease57296386, `stopped_complete/arm_ampere_s4_live_events.txt`): WATCHDOG file timestamp01:46:08.1838086Z; two WER LiveKernelEvent193/815 records at01:46:42Z; reason unresolved.
10. CODE READ (2026-10-23, [Microsoft 0x193 documentation](https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-0x193--video-dxgkrnl-livedump)): 0x193 identifies a graphics live dump; public reason815 semantics not established. No dump binary inspected.
11. MEASURED (lease57296386, `stopped_complete/s4_event_correlation.json`):3856 RPC rows; dump timestamp bracketed by observer ordinals3273/3274, last3855 at01:46:35.472539Z; causal RPC UNKNOWN.
12. MEASURED (lease57296386, `stopped_complete/arm_ampere_s4.log`): desktop/apps/Unigine/600s hold NOTRUN; no acceptance pass and no revised app oracle.
13. MEASURED (lease57296386, `stopped_complete/s4_manual_stop_receipt.json`): wrapper interrupted graceful stop; documented direct QMP quit succeeded02:03:15Z, owned QEMU exe absent.
14. CODE READ (2026-10-23): shared adaptive proof57/57+SW2policy21/21, existing desktop70/70 pass; ten mutants each>=2killers; Rust changes0/added lint sites0, fresh clippy not run.
15. CODE READ (2026-10-23): publish this evidence before exact-ID teardown/API absence, then Turing; remaining total budget is sufficient, no owner approval needed.

MEASURED LIVE UPDATE (2026-10-23, lease57296386, `lease_57296386/bind_complete/`): signed Windows580.88 binds as32.0.15.8088/Code22; both GSPkeys1, cold parent sealed and stopped01:39:44Z. New ON child reachedSSH, exactsource/flags receiptPASS; GPU enable/desktop/apps/hold still pending.

MEASURED LIVE UPDATE (2026-10-23, lease57296386, `lease_57296386/baseline_calibration/box_readiness.json`): nativePASS; generic Windows first installation boot exited0; verification boot48s and one OS CIM query15.4196s produced factor8. Baseline verification/bind/ON/apps/hold are still pending; no new driver verdict.

STATUS: LIVE — adaptive readiness resume, 2026-10-23.

CODE READ (2026-10-23, owner02:20 instruction): earlier GA102 bind readiness timeout is inconclusive about the driver. Shared SW2 policy4ede56ac7 is reused by scripts/bench/windows/readiness.py; GPU-free scaling/retry/ceiling/reserve proofs are green. New GA102 lease follows publication; no lease is active during this proof. See docs/design/S4_ADAPTIVE_READINESS_20261023.md and adaptive/ evidence.

STATUS: PARTIAL — S4 final update, 2026-10-22 23:58 UTC.

CODE READ (2026-10-22): branch `codex/sweep-dies-20261022`; this update supersedes earlier live/pending/stopped summaries below. The publication head is the containing commit, checked against origin after push; the external S4 report records its exact hash at closeout.

1. MEASURED (2026-10-22, `round3/all_resume_green.log`): DONE owner-approved exact channel/VA/type plus CE-family admission; authentic AD102 exact5 and GA102 fourCE+GR admit; wrong channel/VA/type, extra and non-CE refuse.
2. MEASURED (2026-10-22, `lease_57280778/native_complete/`): DONE new GA102/RTX3090 native gates9+10,USER12/12,raw30/30,CUDA4/4,GRtierPASS; host595.91.07 OPEN, runtime905a8bd75; n1,spreadN/A.
3. MEASURED (2026-10-22, `lease_57280778/bind_final/sweep_windows_install.log`): DONE generic Windows baseline/verification fail0, sealed23:28:57Z.
4. MEASURED (2026-10-22, `lease_57280778/bind_final/bwr_bind.log`, `lease_57280778/matrix_row.json`): FAIL GA102 bind readiness23:48:35Z, SSH banner timeout, zero driver RPC; GPU enable/desktop/apps/Unigine/600s hold NOTRUN.
5. MEASURED (2026-10-22, `lease_57280778/cleanup_final/`): DONE owned bind PID exited through QMP; final journal still matches frozen five-record native baseline, no new/unmatched Xid.
6. MEASURED (2026-10-22, `lease_57193877/ADA_RESULT.md`): earlier AD102580.88 Code0/1920x1080/Explorer/DWM and600s zero-TDR holdPASS; full Ada acceptance remains PARTIAL.
7. MEASURED (2026-10-22, `lease_57193877/final_stopped/matrix_ad102_s4_r4/win.res`): Ada18/112 rows:13PASS,4strictUnigineGPU-proofFAIL,1NVRTCNOTRUN,94unrun; no retrospective pass from later harness fixes.
8. MEASURED (2026-10-22, `round3/turing_availability/final_offer_census.json`): NOTRUN new Turing acceptance;0 eligible Turing KVM offers versus30 other KVM offers at23:57:27Z. No new Turing lease created.
9. CODE READ (2026-10-22, `round3/coverage.json`): main consumer/workstation die set20; existing any-item constants14, inherited0, unmeasured6=30%; harvest-dependent data stays runtime-only; datacenter separate/last.
10. MEASURED (2026-10-22, `round3/alloc_export_red.log`, `round3/alloc_export_green.log`, `round3/alloc_export_bundle_proof.json`): DONE missing run-kit metadata parser restored from our public e34886891 source, tests-first5RED→5GREEN and bundled SHA match; no decoder change or binary RE.
11. CODE READ (2026-10-22): unchanged product Rust/QEMU trees from rev170054339; no audit-fix branch, guest patch, timeout change or relaxed app oracle; Rust changes0, added Rust lint sites0, fresh clippy NOTRUN.
12. MEASURED (2026-10-22, `round3/final_cleanup.json`): DONE owned IDs57193877/57209908/57262021/57280778 API-absent; timers/helpers0; own target absent, large caches removed, no owned GPU/cargo lock held.
13. INFERRED (2026-10-22, same cleanup receipt): conservative lane$6.722848<12, Round3$2.919230<8; reserve$5 untouched; final invoice unverified.
14. INFERRED (2026-10-22): remaining work is GA102 pre-driver boot diagnosis and Turing hardware acceptance when a compliant offer appears; full app acceptance is not established on any requested family.
15. CODE READ (2026-10-22): no additional owner approval requested. Existing authorization suffices for the next compliant Turing run; the six unmeasured constant dies remain explicitly visible for the owner's broader coverage decision.

MEASURED (2026-10-22, named proof logs): prior combined guard/transport154/154 pass plus new export5/5 pass. These are separate executions; counts are not added to overlapping historical suites. Hardware counts are given above; no new FPS or percentile result is claimed.

| Mutation group | Kill counts per mutant | Evidence |
|---|---|---|
| Approved CE-family guard |3,2,5,5,5,3|`round3/owner_family_mutants.json`|
| Checked image transport |2,2,2,2,2|`round3/chunks_mutants.json`|
| Allocation export |3,2,2|`round3/alloc_export_mutants.json`|

INFERRED (2026-10-22): recommended next action1 is a bounded public-source investigation of GA102's pre-driver bind boot; next action2 is the already-authorized Turing acceptance run on an eligible KVM offer. Neither recommendation supplies an unverified product fix. Driver-bind cause, post-enable Windows behavior, complete app results, workstation branch/vBIOS/ECC equivalence, six missing die constants and billed cost remain unverified.

CODE READ (2026-10-22): historical reports follow, with their original timestamps and limitations.

---

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

OWNER-APPROVED RESUME — 2026-10-22

1. CODE READ (2026-10-22, `docs/design/S4_XID_ADMISSION_20261022.md`): owner replaces exact CE-instance derivation with exact source channel/VA/type plus CE-client family; no per-die table.
2. CODE READ (commit 597d60fe1): native and guest guards implement that rule; native GR/ESC control, guest PID/twins/recovery and immutable prior history stay exact.
3. MEASURED (2026-10-22, `round3/owner_family_red.log`): EXPECTED RED tests-only commit0ae9e6c35 ran21 tests with5 nonpasses before implementation.
4. MEASURED (2026-10-22, `round3/all_resume_green.log`):154/154 tests pass (138guard/replay +16transport); authentic AD102 exact-five and GA102 four-CE-plus-GR admit; wrong channel/VA/type, extra and non-CE refuse.
5. MEASURED (2026-10-22, `round3/owner_family_mutants.json`): six mutants killed by3/2/5/5/5/3 tests; source restored and138 green.
6. MEASURED (2026-10-22, `round3/pre_resume_guard_discovery.log`):127/127 tests matching the actual provisioning guard-discovery pattern pass.
7. CODE READ (commit 597d60fe1): no Rust/product change; added Rust lint sites0, clippy not rerun; historical tier/coverage facts remain distinct from approved admission policy.
8. MEASURED (2026-10-22, `lease_57262021/TEARDOWN.json`): TU106 preparation lease destroyed22:02:29Z/API-absent; timer cancelled afterward; no new GPU/engine/Windows test ran there.
9. INFERRED (2026-10-22, `lease_57262021/TEARDOWN.json`): TU106 runtime estimate$0.420407, conservative$1.420407; prior lane conservative total$5.224025 before resume.
10. MEASURED (2026-10-22, `lease_57280778/START.json`): new RTX3090 lease57280778 at$0.29/h,23GB advertised RAM, unverified provider; KVM180,170-minute auto-destroy armed first, source905a8bd75.
11. MEASURED (2026-10-22, `lease_57280778/START.json`): prior accounting plus full new exposure$7.045692 below$12; Round3 exposure$3.242074 below$8; reserve$5 untouched.
12. CODE READ (2026-10-22, `lease_57280778/harness_recipe/provision.sh`): first require actual KVM/device health and native exact-source baseline; then Windows580.88 bind/ON/desktop/appmatrix/Unigine/600-second hold.
13. MEASURED (2026-10-22, `lease_57280778/AMPERE_PROGRESS.md`): GA102 native30/30,CUDA4/4,GRtier,gates9+10PASS; approved native5 baseline frozen; standard-VGA baselinePASS; bind-readinessFAIL before driver RPC, apps/Unigine/hold NOTRUN. Turing follows teardown.
14. CODE READ (2026-10-22, `round3/coverage.json`): historical constants still14/20 dies, inherited0, missing6/20; this does not claim Windows acceptance or changed physical topology.
15. CODE READ (2026-10-22): current owned lease only57280778; its deadline00:58:58Z; public evidence before teardown and verified API absence remain required.

AMPERE RESUME RESULT — 2026-10-22

1. CODE READ (2026-10-22): S4 branch `codex/sweep-dies-20261022`, runtime source905a8bd75e0fc53f5494e45b6d2a0fee474d7757; requested Windows580.88 desktop/appmatrix/Unigine/600s hold after native gates.
2. MEASURED (2026-10-22, lease57280778, `native_gates/sweep_host_die.log`): RTX3090 is GA102, host595.91.07 OPEN; KVM present,8 CPUs,17949MiB actual RAM.
3. MEASURED (2026-10-22, lease57280778, `native_complete/sweep_host_preflight.log`, `native_complete/sweep_gates.log`): SOUND, gates9/9+gate10PASS, USER births12/12,0refused.
4. MEASURED (2026-10-22, lease57280778, `native_complete/sweep_bare595.log`): native raw30/30,0fail/0crash; CUDA4/4 and GRtierPASS; n1,spreadN/A.
5. MEASURED (2026-10-22, lease57280778, `native_complete/native_xid_baseline.json`, `native_controls/`): owner-approved fourCE0/clientCE0 source controls plus one GR/ESC control admitted exactly; channel/VA/type/count remain exact.
6. MEASURED (2026-10-22, lease57280778, `appdisk_receipt.json`, `parallel_transfer.log`): pinned app image6785255424 bytes and fullSHA verified; eight checked transport streams completed732.410s; NVRTC supplement also hash-verified on host.
7. MEASURED (2026-10-22, lease57280778, `bind_final/sweep_windows_install.log`): standard-VGA Windows installation/verificationPASS, WIN_CHECK_DONE fail0; base sealed23:28:57Z.
8. MEASURED (2026-10-22, lease57280778, `bind_final/bwr_bind.log`): display/GOP-on bind boot began23:28:59Z; readiness failed23:48:35Z with SSH banner timeout; controller exited1.
9. MEASURED (2026-10-22, lease57280778, `bind_final/arms/bw580_bind/summary.json`): RPC count0, no ordinal or control/class failure; GPU enable, Code0, Explorer/DWM acceptance, appmatrix, Unigine and600s hold NOTRUN.
10. MEASURED (2026-10-22, lease57280778, `BIND_BOOT_SNAPSHOT.json`, `BIND_BOOT_SNAPSHOT_NOTE.md`): QMP running, guest-ping timeout; only initial reset/start observed. Black/BootManager frames do not establish a reboot or TDR; exact cause unverified.
11. CODE READ (2026-10-22, `harness_recipe/`): normal19 production flags, pinned QEMU/source,8GiB guest RAM and framebuffer selected before boot; no timer, CPU, admission or runtime flag change to cure the failure.
12. MEASURED (2026-10-22, lease57280778, `bind_final/s4_final_native_check.log`, `bind_final/post_windows_kernel_snapshot.log`): post-bind journal still passes immutable native baseline; no new/unmatched Xid. No Windows event timestamps available before SSH.
13. MEASURED (2026-10-22, `../round3/all_resume_green.log`, `../round3/owner_family_mutants.json`, `../round3/chunks_mutants.json`):154/154 offline tests; six guard mutants killed3/2/5/5/5/3, five transport mutants each2; added Rust lint sites0, clippy not rerun.
14. INFERRED (2026-10-22): combined acceptance FAIL at pre-driver bind readiness, not evidence of an NVIDIA initialization defect; recommend a separate public-source boot/display investigation. Optional alloc-header exporter was absent; zero-RPC and channel-state metadata were exported without claiming allocation-body coverage.
15. CODE READ (2026-10-22): preserve this failed arm and evidence before exact-id teardown/API absence; Turing follows only after absence. No RE or secrets on the rental, no dev cargo target or shared-host GPU work.

MEASURED (2026-10-22, lease57280778, `lease_57280778/TEARDOWN.json`): exact-id teardown verified API absence23:52:10Z; auto-destroy cancelled afterward. INFERRED: runtime estimate$0.498823, conservative$1.498823; conservative lane total$6.722848, Round3$2.919230; Turing budget remains.

TURING RESUME AVAILABILITY — 2026-10-22

1. CODE READ (2026-10-22): requested Turing after Ampere, targeting TU106/TU104 consumer and mapped Quadro RTX workstation variants, one owned lease at a time.
2. MEASURED (2026-10-22, `turing_availability/no_offer_census.json`): no eligible offer at23:52:59Z; rental selector refused before creating a lease.
3. MEASURED (2026-10-22, `turing_availability/expanded_offer_census.json`): verified and unverified provider queries,180GB storage, Super/Ti and Quadro RTX naming variants returned0 KVM candidates at23:53:26Z.
4. MEASURED (2026-10-22, `turing_availability/availability_control_census.json`): diagnostic5GB-storage search returned28 Turing offers across consumer models; all28 advertised vms_enabled=false. These are not eligible180GB Windows quotes.
5. MEASURED (2026-10-22, same control census): KVM/single-GPU/180GB/price<=1 control query returned31 offers for other GPU families; API was returning marketplace data.
6. MEASURED (2026-10-22, `turing_availability/final_offer_census.json`): final23:57:27Z census returned30 KVM offers and0 Turing candidates; GTX16x names were included in this final check.
7. INFERRED (2026-10-22): Turing is unavailable under this run's KVM/storage/price requirements in these snapshots; this does not establish permanent unavailability or permission to rent non-KVM offers.
8. MEASURED (2026-10-22, `final_cleanup.json`): new Turing resume leases0, new Turing rental cost0; all earlier own lease IDs API-absent.
9. MEASURED (2026-10-22, `../lease_57262021/TEARDOWN.json`): earlier TU106 preparation rental was already destroyed22:02:29Z; no new native/engine/Windows acceptance probe ran there.
10. CODE READ (2026-10-22, `coverage.json`, constants audit): existing TU106 tables/captures are retained; unavailable acceptance testing is not a reason to repeat existing constant measurements.
11. CODE READ (2026-10-22, `coverage.json`): main target20 consumer/workstation dies; any-item constant receipts14, inherited0, unmeasured6 (TU104,TU117,GA103,GA107,AD107,GB207),30% of equal-weight die set.
12. CODE READ (2026-10-22, `TIER_PLAN.md`): workstation driver/vBIOS/ECC axes remain unverified; harvested masks/counts/routes are host runtime facts, never inherited die constants; datacenter remains separate/last.
13. CODE READ (2026-10-22): new Turing Windows580.88 bind/ON/desktop/appmatrix/Unigine/600s hold NOTRUN; hardware n0,spread unavailable; no synthetic PASS or failing RPC/event time claimed.
14. INFERRED (2026-10-22, `final_cleanup.json`): lane conservative total$6.722848<12 and Round3$2.919230<8; budget is not the blocker. Price guard$1/h and reserve$5 remain intact; invoice not verified.
15. INFERRED (2026-10-22): next required hardware action is Turing acceptance when a compliant offer is available; no RE is needed for the owner-approved CE-family admission, and no RE was done in S4.

ADAPTIVE GA102 — LIVE 2026-10-23

1. MEASURED (2026-10-23, `START.json`): lease57296386, RTX3090 candidate,$0.223333/h; auto170min armed first, deadline03:20:59Z; sourceac21aef93.
2. MEASURED (2026-10-23, `initial_inventory.txt`): pinned SSH, KVM present,23 CPUs,50367MiB RAM,175G disk/167G initially free.
3. MEASURED (2026-10-23, `native_gates/sweep_host_die.log`): RM arch0x170/impl2 confirmsGA102 on595.91.07.
4. MEASURED (2026-10-23, `native_gates/sweep_host_preflight.log`): host preflight SOUND; native driver open and device nodes usable.
5. MEASURED (2026-10-23, `native_gates/sweep_gates.log`): gates9/9+gate10PASS, USER12/12,0refused at00:51:28Z.
6. MEASURED (2026-10-23, `deployment_receipt.json`): controller1837 running, source frozenac21aef93,35-file archive/extracted allocation exporter hash verified.
7. MEASURED (2026-10-23, `parallel_transfer.log`, `appdisk_receipt.json`): pinned6.8GB app image complete140.121s/eight streams/fullSHA; NVRTC remote pin also verified.
8. MEASURED (2026-10-23, `native_gates/sweep_windows_media.log`): all four original Windows media pins pass; media helper exited0 at00:48:47Z.
9. CODE READ (2026-10-23, `harness_recipe/`): native raw/CUDA/GR baseline next, then generic Windows baseline and once-per-box calibration before bind.
10. CODE READ (2026-10-23, commitac21aef93): common Windows adapter reuses SW2 policy4ede56ac7; measured4–10x factor, ceilings, persisted stage and export reserve; side effects single attempt.
11. MEASURED (2026-10-23, `../adaptive/green.log`, `../adaptive/sw2_shared_green.log`): adaptive57/57 and shared21/21 pass; prior desktop70/70 pass.
12. MEASURED (2026-10-23, `../adaptive/mutants.json`): ten mutants killed15/6/10/6/9/2/2/3/3/2; source restored; no Rust change/added lint site, fresh clippy not run.
13. CODE READ (2026-10-23): Windows580.88 bind/ON/desktop/apps/Unigine/600s hold pending; no driver verdict is implied by earlier fixed-bound failure.
14. INFERRED (2026-10-23, `START.json`): conservative prior$6.722848 plus full new allowance$1.632778=$8.355626<12; Round3 exposure$4.552008<8; reserve$5 untouched.
15. CODE READ (2026-10-23): only own lease57296386 active; preserve bounded evidence before exact-id teardown/API absence, then Turing if a compliant offer is available.

MEASURED (2026-10-23, lease57296386, `lease_57296386/native_complete/`): fresh GA102 native raw30/30,CUDA4/4,GRtier and9+10 gatesPASS; exactly fourCE/clientCE0 source controls plus oneGR/ESC control frozen. Generic Windows installation began01:00:33Z; adaptive box calibration and bind/ON/apps/hold remain pending.
