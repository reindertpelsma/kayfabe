# `GPU_PROMOTE_CTX` (`0x2080012b`) — ABI, protocol, and why it is not the compute path

**Status:** design, not built. The tree at `ca9e4ae` contains no promote-ctx decoder and no
consumer. This document is the fact-establishment and the build plan; §4 and §5 are the two
blockers that stopped a code drop.

**Citation tags** follow `mode2_gsp_port_plan.md` §0.1:

| tag | tree |
|---|---|
| `ogkm-580:` | `nvidia-gpu-passthrough/research_clones/ogkm-580.159.04/` — **580.159.04**, the bench's own driver |
| `ogkm-610:` | `nvidia-gpu-passthrough/research_clones/ogkm/` — **610.43.02** |
| `[src]` | verified at **both** tags and identical |
| `C:` | `nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c` — evidence (measured on GA106 + 580), never the protocol |
| `rs:` | this repo |

---

## §0 ★★ SCOPING CORRECTION — read this before `gsp_core_bridge.md` §2.7

`gsp_core_bridge.md` §2.7 concludes, correctly, that `RmEvent::MapMemoryDma` has no producer on a
GSP client, and therefore that

> **`GPU_PROMOTE_CTX` is not an optional extra — it is the only address-populating RPC there is.**

That sentence is true and it is routinely over-read. It has been over-read at least once into
*"promote-ctx is the gap between the current tree and `cuCtxCreate → first compute`."* **It is
not.** The correction, from three sources:

1. **The host owns and self-maps the ranges promote-ctx describes.** When the GR object alloc is
   forwarded (Case-1), the host kernel-RM allocates its **own** GR context buffers, issues its
   **own** `PROMOTE_CTX` entirely in-kernel, and maps them at the *same deterministic GR VAs the
   guest uses* (`0x120020000…`) — `rs: docs/design/execution_plane.md:209-217`. The core
   "neither stores nor forwards any host-physical address." So the `gpuPhysAddr` in a guest
   promote entry is a **guest** FB offset for a buffer the host never touches.

2. **For the client that actually mattered, it populated nothing.**
   `nvidia-gpu-passthrough/docs/design/mode2_cuctxcreate_resume.md:210-213`: the GR compute client
   `0xc1d00003` — the one that crashed — promoted **every** context buffer NONMAPPED with `va=0`.
   Under any correct filter, the address table receives zero entries from it.

3. **The compute working set has a different source, already wired.**
   `nvidia-gpu-passthrough/docs/design/mode2_address_table.md:116-129` (the 2026-07-22 audit-S3
   correction) measured, on the Mode-2 GSP-emulated compute path, `INVALIDATE_TLB` RPC = 0,
   `MMU_TLB_INVALIDATE` method = 0, `DMA_FILL_PTE_MEM` = 0. The working set's leaf PTEs are
   published **exclusively** through the CE page-table-write data plane — which
   `rs: crates/kayfabe-fwd/src/lib.rs:2103-2134` already captures and already binds into
   `kayfabe_mmu::AddressTable`.

**What promote-ctx is actually worth.** Under MISS = FAULT, a resolve of a GR context-buffer VA
with no binding faults. The C's table was consulted *first* by `nvkvm_chan_translate`
(`C:5168-5180`) and that is why GR/compute channels resolved at all
(`nvidia-gpu-passthrough/docs/design/mode2_address_virtualization.md:132-147`). So promote-ctx is a
**MISS=FAULT gap-filler for host-owned GR context ranges**: necessary, narrow, and nowhere near
sufficient.

> **Standing statement for the next reader.** §2.7's "the only address-populating RPC" is a claim
> about the *set of RPCs*, not about the *set of populate sources*. There are two co-equal
> sources; the other one is not an RPC. Do not derive a milestone from §2.7 alone.

---

## §1 The ABI facts

### 1.1 ★ Byte-identical across both tags — do not build a version profile

`[src]` The params struct is identical at both tags — `ogkm-580: src/common/sdk/nvidia/inc/ctrl/
ctrl2080/ctrl2080gpu.h:988-1000` and `ogkm-610: …/ctrl2080gpu.h:959-971` — field for field,
alignment for alignment. So is the entry struct (`ogkm-580:922-930`, `ogkm-610:893-901`). So is
`NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES = 16U` (`ogkm-580:946`, `ogkm-610:917`).

And so are **both producer functions**, diffed whole and byte-identical between the trees:
`kgrctxPreparePromoteCtxBuffer_IMPL` and `kgrctxPrepareInitializeCtxBuffer_IMPL`
(`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_context.c`, same functions in
`ogkm-610:`).

> ★★ **This is a load-bearing negative result.** `NV_CHANNEL_ALLOC_PARAMS` diverges at +32 inside
> the supported range, which is why the house rule is *assume nothing until you have checked both
> tags*. Both tags were checked here and they agree. **A version profile / seam for promote-ctx
> would be inventing a seam that does not exist** (`gsp_core_bridge.md`'s Axis-A rule cuts both
> ways: a version-split fact is a seam, and a non-split fact must not become one). If a future
> reader is tempted to add a `MapDmaWire`-style per-version field for this struct, the answer is
> in this paragraph: checked at 580 and 610, identical, do not.
>
> The claim is scoped to the supported range. Re-check on a new tag; do not re-check by adding a
> seam pre-emptively.

