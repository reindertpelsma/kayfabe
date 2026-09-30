# V3_UVM_GUEST_FAULT_PLANE — the guest side of UVM demand paging on the b3 host proof

**STATUS: LIVE — BUILT AND MEASURED (M1–M2), 2026-09-30 — branch `v3-uvm-guest` (from master
`a295e2ce`).** ★ M2 reached at `c849f68d`: guest `cudaMallocManaged` buffers, GPU-first and
CPU-initialised, complete with correct data in a kf3 guest (`traces/v3_uvm_guest/`). ⚠ It depends on
§3.8a, an experiment that reads guest page-table words on the CPU beside the GPU walker — a
constraint change awaiting the owner (ruling A.1/A.11). Lines not marked `[meas]` are design. This is the kf3 side of b3: the stock guest
nvidia-uvm must see its own replayable faults, and its replay and cancel must become scoped host
EFS actions. The host half (the opt-in nvidia-uvm patch) is built and proven host-only
(`V3_UVM_B3_IMPLEMENTATION.md` §0, `traces/v3_uvm_b3/`). The specification this note implements is
`V3_UVM_DEMAND_PAGING.md` §5, "What the guest must see". No guest managed-memory claim is made here;
the first one is milestone M2 (§9).

## Stop note — where the work stands (update this first, every time)

- **2026-09-30 20:05 UTC — M2 reached, M3 next.** Branch head carries M1a–M1f (packet codec,
  FaultRing, rewriter split, EFS session incl. `UVM_MM_INITIALIZE`, the kf-qemu plane) and the §3.8a
  experiment. `[meas]` `uvmg7` at `c849f68d`: `um_probe gpufirst cpuinit prefetch advise malloc`
  (twice) all `ok bad=0`, 3 499 faults delivered and replayed, zero cancels, zero Xid. The finding
  that shaped it (§3.8a): **a parked replayable fault holds the host GPU's GR engine**, so fault
  servicing must be GR-free; the walk kernel deadlocked it (`uvmg4`). Next: M3 (the four apps), then
  E-S1 and the merge bar with EFS off and on. Box `53564695` (`vuvm`) is up with the patched module
  loaded.
- **2026-09-30 18:10 UTC.** Design written from source (ogkm 580.159.04 `b81d58e`, this tree at
  `a295e2ce`). Box `53564695` (RTX 3060, alias `vuvm`, nested Vast KVM VM) rented and provisioning
  (`provision_full.sh`). Next: M1a–M1e GPU-free code (§9), then the EFS-mode twin on hardware (E-T1).
- To resume from the repo alone: read §0, then §9's milestone table; the first unchecked row is the
  next step. Box state and ids are in the last line of this note's §10.

## §0 The answer in one screen

