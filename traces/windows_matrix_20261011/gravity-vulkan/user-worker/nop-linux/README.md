# Exact synchronized-NOP correction: native and Linux

STATUS: MEASURED, 2026-10-12. Source4ec9f9a2415f565a50fd57faa82b071afd799258.
RTX4070/host595.91.07. Native synchronized barrier probes2/2PASS, uid65534,
real GP_GET/PUT and copy/reference checked, no early reference/nohostXid.
Linux gates9/9+gate10, fresh exact QEMU/client/fastguest, fast30/30/crash0/NOTRUN0.
Five expected negative-test Xid31; no32. NVIDIA580.159.04 desktop/gears58.823–60.006FPS
PASS_WITH_WARNINGS: existing startup assertions/DRMflip-timeout. No desktop hostXid.
Stopped18:52:18UTC/noQEMU/DMA-FQ. Own1570 disposable removed after protected-chain
and complete qcow header dependency audit. Logs and binary hashes retained here.
Windows1580 later rejected~100KiB source buffers; these results do not prove Windows
or migration compatibility. See newest resume entry.
