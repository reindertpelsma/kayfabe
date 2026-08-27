# Mode-2 data-plane architecture (consolidated plan)

Status: design, 2026-06-05. Synthesizes the user's bookkeeping-table brainstorm with
the verified doorbell/chid findings (docs/design/mode2_doorbell_chid.md). This is the
buildable plan for the Mode-2 data plane: how guest GPU memory, the doorbell, and
GPU<->CPU DMA are backed by real host GPU resources via UNPRIVILEGED host ioctls.

## Core principle (Mode-1 proven): forward, don't emulate

Anything the guest *userspace* touches on the GPU — USERD, GPFIFO, pushbuffers, compute
data, context buffers — is **real host GPU memory**, double-mmapped into the guest, and
all allocation/mapping is **forwarded to unprivileged host nvidia ioctls** (the isolate/
stub). We do NOT interpret or emulate guest GPU work. Mode-1 proved this is feasible and
fast. If we find ourselves trapping a guest-userspace data range, that's a smell — those
ranges are meant to be forwarded, not trapped. (Interpretation/replay = the rejected
"approach B"; never go there.)

## Two planes

1. **Memory plane** — GPU-physical-backed (FB / sysmem): USERD, GPFIFO, pushbuffers,
   data, context buffers. Backed by real host GPU memory; CPU access via BAR memslots;
   GPU access via per-channel page tables. *Bookkeeping below.*
2. **Register plane** — MMIO registers, NOT GPU-physical: the USERMODE page (doorbell +
   PTIMER). Handled as a "special" object: read-only memslot (native reads), write-fault
   handler (doorbell = chid-translate + forward). See doorbell doc §13.

The older brainstorm's one flaw was putting the doorbell in GPU-physical space; corrected
— it's a register-plane special object. Everything else of the brainstorm holds and is
adopted below.

## Bookkeeping structures (adopted from the brainstorm)

### GPGA table — Guest GPU-Physical Address -> backing object
The emulated GPU's physical address space (FB) is pure bookkeeping. Every RM/object alloc
that consumes GPU-physical space gets a valid guest-GPU-physical address (GPGA) registered
here. Page-granular (4K) lookup GPGA -> range:
```
struct gpga_page_range { u64 gpga_addr; u64 size_pages; gpu_memory_object *target;
                         u64 offset_in_target; bool readable; bool writable; /*CPU bits*/ };
```
(= the address-virtualization #2 side-table, [[mode2-address-virtualization]].)

### gpu_memory_object — the backing descriptor
```
struct gpu_memory_object { gpu_mem_mode_t mode;          // special | general | physical
                           fault_handler_fn fault;       // NULL if none (e.g. doorbell write handler)
                           int nvkvm_handle;             // -1 unset (isolate/stub handle)
                           nvidia_handle_t nvidia_handle;}; // host RM handle, reachable via nvkvm_handle
```
- mode `physical`: a single static object representing real host GPU memory; BAR ranges
  reference it via gpga_page_range (the double-mmap target).
- mode `special`: register pages (doorbell/USERMODE, BAR registers) — often RO-mapped +
  a write fault handler.
- gpu_emul references objects by **nvkvm handles**, not raw nvidia fds.

### PDB table — per channel/context GPU-VA -> GPGA (or sysmem DMA)
Separate from GPGA (kept per channel/context, simple paging):
```
struct pdb_range { ctx_id channel; u64 va_addr; u64 gpga_addr; u64 va_size; };
```
Two leaf kinds: (1) -> GPGA (vidmem), (2) -> sysmem/CPU DMA (GPA). This is the VAS layer;
GPU->CPU DMA falls out of (2) by forwarding the guest's maps as host RM_MAP_MEMORY_DMA at
the SAME GPU VA (caller-fixed, unprivileged).

## CPU -> GPU: BAR memslots + lazy demand-fault

Per BAR region the guest configures on our PCIe device, install KVM memslots. Most GPU-
physical is never CPU-touched, so default to **trap** and back **lazily**:
- Reserve a host-side region (mmap, PROT_NONE) with a QEMU fault handler; KVM_SET_USER_
  MEMORY_REGION it at the BAR-offset GPAs where GPGA ranges should appear.
- On guest CPU access (fault): look up the GPGA -> gpu_memory_object; if it has a valid
  backend, **mmap it now** (RM_MAP_MEMORY on the stub's fd -> host VA, the proven Mode-1
  primitive) and install/refresh the memslot at that GPA. Multiple BAR offsets can alias
  one physical object. Keep memslots in sync when GPGA/BAR pages change.
- Register-only pages (no GPGA: doorbell, control regs) are separate `special` objects,
  usually fully trapped or RO-mapped.

## The doorbell (register plane) — see mode2_doorbell_chid.md §13

USERMODE page = `special` object, **RO memslot backed by the host USERMODE mapping**:
reads native (PTIMER nanosecond clock free), writes fault -> handler. The handler is the
chid namespace translator: {runlist, guest vChid} -> {runlist, host sChid} -> write host
doorbell. Adaptive RW(zero-trap)/RO(trap-write); see §13 + the chid namespace analysis.

## chid namespace (PID-namespace model)

Guest allocates base-0 vChids in its own namespace; host has system sChids. Translation is
a boundary function (like kernel PID-ns translation): real SR-IOV does it in the VF
doorbell HW (zero-trap); we do it in the doorbell write handler (trap). No host->guest
"chid taken" signal exists or should (it would break the namespace). Reservation of a host
sChid block is the privileged-PF job in real SR-IOV; unprivileged we self-reserve via the
forwarded FIXED channel allocs + handle collisions by translation. Identity (vChid==sChid,
single-tenant/no-collision) enables RW/zero-trap doorbell.

## GPU -> CPU DMA

Inherent in the PDB layer: a channel's GPU-VA -> sysmem leaf is forwarded as a host
RM_MAP_MEMORY_DMA into the host channel's VAS at the same GPU VA, backed by the guest-RAM
GPA (KVM/GPA-window). The host GPU then DMAs to the same memory the guest CPU sees.

## UVM residency rule

Production Mode-2 must not depend on reading guest userspace VAs from QEMU. Guest
userspace mappings, `/proc/$pid/pagemap`, and `/tmp/m2_pbmap.txt` are diagnostic
only. The production interface is the same as the rest of the data plane:

- guest GR VA installed in a channel/context PDB;
- GPGA for vidmem-like leaves, or guest-RAM GPA for sysmem leaves;
- host RM objects and host RM_MAP_MEMORY_DMA mappings created by the owning isolate.

UVM pages need a specific rule because normal in-guest UVM migration is not a
valid passthrough boundary. An unprivileged QEMU process cannot reliably know
whether a UVM page is currently resident in host CPU memory or host GPU memory,
and QEMU is not the recipient of the host NVIDIA driver's GPU-fault/migration
interrupts. Therefore the guest driver's UVM state must be steered so UVM ranges
are guest-visible as system-memory/host-RAM resident. When the guest GPU wants
access, Mode-2 maps the corresponding guest-RAM GPA pages into the host context
VAS at the same GPU VA using the unprivileged host ioctl path. Host-side page
faults and migration are then entirely the host NVIDIA kernel's problem.

This means:

- QEMU records UVM external ranges as GPU-VA ranges plus RM/UVM identity and
  backing GPA/GPGA facts, not as guest process VAs to dereference.
- The host channel executes CE/GR work. QEMU may parse pushbuffers for bring-up
  diagnostics, but it must not emulate CE/GR data movement in the production path.
- If the host driver migrates a page for GPU access, the guest is not notified.
  Later CPU access by the guest reaches the same guest-RAM GPA through KVM; any
  host-side migration/fault handling must be resolved below QEMU by the host
  kernel and its UVM state.
- CR3 is only an isolate/process key. Treat the address space it names as opaque;
  do not use CR3 as permission to interpret guest userspace mappings.

## Build order

1. Stand up the forward backend in gpu_emul (isolate/handle/mmap from VirtIONvgpu) — M5.0.
2. GPGA table + gpu_memory_object + lazy BAR memslot fault-mmap (CPU->GPU memory plane).
3. Forward object/memory allocs -> populate GPGA; forward controls.
4. PDB per-channel forward (RM_MAP_MEMORY_DMA at guest VAs) — GPU VAS + GPU->CPU DMA.
5. USERMODE RO memslot + doorbell write handler (chid translate + forward); chid table.
6. cuCtxCreate -> first compute (the scrubber/ctx channels run on host; semaphore fires).

## Security (unchanged)

QEMU/stub unprivileged; only unprivileged nvidia ioctls; one isolate per guest userspace
process; apply Mode-1 allowlists/sanitizers before forwarding ([[access-model-split]]).

## Execution-plane build status (2026-06-05) — primitives PROVEN, assembly remaining

All execution-path primitives are built (gated behind the `m2exec` device prop, default
off = zero regression) and validated on the RTX 3060 / GA106 host:

- **map_dma FIXED** (`nvkvm_m2_map_dma`, NVOS46 V580): place a host memory object into a
  host VASpace at a chosen GPU VA. KEY: `hDma` must be an **NV01_MEMORY_VIRTUAL (0x0070)**
  mapper (`nvkvm_m2_alloc_virtmem`), NOT a raw FERMI_VASPACE_A (only virtual_mem.c
  implements MapTo). Proven into the live GR VASpace 0x5c000007.
- **ctx buffers already host-resident:** FIXED-mapping the PROMOTE_CTX va_map VAs returns
  0x51 NV_ERR_NO_MEMORY = already mapped (host RM self-promoted its GR ctx at the SAME guest
  VAs). Do NOT re-map them.
- **GPFIFO double-mmap:** resolve the GPFIFO guest-FB phys by walking the guest GR PDB
  (VA 0x200200000 -> FB 0xe0200000), `back_and_map` registers the FB overlay + FIXED-maps it
  into 0x5c000007 (st=0). Guest GP writes now land in the host channel's GPFIFO. USERD was
  already double-mmapped (M5.4).
- **doorbell primitives:** alloc host AMPERE_USERMODE_A (0xc561) + RM_MAP_MEMORY + mmap;
  fetch the GR channel work-submit token via NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN
  (0xc36f0108) -> token=0xc. Ringing = write token to usermode_qva + 0x90.

### Remaining final assembly (the cuCtxCreate keystone) — DO ATTENTIVELY
Order, each a checkpoint (the RING is the only wedge-risk step — keep it last):
1. **Forward channel schedule** (safe): `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` (0xa06c0101) on the
   GR TSG (0x5c000012; track it like m2_gr_channel), params `NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS`
   {bEnable=1}. The host TSG isn't scheduled today (schedule is a control; shadow_fwd only
   forwards allocs/frees).
