# V3 — batched guest-RAM maps: N scattered runs, O(1) host calls

**★ CORRECTION (2026-10-09, branch `claude/batched-map-rootcause-20261009`) — read §8 first.** A
range unmap that SPLITS a mapping placed through the space's `NV01_MEMORY_VIRTUAL` range frees the
mapping's WHOLE VA block in host RM; the remnants lose their PTEs. Every batch outside a guest
reservation was such a mapping, so on Windows (whose process VAs lie in the unreserved
`[1 MiB, 4.5 GiB)`) the first single-row unmap of a batch faulted the rest of it (`[measured, runs
242/243]` host Xid 31 `FAULT_PTE` at `0x4034000`; `gpu_vaspace.c:1639` assertions at exit). §4's
claim "with `size != 0` RM unmaps … splitting a straddler" holds for the resserv list only. And a
second defect on EVERY path (per-run included): the walker unmaps WHOLE placements, so a one-page
change inside a coalesced row transiently unmapped the unchanged pages (§8.2). Fixed: the apply
only touches what changed; outside any reservation one host mapping per guest leaf; batches only
inside a VA-reserving `hDma` (guest reservation, or a micro reservation once §8.0 says so); range
unmaps only over owned spans; no remap ever. Model-tested only; hardware verdict pending (§8.0).

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

`kf_host::HostRm::map_scattered(space, fd, pieces, at, defer, kind, perm)`:

