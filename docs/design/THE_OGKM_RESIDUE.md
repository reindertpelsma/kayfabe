# ogkm residue — what NVIDIA left behind about monolithic RM, pre-Turing, and Windows

**STATUS: LIVE, 2026-09-20 (w821).** `[owner]` *"Worth digging ogkm source so we get any leftover
for either Windows behaviour or non-GSP behaviour, and record it. The rest is nouveau."*
Surveyed against 610.43.02 with 580.159.04 as the delta reference.

---

## 0. ⊘⊘⊘ THE HEADLINE REFUTES WHAT I TOLD THE OWNER YESTERDAY

`THE_WINDOWS_AXIS.md` §10.3 argued the no-GSP plane's real cost is **losing the oracle**, because
*"for the native Turing+ path, openrm contains nothing: it is GSP-only."*

⊘ **That is wrong, and substantially so.** The drop is *compiled* as a GSP client, but the
monolithic material is **present and readable** in three distinct forms. Distinguishing them is
the whole skill of reading this tree:

| class | what it is | readable? | built? |
|---|---|---|---|
| **(1) declarative** | the monolithic object model, engine descriptors, module list, the control catalogue | ✔ fully | ⊘ `#define … 0` |
| **(2) GSP-RM-side code inline in shared functions** | `if (RMCFG_FEATURE_PLATFORM_GSP)` blocks = **what the firmware itself does** | ✔ fully | ⊘ constant-folded |
| **(3) chip-named "monolithic-era" bodies** | `*_GM107`, `*_GP100`, `*_GV100` | ✔ fully | ★ **mostly YES, and mostly LIVE** |

★★★ **And (3) inverts the intuition that pre-Turing code is dead code.** 56 of 57 pre-Turing-named
source files are compiled, and they are the **live base implementations Turing+ inherits**. What is
dead is *chip enablement* — every pre-Turing chip has a **zeroed `(arch, impl)` row** in
`g_hal_archimpl.h`, so it can never be matched at probe.

---

## 1. ★★★★★ The reading technique — the VF HAL arm is the surviving proxy for monolithic

One example carries the method. `generated/g_kern_bus_nvoc.c:602-617`:

```c
if (RmVariantHal: VF)  __kbusCommitBar2__ = &kbusCommitBar2_GM107;   // the REAL hardware sequence
else                   __kbusCommitBar2__ = &kbusCommitBar2_KERNEL;  // a no-op / an RPC
```

- `kbusCommitBar2_KERNEL` (`gpu/bus/kern_bus.c:457`) does essentially nothing.
- `kbusCommitBar2_GM107` (`arch/maxwell/kern_bus_gm107.c:5799-5861`) is the **full physical
  sequence**: rewrite PTEs for the self-mapping pointer and flush page, flush the write-combine
  buffer, `kbusFlush_HAL(BUS_FLUSH_VIDEO_MEMORY)`, `kgmmuInvalidateTlb_HAL(… PTE_DOWNGRADE,
  NON_LINK_TLBS)`, swap the PDB, `kbusBindBar2_HAL(BAR2_MODE_VIRTUAL)`.

⇒ ★ **Wherever behaviour is halified on `RmVariantHal`, read the VF arm to learn what the CPU
does itself and the `_KERNEL` arm to learn what it delegates.** **376 members** are halified this
way; `423` generated dispatch sites carry a `RmVariantHal` condition.

⚠ The taxonomy is stated by NVIDIA in prose (`generated/g_chips2halspec_nvoc.h:89-94`):
*"`KERNEL_ONLY`: RM does not own HW, the physical part is offloaded to Ucode. `MONOLITHIC`: RM
owns both the interface to the client and the underlying HW."* ⊘ But the generated selector has
**no arm for `PF_MONOLITHIC` or `UCODE`** (`g_chips2halspec_nvoc.c:205-217`) — only `VF` and
`PF_KERNEL_ONLY`. **That is why the VF arm is the proxy: it is the only surviving "RM owns the
hardware" variant.**

---

## 2. The four artifacts worth having

### 2.1 ★★★★★ `inc/kernel/gpu/gpu_child_list.h` — the complete monolithic object model

**89 children, 34 enabled, 55 disabled**, with construction **order** and a `bConstructEarly`
flag. The 55 disabled are exactly the physical-RM object set — `OBJFIFO`, `OBJGR`, `OBJBUS`,
`OBJGMMU`, `OBJCE`, `OBJDISP`, `OBJVBIOS`, `OBJACR`, `OBJVMMU`, `Pmu`, `OBJSEC2`, … — declared,
readable, not built. ⇒ **Nothing else in the tree tells you what a real RM instantiates, or in
what order.**

