# Vast GPU CI broker

STATUS: LIVE, 2026-11-05, for the deployed rental path. The result store, tree key and
requirements check (section below) are built and tested GPU-free only; NOT DEPLOYED.

The broker connects GitHub Actions to rented NVIDIA GPU machines on Vast.ai so kayfabe can run GPU checks beyond the hardware available to hosted runners. It is a Cloudflare Worker backed by a Durable Object ledger that tracks rentals, deadlines and spending. GitHub asks for a check; the Worker handles the provider account and relays the box's result and bounded logs back to the runner.

## How a GPU check uses it

1. The [GPU workflow](../../.github/workflows/gpu-vast.yml) decides whether a master push or PR needs a check. It fetches the controller from the trusted master base, without checking out proposed code on the GitHub runner. Two independent path decisions must agree before it reports a check as not needed.
2. The controller presents GitHub OIDC identity. The Worker checks the pinned workflow, the exact run attempt's owner triggering actor and the exact tested commit, then applies offer eligibility, concurrency and spending policy before creating a rental.
3. The box bootstrap checks out that exact commit and runs the [repository-side payload and reporter](../../scripts/ci/README.md). Its scoped run token lets it report progress, logs and a structured result, request its own destruction, and obtain permitted private asset download URLs.
4. The GitHub controller independently validates the result bindings and log hashes, scrubs displayed output and supplies the workflow's `gpu-vast` status. After fetching the final log it requests immediate destruction; result grace and Worker deadlines provide fallback cleanup.

## Trust and spending boundaries

The Vast box and its operator are untrusted. The Vast account key stays in the Worker, outside GitHub secrets and rented machines. A box receives only its run capability and, when configured, a Cloudflare Access service-token pair. GitHub communicates through the broker and accepts no executable or build artifact from the box. A GPU PASS is an unattested tripwire: a dishonest box or Worker can forge a verdict. It supplies no release provenance. See the [trust and release trace](../../docs/design/V3_CI_VAST_RUNNER_TRACE_20261022.md).

The ledger reserves liability before rental and applies daily and rolling budgets, a prepaid credit reserve and limits on concurrent work. The default check policy is $1 effective/hour, $6/day, $30 per rolling 30 days and an $8 credit reserve, with one active rental and a 300-minute hard deadline for the check profile, which runs the Windows stage of the master gate ([howto](../../docs/howto/gpu-vast-windows.md)); the sweep keeps three hours. The separate owner-dispatched sweep profile has its own hourly and daily caps. Transfer allowances are reservations, not provider-enforced usage limits. Missed cron runs or provider failures can leave a box billing beyond its deadline; these controls do not guarantee a hard external bill ceiling.

## Current setup and further reading

[CODE READ: wrangler.toml, gpu-vast.yml, gpu-vast-sweep.yml] The public broker origin is
`https://ci.kayfabe.dev`; Worker var `WORKER_URL` and GitHub environment `gpu-vast` variable
`VAST_BROKER_URL` must match because the origin is the OIDC audience. The path and both Access
credentials stay secrets. Protect the whole `ci.kayfabe.dev` hostname with Access and the same
runner/box service tokens; keep workers.dev as a fallback only if it also stays behind Access.
The admin UI uses `https://admin.kayfabe.dev` and its separate owner-identity Access app. Before
the first var deploy the coordinator deletes the old `WORKER_URL` secret, following [DEPLOY](DEPLOY.md).

Renting is enabled by the deployed vars file, which is kept outside the repository ([deployment guide](DEPLOY.md)); `RENTING_ENABLED=false` in the committed [Worker configuration](wrangler.toml) is only the safe default. `RUN_TOKEN_SECRET` signs run capabilities and `OWNER_STOP_TOKEN` controls emergency cleanup. As of 2026-10-25 acceptance of the current hosted workflow with the protocol-7 payload is still open; historical local smoke results do not establish it.

