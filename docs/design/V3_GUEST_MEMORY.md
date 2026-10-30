# V3 — guest RAM is a `translate(gpa, len)`, not a memfd

**STATUS: DESIGN-ONLY, 2026-10-31 (owner direction 2026-10-31).** Step 1 (`kf_mem::guestmem`: the type, the
contract, the tests) is code; nothing consumes it yet. This note becomes LIVE when step 3 lands (the QEMU RAM
source implements the trait and the batch path selects on the optional fd capability). Until then the sections
marked *today* describe what the tree still does.

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

## 4. Where it plugs in (*today*, before step 3)

*Today* the system numbers guest RAM by the memfd file offset. `RamMap::file_range` (kf-qemu `mem.rs`) gives it, the
VA manager turns a sysmem leaf into `Desired { ram: true, off: file offset }`, the whole-guest-RAM OS descriptor is
built by mapping the entire memfd, and `RamMap::block_for` accepts only a range inside ONE block. Step 3 makes `RamMap` implement `GuestMemory` over the
blocks QEMU already registers (`RamBlock { gpa, mem, fd: Option<_>, fd_off }`, `ram_add`'s `fd < 0` case).

## 5. The optional capability: the stitch

`kf_linux_raw::MappedRegion::stitch` (via `kf_host::HostRm::map_scattered_through`, called from `kf_mem::ledger` and
`batch.rs`) builds ONE host view out of pieces of the guest memfd and describes it to RM with ONE OS descriptor. It is
the only consumer that needs a fd. The owner believes it brings no map-side performance win; an A/B will decide
(`KF3_NO_BATCHED_MAP=1` per-run path vs the default batched path). It is NOT deleted. When the fd capability is absent
the batch path falls back, by name, to the existing per-run path. See the step-3 PR for the exact fallback.

## 6. What this note does not claim

- No performance change. Defaults stay as they are for QEMU (memfd present, behaviour unchanged).
- Nothing in this note was run on a GPU or a VM. The tests above are the GPU-free tests of the pure type.
