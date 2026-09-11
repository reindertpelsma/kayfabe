# whitepaper

**`kayfabe_architecture.pdf` — the architecture paper.** Start there; it is the
best single description of the project that exists.

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
>    nothing blocking under a lock a vCPU takes, and a 1 ms trap budget. New §3.5.
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

Source and PDF in this directory are in sync — the committed PDF is built from the
committed `.tex` (verified by checksum).

```sh
./build.sh
```

Needs **xelatex** (TeX Live) with fontspec and TikZ. Every diagram is native TikZ, so
the output stays vector at any zoom. `lualatex` is deliberately not used. Two passes
are required, which `build.sh` does for you.

## Reading order

`§1` says what the document is and how to attack it. `§8` is the uncomfortable one
and is where to start if you only read one section — it is also the longest, which is
the point. `§10` lists claims corrected *while the paper was being written*, and `§11`
the open questions.