### 1.2 Pinned offsets

Pinned by compiling the vendored declarations with `offsetof`/`sizeof` under the real
`NV_DECLARE_ALIGNED` semantics, not by hand arithmetic.

```
NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS — sizeof = 560, align = 8
  +0    NvU32    engineType
  +4    NvHandle hClient
  +8    NvU32    ChID
  +12   NvHandle hChanClient
  +16   NvHandle hObject
  +20   NvHandle hVirtMemory
  +24   NvU64    virtAddress
  +32   NvU64    size
  +40   NvU32    entryCount
  +44   (4 bytes tail padding — promoteEntry is 8-aligned)
  +48   NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY promoteEntry[16]   (512 bytes)

NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY — sizeof = 32, align = 8
  +0    NvU64 gpuPhysAddr
  +8    NvU64 gpuVirtAddr
  +16   NvU64 size
  +24   NvU32 physAttr
  +28   NvU16 bufferId        ★ TWO bytes, not four
  +30   NvU8  bInitialize
  +31   NvU8  bNonmapped
```

Corroborated independently by `C:2276-2279`'s own offset comment and by the captured host RPC
(`nvidia-gpu-passthrough/docs/research/captures/ga106_initctrl_580.log:2422`) recording
`cmd=0x2080012b … psize=560`.

### 1.3 Wire form: flat, unserialized, `ROUTE_TO_PHYSICAL`, single record

- **Not FINN-serialized.** Absent from `ogkm-580: src/common/sdk/nvidia/inc/g_finn_rm_api.h`.
  `rpcRmApiControl_GSP` (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:10948-11020`) therefore takes
  the non-serialized branch: `portMemCopy(rpc_params->params, …, pParamStructPtr, paramsSize)` — a
  **flat 560-byte memcpy**.
- **`ROUTE_TO_PHYSICAL`.** `ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:310-322` gives
  `methodId = 0x2080012bu`, `paramSize = sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS)`, and
  `flags = 0x10244` = `GSP_PLUGIN_FOR_VGPU_GSP(0x10000) | ROUTE_TO_VGPU_HOST(0x200) |
  ROUTE_TO_PHYSICAL(0x40) | PRIVILEGED(0x4)`
  (`ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:202,233,253,290`). Because
  `ROUTE_TO_PHYSICAL` is set, CPU-RM compiles the implementation out entirely
  (`NVOC_EXPORTED_METHOD_DISABLED_BY_FLAG(0x10244u)` → `pFunc = NULL`) — there is **no**
  `subdeviceCtrlCmdGpuPromoteCtx_IMPL` anywhere in the open kernel modules. It exists only in GSP
  firmware, i.e. in the thing we are faking.
- **Not the vGPU `_v1A_20` path.** `rpcCtrlGpuPromoteCtx_v1A_20` (`ogkm-580: rpc.c:6410-6438`,
  reached from the `rpcDmaControl_wrapper` switch at `rpc.c:4582`) is the legacy paravirt route. A
  GSP client never takes it. Noted because it is easy to find first and it is a decoy — although
  its serialized layout (`ogkm-580: src/nvidia/generated/g_sdk-structures.h:86-111`) happens to be
  byte-identical to the SDK struct anyway.
- **Never fragments.** `RpcControlReq::HEADER = 40` (`rs: crates/kayfabe-abi/src/view.rs:194`) plus
  560 params = **600 bytes**, against a message-buffer remainder of roughly
  `4096 − 48 (queue-element header) − 32 (rpc_message_header) − 40 = 3976`. `rpcRmApiControl_GSP`
  only calls `_issueRpcAndWaitLarge` when `message_buffer_remaining < paramsSize`, so promote-ctx
  is always a **single record**. The reassembler (`rs: crates/kayfabe-rmrpc/src/reasm.rs`) is not
  on this path.

  > This corrects a plausible-looking wrong inference: *"16 entries, so the blob is ~4 KB and will
  > fragment."* The array is 16 × **32** = 512 bytes, not 16 × 256. The capture says `psize=560`.

### 1.4 `physAttr`

`ogkm-580: ctrl2080gpu.h:1095-1102` — the fields live in the `INITIALIZE_CTX` namespace and are
reused verbatim by promote-ctx (`ogkm-580: kernel_graphics_context.c:1814-1843` sets them with
`FLD_SET_DRF(2080, _CTRL_GPU_INITIALIZE_CTX, _APERTURE, …)`):

| bits | field | values |
|---|---|---|
| `1:0` | `APERTURE` | `0 = VIDMEM`, `1 = COH_SYS`, `2 = NCOH_SYS`, **`3` = undefined** |
| `2:2` | `GPU_CACHEABLE` | `0 = YES`, `1 = NO` (always set to `NO` by the producer) |
| `3:3` | `PRESERVE_CTX` | `0 = NO`, `1 = YES` |

This maps **exactly onto `kayfabe_arch::Aperture`** (`rs: crates/kayfabe-arch/src/lib.rs:179-188`):
`VIDMEM → Vidmem`, `COH_SYS → SysmemCoherent`, `NCOH_SYS → SysmemNonCoherent`. Value `3` has no
`Aperture` and **must be refused by name**, not folded into sysmem.

Note the two aperture vocabularies are not yet connected: `kayfabe_abi::view::PdbAperture`
(`rs: crates/kayfabe-abi/src/view.rs:419-471`) and `kayfabe_arch::Aperture` have no `From` impl in
either direction, and every production `Binding.aperture` today is a hard-coded constant. P2/P4
below is the first site that derives one from wire bits.

---

## §2 ★★ The two-pass protocol, and the three legitimate wire states

### 2.1 Two preparers, one entry slot

`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:90-124` — `kgrobjPromoteContext`
zeroes the whole params struct and then, **for each buffer id, runs two preparers into the same
entry slot**:

```c
    portMemSet(&params, 0, sizeof(params));
    ...
        kgrctxPrepareInitializeCtxBuffer(..., promoteIds[i], &params.promoteEntry[entryCount], &bInitialize);
        if (bAttemptPromote)
            kgrctxPreparePromoteCtxBuffer(..., promoteIds[i], &params.promoteEntry[entryCount], &bPromote);
        if (bInitialize || bPromote)
            entryCount++;
