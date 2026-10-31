# V3 — guest RAM is a `translate(gpa, len)` under a layout guard, not a memfd

**STATUS: LIVE (code), 2026-10-31 (owner direction 2026-10-31); the file-less path and the layout guard are UNTESTED ON HARDWARE.**
Measured here: the GPU-free tests of `kf-linux-raw`, `kf-mem` (including the model host RM in `kf_mem::sim`) and `kf-qemu`,
and `kf3.c` / `kf3.h` against the QEMU 10.2.4 headers with `gcc -fsyntax-only` (compile only: no QEMU binary was built or
run, the new C verbs were never called). NOT measured: any GPU, any VM, any QEMU without a memfd, the C hold verbs at run time.
Defaults for a memfd-backed QEMU are unchanged and no performance change is claimed.

**Correction folded 2026-10-31 (this revision, `claude/guestmem-translate-only-20261031`, over the #211/#212/#213 text):**
1. `translate()` no longer returns `HostBlock { hva: u64, len: u64 }`. A VMM address as a plain integer in safe code is a
   pointer with the checks filed off; the owner's rule is that **no VMM address, integer or pointer, ever crosses from safe code
   into unsafe code, and no unvalidated offset + length either.** `translate()` now returns `RamSegment<'g>` objects
   (§2) borrowed from a `LayoutGuard`, plus the `LayoutEpoch`.
2. The agnostic part knows no fd: `GuestMemory::export_fd` and `FdRange` are gone. The memfd is a fact of the QEMU adapter
   (`RamMap::backing_fd`, `file_range`), used only by the stitch and by the paths in §6.
3. The RAM-offset numbering of block objects is the **guest-physical address** (`RamPlan::by_gpa`), not a cumulative offset,
   so adding or removing one block never renumbers another (§5). The cumulative form of #213 renumbered every block after a
   removed one.
4. A layout change (hot-unplug, `virtio-mem`, a resize) is handled, not refused for ever: the objects of a block that went
   have their GPU-VA slices unmapped, then are freed; a layout that moves *during* a descriptor call is refused by name
   `LAYOUT_CHANGED` (§4, §5).
5. The brief for this revision offered "QEMU RCU read lock + `MemoryRegion` reference" as the guard. The guard takes the
   `MemoryRegion` reference only; the RCU read lock is deliberately NOT taken, with evidence (§4).
6. (PR 2 of this series) CPU reads and writes of guest RAM that needed a file offset (`at_file_offset`, `dma_to_file_range`
   detours in `chan.rs`: Translated-channel reads, completion probes, USERD snapshots, the sysmem USERD relay) now go through
   `translate()` with a short layout guard (§6). What remains file-bound is GPU-side or an aliased mapping, never a CPU access;
   the table and the PRAMIN evidence are in §6.

## 1. The rule

Guest RAM must NOT be assumed to be a memfd. A hypervisor is not required to back RAM with a file. All kayfabe
needs from a VMM is one VMM-agnostic function:

```
LayoutGuard::translate(gpa, len) -> Result<Translated<'g> { epoch, segments: Vec<RamSegment<'g>> }, TranslateError>
```

the parts of the VMM's own mapping that back the guest-physical range `[gpa, gpa+len)`, in order, and the layout epoch the
answer holds for.

- A range inside one RAM block is ONE segment.
- A range that spans blocks returns one segment per block touched, in guest-physical order. Segments of different blocks are
  **never merged** (#211 merged host-adjacent blocks; only the VMM can vouch that two mappings stay together, and merging
  needed the host address as an integer).
- A hole is an error that names the first unbacked address; nothing is clamped.

## 2. The contract and the safe/unsafe boundary

| point | decision |
|---|---|
| `RamSegment<'g>` | defined in `kf-linux-raw/src/mapping_unsafe.rs` next to `Mapping`/`MappedRegion` (`THE_CONSTRAINTS.md` §13: private fields, one constructor). Private `base: NonNull<u8>`, `len`; **type invariant**: `[base, base+len)` is mapped read-write for as long as `'g`, the lifetime of the layout guard its constructor borrowed. No getter for the address, no `as_ptr`, no `Deref`; `tests/ui/ram_segment_has_no_address.rs`, `..._fields_are_private.rs`, `..._cannot_be_forged.rs` pin it. |
| constructors | ONE from an address: `unsafe fn RamSegment::from_raw(base, len, &'g guard)` (SAFETY: the guard keeps the block alive), called once, in `kf-qemu`'s `QemuLayoutGuard::translate`. One safe constructor over a `&'g [AtomicU8]` (`over_cells`) for model hypervisors and tests (the borrow is the proof). |
| offsets | `HostOffset` + `bounds::checked_span`: `slice(off, len) -> Result` (zero length, overflow and out-of-range are errors; `u64::MAX` tested), `read_into`, `write_from` re-check every `(offset, len)` inside the method that uses it. No unvalidated offset + length reaches an `unsafe` block. |
| the RM call | `HostRm::alloc_os_descriptor_ram(&RamSegment)` (`kf-host`): the segment lends a bounded `MappedRegion` for the length of a closure (`with_region`); the region is a no-op on drop, offers no address, and cannot outlive the closure. A segment that is not whole host pages is refused (`RAM_SEGMENT_UNALIGNED`). The RM pins the pages and keeps the page array, **not the address** (`os-mlock.c:216-254`). |
| geometry | `kf_mem::guestmem::RamLayout` is pure `(gpa, len)` per block (sorted, `len > 0`, no overflow, no overlap, at most 65 536 blocks). `locate(gpa, len)` gives `BlockRun { index, off, len }` (private fields, proved `off + len <= block.len`) and `translate_blocks` turns runs into segments by `RamSegment::slice` of the adapter's per-block segment, so the offsets are proved against the segment itself, not trusted from the layout. |
| `len == 0` | `TranslateError::ZeroLength` (a zero-length access names no byte). |
| `gpa + len` overflows | `TranslateError::Overflow`. |
| a byte no block backs | `TranslateError::Unbacked { gpa }`, the FIRST such address; no partial answer. |
| bound | at most `MAX_FRAGMENTS = 8192` segments per call, else `TooFragmented`; `O(log blocks + blocks touched)`. |
| the guard | `GuestMemory::lock_layout() -> Guard`; `LayoutGuard::{epoch, still_current, layout, translate}`. A segment borrows the guard. |
| the epoch | `LayoutEpoch(u64)`, monotonic: bumped by every RAM block add, removal or resize the VMM reports (QEMU: the registry generation, bumped by the memory listener's `region_add`/`region_del`). |

Hostile guest: `gpa` and `len` come from guest page tables. Every sum is `checked_*`/`saturating_*` (the crate denies
`arithmetic_side_effects`, indexing, `unwrap`, `panic` outside tests); a `u64::MAX` gpa or len returns an error.

## 3. How it is tested (GPU-free)

`cargo test -p kf-mem --lib guestmem` (geometry), `... ramobj` (numbering, book), `... sim` (model host RM),
`cargo test -p kf-linux-raw` (the segment, the compile-fail rows), `cargo test -p kf-qemu --lib` (the guard over the real registry).

1. **Per-byte model oracle, exhaustive**: every layout of 1 to 4 blocks with gaps 0..2 and lengths 1..3 at two start offsets
   (more than 10 000 layouts, more than 500 000 `(gpa, len)` queries) against `locate`; a second, block-walking oracle agrees.
2. **Seeded random**: 4 000 random layouts on an 8-byte grid (tiny blocks, holes, adjacent blocks, unaligned starts).
3. **Huge addresses**: layouts at the top of the address space (a block ending exactly at `u64::MAX`), lengths near `u64::MAX`.
4. **Named edges**: one block; a whole block; block start/end; the last byte of RAM; the first byte after a hole; through a
   hole (names the first unbacked byte); start in a hole; past all RAM; empty layout; two and three blocks; unaligned.
5. **Hostile cross product**: 17 extreme values squared and 20 000 random pairs over five layouts: no panic, every `Ok` well
   formed, every error names an address inside the range, the vector capacity stays bounded.
6. **The cap**: exactly 8 192 runs served; 8 193 refused; a 65 536-block layout asked for everything is refused, not truncated.
7. **The segments**: through a model hypervisor (`guestmem::testvmm`, plain Rust memory, an epoch that moves on add/remove/resize,
   a guard that holds the memory of the layout it saw), what the segments read and write equals what the model says is at
   those guest-physical addresses, across one, two and three blocks, at the edges; a segment cannot be narrowed out of bounds;
   an adapter whose per-block segment is the wrong length is refused (`Inconsistent`).
8. **The real registry** (`kf-qemu`): the guard holds every owner once and releases it on drop (also when it is refused), does
   not hold the registry lock (a removal goes through while a guard lives), notices the epoch move, and refuses overlapping
   blocks and a missing hold verb by name.
9. **Layout changes** (`kf_mem::ramobj` with the model VMM; `kf_mem::sim` with the model host RM; `kf-qemu` with the real
   registry): a layout that moves between `translate` and the end of the descriptor call is refused as `LAYOUT_CHANGED`, what the
   call made is freed, nothing is committed and the epoch does not advance; the next sync succeeds. A removed block has its
   slices unmapped BEFORE its object is freed (the model host RM records a violation if a freed object still backs a mapping);
   its kept neighbours keep their objects, numbers and mappings; a resize rebuilds the object over the new length and the base
   does not move; an unmap that fails keeps the object and the next sync retries.

## 4. The QEMU adapter: the guard (`kf-qemu/src/raw_unsafe/ramguard_unsafe.rs`)

`RamMap` (kf-qemu `mem.rs`) keeps the blocks QEMU registered (`RamBlock { gpa, mem: RawRegion, fd, fd_off }`) and a
generation counter; `ram_add`/`ram_del` bump it under the write lock. It implements `GuestMemory`.

**The hold.** QEMU frees a RAM block through `call_rcu(reclaim_ramblock)` (`system/physmem.c`, `qemu_ram_free`, called from
`memory_region_destructor_ram`, `system/memory.c:1179`) only when its `MemoryRegion` is finalized, and `memory_region_ref` is
`object_ref(mr->owner)` (`system/memory.c:1849-1864`), so a held owner reference keeps the block. `QemuLayoutGuard::take` is given
the registry's READ guard (so the blocks are registered *now*, which is when a reference on their owners is valid), takes a hold
on each owner (`MrHold`; production `CMrHold` = the two C verbs), copies the regions and **releases the registry lock**: a
hot-unplug is never blocked behind a multi-second host call, it just cannot free the memory while the guard lives. Dropping the
guard releases the holds. `translate` makes each segment with `RamSegment::from_raw(.., self)`, one `unsafe` block whose SAFETY
cites the hold. `still_current()` re-reads the generation.

**C side, ABI 31** (`qemu/hw/misc/kf3/kf3.{h,c}`, `ffi_unsafe.rs`): `kf3_ram_add` gains `mr` (the section's `MemoryRegion *`,
opaque to Rust; stored in the block's `RawRegion` as an `Owner` token with no getter), and `kf3_ram_ops(h, &Kf3RamOps)` registers
`memory_region_ref` / `memory_region_unref` once, before the listener registers. Both new C functions are three lines.

**Why no RCU read lock** (the brief offered it; checked against the 10.2.4 tree):
- `include/qemu/rcu.h`: "Each thread containing read-side critical sections must be registered with `rcu_register_thread()`":
  a Rust-created thread (the VA thread) would need a C helper and a registration per thread; an unregistered thread's read
  section is invisible to `synchronize_rcu` (it protects nothing).
- A read-side section as long as the host RM's pin (seconds for all of guest RAM) stalls every `synchronize_rcu()` /
  `drain_call_rcu()` caller, among them QMP `device_add` and `device_del` on the main loop with the BQL held
  (`system/qdev-monitor.c:875`, `:1021`) — exactly the hot-plug this guard exists for.
- The `MemoryRegion` reference alone keeps the block alive (above), is atomic and callable from any thread, and does not
  involve the grace period. `memory_region_unref` from a non-BQL thread may finalize the owner there if the VMM dropped its own
  reference meanwhile; that is the standard `address_space_unmap` situation and is **UNVERIFIED on hardware** (NEEDS-OWNER).

**ALTERNATIVE (design note, not code): an owned persistent mapping.** If the hold above proves unusable (e.g. owner finalization
off the main loop is unacceptable), the adapter can instead `dup` the block's fd (`qemu_ram_get_fd`) and map the WHOLE block once
at block add with the existing `MappedRegion` machinery (address space is cheap), serving segments from that mapping: stale then
only means "no longer guest RAM", never a use-after-free, and the epoch handles identity. It needs the memfd that
`docs/howto/requirements.md` already requires, and it would map the memfd at a second address, which the owner's direction
(2026-10-31, (1)) says not to do in the hot path; it stays a note.

**Unmapped from this note:** page identity. The RM pins the page array; a balloon or free-page-hint discard that replaces a page
under a pinned descriptor is independent of the VMM address and of any memfd. kayfabe relies on the guest freeing pages only after
its own invalidate completes (UNVERIFIED by contract; NEEDS-OWNER). `kf3.c` holds `ram_block_discard_require(true)` (it wants
discard to stay allowed: it refuses VFIO-like devices); this revision does not change that.

## 5. The GPU side: numbers, one object per block, the book

**With a memfd (unchanged by this revision):** the whole file is ONE host object (`MemPlane::guest_ram_object`), a sysmem row's
`Desired::off` is a file offset, the batched map stitches pieces of the file (`MappedRegion::stitch`) into one object.

**Without a file:**
- *Numbering.* `Desired::off` of a guest-RAM row is the **number** of its first byte: its block's base number plus the offset
  in the block. The base is chosen by the adapter and must not depend on the other blocks: `RamPlan::by_gpa` numbers a byte by
  its own guest-physical address (a hole costs nothing, a layout change renumbers nothing). A range across two blocks has one
  number only if the blocks are adjacent in both guest-physical space and numbering; otherwise it is refused by name where
  leaves are classified.
- *Objects.* `RamObjectBook` (`kf_mem::ramobj`) keeps ONE `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` per block, each over the whole
  block's segment (`HostRm::alloc_os_descriptor_ram`), on the VA thread. A row is mapped as an offset + length slice of its
  block's object (`RamObjects::resolve`); a row across an object edge is cut there (`SpaceVerbs::ram_cuts`, one host map per
  object, all or nothing); the model host RM records a violation for any map reaching past its object.
- *Sync.* `RamObjectBook::sync(vmm, number, host, unmap)` takes ONE guard for `translate + descriptor creation`: it compares the
  layout with the blocks that have an object; for each block that went (or changed length) it calls `unmap` — the caller takes
  down every GPU-VA slice backed by its numbers (`kf-qemu`: `resync_ram` unmaps every mirror row overlapping the block, deferred,
  one invalidate per mirror) — and only then frees the object; for each new block it translates the whole block under the same
  guard, makes the object, and **re-checks the epoch after the host call**: a moved layout frees what the call made and the call
  is `LAYOUT_CHANGED` (never retried silently); only then does it commit the table (`RamObjectCell`, read by every space) and the
  epoch. The VA thread calls it when `MemPlane::ram_layout_moved()` (never while a walk is in flight).
- *The stitch is optional.* `GpuMirror::map_batch` answers `kf_mem::ledger::NO_RAM_FD` without a file; the apply layer counts it
  apart (`Applied::batch_no_fd`) and places the rows per run. The stitch itself (`MappedRegion::stitch`,
  `HostRm::map_scattered_through`) is untouched: the owner's A/B (`KF3_NO_BATCHED_MAP=1` per run vs batched) decides its future.

## 6. Who touches guest RAM, and how (inventory, 2026-10-31)

Every consumer of guest RAM is exactly one of: a **CPU read**, a **CPU write**, an **RM-side reference** (object + offset
handed to the host RM, no CPU access), a **GPU-VA mapping** (a map of a host object into a GPU VA space), or a **mapping
aliased into a BAR window** (the guest CPU reaches guest RAM through a window of the device).

| consumer (file) | kind | how it reaches guest RAM now |
|---|---|---|
| Translated channel: GPFIFO entries, push-buffer words, completion probes (`chan.rs` `Mem::read`, `read_ram_va`, `probe_read`, `probe_side`) | CPU read | `RamMap::read_row` / `load_row_u32`: placed-row number -> gpa -> `translate` (any numbering, any number of blocks, no file) |
| device-DMA reads: `probe_operand`, PT-SNAP USERD snapshot, error-notifier dump | CPU read | `RamMap::read_dma` -> `dma_to_gpa` (THE vIOMMU seam for CPU access) -> `translate` |
| sysmem USERD: relay `GP_PUT` in, `GP_GET` out (`UserdView::Ram`) | CPU read + CPU write, 4-byte words | `RamMap::load_u32_gpa` / `store_u32_gpa` per access (`Relaxed` atomic word); the view holds a gpa, never a host pointer, so a removed block is a named refusal |
| sysmem USERD / error notifier adopted by the host-RM channel (`UserdAt::Ram`, `err_at`) | RM-side reference | unchanged: object + FILE offset of the whole-RAM object (needs the memfd object) |
| GR-tier `GP_GET` word address (`windows(..).ram(off, 4)`), `Rows::dma_to_file_range` in the T-mode rewriter | GPU-VA address arithmetic | unchanged: file offset into the T-space window |
| T-space RAM window (`tspace.rs`) | GPU-VA mapping (the whole-RAM object at `ram_base`) | unchanged: needs the whole-RAM object |
| PRAMIN pointed at system memory (`ViewOps::place_pramin_ram`) | mapping aliased into the BAR0 PRAMIN window | unchanged: `mmap(MAP_FIXED)` of the memfd into the window; without a memfd refused by name and counted (`Repointed::refused`) |
| BAR1/BAR2 sysmem leaves (`cpuwin`) | mapping aliased into a BAR window | REFUSED by name, counted (#212) |
| display, GSP queue, MemoryList reads (`block_for`) | CPU read/write | unchanged: already file-less |

**CPU access through `translate()`.** Each access takes a SHORT layout guard (`RamMap::lock_layout_short`, `QemuLayoutGuard::take_locked`):
the registry read lock is kept for the length of one bounded copy (at most `RamMap::CPU_COPY_MAX`), so a block removal waits
microseconds instead of racing it, and no owner hold is taken (a hold per 4-byte USERD access would be two atomic RMWs per block).
The long guard of §4 is for the host RM call that pins all of a block.

**The two that cannot read through `translate()` — and the evidence (NOT guessed):**
- *BAR1/BAR2 sysmem leaves*: decided in #212 (ogkm: sysmem is CPU-mapped directly by `osMapSystemMemory`; BAR1/BAR2 never map it).
- *PRAMIN -> sysmem* (an aliased mapping: the guest's own CPU reads and writes guest RAM through the window; no host-side
  access can replace it). ogkm 580.65.06: `kbusSetBAR0WindowVidOffset_GM107` ALWAYS writes `_TARGET _VID_MEM`
  (`kern_bus_gm107.c:4747`) and is the only window setter on the normal path; the target field is written with a derived
  aperture only in the BAR2 self-test `kbusVerifyBar2_GM107` (`:4067-4208`), with a sysmem memdesc only when
  `PDB_PROP_GPU_IS_ALL_INST_IN_SYSMEM` is set (registry `RMInstLoc*` overrides, `gpu_registry.c:309-333`; a non-default test
  configuration) — its callers pass NULL (`kern_bus_gm107.c:360,651,678`); Hopper+ has no target field (`kern_bus_gh100.c:206`);
  and `gpu_registry.c:337-343` says reflected BAR mappings deadlock Ampere+. So **in the open driver with default registry the
  PRAMIN window never targets system memory.** What the evidence cannot settle: the closed Windows driver (no evidence is taken
  from it) and hardware acceptance. **Decision: PRAMIN keeps its behaviour (NEEDS-OWNER)**; if the owner accepts the open-driver
  evidence alone, refusing it by name is one arm in `PraminPool::repoint` plus the trait method (as #212 did for the BARs), and a
  counter says whether it ever fires.

What still needs the memfd after this change: the RM-side USERD / notifier reference, the T-space window and its address
arithmetic, and PRAMIN -> sysmem (all GPU-side or aliased, none a CPU access). They refuse by name without one.

## 7. What this note does not claim

- No performance change; nothing in this note was run on a GPU or a VM.
- That an OS descriptor over an anonymous or hugetlb mapping works exactly like the one over the memfd view (same ioctl; never issued).
- That QEMU re-adds anonymous RAM at the same addresses after a machine reset (UNSURE); a changed layout is now handled by the
  book rather than refused for ever, but the path has run only in the model.
- That the C hold verbs behave as read from the source: they were compiled (syntax) against 10.2.4 and never run.
