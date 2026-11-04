-- kayfabe-platform D1 schema, version 1.
-- Design: docs/design/V3_PLATFORM_TESTING.md section 5 (DESIGN-ONLY, 2026-11-04).
-- Writers: the CI Worker only. Reader: the public Worker (ci/platform-api/src), through the
-- fixed statements in src/queries.js. Tables hosts_private and audit are never selected by a
-- public statement (test/static.test.js checks this), and runs.tag is never selected either.
-- Timestamps are UTC text 'YYYY-MM-DDTHH:MM:SSZ', so text order is time order.

CREATE TABLE families (
  id TEXT PRIMARY KEY CHECK (length(id) BETWEEN 3 AND 16 AND id NOT GLOB '*[^a-z0-9]*'),
  name TEXT NOT NULL,
  ord INTEGER NOT NULL
);

CREATE TABLE dies (
  id TEXT PRIMARY KEY CHECK (length(id) BETWEEN 4 AND 8 AND id NOT GLOB '*[^A-Z0-9]*'),
  family_id TEXT NOT NULL REFERENCES families (id),
  name TEXT NOT NULL,
  ord INTEGER NOT NULL
);
CREATE INDEX dies_family ON dies (family_id, ord);

-- One row per distinct platform. key = 'p-' + first 24 hex digits of SHA-256 over the
-- '|'-joined fields in this column order (die .. guest_driver); written by the CI Worker.
CREATE TABLE platforms (
  key TEXT PRIMARY KEY,
  die TEXT NOT NULL REFERENCES dies (id),
  host_driver TEXT NOT NULL,
  cpu_vendor TEXT NOT NULL CHECK (cpu_vendor IN ('intel', 'amd')),
  cpu_model TEXT NOT NULL,
  nested INTEGER NOT NULL CHECK (nested IN (0, 1)),
  kernel TEXT NOT NULL,
  qemu_build TEXT NOT NULL,
  iommu_mode TEXT NOT NULL,
  guest_os TEXT NOT NULL CHECK (guest_os IN ('linux', 'windows')),
  guest_driver TEXT NOT NULL
);
CREATE INDEX platforms_filters ON platforms (die, host_driver, cpu_vendor, nested);

-- Pushed in by the CI side; immutable once written.
CREATE TABLE commits (
  sha TEXT PRIMARY KEY CHECK (length(sha) = 40 AND sha NOT GLOB '*[^0-9a-f]*'),
  boundary_hash TEXT NOT NULL CHECK (length(boundary_hash) = 64),
  message TEXT NOT NULL CHECK (length(message) <= 200),
  author TEXT NOT NULL CHECK (length(author) <= 100),
  committed_at TEXT NOT NULL,
  pushed_at TEXT NOT NULL
);
CREATE INDEX commits_boundary ON commits (boundary_hash);

-- First-parent order per branch: seq grows by one per pushed commit.
CREATE TABLE branch_commits (
  branch TEXT NOT NULL CHECK (branch IN ('master', 'next')),
  seq INTEGER NOT NULL CHECK (seq >= 1),
  sha TEXT NOT NULL REFERENCES commits (sha),
  PRIMARY KEY (branch, seq)
);
CREATE INDEX branch_commits_sha ON branch_commits (sha, branch);

CREATE TABLE runs (
  id TEXT PRIMARY KEY CHECK (length(id) BETWEEN 6 AND 64),
  commit_sha TEXT NOT NULL REFERENCES commits (sha),
  boundary_hash TEXT NOT NULL,
  requirement_key TEXT NOT NULL,
  platform_key TEXT NOT NULL REFERENCES platforms (key),
  die TEXT NOT NULL REFERENCES dies (id),
  suite TEXT NOT NULL,
  -- NULL while the run is in flight; final verdicts never change afterwards.
  verdict TEXT CHECK (verdict IS NULL OR verdict IN ('PASS', 'FAIL', 'RETRY', 'UNAVAILABLE')),
  -- JSON object name -> ratio to a same-box reference; never gates.
  scores_json TEXT,
  host_facts_json TEXT NOT NULL DEFAULT '{}',
  -- JSON array of log names; objects live at runs/<id>/<name> in the public bucket.
  logs_json TEXT NOT NULL DEFAULT '[]',
  started_at TEXT NOT NULL,
  finished_at TEXT,
  -- finished_at, or started_at while in flight: the single ordering column for pagination.
  sort_at TEXT NOT NULL,
  source TEXT NOT NULL CHECK (source IN ('reproduced', 'reviewed', 'reported')),
  tier TEXT NOT NULL CHECK (tier IN ('largest', 'middle', 'small', 'adhoc')),
  -- HMAC row tag written by the CI Worker; read by the CI Worker's gate only.
  tag TEXT
);
CREATE INDEX runs_cell ON runs (die, suite, sort_at DESC, id DESC);
CREATE INDEX runs_time ON runs (sort_at DESC, id DESC);
CREATE INDEX runs_commit ON runs (commit_sha, sort_at DESC);
CREATE INDEX runs_boundary ON runs (boundary_hash, die, suite);
CREATE INDEX runs_platform ON runs (platform_key);

CREATE TABLE run_stages (
  run_id TEXT NOT NULL REFERENCES runs (id),
  ord INTEGER NOT NULL,
  name TEXT NOT NULL,
  verdict TEXT CHECK (verdict IS NULL OR verdict IN ('PASS', 'FAIL', 'RETRY', 'UNAVAILABLE', 'SKIPPED')),
  started_at TEXT,
  finished_at TEXT,
  PRIMARY KEY (run_id, ord)
);

-- The answer to "did requirement R pass for commit X", as the CI Worker last decided it.
CREATE TABLE requirement_results (
  commit_sha TEXT NOT NULL REFERENCES commits (sha),
  requirement_key TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('PASS', 'FAIL', 'PENDING', 'UNAVAILABLE', 'NOT_NEEDED')),
  run_id TEXT REFERENCES runs (id),
  updated_at TEXT NOT NULL,
  PRIMARY KEY (commit_sha, requirement_key)
);

-- Public host facts: the API returns counts only.
CREATE TABLE hosts (
  id TEXT PRIMARY KEY,
  class TEXT NOT NULL CHECK (class IN ('vast', 'enrolled')),
  die TEXT REFERENCES dies (id),
  status TEXT NOT NULL CHECK (status IN ('online', 'offline', 'retired')),
  trust_level INTEGER NOT NULL DEFAULT 1,
  rentable_for_ci INTEGER NOT NULL DEFAULT 0 CHECK (rentable_for_ci IN (0, 1)),
  remote_access INTEGER NOT NULL DEFAULT 0 CHECK (remote_access IN (0, 1)),
  last_seen_at TEXT
);
CREATE INDEX hosts_counts ON hosts (die, class, status);

-- Private: never selected by the public Worker.
CREATE TABLE hosts_private (
  host_id TEXT PRIMARY KEY REFERENCES hosts (id),
  address TEXT,
  box_token_sha256 TEXT,
  provider_machine_id TEXT
);

-- Private: never selected by the public Worker.
CREATE TABLE audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  detail TEXT
);
