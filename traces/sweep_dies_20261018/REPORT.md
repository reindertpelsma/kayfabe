S4 — PARTIAL, ROUND4 SAFETY STOP CLOSED, 2026-10-23 16:53 UTC.

1. Lane S4; branch `codex/sweep-dies-20261022`. CODE READ (owner ROUND4): retry GA102 Windows acceptance/600s hold/event-RPC correlation, then other Ampere (native GA104 Windows binding first) and Turing; cap3USD/stop2.50USD, one box, prearmed destroy,170min maximum. The publication head is the containing commit, verified against origin after push; the external S4 report records its exact hash. This report supersedes earlier LIVE summaries.

2. Results (15 lines maximum):

1. MEASURED (2026-10-23, `lease_57384343/TEARDOWN.json`): initial provider closed14:51:09Z before authenticated SSH/GPU/Windows; zero driverRPC,0.281506USD conservative cost.
2. MEASURED (2026-10-23, `lease_57385378/native_complete/`): DONE actual GA102595.91.07 OPEN, hostSOUND, gates9/9+10PASS,USER12/12,raw30/30,CUDA4/4,GRtierPASS; n1,spreadN/A.
3. MEASURED (2026-10-23, `lease_57385378/baseline_calibration/`): DONE generic Windows install/check/seal16:13:52Z; actualboot173s/query31.815s/factor10, same shared readiness policy.
4. MEASURED (2026-10-23, `lease_57385378/bind_inconclusive/bwr_install_readiness.json`): PARTIAL disabled bind; one install hit1200s absolute ceiling16:40:09Z, rc75 READINESS_INCONCLUSIVE; no installer replay.
5. MEASURED (2026-10-23, `lease_57385378/bind_inconclusive/arms/bw580_bind/summary.json`): driverRPC0; failing RPC/ordinal none; no actual ON/Code22/Code0/desktop/apps/Unigine/600s hold this round. No driver-failure or stability verdict.
6. MEASURED (2026-10-23, `lease_57385378/bind_inconclusive/bwr_setupapi_progress.txt`, `bwr_final_windows_events.txt`): file copy/catalog validation active before ceiling; only Kernel-Boot153/VBS records returned, including a future guest timestamp. Not nvlddmkm153/TDR evidence.
7. MEASURED (2026-10-23, `lease_57385378/CLEANUP.json`, `TEARDOWN.json`): DONE exact owned VM stopped16:44:41Z, final Xid guard unchanged five controls; lease API-absent16:46:12Z, timer cancelled afterward.
8. MEASURED (2026-10-23, `lease_57398382/PRE_TEARDOWN.md`, `START_SAFETY.json`): STOP real lease-accounting event: create reply failed exact-ID guard, but one new owned request-label instance existed. Ownership matched machine/GPU/time; exact ID registered with prearmed reaper, deadline retained.
9. MEASURED (2026-10-23, `lease_57398382/TEARDOWN.json`): DONE safety containment: no SSH/provisioning/probes/VM/GPU work, API-absent16:50:03Z, timer cancelled afterward; no further rentals/GPU work.
10. MEASURED (2026-10-23, `round4/offers.json`): GA104 native binding NOTRUN, no candidate class in latest39 KVM offers; TU106/GA106 candidates existed, their acceptance remains UNRUN after safety stop.
11. MEASURED (2026-10-22, `lease_57193877/ADA_RESULT.md`): prior AD102 desktop/600s holdPASS, full acceptance PARTIAL18/112:13PASS/4strictUnigineGPU-proofFAIL/1NOTRUN/94unrun.
12. CODE READ (2026-10-23, `round3/coverage.json`): existing any-item constant receipts14/20, inherited0, missing6/20=30%; no new constant die coverage, no production per-die CLIENT table, no RE.
13. INFERRED (2026-10-23, `round4/final_cleanup.json`): Round4 conservative1.574404USD<stop2.50/cap3; lane9.645524USD<12. Original transfer allowances retained even for unused boxes; estimates are not invoices.
14. MEASURED (2026-10-23, `round4/account_credit.json`, `round4/final_cleanup.json`): shared credit28.111147USD>reserve5; all eight own IDs API-absent, timers/helpers0, own target absent,7.10GB caches removed.
15. CODE READ (2026-10-23, `round4/proof/`, `lease_57385378/signin_overlay_receipt.json`): tests-first prearm/hold-first/provider/sign-in fixes completed; only operator harness changed, GPU source98b599bbf/product trees unchanged; no audit-fix merge.

