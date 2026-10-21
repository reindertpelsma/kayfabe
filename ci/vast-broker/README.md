# Vast broker (stage 1)

[CODE READ: 2026-10-21 provider documentation] **BLOCKED: keep renting disabled.** The strict no-SSH instruction conflicts with the documented VM/onstart launch modes. See the design's STOP section; the prototype is not ready for deployment.

[CODE READ] Plain ES-module JavaScript, zero npm dependencies, Node >=20. This is an **untrusted GPU tripwire**, never release provenance. The Worker alone holds the owner's dedicated Vast key. No deployment or rental is performed by tests. Binding defaults refuse spending until configured. The complete threat model, API contracts, billing risks and deferred merge queue are in [the design](../../docs/design/V3_CI_VAST_GPU_20261021.md).

[CODE READ] Automatic PR events use default `owner_push_or_review`: numeric GitHub event actor/triggering-actor identity is the owner, or an owner review approves exactly that PR head SHA. Commit author text has no authority. An owner push also vouches for earlier commits on that branch. Drafts refuse by default. Push storms do not queue rentals: superseded work is destroyed and the newer request returns BUSY. `review_or_author`, `review_only`, `owner_push` and `actor_only` are explicit alternatives. Automatic PRs require an API-checked master base (ALLOWED_BASE_BRANCHES) and a non-draft head. The Worker excludes only a complete all-skipped PR file list using PATH_SKIP_PATTERNS; unknown/truncated lists run, and API errors fail. ci/vast-broker always runs. NOT_NEEDED is an explicit successful job/status. Owner dispatch can run any branch, with an optional PR cross-check; rerun that owner's dispatch after the minimum gap to force a fresh rental. Ordinary same-SHA requests return the cache.

[MEASURED: 2026-10-21 local Node test/bundle commands] Run `npm test` (or `node --test ci/vast-broker/test` from the repo root), `npm run bundle`, and `npm run mutants` from this directory. No npm install is needed. The fixed-source linker writes ignored `dist/worker.js`, checks syntax and preserves the entrypoint exports. Tests use generated RSA keys, fake JWKS, mocked GitHub/Vast, an in-memory Durable Object adapter and a local HTTP server.

## Deploy with the owner's credentials (9 steps)

1. Review this branch and its evidence. Choose the numeric GitHub user ID and keep default `owner_push_or_review`; confirm dedicated-key permissions with Vast. Set `RENTING_ENABLED=false` while configuring. Never put a Cloudflare token or Vast key in GitHub.
2. Choose the upload route. **CLI:** from this directory run `npx wrangler login`, fill `wrangler.toml` public vars, then `npx wrangler deploy` (legacy SQLite migration included). **No CLI:** run the bundle via the provided Node script, upload the module and initial metadata with a graphical HTTP client using the Cloudflare script-upload API described below; thereafter paste `dist/worker.js` into the dashboard module editor for updates. Pure dashboard paste alone does not provision a new Durable Object namespace.
3. Ensure a SQLite `BrokerLedger` namespace is bound as `LEDGER` to this same Worker, and schedule `*/5 * * * *`. Preserve the namespace when updating; never clear it to bypass a blocked reservation.
4. Set public variables matching `wrangler.toml`, replacing `OWNER_GITHUB_ID=REPLACE_ME`, `REPOSITORY` and the exact HTTPS `WORKER_URL` audience. Keep budgets at 3 USD/day, 30 USD/rolling 30 days, 8 USD reserve, max 0.50 USD/hour, TTL 150 min, max 4 rentals/day and 24h minimum same-SHA gap unless deliberately reviewed. `RENT_DRAFTS=false` is recommended.
5. Set secret binding `VAST_API_KEY` through the dashboard or `npx wrangler secret put VAST_API_KEY`. No stored GitHub secret is needed: the broker job passes its ephemeral read-only GITHUB_TOKEN in X-GH-Token on each request. It has contents:read and pull-requests:read (plus OIDC minting permission); separate status-only jobs never send their tokens to the Worker. The Worker makes fixed-host GETs only, never stores/logs the token, and falls back to public reads on 401/403. Cron uses public reads; shared egress rate limits can cause UNAVAILABLE, while TTL deletion precedes those reads.
6. In GitHub, set repository variable `VAST_BROKER_URL` to that exact audience URL and create environment `gpu-vast`. Protect allowed branches and workflow/broker changes. Required environment reviewers add a manual delay; the owner's 2b ruling uses the authenticated owner push as approval, so avoid a redundant reviewer requirement if immediate starts are intended. Forks never rent.
7. Authorize and measure one live smoke run separately, including the KVM image's keyless onstart, Actions/OIDC PR-head contract, asynchronous S3 log path, CPU use and final-marker cleanup. Mocks cannot establish these live contracts. Confirm current Cloudflare Free/SQLite limits and Vast billing/overdraft/top-up rules first. No live smoke run is authorized by this lane.
8. Only after that validation set `RENTING_ENABLED=true`. Make commit-status context **gpu-vast** required on master. It posts on the exact PR head and fails on NEEDS_APPROVAL, NOT_ELIGIBLE, BUSY, UNAVAILABLE or UNAVAILABLE_BUDGET, and on skipped/cancelled dependencies. A lying box can fake PASS, so no release/attestation consumes this workflow or its data. Record any administrator bypass reason.
9. For the kill switch set `RENTING_ENABLED=false`; cron/status cleanup stays active. If the key is compromised, revoke it in Vast and reconcile/destroy the broker's known IDs manually (a revoked key cannot delete). Rotate by updating the Worker binding then revoking the old key, preserving the ledger.