1. **Two halves, both needed for M2.** The *guest-visible fault plane* (packets, `PUT`/`GET`, the
   interrupt level, replay/cancel) is necessary but not sufficient: a host twin raises a
   **replayable** fault only when its VA space is **fault-capable**, and a fault-capable VA space
   must be **externally owned** (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/vaspace_api.c:678-690`),
   so RM builds no page tables in it and refuses `NV_ESC_RM_MAP_MEMORY_DMA` into it (`[meas]` E6′,
   `0x33`, `V3_UVM_DEMAND_PAGING.md` §12.3.1 (5)). ⇒ In EFS mode the twin VAS of a fault-capable
   guest VAS becomes **UVM-owned** (registered on an EFS UVM file) and the publish executor for that
   one space becomes **UVM's external-mapping ioctls**. This is the "kf3 publish-executor swap" that
   `V3_UVM_DEMAND_PAGING.md` §4.4 and §5 name; it is scoped to fault-capable guest VA spaces only.
2. **The selector is the guest's own statement.** A guest VA space allocated with
   `NV_VASPACE_ALLOCATION_FLAGS_ENABLE_PAGE_FAULTING | IS_EXTERNALLY_OWNED` is a guest-UVM space
   (what libcuda allocates); only its mirror becomes EFS-mode. Every other space (guest kernel
   spaces, RM-owned graphics spaces) keeps today's RM-owned mirror, windows and rings, unchanged.
3. **Default off, two keys, no behaviour change otherwise.** `KF3_UVM_EFS=1` in the VMM's
   environment asks for it; realize then probes the host (`UVM_INITIALIZE` with the EFS flag,
   `UVM_EFS_QUERY`). A stock host nvidia-uvm, or the patched one loaded with `uvm_efs_enable=0`,
   is refused **by name** and the device runs exactly today's code paths (§6).
4. **Attribution is by VA space, not by channel.** EFS v1 records carry no channel identity
   (by design: no instance pointer, no PDB). One EFS file per EFS-mode twin VAS makes the VA space
   exact; the packet names a *representative* guest GR channel of that guest VA space (its guest
   instance block and its guest VEID). Stock guest UVM needs nothing more to route the fault to the
   right `gpu_va_space` (§3.5).
5. **Replay/cancel are authored, never forwarded.** The Translated rewriter already drops every
   `MEM_OP` TLB invalidate (a privileged host method) at a split; it now also decodes the `REPLAY`
   field and emits a `Piece::Fault` after the split. At that piece the worker resolves this VM's
   own delivered EFS records: `START`/`START_ACK_ALL` → `UVM_EFS_RESOLVE(REPLAY)`, `CANCEL_*` →
   `UVM_EFS_RESOLVE(CANCEL)` of the records the cancel names. No guest byte reaches the host GPU as
   a method; no record of another VM is reachable (they live in other processes' files).

## §1 What the stock guest does — the protocol, from source

All `[src]`, ogkm 580.159.04 (`kernel-open/nvidia-uvm/…` unless another tree is named).

| step | guest | source |
|---|---|---|
| buffer | CPU-RM allocates the replayable buffer in **guest sysmem** (CC off), size = GMMU static info `replayableFaultBufferSize`, 4 KiB pages, and sends the page list in `0x20800a9b` | `src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c` `kgmmuFaultBufferReplayableAllocate_IMPL`, `kgmmuFaultBufferGetAddressSpace_IMPL` |
| registers | `pFaultBufferGet/Put` = VF-PRIV `MMU_FAULT_BUFFER_GET/PUT(1)`; `pHubIntr`/`EnSet`/`EnClear` = `CPU_INTR_LEAF*(leaf of the REPLAYABLE_FAULT vector)`; `pPrefetchCtrl` = `MMU_PAGE_FAULT_CTRL` | `kern_gmmu_tu102.c:188-231` (`kgmmuGetFaultRegisterMappings_TU102`; GH100 uses it too with CC off) |
| capacity | `max_faults = bufferSize / 32`; `GET`/`PUT` `PTR` is 19:0 | `uvm_gpu_replayable_faults.c`, `dev_vm.h` |
| top half | pending ⇔ `cached_get != cached_put`, or the `VALID` bit of the entry at `cached_get`, or `PUT` read from BAR0 ≠ `GET`; then `EN_CLEAR(mask)` + `LEAF(mask)` W1C | `uvm_gpu_isr.c:90-117`, `uvm_turing_fault_buffer.c` (`disable_replayable_faults`) |
| fetch | for each index in `[GET, PUT)`: **spin until `VALID`**, parse, clear `VALID`; then write `GET` | `uvm_gpu_replayable_faults.c` `fetch_fault_buffer_entries`, `uvm_volta_fault_buffer.c` `parse_replayable_entry` |
| packet | `clc369` 32 bytes: `INST_LO/HI` + `INST_APERTURE`, `ADDR` (4 KiB page), `TIMESTAMP`, `ENGINE_ID`, `FAULT_TYPE`, `REPLAYABLE_FAULT=1`, `CLIENT`, `ACCESS_TYPE`, `MMU_CLIENT_TYPE`, `GPC_ID`, `REPLAYABLE_FAULT_EN=1`, `VALID` | `clc369.h:34-67`; hwref rows `class clc369.h NVC369_BUF_ENTRY_*` |
| attribution | `instance_ptr → user channel` (radix tree of **registered** channels); in a subcontext TSG, `ve_id = ENGINE_ID − NV_PFAULT_MMU_ENG_ID_GRAPHICS` selects the TSG's subctx → `gpu_va_space` | `uvm_gpu.c:3534-3630`, `uvm_volta_fault_buffer.c:get_ve_id`, `uvm_hopper_fault_buffer.c:31-43` |
| service | allocate/migrate, write PTEs with its CE, **TLB invalidate** (`MEM_OP`, the va_space's PDB) | `uvm_va_block.c`, `uvm_tlb_batch_end` |
| replay | `MEM_OP_A..D`: `TLB_INVALIDATE_TARGETED`, `PDB=ONE` at address 0 (dummy), `REPLAY=START` or `START_ACK_ALL`, pushed on a MEMOPS channel that **acquires** the service tracker first | `uvm_volta_host.c:234-264` (`uvm_hal_volta_replay_faults`), `push_replay_on_gpu` |
| cancel (Volta+) | `MEM_OP` `REPLAY=CANCEL_VA_GLOBAL`, `PDB=ONE` = the va_space's PDB, `TARGET_ADDR` = the page, `ACCESS_TYPE`, `CANCEL_MMU_ENGINE_ID` | `uvm_volta_host.c:66-114`; Hopper `uvm_hal_hopper_cancel_faults_va` |
| cancel (error paths) | `GP100_UVM_SW` (`C076`) `FAULT_CANCEL_A/B/C` = instance pointer + `MODE` `GLOBAL`/`TARGETED` (+ GPC/client) | `uvm_pascal_host.c:300-363` (inherited by every later HAL), used by `cancel_faults_all` and the torn-subctx path |
| resume (non-replayable) | Ampere+: `C076 CLEAR_FAULTED_A/B`; Volta/Turing: the host `CLEAR_FAULTED` method with the channel's token | `uvm_ampere_host.c:190-210`, `uvm_volta_host.c:116+` |
| re-arm | end of bottom half: `EN_SET(mask)`, `LEAF(mask)` W1C, then **write `GET`** "to force the re-evaluation of the interrupt condition" | `uvm_gpu_isr.c:780-820`, `uvm_turing_fault_buffer.c` (`clear_replayable_faults`) |

⇒ The emulated hardware contract is: an entry is fully written before its `VALID` bit; `PUT` moves
only past `VALID` entries; the replayable leaf bit is (re)asserted whenever the condition
`GET != PUT` holds at an **evaluation point** — a `PUT` advance or a `GET` write. That is the level
`V3_UVM_DEMAND_PAGING.md` §5 row 3 asks for, never an edge.

## §2 The host half as built — and what it gives the guest side

EFS v1 (`tools/uvm_efs/patch/uvm_efs_ioctl.h`, `uvm_efs.c`), measured host-only on 2026-09-30
(`traces/v3_uvm_b3/run_full.log`, `clean_reruns.txt`, repo `c94601d9`):

- **One UVM file = one UVM va_space = at most one GPU VA space per GPU.** Opt-in at
  `UVM_INITIALIZE` (`UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE` + `DISABLE_HMM` or
  `MULTI_PROCESS_SHARING_MODE`), module parameter `uvm_efs_enable=1`.
- **Records** (`UvmEfsFaultRecord`): kernel-issued opaque id, page-aligned GPU VA, GPU timestamp,
  divert time, GPU UUID, `accessType` / `accessTypeMask` / `faultType` / `clientType` as **nvidia-uvm
  internal enums** (`uvm_hal_types.h:245-355`), raw `clientId`, `gpcId`, `utlbId`, host `veId`, raw
  `mmuEngineId`, `numInstances`. **No channel identity**, by design.
- **Dedupe per (GPU, page)** while parked; parked ≤ `uvm_efs_max_records` (1024); a kernel timeout
  (`uvm_efs_timeout_ms`, default 10 s) cancels a record nobody resolved; teardown cancels every
  parked record in hardware.
- **`UVM_EFS_WAIT` blocks** (bounded, ≤ 1 s); the file has **no `poll`**. `UVM_EFS_RESOLVE(REPLAY)`
  issues one hardware replay per GPU per call; `RESOLVE(CANCEL)` is `cancel_fault_precise_va` with
  the kernel's saved packet and the va_space's own PDB, `UVM_FAULT_CANCEL_VA_MODE_ALL`.
- **Only the initializing thread group** may WAIT/RESOLVE — every thread of the VMM qualifies.
- **Replayable only.** Non-replayable (CE, host) faults stay stock in v1.

Consequences here, each owned by a row in §3:
- no channel identity ⇒ attribution by EFS file = by VA space (§3.5);
- internal enums ⇒ a pinned inverse map to the packet's hardware values, per family (§3.4);
- blocking WAIT without `poll` ⇒ one waiter thread per EFS file (§4); a `poll` in a v2 ABI would
  fold them into one epoll thread (§10 Q3);
- cancel is always `MODE_ALL` ⇒ a guest `WRITE_AND_ATOMIC` cancel is widened to all accesses of that
  page (stricter, never looser: the context faults either way) (§3.7);
- replayable only ⇒ the `CLEAR_FAULTED` + shadow-buffer rows need a host v2 first (§3.10).

## §3 The design, row by row — `V3_UVM_DEMAND_PAGING.md` §5 → crate, thread, lock

Naming: **vCPU** = a guest vCPU inside a BAR0 write exit (lock-free, allocation-free); **drainer** =
`kf3-drainer` (GSP FSM, holds the GSP lock); **chan-act** = `kf3-chan-act` (host channel verbs, in
statement order); **vamgr** = `kf3-vamgr` (the VA manager: walker, diff apply, mirrors); **worker** =
`kf3-worker*` (Translated channels); **efs-wait** = new, one per EFS file (§4).

### 3.0 The EFS-mode twin — the prerequisite (new; host side of kf3)

| piece | what | crate / thread |
|---|---|---|
| probe | realize: `KF3_UVM_EFS=1` ⇒ open `/dev/nvidia-uvm`, `UVM_INITIALIZE(EFS \| DISABLE_HMM)`, `UVM_EFS_QUERY`: accept iff `abiVersion == 1 && moduleEnabled && active`; else print `kf3: UVM EFS REFUSED: <reason>` and leave the fault plane off | `kf-host::efs` verbs over `kf-linux-raw::uvm`; realize thread |
| selector | the guest's `FERMI_VASPACE_A` alloc flags are recorded at alloc (`PageDirPolicy::observe_alloc` already sees every VA-space alloc) and carried on `MemStatement::PageDir` as `fault_capable` | `kf-rm::barpde`; drainer |
| create | `create_mirror` for a `fault_capable` key with the plane on: `FERMI_VASPACE_A` with `ENABLE_PAGE_FAULTING \| IS_EXTERNALLY_OWNED` (no `NV01_MEMORY_VIRTUAL`, no reservations, **no store/RAM windows**); open an EFS UVM file; `UVM_REGISTER_GPU`; `UVM_REGISTER_GPU_VASPACE(kf's ctl fd, kf's client, the VAS)`; two `UVM_CREATE_EXTERNAL_RANGE`s covering the guest-usable VA except the **channel window**; start its efs-wait thread | `kf-host::efs`, `kf-qemu::mem`; vamgr |
| channel window | `[HOST_HOLE_LO, 1 TiB − 4 GiB)` (the hole host RM uses today for its own placements; GR's global context pointers are `VA >> 8` in 32 bits, so it must stay below 1 TiB — `kf-host/src/channel.rs:80-98`) | constant beside `GUEST_VA_RANGES` |
| executor | a UVM `MapTarget` (`kf-mem/src/ledger.rs:160-274`): `map` = `UVM_MAP_EXTERNAL_ALLOCATION(base=va, offset, hMemory = the store or the guest-RAM OS descriptor)` with `gpuMappingType` from the leaf's permissions (RO → `ReadOnly`, atomic-disable → `ReadWrite`, else `ReadWriteAtomic`) and `gpuCachingType` (volatile → `ForceUncached`); `unmap(va)` = `UVM_UNMAP_EXTERNAL(va, placed length)`; `invalidate` = no-op (UVM invalidates and waits per call); kinds other than the backing object's default are refused by name (graphics kinds are `§4.4`'s open question, out of scope) | `kf-mem` (target), `kf-host::efs` (verbs); vamgr |
| twin channels | births unchanged (`birth_group`/`birth_member` name the VAS); then `UVM_REGISTER_CHANNEL(kf's client, hChannel, base/length = the channel window)` **after the first engine object and before the first schedule**; `UVM_UNREGISTER_CHANNEL` before `free_member` | `kf-qemu::chan` acts; chan-act |
| teardown | retire with no live twin: stop the waiter, `UVM_FREE` both ranges, `UVM_UNREGISTER_GPU_VASPACE`, close the file (EFS shutdown cancels parked faults in hardware — `[meas]` `dmesg_efs_shutdown.txt`), free the RM VAS. Retire with live twins: keep, and tear down when the last twin goes (today's "KEPT, never recycled" branch, `mem.rs:1840-1848`, gains the missing second half for EFS mirrors). EFS mirrors are never recycled as spares | vamgr, chan-act |
| permissions | with the plane on, `ATOMIC_DISABLE` is carried (`PermPolicy::carry_atomic_disable`): a GPU atomic on an atomic-disabled page now becomes a delivered fault the guest services (the reason it was withheld, `V3_UVM_DEMAND_PAGING.md` §6) | `kf-qemu::device` realize |

⊘ **Why not keep RM mapping and add a fault source?** A replayable fault needs `FAULT_REPLAY_TEX/GCC`
in the twin's instance block, which RM sets only for a fault-capable VAS (`kern_gmmu_gv100.c:373-377`,
§12.3.1 (1)); a fault-capable VAS must be externally owned; RM maps nothing into it. There is no
fourth configuration.

### 3.1 Row 1a — where the buffer is (`0x20800a9b`)

- Today: answered `NV_OK` and recorded (`FaultBufferRecorder`, a `CommandObserver` on the drainer).
- Now: the recorder also hands the registration to the fault plane (`FaultPlane::register`), which
  validates it **before** anything is written: `size ≤ 256 pages` (`exceeds_vendor_bound`), `size`
  a multiple of 32, every page 4 KiB-aligned and resolving to **guest RAM** through the device's
  `RamMap` (never a kf memslot: shadow, doorbell bitmap, boot pages). A registration that fails any
  check is refused **by name** in the boot report and the plane stays unregistered (records then
  cancel, §3.6) — the `NV_OK` to the guest is unchanged (pure `[IN]`, nothing fabricated,
  `kf-abi/src/faultbuffer.rs:71-111`).
- A second registration replaces the first (guest UVM reload): `GET = PUT = 0`, undelivered records
  are cancelled, delivered ones are forgotten (their host records are cancelled at the next replay
  or by the kernel timeout).
- Writes use the existing bounds-checked page-list writer `kf_gsp::RegionMap::from_pages(4096,
  pages)` over the device's `Ram` (`device.rs:2325-2354`, `gsp-ram.rs:163-266`).

### 3.2 Row 1b — `GET`/`PUT` over BAR0

- Offsets, per family, from the derived table: `NV_VIRTUAL_FUNCTION_PRIV_MMU_FAULT_BUFFER_GET(1)` =
  PRIV + `0x3028`, `PUT(1)` = PRIV + `0x302C`, `PTR` 19:0, `GETPTR_CORRUPTED` 30, `OVERFLOW` 31
  (`kf-chip/data/hwref-580.159.04.tsv`: `turing/tu102` rows, inherited by GA10x/AD10x/GH100;
  `blackwell/gb100` rows, inherited by GB20x). PRIV = usermode window − `0x30000`, as for the
  interrupt tree and the invalidate registers. Held to hwref by a GPU-free test, like
  `kf-trap/tests/hw_boundary_vs_ogkm.rs`.
- State: `kf_trap::faultring::FaultRing` — `get`, `put`, `entries` (0 = unregistered), all atomics.
  **vCPU arm** in `Device::bar0_write_inner`, beside the interrupt-tree arm, only when the plane is
  on: `GET(1)` ⇒ store `val & PTR` (the clear bits 30/31 are write-1-to-clear of flags we never
  set), publish the shadow word, and **evaluate** (§3.3); return — never the privileged ring.
  `PUT(1)` writes are ignored (`R--4A`). Every other fault-buffer register keeps today's plain
  shadow + ring behaviour (`MMU_PAGE_FAULT_CTRL` is the prefetch toggle: recorded, row §3.11).
- `PUT` belongs to the delivery path (§3.6): packet bytes → `fence(Release)` → `VALID` →
  `fence(Release)` → `put.store(SeqCst)` → shadow store → evaluate.
- A hostile `GET` (≥ `entries`) is clamped to "no space" for delivery purposes and reads back as
  written: it can only stall its own guest's fault delivery (self-harm), bounded by the host timeout.

### 3.3 Row 2 — the interrupt, a level derived from `GET != PUT`

- Vector: the served interrupt table's `REPLAYABLE_FAULT` row (`MC_ENGINE_IDX` 59), which is the
  **host die's own** vector (`MC_GET_STATIC_INTR_TABLE`, `kf-rm/src/hostfacts.rs:771-889`;
  64 on GA106). Per die, never a constant here.
- Evaluation points: the vCPU `GET` write and the delivery thread's `PUT` publication. Each does
  `store(own); fence(SeqCst); load(other)`; if `get != put` ⇒ `CpuIntr::latch(vector)` +
  `shadow_all` + `deliver` (MSI-X vector 0, the one this device signals). Dekker ordering: at least
  one side sees the final state, so a non-empty buffer always leaves the leaf latched; a double
  latch is a harmless duplicate (§ `the_interrupt_arming_model.md`: over-report, never under-report).
- The guest's own `EN_SET` of an already-pending leaf already raises (`cpuintr.rs:214-222`), which
  is exactly the re-arm sequence of §1.

### 3.4 Row 1c — the 32-byte packet, per family

- `kf_abi::faultpacket::encode(&FaultPacket, &FamilyFaultConsts) -> [u8; 32]`: fields and positions
  from the `clc369.h` rows of the hwref table (identical for every family that has the class).
- Values: `INST` = the guest's instance block (§3.5), `ADDR` = the record's page, `TIMESTAMP` = the
  record's GPU timestamp (the same timer the guest reads — `HostRm::gpu_time_ns`, `§53.1`),
  `ENGINE_ID` = `NV_PFAULT_MMU_ENG_ID_GRAPHICS(family) + guest VEID` for GPC clients (64 on
  Volta…Ada, 384 on Hopper/Blackwell — hwref `dev_fault.h`), the record's raw `mmuEngineId` for HUB
  clients; `CLIENT`, `GPC_ID` raw; `REPLAYABLE_FAULT = 1`, `REPLAYABLE_FAULT_EN = 1`, `PROTECTED_MODE
  = 0`, `MMU_CLIENT_TYPE` from the record; `ACCESS_TYPE` / `FAULT_TYPE` = the **inverse** of UVM's
  parse (`uvm_volta_fault_buffer.c:get_fault_access_type`, `uvm_hal_volta_fault_buffer_get_fault_type`):
  `READ→VIRT_READ, WRITE→VIRT_WRITE, ATOMIC_STRONG→VIRT_ATOMIC_STRONG, ATOMIC_WEAK→VIRT_ATOMIC_WEAK,
  PREFETCH→VIRT_PREFETCH`; `INVALID_PDE→PDE, INVALID_PTE→PTE, ATOMIC→ATOMIC_VIOLATION,
  WRITE→RO_VIOLATION, READ→WO_VIOLATION`. Anything else is refused by name and cancelled (EFS only
  diverts `NV_ERR_INVALID_ADDRESS` faults, i.e. these five types).
- ⊘ The UVM enum values are the host module's **internal** enums, copied raw by EFS v1; they are
  pinned in `kf-abi` with the header lines they come from and a test, and the pin is valid only for
  the patch's own version (580.159.04). A v2 ABI should carry EFS-defined constants (§10 Q2).
- Guard: `gpcId < the GPC count this device advertised` and `utlb` in range, else refuse by name —
  guest UVM indexes `batch_context->utlbs[gpc * utlb_per_gpc + …]` with it (`uvm_volta_fault_buffer.c`
  `parse_fault_entry_common`; its `UVM_ASSERT`s compile out of a release build).

### 3.5 Row 3 — the guest's instance-block identity

- `kf-rm::chanlink` decodes two fields it reads today for nothing: `NV_CHANNEL_ALLOC_PARAMS.instanceMem`
  (`NV_MEMORY_DESC_PARAMS {base, size, addressSpace, cacheAttrib}`, at the guest driver's measured
  offset — `kf-abi` generated matrix, `+144` at 580) and the context share's `subctxId`
  (`NV_CTXSHARE_ALLOCATION_PARAMETERS`, +8), plus whether the channel named a context share.
- `ChanPlane` keeps, per guest VA-space key, the live **GR** passthrough twins with `(inst address,
  inst aperture VID/SYS, guest VEID or 0 when not in a subcontext)`. The fault plane asks for the
  **representative** of a key: the most recently scheduled live GR twin.
- Why a representative is enough: guest UVM resolves `instance → registered channel → (TSG subctx
  table by VEID) → gpu_va_space` (`uvm_gpu.c:3534-3630`). Every guest GR channel in that guest VA
  space is registered with guest UVM (a GR channel in an externally owned VAS cannot run until UVM
  binds it, `kernel_channel.c:2200-2206`), and its own VEID names its own subcontext, whose
  `gpu_va_space` is this VA space. ⇒ The fault reaches the right `gpu_va_space`. Cancels on Volta+
  are by VA + PDB (§3.7), so no per-channel precision is lost for correctness; only tools events
  name a sibling channel.
- No representative (no live GR twin in that key) ⇒ the record is cancelled, counted, printed.
- ⊘ Never the host's instance pointer: the packet carries only guest values the guest itself sent.

### 3.6 Delivery — the efs-wait thread

Per EFS file, a thread loops: `UVM_EFS_WAIT(max 64, 100 ms)` (no lock held) → for each record:
refuse-or-encode (§3.4, §3.5) → take the fault-plane lock (never taken by a vCPU) → if the guest
buffer has room, write the entry, append the id to the file's `delivered` set; else append to
`undelivered` (retried each loop and at every `GET`-advancing replay) → after the batch: publish
`PUT` and evaluate (§3.3) → release the lock. Bounds: `undelivered + delivered ≤` the host's parked
bound for that file (the host never gives more); the guest ring holds ≤ `entries − 1`.
A record that cannot be delivered (plane unregistered, no representative, refused type/GPC) is
`RESOLVE(CANCEL)`ed at once — the host would cancel it at the timeout anyway, and stock UVM would
have cancelled it as fatal: never parked for nobody.

### 3.7 Row 4 — replay and cancel, split out of the Translated stream

- **Rewriter** (`kf-chan/src/translated.rs`): at `MEM_OP_D` `TLB_INVALIDATE[_TARGETED]` decode
  `MEM_OP_C.REPLAY` (4:2; same field and values `NONE/START/START_ACK_ALL/CANCEL_TARGETED/CANCEL_GLOBAL/
  CANCEL_VA_GLOBAL` = 0…5 in `C36F`, `C46F`, `C56F`, `C86F`, and inherited by `C96F`/`CA6F` UVM HALs;
  pinned to hwref), `MEM_OP_A` (`TARGET_ADDR_LO`, `CANCEL_MMU_ENGINE_ID` — 6:0 up to C56F, 8:0 from
  C86F), `MEM_OP_B` (`TARGET_ADDR_HI`), `MEM_OP_C.ACCESS_TYPE`. Emit `Piece::Invalidate{pdb}`
  exactly as today, then — when `REPLAY != NONE` — `Piece::Fault(FaultOp)`. The `GP100_UVM_SW`
  subchannel's `FAULT_CANCEL_C` (after `_A`/`_B`) becomes `Piece::Fault(CancelInstance{…})`
  instead of today's `Refusal::SwMethod` (which poisons the device: a kernel channel).
- **Ring** (`ring.rs`): `Piece::Fault` → `Next::Fault{op, retires}`.
- **Pump** (`host.rs`): like `Next::Walk` — fence everything already pushed, suspend, and when the
  fence completes call `Publisher::fault(op)`; resume after. The fence is what makes the guest's own
  ordering ours: the replay push **acquires** the service tracker before its `MEM_OP`, so the fence
  completes only after the servicing channel's work — whose own invalidate split already walked and
  mapped (§3.8).
- **Resolution** (`FaultPlane`, called from the worker; no lock held across an ioctl):
  - `Replay{START|START_ACK_ALL}` ⇒ for every EFS file of this device with a non-empty `delivered`
    set: `RESOLVE(REPLAY, delivered)`, then clear it. Nothing delivered ⇒ **no host call** (a guest
    spinning on replays reaches the host GPU zero times).
  - `CancelVa{pdb, va, …}` ⇒ keys whose guest root is `pdb` (`VasTable::keys_for_pdb`) ⇒ their files ⇒
    delivered records at page `va` ⇒ `RESOLVE(CANCEL)`. The mask is widened to all accesses (EFS v1
    `MODE_ALL`), stated in the boot report.
  - `CancelGlobal` (`MEM_OP`) ⇒ every delivered record of this device ⇒ `RESOLVE(CANCEL)`.
  - `CancelTargeted{gpc, client}` (`MEM_OP`) ⇒ delivered records with that GPC/client ⇒ `CANCEL`.
  - `CancelInstance{inst, aperture, mode, gpc, client}` (`C076`) ⇒ the guest channel whose instance
    it is ⇒ its key ⇒ its file ⇒ delivered records (all, or GPC/client-matched) ⇒ `CANCEL`.
  - A cancel naming nothing of ours ⇒ counted, no host call.
- A host `CANCEL` makes the twin's context fault; host RM RCs it; the existing RC plane forwards
  `RC_TRIGGERED` to the guest channel (`chan.rs` `rc_scan`, `device.rs` `deliver_rc`) — the
  observable a cancelled fault has on bare metal (the context dies, CUDA reports 719).

### 3.8 Row 5 — the guest's new mapping, mirrored before the replay

- The guest's service ends in a `MEM_OP` invalidate naming its PDB → today's split: fence the PTE
  writes, walk that root, diff, apply (§3.0 executor: each `UVM_MAP_EXTERNAL_ALLOCATION` returns only
  after its PTE writes and TLB invalidate completed — `uvm_map_external.c` `uvm_tracker_wait_deinit`),
  ack → resume.
- The replay push acquires that work's tracker semaphore; the semaphore release sits **after** the
  split in the servicing channel's stream, so it runs only after the reconcile. The replay's split
  fence therefore completes only after the mapping exists on the host. ⇒ "reconcile, ack, then
  replay" holds by the guest's own synchronization, with no extra walk.
- ⚠ `[inf]` until measured: a fallback `KF3_UVM_EFS_REPLAY_WALK=1` walks every key with delivered
  records at the replay piece (one split per root) before resolving — the belt to the braces above,
  off by default, used only if a hardware run shows a replay racing its own mapping.

### 3.8a ★★★ The GR engine is held while a fault is parked — the walker cannot run (`[meas]` 2026-09-30, RTX 3060)

**`[meas]` uvmg4, 2026-09-30, `d178a737`, box `vuvm` (RTX 3060, nested), `KF3_FAULTLOG=1 KF3_MAPLOG=1`,
`traces/v3_uvm_guest/run_uvmg4_qemu.log.gz`.** `um_probe gpufirst`: 14 faults delivered at
`t=5419.8899`; the guest's servicing invalidate reached its split at `5419.891` and the walk kernel
was submitted — and it completed only at `5424.1997`, **4.3 s later**, 7 ms after the host's ctxsw
watchdog killed every twin of the context (`RC-SEEN … except_type=0x6d`, host `Xid 109 CTX SWITCH
TIMEOUT` then `Xid 31 … FAULT_PDE`). The mapping landed at `5424.2007`, the replay came at
`5424.2023` — ordered correctly, and 4.3 s too late. ⇒ **A GR context stalled on a PARKED
replayable fault cannot be context-switched out on this GPU, so no other GR work runs — including
kf3's walk kernel — until the fault is replayed or cancelled.** The b3 host-only proof already
shows it from the other side: `coexist` lost ~3 s of GR time during a 3 s park
(`traces/v3_uvm_b3/run_full.log` §D2, 134.7 vs 211.9 iters/s over 8 s) — recorded there as a PASS
because the data were right.

Consequences:
1. **Design:** §3.8's "the split walks, then the replay" deadlocks by construction: the walk needs
   GR, GR is held until the replay, the replay waits for the split. Fault servicing must be
   **GR-free end to end**.
2. **Isolation / availability (owner):** while ANY EFS fault is parked, every GR context on the host
   GPU is stalled (other VMs, host CUDA). The bound is the host's ctxsw watchdog (~4.3 s, then the
   faulted context is RC'd), not `uvm_efs_timeout_ms`. A guest that never replays can therefore
   stall the host GPU's GR for ~4 s per fault. This is a property of replayable faults on this
   hardware, not of kf3; it belongs in §5 and in the b3 host-privilege review.

**The experiment (this branch, `b875595a` onward, for the owner — ruling A.1/A.11):** while a
guest-UVM space has parked faults, its parked pages are translated **on the CPU** with the walk
kernel's own rules (`kf_cuda::point`, the `kf_walk.cu` decode transcribed off the same `KfFormat`
descriptor) reading guest page-table words through read-only CPU views of OUR store
(`mem::PtReader`, the verb the Translated plane uses for a vidmem GPFIFO), and placed with the
same host verb (`EfsMirror::place`, now idempotent):
- at a split naming that space: point-map its parked pages, release the split at once, and
  schedule a GPU walk of the space that reconciles everything else once GR is free;
