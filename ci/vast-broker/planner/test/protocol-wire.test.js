import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import * as api from '../index.js';
const fixture = JSON.parse(
  readFileSync(new URL('../fixtures/protocol-wire-vectors.json', import.meta.url)),
);
for (const v of fixture.vectors)
  test('shared artifact wire vector: ' + v.name, () => {
    if (v.valid) assert.equal(api.checkSchema(v.schema, v.value), true);
    else
      assert.throws(
        () => api.checkSchema(v.schema, v.value),
        (e) => e instanceof api.PlanError,
      );
  });
test('artifact endpoint descriptor names implemented pure handlers and schemas', () => {
  const descriptor = JSON.parse(
    readFileSync(new URL('../protocol-endpoints.json', import.meta.url)),
  );
  assert.equal(descriptor.version, 1);
  assert.equal(descriptor.cache_control, 'no-store');
  assert.equal(descriptor.endpoints.length, 4);
  for (const e of descriptor.endpoints) {
    assert.equal(typeof api[e.handler], 'function');
    for (const name of [e.request, e.response].filter(Boolean))
      assert.ok(api.contractSchema.$defs[name]);
  }
});
