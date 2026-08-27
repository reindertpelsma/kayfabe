# whitepaper

**`kayfabe_architecture.pdf` — the architecture paper.** Start there; it is the
best single description of the project that exists.

It is pinned to Rust HEAD `5c367a38` (2026-08-20), with twelve further commits'
worth of corrections applied in place at `56dc01f3` — including the two that
matter most: sequential multi-process no longer fails, and the "object graph"
wall named in earlier revisions turned out not to exist.

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
