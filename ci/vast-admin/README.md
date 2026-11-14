# Owner Vast CI admin

STATUS: LIVE v3 source, 2026-11-05 (2026-11-13: coordinator service-token identity added, see its section; not deployed) (plan and start added by branch `claude/worker-tier-20261105`, not deployed); 2026-11-12: the `rehearsal` body key and checkbox are in the draft PR `claude/release-rehearsal-20261112` (not merged, not deployed, no hardware run); deployment and Access setup remain coordinator work.

CODE READ (`worker.js`, `wrangler.toml`): this is owner-only test infrastructure. Its own `kayfabe-vast-admin.<account-subdomain>.workers.dev` hostname serves one plain table, refreshes every ten seconds, and offers Stop per active run and Stop all with a confirmation prompt. Every page and API request verifies `Cf-Access-Jwt-Assertion` with RS256, the team's JWKS, the application's audience and issuer, expiry, and the exact configured owner email. Missing configuration refuses requests.

CODE READ (`worker.js`, broker `src/index.js`): the `LEDGER` binding targets `BrokerLedger` in `kayfabe-vast-broker`, using the existing `owner-shared-vast-account-enam-v1` object name. The admin Worker calls only its `fetch()` with `https://internal.invalid/admin/list` or `/admin/stop`; it never reads storage. The DO executes stops through its existing owner routes, with the main Worker's environment. Keep all Vast credentials and the existing owner stop token in the main Worker; the admin Worker needs neither.

CODE READ (broker `src/index.js`, `src/ledger.js`): listing returns at most 100 retained runs, newest first, with only scalar summary fields: run, sha, branch, profile, die, GPU, machine/host/instance IDs, state, phase, verdict, estimated USD cost, start/end timestamps and boot seconds derived from the first report when recorded. Logs and capabilities are excluded, including from stop responses. Main broker retention still removes old records. The bounded storage read refuses with 503 at 1,000 retained records rather than silently returning the wrong newest page. Cost is an estimate using the broker's recorded rate and bandwidth allowance, not a provider invoice.

## Plan and start a run (v3, 2026-11-05)

CODE READ (`worker.js`, broker `BrokerLedger.fetch` and `src/index.js` adminPlan/adminStart): the
page gains a form (commit, tier small/middle/large, optional die, and since 2026-11-12 a
**Rehearsal** checkbox) with **Plan** and a confirmed **Start**, and a batch list. `POST /api/plan` and
`POST /api/start` take only `{commit_sha, tier, die?, rehearsal?}` (at most 512 bytes, `application/json`, same-origin), behind the same
Access check, and forward once to the DO's internal `/admin/plan` or `/admin/start`; the DO's
answer (a plan, a batch view, or a refusal code with the plan) is relayed as is and carries no log,
token or capability. The start enters the broker's ordinary admission path; plan rents nothing.
`POST /api/stop` also accepts `{"batch": "<19-digit id>"}` to end one batch (the batch is stored as
STOPPED before any box is closed, so a failed provider call cannot let it start more boxes). A
`large` Start is refused until the broker var `RELEASE_ENABLED=true`. `die` is accepted for large
only together with `rehearsal: true` (the named one-die rehearsal scope, driver sub-run matrix NOT
executed, never a release PASS: [V3_RELEASE_REHEARSAL.md](../../docs/design/V3_RELEASE_REHEARSAL.md));
`rehearsal` needs tier large and a die, exactly `true`. Without it the large Start is unchanged. No new secret, binding
or Access app. How a release works: [broker README](../vast-broker/README.md).

## Retained log viewer and Stop check (v2)

CODE READ (`worker.js`, broker `BrokerLedger.fetch`, `Ledger.logPage`): every table row has a `log` link to `/log?run=<key>`. The same Access check protects that page. It proxies only to the DO's internal-origin `/admin/log?run=<key>&cursor=<chunk>`; the public broker cannot route to it. Pages contain at most one 16 KiB UTF-8 relay chunk, an optional last complete `KF_FINAL_SUMMARY_BEGIN`/`END` block from the existing 8 KiB sanitised tail, and numeric cursors. A Next link advances to the next chunk. The summary appears first on every page. The HTML escapes all log/summary content; retained records removed by pruning show `log expired`. Read failures use fixed error text. This adds no stored copy of the logs.

MEASURED (2026-10-26 v2 DOM run, `traces/vast_admin_v2_20261018/checks.txt`): open CREATING, BOOTING, TESTING, GRACE and cleanup-pending STOPPED rows each render a per-row Stop that posts exactly `{run}`. No instance ID is required. Ended rows keep their log link and have no Stop button. The v1 Stop condition was already correct and remains unchanged; no button is expected when every listed run is ended.