- before every guest replay: the resolver waits (≤ 2 s) for the VA thread to point-map every
  parked page, then `RESOLVE(REPLAY)`.
Rows placed this way are "fast rows" until a GPU walk maps the same leaf (no UVM call — the
placement is identical); a later split naming the space re-validates the rest and takes down any
the guest no longer maps (a GPU walk never removes a row it did not place).
⚠ Known limits of the experiment: (a) changes in the SAME invalidate other than the parked pages
(e.g. an unmap) reach the host only with the reconciling GPU walk, after the replay; (b) any other
split or register invalidate issued while faults are parked waits for GR; (c) it reads guest
page-table words on the CPU beside the GPU walker — a constraint change the owner must rule on.

### 3.9 Rows 6/7 — prefetch toggle and access counters

- `MMU_PAGE_FAULT_CTRL` (prefetch faults): recorded, plain shadow, not mirrored (EFS never diverts
  prefetch faults; stock host UVM handles them — `uvm_efs.c` `uvm_efs_try_divert`).
- Access counters: unchanged (advertised, never written; no replay dependency).

### 3.10 Row 8 — `CLEAR_FAULTED` and the non-replayable shadow buffer (after the four apps)

- Non-replayable faults (a user CE channel touching an unmapped managed page) reach the guest only
  through the client shadow buffer (`0x20800a9d`, answered today) + the `MMU_FAULT_QUEUED` event
  (`0x1005`, in the generated matrix only), and the guest resumes the channel with `CLEAR_FAULTED`.
