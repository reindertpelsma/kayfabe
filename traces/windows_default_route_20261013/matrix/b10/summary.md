
### /var/lib/kf-windows-20261005/claude-default-matrix-ec45aad99/b10

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| triad_t | - | PASS | - | proof out |
| vkpeak | - | PASS | - | proof out,pdh:copy |
| vulkan_video_decode | - | PASS | - | 3 guest TDR; gsp_cycles=1; proof pdh:videodecode |
| vulkaninfo | - | PASS | - | proof out |
| zerocopy_t | - | PASS | - | proof out |

PASS: 5  (total 5)
per category: cuda-equiv: 2/2; video: 1/1; vulkan: 2/2
guest TDR events in passing+failing apps: 3; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
