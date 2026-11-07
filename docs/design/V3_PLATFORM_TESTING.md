# Platform testing: a coverage service behind CI, and a public status site

**STATUS: DESIGN-ONLY, 2026-11-06.** Owner-approved design of 2026-11-04 (lane 1 brief), recorded
here, corrected on 2026-11-05 by the owner's later decisions (box below); nothing in this document is deployed. Branch `claude/platform-status-20261104` builds the
public read-only API (`ci/platform-api/`), its D1 schema and the static status pages
(`site/status/`) against **synthetic** fixture data only. The CI Worker side (ingest, lazy
satisfaction, HMAC tags, redaction, HostHub) is design only. Nothing here has been measured: there
is no run, revision or date behind any number below except where a source is cited.

**Correction, 2026-11-05 (folded above the text it corrects).** The owner's decisions of 2026-11-04/05
are in [V3_CI_TIERS_AND_IGNORE_LADDER.md](V3_CI_TIERS_AND_IGNORE_LADDER.md) (DESIGN-ONLY, nothing
built) and they change three things below. (1) The fall-through key is the **tree hash** of the files
left after the commit's own `.workerignore`, not a hash over a BASE-branch `ci/test_boundary.toml`
(§1, §4, §11, §13). (2) The tier table gains an admin start route, a die rotation in the middle tier,
a small tier that rides on the middle box, no retries, no scheduled runs, and the event-to-tier rule
(§2). (3) The tiers' stages gain the two-guest and TRIO stages (that document, §5). Where a section
below still shows the older text, it is marked SUPERSEDED in place.

**Addition, 2026-11-06 (owner request): the setup picker.** A visitor enters a guest OS, a GPU (a
marketing name or a die) and optionally a CPU vendor, and the status site shows the runs across
driver versions with result, counts, latest date and run. Built (GPU-free tests only, nothing
deployed, no box ran it): migration `0003_ci_run_claims.sql`; `GET /v1/ci/setups` and
`GET /v1/ci/facets` (§6a); an optional `host_claims` block in the box result, accepted by the CI
Worker only when it asked for it with `KF_HOST_CLAIMS=1`; the static name table
`site/status/gpu_names.json`; the page `/status/find/` (§9a). The CPU vendor, the CPU model, the guest
OS versions and the host and guest drivers are **claims by the test box** (`claimed = 1`): the Worker
checks their shape, they group and filter public rows, and none decides a verdict, a die or a
requirement (the die and the PASS cross-check of the box's GPU name are the Worker's, unchanged).
The public API will later also serve rows of a separate community store, read-only; every row
therefore carries a `source` field now, and the `source` and `report_tier` filters exist (community
rows are not populated, accepted, and return nothing; **DESIGN-ONLY**).

Labels used below: **Owner ruling (2026-11-04)** is binding text from the owner's brief.
**Proposal (lane 1)** is this branch's addition and needs owner review. **Built** names what this
branch implements and tests (GPU-free, node and python unit tests only). Everything else is design.

Related: the CI trust model in [V3_CI_VAST_GPU_20261021.md](V3_CI_VAST_GPU_20261021.md), its
runner trace [V3_CI_VAST_RUNNER_TRACE_20261022.md](V3_CI_VAST_RUNNER_TRACE_20261022.md), the
Worker's [threat model](../../ci/vast-broker/docs/THREAT_MODEL.md), the SPECviewperf publication
rule [SPEC_RESULTS.md](../SPEC_RESULTS.md) and [OWNER_RULINGS.md](../OWNER_RULINGS.md) §AN (the
Worker is test infrastructure, not a product).

## 1. Purpose

**Owner ruling (2026-11-04).** Coverage is kayfabe's real problem: dies, families, host and guest
drivers, CPU vendor (Intel, AMD), nested versus bare metal. The Worker becomes **platform testing
infrastructure, decoupled from CI**. CI asks one question: *did requirements R pass on platform P
for commit X?* The Worker answers it lazily:

- it **joins an in-flight run** that already answers the question instead of renting again;
- it **caches FAIL** as well as PASS (a FAIL is an answer; a RETRY/UNAVAILABLE is not);
- it applies the existing caps (budget, concurrency, daily counts);
- **forks never trigger rentals**; a fork PR can only read answers that already exist.

