
### /var/lib/kf-windows-20261005/codex-managed-route-focus-20261011-72741e2b

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| managed_t | - | TIMEOUT | - | rc=124 180s quiet=36 — timeout |
| glgears_wgl | - | PASS | - | proof out |
| vectorAdd_demo | - | PASS | - | proof cuda-vectoradd-readback |
| cup8_ladder | - | PASS | - | proof cuda-cup8-readback |

PASS: 3, TIMEOUT: 1  (total 4)
per category: cuda-demo: 1/1; cuda-equiv: 0/1; cuda-ladder: 1/1; opengl: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
