# Public platform API (`api.kayfabe.dev`)

**STATUS: DESIGN-ONLY, 2026-11-07.** Code and tests exist; nothing is deployed. 2026-11-07: run reports,
host-session subruns with similarity ranking, `/v1/ci/subruns`, `/v1/ci/stats`, the published full log's
identity and migration 0004 added (section "Run reports, subruns and stats"). 2026-11-06: the setup
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
| `migrations/0004_ci_run_reports.sql` | `ci_run_reports` (the sanitised report and the log's size and SHA-256), `ci_subruns`, `ci_run_tests`, `ci_run_perf`, `ci_run_apps`, keyed by (run, subrun); adds tables and indexes only |
| `src/subruns.js`, `src/rank.js`, `src/dies.js` | `/v1/ci/subruns` and `/v1/ci/stats`, the similarity ranking (pure functions), and the die order table of each family |
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
| `/v1/ci/runs/<id>` | none | one run: stages, GPU name, the host driver and each guest's OS version and driver (claims by the test box; never the provider's offer label), log key (immutable once final) |
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
  each OS. A PASS is a PASS of both. A FAIL is charged to an OS unless every failed stage is known to
  belong to the other OS: stage names starting with `windows`, and `specviewperf`, are Windows'; `kvm_tests`,
  `gates`, `kf3_build`, `raw_suite`, `thin_suite`, `birth_census` and `cuda_fat` are Linux'. Any other
  failed stage (`timeout`, `app_exit`, `incomplete`, `provision`, the reporter's `result_*` and `payload_fail`
  rows) counts against BOTH OSes, so a failure is never hidden by a guess. The spared OS gets
  `UNATTRIBUTED`, which is neither a pass nor a fail of that setup (`latest.verdict` still says `FAIL`).
  A FAIL with no stage rows is charged to both. `RETRY` and `UNATTRIBUTED` runs never pass. A run in
  flight is not counted.
- `cpu_vendor`, `host_driver` and `guest_driver` are claims: the box read them from its own `/proc`
  and `nvidia-smi` and the CI Worker checked only their shape (`ci_run_claims.claimed = 1`; the
  response lists them under `claimed`). They group and filter rows and never decide a verdict, a die or
  a requirement. The host driver is the NVIDIA driver on the machine that runs kayfabe (the box's
  `nvidia-smi`, never the provider's offer label); the guest driver is the driver inside the guest. The
  Windows guest's OS version and driver are the test harness's configured values (the gate checks the
  driver, but the box sends the constant even when the stage fails first), not values read from the
  guest; the Linux guest driver is read from the receipt of the guest's provisioning.
- `source`: `reproduced` rows are CI Worker runs (`ci_runs.source = 'reproduced'`). `community` is the
  reserved `reported` (report tier `unverified`) and `reviewed` rows of a later, separate store; none
  are written today, the filter is accepted and returns nothing. `imported` is the evidence file
  `site/status/coverage.json`, which is not a database row: asking for it here returns nothing. Rows
  of different sources are never merged, and every row carries its `source`.
- Which dies exist for a marketing name is not this API's business: the site's
  `site/status/gpu_names.json` maps names to dies and the page asks per die.

Tests: `test/setups.test.js` (hand-written rows on the read-only database), the statement coverage in
`test/api.test.js`, and the `setups` and `facets` mutants in `scripts/mutate.mjs`.

## Run reports, subruns and stats (2026-11-07)

CODE READ (`src/subruns.js`, `src/rank.js`, `src/dies.js`, `src/queries.js`, migration 0004); GPU-free tests
only (`test/rank.test.js`, `test/subruns.test.js`, `test/reports.test.js`, the mutants of `scripts/mutate.mjs`);
nothing deployed, no real D1. Written by the CI Worker from the box's structured report
([box protocol](../vast-broker/docs/BOX_PROTOCOL.md), "Optional run report"). Every value from the box is a
CLAIM: the CI Worker bounded it on the way in, this Worker bounds it again on the way out (printable ASCII, at most
200 characters, nothing token-shaped, no SPECviewperf), and no row says more than the box said. Same rules as the
rest of the API: GET only, fixed statements, CORS `*`, 405 elsewhere.

**A host session** is one rental (`ci_runs.run_id`): die, GPU, host driver, CPU facts, commit and tree, tier,
times, the Worker's verdict, the full log. It holds **subruns**: `host` (bare-metal arms and the host CUDA
ladder), `linux-<guest driver>` (thin suite, guest CUDA ladder), `windows-<driver>` (Windows gate). A subrun has a
kind (`host_bare`, `linux_guest`, `windows_guest`), its OS, drivers, a claimed verdict (`PASS`, `FAIL`, `RETRY`,
`UNATTRIBUTED`, or `NOT_RUN` with a reason), counts, a duration (the sum of its timed tests) and the
byte range of its part of the published log. A future small tier adds several `windows-<driver>` subruns to one
session.

