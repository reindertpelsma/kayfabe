# How an NVIDIA GPU Talks to Hardware — Internals Reference for Mode-2

Status: reference. Written 2026-06-03 to ground the Mode-2 reverse-driver work
([[mode2_plan]], [[mode2_attestation_spike_GO]]). Target silicon: **GSP-era**
(Turing TU10x → Blackwell), specifically **GA106 (RTX 3060)** to match the host.

Citations are file:line into the local research clones
(`research_clones/ogkm/` = open-gpu-kernel-modules @ 610.43.02,
`research_clones/linux/.../nouveau/` = nouveau). Where a mechanism is
architectural knowledge not pinned to a line, it's stated plainly.

> Mental model in one sentence: on a GSP GPU, the **real resource manager (RM)
> runs *on the GPU itself*** (the GSP RISC-V coprocessor); the host kernel driver
> is a thin client that boots that coprocessor and then talks to it over a
> shared-memory **RPC**, while user processes submit work directly to engines via
> **memory-mapped rings + a doorbell**. Mode-2 impersonates the GPU side of all
> three: the boot registers, the RPC endpoint, and the submission rings.

---

## 1. PCI presence — BARs and config space

The GPU is a PCIe function (vendor `0x10DE`). The driver `probe` reads PCI config
(vendor/device/subsystem/revision) to bind, then uses the BARs:

| BAR | Typical GA10x | Purpose |
|-----|---------------|---------|
| BAR0 | 16 MiB, MMIO, 32-bit | **Register aperture** — the entire MMIO register file (PMC, PFB, PFIFO, PFALCON, PRISCV, PGC6 scratch, …). Everything the driver pokes during boot is here. |
| BAR1 | 256 MiB+ (resizable), prefetchable, 64-bit | **Windowed aperture into GPU memory** (VRAM and mapped sysmem). User mmaps (USERD, doorbell page, semaphores, framebuffers) and CPU access to GPU buffers go through BAR1 via the GPU's page tables. |
| BAR2/BAR3 | small, 64-bit | Secondary aperture / "instance block" window on some arches (used to bootstrap page tables before BAR1 is fully up). |
| VGA/ROM | — | legacy VGA + the VBIOS ROM image (devinit/GFW lives here). |

`NV_PMC_BOOT_0` is at **BAR0 offset 0** (`dev_boot.h:27`) and encodes
ARCHITECTURE/IMPLEMENTATION/MAJOR/MINOR — the chip ID the driver uses to select
its HAL. **Mode-2 must report a GA106-consistent value** and keep PCI IDs, HWCFG,
and PMC boot regs all describing the same chip (spike §5.2).

**Mode-2:** BAR0 is a pure-software register model (we own every read/write).
BAR1 is the crux of address virtualization — the windows the driver programs
point at GPU-physical/sysmem addresses we record and translate to host/stub
addresses; raw guest addresses never reach the real GPU.

### 1.1 BAR0 register-block prefixes (the "priv" map)

Every block below sits at a fixed offset range in the BAR0 priv aperture. `P` =
priv/BAR0 register space; the suffix names the hardware unit:

- **PMC** — Master Control. `NV_PMC_BOOT_0` (chip ID), `NV_PMC_ENABLE` (engine
  enables), `NV_PMC_INTR` (master interrupt status/dispatch). First reads on probe.
