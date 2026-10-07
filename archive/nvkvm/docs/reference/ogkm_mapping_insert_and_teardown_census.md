# ogkm mapping inserts and their teardown routes — where a GPU VA gets recorded, and how it is removed

> **STATUS — LIVE, 2026-08-13.** Measured read-only against
> `research_clones/ogkm-580.159.04/` (**version-specific: 580.159.04**). All nvoc HAL bindings
> resolved for **GA106 + GSP-client** from the generated dispatch tables, not from the `.c` alone.
> Scope: **RM proper** (`src/nvidia/`) **+ the UVM plane** (`kernel-open/nvidia-uvm/`, §5A)
> **+ the GSP RPC surface** (§2.6). All three lanes folded in.
> ⚠ **Configuration-scoped**: true of a **default bare-metal GA106 GSP client**. Two switches invert
> it — see the box at the end of §7.
> ⊘ **Contains two corrections to this document's own first draft**, both marked inline: §2.5 (RM's
> PTE writes are **BAR2 CPU stores, not CE writes** — which changes what our source (2) can see) and
> §0 (the flat *"zero GSP crossings"* was too strong; five UVM sites cross, four carrying no VA).
>
> Answers the owner's question of 2026-08-12: *"does ogkm issue rm ioctls and then automatically
> insert mappings in its own bookkeeping, if so which ones. iterate them, all sites, so we also
> know the cleanup routes"*.

---

## 0. THE DIRECT ANSWER

**YES.** Every RM operation that establishes a GPU VA records it in RM's own bookkeeping, in up to
four parallel structures at once.

But the count that matters is not the number of insert sites. It is this:

| | count | |
|---|---|---|
| distinct bookkeeping structures | **5 in RM proper + 1 in UVM** (28 UVM rows) | §3, §5A |
| insert call sites across them | **~14 in RM, ~26 in UVM** | §3, §5A |
| **PTE-writing choke points** | **exactly 2** — RM's walker, UVM's walker | §2.5 |
| **insert sites whose GPU VA is visible to a GSP** | ⊘ **ZERO** | §2 |
| GSP-crossing insert sites that carry **no VA** | 5 (UVM only) | §5A |
| CPU-RM-local / UVM-local insert sites | **everything else** | §2 |

★★★★★ **On a GA106 GSP client, no site in this census publishes a GPU VA to the GSP.** The client
RM owns the VA space, owns the mapping bookkeeping, and **writes the GMMU page tables itself**; UVM
owns and writes its own. The GSP is told **where the page tables are** — never what is in them.

⊘ **REFINED after the two sub-lanes returned (this is a correction to my own first draft's flat
"ZERO GSP-crossing").** Five UVM sites *do* cross the wire — `SET_PAGE_DIRECTORY`, `PROMOTE_CTX`,
`RetainChannel`, `DupMemory`→`DUP_OBJECT`, `MemoryAllocSys/FB` (§5A). **Four of the five carry no
GPU VA at all**, and the fifth (`PROMOTE_CTX`) carries a VA **and has no un-promote** (§5A F2). So
the headline holds for *mapping* purposes, but "zero RPCs" was too strong and is withdrawn.

⇒ **This is the structural answer to the owner's *"is it PDB or RM?"* fork: it is RM, and RM is on
the guest side of our boundary.** A GSP emulator that watches only the wire will see **none** of
these mappings. See §2 for the citation chain and §6 for what that means for the CE-operand fault.

⊘ **Do not over-read this.** It says our *populate source (1)* — bind-time RPC/ioctl bindings —
**structurally cannot cover the user-VA mapping plane on a GSP client**. It does **not** by itself
prove the `0x1_20000000` CE source operand is unmapped; a sibling lane's `NV0080_CTRL_CMD_DMA_GET_PTE_INFO`
probe answers that empirically and should be believed over this inference. §6 states the hypothesis
and its falsifier.

---

## 1. ★★★ THE TEARDOWN ROUTES — kernel-guaranteed vs best-effort

This is the half that no empirical probe can supersede, and the half the owner's standing rule
depends on: *"if we need some kind of pinning that we can unpin it or that we have a cleanup
mechanism later."*

### 1.1 The kernel-guaranteed backstop, end to end

There is **one** guaranteed teardown chain, and it is anchored on **file-descriptor release**, not
on anything userspace chooses to do:

```
close(fd)  /  process death  /  SIGKILL
  └─ nvidia_close                          kernel-open/nvidia/nv.c:2208   (.release = nvidia_close, nv.c:245)
      └─ nvidia_close_callback                                    nv.c:2123
          └─ rm_cleanup_file_private
              └─ RmFreeUnusedClients            src/nvidia/arch/nvalloc/unix/src/osapi.c:2914 (call), :453 (def)
                  └─ pRmApi->DisableClients   → rmapiDisableClientsWithSecInfo   rmapi/alloc_free.c:1587
                      └─ serverMarkClientListDisabled            resserv/src/rs_server.c:995
                          └─ serverFreeClient(bDisableOnly=NV_TRUE)          rs_server.c:1019
              └─ serverFreeDisabledClients                        osapi.c:2918 / rs_server.c:1046
                  └─ serverFreeResourceTree                                  rs_server.c:1100
                      └─ clientFreeResourceTree → clientFreeResource_IMPL    resserv/src/rs_client.c
                          ├─ clientUnmapResourceRefMappings   rs_client.c:832   ← CPU mappings
                          ├─ _clientUnmapBackRefMappings      rs_client.c:833
                          ├─ _clientUnmapInterMappings        rs_client.c:836   ← ★ GPU-VA mappings
                          └─ _clientUnmapInterBackRefMappings rs_client.c:837   ← ★ GPU-VA mappings
```

★ **Why this is kernel-guaranteed, in ogkm's own words** — `osapi.c:463-468`, the comment directly
above the loop in `RmFreeUnusedClients`:

> *"The 'nvfp' pointer uniquely identifies an open instance in kernel space and the kernel interface
> layer guarantees that we are not called before the associated nvfp descriptor is closed. We can
> thus safely free abandoned clients with matching 'nvfp' pointers."*

The Linux kernel forces `.release` on every open fd at process teardown, including under `SIGKILL`.
⇒ **Every route in §1.2 that terminates in `clientFreeResource_IMPL` fires unconditionally.**

