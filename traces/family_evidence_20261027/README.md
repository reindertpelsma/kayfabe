# Family evidence collection

STATUS: LIVE, 2026-10-27. L0 archival import; GPU-free and data only.

CODE READ (2026-10-27; [coverage page](../../docs/design/V3_FAMILY_COVERAGE.md)):
the input audit distinguishes MEASURED M, MEASURED Mb, CLAIMED C and INFERRED I.
Those classes remain explicit. Historical results qualify their recorded build and
configuration; no current-master, full-feature or Windows stability bar is inferred.

MEASURED (2026-10-27; [validation](validation.json)): 277 bounded text artifacts were
imported at their original paths, totaling 672533 bytes after sanitization. There
are no new binaries, VM images, firmware/VBIOS bodies, executable kits or GPU runs.
The import carries results, suite outputs, native-reference audits and provenance.

CODE READ (2026-10-27; [manifest](manifest.json)): source commits, original Git blobs,
source/import SHA256 values, sizes, redactions and formatting changes are recorded
per file. Network addresses, private filesystem paths, GPU UUIDs and serial fields
are masked. Twenty-three files have whitespace/line-ending normalization. One
historical RESULT.md has its original date/lease repeated in MEASURED paragraphs
for the current claims gate. Numeric JSON fields and original verdicts are retained.

| Evidence source | Imported files |
| --- | ---: |
| codex/sw5-20261024 | 55 |
| codex/sweep-ada-20261022 | 29 |
| codex/sweep-dies-20261022 | 27 |
| claude/tur1-20261027 | 71 |
| codex/wc1-20261026 | 9 |
| codex/bw6b-20261024 | 32 |
| codex/bw7b-20261024 | 10 |
| claude/bw11-20261026 | 2 |
| claude/bw4-b1n-20261024 | 13 |
| claude/bw5-urtfix-20261024 | 29 |

CODE READ (2026-10-27; manifest requested-source checks): SW3 already exists on the
pinned master and is reused. The requested blackwell-fixes ref names the product
build; its GB205 lease57503040 result was found on the BW5 evidence branch named in
the supplied audit. Both identities remain separate. Large logs and executable
operator files were excluded rather than copied wholesale.

CODE READ (2026-10-27; [revision checks](revisions.json)): each of 35 revisions has
an independent ancestry result against the pinned master and next tips. This is
source reachability, not new hardware validation. The 20-die [matrix](matrix.json)
records unknown and dirty tree identities explicitly.

The BW6 failure, same-lease Linux baseline, same-machine BW7 native Windows hold,
and different-machine BW11 non-reproduction are available together. Historical
source interpretations retain their labels and UNVERIFIED-HOST/OBSERVATION markers.
L0 adopts no GB202 attribution verdict. TU106's expanded-budget result remains
separate from the original 180-second oracle. TU106/GA106 Windows TDR storms remain
visible; Code 0 and an idle hold do not establish application stability.
