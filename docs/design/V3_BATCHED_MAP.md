# V3 — batched guest-RAM maps: N scattered runs, O(1) host calls

**★ CORRECTION (2026-10-10, branch `claude/batched-map-decisions-20261010`, off `integration/windows-20261010` @ eff1b692) —
the three decisions §8.7 left open are decided and implemented; read §8.8 first.** (D1) The
transient of an UNCHANGED VA (`remade_unchanged_pages`) is gone by construction: outside every
VA-reserving `hDma` a mapping of ours is ONE 4 KiB page, or sits in a micro reservation sized to its
row — so any later partial change is exact, and no RM partial-unmap semantics are relied on in the
4 KiB case. §8.7.3's "Owner decision (open)" is ANSWERED. (D2) The act thread (falcon-context steer)
taking the ledger mutexes is accepted on condition: every hold is bounded by `LEDGER_CHUNK` entries
and tested. (D3) Micro reservations are the DEFAULT (no flag; `KF3_BATCH_MICRO_RESERVE` is gone);
gate 10 of `scripts/bench/v3_gates.sh` re-measures them. Model-tested (`cargo test -p kf-mem`); no
hardware run yet (§8.8.6).

**★ CORRECTION (2026-10-10, branch `claude/batched-map-review-fixes-20261010`, off 6fafcc6e) — the
adversarial review of the 2026-10-09 fix found its verdicts could leave the walker and the host out
of sync; read §8.7 first.** (1) A refused or unsplittable unmap of a changed sub-range acked the
UNMAP run FAILED but its fully-kept MAP runs APPLIED: the walker's slot held overlapping placements
and the next refresh unmapped the old one whole, unchanged pages with it — reproducible with NO host
error by a > 512 MiB 4 KiB row in the `NV01` range (one host mapping, `MAX_LEAF_PIECES`). (2) A
refused new part of a MAP run took its UNCHANGED part down. (3) A leaf-size change with an identical
translation transiently unmapped every VA. (4) The per-run fallback decided "nothing of ours here"
from a run's start. (5) A refused micro-reservation free was forgotten. (6) The falcon-context steer
bypassed the ledger. All fixed and model-tested (§8.7); `KF3_NO_BATCHED_MAP=1` no longer reproduces
run 244 (§8.7.6). Hardware verdict pending (§8.6).

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
- ⊘ **Corrected 2026-10-10 (§8.7.6), above the text it corrects:** the ledger's mutexes (book,
  own, micro) are also taken by the act thread's falcon-context steer. They are still never held
  across a host call.
- **No blocking on a vCPU / under a shared lock** — all of it runs where the per-run maps ran (the
  VA-manager thread); the book's mutex is VA-thread-only and never held across a host call.
- **VMM addresses never guest-visible** — the stitched VA exists inside one call, inside
  `kf-linux-raw`, and is scrubbed from the ioctl argument; the GPU VA is the guest's own.
- **§13** — the only new `unsafe` is in `mapping_unsafe.rs` (`kf-linux-raw`).
- **All families first-class** — nothing here is per-die: NVOS46/NVOS47 and the OS descriptor are
  family-independent RM API; the grain is 4 KiB on Turing … Blackwell; kinds are carried as before.
- ⊘ **Corrected 2026-10-10 (§8.7.6), above the text it corrects:** the flag no longer restores the
  pre-batch path. It turns off stitched BATCH objects only; the net diff, the one-mapping-per-leaf
  placement outside reservations and the owned-span range unmaps stay on. It does not reproduce run
  244; build `c6fff2e3` for that.
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

⊘ **Corrected 2026-10-10 (§8.7.7), above the text it corrects:** the hand-over to the reaper no
longer blocks. A full queue parks the view in a bounded overflow, and the stitch is refused while
the overflow is full.

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

⊘ **ANSWERED 2026-10-10 (§8.8.3), above the text it answers:** the decision is taken — micro
reservations are the default, with no flag. Basis: the delegating session reports `kf-micro-reserve-probe
reserve` PASSED on the trusted host at `6fafcc6e` (no log is in this tree; not re-run here). The probe is
now gate 10 of `scripts/bench/v3_gates.sh` and also records, information only, whether host RM accepts
the 7.9 GiB flat-FB-alias reservation. Rows below that read "make `KF3_BATCH_MICRO_RESERVE=1` the
default" are done by DELETING the flag.

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

⊘ **Corrected 2026-10-10 (§8.7.6), above the text it corrects:** "the only flag difference" held
at `c6fff2e3`. Since 833a6f5a the flag no longer selects that difference: run 244's behaviour is the
REVISION `c6fff2e3` with the flag, not any later revision with it.

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

