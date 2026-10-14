
### /var/lib/kf-windows-20261005/codex-managed-remaining-apps-20261011-72741e2b

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| valley_opengl | - | TIMEOUT | - | rc=124 272s quiet=- — timeout; gsp_cycles=1 |
| vkcube | - | FAIL | - | rc=0 16s quiet=15 — no-gpu-proof |
| llama_cuda_gen | - | PASS | - | proof out |
| furmark_gl_stress | - | FAIL | - | rc=0 65s quiet=64 — no-gpu-proof |
| torch_burn | - | PASS | - | proof out |
| dxva2_h264 | - | PASS | - | proof pdh:videodecode |
| dxdiag_report | - | PASS | - | proof out |
| llama_cuda_bench | - | PASS | - | proof out |
| llama_vulkan_bench | - | PASS | - | proof out |
| blender_cycles | - | PASS | - | proof out |
| vkpeak | - | PASS | - | proof out |
| gravitymark_d3d12_raster | - | FAIL | - | rc=0 188s quiet=188 — no-gpu-proof |
| gravitymark_d3d12_rt | - | FAIL | - | rc=1 33s quiet=0 — rc:1,success-string-missing,no-gpu-proof | GM_FAIL no benchmark image; 3 guest TDR; gsp_cycles=1 |
| torch_ai_bench | - | FAIL | - | rc=- 450s quiet=- — guest-rebooted (bugcheck?); gsp_cycles=1 |
| gravitymark_d3d12 | - | FAIL | - | rc=1 28s quiet=1 — rc:1,success-string-missing,no-gpu-proof | GM_FAIL no benchmark image; 3 guest TDR; gsp_cycles=1 |
| gravitymark_gl | - | FAIL | - | rc=-1073740791 58s quiet=58 — rc:-1073740791,success-string-missing,no-gpu-proof | GM_FAIL no benchmark image; 3 guest TDR; gsp_cycles=1 |
| gravitymark_d3d11 | - | FAIL | - | rc=0 189s quiet=1 — no-gpu-proof |
| clpeak_ocl | - | FAIL | - | rc=-1073741819 350s quiet=4 — rc:-1073741819 | Failed to read file: C:\Windows\TEMP\dep-eb4b8e.d; 3 guest TDR; gsp_cycles=1 |
| gravitymark_vk | - | TIMEOUT | - | rc=124 611s quiet=0 — KFWAIT gravitymark_vk STILL_RUNNING_AFTER 600 s; 3 guest TDR; gsp_cycles=1 |

PASS: 8, FAIL: 9, TIMEOUT: 2  (total 19)
per category: d3d: 0/4; llm: 3/3; opencl: 0/1; opengl: 0/2; probe: 1/1; render: 1/1; stress: 1/2; torch: 0/1; video: 1/1; vulkan: 1/3
guest TDR events in passing+failing apps: 15; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
