import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
// 2026-11-05: gpu-vast.yml uploads the runner's sanitised full box log. The gpu job holds the Worker
// credentials, so the upload is the ONE action that job may hold, as an exact pinned step with no env.
// FALSIFIER: any other action in that job, a changed upload step (env, path, retention, pin, position),
// a widened permission, or an upload from another workflow/job passes workflow-guard.js.
const root = new URL('../../../', import.meta.url);
const text = await readFile(new URL('.github/workflows/gpu-vast.yml', root), 'utf8');
const sweep = await readFile(new URL('.github/workflows/gpu-vast-sweep.yml', root), 'utf8');
const { checkWorkflows } = await import('../workflow-guard.js');
const { runlogUploadStep, runnerBootstrap, sweepBootstrap } =
  await import('../workflow-bootstrap.js');
const PIN = 'actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02';
const guard = (gpu, extra = {}) => checkWorkflows({ 'gpu-vast.yml': gpu, ...extra });
const mutate = (old, replacement) => {
  assert.ok(text.includes(old), old);
  return text.replace(old, replacement);
};

test('runlog: the repository workflow passes and holds exactly the pinned upload', () => {
  assert.deepEqual(guard(text), []);
  assert.ok(text.includes(runlogUploadStep));
  assert.equal(text.split('upload-artifact').length - 1, 2); // the `uses:` and the comment
  assert.equal(
    [...text.matchAll(/^\s*uses: (\S+)/gm)].filter((m) => m[1].startsWith('actions/upload')).length,
    1,
  );
  assert.match(runlogUploadStep, new RegExp('uses: ' + PIN + ' # v4.6.2'));
  assert.match(runlogUploadStep, /retention-days: 30\n/);
  assert.match(runlogUploadStep, /if-no-files-found: ignore/);
  assert.match(runlogUploadStep, /continue-on-error: true/);
  assert.doesNotMatch(runlogUploadStep, /\benv:|secrets\.|WORKER_|vars\./);
  assert.match(runlogUploadStep, /path: \$\{\{ runner\.temp \}\}\/gpu-vast-runlog\/runlog\.txt\n/);
});

test('runlog: the job keeps exactly id-token write and the step is the last one', () => {
  const job = text.split('  gpu:\n')[1].split('  report:\n')[0];
  assert.match(job, /permissions:\n      id-token: write\n/);
  assert.doesNotMatch(job, /actions: |contents: |statuses: |GH_TOKEN|github\.token/);
  assert.ok(job.trimEnd().endsWith(runlogUploadStep.split('\n').pop()));
  assert.equal(job.indexOf('upload-artifact@') > job.indexOf('python3 -I'), true);
});

test('runlog: the runner gets the directory, the sweep does not', () => {
  assert.match(runnerBootstrap, /KF_RUNLOG_DIR="\$RUNNER_TEMP\/gpu-vast-runlog" \\\n/);
  assert.doesNotMatch(sweepBootstrap, /KF_RUNLOG_DIR/);
  assert.match(sweepBootstrap, /--profile sweep/);
  assert.doesNotMatch(sweep, /upload-artifact|KF_RUNLOG_DIR/);
  assert.deepEqual(checkWorkflows({ 'gpu-vast.yml': text, 'gpu-vast-sweep.yml': sweep }), []);
});

const MUTANTS = {
  'unpinned tag': [PIN, 'actions/upload-artifact@v4'],
  'other pin': [PIN, 'actions/upload-artifact@' + 'a'.repeat(40)],
  'retention 90': ['retention-days: 30', 'retention-days: 90'],
  'whole workspace path': [
    'path: ${{ runner.temp }}/gpu-vast-runlog/runlog.txt',
    'path: ${{ runner.temp }}',
  ],
  'secret in step env': [
    '        continue-on-error: true\n        uses: ' + PIN,
    '        continue-on-error: true\n        env:\n          X: ${{ secrets.VAST_BROKER_PATH }}\n        uses: ' +
      PIN,
  ],
  'always runs on reuse': [
    "needs.pending.outputs.reused != 'true' }}\n        continue-on-error",
    'true }}\n        continue-on-error',
  ],
  'token input': [
    '          overwrite: false',
    '          overwrite: false\n          github-token: ${{ github.token }}',
  ],
};
for (const [name, [old, replacement]] of Object.entries(MUTANTS))
  test('runlog: guard rejects a changed upload step: ' + name, () => {
    const errors = guard(mutate(old, replacement));
    assert.ok(errors.length, name);
  });

test('runlog: guard rejects a second action or a second upload in the gpu job', () => {
  const second = mutate(
    runlogUploadStep.split('\n')[0],
    '      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' +
      runlogUploadStep.split('\n')[0],
  );
  assert.ok(guard(second).length);
  assert.ok(
    guard(text.trimEnd().replace('\n  report:', '\n' + runlogUploadStep + '\n  report:')).length,
  );
  assert.ok(
    guard(
      mutate(
        '  report:\n',
        '  extra-upload:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: ' +
          PIN +
          '\n  report:\n',
      ),
    ).length,
  );
});

test('runlog: guard rejects a widened permission on the gpu job', () => {
  for (const permission of ['actions: write', 'contents: read', 'artifact-metadata: write'])
    assert.ok(
      guard(
        mutate(
          '      id-token: write\n    outputs',
          '      id-token: write\n      ' + permission + '\n    outputs',
        ),
      ).length,
      permission,
    );
});

test('runlog: guard rejects the upload step moved ahead of the broker step', () => {
  const head = text.split('      - name: Upload sanitised full box log')[0];
  const body = runlogUploadStep + '\n';
  const moved = head.replace(
    '      - name: Fetch BASE controller',
    body + '      - name: Fetch BASE controller',
  );
  assert.ok(guard(moved).length);
});

test('runlog: another workflow cannot upload artifacts from the sweep', () => {
  assert.ok(
    checkWorkflows({
      'gpu-vast.yml': text,
      'gpu-vast-sweep.yml': sweep.replace(
        '    steps:\n',
        '    steps:\n      - uses: ' + PIN + '\n',
      ),
    }).length,
  );
});