2. **Map pushbuffers** (safe): at the guest doorbell, walk the guest GPFIFO entries
   [gp_get,gp_put) -> pushbuffer VAs -> resolve phys (guest PDB walk) -> `back_and_map` each
   (double-mmap + FIXED into 0x5c000007). Also double-mmap the completion-semaphore buffer so
   the guest's PRAMIN poll sees the HOST GPU's write.
3. **Disable chan_execute faking** under m2exec (so a green guest can ONLY come from the host).
4. **RING** (wedge-risk): on the guest doorbell write the token (0xc) to usermode_qva+0x90.
5. **VERIFY ON HOST**: `ssh vh nvidia-smi` must show utilization/process AND the completion
   semaphore must be written by the GPU (not QEMU). Guest-green + host-idle = emulated, FAIL.
   See [[mode2-real-forward-not-fake]]. If the host GPU wedges (100% util / no procs): reload
   the host driver (rmmod nvidia_uvm nvidia_drm nvidia_modeset nvidia; modprobe nvidia).

## cuCtxCreate blocker RE-DIAGNOSED (2026-06-05) — it's a GR-VAS page-table poll, not channels

CRASHWIN data (m2exec run): when cuCtxCreate hangs, the guest RM busy-loops (~13.7k iters)
manually WALKING/SCANNING its own GR VAS page tables via PRAMIN — PDE chain
0x2f3392000->0x2efbc3000->4000->5000 (PD0 dual-PDE: SMALL half @0x2efbc5000 = 0 / not
installed; BIG half @0x2efbc5008 -> 0x2efbc6000), then a swath of big-PT entries
(0x2efbc6188..6360). The walk TARGET page is never read; there are NO channel-USERD (gva!=0)
polls. So it is the guest KERNEL polling GR-VAS PAGE-TABLE STATE, not libcuda polling a
channel completion. => the busy worker channels (client 0xc1d00001, scrubber gpfifo
0x1210d0000) are UNRELATED to this hang; the M5.9 multi-channel channel-forward would NOT
clear cuCtxCreate. The real gap is GR-VAS page-table POPULATION (the guest awaits a mapping/
state GSP would install) — or an event the guest waits on before installing it. The
exec-forward primitives (map_dma/double-mmap/USERMODE/token/schedule) remain correct and
reusable, but must target populating the guest's GR page tables / the awaited mapping, not
channel rings, for cuCtxCreate. See memory mode2_cuctxcreate_pagetable_poll.

