
### /var/lib/kf-windows-20261005/claude-default-matrix-ec45aad99/b9

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| stream_perthread_t | - | PASS | - | proof out |
| stream_two_t | - | PASS | - | proof out |
| streams_t | - | PASS | - | proof out |
| tc16_t | - | PASS | - | proof out |
| tcbf16_t | - | PASS | - | proof out |
| torch_burn | - | PASS | - | proof out |
| torch_correct | - | PASS | - | proof out |
| transpose_t | - | PASS | - | proof out |

PASS: 8  (total 8)
per category: cuda-equiv: 4/4; stream-probe: 2/2; stress: 1/1; torch: 1/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
