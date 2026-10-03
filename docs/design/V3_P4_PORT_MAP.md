# V3 P4 PORT MAP — memory, VA and the BARs

**STATUS: PROPOSAL, 2026-09-24 (w826).** A read-only survey (agent) of the P4 design, the v3 crates
at `v3` HEAD `2df4dfc3` (plus `v3-hostfacts` `28638efa` where noted), and the old tree, in the
format of `V3_P2_PORT_MAP.md`. Nothing in `/workspace/kf-master` was edited. Line numbers are as of
`2df4dfc3`. ogkm citations are `research_clones/ogkm-580.159.04` (the target driver).

**STATUS UPDATE, 2026-09-24 (w826, branch `v3-p4a`): build-order steps 2 and 3 are BUILT, UNRUN on
hardware.** §2.1(d): `WalkKernel::submit`/`try_collect` (own stream, pinned read-back,
`cuLaunchHostFunc` → eventfd; no `cuCtxSynchronize`; `ack` deleted, every report RESYNC and
`Report::require_full` refuses a delta). §2.1(a): `kf-trap/src/mmuinval.rs` `InvalidatePort`
(arm-before-publish via `Trigger::arm_next`). §2.1(b)+(c): `kf-mem/src/vasmgr.rs` (`VasTable`,
`Walker`, `GpuWalker`, `VaManager`) and `ledger::MapTarget`. Gates 8a/8b/8 are folded into ONE
binary, `kf-gate8` (rows 2+3, VER2 and VER3). ⊘ Not yet done: §2.2 (`MemObjects`, fn 70), the
kf-qemu wiring of the three registers (device.rs is owned elsewhere), Q6's faulted-channel walk,
Q7's pre-walk uncached read, and the device-side snapshot tables in `kf_walk.cu` (host no longer
uses them; removing them needs a PTX regen and the CUDA suite on hardware).

**STATUS UPDATE, 2026-09-25 (w826, branch `v3-p4b`): the guest-facing pieces are BUILT and the
thin guest is PAST `kbusVerifyBar2` on GA106 (rev `94d05853`, boots `p4b4`-`p4b6`).** Built:
§2.0 (`bar2PdeBase` declared, both roots zeroed on the GPU; realize = walker → reserve → import),
§2.1(a-c) wired into kf3 (the three `MMU_INVALIDATE` registers trap on the vCPU; ONE VA-manager
thread owns the walker and completes them), §2.2 (fn 70 writes the guest's `PDE3[0]` into our
root on the GPU; `SET_PAGE_DIRECTORY` and the `COPY_SERVER_RESERVED_PDES` publications become
`VasTable` roots; replies held until settled), §2.3(a,b) for BAR2 (`kf_mem::cpuwin::CpuWindow`)
and §2.4 (PRAMIN from 160 pre-armed 64 KiB views; Q2 as recommended). Q1, Q3 (per-BAR scratch
memfd) and Q4 taken as recommended. BAR1 stays on scratch (row 6 not built).
`[measured p4b4]` 25 invalidates: 24 cleared after the reconcile, 4 of them `named_missed`; 18
BAR2 views placed and all 18 released at teardown; PRAMIN 22 re-points, 0 misses, worst trap
542 µs. The boot now stops at the P6 wall (`_memmgrMemUtilsScrubInitScheduleChannel: Unable to
schedule channel, status: 56` — `0xa06f0103` GPFIFO_SCHEDULE unserviced), as §3 row 5 predicts.
⚠ Open: invalidate #24 (the guest kernel VAS `0xc1e00006:0xc`) stays ARMED by design — its walk
has 6 sysmem leaves and the host VA space has no guest-RAM object yet — so the guest spins ~22 s
to its own timeout. The guest-RAM object (`alloc_os_descriptor` over the memfd) is the fix.

**STATUS UPDATE, 2026-10-03 (branch `v3-scratch-bound`): Q3 is CORRECTED, twice.** Its bound ("bounded
by the BAR size") was reachable by guest root and was host RAM outside the VM's memory. The scratch
is now one small tile per window, repeated; see the correction at the top of Q3. The same day's
review then found the first correction's mapping bound false in production (QEMU's own `madvise`
kept every sink from merging back); that, a PRAMIN view retired while still reachable, and the VFIO
note are corrected in the block above it.

**Summary.** P4 is **~2.2k lines of product code plus ~0.9k of harness**. Only **~0.4k** of it is
copied old-tree code; the rest is new, because the old tree walked and mirrored guest tables on
the CPU and v3 forbids both. The main parts already exist in v3:
- `kf-mem` has the ledger and the reconcile planner.
- `kf-cuda` has the GPU walker, with VER2 and VER3 formats proven by gates 2 and 7.
- `kf-host` has the VA verbs (map with deferred invalidate, unmap, one `invalidate_tlb`) and the
  CPU-view verbs.
- `kf-trap` has the invalidate `Trigger`, with its compare-and-set clear.

What does **not** exist:
- the trigger that starts a walk;
- a table from each VA-space object to its PDB;
- serving `UPDATE_BAR_PDE` (fn 70) and the page-directory statements;
- our BAR2 root;
- any guest CPU view: BAR1, BAR2 and PRAMIN;
- a walk that does not block the thread that launches it.

⚠ **Two findings change the P4 gate. The six named arms cannot be graded at the end of P4** (§1.2).
- Four of the six need a channel and a copy engine (P5/P6).
- `--uvm-invalidate` needs UVM's own kernel channels (P6).
- The guest cannot finish `RmInitAdapter` until the CeUtils scrub completes (P6).

⇒ P4 is graded by **harness gates plus one boot-progress observable**. The arms are graded when P6
lets the guest boot.

---

## 1. Scope

### 1.1 What `RmInitAdapter` needs from the memory plane (ogkm-580)

The steps are listed in the order the guest runs them. Everything before step 9 is P4.

| # | guest action | where in ogkm | what we must provide |
|---|---|---|---|
| 1 | reads `bar1PdeBase` / `bar2PdeBase` from `GET_GSP_STATIC_INFO` | `kern_bus.c:755` (`kbusPatchBar1Pdb_GSPCLIENT`), `:829` (`kbusPatchBar2Pdb_GSPCLIENT`) | **our two root pages**, inside our reserved carve-out, zeroed, with their addresses declared. BAR1 has **no RPC**: the guest writes its PDEs directly into our page (`THE_OGKM_RESIDUE.md` §3) |
| 2 | builds the BAR2 page tables **through the BAR0 window**, because BAR2 cannot build itself | `mmu/bar2_walk.c:245` | **PRAMIN over the store**, re-pointed by writes to `NV_PBUS_BAR0_WINDOW` (`0x1700`, 64 KiB granular, `_TARGET` selects vidmem or sysmem: `THE_SURFACE_v3.md:427`) |
| 3 | reads its old PDE3[0] through the BAR0 window, then sends `UPDATE_BAR_PDE` (fn 70) with `entryValue` | `kern_bus.c:829+43`, `+52` | serve fn 70: write `entryValue` into entry 0 of **our** BAR2 root, and answer only after the write has landed |
| 4 | points its BAR2 PDB cache at **our** `bar2PdeBase` | `kern_bus.c:829+20..23` | nothing more to do. Every later BAR2 invalidate names our PDB |
| 5 | writes BAR2 PTEs, flushes write-combine buffers, runs `kbusFlush`, then invalidates the BAR2 PDB | `kern_bus_gm107.c:2797-2818` (`kbusUpdateRmAperture_GM107`), `:5819` (`kbusCommitBar2`) | the **invalidate is the sync point**: trap the `MMU_INVALIDATE` write, walk the BAR2 VAS, re-point the BAR2 CPU window, **then** clear `TRIGGER` (§49.1) |
| 6 | CPU reads and writes through PCI BAR3 ("BAR2") | everywhere RM touches instance memory and page tables | the BAR2 CPU window: slices of the store placed at the walked BAR2 VAs, with no traps |
| 7 | `kbusVerifyBar2`: writes and reads back through PRAMIN, then writes through BAR2 and reads back through PRAMIN | `kern_bus_gm107.c:3970`, `:4084-4096`, `:4164-4200` | PRAMIN and the BAR2 window must alias **the same store bytes** coherently |
| 8 | vidmem heap allocations, including RM's own reserved memory | GSP client heap (`THE_SURFACE_v3.md` §2.4: VASpace alloc is status-only) | **nothing per allocation.** An FB offset is a store offset (§56 rule 4); the store already covers `fb_length` (`kf-qemu/src/device.rs:139-140`) |
| 9 | kernel CeUtils scrub | — | **P6** (Translated). This is where a P4-complete boot must stop |