```

The initialize preparer (`kernel_graphics_context.c:1843-1849`) writes:

```c
    pEntry->gpuPhysAddr = memdescGetPhysAddr(pMemDesc, AT_GPU, 0);
    pEntry->size        = pMemDesc->Size;
    pEntry->physAttr    = physAttr;
    pEntry->bufferId    = externalId;
    pEntry->bInitialize = NV_TRUE;
    pEntry->bNonmapped  = NV_TRUE;
```

— and **never touches `gpuVirtAddr`**. The promote preparer
(`kernel_graphics_context.c:1949-1955`) writes **only three fields**:

```c
    pEntry->bufferId    = externalId;
    pEntry->gpuVirtAddr = vaddr;
    pEntry->bNonmapped  = NV_FALSE;
```

— and **never touches `gpuPhysAddr`, `size`, `physAttr`, or `bInitialize`**. Either preparer may
decline (`*pbAddEntry = NV_FALSE`) and return without writing anything: the initialize pass
declines once `bKGr*CtxBufferInitialized` is set, the promote pass declines when the VA-list
refcount is `> 1` or the VAS is externally owned (UVM).

### 2.2 The three states

Because the struct is pre-zeroed, an entry on the wire is exactly one of:

| # | state | `gpuPhysAddr` | `size` | `gpuVirtAddr` | `bInitialize` | `bNonmapped` | bindable? |
|---|---|---|---|---|---|---|---|
| A | **initialize-only** | set | set | **0** | 1 | **1** | no — declares no VA, and `bNonmapped` says so explicitly |
| B | **promote-only** | **0** | **0** | set | 0 | 0 | **no** — `AddressTable::bind` with `len == 0` returns `AddressFault::Malformed` |
| C | **both** | set | set | set | 1 | 0 | **yes** — the only complete mapping |

State B arises when a buffer was initialized against one channel/VAS and is newly mapped into
another — the multi-channel-TSG case the `PRESERVE_CTX` doc paragraph
(`ogkm-580: ctrl2080gpu.h:1104-1112`) describes.

### 2.3 ★★ `gpuPhysAddr == 0 && size == 0` means "not supplied", not physical address zero

This is a **fourth reading of zero**, distinct from the three the project has already settled
(verbatim on edge fields; `None` on params fields; refused on `hVASpace`). Here:

> In a **promote-only** entry, `gpuPhysAddr == 0` and `size == 0` are the *pre-zeroed initial
> values of fields this pass does not write*. They are **absence of a fact**, not a fact.

Consequences, both mandatory:

- **Never bind VA → phys `0`.** That is manufacturing an address, which is precisely what
  MISS = FAULT forbids (`mode2_address_table.md` §6; `mode2_address_table_of_truth.md`:
  *"no backwards/heuristic resolve"*). A promote-only entry must be **classified and counted**, not
  bound and not silently dropped.
- **Never treat `size == 0` as malformed input.** It is a well-formed, expected, protocol-legal
  entry. A refusal here would reject legitimate guest traffic.

The type system helps: `AddressTable::bind` (`rs: crates/kayfabe-mmu/src/lib.rs:162-176`) maps a
zero length to `AddressFault::Malformed`, so state B is *structurally* unbindable. That makes
dropping it forced — which is exactly why it must be an explicitly named outcome rather than a
`continue`, or the forced behaviour will read as an intentional decision it isn't.

### 2.4 Empirical confirmation — the repo's own captured blob

The C's canned-response table holds a real 560-byte capture
(`nvidia-gpu-passthrough/src/qemu/mode2_initctrl_ga106.h:3279-3315`, indexed at `:6237`). Decoded:

```
engineType=1 (GRAPHICS)  hChanClient=0xc1e00009  hObject=0xbaba0045  entryCount=9
 [0] bufId=0  MAIN         phys=0x2ef946000 va=0x120020000 sz=0xea000 physAttr=0x4  → state C
 [1] bufId=2  PATCH        phys=0x2efa40000 va=0x12010a000 sz=0x4000  physAttr=0x4  → state C
 [2] bufId=3  BUFFER_BUNDLE_CB   phys=0 va=0x120190000 sz=0        → state B
 [3] bufId=4  PAGEPOOL           phys=0 va=0x1201a0000 sz=0        → state B
 [4] bufId=5  ATTRIBUTE_CB       phys=0 va=0x121000000 sz=0        → state B
 [5] bufId=6  RTV_CB_GLOBAL      phys=0 va=0x1201c0000 sz=0        → state B
 [6] bufId=9  FECS_EVENT   phys=0x107900000 va=0x120010000 sz=0x10000 physAttr=0x5 (COH_SYS) → state C
 [7] bufId=10 PRIV_ACCESS_MAP    phys=0x2ef820000 va=0 sz=0x80000 bNonmapped=1     → state A
 [8] bufId=11 UNRESTRICTED_PRIV_ACCESS_MAP phys=0x2eed80000 va=0x120110000 sz=0x80000 → state C
