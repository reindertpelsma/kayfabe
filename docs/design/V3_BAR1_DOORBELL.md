# V3 — the Hopper+ BAR1 doorbell: follow where the guest RM puts it

**STATUS: DESIGN+CODE, 2026-09-26 — HARDWARE-UNVERIFIED.** Branch `v3-bar1db` (off `v3-mc2`).
Everything below about RM is read from ogkm-580.159.04 source (the owner's rule: the guest kernel
driver is open, so derive per-arch behaviour from it); nothing has run on a Hopper or Blackwell
GPU. The unit tests use a GH100-shaped fixture built from those citations. §7 is the H100 test
plan. Turing, Ampere and Ada behaviour is unchanged by construction (§6).

Supersedes: `kf-trap`'s fixed `DoorbellPlacement::Bar1 { page_base: 0x9_0000 }` (no source in any
ogkm header, flagged in `V3_FAMILY_PORT_ADA.md` and `V3_GUEST_DOORBELL_MODULE.md` §3), and the v3
reading of `THE_DESIGN.md` §5.7's "identify it by RM object handle" (corrected there, §1.4 below).

All ogkm paths are relative to `research_clones/ogkm-580.159.04/src/nvidia/` unless they start
with `kernel-open/` or `src/common/`.

## 1. What RM does — the findings

### 1.1 (a) The default: a BAR0 view

- `usrmodeConstruct_IMPL` (`src/kernel/gpu/fifo/usermode_api.c:32-105`) starts from
  `pMemDesc = pKernelFifo->pRegVF` (`:47`), class `NV01_MEMORY_LOCAL_PRIVILEGED` (`:48`). It keeps
  that unless the class is `>= HOPPER_USERMODE_A` **and** the client passed `bBar1Mapping`
  (`:61-65`, `:94-98`).
- `pRegVF` is built by `kfifoConstructUsermodeMemdescs_GV100`
  (`src/kernel/gpu/fifo/arch/volta/kernel_fifo_gv100.c:353-377`) as an `ADDR_REGMEM` memdesc at
  `kfifoGetUsermodeMapInfo_GV100` (`:155-172`) → `gpuGetRegBaseOffset_HAL(NV_REG_BASE_USERMODE)`,
  size `DRF_SIZE(NVC361)` = 64 KiB. On a GSP client that offset is **the GSP's answer**
  (`gpuGetRegBaseOffset_FWCLIENT`, `src/kernel/gpu/gpu_gspclient.c:308-325`, from
  `INTERNAL_GPU_GET_CHIP_INFO.regBases[]`) — i.e. **kayfabe authors it**
  (`kf-abi/src/chipinfo.rs` `reg_base::USERMODE`; `kf-trap/src/memmap.rs` `VF_USERMODE_PAGE =
  0xBB0000` = `NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET 0xB80000` + `NV_VIRTUAL_FUNCTION 0x30000`).
  ⇒ The BAR0 view never moves and is already served (§53.1 disposition C).
- Platforms with BAR1 disabled or all instance memory in sysmem silently get the BAR0 view even
  with `bBar1Mapping` (`usermode_api.c:84-92`). Not a discrete-GPU guest case.
- **RM's own internal channels always ring BAR0**: `kfifoRingChannelDoorBell_GH100` →
  `kfifoRingChannelDoorBell_GV100` (`arch/hopper/kernel_fifo_gh100.c:578-586`,
  `arch/volta/kernel_fifo_gv100.c:428`) → `GPU_VREG_WR32`.
  ⊘ **This is the first defect the old code had**: `DoorbellPlacement::Bar1` made the BAR0
  doorbell write unrecognised on Hopper+ (`device.rs` computed `doorbell = false` for that
  variant). Fixed: the BAR0 doorbell is live on every family.

### 1.2 (b) `bBar1Mapping`: a BAR1 view at a VA the guest RM's BAR1 allocator chooses

