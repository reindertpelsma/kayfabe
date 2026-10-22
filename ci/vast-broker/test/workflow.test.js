import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
const fixtures = [];
after(async () => {
  for (const path of fixtures) await rm(path, { recursive: true });
});
const root = new URL('../../../', import.meta.url);
const workflow = new URL('.github/workflows/gpu-vast.yml', root);
const moduleGuard = () => import('../workflow-guard.js');
const trustedFixture = await readFile(workflow, 'utf8');
const simple = (extra) => trustedFixture + extra;
test('actual repository has safe workflow boundaries', async () => {
  const { checkRepository } = await moduleGuard();
  assert.deepEqual(await checkRepository(root), []);
});
test('poll job permits exactly OIDC', async () => {
  const { checkWorkflows } = await moduleGuard();
  const text = await readFile(workflow, 'utf8');
  assert.deepEqual(checkWorkflows({ 'gpu-vast.yml': text }), []);
  assert.match(text, /statuses: write/);
  assert.doesNotMatch(text, /merge_group:|upload-artifact|ssh /);
});
for (const permission of [
  'attestations: write',
  'actions: write',
  'contents: write',
  'packages: write',
])
  test('extra workflow permission fails ' + permission, async () => {
    const { checkWorkflows } = await moduleGuard();
    const t = await readFile(workflow, 'utf8');
    assert.ok(
      checkWorkflows({
        'gpu-vast.yml': t.replace(
          '      id-token: write',
          '      id-token: write\n      ' + permission,
        ),
      }).length,
    );
  });
test('direct attestation dependency is rejected', async () => {
  const { checkWorkflows } = await moduleGuard();
  assert.ok(
    checkWorkflows({
      'gpu-vast.yml': simple(
        '  attest:\n    needs: gpu\n    steps:\n      - uses: actions/attest@v2\n',
      ),
    }).length,
  );
});
test('transitive release dependency is rejected', async () => {
  const { checkWorkflows } = await moduleGuard();
  assert.ok(
    checkWorkflows({
      'gpu-vast.yml': simple(
        '  bridge:\n    needs: [gpu]\n    steps: []\n  release:\n    needs: bridge\n    steps:\n      - run: gh release create x\n',
      ),
    }).length,
  );
});
test('cross-workflow GPU consumer is rejected', async () => {
  const { checkWorkflows } = await moduleGuard();
  assert.ok(
    checkWorkflows({
      'gpu-vast.yml': simple(''),
      'release.yml':
        'on:\n  workflow_run:\n    workflows: [Vast GPU]\njobs:\n  release:\n    steps:\n      - uses: actions/attest@v2\n',
    }).length,
  );
});
test('reusable GPU workflow cannot hide attestation', async () => {
  const { checkWorkflows } = await moduleGuard();
  assert.ok(
    checkWorkflows({
      'gpu-vast.yml': simple('').replace(
        '    steps:',
        '    uses: ./.github/workflows/opaque.yml\n    steps:',
      ),
    }).length,
  );
});
test('dashboard bundle parses and exports source handlers', async () => {
  const r = spawnSync(process.execPath, ['bundle.js'], {
    cwd: new URL('../', import.meta.url),
    encoding: 'utf8',
  });
  assert.equal(r.status, 0, r.stderr);
  const source = await import('../src/index.js');
  const bundle = await import('../dist/worker.js');
  assert.deepEqual(Object.keys(bundle).sort(), Object.keys(source).sort());
  assert.equal(typeof bundle.default.fetch, 'function');
  assert.equal(typeof bundle.default.scheduled, 'function');
});
test('bundle exercises the same refusal behavior', async () => {
  const r = spawnSync(process.execPath, ['bundle.js'], {
    cwd: new URL('../', import.meta.url),
    encoding: 'utf8',
  });
  assert.equal(r.status, 0, r.stderr);
  const source = await import('../src/index.js');
  const bundle = await import('../dist/worker.js');
  const { env, MemoryStorage, MockCloud } = await import('./helpers.js');
  for (const m of [source, bundle]) {
    const s = m.createService({
      env: { ...env, RENTING_ENABLED: 'false' },
      storage: new MemoryStorage(),
      fetch: new MockCloud().fetch,
    });
    assert.equal(
      (await s.fetch(new Request(env.WORKER_URL + '/rent', { method: 'POST' }))).status,
      503,
    );
  }
});
test('box deliberate control exits failure on missing evidence', async () => {
  const path = new URL('scripts/ci/vast_box_main.sh', root);
  await readFile(path);
  const r = spawnSync('bash', [path.pathname, '--must-fail-control'], {
    encoding: 'utf8',
    timeout: 5000,
  });
  assert.notEqual(r.status, 0);
  assert.match(r.stdout + r.stderr, /BIRTH_CENSUS_FAIL/);
});
test('box harness accepts only a deliberately failed control', async () => {
  const path = new URL('scripts/ci/vast_box_main.sh', root);
  await readFile(path);
  const r = spawnSync('bash', [path.pathname, '--check-control'], {
    encoding: 'utf8',
    timeout: 5000,
  });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /MUST_FAIL_CONTROL_FAILED_AS_REQUIRED/);
});
for (const defect of ['none', 'guest-fail', 'bare-fail', 'gate-fail', 'missing'])
  test('box suite evidence validation ' + defect, async () => {
    const path = new URL('scripts/ci/vast_box_main.sh', root);
    await readFile(path);
    const dir = await mkdtemp(join(tmpdir(), 'v1-box-test-'));
    fixtures.push(dir);
    const files = {
      'gates.log': 'V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS\n',
      'bare.log': 'BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30\n',
      'suite.log': 'FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30\n',
      'births.log': 'BIRTH_CENSUS_OK arms=30\n',
    };
    for (const [name, text] of Object.entries(files))
      if (!(defect === 'missing' && name === 'births.log'))
        await writeFile(
          join(dir, name),
          text.replace(
            defect === 'guest-fail'
              ? 'FAST_SUITE_PASS=30'
              : defect === 'bare-fail'
                ? 'BARE_SUITE_PASS=30'
                : defect === 'gate-fail'
                  ? 'pass=9'
                  : '__never__',
            'bad',
          ),
        );
    const r = spawnSync('bash', [path.pathname, '--check-results', dir], {
      encoding: 'utf8',
      timeout: 5000,
    });
    assert.equal(r.status === 0, defect === 'none', r.stderr);
  });