- [Threat model](docs/THREAT_MODEL.md): who is trusted, and which behaviours are intended design rather than vulnerabilities.
- [Box protocol](docs/BOX_PROTOCOL.md): bootstrap environment, callbacks and result contract.
- [Placement decision](docs/PLACEMENT.md): US/Canada offer preference and the proposed ledger location.
- [Evidence summary](../../traces/vast_broker_summary/README.md): recorded runs, retained evidence and its limitations.
- [Internal implementation and evidence notes](docs/IMPLEMENTATION_NOTES.md): the former README's dated rounds, corrections and superseded behavior.

After deploying the matching Worker bundle, use a fresh GPU-relevant commit to start validation; reruns retain the original trusted BASE pin.

CODE READ (`src/provider-fault.js`, `src/ledger.js`, `runner.py`): provider failures stop and destroy the box before one replacement on a different known host. Two failed machines condemn their shared host; static seed `224600` comes from runs 40425285335 and 40478005010. An exhausted replacement stays a non-passing infrastructure error, with no kayfabe verdict. See [provider-fault handling](../../docs/howto/vast-provider-faults.md).

CODE READ 2026-10-27 (`src/github.js`): attempt3 and later have no special authorization ceiling. Admission reads the signed attempt's REST resource and retains strict actor, repository, status and SHA bindings. For a 401, use the fixed field-only observability code; see [rerun authorization](../../docs/howto/vast-rerun-auth.md).

CODE READ 2026-10-27 (`gate_reuse.py`): proposed master-push reuse preserves the existing GPU job with readable original result logs and summary. It requires identical trees and complete GitHub status/CI/run evidence; uncertainty uses the existing gate. See [reuse behavior](../../docs/howto/gpu-gate-reuse.md).

## Result store, tree key and requirements check (built 2026-11-05, NOT DEPLOYED)

Design: [V3_PLATFORM_TESTING.md](../../docs/design/V3_PLATFORM_TESTING.md) §4/§8 and
[V3_CI_TIERS_AND_IGNORE_LADDER.md](../../docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md) §1/§3.
Deployment steps: [DEPLOY](DEPLOY.md), "Result store (D1)". No gate reads the store yet: the
`gpu-vast` check still waits for its own rental, and nothing here starts a rental.

- **Store** ([`src/results.js`](src/results.js), tables of
  [`0002_ci_results.sql`](../platform-api/migrations/0002_ci_results.sql)). `recordRun(env, row)`
  is idempotent on `run_id`: an in-flight row (no verdict) is finalised once, a repeated final
  row is `DUPLICATE`, a different final row is `CONFLICT` and changes nothing. Every field is
  checked against a fixed pattern (no box text is stored), a token-shaped value refuses the row,
  and each final row carries an HMAC tag over its fields and stages with the Worker secret
  `ROW_TAG_KEY` (not a run token). `requirementsMet(env, {commit_sha, tree_hash, tier,
  requirements})` reads the rows of one result key, ignores rows whose tag does not verify, and
  answers `{met, missing, inflight}`.
- **Ingest** ([`src/ingest.js`](src/ingest.js)). The ledger's verdict is not changed. A queue entry
  `store:<run>-<attempt>:<leg>` is written when a rental is reserved; the cron sweep records an
  in-flight row (so a caller can join it) and, after settlement, the final row, as run id
  `vast-<run>-<attempt>-<leg>`. Store verdicts: PASS only for a verified box PASS; FAIL for a box
  FAIL or a box that reported and then hit a test-side limit (hard or heartbeat timeout, report
  limits); RETRY for a provider fault; UNATTRIBUTED for everything else (no box report, owner
  stop, supersede, provider absent). RETRY and UNATTRIBUTED never meet a requirement; a FAIL is
  an answer, so the same tree is not retried automatically. Profile `check` is stored as tier
  `middle` (platform `both`), `sweep` as `adhoc` (`linux`).
- **Tree key** ([`src/treekey.js`](src/treekey.js)). SHA-256 over `kf-tree-v1` and, for every
  file not ignored by the commit's own `.workerignore`, `path NUL mode NUL object-id LF` in UTF-8
  byte order of the path. Without a `.workerignore` the built-in default `/archive/`, `/docs/`,
  `/site/` applies (INFERRED from a 2026-11-05 grep: no build or box script reads them; `traces/`
  stays because `kf-abi` includes a TSV from it). Fetched only by this Worker, through the GitHub
  git data API: the commit, its root tree, the `.workerignore` blob and one recursive listing per
  top-level directory that is not wholly ignored. A truncated listing, a GitHub error, a symlinked
  `.workerignore` or a pattern outside the subset means no tree key: the run is stored under a
  per-commit key that nothing else shares (fail closed).
