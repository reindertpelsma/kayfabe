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