```

**4 promote-only, 1 initialize-only, 4 complete.** Five of nine entries are structurally
unbindable — predicted from `ogkm` source, confirmed by a real capture, and matching the four
mappings the C's design notes report as its entire captured side-table
(`nvidia-gpu-passthrough/docs/design/mode2_compute_forwarding.md:342-345`). Three independent
oracles agree.

Note also that every complete entry is a **GR context buffer the host owns and self-maps** — which
is §0 restated from the data.

---

## §3 The C's seven defects

`C:2275-2306` is the handler; `C:2249-2273` the sink; `C:2422-2438` the dispatch. **Its offsets are
correct** and its `cmd+120` independently corroborates `RpcControlReq::HEADER = 40` (48-byte queue
element header + 32-byte `rpc_message_header` + 40 = 120). Port the offsets and the sequence.
Subtract the following. Defects 1, 2, 3, 4, 6 and 7 were **previously unnamed** anywhere in either
repo; 5 was already on the refactor plan.

### D1 — ★ SECURITY: `entryCount` clamped to 64; the truth is 16 *(previously unnamed)*

```c
    if (ec > 64) {              /* NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES=20;
                                 * clamp generously, never trust guest count. */
        ec = 64;
    }
```

`C:2285-2288`. Three numbers, all different, and the comment's `20` and the code's `64` are both
wrong: the constant is **16** (`[src]`, `ogkm-580:946`, `ogkm-610:917`). A guest declaring
`entryCount` in `17..=64` makes the loop read `params+560 … params+2096` — **1536 bytes past the
560-byte struct**, out of the guest-writable 4096-byte queue element. Every 32-byte window there
with a nonzero `va`/`sz` and `bNonmapped == 0` is inserted into `va_map[]` under the guest's own
choice of `hChanClient` — and `nvkvm_chan_translate` (`C:5168-5180`) consults `va_map[]` **first**,
before the PDB walk. That is a guest-controlled VA → arbitrary-physical redirect on the hot
address-resolution path. No `paramsSize` (`cmd+96`) validation is performed anywhere in the
handler. (There is no memory-safety violation of the QEMU process — the reads stay inside the
4096-byte stack buffer — the defect is semantic injection, which is worse here.)

The dead host-forward `nvkvm_m2_forward_promote_ctx` (`C:7344`, `G_GNUC_UNUSED`) clamps to `20`,
also wrong.

**Port as:** refuse `entryCount > 16` by name, and validate the declared `paramsSize` against
`560` *exactly* before touching a single entry.

### D2 — `bufferId` read 32-bit over a 16-bit field *(previously unnamed)*

`C:2293`: `uint32_t bufferId = ldl_le_p(e + 28);` — but the field is `NvU16 bufferId` at +28 with
`bInitialize` at +30 and `bNonmapped` at +31 (`[src]`). The value therefore carries
`bufferId | (bInitialize << 16) | (bNonmapped << 24)`. The handler's **own comment two lines up**
(`C:2278`) states the correct layout, so this is a transcription slip, not a misunderstanding.

Not cosmetic: `mode2_cuctxcreate_resume.md:210` records a human reverse-engineering the packing
back out of the emulator's log (*"low byte = type; `0x0001xxxx`=mapped, `0x0101xxxx`=NONMAPPED"*).
An analysis artefact was shaped by an ABI bug. Additionally the C **never stores** `bufferId`, so
its table cannot distinguish MAIN from PATCH from PRIV_ACCESS_MAP — which
`mode2_compute_forwarding.md:411-417` says is needed to identify the double-mmap targets.

**Port as:** `u16` at +28, and **keep** it in the decoded view.

### D3 — `!sz` silently swallows every promote-only entry *(previously unnamed)*

`C:2301`: `if (!va || !sz || bNonmapped) continue;`. The `!va` and `bNonmapped` arms are
protocol-correct (state A declares no VA and says so). The `!sz` arm is not a filter on malformed
input — it is state **B**, a distinct and legitimate protocol case, discarded without a name or a
count. In the captured blob that is 4 of 9 entries.

**Port as:** an explicit three-way classification (§2.2) with each non-bindable outcome named and
counted. The *behaviour* (no bind) is forced by `AddressTable`; the *silence* is the defect.

### D4 — aperture collapsed to a bool; illegal value 3 accepted *(previously unnamed)*

`C:2304`: `(physAttr & 0x3u) != 0` → `bool sys`. This discards the COH_SYS/NCOH_SYS distinction and
maps the undefined value `3` to sysmem. `kayfabe_arch::Aperture` models the distinction
(`rs: crates/kayfabe-arch/src/lib.rs:179-188`), so porting this collapse would be a **strict
regression** against the Rust core's own vocabulary.

**Port as:** total decode of `physAttr[1:0]` into `Aperture`, with `3` a named refusal.

### D5 — client-keyed, not PDB-keyed *(already named)*

`C:299-314` keys `va_map[]` on `hChanClient`; `C:5176` matches on `s->chan_client`. This is the
anti-pattern `mode2_address_table.md` §13 identifies as the root cause of #12, and
`mode2_multiprocess_refactor_plan.md:245` already lists it for re-keying to PDB.
`kayfabe_mmu::AddressTable` is per-`Vas` keyed `(GpuId, Pdb)` per `Proc`
(`rs: crates/kayfabe-core/src/gpu.rs:292-296`), so the Rust structure fixes this by construction —
provided §5's resolution step is done and not shortcut.

### D6 — silent table-full drop *(previously unnamed)*

`C:2264-2266` returns silently at 1024 entries (*"table full — DoS-bounded; oldest stay"*): no log,
no fault. A binding we failed to capture must be loud (`mode2_address_table.md:181-196`).
`AddressTable` has no capacity limit, so this does not port; recorded so the C's comment is not
mistaken for a design.

### D7 — ★ SECURITY: the reply clobbers the guest's params with a foreign-boot capture *(previously unnamed)*

`0x2080012b` is present in the canned-response table (`mode2_initctrl_ga106.h:6237`,
`psize = dlen = 560`) but has **no dedicated case** in the reply builder's `else if` chain
(`C:2883-3216`). It therefore falls into the generic replay at `C:3198-3210`:

```c
} else if (cr && (120u + cr->psize) <= NVKVM_RESP_MAX) {
    memset(resp + 120, 0, cr->psize);
    memcpy(resp + 120, cr->data, cr->dlen);
    stl_le_p(resp + 92, cr->status);   /* NV_OK */
```

Because `cr->psize == 560 == req_psize`, the M9 over-size clamp (`C:3241-3249`) never fires. Every
`PROMOTE_CTX` reply therefore overwrites the guest's own params buffer with the **hard-coded
capture from a different machine and a different boot** decoded in §2.4 — stale
`hChanClient=0xc1e00009`, stale `hObject=0xbaba0045`, stale VAs, stale FB physicals — and reports
`NV_OK`. The guest CPU-RM then reads `params.promoteEntry[i].bInitialize` back out of that foreign
blob to decide which buffers to mark initialized (`kernel_graphics_object.c:141-157`), so the
clobber feeds guest state, not just guest memory.

This also falsifies the M6.5 comment at `C:2429-2438`, which asserts the control is "ack-only to
the guest."

**Port as:** a Case-2 ACK writes back **nothing** (or the guest's own bytes unchanged). Never
replay a captured payload into a caller's buffer.

---

## §4 Blocker 1 — the generator cannot express this struct

`kayfabe-abi` is codegen-first, and `promoteEntry[16]` is **a fixed array of a nested struct** —
the same shape that caused `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`0x90f10106`) to be
deferred as `ControlParams::PageDirNotModelled` with the note that its *"184-byte struct ending in
a six-element array of 24-byte level records … needs the generator and a `RUSTC_OFFSETS` pin"*
(`rs: crates/kayfabe-abi/src/versions.rs:999-1005`). **The brief's suspicion was correct: this is
the same dependency, not a detail.**

