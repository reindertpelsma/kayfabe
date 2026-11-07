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
import { withReports, nearestAnswer } from './report_fixture.mjs';

const site = fileURLToPath(new URL('.', import.meta.url));
const code = readFileSync(join(site, 'status/status.js'), 'utf8');
const fixture = JSON.parse(readFileSync(join(site, 'status/fixture.json'), 'utf8'));
const coverage = JSON.parse(readFileSync(join(site, 'status/coverage.json'), 'utf8'));
const gpuNames = JSON.parse(readFileSync(join(site, 'status/gpu_names.json'), 'utf8'));
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
    } else if (href === 'https://kayfabe.dev/status/gpu_names.json') {
      if (opts.names === null) return { ok: false, status: 404, json: async () => ({}) };
      body = opts.names ?? gpuNames;
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
    links.every(
      (h) =>
        h.startsWith('https://kayfabe.dev/status/run/') ||
        h.startsWith('https://kayfabe.dev/status/session/') ||
        h.startsWith(REPO),
    ),
  );
  assert.ok(links.some((h) => h.startsWith('https://kayfabe.dev/status/session/?id=')), 'each live row links to its session');
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
  hostile[key].run.host.host_driver = evil;
  hostile[key].run.host.guests = [{ os: 'linux', os_version: evil, driver: evil }];
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

// --- Find my setup (site/status/find) ---------------------------------------------------------
// FALSIFIER: a filter is not a URL parameter (a view cannot be shared); an invalid parameter reaches
// a request; a laptop or unknown name gets results; a CPU, driver or source claim is drawn as
// verified or as a verdict; imported evidence is drawn as a gate pass or merged with live rows; an
// untested combination is blank instead of saying "not tested" with the repository's reason;
// community rows are shown or requested without the opt-in.
const setupsOf = (os, die) => fixture.responses['/v1/ci/setups?os=' + os + '&gpu=' + die].setups;
const setupRequests = (page) => page.requests.filter((r) => r.href.includes('/v1/ci/setups'));
const norm = (node) => node.textContent.replace(/\s+/g, ' ').trim();
// Text with a space at every element boundary, so cells read as words.
const tx = (node) => {
  const parts = [];
  const walk = (n) => {
    if (n.nodeType === 3) parts.push(n.nodeValue);
    else for (const c of n.childNodes) { parts.push(' '); walk(c); parts.push(' '); }
  };
  walk(node);
  return parts.join('').replace(/\s+/g, ' ').trim();
};
const find = (search, opts) => render('fixture', 'find', search, opts);
// The fixture mode reads one file; request counting needs the live mode, which asks per die.
const findLive = (search, opts) => render('live', 'find', search, opts);

test('find: the form has labelled controls, Arm as not tested yet, and the claims note', async () => {
  const { document } = await find('');
  assert.match(norm(document.querySelector('.kf-synthetic')), /Synthetic data\./);
  assert.equal(norm(document.querySelector('h1')), 'Find my setup');
  const form = document.querySelector('form.kf-find');
  assert.equal(form.method, 'get');
  assert.deepEqual(
    [...form.querySelectorAll('legend')].map(norm),
    ['Operating system in the guest', 'CPU vendor of the host', 'Show'],
  );
  for (const input of form.querySelectorAll('input:not([type=radio]):not([type=checkbox]):not([type=hidden]), select')) {
    const label = form.querySelector('label[for="' + input.id + '"]');
    assert.ok(label, input.name + ' has a label');
  }
  for (const input of form.querySelectorAll('input[type=radio], input[type=checkbox]'))
    assert.ok(input.closest('label'), input.name + '=' + input.value + ' has a label');
  const checked = (name) =>
    [...form.querySelectorAll('input[name=' + name + ']:checked')].map((i) => i.value);
  assert.deepEqual(checked('os'), ['linux']);
  assert.deepEqual(checked('cpu'), ['any']);
  assert.deepEqual(checked('show'), ['reproduced', 'imported'], 'community is opt-in');
  const arm = form.querySelector('input[value=arm]');
  assert.equal(arm.disabled, true);
  assert.match(norm(arm.closest('label')), /Arm \(arm64\): not tested yet/);
  assert.deepEqual(
    [...form.querySelectorAll('input[name=os]')].map((i) => i.value),
    ['linux', 'windows'],
  );
  assert.match(norm(document.querySelector('.kf-rule')), /reported by the test box and are not verified.*configured values/);
  assert.equal(setupRequests(await findLive('')).length, 0, 'nothing is asked before a GPU is entered');
  assert.match(document.getElementById('kf-status-state').textContent, /Enter a GPU/);
});

test('find: a die shows its setups by driver with result, counts, date and run link', async () => {
  const { document } = await find('?os=linux&gpu=GA106');
  const rows = [...document.querySelectorAll('table.kf-setups tbody tr')];
  const want = setupsOf('linux', 'GA106');
  assert.equal(rows.length, want.length);
  assert.deepEqual(
    [...document.querySelectorAll('table.kf-setups thead th')].map(norm),
    ['Guest driver', 'Host driver', 'CPU vendor (reported)', 'Latest result', 'Runs', 'Latest date', 'Run', 'Last pass'],
  );
  rows.forEach((row, i) => {
    const s = want[i];
    const cells = [...row.querySelectorAll('td')].map(norm);
    assert.equal(cells[0], s.guest_driver ?? 'not reported');
    assert.equal(cells[1], s.host_driver ?? 'not reported');
    assert.equal(cells[2], s.cpu_vendor ? { amd: 'AMD', intel: 'Intel' }[s.cpu_vendor] : 'not reported');
    assert.match(cells[3], new RegExp('^' + s.latest.result));
    assert.match(cells[3], /CI Worker run \(reproduced\)/);
    assert.equal(cells[4], `${s.passed} pass, ${s.failed} fail, ${s.other} other of ${s.runs}`);
    assert.equal(cells[5], s.latest.ended_at.slice(0, 10));
    assert.ok(cells[6].startsWith(s.latest.id));
    const link = row.querySelector('a[href*="/status/run/"]');
    assert.equal(link.href, 'https://kayfabe.dev/status/run/?id=' + s.latest.id);
  });
  assert.match(norm(document.querySelector('h2')), /^GA106 \(Ampere\)$/);
  assert.match(document.getElementById('kf-status-state').textContent, /GA106 on Linux: \d+ setups? with results/);
  // The newest-first order of the API is kept and the claims note is on the page.
  assert.match(norm(document.querySelector('.kf-rule')), /host driver .* guest driver/);
});