| Path | Query parameters | Returns |
|---|---|---|
| `/v1/ci/runs/<id>` | none | as before, plus `subruns` (all of them), `report` (the stored report, `null` when the box sent none) and `log` (`bytes`, `sha256`, and `url` when `LOGS_BASE` is set) |
| `/v1/ci/subruns` | `commit`, `tree`, `die`, `os` (`linux`, `windows`, `host`), `host_driver`, `guest_driver`, `cpu` (`amd`, `intel`, `arm`, `other`, optionally `-x86_64` or `-aarch64`), `relax`, `require`, `ignore`, `limit`, `cursor`, `sort`, `include`, `min_closeness` | ranked subruns, each with `match` and a link to its session |
| `/v1/ci/stats` | `die`, `tier`, `driver` (host driver), `os`, `limit`, `sort` | the latest reported final session per die and host driver: subrun pass counts and the perf rows the box recorded |
| `/v1/logs/<run>/log.txt`, `/v1/logs/<run>/report.json` | none | 302 to `LOGS_BASE/<run>/<name>` for a final run that has the object; 404 otherwise (and the legacy `runs` table still answers its own names) |

**Similarity ranking** (`/v1/ci/subruns`). Name what you want; when no subrun matches exactly, the nearest are
returned and every difference is stated. Seven dimensions, in the default ladder (the most acceptable thing to
relax first): `commit` (same tree, then the same 24 hour window, then older or newer by distance), `cpu` (same
vendor and architecture, then the same architecture, then other or unknown), `host_driver` (same, same branch
i.e. same major number, numeric distance), `guest_driver` (same), `die` (same, then the nearest die of the same
family by places in the family's chip order, `src/dies.js`), `os`, `family` (a die of another family; this also
relaxes `die`, so it is never "close"). Results are sorted by the number of relaxed dimensions, then by the
ladder position of the most-relaxed one, then by the per-dimension closeness (level, then distance, in ladder
order), then by recency (`ended_at`), then run id and subrun id: a total order, so a call is repeatable and a
cursor is stable. Each hit says what it matched and what it did not:
`match: {exact: ["die", "os"], relaxed: [{dim: "host_driver", wanted: "580.95.05", found: "575.64.05"}]}` (a
`note` such as `older`, `newer`, `same_day_older` or `reference_unknown` explains a commit step). Candidates
are chosen by at most nine fixed statements of at most 60 rows (every wanted dimension, each wanted dimension
left out, and the newest rows) and ranked in JavaScript; a subrun that did not run (`NOT_RUN`) or belongs to a run
still in flight is never a candidate.

| Parameter | Meaning |
|---|---|
| `relax=a,b,c` | the ordered list of dimensions that may be relaxed (default: all seven in ladder order); a dimension that is not listed must match exactly; `relax=none` relaxes nothing |
| `require=a,b` | dimensions that must match exactly (never relaxed, filtered in SQL; each needs its wanted value, else 400); wins over `relax` |
| `ignore=a,b` | dimensions dropped from matching entirely (neither exact nor relaxed); cannot also be in `require` |
| `min_closeness=n` | at most n relaxed dimensions per hit (0 = exact matches only; default 7) |
| `limit=n`, `cursor` | page size 1-100 (default 20); `next_cursor` resumes the same request (a cursor of another request is a 400) |
| `sort` | `closeness` (default), `recent` (newest first, then closeness), `duration` (shortest summed test time first, none last) |
| `include=a,b` | expand each hit: `counts`, `stages` (the session's stage rows of this subrun's suites), `tests`, `perf`, `apps`; the per-subrun row cap shrinks as the page grows (tests 100, perf and apps 50 at most, about 1000 and 500 rows in all) and `truncated: true` says when rows were cut |

The response states `effective: {relax, require, ignore, limit, sort, include, min_closeness}` so a page can show
and edit them. An unknown parameter is `400 {"detail": "unknown parameter", "allowed": [...]}`; an unknown or
repeated dimension or `include` word, a list longer than the vocabulary, a `limit` above 100, a malformed or
foreign cursor, `require` with no wanted value, or a dimension in both `require` and `ignore` is a `400` that
lists the allowed words and echoes nothing of the request. The dimension names are `commit`, `cpu`, `host_driver`,
`guest_driver`, `die`, `os`, `family`; the owner's `host` (session grouping) is not a dimension: a hit names its
session in `session` and `run`.

`/v1/ci/stats` has the same shape of answer (`effective`, `groups`), the filters above, `limit` 1-100 (default
20) and `sort` `recent` (default), `die` or `driver`. Each group: die, family, GPU name, host driver, tier, the
run, commit, `ended_at`, the Worker's verdict, summed and per-subrun counts, and at most 50 perf rows
(`truncated` says when more exist). The perf rows are numbers the box recorded (cup8bench GFLOPS and timings;
a guest row carries the host row of the same benchmark as `ref_value` and `ref_ratio`); they never gate
anything, and no SPECviewperf score is ever in them. Where the box recorded nothing (an older box, a failed
stage, the application matrix, which is not a payload stage yet) there is no row: the answer is empty, never
invented.

A database without migration 0004: the run page still answers (no `report`, no `subruns`); `/v1/ci/subruns` and
`/v1/ci/stats` answer a fixed 500 until it is applied.

The development fixture (`site/status/fixture.json`, SYNTHETIC) has reports for the picker runs and the newest
regular runs only (the rest has none, as the live site will until the first report arrives), one session with
three Windows drivers and two Linux drivers (`syn-ci-p-8`), a FAIL that stops the payload (later subruns
`NOT_RUN`), and these request paths: `/v1/ci/stats` (with each tier, each OS, each die that has a report),
`/v1/ci/subruns` (plain; with `include=counts,perf&limit=5`; per die and OS with `include=counts`; and
the ranking examples built on `?die=GA106&os=windows&host_driver=580.95.05&guest_driver=580.88&cpu=amd` with
`min_closeness=0`, `require=die,os`, a custom `relax`, `sort=recent&include=stages,tests,perf`).

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