The refusal is deliberate, in two places:

- `rs: crates/kayfabe-abi/gen/src/ctype.rs:49-53` — *"The complete scalar table. **Deliberately
  closed.** An unrecognised type is a hard error, never a guessed width — the L11 bug class
  (`abi_struct_truncation`, `nvos64_abi_fix`) is precisely what a guessed width produces."* Array
  fields are arrays of a `Scalar`.
- `rs: crates/kayfabe-abi/gen/src/parse.rs:36-38, 62-66` — `ParseError::NestedAggregate`,
  *"nested struct/union body — not supported, add it explicitly."*

The flexible-array-member path (`T name[]`, mirror-omitted, manifest `fam_align`) is for genuinely
open tails and does not model a fixed `[16]`.

### 4.1 The decomposition that works today

**Generate the entry; hand-transcribe the header.**

- `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY` is **all scalars** (`NvU64` ×3, `NvU32`, `NvU16`,
  `NvU8` ×2). The generator can emit it **unchanged, today**, with its full pinning stack: the
  `#[repr(C)]` mirror, `SIZE`/`ALIGN`, the generator's own `LAYOUT`, rustc's `RUSTC_OFFSETS` via
  `offset_of!`, and the compile-time `const _: () = { assert!(…) }` block. That is where the risk
  actually lives — the mixed `u32`/`u16`/`u8`/`u8` tail is exactly what D2 got wrong.
