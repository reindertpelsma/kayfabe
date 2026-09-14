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
| `kf_tests.cu` | 54 cases: correctness, change detection, **round-trip closure**, hostile, racing, scope, differential |
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

## ★★★★★ Delta round-trip closure

    apply(model, deltas_from_walk_N) == full_walk_N        for all N

The nine `delta/*` cases each build one change and assert one expected delta.
Nothing accumulates, so nothing checks **closure** — and a stream of individually
plausible deltas can drift silently. `roundtrip/*` does the accumulating: keep a
host-side model of the mapping set, then for hundreds of seeded random steps
mutate the tables, walk, **apply the report to the model**, and compare against a
fresh full walk of the same tables.

`[measured 2026-09-14, RTX 3060]` **2000 steps over 8 seeds, 2000 closures
checked, 1105 non-empty deltas, 1525 steps that changed the mapping set, 416
steps with BOTH halves of a dual PDE live, max model 392 mappings, deepest walk
10 380 entries.** Benign and hostile streams, plus one under a live racer.

Three things make it worth more than the cases it complements:

- **It is not coupled to the shadow.** "A fresh full walk" comes from a *second*
  walker that is **never acked**, so every report it emits is a `RESYNC`, i.e.
  the full current state. If the open design question settles on the kernel
  returning state instead of deltas, `rt_full_state` is unchanged and
  `model_apply` becomes the host-side diff this same loop tests.
- **It asserts order-independence, not just closure.** Every report is applied
  forwards *and backwards* and both must give the same model. That is a stronger
  property than "apply them in the order given", and it is what the per-class
  segment diff exists to provide.
- **It says when it was vacuous.** Closures checked, deltas that were non-empty,
  steps that actually changed the mapping set, steps where both dual-PDE halves
  were live, maximum model size and deepest walk are all asserted above floors.
  A stream that scribbled the root would pass every closure check while the model
  stayed empty; these are what stop that reading as a result.

`roundtrip/under_racer` is **deliberately scoped**: under concurrent mutation
there is no "the tables at step N" for a second walk to agree with, so per step
it asserts only well-formedness and order-independence. The exact claim is made
once, at the end — stop the racer, take one more delta, apply it, and the model
must equal a fresh full walk of the now-quiesced tables. A racing round-trip that
asserted a specific value would be asserting nothing.

`roundtrip/truncated_is_never_a_delta` closes the loop on I3: a `TRUNCATED`
report is refused by the applier, the walker is not acked, and the next report is
a full resync that reconstructs the state exactly — **and the case demonstrates
the divergence**, by applying the truncated report to a copy of the model and
asserting the copy is now wrong.

### The five known-positives

`make check-negative` and `make check-closure-negative` rebuild the suite with
one decision removed at a time and **require the corresponding cases to fail**,
with an unbroken control each time. Without them these are all green over checks
nobody has ever seen fire.

| flag | removes | must break |
|---|---|---|
| `-DKF_BREAK_BOUNDS` | the two bounds checks | `hostile/pointer_past_end`, `hostile/bounds_window_respected` |
| `-DKF_OLD_MERGE` | restores the superseded whole-run merge join | `roundtrip/*`, on order-independence |
| `-DKF_BREAK_ORDER` | walks the big table downwards, so one class comes out descending in VA | `roundtrip/*`, on order-independence |
| `-DKF_DROP_UNMAP` | drops every `UNMAP`, and nothing else | `roundtrip/*`, on **closure itself** — and it must *not* disturb order-independence |

★ Each step also greps for *which* assertion fired: a negative control that fails
for a different reason than intended is worth nothing, and `-DKF_OLD_MERGE` and
`-DKF_BREAK_ORDER` both trip the order assertion *before* the closure one, which
is why `-DKF_DROP_UNMAP` exists at all — it is the only one that reaches
`apply(model, delta) != full walk`.

