# Windows Code43: one implementation gap per boot

**STATUS: RESEARCH, 2026-10-07.** Owner-directed continuation after
[exact request decoding](../windows_exact_decode_20261007/README.md). No Windows
success or master-promotion claim. Borrowed RTX4070; host595.91.07 remains attached
to its ordinary display driver. Only owned fresh-overlay Kayfabe VMs are used.

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
