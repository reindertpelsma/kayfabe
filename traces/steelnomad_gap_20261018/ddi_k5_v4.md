| function | calls/s k5 | calls/s v4 | median us k5 | median us v4 | p99 us k5 | p99 us v4 | total ms/s k5 | total ms/s v4 |
|---|---|---|---|---|---|---|---|---|
| DpiFdoMessageInterruptRoutine | 838.7 | 403.7 | 70.6 | 20.8 | 131.9 | 54.9 | 61.53 | 9.2 |
| DxgkWaitForSynchronizationObject | 47.9 | 40.4 | 4.8 | 4.1 | 10734.3 | 226.0 | 52.06 | 0.41 |
| DdiNotifyDpc | 367.0 | 398.8 | 10.8 | 11.7 | 444.9 | 192.4 | 18.49 | 14.87 |
| DpiDpcForIsr | 357.6 | 362.0 | 12.0 | 11.9 | 458.9 | 194.6 | 18.34 | 13.64 |
| DxgkEscape | 146.3 | 238.0 | 90.3 | 13.1 | 196.1 | 5121.7 | 9.99 | 104.47 |
| DdiEscape | 146.3 | 238.0 | 88.1 | 12.6 | 192.5 | 5118.2 | 9.65 | 104.0 |
| DdiSubmitCommandToHwQueue | 680.8 | 1223.8 | 10.6 | 1.5 | 23.3 | 5.7 | 7.95 | 2.24 |
| DxgkCreateAllocation | 15.0 | 0.8 | 277.4 | 130.2 | 562.1 | 5763.1 | 3.92 | 0.27 |
| DdiSetVidPnSourceAddressWithMultiPlaneOverlay3 | 22.0 | 39.3 | 163.9 | 80.2 | 238.9 | 135.8 | 3.66 | 3.2 |
| DdiCreateAllocation | 15.0 | 0.8 | 255.5 | 88.8 | 384.4 | 5703.8 | 3.43 | 0.24 |
| DxgkSubmitCommandToHwQueue | 681.8 | 1223.6 | 3.1 | 2.9 | 16.4 | 11.7 | 2.68 | 4.28 |
| DdiSubmitCommandVirtual | 46.0 | 4.5 | 30.7 | 11.7 | 64.6 | 73.7 | 1.48 | 0.06 |
| DxgkPresent | 22.1 | 39.3 | 51.7 | 46.2 | 226.8 | 79.0 | 1.45 | 1.85 |
| DdiQueryAdapterInfo | 9.6 | 10.8 | 96.6 | 612.1 | 652.7 | 5453.5 | 1.43 | 10.76 |
| DxgkQueryStatistics | 14.5 | 11.4 | 89.4 | 552.7 | 502.6 | 4709.6 | 1.39 | 8.82 |
| DdiSetInterruptTargetPresentId | 22.3 | 39.6 | 35.6 | 16.7 | 52.4 | 30.1 | 0.8 | 0.68 |
| DxgkSubmitWaitForSyncObjectsToHwQueue | 245.0 | 438.0 | 2.5 | 2.4 | 8.4 | 7.8 | 0.72 | 1.19 |
| DxgkSubmitSignalSyncObjectsToHwQueue | 139.8 | 235.8 | 3.5 | 3.2 | 17.4 | 16.0 | 0.69 | 1.08 |
| DdiBuildPagingBuffer | 227.8 | 12.0 | 2.2 | 2.3 | 10.4 | 12.4 | 0.62 | 0.04 |
| DdiValidateSubmitCommand | 681.8 | 1223.6 | 0.6 | 0.5 | 3.0 | 2.6 | 0.49 | 0.79 |
