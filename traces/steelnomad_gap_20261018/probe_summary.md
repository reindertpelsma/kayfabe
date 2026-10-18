| probe (median of per-run medians [min..max over 3 runs]) | kayfabe | VFIO native | ratio |
|---|---|---|---|
| A empty-queue Signal round trip, polled | 1.4 [1.3..1.6] (n=3) | 1.4 [1.4..1.7] (n=3) | 1 |
| A2 64KiB-copy + Signal round trip, polled | 837.4 [823.2..929.4] (n=3) | 61 [60.4..61.4] (n=3) | 13.7 |
| B empty-queue Signal round trip, event-wait | 2.6 [2.5..5.4] (n=3) | 2.8 [2.7..3.3] (n=3) | 0.929 |
| B2 64KiB-copy + Signal round trip, event-wait | 879.4 [876.1..979.5] (n=3) | 69.9 [69.8..69.9] (n=3) | 12.6 |
| C vram->vram copy 256MiB GiB/s | 120.9 [113.8..120.9] (n=3) | 205.5 [205.2..205.8] (n=3) | 0.588 |
| D upload(sysmem)->vram copy 128MiB GiB/s | 22.9 [22.6..22.9] (n=3) | 24.6 [24.6..24.6] (n=3) | 0.931 |
| E vram->readback copy 128MiB GiB/s | 22.6 [21.9..22.6] (n=3) | 24.5 [24.4..24.5] (n=3) | 0.922 |
| F DIRECT-signal -> COPY-wait -> COPY-signal, polled | 3.8 [3.8..3.8] (n=3) | 3.9 [3.9..4.3] (n=3) | 0.974 |
| G 64 lists in ONE ExecuteCommandLists + Signal, polled | 1138 [1137..1231] (n=3) | - | - |
| G0 1 list + Signal, polled (same loop) | 867.5 [862.8..950.8] (n=3) | - | - |
| H 64 x ExecuteCommandLists(1 list) + one Signal, polled | 3.556e+04 [3.553e+04..3.563e+04] (n=3) | - | - |
| I vram->vram 16MiB round trip | 881.4 [870.6..1000] (n=3) | - | - |
| I vram->vram 1MiB round trip | 863.7 [834..883] (n=3) | - | - |
| I vram->vram 256MiB round trip | 2072 [2066..2180] (n=3) | - | - |
| I vram->vram 4MiB round trip | 829.2 [816.2..927] (n=3) | - | - |
| I vram->vram 64KiB round trip | 852.3 [822.2..934.4] (n=3) | - | - |
| I vram->vram 64MiB round trip | 1114 [1107..1173] (n=3) | - | - |
| J CPU time inside ExecuteCommandLists(1 tiny list) | 16.1 [13.9..30.7] (n=3) | - | - |
| J CPU time inside Signal | 2.4 [2..4.7] (n=3) | - | - |
| J then wait for the fence (poll) | 819 [788..850.6] (n=3) | - | - |
