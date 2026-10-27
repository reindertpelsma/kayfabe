# V3 selective sparse twin mappings (`KF3_SPARSE_TWIN`)

**STATUS: LIVE (default OFF), 2026-10-27.** ENS7 scales the finite object budget with configured guest VRAM, excludes host-owned windows before RM, and reports current missing coverage with bounded warnings: see `docs/design/V3_ENS7_SPARSE_BUDGET.md`. ENS6 (2026-10-27): a host RM `NoMemory` during sparse admission is now a counted closed
fallback with a back-off (not a hard error), reservation work a refresh may start is bounded by a time slice, and per-verb RM
timing is in the heartbeat: see `docs/design/V3_ENS6_RM_COST.md` (it corrects the *Fallbacks* and *Counters* text below where they differ).
(2026-10-26 text follows.) Implementation by lanes SPR1 (tasks 1-3) and SPR2 (harness fix, hardware
validation). Labels: CODE READ (source at the named revision), MEASURED (a named log or run), INFERRED (reasoning).
Design inputs: `docs/design/V3_SPARSE_PTE.md` (the H6 fault), the twin-allocation design notes (sections A, B, D, E),
and the owner contract of 2026-10-26 (see below). Evidence: `traces/sparse_spr1_20261018/` and `traces/sparse_spr2_20261026/`.

## What it does

A guest page-table leaf that the guest wrote with the sparse encoding (VER2 `valid=0, vol=1`; VER3 leaf PCF sparse) is a
THIRD leaf kind beside vidmem and DMA. With `KF3_SPARSE_TWIN=1` the host mirror maps it as a host SPARSE reservation, so a
GPU access to a guest-sparse tile is absorbed (reads give zeros, writes are discarded) exactly as on bare metal. A plain
invalid page still faults: the owner-approved negative controls stay red. The blanket diagnostic `KF3_DIAG_SPARSE_RESV=1`
absorbs both kinds and cannot reserve above 1 TiB (CODE READ: `NoMemory`), so it is not a fix.

## Owner contract (2026-10-26, binding)

Only the changed VAs are undefined; everything else stays accessible; a refresh never pauses the GPU. "Undefined" is
fail-safe closed: a changed VA resolves to the OLD framebuffer offset, the NEW framebuffer offset, a sparse absorb, or a
fault, never random memory. No unmap-then-remap interval: old to sparse and sparse to new are single transitions.

## Parts

1. **Walker and diff (ABI 7).** `cuda/walk/kf_walk.cu` emits a SPARSE run (`KFWR_RF_SPARSE`, no backing address) for small
   and big sparse leaves when the explicit switch in the format descriptor is set; the diff key includes the sparse state
   and a sparse run never merges with a backed run (`crates/kf-cuda/src/diffmodel.rs`). VER2 NV4K (`0x28`) is not sparse
   (privilege clear is required, `kf_mem::sparse::classify_leaf`). PDE-level sparse (VER3 PDE PCF) is counted and stays
   INVALID in v1.
2. **Apply** (`crates/kf-mem/src/apply/sparse.rs`). SPARSE is a mapping state with no backed row. Old sparse intent is
   retired before replacements are admitted (the block is preserved for an incoming mapping, so sparse to new is one host
   map and old to sparse is one host unmap); new sparse intent is admitted BEFORE backed unmaps; a failed entry withdraws
   only its new intent.
3. **Reservation tree** (`crates/kf-host/src/sparse.rs`, `channel/sparse.rs`). The static guest reservations become a tree of
   host reservation blocks split only while EMPTY (the RM refuses a reservation inside a live block). Normal blocks have a
   2 MiB grain, sparse blocks 64 KiB, created on demand for a sparse run whose full 64 KiB tile is derived sparse.
4. **Bounds** (hostile guest). The configured VRAM policy caps each space at65536 blocks and the VM/session at131072 objects (minimum8192); a decline is
   counted and the sparse VA stays INVALID (a plain map is never declined, see below). Counters saturate; log lines are capped (128 per process).
   Nothing on this path runs on a vCPU thread.

## Plain maps are never touched (owner ruling 2026-10-26, binding)