★ Corollary the docs never state: in this drop **RM never parses the VBIOS, never runs devinit,
never owns ACR/LSFM ucode loading, never owns VMMU, and never owns the runlist scheduler** — all
are `RMCFG_MODULE_* = 0`. Every engine exists as a `X` / `KERNEL_X` pair (33 of them) and in
**every pair the physical half is 0**.

### 2.2 ★★★★★ `ROUTE_TO_PHYSICAL` — a machine-readable manifest of the CPU/GSP boundary

The routing is one function (`rmapi/resource.c:254-300`): if `IS_FW_CLIENT` and the flag is set,
RPC and **skip the local body**; otherwise **run it**. Measured over the generated method tables:

| | count |
|---|---|
| exported RM control methods | **1372** |
| carrying `ROUTE_TO_PHYSICAL` | **753 (54.9 %)** |
| …**body stripped** (`pFunc = NULL`) | **679** |
| …**body retained** (also `IMPLEMENTED_ON_VGPU_GUEST`) | **74** |

⇒ ★ For anything Mode-2-shaped this is the definitive list of **what must be answered rather than
executed** — with method id, parameter struct and function name for all 753. And the **74 retained
bodies are the physical computation, readable**, including `FifoGetDeviceInfoTable` (`0x20801112`),
`MemSysGetStaticConfig` (`0x20800a1c`), `BifGetStaticInfo` (`0x20800aac`), `GmmuGetStaticInfo`
(`0x20800a59`).

### 2.3 ★★★★ 42 orphaned monolithic bodies — defined once, referenced nowhere

Compiled but unreachable. The highest-value cluster is BAR1/BAR2 bring-up:
`kbusInitInstBlk_GM107` (`kern_bus_gm107.c:5223`), `kbusBar2InstBlkWrite_GM107` (`:5299`),
`kbusBindBar2_GM107` (`:5111`).

★★★ Together they give the **complete instance-block field layout and BOTH write paths**: the
bootstrap path pokes `NV_RAMIN_ADR_LIMIT_LO/HI` and `NV_RAMIN_PAGE_DIR_BASE_{TARGET,VOL,LO,HI}`
through the **BAR0/PRAMIN window**; the steady-state path writes the same fields via a **BAR2
mapping**. ⇒ Directly relevant to this project's PRAMIN and instance-block modelling.

⚠ **A pre-Turing name proves nothing about liveness.** `rpcGetEngineUtilizationWrapper_GM204`
looks equally orphaned and is wired to **every Turing+ chip** (`g_rpc_private.h:4021…5233`).
**Only the generated tables decide.**

### 2.4 ★★★ 163 readable `if (RMCFG_FEATURE_PLATFORM_GSP)` blocks — the firmware's own code

Compiled out, intact. ⇒ The cheapest window into **GSP-RM behaviour** available anywhere,
concentrated in `mem_mgr/gpu_vaspace.c`, `mem_mgr/virtual_mem.c`, `gpu/mem_mgr/heap.c`,
`gpu/gr/kernel_graphics_context.c`, `rmapi/control.c`.

---

## 3. ★★★★★ The finding that lands on OUR CURRENT DESIGN: BAR1/BAR2 are SPLIT

⊘ **An emulator that models one flat BAR2 (or BAR1) VA space will diverge from what the guest
expects**, and NVIDIA says so twice:

- `gpu/bus/arch/turing/kern_bus_tu102.c:215-219` — *"In RM-offload scenario, **Kernel RM and
  Physical RM use their own GPU VA space** respectively. The expectation is that the **Physical RM
  base starts from the second PD entry of the topmost PD**"* ⇒
  `mmuFmtEntryIndexVirtAddrLo(pFmt->pRoot, 0, 1)`.
- `mem_mgr/gpu_vaspace.c:255-262`, `:820-824` — *"**BAR1 construction is split between GSP and CPU.
  Only CPU side performs the sparsification.**"* And `_gvaspaceBar1VaSpaceConstruct/Destruct` are
  **no-ops unless `RMCFG_FEATURE_PLATFORM_GSP`** (`:233-237`) — pinning the BAR1 root page
  directory is **the firmware's job**, i.e. **ours**.

⊘⊘⊘ **[CORRECTED w821 — I stated the ownership BACKWARDS, and the direction is the whole point.]**
I wrote *"we own PD0[0] and the BAR1 root pin; the guest owns PD0[1]."* **The reverse is true**, per
`kbusPatchBar2Pdb_GSPCLIENT` (`gpu/bus/kern_bus.c:816`):

> *"CPU-RM owns the VA range under **PDE3[0]** and GSP-RM owns the VA range under **PDE3[1]** …
> CPU-RM passes its PDE3[0] value to GSP-RM, then GSP-RM will fill this value to PDE3[0] of
> GSP-RM's table (**only GSP-RM's BAR2 table will be bound to HW**)."*

