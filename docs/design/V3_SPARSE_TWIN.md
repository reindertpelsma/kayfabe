# V3 selective sparse twin mappings (`KF3_SPARSE_TWIN`)

**STATUS: LIVE (default OFF), 2026-10-26.** Implementation by lanes SPR1 (tasks 1-3) and SPR2 (harness fix, hardware
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
4. **Bounds** (hostile guest). At most 8192 blocks per space and 32768 reservation objects per session; a refusal is
   counted (`refused`, `overflow`) and the VA stays INVALID. Counters saturate; log lines are capped (128 per process).
   Nothing on this path runs on a vCPU thread.

## Fallbacks (all fail closed)

- A sparse run inside a POPULATED normal block stays INVALID, is counted (`unhonoured_bytes`) and is retried at a later
  refresh once the block is empty.
- A sparse to invalid change inside a sparse block that cannot be freed stays sparse (`retained_bytes`). MEASURED on 2026-10-26 (RTX 4070, bare
  metal, copy engine only, 38.4 M words, no non-zero word): sparse is not a data channel
  (experiment E2 of branch `claude/sparse-hw-20261026`; shader loads untested).
- An uncertain layout repair refuses further host placements (`layout_poisoned`).

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

1. **Object exhaustion during a plain map.** CODE READ (`crates/kf-host/src/sparse.rs`, `Reservations::normal`; test
   `normal_object_exhaustion_never_places_a_backing_in_a_coarse_block`): when the block budget is exhausted, a plain map that
   needs a split is REFUSED rather than placed in the coarse block. That fails closed but also faults a legitimate mapping.
   Alternative: place it in the coarse block (that block can then never be split, so sparse inside it stays invalid and
   counted). Recommendation: the alternative, decided after the Windows run shows how close to the cap the counters get
   (a Linux boot uses about 4000 live objects).
2. **Default.** Default OFF stays until the Windows one-boot SPEC sequence has run with the counters recorded.
