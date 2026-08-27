# NVIDIA Terminology & Vocabulary Glossary

A living reference for the **nvkvm** project — the acronyms, BAR0 register-block
prefixes, boot-chain / GSP-RPC concepts, MMU structures, channel/engine terms,
and software-stack names that recur throughout this repo and the NVIDIA open
kernel driver (`research_clones/ogkm/` = open-gpu-kernel-modules).

Where a term maps to a concrete register offset or file in this repo, it is
cited (e.g. `src/qemu/mode2_regs_ga10x.h`). Expansions sourced from the open
driver's swref headers (`src/common/inc/swref/published/<arch>/<chip>/dev_*.h`)
and the project design docs (`docs/design/nvidia_gpu_internals.md`,
`mode2_bar2_mmu.md`, `mode2_device_data_model.md`, `mode2_m3_gsp_rpc.md`). Where
an expansion is not authoritative it is marked "(expansion uncertain)".

Conventions:
- A leading **`P`** on a register-block name = "priv" — i.e. it lives in the
  privileged BAR0 register aperture. The suffix names the hardware unit.
- A leading **`NV_`** is the driver's symbol namespace; **`NV0x/NVxxxx`** are RM
  object/engine *class* numbers.

---

## 1. Register blocks / hardware units (BAR0 "priv" map)

Each block occupies a fixed offset range in the BAR0 priv aperture. See
`docs/design/nvidia_gpu_internals.md` §1.1.

- **PMC** — *Master Control.* Top-level chip control: `NV_PMC_BOOT_0` (chip ID at
  BAR0 offset 0), `NV_PMC_BOOT_42`, `NV_PMC_ENABLE` (per-engine enables),
  `NV_PMC_INTR` (master interrupt status/dispatch). First registers the driver
  reads on probe. `NV_PMC_BOOT_0` = 0x0, `NV_PMC_BOOT_42` = 0xA00, see
  `src/qemu/mode2_regs_ga10x.h`.
- **PBUS** — *Privileged Bus.* The host/PCIe bus interface block. Holds the BAR0
  PRAMIN window control (`NV_PBUS_BAR0_WINDOW` = 0x1700) and the Maxwell-era BAR2
  instance-block register (`NV_PBUS_BAR2_BLOCK` = 0x1714). See
  `src/qemu/mode2_regs_ga10x.h`.
- **PFB** — *Frame Buffer / memory controller.* VRAM config, the GPU MMU
  (`NV_PFB_PRI_MMU_*`), BAR1/BAR2 window setup, and the WPR write-protect regions
  (`NV_PFB_PRI_MMU_WPR2_ADDR_LO/HI` = 0x1FA824/0x1FA828). Also reports usable FB
  size (`NV_USABLE_FB_SIZE_IN_MB` = 0x1183A4). See `src/qemu/mode2_regs_ga10x.h`.
- **PFIFO** — *FIFO / Host engine.* The work front-end ("HOST"): channels,
  GPFIFO, runlists, scheduling. Snoops USERD and takes doorbells.
- **PGRAPH** — *Graphics engine.* The graphics/compute engine register block
  (SM/TPC/GPC control, context switch).
- **PFALCON** — *Falcon microcontroller interface.* Generic per-engine Falcon
  control regs at each engine's base (SEC2 / PMU / NVDEC / GSP-boot falcon):
  IMEM/DMEM, CPUCTL, MAILBOX0/1, HWCFG2. `NV_PFALCON_FALCON_CPUCTL_HALTED`
  (bit 4), `NV_PFALCON_FALCON_HWCFG2_RISCV_ENABLE` (bit 10),
  `NV_PFALCON_DMATRFCMD` (DMA-transfer command). See `mode2_regs_ga10x.h`.
- **PRISCV** — *RISC-V ("Peregrine") core interface.* The GSP is a RISC-V core;
  `NV_PRISCV_RISCV_CPUCTL` `_ACTIVE_STAT` (bit 7) is the single "GSP booted"
  signal. `NV_PGSP_RISCV_CPUCTL` = 0x111388. See `mode2_regs_ga10x.h`.
