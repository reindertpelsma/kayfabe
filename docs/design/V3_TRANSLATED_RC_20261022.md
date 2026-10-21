**STATUS: DESIGN-ONLY, 2026-10-22.** B1 stopped at the baseline denied-Clippy gate; no implementation.
Labels: CODE READ means public source/repository code; MEASURED means a named test artifact;
INFERRED means a prediction requiring hardware validation.

CODE READ: Translated `HostRing` births pass zero `hObjectError`. Passthrough/Windows USER
births instead bind an error-context DMA and an OS event, and `rc_scan` posts `RC_TRIGGERED`.
The continuation memo explicitly does not establish which ring produced the Blackwell Xid32.
This fixes observability; it neither diagnoses that method nor promises Windows recovery.

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

Birth allocates a 16-byte context over the unused tail of the private ring's USERD allocation
(outside the T-space GPU mappings), clears it before native birth, and supplies it as hObjectError.
Every QEMU Translated birth arms a dataless OS event on that context before publishing the slot.
Failures refuse birth and unwind; native channel free precedes event/context/backing release.
There is no new GPU map, guest handle forwarding, boot flag or timeout adjustment.

The record is PRIVATE: a hostile guest cannot manufacture host RC by changing its own notifier.
An fd wake hints that the drainer should scan; the drainer also scans authoritative records on
its existing passes, with try-locks and fixed reads per live slot. Busy slots and partial records
remain pending; event coalescing is harmless. No GP_PUT polling. A status-first snapshot prevents
consuming old info32 beside a newly published status. One record is accepted per channel life.

The slot's existing guest index maps through token_index.chid_of (including Blackwell runlist
bits); the declared guest engine, not the host CE engine/token, routes the event. On observation,
mark only this slot dead and remove its completion interest. Do not retire entries, write GP_GET,
release semaphores, or signal success. Write only the allocation-declared guest notifier's four
words, metadata first and info16/status last, then queue the guest RC event with CHANNEL scope.
Write failure keeps one pending error for retry; no event precedes successful publication. If the
guest waived a notifier, still report the channel event. Unsupported/invalid declared notifier
locations refuse birth. A per-allocation liveness token suppresses queued events after retirement,
so reuse of a host token or guest chid cannot redirect an old error to a new channel.

Hostile bounds: at most one private context/event, one bounded guest view (at most two pages), one
pending record and one event per live Translated channel. Sysmem uses the checked DMA-to-file
16-byte range; FB uses the usable-heap check including overflow/carve-out exclusion. A guest can
corrupt its own notification, request invalid destinations, churn its own channels or cause its own
GPU work to fail; none supplies a host address, host verb flags, cross-VM destination or unbounded
per-wake list. Retired pending events are pruned. No relay, worker algorithm or VA change is needed.

MEASURED baseline: `traces/translated_rc_20261018/` records baseline/EXPECTED RED counts, isolated
EXPECTED RED commits, mutation killers and clippy delta. Hardware numbers: n=0 in this lane.

CODE READ stop/options: the required continuation base has 93 denied Clippy diagnostics in
six unchanged channel/relay files (51 in `kf-qemu/src/userpump.rs`), which B1 may not repair
beyond the narrow RC hook. MEASURED: the unchanged-file hashes and diagnostics are recorded with the evidence. The no-op delivery API
and tests are a reviewable checkpoint only. Recommendation: repair that baseline in a
separately scoped lane, then resume implementation; alternatively explicitly expand B1's
scope. Do not suppress the denies. No behavior fix, mutation result or hardware success is
claimed. No public observation verb is missing; the blocking issue is the existing safety gate.
