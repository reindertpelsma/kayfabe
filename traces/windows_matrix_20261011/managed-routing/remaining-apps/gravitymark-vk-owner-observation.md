# GravityMark Vulkan device-lost dialog, run1288

**STATUS: LIVE, 2026-10-11.** Owner reported the dialog during run1288;
confirmed by a QMP screenshot at18:20UTC without sending input or opening a
second QGA client. Product/harness72741e2b, RTX4070, guest580.88.

Dialog transcription from shots/gravitymark_vk-owner-device-lost.png:

```text
VK::error(): device lost
VKContext::Frame::submit(): can't submit command buffer
VKContext::submit(): can't submit frame
VKWindow::present(): can't submit context
GravityMark::render(): can't present window

Run-time error
```

Measured: bounded kernel journal records host Xid32 at18:17:10UTC for this
QEMU PID240656, channel0x4e, intr00800000. The QEMU snapshot records a subsequent
Running→Suspending→Halted→ProtectedRegionUp→Booted→Running cycle. The faulting
host channel corresponds to guest0xc1d0007a:0xff040025, Windows user-work
Passthrough twin. Snapshot and filtered records saved alongside. The exact
offending method and initiating defect are unmeasured; shared raw Xid bits
with other workloads do not establish the same cause.

At the18:20UTC collection the original600s test was still active. Final result
at18:26:46UTC is TIMEOUT611s/rc124: the application stayed open with the dialog,
three guest driver153 records, WER0, no guest reboot, one GSPcycle. Saved app
facts/verdict and end screenshot retain the original failure. No dialog dismissal
or deadline change was used.

Reference580.88/RTX4090 previously TIMEOUT603s with a blank window. The same
device-lost dialog, host fault or cause has NOT been established there. A
reference timeout does not explain away this candidate GPU fault.
