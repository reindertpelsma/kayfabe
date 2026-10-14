
### /var/lib/kf-windows-20261005/codex-corrected-apps-20261011-a61af1c3

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| oceanFFT_demo | - | PASS | - | proof cuda-oceanfft-readback |
| randomFog_demo | - | PASS | - | proof cuda-randomfog-readback |
| hf_generate | - | PASS | - | proof out |
| cdp_quicksort | - | PASS | - | proof out |
| vectorAdd_demo | - | PASS | - | proof cuda-vectoradd-readback |
| cup8_ladder | - | PASS | - | proof cuda-cup8-readback |
| stream_perthread_t | - | TIMEOUT | - | rc=124 121s quiet=46 — timeout |
| cupy_sha256 | - | PASS | - | proof out |
| cupy_cg | - | PASS | - | proof out |
| cupy_asynccopy | - | PASS | - | proof out |
| cupy_check | - | PASS | - | proof out |

PASS: 10, TIMEOUT: 1  (total 11)
per category: ai: 1/1; cuda-demo: 3/3; cuda-equiv: 4/4; cuda-ladder: 1/1; stream-probe: 0/1; torch: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
