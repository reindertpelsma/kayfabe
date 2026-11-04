# Public platform API (`api.kayfabe.dev`)

**STATUS: DESIGN-ONLY, 2026-11-04.** Code and tests exist; nothing is deployed. Design and owner
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
| `shared/row-tag.js` | HMAC row-tag reference for the CI Worker; not bundled by this Worker |
| `scripts/seed-data.mjs`, `scripts/seed.mjs` | synthetic rows; `seed.mjs` prints them as SQL |
| `scripts/make-fixture.mjs` | writes `site/status/fixture.json` through the real handler |
| `scripts/mutate.mjs` | mutants for read-only, fixed SQL, validation and leak rules |

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
npx wrangler d1 migrations apply kayfabe-platform --local
npx wrangler d1 execute kayfabe-platform --local --file /tmp/kf-seed.sql
npx wrangler dev
```

Never apply the synthetic seed to the remote database.

## Deployment (coordinator only)

See the design document, section 12. The Worker needs the D1 binding `DB`, the custom domain
`api.kayfabe.dev`, and optionally the var `LOGS_BASE` (https origin on kayfabe.dev).
