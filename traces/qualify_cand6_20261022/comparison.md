# Q1 comparison availability

STATUS: INCOMPLETE, 2026-10-22.

MEASURED: Q1 candidate hardware samples n=0; no candidate score, spread or ratio exists.
CODE READ: prior baseline and VFIO numbers below are transcribed from K1's `traces/specviewperf_kayfabe_20261022/README.md` at evidence head `eb71a15b96fabdb3babfd6d0f9fcf91a3dc15fb0`. They were not remeasured by Q1. The baseline source is `89fc8a222`, distinct from the candidate's master parent `e64b8e5f6`. K1 baseline n=1 where completed; VFIO n=2. Blank results are unavailable, never zero.

| Workload | Candidate f7cf3ed81 | Baseline 89fc8a222 | Prior VFIO 1 / 2 | Candidate / baseline | Candidate / VFIO mean |
|---|---:|---|---:|---:|---:|
| blender-01 | — | 35.85, Completed | 61.61 / 60.61 | — | — |
| catia-07 | — | Failed | 63.10 / 63.13 | — | — |
| creo-04 | — | Incomplete | 104.94 / 103.97 | — | — |
| energy-04 | — | NotRun | 54.57 / 54.67 | — | — |
| enscape-01 | — | NotRun | 37.26 / 37.39 | — | — |
| maya-07 | — | NotRun | 162.14 / 160.37 | — | — |
| medical-04 | — | NotRun | 50.54 / 50.58 | — | — |
| solidworks-08 | — | NotRun | 29.51 / 29.19 | — | — |
| unreal_engine-01 | — | NotRun | 63.58 / 63.70 | — | — |

CODE READ: K1's Phase 1 source was `468a35c9`, not the merged candidate. Its application survival and large-Pointer counts cannot qualify the additional privilege-stamp and batched-walk merges. Prior VFIO range/mean spread is reported by K1 as 0.05–1.64%; candidate spread is unavailable. Prior RAM/VRAM and display differences limit interpretation.

MEASURED: Heaven, Minecraft/other regression and Steel Nomad candidate samples n=0. Installed-harness/game availability, old production Budget counts, guest events/process kills, broker new-content gaps/presents and candidate descriptor counters were not queried after the mandatory safety stop.
