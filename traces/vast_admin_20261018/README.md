# UI1 owner admin evidence

STATUS: LIVE local test evidence, 2026-10-26; no deployment or live Vast acceptance.

## Current integration: master 3cb9469aa (PRs #144 and #145)

CODE READ (merge comparisons, `merge2-checks.txt`): `index.js` is unchanged from the prior UI1 head, so the admin routes remain isolated in `BrokerLedger.fetch`. Ledger, sanitizer and runner exactly match master. Only the manifest/TOML hash fields conflicted; no implementation adjustment was needed.

MEASURED (2026-10-26, [merge2-checks.txt](merge2-checks.txt)): the bundle test first reported the expected mismatch against master's base hash and supplied the combined module hash. After updating both pins, all 2,042 broker Node tests, 37 workerd tests with `MINIFLARE_ASSERT_BODIES_CONSUMED=true`, and 50 admin Worker tests pass with no failures/skips. All 24 admin mutants still fail at least two tests each, and the restored source passes all 83 focused tests. Formatting, the UI1 100-column audit, whitespace and credential/private-path checks pass. The credential audit reviewed two synthetic scrub vectors inherited from master without printing their values. The current reviewed hash supersedes both older integration hashes below:

```text
700463eb10f57c3bfc88f4c2d49101649bc53bee85bbeeae0016142660468062
```

## Previous integration: master 737402577 (PR #141)

MEASURED (2026-10-26 merge validation, [merge-checks.txt](merge-checks.txt)): merging master into UI1 preserves the admin class byte-for-byte and keeps the ledger/bootstrap exactly as master. Only the two bundle-hash fields conflicted; both now name the combined dry-run bundle. No history is rewritten and no deployment is performed.

MEASURED (2026-10-26, `merge-checks.txt`): all 1,968 broker Node tests, 37 workerd tests (with `MINIFLARE_ASSERT_BODIES_CONSUMED=true`) and 50 admin Worker tests pass, with zero failures/skips. The 24 admin mutants still fail at least two tests each; the restored source then passes all 83 focused tests. Formatting also passes. A full PR line audit corrected one original 102-column test declaration missed at the initial checkpoint; assertions are unchanged. The reviewed combined bundle hash supersedes the initial UI1 hash below:

```text
edc934d70573d676ccb5123c13ac0fc4c0b0fe3f9c37d158ea717c8a6476642d
```

## Initial UI1 checkpoint (be85b084b)

CODE READ (`ci/vast-admin/worker.js`, broker `BrokerLedger.fetch`): UI1 adds the owner-only Access-authenticated Worker, its external DO binding and plain table, internal list/stop routes, and coordinator-only Access/deployment instructions. The broker source diff stays in `BrokerLedger.fetch` plus two constructor assignments retaining its context/environment. The existing service, owner stop handlers and provider cleanup implementation are reused without changes.

FALSIFIER, stated before implementation in the tests: a public admin request reaches the ledger; a summary leaks non-whitelisted fields; an internal stop bypasses owner cleanup or settles before API absence; a missing/invalid Access identity reads or stops; a wrong email enters; or a proxy issues more than one DO call. Mutation falsifier: any deliberately broken boundary fails fewer than two independent tests.

MEASURED (UI1 EXPECTED RED commit `41b9a37c2`, 2026-10-26): the initial two test files ran 62 tests, with 59 failures and three passes against the original code. This commit contains tests only. Two fixture assumptions were then corrected: unprefixed routing with renting enabled produces a configuration refusal, and the existing deletion verifier inspects `/api/v0/instances/<id>/`, not the account listing endpoint. The corrected public test disables renting; the cleanup test asserts the existing instance inspection.

MEASURED (UI1 corrected-baseline run, 2026-10-26, `checks.txt`): restoring the pre-implementation broker source and temporarily removing the admin entry/config ran 82 focused tests with 77 failures and five passes. The complete source/config were restored in `finally`. The extra truncation boundary case was added afterward.

MEASURED (UI1 final focused run, 2026-10-26, `checks.txt`): 83 focused tests pass, zero fail. Coverage includes strict origin/public routing, 100-row/newest-first summaries, a 1,000-record scan refusal, scalar filtering, per-run and all-run cleanup, deletion acknowledgement without actual absence, malformed selections, missing configuration, signed Access claims and signature failures, exact owner email, proxy counts, DOM text insertion, active-only Stop, and Stop-all confirmation.

MEASURED (UI1 final full broker run, 2026-10-26, `checks.txt`): all 1,943 tests pass with zero failures and zero skips, including the deterministic bundle checks. Prettier 3.6.2, the changed-code 100-column scan, secret/private-path scan, whitespace check and claims ratchet (`new=0`) also pass.

MEASURED (UI1 mutation run, 2026-10-26, [mutations.md](mutations.md), [mutation-output.txt](mutation-output.txt)): all 24 mutants fail between two and 13 independent tests each. Each source file is restored before the next probe and in an outer `finally`; the final green focused suite runs after restoration. The table records two named witnesses per mutant. Reproduce locally with `python3 traces/vast_admin_20261018/mutate.py` after installing the two Workers' test dependencies; it uses only fixture providers.

MEASURED (UI1 Wrangler dry runs, 2026-10-26, `checks.txt`): both Worker configurations bundle locally. The initial main bundle hash was pinned at `be85b084b` and is superseded by the integration update above:

```text
8a0c7358318434f48a9a60255c1ecd47bac2af29f5725c8586cf841d3883f51b
```

CODE READ (`ci/vast-broker/test/bundle.test.js`): the full broker suite emits two independent bundles, checks deterministic bytes against the manifest, rejects Node imports, and exercises the source/bundled entrypoints. The admin dry-run bundle hash is recorded for evidence only; the admin has no deployment-manifest protocol.

NOT VERIFIED: Cloudflare deployment, the external namespace binding in a hosted account, the Access application/login flow, actual provider stop, or GPU hardware. No direct Vast or Claude calls were made. The coordinator must configure the dedicated Access application and deploy both Workers in the documented order; WK1 integration is recorded above; recompute the broker hash after any further source change.
