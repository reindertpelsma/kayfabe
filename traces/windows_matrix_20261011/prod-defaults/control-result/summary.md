
### /var/lib/kf-windows-20261005/codex-prod-defaults-control-20261011-3252a087

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| glgears_wgl | - | PASS | - | proof out |
| vectorAdd_demo | - | PASS | - | proof cuda-vectoradd-readback |
| cup8_ladder | - | PASS | - | proof cuda-cup8-readback |

PASS: 3  (total 3)
per category: cuda-demo: 1/1; cuda-ladder: 1/1; opengl: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
