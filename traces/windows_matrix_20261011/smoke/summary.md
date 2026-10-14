
### /var/lib/kf-windows-20261005/codex-matrix-smoke-20261011

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| nvidia_smi | - | PASS | - | proof out |
| deviceQuery_demo | - | PASS | - | proof out |
| glgears_wgl | - | PASS | - | proof out |
| matmul_t | - | PASS | - | proof out |

PASS: 4  (total 4)
per category: cuda-demo: 1/1; cuda-equiv: 1/1; opengl: 1/1; smi: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (6): cdpSimpleQuicksort, geekbench_gpu, hf_generate, simpleCallback, simpleIPC, simpleOccupancy
