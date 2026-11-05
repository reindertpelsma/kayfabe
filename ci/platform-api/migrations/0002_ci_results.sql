-- kayfabe-platform D1 schema, version 2: the CI Worker's result store.
-- Design: docs/design/V3_PLATFORM_TESTING.md §4/§8 and V3_CI_TIERS_AND_IGNORE_LADDER.md §3
-- (both DESIGN-ONLY, 2026-11-05). Writer: the CI Worker only (ci/vast-broker/src/results.js).
-- Reader: the public Worker through fixed statements (ci/platform-api/src/queries.js), which never
-- select ci_runs.tag, ci_runs_private or ci_requirement_snapshots (test/static.test.js).
--
-- Why new tables instead of rows in 0001's `runs`: a CI Worker run is one rental with one
-- verdict over a whole profile (for example the Linux bar plus the Windows gate), keyed by the
-- tree hash, while 0001's `runs` row is one suite on one fully described platform. Writing CI rows
-- there would need facts the Worker does not have (`platforms.cpu_vendor` is CHECKed to
-- intel/amd, `guest_os` to one OS, `verdict` has no UNATTRIBUTED, `commit_sha` needs a pushed
-- `commits` row with message and author), so the Worker would have to invent them. 0001 stays
-- unchanged and every public query on it keeps working; these tables reuse its conventions
-- (UTC text timestamps, `source`, `tier` vocabulary, a private table for private columns, an
-- HMAC `tag` that only the CI Worker reads).

CREATE TABLE ci_runs (
  run_id TEXT PRIMARY KEY CHECK (
    length(run_id) BETWEEN 6 AND 64 AND run_id NOT GLOB '*[^a-z0-9-]*'
    AND substr(run_id, 1, 1) <> '-'),
  commit_sha TEXT NOT NULL CHECK (length(commit_sha) = 40 AND commit_sha NOT GLOB '*[^0-9a-f]*'),
  -- The result key: 'tree' = the .workerignore-filtered tree hash computed by the CI Worker
  -- itself; 'claimed' = a tree hash a caller supplied (never answers for another commit);
  -- 'commit' = no tree key could be computed (fail closed), a per-commit hash nothing shares.
  tree_hash TEXT NOT NULL CHECK (length(tree_hash) = 64 AND tree_hash NOT GLOB '*[^0-9a-f]*'),
  tree_kind TEXT NOT NULL CHECK (tree_kind IN ('tree', 'commit', 'claimed')),
  tier TEXT NOT NULL CHECK (tier IN ('largest', 'middle', 'small', 'adhoc')),
  profile TEXT NOT NULL CHECK (length(profile) BETWEEN 1 AND 32),
  family TEXT NOT NULL CHECK (length(family) BETWEEN 3 AND 16 AND family NOT GLOB '*[^a-z0-9]*'),
  die TEXT NOT NULL CHECK (length(die) BETWEEN 4 AND 8 AND die NOT GLOB '*[^A-Z0-9_]*'),
  platform TEXT NOT NULL CHECK (platform IN ('linux', 'windows', 'both')),
  -- NULL while in flight; a final verdict never changes. RETRY and UNATTRIBUTED never pass.
  verdict TEXT CHECK (verdict IS NULL OR verdict IN ('PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED')),
  started_at TEXT NOT NULL,
  ended_at TEXT,
  -- ended_at, or started_at while in flight.
  sort_at TEXT NOT NULL,
  -- Object key of the redacted public log (runs/<run_id>/<name>), when one was written.
  log_key TEXT CHECK (log_key IS NULL OR length(log_key) <= 140),
  -- 'reproduced' today; 'reviewed' and 'reported' are reserved for community rows (not built).
  source TEXT NOT NULL DEFAULT 'reproduced' CHECK (source IN ('reproduced', 'reviewed', 'reported')),
  -- HMAC row tag over the row and its stages (ROW_TAG_KEY); read by the CI Worker only.
  tag TEXT,
  recorded_at TEXT NOT NULL,
  CHECK ((verdict IS NULL) = (ended_at IS NULL))
);
-- "results for tree_hash X by family/die/platform", newest first (requirements check, tree page).
CREATE INDEX ci_runs_key ON ci_runs (tree_hash, family, die, platform, sort_at DESC);
-- The matrix: latest run per tier/family/die/platform.
CREATE INDEX ci_runs_cell ON ci_runs (tier, family, die, platform, sort_at DESC);
CREATE INDEX ci_runs_commit ON ci_runs (commit_sha, sort_at DESC);
CREATE INDEX ci_runs_time ON ci_runs (sort_at DESC, run_id DESC);

CREATE TABLE ci_run_stages (
  run_id TEXT NOT NULL REFERENCES ci_runs (run_id),
  ord INTEGER NOT NULL CHECK (ord BETWEEN 0 AND 63),
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 48 AND name NOT GLOB '*[^a-z0-9_]*'),
  rc INTEGER CHECK (rc IS NULL OR rc BETWEEN -255 AND 255),
  duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms BETWEEN 0 AND 604800000),
  result TEXT NOT NULL CHECK (result IN ('PASS', 'FAIL', 'SKIP', 'NOT_RUN', 'RETRY', 'UNATTRIBUTED')),
  PRIMARY KEY (run_id, ord)
);

-- Public host facts of a run (what the box and the offer reported).
CREATE TABLE ci_host_facts (
  run_id TEXT PRIMARY KEY REFERENCES ci_runs (run_id),
  gpu_name TEXT CHECK (gpu_name IS NULL OR length(gpu_name) <= 100),
  driver_versions_json TEXT NOT NULL DEFAULT '{}' CHECK (length(driver_versions_json) <= 1024)
);

-- Private: never selected by the public Worker.
CREATE TABLE ci_runs_private (
  run_id TEXT PRIMARY KEY REFERENCES ci_runs (run_id),
  machine_id INTEGER,
  price_usd_h REAL,
  cost_usd REAL
);

-- Private: the BASE-branch ci/requirements.toml texts the CI Worker evaluated, by SHA-256.
CREATE TABLE ci_requirement_snapshots (
  sha256 TEXT PRIMARY KEY CHECK (length(sha256) = 64),
  base_ref TEXT NOT NULL CHECK (base_ref IN ('master', 'next')),
  content TEXT NOT NULL CHECK (length(content) <= 65536),
  first_seen_at TEXT NOT NULL
);
