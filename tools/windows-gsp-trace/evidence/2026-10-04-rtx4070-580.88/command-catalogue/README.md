# Windows GSP calls: meanings, Linux counterexamples and version limits

**STATUS: RESEARCH, 2026-10-04. No command in this inventory is established as
Windows-only.** This extends and corrects the earlier [handler audit](../command-audit/README.md).
It describes the retained native Windows capture, not successful Windows
execution through Kayfabe. No new live hardware comparison was performed.

[All 129 direct controls, with descriptions](controls.md) ·
[Machine-readable catalogue](catalogue.json) ·
[Linux evidence and exact record locations](linux-evidence.json) ·
[Public source index and revisions](source-index.json)

## Answer to the OS/version/die question

The hypothesis that these are collectively Windows-only commands is false:
**68 of the 129 numeric IDs also occur in saved NVIDIA Linux-driver traffic.**
The evidence has different observation boundaries, which must remain distinct:

- **59** occur in native Linux ioctl/GSP captures. Seven of these are observed
  at a real NVIDIA GSP boundary; the remaining examples at the ioctl boundary
  establish Linux requests, not that every request reached GSP.
- **50** occur in Linux 580.65.06 guest GSP requests to Kayfabe. These overlap
  the native set in 41 IDs and add nine. They establish Linux-driver emission;
  Kayfabe's replies are not native NVIDIA responses.
- **61** have no occurrence in these bounded Linux samples. That is an
  observation gap, not proof of OS exclusivity or even Linux non-use.
- **Zero** have been established as Windows-only. The table deliberately has
  no inferred Windows-only category.

This is an ID-level counterexample census. The same ID can have different
selectors, layouts, optional features or state-dependent behavior. For example,
Windows uses variants of the opaque clock queries that the current authored
Linux-derived input gates reject. Finding the ID on Linux does not implement
those Windows variants or justify copying a reply.

**The closed proprietary Linux kernel-module question remains untested at the
GSP boundary.** NVIDIA's proprietary Linux userspace emitting an ioctl to an
open kernel module is not the same experiment as tracing the closed kernel
module. The GA106 and GB203 native fixtures explicitly use open modules; the
GA102/GA104 baseline receipts used here do not pin module flavour. Earlier
closed-module application tests are not substitutes for a comparable GSP census.
OGKM is the preferred public semantic source, not evidence of all closed-module
behavior.

## What source can prove about OGKM callability

**Follow-up, 2026-10-04:** an operation accepted by the proprietary Windows
driver but rejected by every relevant path in specified unmodified OGKM
releases can be classified as **accepted on Windows, unavailable through those
OGKM interfaces**. That is stronger than absence from a Linux trace. It still
does not distinguish Windows from the proprietary Linux kernel module, and
“all versions” requires an enumerated release range rather than extrapolation
from the sampled trees. No all-version uncallability claim has been established
for this inventory.

Distinguish five questions: is a command declared; does stock OGKM contain a
caller; can an authorized caller reach a dispatcher; will the selected firmware
accept it; and does the normal workload actually use it? Each needs different
evidence. Missing declarations, missing callers and missing CPU function bodies
are individually insufficient proofs of uncallability.

The 580.65.06 source makes the distinction concrete:

