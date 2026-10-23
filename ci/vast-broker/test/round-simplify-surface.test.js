import test from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const repoRoot = fileURLToPath(new URL('../../../', import.meta.url));
const text = (path) => readFile(repoRoot + path, 'utf8');
const exists = (path) =>
  access(repoRoot + path).then(
    () => true,
    () => false,
  );
// FALSIFIER H-SURFACE (owner 19:10): a retired manual/hold, Access-switch or dispatch-SHA
// surface remains reachable from the runner, the box helpers or the Worker source tree.
// (The box Access pair pass-through returned on 21:45; see box-access.test.js.)
for (const path of [
  'scripts/ci/kf-box-hold',
  'scripts/ci/kf-ci-hold.sh',
  'ci/vast-broker/src/access.js',
])
  test('simplify retired file is absent ' + path, async () => {
    assert.equal(await exists(path), false);
  });
for (const [path, pattern] of [
  ['ci/vast-broker/runner.py', /MANUAL|manual_deadline_at|"sha", ""\)/],
  ['ci/vast-broker/src/index.js', /MANUAL|stopTimer|hold\(|AccessPolicy/],
  ['ci/vast-broker/src/routes.js', /stop-timer|hold/],
  ['ci/vast-broker/src/policy.js', /APPROVAL_MODE|RENT_DRAFTS|ALLOWED_BASE|PATH_SKIP|MANUAL_/],
  ['ci/vast-broker/defaults.json', /manualCeiling|boxHold/],
  ['ci/vast-broker/docs/BOX_PROTOCOL.md', /\/hold|stop-timer|MANUAL/],
])
  test('simplify retired surface is absent from ' + path, async () => {
    assert.doesNotMatch(await text(path), pattern);
  });
for (const ref of ['refs/heads/master', 'refs/heads/feature'])
  test('simplify runner refuses check-profile dispatch ' + ref, () => {
    const p = spawnSync(
      'python3',
      ['-I', 'ci/vast-broker/test/round3_main_fixture.py', JSON.stringify({ manual: true, ref })],
      { encoding: 'utf8', timeout: 5000, cwd: repoRoot },
    );
    assert.equal(p.status, 0, p.stderr);
    const r = JSON.parse(p.stdout);
    assert.equal(r.status, 1);
    assert.equal(r.calls.filter((x) => x.url.endsWith('/rent')).length, 0);
  });
