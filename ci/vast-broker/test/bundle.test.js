import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { env, MemoryStorage, MockCloud } from './helpers.js';
// Owner addendum 19:45: wrangler bundles src/ from the lockfile-pinned dependencies; there is no
// project linker. deployment.json pins the SHA-256 of wrangler's emitted module.
// FALSIFIER: the committed hashes disagree, wrangler's output is not the reviewed hash, differs
// between two runs, needs a Node builtin, or behaves differently from the source module.
const broker = fileURLToPath(new URL('../', import.meta.url));
const wrangler = join(broker, 'node_modules/.bin/wrangler');
const tomlVar = (toml, key) => new RegExp('^' + key + ' = "([^"]*)"$', 'm').exec(toml)?.[1];
test('bundle: deployment.json and wrangler.toml name the same reviewed hash', async () => {
  const manifest = JSON.parse(await readFile(join(broker, 'deployment.json'), 'utf8'));
  const toml = await readFile(join(broker, 'wrangler.toml'), 'utf8');
  assert.match(manifest.bundle_sha256, /^[0-9a-f]{64}$/);
  assert.equal(tomlVar(toml, 'DEPLOYED_BUNDLE_SHA256'), manifest.bundle_sha256);
});
test('bundle: wrangler bundles the real entry, no project linker', async () => {
  const toml = await readFile(join(broker, 'wrangler.toml'), 'utf8');
  assert.match(toml, /^main = "src\/index\.js"$/m);
  assert.doesNotMatch(toml, /^no_bundle/m);
  assert.equal(existsSync(join(broker, 'bundle.js')), false);
});
async function emit() {
  const out = await mkdtemp(join(tmpdir(), 'kf-vast-bundle-test-'));
  const run = spawnSync(
    wrangler,
    ['deploy', '--config', 'wrangler.toml', '--dry-run', '--outdir', out],
    {
      cwd: broker,
      encoding: 'utf8',
      timeout: 120000,
      env: { ...process.env, WRANGLER_SEND_METRICS: 'false' },
    },
  );
  assert.equal(run.status, 0, run.stderr);
  return { out, bytes: await readFile(join(out, 'index.js')) };
}
const skip = existsSync(wrangler) ? false : 'wrangler not installed (npm ci --ignore-scripts)';
test(
  'bundle: wrangler output is the reviewed, deterministic, Node-free module',
  { skip },
  async () => {
    const [a, b] = [await emit(), await emit()];
    try {
      const manifest = JSON.parse(await readFile(join(broker, 'deployment.json'), 'utf8'));
      const digest = createHash('sha256').update(a.bytes).digest('hex');
      assert.equal(createHash('sha256').update(b.bytes).digest('hex'), digest, 'deterministic');
      assert.equal(digest, manifest.bundle_sha256, 'reviewed hash');
      const text = a.bytes.toString('utf8');
      assert.doesNotMatch(text, /\bfrom\s*["']node:|\brequire\(|\bimport\(\s*["']/);
      const source = await import('../src/index.js');
      const bundle = await import(pathToFileURL(join(a.out, 'index.js')).href);
      assert.deepEqual(Object.keys(bundle).sort(), Object.keys(source).sort());
      // Same refusal behaviour as the source module.
      for (const m of [source, bundle]) {
        const s = m.createService({
          env: { ...env, RENTING_ENABLED: 'false' },
          storage: new MemoryStorage(),
          fetch: new MockCloud().fetch,
        });
        const r = await s.fetch(new Request(env.WORKER_URL + '/rent', { method: 'POST' }));
        assert.equal(r.status, 503);
      }
      // The scheduled binding reaches Durable Object cleanup; the cron route is not public.
      let swept = 0,
        forwarded = 0,
        promise;
      const object = new bundle.BrokerLedger({ storage: new MemoryStorage() }, env);
      object.service = { sweep: async () => swept++, fetch: async () => new Response('', {}) };
      bundle.default.scheduled(
        {},
        { LEDGER: { idFromName: (n) => n, get: () => object } },
        { waitUntil: (p) => (promise = p) },
      );
      await promise;
      assert.equal(swept, 1);
      const ledger = { fetch: () => (forwarded++, new Response('bad')) };
      const response = await bundle
        .createGateway({ authorize: async () => {} })
        .fetch(new Request('https://broker.example/cron', { method: 'POST' }), {
          LEDGER: { idFromName: (n) => n, get: () => ledger },
        });
      assert.equal(response.status, 404);
      assert.equal(forwarded, 0);
    } finally {
      await rm(a.out, { recursive: true });
      await rm(b.out, { recursive: true });
    }
  },
);