- The 48-byte params **header** is nine scalar fields; transcribe it into
  `rs: crates/kayfabe-abi/src/transcribed.rs`, which exists for precisely this purpose (currently
  holding only `Nvos46ParametersPre580`) and carries its own `LAYOUT` + `RUSTC_OFFSETS` pair.
- The array is then **stride arithmetic**: `HEADER + i * Entry::SIZE`, bounded by `min(entryCount,
  16)` *after* the `paramsSize == 560` check.

This keeps the generator off the critical path while leaving nothing hand-computed that a compiler
could have checked. Extending the generator to support struct-typed fields is the right long-term
fix — it would also unblock `0x90f10106` — but it is a change to the artefact that *guards* the
L11 truncation bug class, and it should not ride along with a decoder.

### 4.2 Knock-ons

`rs: crates/kayfabe-abi/tests/oracle_layout.rs` hard-codes three things that all move:

- `:114` — `assert_eq!(checked_structs, 13, "the slice is 13 generated structs")`
- `:118` — `assert_eq!(checked_fields, 89, "…with 89 fields between them")`
- `:730-765` — `every_generated_struct_is_covered_by_an_oracle_assertion`'s literal typedef list

These are features, not friction: the test's own comment records that the first draft of the count
line was wrong and the test caught it.

Two more assertions flip from negative to positive:
`rs: crates/kayfabe-abi/tests/mean_wire.rs:1839-1850` (`control_params(0x2080012b) == None`) and
`rs: tests/tests/rmrpc_bridge.rs:1091` (`BridgeRefusal::UnknownControl { cmd: 0x2080_012b }`).

---

## §5 Blocker 2 — the consumer is a new seam, on top of in-flight lock work

The classification already exists and already routes promote-ctx correctly:
`kayfabe_fwd::classify_control` (`rs: crates/kayfabe-fwd/src/lib.rs:1799-1808`) returns
`ControlRoute::AckOnly` for `Arch::is_case2_control`, and `mock_ctrl::PROMOTE_CTX`
(`rs: crates/kayfabe-mocks/src/lib.rs:313`) is already in the mock's Case-2 set. **Nothing today
harvests facts from an `AckOnly` control**, and that is the seam.

`DeviceHandle::route_control` (`rs: crates/kayfabe-rt/src/device.rs:1294-1311`):

```rust
        {
            let ack = match self.mode {
                LockMode::Sharded => kayfabe_fwd::classify_control(&self.state.read().spine, cmd),
                LockMode::Degenerate => kayfabe_fwd::classify_control(&self.state.write().spine, cmd),
            };
            if let ControlRoute::AckOnly = ack {
                return Ok(ControlRoute::AckOnly);
            }
        }
```

The Case-2 arm returns **under the read lock, before any `Proc` is touched**. Binding needs:

1. `&mut Proc` — a write-side act phase, for a command that is currently a read-lock fast path;
2. a resolved `(GpuId, Pdb)` — which requires taking `hObject` (a **channel or TSG** handle, per
   `ogkm-580: ctrl2080gpu.h:962-968`; both callers pass a channel handle, at
   `kernel_graphics_object.c:131` and `kernel_graphics_context.c:2166`, but the SDK contract admits
   a group) in the `hChanClient` namespace, through the `RmGraph` to `Channel.vas_pdb`
   (`rs: crates/kayfabe-core/src/project.rs:747`, via `pdb_of_resource` at `:713`).

`route_control`'s signature carries neither: its `obj: HostHandle` is a handle in the *isolate's*
namespace, and its `pid: ProcId` is already resolved upstream. The guest `hClient`/`hObject` needed
for step 2 are not in scope there.

**This lands directly on top of the R1 (no blocking under lock) / R3 (lock rank) / R5 (revalidate)
invariants that L1-M1 is mid-build.** Converting a read-lock fast path into a route/act/commit
sequence for one command is a lock-discipline change, and it should be designed against the L1-M1
shape rather than merged ahead of it. That, more than §4, is why this is a design and not a patch.

The bind itself is then the established pattern, the CE-capture arm at
`rs: crates/kayfabe-fwd/src/lib.rs:2111-2134`:
`proc.vases.get_mut(&(cgpu, pdb))` → `vas.table.bind(pdb, va, len, Binding { phys, aperture, host:
None })`. `host: None` is exactly right and its doc says why: *"declared by the RPC/CE-capture
source only — nothing host-side exists yet, and nothing host-side needs reclaiming"*
(`rs: crates/kayfabe-mmu/src/lib.rs:88-92`) — which is precisely the status of a GR context buffer
the host allocated and mapped for itself.

Two gates constrain the implementation:
**bridge-exclusivity** (`.github/workflows/ci.yml:483-510` — only `kayfabe-rmrpc` may name both
`RpcCommand` and `RmEvent`; `kayfabe-fwd` naming one is fine, and it may take a `kayfabe-abi`
dependency, which it does not have today), and the **generation-name gate** (`:257-288` — no
concrete chip or driver constant in `kayfabe-fwd`/`kayfabe-mmu`; `kayfabe-abi` is exempt, which is
where all the NVIDIA constants must therefore live).

---

## §6 Two traps, and a recurring pattern

