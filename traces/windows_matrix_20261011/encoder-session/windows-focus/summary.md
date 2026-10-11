
### /var/lib/kf-windows-20261005/codex-encoder-windows-focus-20261011-a61af1c3

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| glgears_wgl | - | PASS | - | proof out |
| nvenc_h264 | - | PASS | - | 3 guest TDR; gsp_cycles=1; proof pdh:videoencode |
| nvenc_hevc | - | PASS | - | 3 guest TDR; gsp_cycles=1; proof pdh:videoencode |
| nvenc_av1 | - | PASS | - | 3 guest TDR; gsp_cycles=1; proof pdh:videoencode |

PASS: 4  (total 4)
per category: opengl: 1/1; video: 3/3
guest TDR events in passing+failing apps: 9; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
