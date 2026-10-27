# Whitepapers

**STATUS: RESEARCH, 2026-10-27 — the v3 successor is [kayfabe_v3.pdf](kayfabe_v3.pdf).** It is pinned to `origin/next` at `12c4008705f17033a803e56d4d479bf1275cdb94`, with separately identified experimental sources. This writing task ran no GPU or VM. The cold-reader revision states the Windows stall hypothesis before the internals, restores the Windows positives and performance denominators, and separates reported results from unverified analysis. It contains five mechanism diagrams and 36 standalone questions. Host-plus-guest throughput and host responsiveness remain unmeasured; coexistence has screenshot evidence only.

The [companion index](kayfabe_v3_index.md) carries the full source list, 101 historical gap topics and the end-of-revision snapshot of 87 open PRs, including bases and known stack parents. These are frozen metadata and historical topics, not current acceptance results. [Result distances](result_distances_v3.json) expose how far each retained hardware runtime is from the pin; none was rerun on hardware for this edition.

Build the successor from committed sources:

```sh
bash docs/whitepaper/build.sh v3
python3 docs/whitepaper/build_v3.py --check
```

The two-pass XeLaTeX publication gate rejects TeX errors, missing glyphs and undefined references. It requires recorded repository inputs to match committed Git blobs and writes a source/PDF hash receipt. See [publication checks](BUILD_GATES.md), [immutable source index](sources_v3.json) and [PR snapshot](pr_snapshot_v3.json). A receipt is local build evidence, not a signed attestation.

**STATUS: SUPERSEDED, 2026-10-27 — `kayfabe_architecture.tex` / `.pdf`.** The old paper is retained below as history, with this supersession also recorded inside its text. It describes the pre-v3 tree. Its old counts and open verdicts are not the status of v3. The rebuilt archive uses Roman front matter so two fresh TeX passes settle its references; the historical body is preserved.

## Historical paper

It is pinned to Rust HEAD `5c367a38` (2026-08-20), with twelve further commits'
worth of corrections applied in place at `56dc01f3` — including the two that
matter most: sequential multi-process no longer fails, and the "object graph"
wall named in earlier revisions turned out not to exist.

> ### ⊘⊘⊘ RE-STAMPED 2026-09-11 at `5f323dd5` — **seven claims in it were false and are corrected in place.**
>
> **477 commits** landed between the pin and that pass. The structural counts
> (lines, crates, dependency edges, test counts) are **still pinned to `5c367a38`**
> except the four re-derived in the subject block; every *behavioural* claim now
> carries a dated correction where it was wrong. The seven, in the paper's own §1:
>
> 1. **The LLM generates.** *"No LLM has ever produced a token in this guest"* is
>    false. 2026-09-09, rev `c8fe67d6`: 16 tokens, text **byte-identical to the
>    same-boot CPU oracle**, host `Xid` 0. ⊘ At **45 s/token**, graded on a count
>    the project's own requirements mark RED, n=1–2 boots, and **never run under
>    the three blocking constraints**.
> 2. **The blocking model is three owner rules** — nothing blocking in a vCPU trap,
>    nothing blocking under a lock a vCPU takes, and a 1 ms trap budget. New §4.4.
>    Two of the three are met; the budget is *counted, never enforced*.
> 3. **BAR1 and BAR2 default to untrapped.** BAR1 traps 88,193 → 91.
> 4. **The doorbell defaults to deferred, and a doorbell is a hint, not a barrier.**
>    Nothing may gate on one.
> 5. ***"Both invalidate transports measured zero"* was a census gap**, not a
>    property of the path — and the paper had built a normative rule on it. There
>    are **three** synchronisation points, and the third is the interesting one:
>    on a GSP part the mapping happens inside the GSP, **which is us**, so for RM's
>    own map calls no barrier is coming at all.
> 6. **Q2 is answered** — the GSP command queue is serviced off the vCPU entirely.
>    ⊘ And the wait it asked about *did not move*.
> 7. **`gpga.rs` (1,391 lines) has zero production callers**, and the paper listed
>    it inside a BUILT crate without saying so. So does `refresh.rs`.
>
> **The committed PDF is rebuilt from the committed `.tex` at this revision** (84
> pages, build gate clean, two passes, zero undefined references).

It is written to be attacked. Roughly half of it is about what does not work, is
not built, or is not known, and it ends with open questions posed so that a reader
with no access to this tree can answer them. Where it states a number it names the
source; where a claim could not be verified it is marked `[unverified]` rather than
softened.

## Rebuilding

For a publishable rebuild of the historical PDF and its new in-text notice, use the committed-input gate:

```sh
python3 docs/whitepaper/build_v3.py --legacy
python3 docs/whitepaper/build_v3.py --legacy --check
```

Needs **xelatex** (TeX Live) with fontspec and TikZ. Every diagram is native TikZ, so
the output stays vector at any zoom. `lualatex` is deliberately not used. Two passes
are required. The Python publication target records the inputs and checks the final log.
The old default `build.sh` target remains available for historical compatibility.

## Reading order

`§1` says what the document is and how to attack it. `§8` is the uncomfortable one
and is where to start if you only read one section — it is also the longest, which is
the point. `§10` lists claims corrected *while the paper was being written*, and `§11`
the open questions.
