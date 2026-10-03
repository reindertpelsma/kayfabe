# V3 — the guest's IOMMU (and the host's)

**STATUS: LIVE, 2026-10-04.** **Guest IOVAs are not supported today, and until `v3-viommu` they
were not refused either.** The seam and the detection are built on branch `v3-viommu` (not merged;
§7): kf3 classifies its DMA regime at machine-done (§4.2) and refuses every guest-RAM lookup of a
device address unless the regime is Direct or Identity (§4.3); the typed device-address seam (§3) is
built except for display and the PRAMIN window (§7.2). The translator itself (§5) is not built.
Owner, 2026-10-03: *"We must check iova addresses are supported in kayfabe for guests requiring iommu
protection. Not that this becomes a hard retrofit later."* The design below was revised after an
adversarial review on 2026-10-04. OD-1 … OD-6 (§8) wait on the owner.

⊘ **What this supersedes, each marked in its own text:** the 2026-09-25 design kept at the end of
this file (§P1–§P4; its *"Until it is built"* paragraph and its §3 items 2–4); the realize check
proposed in audit S1-32 (`docs/audits/2026-10-03-v3-stage1.md`); the first bullet of `THE_DESIGN.md`
§9.4 and the *"A guest IOMMU"* row of `THE_OPEN_QUESTIONS.md`, both of which describe a refusal at
realize that was never built.

**Basis.** The kayfabe citations in §1–§6 are to `origin/master` **789dee9f**; between `e4fb0190`
and 789dee9f only `docs/OWNER_RULINGS.md`, `docs/OWNER_QUESTIONS_2026-10-03.md` and the stage-1 audit
changed, so every code line cited holds at both. §7 names the `v3-viommu` files that changed them.
Other sources: QEMU 10.2.4 (`/workspace/bench/qemu-10.2.4`), the open GPU kernel modules 580.159.04
(`ogkm-580`), Linux 7.1-rc6 (`linux`). A citation the design's author did not re-read carries
**(lens)**. Closed firmware and Windows behaviour is UNVERIFIED unless sourced.

**Governing rulings** (`docs/OWNER_RULINGS.md`).
- §Q, last bullet (`:487-491`): *"A guest DMA address (an IOVA under a guest vIOMMU) is a fourth kind.
  It must be translated to a GPA at one validated boundary before any of the rules above apply."*
- §Q, owner, verbatim: *"Gpga offsets can't, outside guest vram is invalid. Gpa offset can't either,
  only reference guest ram or its bar or an error. Same for bar offsets. Passthrough uses a sandboxed
  channel, that strictly only maps based on pte/pdb and nothing else. Translated uses a channel only
  gpga and/or gpa is mapped."*
- On host-process addresses, verbatim: *"only for the cuda channel (vmm ptx refresher), and this
  offset is never supplied or given by guest."*
- §A.4: no blocking on a vCPU; the only exception is a PRAMIN window move. §A.9: in-guest isolation
  matters. §R: the `_unsafe` perimeter.
- 2026-09-25 (§P0 below): a vIOMMU is a **compatibility** requirement — *"to support any stock OS"*.

