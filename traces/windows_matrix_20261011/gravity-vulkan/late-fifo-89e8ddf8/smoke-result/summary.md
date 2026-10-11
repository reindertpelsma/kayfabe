
### /var/lib/kf-windows-20261005/codex-late-fifo-20261011-89e8ddf8/smoke

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| glgears_wgl | - | PASS | - | proof out |
| vectorAdd_demo | - | PASS | - | proof cuda-vectoradd-readback |
| cup8_ladder | - | PASS | - | proof cuda-cup8-readback |
| torch_ai_bench | - | FAIL | - | rc=- 411s quiet=- — guest-rebooted (bugcheck?); gsp_cycles=1 |

PASS: 3, FAIL: 1  (total 4)
per category: cuda-demo: 1/1; cuda-ladder: 1/1; opengl: 1/1; torch: 0/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
