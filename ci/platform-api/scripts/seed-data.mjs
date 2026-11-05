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
  return rows;
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
