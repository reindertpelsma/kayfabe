// SYNTHETIC development data for the platform database. Nothing here was measured: every
// commit message starts with "[synthetic]", every run id with "syn-", every host fact set has
// synthetic=true, and private host addresses come from the documentation range 192.0.2.0/24.
// Never load this into the production database (design section 12).
import { createHash } from 'node:crypto';

const FAMILIES = [
  ['turing', 'Turing', ['TU102', 'TU104', 'TU106', 'TU116']],
  ['ampere', 'Ampere', ['GA102', 'GA104', 'GA106']],
  ['ada', 'Ada Lovelace', ['AD102', 'AD103', 'AD104', 'AD106']],
  ['hopper', 'Hopper', ['GH100']],
  ['blackwell', 'Blackwell', ['GB202', 'GB203', 'GB205']],
];
const SUITE_STAGES = {
  'linux-gate': ['provision', 'v3_gates', 'merge_bar', 'cuda_guest'],
  'linux-fast': ['provision', 'fast_suite'],
  'linux-apps': ['provision', 'app_matrix'],
  'windows-gate': ['provision', 'windows_install', 'windows_gate'],
  'windows-apps': ['provision', 'windows_install', 'windows_apps'],
  'windows-specviewperf': ['provision', 'windows_install', 'specviewperf'],
};
const REQUIREMENTS = {
  'linux-gate': 'linux-gate',
  'linux-fast': 'linux-fast',
  'linux-apps': 'apps-middle',
  'windows-gate': 'windows-gate',
  'windows-apps': 'apps-largest',
  'windows-specviewperf': 'spec-largest',
};

function prng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const hex = (text) => createHash('sha256').update(text).digest('hex');
const time = (ms) => new Date(ms).toISOString().replace(/\.\d{3}Z$/, 'Z');
const BASE = Date.UTC(2026, 9, 1, 0, 0, 0);
const HOUR = 3600 * 1000;

function platformKey(fields) {
  return 'p-' + hex(fields.join('|')).slice(0, 24);
}

