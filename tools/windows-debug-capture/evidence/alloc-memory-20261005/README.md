# GSP ALLOC_MEMORY: guest system-memory registration

**STATUS: RESEARCH, 2026-10-05.** Public-source contract and compiler measurements;
no product change or new hardware result. This is not a Windows-only operation.

The Windows 580.88 experiment I at Kayfabe revision `302f6c9a` reported function 4,
class `0x81`, flags `0x48002000`, `length=0x7000`, `pageCount=1`,
`pteAdjust=format=0`, and a 64-byte payload. That is consistent with registration
of an existing contiguous 28 KiB system-memory span. It is not a request to
allocate one 4 KiB page. The subsequent `0x20801111` request had a null memory
handle and zero count; it is consistent with cleanup after registration failed,
not evidence that an empty scheduling request should succeed. See the separate
[scheduling audit](../runlist-20261005/scheduling-control.md).

## Evidence and limits

The principal behavioral oracle is public OGKM 580.65.06, commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`. The exact sources are linked below.
The Windows observation selects the investigation; it supplies no product
constant or GPU-die table. The public sender and shared memory-list constructor
are available; this audit has not found or claimed the complete GSP-side RPC
dispatcher that translates this wire record to allocation parameters. It also
does not establish unprivileged host-callability or runtime Linux coverage.

[compiler-layouts.json](compiler-layouts.json) records 13 explicit public tags,
their dereferenced commits, source Git blob IDs, compiler identity, measured
offsets, bitfield masks, constants, and reviewed flag fields. The
[C probe](../../alloc-memory-layout.c) compiles the declarations, initializes
the real bitfields, and copies their bytes. No C values or layouts are parsed
with regular expressions. Its [runner](../../inspect-alloc-memory.py) uses each
tag's own build include environment via the existing driver-matrix tool. Only
our probe executable is run; no NVIDIA binary is executed.

Reproduce from the repository root using the local public-source Git clone:

```sh
python3 tools/windows-debug-capture/inspect-alloc-memory.py \
  --ogkm-git /workspace/ogkm-full \
  --out /tmp/alloc-memory-layouts.json
