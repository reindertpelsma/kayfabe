# Vast broker (stage 1)


## V5 lifecycle and shipped policy (2026-10-22; acceptance pending)

[CODE READ: defaults.json, src/defaults.js, src/policy.js] Numeric defaults live
in `defaults.json`; JavaScript imports them through `src/defaults.js`, the bundle
embeds that same file, and wrangler.toml has no numeric override. The owner-approved
check cap is $1.00/hour, daily $6, rolling 30-day $30, prepaid reserve $8.
Renting remains disabled in the committed configuration. Profiles and effective storage/network admission are implemented below. The V5 runner protocol is implemented; payload integration and live acceptance
are still pending.

[CODE READ: src/index.js, src/onstart.js] States are NONE, CREATING, BOOTING,
TESTING, GRACE and MANUAL. Creation stores absolute boot (40 minutes), hard
(3 hours), soft (hard minus 10 minutes) and manual-ceiling (8 hours) deadlines.
Empty logs count as heartbeats: the sender emits one within 10 minutes; 15 minutes
without an authenticated post destroys the box. PASS retains 5 minutes of grace;
FAIL retains 10. SSH login calls the per-run hold helper to enter MANUAL, suspending
grace and heartbeat requirements while retaining the hard limit. Only the owner
can remove that hard timer; admission checks the extended billing reservation.
MANUAL still ends at the absolute ceiling, and cron writes a Worker-authored line.

[INFERRED: defaults.json and src/ledger.js arithmetic] Two complete three-hour
runs at $1.00/hour cost $6 in work time. The explicit budget check also reserves
one cron minute: each reservation is $3.016667, so two worst-case reservations
are $6.033334 and do **not** fit the unchanged $6 daily cap. Two such reservations
fit at no more than $0.994475/hour; shorter settled runs can release enough room.
Admission refuses excess daily/rolling liability or prepaid credit below reserve
plus the new liability. The cron margin is a healthy-scheduler estimate, not a
provider billing bound during an outage.


## V5 profiles, forced admission and availability

[CODE READ: defaults.json, src/policy.js, src/offers.js] `check` is the PR profile:
configured measured classes GA106 first, then AD104, GA102 and AD102, capped at
$1 effective/hour and the shared $6 daily/$30 rolling/$8 reserve limits. No
qualifying class returns UNAVAILABLE; the Worker never substitutes another die.
AD107 is excluded until the owner adds it after three qualifying sweep passes.
`sweep` is a separate owner-only workflow_dispatch profile, with a $2.50 effective
hourly cap and its own $8 daily budget; rolling liability and prepaid reserve stay
shared. Its workflow must not write the required gpu-vast status. Sweep rotation
uses the configured order and the persisted last-rented class, with an optional
recent-pass exclusion. The runner prints class/driver/date coverage tags from accepted structured results;
hosted dispatch execution remains unverified.

[CODE READ: 2026-10-22 owner verification rule, src/policy.js] Every profile requires an explicitly verified provider
response: verified=true, or the documented verification="verified" when the boolean
alias is absent. Missing/unknown/conflicting verification refuses; no configuration
can disable this requirement. The default quality thresholds are reliability >=0.98,
8 effective cores, 2 GHz, 34360 MB RAM (at least 32 GiB), 200 Mb/s down, 50 Mb/s up,
100 MB/s disk bandwidth, and positive dlperf at least half its class's median in
the current verified offer sample. Missing/odd fields refuse. Exact offer driver
tags must be in the configured tested matrix. In VM mode this is an offer-quality
filter; the actual kayfabe host driver is the separately provisioned VM driver,
which must be checked again on the box. Performance scores never grade the code.

[CODE READ: defaults.json, src/policy.js] GPU eligibility uses numeric compute_cap
>=750 (7.5), NVIDIA gpu_arch, and the consumer/workstation RTX or GTX 16xx name
class; datacenter names are excluded. Configured name/capability rules map known
models to die classes. Unmapped workstation names use CC_<capability> classes in
sweep; they cannot enter the check profile. Provider names are class hints, not
hardware attestation; a wrong/ambiguous mapping refuses or remains a coverage
finding. GPU patterns and known-bad machine IDs are configurable deny lists.
The historical public nvkvm-pv machine list was reviewed for its KVM failure
signature; only machine IDs were carried over, with no old endpoints or PSK flow.
Boot expiry can learn an own machine only after created plus an exact own-instance
libvirt domain-not-found log remains byte-identical across observations at least
two minutes apart, with a recent confirming observation. Time alone never learns.