⊘ **Corrected 2026-10-10 (§8.8), above the text it corrects:** rule 2's "one host map per 4 KiB /
64 KiB / 2 MiB leaf in the `NV01` range" is wrong since D1: outside every VA-reserving `hDma` a row
whose leaf is bigger than 4 KiB goes THROUGH one micro reservation sized to it, else at 4 KiB grain
(one host mapping per page) — never one mapping per big leaf. Rule 3's "`KF3_BATCH_MICRO_RESERVE=1`
(default OFF until §8.0 passes)" is wrong: ON by default (D3). The "re-made, declared" part of the
§8.7 correction below is wrong too: nothing is re-made (`remade_unchanged_pages` is a guard that
stays 0).

⊘ **Corrected 2026-10-10 (§8.7), above the text it corrects:** rule 1's key no longer has the LEAF
SIZE (an identical translation is unchanged, §8.7.3); a kept page must be covered by a mapping of
ours, and part of an `NV01` mapping that a changed piece would split is re-made, declared (§8.7.3);
a failed MAP run never takes its kept part down — the runs linked to it fail with it (§8.7.1-2).
Rule 2's "bounded, `MAX_LEAF_PIECES`" now means: per leaf up to 2^20 leaves, beyond that a micro
reservation or a refusal by name, never one unsplittable mapping (§8.7.1). Rule 3: a refused
reservation free stays tracked (§8.7.5).

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

⊘ **Corrected 2026-10-10 (§8.8), above the text it corrects:** the "per run / per leaf" row now
reads "per run / per 4 KiB page" wherever no micro reservation holds the row (reservations off, or
refused by host RM at run time): a 64 KiB leaf costs 16 RM map calls and a 2 MiB leaf 512 there
(cost only reasoned, not measured). With reservations (the default) a big-leaf row costs one
reservation alloc, one map, and later one free: ~3 RM calls whatever its size. The "micro" columns
of the table below describe the default now.

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

⊘ **Extended 2026-10-10 (§8.8.4):** 148 lib tests now (was 135), ~41 s wall in a debug build; the property tests also run with runtime refusals of the reservation machinery and assert `remade_unchanged_pages == 0`, nothing rigid, no NV01 mapping bigger than a page, and every lock hold within one chunk.

⊘ **Corrected 2026-10-10 (§8.7.7), above the text it corrects:** the model had two blind spots. Its
walker keyed placements by VA, so overlapping placements overwrote each other silently. Its host
never refused an unmap. Both are fixed, and its guard no longer keys on the leaf size. The suite now
takes ~28 s wall (debug build; the longest test is the adversarial error injection, ~20 s).

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

⊘ **Corrected 2026-10-10, above the text it corrects:** item 2 must run at a revision that has the
§8.7 fixes, and items 4-7 are added. 4: the falcon-context steer on a video lane — the log line says
`steered` and NVDEC writes frames. 5: no `kf-mem: … UNCHANGED page(s) re-made` lines, or only a few,
in a Windows boot and the CUDA lanes; each line names the §8.7.3 inexact case. 6: the status line's
batch counters, and no `ReaperBacklog` refusals with batching on. 7: the retire lines never report
`could not be released` (§8.7.5).

⊘ **Corrected 2026-10-10 (§8.8.6), above the text it corrects:** item 1 is now gate 10 of
`scripts/bench/v3_gates.sh` (`GATE10_VERDICT=PASS`), item 2 runs with micro reservations ON (the
default; there is no flag), item 5 is a guard (`UNCHANGED page(s) re-made` must NEVER appear), and
items 8-10 are added in §8.8.6.

1. §8.0 `kf-micro-reserve-probe reserve` (and optionally `nv01-control`).
2. A Windows boot WITHOUT `KF3_NO_BATCHED_MAP` (and, after 1, with `KF3_BATCH_MICRO_RESERVE=1`):
   expect no Xid 31, no `:1639` assert at exit, sign-in survives past `inval=4955`.
3. `v3_gates.sh` 9/9 (gate 4: 1 batch, 1-call piece, 2 ranges, 1 free), the 30-arm fast suite, the
   CUDA no-PM lane (batch counts unchanged for CUDA spaces), the Linux broker lane.

⊘ **Corrected 2026-10-10 (§8.7.6), above the text it corrects:** the follow-up is done. The steer
now goes through the ledger (`BatchedVas::hand_to_host`).

