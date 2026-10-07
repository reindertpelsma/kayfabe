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

> **Correction, 2026-10-07 (after run35; [the VFIO MMIO reference](#the-vfio-mmio-reference-of-the-window-no-boot-2026-10-07)).**
> The observer export's own header gives `qpc_frequency` 1 000 000 000 (`boundary-vfio-10/gsp.jsonl`,
> line 1), so the ticks below are nanoseconds, not 10 MHz units: RPCs here arrive every 0.3-0.6 ms,
> and 2861 → 2862 takes **13.0 ms**, not 1.3 s. "It does GPU work there" is also wrong: the vfio-1
> MMIO trace of the same RPC interval shows no doorbell, BAR1 or BAR3 access at all, only the RPC
> handshake.

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

### Run31 result (DIAGNOSTIC): Windows' first kernel-channel work is real 2D/I2M GR work plus a software-subchannel bind

Product/QEMU c15c2628a50c78473290a6e202a2e5ad2bd86bc7, `kf3-bins/c15c2628`, run on 2026-10-07 at
12:52-12:55 UTC. [command](run31-command.json), [status](run31-status.json),
[trace](run31-qemu.log.gz), [requests](run31-requests.log), [completion](run31-complete.json),
[host health](run31-host-health.txt), [unit result](run31-unit-result.txt),
[9/9 gates](run31-gates.log) (11/11 USER births), [build](run31-build.log),
[watchdog recovery](run31-watchdog-recovery.log) (cleanup verified). After 97 s of uptime: Code43,
nvidia-smi exit 9. Host afterwards: display enabled, P8, no Xid, no QEMU, NBD disconnected.

**Abort point:** identical to run30: 858 RPCs, teardown at 544, right after the successful
`GPFIFO_SCHEDULE` of c1d00020's paging channel (VFIO 2861). The same two refusals recur first.

**The refused segments, decoded with the NVC56F method-header layout** (`clc56f.h`: address 11:0,
subchannel 15:13, count 28:16, sec-op 31:29; seen in run31 at c15c2628):

- *CE channel c1d00013:ff040000* (token 0x802, engine 0xb), GP 0, 7 words
  `20020017 2023b060 00000001 20018000 0000c7b5 2001a000 00000001`: SEM_ADDR_LO/HI =
  0x1_2023b060; `SET_OBJECT` subchannel 4 = 0xc7b5 (AMPERE_DMA_COPY_B); `SET_OBJECT` subchannel 5
  = 0x0001. Subchannel 5 is a software subchannel; 0x0001 is `NV01_ROOT_NON_PRIV` in the SDK
  (`cl0001.h`), not an engine class. The segment sends no method to subchannel 5.
- *GR channel c1d00015:ff040001* (token 0x3, engine 1), GP 0, 46 words, first 32:
  `20020017 2023b000 00000001 20014000 0000a140 20016000 0000902d 20016222 00000001 200160a4
  00000000 200160a7 00000000 200160ab 00000003 20016201 000000cf 20046210 00000000 00000001
  00000000 00000001 20046214 00000000 00000000 00000000 00000000 20046230 00000000 00000001
  00000000 00000001`: SEM_ADDR = 0x1_2023b000; `SET_OBJECT` subchannel 2 = 0xa140
  (KEPLER_INLINE_TO_MEMORY_B), subchannel 3 = 0x902d (FERMI_TWOD_A), then 2D-engine state
  methods on subchannel 3. This is graphics work, not a copy.

**Falsifier outcome.** Hypothesis 2 is wrong: "class 1" is Windows' own `SET_OBJECT` value, read
correctly. Hypothesis 1 is not weakened: the abort is again right after VFIO 2861, after the same
two refusals.

**Where this stops (owner decision needed).** Moving on means executing Windows' kernel-channel
work on the real GPU. The Translated route (`kf_chan::translated`, `tmode`) is built for copy-engine
kernel work only (RM's scrubber, UVM) and refuses every other class by design. Two things are
needed, and neither is covered by a ruling:
1. **Kernel GR work** (FERMI_TWOD_A and KEPLER_INLINE_TO_MEMORY_B here; 3D or compute may follow)
   from a guest *kernel* channel, run on kayfabe's unprivileged host twin in the guest-kernel mirror
   VA space. This is a new Translated tier: per-class method vocabularies, and checks that every
   operand is a virtual address in the mirror space (the 2D and I2M classes take virtual surface
   offsets).
2. **A `SET_OBJECT` of a non-engine value on a software subchannel** (5-7). Real hardware stores it
   and traps later methods to RM as software methods. Consuming the bind and refusing any later
   software method by name would match that. That hardware behaviour is inferred; no run has
   tested it.

## Eleventh repair: the kernel-GR tier and the software-subchannel rule (run32 setup)

**Owner rulings 2026-10-07** (`docs/OWNER_RULINGS.md` §S, "§S applied to guest-kernel GR work"):
kernel-GR work may run for real on an unprivileged USER host channel only; the rewriter re-authors
host methods from a per-class allowlist and never copies guest words; native validation comes
first; a software-subchannel bind to a non-class value is accepted and its later methods are
refused; GPU completion data comes from the GPU; completion interrupts are relayed host event →
guest vector.

**Change** (all default off; `KF3_KERNEL_GR_WORK=1` needs `KF3_KERNEL_GR_CE=1` and
`KF3_TSPACE=1`; `KF3_SW_SUBCH_INERT=1` is separate):
- *USER assertion.* `kf_host::Channel` now carries the birth stamp RM's reply gave
  (`born_user`, set only by the birth path, which already clears `CAP_SYS_ADMIN` for the call and
  reads `PRIVILEGED_CHANNEL` and the `internalFlags` level back). `Channel::assert_user` refuses a
  channel with no stamp or a privileged one; `HostRing::admit_gr_tier` calls it first and refuses
  the tier by name otherwise (unit test `the_gr_tier_assertion_admits_only_a_user_birth_stamp`).
- *Host objects from a fixed allowlist.* The same admission allocates one host object of
  `FERMI_TWOD_A` (0x902d) and `KEPLER_INLINE_TO_MEMORY_B` (0xa140) on the kernel-GR ring, each only
  if the host family lists it and host RM reports it supported. Nothing about it comes from guest
  bytes.
- *Re-authoring* (`kf_chan::grtables`, T-mode decoder). A `SET_OBJECT` of one of those classes on
  hardware subchannel 0-3 binds it (subchannel 4, the GR runlist's CE subchannel, and 5-7 are
  refused by name: `GrSubchannel`). Methods on a bound subchannel are admitted only by a row:
  exactly the 17 `FERMI_TWOD_A` state methods of run31's segment, each with its field rule from
  `ogkm-580.65.06: src/common/sdk/nvidia/inc/class/cl902d.h` (537-540, 555-558, 572-580, 815-832,
  868-890, 935-938, 960-970; byte-identical in 580.159.04). The emitted word is rebuilt from the
  decoded field. No I2M method is admitted (OGKM's `cla140.h` names only the class id). Privileged
  or address state (MME programming, PM trigger, instrumentation, notify, render enable,
  `SET_DST/SRC_OFFSET`, falcon methods, MME calls) and the triggers (`PIXELS_FROM_MEMORY_SRC_Y0_INT`,
  `PIXELS_FROM_CPU_DATA`) are refused under their header names (`GrMethod`); anything else as "not
  in the allowlist". A refusal kills only that channel. No admitted row carries an address.
- *Software subchannel rule.* With `KF3_SW_SUBCH_INERT`, `SET_OBJECT` on subchannel 5-7 of a value
  no family lists as a class (Windows sends 1, `NV01_ROOT_NON_PRIV`) is stored and nothing is
  emitted; a later method at or above 0x100 there is refused as `InertSubchannelMethod`.
  **Inferred, untested:** that real hardware stores the bind and traps later methods to RM.
- *GPU-written GP_GET.* On a GR-tier ring the fence tail that retires guest entry `g` starts with a
  host `SEM_EXECUTE` RELEASE (32-bit, `RELEASE_WFI`) of `g` at the T-space window address of the
  guest USERD's GP_GET word; the CPU never stores GP_GET for that channel.
- *Interrupt relay.* Read in the code before this change: a Translated ring's completion never
  raised a guest interrupt. The engine-event relay is gated on live Passthrough twins
  (`device.rs` `on_other`, `EngineEvent::live`), and Translated births never count. Measured by the
  native oracle below: the GR ring's fence NSI wakes `FIFO_EVENT_MTHD` (kf3's session completion fd)
  and not the GR0 notifier. So the relay is driven from the pump: when a GR-tier pump finds entries
  the engine retired, the worker raises the guest's GR0 vector (`kf3: NSI RELAY …` log line, first
  16 then each power of two). Nothing runs on a vCPU or under a lock a vCPU takes.
- The refused-segment log now shows up to 128 words (run31's GR segment had 46; 14 were unseen).

**Native oracle first** (`kf-gr-tier`, bare metal, no QEMU; results in the next subsection, at the
product revision). It plays Windows' kernel GR channel with run31's 32 logged words unchanged
(their `SEM_ADDR` included) plus a semaphore release, then a hostile `LOAD_MME_INSTRUCTION_RAM`;
and run31's CE segment on a CE T-mode ring with the inert rule, then a software method on
subchannel 5. It checks the USER assertion, the two host objects, engine completion (guest
semaphore and guest GP_GET written by the engine, zero CPU GP_GET stores), which host edge the
completion raises, named refusals with GP_GET unmoved, and full release. It cannot check the
guest-vector relay (no VMM): that is run32's evidence.

**Falsifier, stated before run32.** Hypothesis: the abort at VFIO 2861 follows from kayfabe killing
Windows' first kernel-channel work. It is wrong if, in run32, both channels' first segments are
executed by the engine (the GR channel retires its GP 0 with `gp_get_by_engine`, the CE channel
binds subchannel 5 inertly and retires its GP 0) and the abort still sits at VFIO ≤ 2866 (within
~5 indices of 2861). **Prediction:** the 14 unseen GR words carry methods outside the 17-method
allowlist, so the GR channel is refused by name at its first unadmitted method (now logged with up
to 128 words), the abort stays near 2861, and the run names the next 2D/I2M method set. In that
case the falsifier is not decided, and the next batch is that method set (native oracle first).
If the GR segment does retire, run32 must show `NSI RELAY` lines for GR0 and the abort moving past
2861.

### Native oracle result, before run32 (`kf-gr-tier` at 01870988)

> **Correction, 2026-10-07 (compliance audit `docs/audits/2026-10-07-code43-gr-derive-compliance.md`
> on `claude/audit-derive-20261007`, BLOCKER 1).** The subsection promised above was missing when
> run32's commits were pushed, and the oracle's log was not in git. What happened, in order: the
> oracle ran on a dirty tree (two shake-out runs, the first failing its GR0-edge check, which was
> then turned into a measurement), then at the clean product revision 01870988 by
> `loop-build.sh` at 15:32:46 local, before run32's VM started (15:34). Its log is now committed:
> [gr-tier-native-run32.log](gr-tier-native-run32.log). What it does not cover is stated below.

Bare metal, borrowed RTX 4070, host 595.91.07, no QEMU, exclusive GPU lock; Xid count 5 before
and after (all five predate this session); display active, P8 → P5 during the run.
- **Measured:** host channel born USER and `assert_user` admits it; host objects 0x902d and
  0xa140 allocated; run31's 32 words re-authored (17 GR methods) and completed by the engine:
  guest semaphore `0x6a0b0001` written by the engine, guest GP_GET = 1 written by the engine with
  zero CPU stores; the completion raised `FIFO_EVENT_MTHD` (kf3's session fd) and **not** the GR0
  notifier; a hostile `LOAD_MME_INSTRUCTION_RAM` refused by name with GP_GET unmoved; run31's CE
  segment completed on a CE T-mode ring with subchannel 5 bound inertly, and a later method on
  subchannel 5 refused as `InertSubchannelMethod`; both rings, objects, mappings and CPU views
  released.
- **Not covered:** the guest-vector relay (no VMM in the oracle), and any GR work beyond run31's
  32 words.

### Run32 result: the GR segment's last 14 words bind 3D and compute objects; abort unchanged (VFIO 2861)

Product/QEMU 01870988bc7fe1d64e00164ffd24368d3e1ac63c, `kf3-bins/01870988`, flags as run31 plus
`KF3_KERNEL_GR_WORK=1` and `KF3_SW_SUBCH_INERT=1`; run on 2026-10-07 at 13:33-13:36 UTC.
[command](run32-command.json), [status](run32-status.json), [trace](run32-qemu.log.gz),
[requests](run32-requests.log), [completion](run32-complete.json),
[host health](run32-host-health.txt), [unit result](run32-unit-result.txt),
[9/9 gates](run32-gates.log) (11/11 USER births), [build](run32-build.log),
[watchdog recovery](run32-watchdog-recovery.log) (cleanup verified, NBD disconnected). After 97 s:
the NVIDIA adapter has ConfigManagerErrorCode 43 and nvidia-smi exits 9. Host afterwards: no
QEMU, no NBD attached, display enabled, P8, Xid count unchanged (5).

**Measured (seen in run32 at 01870988):**
- Both kernel-GR channels were admitted to the tier (`kf-chan: GR tier admitted … privilege=USER
  objects=[(902d, …), (a140, …)]`), and every Translated birth logged `sw_subch_inert=true`.
- **CE channel c1d00013:ff040000** (token 0x802): the subchannel-5 bind was accepted inertly
  (`inert_binds=1`), and the segment ran on the engine: `forwarded=1 submissions=1 gp_get=Some(1)`.
  This is the first time this channel's work completed.
- **GR channel c1d00015:ff040001** (token 0x3): the 17 2D methods were re-authored, then the
  segment was refused at word 35, `ForeignClass { subch: 0, class: 0xc997 }`. All 46 words are
  now logged. The 14 unseen words are: `SET_OBJECT` on subchannel 4 = 0xc7b5 (CE), on 0 = 0xc997
  (ADA_A, 3D), `SET_NOTIFY_A/B` on 0 = (1, 0x2023b3a0), `SET_OBJECT` on 1 = 0xc9c0
  (ADA_COMPUTE_A), `SET_NOTIFY_A/B` on 1 = (1, 0x2023b3a0), `SET_OBJECT` on 5 = 1. The segment
  releases no semaphore: its completion is GP_GET alone.
- No `NSI RELAY` line (no GR-tier work retired). **Abort point unchanged:** 858 RPCs, teardown at
  544, last RPC `GPFIFO_SCHEDULE` 0xa06f0103 returning 0 — identical to runs 30-31 (VFIO 2861).

**Falsifier outcome: not decided.** As predicted, the GR channel was refused at its first
unadmitted item (a 3D `SET_OBJECT`, not a 2D method), so its first work did not run; the CE
channel's did. The abort did not move, which only says that executing the CE channel's segment
alone is not enough.

## Twelfth repair: 3D and compute notifier addresses on the GR tier, audit fixes (run33 setup)

**Audit fixes first** (BLOCKER 2): the tier no longer types a class id. `GrClass::of_class` takes
every kind (2D, inline-to-memory, 3D, compute) from kf-chip's generated per-family sets, and a test
pins that every family's 2D class is the one `cl902d.h` defines (generated `class_ids:FERMI_TWOD_A`).
`admit_gr_tier` admits each kind on its own from the host family's set (Blackwell's
inline-to-memory class 0xcd40 included) and refuses a kind the host lacks loudly, without failing
the others. The 2D and inline-to-memory method allowlists are unchanged.

**Batch 2 (exactly run32's 14 new words).** The family 3D class gets a host object too (the
compute object is the ring's GR-context object). On a 3D or compute subchannel only
`SET_NOTIFY_A` (`ADDRESS_UPPER` 7:0) and `SET_NOTIFY_B` (`ADDRESS_LOWER` 31:0, 4-byte aligned) are
admitted (`ogkm-580.65.06: clc997.h:41-45`; compute from `clc7c0.h:41-45`, because OGKM's
`clc9c0.h` names only the class id at line 27 — that ADA_COMPUTE_A keeps the layout is
**inferred**). The pair is an address: the decoder emits `Ir::GrAddress` with the guest VA, the
binder resolves it through the channel's placement rows (one writable row, 16 bytes, the size of
one `NvNotification`) into a T-space window address, and only the address perimeter
(`tspace_unsafe::put_gr_address`) emits the two words. An address no row covers is refused by
name (`VirtualUnresolved`). `NOTIFY` (0x10c), the trigger that would write there, stays refused,
as do MME programming, `PM_TRIGGER` and MME calls. Inferred, not tested: a later remap of that VA
leaves the engine's notifier address on the old backing, which is still inside this VM's windows.

**Open questions for the owner (audit SHOULD-FIX 4 and 5, not acted on).**
- Notifiers 12, 23, 24 and 26 are accepted silently on the strength of a three-boot capture on one
  RTX 4070. In OGKM the guest's own interrupt handlers raise them (`kernel_ce.c:702`,
  `kernel_graphics.c:2631`), and Translated CE rings have no relay that would deliver them, so
  they are never delivered. That needs a ruling against §S.3.
- `PERF_GET_POWERSTATE` answers "AC" (`kf_rm::vfguest`, line 174) without a ruling.
- The capability allowlist change in `kf-abi/src/capability.rs:1531-1543` (admits `NV01_EVENT_KERNEL_CALLBACK` for every guest) needs owner review
  before any merge.
- Still to do in code (SHOULD-FIX 3 and 5): key `RULED_NOTIFIERS` by driver version from the
  generated tables, and take `vfguest.rs`'s status offset and control ids/sizes from the generated
  tables.

**Native oracle at 9b178991, before run33** ([log](gr-tier-native-run33.log); 9/9 gates, 11/11
USER births). Measured: objects of the derived classes 0x902d, 0xa140, 0xc997 and the context's
0xc9c0 on a USER channel; run32's whole 46-word segment unchanged plus a release: 21 GR methods
re-authored (both notifier addresses through the windows), subchannel 5 inert, completed by the
engine (guest semaphore and guest GP_GET = 2 written by the engine, zero CPU stores); completion
on `FIFO_EVENT_MTHD`, not GR0; a 3D notifier address no row covers refused at bind
(`VirtualUnresolved`) with GP_GET unmoved; the CE inert arm as before; everything released. Xid
count unchanged (5); display active. Not covered: the guest-vector relay.

**Falsifier, stated before run33.** Hypothesis: the abort at VFIO 2861 follows from Windows' first
kernel GR work not completing. It is wrong if run33's GR channel retires GP 0
(`gp_get_by_engine=(true, n≥1)` on token 0x3, an `NSI RELAY` line) and the abort still sits at
VFIO ≤ 2866. **Prediction:** the GR channel retires its first segment, the guest's GR0 vector is
raised (`NSI RELAY`), and the abort moves past 2861 — or the GR channel is refused by name at a
later segment, naming the next method set.

### Run33 result: both kernel channels' first work completes on the engine; the abort does not move (falsifier triggered)

Product/QEMU 9b1789912b6ea7a7d8fa722f4a85125a44eee4e1, `kf3-bins/9b178991`, flags as run32; run on
2026-10-07 at 13:52-13:55 UTC. [command](run33-command.json), [status](run33-status.json),
[trace](run33-qemu.log.gz), [requests](run33-requests.log), [completion](run33-complete.json),
[host health](run33-host-health.txt), [unit result](run33-unit-result.txt),
[9/9 gates](run33-gates.log) (11/11 USER births), [build](run33-build.log),
[native oracle](gr-tier-native-run33.log), [watchdog recovery](run33-watchdog-recovery.log)
(cleanup verified). After 97 s: Code43, nvidia-smi exit 9. Host afterwards: no QEMU, no NBD
attached, display enabled, P8, Xid count unchanged (5).

**Measured (seen in run33 at 9b178991):**
- GR channel c1d00015:ff040001 (token 0x3) on a USER host channel with objects 0x902d, 0xa140,
  0xc997 and 0xc9c0: its whole 46-word segment was re-authored (`gr[methods=21 inert_binds=1]`)
  and retired by the engine: `forwarded=1 submissions=1 gp_get=Some(1)
  gp_get_by_engine=(true, 1)`. No refusal.
- The completion was relayed: `kf3: NSI RELAY host non-stall (FIFO_EVENT_MTHD) -> guest GR0 vector
  0 … relay #1`. The status line shows `nsi=[GR0:290/1raised …]` and `irq[… raised=2 held=0]`, so
  the MSI was signalled, not held by a cleared enable. Whether the guest's handler consumed it is
  not observable here.
- CE channel c1d00013:ff040000 (token 0x802): inert subchannel-5 bind, retired as in run32.
- **Abort point unchanged:** 858 RPCs, teardown at 544, last RPC `GPFIFO_SCHEDULE` of
  c1d00020's paging channel ff040009, returning 0 (VFIO 2861). The first `Free` follows it within
  about 3 ms (`mem t=8.33s` → `8.34s`).

**Falsifier outcome: triggered.** Windows' first kernel GR and CE work both completed on the
engine, with engine-written GP_GET and a relayed GR0 interrupt, and the abort stayed at VFIO
2861. The hypothesis "the abort follows from kayfabe killing that work" is wrong as the sole cause.

**Three consecutive runs (31, 32, 33) end at VFIO 2861.** Per the loop's stop rule this iteration
stops here. Ranked hypotheses (none tested):
1. *(High, in the RPC stream.)* The paging channel's own controls. Right before 2861, client
   c1d00020 sends `0x0080170e` (FIFO latency-buffer size query) and `0x0080170f`
   (SET_CHANNEL_PROPERTIES, property 0 = timeslice 4000 µs for ff040009); kayfabe leaves both
   UNSERVICED. In vfio-10 both return 0 at indices 2855 and 2860 for the same client. Windows may
   check those results only after scheduling, and the teardown starts ~3 ms after
   `GPFIFO_SCHEDULE`, too soon for a timeout. The same refusals were tolerated for c1d00016's
   channel earlier, but that is a different channel role. Next step under §S.2 (timeslice is
   "implement for real"): apply the timeslice as a real unprivileged host control on the twin's
   group (run19 did this for `a06c0103`) and answer the latency-buffer query from a host fact.
2. *(Medium, outside the RPC stream.)* Interrupt delivery for Translated CE work. Translated CE
   rings still raise no guest interrupt (only the GR tier relays), and the guest's CE non-stall
   notifiers 12/23/24/26 are never delivered (audit SHOULD-FIX 4). A WDDM paging fence waiting on
   a CE interrupt would fail, but ~3 ms is short for a wait.
3. *(Medium-low, outside the RPC stream.)* A BAR0/BAR1 access right after scheduling: the paging
   channel's USERD read through BAR1, the usermode doorbell, or a PTIMER read. No MMIO trace is
   taken on kayfabe runs, so this is unmeasured. An MMIO/doorbell trace of the 3 ms window would
   decide it.
4. *(Low.)* The relayed GR0 interrupt itself (vector 0, raised once) confuses the guest's ISR.

## Thirteenth repair: the paging client's FIFO controls, answered for real (run34 setup)

Branch `claude/code43-lat-20261007`, from `d7d4bbb5`. Hypothesis 1 of run33's list.

**What the reference does (measured in vfio-10, the 2026-10-05 VFIO boot on the RTX 4070, decoded
with the 580 control header: params at body offset 40).** Every new Windows kernel client asks `NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE`
(`0x0080170e`) once, and some then send `NV0080_CTRL_CMD_FIFO_SET_CHANNEL_PROPERTIES` (`0x0080170f`)
for the channel they just allocated. The real GSP answers all of them with 0:

| VFIO | client | request | reply |
|---|---|---|---|
| 2363, 2416, 2445, 2855, 2883 | c1d00013/16/19/21/24 | `engineID 0xb` (COPY2) | `gpEntries 0x20, pbEntries 0xe00` |
| 2382, 2897, 2937, … | c1d00015/25/28 | `engineID 0x1` (GR0) | `0x240, 0x2880` |
| 2433, 2459, 2471, 2483, 2496, 2508 | c1d00018/1a/1b/1c/1d/1e | `0x13`, `0x26`, `0x1c`, `0x1c`, `0x33`, `0x1c` | `0x20, 0x80` each |
| 3076, 3218, … | c1d0002c/30 | `0xc` (COPY3) | `0x20, 0xe00` |
| 2421, 2489, **2860**, 2913 | c1d00016/1c/21/25 | `hChannel` ff040002/07/0a/0c, property 0 (`ENGINETIMESLICEINMICROSECONDS`), value 4000/4000/4000/1000 | the request echoed |

kayfabe left every one unserviced (refused). The refusals before 2855 were tolerated; the paging
client's pair (VFIO 2855 and 2860; kayfabe's c1d00020) is the last pair before the abort.

