// Source-level invariants of the public Worker (design sections 3 and 6).
// FALSIFIER: a statement that is not one plain SELECT; SQL assembled from strings; a statement
// that reads runs.tag, hosts_private or audit; any outbound request or GitHub reference in the
// Worker source; a second path to D1; the Worker bundling the row-tag key code; a secret binding.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { QUERIES } from '../src/queries.js';

const SRC = new URL('../src/', import.meta.url);
const sources = Object.fromEntries(
  readdirSync(SRC)
    .filter((name) => name.endsWith('.js'))
    .map((name) => [name, readFileSync(new URL(name, SRC), 'utf8')]),
);
const code = (text) => text.replace(/\/\/[^\n]*/g, '').replace(/\/\*[\s\S]*?\*\//g, '');

test('every statement is a single read-only SELECT', () => {
  const forbidden =
    /\b(INSERT|UPDATE|DELETE|REPLACE|UPSERT|CREATE|DROP|ALTER|PRAGMA|ATTACH|DETACH|VACUUM|REINDEX|TRUNCATE|RETURNING|BEGIN|COMMIT|ROLLBACK|SAVEPOINT|ANALYZE|load_extension)\b/i;
  assert.ok(Object.keys(QUERIES).length >= 10);
  assert.ok(Object.isFrozen(QUERIES));
  for (const [name, sql] of Object.entries(QUERIES)) {
    assert.equal(typeof sql, 'string', name);
    assert.match(sql.trim(), /^(SELECT|WITH)\s/i, name);
    assert.doesNotMatch(sql, /;/, name + ': one statement');
    assert.doesNotMatch(sql, forbidden, name);
    assert.doesNotMatch(sql, /--|\/\*/, name + ': no SQL comments');
    assert.doesNotMatch(sql, /\$\{|\?(?!\d)|:[a-z]|@[a-z]/i, name + ': numbered parameters only');
  }
});

test('queries.js builds no SQL from strings', () => {
  const text = code(sources['queries.js']);
  assert.doesNotMatch(text, /\$\{/, 'no template interpolation');
  assert.doesNotMatch(text, /`\s*\+|\+\s*`|'\s*\+|\+\s*'|"\s*\+|\+\s*"/, 'no concatenation');
  assert.doesNotMatch(text, /\.(concat|join|replace|replaceAll)\(/, 'no string assembly');
  assert.doesNotMatch(text, /\bimport\b/, 'no imported fragments');
});

test('no statement reads the row tag or a private table', () => {
  for (const [name, sql] of Object.entries(QUERIES)) {
    assert.doesNotMatch(sql, /\btag\b/i, name);
    assert.doesNotMatch(sql, /hosts_private|\baudit\b|address|box_token|provider_machine/i, name);
    assert.doesNotMatch(
      sql,
      /ci_runs_private|ci_requirement_snapshots|machine_id|price_usd|cost_usd|\.\*/i,
      name,
    );
  }
});

test('the Worker makes no outbound request and names no external service', () => {
  let definitions = 0;
  for (const [name, text] of Object.entries(sources)) {
    const body = code(text);
    for (const match of body.matchAll(/\bfetch\s*\(/g)) {
      assert.equal(name, 'index.js', name + ': fetch call');
      assert.match(body.slice(match.index - 6, match.index + 22), /async fetch\(request, env\)/);
      definitions++;
    }
    assert.doesNotMatch(text, /github/i, name);
    assert.doesNotMatch(body, /https?:\/\//, name + ': no URL literals');
    assert.doesNotMatch(
      body,
      /\bconnect\s*\(|WebSocket|XMLHttpRequest|EventSource|sendBeacon|\bimport\s*\(|caches\.|navigator\./,
      name,
    );
    assert.doesNotMatch(body, /\beval\s*\(|new Function/, name);
  }
  assert.equal(definitions, 1, 'exactly the fetch handler definition');
});

test('db.js is the only D1 access path and takes statement names only', () => {
  for (const [name, text] of Object.entries(sources)) {
    const calls = [...code(text).matchAll(/\.prepare\s*\(([^)]*)\)/g)];
    if (name === 'db.js') {
      assert.equal(calls.length, 1);
      assert.equal(calls[0][1].trim(), 'QUERIES[name]');
    } else assert.equal(calls.length, 0, name);
    if (name !== 'db.js') assert.doesNotMatch(code(text), /\.d1\b|env\.DB\./, name);
    assert.doesNotMatch(code(text), /\.(exec|batch|dump|raw)\s*\(/, name);
  }
  const env = new Set();
  for (const text of Object.values(sources))
    for (const m of code(text).matchAll(/\benv\.([A-Za-z_]+)/g)) env.add(m[1]);
  assert.deepEqual([...env].sort(), ['DB', 'LOGS_BASE']);
});

test('the Worker does not import the row-tag code', () => {
  for (const [name, text] of Object.entries(sources)) {
    assert.doesNotMatch(text, /row-tag|shared\//, name);
    for (const m of text.matchAll(/^import[^'"]*['"]([^'"]+)['"]/gm))
      assert.match(m[1], /^\.\/[a-z-]+\.js$/, name + ' imports ' + m[1]);
  }
});

test('wrangler.toml binds D1 and the custom domain and declares no secret', () => {
  const toml = readFileSync(new URL('../wrangler.toml', import.meta.url), 'utf8');
  assert.match(toml, /^main = "src\/index\.js"$/m);
  assert.match(toml, /^binding = "DB"$/m);
  assert.match(toml, /^database_name = "kayfabe-platform"$/m);
  assert.match(toml, /^database_id = "c6292be2-742d-4a05-95fd-749705ab30bc"$/m);
  assert.match(toml, /pattern = "api\.kayfabe\.dev", custom_domain = true/);
  assert.match(toml, /^LOGS_BASE = "https:\/\/logs\.kayfabe\.dev"$/m);
  assert.match(toml, /^workers_dev = false$/m);
  const config = toml.replace(/#[^\n]*/g, '');
  assert.doesNotMatch(config, /secret|token|key\s*=|password|durable_objects|r2_buckets|services/i);
});

test('the schema keeps private data in separate tables', () => {
  const sql = readFileSync(new URL('../migrations/0001_init.sql', import.meta.url), 'utf8');
  const table = (name) =>
    new RegExp('CREATE TABLE ' + name + ' \\(([\\s\\S]*?)\\n\\);').exec(sql)[1];
  assert.doesNotMatch(table('hosts'), /address|token|machine/);
  assert.match(table('hosts_private'), /address/);
  assert.doesNotMatch(table('runs'), /address|token/);
  const ci = readFileSync(new URL('../migrations/0002_ci_results.sql', import.meta.url), 'utf8');
  const ciTable = (name) =>
    new RegExp('CREATE TABLE ' + name + ' \\(([\\s\\S]*?)\\n\\);').exec(ci)[1];
  for (const name of ['ci_runs', 'ci_run_stages', 'ci_host_facts'])
    assert.doesNotMatch(ciTable(name), /machine|price|cost|token|address/, name);
  assert.match(ciTable('ci_runs_private'), /machine_id[\s\S]*price_usd_h[\s\S]*cost_usd/);
  // 0002 only adds: it never alters or drops what 0001 created.
  assert.doesNotMatch(ci.replace(/--[^\n]*/g, ''), /\b(?:ALTER|DROP|DELETE|UPDATE|INSERT)\b/i);
});
