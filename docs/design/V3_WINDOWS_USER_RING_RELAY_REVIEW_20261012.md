# Windows user-ring relay: P0 review and restricted prototype

**STATUS: RESEARCH, 2026-10-12.** Review of integration328fe4bf8's
V3_WINDOWS_USER_RING_RELAY.md. Owner approved the restricted revision in
OWNER_RULINGS §AI. No VM implementation or GravityMark pass yet.

## Decision and required corrections

A stable host GPFIFO and private USERD are a useful direction: a guest ring
relocation then need not relocate the native channel's ring. Develop this as
restricted mediated Windows user-work selected at birth. Linux Passthrough
continues to have no host-owned mappings. Use workers and immutable command
inspection from birth. Do not introduce the proposed inline path.

Two original safety arguments do not hold:

1. Refusing guest page-table leaves in a reserved range prevents collisions. It
   does not stop a shader/copy/semaphore from directly naming an already mapped
   address in the same GPU VAS. GPU-read-only ring/staging mappings can protect
   integrity; they do not make bytes unreadable. Never include secrets or an
   alias exposing writable host objects, and retain the hardware physical-mode
   denial. Native write protection is a measurement gate, not a model result.
2. Holding a registration reply does not make a late switch to parsing safe.
   A hostile guest can guess/publish a handle before the reply; an already
   forwarded mutable pushbuffer can contain it when the GPU fetches it after
   registration. Even a cooperative guest can have unfetched published entries.
   Parse/copy from birth; registration can populate metadata but cannot change
   the security mode. REMOVE_API is not a stream drain either, and nonexplicit
   deferred entries can be removed implicitly on execution.

Native DMA_INVALIDATE_TLB is not established as a harmless no-op: public source
performs a VAS invalidate and updates deferred waiting state. Its acceptance
would not prove migration completion or the VA-manager ordering required by §U.
No substitute registration or registration-time source switch is implemented.

## P0 findings: source facts versus measurement gaps

- **Source-confirmed GPFIFO VA:** OGKM595.84
  src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:1129 and
  arch/hopper/kernel_fifo_gh100.c:598 explicitly call gpFifoOffset a virtual
  address; their functions write the RAMFC GP base. kernel_channel.c:2581 merely
  forwards this parameter. No captured register offset enters the implementation.
- **Source-confirmed protection API:** kf-host/src/channel.rs MapPerm encodes
  NVOS46_FLAGS_ACCESS_READ_ONLY. kf-chan/src/host.rs TSPACE_LAYOUT already maps its
  command regions read-only, but that is a different space and not a user-channel
  containment proof. No unprivileged PRIVILEGE flag is available in MapPerm.
- **Source-confirmed mirror allocation:** alloc_vaspace_with supplies va_base
  with otherwise default parameters (va_size=0). It does not size the host VAS
  to the guest's declaration. A free, collision-proof, usable private region
  below the command-fetch address limit is still unmeasured. HELD-BY-HOST is a
  collision report, not a reserved-region allocator or access-control mechanism.
- **Measured1510:** post-forward sample GP[0x4b58,0x4b64), twelve entries, the
  last containing candidate method0x200/API4000002c matching the FIFO-update
  bundle. This is a submitted interval, not occupancy: no simultaneous hardware
  GP_GET is captured at registration. The eventual release cursor0x4b64 does not
  supply the missing earlier value. Original ring32768/request65536 is measured;
  newFIFO0x42be000 and USERDoffset0x600 are diagnostic facts, never product constants.
- **Native cursor transfer UNMEASURED:** old/new USERD contents, initial new
  GP_GET/GP_PUT, original channel alloc and backing aperture/size are absent from
  the reference. Neither reset-to-zero nor preserving indices can be assumed.
- **Native deferred execution UNMEASURED:** two successful registrations and no
  plain20801116 RPC do not distinguish execution inside GSP. The proposed P0(d)
  plain-RPC criterion is invalid. Need an interface-level stream/cursor witness.
- **Staging retirement UNMEASURED:** hardware GP_GET progress must not be called
  GPU-work completion. The current Translated runner uses a private writable
  RELEASE_WFI fence, which §AI does not authorize in a user mirror. Do not reuse
  that runner/fence mapping wholesale. Establish consumption/lifetime semantics
  or a separate permitted completion mechanism before binding the prototype.

Public reference: NVIDIA/open-gpu-kernel-modules tag595.84. Existing evidence:
traces/windows_matrix_20261011/gravity-vulkan/{h1-channel-audit,late-fifo-89e8ddf8,vfio-reference}.
No private diagnosis supplies these facts or implementation interfaces.

## First native probe and falsifiers, written before its run

Source: crates/kf-harness/src/bin/kf-user-ring-probe.rs. A standalone authored
USER channel, one private command object, a separate unmapped USERD object and
a writable probe-output fixture. This fixture represents guest-owned output;
it is not a proposal for a writable host fence in a VM. No guest or personal
image is booted. All cases run unprivileged, serial under the shared GPU lock.

H-RO-FETCH: a USER channel can fetch its GPFIFO and method words through a GPU
read-only mapping. Falsifier: read-write control completes but otherwise identical
read-only case fails allocation/fetch/tail, or incurs a new attributed Xid. Both
cases must produce the real GPU tail and unchanged sentinel, with no Xid.

