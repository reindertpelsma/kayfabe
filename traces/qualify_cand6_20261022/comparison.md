# Candidate-6 comparison

MEASURED: candidate afe00b3cbc7e345e2953b1bc199d9cce1054cc38, one FullHD iteration on K1 harness eb71a15b96fabdb3babfd6d0f9fcf91a3dc15fb0. Windows run active; only completed vendor composites are qualified below. Candidate n=1 for completed workloads, spread unavailable.

CODE READ: baseline 89fc8a222 and VFIO references come from K1 comparison.csv at that pinned evidence head; Q1 also read the VFIO host benchmark summary. Baseline n=1 where completed; VFIO n=2. The candidate master parent e64b8e5f6 differs from the performance baseline revision. Ratios below are arithmetic inferences from those measured inputs.

| Workload | Candidate | Baseline 89fc8a222 | VFIO 1 / 2 | Candidate / baseline | Candidate / VFIO mean |
|---|---:|---|---:|---:|---:|
| blender-01 | 36.03 | 35.85, Completed | 61.61 / 60.61 | 1.0050 | 0.5896 |
| catia-07 | 50.53 | Failed | 63.10 / 63.13 | — | 0.8006 |
| creo-04 | Pending | Incomplete | 104.94 / 103.97 | — | — |
| energy-04 | Pending | NotRun | 54.57 / 54.67 | — | — |
| enscape-01 | Pending | NotRun | 37.26 / 37.39 | — | — |
| maya-07 | Pending | NotRun | 162.14 / 160.37 | — | — |
| medical-04 | Pending | NotRun | 50.54 / 50.58 | — | — |
| solidworks-08 | Pending | NotRun | 29.51 / 29.19 | — | — |
| unreal_engine-01 | Pending | NotRun | 63.58 / 63.70 | — | — |

CODE READ: K1 prior Phase 1 source 468a35c9 was not the merged candidate; its scores are not substituted. Kayfabe uses 8 GiB virtual VRAM versus native 12 GiB, with 12 GiB guest RAM in both; display and memory differences limit interpretation. VFIO range/mean spread is 0.05–1.64%. A completed CATIA score compares survival with the failed baseline; it supplies no valid baseline speedup ratio.

MEASURED: original production run3439 Budget/Scan-budget/refusal counts were all zero (complete log scan in production-before.json). Candidate Heaven/Minecraft and installed Steel Nomad remain pending; no guest FPS is labeled broker new-content FPS.
