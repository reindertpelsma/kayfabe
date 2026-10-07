# Windows memory and COPY2 allocation decoding

**STATUS: RESEARCH, 2026-10-07.** Owner-requested investigation, not an implementation
or Windows-success claim. Borrowed RTX4070, host595.91.07; Windows580.88 with the
580.65.06 public guest ABI. Run14 product `b928ab4c5fdf3e847f50978df56016ca76140921`
adds a bounded diagnostic only. The fresh watchdog has the same 24 saved
assertions as run13 (including first changed hint `0x1a103e6`); Code43/smi9 persist.
[Trace](run14-qemu.log.gz), [request excerpt](run14-requests.log),
[command](run14-command.json), [status](run14-status.json),
[dump comparison](run14-watchdog-comparison.json), [recovery](run14-recovery.log).
Run15 at `b6336577b5e9628f60a7a543518426ac9637a5f1` additionally records
Device/TSG allocation facts and establishes the missing VA-share relation.
[Trace](run15-qemu.log.gz), [request excerpt](run15-requests.log),
[command](run15-command.json), [status](run15-status.json),
[completion](run15-complete.json), [host health](run15-host-health.txt).
Both owned VMs exited; no QEMU/NBD process remains and the host GPU is healthy.
No host display configuration, physical driver detach or guest success substitution.

## ALLOC_MEMORY: a framebuffer MemoryList registration, not malformed SYSRAM

Function4 is `NV_VGPU_MSG_FUNCTION_ALLOC_MEMORY`, wire `rpc_alloc_memory_v13_01`.
Offsets exclude the RPC envelope. The existing compiler-derived source audit is
[here](../../tools/windows-debug-capture/evidence/alloc-memory-20261005/README.md).

| Offset | Field | Captured run14 value |
|---:|---|---|
| 0 | hClient | `0xc1d00002` |
| 4 | hDevice (Device/Subdevice parent) | `0xff030000` |
| 8 | hMemory | `0xcaf0000c` |
| 12 | hClass | `0x82`, `NV01_MEMORY_LIST_FBMEM` |
| 16 | NVOS02 flags | `0x48040200` |
| 20 | pteAdjust | `0` |
| 24 | format / PTE kind | `0`, pitch-linear |
| 32 | logical length | `0x20000`, 128 KiB |
| 40 | transmitted pageCount | `1` |
| 48 | pteDesc bitfields | `0x00010000`: IDR_NONE=0, reserved=0, length=1 |
| 56 | inline PFN | `0xea6e0` |

Bytes28..31,44..47,52..55 are C padding. The two captured raw u64 words include
this padding; it is not given invented semantics. PFN shift12 describes the
FB range `[0xea6e0000,0xea700000)`. One PFN describes a contiguous span of32
4KiB pages; it does not limit the allocation to one page. The request does not
ask us to chase a host pointer or an indirect guest table.

