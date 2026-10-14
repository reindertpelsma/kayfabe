
### /var/lib/kf-windows-20261005/claude-default-matrix-ec45aad99/b6

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| llama_cuda_gen | - | PASS | - | proof out |
| llama_vulkan_bench | - | PASS | - | proof out,pdh:copy |
| mandelbrot_t | - | PASS | - | proof out |
| matmul_t | - | PASS | - | proof out |
| memcpy2d_t | - | PASS | - | proof out |
| nbody_demo_bench | - | PASS | - | proof out |
| nbody_t | - | PASS | - | proof out |
| nvdec_h264 | - | PASS | - | 3 guest TDR; gsp_cycles=2; proof pdh:videodecode |

PASS: 8  (total 8)
per category: cuda-demo: 1/1; cuda-equiv: 4/4; llm: 2/2; video: 1/1
guest TDR events in passing+failing apps: 3; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
