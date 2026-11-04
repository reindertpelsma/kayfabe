// Status pages in jsdom against the committed SYNTHETIC fixture (status/fixture.json).
// FALSIFIER: a page fails to render from the fixture; API text becomes markup; live mode asks
// anything but the configured API origin, or sends credentials; an invalid query parameter
// reaches a request; status.js holds a URL literal or uses an HTML sink.
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
const API = 'https://api.kayfabe.dev';

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

async function render(mode, path, search = '', responses = fixture.responses) {
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
    else if (href.startsWith(API + '/')) {
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

const firstRun = Object.entries(fixture.responses).find(([k]) => k.startsWith('/v1/runs/'))[1].run;
const specRun = Object.values(fixture.responses).find(
  (b) => b.run?.suite === 'windows-specviewperf',
).run;
const commitKey = Object.keys(fixture.responses).find((k) => k.startsWith('/v1/commits/'));

test('matrix renders every die and suite from the fixture, with the synthetic banner', async () => {
  const { document } = await render('fixture', '');
  assert.match(document.querySelector('.kf-synthetic').textContent, /Synthetic data/);
  const rows = document.querySelectorAll('.kf-matrix tbody tr:not(.kf-family)');
  assert.equal(rows.length, 15);
  for (const row of rows) assert.equal(row.querySelectorAll('td.kf-cell').length, 6);
  assert.match(document.getElementById('kf-status-state').textContent, /^Reference: next/);
  const hrefs = [...document.querySelectorAll('#kf-status-content a')].map((a) => a.href);
  assert.ok(hrefs.length > 50);
  for (const href of hrefs)
    assert.ok(
      href.startsWith('https://kayfabe.dev/status/') ||
        href.startsWith('https://github.com/reindertpelsma/kayfabe/commit/'),
      href,
    );
  assert.ok(document.querySelector('.kf-filters select[name="family"] option[value="ada"]'));
});

test('cell, run and commit pages render from the fixture', async () => {
  const cell = await render('fixture', 'cell', '?die=GA106&suite=linux-gate');
  assert.ok(cell.document.querySelectorAll('.kf-runs tbody tr').length >= 2);
  const run = await render('fixture', 'run', '?id=' + firstRun.id);
  assert.equal(run.document.querySelector('h1').textContent, 'Run ' + firstRun.id);
  assert.ok(run.document.querySelectorAll('.kf-runs tbody tr').length >= 1);
  assert.match(
    run.document.getElementById('kf-status-content').textContent,
    /no log files in the fixture/,
  );
  const spec = await render('fixture', 'run', '?id=' + specRun.id);
  assert.match(
    spec.document.getElementById('kf-status-content').textContent,
    /not published \(SPEC_RESULTS\.md\)/,
  );
  const sha = commitKey.split('/')[3];
  const commit = await render('fixture', 'commit', '?sha=' + sha);
  assert.ok(commit.document.querySelectorAll('.kf-req').length >= 1);
});

test('API text is rendered as text, never as markup', async () => {
  const hostile = structuredClone(fixture.responses);
  const evil = '<img src=x onerror="window.pwned=1">';
  hostile['/v1/matrix'].families[0].name = evil;
  hostile['/v1/matrix'].families[0].dies[0].id = evil;
  hostile['/v1/runs/' + firstRun.id].run.commit_info.message = evil;
  hostile['/v1/runs/' + firstRun.id].run.host_facts = { note: evil };
  const matrix = await render('fixture', '', '', hostile);
  assert.equal(matrix.document.querySelector('#kf-status-content img'), null);
  assert.ok(matrix.document.getElementById('kf-status-content').textContent.includes(evil));
  const run = await render('fixture', 'run', '?id=' + firstRun.id, hostile);
  assert.equal(run.document.querySelector('#kf-status-content img'), null);
  assert.equal(run.window.pwned, undefined);
});

test('live mode asks only the configured API origin, without credentials', async () => {
  const live = await render('live', '');
  assert.ok(live.requests.length >= 2);
  for (const { href, init } of live.requests) {
    assert.ok(href.startsWith(API + '/v1/'), href);
    assert.equal(init.credentials, 'omit');
  }
  const run = await render('live', 'run', '?id=' + firstRun.id);
  const log = run.document.querySelector('#kf-status-content ul a');
  assert.equal(
    log.href,
    'https://logs.kayfabe.dev/runs/' + firstRun.id + '/' + firstRun.logs[0].name,
  );
});

test('invalid page parameters never reach a request', async () => {
  for (const [path, search] of [
    ['run', '?id=%3Cscript%3E'],
    ['run', '?id=../../v1/hosts'],
    ['cell', '?die=AD104&suite=..%2F..%2Fhosts'],
    ['cell', '?die=ad104&suite=linux-gate'],
    ['commit', '?sha=HEAD'],
  ]) {
    const page = await render('live', path, search);
    assert.equal(page.requests.length, 0, path + search);
    assert.match(page.document.getElementById('kf-status-state').textContent, /needs/);
  }
});

test('status.js has no URL literal and no HTML sink', () => {
  assert.doesNotMatch(code, /https?:\/\/|kayfabe\.dev|api\./);
  assert.doesNotMatch(
    code,
    /innerHTML|outerHTML|insertAdjacentHTML|document\.write|eval\(|new Function/,
  );
});
