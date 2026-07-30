# The GSP boot gate specification — what the stock driver requires, in order

> **What this file is.** An ordered list of every place the stock NVIDIA driver can decide
> our emulated GPU is not a GPU, from `RmInitAdapter` to `GSP_INIT_DONE`. For each gate:
> the precondition (which register must read what, or which memory must hold what), the
> driver's exact check, the error it prints on failure, and the `file:line` it lives at.
>
> **Why it is a deliverable and not a note.** Every failure mode on this path is an
> enumerable `NV_PRINTF(LEVEL_ERROR, …)`. That makes the list a *test suite*: each row is a
> predicate we can assert our own register model satisfies, with no GPU, no VM and no C
> harness. `crates/kayfabe-crec/tests/gsp_boot_gates.rs` is that suite.
>
> **Citation rule** (`mode2_gsp_port_plan.md` §0.1, normative here too): every `ogkm`
> citation carries its tag. `ogkm-580:` = `research_clones/ogkm-580.159.04/` (580.159.04,
> the bench's own driver). `ogkm-610:` = `research_clones/ogkm/` (610.43.02). Line numbers
> belong to the tagged tree only. Paths below are relative to `src/nvidia/` unless noted.

## 0. The isolation verdict this file rests on — measured, not estimated

**The GSP boot code isolates.** Measured on 2026-07-31 against `ogkm-580.159.04`:

| measurement | result |
|---|---|
| `kernel_gsp.c` (5 903 lines) compiled standalone, userspace, `-ffreestanding` | **0 errors**, using only the vendored include paths + the build's `-D` set |
| the whole GSP boot TU set — `kernel_gsp.c`, `_fwsec.c`, `_booter.c`, all of `arch/turing/`, all of `arch/ampere/`, and `generated/g_kernel_gsp_nvoc.c` | **12/12 compiled, 0 failures** |
| undefined symbols across that set | 381 referenced, 519 defined internally, **307 unresolved** |
| a runnable spike: gate G1 (`gpuWaitForGfwBootComplete_TU102`) linked into a userspace binary and driven against a synthetic register plane | **ran; 4/4 cases green**, 21 register reads served to real driver code |
| stubs that spike needed | **26 NVOC class-descriptor data symbols + 7 functions** |

**No NVOC codegen has to be run.** The `g_*_nvoc.{c,h}` files are *vendored* in
`src/nvidia/generated/`. That is the fact the whole approach turns on.

**Two costs the spike measured that an estimate would have missed:**

1. **NVOC objects carry constructor-installed self-pointers.** `staticCast(pKernelGsp,
   KernelFalcon)` reads `pKernelGsp->__nvoc_pbase_KernelFalcon`; a `calloc`'d object has it
   NULL and segfaults. Wiring it is one assignment per base, but it must be done — the
   spike crashed before printing a line until it was.
2. **HAL dispatch is a plain function-pointer field, not a vtable, for `KernelFalcon`.**
   `kflcnWaitForHalt_HAL(...)` expands to `pKernelFlcn->__kflcnWaitForHalt__(...)`
   (`ogkm-580: generated/g_kernel_falcon_nvoc.h:139, 515`). So the HAL table can be
   populated *by hand*, one assignment per slot, and the entire per-chip HAL-init machinery
   in `g_kernel_gsp_nvoc.c` can be skipped — which also makes ~100 of the 307 unresolved
   symbols (`kgsp*_GH100`, `kgsp*_GB202`, the bindata archive getters) disappear. **OBJGPU's
   own HALs are also plain fields** (`gpuFuseSupportsDisplay_FNPTR(pGpu)` =
   `pGpu->__gpuFuseSupportsDisplay__`, `g_gpu_nvoc.h:3575`).

**The seam is two symbols.** `GPU_REG_RD32(g,a)` expands, through `REG_INST_RD32`, to
`regRead032(GPU_GET_REGISTER_ACCESS(g), DEVICE_INDEX_GPU, 0, a, NULL)`
(`ogkm-580: generated/g_gpu_access_nvoc.h:225, 237`), and `GPU_GET_REGISTER_ACCESS(g)` is
`(&(g)->registerAccess)` (`g_gpu_nvoc.h:469`) — a plain field. **Every register read in the
entire driver funnels through `regRead032`/`regWrite032`.** That is exactly the seam we
already own.

### Classification of the 307 unresolved symbols

| class | count | what a harness owes it |
|---|---|---|
| per-chip HAL leaves (`kgsp*_GH100/_GB202/_AD102/…`) + bindata archive getters | ~100 | **nothing** — only referenced from `g_kernel_gsp_nvoc.c`'s HAL table; skip that TU |
| NVOC runtime plumbing (`__nvoc_*` thunks, ctors, dtors, class defs) | ~35 | same, or ~500 bytes of dummy data per class descriptor |
| `regRead032` / `regWrite032` | **2** | ★ **the seam.** Route to our core |
| `memdesc*` (Create/Alloc/Free/Describe/Map/Unmap/GetPhysAddr/…) | 16 | ★ **the real work.** A flat "sysmem + FB" allocator, a few hundred lines |
| `port*` (`portMemAlloc/Copy/Free/Set`, `portString*`) | 9 | libc shims |
| `os*` (`osDelayUs`, `osSpinLoop`, `osGetTimestamp`, `osReadRegistryDword`, …) | 17 | trivial |
| `timeout*` (`timeoutSet`, `timeoutCheck`, `timeoutCondWait`) | 4 | ★ the harness owns virtual time — this is how a 2 s poll becomes 10 iterations |
| `libos*` log decoder | 10 | no-op |
| `bindata*` | 5 | serve a firmware blob, or refuse loudly |
| `GspMsgQueue*` | 5 | compile `message_queue_cpu.c` — it is part of the closure, not outside it |
| RM object framework (`engstate*`, `obj*`, `rmapiLock*`, `gpumgr*`, `serverGetClient*`) | ~40 | stub; a call that reaches one is a finding |
| diagnostics (`prb_*`, `rcdb*`, `crashcat*`, `nvErrorLog*`, `nvDbg_Printf`) | ~35 | ★ `nvDbg_Printf` is where the gate vocabulary below is *captured*; the rest no-op |
| engine cross-calls (`kfifo*`, `krc*`, `knvlink*`, `kdisp*`, `kperf*`, `kpmu*`) | ~30 | stub |

**Understated conclusion:** compilation and linking are settled. What is *not* settled is
how far into `kgspInitRm` a harness gets before it needs a real `memdesc` allocator — the
spike proved gate G1 only, and G1 is register-only. Everything from **G30 onwards touches
allocated memory**, and that is where a harness stops being 33 stubs and starts being a
small memory model.

---

## 1. The ordered gate list

Legend: **[POLL]** = spins on a register until a value appears or a timeout expires;
**[1-SHOT]** = one read, one compare, immediate fail; **[SW]** = pure software/memory.
"we serve" = the emulator must produce this; "we consume" = the driver wrote it and we act.

### Stage 0 — `RmInitAdapter` prologue (`arch/nvalloc/unix/src/osinit.c:1873`)

| # | gate | check | precondition | error text (verbatim) | status |
|---|---|---|---|---|---|
| G0.1 | `RmFetchGspRmImages` `osinit.c:1829` **[SW]** | firmware lookup failed | `gsp_ga10x.bin` present on the guest filesystem | `"No firmware image found\n"` | `NV_ERR_NOT_SUPPORTED` → `RM_INIT_FIRMWARE_FETCH_FAILED` |
| G0.2 | `RmInitDeviceDma` `osinit.c:1905` **[SW]** | DMA mask not settable | guest-side only | `"Cannot configure the device for DMA\n"` | `RM_INIT_GPU_DMA_CONFIGURATION_FAILED` |
| G0.3 | `osInitNvMapping` `osinit.c:1949` **[SW]** | BAR0 unmappable / GPU ID unreadable | **BAR0 must decode** | `"osInitNvMapping failed, bailing out of RmInitAdapter\n"` | varies |
| G0.4 | `kgspInitRm` `osinit.c:2024` **[SW]** | everything below | — | `"Cannot initialize GSP firmware RM\n"` | `RM_INIT_FIRMWARE_INIT_FAILED` |
| G0.5 | `osinit.c:2285` | final banner | — | `"RmInitAdapter failed! (0x%x:0x%x:%d)\n"` | — |

★ `_kgspInitRpcInfrastructure` runs **earlier** than G0.4 — inside
`kgspConstructEngine_IMPL` (`kernel_gsp.c:3607`), reached from `osInitNvMapping`. Its gates
are G4.x below; they are listed there for readability, not in wall-clock order.

### Stage 1 — GFW boot completion — **the first gate a stock driver hits**

★ This is the stage the spike ran end to end.

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| **G1.1** | `kflcnWaitForHalt_TU102` `src/kernel/gpu/falcon/arch/turing/kernel_falcon_tu102.c:345` **[POLL]** | `while (!FLD_TEST_DRF(_PFALCON, _FALCON, _CPUCTL_HALTED, _TRUE, kflcnRegRead_HAL(…, NV_PFALCON_FALCON_CPUCTL)))` | **we serve** GSP falcon `CPUCTL.HALTED = TRUE`. Timeout **2.05 s** (`50 ms + 2 s`, `kern_gpu_tu102.c:431-434`), × `gpuScaleTimeout` | `"Timeout waiting for Falcon to halt\n"` | `NV_ERR_TIMEOUT` |
| **G1.2** | `_gpuIsGfwBootCompleted_TU102` `src/kernel/gpu/arch/turing/kern_gpu_tu102.c:406` **[1-SHOT]** | `if (!FLD_TEST_DRF(_PGC6, _AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK, _READ_PROTECTION_LEVEL0, _ENABLE, regVal))` | **we serve** `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK` with read-PL0 granted — "FWSEC lowered its PLM" | returns FALSE **and forces `progress = 0x0`** | — |
| **G1.3** | same, `kern_gpu_tu102.c:427` **[1-SHOT]** | `return FLD_TEST_DRF(_PGC6, _AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT, _PROGRESS, _COMPLETED, regVal)` | **we serve** `PROGRESS = 0xFF` | — | — |
| G1.4 | `gpuWaitForGfwBootComplete_TU102` `kern_gpu_tu102.c:462` | `if (!bGfwBootCompleted)` | G1.2 ∧ G1.3 | `"failed to wait for GFW_BOOT: (progress 0x%x)\n"` | `NV_ERR_NOT_READY` |
| G1.5 | same, `kern_gpu_tu102.c:451` | `if (status != NV_OK)` from G1.1 | — | `"GSP failed to halt with GFW_BOOT: (progress 0x%x)\n"` | `NV_ERR_TIMEOUT` |
| G1.6 | `kgspWaitForGfwBootOk_TU102` `src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:1273` | wrapper | — | `"failed to wait for GFW boot complete: 0x%x VBIOS version %s\n"` then `"(the GPU may be in a bad state and may need to be reset)\n"` | propagated |

★★ **G1.2 is an ordering trap.** The PLM is read *first*; if it is not lowered the driver
never reads the progress word at all and reports `progress 0x0` regardless of what the
progress register holds. An emulator that serves `PROGRESS=0xFF` but leaves the PLM at
reset prints an error naming the register it *did* answer correctly. The spike found this
by failing: case 1 was written with only `PROGRESS` set and reported `progress 0x0`.

### Stage 2 — VBIOS extraction, BIT parse, FWSEC descriptor

Fully covered by `kayfabe-abi::vbios` (commit `7825926`) and by the VBIOS-oracle work; the
gates are reproduced here only so the ordering is complete. `kgspExtractVbiosFromRom_TU102`,
`src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c`; BIT parse in
`src/kernel/gpu/gsp/kernel_gsp_fwsec.c`.

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| G2.1 | `kernel_gsp_vbios_tu102.c:505` | `if (!IS_VALID_PCI_ROM_SIG(romSig))` | **we serve** `0xAA55` at the ROM signature offset through `NV_PROM_DATA` | `"did not find valid ROM signature\n"` | `NV_ERR_INVALID_DATA` |
| G2.2 | `s_romImgFindPciHeader_TU102` `:254` | ROM-directory magic ≠ `NV_ROM_DIRECTORY_IDENTIFIER` | IFR v0x03 directory | `"Error: ROM Directory not found = 0x%08x.\n"` | `NV_ERR_INVALID_DATA` |
| G2.3 | same `:261` | `default:` on `switch (ifrVersion)` | IFR version ∈ {1,2,3}, `NV_PBUS_IFR_FMT_FIXED0_SIGNATURE` valid | `"Error: IFR version not supported = 0x%08x.\n"` | `NV_ERR_INVALID_DATA` |
| G2.4 | `:519` | `if (biosSizeFromRom > biosSize)` | image ≤ **1 MiB** (`s_getBaseBiosMaxSize_TU102`) | `"expansion ROM has exceedingly large size: 0x%x\n"` | `NV_ERR_INVALID_DATA` |
| G2.5 | `kernel_gsp_fwsec.c:1133` | BIT header id + signature + 8-bit checksum ≡ 0 (`:399-447`) | valid BIT header | `"failed to find BIT header in VBIOS image: 0x%x\n"` | `NV_ERR_GENERIC` |
| G2.6 | `:504` | `bitHeader.TokenSize >= BIT_TOKEN_V1_00_SIZE_6` | — | `"Invalid BIT token size: %u\n"` | `NV_ERR_INVALID_STATE` |
| G2.7 | `:640` | `FLD_TEST_DRF(_BIT, _FALCON_UCODE_DESC_HEADER_VDESC_FLAGS, _VERSION, _UNAVAILABLE, …)` | descriptor declares a version | `"unexpected ucode desc version missing for entry %u (BIT token %u), skipping\n"` | skip |
| G2.8 | `:663` | not V2 (≥`_V2_SIZE_60`) nor V3 (≥`_V3_SIZE_44`) | — | `"unexpected ucode desc version 0x%x or size 0x%x for entry %u (BIT token %u), skipping\n"` | skip |
| G2.9 | `:1142` | no FWSEC entry matched at all | an entry with `ApplicationID == FALCON_UCODE_ENTRY_APPID_FIRMWARE_SEC_LIC` **matching the debug/prod fuse** | `"failed to parse FWSEC ucode desc from VBIOS image: 0x%x\n"` | `NV_ERR_INVALID_DATA` |
| G2.10 | `kernel_gsp.c:4020` | wrapper | — | `"failed to extract VBIOS image from ROM: 0x%x\n"` | propagated |
| G2.11 | `kernel_gsp.c:4006` | wrapper | — | `"failed to parse FWSEC ucode from VBIOS image (VBIOS version %s): 0x%x\n"` | propagated |

★ G2.9's debug/prod selection reads a **fuse** — `kgspIsDebugModeEnabled_HAL` →
`NV_FUSE_OPT_SECURE_GSP_DEBUG_DIS`, whose **address moves between Turing and Ampere+**
(`0x0002174C` → `0x0082074C`). That is one of only two register addresses on the whole path
that move. See §3.

### Stage 3 — ucode image preparation (software only, no registers)

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| G3.1 | `kernel_gsp.c:3961` | `(pGspFw == NULL) \|\| (pGspFw->pBuf == NULL) \|\| (pGspFw->size == 0)` | firmware blob mapped | `"need firmware to initialize GSP\n"` | `NV_ERR_INVALID_ARGUMENT` |
| G3.2 | `kernel_gsp.c:4051` / `:4062` | Booter Load / Unload allocation | bindata archive + **SEC2 fuse version ≤ signature count** (G3.3) | `"failed to allocate Booter Load ucode: 0x%x\n"` / `"…Booter Unload…"` | propagated |
| G3.3 | `s_patchBooterUcodeSignature` `kernel_gsp_booter.c:158` **[1-SHOT]** | `if (numSigs > 1) { if (fuseVer > numSigs - 1) … }`, `sigIndex = numSigs - 1 - fuseVer` | **we serve** the SEC2 ucode-fuse-version register such that `fuseVer ≤ numSigs-1` | `"signature for fuse version %u not present\n"` | `NV_ERR_OUT_OF_RANGE` |
| G3.4 | `kernel_gsp.c:4072` | boot binary prep | GSP-RM boot bindata + RISCV desc | `"Error preparing boot binary image\n"` | propagated |
| G3.5 | `_kgspFwContainerVerifyVersion` `kernel_gsp.c:4730` **[SW]** | `(fwversionSize != expectedVersionLength + 1) \|\| portStringCompare(pFwversion, NV_VERSION_STRING, …) != 0` | the `.fwversion` ELF section of `gsp_ga10x.bin` **must equal the driver's own version string** | `"%s version mismatch: got version %s, expected version %s\n"` | `NV_ERR_INVALID_DATA` |
| G3.6 | `_kgspFwContainerGetSection` `kernel_gsp.c:5066` **[SW]** | ELF magic `0x464C457F`, little-endian, `elfClass64`, `shentsize == sizeof(LibosElf64SectionHeader)`, overflow-checked bounds, `shstrndx <= shnum` | valid ELF | `NV_CHECK` macro prints | `NV_ERR_INVALID_DATA` |
| G3.7 | `kgspCreateRadix3` `kernel_gsp.c:4913` **[SW]** | `NV_ASSERT_OR_RETURN(radix3[0].nPages == 1, NV_ERR_OUT_OF_RANGE)` — **the top PDE must be one 4 KiB page**, which bounds the image size | — | `"VA error for radix3 shared buffer\n"` (`:4976`) | `NV_ERR_OUT_OF_RANGE` |
| G3.8 | `kernel_gsp.c:4080` | wrapper | — | `"Error preparing GSP-RM image\n"` | propagated |

### Stage 4 — LibOS logging, boot args, message queue construction

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| G4.1 | `kernel_gsp.c:4088` **[SW]** | `_kgspInitLibosLoggingStructures` failed | sysmem log buffers allocatable | `"init libos logging structures failed: 0x%x\n"` | `NV_ERR_INVALID_STATE` |
| G4.2 | `kgspAllocBootArgs_HAL` ← `kernel_gsp.c:3644` **[SW]** | `if (nvStatus != NV_OK)` | `pLibosInitArgumentsDescriptor`, `pGspArgumentsDescriptor` allocated | `"boot arg alloc failed: 0x%x\n"` | propagated |
| G4.3 | `_gspMsgQueueInit` `src/kernel/gpu/gsp/message_queue_cpu.c:135` **[SW]** | `if (pMQI->pWorkArea == NULL)` | — | `"Error allocating pWorkArea.\n"` | `NV_ERR_NO_MEMORY` |
| G4.4 | same `:148` **[SW]** | `msgqInit` returned < 0 | — | `"msgqInit failed: %d\n"` | `NV_ERR_GENERIC` |
| G4.5 | same `:155` **[SW]** | `msgqTxCreate(…, GSP_MSG_QUEUE_ELEMENT_SIZE_MIN, GSP_MSG_QUEUE_HEADER_ALIGN, GSP_MSG_QUEUE_ELEMENT_ALIGN, MSGQ_FLAGS_SWAP_RX)` < 0 | shared sysmem page-aligned; writes `tx.version = MSGQ_VERSION` (**= 0**, `src/common/shared/msgq/inc/msgq/msgq_priv.h:38`) | `"msgqTxCreate failed: %d\n"` | `NV_ERR_GENERIC` |
| G4.6 | `GspMsgQueuesInit` `:203` **[SW]** | `if (*ppMQCollection != NULL)` | — | `"GSP message queue was already initialized.\n"` | `NV_ERR_INVALID_STATE` |
| G4.7 | same `:264` **[SW]** | `if (pVaKernel == NvP64_NULL)` | contiguous sysmem for `pageTable + cmdQueue + statusQueue` | `"Error allocating message queue shared buffer\n"` | `NV_ERR_NO_MEMORY` |
| G4.8 | `_kgspInitRpcInfrastructure` `kernel_gsp.c:2528` / `:2540` | wrappers | — | `"GspMsgQueueInit failed\n"` / `"init task RM RPC infrastructure failed\n"` | propagated |
| G4.9 | `_kgspConstructRpcObject` `kernel_gsp.c:2573` | `initRpcObject` returned NULL | — | `"initRpcObject failed\n"` | `NV_ERR_INSUFFICIENT_RESOURCES` |
| G4.10 | `kgspSetupLibosInitArgs_IMPL` `kernel_gsp.c:5141` **[SW, NO failure path]** | ★ builds the region array — `LOGINIT` **must be first** (`:5153`), then the `"RMARGS"` entry whose `pa = memdescGetPhysAddr(pGspArgumentsDescriptor)` | **we consume**: this array is the emulator's only description of where the guest put everything | — | `void` — **a bad entry manifests only as a hang at G11.1** |

★★ G4.10 is the highest-risk gate on the path *for us* precisely because it has no error
path. Everything downstream depends on parsing this array correctly, and getting it wrong
produces silence, not a message. `LibosRegionLayout` in `kayfabe-arch::gsp` is the seam.

### Stage 5 — WPR2 precheck and FB layout

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| **G5.1** | `_kgspBootGspRm` `kernel_gsp.c:3870` **[1-SHOT]** | `if (kgspIsWpr2Up_HAL(…) && !pGpu->getProperty(pGpu, PDB_PROP_GPU_PREINITIALIZED_WPR_REGION))` where `kgspIsWpr2Up_TU102` (`kernel_gsp_tu102.c:1249`) is `DRF_VAL(_PFB, _PRI_MMU_WPR2_ADDR_HI, _VAL, GPU_REG_RD32(…)) != 0` | **we serve** `NV_PFB_PRI_MMU_WPR2_ADDR_HI._VAL == 0` **at cold boot** | `"unexpected WPR2 already up, cannot proceed with booting GSP\n"` then `"(the GPU is likely in a bad state and may need to be reset)\n"` | `NV_ERR_INVALID_STATE` |
| **G5.2** | `kgspPopulateWprMeta_TU102` `kernel_gsp_tu102.c:742` **[1-SHOT]** | `NV_ASSERT_OK_OR_RETURN(kmemsysGetUsableFbSize_HAL(…, &pWprMeta->fbSize))` → `kmemsysReadUsableFbSize_GA102` (`src/kernel/gpu/mem_sys/arch/ampere/kern_mem_sys_ga102.c:48`): `*pFbSize = DRF_VAL(_USABLE, _FB_SIZE_IN_MB, _VALUE, GPU_REG_RD32(pGpu, NV_USABLE_FB_SIZE_IN_MB)) << 20` | ★★ **we serve `NV_USABLE_FB_SIZE_IN_MB` = `NV_PGC6_AON_SECURE_SCRATCH_GROUP_42` = `0x001183A4`.** Zero here silently underflows the whole layout | (assert) | propagated |
| G5.3 | same `:769` | `NV_ASSERT_OK_OR_RETURN(memmgrReadMmuLock_HAL(…, &bIsMmuLockValid, &lo, &hi))` | VBIOS MMU-lock registers; `bIsMmuLockValid = FALSE` is the simple answer | (assert) | propagated |
| G5.4 | same `:866-869` | fills `pWprMeta->revision = GSP_FW_WPR_META_REVISION` (**1**) and `->magic = GSP_FW_WPR_META_MAGIC` (**`0xdc3aae21371a60b3`**) | **we consume** — the Booter validates both; a mismatch surfaces as G8.2 | — | — |
| G5.5 | `_kgspPrepareScrubberImageIfNeeded` `kernel_gsp.c:3733` | `(neededSize > prescrubbedSize) \|\| kgspIsScrubberImageSupported(…)` | `kgspGetPrescrubbedTopFbSize` = **256 MiB** on TU/GA/AD | (macro) | propagated |
| G5.6 | `kgspPrepareForBootstrap_TU102` `kernel_gsp_tu102.c:407` **[SW]** | `if (!IS_GSP_CLIENT(pGpu))` | — | `"IS_GSP_CLIENT is not set.\n"` | `NV_ERR_NOT_SUPPORTED` |
| **G5.7** | same `:413` **[1-SHOT]** | `if (!kflcnIsRiscvCpuEnabled_HAL(…))` → `kflcnIsRiscvCpuEnabled_TU102` (`kernel_falcon_tu102.c:124`): `FLD_TEST_DRF(_PFALCON, _FALCON_HWCFG2, _RISCV, _ENABLE, reg)` | **we serve** GSP falcon `HWCFG2.RISCV = ENABLE` | `"RISC-V core is not enabled.\n"` | `NV_ERR_NOT_SUPPORTED` |

★★★ **G5.2 → G6.3 is an arithmetic chain that couples two registers we serve, and nothing
in this repo stated it until now.** The driver computes, all in
`kgspPopulateWprMeta_TU102`:

```
fbSize             = NV_USABLE_FB_SIZE_IN_MB._VALUE << 20            (kern_mem_sys_ga102.c:48)
vgaWorkspaceOffset = fbSize - DRF_SIZE(NV_PRAMIN)                    (kernel_gsp_tu102.c:761)
                     DRF_SIZE(NV_PRAMIN) = 0x100000  (dev_ram.h:26 -> 0x7FFFFF:0x700000)
gspFwWprEnd        = ALIGN_DOWN64(vgaWorkspaceOffset - wprEndMargin, 0x20000)   (:776)
                     wprEndMargin = 0 unless a regkey is set        (kernel_gsp.c:5637)
frtsOffset         = gspFwWprEnd - kgspGetFrtsSize()                 (:779)
                     frtsSize = 1 MiB on TU/GA/AD  (kernel_gsp_frts_tu102.c:49)
```

and then, at G6.3, **compares our `WPR2_ADDR_LO` against `frtsOffset >> 12` for exact
equality**. So `WPR2_ADDR_LO` is a *function of the FB size we advertise*, not a free
constant. Verified numerically against the C oracle
(`C: src/qemu/mode2_regs_ga10x.h:57-62`): `NVKVM_FB_SIZE_MB = 12288` gives
`frtsOffset = 0x2FFE00000`, `>> 12 = 0x2FFE00`, and the C's `NVKVM_WPR2_LO_VAL = 0x02FFE000`
places exactly `0x2FFE00` in the `31:4` `_VAL` field. It agrees — but by coincidence of
maintenance, not by construction. `crates/kayfabe-crec/tests/gsp_boot_gates.rs` now derives
it.

### Stage 6 — FWSEC / FRTS execution and WPR2 verification

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| G6.1 | `kgspExecuteFwsec_TU102` `kernel_gsp_frts_tu102.c:483` **[POLL inside]** | `kgspExecuteHsFalcon_HAL` failed; inner poll is `kflcnWaitForHalt_HAL(…, GPU_TIMEOUT_DEFAULT, 0)` (`kernel_gsp_falcon_tu102.c:355`) | **we serve** GSP falcon `CPUCTL.HALTED` again after STARTCPU. `GPU_TIMEOUT_DEFAULT` on Linux = **4 s** graphics / **30 s** compute (`arch/nvalloc/unix/src/os.c:1993-2001`) | `"failed to execute FWSEC cmd 0x%x: status 0x%x\n"` | `NV_ERR_TIMEOUT` |
| **G6.2** | same `:497` **[1-SHOT]** | `frtsErrCode = DRF_VAL(_VBIOS, _FWSECLIC, _FRTS_ERR_CODE, GPU_REG_RD32(pGpu, NV_PBUS_VBIOS_SCRATCH(NV_VBIOS_FWSECLIC_SCRATCH_INDEX_0E))); if (frtsErrCode != …_NONE)` | **we serve** `NV_PBUS_VBIOS_SCRATCH(0x0E)` bits **31:16 == 0** (`:133-135`) | `"failed to execute FWSEC for FRTS: FRTS error code 0x%x\n"` | `NV_ERR_GENERIC` |
| **G6.3a** | same `:505` **[1-SHOT]** | `if (wpr2HiVal == 0)` | **we serve** `WPR2_ADDR_HI._VAL != 0` — WPR2 must now be **up** (the inverse of G5.1) | `"failed to execute FWSEC for FRTS: no initialized WPR2 found\n"` | `NV_ERR_GENERIC` |
| **G6.3b** | same `:514` **[1-SHOT]** | `expectedLoVal = (NvU32)(pPreparedCmd->frtsOffset >> NV_PFB_PRI_MMU_WPR2_ADDR_LO_ALIGNMENT); if (wpr2LoVal != expectedLoVal)` | ★★★ **exact equality** against the driver's own arithmetic. `_ALIGNMENT = 0xc`, `_VAL = 31:4` (`src/common/inc/swref/published/turing/tu102/dev_fb.h:34-39`) | `"failed to execute FWSEC for FRTS: WPR2 initialized at an unexpected location: 0x%08x (expected 0x%08x)\n"` | `NV_ERR_GENERIC` |
| G6.4 | same `:528` / `:536` / `:546` **[1-SHOT]** (FWSEC-SB path, driver unload) | PLM lowered; `GFW_BOOT.PROGRESS == COMPLETED`; `NV_PBUS_VBIOS_SCRATCH(0x15)[15:0] == 0` | same registers as G1.2/G1.3 plus SB error code | `"failed to execute FWSEC for SB: GFW PLM not lowered\n"` / `"…GFW progress not completed\n"` / `"…SB error code 0x%x\n"` | `NV_ERR_GENERIC` |
| G6.5 | `:559` | trailer on any of G6.1–G6.4 | — | `"(note: VBIOS version %s)\n"` | — |

### Stage 7 — falcon reset into RISC-V, LibOS boot-args address

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| G7.1 | `kflcnPreResetWait_GA102` `src/kernel/gpu/falcon/arch/ampere/kernel_falcon_ga102.c:213` **[POLL]** | `while (!FLD_TEST_DRF(_PFALCON, _FALCON_HWCFG2, _RESET_READY, _TRUE, hwcfg2))` | **we serve** `HWCFG2.RESET_READY = TRUE`. Timeout **150 µs**; **never fatal** (Bug 3419321 WAR) | — | always `NV_OK` |
| G7.2 | `kflcnWaitForResetToFinish_GA102` `kernel_falcon_ga102.c:254` **[POLL]** | `FLD_TEST_DRF(_PFALCON, _FALCON_HWCFG2, _MEM_SCRUBBING, _DONE, …)` (Turing: `_DMACTL._{D,I}MEM_SCRUBBING`, `kernel_falcon_tu102.c:305`) | **we serve** scrubbing DONE. `GPU_TIMEOUT_DEFAULT` | — | `NV_ERR_TIMEOUT` |
| G7.3 | `kflcnResetIntoRiscv_GA102` `kernel_falcon_ga102.c:84` | writes `NV_PRISCV_RISCV_BCR_CTRL` = `_CORE_SELECT _RISCV \| _VALID _TRUE` | **we consume** | (asserts) | propagated |
| **G7.4** | `kgspProgramLibosBootArgsAddr_TU102` `kernel_gsp_tu102.c:363` **[write]** | writes `NV_PGSP_FALCON_MAILBOX0 = LO32(pa)`, `MAILBOX1 = HI32(pa)` where `pa = memdescGetPhysAddr(pLibosInitArgumentsDescriptor, AT_GPU, 0)` | ★ **we consume** — this is how we learn where the LibOS region array (G4.10) lives | — | — |

### Stage 8 — Booter Load

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| G8.1 | `s_executeBooterUcode_TU102` `kernel_gsp_booter_tu102.c:70` **[POLL inside]** | `kgspExecuteHsFalcon_HAL` on **SEC2** failed | **we serve** SEC2 `CPUCTL.HALTED` within `GPU_TIMEOUT_DEFAULT` | `"failed to execute Booter: status 0x%x, mailbox 0x%x\n"` | `NV_ERR_TIMEOUT` |
| **G8.2** | same `:76` **[1-SHOT]** | `if (mailbox0 != 0)` — read back from SEC2 `NV_PFALCON_FALCON_MAILBOX0` | ★ **we serve SEC2 `MAILBOX0 == 0`.** The driver wrote the physical address of the `GspFwWprMeta` there (G5.4); a real Booter replaces it with an error code | `"Booter failed with non-zero error code: 0x%x\n"` | `NV_ERR_GENERIC` |
| G8.3 | `kgspExecuteBooterLoad_TU102` `:119` / `kgspBootstrap_TU102` `kernel_gsp_tu102.c:538` | wrappers | — | `"failed to execute Booter Load: 0x%x\n"` / `"failed to execute Booter Load (ucode for initial boot): 0x%x\n"` | propagated |

### Stage 9 — RISC-V liveness

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| **G9.1** | `kgspBootstrap_TU102` `kernel_gsp_tu102.c:551` **[1-SHOT]** | `if (kflcnIsRiscvActive_HAL(…) \|\| _kgspIsProcessorSuspended(…)) … else` | **we serve** `NV_PRISCV_RISCV_CPUCTL._ACTIVE_STAT = _ACTIVE` (GA102+, `kernel_falcon_ga102.c:47`; Turing reads `NV_PRISCV_RISCV_CORE_SWITCH_RISCV_STATUS` instead) | `"Failed to boot GSP.\n"` | `NV_ERR_NOT_READY` |
| G9.2 | `_kgspIsProcessorSuspended` `kernel_gsp_tu102.c:1224` **[teardown]** | `return (mailbox == 0x80000000);` — ★ **exact equality at 580**; 610 masks | **we serve** `NV_PGSP_FALCON_MAILBOX0 == 0x80000000` exactly, replacing the boot-args echo | — | — |
| G9.3 | `kgspExecuteBooterUnloadIfNeeded_TU102` `kernel_gsp_booter_tu102.c:176` **[1-SHOT, unload]** | non-GC6: `if (kgspIsWpr2Up_HAL(…))` | **we serve** `WPR2_ADDR_HI._VAL == 0` after unload — this is what makes the *next* boot pass G5.1 | `"failed to execute Booter Unload: WPR2 is still up\n"` | `NV_ERR_GENERIC` |

### Stage 10 — status queue link

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| **G10.1** | `GspStatusQueueInit` `message_queue_cpu.c:311` **[POLL]** | `msgqRxLink(hQueue, pStatusQueue, statusQueueSize, GSP_MSG_QUEUE_ELEMENT_SIZE_MIN)` until it returns 0. Timeout **4 s** (`NV_U32_MAX` on emu/sim) | ★★ **we serve — by writing guest memory.** The GSP must have written its own `msgqTxHeader` into the status-queue page: `version` (**= 0**), `size`, `msgSize`, `entryOff`, `msgCount`. Failure codes (`src/common/shared/msgq/msgq.c:331-441`): `-2` msgSize too small, `-3` msgSize > size, `-7` size mismatch, `-8` msgSize mismatch, **`-9` version mismatch**, `-10` derived-field sanity (`rxHdrOff >= sizeof(msgqTxHeader)`, `entryOff >= rxHdrOff + sizeof(msgqRxHeader)`, `msgCount == (size - entryOff)/msgSize`) | `"msgqRxLink failed: %d, nvStatus 0x%08x, retries: %d\n"` | `NV_ERR_TIMEOUT` / `NV_ERR_RESET_REQUIRED` |
| G10.2 | same loop | `if (!kgspHealthCheck_HAL(…))` each iteration | CrashCat queue must not report a fatal | Xid 120 + `"****************************** GSP-CrashCat Report ****…"` | `NV_ERR_RESET_REQUIRED` |

### Stage 11 — `GSP_INIT_DONE`

| # | gate | check | precondition | error text | status |
|---|---|---|---|---|---|
| **G11.1** | `kgspWaitForRmInitDone_IMPL` `kernel_gsp.c:5214` **[POLL]** | `rpcRecvPoll(pGpu, pRpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0)` then `NV_ASSERT_OK_OR_RETURN(RPC_HDR->rpc_result)` | ★★★ **we serve** — a `GSP_INIT_DONE` element in the status queue with `rpc_result == NV_OK` | `NV_CHECK`/`NV_ASSERT_OK` macro prints | `NV_ERR_TIMEOUT` |
| G11.2 | `_kgspRpcRecvPoll` `kernel_gsp.c:2355-2382` **[POLL]** | timeout = `defaultus + defaultus/2` = **6 s** graphics / **45 s** compute | — | on timeout `_kgspLogXid119`; at 3 back-to-back, `NV_ASSERT_FAILED("Back to back GSP RPC timeout detected! GPU marked for reset")` | `NV_ERR_TIMEOUT` |
| G11.3 | `GspMsgQueueReceiveStatus` `message_queue_cpu.c:608-781` **[SW, 3 retries]** | `_checkSum32(elem, HDR_SIZE + rpc.length) != 0`; `elem->seqNum != pMQI->rxSeqNum`; `msgLen < sizeof(GSP_MSG_QUEUE_ELEMENT) \|\| msgLen > GSP_MSG_QUEUE_ELEMENT_SIZE_MAX` | ★ **we serve** a correct 32-bit checksum, a monotone `seqNum`, and a length in range | `"Bad checksum.\n"` / `"Bad sequence number.  Expected %u got %u. Possible memory corruption.\n"` / `"Incorrect message length %u\n"` / `"Incomplete read.\n"` | `NV_ERR_INVALID_DATA` / `NV_ERR_NOT_READY` / `NV_ERR_INVALID_PARAM_STRUCT` |
| G11.4 | `_kgspRpcSanityCheck` `kernel_gsp.c:281-325` **[SW, every iteration]** | `bFatalError` → `NV_ERR_RESET_REQUIRED`; GPU in reset / lost / shutdown / not full power / no sysmem access | — | all `LEVEL_INFO`: `"GSP crashed, skipping RPC\n"` etc. | as listed |
| G11.5 | `kernel_gsp.c:4228` / `:4235` | `SET_GUEST_SYSTEM_INFO` / `GET_GSP_STATIC_INFO` RPCs after INIT_DONE | ★ we must **answer** these, not just accept them | `"SET_GUEST_SYSTEM_INFO failed: 0x%x\n"` / `"GET_GSP_STATIC_INFO failed: 0x%x\n"` | propagated |
| G11.6 | `kernel_gsp.c:4211` | `pKernelGsp->bootAttempts >= maxGspBootAttempts` | the whole of stages 5–11 is inside a retry loop (`:4174-4197`), bounded by `NV_REG_STR_RM_GSP_BOOT_RETRY_ATTEMPTS`; retries fire only when `gpuCheckEccCounts_HAL` or ECC-disabled | `"Max GSP-RM boot attempts exceeded: %d/%d\n"` | last boot status |

---

## 2. Every poll, and its timeout — the harness's virtual clock

A harness that runs this code must own `timeoutCheck`; these are the budgets it emulates.

| gate | polled fact | timeout |
|---|---|---|
| G1.1 | GSP `CPUCTL.HALTED` (GFW boot) | **2.05 s** (`50 ms + 2 s`), × `gpuScaleTimeout` |
| G6.1, G8.1 | GSP / SEC2 `CPUCTL.HALTED` (FWSEC, Booter) | `GPU_TIMEOUT_DEFAULT` = **4 s** graphics / **30 s** compute |
| G7.1 | `HWCFG2.RESET_READY` | **150 µs**, never fatal |
| G7.2 | `HWCFG2._MEM_SCRUBBING` / `DMACTL._{D,I}MEM_SCRUBBING` | `GPU_TIMEOUT_DEFAULT` |
| G9.2 | `MAILBOX0 == 0x80000000` | `GPU_TIMEOUT_DEFAULT` |
| G10.1 | `msgqRxLink` on the GSP-written tx header | **4 s** |
| G11.1/2 | `GSP_INIT_DONE` in the status queue | **6 s** graphics / 45 s compute |

---

## 3. Which gates are genuinely chip-specific — and which are a table row

Read `mode2_gsp_port_plan.md` for the seam; this is the data.

**GA10x boot is overwhelmingly the Turing code path.** Of ~30 `kgsp*` HAL slots on the boot
path, GA102–GA107 dispatch to a `_TU102` implementation in **all but four**:
`kgspConfigureFalcon_GA102`, `kgspGetGspRmBootUcodeStorage_GA102`,
`kgspExecuteSequencerCommand_GA102`, `kgspExecuteHsFalcon_GA102` — and only the last is a
materially different *sequence* (DMA + BROM signature registers instead of PIO IMEM/DMEM).
**Ada AD10x dispatches to `_TU102` or `_GA102` for literally every boot HAL slot**; the only
Ada-specific code in the whole GSP tree is `kgspExecuteScrubberIfNeeded_AD102`.

### Addresses that DO NOT move — no table row needed

`NV_PGSP` base `0x110000`; `NV_FALCON2_GSP_BASE` `0x111000`; `NV_PGSP_FBIF_BASE` `0x110600`;
`NV_PGSP_FALCON_MAILBOX0` `0x110040`; `NV_PGSP_QUEUE_HEAD(i)` `0x110c00 + i*8`;
`NV_PGSP_FALCON_ENGINE` `0x1103c0`; **all** `NV_PFALCON_FALCON_*` engine offsets
(`MAILBOX0 0x40`, `OS 0x80`, `HWCFG2 0xf4`, `CPUCTL 0x100`, `BOOTVEC 0x104`, `DMACTL 0x10c`,
`CPUCTL_ALIAS 0x130`); `NV_PFB_PRI_MMU_WPR2_ADDR_LO/HI` `0x1FA824`/`0x1FA828`; all
`NV_PGC6_AON_SECURE_SCRATCH_GROUP_03/05` + `PRIV_LEVEL_MASK`; `NV_PGC6_BSI_SECURE_SCRATCH_14`.
Ampere and Ada compile the WPR2 and falcon gates against the **Turing** headers.

### The four things that ARE chip-specific on the TU→GA→AD range

| fact | TU10x | GA10x | AD10x | why |
|---|---|---|---|---|
| `NV_FUSE_OPT_SECURE_GSP_DEBUG_DIS` | `0x0002174C` | `0x0082074C` | `0x0082074C` | sole reason `kgspIsDebugModeEnabled` is halified; feeds **G2.9** |
| `IMEMC_BLK`/`DMEMC_BLK` field width | `15:8` | `23:8` | `23:8` | sole reason `kflcnMask{I,D}memAddr` is halified |
| `NV_PRISCV_RISCV_CPUCTL` | `0x268` | `0x388` | `0x388` | **G9.1** reads a different register on Turing (`CORE_SWITCH_RISCV_STATUS` `0x240`, which does not exist on GA102+) |
| `NV_PRISCV_RISCV_BCR_CTRL` | **absent** | `0x668` | `0x668` | **G7.3** does not exist on Turing at all |

Plus pure constants: `ememPort` 0/2/2, libos version 2/3/3, WPR heap min/max/carveout MB
64/256/0 → 88/280/22, `bBootFromHs` false/true/true, HS-falcon load method PIO/DMA/DMA,
`NV_USABLE_FB_SIZE_IN_MB` (**Ampere+ only** — it is not in the Turing header).

**⇒ Adding Ada is a table row.** The only non-constant is emulating
`NV_PGC6_BSI_SECURE_SCRATCH_15[31:29] == 3` so `_kgspIsScrubberCompleted` short-circuits —
a one-register fake, not new logic.

**⇒ Hopper GH100 is NEW LOGIC, not a row.** Different boot model entirely: FSP/SEC2
chain-of-trust boots a partitioned GSP-FMC; **no Booter, no FWSEC-FRTS, no VBIOS ROM parse,
and RM does not compute the WPR layout.** The entry points needing genuinely new
implementations are `kgspPrepareForBootstrap`, `kgspSetupGspFmcArgs` (does not exist
pre-Hopper), `kgspBootstrap`, `kgspPopulateWprMeta`, `kgspTeardown`, `kgspResetHw` (same
register `0x1103c0`, but `_ASSERT`/`_DEASSERT` encoding **and** a `RESET_STATUS` poll),
`kgspWaitForGfwBootOk` (→ `kfspWaitForSecureBoot`, polling
`NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE`). Stages 1, 2, 6 and 8 of this document **do not
exist** on Hopper.

---

## 4. Where 580 and 610 disagree on this path

Diffed directly, `diff -q` for identity.

### Byte-identical at both tags — these gates are version-stable

`kernel_gsp_fwsec.c` (1158 lines), `kernel_gsp_booter.c` (490), `kernel_gsp_frts_tu102.c`,
`kernel_gsp_vbios_tu102.c`, `kernel_gsp_ga100.c`, `g_rpc-message-header.h`,
`message_queue.h`, `libos_init_args.h`, `gsp_fw_wpr_meta.h` (so
`GSP_FW_WPR_META_MAGIC`/`_REVISION` are unchanged), and the **msgq library itself**
(`msgqTxHeader` = 7×`NvU32`, `msgqRxHeader`, byte-identical — only the file moved from
`src/common/shared/msgq/` at 580 to `src/nvidia/src/libraries/msgq/` at 610).

⇒ **Stages 2, 6, 8 and the G10.1 msgq algebra are the same specification at both tags.**

### The GSP queue — NOT wire-compatible

The break is entirely in the **GSP element header layered on top of msgq**.

| | 580 (`inc/kernel/gpu/gsp/message_queue_priv.h:43-51`) | 610 (`:52-67`) |
|---|---|---|
| off 0 | `NvU8 authTagBuffer[16]` | `NvU32 mctpHeader` |
| off 4–12 | — | `nvdmHeader`, `checkSum`, `seqNum` |
| off 16 | `NvU8 aadBuffer[16]` | `NvU8 payload[]` |
| off 32/36 | `checkSum`, `seqNum` | — |
| off 40 | **`NvU32 elemCount`** | **field deleted** |
| off 48 | `rpc_message_header_v rpc` | — |
| **header size** | **48 bytes, fixed** (`:93`) | **16 non-CC / 56 CC** (`message_queue_cpu.c:82-86`) |

- **Element count**: 580 *transmits* it (`message_queue_cpu.c:482`) and *trusts the wire
  field* on receive (`:658`). 610 *derives* it on both ends and puts nothing on the wire
  (`:503`, `:690-701`).
- **CC fields**: 580 puts `authTag`/`aad` unconditionally in every element. 610 factors them
  into an optional 40-byte `GSP_MSG_QUEUE_ENCRYPTION_TAG` present only under CC.
- **New mandatory magics at 610**: `MCTP_HEADER_VERSION == 0x1`,
  `MCTP_MSG_HEADER_VENDOR_ID_NV == 0x10de`, `NVDM_TYPE_RM_RPC == 0x25`, **rejected on
  receive** (`message_queue_cpu.c:737-760`) — a check 580 does not have.
- **Nine compile-time macros → runtime fields**: 580's `GSP_MSG_QUEUE_ELEMENT_SIZE_MIN/MAX`,
  `_HDR_SIZE`, `_ALIGN`, `_HEADER_SIZE/_ALIGN`, `_BYTES_TO_ELEMENTS`, `_RPC_SIZE_MAX`
  (`message_queue_priv.h:91-104`) all deleted at 610 and stored per-queue. **The values are
  unchanged** (4 KiB min, 64 KiB max, `RM_PAGE_SHIFT` alignment, header align 4).
- **610 negotiates**: `MESSAGE_QUEUE_INIT_ARGUMENTS` grows 4 → 9 members
  (`gsp_init_args.h:29-34` vs `:32-42`), publishing `queueElementHdrSize/SizeMin/SizeMax/
  HeaderAlign/ElementAlign` to the firmware. **That channel does not exist at 580.**
- `rxSeqNum` bump moved out of the success path (580 `:774-783` conditional; 610 `:836-842`
  unconditional, before `msgqRxMarkConsumed`).

### Other 580↔610 differences that touch this document

| area | 580 | 610 | class |
|---|---|---|---|
| **CPU sequencer** | present: `rmgspseq.h`, `kgspExecuteSequencerCommand` HAL, the whole `GSP_SEQ_BUF_OPCODE_*` interpreter (`kernel_gsp.c:5259-5400`, `kernel_gsp_tu102.c:900-985`, `kernel_gsp_ga102.c:122-206`) | **entirely removed**; replaced by `GSP_LOAD_EXEC_GENERIC_BOOTLOADER` / `_HS_BINARY` events | behavioural |
| **init-RPC ordering** | queued *into the ring before boot* (`kgspQueueAsyncInitRpcs_IMPL`, `kernel_gsp.c:3754`, called `:4141`) with an SPDM `bDelayInitRpcs` escape | sent *after Booter Load, before `FALCON_OS`* (`kgspSendInitRpcs_IMPL`, `kernel_gsp.c:4686`; call site `kernel_gsp_tu102.c:574-584`); SPDM branch deleted | behavioural |
| **G9.2 suspend sentinel** | `return (mailbox == 0x80000000);` — **exact equality**, inline literal; the symbol `INTERRUPT_PROCESSOR_SUSPENDED_VALUE` does not exist at 580 | `(mailbox & INTERRUPT_PROCESSOR_SUSPENDED_VALUE) != 0` | behavioural ★ 580 governs |
| **RPC function enum** | `NUM_FUNCTIONS 229` | `230` (`+CTRL_GPU_SET_MIGRATION_BLOCK`) | structural |
| **RPC event enum** | `NUM_EVENTS 0x1023` | `0x102a`; ★ `0x1023` **reused** for a different meaning | structural |
| `GSP_ARGUMENTS_CACHED` | `gsp_init_args.h:36-64` | `:44-83`, **+32 bytes** (`rmStateMonitorBufferArgs`, `bindataArgs`) | structural |
| `GSP_ACR_BOOT_GSP_RM_PARAMS` | `gspifpub.h:59-73` | `:59-75`, `+NvBool bInstInSysMode` ⇒ `GSP_FMC_BOOT_PARAMS` layout shifts | structural |
| `GspStaticConfigInfo` | `gsp_static_config.h:78-169` | `:80-167`; removes `grCapsBits[]`, `fbio_mask`, `fb_bus_width`, `fb_ram_type`, `fbp_mask`, `l2_cache_size`, `gpuNameString_Unicode[]`; adds `bPdiValid`/`pdi`, `vbiosRevision` | structural — **G11.5 answers this** |
| **boot-retry ECC gate** | `gpuCheckEccCounts_HAL(pGpu) \|\| (bEccDisabled && !hypervisorIsVgxHyper())` | `… \|\| bEccDisabled` | behavioural |

---

## 5. What this repo can answer today, and what it cannot

Checked against `crates/kayfabe-arch/src/gsp.rs` (`GspReg`) and
`crates/kayfabe-crec/src/ga10x.rs` (`Ga10xGspModel`).

| gate | representable? | note |
|---|---|---|
| G1.1 GSP `CPUCTL.HALTED` | ✔ `GspFalconCpuctl` → `CPUCTL_HALTED` | |
| G1.2 GFW PLM | ✔ `GfwBootPlm` → `0xFFFF_FFFF` | |
| G1.3 GFW progress | ✔ `GfwBootProgress` → `0xFF` | |
| G5.1/G6.3a WPR2 hi | ✔ `Wpr2AddrHi`, gated on `obs.wpr2_up` | |
| **G5.2 usable FB size** | ✘ **and it must stay out of `GspReg`** | see the box below |
| **G6.3b WPR2 lo exact** | ⚠ was a hand-written literal → **now derived** | ★ the doc comment claimed "only zero-vs-nonzero is load-bearing", citing only `kgspIsWpr2Up_TU102`. **G6.3b is an exact compare.** Corrected; derived from `FB_SIZE_MB` by the driver's own chain and pinned by a test |
| G5.7 `HWCFG2.RISCV` | ✔ `GspFalconHwcfg2` → `HWCFG2_RISCV_ENABLE` | |
| G7.1 `HWCFG2.RESET_READY` | ✘ | never fatal (150 µs WAR) — a real gap, but a benign one |
| G7.2 mem scrubbing done | ✘ | **not benign**: `GPU_TIMEOUT_DEFAULT`, and it is on the FWSEC reset path |
| G7.4 boot-args mailboxes | ✔ `GspFalconMailbox0/1` echo | |
| G8.2 SEC2 `MAILBOX0 == 0` | ✔ `Sec2FalconMailbox0` → 0 | |
| G9.1 RISC-V active | ✔ `GspRiscvCpuctl`, gated on `obs.riscv_active` | |
| G9.2 suspend sentinel | ✔ **replaces**, never ORs — 580's exact-equality | |
| G6.2 FRTS error code | ✘ | `NV_PBUS_VBIOS_SCRATCH(0x0E)` unrepresentable |
| G6.4 SB error code | ✘ | `NV_PBUS_VBIOS_SCRATCH(0x15)` unrepresentable |
| G3.3 SEC2 ucode fuse version | ✘ | reads 0 ⇒ `sigIndex = numSigs-1`, which passes — benign *by luck*, not by design |
| G2.9 GSP debug fuse | ✘ | reads 0 ⇒ prod-signed FWSEC selected; must match what `kayfabe-abi::vbios` emits |
| G10.1, G11.1–11.5 | ✔ `kayfabe-gsp` (`ring`, `element`, `rpc`, `boot`) | |

**Six named gaps, all of the form "a register the driver reads and we cannot name".** Four
of them (G6.2, G6.4, G3.3, G2.9) currently read as zero and happen to pass; two (G7.1, G7.2)
are polls that would hang. None of this is visible to a trace replay, because a replay feeds
the C's recorded stimulus rather than reacting to our answers — which is exactly the
argument for the harness.

### ★★ G5.2 — a gap that is real, in a place that is wrong

`NV_USABLE_FB_SIZE_IN_MB` (`0x001183A4`) is read by the driver on the boot path and served
by **nothing** in this repo. Adding it to `GspReg` was tried on 2026-07-31 and **reverted**,
for two independent reasons, both measured:

1. **It violates `GspModel`'s own stated rule.** `kayfabe-arch/src/gsp.rs` says: *"a register
   whose served value is a function of the GSP boot FSM's state belongs here; every other
   register does not."* This one is a devinit constant — it is published before the driver
   looks and no FSM transition changes it. It belongs with PTIMER and the fuses, on the open
   side of the seam (`mode2_gsp_port_plan.md` §11-O1).
2. **The C's `cap1_coldboot_hermetic` capture reads that address exactly 3 times.** Teaching
   `Ga10xGspModel::decode_reg` to name it moved **every positional golden** in
   `crates/kayfabe-crec/tests/cap1_differential.rs` by +3 — decoded transactions
   1955→1958, register plane 498→501, closure limit 978→981, first divergence 255→258, and
   every census `txn` index. A control run with only that one decode row removed returned
   all of them to their prior values, which is how the count of 3 was established.

⇒ **The gap stays open and is now named rather than invisible.** The load-bearing part —
that `Wpr2AddrLo` is a *function* of the FB size and not a free constant — is fixed
regardless: `WPR2_LO_UP` and `WPR2_HI_UP` are now computed by the driver's own chain from a
single `pub const FB_SIZE_MB`, so whichever plane ends up answering `0x001183A4` has one
constant to share. The C keeps exactly one too
(`C: src/qemu/nvkvm_gpu_emul.c:1546` answers the address with `NVKVM_FB_SIZE_MB`).

★ Side effect worth keeping: because the derivation is a `const fn`, an FB size too small to
hold PRAMIN + FRTS is now a **compile error** (`E0080`, attempt to subtract with overflow in
a constant), not a runtime surprise.

---

## 6. What generalising the VBIOS oracle harness costs

The VBIOS-parser harness (in flight, separate agent) feeds an image through a stubbed
`GPU_REG_RD32`. Generalising it to the GSP boot path needs, measured above:

| item | cost |
|---|---|
| compile the GSP TU set | **zero** — same include set and `-D` set; 12/12 already compile |
| add `regRead032`/`regWrite032` | **2 symbols**, if the VBIOS harness stubbed `GPU_REG_RD32` at a higher level it already has the plumbing |
| the object graph | ~15 lines: `calloc(sizeof(OBJGPU))`, `calloc(sizeof(KernelGsp))`, `children.named.pKernelGsp`, `__nvoc_pbase_*`, `timeoutData.scale` |
| the HAL table | one assignment per slot used, ~20 for the whole path |
| `timeoutSet`/`timeoutCheck`/`timeoutCondWait` | ~30 lines — **the harness owns virtual time**, which is what turns a 2 s poll into ten iterations |
| `nvDbg_Printf` | ~20 lines — ★ this is where the error vocabulary in §1 is *captured*, so a gate failure becomes a test assertion on the exact string |
| `port*` + `os*` | ~60 lines of libc shims |
| NVOC class descriptors | 26 dummy objects, one macro |
| **`memdesc*`** | ★ **the only real work: a flat sysmem/FB allocator, a few hundred lines.** Needed from stage 3 onward; not needed at all for stages 0–1 |

⇒ Stages 0–1 are essentially free once their harness lands. Stages 3–11 are gated on
`memdesc`, and that is the honest boundary of the estimate.
