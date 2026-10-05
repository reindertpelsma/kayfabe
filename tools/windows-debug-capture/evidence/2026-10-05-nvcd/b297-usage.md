# Class 0xb297: exact-driver successful-use audit

**STATUS: RESEARCH, 2026-10-05.** Offline analysis of the trusted Windows 580.88
driver ties the failing allocation to a scheduling resource used by the Windows
kernel. This is a contract investigation, not an emulation implementation. No
host forwarding or success stub is authorized by these observations.

The driver hash, dump bounds and allocation-to-StartDevice failure chain are in
[the parent evidence](README.md). All RVAs below refer to that same file. These
binary offsets must never become product layout constants or per-GPU tables.

## Allocation, identity and lifetime

The owner object's vtable is at RVA `0xe93398`; its initialization callback at
`0x19c2c30` allocates two class `0xb297` resources under a subdevice. Each has a
distinct RM handle, stored in the owner at offsets `0x1d4` and `0x1d8`. In this
driver the second handle is the first plus `0x200`. These values are private
driver handle choices, not identifiers Kayfabe should recognize specially.

The allocation at `0x19c2d77` passes a parameter block whose first DWORD comes
from a per-instance descriptor at offset `0x20`; the following eight bytes are
zeroed. The second allocation at `0x19c2ec3` reuses the block. Both are required
for initialization to return true. The RM resource descriptor at RVA `0x1135a40`
independently records external class `0xb297`, internal NVOC ID `0x00f4b771`,
12-byte required parameters, multiple instances allowed, and a parent list
containing only public NVOC `Subdevice` ID `0x4b01b3` plus its zero terminator.

The descriptor's flags are `0xa153`. Public OGKM580
`src/nvidia/src/kernel/rmapi/resource_desc_flags.h` assigns bit8 to
`RS_FLAGS_ALLOC_NON_PRIVILEGED`; the admin/kernel-only allocation bits are clear.
This is registration evidence, **not a successful unprivileged probe** and not
proof of constructor/operation permissions. It does not justify forwarding this
resource or any associated control.

Public `nvos.h:2914–2935` defines the matching 12-byte
`NV_SWRUNLIST_ALLOCATION_PARAMS` as `engineId`, `maxTSGs`, and
`qosIntrEnableMask`. Its comments describe software-runlist double buffers and
hardware-format entries. This is a strong contract candidate supported by the
successful-use path below; this note does not assert a public class-ID definition
that was not found. The completed [independent proprietary Linux audit](../runlist-20261005/README.md)
corroborates the identity and engine namespace. It establishes that zero
`maxTSGs` requests native default capacity and that allocation may create two
hardware buffers; metadata-only acceptance is an incomplete diagnostic.

Cleanup callbacks at `0xe059d0` and `0xe05a40` free each nonzero handle separately
and clear the stored handle. The latter also cleans up the memory/event objects
created after these allocations. Allocation and free alone do not characterize
the resource's scheduling effects.

## Successful-use path

The two handles alternate through a state machine. Helpers at `0xe04930` and
`0xe041e0` put one of those handles in the first DWORD of a zeroed 40-byte
submission record. A separate memory handle is written at byte8, a 16-bit
offset computed as twelve times a capacity at byte16, and a boolean at byte24.
Helper `0xe054c0` builds records with a twelve-byte stride in that memory.
The remaining meanings are not inferred from field widths alone.

The common submit routine at `0x19c4400` invokes subdevice control **`0x20801111`**
with that 40-byte record (`0x19c45ec` selects the command). A rejected call returns
`STATUS_INSUFFICIENT_RESOURCES`. A separate owner routine issues control
**`0x20801110`** with eight bytes. Neither ID is defined in the inspected public
OGKM580 headers. Later OGKM610 source explicitly names `0x20801110`
`NV2080_CTRL_CMD_FIFO_CONFIG_CTXSW_TIMEOUT`; it configures context-switch timeout,
not a runlist submission. The proprietary Linux audit separately identifies
`0x20801111` as the scheduling operation. These findings do not provide a complete
implementation contract.

Initialization after the class allocations also allocates class `0x90cd`, then
calls **`0x20801231`** with twenty bytes. The public header names this
`NV2080_CTRL_CMD_GR_FECS_BIND_EVTBUF_FOR_UID` (`ctrl2080gr.h:1776`). This is an
additional capture target, not evidence that event delivery may be fabricated.

## Independent native traffic

The older successful native Windows capture already contains twelve request/reply
pairs for `0x20801111` (all40 bytes, all status0), despite its missing initial
allocation prefix. Source:
[`gsp.jsonl.gz`](../../../windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/gsp.jsonl.gz),
decompressed SHA256
`9d75e47b8ec5dd837dc64e14847779d62a16f14d92169ede8f9b62681b93de29`.
The native D3D11 capture contains another four pairs.

| Record offset | Observed value(s) | Supported interpretation |
|---|---|---|
| `+0`, DWORD | `0xff008250` or `0xff008254` | Matches the second-handle family from the class allocation formula |
| `+4`, DWORD | `0` through `3` | Meaning unresolved |
| `+8`, DWORD | `0xff000100` or `0xff000104` | Matches the separate memory handle constructed by the same owner |
| `+12`, DWORD | `1` through `7` | Meaning unresolved; equals the DWORD at+20 in these samples |
| `+16`, WORD | `0x6000` | Matches twelve times2048 from the observed builder |
| `+18`, WORD | zero | Meaning unresolved |
| `+20`, DWORD | `1` through `7` | Meaning unresolved |
| `+24`, byte | one | Matches the caller's boolean |
| `+25` through `+39` | zero | Does not establish that these bytes are always padding |

All replies preserve their request's complete parameter bytes. Request queue
sequences are `2914,2957,3088,3103,3151,3167,3281,3314,3474,3489,3493,3495`.
These are observations from a single driver/die capture, suitable for constraining
research and cross-checking source. They are not a supported input language.

## Implementation questions still open

The apparent runlist operation can affect real submitted GPU work. A response
must preserve per-channel lifetime, scheduling order, preemption and completion
semantics; success may not merely discard a list. Any implementation needs to
resolve every guest handle through the guest's typed ownership graph, check all
counts/offsets against the referenced guest memory, and author only admitted
unprivileged per-twin host verbs. Guest-supplied pointers, global physical host
runlists and guest bytes must not be forwarded. The native trace and proprietary
Linux handler are being examined before choosing such a design.