1. `kf_linux_raw::MappedRegion::stitch(fd, pieces)` — a `PROT_NONE` reservation at a
   **kernel-chosen** address, tiled by one `MAP_FIXED | MAP_SHARED` per file-discontiguous piece at
   a cursor that only advances (no overlap, no hole — the pieces' lengths sum to the reservation).
   The only new `MAP_FIXED` in the tree; same argument as `Reservation::map_fixed_in` (§13: the
   address never leaves `kf-linux-raw`; `Indirect::describing` patches and scrubs it).
2. `alloc_os_descriptor(&view, 0, len)` — ONE `NV_ESC_RM_ALLOC_MEMORY`. The view is **dropped
   before any map exists** (RM holds the pages, not the VA).
3. ONE fixed `NV_ESC_RM_MAP_MEMORY_DMA` of the whole object at the first row's VA, 4 KiB-pinned
   (`MapBacking::SharedSlice`: a stitched object is contiguous at no bigger page), with the rows'
   (uncompressed) kind, their permissions (`MapPerm`, v3-roperm) and `DEFER_TLB_INVALIDATION` —
   the entry's single invalidate follows as before.

**All or nothing.** `Ok` ⇔ every piece is placed at its VA. A refused map is rolled back by RM
(`virt_mem_allocator_gm107.c:1540-1557`), a relocated one is torn down by the existing placement
assertion, and the object is freed before the error returns. ⇒ the caller's contract is binary.

### 3.1 What is batched (`kf_mem::apply`)

After every per-run check (store/RAM bound, whole pages, extent, `reserved()` overlap, a refused
unmap underneath), the surviving map rows are sorted by VA and cut into maximal groups that are
VA-adjacent, all guest RAM, one kind, **one permission set**, ≤ `BATCH_MAX_RUNS` (4 096). A group
of ≥ 2 goes to `MapTarget::map_batch`. ★ v3-roperm (2026-09-26): one host map carries ONE
permission set, so a batch across a guest read-only/read-write boundary would widen the RO rows —
the silent read-duplication corruption `V3_UVM_DEMAND_PAGING.md` §6 (branch `v3-uvm-research`)
found — or narrow the RW ones. `HostVas::map_scattered` refuses a mixed batch by name as well. Vidmem rows are never batched (they are slices of the store, already one
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

⊘ **Corrected 2026-10-09 (§8):** "the only mappings in it are those" was the CALLER's promise, not
enforced — `BatchedVas::unmap_range` now plans the range against its own ledger of host mappings
(`OwnMaps`) and unmaps one range per maximal OWNED span, never a gap; and a range that would split
one of our mappings outside a guest reservation is refused before any host call. The split rule
below ("splitting a straddler") is the resserv mapping list only: in the `NV01` range the VA block
of the split mapping is freed whole.

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
- ⊘ (2026-10-09: an explicit A/B OPT-OUT only; the default — batched on — is the correct path, §8.
  Windows runs no longer need it.) **Off switch** — `KF3_NO_BATCHED_MAP=1` restores the per-run
  path (A/B, and an escape hatch).

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

## 8. Root cause of the Windows post-sign-in freeze, the fix, and the low-range decision (2026-10-09)

**STATUS: LIVE, 2026-10-09 — CODE + MODEL-TESTED (`kf_mem::sim`); hardware verdict pending**
(branch `claude/batched-map-rootcause-20261009`). Owner directives folded in: split-by-remap is
FORBIDDEN (never written); every VA whose guest mapping is unchanged stays mapped at every instant;
outside a reservation the host mapping unit is the guest LEAF.

### 8.0 ★ THE DECISION POINT — the micro-reservation experiment (run it first)

Can an unprivileged client reserve a SMALL fixed range in `[1 MiB, 4.5 GiB)` of a Windows-config
twin space (`vaBase = TWIN_VA_FLOOR` 64 KiB, `[64 KiB, 1 MiB)` already reserved) and unmap part of
a batch mapped THROUGH it exactly?

```text
cargo build --release -p kf-harness --bin kf-micro-reserve-probe
KF3_WIN_USER_CHANNELS_PASSTHROUGH=1 target/release/kf-micro-reserve-probe reserve
KF3_WIN_USER_CHANNELS_PASSTHROUGH=1 target/release/kf-micro-reserve-probe nv01-control   # optional
```

`reserve`: census of 5 small reservations at Windows-like VAs (+ the old LARGE `[1 MiB, 4 GiB)`),
then reserve 8 pages at `0x4030000`, map 8 VRAM pages THROUGH it as one mapping, unmap page 3 by
range, CE-read pages 0 and 7 (must deliver), re-map page 3 (must deliver), tear down, free, VA free
again. `nv01-control`: the same through the `NV01` range — the remnant's VA block is free (a FIXED
map there lands) and page 7 faults (ONE Xid 31 on the probe's own channel; `gpu_vaspace.c:1639`
at exit): runs 242/243 reproduced in isolation.

| result of `reserve` | then |
|---|---|
| PASS (`reserve_small_accepted`, `remnant_*_reads`, `hole_remapped_reads`) | make `KF3_BATCH_MICRO_RESERVE=1` the default: the low range batches, partial unmaps exact (§8.3 rule 3) |
| `reserve_*` refused | keep the default: one host map per guest leaf in the low range (zero risk, O(leaves) calls — §8.4); batch only inside guest reservations |
| accepted but a remnant does not read | a defect in our reasoning about NV50 partial unmaps — stop, the CUDA path (which relies on it in the big reservations, gate 4) must be re-examined |

Why it is expected to pass (source, inferred until measured): a FIXED `NV50_MEMORY_VIRTUAL` is a
FIXED `eheapAlloc` (`gpu_vaspace.c:1374-1386`) refused with `NV_ERR_NO_MEMORY` only over an
existing heap block; the client RM of a GSP client withholds only the split window
`[4 GiB, 4.5 GiB)` (`gpu_vaspace.c:394-467`, `SPLIT_VAS_SERVER_RM_MANAGED_VA_START/SIZE`,
`g_gpu_vaspace_nvoc.h:90-91`). `[measured gfx8]` refused ONE LARGE `[1 MiB, 4 GiB)` — under the
default 1 MiB start, with host RM's own placements (the twin's context buffers, its rings) already
in that range ⇒ an overlapping block (inferred). Our per-run FIXED maps there succeed through the
same heap, so the VA we reserve (exactly a batch's, about to be mapped) is free.

### 8.1 Root cause of the freeze (measured + source)

**Measured** (GPU host, RTX 4070 Ada, 595.91.07, kf3 `c6fff2e3`): runs 242/243 (batched on; the
ONLY flag difference from run 244 is `KF3_NO_BATCHED_MAP`) each raise exactly one host
`Xid 31 … GR0_PBDMA0 HUBCLIENT_ESC faulted @ 0x0_04034000 FAULT_PTE ACCESS_TYPE_VIRT_WRITE` on host
channel `0x3c` (twin of a user-work passthrough channel, GPFIFO at `0x4000000`) ~47 s in; the guest
then stalls with no TDR; at QEMU exit host RM asserts `NULL != pMemBlock @ gpu_vaspace.c:1639`
(242: 3×, 243: 2×). Run 244 has neither. 242/243 placed batches (`map_scattered` lines; the
status line's `batch=0` is the doorbell fast path's `max_batch`, not batched maps).

**Source (ogkm-595.84):** `VaSpace::range` is an `NV01_MEMORY_VIRTUAL` (`bReserveVaOnAlloc =
NV_FALSE`, `virtual_mem.c:349`): each FIXED map allocates its own VA heap block, and
`dmaFreeMapping_GM107` calls `vaspaceFree(pVAS, DmaOffset)` (`virt_mem_allocator_gm107.c:1635-1638`)
whose `eheapGetBlock` is a containment search (`gpu_vaspace.c:1631-1640`, `eheap_old.c:1005-1027`).
A partial unmap (`virtmemUnmapFrom`, `virtual_mem.c:1697-1804`) keeps left/right remnants in the
mapping list but frees the whole block ⇒ remnant PTEs gone (`FAULT_PTE`), freeing a remnant later
finds no block (the `:1639` assert). Inside an `NV50_MEMORY_VIRTUAL` (`bReserveVaOnAlloc =
NV_TRUE`) only the unmapped PTEs are invalidated (`:1578-1633`) — exact. Windows process VAs lie in
the unreserved `[1 MiB, 4.5 GiB)`; CUDA VAs (gate 4: 128 GiB) in a reservation — why the gates and
Linux suites never saw it. **Inferred:** `0x4034000` was a page of a batch that a single-row unmap
split. Falsifier stated before the model run — "if the model of the current code passes, the cause
is elsewhere" — not met: 267/300 seeds failed (251 PTE losses, 16 `:1639` asserts), 0 foreign
mappings touched by the real apply/retire paths, the per-run path clean (commit `4b1f8440`).

### 8.2 The second defect, on every path: coalesced rows (owner correction, 2026-10-09)

The walker coalesces leaves into runs and diffs WHOLE placements: its UNMAP run IS the committed
placement (`kf_walk.cu:1157-1162`, same `gpga`/flags), the MAPs are the pieces in the gaps. A guest
that remaps ONE page of a 16-page VA+GPA-contiguous row produces UNMAP(16) + MAP(5) + MAP(1, new) +
MAP(10). Before this branch the apply unmapped all 16 and mapped 16 again: the 15 unchanged pages
were transiently unmapped — `[model, commit 5cec7adb]` 6 of 6 configurations (NV01 range and
reservation × per-run / batched / batched+micro). In the NV01 range a per-run row is moreover ONE
host mapping, so no exact partial unmap of it exists at all. (Whether this transient window caused
faults in Windows runs is not measured.)

### 8.3 The design (code)

1. **Net diff (`kf_mem::apply`, GPU targets).** A page an UNMAP and a MAP of the same entry name
   with the same aperture, kind, permissions, privilege, LEAF SIZE and linear backing is
   UNCHANGED: no host call at all. Only the changed sub-ranges are unmapped — by exact range over
   OUR placements (`MapTarget::unmap_range`, now independent of `KF3_NO_BATCHED_MAP`) — and only
   the new pieces are mapped. A leaf-size change is a change (its VAs are re-made). A MAP run whose
   new piece fails takes its kept part down too (named) so the ledger never holds what the walker
   did not commit.
2. **Leaf-granular placement outside any reservation (`BatchedVas::map`).** The walk report's page
   size (`KfMapRun::page_size` → `DiffRun::leaf` → `Desired::leaf`) sets the unit: one host map per
   4 KiB / 64 KiB / 2 MiB leaf in the `NV01` range (bounded, `MAX_LEAF_PIECES`), one map per row
   inside a reservation. Every later change there is a whole-mapping removal.
3. **Batches only inside a VA-reserving `hDma`.** A guest reservation (as before), or — with
   `KF3_BATCH_MICRO_RESERVE=1` (default OFF until §8.0 passes) and ≥ `LOW_RANGE_MIN_RUNS` (8) rows —
   a MICRO reservation made over exactly the batch's VA (`HostRm::reserve_va`), the batch mapped
   THROUGH it (`map_scattered_through`), later rows over its dead pages routed through it
   (`map_in`), unmaps inside it by `unmap_in`, the reservation freed when nothing of ours is left in
   it, every one freed at retire. A refused reservation ⇒ per leaf, named and counted.
4. **Owned spans only (`OwnMaps`).** Every host mapping of ours is recorded at host-mapping
   granularity with its `hDma`; a range unmap issues one call per maximal owned span per `hDma`,
   never over a gap or a foreign mapping, and refuses (`SPLIT_OUTSIDE_RESERVATION`) any range that
   would split one of ours in the `NV01` range.
5. **No remap, ever.** RM refuses two maps of one VA (`VA_ALREADY_MAPPED`;
   `intermapRegisterDmaMapping` "ensure no page can be mapped twice"), so make-before-break is
   impossible and break-before-make transiently unmaps unchanged VAs: partial unmaps are made exact
   instead (§8.0 or per leaf).
6. Unchanged constraints: the VA-manager thread does all of it (no new thread, no wait, no lock
   held across a host call); guest values bounded (checked/saturating arithmetic, piece caps); no
   new `unsafe`; nothing per family.

### 8.4 Syscall budget (reasoned from the numbers measured earlier; no new measurement)

Inputs — *measured earlier (nested vast box, §1/§7)*: per-run RM map 19-22 µs; per-run unmap
84-123 µs at ~12 k mappings in the `hDma` (35-39 µs at 1-4 k: grows with the count); stitch 6-14 µs
per `MAP_FIXED` piece; descriptor 7-18 ms per 4096 pages (~2-4 µs/page: RM pins); the view's
`munmap` 80-97 ms per 4096 VMAs (~20-24 µs/VMA, on the reaper thread); RM map of a batch 0.5 ms;
range unmap of a 12 k-run space 3-8 ms. *Reasoned*: a micro reservation alloc/free ≈ one RM call
each (~20 µs); bare metal several times cheaper than nested for every mm syscall (EPT-on-EPT page
walks) — not measured.

Per 4 KiB page, VA-manager thread, before the invalidate can clear (reasoned, nested):

| strategy | mmap | munmap | RM map | RM unmap (later) | desc alloc/free | reserve alloc/free | VA-thread time / page |
|---|---|---|---|---|---|---|---|
| per run / per leaf | 0 | 0 | 1 | ≤ 1 (a range covers many) | 0 | 0 | ~20 µs map + 40-120 µs unmap (O(M)) |
| batch, guest reservation (today) | 1 per file-discontiguous piece + 1 per batch | 1 per batch (reaper; O(pieces) work) | 1 per batch | 1 range per span | 1 + 1 per batch | 0 | ~8-18 µs (stitch + pin) |
| batch + micro reservation (low range) | same | same | 1 per batch | 1 range per span | 1 + 1 per batch | 1 + 1 per batch | ~8-18 µs + ~40 µs per batch |

Totals (reasoned, nested; scattered 4 KiB pages; "M" = mappings in the space):

| pages | per run: map / unmap / ioctls | batch (4096 cap): VA thread / reaper / ioctls |
|---|---|---|
| 4096 | 82 ms / ~0.2-0.5 s / 8192 | ~40-75 ms / 80-97 ms / 3 + ranges |
| 10 k | 200 ms / ~1 s / 20 k | ~100-180 ms / ~0.2 s / ~9 + ranges |
| 100 k | 2 s / ≫ 10 s (O(M) per unmap, extrapolated) / 200 k | ~1-1.8 s / ~2.2 s / ~75 + ranges |

**Crossover** (reasoned): a batch costs a fixed ~5 RM calls (+2 with a micro reservation) + 2 mm
syscalls ≈ 120-160 µs, and saves per page ≈ (20 µs map − ~12 µs stitch+pin) + the later unmap
(40-120 µs). On the map side alone that is ~15-20 pages; with the unmap side ≈ 2-4 pages. Kept
conservative: `LOW_RANGE_MIN_RUNS = 8` in the low range (guest reservations keep ≥ 2). Simulator
counts (`cargo test -p kf-mem workload_numbers -- --nocapture`, 2000 Windows-like steps = 19 790
pages of 1-16-page allocations, 40 % frees; 60 CUDA-like steps = 40 278 pages):

| workload | per run: RM calls/page | batch (guest resv only) | batch + micro: RM calls/page, mmaps, munmaps |
|---|---|---|---|
| Windows-like (low range) | 1.054 (19 790 maps + 1 070 unmaps) | 1.054 (nothing batches there) | 0.492 (3 224 maps, 1 759 unmaps, 1 491 desc, 883 reserve, 2 374 free), 19 548 mmaps, 1 491 munmaps |
| CUDA-like (reserved) | 1.001 | 0.004 (36 maps, 33 unmaps, 36 desc, 36 free), 40 314 mmaps, 36 munmaps | identical (no micro reservation in a guest one) |

**Cutting the per-piece syscalls** — what exists and what does not:
- Coalescing file-contiguous pieces: already done (`HostVas::map_scattered`); a GPA-contiguous run
  is ONE `MAP_FIXED`.
- Does RM need the view after pinning? No: `os_lock_user_pages` pins with
  `pin_user_pages(FOLL_LONGTERM)` and keeps the `struct page` array, never the address
  (`kernel-open/nvidia/os-mlock.c:216-254`, `nv.c:3357-3400`); the view is already dropped right
  after the descriptor (to the reaper). A persistent arena re-`MAP_FIXED` over old pieces saves
  only the per-batch `PROT_NONE` mmap + one munmap call: each replacement still zaps and frees the
  old VMA under `mmap_lock` for write, so the O(pieces) kernel work moves rather than vanishes.
  Recommended instead (not done here; `kf-linux-raw` unsafe code): munmap the view in chunks
  (e.g. 256 VMAs per call) so the reaper never holds `mmap_lock` for writing ~90 ms at a time.
- Hugepages: a 2 MiB-backed memfd with 2 MiB guest leaves gives file-contiguous 2 MiB pieces —
  already one mmap each by coalescing; 4 KiB pieces of a hugetlb memfd cannot be mapped at all
  (the stitch is refused by name → per leaf).
- One syscall for N scattered file pages: **none exists** in Linux for a shared file mapping
  (`remap_file_pages` is emulated with one VMA per call since 4.0; `mremap` moves one VMA per
  call; `madvise`/`process_madvise` map nothing; io_uring has no mmap op). `NV01_MEMORY_LIST_SYSTEM`
  (a page list) is `RS_FLAGS_ALLOC_PRIVILEGED` (`resource_list.h:622-628`) — closed to kayfabe's
  unprivileged client.

**End state (recommended):** §8.0 passes ⇒ micro-reserved batches in the low range (≥ 8 runs),
per leaf below that; guest reservations unchanged. §8.0 fails ⇒ per leaf in the low range (≈ 1 RM
map per leaf, the unmap side ranges over whole leaves). Either way no partial unmap ever splits an
`NV01` mapping, and no unchanged VA is ever transiently unmapped.

### 8.5 Tests (GPU-free, `cargo test -p kf-mem`, ~17 s; `kf_mem::sim`)

A host RM model of one client (per-`hDma` lists; NV01 per-map blocks and their whole-block free;
NV50 reservations as one heap block, exact inside; occupancy; `size == 0` / `size != 0`; object and
reservation free; foreign mappings) driven through the REAL `apply_entry` + `BatchedVas` with
`GpuMirror`'s glue. **The GUARD:** every page whose guest mapping is the same before and after a
refresh must translate identically after EVERY host call of that refresh. Invariants after every
step: no foreign mapping touched, no transient, no `:1639` assert, no PTE-less mirror mapping,
mirror translation == walker's committed set, zero gap bytes, zero unsafe splits.
- property: 300 seeds × 60 steps × {guest-reservation batching, + micro reservations}, 100 seeds
  per-run — maps, remaps, PARTIAL remaps inside placements, sparse/partial unmaps, straddles,
  retires, foreign mappings.
- targeted: the root-cause regression; one-page change inside a 16-page row (6 configurations);
  sparse changes; 64 KiB and 2 MiB leaves; leaf-size change both ways; a deliberate split-by-remap
  is caught by the guard; micro: exact partial unmap, refused reservation → per leaf, map failure
  inside its reservation releases it, > cap = 2 reserved batches; gate-4 numbers unchanged (1 batch
  / 1-call piece / 2 ranges + 1 free); foreign between/ending/sparse; empty gap = one call per span;
  no call over nothing of ours; retire with foreign windows; workload counts.

### 8.6 Still to verify on hardware

1. §8.0 `kf-micro-reserve-probe reserve` (and optionally `nv01-control`).
2. A Windows boot WITHOUT `KF3_NO_BATCHED_MAP` (and, after 1, with `KF3_BATCH_MICRO_RESERVE=1`):
   expect no Xid 31, no `:1639` assert at exit, sign-in survives past `inval=4955`.
3. `v3_gates.sh` 9/9 (gate 4: 1 batch, 1-call piece, 2 ranges, 1 free), the 30-arm fast suite, the
   CUDA no-PM lane (batch counts unchanged for CUDA spaces), the Linux broker lane.

⊘ **Related, fixed here:** the v3-video falcon-context steer (`kf_qemu::chan`) unmapped a guest row
by its START outside `BatchedVas`; with per-leaf mappings that would take only the row's first
leaf. It now unmaps the row by exact range (whole mappings of ours only). It still bypasses
`OwnMaps` (the ledger keeps a stale entry for that row; a later owned-span range over it finds
nothing of this client's `hDma` there and answers `NV_OK`) — route it through the mirror in a
follow-up.
