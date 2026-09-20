# The Windows axis — what a Windows guest actually changes

**STATUS: LIVE, 2026-09-20 (w821).** First survey of the `OS` axis (§0 of
`THE_ARCHITECTURE_v3.md`), which until now carried **zero coverage**. Sources: ogkm 610.43.02
and 580.159.04 (the shared RM core), plus lawfully published Microsoft/NVIDIA documentation,
vendor bulletins, open-source projects and conference talks.

⊘ **No leaked source or breach material was used.** Two cited items are published security
research that reverse-engineers a *shipped binary* (Google Project Zero; a public patcher repo) —
legitimate published work, flagged where used. One conference deck was encountered and
deliberately **not** relied on; nothing here depends on it.

---

## 1. ⚠ The headline is a PREMISE problem, not a compatibility problem

> ★★★★★ **GSP is off by default on a stock Windows guest.**

The firmware blobs ship in the DriverStore, `nvidia-smi -q` reports
`GSP Firmware Version : N/A`, and the feature is turned on by a **guest registry DWORD**:
`…\Class\{4d36e968-…}\0000\EnableGpuFirmware = 1`. Community-confirmed on 30-, 40- and
**50-series (Blackwell)** parts, with side effects (HDCP loss, ~300 MB VRAM, DSC/G-SYNC issues).

★ **That key name is not a coincidence, and ogkm corroborates the path from NVIDIA's own code.**
The Linux macro is `#define __NV_ENABLE_GPU_FIRMWARE EnableGpuFirmware` (`kernel-open/nvidia/nv-reg.h`)
— one cross-OS registry abstraction. And the Windows GSP-client path demonstrably exists in the
shared core: `RMCFG_FEATURE_PLATFORM_WINDOWS && IS_GSP_CLIENT(pGpu)` (`gpu_registry.c:224`,
`gpu_user_shared_data.c:335`), plus a Windows-only enable *inside* `GSP_SET_SYSTEM_INFO`
(`kernel_gsp.c:4626`).

⇒ ⊘⊘⊘ **Our entire architecture is "we are the GSP." On a stock Windows guest there is no GSP to
be.** The guest drives the hardware from CPU-RM, underneath the whole WDDM DDI contract — a far
larger emulation surface than the one v3 describes.

⚠ Newest datapoint is **September 2025** (drivers 581.x); **no 2026 datapoint either way.**
★ The one-line experiment if this is ever pursued: set `EnableGpuFirmware=1` in the guest and look
for an RPC message queue.

---

## 2. Under WDDM the OS owns the page tables — and we never see the writes

Microsoft, verbatim: *"**VidMm manages the GPU virtual address space of all processes. VidMm is
also responsible for allocating, growing, updating, ensuring residency, and freeing page
tables.**"* NVIDIA's side matches: VA spaces are `SHARED_MANAGEMENT` (*"management of the VA space
is shared with another component (e.g. driver layer, OS…)"*, `nvos.h:1384`), and ranges are
`NVOS32_ALLOC_FLAGS_EXTERNALLY_MANAGED` — *"Page tables for this allocation will be managed
outside of RM."*

⊘⊘⊘ **And the transport for individual PDEs is invisible to us.**
`NV0080_CTRL_CMD_DMA_UPDATE_PDE_2` (`0x80180f`) is documented **"only available on Windows and
MODS platforms"** and **formats the PDE into the caller's own buffer** rather than RM's directory.
Critically, it is **handled purely in guest CPU-RM and is NEVER RPC'd to GSP for a GSP client**
(`dma.c:320-324` RPCs only under `IS_VIRTUAL_WITHOUT_SRIOV`; `:340-355` does the work locally).
Same for `NV90F1_CTRL_CMD_VASPACE_RESERVE_ENTRIES` (`gpu_vaspace.c:4109`).

⇒ **The address model in Part 1 §4 assumes RM authorship of page tables. On Windows that
assumption is false**, and our fake GSP hears nothing.

