# Stage 1 A/A — fast class qualified, independent class pending

STATUS: MEASURED, 2026-10-20 04:09 CEST. Supersedes the earlier stopped status
for the collector; the original stopped-run report remains historical evidence.

MEASURED: real Windows PowerShell transport has a separate EXPECTED RED commit
(four failures, including the exact header exception), a shared recursive plain-
JSON fix (4/4 green), and a bounded header-object mutant killed by three tests.
Delayed PresentMon completion ordering has its own EXPECTED RED pair and sorting
fix; no rows are discarded to bypass ordering. Full harness 59/59 green, including
transport receipts, classification, ownership, archive and role validation.

MEASURED: fresh A runs 3404/3405, product 0d7ba743, owner profile plus
KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5 KF3_TSG_MIRROR=0; completion probe
off, windowed Heaven D3D11 + GravityMark Vulkan CS (1280x720 each, 20k asteroids).
Exact flags, durations, events and per-app intervals are in AA-runs.json.

| Run | Observed s | First nvlddmkm153 s | First stderr device-loss s | Heaven silences / longest s | Gravity silences / longest s |
|---|---:|---:|---:|---|---|
| 3404 | 724.6694 | 61.8825 | 64.2226 | 1 / 43.4974 | 1 / 666.0104 censored |
| 3405 | 725.5358 | 64.4460 | 66.5817 | 1 / 48.2639 | 1 / 664.3130 censored |

MEASURED: n=2 boots, 24.1701 min total; count ratio 1 for each selected app;
first-153 range 61.8825–64.4460 s, three 153 records per run, one device-loss
record per run. Zero malformed rows; both expected clients presented; Xid 21→21
through both clean stops. H-fast's 90s falsifier was not met. H1's factor-three
count check passes for this **device-loss** workload only. Sustained two-client
execution ended when Heaven crashed; these are not twelve-minute healthy-load
runs. Both were cleanly stopped before another boot.

CODE READ / MEASURED: DEVICE_LOST_TDR excludes both windows from the independent-
freeze class. Its zero eligible independent count does not mean independent
freezes are absent. Two-Heaven, Blender and Edge pair trials remain pending.
No product behavior, GPU timeout, timer or GP_PUT poll was changed. An initial
broad serialization mutation timed out on the diagnostic channel and is not
counted as a successful mutation kill; clean ACPI shutdown recovered that test
session before fresh A boots. The bounded mutant and restored implementation
then completed normally before A GPU load.