## cuCtxCreate crash access-path RESOLVED (2026-06-05) — un-backed CPU-mmap of RM sysmem

gdb + strace at the rbp=0 SIGSEGV: ALL guest open/ioctl/mmap SUCCEED (no failed fd; the
gdb rdi=-1 was a mid-computation value). The crash is libcuda dereferencing a NULL it READ
from a CPU-mmap'd RM buffer. Pre-crash pattern = RM_MAP_MEMORY (ioctl NR 0x4e / NVOS33) +
mmap(MAP_SHARED|MAP_FIXED) on /dev/nvidia0 and /dev/nvidiactl:
  mmap(0x200200000, 2 MiB,  /dev/nvidia0)     GPFIFO (FB; covered by m2_fbback via BAR1)
  mmap(0x200400000, 64 MiB, /dev/nvidiactl)   large RM region   <- NOT FB; uncovered
  mmap(0x77f2f2ddf000, 4 KiB,/dev/nvidiactl)  small RM struct (last mmap pre-crash)
The /dev/nvidiactl maps are RM SYSTEM/heap memory (NV_CTL device), NOT GPU FB -> NOT covered
by the FB/PRAMIN m2_fbback overlay -> content is UN-BACKED (zeros) -> libcuda reads a NULL
struct ptr -> rbp=0 deref. This is the access path the FB-read CRASHWIN probe couldn't see
(direct CPU mmap, not a PRAMIN/BAR access).

NEXT BUILD: back the RM_MAP_MEMORY+mmap'd buffers with REAL host content. On the guest's
RM_MAP_MEMORY (NR 0x4e), make the guest CPU mapping resolve to the forwarded host object's
real memory (double-mmap: we already forward the alloc; map the host object + overlay the
guest mmap target — including the /dev/nvidiactl sysmem maps, not just FB). Pin which mapped
buffer holds the NULL (instrument the 0x4e handler + the mmap GPA, or LD_PRELOAD-log the bytes
libcuda reads from 0x200400000 / 0x77f2f2ddf000 pre-crash). See memory
mode2_cuctxcreate_pagetable_poll. Supersedes the "multi-channel forward" and "page-table
population" next-steps (both were symptoms, not the crash).

## cuCtxCreate root cause CONFIRMED (2026-06-05): un-backed CPU mmap; fix = item-2 memory plane

gdb memory dumps at the crash: both GPU-mapped CPU regions libcuda reads are ALL ZEROS —
0x200200000 (GPFIFO, /dev/nvidia0) and 0x200400000 (64 MiB, /dev/nvidiactl). The crash is a
method dispatch through a zeroed structure (`*global -> +0x48 -> vtable -> call *0x560`) on the
GR channel -> rbp corrupted -> SIGSEGV. With m2exec=on the GPFIFO backing FIRES (m2_fbback at
FB 0xe0200000) but the region STAYS ZERO -> the guest's CPU mmap does NOT route through the FB
overlay (it uses the BAR1 mapping / possibly guest-RAM, which resolves elsewhere than the
channel-vaspace FB addr the overlay was keyed on). MECHANISM, not timing (backing ran before
crash; 133 doorbells). RULED OUT this session as the cause: channels, GR page-table population,
faked controls (fn=76), failed fds — all red herrings/symptoms.

THE FIX (singular) = plan item-2 CPU->GPU memory plane: back the guest's CPU mmaps of GPU
objects with the forwarded host objects' real memory. Immediate next step: instrument
nvkvm_baraperture_read to log the bar1_pdb resolve of VA 0x200200000 (and whether libcuda's
read traps there) -> decide BAR1-overlay vs KVM-memslot-over-guest-RAM -> back the CPU-mmap'd
regions. See memory mode2_cuctxcreate_pagetable_poll.

## MECHANISM RESOLVED: guest-RAM, not BAR1 — fix = Mode-1 GPA-window memslot (2026-06-05)

Code smoking gun: emulated BAR1 = 256 MiB (bar1_size=256<<20, MMIO). libcuda's CPU mmaps are
at 8 GiB+ VAs (0x200200000, 0x200400000) -> cannot be BAR1 accesses; CRASHWIN logged no BAR1
access. So the guest driver maps these GPU objects to GUEST RAM (no real VRAM), CPU reads
don't trap, and the m2_fbback overlay can never cover them (why m2exec leaves them zero).

FIX (singular, plan item-2) = back the guest-RAM GPA of each CPU-mmap'd GPU object with the
forwarded host object's real memory via a KVM memslot (KVM_SET_USER_MEMORY_REGION) — Mode-1's
GPA-window mechanism (docs: gpa_window_design). Intercept the guest object alloc + CPU-map to
learn the guest-RAM GPA + size, RM_MAP_MEMORY the host object -> host VA, install the memslot.
Reuses Mode-1 GPA-window code; NOT the FB overlay. This is the one remaining cuCtxCreate fix.

## cuCtxCreate DIAGNOSIS CONVERGED (2026-06-05): un-backed SYSMEM GR-context buffers

LD_PRELOAD mapshim correlated the crash buffers to RM objects (by RM_MAP_MEMORY len):
2 MiB GPFIFO = hMem 0x5c000014, 64 MiB = 0x5c000016, 4 KiB (last mmap before crash) =
0x5c000018 (client 0xc1d00003). NONE are in the SHADOW (GSP_RM_ALLOC fn=103) list -> the
guest RM allocated them LOCALLY (NVOS32/VidHeapControl, ioctl 0x2a; not forwarded) with no
GSP-RPC. Combined with 0 BAR1 reads -> these are SYSMEM (guest RAM) GR-context buffers the
guest CPU-RM manages itself (no GSP cooperation), CPU-mapped by libcuda, that the GPU should
fill (golden GR context / DMA) but doesn't (no host execution, no DMA forwarding) -> libcuda
reads zeros -> NULL deref -> rbp=0 SIGSEGV.

FIX = the full Mode-2 data plane (multi-week keystone): the guest-RM-managed sysmem GR
objects must be backed by host memory the host GPU fills — forward the guest's sysmem GPU
mappings (RM_MAP_MEMORY_DMA) so the host GPU DMAs into the guest RAM (GPU->CPU DMA, item-4) +
forward channel execution so the GPU runs the GR-context fill. Diagnosis fully converged:
channels (no), page-table poll (symptom), faked controls (not the direct filler), fds (ok);
the EXACT buffers + their local-sysmem-alloc path are now identified. See memory
mode2_cuctxcreate_pagetable_poll.

## item-4 hard prerequisite (2026-06-05): shared guest-RAM memfd so the stub can OS_DESCRIPTOR it

