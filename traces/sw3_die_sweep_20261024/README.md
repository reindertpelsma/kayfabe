# SW3 die sweep — GA104 qualification

STATUS: PARTIAL LANE / GA104 PASS, 2026-10-24 UTC. This supersedes the first-attempt STOP summary; the original receipts and preregistrations remain in git and the private captures.

1. MEASURED (2026-10-24, git base commit `a9105acb6`): lane SW3; branch `codex/sw3-20261024`, based on `origin/master` `a9105acb6`. Requested: extend the Linux die/driver matrix, native first, using production candidate `714ec29d3`. The evidence head is the containing commit; the external lane report records its exact origin-verified hash. No merge, PR, deployment or Worker rental was performed.

2. Results:

- MEASURED (2026-10-24, lease57568731, `ga104_57568731/binding.json`): actual RM architecture368/implementation4/revision161, PCI10de:2487 is GA104; the provider marketed it as RTX3060 and the original forecast was GA106.
- CODE READ (`crates/kf-chip/src/die.rs`, base `a9105acb6`): the public ogkm-derived tuple names GA104. Marketing names do not determine the die. No product code or die table changed.
- MEASURED (2026-10-24, lease57568731, `ga104_57568731/actual_die_stop.json`): the operator's hardcoded GA106 forecast stopped before gates. Original stopped receipt/capture retained; COORD explicitly authorized tests-first correction and same-lease GA104 repreregistration.
- MEASURED (2026-10-24, lease57568731, `ga104_57568731/ga104_preregistration.json`): fresh exact-die/driver/source/falsifier plan preceded gates. Production714 predates `die.rs`; the separately pinned open-source mapping was packaged in the operator kit, leaving the production checkout unchanged.
- MEASURED (2026-10-24, lease57568731, `ga104_57568731/result.json`): native GA104 Linux binding PASS, host595.91.07 open, gates9/9+gate10PASS, gate USER births12/12, bare30/30PASS.
- MEASURED (2026-10-24, lease57568731, `ga104_57568731/fast_scorecard.log`): production714ec29d3, host-mode Linux guest595.91.07 open, fast30/30PASS, zero failures/crashes/unrun; fixed180s oracle. The larger copy arm passed119s; n1, no performance extrapolation.
- MEASURED (2026-10-24, lease57568731, `ga104_57568731/result.json`): QEMU SHA256 `9e99b20f7b98b79af8c8661a0b313fcf1fea8915fbe6e55d869660d059f2cfed`, installed under `kf3-bins/714ec29d`; 30 supervised child completions,161/161 USER suite births,90 CUDA thread-posture records.
- MEASURED (2026-10-24, lease57568731, `ga104_57568731/xid_proof_summary.json`): host Xids0→10, exactly5 native plus5 guest source-approved negative controls; final kernel lines equal the accepted sequence, unexpected0. No global Xid exemption or identity waiver.
- MEASURED (2026-10-24, lease57568731, `ga104_57568731/result.json`): host BAR1 total256MiB, no resize; flags `KF_DEVICE=kf3 KF_FROM_HOST=1`. Windows acceptance UNRUN.
- MEASURED (2026-10-24, lease57568731, `ga104_57568731/capture_receipt.json`, `teardown.json`): final text capture SHA256 `ecb7ae5f11989e4afb119fac2be8de769b2dc7211a48179c054f3ea0358830a7` pulled/verified22:31:03UTC before exact-ID API absence22:31:08UTC; watchdog3441128 cancelled afterward, fixed00:13:05UTC bound never extended.
- MEASURED (2026-10-24, `cleanup.json`, `cost.json`): all three owned IDs absent; final-lease runtime estimate0.175823USD, lane runtime estimate0.201700USD. New-spend guard0.675823USD includes a retained0.50USD transfer/billing liability reserve, separate from runtime; neither reserve nor runtime estimate is a settled invoice. Credit23.097188USD exceeds reserve8; stop2.50/cap3.00 preserved.
- CODE READ (`leases.json`, `operator_plan.json`, `attempt2/candidates.json`): three leases exhausted. First attempt selected a deverified offer in error and stopped; the third lease's measured GA104 result supersedes the marketing-only NOT AVAILABLE forecast. Other dies remain UNRUN, not failures.