test('find: a FAIL in the other guest OS is a word, not a fail of this setup', async () => {
  const { document } = await find('?os=windows&gpu=GA106');
  const row = [...document.querySelectorAll('table.kf-setups tbody tr')].find((r) =>
    norm(r).includes('the run failed in the other guest OS'),
  );
  assert.ok(row, 'the fixture has a Linux-stage FAIL over both guests');
  assert.equal(row.querySelector('.kf-v-fail'), null);
  assert.ok(row.querySelector('.kf-v-unavailable, .kf-v-none') || /UNATTRIBUTED/.test(norm(row)));
});

test('find: a name with several dies lists each die and says so', async () => {
  for (const typed of ['RTX 3060', 'rtx3060', 'GeForce RTX 3060 12GB', '3060']) {
    const { document } = await find('?os=linux&gpu=' + encodeURIComponent(typed));
    const heads = [...document.querySelectorAll('section.kf-die-result h2')].map(norm);
    assert.deepEqual(heads, ['GA106 (Ampere)', 'GA104 (Ampere)'], typed);
    assert.match(norm(document.querySelector('#kf-status-content')), /boards use these dies: GA106, GA104/);
    assert.match(norm(document.querySelector('#kf-status-content')), /Boards sold as RTX 3060 use GA106 or GA104/);
    assert.equal(setupRequests(await findLive('?os=linux&gpu=' + encodeURIComponent(typed))).length, 2);
    assert.equal(norm(document.querySelector('h1')), 'Find my setup: RTX 3060');
  }
});

test('find: Ti, SUPER and spacing are fuzzy; an exact name beats longer names', async () => {
  const dies = async (typed) =>
    [...(await find('?gpu=' + encodeURIComponent(typed))).document.querySelectorAll('section.kf-die-result h2')].map(
      (h) => norm(h).split(' ')[0],
    );
  assert.deepEqual(await dies('rtx 3060 ti'), ['GA104']);
  assert.deepEqual(await dies('RTX3060TI'), ['GA104']);
  assert.deepEqual(await dies('GeForce RTX 4070 Ti Super'), ['AD103']);
  assert.deepEqual(await dies('rtx 4070 super'), ['AD104']);
  assert.deepEqual(await dies('RTX 4070'), ['AD104'], 'RTX 4070 is not RTX 4070 Ti');
  assert.deepEqual(await dies('gtx 1660 ti'), ['TU116']);
  assert.deepEqual(await dies('RTX 5090'), ['GB202']);
  assert.deepEqual(await dies('ga106'), ['GA106'], 'a die id in any case');
});

test('find: several matches ask which card, and the links keep every filter', async () => {
  const query = '?os=windows&gpu=RTX+40&cpu=amd&tier=middle&show=community&show=imported&driver=580.95.05';
  const { document } = await find(query);
  assert.equal(setupRequests(await findLive(query)).length, 0);
  assert.match(norm(document.querySelector('#kf-status-content')), /Several cards match/);
  const links = [...document.querySelectorAll('#kf-status-content li a')];
  assert.ok(links.length >= 8);
  for (const a of links) {
    const url = new URL(a.href);
    assert.equal(url.pathname, '/status/find/');
    assert.equal(url.searchParams.get('os'), 'windows');
    assert.equal(url.searchParams.get('cpu'), 'amd');
    assert.equal(url.searchParams.get('tier'), 'middle');
    assert.equal(url.searchParams.get('driver'), '580.95.05');
    assert.deepEqual(url.searchParams.getAll('show'), ['community', 'imported']);
    assert.match(url.searchParams.get('gpu'), /^RTX 40/);
  }
});

test('find: laptops are not covered and unknown names ask for the die', async () => {
  for (const typed of ['RTX 3060 Laptop', 'rtx 4070 mobile', 'RTX 3080 Max-Q', 'notebook 3060']) {
    const page = await find('?gpu=' + encodeURIComponent(typed));
    assert.match(norm(page.document.querySelector('#kf-status-content')), /laptops are not covered/, typed);
    assert.equal(setupRequests(await findLive('?gpu=' + encodeURIComponent(typed))).length, 0, typed);
    assert.equal(page.document.querySelector('table.kf-setups'), null);
  }
  const unknown = await find('?gpu=RTX+9999');
  assert.match(norm(unknown.document.querySelector('#kf-status-content')), /No card named “RTX 9999” is in the table/);
  assert.match(norm(unknown.document.querySelector('#kf-status-content')), /Enter the die/);
  assert.equal(setupRequests(await findLive('?gpu=RTX+9999')).length, 0);
  // Without the name table the page still takes a die id.
  const bare = await find('?gpu=GA106', { names: null });
  assert.match(norm(bare.document.querySelector('#kf-status-content')), /name table could not be loaded/);
  assert.ok(bare.document.querySelector('table.kf-setups'));
});

test('find: the CPU filter is a URL parameter and hides what it should', async () => {
  const all = setupsOf('linux', 'GA106');
  for (const vendor of ['amd', 'intel']) {
    const { document } = await find('?os=linux&gpu=GA106&cpu=' + vendor);
    const rows = [...document.querySelectorAll('table.kf-setups tbody tr')];
    const want = all.filter((s) => s.cpu_vendor === vendor);
    assert.equal(rows.length, want.length, vendor);
    for (const row of rows) assert.match(norm(row.querySelectorAll('td')[2]), vendor === 'amd' ? /^AMD$/ : /^Intel$/);
    assert.match(norm(document.querySelector('section.kf-die-result')), /setups? (?:is|are) hidden by the CPU vendor or driver filter/);
    assert.equal(document.querySelector('input[name=cpu]:checked').value, vendor);
  }
  // Rows from a box that reported no CPU are shown under Any, as "not reported", never as a vendor.
  const any = await find('?os=linux&gpu=GA106');
  assert.ok(norm(any.document.querySelector('table.kf-setups')).includes('not reported'));
  // A vendor with no run says not tested, and that runs exist but not for that vendor.
  const none = await find('?os=linux&gpu=GA104&cpu=amd&driver=575.64.05&tier=');
  const text = norm(none.document.querySelector('#kf-status-content'));
  assert.match(text, /not tested/);
  assert.match(text, /No runs for this combination\./);
  assert.equal(none.document.querySelector('table.kf-setups'), null);
  assert.equal(none.document.querySelector('.kf-v-pass'), null);
});

