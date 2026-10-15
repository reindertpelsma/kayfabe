
### /var/lib/kf-windows-20261005/claude-default-matrix-ec45aad99/b2

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| d3d12_signal_probe | - | PASS | - | proof out |
| deviceQuery_demo | - | PASS | - | proof out |
| driver_status | - | PASS | - | proof out |
| dxdiag_report | - | PASS | - | proof out |
| dxgi_adapters | - | PASS | - | proof out |
| dxprobe_d3d11 | - | PASS | - | proof out |
| dxprobe_d3d12 | - | PASS | - | proof out |
| dxva2_h264 | - | PASS | - | proof pdh:videodecode |

PASS: 8  (total 8)
per category: cuda-demo: 1/1; d3d: 3/3; probe: 3/3; video: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
