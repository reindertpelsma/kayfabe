MEASURED LIVE Round4 (lease57385378, `lease_57385378/START.json`): GA102 replacement candidate, source98b599bbf, quote0.29USD/h; prearmed guard1498466, fixed expiry2026-10-23T17:42:19.188825+00:00. Native/Windows/hold/apps pending; no driver outcome. INFERRED (same receipt): prior0.281506USD plus full new reservation1.421146USD =1.702652USD<stop2.50/cap3, reserve5 intact.

STATUS: PARTIAL,2026-10-23; provider failure before authenticated SSH.

MEASURED (lease57384343, `provider_failure.json`): advertised RTX3090/GA102 candidate, provider state running, relay endpoint present, no host key on two scans inside original480s budget. Actual die/KVM/native/Windows all unverified;0 authenticated SSH,0 uploads,0 driver RPC. This is not a driver failure or a consumed Windows ON attempt.

CODE READ: own reaper remains armed, exact-ID teardown/API absence follows this pushed receipt; timer cancelled only afterward. One provider replacement will use another machine, preserve budget/price/KVM/disk/country/key guards, and perform the sole requested Windows retry. No blind repeated rental loop.

MEASURED LIVE Round4 (lease57384343, `lease_57384343/START.json`): GA102 candidate RTX3090, sourceb66adb6e7, quote0.196667USD/h, prearmed guard1483306, fixed expiry17:31:30Z. Native/Windows/hold/apps pending; no driver outcome yet. INFERRED (same receipt): reserved1.090556USD against stop2.50/cap3, credit before28.804882USD, reserve5 untouched.

S4 — PARTIAL, 2026-10-23

CODE READ: lane S4, branch `codex/sweep-dies-20261022`, worktree S4; requested adaptive shared Windows readiness, fresh GA102 Windows acceptance, then Turing. This report supersedes all earlier LIVE summaries. Git publication is the containing commit, verified against the lane branch on origin after push; the external report records the exact head.

1. MEASURED (`adaptive/green.log`, `adaptive/sw2_shared_green.log`): DONE shared readiness, S4 adaptive57/57 and SW2 policy21/21 pass; no separate scaling-policy fork.
2. MEASURED (`adaptive/mutants.json`): DONE ten adaptive mutants, each>=2 killing tests; EXPECTED RED commits20db1f7bb/1a33267d1 precede fixac21aef93.
3. MEASURED (`lease_57296386/baseline_calibration/box_readiness.json`): DONE actual one-box boot48s/query15.4196s, factor8; ceilings/stage/export reserve enforced.
4. MEASURED (`lease_57296386/native_complete/`): DONE GA102 native595.91.07 raw30/30,CUDA4/4,GR,gates9+10,USER12/12; n1,spreadN/A.
5. MEASURED (`lease_57296386/bind_complete/`): DONE actual signed580.88 Code22, GSPkeys1, sealed parent; adaptive bind SSH reply55.35s.
6. MEASURED (`lease_57296386/stopped_complete/arm_ampere_s4_enable.txt`): DONE actual single-enable Code0/OK at01:46:12.1289843Z.
7. MEASURED (`lease_57296386/stopped_complete/arm_ampere_s4_diag_min_readiness.json`): PARTIAL next actual-state diagnostic timed out twice240s; readiness inconclusive, not a driver-failure verdict.
8. MEASURED (`lease_57296386/stopped_complete/arm_ampere_s4_live_events.txt`): PARTIAL graphics LiveKernelEvent193/815 unresolved; apps/Unigine/600s hold NOTRUN.
9. MEASURED (`adaptive/cleanup/`): DONE cleanup control-flow repair2RED→4GREEN, omitted fallback killed by2tests, complete desktop74/74PASS.
10. MEASURED (`lease_57296386/TEARDOWN.json`): DONE exact-ID API absence02:04:33Z, timer cancelled afterward; evidence pushed before teardown.
11. MEASURED (`lease_57193877/ADA_RESULT.md`): prior Ada desktop/600s zero-TDR holdPASS; full acceptance PARTIAL,18/112 rows13PASS/4UnigineFAIL/1NOTRUN/94unrun.
12. MEASURED (`round3/adaptive_turing_availability/`): NOTRUN Turing;0 eligible versus35 KVM marketplace offers;24 diagnostic Turing offers all KVM-disabled; selector refused before create.
13. CODE READ (`round3/coverage.json`, `round3/TIER_PLAN.md`): target20, existing any-item constants14, inherited0, unmeasured6=30%; no runtime per-die client table, no RE.
14. INFERRED (`round3/final_cleanup.json`, owned teardown receipts): conservative lane$8.071121<12, Round3$4.267503<8; reserve$5 untouched, estimates not invoices.
15. MEASURED (`round3/final_cleanup.json`): owned leases/timers/helpers0, target absent, owned asset caches removed, no owned GPU/cargo locks; full requested hardware acceptance remains incomplete.

