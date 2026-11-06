// Every SQL statement the public Worker can run: read-only, fixed text, numbered parameters.
// The database wrapper (db.js) accepts a statement NAME from this table, never SQL text.
// test/static.test.js refuses a statement that is not a single SELECT, any `${` or string
// concatenation in this file, and any reference to a row tag, hosts_private, audit, or the CI
// store's private tables (ci_runs_private, ci_requirement_snapshots).
// Optional filters use `(?N IS NULL OR column = ?N)` so the text never depends on the request.

export const QUERIES = Object.freeze({
  health: `SELECT 1 AS ok`,

  dies: `
    SELECT f.id AS family_id, f.name AS family_name, d.id AS die, d.name AS die_name
    FROM families f LEFT JOIN dies d ON d.family_id = f.id
    ORDER BY f.ord, f.id, d.ord, d.id`,

  dieById: `SELECT d.id AS die, d.name AS die_name, d.family_id FROM dies d WHERE d.id = ?1`,

  commitBySha: `
    SELECT c.sha, c.boundary_hash, c.message, c.author, c.committed_at, c.pushed_at
    FROM commits c WHERE c.sha = ?1`,

  commitBranches: `SELECT b.branch, b.seq FROM branch_commits b WHERE b.sha = ?1 ORDER BY b.branch`,

  branchHead: `
    SELECT b.seq, b.sha FROM branch_commits b WHERE b.branch = ?1 ORDER BY b.seq DESC LIMIT 1`,

  // Latest run per (die, suite) under the filters ?1 family, ?2 host driver, ?3 CPU vendor,
  // ?4 nested; ?5 restricts to one boundary hash (the matrix for one commit).
  matrixLatest: `
    SELECT x.id, x.die, x.suite, x.verdict, x.tier, x.source, x.commit_sha, x.sort_at,
           x.finished_at
    FROM (
      SELECT r.id, r.die, r.suite, r.verdict, r.tier, r.source, r.commit_sha, r.sort_at,
             r.finished_at,
             ROW_NUMBER() OVER (PARTITION BY r.die, r.suite ORDER BY r.sort_at DESC, r.id DESC)
               AS rn
      FROM runs r
      JOIN platforms p ON p.key = r.platform_key
      JOIN dies d ON d.id = r.die
      WHERE (?1 IS NULL OR d.family_id = ?1)
        AND (?2 IS NULL OR p.host_driver = ?2)
        AND (?3 IS NULL OR p.cpu_vendor = ?3)
        AND (?4 IS NULL OR p.nested = ?4)
        AND (?5 IS NULL OR r.boundary_hash = ?5)
    ) x
    WHERE x.rn = 1`,

  // Last PASS per (die, suite) on branch ?5 at or before seq ?6 (same filters ?1-?4), and the
  // number of distinct test-relevant changes (boundary hashes) on the branch since, up to ?6.
  matrixLastPass: `
    SELECT x.id, x.die, x.suite, x.tier, x.commit_sha, x.seq, x.sort_at,
           (SELECT COUNT(DISTINCT c2.boundary_hash)
              FROM branch_commits b2 JOIN commits c2 ON c2.sha = b2.sha
             WHERE b2.branch = ?5 AND b2.seq > x.seq AND b2.seq <= ?6
               AND c2.boundary_hash <> x.boundary_hash) AS changes_since
    FROM (
      SELECT r.id, r.die, r.suite, r.tier, r.commit_sha, r.boundary_hash, b.seq, r.sort_at,
             ROW_NUMBER() OVER (
               PARTITION BY r.die, r.suite ORDER BY b.seq DESC, r.sort_at DESC, r.id DESC
             ) AS rn
      FROM runs r
      JOIN branch_commits b ON b.sha = r.commit_sha AND b.branch = ?5
      JOIN platforms p ON p.key = r.platform_key
      JOIN dies d ON d.id = r.die
      WHERE r.verdict = 'PASS' AND b.seq <= ?6
        AND (?1 IS NULL OR d.family_id = ?1)
        AND (?2 IS NULL OR p.host_driver = ?2)
        AND (?3 IS NULL OR p.cpu_vendor = ?3)
        AND (?4 IS NULL OR p.nested = ?4)
    ) x
    WHERE x.rn = 1`,

  // Newest first. Filters: ?1 commit, ?2 die, ?3 suite, ?4 verdict ('RUNNING' = no verdict
  // yet), ?5 tier, ?6 source. Keyset cursor: ?7 sort_at, ?8 id. ?9 = page size + 1.
  runList: `
    SELECT r.id, r.commit_sha, r.boundary_hash, r.requirement_key, r.platform_key, r.die,
           r.suite, r.verdict, r.scores_json, r.started_at, r.finished_at, r.sort_at,
           r.source, r.tier,
           p.host_driver, p.cpu_vendor, p.cpu_model, p.nested, p.kernel, p.qemu_build,
           p.iommu_mode, p.guest_os, p.guest_driver
    FROM runs r JOIN platforms p ON p.key = r.platform_key
    WHERE (?1 IS NULL OR r.commit_sha = ?1)
      AND (?2 IS NULL OR r.die = ?2)
      AND (?3 IS NULL OR r.suite = ?3)
      AND (?4 IS NULL OR (?4 = 'RUNNING' AND r.verdict IS NULL) OR r.verdict = ?4)
      AND (?5 IS NULL OR r.tier = ?5)
      AND (?6 IS NULL OR r.source = ?6)
      AND (?7 IS NULL OR r.sort_at < ?7 OR (r.sort_at = ?7 AND r.id < ?8))
    ORDER BY r.sort_at DESC, r.id DESC
    LIMIT ?9`,

  runById: `
    SELECT r.id, r.commit_sha, r.boundary_hash, r.requirement_key, r.platform_key, r.die,
           r.suite, r.verdict, r.scores_json, r.host_facts_json, r.logs_json, r.started_at,
           r.finished_at, r.sort_at, r.source, r.tier,
           p.host_driver, p.cpu_vendor, p.cpu_model, p.nested, p.kernel, p.qemu_build,
           p.iommu_mode, p.guest_os, p.guest_driver,
           d.family_id,
           c.message AS commit_message, c.author AS commit_author,
           c.committed_at AS commit_committed_at
    FROM runs r
    JOIN platforms p ON p.key = r.platform_key
    JOIN dies d ON d.id = r.die
    LEFT JOIN commits c ON c.sha = r.commit_sha
    WHERE r.id = ?1`,

  runStages: `
    SELECT s.ord, s.name, s.verdict, s.started_at, s.finished_at
    FROM run_stages s WHERE s.run_id = ?1 ORDER BY s.ord`,

  runLogs: `SELECT r.logs_json FROM runs r WHERE r.id = ?1`,

  commitRequirements: `
    SELECT q.requirement_key, q.state, q.run_id, q.updated_at,
           r.die, r.suite, r.verdict, r.tier, r.finished_at
    FROM requirement_results q LEFT JOIN runs r ON r.id = q.run_id
    WHERE q.commit_sha = ?1
    ORDER BY q.requirement_key`,

  // CI Worker result store (migrations/0002_ci_results.sql). Never ci_runs.tag, never the
  // private tables. ?1 tier, ?2 page size.
  ciTrees: `
    SELECT r.tree_hash, r.tree_kind, MAX(r.sort_at) AS last_at, MIN(r.started_at) AS first_at,
           COUNT(*) AS runs,
           SUM(CASE WHEN r.verdict = 'PASS' THEN 1 ELSE 0 END) AS passed,
           SUM(CASE WHEN r.verdict = 'FAIL' THEN 1 ELSE 0 END) AS failed,
           SUM(CASE WHEN r.verdict IS NULL THEN 1 ELSE 0 END) AS running
    FROM ci_runs r
    WHERE (?1 IS NULL OR r.tier = ?1)
    GROUP BY r.tree_hash, r.tree_kind
    ORDER BY last_at DESC, r.tree_hash DESC
    LIMIT ?2`,

  // Latest run per (tier, family, die, platform); filters ?1 tier, ?2 tree hash, ?3 family.
  ciMatrixLatest: `
    SELECT x.run_id, x.tier, x.family, x.die, x.platform, x.verdict, x.commit_sha, x.tree_hash,
           x.tree_kind, x.started_at, x.ended_at
    FROM (
      SELECT r.run_id, r.tier, r.family, r.die, r.platform, r.verdict, r.commit_sha, r.tree_hash,
             r.tree_kind, r.started_at, r.ended_at,
             ROW_NUMBER() OVER (
               PARTITION BY r.tier, r.family, r.die, r.platform
               ORDER BY r.sort_at DESC, r.run_id DESC
             ) AS rn
      FROM ci_runs r
      WHERE (?1 IS NULL OR r.tier = ?1)
        AND (?2 IS NULL OR r.tree_hash = ?2)
        AND (?3 IS NULL OR r.family = ?3)
    ) x
    WHERE x.rn = 1
    ORDER BY x.tier, x.family, x.die, x.platform`,

  // Last PASS per (tier, family, die, platform) under the same filters.
  ciMatrixLastPass: `
    SELECT x.run_id, x.tier, x.family, x.die, x.platform, x.commit_sha, x.tree_hash, x.ended_at
    FROM (
      SELECT r.run_id, r.tier, r.family, r.die, r.platform, r.commit_sha, r.tree_hash,
             r.ended_at,
             ROW_NUMBER() OVER (
               PARTITION BY r.tier, r.family, r.die, r.platform
               ORDER BY r.sort_at DESC, r.run_id DESC
             ) AS rn
      FROM ci_runs r
      WHERE r.verdict = 'PASS'
        AND (?1 IS NULL OR r.tier = ?1)
        AND (?2 IS NULL OR r.tree_hash = ?2)
        AND (?3 IS NULL OR r.family = ?3)
    ) x
    WHERE x.rn = 1`,

  ciRunById: `
    SELECT r.run_id, r.commit_sha, r.tree_hash, r.tree_kind, r.tier, r.profile, r.family, r.die,
           r.platform, r.verdict, r.started_at, r.ended_at, r.log_key, r.source,
           f.gpu_name, f.driver_versions_json
    FROM ci_runs r LEFT JOIN ci_host_facts f ON f.run_id = r.run_id
    WHERE r.run_id = ?1`,

  ciRunStages: `
    SELECT s.ord, s.name, s.rc, s.duration_ms, s.result
    FROM ci_run_stages s WHERE s.run_id = ?1 ORDER BY s.ord`,

  // Runs of one result key (?1), or of one commit (?2), newest first; ?3 page size.
  ciRunList: `
    SELECT r.run_id, r.commit_sha, r.tree_hash, r.tree_kind, r.tier, r.profile, r.family, r.die,
           r.platform, r.verdict, r.started_at, r.ended_at
    FROM ci_runs r
    WHERE (?1 IS NULL OR r.tree_hash = ?1)
      AND (?2 IS NULL OR r.commit_sha = ?2)
    ORDER BY r.sort_at DESC, r.run_id DESC
    LIMIT ?3`,

  // Result keys a commit was recorded under.
  ciCommitTrees: `
    SELECT r.tree_hash, r.tree_kind, COUNT(*) AS runs, MAX(r.sort_at) AS last_at
    FROM ci_runs r WHERE r.commit_sha = ?1
    GROUP BY r.tree_hash, r.tree_kind ORDER BY last_at DESC`,

  // Commits recorded under one result key (shown for information; see schema.js notes).
  ciTreeCommits: `
    SELECT r.commit_sha, COUNT(*) AS runs, MAX(r.sort_at) AS last_at
    FROM ci_runs r WHERE r.tree_hash = ?1
    GROUP BY r.commit_sha ORDER BY last_at DESC LIMIT 100`,

  // Setup picker (migration 0003). One row per (guest OS, die, CPU vendor, host driver, guest
  // driver) over FINAL runs. A run with platform 'both' counts once in each OS. Its FAIL is
  // attributed to the OS whose stage failed (stage names starting with 'windows' belong to
  // Windows); the other OS gets UNATTRIBUTED, which is neither a pass nor a fail of that setup.
  // CPU vendor and drivers are claims (ci_run_claims.claimed = 1); they only group and filter.
  // Filters: ?1 guest OS, ?2-?5 up to four dies, ?6 CPU vendor, ?7 driver (host or guest),
  // ?8 tier, ?9 reproduced rows wanted (1/0), ?10 community rows wanted (1/0; ci_runs.source
  // 'reported' or 'reviewed', reserved: none exist yet), ?11 report tier of a community row
  // ('unverified' = reported, 'reviewed'); ?12 = page size + 1.
  ciSetups: `
    WITH per_os AS (
      SELECT r.run_id, r.die, r.family, r.tier, r.verdict, r.commit_sha, r.tree_hash, r.ended_at,
             r.sort_at, o.os AS os, r.source AS source,
             CASE
               WHEN r.verdict = 'PASS' THEN 'PASS'
               WHEN r.verdict = 'FAIL' THEN
                 CASE
                   WHEN r.platform <> 'both' THEN 'FAIL'
                   WHEN NOT EXISTS (SELECT 1 FROM ci_run_stages s
                                    WHERE s.run_id = r.run_id AND s.result = 'FAIL') THEN 'FAIL'
                   WHEN EXISTS (SELECT 1 FROM ci_run_stages s
                                WHERE s.run_id = r.run_id AND s.result = 'FAIL'
                                  AND ((o.os = 'windows') = (s.name LIKE 'windows%'))) THEN 'FAIL'
                   ELSE 'UNATTRIBUTED'
                 END
               ELSE r.verdict
             END AS res,
             c.cpu_vendor, c.host_driver, c.guest_driver, f.gpu_name
      FROM ci_runs r
      JOIN (SELECT 'linux' AS os UNION ALL SELECT 'windows') o
        ON r.platform = o.os OR r.platform = 'both'
      LEFT JOIN ci_run_claims c ON c.run_id = r.run_id AND c.guest_os = o.os
      LEFT JOIN ci_host_facts f ON f.run_id = r.run_id
      WHERE r.verdict IS NOT NULL
        AND (?1 IS NULL OR o.os = ?1)
        AND (?2 IS NULL OR r.die IN (?2, ?3, ?4, ?5))
        AND (?6 IS NULL OR c.cpu_vendor = ?6)
        AND (?7 IS NULL OR c.host_driver = ?7 OR c.guest_driver = ?7)
        AND (?8 IS NULL OR r.tier = ?8)
        AND ((?9 = 1 AND r.source = 'reproduced')
             OR (?10 = 1 AND r.source IN ('reported', 'reviewed')))
        AND (?11 IS NULL OR (?11 = 'unverified' AND r.source = 'reported')
             OR (?11 = 'reviewed' AND r.source = 'reviewed'))
    ),
    ranked AS (
      SELECT p.run_id, p.die, p.family, p.tier, p.verdict, p.commit_sha, p.tree_hash, p.ended_at,
             p.sort_at, p.os, p.source, p.res, p.cpu_vendor, p.host_driver, p.guest_driver,
             p.gpu_name,
             ROW_NUMBER() OVER (
               PARTITION BY p.os, p.die, p.source, p.cpu_vendor, p.host_driver, p.guest_driver
               ORDER BY p.sort_at DESC, p.run_id DESC
             ) AS rn_latest,
             ROW_NUMBER() OVER (
               PARTITION BY p.os, p.die, p.source, p.cpu_vendor, p.host_driver, p.guest_driver,
                            p.res
               ORDER BY p.sort_at DESC, p.run_id DESC
             ) AS rn_result
      FROM per_os p
    )
    SELECT x.os, x.die, x.source, MAX(x.family) AS family, x.cpu_vendor, x.host_driver, x.guest_driver,
           COUNT(*) AS runs,
           SUM(CASE WHEN x.res = 'PASS' THEN 1 ELSE 0 END) AS passed,
           SUM(CASE WHEN x.res = 'FAIL' THEN 1 ELSE 0 END) AS failed,
           MAX(x.sort_at) AS last_at,
           MAX(CASE WHEN x.rn_latest = 1 THEN x.run_id END) AS latest_id,
           MAX(CASE WHEN x.rn_latest = 1 THEN x.res END) AS latest_result,
           MAX(CASE WHEN x.rn_latest = 1 THEN x.verdict END) AS latest_verdict,
           MAX(CASE WHEN x.rn_latest = 1 THEN x.commit_sha END) AS latest_commit,
           MAX(CASE WHEN x.rn_latest = 1 THEN x.tree_hash END) AS latest_tree,
           MAX(CASE WHEN x.rn_latest = 1 THEN x.tier END) AS latest_tier,
           MAX(CASE WHEN x.rn_latest = 1 THEN x.ended_at END) AS latest_ended_at,
           MAX(CASE WHEN x.rn_latest = 1 THEN x.gpu_name END) AS gpu_name,
           MAX(CASE WHEN x.res = 'PASS' AND x.rn_result = 1 THEN x.run_id END) AS pass_id,
           MAX(CASE WHEN x.res = 'PASS' AND x.rn_result = 1 THEN x.commit_sha END) AS pass_commit,
           MAX(CASE WHEN x.res = 'PASS' AND x.rn_result = 1 THEN x.ended_at END) AS pass_ended_at
    FROM ranked x
    GROUP BY x.os, x.die, x.source, x.cpu_vendor, x.host_driver, x.guest_driver
    ORDER BY last_at DESC, x.os, x.die, x.host_driver DESC, x.guest_driver DESC, x.cpu_vendor,
             x.source
    LIMIT ?12`,

  // Facets: the values that exist among final runs.
  ciFacetDies: `
    SELECT r.die, MAX(r.family) AS family, COUNT(*) AS runs, MAX(r.sort_at) AS last_at
    FROM ci_runs r WHERE r.verdict IS NOT NULL
    GROUP BY r.die ORDER BY family, r.die LIMIT 100`,

  ciFacetNames: `
    SELECT r.die, f.gpu_name, COUNT(*) AS runs
    FROM ci_runs r JOIN ci_host_facts f ON f.run_id = r.run_id
    WHERE r.verdict IS NOT NULL AND f.gpu_name IS NOT NULL
    GROUP BY r.die, f.gpu_name ORDER BY r.die, runs DESC, f.gpu_name LIMIT 300`,

  ciFacetSources: `
    SELECT r.source, COUNT(*) AS runs FROM ci_runs r
    WHERE r.verdict IS NOT NULL GROUP BY r.source ORDER BY r.source`,

  ciFacetPlatforms: `
    SELECT r.platform, COUNT(*) AS runs FROM ci_runs r
    WHERE r.verdict IS NOT NULL GROUP BY r.platform ORDER BY r.platform`,

  ciFacetTiers: `
    SELECT r.tier, COUNT(*) AS runs FROM ci_runs r
    WHERE r.verdict IS NOT NULL GROUP BY r.tier ORDER BY r.tier`,

  // ?1 names the claim column (an allow-listed constant chosen by the handler, never request text).
  ciFacetClaim: `
    SELECT CASE ?1
             WHEN 'cpu_vendor' THEN c.cpu_vendor
             WHEN 'cpu_arch' THEN c.cpu_arch
             WHEN 'host_driver' THEN c.host_driver
             WHEN 'guest_driver' THEN c.guest_driver
           END AS value,
           COUNT(DISTINCT c.run_id) AS runs
    FROM ci_run_claims c JOIN ci_runs r ON r.run_id = c.run_id
    WHERE r.verdict IS NOT NULL
    GROUP BY value HAVING value IS NOT NULL
    ORDER BY runs DESC, value DESC LIMIT 64`,

  hostCounts: `
    SELECT h.die, h.class, h.status, COUNT(*) AS hosts
    FROM hosts h
    GROUP BY h.die, h.class, h.status
    ORDER BY h.die, h.class, h.status`,
});
