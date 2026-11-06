# Public platform API (`api.kayfabe.dev`)

**STATUS: DESIGN-ONLY, 2026-11-06.** Code and tests exist; nothing is deployed. 2026-11-06: the setup
picker's endpoints `GET /v1/ci/setups` and `GET /v1/ci/facets` and migration 0003 (host facts the
test box claims) added. 2026-11-05: read endpoints for the CI Worker's result store (`/v1/ci/...`,
migration 0002) added. Design and owner
rulings: [V3_PLATFORM_TESTING.md](../../docs/design/V3_PLATFORM_TESTING.md). All data in this
directory's seed and in `site/status/fixture.json` is **synthetic**.

A Cloudflare Worker that answers GET requests from one D1 database (`kayfabe-platform`). It has no
secrets, no authentication and makes no outbound request; the CI Worker is the database's only
writer. Every SQL statement is a constant in [`src/queries.js`](src/queries.js); the database
wrapper takes statement names, not SQL.

| Path | Purpose |
|---|---|
| `src/` | the Worker: routing and handlers (`api.js`), validation (`validate.js`), statements (`queries.js`), D1 wrapper (`db.js`), `/v1/schema` (`schema.js`) |
| `migrations/0001_init.sql` | D1 schema and indexes |
| `migrations/0002_ci_results.sql` | the CI Worker's result store: `ci_runs`, `ci_run_stages`, `ci_host_facts` (public), `ci_runs_private`, `ci_requirement_snapshots` (never selected here); adds only, 0001 is unchanged |
| `shared/row-tag.js` | HMAC row-tag reference for the CI Worker; not bundled by this Worker |
| `migrations/0003_ci_run_claims.sql` | `ci_run_claims`: the CPU vendor, CPU model, CPU architecture, guest OS version and the host and guest NVIDIA driver a test box reported, one row per guest OS of a run, `claimed = 1`; adds a table and two indexes only |
| `scripts/seed-data.mjs`, `scripts/seed.mjs` | synthetic rows; `seed.mjs` prints them as SQL |
| `scripts/make-fixture.mjs` | writes `site/status/fixture.json` through the real handler |
| `scripts/mutate.mjs` | mutants for read-only, fixed SQL, validation and leak rules |

## CI Worker result endpoints (2026-11-05)

Rows written by the CI Worker (`ci/vast-broker/src/results.js`; one row per rental leg, keyed by
the tree hash of the commit after its `.workerignore`). Same rules as the rest of the API: GET
only, fixed statements, CORS `*`, 405 elsewhere; never the row tag, machine id, price, cost or
requirement snapshots; no SPECviewperf score exists in these tables. The status pages (`site/status`)
read them (2026-11-05); `scripts/make-fixture.mjs` also records them in the synthetic fixture.

| Path | Query parameters | Returns |
|---|---|---|
| `/v1/ci/matrix` | `tier`, `tree` (64 hex), `family` | per tier, family, die and platform: the newest run and the newest PASS |
| `/v1/ci/trees` | `tier`, `limit` | result keys, newest first, with run, PASS, FAIL and running counts |
| `/v1/ci/trees/<hash>` | none | one result key: the commits recorded under it and its runs |
| `/v1/ci/runs/<id>` | none | one run: stages, GPU name, driver versions, log key (immutable once final) |
| `/v1/ci/commits/<sha>` | none | a commit's result keys and every run under them; `other_commit: true` marks runs of other commits with the same tree hash, shown for information: they count for this commit only when the CI Worker's `TREE_KEY_CARRY_OVER` is on (off by default) and `tree.kind` is `tree` |

## Setup picker endpoints (2026-11-06)

For the status page "Find my setup" (`site/status/find`): a visitor names a guest OS, a GPU and a CPU
vendor and sees the runs across driver versions. Same rules as everything here: GET only, fixed
statements, CORS `*`, `405` elsewhere, every parameter an anchored pattern, unknown or repeated
parameters `400`. Neither response contains the row tag, machine id, price, cost, CPU model, a
requirement snapshot or any SPECviewperf datum.

| Path | Query parameters | Returns |
|---|---|---|
| `/v1/ci/setups` | `os` (`linux`, `windows`), `gpu` (one to four die ids, comma separated, for example `GA106,GA104`), `cpu` (`amd`, `intel`, `arm`, `other`), `driver` (matches the host or the guest driver), `tier`, `source` (comma list of `reproduced`, `imported`, `community`; default `reproduced`), `report_tier` (`unverified`, `reviewed`), `limit` (1-100) | one row per (guest OS, die, source, CPU vendor, host driver, guest driver) over final runs |
| `/v1/ci/facets` | none | the values that exist: guest OSes, dies with the GPU names boxes reported, CPU vendors and architectures, host and guest drivers, tiers, `sources` and `report_tiers` |

