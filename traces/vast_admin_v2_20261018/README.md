# UI1 v2: Stop fixtures and retained log viewer

STATUS: LIVE local source/test evidence, 2026-10-26; coordinator deployment pending.

CODE READ (`ci/vast-admin/worker.js`, broker `BrokerLedger.fetch`, `Ledger.logPage`): every row links to its Access-authenticated escaped log page. The DO-only route accepts a validated run/cursor and emits exactly sanitised text, optional summary text and numeric cursors. It reuses the relay pager with one chunk per page; the runner retains its four-chunk default. A page is at most 16 KiB of log text plus a complete final-summary block from the existing 8 KiB tail. Expired records show `log expired`; failures use fixed messages. Nothing is newly persisted. The public broker never routes `/admin/log`.

FALSIFIER, recorded in the tests before implementation: public or wrong-owner callers reach logs; capability fields leak; a page exceeds its byte bound; cursor traversal skips/duplicates chunks; pruned records appear available; log markup becomes HTML; or open records without instance IDs cannot be stopped.

MEASURED (`checks.txt`, EXPECTED RED commit `a94810903`): the tests-only checkpoint ran 47 new Node tests with 34 failures/13 passes and six real workerd tests with four failures/two passes. Five active/open Stop fixtures already passed: the original Stop condition correctly accepts CREATING through cleanup-pending STOPPED without an instance ID. The implementation preserves it. Tests with ended records and retained historical instance IDs verify no Stop is offered. A log-proxy fixture was corrected to return an advancing cursor, and two invalid-run cases were added afterward.

MEASURED (`checks.txt`, 2026-10-26): all 2,068 full broker tests, 43 workerd tests and 77 admin Worker tests pass with zero skips/failures. The 132 combined focused tests pass after restoring every mutation. Workerd runs with `MINIFLARE_ASSERT_BODIES_CONSUMED=true` and outbound traffic rejected/intercepted; CI now runs all `workerd/*.test.js` files. Master `6a1c43269` (deployment allowlist and tests, no Worker-source change) was integrated before the final full run.

MEASURED (`mutations.md`, `mutation-output.txt`, `mutate.py`, 2026-10-26): all 20 v2 mutants are killed by at least two independent tests each. This includes origin/public routing, scalar whitelists, byte/page bounds, summary extraction, expiry, cursor traversal, HTML escaping, Next binding, row links, active Stop without ID, terminal Stop suppression and owner checks.

MEASURED (2026-10-26 v2 bundle run, `checks.txt`): both bundle pins now name the v2 emitted module; the full suite emits two matching bundles and checks Node-free imports:

```text
9ac12a86ed2dbc4bc7a63c9a35d739c9dcc9acf7e18eb117187f6e92082ecd09
```

INFERRED ([retention analysis](retention.md), source and local SQLite model): keep `LOG_RETENTION_HOURS=6` at default admission caps, with healthy pruning. The two-run model, raw [storage results](storage.jsonl) and reproducible [probe](storage.mjs) quantify payload plus hash/KV/page overhead. No production database or plan was queried; the model is not a strict hosted ceiling. Higher admission caps and stalled pruning need separate capacity checks.

NOT VERIFIED: hosted Cloudflare Access/login/DO binding, actual browser acceptance or live Vast behavior. No deployment, Cloudflare setting/secret change, direct Vast call or Claude call was performed. Review and deployment remain with the coordinator.
