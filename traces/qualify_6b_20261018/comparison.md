# Partial SPECviewperf comparison

> Release: SPECviewperf 15.x (patch level not recorded)
> Configuration: Windows guest (owner image) in a QEMU/KVM VM; kayfabe kf3 virtual GPU, candidate 6b versus master 89fc8a222; VFIO physical-GPU reference arm copied from the vfio-ref results; Full HD
> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

MEASURED on 2026-10-22: comparison.json combines vendor-completed scores from run 4400 and the passing fresh Energy arms. The full nine-workload/two-iteration attempt failed in Energy. Completed Energy scores describe survivors; they do not remove failures. Reference runs are historical, not interleaved.

| Workload | 6b completed scores (n) | Master 89fc8a222 | VFIO (n=2) | 6b/master -- (nc) research comparison, not a SPEC-ranked result | 6b/VFIO -- (nc) research comparison, not a SPEC-ranked result |
|---|---|---|---|---|---|
| blender-01 | 40.92 (nc) (1) | 35.85 (nc) | 61.61 (nc), 60.61 (nc) | 1.141 (nc) | 0.670 (nc) |
| catia-07 | 50.38 (nc) (1) | unavailable | 63.10 (nc), 63.13 (nc) | — | 0.798 (nc) |
| creo-04 | 72.71 (nc) (1) | unavailable | 104.94 (nc), 103.97 (nc) | — | 0.696 (nc) |
| energy-04 | 42.39 (nc), 43.37 (nc), 41.48 (nc) (3) | unavailable | 54.57 (nc), 54.67 (nc) | — | 0.777 (nc) |
| enscape-01 | unavailable (0) | unavailable | 37.26 (nc), 37.39 (nc) | — | — |
| maya-07 | unavailable (0) | unavailable | 162.14 (nc), 160.37 (nc) | — | — |
| medical-04 | unavailable (0) | unavailable | 50.54 (nc), 50.58 (nc) | — | — |
| solidworks-08 | unavailable (0) | unavailable | 29.51 (nc), 29.19 (nc) | — | — |
| unreal_engine-01 | unavailable (0) | unavailable | 63.58 (nc), 63.70 (nc) | — | — |

CODE READ: the K1 baseline has only Blender completed; CATIA failed and later workloads were not run. Missing baseline rows are unavailable, not zero. Ratios use arithmetic means of valid completed composites.

| Fresh Energy arm | Run | Verdict | Guest 153 / 4101 | New host Xids | Restored |
|---|---|---|---|---|---|
| E1 | 4401 | FAIL | 3 / 0 | 0 | True |
| E2 | 4402 | PASS | 0 / 0 | 0 | True |
| E3 | 4403 | PASS | 0 / 0 | 0 | True |
| E4 | 4404 | PASS | 0 / 0 | 0 | True |