### 1.2 The raw-client arms that exercise memory, VA or BAR

Survey of `crates/kayfabe-rm-ladder/src/main.rs`; the suite list is `scripts/bench/rmladder_suite.sh:210-217`.

| arm | fn | what it does | needs a channel/CE? | gradable after P4? |
|---|---|---|---|---|
| **`--alias-two-vas`** (P4 gate) | `main.rs:5888` | one `NV01_MEMORY_LOCAL_USER` at two VAs; a release through VA_A still lands after VA_B is mapped | **yes**: COPY0 channel, `LAUNCH_DMA`, semaphore | ✘ P5+P6 |
| **`--alias-unmap-observe`** (P4 gate) | `:6150` | unmap VA_A; VA_B survives | yes | ✘ P5+P6 |
| **`--map-propagation`** (P4 gate) | `:6368` | map at a VA we dictate; `probe_va` + `pde_info` say it is mapped, and free again after unmap | **no** | ✔ **the only pure-P4 gate arm**, once the guest boots |
| **`--late-map-race`** (P4 gate) | `:3083` | a map made after the doorbell must still land | yes (doorbell) | ✘ P5+P6 |
| **`--missing-page-fault`** (P4 gate) | `:6540` | a real Xid 31 on the victim; the bystander is contained | yes (plus error notifier) | ✘ P5+P6, and P5.5 for fault delivery |
| **`--uvm-invalidate`** (P4 gate) | `uvm_raw::drive` `:16252` | UVM init, `REGISTER_GPU`, `REGISTER_GPU_VASPACE` (→ `SET_PAGE_DIRECTORY` to us) and a `tlb_invalidate_all` push | no user channel, **but UVM's own kernel channels** (`UVM_REGISTER_GPU` builds a channel manager). ⊘ An earlier survey said "no channel"; that is wrong | ✘ P6 (`MEM_OP` split) |
| `--map-stress` | `:6787` | rolling alloc/map/unmap/probe | yes | ✘ |
| `--defer-liveness` | `:11871` | a map with the deferred-invalidate flag; is it live? | yes | ✘ |
| `--guest-ram-pin` | `:1281` | memfd → `OS_DESCRIPTOR` → map at a dictated VA | no | ✔ once booted |
| `--bar1-crossing` | `:1379` | a vidmem CPU view (`NV_ESC_RM_MAP_MEMORY`) crosses to a child process | no | ✔ once booted. ★ This is the **guest BAR1 window** check |
| `--gpga-reserve-probe` | dispatch `:15032` | reserve 256 MiB, then read it back through BAR1 | no | ✔ once booted (needs a ≥90 s budget) |
| `--executor-vas`, `--rpc-mixed-allocs`, `--cross-client-leak`, `--dictated-ring(-negative)`, `--guest-ring-channel` | `:3416`, `:7177`, `:7608`, `:2603/:2745`, `:4754` | VA placement, alloc ordering, isolation | yes | ✘ P5/P6 |

⇒ **Recommended P4 gate:** harness gates 8-10 (§3), plus a fast-guest boot that gets past
`kbusVerifyBar2` and whose first kernel CE doorbell is the CeUtils scrub. Then, at the P6 boot:
`--map-propagation`, `--bar1-crossing`, `--gpga-reserve-probe` and `--guest-ram-pin` must pass
**with P4 code unchanged**. The alias, late-map, fault and UVM arms follow at P5/P6.

---

## 2. The pieces

Legend for the old-tree column: **COPY** / **ADAPT** / ✘ (does not fit, with the reason).

### 2.0 Declaring the guest FB layout — the first task of P4 (`THE_V3_PLAN.md` P4, "prerequisite")

- **Design:**
  - `THE_V3_PLAN.md` P4: *"guest FB layout must be declared before static info … `bar2PdeBase` is deliberately not written ⇒ first task of P4"*.
  - `THE_OGKM_RESIDUE.md` §3 obligations 1-3.
  - `THE_CONSTRAINTS.md` §49.4.
  - `THE_TRANSLATED_PLANE.md` §21.2 / §22.2: advertised size = object size = window size. The walker's CUDA context must come up **before** the reservation.
- **v3 today:**
  - `kf-chip/src/bar0.rs:183-229` `FbLayout{fb_length, regions, bar1_pde_base}`. The carve-out `FW_CARVE_OUT_BYTES = 0x1042_0000` and `BAR1_PDE_ABOVE_CARVE_OUT = 0x20C_C000` are **captured GA106 offsets**, used as layout constants.
  - `kf-rm/src/staticinfo.rs:178` serves `bar1_pde_base`.
  - `kf-abi/src/gspstaticinfo.rs:227`: `BAR2_PDE_BASE_OFF` is **"named and deliberately NOT written"**, citing a CPU plane that v3 drops (see open question Q4).
  - `kf-qemu/src/device.rs:139-140` reserves the store **before** any walker context exists.
- **Old tree:**
  - `kayfabe-device/src/ga10x.rs:224-290` (the `_for(fb_size)` derivations for WPR2, FRTS and fb_length): **COPY**, ~67 lines, fits.
  - `ga10x.rs:1273-1283` `bar1_pde_base_for`: already in kf-chip.
  - `kayfabe-qemu-raw/src/scratchpad.rs:607` `identity_window_verdict`: **ADAPT**, the check only (~60).
- **New:**
  - `FbLayout.bar2_pde_base`, plus root-page sizes per MMU format (VER2 and VER3 roots differ; take them from the kf-chip family row).
  - Write `bar2PdeBase` at offset 1672.
  - Zero both root pages at realize. This is a GPU memset through the walker context: the pages are ours, so no CPU read is involved.
  - Realize order: walker context → `reserve_gpga` → export/import into the walker context. Refuse by name if the advertised size exceeds the window.
- **≈ 180 lines.**

### 2.1 The invalidate as the sync point (trap → walk → reconcile → clear)

- **Design:**
  - `THE_TRANSLATED_PLANE.md` §5 and the `[w824b]` table: the vCPU records `(pdb, aperture)` and wakes a worker; the worker walks, maps, then clears `TRIGGER`.
  - `THE_CONSTRAINTS.md` §49.1: the guest must never observe the invalidate complete early.
  - §41 and §48: nothing but an enqueue on the vCPU.
  - `rm_cannot_express_a_narrow_invalidate`: the register path is always `ALL_VA`.
  - `THE_ARCHITECTURE_v3.md` §4.2: the ledger diff with deferred invalidation, then one invalidate.