test('find: the driver filter matches the host or the guest driver exactly', async () => {
  const { document } = await find('?os=windows&gpu=GA106&driver=580.88');
  const want = setupsOf('windows', 'GA106').filter((s) => s.guest_driver === '580.88' || s.host_driver === '580.88');
  assert.equal(document.querySelectorAll('table.kf-setups tbody tr').length, want.length);
  const host = await find('?os=windows&gpu=GA106&driver=575.64.05');
  assert.equal(
    host.document.querySelectorAll('table.kf-setups tbody tr').length,
    setupsOf('windows', 'GA106').filter((s) => s.host_driver === '575.64.05').length,
  );
  // The form's own fields name the host and the guest driver; `driver` (either) stays valid in a
  // shared address.
  const byHost = await find('?os=windows&gpu=GA106&host_driver=575.64.05');
  assert.equal(
    byHost.document.querySelectorAll('table.kf-setups tbody tr').length,
    setupsOf('windows', 'GA106').filter((s) => s.host_driver === '575.64.05').length,
  );
  assert.equal(byHost.document.querySelector('input[name=host_driver]').value, '575.64.05');
  const byGuest = await find('?os=windows&gpu=GA106&guest_driver=580.88');
  assert.equal(
    byGuest.document.querySelectorAll('table.kf-setups tbody tr').length,
    setupsOf('windows', 'GA106').filter((s) => s.guest_driver === '580.88').length,
  );
  assert.equal(byGuest.document.querySelector('input[name=guest_driver]').value, '580.88');
});

test('find: an untested die says not tested with the reason the repository states', async () => {
  const { document } = await find('?os=linux&gpu=RTX+2080+Ti');
  const text = norm(document.querySelector('#kf-status-content'));
  assert.match(text, /TU102 \(Turing\)/);
  assert.match(text, /not tested/);
  assert.match(text, /No runs for this combination\./);
  assert.match(text, /No Vast VM offer during the lane/);
  assert.equal(document.querySelector('table.kf-setups'), null);
  assert.equal(document.querySelector('.kf-v-pass'), null);
  // Without a reason in the evidence file the page says that none is stated.
  const bare = await find('?os=linux&gpu=TU102', { coverage: { schema: 1, families: [] } });
  assert.match(norm(bare.document.querySelector('#kf-status-content')), /the repository states no reason/);
  const missing = await find('?os=linux&gpu=TU102', { coverage: null });
  assert.match(norm(missing.document.querySelector('#kf-status-content')), /not tested/);
});

test('find: imported evidence is shown apart, labelled, and never as a gate pass', async () => {
  const { document } = await find('?os=windows&gpu=RTX+3060');
  const sections = [...document.querySelectorAll('section.kf-die-result')];
  const ga106 = sections.find((s) => norm(s.querySelector('h2')).startsWith('GA106'));
  const heading = [...ga106.querySelectorAll('h3')].map(norm);
  assert.deepEqual(heading, ['Evidence in the repository, not CI gate results']);
  const blocks = [...ga106.querySelectorAll('.kf-block-imported')];
  assert.ok(blocks.length > 0);
  for (const block of blocks) {
    assert.match(norm(block), /Direct Vast run, evidence in repo \(imported\)/);
    assert.equal(block.querySelector('.kf-v-pass, .kf-v-fail, .kf-v-retry'), null);
    assert.doesNotMatch(norm(block), /\bPASS\b/);
    assert.match(norm(block), /revision [0-9a-f]{8}/);
  }
  // The live table and the evidence are different elements.
  assert.equal(ga106.querySelector('table.kf-setups').querySelector('.kf-block-imported'), null);
  // Imported evidence alone: no result request at all, and the live table is absent.
  const only = await find('?os=windows&gpu=GA106&show=imported');
  assert.equal(setupRequests(await findLive('?os=windows&gpu=GA106&show=imported')).length, 0);
  assert.equal(only.document.querySelector('table.kf-setups'), null);
  assert.ok(only.document.querySelector('.kf-block-imported'));
  assert.match(norm(only.document.querySelector('#kf-status-content')), /No runs for this combination\./);
});

test('find: Show defaults to reproduced and imported; community is opt-in and asked for explicitly', async () => {
  const base = await find('?os=linux&gpu=GA106');
  assert.ok(base.document.querySelector('.kf-block-imported'));
  assert.ok(base.document.querySelector('table.kf-setups'));
  const reproduced = await find('?os=linux&gpu=GA106&show=reproduced');
  assert.equal(reproduced.document.querySelector('.kf-block-imported, .kf-block-worker'), null, 'imported is off');
  assert.ok(reproduced.document.querySelector('table.kf-setups'));
  const community = await find('?os=linux&gpu=GA106&show=reproduced&show=community');
  assert.deepEqual(
    [...community.document.querySelectorAll('input[name=show]:checked')].map((i) => i.value),
    ['reproduced', 'community'],
  );
  assert.match(norm(community.document.querySelector('#kf-status-state')), /GA106 on Linux/);
  const communityOnly = await find('?os=linux&gpu=GA106&show=community');
  assert.match(norm(communityOnly.document.querySelector('#kf-status-content')), /not tested/);
  assert.match(norm(communityOnly.document.querySelector('#kf-status-content')), /No runs for this combination\./);
  assert.equal(communityOnly.document.querySelector('table.kf-setups'), null);
  assert.equal(communityOnly.document.querySelector('.kf-block-imported'), null);
  // Nothing ticked is the default, never an empty page.
  const none = await find('?os=linux&gpu=GA106&show=bogus');
  assert.deepEqual(
    [...none.document.querySelectorAll('input[name=show]:checked')].map((i) => i.value),
    ['reproduced', 'imported'],
  );
});