★ What `-DKF_BREAK_BOUNDS` showed is worth keeping: with the check removed, one
case **silently returned data from beyond the declared window** (it reported a
mapping at `gpga=0xdead000` that lives past the end) while the other raised *"an
illegal memory access"*. **An out-of-bounds read does not reliably fault.** That
is the whole argument for making the check structural rather than probable.

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
5. **The doc does not say how a dual PDE's two halves are ordered**, and a report
   "produced already sorted" needs a total order because both sub-tables cover the
   *same* 2 MiB of VA. The walk emits **big before small inside each 64 KiB
   chunk** — (VA ascending, page size descending).
   ⊘ An earlier draft of this file said *"the diff's comparator must be the same
   one"*. **That coupling is now dissolved rather than merely verified**: the diff
   is computed **per page-size class**, so it does not depend on the cross-class
   order at all. What it does depend on is each class being ascending in VA, and
   that has its own known-positive (`-DKF_BREAK_ORDER`) and is asserted by 416
   round-trip steps in which both halves were live at once.

7. **A whole-run merge join does not satisfy closure, and the doc's format
   invites one.** Diffing prev against cur by comparing whole runs on the key
   `(va, page size)` — the obvious reading of *"a merge join — linear, no hashing,
   no allocation"* — emits, for a cur run that covers several prev runs, one
   `REMAP` followed by `UNMAP`s of the runs it has just replaced. Applying that
   report in order **loses those mappings**. All nine single-step `delta/*` cases
   pass against it. The diff is now computed **per page-size class at segment
   granularity**: `UNMAP` where prev covers and cur does not, `MAP` where cur
   covers and prev does not, `REMAP` where both cover and differ.

8. **⇒ A report's runs are order-independent, and the doc should say so.** Under
   the segment diff the `UNMAP` set and the `MAP`/`REMAP` set are disjoint in
   `(va, page-size class)` **by construction**, so a host may apply a report in
   any order — asserted directly, by applying every report backwards as well as
   forwards. ⊘ A consequence: the **report's** own order is now (page-size class
   ascending, VA ascending within a class), which is *not* the walk's order. The
   format doc's "produced already sorted" describes the kernel's table, not the
   delta.
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
- **The diff is single-threaded**, because it is a linear scan over two sorted
  lists per page-size class, producing a dense report.
- **`GONE` carries no runs.** The doc leaves the choice open; a whole-VAS teardown
  is one flag, not N unmaps.

## ★★★★★ TURING+ IS A HARD REQUIREMENT — what is met, and the one thing that is not

**Owner, 2026-09-14, stated three times:** *"our ptx must be Turing+ compatible."*

### ✔ Met and verified on the PTX side

| | status |
|---|---|
| compile target | `-gencode arch=compute_75,code=compute_75` — **Turing**, PTX embedded, **no cubin** |
| "it JITs forward" is checked, not believed | `make check-ptx` fails if `cuobjdump -sass` finds any `code for sm_` |
| arch-specific intrinsics | **none** — no `__shfl`/`__ballot`/`redux`/`cp.async`/`mbarrier`/cluster ops |
| forward-JIT demonstrated | ran on **sm_86** from the `compute_75` target |

⚠ **Never run on actual Turing silicon (sm_75).** sm_75 is the floor we claim; a container-hour
closes it. Until then "Turing+" is *"compiled for Turing, proven forward to Ampere"*.

### ⊘⊘⊘ THE REAL ARCHITECTURE LIMIT IS THE FORMAT, NOT THE PTX

This kernel hardcodes **VER2**, which covers **Turing → Ada**. **Hopper and Blackwell use VER3**
(PCF; `grep -c PCF` on the Pascal/Turing headers is 0). ⇒ **`grep -niE 'ver3|pcf' kf_walk.cu` →
nothing.**

★★★ **And that bears directly on deleting the host parsing code.** The host walker is
**format-polymorphic** — it takes `fmt: &dyn GmmuFmt` (`kayfabe-fwd/src/ptdecode.rs`,
`kayfabe-mmu/src/reach.rs:640`), so VER3 there is **a new impl, not a rewrite**. This kernel has no
such seam.

⇒ **Deleting the host walker today would delete the abstraction Blackwell needs**, and Blackwell is
goals 1 and 10 of the standing directive.

### ⇒ Sequencing

**Add the format seam to the kernel FIRST, then delete the host parsing.** A template specialised
per format, or a format enum switched at each decode site — cheap while only one format exists,
and much worse to retrofit into a working walker later. Same destination; it just stops the
deletion from quietly capping the product at Ada.
