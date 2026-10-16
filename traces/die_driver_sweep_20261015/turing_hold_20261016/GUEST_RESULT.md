# Turing W3 hardware results

**STATUS: RESEARCH, 2026-10-16 03:42 UTC. Guest hold and R34 still pending.**

Owned lease56180232, native unprivileged TU106 arch0x160/impl0x6, UID65534,
CapEff0. Host Linux595.91.07. Guest Windows LTSC26100 actual32.0.15.8088
(public ABI580.65.06); Linux guest staged580.159.04, kernel6.8.0-142.

| Run | Source | Measured result |
|---|---|---|
| Native Linux raw | baseline product8e3a9d325 | 30/30, zero timeout/recovery |
| Baseline native gates | 05442b10 | 9/9 plus gate10, USER births12/12 |
| Event initial native gates | ef4fce21f | 8/9; gate8 callback-wake assumption failed |
| Event corrected native gates | 5b523ed5 | 9/9 plus gate10, USER births12/12; VER2/3 collect real GPU event without fd wake |
| Baseline Windows short suite | 05442b10 | Code0, DWM/Explorer, CUDA3, D3D11/12, zero System4101 TDR |
| Baseline Windows seven-minute hold | 05442b10 | UNVALIDATED: helper deadline300s cut off requested660s |
| Baseline Windows retry | ca0ad248 | Running; no hold verdict yet |
| R34 baseline/event at180s | pending | Queued, not measured |

H-T7: native reference421.03s/13 rounds/zero TDR passes; first native desktop
precondition failure preserved in native-first/. Guest short checks pass;
the helper timeout is not evidence of either seven-minute success or a TDR.
H-W1 event/callback falsifier has not been evaluated on R34. Gate8's old
fd-only harness failure does not establish product failure: corrected native
real-copy/map/clear/supersession checks pass using GPU event polling.

All known negatives are under guest-wave/: QGA prebind handshake, five-minute
hold truncation, initial gate8 failure and build prerequisite failures.
Actual process receipts carry source, private binary SHA, actual KF3 env and
host/guest ABI. Raw screenshots/executables stay on rental; text only back.
Clean code PRs13,19,20 target claude/gpu-types-20261015; evidence stays here.
No full event guest regressions, family completion or multiple Windows builds
are claimed. Deadline04:11:46 UTC/hard04:16:45 remains unchanged.