★ **One thing survives:** `SET_PAGE_DIRECTORY` **is** RPC'd to GSP when `IS_GSP_CLIENT`
(`dma.c:507-521`). ⚠ But with different semantics — aperture typically `_VIDMEM` not
`_SYSMEM_COH`, arriving per device creation rather than per UVM VA space.

★★★ **And the silver lining, which is real:** unlike Linux, Windows *tells the driver every PTE it
wants written, explicitly and in order*, through `DxgkDdiBuildPagingBuffer` ops
(`UpdatePageTable`, `FlushTlb`, `MapApertureSegment`) submitted as **GPU-executed paging
buffers**. ⇒ If we can see paging-buffer submissions, that is a **richer** signal than the
doorbell-time page-table sweep the C artifact used. ⊘ **[UNVERIFIED]** whether NVIDIA's WDDM path
routes those through CE (visible to us), BAR1 CPU writes, or `UPDATE_PDE_2` (invisible). **This is
the single most important open question on this axis.**

---

## 3. TDR — a hard, OS-enforced 2-second deadline

| | Linux | Windows |
|---|---|---|
| graphics timeout | 4 s | ★ **2 s**, OS-enforced |
| compute timeout | 30 s | 2 s (5 s `TdrDdiDelay`) |
| consequence of exceeding | the driver soldiers on with stale TLBs | **adapter reset**; `VIDEO_TDR_FAILURE (0x116)` |
| repeat | — | ⊘ **five hangs in one minute ⇒ bugcheck `0x117`** |

RM's own comment concedes it: *"90 % of the overall hard limit of **2.0 seconds, imposed by WDDM
driver rules**"* (`os.c:2116`).

⇒ ⊘ **On Linux a slow emulator is merely slow. On Windows a completion path slower than 2 s is
diagnosed as a hung GPU.** ★ Cold boot is probably safe — device init gets a 60 s exception
(`thread_state.c:331`) — but **runtime is not**, and our data plane is currently a CPU copy loop.

⚠ Related and worse: Windows has a **first-class page-fault interrupt** carrying
`{FaultedVirtualAddress, PageTableLevel, FaultErrorCode, FaultedProcessHandle}` and the OS
*expects* an accurate fault path, using it to kill the offending context rather than the adapter.
⊘ The C artifact went green *"without servicing or forwarding a single GPU fault."* That is a
**Linux-only luxury**.

---

## 4. Four more that matter, briefly