**Docs-only commits fall through** by the **tree hash** (2026-11-05, see
[V3_CI_TIERS_AND_IGNORE_LADDER.md](V3_CI_TIERS_AND_IGNORE_LADDER.md) §3): commits whose tree is
identical after applying the commit's own `.workerignore` share every result. The requirement sets
live in `ci/requirements.toml`, **read from the BASE branch, never from the PR**, so a PR cannot
shrink what it is tested against; a PR cannot hide code by ignoring it either, because ignoring it
changes the filtered tree.

SUPERSEDED (2026-11-05, by the tree hash): the 2026-11-04 text here said the fall-through used a
hash of a declared, fail-closed path set in `ci/test_boundary.toml`, also read from the BASE branch.

## 2. Tiers

**Correction (2026-11-05).** The tier table of the owner's 2026-11-05 decisions, with the release tier
at up to 8 boxes, the middle tier's die rotation, no retries (RETRY never passes) and the rule that
the event picks the tier (PR to master = middle, release tag = largest, push or schedule = small), is
in [V3_CI_TIERS_AND_IGNORE_LADDER.md](V3_CI_TIERS_AND_IGNORE_LADDER.md) §1. The small tier rides on
the box the middle run rented. Scheduled runs were dropped for now. The table below is the
2026-11-04 text and is SUPERSEDED where it differs: "8 boxes in parallel" is **up to 8**.

**Owner ruling (2026-11-04).**

| Tier | When | Shape |
|---|---|---|
| largest (release) | release tags | at least 1 driver and 2 dies per family, 8 boxes in parallel, everything including SPECviewperf |
| middle (master PR) | the `next` -> master batch PR | 1 box, about 70 apps in about 10 minutes, no SPECviewperf, Windows required |
| small | driver patches, host/guest driver mismatch matrix | about 1 minute per run |

The tier is a column on every run row (`tier`). The 65-app Linux guest pass recorded in
`traces/v3_app_matrix/va1_rtx3060/r1/guest.res` and `va2_rtx3070/r1/guest.res` (committed in
`b6b936c47`, 2026-09-26) sums to 2460 s and 1759 s of `secs=` (re-summed 2026-11-04; the kayfabe
revision of those runs is not restated here). **Inferred:** "about 70 apps in about 10 minutes" is
below that, so the middle tier needs a defined subset; that subset is an open question (§13).

## 3. Components

**Owner ruling (2026-11-04)** unless marked.

| Component | Where | Holds secrets | Writes D1 | Notes |
|---|---|---|---|---|
| CI Worker | today's `kayfabe-vast-broker`, `ci.kayfabe.dev` | **all** (Vast key, run-token secret, row-tag key, R2 write keys) | yes | answers CI questions, rents, ingests results and logs, writes rows and HMAC tags; HostHub Durable Object for enrolled hosts (post-v1); admin UI |
| Public Worker | `api.kayfabe.dev`, `ci/platform-api/` | **none** | **no** | GET only, fixed parameterised queries, no raw SQL, no auth, CORS `*` on GET only, **never calls GitHub or any other URL**; data is push-based only |
| D1 database | `kayfabe-platform` | - | CI Worker only | one database shared by both Workers |
| R2 public bucket | `logs.kayfabe.dev` | - | CI Worker only | redacted logs, served `text/plain` with `nosniff` |
| R2 private bucket | - | - | owner/trusted upload | SPECviewperf media, Windows app disk (not redistributable), read through run tokens only |
| Frontend | `kayfabe.dev/status/` on GitHub Pages | none | no | built by the existing site workflow; reads the API base from repository variable `STATUS_API_BASE` |

**Accepted trade-off (owner ruling).** The public Worker has a D1 binding to the same database. The
worst case of a public-Worker defect is that it **leaks private columns or shows wrong data**. That
is bounded by three rules: private columns are minimal and live in separate tables (`hosts_private`,
`audit`) that no public query selects; reproduced rows carry an HMAC **tag** written by the CI
Worker, so a forged row never counts for a gate; and the gate reads rows **through the CI Worker
only** (§8).