To back the guest's sysmem GR buffers, the host nvidia driver (STUB process) must
OS_DESCRIPTOR-register the guest RAM. But the stub is a separate process; QEMU's guest RAM
(anon mmap) isn't in its address space. Mode-1 works only because the guest nvkvm MODULE
allocates in a SHARED GPA-window (memfd); the stock Mode-2 driver allocates in ordinary guest
RAM with no cooperation. So item-4's prerequisite:
  1. back Mode-2 guest RAM with -object memory-backend-memfd; pass the fd to the emul device.
  2. share the fd to the stub (SCM_RIGHTS); stub mmaps it (any guest GPA -> stub VA).
  3. OS_DESCRIPTOR(memfd + gpa_offset, size) primitive (NV01_MEMORY_SYSTEM_OS_DESCRIPTOR 0x71).
  4. for each GR-VA->guest-GPA sysmem mapping (va_map sys=true / PROMOTE_CTX + more), OS_DESCRIPTOR
     + RM_MAP_MEMORY_DMA FIXED at the GR VA into the host GR VAS.
  5. forward channel execution so the host GPU DMA-fills the buffers libcuda reads.
The host-GPU-DMA-to-shared-RAM step is Mode-1-proven (partly de-risks the DMA-virt concern);
the work is the shared-RAM plumbing + GR-mapping enumeration + execution. Multi-week keystone.

## cuCtxCreate crash — CORRECTED root cause (2026-06-05 late): unfilled VIDMEM ctx buffer via BAR1

Boot-free host-vs-guest LD_PRELOAD diff (shims in tests/mode2/shims/) + guest hObject<->class
correlation DEFINITIVELY corrects the earlier "un-backed sysmem" diagnoses above — those were
WRONG. Findings:

- Host (real GA106, **same open KMD 580.159.04 + same libcuda** as guest) cuCtxCreate: allocates
  client/device/subdevice/vaspace then the per-channel set, does 25 RM_MAP_MEMORY but only ONE
  MAP_SHARED mmap (the 2MiB GPFIFO, which stays ALL-ZERO), and PASSES. A zero CPU buffer is not
  fatal.
- Guest replays the host's alloc sequence IDENTICALLY through 0xc56f (channel) + 0xc7c0 (compute),
  then SIGSEGVs (rbp=0) exactly where the host does its next 0xc7b5 (DMA copy).
- The crash buffers are **VIDMEM**: hMem 0x5c000016 (64MiB) and 0x5c000018 (4KiB) are alloc
  **class 0x3e = NV01_MEMORY_LOCAL_USER**, ret=0 (alloc SUCCEEDS). Only the GPFIFO (0x5c000014) is
  class 0x40 sysmem — sysmem on the host too. NEITHER side mmap()s the 0x3e buffers -> libcuda
  reads them via the **BAR1 aperture**, not a CPU mmap.
- HOST BAR1 read -> real GPU vidmem -> GPU-written **golden GR context** (non-zero) -> works.
  GUEST BAR1 read -> emulated vidmem (m2_fbback/guest-RAM) -> **ZERO** (never filled) -> NULL deref.

So the singular cuCtxCreate blocker = the emulated GPU's VIDMEM is never filled with the golden GR
context that the real GPU writes. FIX = CPU->GPU memory plane (item-2) for vidmem-via-BAR: the
guest's BAR1 read of the vidmem GR ctx (0x5c000016) must resolve to the host's real golden-context
vidmem. The host HAS the golden ctx in host vidmem (self-promoted from the forwarded channel/
compute allocs), but 0x5c000016 is allocated GUEST-LOCALLY (not in the fn=103 SHADOW stream) so
there's no host counterpart handle. Options: (A) host GPU fills it — OS_DESCRIPTOR 0x5c000016's
emulated-vidmem (guest-RAM) backing + map into the host GR VAS + trigger golden-ctx load
(execution); (B) copy host golden-ctx content into 0x5c000016's m2_fbback backing (needs to name
the host ctx buffer; GET_CTX_BUFFER_INFO was privileged 0x1b on the unprivileged stub). The CPU-RM
regkey RMInstLoc* (force aperture) does NOT change it — aperture is already vidmem; the problem is
content, not location. Supersedes ALL prior sysmem/page-table/channel diagnoses for this crash.

## REFACTOR PLAN (2026-06-05, user-directed): replace fb_pages with the GPGA/gpu_memory_object model

Decision (user): the ad-hoc memory backing is the core flaw and must be replaced wholesale,
not patched. Point-fixes (M5.7/M6.5/M6.6) kept dead-ending on it. Evidence it's the foundation,
not a logic bug: at the cuCtxCreate rbp=0 SIGSEGV, the libcuda dispatch chain is INTACT
(global->A(heap)->B(libcuda data)->FP=valid libcuda function with a normal prologue) — so it is
NOT a corrupt function pointer from a wrong control value; the corruption is downstream of the
broken memory model. Also: 0 BAR1 reads in the crash window, so the buffer libcuda faults on is
reached via a path the current fb_pages/m2_fbback design does not even cover.

WHY fb_pages is fatal: emulated "vidmem" = g_malloc0 pages in a hash (nvkvm_fb_page). Coherent
for guest-only access (BAR1/PRAMIN/BAR2 -> our handlers), but the real host GPU CANNOT touch it.
So nothing the host GPU must produce (golden GR ctx, compute output, HW semaphores) can ever land
where the guest reads it. Every GPU-physical byte that must cross to the real GPU has to live in
REAL host-GPU memory (a host RM object, double-mmapped), not malloc.

### Target model (from the bookkeeping section above)
- `gpu_memory_object { mode(special|general|physical); fault_handler; int nvkvm_handle;
  nvidia_handle; void *cpu_qva; uint64_t host_va_in_gr_vas; ... }` — one descriptor per real
  backing (a host RM_ALLOC, double-mmapped: cpu_qva for QEMU/guest-CPU view, mapped into the host
  GR VAS at the guest VA for the host-GPU view).
- GPGA page table: `gpga_page_range { u64 gpga_addr; u64 size_pages; gpu_memory_object *target;
  u64 offset_in_target; bool readable, writable; }` — page-granular GPGA -> (object, offset).
- `nvkvm_fb_read/write` resolve fb_addr(=GPGA) -> gpga_page_range -> object->cpu_qva+offset.
  fb_pages becomes the FALLBACK only for GPGAs with no real backing (pure guest bookkeeping that
  the host GPU never needs); everything GR/channel/ctx is real-backed.

### Sequenced increments (each gated by m2exec, each a commit + no-regression boot)
- R1. Introduce structs + a gpga table keyed by page; `nvkvm_m2_gpga_lookup(fb_addr)`. Route
  nvkvm_fb_read/write through it (miss => current fb_pages path => zero behavior change). COMMIT.