★★★ ⇒ **Guest = PDE3[0]. We = PDE3[1]. And OUR table is the one hardware walks.** The function
literally rewrites the guest's own PDB cache to **our** address: `memdescDescribe(pMemDesc,
ADDR_FBMEM, pGSCI->bar2PdeBase, …); pKernelBus->virtualBar2[…].pPDB = pMemDesc;`

⇒ **Three concrete obligations v3 does not state:**

1. ★ **We allocate and bind the REAL BAR2 root**, in reserved framebuffer, and report its address
   as **`bar2PdeBase` in `GET_GSP_STATIC_INFO`**. The guest then swaps its PDB cache to ours — so
   **its TLB invalidates name OUR PDB**, and our BAR2 walker roots at our page with **entry 0 = the
   guest's `entryValue`** handed to us by `UPDATE_BAR_PDE` (fn 70).
   ⚠ `THE_SURFACE_v3.md` §1.3's fn-70 row is imprecise: it carries the **PDE3[0] entry**, not a
   root address.
2. ⊘⊘ **BAR1 has NO RPC AT ALL.** `kbusPatchBar1Pdb_GSPCLIENT` (`kern_bus.c:766`) makes the guest
   **adopt our root page at `bar1PdeBase` and write PDEs into it directly.** ⇒ Anything modelling a
   BAR1 update *message* is modelling something that does not exist.
3. ⊘ **Nobody sparsifies BAR1 unless we do.** The CPU side skips it for a GSP client
   (`gpu_vaspace.c:255`), so an unpopulated BAR1 PDE is **sparse, not an error**.

★ ⇒ The real statement of what *"we are the GSP"* obliges here: **we own the root pages of both
BARs, we declare their addresses in static info, and the guest writes into tables we allocated.**

### 3.1 Fault-buffer ownership is the sharpest divergence

| | monolithic | GSP client |
|---|---|---|
| map the HW fault buffer | ★ into CPU-invisible BAR2 | ⊘ **asserted impossible** — `kgmmuFaultBufferMap` has `NV_ASSERT_OR_RETURN(!IS_GSP_CLIENT(pGpu), …)` (`kern_gmmu.c:2743`) |
| `MC_ENGINE_IDX_GMMU`, `_NON_REPLAYABLE_FAULT`, `_ERROR` | CPU services them (`:2278-2299`) | ⊘ never reach the host; *"bounce the interrupt to GSP"* (`:2435`) |
| UVM's fault-buffer `GET` register | real | ⊘ *"a **dummy register**… The real GET register is owned by GSP-RM"* (`uvm_gpu_isr.c:693`) |

⇒ ★ Directly relevant to `THE_ARCHITECTURE_v3.md` §10's fault-delivery item: **as the GSP we own
the PUT pointer and the guest reads it.**

---

## 4. Pre-Turing — constants are thin, implementations are LIVE

| | finding |
|---|---|
| **swref registers** | ⊘ **Fermi has no directory at all; Kepler is ONE 41-line file, included by zero `.c`.** Usable coverage is Maxwell/Pascal/Volta only: **42 headers / 2222 lines** |
| **class headers** | ✔ **all present with full method encodings** (~36 000 lines Fermi→Volta). `GF100_CHANNEL_GPFIFO 0x906f`, `KEPLER_CHANNEL_GPFIFO_A 0xa06f`, `MAXWELL_ 0xb06f`, `PASCAL_ 0xc06f`, `VOLTA_ 0xc36f` are **enabled = 1** and allocatable today |
| **implementations** | ★ **57 files, 56 compiled, and LIVE as base classes.** `dmaUpdateVASpace_GF100` is the **live PTE writer for every supported GPU**; BAR2 bring-up is `_GM107` for everyone; Ampere's MMU format literally calls `kgmmuFmtInitLevels_GP10X` |
| **probe gate** | ⊘ `g_hal_archimpl.h` retains pre-Turing rows **zeroed** — `{ 0x0, 0x0, 0x0 } , // GF100 (disabled)`. **That table is what makes pre-Turing unmatchable** |
| **boundary constants** | ✔ free for the taking: `NV_PMC_BOOT_42_ARCHITECTURE_{GM100 0x11 … TU100 0x16}`; `GMMU_FMT_VERSION_{1,2,3}` with generation boundaries; `NV_RAMIN`/`NV_RAMUSERD` offsets (`maxwell/gm107/dev_ram.h:36-67`) |

⇒ ★★★ **For pre-Turing REGISTERS we still need nouveau** — ogkm's swref stops at Maxwell and has
nothing for Fermi/Kepler. ⇒ **For pre-Turing SEMANTICS ogkm is excellent**, because the code is
live and the class encodings are complete. **That is the division of labour the owner asked for.**

