# whitepaper

**`kayfabe_architecture.pdf` is the current paper. Read that one.**

It is pinned to Rust HEAD `5c367a38` (2026-08-20), with twelve further commits'
worth of corrections applied in place at `56dc01f3` — including the two that
matter most: sequential multi-process no longer fails, and the "object graph"
wall named in earlier revisions does not exist.

## The `.tex` in this directory is older than the PDF

`kayfabe_architecture.tex` here is an **earlier source** and will **not** rebuild
the committed PDF. It still contains claims the PDF has since retired, notably:

> `:2911` — "The multi-process property is unmeasured on hardware, and it is the
> founding requirement…"

That is **superseded**. Two concurrent guest CUDA processes both computed `43` on
2026-08-14 (w299, rev `f459cffa`), including a staggered arm that starts the second
process inside the first's `cuCtxCreate`. The PDF records this; the `.tex` predates it.

Where the two disagree, the PDF is newer. Where a dated measurement disagrees with
prose, believe the measurement — that rule is the paper's own (§8.21, "claims that
were true *once*").
