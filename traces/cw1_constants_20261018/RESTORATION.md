# CW1 constants documentation restoration

**STATUS: LIVE, 2026-10-22 (CODE READ extraction; no hardware run).**

CODE READ: base master `e9c6d39e5`; branch
`codex/cw1-constants-docs-20261022`. The source list, original SHA-256, restored
SHA-256 and selection rule for each artifact are in `source_manifest.tsv`.
Every source revision is pinned; other agents' branches were read only.
The matrix has a scope correction above its historical status and local citations
clarifying its original claim labels. The coverage page is the current inventory.

## Selection rules

| manifest selection | transformation |
|---|---|
| `verbatim` | complete original text bytes |
| `without-scan-ladder` | omit only lines beginning `SCAN ` that contain ` size=`; retain the range/accepted-count summaries and every non-SCAN line |
| `rm-controls-only` | retain TSV header and original rows for 20800a59, 20802a08, 20802806, 20802801, 2080121f, 50700709 |
| `summary-without-zbc-columns` | retain original summary columns 0..15 and 20..23; omit huge scan lists and ZBC arrays; raw per-die ZBC evidence is retained separately |
| `caps-page-only` | retain from `# kf-display-caps-page` through the first `# end words=…` line; omit module-install/setup output before and after the page |
| `counts-only` | omit `== ` lines naming original host log paths; retain all counter lines |
| `historical-matrix-with-scope-correction` | imported matrix with the explicit branch/master correction and claim citations |

CODE READ: this is a selected evidence restoration, not an archive of every file in
those source trees. Binary captures, images, compressed logs, full large body dumps
and unrelated acceptance logs are omitted. Historical references to an omitted file
are references into the pinned source branch, not a claim it exists on master.
No probe executable or runtime consumer is introduced. No README is changed.

## Hypotheses and falsifiers, recorded before checking

INFERRED H1: the cited constant rows can be backed by small public text artifacts.
Falsifier: a cited file is absent from its source revision, or its selected text
cannot support the row. Result: the cited ZBC and native caps paths were found;
Windows `rm.*` paths are backed by the selected command/status body rows.
The legacy GA106 cudart/8162 evidence already exists on master as source/trace.
AD104 caps has source-tag text and home captures, not a new standalone file here.

INFERRED H2: TU104, TU117, GA103, GA107, AD107, GB207 remain gaps in the
published constant inventory. Falsifier: any same-die table row or matching trace
artifact in the fetched origin tip trees. CODE READ result: 552 refs, 16 unique
table blobs, no matching non-comment row or trace pathname; see coverage §gap
recheck for the precise search scope. This is not a global absence proof.

INFERRED H3: this PR changes documentation/evidence only. Falsifier: any changed
path outside `docs/` and `traces/`, or a changed compiler/Clippy input. The local
validation receipt records the result; no behavior mutation campaign applies.

## Validation

MEASURED: run `CW1-docs-20261022` (2026-10-22) results are in `validation.txt`. Hardware n=0;
historical hardware results stay attributed to their original files/revisions.
No new Windows, GPU acceptance, runtime table validation or wiring result is claimed.
