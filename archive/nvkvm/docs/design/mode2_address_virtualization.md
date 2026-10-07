# Mode-2 address virtualization — the reverse-driver core

Status: design, 2026-06-03 (from user brainstorm). This is THE core of the
reverse driver: converting the guest's GPU address maps back to host-userspace
GPU VAs / host memory. It is the layer the whole Mode-2 thesis rests on, and the
current `UVM_REGISTER_GPU` blocker ([[mode2-cuinit-sm-order-fix]]) is the first
place it's strictly required.

## Core principle: GPU-physical is OUR bookkeeping

The "GPU physical address space" the guest kernel driver sees — and the BAR
addresses of our emulated GPU — exist **only between the guest kernel module and
the QEMU device extension**. Neither the host kernel, the host GPU, nor guest
userspace ever observes it. So we define its semantics entirely; it is pure
bookkeeping. We are free to lazily allocate it, categorize it, and map each
page wherever is correct.

## The two translation chains we follow

Both start at a **guest GPU virtual address** and end at a **VMM (QEMU) VA**;
both pass through the same intermediate bookkeeping layers
(GPU-VA → GPU-phys → guest-phys/GPA). They differ only in what the final GPA
denotes:

1. **MMIO / PCIe-BAR maps** (e.g. USERD, doorbell, register windows):
   GPU-VA → (walk the guest's virtualized PDB) → GPU-phys → GPA **inside our
   emulated PCIe BAR window** → the matching BAR sub-range(s) → `mmap`/install.
   Multiple BAR ranges may match one VA span (if the physical pages are split).

2. **GPU DMA into CPU memory** (pushbuffers, GPFIFO rings, semaphores, UVM
   managed buffers, HtoD/DtoH staging):
   GPU-VA → (walk PDB) → GPU-phys → GPA **in an ordinary KVM memory slot**
   (usually guest RAM). Iterate the KVM memslots to map GPA → VMM VA, then
   read/write or OS-descriptor it into the host context's VAS.

   **`DMA_FILL_PTE_MEM` is exactly this (user 2026-06-04):** it means "for this
   context / GPU-VA space, when this VA is accessed, DMA directly into my (the
   guest's) host-process RAM." So a PDB/PTE entry can be a **DMA entry to a GPA**
   (system memory) rather than to virtual GPU-physical. Mechanism:
   - PTE is a sysmem/DMA entry → take its GPA → convert GPA → QEMU VMM VA by
     iterating KVM memory regions → hand `(VMM-VA pointer, size)` to the real
     NVIDIA ioctl on the host (OS-descriptor: `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`
     0x00DE) so the host GPU context DMAs the guest's actual RAM at the guest's
     chosen GPU-VA. Batch adjacent PTEs into one descriptor (size = the run).
   - PTE removed / PFB torn down → revoke the host access (free the OS-descriptor
     mapping in QEMU/stub) so a stale GPU-VA can't reach freed guest RAM.
   This is the unprivileged Mode-1 OS-descriptor path; the guest's GPU-VA
   resolves through the host context's MMU to the same guest RAM bytes.

The walk is a **range translation**: a set of contiguous GPU-VA pages goes in;
follow the PDB/PTEs; a set of (possibly more, if fragmented) physical page
ranges comes out. Batch adjacent pages — it is fast and the common case is large
contiguous spans.

**Output of a translation** (per the user): a tuple
`(host context/channel, stub mmap-fd with its GPU-VA, host VMM VA)`. We then run
the unprivileged nvidia alloc/`mmap` for the stub/isolate/host-context GPU-VA →
the GPA, i.e. install the real backing under the guest's chosen address.

## GPU-physical page categories

Every emulated GPU-phys page is in exactly one state:

1. **Special** — kernel doorbells / control regs. A small fixed set. Fully
   trapped, or read-only with write-traps, or read-write-but-tracked. (See the
   doorbell model in [[mode2-plan]]: kernel doorbells trap; userspace work-submit
   doorbells direct-map after channel-create.)
2. **Read-only constant** — VBIOS, chip-ID data, placeholder GSP firmware. Served
   from the captured blobs.
3. **Unallocated** — no context owns it, not allocated. R/W trapped; first touch
   promotes it. (Default state of the lazily-allocated FB.)
4. **Zero-unallocated** (optional) — like unallocated but reads as zeros, write
   traps. Lets debug tools that dump the whole FB read fast zeros without
   per-page allocation (map a single preallocated 16–128 MiB zero region,
   repeated over the untouched GPA range). Possibly overkill.
5. **Unassigned** — a real RM-allocated buffer with data written, but no GPU
   context uses it yet. Rare; correctness-only. Small ones live in host memory.
6. **Assigned** — a real `mmap` of the host GPU at a specific host-userspace
   context/isolate, installed at the guest PCIe GPA. The hot, real path
   (= the double-mmap + GPA-window forwarding, [[mode2-matmul-requirements]] §C/§D).
7. **Inaccessible** — outside the FB region; illegal on real HW → fault.

### The "unassigned" simplification (clear-on-assign)

Unassigned is the tricky state: if a real GPU context later maps that GPU-phys,
we must have preserved the bytes. Two options:
- Back unassigned by **anonymous RM allocations** that can be *transferred* into
  a real context on assign (move the GPU object).
- **OR** lean on the fact that the **NVIDIA driver always clears memory when
  assigning it to a new context** (never hands a context stale garbage). If that
  holds, we can simply **ignore writes to unallocated** memory: nothing the guest
  writes to not-yet-assigned GPU-phys ever needs to survive into a context,
  because the driver will wipe it on assign. This also cleanly answers *when can
  we free* stub-side RM objects: assigned→unassigned always wipes on re-assign,
  so it can never carry data back; it becomes unallocated and is freed on the
  host. **Preferred** (assume clear-on-assign) unless a host driver is observed
  doing the weird stale-transfer thing.

## Lazy allocation & capacity

We lazily allocate GPU-phys backing on first touch (the FB is sparse). Because of
that we can **advertise the full VRAM size even though the host also uses some
VRAM**; if a real backing allocation later fails, report it as a PDB-mapping
failure (or the HW-faithful out-of-memory the guest would see). Watch for this
when sizing FB vs. host availability.

## How this resolves the current blocker

`UVM_REGISTER_GPU` allocates a UVM GPFIFO channel; its GPFIFO ring lives at
GPU-VA `0x121010000`. cuInit hangs because `chan_exec` reads that ring as 0:
- The ring is chain #2 (GPU DMA into CPU memory): GPU-VA → PDB walk → GPU-phys →
  GPA (guest RAM) where the guest wrote the entry.
- We fail at the PDB walk: we don't have the UVM channel's VAS root PDB (it's
  not among the `0x90f10106` snoops, and the instance block is GSP-managed/empty),
  so we can't follow GPU-VA → GPU-phys, and the content-pick heuristic finds no
  snooped VAS that maps it.

So the immediate need is **capture every VAS's root PDB** (especially the
device-default VAS used by `hVASpace=0` channels), then walk → GPU-phys → GPA via
the KVM memslots (chain #2). The PDB is bookkeeping the guest kernel module
established and communicates to us; we must snoop *all* the places it does so
(not just `VASPACE_COPY_SERVER_RESERVED_PDES`), or have the guest module report
it explicitly. Once the walk resolves, the GPFIFO/pushbuffer/semaphore read
correctly and the init push completes (emulate-completion bucket); real compute
channels then use category-6 "assigned" forwarding.

## IMPLEMENTED + MEASURED 2026-06-04 (GA106, open 580.159.04 guest)

Hardware bring-up turned the design into concrete findings. Read these before
extending the side-table — they overturn two earlier assumptions.

### The GSP-RM map op is PROMOTE_CTX, not FILL_PTE_MEM
In GSP offload mode the legacy paravirt RPCs (ALLOC_MEMORY=4, MAP_MEMORY=7,
MAP_MEMORY_DMA=14, DMA_FILL_PTE_MEM=27) **never fire**. The whole init runs on
GSP_RM_ALLOC (103), GSP_RM_CONTROL (76), FREE (10). Leaf PTEs are filled
GSP-side, so `nvkvm_walk_pdb` FAULTs on every GSP-managed VAS (the PDB root from
`VASPACE_COPY_SERVER_RESERVED_PDES` 0x90f10106 only covers *reserved* PDEs, not
arbitrary leaf PTEs). The GR/compute context-buffer mappings, however, ARE
communicated to GSP via **`NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (0x2080012b)**:
`NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS` { hChanClient@+12, entryCount@+40,
promoteEntry[]@+48 } with each 32-byte entry
{ gpuPhysAddr@0, gpuVirtAddr@8, size@16, physAttr@24 }, physAttr[1:0] aperture
(0=VIDMEM, 1/2=SYSMEM). **This is the #2 side-table capture point** and is
implemented in `nvkvm_gpu_emul.c` (`nvkvm_snoop_promote_ctx` →
`nvkvm_record_va_map`, keyed by hChanClient; `nvkvm_chan_translate` consults it
first). Resolves every GR/compute channel — the path real matmul/LLM/Vulkan
workloads use.

### The wall: UVM-internal channel sysmem mappings are GSP-internal & UNOBSERVABLE
cuInit hangs busy-polling (`uvm_gpu_semaphore_get_cpu_va`, process RUNNING) on
the **UVM bootstrap CE channel's tracking semaphore** (client 0xc1d00001, class
AMPERE_CHANNEL_GPFIFO_A 0xc56f, gpFifoVA 0x121010000, hVASpace=0). Facts:
- That client allocates NO sysmem memory objects, NO os-descriptor, and never
  calls PROMOTE_CTX. Its GPFIFO/pushbuffer/semaphore mappings are established
  entirely **GSP-internally** — NO fn=76/103/10 RPC carries their physical.
- The GPFIFO physical is observable ONLY via the CPU's BAR1 write (FB 0x3130000
  for gpFifoVA 0x121010000). The pushbuffer (GP_ENTRY → VA 0x120000000) and the
  tracking semaphore are **SYSMEM** (`uvm_gpu_semaphore_pool` is always
  `UVM_APERTURE_SYS`); the CPU polls the semaphore in plain guest RAM (no BAR1
  read seen). The GPU releases it via a CE SEM_RELEASE in the push:
  payload = `(NvU32)(++channel->tracking_sem.queued_value)`, wait is
  `completed >= queued` with **no hard timeout** (warns at 30 s, spins forever).
- To satisfy it we must write the payload to the semaphore's **GPA**, which we
  cannot derive: the push is unreadable (sysmem VA 0x120000000 in a GSP-managed
  VAS) and no RPC conveys the GPA.

### PROVEN 2026-06-04: forging the UVM completion unblocks cuInit
A debug backdoor validates the mechanism end-to-end. Patched guest UVM
(`docs/kernel_patches/mode2_uvm_complete_proof.patch`) reports, in
`uvm_channel_end_push` for CE channels, the tracking-semaphore GPA + payload to
the emulated GPU via a BAR0 backdoor (offsets 0xFFF500/4/8, written through a
temporary `ioremap` of BAR0 — UVM has `parent->pci_dev`). QEMU
(`nvkvm_bar0_write`) DMA-writes the payload to that guest-RAM GPA, forging the
GPU's CE SEM_RELEASE. Result — **Mode-2 cuInit now PASSES**:
```
ok cuInit(0); devices=1; ok cuDeviceGet; ok cuDeviceGetName
```
The semaphore GPA varies per boot (not hardcodable) but is stable within a boot;
the pool holds many 4-byte semaphores (0x121000000 + 4*n), payloads increment
per channel — the forge handles all of them. NEXT WALL: libcuda segfaults right
after `cuDeviceGetName` (near-NULL deref, `at ...ffc8`), in `cuDeviceGetAttribute`
or `cuCtxCreate` — a downstream Mode-2 gap, not the UVM wall.

This is a DEBUG PROOF (the guest is untrusted; QEMU blindly DMA-writes a
guest-supplied GPA — a write-anywhere primitive). Production needs a validated
guest<->VMM mapping-report channel: the guest reports GPU-VA->GPA *mappings*
(bounded to its own RAM), QEMU records them in the #2 side-table, and the
existing chan_exec executes the real CE SEM_RELEASE (or forwards real work),
rather than the guest dictating an arbitrary write. See [[access-model-split]].

### Consequence for the plan — two ways past the wall
1. **Fuller fake-GSP page-table ownership.** When RM hands the channel's memory
   descriptors (instanceMem/userd/ramfc are in the c56f alloc; FB) and GSP would
   map the channel's GPFIFO/pushbuffer/semaphore, WE must allocate GPU-phys and
   *write the leaf PTEs into our FB ourselves* so the walk resolves. The gap:
   the sysmem pushbuffer/semaphore GPA is chosen by RM and not in the RPC stream
   — so this still needs the GPA from somewhere.
2. **Guest instrumentation (Mode-2 has no nvkvm guest module — stock driver).**
   The decisive empirical step: add a printk in `uvm_channel_end_push`
   (`uvm_channel.c`) dumping the channel's tracking-semaphore GPU VA + payload +
   the backing sysmem **GPA**, and the pushbuffer GPA. With the GPA, forge the
   release on the ch doorbell (write payload to the GPA via `pci_dma_write`).
   This is user-endorsed (guest printk OK) and is the next concrete step.

The deeper lesson: "fake the GSP" is leak-free for the boot/compute path
(PROMOTE_CTX gives us the maps) but NOT for RM/UVM-internal channels whose
mappings GSP would own end-to-end. Those need either (1) us to fully own those
VASes' page tables, or (2) the eventual reverse-driver guest module reporting
the GPU-phys↔GPA bookkeeping (the original design's premise — which the stock
Mode-2 driver does not provide).

## Capture strategy: #2 primary (map-call side-table), #1 only for correctness

Refined 2026-06-03 (user). Two ways to obtain the guest's GPU-VA → physical
mappings:

- **#2 — reconstruct from the map/alloc calls (PRIMARY).** As the guest issues
  the RM ops that *establish* a mapping (channel alloc's gpFifoOffset, MAP_MEMORY
  / MAP_MEMORY_DMA, UVM maps), record `GPU-VA span → physical (GPU-phys/GPA) →
  owning context/channel` in a side-table. `chan_translate` (and the forwarder)
  consult this table directly — **no page-table walk / VAS-root needed.** This is
  better because it inherently ties the virtualized GPU-physical to a context
  (the unit of forwarding/isolation). **This is also the fix for the current
  UVM-channel blocker**: capture the op that placed the GPFIFO at GPU-VA
  0x121010000 → its physical, instead of trying to walk the device-default VAS
  we can't root.
- **#1 — capture PTE writes / `DMA_FILL_PTE_MEM` (LIKELY REQUIRED FOR PROD).**
  Two roles: (a) the "unassigned" write-before-map correctness case, and (b) the
  bulk mapping path — `DMA_FILL_PTE_MEM` is the RPC RM uses to fill PTEs for
  LARGE mappings. **Caveat (user 2026-06-03): "never in OUR run" ≠ "never in
  prod".** Our only evidence is a minimal cuInit probe (`cup.c`); it allocates
  almost nothing. Real targets — 7B LLM (huge VRAM), games (textures/vertex/cmd
  buffers), full PyTorch/Vulkan — WILL do large allocations and very likely DO
  use `DMA_FILL_PTE_MEM`. So **do NOT conclude #1 is skippable from the cuInit
  run.** Validate any skip decision against the **full real-app matrix**
  (`tests/perf/run_matrix.sh`: PyTorch CNN/ViT/BERT, HPC, gpu-burn, 7B LLM,
  Vulkan, OpenGL), not a probe. Default: **support both #1 and #2.** The
  write-before-map / unknown-mapping path must **error / fall back gracefully**
  (per the [[mode2-plan]] validation policy), never silently mistranslate.
  Instrument both: a write-before-first-map detector AND a `DMA_FILL_PTE_MEM`
  (RPC fn=27) counter across the matrix.

## Implementation order (proposed)

1. **Map-call side-table (#2)** — snoop the RM ops that establish GPU-VA →
   physical mappings (channel alloc gpFifoOffset, MAP_MEMORY/MAP_MEMORY_DMA, UVM
   maps), record `GPU-VA span → physical → context` in a table; `chan_translate`
   consults it (no VAS-root/page-table walk). Unblocks the UVM channel: capture
   what placed the GPFIFO at 0x121010000. Also instrument the write-before-map
   check to confirm #1 is skippable.
2. **Range translator** — keep the PDB-walk fallback for VASes we *can* root
   (already works for snooped channels), but the side-table is authoritative;
   batch GPU-VA spans → physical ranges, FB/sysmem apertures.
3. **Category state machine** — track GPU-phys pages (1–7); default unallocated,
   clear-on-assign simplification; lazy FB.
4. **Chain #1 (BAR/MMIO install)** and **chain #2 (GPA via KVM slots)** wired to
   the stub: assigned pages become real host-context mmaps installed at the
   guest GPA (the double-mmap + GPA window).

Step 1 (#2 side-table) unblocks cuInit/UVM and is the primary capture; 3–4 are
the parity compute path. **#1 (`DMA_FILL_PTE_MEM` / PTE capture) is a separate
prod requirement, NOT optional** — add it before the real-app matrix (large
mappings in LLM/games/Vulkan will use it); the cuInit probe just never triggers
it. Gate the matrix on a `DMA_FILL_PTE_MEM` counter + write-before-map detector
so we know exactly when it's exercised.
