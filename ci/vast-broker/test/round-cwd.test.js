import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
// FALSIFIER V11: the documented `npm test` in ci/vast-broker fails on cwd-relative paths.
const broker = fileURLToPath(new URL('../', import.meta.url));
for (const file of [
  'broker.test.js',
  'round-lifecycle.test.js',
  'round-policy-bootstrap.test.js',
  'round-workflow-policy.test.js',
])
  for (const [where, cwd] of [
    ['broker directory', broker],
    ['foreign directory', tmpdir()],
  ])
    test('round suite file passes from the ' + where + ' ' + file, () => {
      const run = spawnSync(process.execPath, ['--test', broker + 'test/' + file], {
        cwd,
        encoding: 'utf8',
        timeout: 120000,
        env: Object.fromEntries(
          Object.entries(process.env).filter(([k]) => k !== 'NODE_TEST_CONTEXT'),
        ),
      });
      assert.equal(run.status, 0, run.stdout.slice(-1500));
    });