⚠ **ONE CAVEAT, and it is a real one for a mirror: the free may be DEFERRED.**
`osapi.c:2917-2918` runs the immediate free **only** under `if (!pSys->bUseDeferredClientListFree)`;
otherwise the actual `serverFreeResourceTree` is punted to a worker (`osapi.c:2924`, *"Start the
deferred free callback if necessary"*). ⇒ **Teardown is guaranteed to HAPPEN but not guaranteed to
have happened by the time the fd is closed.** A mirror that keys "unjoin" on fd-close will unjoin
*before* RM does. Key on the observable teardown, or tolerate the window.

### 1.2 Route table — every teardown, marked

| # | What it removes | Teardown site | Reached from | **Guarantee** |
|---|---|---|---|---|
| T1 | `CLI_DMA_MAPPING_INFO` from `VirtualMemory::pDmaMappingList` (btree) | `intermapDelDmaMapping` — `rmapi/mapping_list.c:291`, unlink at `:326` | `virtmemUnmapFrom_IMPL` `mem_mgr/virtual_mem.c:1788` | ★ **KERNEL-GUARANTEED** via T2 |
| T2 | `RsInterMapping` from `pMapperRef->interMappings` | `refRemoveInterMapping` — `resserv/src/rs_resource.c:656`; driver `serverInterUnmapMapping` `rs_server.c:2358` | **(a)** explicit `NV_ESC_RM_UNMAP_MEMORY_DMA` (0x58, `escape.c:640`); **(b)** `_clientUnmapInterMappings` `rs_client.c:836`; **(c)** `_clientUnmapInterBackRefMappings` `rs_client.c:837` | (a) best-effort · **(b)(c) ★ KERNEL-GUARANTEED** |
| T3 | The GMMU PTEs + the VAS mapping-tree node | `gvaspaceUnmap_IMPL` — `mem_mgr/gpu_vaspace.c:2252`; record removed at `:2274` (`_gvaspaceMappingRemove`), PTEs at `:2284/:2289` (`mmuWalkSparsify` / `mmuWalkUnmap`) | `dmaFreeMap` → `dmaUpdateVASpace_*` → `virt_mem_allocator_gm107.c:2373` — called from `virtmemUnmapFrom_IMPL:1773` | ★ **KERNEL-GUARANTEED** via T2 |
| T4 | Whole-VAS sweep: **every** `GVAS_MAPPING` still in a VA block | `gpu_vaspace.c:1763-1772` — `btreeEnumStart` over `pVASBlock->pMapTree`, `gvaspaceUnmap` per node, **looping until the tree is empty** | VAS block free / `gvaspaceDestruct` (`:914-922`, `eheapTraverse` then `eheapDestruct`) | ★★ **KERNEL-GUARANTEED — this is the VAS-teardown backstop** |
| T5 | `VA_LIST` entry for an engine ctx buffer (`ENGINE_CTX_DESCRIPTOR::vaList`, `globalCtxBufferVaList[]`, `*.vAddrList`) | `vaListRemoveVa` — `mem_mgr/vaddr_list.c:430`; driver `kgraphicsUnmapCtxBuffer` `gpu/gr/kernel_graphics.c:2037` (paired with `dmaUnmapBuffer_HAL`) | `shrkgrctxDetach_IMPL` → `kgrctxUnmapBuffers_HAL` `kernel_graphics_context.c:3699`, gated `if (!kgrctxShouldCleanup(...)) return;` `:3691` | ★ **KERNEL-GUARANTEED** (channel free cascade) — **but see the gate below** |
| T6 | The `VA_LIST` container itself | `vaListDestroy` × 10 — `kernel_graphics_context.c:3619-3632` | `shrkgrctxDestructUnicast_IMPL` `:3602` | ★ **KERNEL-GUARANTEED** |
| T7 | VA range reservation in the VAS eheap | `pGVAS->pHeap->eheapFree` — `gpu_vaspace.c:1816`, `:3434` | `gvaspaceFree` / VAS destruct | ★ **KERNEL-GUARANTEED** |
| T8 | `RsCpuMapping` (CPU BAR mapping — **not** a GPU VA; listed for completeness) | `clientUnmapMemory_IMPL` `rs_client.c:877`; sweep `clientUnmapResourceRefMappings` `rs_client.c:1144` | `clientFreeResource_IMPL:832`; also `rs_server.c:1298` | ★ **KERNEL-GUARANTEED** |

★★ **T5's gate is the one to read carefully.** `kgrctxShouldCleanup` resolves **unconditionally** to
`kgrctxShouldCleanup_KERNEL` (macro, `generated/g_kernel_graphics_context_nvoc.h:771` — no chip
variance), whose whole body is `return gpuIsClientRmAllocatedCtxBufferEnabled(pGpu);`
(`kernel_graphics_context.c:2489-2495`). On a GSP client that is **`NV_TRUE`**
(`gpu_registry.c:153-156`, enclosing `else if (IS_GSP_CLIENT(pGpu) || RMCFG_FEATURE_PLATFORM_GSP)`
taken when the `RMClientRmAllocatedCtxBuffer` regkey is absent). ⇒ **On our target the gate is OPEN
and T5 fires.** On a non-GSP monolithic driver it would be closed and physical RM would do the
unmapping instead. **This teardown is configuration-dependent, and the configuration is what makes
it ours.**

### 1.3 What is **best-effort** — do not build on these

| Route | Why best-effort |
|---|---|
| `NV_ESC_RM_UNMAP_MEMORY_DMA` (0x58) — `arch/nvalloc/unix/src/escape.c:640` | A userspace ioctl. **Issues nothing under `SIGKILL`.** |
| `NV_ESC_RM_FREE` on an individual `VirtualMemory` / `Memory` object | Same. |
| UVM's `UVM_UNMAP_EXTERNAL` / `UVM_FREE` | Same class — userspace-initiated. (UVM's own file-release backstop is a separate question; see §7.) |

⇒ ★★★ **This confirms the prior owner ruling, from ogkm's source rather than from inference: an
unjoin must key on the KERNEL's teardown — routes T2(b)/T2(c) via `clientFreeResource_IMPL`, with
T4 (the whole-VAS `pMapTree` sweep) as the backstop — and never on a userspace free.** Both of
those are reachable with no cooperation from the guest process.

---

## 2. ★★★ GSP-CROSSING vs CPU-RM-LOCAL — the decisive column

### 2.1 The switch: `bSplitVasManagementServerClientRm` defaults to TRUE on a GSP client

`src/nvidia/src/kernel/gpu/gpu_registry.c:171-186`. Enclosing condition, quoted in full:

```c
if ((pGpu->bSriovEnabled && !gpuIsWarBug200577889SriovHeavyEnabled(pGpu)) ||
    RMCFG_FEATURE_PLATFORM_GSP || IS_GSP_CLIENT(pGpu))
{
    if (osReadRegistryDword(pGpu, NV_REG_STR_RM_SPLIT_VAS_MGMT_SERVER_CLIENT_RM, &data32) == NV_OK)
        pGpu->bSplitVasManagementServerClientRm = (data32 == ..._ENABLED);
    else
        pGpu->bSplitVasManagementServerClientRm = NV_TRUE;      // ← :181, the default
}
```

The regkey `"RMSplitVasMgmtServerClientRm"` (`interface/nvrm_registry.h:1231`) is not set by
default. ⇒ **`gpuIsSplitVasManagementServerClientRmEnabled()`** (`generated/g_gpu_nvoc.h:5456-5458`,
a plain field read) **is TRUE on a GA106 GSP client.**

### 2.2 What that switch does to `NV_ESC_RM_MAP_MEMORY_DMA`

`virtmemConstruct_IMPL`, `mem_mgr/virtual_mem.c:458-470` — enclosing `if (IS_VIRTUAL(pGpu) || IS_GSP_CLIENT(pGpu))`:

```c
bRpcAlloc = !(gpuIsSplitVasManagementServerClientRmEnabled(pGpu) ||
              (bSriovFull && (bBar1VAS || pVirtualMemory->bFlaVAS)));
```

On a bare-metal GSP client `bSriovFull` is false, so **`bRpcAlloc = !TRUE = NV_FALSE`**, stored at
`:587`. Consequences in `virtmemMapTo_IMPL`:

| line | condition | on GA106 GSP-client | effect |
|---|---|---|---|
| `virtual_mem.c:1421` | `if (!pMemory->bRpcAlloc \|\| gpuIsSplitVas...Enabled(pGpu))` (sysmem/EGM/fabric arm) | **TRUE** | `dmaAllocMap` runs **locally** (`:1426`); mapping registered locally (`:1430`) |
| `virtual_mem.c:1463` | same predicate (`ADDR_FBMEM` arm) | **TRUE** | `dmaAllocMap` runs **locally** (`:1466`); registered at `:1472` |
| `virtual_mem.c:1520` | `if (pMemory->bRpcAlloc)` — call-site guard on `NV_RM_RPC_MAP_MEMORY_DMA` (`:1522`) | ⊘ **FALSE** | ★★★ **NO RPC IS SENT** |
| `virtual_mem.c:1850` | `if (pMemory->bRpcAlloc && ...)` — call-site guard on `NV_RM_RPC_UNMAP_MEMORY_DMA` (`:1863`) | ⊘ **FALSE** | ★★★ **NO RPC ON UNMAP EITHER** |

★★ **It is DOUBLE-blocked, and the second gate is inside the macro itself** — which a call-site-only
sweep would miss. `inc/kernel/vgpu/rpc.h:154-167` (verbatim):

```c
#define NV_RM_RPC_MAP_MEMORY_DMA(pGpu, hclient, ... , status)                  \
    do { OBJRPC *pRpc; pRpc = GPU_GET_RPC(pGpu); NV_ASSERT(pRpc != NULL);      \
        if ((status == NV_OK) && (pRpc != NULL) &&                             \
            !gpuIsSplitVasManagementServerClientRmEnabled(pGpu))               \   // ← second gate
            status = rpcMapMemoryDma_HAL(...);                                 \
        ...
```