test('find: live mode asks /v1/ci/setups with exactly the validated filters', async () => {
  const asked = async (search) =>
    setupRequests(await render('live', 'find', search)).map((r) => r.href.slice(API.length));
  assert.deepEqual(await asked('?os=linux&gpu=GA106'), ['/v1/ci/setups?os=linux&gpu=GA106']);
  assert.deepEqual(await asked('?os=windows&gpu=GA104&tier=middle'), [
    '/v1/ci/setups?os=windows&gpu=GA104&tier=middle',
  ]);
  assert.deepEqual(await asked('?gpu=GA106&show=reproduced&show=community'), [
    '/v1/ci/setups?os=linux&gpu=GA106&source=reproduced,community',
  ]);
  // The order of the show parameters in the address never changes the request.
  assert.deepEqual(await asked('?gpu=GA106&show=community&show=reproduced'), [
    '/v1/ci/setups?os=linux&gpu=GA106&source=reproduced,community',
  ]);
  assert.deepEqual(await asked('?gpu=GA106&show=community'), [
    '/v1/ci/setups?os=linux&gpu=GA106&source=community',
  ]);
  // The CPU vendor and driver filters are sent to the API in live mode (a page limit can never
  // hide a matching setup), in the order os, gpu, cpu, driver, tier, source.
  assert.deepEqual(
    await asked('?os=windows&gpu=GA106&cpu=amd&driver=580.95.05&tier=middle&show=reproduced&show=community'),
    [
      '/v1/ci/setups?os=windows&gpu=GA106&cpu=amd&driver=580.95.05&tier=middle&source=reproduced,community',
    ],
  );
  assert.deepEqual(await asked('?gpu=GA106&cpu=any'), ['/v1/ci/setups?os=linux&gpu=GA106']);
  // Invalid values are ignored, not sent.
  assert.deepEqual(
    await asked('?os=macos&gpu=GA106&cpu=via&tier=huge&driver=1%27&show=all'),
    ['/v1/ci/setups?os=linux&gpu=GA106'],
  );
  const live = await render('live', 'find', '?gpu=RTX+3060');
  for (const { href, init } of live.requests) {
    assert.ok(
      href.startsWith(API + '/v1/ci/') ||
        ['coverage.json', 'gpu_names.json'].some((f) => href === 'https://kayfabe.dev/status/' + f),
      href,
    );
    assert.equal(init.credentials, 'omit');
  }
  assert.equal(live.document.querySelector('.kf-synthetic'), null);
});

test('find: a hostile query or API value never becomes markup', async () => {
  const hostile = await find('?gpu=%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E&os=%3Cb%3E');
  assert.equal(hostile.document.querySelector('#kf-status-content img, #kf-status-content b'), null);
  assert.equal(hostile.document.querySelector('input[name=gpu]').value, '');
  const odd = await find('?os=linux&gpu=GA106&driver=1%27%3Cb%3E&host_driver=1%27%3Cb%3E&guest_driver=1%27%3Cb%3E&commit=zz&relax=x');
  assert.equal(odd.document.querySelector('input[name=host_driver]').value, '', 'an invalid driver is ignored');
  assert.equal(odd.document.querySelector('input[name=guest_driver]').value, '', 'an invalid guest driver is ignored');
  assert.equal(odd.document.querySelector('input[name=commit]').value, '', 'an invalid commit is ignored');
  assert.equal(
    odd.document.querySelectorAll('table.kf-setups tbody tr').length,
    setupsOf('linux', 'GA106').length,
  );
  const evil = structuredClone(fixture.responses);
  const key = '/v1/ci/setups?os=linux&gpu=GA106';
  evil[key].setups[0].host_driver = '<img src=x onerror=alert(1)>';
  evil[key].setups[0].cpu_vendor = '<b>amd</b>';
  evil[key].setups[0].latest.result = '<script>1</script>';
  const page = await find('?os=linux&gpu=GA106', { responses: evil });
  assert.equal(page.document.querySelector('#kf-status-content img, #kf-status-content b, #kf-status-content script'), null);
  assert.match(norm(page.document.querySelector('table.kf-setups')), /<img src=x/);
  assert.match(norm(page.document.querySelector('table.kf-setups tbody tr')), /no data|<b>amd<\/b>/);
});

test('find: typing offers matching cards as buttons that fill the field', async () => {
  const { document, window } = await find('');
  const input = document.getElementById('kf-gpu');
  input.value = 'rtx 3060';
  input.dispatchEvent(new window.Event('input'));
  const buttons = [...document.querySelectorAll('#kf-suggest button')];
  assert.deepEqual(buttons.map((b) => b.type), buttons.map(() => 'button'));
  assert.deepEqual(buttons.map((b) => norm(b)), ['RTX 3060 GA106, GA104']);
  input.value = 'rtx 30';
  input.dispatchEvent(new window.Event('input'));
  assert.ok(document.querySelectorAll('#kf-suggest button').length >= 5);
  assert.ok(document.querySelectorAll('#kf-suggest button').length <= 8);
  input.value = '';
  input.dispatchEvent(new window.Event('input'));
  assert.equal(document.getElementById('kf-suggest').hidden, true);
  assert.equal(document.getElementById('kf-suggest').getAttribute('aria-label'), 'Matching cards');
});

