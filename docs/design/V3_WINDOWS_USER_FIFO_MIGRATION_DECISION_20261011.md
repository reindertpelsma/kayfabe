# Windows user-channel FIFO/USERD migration: next design decision

**STATUS: DESIGN-ONLY, 2026-10-11 23:06UTC.** No migration implementation or host
patch. The narrow refusal experiment has reached its failure condition; final
application/deadline recording is pending. This note is an implementation
proposal, not a new owner ruling.

## What the measurements establish

GravityMark1.89 Vulkan passes183s on the same RTX4070/Windows580.88 under VFIO.
The native observer records two successful deferred FIFO-update registrations;
actual GSP execution is unmeasured. No later plain RmControl is required for a
GSP-internal deferred call, so its absence is not evidence that native skipped it.
Original native channel alloc parameters and USERD memory-object aperture/size
are absent; do not reconstruct them from handles or infer them from another run.

Kayfabe1361's faulting channel4e is Passthrough/user-work, already scheduled
and4096+ submissions forwarded before registration. Probe1510 captures a
matching submitted0x200/API4000002c and bundle: old32768 entries, requested65536,
newFIFO0x42be000/USERDoffset0x600. Earlier object binding and actual GPU execution
are unmeasured. This makes a Translated-only software fix inapplicable to the
observed target; it does not independently prove the initiating cause.

Exactb081f705a run1540 deliberately returns NOT_SUPPORTED for that inner
registration on live Passthrough user-work parents. First rejected API40000030
on clientc1d0007d/objectff1fe010; counter1. HostXid32/channel4e at23:02:37UTC and
mid-run screenshot shows Vulkan device-lost/cannot-submit. Thus a clean fallback
under this status is falsified for this run. Actual later refused-handle trigger
is UNMEASURED; Xid32 alone does not identify its method. No status-handling error
has been measured to justify changing the error code as another variable.

Sources: [registration-refusal evidence](../../traces/windows_matrix_20261011/gravity-vulkan/registration-refusal/README.md),
[matched submission capture](../../traces/windows_matrix_20261011/gravity-vulkan/late-fifo-89e8ddf8/README.md),
[original audit](../../traces/windows_matrix_20261011/gravity-vulkan/h1-channel-audit/README.md).
Manual1380 has no matching observed hostXid/FIFO signature; its Minecraft freeze
is a separate unresolved observation.

## A. Restricted mediated user channels, selected at birth

A possible stock-host design owns a stable private host GPFIFO/USERD and reads
and mediates the guest submission stream. The host channel retains the guest's
restricted mirrored VA space. It never receives the privileged kernel T-space
or whole-GPGA mapping; physical operands are denied/refused, with per-engine
hardware containment validated as required by OWNER_RULINGS §§U.3,V,AB.

The deferred trigger is a method inside a pushbuffer, not a GPFIFO entry type.
Kayfabe must inspect packet framing and object binding, split/replace the relevant
method with host-authored ordering, and preserve all GPU work and real completion.
It must never edit guest-owned buffers in place, strip arbitrary matching words,
or execute GPU commands on the CPU. Unknown methods remain subject to the
existing unprivileged-channel containment and bounded-decoding rules (§V).

A stream-position action can re-point only the guest-side ring/USERD source
while the private host ring stays stable. Before building it, define cursor
continuity, old-ring retirement, new-ring publication, same-client USERD ownership,
complete-span validation through the guest page tables, mapping/backing lifetime,
free/reuse races and ordering against the VA-manager gate. Existing metadata
captures do not establish native cursor-transfer semantics. A doorbell is not a
safe execution boundary (§U.2); a registered handle alone is not completion proof.

Choose the mode before the channel can be scheduled or receive a usable token.
No conversion at registration: the measured sequence is already too late. A
policy mediating all declared Windows user-work channels at birth is an option
requiring evaluation; no earlier per-channel migration indicator is established.
Do not select by application name, captured allocation size or GPU family.

The user doorbell page is shared. Mediation must use the existing permitted
BAR0-write path and worker routing; no new BAR1/BAR2 traps. Token-based routing
must tolerate another guest process ringing the same token. Channels selected
for mediation lose the direct inline-ring advantage; relayed1540/1510 already
use workers, so claiming a new VM exit per observed relay submit would be wrong.

**Estimated cost, not measured:** for E GPFIFO entries and P pushbuffer words,
CPU inspection reads at least8E+4P bytes. Copying an immutable private command
stream additionally writes approximately4P bytes plus private descriptors and
ordering methods. It does not copy the GBs of upload payload merely because
commands reference them. Staging, host-ring and lifetime metadata must have
explicit per-channel/per-VM bounds and backpressure. Pushbuffers in VRAM require
a safe read/transfer path absent from the current kernel Translated parser;
its extra latency is unmeasured. No CPU%, FPS or throughput claim is justified.
Measure queue throughput, p50/p99 submit-to-completion and CPU time on the same
workload/reference before selecting defaults. No synchronous GPU read on a vCPU.

## B. Optional host-patch tier

[H8 in the host-patch list](V3_HOST_PATCH_LIST.md#48-h8--validated-own-channel-fifouserd-migration)
is a candidate: a narrowly authorized operation for a client's own channel and
its own host USERD memory, preserving the native stream-position behavior.
Do not globally remove RMCTRL_FLAGS_PRIVILEGED or pass guest handles/bytes to RM.
The VMM must author host parameters, own the memory/mappings and enforce complete
spans, bounds, provenance, lifetime and channel ownership.

The patch must solve or explicitly leave unsolved the private-USERD relay
transition and exact execution/completion observation; permission alone does not
prove compatibility. It changes open RM core, so it is a separate rebuild/probe/
rollback tier. H5's USERD DMA-address-size fix is related infrastructure, not a
FIFO-migration implementation. No stock-host support claim depends on H8.

**Estimated cost, not measured:** native processing should avoid full CPU
pushbuffer mediation for this operation; setup/validation and each deferred
migration still have their normal work. Neither performance gain nor successful
migration has been measured. Validate the stock/optional tier A/B on exact source.

## C. Honest stock refusal / partial scope

A clean registration refusal can state that migration is unsupported. Run1540
shows that this does not make GravityMark's fallback safe; do not ship it as a
GravityMark fix or hardwire it on the strength of successful boot. Preserve the
negative-control diagnostic default OFF. It has no established compatibility
benefit for other workloads. Refusing the whole5080 object is broader and was
already measured to break Windows device creation.

## Decision waiting for the owner

Choose whether to prioritize a restricted mediated user-channel design (A), an
optional host-patch proof (B), or leave migration explicitly unsupported while
other app defects proceed (C). Recommendation: prioritize A's bounded model and
public/measurement cursor-semantics work for the stock-host objective; keep B a
separate optional proposal. This note proposes the options and implements neither.
Before implementing either, close the exact trigger/cursor/lifetime measurement
gaps and review the hostile-guest and nonblocking paths. A status-code sweep,
late channel conversion or relay-only pointer edit is not a substitute.