- **v3 today:**
  - `kf-trap/src/shadow.rs` `Trigger`: `arm(seq)`, then `complete(seq)` is a compare-and-set that returns `Superseded` when a later write re-armed it. It is **not wired**: `kf-qemu/src/device.rs:303-336` classes every privileged write as `Plain`.
  - `kf-mem/src/ledger.rs:72` `desired_from_leaves`, `:159` `plan_reconcile`, `:326` `Ledger::apply` (deferred unmaps and maps, then **one** `invalidate_tlb`).
  - `kf-host/src/channel.rs:87` `alloc_vaspace`, `:126` `map(…, defer)`, `:148` `map_window`, `:157` `unmap(defer)`, `:166` `invalidate_tlb`.
  - `kf-cuda/src/walk.rs:404` `bring_up`, `:603` `refresh`, `:929` `import_store`.
  - Gates 2 and 7 prove walk → ledger → reconcile on hardware.
- **Old tree:**
  - `kayfabe-device/src/mmuinval.rs:241-276` `Invalidate::decode` (trigger, `ALL_VA`, `ALL_PDB`, `HUBTLB_ONLY`, the `SCOPE`/`CANCEL_GPC` overlap, PDB rebuilt from the lo/hi registers) and the offsets `:123-136` (`NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE{,_PDB,_UPPER_PDB}` = `0x30B0/0x30A0/0x30A4` above the PRIV base, which is the usermode base minus `0x30000`): **COPY**, ~60 lines. Pure; the offsets become generated kf-chip rows.
  - `note_trigger` / `drain_dirty_pdbs` (`:526-630`): ✘. They feed a table re-sweep (a mirror) and hold the poll behind a read.
- **New:**
  - **(a) kf-trap:** classify the three registers. The PDB lo/hi writes are `Plain` shadow cells. The `TRIGGER` write is `WriteSemantics::Trigger`: it arms and pushes one `PrivRing` entry `{pdb, aperture, all_pdb, seq}`. The `TRIGGER` read is served from the page (disposition B, never a read exit). ~120 lines.
  - **(b) kf-mem `VasTable`:** VA-space **object** (resource key) → `Option<(Pdb, Aperture)>` + `Option<host VaSpace>` + `Ledger`, following `THE_ARCHITECTURE_v3.md` §4.3 (identity is the object, not the PDB). An invalidate looks up every object carrying that PDB; `ALL_PDB` means every object that has one. A PDB no object carries is counted as `named&missed` and the trigger is still cleared (we hold nothing for it). ~250 lines.
  - **(c) The VA-manager step on the worker:** launch the walk; on its eventfd, run `plan_reconcile` → `apply` → `Trigger::complete(seq)`. A `Superseded` result is counted and never clears. ~150 lines.
  - **(d) kf-cuda:** make `refresh` asynchronous. Today `walk.rs:640` and `:660` call `ctx_synchronize()` on the caller's stack, which §5 forbids. Replace it with an event plus `cuLaunchHostFunc` writing an eventfd. **Strip** the delta-snapshot handshake (`walk.rs:699-720` `ack`, `KFWR_HF_RESYNC` in `abi.rs:77`): V3_BUILD forbids the walk kernel's delta snapshot, and the ledger diff replaces it. ~200 lines (plus PTX).
- **≈ 730 lines.**

### 2.2 Page-directory statements into kf-mem (`SET_PAGE_DIRECTORY`, `COPY_SERVER_RESERVED_PDES`, fn 70, promote)

- **Design:**
  - `THE_ARCHITECTURE_v3.md` §4.3: the PDB is a mutable, per-GPU, possibly-absent **attribute** of the VAS object.
  - §4.4: `COPY_SERVER_RESERVED_PDES` is the guest handing us real PDE addresses for the server's 512 MiB window. `SET_PAGE_DIRECTORY` is UVM's call, relocating the root onto client memory. `UNSET` is its inverse.
  - `THE_SURFACE_v3.md` §2.4: `SET_PAGE_DIRECTORY` and `GPU_PROMOTE_CTX` are **status-only**.
  - `V3_P2_PORT_MAP.md` §1: *"record the guest's statement as an attribute of the VA-space object"*.
- **v3 today:**
  - `kf-rm/src/rmrpc/mod.rs:1479-1670`: `translate_control` → `Translation::PageDir` for `0x00801813`, and `translate_published_pdes` (`:1721`) for `0x90f10106` / `0x20800a9f`, **with the aperture fork**. `PageDirStatement{client, vaspace, pdb, pdb_aperture}` is at `:295`.
  - The consumer refuses: `rmrpc/policy.rs:259-263` `GraphObjects::page_dir` → `NotModelled("…the memory plane is P4")`.
  - `UNSET_PAGE_DIRECTORY` → `PageDirNotModelled` (`kf-abi/src/capability.rs:3182`).
  - fn 70 is `Translation::Inert` (`mod.rs:1151`) and **nothing writes our root**.
  - `GPU_PROMOTE_CTX` → `PromoteCtxNotModelled` (`mod.rs:1631`), which is P7.
  - `kf-rm/src/lib.rs:169-171`: `BarPdePolicy` and `SetPageDirPolicy` were cut until P4.
- **Old tree:**
  - `kayfabe-device/src/bar2.rs:256-278`: the fn 70 body decode (`entryValue`, level shift). **COPY**, ~25 lines.
  - `bar2.rs:22-33`: the header, which is the right statement of ownership.
  - `setpagedir.rs:192-237`: the record fields. **COPY** the decode only; v3's `PageDirStatement` already carries what is needed.
  - `gvaspub.rs:150-180`: already folded into `translate_published_pdes`.
  - Logs and recorders (`BarPdeLog`, `GvasPubLog`, `SetPageDirLog`): ✘. They are records beside the plane, not inputs to it.
- **New:**
  - A kf-core `MemObjects: RmObjects` that wraps `GraphObjects`. `page_dir()` sets or clears the attribute in `VasTable`. On a changed root it schedules a full walk for that object, because nothing was invalidated.
  - `UNSET` clears the attribute.
  - fn 70 writes 8 bytes (`entryValue`) into our BAR2 root. This is a GPU write through the walker's window: the page is **ours**, so it is not a guest table we store. The reply is held until the write lands (the RPC-map sync point that `V3_P2_PORT_MAP.md` keeps), then BAR2 is walked.
  - The server-window levels from `0x90f10106` are **kept** on the object for P7, where promoted context buffers are mapped into the server's window.
  - ⊘ **No PTE encoder is needed in P4.** We copy one guest entry verbatim. P7 may need one: the reference masks are `kayfabe-chips/src/ga10x.rs:814-1005` (`Ga10xGmmu` decode + `relocate_entry`).
  - A sysmem-rooted `SET_PAGE_DIRECTORY` (UVM can place tables in sysmem) is refused by name until the walker imports the guest-RAM window.
- **≈ 250 lines.**

### 2.3 The BAR2 (instance) window, then the BAR1 root and guest CPU view

