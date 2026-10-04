# Windows graphics-preemption pool experiment

**STATUS: RESEARCH, 2026-10-05.** Branch `codex/windows-pool-2026-10-05`, based on
`v3-windows` at `c50fad9a`. This is a boot diagnostic, not a completed pool implementation.

The previous Windows 580.88 run on Kayfabe refused `GR_GFX_POOL_QUERY_SIZE` immediately
before driver teardown. Ordering alone does not establish causality. The experiment supplies
bounded virtual dimensions and compares startup with the same binary's default refusal.

`KF3_GFX_POOL_PROBE=1` enables the experiment on the host. It is off by default and restricted
to the audited Windows 580.88 / Linux 580.65.06 wire pair. The public `ctrl2080gr.h` query
is 40 bytes: maximum slots, slot stride, control size/alignment, pool size/alignment.
The experiment uses one 4-KiB page per virtual slot and one page for the control structure,
with at most 4096 slots. **These are invented virtual dimensions, not NVIDIA's physical
layout or a source-derived sizing formula.** The 64 entries in ADD/REMOVE are a batch limit,
not a proven maximum pool size.

Only the size query is implemented. No guest address is dereferenced, no pool memory is
modified, no host command is sent, and no GPU work is completed by this policy. Malformed
sizes/counts and serialized parameters are refused. Initialization, add/remove, and context
binding remain separate work; a successful query is not evidence that any of them works.

The possible implementation route is guest-kernel bookkeeping plus host-owned non-pooled
GfxP storage. Native RTX 4070 / open Linux 595.91.07 accepts an unprivileged graphics mode-1
request on a real OpenGL channel, whose framebuffer digest is unchanged. Mode 2 is rejected
with `NV_ERR_INVALID_ARGUMENT`. This supports investigating that route, but does not prove
preemption under contention, a Linux pooled-query path, Windows compatibility, or other GPUs.
The native test source is research-branch commit `0af50483`; text evidence is under
`tools/windows-gsp-trace/evidence/2026-10-05-rtx4070-linux-preemption/` on that branch.

The PC runs host NVIDIA 595.91.07. It must be measured from its exact public OGKM tag before
Kayfabe can realize; never bypass the ABI guard. Windows runs against `kf3-gpu` while Linux
keeps the physical GPU. The native Windows fixture is copied before use. VFIO success is
only a native reference, not a Kayfabe result.
