# Same-GPU GravityMark Vulkan VFIO reference

**STATUS: RESEARCH, 2026-10-11.** Hardware pending.

Use the RTX4070 at172.22.1.20 with the existing NVIDIA580.88 Windows baseline,
exclusive GPU lock, and the existing GSP-observer reference QEMU. The adapter
changes VM launch/stop only; it imports pinned product72741e2b's matrix lane,
same GravityMark1.89 workload,600s deadline, proof oracle, MPOoff and app disk.
Kayfabe product code is absent from this reference device path. The GSP JSONL
is retained for schema-aware analysis; the kayfabe phase parser does not apply,
so the matrix records GSPcycles as unmeasured, never zero by assumption.

Hypothesis: kayfabe introduces the device-lost/recovery symptom on this workload.
Falsifier: the same symptom appears on this same-GPU VFIO reference. A timeout
without device lost differs and is not a clean baseline pass; a clean reference
pass supports a kayfabe defect but does not identify its initiating method.

The reference precedes the already queued3252 regression/manual campaign.
That campaign's existing commands, exact product and checks stay frozen in a
separate copied script. Its original waiting process retains the GPU lock and
manual lifetime; the after-manual allocator job still waits for that same PID.
No owner session input or interruption. A failed reference app does not mask its
verdict or cancel independent regression; unsafe cleanup prevents the campaign.
