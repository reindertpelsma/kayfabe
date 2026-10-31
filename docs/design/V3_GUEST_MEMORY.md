# V3 — guest RAM is a `translate(gpa, len)`, not a memfd

**STATUS: LIVE (code), 2026-10-31 (owner direction 2026-10-31); the file-less path is UNTESTED ON HARDWARE.** Step 1
(`kf_mem::guestmem`: the type and the contract) was DESIGN-ONLY until step 3 landed. Step 3 (this note's §4, §5,
§6) is code: `kf-qemu`'s `RamMap` implements `GuestMemory`; the GPU-side per-run map carries guest RAM as one host
object per RAM block when there is no file; the batched map falls back to the per-run path by name. Measured here:
the GPU-free tests of `kf-mem` (the model host RM), `kf-linux-raw` and `kf-qemu`. NOT measured: any GPU, any
VM, any QEMU without a memfd. Defaults for a memfd-backed QEMU are unchanged, and no performance change is claimed.
**Correction folded 2026-10-31 (step 3):** the first version of this note said the per-run path builds "the
descriptor" from the `translate()` blocks; one OS descriptor describes ONE contiguous address range, so there is
one object per RAM block (§5), not one descriptor.

## 1. The rule

Guest RAM must NOT be assumed to be a memfd. A hypervisor is not required to back RAM with a file. All kayfabe
needs from a VMM is one VMM-agnostic function:

```
translate(gpa, len) -> Result<Vec<HostBlock { hva: u64, len: u64 }>, TranslateError>
```

the host (VMM address space) blocks that back the guest-physical range `[gpa, gpa+len)`, in order.

- A range inside one RAM block is ONE block.
- A range that spans blocks, or whose blocks are not contiguous in the VMM's address space, returns several, in
  guest-physical order. Contiguity in the VMM VA is NOT guaranteed; access is possible block by block. Fragmented
  output is the edge case, it exists for correctness, and it is tested as thoroughly as the common case.
- A fd is an OPTIONAL capability (`export_fd`), used only by the GPU-side stitch (section 5). Nothing else may
  require it.

## 2. The contract (`crates/kf-mem/src/guestmem.rs`)

| point | decision |
|---|---|
| `HostBlock` | `{ hva: u64, len: u64 }`, plain integers. An address becomes a pointer only in `*_unsafe.rs` (kf-linux-raw, kf-qemu, kf-cuda). |
| input | `RamLayout::new(&[RamBlockSpec { gpa, hva, len }])`: sorted by `gpa`, `len > 0`, no `gpa+len` or `hva+len` overflow, no guest-physical overlap, at most 65 536 blocks. Anything else is refused by name and index (`LayoutError`). Host ranges MAY alias or overlap (a VMM can map one host block at two guest addresses). |
| `len == 0` | `TranslateError::ZeroLength`. A zero-length access names no byte; an empty answer would hide a caller defect. (Chosen over "empty vec"; tested.) |
| `gpa + len` overflows | `TranslateError::Overflow { gpa, len }`. The exclusive end must be representable: a range ending at exactly 2^64 is refused even if its last byte is backed (no RAM block can end above `u64::MAX` either). |
| a byte no block backs (hole, MMIO, beyond the end) | `TranslateError::Unbacked { gpa }` naming the FIRST such guest-physical address, not the end of the hole. No partial answer is returned. |
| merging | Adjacent guest-physical blocks whose host ranges are also adjacent (`hva_a + len_a == hva_b`) are returned as ONE `HostBlock`. Adjacent but host-discontiguous (including host order reversed) stay separate. No two neighbours of an `Ok` answer are mergeable. |
| bound | At most `MAX_FRAGMENTS = 8192` blocks per call, else `TooFragmented` (nothing truncated). A caller cannot make `translate` allocate more than `8192 * 16` bytes, whatever `gpa`/`len` it passes. Cost is `O(log blocks + blocks touched)`. |
| optional fd | `export_fd(gpa, len) -> Option<FdRange { fd: BorrowedFd, offset }>`, default `None`. `Some` only when the whole range is one contiguous range of one file. `None` is the normal answer, never an error. |

Hostile guest: `gpa` and `len` come from guest page tables and descriptors. Every sum is `checked_*`/`saturating_*`
(the crate denies `arithmetic_side_effects`, indexing, `unwrap`, `panic` outside tests); a `u64::MAX` gpa or len
returns an error. `kf_mem::guestmem` tests cover that.

## 3. How it is tested (GPU-free, `cargo test -p kf-mem --lib guestmem`)

1. **Model oracle, exhaustive**: a per-byte model (map every guest-physical byte to its host byte by linear scan, run-length
   encode consecutive host bytes) compared with `translate` on every layout of 1 to 3 blocks over small parameter sets
   (gap/no gap, length 1-2, host-contiguous / far / aliased) and every `(gpa, len)` over and just past the layout.
2. **Model oracle, seeded random**: 4 000 random layouts on an 8-byte grid (tiny blocks, holes, adjacent-and-contiguous,
   adjacent-but-host-discontiguous, host-reversed, aliased, unaligned starts) with queries biased to block edges.
3. **Huge addresses**: a second, block-walking oracle (different structure from the implementation) on layouts at the
   top of the address space (including a block ending exactly at `u64::MAX`) with lengths near `u64::MAX`.
