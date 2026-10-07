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

### Run26 result (DIAGNOSTIC; falsifier triggered for now)

Product/QEMU 8208effee6b020c421e3673c877856da252e6aca, `kf3-bins/8208effe`, run on
2026-10-07 at 10:18-10:21 UTC. [command](run26-command.json) (`flags` records
`KF3_RC_RECOVERY_ENABLED_DIAG: "1"`), [status](run26-status.json),
[trace](run26-qemu.log.gz), [requests](run26-requests.log),
[completion](run26-complete.json), [host health](run26-host-health.txt),
[unit result](run26-unit-result.txt), [9/9 gates](run26-gates.log) (11/11 USER
births), [build](run26-build.log), [watchdog recovery](run26-watchdog-recovery.log)
(cleanup verified 2026-10-07). After 98 s of uptime the NVIDIA adapter has
ConfigManagerErrorCode 43 and nvidia-smi exits 9, so Code43 persists. No
initialization success is claimed. Afterwards the host was healthy: display
enabled, P8, no Xid, no QEMU, NBD disconnected.

**Abort point:** 699 RPCs, teardown at 451. The last RPC is
`NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` 0x20800301 → 0x56. The sequence was:
GET (now ENABLED) 0x0, SET(ENABLED) 0x0, the first class-0x78 alloc 0x0
(handle ff060040, parent the subdevice), and then the first notifier arming,
which was refused. That is **VFIO index 2518**, so the abort moved exactly to
the falsifier's line and no further. The other 24 0x78 allocations, the NV0073
event notifications (2558/2560), PERF_GET_POWERSTATE (2571) and every display
control were **not reached**. The distinct refusal set is run25's with
0x2080220d replaced by 0x20800301.

**Cause.** The `0x20800301` handler accepts a notifier index only if
`kf_abi::eventnotify` can argue for it as silent, delivered or guest-raised.
Index 0x2c (44, THERMAL_DIAG_ZONE) has no such argument. The VFIO boot then arms
23 indices, all REPEAT, in this order: 44 THERMAL_DIAG_ZONE, 43 COOLER_DIAG_ZONE,
113 STEREO_EMITTER_DETECTION, 120 HOTPLUG_PROCESSING_COMPLETE, 4 THERMAL_HW,
33 PSTATE_CHANGE, 139 RUNLIST_PREEMPT_COMPLETE, 157 UCODE_RESET, 197 GPU_RC_RESET,
122 RESERVED122, 158 PLATFORM_POWER_MODE_CHANGE, 2 POWER_CONNECTOR, 26 CE3,
12 GRAPHICS, 23 CE0, 24 CE1, 1 HOTPLUG, 7 DP_IRQ, 45 AUDIO_HDCP_REQUEST,
34 HDCP_STATUS_CHANGE, 118 POWER_EVENT, 178 HDMI_FRL_RETRAINING_REQUEST and
182 AUX_POWER_STATE_CHANGE.

**Next walls, ranked by what each needs:**
- *Derivable now* (a source-backed "cannot occur on this virtual device"
  argument per index; add rows to `SILENT_NOTIFIERS`): thermal/cooler diag
  zones, stereo emitter, power connector, platform power mode, HDCP/audio-HDCP,
  HDMI FRL, DP_IRQ (the virtual connector is DVI), aux power. Each needs its
  own OGKM citation for who raises it.
- *Needs owner decision:* GPU_RC_RESET (197) and UCODE_RESET (157). They depend
  on the RC-recovery question, and arming them while recovery is diagnostic-only
  would promise events that never come. Also HOTPLUG (1) and
  HOTPLUG_PROCESSING_COMPLETE (120): the virtual monitor can be resized (display
  step 3c), so these are not silent.
- *Needs new semantics* (events that do occur on real work): CE0/CE1/CE3 and
  GRAPHICS non-stall notifiers, RUNLIST_PREEMPT_COMPLETE, PSTATE_CHANGE and
  POWER_EVENT. Either kayfabe delivers them, or the guest raises them itself
  (`GUEST_RAISED_NOTIFIERS`), which has to be shown per index. After these
  come PERF_GET_POWERSTATE (served) and the display walls listed in the sixth
  repair.

## Seventh repair: owner ruling §S applied (RC-recovery stub, the notifier family, real hotplug)

