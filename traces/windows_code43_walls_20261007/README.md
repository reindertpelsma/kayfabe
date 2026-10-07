# Windows Code43: one implementation gap per boot

**STATUS: RESEARCH, 2026-10-07.** Owner-directed continuation after
[exact request decoding](../windows_exact_decode_20261007/README.md). No Windows
success or master-promotion claim. Borrowed RTX4070; host595.91.07 remains attached
to its ordinary display driver. Only owned fresh-overlay Kayfabe VMs are used.

> **Correction, 2026-10-07 (after run24; [analysis](#abort-point-analysis-against-the-vfio-reference)).**
> The run sections below read "24 saved assertions unchanged" as weak evidence that
> a wall might not matter. That inference is wrong. The kernel-RM assert journal
> is a fixed 4 KiB buffer, and a 168-byte `RmRC2SwRmAssert3_RECORD` gives exactly
> 24 slots (OGKM 580.65.06 `journal.c:79,187,953`, `rmcd.h:236-245`, sizes checked
> with a compiler). Once it is full, later asserts are dropped. So the dump keeps
> only the first 24 asserts of the boot and *cannot* show a late wall. The RPC
> traces of runs 13-24 show that each wall was the abort point. In every run,
> the driver's teardown (20 or more consecutive `Free` RPCs) begins with the RPC
> right after the first refusal that StartDevice does not tolerate. The livedump
> is `VIDEO_MINIPORT_FAILED_LIVEDUMP` (0x1B0), Arg1=2 "Start device failed",
> NTSTATUS 0xC000009A. It is identical whether the abort RPC returned 0x40 (runs
> 13-15) or 0x56 (runs 16-24), so it is a generic StartDevice mapping, not the
> RM status. Also, OGKM `rcdbRmAssertStatus` stores the NV_STATUS as the record
> level (`journal.c:2421-2461`). The two level-86 (0x56) saved records therefore
> match kayfabe refusals. Earlier tool notes said that "level is not NV_STATUS";
> this holds for level-1 records only.

## First repair: Device-shared default VA resolution

The failing COPY2 channel has hVASpace=0 under client c1d00012. Its Device
explicitly shares client c1d00002's Device default. The old resolver searches only
the channel client's namespace and returns INVALID_STATE before channel creation.

The repair retains Device allocation facts in the graph, records a bounded
same-GPU Device-share relationship in ChannelPolicy, and resolves the canonical
VA's client and handle. Explicit channel, context-share and TSG declarations
retain their existing precedence. The existing mirror and channel birth checks
still decide whether actual GPU resources are ready; this does not bypass them.

Source: OGKM580.65.06 commit307159f2623d3bf45feb9177bd2da52ffbc5ddf9,
`src/nvidia/src/kernel/gpu/device_share.c:91` (share by client and GPU instance),
`src/nvidia/src/kernel/mem_mgr/vaspace.c:183` (implicit default permitted except
MULTIPLE_VASPACES). Mode constants are generated from OGKM580.159.04 headers by
`crates/kf-abi/gen/`; no captured per-die or host-privileged facts are added.

Missing/ambiguous targets, cross-GPU targets, unknown/private modes and cycles
refuse resolution. Traversal is capped at32 and declarations at the existing
MAX_LIVE_HANDLES budget. Target Device/client destruction revokes dependent
resolution; numeric handle reuse cannot revive it. Freeing the transient VA
handle retains the Device default, as the existing lifetime model requires.
This conservative support does not implement cross-Device VA reference retention
after the target Device is destroyed; existing live GPU channels retain their
ordinary resource lifetime and new births fail closed.

Regression tests exercise distinct Device handle values, canonical namespace,
local-default suppression for an explicit share, transient-VA free, target free,
handle reuse, missing targets, cross-GPU targets, explicit-VA-only mode and cycles.
The framebuffer MemoryList rejection and latency-buffer query are independent
remaining gaps; this iteration changes neither.

Local validation before the first hardware boot: all611 kf-rm tests pass,
including12 channel-link tests. Targeted kf-rm/kf-qemu Clippy reports zero new
debt (207 existing); formatting, diff and documentation claims checks pass.

## Run16: VA wall removed, Code43 persists

Product/QEMU `c30ee9106f7c434abcd359080f4fbfeb39b8f141`; GPU gates9/9,
11/11 USER births. [Command](run16-command.json), [settled status](run16-status.json),
[trace](run16-qemu.log.gz), [excerpt](run16-requests.log),
[clean completion](run16-complete.json), [host health](run16-host-health.txt).

The COPY2 allocation now returns0 and creates an actual Translated host channel
with PRIVILEGED_CHANNEL=0 and privilege=USER, on canonical VA
c1d00002:ff000870. No mirror/birth refusal remains for that request. The next
allocation, NV50_DEFERRED_API_CLASS=0x5080 under the new channel, returns0x56;
the channel is retired with forwarded=0 and submissions=0. Windows retains
Code43/smi9; display still reaches8330 methods but zero UPDATEs/scanouts.
The FBMEM registration and FIFO latency-buffer query still refuse. No exact
RPC-to-Windows-assertion causal claim is made.

Fresh run16 watchdog [recovery](run16-recovery.log) and
[comparison](run16-watchdog-comparison.json):24 saved assertions, first21 identical
to run13; tail hints change from3901fd/391f76/1a1267d to1a1267d/1a3704b/1959d9f.
The earlier1a103e6 assertion is still inside the matching prefix. This does not
prove which RPC caused the tail assertions. The outer NVCD remains one byte
short; complete journal records are usable, no valid-checksum claim. The linked
run16 recovery receipt records read-only NBD/NTFS and cleanup; raw dump remains private.

## Second repair: bounded framebuffer MemoryList registration

The function4 request registers class82, flags48040200, one direct PFN ea6e0,
length128KiB. Its span ea6e0000..ea700000 lies in the VM's usable framebuffer
heap. The former SYSRAM-only class/location/flag decoder refused it.

FBMEM admission is explicit on the audited580.65.06 and580.159.04 guest contracts;
other rows retain SYSTEM-only admission. Class, location and GPU-cache mask come
from the existing public C compiler artifacts. The decoder still requires an
exact direct one-PFN descriptor, contiguous pitch storage, safe arithmetic and
reviewed flags. A separate backing tag prevents video PFNs from reaching guest
RAM topology or CPU-copy authority. SYSTEM cannot name VIDMEM and FBMEM cannot
name PCI RAM. Original flags are retained; registration creates no new mapping.

The object seat validates the live original Device/Subdevice/Client ancestry,
then checks the whole page-rounded span in one unreserved heap region of this
VM's immutable FbLayout. It registers a typed graph resource referencing that
existing storage, with the same parent/device lifetime, quota, duplicate and
free semantics as SYSTEM registration. Firmware carveouts, overflow and outside
store ranges refuse without mutation. No host PFN is forwarded. FB descriptors
mint no RAM generation/CPU-access token; future GPU consumers require their own
checked resource resolution. This is object registration, not a GPU submission.

Source580.65.06 mem_list.c SHA256
689ca84a853980d1d119a5ccb0853e28d3f21da95aabe3668a9e0ab31992d334;
580.159.04 commitb81d58ee0224d1d290bef1c080592b619e184042, fileSHA256
23bf31c98430f6ed76fbfcf034c90e5a337b205a4c92667a55346f58c996e87b.
The only file difference is an extra HW-resource reference increment at740 for
a nonzero source HW-resource handle, which this function4 subset does not carry.
The shared FBMEM branch validates one contiguous PFN and extent against the
Device FB heap before constructing the memory resource (lines418..530).
This is source-backed constructor coverage, not proof that public GSP dispatcher
code or Windows binary internals are available.

Second-repair local validation:18 focused MemoryList tests across ABI, RM, QEMU
and the full policy chain pass; all612 kf-rm tests pass. Clippy across kf-abi,
kf-rm and kf-qemu reports zero new debt (208 existing). Formatting/diff checks
pass. Hardware result pending until a fresh source-pinned Windows boot.

## Run17: FBMEM registration wall removed, Code43 persists

Product/QEMU5348fccfc6469c8b1c2782122b1ea29efd209edf; gates9/9 with11/11
USER births. [Command](run17-command.json), [status](run17-status.json),
[trace](run17-qemu.log.gz), [excerpt](run17-requests.log),
[completion](run17-complete.json), [host health](run17-host-health.txt).
The exact captured class82 FBMEM registration now returns0; no fn4 refusal
remains. COPY2 is born as before. Class5080 construction still returns0x56;
COPY2 retires without submissions, and Windows remains Code43/smi9. No display
UPDATE/scanout is reached. This removes a real registration failure without
claiming that it was the sole Windows initialization blocker.

Run17 [fresh recovery](run17-recovery.log) and
[journal comparison](run17-watchdog-comparison.json):24 assertions, first18 and
last5 match run16; the intervening hint e23660 is replaced by1961253 with a
different immediate caller. The earlier1a103e6 remains; the linked comparison
records this journal-path difference. Names/types or exact RPC causality are not inferred.

## Third repair: Deferred API software-object constructor

The new wall is class5080 allocation under COPY2. Public source constructor
`defapiConstruct_IMPL` at deferred_api.c:237 merely stores optional notification
policy on a ChannelDescendant; the base software object owns an initially empty
DeferredApiList. Its destructor isolates from its channel and deletes pending
entries. The580.65.06 and580.159.04 deferred_api.c files are byte-identical:
SHA2567f1691356d5d39afb64decd1c730b664ea7d979c93adc3a96856d6c96f2c43b0.
Their cl5080.h headers are also identical: optional NV5080_ALLOC_PARAMS is one
NvBool, notifyCompletion. Generated class/layout facts come from that header.

This iteration implements only creation of the software object: original live
channel parent required, no host engine twin, exact optional size/boolean,
notification policy retained as graph facts, graph quotas, idempotent retry,
conflicting retry refusal and channel-subtree destruction. The two audited guest
ABI contracts are explicit; other contracts refuse constructor decoding. The
capability row's origin is Mode2Rpc, not an unverified nvproxy claim.

Deferred controls register actions to be performed later, not a request to run
an action immediately. Source deferred_api.c:682 exposes methods100 (NOP) and200
(execute named pending API); V2/internal controls use a bounded API bundle of
allowed command types, and WAIT_FOR_TLB_FLUSH can postpone completion. Those
controls/methods are not implemented by this constructor change and continue to
refuse. No pending action, notification, GPU work or completion is fabricated.
The next fresh Windows boot will identify which part is actually requested.

Constructor validation:1172 tests across kf-abi and kf-rm pass, including the
independent C-compiler layout oracle, two constructor lifecycle tests, complete
historical capability snapshot comparison after only the two pinned constructor
additions, and proof that deferred controls remain denied. Targeted Clippy across
kf-abi/kf-rm/kf-qemu reports zero new debt (208 existing). Formatting/diff checks
pass. [Compiled layout evidence](deferred-compiled-layouts.json).

## Run18: Deferred constructor wall removed, Code43 persists

Product/QEMU c34d8dac7f6c52cf1db638f5f7b19e65b087f6bf; [gates](defapi-gates.log)
9/9 with11/11 USER births; [build](defapi-build.log). [Command](run18-command.json),
[status](run18-status.json), [trace](run18-qemu.log.gz), [excerpt](run18-requests.log),
[completion](run18-complete.json), [host health](run18-host-health.txt).
Both captured class5080 constructors now return0; COPY2 also allocates its
C7B5 engine object and successfully schedules its group. The newly reached
SET_TIMESLICE control a06c0103 returns0x56 on clientc1d00012/groupff0e0000.
The existing handler only resolves passthrough group members, so it misses
this Translated channel. The shared OGKM handler at kernel_channel_group_api.c:1296
RPCs the scheduling change and records the requested quantum only on success.
Next repair must apply the existing authored unprivileged host control to the
owned Translated channel's group, on the act thread. The run18 trace does not establish that the timeslice refusal causes Code43;
its next group-schedule request succeeds.
Windows remains Code43/smi9; zero display UPDATEs/scanouts and no GPU submissions.

Run18 [fresh recovery](run18-recovery.log) and [journal comparison](run18-watchdog-comparison.json)
retain24 assertions with the first21 matching run17. The tail hints change to
e38498/e37654/304308. The earlier1a103e6 remains. This records a later path
without assigning symbols/types or asserting RPC causality. The outer NVCD is
still one byte short; the complete records are usable, no checksum claim.

## Fourth repair: Translated channel-group timeslice

Run18 reaches a06c0103 after engine-object construction. The existing RM decoder
already requires exactly eight bytes and decodes the public NvU64 timesliceUs.
The QEMU handler now also considers Translated channels in the original client
namespace and resolves their live TSG membership on the act thread. It invokes
the existing host set_timeslice verb on each distinct owned host group; guest
handles and raw control bytes are never sent to the host. Missing/dead members
refuse; a host error becomes the reply error. Host RM performs the scheduling
change and validates/rounds the quantum. No GPU completion is invented, and
no slot lock is held across a host call. Passthrough groups retain their path
with duplicate host-group controls removed.

Source: OGKM580.65.06 ctrla06c.h:129..150 and
kernel_channel_group_api.c:1296..1364, commit307159f2623d3bf45feb9177bd2da52ffbc5ddf9.
Local validation: all101 kf-qemu tests pass; Clippy reports zero new debt
(197 existing in the targeted report). Formatting and diff checks pass.
Hardware result pending until the source-pinned run19.

## Run19: real timeslice control succeeds, Code43 persists

Product/QEMU f6aa9d2e0e16b08873f7e7fb0272fc45d2ddaced; [gates](timeslice-gates.log)
9/9 with11/11 USER births; [build](timeslice-build.log). [Command](run19-command.json),
[status](run19-status.json), [trace](run19-qemu.log.gz), [excerpt](run19-requests.log),
[completion](run19-complete.json), [host health](run19-host-health.txt).
SET_TIMESLICE requests4000us and the actual host control succeeds in207us on
one owned host group, off the GSP lock. The guest reads0 and schedules COPY2.
No a06c0103 refusal remains. Windows still reports Code43/smi9; no display
UPDATE/scanout or GPU submission.

Run19 [fresh recovery](run19-recovery.log) and [journal comparison](run19-watchdog-comparison.json)
retain24 assertions. The first21 match run18, but hint e38498 disappears;
e37654/304308 remain and1a1267d appears at the tail. Offline inspection of the
pinned580.88 driver places e38498 immediately after a failed Boolean call
with a nonzero scheduling quantum; the successful4000us host request plus
its disappearance support associating this hint with SET_TIMESLICE. This
does not assign an NV_STATUS type or claim it caused Code43.

Both boots also show a larger existing gap: kernel GR channels baba0045 and
ff040001 are acknowledged without a birth, then GPU_PROMOTE_CTX refuses
because no owned twin exists. They must gain an actual owned unprivileged
context, and any submitted work must execute with real GPU completion.
Next diagnostic records the fixed inline entries (maximum16 per request,
16 records per VM) under the existing default-off KF3_RPC_TRACE flag.
It uses the generated entry layout and bounded decoded header, never follows
a guest pointer, and changes no reply/admission/execution.

Promotion observer validation: all615 kf-rm tests pass. The new hostile-wire
regression retains both namespaces and raw initialize/nonmapped flags, never
dereferences a numeric physical address, exhausts its16-record budget, and
rejects truncation, extra parameters and an entry count beyond the source
bound. Targeted RM/QEMU Clippy reports zero new debt (207 existing).

## Run20: exact missing-kernel-GR promotion entries

Product/QEMU bf909e1affb5cdbcb3a7f1d87fbff44642ebf166; [gates](promote-observer-gates.log)
9/9,11/11 USER births; [build](promote-observer-build.log), [command](run20-command.json),
[status](run20-status.json), [trace](run20-qemu.log.gz), [inline entries](run20-requests.log),
[completion](run20-complete.json), [host health](run20-host-health.txt).
The observer records two requests, with nine entries each, and no reply change.
Both fail on the missing kernel-GR twin, rather than a decoder error. MAIN(0),
PATCH(2), FECS_EVENT(9) and privilege map(11) have PA+VA+size and initialize=1.
Bundle/pagepool/attribute/RTV(3..6) carry only VA, with size/PA/initialize=0.
Unrestricted privilege map(10) is initialize-only/nonmapped. Initialized ids
are0/2/9/10/11; mapped ids0/2/3/4/5/6/9/11. PhysAttr4 is VIDMEM plus
GPU_CACHEABLE_NO; PhysAttr5 is coherent system memory plus GPU_CACHEABLE_NO.
Source: ctrl2080gpu.h:892..933 and kernel_graphics_context.c:1690..1950,
OGKM580.65.06 commit307159f2623d3bf45feb9177bd2da52ffbc5ddf9.
This explains the declared buffers, without assuming a Windows-only consumer.
Windows remains Code43/smi9 with no display UPDATE/scanout or GPU submission.
Next: real owned unprivileged GR context and Translated CE work on its runlist,
first tested on bare metal; promotion must require that context, per owner rulingB.

## Fifth repair, first increment: bare-metal GR-runlist CE execution

HostRing now supports a graphics-runlist ring with a real owned compute object
(and hence a host-constructed GR context). It selects a graphics copy engine
from live unprivileged host CE capabilities, instead of assuming a per-die
index. The channel header's dedicated copy subchannel is generated from
cla06fsubch.h; all normalized command headers are routed there while every
operand remains identical, including RELEASE_WFI completion and NSI. The next
increment translates a compatible CE SET_OBJECT selector to the actual host class.
Unsupported forms/truncated arguments refuse before any host PB store. Segment
length remains bounded to half the owned ring's PB. The ordinary async-CE
ring is unchanged. Guest kernel-GR admission/promotion is not enabled yet.

Additional native arm: KF_GATE3_GRAPHICS=1 runs the existing physical/sysmem/
virtual copy, remap-at-split, native guest semaphore and hostile-entry checks
on this ring, and checks that an actual host GR object exists. This establishes
execution before a Windows boot. Source580.65.06 kernel_ce_context.c:110..154
requires a COPY engine affinity even on a GR channel; cla06fsubch.h defines its
CE routing. The context constructor uses the existing authored unprivileged
compute-object verb and generated family class, never guest context bytes.

Local ABI/channel validation: 645 tests pass, including two route regressions
for unchanged arguments/native ordering and rejection before partial routing.
The gate3 harness compiles. Clippy and formatting/diff checks pass; hardware
result pending.

### Native result and kernel-GR admission increment

Native source b7ef3e5685319ca087ce73a5c01569083f977b9c: [gates](gr-ce-native-gates.log)
9/9,11/11 USER births; [graphics-runlist arm](gr-ce-native.log) passes every
gate3 check, including the actual owned GR context, native physical/sysmem/virtual
copies, remap-at-split, GPU semaphore and rejection without hostile retirement.
The host remains healthy. This arm precedes guest kernel-GR admission.

The next increment admits kernel GR channels only under KF3_KERNEL_GR_CE=1
and KF3_TSPACE=1, both on this experiment branch. Default admission is unchanged.
They get an actual unprivileged graphics-runlist ring and owned host context
at allocation, with the ordinary bounded slot, USERD and lifetime checks.
Promotion/engine bind requires the matching live context and retains guest
context state; it does not read/write guest context buffers or replay addresses.
Eviction performs an actual host runlist disable before replying, retaining
the owned context for later scheduling. This follows owner rulingB's host-owned
context substitution; Windows buffer read-back semantics remain an experiment.

GPU execution is limited to the existing Translated CE/host-method protocol;
arbitrary kernel GR methods and unsupported software methods still refuse.
No kernel shader/3D execution or Deferred API execution support is claimed.
The additional compatible-class arm sets the source-defined guest CE selector
and routes it to the host's allocated CE class. SET_OBJECT admits only one
source-derived CE class; foreign classes and wider writes refuse. Other data
is unchanged. Native validation of that increment and Windows run21 pending.

Native compatible-class result at3fd6fc39513edf8581dfba16f5887d0d4d7a0b36:
[arm](gr-ce-compat-native.log) passes all gate3 checks with guest C7B5 selecting
the actual host CE object. [Gates](kernel-gr-gates.log) pass9/9 with11/11 USER
births; [immutable build](kernel-gr-build.log). Local188 channel/QEMU tests pass,
with zero new Clippy debt (197 existing); gate3/route Clippy also has zero new
debt. Windows run21 now uses this source-pinned binary and records the new flag.

## Run21: kernel-GR contexts exist, real GPU work begins, NVDEC wall follows

Product/QEMU3fd6fc39513edf8581dfba16f5887d0d4d7a0b36; [command](run21-command.json),
[status](run21-status.json), [trace](run21-qemu.log.gz), [excerpt](run21-requests.log),
[completion](run21-complete.json), [host health](run21-host-health.txt).
Both captured kernel-GR channels now birth with actual USER host rings/contexts
and their promotions return0, with initialized/bound state retained. This also
lets the RM-internal scrubber execute two real GPU submissions and advance
GP_GET to2. Five Translated births, zero poisoned/contended serves or refused
acts; every released T-space ring slot is recovered. Windows still has
Code43/smi9 and zero display UPDATEs/scanouts.

The next missing channel is kernel NVDEC0, engine13 (`cl2080_notification.h:301`).
ClassC56F channel ff040003 is acknowledged without birth. Its subsequent legacy
Falcon-context promotion declares VA1203cd000 +4KiB, entryCount0 and engine13;
it refuses0x56 because no owned twin exists. This is a distinct video-context
constructor/lifetime gap, not a malformed nine-entry GR request. A newly reached
0080170f control also refuses; it is FIFO_SET_CHANNEL_PROPERTIES
(`ctrl0080fifo.h:219-290`), whose individual payload still needs decoding. Next:
actual owned unprivileged NVDEC context, independently tested on bare metal
before enabling its guest path; no unsupported codec work may be completed.

Run21 [fresh watchdog recovery](run21-watchdog-recovery.log) verifies read-only
disk access and cleanup. The [journal comparison](run21-watchdog-comparison.json)
retains the independent L reference and pinned Windows driver identity; its outer
NVCD envelope remains one byte short, so no complete checksum claim is made.

## Native NVDEC context increment (guest path not enabled yet)

HostRing now constructs an actual decoder object on its USER NVDEC channel,
choosing a class from the generated family set intersected with this host's
bounded unprivileged class-list response. Object construction performs host
Falcon allocation/promotion; no guest context address reaches the host.
Decoder rings refuse every nonempty public submission before PB writes;
only their private authored FIFO fence can be queued. The native
`kf-nvdec-context` oracle checks ownership, codec-selector refusal, actual GPU
fence completion and channel/ring release. Hardware execution pending.
GPU-free host/channel tests:114 passed; oracle compiles; Clippy new0.

Native source7152d1a8448715071cdb244daf5f672f48d85568 on the borrowed4070/
open595.91.07: [decoder oracle](nvdec-native.log) constructs USER engine13
channel and ownedC9B0 object, refuses a codec selector, then its real GPU fence
reaches seq1. Channel free and all ring mappings/view/object releases succeed;
host display remains enabled. This gates the default-off experimental guest
`KF3_KERNEL_NVDEC_CTX=1` path, limited to private T-space and an actually owned
matching context. Promotion accepts decoder entryCount0 only; codec/CE work
remains refused before host PB writes.

Guest increment validation:101 QEMU tests passed; Clippy new0. The existing
bounded inline observer also records up to16 combined promotion/property
requests:0080170f requires an exact16-byte inline scalar payload, never follows
an address, and changes no reply. Two observer tests pass, including truncated/
extra-size refusal and record-budget exhaustion. Windows run22 pending.

## Native NVENC context increment (guest path not enabled yet)

The native decoder oracle additionally selects NVENC0 with KF_NVENC_CONTEXT=1.
HostRing's owned video context selects generated decoder/encoder family classes
intersected with the actual host's class list and authors its engine index.
All nonempty public submissions still refuse before PB writes; host-only FIFO
fence execution and release are the native oracle. Hardware NVENC pending.
Channel tests87 passed and the extended native oracle compiles; Clippy new0.
Full RM suite after the bounded property observer:616 passed.

## Run22: NVDEC promotion succeeds; next Falcon context is NVENC1

Product/QEMUb52da0c7 (full identity in [command](run22-command.json));
[status](run22-status.json), [trace](run22-qemu.log.gz),
[requests](run22-requests.log), [completion](run22-complete.json),
[host health](run22-host-health.txt), [9/9 gates](run22-gates.log),
[immutable build](run22-build.log). Code43/smi9 persist; BAR0 decoding is
already disabled at sampling time, so capability capture attempts no page read.
The exact source identifier is retained in command and build records.

NVDEC0 clientc1d00018/ff040003 births as an actual USER host channel with owned
C9B0 object, then legacy Falcon GPU_PROMOTE_CTX returns0. Its kernel guest
context VA1203cd000 +4KiB is never used on the host. Seven Translated channels
birth; two actual scrubber GPU submissions complete, GP_GET2. Decoder retires
with no submissions, no dead state and no codec completion is authored.

Next: clientc1d0001a/ff040005 kernel engine1c = NVENC1 has no birth path;
its legacy Falcon promotion carries VA1203ce000 +4KiB, entryCount0, and
returns0x56. Native owned encoder construction/fence oracle is next.
The scalar property observer captures0080170f on clientc1d00016's COPY2 channel
ff040002: property0 ENGINE_TIMESLICE_IN_MICROSECONDS, value0xfa0 =4000.
This independent channel-specific timeslice still refuses; no causal claim
about the current Code43 outcome follows from that refusal alone.

Run22 [fresh watchdog recovery](run22-watchdog-recovery.log) verifies read-only
access and cleanup; [journal comparison](run22-watchdog-comparison.json) retains
all24 complete assertions identical to run21 despite the further NVDEC RPC
progress. The outer NVCD envelope is still one byte short; no valid checksum
or journal-only causal assignment is asserted.

Native203a9617 NVENC0 test: USER allocation succeeds but host bind refuses
Other(87). The Windows request is NVENC1, not NVENC0 (`cl2080_notification.h:313`).
The oracle now takes a bounded explicit KF_NVENC_CONTEXT_INDEX selecting0..3;
probe the requested instance1 next. No guest NVENC admission yet.

Native sourcee465d35625d7b13893285ab51df3d879c47ffb96 probes the exact
NVENC1/engine1c instance: [native oracle](nvenc1-native.log) constructs a USER
channel and actual ownedC9B7 object, rejects codec selection, completes GPU
fence seq1, then frees its channel and every ring resource. Host display remains
active. This gates experimental KF3_KERNEL_NVENC_CTX=1 with private T-space;
matching owned-context checks and refusal of all codec/CE submissions are shared
with the decoder path. Windows run23 pending.

NVENC guest increment:101 QEMU tests passed; Clippy new0.

## Native OFA context increment (guest path not enabled yet)

The native oracle selects bounded OFA0..1 with KF_OFA_CONTEXT_INDEX.
HostRing selects optical-flow classes from the generated family set intersected
with this host's actual class list. The existing passthrough allocation helper
already authors OFA's12-byte `{size, prohibitMultipleInstances, engineInstance}`
(`nvos.h:3011-3016`); decoder and OFA have identical scalar layouts. No guest
address/parameter is forwarded and every nonempty public submission refuses.
Channel tests87 pass, native oracle compiles, Clippy new0. Hardware OFA pending.

## Run23: both NVENC1 promotions succeed; OFA0 is next

Product/QEMUe462194d (full revision in [command](run23-command.json));
[status](run23-status.json), [trace](run23-qemu.log.gz),
[requests](run23-requests.log), [completion](run23-complete.json),
[host health](run23-host-health.txt), [9/9 gates](run23-gates.log),
[immutable build](run23-build.log). Code43/smi9 persist; BAR0 disabled at sampling.

Clients c1d0001a/ff040005 and c1d0001b/ff040006 both birth real USER NVENC1
channels and ownC9B7 contexts. Both legacy Falcon promotions return0. Nine
Translated channels birth; the same two real scrubber GPU submissions complete.
No decoder/encoder work is submitted or emulated. The newly reached kernel
OFA0/engine33 clientc1d0001c/ff040007 has no birth path; its Falcon promotion
(entryCount0, guest VA120537000 +4KiB) refuses0x56. Native OFA context/fence
oracle precedes any new guest admission. A second0080170f request sets the
new NVENC1 channel ff040006 engine timeslice to4000us; it also still refuses.

Run23 [fresh watchdog recovery](run23-watchdog-recovery.log) confirms read-only
access and cleanup. The [comparison](run23-watchdog-comparison.json) retains
all24 complete assertions identical to run22, despite further RPC progress;
outer NVCD still one byte short, no valid checksum claim.

Native sourceff12e6a7f2d22a72ee0380d0e27555aca716a3a9 probes OFA0/engine33:
[native oracle](ofa-native.log) constructs a USER channel and ownedC9FA object,
refuses class selection, completes real GPU fence seq1 and releases all ring/
channel resources. Host display stays enabled. This gates default-off
KF3_KERNEL_OFA_CTX=1 with private T-space and actual context ownership; OFA/codec/
CE submissions remain unsupported. Windows run24 pending (superseded 2026-10-07:
[run24 result](#run24-ofa0-promotion-succeeds-the-abort-moves-to-get_rc_recovery)).

OFA guest increment:101 QEMU tests pass; Clippy and claims new0.

## Run24: OFA0 promotion succeeds; the abort moves to GET_RC_RECOVERY

Product/QEMU a6f84d0d9de8ba2022868356466fc2a4fab02343 (see [command](run24-command.json));
[status](run24-status.json), [trace](run24-qemu.log.gz), [requests](run24-requests.log),
[completion](run24-complete.json), [host health](run24-host-health.txt),
[9/9 gates](run24-gates.log), [immutable build](run24-build.log). The controller
saw two identical status samples after more than 90 s of uptime: the NVIDIA
adapter has ConfigManagerErrorCode 43 and nvidia-smi exits 9. The guest shut down
cleanly, the unit result is success, and the host RTX 4070 on 595.91.07 is healthy.

Kernel OFA0 on client c1d0001c, channel ff040007 (engine 0x33) is born as a real
USER host channel with an owned C9FA object. Its legacy Falcon promotion returns 0
and its GPFIFO_SCHEDULE returns 0. Eleven Translated channels are born, with the
same two real scrubber GPU submissions. No OFA, codec or CE work is submitted or
emulated. The next refusal is NV2080_CTRL_CMD_GET_RC_RECOVERY (0x2080220e,
`ctrl2080rc.h:235-269`); the driver starts tearing down immediately after it
(see the analysis below).

[Fresh watchdog recovery](run24-watchdog-recovery.log) used the audited read-only
NBD/NTFS tool after the supervisor stopped, and cleanup was verified (2026-10-07). The dump
mtime is 23 s after experiment start. The [comparison](run24-watchdog-comparison.json)
shows all 24 assertions identical to run23 and run22. The outer NVCD is still
one byte short, so no checksum claim is made. The raw dump remains private on
the controller.

## Abort-point analysis against the VFIO reference

**Method.** The VFIO boots boundary-vfio-8/9/10 (same baseline disk, Windows
580.88, real RTX 4070 behind VFIO) all ended with Code 0 and nvidia-smi exit 0.
Each one has a complete narrow GSP-observer export: 8208 records, no drops and no
sequence gaps for vfio-10. No new VFIO run was needed. `scripts/bench/windows/abort_point.py`
decodes that export and pairs requests with replies in queue order. It also parses
the kayfabe `KF3_RPC_TRACE` logs. Handles and timestamps are left out of every key.
The control names come from OGKM 580.65.06 headers
([names](ogkm-580.65.06-control-names.txt)).

**Abort point per run** ([table](abort-points-run13-24.txt)):

| runs | last RPC before teardown | kayfabe result | RPCs before teardown |
|---|---|---|---|
| 13-15 | alloc AMPERE_CHANNEL_GPFIFO_A (COPY2) | 0x40 | 296 |
| 16-17 | alloc NV50_DEFERRED_API_CLASS | 0x56 | 297 / 300 |
| 18-23 | GPU_PROMOTE_CTX (GR, then NVDEC, then NVENC1, then OFA) | refused | 322 → 433 |
| 24 | GET_RC_RECOVERY | refused | 448 |

Every repair moved the abort point forward. About 50 other refusals earlier in
each boot are tolerated, and the real GSP itself returns 24 non-OK statuses
before this point (for example 0x20800a87 and 0x20800b05 both return 0x56).
So a refusal is fatal only at specific StartDevice call sites.

> **Correction, 2026-10-07 (batch after run24; [details](#sixth-repair-startdevice-batch-after-get_rc_recovery)).**
> The paragraph below reads `rcEnable` at the wrong offset. From 575 on, the
> `rpc_gsp_rm_control` header is 40 bytes and `params` starts at body offset 40
> (`kf_abi::versions`, measured layout); offset 36 is `reserved0`, which is always zero. Read at
> offset 40, the physical GSP answered **`rcEnable=1` (ENABLED)** at index 2515 in
> vfio-8, vfio-9 and vfio-10 alike. Windows then sent **`SET_RC_RECOVERY(ENABLED)`**.
> The GET request carried uninitialised stack bytes (vfio-10: `0x4da066c8`). So
> "DISABLED, matching the VFIO reply" is false. DISABLED matches only the `_VF`
> HAL, which is what a vGPU guest receives. The class-0x78 and
> EVENT_SET_NOTIFICATION parts of the paragraph are correct.

**VFIO at the same point.** In all three VFIO boots, GET_RC_RECOVERY is at RPC
index 2515. The physical GSP answers with status 0 and `rcEnable=0`
(DISABLED), with rmctrlFlags 0x40154 (PRIVILEGED, ROUTE_TO_PHYSICAL,
PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST, among others). Windows then calls
SET_RC_RECOVERY with DISABLED, followed by 25 NV01_EVENT_KERNEL_CALLBACK (0x78)
allocations plus EVENT_SET_NOTIFICATION, and then display bring-up. OGKM's
vGPU-guest handler `subdeviceCtrlCmdGetRcRecovery_VF` (`kernel_rc_ctrl.c:321-329`)
returns DISABLED as a constant.

**Forecast** ([full list](vfio10-forecast-after-run24.txt)). After index 2515,
98 distinct controls/classes (991 RPCs) were either refused or never seen in
any kayfabe run 13-24. In VFIO order, the first ones are SET_RC_RECOVERY,
class 0x78, NV0073 display event/active/SOR/DP-AUX controls, and NVC372
IS_MODE_POSSIBLE (156 calls). After those come ZBC_CLEAR, DMA_SET_DEFAULT_VASPACE,
FIFO_DISABLE_CHANNELS and the perf/thermal 0x2080a0xx groups. Some of these
are fatal and some are tolerated; only a boot can tell which. This list lets
source-backed support be implemented in batches instead of one wall per boot.

**Ranked hypotheses.**
1. *(High; direct evidence in 12 runs.)* Code43/smi9 is StartDevice aborting
   on the first refusal it does not tolerate. Today that is GET_RC_RECOVERY.
   0xC000009A is the KMD's generic StartDevice status.
2. *(High, as a structural fact.)* The 24 saved assertions are a full 4 KiB
   journal holding the first asserts of the boot. They cannot identify or
   exclude late walls, so their invariance is not evidence against causality.
3. *(Medium.)* More fatal walls follow inside the forecast list, especially in
   display bring-up (NV0073/NVC372). These need real semantics, not success
   stubs.
4. *(Low; no evidence.)* Non-RPC causes such as BAR sizes, the 4 GiB virtual FB
   or timing. In every run the abort coincides with an RPC refusal, and no
   BAR0 or timing difference has been needed to explain it. Not ruled out for
   later walls.

**Next experiment (proposed, not implemented).** *(Superseded 2026-10-07: implemented as
part of the [sixth repair](#sixth-repair-startdevice-batch-after-get_rc_recovery). The
"matching the VFIO reply" reason below is wrong; see the correction above.)* Give 0x2080220e/0x2080220d a
VM-scoped, source-backed implementation. GET reports the VM's own RC-recovery
setting, initially DISABLED (matching the VFIO reply and the `_VF` HAL). SET
records DISABLED and refuses ENABLED unless kayfabe implements RC recovery for
the VM's own channels. Nothing is forwarded to the host GPU's global policy.
Then boot run25 and check the predicted outcome: the teardown moves past VFIO
index 2516, and the new abort RPC is one of the forecast entries (most likely
class 0x78 or an NV0073 control). If the abort is anywhere else, hypothesis 1
needs revisiting.

## Sixth repair: StartDevice batch after GET_RC_RECOVERY

This batch takes the first forecast entries in VFIO order. Each one is answered
from OGKM-backed semantics or refused by name. Nothing reaches the host GPU,
and no display state or completion is invented.

| VFIO index | RPC | kayfabe answer | source |
|---|---|---|---|
| 2515 | GET_RC_RECOVERY 0x2080220e | `rcEnable=DISABLED`, the VM's own setting | `_VF` HAL, `kernel_rc_ctrl.c:321-329` (580.65.06) |
| 2516 | SET_RC_RECOVERY 0x2080220d | DISABLED → OK; ENABLED → 0x56; other values → 0x1f | `ctrl2080rc.h:251-269`; the `_VF` body returns OK for any value (`g_subdevice_nvoc.h:7788-7790`), so kayfabe is stricter |
| 2517… | alloc NV01_EVENT_KERNEL_CALLBACK 0x78 (×25) | object-graph edge; params never decoded | `resource_list.h:2200-2210`, same `NV0005_ALLOC_PARAMETERS` as 0x7e |
| 2558/2560 | NV0073 EVENT_SET_NOTIFICATION 0x00730301 | per-object action table with RM's checks (no event list, out-of-range notifier or subdevice, unbound hEvent, double enable) | `disp_objs.c:629-700`, `event_notification.c:1101-1129` |
| 2571 | PERF_GET_POWERSTATE 0x2080205a | `AC` | `_VF` HAL, `kern_perf_ctrl.c:293-308` |

Code: `kf_rm::vfguest` (new chain link), the 0x78 capability row at 580.65.06+,
`kf_disp` display-event bindings (bounded to 64 objects × 32 events, retired on
FREE and on GSP re-init), and re-derived display layouts
(`tools/derive_display_layouts.sh`, which now has 5 more facts per version).
Tests: kf-abi 558, kf-chip 71, kf-disp 119, kf-rm 621 (including
`tests/code43_startdevice_batch.rs`, which runs the VFIO sequence through the
whole chain), kf-qemu 101. Clippy new 0. rustfmt is clean.

**What this batch does not claim.**
- *RC recovery.* kayfabe reports DISABLED because it performs no robust-channel
  recovery for the VM. The real GSP reported ENABLED (see the correction above),
  and the host's own setting is PRIVILEGED (flag 0x4 of 0x40154), so an
  unprivileged host client cannot read it. If Windows insists on
  `SET(ENABLED)`, kayfabe refuses it, and the next decision is the owner's
  (see below).
- *Events.* The 0x78 registrations and the enabled NV0073 notifiers 1 and 2 are
  bookkeeping. kayfabe posts no event for them. That is accurate as long as the
  virtual display raises no such notifier.
- *Unreached display controls.* SYSTEM_GET_ACTIVE (0x0073010c) and
  NVC372 IS_MODE_POSSIBLE (0xc3720101) were already claimed by the display
  model. They are "never-seen" in the forecast only because no boot reached them.

**Left refused in this batch (each needs semantics that do not exist yet).**
`0x007302a3` (not named in the 580.65.06 headers), DFP_ASSIGN_SOR
`0x00731152`, DP_AUXCH_CTRL `0x00731341` (the virtual connector is DVI-D, not DP),
PSR_GET_SR_PANEL_INFO, DP_GET_LINK_CONFIG, `0x00730282` (the real GSP returns
0x56 here too), NV5070 IMP_SET_GET_PARAMETER, DFP_SET_ELD_AUDIO_CAPS, and the
later perf/thermal groups.

**Prediction for run25.** The teardown moves past VFIO index 2516. If Windows
echoes the GET value, it sends SET(DISABLED), which is accepted. If it always
sends SET(ENABLED), run25 aborts at 0x2080220d with 0x56. *(Outcome 2026-10-07: the second branch; see [run25](#run25-get_rc_recovery-is-served-windows-insists-on-set_rc_recoveryenabled).)*

## Run25: GET_RC_RECOVERY is served; Windows insists on SET_RC_RECOVERY(ENABLED)

Product/QEMU 3b436488c3476446f26c620bd91077d23f382fb5 (see [command](run25-command.json));
[status](run25-status.json), [trace](run25-qemu.log.gz), [requests](run25-requests.log),
[completion](run25-complete.json), [host health](run25-host-health.txt),
[unit result](run25-unit-result.txt), [9/9 gates](run25-gates.log) (11/11 USER births),
[immutable build](run25-build.log) (`kf3-bins/3b436488`). Run on 2026-10-07 at
08:46-08:49 UTC: one VM, serial, same baseline and flags as run24. The controller
saw two identical status samples at 98 s of uptime. The NVIDIA adapter has
ConfigManagerErrorCode 43 and nvidia-smi exits 9, so Code43 persists. No
initialization success is claimed. The guest shut down cleanly and the unit
result is success. Afterwards the host RTX 4070 on 595.91.07 had its display
enabled, was in P8, logged no Xid since the build started, ran no QEMU, and had
NBD disconnected.

**Abort point** (`abort_point.py points`, appended to
[the table](abort-points-run13-24.txt)): 696 RPCs, teardown at 449, last RPC
`SET_RC_RECOVERY` 0x2080220d → 0x56. It moved by exactly one RPC.
`GET_RC_RECOVERY` now returns 0 with `rcEnable=DISABLED`. The next RPC is
`SET_RC_RECOVERY`, which `kf_rm::vfguest` refuses with 0x56. That status is
returned only for `rcEnable=ENABLED`: a malformed size or value returns 0x1f.
So Windows sent ENABLED even though GET had reported DISABLED. It does not echo
the GET value. This matches the second branch of the prediction above.
Everything else matches run24: 11 Translated births, and the distinct refusal set is run24's with
0x2080220e replaced by 0x2080220d. No 0x78 allocation or NV0073 event control was reached (they
follow SET in the VFIO order). The [watchdog recovery](run25-watchdog-recovery.log)
(read-only NBD/NTFS, cleanup verified 2026-10-07) found a fresh dump 10 s after
experiment start. No assertion comparison was made: the 4 KiB journal cannot
show a late wall (see the correction at the top).

**Owner decision needed: what SET_RC_RECOVERY(ENABLED) may return.** The
StartDevice path needs it to succeed. These are the options, none of them
implemented:

1. *Accept it with no effect, as the `_VF` HAL does* (`g_subdevice_nvoc.h:7788-7790`
   returns OK for any value, and GET keeps reporting DISABLED). This is NVIDIA's
   own vGPU-guest behaviour. It is also a SET that is acknowledged and then
   ignored, which the "no faked success" rule normally forbids.
2. *Report and accept ENABLED as a derived fact.* The VM's channels are
   host-RM channels, and host RM performs robust-channel recovery on them. But
   the host's setting is a PRIVILEGED control (0x40154 includes 0x4), so an
   unprivileged client cannot read it. "ENABLED" would then be inferred, not
   derived.
3. *Implement per-VM RC recovery* for the VM's own channels: fault → channel
   reset → guest notification. Then ENABLED becomes true. This is a large
   piece of work.

**After this decision (no further owner input needed):** the 25 class-0x78
allocations and the NV0073 EVENT_SET_NOTIFICATION pair are already served by
this batch and are covered by `tests/code43_startdevice_batch.rs`.
PERF_GET_POWERSTATE (AC) is next. SYSTEM_GET_ACTIVE and IS_MODE_POSSIBLE are
already claimed. The first refusals after those in VFIO order are 0x007302a3,
DFP_ASSIGN_SOR and DP_AUXCH_CTRL (see the sixth repair's "left refused" list).

## DIAGNOSTIC run26: RC recovery reported and accepted as ENABLED

**DIAGNOSTIC, not a repair. Owner request, 2026-10-07.** The default-off flag
`KF3_RC_RECOVERY_ENABLED_DIAG=1` (`kf_rm::vfguest::RcRecovery::EnabledDiagnostic`)
makes GET report ENABLED, as the passthrough GSP did. SET then accepts both
ENABLED and DISABLED, and any other value still gets 0x1f. kayfabe performs no
per-VM recovery and nothing reaches the host, so ENABLED is knowingly unbacked.
With the flag off, behaviour is unchanged. The runner adds the flag only with
`pc_sdr_experiment.py --rc-recovery-enabled-diag`, and records it in
`command.json`. This setting must not be merged turned on.

**Falsifier, stated before the run:** the 2026-10-07 batch (class-0x78 edges,
NV0073 events, power state) is worth keeping only if the abort moves well past
VFIO index 2518. That index is the first 0x78 allocation plus its
EVENT_SET_NOTIFICATION.