CODE READ (production identity): crates treefeda030ab2b0dd7a9aad43bcd163d9ea7afe0842 and QEMU treefa8938d9c1a4b2caeaeb6f15a7daf96ed1e2f190 equal rev170054339. Harness-only changes; no audit-fix merges or product timer/guest TDR/app-oracle changes. Rust changes0, added Rust lint sites0; fresh clippy NOTRUN.

CODE READ (2026-10-23): each row below cites its individual suite execution; overlapping/historical runs are not summed.

| Proof | Before → after | Mutant kills per variant | Evidence |
|---|---|---|---|
| Adaptive Windows readiness | EXPECTED RED →57/57; shared SW2 21/21 |15,6,10,6,9,2,2,3,3,2|`adaptive/`|
| Provider route readiness | EXPECTED RED →8/8 |2,2,2|`adaptive/provider_*`|
| Cleanup fallback |4tests/2RED →4/4 |2|`adaptive/cleanup/`|
| Desktop regression |70/70 before cleanup →74/74 afterward |included above|`adaptive/cleanup/desktop_cleanup_all_green.log`|
| Approved exact channel/VA/type + CE-family |EXPECTED RED →138/138 |3,2,5,5,5,3|`round3/owner_family_*`|
| Checked image transport |EXPECTED RED →16/16 |2,2,2,2,2|`round3/chunks_*`|
| Run-kit allocation metadata export |5RED →5/5 |3,2,2|`round3/alloc_export_*`|

CODE READ: cleanup tests-only EXPECTED RED1c37eaf98 precedes fix5124411b7; the real shell cleanup is exercised with owned CPU-only fixtures. The added fallback uses the same owned PID/socket and keeps final exit verification. Source restored after mutation; no cases dropped.

MEASURED (2026-10-23, `adaptive/publication_receipt.json`): claims introduced on S4 changed files0 at the recorded publication; inherited branch attribution debt is scoped out by explicit owner ruling, no debt entries added.

AD102 — PARTIAL, prior2026-10-22 run

1. MEASURED (`lease_57193877/ADA_RESULT.md`): RTX4090/AD102, host595.91.07, guest580.88, productionrev170054339, lease57193877.
2. MEASURED (same evidence): native gates9+10,USER12,raw30/30,CUDA4/4,GRPASS; n1/spreadN/A.
3. MEASURED (`lease_57193877/final_stopped/`): native exact five intentional controls unchanged; no unmatched host Xid.
4. MEASURED (`lease_57193877/final_stopped/arm_ad102_s4_hold_diag.txt`): actual Code0,1920x1080,DWM/Explorer and signed-in desktop.
5. MEASURED (`lease_57193877/final_stopped/arm_ad102_s4_hold.log`):600s idle hold12:14:21–12:24:22Z passed, NVIDIA153/Display4101/bugcheck1001 zero before/after.
6. MEASURED (`lease_57193877/final_stopped/matrix_ad102_s4_r4/win.res`):18/112 rows,13PASS/4FAIL/1NOTRUN;94unrun under lease reserve.
7. MEASURED (`lease_57193877/first_matrix_failure.json`): all4 Unigine paths FAIL strict no-GPU-proof predicate despite surviving45s.
8. MEASURED (same receipt): first heaven_d3d11 start11:59:24.8192192Z/end12:00:14.0270493Z/verdict12:00:30Z.
9. MEASURED (same receipt): observer ordinals7566/8058/8155 bracket those app times; no causal failing RPC established for a proof refusal.
10. MEASURED (`lease_57193877/first_desktop_event_correlation.json`): first child produced193/815 live dump; reason unresolved; later clean hold does not cover all boots.
11. MEASURED (`lease_57193877/ADA_RESULT.md`): Blender RADAR_PRE_LEAK_64 WER1001 at12:08:20.4498130Z retained; CUDA render row passed.
12. CODE READ (same report): absent NVRTC supplement left cdp_quicksort NOTRUN; later harness staging fix supplies no retrospective pass.
13. CODE READ: broker new-content metrics, full112rows, accepted UnigineGPUproof and prolonged mixed-workload stability unverified.
14. MEASURED (`lease_57193877/TEARDOWN.json`): exact lease destroyed/API-absent, own timer cancelled after absence.
15. CODE READ: no Ada remeasurement in the adaptive resume; bounded historical receipts remain authoritative.

