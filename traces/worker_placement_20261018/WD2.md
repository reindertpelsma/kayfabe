# WD2 ledger placement preparation

STATUS: SOURCE TESTED LOCALLY, NOT DEPLOYED (2026-10-25); owner acceptance pending.

CODE READ: source based on origin/master `a9105acb656ec212daf5da7b8b916387f5da1671`.
The shared `placedLedger` accessor selects `owner-shared-vast-account-enam-v1`
with `locationHint: 'enam'` for gateway forwarding and scheduled cleanup.
The Durable Object binding, class, migration declarations and storage schema are unchanged.

FALSIFIER (stated before runs): either entry point selects the old identity, lacks the
`enam` option, or a refused request resolves/accesses the namespace.

MEASURED: WD2 local run, tests-only commit `44423912c`, placement suite: 6 pass,
4 fail against the old source (EXPECTED RED). The new implementation passes the
73-test targeted run (placement, front authorization, path prefix and round lifecycle).
The same WD2 run passes all 3 bundle tests, including two identical Wrangler emissions,
reviewed hash equality, source/bundle refusal behavior and scheduled cleanup dispatch.

MEASURED: WD2 full Worker run after commit `44423912c`, `npm test`: 1823 passed,
0 failed, 0 skipped (including the 10 placement tests and the deterministic bundle test).

## Behavioral mutation results

MEASURED: WD2 local mutation run after commit `44423912c`, using only
`ledger-placement.test.js`; each mutant was restored before testing the next one.
Every valid behavioral mutant exits nonzero and fails at least two distinct tests.

| Mutant | Distinct failing tests |
| --- | ---: |
| shared-old-identity | 4 |
| shared-missing-hint | 4 |
| shared-wrong-hint | 4 |
| http-old-ledger | 2 |
| http-no-hint | 2 |
| cron-old-ledger | 4 |
| cron-no-hint | 4 |
| early-before-auth | 4 |
| early-before-routing | 8 |

The two authenticated routes are `/status/123-1` and `/logs/123-1`; both exercise
real gateway JWT verification, forwarding and scheduled cleanup with fake namespace
bindings. Two standalone cron cases check one and two invocations. Refusal cases
check missing/garbage credentials, public cron/unknown routes and two wrong prefixes;
namespace name resolution and get calls must both remain absent.

Mutation harness correction: the first missing-hint attempt left a stray comma and
failed parsing; that is excluded. A cron mutation initially matched no source after
formatting; it is also excluded. The table contains the corrected behavioral runs.

## Bundle and operating limits

CODE READ: previous deployment.json and wrangler.toml hash:
`a71415063f1d4c4c323b07b8ccd4d2fdb9a84f5a3c653278b1888fe7a8085fa7`.
MEASURED: WD2 bundle run after commit `44423912c` used Wrangler 4.152.0
with `deploy --dry-run --outdir` and emitted index.js:
`05ca76f311350da85a1e90235390d37c3dd64eedac9fff4668a0afa10734f541`.
Both hash files were updated once to that final digest. Dependencies were installed
with `npm ci --ignore-scripts` from the unchanged lockfile; formatting used Prettier 3.6.2.

CODE READ: RENTING_ENABLED remains false. No production GPU source, workflows,
Durable Object schema or dependency files changed.

INFERRED: requesting initial placement does not establish the live object's actual
location or latency. No Worker/Cloudflare request, deployment, secret access, rental,
old-ledger query, reset or migration was performed. The owner/controller must review
the old ledger for active state and spend before any deployment and explicitly accept
the new identity; this task does not establish that the old ledger is empty.