export function syntheticRows() {
  const random = prng(20261104);
  const pick = (list) => list[Math.floor(random() * list.length)];
  const rows = {
    families: [],
    dies: [],
    platforms: [],
    commits: [],
    branch_commits: [],
    runs: [],
    run_stages: [],
    requirement_results: [],
    hosts: [],
    hosts_private: [],
    audit: [],
    ci_runs: [],
    ci_run_stages: [],
    ci_host_facts: [],
    ci_run_claims: [],
    ci_runs_private: [],
    ci_requirement_snapshots: [],
  };
  const dies = [];
  FAMILIES.forEach(([id, name, list], f) => {
    rows.families.push({ id, name, ord: f + 1 });
    list.forEach((die, d) => {
      rows.dies.push({ id: die, family_id: id, name: die + ' (synthetic row)', ord: d + 1 });
      dies.push({ die, family: id });
    });
  });

  // Two platforms per die (Linux and Windows guest); some dies get a second host driver.
  const platforms = new Map();
  for (const [index, { die }] of dies.entries()) {
    const variants = [['580.159.04', index % 3 === 2 ? 'amd' : 'intel']];
    if (index % 4 === 1) variants.push(['575.64.05', 'amd']);
    for (const [driver, cpu] of variants) {
      for (const [os, guestDriver] of [
        ['linux', '580.159.04'],
        ['windows', '580.88'],
      ]) {
        const fields = [
          die,
          driver,
          cpu,
          cpu === 'amd' ? 'synthetic EPYC' : 'synthetic Xeon',
          '1',
          '6.8.0-synthetic',
          'kf3-synthetic',
          'nested-vfio',
          os,
          guestDriver,
        ];
        const key = platformKey(fields);
        rows.platforms.push({
          key,
          die,
          host_driver: driver,
          cpu_vendor: cpu,
          cpu_model: fields[3],
          nested: 1,
          kernel: fields[5],
          qemu_build: fields[6],
          iommu_mode: fields[7],
          guest_os: os,
          guest_driver: guestDriver,
        });
        const list = platforms.get(die + '|' + os) ?? [];
        list.push(key);
        platforms.set(die + '|' + os, list);
      }
    }
  }

  // 24 commits on next; every 6th is merged to master. Every third commit is docs-only and
  // keeps the previous boundary hash.
  let boundary = hex('synthetic-boundary-0');
  const commits = [];
  for (let seq = 1; seq <= 24; seq++) {
    const docsOnly = seq % 3 === 0;
    if (!docsOnly) boundary = hex('synthetic-boundary-' + seq);
    const sha = hex('synthetic-commit-' + seq).slice(0, 40);
    const at = BASE + seq * 6 * HOUR;
    const commit = {
      sha,
      boundary_hash: boundary,
      message: '[synthetic] commit ' + seq + (docsOnly ? ': docs only' : ': code change'),
      author: 'synthetic',
      committed_at: time(at),
      pushed_at: time(at + 5 * 60 * 1000),
    };
    rows.commits.push(commit);
    rows.branch_commits.push({ branch: 'next', seq, sha });
    if (seq % 6 === 0) rows.branch_commits.push({ branch: 'master', seq: seq / 6, sha });
    commits.push({ ...commit, seq, at, docsOnly });
  }

  const latestByBoundary = new Map();
  for (const commit of commits) {
    if (commit.docsOnly) continue; // docs-only commits fall through to the earlier answer
    const chosen = new Set();
    while (chosen.size < 3) chosen.add(pick(dies).die);
    let minute = 0;
    for (const die of chosen) {
      const family = dies.find((d) => d.die === die).family;
      for (const suite of Object.keys(SUITE_STAGES)) {
        const windows = suite.startsWith('windows');
        const chance = { 'linux-gate': 1, 'linux-fast': 1, 'linux-apps': 0.5 }[suite] ?? 0;
        const roll = random();
        if (!windows && roll >= chance) continue;
        if (windows && roll >= (suite === 'windows-gate' ? 0.6 : 0.2)) continue;
        if (family === 'hopper' && windows) continue;
        const keys = platforms.get(die + '|' + (windows ? 'windows' : 'linux'));
        const platform = keys[Math.floor(random() * keys.length)];
        const started = commit.at + HOUR + minute * 60 * 1000;
        minute += 7;
        const inFlight = commit.seq >= 23 && random() < 0.5;
        let verdict = null;
        if (!inFlight) {
          const v = random();
          verdict = v < 0.8 ? 'PASS' : v < 0.9 ? 'FAIL' : v < 0.95 ? 'RETRY' : 'UNAVAILABLE';
          if (family === 'blackwell' && windows) verdict = 'UNAVAILABLE';
        }
        const finished = inFlight ? null : time(started + (20 + Math.floor(random() * 90)) * 60000);
        const tier =
          suite === 'windows-specviewperf' || suite === 'windows-apps'
            ? 'largest'
            : suite === 'windows-gate'
              ? 'middle'
              : pick(['middle', 'middle', 'small', 'adhoc']);
        const id = 'syn-' + commit.seq + '-' + die.toLowerCase() + '-' + suite;
        const scores = suite.endsWith('apps')
          ? { glmark2: round(0.8 + random() * 0.2), vkmark: round(0.7 + random() * 0.3) }
          : suite === 'windows-specviewperf'
            ? { 'snx-04': round(0.5 + random() * 0.4) }
            : null;
        rows.runs.push({
          id,
          commit_sha: commit.sha,
          boundary_hash: commit.boundary_hash,
          requirement_key: REQUIREMENTS[suite],
          platform_key: platform,
          die,
          suite,
          verdict,
          scores_json: scores ? JSON.stringify(scores) : null,
          host_facts_json: JSON.stringify({
            synthetic: true,
            gpu_model: 'synthetic ' + die + ' board',
            ram_gib: 64,
            disk_gb: 120,
          }),
          logs_json: JSON.stringify(['box.log', windows ? 'windows.txt' : 'guest.txt']),
          started_at: time(started),
          finished_at: finished,
          sort_at: finished ?? time(started),
          source: 'reproduced',
          tier,
          tag: 'synthetic-tag-' + hex(id).slice(0, 16),
        });
        const stages = SUITE_STAGES[suite];
        const failAt = verdict === 'FAIL' ? stages.length - 1 : -1;
        stages.forEach((name, ord) => {
          let stageVerdict = inFlight ? (ord === 0 ? 'PASS' : null) : 'PASS';
          if (verdict === 'FAIL' && ord === failAt) stageVerdict = 'FAIL';
          if ((verdict === 'RETRY' || verdict === 'UNAVAILABLE') && ord > 0)
            stageVerdict = 'SKIPPED';
          if ((verdict === 'RETRY' || verdict === 'UNAVAILABLE') && ord === 0)
            stageVerdict = verdict;
          rows.run_stages.push({
            run_id: id,
            ord,
            name,
            verdict: stageVerdict,
            started_at: time(started + ord * 5 * 60000),
            finished_at: stageVerdict === null ? null : time(started + (ord + 1) * 5 * 60000),
          });
        });
        const prior = latestByBoundary.get(commit.boundary_hash) ?? [];
        prior.push({ id, suite, verdict, sortAt: finished ?? time(started) });
        latestByBoundary.set(commit.boundary_hash, prior);
      }
    }
  }

  // The CI Worker's answer per commit and requirement, shared across equal boundary hashes.
  for (const commit of commits) {
    const answers = latestByBoundary.get(commit.boundary_hash) ?? [];
    const byRequirement = new Map();
    for (const run of answers) {
      const key = REQUIREMENTS[run.suite];
      const best = byRequirement.get(key);
      if (!best || run.sortAt > best.sortAt) byRequirement.set(key, run);
    }
    for (const [key, run] of [...byRequirement].sort()) {
      const state =
        run.verdict === null
          ? 'PENDING'
          : run.verdict === 'PASS' || run.verdict === 'FAIL'
            ? run.verdict
            : 'UNAVAILABLE';
      rows.requirement_results.push({
        commit_sha: commit.sha,
        requirement_key: key,
        state,
        run_id: run.id,
        updated_at: run.sortAt,
      });
    }
  }

  for (let n = 1; n <= 14; n++) {
    const id = 'syn-host-' + n;
    const enrolled = n > 12;
    rows.hosts.push({
      id,
      class: enrolled ? 'enrolled' : 'vast',
      die: pick(dies).die,
      status: pick(['online', 'online', 'offline', 'retired']),
      trust_level: 1,
      rentable_for_ci: enrolled ? 0 : 1,
      remote_access: enrolled ? 1 : 0,
      last_seen_at: time(BASE + 24 * 6 * HOUR),
    });
    rows.hosts_private.push({
      host_id: id,
      address: '192.0.2.' + n,
      box_token_sha256: hex('synthetic-box-token-' + n),
      provider_machine_id: 'synthetic-machine-' + n,
    });
  }
  rows.audit.push({
    at: time(BASE),
    actor: 'synthetic',
    action: 'seed',
    detail: 'synthetic private audit row',
  });
  ciRows(rows, dies, commits);
  return rows;
}

