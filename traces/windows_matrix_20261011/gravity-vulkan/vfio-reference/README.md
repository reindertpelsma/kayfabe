# Same-GPU GravityMark Vulkan VFIO reference

**STATUS: ANSWERED BASELINE, 2026-10-11.** Same-GPU workload PASS; kayfabe defect remains open.

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

## Measured result, 2026-10-11 19:43UTC

GravityMark1.89 Vulkan PASS183s/rc0. Image864279bytes and118694 timing lines,
NVIDIA3D98.43%/copy44.80%, zero non-NVIDIA engines, guest153/display4101/WER0,
no reboot. Original600s workload; unchanged oracle. Same RTX4070 and guest580.88
as kayfabe; reference guest has physical12GiB VRAM versus declared4GiB on kayfabe,
so capacity/environment are explicit differences. This falsifier did not occur:
reference worked, while kayfabe1361 times out with hostXid32 and recovery.
A kayfabe or declared-device-path defect remains; no specific cause proven.

Observer has21837 records, zero malformed/declared missing records, generations
1 and2, but capture_complete=false. Whole-boot counts and generation transitions
are not an app-scoped GSP recovery count; that metric remains unmeasured.
There are2 request/reply pairs for50800104, both reply status0,584byte params,
clientc1d00076. Public deferred-API bundle identifies inner20801116 channel-info
update, API handles40000028/29. Raw trace and bounded parser summary preserved.
This is successful registration, not an independently measured trigger completion.
No initiating GPU method or root cause identified by these control counts.

Shutdown restored NVIDIA595.91.07 and DMA-FQ, no QEMU; exact bounded host kernel
window reports0Xid. Reference wrapper exit0 at19:47:15UTC. Only GSP observer data,
command metadata and lane facts are backed up here; input-event trace is omitted
because it records password key events. Guest RAM and disk are not published.

The subsequent unprivileged RC count query refused0x1b; it did not report an
empty journal. The frozen3252 regression then started. Its Linux gates9/9,
gate10 and fast30/30 pass; remaining Windows controls/manual session pending.