[CODE READ: src/offers.js, src/index.js] GET /availability?profile=check|sweep
requires GitHub OIDC and profile authorization. A shared <=60-second read-only
market cache serves both profiles; a 30/minute admission limiter also covers rent
refusals. Returned class/count/price/quality and rejection totals contain no offer,
machine or network identifiers. Counts describe a bounded query sample. /rent
accepts at most 16 ordered class tokens, intersects them with the Worker's forced
set, reports how many were ignored, and refuses an empty intersection. A budget
refusal can show the local chosen-class enum without creating a rental. The
runner must independently schema-check and scrub every display field.

[CODE READ: src/policy.js, src/ledger.js; [Vast offer fields](https://docs.vast.ai/api-reference/search/search-offers)]
Storage is priced in USD/GB/month; the requested 120 GB is converted using 720
hours/month and cross-checked against dph_total, which already includes it.
inet_down_cost/inet_up_cost are USD/GB (not the console's per-TB display).
Default transfer allowance is 15 GB down plus 2 GB up. Effective hourly price is
compute plus storage plus the full transfer allowance amortised over the hard
limit. Reservation uses the same compute/storage lifetime plus cron margin and
full transfer allowance. Early settlement retains that unmeasured allowance;
it does not release it merely because the box ran briefly.

[MEASURED: run v5-readonly-20261022-availability-1, traces/vast_broker_v5_20261018/read_only.json]
The instance API exposes inet_down_billed/inet_up_billed, but their units and
refresh reliability are unverified. Consequently cron does not assert a bandwidth
usage cutoff: only the conservative allowance reservation is implemented. A
compromised box can exceed that allowance; neither it nor delayed cron/API
accounting establishes a finite external bill. This is the owner's specified
reservation-only fallback, not a claim of provider-enforced transfer limits.


[CODE READ: owner correction 2026-10-22; runner.py, gpu-vast.yml and src/onstart.js] The exact code is approved before Vast starts. A forged GPU verdict from the Worker **or** the box cannot change approved code or contaminate the final build. `gpu-vast` is an unattested tripwire; the Worker is never part of an attested result. The one-page [trust and public trace path](../../docs/design/V3_CI_VAST_RUNNER_TRACE_20261022.md) defines these boundaries:

| Who distrusts whom | Why and enforced boundary |
| --- | --- |
| Approved-code/release policy → Worker **and** Vast GPU verdicts | The exact code is approved **before Vast starts**. Either a malicious Worker or a malicious box can forge a GPU verdict, but cannot change that approved code or supply final build inputs: **no contamination**. `gpu-vast` is an unattested tripwire. |
| CI → Worker | Build provenance must not trust the developer's assertions or machine; putting a developer-controlled Cloudflare Worker in an attested result's path would bypass that independence. Worker/GPU results are never attested. The protected BASE runner validates structure, bindings and every displayed string itself, and independently scrubs/caps output. |
| Worker → Vast box/operator | These are community/consumer hosts with observable guest root. The box never receives the Vast account key, App key, owner stop token or Worker signing secret; it gets only its limited run capability. |
| Worker → CI content/secrets | The billed Vast key must **never be a GitHub secret**: LLM agents and future contributors may reach repository secrets. CI presents only OIDC identity; the Worker independently checks identity, approved content and spending policy. |
| CI → Vast | Box output can inject logs/commands; a community box cannot supply an attested result. CI sees Vast only through the Worker, with no direct box connection and no executable/artifact input from Vast. |
| Vast box → Worker, but not its network | The box trusts the Worker's fixed task/callback, uses **HTTPS only, no redirects**, and has a per-run token with no authority beyond report/kill/hold/blob-URL access for that run. Live authority dies with the box/run. The host operator can already see the box and its token; network interception must not add authority. |

[CODE READ: src/index.js, src/tokens.js, deployment.json] Current per-run authority is report/kill/hold; no blob-URL route is implemented. Blob-URL scope above is a policy ceiling, not an added capability. Worker protocol/version and expected bundle hash are visible, but the deployed hash is an owner assertion, not remote deployment attestation.

[CODE READ: 2026-10-22 Round 3, deployment.json] Build with `npm run bundle`; independently compare SHA256 of `dist/worker.js` with `dist/deployment.json`, committed `deployment.json` and public `DEPLOYED_BUNDLE_SHA256` before a separately authorized deployment. CLI deployment must upload those exact bundled bytes (use `npx --yes wrangler@4.152.0 deploy dist/worker.js --no-bundle`); dashboard/module upload uses the same file. The hash is a configuration assertion, avoiding a self-hash in the bundle. Any source update requires regenerating/reviewing the manifest and configuration together; mismatches refuse CI. This task performs no deployment and leaves renting disabled.

[CODE READ: 2026-10-22 V4 round 2 owner instruction] **Stage only with RENTING_ENABLED=false.** The startup contract is SSH launch (`ssh_direct`) plus onstart; Vast injects keys already registered on the owner's account, and the Worker neither manages keys nor connects over SSH. Round 2 supersedes the earlier strict no-SSH STOP. The deployable source retains its KVM image/vm=true; the local tiny Docker/SSH flow is measured separately under [round 2 evidence](../../traces/vast_broker_v4_round2_20261018/README.md) and does not establish that KVM workload's validity. Those historical V4 shipped limits are superseded by the owner-approved V5 policy above. The local V4 round allowed $0.80/hour and any NVIDIA GPU.

[CODE READ] Plain ES-module JavaScript, zero npm dependencies, Node >=22. This is an **untrusted GPU tripwire**, never release provenance. The Worker alone holds the owner's dedicated Vast key. No deployment or rental is performed by tests. Binding defaults refuse spending until configured. The complete threat model, API contracts, billing risks and deferred merge queue are in [the design](../../docs/design/V3_CI_VAST_GPU_20261021.md).

[CODE READ: 2026-10-22 smoke-live.py, live/] Round 2's real controller is `python3 smoke-live.py --runtime <private-new-directory> --wrangler <temporary-install>/node_modules/.bin/wrangler`. It uses real Vast API calls and local fake GitHub/JWKS identity, a private any-NVIDIA model overlay, and a tiny deterministic workload under the production reporting sender. It starts an independent exact-label reaper before every create, drives rent/status/cursor logs, and invokes the real local scheduled handler once per minute without a fake clock. PASS runs the shipped kill CLI inside the box after 30 seconds; FAIL waits for result grace. `--docker-smoke` additionally selects a local Ubuntu Docker image with vm=false, as authorized for round 2; that result cannot establish the shipped KVM image/workload's validity. No overlay is deployed. The callback tunnel accepts only report/kill from the provider's exact public_ipaddr. On exit, exact-ID and exact-label absence precede reaper cancellation; all controller children stop. Runtime files contain credentials and must remain private and be deleted after sanitized evidence is retained.

[CODE READ: 2026-10-22 src/index.js] A provider row whose actual_status is initially null or absent gets only the existing provisioning window, like created/loading/starting. It cannot report a result until actual_status is running. Neither cur_state nor intended_status grants liveness. This corrects premature destruction observed in the round 2 startup evidence.

[CODE READ] Automatic PR events use default `owner_push_or_review`: numeric GitHub event actor/triggering-actor identity is the owner, or an owner review approves exactly that PR head SHA. Commit author text has no authority. An owner push also vouches for earlier commits on that branch. Drafts refuse by default. Push storms do not queue rentals: superseded work is destroyed and the newer request returns BUSY. `review_or_author`, `review_only`, `owner_push` and `actor_only` are explicit alternatives. Automatic PRs require an API-checked master base (ALLOWED_BASE_BRANCHES) and a non-draft head. The Worker excludes only a complete all-skipped PR file list using PATH_SKIP_PATTERNS; unknown/truncated lists run, and API errors fail. ci/vast-broker always runs. NOT_NEEDED is an explicit successful job/status. The BASE workflow permits owner dispatch on master only. A repeat request within the same runner nonce is idempotent; a fresh run cannot inherit another run's cached PASS. Rerun the owner's dispatch after the minimum gap to request another rental.

[MEASURED: 2026-10-22 local Node test/bundle commands] Run `npm test` (or `node --test ci/vast-broker/test` from the repo root), `npm run bundle`, and `npm run mutants` from this directory. No npm install is needed. The fixed-source linker writes ignored `dist/worker.js`, checks syntax and preserves the entrypoint exports. Tests use generated RSA keys, fake JWKS, mocked GitHub/Vast, an in-memory Durable Object adapter and a local HTTP server.

[MEASURED: 2026-10-22 format bundle run, traces/vast_broker_v5_20261018/FORMAT.md]
The formatted checkpoint bundle is 103026 bytes, SHA256
`1a06b7c87dd069240c4404503353b458d32e4d3ae64a680f35a5ac8f33d83eb8`.
Prettier 3.6.2 and Black 25.1.0 use width 100; CI enforces both pinned versions.

[CODE READ: 2026-10-23 owner V5_SECURITY and V5_ONSTART_AUTH instructions]
**Security transition incomplete; keep renting disabled.** The working security
checkpoint fixes cleanup starvation, failed-create recovery and bounded holds,
but the mandatory prefix and tiny bootstrap/env transition remain red. Further
claim/nonce/redaction/rate-limit hardening, blob route and live acceptance are
pending; this is not a replacement renting-ready verdict.

[MEASURED: 2026-10-23 security transition run, traces/vast_broker_v5_20261018/SECURITY_PROGRESS.md]
The emitted security-work bundle is 110672 bytes, SHA256
`2ab3856a7430129334fa430070419f83d1e2f920d55a3190aff7718f04aaf897`.
The portable test entry now discovers every test, including unfinished owner
contract regressions. A non-green baseline blocks mutation attribution.

## Deploy with the owner's credentials (9 steps)

[CODE READ: 2026-10-22 wrangler.toml, defaults.json, deployment.json] This is the
Worker code checkpoint. The owner deploys; this lane leaves renting disabled.

1. From `ci/vast-broker`, run `npm test` and `npm run bundle`. Compare `sha256sum dist/worker.js` with `deployment.json`; upload those exact bytes.
2. Set public vars: `OWNER_GITHUB_ID=229738317`, `REPOSITORY=reindertpelsma/kayfabe`, `APPROVAL_MODE=owner_push_or_review`, `RENT_DRAFTS=false`, `ALLOWED_BASE_BRANCHES=["master"]`, and initially `RENTING_ENABLED=false`. Retain `PATH_SKIP_PATTERNS` from wrangler.toml and set `DEPLOYED_BUNDLE_SHA256` to the emitted hash.
3. Use the SQLite `BrokerLedger` Durable Object binding named `LEDGER`, initial migration `v1`, and cron `* * * * *` from wrangler.toml. Preserve this namespace across updates.
4. Authenticate Cloudflare on the owner's machine, then upload with `npx --yes wrangler@4.152.0 deploy dist/worker.js --no-bundle`; alternatively use the module-upload/bootstrap recipe below. No Cloudflare credentials belong in GitHub.
5. Set Worker secrets `WORKER_URL` (exact HTTPS origin, no path), `VAST_API_KEY`, `RUN_TOKEN_SECRET`, `OWNER_STOP_TOKEN`. The last two are independent random base64url strings, at least 32 characters. Keep account/signing/owner secrets out of CI and Vast boxes.
6. Optionally set Worker secret `BROKER_PATH_PREFIX` to one random base64url segment, 32–128 characters. GitHub environment `gpu-vast` receives only secret `VAST_BROKER_URL` equal to WORKER_URL and optional secret `VAST_BROKER_PATH` equal to that segment. The prefix is excluded from the OIDC audience. App/PAT credentials are unnecessary until the blob route is added.
7. Keep numeric defaults in defaults.json: check effective cap $1/hour, daily $6, rolling $30, reserve $8; boot 40 minutes, hard 180, soft margin 10, heartbeat 10/15, PASS/FAIL grace 5/10. Sweep is owner-only, $2.50/hour and separate daily $8 with shared rolling/reserve. The explicit reservation includes storage, transfer allowance and a cron minute.
8. The owner may stage this code with renting disabled while the separately authorized local live acceptance runs. Enable `RENTING_ENABLED=true` only when ready to accept bills. Put the protected BASE workflow/controller/manifest on master before requiring `gpu-vast`; hosted OIDC and Cloudflare billed CPU remain unverified. Unauthenticated GitHub read limits can cause a fail-closed refusal; no App/PAT is needed for public repository reads.
9. To stop new rentals set `RENTING_ENABLED=false`; cron and cleanup remain active. Owner-token POST `/stop-all` destroys only ledger-owned instances. Preserve the ledger during key rotation; verify API absence before considering a bill settled.

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

## Historical V4 cost and operational limits (superseded by V5 above)

[INFERRED from source: src/policy.js, arithmetic estimate] The 20-minute hard TTL plus one minute of cron reserves 0.175 USD at 0.50 USD/hour. The workload must fit this shorter allowance; the previous 150-minute default is superseded. Cron/API outages can leave a box billing 12 USD/day or 360 USD/30 days at that rate, plus other fees; there is no proven finite maximum with automatic account funding. Keep modest prepaid credit and disable automatic top-up only after confirming Vast debt rules. Stopped instances still bill storage; an on-box timeout bounds honest work, **not** the upstream bill. Grace/box-kill/stop/provisioning/TTL destroy and ledger accounting are the actual cleanup paths.

[CODE READ: Amendment 3/3b] The box posts progress/log/result JSON to POST /report/<run> with a derived per-run HMAC bearer. It receives no account key, HMAC signing secret or owner-stop secret; it knows its own callback endpoint and run bearer. The Worker bounds raw input to 64 KiB, transport input to 4 MiB and attempts to 2000 per rental, with a 20/10-second rate limit; validates shallow schema/sequence/nonce; neutralises every :: and ##[ delimiter, prefixes all output lines and retains a bounded sanitised tail. Only a structured result decides verdict. Report retries consume caps; identical latest sequences are idempotent. The fixed sender limits ordinary sends to 600 attempts and 900000 logical bytes, reserves final-result allowance, retries the same body with curl only to the Worker origin, and keeps full raw output in /workspace/kf-ci-output.log on the box. Guest root can still lie or connect elsewhere; this script is not an egress sandbox.

[CODE READ: Amendment 3/3b] Callback tokens work only while the ledger is open and Vast confirms the recorded instance running. A positive liveness check is cached for at most three seconds; provider errors close the run. Result revokes reporting immediately but preserves the limited kill capability during inspection grace; stop and TTL revoke all live authority. GET /status/<run> uses same-run OIDC and returns log_tail_sanitised/bytes_dropped. POST /stop/<run> accepts same-run OIDC or the owner bearer; POST /stop-all accepts only OWNER_STOP_TOKEN. Stop revokes before deletion, touches only owned ledger IDs/labels, and is idempotent; cron retries failed cleanup. Vast logs are an optional diagnostic fallback, not implemented or required for verdicts. Merge-group support remains deferred.

[CODE READ: sanitizer and stop implementation] Sanitisation removes command delimiters anywhere in the text because the legacy GitHub command parser scans inside lines. The visible prefix and the runner’s fresh stop-commands frame are additional boundaries. Missing ledger records produce a fixed diagnostic and never imply ownership; they do not block cleanup of other recorded instances. A failed delete keeps the token revoked and the billing reservation open.

[CODE READ: owner endpoint-secrecy request] With BROKER_PATH_PREFIX set, all public routes become /<prefix>/rent, /<prefix>/status/<run>, /<prefix>/report/<run>, /<prefix>/stop/<run> and /<prefix>/stop-all. Missing/wrong prefixes and unknown routes return a generic 404; prefix comparison uses native constant-time HMAC verification. Cron stays private and does not require the public prefix. The onstart callback includes the prefix; the Vast operator can read it. URL secrecy reduces scanning/DoS noise and is not authentication: OIDC plus approval gates protect admission and the run HMAC protects reports. Do not put real origins or prefixes in committed configuration.

[CODE READ: workflow/guard] Endpoint secrets are step-scoped. The broker disables shell tracing, masks canonical origin, host/derived labels, prefix and encoded audience before requests, and suppresses raw transport/parser errors. Known endpoint components are also redacted from stored box text. No endpoint goes into outputs, summaries, artifacts, names or status descriptions. workflow-guard.js rejects secret/alias access elsewhere, unsupported secret-object access/inheritance and scalar forwarding (including aliases/quoted keys), endpoint aliases in metadata, endpoint outputs/printing and unsupported broker run syntax. This static guard supports a bounded workflow subset; workflow protection/review remains necessary against deliberate arbitrary shell obfuscation. The box already knows the endpoint and can disclose it elsewhere, so locator secrecy is never relied on for authorisation.

[INFERRED from source: prefix rotation] Configure matching endpoint secrets before enabling rentals. Rotating the prefix during a run rejects its old callback/status URLs, so drain active work first; preserve the ledger and private cron throughout. A URL/prefix mismatch fails closed with 404 and does not bypass authentication or budgets.


## Historical V4 lifecycle (superseded by V5 above)

[CODE READ: 2026-10-22 V4 addendum 3, src/index.js] PASS, FAIL and UNAVAILABLE structured results retain the box for POST_RESULT_GRACE_MINUTES (default 5, maximum 15), capped by TTL_MINUTES (default 20, maximum 60). First accepted callback must arrive within PROVISION_MINUTES (default and maximum 10). Known pending provider states do not establish liveness: reports receive NOT_READY until actual_status=running; pending state alone does not destroy a boot before provisioning expiry. This is a bounded refusal policy, not evidence that cur_state proves VM readiness.

[CODE READ: 2026-10-22 V4 addendum 3, src/onstart.js] The payload installs `kf-box-kill [reason]` (shell and curl only). It POSTs to /kill/<run> with exactly the report HMAC capability. That capability cannot rent, read status, stop another run or use owner stop-all. A pre-result kill records FAIL/STOPPED. A completed own kill can replay a fixed receipt without authority or further provider calls; reports remain revoked. Kill shares the report rate limiter. Stop/stop-all are immediate; all destruction requires exact owned-ID/label inspection and a subsequent API absence check before ledger settlement. A delete acknowledgement alone keeps liability open for cron retry.

[CODE READ: 2026-10-22 [Cloudflare cron documentation](https://developers.cloudflare.com/workers/configuration/cron-triggers/)] The configured schedule is every minute, with no Durable Object alarms. Healthy-scheduler nominal bounds are provisioning <11 minutes, default hard TTL <21 minutes (maximum <61), and result grace <6 minutes (maximum <16), plus provider request/absence latency. These are not outage guarantees: missed cron/API failures leave liability open. Budget and prepaid checks reserve hard TTL plus the one-minute cron period; failed cleanup keeps accumulating estimated actual cost, without a false TTL cap on the external bill. Only cron enforces unattended destruction; the box timeout limits honest test work and cannot destroy provider resources.

[INFERRED: 2026-10-22 V4 accounting] The live override is reserve 5 USD, hourly cap 0.50 USD, cumulative stop threshold 0.60 USD, absolute authorization 1 USD, one box at a time. The shipped Worker reserve remains 8 USD. At the cap, three full 21-minute reservations total 0.525 USD before transfer charges. Provisioning/grace can shorten actual lifetime; they never extend the hard TTL. The payload prints the early-kill command and configured grace; remaining time is also capped by the recorded hard deadline.

## Public GitHub log (V4 addendum 2, 2026-10-22)

[CODE READ: 2026-10-22 src/ledger.js and gpu-vast.yml] GET /logs/<run>?cursor=N requires same-run GitHub OIDC, including for a cached SHA grant. The cursor is a canonical nonnegative chunk index; four bounded chunks form a page. The Worker persists only sanitized chunks, with total rendered bytes capped by MAX_REPORT_BYTES (default/hard maximum 4 MiB), plus the existing 200-line/8192-byte tail. Status returns log_cursor; polling prints each new page once, then replays the complete retained log in a separately labelled final group. Per-line truncation, redaction and full-cap omissions are counted as dropped bytes; tail omissions have a separate counter. Transport/report caps still apply. Full raw box output remains only on the untrusted box.

[CODE READ: 2026-10-22 src/sanitize.js and public-log.js] Before public output the Worker removes controls/ANSI and command delimiters, known endpoint/binding/capability/nonce/instance ID/label strings, address literals, bearer shapes and opaque base64url-like strings of at least 32 characters. Trusted workflow code opens/closes groups and disables command parsing around live output; the literal renderer neutralizes commands again. The step summary contains allowlisted state/verdict, a nonce-free fixed summary, timings, selected GPU model from the provider offer, estimated cost and tested commit, followed by the sanitized tail in a fence longer than every backtick run. It labels box data UNTRUSTED output of the rented box. Only the structured authenticated result decides verdict. A completed FAIL does not trigger the workflow's error-stop trap and cancel inspection grace. No endpoint, header, HMAC, account ID or SSH metadata enters these public channels.

[CODE READ: 2026-10-22 workflow-guard.js] The guard permits only the exact embedded renderer to write GITHUB_STEP_SUMMARY. General summary/environment sinks, endpoint aliases and arbitrary renderer edits remain rejected. The job executes no proposed checkout. Refusals such as NOT_ELIGIBLE, NEEDS_APPROVAL, RENTING_DISABLED and BUSY receive explicit summary reasons. No artifact is uploaded: the live/final log and summary are the primary view.

[CODE READ: 2026-10-22 GitHub documentation] [Workflow-run information](https://docs.github.com/en/actions/how-tos/monitor-workflows/use-workflow-run-logs) requires GitHub sign-in even for public repositories; [artifact downloads](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts) require sign-in and repository read access. Public repository visibility does not guarantee anonymous Actions UI access. A bounded sanitized rendered excerpt committed under traces is the anonymous git-readable alternative; local synthetic excerpts are labelled as such and do not claim a live rental or hosted Actions run.

[CODE READ: 2026-10-22 src/onstart.js] The sender flushes bounded chunks before replacing its buffer, including fast workloads; its result carries a fixed completion/omission message rather than replaying the already-delivered tail. The 900000-byte sender transport allowance is below the Worker's maximum; the completion message states any sender-side omissions. The box's raw backup is diagnostic only. A structured result still requires the exact nonce.

[CODE READ: 2026-10-22 workerd upstream issue [918](https://github.com/cloudflare/workerd/issues/918)] Returning a response from a Durable Object with an unread upload can produce a native read-after-response exception. The service cancels unlocked unused uploads before returning; this does not establish that the forwarded-pipe runtime warning is fixed. Do not drain an unbounded rejected upload or extend a timer to hide it. Native-runtime warning counts and actual response outcomes belong in the smoke evidence; hosted runtime behavior remains unverified.

[CODE READ: 2026-10-22 smoke.py, native-smoke.mjs] The controller driver defaults to real read-only offer/credit preflight. It refuses to claim a live run when no qualifying offer exists. The earlier no-SSH startup STOP is superseded by round 2; use the separately documented live driver for rentals. `--native-mock --vars .dev.vars --evidence <private-output-directory>` drives the local Worker rent/status/log/report/kill/cron protocol against an explicit fake provider and synthetic clock. It never turns mock success into live evidence. The local adapter and fake GitHub/JWKS entrypoint must never be deployed. Native fixture budgets may be raised to repeat mocked runs; live policy caps follow the explicit owner round limits. No independent live reaper or ingress is armed when no creation will occur.


[CODE READ: 2026-10-22 src/index.js, V4 ingress correction] The public handler buffers at most the route body limit plus one sentinel byte before entering the serialized Durable Object: report 65537 bytes, kill 257, and empty-body routes one byte. The sentinel preserves oversize rejection/accounting in the authenticated handler. Original Content-Length/Transfer-Encoding cannot control the forwarded buffer. An unfinished upload waits outside the ledger, so cron and owner cleanup do not share its wait. No drain of an unbounded upload or new watchdog interval is required. The service also releases unused bounded bodies before responding. The earlier cancellation-only attempt did not fix the network-backed forwarded-stream warning.

[CODE READ: 2026-10-22 src/onstart.js and result acknowledgement] At completion the box prints `kf-box-kill [reason]` beside the remaining minutes supplied by the authenticated Worker acknowledgement, capped by both grace and hard TTL. If the acknowledgement is unavailable it prints unknown; it does not present the configured five minutes as actual remaining lifetime.

[CODE READ: 2026-10-22 post-result kill regression] After the box calls kf-box-kill with a recorded result, status remains COMPLETE with that structured verdict even if CI's first poll arrives after destruction. A kill before any result remains FAIL/STOPPED. Owner stop remains an explicit STOPPED action.

[CODE READ: 2026-10-22 mutation verification] `npm run mutants` executes the 96 legacy cases and the 32 V4 cases sequentially, with named failures required for every patch and restoration in finally. `npm run mutants:v4` runs the focused V4 cases using Python 3's standard library. Preflight anchors with `MUTANT_PREFLIGHT=1 node mutants.js` and `python3 v4-mutants.py --preflight`. The repaired workflow fixture has a positive control so unrelated guard failures cannot mask dependency checks; token and UTF-8 probes exercise boundaries the generic scrubber cannot hide.

[CODE READ: 2026-10-22 log pagination] Log pages may advance past a status snapshot when a report arrives concurrently. The workflow accepts a strictly advancing bounded cursor and prints that page once, then resumes from the newer cursor. A structured result remains independent of the text. The cost estimate is elapsed-rate accounting at the poll, with final grace included only once settled; the ledger reserves the future allowance separately.

## Round 3 runner boundary and release path

[CODE READ: runner.py, release-build.yml] `pull_request_target` selects the master-base workflow; fork heads refuse before any Worker request. The controller and deployment manifest are fetched at that trusted base SHA, with no PR checkout/cache/artifact input, and Python runs with `-I` in a scrubbed environment. Structured schemas reject extra/duplicate keys, invalid UTF-8, out-of-range values and incorrect bindings. Each log chunk is hashed before local filtering; all displayed text uses the runner's allow-list, caps, redactions and fresh stop-commands frame. Only local enums and verified=0/1 enter outputs; status writers are independent. Hosted target-event claims/run membership remain an unverified fail-closed contract until this base workflow is present on master and separately exercised.

[CODE READ: round3-mutants.py, mutants.js, v4-mutants.py] `python3 round3-mutants.py` runs the current hostile-runner/protocol/isolation/release campaign; `npm run mutants` runs legacy, V4 and Round 3 sequentially. Seven legacy inline-client/cache-grant probes and four V4 inline-shell probes were replaced explicitly; the migration map is in the [Round 3 evidence](../../traces/vast_broker_v4_round3_20261018/README.md). Old V4 excerpts and their renderer fixtures are historical and are not the production runner. Local smoke helpers have been migrated to the bound protocol; they were not rerun live for Round 3. Release workflow execution/build/attestation and artifact distribution remain unverified; no binary publication is configured.

## V5 protocol checkpoint and remaining live evidence

[CODE READ: 2026-10-22 src/results.js, src/onstart.js, runner.py] Protocol 4 / v5
requires a bounded payload result file with the exact approved SHA and run nonce,
then adds the runner nonce in the fixed sender. Exit zero or log text alone is
FAIL. PASS requires a nonempty table of uniquely named, bounded successful suites
with zero failures/skips. The approved repo-side payload defines its required suites
and counts; these are deliberately absent from the Worker transport. Both Worker and BASE runner validate the
suite/hardware structures. The runner independently validates availability,
prints its numeric thresholds/rejections, and uses only its own preference list.
Its check job/step horizons now cover the explicit default three-hour lifecycle;
the separate sweep workflow is dispatch-only/OIDC-only and writes no merge status.

[CODE READ: 2026-10-22 src/index.js] Callback, hold and owner timer-removal paths
compare stored absolute deadlines before accepting a late request, so overdue
cron cannot let a post revive an expired boot, heartbeat, grace or manual ceiling.
MANUAL after a result remains terminal for CI and retains the box for the owner.

[MEASURED: 2026-10-22 protocol_tests.txt and v5_mutants.json in traces/vast_broker_v5_20261018]
The local baseline passes 964 tests; 69 focused V5 mutation cases each have at
least two named killers. Migrated V4 and runner campaigns pass 28 and 38 cases,
respectively. The legacy campaign's 94 anchors pass preflight; its full rerun is
still pending at this checkpoint. Historical model-query, depth, lifecycle,
ingress and workflow mutation sites were moved to their equivalent V5 guards;
no mutation threshold or debt baseline was relaxed.

[CODE READ: 2026-10-22 src/github.js, src/index.js and [GitHub API limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)]
CI supplies only OIDC. The Worker reads public GitHub metadata without an App/PAT;
shared unauthenticated rate limits can refuse a long run. GitHub authentication
for higher read limits can follow separately and is not a blob-route prerequisite.
Hosted OIDC and Cloudflare billed CPU remain unverified. The owner has replaced the
next acceptance with short non-VM dummy FAIL/PASS runs; the real KVM/full-suite
payload and SSH MANUAL polish are deferred. No deployment is performed here.