// CI Worker result store (migration 0002), SYNTHETIC. Run ids start with "syn-ci-"; every tree
// hash is derived from "synthetic-tree-"; a separate PRNG keeps the 0001 rows (and the status
// fixture built from them) unchanged. Two commits share one tree hash (a docs-only change).
function ciRows(rows, dies, commits) {
  const random = prng(20261105);
  const pick = (list) => list[Math.floor(random() * list.length)];
  const consumer = dies.filter((d) => d.family !== 'hopper');
  let tree = null;
  commits.slice(-12).forEach((commit, i) => {
    if (!commit.docsOnly || tree === null) tree = hex('synthetic-tree-' + commit.seq);
    const last = i === 11;
    const cells = [['middle', 'both']];
    if (i % 4 === 0) cells.push(['small', 'linux']);
    if (i === 6) cells.push(['largest', 'both'], ['largest', 'both']);
    cells.forEach(([tier, platform], j) => {
      const { die, family } = pick(consumer);
      const id = 'syn-ci-' + commit.seq + '-' + j;
      const started = commit.at + 2 * HOUR + j * 20 * 60000;
      const inFlight = last && j === 0;
      const roll = random();
      const verdict = inFlight
        ? null
        : i === 3 && j === 0
          ? 'FAIL'
          : roll < 0.75
            ? 'PASS'
            : roll < 0.88
              ? 'FAIL'
              : roll < 0.94
                ? 'RETRY'
                : 'UNATTRIBUTED';
      const ended = inFlight ? null : time(started + (30 + Math.floor(random() * 60)) * 60000);
      rows.ci_runs.push({
        run_id: id,
        commit_sha: commit.sha,
        tree_hash: tree,
        tree_kind: 'tree',
        tier,
        profile: tier === 'middle' ? 'check' : 'synthetic',
        family,
        die,
        platform,
        verdict,
        started_at: time(started),
        ended_at: ended,
        sort_at: ended ?? time(started),
        log_key: null,
        source: 'reproduced',
        tag: verdict === null ? null : 'synthetic-tag-' + hex(id).slice(0, 16),
        recorded_at: ended ?? time(started),
      });
      if (verdict === 'PASS' || verdict === 'FAIL') {
        const names =
          platform === 'both' ? ['provision', 'gates', 'windows_gate'] : ['provision', 'gates'];
        names.forEach((name, ord) =>
          rows.ci_run_stages.push({
            run_id: id,
            ord,
            name,
            rc: verdict === 'FAIL' && ord === names.length - 1 ? 1 : 0,
            duration_ms: 60000 * (ord + 1),
            result: verdict === 'FAIL' && ord === names.length - 1 ? 'FAIL' : 'PASS',
          }),
        );
      }
      rows.ci_host_facts.push({
        run_id: id,
        gpu_name: 'synthetic ' + die,
        driver_versions_json: JSON.stringify({ guest: '580.159.04', host: '580.95.05' }),
      });
      rows.ci_runs_private.push({
        run_id: id,
        machine_id: 900000 + rows.ci_runs.length,
        price_usd_h: 0.123,
        cost_usd: 0.456,
      });
    });
  });
  claimRows(rows);
  pickerRows(rows, commits);
  rows.ci_requirement_snapshots.push({
    sha256: hex('synthetic requirements'),
    base_ref: 'next',
    content: '# synthetic requirements snapshot\nversion = 1\n',
    first_seen_at: time(BASE),
  });
}