- ⊘ **No UVM on Windows.** *"Currently, we don't support UVM on Windows/MODS"* (`pool_alloc.c:488`);
  in WDDM mode CUDA contexts are dxgkrnl contexts (`D3DKMT_CLIENTHINT_CUDA`). ⇒ The `UVM_*` ioctl
  stream **disappears** — and `UVM_MAP_EXTERNAL_ALLOCATION` is the lockstep boundary of our nvdiff
  oracle (221 of `cuCtxCreate`'s 479 ioctls).
- ⊘ **The OS manages the framebuffer** on non-TCC Windows — RM verbatim: *"Disabling in Non-TCC
  windows because **the OS manages FB**"* ⇒ **no scrub-on-free**, and RM stops choosing PTE kinds
  (`bRmToChooseKind = NV_FALSE`). ⇒ The scrub path in Part 5 is one a Windows guest largely does
  not take.
- ⊘ **Sysmem GPU mappings become 4 KB-granular** where Linux uses 64 KB/2 MB — a 16–512× PTE-count
  increase. ⚠ And Windows **guarantees** VA/PA 64 KB congruence and lets the driver *rely* on it, so
  a violating emulator faults with no obvious cause — the same class as this tree's existing
  congruence trap.
- ★ **`DEV_xxxx` must already be in NVIDIA's signed INF.** PnP binds by hardware id from a
  catalog-signed package, and *"even a single-byte change… invalidates the digital signature."*
  ⇒ **A gate before any of the above matters.**

★ **And one that lands directly on w821's Hopper work:** RM allocates the BAR1 doorbell region
**downwards from the top of BAR1 on Windows** — *"WAR for Bug 3564398, need to allocate doorbell
for windows differently"* ⇒ `BUS_MAP_FB_FLAGS_MAP_DOWNWARDS` for `MEMDESC_FLAGS_MAP_SYSCOH_OVER_BAR1`
(`mapping_cpu.c:506`), **the exact flag Hopper+ sets on the usermode/doorbell region**
(`kernel_fifo_gh100.c:123`). ⇒ Our BAR1 doorbell-page identification (Part 1 §2.1) must not assume
Linux allocation order.

---

## 5. ★★★ The escape hatch — TCC, and it collapses most of the table

**TCC mode is not WDDM at all** — it *"uses the Windows WDM driver model"* and disables graphics
(`nvidia-smi -dm 1`). ★ And nearly every Windows branch found in ogkm is gated on **`!TCC`**:
FBSR-WDDM mode, PMA client page tables, P2P mailbox, scrub-on-free. `nv_gpu_ops.c:2327` states it
directly: *"**Non-TCC mode on Windows implies WDDM mode.**"*

⇒ **In TCC: scrub-on-free returns, PMA-managed client page tables return, `EXTERNAL_HEAP_CONTROL`
is off, FBSR-WDDM is off, and dxgkrnl/VidMm/VidSch are entirely out of the loop.** §§2, 4 largely
evaporate. ⚠ TCC is not available on GeForce parts. (MCDM is the middle option — render-only, no
display, and it **requires an MMU**.)

**[PROPOSE]** If a Windows guest ever becomes a target, **TCC is dramatically the smallest one**,
and the order is: TCC → MCDM → WDDM.

---

## 6. ✔ What is the SAME — this bounds the work, and it is a lot

1. **The RM API ABI is identical** — `NVOS21/54/32/33/38`, handle semantics, class ids, control
   numbers. No Windows variants in `common/sdk/nvidia/inc/`.
2. **`NV01_EVENT_WIN32_EVENT` *is* `NV01_EVENT_OS_EVENT`** — literally a `#define`, value `0x79`.
3. ★ **The GSP boot sequence is OS-agnostic** — **zero** Windows conditionals in the bootstrap
   path; FWSEC/WPR2/booter/LibOS-args/msgq/`GSP_INIT_DONE` all shared. The only platform
   conditional makes Windows hold the API lock across *all* of GSP init ⇒ **strictly more serial,
   the easier case for us.**
4. **`GspSystemInfo` layout is identical**; 610 vs 580 differ by four fields — a version
   difference, not an OS one.
5. **`SET_REGISTRY` format is the same**; `nvrm_registry.h:24` says outright *"shared between
   Windows and Unix."*
6. ★★★ **The doorbell / work-submit path has NO Windows conditional in RM** — usermode region,
   `workSubmitToken`, `kfifoUpdateUsermodeDoorbell_*`, USERD, GPFIFO: **zero** Windows branches
   across `gpu/fifo/`. ⇒ **Part 1 §2 is OS-independent**, modulo §7's two unknowns.
7. **GMMU formats, PTE/PDE encodings, apertures, big-page selection** — chip-keyed, not OS-keyed.
8. **Hypervisor detection is shared and CPUID-based**, and ⊘ **there is no error-43-style refusal
   anywhere in the open RM core.**
9. **Robust channels / RC are enabled on Windows, GSP and Unix alike.**
10. **PMA is supported on all platforms** — WDDM disables *client page tables under PMA*, not PMA.

---

## 7. ★★★ A free gift — detecting a Windows guest from the FIRST RPC

`GSP_SET_SYSTEM_INFO` carries `NvBool bGspNocatEnabled`, and the **only** assignment in the entire
tree is:

```c
if (RMCFG_FEATURE_PLATFORM_WINDOWS) { rpcInfo->bGspNocatEnabled = NV_TRUE; }
```

`kernel_gsp.c:4624` (610) / `rpc.c:10650` (580) — present and identical in both generations.

⇒ **`bGspNocatEnabled == NV_TRUE` in the first RPC we ever receive ⟹ the guest RM was built for
Windows.** ⚠ It is an *enable* flag, not an OS field — **assert on it, do not build on it**. There
is no explicit OS identifier in `GspSystemInfo`.

★ **[PROPOSE] Implement this now, as a refusal, before any Windows work happens.** It costs one
comparison and converts *"a Windows guest silently does something strange"* into a named refusal at
the first message — which is exactly what `support_matrix_asymmetry` demands of an out-of-band
configuration.

---

## 8. ⚠ A correction to this project's own framing

`CLAUDE.md` describes nvkvm as *"WSL2-style NVIDIA GPU ioctl/RPC forwarding."* ⊘ Publicly, WSL2's
mechanism is **D3DKMT-shaped thunks over VM Bus to a host `dxgkrnl`** — the Windows Graphics Kernel
lead, XDC 2020: *"Level of abstraction is the **WDDM interface**… **Not a straight pass-through**…
**No data copy — only control information exchanged over VM bus**."*
⇒ **Mode 1 resembles WSL2's *shape*, not its *protocol*.** The phrase should be corrected where it
appears, because it invites the assumption that a documented design exists for what we do.

★ Related, and worth knowing: Microsoft's own answer to our problem statement is **not to run the
vendor KMD in the guest at all** — GPU-PV puts *"no KMD in the guest, only UMD… no video memory
manager (VidMm) or scheduler (VidSch) in the guest."* ⇒ **The industry cut line is at D3DKMT, not
at the hardware.** We are deliberately below it, and that is the thing that makes this project
unusual — and the reason no prior art exists for emulating an NVIDIA GPU to a Windows guest.

---

## 9. What could NOT be determined, and the first measurements to take

| question | status |
|---|---|
| The Windows OS layer itself (`osGetPageSize`, `osQueueDpc`, `osIsRaisedIRQL`, …) | ⊘ `arch/nvalloc/win/` is not published. **The 4 KB page-size finding rests on an inferred `osGetPageSize()==4096`** |
| The Windows `rmconfig` profile — which chips, classes, engines and HAL bindings | ⊘ openrm hard-sets `RMCFG_FEATURE_PLATFORM_WINDOWS 0`; every NVOC table here is the UNIX profile |
| Is the **GSP firmware image byte-identical** between Windows and Linux packages? | ⊘ Nothing speaks to it. If it differs, our WPR2/radix3 sizing constants do too |
| Is the **GSP-RM RPC protocol** the same on Windows? | ◐ **inference only** (one RM codebase). ⊘ **Do not assume our msgq work transfers without measuring** |
| What do dxgkrnl/VidMm actually issue, and in what order — CE, BAR1 CPU writes, or `UPDATE_PDE_2`? | ⊘ **The most important open question on this axis** (§2) |
| Is display mandatory for `nvlddmkm` to load? | ⊘ Cosmetic on Linux, plausibly fatal on WDDM. TCC/MCDM sidestep it |
| Does NVIDIA enable WDDM 3.2 **user-mode doorbells**? | ⊘ If so, the guest rings a doorbell from user mode with **no kernel transition** — a trap-per-doorbell design becomes a per-write exit on a user-mode hot path |
| Was the Code-43 CPUID check **removed** at R465, or gated? | ⊘ NVIDIA said a narrow configuration is *supported*; it never said the check was removed |

★ **[PROPOSE] The first two measurements, in order:**
1. **`D3DKMT_QUERY_GPUMMU_CAPS` on real hardware** — `VirtualAddressBitCount`, `PageTableLevelCount`,
   4 K vs 64 K segments, GpuMmu vs IoMmu. Cheap, and it decides §2 and the page-size finding.
2. **Check our emulated `DEV_xxxx` is already in NVIDIA's signed INF.** If it is not, nothing else
   on this axis is reachable with a stock guest.