3. Proofs (separate, overlapping executions are not summed):

| Proof | Before / after | Distinct killing tests per mutant | Evidence |
|---|---|---|---|
| Round4 lease guard + optional first hold | EXPECTED RED2c118854b/ea42930ca/38657b884; lease22/22,desktop77/77 |2,2,3,3,2,3|`round4/proof/lease_guard_*`, `hold_first_*`, `mutants.json`|
| Provider direct/relay endpoint | EXPECTED REDb79170c00; new8+existing8=16/16 |4,2|`round4/proof/endpoint_*`|
| Sign-in shared observer/action bounds | EXPECTED RED589e95c32/3afebf561;13new+6existing=19/19; desktop77/77 |3,2,2,2|`round4/proof/signin_*`, `desktop_after_signin_green.log`|
| Existing shared readiness | EXPECTED RED20db1f7bb/1a33267d1; S457/57,SW221/21 |15,6,10,6,9,2,2,3,3,2|`adaptive/green.log`, `sw2_shared_green.log`, `mutants.json`|
| Existing owned-VM cleanup fallback |4tests/2RED to4/4; desktop74/74 |2|`adaptive/cleanup/`|
| Existing CE-family exact fault guard | EXPECTED RED to138/138 |3,2,5,5,5,3|`round3/owner_family_*`|

MEASURED (2026-10-23, named proof files above): all listed fixed-code suites passed and mutants killed by at least two distinct methods; source restored. CODE READ (2026-10-23, source98b599bbf and harness37c5abdc4): product Rust changes0, added Rust lint sites0; fresh clippy NOTRUN. Per-push secret scans and claims on S4 changed files are required; no debt entries added. Historical inherited claims debt is excluded only under the owner's explicit evidence-branch ruling; publication heads carry skip-ci.

MEASURED (2026-10-23, `lease_57385378/NATIVE_BINARY_IDENTITY.json`): actual QEMU52db5101cac76574e1c59e63362489327678abb1af3cc805ec7a9bba1cbc4744/raw d971aab3ee9fd810325b5db8f6c99170a2f9d2d4dd816c89fd693fe4559474f6/guard2e7a0c25dd07442bfe0b20519c1493ecc760fb4a0dfdb39d07d72c4711fcd490. CODE READ (same receipt): product crates treefeda030ab2b0dd7a9aad43bcd163d9ea7afe0842/QEMU treefa8938d9c1a4b2caeaeb6f15a7daf96ed1e2f190 equal rev170054339;37c5abdc4 sign-in overlay affects only Python/shell observer code and was applied before any ON marker. No guest TDR/product timer/app-oracle changes.

4. Owner decisions:

1. INFERRED (2026-10-23, `lease_57398382/PRE_TEARDOWN.md`): before further leases, recommend durably recording the safely projected create reply before validation and testing exact-ID registration/refusal against an observed response; do not weaken the guard. The original response was lost, so its exact malformed type/cause is unknown. No live retry was used to investigate it.
2. INFERRED (2026-10-23, `round4/GA104_NATIVE_PLAN.md`, `NATIVE_ONLY_PLAN.md`): after the accounting issue is resolved, resume a compliant GA104 native Windows580.88 binding first when offered, then bounded remaining Ampere/Turing work; full app/Unigine/hold acceptance is still required.
3. CODE READ (2026-10-23, `round3/coverage.json`, `round4/offers.json`): owner may decide a separate RE lane from the explicit residue:20 target consumer/workstation dies,14 any-item measured,0 inherited,6 unmeasured;0/6 missing-die candidate classes rentable in the latest bounded snapshot.30% is an equal-weight die fraction, not market sales/share. RE was not performed here.