**OGKM (580.159.04, identical declarations in 580.65.06 `ctrl0080fifo.h:189-307`).** Both controls
are `ROUTE_TO_PHYSICAL` and `NON_PRIVILEGED` (`g_device_nvoc.c:640-669`, flags `0x50048` and
`0x10248`), so a GSP client has no body for them and a host USER client may issue them. The only
open body is the vGPU guest's `deviceCtrlCmdFifoGetLatencyBufferSize_VF`
(`src/nvidia/src/kernel/gpu/fifo/kernel_fifo_ctrl.c:983-1008`): a table lookup by `engineID`,
`NV_ERR_INVALID_ARGUMENT` for an engine the table lacks. `SET_CHANNEL_PROPERTIES` has no open body
(`deviceCtrlCmdFifoSetChannelProperties_IMPL` is only declared, `g_device_nvoc.h:1105`).

**Change.**
- *Generated, not typed.* The driver matrix now measures both command ids, both params structs,
  the property value `ENGINETIMESLICEINMICROSECONDS`, and (for the audit items below)
  `SET/GET_RC_RECOVERY`, `RC_RECOVERY_DISABLED/ENABLED`, `PERF_GET_POWERSTATE`,
  `PERF_POWER_SOURCE_AC` and `DMA_SET_DEFAULT_VASPACE` with their params (`tools/drivermatrix`:
  `consumed.txt`, a `ctrl_values` line in `sdk.spec`; full 30-tag `regen.sh`). `kf_abi::fifoctl`
  reads ids and layouts at the guest's version; `kf_abi::hostabi` lists the latency control for the
  host carry.