- **PGSP** — *GSP Falcon/RISC-V register base* (0x110000). Mailboxes
  (`NV_PGSP_FALCON_MAILBOX0/1` = 0x110040/44), CPUCTL, HWCFG2, DMATRFCMD, and the
  GSP command-queue doorbell (`NV_PGSP_QUEUE_HEAD(0)` = 0x110C00). See
  `mode2_regs_ga10x.h`.
- **PSEC** — *SEC2 security-engine Falcon base* (0x840000). Used during HS-ucode
  (FWSEC/Booter) bring-up: `NV_PSEC_FALCON_CPUCTL/HWCFG2/DMATRFCMD`. See
  `mode2_regs_ga10x.h`.
- **PGC6** — *GC6 deep-sleep power island* ("AON" = always-on). Its **secure
  scratch** survives power transitions, so GFW boot progress lives here:
  `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT` = 0x118234, with its
  `PRIV_LEVEL_MASK` = 0x118128. See `mode2_regs_ga10x.h`.
- **PTIMER** — *GPU nanosecond timer.* GPU free-running ns clock. On GA10x
  relocated to 0xBB0000 (`NV_PTIMER_TIME_0/1` = 0xBB0080/84); its
  `PRIV_LEVEL_MASK` (0x9430) is at the legacy un-relocated offset. See
  `mode2_regs_ga10x.h`.
- **PRAMIN** — *Private RAM Instance window.* A 1 MiB BAR0 window (0x700000–
  0x7FFFFF) that aliases a slice of FB/instance memory so the CPU can poke
  instance blocks and page tables before BAR1/BAR2 paging is live. The base is
  selected by `NV_PBUS_BAR0_WINDOW`. `NVKVM_PRAMIN_BASE` = 0x700000,
  `NVKVM_PRAMIN_SIZE` = 0x100000. See `mode2_regs_ga10x.h`, `mode2_bar2_mmu.md`.
- **PROM** — *VBIOS ROM aperture* (`NV_PROM_DATA`, base 0x300000, 1 MiB). The
  VBIOS/devinit image (GFW) is read through this window. See `mode2_regs_ga10x.h`.
- **PDISP** — *Display engine* register block (scanout / heads / CRTC).
- **PVM / NV_PVM** — *Virtual-function / virtualization* register block. On
  Turing/Ampere the BAR2 bind register moved here:
  `NV_VIRTUAL_FUNCTION_PRIV_BAR2_BLOCK` = VF base 0xB80000 + 0xF48
  (`NVKVM_VF_BAR2_BLOCK` = 0xB80F48). See `mode2_regs_ga10x.h`.
- **PMU** — *Power Management Unit.* On-die Falcon microcontroller for power /
  clock / thermal management (pre-GSP, much of its role folded into GSP-RM).
- **IMEM / DMEM** — *Instruction Memory / Data Memory.* The per-Falcon code and
  data SRAMs into which signed ucode is DMA'd before the core is started.
- **NVDEC** — *NVIDIA Decoder.* Hardware video-decode engine (a Falcon-fronted
  engine). `dev_nvdec_pri.h`.
- **NVENC / MSENC** — *NVIDIA Encoder* (MSENC = "Multi-Standard ENCoder", the
  legacy name). Hardware video-encode engine (H.264/HEVC/AV1).
- **SEC2** — *Security engine 2.* A Falcon used to run Heavy-Secure (HS) ucode
  during boot (e.g. FWSEC/Booter); see §2.
- **CE** — *Copy Engine* (a.k.a. DMA copy). `dev_ce_base.h`; RM classes
  `*_DMA_COPY_*`.
- **GPC / TPC / SM** — *Graphics Processing Cluster / Texture Processing Cluster /
  Streaming Multiprocessor.* The compute hierarchy inside PGRAPH; SM is the unit
  that runs CUDA warps.

---

## 2. Boot & firmware chain ("fake-the-boot" target)

The Mode-2 boot handshake the driver observes (it only reads *success values*;
on real silicon these come from firmware). See `nvidia_gpu_internals.md` §3.

- **GFW** — *GPU FirmWare* (the VBIOS-resident devinit/init firmware). Driver
  polls `GFW_BOOT` `_PROGRESS == _COMPLETED (0xFF)` to confirm devinit finished.
  `NV_PGC6_GFW_BOOT_PROGRESS_COMPLETED` = 0xFF. See `mode2_regs_ga10x.h`.
