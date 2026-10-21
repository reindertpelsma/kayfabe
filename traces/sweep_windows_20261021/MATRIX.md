# Current Windows sweep matrix

STATUS: LIVE, 2026-10-21. MEASURED rows carry the actual source. No historical PASS is promoted to the merged revision.

| Die | OS / driver | Source | Flags | Result | Evidence / scope |
|---|---|---|---|---|---|
| GB202 | Native Linux595.91.07 | af8bdcda4 | stock; GR oracle KF3_TRANSLATED_CE_RELAY=1 | PASS, n1 | lease_57046541/native/: SOUND, unprivileged die probe, gates9/9+10, raw30/30, CUDA4/4, corrected native GR GPU payload/GET/session event and hostile-entry checks |
| GB202 | Windows LTSC /580.88 | af8bdcda4 | p19.flags +RPC/BODY/DIAG_PROGRESS1000; display ON, signed GOP | FAIL guest0x50 before CIM verdict, n1 | lease_57046541/bw_on_m1/: bugcheck12:47:33.658586UTC, 11.066s afterenable, legacyordinal275 / complete277 fourthcursor; no Code0/43 observation, noold0x116; actualPROGRESS_STRAND0, hostXid0 |
| GB202 | Windows LTSC /580.88 OFF | af8bdcda4 | P19+RPC/BODY/DIAG_PROGRESS1000; display/GOPoff, approvedemptyROM | FAIL Code0 then141/0x116, n1 | lease_57046541/bw_off_m1/: actualCode0,3921RPC/0unparsed,3624teardown,3920unload; twoXid32→STOPGPU; no actualstrand |
| GB202 | Windows580.88 OFF | 71588cecb | historicalP19 +RPC, display/GOPoff | FAIL full validation; historical Code0 then0x116, n1 | predecessor df87431d5 fix4_off_1, GR GET=PUT2 and CE80f GET=PUT2968; not current-source result |
| GB202 | Windows580.88 ON | 048c28b9c | historicalP19 +RPC | FAIL Code43 after fourth cursor, n1 | predecessor df87431d5 fix5_on_1; six POSTCOMP presence bits insufficient |
| GB202 | Native Windows580.88 forced GSP | native firmware580.65.05; tracer8f9bd4e5 | reference receipt flags | Prior PASS Code0/CUDA3; complete trace | prior own0bbf3c55f phase2_windows_gb202_56816449/native_trace: 3804cmd/3863status, zero-loss; not virtual acceptance |
| AD102 | Windows580.88 | 0c87acda8 | historicalP19 +RPC | Prior PASS, STALE for af8bdcda4 | desktop/CUDA3/DX11+12/426.09s zero-TDR, n1; prior own phase2_windows_ad102_56816419/windows580_pass |
| AD102 | Linux580.159.04 and595.91.07 broker | 7f22d18df | actual per-lane flags | Prior PASS both, STALE for af8bdcda4 | prior own phase2_ada_broker_followup_20261020, n1each |
| AD106 | Native Linux595.91.07 | 1ce11cf4d; product unchanged vs af8 | stock; native GR translated CE | PASS, n1 | lease_57064037/native/: actual unprivileged die, SOUND, gates9/9+10, raw30/30, CUDA4/4, nativeGR; five intentional native Xid31 faults documented |
| AD106 | Linux guest595 / Windows580.88 | 1ce11cf4d | planned lane flags; no guest launched | UNRUN / STOP; lease closed | lease_57064037/NATIVE_XID_STOP.md: extra absolute-zero-Xid benchmark guard rejected intentional native faults; approval pending; generic LTSC standard-VGA setup/check passed, no NVIDIA bind |
| AD103 | Windows580.88 | current revision planned | P19 planned, actual receipt required | UNRUN | no die inferred from marketed GPU name |
| GA104 | Windows580.88 | current revision planned | P19 planned | UNRUN | native verified580.88 binding through documented windows-on-vast flow first; no installer replay |
| TU106 | Windows580.88 regression | current revision planned | P19 planned | UNRUN | preserve die/driver policy |
| GB205/GB206 | Windows580.88 | current revision planned | P19 planned | UNRUN | per-die exact measured caps scope; no copied captured page |
| Remaining dies | Linux580/595 broker | current revision planned | actual per-lane flags | UNRUN | after primary priorities / independent idle work |

Frozen hardware source af8bdcda4c700d8b40163ee3bf27feb7cfa6539c includes clean source merge d93cf38e5 (sweep e48bc13e4 + candidateM89fc8a222). Later commits are documentation/bench collection only. P19 is printed in p19.flags; process receipts must record all additional flags/QOM and binary hash. Code0 alone is initialization progress; PASS requires the600s hold, desktop, real CUDA/DX11/DX12 results and no141/116/4101 in the observed window. No PROGRESS_STRAND header is counted as an actual strand.