⊘ **Related, fixed here:** the v3-video falcon-context steer (`kf_qemu::chan`) unmapped a guest row
by its START outside `BatchedVas`; with per-leaf mappings that would take only the row's first
leaf. It now unmaps the row by exact range (whole mappings of ours only). It still bypasses
`OwnMaps` (the ledger keeps a stale entry for that row; a later owned-span range over it finds
nothing of this client's `hDma` there and answers `NV_OK`) — route it through the mirror in a
follow-up.

## 8.7 Adversarial review fixes (2026-10-10)

**STATUS: LIVE, 2026-10-10 — CODE + MODEL-TESTED; hardware verdict pending (§8.6).** Branch
`claude/batched-map-review-fixes-20261010`, off 6fafcc6e (`integration/windows-20261010`). The
review's tests are `crates/kf-mem/src/sim/adversarial.rs`, cherry-picked from
`review/batched-map-adversarial-20261010`. At 6fafcc6e five of them failed; for example,
`adversarial_error_injection_at_every_call_index` broke an invariant in 362 of 5 016 injected runs.
All of them now pass in the default suite. *Measured* below means the GPU-free model; nothing here
was run on hardware.

### 8.7.1 Commit consistency (findings 1, 2)

The walker commits WHOLE runs (`kf_cuda::diffmodel::commit`). An APPLIED UNMAP leaves its slot. An
APPLIED or HELD MAP enters it. A FAILED run leaves the slot unchanged. `apply::fail_linked` keeps
that slot equal to what the host holds, iterated to a fixpoint:

- (a) A MAP that overlaps an UNMAP that stays committed fails too. Without this, the slot holds two
  placements over one VA. This was finding 1. Its consequence: the next refresh unmapped the old
  placement whole, unchanged pages with it.
- (b) An UNMAP whose kept pages a failed MAP was to carry fails too. The kept pages stay mapped:
  they are never taken down for a neighbour's refusal (finding 2). So their placement stays
  committed.

Every plain MAP run's rows are checked before any host call, so (b) mostly acts before anything is
touched. The changed pieces of an UNMAP failed this way are still unmapped. What the guest changed
must not stay reachable: it becomes absence, which §AA clears over. The new pieces a failed MAP
already placed are taken down again. A kept page is never taken down.

Finding 1 with no host error was a > 512 MiB 4 KiB row in the `NV01` range, kept as one mapping.
Now a row is placed one mapping per leaf up to `MAX_LEAF_PIECES` = 2^20 leaves (4 GiB of 4 KiB:
the whole Windows low range). ⊘ **Corrected 2026-10-10 (§8.8.1), above the text it corrects:** the unit of the 2^20 bound is the 4 KiB
grain now (not the guest leaf), and the reservation is the default (D3), not `KF3_BATCH_MICRO_RESERVE=1`.
Beyond that, the row goes through a micro reservation when
`KF3_BATCH_MICRO_RESERVE=1`, or it is refused by name (`HUGE_ROW_OUTSIDE_RESERVATION`, absence,
counted in `huge_refused`). No row is ever one mapping that cannot be split.

Residual (inferred, not measured): an UNMAP failed by (b) leaves its changed pages absent while the
walker still lists the old placement. If the guest restores exactly the old translation before the
refused leaf is fixed, the walker sees nothing to do and the page stays absent until that placement
changes again. A range that host RM refuses after acting has the same window. The walker protocol
has no way to un-commit part of a placement. Closing this needs a walker change (a "dirty" re-emit),
which is out of scope here.

### 8.7.2 Keep only what a mapping of ours covers

`MapTarget::own_view` reports what the ledger holds. A kept interval survives only where a mapping
of ours covers it. A page the walker still lists but whose host mapping is gone is mapped again as
a new piece. Such a page comes from a range host RM refused after acting, or from a take-down.
Before, it was "kept" absent forever. `[model]` Disabling this alone fails the fault-injection
property variants: 5 of 300 seeds, "walker committed …, host translates None".

### 8.7.3 Leaf size is not identity (finding 3) — and the one inexact case

⊘ **ANSWERED 2026-10-10 (§8.8.1), above the text it answers:** "the one inexact case" no longer
exists. Outside every VA-reserving `hDma` a mapping of ours is one 4 KiB page or sits in a micro
reservation, so the apply never re-makes anything (`remade_unchanged_pages` == 0, a guard), and
`SPLIT_OUTSIDE_RESERVATION` is unreachable for page-aligned ranges. The paragraphs "The inexact
case", "Owner decision (open)" and the "pathological case" below are SUPERSEDED by §8.8.1 and kept for
the record. The sentence *"`[model]` Property test … 668 pages re-made, all in the `NV01` range"* is
the pre-D1 measurement; the same property test now re-makes 0 pages.

`same_mapping` compares aperture, kind, permissions, privilege and linear backing. It no longer
compares the leaf size. An identical translation re-expressed with another leaf size makes NO host
call. The walker's UNMAP (old page-size class) and MAP (new class) are both acked APPLIED, and its
own commit moves the placement between classes.

**⊘ SUPERSEDED 2026-10-10 (§8.8.1) — The inexact case.** Outside every VA-reserving `hDma` (the `NV01` range), a big leaf WAS ONE host
mapping (rule 2). The guest can split it into 4 KiB leaves and re-point part of it. Host RM cannot
remove that part alone: a partial unmap frees the whole VA block (§8.1). It also cannot map a second
mapping over the first (`VA_ALREADY_MAPPED`). The same holds for a 64 KiB mapping kept across a
64 KiB → 4 KiB re-expression that is later partly changed, and for a mapping that two different MAP
runs would keep. For each of these, `apply::keep_only_what_stays_exact` RE-MAKES the mapping whole:
it is unmapped with the changed pieces, and its kept pages are mapped again as new pieces of their
MAP runs. This is the one remaining transient of an UNCHANGED VA. It is declared
(`Applied::remade_unchanged_pages` and `remade`, `VaStats::remade_unchanged_pages`, a bounded
`UNCHANGED page(s) re-made` log line), and the model guard counts it apart (`remade_transients`).

At 6fafcc6e this case had the same transient, undeclared: the leaf-size key made every page
"changed". The alternative, refusing the unmap, would hold the guest's invalidate forever (§AA does
not cover a refused unmap: the run-223 hang class).