- **GFW_BOOT** — the secure-scratch field (in PGC6 AON scratch) reporting GFW
  boot progress; boot check #1.
- **VBIOS** — *Video BIOS.* The card's ROM image (read via PROM); contains
  devinit tables and the GFW firmware.
- **devinit** — *device initialization.* The VBIOS-driven init sequence that
  brings up clocks/memory before the driver takes over.
- **FWSEC** — *FirmWare SECurity* ucode. An HS (Heavy-Secure) Falcon image run on
  SEC2; sets up FRTS. The driver only *patches in* the NVIDIA signature, never
  verifies it (the on-silicon HS bootROM does).
- **FRTS** — *Firmware RunTime Security (region).* A write-protected region FWSEC
  carves out in FB at boot; `kgspPopulateWprMeta_*` computes its layout.
  (expansion: "FW RunTime Security" per common usage.)
- **Booter** — the Heavy-Secure Falcon ucode that loads/unloads GSP-RM into/out
  of the protected WPR region ("Booter Load" / "Booter Unload"). Signed; never
  verified on a fake device.
- **HS ucode / Heavy Secure** — Falcon microcode that runs at the highest
  security level (level 3), entered via the on-silicon HS bootROM that checks the
  signature. FWSEC and Booter are HS ucode.
- **WPR / WPR2** — *Write-Protect Region (2).* A range of FB the booter marks
  write-protected, where GSP-RM and its metadata live. Driver checks
  `NV_PFB_PRI_MMU_WPR2_ADDR_HI` nonzero (boot check #6).
  `NV_PFB_PRI_MMU_WPR2_ADDR_LO/HI` = 0x1FA824/0x1FA828. See `mode2_regs_ga10x.h`.
- **LibOS** — the small RISC-V operating system GSP-RM runs on top of. The driver
  programs LibOS boot-args / init-args GPAs into GSP mailbox regs at reset.
  `LIBOS_REGION_STRIDE` = 32, `LIBOS_REGION_LOC_SYSMEM` = 1. See
  `mode2_regs_ga10x.h`, `mode2_m3_gsp_rpc.md`.
- **RISC-V / Peregrine** — the GSP's CPU ISA / core family. Boot check #2 reads
  `HWCFG2._RISCV` (core present); check #5 reads `RISCV_STATUS._ACTIVE_STAT`
  (core running).
- **CPUCTL** — Falcon/RISC-V *CPU control* register (start/halt/reset the core).
  `_HALTED` (bit 4) reports a Falcon has halted after running HS ucode.
- **MAILBOX0/1** — Falcon mailbox registers used to pass small
  args/status between the host driver and Falcon ucode (e.g. FWSEC result,
  LibOS args address). `NV_PGSP_FALCON_MAILBOX0/1` = 0x110040/44.
- **DMATRFCMD** — Falcon *DMA-transfer command* register; drives IMEM/DMEM loads
  of ucode from sysmem. `_IDLE` value 0x2 = IDLE&!FULL. See `mode2_regs_ga10x.h`.

---

## 3. GSP & RPC

See `nvidia_gpu_internals.md` §4 and `docs/design/mode2_m3_gsp_rpc.md`.

- **GSP** — *GPU System Processor.* The RISC-V coprocessor on the GPU that runs
  the real Resource Manager (GSP-RM). On a GSP-era card the host kernel driver is
  a thin client that boots GSP and then talks to it over shared-memory RPC.
- **GSP-RM** — the Resource Manager firmware running *on* the GSP. The host's
  `RmInitAdapter` succeeds once GSP-RM posts `GSP_INIT_DONE`.
- **RPC** — *Remote Procedure Call.* The host driver drives the GPU by sending
  RPC messages to GSP-RM over two shared-sysmem ring buffers.
- **Command queue / status queue** — the two SPSC rings: command (CPU→GSP) and
  status (GSP→CPU), set up by `GspStatusQueueInit`. Backing store is sysmem GPAs
  the host owns; no crypto unless Confidential Compute is on.
- **msgq** — the SPSC ring primitive (`msgq/msgq_priv.h`): a `msgqTxHeader`
  (32 B: version/size/msgSize/msgCount/writePtr/flags/rxHdrOff/entryOff) written
  by the TX side + a `msgqRxHeader` (readPtr) written by the RX side. See
  `mode2_m3_gsp_rpc.md`.
- **GSP_MSG_QUEUE_ELEMENT** — one message: `mctpHeader`, `nvdmHeader`, `checkSum`
  (whole element sums to zero), `seqNum`, then `payload[]` carrying
  `rpc_message_header_v` + the RPC body.
- **NV_VGPU_MSG_\*** — the GSP-RM RPC vocabulary (function codes). Many map 1:1
  onto the RM control/alloc ioctls Mode-1 already forwards.
- **GSP_INIT_DONE** — the first required status-queue reply
  (`rpc_init_done_v17_00`); `kgspWaitForRmInitDone` asserts `rpc_result == NV_OK`.
- **GSP doorbell** — a BAR0 register the kernel writes to wake GSP after posting a
  command element (`NV_PGSP_QUEUE_HEAD(0)` = 0x110C00).
- **radix3 / kgspCreateRadix3** — the GSP's own 3-level page tables built in
  sysmem at boot.
- **GspStaticConfigInfo** — a structure GSP reports at init (e.g.
  `bar2PdeBase`). `NVKVM_GSPSTATIC_BAR2PDEBASE_OFF` = 1672. See
  `mode2_regs_ga10x.h`, `src/qemu/mode2_gspstaticinfo_ga106.h`.
- **MCTP / NVDM** — *Management Component Transport Protocol / NVIDIA Data Model*
  header layers that wrap each RPC element. (expansion: NVDM = NVIDIA Data
  Message, uncertain.)

---

## 4. Memory & MMU

See `nvidia_gpu_internals.md` §2 and `docs/design/mode2_bar2_mmu.md`.

- **GPA** — *Guest Physical Address.* What the guest kernel/driver allocates
  sysmem in. Guest RAM is QEMU VMM memory, so QEMU can translate GPA→HVA.
- **HVA** — *Host Virtual Address* (QEMU/VMM process address).
- **FB** — *Frame Buffer.* GPU VRAM. "FB offset" = a GPU-physical VRAM address.
  `NVKVM_FB_SIZE_MB` = 12288 (12 GiB, RTX 3060). See `mode2_regs_ga10x.h`.
- **GVA / GPU VA** — *GPU Virtual Address.* Per-context address spaces the GPU
  MMU translates via multi-level page tables.
- **GMMU** — *Graphics MMU.* The GPU's memory-management unit; translates GPU-VA
  → physical via PDB→PDE→PTE walks. Formats in `dev_mmu.h`.
- **PDB** — *Page Directory Base.* The root pointer of a VA space's page tables;
  stored in the instance block (RAMIN) at `NV_RAMIN_PAGE_DIR_BASE_LO/HI`.
  `NVKVM_RAMIN_PDB_LO_OFF` = 0x200, `_HI_OFF` = 0x204. See `mode2_regs_ga10x.h`.
- **PDE / PTE** — *Page Directory Entry / Page Table Entry.* PDEs point at lower
  page-table levels; PTEs carry a physical page address + aperture + kind +
  privilege/volatile bits. PTE VALID = bit 0.
- **VER2 / VER3** — GMMU page-table *format versions.* GA10x uses **NV_MMU_VER2**
  (Pascal-format): 8-byte PDE/PTE, 5 levels (PD3→PD2→PD1→PD0→PT). `ADDRESS_VID` =
  bits 32:8 << 12. DUAL_PDE (PD0) is 16 B with separate small/big sub-table
  pointers. Per-level VA bit ranges in `src/qemu/mode2_regs_ga10x.h`.
- **DUAL_PDE** — a 16-byte PD0 entry holding two sub-pointers: one for the
  small-page (4 KiB) sub-table and one for the big-page (64 KiB) sub-table.
- **aperture** — which memory a PTE/PDE points into: VID_MEM (VRAM),
  SYS_MEM_COHERENT, or SYS_MEM_NONCOHERENT.
- **kind** — PTE tiling/format: PITCH (linear) vs block-linear (tiled).
- **instance block / RAMIN** — a per-channel/per-engine ~1 KiB block (in FB)
  holding the PDB pointer + engine context pointers + address limit. Bootstrapped
  through BAR2/PRAMIN before BAR1 paging is live. `NV_RAMIN_ALLOC_SIZE` = 4096.
  `dev_ram.h`.
- **BAR0/1/2/3** — *Base Address Registers* (PCI apertures):
  - **BAR0** — register aperture (the whole MMIO priv register file).
  - **BAR1** — windowed aperture into GPU memory (VRAM + mapped sysmem); user
    mmaps (USERD, doorbell, semaphores, framebuffers) go through here.
  - **BAR2/BAR3** — small secondary "instance block" aperture used to bootstrap
    page tables before BAR1 is fully up.
- **BAR0 window / PRAMIN window** — `NV_PBUS_BAR0_WINDOW` (= 0x1700): BASE[23:0]
  << 16 selects which FB slice the PRAMIN aperture aliases.
- **BAR2_BLOCK** — the register binding the BAR2 aperture to an instance block:
  PTR[27:0] << 12 = instblk FB addr, TARGET[29:28], MODE bit31 (1=VIRTUAL /
  0=PHYSICAL). `NVKVM_BAR2_BLOCK_MODE_VIRTUAL` = 0x80000000. See
  `mode2_regs_ga10x.h`.
- **WPR meta** — write-protect-region metadata struct (`kgspPopulateWprMeta_*`,
  `ct_assert(sizeof==256)`) describing the FRTS/WPR2 FB layout.
- **IOMMU / bus address** — what the GPU's DMA engines drive onto PCIe; equals GPA
  unless an IOMMU remaps. Mode-2 honors a guest-declared DMA window (VFIO-like).
- **double-mmap / GPA-window** — Mode-1's mechanism: map a host NVIDIA buffer into
  both the stub and (via MAP_FIXED into a pre-installed KVM memslot window) the
  guest, with WB PTEs, so a guest GPU-VA resolves to host-real memory.

---

## 5. Channels & engines (command submission)

See `nvidia_gpu_internals.md` §5–§8.

- **channel** — a hardware work queue bound to an engine; the unit of GPU work
  submission. RM class `AMPERE_CHANNEL_GPFIFO_A` = class `C56F`.
- **channel group / TSG** — *Time-Slice Group* (`KEPLER_CHANNEL_GROUP_A`): a group
  of channels scheduled together.
- **runlist** — the per-engine list of runnable channels HOST schedules from.
  Binding a channel to the wrong runlistId (GR vs copy) was a cuCtxCreate=401 bug.
- **GPFIFO** — *GP (GetPut) FIFO.* A ring of 8-byte GP entries, each pointing at a
  pushbuffer segment (GPU VA + length + flags). The channel consumes them in
  order.
- **pushbuffer** — a GPU-mapped buffer of **methods** (engine register writes
  encoded as `(method, data)` via `NVCxxx` class headers); the actual command
  stream.
- **method** — a single `(method-offset, data)` pair in a pushbuffer; an encoded
  engine register write (e.g. launch a grid, DMA copy).
- **USERD** — *USER Data* (per-channel user-mapped struct, in VRAM/sysmem, mapped
  via BAR1) holding `GP_PUT` (producer index the app advances), `GP_GET`
  (consumer index the GPU advances), plus semaphore/error fields.
- **GP_PUT / GP_GET** — the GPFIFO producer/consumer indices in USERD.
- **doorbell** — a write to the usermode (VF) region's doorbell register that
  tells HOST "channel N has work" without a syscall; the value is the channel's
  **work-submit token**.
- **work-submit token** — the per-channel identifier written to the doorbell;
  obtained from RM at channel setup (`GET_WORK_SUBMIT_TOKEN`, `ctrlc36f.h`).
- **usermode / VF region** — a BAR-mapped page (class `NVC361`-family) the app
  maps R/W; the doorbell register lives here. "VF" = Virtual Function.
- **semaphore** — a small memory location a pushbuffer writes on completion; the
  app polls it instead of involving the kernel.
- **notifier** — memory the GPU writes to signal operation completion/status
  (alternative to / alongside semaphores).
- **engine class** — RM object classes for engines: compute (`AMPERE_COMPUTE_B`),
  copy (`AMPERE_DMA_COPY_A/B`, carries an 8-byte `NVB0B5_ALLOCATION_PARAMETERS`
  engineType), etc.

---

## 6. RM object model & ioctls

See `nvidia_gpu_internals.md` §5.

- **RM** — *Resource Manager.* NVIDIA's core driver layer that owns the GPU object
  graph (clients/devices/memory/channels). On GSP cards it runs as GSP-RM on the
  GPU; the host side is a thin client.
- **client** — `NV01_ROOT` object; the root of a process's RM object graph.
  Identified by an `hClient` handle (e.g. `0xc1d00001`).
- **device / subdevice** — `NV01_DEVICE_0` (per-GPU-group) → `NV20_SUBDEVICE_0`
  (per-GPU handle).
- **VA space** — `FERMI_VASPACE_A`: a GPU virtual address space (its own page
  tables / PDB).
- **memory objects** — `NV01_MEMORY_*`, `NV50_MEMORY_VIRTUAL`, OS-descriptors for
  sysmem; allocated in VRAM/sysmem then **mapped** into a VA space (writes PTEs).
- **hClient / hObject / hParent** — RM object **handles**. A global,
  access-gated namespace (NOT fd-scoped — see project memory). Used to address
  objects in alloc/control/free ioctls.
- **class** — the numeric type of an RM object (`NV0xxx` / `NVxxxx`), selecting
  alloc-param layout and behavior.
- **NVOS structs** — the RM ioctl parameter structs: `NVOS21` (legacy alloc),
  `NVOS64` (alloc with extended fields), `NVOS46` (map memory), `NVOS56`
  (update mapping info), `NVOS02` (alloc memory), `NVOS54` (RM control), etc.
  ABI sizes are load-bearing (truncation bugs); enforced by abi_parity tests.
- **NV_ESC_\*** — the frontend ioctl numbers on `/dev/nvidiactl` / `/dev/nvidia*`:
  `NV_ESC_RM_ALLOC`, `NV_ESC_RM_CONTROL`, `NV_ESC_RM_FREE`,
  `NV_ESC_RM_MAP_MEMORY`, `NV_ESC_REGISTER_FD`, `NV_ESC_ALLOC/FREE_OS_EVENT`, etc.
  Always gate on `_IOC_TYPE == 'F'` (frontend) — NRs collide with UVM cmds.
- **RM control** — `NV_ESC_RM_CONTROL` (NVOS54): the catch-all "do a command on an
  object" call; command id selects behavior (e.g. `NV2080_CTRL_CMD_PERF_BOOST`).
- **NV01_EVENT_OS_EVENT** — an RM event object wrapping an OS event (an fd/eventfd
  the app waits on for GPU notifications).
- **work-submit / GET_WORK_SUBMIT_TOKEN** — RM control to fetch a channel's
  doorbell token (`ctrlc36f.h`).

---

## 7. Driver / software stack

- **nvidia.ko / nvidia-uvm / nvidia-modeset / nvidia-drm** — the host kernel
  modules: core RM, Unified Virtual Memory, display modeset (NVKMS), and the DRM
  KMS/render-node shim respectively.
- **UVM** — *Unified Virtual Memory.* Manages CUDA managed memory / page
  migration; ioctls on `/dev/nvidia-uvm` (ignore `_IOC_SIZE`). Binds fd→mm at
  `UVM_INITIALIZE`.
- **NVKMS** — *NVIDIA Kernel Mode Setting.* The display/modeset interface
  (`/dev/nvidia-modeset`); closed, never forwarded by this project by design.
- **libcuda** — the CUDA driver-API user library; the primary client whose ioctl
  stream nvkvm forwards.
- **PTX / SASS / PTX JIT** — *Parallel Thread eXecution* (virtual ISA) /
  *Streaming ASSembler* (machine ISA) / the just-in-time compiler
  (`libnvidia-ptxjitcompiler`) that lowers PTX→SASS. Version must match libcuda.
- **NVML** — *NVIDIA Management Library* (`libnvidia-ml`); backs `nvidia-smi`.
- **GLVND / EGL / GLX / Vulkan ICD** — the graphics user-stack vendor-dispatch
  layers; the NVIDIA Vulkan ICD enumerates via the DRM render node
  (`/dev/dri/renderD128`).
- **GBM / DRM-KMS / dma-buf / PRIME** — Linux graphics buffer/display
  primitives used in the present-path / scanout work.
- **nvproxy** — gVisor's reference ioctl-forwarding implementation; nvkvm mirrors
  much of its object/pointer-translation model. nvproxy is default-deny on
  control cmds / classes (nvkvm is closing that gap).
- **RMARGS** — *Resource Manager (boot) arguments* — the init-args structure
  passed to GSP-RM at boot via the LibOS region / mailboxes. (expansion
  uncertain; commonly "RM args".)
- **HAL** — *Hardware Abstraction Layer.* The driver's per-chip function tables,
  selected from `NV_PMC_BOOT_0`. Method suffixes like `_TU102` / `_GV100` /
  `_GM107` name the chip whose HAL implementation a function belongs to.
- **Confidential Compute / SPDM** — encrypted GSP transport + device attestation
  (`SPDM` = Security Protocol and Data Model). The one real attestation path;
  nvkvm never enables it.

---

## 8. nvkvm project terms

- **Mode-1** — the working forwarding model: guest ioctls are proxied over virtio
  to a per-guest-process host **isolate** that issues the real NVIDIA ioctls.
- **Mode-2** — the in-progress reverse-driver model: emulate an NVIDIA PCI device
  (BAR0 regs, GSP-RPC endpoint, submission rings) so a *stock* guest driver
  binds, then forward real compute into the Mode-1 core. Plan in
  `docs/design/mode2_plan.md`.
- **isolate** — a sandboxed host process (one per guest mm / per (device,CR3))
  that holds the real NVIDIA fds and issues forwarded ioctls.
- **stub** — `nvkvm_stub`: the small forwarding helper binary inside the isolate.
- **CR3 key** — Mode-2 identity = the guest userspace address space; QEMU keys an
  isolate on the vCPU **CR3** at the trapping MMIO (doorbell/USERD) access.
- **GPA window** — a large pre-installed KVM memslot over which forwarded host
  buffers are MAP_FIXED'd (with forced-WB PTEs) so guest GPU-VAs hit host memory.
- **abi_parity** — the test suite asserting forwarded struct sizes match the
  driver's, preventing silent writeback truncation.

---

## 9. General acronyms (quick reference)

- **PCIe** — PCI Express. GPU vendor id `0x10DE`. **BDF** = Bus/Device/Function.
- **MMIO** — Memory-Mapped I/O (BAR register/aperture access).
- **MSI / MSI-X** — *Message-Signaled Interrupts (eXtended).* PCIe interrupt
  mechanism; the GPU raises these for GSP replies, channel errors, completions.
  Mode-2 synthesizes MSI-X to the guest on real completion.
- **PCIR / PCIR struct** — the *PCI Data Structure* signature ("PCIR") in a PCI
  expansion-ROM (VBIOS) image header.
- **eventfd / poll** — the Linux fd readiness primitives the isolate uses to
  surface real GPU events for QEMU to turn into guest MSI-X.
- **VFIO** — *Virtual Function I/O*; the kernel framework for safe device DMA
  passthrough. Mode-2 mirrors its DMA-window enforcement in software.
- **ISA** — Instruction Set Architecture (here: RISC-V for GSP; PTX/SASS for SMs).
- **TSG** — Time-Slice Group (see channel group, §5).
- **AON** — *Always-On* (the PGC6 power island that keeps secure scratch alive).
- **PLM / PRIV_LEVEL_MASK** — per-register *Privilege Level Mask* gating which
  privilege levels may read/write a register. Boot check #1 requires the GFW
  scratch PLM read-protection level0 lowered.
- **HS** — Heavy Secure (Falcon security level 3; see §2).
- **GFLOP/s, HtoD/DtoH** — throughput units / host-to-device & device-to-host
  copy directions (used in the perf benchmarks).
