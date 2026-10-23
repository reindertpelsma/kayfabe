# Owner deployment and independent cleanup

[CODE READ: wrangler.toml, deployment.json, src/index.js] The owner/controller deploys; this round performs no deployment or provider calls. Keep `RENTING_ENABLED=false` until acceptance. `main=dist/worker.js` and `no_bundle=true` upload the reviewed single-file module. No Cloudflare token belongs in GitHub.

[CODE READ: package.json; README.md deployment tooling] Install Node 22+ and the pinned CLI outside the checkout: `npm install --prefix /tmp/kf-vast-wrangler wrangler@4.152.0`. This pins the CLI version, not the transitive dependency integrity; retain the owner's verified installation/lockfile for reproducible deployment tooling.
In `ci/vast-broker`, run `node --test test` and `node bundle.js`. Check `sha256sum dist/worker.js` against deployment.json, dist/deployment.json and wrangler.toml. This is a candidate bundle, not proof of what is deployed. Preserve the SQLite LEDGER namespace and migration when updating.
Every main deployment MUST override the template's OWNER_GITHUB_ID=REPLACE_ME, including dry runs:

```sh
/tmp/kf-vast-wrangler/node_modules/.bin/wrangler deploy --dry-run \
  --var OWNER_GITHUB_ID:229738317
/tmp/kf-vast-wrangler/node_modules/.bin/wrangler deploy \
  --var OWNER_GITHUB_ID:229738317
```

