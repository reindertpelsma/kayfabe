# V3 — batched guest-RAM maps: N scattered runs, O(1) host calls

**STATUS: LIVE, 2026-09-26 — CODE + MEASURED on `vh` (RTX 3060 GA106, 580.159.04).** Branch
`v3-batchmap` (off master `ce2cb06d`). Verb choice read from ogkm-580.159.04 source; §7 holds the
bench numbers and the revision they were measured at. Answers the follow-up `V3_BUILD.md` w829
named (*"each scattered 4 KiB run is ONE host map call on the guest-RAM object … a batched verb is
the next budget"*).

## 1. The problem, measured

A no-PM CUDA process in the fat guest maps a fragmented guest-RAM range: ~56 MiB VA-contiguous at
`0x2_0000_0000`, **12 288 runs of 4 KiB, 0 of them GPA-contiguous with their VA neighbour**
(w829 census). The walker emits one MAP per run; kf placed each as one fixed
`NV_ESC_RM_MAP_MEMORY_DMA` slice of the whole-guest-RAM OS descriptor, and removed each with one
`NV_ESC_RM_UNMAP_MEMORY_DMA` at exit.

Baseline `bm0_nopm` (rev `577cb1ee` = master + a timing line only), 24 processes, all PASS:
12 288 maps in 234–273 ms (19–22 µs/call), 12 288 unmaps in 1 028–1 516 ms (**84–123 µs/call**).

★ **The unmap cost is not a constant — it is O(mappings in the space).** Per-call cost follows the
space's mapping count: 3 767-run space 35 µs, 1 183-run space 39 µs, 12 288-run space ~100 µs. The
source says why: `serverInterUnmapInternal` walks the mapper's **whole** `interMappings` list for
every unmap (`ogkm-580 src/nvidia/src/libraries/resserv/src/rs_server.c:2375-2427`) — one list
node per mapping in the `hDma`. 12 288 unmaps × a 12 288-node list is quadratic. ⇒ Fewer *mappings*
fixes the unmap side twice over: fewer calls, and each call walks a short list.

## 2. The verbs surveyed (ogkm-580.159.04), and why each was taken or refused

| candidate | what it does | verdict |
|---|---|---|
| `NV00FE` `NV_MEMORY_MAPPER` (`mem_mapper.c`) | a queue of map/unmap ops | ⊘ executes **one** `MapWithSecInfo` / `UnmapWithSecInfo` per op internally (`mem_mapper.c:62-150`) — saves only syscall crossings, not RM's per-mapping work or the O(M) walk; adds a semaphore surface. Async sparse-binding machinery, not a batch. |
| `NV01_MEMORY_LIST_SYSTEM` (page array) | a memory object from a PTE array | ⊘ `RS_FLAGS_ALLOC_PRIVILEGED` (`rmapi/resource_list.h:612-619`) and refused outside vGPU/GSP-client (`mem_list.c:86-100`). Takes physical addresses. |
| `NV0080_CTRL_CMD_DMA_UPDATE_PDE_2` | point a PDE at caller-owned page tables | ⊘ non-privileged (`g_device_nvoc.c` flags `0x8`) but it is the *externally managed VAS* model: we would write PTEs holding physical/IOVA addresses, which unprivileged userspace does not have. |
| `NV0080_CTRL_CMD_DMA_FILL_PTE_MEM` | fill PTEs from a page list | ⊘ declared (`ctrl0080dma.h:205`), **no dispatch entry** in 580's `g_device_nvoc.c`. |
| `NVOS32_DESCRIPTOR_TYPE_OS_PAGE_ARRAY` / `OS_PHYS_ADDR` / `OS_DMA_BUF_PTR` / `OS_SGT_PTR` | describe pages directly | ⊘ page array is only reached internally (a user VA is converted to it after `os_lock_user_pages`, `escape.c:133-166`); the others are kernel-client only (`osmemdesc.c:94-134, 143-148`). |
| ★ `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over a **stitched** user VA | one object whose page *i* is whatever user page *i* is | ✔ **taken** — the verb kf already uses unprivileged for guest RAM (R25). RM pins the pages of the VA range with `pin_user_pages(FOLL_LONGTERM)` (`kernel-open/nvidia/os-mlock.c:216-254`), across any number of VMAs, and keeps only the `struct page` array (`nv.c:3357-3400`; unpinned by page in `os-mlock.c:~325`) — **never the address**. So a VA range built from N scattered `mmap`s of the guest memfd becomes ONE object whose pages are the guest's own pages, in our order. |
| ★ `NV_ESC_RM_UNMAP_MEMORY_DMA` with `size != 0` | range unmap | ✔ **taken** — `size` = *"size to unmap, 0 to unmap entire mapping"* (`nvos.h:2195-2205`). With `size != 0` RM unmaps **every** mapping of the `hDma` intersecting the range, splitting a straddler (`rs_server.c:2388-2415`, `2310-2360`); `VirtualMemory` supports it (`virtmemIsPartialUnmapSupported` = `NV_TRUE`, `g_virtual_mem_nvoc.h:369-371`; body `virtual_mem.c:1681-1790`, whose partial-path precondition `pMapping->pMemDesc == NULL` holds because nothing writes `*ppMemDesc`, `rmapi/client.c:332`). A range intersecting nothing is `NV_OK` (`status` starts `NV_OK` for a partial unmap, `rs_server.c:2380`). |

## 3. Map: stitch → one descriptor → one fixed map

`kf_host::HostRm::map_scattered(space, fd, pieces, at, defer, kind)`:

1. `kf_linux_raw::MappedRegion::stitch(fd, pieces)` — a `PROT_NONE` reservation at a
   **kernel-chosen** address, tiled by one `MAP_FIXED | MAP_SHARED` per file-discontiguous piece at
   a cursor that only advances (no overlap, no hole — the pieces' lengths sum to the reservation).
   The only new `MAP_FIXED` in the tree; same argument as `Reservation::map_fixed_in` (§13: the
   address never leaves `kf-linux-raw`; `Indirect::describing` patches and scrubs it).
2. `alloc_os_descriptor(&view, 0, len)` — ONE `NV_ESC_RM_ALLOC_MEMORY`. The view is **dropped
   before any map exists** (RM holds the pages, not the VA).
3. ONE fixed `NV_ESC_RM_MAP_MEMORY_DMA` of the whole object at the first row's VA, 4 KiB-pinned
   (`MapBacking::SharedSlice`: a stitched object is contiguous at no bigger page), with the rows'
   (uncompressed) kind and `DEFER_TLB_INVALIDATION` — the entry's single invalidate follows as
   before.

**All or nothing.** `Ok` ⇔ every piece is placed at its VA. A refused map is rolled back by RM
(`virt_mem_allocator_gm107.c:1540-1557`), a relocated one is torn down by the existing placement
assertion, and the object is freed before the error returns. ⇒ the caller's contract is binary.

### 3.1 What is batched (`kf_mem::apply`)

After every per-run check (store/RAM bound, whole pages, extent, `reserved()` overlap, a refused
unmap underneath), the surviving map rows are sorted by VA and cut into maximal groups that are
VA-adjacent, all guest RAM, one kind, ≤ `BATCH_MAX_RUNS` (4 096). A group of ≥ 2 goes to
`MapTarget::map_batch`. Vidmem rows are never batched (they are slices of the store, already one
object). A lone run keeps the per-run verb.

### 3.2 Commit-on-ack is unchanged

- A batch `Ok` ⇒ every run in it is OUR placement ⇒ each acknowledged `APPLIED` — the host
  confirmed exactly those placements.
- A batch `Err` ⇒ nothing was placed ⇒ **its runs go one by one through the old per-run verb**,
  each with its own verdict (`APPLIED` / `HELD` for a VA host RM already holds / `FAILED`). So a
  batch never hides which run failed, and `HeldByHost` inside a batch is found the same way it
  always was. Counted as `batch_fallbacks`, never as a refusal.

### 3.3 The cap

The stitched view holds one VMA per file-discontiguous piece while the descriptor is built; Linux
caps a process at `vm.max_map_count` (65 530 default), QEMU's own VMAs included. 4 096 keeps the
transient cost small and the call count at ⌈12 288 / 4 096⌉ = 3 batches for the measured process.
A stitch the host refuses (e.g. a hugetlb memfd that cannot be mapped at 4 KiB offsets) is a batch
`Err` → per-run fallback, by name.

## 4. Unmap: one range per VA-contiguous stretch

`apply_entry` sorts the non-held UNMAP runs by VA and groups VA-adjacent ones; a group of ≥ 2 is
ONE `MapTarget::unmap_range(va, len)` = one `NVOS47` with `size = len`. The range is by
construction **exactly the union of committed placements being removed**, so the only mappings in
it are those. `GpuMirror::unmap_range` still refuses (→ fallback) a range that touches one of our
VMM placements (`reserved()`: windows, ring region) — belt and braces, since a guest row over one
is already refused at map time. A held run is never part of a range (it never reaches the host).

A refused range may have removed some mappings before RM stopped; the fallback re-issues the runs
one by one, and each per-run unmap of a batch piece is itself a range of that run's length, which
answers `NV_OK` when nothing is left there — so every run's final verdict states the end state.

A **single** run that is a piece of a live batch cannot use the legacy whole-mapping unmap (keyed
by the exact start: it would take the rest of the batch, or find nothing) — `BatchedVas::unmap_run`
asks the book and unmaps by range. Anything not in a batch keeps the legacy verb, unchanged.

## 5. The batch objects' lifetime (`kf_mem::batch`)

A batch object must live exactly as long as any piece of it is mapped: freeing it early makes RM
unmap every piece still in use (`_clientUnmapInterBackRefMappings`, `rs_client.c:1342-1395`);
never freeing it pins guest pages for the VM's life. `BatchBook` records, per host space, each
batch object `(start VA, handle) → live-page bitmap`, and `unmapped(va, len)` clears what WE
unmapped and returns the handles that just emptied — the caller frees them outside the lock.
Idempotent per page (a repeated or overlapping unmap cannot double-count); two batches may start
at one VA (a new batch over an old one's dead pages).

This is a ledger of **our own host handles** (the kind `V3_BUILD.md`'s 2026-09-25 amendment
sanctions) — never guest table content: nothing resolves through it. `PlacedRows` still records
one row per run, so a Translated reader resolves exactly as before.

Retire (`retire_mirror`): rows are unmapped as VA-contiguous ranges, then **every** remaining batch
object is freed (freeing unmaps any straggler), then the one invalidate. A space kept because a
live channel runs in it keeps its batches mapped — as it keeps its rows today.

## 6. Constraints, checked

- **Host verbs authored, never forwarded** — every flag word, offset, VA and piece list is ours;
  pieces come from `Desired.off`, which `desired_from_leaves` already bounded to the guest memfd.
- **The host RM is the ledger** — §3.2 / §4: verdicts only from host answers; `Err` means nothing.
- **No blocking on a vCPU / under a shared lock** — all of it runs where the per-run maps ran (the
  VA-manager thread); the book's mutex is VA-thread-only and never held across a host call.
- **VMM addresses never guest-visible** — the stitched VA exists inside one call, inside
  `kf-linux-raw`, and is scrubbed from the ioctl argument; the GPU VA is the guest's own.
- **§13** — the only new `unsafe` is in `mapping_unsafe.rs` (`kf-linux-raw`).
- **All families first-class** — nothing here is per-die: NVOS46/NVOS47 and the OS descriptor are
  family-independent RM API; the grain is 4 KiB on Turing … Blackwell; kinds are carried as before.
- **Off switch** — `KF3_NO_BATCHED_MAP=1` restores the per-run path (A/B, and an escape hatch).

## 7. Measured — `vh` (vast 52624429: EPYC 7452 KVM guest ⇒ nested, RTX 3060 GA106, 580.159.04)

Workload: `scripts/bench/uvm_reinit_box.sh` — fat guest (`KF_DEVICE=kf3`, 8 GiB, 4 vCPU), **no
persistence mode**, 24 torch processes in a row (`x=torch.ones(1<<20,'cuda'); (x*2).sum()`), each
a full adapter re-init. Parsed by the `mem large apply` / `census retire` lines (`KF_VAS_CENSUS=1`).

### 7.1 Before / after

| | before `bm0_nopm` (rev `577cb1ee`) | after `bm4_nopm` (rev `c0ebcda1`) |
|---|---|---|
| processes | **24/24** PASS | **24/24** PASS |
| map of a 12 288-run space: host map verbs | 12 288 | **3** (3 batches ≤ 4 096 runs) |
| … wall time (min / mean / max) | 234 / 265 / 273 ms | 286 / 352 / 409 ms |
| exit unmap of the same space: host unmap verbs | 12 288 | **1** (one range) |
| … wall time (min / mean / max) | 1 028 / 1 256 / 1 516 ms | **3 / 7 / 8 ms** |
| map + unmap of that space | ~1 520 ms, ~24 600 host calls | **~360 ms, 4 verbs (8 RM ioctls incl. 3 descriptors + 3 frees)** |
| whole CUDA space, lifetime (13 spaces of 13 075-16 552 runs) | ~15 k maps + ~15 k unmaps | **59 map calls + 42 unmap calls + 15 frees**; 437 ms map, 12 ms unmap (means) |
| process wall time, procs 2-24 (min / mean / max) | 3 798 / **4 716** / 6 016 ms | 3 822 / **4 286** / 4 859 ms |

(Proc 1 includes the cold boot of the guest driver: 17.0 s → 16.5 s.) Intermediate `bm2_nopm`
(rev `b675274f`, before the reaper): 24/24, maps 428 ms, unmaps 7 ms, procs 4 321 ms mean.

### 7.2 Where the map side's time went (`bm3_diag`, per 4 096-piece batch)

stitch 25-59 ms (4 096 `MAP_FIXED` ≈ 6-14 µs each on this nested box) · descriptor 7-18 ms (RM
pins + IOMMU-maps 4 096 pages) · **the view's `munmap` 80-97 ms** · the map itself **0.5 ms**.
⇒ The `munmap` now runs on a reaper thread (`reap_view`, bounded queue of 2); handing it over costs
~10 µs. ⊘ The map half is still **slower** than per-run (352 vs 265 ms): the stitch's `mmap`s and
the reaper's concurrent `munmap` (same `mmap_lock`) are the floor here, not RM. The trade is taken
because the unmap half — quadratic before (§1) — falls by ~1.25 s per process, and every later
unmap in the space walks a list of tens of mappings instead of ~15 000.
⚠ Open budget: a cheaper stitch (fewer VMA operations per piece) is the next lever on the map
half; on a non-nested host the `mmap`/`munmap` costs are expected to be several times smaller —
unmeasured.

### 7.3 Verification

- Unit: `kf-linux-raw` (stitch: page order, write-through to the file, refusals), `kf-mem` (batch
  grouping, cap, fallback verdicts incl. HELD/FAILED, range grouping and its fallback, `BatchBook`
  accounting) — `cargo test -p kf-linux-raw -p kf-mem -p kf-host -p kf-qemu -p kf-harness` green.
- **v3 gates 9/9 at `c0ebcda1`** (`/workspace/bench/uvmwall/bm_gates.log` on vh). Gate 4 now
  scatters its 192 channel pages across guest RAM in reverse order and asserts on the GPU:
  `scattered_sysmem_rows_placed_as_one_batch` (1 batch, 192 runs, 1 map verb), all three
  channels' **engine-written semaphores at the scattered pages** (page order through the batch is
  right), `a_piece_of_a_batch_unmaps_alone` (a middle page by range; the object stays booked), and
  `teardown_is_ranges_and_frees_the_batch` (2 ranges around the hole, the object freed once).
- 24/24 no-PM torch processes (`bm2`, `bm4`), 0 batch fallbacks, 0 free refusals.
- **30-arm fast suite 30/30 PASS at `c0ebcda1`** (`KF_DEVICE=kf3`, budget 180 s; vh `/workspace/bench/bmfs_suite.out`); arm times in line with earlier revisions (e.g. `--ce-client-guest-ram` 81 s vs 84-90 s).
- ⊘ Found on the way (`bm1`, rev `366e6db1`): `kf_qemu::mem::Target` forwarded only the trait
  methods that existed when it was written, so the new verbs hit the trait DEFAULT for every space
  — 3 groups formed, **0** batched, and nothing said so except 3 surplus map verbs. Fixed by
  explicit forwarding (`b675274f`). A trait default that means "not supported" is invisible
  through a wrapper; the instrument that caught it was `map verbs = runs + 3`.
