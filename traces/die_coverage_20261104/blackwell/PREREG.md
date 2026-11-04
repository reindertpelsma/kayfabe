# Blackwell die coverage on Vast, 2026-11-04: pre-registration

STATUS: LIVE, 2026-11-04. Written before any result of this lane was read (leases created 00:08Z, no run
started). Revision under test: `origin/claude/integration-20261104` at `0c515b3b3`. Labels in the results
README: MEASURED = a line in a file of this directory, named; INFERRED = reasoning.

## Leases (own ids only; teardown by these ids)

| die | SKU asked | Vast instance | offer | created (UTC) |
|---|---|---|---|---|
| GB202 | RTX 5090 | 59031351 | 53366467 | 2026-11-04T00:08:44Z |
| GB205 | RTX 5070 | 59031353 | 58781487 | 2026-11-04T00:08:47Z |
| GB206 | RTX 5060 Ti | 59031356 | 49733338 | 2026-11-04T00:08:49Z |

GB203 is not repeated (passed on the earlier tree). GB207 (RTX 5050): no offer at 2026-11-04T00:07Z in the
`vms_enabled=true` search; recorded as unmeasured unless one appears.

## Hypotheses and falsifiers

- **H-gates (per die):** on `0c515b3b3` each die passes `V3_GATES_SUMMARY pass=9 fail=0`, `KF3_RC=0`,
  `FAST_SUITE_PASS=30` and `BIRTH_CENSUS_OK`. Falsifier: any gate fail, a kf3 build failure, a suite arm
  that fails or crashes, or a census line that is not USER.
- **H-win (GB202, GB205, GB206 if budget):** the GB203 display fixes (type-0 virtual tiles, the per-class
  64 KiB pushbuffer format) carry to the other GB20x dies, so `windows_gate.sh` ends `WINDOWS_GATE_PASS`.
  Falsifier: `WINDOWS_GATE_FAIL` (in particular LiveKernelEvent 0x1A8/0x1B8, or a core channel halted at a
  pushbuffer bound). `WINDOWS_GATE_RETRY` is not a verdict on the guest: re-run on a fresh box once.
- **H-precomp-GB205:** GB205's PRECOMP page equals GB202's committed rows (the inheritance in
  `crates/kf-rm/src/precomp.rs`, branch 595). Falsifier: any of the 48 words or 328 decoded fields measured
  by the read-only kit (`kf-precomp-tier3` at `a6456796e`, own GPU, host 595.91.07) differs from GB202's.
- **H-precomp-GB202 (reference check):** a fresh GB202 read equals the committed GB202 rows
  (`crates/kf-rm/data/precomp_measured.tsv`). Falsifier: any differing word or field.
- **H-precomp-GB206 (if read):** equals the committed GB206 rows. Falsifier: any difference.
- **H-caps-00c:** each die's caps word 0x00c, read with `tools/read_display_caps_page.py` (read-only BAR0
  page, own GPU, root), equals the committed `crates/kf-disp/data/caps_measured.tsv` row for that die and the
  host branch it was read under. Falsifier: a different value, or (for a die/branch with no row) recorded as
  a new measurement, not as a row change in this lane.

## Rules followed

Read-only kits on our own rented GPU only; no other tenant's GPU, page or memory. No secrets on a box.
Leases <= 3 h (destroy timers armed at creation, 10500 s). At most ~3 negative boots per question.