- [Ordinary resource lookup](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/libraries/resserv/src/rs_resource.c#L117)
  returns NOT_SUPPORTED when the exported method is absent. This can support a
  rejection proof for that resource/interface once overrides, inherited entries
  and alternate routes have been checked.
- [Legacy-GSS selection](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/interface/deprecated/rmapi_deprecated_control.c#L92)
  recognizes a command bit and GSP/vGPU state, rather than requiring a separate
  export per ID. Its [handler](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/interface/deprecated/rmapi_gss_legacy_control.c#L32)
  applies caller/object checks and forwards to physical RM/GSP.
- [BinAPI control](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/rmapi/binary_api.c#L62)
  also forwards opaque controls on a firmware-client GPU. A missing symbolic
  command definition is not evidence that this path rejects its numeric ID.
- The pool-query route described below is exported with a GSP-routing flag.
  [Privilege validation](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/rmapi/control.c#L701)
  defaults such controls to kernel callers. Rejection from userspace would not
  prove rejection from that authorized context.

OGKM itself uses proprietary GSP firmware. “No open implementation of the
firmware operation” and “requires the proprietary CPU kernel driver” are
therefore separate claims. Source proves the routing and rejection conditions;
it does not disclose firmware behavior that is absent from the public tree.

## Capture and comparison matrix

| Label | OS / driver | GPU / die | What this evidence establishes | Principal limit / provenance |
| --- | --- | --- | --- | --- |
| W104 (this capture) | Windows 11 build 26100, NVIDIA 580.88; reported GSP 580.65.05 | RTX 4070 / AD104 | Real GSP requests and responses while the GPU is assigned by VFIO | Unknown early prefix; one later missing reply; not Kayfabe success. [Capture provenance](../README.md) |
| I102 | NVIDIA Linux 580.159.04 | RTX 3090 / GA102 | Native `/dev/nvidiactl` control ioctls across 23 traces | Module flavour not established here; not a full GSP boot trace. [Fixture](../../../../../traces/v3_refusal_audit/ga102_vrf/README.md) |
| I104 | NVIDIA Linux 580.159.04 | RTX 3070 / GA104 | Native graphics-control ioctls across eight traces | Module flavour not established here. [Graphics baseline](../../../../../docs/design/V3_GFX_TESTSET.md) |
| I203 | NVIDIA Linux 580.159.04, open kernel module | RTX 5080 / GB203 | Native copy-engine fixture ioctls | Narrow workload. [Blackwell provenance](../../../../../docs/design/V3_FAMILY_PORT_BLACKWELL.md) |
| G106 | NVIDIA Linux 580.159.04, instrumented open kernel module | RTX 3060 / GA106 | Native GSP control call/status transcript during initialization | Narrow startup workload; no comparable complete display capture. [Capture method](../../../../../traces/real_ga106/README.md) |
| I106 | NVIDIA Linux 580.159.04, open kernel module | RTX 3060 / GA106 | Native CUDA-init control ioctls | Userspace boundary; not every ioctl becomes an RPC. [Capture method](../../../../../traces/real_ga106/README.md) |
| K106 | NVIDIA Linux guest 580.65.06 | Kayfabe GA106 model on RTX 3060 | Linux-driver GSP requests during CUDA/display probes | Replies are Kayfabe's. [Pinned Windows-branch baseline](https://github.com/reindertpelsma/kayfabe/blob/c50fad9ac485f53d45d4ea77a21cb7206267c65a/traces/v3_windows/discovery_20261004/linux_baseline/run_lin_b1t_580.65.06_qemu.trimmed.log) |

The original failed Windows-through-Kayfabe experiment used RTX 3060/GA106,
Windows 580.88 and the Linux 580.65.06 comparison, whereas this native Windows
capture uses AD104. Those differences have **not** been factored out by a
same-board native Windows/Linux test. “580” alone also does not guarantee
identical firmware or ABI. Driver branch, exact firmware, module flavour,
display configuration, feature policy and workload are separate variables.

## Public source search, in the requested order

The search covered literal command IDs throughout each local source tree,
including generated dispatch code and declarations without callers. Definitions
were then read for meaning and parameter types; the SDK structure sizes were
compiled on x86-64. Literal hits by themselves do not establish semantic matches.
The saved index records all hit locations and selected semantic references.

| Priority / source | Pinned version or commit | Result for the 129 direct IDs |
| --- | --- | --- |
| OGKM | 580.65.06, `307159f2623d3bf45feb9177bd2da52ffbc5ddf9` | 65 public numeric command definitions; full local tree searched |
| OGKM | 580.159.04, `b81d58ee0224d1d290bef1c080592b619e184042` | Same 65; full local tree searched |
| OGKM | 595.84 release archive, hash in source-index.json | Adds HDCP-state 0x00730280 and HDCP-control 0x00730282; latter has a size mismatch |
| OGKM | 610.57.04 `e4a5faa2567f28c8eabe0ebb6422b6d0abcf37eb`; 610.43.02 `57130a2702d565be81200ea2e114abcb0455e8bb` | Same 67 numeric definitions; 610.57.04 local tree is a partial SDK checkout |
| Nouveau | Linux `156fa7417fac89fd9dcf3a4ee88785ff90ab6411`, Nouveau subtree | 18 numeric matches; only extra candidate is historical 0x00730122, with an incompatible layout |
| Nova | Linux `6f3ed7fec72fc8979b2a8c7219c0a9fcfc8d07b5`, Nova subtree | No matches in this six-file stub snapshot; not a claim about later Nova implementations |
| Envytools | `f102b82381f3f11cee113d16374c87091db039d9` | One matching method, 0xa06f0103, already defined by OGKM |
| gVisor nvproxy | `bc2e4d8a69cb7703793876690c8e670b98e13699` | 21 numeric matches, no additional unresolved meanings |

These are official public source repositories and public reverse-engineering
projects, not leaked sources. This is a bounded search, not a claim that no
other public definition exists. Nova's limited snapshot is recorded explicitly.

Three discoveries change the earlier narrow SDK-name result:

1. `NV0080_CTRL_DMA_SET_DEFAULT_VASPACE` (0x00801812) lacks `_CMD_` in its name.
   Searching the full tree finds it in OGKM 580.65.06; the earlier name-pattern
   search missed it. The old “64 named” count described that narrower method.
2. 0x00730280/0x00730282 appear in newer OGKM. For the latter, **595's structure
   is 2608 bytes but Windows 580 declares 2600**. The name is useful evidence,
   not permission to transplant the newer layout.
3. Nouveau r535 calls 0x00730122 GET_CONNECT_STATE with a **16-byte** structure;
   Windows declares **8 bytes**. r570/OGKM 580 use 0x00730108 for connect state.
   The historical numeric match does not decode the Windows call.

Thus 67 direct IDs have OGKM definitions across the searched versions (one
layout-incompatible newer definition); one further ID has a historical Nouveau
candidate with incompatible layout. The other 61 have no public semantic
resolution here, although several have partially understood empirical behavior
in Kayfabe. Every unresolved ID still gets its own table row with observed
length, status and Linux occurrences. A full invented description would be
less useful than recording exactly which semantics are still missing.

## Correction: GR_GFX_POOL_QUERY_SIZE is not proved Linux-inaccessible

The earlier report said Linux “compiles that control out” and therefore cannot
answer it. **That conclusion was wrong.** The generated CPU function pointer
can be absent because the exported method routes to the physical/GSP side.

- The [580.65.06 control flags](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/inc/kernel/rmapi/control.h#L159)
  disable CPU implementations for `ROUTE_TO_PHYSICAL` (0x40); that is a routing
  flag, not an OS restriction.
- [RM's control prologue](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/rmapi/resource.c#L258)
  forwards such controls with `NV_RM_RPC_CONTROL` for a firmware client.
- The [query's public definition](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gr.h#L1298)
  explicitly documents kernel callers and the parameter meanings.

| Field | Offset / width in the public structure | Meaning |
| --- | --- | --- |
| maxSlots | 0 / u32 | Input: nonzero maximum number of graphics preemption-pool slots |
| slotStride | 4 / u32 | Output: byte distance between consecutive slots |
| ctrlStructSize | 8 / u64 | Output: required control-structure size in bytes |
| ctrlStructAlign | 16 / u64 | Output: control-structure alignment |
| poolSize | 24 / u64 | Output: required pool size in bytes |
| poolAlign | 32 / u64 | Output: pool alignment |

The layout is 40 bytes. Field meanings are known; the sizing formula is not.
No successful query response has been retained in this Windows capture, and
no targeted Linux kernel-caller probe has been performed. Public routing code
makes that probe worth doing, but does not predict its return status. A userspace
permission failure would not settle kernel-caller behavior. Even if a trusted
host probe succeeds, Kayfabe must derive authored facts without forwarding
untrusted guest control requests.

The original startup refusal remains a blocker hypothesis, not a demonstrated
cause or proof that Windows always needs a separate allocation strategy.
There is no source-backed basis here for a universal “Linux doesn't use this
buffer because Windows owns it” explanation.

## Is this only GSP, or also kernel channels?

**The observation plane is GSP RPC traffic.** It includes channel allocation,
context initialization/promotion, preemption-buffer binding, scheduling and
address-space publication. It does **not** capture the channel's GPFIFO entries,
GPU pushbuffer methods, command execution, doorbell MMIO or WDDM/TDR timing.
Even hardware-engine class allocations are resource-management observations,
not recordings of work submitted to those engines.

A new limitation matters for channel ownership: **all 472 retained RM_ALLOC
frames are 112 bytes total and end after the allocation header**. For 179
requests and 179 replies the header declares a nonempty parameter block, but
that block is absent from the retained frame. All twelve 0xc56f channel requests
declare 368 parameter bytes without containing them. The catalogue validates
this directly against the export; the underlying reason is not established.
Do not silently supply those bytes from a different ABI or assume they were
captured merely because paramsSize is nonzero.

There is a concrete recorder follow-up: [OGKM's allocator RPC builder](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/vgpu/rpc.c#L11166)
sets the common length using the fixed allocation header, then copies parameter
bytes into the buffer. The current [recorder](../../../queue.c) exports only
the common header's declared length plus framing. This is a plausible reason
for losing allocation bytes; it does not prove the Windows queue contained
valid extra bytes. A future recorder change needs bounded, stable queue-page
capture and explicit validation before interpreting any bytes past that length.

The [public channel allocation structure](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/alloc/alloc_channel.h#L135)
contains privilege-related flags, but those flags are not readable in these
allocation frames. A request emitted by kernel RM does not by itself identify
an RM-internal privileged channel. This capture therefore does not establish
which observed channels require Kayfabe's kernel-channel translation/emulation.

The strict export checks cover all 4,535 retained records. Queue prefixes are
unknown (first retained sequences 2707 request / 2710 reply), and one later
reply is missing. Seven control-request heads and six reply heads have
fragmented bodies that this catalogue does not reassemble. RPC sequence fields
are zero, so counts and status histograms are independent per direction, not
proof of one-to-one temporal pairing. Zero recorder FIFO drops is not a claim
of a complete boot capture. A successful deferred wrapper is not evidence of
the nested operation's later execution.

## Outer RPCs, allocations and deferred calls

All five observed RPC IDs are known. Request/reply counts below are independent;
outer reply status is zero throughout, while individual controls can return an
inner error. The public class-name generation (Fermi/Kepler/Ampere/Ada) describes
an interface, not necessarily the die using it.

| RPC ID / source | Requests / replies | Meaning |
| --- | --- | --- |
| [10: FREE](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/generated/g_rpc-structures.h#L160) | 23 / 23 | Release an RM object using client/parent/object handles. This changes resource lifetime; successful FREE does not describe prior GPU execution. |
| [71: CONTINUATION_RECORD](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/vgpu/rpc.c#L2030) | 7 / 7 | Continue an oversized RPC payload across queue records. This is transport framing, not a separate resource-control operation. This catalogue counts continuations but does not reassemble them. |
| [76: GSP_RM_CONTROL](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/generated/g_rpc-structures.h#L1506) | 1999 / 1999 | Issue an RM control for a client/object: command ID, inner status, declared parameter length, flags/access rights and parameter bytes. The 129 nested direct command IDs are tabulated separately. |
| [103: GSP_RM_ALLOC](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/generated/g_rpc-structures.h#L1491) | 236 / 236 | Allocate an RM resource under a parent, identified by client/object handles and class, with status/flags and declared parameter size. This capture retains only allocation headers; it cannot establish the missing parameter fields. |
| [4099: POST_EVENT](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/generated/g_rpc-structures.h#L1545) | 0 / 5 | GSP-to-CPU event notification with client/event handle, notify index, status/data and optional event bytes. The five records are asynchronous events, not replies to five new control commands. Event-specific payload semantics are not decoded here. |

All 20 allocation classes follow. Every observed allocation reply has inner
status zero. Declared nonzero lengths do not mean those bytes were retained.

| Class / public source | Requests / replies | Declared bytes | Meaning / capture limit | Audited Kayfabe status |
| --- | --- | --- | --- | --- |
| `0x00000000` [NV01_ROOT](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl0000.h) | 30 / 30 | 120 | RM client root; owns the resource hierarchy and client-level allocation state. | allocation decoder exists |
| `0x00000070` [NV01_MEMORY_VIRTUAL](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl0070.h) | 36 / 36 | 24 | Virtual-memory resource object used to describe/reserve GPU address-space mappings; allocation does not establish mapped physical backing. | allocation decoder exists |
| `0x00000073` [NV04_DISPLAY_COMMON](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl0073.h) | 11 / 11 | 0 | Common display-management resource through which connector, head and link controls are issued. | allocation decoder exists |
| `0x0000007e` [NV01_EVENT_KERNEL_CALLBACK_EX](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/nvos.h#L390) | 2 / 2 | 24 | Extended kernel-callback event resource for RM notifications. Registration is not a captured callback invocation or a GPU work item. | allocation decoder exists |
| `0x00000080` [NV01_DEVICE_0](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl0080.h) | 30 / 30 | 56 | Logical GPU device resource under a client, containing subdevices and device-wide settings. | allocation decoder exists |
| `0x00002080` [NV20_SUBDEVICE_0](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl2080.h) | 30 / 30 | 4 | One GPU subdevice resource; target of GPU/memory/bus and internal controls. | allocation decoder exists |
| `0x00002081` [NV2081_BINAPI](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl2081.h) | 5 / 5 | 4 | Opaque binary-API resource. The class is public, but the observed control payload semantics are not fully resolved. | allocation decoder exists |
| `0x0000402c` [NV40_I2C](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl402c.h) | 3 / 3 | 0 | Physical board I2C access resource. Kayfabe deliberately refuses this capability; native allocation does not justify host bus forwarding. | denied by capability policy |
| `0x00005080` [NV50_DEFERRED_API_CLASS](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl5080.h) | 12 / 12 | 0 | Deferred-API resource used to register control operations for later execution. The observed wrappers carry context initialization/promotion. | denied by capability policy |
| `0x0000902d` [FERMI_TWOD_A](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl902d.h) | 6 / 6 | 0 | 2D graphics engine object. Its allocation is visible, but 2D method-stream execution is outside this capture. | allocation decoder exists |
| `0x00009067` [FERMI_CONTEXT_SHARE_A](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl9067.h) | 5 / 5 | 12 | Context-sharing resource for channel/group context relationships. Allocation parameters are absent, so the observed sharing policy is unresolved. | allocation decoder exists |
| `0x00009096` [GF100_ZBC_CLEAR](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl9096.h) | 6 / 6 | 0 | Zero-bandwidth-clear management resource for graphics clear values/compression state; not an observed framebuffer-clear instruction. | allocation decoder exists |
| `0x000090e7` [GF100_SUBDEVICE_INFOROM](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl90e7.h) | 1 / 1 | 0 | InfoROM management resource, including black-box telemetry controls. It has no allocation decoder in the audited Windows branch. | no allocation decoder |
| `0x000090f1` [FERMI_VASPACE_A](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl90f1.h) | 6 / 6 | 56 | GPU virtual address-space resource, used by mappings and page-directory publication controls. | allocation decoder exists |
| `0x0000a06c` [KEPLER_CHANNEL_GROUP_A](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cla06c.h) | 11 / 11 | 20 | Channel group (TSG), grouping channels for shared scheduling/context state. Group allocation does not imply privileged ownership. | allocation decoder exists |
| `0x0000a140` [KEPLER_INLINE_TO_MEMORY_B](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cla140.h) | 6 / 6 | 0 | Inline-to-memory engine object for writing method-supplied data to GPU-addressable memory. Actual methods/data are not recorded here. | allocation decoder exists |
| `0x0000c56f` [AMPERE_CHANNEL_GPFIFO_A](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/clc56f.h) | 12 / 12 | 368 | GPFIFO channel resource used on this Ada device. Twelve requests declare 368 bytes but retain no allocation parameter body, so privilege/ring addresses cannot be read from this evidence. | allocation decoder exists |
| `0x0000c7b5` [AMPERE_DMA_COPY_B](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/clc7b5.h) | 12 / 12 | 8 | DMA copy-engine object; the eight declared allocation parameter bytes are absent. Copy commands and transfer completion are outside the captured management plane. | allocation decoder exists |
| `0x0000c997` [ADA_A](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/clc997.h) | 6 / 6 | 0 | Ada 3D graphics engine object. Allocation proves resource creation, not rendered output or successful graphics submission. | allocation decoder exists |
| `0x0000c9c0` [ADA_COMPUTE_A](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/clc9c0.h) | 6 / 6 | 0 | Ada compute-engine object. Allocation proves resource creation, not a CUDA launch or result. | allocation decoder exists |

The deferred wrapper contributes one extra command ID that is not a direct
RM_CONTROL in this capture; it does not increase the 129-direct-ID count:

| Nested command | Requests / replies inside wrapper | Description |
| --- | --- | --- |
| `0x2080012b` [NV2080_CTRL_CMD_GPU_PROMOTE_CTX](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h#L984) | 4 / 4 | Promote/bind supplied engine-context resources for a selected client/channel using context-buffer descriptors, flags and routing information. This changes context state; it is also nested in four deferred requests here. |
| `0x2080012d` [NV2080_CTRL_CMD_GPU_INITIALIZE_CTX](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h#L1076) | 4 / 4 | Initialize a virtual engine context whose storage the caller has already cleared. Inputs identify engine/client/channel or group, virtual-memory handle or size, physical address/attributes and DMA handle/offset. The preserve flag permits rebinding an already initialized context. Seen only inside four deferred wrappers here; acceptance is not proof of later execution. |

## What would establish a Windows-specific requirement?

1. On the **same GPU/board**, capture native Windows and native Linux from
   before RM initialization, with exact driver/firmware versions, GSP policy,
   display topology and equivalent workloads recorded. Repeat Linux with its
   open and closed kernel modules where supported. Keep ioctl, GSP and GPU
   channel capture layers separate.
2. Repeat the version axis on that board. A command appearing in another Linux
   version is a counterexample to OS exclusivity; a layout change needs its own
   decoder. Record selectors and return status, not just command IDs.
3. Repeat the matched test on GA106 and AD104 (then additional architectures
   for a portability claim). Device capability or display-path selection can
   explain absence without an OS distinction.
4. For the remaining candidates, inspect Linux callers and feature guards and
   deliberately exercise the corresponding feature. Probe the pool query from
   an appropriate kernel caller. One trace's silence can never establish that
   no Linux path or future driver version uses the command.
5. Record a conclusion as “Windows-specific in this tested matrix” unless a
   stronger interface/source contract establishes exclusivity. Separately test
   the Kayfabe implementation; native VFIO success is only the reference.

The borrowed PC now times out and the owner says it was probably handed over;
it is treated as unavailable, with no further reconnect/reboot attempts. The
capture, installer/recorder source and this analysis are durable on the
controller and research branch. This offline update neither rents new hardware
nor claims to have completed the controlled matrix above.

## Reproduction and checks

From this repository, with the audited Windows branch available separately:

```sh
python3 tools/windows-gsp-trace/audit/catalogue.py \
  --repo . --windows-repo /workspace/kf-windows \
  --output /tmp/windows-command-catalogue
```

The script requires Python 3 and `zstd`. It parses each native ioctl header,
keeps actual GSP and emulator-request evidence distinct, records compressed-file
hashes and line/record locations, validates allocation-frame lengths and the
Windows export hash, and renders one row per direct ID from the existing audit.
It fails if a required dataset is missing. The pinned Linux guest baseline is
at Windows-branch `c50fad9ac485f53d45d4ea77a21cb7206267c65a`; verify file hashes
against linux-evidence.json when reproducing from another checkout.

Source references/descriptions are reviewed inputs, not machine-inferred
semantics. source-index.json records source revisions, search scope and hit
locations; re-run the numeric search across local clones when extending it.
For example, `rg -n -i '0x0*2080121f[uUlL]*\b' SOURCE_TREE` includes declarations
and unreferenced generated entries. Then search the resolved symbols for callers
and implementations. Do not conclude absence merely from a name pattern.

The two `*-sizes.c` files reproduce selected SDK layout sizes; compile each
against the corresponding OGKM release's `src/common/sdk/nvidia/inc` and
`src/common/inc` include paths. No binaries are checked in. Zero-size entries
come from parameterless declarations. All selected 580.65.06 structures match
the captured declared lengths; the newer HDCP and historical Nouveau exceptions
are explicitly called out above. A length match is necessary, not sufficient,
for semantic equivalence.

Validation for this revision: all 70 selected source-definition locations were
checked against the local pinned files; 67 selected 580.65.06 layouts and two
595.84 layouts were compiled and checked (including zero-parameter entries).
The prior independent native census agrees for all 59 IDs and their counts,
statuses and examples. All 35 input-file hashes match; regeneration produces
byte-identical JSON/tables; every direct control and class appears once; local
documentation links and `git diff --check` pass.

This update changes research documentation and analysis tooling only. It does
not add product handlers, weaken a capability rule, claim CUDA/D3D success or
change the hardware merge requirements.