H-RO-WRITE: a virtual GPU semaphore store cannot modify the read-only command
mapping. Falsifier: the sentinel changes to the attempted value. A clean denial
requires attributed hardware fault plus unchanged bytes; absence of a tail or
expiry of the collection deadline alone is inconclusive. No CPU completion is
authored. This is a limited FIFO store probe, not every-engine security proof.

The three-second bench deadline only ends collection; channel teardown must
succeed before any explicit memory revoke. Journal collection includes a delayed
delivery observation window. No timer releases product state. Probe setup errors
are failures, never inferred protection. Save exact revision, binary SHA, UID,
PID, start/exit and journal evidence; no GravityMark or full Linux claim follows.

## Next implementation gates

GPU-free bounds/wrap/backpressure and interleaving models first. Private-ring
admission must require validated declared Windows user-work; negative tests must
pin Linux/no-identity/kernel/UVM/non-user routes to no host command mappings.
Mapping reservations and references must share the VA-manager ownership ledger;
free/reuse cannot silently replace a pinned source or command mapping.

Then close the native migration cursor and staging-consumption gaps. Only after
those gates add a default-OFF runtime experiment and independent hostile-guest/
nonblocking review, run exact-revision Linux9/9+gate10/fast30, then a single-variable
Windows GravityMark run against the same-GPU VFIO183s/image reference. None of
these later gates is claimed complete by the native permission probe.

## Second native discriminator: registration privilege, before the run

The previous same-configuration deferred-update probe ran uid65534: NOP clean,
update/update-hold Xid32 even with resources retained. Public deferred_api.c
stores the registering caller's privilege. A USER GPU-channel stamp is not the
registering CPU caller's privilege. To obtain a native migration reference, first
compare **the same existing b081f705a probe/binary and update-hold workload** under
uid65534 and root, retaining its asserted USER GPU channel. The only controlled
variable is registering-process UID/privilege. This is privileged reference
measurement (§AH's measurement tier), never a stock-product host path.

H-PRIV-REFERENCE: a privileged registrant supplies a fault-free reference for the
same-configuration deferred FIFO update. Falsifier: root also faults, refuses, or
fails the real GPU tail/resource retention/teardown. If root succeeds and uid65534
faults, that supports further root-only reference measurement; it does not yet
prove actual inner execution or cursor transfer, nor identify the exact rejected
check in the unprivileged case. A same-configuration update is not a migration.
If root also fails, stop this reference route and compare the public control
contract; do not change multiple allocation fields to clear walls indefinitely.

**MEASURED 00:16UTC:** H-PRIV-REFERENCE is falsified as a root-UID reference
route. Exactb081f705a native binary9acb5976 runs update-hold under uid65534 and
uid0; both actualGPUtail/2s ownership hold/cleanup succeed, both cause one
PID-attributedXid32 inside the hold. Both channels remain assertedUSER (root
birth clears CAP_SYS_ADMIN for that alloc). The saved GSP/RM registration
privilege level and inner failure status are unmeasured: rootUID is not proof
that the deferred record carried ADMIN. No further configuration sweep or
privileged product call follows. Evidence: user-ring-p0/privilege-reference.

## Prototype and model results, 00:20UTC

`kf-chan::user_ring::Ledger` is an unconnected accounting prototype. Host slots
wrap independently of opaque guest-generation/cursor receipts; bounded metadata,
one-empty-slot backpressure, named invalid fetch/future-completion refusals and
duplicate completion handling are modeled. Only simulated authenticated engine
completion returns retirement receipts. GP_GET alone never grants staging reuse.
No production completion adapter, native migration reset/preserve rule or mapping
lifetime implementation is supplied. This conservative model is not yet proof
that the migrated workload can progress.

Failing-first compile logs precede implementation. Full kf-chan tests pass,
including six new integration models and exhaustive small-ring schedules.
`kf-rm::chanlink::WindowsUserRingPermit` is an unforgeable Rust construction
witness over existing declared user-work birth facts, not a GPU mapping or guest
attestation. Two failing-first/full-suite-green scope tests pin every existing
fact-grid row with Linux/unknown identity to no permit; kernel/UVM/internal/
incomplete routes are also refused. It is not called by any runtime birth path.

The separate ordering model finds seven valid late-parse counterexamples and
zero for immutable inspection from birth over the same schedules (four tests).
This proves the small scheduling argument, not a full parser/GPU contract.
Native permission positives both PASS; the negative has attributedXid31
FAULT_RO_VIOLATION and unchanged bytes. It directly observes GP_GET1/tail0.
Actual mapping/free/reuse races, native migration cursors, command parser and
completion adapter remain open. Linux exact70aa hardware regression has since
passed below; Windows integration is not enabled by these prototype additions.

## Exact prototype Linux verification, 2026-10-12 00:35UTC

MEASURED exact70aa0d666: gates9/9+gate10 PASS, USER births12/12, rebuilt
client/fastguest, fast30/30 PASS with zeroFAIL/CRASH/NOTRUN. Initial fast attempt
was NOT_RUN due to runner directory/path errors; corrected immutable runner used
the same exact binary. Both attempts are retained in user-ring-p0/linux-result.
This is regression evidence for the unconnected prototype, not a Windows
migration, complete ownership model or app pass. Personal Windows image unused.
