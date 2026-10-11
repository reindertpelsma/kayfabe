
### /var/lib/kf-windows-20261005/codex-nbody-stall-snapshot-20261011-72741e2b

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| nbody_demo_gl | - | FAIL | - | rc=0 29s quiet=2 — no-gpu-proof |
| gravitymark_vk | - | TIMEOUT | - | rc=124 603s quiet=1 — KFWAIT gravitymark_vk STILL_RUNNING_AFTER 600 s; 3 guest TDR; gsp_cycles=1 |

FAIL: 1, TIMEOUT: 1  (total 2)
per category: cuda-demo: 0/1; vulkan: 0/1
guest TDR events in passing+failing apps: 3; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
