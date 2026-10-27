import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { checkWorkflows } from '../workflow-guard.js';
const text = readFileSync(
  new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
  'utf8',
);
const section = (id) => text.split('  ' + id + ':\n')[1]?.split(/^  [a-z][a-z-]*:/m)[0] ?? '';
for (const label of ['parent proof', 'readable result'])
  test('reuse workflow executes the original named job: ' + label, () => {
    assert.match(section('pending'), /reused:.*steps.reuse.outputs.reused/);
    assert.match(section('gpu'), /needs.pending.outputs.reused == 'true'/);
    assert.match(section('gpu'), /name: Replay verified parent result/);
    assert.match(section('gpu'), /gate_reuse.py.*replay/);
    assert.match(section('gpu'), /if:.*needs.pending.outputs.reused != 'true'/);
  });
for (const label of ['final status', 'original run link'])
  test('reuse workflow preserves the status job: ' + label, () => {
    assert.match(section('report'), /REUSE_PROOF:/);
    assert.match(section('report'), /reused: same tree verified on/);
    assert.match(section('report'), /proof\[[\"']url[\"']\]/);
    assert.match(section('report'), /GPU_RESULT.*success/);
  });

for (const name of ['named replay job', 'scoped metadata permissions'])
  test('reuse guard permits ' + name, () => {
    assert.deepEqual(checkWorkflows({ 'gpu-vast.yml': text }), []);
  });
for (const permission of ['checks', 'contents'])
  test('reuse guard refuses write permission ' + permission, () => {
    assert.ok(
      checkWorkflows({
        'gpu-vast.yml': text.replace(
          '      ' + permission + ': read',
          '      ' + permission + ': write',
        ),
      }).length,
    );
  });
for (const command of ['prepare', 'steal'])
  test('reuse guard refuses changed replay command ' + command, () => {
    assert.ok(
      checkWorkflows({
        'gpu-vast.yml': text.replace('gate_reuse.py" replay', 'gate_reuse.py" ' + command),
      }).length,
    );
  });
for (const condition of ["if: needs.pending.outputs.reused == 'true'", 'if: always()'])
  test('reuse guard refuses broker condition ' + condition, () => {
    assert.ok(
      checkWorkflows({
        'gpu-vast.yml': text.replace("if: needs.pending.outputs.reused != 'true'", condition),
      }).length,
    );
  });
