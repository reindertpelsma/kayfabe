| boot | cycle | verdict | cycle start (UTC) | guest Kernel-Power 42, entering sleep (UTC) | QMP suspended after request (s) | wake (UTC) | ssh back (s) | nvlddmkm 153 | kf3 KEPT/GSP-RESUME | explorer | nvidia-smi | reasons |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| H1_b1 | 1 | CLEAN | 23:55:57Z | 23:56:00 | 6 | 23:56:07 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H1_b1 | 2 | FAIL-OTHER | 23:59:19Z | - | 0 | 23:59:40 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | new boot (LastBootUpTime changed) |

| boot | host dmesg mmuWalkMap lines | host NV_ERR_NO_MEMORY lines | QEMU log 0x51 lines | QMP events |
|---|---|---|---|---|
| H1_b1 | 1 | 20 | 0 | RTC_CHANGE RESET RTC_CHANGE NIC_RX_FILTER_CHANGED SUSPEND WAKEUP SUSPEND WAKEUP SHUTDOWN |
