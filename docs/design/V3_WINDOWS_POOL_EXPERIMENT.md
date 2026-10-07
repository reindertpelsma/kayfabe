# Windows graphics-preemption pool experiment

**STATUS: RESEARCH, 2026-10-05.** Branch `codex/windows-pool-2026-10-05`, based on
`v3-windows` at `c50fad9a`. This is a boot diagnostic, not a completed pool implementation.

The previous Windows 580.88 run on Kayfabe refused `GR_GFX_POOL_QUERY_SIZE` immediately
before driver teardown. Ordering alone does not establish causality. The experiment supplies
bounded virtual dimensions and compares startup with the same binary's default refusal.

`KF3_GFX_POOL_PROBE=1` enables the experiment on the host. It is off by default.
**Correction, 2026-10-05:** the original 580.65.06-only gate was an experiment limit,
not a v3-compatible driver policy. The query now requires the exact driver tag's
compiled layout to match all six fields and the 40-byte size. All 30 measured tags
(535.309.01 through 615.71.09) have that layout; unmeasured tags fail closed, and
the existing unsupported encrypted 615 queue is still refused separately. Both
24-byte and 40-byte control envelopes are tested. This establishes ABI coverage,
not runtime compatibility across GPUs or Windows releases. Hardware evidence is
still only Windows 580.88 / guest wire 580.65.06 on AD104, host 595.91.07.
The public `ctrl2080gr.h` query has maximum slots, slot stride, control
size/alignment, and pool size/alignment.
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

## Timer allocation follow-up

The same-binary comparison at `167fe2de` passes the query only with the opt-in
enabled. Windows then reaches more context/display initialization, but still
reports Code 43. The next allocation refusals are unknown class `0xb297` and
`NV01_TIMER` (`0x4`), followed immediately by teardown. Evidence:
`traces/windows_pool_20261005/{baseline-a,probe-b}/`.

The next experiment admits **only timer object allocation bookkeeping** into the
existing graph, classified as `Other`, with normal namespace and handle lifetime
handling. No host object, channel, mapping, alarm, or completion is created by
this allocation. The graph keeps its existing order-tolerant edge semantics;
this is not a complete implementation of TimerApi's parent/single-instance
rules or its controls. Unsupported timer controls still fail. A later timer
operation must validate its target and implement its actual behavior separately.

**Audit qualification, 2026-10-05:** the repeatable source spot-check below uses
regex/string matching, which departs from v3 §0.3's requirement for a real C parser.
It is research evidence, not a compiler-verified semantic proof or a product-code
generator. Manual source inspection explains the behavior; replacing this research
helper with an AST audit would strengthen its repeatability. Product class IDs and
query layouts use the compiled driver matrix, not this helper's output.

The source spot-check reports the following across all 30 measured OGKM tags:
`tmrapiConstruct_IMPL` only returns `NV_OK` and the destructor is empty.
`resource_list.h` permits unprivileged allocation, has no parameters (`RS_NONE`),
requires a Subdevice parent, and allows one instance per parent. The explicit
`ALLOC_RPC_TO_PHYS_RM` flag is present from the measured 555.42.02 tag onward;
it is absent in the measured 535/545/550 tags. Thus this is not Windows-only.
`scripts/bench/windows/audit_timer_alloc.py <local-ogkm-clone>` reproduces the
checks; the table with peeled commit IDs is
`traces/windows_pool_20261005/timer-source-audit.tsv`.

The existing allocation transport rejects serialized parameters, out-of-message
declared lengths and the reserved client namespace. Parameter bytes themselves
are unused, as in the other `NoDeclaredFacts` classes; they cannot become host
pointers. Admitting this class changes no control allowlist. Unknown `0xb297`
remains refused. This branch has not met the integration/merge validation bar.

At `60d36db5`, a fresh Windows run (`probe-c`) accepts timer allocation but still
tears down immediately and reports Code 43; no GPU channel is created. Timer
allocation alone does not fix startup. Source shows that a subsequent CPU mapping
can fail locally because CHIP_INFO advertises TIMER as unavailable. That is the
next hypothesis, not an established explanation of this Windows failure. Publishing
a timer base requires genuine read-only host backing first; returning a zero page
or fabricating a timer is not an acceptable substitute.

**Existing-base security blocker:** this experiment inherits S1-21: identity/sysmem
windows are mapped into mirrored passthrough spaces (`kf-qemu/src/mem.rs`). The new
query/allocation delta adds no host forwarding, pointer access, or GPU completion,
but the whole branch cannot be described as secure. Integration requires the private
Translated-space fix and its validation, plus the exact-revision hardware merge bar.