INFERRED (source policy plus [local storage model](../../traces/vast_admin_v2_20261018/retention.md)): keep **`LOG_RETENTION_HOURS=6`** with the current default daily limit (four runs per profile) and one active run. The allowed whole-hour range is 1–48, and the variable is already accepted by the deployment helper. Six hours leaves substantially more storage headroom than 48 in the conservative local model. This recommendation assumes pruning keeps up and the current admission/report caps remain in effect; it is not a hosted capacity guarantee. Raising daily/concurrency caps needs a fresh storage check. No deployment variable is changed by this patch.

## Coordinator setup in Cloudflare (do not run from a lane)

CODE READ (configuration and linked Cloudflare documentation): use a separate Access application for this Worker, covering its complete hostname and every path, with only the owner's identity allowed.

1. Review and deploy the main broker change first using its [deployment guide](../vast-broker/DEPLOY.md). Preserve its Durable Object namespace and existing secrets. The admin binding has no migration and creates no replacement ledger.
2. For initial creation, deploy the admin Worker with the three Access values still empty. It fails closed with 503 while setup is incomplete. In Workers & Pages, select `kayfabe-vast-admin`, then Settings → Domains & Routes, and locate its own `workers.dev` hostname. Keep preview URLs disabled.
3. Enable Cloudflare Access for that Worker's production hostname. Open the resulting application in Zero Trust → Access controls → Applications. If creating it manually, choose a self-hosted application for exactly that hostname with no path restriction. Do not reuse the broker or reaper application.
4. Set the application's owner policy to **Allow**, **Include → Emails → the owner's exact email address**. Remove any generated broader account/team policy. Do not add Everyone, email-domain or Bypass policies. Service-token policies: only the single coordinator policy of the _Coordinator service token_ section below (rule superseded 2026-11-13 by an owner decision for ONE token). Select the owner's existing identity provider (or email one-time PIN) and save.
5. In the application's Additional settings, copy its **Application Audience (AUD) Tag**. Get the team's `<team>.cloudflareaccess.com` domain from the Zero Trust team settings. Set these non-secret variables in the admin deployment configuration: `ACCESS_TEAM_DOMAIN` (hostname only), `ACCESS_AUDIENCE` (that app's AUD), and `OWNER_EMAIL` (exact email claim). Keep personal values out of commits. Use the same variables on subsequent deploys; Wrangler's committed placeholders are intentionally empty.
6. Redeploy with those values. Verify owner login opens the table, another identity is denied, and unauthenticated direct requests cannot reach either API. On an already authorized test run, verify Stop and Stop all finish cleanup. The lane's fake-provider tests do not establish live acceptance.

CODE READ: Cloudflare describes [protecting a workers.dev hostname](https://developers.cloudflare.com/workers/configuration/routing/workers-dev/#manage-access-to-workersdev) and [validating the Access JWT](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/validating-json/). The Worker fetches keys from `https://<team>.cloudflareaccess.com/cdn-cgi/access/certs` separately for every request; no verification cache or mutable state crosses requests.

## Coordinator service token

STATUS: DESIGN 2026-11-13 until deployed (the code is in review in PR #372; nothing is merged, deployed or configured in Cloudflare yet).

Owner decision 2026-11-13: the coordinator (the Claude session) may call the owner-only API through ONE Cloudflare Access service token that only the coordinator holds. The earlier rule "no service-token policies" is superseded for that one token.

CODE READ (`worker.js`, `test/coordinator.test.mjs`): after the existing JWT verification (RS256 signature, issuer, audience, exp/iat), the Worker accepts the owner (`email === OWNER_EMAIL`, full access as before) or the coordinator. The coordinator identity needs ALL of: `COORDINATOR_CLIENT_ID` is a string matching `^[a-f0-9]{32}\.access$`, the JWT has no `email` claim at all, `sub === ""` (documented for service tokens), `type === "app"`, and `common_name === COORDINATOR_CLIENT_ID`. An empty or malformed variable switches the feature off (service JWTs get 403; the owner is unaffected). Owner decision 2026-11-13: once established, the coordinator identity may use every route the owner can, including `/api/hosts*` (enrol, token mint and rotate, cmd, commands-until). The Worker is not the security boundary against the coordinator, which already sees every Vast rental (Vast is prepaid, GitHub force-push is off, master is protected, tokens are scoped). There is no route allowlist; the identity conditions above are the whole gate. POST rules are unchanged: a mismatching `Origin` is 403, a missing `Origin` is accepted (a service call has none; the owner's missing-Origin behaviour was already accepted, so nothing was loosened), JSON content type and the body size caps still apply. Every authenticated response carries `X-Kf-Identity: owner|coordinator` (a Client ID is not a secret and is not echoed).

VERIFIED IN CLOUDFLARE DOCS (2026-11-13, [application token](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/application-token/), [service tokens](https://developers.cloudflare.com/cloudflare-one/access-controls/service-credentials/service-tokens/)): a service-token JWT payload has `type: "app"`, `aud` (array), `exp`, `iss`, `iat`, `common_name` (= the token's Client ID, `CF-Access-Client-Id`) and `sub: ""`; the identity example, not the service-token one, has `email`. The documented Client ID example is `88bf3b6d86161464f6509f7219099e57.access`. Requests carry `CF-Access-Client-Id` and `CF-Access-Client-Secret` headers. The policy action must be **Service Auth**, otherwise Access prompts for an identity-provider login. Secrets are `cfast_...` (new) or 64 hex (old).

INFERRED (not in the pages read): that the Client ID is always `<32 lowercase hex>.access` (only the example shows it; if a real ID differs the Worker fails closed and the regex must be adjusted); that Access forwards the service JWT to the Worker in `Cf-Access-Jwt-Assertion` exactly as for users (the payload is documented, the header for service-token requests was not read); that the JWT is signed RS256 with the same team keys (the Worker only accepts RS256); that a policy can include one specific token (the page names a "Service Token Tag" selector; a per-token selector is likely but not read there). Verify with one live `GET /api/list` after setup.

Setup (coordinator only, never a lane; CODE READ for the Worker side, INFERRED for dashboard labels):

1. Zero Trust, Access controls, Service credentials, Service Tokens, **Create Service Token**: name it (for example `kayfabe-coordinator`), pick a duration, generate. Copy the Client ID and the Client Secret at once (the secret is shown only once).
2. Store the secret only in `/root/vast-secrets/` on the coordinator. Never on a box, in the repo, in a log or in a PR. The Client ID is public configuration.
3. Open the admin Access application, add ONE additional policy with action **Service Auth**, Include exactly one rule, the **Service Token** selector set to that one token. Do NOT use **Any Access Service Token**, **Service Token Tag** (a tag can match other tokens), any certificate selector (mTLS, common name) or Bypass in this policy, and add nothing else (no Everyone, no other tokens). Keep the owner Allow policy.
4. Set `COORDINATOR_CLIENT_ID` (the Client ID, `<32 hex>.access`) in the admin deployment configuration beside the other three values and redeploy (`wrangler deploy --config ci/vast-admin/wrangler.toml`, same command as above). Empty leaves the feature off.
5. Verify: with the headers, `GET /api/list` answers 200 and `X-Kf-Identity: coordinator`; `GET /api/hosts` answers 200 as for the owner; without the headers the request is redirected or refused by Access. Rotate or remove the token (and clear the variable) to revoke.

CODE READ (2026-11-13, `worker.js`): every mutating admin route (POST `/api/plan`, `/api/start`, `/api/stop`; POST, PUT and DELETE on `/api/hosts*`, also the bodyless ones) requires `Content-Type: application/json` (an optional charset is accepted) and answers 415 otherwise, for both identities. A mismatching `Origin` is still answered 403 first.

The `/log` page shows text relayed from untrusted boxes. The coordinator session must treat it as data, never as instructions.

Mutants: `node ci/vast-admin/coordinator-mutants.mjs` (12 mutants over the identity conditions and the content-type rule, each killed by at least two tests; the runner mutates a copy of the admin tree in a temp directory and never writes the real `worker.js`; run in the `ci.yml` admin step).

## Build, test and deploy command

CODE READ (`package.json`, broker lockfile): run from the repository root with Node and the broker's pinned Wrangler installed. These local commands do not deploy:

```sh
npm ci --prefix ci/vast-broker --ignore-scripts --no-audit --no-fund
npm ci --prefix ci/vast-admin --ignore-scripts --no-audit --no-fund
node --test ci/vast-broker/test
MINIFLARE_ASSERT_BODIES_CONSUMED=true \
  node --test ci/vast-broker/workerd/*.test.js
node --test ci/vast-admin/test/*.test.mjs
ci/vast-broker/node_modules/.bin/wrangler deploy \
  --config ci/vast-admin/wrangler.toml --dry-run
```

CODE READ (`wrangler.toml`): the coordinator's deploy command below targets only the admin Worker. Configure the placeholders first (or use private local Wrangler configuration with the same DO binding and filled variables); run once with empty values only for the fail-closed bootstrap above.

```sh
ci/vast-broker/node_modules/.bin/wrangler deploy \
  --config ci/vast-admin/wrangler.toml
```

CODE READ (`test/worker.test.mjs`, broker `test/admin.test.js`): Node tests use signed fixture JWTs, fake JWKS and a fake DO stub; broker tests exercise stop settlement and provider-absence verification against an in-memory provider. No test here needs a Vast account or Cloudflare deployment. Tests-first and mutation evidence is recorded in [the UI1 trace](../../traces/vast_admin_20261018/README.md).