- Host side, EFS v1 does **not** divert non-replayable faults: stock host UVM treats a CE fault on an
  external range as fatal and host RM RCs the twin — the same outcome as today, not a regression.
- Needed, in order: (1) a v2 host patch that parks non-replayable faults of EFS spaces and resolves
  them with `CLEAR_FAULTED` on the kernel's saved channel (`uvm_gpu_non_replayable_faults.c`
  `clear_faulted_method_on_gpu`); (2) kf3 writes the shadow queue entry (`queuePushNonManaged` shape,
  `kern_gmmu_gv100.c:1246-1279`) and posts `MMU_FAULT_QUEUED` on the drainer like `deliver_rc`;
  (3) the rewriter turns `C076 CLEAR_FAULTED_A/B` (Ampere+) and the host `CLEAR_FAULTED` method
  (Volta/Turing) into that resolution. Built only if the four apps measure a CE fault (§9 M3).

## §4 Threads and locks

| thread | new work | may block on | holds while blocking |
|---|---|---|---|
| vCPU | `GET(1)` store, evaluate, latch, one eventfd write | nothing | nothing |
| drainer | registration hand-off (validate, store under the fault-plane lock) | the fault-plane lock (short, no ioctl under it) | GSP lock (as today) |
| chan-act | `UVM_REGISTER_CHANNEL` / `UNREGISTER` | the UVM ioctl | nothing new |
| vamgr | EFS VAS create/teardown; UVM external map/unmap | UVM ioctls | nothing new |
| worker | `Publisher::fault` → `RESOLVE` | the UVM ioctl | **nothing** (the plane lock is released before the ioctl) |
| efs-wait (new) | `WAIT`, encode, guest-RAM write, `PUT` publish, latch | `WAIT` (≤ 100 ms) | nothing during `WAIT`; the plane lock only around the ring write |

