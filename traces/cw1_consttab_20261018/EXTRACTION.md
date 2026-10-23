# CW1 constants data extraction

**STATUS: LIVE, 2026-10-22 (CODE READ; data and pure loader only, no consumers).**

CODE READ: base master `e9c6d39e5`. Source
`codex/sweep-dies-20261022` at `bc3a967e061aa0a267467b59393afaf10eb0ec70`
provides `kf-rm/src/consttab.rs`, `kf-chip/src/die.rs`,
`kf-rm/data/measured_constants.tsv` and `board_specific.tsv`.
Only the two module exports and the `DieId` re-export touch existing production
files. HostFacts, init tables, ZBC, rowserve and display consumers are unchanged.

CODE READ: at commit `bc3a967e0`, 95 observation rows are retained without changing their values or
provenance fields. The software-state 2209 rule is omitted from this measurement
subset; the existing implementation remains unchanged. The 39 ZBC rows retain
source-schema branch `*` for host-installed state, an exact die, and a host
firmware/driver/date tag. That scope does not authorize wildcard guest RM rows.
The measurement-table gaps are TU104, TU117, GA103, GA107, AD107, GB205 and
GB207, plus GA100/Hopper/datacenter dies; no entry is borrowed for them.
`rm.*` includes withheld candidates such as 121f/0709; storing an observation does
not authorize serving it or using captured floorsweeping-dependent state.

CODE READ: the separate caps schema is deferred. Its AD104 source text lacks a
standalone evidence artifact path. D-1 Windows A/B and PR #85 stay held. No part
of the large PR #3 was merged. This is unverified on Windows; GPU/Windows n=0.

CODE READ: the loader adaptation rejects unnamed dies before any wildcard can
answer, checks calendar dates, exact header names/order and relative nonempty
citation paths, and exposes an explicit evidence-path checker for offline
validation. It performs no implicit filesystem I/O. Open-kernel source tags must
name a version; no unversioned citation is introduced.

CODE READ: minimal evidence absent on master is copied byte-for-byte from
[docs PR #94](https://github.com/reindertpelsma/kayfabe/pull/94), revision
`eafe67cc2884f46608995d7b41a9c7965da4907e`; hashes are in
`evidence_sources.tsv`. This branch starts from master independently of that PR.
The common evidence files leave the comparison when that PR lands. The broader
coverage grid and the excerpt provenance are in that PR; no README is changed.

## Hypotheses and falsifiers, stated before tests

INFERRED H1: the source observations can be extracted with exact die, driver,
date and existing evidence, without consumer changes. Falsifier: missing or
mismatched metadata/artifact, or any existing consumer diff.

INFERRED H2: the loader can refuse unknown/absent/neighbor dies and inappropriate
RM branches or requests. Falsifier: a lookup returns a row in any such case, or
invalid provenance/evidence parses successfully.

INFERRED H3: the new contracts are nonvacuous. Falsifier: deliberately breaking a
contract fails fewer than two tests in the corresponding mutation run.

CODE READ: the 26 retained die implementation names/values were checked against
`ogkm-595.84: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080mc.h`.
The falsifier, stated before fetching that header, was any mismatching retained
tuple. None differed. Source/extracted file hashes are in `source_snapshot.tsv`.

## Validation

MEASURED: run `CW1-consttab-20261022` (2026-10-22) results and mutation kills are recorded in
`traces/cw1_consttab_20261018/validation.txt`; no hardware result is implied.