**Proposal (lane 1): enforce "no write" at three layers in the public Worker.** (a) Every SQL
statement is a frozen constant in `ci/platform-api/src/queries.js`; the database wrapper accepts a
statement *name*, never SQL text. (b) A test parses each statement and refuses anything but a single
`SELECT`/`WITH ... SELECT`. (c) The tests run every statement against a seeded SQLite file opened
**read-only** (`node:sqlite`, `readOnly: true`), so a write would fail at the engine. A D1
read-replica or read-only binding does not exist for Workers as far as the D1 Worker API docs page read on 2026-11-04 states (it mentions none); if Cloudflare adds one, use it.

## 4. Questions, keys and lazy satisfaction (CI Worker, design only)

- **Requirement key** (`requirement_key`): a short name from `ci/requirements.toml` on the BASE
  branch, for example `linux-gate`, `windows-gate`, `apps-middle`. A requirement set names the
  suites and the platform constraints (family, die list, guest OS, tier).
- **Platform key** (`platform_key`): the canonical string of the platform facts, in a fixed field
  order: die, host driver, CPU vendor, CPU model, nested (0/1), host kernel, QEMU build, IOMMU mode,
  guest OS and guest driver. **Proposal (lane 1):** the key is `p-` plus the first 24 hex digits of
  SHA-256 over the `|`-joined canonical fields; the fields themselves are stored in `platforms`.
- **SUPERSEDED (2026-11-05) by the tree hash** (the ladder document, §3): the key is a hash of the
  sorted (path, mode, blob sha) list of files that survive the commit's own `.workerignore`, and the
  column keeps the name `boundary_hash` for the schema's sake. Truncated listings, unknown patterns
  or a matcher that disagrees with `git check-ignore` mean run (fail closed). Do not implement it as
  "all changed paths match the new ignore". The 2026-11-04 proposal follows and is retained as the
  superseded text.
- **Boundary hash** (`boundary_hash`, 2026-11-04 proposal, SUPERSEDED): SHA-256 over the sorted `path NUL mode NUL blob-id LF` lines
  of every tracked file that `ci/test_boundary.toml` (BASE copy) does not exclude.
  **Proposal (lane 1)**, fail-closed shape:

  ```toml
  # ci/test_boundary.toml (proposal; read from the BASE branch only)
  # Every tracked path is test-relevant unless it matches an exclusion.
  # Unknown, new or renamed paths are relevant. ci/** is always relevant.
  version = 1
  exclude = ["docs/**", "site/**", "traces/**", "archive/**", "**/*.md", "LICENSE*"]
  always_relevant = ["ci/**", ".github/**", "scripts/**"]
  ```

- **Satisfaction**: for question (R, P, X) the CI Worker computes `boundary_hash(X)` and looks for a
  final row with the same (R, P-constraints, boundary hash) and a valid tag. PASS or FAIL answers.
  An open run with the same triple is joined. Otherwise, if caps allow and the requester is not a
  fork, it rents. RETRY and UNAVAILABLE never answer a question and are never cached as answers.

## 5. Data model (Built: `ci/platform-api/migrations/0001_init.sql`)

**Owner ruling (2026-11-04)** for the row content; table split and indexes are **Proposal (lane 1)**.

| Table | Content | Public |
|---|---|---|
| `families`, `dies` | family id/name/order; die id (`AD104`), family, display name, order | yes |
| `platforms` | platform key and its facts (die, host driver, CPU vendor/model, nested, kernel, QEMU build, IOMMU mode, guest OS, guest driver) | yes |
| `commits` | sha, boundary hash, message, author, committed-at; **pushed in by the CI side, immutable** | yes |
| `branch_commits` | (branch, seq, sha): first-parent order per branch, for "N test-relevant changes ago" | yes |
| `runs` | id, commit sha, boundary hash, requirement key, platform key, die (denormalised for indexes), suite, verdict PASS/FAIL/RETRY/UNAVAILABLE, scores as ratios (JSON), host facts (JSON), log names (JSON), started/finished, source, tier, tag | yes, **except `tag`** |
| `run_stages` | run id, order, name, verdict, started/finished | yes |
| `requirement_results` | per commit and requirement key: state, the answering run, updated-at | yes |
| `hosts` | id, class (`vast`/`enrolled`), die, status, trust level, rentable-for-CI, remote-access | counts only |
| `hosts_private` | address, box-token hash, provider machine id | **never** |
| `audit` | who/what/when for CI-side writes | **never** |

