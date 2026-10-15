
### /var/lib/kf-windows-20261005/claude-cpuexp-ec45aad99/expE2_enl_d3d12_cupy

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| gravitymark_d3d12 | - | FAIL | - | rc=0 180s quiet=1 — no-gpu-proof |
| cupy_asynccopy | - | PASS | - | proof out |

PASS: 1, FAIL: 1  (total 2)
per category: cuda-equiv: 1/1; d3d: 0/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