5. Unverified:

CODE READ (2026-10-23, cited run receipts): GA102 post-enable stability/600s hold and full app/Unigine matrix; native GA104 binding; current GA106/Turing acceptance; six missing-die constants; completeness for every item/driver; workstation branch/vBIOS/ECC equivalence; broker new-content metrics; the earlier193/815 live-dump cause; provider create-response cause; final invoices. Driver installation timeout alone does not establish a driver fault. The earlier GA102 sourceac21aef93 run reached Code0 but then had two240s diagnostic timeouts and unresolved193/815 reports (`lease_57296386/stopped_complete/`);3856 RPCs were retained with no causal RPC established. This round did not reach ON or reproduce that observation.

CODE READ (2026-10-23, `round3/coverage.json`, `round3/TIER_PLAN.md`): main target20 is TU102/104/106/116/117,GA102/103/104/106/107,AD102/103/104/106/107,GB202/203/205/206/207; missingTU104,TU117,GA103,GA107,AD107,GB207. Workstation SKUs map to their die, but a measured die only covers floorsweeping-independent constants; inheritance0 has no assumed exceptions. PCE/LCE masks, engine counts and CE routes must come from the current host at runtime. Pascal/older excluded. Datacenter GA100/GH100/GB100/GB200 and shared-silicon A10/L4/L40/A2 remain separate, wanted last and outside the20-die target.

6. Cleanup:

MEASURED (2026-10-23, `round4/final_cleanup.json`, all per-lease `TEARDOWN.json`): exact owned IDs57193877,57209908,57262021,57280778,57296386,57384343,57385378,57398382 are API-absent. Owned reapers/helpers0, no owned dev cargo target or GPU/cargo lock;7,099,864,172 cache bytes deleted. No task process remains. Current account-credit snapshot is not a lane invoice. Evidence was on origin before each exact-ID teardown; no merge/master/force-push.

Current matrix (CODE READ of the dated receipts named in each row):

| Die | Driver / source / flags | Result | Evidence |
|---|---|---|---|
| AD102 | Win580.88 /rev170054339 /P19+RPC+display/GOP ON | prior desktop+600s holdPASS; full matrixPARTIAL |`lease_57193877/ADA_RESULT.md`|
| GA102 | Linux595.91.07 /98b599bbf | nativePASS n1 |`lease_57385378/native_complete/`|
| GA102 | Win580.88 planned /98b599bbf /P19+RPC/BODY/DIAG1000,display/GOP ON | bindREADINESS_INCONCLUSIVE; actual ON/desktop/apps/holdUNRUN |`lease_57385378/bind_inconclusive/`|
| GA104 | native Win580.88 prerequisite/e8883e2 planned | UNRUN/no current candidate offer |`round4/GA104_NATIVE_PLAN.md`, `offers.json`|
| GA106 candidate | RTX3060 /runtime implementation unverified | UNRUN/safety containment only |`lease_57398382/PROGRESS.md`|
| Turing candidate | RTX2060S/TU106 candidate /runtime implementation unverified | UNRUN after safety stop, offer existed |`round4/offers.json`, `lease_57398382/PRE_TEARDOWN.md`|

GA102 — Round4 per-die receipt (paths below relative to `lease_57385378/`):