- *`0x0080170e` from a host fact.* At realize kayfabe asks the host's own
  `FIFO_GET_LATENCY_BUFFER_SIZE` on its host Device for every engine it advertises (the host's
  `GET_ENGINES_V2` types; NON_PRIVILEGED; the host's GSP answers). `kf_rm::inittables` answers the
  guest with the host's row for the guest's `engineID` (the `_VF` body's lookup) and
  `NV_ERR_INVALID_ARGUMENT` for an engine without a host row. The realize log line
  `kf3: host facts: FIFO latency buffers` shows the host rows.
- *`0x0080170f` as a real timeslice.* `kf_rm::chanlink` carries property 0 only, as
  `ChanStatement::ChannelTimeslice`; `kf-qemu` resolves `hChannel` in the caller's client to its
  owned host channel (passthrough twin or Translated ring) and applies the host's
  `NVA06C_CTRL_CMD_SET_TIMESLICE` to that channel's own host group on the act thread (the run19
  verb; unprivileged, nothing taken from guest bytes but the value, which host RM bounds). The reply
  (the request echoed, as vfio-10) is held until that host act returns. The capability gate asked is
  the host verb's (`a06c0103`); **no allowlist entry was added**. Other properties stay unserviced.

**Falsifier, stated before run34.** Hypothesis 1: the abort right after the paging channel's
`GPFIFO_SCHEDULE` (VFIO 2861) follows from the unanswered `0x0080170e`/`0x0080170f` of that client.
It is wrong if, in run34, kayfabe's `0x0080170e` for c1d00020 returns 0 with the host's row and its
`0x0080170f` returns 0 after a logged host `SET_TIMESLICE` on that channel's group, and the abort
still sits at VFIO ≤ 2866 (teardown within ~5 RPCs of `GPFIFO_SCHEDULE`). **Prediction:** the
abort moves past 2861 (vfio-10's next RPCs are subdevice perf controls `0x2080852e/30/2a` at
2862-2864). The result will keep what is seen apart from what is inferred.

### Run34 result: both controls served from the host; the abort does not move (falsifier triggered)

Product/QEMU 0c74f4fc952fedf97aa2366ab189d1e482024d0f, `kf3-bins/0c74f4fc`, flags as run33; run on
2026-10-07 at 14:28-14:31 UTC. [command](run34-command.json), [status](run34-status.json),
[trace](run34-qemu.log.gz), [requests](run34-requests.log), [completion](run34-complete.json),
[host health](run34-host-health.txt), [host after](run34-host-after.txt),
[unit result](run34-unit-result.txt), [9/9 gates](run34-gates.log) (11/11 USER births),
[build](run34-build.log), [native oracle](gr-tier-native-run34.log) (GR tier unchanged, PASS, Xid
count 5 before and after), [watchdog recovery](run34-watchdog-recovery.log) (cleanup verified).
After 99 s: the NVIDIA adapter has ConfigManagerErrorCode 43 and nvidia-smi exits 9. Host
afterwards: no QEMU, NBD disconnected, display enabled, P8, Xid count unchanged (5).

**Measured (seen in run34 at 0c74f4fc):**
- At realize the host answered `FIFO_GET_LATENCY_BUFFER_SIZE` for all nine advertised engines:
  `(engine, gp, pb)` = `(1, 0x240, 0x2880)`, `(9, 0x240, 0x2880)`, `(0xa, 0x240, 0x2000)`,
  `(0xb, 0x20, 0xe00)`, `(0xc, 0x20, 0xe00)`, `(0x13, 0x20, 0x80)`, `(0x1c, 0x20, 0x80)`,
  `(0x22, 0x20, 0x80)`, `(0x33, 0x20, 0x80)`. Where vfio-10 asked the same engine the values are
  identical (0xb, 0xc, 0x1, 0x13, 0x1c, 0x33).
- All ten guest `0x0080170e` requests returned 0 with the host's row, the paging client's
  included (c1d00020, engine 0xb: `gp 0x20, pb 0xe00`, the vfio-10 2855 reply). None was refused.
- All three guest `0x0080170f` requests (property 0, 4000 µs; c1d00016/ff040002, c1d0001b/ff040006,
  c1d00020/ff040009) returned 0 after the host `SET_TIMESLICE` on the channel's own host group
  (`act channel timeslice … on host group 0xcafe0060 (twin 0x10038) (222 us, off the GSP lock)`
  for the paging channel), reply held until then.
- Kernel CE (0x802) and GR (0x3) channels retired their first work as in run33; one `NSI RELAY` to
  GR0.
- **Abort point unchanged:** 858 RPCs, teardown at 544, last RPC `GPFIFO_SCHEDULE` of
  c1d00020/ff040009 returning 0 (VFIO 2861). The paging channel was never rung (`submissions=0
  last_put=Some(0)`) and is freed by the third `Free` of the teardown.

**Falsifier outcome: triggered.** Both controls of the paging client now return 0 with the
reference's reply shape (the latency row byte-identical to vfio-10's; the timeslice a real host act),
and the abort stays at VFIO 2861. Hypothesis 1 is wrong as the cause. *Inferred, not tested:* the
earlier refusals of the same controls were tolerated for the same reason — Windows does not check
them at this step.

## Fourteenth repair: Translated copy-engine completions relayed to the guest's CE vector (run35 setup)

Hypothesis 2 of run33's list, next because run34 falsified hypothesis 1.

**What was missing (read in the code at 893fac67).** Only GR-tier rings relayed a completion to a
guest vector (`ChanPlane::take_gr_relay`). A Translated copy-engine ring — Windows' kernel CE
channel (token 0x802, guest COPY2) and the paging channel — authors the guest's `GP_GET` after its
host fence is reached, and the guest never got an interrupt for it. Passthrough twins have their own
relay (`EngineEvent::live`), which Translated births never count.

**Change** (default off, `KF3_TRANSLATED_CE_RELAY=1`; the Windows runner's flag list gains it):
- When a Translated ring on a guest copy engine (`is_copy_engine(guest_engine)`, not GR tier) is
  pumped and its `GP_GET` advanced — which the pump does only for entries whose HOST fence was
  reached (no forged completion) — the plane counts a pending relay on the `EngineEvent` of the
  ring's own guest engine. The worker, right after `serve`, raises that engine's guest vector
  (`latch_and_deliver`) and logs `kf3: NSI RELAY … -> guest CEn vector v: a Translated copy-engine
  ring retired work after its host fence (relay #n)` (the first 16, then each power of two). The
  same path as the GR0 relay: on the worker, never on a vCPU, no lock a vCPU takes is held.
- *Wake path (read in the code):* the worker passes until no token is served, then parks in
  `epoll` (50 ms re-check) on its eventfd and the session fd, which carries `FIFO_EVENT_MTHD` and
  the host CE's own non-stall notifier (`ChanPlane::new`, `Completions::also`). A completion
  re-rings the in-flight channels' tokens, the pump sees the fence, and the relay follows.
- *Native oracle extended* (`kf-gr-tier`): on the CE T-mode arm it now measures, each on its own fd,
  whether the CE ring's completion raises `FIFO_EVENT_MTHD` and/or the host CE notifier, and checks
  that at least one of the fds kf3's worker parks on fires (`ce_completion_wakes_worker_fds`). The
  guest-vector relay itself needs the VMM; that is run35's evidence.

**Also in this revision (audit SHOULD-FIX 3 and 5; no answer changes, pinned by tests).**
`RULED_NOTIFIERS` rows name the generated `nv2080_notifiers` runs and are resolved at the guest's
version (`is_ruled_notifier(version, index)`; at 535.309.01 `AUX_POWER_STATE_CHANGE` is 0xb4 and
`GPU_RC_RESET` is absent). `kf_rm::vfguest` takes the RC-recovery and power-state ids, params sizes
and values from the driver matrix and the status offset from `rm_control_wire().status_off`
(`VfGuestIds`); `kf_rm::chanlink` takes `SET_DEFAULT_VASPACE`'s id and size from the matrix
(`DefaultVaspaceCtl`). `PERF_GET_POWERSTATE`'s `AC` is recorded in `OWNER_RULINGS.md` §S as a stub
*assumed* from the power ruling, owner to confirm. The capability allowlist change in
`kf-abi/src/capability.rs:1531-1543` (`NV01_EVENT_KERNEL_CALLBACK` for every guest) is untouched
and **still needs owner review before any merge**.

**Falsifier, stated before run35.** Hypothesis 2: the abort right after the paging channel's
`GPFIFO_SCHEDULE` follows from the guest never receiving an interrupt for its Translated CE work.
It is wrong if, in run35, an `NSI RELAY … guest CE… vector` line is logged for the kernel CE
channel's retired work (token 0x802 retires GP 0, as in runs 32-34), the MSI is raised (not held),
and the abort still sits at VFIO ≤ 2866. If no CE relay is logged (for example, the guest engine has
no vector in the served table), the falsifier is undecided and the run says why. **Prediction:** a
CE relay is logged and the abort moves past 2861.

### Run35 result: CE completions reach the guest's CE2 vector; the abort does not move (falsifier triggered)

Product/QEMU d1b6cfdde5f6027decfb81c7a9b71ea3f91d3a4e, `kf3-bins/d1b6cfdd`, flags as run34 plus
`KF3_TRANSLATED_CE_RELAY=1`; run on 2026-10-07 at 14:41-14:44 UTC. [command](run35-command.json),
[status](run35-status.json), [trace](run35-qemu.log.gz), [requests](run35-requests.log),
[completion](run35-complete.json), [host health](run35-host-health.txt),
[host after](run35-host-after.txt), [unit result](run35-unit-result.txt),
[9/9 gates](run35-gates.log) (11/11 USER births), [build](run35-build.log),
[native oracle](gr-tier-native-run35.log), [watchdog recovery](run35-watchdog-recovery.log)
(cleanup verified). After 98 s: ConfigManagerErrorCode 43, nvidia-smi exit 9. Host afterwards: no
QEMU, NBD disconnected, display enabled, P8, Xid count unchanged (5).

**Native oracle at d1b6cfdd, before run35 (seen there):** the CE T-mode ring's completion raises
`FIFO_EVENT_MTHD` and not the host CE2 notifier (`ce_completion_edges`); the check that one of the
worker's fds fires passes; GR tier PASS as before; Xid count 5 before and after.

**Measured (seen in run35 at d1b6cfdd):**
- Three CE relays to the guest's CE2 vector (vector 1): #1 and #2 for RM's internal scrubber ring
  (c1e00008:0x2, token 0x801) at boot, #3 for Windows' kernel CE channel (token 0x802) right after
  the last subdevice armings, next to the GR0 relay. Status line `nsi=[GR0:291/1raised
  CE2:3/3raised]`, `irq[… raised=6 held=0]`: every MSI was signalled, none held.
- 0x0080170e/0x0080170f served as in run34.
- **Abort point unchanged:** 858 RPCs, teardown at 544, last RPC `GPFIFO_SCHEDULE` of
  c1d00020/ff040009 returning 0 (VFIO 2861); the paging channel never rung; the third `Free` of the
  teardown frees it.

**Falsifier outcome: triggered.** The kernel CE work's completion now reaches the guest as a CE2
interrupt (after the host fence, on the worker) and the abort stays at VFIO 2861. Hypothesis 2 is
wrong as the cause. Whether the guest's ISR consumed the interrupts is not observable here.

## Stop: two runs after A and B moved the abort by 0 indices (2026-10-07)

Runs 34 and 35 each removed one candidate cause and the abort stayed at VFIO 2861 both times
(runs 30-35: six runs at the same point). Per the stop rule the loop stops here. Everything kayfabe
answers in the RPC stream up to 2861 now matches vfio-10's status; the teardown begins about 3 ms
after a SUCCESSFUL `GPFIFO_SCHEDULE`, with no RPC, doorbell or submission on the new channel in
between. So the deciding event is outside the RPC stream.

**What the teardown order says.** Seen in runs 33-35: the third `Free` of the teardown frees the
paging channel ff040009 (its `TSPACE-RETIRE` follows it). Inferred (the `Free` trace carries no
handles): the first two are its only children, the 0x5080 and 0xc7b5 objects — Windows abandons
the paging channel it has just scheduled. In vfio-10 the same point is followed
by ~1.3 s of GPU work with no RPC (run30's timing note).

**Ranked hypotheses outside the RPC stream (all inferred, none tested):**
1. *(Medium-high.)* A BAR0 read by Windows' KMD in that ~3 ms (after scheduling, before first use
   of the paging channel) returns a value it rejects: a PTIMER, `NV_PMC`/boot, usermode, or FIFO/
   runlist/CHRAM status register that kayfabe's BAR0 model answers from its shadow (BAR0 reads do
   not trap, so no kayfabe log can show it).
2. *(Medium.)* Guest-visible state of the new channel read by the CPU: its USERD (sysmem, 512 bytes;
   `GP_GET`/`GP_PUT`/reference words) or its error notifier, which a Translated birth leaves as the
   guest wrote it and which the real GSP may initialise at schedule time.
3. *(Medium-low.)* A wait on earlier kernel-channel work that completed with a different value: the
   CE (0x802) and GR (0x3) first segments complete on the engine with GP_GET only (no semaphore
   release in the CE segment); Windows may instead check a semaphore or timestamp value written by
   later entries it expected to have run.
4. *(Low.)* The relayed interrupts themselves (GR0 once, CE2 three times) are read by the guest's
   ISR through interrupt-leaf registers whose state kayfabe models differently from the GSP.

**Proposed ONE bounded MMIO trace run (needs a go-ahead; diagnostic only, default off).** A kf3
diagnostic that, from the `GPFIFO_SCHEDULE` of the last kernel client's channel until the first
following `Free` (or 4096 accesses, whichever is first), makes BAR0 reads trap and logs every BAR0
read and write (offset, width, value returned or written) plus any usermode-doorbell write. Bounded
in time and count, no answer changes, nothing forwarded. Its output names hypothesis 1's register
(or rules hypothesis 1 out); a USERD/error-notifier dump at the same `GPFIFO_SCHEDULE` (64 words,
logged once) would cover hypothesis 2 in the same run. The equivalent VFIO-arm trace (the audited
runner's MMIO mode, `x-no-mmap`) would give the reference window if the kayfabe trace is not
conclusive.

## The VFIO MMIO reference of the window (no boot, 2026-10-07)

Branch `claude/code43-trace-20261007`, from `1b505412`. Step 1 of the owner's plan of 2026-10-07:
before spending a boot, look for a VFIO BAR0 trace of the same window.

**What exists (measured on 172.22.1.20, `/var/lib/kf-windows-20261005/`).** vfio-8/9/10 carry the
GSP observer and no MMIO trace (the runner refuses the two together). vfio-1 and vfio-2 (2026-10-05,
`x-no-mmap=on`, QEMU `-trace vfio_region_read/write`) carry an MMIO trace; vfio-2 traced writes
only, vfio-1 traced reads too. The runner killed both at its 256 MiB trace bound, but only after the
window: vfio-1 holds **2932** writes of the GSP command-queue head (BAR0 `0x110c00`, one per RPC
request). No new VFIO boot was needed.

**Alignment (measured in vfio-1 and vfio-10, both booted 2026-10-05 on the RTX 4070; the tool is [`mmio_window.py`](../../scripts/bench/windows/mmio_window.py)).**
Head ordinal N (1-based) in vfio-1 is RPC index N of vfio-10's observer (`abort_point.py`'s
indexing). In vfio-10, `0x0070` allocations at 2848, 2876 and 2930 are each followed by a
`0x9096` allocation. In vfio-1, the only three ~16 400-line BAR3 bursts of heads 2840-2932 fall in
exactly the intervals after heads 2848, 2876 and 2930: page-table writes through BAR3 plus four MMU
invalidates each. The spacing (28, 54) is irregular, so a shifted alignment would not match it.
Per-interval summary of heads 2300-2932: [vfio1-mmio-2300-2932-summary.txt](vfio1-mmio-2300-2932-summary.txt);
every non-display BAR0 register read or written there, with up to 8 values:
[vfio1-mmio-2300-2932-registers.txt](vfio1-mmio-2300-2932-registers.txt).

