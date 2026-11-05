// Status pages in jsdom against the committed SYNTHETIC fixture (status/fixture.json) and the
// committed evidence file (status/coverage.json).
// FALSIFIER: a page fails to render; API or evidence text becomes markup; live mode asks anything
// but the configured API origin (or the same-origin evidence file), or sends credentials; an invalid
// query parameter reaches a request; imported evidence is drawn as a gate verdict or without its
// source, revision and date; status.js holds a URL literal or uses an HTML sink.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { JSDOM } from 'jsdom';

const site = fileURLToPath(new URL('.', import.meta.url));
const code = readFileSync(join(site, 'status/status.js'), 'utf8');
const fixture = JSON.parse(readFileSync(join(site, 'status/fixture.json'), 'utf8'));
const coverage = JSON.parse(readFileSync(join(site, 'status/coverage.json'), 'utf8'));
const API = 'https://api.kayfabe.dev';
const REPO = 'https://github.com/reindertpelsma/kayfabe';

function write(mode) {
  const out = mkdtempSync(join(tmpdir(), 'kf-status-dom-'));
  const args =
    mode === 'fixture'
      ? `fixture=${JSON.stringify(join(site, 'status/fixture.json'))}`
      : `api_base=${JSON.stringify(API)}, logs_base="https://logs.kayfabe.dev"`;
  const run = spawnSync(
    'python3',
    [
      '-c',
      `import sys; sys.path.insert(0, ${JSON.stringify(site)}); import status_pages; ` +
        `print(status_pages.write_status(${JSON.stringify(out)}, ${args}))`,
    ],
    { encoding: 'utf8' },
  );
  assert.equal(run.status, 0, run.stderr);
  assert.equal(run.stdout.trim(), mode);
  return out;
}
const pages = { fixture: write('fixture'), live: write('live') };

async function render(mode, path, search = '', opts = {}) {
  const responses = opts.responses ?? fixture.responses;
  const evidence = opts.coverage === undefined ? coverage : opts.coverage;
  const name = path ? path + '/index.html' : 'index.html';
  const html = readFileSync(join(pages[mode], 'status', name), 'utf8');
  const url = 'https://kayfabe.dev/status/' + (path ? path + '/' : '') + search;
  const dom = new JSDOM(html, { url, runScripts: 'outside-only' });
  const { window } = dom;
  const requests = [];
  window.fetch = async (target, init) => {
    const href = String(target);
    requests.push({ href, init });
    let body;
    if (href === 'https://kayfabe.dev/status/fixture.json') body = { synthetic: true, responses };
    else if (href === 'https://kayfabe.dev/status/coverage.json') {
      if (evidence === null) return { ok: false, status: 404, json: async () => ({}) };
      body = evidence;
    } else if (href.startsWith(API + '/')) {
      const key = href.slice(API.length);
      if (!Object.hasOwn(responses, key)) return { ok: false, status: 404, json: async () => ({}) };
      body = responses[key];
    } else throw new Error('unexpected request ' + href);
    return { ok: true, status: 200, json: async () => structuredClone(body) };
  };
  Object.defineProperty(window.document, 'currentScript', {
    value: { src: 'https://kayfabe.dev/status/status.js' },
  });
  window.eval(code);
  for (let i = 0; i < 50; i++) {
    await new Promise((resolve) => setTimeout(resolve, 5));
    if (window.document.getElementById('kf-status-state').textContent !== 'Loading.') break;
  }
  return { window, document: window.document, requests };
}

const ciRuns = Object.entries(fixture.responses).filter(([k]) => k.startsWith('/v1/ci/runs/'));
const passRun = ciRuns.map(([, v]) => v.run).find((r) => r.verdict === 'PASS' && r.stages.length);
const failRun = ciRuns.map(([, v]) => v.run).find((r) => r.verdict === 'FAIL');
const runningRun = ciRuns.map(([, v]) => v.run).find((r) => r.state === 'running');
const treeKey = Object.keys(fixture.responses).find((k) =>
  /^\/v1\/ci\/trees\/[0-9a-f]{64}$/.test(k),
);
const commitKey = Object.keys(fixture.responses).find((k) => k.startsWith('/v1/ci/commits/'));
const dieCount = coverage.families.reduce((n, f) => n + f.dies.length, 0);
const text = (node) => node.textContent.replace(/\s+/g, ' ');