// Host facts a synthetic box "claimed" (migration 0003). A separate PRNG keeps every other row
// unchanged. About one run in seven has no claims at all (an older box).
const SYN_HOST_DRIVERS = ['580.95.05', '575.64.05', '570.172.08'];
const SYN_WINDOWS_DRIVER = '580.88';
const GUEST_OS = { linux: 'ubuntu-24.04', windows: '11-ltsc-2024-26100' };
function claim(runId, os, { cpu, host, model = null }) {
  const vendor = cpu ?? null;
  return {
    run_id: runId,
    guest_os: os,
    guest_os_version: GUEST_OS[os],
    guest_driver: os === 'windows' ? SYN_WINDOWS_DRIVER : host,
    host_driver: host,
    cpu_vendor: vendor,
    cpu_model:
      model ??
      (vendor === 'amd'
        ? 'synthetic AMD EPYC'
        : vendor === 'intel'
          ? 'synthetic Intel Xeon'
          : null),
    cpu_arch: vendor === null ? null : 'x86_64',
    claimed: 1,
  };
}
const osesOf = (platform) => (platform === 'both' ? ['linux', 'windows'] : [platform]);

function claimRows(rows) {
  const random = prng(20261106);
  for (const run of rows.ci_runs) {
    if (run.verdict === null) continue;
    const roll = random();
    const host = SYN_HOST_DRIVERS[Math.floor(random() * SYN_HOST_DRIVERS.length)];
    if (roll < 0.14) continue;
    const cpu = roll < 0.57 ? 'amd' : 'intel';
    for (const os of osesOf(run.platform))
      rows.ci_run_claims.push(claim(run.run_id, os, { cpu, host }));
  }
}

