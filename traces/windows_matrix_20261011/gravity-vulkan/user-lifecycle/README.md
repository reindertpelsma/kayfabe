**STATUS: RESEARCH, 2026-10-12 04:28UTC.**

Production prerequisite for the Windows immutable user-ring relay; no relay is
wired by this change. Based on pushed1373317ce. Exact source is the commit
containing this file; the separate Linux runner records its full revision.

PtChan has a distinct allocation identity. A queued/parked EngineObj retains
that identity AND its pending engine-object identity, validates both before
steer/native allocation and again in the attachment transaction. Guest Free
invalidates pending child identity at statement time. Native handles/tokens may
reuse without restoring either identity. The object record retains the exact
validated allocated class for the future parser binding lookup.

The pending map is bounded64 per channel, a trusted bookkeeping policy unrelated
to die/driver facts. Weak pending rows cannot own failed/dropped acts; admission
prunes dead weak identities. Deferred orphan cleanup withdraws only the exact
channel/object lifetime once, so an old rejection cannot free a replacement.
Every native allocation/free remains outside pt/pt_objs metadata locks.

MEASURED: full GPU-free kf-chan/kf-qemu/kf-rm tests pass; qemu lib173/173. Tests
cover identical numeric channel/native reuse, same-live-channel object Free/reuse,
old orphan versus replacement, once-only withdrawal, cross-client cancellation,
pending budget/drop and exact class retention. Independent source review found
and corrected the child-lifetime gap in the first prototype, then no remaining
source blocker. Filtered Cargo summary retained. No hardware claim at checkpoint.

Linux hypothesis, written BEFORE the run: exact source preserves Linux USER birth,
VM isolation and existing fast applications; the Windows private-VA primitives
remain unconnected and Linux Passthrough gains no host-owned mapping.
Falsifier: any of gates1-9, gate10, privilege census or the original30 fast arms
fails, crashes, times out or does not run. Serial runner records START/EXIT,
clean source revision, revision-specific binary, shared GPU lock and postconditions.
NOT_RUN at checkpoint. Gravity kayfabe remains FAIL/TIMEOUT, same-GPU VFIO PASS.