### 4.1 ⚠ Version rule: use 580 for pre-Turing UVM, 610 for everything else

**610 DELETED all pre-Turing UVM material** — `uvm_{maxwell,pascal,volta}*` (19 files / 4560
lines), the pre-Turing `hwref/` headers (7 files / 2265 lines) and UVM's local class-header copies.
⇒ **580.159.04 is now the ONLY source for pre-Turing UVM MMU, fault-buffer, CE and host
behaviour.** ★ Conversely 610 *added* four pre-Turing swref headers, so **610 is the better source
for pre-Turing RM-core registers.** Keep both trees.

---

## 5. Windows residue beyond what Part 3 already had

| ★ | finding | where |
|---|---|---|
| ★★★★★ | **GSP can BSOD the host.** Event `TRIGGER_BUGCHECK 0x1023` → `osBugCheck(rpc_params->bugCode)` (`kernel_gsp.c:1748`), codes incl. `PAGED_SEGMENT 5`, `BSOD_ON_ASSERT 6`. ⊘⊘ **We are the GSP.** We must never emit it, and a guest that can induce us to emit it has a **guest-crash primitive**. ⚠ The code→string table has **drifted** (`g_os_nvoc.h:198-208`) — index 5 reads *"Invalid Bindata Access"* | `rpc_global_enums.h:288` |
| ★★★★ | **WDDM creates every address space with `MINIMIZE_PTETABLE_SIZE`** — *"to reduce the overhead of private address spaces per application, **at the cost of holes in the virtual address space**"* | `nvos.h:1397` |
| ★★★★ | **PTE writes happen at high IRQL on Windows** — *"This code gets called in a high IRQL path on Windows and shadow buffer allocation may fail there"* | `gmmu_walk.c:838` |
| ★★★★ | **The GPU lock is released via a DPC at DIRQL** — *"at high IRQL we can't signal the semaphore, so we use a second pGpu to schedule a DPC"* (three copies) | `core/locks.c:1232, 1333, 1419` |
| ★★★ | **FBSR — the only public description of Windows VRAM eviction**: pre-reserved **pageable** sysmem, pinned + DMA'd, or walked in 64 KB chunks through a pinned scratch. `MEMDESC_FLAGS_PAGED_SYSMEM` is **hard-refused on Linux** | `g_fbsr_nvoc.h:338-347`; `mem_desc.c:367` |
| ★★★ | **A third runlist policy exists solely for WDDM** — `..._CHANNEL_INTERLEAVED_WDDM = 0x2`; and **Linux RM advertises the WDDM-interleaving cap unconditionally** | `ctrl2080fifo.h:698`; `kernel_fifo.c:2842` |
| ★★★ | **`UPDATE_PDE_2` requires `NVOS32_ALLOC_FLAGS_EXTERNALLY_MANAGED`** — the WDDM externally-managed-page-table contract, stated in the control's own doc | `ctrl0080dma.h:600` |
| ★★ | **vGPU Windows guests get an 8 KB GSP debug buffer** the Linux guest does not; and the emulated guest-OS register is `NV_VGPU_GUEST_OS_TYPE {_LINUX 1, _WINDOWS7_DEPRECATED 2, _WINDOWS10 3}` | `vgpu/rpc.c:539`; `vgpu/dev_vgpu.h:41-47` |
| ⊘ | **Clean negative: `WSL`, `GPU-P`, `DDA`, `para-virtualized` → ZERO hits.** No WSL GPU-paravirtualisation residue anywhere | — |

⊘ **The one Windows × non-GSP statement in the tree**: *"since this functionality is supported only
on Linux, and we also need support on Windows, **most of the information is collected in physical
RM itself**, rather than using a Linux OS layer function"* (`gpu/disp/kern_disp.c:815-822`).

---

## 6. ⇒ What this changes

1. ⊘ **Retract the §10.3 pessimism.** The no-GSP oracle position is **far better** than "nouveau
   as a register dictionary". ogkm supplies the object model, the control-plane manifest, the VF
   HAL arms, 42 orphaned physical bodies and the class encodings. ⇒ **nouveau is needed for
   pre-Turing REGISTERS; ogkm still carries the SEMANTICS.** ★ That does **not** change the
   priority ruling — GSP stays the stable target — but it materially lowers the no-GSP cost.
2. ★ **Adopt the VF-arm reading rule** as standing method, beside the ogkm differential.
3. ★ **Keep both driver trees**: 610 for RM-core registers, **580 for pre-Turing UVM**, which 610
   deleted.
4. ⊘ **Fix the BAR1/BAR2 model** if it assumes one flat VA space — the guest expects the split at
   PD0[1], and expects us to pin the BAR1 root.
5. ⊘ **Never emit `TRIGGER_BUGCHECK`**, and treat any path that could induce it as a guest-crash
   primitive.