`source` is `reproduced` today; `reviewed` and `reported` are reserved for community results after
release (§8). `tier` is `largest`, `middle`, `small` or `adhoc` (**Proposal (lane 1):** `adhoc` for
owner-dispatched sweeps outside the three tiers).

Indexes cover the four public query shapes: matrix (latest run per die and suite, with filters),
cell history (die, suite, time), run list (time, with filters) and commit requirements.

## 6. Public API v1 (Built: `ci/platform-api/src/`)

**Owner ruling (2026-11-04)**: GET only, stable resource URLs, immutable run responses carry
`Cache-Control: immutable`, others a short TTL.

| Path | Query parameters | Returns |
|---|---|---|
| `/v1/matrix` | `commit`, `family`, `driver`, `cpu`, `nested`, `branch` | dies by family x suites; per cell the latest run, its verdict and tier, the last PASS commit and the number of test-relevant changes since it |
| `/v1/cells/<die>/<suite>` | `before` (cursor), `limit` | the cell's run history, newest first, paginated |
| `/v1/runs` | `commit`, `die`, `suite`, `verdict`, `tier`, `source`, `cursor`, `limit` | run list, newest first, paginated |
| `/v1/runs/<id>` | none | one run with platform facts, stages, logs, provenance |
| `/v1/commits/<sha>/requirements` | none | commit facts and the requirement checklist |
| `/v1/dies` | none | families and dies |
| `/v1/hosts` | none | host counts by die, class and status; no addresses |
| `/v1/logs/<run>/<name>` | none | **302** to the public log bucket, only for a log the run lists |
| `/v1/schema` | none | machine-readable description of this table |
| `/v1/health` | none | `{"ok": true}` after one `SELECT 1` |

**Proposal (lane 1), implemented:**

- `branch` (`next` default, or `master`) selects the first-parent order used for "last passed at
  commit X, N test-relevant changes ago": N counts the distinct boundary hashes on that branch
  after X, up to the branch head or up to `commit` when given. With `commit`, cells show only runs
  whose boundary hash equals that commit's.
- A run in flight has `verdict: null` and `state: "running"`; `verdict` never changes once set.
- Every parameter has a strict regular expression; unknown or repeated parameters are `400`; an
  unknown path is `404`; any method other than GET is `405` with `Allow: GET` and no CORS header.
- `limit` is 1-100 (default 50). Cursors are base64url of a small JSON object whose fields are
  re-validated after decoding; a cursor is opaque to clients.
- Caching: a finished run (`finished_at` set and verdict final) is
  `public, max-age=31536000, immutable`; a run in flight and every list is `public, max-age=30`;
  `/v1/schema` and `/v1/dies` `max-age=300`; errors and `/v1/health` `no-store`.
- Headers on every response: `Content-Type` (`application/json; charset=utf-8`),
  `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`,
  `Referrer-Policy: no-referrer`.
- The Worker contains no `fetch(` call and no GitHub reference (a test reads the sources). Commit
  links on the site are plain hyperlinks built by the frontend.
- SPECviewperf scores are **never** returned in v1 (§8); the run carries `scores_hidden` instead.

## 6a. Setup picker endpoints (Built: `ci/platform-api/src/`, 2026-11-06)

`GET /v1/ci/setups?os=&gpu=&cpu=&driver=&tier=&source=&report_tier=&limit=` and
`GET /v1/ci/facets`; the parameters, the row shape, the attribution of a run over both guest OSes and
the meaning of `source` are in [`ci/platform-api/README.md`](../../ci/platform-api/README.md), "Setup
picker endpoints". Rules kept: fixed statements, anchored patterns, GET only, CORS `*`, no row tag,
machine id, price, cost, CPU model or SPECviewperf datum in a response. The host driver is the
box's own `nvidia-smi` driver, never the provider's offer label (owner ruling 2026-10-24: the offer's
driver label says nothing about a KVM rental).

## 7. Logs

**Owner ruling (2026-11-04).** Redacted logs go to the **public** R2 bucket served at
`logs.kayfabe.dev`, `text/plain` with `nosniff`. The public Worker 302-redirects
`/v1/logs/<run>/<name>` to it; run JSON may carry direct log URLs. Run-token redaction happens **on
the ingest stream in the CI Worker**, by exact value, with a **tail buffer across chunks** (a token
split over two chunks is still caught), plus a **token-pattern lint that holds a log back** when it
finds anything token-shaped. There is no raw/scrubbed split. Vast boxes need no GPU-UUID stripping.