> ⊘ *Supersedes the DIAGNOSTIC flag above (2026-10-07):* `KF3_RC_RECOVERY_ENABLED_DIAG` and
> the runner's `--rc-recovery-enabled-diag` option are removed. RC recovery is now a stub under
> owner ruling §S (`docs/OWNER_RULINGS.md`), not a diagnostic.

Owner ruling §S (2026-10-07) has three parts. Privileged host management with no compute or
display effect is stubbed. Unprivileged features, including the virtual monitor and its
hotplug, are implemented for real. GPU work is never forged. Applied as follows:

- **RC recovery (stub).** GET reports the VM's recorded setting, which starts ENABLED, the
  value the passthrough GSP gave. SET records ENABLED or DISABLED; any other value returns 0x1f.
  There is no host action and no per-VM recovery behind it (`kf_rm::vfguest`).
- **`0x20800301` notifier arming.** These are the 23 indices of vfio-10's RPCs 2518-2566.
  `kf_abi::eventnotify::RULED_NOTIFIERS` gives each accepted index its source citation:

  | treatment | indices |
  |---|---|
  | stub (§S.1) | 2 POWER_CONNECTOR, 4 THERMAL_HW, 43 COOLER_DIAG_ZONE, 44 THERMAL_DIAG_ZONE, 157 UCODE_RESET (raised by the guest's own CPU-RM at `kernel_gsp.c:2469`), 158 PLATFORM_POWER_MODE_CHANGE (raised by the guest's CPU-RM at `platform_request_handler_ctrl.c:2129`), 182 AUX_POWER_STATE_CHANGE, 197 GPU_RC_RESET (raised by the guest's CPU-RM at `kernel_rc_callback.c:360`) |
  | absent on the virtual DVI-D/TMDS display | 7 DP_IRQ, 34 HDCP_STATUS_CHANGE, 45 AUDIO_HDCP_REQUEST, 113 STEREO_EMITTER_DETECTION, 178 HDMI_FRL_RETRAINING_REQUEST |
  | real, posted by the display plane (§S.2) | 1 HOTPLUG: the class-0x78 event whose notify index is HOTPLUG now registers as the hotplug target, so a monitor resize posts a list `POST_EVENT` to it (fanned out at `kernel_gsp.c:514-522`) |
  | already accepted as raised by the guest | 118 POWER_EVENT |
  | **refused** (next walls) | 120 HOTPLUG_PROCESSING_COMPLETE (no producer anywhere in OGKM, so kayfabe cannot post it from its own state), 33 PSTATE_CHANGE, 139 RUNLIST_PREEMPT_COMPLETE, 12 GRAPHICS, 23/24/26 CE0/CE1/CE3 (none derived from real host events yet), 122 RESERVED122 (no defined meaning) |
- **NV0073 notifiers 1 and 2** (display common): kayfabe mirrors RM's own handler, which
  accepts any index below 6. The open headers name none of 1-4, and this display raises none of
  them.
- **Guest thermal and power queries** are not answered by any kayfabe link (they stay
  refused), so they agree with the stubs. PERF_GET_POWERSTATE reports AC, the `_VF` value.

Tests: 1471 across kf-abi, kf-chip, kf-disp, kf-rm and kf-qemu, including the Windows arming
order in `tests/code43_startdevice_batch.rs` and the pinned ruled list. Clippy new 0,
rustfmt clean, ci_gates clean.

**Falsifier, stated before run27:** this batch is worthwhile only if the abort moves well past
VFIO index 2518 into display setup (the NV0073 controls and IS_MODE_POSSIBLE, 2558 onwards).
**Prediction:** the fourth arming, HOTPLUG_PROCESSING_COMPLETE at VFIO index 2524, is refused.
If Windows treats that refusal as fatal like the first, run27 aborts there and the falsifier
holds again, until 120 has a source-backed producer.

### Run27 result: the abort moves to HOTPLUG_PROCESSING_COMPLETE (VFIO 2524); the falsifier holds

Product/QEMU 4448be5399683e6534a3f6a72b005164ae7bebe6, `kf3-bins/4448be53`, run on
2026-10-07 at 11:27-11:30 UTC. [command](run27-command.json), [status](run27-status.json),
[trace](run27-qemu.log.gz), [requests](run27-requests.log),
[completion](run27-complete.json), [host health](run27-host-health.txt),
[unit result](run27-unit-result.txt), [9/9 gates](run27-gates.log) (11/11 USER births),
[build](run27-build.log), [watchdog recovery](run27-watchdog-recovery.log) (cleanup verified
2026-10-07). After 99 s of uptime the NVIDIA adapter has ConfigManagerErrorCode 43 and
nvidia-smi exits 9, so Code43 persists. No initialization success is claimed. Afterwards the
host was healthy: display enabled, P8, no Xid, no QEMU, NBD disconnected.

**Abort point:** 711 RPCs, teardown at 457. The last RPC is `EVENT_SET_NOTIFICATION`
0x20800301 → 0x56, the arming that follows the fourth class-0x78 allocation (ff060090). In VFIO
order that arming is index 120 HOTPLUG_PROCESSING_COMPLETE at **VFIO index 2524**, as
predicted. GET and SET_RC_RECOVERY, four 0x78 allocations and the armings of 44, 43 and 113 all
returned 0. The distinct refusal set is unchanged from run26, because the refused control is the
same command with a different index. The falsifier holds: the abort moved six RPCs, not into
display setup.

**Next walls, ranked by what each needs** (in VFIO order from 2524):
1. *Needs new semantics or an owner decision:* **120 HOTPLUG_PROCESSING_COMPLETE.** No
   producer or event-data struct exists anywhere in OGKM, so kayfabe cannot post it from its
   own state as §S.2 requires. Options: accept it with a defined producer (kayfabe posts 120
   after each hotplug post it makes; the ordering would be inferred, not sourced), classify it
   as a stub, or obtain the semantics.
2. *Needs new semantics (derive from real host events):* 33 PSTATE_CHANGE, 139
   RUNLIST_PREEMPT_COMPLETE, 12 GRAPHICS, 23/24/26 CE0/CE1/CE3. One possible source: kayfabe
   arms the same notifiers on its own unprivileged host subdevice and relays them, or the
   copy/graphics non-stall completions of the VM's own host channels.
3. *Needs an owner decision:* 122 RESERVED122. It has no defined meaning, so it can be neither
   argued silent nor implemented.
4. *Then, already implemented and not yet reached:* the remaining 0x78 allocations, NV0073
   EVENT_SET_NOTIFICATION, PERF_GET_POWERSTATE, SYSTEM_GET_ACTIVE and IS_MODE_POSSIBLE.
   *Needs new semantics:* 0x007302a3, DFP_ASSIGN_SOR, DP_AUXCH_CTRL.

## What the real GSP posts: VFIO event census (no boot; 2026-10-07)

> ⊘ *Corrects the seventh repair's table (2026-10-07):* 34 HDCP_STATUS_CHANGE and 45
> AUDIO_HDCP_REQUEST are **posted** by the real GSP (see below). They are withdrawn from
> "absent on the virtual display" and refused pending the owner's decision. The indices 12, 23,
> 24, 26, 120 and 122 listed there as refused are now accepted, on the evidence below.

**Method.** `scripts/bench/windows/vfio_events.py` reads the existing gsp-observer exports of
vfio-8, vfio-9 and vfio-10. It uses abort_point.py's de-duplication and request indexing, so
the indices below are VFIO RPC indices. It decodes every GSP-initiated message, including
POST_EVENT 0x1003 (`rpc_post_event_v17_00`, `g_rpc-structures.h:1545-1556`), and every arming.
Outputs: [vfio8](vfio8-gsp-events.txt), [vfio9](vfio9-gsp-events.txt),
[vfio10](vfio10-gsp-events.txt). Limitation: the exports cover the boot up to the end of
capture (about 4,090 RPCs, no dropped records, no sequence gaps). Events after capture ends,
and events about hardware the boots never exercised, cannot appear.

**Measured (VFIO boots of 2026-10-05, RTX 4070), identical in all three unless a count is given.** The GSP-initiated messages
were GSP_INIT_DONE x1, RUN_CPU_SEQUENCER x1, UCODE_LIBOS_PRINT x2, POST_NOCAT_RECORD x3/x15/x11
(vfio-9/8/10) and POST_EVENT x15/x24/x24. Every POST_EVENT is a list post
(`bNotifyList=1`, hClient c1d00002):

| index | count (vfio-8/9/10) | first post (vfio-10 after_rpc) | data |
|---|---|---|---|
| 45 AUDIO_HDCP_REQUEST | 1/1/1 | 3119 (vfio-8: 3152, vfio-9: 3147) | eventData `00020000 00000000` (display 0x200), hEvent ff0f0000 |
| 139 RUNLIST_PREEMPT_COMPLETE | 20/11/20 | 3308, then bursts at 4043-4068 | 8-byte eventData of guest kernel addresses (`…8dd5ffff`), hEvent ff0620a0 |
| 33 PSTATE_CHANGE | 2/2/2 | 3560 and 3564 (vfio-8/9: 3556 and 3560) | data 0x20, then 0x100; 12-byte eventData whose last word equals `data` |
| 34 HDCP_STATUS_CHANGE | 1/1/1 | 4071 (vfio-8: 4078, vfio-9: 4073) | eventData `00020000 04000000` (display 0x200, value 4), hEvent ff150100 |

**Never posted in any of the three boots although armed:** 1 HOTPLUG, 2, 4, 7, 12 GRAPHICS,
23/24/26 CE0/CE1/CE3, 35, 43, 44, 113, 118 POWER_EVENT, **120 HOTPLUG_PROCESSING_COMPLETE**,
**122 RESERVED122**, 157, 158, 178, 182, 194 and 197. No hotplug was posted, so these boots
cannot show whether a processing-complete follows one. That question remains open (inferred
gap, not measured).

**Armings from 2518 to the first NV0073 display control (2567)**, all with action 2 (REPEAT)
and identical in all three boots: NV2080 at 2518 44, 2520 43, 2522 113, 2524 120, 2526 4,
2528 33, 2530 139, 2532 157, 2534 197, 2536 122, 2538 158, 2540 2, 2542 26, 2544 12, 2546 23,
2548 24, 2550 1, 2552 7, 2554 45, 2556 34, 2562 118, 2564 178, 2566 182. NV0073: 2558 event 1
(hEvent ff060070) and 2560 event 2 (hEvent ff1400f0).

**Applied (no boot):** every never-posted index in that set is accepted, with this evidence
cited per row (`RULED_NOTIFIERS`, class `NeverPostedByRealGsp`): 12, 23, 24, 26, 120 and 122
are new. 33, 139, 45 and 34 are refused. Tests 1471 pass, Clippy new 0, rustfmt clean.

**Decision items (the GSP posts these; no producer is invented):**
1. **33 PSTATE_CHANGE** (armed at 2528): two posts, data 0x20 then 0x100. These look like
   P-state transitions of the physical GPU; the interpretation is inferred. A real source
   would be host P-state events relayed from kayfabe's own unprivileged host subdevice.
2. **139 RUNLIST_PREEMPT_COMPLETE** (armed at 2530): 11-20 posts. The eventData is a guest
   kernel pointer, presumably the preempt request's own handle (inferred). The source would be
   the VM's own channels' preempt completions on the host.
3. **45 AUDIO_HDCP_REQUEST and 34 HDCP_STATUS_CHANGE** (armed at 2554 and 2556): one post each,
   both for display 0x200, a real HDMI/DP sink. On kayfabe's DVI-D virtual display these
   either stay absent (an owner call) or need HDCP and audio semantics.

**Run28 not started:** the remaining set is not fully covered by never-posted indices (33 and
139 come first, at 2528 and 2530), so condition (c) is not met.

## Eighth repair: owner rulings for 33, 34, 45 and 139; run28 setup

> ⊘ *Supersedes the census section's "decision items" (2026-10-07):* the owner ruled on all
> four (`docs/OWNER_RULINGS.md` §S). They are now silent arms (`RULED_NOTIFIERS`, class `Stub`,
> each row citing the ruling and the VFIO evidence), so every arming Windows sends between
> 2518 and 2566 is accepted.

- **33 PSTATE_CHANGE:** a no-op (the host does power management). Armed, never posted.
- **34 and 45 (HDCP):** armed and never posted. The guest sees no HDCP consistently:
  GET_CAPS_V2's capsTbl is all zero, so KSV_SRM_VALIDATION_SUPPORTED is clear
  (`ctrl0073system.h:51-55`). DFP_GET_INFO reports single-link TMDS (DVI). NV40_I2C, the DDC
  path where Bcaps would be read, is a denied class. DFP_UPDATE_DYNAMIC_DFP_CACHE, which
  carries `bHdcpCapable`, is unclaimed and refused (it appears at VFIO 2983).
- **139 RUNLIST_PREEMPT_COMPLETE:** armed silently. **Preempt-type controls in the VFIO
  sequence** (`vfio_events.py`/seqscan over vfio-10): `NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS`
  0x2080110b with `bDisable=1`, one channel and a `pRunlistPreemptEvent` guest pointer, first
  at **3307**. The GSP posts 139 with eventData equal to that pointer (after 3308). It re-enables
  at 3309-3310 and bursts at 4041-4067, between display controls (0x00731359 at 4068), so the
  bursts are not teardown frees. `GR_CTXSW_PREEMPTION_BIND` 0x20801211 first appears at 3068. No
  `NVA06C_CTRL_CMD_PREEMPT` 0xa06c0105 is sent. **No preempt control is issued before display
  setup (2558)**, so none is served in this batch; this is logged rather than implemented.

Tests 1471 pass, Clippy new 0, rustfmt clean.

**Falsifier, stated before run28:** worthwhile only if the abort reaches the NV0073 display
controls (VFIO index ≥ 2558). The run log will list every preempt-type control Windows sends
and its result.

### Run28 result: the abort reaches VFIO 2558, the first NV0073 control, and fails there

Product/QEMU c474de602a948c8b43dd328509874b3edc200a4f, `kf3-bins/c474de60`, run on 2026-10-07
at 12:07-12:10 UTC. [command](run28-command.json), [status](run28-status.json),
[trace](run28-qemu.log.gz), [requests](run28-requests.log), [completion](run28-complete.json),
[host health](run28-host-health.txt), [unit result](run28-unit-result.txt),
[9/9 gates](run28-gates.log) (11/11 USER births), [build](run28-build.log),
[watchdog recovery](run28-watchdog-recovery.log) (cleanup verified 2026-10-07). After 98 s of
uptime the NVIDIA adapter has ConfigManagerErrorCode 43 and nvidia-smi exits 9, so Code43
persists. No initialization success is claimed. Afterwards the host was healthy: display
enabled, P8, no Xid, no QEMU, NBD disconnected.

**Abort point:** 775 RPCs, teardown at 489. The last RPC is
`NV0073_CTRL_CMD_EVENT_SET_NOTIFICATION` 0x00730301 on the display-common object ff0a0000,
returning **0x40 (INVALID_STATE)**. That is VFIO index **2558**, the first NV0073 control. All
20 class-0x78 allocations and every 0x20800301 arming from 2518 to 2556 returned 0. The 0x78
event ff060070 (allocated under ff0a0000 with notify index HOTPLUG) did register as the hotplug
target. **The falsifier's threshold is reached exactly (index 2558), but the abort is in
kayfabe's own NV0073 handler, not further into display setup.**

**Cause (kayfabe's own refusal, partly inferred).** The handler returns INVALID_STATE when no
event is bound to the object. The binding is made from `NV0005_ALLOC_PARAMETERS`
`hParentClient`/`hSrcResource`, as RM's `eventInit` does (`event.c:116-120, 385`). For this
event the pair did not name (c1d00002, ff0a0000). The trace does not carry those two fields,
so which one differs is unknown (inferred). The commit after c474de60 adds a diagnostic line (the two
handles only, never `data`) for exactly this case; it has not been run yet.

**Preempt-type controls sent in run28:** none. No FIFO_DISABLE_CHANNELS, NVA06C PREEMPT,
GR_CTXSW_PREEMPTION_BIND/MODE, RESTART_RUNLIST, STOP/START_RUNLIST or RUNLIST_SET_SCHED_POLICY
occurred before teardown. PERF_GET_POWERSTATE (VFIO 2571) was not reached.

**Next walls, ranked by what each needs:**
1. *Derivable now (one diagnostic boot needs go-ahead):* the NV0073 event binding. Log the
   event's hParentClient/hSrcResource, then bind exactly as RM does.
2. *Already implemented, then reached in order:* NV0073 SYSTEM_GET_ACTIVE (2567), PERF_GET_POWERSTATE (2571).
3. *Needs new semantics:* 0x007302a3 (2572; unnamed in the 580.65.06 headers), DFP_ASSIGN_SOR
   (2575), DP_AUXCH_CTRL (2581; refuse as not-DP?).
4. *Needs owner rule plus real implementation, later (VFIO 3307 onwards):* FIFO_DISABLE_CHANNELS
   preempt, per §S.

## Ninth repair: the display-event binding uses the alloc's hParent (run29 setup)

**Cause, derived from source (no boot needed).** RM never uses the guest's `hSrcResource` for an
event: `rmapiFixupAllocParams` overwrites it with the alloc's `hParent` before the event is
constructed (`ogkm-580.65.06` and `ogkm-580.159.04`, `src/nvidia/src/kernel/rmapi/rmapi_specific.c:71`;
`ogkm-610` same line). A GSP client sends `hSrcResource = 0` anyway: `NV_RM_RPC_ALLOC_EVENT`
zero-fills `NV0005_ALLOC_PARAMETERS`, sets only `hParentClient`, `hClass`, `notifyIndex` and
`data = 0`, and allocates under `hNotifierResource` (`ogkm-580.65.06: src/nvidia/inc/kernel/vgpu/rpc.h:337-356`).
kayfabe read `hSrcResource`, found no display object `(c1d00002, 0)`, bound nothing, and refused
the enable at VFIO 2558 with INVALID_STATE. *Inferred, not measured:* the observer exports
carry no alloc params (allocs are captured as 32-byte headers), so the value Windows sent was
not seen; the source path above says it is 0, and the RPC header's parent (ff0a0000) is in the
run28 trace. The unrun diagnostic line from 73bec71a is removed; the fix makes it moot.

**Change.** `kf_rm::display` binds an accepted `NV01_EVENT_KERNEL_CALLBACK(_EX)` alloc to the
RPC header's `hParent` when that is a remembered display object, for `hParentClient` equal to 0
or the allocating client (cross-client stays unbound, as before). Only `hParentClient` is read
from the params. The batch test now sends the GSP-client shape (`hSrcResource = 0`) and a
second event (vfio-10 2559/2560, ff1400f0, notifier 2). kf-rm 622 tests pass, Clippy new 0,
rustfmt clean, ci_gates clean.

**Not added in this batch (and why).** DFP_ASSIGN_SOR (VFIO 2575): kayfabe advertises no
crossbar (`SYSTEM_GET_CAPS_V2` clear) and NVKMS skips the call in that case
(`nvkms-evo.c:5603-5606`); the physical answer for a no-crossbar GPU is not in OGKM (the
handler is GSP-only, `g_disp_objs_nvoc.c` flags 0x44), so it stays refused. `0x007302a3`
(VFIO 2572) is in no OGKM header or NVOC export table (580.65.06, 580.159.04, 595.84, 610);
the real GSP answered OK with 12 zero bytes in and out. Its semantics are not derivable, so it
stays refused; if that refusal is fatal it becomes an owner item.

**Falsifier, stated before run29:** the hypothesis "run28's abort was kayfabe's own
hSrcResource misread" is wrong if `0x00730301` on ff0a0000 still returns 0x40, or if the abort
point stays at VFIO index ≤ 2560. **Prediction:** both NV0073 enables return 0 and the abort
moves to a later display control (0x007302a3 at 2572 is the first candidate kayfabe refuses).

### Run29 result: the hypothesis holds; display setup passes and the abort moves to VFIO 2846

Product/QEMU 23dbc5b7182ecd072c8b004e8cfaadcf176d603e, `kf3-bins/23dbc5b7`, run on 2026-10-07 at
12:24-12:27 UTC. [command](run29-command.json), [status](run29-status.json),
[trace](run29-qemu.log.gz), [requests](run29-requests.log), [completion](run29-complete.json),
[host health](run29-host-health.txt), [unit result](run29-unit-result.txt),
[9/9 gates](run29-gates.log) (11/11 USER births), [build](run29-build.log),
[watchdog recovery](run29-watchdog-recovery.log) (cleanup verified). After 97 s of uptime the
NVIDIA adapter has ConfigManagerErrorCode 43 and nvidia-smi exits 9: Code43 persists. Host
afterwards: display enabled, P8, no Xid in the last 30 min, no QEMU, NBD disconnected.

**Falsifier not triggered.** Both NV0073 enables on ff0a0000 (hEvents ff060070 and ff1400f0)
returned 0. **Abort point:** 857 RPCs, teardown at 554 (was 489). The last RPC is
`NV0080_CTRL_DMA_SET_DEFAULT_VASPACE` 0x00801812 on Device c1d0001e:ff020000, refused
(unserviced), right after that Device's `FERMI_VASPACE_A` ff000850 and its
`COPY_SERVER_RESERVED_PDES`: **VFIO index 2846**, 288 indices past run28.

**Seen in run29 at 23dbc5b7 (tolerated refusals).** `0x007302a3` (VFIO 2572) was refused and tolerated.
IS_MODE_POSSIBLE was never called; instead `NV5070_CTRL_CMD_IMP_SET_GET_PARAMETER` 0x50700118
was sent 26 times, all refused (VFIO sends it once, at 2645, as a GET of IMP_ENABLE, then 156
IS_MODE_POSSIBLE). 0x00730128 and 0x0073012c (VRR_DISPLAY_INFO), the thermal/perf group 0x2080a801,
0x2081010d, 0x2080a630, and the display-path DP/HDCP/audio controls of the VFIO DP monitor were
not needed by the DVI path. No preempt-type control was sent.

## Tenth repair: SET_DEFAULT_VASPACE and the next display controls (run30 setup)

| VFIO index | RPC | kayfabe answer | source |
|---|---|---|---|
| 2846 (×11) | `NV0080_CTRL_DMA_SET_DEFAULT_VASPACE` 0x00801812 | RM's checks in order: null → 0x1f; not a VA space whose parent is this Device → 0x33; Device already has a VA space (set before, or acquired by an `index = GPU_DEVICE` VA-space alloc) → 0x33; otherwise the VA space becomes the Device's default (`kf_rm::chanlink`), params echoed (the vfio-10 reply is the request's 4 bytes). No host action: the default only decides what a later `hVASpace = 0` channel resolves to. | `device_share.c:363-400`, `dma.c:856-885`, `ctrl0080dma.h:748-778` (580.65.06) |
| 2645 | `NV5070_CTRL_CMD_IMP_SET_GET_PARAMETER` 0x50700118 | GET of IMP_ENABLE → FALSE; every other index/operation stays NOT_SUPPORTED | `ctrl5070chnc.h:934-1100`: FALSE means "all Is Mode Possible queries are answered with 'mode is possible'", which is what kayfabe's IS_MODE_POSSIBLE does. The real GPU answered TRUE (it runs IMP). |
| 2916 | `INTERNAL_DISPLAY_ACPI_SUBSYSTEM_ACTIVATED` 0x20800af0 | OK, no params, no effect | "initializes display ACPI child devices" (`ctrl2080internal.h:3519-3526`); the virtual display has none |
| 2917 | `NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_STATE` 0x0073010a | lid open; `hotplugAfterEdidMask` = every display of the device | `ctrl0073system.h:476-528`. Measured: the real GPU answered all its displays (0x7f00). Inferred: the physical side counts only its own EDID reads. |
| 2978 / 2982 | `INTERNAL_DISPLAY_PRE/POST_MODESET` 0x20800af1/2 | OK, no params, no effect | display bandwidth arbitration around a modeset (`kern_disp.c:1850-1890`): memory power management, host-owned, a §S.1 stub |

Layouts and command ids are re-derived (`tools/derive_display_layouts.sh`, 580.65.06 and
580.159.04). kf-disp + kf-rm 743 tests pass, Clippy new 0, rustfmt and ci_gates clean.

**Left refused (no derivable semantics, or owner rulings say absent):** `0x007302a3`, `0x00730128`,
`0x0073117a`, `0x00730122`, `0x007302a5`, `0x00731368` (in no OGKM header or NVOC table);
`0x00730280` GET_HDCP_STATE and `0x00730282` HDCP_CTRL (no HDCP, §S); DFP_SET_ELD_AUDIO_CAPS
(a DVI sink has no audio); the THERMAL legacy group `0x208085xx` (host-owned, §S.1, queries stay
refused); DFP_ASSIGN_SOR (no crossbar).

**Falsifier, stated before run30:** the hypothesis "the run29 abort is the unserviced
SET_DEFAULT_VASPACE" is wrong if SET_DEFAULT_VASPACE returns 0 and the abort stays at VFIO ≤ 2846,
or if SET_DEFAULT_VASPACE is refused by one of RM's checks (that would show kayfabe's VA-space
model disagrees with the guest's). **Prediction:** the new client's channels are born (VFIO
2857-2891, already served in earlier runs), and the abort moves to a later wall.

### Run30 result: SET_DEFAULT_VASPACE served; the teardown now follows a SUCCESSFUL control (VFIO 2861)

Product/QEMU 0fc7a7dba988ed392e05c7b5d913c8b60e24e0b8, `kf3-bins/0fc7a7db`, run on 2026-10-07 at
12:40-12:43 UTC. [command](run30-command.json), [status](run30-status.json),
[trace](run30-qemu.log.gz), [requests](run30-requests.log), [completion](run30-complete.json),
[host health](run30-host-health.txt), [unit result](run30-unit-result.txt),
[9/9 gates](run30-gates.log) (11/11 USER births), [build](run30-build.log),
[watchdog recovery](run30-watchdog-recovery.log) (cleanup verified). After 97 s of uptime: Code43,
nvidia-smi exit 9. Host afterwards: display enabled, P8, no Xid, no QEMU, NBD disconnected.

**Falsifier not triggered.** SET_DEFAULT_VASPACE returned 0 on c1d0001e:ff020000. IMP_SET_GET_PARAMETER
was sent once (as in VFIO) and answered IMP_ENABLE = FALSE; Windows then sent no IS_MODE_POSSIBLE.
The next client c1d00020 shares c1d0001e's VA space; its kernel CE channel ff040009 (engine 0xb) is
born Translated, its 5080 and C7B5 objects return 0, SET_CHANNEL_PROPERTIES (4000 us) is refused as
in every earlier run, and `GPFIFO_SCHEDULE` returns 0: **VFIO index 2861**, 15 indices past run29.
**Teardown starts right after that successful control** (858 RPCs, teardown at 544): for the first
time the abort does not follow a refusal. No doorbell was rung on the new channel (submissions 0).

**New evidence (run29 and run30 alike, not seen before run29).** Right after the last subdevice
armings (about VFIO 2566), Windows rings its two kernel channels for the first time, and kayfabe's
Translated rewriter refuses both first segments, killing the channels (no work runs, nothing is
forged): CE channel c1d00013:ff040000 (token 0x802, engine 0xb) with `ForeignClass { subch: 5,
class: 1 }`, and GR channel c1d00015:ff040001 (token 0x3, engine 1) with `ForeignClass { subch: 2,
class: 0xa140 }` (KEPLER_INLINE_TO_MEMORY_B). The Translated route accepts only copy-engine classes
and `GP100_UVM_SW` (`kf_chan::translated`, `tmode`).

**Timing in vfio-10 (observer QPC, unit assumed 10 MHz; relative values only).** RPCs in this
stretch arrive every 30-60 ms, but between 2861 (this GPFIFO_SCHEDULE) and 2862 the real driver spends
about 1.3 s without any RPC: it does GPU work there. In run30 the teardown starts within about
30 ms of the refused segments' channel deaths and immediately after 2861.

**Ranked hypotheses for the run30 abort (all inferred, none tested yet):**
1. *(High.)* Windows waits at that point for the work it submitted on its kernel CE and GR channels;
   kayfabe killed both channels, so the fences never complete (or the channels report an error),
   and StartDevice fails. Executing that work needs the Translated route to run Windows' kernel GR
   work (I2M at least) and whatever CE "class 1" is, on the real GPU.
2. *(Medium.)* The CE refusal `class: 1` is kayfabe misreading Windows' pushbuffer (a header form or
   subchannel convention it does not decode), which would be a kayfabe defect.
3. *(Low.)* A refused tolerated control on the new client (FIFO_GET_LATENCY_BUFFER_SIZE 0x0080170e,
   FIFO_SET_CHANNEL_PROPERTIES 0x0080170f) is checked late. Both were refused identically for
   c1d00016's channel earlier in the same boot, and that boot continued.

## DIAGNOSTIC run31: the refused kernel-channel segments, word by word

**Change (diagnostic only, bounded):** `kf_chan::ring` logs a refused segment's GP index, VA, length
and first 32 words when the rewriter refuses it. A refusal is terminal for the ring, so each channel
logs at most once. No behaviour changes. kf-chan 87 tests pass, Clippy new 0, ci_gates clean.

**Falsifier, stated before run31:** hypothesis 2 is wrong if the logged words decode, by the
class headers, to a well-formed `SET_OBJECT` naming a non-copy class (then "class 1" is Windows' own
value and the refusal is correct). Hypothesis 1 is weakened if the run31 abort happens somewhere
other than right after the paging channel's GPFIFO_SCHEDULE (VFIO 2861) while the same two
refusals recur.
