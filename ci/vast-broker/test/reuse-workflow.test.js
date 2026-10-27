import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
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
    assert.match(section('report'), /proof\['url'\]/);
    assert.match(section('report'), /GPU_RESULT.*success/);
  });