**What the review changed, in short.** (1) amd-iommu with its default `dma-remap=off` was fail-open
under the draft; it is refused by name as UNTRACKED. (2) The default regime and the classifier are
fail-closed: UNSET and BLOCKED both refuse. (3) The draft's UNMAP fast path had a TOCTOU hole; it is
closed with a shadow generation, and the notifier always enqueues (§5.3). (4) The seam's
compile-time guarantee is stated honestly: it arrives only at Tier 3. (5) Device addresses are typed
at the decode sites, not at the consumers. (6) Three unlisted DMA-address sites were added (§2 #13,
#13b). (7) The collision map was corrected. (8) Three wrong claims were fixed: virtio-iommu is not
strict by default; a vIOMMU mode switch does pass through an intermediate state (now read as BLOCKED,
never as IDENTITY); and the notifier does not run on the main thread only.
(9) Two detection lags are recorded: intel `caching-mode=off`, and AMD clearing IOMMUEN at runtime.

---

## 1. Answer: guest IOVAs are not supported, and were not refused either

- **Nothing in the code path knew a vIOMMU exists.** kf3 learns guest RAM from one MemoryListener on
  `address_space_memory` (`qemu/hw/misc/kf3/kf3.c:909-916`) and never asked QEMU for the device's DMA
  address space. Every `iommu`/`iova` match under `crates/`, `qemu/` and `cuda/` is prose about the
  *host* (`kf-abi/src/bringup.rs:792,799`) or an NVDisplay constant name.
- **Every guest-programmed sysmem address is treated as a GPA.** All of them reach
  `RamMap::block_for` / `RamMap::file_range` (`crates/kf-qemu/src/mem.rs:103-115, 139-143`), from
  exactly **13 call sites** (grep at 789dee9f): `chan.rs:518, 801, 2751, 2773, 3187`;
  `device.rs:747, 2605, 2618`; `display.rs:1064, 1094, 1122, 1170`; `mem.rs:1611`. The guest driver
  programs these addresses with `memdescGetPhysAddr(…, AT_GPU)`, which under an IOMMU mapping returns
  the `iovaArray` (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_desc.c:3305-3324`).
- **The refusal the docs describe does not exist.** `THE_DESIGN.md` §9.4 (`:1160-1166`) says *"A
  guest IOMMU is detected and refused at device realize"*, and `THE_OPEN_QUESTIONS.md:41` lists the
  same refusal. Neither was built: audit S1-32 is open (`docs/audits/2026-10-03-v3-stage1.md:658-668`)
  and §P3 below says *"Not built either"*. The pre-v3 tree's CI "GPA-accessor gate", which funnelled
  guest-physical access through one site, was removed at the 2026-09-26 archive move
  (`scripts/ci_gates.sh:93-96`), and v3 had nothing in its place.

### 1.1 What happened before `v3-viommu` when the guest translates kf3's DMA

Predicted from source, not measured.

**Where the IOVAs land.** Linux allocates PCI IOVAs below 4 GiB first, top-down
(`linux drivers/iommu/dma-iommu.c:791-797`), reserving the host bridge's MMIO windows out of the IOVA
space (`:507-523`). q35 puts its PCI hole at end-of-low-RAM (QEMU `hw/pci-host/q35.c:508-514`); low
RAM ends at `0x80000000` or `0xb0000000` (`hw/i386/pc_q35.c:161-165`) (lens). So early IOVAs fall
**inside valid guest RAM**, and a misread IOVA usually resolves, silently, to an unrelated guest page.
Only IOVAs above top-of-RAM, which appear once 32-bit IOVA space is exhausted, were refused, as
`NotGuestRam`.

**Per site** (detail in §2):
- **Fake GSP boot** is the first consumer. It reads the LibOS args and RMARGS through the guest-RAM
  port and gets unrelated pages. The likely outcome is a named `GspFault` (`RmargsRegionAbsent`,
  `kf-gsp/src/boot.rs:1265, 1419`, or `RegionMalformed`) and a failed driver load (inferred). If the
  garbage happens to parse, the VMM CPU writes status-queue headers and replies into unrelated pages.
- **Walked sysmem PTE leaves** become host rows over unrelated guest pages, which the host GPU then
  reads and writes — including rows built from unprivileged guest userspace's page tables. That is an
  **in-guest isolation break (§A.9)**, not only corruption.
- **Passthrough USERD and the error notifier** (`chan.rs:2741-2790`): host RM and the host GSP write
  `GP_GET` and RC records into unrelated pages.
- **Translated CE physical operands**: copies hit unrelated pages, or are refused as `Untranslatable`
  when the range is not contiguous (`kf-chan/src/translated.rs:392, 726-736`).
- **Display**: VMM-CPU notifier and semaphore writes, and instance and pushbuffer reads, hit wrong
  pages (`display.rs:1060-1174`).

**Nothing reaches outside the VM.** Every resolution ends in `RamMap`, which holds only real guest RAM
(`kf3_is_guest_ram` excludes ROM and ram_device regions, `kf3.c:536-540`; lookups resolve into the
guest's own memfd, `mem.rs:139-143`), and host channels keep `DENY_PHYSICAL_MODE_CE` (§P2). The blast
radius is the guest's own RAM — but inside the guest it is silent corruption plus an isolation break,
more serious than the "minor" S1-32 gave it.

---

## 2. Site table

**S** = cost with the seam (§3) placed now; **L** = cost if retrofitted later without it.

| # | Consumer (kayfabe) | Guest source (AT_GPU) | Thread | Under a translating vIOMMU before `v3-viommu` | With translation (later) | S / L |
|---|---|---|---|---|---|---|
| 1 | **Walked sysmem leaves → host VAS rows**, both planes. The walker emits them raw and coalesces by address contiguity (`cuda/walk/kf_walk.cu:488-504`) — under a vIOMMU, IOVA contiguity. Bound at `kf-mem/src/ledger.rs:78-110`, one call per run (`kf-mem/src/apply.rs:404-421`), closure `kf-qemu/src/device.rs:744-748` → `file_range` | RM sysmem PTEs (`ogkm-580: …/mem_desc.c:3305-3324`); UVM CPU chunks (`uvm_gpu.c:3722-3751`) | VA manager | Host GPU reads and writes an unrelated guest page; above RAM, `NotGuestRam` | One IOVA-contiguous run becomes k GPA pieces, each ≥ the host grain, placed as **one** placement; unmap by extent; tombstone and re-place on a vIOMMU remap. Walker unchanged | moderate / moderate+ |
| 2 | **BAR1/BAR2 window sysmem targets**: same closure, through `kf-mem/src/cpuwin.rs:481-500` → `place_ram` | BAR PTEs (AT_GPU) | VA manager | Wrong guest page in the CPU window | Same resolver; on UNMAP, re-point to scratch. ⊘ §P3 item 4 is wrong here: the window *aperture* is CPU MMIO, its sysmem *target* is GPU DMA | trivial / moderate |
| 3 | **PRAMIN at a sysmem target**, decided on the vCPU: `mem.rs:1699-1712` (plan), `:1608-1612` (closure), `cpuwin.rs:774-783` | `kbusVerifyBar2` sets `_TARGET` from the memdesc aperture (`kern_bus_gm107.c:4071-4086`) (lens), pre-Hopper only (`kern_bus_gh100.c:207`) (lens). Whether stock configurations ever point it at sysmem is UNVERIFIED | **vCPU** | Wrong guest page in the window | Non-blocking shadow lookup on the vCPU with the §5 generation check; revoke the view on UNMAP | trivial / moderate |
| 4 | **Fake-GSP guest-RAM port** (`kf-gsp/src/ram.rs:51-66`): LibOS args (on FSP parts through the `GSP_FMC_BOOT_PARAMS` indirection, `boot.rs:1254-1265`), RMARGS, the msgq region and its self-describing page table (`ram.rs:12-48`), FWSEC DMEM, FSP COT. Only implementation `device.rs:2600-2629` → `block_for` | `kernel_gsp_tu102.c:363-375`, `message_queue_cpu.c:283-288, 306-307`, `kernel_gsp_falcon_ga102.c:215-248` (lens) | register drainer | Most likely a named GSP boot fault (inferred); otherwise VMM-CPU writes to wrong pages | Translate on every access inside the port, split at IOVA page boundaries. The port caches IOVAs (still valid guest values), never host pointers, so nothing to revoke | trivial / trivial |
| 5 | **Passthrough twin sysmem USERD**, a host-RM binding for the channel's life: `chan.rs:2741-2762`; decoded at `kf-abi/src/notifier.rs:542` | `kernel_channel.c:2748-2758` (lens) | channel birth (drainer) | Host GPU writes `GP_GET` into a wrong page | Translate at birth (512 B, one page). On UNMAP of a live binding, RC the twin (hardware would take an IOMMU fault). Needs the reverse index | trivial / moderate |
| 6 | **Passthrough error notifier**, where the host GSP writes the RC record: `chan.rs:2771-2790`; decoded at `notifier.rs:270` | `kernel_channel.c:548-584` (lens) | channel birth | RC record lands in a wrong page | As #5 | trivial / moderate |
| 7 | **Translated USERD view**: a persistent raw view; the VMM CPU reads `GP_PUT` and writes `GP_GET` (`chan.rs:3184-3193`) | as #5 | worker | Wrong page; the cached host pointer goes stale on a remap | Translate at view creation; kill the view on UNMAP (channel dead, `kf-chan/src/host.rs:492-503`) | trivial / moderate |
| 8 | **Translated physical-mode CE operands**: `SlotWindow::translate` (`chan.rs:509-522`) → the whole-memfd `OS_DESCRIPTOR` window (`THE_TRANSLATED_PLANE.md:198-203`) | CeUtils (`ce_utils.c:761-779`); UVM physical operands | worker | Copy to or from wrong pages, or `Untranslatable` | **Single-piece** resolve, then base+off — a producer-observed property, not a hardware contract; a multi-piece operand is refused by name (kernel channel → poison). CeUtils splits per `pageArrayGranularity` unless `MEMDESC_FLAGS_PHYSICALLY_CONTIGUOUS` (`ce_utils.c:761-779`; `mem_desc.c:4186-4189`); a contiguous RM allocation is one `dma_map_page_attrs` (`ogkm-580: kernel-open/nvidia/nv-dma.c:58-74`); UVM consolidates only when one CPU chunk covers the whole block (`uvm_va_block.c:3590-3600, 4250-4253`), each chunk one `dma_map` (`uvm_pmm_sysmem.c:252`); pageable migration copies `PAGE_SIZE` (`uvm_migrate_pageable.c:598-661`) (lens). So no launch splitting | trivial / moderate |
| 9 | `KF3_COMPLETION_PROBE` diagnostic, default off: `chan.rs:801` | CE operand | worker | Misread | Through the port | trivial / trivial |
| 10 | **Display**: instance memory (`display.rs:1060-1069`), context-DMA read/write (`:1089-1126`), pushbuffer (`:1164-1174`) | `disp_inst_mem.c:386-389` (AT_GPU), `disp_channel.c:835`, `disp_inst_mem_0300.c:168-169` (lens) | display worker | Wrong pages, including VMM-CPU notifier writes | Same port as #4, split per page. `v3-broker` rewrites `display.rs`, so this waits until it merges | trivial / trivial |
| 11 | Fault, shadow-fault and access-counter buffers (`kf-abi/src/faultbuffer.rs:145-151, 320-325, 489-494`) | `faultBufferPteArray`, `shadowFaultBufferPteArray`, `bufferPteArray` (AT_GPU) | — | Recorded only; no writer exists (`DELIVERY_UNBUILT`) | The future writer uses the port. The `_gpa` names mislead | trivial / grows |
| 12 | Sysmem page-directory roots and PDEs: refused (`kf-mem/src/vasmgr.rs:64-70, 211-213`; `kf_walk.cu:545, 623, 630`; `UPDATE_BAR_PDE` sysmem refused, `kf-rm/src/barpde.rs:226`) | UVM fallback | — | No exposure: already refused | Keep "sysmem page tables" and "vIOMMU" mutually exclusive: supporting both is the only case that forces GPU-side translation | n/a / hard |
| 13 | **Inert addresses**, never dereferenced: the GSP trace-crash buffer (`kf-gsp/src/rpc.rs:97`, `kf-rm/src/inert.rs:29`); `PROMOTE_CTX` physical addresses; channel instance, RAMFC and method-buffer memory; **`eccErrorNotifierMem`** (decoded in `kf-abi`'s generated matrix, unused); sysinfo / WPR metadata; the sysmem flush buffer (`NV_PFB_NISO_FLUSH_SYSMEM_ADDR`); the notifyOp surface | AT_GPU | — | Never dereferenced | A future writer goes through the port and takes `DevAddr` | trivial |
| 13b | **Refused today, unlisted in the draft**: the `ALLOC_MEMORY` RPC `pte_desc` page arrays — `memRegisterWithGsp` (`mem.c:483-488, 515-560`) → `_issuePteDescRpc` (`rpc.c:2252-2302`). Producers: display (`disp_common_kern_ctrl_minimal.c:316-327`), profiler PMA (`kern_profiler_v2_ctrl.c:299-302`), `deferred_api.c:371`, **FBSR** via `memdescSendMemDescToGSP` (AT_GPU page list `mem_desc.c:4842-4843`) and `FBSR_INIT.sysmemAddrOfSuspendResumeData` (`fbsr_gm107.c:120-126`) | AT_GPU page arrays | drainer | Refused by name as `RpcFunction::Other` (`kf-gsp/src/rpc.rs:243`) | Whoever builds it decodes the array as `Vec<DevAddr>` and resolves per page | trivial / grows |
| 14 | **Non-DMA sysmem-aperture leaves**: Hopper+ usermode MMIO views and SKED CDP, diverted before `ram_offset` (`apply.rs:361-392`) | — | — | Correct | **Must never be translated.** The translator stays behind this classification, which holds while it lives in the resolver | — |
| 15 | MSI-X under interrupt remapping: routes are created with the `PCIDevice` (`kf3.c:633, 642`); QEMU's `kvm_arch_fixup_msi_route` remaps them by requester ID (`target/i386/kvm/kvm.c:6308-6343`) and refreshes them on IRTE invalidation (`:6378-6443`) | — | — | Correct by construction | Nothing; keep passing `pci` | — |
| 16 | ATS, PRI, PASID: kf3 is `INTERFACE_CONVENTIONAL_PCI_DEVICE` (`kf3.c:990`), so it has no such capability | `nv-pci.c:1515-1545` would try SVA if it saw ATS (lens) | — | Unreachable | Never advertise ATS | — |
| 17 | Detection: absent before `v3-viommu` (`kf3.c:909-916`; S1-32) | — | — | Silent | §4 (built, §7.1) | trivial |
| 18 | **Invalidation coherence.** Caches that hold a resolved address: host rows and PlacedRows, which carry no source address (`mem.rs:477-501, 734-741`) (lens); the walker's committed diff, keyed by device address (`kf_walk.cu:807-822`); CPU windows and PRAMIN views; the #5–#7 bindings | — | — | n/a | §5 | **hard** / hard |

NVIDIA sources in the table without a tree prefix are `ogkm-580` paths by file name.

**Out of scope, recorded.** On SEV-SNP and TDX guests the device address differs from the GPA even
with no vIOMMU (SEV uses swiotlb bounce buffers; TDX sets a shared bit) — UNVERIFIED. kf3 cannot
serve encrypted guest RAM anyway. The `DmaSpace` seam (§3) is where such a hop would live.

---

## 3. The seam: one device-address boundary, the identity when there is no vIOMMU

### 3.1 Address kinds (§Q)

| Kind | Type | Constructed by | May reach |
|---|---|---|---|
| GPGA (guest VRAM offset) | `u64` / `StoreOffset` (`kf-mem/src/addr.rs:20-23`) | walker vidmem leaves | the store, bounded by `kf_walk.cu:496` and `ledger.rs:86-97` |
| **DevAddr** (guest DMA address) | `kf_arch::dma::DevAddr`, private field; `DevAddr::from_guest(raw)`; `Debug`, `LowerHex`, `Ord`, `Hash`, `checked_add`; **no** `From`/`Into` to or from any GPA type | **the decode sites** (§7.2) | nothing directly — only through `DmaSpace` |
| GPA | private to `kf-qemu`'s boundary (`DmaSpace::translate`'s result). ⊘ **Not** the existing public newtypes `kf_arch::ids::Gpa` (`ids.rs:12-16, 98-101`) or `kf_mem::addr::Gpa` (`addr.rs:13`): both have `pub` fields and are not the boundary | `DmaSpace::translate`, and QEMU's RAM feed | guest RAM via `RamMap`, the device's own BAR (refused today, which §Q allows), or an error |
| BAR offset | existing | trap decode | validated per BAR |
| host-process VA | never in safe code (§R) | kayfabe's own CUDA contexts (the walker, the display compose); never a guest value (§Q) | — |

**What the types do and do not guarantee.** `DevAddr` needs a getter so the translator can read the
integer (`translator_raw`, documented "translator only"), and Rust cannot scope a getter to one other
crate, so **`DevAddr` is a labelling aid, not the boundary.** The boundary is `RamMap`'s lookups being
private to the DMA module: then no safe code outside it can turn any integer into guest memory. That
needs `display.rs` migrated, which waits for `v3-broker` (Tier 3, §6). **Until Tier 3 the guarantee
is the runtime gate (§4.3) plus review.** Tier 3 adds a compile-fail test proving `RamMap`'s lookups
are unreachable outside the DMA module.

### 3.2 The single choke point: `DmaSpace` in `kf-qemu`, over `RamMap`

- **Hop 1, `translate(at, len)`.** Direct or Identity regime: one run, the raw value (today's
  behaviour). Any other regime: **interim** `Err(DmaRefusal::Regime { regime })`. **Later**
  (Translating only): a lookup in the shadow map (§5) per 4 KiB, coalescing adjacent GPAs; permission
  = PTE permission ∧ IOMMU permission (`MapPerm` already carries the PTE's, `apply.rs:36-41`); the
  lookup returns the shadow **generation** it read.
- **Hop 2.** `RamMap`'s guest-RAM-only lookup per run. The guest-RAM-only check stays (§Q: *"only
  reference guest ram or its bar or an error"*): an IOVA the guest maps to MMIO, another device's BAR
  or kf3's own BAR is refused here.
- **Verbs** (built, §7.2): `piece(at, len, want)` — exactly one piece of the guest-RAM object, no
  allocation; `DmaResolve::resolve` — the pieces in device order (the identity: one); `view(at, len)`
  — one block for a persistent view (the Translated USERD; later also a revocation token);
  `read(at, buf)` / `write(at, bytes)` — CPU copies for the GSP port (later split per piece, also for
  display and the fault-buffer writer).
- **`DmaRefusal`**: `Regime { at, len, regime }` (interim; regime ∈ Unset / Translating / Untracked /
  Blocked), `NotGuestRam { at, len }`, `Fragmented { at, len, pieces }`; later `Unmapped { at, len }`
  (a vIOMMU fault), and reserved `OwnBar { at, len }` — P2P through `dma_map_resource`
  (`ogkm-580: kernel-open/nvidia/nv-dma.c:688-713`) (lens); RM itself refuses a GPU's own BAR
  (`os.c:3546-3550`) (lens).

### 3.3 Signatures after the seam

- `ledger::desired_from_leaves(leaves, store_bytes, dma: &dyn DmaResolve)` replaces
  `ram_offset: &dyn Fn(u64,u64) -> Option<u64>` (`ledger.rs:81`). A sysmem leaf calls
  `dma.resolve(DevAddr::from_guest(at), len, perm)` and yields one `Desired` per piece — the walker
  report is the decode site for leaves. A refusal is `LeafRefusal::DeviceAddress { va, at: DevAddr,
  len, why }`, replacing `NotGuestRam { gpa }` (`ledger.rs:52-60`).
- `ApplyCfg.ram_offset` (`apply.rs:54`) becomes `dma: &'a dyn DmaResolve`. The per-run call stays
  (`apply.rs:404-421`): k > 1 is refused by name, *"fragmented device range … multi-piece placement
  not built"*. Under the identity k is always 1, so behaviour is identical.
- `VaManager::new(…, dma: Box<dyn DmaResolve + Send>)` (`vasmgr.rs:807, 832, 1282`).
- `PraminPool::new(…, dma: Box<dyn DmaResolve + Send + Sync>)` (`cpuwin.rs:684, 714, 776`) —
  **deferred**, §7.2.
- `SlotWindow { mirror, dma }` (`chan.rs:509-522`): `Some` only for exactly one piece; the
  `kf_chan::Window` trait is unchanged (`translated.rs:100-105`). The trait's `None` loses the name;
  a regime refusal is counted by the device's regime cell.
- `chan.rs:2751, 2773`: one piece of 0x200 or 16 bytes, from the `DevAddr` the decode produced.
  `chan.rs:3187` takes a `view`; `chan.rs:801` a `piece`.
- The `GuestRam` implementation (`device.rs:2604-2628`) uses `read` / `write`. The `GuestRam` trait
  (`kf-gsp/src/ram.rs:51-66`) keeps `u64`; only its docs change, to "device address" — `kf-gsp`'s
  `boot.rs` is edited by `v3-broker` and `v3-dispsw-exp`, so its signatures are not churned now.
- `display.rs` ×4: the same as `GuestRam`, **after `v3-broker` merges**. Then `RamMap` moves into the
  DMA module with private lookups (Tier 3).
- Test and harness closures (`apply.rs:903`; `cpuwin.rs:1129`; `vasmgr.rs:1837`; `kf-harness`'s
  gates 2–8 and `publish.rs:191-217`) are wrapped in `kf_mem::dma::IdentityFn(|gpa, _| Some(gpa))` —
  a **named** wrapper, not a blanket `impl` for closures, so a raw-`u64` closure cannot silently
  become a production resolver. No harness constructs the kf-qemu `Device`, so none depends on
  `RamMap` (grep at 789dee9f).

### 3.4 GPU-side choice: (b)

The premise behind `THE_DESIGN.md:1163-1164` (*"a GPU kernel that cannot call the VMM"*) is wrong:
**the walker never dereferences or bounds a sysmem address.** Every GPU read is bounded to the store
window (`kf_walk.cu:163-165, 416-450`); sysmem PDEs are refused (`:545, 623, 630`); sysmem leaves are
emitted raw on purpose (`:488-497`) and bounded **once, on the host** (`ledger.rs:69-110`).

- **(a) An IOVA→GPA table on the GPU, consulted by the walker — rejected.** It adds a second
  guest-steered dereference surface inside the walker's audited bound; the table is sparse up to the
  aperture width (`aw-bits`); walks would need a generation handshake against vIOMMU events that land
  on any vCPU; the resulting GPA still needs the host check, so there would be two validation sites;
  and it conflicts with §Q.
- **(b) The host resolves each device address at the one existing sysmem bound — chosen.** Walker,
  PTX, report ABI and gate 9 are unchanged; the bound stays where it is (`ledger.rs:98`). The walker's
  diff, keyed by IOVA, is correct for PTE content; a remap under an unchanged PTE is handled host-only
  (tombstone and re-place, §5).
- **(c) An IOVA-indexed host-VA window filled from MAP events — reserved.** §Q forbids windows in
  passthrough twins, so it could serve only Translated spaces, where it buys nothing because CE
  physical operands are single-piece (§2 #8); and it costs one host RM map per MAP event: ~19-22 µs
  per map and 35-123 µs per unmap (`V3_BATCHED_MAP.md:18-21`). Kept only for a future walker over
  sysmem-resident page tables (§2 #12).
- **(d) Deployment-level, no code — the documented workaround, not support.** Each keeps device
  addresses equal to GPAs, and the MSI path stays remapped (§2 #15):
  - (d1) a `bypass_iommu` host bridge: `pci_device_iommu_address_space` then returns
    `&address_space_memory` (QEMU `hw/pci/pci.c:2943, 2958-2970`);
  - (d2) `dma-translation=off` (`hw/i386/x86-iommu.c:133`), advertised to the guest: intel publishes
    no SAGAW (`intel_iommu.c:4827-4833`), amd sets HATDis (`acpi-build.c:1866-1867`,
    `amd_iommu.c:120-122`); interrupt remapping stays;
  - (d3) guest `iommu=pt`, **on intel-iommu, virtio-iommu, or amd-iommu with `dma-remap=on`**
    (§4.2 on AMD).

### 3.5 Conformance with the owner's address model

- **Passthrough.** Under (b) a twin row is *PTE leaf ∘ the guest's own IOMMU table*, both authored by
  the guest kernel, which is what hardware does. The vIOMMU can only **remove** a row (an unmapped
  IOVA: a refusal or tombstone) or **redirect it within guest RAM**; it can never add a row the PTEs
  did not name, and never a window or kayfabe memory. The owner is asked to confirm this counts as
  *"based on pte/pdb and nothing else"* (OD-6).
- **Translated.** Spaces still map only GPGA rows, GPA rows and the GPA-indexed guest-RAM window
  (§Q answer 6c).
- **Residual, accepted and recorded.** That window lets a guest-**kernel** channel's *virtual* operand
  that names it bypass the guest's own vIOMMU. Translated channels are guest-kernel channels
  (`chan.rs:524-525`), §Q allows the window, and the guest kernel is the vIOMMU's master. It stays
  tied to S1-21 P1/P2 (Translated work in its own space).

---

## 4. Detection and interim behaviour

### 4.1 Why not refuse at realize (S1-32's proposed check)

1. **It depends on command-line order.** Each vIOMMU installs its PCI hooks in its own realize
   (`pci_setup_iommu` at `intel_iommu.c:5458`, `amd_iommu.c:2650`, `virtio-iommu.c:1473`,
   `smmu-common.c:966-968`), and `-device`s realize in command-line order (`system/vl.c:2751-2752`),
   so a kf3 listed before the vIOMMU sees `&address_space_memory`. QEMU itself defers bus-master setup
   to machine-done for exactly this reason (`hw/pci/pci.c:138-160, 1384-1386`).
2. **It refuses by presence, not by translation** — guests that work today: intel or virtio-iommu
   with guest `iommu=pt`, and `dma-translation=off`, which are the interrupt-remapping-only cloud
   cases.
3. **Correction to the draft:** amd-iommu with its default `dma-remap=off` is **not** a guest that
   works today. QEMU never applies the guest's translation for any device there while the guest still
   translates (§4.2). A realize check would refuse it; the draft's runtime design read it as IDENTITY,
   which is **fail-open**. Fixed below.

### 4.2 Chosen mechanism: runtime, per device, order-independent, fail-closed

**At machine-done** (`qemu_add_machine_init_done_notifier`, which fires at once for a hot-plugged
device, `hw/core/machine.c:1752-1758`):
1. `as = pci_device_iommu_address_space(pci)` (`hw/pci/pci.c:2958-2970`).
2. `as == &address_space_memory`: **DIRECT**, for the device's lifetime.
3. Otherwise, an `amd-iommu` with `dma-remap=false` and `dma-translation=true` makes it
   **UNTRACKED**, for the device's lifetime: QEMU never enables that model's IOMMU region
   (`amd_iommu.c:1109-1122`; the DTE mode is consulted only `if (s->dma_remap)`, `:1259-1261`; address
   spaces are created in pass-through, `:2340, 2385`; `dma-remap` defaults to false, `:2657`); nothing
   tells the guest (HATDis or HATS-reserved appear only for `dma-translation=off`); and Linux gives kf3
   a DMA domain by default (`linux drivers/iommu/amd/iommu.c:3128-3153`). So whether kf3's addresses
   are IOVAs cannot be known from QEMU's state. Looked up by QOM
   (`object_resolve_path_type("", "amd-iommu", &ambiguous)`); **more than one, or a property that
   cannot be read, is UNTRACKED too** (built that way, §7.1).
4. Otherwise a second MemoryListener on `as` (`region_add`, `region_del`, `commit`); registration
   replays the current sections and then commits (`system/memory.c:3076-3128`).

**The classifier**, at `commit`, from counts kept in `region_add` / `region_del` (IOMMU sections:
`memory_region_is_iommu(sec->mr)`; RAM sections: `memory_region_is_ram(sec->mr)`):

| IOMMU sections | RAM sections | Regime |
|---|---|---|
| > 0 | any | **TRANSLATING** |
| 0 | > 0 | **IDENTITY** |
| 0 | 0 | **BLOCKED** |

IDENTITY needs positive evidence; BLOCKED refuses; Rust starts at **UNSET**, which refuses until the
first publish; an unknown wire value refuses. ⊘ **Correction to the draft: there IS an intermediate
state.** `memory_region_set_enabled` commits each change on its own (`system/memory.c:2731-2740`), and
every vIOMMU switches a device by disabling one region and then enabling the other — intel
(`intel_iommu.c:1760-1777`), amd (`amd_iommu.c:1113-1121`), virtio (`virtio-iommu.c:124-130`) — so a
real transition passes through a commit where neither is enabled. The classifier makes that BLOCKED,
which refuses, rather than flashing IDENTITY. Other devices' changes cause no transient:
`set_enabled` returns early when nothing changes (`memory.c:2733-2735`).

**What each vIOMMU's region state tracks, and its known lags.**
- **intel.** `use_iommu = dmar_enabled && !pt`, switched on GCMD.TE, on context invalidation, and
  lazily on a device's first translate (`intel_iommu.c:1738-1811, 2184-2200`). At boot, guest
  `iommu=pt` is seen correctly: Linux attaches default domains (`linux drivers/iommu/intel/iommu.c:2659`)
  before it enables translation (`:2669-2672`). **Lag, fail-closed:** with `caching-mode=off` Linux
  flushes only the write buffer on a not-present→present context change (`:1128-1140`); kf3 never
  translates, so a *runtime* switch of kf3 to identity is never observed and kf3 stays TRANSLATING —
  over-refusal, whose remedy is `caching-mode=on`, which full support needs anyway (OD-3). ⊘ Do **not**
  probe-translate to wake the fast path: on a non-pass-through device it raises guest-visible DMAR
  faults.
- **amd with `dma-remap=on`.** Switched on INVALIDATE_DEVTAB_ENTRY by DTE mode
  (`amd_iommu.c:1198-1240`), so guest `iommu=pt` (DTE Mode=0) gives IDENTITY. **Lag:** clearing
  IOMMUEN at runtime switches nothing (`:1592-1620`) while translate then returns identity
  (`:1941-1949`); the interim over-refuses (fail-closed); full support must treat that state as
  UNTRACKED (§5).
- **amd with `dma-remap=off`.** UNTRACKED, above.
- **virtio-iommu.** Switches between `iommu_mr` and `bypass_mr` per endpoint and domain
  (`virtio-iommu.c:98-131`). Linux's identity domain uses the bypass flag when offered, giving
  IDENTITY; an identity domain built from 1:1 maps reads TRANSLATING, which over-refuses in the
  interim.
- **SMMUv3.** Always shows its IOMMU region, so always TRANSLATING. ARM only.

**Thread.** `commit` is a *global* listener callback (`system/memory.c:1155-1163`): it runs on
whichever BQL holder commits any memory transaction in the VM — the main loop, a vCPU, an IOThread.
So it is O(1): compare the counts, call Rust only on a change, and the Rust call is **one atomic
store**, with no syscall, log or lock (§A.4).

### 4.3 Interim behaviour: fail closed at use, by name

- **The gate** sits at the top of `RamMap::block_for`, and `file_range` goes through it
  (`mem.rs:140`), so it covers all 13 sites — display included — with no `display.rs` edit. A lookup
  proceeds only in DIRECT or IDENTITY; every other regime (UNSET, TRANSLATING, UNTRACKED, BLOCKED)
  refuses and is counted. `at_file_offset`, the inverse used on rows already placed, is not gated.
- **What the guest sees.** Each existing refusal path is a real refusal, never a forged completion
  (§A.3): the GSP boot read is refused, so the driver load fails and RM times out waiting for
  INIT_DONE; leaves are acked FAILED, one per run (`apply.rs:404-421`), so the host GPU faults or the
  twin is RC'd; Translated channels die (`kf-chan/src/host.rs:492-503`) or are poisoned; passthrough
  channel allocation returns `NV_ERR_NOT_SUPPORTED` (`chan.rs:2753-2759`); the error notifier becomes
  `RC-UNARMED` (`chan.rs:2779-2785`).
- **One substitution, not a refusal.** A BAR1/BAR2/PRAMIN window page whose sysmem target is refused
  shows per-BAR scratch under the existing "never a hole" rule (`cpuwin.rs:776-781`): a guest CPU read
  there returns scratch contents, not an error pattern, and writes are discarded — the same as today's
  unbacked pages. Counted in `dma_refused` and recorded here.
- **Naming.** At machine-done the main thread logs `kf3: device DMA regime <R> (<as name>)`; for
  UNTRACKED it adds the remedy (*set dma-remap=on or dma-translation=off*). The GSP refusal text names
  the regime. The status line carries `dma=<Unset|Direct|Identity|Translating|Untracked|Blocked>
  dma_refused=<n>`.
- **Runtime flips.** IDENTITY→TRANSLATING with live GPA-derived state: new lookups are refused;
  existing rows and bindings persist until the guest tears them down, and the harm stays in the guest
  — the one interim case where host access after the flip is not refused. Linux changes a group's
  domain type only with no driver bound (`linux drivers/iommu/iommu.c:3290-3292`), and hardware needs
  a quiesced device across a TE flip, so the remaining path is kexec. TRANSLATING→IDENTITY: lookups
  resume as GPAs.

**Works meanwhile:** no vIOMMU; intel or virtio-iommu not translating kf3 (guest `iommu=pt` set at
boot, or a bypass identity domain); `dma-translation=off` on intel or amd; amd with `dma-remap=on` and
guest `iommu=pt`; a `bypass_iommu` bus.

**Refused by name:** any guest that translates kf3's DMA; amd `dma-remap=off` (UNTRACKED); the
transient BLOCKED state; the intel `caching-mode=off` runtime-identity lag.

**OD-1 (owner).** Refusal at use replaces §P3's *"Until it is built"* and S1-32's realize check. It
refuses every guest the old text left silently broken, plus amd `dma-remap=off` guests running
`iommu=pt` (OD-5).

---

## 5. The invalidation sync point (full support; nothing in §5 is built)

### 5.1 QEMU facts

- **The notifier is synchronous.** In all four vIOMMUs it runs synchronously and the guest's wait
  completes only after it returns — intel: QI with a WAIT descriptor (`intel_iommu.c:3527-3545,
  :2838`) (lens); amd COMPLETION_WAIT, the virtio-iommu reply and SMMUv3 CMD_SYNC (lens). No API lets a
  notifier defer that completion.
- **UNMAP-only events carry only the IOVA** (`translated_addr = 0`, `intel_iommu.c:2514-2525`), and
  UNMAP-only notifiers also receive over-sized and whole-space UNMAPs (`:1648-1660`).
- **kf3 therefore needs MAP|UNMAP notifiers**, because host rows cannot be refilled after a host GPU
  fault. Available on intel with `caching-mode=on` (refused without it, `intel_iommu.c:3998-4004`;
  `snoop-control` refuses every notifier, `:3993-3997`; with `x-flts` in scalable mode QEMU skips the
  shadow sync, so MAPs never arrive, `:2500-2507`), amd with `dma-remap=on` (`amd_iommu.c:2419-2430`),
  and virtio-iommu.
- **Invalidation policy per guest:** intel with CM, strict (`linux drivers/iommu/intel/iommu.c:2645-2648`),
  unless first-level translation is the default; amd with NpCache, strict
  (`linux drivers/iommu/amd/init.c:2213-2216`). ⊘ **Correction to the draft:** virtio-iommu advertises
  `IOMMU_CAP_DEFERRED_FLUSH` (`linux drivers/iommu/virtio-iommu.c:1084-1085`), so its default is
  DMA-FQ (lazy): an UNMAP reaches QEMU only when the flush queue is flushed — the same class of harm
  as hardware lazy mode.

### 5.2 Forward direction (MAP)

MAP composes by causality with the three existing sync points: the MAP notification updates the
shadow before the guest's `dma_map` returns (intel and amd through the synchronous invalidation,
virtio through `iotlb_sync_map`); that is before the guest writes the GPU PTE and invalidates (sync
point 1: the VA manager walks and reconciles before it clears, `mem.rs:9-10`), before it sends the
RPC naming the address (sync point 2), and before it rings a Translated doorbell (sync point 3).

### 5.3 Reverse direction (UNMAP): the new fourth point

**Requirement:** the guest's IOTLB invalidation must not complete while host state still maps the old
target.

**Shadow.** An interval map IOVA→(GPA, perm) with a monotonically increasing **generation**, behind a
short non-blocking critical section (the precedent: `RamMap`'s `RwLock`, already read on the PRAMIN
vCPU path). Filled by MAP and by `memory_region_iommu_replay` at registration, as vfio does
(`hw/vfio/listener.c:523-563`) (lens).

**Consumers (the TOCTOU fix; the draft's fast path had a hole).** Every consumer (1) resolves through
the shadow and records the generation `g`; (2) registers its binding in the reverse index (IOVA range
→ consumer) **before** the binding becomes reachable; (3) re-reads the generation for its range and,
if anything overlapping changed after `g`, revokes itself. This covers the VA thread mid-reconcile,
the drainer at twin birth, the worker building a USERD view, and a vCPU re-pointing PRAMIN.

**Notifier** (a vCPU or the main thread, BQL held; it never calls QEMU translate and never waits on a
kf3 thread): (1) remove `[iova, iova+mask]` from the shadow and bump the generation; (2) **always**
enqueue `UNMAP{range, gen}` to the VA-manager FIFO with one eventfd write — the vCPU budget of
`MemPlane::invalidate_write` (`mem.rs:13-14`); (3) count whether the reverse index overlapped,
diagnostics only — the draft's *"nothing overlaps, return"* is **not** safe, because a consumer may
sit between its steps 1 and 2; (4) return.

**VA manager.** In FIFO order, after any reconcile that resolved before it, it revokes the overlapping
consumers: a host row → unmap plus a tombstone, re-placed on a later MAP from the host's own record
(va, DevAddr, len, perm, kind, privileged, ap); a CPU window → scratch; a PRAMIN view → revoked; a twin
USERD or notifier → RC the twin; a Translated USERD view → channel dead.

**Residual window.** Revocation completes µs to ms after the guest's invalidation completes; in that
window in-flight host GPU work can reach the old guest page. The harm stays inside the VM, in the same
class as lazy IOTLB mode. **Exact alternatives:** patch the vIOMMU models to hold WAIT /
COMPLETION_WAIT / the virtio reply until kf3 signals (the shape of `MMU_INVALIDATE`: the guest spins,
the vCPU never blocks); or VFIO-style teardown inside the notifier, which needs an **exception to
§A.4**.

**AMD IOMMUEN.** Clearing IOMMUEN at runtime emits no notification and switches no address space
(`amd_iommu.c:1592-1620`) while translate returns identity (`:1941-1949`); a shadow kept through that
would mistranslate. Full support models `enabled`, or refuses it as UNTRACKED.

**Lock rules.** SMMUv3 holds a non-recursive mutex across notifications that its translate path also
takes; virtio-iommu holds a recursive one (`virtio-iommu.c:91-106`). The VA thread already waits,
bounded, for a main-loop bottom half (BAR1 overlays, `kf3.c:370-375`). So the **shadow and
reverse-index locks are never held across any wait, syscall or host RM call**, or a notifier under the
BQL and the VA thread deadlock; and translation comes only from the Rust shadow, never from
`address_space_translate`.

**Owner decisions for §5:** OD-2 and OD-3 (§8).

---

## 6. Effort, and the risk of waiting

**(i) The seam now.**
- **Tier 1:** detection (with AMD UNTRACKED and the BLOCKED/UNSET classifier), the fail-closed gate,
  tests — ~90 lines of C, ~100 of Rust, ~120 of tests; 0.5–1 day. **Built, §7.1.**
- **Tier 2:** the typed seam with identity behaviour across kf-arch, the kf-abi decode, kf-mem,
  kf-harness and 9 kf-qemu sites; 1.5–2.5 days, behaviour identical. **Built except PRAMIN and
  display, §7.2.**
- **Tier 3, after `v3-broker` merges:** display ×4, `RamMap` inside the DMA module with private
  lookups, a compile-fail test; 0.5–1 day. This is the tier that makes the boundary compile-time.

**(ii) Full support later**, assuming the seam:

| Work | Days |
|---|---|
| Shadow with generation, MAP\|UNMAP notifier and replay, configuration refusals, FFI | 3–4 |
| k-piece placement: all-or-nothing `map_batch`, unmap by extent, `BatchBook` | 3–5 |
| Reverse index, revocation per cache, tombstone and re-place, the generation re-check at every consumer, OD-2 | 5–8 |
| Per-page splitting in the GSP and display port | 1 |
| Fake-vIOMMU unit tests plus a box matrix: intel `cm=on`; amd `dma-remap=on`; virtio-iommu (strict and lazy guests); a UVM-heavy workload; `iommu=pt` regression | 4–6 |
| **Total** | **~16–24 engineer-days** |

Windows guests (Kernel DMA Protection, per-driver DMA remapping opt-in) are UNVERIFIED and extra.
Without the seam, add **3–6 days** of re-plumbing and re-auditing, plus a missed-site risk nothing can
catch without vIOMMU boxes.

**Risks of not placing the seam:**
1. Silent in-guest corruption and the §A.9 break stay possible while two docs claim a refusal that
   does not exist — including AMD's default configuration.
2. New consumers keep arriving as bare `u64` named `gpa`, each a site the compiler cannot list: three
   unbuilt fault-buffer writers; the `ALLOC_MEMORY` page arrays (§2 #13b); the notifyOp surface;
   display growth; `v3-broker`'s `post_subdevice_event` through the GSP port (its `kf-gsp/src/boot.rs`
   hunk). v3 already lost the pre-v3 GPA-accessor gate (`scripts/ci_gates.sh:93-96`).
3. The `Option<u64>` one-row contract hardens as more code relies on it (batched map, PlacedRows
   without a source address, the S1-21 P1/P2 rework), so k-piece support gets dearer.
4. Pressure grows to move the sysmem bound GPU-side, which turns IOVA support into option (a).
5. The vfio-user frontend (`V3_VFIO_USER_FRONTEND.md`) receives memory as IOVA-keyed
   `DMA_MAP{iova, size, fd, offset}` (QEMU `hw/vfio-user/container.c:83-130`) (lens) — the same table
   as the shadow; building it without the seam forks the address model.

---

## 7. What `v3-viommu` builds (2026-10-04)

Branch `v3-viommu` from `origin/master` 789dee9f. No box was rented: every result below is GPU-free,
and the merge bar's box tests (§7.5) are recorded, not run.

### 7.1 Commit A — detection and the fail-closed interim (`97cadd42`; ABI number `70227227`)

- **`qemu/hw/misc/kf3/kf3.h`**: `KF3_DMA_DIRECT 0`, `_IDENTITY 1`, `_TRANSLATING 2`, `_UNTRACKED 3`,
  `_BLOCKED 4`; `void kf3_dma_regime(void *h, uint32_t regime)`; **`KF3_ABI 15`**.
- **`qemu/hw/misc/kf3/kf3_dma.h`** (new, pure C): `kf3_dma_classify(untracked, iommu, ram)`,
  `kf3_dma_amd_untracked(found, ambiguous, readable, dma_remap, dma_translation)`, `kf3_dma_name`.
- **`qemu/hw/misc/kf3/kf3.c`**: `kf3_dma_add` / `kf3_dma_del` (section counts), `kf3_dma_commit`
  (classify; call Rust only on a change), `kf3_dma_amd_lookup` (QOM), `kf3_dma_machine_done` (§4.2
  steps 1–4, then `info_report`), `kf3_dma_disarm`, `kf3_instance_finalize`; armed in realize after the
  guest-RAM listener registers; disarmed in exit before `kf3_unrealize`.
- **`crates/kf-arch/src/dma.rs`** (new, pure): `DmaRegime { Unset, Direct, Identity, Translating,
  Untracked, Blocked }`, `from_wire` (unknown → Translating), `admits`; `DmaRegimeCell` (default Unset;
  `set` is one atomic store; `admit` counts a refusal; `admitted(lookup)` runs the lookup only when
  admitted).
- **`crates/kf-qemu`**: `kf3_dma_regime` → `Device::dma_regime` (`ffi_unsafe.rs`, `device.rs`);
  `RamMap.dma` and the gate in `RamMap::block_for` (`mem.rs`); the GSP refusal names the regime; the
  status line carries `dma=` and `dma_refused=`.

**Where it departs from the design text, and why.**
- **ABI 15, not 14.** `v3-cand-1`'s `kf3.h` (2026-10-04) reserves 14 for `v3-broker` merged onto 13
  (its `kf3_realize` takes both `x11_dispsw` and `display_broker`). Two surfaces under one number is
  what the ABI check exists to refuse.
- **The C rules live in `kf3_dma.h`,** which CI compiles and runs (`tests/dma_regime.rs`), as it does
  `kf3_gop.h`; kf3.c itself is not compiled by CI.
- **`kf3_instance_finalize` disarms** the notifier and listener: QEMU calls no exit for a failed
  realize, and `v3-broker` adds a realize step that can fail after the arming point
  (`kf3_broker_realize`), which on hot-plug would leave both in QEMU's global lists.
- **The arming point** is just after `memory_listener_register(&s->listener, …)`, not before it:
  `v3-cand-1` changes the lines above (`return` → `goto fail`).
- **amd-iommu ambiguity and unreadable properties** are UNTRACKED (fail-closed), not only ambiguity.

kf3.c was syntax-checked with QEMU 10.2.4's `libsystem` flags (`compile_commands.json` of
`/workspace/bench/qemu-build`, plus pixman) and `-Werror`, clean before and after, with no new
`-Wextra` warning. It is compiled for real at the box merge bar.

### 7.2 Commit B — the typed seam, Tier 2 (`509f590e`)

- **Decode sites carry `DevAddr`**: `kf_arch::UserdMem::Sysmem { base: DevAddr, size }` and
  `kf_arch::fault::ErrorNotifier::Sysmem { at: DevAddr }` (was `gpa: u64`), built in
  `kf-abi/src/notifier.rs`'s two decoders; the walker report's sysmem leaves in `desired_from_leaves`;
  the GSP guest-RAM port (`device.rs`); CE physical operands (`SlotWindow`).
- **`crates/kf-mem/src/dma.rs`** (new): `RamPiece`, `DmaRefusal`, `trait DmaResolve` (`resolve`;
  `resolve_one`, exactly one piece or `Fragmented`), `IdentityFn`. `kf-mem` gains a `kf-arch`
  dependency (pure; no cycle).
- **The kf-mem contract**: `desired_from_leaves`, `ApplyCfg.dma`, `VaManager::new(…, dma)`;
  `LeafRefusal::DeviceAddress`; a multi-piece run is refused by name in `apply_entry`.
- **`kf-qemu` `DmaSpace`** (`mem.rs`), the boundary; through it: the VA manager's leaves, the GSP
  port, `SlotWindow`, the completion probe, passthrough USERD and error-notifier birth, the Translated
  USERD view.
- **`kf-harness`**: gates 2, 3, 4, 5, 6, 7, 8 and `publish.rs` wrap their closures in `IdentityFn`
  (the design listed only 3, 8 and `publish.rs`; 2, 4, 5, 6 and 7 call `publish` too).

**Deferred, both still behind §4.3's gate in `RamMap`:**
- **display** ×4: `v3-broker` rewrites `display.rs` (Tier 3);
- **the PRAMIN window's sysmem target** (`PraminPool`'s closure in `kf-mem/src/cpuwin.rs` and its
  construction in `kf-qemu/src/mem.rs`): `v3-cand-1` (the merge candidate, through
  `v3-scratch-bound`) reworks exactly those lines (`PraminPool`'s state and `repoint`), so moving them
  now is a certain conflict. The design's collision map did not list `v3-cand-1`. Move it after that
  merges, together with the PRAMIN unit test the design asks for (*a slot with a refusing resolver
  shows scratch and counts the refusal*), which is not written yet for the same reason.

⊘ **The compile-time boundary arrives at Tier 3.** Until then the guarantee is §4.3's runtime gate
plus review: `DevAddr::translator_raw` is public and `RamMap`'s lookups are public.

### 7.3 Commit C — the documents

This file; `THE_DESIGN.md` §9.4 and `THE_OPEN_QUESTIONS.md`'s *"A guest IOMMU"* row marked
SUPERSEDED in their own text; `THE_TRANSLATED_PLANE.md`'s vIOMMU sentence (one single-piece resolve,
then arithmetic); audit S1-32's status, severity and row 29 of its table.

### 7.4 Tests (CI, no GPU) and the mutation each was run against

Each mutation was applied to the tree, the named tests were run, the tree was restored; every one
failed the tests named (`cargo test`, 2026-10-04).

| Test (file) | What it pins | Mutation → the tests that failed |
|---|---|---|
| `kf_arch::dma` units (`crates/kf-arch/src/dma.rs`) | the wire mapping (5 and `u32::MAX` refuse); `Default` is Unset and refuses; only Direct and Identity admit; transitions both ways; a refused lookup never runs; the counter | `admits()` also admits Translating → 3 tests (and the kf-qemu gate test); unknown wire → Direct → the wire test; a fresh cell starts Direct → the fresh-cell test (and the gate test); `admitted` runs a refused lookup → 2 tests; `admit` does not count → 3 tests (and the gate test) |
| `every_device_address_lookup_asks_the_dma_regime_first` (`kf-qemu/src/mem.rs`) | `block_for` and `file_range` consult the regime and count, only when it refuses, Unset included | `block_for` bypasses the gate → this test |
| `tests/dma_regime.rs` (kf-qemu) | the `KF3_DMA_*` values are the ones Rust reads; the C classifier needs positive evidence for IDENTITY; a mode switch passes through BLOCKED; the amd rule, fail-closed | TRANSLATING sent as 1 → all 5; RAM beside an IOMMU section reads IDENTITY → the classifier test; no sections reads IDENTITY → the classifier and mode-switch tests; unreadable amd properties trusted → the amd test; amd ambiguity ignored → the amd test; untracked loses precedence → the classifier test |
| `tests/wire_mirror.rs` (existing) | the entry point and `KF3_ABI` on both sides | `kf3_dma_regime` retyped to `u64` → 2 tests; `KF3_ABI` bumped on the Rust side only → 2 tests |
| `kf_mem::dma` units | the identity is one piece or a named hole; `resolve_one` is exactly one piece, `Fragmented` for two | the identity ignores the layout's offset → the identity tests (and the ledger golden); `resolve_one` takes the first of several → that test |
| `kf_mem::ledger` units | golden: the identity yields the rows the raw closure did (a layout with a hole); a refused leaf names its device address and the regime; two pieces are two rows at consecutive VAs | the ledger keeps only the first piece → the two-piece test; a leaf refusal loses its reason → 2 ledger tests and the apply test |
| `a_fragmented_or_refused_device_range_is_acked_failed_by_name` (`kf-mem/src/apply.rs`) | a two-piece run is acked FAILED by name, never placed at the first piece; a regime refusal names the address | `apply_entry` places the first of several → this test |
| `the_device_address_boundary_names_the_regime_or_the_hole` (`kf-qemu/src/mem.rs`) | every `DmaSpace` verb answers `Regime` (counted) in a refusing regime and `NotGuestRam` (uncounted) in an admitting one; the refusal sentences | `DmaSpace` admits in every regime → this test |
| `kf-abi` notifier tests and `rmrpc_bridge` (updated) | the notifier and USERD decode produce the guest's value as a `DevAddr` | the decode shifts the address → 3 kf-abi tests, 1 rmrpc_bridge test |

Not testable without a box: the listener's section counts against a real vIOMMU, and the
machine-done order.

### 7.5 Box tests for the merge bar (recorded, not run)

1. No vIOMMU: the CUDA and LLM rows unchanged, `dma=Direct`.
2. `intel-iommu` plus guest `iommu=pt`: works, `dma=Identity`.
3. `intel-iommu` with the guest translating: the driver load fails by name, `dma=Translating
   dma_refused>0`, the guest's other devices fine.
4. `dma-translation=off` on intel and on amd: works, `dma=Identity`.
5. kf3 listed **before** `intel-iommu`: the same results as 2 and 3.
6. `device_add` hot-plug behind `intel-iommu`.
7. `amd-iommu` (default `dma-remap=off`), guest translating **and** guest `iommu=pt`: both
   `dma=Untracked`, refused by name.
8. `amd-iommu,dma-remap=on` plus guest `iommu=pt`: `dma=Identity`, works.
9. virtio-iommu with the bypass identity domain: `dma=Identity`.
10. intel `caching-mode=off`, a runtime switch of kf3 to identity through sysfs with the driver
    unbound: stays `Translating` (the documented over-refusal); with `caching-mode=on`: `Identity`.

### 7.6 Collision map (checked with `git merge-tree`, 2026-10-04)

Against `v3-broker` (34696441), `v3-cand-1` (8a682f1b) and `v3-dispsw-exp` (86b4fa10), the branch
conflicts only on the `KF3_ABI` lines of `kf3.h` and `ffi_unsafe.rs`, which every ABI bump does;
`kf3.c`, `device.rs`, `mem.rs`, `chan.rs` and the kf-mem files merge clean. `v3-sec-nonpriv`,
`v3-sec-p0` and `v3-scratch-bound` show the same conflicts against master alone as against this
branch (none added). Not touched: `kf-cuda`, `kf-qemu/src/display.rs`, `kf-rm/src/display.rs`,
`kf-gsp/src/boot.rs`, `kf-rm/src/chanlink.rs` and `lib.rs`, `kf-host`, `kf-linux-raw`.
`v3-sec-rawaddr` (e4fb0190) is already contained in master.

---

## 8. Owner decisions

- **OD-1.** Refusal at use instead of at realize (§4.3).
- **OD-2.** The vIOMMU-invalidation sync point: async revocation with the overlap counter
  (recommended now); deferred completion through a QEMU vIOMMU patch (recommended if the counter is
  ever non-zero on a stock driver); or an exception to §A.4 (§5.3).
- **OD-3.** Require MAP-capable vIOMMU configurations (intel `caching-mode=on`, `snoop-control=off`,
  `x-flts=off`; amd `dma-remap=on`; virtio-iommu), and refuse SMMUv3, UNMAP-only setups and AMD
  IOMMUEN-off by name.
- **OD-4.** Document `bypass_iommu` / `dma-translation=off` as the zero-code answer for guests that
  need a vIOMMU only for interrupt remapping; such guests get no DMA confinement for kf3.
- **OD-5.** Refuse amd-iommu `dma-remap=off` as UNTRACKED (recommended; fail-closed — built that
  way). It also refuses `iommu=pt` guests in that configuration, whose zero-code remedy is
  `dma-remap=on` or `dma-translation=off`. The alternative, trusting it, is fail-open for translating
  guests.
- **OD-6.** Confirm that composing passthrough rows with the guest's own IOMMU table (§3.5: it can
  only remove a row or redirect within guest RAM, never add a row or a window) satisfies *"strictly
  only maps based on pte/pdb and nothing else"*.

## 9. Unverified

- That NVIDIA RM and UVM tear down GPU PTEs and invalidate before `dma_unmap`. The §5 overlap counter
  would measure it; correctness no longer depends on it.
- Whether stock RM ever points PRAMIN at sysmem (`kern_bus_gm107.c:4071-4086` makes it possible)
  (lens).
- AMD behaviour without a boot: that a Linux guest under `dma-remap=off` translates kf3 (read from
  source: QEMU `amd_iommu.c:1113-1121, 1259-1261`; Linux `drivers/iommu/amd/iommu.c:3128-3153`), and
  that `dma-remap=on` with `iommu=pt` reads IDENTITY.
- The GSP boot failure mode before `v3-viommu`: inferred, not run.
- Windows guests: DMA remapping, `caching-mode` handling, `kvm-msi-ext-dest-id`.
- SEV-SNP and TDX device addresses (§2, out of scope).
- Closed GSP firmware: not relevant, because the GSP is emulated.
- None of the behaviour in §1, §4 or §7 has been run on a box.

---

## The 2026-09-25 design (§P0–§P4), kept as written but renumbered

⊘ **SUPERSEDED IN PART, 2026-10-04, by §1–§9 above.** Its sections were numbered §1–§4 and are
renamed §P1–§P4 here so the 2026-10-04 design could keep its own numbers; nothing outside this file
cited them (`git grep V3_VIOMMU`, 789dee9f). Each superseded passage is marked where it stands.
§P1, §P2 and §P4 still hold.

**§P0 — the original STATUS line.** ⊘ Superseded by the STATUS at the top of this file.

**STATUS: DESIGN-ONLY, 2026-09-25.** Owner: *"just write it down … under kayfabe the security is
slim compared to bare metal GPU, but it's to support any stock OS."* ⇒ vIOMMU support is a
**compatibility** requirement (a stock guest may enable one), not a security feature. Nothing
below is built. Today a guest that puts a vIOMMU in front of kf3 would have its GPU's
system-memory DMA land at the wrong addresses (see §P3).

### P1. What a GPU page table holds for system memory

The NVIDIA driver never writes a physical address for sysmem. It pins pages and calls the kernel
DMA API — `dma_map_page_attrs` / `dma_map_sg` (`ogkm-580 kernel-open/nvidia/nv-dma.c:64, :222`,
`nv_dma_map_pages` `:439`, `nv_dma_map_sgt` `:363`) — and writes the returned `dma_addr_t` into
the sysmem PTEs. What that value is depends on the IOMMU mode of the machine the driver runs on:

| IOMMU mode | `dma_addr_t` = | the device can reach |
|---|---|---|
| none, or `iommu=pt` (identity domain) | the physical address | all of RAM |
| translated (DMA / DMA-FQ domain) | an IOVA; the IOMMU translates it to the physical address | only what the kernel `dma_map`'d (+ stale entries until the IOTLB flush) |

⊘ Terminology: `iommu.strict=1` is the **IOTLB invalidation policy**, not the translate-or-not
switch. Lazy mode (DMA-FQ, a common default) defers flushes, so a just-unmapped page stays
reachable briefly; strict closes that window. Either way a device keeps every mapping the driver
legitimately made — and a GPU driver maps a lot.

### P2. The host's IOMMU — covered by construction

kayfabe is an unprivileged host process and never sees an HPA or a host IOVA:
- guest RAM reaches the host GPU as an OS descriptor over the guest memfd (`kf-qemu mem.rs
  guest_ram_object`): host RM pins it and `dma_map`s it through the host kernel;
- the store is host vidmem that host RM maps.

So the host's IOMMU mode needs nothing from us. ⚠ It does set the **stakes of a bug**: with
`iommu=pt`, anything that makes our host channel emit a PHYSICAL address reaches any host RAM
(the subchannel hole, `00f62991`, would have). ⇒ `DENY_PHYSICAL_MODE_CE` on every host channel we
birth is the guarantee that holds on `iommu=pt` hosts too; it must never be dropped.

### P3. The guest's IOMMU — the gap

The guest driver runs the same `dma_map` inside the guest, against our emulated device:

- **No vIOMMU (today's q35 without `intel-iommu`):** `dma_addr_t` = guest-physical. The walker's
  sysmem leaf is a GPA → `RamMap::file_range` → memfd offset → host RM (host kernel's IOVA below).
  This is the translation chain kf3 implements.
- **vIOMMU present** (common: clouds with >255 vCPUs need x2APIC interrupt remapping; any guest
  wanting DMA isolation from its devices): the guest's PTEs hold **guest IOVAs**. kf3 treats them
  as GPAs ⇒ wrong memory.

#### Double translation — the design

```
guest PTE (guest IOVA) ──vIOMMU tables (guest-owned)──▶ GPA ──RamMap──▶ memfd offset ──host RM──▶ host IOVA/HPA
```

⊘ **Items 1–4 corrected 2026-10-04**, each above the text it corrects:
- *Item 1:* still right, but the address space is read at **machine-done**, never at realize
  (§4.1–§4.2): at realize it depends on the `-device` order.
- *Item 2:* ⊘ superseded. Translation comes from a **notifier-fed shadow with a generation**, not
  from `address_space_translate`: that call needs `rcu_register_thread`, can take the BQL, logs
  guest-visible faults on a miss and under-reports permissions (lens) (§3.2, §5.3). A run becomes k
  pieces (§3.3).
- *Item 3:* ⊘ superseded. The notifier runs under the BQL on **any** thread (a vCPU or the main
  thread), never waits on a kf3 thread, and **always** enqueues; revocation is asynchronous with a
  residual window, or deferred through a vIOMMU patch, or an exception to §A.4 — OD-2 (§5.3).
- *Item 4:* ⊘ wrong. The BAR1/BAR2/PRAMIN *apertures* are CPU MMIO, but their **sysmem targets are
  GPU DMA and are behind the vIOMMU** (§2 #2, #3).

1. **Address space.** kf3 obtains its DMA address space with `pci_device_iommu_address_space()`
   (QEMU; Cloud Hypervisor: virtio-iommu's equivalent) instead of assuming system memory.
2. **Translate per run, off the vCPU.** The VA-manager thread translates each walked sysmem run
   IOVA → GPA through that address space (`address_space_translate` / the IOMMU region's
   `translate`), splitting a run where the vIOMMU mapping is discontiguous. A run the vIOMMU does
   not map is a **fault** for the guest (as on real hardware: an IOMMU fault, not a read of
   whatever is there) — refused by name, never mapped.
3. **Invalidation is a sync point.** Register an IOMMU notifier: a guest vIOMMU unmap/invalidate
   must unmap our host rows for every run it covers **before** the guest's invalidation completes
   — the same shape as the MMU_INVALIDATE trigger (`THE_THREE_SYNCHRONIZATION_POINTS`). Mapping
   and unmapping stay authored host verbs; nothing guest-chosen reaches a host flag.
4. **Scope.** The CPU windows (BAR1/BAR2/PRAMIN) are guest-physical MMIO and are not behind the
   vIOMMU; only the device's DMA (sysmem leaves, USERD/GPFIFO in guest RAM, semaphores) is.
5. **Per family.** Nothing here depends on the GPU family; the vIOMMU is a VMM property.

#### Until it is built

⊘ **SUPERSEDED 2026-10-04 (§4).** The refusal at realize below was never built, and it is rejected:
it depends on the `-device` order and refuses by presence rather than by translation (§4.1). Replaced
by **refusal at use**: a regime classified at machine-done, with every guest-RAM lookup of a device
address refused unless Direct or Identity — and amd-iommu `dma-remap=off` refused as UNTRACKED,
which the realize check would have caught and the 2026-10-04 draft read as working. Built on
`v3-viommu` (§7.1); OD-1.

kf3 should **refuse at realize** when it sits behind a vIOMMU (the device's DMA address space is
not system memory), naming the reason, rather than silently mistranslating. ⊘ Not built either.

### P4. What this is and is not

- It adds protection **for the guest against its own (emulated) GPU**. It does not strengthen host
  isolation, which is bounded by host RM + the host IOMMU + `DENY_PHYSICAL_MODE_CE` (§P2).
- Without a vIOMMU the device can reach all guest RAM — kf3 pins the whole guest memfd as one OS
  descriptor, the same posture as any emulated DMA-capable device.