**The memdesc.** `kfifoConstructUsermodeMemdescs_GH100`
(`arch/hopper/kernel_fifo_gh100.c:90-131`) builds `pBar1VF` (and the kernel-only `pBar1PrivVF`) as:
- `ADDR_SYSMEM`, contiguous, at "physical address" `DRF_BASE(NV_VIRTUAL_FUNCTION)` = `0x30000`,
  size `DRF_SIZE(NV_VIRTUAL_FUNCTION)` = `0x10000` (`:111-115`;
  `src/common/inc/swref/published/hopper/gh100/dev_vm.h:26-27`, compiled in via
  `kernel_fifo_gh100.c:30`). The PRIV one is `0x0..0x30000`.
- PTE kind `memmgrGetMessageKind_HAL` = `NV_MMU_PTE_KIND_SMSKED_MESSAGE` = `0xF` (`:117`;
  `arch/turing/mem_mgr_tu102.c:400-406`; `hopper/gh100/dev_mmu.h:50`, `blackwell/gb202/dev_mmu.h:41`).
- `MEMDESC_FLAGS_MAP_SYSCOH_OVER_BAR1`, GPU page size 4 KiB, CPU+GPU cache snoop ENABLED
  (`:118-126`) ⇒ GMMU aperture **SYSTEM_COHERENT**. RM names the encoding itself: *"MMIO surface is
  encoded with aperture = syscoh and KIND = SKED"* (`src/kernel/gpu/gr/kernel_graphics_object.c:467-468`).
- No IOMMU mapping for it (`src/kernel/gpu/mem_mgr/mem_desc.c:2748`,
  `src/kernel/rmapi/nv_gpu_ops.c:8015-8020`), volatility untouched
  (`arch/maxwell/virt_mem_allocator_gm107.c:1449`): the PTE address is the raw register offset.
- The same HAL serves **GH100, GB100/GB102/GB10B/GB110/GB112, GB202/GB203/GB205/GB206/GB207/
  GB20B/GB20C** (`generated/g_kernel_fifo_nvoc.c:524-528`); Turing/GA10x/AD10x get the GV100 HAL,
  which builds no BAR1 memdesc (`:520-523`). Class `BLACKWELL_USERMODE_A` (`0xc761`) takes the same
  params (`src/kernel/rmapi/resource_list.h:895-904`).