### 6.1 ★ Do not reuse `Vas::rpc_bound` — it will reap every promote-ctx binding

`Spine::sync_rpc_mappings` (`rs: crates/kayfabe-core/src/gpu.rs:1282-1317`) builds its `desired`
set **exclusively** from `self.rmgraph.mappings()`:

```rust
        let mut desired: BTreeMap<(GpuId, u64, u64), (u64, u64)> = BTreeMap::new();
        for m in self.rmgraph.mappings() { ... desired.insert((gpu, pdb.0, m.va.0), (m.len, phys)); }
```

and `sync_proc_rpc_bindings` (`:1321-1362`) unbinds every VA in `Vas::rpc_bound` that is **not** in
it. Promote entries are not `RmEvent::MapMemoryDma` mappings and never will be (§0, §5), so a
promote-ctx binding placed into `rpc_bound` would be silently unbound on the **very next**
`Spine::apply`. The failure mode is a table that is correct immediately after the control and empty
a moment later — the kind of bug that reads as a race.

**Promote-ctx bindings need their own idempotence set on `Vas`** (mirroring `rpc_bound`'s role for
the RPC source), or must resolve on the `resolve`-then-`bind` pattern the CE arm uses
(`rs: crates/kayfabe-fwd/src/lib.rs:2122`). They must not enter `rpc_bound`.

### 6.2 The `hChanClient` rule needs a **scope**, not enforcement

`gsp_core_bridge.md` §3.2 and `rs: crates/kayfabe-rmrpc/src/lib.rs:34-46` state:

> The namespace is **always** the RPC body's own `hClient`. Never a params field. Never inferred.

and name the C's promote-ctx handler as *the* counter-example, because `C:2283` reads `hChanClient`
from `params+12` and never looks at the envelope's client. There is a test pinning that reading
(`rs: tests/tests/rmrpc_bridge.rs:4995`).

**Per `ogkm`, the C is right to read `hChanClient` for the channel lookup.** The two clients are
deliberately independent: `kernel_graphics_object.c:130-135` sets
`params.hChanClient = RES_GET_CLIENT_HANDLE(pChannelDescendant)` while the very next statement
issues the control with `RES_GET_CLIENT_HANDLE(pSubdevice)` as the **envelope** client. They are
usually equal and are not required to be. The correct formulation separates two jobs:

| job | source | why |
|---|---|---|
| **namespace attribution** — which client's component a fact lands in | the **envelope's** `hClient` | a params field naming a different client would be a silent cross-namespace substitution |
| **object resolution** — which client's handle space `hObject` is a handle *in* | `hChanClient` | `hObject` is documented as living in `hChanClient`'s namespace (`ogkm-580: ctrl2080gpu.h:958-968`), and RM populates it accordingly |

So promote-ctx is not a violation of the rule; it is a case the rule's phrasing does not cover. The
C's actual defect is the *unprincipled* part — it reads `hChanClient` without ever looking at the
envelope, so it cannot notice a disagreement, let alone refuse one (compare §2.2a's treatment of
the client root, which compares both and refuses a mismatch).

> **★ The recurring pattern.** This is the second time this rule has needed a scope rather than
> enforcement — `DUP_OBJECT` broke *"the client is always the envelope's `hClient`"* the same way,
> and B5 had to scope it rather than reverse it. When a params field names a client, the question
> is never "envelope or params?" but **"attribution or resolution?"** Expect a third case; write
> the next such rule with the two jobs already distinguished.

---

## §7 Build plan P1–P4, with per-stage test strategy

Consumer-first: `kayfabe-abi` gains nothing that no one reads. **P4 lands with P2 or neither
lands** — an accessor with no consumer is dead API, and the bite-check will (correctly) say so.

### P1 — the entry struct, through the generator

Add a `StructReq` for `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY` to the slice manifest
(`rs: crates/kayfabe-abi/gen/src/main.rs`), regenerate `src/generated/ctrl.rs`. Remember
`rm -rf crates/kayfabe-abi/gen/target` before running the gates.

**Tests.** The pinning stack is mostly automatic: the compile-time `offset_of!` block, plus the
`oracle_layout.rs` loop comparing the generator's `LAYOUT` against rustc's `RUSTC_OFFSETS`, plus
the three count/coverage literals from §4.2. Add a fourth oracle assertion against the C artefact's
independently-transcribed entry offsets (`C:2278`, `e+0/8/16/24/28/31`), and — because D2 is the
live risk — write the `bufferId` check as a **negative**: an entry with `bInitialize = 1` and
`bNonmapped = 1` must decode to the *same* `bufferId` as one with both zero.

**Bite reading.** A survivor on a `RUSTC_OFFSETS` row is a **missing test** (the oracle loop should
cover it — check the count literals were actually raised). A survivor on the field-width read is
**wrong code**.

### P2 — the params header, the decoder, the control table

Hand-transcribe the 48-byte header into `transcribed.rs` with its own `LAYOUT`/`RUSTC_OFFSETS`. Add
`ControlParams::PromoteCtx` with `params_size() == Some(560)` — checked **exactly**, per
`gsp_core_bridge.md` §4.3, not as a lower bound. Add `DriverAbiTable::decode_promote_ctx` returning
a domain view in `view.rs`:

