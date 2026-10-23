# Publishing SPECviewperf results

**STATUS: LIVE, 2026-10-23.** Owner decision of 2026-10-23: SPECviewperf scores from kayfabe's
virtualised runs may be public, under the rules below. This file is the publication rule;
`docs/DOWNLOADS.md` (open PR #106) covers fetching the benchmark and its licence.

## Rules

Every SPECviewperf number published in this repository (tables, READMEs, PR bodies, public CI
logs) is a non-compliant number under SPEC's Fair Use Rule. It may be published only when
**every number carries `(nc)`** and **the same page carries this disclaimer, verbatim**:

> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not
> submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance
> Evaluation Corporation, https://www.spec.org

Name the exact SPECviewperf release (never "latest", never a retired release) and state the
configuration: host GPU, virtualisation layer, guest OS and driver. A ratio between two
configurations is allowed only under the heading `(nc) research comparison, not a SPEC-ranked
result`; never rank a number against spec.org results and never call it an official score.
Never print proprietary application content (models, screenshots) from the workloads. 3DMark,
Steel Nomad and Geekbench scores stay private. Raw logs and JSON evidence are left unedited.

## Tooling

Tables come from `scripts/bench/specviewperf/spec_stamp.py`, never from hand-typed text.

- `result_table.py` requires `--release` and `--config` and writes stamped CSV (a `# ` header
  block holding the release, the configuration and the disclaimer; `(nc)` after each score).
- `spec_stamp.py compare --arm NAME=scores.csv ...` writes a stamped comparison. The ratio
  column exists only with `--ratios --baseline A --candidate B`.
- `spec_stamp.py check FILE...` exits 1 for a table without the disclaimer, without an exact
  release line, with an unlabelled number, or with a ratio column not headed as above.
- `mutate_spec_stamp.py` weakens each rule once and requires the tests to fail.

Tests: `python3 -m unittest discover -s scripts/bench/specviewperf -p 'test_*.py'`.