test('find: the overview links to it, and its stylesheet uses only theme tokens', async () => {
  const { document } = await render('fixture', '');
  const link = [...document.querySelectorAll('#kf-status-content a')].find((a) => norm(a) === 'Find my setup');
  assert.equal(link.href, 'https://kayfabe.dev/status/find/');
  const css = readFileSync(join(site, 'status/status.css'), 'utf8');
  const block = css.slice(css.indexOf('/* Find my setup */'), css.indexOf('@media (max-width: 40rem)'));
  assert.ok(block.length > 500);
  assert.doesNotMatch(block, /#[0-9a-fA-F]{3,8}\b|rgb\(|hsl\(/, 'colours come from tokens, so dark mode follows');
  assert.match(css, /prefers-color-scheme: dark/);
  assert.match(css, /\.kf-setups td::before/, 'a stacked layout at phone width');
});

test('find: in live mode a setup the unfiltered page limit would cut is found through the filter', async () => {
  const responses = structuredClone(fixture.responses);
  const base = '/v1/ci/setups?os=linux&gpu=GA106';
  const row = structuredClone(responses[base].setups[0]);
  row.cpu_vendor = 'amd';
  row.host_driver = '590.12.34';
  row.guest_driver = '590.12.34';
  responses[base].truncated = true;
  responses[base].setups = responses[base].setups.filter((s) => s.cpu_vendor !== 'amd');
  responses[base + '&cpu=amd&driver=590.12.34'] = { ...responses[base], truncated: false, setups: [row] };
  const page = await render('live', 'find', '?os=linux&gpu=GA106&cpu=amd&driver=590.12.34', { responses });
  const rows = [...page.document.querySelectorAll('table.kf-setups tbody tr')];
  assert.equal(rows.length, 1);
  assert.match(norm(rows[0]), /590\.12\.34/);
  assert.equal(page.document.querySelector('.kf-block-none .kf-v-none'), null);
});

test('run page: shows the box driver and each guest as reported, and no provider label', async () => {
  const withClaims = Object.entries(fixture.responses).find(
    ([k, v]) => k.startsWith('/v1/ci/runs/') && v.run.host.host_driver && v.run.host.guests.length,
  )[1].run;
  const { document } = await render('fixture', 'run', '?id=' + withClaims.id);
  const t = norm(document.querySelector('#kf-status-content'));
  assert.match(t, new RegExp('Host driver \\(reported\\)' + withClaims.host.host_driver.replaceAll('.', '\\.')));
  for (const g of withClaims.host.guests)
    assert.match(t, new RegExp((g.os === 'windows' ? 'Windows' : 'Linux') + ' guest \\(reported\\)'));
  assert.doesNotMatch(t, /offer|provider|Host driver label|guest driver 580\.159/i);
  const none = Object.entries(fixture.responses).find(
    ([k, v]) => k.startsWith('/v1/ci/runs/') && !v.run.host.host_driver,
  )[1].run;
  const bare = await render('fixture', 'run', '?id=' + none.id);
  assert.doesNotMatch(norm(bare.document.querySelector('#kf-status-content')), /Host driver \(reported\)/);
});

// --- Run report, session, stats, nearest subruns (report contract, 2026-11-07) ----------------------
// FALSIFIER: a number, test or application is drawn that the report does not hold; a missing report
// or stage reads as a pass; the log button points anywhere but the configured log origin and the
// run's own key; a SPECviewperf-like row is drawn; an API value becomes markup; a nearest subrun is
// shown without saying what differs; a sort or filter is not a URL parameter.
const reported = withReports(fixture.responses);
const reportRuns = Object.entries(reported)
  .filter(([k, v]) => /^\/v1\/ci\/runs\//.test(k) && v.run.report)
  .map(([, v]) => v.run);
const goodRun = reportRuns.find((r) => r.verdict === 'PASS' && r.platform === 'both' && r.report.apps.length);
const badRun = reportRuns.find((r) => r.verdict === 'FAIL');
const LOGS = 'https://logs.kayfabe.dev';

test('run page: summary block first, then numbers, tests, applications, stages, raw details closed', async () => {
  const { document } = await render('live', 'run', '?id=' + goodRun.id, { responses: reported });
  const content = document.getElementById('kf-status-content');
  const order = [...content.children].map((n) => n.getAttribute('aria-label') || n.querySelector('h2')?.textContent || n.tagName);
  assert.equal(order[1], 'Summary', 'the summary is the first block after the back link');
  const summary = norm(content.querySelector('.kf-summary'));
  for (const word of ['PASS', 'Tier', 'Revision', 'Date', 'Duration', 'Die', 'Platform'])
    assert.match(summary, new RegExp(word), word);
  assert.match(summary, /43\.0 min/);
  const headings = [...content.querySelectorAll(':scope > section > h2')].map(norm);
  assert.deepEqual(headings, ['Performance numbers', 'Tests', 'Applications', 'Windows gate', 'Stages']);
  const details = content.querySelector('details.kf-details');
  assert.equal(details.open, false, 'raw details are collapsed');
  assert.match(norm(details), /Result key/);
  assert.match(norm(details), /Log SHA-256/);
});

test('run page: the log and report buttons open the configured origin and this run only', async () => {
  const { document } = await render('live', 'run', '?id=' + goodRun.id, { responses: reported });
  const full = document.querySelector('.kf-summary a.kf-btn[href$="/log.txt"]');
  assert.equal(full.href, LOGS + '/' + goodRun.id + '/log.txt');
  assert.equal(full.target, '_blank');
  assert.match(full.rel, /noopener/);
  assert.match(norm(full), /^Full log \(\d/);
  const json = [...document.querySelectorAll('.kf-summary a.kf-btn')].find((a) => /report\.json/.test(norm(a)));
  assert.equal(json.href, LOGS + '/' + goodRun.id + '/report.json');
  // The fixture build has no log origin: the button says so and is not a link.
  const noOrigin = await render('fixture', 'run', '?id=' + goodRun.id, { responses: reported });
  assert.equal(noOrigin.document.querySelector('.kf-summary a[href*="log.txt"]'), null);
  assert.match(norm(noOrigin.document.querySelector('.kf-summary')), /Full log: no log origin in this build/);
  // A log url that is another origin, or another run's key, is never linked.
  for (const url of ['https://evil.example/' + goodRun.id + '/log.txt', LOGS + '/other-run-1/log.txt', LOGS + '/' + goodRun.id + '/x.txt']) {
    const responses = structuredClone(reported);
    responses['/v1/ci/runs/' + goodRun.id].run.log.url = url;
    const page = await render('live', 'run', '?id=' + goodRun.id, { responses });
    assert.equal(page.document.querySelector('a[href*="log.txt"], a[href*="evil"]'), null, url);
  }
  // No log stored: said in words, no link.
  const responses = structuredClone(reported);
  delete responses['/v1/ci/runs/' + goodRun.id].run.log;
  delete responses['/v1/ci/runs/' + goodRun.id].run.report.log;
  const none = await render('live', 'run', '?id=' + goodRun.id, { responses });
  assert.match(norm(none.document.querySelector('.kf-summary')), /Full log: none stored for this run/);
});

test('run page: performance rows show value, unit, direction and the reference ratio in words', async () => {
  const { document } = await render('live', 'run', '?id=' + goodRun.id, { responses: reported });
  const table = document.querySelector('table.kf-perf');
  assert.deepEqual(
    [...table.querySelectorAll('thead th')].map(norm),
    ['Subrun', 'Metric', 'Suite', 'Value', 'Unit', 'Better when', 'Same-box reference'],
  );
  const rows = [...table.querySelectorAll('tbody tr')].map(tx);
  assert.ok(rows.some((r) => /cup8bench FP32 matmul \(guest\).*19\.90 TFLOPS higher 0\.970. the reference \(20\.50 TFLOPS\)/.test(r)), rows.join('|'));
  assert.ok(rows.some((r) => /no reference measured/.test(r)));
  // A viewport-benchmark row is never drawn, in the page or in the stats.
  const responses = structuredClone(reported);
  const run = responses['/v1/ci/runs/' + goodRun.id].run;
  run.report.perf.push({ sub: 'host', name: 'SPECviewperf 2020 maya', value: 321, unit: 'fps', suite: 'x' });
  const page = await render('live', 'run', '?id=' + goodRun.id, { responses });
  assert.doesNotMatch(norm(page.document.getElementById('kf-status-content')), /SPECviewperf|321/);
});

test('run page: tests list FAIL first, count per suite, and filter by result', async () => {
  const { document } = await render('live', 'run', '?id=' + badRun.id, { responses: reported });
  const rows = [...document.querySelectorAll('table.kf-tests tbody tr')];
  assert.equal(rows[0].dataset.result, 'FAIL');
  assert.ok(rows[0].querySelector('.kf-v-fail'));
  assert.match(norm(document.querySelector('.kf-counts')), /thin_suite: \d+ pass, 1 fail, \d+ skipped/);
  const press = (name) => [...document.querySelectorAll('.kf-seg button')].find((b) => norm(b) === name);
  press('Failed or refused').click();
  assert.deepEqual(rows.filter((r) => !r.hidden).map((r) => r.dataset.result), ['FAIL']);
  assert.equal(press('Failed or refused').getAttribute('aria-pressed'), 'true');
  assert.equal(press('All').getAttribute('aria-pressed'), 'false');
  assert.match(norm(document.querySelector('[role=status].kf-small')), /Showing 1 of \d+ tests\./);
  press('All').click();
  assert.equal(rows.filter((r) => r.hidden).length, 0);
});

test('run page: what is absent says so and nothing is invented', async () => {
  const { document } = await render('fixture', 'run', '?id=' + passRun.id);
  const text = norm(document.getElementById('kf-status-content'));
  assert.match(text, /Not measured yet: this run has no stored report/);
  assert.match(text, /the application matrix stage does not exist in CI yet/);
  assert.equal(document.querySelector('table.kf-perf, table.kf-tests, table.kf-apps'), null);
  assert.equal(document.querySelector('.kf-counts'), null);
  // A report without numbers or applications.
  const responses = structuredClone(reported);
  const run = responses['/v1/ci/runs/' + goodRun.id].run;
  run.report.perf = [];
  run.report.apps = [];
  run.report.windows = undefined;
  const page = await render('live', 'run', '?id=' + goodRun.id, { responses });
  const t = tx(page.document.getElementById('kf-status-content'));
  assert.match(t, /this report holds no performance numbers\. Nothing is estimated here/);
  assert.match(t, /the application matrix stage does not exist in CI yet/);
  assert.match(tx(page.document.getElementById('kf-status-content')), /Windows gate Not measured yet: this report has no Windows block/);
});

test('run page: report values are text, never markup, and bad numbers are dropped', async () => {
  const responses = structuredClone(reported);
  const run = responses['/v1/ci/runs/' + goodRun.id].run;
  run.report.tests[0].name = '<img src=x onerror=alert(1)>';
  run.report.perf[0].name = '<b>x</b>';
  run.report.perf[1].value = 'NaN';
  run.report.apps = [{ sub: 'host', name: '<script>1</script>', os: 'windows', result: '<i>PASS</i>', note: '<u>n</u>' }];
  const { document } = await render('live', 'run', '?id=' + goodRun.id, { responses });
  assert.equal(document.querySelector('#kf-status-content img, #kf-status-content b, #kf-status-content script, #kf-status-content u, #kf-status-content i'), null);
  assert.match(norm(document.querySelector('table.kf-tests')), /<img src=x/);
  assert.doesNotMatch(norm(document.querySelector('table.kf-perf')), /NaN/);
  assert.equal(document.querySelector('table.kf-apps .kf-v-pass'), null, 'an unknown result is not a pass');
});

test('session page: header, a row per subrun with its log range, and a section per subrun', async () => {
  const { document } = await render('live', 'session', '?id=' + goodRun.id, { responses: reported });
  assert.equal(document.querySelector('h1').textContent, 'Host session ' + goodRun.id);
  const head = tx(document.querySelector('.kf-summary'));
  assert.match(head, /Host driver \(reported\) 580\.159\.04/);
  assert.match(head, /CPU vendor \(reported\) (?:amd|intel)/);
  const rows = [...document.querySelectorAll('table.kf-subruns tbody tr')];
  assert.deepEqual(rows.map((r) => norm(r.querySelector('td'))), goodRun.subruns.map((s) => s.sub));
  const windows = rows.find((r) => /windows-580\.88/.test(norm(r)));
  assert.match(tx(windows), /Windows guest 580\.88 .*PASS .*2 pass, 0 fail, 0 skipped .*bytes \d+–\d+ of the log/);
  const link = windows.querySelector('a[type="text/plain"]');
  assert.equal(link.href, LOGS + '/' + goodRun.id + '/log.txt');
  const sections = [...document.querySelectorAll('section.kf-sub')];
  assert.equal(sections.length, goodRun.subruns.length);
  for (const [i, s] of goodRun.subruns.entries()) {
    assert.equal(rows[i].querySelector('a').getAttribute('href'), '#' + sections[i].id);
    assert.match(norm(sections[i].querySelector('h2')), new RegExp('^' + s.sub));
  }
  // Rows are filtered by subrun: the host section holds no Linux guest test.
  const host = sections.find((s) => s.id === 'sub-host');
  assert.match(norm(host), /merge_bar arm 1/);
  assert.doesNotMatch(norm(host), /concurrency/);
  assert.equal(document.querySelectorAll('section.kf-sub h3').length > 0, true, 'inner headings are one level down');
});

test('session page: a subrun that was not run says so with its reason; no breakdown is an empty state', async () => {
  const withNotRun = reportRuns.find((r) => r.subruns.some((s) => s.verdict === 'NOT_RUN'));
  const { document } = await render('live', 'session', '?id=' + withNotRun.id, { responses: reported });
  const notRun = document.getElementById('sub-windows-575-64-05');
  assert.match(norm(notRun), /not run/i);
  assert.match(norm(notRun), /Not run: the small tier does not install a second Windows driver yet/);
  assert.equal(notRun.querySelector('table.kf-tests'), null);
  const old = await render('fixture', 'session', '?id=' + passRun.id);
  assert.match(norm(old.document.getElementById('kf-status-content')), /this session has no subrun breakdown/);
  assert.ok(old.document.querySelector('.kf-runs'), 'the stages still show');
  const missing = await render('fixture', 'session', '');
  assert.match(missing.document.getElementById('kf-status-state').textContent, /needs a run id/);
});

test('every run row elsewhere links to its session', async () => {
  const matrix = await render('fixture', '', '?tier=middle');
  assert.ok([...matrix.document.querySelectorAll('.kf-block-live a')].some((a) => /\/status\/session\/\?id=/.test(a.href)));
  const commit = await render('fixture', 'commit', '?sha=' + commitKey.split('/').pop());
  assert.ok([...commit.document.querySelectorAll('.kf-runs tbody tr')].every((r) => r.querySelector('a[href*="/status/session/?id="]')));
  const f = await find('?os=linux&gpu=GA106');
  assert.ok([...f.document.querySelectorAll('table.kf-setups tbody tr')].every((r) => r.querySelector('a[href*="/status/session/?id="]')));
});

test('stats page: rows, sort and filters are URL parameters, and live mode sends the validated filters', async () => {
  const page = await render('live', 'stats', '', { responses: reported });
  const rows = [...page.document.querySelectorAll('table.kf-stats > tbody > tr')];
  assert.equal(rows.length, reportRuns.length);
  assert.deepEqual(page.requests.filter((r) => r.href.includes('/v1/ci/stats')).map((r) => r.href), [API + '/v1/ci/stats']);
  const sorted = await render('live', 'stats', '?sort=pass&dir=desc', { responses: reported });
  const passes = [...sorted.document.querySelectorAll('table.kf-stats > tbody > tr')].map((r) => Number(r.querySelectorAll('td')[4].textContent));
  assert.deepEqual(passes, [...passes].sort((a, b) => b - a));
  assert.equal(sorted.document.querySelector('th[aria-sort="descending"]').textContent.trim().startsWith('Tests passed'), true);
  const headers = [...sorted.document.querySelectorAll('th a.kf-sort')];
  assert.ok(headers.every((a) => /\/status\/stats\/\?/.test(a.href)), 'sorting is a link, so it can be shared');
  const die = reportRuns[0].die;
  const filtered = await render('live', 'stats', '?die=' + die + '&tier=middle&driver=580.159.04&bogus=1', { responses: reported });
  assert.ok(filtered.requests.some((r) => r.href === API + '/v1/ci/stats?die=' + die + '&tier=middle&driver=580.159.04'));
  assert.ok([...filtered.document.querySelectorAll('table.kf-stats > tbody > tr')].every((r) => r.querySelector('td').textContent.trim() === die));
  assert.equal(filtered.document.querySelector('input[name=die]').value, die);
  // An invalid value is ignored and never reaches a request.
  const bad = await render('live', 'stats', '?die=x%27y&tier=zz&driver=1&sort=evil', { responses: reported });
  assert.ok(bad.requests.every((r) => !/evil|x%27|zz/.test(r.href)));
  assert.deepEqual(bad.requests.filter((r) => r.href.includes('/v1/ci/stats')).map((r) => r.href), [API + '/v1/ci/stats']);
  assert.ok(bad.document.querySelector('table.kf-stats'), 'an unknown sort key falls back to the default sort');
  assert.match(bad.document.getElementById('kf-status-state').textContent, /sorted by Die\./);
});

test('stats page: numbers sit behind each row, an empty answer is not measured yet, nothing is invented', async () => {
  const page = await render('fixture', 'stats', '', { responses: reported });
  const row = page.document.querySelector('table.kf-stats > tbody > tr');
  assert.match(norm(row.querySelector('details summary')), /\d+ numbers?/);
  assert.ok(row.querySelector('table.kf-perf'));
  assert.ok(row.querySelector('a[href*="/status/session/?id="]'));
  const empty = await render('fixture', 'stats', '');
  assert.match(norm(empty.document.getElementById('kf-status-content')), /Not measured yet: no stored report has pass counts or performance numbers/);
  assert.equal(empty.document.querySelector('table.kf-stats'), null);
  assert.doesNotMatch(norm(empty.document.getElementById('kf-status-content')), /SPECviewperf/);
  const responses = structuredClone(reported);
  responses['/v1/ci/stats'].stats[0].die = '<img src=x>';
  responses['/v1/ci/stats'].stats[1].perf = [{ name: 'SPECviewperf 2020', value: 5, unit: 'fps' }, { name: 'ok', value: 1, unit: 'u' }];
  const hostile = await render('fixture', 'stats', '', { responses });
  assert.equal(hostile.document.querySelector('#kf-status-content img'), null);
  assert.doesNotMatch(norm(hostile.document.getElementById('kf-status-content')), /SPECviewperf/);
});

test('find: when nothing matches exactly the nearest subruns come with what differs', async () => {
  const responses = structuredClone(reported);
  const near = nearestAnswer(reportRuns);
  const path = '/v1/ci/subruns?commit=a1b2c3d4&die=TU102&os=linux&limit=10';
  responses[path] = near;
  const page = await render('live', 'find', '?os=linux&gpu=RTX+2080+Ti&commit=a1b2c3d4', { responses });
  assert.ok(page.requests.some((r) => r.href === API + path), page.requests.map((r) => r.href).join('\n'));
  const text = norm(page.document.querySelector('.kf-nearest'));
  assert.match(text, /No exact match: the nearest subruns/);
  const first = norm(page.document.querySelector('.kf-nearest tbody tr'));
  assert.match(first, /same guest OS/);
  assert.match(first, /same host driver/);
  assert.match(first, /different commit revision \(wanted a1b2c3d4, found [0-9a-f]{8}\)/);
  assert.match(first, /different CPU type \(wanted amd, found intel\)/);
  const second = norm(page.document.querySelectorAll('.kf-nearest tbody tr')[1]);
  assert.match(second, /different host driver \(wanted 590\.12\.34, found 580\.159\.04\)/);
  assert.match(second, /different die within the same family/);
  assert.ok(page.document.querySelector('.kf-chip-diff'));
  const link = page.document.querySelector('.kf-nearest tbody a[href*="/status/session/?id="]');
  assert.match(link.href, /#sub-linux-580-159-03$/);
  // No subrun at all is said plainly, not as a blank.
  const none = structuredClone(reported);
  none[path] = { api: 'v1', subruns: [] };
  const empty = await render('live', 'find', '?os=linux&gpu=RTX+2080+Ti&commit=a1b2c3d4', { responses: none });
  assert.match(norm(empty.document.querySelector('.kf-block-none:last-of-type, .kf-nearest, section.kf-die-result')), /no near match|No subrun of TU102 is close/);
});

test('find: closeness is an ordered list in the address, validated, reorderable, and sent to the API', async () => {
  const responses = structuredClone(reported);
  const mk = (relax) => '/v1/ci/subruns?die=TU102&os=linux' + (relax ? '&relax=' + relax : '') + '&limit=10';
  responses[mk('')] = nearestAnswer(reportRuns);
  responses[mk('cpu,commit')] = nearestAnswer(reportRuns);
  responses[mk('none')] = { api: 'v1', subruns: [] };
  const given = await render('live', 'find', '?os=linux&gpu=TU102&relax=cpu,commit', { responses });
  assert.ok(given.requests.some((r) => r.href === API + mk('cpu,commit')));
  const boxes = [...given.document.querySelectorAll('.kf-relax li')].map((li) => [li.dataset.dim, li.querySelector('input').checked]);
  assert.deepEqual(boxes.slice(0, 2), [['cpu', true], ['commit', true]]);
  assert.ok(boxes.slice(2).every(([, on]) => on === false), 'dropped steps are unticked');
  assert.equal(given.document.querySelector('details.kf-closeness').open, true);
  // Default and bogus values both mean the default ladder, and neither is sent.
  for (const value of ['', '&relax=bogus', '&relax=cpu,cpu', '&relax=commit,evil']) {
    const page = await render('live', 'find', '?os=linux&gpu=TU102' + value, { responses });
    const asked = page.requests.map((r) => r.href).filter((h) => h.includes('/v1/ci/subruns'));
    assert.ok(asked.every((h) => !/evil|bogus/.test(h)), value);
    assert.deepEqual(
      [...page.document.querySelectorAll('.kf-relax li')].map((li) => li.dataset.dim),
      ['commit', 'cpu', 'host_driver', 'guest_driver', 'die_family', 'os', 'die_any'],
      value,
    );
  }
  const none = await render('live', 'find', '?os=linux&gpu=TU102&relax=none', { responses });
  assert.ok(none.requests.some((r) => r.href === API + mk('none')));
  assert.ok([...none.document.querySelectorAll('.kf-relax input')].every((i) => !i.checked));
  // Reordering with the buttons, then submitting, writes the new order into `relax`.
  const page = await render('live', 'find', '?os=linux&gpu=TU102', { responses });
  const { document, window } = page;
  const second = document.querySelector('.kf-relax li[data-dim=cpu]');
  second.querySelector('[data-move=up]').click();
  assert.deepEqual([...document.querySelectorAll('.kf-relax li')].slice(0, 2).map((l) => l.dataset.dim), ['cpu', 'commit']);
  assert.equal(document.querySelector('.kf-relax li:first-child [data-move=up]').disabled, true);
  document.getElementById('kf-relax-os').checked = false;
  const form = document.querySelector('form.kf-find');
  form.dispatchEvent(new window.Event('submit', { cancelable: true }));
  const hidden = form.querySelector('input[name=relax]');
  assert.equal(hidden.disabled, false);
  assert.equal(hidden.value, 'cpu,commit,host_driver,guest_driver,die_family,die_any');
  for (const li of document.querySelectorAll('.kf-relax li')) li.querySelector('input').checked = true;
});

test('find: filters and closeness survive the links of an ambiguous name', async () => {
  const { document } = await find('?os=linux&gpu=RTX+40&host_driver=580.159.04&guest_driver=580.159.03&commit=a1b2c3d4&relax=cpu');
  const links = [...document.querySelectorAll('#kf-status-content li a')];
  assert.ok(links.length >= 1);
  for (const a of links) {
    const q = new URL(a.href).searchParams;
    assert.equal(q.get('host_driver'), '580.159.04');
    assert.equal(q.get('guest_driver'), '580.159.03');
    assert.equal(q.get('commit'), 'a1b2c3d4');
    assert.equal(q.get('relax'), 'cpu');
  }
});

test('the site stylesheet keeps its promises: tokens for both themes, focus, motion, print', () => {
  const css = readFileSync(join(site, 'static/style.css'), 'utf8') + readFileSync(join(site, 'status/status.css'), 'utf8');
  assert.match(css, /prefers-color-scheme: dark/);
  assert.match(css, /\[data-theme="dark"\]/);
  assert.match(css, /prefers-reduced-motion: reduce/);
  assert.match(css, /@media print/);
  assert.match(css, /:focus-visible/);
  assert.doesNotMatch(css, /url\(|@import|https?:\/\//);
});
