
### /var/lib/kf-windows-20261005/claude-default-matrix-ec45aad99/b1

| app | linux guest | win (batched) | win (alone) | detail |
|---|---|---|---|---|
| cup2_ladder | - | PASS | - | proof out |
| cup3_ladder | - | PASS | - | proof out |
| cup8_ladder | - | PASS | - | proof cuda-cup8-readback |
| cupy_asynccopy | - | TIMEOUT | - | rc=124 182s quiet=69 — timeout |
| cupy_cg | - | BOOT_FAIL | - | rc=- 0s quiet=- — guest-did-not-start:pre-guest hook failed rc:1 |
| cupy_check | - | BOOT_FAIL | - | rc=- 0s quiet=- — guest-did-not-start:pre-guest hook failed rc:1 |
| cupy_sha256 | - | BOOT_FAIL | - | rc=- 0s quiet=- — guest-did-not-start:pre-guest hook failed rc:1 |

PASS: 3, TIMEOUT: 1, BOOT_FAIL: 3  (total 7)
per category: cuda-equiv: 0/3; cuda-ladder: 3/3; torch: 0/1
guest TDR events in passing+failing apps: 0; WER reports: 0

Linux rows with no Windows equivalent (2): geekbench_gpu, simpleIPC
