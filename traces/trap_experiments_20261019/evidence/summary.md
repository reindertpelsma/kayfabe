| arm | rev | flags (experiment) | SN n | SN score median [min,max] | fps (score/100) | DIRECT / COMPUTE / COPY us (n) | Xid delta | 4101/153 | error |
|---|---|---|---|---|---|---|---|---|---|
| control1 | 02911cbb-ctl | - | 5 | 2936 [2885,2963] | 29.36 | 266 / 229 / 192 (10) | 0 | tdr_4101=0 gpu_153=0 app_crashes=0 |  |
| control2 | 02911cbb-ctl | - | 5 | 3006 [2904,3034] | 30.06 | 277 / 272 / 195 (10) | 0 | tdr_4101=0 gpu_153=0 app_crashes=0 |  |
| db10 | 02911cbb | KF3_DIAG_TRAP_DELAY_DB_US=10 | 5 | 2967 [2923,2995] | 29.67 | 259 / 253 / 205 (10) | 0 | tdr_4101=0 gpu_153=0 app_crashes=0 |  |
| db20 | 02911cbb | KF3_DIAG_TRAP_DELAY_DB_US=20 | 1 | 3027 [3027,3027] | 30.27 | 267 / 269 / 217 (10) | 0 | tdr_4101=0 gpu_153=0 app_crashes=0 | no result for run 2 |
| intr10 | 02911cbb | KF3_DIAG_TRAP_DELAY_INTR_US=10 | 5 | 2954 [2947,2979] | 29.54 | 427 / 378 / 338 (10) | 0 | tdr_4101=0 gpu_153=0 app_crashes=0 |  |
| intr100 | 02911cbb | KF3_DIAG_TRAP_DELAY_INTR_US=100 | 5 | 2951 [2913,2977] | 29.51 | 1711 / 1652 / 1611 (5) | 0 | tdr_4101=0 gpu_153=0 app_crashes=0 |  |
| intr20 | 02911cbb | KF3_DIAG_TRAP_DELAY_INTR_US=20 | 5 | 2967 [2910,3024] | 29.67 | 576 / 524 / 482 (10) | 0 | tdr_4101=0 gpu_153=0 app_crashes=0 |  |
| spin | 02911cbb | KF3_DIAG_INTR_SPIN=1 KF3_DIAG_INTR_SPIN_CPU=23 | 0 | - | - | - (None) | 0 |  | agent never came up |
| spinb | 1bd33400 | KF3_DIAG_INTR_SPIN=2 KF3_DIAG_INTR_SPIN_CPU=23 | 0 | - | - | - (0) | 0 | tdr_4101=0 gpu_153=0 app_crashes=0 | no result for run 1 |
| spinb3 | cacfcc62 | KF3_DIAG_INTR_SPIN=2 KF3_DIAG_INTR_SPIN_CPU=23 KF3_DIAG_INTR_SPIN_HOLD_US=2000 | 0 | - | - | - (None) | 0 |  | agent never came up |

interrupt-tree delay: SN score slope 0.04 per added us (fps/us = 0.0004)
  tiny probe DIRECT slope 14.35 us per added us
  tiny probe COMPUTE slope 14.19 us per added us
  tiny probe COPY slope 14.16 us per added us

doorbell delay: SN score slope 4.55 per added us (fps/us = 0.0455)
  tiny probe DIRECT slope 0.04 us per added us
  tiny probe COMPUTE slope 2.01 us per added us
  tiny probe COPY slope 1.22 us per added us