## No-CLI namespace bootstrap

[CODE READ] Cloudflare documents that class lifecycle changes require a deployment, not merely pasting a class export: [Durable Object lifecycle](https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/). Its [script-upload API](https://developers.cloudflare.com/api/resources/workers/subresources/scripts/methods/update/) accepts multipart module code plus migration/binding metadata. A GUI HTTP client is the no-CLI bootstrap; its authentication remains on the owner's machine. This route is documented, not live-tested here.

Use `PUT https://api.cloudflare.com/client/v4/accounts/{account_id}/workers/scripts/kayfabe-vast-broker`, with the owner's Authorization header. Add multipart field `worker.js` as the generated file with content type `application/javascript+module`, and field `metadata` as JSON:

```json
{
  "main_module": "worker.js",
  "compatibility_date": "2026-10-21",
  "migrations": {"new_tag": "v1", "steps": [{"new_sqlite_classes": ["BrokerLedger"]}]},
  "bindings": [{"type": "durable_object_namespace", "name": "LEDGER", "class_name": "BrokerLedger"}]
}
```

Configure variables, secrets, the public route and cron in the dashboard after bootstrap. Subsequent updates must preserve the namespace/bindings; do not replay the initial migration or replace all bindings accidentally. Export `BrokerLedger` as well as the default handlers when pasting the bundle. Do not substitute KV: eventual consistency cannot enforce the single-writer budget and idempotence rules.

## Cost and operational limits

[INFERRED from source: src/policy.js, arithmetic estimate] A 150-minute allowance at 0.50 USD/hour reserves 1.25 USD of estimated compute. Cold execution is estimated at 60–120 minutes (0.50–1 USD). Cron/API outages can leave a box billing 12 USD/day or 360 USD/30 days at that rate, plus other fees; there is no proven finite maximum with automatic account funding. Keep modest prepaid credit and disable automatic top-up only after confirming Vast debt rules. Stopped instances still bill storage; an on-box timeout bounds honest work, **not** the upstream bill. Final-marker/TTL destroy and ledger accounting are the actual cleanup paths.

[CODE READ] Log retrieval is an asynchronous Vast request then a separate S3 read. It may be delayed or truncated; the Worker accepts only the fixed raw nonce marker and rejects oversized/invalid transfers. It sanitizes and caps display text; the runner wraps it again with stop-commands. The optional tokenized result-POST fallback is not built. Ambiguous creates are reconciled only against their persisted exact intent; unknown/duplicate/mismatched candidates block further spending. The owner may need to reconcile manually. Merge-group membership and queue-time approvals remain deferred; organization transfer is an owner decision.