**The mapping call (the VA's origin).** A CPU map of the object (`RmMapMemory`):
1. `rmapiGetEffectiveAddrSpace` answers `ADDR_FBMEM` for a `SYSCOH_OVER_BAR1` memdesc
   (`arch/maxwell/kern_bus_gm107.c:6407-6410`), so `src/kernel/rmapi/mapping_cpu.c` takes the BAR1
   path: caching `NV_MEMORY_UNCACHED` (`:289-296`), `BUS_MAP_FB_FLAGS_ALLOW_DISCONTIG` for a user
   (non-kernel) map (`:484`), `kbusMapFbAperture_HAL` (`:512-516`).
2. `kbusMapFbAperture_GM107` (`kern_bus_gm107.c:3018`): static BAR1 refuses a SYSMEM memdesc
   (`arch/turing/kern_bus_tu102.c:1058`), so the mapping is **dynamic**; reuse is off for a
   discontig (i.e. every user) map (`:3043-3047`), on for a kernel map (`:3176-3189`, refcounted).
3. `_kbusMapAperture_GM107` → `dmaAllocMapping_HAL(pGpu, pDma, bar1[gfid].pVAS, ...)`
   (`:3412-3435`): **RM's BAR1 VA allocator picks the VA** and RM writes the BAR1 PTEs itself —
   aperture SYS_COH, kind `0xF`, address `0x30000 >> 12` + page index — then the CPU gets
   `BAR1 base + VA`, uncached.
4. Unmap: `kbusUnmapFbAperture_GM107` (`:3213-3290`) drops the refcount and frees the mapping
   (`_kbusInternalBar1Unmap` → `dmaFreeMapping_HAL`, `:2940-2960`).

**The allocation never reaches the GSP**: `HOPPER_USERMODE_A`/`BLACKWELL_USERMODE_A` carry only
`RS_FLAGS_ALLOC_NON_PRIVILEGED | RS_FLAGS_ACQUIRE_GPUS_LOCK` (`resource_list.h:885-904`), no RPC
flag; `memConstructCommon` sends nothing either. ⇒ In v3 (stock guest RM over our GSP) **the only
observable is the guest's BAR1 PTE write**, which the GPU walker already sees at the guest's BAR1
invalidate (`kf-qemu/src/mem.rs` `K_BAR1`).

**Who sets `bBar1Mapping`** (ogkm-readable consumers):
- **UVM, always on Hopper+** — `gpuDeviceMapUsermodeRegion` (`src/kernel/rmapi/nv_gpu_ops.c:5535-5590`,
  `.bBar1Mapping = NV_TRUE` at `:5548`), write-only CPU map, and it **rings its channels through
  it**: `workSubmissionOffset = clientRegionMapping + NVC361_NOTIFY_CHANNEL_PENDING` (`:5625`) →
  `UVM_GPU_WRITE_ONCE(*workSubmissionOffset, token)` (`kernel-open/nvidia-uvm/uvm_volta_host.c:44`).
  ★ So **every Hopper+ guest that loads nvidia-uvm rings UVM's channels through a BAR1 view** —
  UVM builds its channel manager inside `cuInit` (`UVM_REGISTER_GPU`). This is required whether or
  not libcuda sets the flag. (UVM reads PTIMER through BAR0 on Turing+, `nv_gpu_ops.c:2438-2447`.)
- **nvidia-push (nvkms)** — `AllocUserMode` (`src/common/unix/nvidia-push/src/nvidia-push-init.c:994-1002`):
  *"The BAR1 mapping is used for (faster and more efficient) writes to perform work submission,
  but can't be used for reads."*
- **libcuda** — closed; **cannot be read**. Both of NVIDIA's own open clients set the flag on
  Hopper+ with a performance rationale, so libcuda very probably does too. §7 T0 measures it
  unprivileged on the H100 host. The design does not depend on the answer.

⊘ **The second defect the old code had** — worse than a wrong offset. Before this branch, the
walker's BAR1 leaf for the view (SYS_COH, address `0x30000`) went down the ordinary sysmem path
(`kf_mem::ledger::desired_from_leaves`), which placed **guest RAM at guest-physical `0x30000`**
into the BAR1 window at that VA. Every doorbell through it was a silent store into low guest
memory: UVM's channels would never run and a real guest page would be corrupted. The fixed
`0x9_0000` carve was a separate, equally unsourced trap that could shadow whatever RM put there.

### 1.3 (c) The GPU-VA "internal MMIO" mapping

- Same flag: `bInternalMmio = bBar1Mapping` (`usermode_api.c:75-81`), and
  `usrmodeGetMemInterMapParams_IMPL` (`:112-135`) hands `pBar1VF` to `RmMapMemoryDma`, so a client
  can map the usermode page into **any GPU VA space it owns** (a channel's VAS) — same PTE shape:
  SYS_COH, kind `0xF`, address `0x30000`. Refused unless the object was allocated with the flag
  (`:129`).
- **Can the GPU ring doorbells through it? Yes** — that is its purpose: UVM's Confidential
  Computing path maps it (`nv_gpu_ops.c:5649-5676`, only `if (gpuIsCCFeatureEnabled(pGpu))`) and
  has CE channels ring *other* channels by a semaphore release to `workSubmissionOffsetGpuVa`
  (`kernel-open/nvidia-uvm/uvm_channel.c:1232-1234, 1422-1424, 1524, 1679`; the field's own doc,
  `kernel-open/common/inc/nv_uvm_types.h:288-291`).
- **In kayfabe that path cannot occur**: we answer `CONF_COMPUTE_GET_STATIC_INFO` with CC off
  (`kf-abi/src/confcompute.rs`). No other open-source consumer maps it into a GPU VA. Whether
  libcuda does (CUDA graphs with device launch, dynamic parallelism) cannot be read; CUDA's
  device-side launch goes through the GPU's own scheduler, and nothing in ogkm suggests a
  doorbell write from the GPU, but that is an inference — §7 T4 counts it.
- **Decision: refuse by name, without wedging the guest.** A GPU-originated write never traps, and
  the value it writes is a **guest-computed** token (`kchannelCtrlCmdGpfifoGetWorkSubmitToken_IMPL`
  never reaches the GSP — `V3_GUEST_DOORBELL_MODULE.md` §3). Mapping the host's real doorbell page
  into the twin's VAS would let the GPU write a guest token into the host doorbell and ring an
  arbitrary host channel — `THE_DESIGN.md` §4's untranslated-token hazard, and no table lookup is
  possible on a write we never see. So: **no host mapping is made; the walker run is acknowledged
  HELD** (the guest's statement is satisfied, its invalidate clears), counted as
  `usermode_unmirrored` and logged (bounded) as `USERMODE-VIEW-NOT-MIRRORED`. A GPU-originated ring
  through that VA then faults on the host twin — contained (`gpu_fault_is_contained`) and visible —
  instead of landing in guest RAM. ⚠ Owner decision (§8.1).

### 1.4 Correction folded into `THE_DESIGN.md` §5.7

§5.7 says *"We identify it by RM object handle — we serve the allocation, so we hold the handle
and the flag — and use the page-table diff only to confirm. Identifying it by 'it looks like the VF
register block' is a pattern match on guest-chosen data."* True of Mode 1 (ioctl forwarding); **not
of v3**: the allocation is never forwarded (§1.2), so the page-table leaf is the only source. And
it is not a pattern match on guest data: aperture SYS_COH **and** kind SMSKED_MESSAGE is the
GMMU's own decode of "this is internal MMIO at this register offset" — the same decode the
hardware performs. A guest can only produce such a PTE from guest kernel (it writes the BAR1
tables), and the worst it buys is a trap on a BAR1 page it chose, whose writes are decoded by our
token table like any doorbell.

## 2. The PTE signature, per family (`kf_chip::usermode`)

| family | usermode page as internal MMIO? | aperture | kind | user page `at` | PRIV page `at` | doorbell |
|---|---|---|---|---|---|---|
| Turing, Ampere (GA10x), Ada | **no** — `kfifoConstructUsermodeMemdescs_GV100` | — | — | — | — | BAR0 only |
| Hopper (GH100) | yes | 2 (SYS_COH) | `0xF` | `0x30000..0x40000` | `0x0..0x30000` | `+0x90` |
| Blackwell (GB10x, GB20x) | yes (same HAL) | 2 | `0xF` | same | same | `+0x90` |

`Family::usermode_mmio()` returns the row; `UsermodeMmio::classify(ap, kind, at, len)` answers
`None` (ordinary memory), `User { vf_rel }`, `Priv`, or `Stray`. ⊘ SYS_NONCOH + kind `0xF` is the
"SKED reflected" surface and VIDMEM + kind `0xF` is the compute-object MMIO
(`kgrobjSetComputeMmio_IMPL`, `kernel_graphics_object.c:405-473`); neither matches.

## 3. The design

```
guest RM: RmMapMemory(usermode obj, bBar1Mapping)   ── kbusMapFbAperture → BAR1 PTEs (SYS_COH, 0xF, 0x30000)
guest RM: BAR1 invalidate (trapped BAR0 write, vCPU arms the port, no blocking)
VA thread: GPU walk of OUR BAR1 root → diff run {va, len, at=0x30000, ap=2, kind=0xF}
           apply_entry: classify FIRST (before any memory row)
             User → Bar1Target::map_usermode → Bar1Doorbells::place (tracker, validates)
                                              → C verb QUEUES "install" (FIFO) + qemu_bh_schedule; returns
             ack APPLIED to the walker; the target's settle() = Pending
             ⇒ ONLY this invalidate's clear is deferred (VaManager.awaiting); the guest keeps polling
main loop (BQL): bottom half pops the FIFO, adds an ALIAS of the usermode ROM at BAR1+va (priority 1),
                 kf3_bar1_overlay_done(seq, 0) → Rust posts + signals the VA thread's eventfd
VA thread: wake → on_targets → settle() = Live → trigger cleared (overlay live BEFORE the clear)
vCPU: store to BAR1+va+0x90 → KVM read-only slot → lockless MMIO exit → kf3_bar1_usermode_write(vf_rel)
      → exactly the BAR0 usermode-page write: Class::Doorbell → token table (never the trap's identity)
guest RM: unmap → PTEs cleared + invalidate → walk emits UNMAP(va) → Bar1Target::unmap queues "remove"
      → clear deferred until the main loop reports it (overlay removed BEFORE the clear)
```

### 3.1 Asynchronous by ruling (coordinator, 2026-09-26, ruling 5)

The first version had the VA thread wait (≤ 5 s) for the main loop. ⊘ **Rejected**: a thread that
blocks on the BQL holder is the deadlock class the blocking invariants forbid. Now:
- The C verb (`kf3_bar1_overlay`) only pushes onto a mutex-guarded FIFO and calls
  `qemu_bh_schedule` — it never waits. ⊘ Not `aio_bh_schedule_oneshot`: QEMU's BH list is LIFO
  within a slice, and an unmap queued before a re-map of the same BAR1 VA must apply first.
- `MapTarget::settle()` (`kf_mem::ledger::Settle`: `Live | Pending | Failed`) is how an async target
  says its accepted work is not visible yet. `VaManager` defers the clear of exactly the invalidates
  whose spaces answer `Pending` (`awaiting`, counted `deferred_clears`), and `VaManager::on_targets`
  — called on every VA-thread wake — clears them once every named space is `Live`. Invalidates of
  other spaces are never held behind it. With nothing deferred it asks no target anything.
- The main loop reports each change through `kf3_bar1_overlay_done(seq, rc)`: one push into
  `Bar1Overlay::done` (a Rust mutex held only for the push/take) and one write to the VA thread's
  eventfd. Lock order: BQL → that mutex; the VA thread never takes the BQL.
- A change that fails in QEMU after its walker run was acknowledged (it cannot, short of a bug: the
  tracker pre-validates alignment, bounds, overlap and the pool size exactly as the C side does)
  answers `Failed`: named in the VA refusals, counted, the invalidate stays armed; the view is
  retired and its later UNMAP retires quietly.
- A memory leaf over a view whose REMOVAL is queued is placed (it shows when the removal lands,
  before the clear); over a live or installing view it is refused by name.

- **Learned from the walk, not the alloc** (§1.4): the tracker (`kf_trap::bar1db::Bar1Doorbells`)
  is fed only by walked BAR1 leaves the family row classifies as the user page. It validates page
  alignment, the BAR1 extent, the 64 KiB page bound, overlap, and pool capacity, each refused by
  name. It is the single authority for which BAR1 pages carry a write trap.
- **No setup-time BAR1 carve on any family**: BAR1 is one whole plain-RAM memslot
  (`THE_CONSTRAINTS.md` §23); `memory_map` and `trap_regions` contribute nothing for BAR1.
  `DoorbellPlacement` is now `Bar0 { offset }` (Turing … Ada) or `Bar0AndGuestBar1 { offset }`
  (Hopper+); `may_trap_write(BAR1, ..)` is false statically, and the dynamic gate is
  `Bar1Doorbells::may_trap_write`.
- **The overlay is the §6.3 exception, used as §6.3 describes it**: a memslot change at the first
  mapping of the usermode object, slow path, made by the main loop under the VMM's global lock (the
  BQL) — never by a vCPU, and never waited for (§3.1). QEMU-side it is an alias of one 64 KiB ROM
  device created at realize; the alias pool (device property `bar1-overlays`, default 64) is
  created once and only added/removed (QEMU's rule: do not destroy regions during a device's
  lifetime). Adding it makes KVM split BAR1's slot around a read-only slot.
- **Reads** of the view hit the host's live usermode window (the ROM device's RAM is the host's
  64 KiB BAR0 usermode mapping, the same pages BAR0's disposition-C page aliases), so a read
  returns what the BAR0 view returns — a superset of hardware, where the BAR1 view "can't be used
  for reads". No read exit.
- **Writes** enter as the same register's BAR0 write: `vf_rel < 4 KiB` → `bar0_write(0xBB0000 +
  vf_rel)` (doorbell at `+0x90`; anything else `Class::UserspaceMappable`, which does nothing);
  pages 1..15 of the view do nothing and never reach BAR0's privileged path. Lock-free: the alias
  hands QEMU the offset inside the usermode page directly.
- **The doorbell is adversarial to guest root** (`the_doorbell_is_adversarial_to_guest_root`):
  unchanged. The trap carries no identity; the token is masked and looked up in our table; an
  unknown token does nothing.
- **Idempotent**: install of an identical live view = 0, remove of an absent one = 0.
- **Doorbell-module asymmetry** (`V3_GUEST_DOORBELL_MODULE.md` §2): map installs our trap at once
  (the module may take over later — `Bar1Target::views()` is the replay set); unmap removes our
  trap before the clear, and is the single place the module's "wait for the module's ack" will go.
- **No memory under a live view**: an ordinary BAR1 leaf overlapping a live doorbell view is refused
  by name (`Bar1Target::map`) — the overlay would shadow it and turn its stores into doorbell
  writes. The walker unmaps whole placements before re-mapping a VA, so a correct guest never hits it.
- **Status line** (Hopper+ only): `bar1db[trapped= unmirrored= deferred_clears= installed= removed= refused=]`;
  the C device's exit line adds `bar1_ov_applied= bar1_ov_failed=`.

## 4. Code map

| piece | file |
|---|---|
| per-family data + classifier | `crates/kf-chip/src/usermode.rs` (new) |
| placement tracker | `crates/kf-trap/src/bar1db.rs` (new) |
| `DoorbellPlacement::Bar0AndGuestBar1`, BAR1 static gate | `crates/kf-trap/src/trappolicy.rs` |
| BAR1 = one plain-RAM region on every family | `crates/kf-trap/src/memmap.rs`, `tests/memory_map.rs` |
| `UsermodeRow`, `MapTarget::map_usermode` (default = not mirrored) | `crates/kf-mem/src/ledger.rs` |
| classify before any memory row; counters | `crates/kf-mem/src/apply.rs`, `vasmgr.rs` (`with_usermode_mmio`) |
| `Settle`, deferred clears (`awaiting`, `on_targets`) | `crates/kf-mem/src/ledger.rs`, `vasmgr.rs` |
| `Bar1Target`, `Bar1Overlay`, `BAR1_OVERLAY_SLOTS` | `crates/kf-qemu/src/mem.rs` |
| BAR0 doorbell live on all families; `bar1_usermode_write`; wiring | `crates/kf-qemu/src/device.rs` |
| FFI (ABI 6 — on top of v3-reinit's 5, `kf3_bar0_read`) | `crates/kf-qemu/src/ffi_unsafe.rs`, `raw_unsafe.rs` (`OverlayHook`) |
| overlay ROM, alias pool (`bar1-overlays`), FIFO + one BH, completion callback | `qemu/hw/misc/kf3/kf3.c`, `kf3.h` |

## 5. Tests (local, no GPU)

- `kf-chip usermode::tests` — only Hopper/Blackwell have the row; the GH100 PTE of the BAR1 view
  classifies as the user page; guest RAM at `0x30000` (kind PITCH/GENERIC), SYS_NONCOH+SKED and
  VIDMEM+SKED never do.
- `kf-trap bar1db::tests` — a view at a guest-chosen BAR1 VA resolves `+0x90`; nothing outside it
  (including the old `0x9_0000`) traps; a discontiguous view keeps each piece's page offset;
  every refusal is named; removal ends the trap and the VA can be reused.
- `kf-trap tests/memory_map.rs` — BAR1 is one non-exiting region on every family.
- `kf-mem apply::tests` (GH100 fixture: view at BAR1 VA `0x0123_0000`, `at = 0x30000`, ap 2, kind
  `0xF`) — the BAR1 target gets a trap, never a guest-RAM row, and its UNMAP reaches the target; a
  GPU VA space answers HELD with no host call; **with the GA10x config (`usermode: None`) the same
  leaf takes the old memory path byte for byte**; PRIV/stray internal MMIO is refused while plain
  RAM at `0x30000` still maps; a view crossing the BAR1 extent is clipped like any row.
- `kf-mem vasmgr::tests` — an async target (`Settle::Pending`) defers only its own invalidate's
  clear, a wake with the work in flight changes nothing, the clear happens on `Live`, another
  space's invalidate is never held behind it; `Failed` leaves the invalidate armed and named.
- `kf3.c`: compile-checked against the local QEMU 11.1.1 tree with QEMU's own warning flags (header
  shims for the 10.2→11 moves): the new code is warning-free; the only errors are the pre-existing
  11.x API drift in untouched lines. The bench build against QEMU 10.2 is recorded in §5.1.

### 5.1 Bench evidence (vh, GA106 RTX 3060, 2026-09-26) — GA10x unchanged

- **kf3.c against the bench QEMU 10.2.4**: compile-only with the kf3 build's own
  `compile_commands.json` flags plus `-Werror`, base (`a7a07394`) and new side by side, in a private
  dir under the fast-guest lock: **both rc=0**. Then the real build (`scripts/bench/build_kf3.sh`,
  under the lock): `KF3_BUILT /workspace/bench/kf3-bins/427be031/qemu-system-x86_64`; `nm` shows
  `kf3_bar0_read` (v3-reinit's ABI 5) and `kf3_bar1_overlay_done`/`kf3_set_bar1_overlay`/`kf3_ov_bh`
  (this branch) in one binary — ABI 6 carries both.
- **30-arm fast suite at `427be031`** (`KF_DEVICE=kf3 fast_suite.sh bar1db_427be031 180`, clean
  checkout ⇒ the runner can only execute `kf3-bins/427be031`):
  **`FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0`, `FAST_SUITE_RC=0`** (`02:16:49`–`02:34:28`).
  Baseline `a7a07394` (`mc7`, same box, same budget): 30/30. Per-arm times match within a few
  seconds (e.g. `ce-client-guest-ram` 86 s vs 87 s, `gpga-reserve-probe` 42 s vs 46 s). Every
  status line says `family=Ampere` and none carries a `bar1db[...]` segment, as §6 predicts.
- ⚠ A first launch without `KF_DEVICE=kf3` scored 30× NOTRUN (the runner defaults to the nvkvm
  device and refused before its lock); it was stopped, kept as
  `bar1db_427be031_NOTRUN_nokf3_suite.out`, and is not a measurement.
- Nothing here exercises Hopper: §7 remains the proof plan.

## 6. Why Turing … Ada cannot change

`Family::usermode_mmio()` is `None` for them ⇒ `VaManager::usermode = None` ⇒ `apply_entry` never
classifies (test `ga10x_config_leaves_every_leaf_on_the_memory_path`); BAR1 stays
`Target::Window`; `DoorbellPlacement::Bar0 { offset: 0x90 }` is the same value; the memory map
yields the identical single BAR1 region it always did for them; the C device builds no overlay pool
(`kf3_bar1_follows_guest` = 0); the status line has no `bar1db[...]` segment. The only shared-code
edit on their path is the doorbell comparison, now `off == VF_USERMODE_PAGE + offset()` — the same
expression.

## 7. Hopper hardware test plan (H100 box)

Preconditions: a kayfabe build of this branch (`scripts/bench/build_kf3.sh`, ABI 5), a Hopper guest
that reaches `RmInitAdapter` (the Hopper port's own work), `KF3_PROF` off. Every result carries the
source revision.

- **T0 — does libcuda set `bBar1Mapping`? (bare metal, unprivileged, no kayfabe)** Run `cup2` under
  `tests/mode2/nvdiff/nvdiff_shim.c` (records every `/dev/nvidia*` ioctl with its parameter buffer on
  both sides). Find `NV_ESC_RM_ALLOC` with `hClass` `0xc661`/`0xc761`; decode the 2-byte
  `NV_HOPPER_USERMODE_A_PARAMS` (`bBar1Mapping`, `bPriv`). Then the `NV_ESC_RM_MAP_MEMORY` of that
  handle and the process's `/proc/PID/maps`: a BAR1-backed mapping is at `BAR1 phys + VA`, a BAR0
  one at `BAR0 phys + 0xBB0000`. ⇒ answers §1.2's open item. Also run with `nvidia-uvm` loaded and
  confirm UVM's own view exists (`sudo cat /proc/driver/nvidia/...` is not needed — T1 sees it).
- **T1 — UVM's view is trapped (guest).** Boot, `modprobe nvidia-uvm`, run `nvidia-smi` then
  `cuInit` only. Expect `bar1db[trapped>=1 installed>=1 refused=0]`, `bar1_ov_timeouts=0`, and UVM's
  channel tokens `fwd>0` in `chan[tokens=...]`. **Negative control** (known-positive for the
  detector): the same boot on a build with `with_usermode_mmio(None)` must hang in UVM's channel
  manager or show UVM tokens `fwd=0` — proving the counter measures the thing. Persist `dmesg` to
  `run_<tag>_dmesg.log` and assert it contains `NVRM`.
- **T2 — CUDA through the BAR1 view.** `cup2` → `cupctx` → `cup8` ladder: numerically correct,
  tokens rung via BAR1 advance `GP_GET`. If T0 said libcuda uses BAR1, each process adds one
  `installed` and, on exit, one `removed` (user maps are never reused, §1.2 step 2).
- **T3 — churn and capacity.** 200 sequential CUDA processes: `installed − removed` returns to
  UVM's 1; no `refused`. 70 concurrent: the 65th process's view is refused by name
  (`Full { cap: 64 }`) — confirm the message, then decide §8.3.
- **T4 — GPU-VA internal MMIO census.** `cup8`, the LLM lane, a CUDA-graph sample with
  `cudaGraphInstantiateFlagDeviceLaunch`, and a dynamic-parallelism sample: `bar1db[unmirrored=]`
  must be `0`. Non-zero ⇒ libcuda maps the page into a GPU VA; capture which process/VA and revisit
  §8.1 before anything else.
- **T5 — the signature.** Log the walked run for the view once (the `kf-mem` refusal/held lines
  already name `va`, `len`, page): `ap=2 kind=0xF at=0x30000 len=0x10000` for UVM's view; a user
  view at a different BAR1 VA per process.

**What proves it:** T1 and T2 green with `trapped>0`, `refused=0`, `timeouts=0`, the BAR1-rung
tokens making progress, the negative control failing, and T4 `unmirrored=0`. T0 settles the
libcuda question independently of kayfabe.

## 8. Rulings and open items

Coordinator rulings, 2026-09-26 (within the v3 constraints):
1. GPU-VA views: acknowledged HELD, not mirrored, counted — **kept**.
2. Reads through the BAR1 view = the host's live usermode page — **kept**.
3. Overlay pool 64 — **kept, as the kf3 device property `bar1-overlays` (default 64)**; one more
   view is refused by name (`Full { cap }`).
4. PRIV/stray internal-MMIO leaves fail loudly, logged by name — **kept** (guest self-harm only).
5. ⊘ **Changed**: no bounded wait on the VA thread — asynchronous overlay changes with deferred
   clears (§3.1).

Open:
- **The compute object's MMIO page** (left alone by ruling 6): on every family
  `kgrobjSetComputeMmio_IMPL` (`kernel_graphics_object.c:405-473`) describes a VIDMEM page with kind
  `0xF`, address `chid × 4 KiB` (*"completely ignored"*). If a client maps it into a GPU VA, kayfabe
  maps store offset `chid × 4 KiB` as PITCH memory in the host VAS instead of the internal MMIO.
  Needs its own look (whether CUDA maps it, and what the host twin should see there).
- libcuda's use of `bBar1Mapping` and of GPU-VA views — §7 T0 and T4.