test('overview: families as sections, dies as rows, Linux and Windows columns, the gate rule', async () => {
  const { document } = await render('fixture', '');
  assert.match(document.querySelector('.kf-synthetic').textContent, /Synthetic data\./);
  const sections = [...document.querySelectorAll('section.kf-family-section')];
  assert.deepEqual(
    sections.map((s) => s.querySelector('h2').textContent),
    coverage.families.map((f) => f.name),
  );
  const rows = document.querySelectorAll('.kf-matrix tbody tr');
  assert.equal(rows.length, dieCount);
  for (const row of rows) assert.equal(row.querySelectorAll('td.kf-cell').length, 2);
  const heads = [...document.querySelectorAll('.kf-matrix thead')[0].querySelectorAll('th')];
  assert.deepEqual(
    heads.map((h) => h.textContent),
    ['Die', 'Linux guest', 'Windows guest'],
  );
  assert.match(
    text(document.querySelector('.kf-rule')),
    /Only runs reproduced by the CI Worker count for the release gate/,
  );
  assert.match(text(document.querySelector('.kf-legend-box')), /never counts for the release gate/);
  assert.equal(document.querySelectorAll('.kf-tabs a').length, 4);
  assert.equal(
    document.querySelector('.kf-tabs a[aria-current="page"]').textContent,
    'Largest (release)',
  );
  const hrefs = [...document.querySelectorAll('#kf-status-content a')].map((a) => a.href);
  assert.ok(hrefs.length > 80, String(hrefs.length));
  for (const href of hrefs)
    assert.ok(href.startsWith('https://kayfabe.dev/status/') || href.startsWith(REPO + '/'), href);
});

test('every covered entry shows source, revision, date, counts and an evidence link', async () => {
  const { document } = await render('fixture', '');
  const blocks = [...document.querySelectorAll('.kf-block-imported, .kf-block-worker')];
  const covered = coverage.families.flatMap((f) =>
    f.dies.flatMap((d) => Object.values(d.cells).filter((c) => c.state === 'covered')),
  );
  assert.equal(blocks.length, covered.length);
  for (const block of blocks) {
    const t = text(block);
    if (block.classList.contains('kf-block-imported')) {
      assert.match(t, /Direct Vast run, evidence in repo \(imported\)/);
      assert.doesNotMatch(t, /CI Worker run/);
    } else {
      assert.match(t, /CI Worker run \(reproduced\)/);
      assert.doesNotMatch(t, /Direct Vast run/);
    }
    assert.match(t, /revision [0-9a-f]{8}/);
    assert.match(t, /20\d\d-\d\d-\d\d/);
    assert.match(t, /\d+ pass, \d+ fail of \d+ runs?/);
    const links = [...block.querySelectorAll('a')].map((a) => a.href);
    assert.ok(
      links.some((h) => h.startsWith(REPO + '/commit/')),
      t,
    );
    assert.ok(
      links.some((h) => h.startsWith(REPO + '/tree/next/traces/')),
      t,
    );
  }
});

test('imported evidence is never drawn as a gate pass', async () => {
  const { document } = await render('fixture', '');
  for (const block of document.querySelectorAll('.kf-block-imported')) {
    assert.equal(block.querySelector('.kf-v-pass, .kf-v-fail, .kf-v-retry'), null);
    assert.ok(block.querySelector('.kf-imp'));
    assert.doesNotMatch(text(block), /\bPASS\b/);
  }
  // The one Worker-run entry in the evidence file is labelled as such and names its run.
  const ga102 = [...document.querySelectorAll('.kf-matrix tbody tr')].find((r) =>
    r.querySelector('th').textContent.startsWith('GA102'),
  );
  const windows = ga102.querySelectorAll('td')[1];
  assert.match(text(windows), /CI Worker run \(reproduced\)/);
  assert.match(text(windows), /d74f6063/);
  assert.match(text(windows), /2026-11-05/);
  assert.ok(
    [...windows.querySelectorAll('a')].some((a) => a.href === REPO + '/actions/runs/41936606326'),
  );
  // A die with a failed run says so, in words, with the README's note.
  const ga106 = [...document.querySelectorAll('.kf-matrix tbody tr')].find((r) =>
    r.querySelector('th').textContent.startsWith('GA106'),
  );
  assert.match(text(ga106.querySelectorAll('td')[1]), /1 pass, 1 fail of 2 runs/);
  assert.match(text(ga106.querySelectorAll('td')[1]), /gate-harness defect/);
});

test('the gate rule is stated even when the evidence file lacks it', async () => {
  const bare = structuredClone(coverage);
  delete bare.gate_rule;
  const { document } = await render('fixture', '', '', { coverage: bare });
  assert.match(text(document.querySelector('.kf-rule')), /Imported evidence never does/);
});

test('uncovered dies say not covered, with the reason the repository states', async () => {
  const { document } = await render('fixture', '');
  const row = [...document.querySelectorAll('.kf-matrix tbody tr')].find((r) =>
    r.querySelector('th').textContent.startsWith('TU104'),
  );
  for (const cell of row.querySelectorAll('td')) {
    assert.match(text(cell), /not covered/);
    assert.match(text(cell), /No Vast VM offer/);
  }
});

