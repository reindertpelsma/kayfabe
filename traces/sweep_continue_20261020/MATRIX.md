# Continuation matrix

STATUS: LIVE, 2026-10-21. Every row carries its measured revision; historical passes are not promoted. Final predecessor handoff df87431d5 read completely (supersedes the initial362fc snapshot). Next frozen sourcefe3a978ee includes harness-only family fixture repair and derived OLUT26691c14a; no new Windows result yet.

| Die | OS / actual driver | Source | Flags | Verdict | Evidence / scope |
|---|---|---|---|---|---|
| GB202 | Windows580.88/8088 OFF | c58989dab (2runs), ccb4b706c (1run) | P19 + RPCdiagnostics, displayOFF/GOPoff/emptyROM | MEASURED Code0 at20s, then0x116 around40s, P3C000009A; fullPASS FAIL | predecessor arms/fix2_off_1,fix2_off_2,fix3_off_1; n3 |
| GB202 | Windows580.88/8088 ON | base7f/aa8/c589/ccb | P19 + per-arm named diagnostics | MEASURED Code43 after4thCA7A, n10 | predecessor README arm table; actualsource eachreceipt |
| GB202 | Windows580.88 OFF public semaphore fix | 71588cecb | P19 + RPC diagnostics | MEASURED Code0 then0x116, n1; GR alive/GET=PUT2, CE80f GET=PUT2968 | predecessor df874/lease2/arms/fix4_off_1; public GR refusal not sole TDR cause |
| GB202 | Windows580.88 ON derivedPOSTCOMP CAPA | 048c28b9c | P19 + RPC diagnostics, no captured hook | MEASURED Code43 after cursor4, n1 | predecessor df874/lease2/arms/fix5_on_1; six presence bits insufficient |
| GB202 | Windows580.88 native forcedGSP | nativeGSP580.65.05, reference tracer8f9bd4e5 | Actual native firmware/trace flags inreceipt | MEASURED priorCode0/CUDA3, complete3804cmd/3863status/zero-loss | prior own0bbf evidence phase2_windows_gb202_56816449/native_trace/ |
| AD102 | Windows580.88/8088 | 0c87acda8 | exactP19+RPCdiagnostics | MEASURED priorPASS desktop/CUDA3/DX11+12/426.09s/0TDR, n1 | prior own phase2_windows_ad102_56816419/windows580_pass/; not048c acceptance |
| AD102 | Linux580.159.04/595.91.07 broker, host595 | 7f22d18df | laneactualflags | MEASURED priorPASS both,n1each | prior own phase2_ada_broker_followup_20261020/RESULT.md@0bbf; not048c acceptance |
| GB202 | Linux native595.91.07 | 90e7f2a54 | Stock/unprivileged probes | MEASURED gates9/9+10PASS, raw30/30, CUDA4/4; nativeGRoracleFAIL class fixture | lease56949833/native; WindowsUNRUN, lease destroyed |
| GB202 | Windows580.88 ON/OFF follow-up | frozenfe3a978ee (code26691c14a) | P19 + named diagnostics | UNRUN; own56964717 preparing | Native GR gate must pass first; no inference of WindowsPASS |
| AD103/AD106 | Windows580.88 | newstack | P19planned | UNRUN; oldpassesSTALE | AfterTDR/display priorities |
| GA104 | Windows580.88 | newstack | P19planned | UNRUN; native verified binding prerequisite | No installer replay |
| TU106 | Windows580.88 | newstack | P19planned | UNRUN regression | R34 policy not silently changed |
| GB205/GB206 | Windows580.88 | newstack | P19planned | UNRUN with new fixes | Per-die measured rows remain exact scope |
| Remaining dies | Linux580/595 brokers | newstack | laneactualflags | UNRUN | Only after priorities or independent idle preparation |

P19 exact values and supplementarydiagnostics are in the prior matrix and predecessor box_scripts/p19.flags; each future arm receipt must print/assert actualenvironment/QOM/source/hash. No flagless claim. Whole own-die page diagnostics in predecessor tests remain oracle-only and are never shipped; diagnosticbranches stay unmerged.