[CODE READ: wrangler.toml, defaults.json, src/policy.js] Retain REPOSITORY_GITHUB_ID=1311106387, REPOSITORY=reindertpelsma/kayfabe, APPROVAL_MODE=owner_push_or_review. Retain the exact OIDC_SUB, CHECK_JOB_WORKFLOW_REFS and SWEEP_JOB_WORKFLOW_REF templates. Supported events are pull_request_target, owner workflow_dispatch and owner push; pull_request, workflow_run and merge_group refuse. The exact master workflow pin remains available to ordinary callers. An additional own-repository gpu-vast.yml refs/heads/* workflow is permitted only when the signed actor and API actor/triggering_actor are all the owner, with matching own-repository run/head/ref/attempt. This exception does not extend to tags, PR refs, another workflow or forks. OIDC requires integer iat, maximum token age 600 seconds, exp-iat at most 3600 seconds, valid issuer/audience/signature and bound repository/job/run identity. Hosted claim issuance remains an acceptance gap.

[CODE READ: README.md; gpu-vast.yml, trigger-policy.mjs, src/github.js] Trigger policy lives in the workflow. Automatic pushes target master; target PRs target master and drafts do not rent. The trusted BASE policy job skips only complete, unchanged-head, all-excluded paths; unknown, malformed, renamed-out-of-scope, truncated or failed metadata runs. Manual dispatch always runs and accepts an optional 40-hex sha reachable from a branch of this repository. Reachability checks at most five branch pages and sixteen distinct head comparisons, refusing if not proved within those bounds. The isolated controller still loads at the workflow BASE SHA. Worker admission has no branch, draft or path trigger filters; authorization and spending checks remain independent. The final status explicitly succeeds for a verified workflow exclusion and fails unexplained skipped/failed jobs. Review the YAML commit separately before adoption.

[CODE READ: defaults.json, src/policy.js, src/offers.js, src/index.js] Check policy remains $1/hour, $6 daily, $30 rolling, $8 reserve, 40-minute boot and 180-minute hard limit. MAX_INET_USD_PER_GB defaults to 0.005 in each direction; the offer query, admission and contract recheck enforce it. DENIED_COUNTRIES defaults to CU, IR, KP, SY before country preference. ALLOW_UNKNOWN_COUNTRY defaults to true; set false to reject unknown metadata. PREFERRED_COUNTRIES ranks only remaining eligible offers. Contract price failure revokes the run and attempts deletion; failed deletion retains the liability for cleanup retry.

[INFERRED: src/ledger.js transfer-allowance accounting] Per-GB limits reduce exposure but do not make reserved 15-GB-down/2-GB-up allowances a hard bill ceiling. A hostile box can exceed the allowance. Keep the owner's main Vast account balance small, monitor it and retain an independent cleanup backstop; price/credit/spend reservations cannot prevent provider overbilling or an outage.

## Worker and GitHub secrets

[CODE READ: src/tokens.js, src/routes.js] Set Worker secrets with the owner's pinned CLI: WORKER_URL (exact HTTPS origin), VAST_API_KEY, RUN_TOKEN_SECRET, OWNER_STOP_TOKEN and BROKER_PATH_PREFIX. Prefix is required when renting is enabled. Generate independent values without printing them into logs:

```sh
head -c 32 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=\n'
```

[CODE READ: src/tokens.js, src/routes.js, src/index.js] The alphabet is `[A-Za-z0-9_-]`; signing/owner secrets accept 32–256 characters and prefix accepts 32–128. Raw padded base64 containing `+`, `/` or `=` refuses. An owner-authenticated availability request reports CONFIG_SECRET_FORMAT:<name> without the value; other callers receive generic refusal. Keep signing and owner-stop values independent. URL/prefix secrecy reduces scanning and DoS noise; OIDC, approval policy, live-ledger checks and per-run HMAC are authentication. A Vast operator can read its KF_* env and callback URL; the box holds only its own limited capability.

[CODE READ: src/github.js, src/blobs.js, src/index.js] Optional Worker secret GITHUB_API_TOKEN is the default for GitHub checks and private release metadata. Use one fine-grained read-only token selected only for reindertpelsma/kayfabe and the private store repository/repositories: Contents read, Pull requests read, Actions read and Metadata read. No write/admin permission is needed. Authenticated read failures 401/403 retry public metadata without the token. Lifecycle run/PR checks cache for 60 seconds; admission approval/head checks stay fresh. Workflow file-policy reads are fresh and use the separate job-local read token. Client X-GH-Token is unsupported and ignored. BLOB_GITHUB_TOKEN is an asset-only deprecated fallback when GITHUB_API_TOKEN is unset; the primary wins when both exist. Neither token enters box env, storage or public responses/logs. Do not put a Worker GitHub token in GitHub Actions.

[CODE READ: src/blobs.js] For blob access, set BLOB_REPO=owner/store and a NONEMPTY BLOB_ALLOWED_TAG_PREFIXES JSON array such as ["ci-image-"]. Missing/empty prefixes deny all mints. The Worker reads metadata and returns only an approved short-lived release-asset redirect URL, size and asset name; it never downloads asset bytes or returns a GitHub Authorization header. Credential-bearing redirect locators refuse. Run/day quotas are checked before provider inspection. Store repositories and allowed tag namespaces should contain only bytes a rented box is permitted to read.

[CODE READ: gpu-vast.yml, gpu-vast-sweep.yml, workflow-guard.js] GitHub environment gpu-vast contains VAST_BROKER_URL and VAST_BROKER_PATH secrets. Optional runner Access token A uses VAST_BROKER_ACCESS_ID and VAST_BROKER_ACCESS_SECRET, bound only in the sole trusted BASE broker step. The isolated controller receives them as CF_ACCESS_CLIENT_ID/CF_ACCESS_CLIENT_SECRET. Token A reaches only broker HTTPS calls, never the OIDC issuer or box. No credential or endpoint is printed outside GitHub masking commands; fixed enum outputs/status descriptions cannot carry them.

## Optional Cloudflare Access

[CODE READ: src/access.js, src/routes.js, src/oidc.js] Configure ONE hostname-wide workers.dev Access application and ONE service-auth policy allowing BOTH service tokens A (runner) and B (box). The old path-split proposal is superseded. Set these four public Worker vars together: ACCESS_TEAM_DOMAIN (the owner-provided public team olifon.cloudflareaccess.com), ACCESS_AUD (application audience), ACCESS_RUNNER_CLIENT_ID and ACCESS_BOX_CLIENT_ID. Values must identify distinct A/B clients and retain the full `.access` suffix in common_name. Real client IDs are supplied only at deployment. All absent disables this optional layer; partial/malformed configuration refuses by variable name. Configure Access to include the signed Cf-Access-Jwt-Assertion for service-token requests and confirm common_name equals the expected client ID during controller acceptance. The Worker verifies RS256, issuer, application audience, exp and integer iat before ledger work; it never reads the Client-Secret half to assign a role. Access assertions accept a matching scalar audience or a nonempty array of at most 16 bounded strings containing the application audience; GitHub OIDC stays scalar-only. Access assertions have at most a 24-hour issuance-to-expiry window. JWKS caches are separate from GitHub, with single-flight refresh and 60-second unknown-key cooldown.

| Access identity | Worker routes (all beneath the configured prefix) |
| --- | --- |
| Runner A | POST rent, GET availability, GET status/<run>, GET logs/<run>, POST stop/<run>, POST stop-all, POST stop-timer/<run> |
| Box B | POST report/<run>, POST hold/<run>, POST kill/<run>, GET blob-url/<run>/<tag>/<name> |

[CODE READ: src/routes.js, src/index.js] Heartbeats use report with a heartbeat kind; there is no separate heartbeat route. Access permits a route only; existing OIDC/run/owner authentication still applies. In particular A cannot use stop-all or stop-timer without the separate owner bearer, and B cannot read logs. Missing, forged, expired, wrong-audience and unknown-role assertions return 403 before the ledger. Only the fixed team certs fetch can occur first.

[CODE READ: src/index.js, scripts/ci/vast_report.py, scripts/ci/kf-access-headers.sh] Set Worker BOX_ACCESS_ID/BOX_ACCESS_SECRET as the complete B pair; the Worker injects KF_ACCESS_ID/KF_ACCESS_SECRET into its own create env. The reporter and kill/hold/blob helpers send CF-Access-Client-Id/CF-Access-Client-Secret only when configured, over fixed broker HTTPS requests without redirects. Curl receives headers over stdin rather than argv. Partial/malformed pairs refuse before rental. Secrets are excluded from the payload's inherited env, but guest root can read the root-only env file and process environment: this is not a boundary against a hostile host.

[INFERRED: Access operational policy and src/access.js single-role IDs] Choose service-token expiry to cover all active rentals plus cleanup margin. Prepare replacements with an overlap in the Access policy; the Worker deliberately accepts one ID per role, so rotate the matching public role ID and client headers together while the ledger is empty. The Access policy overlap alone does not make both IDs valid in Worker routing. Keep the old policy/token available for rollback until validation, then revoke. Emergency rotation during a rental can deny old callbacks and lead to cleanup. Do not claim seamless rotation of already-running boxes.

[INFERRED: Worker routing and deployment configuration] Protect every reachable hostname, including workers.dev and preview URLs. If using only a custom domain, disable workers.dev/previews and verify there is no unprotected alternate route. The Worker's assertion check also runs on alternate hosts when configured, but an Access app covering only one hostname is not a substitute for auditing those routes. Access reduces request noise; account-wide request quotas and provider outages remain operational risks.

## Vast key scope

[CODE READ: controller-provided scoped permission JSON, src/vast.js] The reviewed console-front-end permission names are:

```json
{
  "misc": ["api.bundles"],
  "user_read": ["api.users.current"],
  "instance_read": ["api.instance", "api.instance.request_logs", "api.instances"],
  "instance_write": ["api.ask", "api.instance"]
}
```

[CODE READ: src/vast.js API routes and controller permission-name evidence] This is the permission-name projection, not a credential file or a claim about the console envelope's serialization. Use the controller's scoped key JSON in the Vast console. The console uses api.instance; the documentation spelling api.instance.show is invalid there. The main key may read offers/credit/instances and create/delete contracts. No subaccount is assumed; the Worker is the capability boundary. Reconciliation needs an exact label plus fixed onstart hash/image. Create-returned IDs stay owned even when their label changes; a fixed OWNERSHIP_MISMATCH event is logged, and deletion still requires a confirming absence before settlement.

## Independent reaper

[CODE READ: ../vast-reaper/worker.js, ../vast-reaper/wrangler.toml] Deploy the separate ci/vast-reaper Worker with its OWN cron and secret REAPER_VAST_API_KEY, never the main broker key. Grant only list/delete-capable routes api.instances and api.instance, without api.ask, api.bundles or user reads. Provider route-level permissions may include more methods than DELETE; confirm the available console granularity. The implementation itself only lists and deletes. It has no public controls, rental path, broker imports or LEDGER binding. Its own single-file source is the deploy artifact.

```sh
cd ../vast-reaper
/tmp/kf-vast-wrangler/node_modules/.bin/wrangler secret put REAPER_VAST_API_KEY
/tmp/kf-vast-wrangler/node_modules/.bin/wrangler deploy --dry-run \
  --var OWNER_GITHUB_ID:229738317
/tmp/kf-vast-wrangler/node_modules/.bin/wrangler deploy \
  --var OWNER_GITHUB_ID:229738317
```

[CODE READ: ../vast-reaper/worker.js] Default reaper hard=180 minutes, margin=15, cron every 5 minutes. It deletes only strictly older exact kf-ci-<run>-<attempt> labels with valid numeric provider start_date, after matching ID/label/start_date in two list snapshots. Unknown/future age, renamed rows, foreign labels, ambiguous IDs and incomplete/oversize lists fail closed. Each sweep deletes at most 20 and confirms absence; an acknowledged-but-present instance stays unconfirmed for the next sweep. Logs contain fixed event/count fields only. This deliberately reserves that exact namespace for broker rentals; do not apply it to unrelated boxes. The reaper cannot recover label-changed rentals from the broker's ledger: main cleanup handles those IDs.

[INFERRED: independent scheduler and account quotas] Put the reaper in a separate Cloudflare account/quota domain if request-quota independence is required. A separate Worker in the same exhausted free account is not an independent backstop. Provider outages, throttling and inaccurate start_date/labels can still prevent deletion. Keep prepaid balance small. The default reaper ends a rental past 195 minutes even if an owner stop-timer extended the main ledger; an extension does not extend the v2 callback token either. Agree any changed manual/reaper horizon before enabling rentals; do not silently increase the billing ceiling.

[CODE READ: docs/BOX_PROTOCOL.md; INFERRED: unexercised infrastructure] Deploy disabled first, configure the independent reaper and Access if chosen, verify owner stop and bundle metadata, then let the controller perform separate hosted/provider acceptance. Hosted OIDC/Access issuance, actual Vast contract fields/prices, provider scopes and bill accounting, scheduler CPU/quota usage and full GPU payload are not verified by mocks. No full-suite or hardware claim follows from dummy/mock success.

[CODE READ: src/tokens.js, src/onstart.js, scripts/ci/vast_report.py] This rollout intentionally rejects v1 run capabilities. Disable admission and drain/stop existing rentals before upgrading. Keep the existing ledger namespace for cleanup; do not erase reservations. The tested repository SHA must contain this round's v2-aware reporter and Access helper installation before rentals are enabled; an old reporter is not compatible with the new token. The candidate is not a live-deployment claim.