A row: `os`, `die`, `family`, `source`, `report_tier`, `cpu_vendor`, `host_driver`, `guest_driver`
(`null` = not reported), `gpu_name` (the latest run's), `runs`, `passed`, `failed`, `other`,
`last_at`, `latest` (`id`, `result`, `verdict`, `tier`, `commit`, `tree`, `ended_at`) and `last_pass`
(`id`, `commit`, `ended_at`, or `null`). Example (a SYNTHETIC row of the development fixture, not a run):

```json
{"os":"windows","die":"GA106","family":"ampere","source":"reproduced","report_tier":null,
 "cpu_vendor":"intel","host_driver":"575.64.05","guest_driver":"580.88","gpu_name":"synthetic GA106",
 "runs":1,"passed":0,"failed":0,"other":1,"last_at":"2026-10-06T04:13:00Z",
 "latest":{"id":"syn-ci-p-4","result":"UNATTRIBUTED","verdict":"FAIL","tier":"middle",
 "commit":"158cb913465a4cffc63beaa7faf74bceb87a825f","tree":"b8ba81bdb9253f92e4e851c46d94288f2fef622326c185ed103bcbbd91860049","ended_at":"2026-10-06T04:13:00Z"},"last_pass":null}
```

What the numbers mean:

- A run over both guest OSes (`ci_runs.platform = 'both'`: one rental, one verdict) counts once under
  each OS. A PASS is a PASS of both. A FAIL is charged to the OS whose stage failed (stage names that
  start with `windows` belong to Windows); the other OS gets `UNATTRIBUTED`, which is neither a pass
  nor a fail of that setup (`latest.verdict` still says `FAIL`). A FAIL with no stage rows is charged
  to both. `RETRY` and `UNATTRIBUTED` runs never pass. A run in flight is not counted.
- `cpu_vendor`, `host_driver` and `guest_driver` are claims: the box read them from its own `/proc`
  and `nvidia-smi` and the CI Worker checked only their shape (`ci_run_claims.claimed = 1`; the
  response lists them under `claimed`). They group and filter rows and never decide a verdict, a die or
  a requirement. The host driver is the NVIDIA driver on the machine that runs kayfabe (the box's
  `nvidia-smi`, never the provider's offer label); the guest driver is the driver inside the guest.
- `source`: `reproduced` rows are CI Worker runs (`ci_runs.source = 'reproduced'`). `community` is the
  reserved `reported` (report tier `unverified`) and `reviewed` rows of a later, separate store; none
  are written today, the filter is accepted and returns nothing. `imported` is the evidence file
  `site/status/coverage.json`, which is not a database row: asking for it here returns nothing. Rows
  of different sources are never merged, and every row carries its `source`.
- Which dies exist for a marketing name is not this API's business: the site's
  `site/status/gpu_names.json` maps names to dies and the page asks per die.

Tests: `test/setups.test.js` (hand-written rows on the read-only database), the statement coverage in
`test/api.test.js`, and the `setups` and `facets` mutants in `scripts/mutate.mjs`.

## Tests (GPU-free, no network, no install)

Node 22.13 or newer (for `node:sqlite`). From this directory:

```sh
npm test                    # node --test test/*.test.js
npm run mutate              # every mutant must be killed
npm run fixture -- --write  # after changing the seed or the response shapes
```

The tests open the seeded SQLite file read-only, so a write statement fails in the engine.

## Local development against wrangler (optional; needs `npx wrangler`)

```sh
node scripts/seed.mjs > /tmp/kf-seed.sql
npx wrangler d1 migrations apply kayfabe-platform --local   # 0001, 0002 and 0003
npx wrangler d1 execute kayfabe-platform --local --file /tmp/kf-seed.sql
npx wrangler dev
```

Never apply the synthetic seed to the remote database.

## Deployment (coordinator only)

See the design document, section 12. The Worker needs the D1 binding `DB`, the custom domain
`api.kayfabe.dev`, and optionally the var `LOGS_BASE` (https origin on kayfabe.dev). Migrations are
applied from this directory only (`npx wrangler d1 migrations apply kayfabe-platform --remote`);
the order with the CI Worker's deployment is in `ci/vast-broker/DEPLOY.md`, "Result store (D1)".