The fault-plane lock is never taken by a vCPU and never held across an ioctl or a sleep; the
vCPU side is atomics only. Constraint `THE_CONSTRAINTS.md` 4 (no blocking on a vCPU, none under a
lock another thread blocks on) holds.

## §5 Security — hostile guest, both boundaries, other VMs

| actor | can it… | why not |
|---|---|---|
| guest userspace | write `GET`/`PUT` or the interrupt tree | VF-PRIV registers are not on the usermode page; `Class::UserspaceMappable` does nothing (`trap.rs`) |
| guest userspace | issue a host replay/cancel from its passthrough channel | its twin is an unprivileged host channel: `TLB_INVALIDATE` is a privileged host method (`alloc_channel.h:207-214`) and host RM faults the channel; `C076` needs an RM object the twin never gets |
| guest userspace | make kf3 replay/cancel | only guest-**kernel** (Translated) channels reach the rewriter; a user channel's stream is never parsed |
| guest root | cancel/replay another VM's faults | records live in other VMMs' UVM files; EFS refuses a foreign tgid (`efs_auth` `[meas]`) and resolves only ids its own file issued |
| guest root | spin replays to storm the host GPU | a replay reaches the host only with delivered records, i.e. at most once per host fault batch; rate is bounded by real faults |
| guest root | hold the host GPU | parked records are cancelled by the host kernel at `uvm_efs_timeout_ms`; teardown cancels all (`[meas]`) |
| guest root | steer a packet write | pages validated to guest RAM at registration, writes via the bounds-checked `RegionMap`; `GET` clamped; entries never straddle pages (4096 % 32 = 0) |
| guest root | learn a VMM or host address | packets carry only guest values (its instance block, its VA, its VEID) and the GPU timer the guest already reads; no host instance pointer or PDB exists in the EFS ABI |
| guest root | reach the twin's GR context buffers | they sit in the channel window, mapped by host UVM as channel ranges; a guest leaf there fails to map (no external range) and is refused by name — self-harm only (ruling B Q11) |
| another host process | register kayfabe's twin VAS in its own UVM file | ⚠ **Bug 1624521 (owner-open, `V3_UVM_B3_IMPLEMENTATION.md` §0.3).** In EFS mode kayfabe's twin VASes become externally owned, i.e. registrable through the same stock path every CUDA process's VAS is; RM's dup path applies its default PID share policy (`sharing.c`, `client_resource.c` `cliresShareCallback_IMPL`). Not closed here, not widened beyond "like any host CUDA process", and only with the plane on. A negative control (a foreign process attempting `UVM_REGISTER_GPU_VASPACE` on kf's handles) is evidence item E-S1 |

⊘ **The windows gap, fixed for EFS spaces only.** Today every mirror maps the store identity window
and the guest-RAM window, including user twins' spaces (read from code, not measured). EFS-mode
spaces map neither (they need neither: no Translated channel lives in a user VA space). The
RM-owned mirrors are unchanged here; the gap is reported for them separately.

## §6 Default off, and what "absent" means

- `KF3_UVM_EFS` unset (default): no probe, no FaultRing arm on the vCPU, no new threads, every
  mirror RM-owned, rewriter behaviour unchanged except that a `REPLAY`/`CANCEL` field is **decoded
  and counted** (still dropped). ⇒ The merge bar with EFS off exercises today's paths.
- `KF3_UVM_EFS=1` but the host module is stock, too old, or loaded with `uvm_efs_enable=0`: one line
  `kf3: UVM EFS REFUSED: <what the probe saw>`, then exactly the default-off behaviour.
- `KF3_UVM_EFS=1` and accepted: fault-capable guest spaces get EFS mirrors; everything else as today.
- The DELIVERY_UNBUILT sentences in the boot report become conditional: printed when the plane is
  off, replaced by the plane's counters when it is on.

## §7 All families

| fact | Turing | Ampere/Ada | Hopper | Blackwell | source |
|---|---|---|---|---|---|
| fault buffer regs (PRIV-relative) | tu102 | tu102 | tu102 (CC off) | gb100 (same offsets) | hwref `dev_vm.h` |
| packet layout | clc369 | clc369 | clc369 | clc369 | hwref `class clc369.h` |
| `ACCESS_TYPE`/`FAULT_TYPE` values | tu102 | ga100 | gh100 | gb100/gb202 (`0xb` = `CC_VIOLATION`) | hwref `dev_fault.h` |
| `MMU_ENG_ID_GRAPHICS` | 64 | 64 | 384 | 384 | hwref `dev_fault.h` |
| replay/cancel `MEM_OP` | C46F | C56F / C56F | C86F (`CANCEL_MMU_ENGINE_ID` 8:0) | C96F (UVM HAL inherits C36F encodings) | hwref `class` rows |
| replayable vector | host table | host table | host table | host table | `MC_GET_STATIC_INTR_TABLE` |
| `C076` | yes | yes | yes | yes | `clc076.h` |
| EFS host patch | not built | **580.159.04** | not built | not built | `tools/uvm_efs` |

Nothing above is a GA10x constant in code: every value is looked up per family from the derived
table or the host. The only hand row is the UVM-enum inverse, which is version-pinned (§3.4).
Measured first on GA10x (GA106); other families are source-derived until run.

## §8 Test plan

GPU-free (each a `#[test]` in the crate that owns the logic):
- `kf-abi::faultpacket`: every field at the hwref position for every family; `VALID` in the last
  dword; round trip against a decoder written from `parse_fault_entry_common`; the UVM-enum inverse
  pinned; refused types/GPCs.
- `kf-abi::uvmefs`: `UVM_EFS_*` and UVM ioctl params at their C offsets/sizes (header lines cited).
- `kf-trap::faultring`: capacity from size, wrap, full (`put + 1 == get`), clamp of hostile `GET`,
  the evaluate rule (`get != put`), and a two-thread Dekker test (no lost latch over N rounds).
- `kf-chan::translated`: the replay `MEM_OP` UVM pushes (`uvm_hal_volta_replay_faults` bytes) →
  `[Invalidate{Some(0)}, Fault(Replay)]`; `CANCEL_VA_GLOBAL` → the PDB/VA/access/engine decoded per
  channel class; `C076 FAULT_CANCEL_A/B/C` → `CancelInstance`; `REPLAY_NONE` unchanged.
- `kf-chan::ring`/`host`: `Next::Fault` ordering (fence before the op; words after it wait).
- fault plane (pure core in `kf-qemu` or `kf-core`): ownership refusals — a cancel naming another
  key's PDB touches nothing; `CancelInstance` of an unknown instance touches nothing; replay with
  nothing delivered issues no host call; delivered-set bookkeeping; attribution representative.
- `kf-linux-raw::uvm`: every op's minimum buffer covers the kernel struct (the `_IOC_SIZE` trap:
  UVM numbers are plain integers, `UVM_INITIALIZE` decodes to 12288).