// Extra runs of two dies so the setup picker has several groups to show: two CPU vendors, two
// host drivers, a FAIL in the Windows stage, a FAIL in a Linux stage, a RETRY and a run from a box
// that reported no claims. Hand-written (no PRNG), ids "syn-ci-p-<n>".
function pickerRows(rows, commits) {
  const recent = commits.slice(-12);
  const specs = [
    // [commit index, die, family, platform, tier, verdict, failing stage, host driver, cpu]
    [0, 'GA106', 'ampere', 'both', 'middle', 'PASS', null, '580.95.05', 'amd'],
    [2, 'GA106', 'ampere', 'both', 'middle', 'PASS', null, '580.95.05', 'intel'],
    [4, 'GA106', 'ampere', 'both', 'middle', 'FAIL', 'windows_gate', '580.95.05', 'amd'],
    [5, 'GA106', 'ampere', 'linux', 'small', 'PASS', null, '575.64.05', 'amd'],
    [7, 'GA106', 'ampere', 'both', 'middle', 'FAIL', 'gates', '575.64.05', 'intel'],
    [9, 'GA106', 'ampere', 'both', 'middle', 'RETRY', null, '580.95.05', 'amd'],
    [1, 'GA104', 'ampere', 'both', 'middle', 'PASS', null, '580.95.05', 'intel'],
    [3, 'GA104', 'ampere', 'linux', 'small', 'PASS', null, '575.64.05', null],
    [8, 'GA104', 'ampere', 'both', 'largest', 'PASS', null, '580.95.05', 'amd'],
    [6, 'GA106', 'ampere', 'linux', 'small', 'PASS', null, null, 'none'],
  ];
  specs.forEach(([ci, die, family, platform, tier, verdict, failing, host, cpu], n) => {
    const commit = recent[ci];
    const id = 'syn-ci-p-' + n;
    const started = commit.at + 3 * HOUR + n * 7 * 60000;
    const ended = time(started + 45 * 60000);
    rows.ci_runs.push({
      run_id: id,
      commit_sha: commit.sha,
      // The tree the commit's other synthetic runs already use (a docs-only commit shares it).
      tree_hash: rows.ci_runs.find((r) => r.commit_sha === commit.sha).tree_hash,
      tree_kind: 'tree',
      tier,
      profile: tier === 'middle' ? 'check' : 'synthetic',
      family,
      die,
      platform,
      verdict,
      started_at: time(started),
      ended_at: ended,
      sort_at: ended,
      log_key: null,
      source: 'reproduced',
      tag: 'synthetic-tag-' + hex(id).slice(0, 16),
      recorded_at: ended,
    });
    if (verdict === 'PASS' || verdict === 'FAIL') {
      const names =
        platform === 'both' ? ['provision', 'gates', 'windows_gate'] : ['provision', 'gates'];
      const cut = failing === null ? names.length : names.indexOf(failing) + 1;
      names.slice(0, cut).forEach((name, ord) =>
        rows.ci_run_stages.push({
          run_id: id,
          ord,
          name,
          rc: name === failing ? 1 : 0,
          duration_ms: 60000 * (ord + 1),
          result: name === failing ? 'FAIL' : 'PASS',
        }),
      );
    }
    rows.ci_host_facts.push({
      run_id: id,
      gpu_name: 'synthetic ' + die,
      driver_versions_json: JSON.stringify({ guest: host ?? '580.159.04', host: '580.95.05' }),
    });
    rows.ci_runs_private.push({
      run_id: id,
      machine_id: 910000 + n,
      price_usd_h: 0.123,
      cost_usd: 0.456,
    });
    if (host !== null)
      for (const os of osesOf(platform))
        rows.ci_run_claims.push(claim(id, os, { cpu: cpu === 'none' ? null : cpu, host }));
  });
}

function round(value) {
  return Math.round(value * 1000) / 1000;
}

// Write rows with node:sqlite (tests) ...
export function applyRows(database, rows) {
  database.exec('BEGIN');
  for (const [table, list] of Object.entries(rows)) {
    for (const row of list) {
      const columns = Object.keys(row);
      const statement = database.prepare(
        'INSERT INTO ' +
          table +
          ' (' +
          columns.join(', ') +
          ') VALUES (' +
          columns.map(() => '?').join(', ') +
          ')',
      );
      statement.run(...columns.map((c) => row[c]));
    }
  }
  database.exec('COMMIT');
}

// ... or as a SQL file for `wrangler d1 execute kayfabe-platform --local --file seed.sql`.
export function toSql(rows) {
  const literal = (value) =>
    value === null
      ? 'NULL'
      : typeof value === 'number'
        ? String(value)
        : "'" + String(value).replaceAll("'", "''") + "'";
  const lines = ['-- SYNTHETIC development data (ci/platform-api/scripts/seed-data.mjs).'];
  lines.push('-- Not measured. Never apply to the production database.');
  for (const [table, list] of Object.entries(rows)) {
    for (const row of list) {
      const columns = Object.keys(row);
      lines.push(
        'INSERT INTO ' +
          table +
          ' (' +
          columns.join(', ') +
          ') VALUES (' +
          columns.map((c) => literal(row[c])).join(', ') +
          ');',
      );
    }
  }
  return lines.join('\n') + '\n';
}
