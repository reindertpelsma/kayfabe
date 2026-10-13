# Private USER channel identity for a guest-default declaration

**STATUS: RESEARCH, 2026-10-13.** Written before the Windows run. No Vulkan
pass claimed. Ordinary Linux Passthrough and opaque shadow behavior unchanged.

## Measured result at 3ee0dd10a

Source `3ee0dd10ac89d62c44d64ac9ca412a685503a5ad`, RTX4070/host595.91.07,
Windows580.88, run1630, 2026-10-13 09:05:59–09:11:22UTC: GravityMark Vulkan
PASS181s/rc0, original benchmark image/timing and pdh:copy proof, all guest
153/4101/livekernel/WER/reboot counters0, GSPcycles0, bounded hostXids0.
The worker logs two source migrations:32768→65536→131072 entries. No USER
relay DEAD/refusal. H-IDENTITY's and H-MIGRATION's falsifiers did not occur.
Mid-run screenshot shows the actual Vulkan asteroid scene; end result and
original app/health verdicts are in [windows-1630](windows-1630/relay/summary.md).
Same-GPU VFIO reference183s is retained; this is not a general performance claim.

Exact GPU-free2156PASS/0FAIL/3ignored; GPU gates9/9+gate10PASS, USERbirths12/12,
fastguest30/30/crash0/NOTRUN0. Linux desktop/gears PASS_WITH_WARNINGS (existing
startup assertions/DRM flip timeout), no desktop hostXid. Source/binary hash,
gates/fast/raw guest logs are in linux/ and desktop/. Five hostXid31 records
belong to expected negative Linux suite arms; zero Windows hostXids is a separate
precise campaign window. Host cleanstop/noQEMU/DMA-FQ, personal image untouched.

This pass uses the opt-in relay flag. A default-route change is now under
verification; do not read this evidence as a flag-free Windows result.

## Original pre-run plan

The full worker relay already implements a guarded guest FIFO/USERD source
transition at an intercepted trigger, while its private native ring/USERD stay
fixed. Windows1600 never reached it: default subDeviceId 0 was rejected at
birth. The same-host native zero/id1 fixture passes both, including all excluded
mask bits, stored-mask controls and real copy/fetch/completion checks.

The opt-in Windows worker now authors native bit0 (value1) for guest default0
and supplies exactly that same identity to its scanner. Explicit valid one-hot
identities retain their existing policy; malformed values remain refused. This
is a deliberate private host-channel configuration, not a claim about hidden
guest RAMFC initialization. USER privilege, restricted own VA space, physical
operand denial, source admission, publication guards and all quotas remain.

H-IDENTITY: matched explicit host/scanner identity removes the pre-benchmark
zero-ID admission wall. Falsifier: zero-ID birth refusal remains or scanner/
native filtering diverges. H-MIGRATION: the worker source transition then lets
the original GravityMark Vulkan workload finish. Falsifier: clean refusal,
stalled migration, device loss, wrong/missing benchmark image/timing/GPU proof,
host Xid or guest recovery. The original 600-second workload budget and pinned
media/oracles are unchanged. Native per-slot evidence supports zero-start only
for its prepared fixture, not a general in-flight Windows cursor contract.

Verification order: GPU-free tests; exact source gates9/9 + gate10 + USER census;
rebuilt QEMU/raw client/fresh fastguest30/30; Linux desktop/gears; one fresh
Windows disposable under the global GPU lock, production profile plus only
KF3_WIN_USER_RING_RELAY=1. No build during timed guest work, personal Windows
image untouched. Batch any next known parser/source walls, time-box full-worker
iterations and compare with the retained same-GPU VFIO pass before continuing.
