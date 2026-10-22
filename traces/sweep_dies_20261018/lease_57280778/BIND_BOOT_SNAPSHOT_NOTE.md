MEASURED (2026-10-22, lease57280778, `BIND_BOOT_SNAPSHOT.json`): read-only QMP reports owned PID102996 running; guest-ping timed out after5s, with zero guest-driver RPCs in the QEMU log at23:34:54Z. Generic standard-VGA installation and checks passed (`bind_wait/sweep_windows_install.log`).

MEASURED (2026-10-22, lease57280778): serial log contained one Windows Boot Manager start and product reset counter1 (main1/vcpu0) at23:39:24Z. A later Boot Manager screenshot does not independently establish a reboot.

INFERRED (2026-10-22): the bind boot is waiting before a responsive guest agent or NVIDIA initialization; its exact cause remains unknown. This is a live observation, not a completed readiness verdict or a driver failure claim. No guest memory, disassembly, closed binary analysis, policy relaxation or timeout change was used.
