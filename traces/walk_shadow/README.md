# §6 step 1 — the shadow comparison, and the one number its live half turns on

**Measured 2026-09-15**, vast instance `51063567` (RTX 3060 / GA106, driver **580.159.04**),
binary **`kayfabe-rev:1c5d1be06c5d5639c5798cc797de5a3559a5c850`**.

## What was built

`kayfabe_mmu::walkshadow` — the host walk's leaves as `walkdiff::Run`s, the comparison against
the kernel's report, and a census counting disagreements **by kind**. 12 tests, all green,
including every known-positive the census's zero depends on. **Nothing published changes.**

## ★★★ The number this boot exists for

```
BAR-MIRROR MECHANISM AT END OF RUN: … arena[pages live=2431 peak=2431 allocations=4103
  span_pages=3087533 store_refused=0 store_migrated=0 store_read_refused=0 store_resets=0 …]
[client] W392D_GUEST_OUTCOME=(P)  THREADS 8 of 8 ✔  MEAN_FALSIFIER=PASS
```

**`store_refused=0`.** Over 4103 allocations across a full raw-client boot, the page arena
refused **zero** framebuffer pages — none fell back to the heap.

⇒ **The memfd route for the live shadow is VIABLE.** The blocker was that `SparseFb`'s pages
are each *"on the heap **or** in the arena"*, so granting the isolate the arena's sparse memfd
could have handed it only part of the framebuffer — and a kernel walk over a partial image
follows a zero PDE and reports *nothing*, i.e. `missing_in_kernel` for mappings that are not
missing. On this workload there is no such part.

⚠ **One boot, one workload.** `store_refused == 0` must be a **checked assertion beside the
grant**, not an assumption: the counter exists precisely because the arena *can* refuse, and a
boot where it does would silently make the shadow lie in its most serious direction. ⊘ The
honest shape is: grant the fd, assert the counter is zero at every use, and refuse the shadow
(not the boot) if it is not.

## What remains for the live half

1. Grant the scratchpad isolate the arena's memfd — the `with_guest_ram` shape, within its
   precedent (that already hands the isolate the guest's entire RAM).
2. A wire verb: the child enumerates the memfd's resident extents (`SEEK_DATA`/`SEEK_HOLE` on
   a sparse file), uploads them to a device buffer, launches the committed PTX, returns the
   report bytes.
3. Hook at `SharedDevice`'s sweep **EXECUTE** phase — verified to run with **no lock held**,
   so a synchronous isolate round trip there is legal and no deferred queue is needed.

## Files

- `boot_i6arena.log` — the harness output, including the client grade.
- `arena_census.txt` — the arena line verbatim.