**The window (measured in vfio-1, 2026-10-05, RTX 4070, between head 2861 — `GPFIFO_SCHEDULE` of c1d00021's paging channel
ff04000a — and head 2862).** 356 accesses, all of them the RPC handshake: 175 reads of `0x110094`,
174 reads of `0xb81010`, and 7 interrupt-tree writes (`0xb81208/210/408/410/608/610`). There is no
other BAR0 read, no doorbell (`0xbb0090`), no PTIMER read (`0x9400`, `0xbb0080`), and no BAR1 or BAR3
access at all. The next RPCs (2862-2864, perf controls on c1d00002) follow 13.0 ms later in vfio-10.
In the interval before the schedule (after head 2860, `SET_CHANNEL_PROPERTIES`), Windows clears one
4 KiB page through BAR3, writes two PTE words and issues one MMU invalidate. In the interval before
that (after head 2859, the `0xc7b5` alloc), it writes 410 bytes through BAR3 with 6 invalidates.

**The kernel-channel doorbells (measured in vfio-1, 2026-10-05).** In heads 2300-2932 the usermode doorbell (`0xbb0090`)
is written twice, both after head 2567 (values `0x3`/`0xe`/`0x10002`/`0x1000d` across the run): the
two kernel channels' first work, which matches kayfabe's first rings near VFIO 2566.

**What this says about hypothesis 1 (inferred).** In the reference, Windows reads no BAR0 register
in the window except the RPC handshake. For a BAR0 read to be the cause on kayfabe, it must be either
a read the reference never makes (an error path taken earlier), or a handshake read (`0x110094`,
`0xb81010`, the interrupt leaves) whose kayfabe value differs. Only a kayfabe trace of the same window
can tell. Hypothesis 1 drops from medium-high to medium; hypotheses 2 and 3 (guest memory that the
CPU reads, which no MMIO trace shows) rise relatively.

