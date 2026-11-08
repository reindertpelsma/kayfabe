import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolveDependencies } from '../index.js';
const vectors = JSON.parse(
  readFileSync(new URL('../fixtures/dependency-vectors.json', import.meta.url)),
);
for (const v of vectors.vectors)
  for (const reverse of [false, true])
    test('shared dependency vector: ' + v.id + ' reversed=' + reverse, () => {
      const c = structuredClone(v.catalog);
      if (reverse) {
        c.runs.reverse();
        c.artifacts.reverse();
      }
      if (v.expected_error)
        assert.throws(
          () => resolveDependencies(c, v.requested, { available: v.available }),
          (e) => e.code === v.expected_error,
        );
      else
        assert.deepEqual(
          resolveDependencies(c, v.requested, { available: v.available }),
          v.expected_order,
        );
    });
import { plan } from '../index.js';
import { fixture } from './helpers.js';
for (const ids of [
  ['merge_bar', 'linux_apps_full'],
  ['windows_gate', 'windows_install_gate'],
  ['windows_gate', 'windows_gate'],
])
  test('real assignment uses shared resolver order: ' + ids.join(','), () => {
    const f = fixture();
    f.requirements.items[0].runs = ids;
    const p = plan(f);
    assert.deepEqual(
      p.assignments[0].runs.map((r) => r.run_id),
      resolveDependencies(f.catalog, ids),
    );
  });
