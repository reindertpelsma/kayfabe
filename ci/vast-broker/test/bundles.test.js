// Tests of src/bundles.js: the port of scripts/ci/propimage_config.py. The shared parity cases are the contract (reasons included); the
// rest pins the JS-only parts and the committed table. Mutants: bundles-mutants.mjs (two named killers each).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  BUNDLES,
  ConfigError,
  MAX_APPS,
  MAX_BUNDLES,
  bundlesFor,
  downloadBytes,
  validate,
} from '../src/bundles.js';

const cases = JSON.parse(
  readFileSync(new URL('./fixtures/bundles/parity-cases.json', import.meta.url), 'utf8'),
);
const reasonOf = (fn) => {
  try {
    fn();
  } catch (e) {
    assert.ok(e instanceof ConfigError, String(e));
    return e.reason;
  }
  return null;
};
const doc = () => ({
  schema: 'kayfabe.propimage-bundles/1',
  bundles: {
    w1: { os: 'windows', r2: 'win/one.zip', size_bytes: 100, apps: ['alpha', 'beta'] },
    w2: { os: 'windows', r2: 'win/two.zip', size_bytes: 200, apps: ['beta', 'gamma'] },
    l1: { os: 'linux', r2: 'lin/one.zip', size_bytes: 50, apps: ['delta'] },
  },
});

for (const c of cases.validation)
  test('parity validation: ' + c.name, () => {
    assert.equal(
      reasonOf(() => validate(structuredClone(c.doc))),
      c.reason,
    );
  });
for (const c of cases.validation)
  test('parity validation through the resolver: ' + c.name, () => {
    let got = null;
    try {
      assert.deepEqual(bundlesFor(structuredClone(c.doc), []), []);
    } catch (e) {
      assert.ok(e instanceof ConfigError, String(e));
      got = e.reason;
    }
    assert.equal(got, c.reason);
  });
for (const c of cases.resolver)
  test('parity resolver: ' + c.name, () => {
    let got;
    try {
      got = { bundles: bundlesFor(structuredClone(c.doc), [...c.apps], c.os ?? null) };
    } catch (e) {
      assert.ok(e instanceof ConfigError, String(e));
      got = { error: e.reason };
    }
    assert.deepEqual(got, c.expect);
  });

for (const c of cases.resolver)
  test('parity resolver, request reversed: ' + c.name, () => {
    let got;
    try {
      got = { bundles: bundlesFor(structuredClone(c.doc), [...c.apps].reverse(), c.os ?? null) };
    } catch (e) {
      assert.ok(e instanceof ConfigError, String(e));
      got = { error: e.reason };
    }
    assert.deepEqual(got, c.expect);
  });

test('the committed table is valid and every bundle resolves to itself', () => {
  validate(BUNDLES);
  for (const [id, b] of Object.entries(BUNDLES.bundles))
    assert.deepEqual(bundlesFor(BUNDLES, b.apps), [id]);
});
test('the committed table has the five project bundles', () => {
  assert.deepEqual(Object.keys(BUNDLES.bundles).sort(), ['imga', 'imgb', 'imgc', 'imgd', 'imge']);
});
test('the committed sizes sum for a mixed request', () => {
  const ids = bundlesFor(BUNDLES, ['solidworks', 'matlab']);
  assert.deepEqual(ids, ['imga', 'imgc']);
  assert.equal(downloadBytes(BUNDLES, ids), 50496618964 + 8128391922);
});
test('the limits are the Python limits', () => {
  assert.equal(MAX_BUNDLES, 16);
  assert.equal(MAX_APPS, 64);
});
test('sixteen bundles and sixty-four apps are accepted, one more is not', () => {
  const many = (n) => ({
    schema: 'kayfabe.propimage-bundles/1',
    bundles: Object.fromEntries(
      Array.from({ length: n }, (_, i) => [
        'b' + i,
        { os: 'linux', r2: `k/${i}.zip`, size_bytes: 1, apps: ['a' + i] },
      ]),
    ),
  });
  validate(many(16));
  assert.equal(
    reasonOf(() => validate(many(17))),
    'BUNDLES',
  );
  const d = doc();
  d.bundles.w1.apps = Array.from({ length: 64 }, (_, i) => 'a' + i);
  validate(d);
  d.bundles.w1.apps.push('a64');
  assert.equal(
    reasonOf(() => validate(d)),
    'BUNDLE_APPS:w1',
  );
});
test('the request may be a Set; a string or a number list is not a request', () => {
  assert.deepEqual(bundlesFor(doc(), new Set(['alpha', 'gamma'])), ['w1', 'w2']);
  assert.equal(
    reasonOf(() => bundlesFor(doc(), 'alpha')),
    'APPS_SHAPE',
  );
  assert.equal(
    reasonOf(() => bundlesFor(doc(), [1])),
    'APPS_SHAPE',
  );
});
test('a bare string is not a request', () => {
  assert.equal(
    reasonOf(() => bundlesFor(doc(), 'alpha')),
    'APPS_SHAPE',
  );
});
test('a list of numbers is not a request', () => {
  assert.equal(
    reasonOf(() => bundlesFor(doc(), [1, 2])),
    'APPS_SHAPE',
  );
});
test('an invalid table is refused before resolving, even for an empty request', () => {
  const d = doc();
  d.bundles.w1.size_bytes = 0;
  assert.equal(
    reasonOf(() => bundlesFor(d, [])),
    'BUNDLE_SIZE:w1',
  );
});
test('resolving does not change the table or depend on the request order', () => {
  const d = doc();
  const before = structuredClone(d);
  assert.deepEqual(bundlesFor(d, ['gamma', 'alpha']), bundlesFor(d, ['alpha', 'gamma']));
  assert.deepEqual(d, before);
});
test('downloadBytes sums the named bundles and refuses an unknown one', () => {
  assert.equal(downloadBytes(doc(), ['w1', 'w2', 'l1']), 350);
  assert.equal(downloadBytes(doc(), []), 0);
  assert.equal(
    reasonOf(() => downloadBytes(doc(), ['w1', 'w9'])),
    'UNKNOWN_BUNDLE',
  );
  assert.equal(
    reasonOf(() => downloadBytes(doc(), ['nobody'])),
    'UNKNOWN_BUNDLE',
  );
});
test('a prototype name is not a bundle', () => {
  assert.equal(
    reasonOf(() => downloadBytes(doc(), ['constructor'])),
    'UNKNOWN_BUNDLE',
  );
});
test('an Object method name is not a bundle either', () => {
  assert.equal(
    reasonOf(() => downloadBytes(doc(), ['toString'])),
    'UNKNOWN_BUNDLE',
  );
  assert.equal(
    reasonOf(() => downloadBytes(doc(), ['hasOwnProperty', 'w1'])),
    'UNKNOWN_BUNDLE',
  );
});
