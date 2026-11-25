| boot | cycle | verdict | cycle start (UTC) | guest Kernel-Power 42, entering sleep (UTC) | QMP suspended after request (s) | wake (UTC) | ssh back (s) | nvlddmkm 153 | kf3 KEPT/GSP-RESUME | explorer | nvidia-smi | reasons |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| H1_b1 | 1 | CLEAN | 00:05:50Z | 00:05:52 | 6 | 00:05:59 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H1_b1 | 2 | CLEAN | 00:09:13Z | 00:09:16 | 6 | 00:09:23 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H1_b1 | 3 | CLEAN | 00:12:37Z | 00:12:39 | 2 | 00:12:42 | 15 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H1_b1 | 4 | CLEAN | 00:16:04Z | 00:16:07 | 3 | 00:16:12 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H1_b1 | 5 | CLEAN | 00:19:23Z | 00:19:24 | 2 | 00:19:28 | 7 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H1x_b1 | 1 | CLEAN | 00:48:38Z | 00:48:40 | 5 | 00:48:47 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H1x_b1 | 2 | CLEAN | 00:52:00Z | 00:52:03 | 4 | 00:52:08 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H1x_b1 | 3 | CLEAN | 00:55:22Z | 00:55:24 | 2 | 00:55:28 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H2_b1 | 1 | STORM | 00:25:14Z | - | 7 | 00:25:25 | None | - | 1/0 | None | - | no ssh within 150 s |
| H2_b2 | 2 | STORM | 00:30:06Z | - | 7 | 00:30:17 | None | - | 1/0 | None | - | no ssh within 150 s |
| H3_b1 | 1 | CLEAN | 00:35:03Z | 00:35:05 | 7 | 00:35:14 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H3_b1 | 2 | CLEAN | 00:38:30Z | 00:38:32 | 6 | 00:38:40 | 14 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |
| H3_b1 | 3 | CLEAN | 00:42:02Z | 00:42:02 | 3 | 00:42:08 | 8 | 0 | 1/1 | 1 | RTX 4070, 580.88 | - |

| boot | host dmesg mmuWalkMap lines | host NV_ERR_NO_MEMORY lines | QEMU log 0x51 lines | QMP events |
|---|---|---|---|---|
| H1_b1 | 1 | 20 | 0 | RTC_CHANGE RESET RTC_CHANGE NIC_RX_FILTER_CHANGED SUSPEND WAKEUP SUSPEND WAKEUP SUSPEND WAKEUP SUSPEND WAKEUP SUSPEND WAKEUP SHUTDOWN |
| H1x_b1 | 0 | 0 | 0 | RTC_CHANGE RESET RTC_CHANGE NIC_RX_FILTER_CHANGED SUSPEND WAKEUP SUSPEND WAKEUP SUSPEND WAKEUP SHUTDOWN |
| H2_b1 | 1 | 18 | 0 | RTC_CHANGE RESET RTC_CHANGE NIC_RX_FILTER_CHANGED SUSPEND WAKEUP SHUTDOWN |
| H2_b2 | 1 | 18 | 0 | RTC_CHANGE RESET RTC_CHANGE NIC_RX_FILTER_CHANGED SUSPEND WAKEUP SHUTDOWN |
| H3_b1 | 1 | 20 | 0 | RTC_CHANGE RESET RTC_CHANGE NIC_RX_FILTER_CHANGED SUSPEND WAKEUP SUSPEND WAKEUP SUSPEND WAKEUP SHUTDOWN |