[Compiled flags](compiled-flags.json), from exact public commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`, decode:

- PHYSICALITY=CONTIGUOUS, LOCATION=VIDMEM, COHERENCY=UNCACHED.
- GPU_CACHEABLE=YES, REGISTER_MEMDESC_TO_PHYS_RM=TRUE, MAPPING=NO_MAP.
- Remaining NVOS02 fields are zero, including ALLOC; zero ALLOC is not ALLOC_NONE=1.

The source producer `rpcAllocMemory_v13_01()` sends an existing memory descriptor
and its physical-page description to the GSP. `memlistConstruct_IMPL()` selects
ADDR_FBMEM for class82, builds a video-memory descriptor and performs FB heap/
resource accounting for that described range. It is a memory-object operation;
it is not a LUT color transform or channel submission. NO_MAP concerns the CPU
mapping policy; GPU_CACHEABLE concerns GPU caching. The firmware's FB object
construction is not interchangeable with the existing checked SYSRAM contract.

The kayfabe failure is exact and local: `kf_abi::memory_list::Cell::decode()`
requires class81, PCI location and an allowed-field mask excluding GPU_CACHEABLE.
This request violates all three (outside-mask bits `0x40000`). It returns None;
`Bridge::deliver()` answers NOT_SUPPORTED before calling `objects.memory_list()`.
The coarse diagnostic "unsupported or malformed descriptor" does not establish
malformation. The captured direct descriptor's shape is valid for the public
compressed-contiguous producer. Supporting this requires a bounded guest-FB
registration path with ownership, carveout, flags and lifetime semantics, not
passing the guest PFN to the host RM or simply returning success.

## COPY2 GPFIFO allocation

Function103 is `GSP_RM_ALLOC`; class `0xc56f` is `AMPERE_CHANNEL_GPFIFO_A`.
The class does not mean that this is a graphics engine channel; engineType selects
COPY2 (`NV2080_ENGINE_TYPE_COPY2=0xb`) in this request.

| Declared fact | Run14 value / meaning |
|---|---|
| Client, channel | `0xc1d00012:0xff040000` |
| Parent TSG | `0xff0e0000`, successful class `KEPLER_CHANNEL_GROUP_A=0xa06c` allocation |
| Parent Device | `0xff010000` |
| GPFIFO VA | `0x12024b000` |
| Entries | `0x4000` = 16,384; 8-byte GPFIFO entries imply a 128-KiB ring |
| flags | `0x200120` |
| hVASpace, hContextShare | both zero |
| USERD | SYSMEM base `0x239871000`, size `0x200` (512 bytes) |
| decoded internal privilege | `2`, KERNEL; uvm_owned=false |
| notifier | none in decoded request |

Flags decode to PRIVILEGED_CHANNEL=TRUE, USERD_INDEX_VALUE=1,
USERD_INDEX_PAGE_VALUE=0, USERD_INDEX_PAGE_FIXED=TRUE,
USERD_INDEX_FIXED=FALSE. This combination requests an index within the specified
USERD page and is permitted by the public flag comments. Other fields are zero,
including DELAY_CHANNEL_SCHEDULING, DENY_PHYSICAL_MODE_CE and MAP_CHANNEL.
A GPFIFO entry names a pushbuffer span; USERD carries CPU/GPU queue control state.
The channel provides an asynchronous GPU copy-engine queue, not a display UPDATE.
Guest KERNEL privilege does not authorize a privileged host channel.

`ChannelPolicy` tries explicit hVASpace, context-share VA, TSG VA, then device
`vas_under`, then a unique page-directory-stated VA in this client. All failed in
this request; `ChannelPlane::birth()` returns INVALID_STATE=0x40 before host
channel creation, mirror lookup, scheduling or GPU execution. No VA object or
page-directory statement for client `0xc1d00012` occurs before this allocation
in the complete trace. No host channel was attempted for the failing request.

In OGKM, zero hVASpace can select the parent/group/device default. The helper
`vaspaceGetByHandleOrDeviceDefault_IMPL()` rejects implicit VA only in
MULTIPLE_VASPACES mode; other modes use `deviceGetDefaultVASpace()`. That lazily
calls `deviceInitClientShare()`: share zero attaches global VA, share self creates
a new VA, and another client shares that client's device VA.

Run15's bounded, source-decoded Device allocation facts identify that branch:

| Device allocation field | Value / meaning |
|---|---|
| Client / Device | `0xc1d00012:0xff010000` |
| hClientShare | `0xc1d00002` |
| hTargetClient / hTargetDevice | `0xc1d00002:0xff010000` |
| flags | `0x38`: VASPACE_SIZE, MAP_PTE, VASPACE_IS_TARGET |
| vaSpaceSize | `0x10000000000`, 1 TiB |
| vaMode | `0`, OPTIONAL_MULTIPLE_VASPACES; implicit default is permitted |
| TSG hVASpace / engineType | `0` / `0xb`, COPY2 |

Client2 already declares default FERMI_VASPACE_A `0xff000870` under its Device.
The request explicitly shares that client's Device default VA. Kayfabe's
translation retains only device_id, dropping Device-share/target/mode facts;
its channel resolver searches the allocating client's namespace and misses
this existing default. This is a concrete missing ownership/resolution path,
not evidence of a malformed channel or a need to guess a VA handle. A repair
must retain validated Device relationships and resolve the canonical VA while
preserving ownership, lifetime/revocation and mirror readiness.

The failed framebuffer object belongs to `0xc1d00002`; the failed channel belongs
to `0xc1d00012`. There is no direct same-handle namespace dependency established.
Run15 establishes a Device VA-share relationship between those clients, but
that is not a dependency on memory handle `0xcaf0000c`. Their equal 128-KiB
extents do not prove they are the same buffer.

## Adjacent query and causality limit

Control `0x0080170e` immediately precedes the COPY2 setup and returns0x56.
It is `NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE`: a 12-byte structure with
engineID input and gpEntries/pbEntries outputs; pbEntries are in32-byte rows.
The public VF implementation searches the static per-engine buffer-size table.
The request's parameter values were not captured by this run's control census,
so no particular input engineID or returned size is inferred from its name.
The subsequent TSG allocation and `0xa06c010a` fault-method-buffer promotion
both return0; then the channel allocation fails. This sequence does not prove
which failure produced the saved Windows assertion. In particular, the assertion
is a local precondition, not an RPC status, and no live locals were recovered.
The previous description of its tested 64-bit argument as a "pointer" was too
specific: its semantic type has not been established.

## Reproduction

```sh
python3 tools/windows-debug-capture/decode-request-flags.py \
  --source /var/tmp/kf-ogkm-windows-decode-580 \
  --memory-flags 0x48040200 --channel-flags 0x200120 \
  --out /tmp/request-flags.json
```

The legacy `prefix_only=true` log label remains; the appended
`raw_descriptor_u64_le` field is the bounded two-word observation described below.
Macro scanning selects identifiers only; the C compiler supplies masks, shifts
and enum values. Headers and compiler identities are recorded in the artifact.
The one-inline-entry observer logs exactly two u64 words only when the declared
payload is the compiled fixed record plus8 bytes; larger/shorter/indirect tails
are never followed. Observer tests cover unchanged requests, no admission and
short/extended/budget cases. Host driver595 is not used to derive guest flag values.
Raw watchdog/Windows driver binaries remain private. The recovered NVCD payload
is one byte short; complete records compare, but no valid-checksum claim is made.

## Source anchors and validation

All OGKM anchors below are at public commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9` (580.65.06):

- `src/nvidia/src/kernel/vgpu/rpc.c:3436`: `rpcAllocMemory_v13_01` producer.
- `src/nvidia/src/kernel/mem_mgr/mem_list.c:46`: `memlistConstruct_IMPL`.
- `src/nvidia/src/kernel/mem_mgr/vaspace.c:183`: implicit/default VA resolution.
- `src/nvidia/src/kernel/gpu/device_share.c:91`: Device client-sharing semantics;
  line324: lazy default initialization.

Validation: 13 observer tests and five census tests pass. Formatting and diff
checks pass; targeted Clippy reports zero new debt (45 existing). Documentation
claims report zero new debt (3150 existing). The standalone compiler decoder
reproduces `compiled-flags.json` byte-for-byte. These checks validate diagnostics
and decoding; they do not constitute a Windows initialization pass.