**Owner decision — ANSWERED 2026-10-10 (§8.8.1): both, in this order — (2) when a micro reservation is available, else (1).** Two exact alternatives existed:
1. Map every leaf in the `NV01` range at 4 KiB grain. A 64 KiB leaf then costs 16 RM map calls and a
   2 MiB leaf 512, and the twin uses 4 KiB host PTEs (GPU TLB reach). Cost only reasoned.
2. After §8.0 passes, map every big-leaf row in the `NV01` range through a micro reservation sized
   to the row. That costs one reservation alloc and one free per row.

**SUPERSEDED (§8.8.1: unreachable) —** One combination is not re-made: an UNMAP whose placement holds such a mapping AND that already
fails by a link (§8.7.1 (b)) before any host call, for example a split big leaf plus a refused leaf
coalesced into the same placement in one refresh. Its kept pages must stay, so its changed pages
inside the mapping cannot be removed. That unmap is refused (`SPLIT_OUTSIDE_RESERVATION`, counted in
`unsafe_splits`), and the invalidate is held, as for any refused unmap. This is named, and is the
pathological case.

Inside guest reservations nothing is ever re-made: their splits are exact.
`[model]` Property test, 300 seeds × 60 steps: 101 leaf re-expressions (44 of them with a change),
668 pages re-made, all in the `NV01` range.

### 8.7.4 The rows follow the ledger (finding 4)

`BatchedVas::unmap_run` with a known length takes the WHOLE run `[va, va+len)`. Before, it decided
"nothing of ours here" from `va`, so a tail outlived the guest's unmap.