- **`.workerignore` subset** (read identically by git; `test/treekey-parity.test.js` compares the
  matcher with `git check-ignore --no-index` on every path of this repository through
  [`treekey_ref.py`](treekey_ref.py)): one pattern per line, `#` comments; characters
  `A-Z a-z 0-9 . _ - + @ / * ?` only (no `!`, `\`, `[...]`, spaces); a trailing `/` matches
  directories (and submodules) only; a `/` at the start or in the middle anchors the pattern at the
  root, otherwise it matches a name at any depth (`docs/` ignores every directory called docs,
  `/docs/` only the top-level one); `*` and `?` stay within one path segment; `**` only as a whole
  segment of an anchored pattern (`**/x`, `a/**/b`, `a/**`); a path is ignored when it or any
  parent directory matches. Docker reads unanchored patterns differently; the ladder lint of the
  design is not built.
- **Limitation (INFERRED).** A key computed from a filtered tree describes what the box tested
  only once the box deletes ignored files after cloning (design §4, the strip step), which is not
  built. Until then a change to an ignored file can matter to a run without changing its key.

### Requirements file

[`ci/requirements.toml`](../requirements.toml) is read from the BASE branch (`master` or `next`,
named by the caller; never from the PR) by [`src/requirements.js`](src/requirements.js). It is a
strict TOML subset (`test/requirements.test.js` also parses it with Python's `tomllib`):
`version = 1`, then one `[[require]]` block per rule with `tier` (`small`, `middle`, `large`),
`family` (a family id or `"any"`), `dies` (die ids such as `"GA102"` or `"CC_89"`, or `["any"]`),
optional `min_dies` (with `["any"]`: distinct dies that must pass, default 1), `platforms`
(`"linux"`, `"windows"`; a `both` run covers each), optional `stages` (suite rows that must be
PASS in the answering run) and optional `min_passes` (passing runs per die and platform, default
1). Strings are `"..."` without escapes; any other key, value or table is refused. The newest
verified PASS or FAIL of a die and platform answers; a run of a larger tier also answers a smaller
tier (`large` > `middle` > `small`; `adhoc` answers none). A tier without blocks, a missing file
or a refused file is never met.

### `POST /v1/requirements/check`

Authenticated exactly like `/availability` (GitHub OIDC of the pinned master workflow, the
`X-KF-Protocol` header, an owner-triggered in-progress run); counted in the same admission rate
limit. Body: `{"commit_sha": "<40 hex>", "base": "master"|"next", "tier": "small"|"middle"|"large"}`
(no other keys). Answer: `{met, missing: [{family, die, platform, reason, need?, have?}], inflight:
[run_id], tier, tree_hash, tree_kind, requirements_sha256}`. Reasons: `NO_RESULT`, `FAIL`,
`STAGE`, `TOO_FEW_PASSES`, `INFLIGHT`, `NO_REQUIREMENTS`, `REQUIREMENTS_INVALID`. It never
reserves, rents or queues; its only write is the content-addressed copy of the requirements text
(`ci_requirement_snapshots`). Without the D1 binding or `ROW_TAG_KEY` it answers 503
`STORE_UNAVAILABLE`. Forks cannot reach it (the OIDC identity is the master workflow's).

### Tests

GPU-free, from this directory (Node 22.13+ for `node:sqlite`; the D1 tests skip on older Node):
`node --test test/treekey.test.js test/treekey-parity.test.js test/requirements.test.js
test/results-store.test.js test/store-ingest.test.js`, and `node store-mutants.mjs` (every mutant
must be killed). The D1 tests run against the real migrations on `node:sqlite` with a D1-shaped
fake (`test/d1.js`); real D1, the GitHub trees API and Cloudflare are not exercised.

