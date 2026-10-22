# Partial SPECviewperf comparison

MEASURED on 2026-10-22: comparison.json combines vendor-completed scores from run 4400 and the passing fresh Energy arms. The full nine-workload/two-iteration attempt failed in Energy. Completed Energy scores describe survivors; they do not remove failures. Reference runs are historical, not interleaved.

| Workload | 6b completed scores (n) | Master 89fc8a222 | VFIO (n=2) | 6b/master | 6b/VFIO |
|---|---|---|---|---|---|
| blender-01 | 40.92 (1) | 35.85 | 61.61, 60.61 | 1.141 | 0.670 |
| catia-07 | 50.38 (1) | unavailable | 63.10, 63.13 | — | 0.798 |
| creo-04 | 72.71 (1) | unavailable | 104.94, 103.97 | — | 0.696 |
| energy-04 | 42.39, 43.37, 41.48 (3) | unavailable | 54.57, 54.67 | — | 0.777 |
| enscape-01 | unavailable (0) | unavailable | 37.26, 37.39 | — | — |
| maya-07 | unavailable (0) | unavailable | 162.14, 160.37 | — | — |
| medical-04 | unavailable (0) | unavailable | 50.54, 50.58 | — | — |
| solidworks-08 | unavailable (0) | unavailable | 29.51, 29.19 | — | — |
| unreal_engine-01 | unavailable (0) | unavailable | 63.58, 63.70 | — | — |

CODE READ: the K1 baseline has only Blender completed; CATIA failed and later workloads were not run. Missing baseline rows are unavailable, not zero. Ratios use arithmetic means of valid completed composites.

| Fresh Energy arm | Run | Verdict | Guest 153 / 4101 | New host Xids | Restored |
|---|---|---|---|---|---|
| E1 | 4401 | FAIL | 3 / 0 | 0 | True |
| E2 | 4402 | PASS | 0 / 0 | 0 | True |
| E3 | 4403 | PASS | 0 / 0 | 0 | True |
| E4 | 4404 | PASS | 0 / 0 | 0 | True |