A plain, valid mapping is never refused or changed by the sparse twin. Refining the block tree around a map only
prepares a later sparse carve, so it is best-effort: when the block budget is exhausted, a block cannot be created, the
coarse block is already populated, or RM refuses the split, the plain map is placed in the COARSE block exactly as with
the twin OFF (`Reservations::route_map`, `crates/kf-host/src/sparse.rs`). Only SPARSE runs are best-effort. A map is
still refused for arguments the twin-OFF path also refuses (below the VA floor, wrapping). A block lost to a failed layout
repair is routed through the range object like an unreserved range, and routing is not stopped by a poisoned layout
(only sparse bookkeeping and host auto-placement are). Reservations that are not tree blocks (micro reservations) are not
counted against the session cap. Why this exists: the first Windows campaign (rev `a5d709d5`) failed at the start of
`catia-07` because the map that follows a refused refinement was refused (the 8192-object per-space budget was spent; the
twin's own 2 MiB-tiled windows are a large part of the use, INFERRED from `made=7979 freed=3988` at the first space of a
Linux boot).

## Fallbacks for sparse (all fail closed)

- A sparse run that the budget declines, or that lands in a POPULATED normal block, stays INVALID, is counted and is not
  an error: the ack is HELD (the walker does not retry it every refresh). Every tile created for it is withdrawn.
  A populated-block case is retried at a later refresh once the block is empty.
- A sparse to invalid change inside a sparse block that cannot be freed stays sparse (`retained_bytes`). MEASURED on
  2026-10-26 (RTX 4070, bare metal, copy engine only, 38.4 M words, no non-zero word): sparse is not a data channel
  (experiment E2 of branch `claude/sparse-hw-20261026`; shader loads untested).
- An uncertain layout repair refuses further host auto-placements (`layout_poisoned`).

## Counters (what each says; all saturating events or bytes, cumulative over the session)

`made`/`freed`: tree reservation objects created/released (`made - freed` is the live count). `refused`: host RM verb
refusals (a reserve or release that RM or the session cap refused). `overflow`: SPARSE admissions declined by the object
budget, one per declined admission (before the 2026-10-26 fix it also counted every refinement attempt of a plain map,
which gave 8.3 M on the first Windows campaign: CODE READ). `unhonoured_bytes`: sparse bytes left invalid at admission,
summed per admission (a retried run is counted again). `retained_bytes`: sparse-to-invalid bytes kept sparse.
`coarse_maps`: plain maps placed in a coarse block because refinement was declined, one per map call; they are not
refused. Bounds: the ENS7 configured per-space/per-VM policy above;128 legacy log lines. ENS6 adds `rm_declined` (host RM `NoMemory` turned into
the counted fallback), `backoff_skips` (layout work skipped during a refusal back-off), `slice_deferred` (work left for a later
refresh by the time slice), `speculative_skipped` (plain-map refinement skipped above the high-water mark, no RM verb),
`window_hits` (admissions answered by the refused-window cache), `layout_refresh_peak_{us,verbs}` (busiest refresh), and the
`rmverbs[...]` table (per RM verb class: calls/total us/max us).

## Flag

`KF3_SPARSE_TWIN=1` (exactly `1`): sampled when the host session and the walker are created. Default OFF, never
required to boot. OFF is behaviour-identical to the build without the feature (the walker emits no sparse run; the tree is
not created). The owner decides the default after validation.

## Results (hardware, RTX 4070, rev `0053c83b4`)

Evidence and attribution: `traces/sparse_spr2_20261026/README.md`. In short: the Linux reproducer (Vulkan sparse unbind then a
CE write into the tile) faults with the flag OFF (Xid 31, 2 of 2 boots) and is absorbed with it ON (3 of 3 boots, 0 Xids); the
plain-invalid negative control faults in both states (4 of 4 boots); the native gates pass 9/9 and the fast suite 30/30 in both
states; the session counters read `refused=0 overflow=0`. Not run: Windows, shader loads through a sparse tile.

## Open questions for the owner

1. RESOLVED by the owner ruling of 2026-10-26 (section above): plain maps always succeed.
2. **Default.** Default OFF stays until the Windows one-boot SPEC sequence has run with the counters recorded.