test('flow-style attestation job cannot bypass the dependency guard', async () => {
  const { checkWorkflows } = await moduleGuard();
  assert.ok(
    checkWorkflows({
      'gpu-vast.yml': simple('  attest: {needs: gpu, steps: [{uses: actions/attest@v2}]}\n'),
    }).length,
  );
});
test('flow-style release dependency cannot be silently ignored', async () => {
  const { checkWorkflows } = await moduleGuard();
  assert.ok(
    checkWorkflows({ 'gpu-vast.yml': simple('  release: {needs: gpu, steps: []}\n') }).length,
  );
});
test('quoted release job cannot bypass structural parsing', async () => {
  const { checkWorkflows } = await moduleGuard();
  assert.ok(
    checkWorkflows({ 'gpu-vast.yml': simple("  'release':\n    needs: gpu\n    steps: []\n") })
      .length,
  );
});
for (const bytes of ['22000000000', '59999999999', '60000000000', 'unknown'])
  test('cold-box disk preflight ' + bytes, async () => {
    const path = new URL('scripts/ci/vast_box_main.sh', root);
    await readFile(path);
    const dir = await mkdtemp(join(tmpdir(), 'v1-disk-test-'));
    fixtures.push(dir);
    await writeFile(join(dir, 'df'), '#!/bin/sh\nprintf "Available\\n' + bytes + '\\n"\n', {
      mode: 0o700,
    });
    const r = spawnSync('bash', [path.pathname, '--check-disk'], {
      encoding: 'utf8',
      timeout: 5000,
      env: { ...process.env, PATH: dir + ':' + process.env.PATH },
    });
    assert.equal(r.status === 0, bytes === '60000000000');
    if (bytes !== '60000000000') assert.match(r.stdout, /VAST_CI_DISK_UNAVAILABLE/);
  });

test('OIDC-only broker and status writer tokens stay separate', async () => {
  const t = await readFile(workflow, 'utf8');
  const gpu = t.split('  gpu:\n')[1].split('  report:\n')[0];
  assert.doesNotMatch(gpu, /contents:|pull-requests:|GH_TOKEN|github.token/);
  assert.doesNotMatch(gpu, /statuses: write|\/statuses\//);
  assert.match(gpu, /id-token: write/);
  for (const id of ['pending', 'report']) {
    const block = t.split('  ' + id + ':\n')[1]?.split(/\n  [\w-]+:\n/)[0];
    assert.ok(block);
    assert.match(block, /statuses: write/);
    assert.doesNotMatch(block, /X-GH-Token|id-token: write/);
  }
});
test('final status job runs even on skipped or failed broker', async () => {
  const t = await readFile(workflow, 'utf8');
  const block = t.split('  report:\n')[1];
  assert.ok(block);
  assert.ok(block.includes('if: ${{ always() }}'));
  assert.match(block, /needs: \[pending, gpu\]/);
});
for (const id of ['pending', 'report'])
  test('missing status writer job fails the guard ' + id, async () => {
    const { checkWorkflows } = await moduleGuard();
    const t = await readFile(workflow, 'utf8');
    assert.ok(
      checkWorkflows({ 'gpu-vast.yml': t.replace('  ' + id + ':\n', '  removed-' + id + ':\n') })
        .length,
    );
  });
for (const id of ['pending', 'report'])
  test('status writer cannot gain content permission ' + id, async () => {
    const { checkWorkflows } = await moduleGuard();
    const t = await readFile(workflow, 'utf8');
    const start = t.indexOf('  ' + id + ':\n');
    const changed =
      t.slice(0, start) + t.slice(start).replace('statuses: write', 'contents: write');
    assert.ok(checkWorkflows({ 'gpu-vast.yml': changed }).length);
  });
test('workflow mutation fixture is valid before adding a forbidden consumer', async () => {
  const { checkWorkflows } = await moduleGuard();
  assert.deepEqual(checkWorkflows({ 'gpu-vast.yml': simple('') }), []);
});
