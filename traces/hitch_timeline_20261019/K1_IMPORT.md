# H1 import contract status

STATUS: PENDING FORMAT, 2026-10-20. No runtime H1 recorder is authorized.

CODE READ: read-only fetch of `codex/kf-prof-20261019`, pinned revision
`9b5f12f292c7ede0867b2834d7be427eda15d5e1`, design
`docs/design/V3_PROFILER_20261019.md`. The design explicitly calls wire v1 a
proposal and K1 records its stage-2 stop. `crates/kf-prof/src/lib.rs` has core
types, not a JSON writer/reader. No complete canonical JSONL object or fixture
is published at this revision. `schema:1` alone is insufficient for an importer.
H1 has not guessed a format or treated the mmap binary layout as JSON fields.

CODE READ (coordinator ownership): K1 supplies `prof.jsonl` / `prof.mmap`, one
CLOCK_MONOTONIC clock, runtime hooks and collectors. H1 supplies offline analysis
and will add a tests-first importer when K1 publishes the actual object schema.
This resolves ownership of the historical [STOP.md](STOP.md) collection issue;
it does not prove that collection or causal correlation has been implemented.

INFERRED importer acceptance requirements, for the published format:

- Pin schema/version and revision; test against an actual K1-generated fixture.
- Reject unsupported clocks/schema, invalid fields and ambiguous sessions rather
  than silently coerce them. Preserve clock calibration/error and boot identity.
- Carry loss, coverage gaps, stale/partial records and rotation discontinuities
  through to UNKNOWN attribution and unknown exact rates.
- Require actual individual broker presentations with verified content IDs;
  never reconstruct frames from per-second fps, copy counts or output sequences.
- Preserve command instance plus device/generation identity, distinct hold,
  row-apply/act-resolve and successful-post edges; attempts do not close waits.
- Use explicit guest calibration and observed guest waiter linkage; handles,
  temporal overlap and generic nvlddmkm event 153 cannot supply missing facts.
- Keep incomplete/pending evidence and unsupported sources visible. Do not
  grant causal coverage just because a well-formed JSONL record was read.

MEASURED: H1 only read the remote branch, without modifying K1 files or messaging
its session. No profiler binary, host collector, VM/guest query or GPU operation
was run for this compatibility inspection. Part B remains NOT GO.
