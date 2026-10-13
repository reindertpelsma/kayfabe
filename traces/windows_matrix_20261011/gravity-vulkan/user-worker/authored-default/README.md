# Private USER channel identity for a guest-default declaration

**STATUS: RESEARCH, 2026-10-13.** Written before the Windows run. No Vulkan
pass claimed. Ordinary Linux Passthrough and opaque shadow behavior unchanged.

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