**Also seen (measured, not pursued yet).** vfio-10 has one more kernel client than kayfabe in this
stretch: c1d0001a with a channel on engine `0x26` (`NV2080_ENGINE_TYPE_SEC2`, OGKM 580.65.06
`cl2080_notification.h:323`), scheduled at VFIO 2465. kayfabe advertises no SEC2 engine (the host's
`GET_ENGINES_V2` list in run34's realize log has no 0x26), so the guest never creates that client,
and kayfabe's client numbers from there on are one lower than vfio-10's (kayfabe c1d00020 is vfio
c1d00021).

## DIAGNOSTIC run36: the approved BAR0 trace of the post-schedule windows (setup)

**Owner approval, 2026-10-07** (`docs/OWNER_RULINGS.md` §S, "§S exception: a diagnostic BAR0-read
trap"): default off, bounded at 4096 accesses, only from a guest-kernel channel's `GPFIFO_SCHEDULE`
to the first `Free`, logging every BAR0 read and write plus the doorbell writes, and a one-time dump
of 64 words of the new channel's USERD and its error notifier.

**Change (diagnostic only; `KF3_BAR0_TRACE=1`, the runner's flag list gains it).**
- `crates/kf-qemu/src/bar0trace.rs`: a window is armed when the served chain answers a Translated
  guest-kernel channel's `GPFIFO_SCHEDULE` (enable). It opens on the drainer right after the reply
  is published, and closes on the vCPU at the guest's next queue-head write (the next RPC), or at the
  cap. While a window is open, every BAR0 read (with the value served) and every BAR0 write (the
  usermode doorbell included) is recorded in a preallocated lock-free ring. The drainer prints the
  window, with the function of the RPC that closed it, as `kf3: BAR0-TRACE …` lines; consecutive
  identical records are collapsed with a count. When that RPC is the first `Free`, it also dumps 64
  USERD words and the 4-word error notifier of the window's channel once (`kf3: BAR0-TRACE dump`).
  The cap is 4096 records per run; once it is reached, no window opens again.
- `qemu/hw/misc/kf3/kf3.c` (KF3 ABI 21, `kf3_set_read_trap`): a main-loop bottom half turns ROMD
  off on the shadow pieces (their reads then exit to `kf3_bar0_read`) and back on. The usermode and
  timer passthrough pages and PRAMIN are never trapped. `Device::bar0_read` answers a non-hole read
  from the same shadow bytes ROMD would have shown, so no answer changes.
- With the flag off, reads never exit: the only addition on the vCPU path is one atomic load per
  trapped write. kf-qemu tests 106 pass (5 new in `bar0trace`), Clippy new 0. The unsafe ratchet
  moves 61 → 67 (the read-trap verb's FFI, itemised in `.github/workflows/ci.yml`).

**Every kernel-channel schedule opens a window**, about ten per boot. The earlier windows end at
their next RPC, and Windows continued after each of them, so they are the accepted baseline for the
paging channel's window, which is closed by the first `Free`.

**Falsifiers, stated before run36.**
- *Hypothesis 1 (a BAR0 read in the window returns a value Windows rejects).* Wrong if the window
  closed by the first `Free` contains no BAR0 read except registers that the vfio-1 window also reads
  (`0x110094`, `0xb81010`) or that the accepted windows of the same boot also read with the same
  values. Supported if it contains a read that neither has. That read is then the candidate, to be
  compared with OGKM and the VFIO value.
- *Hypothesis 2 (the new channel's USERD or error notifier holds a state Windows rejects).* The dump
  (taken in run36, 2026-10-07) is a measurement, not a test. Hypothesis 2 is weakened if USERD shows `GP_PUT = GP_GET = 0` with
  nothing else written and the error notifier's `status` is 0. Before any change it would have to be
  compared with what OGKM's GSP-RM writes at allocation or schedule time.
- *Prediction:* the abort stays at VFIO 2861 (the run changes no answer). If it moves, the trap's
  timing changed the guest's behaviour, and that is recorded as the result.

### Run36 result (DIAGNOSTIC): the paging channel's window holds no BAR0 read; USERD is zero, no error notifier

Product/QEMU 68c5879fc6b8600c6d045f6234f2d8cc128f8bfa, `kf3-bins/68c5879f`, flags as run35 plus
`KF3_BAR0_TRACE=1`; run on 2026-10-07 at 16:04-16:06 UTC. [command](run36-command.json),
[status](run36-status.json), [trace](run36-qemu.log.gz), [requests](run36-requests.log) (every
`BAR0-TRACE` line), [completion](run36-complete.json), [host health](run36-host-health.txt),
[host after](run36-host-after.txt), [unit result](run36-unit-result.txt),
[9/9 gates](run36-gates.log) (11/11 USER births), [build](run36-build.log),
[native oracle](gr-tier-native-run36.log) (GR tier PASS, Xid count 5 before and after),
[watchdog recovery](run36-watchdog-recovery.log) (cleanup verified). After 98 s:
ConfigManagerErrorCode 43, nvidia-smi exit 9. Host afterwards: no QEMU, NBD disconnected, display
enabled, P8, Xid count unchanged (5).

**Measured (seen in run36 at 68c5879f):**
- **Abort point unchanged:** 858 RPCs, teardown at 544, last RPC `GPFIFO_SCHEDULE` of
  c1d00020/ff040009 returning 0 (VFIO 2861).
- Eleven windows opened, one per Translated kernel-channel schedule, and the read trap flipped 22
  times. Window #1 (RM's own scrubber channel) closed before its bottom half ran. Windows #2-#9
  (the kernel channels of c1d00013 … c1d0001c, each accepted by Windows) hold 15 accesses each (8 for
  #3): two interrupt-mask groups and the next RPC's queue-head write. Each group is 7 writes:
  `LEAF_EN_SET(2)=0x101`, `LEAF_EN_SET(4)=0xc10007a`, `TOP_EN_SET=0xfffffffe`,
  `LEAF_EN_CLEAR(2)=0x101`, `LEAF_EN_CLEAR(4)=0xffffffff`, `TOP_EN_CLEAR=0x8`,
  `LEAF_EN_SET(4)=0x4000000` (`kf_trap::cpuintr` offsets). **None of them holds a BAR0 read.**
- Window #10 (after c1d0001d's schedule, closed by an `RmControl`) holds the guest's interrupt
  self-test: `CPU_INTR_LEAF_TRIGGER` (`0xb81640`) = `0x81`. Then come reads of the PCI-config
  mirror's MSI control (`0x088068`) = **0** and MSI-X header (`0x0880c8`) = **0**, then the leaf and
  top reads and their clears. In vfio-1 the same self-test is at head 2515, where `0x088068` reads
  `0x00817805` (MSI enabled); `0x0880c8` read `0x00050011` at head 55 (MSI-X present, disabled).
- **Window #11 (the paging channel, closed by the first `Free`): 29 accesses, all writes. Four
  interrupt-mask groups, then the `Free`'s queue-head write. No BAR0 read.**
- One-time dump at that `Free`: the channel's USERD words 0-63 are all `00000000` (`scheduled=true
  serves=1 last_put=Some(0)`); error notifier: **none declared** by the channel's allocation.
- Windows' own record (event logs read from the run's overlay after the VM stopped, read-only NBD
  and NTFS, the watchdog-recovery procedure; private copies on the host):
  `Microsoft-Windows-DxgKrnl-Admin` event 549, GraphicsVendorId 4318 (0x10DE), Status `0xC0000001`
  (STATUS_UNSUCCESSFUL), FailureReason 1; event 457 "Start Device Failed", Status `0xC0000001`. The
  System log holds no nvlddmkm error event.
- Limits of the trace: a read between the reply's publication and the bottom half's ROMD flip (an
  untimed, short interval) is not recorded. The usermode passthrough page (`0xbb0000`, the doorbell
  and usermode timer) and the host timer page are never trapped. The vfio-1 window reads none of
  their registers either.

**Falsifier outcomes.**
- *Hypothesis 1 (a BAR0 read in the window returns a value Windows rejects): falsified.* The
  window closed by the first `Free` contains no BAR0 read at all, like the accepted windows #2-#9.
- *Hypothesis 2 (USERD or error notifier state): weakened.* USERD holds `GP_PUT = GP_GET = 0` and
  nothing else, which is what the guest's own CPU-RM writes for a sysmem USERD: `memset` 0 of
  `NV_RAMUSERD_CHAN_SIZE` (`kfifoSetupUserD_GM107`, `ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:797-808`,
  called for sysmem USERD on a GSP client at `ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2323-2336`;
  both read in OGKM 580.65.06). The channel declares no error notifier, so none can hold an error.
- *Prediction held:* the abort did not move (the trace changes no answer).

**What the interrupt groups suggest (inferred, not tested).** Per RPC, an accepted window shows two
mask groups in kayfabe and one in vfio-1. The abort window shows four. That fits one extra
GPU-locked RM entry without an RPC before the `Free`: for example a free of an object that needs no
RPC, or a local query. So Windows decides to tear down right after `GPFIFO_SCHEDULE` returns, with
no MMIO and no RPC in between. It decides from state it already holds, which includes everything
kayfabe answered earlier.

## Hypothesis 5: a refused query consumed later (2026-10-07, after run36)

**Measured (run36 at 68c5879f against vfio-10, aligned with `difflib` on (function, control/class)).**
- From the GR kernel channel's creation to the abort, the two RPC streams match except for three
  divergences. (1) vfio-10 creates a SEC2 (`0x26`) kernel channel (c1d0001a, 2454-2465), which
  kayfabe does not advertise. (2) vfio-10 arms notifier 34 (HDCP), which kayfabe reports absent
  under §S. (3) vfio-10 runs about 250 more display controls: DP AUX, ELD, `IS_MODE_POSSIBLE`.
  kayfabe answers IMP as bypassed (run30). From VFIO 2834 on, the last 28 RPCs before the abort
  are identical.
- In run36 the guest's own clients leave **29 distinct controls unserviced that the real GSP
  answers with status 0** in vfio-10. Six more are refused by both or are absent from vfio-10. In
  the power/thermal/perf/clock area: `0x20808524` (THERMAL, legacy, 4-byte params), `0x2080a70a`
  (POWER, legacy), `0x2080a630` (PMGR, legacy, 1160 B), `0x2080a801` (LPWR, legacy, 1028 B),
  `0x2080a060` (PERF, legacy), `0x20809004` (CLK, legacy, 1544 B), `0x20810108` and `0x2081010d`
  (the `0x2081` binary-API class), `PERF_SET_POWERSTATE` `0x2080205b`,
  `PERF_GET_CURRENT_PSTATE` `0x20802068`, `LPWR_DIFR_CTRL` `0x20802801`, `0x20802806` (LPWR),
  `BIF_GET_PCIE_POWER_CONTROL_MASK` `0x00800106`. The names come from OGKM 580.65.06 where it has
  them. Interface numbers are from `ctrl2080base.h:46` (`0x85` = THERMAL legacy non-privileged)
  and the same file's other rows.
- In vfio-10, the RPCs that come right after the abort point (2862-2864) are THERMAL legacy queries
  on c1d00002: `0x2080852e`, `0x20808530`, `0x2080852a`.

**Hypothesis 5 (inferred, medium):** StartDevice's stage after the paging channel is thermal and
power initialisation. It consumes the results of earlier power, thermal or perf queries that kayfabe
refused. The refusals were tolerated when they happened, which is why the abort-point rule ("the
teardown follows the first refusal not tolerated") never named them. The stage then fails before it
sends its first RPC.

## DIAGNOSTIC run37: zero `NV_OK` for the refused power/thermal/perf/clock queries (setup)

**Change (diagnostic only, never a product answer; `KF3_DIAG_ZERO_OK=1`, default off, the runner's
flag list gains it; `crates/kf-rm/src/diagzero.rs`).** A link placed just before the unserviced
ledger answers 13 controls with `NV_OK` and their params zeroed. Each was sent by one of the guest's
own clients, left unserviced by kayfabe in run36 and answered with status 0 by the real GSP in
vfio-10: `0x20808524`, `0x2080a70a`, `0x2080a630`, `0x2080a801`, `0x2080a060`, `0x20809004`,
`0x20810108`, `0x2081010d`, `0x2080205b`, `0x20802068`, `0x20802801`, `0x20802806`, `0x00800106`.
A serialized envelope is left unserviced. Nothing is forwarded to the host and no GPU state is
touched. The list was captured from one boot, which is acceptable for a default-off diagnostic and
is why it can never become a product answer. `KF3_BAR0_TRACE` stays on. kf-rm tests pass (2 new).

**Why zero, and what it cannot show.** Zero is the least specific `OK`. If Windows checks a value
(for example "at least one thermal sensor"), a zero answer fails that check just as the refusal
does. So a run where the abort does not move weakens hypothesis 5 but does not falsify it outright.

**Falsifier, stated before run37.** Hypothesis 5 (power/thermal/perf/clock part): the abort after
the paging channel's `GPFIFO_SCHEDULE` follows from one of these 13 refusals, consumed later. It is
**supported** if the run logs `DIAG-ZERO-OK` answers for the listed controls and the abort moves
past VFIO 2861: the next RPCs would be the THERMAL legacy queries `0x2080852e`/`0x20808530`/
`0x2080852a`, which kayfabe would then refuse or answer, or something later. It is **weakened** if
the answers are logged and the abort stays at VFIO 2861. If the abort moves EARLIER, a zero answer
broke something the refusal did not, and the run names it. *Prediction:* the abort moves past 2861.

### Run37 result (DIAGNOSTIC): StartDevice gets past the paging channel; the guest then bugchecks 0x119 on a command submission

Product/QEMU fd636c98eff3b4998e32dd8e42b3085d7cc80fab, `kf3-bins/fd636c98`, flags as run36 plus
`KF3_DIAG_ZERO_OK=1`; started 2026-10-07 16:22 UTC. [command](run37-command.json),
[trace](run37-qemu.log.gz), [requests](run37-requests.log) (`DIAG-ZERO-OK`, refusals, windows),
[gates and build](run37-gates-build.txt) (9/9, 11/11 USER births), [native oracle](gr-tier-native-run37.log)
(GR tier PASS, Xid count 5 before and after), [controller error](run37-controller-error.txt),
[unit journal](run37-failure-journal.txt), [host after](run37-host-after.txt). Host afterwards: no
QEMU, NBD disconnected, display enabled, P8, Xid count unchanged (5).

**Measured (seen in run37 at fd636c98):**
- All 13 listed controls were answered `NV_OK` with zeroed params (14 answers: `0x2080a801` twice),
  logged `DIAG-ZERO-OK #1-#14`.
- **The abort moved.** After the paging channel's `GPFIFO_SCHEDULE` (VFIO 2861), Windows went on.
  It sent the THERMAL legacy queries `0x2080852e` and `0x2080852a`. kayfabe left both unserviced,
  and Windows tolerated that; `0x20808530` was not sent. Windows then created clients c1d00021-c1d00026
  (vfio-10's c1d00022-c1d00027), a second VA space with `SET_DEFAULT_VASPACE`, a TSG copy channel
  (token 0x80c: two submissions retired by the engine, `GP_GET` 2, CE2 relays #4-#5) and a TSG GR
  channel (token 0xd: GR tier admitted, `GPU_PROMOTE_CTX` satisfied, never rung). 620 RPCs, **no
  teardown**. The last RPC is the `0x9096` alloc of c1d00026, which is **VFIO 2931**. Aligned with
  `difflib`, the 74 RPCs from VFIO 2857 match vfio-10, except six extra `0x00730245` display
  controls and the missing `0x20808530`.
- Refusals in that stretch that vfio-10 answers with status 0: `0x2080852e` and `0x2080852a`;
  `NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE` `0x20800a38` (4×, one after each GR object
  alloc; OGKM 580.65.06 `ctrl2080internal.h`); `0x20801111`, the software-runlist control that the
  `KF3_SW_RUNLIST_PROBE` experiment only observes; and the second `FERMI_VASPACE_A`
  `COPY_SERVER_RESERVED_PDES` (`0x90f10106`) on c1d00021's new VA space. kayfabe refuses that last
  one with 0x56; vfio-10 answers OK for the same second range on c1d00022.
- QEMU exited (status 0) 22 s after the boundary start, because the guest reset and the runner's
  `-action reboot=shutdown` turns a reset into an exit. The guest agent never answered, and the
  controller recorded `FAILED … Guest agent did not become ready` (the unit itself succeeded).
- **Crash dump header** in the run's `pagefile.sys`, read through the read-only NBD procedure. The
  baseline's and run36's pagefile headers are all zero. `PAGEDU64`, build 26100,
  **BugCheckCode `0x119`** (VIDEO_SCHEDULER_INTERNAL_ERROR), parameters `0x2`, `0xC000000D`
  (STATUS_INVALID_PARAMETER), `0xffff968132ef72f0`, `0xffffbd87208840a0`.
- vfio-1's doorbells in the same stretch (`0xbb0090`): `0x1000d` after head 2895, `0xe` after 2917
  and `0x1000d` after 2918. The first WDDM submissions come with ~1 MB of BAR1 writes and 2068 MMU
  invalidates after head 2918.

**Falsifier outcome: hypothesis 5 supported.** With zero `NV_OK` answers to the 13 refused
power/thermal/perf/clock queries, the teardown after VFIO 2861 does not happen. *Not measured:*
which of the 13 is needed. They were not bisected.

**Inferred, not tested:**
- Bugcheck 0x119 means dxgkrnl's scheduler was running, so StartDevice completed this time. By
  Microsoft's documentation, parameter 1 = 2 is "the driver failed upon the submission of a
  command", and parameter 2 is the driver's status. That reading is quoted from memory and was not
  re-checked in this session.
- Ranked candidates for the submission failure:
  1. *(Highest.)* The software-runlist control `0x20801111`. vfio-10 sends it at 2914 for the new GR
     channel, which gets no `GPFIFO_SCHEDULE`. In vfio-1 the GR channel (chid 0xe) is rung after
     2917. kayfabe leaves `0x20801111` unserviced, so that channel is never on a runlist the guest
     driver believes in.
  2. The refused second `COPY_SERVER_RESERVED_PDES`.
  3. `INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE` unserviced.

## Stop: an owner decision is needed (2026-10-07, after run37)

Runs 36-37 used 2 of the 10 runs this loop allows. The stop condition "an owner decision is needed"
holds. Run38 is not taken.
1. **The 13 power/thermal/perf/clock queries** (`diagzero.rs` lists them with names and sizes). §S
   makes power, thermal and P-state host-owned stubs: "refused or reported absent, never filled
   with invented values". Measured in runs 36-37 (2026-10-07): refusal stops StartDevice at VFIO 2861, and zero-filled
   `NV_OK` lets it complete. Nine of the 13 have no public layout (closed legacy GSS or `0x2081`
   controls), so "reported absent" cannot be derived from OGKM for them. Options: (a) zero `NV_OK`
   as the stub's "reported absent" for these queries; (b) ask the host's own GSP with
   kayfabe-authored params (impossible for the closed layouts without capturing them); (c) keep
   refusing, which leaves Code43. A bisect (1-4 diagnostic runs) would shrink the set before the
   decision.
2. **The next wall is design work, not a single answer.** The guest's software runlist
   (`0x20801111`), plus the second reserved-PDE copy and the FECS-trace query, stand between
   StartDevice and the first successful WDDM submission. Today the software runlist is an
   observation-only experiment (`KF3_SW_RUNLIST_PROBE`).

## Bisect of the 13 zero answers, step 1, plus the server-context-only PDE copy (run38 setup)

Branch `claude/code43-stubs-20261007`, from `1fd82ed2`, 2026-10-07. The owner's rulings of 2026-10-07
(relayed by the coordinator): privileged, non-compute host-management queries may be stubbed, but
only the minimal set Windows needs. This section starts the bisect of run37's 13 answers and
batches one product repair that acts only after VFIO 2861.

**What the RPC streams say before any run (measured in run36 and run37 at 68c5879f and fd636c98, both 2026-10-07, against vfio-10 of 2026-10-05).**
- All 13 controls are sent before the abort in both runs, at the same RPC indices: ten early
  (run36 RPCs 110-166; vfio-10 116-205) and three late, `0x2080a801` (a second time), `0x2081010d`
  and `0x2080a630` (run36 RPCs 520-522; vfio-10 2834-2836), 22 RPCs before the paging client.
- The only other difference between run36 and run37 before the abort is ONE extra RPC in run37:
  right after `0x20809004` (CLK legacy 0x04) is answered, Windows sends `0x2080a0a7` (PERF legacy;
  kayfabe leaves it unserviced, vfio-10 sends it at 117 and answers 0). So Windows reads
  `0x20809004`'s answer. That does not make it the needed one.

**Bisect step 1 (diagnostic, `KF3_DIAG_ZERO_OK=1`, `diagzero.rs` `BISECT_STEP`).** Only the
three late controls are answered `NV_OK` with zeroed params; the other ten are left unserviced as
in run36. `0x2080a801` is answered at both of its occurrences (one id).

**Product repair (default on): the server-context-only reserved-PDE copy.** In run37 the second
`COPY_SERVER_RESERVED_PDES` (`0x90f10106`) on c1d00021's VA space was refused with 0x56. Decoded from
vfio-10's request at 2915 (the same call on c1d00022): `pageSize` 0x1000, `virtAddrLo` 1,
`virtAddrHi` 0, one level, physical 0x3200000, size 0x1000, video memory, page shift 29. OGKM names
this form: *"BUG 4580145 WAR: make sure only GSP's context is updated; caller is WAR if (AddrHi <
AddrLo)"* (`ogkm-580.65.06: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:4418-4425`). On the GSP
platform it skips the reservation (`:4477-4484`) and only swaps that level's backing in the
server's own walker for the server-RM-owned VA (`:4492-4543`); vfio-10 answers 0.
- `kf_abi::gvaspacepdes` no longer refuses an inverted range as malformed. It decodes it, skips
  only the two reservation-alignment rules, and names the form (`is_server_context_only`). The
  level-count, page-size and zero-level-size rules still refuse.
- `kf_rm::rmrpc::translate_published_pdes`: that form is `Translation::Inert`. No page-directory
  statement is made (its `levels[0]` is a PD level, not the root), and the init-table link answers
  `NV_OK` with the re-encoded request. kayfabe keeps no server walker; the deeper levels of an
  ordinary publication are dropped for the same reason.
- Tests: `kf-abi` `gvaspace_pdes` (the vfio-10 shape decodes, is not a root, re-encodes; its
  other rules still refuse), `kf-rm` `gvas_publication` (through the served chain: `NV_OK`, the
  re-encoded body, no statement reaches the memory plane, nothing held).

**Falsifiers, stated before run38.**
- *Bisect, H-late: the needed answers are among the three late controls.* Supported if the run
  logs `DIAG-ZERO-OK` for exactly those three ids and the abort moves past VFIO 2861 (no teardown
  after the paging channel's `GPFIFO_SCHEDULE`; THERMAL legacy `0x2080852e`/`0x2080852a` follow).
  Falsified if the abort stays at VFIO 2861: then at least one of the ten early controls is
  needed (step 2 answers the ten early ones alone).
- *The PDE repair:* only testable if the abort moves. Then it is supported if the second
  `0x90f10106` on the new VA space returns 0 and Windows goes past VFIO 2915; falsified if the
  same call is still refused. Whether the bugcheck 0x119 changes is recorded, not predicted.
- *Prediction:* the abort moves past 2861 (the late trio is sent right before the stage that
  failed).

### Run38 result: the late trio is enough to pass VFIO 2861; the PDE copy is served; a TDR follows a dead kernel copy channel

Product/QEMU 0e8a3dd535b9eb0664b219e0ba4e467dcaaaaa97, `kf3-bins/0e8a3dd5`, flags as run37 (`KF3_DIAG_ZERO_OK=1`
now answers only `BISECT_STEP`); started 2026-10-07 16:45 UTC on 172.22.1.20 (RTX 4070, host 595.91.07).
[command](run38-command.json), [trace](run38-qemu.log.gz), [requests](run38-requests.log),
[gates, oracle and build](run38-gates-build.txt) (9/9, 11/11 USER births, GR tier PASS, Xid 5 before
and after; [oracle log](gr-tier-native-run38.log)), [controller error](run38-controller-error.txt),
[unit journal](run38-failure-journal.txt), [bugcheck header](run38-bugcheck.json),
[Windows events](run38-evtx.txt), [host after](run38-host-after.txt). Host afterwards: no QEMU, NBD
disconnected, display enabled, P8, Xid count unchanged (5).

**Measured (run38 at 0e8a3dd5, 2026-10-07):**
- `DIAG-ZERO-OK` answers: exactly the three ids, four answers (`0x2080a801` twice).
- **The abort moved past VFIO 2861** with only those three answered: the THERMAL legacy queries
  `0x2080852e`/`0x2080852a` follow (RPCs 548-549), as in run37.
- **The second `COPY_SERVER_RESERVED_PDES` on c1d00021's new VA space returns 0** (RPC 600, the
  server-context-only form), where run37 refused it with 0x56. Windows then creates c1d00025/26 and
  the `0x9096` alloc (VFIO 2931), as in run37.
- **New wall, kayfabe's own:** right after that, the kernel copy channel (c1d00023, token 0x80c,
  host 0x10039; it had retired two submissions, CE2 relays #4-#5) is killed by kayfabe:
  `DEAD: ring: Read { gp: 2, va: 0x200c4010, why: "0x200c4010+0x8: 0x200c4010 not placed by us" }`,
  `REFUSED-AND-POISONED (§7)`. The address is GP entry 2 of the channel's own ring
  (`gpfifo=0x200c4000`), in the same 4 KiB page as entries 0 and 1, which were read. No walk of that
  VA space is logged between the second submission and the death, and the invalidate count (2204)
  equals run37's, so this log cannot say when or why the row went.
- Then Windows sends performance/thermal queries kayfabe leaves unserviced (`0x20809064` served,
  `0x20809004`, `0x2080b201`, `0x2080852e`, `0x2080852f`, `0x2080a618`, `0x2080a619`, `0x2080853a`,
  `0x2080853b`), frees 11 objects, sends `0x20801111` twice (unserviced), and tears down: 53 `Free`s
  (`abort_point.py`: rpcs=2607, teardown_at=644, last `fn76/20801111`, refused). Windows then
  re-initialises the adapter twice more (clients from c1d00028 and from c1d00050); each cycle's new
  kernel copy channel (token 0x80b) dies the same way at GP entry 1 (`0x200c4008`).
- Windows' System log (read-only, `recover_bugcheck.py`): nvlddmkm event 153 *"Resetting TDR
  occurred on GPUID:6"*, *"Reset TDR …"*, *"Restarting TDR …"* at 16:46:15 UTC. The pagefile dump
  header is all zero: **no bugcheck** this run (run37's 0x119 did not recur). The guest agent
  answered a ping, but the status script timed out after 150 s, so ConfigManagerErrorCode and
  nvidia-smi were **not measured**; the controller stopped the unit at 16:48:49.

**Falsifier outcomes.**
- *H-late: supported.* The three late answers alone move the abort past VFIO 2861.
- *The PDE repair: supported.* The call is answered 0 and Windows goes past VFIO 2915 (to 2931).
- *Prediction held.*

**Inferred, not tested.** The TDR follows from the dead copy channel: Windows' work on it can never
complete, so its scheduler times out. In vfio-1 the same stretch rings the doorbell `0x1000d` after
head 2895 and `0xe`/`0x1000d` after 2917-2918, so the reference's kernel channels keep running. The
`0x20801111` refusals after the frees are runlist updates of the teardown, not its cause.

## Bisect step 2, and a placement diagnostic (run39 setup)

**Bisect step 2 (diagnostic):** `BISECT_STEP = [0x2080a801]` (both occurrences). **Diagnostic for the
new wall:** `KF3_MAPLOG=1` (the existing VA-plane timeline, `kf_mem::maplog`, default off) is added
to the runner's flag list, and the ring-read death message now names the placement rows around the
unplaced address (`kf_qemu::mem::describe_neighbours`, error path only; a unit test pins the format).
No answer changes besides the bisect.

**Falsifiers, stated before run39.**
- *Bisect, H-a801: `0x2080a801` alone is the needed answer.* Supported if the abort moves past VFIO
  2861 with only `0x2080a801` answered. Falsified if it stays at 2861: then `0x2081010d` or
  `0x2080a630` is needed (step 3 answers those two).
- *Placement (a reading of run39, 2026-10-07, not a test):* if the copy channel dies again, the maplog shows which
  walk or root change removed the row of `0x200c4000`, or shows that none did. If no walk removed
  it, the hypothesis "a walk diff retired the ring's row" is falsified and the death is in the
  T-space resolver instead.
- *Prediction:* the abort moves past 2861 (`0x2080a801` is the one control of the trio sent both
  early and late).

### Run39 result: `0x2080a801` alone does not pass VFIO 2861 (H-a801 falsified)

Product/QEMU 7eb96b9885199438fa17e12ffe47aee42d1093a9, `kf3-bins/7eb96b98`, flags as run38 plus
`KF3_MAPLOG=1`; started 2026-10-07 16:56 UTC. [command](run39-command.json), [trace](run39-qemu.log.gz),
[requests](run39-requests.log), [status](run39-status.json), [unit result](run39-unit-result.txt),
[gates, oracle and build](run39-gates-build.txt) (9/9, 11/11 USER births, GR tier PASS, Xid 5 before
and after; [oracle log](gr-tier-native-run39.log)), [bugcheck header](run39-bugcheck.json),
[Windows events](run39-evtx.txt), [host after](run39-host-after.txt). Host afterwards: no QEMU, NBD
disconnected, display enabled, P8, Xid 5 (unchanged).

**Measured (run39 at 7eb96b98, 2026-10-07):** `DIAG-ZERO-OK` answers `0x2080a801` twice and nothing
else. `abort_point.py`: rpcs=858, teardown_at=544, last `fn76/a06f0103` (`GPFIFO_SCHEDULE` of the
paging channel) result 0: **the abort is back at VFIO 2861**, as in run36. After 92 s:
ConfigManagerErrorCode 43, nvidia-smi exit 9. No bugcheck (pagefile header all zero), no display
event in the System log. The copy channel of run38 is never reached, so the maplog and the new
death message measure nothing this run.

**Falsifier outcome: H-a801 falsified.** With run38: `0x2081010d` or `0x2080a630` is needed.

## Bisect step 3 (run40 setup)

`BISECT_STEP = [0x2080a630]` (PMGR legacy, 1160 B), chosen over `0x2081010d` because the stage
that fails is the power/thermal one and PMGR is the power manager (inferred; a coin-toss guard:
whichever singleton is tried, one more run settles the other). Everything else as run39
(`KF3_MAPLOG` stays on for the copy-channel death if the abort moves).

**Falsifiers, stated before run40.**
- *H-a630: `0x2080a630` alone is the needed answer.* Supported if the abort moves past VFIO 2861.
  Falsified if it stays: then `0x2081010d` is needed (alone or with one of the other two), and
  step 4 answers `0x2081010d` alone.
- *Placement (a reading of run40, 2026-10-07, not a test):* as stated for run39.

### Run40 result: `0x2080a630` alone does not pass VFIO 2861 (H-a630 falsified)

Product/QEMU c8fa737ee2bb6667480d5da3392891595cb040f7, `kf3-bins/c8fa737e`, flags as run39; started
2026-10-07 17:00 UTC. [command](run40-command.json), [trace](run40-qemu.log.gz),
[requests](run40-requests.log), [status](run40-status.json), [unit result](run40-unit-result.txt),
[gates, oracle and build](run40-gates-build.txt) (9/9, 11/11 USER births, GR tier PASS, Xid 5 before
and after; [oracle log](gr-tier-native-run40.log)), [bugcheck header](run40-bugcheck.json),
[Windows events](run40-evtx.txt), [host after](run40-host-after.txt). Host afterwards: no QEMU, NBD
disconnected, display enabled, P8, Xid 5 (unchanged).

**Measured (run40 at c8fa737e, 2026-10-07):** one `DIAG-ZERO-OK` answer (`0x2080a630`).
`abort_point.py`: rpcs=858, teardown_at=544, last `fn76/a06f0103` result 0 — **VFIO 2861 again**.
After the status settled: ConfigManagerErrorCode 43, nvidia-smi exit 9. No bugcheck, no display
event in the System log.

**Falsifier outcome: H-a630 falsified.** With runs 38-39: `0x2081010d` is needed, alone or with
`0x2080a801` or `0x2080a630`.

**What the bisect already settles about the coordinator's 2026-10-07 note (four of the 13 have
public layouts).** `0x2080205b` `PERF_SET_POWERSTATE`, `0x20802068` `PERF_GET_CURRENT_PSTATE`,
`0x20802801` `LPWR_DIFR_CTRL` and `0x00800106` `BIF_GET_PCIE_POWER_CONTROL_MASK` are in OGKM
580.65.06 (`ctrl2080perf.h:162`, `:885`; `ctrl2080lpwr.h:42`; `ctrl0080bif.h:137`). All four are
early controls, and run38 passed VFIO 2861 with them unserviced (measured, run38 at 0e8a3dd5,
2026-10-07). So they are outside the minimal set: the product stub answers none of them, and no
zero-filled reply is shipped for them. The minimal set lies inside the late trio, whose three ids
have no public name or layout in any local OGKM tree.

## Bisect step 4 (run41 setup)

`BISECT_STEP = [0x2081010d]` (the `NV2081` binary-API interface, `FINN_NV2081_BINAPI_INTERFACE_ID`
`0x208101`, OGKM 580.65.06 `src/common/sdk/nvidia/inc/g_finn_rm_api.h:425`; message 0x0d and its zero-byte params have no public
definition).

**Falsifiers, stated before run41.**
- *H-2081010d: `0x2081010d` alone is the needed answer.* Supported if the abort moves past VFIO
  2861. Falsified if it stays: then the minimal set is `0x2081010d` plus `0x2080a801` or
  `0x2080a630`, and this would be the third consecutive run that does not move the abort, so the
  loop stops (stop rule) and reports.
- *Placement (a reading of run41, 2026-10-07, not a test):* as stated for run39.

### Run41 result: `0x2081010d` alone passes VFIO 2861 — the minimal set is one control

Product/QEMU 846a4b9c3d9ebbffd6bb23af7d25e1deab20a1b6, `kf3-bins/846a4b9c`, flags as run40; started
2026-10-07 17:05 UTC. [command](run41-command.json), [trace](run41-qemu.log.gz),
[requests](run41-requests.log), [maplog of c1d00021's VA space](run41-maplog-c1d00021.txt),
[gates, oracle and build](run41-gates-build.txt) (9/9, 11/11 USER births, GR tier PASS, Xid 5 before
and after; [oracle log](gr-tier-native-run41.log)), [controller error](run41-controller-error.txt),
[unit journal](run41-failure-journal.txt), [bugcheck header](run41-bugcheck.json),
[Windows events](run41-evtx.txt), [host after](run41-host-after.txt). Host afterwards: no QEMU, NBD
disconnected, display enabled, P8, Xid 5 (unchanged).

**Measured (run41 at 846a4b9c, 2026-10-07):**
- One `DIAG-ZERO-OK` answer, `0x2081010d` (zero params). **The abort moved past VFIO 2861** and the
  run follows run38's course: `abort_point.py` rpcs=2605, teardown_at=644, last `fn76/20801111`
  (unserviced); the server-context-only PDE copy returns 0; the kernel copy channel (token 0x80c)
  dies at GP entry 2; nvlddmkm logs two TDR cycles (Resetting/Reset/Restarting at 17:06:34 and
  17:06:40 UTC); no bugcheck; the status script timed out again (Code43/smi **not measured**).
- **The copy channel's placement rows are EMPTY at its death:** `0x200c4010 not placed by us
  (rows=0 below=none above=none)`. The maplog shows walk #248 mapping `va=0x200c4000 len=0x20000`
  (sysmem, aperture 3) into that VA space right after the channel's birth, then only `MAP`s for that
  space up to the death — no `UNMAP`, no retire, no root statement for it — and two submissions
  completing on the engine in between.

**Bisect conclusion (measured over runs 38-41, 2026-10-07):** answering `0x2081010d` alone is
sufficient; `0x2080a801` alone and `0x2080a630` alone are not; the ten early controls, the four
with public layouts included, are not needed. **The minimal set is `{0x2081010d}`.**
- What is known about it (read, 2026-10-07): the pinned retail Windows 580.88 `nvlddmkm.sys`
  (SHA256 `31c79cce…`, the same file `sw_runlist` pins) exports it in its NVOC method table
  (`NVOC_EXPORTED_METHOD_DEF`, OGKM 580.65.06 `src/nvidia/inc/libraries/nvoc/runtime.h:73-84`) at
  file offset 0xecfbf0: `methodId 0x2081010d`, `paramSize 0`, `flags 0x10208` =
  `NON_PRIVILEGED | ROUTE_TO_VGPU_HOST | GSP_PLUGIN_FOR_VGPU_GSP` (`control.h:208, 253, 290`). Its
  interface is OGKM's `FINN_NV2081_BINAPI_INTERFACE_ID` (`g_finn_rm_api.h:425`); message 0x0d has no
  public name, layout or body. A web search (2026-10-07) found no public layout (gVisor's `nvgpu`
  ABI names only the class `NV2081_BINAPI`).
- ⚠ So the one needed answer is **not** shown to be a power/thermal control, and the retail flags
  say it is **non-privileged**. Whether it belongs under §S ("privileged host management: stub")
  is for the owner; see the product section below.

**Inferred, not tested:** the TDR follows from the dead copy channel. Why its rows map is empty is
open; the candidates are a second writer of the same `PlacedRows` that clears it outside a walk,
or a channel holding a different mirror's rows than the walker fills (rows=0 rules out "one row was
retired" for this death).

## The product stub, the FECS-trace answer, and a death-mirror diagnostic (run42 setup)

**A. The bisect's result becomes the product (default on; `KF3_DIAG_ZERO_OK` and `diagzero.rs` are
removed, and the runner's flag list loses it).**
- `kf_abi::hoststub` + `hoststub_generated.rs`, generated by `tools/windows-ctrl-export/derive.py`
  from public OGKM 580.65.06 (`FINN_NV2081_BINAPI_INTERFACE_ID`, `g_finn_rm_api.h:425`) and the
  pinned retail Windows 580.88 `nvlddmkm.sys` (the control's `NVOC_EXPORTED_METHOD_DEF` row at RVA
  0xed23f0: `paramSize 0`, flags `0x10208`). The one non-public number, message 0x0d, is listed once
  in `tools/windows-ctrl-export/stubs.txt` with the bisect that selected it; the tool refuses an id
  the retail driver does not export exactly once. No id is hand-typed in Rust.
- `kf_rm::hoststub::HostStubPolicy`: answers `0x2081010d` `NV_OK` only after the guest's fn1
  declares Windows 580.88 at wire version 580.65.06 (the `sw_runlist` identity rule; a changed
  declaration revokes it), only at the generated params size, never for a serialized envelope;
  nothing reaches the host. Unit tests: no cell for other drivers, inert without the identity,
  revoked by a changed identity, and only that control at that size is answered.
- Recorded in `docs/OWNER_RULINGS.md` §S as ASSUMED, owner to confirm, with the retail flags that
  say the control is non-privileged.

**C. `NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE` (`0x20800a38`) answered as absent.**
- Driver matrix: the command id and `NV2080_CTRL_INTERNAL_GR_FECS_TRACE_HW_ENABLE_PARAMS` (24 bytes;
  `grRouteInfo` at 0, `bEnable` at 16, every tag 535.309.01-615.71.09) are now measured (full
  30-tag regen on 172.22.1.20; `ranges.tsv` only gains rows). `kf_abi::fecstrace` reads them.
- `kf_rm::fecstrace::FecsTracePolicy`: `NV_OK`, `bEnable = NV_FALSE`, for the default route only
  (all-zero `grRouteInfo`), at the measured size. kayfabe gives the guest no FECS context-switch
  trace, so "disabled" is the feature-absent answer (§H profiler class); vfio-10 answers 1 because
  there the guest's GSP owns the engine. The SET controls Windows then sends (`0x20800a3a`,
  `0x20800a39`, `0x20800a37`; `fecs_event_list.c:1539-1590`) stay unserviced ("refuse the rest").
  The sweep row for `0x20800a38` carries a correction above its old argument.

**C. The second `COPY_SERVER_RESERVED_PDES`** is already served (run38).

**Diagnostic (error path only).** When a channel dies, kf3 now also prints whether its mirror's rows
are still the plane's mirror for its key, with both row counts and host spaces. Run41's death showed
an empty rows map although the walker had mapped the ring; this tells a stale mirror from a cleared
one. `KF3_MAPLOG` stays on.

**Falsifiers, stated before run42.**
- *A (the product stub reproduces the bisect).* Supported if the run logs `HOST-STUB 0x2081010d`
  answers, no `DIAG-ZERO-OK` line, and the abort moves past VFIO 2861 as in runs 38 and 41.
  Falsified if the abort is at VFIO 2861 (then the identity gate or the generated cell did not
  apply, and the log says which).
- *C (FECS).* Supported if `FECS-TRACE` answers are logged and Windows continues past each of them
  (the first is at run41's RPC ~200). Prediction: Windows then sends the three SET controls, which
  are refused; falsified-as-harmless if the teardown starts right after one of them.
- *Death mirror (a reading of run42, 2026-10-07, not a test):* if the copy channel dies again, the
  new line says "same rows: false" (a stale mirror) or "same rows: true" with 0 rows (cleared).
- *Prediction:* the abort passes VFIO 2861; the copy channel dies again and a TDR follows
  (nothing here repairs it).

### Run42 result: the product stub reproduces the bisect; FECS answered; the copy channel's mirror is the live one, emptied

Product/QEMU c53554788421d5b4045f75e975e958764e1106b4, `kf3-bins/c5355478`, flags as run41 without
`KF3_DIAG_ZERO_OK` (removed); started 2026-10-07 17:23 UTC. [command](run42-command.json),
[trace](run42-qemu.log.gz), [requests](run42-requests.log), [gates, oracle and build](run42-gates-build.txt)
(9/9, 11/11 USER births, GR tier PASS, Xid 5 before and after; [oracle log](gr-tier-native-run42.log)),
[controller error](run42-controller-error.txt), [unit journal](run42-failure-journal.txt),
[bugcheck header](run42-bugcheck.json), [Windows events](run42-evtx.txt), [host after](run42-host-after.txt).
Host afterwards: no QEMU, NBD disconnected, display enabled, P8, Xid 5 (unchanged). Host checks at
c5355478: kf-abi, kf-rm, kf-qemu tests pass, rustfmt clean, `ci_gates.sh` 0, Clippy 258 = base 258,
no new signature.

**Measured (run42 at c5355478, 2026-10-07):**
- One `HOST-STUB 0x2081010d` answer (RPC 526, the only time Windows sent it), no `DIAG-ZERO-OK`
  line. **The abort passes VFIO 2861**: `abort_point.py` rpcs=2631, teardown_at=653, last
  `fn76/20801111` (unserviced) — run41's course, with nine more RPCs (below).
- `0x20800a38` answered 31 times (`bEnable = NV_FALSE`). After each, Windows sends exactly one SET,
  `0x20800a3a` (`SET_FECS_TRACE_WR_OFFSET`), 27 times, left unserviced; it never sends `0x20800a39`
  or `0x20800a37`, so `fecsBufferReset` returns at its first SET, as read. No teardown follows any
  of them.
- The kernel copy channel dies again at GP entry 2 (`rows=0`), and the new line says: **the
  channel's mirror IS the plane's live mirror for its key** (same rows map, host space 0xcafe0063),
  and that map is empty. Two TDR cycles follow (System log 17:24:36 UTC), no bugcheck; Code43/smi
  **not measured** (the status script timed out again).

**Falsifier outcomes.** *A: supported* (the generated, identity-gated stub alone moves Windows past
VFIO 2861). *C (FECS): supported, harmless:* answered, and the one refused SET per answer is
tolerated. *Death mirror:* "cleared, not stale" — the walker's rows for the space were emptied after
the walker had placed the ring.

## A row-commit diagnostic for the copy channel's death (run43 setup)

No answer changes. On a channel's death kf3 now also prints the mirror's row-log epoch and its last
eight commits (`RowsLog::recent`, error path only; unit-tested): every insert or removal of a row
commits its range under the rows' write lock, so the last commits name the change that emptied the
map, or show that nothing committed (a writer that bypasses the log). `KF3_MAPLOG` stays on. The
hoststub/fecstrace unit tests now fail instead of skipping if the 580.65.06 wire table is missing.

**Falsifiers, stated before run43 (a reading of run43, 2026-10-07, not a test of a repair).**
- *H-cut: a range unmap (`unmap_range`/`cut_rows`) or a whole-space retire emptied the rows.*
  Supported if the last commits before the death cover the ring's VA (0x200c4000) with a range
  removal. Falsified if the last commit touching it is the walk's insert at walk #248's time.
- *H-bypass: the rows were replaced or cleared without a commit.* Supported if the epoch is
  non-zero, the last commits do not cover 0x200c4000, and the map is still empty.
- *Prediction:* the abort passes VFIO 2861 and the copy channel dies as in runs 38-42.

### Run43 result: one walk empties the copy channel's VA space right before Windows' server-context-only PDE copy

Product/QEMU 5f201b8ae5fe680d2c4358c11ea47a34a672e756, `kf3-bins/5f201b8a`, flags as run42; started
2026-10-07 17:30 UTC. [command](run43-command.json), [trace](run43-qemu.log.gz),
[requests](run43-requests.log), [the walk](run43-walk2192.txt), [gates, oracle and build](run43-gates-build.txt)
(9/9, 11/11 USER births, GR tier PASS, Xid 5 before and after; [oracle log](gr-tier-native-run43.log)),
[controller error](run43-controller-error.txt), [bugcheck header](run43-bugcheck.json),
[Windows events](run43-evtx.txt), [host after](run43-host-after.txt). Host afterwards: no QEMU, NBD
disconnected, display enabled, P8, Xid 5 (unchanged).

**Measured (run43 at 5f201b8a, 2026-10-07):**
- Same course as run42 (`abort_point.py` rpcs=2629, teardown_at=653, last `fn76/20801111`); TDR
  cycles in the System log; no bugcheck; Code43/smi **not measured** (status script timeout).
- The death line: the copy channel's mirror (host space 0xcafe0063) has 0 rows; its row log's last
  commits are four range removals 57 ms before the death, covering `[0x20002000, 0x2024f000)`,
  `[0x20250000, 0x20370000)`, `[0x24000000, 0x265f0000)` and `[0x3fdf0000, 0x3fff1000)`.
- The maplog names them: **walk #2192**, triggered by the guest's `ALL_VA` invalidate of
  c1d00021's root `0xefa38000` (hub_only=false), UNMAPs all 22 rows of the space (`APPLIED +0 -22`),
  the ring `0x200c4000` included. The next RPC is Windows' server-context-only
  `COPY_SERVER_RESERVED_PDES` on that VA space (answered 0). Every removed row lies in
  `[0x20000000, 0x40000000)`: one entry of the page-shift-29 level, the level whose instance that
  copy names (one level, shift 29, now at FB 0x3200000; decoded from vfio-10's same call). In the
  BAR1 space Windows maps three 64 KiB pages just before (walks #2187-#2191).

**Inferred, not tested.** Windows moves its shift-29 page-directory level of that VA space to a new
instance at FB 0x3200000, points the parent at it, invalidates, and tells the GSP (the WAR). The GSP
only swaps its server walker's pointer (`mmuWalkModifyLevelInstance(…, bCopyEntries=NV_FALSE,
bUpdatePde=NV_FALSE, …)`, `ogkm-580.65.06: gpu_vaspace.c:4534-4541`,
`mmu_walk_migrate.c:30-171`). On real hardware the GPU walks the new level, which the client filled.
kayfabe's walk at that invalidate finds no valid entry under it, so it unmaps everything and the
copy channel dies at its next GP fetch. Why the walk sees an empty level is open: the new level's
contents not yet visible in kayfabe's store when the walk reads it, the walk reading a cached old
instance, or a level at FB 0x3200000 being outside what the walker reads. This is kayfabe's own
memory plane, an unprivileged area: no owner decision is needed to continue it. The next
measurement is the walk's PDE chain for VA 0x200c4000 at that invalidate (level addresses and entry
values).

## Stop: an owner decision is needed for the software runlist; the bisect and C are done (2026-10-07, after run43)

Runs 38-43 used 6 of this loop's 10 runs.

**A (done).** Minimal set `{0x2081010d}` (runs 38-41), shipped as the generated, identity-gated
`kf_rm::hoststub` (run42 verified it reproduces the bisect). §S entry in `docs/OWNER_RULINGS.md`:
ASSUMED, owner to confirm; the retail export row says the control is NON_PRIVILEGED and its area is
not shown to be power. The 12 other controls stay refused; the four with public layouts are among
them, so no invented reply is shipped for them.

**C (done).** The second `COPY_SERVER_RESERVED_PDES` is served (run38). `0x20800a38` is answered
"disabled" (run42); its one SET is refused and tolerated.

**D.** Bugcheck per run (pagefile header, `recover_bugcheck.py`): run37 0x119; runs 38-43 none.
Windows now runs well past StartDevice, but goes into TDR cycles (nvlddmkm event 153 in runs 38, 41,
42, 43), and the guest stops answering the status script within 150 s, so ConfigManagerErrorCode
and nvidia-smi were last measured in runs 39-40 (7eb96b98, c8fa737e; 2026-10-07): 43 / exit 9, abort at VFIO 2861. **No success is
claimed.**

**B: `0x20801111`, the software-runlist submit — the decision (data, 2026-10-07).**
- What it is (read): no OGKM name or layout; neighbour `0x20801110` is public in OGKM 610.43.02
  (`FIFO_CONFIG_CTXSW_TIMEOUT`). The pinned retail Windows 580.88 export row (file offset 0xec9de0):
  `paramSize 40`, flags `0x40` = `ROUTE_TO_PHYSICAL` only, i.e. kernel-privileged by default
  (`control.h:170-178`). The proprietary Linux 535/610 handlers (repo evidence at `8223efc9`,
  `tools/windows-debug-capture/evidence/runlist-20261005/README.md`) resolve the `0xb297` runlist
  object and a Memory object from the params, read 12-byte records and a 16-bit index list from that
  memory, and call the scheduler: it submits a guest-built runlist.
- What Windows sends (vfio-10, 2026-10-05): at 2400, 2914, 2957, … ; request and reply are the same
  40 bytes, status 0. Run41/42 raw words: `[ff008250, 0, ff000100, 1|2, 0x6000, 1|2, 1, 0, 0, 0]`
  (runlist handle, runlist buffer memory, count, offset 0x6000, …). The second GR TSG (c1d00024)
  gets no `GPFIFO_SCHEDULE` in either vfio-10 or kayfabe: only this control puts it on a runlist.
- What kayfabe does today: `KF3_SW_RUNLIST_PROBE` logs the raw words and leaves the control
  unserviced (0x56 to the guest); the first GR channel runs anyway because it also gets
  `GPFIFO_SCHEDULE`.
- **Options.** (a) Decode the 40 bytes and the guest's 12-byte records from its runlist buffer
  (bounded, hostile input), map each record to the VM's own Translated TSG, and enable exactly those
  host twins with the host's unprivileged `GPFIFO_SCHEDULE` on the VM's own groups; answer after the
  host acts. Needs the owner to accept a layout known only from proprietary-driver disassembly
  (refuse-rather-than-guess). (b) Treat runlist submission as host-owned scheduling under §S: answer
  `NV_OK`, and enable every live Translated kernel TSG's host twin at birth, ignoring the guest's
  runlist contents (no closed layout parsed; Windows' ordering and removals have no effect). (c)
  Keep refusing: the second GR channel never runs, so Windows' first GR submission cannot complete.
- Neither (a) nor (b) needs a privileged host verb or a new emulated channel; both forward nothing
  from guest bytes. The choice is about semantics, so it is the owner's.

**Next without an owner decision:** the memory-plane wall above (walk #2192 empties the space). It
probably comes before `0x20801111` matters: the copy channel dies first.

## Memory plane: a directory level at FB offset 0 was walked as "no sub-table" (run44 setup)

Branch `claude/code43-mem-20261007`, from `6003d790`, 2026-10-07. Task A of the loop (no owner
decision needed): why walk #2192 (run43) found nothing under c1d00021's VA space and removed all 22
rows.

**What run43 already shows (measured, run43 at 5f201b8a, re-read 2026-10-07).**
- The pattern is identical in all three adapter starts of run43 (walks #2192, #4477, #6801; the
  copy channel dies after each): in the BAR1 space (root `0xf1cac000`) Windows maps FB
  `0xefa30000` (the 64 KiB holding that space's directory tables; the root is `0xefa38000`) twice,
  then **FB `0x0`** (`va=0x6380000 … at=0x0`, walks #2191/#4476/#6800); the next invalidate is the
  `ALL_VA` one of c1d00021's space, whose walk unmaps every row (`+0 -22`, `-24`, `-23`); the next
  RPC is the server-context-only `COPY_SERVER_RESERVED_PDES` (one shift-29 level); then the three
  BAR1 mappings are removed. FB 0 is ordinary guest heap here (walk #82 maps
  `0x120000000+0xefc00000 at=0x0` in another space).
- No walk of the run carried a refusal (`walk report carried refusals` never logged). So the level
  was not refused (OOB, unaligned, foreign aperture): the walk read it as absent.

**The defect (read, 2026-10-07).** Every walk path of the walk kernel had a second presence test
besides the aperture: a sub-table address of 0 was "a null pointer, not a sub-table", skipped with
no refusal (`cuda/walk/kf_walk.cu` at 6003d790: lines 547 and 626/633 in the serial walk, 1764 and
1785/1795 in the parallel walk). The rule was kayfabe's own (carried from the pre-v3 C emulator and
`kayfabe-mmu`'s walker), not the hardware's: for a PDE the aperture alone is the validity, and RM's
own software walker returns the PDE address whenever the aperture is not INVALID
(`ogkm-580.65.06: src/nvidia/src/kernel/gpu/mmu/gmmu_trace.c:111-134`); its "no address" sentinel is
`0xf` because "All base addresses are aligned and 0xf is unaligned"
(`src/nvidia/inc/kernel/gpu/mmu/mmu_trace.h:32`). VER2 (Pascal-Ada: `kern_gmmu_fmt_gm10x.c:165-182`,
`_PDE_APERTURE_INVALID`) and VER3 (Hopper, Blackwell: `kern_gmmu_fmt_gh10x.c:132-158`, aperture +
PCF + address, no valid bit) both say so, and the walk kernel decodes both from the one format
descriptor, so the fix is per format, not Ada-specific.

**Inferred, tested by run44:** Windows put the moved shift-29 level (VER2 PD1, the instance whose
entry 1 covers `[0x2000_0000, 0x4000_0000)`) at FB 0, CPU-wrote it through the BAR1 mapping of FB 0,
repointed its parent and invalidated. The walk dropped the whole subtree, so the diff protocol
correctly unmapped every placement under it: that is why **all 22** rows went (every row of the
space lies under that one entry).

**Changes (product, default on — a correctness fix, no flag).**
- `cuda/walk/kf_walk.cu`: the six zero-address skips are removed; a correction block
  (`KF_PDE_ADDR_ZERO_IS_A_TABLE`) above `kf_dir_present` cites the source. A present, aligned,
  in-window table at 0 is walked like any other; hostile input is bounded exactly as before (I1
  nesting, budget, the window check). `kf_walk.ptx` regenerated with NVRTC 12.2.140 (`make_ptx.py`;
  the committed PTX at 6003d790 was first reproduced byte for byte).
- Fixtures: the VER2 builders wrote a dual PDE's unused big half as `big_pde(0)`, which is a
  PRESENT big-page table at FB 0, read as absent only because of the defect. They now write
  `_APERTURE_INVALID` (`kf_cuda::synth::DUAL_HALF_ABSENT`; `kf-harness` `Tree`), as the VER3
  builder already did.
- Regression tests: `kf-gate9` gains a run43 replay per format (`ver2/ver3_level_move_to_fb0_is_quiet`:
  rows under one PD1 entry, walk and acknowledge, move PD1 to FB 0 and repoint, the next diff must be
  empty with no refusal; plus a known-positive: the level at 0 emptied must UNMAP every placement).
  The CUDA suite gains `diff/move_directory_level_to_gpga0_is_quiet`. GPU-free unit tests: the
  harness's level-move helper (VER2 and VER3) and the fixtures' INVALID big half.
- Diagnostic (kf-rm, logged only for the rare server-context-only form): `kf-rm: server-context-only
  PDE copy … levels=[shiftN phys=… size=… ap=…]`, so the moved instance's address is measured.

**Falsifiers, stated before run44.**
- *H-zero: the moved level is at FB 0 and the null rule hid it.* Supported if the new line shows the
  shift-29 level at `phys=0x0` and the `ALL_VA` walk of c1d00021's space right before it no longer
  unmaps the space (no `-22`-style `APPLIED`), and the kernel copy channel (token 0x80c) does not die
  with "not placed by us". Falsified if `phys` is not 0 (then the null rule did not hide this level),
  or if it is 0 and the walk still empties the space.
- *Native:* `kf-gate9` must pass both new replay checks and both known-positives (9/9 gates).
- *Recorded, not predicted:* whether the TDR cycles stop. Prediction: they do not all stop, because
  the second GR channel (c1d00024) is still never scheduled (`0x20801111` refused, task B).

### Run44 result: the level at FB 0 is walked, the copy channel lives, and the adapter reports OK until a GR-tier refusal leads to TDR and bugcheck 0x116

Product/QEMU a7a6108e65cdb98edc3b37519bbd04019f378926, `kf3-bins/a7a6108e`, flags as run43; started
2026-10-07 17:57 UTC on 172.22.1.20 (RTX 4070, host 595.91.07). [command](run44-command.json),
[trace](run44-qemu.log.gz), [requests](run44-requests.log), status samples at
[uptime 13 s](run44-status-uptime13s.json) and [50 s](run44-status-uptime50s.json),
[gates, replay checks, oracle and build](run44-gates-build.txt) (9/9, 11/11 USER births, both
`level_move` replays and both known-positives PASS, GR tier PASS, Xid 5 before and after;
[oracle log](gr-tier-native-run44.log)), [controller error](run44-controller-error.txt),
[unit journal](run44-failure-journal.txt), [bugcheck header](run44-bugcheck.json),
[Windows events](run44-evtx.txt), [host after](run44-host-after.txt). Host afterwards: no QEMU, NBD
disconnected, display enabled, P8, Xid 5 (unchanged). Host checks at a7a6108e: kf-cuda, kf-harness,
kf-rm tests pass, rustfmt clean, `ci_gates.sh` 0, Clippy 260 = base 260 (6003d790), no new signature.

**Measured (run44 at a7a6108e, 2026-10-07):**
- **The moved level is at FB 0:** `server-context-only PDE copy client=0xc1d00021 vaspace=0xff000850
  page_size=0x1000 levels=[shift29 phys=0x0 size=0x1000 ap=1]` at both adapter starts that reach it
  (log lines 20498 and 43979; each call prints the line twice — the translation runs twice per call).
- **The walk at that invalidate (walk #2192 again) keeps the space:** `APPLIED +59 -0`. All 22 rows
  of run43 stay; the new rows are Windows' new mappings at `0x6000_0000…` (PD1 index 3) and the
  same rows again at `0x100_xxxx_xxxx`: a second PD2 entry (index 4) resolves to the same level at
  FB 0. No walk of the run carried a refusal.
- **The kernel copy channel lives:** no "not placed by us" death. The second start's copy channel
  (token 0x80b) forwarded 1356 GP entries in 182 submissions.
- **The adapter reports OK for the first time in this loop:** at guest uptime 13 s and 50 s the
  NVIDIA RTX 4070 has ConfigManagerErrorCode 0, Status OK, and `nvidia-smi` exits 0 printing
  `NVIDIA GeForce RTX 4070, 580.88` (the Microsoft Basic Display Adapter reports code 10).
- **The next wall (kayfabe's own, the GR tier's allowlist):** the kernel GR channel (token 0x3,
  c1d00015:ff040001) is killed 3.2 s after the first driver walk: `GrMethod { class: 0xC997 (ADA_A),
  subch: 0, method: 0x1B00, "not in the 3D/compute allowlist" }`. Decoded from the segment's first
  words (`200406c0 00000001 20286060 00000001 1000f010`): a 4-word incrementing write of
  `SET_REPORT_SEMAPHORE_A..D` (`ogkm-580.65.06: src/common/sdk/nvidia/inc/class/clc997.h:3913-3922`):
  address `0x1_2028_6060`, payload 1, D = `RELEASE`, `RELEASE_AFTER_ALL_PRECEEDING_WRITES_COMPLETE`,
  pipeline location `ALL`, `STRUCTURE_SIZE` bit 28 set (`:3923-3990`). It is the completion of
  Windows' first GR work. Windows then re-initialises the adapter (a second client set from
  c1e00037), whose GR channel dies the same way 10.1 s after the first walk.
- `0x20801111` is still refused (task B not applied); `abort_point.py`: rpcs=6225, teardown_at=659,
  last `fn76/20801111` (unserviced).
- After the 50 s sample the guest agent's socket closed. Pagefile header: **bugcheck 0x116**
  (`VIDEO_TDR_FAILURE`, parameters `0xffff80849bde9010`, `0xfffff8063b1a4580`, `0x0`, `0xd`). The
  System log holds no display event since the start (inferred: the crash came before it was flushed).

**Falsifier outcomes.** *H-zero: supported* (`phys=0x0`; the walk no longer empties the space; the
copy channel does not die). *Native:* both replays and both known-positives PASS. *Prediction*
"the TDRs do not all stop" held, but the cause seen (run44 at a7a6108e, 2026-10-07) is the GR
channel's refused 3D semaphore release, not (shown) the unscheduled second GR channel.

**Inferred, not tested.** The TDR and the 0x116 follow from the dead GR channel: its semaphore is never
released, Windows' scheduler times out, the reset re-creates the channel, it dies the same way, and
the second failed recovery is fatal. The aliasing PD2 entry is what the hardware would also walk (the
aperture is the validity), so mirroring it is correct by construction; why Windows points PD2[4] at
the level is not known.

## GR tier batch 3: the 3D report-semaphore release (run45 setup)

Run44's wall is the GR tier's own allowlist (§S GR ruling, item 2: one class/method set at a time,
native first). **Change (product, under the existing `KF3_KERNEL_GR_WORK` tier):**
`kf_chan::grtables` admits `SET_REPORT_SEMAPHORE_A/B/C/D` (`0x1B00-0x1B0C`) on the 3D class:
`A`/`B` as an address pair (`AddressUpper8` / `AddressLower32`, 4 bytes), resolved through the
placement rows and emitted by the perimeter like the notifier; `C` the payload word; `D` re-authored
only as a plain one-word RELEASE (`STRUCTURE_SIZE_ONE_WORD`, `REPORT_NONE`, a named
`PIPELINE_LOCATION`, `RELEASE` 4:4 carried; operation, report, reduction, trap, awaken,
flush-disable and every unnamed bit refused by name; `clc997.h:3913-4010`). A family row by hand:
the offsets and every `D` field are identical in the public TURING_A, AMPERE_A/B, ADA_A and HOPPER_A
headers (`clc597/c697/c797/c997/cb97.h`, compared 2026-10-07); BLACKWELL_A/B's headers name only the
class id, so the rows are refused there by name. Compute and 2D keep their tables.
- Unit tests: `D` (run44's word and named variants pass; eleven hostile variants refused); rows only
  for the 3D class of the header families; `tmode_gr_tests` replays run44's 39-word segment: the
  release is admitted (one `GrAddress` `0x1_2028_6060`, 4 bytes) and the segment then stops, by
  name, at `InertSubchannelMethod { subch: 5, method: 0x200, value: 1 }` (ruling 4).
- Native oracle (`kf-gr-tier`, before the run): GP 1 now starts with run44's release
  (`D = 0x1000_f010`) and the oracle checks the 3D engine wrote the payload at the guest VA
  (`3d_report_semaphore_written_by_engine`) and that 25 GR words were re-authored.

**What the segment does next (read from run44's words, 2026-10-07).** After the release, a host
semaphore ACQUIRE on the same address waits for the payload, then `0x2001a080 0x4000000N`: one
method `0x200` on software subchannel 5, which run32's segment bound to the non-class value 1. Ruling
4 (§S GR ruling) says: refuse every later software method on that subchannel by name. So this batch
is predicted to move the GR channel's death by 19 words, not to remove it.

**Falsifiers, stated before run45.**
- *H-sem: the 3D release is now executed.* Supported if no GR channel dies at method `0x1B00`-`0x1B0C`
  and the first GR death names `InertSubchannelMethod { subch: 5, method: 0x200 }`. Falsified if a GR
  channel still dies at `0x1B00`-`0x1B0C` (a `GrField` on `D` names which field) or at its address
  (`VirtualUnresolved`).
- *Native:* `kf-gr-tier` PASS with the new check.
- *Prediction:* TDR and bugcheck 0x116 recur (the channel still dies, at the software method);
  the adapter still reads OK at the first status samples.

## Task B, default off: the software-runlist submit as host-owned scheduling (awaiting owner confirmation)

⚠ **AWAITING OWNER CONFIRMATION.** The owner has not chosen among options (a), (b), (c) of the
2026-10-07 stop above; the controller recommended (b) for one experiment behind a new default-off
flag. `KF3_SW_RUNLIST_HOST_OWNED=1` (`kf_rm::sw_runlist_host`, `kf_qemu::chan`):
- kf-rm answers the retail control `0x20801111` `NV_OK` with the 40-byte request echoed (vfio-10's
  answer), only after the guest declared the Windows 580.88 identity of the `sw_runlist` cell, only
  at the exact params size, never for a serialized envelope; its contents are IGNORED: no field is
  decoded, the guest's runlist buffer is never read, nothing reaches the host.
- kf-qemu opens every Translated (kernel) channel's guest-side gate at birth (`scheduled = true`).
  Its host ring is already scheduled on the host at birth by kayfabe's own unprivileged
  `GPFIFO_SCHEDULE` of its own group (`kf_chan::host` ring birth), so a kernel channel then runs
  whether or not Windows ever schedules it; the guest's `GPFIFO_SCHEDULE(false)`, STOP and EVICT
  still close it. The BORN line logs `scheduled_at_birth`.
- No privileged verb, no new emulated channel, nothing forwarded from guest bytes. Windows'
  ordering, timeslices and removals have no effect: that is the semantic for the owner to accept
  or reject. Unit tests: identity gate, exact size, serialized and neighbour controls refused, the
  request echoed.