**Proposal (lane 1):** object key `runs/<run id>/<name>`; names match
`^[a-z0-9][a-z0-9._-]{0,63}\.(log|txt)$`; the run row lists its log names, and the public Worker
redirects only for a listed name, to `LOGS_BASE` + `/runs/<run>/<name>`. With `LOGS_BASE` unset the
route returns `404`. The tail buffer is `max(token length) - 1` bytes, held back until the next chunk
or the end of the stream.

## 8. Trust and gates

**Owner ruling (2026-11-04).**

- **Level 1**: Worker-rented Vast boxes and owner-enrolled hosts. Untrusted for secrets; their PASS
  counts.
- **Community and stranger results are deferred until after release.** The `source` column
  (`reproduced` now; `reviewed`, `reported` later) and the `tier` column stay so they can be added
  without a schema change.
- **Nothing the public side stores or reads may influence a gate.** The gate reads reproduced rows
  **with a valid HMAC tag, through the CI Worker only.**
- **Performance** is stored as a **ratio to a same-box reference** and **never gates**.
- **SPECviewperf numbers** follow [SPEC_RESULTS.md](../SPEC_RESULTS.md) before the site shows them.

**Proposal (lane 1): row tag.** `tag = base64url(HMAC-SHA256(ROW_TAG_KEY, canonical))` where
`canonical` is `kf-row-v1` followed by the LF-joined fields id, commit sha, boundary hash,
requirement key, platform key, suite, verdict, source, tier and finished-at. Only the CI Worker holds
`ROW_TAG_KEY`; the public Worker neither verifies nor returns tags. A reference implementation with
tests is in `ci/platform-api/shared/row-tag.js` for the CI Worker to import; the public Worker does
not bundle it (a test checks this).

**Proposal (lane 1): what SPEC_RESULTS.md requires, encoded.** The rule requires `(nc)` after every
number, the verbatim disclaimer on the same page, the exact SPECviewperf release, the configuration
(host GPU, virtualisation layer, guest OS and driver), ratios only under the heading
`(nc) research comparison, not a SPEC-ranked result`, no ranking against spec.org and no
proprietary content; tables come from `scripts/bench/specviewperf/spec_stamp.py`. v1 therefore
**hides** SPECviewperf scores: the public API returns the verdict and stages for
`windows-specviewperf` runs but never their scores (`scores: null`, `scores_hidden: "SPEC_RESULTS.md"`),
and the frontend shows only PASS/FAIL. Showing scores later needs a stamped table produced by
`spec_stamp.py` and a page that carries the disclaimer; that is a separate owner decision.

## 9. Frontend (Built: `site/status/`, `site/status_pages.py`)