GA102 — PARTIAL, adaptive2026-10-23 run

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
15. MEASURED (2026-10-23, lease57296386, `lease_57296386/TEARDOWN.json`): own API absence02:04:33Z, timer cancelled after absence; runtime estimate$0.348273, conservative$1.348273.

TURING — NOTRUN, 2026-10-23

1. CODE READ (2026-10-23, owner02:20 instruction): Turing follows fresh GA102 adaptive Windows attempt; no new Turing lease before GA102 API absence.
2. MEASURED (2026-10-23, `lease_57296386/TEARDOWN.json`): preceding owned GA102 lease API-absent02:04:33Z, timer cancelled afterward.
3. MEASURED (2026-10-23, `round3/adaptive_turing_availability/census.json`):02:06:18Z,35 KVM/single-GPU/180GB/price<=1 offers,0 Turing offers.
4. MEASURED (same census): expanded consumer/Super/Ti/GTX16/Quadro RTX candidate names included; no main-target Turing class was offered with KVM.
5. MEASURED (same census): separate5GB diagnostic search returned24 Turing offers, all vms_enabled=false; these are not eligible Windows quotes.
6. MEASURED (2026-10-23, `round3/adaptive_turing_availability/selector_refusal.json`): explicit Turing selector queried provider verified=true/false/any and refused before create.
7. MEASURED (same selector receipt): new lease0, new ledger absent, new Turing cost0; no eligibility or price guard relaxed.
8. CODE READ (2026-10-23): current native/engine-fault-table/Windows580.88 bind/ON/desktop/apps/Unigine/600s hold all NOTRUN, hardware n0/spreadN/A.
9. CODE READ (2026-10-23): no first failing driver RPC or guest event time exists for an unrun arm; no synthetic failure or success is reported.
10. MEASURED (2026-10-22, `lease_57262021/TEARDOWN.json`): earlier TU106 preparation rental closed22:02:29Z; it did not establish Windows acceptance.
11. CODE READ (2026-10-23, `round3/coverage.json`): existing TU106 constant/native receipts reused; no duplicate constant rental or capture performed.
12. CODE READ (same matrix): main target20 dies, existing any-item constant evidence14/20, inherited0, unmeasured6/20=30%; workstation axes remain explicitly unverified.
13. CODE READ (same matrix and census): unmeasured TU104,TU117,GA103,GA107,AD107,GB207 have0/6 current KVM candidate classes; this is not a sales-market fraction.
14. INFERRED (2026-10-23): compliant Turing availability is the remaining external prerequisite; the current snapshot does not establish permanent unavailability.
15. CODE READ (2026-10-23): existing owner authorization permits the next compliant Turing acceptance run; no new approval or RE is requested.

CODE READ (constant-tier handoff, `round3/TIER_PLAN.md`): tier1 public ENGINE_ID and CLIENT are separate; no independent route derivation. Tier2 public engine-table request is kernel-only and lacks HUB client identity. Tier3 reused the audit instead of repeating14 existing die captures; six missing dies have no current eligible KVM class. Tier4 RE was not performed. Owner-approved CE-family admission supersedes route derivation as a Windows-run prerequisite; no partial per-die runtime route table was shipped.

CODE READ (`round3/coverage.json`): missing TU104,TU117,GA103,GA107,AD107,GB207; all6 are30% of the equal-weight20-die target, not sales market share. Workstation SKU mapping is explicit and branch/vBIOS/ECC remain unverified unless named in the original receipt. PCE/LCE masks, engine counts and CE routes always come from the actual host at runtime. Pascal/older excluded; datacenterGA100/GH100/GB100/GB200 and sharedA10/L4/L40/A2 remain separate,last,out of denominator.

INFERRED owner decisions:
1. Recommend preserve GA102 as Code0 plus unresolved graphics-live-dump/readiness evidence; investigate event/RPC correlation before claiming desktop stability. No approval is needed for routine authorized harness work.
2. Recommend resume Turing when a compliant offer appears; preserve KVM/disk/price/budget guards and existing authorization.
3. Recommend use the explicit six-die residue when deciding broader constant acquisition or separate RE; S4 requests no RE and supplies no guessed inheritance.

CODE READ (unverified limits): complete per-item/per-driver constants, workstation equivalence, complete appmatrix/Unigine acceptance on any requested family, GA102600s hold, Turing Windows outcome, causal RPC for193/815 and final provider invoice remain unverified. Raw logs/dumps on destroyed rentals are ephemeral; only bounded text/PNG and source hashes were preserved. No proprietary dump binary or reverse-engineering material was examined or committed.
