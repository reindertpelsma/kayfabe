# Candidate-6 final comparison

> Release: SPECviewperf 15.x (patch level not recorded)
> Configuration: Windows guest (owner image) in a QEMU/KVM VM, Full HD, one iteration; kayfabe kf3 virtual GPU, candidate 6 (afe00b3c) versus baseline 89fc8a222; VFIO physical-GPU reference arm copied from the vfio-ref results; 8 GiB virtual VRAM versus 12 GiB native, as stated in comparison.md
> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

MEASURED: afe00b3cbc7e345e2953b1bc199d9cce1054cc38, FullHD, one iteration planned for all nine workloads on pinned K1 harness eb71a15b96fabdb3babfd6d0f9fcf91a3dc15fb0. Run4260 stopped at its guest-event guard during Energy. Three workloads Completed; only Blender and CATIA composites were exported before the stop. Creo remains Completed with composite unavailable in this text export. Raw CoW is preserved; no guest reboot was used to recover another score.

CODE READ: baseline89fc8a222 and two VFIO runs come from K1 comparison.csv at the pinned evidence head, also checked against the VFIO host summary. Q1 did not remeasure them. The candidate master parent e64b8e5f6 differs from that performance baseline. Ratio columns are arithmetic INFERRED from MEASURED composites; missing results are never zero.

| Workload | Candidate result | Baseline89fc8a222 | VFIO1 /2 | Candidate/baseline -- (nc) research comparison, not a SPEC-ranked result | Candidate/VFIO mean -- (nc) research comparison, not a SPEC-ranked result |
|---|---|---|---:|---:|---:|
| blender-01 | 36.03 (nc), Completed | 35.85 (nc), Completed | 61.61 (nc) / 60.61 (nc) | 1.0050 (nc) | 0.5896 (nc) |
| catia-07 | 50.53 (nc), Completed | Failed | 63.10 (nc) / 63.13 (nc) | — | 0.8006 (nc) |
| creo-04 | Completed; score not exported | Incomplete | 104.94 (nc) / 103.97 (nc) | — | — |
| energy-04 | Incomplete (guard stop) | NotRun | 54.57 (nc) / 54.67 (nc) | — | — |
| enscape-01 | NotRun (guard stop) | NotRun | 37.26 (nc) / 37.39 (nc) | — | — |
| maya-07 | NotRun (guard stop) | NotRun | 162.14 (nc) / 160.37 (nc) | — | — |
| medical-04 | NotRun (guard stop) | NotRun | 50.54 (nc) / 50.58 (nc) | — | — |
| solidworks-08 | NotRun (guard stop) | NotRun | 29.51 (nc) / 29.19 (nc) | — | — |
| unreal_engine-01 | NotRun (guard stop) | NotRun | 63.58 (nc) / 63.70 (nc) | — | — |

MEASURED: candidate completed-score n=1 per workload, spread unavailable. VFIO n=2, range/mean0.05–1.64%. CODE READ: 8GiB virtual VRAM versus12GiB native, 12GiB guest RAM and display differences limit interpretation; these ratios are not isolated virtualization overhead. Prior Phase1 source468a35c9 is not substituted for the merged candidate.

Steel Nomad: private

MEASURED: Heaven and Minecraft were installed/staged but NOT RUN after the mandatory guard stop. No candidate FPS, frame-gap metric or ratio exists for these arms. Original production run3439 Budget/Scan-budget/refusal counts were zero (complete before-log scan; production-before.json).