1. MEASURED (2026-10-23, lease57385378, `START.json`): GA102 provider replacement, frozen source98b599bbf, actual0.29USD/h, prearmed fixed expiry17:42:19Z.
2. MEASURED (2026-10-23, lease57385378, `SSH_READY.json`): strict pinned SSH/KVM/167GB free/8CPUs/17949MiB RAM; no pin or deadline changed.
3. MEASURED (2026-10-23, lease57385378, `native_gates/sweep_host_die.log`): unprivileged arch0x170/impl2 confirms GA102 on595.91.07 OPEN.
4. MEASURED (2026-10-23, lease57385378, `native_gates/`): hostSOUND; gates9/9+gate10PASS,12/12USER births,0refused.
5. MEASURED (2026-10-23, lease57385378, `native_complete/`): native raw30/30,CUDA4/4,GRtierPASS completed15:28:31Z; n1/spreadN/A.
6. MEASURED (2026-10-23, lease57385378, `NATIVE_BINARY_IDENTITY.json`): QEMU52db5101/raw d971aab3/guard2e7a0c25 match the earlier source product binaries.
7. MEASURED (2026-10-23, lease57385378, `baseline_calibration/`): generic standard-VGA install/checks PASS/sealed16:13:52Z; actualboot173s/query31.815s/scale10.
8. MEASURED (2026-10-23, lease57385378, `bind_inconclusive/bwr_install_readiness.json`): first failure16:40:09.742797Z, bwr_install one attempt1200s rc124, controllerrc75 READINESS_INCONCLUSIVE.
9. MEASURED (2026-10-23, lease57385378, `bind_inconclusive/nv_install_20261023T163212Z.txt`, `bwr_setupapi_progress.txt`): signed package/device-disable seen; copy completed16:38:53 and catalog validation16:38:57, no completed bind verification.
10. MEASURED (2026-10-23, lease57385378, `bind_inconclusive/arms/bw580_bind/summary.json`): driverRPCs0; first failing RPC/ordinal none; no actual ON, Code22/Code0 verdict, desktop, apps, Unigine or600s hold.
11. MEASURED (2026-10-23, lease57385378, `bind_inconclusive/bwr_final_windows_events.txt`): query returned only Kernel-Boot153/VBS-disabled records16:15:55Z and guest-clock22:52:35Z; not nvlddmkm153 or a TDR; timestamp anomaly retained.
12. CODE READ (2026-10-23, lease57385378, `signin_overlay_receipt.json`): sign-in observer-only overlay37c5abdc4 applied before ON marker; no Rust/product/audit branch change, fresh clippy not run.
13. MEASURED (2026-10-23, `../round4/proof/`): lease22/22, desktop77/77, provider endpoint16/16, sign-in19/19PASS; mutation killers all>=2 distinct methods.
14. CODE READ (2026-10-23, lease57385378): exact native controls guard unchanged; cleanup/export/API-absence receipts follow before any next rental; no installer replay or new GA102 retry.
15. CODE READ (2026-10-23, `../round4/offers.json`): latest39-offer snapshot has no GA104 class; GA106/TU106 candidates remain; native/runtime query plans subtract existing constant tables and require budget-fit before create.

GA106 candidate — Round4 safety receipt (paths below relative to `lease_57398382/`; actual die unmeasured):