```

Measurements target little-endian x86-64. They establish these declared wire
facts, not firmware behavior on every OS/driver/family combination. GPU family
and die do not enter the inspected producer's encoding decisions. A product
implementation still needs explicit guest-driver rows and architecture-valid
guest RAM authority; a measured layout must not imply a supported behavior.

The older runlist evidence called annotated tag IDs `15b1a21d...` (535.309.01)
and `86856f77...` (610.43.02) public commits. Those identify the correct source
trees through tag dereference; the actual commits are `9756a4df...` and
`57130a27...`. This new manifest distinguishes `tag_object` and `commit`.

## Wire record and page meaning

Offsets are relative to the RPC payload, excluding the common RPC header.
All 13 measured tags have this layout for `rpc_alloc_memory_v13_01` and its
current alias. Declaration:
[g_rpc-structures.h](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/generated/g_rpc-structures.h#L79).

| Offset | Width | Field | Meaning |
|---:|---:|---|---|
| 0 | 4 | hClient | Client owning the registration |
| 4 | 4 | hDevice | Parent handle; public class permits Device or Subdevice |
| 8 | 4 | hMemory | New memory-object handle in that client's namespace |
| 12 | 4 | hClass | `NV01_MEMORY_LIST_SYSTEM=0x81` for this case |
| 16 | 4 | flags | NVOS02 fields, not NVOS32 fields |
| 20 | 4 | pteAdjust | Byte offset of data within first page |
| 24 | 4 | format | Memory kind; zero is the bounded linear case audited here |
| 32 | 8 | length | Logical byte size, not number of supplied PFNs |
| 40 | 4 | pageCount | Number of transmitted entries in the newer GSP path |
| 48 | 4 | pteDesc bitfields | idr bits 1:0; reserved1 bits 15:2; length bits 31:16 |
| 56 | 8 each | pteDesc.pte_pde[] | PFNs when idr is NONE |

The fixed record is 56 bytes; `struct pte_desc` is eight bytes before its
flexible array. Bytes 28–31, 44–47, and 52–55 are C alignment padding, not
declared reserved fields. Padding must not acquire an invented zero-value
requirement. The actual reserved bitfield is separate. `idr` values
NONE/SINGLE/DOUBLE/TRIPLE compile to 0/1/2/3. The public descriptor says entries
are PTEs for idr zero and PDEs otherwise; this audit supports only direct PFNs,
not following guest-supplied indirection trees.
[sdk-structures.h](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/inc/kernel/vgpu/sdk-structures.h#L199)

The newer [`_issuePteDescRpc`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/vgpu/rpc.c#L2230)
sets idr NONE, descriptor length to the supplied PTE count, and each entry to
`guestPages[i] >> RM_PAGE_SHIFT`. The compiled shift is 12 and entry width is
8. Thus an entry is a 4 KiB PFN, not a byte address, a CPU pointer, or an NVIDIA
hardware PTE with permissions. Contiguity comes from the flags, not a bit in
the PFN or descriptor. The logical contiguous data span is
`[(PFN << 12) + pteAdjust, start + length)` with checked arithmetic.

[`rpcAllocMemory_v13_01`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/vgpu/rpc.c#L3436)
uses `memdescGetPteArraySize(..., AT_GPU)` for GSP; the source explicitly sends
one PTE when contiguous. [`memdescGetPteArraySize`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/gpu/mem_mgr/mem_desc.c#L3237)
returns one for contiguous AT_GPU memory and PageCount otherwise. Therefore
`pageCount=1`, descriptor length 1, and 64 payload bytes are correct for this
28 KiB span. Multi-entry messages can use the large-RPC continuation path;
support for the single-entry subset must not silently parse a truncated list.
The sender waits for status and requests no bidirectional payload data. It
does not use a returned address or returned PFN array.

## Driver compatibility: identical layout, different behavior

This table is a source review, not inference from the compiler layout. The
flag mask is the union of fields measured by this audit, not a permission to
accept all field values or all effects.

| Public tag | Reviewed NVOS02 field mask | GSP producer for contiguous memory | Relevant receiver source |
|---|---|---|---|
| 515.43.04 | `c1fffff0` | Expanded PFN per physical page | mem_list.c not in this published tree |
| 535.309.01 | `c7fffff0` | Expanded PFN per physical page | SYSRAM constructor rejects bContig |
| 550.90.12 | `c7fffff0` | Expanded PFN per physical page | SYSRAM constructor permits bContig |
| 560.35.03 | `cffffff0` | Expanded PFN per physical page | Not separately audited here |
| 565.57.01 | `cffffff0` | Actual PTE array, one for contiguous | Explicit GSP CPU mapping through memdescMap |
| 570.86.15 | `cffffff0` | Actual PTE array, one for contiguous | GSP CPU mapping for pageCount 1 |
| 580.65.06 | `cffffff0` | Actual PTE array, one for contiguous | Detailed constructor audit below |
| 580.126.09 | `cffffff0` | Actual PTE array, one for contiguous | GSP CPU mapping for pageCount 1 |
| 580.159.04 | `cffffff0` | Actual PTE array, one for contiguous | GSP CPU mapping for pageCount 1 |
| 580.173.02 | `cffffff0` | Actual PTE array, one for contiguous | GSP CPU mapping for pageCount 1 |
| 595.91.07 | `cffffff0` | Actual PTE array, one for contiguous | Updated page-array/actual-size checks; GSP mapping remains |
| 610.43.02 | `cffffff0` | Actual PTE array, one for contiguous | GSP CPU mapping for pageCount 1 |
| 615.71.09 | `cffffff0` | Actual PTE array, one for contiguous | Updated page-array/actual-size checks; GSP mapping remains |

The precise public change is
[565.57.01 commit d5a0858f](https://github.com/NVIDIA/open-gpu-kernel-modules/commit/d5a0858f901d15bda4c3d6db19a271507722a860):
the helper drops `physicallyContiguous`, stops generating `contigBase+i`, and
the GSP caller switches from PageCount to actual array size. This is a positive
source change, not a capture-derived version threshold. Do not generalize the
table to unreviewed tags, backports, or every proprietary Windows version.
Bit 27 alone is not a behavioral gate: it already exists in 560.35.03, whose
producer still expands the array. Compared with 580.65.06, the inspected 595,
610 and 615 constructors additionally check pageCount against pageArraySize
and calculate aligned actual size; identical wire offsets do not erase these
implementation changes.

## Why format zero is a pitch-only subset

This is a hardware PTE kind: the RPC producer reads `memdescGetPteKind`, and
the receiver calls `memdescSetPteKind` with format. It is not a Windows-specific
format number. The public
[`memmgrGetPteKindPitch_GM107`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_mgr_gm107.c#L1131)
explicitly returns the kind of pitch-linear surfaces using the published
`NV_MMU_PTE_KIND_PITCH` definition. The compiler probe now measures that constant
from the exact header included by this HAL, separately at every recorded tag;
it is zero in all 13 cells.

Although the HAL function retains a GM107 suffix, the
[generated dispatch](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/generated/g_mem_mgr_nvoc.c#L855)
selects it for the supported discrete/datacenter families, including Turing,
Ampere, Ada, Hopper and Blackwell; its exception is Tegra. The published
TU102, GH100, GB202 and GB20B kind headers independently name the same pitch
value. `pitch_header_facts` compiles every published dev_mmu.h separately at
each tag and records whether it declares this symbol and its value; absent
headers or definitions do not prove a family is unsupported. This is shared
source behavior, not a table inferred from individual GPU captures.
Ampere/Ada need not have a separate duplicate pitch definition.

That helper's dispatch itself must not be projected across releases:
565.57.01's generated helper asserts on GH100/GB100/GB102, while 570.86.15
directly aliases the GM107 implementation. The 580 dispatch cited above has
the Tegra exception. The fn4 producer/consumer does not call the pitch-kind
selection helper; it gets/sets the supplied hardware kind, and the published
GH100 header explicitly defines pitch zero even at 565. These observations
justify the restricted encoding, not a claim that every format-selection
helper or surface-allocation path behaves identically across families.

Do not label zero the generic/default kind. `RM_DEFAULT_PTE_KIND` is a separate
software sentinel (`0x100`); the
[TU102 conversion](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/gpu/mem_mgr/arch/turing/mem_mgr_tu102.c#L524)
maps that sentinel to GENERIC_MEMORY, while preserving other kinds. Also do
not substitute the unrelated `NVOS03_FLAGS_PTE_KIND_PITCH` flag enum. The
[system-memory constructor](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/mem_mgr/system_mem.c#L593)
passes the allocation's requested format into the descriptor, so the source
does not prove that every legal SYSRAM registration uses pitch. Accepting only
the compiler-derived pitch value is an explicit restricted contract; other
kinds require their own layout/access semantics.

The 13 ABI measurements contain nine positive compressed-contiguous source
rows. That is sufficient for a diagnostic which admits exactly those rows and
refuses unknown cells; it is not full coverage of Kayfabe's driver matrix.
Extending to the remaining measured driver tags requires both compilation and
behavioral review, because the 560/565 transition demonstrates why matching
headers alone are insufficient. No unsupported cell may inherit the nearest
version's behavior. The registration is wholly guest-side and adds no host RM
verb or die-specific constant, so it adds no dependence on the host driver's
version; native hardware validation is still narrower than source support.

## Flags and native constructor

The observed `0x48002000` decodes through the compiled public definitions as
CONTIGUOUS, PCI/system memory, WRITE_COMBINE, REGISTER_MEMDESC_TO_PHYS_RM true,
and MAPPING_NO_MAP. GPU cacheability, kernel-mapping request, NISO display,
user/device read-only, peer override, syncpoint, and protection fields are zero.
The registration bit is absent from the measured 535 and 550 headers; memory
protection is also absent from the measured 515 header. Unknown bits and invalid
enum values must fail closed under the selected row.
[nvos.h](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/nvos.h#L190)

All six published cache values are recognized by `mem_list.c`: UC, cached, WC,
and WT/WP/WB (the latter three select its cached descriptor mode). The conversion
helper accepts their distinct NVOS02 enum values and rejects unknown values.
WRITE_PROTECT coherency is not the same as the separate USER_READ_ONLY and
DEVICE_READ_ONLY restrictions. A VMM must either preserve the promised semantics
or document and enforce a smaller supported subset; native acceptance alone is
not proof of its cache-policy fidelity.
[mem_list.c](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/mem_mgr/mem_list.c#L104),
[flag conversion](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/interface/deprecated/rmapi_deprecated_utils.c#L350)

[`cl84a0.h`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/cl84a0.h#L37)
defines a descriptor/object referring to already assigned pages, not newly
allocated backing memory. Its `NV_MEMORY_LIST_ALLOCATION_PARAMS` is a separate
native API structure, not this RPC record. In particular its pageNumberList is
a native pointer; forwarding a guest value as that pointer is forbidden.
The [resource row](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/rmapi/resource_list.h#L611)
is multi-instance, permits Device/Subdevice parents, and requires PRIVILEGED
allocation. This is not a candidate for unprivileged host forwarding.

The SYSRAM [`memlistConstruct_IMPL`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/mem_mgr/mem_list.c#L262)
creates a descriptor, marks GUEST_ALLOCATED, records the adjustment/kind, copies
the PFN list, shifts PFNs to addresses, tags the alias allocated, and constructs
the Memory object. It rejects pteAdjust at least RM_PAGE_SIZE and invalid source
object/parent combinations. It can recognize physical PCI BAR ranges and mark
them CPU-only: class SYSRAM by itself therefore does not prove ordinary RAM.
A RAM-only Kayfabe subset must independently reject MMIO, ROM, BAR aliases and
non-RAM holes through trusted VMM memory-region validation.

Most importantly, the public GSP constructor maps registered single-entry
memory read/write. `MAPPING_NO_MAP` suppresses a user mapping; it does not
suppress GSP's CPU access. The call is
`memCreateKernelMapping(pMemory, NV_PROTECT_READ_WRITE, NV_FALSE)`.
[`memCreateKernelMapping_IMPL`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/mem_mgr/mem.c#L257)
maps the object's length and clears bytes only if bClear is true. Here it is
false. There is no buffer initialization write in this inspected path.
The public Linux alias equivalent likewise builds page metadata without
zeroing the backing: [`osAllocPages`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/arch/nvalloc/unix/src/os.c#L925)
uses [`nv_alias_pages`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/kernel-open/nvidia/nv.c#L3187)
for GUEST_ALLOCATED. That is evidence for alias behavior, not a claim that the
closed GSP OS implements Linux's allocation code.

## Lifetime and the minimum honest virtual object

[`memRegisterWithGsp_IMPL`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/mem_mgr/mem.c#L513)
looks up a Memory in the same client, rejects unsupported address spaces and
subdevice memdescs, chooses class81 or82, converts its flags, and sends this RPC.
It marks bRegisteredWithGsp only after NV_OK and avoids registering it twice.
This public Linux-built path is stronger evidence than a Windows capture that
the operation is not intrinsically Windows-specific. Runtime stimulus and
unprivileged reachability remain separate questions.

The corresponding destructor calls `_memUnregisterFromGsp` with the same
client/parent/memory handles using RPC FREE before unmapping its kernel view.
The native Memory destructor then drops/frees the descriptor, with reference
counting and deferred frees for shared descriptors. In Linux's alias backend,
[`nv_free_pages`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/kernel-open/nvidia/nv.c#L3877)
frees alias metadata but skips freeing the physical pages when the guest flag
is set. This object never owns those guest pages.

A correct bounded Kayfabe implementation can represent the object without a
host RM allocation or a GPU submission, provided it actually preserves:

1. A distinct guest-RAM alias kind, in the owning client's validated
   Device/Subdevice subtree, with unique handle, generation/lifetime, quotas
   and rollback on every failed validation. It must not masquerade as VRAM or
   an ordinary host-backed memory allocation in generic consumers.
2. A checked direct-PFN span. For the selected newer contiguous subset:
   exact complete payload size, class81, direct idr, matching one-entry counts,
   reserved bits, supported kind and flags, `pteAdjust < 4096`, nonzero bounded
   length, shift/addition overflow checks, and trusted writable ordinary RAM
   over the entire adjusted span. No CPU pointer or host physical address may
   be derived directly from guest bytes.
3. Access to those same live guest bytes when a supported GSP-side consumer
   needs them, including permission/cache restrictions and synchronization.
   A trusted GPA/length reference with checked, revalidated VMM access can
   represent this view; a disconnected host allocation or a handle-only
   success cannot. Any retained raw mapping needs lifetime protection, and
   migration/hot-unplug/reset cannot leave stale access authority.
4. FREE/parent teardown/reset revoke access and drop references without
   clearing/freeing the guest's backing. Aliases/duplicates need correct
   reference lifetimes or explicit refusal until supported. Any later DMA,
   scheduler, event or GPU consumer needs its own source-audited contract.

Registration itself does not submit GPU work or signal completion. This is
kernel bookkeeping with real guest-memory identity; it does not permit a CPU
executor, forged GPU completion, passthrough UMD interception, privileged host
forwarding, or success from an unimplemented scheduling request. The source
supports implementing this bounded object; it does not support an unconditional
NV_OK stub. Product and hardware verification remain outside this note.
