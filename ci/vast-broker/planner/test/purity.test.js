import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { plan, contractSchema, validatePlan } from '../index.js';
import { fixture } from './helpers.js';
for (const provider of ['vast', 'owned'])
  test('pure API never consults clock RNG or I/O ' + provider, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.provider = provider));
    const expected = plan(f),
      oldFetch = globalThis.fetch,
      oldNow = Date.now,
      oldRandom = Math.random;
    try {
      globalThis.fetch = () => {
        throw Error('unexpected network');
      };
      Date.now = () => {
        throw Error('unexpected clock');
      };
      Math.random = () => {
        throw Error('unexpected RNG');
      };
      assert.deepEqual(plan(f), expected);
      validatePlan(f, expected);
    } finally {
      globalThis.fetch = oldFetch;
      Date.now = oldNow;
      Math.random = oldRandom;
    }
  });
for (const field of ['catalog', 'assignment'])
  test('published schema cannot mutate the validator ' + field, () => {
    assert.ok(Object.isFrozen(contractSchema));
    assert.ok(Object.isFrozen(contractSchema.$defs[field]));
    assert.throws(() => (contractSchema.$defs[field].properties.version.const = 2), TypeError);
    assert.equal(contractSchema.$defs[field].properties.version.const, 1);
  });
test('pure core import graph stays within the module and static schemas', () => {
  const root = new URL('../', import.meta.url);
  const names = readdirSync(root).filter((n) => n.endsWith('.js'));
  for (const name of names) {
    const source = readFileSync(new URL(name, root), 'utf8');
    for (const m of source.matchAll(/\bfrom\s*['"]([^'"]+)['"]/g))
      assert.ok(
        m[1].startsWith('./') && !m[1].includes('..'),
        'external runtime import ' + name + ':' + m[1],
      );
    assert.ok(
      !/\b(?:fetch|eval|setTimeout|setInterval)\s*\(|\b(?:Date|process|Buffer)\b|Math\.random/.test(
        source.replace(/\/\/[^\n]*/g, ''),
      ),
      name + ' contains a runtime effect',
    );
  }
});