- **Design:**
  - `THE_CONSTRAINTS.md` §49.3: BAR1 and BAR2 are **ordinary VA spaces**, walked by the same walker. No split apertures, no MMIO-read walk.
  - §16: *"memslots are SETUP"*. One memslot per window; backings are placed inside it with `MAP_FIXED`.
  - `THE_CONSTRAINTS.md` §995 (w721): one world, no fake FB. BAR1, BAR2 and PRAMIN are all views of the store.
  - `bar1_simultaneous_view_ceiling.md`: the host BAR1 pool is ~254 MiB, allocated first-fit and **shared** with every CUDA context. It is reclaimed only by `NV_ESC_RM_UNMAP_MEMORY`, and each view costs one fd.
  - §w727: `guest_bar1 + headroom ≤ host_bar1`, powers of two, refuse at startup.
  - `userfaultfd_is_ruled_out` (memory): a memslot over a device VMA without the right permissions gives `KVM_RUN` → `EFAULT`, which **kills the guest**.
- **v3 today:**
  - `kf3.c:180-202` and `:270-275`: BAR1 and BAR3 are `memory_region_init_io` regions that read zero and count accesses.
  - `kf-trap/src/memmap.rs:163-179` already classifies BAR1 and BAR2 as `PlainRam`, with the Hopper+ doorbell page cut out (`:165-173`).
  - `kf-host/src/lib.rs:1045` `arm_cpu_view(node, h_memory, offset, len, access)` returns `(node, cookie)`.
  - `kf-host/src/lib.rs:1101` `release_cpu_view` (the `0x4F` release that w722 found missing is **built**).
  - `kf-trap/src/vmm.rs:112-119` `install_memslot` / `remove_memslot`.
- **Old tree:**
  - `kayfabe-qemu-raw/src/bar1budget.rs` (609 lines): the sizing relation. **ADAPT** ~60 lines into a startup check.
  - `nvkvm.c:1338-1404` `nvkvm_bars_realize`: the BAR registration pattern only.
  - ✘ `barmirror.rs` (3507 lines): one memslot per page, filled on a trap by a CPU walk. That is W+M+R+B together.
  - ✘ `plane.rs:5993-6300` `bar1_translate` / `bar2_translate`: CPU walks under the plane lock.
  - ✘ `nvkvm.c:809-1044`: BAR read and write traps.
- **New:**
  - **(a) kf3.c + ffi.** BAR1 and BAR3 become RAM regions over **one HVA reservation per BAR**, created by Rust at realize (`memory_region_init_ram_ptr`). The whole range is initially backed RW by a **sink** (one sparse memfd per BAR). There is exactly one memslot per BAR, and nothing ever traps except the Hopper+ doorbell page. ~80 lines of C plus ~60 of FFI.
  - **(b) kf-mem `CpuWindow`: a second apply target for `Ledger`.** `Ledger::apply` (`ledger.rs:326`) is hard-wired to `HostRm::map`, so factor it behind a `MapTarget` trait: `GpuVas{space}` or `CpuWindow{bar, hva}`. For `CpuWindow`, a `Desired{va, off, len}` row becomes `arm_cpu_view(store, off, len)`, then `mmap(MAP_FIXED)` at `hva + va`. An unmap becomes: re-point to the sink, then `release_cpu_view`. Contiguous runs are coalesced to bound the fd count. Refusal of a view by the host (`0x51`) is **counted and named**, never clamped. ~350 lines.
  - **(c) The walk roots.** BAR2 is walked from our `bar2_pde_base`, BAR1 from `bar1_pde_base`. Both are ordinary `VasTable` objects of kind `Bar(1|2)`, with no host `VaSpace`.
  - **(d) The size check.** The kf3 default `bar1-size = 256 MiB` (`kf3.c:320`) **violates §w727 as soon as views exist.** Derive the default as the largest power of two ≤ `host_bar1 − headroom` (on GA106 that is 128 MiB), and refuse at startup if it does not fit. ~60 lines.
- **≈ 610 lines** (BAR2 first, because boot needs it; BAR1 uses the same code).

### 2.4 The PRAMIN window over the store

- **Design:**
  - `THE_CONSTRAINTS.md` §53.1: PRAMIN is disposition **A** (r/w, no trap). It is re-pointed by `mmap(MAP_FIXED)` **synchronously inside the trapped `0x1700` write**, which is itself a B register.
  - `THE_BAR0_DISPOSITION_MAP.md:266,279`: *"297 µs worst, ~22 moves/boot"*. That was measured over the **old fake-FB memfd**, not over vidmem.
  - `pramin_is_a_bringup_aperture…`: RM's own bring-up uses it (`bar2_walk.c:245`, `memUtilsMemSetNoBAR2`, `kbusVerifyBar2`).
- **v3 today:**
  - `kf-trap/src/memmap.rs:136` PRAMIN is `PlainRam`.
  - `kf3.c:159-170` maps it as a **static shadow ROM device**. Reads come from Rust's RAM and writes trap, so it is neither disposition A nor the store.
  - `kf-chip/src/falcon_gsp.rs:175,184` has `PRAMIN_BASE` and `BAR0_WINDOW_REG`.
- **Old tree:**
  - `kayfabe-device/src/fbwin.rs:106-186` `Bar0Window` (latch decode: `fb_addr = (base<<16)+off`, `_TARGET`): **COPY**, ~80 lines, pure.
  - ✘ `fbwin.rs:1254` `SparseFb`: a CPU-side FB store (a shadow).
- **New:**
  - PRAMIN becomes a 1 MiB RW HVA region under one memslot.
  - A `0x1700` write stores the raw word in the shadow (`THE_SURFACE_v3.md:427`: read-modify-write must see it back), then re-points the region:
    - vidmem target → a store view;
    - sysmem target → the guest-RAM memfd at the file offset;
    - otherwise → the sink, counted.
  - ⚠ **The vidmem re-point cannot arm a view inside the trap.** Arming is an RM ioctl, which is forbidden on the vCPU path (§41). See Q2: views are pre-armed off the vCPU and the trap only does `MAP_FIXED`.
  - ~220 lines.
- **≈ 300 lines.**

### 2.5 The store's backing of vidmem allocations

- **Design:** `THE_TRANSLATED_PLANE.md` §18 (two ground truths, everything else is a map) and `THE_CONSTRAINTS.md` §56 rules 1 and 4 (no joins, no per-GPGA backings).
- **v3 today:**
  - `kf-mem/src/store.rs` is `{token, len}` plus a bounds check.
  - `kf-host/src/lib.rs:1263` `reserve_gpga` tries contiguous and 1 GiB-aligned first.
  - `kf-mem/src/ledger.rs:72` bounds vidmem leaves to the store and sysmem leaves to the guest-RAM layout.
