# Ada first desktop arm

STATUS: PARTIAL, 2026-10-22; a second fresh child is running.

MEASURED on 2026-10-22, lease57193877 / source170054339, `first_desktop_stopped/arm_ad102_s4_diag.txt`: Windows580.88 Code0, 1920x1080 at10:56:53UTC. `desktop_signed_in.png` shows Explorer desktop; setup.json records Explorer, KFWIN\kf and NVML580.88.
MEASURED on 2026-10-22, lease57193877, `first_desktop_stopped/arm_ad102_s4_matrix.log`: USB CD attached in QEMU but no KFAPPS volume appeared; zero application rows ran. No matrix, Unigine or hold acceptance.
MEASURED on 2026-10-22, lease57193877, `first_desktop_stopped/arm_ad102_s4_live_events.txt`: one1108750336-byte WATCHDOG dump at10:56:35.5976738UTC; Application WER LiveKernelEvent193 at10:56:56.4634652UTC,10:56:56.6140658UTC and10:58:37.6349716UTC, reason text815. Queries found no nvlddmkm/Display/System4101/System1001 records. Kernel-Boot153 was not queried in this export.
CODE READ: Microsoft identifies [0x193](https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-0x193--video-dxgkrnl-livedump) as a graphics live-dump identifier rather than a bugcheck. The specific reason remains unclassified; this is not a zero-anomaly desktop claim. No dump code, disassembly or proprietary-driver binary was inspected.
MEASURED on 2026-10-22, lease57193877, `first_desktop_event_correlation.json`: retained nearest observer-time RPCs bracket the dump/report times; this establishes timing correlation, not the causal RPC.
CODE READ: the native Windows reference used the inbox read-only ISO mount API; S4 now reuses it, with complete guest SHA/size validation and existing verified probe overrides.
MEASURED on 2026-10-22, lease57193877, `guest_iso_arm_receipt.json`: parent hash unchanged; a second fresh disabled-driver child started after rechecking the unchanged native-five baseline. No native test or driver binding was replayed.
