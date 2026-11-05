# Public platform API (`api.kayfabe.dev`)

**STATUS: DESIGN-ONLY, 2026-11-05.** Code and tests exist; nothing is deployed. 2026-11-05: read
endpoints for the CI Worker's result store (`/v1/ci/...`, migration 0002) added. Design and owner
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
npx wrangler d1 migrations apply kayfabe-platform --local   # 0001 and 0002
npx wrangler d1 execute kayfabe-platform --local --file /tmp/kf-seed.sql
npx wrangler dev
```

Never apply the synthetic seed to the remote database.

## Deployment (coordinator only)

See the design document, section 12. The Worker needs the D1 binding `DB`, the custom domain
`api.kayfabe.dev`, and optionally the var `LOGS_BASE` (https origin on kayfabe.dev). Migrations are
applied from this directory only (`npx wrangler d1 migrations apply kayfabe-platform --remote`);
the order with the CI Worker's deployment is in `ci/vast-broker/DEPLOY.md`, "Result store (D1)".
