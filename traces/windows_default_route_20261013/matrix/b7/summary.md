
### /var/lib/kf-windows-20261005/claude-default-matrix-ec45aad99/b7

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| nvenc_av1 | - | PASS | - | 3 guest TDR; gsp_cycles=1; proof pdh:videoencode |
| nvenc_h264 | - | PASS | - | proof pdh:videoencode |
| nvenc_hevc | - | PASS | - | proof pdh:videoencode |
| nvidia_smi | - | PASS | - | proof out |
| oceanFFT_demo | - | PASS | - | proof cuda-oceanfft-readback |
| opencl_icd | - | PASS | - | proof out |
| ort_directml | - | PASS | - | proof out |
| randomFog_demo | - | PASS | - | proof cuda-randomfog-readback |

PASS: 8  (total 8)
per category: cuda-demo: 2/2; d3d: 1/1; opencl: 1/1; smi: 1/1; video: 3/3
guest TDR events in passing+failing apps: 3; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