On a GPU target the apply no longer uses the per-run verb. Each changed piece falls back to its own
exact range. After a refused range, `GpuMirror` (and the model's glue) cut the rows to what the
ledger still holds. Before, the rows were put back whole while the ledger had been cut, so a reader
resolved through mappings that were gone.

A map over a VA where the ledger still holds a STRAY of ours removes the stray first
(`clear_strays`, counted in `strays_removed`). A stray is a refused rollback or take-down. A rollback
host RM refuses is recorded as a stray, never forgotten, and the row is refused instead of reported
HELD.

### 8.7.5 Micro reservations and batch objects are never forgotten (finding 5)

A reservation leaves `micro` only when host RM has freed it. A refused free stays tracked:
- a later map there goes THROUGH it;
- the next release over it retries the free;
- `drain` retries once more.

An erroring range still releases what it emptied. A refused batch-object free goes to
`stuck_objects`. `BatchedVas::leftovers` counts what a retire could not release. When it is
non-zero, `GpuMirror::unmap_all_rows` reports it as refused, and `retire_mirror` frees the host
space instead of recycling it into another guest VA space.

### 8.7.6 The falcon steer through the ledger (finding 6); what `KF3_NO_BATCHED_MAP` does now (7)

**The steer.** `kf_qemu::chan`'s steer calls `BatchedVas::hand_to_host`. That unmaps owned spans
only, each through the `hDma` it was mapped through (a micro reservation included). It cuts the
ledger, releases an emptied micro reservation, and says whether host RM can place there:
`Free`, `StillReserved` (other rows live in the reservation) or `StillOurs`. The ledger
(`Arc<BatchedVas>`) is shared between `GpuMirror` and the channel plane's `Mirror::ledger`.

Running on the act thread is safe for two reasons. No ledger lock is held across a host call. And a
reservation with a map in flight through it is pinned (`MicroResv::pins`), so the act thread never
frees it under the VA thread.

⊘ **ANSWERED 2026-10-10 (§8.8.2), above the text it answers:** accepted, on a condition that is now
code and a test — every ledger lock hold touches at most `LEDGER_CHUNK` (2048) entries. The
"O(log n + k)" claim below was FALSE for several paths (the book's backward scan, `OwnMaps::within`
collecting k entries under the lock, `release_micro`/`segments` iterating all reservations from 0,
`unpin` and `drain` O(n)); §8.8.2 lists them.

Non-stall note (rule 4): the steer now takes the ledger's mutexes. Each is held O(log n + k) for
the range touched; `forget_batch` was O(n) and is now range-limited. This is the same class as the
`rows` lock the steer already took. It is still a lock the VA thread also takes. Whether that is
acceptable on the act thread is an owner judgment; the alternative is a steer request answered by
the VA thread.

**`KF3_NO_BATCHED_MAP=1` today** (`kf_qemu::mem::batching_enabled`): `GpuMirror::map_batch`
answers `NOT_BATCHED`, so no stitched batch object is ever made. Everything else is on with or
without the flag:
- the net diff (kept pages get no host call; linked failures);
- one host mapping per guest leaf outside reservations, with the 2^20-leaf bound;
- owned-span range unmaps (`GpuMirror::unmap_range` is not gated);
- the ownership ledger, stray removal, micro reservations (⊘ 2026-10-10: ALL big-leaf and over-bound
  rows, ON by default — D1/D3; was "over-bound rows when `KF3_BATCH_MICRO_RESERVE=1`");
- the steer through the ledger.

So the flag is an A/B of the batch objects only. It does NOT reproduce run 244, which mapped each
walker run as one host mapping per `hDma` and unmapped whole placements. Run 244's behaviour is
revision `c6fff2e3` (or any revision before 833a6f5a) with the flag. No flag was added.

### 8.7.7 The model (finding 8) and the non-stall item

**The model.** `sim::apply_and_commit` commits like the walker and records a `WALKER SLOT OVERLAP`
violation when an acknowledged MAP lands over a committed placement. The guard keys on the
translation only, and on pages the host maps right now. `SimRm::fail_unmaps` refuses every n-th
mirror unmap, either before acting or after acting.

The property test runs 300 seeds × 60 steps in six configurations: batched, batched with micro
reservations, and each of those with refusals every 5th call (before) and every 7th call (after).
The per-run path runs 100 seeds × 3. The test includes leaf-size re-expressions with and without a
same-refresh change. After each refusal the walker's retry runs: exactly the runs acked FAILED.

Mutations confirm the suite catches each fix when it is reverted:
- the linked-failure closure → 6 tests fail;
- the leaf-size key → 4 fail, 89/300 seeds TRANSIENT;
- the start-only `unmap_run` → 1 fails;
- the old micro release → 3 fail;
- the rigid re-make → property fails with unsafe splits;
- the owned-intersection → the fault variants fail.

**The view reaper.** `reap_view`'s blocking `SyncSender::send` (queue of 2) could stall the VA
thread behind a slow reaper. It is now `kf_host::channel::Reaper::hand`: `try_send`, plus a bounded
overflow (`REAP_OVERFLOW` = 2) retried on every hand-over and before every stitch. `map_scattered`
refuses to stitch while the overflow is full (`ScatterError::ReaperBacklog`), so the rows go per
run. The VA thread never waits for the reaper, and no view is forgotten. Tested GPU-free in
`kf-host` (`channel::reaper_tests`). A remaining kernel-level coupling is documented in §7.2: the
stitch's `mmap`s and the reaper's `munmap` share `mmap_lock`.


## 8.8 Delegated decisions 2026-10-10 (D1 big-leaf split, D2 act-thread ledger locks, D3 reservation default)

**STATUS: LIVE, 2026-10-10 — CODE + MODEL-TESTED; hardware verdict pending (§8.8.6).** Branch
`claude/batched-map-decisions-20261010`, off `integration/windows-20261010` @ eff1b692. The owner
delegated the three items §8.7 left open to the coordinating session, which decided them as below.
They are binding in the code; they are not yet entered in `docs/OWNER_RULINGS.md` (the owner may
confirm or amend them there). The invariant behind D1 is the owner's, 2026-10-09 (§8 STATUS): *an
UNCHANGED VA is never transiently unmapped during a refresh; no unmap-then-remap of kept pages.*
*Measured* below means the GPU-free model (`cargo test -p kf-mem`) or a number printed by a test;
nothing here ran on hardware.

### 8.8.1 D1 — outside a reservation every change is exact

**The rule (`kf_mem::batch::BatchedVas::leaf_segments`).** Outside every VA-reserving `hDma` (the
`NV01` range) a mapping of ours is either exactly ONE 4 KiB page, or lies inside a micro
reservation. A page is the unit nothing can split, and the walker changes translations in whole
pages, so a later partial change can never need a partial unmap of an `NV01` mapping. Per unreserved
segment `[s, e)` of a row whose guest leaf is `leaf`:

| case | placement | later partial change |
|---|---|---|
| (a) `leaf` > 4 KiB, aligned, or the segment exceeds `max_leaf_pieces` (2^20) grains, and reservations are on | ONE micro reservation over exactly `[s, e)` (alloc + free per segment), ONE host mapping through it | exact range unmap inside the reservation (NV50 invalidates only the unmapped PTEs, `virt_mem_allocator_gm107.c:1578-1633`) |
| (b) reservations off, or host RM refused (a) at run time | 4 KiB grain: one host mapping per page; bounded by 2^20 pieces | each page is its own host mapping and its own unit of unmap — **no RM partial-unmap semantics are relied on** |
| (a2) *(my addition)* (a) refused AND (b) would exceed the bound, segment is whole leaves ≤ 2^20 of them | ONE reservation per leaf; a leaf whose reservation is refused too goes at 4 KiB grain, the grains of all such leaves within the same 2^20 budget | as (a) / (b) |
| none of them fits | refused by name, `HUGE_ROW_OUTSIDE_RESERVATION` (absence; nothing of ours left behind) | — |

A 4 KiB-leaf row is case (b) already (as before). A refused reservation changes nothing: the alloc
happens before any map, nothing is placed or lost (model: reservation allocs refused every 3rd/4th
call, and `refuse_reserve_over`). A reservation host RM accepts is always used for its segment
(`out` carries `(Some(h), s, e)`, pinned until the map is recorded). A map that fails in the middle
of a multi-piece row rolls back the pieces already placed (existing all-or-nothing code, now also
exercised with reservations; model: every 7th/11th row map fails).

**Why (a2) exists.** The measured regression at 6fafcc6e (fast suite 0/30) was a refused row: a
guest-KERNEL space's flat FB alias is ONE run of 2 MiB leaves of 7.9 GiB; below the carve-out it is
3 963 whole leaves = 2 029 056 grains — more than 2^20, so it can only be placed through a reservation. If host RM refused the one 7.9 GiB reservation
(the probe passed on small ranges; `[measured gfx8]` it refused one LARGE `[1 MiB, 4 GiB)`), D1 as
literally decided would refuse the row again and poison every kernel CE channel. (a2) keeps the row
placed and exact (3 963 leaf reservations, ~8 000 RM calls, reasoned ≈ 0.2 s). If every reservation is
refused (or reservations are off) the alias IS refused by name — the decision's rule, and the largest
residual risk (§8.8.7). Model test: `flat_alias(AliasHost::{Accepts, RefusesBig, RefusesAll, Off})`.

**What disappears, and the proof.**
- `Applied::remade_unchanged_pages` (the re-make in `apply::keep_only_what_stays_exact`) is 0 by
  construction: `OwnView::rigid` needs a mapping of ours bigger than a page outside a reservation,
  and none can be made. The counter and the declared re-make stay as a last resort. Guards:
  `BatchedVas::rigid_seen` counts any rigid mapping `own_view` reports; the property tests assert
  `remade_pages == 0`, `rigid_seen == 0`, `remade_transients == 0` after every step, and
  `sim::check` fails any mirror mapping in the `NV01` range that is not exactly one page
  (mutation: placing big leaves one mapping per leaf again → all property tests fail at the first
  big map, "NV01 mapping … is bigger than one 4 KiB page"). `[model]` 100 releaf steps per
  config, 44 with a same-refresh change: **0 pages re-made** (was 668).
- `SPLIT_OUTSIDE_RESERVATION` is **unreachable for page-aligned ranges** (state: proven by the
  invariant above, not by a separate argument): it fires only when an edge of the range cuts a
  mapping of ours that has `via == None` and is not guest-reserved; every such mapping is one page, so
  a page-aligned edge never cuts one. Every apply path unmaps whole pages. The check stays as the
  refusal for an UNALIGNED range, and a test plants a 64 KiB `NV01` mapping to show the branch, the
  counter (`unsafe_splits`), `rigid_seen` and the last-resort re-make (declared, 15 pages) still work.
- The tests that asserted the old shape are replaced, not weakened:
  `big_leaves_are_placed_exactly_and_change_alone` (was "…one mapping each…"),
  `splitting_a_big_leaf_and_changing_part_of_it_is_exact_everywhere` (was "…remakes_it_declared": now
  exactly 1 unmap + 1 map, 0 re-made, in 3 configurations), `no_nv01_mapping_bigger_than_a_page_can_be_made`,
  `a_planted_rigid_mapping_is_counted_refused_and_remade_declared`.

**Cost (reasoned, not measured).** With reservations (default): per big-leaf segment one RM alloc +
one map + one free — independent of its size. Without: 16 (64 KiB) / 512 (2 MiB) RM map calls per
leaf and the TLB reach of 4 KiB host PTEs; the unmap side is one range call per owned span.

### 8.8.2 D2 — the act thread takes the ledger locks, on a condition that is now a test

**Decision.** Accept the steer (`BatchedVas::hand_to_host`, act thread) taking the ledger mutexes
(`book`, `own`, `micro`). The alternative — a steer request answered by the VA thread — makes the
act thread WAIT on the VA thread, strictly worse. Condition: no ledger lock is held across a host RM
call, a syscall, a log line, or an allocation proportional to a large `n`; every critical section
touches at most `LEDGER_CHUNK` = 2048 ledger entries (or bitmap words) and the structure is
consistent between chunks.

**Audit: violations found and fixed.** All locks go through `BatchedVas::hold`/`hold2`, which record
the entries touched and the time held (`HoldStats`; `BatchedVas::hold_stats()` is `(max entries,
longest µs)` and is now part of the steer's log line, so the cost is observed on every steer).

| violation (the §8.7.6 claim "O(log n + k)" was false for these) | fix |
|---|---|
| `OwnMaps::within` collected all k entries (k up to 2^20) under the lock: `plan`, `cut`, `own_view`, `unmap_run`, `any_in` | `within_limited` (≤ 2048); `any_in` O(log n); `unmap_owned` forms and unmaps spans while the chunks stream by; `cut_own` / `forget_batch_range` / `collect_within` loop over chunks |
| `BatchBook::unmapped_extents`: backward scan from `va - max_len` plus a bit-per-page loop, all under one hold | `unmapped_step` with a unit budget and a (entry, page) cursor; always makes progress |
| `BatchBook::insert` built the liveness bitmap (pages/64 words, hostile-sized) under the lock | `BatchBook::prepare` outside, `insert_prepared` O(log n) inside |
| `BatchBook::drain` and `OwnMaps::forget_batch` (`retain`, O(n) per batch) under the lock | `take_all` O(1) + dismantled outside; chunked `forget_batch_step` over the batch's own extent |
| `segments`, `release_micro`, `micro_covers` iterated every reservation from key 0 (`range(..end)`) under the lock; `unpin` scanned all reservations once per handle (O(n·h)) | reservations never overlap: only the predecessor can reach into `va`, then `range(va..end)` in chunks; `unpin` finds its reservation by `range(..=s).next_back()`; emptiness test O(log n) |
| a poisoned lock was silently treated as "nothing of ours" | poison-tolerant `lk()` |

Audited and clean: no host call, syscall, `eprintln!` or `free` runs inside a `hold` closure (read
line by line; the logs and frees are after it); the `micro`-then-`own` order is unchanged.

**Consistency between chunks (reasoned).** A chunked `segments` pins reservations as it goes; only
the VA thread ADDS a reservation, so none appears in a gap meanwhile, and one the act thread
releases before it is pinned is simply not seen — the same window the single-hold version had after
`release_micro` took a reservation out and before it freed it. A chunked cut leaves, between chunks,
ledger entries for mappings host RM already removed (the span was unmapped first); that window
existed (host call → cut) and is now as long as the cut. A concurrent `clear_strays` finds them and
unmaps an empty range, which host RM answers `NV_OK`.

**Measured (model + release build, `a_2_pow_20_piece_row_never_holds_a_ledger_lock_beyond_one_chunk`).**
A 2^20-piece row (4 GiB of 4 KiB grains, the largest row the ledger holds outside a reservation) is
placed, read, handed to host RM in part and whole, unmapped by run and by range: 1 054 750 holds,
**at most 2 048 entries touched by any one hold** (the bound is 2 049 = chunk + the one predecessor a
range scan adds); before, one hold touched 1 048 576 (mutation: `cut_chunk` unbounded → the test fails
with "a hold touched 1048576 entries"). Longest hold seen: 5.0 ms in three release runs (4 984 /
5 003 / 5 210 µs), 1.5-5.7 ms in debug; the outliers were single `OwnMaps::cut` holds on a 1M-entry
map (backtrace: `unmap_run`, `cut_own`) — cause *inferred* to be the allocator freeing tree nodes, not
measured; no histogram is kept, so the typical hold is not claimed. For comparison the unbounded
hold was O(2^20). Other tests: 6 151 reservations walked by `segments`/`hand_to_host`/`release_micro`;
chunked book step and chunked cut equal to the unbounded versions on 200 random rounds each at
budgets 1-5; property tests assert the bound after every step.

### 8.8.3 D3 — micro reservations are the default

- `BatchedVas::new` → reservations ON; `kf_qemu::mem::micro_reserve_enabled` → ON. The flag
  `KF3_BATCH_MICRO_RESERVE` is **deleted** (setting it does nothing). Reasons: (i) a row beyond 2^20
  grains is otherwise refused — a correctness hole (the 7.9 GiB flat alias is one); (ii) the delegating
  session reports `kf-micro-reserve-probe reserve` PASSED on the host at `6fafcc6e` (no log in this tree;
  not re-run here).
- Opt-out: `KF3_NEGCTL_NO_MICRO_RESERVE=1` (read once). **Named NEGCTL, not `KF3_DIAG_NO_MICRO_RESERVE`
  as suggested:** `V3_FLAG_INVENTORY.md` §8 reserves `KF3_DIAG_*` for pure measurement that changes no
  behaviour; this flag changes placement, so it is a negative control. It is for A/B and bisecting
  a host that misbehaves, never a launcher setting; off, over-bound rows are refused by name.
- A run-time refusal is a clean fallback to the 4 KiB grain / per-run (batch: `NOT_BATCHED`), nothing
  placed or lost; an accepted reservation is always used consistently (§8.8.1). A refused FREE stays
  tracked and retried (§8.7.5, now model-tested with injected free refusals, including the retire).
- **Gate 10.** `scripts/bench/v3_gates.sh` runs `kf-micro-reserve-probe reserve` after the nine
  `kf-gate*` and prints `GATE10_VERDICT=PASS|FAIL`; the script's exit status requires it.
  `V3_GATES_SUMMARY pass=9 fail=0` keeps its meaning (the nine only; `merge_check.sh` still greps it)
  and `merge_check.sh` / `box/README.md` additionally require `GATE10_VERDICT=PASS`. The probe also
  records, information only, whether host RM accepts the 7.9 GiB flat-alias reservation
  (`reserve_flat_fb_alias`). **Not run here.**

### 8.8.4 Tests (GPU-free)

`cargo test -p kf-mem`: **153 passed** (148 lib + 2 + 3 integration), 0 failed, 2 ignored (was 140:
135 + 2 + 3); `cargo test -p kf-qemu`: 162 passed, 0 failed; `cargo test -p kf-harness`: 3 passed;
`cargo clippy -p kf-mem --all-targets`: no warning in `batch.rs`, `sim.rs`, `sim/tests.rs`.
Added: runtime-fault properties (300 seeds × 60 steps each): reservation allocs refused (136 hits),
reservation frees refused (81 hits, retire retried), a row map failing mid-row (4 138 hits at low=true,
4 488 at low=false), all three together with unmap refusals (2 configs); targeted D1 tests
(§8.8.1); D2 tests (§8.8.2); the flat-alias tests in four host behaviours. The six existing
property configurations (300 × 60) all pass with the stronger invariants. Mutations that make
the suite fail: big leaves placed as one `NV01` mapping each again (all property tests); unbounded
`cut_chunk` (the 2^20 test).

### 8.8.5 Measured vs inferred

| claim | status |
|---|---|
| 0 pages re-made, 0 transients, 0 rigid mappings in 300 seeds × 60 steps × {6 + 5 configs} | measured (model) |
| every ledger lock hold ≤ 2 049 entries on the 2^20-piece row and in the property tests | measured (model + release build) |
| longest hold ~5 ms on a 1M-entry ledger | measured; cause (allocator frees) inferred |
| host RM accepts the reservations D1 asks for (small: probe passed per the delegating session; 7.9 GiB: unknown) | small: reported, not re-run; large: **unknown** |
| a reservation alloc costs ≈ one RM call; 4 KiB grain costs 16/512 calls per 64 KiB/2 MiB leaf | inferred |
| the stale-ledger window during a chunked cut is harmless | inferred |
| act thread blocking time on the ledger in production | not measured (the steer log line now prints it) |

### 8.8.6 Still to verify on hardware (adds to §8.6)

8. `v3_gates.sh`: 9/9 and `GATE10_VERDICT=PASS`; read `reserve_flat_fb_alias` in the log.
9. The 30-arm fast suite at this revision (the flat FB alias goes through the reservation path
   now; watch `kf-mem: micro reservation … refused` lines — a few at most, and none for the alias, or
   the (a2) fallback is what carries it).
10. A Windows boot on the production flags: no Xid 31, no `UNCHANGED page(s) re-made` line (that
    line is now a defect, not a declared case), micro reservation counters sane, the retire never
    reports `could not be released`.
11. A video lane: the steer's log line `ledger lock holds: at most N entries, longest M us`.
12. Windows low-range cost with reservations on (`workload_numbers` predicts 0.49 RM calls/page;
    measure).

### 8.8.7 Residual risks

- **Both reservation routes refused for a row beyond the grain bound** → refused by name (absence).
  For the flat FB alias this is the 6fafcc6e failure class (poisoned kernel CE channels). The decision
  accepted it; (a2) shrinks the exposure to "host RM refuses leaf-sized reservations too". Gate 10 and
  the fast suite tell. If it ever matters, the exact fallback is one more tier (the old one-mapping-
  per-leaf placement, with its declared re-make), which D1 forbids.
- Reservations are one RM object per big-leaf row: a guest that maps many big-leaf rows makes many
  (bounded by the walker's rows; each is a map entry in `micro`).
- The ledger scans of the book still start at `va - max_len` (longest batch ever); the scan is
  chunked (no long hold) but its total work grows with the number of batches below `va`.
- A window between "reservation removed from `micro`" and "host free done" lets the VA thread map
  through the `NV01` range over a VA still reserved on the host (answered HELD) — pre-existing, not
  worsened.
