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


## Exact cec428397 Linux result — 2026-10-12 04:33UTC

MEASURED source **cec4283976009f2662e3f6812e810959911144c4**, clean trusted
RTX4070/595.91.07 host: gates9/9, gate10 PASS, native births12/12 USER/refused0;
rebuilt revision-private QEMU SHA256
91928b2206efe6c56e10af51e88409066339a742aabfef51d73f06c53776059c,
rebuilt raw client/fresh fast guest, original fast suite30/30 PASS, zero crash
or NOTRUN, runner EXIT0. The stated falsifier was not observed.

Bounded kernel journal contains5 Xid31 at the addresses explicitly named in
negative tests: defer-liveness0x6000010000/0x6200010000/0x6400010000 and
missing-page-fault0xa11000000. No Xid32. This is not a zero-Xid claim.
Complete text lane logs and their hashes are retained in linux-result/.
Postconditions: noQEMU, DMA-FQ, lockfree, stopfile absent, >100GiBfree; personal
image remains0444/28240642048bytes. No Windows guest/image was opened.
The result covers cec428397, not subsequent decoder changes or an unwired relay.
