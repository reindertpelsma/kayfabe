import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
// FALSIFIER: a plain pinned wrangler deploy rebuilds unreviewed source bytes.
for (const [name, pattern] of [
  ['main', /^main = "dist\/worker.js"$/m],
  ['no-bundle', /^no_bundle = true$/m],
])
  test('production deploy binds hashed module ' + name, async () => {
    const source = await readFile(new URL('../wrangler.toml', import.meta.url), 'utf8');
    assert.match(source, pattern);
  });
