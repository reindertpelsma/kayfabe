// Every SQL statement the public Worker can run: read-only, fixed text, numbered parameters.
// The database wrapper (db.js) accepts a statement NAME from this table, never SQL text.
// test/static.test.js refuses a statement that is not a single SELECT, any `${` or string
// concatenation in this file, and any reference to runs.tag, hosts_private or audit.
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

  hostCounts: `
    SELECT h.die, h.class, h.status, COUNT(*) AS hosts
    FROM hosts h
    GROUP BY h.die, h.class, h.status
    ORDER BY h.die, h.class, h.status`,
});