1. MEASURED (2026-10-23, lease57398382, `START_SAFETY.json`): advertised RTX3060/GA106 candidate, actual implementation unmeasured; no die inferred from its name.
2. MEASURED (2026-10-23, lease57398382, `SAFETY_INSTANCE_FIELDS.json`): offer46510011/machine22775, price0.133333USD/h, disk180, provider loading/stopped state.
3. MEASURED (2026-10-23, lease57398382, `PREARM_READY.json`):90min reaper1596919 armed16:47:18Z before create, deadline18:17:18Z never extended.
4. MEASURED (2026-10-23, lease57398382, `PRE_TEARDOWN.md`): create reply failed exact-ID validation; normal registration and SSH-key attachment did not execute.
5. MEASURED (2026-10-23, lease57398382, `SAFETY_INSTANCE_FIELDS.json`): account lookup found one new instance under the unique request label with matching machine/GPU/creation time.
6. CODE READ (2026-10-23, `scripts/bench/diesweep/lease_guard.py`): invalid replies remain refused; no string coercion, parser weakening or second create was used.
7. CODE READ (2026-10-23, lease57398382, `PRE_TEARDOWN.md`): original create reply was not retained; its malformed field/type and exact cause remain unknown.
8. MEASURED (2026-10-23, lease57398382, `START_SAFETY.json`): exact recovered owned ID registered with the already-armed reaper after identity/time checks, fixed deadline unchanged.
9. CODE READ (2026-10-23, lease57398382, `PRE_TEARDOWN.md`): real lease-accounting safety stop; no further rental/GPU work in this round.
10. MEASURED (2026-10-23, lease57398382, `START_SAFETY.json`): no authenticated SSH, uploads, provisioning, die probe, host query, VM or GPU workload.
11. CODE READ (2026-10-23, lease57398382): native constants/table comparison and engine-fault-ID collection UNRUN; existing audited GA106 receipts retained.
12. CODE READ (2026-10-23, lease57398382): Windows bind/ON/desktop/apps/Unigine/600s hold UNRUN; hardware acceptance n0, spread unavailable.
13. MEASURED (2026-10-23, lease57398382, `TEARDOWN.json`): exact-ID API absence16:50:03Z; identity-checked reaper cancelled only afterward.
14. INFERRED (2026-10-23, lease57398382, `TEARDOWN.json`): runtime estimate0.006139USD plus original0.420313 transfer allowance=0.426452USD conservative cost, not provider invoice.
15. MEASURED (2026-10-23, `../round4/final_cleanup.json`): all eight owned IDs API-absent, timers/helpers0, own target absent and7.10GB caches removed; no partial table or GPU acceptance claim.

AD102 — PARTIAL, prior2026-10-22 run

1. MEASURED (2026-10-22, `lease_57193877/ADA_RESULT.md`): RTX4090/AD102, host595.91.07, guest580.88, productionrev170054339, lease57193877.
2. MEASURED (2026-10-22, same evidence): native gates9+10,USER12,raw30/30,CUDA4/4,GRPASS; n1/spreadN/A.
3. MEASURED (2026-10-22, `lease_57193877/final_stopped/`): native exact five intentional controls unchanged; no unmatched host Xid.
4. MEASURED (2026-10-22, `lease_57193877/final_stopped/arm_ad102_s4_hold_diag.txt`): actual Code0,1920x1080,DWM/Explorer and signed-in desktop.
5. MEASURED (2026-10-22, `lease_57193877/final_stopped/arm_ad102_s4_hold.log`):600s idle hold12:14:21–12:24:22Z passed, NVIDIA153/Display4101/bugcheck1001 zero before/after.
6. MEASURED (2026-10-22, `lease_57193877/final_stopped/matrix_ad102_s4_r4/win.res`):18/112 rows,13PASS/4FAIL/1NOTRUN;94unrun under lease reserve.
7. MEASURED (2026-10-22, `lease_57193877/first_matrix_failure.json`): all4 Unigine paths FAIL strict no-GPU-proof predicate despite surviving45s.
8. MEASURED (2026-10-22, same receipt): first heaven_d3d11 start11:59:24.8192192Z/end12:00:14.0270493Z/verdict12:00:30Z.
9. MEASURED (2026-10-22, same receipt): observer ordinals7566/8058/8155 bracket those app times; no causal failing RPC established for a proof refusal.
10. MEASURED (2026-10-22, `lease_57193877/first_desktop_event_correlation.json`): first child produced193/815 live dump; reason unresolved; later clean hold does not cover all boots.
11. MEASURED (2026-10-22, `lease_57193877/ADA_RESULT.md`): Blender RADAR_PRE_LEAK_64 WER1001 at12:08:20.4498130Z retained; CUDA render row passed.
12. CODE READ (2026-10-22, same report): absent NVRTC supplement left cdp_quicksort NOTRUN; later harness staging fix supplies no retrospective pass.
13. CODE READ: broker new-content metrics, full112rows, accepted UnigineGPUproof and prolonged mixed-workload stability unverified.
14. MEASURED (2026-10-22, `lease_57193877/TEARDOWN.json`): exact lease destroyed/API-absent, own timer cancelled after absence.
15. CODE READ: no Ada remeasurement in the adaptive resume; bounded historical receipts remain authoritative.

