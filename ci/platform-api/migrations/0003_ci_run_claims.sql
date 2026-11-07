-- kayfabe-platform D1 schema, version 3: host facts the test box CLAIMS (setup picker).
-- Design: docs/design/V3_PLATFORM_TESTING.md (DESIGN-ONLY, 2026-11-06). Writer: the CI Worker only
-- (ci/vast-broker/src/results.js). Reader: the public Worker (GET /v1/ci/setups and /v1/ci/facets).
-- 0001 and 0002 are unchanged: this migration only ADDS a table and indexes.
--
-- Every value here is a CLAIM: the box's payload read it from its own /proc and nvidia-smi and the
-- Worker only checked its shape. Nothing in this table decides a verdict (a claim never upgrades a
-- PASS; the verdict and the die come from the Worker's own checks) and nothing here is a serial,
-- UUID, address, hostname, machine id or price. One row per guest OS a run covered: a run with
-- ci_runs.platform 'both' has a 'linux' and a 'windows' row that share the host-side columns.
-- NULL means "not reported" (an older box, or a value that failed its pattern and was dropped).

CREATE TABLE ci_run_claims (
  run_id TEXT NOT NULL REFERENCES ci_runs (run_id),
  guest_os TEXT NOT NULL CHECK (guest_os IN ('linux', 'windows')),
  -- Bounded free text from the harness ('ubuntu-24.04', '11-ltsc-2024-26100').
  guest_os_version TEXT CHECK (guest_os_version IS NULL OR length(guest_os_version) BETWEEN 1 AND 32),
  -- NVIDIA driver version of the guest and of the machine that runs kayfabe, '580.95.05' style.
  guest_driver TEXT CHECK (guest_driver IS NULL OR length(guest_driver) BETWEEN 5 AND 16),
  host_driver TEXT CHECK (host_driver IS NULL OR length(host_driver) BETWEEN 5 AND 16),
  cpu_vendor TEXT CHECK (cpu_vendor IS NULL OR cpu_vendor IN ('amd', 'intel', 'arm', 'other')),
  cpu_model TEXT CHECK (cpu_model IS NULL OR length(cpu_model) BETWEEN 1 AND 64),
  cpu_arch TEXT CHECK (cpu_arch IS NULL OR cpu_arch IN ('x86_64', 'aarch64')),
  -- 1 = reported by the box, not verified. 0 is reserved for a fact the Worker itself verified
  -- (none exists yet), so a reader never has to guess which kind a row is.
  claimed INTEGER NOT NULL DEFAULT 1 CHECK (claimed IN (0, 1)),
  PRIMARY KEY (run_id, guest_os)
);
CREATE INDEX ci_run_claims_filters ON ci_run_claims (guest_os, cpu_vendor, host_driver, guest_driver);
-- "results for GPU die X", newest first (the setups query).
CREATE INDEX ci_runs_die ON ci_runs (die, sort_at DESC);
