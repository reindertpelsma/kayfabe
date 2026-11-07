-- kayfabe-platform D1 schema, version 4: the structured run report, host-session subruns and the
-- published full log's identity. Design: /root/night/REPORT_CONTRACT.md (coordinator, 2026-11-07),
-- docs/design/V3_PLATFORM_TESTING.md. Writer: the CI Worker only (ci/vast-broker/src/runreport.js).
-- Reader: the public Worker through fixed statements (ci/platform-api/src/queries.js).
-- 0001, 0002 and 0003 are unchanged: this migration only ADDS tables and indexes (additive; an old
-- Worker that never heard of them keeps working, and a new Worker without them fails open, see
-- ci/vast-broker/DEPLOY.md "Run reports").
--
-- Every value here is a CLAIM of the test box except the identity of the published log (its size
-- and SHA-256 are computed by the CI Worker from the exact bytes it wrote to the public bucket).
-- Nothing here decides a verdict, a die or a requirement, and none of it is part of the HMAC row
-- tag of ci_runs. A session (one rental, one ci_runs row) holds one or more SUBRUNS: the host
-- itself (bare-metal tests, host driver), and one per guest OS and guest driver it booted.
-- No SPECviewperf score exists in these tables (docs/SPEC_RESULTS.md); the Worker drops such rows.

CREATE TABLE ci_run_reports (
  run_id TEXT PRIMARY KEY REFERENCES ci_runs (run_id),
  schema_version INTEGER NOT NULL CHECK (schema_version = 1),
  -- The bounded, sanitised report.json (NULL when the box sent none). At most 256 KiB.
  report_json TEXT CHECK (report_json IS NULL OR length(CAST(report_json AS BLOB)) <= 262144),
  -- The NVIDIA driver of the machine that ran kayfabe (claim), for the stats query.
  host_driver TEXT CHECK (host_driver IS NULL OR length(host_driver) BETWEEN 5 AND 16),
  -- The published full log: bytes and SHA-256 of the object the CI Worker wrote (NULL: none).
  log_bytes INTEGER CHECK (log_bytes IS NULL OR log_bytes BETWEEN 0 AND 33554432),
  log_sha256 TEXT CHECK (log_sha256 IS NULL OR (length(log_sha256) = 64 AND log_sha256 NOT GLOB '*[^0-9a-f]*')),
  recorded_at TEXT NOT NULL
);
-- The stats query: latest reported run per die and host driver.
CREATE INDEX ci_run_reports_driver ON ci_run_reports (host_driver);

CREATE TABLE ci_subruns (
  run_id TEXT NOT NULL REFERENCES ci_runs (run_id),
  -- 'host', 'linux-580.159.03', 'windows-580.88' (os, then the guest driver when there is one).
  sub TEXT NOT NULL CHECK (length(sub) BETWEEN 4 AND 40 AND sub NOT GLOB '*[^a-z0-9._-]*'),
  ord INTEGER NOT NULL CHECK (ord BETWEEN 0 AND 63),
  kind TEXT NOT NULL CHECK (kind IN ('host_bare', 'linux_guest', 'windows_guest')),
  os TEXT NOT NULL CHECK (os IN ('linux', 'windows', 'host')),
  guest_driver TEXT CHECK (guest_driver IS NULL OR length(guest_driver) BETWEEN 5 AND 16),
  host_driver TEXT CHECK (host_driver IS NULL OR length(host_driver) BETWEEN 5 AND 16),
  -- The box's claim about this subrun. NOT_RUN always carries a reason.
  verdict TEXT NOT NULL CHECK (verdict IN ('PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED', 'NOT_RUN')),
  reason TEXT CHECK (reason IS NULL OR length(reason) <= 200),
  -- JSON array of the suite-row names that belong to this subrun.
  stages_json TEXT NOT NULL DEFAULT '[]' CHECK (length(stages_json) <= 1024),
  pass_count INTEGER NOT NULL DEFAULT 0 CHECK (pass_count BETWEEN 0 AND 100000000),
  fail_count INTEGER NOT NULL DEFAULT 0 CHECK (fail_count BETWEEN 0 AND 100000000),
  skip_count INTEGER NOT NULL DEFAULT 0 CHECK (skip_count BETWEEN 0 AND 100000000),
  started_at TEXT,
  ended_at TEXT,
  duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms BETWEEN 0 AND 604800000),
  -- Byte offsets into the published log.txt, found by the CI Worker itself from the stage markers
  -- of the log it wrote (never taken from the box: the box's own offsets do not survive redaction).
  log_start INTEGER CHECK (log_start IS NULL OR log_start >= 0),
  log_end INTEGER CHECK (log_end IS NULL OR log_end >= 0),
  PRIMARY KEY (run_id, sub)
);
CREATE INDEX ci_subruns_lookup ON ci_subruns (os, host_driver, guest_driver);
CREATE INDEX ci_subruns_recent ON ci_subruns (ended_at DESC, run_id);

-- Per-arm and per-test rows (30 raw arms, 30 thin arms, cuda rungs), keyed by subrun.
CREATE TABLE ci_run_tests (
  run_id TEXT NOT NULL,
  sub TEXT NOT NULL,
  ord INTEGER NOT NULL CHECK (ord BETWEEN 0 AND 1999),
  suite TEXT NOT NULL CHECK (length(suite) BETWEEN 1 AND 48),
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  result TEXT NOT NULL CHECK (result IN ('PASS', 'FAIL', 'SKIP', 'REFUSED')),
  ms INTEGER CHECK (ms IS NULL OR ms BETWEEN 0 AND 604800000),
  PRIMARY KEY (run_id, ord),
  FOREIGN KEY (run_id, sub) REFERENCES ci_subruns (run_id, sub)
);
CREATE INDEX ci_run_tests_sub ON ci_run_tests (run_id, sub, ord);

-- Measured numbers (benchmark rows). Never a SPECviewperf score; never a gate.
CREATE TABLE ci_run_perf (
  run_id TEXT NOT NULL,
  sub TEXT NOT NULL,
  ord INTEGER NOT NULL CHECK (ord BETWEEN 0 AND 199),
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  value REAL NOT NULL,
  unit TEXT NOT NULL CHECK (length(unit) BETWEEN 1 AND 16),
  higher_is_better INTEGER NOT NULL CHECK (higher_is_better IN (0, 1)),
  suite TEXT NOT NULL CHECK (length(suite) BETWEEN 1 AND 48),
  -- A same-box reference the box measured itself (the host run of the same benchmark), when any.
  ref_value REAL,
  ref_ratio REAL,
  PRIMARY KEY (run_id, ord),
  FOREIGN KEY (run_id, sub) REFERENCES ci_subruns (run_id, sub)
);
CREATE INDEX ci_run_perf_sub ON ci_run_perf (run_id, sub, ord);

-- The application matrix (not a payload stage yet: no row is written today).
CREATE TABLE ci_run_apps (
  run_id TEXT NOT NULL,
  sub TEXT NOT NULL,
  ord INTEGER NOT NULL CHECK (ord BETWEEN 0 AND 499),
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  os TEXT NOT NULL CHECK (os IN ('linux', 'windows')),
  result TEXT NOT NULL CHECK (result IN ('PASS', 'FAIL', 'NOT_RUN')),
  note TEXT CHECK (note IS NULL OR length(note) <= 200),
  PRIMARY KEY (run_id, ord),
  FOREIGN KEY (run_id, sub) REFERENCES ci_subruns (run_id, sub)
);
CREATE INDEX ci_run_apps_sub ON ci_run_apps (run_id, sub, ord);
