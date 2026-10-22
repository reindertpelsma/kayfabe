# DR1 validation and reading guide

**STATUS: RESEARCH, 2026-10-23.**

CODE READ: this lane starts at origin/master e9c6d39e5 and imports C1’s parser and 27 tests from PR #87. The active gate extends that parser; C1’s metadata patch and nine judgement decisions remain with that PR. CW1 PR #94 owns the constant-strategy status. No README, AGENTS or archive file is edited.

INFERRED H1, registered before implementation: editing a legacy document must invalidate its exemption. FALSIFIER: edit a baselined document without clearing its finding and obtain a passing gate. MEASURED (2026-10-23) at traces/doc_rot_20261018/mutations.json: bypassing the content hash fails two tests.

INFERRED H2, registered before implementation: a linked successor preserves navigation. FALSIFIERS: missing, external, self or directory successors pass. MEASURED (2026-10-23) at traces/doc_rot_20261018/mutations.json: bypassing the successor check fails five tests.

MEASURED at tests-first commit 1b8c2373f: the initial 50 contracts yielded 47 failures and two errors before implementation. MEASURED (2026-10-23) at traces/doc_rot_20261018/validation.json: the final complete CI Python suite passes 147/147; each mutant run executes 108 documentation contracts. The checker SHA-256 in that file identifies the working source for these results.

MEASURED (2026-10-23) at traces/doc_rot_20261018/validation.json: the 2026-10-23 inventory covers all 638 docs/ files plus two root documents: 171 active Markdown, 194 archived files and 275 assets. There are 61 legacy findings, including two STALE-SUSPECT documents and zero broken local file/heading links. The initial baseline has 139 diagnostic/reference identities and new=0. Claims debt is 3758 existing, new=0, resolved=0; its baseline is unchanged.

CODE READ: [inventory.md](inventory.md) is the full per-file report and proposed action list. [owner-decisions.md](owner-decisions.md) adds source-specific semantic decisions beyond C1. The workflow publishes the same report as a job summary and downloadable artifact weekly and on docs PRs; the committed inventory is a dated snapshot, with working-tree changes explicitly indicated.

## Mutation kills

| Broken behavior | Failing tests | Errors |
|---|---:|---:|
| metadata | 9 | 0 |
| successor | 5 | 0 |
| file-links | 12 | 0 |
| age-and-references | 21 | 0 |
| changed-document-hash | 2 | 0 |
| env-references | 10 | 0 |
| flag-references | 2 | 0 |
| crate-references | 2 | 0 |
| branch-references | 5 | 0 |
| path-references | 3 | 0 |
| heading-fragments | 2 | 0 |
| archive-exclusion | 2 | 0 |
| stale-reference-identities | 2 | 0 |
| crate-reference-context | 3 | 0 |
| absolute-host-paths | 3 | 0 |
| asset-classification | 2 | 0 |

MEASURED (2026-10-23) at traces/doc_rot_20261018/mutations.json: all 16 mutants exit with test failures, each in at least two tests; all have zero test errors. The mutation runner restores the original source in a finally block; the final source digest is recorded in validation.json.

CODE READ: semantic applicability, external-source references and non-Markdown fragments remain outside the mechanical verdict. No runtime or hardware conclusion follows from this GPU-free documentation lane. No archive move or deletion is proposed automatically.


MEASURED (2026-10-23) at tests-first commit 6ad935677: three of 108 documentation contracts fail before the absolute-host-filename fix. The new mutant disables that distinction and fails three tests. MEASURED (2026-10-23) at traces/doc_rot_20261018/validation.json: the unchanged private-path scanner reports zero findings after report regeneration; its SHA-256 is recorded there. Root-relative Markdown destinations remain checked.
