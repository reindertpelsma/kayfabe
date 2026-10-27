# Owner Vast CI admin

STATUS: LIVE v2 source, 2026-10-26; deployment and Access setup remain coordinator work.

CODE READ (`worker.js`, `wrangler.toml`): this is owner-only test infrastructure. Its own `kayfabe-vast-admin.<account-subdomain>.workers.dev` hostname serves one plain table, refreshes every ten seconds, and offers Stop per active run and Stop all with a confirmation prompt. Every page and API request verifies `Cf-Access-Jwt-Assertion` with RS256, the team's JWKS, the application's audience and issuer, expiry, and the exact configured owner email. Missing configuration refuses requests.

CODE READ (`worker.js`, broker `src/index.js`): the `LEDGER` binding targets `BrokerLedger` in `kayfabe-vast-broker`, using the existing `owner-shared-vast-account-enam-v1` object name. The admin Worker calls only its `fetch()` with `https://internal.invalid/admin/list` or `/admin/stop`; it never reads storage. The DO executes stops through its existing owner routes, with the main Worker's environment. Keep all Vast credentials and the existing owner stop token in the main Worker; the admin Worker needs neither.

CODE READ (broker `src/index.js`, `src/ledger.js`): listing returns at most 100 retained runs, newest first, with only scalar summary fields: run, sha, branch, profile, die, GPU, machine/host/instance IDs, state, phase, verdict, estimated USD cost, start/end timestamps and boot seconds derived from the first report when recorded. Logs and capabilities are excluded, including from stop responses. Main broker retention still removes old records. The bounded storage read refuses with 503 at 1,000 retained records rather than silently returning the wrong newest page. Cost is an estimate using the broker's recorded rate and bandwidth allowance, not a provider invoice.

## Retained log viewer and Stop check (v2)

CODE READ (`worker.js`, broker `BrokerLedger.fetch`, `Ledger.logPage`): every table row has a `log` link to `/log?run=<key>`. The same Access check protects that page. It proxies only to the DO's internal-origin `/admin/log?run=<key>&cursor=<chunk>`; the public broker cannot route to it. Pages contain at most one 16 KiB UTF-8 relay chunk, an optional last complete `KF_FINAL_SUMMARY_BEGIN`/`END` block from the existing 8 KiB sanitised tail, and numeric cursors. A Next link advances to the next chunk. The summary appears first on every page. The HTML escapes all log/summary content; retained records removed by pruning show `log expired`. Read failures use fixed error text. This adds no stored copy of the logs.

MEASURED (2026-10-26 v2 DOM run, `traces/vast_admin_v2_20261018/checks.txt`): open CREATING, BOOTING, TESTING, GRACE and cleanup-pending STOPPED rows each render a per-row Stop that posts exactly `{run}`. No instance ID is required. Ended rows keep their log link and have no Stop button. The v1 Stop condition was already correct and remains unchanged; no button is expected when every listed run is ended.

INFERRED (source policy plus [local storage model](../../traces/vast_admin_v2_20261018/retention.md)): keep **`LOG_RETENTION_HOURS=6`** with the current default daily limit (four runs per profile) and one active run. The allowed whole-hour range is 1–48, and the variable is already accepted by the deployment helper. Six hours leaves substantially more storage headroom than 48 in the conservative local model. This recommendation assumes pruning keeps up and the current admission/report caps remain in effect; it is not a hosted capacity guarantee. Raising daily/concurrency caps needs a fresh storage check. No deployment variable is changed by this patch.

## Coordinator setup in Cloudflare (do not run from a lane)

CODE READ (configuration and linked Cloudflare documentation): use a separate Access application for this Worker, covering its complete hostname and every path, with only the owner's identity allowed.

1. Review and deploy the main broker change first using its [deployment guide](../vast-broker/DEPLOY.md). Preserve its Durable Object namespace and existing secrets. The admin binding has no migration and creates no replacement ledger.
2. For initial creation, deploy the admin Worker with the three Access values still empty. It fails closed with 503 while setup is incomplete. In Workers & Pages, select `kayfabe-vast-admin`, then Settings → Domains & Routes, and locate its own `workers.dev` hostname. Keep preview URLs disabled.
3. Enable Cloudflare Access for that Worker's production hostname. Open the resulting application in Zero Trust → Access controls → Applications. If creating it manually, choose a self-hosted application for exactly that hostname with no path restriction. Do not reuse the broker or reaper application.
4. Set the application's only policy to **Allow**, **Include → Emails → the owner's exact email address**. Remove any generated broader account/team policy. Do not add Everyone, email-domain, Bypass or service-token policies. Select the owner's existing identity provider (or email one-time PIN) and save.
5. In the application's Additional settings, copy its **Application Audience (AUD) Tag**. Get the team's `<team>.cloudflareaccess.com` domain from the Zero Trust team settings. Set these non-secret variables in the admin deployment configuration: `ACCESS_TEAM_DOMAIN` (hostname only), `ACCESS_AUDIENCE` (that app's AUD), and `OWNER_EMAIL` (exact email claim). Keep personal values out of commits. Use the same variables on subsequent deploys; Wrangler's committed placeholders are intentionally empty.
6. Redeploy with those values. Verify owner login opens the table, another identity is denied, and unauthenticated direct requests cannot reach either API. On an already authorized test run, verify Stop and Stop all finish cleanup. The lane's fake-provider tests do not establish live acceptance.

CODE READ: Cloudflare describes [protecting a workers.dev hostname](https://developers.cloudflare.com/workers/configuration/routing/workers-dev/#manage-access-to-workersdev) and [validating the Access JWT](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/validating-json/). The Worker fetches keys from `https://<team>.cloudflareaccess.com/cdn-cgi/access/certs` separately for every request; no verification cache or mutable state crosses requests.

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