- R2. `nvkvm_m2_gpga_back(va, fb_addr, size)`: alloc ONE blank host vidmem gpu_memory_object,
  CPU-map it (cpu_qva), map_dma it into the host GR VAS at `va`, register gpga_page_range(s) for
  [fb_addr, fb_addr+size). Both views = one host object => coherent. Replaces back_and_map's
  split FB-overlay-vs-map. COMMIT.
- R3. SOLVE 0x51: do NOT let the host self-promote its GR ctx (which collides). Either (a) before
  the host constructs the ctx, pre-map OUR objects at the ctx VAs so the host adopts them; or
  (b) intercept PROMOTE_CTX and substitute our gpu_memory_object handles so the host promotes
  OURS. Goal: every GR-ctx VA is backed by an object we own (CPU+GPU). COMMIT.
- R4. Drive R2 from the guest PDB walk (M6.5 enumerator) for BOTH apertures: each leaf {VA, GPGA,
  sys} -> gpga_back. Lazy variant: on a BAR1/PRAMIN miss in the crash path, back on demand. COMMIT.
- R5. Re-test cuCtxCreate. Expect: libcuda's faulting buffer now resolves to real host memory the
  host GPU also sees; rbp=0 should clear or move. VERIFY host nvidia-smi for real work
  ([[mode2-real-forward-not-fake]]). COMMIT.

### Risks / notes
- fb_pages is used by boot/page-table/channel paths; R1 must be a pure pass-through (miss=fb_pages)
  to avoid breaking boot. Gate everything on m2exec.
- The hard core is unchanged by the refactor: connecting the guest buffer to host-GPU-written
  content (R3 0x51 + R4 VA mapping). The refactor makes that connection EXPRESSIBLE (one object,
  both views) instead of impossible (malloc). It does not by itself prove the host GPU fills the
  ctx — that's R5 + the execution/doorbell plane (still the DMA-virt gate).

## REFACTOR perf rules (user, 2026-06-05): trap only rare triggers; hot paths native via memslots

Two refinements that apply across the refactor — the guiding rule is "trap only the rare control/
trigger events; make every hot path native via KVM memslots."

### BAR0 is THREE tiers (not two)
boot/GSP/PMC/control regs can't come from host ioctls (privileged/root-only) -> fully simulated,
agreed. But split BAR0 by access pattern:
  1. CONSTANT regs (chip id, fused caps, invariant read-only config): KVM_MEM_READONLY memslot
     PRE-WRITTEN with the constant values -> reads native (no exit), writes trap/ignored. Kills
     the exit cost of the many constant-register polls during init.
  2. DYNAMIC/logic regs (GFW_BOOT progress, WPR2 state machine, the GSP-RPC doorbell 0x110c00,
     anything needing emulation logic): MMIO-emulated, both directions trap.
  3. USERMODE + PTIMER window: host-mapped RO (real host regs) -> native reads (real PTIMER),
     doorbell writes trap (chid translate -> host doorbell). (See doorbell §15.)
  CAVEAT: classify carefully — some "config" regs change during boot (GFW_BOOT, WPR2); those stay
  in tier 2. Only TRULY invariant data goes in the tier-1 RO-constant memslot.

### PDB tables: never trap per-access; walk live + re-sync on trigger
Trapping every page-table read/write is a hot-path killer. Instead:
  - Back the PDB/FB memory with a real object as a NORMAL RW memslot -> the guest reads/writes its
    page tables NATIVELY, untrapped.
  - Keep NO separate copy. WALK the live RAM-backed tables on-demand (via cpu_qva) only when we
    actually need a resolution: backing a new buffer, or a doorbell/exec. Always reads current
    state; no per-write trap, no staleness.
  - Use the TLB-INVALIDATE as the proactive re-sync hook (the guest MMU invalidate register write
    — rare, we DO trap it) ONLY when we must react to a change: tear down a memslot for an
    unmapped range, or install one for a newly-mapped GPGA. Steady-state PTE r/w is never trapped.

Consistent rule: trap rare triggers (invalidate, doorbell, dynamic regs); hot (PTIMER reads, PDB
r/w, backed GPGA access) is native via memslots.

---

## Execution-path blueprint (the cuCtxCreate keystone) — 2026-06-05

Root cause of the cuCtxCreate hang/crash, pinned by the CRASHWIN read-probe (m2exec=on) and
host-vs-guest gdb (see memory mode2-grctx-privilege-wall, mode2-cuctxcreate-rbp-clobber):
the guest RM busy-loops (~13k iters) walking its GR-VAS page tables down to a **completion
semaphore (guest-FB 0x2efbaf000, reads 0/UNBACKED)** and channel **USERD GP_GET fields (0x..008c,
read 0)**. It is polling a channel completion that NEVER arrives because the HOST GPU never *runs*
the submitted GR-init/scrubber work. The rbp=0 SIGSEGV is the downstream symptom of that stall.

### Key simplifying insight (avoids the golden-ctx privilege fork)
When the HOST channel runs the work it uses the HOST's OWN GR context buffers, which the host RM
already built+self-mapped (st=0x51) at the SAME deterministic GR VAs (0x120020000…) the guest uses.
So **the guest never needs to read GR-ctx-buffer CONTENT** — only the COMPLETION. Therefore we do
NOT need PROMOTE_CTX / GET_CTX_BUFFER_INFO (both privileged, 0x1b) and do NOT need to bridge the
golden context. The entire problem reduces to EXECUTION forwarding, which is fully UNPRIVILEGED.

### What already works
- GPFIFO double-mmap + FIXED map_dma into the host channel VAS: st=0x0 (host does NOT auto-map it).
- OS_DESCRIPTOR pins guest RAM (st=0); GPGA/gpu_memory_object model; GR construct forwards (status=0).
- Host's GR ctx buffers are valid (host built them) at the matching VAs.

### The build (M5.4 steps 2-3 / item-5), unprivileged, in order
1. **Working-set inventory** per channel that must run (the GR-init / golden-image / scrubber chan
   that releases 0x2efbaf000, plus the GR compute channel): GPFIFO, pushbuffers (from GPFIFO
   entries), referenced data buffers, and the completion semaphore surface. Sources: the snooped
   NV_CHANNEL_ALLOC_PARAMS (gpFifoOffset/USERD) + GPFIFO-entry walk (pb VA from e0/e1) + the
   SEM_RELEASE addr parsed from the pushbuffer method stream.
