
### /var/lib/kf-windows-20261005/codex-matrix-full-20261011

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| atomics_t | - | PASS | - | proof out |
| bandwidthTest_demo | - | PASS | - | proof out |
| blackscholes_t | - | FAIL | - | rc=-1073741819 108s quiet=107 — rc:-1073741819,success-string-missing,no-gpu-proof; 3 guest TDR; gsp_cycles=1 |
| blender_cycles | - | PASS | - | proof out |
| cg_t | - | PASS | - | proof out |
| clpeak_cuda | - | TIMEOUT | - | rc=124 1501s quiet=1488 — timeout |

PASS: 4, FAIL: 1, TIMEOUT: 1  (total 6)
per category: cuda-demo: 1/1; cuda-equiv: 2/3; opencl: 0/1; render: 1/1
guest TDR events in passing+failing apps: 3; WER reports: 0

Linux rows with no Windows equivalent (6): cdpSimpleQuicksort, geekbench_gpu, hf_generate, simpleCallback, simpleIPC, simpleOccupancy