4. **Named edge cases**: one block; whole block; starts at a block start; ends at a block end; last byte of RAM; first
   byte after a hole; range through a hole (names the first unbacked byte); start in the hole; past all RAM; empty
   layout; merge; no merge; reversed host order; three blocks; aliases; unaligned; zero length; overflow.
5. **Hostile**: a cross product of 17 extreme values and 20 000 random pairs over five layouts: no panic, every `Ok` is
   well-formed, every error names an address inside the range, the vector capacity stays bounded.
6. **The cap**: exactly 8 192 fragments served; 8 193 refused; merged neighbours do not count; a 65 536-block layout
   asked for everything is refused, not truncated.

## 4. Where it plugs in

`RamMap` (kf-qemu `mem.rs`) keeps the blocks QEMU registered (`RamBlock { gpa, mem, fd: Option<_>, fd_off }`;
`ram_add` with `fd < 0` already meant "no file", so no QEMU C change is needed). It now implements `GuestMemory`:
`translate` builds a `RamLayout` of plain integers from the blocks (the VMM address is `RawRegion::base_addr()`, an
integer) and answers; a registry that is not a layout (overlapping blocks) backs nothing, at the first byte.
`export_fd` is `file_range` as a `FdRange` (None without a file, across a block end, in a hole, for length 0).
The existing consumers (`block_for`, `at_file_offset`, `dma_to_file_range`) are NOT moved to `translate` in this series;
they still need one block or a file offset, and refuse by name otherwise.

## 5. The GPU side: RAM offsets, one object per block, the optional stitch

**With a memfd (unchanged):** the whole file is ONE host object (`MemPlane::guest_ram_object`), a sysmem row's
`Desired::off` is a file offset, the batched map stitches pieces of the file (`MappedRegion::stitch`) into one object.

**Without a file (new):**
- *Numbering.* `Desired::off` of a guest-RAM row is a **RAM offset**: the position among the bytes of guest RAM in
  guest-physical order (`kf_mem::ramobj::RamPlan::ram_offset`, reached by `RamMap::gpu_ram_offset`). Blocks adjacent
  in guest-physical space are adjacent in RAM offsets; a hole costs nothing; a range across a hole is not RAM and is
  refused by name where leaves are classified (`desired_from_leaves`).
- *Objects.* `MemPlane::guest_ram_objects` makes one `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` per RAM block, each over
  that block's VMM mapping (`RawRegion::describe_range` -> `MappedRegion::adopt_foreign`, an unsafe constructor
  in `kf-linux-raw` that adopts memory it never maps or unmaps; `reprotect`/`request_huge_pages` refuse on it), on
  the VA thread, once. `RamObjects` is the table (RAM offset -> object, object offset).
- *The cut.* A host map can only carry bytes of its own object, so a guest-RAM row that spans an object edge is cut
  there: `SpaceVerbs::ram_cuts` -> `BatchedVas::map_segments` maps one piece per object (all or nothing, each piece
  recorded and unmapped by its own start, like the pieces of a row across a reservation edge). The model host RM
  records a violation for any map that reaches past its object's end (a test of the model itself covers it).
  `HostVas::map` refuses a row that is not inside one object, by name (`Crosses`, `Uncovered`).
- *The stitch is optional.* `GpuMirror::map_batch` answers `kf_mem::ledger::NO_RAM_FD` when there is no file; the
  apply layer counts it apart (`Applied::batch_no_fd`, `VaStats::batch_no_fd`), names it once in the log, and places
  the rows per run. It is not a refusal and not in `batch_fallbacks`. The stitch itself (`MappedRegion::stitch`,
  `HostRm::map_scattered_through`) is untouched: the owner's A/B (`KF3_NO_BATCHED_MAP=1` per run vs batched) decides
  its future.
- *Topology.* The objects describe the addresses the blocks had when they were built. `MemPlane::ram_objects()` is
  `None` once the registered layout differs (a RAM block added or removed afterwards): rows are then refused by
  name until a restart. QEMU re-renders RAM at machine reset; with a memfd the file survives that, with VMM memory
  it may not. UNSURE whether QEMU re-adds anonymous RAM at the same addresses; the conservative answer is refuse.

## 6. What still needs a file (and refuses by name without one)

Everything that numbers guest RAM by FILE offset: `RamMap::file_range`/`at_file_offset`/`dma_to_file_range`
consumers in `chan.rs` (a Translated channel reading guest memory by gpa, sysmem USERD and notifiers), the T-space
RAM window (`tspace.rs`, one object over all RAM), PRAMIN pointed at system memory (`ViewOps::place_pramin_ram`),
the display and GSP queue reads through `block_for` still work (they use the VMM mapping, not a file). Moving the
channel-plane readers to `translate` is the next step if file-less RAM is to be a supported product configuration;
this series makes the GPU-side map capable of it and nothing more.

## 7. What this note does not claim

- No performance change. Defaults stay as they are for QEMU (memfd present, behaviour unchanged).
- Nothing in this note was run on a GPU or a VM. The tests are GPU-free: the pure type (§3), the RAM-object table and
  plan (`kf_mem::ramobj`: per-byte model, plan + table vs `translate`), and the model host RM (`kf_mem::sim`: a row across an
  object edge, across three objects, across a hole; file-backed vs fragmented give the same translation at every step; the
  random-diff property on fragmented file-less RAM with unmap and mid-row map refusals injected).
- The real descriptor over VMM memory (an OS descriptor over an anonymous or hugetlb mapping rather than a memfd view) is the
  same ioctl but has not been issued; it pins every page for the VM's life, like the memfd object.