2. **Back + FIXED-map each into the HOST channel's VAS at the guest VA**, backed by the guest's REAL
   bytes (so the host reads real methods and writes the real semaphore the guest polls):
   - guest-RAM (sysmem) buffers: OS_DESCRIPTOR(guest GPA)→host hMem→map_dma FIXED (item-4, proven).
   - guest-FB (vidmem) buffers: host vidmem obj + copy the guest's emulated-FB content in + map_dma
     FIXED; double-mmap the completion-semaphore page so the guest CPU reads the host-written value.
   - VAS reconciliation: all VAs the pushbuffer references must resolve in the host channel VAS;
     where the host already self-mapped (ctx buffers, st=0x51) leave it — those are the host's valid
     buffers and the deterministic VAs already match.
3. **Schedule + ring**: GPFIFO_SCHEDULE the host TSG (done in M5.8), then on the guest doorbell write
   translate vChid→host work-submit token and write the host USERMODE doorbell (m2ring). Naive ring
   today breaks cuInit→999 BECAUSE the working set isn't mapped first — gate the ring on
   "working-set fully mapped for this channel".
4. Host GPU runs the channel → writes 0x2efbaf000 + advances USERD GP_GET → guest poll satisfies →
   cuCtxCreate proceeds. Verify via CRASHWIN: 0x2efbaf000 transitions 0→nonzero (host-written),
   and via HOST nvidia-smi util (real work) — never a green guest log alone (mode2_real_forward_not_fake).

### Quarantine
`nvkvm_chan_execute()` (QEMU parses pushbuffer + writes the semaphore itself) is the FAKING path —
keep it OFF for the real build; it masks whether the host actually ran the work.

Iteration is slow (~6 min boot/attempt + GPU-wedge risk on a bad ring) → this is a sustained focused
build, not an overnight tick. Reload host driver if wedged (rmmod nvidia_uvm nvidia_drm
nvidia_modeset nvidia; modprobe nvidia).

---
## §X cuCtxCreate completion CHAIN — measured 2026-06-05 (HEAD 5691bd0)

After forwarding the GR object alloc (0xc7c0) and faking PROMOTE_CTX (0x2080012b, status=0 to the
fake GSP), the guest CPU-RM enters a sequence of completion waits that the fake-GSP golden-image
flow never satisfies. Located with two new tools (`nvkvm_m2_probe_sem_pdb` dry-run PDB walk;
`m2semval`/`m2sempage` sentinel-inject props, both default-off).

**Poll #1 (CLEARED):** vidmem 0x2efbaf000 (GR-VA 0x40fbaf000 via a linear FB window in chan_vas
pdb=0x2efa6c000; CPU-read via bar2_pdb=0x2f3392000 at BAR2-VA 0xfea000). The pushbuffer faker
releases a *different* semaphore (NVC56F host sema 0x12006c004, sysmem), so 0x2efbaf000 is
engine/kernel-written (golden-ctx status) and never set in our model. Injecting a sentinel (0x1)
satisfies it (331 reads served) and the guest advances.

**Poll #2 (CURRENT):** the CPU-RM repeatedly resolves one VA via bar2_pdb → guest-phys 0x2efa6000
(a GR ctx-buffer page) ~2225× and polls it; the golden-image CONTENT there is blank in the fake-GSP
model. This is the golden-context coherence wall at the *content* level (host's real golden ctx,
built when we forward 0xc7c0 with st=0x51 ALREADY-HOST-MAPPED, lives in the host's privileged ctx
buffers — GET_CTX_BUFFER_INFO / PROMOTE_CTX forward = 0x1b INSUFFICIENT_PERMISSIONS).

**The fork (unchanged shape, now with decisive data):**
- (A) **Sentinel-satisfy chain** — keep injecting per-poll sentinels (unprivileged; matches
  "satisfy guest-kernel checks, host owns real ctx-switch"). Viable iff the chain is short and each
  link is a scalar status; risk = faking past a check whose *content* the guest actually consumes.
- (B) **Real host golden-ctx** — make the guest's ctx reads resolve to the host's real golden image,
  or forward the golden-image/PROMOTE_CTX GSP ops. Blocked by the ctx-buffer privilege wall (needs a
  privileged ctx helper or a CE-copy bridge).

Poll #1 favored (A); poll #2 (content) tilts toward (B). Next cheap experiment: multi-page sentinel
or bar2-read instrumentation to inject poll #2 and measure whether the chain converges or explodes.

### §X.1 Update 2026-06-05 — option C disproven; golden-ctx coherence needs a privileged path (B)
Followed up the fork with source+experiment:
- The golden-image channel is created whenever `IS_GSP_CLIENT` (kernel_graphics.c:478) — it is
  CORRECT GSP-client behavior, not an artifact of a mis-advertised capability. So no clean cap flip
  (C) suppresses it. The one plausible lever — regkey `RMSetClientRMAllocatedCtxBuffer=0` — changed
  ctx-buffer management (PROMOTE_CTX stopped being issued) but the golden-image poll/wall persisted.
- Ringing the host doorbell for real (A-real) does NOT write the guest's status/ctx pages: the GR
  engine (FECS) writes the golden image + status into RM-INTERNAL PRIVILEGED ctx buffers (the
  st=0x51 host-self-mapped set), not via a guest-visible pushbuffer SEM_RELEASE. The host writes its
  own buffers; the guest's copies (0x2efbaf000 / 0x2efa6xxx) remain blank.
