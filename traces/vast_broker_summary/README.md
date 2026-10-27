# Vast broker / Worker: evidence summary (slim branch)

STATUS: LIVE, 2026-10-23. This file replaces eight raw run-evidence trace trees that the
Vast broker PR chain (PR #68) introduced under `traces/vast_broker_*`. They were removed from
this branch only because of size (160 files, 782,502 bytes of run output, logs and mutant
tables). Nothing was changed in `ci/`, `scripts/ci/`, `.github/` or the design docs except
the pointers listed in the slimming report.

## Where the raw traces are

[CODE READ: git] The pre-slim head is `c8287a55e811f0c4c2497528c1ef0af13ea9f40e` (branch
`claude/vast-broker-jose-20261023`, kept unchanged as the private evidence branch). Every file
named below exists at that commit, e.g.
`git show c8287a55e811f0c4c2497528c1ef0af13ea9f40e:traces/vast_broker_v5_20261018/LIVE.md`. The commits of this branch carry
the same messages and the same non-trace content; their hashes differ from the pre-slim ones
for the 227 commits after the master merge base `e64b8e5f6`. Commit messages and docs cite the pre-slim
hashes of the red and green commits, and those hashes resolve in the evidence branch.

## What each removed set holds

| Set (files, bytes) | What it holds |
| --- | --- |
| `vast_broker_20261018` (1, 10,960) | V1 prototype report, 2026-10-22: tests-first red/green table for endpoint-secret scope and prefix routing; Node suite 473/473 at 833a1a865. Mocks only, no rentals. |
| `vast_broker_smoke_20261018` (33, 91,936) | V3 local Worker smoke on native workerd/SQLite with a read-only real Vast key, 2026-10-22: baseline 376/376 and final 514/514 Node tests; one real lease created and destroyed; the credit-field defect and five other defects found and fixed; callbacks blocked by a startup failure at that time. |
| `vast_broker_v4_20261018` (16, 85,721) | V4 native run `v4-native-20261022`: 626/626 tests, 32 V4 plus 96 legacy mutants each with at least two named killers, mock PASS/FAIL/cron protocol runs, local workerd CPU samples (not Cloudflare billed CPU), real read-only preflight with zero rentals. |
| `vast_broker_v4_round2_20261018` (14, 56,403) | Two real Docker/SSH runs (PASS then kill, FAIL then cron) through local Wrangler with fake GitHub/JWKS; six rentals in total, about $0.057 estimated; tests 626 to 638; API absence verified after each. |
| `vast_broker_v4_round3_20261018` (7, 21,376) | Runner-never-trusts-Worker boundary: 724/724 tests, 38 Round 3 mutants with at least two killers each, synthetic controller log; gaps listed (hosted OIDC, billed CPU, login-to-MANUAL). |
| `vast_broker_fixes_20261018` (14, 112,050) | Security-fixes round 2026-10-23: pre-change 1113/1113, EXPECTED RED commits and mutation tables. Partly superseded by fixes2 (owner rulings 19:10 and 19:25). |
| `vast_broker_v5_20261018` (60, 296,991) | V5 deployable checkpoint: 1113/1113 portable tests, 279 Worker mutants, dummy-lifecycle live runs (six real rentals, all verified absent, $0.268 total), region/price/read-only API sweeps, PR #104 alignment (72 Python tests, 60 mutants). Its `final_verification.json` is kept here. |
| `vast_broker_fixes2_20261023` (15, 107,065) | fixes2 verification: 1369 tests, 358 plus 52 box mutants, 104 scripts/ci unittests; candidate bundle 5ae53f61 (110,401 bytes), `claims_new_debt: 0`. Superseded by the jose round (bundle 145,487 bytes, c44caad8, node 1406, scripts/ci 109), whose numbers live in the design docs and the commit messages. |

## Kept: V5 final verification

[MEASURED: 2026-10-23, run v5-final-verification-20261023] `final_verification.json` in this
directory (2,772 bytes) is the V5 checkpoint record: 1113 portable tests, 279 Worker mutants
over eleven campaigns, canonical box scripts 72 tests and 60 mutants, Wrangler 4.152.0, no
upload, renting disabled in the committed configuration. It describes the V5 bundle, not
the jose bundle that this branch ships.

## Limits

Every figure above is copied from the removed files' own statements. Nothing was re-run for this
summary on 2026-10-23. Anything that matters for a decision should be re-read in the evidence branch.
