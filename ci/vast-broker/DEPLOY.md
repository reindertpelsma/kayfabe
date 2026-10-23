# Owner deployment and independent cleanup

[CODE READ: wrangler.toml, deployment.json, deploy.mjs] The owner/controller deploys; no round
performs a deployment or a provider call. Keep `RENTING_ENABLED=false` until acceptance.
`main = "src/index.js"`: wrangler bundles the source with the lockfile-pinned dependencies (there
is no project linker), and `deploy.mjs` uploads only a bundle whose SHA-256 equals
`deployment.json`. No Cloudflare token belongs in GitHub.

## Authorization policy

[CODE READ: src/index.js, src/github.js, src/workflow-authority.js; owner rulings 2026-10-23
19:10 and 19:25] The whole rule is:

> Worker authorizes: signed identity + pinned workflow + triggering_actor == owner +
> sha == run head. YAML decides when to run.

- **Signed identity.** jose 6.2.13 (`jwtVerify` + `createRemoteJWKSet`, `src/oidc.js`): RS256
  only, issuer, audience (RFC 7519: the Worker URL, or an array containing it), `iat`, `nbf` and
  `exp` required, `iat` at most 600 s old, 30 s clock tolerance; keys from GitHub's JWKS, cached
  1 h, unknown-kid refetch at most once per 60 s cooldown. Then the Worker checks repository,
  repository id, owner id, environment subject, `ref == refs/heads/master`, numeric run id and
  attempt. (The hand-written verifier's `exp - iat <= 3600` bound is gone: `iat` age bounds use.)
- **Pinned workflow.** `job_workflow_ref == workflow_ref` and it is exactly this repository's
  `gpu-vast.yml@refs/heads/master` (events `pull_request_target` and `push`) or
  `gpu-vast-sweep.yml@refs/heads/master` (event `workflow_dispatch`, sweep profile only).
- **One run read.** `GET actions/runs/<id>` must match id, attempt, event and `in_progress`, name
  this repository as both `repository` and `head_repository` (fork heads refuse: the safer
  choice), and have `triggering_actor.id == OWNER_GITHUB_ID`. That also covers an owner re-run
  of someone else's run.
- **Tested commit.** The requested `tested_sha` must equal the run's `head_sha`. For `push` the
  OIDC `sha` must equal it too. MEASURED 2026-10-23 (read-only `gh api` on public
  `github/docs` `pull_request_target` runs, PR #46406): the
  run's `head_sha` is the PR head, while the OIDC `sha`/`workflow_sha` name the base commit, so
  those claims are not compared for that event.
- `pr_number` in the rent body is an informational echo for the runner; it authorizes nothing.
- When to run (branches, drafts, docs-only paths) lives only in `gpu-vast.yml` and the trusted
  BASE helper `trigger-policy.mjs`.

**Warning.** Owner-triggered events on someone else's PR (Reopen, Ready for review or Update
branch, or an owner re-run) make the owner the `triggering_actor`, so they authorize that PR's
current head commit, reviewed or not. If that is not wanted, add a YAML filter before enabling
renting: run `pull_request_target` only for owner-authored PRs, or only when an owner-applied
label is present. This round adds no such filter (owner decision).

[CODE READ: owner rulings 19:10/19:25] Removed from the shipping code (each retired item is
ignored if still configured):
- retired: review/approval lookups and `APPROVAL_MODE`, `RENT_DRAFTS`, `ALLOWED_BASE_BRANCHES`;
- retired: Worker `PATH_SKIP_PATTERNS` (path policy is YAML-only), PR lookups and PR authority;
- retired: the dispatch `sha` input, sweep `tested_sha` and owner branch-copy workflow authority;
- retired: MANUAL state, `/hold` box route, owner `/stop-timer` route and `MANUAL_CEILING_MINUTES`;
- retired: the Worker Access JWT switch and `ACCESS_*` vars (the box Access pair pass-through
  returned on 2026-10-23 21:45; see *Cloudflare Access* below).

Owner `stop`, `stop-all` and the box `kf-box-kill` switch remain.

## Build and test

[CODE READ: package.json, package-lock.json] Install first: `npm ci --ignore-scripts` in
`ci/vast-broker` (tests alone need `npm ci --omit=dev --ignore-scripts`: only `jose`). In
`ci/vast-broker`, run `npm test`; from the repository root, run `node --test ci/vast-broker/test`
(the CI form). Both run the same suite.
`--ignore-scripts` skips the install scripts of esbuild and workerd; their platform binaries
arrive as optional dependencies, and `npm ci` checks every tarball against the lockfile's sha512.

[MEASURED 2026-10-23, wrangler 4.152.0 / esbuild 0.28.2] The reviewed bundle is wrangler's own
output: `wrangler deploy --config wrangler.toml --dry-run --outdir <dir>` writes `<dir>/index.js`
(byte-identical across runs and checkout paths), and its SHA-256 is `bundle_sha256` in
`deployment.json` and `DEPLOYED_BUNDLE_SHA256` in `wrangler.toml` (vars are not part of the
module). `test/bundle.test.js` re-checks this when the dev dependencies are installed. After any
change under `src/`, `defaults.json` or the lockfile, record the new hash in both files. Preserve
the SQLite LEDGER namespace and migration when updating.

## Deploy the main Worker

[CODE READ: package-lock.json; deploy.mjs] Install Node 22+ and run `npm ci --ignore-scripts` in
`ci/vast-broker`: the lockfile pins wrangler 4.152.0 and every transitive package by integrity
(review F6). The helper uses `ci/vast-broker/node_modules/.bin/wrangler`.

Keep public vars in a local file outside the checkout (it is never committed). Example with
placeholders:

```sh
# /path/outside/checkout/broker.vars - KEY=VALUE, public values only
OWNER_GITHUB_ID=<numeric owner id, must equal wrangler.toml>
```

```sh
node ci/vast-broker/deploy.mjs /path/outside/checkout/broker.vars --dry-run
node ci/vast-broker/deploy.mjs /path/outside/checkout/broker.vars
```

The helper refuses before running wrangler when: the file is inside the checkout (realpath, so a
symlink is followed); a line is not `KEY=VALUE`; a key repeats; a key is not a listed public var
(secret names, `ALLOW_TEST_ISSUER`, `TEST_ISSUER`); a key is a retired `ACCESS_*` var or a
`BOX_ACCESS_*` secret (so an old Access vars file is refused); `OWNER_GITHUB_ID` is missing,
non-numeric or differs from `wrangler.toml`; `deployment.json` and `DEPLOYED_BUNDLE_SHA256`
differ; the wrangler at the pinned path is not 4.152.0; or the module wrangler emits for
`--dry-run --outdir <temp>` does not hash to `deployment.json`. It then uploads exactly that
verified file (`wrangler deploy --no-bundle <temp>/index.js`, so wrangler does not bundle twice)
with one `--var KEY:VALUE` per entry, runs wrangler with `WRANGLER_SEND_METRICS=false`, and
redacts every value from wrangler's output; refusals name a key or line number, never a value.
`WRANGLER=<path>` selects another binary of the same version (tests use a fake).

[INFERRED: wrangler 4.152.0 `deploy --help`] Without `--keep-vars`, a deploy replaces dashboard
vars with the toml plus `--var` values, so every public var must be in `wrangler.toml` or the vars
file. `OWNER_GITHUB_ID=229738317` is committed in `wrangler.toml` (a public account id, V1); a
deploy without it refuses rentals with `CONFIG_VAR:OWNER_GITHUB_ID` and the cron logs
`cleanup_config_error` instead of silently cleaning nothing.

Main Worker cron schedule: `* * * * *`

`preview_urls = false` (supported by the wrangler 4.152.0 schema) keeps version preview hostnames
off; `workers_dev = true` remains the public hostname.

[CODE READ: defaults.json, src/policy.js, src/offers.js, src/index.js] Check policy remains
$1/hour, $6 daily, $30 rolling, $8 reserve, 40-minute boot and 180-minute hard limit.
MAX_INET_USD_PER_GB defaults to 0.005 in each direction; the offer query, admission and contract
recheck enforce it. DENIED_COUNTRIES defaults to CU, IR, KP, SY before country preference.
ALLOW_UNKNOWN_COUNTRY defaults to true; set false to reject unknown metadata. PREFERRED_COUNTRIES
ranks only remaining eligible offers. Contract price failure revokes the run and attempts
deletion; failed deletion retains the liability for cleanup retry.

[INFERRED: src/ledger.js transfer-allowance accounting] Per-GB limits reduce exposure but do not
make reserved 15-GB-down/2-GB-up allowances a hard bill ceiling. A hostile box can exceed the
allowance. Keep the owner's main Vast account balance small, monitor it and retain an independent
cleanup backstop.

## Worker and GitHub secrets

[CODE READ: src/tokens.js, src/routes.js] Set Worker secrets with the pinned CLI (`wrangler secret
put <NAME>`): WORKER_URL (exact HTTPS origin), VAST_API_KEY, RUN_TOKEN_SECRET, OWNER_STOP_TOKEN
and BROKER_PATH_PREFIX (required when renting is enabled). Generate independent values without
printing them into logs:

```sh
head -c 32 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=\n'
```

[CODE READ: src/tokens.js, src/routes.js, src/index.js] The alphabet is `[A-Za-z0-9_-]`;
signing/owner secrets accept 32-256 characters and the prefix 32-128. Raw padded base64 with `+`,
`/` or `=` refuses. An owner-authenticated availability request reports
CONFIG_SECRET_FORMAT:<name> without the value; other callers receive a generic refusal. URL and
prefix secrecy reduce scanning noise; OIDC, the run read, live-ledger checks and the per-run
HS256 JWT run token (jose; signed with RUN_TOKEN_SECRET, expires at the hard limit) are the
authentication. Owner-token and prefix comparisons use the platform's `timingSafeEqual`. A Vast operator can read its KF_* env and callback URL; the box holds only
its own limited capability.

[CODE READ: src/github.js, src/blobs.js, src/index.js] Optional Worker secret GITHUB_API_TOKEN is
the default for GitHub reads (the run read, the box protocol marker and release metadata). Use one
fine-grained read-only token selected only for reindertpelsma/kayfabe and the private store
repositories: Contents, Pull requests, Actions and Metadata read. Authenticated 401/403 reads retry
public metadata without the token. Lifecycle run reads cache for 60 seconds; admission reads stay
fresh. BLOB_GITHUB_TOKEN is an asset-only deprecated fallback. Neither token enters box env,
storage or public responses/logs. Do not put a Worker GitHub token in GitHub Actions.

[CODE READ: src/blobs.js] For blob access, set BLOB_REPO=owner/store and a NONEMPTY
BLOB_ALLOWED_TAG_PREFIXES JSON array such as ["ci-image-"]. Missing/empty prefixes deny all mints.
The Worker returns only an approved short-lived release-asset redirect URL, size and asset name.

[CODE READ: gpu-vast.yml, gpu-vast-sweep.yml, workflow-guard.js] GitHub environment gpu-vast holds
VAST_BROKER_URL and VAST_BROKER_PATH, plus optional runner token A (VAST_BROKER_ACCESS_ID and
VAST_BROKER_ACCESS_SECRET), bound only in the sole trusted BASE broker step and passed to the
controller as CF_ACCESS_CLIENT_ID/CF_ACCESS_CLIENT_SECRET. Keep the environment's deployment-branch
policy master-only with a required reviewer (review V8); the Worker no longer honours branch
copies, so widening it buys nothing and exposes the environment secrets to branch workflows.

## Cloudflare Access is dashboard-only

[CODE READ: src/index.js; owner rulings 19:10 and 21:45] Access is a dashboard-only front door:
one hostname-wide policy on the workers.dev hostname with two service tokens. The Worker never
reads or verifies an Access header, JWT or var; Cloudflare checks the token at the edge.

- Runner (token A): GitHub environment secrets `VAST_BROKER_ACCESS_ID` /
  `VAST_BROKER_ACCESS_SECRET`; the runner sends `CF-Access-Client-Id` / `CF-Access-Client-Secret`
  only when both are set.
- Boxes (token B): optional Worker secrets `BOX_ACCESS_ID` / `BOX_ACCESS_SECRET`
  (`wrangler secret put`). Only when both are set, the Worker injects them as
  `KF_ACCESS_ID` / `KF_ACCESS_SECRET` next to the run token; the reporter, `kf-box-kill` and
  `kf-box-blob-url` then send the two headers (box protocol 7). The pair is masked like every
  other Worker secret and is never in a response, log or error body; the payload never sees it.

With the hostname-wide policy live, set the box pair BEFORE enabling renting: without it every
box callback is blocked at the edge and each rental bills until `BOOT_TIMEOUT` and FAILs.

## Vast key scope

[CODE READ: controller-provided scoped permission JSON, src/vast.js] The reviewed console
permission names are:

```json
{
  "misc": ["api.bundles"],
  "user_read": ["api.users.current"],
  "instance_read": ["api.instance", "api.instance.request_logs", "api.instances"],
  "instance_write": ["api.ask", "api.instance"]
}
```

[CODE READ: src/vast.js] This is the permission-name projection, not a credential file. The
console uses api.instance; the documentation spelling api.instance.show is invalid there. No
subaccount is assumed; the Worker is the capability boundary. Create-returned IDs stay owned even
when their label changes (fixed OWNERSHIP_MISMATCH event).

## V2 acceptance check (contract recheck)

[CODE READ: src/index.js checkContract, src/policy.js offerPrice; review V2] Behaviour is
unchanged in this round. At create and on every update the Worker re-applies the offer price rule
to the instance row and destroys the box when a field is missing or `dph_total` differs from
`dph_base + storage_cost*120/720` by more than 1e-6 relative. That is money-safe but would make CI
dead if real instance rows differ from offers. INFERRED: no recorded real instance row carries all
five fields. Acceptance check at the first watched rental: read that instance once, read-only, and
confirm `dph_base`, `dph_total`, `storage_cost`, `inet_up_cost` and `inet_down_cost` are present
and consistent. If not, stop with `stop-all` and decide on this proposal (not implemented):
destroy only on a positively higher parsed price; when instance fields are missing or
inconsistent, keep the admitted offer price and log a fixed event.

## Independent reaper

[CODE READ: ../vast-reaper/worker.js, ../vast-reaper/wrangler.toml] Deploy the separate
ci/vast-reaper Worker with its OWN cron and secret REAPER_VAST_API_KEY, never the main broker key.
Grant only list/delete routes api.instances and api.instance. It has no public controls, rental
path, broker imports or LEDGER binding, and takes no main-Worker var. Deploy first, then put the
secret (a `secret put` before the first deploy would prompt to create the Worker):

```sh
cd ci/vast-reaper
../vast-broker/node_modules/.bin/wrangler deploy --dry-run
../vast-broker/node_modules/.bin/wrangler deploy
../vast-broker/node_modules/.bin/wrangler secret put REAPER_VAST_API_KEY
```

Reaper cron schedule: `*/5 * * * *`

[INFERRED: workers_dev=false and no routes] Confirm the deploy output lists that schedule; the
reaper has no other trigger. Until the secret is set each tick logs `reaper_unavailable` and
deletes nothing.

[CODE READ: ../vast-reaper/worker.js] Default hard=180 minutes, margin=15. It deletes only
strictly older exact kf-ci-<run>-<attempt> labels with a valid numeric provider start_date, after
matching ID/label/start_date in two list snapshots; anything uncertain fails closed. Each sweep
deletes at most 20 and confirms absence. A failed DELETE no longer aborts the sweep (V9): the
remaining boxes are still deleted, the report counts `{deleted, unconfirmed, failed}` and the next
tick retries. A failed list still aborts the whole sweep. Logs carry fixed event/count fields only.

[INFERRED: independent scheduler and account quotas] Put the reaper in a separate Cloudflare
account if request-quota independence is required. Provider outages and inaccurate
start_date/labels can still prevent deletion. Keep the prepaid balance small.

## Rollout

[CODE READ: docs/BOX_PROTOCOL.md, src/github.js, scripts/ci/vast_report.py] Box protocol is 7.
Before any Vast call the Worker reads `scripts/ci/vast_report.py` at the exact tested sha and
refuses `NOT_ELIGIBLE` unless it contains the line `BOX_PROTOCOL = "7"` (V6), so commits from
before this round cannot burn a boot window. Earlier (v1/v2) run capabilities are rejected: run
tokens are HS256 JWTs since protocol 7. Disable admission and drain existing rentals before
upgrading, and keep the ledger namespace.

[INFERRED: unexercised infrastructure] Deploy with renting off first, deploy the reaper and confirm
its schedule, verify owner stop and bundle metadata, then do hosted acceptance: OIDC claims, the
V2 instance-row check, the box Access pair (set before renting) and one watched rental. The payload
script `scripts/ci/vast_payload.sh` is still absent (F14), so a real rental today posts
`payload_missing` and FAILs. Mocks prove no hosted, provider or hardware behaviour.