3. Matrix and proof counts:

| Die | OS / driver pair | kf3 source / flags | Verdict | Evidence |
|---|---|---|---|---|
| GA104, actual lease57568731 | Linux6.8.0-59 fast guest; host595.91.07 open / guest595.91.07 open | 714ec29d3; KF_DEVICE=kf3, KF_FROM_HOST=1; host BAR1256MiB, no resize | PASS: native binding,9/9+10,bare30,fast30,birth census | ga104_57568731/result.json |
| GA104 | Windows native / guest unchosen | no Windows binary or test | UNRUN; no Windows binding or desktop claim | ga104_57568731/result.json |
| GA102 candidates, leases57565088/57565292 | actual dies/drivers unverified | planned714ec29d3, unexecuted | UNRUN: pre-SSH provider/selection stops | lease1/, lease2/, leases.json |
| GA106 | Linux planned; RTX3060 candidate proved GA104 | none measured | UNRUN; do not misfile GA104 as GA106 | ga104_57568731/binding.json |
| TU106 / TU116 | Linux planned | none measured | UNRUN; no candidate in fresh filtered snapshot / no lease | attempt2/candidates.json |
| AD103 / AD106 | Linux planned | none measured | UNRUN; final lease allowance exhausted | attempt2/candidates.json |

MEASURED (2026-10-24, lease57568731, `ga104_57568731/result.json`): hardware n1 per complete native/guest qualification, spread not applicable. No native test failed, no native die exclusion and no kayfabe failure was observed in the completed cell. The earlier forecast and missing operator source-path stops were instrument/precondition events, not native test failures.

| Operator proof | Before → after | Distinct killing tests per mutant | Evidence |
|---|---|---|---|
| Offer admission | EXPECTED RED2ff605c1b:14tests/6fail →14/14 | verification removed6; truthiness4; blacklist removed2; VM status removed4; missing VM accepted2 | attempt2/selector_expected_red.log, final_selector_mutants.json |
| Native derived binding | EXPECTED RED7bbb5289e:17tests/3errors →17/17 | forecast reintroduced3; scope removed4; binding removed3; SSH/console removed4 | attempt2/binding_expected_red.log, final_binding_mutants.json |
| Final combined operator suite |31/31; Black25.1.0,100-column check | nine mutants all≥2; temporary copies only | attempt2/final_operator_tests.log |
| Existing payload fixture suite |140/140 in first attempt | unchanged suite, no new mutation claim | payload_tests.log |

CODE READ (`scripts/bench/sw3/`): unknown/out-of-scope tuples, malformed/incomplete probes, inconsistent driver/PCI binding, blacklisted machines, deverified offers and VM-deverified status remain refused. Real SSH/console mismatch or absence remains fatal. Only operator code changed; no fresh product unit-test or lint claim is made.

4. Owner decisions:

1. Recommend accepting only the exact GA104 Linux595.91.07/open ×595.91.07/open,714ec29d3 cell above; broader die/driver/flavour and Windows coverage needs separate authorization.
2. Recommend retaining the actual RM tuple as the key when selecting RTX3060 offers: this verified offer was physically GA104. No fourth lease was created; further dies require a new explicit lease/budget allocation.

5. Unverified: GA106, GA102, Turing and other Ada qualification on this production candidate; Windows native binding/guest/apps/display; broader driver pairs/flavours; long-run or performance behavior; settled transfer and final invoices. The pre-destroy broad process-name probe returned a match whose text was not retained; no specific cause is asserted. Completed supervised children/controller rc0 and exact API absence are the retained teardown evidence.

6. Cleanup: MEASURED (2026-10-24, `cleanup.json`, lease57568731 `teardown.json`):57565088,57565292,57568731 are API-absent, watchdogs cancelled after absence, no own local Cargo target was created and no GPU lock remains. No fourth lease, token-bearing provider metadata, executable/image, closed-binary analysis or other session resource is published. Raw text captures remain in the prescribed private lane directory. Local worktree removal follows the verified final branch push and is recorded in the external report.
