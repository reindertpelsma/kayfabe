# SW5 die sweep

STATUS: CLOSED, 2026-10-25 UTC (2026-10-26 CET). Three qualified Linux cells; TU106 excluded under the frozen native Xid policy.

MEASURED (2026-10-25 UTC; leases57723513/57726516/57731910, their `result.json` files below): GA102, GA106 and AD106 each passed native gates9/9+gate10, bare30/30, fast30/30 and the USER birth census. Host and host-mode Linux guest are595.91.07 open, kernel6.8.0-59; production714ec29d3. Hardware n1 per cell; no performance, long-run or broader compatibility claim.

| Actual die | OS; host / guest | Product; flags | Verdict | Evidence |
| --- | --- | --- | --- | --- |
| GA102 | Linux6.8.0-59;595.91.07 open /595.91.07 open |714ec29d3;KF_DEVICE=kf3,KF_FROM_HOST=1,KF3_FB_MB=8192 | PASS native9+10,bare30,fast30,USER; n1 | [lease57723513](ga102_57723513/result.json) |
| GA106 | Linux6.8.0-59;595.91.07 open /595.91.07 open |714ec29d3;KF_DEVICE=kf3,KF_FROM_HOST=1,KF3_FB_MB=8192 | PASS native9+10,bare30,fast30,USER; n1 | [lease57726516](ga106_57726516/result.json) |
| TU106 | Native Linux595.91.07 open; guest UNRUN |714ec29d3 instruments; guest binary UNBUILT;4096MiB planned | EXCLUDED: native Xid refusal; guest0arms | [lease57729990](tu106_57729990/result.json) |
| AD106 | Linux6.8.0-59;595.91.07 open /595.91.07 open |714ec29d3;KF_DEVICE=kf3,KF_FROM_HOST=1,KF3_FB_MB=8192 | PASS native9+10,bare30,fast30,USER; n1 | [lease57731910](ad106_57731910/result.json) |
| TU116 candidate | Linux planned | None executed | UNRUN; marketed GTX1660S offer had6CPU, below the inherited8CPU floor | [market summary](market_summary.json) |
| AD103 | Linux planned | None executed | UNRUN; no eligible offer in retained snapshots; four leases exhausted | [market summary](market_summary.json) |
| All SW5 targets | Windows | None executed | UNRUN | Linux-only sweep |

MEASURED (2026-10-25 UTC; leases57723513/57726516/57731910, linked results): each completed suite has161/161 USER births and90 CUDA posture records; gates12/12 USER. Host Xids0→10 equal exactly5native plus5guest source-approved controls, unexpected0. Host BAR1 is256MiB on all four boxes, without resize. The three executed QEMU binaries share SHA256 `9e99b20f7b98b79af8c8661a0b313fcf1fea8915fbe6e55d869660d059f2cfed` under `kf3-bins/714ec29d`.

MEASURED (2026-10-25 UTC; lease57729990, [refusal](tu106_57729990/defer_refusal_excerpt.log), [kernel records](tu106_57729990/native_xid_records.log), [scorecard](tu106_57729990/bare_scorecard.log)): native gates9/9+gate10 and12USER births passed. Defer-liveness printed its own PASS/rawrc0; the frozen wrapper returned6 after4Xid31 records named `HUBCLIENT_HSCE0`. Aggregate6PASS/24FAIL consists of6passing arms,1guard refusal and23blocked arms. No guest ran, no kayfabe failure is attributed, and no native retry or policy change was made.

CODE READ ([source provenance](source_provenance.json), retained `native_xid_guard.py`): the existing copy-client predicate admits `HUBCLIENT_CE[0-9]+`; it does not admit `HUBCLIENT_HSCE0`. This qualification refusal establishes no physical hardware defect. [Blacklist](known_bad_machines.json) records the exact qualification scope.

CODE READ (inherited capture collector, lease57729990 result): the extensionless `XID_STOP` file was omitted by the inherited capture filter. Original `KF_XID_REFUSED`, blocked-arm logs and the full final kernel were captured privately; the refusal evidence above is retained. No claim about the omitted marker contents is made.

MEASURED (2026-10-25 UTC; [cleanup](cleanup.json), linked capture receipts in every result): all4owned IDs are API-absent; every final text capture was pulled and SHA256-verified before exact-ID destruction, then each starttime-pinned watchdog was cancelled and verified gone. No own Cargo target or GPU lock was created; no fifth lease or provider retry occurred.

| Lease | Runtime estimate USD | Retained transfer liability USD | Nonruntime margin USD | Conservative retained USD |
| --- | ---: | ---: | ---: | ---: |
| 57723513 | 0.209072636 | 0.169270833 | 0.50 | 0.878343470 |
| 57726516 | 0.086150502 | 0.050781250 | 0.50 | 0.636931752 |
| 57729990 | 0.029681777 | 0.050781250 | 0.50 | 0.580463027 |
| 57731910 | 0.188408890 | 0.152343750 | 0.50 | 0.840752640 |

MEASURED (2026-10-25 UTC; [cost](cost.json), [cleanup](cleanup.json)): runtime estimate0.513313805USD; conservative retained2.936490888USD, including transfer liability0.423177083USD and2USD nonruntime margins. Retained cost is below stop4/cap5USD; shared credit31.957172USD exceeds reserve8USD. These are not settled invoice amounts; shared credit changes are not exclusive lane-spend attribution.

MEASURED (2026-10-25 UTC; [rehearsal](operator_rehearsal_before_lease3.log), [SW3 tests](sw3_operator_tests.log), [pending-die tests](pending_die_green.log), [store-plan tests](store_plan_green.log)):68retained lifecycle/console/watchdog/cost tests,31SW3 admission/binding tests and12new operator-profile tests pass,111total. The first new module produced an EXPECTED RED import error; after its fix6tests passed. The second module likewise produced an EXPECTED RED import error; final new-module suite12passes. Mock-transport rehearsal is not a hardware result.

| Operator addition | Tests-only EXPECTED RED | Fix | Mutants; distinct killing tests |
| --- | --- | --- | --- |
| Pending actual-die admission |0828712f3|011b87924| scope removed2; completed-die guard removed2 |
| Store/headroom profile |344103f7d|5e21959a7| headroom removed2; minimum removed2 |

MEASURED (2026-10-25 UTC; [pending mutants](pending_die_mutants.json), [store mutants](store_plan_mutants.json)): all4guard-removal mutants have at least2distinct killing tests. CODE READ (`scripts/bench/sw5/`): only operator planning changed; product Rust/QEMU code, Xid matching and180s arm limits are unchanged.

Owner recommendation: accept only the three exact qualified Linux cells above. Keep TU106 excluded pending source-backed review of the native HSCE0 refusal; no guest result or Turing hardware defect follows. TU116/AD103 and all Windows/app/performance coverage remain unverified.

Raw captures contain text only and remain in the private lane capture store. Published scorecards omit board-specific identifiers. [Source provenance](source_provenance.json) records production and instrument hashes.

CODE READ (public EXPECTED RED logs): traceback filenames are normalized to repository-relative paths; the private originals retain the exact output.

MEASURED (2026-10-25 UTC; [claims](claims.log), [diff scan](final_diff_scan.json)): claims existing-debt3758/new0/resolved0; added branch lines contain no credential, IP, board UUID or private host path matches, and new code lines stay within100columns. No debt entry or guard was waived.
