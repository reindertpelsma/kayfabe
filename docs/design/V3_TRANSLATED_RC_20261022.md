**STATUS: LIVE, 2026-10-22 — implementation candidate; hardware validation pending.**
CODE READ update after owner-authorized resume: merge `82effe99f` brings in master
`e64b8e5f6` and PR #63's CI repair. MEASURED: baseline Clippy now succeeds, with no denied
errors; full channel/QEMU baseline has 1070 passes, 14 intentional RC failures, 4 existing
ignored tests. The old stop below is historical. Validation evidence is updated separately.
Labels: CODE READ means public source/repository code; MEASURED means a named test artifact;
INFERRED means a prediction requiring hardware validation.

CODE READ before B1: Translated `HostRing` births passed zero `hObjectError`. Passthrough/Windows USER
births instead bind an error-context DMA and an OS event, and `rc_scan` posts `RC_TRIGGERED`.
The continuation memo explicitly does not establish which ring produced the Blackwell Xid32.
CODE READ: this change supplies an observation/delivery path. It neither diagnoses that method
nor establishes Windows recovery; those require native and guest hardware evidence.

CODE READ, public NVIDIA ogkm 580.159.04, commit
`b81d58ee0224d1d290bef1c080592b619e184042` (MIT):

- `src/nvidia/src/kernel/rmapi/resource_list.h:2166-2200` marks `NV01_CONTEXT_DMA` and
  `NV01_EVENT_OS_EVENT` `RS_FLAGS_ALLOC_NON_PRIVILEGED`, `RS_ACCESS_NONE`.
- `src/nvidia/src/kernel/rmapi/event.c:45-208` accepts user OS events; kernel callbacks require
  kernel privilege. Bind source=our error context, index=0, WITHOUT_EVENT_DATA, nonstall=false,
  to the existing session RC fd. No new privileged callback or subdevice RC_ERROR subscription.
- `src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:163-188,352-449` writes the notifier
  and notifies its context-DMA listener. `gpu/gsp/kernel_gsp.c:541-679` handles the host GSP RC
  message and sends these CPU OS notifications; with CC off it does not rewrite the record.
- `gpu/mem_mgr/method_notification.c:138-155,181-185` publishes timestamp/info32 before status;
  the context-DMA variant combines info16/status into its last 32-bit write. `NvNotification`
  is 16 bytes; `kf_abi::notifier` owns its layout. `kernel-open/nvidia-uvm/uvm_channel.c:2058-2082`
  returns RC_ERROR when status is nonzero. This is an error, never completed GPU work.
- `src/common/sdk/nvidia/inc/ctrl/ctrl906f.h:94-133` defines RESET_CHANNEL (engine reset as well
  as channel reset), reasons 0..2 and bIsRcPending. `kernel_channel.c:3036-3083` takes the CPU-RM
  pending fact then RPCs physical management; generated `g_kernel_channel_nvoc.c:265-280`
  exports it NON_PRIVILEGED (0x10008). This lane does not add/reset/revive a dead host ring:
  guest reset intent must not erase unfinished work. Existing refusal remains an explicit gap.

INFERRED design, falsifier stated before implementation: an unprivileged host error-context
subscription plus its durable record can observe every RC on our ring. Falsifier: public source
requires privilege or excludes this source/class, or native testing shows a real RC with no record.
The source check supports it; native/GPU validation remains mandatory and UNRUN in B1.

CODE READ implementation: birth allocates a 16-byte context over the unused tail of the private ring's USERD allocation
(outside the T-space GPU mappings), clears it before native birth, and supplies it as hObjectError.
Every QEMU Translated birth arms a dataless OS event on that context before publishing the slot.
Failures refuse birth and unwind; native channel free precedes event/context/backing release.
There is no new GPU map, guest handle forwarding, boot flag or timeout adjustment.

CODE READ: the record is PRIVATE: a hostile guest cannot manufacture host RC by changing its own notifier.
An fd wake hints that the drainer should scan; the drainer also scans authoritative records on
its existing passes, with try-locks and fixed reads per live slot. Busy slots and partial records
remain pending; event coalescing is harmless. No GP_PUT polling. A status-first snapshot with an acquire fence and a final status check uses at most five u32
loads; it prevents consuming old info32 beside a newly published status. One record is accepted per channel life.

