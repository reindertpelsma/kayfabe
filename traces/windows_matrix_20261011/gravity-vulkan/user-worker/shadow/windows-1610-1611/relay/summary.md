
### /var/lib/kf-windows-20261005/codex-user-shadow-windows-d8afb1e3c/relay

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| edge_video_h264 | - | PASS | - | 3 guest TDR; gsp_cycles=1; proof pdh:videodecode |
| gravitymark_d3d12 | - | FAIL | - | rc=1 19s quiet=0 — rc:1,success-string-missing,no-gpu-proof | GM_FAIL no benchmark image; 3 guest TDR; gsp_cycles=1 |
| gravitymark_vk | - | BOOT_FAIL | - | rc=- 0s quiet=- — guest-did-not-start:pre-guest hook failed rc:1 |

PASS: 1, FAIL: 1, BOOT_FAIL: 1  (total 3)
per category: d3d: 0/1; video: 1/1; vulkan: 0/1
guest TDR events in passing+failing apps: 6; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