test('a live run over both platforms appears in both columns, per tier', async () => {
  const { document } = await render('fixture', '', '?tier=middle');
  assert.equal(
    document.querySelector('.kf-tabs a[aria-current="page"]').textContent,
    'Middle (master PR)',
  );
  const cells = [...document.querySelectorAll('.kf-block-live')].filter((b) =>
    /one run over Linux and Windows/.test(text(b)),
  );
  assert.ok(cells.length >= 2);
  const chips = cells.map((c) => c.querySelector('.kf-v').textContent);
  assert.ok(chips.includes('PASS'));
  const links = [...document.querySelectorAll('.kf-block-live a')].map((a) => a.href);
  assert.ok(
    links.every((h) => h.startsWith('https://kayfabe.dev/status/run/') || h.startsWith(REPO)),
  );
  assert.ok(document.querySelectorAll('section .kf-runs tbody tr').length >= 1);
  assert.match(text(document.getElementById('kf-status-state')), /Middle \(master PR\) tier/);
});

test('die page lists every imported revision and the live rows', async () => {
  const { document } = await render('fixture', 'die', '?family=ada&die=AD102');
  assert.equal(document.querySelector('h1').textContent, 'AD102 (RTX 4090)');
  const windows = [...document.querySelectorAll('section')].find((s) =>
    /Windows guest/.test(s.querySelector('h2').textContent),
  );
  assert.equal(windows.querySelectorAll('.kf-block-imported').length, 5);
  assert.match(text(windows), /experimental branch revision/);
  assert.match(text(windows), /runs ada102a, ada102b/);
  const other = await render('fixture', 'die', '?family=turing&die=TU104');
  assert.ok(other.document.querySelectorAll('.kf-runs tbody tr').length >= 1);
  assert.match(text(other.document.getElementById('kf-status-content')), /not covered/);
});

test('run page: summary, source, stages, log state', async () => {
  const run = await render('fixture', 'run', '?id=' + passRun.id);
  assert.equal(run.document.querySelector('h1').textContent, 'Run ' + passRun.id);
  const content = text(run.document.getElementById('kf-status-content'));
  assert.match(content, /Source\s*CI Worker run \(reproduced\)/);
  assert.match(content, /Result key/);
  assert.ok(run.document.querySelectorAll('.kf-runs tbody tr').length >= 2);
  assert.match(content, /No public log\./);
  const fail = await render('fixture', 'run', '?id=' + failRun.id);
  assert.ok(fail.document.querySelector('.kf-runs .kf-v-fail'));
  const running = await render('fixture', 'run', '?id=' + runningRun.id);
  assert.match(text(running.document.getElementById('kf-status-state')), /still in flight/);
  assert.match(text(running.document.getElementById('kf-status-content')), /in flight/);
});

test('commit and result key pages', async () => {
  const sha = commitKey.split('/')[4];
  const commit = await render('fixture', 'commit', '?sha=' + sha);
  assert.equal(commit.document.querySelector('h1').textContent, 'Commit ' + sha.slice(0, 10));
  assert.ok(commit.document.querySelectorAll('.kf-runs tbody tr').length >= 1);
  assert.ok(commit.document.querySelector('a[href*="?tree="], a[href*="tree="]'));
  const hash = treeKey.split('/')[4];
  const tree = await render('fixture', 'commit', '?tree=' + hash);
  assert.equal(tree.document.querySelector('h1').textContent, 'Result key ' + hash.slice(0, 12));
  assert.match(
    text(tree.document.getElementById('kf-status-content')),
    /Commits recorded under this key/,
  );
});

test('API and evidence text is rendered as text, never as markup', async () => {
  const hostile = structuredClone(fixture.responses);
  const evil = '<img src=x onerror="window.pwned=1">';
  const key = '/v1/ci/runs/' + passRun.id;
  hostile[key].run.host.gpu_name = evil;
  hostile[key].run.host.drivers = { guest: evil };
  hostile[key].run.stages[0].name = evil;
  hostile[key].run.profile = evil;
  const run = await render('fixture', 'run', '?id=' + passRun.id, { responses: hostile });
  assert.equal(run.document.querySelector('#kf-status-content img'), null);
  assert.ok(run.document.getElementById('kf-status-content').textContent.includes(evil));
  assert.equal(run.window.pwned, undefined);
  const bad = structuredClone(coverage);
  const die = bad.families[0].dies[0];
  die.board = evil;
  die.cells.linux.reason = evil;
  bad.gate_rule = evil;
  bad.families[2].dies[0].cells.windows.groups[0].notes = [evil];
  bad.families[2].dies[0].cells.windows.groups[0].detail = evil;
  const matrix = await render('fixture', '', '', { coverage: bad });
  assert.equal(matrix.document.querySelector('#kf-status-content img'), null);
  assert.ok(matrix.document.getElementById('kf-status-content').textContent.includes(evil));
  assert.equal(matrix.window.pwned, undefined);
});

