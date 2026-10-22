import test from 'node:test';
import assert from 'node:assert/strict';
import { Github } from '../src/github.js';
// H6 FALSIFIER: native fetch receives Github as its this receiver and throws before I/O.
for (const token of [undefined, 'mock-job-token'])
  test('GitHub native fetch has no receiver ' + token, async () => {
    let calls = 0;
    const native = async function (_url, init) {
      assert.equal(this, undefined);
      calls++;
      return Response.json({ id: 123 });
    };
    const g = new Github(native, { repo: 'owner/repo' }, token);
    assert.equal((await g.run('123')).id, 123);
    assert.equal(calls, 1);
  });
for (const status of [401, 403])
  test('GitHub public fallback native fetch has no receiver ' + status, async () => {
    let calls = 0;
    const native = async function (_url, init) {
      assert.equal(this, undefined);
      calls++;
      return init.headers.Authorization ? new Response('', { status }) : Response.json({ id: 123 });
    };
    const g = new Github(native, { repo: 'owner/repo' }, 'mock-job-token');
    assert.equal((await g.run('123')).id, 123);
    assert.equal(calls, 2);
  });
