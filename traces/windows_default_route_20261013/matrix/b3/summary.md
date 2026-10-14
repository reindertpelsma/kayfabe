
### /var/lib/kf-windows-20261005/claude-default-matrix-ec45aad99/b3

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| edge_video_vp9 | - | PASS | - | 3 guest TDR; gsp_cycles=1; proof pdh:videodecode |
| edge_webgl | - | PASS | - | proof out |
| edge_webgl_headless | - | PASS | - | proof out |
| edge_webgpu | - | PASS | - | proof out |
| fft_t | - | PASS | - | proof out |
| furmark_gl_bench | - | PASS | - | proof out |
| furmark_gl_stress | - | PASS | - | proof out |
| furmark_glinfo | - | PASS | - | proof out |

PASS: 8  (total 8)
per category: browser: 3/3; cuda-equiv: 1/1; opengl: 2/2; stress: 1/1; video: 1/1
guest TDR events in passing+failing apps: 3; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