CODE READ: the slot's existing guest index maps through token_index.chid_of (including Blackwell runlist
bits); the declared guest engine, not the host CE engine/token, routes the event. On observation,
mark only this slot dead and remove its completion interest. Do not retire entries, write GP_GET,
release semaphores, or signal success. Write only the allocation-declared guest notifier's four
words, metadata first and info16/status last, then queue the guest RC event with CHANNEL scope.
FB uses the aligned atomic u32 port for its last word. RAM's checked authority promises byte
copies, so it writes info16, fences, then writes the status halfword at the public ABI offset;
it never assumes the final four-byte copy is atomic. Both paths fence before event publication.
Write failure keeps one pending error for retry; no event precedes successful publication. If the
guest waived a notifier, still report the channel event. Unsupported/invalid declared notifier
locations refuse birth. A per-allocation posting permit (a read lock held through the actual queue write) serializes
retirement with posting. Retirement invalidates it before allowing reuse. Contention retains
the event for retry; retired events are pruned. An atomic live-bit check alone would race the
queue write, so it is insufficient. No vCPU takes this new lock.

CODE READ hostile bounds: at most one private context/event, one bounded guest view (at most two pages), one
pending record and one event per live Translated channel. Sysmem checks the DMA-to-file 16-byte range, then pins the generation using master's existing
`MemoryListRam` authority. Every four-byte write uses its nonblocking `try_write` and holds its
topology guard; a revoked allocation never refreshes itself to replacement RAM. No RawRegion is
cached. FB uses the usable-heap check including overflow/carve-out exclusion. A guest can
corrupt its own notification, request invalid destinations, churn its own channels or cause its own
GPU work to fail; none supplies a host address, host verb flags, cross-VM destination or unbounded
per-wake list. Retired pending events are pruned. The only worker-path change is the narrow RC check before a Translated pump. Relay and VA code
are unchanged. No timeout, polling interval, boot flag or completion rule changes.

CODE READ (commit 007ba1eb0:78): quoted historical statement, evidence not revalidated by BW9. MEASURED baseline: `traces/translated_rc_20261018/` records baseline/EXPECTED RED counts, isolated
EXPECTED RED commits, mutation killers and clippy delta. Hardware numbers: n=0 in this lane.

CODE READ (commit 007ba1eb0:81): quoted historical statement, evidence not revalidated by BW9. **SUPERSEDED by the master-merge update above (owner authorized resume).**
CODE READ historical stop/options: the original continuation base had 93 denied Clippy diagnostics in
six unchanged channel/relay files (51 in `kf-qemu/src/userpump.rs`), which B1 may not repair
beyond the narrow RC hook. MEASURED: the unchanged-file hashes and diagnostics are recorded with the evidence. The no-op delivery API
and tests are a reviewable checkpoint only. Recommendation: repair that baseline in a
separately scoped lane, then resume implementation; alternatively explicitly expand B1's
scope. Do not suppress the denies. The historical stop claimed no behavior fix, mutation result or hardware success. No public
observation verb was missing; the original blocker was the old baseline safety gate.

INFERRED RAM-lifetime hypothesis, falsifier before its validation: a pending error never writes
replacement backing after guest-RAM topology revocation. Falsifier: a write/event is accepted
on the replacement generation, or an old target refreshes its generation. The implementation
uses the master's checked authority directly; hook tests cover revocation before and during
publication, a busy guard and successful retry. GPU/DMA and Windows acceptance remain UNRUN.

CODE READ (commit 007ba1eb0:96): quoted historical statement, evidence not revalidated by BW9. MEASURED final GPU-free validation: all 22 kf-* crates, 3999 passed / 0 failed / 22 existing
ignored tests; 15 deliberate mutant variants (16 runs), every variant >=2 killers, source
restored. Clippy 264 existing warning sites ->264, added 0, errors 0; formatting passes.
`traces/translated_rc_20261018/` carries per-crate counts, exact commands, every mutation killer
and tests-first evidence. No native event/private-record, guest IRQ or Windows recovery PASS
is asserted. RESET_CHANNEL remains outside this implementation.