test('evidence links and revisions that do not look like a path or a hash are not links', async () => {
  const bad = structuredClone(coverage);
  const group = bad.families[3].dies[0].cells.windows.groups[0];
  group.evidence = '../../etc/passwd';
  group.revision = 'javascript:alert(1)';
  group.revision_full = null;
  group.run = { label: 'x', path: 'https://evil.example/' };
  const { document } = await render('fixture', '', '', { coverage: bad });
  const hrefs = [...document.querySelectorAll('#kf-status-content a')].map((a) => a.href);
  assert.ok(!hrefs.some((h) => /evil|passwd|javascript/.test(h)));
});

test('live mode asks only the configured API origin and the evidence file, without credentials', async () => {
  const live = await render('live', '');
  assert.ok(live.requests.length >= 3);
  for (const { href, init } of live.requests) {
    assert.ok(
      href.startsWith(API + '/v1/ci/') || href === 'https://kayfabe.dev/status/coverage.json',
      href,
    );
    assert.equal(init.credentials, 'omit');
  }
  assert.ok(live.requests.some((r) => r.href === API + '/v1/ci/matrix?tier=largest'));
  assert.equal(live.document.querySelector('.kf-synthetic'), null);
  const tier = await render('live', '', '?tier=small');
  assert.ok(tier.requests.some((r) => r.href === API + '/v1/ci/matrix?tier=small'));
});

test('the log link points at the configured log origin only', async () => {
  const key = '/v1/ci/runs/' + passRun.id;
  const withLog = (log) => {
    const responses = structuredClone(fixture.responses);
    responses[key].run.log = log;
    return responses;
  };
  const logKey = 'runs/' + passRun.id + '/run.log';
  const ok = await render('live', 'run', '?id=' + passRun.id, {
    responses: withLog({ key: logKey, direct: 'https://logs.kayfabe.dev/' + logKey }),
  });
  assert.equal(
    ok.document.querySelector('a[type="text/plain"]').href,
    'https://logs.kayfabe.dev/' + logKey,
  );
  const evil = await render('live', 'run', '?id=' + passRun.id, {
    responses: withLog({ key: logKey, direct: 'https://evil.example/' + logKey }),
  });
  assert.equal(evil.document.querySelector('a[type="text/plain"]'), null);
  const badKey = await render('live', 'run', '?id=' + passRun.id, {
    responses: withLog({ key: '../../x.log' }),
  });
  assert.match(text(badKey.document.getElementById('kf-status-content')), /No public log\./);
});

test('without the evidence file the live rows still render, with a note', async () => {
  const { document } = await render('fixture', '', '?tier=middle', { coverage: null });
  assert.match(
    text(document.getElementById('kf-status-content')),
    /imported-evidence file could not be loaded/,
  );
  assert.ok(document.querySelectorAll('.kf-block-live').length >= 1);
});

test('invalid page parameters never reach a request', async () => {
  for (const [path, search] of [
    ['run', '?id=%3Cscript%3E'],
    ['run', '?id=../../v1/hosts'],
    ['die', '?family=ada&die=..%2Fx'],
    ['die', '?family=ADA&die=AD102'],
    ['die', ''],
    ['commit', '?sha=HEAD'],
    ['commit', '?tree=' + 'g'.repeat(64)],
    ['commit', ''],
  ]) {
    const page = await render('live', path, search);
    assert.equal(page.requests.length, 0, path + search);
    assert.match(page.document.getElementById('kf-status-state').textContent, /needs/);
  }
  const tier = await render('live', '', '?tier=../../v1/hosts');
  assert.ok(tier.requests.every((r) => !r.href.includes('hosts')));
  assert.ok(tier.requests.some((r) => r.href === API + '/v1/ci/matrix?tier=largest'));
});

test('status.js has no URL literal and no HTML sink', () => {
  assert.doesNotMatch(code, /https?:\/\/|kayfabe\.dev|api\./);
  assert.doesNotMatch(
    code,
    /innerHTML|outerHTML|insertAdjacentHTML|document\.write|eval\(|new Function/,
  );
});

test('no SPECviewperf score, GPU identifier or address can be drawn from the committed data', () => {
  const all = JSON.stringify(coverage) + JSON.stringify(fixture);
  assert.doesNotMatch(all, /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/);
  assert.doesNotMatch(all, /\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b/);
  assert.doesNotMatch(code, /scores|specviewperf/i);
});