- Sentinel faking (A-fake) clears the scalar status (poll #1) but poll #2 consumes ctx CONTENT.

Conclusion: cuCtxCreate's keystone is GR golden-context COHERENCE, which requires a privileged
mechanism (Option B) — mirror the host's real golden ctx into the guest's copies, or map the guest's
ctx buffers onto the host's via a privileged path. Both weaken the unprivileged-stub tenet → user
decision. Candidate mitigation: B-OFFLINE — extract the deterministic golden image once via a
privileged/kernel path and replay the bytes at runtime (runtime stays unprivileged; the PATCH buffer
fixes context-specific fields). PARKED for user sign-off.

---

## Addendum 2026-06-12 — host-only completion attempt exposed the CE-emulation regression + the translating-scheduler refinement

This session pushed `cuCtxCreate -> cuMemAlloc -> CE round-trip` toward **provably host-only**
completion (suppress the simulated completion forge so the host GPU must write it) and, in doing
so, surfaced exactly the "approach B" smell this doc warns against, plus the correct fix shape.

### What we found (evidence in agent memory mode2_execfwd_layer2.md, M5.49b)

- The CE round-trip's byte-exact result (`rv=0xabcd1234`) is currently produced by **QEMU
  EMULATING the CE copies in software** (`M5: CE MEMSET/COPY` parser writing bytes to guest-FB
  phys), not by the host CE engine. The host runs the *channels* (schedule/USERD/doorbell/sema
  semantics — proven) but the data movement of small CE ops is QEMU's. This is the rejected
  interpret/replay path that crept in as bring-up scaffolding. It must go.
- It is not merely a shortcut — it is **incorrect**: the emulated CE executor replays entries and
  runs each copy twice (`(phys)` then `(virt)`), ignoring guest submission order and the
  inter-channel semaphores. A guest-ordered page-table *scrub* (`CE MEMSET const=0 out=PT-pool`)
  thus lands AFTER the guest's rebuild, **wiping the live page tables in QEMU's FB mirror**. Since
  the host VAS is built by mirroring that PDB (populate_cvas), the mirror going stale -> host VAS
  missing the user buffer -> host engine FAULT_PDE once it actually has to complete. (Root-caused
  by a kretprobe-free log trace: stale one-shot root snapshot + out-of-order CE PT writes.)

### Sync model (so the fix is precise)

GPU semaphores are **not mutexes** — they are monotonic payload values in coherent memory (a
fence). Acquire = "block until value >= N"; release = "write value = N". The CPU analogue is a
**futex/condvar**, not a mutex (poll fast-path + sleep/wake). Baseline notification is **polling**
(the `uvm_spin_loop`); the wakeups exist only to allow sleeping: **GPU->CPU = interrupt** (release
raises an IRQ -> driver ISR wakes the thread, which re-reads the value); **CPU->GPU = doorbell**
(CPU writes the value then rings the channel doorbell so the host re-evaluates a stalled acquire).
Asymmetric because the CPU takes interrupts while the GPU host is poked via MMIO. A correct forward
must carry BOTH the value (WB sema fwd-map so the host write is guest-visible) AND the wakeups
(host IRQ -> inject into emulated device for guest ISR; guest doorbell -> forward to host USERMODE).
Today the guest *spins*, so value-landing suffices; IRQ injection is the deferred sleep-path piece.

### The architecture to build (translating scheduler; reaffirms "forward, don't emulate")

Security invariant: **never expose host phys (system or GPU) to the guest** — QEMU is unprivileged;
the guest stays entirely in its virtualized address space, and QEMU/stub translate to host backings
via ioctls only. So "make guest-phys == host-phys" is OUT; virtualized translation stays.

Split by what a command's operands carry:
- **VA-operand commands (data/compute copies, kernels):** the operands are GPU VAs the host MMU
  resolves once the VAS is resident. **Forward to the host channel's ring; let the hardware execute
  and honor the semaphores natively.** No per-command interception, just residency. Delete the
  QEMU CE-copy emulation for these.
- **Phys-operand commands (page-table writes/scrubs — the UVM/PT channels):** their payload is
  guest-phys PTE values, which cannot be handed to hardware. **Intercept, translate guest-phys ->
  host backing, apply into the host VAS via RM map ioctls — strictly once, in guest submission
  order, honoring the gating semaphores.** Re-resolve the live PDB from the channel instance block
  (RAMIN PDB_LO/HI), not a one-shot snapshot; treat "PDB had N>0 runs, now 0" as a stale-root event.

Serialization is **selective, not lock-step**: free-flow forwarding everywhere, block only at the
cross-channel sync points where a forwarded command acquires a semaphore a PT channel releases
(ensure the PT mutation is applied to the host VAS before submitting the dependent work). The
durable end-state collapses the dual backing (one buffer / one real host-vidmem backing, #128) and
forwards the whole stream; QEMU's only retained job is the page-table plane, and even that is
ioctls, never raw phys to the guest.

### Fix plan (ranked) and residual risk

1. (do first) Order-correct PT interception + live-PDB re-resolution -> cup2 passes host-only
   (proves the host CE genuinely completing). 2. Collapse the dual backing so data buffers are
   single-backed real host vidmem the host CE operates on. 3. Clean plane split + cross-channel
   sync-point serialization. Deferred: sleep-path host->guest IRQ injection; serialization perf.
   Named existential risk (low now — already through CE round-trip): whether the stock KMD attests
   real silicon (fused IDs/signatures) somewhere downstream.

M5.49b scaffolding (commits c5dae00/f3ef26b) is inert at default (`m2hostsem=off` == M5.48 pass):
per-client host-only gate, CE-copy-client capture (FRESH-VAS), residency sweep extended to CE-copy
clients. Harnesses: scripts/mode2_diag/m549c_hostonly_complete_host.sh + cup2_run_guest.sh.

## Addendum 2026-06-12b — the "stale page-table mirror" root cause was REFUTED; real cause = emulated data plane (no executor-order fix helps)

A careful read-only re-trace of the surviving host-only capture (`/tmp/m549c_delta.txt`, 352 770
lines; Fable agent a383eb2245c5fd356) **overturns** the Addendum-2026-06-12 claim that an
out-of-order emulated CE memset wiped a live page tree. Evidence (absolute log lines):

- cup2 hangs at the **first** host-only-forced copy — `cuMemcpyHtoD(dp,&hv,4)` is the last guest
  line; dp = `0x72e2a2200000`. The matching host fault is `Xid 31 GRAPHICS FE FAULT_PDE @
  0x72e2_a2200000` — **no PDE for dp in the host engine VAS**.
- dp's only host backing is a **blank gpga shadow** (line 252289 `M7 R2 gpga_obj
  va=0x72e2a2000000 gpga=0xa000000`), i.e. QEMU vidmem the CPU fills via the emulated CE parser —
  NOT a real host-resident buffer the host CE/GR can resolve.
- The 2 MiB `CE MEMSET out=0x3400000 const=0` (line 266809) is a **single, genuine, in-order**
  guest pool-scrub on the scrubber channel (monotonic gp_get 24→25), firing ~6 000 lines AFTER
  pdb 0x3401000 was **already** empty (first `runs=0` at 261653). The tree went empty because the
  **guest itself tore it down in order** (root-PDE clear at 260629/260630, entry[131], monotonic
  gp 131→132) — correct guest state, not corruption. The "wipe a live tree" story conflated this
  late teardown/scrub with the early HtoD hang.
- The executor bugs are real but **non-causal** here: six stale re-walks (USERD read back as
  `gp_put=0 < gp_get=1` during channel teardown — lines 260713/262021/263215/264609/265194/265907)
  re-executed 1443 already-consumed CE ops, and 772 phys→virt double executions occurred; both
  self-cancel in this run. Worth fixing only as hygiene (skip any channel whose USERD reads
  gp_put<gp_get), NOT as the cup2 fix.

**Conclusion:** "Order-correct PT interception + live-PDB re-resolution" (the ranked-#1 milestone in
Addendum 2026-06-12) is built on a refuted mechanism and will NOT make cup2 pass host-only. Re-resolving
a PDB pointer or reordering the emulated CE cannot make an **emulated-only** buffer host-resident. The
binding blocker is the one the CRITICAL FINDING already named: the CE data plane is QEMU-emulated, so dp
has no real host backing and the host engine FAULT_PDEs the instant we stop forging completion.

**Therefore the next step is unambiguously the real data plane (no longer optional):** give dp (and the
CE working set) a **real host-vidmem backing** mapped into the host engine's running VAS at dp's guest
VA, and **forward the LAUNCH_DMA copy to the host CE engine** instead of QEMU's `M5: CE COPY/MEMSET`
software emulation. This is identical to the work the matmul GR-kernel north-star forces (a real shader
cannot be emulated). The earlier ranked plan collapses to: **(1) real CE data plane on the cup2 testbed
(smallest debuggable forcing function) → (2) reuse it for the matmul GR kernel.** Both retire the QEMU
CE-emulation shortcut; neither needs the order-correct micro-fix.

## Addendum 2026-06-12c — CE-copy host-only is a separate-client bare-channel detour; PIVOT to matmul (north star)

Took the "real CE data plane on cup2" path one hardware iteration and learned the decisive shape:

- The cup2 HtoD/DtoH runs under a **separate RM client** (0xc1e00007) with its **own** device
  (0x0080→hObj 0xa), subdevice (0xb), VAS (0x90f1→hObj 0xc), and a **bare channel** (0xc56f,
  hObj 0x2) parented directly to the *device* (not a TSG), with `hVASpace@28=0`. On the forwarded
  host device that has no default VAS → channel alloc `0x33 NV_ERR_INVALID_OBJECT_HANDLE`. The
  existing code (lines ~4213) deliberately refuses to substitute hVASpace because it assumed "ALL
  these channels are TSG channels" — true for the GR/COPY channels, FALSE for this bare channel.
- M5.50 experiment (gated m2hostsem): give the bare channel a fresh nvkvm cvas + substitute
  hVASpace. Result on HW: alloc moved `0x33 → 0x1f NV_ERR_INVALID_ARGUMENT` (progress, but still
  fails — the rest of the bare-channel param set / USERD / ctxshare / error-notifier / non-TSG
  schedule path all need forwarding), AND it **regressed** the M5.49b host-only proof: the cvas
  intercepted the grmapper routing so the FRESH-VAS fallback that *identifies* the USER-CE client
  never fired → suppression didn't engage → cup2 passed via **simulation** (CE_SEM_RELEASE sim
  writes), a false green for host-only. **Reverted.**
- Conclusion: making the CE-copy client genuinely host-execute = forward an entire **second client's
  bare-channel stack** (device/subdev/VAS/USERD/ctxshare/notifier) + a **non-TSG schedule+ring**
  path + operand residency. Sizable, and **orthogonal** to the GR compute path matmul needs.

**Why matmul is the better forcing function (and now the active path):** the GR/compute channels
(client 0xc1d00003, parented to TSGs) **already construct on the host** (`status=0x0 OK`) and
already executed in M5.48 (cuCtxCreate + CE round-trip, util>0, zero Xid). A real **matmul GR
kernel** cannot be QEMU-emulated (it is a real shader), so a numerically-correct matmul result is
**un-forgeable** proof of genuine host GR execution — the same first-compute proof we wanted from
host-only cup2, via the actual north-star path. The CE data transfers stay QEMU-emulated for now
(byte-exact; #128 collapses the dual backing later). **Active task: matmul kernel launch** — extend
the pushbuffer/QMD handling for the GR compute launch, drive operand (A/B/C matrices + kernel
code/const/param) residency into the GR channel's cvas, verify a correct result on hardware.

## Addendum 2026-06-13 — ★ Mode-2 GR kernel launch PASSES (first genuine host GR compute) ★

`cup3` (cuCtxCreate → cuModuleLoadData(PTX) → cuLaunchKernel `out=in*3+1` → cuCtxSynchronize →
cuMemcpyDtoH) returns **`rv=43 want=43 PASS`** on hardware (RTX 3060 / 580.159.04), rc=0, zero new
Xid. Un-forgeable: QEMU never parses the compute pushbuffer (forwarded raw) and the CE software path
can only copy/memset — only the real host GR SASS engine produces 43 from 14. The forward-and-execute
bridge (host fetches the guest GP_PUT, runs the real shader, writes the completion semaphore the guest
polls) works end-to-end. Two fixes unblocked it (commit 196822a): the **M5.51 va_seen poisoning fix**
(mark-on-backing-success, ending the recurring `backed=0` / coverage-ends-at-the-buffer fault that hit
cup2-`dp` and matmul-`d_out`), and **scoping the residency sweep to the GR client in default mode**
(the CE-copy clients were redundantly re-backing GR-owned chunks, exhausting the host GPA arena until
`RM_MAP_MEMORY` returned `NO_MEMORY` and the kernel's `d_in`/`d_out` chunk couldn't map). Next: a real
NxN fp32 matmul (util>0, larger working set) for a heavier proof, then performance.

### Correction 2026-06-13 — the kernel-launch PASS is REAL but FLAKY (not yet reproducible)
A confirmation run of cup3 (same binary, fresh boot) HUNG with `Xid 31 CE2 FAULT_PTE @ 0x7a20d7600000`.
So the north-star PASS is genuine (feasibility proven, un-forgeable) but **1/2 flaky** — not production
solid. Root cause is **incomplete/racy residency** (not arena exhaustion this run): the bounded reactive
sweep left a ~0xae000 GAP in the working set (`back_sys` ended at 0x7a20d7600000; next run started at
0x7a20d76ae000), and the host CE faulted in the gap. The fix for reproducibility is **deterministic,
complete residency before any host ring** (completeness loop: re-sweep until a full pass backs zero new
leaves and referenced runs have no gaps, then ring), plus understanding why the sweep drops sub-run gaps.
Verify with N consecutive all-PASS cup3 runs before claiming solid.

### Update 2026-06-13 — reproducible (3/3) after the sysmem poisoning fix
With M5.51b (commit b0a8314) extending the mark-on-backing-success fix to the sysmem run path, cup3
passes 3/3 consecutive fresh-boot runs (rv=43, rc=0, zero new Xid). The flakiness was the sysmem-run
`va_seen` poisoning leaving a residency gap the host CE faulted in; fixing both the vid (M5.51) and sys
(M5.51b) backing paths makes the working set reliably complete before the host runs. The Mode-2 GR
kernel launch is now reproducible genuine host GR compute. Next: a real NxN fp32 matmul.
