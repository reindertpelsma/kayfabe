# Candidate-6 final comparison

MEASURED: afe00b3cbc7e345e2953b1bc199d9cce1054cc38, FullHD, one iteration planned for all nine workloads on pinned K1 harness eb71a15b96fabdb3babfd6d0f9fcf91a3dc15fb0. Run4260 stopped at its guest-event guard during Energy. Three workloads Completed; only Blender and CATIA composites were exported before the stop. Creo remains Completed with composite unavailable in this text export. Raw CoW is preserved; no guest reboot was used to recover another score.

CODE READ: baseline89fc8a222 and two VFIO runs come from K1 comparison.csv at the pinned evidence head, also checked against the VFIO host summary. Q1 did not remeasure them. The candidate master parent e64b8e5f6 differs from that performance baseline. Ratio columns are arithmetic INFERRED from MEASURED composites; missing results are never zero.

| Workload | Candidate result | Baseline89fc8a222 | VFIO1 /2 | Candidate/baseline | Candidate/VFIO mean |
|---|---|---|---:|---:|---:|
| blender-01 | 36.03, Completed | 35.85, Completed | 61.61 / 60.61 | 1.0050 | 0.5896 |
| catia-07 | 50.53, Completed | Failed | 63.10 / 63.13 | — | 0.8006 |
| creo-04 | Completed; score not exported | Incomplete | 104.94 / 103.97 | — | — |
| energy-04 | Incomplete (guard stop) | NotRun | 54.57 / 54.67 | — | — |
| enscape-01 | NotRun (guard stop) | NotRun | 37.26 / 37.39 | — | — |
| maya-07 | NotRun (guard stop) | NotRun | 162.14 / 160.37 | — | — |
| medical-04 | NotRun (guard stop) | NotRun | 50.54 / 50.58 | — | — |
| solidworks-08 | NotRun (guard stop) | NotRun | 29.51 / 29.19 | — | — |
| unreal_engine-01 | NotRun (guard stop) | NotRun | 63.58 / 63.70 | — | — |

MEASURED: candidate completed-score n=1 per workload, spread unavailable. VFIO n=2, range/mean0.05–1.64%. CODE READ: 8GiB virtual VRAM versus12GiB native, 12GiB guest RAM and display differences limit interpretation; these ratios are not isolated virtualization overhead. Prior Phase1 source468a35c9 is not substituted for the merged candidate.

| Steel Nomad arm | Scores | Provenance |
|---|---|---|
| Candidate6 | NotRun | Guard stopped before this installed workload |
| Previous kayfabe reference | 2227 /2991 | Owner-provided, not remeasured by Q1 |
| VFIO reference | 3962 /3795 | Prior owner/host reference, not remeasured by Q1 |

MEASURED: Heaven and Minecraft were installed/staged but NOT RUN after the mandatory guard stop. No candidate FPS, frame-gap metric or ratio exists for these arms. Original production run3439 Budget/Scan-budget/refusal counts were zero (complete before-log scan; production-before.json).
