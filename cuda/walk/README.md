# `cuda/walk` — the walk kernel, and the suite that attacks it

**STATUS: LIVE (2026-09-14).** A standalone CUDA proving ground for the kernel of
`docs/design/dirty_tracking_without_uffd.md` §w720e, emitting the report of
`docs/design/the_walk_kernel_report_format.md`. Bare metal, no VM, no guest,
nothing from the kayfabe runtime — a GPU, a buffer, a kernel and assertions.

| file | what |
|---|---|
| `kf_walk.h` | the report ABI (the format doc's 64/32/32 structs) + the host API |
| `kf_walk.cu` | the walk, the coalescer, the merge-join diff, the report |
| `kf_tables.h` | a **host-side** GA10x VER2 table builder — test scaffolding |
| `kf_tests.cu` | 50 cases: correctness, change detection, hostile, racing, scope, differential |
| `kf_corpus.cpp` | builds the differential corpus (host-only, `g++`, no GPU) |
| `corpus/` | `corpus.bin` (15 images) + `rust_leaves.txt` (the Rust walker's decode) |
| `run_on_box.sh` | ships the source out of a commit, builds and runs, returns a log |

The Rust half of the differential is
`crates/kayfabe-mmu/tests/walk_kernel_differential.rs`.

## Building

```
make check          # invariants, build, PTX-only check, known-positive
./kf_tests          # the suite;  ./kf_tests hostile   filters
./run_on_box.sh tag # from a workstation, against a disposable CUDA container
```

`nvcc -gencode arch=compute_75,code=compute_75` — **PTX only, no cubin**, so the
driver JITs forward onto any Turing-or-later part. `make check-ptx` asserts the
binary contains no SASS, which is what makes "it JITs forward" a checked claim
rather than a compiler flag someone believed in.

## The three invariants

They are properties of the **shape of the code**, and `make check-invariants`
asserts each one in the source rather than arguing for it.

**I1 — no loop terminates on guest data.** The descent is five literally nested
`for` loops with format-constant trip counts (4, 512, 512, 256, 32×16), one per
level. No `while`, no `goto`, no recursion on the guest-data path, and no way to
express a sixth level. ⇒ **A cycle is harmless rather than detected**: a PDE
pointing at its own page is read again one level down and then the nesting runs
out. `KFWR_R_TOO_DEEP` exists in the ABI as the name of a refusal this design
does not need, and every hostile case asserts it is **never set**.

**I2 — every dereference is preceded by a bounds check.** `KF_GPGA_DEREF` is the
only expression that touches the buffer; it appears exactly once outside its own
definition, inside `kf_load64`, immediately after the compare. A table's extent
and alignment are checked once more at each descent, so a table that merely
*straddles* the end is refused before its first entry.

**I3 — output is capped and truncation is loud.** Three independent caps (runs
per address space, entry budget per walk, runs in the report), each setting
`KFWR_HF_TRUNCATED` and its own `refuse_mask` bit. **A truncated walk does not
install its table**, so it can never become the baseline a later delta is
computed against; the next refresh is a full resync.

### The known-positive

`make check-negative` rebuilds with `-DKF_BREAK_BOUNDS`, which deletes the two
bounds checks **and nothing else**, and requires `hostile/pointer_past_end` and
`hostile/bounds_window_respected` to fail, with `correctness/single_4k` on the
unbroken binary as the control. Without it those two are green over a check
nobody has ever seen fire.

★ What it showed is worth keeping: with the check removed, one case **silently
returned data from beyond the declared window** (it reported a mapping at
`gpga=0xdead000` that lives past the end) while the other raised *"an illegal
memory access"*. **An out-of-bounds read does not reliably fault.** That is the
whole argument for making the check structural rather than probable.

## What the format doc got wrong or left unbuildable

1. **`struct ReportHeader { // 64 B }` lists fields summing to 56.** Nothing can
   be built from a contradiction. This header adds `refuse_mask` (the doc's
   `reserved`, given a job) and 8 bytes of `pad` to reach the stated 64.
   `refuse_mask` is what lets a hostile test assert the refusal is *the one it
   intended* rather than merely that something was refused.
2. **"every `first_run + run_count <= run_count`"** is a typo for
   `<= header.run_count`. Implemented as the latter.
3. **A large leaf's target need not be aligned to its own page size.** VER2
   carries a 4 KiB-granular address field at *every* leaf level, so a hostile
   guest can spell a 512 MiB page whose physical base is 4 KiB-aligned and
   nothing else. The doc's validation rule *"every `len` non-zero and
   page-aligned"* has no counterpart for `gpga`, and the encoding is why it
   cannot have one. The kernel refuses such a leaf by name
   (`KFWR_R_MISALIGNED_LEAF`) rather than reporting a mapping nobody can stand
   behind. ⚠ It was found by the doc's own property-3 validator, not by an
   expectation — the validator caught a case the test author had not thought of.
4. **"an unaligned pointer" is inexpressible below the root.** Every VER2 table
   pointer is a bit-field shifted by exactly the bits its target's size needs
   (PDE `<< 12` into a 4096 B table; the dual PDE's big half `<< 8` into a 256 B
   table), so the alignment half of the check can only fire on the **root**,
   which arrives from outside the format. `hostile/unaligned_inexpressible`
   keeps that as a checked claim so a future format change breaks a test instead
   of silently making the check reachable.
5. **The doc does not say how a dual PDE's two halves are ordered.** Both sub-
   tables cover the *same* 2 MiB of VA at different page sizes, so a report
   "produced already sorted" needs a total order. This kernel emits **big before
   small inside each 64 KiB chunk** — i.e. (VA ascending, page size descending) —
   and the diff's merge join uses exactly that comparator. A merge join is only
   correct over the producer's own order, so the two cannot be chosen separately.
6. **Scope pruning happens at 512 MiB granularity and no finer** (whole PD1
   subtrees and above). A hint is widened to that granule before use, which makes
   every carry-forward boundary aligned to a whole PD1 slot, so a clipped run can
   never be cut below its own page size. The doc's asymmetry — *the hint can only
   make the walk faster, never wrong* — is asserted directly by
   `scope/cannot_make_the_walk_wrong`.

## Deliberate scope limits

- **One thread per address space.** Correctness and a deterministic, already-
  sorted run order came first; the doc's *"a full walk is ~67 µs"* is not a claim
  this implementation makes. A parallel decomposition over (PD3, PD2) subtrees is
  the obvious next step and would need a boundary-joining compaction.
- **The diff is single-threaded**, because it is a linear merge over two sorted
  lists producing a dense, ordered report.
- **`GONE` carries no runs.** The doc leaves the choice open; a whole-VAS teardown
  is one flag, not N unmaps.