Identical gate on `NV_RM_RPC_UNMAP_MEMORY_DMA` (`rpc.h:171-183`). ⇒ **Both the caller and the macro
independently refuse.** The HAL binding *is* live (`rpcMapMemoryDma_v03_00`, installed by
`rpc_iGrp_ipVersions_Wrapup` for ipVersion ≥ `0x03000000`, and GA106's is `0x2B130000`) — so this is
a **runtime** refusal, not a missing implementation. Reading the HAL table alone would say "wired
up"; reading the call site alone would find one gate; only both together give the answer.

⊘ **And a whole family of RPC macros you might grep for are INERT — they expand to nothing.**
`inc/kernel/vgpu/rpc_vgpu.h:36-59` defines `NV_RM_RPC_MAP_MEMORY` (`:40`), `NV_RM_RPC_UNMAP_MEMORY`
(`:41`), `NV_RM_RPC_UPDATE_PDE_2` (`:51`), `NV_RM_RPC_UPDATE_GPU_PDES` (`:59`),
`NV_RM_RPC_DMA_FILL_PTE_MEM` (`:42`), `NV_RM_RPC_ALLOC_VIRTMEM` (`:37`) and others as
`static NV_INLINE void NAME(...) { }`. ⇒ **Finding a call site for one of these is not evidence of
wire traffic.** ⚠ Same class as this campaign's *"a vocabulary presence is not a measured event"* —
here the symbol exists, the call site exists, and **the function body is empty.**

⚠ Note the comments at `:1456` and `:1491` — `// !IS_VIRTUAL(pGpu) && !IS_GSP_CLIENT(pGpu)` —
which annotate the local-mapping branch as *not* taken on a GSP client. **Those comments are stale
with respect to the code they annotate.** The `|| gpuIsSplitVasManagementServerClientRmEnabled(pGpu)`
disjunct was added to both predicates and re-opens the branch. Reading the comment instead of the
predicate inverts the answer. (Same class as this campaign's *"the `.c` you read is not the code
that runs"*, one layer up: **the comment you read is not the predicate that runs**.)

### 2.3 Who writes the page tables — and it is not the GSP

`dmaAllocMapping` binds to **`dmaAllocMapping_GM107`** on GA106: the nvoc dispatch
(`generated/g_virt_mem_allocator_nvoc.c:299-307`) installs the no-op `_46f6a7` variant **only** for
`ChipHal: T234D | T264D` (Tegra), and `dmaAllocMapping_GM107` in the `else`. Chain:

```
dmaAllocMapping_GM107      gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:317
  └─ dmaUpdateVASpace_GF100                                                   :2175
      └─ gvaspaceMap                                                          :2605
          └─ gvaspaceMap_IMPL                    mem_mgr/gpu_vaspace.c:2190
              ├─ _gvaspaceMappingInsert  :2230   ← the bookkeeping insert (§3, family D)
              └─ mmuWalkMap              :2239   ← ★ the MMU walker runs HERE, in CLIENT RM
                  └─ _gmmuWalkCBMapNextEntries_RmAperture   virt_mem_allocator_gm107.c:2034
                      └─ memmgrMemBeginTransfer                               :2078
                          └─ _gmmuWalkCBMapNextEntries_Direct  ← the actual PTE stores :2081
```

★★★ **`mmuWalkMap` is called in client RM with no RPC anywhere in the path. The GSP never sees the
mapping.**

### 2.5 ★★★★★ HOW the PTEs are written — and this CORRECTS my own first draft

⊘ **My first draft said the PTE store is `memmgrMemWrite` (`gmmu_walk.c:813`). That is the
fill/clear path, not the map path, and the distinction turns out to matter enormously.**

The map path selects its transport through `memmgrGetMemTransferType` (`gpu/mem_mgr/mem_utils.c:59-124`).
Verified independently, all three legs:

1. The walker passes `transferFlags = TRANSFER_FLAGS_SHADOW_ALLOC | TRANSFER_FLAGS_SHADOW_INIT_MEM`
   (`virt_mem_allocator_gm107.c:2051`) — ⊘ **`TRANSFER_FLAGS_PREFER_CE` is NOT among them.**
2. `memmgrGetMemTransferType` defaults to `TRANSFER_TYPE_PROCESSOR` (`mem_utils.c:68`). The **CE**
   arm is gated `else if (flags & TRANSFER_FLAGS_PREFER_CE)` (`:80`) — not taken. The **GSP_DMA**
   arm is gated `else if (kbusIsBarAccessBlocked(pKernelBus) && ...)` (`:118-122`) — not taken
   outside Confidential Compute.
3. ⇒ **`TRANSFER_TYPE_PROCESSOR`.** For a vidmem page table that is a **CPU MMIO store through the
   BAR2 window**, followed by `kbusFlush_HAL` + `gvaspaceInvalidateTlb`
   (`virt_mem_allocator_gm107.c:2612-2616`).

★★★★★ **THERE ARE TWO PTE WRITERS AND THEY USE DIFFERENT TRANSPORTS:**

| writer | mechanism | citation | would a CE-write watcher see it? |
|---|---|---|---|
| **RM's walker** (RM-owned VAS) | **CPU stores through BAR2** | `mem_utils.c:68` + `virt_mem_allocator_gm107.c:2051` | ⊘ **NO** |
| **UVM's walker** (externally-owned VAS) | **its own CE pushbuffer** — `uvm_push_begin_acquire(… UVM_CHANNEL_TYPE_MEMOPS …)` then `uvm_pte_batch_single_write_ptes` | `kernel-open/nvidia-uvm/uvm_map_external.c:230-241` | ★ **YES** |

⇒ ★★★ **This is a live correction to how we read populate source (2).** `mode2_address_table.md`
calls source (2) *"the observed CE page-table write"* — that instrument sees **UVM's** writes and is
**structurally blind to RM's**. A mapping established by an RM path is invisible to *both* of our
declared sources: not an RPC (§2.2), and not a CE write (here). ⊘ **If we have been treating "no CE
page-table write observed" as "no mapping was established", that inference is unsound for every
RM-established mapping.** ⚠ Two exceptions that would restore visibility, both narrow:
Confidential Compute (`kbusIsBarAccessBlocked` ⇒ `TRANSFER_TYPE_GSP_DMA`, `mem_utils.c:122`, which
*does* put page-table bytes on the wire), and an explicit `TRANSFER_FLAGS_PREFER_CE` caller.

⚠ **Scope this correctly.** `gvaspaceMap_IMPL:2219` guards the bookkeeping insert with
`if (!flags.bRemap)`; the remap path (MODS compression release, Windows BAR1 clobber) skips the
insert and only rewrites PTEs. Neither applies to us, but a sweep keyed on `_gvaspaceMappingInsert`
would miss remaps.

### 2.4 The GSP-crossing column, per family

| family | structure | GSP-crossing on GA106 GSP-client? | what makes it so |
|---|---|---|---|
| **D** | `GVAS_BLOCK::pMapTree` + the PTEs | ⊘ **CPU-RM-LOCAL** | `mmuWalkMap` at `gpu_vaspace.c:2239`, no RPC in path |
| **A** | `VirtualMemory::pDmaMappingList` | ⊘ **CPU-RM-LOCAL** | `bRpcAlloc == NV_FALSE` ⇒ `virtual_mem.c:1520` false |
| **B** | `RsResourceRef::interMappings` | ⊘ **CPU-RM-LOCAL** | pure resserv bookkeeping, `rs_server.c:2266`; no RPC |
| **C** | `VA_LIST` (engine ctx buffers) | ⊘ **CPU-RM-LOCAL** | `bClientRmAllocatedCtxBuffer == NV_TRUE` (`gpu_registry.c:153-156`) ⇒ **client** RM allocates *and* maps ctx buffers |
| **E** | VAS eheap reservations | ⊘ **CPU-RM-LOCAL** | `pGVAS->pHeap`, `gpu_vaspace.c:588-595` |
| — | **UVM's own page-tree writes** | ⊘ **not even RM-local** | see §5 / §7 |

### 2.6 What DOES cross — and ⊘ the flag sweep that would have missed it

**The GSP learns WHERE the page tables are. It never learns WHAT IS IN THEM.** The messages that
carry the "where":

| control | crosses via | citation | enclosing condition |
|---|---|---|---|
| `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` | **hand-rolled `NV_RM_RPC_CONTROL` in the `_IMPL` body** | `gpu_vaspace.c:4459-4471` | `if (IS_GSP_CLIENT(pGpu) \|\| IS_VIRTUAL(pGpu)) { … NV_RM_RPC_CONTROL(…); return status; }` |
| `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` | same | `gpu/mem_mgr/dma.c:508-520` | `if (IS_VIRTUAL_WITH_SRIOV(pGpu) \|\| IS_GSP_CLIENT(pGpu))` inside `SLI_LOOP_START(SLI_LOOP_FLAGS_BC_ONLY)` |
| `NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY` | same | `dma.c:539-551` | revoke first, then `if (IS_GSP_CLIENT(pGpu) \|\| IS_VIRTUAL_WITH_SRIOV(pGpu))` |
| `NV_RM_RPC_UPDATE_BAR_PDE` | direct macro | `gpu/bus/kern_bus.c:880` | comment: *"Provide the PDE3[0] value to GSP-RM so that GSP-RM can merge CPU-RM's page table to GSP-RM's page table"* |
| `NV0080_CTRL_CMD_DMA_FLUSH` | ★ the **`ROUTE_TO_PHYSICAL` flag** (`0x50048`, bit `0x40` set) | `generated/g_device_nvoc.c:751` | routed by `rmresControl_Prologue_IMPL`, `rmapi/resource.c:252-296` |

★★★ **METHODOLOGY FINDING, and it is the kind that silently produces a wrong census:** the first
three cross the wire **while their `RMCTRL_FLAGS` do NOT contain `ROUTE_TO_PHYSICAL`** —
`COPY_SERVER_RESERVED_PDES` and both `PAGE_DIRECTORY` controls carry flags `0x14004`, bit `0x40`
**clear** (`generated/g_vaspace_api_nvoc.c:196`; `g_device_nvoc.c:886`, `:901`). They reach the GSP
because their `_IMPL` bodies **hand-roll `NV_RM_RPC_CONTROL`**. ⇒ **A sweep keyed on
`RMCTRL_FLAGS_ROUTE_TO_PHYSICAL` would report that the single most important mapping-establishing
control does not cross.** ⚠ Exactly the campaign's *"a guard/sweep is scoped to what it names"* —
and the inverse error is available too: of the twelve `NV0080_CTRL_CMD_DMA_*` controls, **only
`DMA_FLUSH` carries the flag**; `GET_PTE_INFO`, `GET_PDE_INFO`, `INVALIDATE_TLB`, `UPDATE_PDE_2`,
`SET_VA_SPACE_SIZE`, `ADV_SCHED_GET_VA_CAPS`, `GET_CAPS`, `ENABLE_PRIVILEGED_RANGE`,
`SET_DEFAULT_VASPACE` are all **CPU-RM-local**.

★ **The routing dispatch itself**, for the record: `rmresControl_Prologue_IMPL`
(`rmapi/resource.c:252-296`) — note the disjunction pairs **different flags with different worlds**:
`IS_VIRTUAL(pGpu) && ROUTE_TO_VGPU_HOST` (vGPU guest) **vs** `IS_FW_CLIENT(pGpu) && ROUTE_TO_PHYSICAL`
(us). Confusing the two inverts every row.

★★ **Who owns which VA range** — `gpu_vaspace.c:4123-4128`, quoted because it is the contract:
> *"RPC the details of these reserved PDEs to server RM so that server RM can mirror these PDEs in
> its mmu walker state. Any lower level PDEs/PTEs allocated under these top level PDEs will be
> modified exclusively by server RM. Client RM won't touch those."*

⇒ `[vaStartServerRMOwned, vaLimitServerRMOwned]` is GSP-owned; **everything else is CPU-RM-owned**
(split created at `gpu_vaspace.c:598-612`, gated on `gpuIsSplitVasManagementServerClientRmEnabled`
and not BAR/FLA/PMU/HDA/HWPM/PERFMON). And CPU-RM explicitly does **not** program the PDB register:
`gmmu_walk.c:663-668`, `_gmmuWalkCBUpdatePdb` —
`else if (IS_VIRTUAL_WITH_SRIOV(pGpu) || IS_GSP_CLIENT(pGpu)) { /* Noop inside a guest or CPU RM. */ return NV_TRUE; }`.

⚠ **`RMCFG_FEATURE_PLATFORM_GSP == 0` in this whole tree** (`generated/rmconfig.h:260`). Every
`#if RMCFG_FEATURE_PLATFORM_GSP` body here is **dead** — GSP-RM's own source is not in this repo.
⇒ This census tells you what CPU-RM **sends**; it cannot tell you what GSP-RM **does** on receipt.

---

## 3. THE INSERT SITES — bounded enumeration

Small enough to list in full (~14). Grouped by structure, **ranked by the §2 column** — all
CPU-RM-local, so ranked by proximity to the page tables instead.

### Family D — `GVAS_BLOCK::pMapTree` — **THE CHOKE POINT** (1 site)

| trigger | insert | teardown | paired? |
|---|---|---|---|
| Any RM GPU-VA map: `NV_ESC_RM_MAP_MEMORY_DMA`, ctx-buffer maps, BAR1/BAR2 setup, HWPM streamout | `_gvaspaceMappingInsert` — called at `gpu_vaspace.c:2230`, body `:4735`, `btreeInsert` at `:4781` | `_gvaspaceMappingRemove` `:2274` (body `:4824`) **and** the whole-block sweep `:1763-1772` | ★ **YES**, doubly |

★★★ **Every RM-side GPU VA in the driver funnels through this one function.** If we mirror exactly
one thing, mirror this. Its teardown is paired *twice over*: a per-range remove and an
enumerate-until-empty sweep at VAS-block free (T4) — which is precisely the "VAS-teardown backstop"
the owner's ruling asks for, already present in the source we are mirroring.

### Family A — `VirtualMemory::pDmaMappingList` (btree keyed by DMA offset range) — 5 sites

Store: `mapping_list.c:276` `btreeInsert(pNode, &pVirtualMemory->pDmaMappingList)`; node keyed
`[RM_ALIGN_DOWN(dmaOffset, alignment) .. RM_ALIGN_UP(dmaOffset+size, alignment)-1]` (`:241-243`).
Read back by `CliGetDmaMappingInfo` (`:391`) — **the known-positive for this sweep, and it is
reachable**: used on the error-notifier path at `gpu/fifo/kernel_channel.c:2033`, and at
`:4027`, `disp/disp_sw.c:146`, `mem_mgr/method_notification.c:312,581`,
`gr/kernel_sm_debugger_session_ctrl.c:118`, `rmapi/nv_gpu_ops.c:5915`.

| # | insert site | when | teardown | paired? |
|---|---|---|---|---|
| A1 | `virtual_mem.c:1430` | sysmem / EGM / fabric map | `:1603` on error; `:1788` on unmap | ★ YES |
| A2 | `virtual_mem.c:1472` | vidmem (`ADDR_FBMEM`) map | same | ★ YES |
| A3 | `virtual_mem.c:1541` | after `NV_RM_RPC_MAP_MEMORY_DMA` — **dead on GA106 GSP-client** (§2.2) | same | ★ YES (but unreached) |
| A4 | `virtual_mem.c:1793` | partial-unmap **left** remainder re-register | `:1821` on error; later `:1788` | ★ YES |
| A5 | `virtual_mem.c:1802` | partial-unmap **right** remainder re-register | `:1829` on error; later `:1788` | ★ YES |

⚠ **A4/A5 are the shape that breaks a naive mirror.** A partial `UNMAP_MEMORY_DMA` does not remove
one record — it removes the record and **re-inserts up to two new ones** for the surviving
remainders, plus a third throwaway (`pDmaMappingInfoUnmap`, `:1754`) that is freed at `:1813`. A
mirror keyed on "unmap ⇒ delete" corrupts state here. Same split logic mirrored in resserv at
`rs_server.c:2323/:2333`.

### Family B — `RsResourceRef::interMappings` (+ two back-ref lists) — 5 sites

Insert `refAddInterMapping` — `resserv/src/rs_resource.c:613`, appends to **three** lists at once:
`pMapperRef->interMappings` (`:627`), `pMappableRef->interBackRefsMappable` (`:632`),
`pContextRef->interBackRefsContext` (`:642`).

| # | insert site | when | teardown | paired? |
|---|---|---|---|---|
| B1 | `rs_server.c:2266` (`serverInterMap`) | every `MAP_MEMORY_DMA`, **before** `resMapTo` | `:2294` on error; `:2358`; `rs_client.c:836/837` | ★ YES |
| B2 | `rs_server.c:2323` | partial-unmap left remainder | `:2349` / `:2358` | ★ YES |
| B3 | `rs_server.c:2333` | partial-unmap right remainder | `:2352` / `:2358` | ★ YES |
| B4 | `mem_mgr/mem_fabric.c:216` | fabric-memory map | `:167`, `:280` | ★ YES |
| B5 | `mem_mgr/mem_multicast_fabric.c:2655` | multicast-fabric map | `:2495`, `:2718` | ★ YES |

★ B1 is the **outermost** hook: it fires for every inter-resource map before RM-specific code runs,
and `rs_client.c:1074-1076` asserts all three lists are empty at ref destruction — **resserv already
enforces the leak check we would otherwise have to write.**

### Family C — `VA_LIST` (RM-internal engine context buffers) — 3 sites

Store: `mem_mgr/vaddr_list.c:335` `vaListAddVa`, refcounted (`:371` re-add path). Containers:
`ENGINE_CTX_DESCRIPTOR::vaList`, `KernelGraphicsContextUnicast::globalCtxBufferVaList[]`,
`*.vAddrList`.

| # | insert site | when | teardown | paired? |
|---|---|---|---|---|
| C1 | `gr/kernel_graphics.c:2009` (in `kgraphicsMapCtxBuffer_IMPL:1913`) | GR ctx buffer mapped into a channel VAS | `kgraphicsUnmapCtxBuffer` `:2037` (`vaListRemoveVa`) via T5/T6 | ★ YES |
| C2 | `fifo/kernel_channel.c:3763` (in `kchannelSetEngineContextMemDesc_IMPL:3692`) | engine ctx memdesc bound to channel | channel free cascade | ★ YES |
| C3 | `fifo/kernel_channel.c:3942` (in `kchannelMapEngineCtxBuf`) | engine ctx buffer VA cached | `:3910` find / channel free | ★ YES |

★★★ **C is DISABLED for a CUDA process.** `kgrctxMapCtxBuffers_IMPL`, `kernel_graphics_context.c:1613-1615`:

```c
pGVAS = dynamicCast(pKernelChannel->pVAS, OBJGVASPACE);
if (gvaspaceIsExternallyOwned(pGVAS))
    return NV_OK;                    // ← RM maps NOTHING
```

Same early-out at `:1884` and `:2072`; `kgraphicsMapCtxBuffer_IMPL:1934` outright **asserts**
`!gvaspaceIsExternallyOwned(pGVAS)`. Under CUDA the VAS is **externally owned by UVM**, so on that
path family C never runs and **UVM establishes those VAs itself**. See §5.

### Family E — VAS eheap VA reservations — ~2 sites

`pGVAS->pHeap->eheapAlloc` — `gpu_vaspace.c:1559` (`gvaspaceAlloc`), `:767` / `:4599` / `:4628`
(reserved ranges). Teardown `eheapFree` `:1816`, `:3434`; container destroyed `:914-922`.
This is **VA-range ownership**, not VA→backing, and is listed for completeness. ★ Paired.

---

## 4. UNPAIRED INSERTS AND UNPAIRED TEARDOWNS

⚠ Both are findings. Here is what the sweep found, and what it would have missed.

**No unpaired insert was found in families A–E.** Every insert site above has a reachable teardown
and, for A/B, an error-path teardown as well. That is a stronger result than expected and it is
worth stating plainly: **RM's mapping bookkeeping is symmetric by construction, and resserv asserts
it** (`rs_client.c:1074-1076`).

**Near-misses / asymmetries worth recording:**

1. ⊘ **`virtmemDestruct_IMPL` does NOT walk `pDmaMappingList`.** `virtual_mem.c:652-717` frees the
   memdesc and the heap allocation and **never touches the btree**. The dma mappings are removed
   *earlier*, by resserv's `_clientUnmapInterMappings` (`rs_client.c:836`) driving
   `virtmemUnmapFrom`, before `objDelete` (`rs_client.c:~880`) reaches the destructor.
   ⇒ **The teardown is real but lives in a different layer than the insert.** A mirror that hooks
   "VirtualMemory freed" and expects to see mapping removals there will see none.

2. ⊘ **`_virtmemCopyConstruct` deliberately drops the mapping list on `DUP_OBJECT`.**
   `virtual_mem.c:254-255`: `// Mappings do not follow virtual memory object` /
   `pDstVirtualMemory->pDmaMappingList = NULL;`
   ⇒ **`NV_ESC_RM_DUP_OBJECT` on a `VirtualMemory` is NOT a mapping-insert site.** It duplicates the
   *object* and the VA *reservation* refcount (`vaspaceIncAllocRefCnt`, `:264`, under
   `if (bIncAllocRefCnt)` where `bIncAllocRefCnt = pSrcVirtualMemory->bReserveVaOnAlloc && !pSrcMemory->bRpcAlloc`
   at `:247`), **not** the mappings. Anyone expecting DUP to publish mappings will find zero and
   must not read that zero as "no mappings exist".
   ★★ **RESOLVED, and the resolution is itself the lesson: there is NO `vaspaceDecAllocRefCnt`.**
   `vaspaceIncAllocRefCnt` → `gvaspaceIncAllocRefCnt_IMPL` (`gpu_vaspace.c:1943-1961`) does one
   thing: `pVASpaceBlock->refCount++` on the eheap block (`:1959`). The **decrement has no function
   of its own** — it is inline at the top of the VA-block free path, `gpu_vaspace.c:1741-1745`:
   ```c
   if (pMemBlock->refCount > 1) { pMemBlock->refCount--; return NV_OK; }   // ← early-out, block survives
   ```
   ⇒ **The pair is symmetric in behaviour and asymmetric in NAME.** A name-based sweep
   (`grep Inc… / grep Dec…`) reports a leak here that does not exist. ⚠ This is the exact shape the
   brief warned about — *a sweep is scoped to what it names* — and it fired inside this very
   document's first draft. **Recorded as a false-positive class, not as a defect.**

3. ⚠ **`_gvaspaceMappingRemove` re-inserts.** `gpu_vaspace.c:4876` and `:4884` call
   `_gvaspaceMappingInsert` from *inside* the remove, to re-record the surviving head/tail of a
   partially-unmapped range (and `:4912` re-inserts the original node on the multi-GPU path). ⇒
   **"remove" is not monotone.** Any mirror driven off remove events must handle re-insertion, or a
   partial unmap will silently drop the surviving remainders from our table.

**No dead teardown was found**, with one qualification: the RPC-side teardown
`NV_RM_RPC_UNMAP_MEMORY_DMA` (`virtual_mem.c:1863`) is **unreachable on a GA106 GSP client**
because its guard `pMemory->bRpcAlloc` is false (§2.2). It is live for the vGPU-guest configuration.
⇒ **Dead for us, not dead in general** — a version/configuration-scoped finding, not a defect.

---

## 5. WHAT THIS SWEEP DELIBERATELY DOES NOT COVER — and why it matters most

★★★ **Under CUDA, the VA space is EXTERNALLY OWNED, and families A/C are bypassed for the buffers
that matter.**

`gvaspaceIsExternallyOwned_IMPL` (`gpu_vaspace.c:2024`) gates:

| site | behaviour when externally owned |
|---|---|
| `kernel_graphics_context.c:1614-1615` | `kgrctxMapCtxBuffers` returns `NV_OK` having mapped **nothing** |
| `kernel_graphics_context.c:1884`, `:2072` | same early-out |
| `kernel_graphics.c:1934` | hard assert — `kgraphicsMapCtxBuffer` **may not be called** |
| `falcon/kernel_falcon.c:167`, `:218` | ctx-buffer map skipped |
| `fifo/kernel_channel.c:2200`, `:3920` | channel/ctx VA handling diverges |

⇒ **There are exactly TWO GMMU page-table writers on this system:**
1. **RM's MMU walker** — `mmuWalkMap` via `gvaspaceMap_IMPL:2239`, for RM-owned VA spaces.
   **Transport: CPU stores through BAR2** (§2.5).
2. **UVM's own page-tree code**, for externally-owned VA spaces. **Transport: UVM's own CE
   pushbuffer** (`kernel-open/nvidia-uvm/uvm_map_external.c:230-241`).

For a CUDA workload, writer (2) dominates. Family F below enumerates it.

---

## 5A. FAMILY F — the UVM plane (28 rows, summarised)

Paths below are relative to `research_clones/ogkm-580.159.04/kernel-open/nvidia-uvm/`.
Ioctl dispatch table: `uvm.c:996-1049`.

### 5A.1 ★★★ The teardown answer — UVM is kernel-guaranteed, and guaranteed EARLIER than RM

**This closes the highest-value gap from my first draft.** Two independent guaranteed routes:

```
close(fd) / SIGKILL → uvm.c:1079 (.release = uvm_release_entry) → uvm.c:250 uvm_release
                    → uvm.c:202 uvm_release_va_space → uvm_va_space.c:463 uvm_va_space_destroy
```
`uvm_va_space_destroy` walks **every** VA range (`uvm_va_space.c:505-510`,
`uvm_for_each_va_range_safe`), detaches all user channels (`:500`), unregisters every GPU (`:517`),
and destroys HMM state (`:519`).

★★ **And earlier still** — `uvm_va_space_mm_shutdown` (`uvm_va_space_mm.c:329`) runs at **mm
teardown**, which under `SIGKILL` precedes fd release: it stops channels (`:352`), detaches them
(`:360`), and calls `nvUvmInterfaceUnsetPageDirectory` for every GPU VA space (`:388-389`).

⇒ **"Process dies without issuing ioctls" does not leak GPU VA mappings in UVM either.** Every
user-visible UVM row is reachable from one of these. **Both RM and UVM give us a
kernel-guaranteed unjoin point.**

### 5A.2 The GSP-crossing rows — five, and four carry no VA

| trigger | insert | what crosses | carries a GPU VA? |
|---|---|---|---|
| `UVM_REGISTER_GPU_VASPACE` (base 25) | `uvm_gpu_va_space_t` → `uvm_va_space.c:1606-1608`; page tree `uvm_mmu.c:1116` | `nvUvmInterfaceSetPageDirectory` `uvm_va_space.c:1394` → `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`rmapi/nv_gpu_ops.c:8870`) | ⊘ no — **the PDB root only** |
| `UVM_REGISTER_CHANNEL` (base 27) | `uvm_user_channel_t` → `list_add` `uvm_user_channel.c:592` | `nvUvmInterfaceRetainChannel` `:141` → `nv_gpu_ops.c:10122` | ⊘ no |
| `UVM_REGISTER_CHANNEL` | RM-side bind `uvm_user_channel.c:467` | **`NV2080_CTRL_CMD_GPU_PROMOTE_CTX`** with `promoteEntry[i].gpuVirtAddr = …resourceVa` (`nv_gpu_ops.c:10888-10891`) | ★★★ **YES — the one row that publishes a VA** |
| `UVM_MAP_EXTERNAL_ALLOCATION` (base 33) | `uvm_ext_gpu_map_t` → `uvm_range_tree_add` `uvm_map_external.c:894` | `nvUvmInterfaceDupMemory` `:909` → `NV_RM_RPC_DUP_OBJECT` **iff** `pMemory->bRpcAlloc && (IS_VIRTUAL \|\| IS_FW_CLIENT)` (`mem_mgr/mem.c:1114-1116`) | ⊘ no — **a memory handle, not a VA** |
| internal (pushbuffer, semaphores) | `uvm_rm_mem.c:223/233` | `nvUvmInterfaceMemoryAllocSys/AllocFB` — RM both allocates *and* maps; UVM never touches these page tables | ⊘ no (RM-side VA) |

★ I verified the `DUP_OBJECT` row myself: `mem.c:1114` reads
`if (pMemory->bRpcAlloc && (IS_VIRTUAL(pSrcGpu) || IS_FW_CLIENT(pSrcGpu)))`, and
`IS_FW_CLIENT(pGpu) = IS_GSP_CLIENT(pGpu) || IS_DCE_CLIENT(pGpu)` (`generated/g_gpu_nvoc.h:5686`).
⇒ **On a GSP client this DOES fire** for GSP-allocated memory.
⇒ ★★ **So we DO get a wire-visible event per `UVM_MAP_EXTERNAL_ALLOCATION` — but it names the
allocation, never the VA it is about to be mapped at.** *"This buffer was duped"* is a strictly
weaker signal than *"this buffer is now at VA X"*, and treating the first as the second would
mis-place every mapping.

★ The **PTE values** are computed CPU-RM-side, not GSP-side:
`nvGpuOpsGetExternalAllocPtesOrPhysAddrs` (`nv_gpu_ops.c:4535`) encodes them locally via
`kgmmuEncodePhysAddrs` (`:4180`); its **only** RPC is `NV_RM_RPC_GET_PLCABLE_ADDRESS_KIND` (`:4216`),
gated on `IS_VIRTUAL_WITH_SRIOV && gpuIsWarBug200577889SriovHeavyEnabled && isCompressedKind && …`
(`:4185-4189`) — **not our path.**

### 5A.3 UVM-local rows (the bulk) — a representative index

`UVM_CREATE_EXTERNAL_RANGE` → `uvm_va_range.c:285` · `UVM_MAP_EXTERNAL_SPARSE` →
`uvm_map_external.c:1138` · `UVM_MAP_DYNAMIC_PARALLELISM_REGION` → `uvm_va_range.c:364` ·
`UVM_ALLOC_SEMAPHORE_POOL` → `uvm_va_range.c:415` / `:1122` / `:842` · channel VA ranges →
`uvm_user_channel.c:364` · fault-routing rb-trees → `uvm_gpu.c:3431`, `:3279` · managed-block GPU
state → `uvm_va_block.c:1425` · GPU PTE bits → `uvm_va_block.c:8560-8562` · chunk reverse map →
`uvm_va_block.c:2979` · sysmem DMA records → `uvm_pmm_sysmem.c:269` · page-table range refcounts →
`uvm_mmu.c:265`, `:574` · HMM blocks → `uvm_hmm.c:684`.
**All paired**; teardowns cited in the lane's table. All invisible to a GSP.

### 5A.4 ⚠ UVM asymmetries worth carrying into our design

- ★★★ **F2 — `PROMOTE_CTX` has no un-promote.** The only VA-carrying GSP crossing in the whole
  census is **inserted and never withdrawn**. `uvm_user_channel_stop` calls only
  `nvUvmInterfaceStopChannel` (`uvm_user_channel.c:787`) and clears a local flag (`:792`); the
  RM/GSP-side record dies only with `nvUvmInterfaceReleaseChannel` (`:857`). Meanwhile UVM clears
  the actual PTEs first (`uvm_va_range.c:539`). ⇒ **A window exists in which the GSP still believes
  a VA is promoted for a context whose PTEs are already gone.** ★ For us this is the one place a
  mirror keyed on GSP traffic would hold a stale VA, and UVM's own mitigation is **ordering**
  (stop, then detach — `uvm_user_channel.c:875-881`) plus a fault-buffer flush
  (`uvm_va_space.c:1972-1977`). **If we mirror `PROMOTE_CTX`, we must expire it on channel
  teardown ourselves — nothing on the wire will tell us to.**
- **F3 — `UVM_FREE` on a semaphore pool silently does nothing** if userspace has not munmap'd:
  `uvm_va_range.c:637-638` returns `NV_ERR_INVALID_ARGUMENT` and `uvm_free` propagates it
  (`:708-709`) **without destroying the range**. Backstopped by `uvm_va_space.c:509`.
- **F4 — `uvm_mmu_sysmem_map` (`uvm_mmu.c:2868`) has an insert with NO per-call teardown.** Grep
  found no `uvm_mmu_sysmem_unmap`; removal is bulk-only at GPU deinit (`uvm_mmu.c:2790`, `:2944`).
  Contrast its refcounted vidmem twin (`uvm_mmu.c:2669/2719`). ⊘ **SR-IOV-heavy only** — not our
  configuration, but it is a genuine unpaired insert and is reported as one.
- **F5 — two deliberate leaks on fatal error:** `uvm_mmu.c:1281-1285` (*"We can't perform the unmap,
  so just leave things in place for debug"*) and `uvm_map_external.c:500-503`
  (*"System-fatal error. Just leak."*). Both conditional on global fatal state.
- **F8 — the highest-value line to audit** in external-mapping teardown: `uvm_va_range.c:508`,
  `if (uvm_processor_mask_empty(&external_range->mapped_gpus)) goto out;`. If `mapped_gpus` ever
  desyncs from the per-GPU trees, that `goto` **skips real mappings**.

---

## 6. ⇒ WHICH ROW COULD BE "THE SOURCE THAT DID NOT FIRE" FOR THE CE OPERAND

Live context: the wall reproduces in 82 ioctls with host faults at **`0x1_20000000`** (a CE `src`
operand) and **`0x7_00100000`**, both `FAULT_PTE / ACCESS_TYPE_VIRT_READ`, and the same binary
passes natively.

**Ranked candidates, most to least likely:**

1. ★★★ **UVM's externally-owned-VAS page-table writes (§5, writer 2).** Not in any RM structure;
   never an RPC; never on our wire. If the failing workload is CUDA/UVM-backed, this is the prime
   suspect **by construction**, and no amount of RM-side mirroring reaches it.
2. ★★★ **Family D — `gvaspaceMap_IMPL` (§3).** RM's own walker writes the PTE from the CPU with no
   RPC. **A GSP emulator sees nothing.** If the CE source operand was mapped by an RM path
   (`MAP_MEMORY_DMA` from a non-UVM client, or an RM-internal buffer), this is where it happened and
   where we are blind.
3. ★★ **Family A/B — `NV_ESC_RM_MAP_MEMORY_DMA` (§3).** The user-visible trigger for (2).
   Observable as an **ioctl** but **not** as an RPC.
4. ★ **Family C — engine ctx buffers.** Only if the VAS is *not* externally owned.

⚠ ⊘ **This is a hypothesis, not a conclusion, and I am explicitly not asserting it.** Three
hypotheses on this campaign have been refuted in two days. **The falsifier is cheap and already in
flight**: the sibling `NV0080_CTRL_CMD_DMA_GET_PTE_INFO` probe at the exact faulting addresses. If
it reports `0x1_20000000` **mapped**, then the fault is *not* a missing mapping, candidates 1–4 are
all wrong, and this section is void — while §1 (teardown) and §2 (the GSP-crossing structure) stand
regardless.

★★★ **What survives either outcome — and it is now sharper than my first draft, and partly a
correction of it:**

**BOTH declared populate sources have a structural blind spot, and they are different blind spots.**

| our source | covers | ⊘ blind to |
|---|---|---|
| **(1) bind-time RPC/ioctl bindings** | nothing on the RM mapping plane — it does not cross (§2.2). Sees UVM's `DUP_OBJECT`, which names an allocation, **not a VA** (§5A.2) | **every** GPU VA |
| **(2) the observed CE page-table write** | **UVM's** writes — UVM uses its own CE pushbuffer (`uvm_map_external.c:230-241`) | ⊘ **all of RM's writes — they are CPU stores through BAR2** (§2.5) |

⇒ ★★★ **An RM-established mapping is invisible to BOTH.** ⊘ **If we have anywhere treated "no CE
page-table write observed" as "no mapping was established", that inference is unsound for the entire
RM plane** — and that is a reading error our own instruments would never flag, because the absence
looks exactly like the mapping not existing. ⚠ Precisely this campaign's *"an absent artefact reads
as favourable"*, one level up: the **instrument** is absent, not the event.

★ This still **corroborates** `mode2_address_table.md` §5's correction — source (2) really is the
surviving source where it applies — while **narrowing its scope**: §5 was measured on the
GSP-emulated compute path, where the writer is UVM. It does not generalise to RM-written mappings.
⇒ *What would close it:* a BAR2-write watcher, or accept RM-plane mappings as unobservable and
mirror `gvaspaceMap_IMPL` semantically instead.

---

## 7. ⊘ WHAT I COULD NOT DETERMINE, AND WHAT WOULD DETERMINE IT

1. ✔ **CLOSED — the UVM plane is now §5A**, and the specific question ("is UVM's teardown
   kernel-guaranteed like RM's?") is answered **yes, twice over**: `uvm_release` → `uvm_va_space_destroy`
   (`uvm_va_space.c:463`), plus `uvm_va_space_mm_shutdown` (`uvm_va_space_mm.c:329`) at mm teardown,
   which fires *earlier* under `SIGKILL`.
2. ✔ **CLOSED — the RPC / `ROUTE_TO_PHYSICAL` surface is now §2.6**, including the methodology trap
   (three key controls cross via hand-rolled `NV_RM_RPC_CONTROL` **without** the flag).
   ⊘ **Residual:** the sub-lane states it did **not** exhaustively sweep all `_IMPL` control bodies
   for embedded `NV_RM_RPC_CONTROL` — it found those three by name-directed grep. **There may be
   more crossings than §2.6 lists.** ⇒ *What would determine it:* a grep for `NV_RM_RPC_CONTROL`
   across every `*Ctrl*_IMPL` body.
2b. ⊘ **GSP-RM's own source is not in this tree** (`RMCFG_FEATURE_PLATFORM_GSP == 0`,
   `generated/rmconfig.h:260`). This census says what CPU-RM **sends**, never what GSP-RM **does**.
3. ✔ **CLOSED during this pass** — `vaspaceIncAllocRefCnt`'s decrement is inline at
   `gpu_vaspace.c:1741-1745`, not a separate function. See §4 item 2.
4. ⊘ **BAR1/BAR2 and `kbus` mapping bookkeeping** (`gpu/bus/arch/*/kern_bus_*.c`) is out of scope.
   It uses family D underneath (`kern_bus_gm107.c:3435` calls `dmaAllocMapping_HAL`), so it is
   covered *transitively*, but its own per-BAR records are not enumerated.
5. ⊘ **P2P / BAR1-P2P mappings** (`dmaAllocBar1P2PMapping_HAL`, `virtual_mem.c:1292`;
   freed `:1596`, `:1711`) are noted as paired but not analysed. Not on a single-GPU GA106 path.
6. ⊘ **Confidential-computing and MIG variants** were not resolved. Several sites carry
   `swizzId` / `KMIGMGR_SWIZZID_INVALID` parameters; MIG changes VAS ownership.

### ★★★ TWO CONFIGURATION SWITCHES THAT INVERT THIS DOCUMENT

⚠ **Both are single flags, and either one flips the central conclusion. A reader who assumes the
default without checking will be exactly wrong.**

1. **`RmSplitVasMgmtServerClientRm` set to DISABLED** (`gpu_registry.c:174-177`) ⇒
   `bSplitVasManagementServerClientRm = NV_FALSE` ⇒ `bRpcAlloc` becomes **TRUE** ⇒
   `NV_RM_RPC_MAP_MEMORY_DMA` **fires** at `virtual_mem.c:1522`, both macro gates open, and
   `MAP_MEMORY_DMA` / `UNMAP_MEMORY_DMA` become **live wire opcodes**. ⇒ **§2's "no RPC" becomes
   "every mapping is an RPC".** A mirror should probably handle both configurations.
2. **Confidential Compute** ⇒ `kbusIsBarAccessBlocked()` true ⇒ `TRANSFER_TYPE_GSP_DMA`
   (`mem_utils.c:118-122`) ⇒ **page-table bytes traverse the wire** and §2.5's "RM's PTE writes are
   invisible" becomes false.

⇒ ★ **The measured claims here are true of a DEFAULT bare-metal GA106 GSP client and are
configuration-scoped, not architectural.** ⚠ Same class as this campaign's *"a ruling's DATE is
part of the citation"* — here it is the **configuration** that is part of the citation.

### What this enumeration would MISS, stated plainly

- **Anything that writes PTEs without going through `gvaspaceMap_IMPL`** — i.e. all of UVM's
  externally-owned-VAS path (§5), and the `flags.bRemap` path inside `gvaspaceMap` itself
  (`:2219`), which rewrites PTEs while skipping the bookkeeping insert.
- **Mappings established before RM bookkeeping exists** — early boot / BAR2 bootstrap
  (`gpu/mmu/bar2_walk.c`) uses a separate walker.
- **Any site reached only under a HAL variant not bound on GA106.** I resolved `dmaAllocMapping`,
  `kgrctxUnmapBuffers`, and `kgrctxShouldCleanup` against the generated dispatch tables; I did
  **not** resolve every function named above. A `_gv100`/`_PHYSICAL`/`_46f6a7` variant elsewhere in
  these paths could be dead on our target and I would not have noticed.
- **Anything whose insert is spelled differently.** This sweep keyed on `btreeInsert`,
  `listAppendNew`, `vaListAddVa`, `refAddInterMapping`, `eheapAlloc`. **A structure using a
  different container idiom would be invisible to it**, and an absence here must not be read as a
  measured zero.

### Sweep integrity — known-positives, both PASSED

- ★ Absent-`_IMPL` control: `grep -rn 'kchannelCtrlCmdGetClassEngineid_IMPL'` → found at
  `gpu/fifo/kernel_channel.c:2923` **and** in the nvoc dispatch table
  `generated/g_kernel_channel_nvoc.c:254`. **PASS.**
- ★ Reachability control: `grep -rn 'CliGetDmaMappingInfo'` → found the definition
  (`rmapi/mapping_list.c:391`), the declaration (`inc/kernel/rmapi/mapping_list.h:176`), and **7
  distinct call sites** including the error-notifier path at `gpu/fifo/kernel_channel.c:2033`
  (inside the `2019-2075` range named in the brief). **PASS.**

---

## 8. ONE-LINE SUMMARY FOR THE DESIGN

**Mirror `gvaspaceMap_IMPL` / `gvaspaceUnmap_IMPL` (family D) for the RM plane and UVM's
`uvm_ext_gpu_map` tree (family F) for the CUDA plane; key the unjoin on `clientFreeResource_IMPL`
(T2b/T2c) and `uvm_va_space_destroy` — both kernel-guaranteed — with the `pMapTree` whole-block
sweep (T4) as the backstop; expire any mirrored `PROMOTE_CTX` VA ourselves, because nothing on the
wire withdraws it (§5A F2); and accept that on a default GSP-client configuration a GPU VA reaches
us through NEITHER declared populate source — not as an RPC (§2.2), and, for the RM plane, not as a
CE page-table write either (§2.5).**