Hardware (box `vuvm`, then evidence under `traces/v3_uvm_guest/`):
- **E-T1** EFS twin, no guest: gate-style host test — an EFS-mode VAS + a GR channel of kf's own +
  external maps of the store; cup-style work runs bit-exact; an unmapped touch becomes an EFS record.
- **E-M2** the milestone: `um_probe gpufirst` (and `cpuinit`) in a kf3 fat guest, host patched with
  `uvm_efs_enable=1`, `KF3_UVM_EFS=1` → correct data, the guest's fault counters moved, zero
  unserviced Xid.
- **E-A** the four apps, each checked for correct results (conjugateGradientUM's own error check,
  attach_verify's verification).
- **E-S1** Bug 1624521 negative control (foreign process, kf's handles) — evidence for the owner.
- **E-B** the merge bar at the exact head, EFS off (default) and EFS on.

## §9 Milestones

| # | milestone | done? |
|---|---|---|
| M1 | this note, pushed | `0a7a1439` |
| M1a | `kf-abi`: `faultpacket`, `uvmefs` (+ tests) | `ed20bd06` |
| M1b | `kf-trap::faultring` (+ Dekker test) | `ed20bd06` |
| M1c | rewriter/ring/pump: `Piece::Fault`, `Next::Fault`, `Publisher::fault` (+ tests) | `ed20bd06`; replay/cancel `MEM_OP` = the op alone since `aeaebe33` |
| M1d | `kf-linux-raw::uvm`, `kf-host::efs` (probe, session, register, map/unmap, wait, resolve) | `6e9b5bb4`; `UVM_MM_INITIALIZE` `89b1ac24` |
| M1e | `kf-rm`: `fault_capable`, `instanceMem`, `subctxId` decodes | `6e9b5bb4` |
| M1f | `kf-qemu`: EFS mirror + UVM `MapTarget`, twin registration, FaultPlane, efs-wait, vCPU arm | `6f5e0674` |
| M1g | §3.8a GR-free servicing (CPU point walk of parked pages, early splits) — **experiment** | `b875595a`…`c849f68d` |
| M2 | E-T1 then **E-M2 correct data** — the first guest managed-memory claim | ★ `[meas]` `uvmg7` at `c849f68d` (E-T1 folded into it: the EFS twin ran every mode) |
| M3 | E-A: the four apps correct (non-replayable §3.10 only if measured necessary) | |
| M4 | E-B: merge bar at the head, EFS off and on | |

## §10 Open questions — for the owner, with the evidence that will decide them

- **Q1 (Bug 1624521, owner-open, unchanged):** EFS mode makes kf's twin VASes externally owned,
  therefore registrable through stock UVM like any CUDA process's. Not solved here. E-S1 records
  what a foreign process can and cannot do.
- **Q2 (host ABI v2):** carry EFS-defined constants instead of nvidia-uvm internal enums; add
  `poll`; add non-replayable parking + `CLEAR_FAULTED` (§3.10); optionally a cancel mode. Each is a
  host patch change and needs the host-only proof re-run.
- **Q3 (threads):** one efs-wait thread per fault-capable guest VA space is acceptable for a
  default-off feature; `poll` (Q2) would make it one thread per device.
- **Q4 (non-nested):** memory-plane changes should also run on a non-nested host; none is reachable
  from this workspace (`172.22.1.20` unreachable on 2026-09-30). Results here are nested Vast KVM.

Box: `53564695` (RTX 3060, `vuvm`), rented 2026-09-30 18:00 UTC for this branch; registered in the
session's `vast_boxes.reg`. Destroy by id when paused or done.
