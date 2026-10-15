
### /var/lib/kf-windows-20261005/claude-vulkan-default-windows-ec45aad99/relay

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| edge_video_h264 | - | PASS | - | gsp_cycles=1; proof pdh:videodecode |
| gravitymark_d3d12 | - | FAIL | - | rc=0 183s quiet=1 — no-gpu-proof |
| vectorAdd_demo | - | PASS | - | proof cuda-vectoradd-readback |
| gravitymark_vk | - | PASS | - | proof pdh:copy |

PASS: 3, FAIL: 1  (total 4)
per category: cuda-demo: 1/1; d3d: 0/1; video: 1/1; vulkan: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
