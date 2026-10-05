# Native VFIO comparison for the b297 allocation investigation

**STATUS: RESEARCH, 2026-10-05.** Windows 580.88 on the borrowed RTX 4070,
with the physical GPU assigned to an independent clone of the preserved native
fixture. This is not Windows running through Kayfabe.

The native device was healthy before and after PnP restart, and nvidia-smi passed.
The passive observer collected and drained 3,704 validated records from two queue
allocations, with zero reported FIFO drops or sequence gaps after attachment.
Both prefixes are unknown: first observed request sequences are 3714 and 2868.
It again missed class 0xb297 allocations and the pool-size query. Absence from
this capture is not evidence those calls did not occur.

The trace contains 23 request/reply pairs for 0x20801111, all 40-byte parameters
and successful statuses. Parameters are retained privately for comparison with
the pinned retail driver's consumer. They do not justify replaying the control
on a host, or returning success without scheduling real work. The validated
text export is saved on the controller; hashes/counts are in summary.json.
No raw memory dump or executable was copied back.

Two harness defects are explicitly preserved: the initial readiness helper
tried opening the observer while the collector owned its exclusive handle,
causing Win32 error 5. The collector continued normally; progress.ps1 checked
its written records and running task before restart. start-fixed.ps1 records
the correction, but was not rerun for this capture. Second, PowerShell
Start-Process lost the pnputil exitcode (null); restart text says successful
and independent device status/nvidia-smi are healthy. Do not claim a captured
zero pnputil exitcode. The capture itself exited 0 and drained fully.

The observer was stopped and its task disabled, then Windows was asked to
shut down cleanly. The VFIO supervisor restores the original Linux binding
and display services after QEMU exits; see the final host journal separately.

The supervisor did restore the Linux NVIDIA driver and original services, and
exited 0. Host nvidia-smi reports the RTX 4070 on 595.91.07 afterward.