- **Old tree:** `kayfabe-isolate-host/src/rm.rs:5020-5110` `reserve_gpga_inner`: already ported.
- **New:**
  - **Nothing per allocation.** A guest `NV01_MEMORY_LOCAL_USER` alloc is a graph node and a status reply.
  - Two checks do need writing:
    1. **The store must be zeroed** before the guest sees it. Host RM scrubs on alloc unless `NO_SCRUB` is set; assert it, because the guest's first reads are PRAMIN and BAR2.
    2. **Every vidmem leaf the walker reports must lie below `fb_length`.** Already enforced (`ledger.rs:78-81`, plus the walker's own §39(c) bound).
- **≈ 40 lines.**

### 2.6 Totals

| crate | new | copied/adapted |
|---|---|---|
| kf-chip / kf-abi (layout, bar2 root, invalidate rows) | 180 | 70 |
| kf-trap (trigger wiring, PRAMIN latch) | 180 | 140 |
| kf-mem (`VasTable`, `MapTarget`/`CpuWindow`, PRAMIN views, budget) | 900 | 60 |
| kf-cuda (asynchronous walk, delta stripped) | 200 | — |
| kf-rm / kf-core (`MemObjects`, fn 70, realize order, worker step) | 350 | 60 |
| kf-qemu + kf3.c (BAR HVAs, ffi) | 220 | — |
| **product** | **≈ 2 030** | **≈ 330** |
| kf-harness gates 8-10 | ≈ 900 | — |

---

## 3. Build order, with a hardware check per step

Every gate runs on a GA106 box and records its revision (§49.6). Every gate is run **twice in one
process** (P4.5 rule, `THE_V3_PLAN.md` §6.1). Gates 1-7 are re-run after every kf-host, kf-mem or
kf-cuda change.

| # | build | hardware check |
|---|---|---|
| **1** | §2.0 layout. Walker context → reserve → import; both roots zeroed; `bar2PdeBase` written; advertised = object = window | **kf-gate8a** (no QEMU): prints `WINDOW advertised=N object=N mapped=N`. Both roots read back as zero **through the walker's window** (GPU `cuMemcpyDtoH`). An invariant falsifier: ask for `advertised > window` and expect a refusal by name. **Fast-guest boot:** the P2/P3 observables do not move (`nvidia-smi` rows unchanged) |
| **2** | §2.1(d) asynchronous walk | gates 2 and 7 re-run on the asynchronous path. **kf-gate8b:** the worker thread's wall time on its own stack per walk is < 50 µs (the walk itself is ~424 µs p50), and the completion arrives as an eventfd wake. ⊘ Fails if any `cuCtxSynchronize` runs on the worker (count the calls) |
| **3** | §2.1(a-c) + §2.2. `MMU_INVALIDATE` trap → `Trigger` → `VasTable` walk → reconcile → CAS clear. `MemObjects` consumes the `page_dir` statements | **kf-gate8** (no QEMU; the harness plays the guest through `kf_core::Plane::trap_write` from 3 threads). It feeds kf-rm the real `0x90f10106` / `0x00801813` bodies, writes VER2 tables into the store, then writes PDB lo/hi + `TRIGGER`. It asserts: (i) `TRIGGER` reads 1 until the reconcile has applied, and **a CE copy through the mirrored VAS issued at that moment lands**; (ii) worst trap time on the trigger write in µs, with zero read exits; (iii) a re-armed trigger gives `Superseded`, not a clear; (iv) `named&missed` = 0 for known roots and counts an unknown one; (v) `ALL_PDB` reconciles every attributed space. The same run is repeated with VER3 tables (gate 7's pattern) |
| **4** | §2.3 BAR2. fn 70 → our root entry 0; BAR3 HVA + sink; `CpuWindow` target | **kf-gate9** (no QEMU, the BAR HVA in-process): BAR2-format tables in the store under our root, fn 70 delivered, invalidate. Then: (i) a CPU write through the HVA at a walked BAR2 VA is read back **on the GPU** through the identity window, and the reverse direction; (ii) after unmap plus invalidate the HVA page is the sink, and the host view's aperture is **released**. Use w722's fresh-offset round-trip: 5 rounds × 32 MiB, each one reaching the ceiling; (iii) an unmapped address reads the sink and nothing in the store changes. **Coherence falsifier (Q7):** write a PTE through the HVA, invalidate immediately, and require the walk to see it on 10⁴ repetitions |
| **5** | §2.4 PRAMIN, re-pointed in the trap from pre-armed views | **kf-gate10:** the `kbusVerifyBar2` shape in the harness (a PRAMIN write and readback at a 64 KiB-offset base, then a BAR2 write and PRAMIN readback of the same store byte), plus the sysmem `_TARGET`. Worst-case time of the `0x1700` trap in µs. **Fast-guest boot:** no `"BAR0 window"` or `"MMUTest"` errors in `run_<tag>_dmesg.log`; zero BAR1/BAR3 MMIO exits (the counted IO regions at `kf3.c:307` are gone, so count KVM exits for those GPAs); the **first kernel-CE token doorbell arrives** (the CeUtils scrub), and the boot stops there because the scrub is P6. ⊘ No fake completion to get past it |
| **6** | §2.3 BAR1: guest root adopted, the view budget enforced, the size derived | **kf-gate9** with a BAR1 root and the budget refusal (`bar1 + headroom > host` refused at startup). **The fast-guest boot** reaches the same scrub wall, and BAR1 PDE writes land in our root (the walker finds entries in `bar1_pde_base`) |
| **7** | (P6 boot) | ⊘ Grading, not building: `--map-propagation`, `--bar1-crossing`, `--gpga-reserve-probe`, `--guest-ram-pin` pass in the guest with P4 code unchanged |

The order is forced:
- **3 comes before 4.** The BAR windows are just a second apply target of the same trigger → walk → reconcile path.
- **4 comes before 5.** `kbusVerifyBar2` needs both views.
- **5 comes before any boot.** The guest builds BAR2 through PRAMIN, so a boot that fails at step 2 of §1.1 says nothing about BAR2.

---

## 4. Open design questions (each needs a decision)

**Q1. Is the ledger of our own map handles permitted under §56 rule 2?**
- Where it is raised: `THE_V3_PLAN.md` §1, `THE_ARCHITECTURE_v3.md` §4.2 box, `THE_TRANSLATED_PLANE.md` §5. All three say *"needs the owner"*.
- It is already built (`kf-mem/src/ledger.rs:288`) and proven by gates 2 and 7.
- **Recommend: ratify.** It records our actions, not the guest's tables: RM's `UNMAP` needs the VA back. §18.2's test does not fire, because nothing is copied or synced from the guest.
- The alternative, unmap-all/map-all per invalidate, costs ~1 800 RM calls and was never measured.

**Q2. PRAMIN over a vidmem store: who arms the host view?**
- §53.1 wants the re-point done inside the `0x1700` trap. The old 297 µs was an `mmap` of host RAM.
- Over vidmem, a re-point needs an RM `MAP_MEMORY` ioctl plus an fd plus BAR1 aperture. That is IO on the vCPU (§41).
- **Recommend:**
  1. Measure first: census every `0x1700` value in `cap1_coldboot_hermetic` (the C trace records BAR0 writes).
  2. Pre-arm views for the measured ranges (the BAR2 table region and the verify buffer) off the vCPU at realize, or lazily at the first fn 70.
  3. The trap then does only a `MAP_FIXED` of an already-armed view (a syscall with no RM lock, bounded).
  4. A miss re-points to the sink and is **refused by name and counted**: a visible boot failure, never a silent zero.
- Reject arming in the trap. It is 22 times per boot, but it establishes an RM ioctl on the vCPU path.

**Q3. What backs the unmapped parts of the BAR1, BAR2 and PRAMIN HVAs?**

> ⊘⊘⊘ **CORRECTED AGAIN 2026-10-03 (adversarial review of `v3-scratch-bound`) — the block below
> is right about host RAM and wrong about mappings, and it called the VFIO case open.** Four fixes,
> each with a test that fails on the code it replaces (bite runs of 2026-10-03, below).
>
> - **⊘ "Scratch mappings ≤ 1024 per window" was the count of an UNMAPPED window's cover, not a
>   bound, and "a sink restores the canonical tiling" was false in production.** kf3 covers a window
>   at realize; QEMU then registers it and advises the whole range (`MADV_HUGEPAGE`, `MADV_DONTFORK`,
>   and `MADV_DONTDUMP` with `dump-guest-core=off`; QEMU 10.2.4 `system/physmem.c:2294-2304`). Every
>   later `MAP_FIXED` (a view, a guest-RAM leaf, a sink) comes back without those flags, and Linux
>   merges neighbouring mappings only when their flags match (7.1 `mm/vma.c:84-96`;
>   `include/linux/mm.h:615-628`). So a sink never merged back, and every guest-driven
>   place-then-sink cycle at a new offset left two mappings for the VM's life. The review counted
>   16 384 after 8192 one-page cycles in a 64 MiB window whose tiling is 32 (master's whole-window
>   shape: 16 385, so this predates the tiling). Each mapping is a `vm_area_struct` plus maple-tree
>   slab, about 212 B (the review's host-wide estimate), charged to QEMU's cgroup outside `-m`:
>   bounded only by `vm.max_map_count` (≈ 13 MiB at the default 65 530; ≈ 212 MiB at 1 048 576, the
>   value on Ubuntu hosts). At the limit QEMU's own `mmap`s fail too: self-harm, but slab first.
> - **Fix.** One VMA-flag set for every window mapping Rust places off the vCPU,
>   `kf_linux_raw::WINDOW_ADVICE` = `DONTDUMP` + `HUGEPAGE` + `DONTFORK`, a superset of QEMU's,
>   applied by the initial cover BEFORE QEMU registers the window (`window_with_scratch` runs inside
>   `kf3_realize`, which precedes `kf3_bar_build`). QEMU's advice then finds the flags set and changes
>   nothing, whatever `dump-guest-core` says, so no re-cover after registration is needed. Every BAR1
>   and BAR2 sink (`kf_qemu::mem::window_sink` → `ScratchTile::cover_advised`) carries the same set
>   and merges back. Guest-RAM placements on BAR1/BAR2 carry it too. Device views do not need it:
>   they are `VM_IO | VM_PFNMAP`, never merge (`VM_SPECIAL`), and the driver marks them `VM_DONTDUMP`.
>   PRAMIN gets no `madvise`: its verbs run on the vCPU (owner ruling 2026-09-25, ONE `mmap` per
>   move), and every move re-places all 16 slots, so it holds at most 16 mappings regardless.
>   ⚠ The superset holds while QEMU's set stays inside it; read in 10.2.4 only.
> - **Decision: `VM_DONTDUMP` is preserved on placements.** With `dump-guest-core=off` a guest-RAM
>   page placed into a window would otherwise land in QEMU's core dump; with `=on` (the default) the
>   same page is still dumped through guest RAM's own mapping, so excluding the window alias loses
>   nothing. Residual: a guest-RAM run placed into PRAMIN (vCPU, no advice) is not excluded, ≤ 1 MiB.
> - **The mapping count now.** `ceil(W / T)` (≤ 1024) once nothing is placed, plus at most two per
>   LIVE placement (itself, and the tile it splits). Live placements never overlap, so they are at
>   most `W / page` per window. **No per-window cap on live placements is built**: BAR2 legitimately
>   holds RM's own 4 KiB structures (page tables, instance blocks), and a cap below `W / page` needs
>   a measured legitimate peak, which no box has taken; a refusal past a cap would only show the
>   guest scratch (self-harm). The memory cgroup must carry the slab term instead (Residual below,
>   and `V3_SWEEP_AND_INSTALL.md` §2.5).
> - **Residual, added term.** Mapping slab: ≈ 212 B × (Σ windows `ceil(W / T)` + 2 × live
>   placements), at most `vm.max_map_count` × 212 B per QEMU. Not bounded by `T`.
> - **⊘ A PRAMIN view could be retired while the guest still reached it** (pre-existing; reachable
>   once the guest can drive QEMU to `vm.max_map_count`). `PraminPool::repoint` retired every old
>   view, even when a run's placement or sink was refused. A refusal before Linux clears the old
>   page tables (7.1 `mm/vma.c:2439-2444`; `ENOMEM` at the map-count limit, `:1416-1418`) leaves
>   the old view mapped, and nvidia.ko does not zap user mappings when RM unmaps (its only revocation
>   is `nv_revoke_gpu_mappings_locked`, power-management paths; `ogkm-580:
>   kernel-open/nvidia/nv-mmap.c:786-816`), so the guest kept a CPU mapping of a released host BAR1
>   aperture. Now an old view is retired only when every slot it was placed over has been
>   re-placed since it was placed; otherwise it is kept and counted. ⊘ *Corrected the same day
>   (third review): the first fix required ONE re-point to re-place the whole range, so a view
>   landed over piecewise (refusals alternating between the two halves of the window) was held
>   for the VM's life, and `pramin_kept=` counted one event per re-point (399 for 200 re-points,
>   the review's probe).* Each slot now remembers the last re-point that landed it
>   (`PraminPool::repoint`, `landed_since`), so a kept view goes with the re-point that lands its
>   last slot; at most one view per slot is ever kept (16). `pramin_kept=` counts distinct views
>   ever kept, `pramin_kept_now=` the views held now. `CpuWindow::map/unmap` were checked and
>   already safe: unmap releases only after its sink lands, and a refused view placement never
>   leaves the new view mapped (`mm/vma.c:2496-2506`).
> - **A refused sink piece may be a HOLE**, not the old backing: Linux 7.1 clears the old page
>   tables before allocating the new mapping (`mm/vma.c:2476`) and then leaves *"a gap where the
>   MAP_FIXED mapping failed"* (`:2368-2388`). `ScratchTile::cover` now retries a refused piece
>   once and documents the hole; callers already release a view only after a successful cover.
> - **VFIO (was "Open, adjacent" below): refused, by mutual exclusion.** QEMU 10.2.4 has no
>   per-region opt-out (`memory_region_set_skip_iommu_map` first ships in 11.1.0, commit
>   `11b9798c7b`), and its VFIO listener DMA-maps every `ram_device` region
>   (`hw/vfio/listener.c:598-631`), pinning window pages and keeping IOMMU mappings of what they
>   showed then (a later re-point, or a view placed before a BAR re-enable, leaves them stale; a
>   released aperture among them). kf3's realize now BEGINS with `ram_block_discard_require(true)`,
>   before `kf3_realize` builds anything, and gives it back at exit or on any later realize
>   failure (one `fail:` label; QEMU calls no `exit` for a failed realize). ⊘ *Corrected the same
>   day (third review): it was first taken as realize's LAST step, so every refusal, an expected
>   outcome, leaked the host store reservation, the RM client, the threads and the windows for
>   QEMU's life, once per `device_add` retry.* Every 10.2.4 device that pins RAM disables discard first (vfio legacy
>   `container-legacy.c:568`, iommufd `iommufd.c:564`, `:599`, vfio-user `container.c:220`), so
>   whichever realizes second is refused, hotplug included: kf3 names the conflict; a VFIO device
>   fails with *"Cannot set discarding of RAM broken"*. Collateral refusals: vhost-vdpa (which
>   skips `ram_device` sections and so was never the hazard, `hw/virtio/vhost-vdpa.c:55-57`), SEV,
>   COLO. ⚠ Not caught: the userspace NVMe block driver (`nvme://`), which DMA-maps every RAM block
>   through a `RAMBlockNotifier` (`util/vfio-helpers.c:464-478`) and disables no discard. Do not
>   combine it with kf3. On QEMU 11.1 the skip flag is the precise fix. Checked by compiling kf3.c
>   against the 10.2.4 headers (`-Werror -fsyntax-only`); not run in a VM.
> - **Residual, the other side of the requirement (third review; pre-existing hazard, not built).**
>   `ram_block_discard_require` and `ram_block_discard_disable` exclude each other (10.2.4
>   `system/physmem.c`, both functions), so kf3 can never also DISABLE discard. Yet kf3 pins all of
>   guest RAM through RM once the guest-RAM object exists (`MemPlane::guest_ram_object`), which in
>   QEMU's model makes it a technology that disables discard, like VFIO. So virtio-balloon
>   (free-page reporting included; it inhibits itself only on `ram_block_discard_is_disabled()`,
>   `hw/virtio/virtio-balloon.c:75`) and virtio-mem (its coordinated requirement coexists with
>   ours) stay enabled and can discard ranges of the shared guest memfd under RM's pin
>   (`ram_block_discard_range`, `virtio-balloon.c:97`): the guest then sees fresh pages where the
>   GPU keeps the pinned old ones, and host RAM can reach about 2× guest RAM. Master's kf3 never
>   disabled discard either. Until it is refused by name at realize (not built), give a kf3 VM no
>   virtio-balloon and no virtio-mem device. On QEMU 11.1, `memory_region_set_skip_iommu_map` plus
>   `ram_block_discard_disable(true)` replaces the requirement and closes this too.
> - **Tests (2026-10-03; all ran and passed in GitHub CI run 37134501679 at `b90c9307`, the
>   kf-qemu one only there; the bites below were run locally on Linux 7.0).** `crates/kf-linux-raw/src/scratch.rs`:
>   `a_sink_restores_the_canonical_tiling_after_qemu_has_advised_the_window` (cover with the advice,
>   QEMU's advice after it, 600 place-then-sink cycles at distinct offsets, both `dump-guest-core`
>   settings, every mapping's `VmFlags` checked); with the advice bitten out it fails at
>   **1229 mappings against 32**. Its known-positive,
>   `an_unadvised_sink_beside_qemus_advice_never_merges_back`, holds the old shape's growth.
>   `scratch_aliases_every_tile_length_and_nowhere_else` now takes `T` from the policy and fails
>   when the tile is the whole window (it passed that bite before). `crates/kf-mem/src/cpuwin.rs`:
>   `a_refused_placement_keeps_the_view_it_did_not_cover`, `a_refused_sink_keeps_the_view_it_did_not_cover`,
>   `a_view_covered_only_in_part_is_kept`; with retirement made unconditional again each fails
>   (the first releases `0x10_0000` while the guest still reaches it). `crates/kf-qemu/src/mem.rs`:
>   `a_bar_sink_merges_back_after_qemu_has_advised_the_window` runs the production doors (CI only).
>   ⚠ All of these run on the host CPU; nothing here was run in a VM or on a box.
> - **Tests, third review (2026-10-03; bites run locally on Linux 7.0).** `crates/kf-mem/src/cpuwin.rs`:
>   `a_view_landed_over_piecewise_is_retired_and_kept_views_stay_bounded` (200 re-points that
>   alternately refuse each half; the first view must go by round 1, at most 2 views live, `kept`
>   = one per view); with retirement bitten back to "this re-point landed every slot" it fails at
>   round 1 holding 2 views. `a_view_kept_across_many_repoints_is_counted_once`; counting events
>   again it fails at 5 against 1. `crates/kf-qemu/tests/kf3_realize_discard.rs` reads kf3.c (CI
>   does not compile it): the requirement precedes `kf3_realize`, and no path after it leaves
>   without the `fail:` label that releases it. Against the previous kf3.c it fails with
>   *"comes after kf3_realize"* and *"no `fail:` label"*; with one `goto fail` turned back into
>   `return`, with *"2 `return;` after the requirement is held"*.

> ⊘⊘ **CORRECTED 2026-10-03 (branch `v3-scratch-bound`) — the recommendation below bounded scratch
> by the BAR size, and guest root can reach that bound. The scratch is now TILED.**
> Owner, 2026-10-03: *"The memfd is only scratch — isn't that a DoS target?"* It was.
>
> - **Threat.** Each window's scratch was one memfd of the WHOLE window, placed 1:1. A shmem page
>   has no zero page: a guest READ of a never-touched page reaches `shmem_fault`, which asks with
>   `SGP_CACHE` and allocates (Linux 7.1 `mm/shmem.c:2765`, `:2527-2564`; KVM gets there through
>   `hva_to_pfn_slow` → `get_user_pages_unlocked`, `virt/kvm/kvm_main.c:2883-2903`). The page is
>   charged to QEMU's memory cgroup (`mm/shmem.c:1985`) and kept until QEMU exits. Guest root that
>   reads every page of BAR1 and BAR2 (no driver needed: `mmap` of `/sys/bus/pci/devices/*/resource1`
>   after unbinding nvidia.ko) and points PRAMIN at an unbacked target therefore makes the host
>   allocate `bar1 + bar2 + 1 MiB` beyond the VM's `-m`, in milliseconds (one load per page).
>   Unprivileged guest users cannot: they reach BAR1 only through views kf3 places before the
>   invalidate clears.
> - **Before (per device).** 289 MiB at the device defaults (256 + 32 + 1, `kf3.c:887`, `:890`),
>   161 MiB under the bench launchers (BAR1 128 MiB). The only ceiling was `cardbudget.rs`: Σ per
>   host card ≤ host BAR1 − 16 MiB per device, which on a resizable-BAR host is about the whole
>   VRAM per card, summed over every card the VM uses.
> - **After.** One tile of `T = min(window, max(2 MiB, next_pow2(ceil(window / 1024))))` per
>   window, mapped again and again (window offset `o` shows tile byte `o % T`;
>   `crates/kf-linux-raw/src/scratch.rs`, used by every sink and by the initial cover through
>   `kf_qemu::mem::window_with_scratch`). Host RAM ≤ `T` per window: **5 MiB per device** at the
>   defaults and under the bench launchers (PRAMIN 1 + BAR1 2 + BAR2 2), 19 MiB with a 16 GiB BAR1,
>   131 MiB with a 128 GiB BAR1. Scratch mappings ≤ 1024 per window (128 for a 256 MiB BAR1).
>   PRAMIN is its own tile, so its trap's sink stays ONE `mmap` (owner ruling 2026-09-25; a
>   compile-time assertion in `kf-qemu/src/mem.rs`).
> - **Why aliasing is legal.** Nothing reads scratch expecting what was written there (nothing
>   copies or syncs it; §18.2), and an unmapped BAR page on real hardware has no defined contents.
>   A guest write into one unmapped page now shows `T` bytes away in the same window of the same
>   VM: self-corruption only.
> - **Residual.** `T` per window, plus host page tables and KVM SPTEs for a fully touched window
>   (about `window / 256` at 4 KiB pages, charged to the same cgroup; a placed store view costs the
>   same, so that term belongs to the BAR's size, not to scratch). Guest RAM is pinned whole and is
>   the large term. A deployment without a memory limit on QEMU's cgroup still lets these terms
>   press on the host: the launcher must size one (`V3_SWEEP_AND_INSTALL.md` §2.6).
> - **Tests, 2026-10-03.** `crates/kf-linux-raw/src/scratch.rs` (run locally on Linux 7.0, and in
>   GitHub CI run 37131187685 at commit `2b3062da`, all passing): `the_old_whole_window_scratch_allocates_the_whole_window_on_reads_alone` is the
>   known-positive (reads alone: `st_blocks` = 64 MiB of a 64 MiB window);
>   `a_tiled_scratch_holds_a_guest_that_touches_every_page_to_one_tile` is the bound (64 and 256 MiB
>   windows, every page read then written: ≤ 2 MiB), and it fails against the whole-window shape
>   (bite-checked that day: 67108864 > 2097152). `kf-qemu/src/mem.rs`'s
>   `a_window_built_for_the_vm_bounds_its_scratch_to_one_tile` runs the same bound through the
>   production constructor (CI only). ⚠ These fault from the host CPU, which enters
>   `handle_mm_fault` where KVM's GUP does; the KVM path itself is not yet measured on a live boot.
>   Bench check: guest root reads every page of `resource1` and `resource3`; on the host,
>   `stat -L -c %b` of QEMU's `/proc/<pid>/fd` entries named `memfd:kf3-scratch-*` (was
>   `memfd:kayfabe-guest-ram`) must stay ≤ 5 MiB in total.
> - **Open, adjacent.** QEMU's VFIO listener DMA-maps `ram_device` regions unless
>   `memory_region_set_skip_iommu_map` was called, and kf3 never calls it: a VM that also has a
>   VFIO device would pin every window page at start and keep stale IOMMU mappings after each
>   re-point. Read in QEMU 11.1.1 (`hw/vfio/listener.c:599-631`); not checked on 10.2.4.
>
> The recommendation below is kept as written; its *"one sparse RW memfd sink per BAR"* and
> *"bounded by the BAR size"* are what this corrects.
- The options are holes (a memslot per mapped run, which §16 forbids, or `PROT_NONE`, which gives `EFAULT` and kills the guest) or a sink.
- **Recommend: one sparse RW memfd sink per BAR,** with usage counted.
- It is not a shadow under §18.2 (nothing ever copies or syncs it with the store), and the worst a guest can do with it is corrupt itself.
- It is bounded by the BAR size. Charge it to the VM's memory budget.

**Q4. Declare `bar2PdeBase`?**
- `THE_V3_PLAN.md` P4 and `THE_OGKM_RESIDUE.md` §3 say yes. `kf-abi/src/gspstaticinfo.rs:227` says *"deliberately NOT written … no measured need"*.
- **Recommend: declare it.** That comment's reason was the old CPU plane walking from the fn 70 entry, which v3 drops.
- Without it, `kbusPatchBar2Pdb` points the guest's BAR2 PDB at FB address **0**, so every BAR2 invalidate names `Pdb(0)`. That is the "at offset 0 AND absent" ambiguity the thin-guest campaign already paid for.
- Update the comment in the same commit. Note that §49.4's "hardware walks our table" means **our walker** in v3.

**Q5. BAR sizes.**
- `kf3.c:320-321` defaults to BAR1 256 MiB and BAR2 32 MiB. The BAR1 default **cannot** satisfy §w727 on a 256 MiB host.
- **Recommend:** derive the default as `pow2_floor(host_bar1 − headroom)`, with headroom = the walker context + our channels + 16 MiB (measured `headroom=16 MiB`, w726). Keep the property as an operator override that is refused if it does not fit.
- BAR2 stays 32 MiB, but its views count against the same host pool: the budget is `bar1_views + bar2_views + pramin_views ≤ host_bar1 − headroom`.

**Q6. Where does the walk block, and does the delta snapshot go?**
> ⊘ **SUPERSEDED IN PART 2026-09-25 (`v3-diff`, owner design + COMMIT-ON-ACK):** the walk stays
> asynchronous as recommended, but "every report is a full report, diffed against the ledger" is
> replaced — the kernel diffs against the placements the host confirmed (committed per run from
> the host's verdict) and reports only the diff; the CPU ledger is gone. See `V3_BUILD.md`'s
> amended rule and `V3_P5_PORT_MAP.md` Q8.
- `walk.rs:640/660` calls `cuCtxSynchronize` on the caller's stack. The walk kernel also still carries the delta-snapshot handshake (`walk.rs:699-720`, `KFWR_HF_RESYNC`), which `V3_BUILD.md` rules out.
- **Recommend:**
  - One VA-manager thread owns the walker context.
  - It launches the walk, and `cuLaunchHostFunc` writes an eventfd into the worker's epoll set. The walk is then *"one more epoll entry"* (§5).
  - Every report is a **full** report, diffed against the ledger.
  - Delete `ack` and the snapshot buffers (this also frees ~64 MiB of device memory, per `walk.rs:123`).
- **Verify** the §5 deadlock caveat: a guest channel stalled on a fault must not block the walker's context. Gate 8 should run one walk while a faulted host channel exists.

**Q7. Coherence between guest CPU page-table writes (through BAR2 write-combined views) and the GPU walker.**
- The guest does `osFlushCpuWriteCombineBuffer` + `kbusFlush` before invalidating (`kern_bus_gm107.c:2797-2808`). That flushes the **CPU's** write-combine buffers, but our host BAR1 writes are PCIe-posted.
- **Recommend:** before launching a walk, the worker does **one uncached read through any store view** (a non-posted read orders the prior posted writes). This costs about 1 µs, off the vCPU.
- The guest's `kbusFlush` register writes stay "complete immediately" in the shadow. Gate 9's falsifier decides whether that is enough.

**Q8. The two docs disagree on the P4 old-tree files.**
- `V3_BUILD.md` "Dropped outright" lists `mmuinval/bar2/setpagedir/fbwin`. `V3_P2_PORT_MAP.md` §1 says *"defer to P4"* / *"P4 ADAPT"*.
- **Recommend:** amend `V3_BUILD.md`. The **pure decoders** are copied (`mmuinval.rs:241-276`, `bar2.rs:256-278`, `setpagedir.rs:192-237`, `fbwin.rs:106-186`; ~200 lines). Everything that records, drains, walks or stores is dropped.
- This is the same correction V3_P2 §4.1 made for `sweep.rs`.

**Q9. The P4 gate as written cannot pass in P4** (§1.2).
- **Recommend:** restate P4's gate as kf-gate8/9/10 plus the scrub-wall boot observable. Move the six arms to the P5/P6 gates with a "P4 code unchanged" clause.
- The same shape as `V3_P2_PORT_MAP.md`'s *"the P3 gate can't pass on P3 code alone"*.

**Q10. What happens to a mirror when a VAS acquires a root with no invalidate?**
- Cases: `COPY_SERVER_RESERVED_PDES` at construct time, `SET_PAGE_DIRECTORY`, and our fn 70.
- **Recommend:** a root change schedules a full walk of that object on the worker. The guest-visible RPC reply is held until the reconcile lands. This is the RPC-map sync point (`the_three_synchronization_points` #2; the held reply kept in kf-gsp) and it keeps §49.1 true for the RPC path.

**Q11. What memory type do the device-VMA memslots get?**
- The guest maps BAR1 write-combined. For a PFNMAP'd, non-RAM PFN, KVM's EPT memory type is likely UC, which would silently demote write-combining.
- **Recommend:** measure it in gate 9 (a streaming-write MiB/s through the HVA from a guest-shaped accessor). Treat it as a performance question for the LLM lane, not a correctness one. Not a P4 blocker.