**Correction (2026-11-05, folded above the text it corrects).** The pages now read the CI Worker's
result store (`/v1/ci/matrix`, `/v1/ci/trees`, `/v1/ci/runs/<id>`, `/v1/ci/commits/<sha>`; PR stacked on
#258) and a static evidence file, `site/status/coverage.json`, generated by
`scripts/site/coverage_from_traces.py` from the committed trace directories
`traces/die_coverage_20261104` (revision 0c515b3b3, 2026-11-04) and `traces/gate_windows_ci_20261105`
(gpu-vast run 41936606326, revision d74f6063, 2026-11-05). The routes are `/status/?tier=`,
`/status/die/?family=&die=`, `/status/run/?id=` and `/status/commit/?sha=` or `?tree=`; the cell page
and the `/v1/matrix`, `/v1/cells`, `/v1/runs` and `/v1/commits/.../requirements` views of the older
text below are no longer used by the pages. Every entry carries one of two source labels:
"CI Worker run (reproduced)" or "Direct Vast run, evidence in repo (imported)". Only runs reproduced
by the CI Worker count for the release gate, and only through its result store; imported evidence is
drawn as run counts, never as a verdict. `traces/trio_vast_20261105` is not imported (placeholder in the
generator, no data). The fixture mode serves the live part from the synthetic fixture and shows the
evidence file unchanged.

**Owner ruling (2026-11-04).** Static pages at `kayfabe.dev/status/` built by the existing site
workflow. The API base comes from repository variable `STATUS_API_BASE` (set by the owner to
`https://api.kayfabe.dev`), optional `STATUS_LOGS_BASE`. **When the variable is empty** (local
builds, forks, PR previews) the status section is **not** generated; a short "not configured" page
is built instead. The workflow **validates** the value (https URL on `kayfabe.dev`, no path, no
credentials) like the analytics snippet. `--status-fixture` serves the pages from a local JSON
fixture. **Local builds contain no Worker URLs.** Pages: coverage matrix, cell history, run page,
commit requirements; later a wanted list. Plain JavaScript, no dependencies, accessible, phone
width, dark mode, no new build step. Commit links are plain hyperlinks to GitHub; the frontend makes
no GitHub API call.

**Proposal (lane 1), implemented:**

- The pages are written by the master build tooling (`build.py`), not by the root landing's Zola
  templates, so a release-tag landing without status templates still gets the current status pages.
- The variables are bound once, as workflow-level `env` in `.github/workflows/site.yml`, because
  the existing workflow guard (`ci/vast-broker/test/site-analytics.test.js`) forbids any `vars.`
  reference inside the build job. The build step passes `--status-api-base` only to the
  deployment-candidate build (`master` push or `v*` tag in this repository) and only after a bash
  check for an https origin on kayfabe.dev; `build.py` checks it again and ignores it in any
  non-candidate build. An invalid value is a warning and yields the "not configured" page,
  mirroring the analytics behaviour. PR, fork and portable builds never receive it.
- The base is written into one `<meta name="kf-status-api">` per page; `status.js` contains no URL.
- Fixture mode copies `site/status/fixture.json` (synthetic, generated from the seed through the
  real API handler by `ci/platform-api/scripts/make-fixture.mjs`) and shows a visible
  "synthetic data" banner on every page. Fixture mode and the deployment candidate are mutually
  exclusive.
- Routes are query-string pages: `/status/`, `/status/cell/?die=&suite=`, `/status/run/?id=`,
  `/status/commit/?sha=`.

## 9a. Find my setup (Built: `site/status/status.js`, `site/status/gpu_names.json`, 2026-11-06)

Route `/status/find/`, every control a URL parameter so a view can be shared: `os`, `gpu` (a name or a
die), `cpu` (`any`, `amd`, `intel`; Arm is listed as not tested yet and cannot be chosen), `show`
(repeated: `reproduced`, `imported` on by default, `community` opt-in), `tier`, `driver`. A name is
looked up in `gpu_names.json`, a hand-written table of desktop names (GTX 16, RTX 20, 30, 40, 50
lines) to dies, with one name allowed to map to several dies (RTX 3060 is GA106 and GA104; the page
lists each); a laptop name is answered with "laptops are not covered", an unlisted name asks for the
die. The table states only dies that are public product knowledge; a test checks it against the
Worker's die rules and the boards named in the evidence file. The Worker's die rules for Turing were corrected in the same PR after review (INFERRED from public
product data, not measured on hardware): the GTX 1650 and GTX 1630 are TU117 (the rule `GTX_16*` said
TU116), the RTX 2070 is TU106 (the rule `RTX_2070*` said TU104; only the RTX 2070 SUPER is TU104).
A test maps 18 Turing names and checks the PASS hardware check against the same rules; the table and
the rules now agree without exception. Name rules cannot see board variants (for example an RTX 2060
on another die); a PCI device id from the box would (follow-up, not built).
The page asks `/v1/ci/setups` once per die; in live mode the CPU vendor, driver, tier and community
opt-in all go to the API (so a page limit cannot hide a matching setup); fixture mode filters the CPU
vendor and driver in the page, because the fixture holds one file per die. Rows from the result store and the repository's imported
evidence (`coverage.json`) are separate blocks; imported evidence is drawn as counts with its source,
revision and date, never as a gate verdict. An empty result says "not tested" and "No runs for this
combination", with the reason `coverage.json` states for that die and OS when it states one. The page
says in words that the CPU vendor and both drivers are reported by the box and not verified. Fixture
mode serves every die of the name table for both OSes and the tier and community variants of each
(a tier together with community is not in the fixture); live mode asks only the configured API
origin and the same-origin static files, without credentials.

## 9b. Site redesign, run, session and stats pages (Built: `site/`, 2026-11-07)

Static pages only; no Worker change. The look: one system-font stylesheet with light and dark
tokens (`prefers-color-scheme`, plus a `data-theme` override that a button in the header sets and
keeps in this browser only), a header menu that is a `<details>` disclosure (open without
JavaScript, collapsed below 48 rem by `site.js`), status chips that carry a glyph as well as a
colour, tables that scroll inside their own frame, and a print stylesheet. `site/test_style.py`
checks the colour pairs of both themes against WCAG AA and that nothing loads from the network.
The portable build and the "no status URL, no analytics" rules are untouched.

Pages added or changed, all reading the report contract (`report` and `log` on
`/v1/ci/runs/<id>`, `/v1/ci/stats`, `/v1/ci/subruns`; the Worker side is built by another branch):

- `/status/run/?id=`: a summary block (verdict, tier, revision, date, duration, die, platform) with
  **Full log**, **Download report.json**, host-session and commit links; then performance numbers
  (value, unit, direction, ratio to the same-box reference, in words), tests (counts per suite,
  FAIL first, filter by result), applications, the Windows gate block, stages with durations, and the
  raw details closed. A button links only `<configured log origin>/<run id>/log.txt` (or the
  `report.json` beside it); any other `log.url` is not linked.
- `/status/session/?id=`: the host session: header, a table of subruns (kind, guest driver,
  result, counts, duration, byte range in the log), then one section per subrun filtered by `sub`.
  The route is a query string because the site is static; a path segment cannot carry a run id.
- `/status/stats/`: one sortable table per die and host driver with pass counts and the numbers
  behind each row; die, tier, driver, sort and direction are URL parameters.
- `/status/find/`: host driver, guest driver and commit fields, and a **Closeness** control (an
  ordered list of the differences that may be accepted, in the URL as `relax=`, `none` for exact
  only). When nothing matches exactly, or when a commit is given, the nearest subruns are listed with
  chips built from the API's `match` object ("same guest OS", "different commit revision (wanted
  X, found Y)").