```
PromoteCtx { engine_type, h_chan_client, h_object, entries: ArrayVec<PromoteEntry, 16> }
PromoteEntry -> Promotable { va, len, phys, aperture, buffer_id }
             | InitializeOnly { phys, len, aperture, buffer_id }
             | PromoteOnly    { va, buffer_id }
```

Refuse by name: `entryCount > 16`; `physAttr[1:0] == 3`; `paramsSize != 560`. Flip the two negative
assertions from §4.2. Do **not** interpret the legacy `hVirtMemory` / `(virtAddress, size)` path —
both real callers leave those zero and `entryCount > 0`; refuse the legacy shape rather than guess
it.

**Tests.** Three independent transcriptions, per the house discipline
(`rs: tests/tests/rmrpc_bridge.rs:7-24`): the import-nothing builder in `tests/src/rpcwire.rs`
written from the header offsets; hand-written offset-annotated hex; the decoder. Assert **exact
variants** and **by exact content, never by count** — an assertion that "3 entries decoded" is
worthless here, because the whole point is *which* three. **Sweep, do not witness**: `entryCount`
over `0..=17`, and the state-A/B/C cross-product across at least two buffer ids, rather than one
fixture per state. Plus the length sweep every decoder gets (refuse short at every length below
`SIZE`, accept long).

**Bite reading.** A survivor on the `entryCount > 16` arm is **wrong code** (D1 reintroduced). A
survivor on an `InitializeOnly`/`PromoteOnly` construction is a **missing test** — the sweep did
not reach that state. A survivor on `aperture == 3` refusal is **wrong code**.

### P3 — classification (pure)

The `PromoteEntry` three-way split as a free function over decoded entries, with counts for the
non-bindable outcomes. Pure, no locks, no `Proc` — fully visible to the mutation gate, and the
place where §2.3's "not supplied" reading is enforced once rather than at every call site.

**Tests.** Property-shaped: for every entry, exactly one variant; `Promotable` implies all four of
`va != 0`, `len != 0`, `!bNonmapped`, and a decodable aperture; `bNonmapped` implies never
`Promotable` **regardless of** whether `va` happens to be nonzero (a hostile guest can set both).

**Bite reading.** Any survivor here is a **missing test** — this stage has no I/O and no
concurrency, so there is no other explanation.

### P4 — the consumer

Resolve `hObject` (channel **or** TSG) in the `hChanClient` namespace → `(GpuId, Pdb)`; bind each
`Promotable` into `vas.table` with `host: None` and its **own** idempotence set (§6.1). Design the
write-side act phase against L1-M1's lock invariants, not around them. `InitializeOnly` and
`PromoteOnly` are counted and dropped — named, never silent (D3).

**Tests.** A MEAN integration test through the mocks, per
`memory/mean_integration_testing_bar.md`: multi-proc × identical guest VAs across procs × the full
state cross-product × re-promote of an already-bound buffer × proc retire mid-sequence. Two
assertions carry the traps:

- promote-ctx bindings **survive a subsequent `Spine::apply`** (the `rpc_bound` reaping trap — this
  test is the only thing that would catch it, and it fails loudly if someone reuses the set);
- two procs' identical guest VAs land in **two distinct tables**, and neither resolves the other's
  phys (#14).

Plus: a Case-2 ACK writes back nothing (D7); a `PromoteOnly` entry never produces a binding **and**
never produces an `AddressFault` (it must be classified out before `bind`, not rejected by it —
otherwise a legal guest sequence returns a fault).

**Bite reading.** A survivor on the resolution step is **wrong code** if it silently picks a PDB, a
**missing test** if the MEAN scenario has only one VAS. A survivor on any accessor added in P2 that
P4 does not read is **dead API** — delete the accessor, do not add a test for it.

### Throughout

Bite-check with `--no-fail-fast`, report every non-biter, carry a statefulness canary; run
`scratchpad/mbite.py` backgrounded, remembering its four known parser limits (`>` → `>=`, duplicate
`OLD` on a line, nested generics in `NEW`, an `OLD` ending in `=>`). Finish line unchanged:
`cargo test --workspace --no-fail-fast` green both ways and `./scripts/ci_gates.sh --all` =
ALL GATES CLEAN.

---

## §8 Provenance

Everything above was verified against primary sources, not transcribed from prior notes:

- **Struct layouts:** read from both vendored trees and **pinned by compiling** the declarations
  with `offsetof`/`sizeof` under real `NV_DECLARE_ALIGNED` semantics.
- **Cross-tag identity:** the two producer functions diffed whole between `ogkm-580` and
  `ogkm-610`; both diffs empty.
- **Wire form:** `g_subdevice_nvoc.c` flags decoded against `rmapi/control.h`; the absence from
  `g_finn_rm_api.h` checked directly; the single-record property computed from
  `rpcRmApiControl_GSP`'s own branch condition.
- **The three wire states:** derived from `ogkm` source, then confirmed independently by decoding
  the repo's captured 560-byte blob, then cross-checked against the four mappings the C's design
  notes report.
- **Rust-side facts:** read from the tree at `ca9e4ae` (691 tests green, `ci_gates.sh --all` =
  ALL GATES CLEAN, 14 steps).
