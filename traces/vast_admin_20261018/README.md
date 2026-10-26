# UI1 owner admin evidence

STATUS: LIVE local test evidence, 2026-10-26; no deployment or live Vast acceptance.

CODE READ (`ci/vast-admin/worker.js`, broker `BrokerLedger.fetch`): UI1 adds the owner-only Access-authenticated Worker, its external DO binding and plain table, internal list/stop routes, and coordinator-only Access/deployment instructions. The broker source diff stays in `BrokerLedger.fetch` plus two constructor assignments retaining its context/environment. The existing service, owner stop handlers and provider cleanup implementation are reused without changes.

FALSIFIER, stated before implementation in the tests: a public admin request reaches the ledger; a summary leaks non-whitelisted fields; an internal stop bypasses owner cleanup or settles before API absence; a missing/invalid Access identity reads or stops; a wrong email enters; or a proxy issues more than one DO call. Mutation falsifier: any deliberately broken boundary fails fewer than two independent tests.

MEASURED (UI1 EXPECTED RED commit `41b9a37c2`, 2026-10-26): the initial two test files ran 62 tests, with 59 failures and three passes against the original code. This commit contains tests only. Two fixture assumptions were then corrected: unprefixed routing with renting enabled produces a configuration refusal, and the existing deletion verifier inspects `/api/v0/instances/<id>/`, not the account listing endpoint. The corrected public test disables renting; the cleanup test asserts the existing instance inspection.

MEASURED (UI1 corrected-baseline run, 2026-10-26, `checks.txt`): restoring the pre-implementation broker source and temporarily removing the admin entry/config ran 82 focused tests with 77 failures and five passes. The complete source/config were restored in `finally`. The extra truncation boundary case was added afterward.

MEASURED (UI1 final focused run, 2026-10-26, `checks.txt`): 83 focused tests pass, zero fail. Coverage includes strict origin/public routing, 100-row/newest-first summaries, a 1,000-record scan refusal, scalar filtering, per-run and all-run cleanup, deletion acknowledgement without actual absence, malformed selections, missing configuration, signed Access claims and signature failures, exact owner email, proxy counts, DOM text insertion, active-only Stop, and Stop-all confirmation.

MEASURED (UI1 final full broker run, 2026-10-26, `checks.txt`): all 1,943 tests pass with zero failures and zero skips, including the deterministic bundle checks. Prettier 3.6.2, the changed-code 100-column scan, secret/private-path scan, whitespace check and claims ratchet (`new=0`) also pass.

MEASURED (UI1 mutation run, 2026-10-26, [mutations.md](mutations.md), [mutation-output.txt](mutation-output.txt)): all 24 mutants fail between two and 13 independent tests each. Each source file is restored before the next probe and in an outer `finally`; the final green focused suite runs after restoration. The table records two named witnesses per mutant. Reproduce locally with `python3 traces/vast_admin_20261018/mutate.py` after installing the two Workers' test dependencies; it uses only fixture providers.

MEASURED (UI1 Wrangler dry runs, 2026-10-26, `checks.txt`): both Worker configurations bundle locally. The main bundle hash is pinned in `ci/vast-broker/deployment.json` and `wrangler.toml`:

```text
8a0c7358318434f48a9a60255c1ecd47bac2af29f5725c8586cf841d3883f51b
```

CODE READ (`ci/vast-broker/test/bundle.test.js`): the full broker suite emits two independent bundles, checks deterministic bytes against the manifest, rejects Node imports, and exercises the source/bundled entrypoints. The admin dry-run bundle hash is recorded for evidence only; the admin has no deployment-manifest protocol.

NOT VERIFIED: Cloudflare deployment, the external namespace binding in a hosted account, the Access application/login flow, actual provider stop, or GPU hardware. No direct Vast or Claude calls were made. The coordinator must configure the dedicated Access application and deploy both Workers in the documented order; rebase/recompute the broker hash after integrating WK1's source changes.