- Every run row on the matrix, die, commit, stats, find and nearest lists links to its session.

Absent data is shown as an empty state with the reason ("Not measured yet: the application matrix
stage does not exist in CI yet"); the pages never compute or estimate a number, and a
viewport-benchmark row is dropped even if a report carried one. The committed fixture does not carry
reports, subruns or stats yet, so fixture mode shows those empty states; the DOM tests build the
pages against `site/report_fixture.mjs`, a synthetic extension in the contract's shape.

**Assumed shapes (the contract is silent; INFERRED, to be reconciled with the Worker's answer):**
`/v1/ci/stats` returns `{stats: [{die, family, tier, host_driver, run_id, commit, ended_at, verdict,
pass, fail, skip, perf: [...]}]}`; `/v1/ci/subruns` returns `{subruns: [{run_id, sub, kind, os, die,
guest_driver, host_driver, cpu_vendor, commit, ended_at, verdict, counts, match}]}`; the session's CPU
vendor comes from `host.cpu_vendor` or `report.cpu_vendor`; `relax` dimension names are `commit,
cpu, host_driver, guest_driver, die_family, os, die_any`.

## 10. Enrolled hosts (post-v1; design only, nothing built)

**Owner ruling (2026-11-04).**

- Enrolled non-Vast hosts join by an **outbound WebSocket** to the HostHub Durable Object with a
  **per-host box token** (hashed in D1, sent in a header or subprotocol, **never in the URL**, one
  connection per token). It is distinct from the per-run token.
- **Three message types only:** liveness/hardware report; job offer (commit + script + run token);
  command (shell string + timeout, **no stdin**, output capped at about 64 KiB with a `truncated`
  flag; files move by presigned R2 URL via `curl` inside the command). **No interactive terminal.**
- Interactive access goes through a **WireGuard hub on a small VPS** (relay only, hub -> host
  direction only, no `authorized_keys` for the owner on the hub) plus SSH, set up with commands.
- Registry fields per host: status; trust level (whether its reports count toward CI
  requirements); rentable-for-CI (only if its trust counts); remote access on/off (**off for lent
  boxes**, enforced locally in the daemon too).
- **Lent boxes** run public commits of `next`/`master` only, ideally inside a Docker container
  (plain Docker, `--device /dev/kvm`, NVIDIA container toolkit). Supported environments are whatever
  normal CUDA runs in.

**Proposal (lane 1):** the hash is SHA-256 of the token; the token is at least 32 random bytes,
base64url; the daemon refuses a `command` message when remote access is off, independent of what
the hub says.

## 11. What this branch builds, and what it does not

| Item | State |
|---|---|
| D1 schema and indexes (`ci/platform-api/migrations/0001_init.sql`) | Built, tested against `node:sqlite` |
| Public Worker (`ci/platform-api/src/`), `wrangler.toml` with the D1 binding and custom domain | Built, tested with a D1-shaped fake over `node:sqlite`; **not deployed** |
| Synthetic seed and fixture generator | Built; every seeded row is labelled synthetic |
| Mutants for read-only, fixed SQL and parameter validation | Built (`npm run mutate` in `ci/platform-api/`; 23 mutants, each killed by a named test in a local run on 2026-11-04; `ci/platform-api` tree as of `2ce1dfdf6`) |
| CI | `ci.yml` job `platform` (Node 22: format check, tests, mutants), required by the `CI` gate; the site workflow runs `test_status.py` and `test_status_dom.mjs` |
| Row-tag reference (`shared/row-tag.js`) | Built and tested; nothing calls it yet |
| Status pages, fixture mode, "not configured" page, workflow validation | Built, tested with the site's own checks |
| Setup picker: migration 0003, `/v1/ci/setups`, `/v1/ci/facets`, `host_claims` in the box result, `gpu_names.json`, `/status/find/` | Built, GPU-free tests and mutants; **not deployed**, no box has sent a claim |
| Community rows (`source` community, `report_tier`) | Filters and the per-row `source` field only; no store, no writer (design only) |
| CI Worker ingest, lazy satisfaction, redaction stream, log lint, R2 writes, HMAC writing | Design only |
| `ci/requirements.toml` | Design only (§4 shows a proposal) |
| `ci/test_boundary.toml` | Superseded 2026-11-05 by `.workerignore` and the tree hash; neither exists yet (design only) |
| Admin start route, die rotation, TRIO stage, SPECviewperf from private R2 | Design only (ladder document) |
| HostHub, enrolled hosts, WireGuard hub | Design only (§10) |
| Wanted list page | Not started |

## 12. Deployment steps for the coordinator (none were run by this branch)

1. Create D1 database `kayfabe-platform`, or confirm the id in `ci/platform-api/wrangler.toml`
   (`c6292be2-742d-4a05-95fd-749705ab30bc`, not a secret), and apply
   `migrations/0001_init.sql` (`wrangler d1 migrations apply kayfabe-platform --remote`).
2. Deploy `ci/platform-api/` (`npx wrangler deploy` from that directory) with custom domain
   `api.kayfabe.dev`; set var `LOGS_BASE` (for example `https://logs.kayfabe.dev`) once the bucket
   exists. The Worker needs **no secret**.
3. Create the public R2 bucket and bind `logs.kayfabe.dev` to it.
4. Bind the same D1 database to the CI Worker (lane 2 / later work) for writes.
5. Repository variable `STATUS_API_BASE` is already set; optionally set `STATUS_LOGS_BASE`.
6. Do not seed production D1 with the synthetic seed. It is for local development only.

## 13. Open questions

1. The middle tier's "about 70 apps in about 10 minutes": which subset? The measured full Linux
   pass is longer (§2). Still open on 2026-11-05; the other open items of the new tier design are in
   the ladder document (rotation order, TRIO floor and N, VRAM numbers).
2. Should a FAIL cached for a boundary hash expire (flaky boxes), or only be replaced by an owner
   re-run? Proposal: never expire; an owner dispatch may add a newer run that answers instead.
3. RETRY versus UNAVAILABLE: which provider faults are RETRY (re-rent) and which UNAVAILABLE?
4. Does the public matrix show `adhoc` sweep rows by default, or only tier rows?
5. Platform constraints in `ci/requirements.toml`: exact grammar (die list, family, "any die of the
   family", CPU vendor, nested).
6. The row-tag key rotation: keep old keys for verification, or re-tag rows on rotation?
7. Should `/v1/hosts` show enrolled hosts per owner, or only totals?
8. Retention of public logs in R2 (size, age).
9. Whether a D1 read-only binding becomes available for the public Worker (§3).
10. SPECviewperf scores on the site: owner decision on a stamped, disclaimed page (§8).