- **PFB** — Frame Buffer / memory controller. VRAM config, the GPU **MMU**
  (`NV_PFB_PRI_MMU_*`), BAR1/BAR2 window setup, and **WPR** write-protect regions
  (`WPR2_ADDR_HI`, boot check #5).
- **PFIFO** — Host / FIFO engine. Channels, **GPFIFO**, runlists, scheduling —
  the work front-end ("HOST") that snoops USERD and takes doorbells (§6).
- **PFALCON** — generic Falcon microcontroller interface. Per-engine
  (SEC2/PMU/NVDEC/GSP-boot-falcon) control regs at each engine base: IMEM/DMEM,
  CPUCTL, **MAILBOX0/1**, **HWCFG2._RISCV** (boot check #2/#4).
- **PRISCV** — RISC-V ("Peregrine") core interface; the **GSP** is RISC-V.
  `RISCV_STATUS._ACTIVE_STAT` is the single "GSP booted" signal (boot check #3).
- **PGC6** — GC6 deep-sleep power island ("AON" always-on). Its **secure scratch**
  survives power transitions, so **GFW boot progress** (`GFW_BOOT`, check #1)
  lives here.

Boot handshake order: PMC (identity) → PGC6 (firmware booted?) → PFALCON/PRISCV
(GSP core alive?) → PFB (WPR up?) → sysmem RPC (§4).

### 1.2 Performance model — emulate the kernel-only regs, FORWARD the userspace mmaps

Parity (v2 must match Mode-1/native) hinges on NOT trapping the hot path. Two
classes of BAR access, handled differently:

1. **Kernel-driver register pokes** (boot regs, PFIFO/PFB control, GSP doorbell):
   low-frequency, kernel-only. → **Trap + emulate** in the QEMU C shell; parity
   here is irrelevant.
2. **Userspace fast-path mappings** (USERD, the usermode doorbell page, BAR1
   apertures onto channel/compute buffers, semaphores): the hot path. → **Back
   with the REAL host-isolate NVIDIA mappings**, installed into the guest as KVM
   memory regions exactly like Mode-1's GPA-window double-mmap. Guest-userspace
   accesses then hit real hardware at **native speed, no trap**.

So the emulated MMIO BAR the guest sees = **self-filled data** (the registers
only the NVIDIA *kernel* reads) **+ real forwarded mmaps** of the isolate's
NVIDIA device (the regions normally handed to guest userspace). Identical to how
Mode-1 forwards mmap fds — just presented through an emulated PCI BAR instead of
the virtio device.

**Doorbell + CR3 reconciliation:** we do NOT trap every doorbell (that would kill
parity). The CR3→isolate binding ([[mode2_isolation_cr3_key]]) is captured at the
**control path** — the channel-alloc / USERD-mmap, which is already trapped — and
the real host channel's doorbell page is then mapped through so the submit write
goes straight to hardware. CR3 is read once at setup, not per-submit.

### 1.3 DMA model — VA-range commands run in QEMU; honor guest DMA protection

DMA is the tricky part; the model (user-specified 2026-06-03):

- **All VA-range / memory commands execute in QEMU.** QEMU maps every guest GPA
  (guest RAM *is* QEMU VMM memory), so QEMU translates GPA→QEMU-HVA and issues the
  real NVIDIA ioctl (e.g. validate/map a VA range) itself, on the owning
  **isolate's control fd**. QEMU already holds all isolate fds, the same way it
  holds the forwarded mmap fds today — so it has the access rights to broker.
- **Honor guest-declared DMA protection (VFIO-like).** If the guest tells the
  PCIe/CPU side that the GPU device may only DMA into certain GPA ranges (the
  device's allowed-DMA window / IOMMU mapping), we enforce that: the
  address-virtualization layer maps **only** those authorized GPAs into the real
  GPU context. This is the Mode-2 analogue of how VFIO + the host IOMMU protect
  the host from a malicious device's DMA — here QEMU + the isolate sandbox
  constrain the real GPU's DMA-on-behalf-of-guest to exactly what the guest
  authorized, translated to host pages. Raw guest addresses never reach silicon.

### 1.4 Multi-GPU — design for N PCIe devices from the start

The device model is **per-instance, no globals**: support multiple emulated
NVIDIA GPU PCI functions in one VM, each bound to a host GPU (by BDF) or sharing
one. Isolates are keyed by **(emulated-device, CR3)**, not CR3 alone. (Mode-1 has
a `g_nvkvm_device` singleton — Mode-2 must avoid that pattern: thread the device
instance through all state.) Every register block, RPC endpoint, address-
translation table, and isolate table is per-device.

---

## 2. Memory & address spaces

Four address spaces are in play, and Mode-2's job is to keep them all virtual:

- **Guest physical (GPA)** — what the guest kernel/driver allocates sysmem in.
- **GPU physical (FB offset)** — VRAM offsets; in Mode-2 these index *emulated* FB
  that we back/translate.
- **GPU virtual (GVA)** — per-context address spaces the GPU MMU translates via
  multi-level page tables (PDB → PDE → PTE). A **PTE** carries the physical
  address + **aperture** (VID_MEM / SYS_MEM_COHERENT / SYS_MEM_NONCOHERENT) +
  **kind** (PITCH / block-linear tiling) + privilege/volatile bits.
- **Bus/IOMMU address** — what the GPU's DMA engines drive onto PCIe; equals GPA
  unless an IOMMU remaps.

Key structures the driver builds and hands the GPU as **physical addresses**
(all of which Mode-2 records and translates — spike §4):

- **Instance block (RAMIN):** per-channel/per-engine block holding the page-
  directory base (PDB) pointer + engine context pointers. Bootstrapped through
  BAR2 before BAR1 paging is live.
- **Page tables (radix):** `kgspCreateRadix3` builds the GSP's own page tables in
  sysmem; downstream, every channel/VA-space has its own.
- **WPR2 / FRTS region:** a write-protected VRAM region the booter sets up;
  `kgspPopulateWprMeta_TU102` (`kernel_gsp_tu102.c:754`, `ct_assert(sizeof==256)`)
  computes its FB-physical layout. We record these offsets; nothing reads them
  back as a secret.

---

## 3. Boot sequence (the "fake-the-boot" target — M1–M3)

On a real chip this is silicon→firmware; the kernel driver only *observes
success values* (the spike's central finding). Ordered, with the BAR0 registers
Mode-2 answers:

1. **GFW / VBIOS devinit complete.** Driver polls `NV_PGC6_AON_SECURE_SCRATCH_
   GROUP_05_0_GFW_BOOT`, field `_PROGRESS == _COMPLETED (0xFF)`
   (`dev_gc6_island_addendum.h:31-32`), after confirming the PLM read-protection
   level0 is lowered (`dev_gc6_island.h:27-31`). → **report COMPLETED + PLM down.**
2. **RISC-V core present.** `kflcnIsRiscvCpuEnabled` reads `NV_PFALCON_FALCON_
   HWCFG2`, field `_RISCV (10:10) == _ENABLE (1)` (`dev_falcon_v4.h:101-103`). →
   **report the bit.**
3. **FWSEC (FRTS) + Booter HS ucode.** Driver writes IMEM/DMEM with NVIDIA-signed
   ucode (it only *patches in* the signature, never verifies — ogkm
   `kernel_gsp_falcon_tu102.c:191`; nouveau `fwsec.c:244-253`), starts the
   falcon, waits for halt, reads `MAILBOX0`. → **report halt + MAILBOX0 = NV_OK.**
   The on-silicon HS-bootROM is what verifies the signature; on a fake device it
   never runs, so there is nothing to forge.
4. **Reset into RISC-V**, program **LibOS boot-args address** + **Booter Load
   args** (`memdescGetPhysAddr(pWprMeta…)`) into GSP mailbox regs — driver
   *writes* these GPAs; we **record/translate**, don't honor on real silicon.
5. **GSP active.** `kflcnIsRiscvActive_TU102` reads `NV_PRISCV_RISCV_CORE_SWITCH_
   RISCV_STATUS`, field `_ACTIVE_STAT (0:0) == _ACTIVE (1)` (`dev_riscv_pri.h:28-
   30`). **This single bit is the only signal the driver uses to conclude "GSP
   booted."** → **report ACTIVE.**
6. **WPR2 up:** `NV_PFB_PRI_MMU_WPR2_ADDR_HI` nonzero. → **report nonzero.**
7. **GSP-RM init done:** see §4 — post the `GSP_INIT_DONE` RPC.

`nouveau/r535_gsp.c` walks the identical sequence (booter load ~1362, riscv-active
:1121/:1788, then poll `GSP_INIT_DONE` :1791) with no driver-side secret read —
the existence proof.

---

## 4. GSP-RM RPC transport (the heart — M3/M4)

After "boot," the kernel driver does **almost nothing directly**; it drives the
GPU by sending RPCs to GSP-RM over two shared-sysmem ring buffers:

- **Command queue** (CPU→GSP) and **status queue** (GSP→CPU), set up by
  `GspStatusQueueInit`; addresses are sysmem GPAs we own. Transport is plain ring
  buffers, no crypto (spike §3).
- Each message is a `GSP_MSG_QUEUE_ELEMENT` (`message_queue_priv.h:52`):
  `mctpHeader`, `nvdmHeader`, `checkSum` (whole element sums to zero), `seqNum`,
  then a flexible `payload[]` carrying `rpc_message_header_v` + the RPC body.
  (An encryption tag prefixes the payload *only* if Confidential Compute is on —
  which we never enable.)
- The RPC vocabulary is `NV_VGPU_MSG_*` (`kernel_gsp.c:1427+` handlers). The
  driver writes a command element, bumps the write pointer, and **rings a
  doorbell** so GSP wakes; GSP posts replies on the status queue and raises an
  interrupt back to the CPU.
- **First required reply:** `GSP_INIT_DONE` (`rpc_init_done_v17_00`,
  `kernel_gsp.c:6293`) with `rpc_result == NV_OK` — `kgspWaitForRmInitDone`
  asserts this and `RmInitAdapter` then returns success.

**Mode-2:** our endpoint consumes command elements, validates the checksum/seqNum,
and posts status elements. `GSP_INIT_DONE` is a canned reply (M3). The rest of
the `NV_VGPU_MSG_*` surface (M4) is triaged: many map onto the **RM
control/alloc ioctls we already forward in Mode-1** — i.e. a GSP RPC like
"alloc object class X under client/device" is the same semantic as the
`NV_ESC_RM_ALLOC` we already proxy. So GSP-RM-on-the-GPU becomes "shim the RPC
into the Mode-1 forwarding core" rather than re-implementing RM.

---

## 5. RM object model & how a context is made

Above the transport, NVIDIA RM is an object graph (identical in Mode-1 and over
GSP-RPC; gVisor nvproxy and our `nvkvm_objects.c` already mirror it):

- **Client** (`NV01_ROOT`) → **Device** (`NV01_DEVICE_0`) → **Subdevice**
  (`NV20_SUBDEVICE_0`, the per-GPU handle).
- **VA space** (`FERMI_VASPACE_A`) — a GPU virtual address space (its own page
  tables / PDB).
- **Memory** objects (`NV01_MEMORY_*`, `NV50_MEMORY_VIRTUAL`, OS-descriptors for
  sysmem) — allocated in VRAM or sysmem, then **mapped** into a VA space (this is
  where PTEs get written and where Mode-1's GPA-window double-mmap lives).
- **Channel group** (`KEPLER_CHANNEL_GROUP_A`, a TSG) → **Channel**
  (`AMPERE_CHANNEL_GPFIFO_A` = class `C56F`) → **engine objects** (compute
  `AMPERE_COMPUTE_B`, copy `AMPERE_DMA_COPY_B`, …).

"Creating a context" (e.g. a CUDA context) = allocate a VA space + a channel
group + one or more channels bound to engines + the USERD/GPFIFO/pushbuffer
memory + map the doorbell — all via RM alloc/control calls. Mode-1 already does
this end-to-end (cuCtxCreate works); Mode-2 reaches the same RM via GSP-RPC shim.

---

## 6. Channels & command submission (GPFIFO / USERD / pushbuffers)

This is the **fast path** — user processes submit work to engines without the
kernel, via memory the GPU snoops:

- **Pushbuffer:** a GPU-mapped buffer of **methods** (engine register
  writes encoded as `(method, data)` via the `NVCxxx` class headers). This is the
  actual command stream (e.g. launch a compute grid, DMA copy).
- **GPFIFO:** a ring of 8-byte **GP entries**, each pointing at a pushbuffer
  segment (GPU VA + length + flags). The channel consumes GPFIFO entries in
  order.
- **USERD** (user doorbell/data): a small per-channel structure (in VRAM/sysmem,
  mapped to the app via BAR1) holding `GP_PUT` (producer index the app advances)
  and `GP_GET` (consumer index the GPU advances), plus semaphore/error fields.
- **Submission** (`nvidia-push.c:416-478`):
  1. App appends methods to the pushbuffer and a GP entry to the GPFIFO.
  2. App writes the new **`GP_PUT`** into USERD.
  3. On GPUs where HOST snoops USERD that's enough; otherwise the app **rings the
     doorbell**: writes the channel's **work-submit token** to the doorbell
     register in the **usermode (`VF`) region** (`*doorbell = token`, after a
     write barrier). The token identifies which channel has new work.
  - Completion is observed by the app polling a **semaphore** the pushbuffer
    writes (or via `GP_GET`/progress-tracker semaphore, `nvidia-push.c:156,278`).

So steady-state GPU work touches the kernel/GSP **not at all** — it's
pushbuffer + GPFIFO + USERD write + doorbell, then poll a semaphore.

---

## 7. Doorbells, the usermode region, and interrupts

- **Usermode/VF region** (class `NVC361`-family, `ctrlc36f.h`): a BAR-mapped page
  the app maps read/write; the **doorbell register** lives here. Writing the
  channel's `WORK_SUBMIT_TOKEN` there tells HOST "channel N has work" without a
  syscall. The token is obtained from RM at channel setup
  (`ctrlc36f.h` GET_WORK_SUBMIT_TOKEN).
- **GSP doorbell:** the kernel rings a separate doorbell (BAR0 register) to wake
  GSP after writing a command-queue element (§4).
- **Interrupts:** the GPU raises **MSI/MSI-X** to the CPU for GSP status-queue
  replies, channel errors, and engine completions. The driver's top-half reads an
  interrupt-status register (BAR0) to demux.

**Mode-2:** the **usermode doorbell page** is the one piece on the hot path we
*could* let the guest write to directly, but in the fake-GPU model there is no
real HOST to snoop it — so the doorbell write must trap (MMIO) and we translate
"channel N token rung" into a submission on the **real** host channel (created via
the Mode-1 core), after translating the pushbuffer/GPFIFO GPU-VAs to the real
context. MSI-X to the guest is synthesized when the real op completes / the real
GSP-equivalent reply is ready.

**Host→guest interrupt delivery (required for parity, [[mode2_interrupt_delivery]]):**
the real GPU interrupts the *host*; the isolate surfaces GPU events via
**eventfd/poll** (the existing OS-event mechanism Mode-1 forwards). QEMU wires
those event fds into its event loop and, on readiness, **synthesizes an MSI-X**
from the emulated GPU device so the guest's stock-driver ISR runs. This must
cover **any** host-side ioctl/eventfd/poll event source (GSP status-queue reply,
engine/channel completion, errors). Honor the guest's MSI-X enable/mask + the
interrupt-status registers (PMC_INTR) so masking behaves. Per-device / multi-GPU
aware. Bidirectional with the Mode-1 guest-signal→interrupt-forwarded-wait path
([[signal_interrupt_delivery_done]]).

---

## 8. mmaps — what userspace actually maps

Through the RM mmap path (Mode-1 already forwards these via the GPA window):

- **USERD** + **doorbell/usermode page** — mapped so the app submits without
  syscalls (§6, §7).
- **GPFIFO + pushbuffer** — GPU-mapped *and* CPU-mapped (BAR1 aperture) so the
  app writes methods.
- **Semaphore/notifier memory** — for completion polling.
- **Compute buffers** (cuMemAlloc, managed memory) — VRAM or sysmem, mapped into
  both the GPU VA space (PTEs) and the app's CPU VA. In Mode-2 these resolve
  through the address-virtualization layer into host/stub-owned memory the real
  GPU can reach.

Mode-1's "double-mmap + GPA-window MAP_FIXED + WB-PTE" machinery (the
forwarded-buffer work, #111) is exactly the substrate Mode-2 reuses to make a
guest-programmed GPU-VA resolve to host-real memory.

---

## 9. What Mode-2 emulates vs forwards (ties to [[mode2_plan]])

| Mechanism | Mode-2 action | Milestone |
|-----------|---------------|-----------|
| PCI config + BAR sizing | **Emulate** (GA106 identity) | M0 |
| BAR0 boot registers (§3 #1–#6) | **Emulate** (report success values) | M1–M2 |
| Signed GSP/FWSEC/Booter ucode | **Ignore** — never runs; no bootROM on fake device | M2 |
| Addresses the driver programs (WPR meta, LibOS args, radix3, BAR1 windows) | **Record + translate** GPA/GPU-phys/bus → host/stub | M2–M5 |
| GSP-RPC sysmem rings + `GSP_INIT_DONE` | **Emulate** transport; canned INIT_DONE | M3 |
| `NV_VGPU_MSG_*` RM RPCs | **Shim into Mode-1 RM ioctls** (don't re-model GSP-RM) | M4 |
| Channel/USERD/GPFIFO/doorbell submission | **Trap doorbell, translate, submit on real host channel** | M5 |
| Engine compute/copy execution | **Forward to real host GPU** (Mode-1 core) | M5 |
| Interrupts (MSI-X) | **Synthesize** on real completion / RPC reply | M3–M5 |
| Display scanout (framebuffer/VGA) | **Emulate** generic FB → host console (`eaf90fc`) | M6 |
| Confidential Compute / SPDM | **Never enable** (the one real attestation) | — |

The throughline: emulate the **front** (what the driver pokes and the rings it
reads), translate every **address**, and forward the **back** (real compute) into
the Mode-1 core we already have working.
