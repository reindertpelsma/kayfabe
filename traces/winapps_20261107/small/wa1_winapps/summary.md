
### /root/prov/wa1_winapps/run

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| nvidia_smi | - | PASS | - | proof out |
| deviceQuery_demo | - | PASS | - | proof out |
| matmul_t | - | PASS | - | proof out |
| cup8_ladder | - | PASS | - | proof cuda-cup8-readback |
| dxprobe_d3d11 | - | PASS | - | proof out |
| glgears_wgl | - | PASS | - | proof out |
| vulkaninfo | - | PASS | - | proof out |
| torch_correct | - | PASS | - | proof out |

PASS: 8  (total 8)
per category: cuda-demo: 1/1; cuda-equiv: 1/1; cuda-ladder: 1/1; d3d: 1/1; opengl: 1/1; smi: 1/1; torch: 1/1; vulkan: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
