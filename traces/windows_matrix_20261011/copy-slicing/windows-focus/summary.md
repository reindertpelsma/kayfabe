
### /var/lib/kf-windows-20261005/codex-matrix-slice-focus-20261011

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| blackscholes_t | - | TIMEOUT | - | rc=124 132s quiet=130 — timeout |
| cuda_callback | - | PASS | - | proof out |
| cuda_occupancy | - | PASS | - | proof out |
| deviceQuery_demo | - | PASS | - | proof out |
| glgears_wgl | - | TIMEOUT | - | rc=124 197s quiet=- — timeout; gsp_cycles=1 |
| matmul_t | - | PASS | - | proof out |

PASS: 4, TIMEOUT: 2  (total 6)
per category: cuda-demo: 1/1; cuda-equiv: 3/4; opengl: 0/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (4): cdpSimpleQuicksort, geekbench_gpu, hf_generate, simpleIPC
