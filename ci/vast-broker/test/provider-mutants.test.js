import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
test('provider invariants have two named killers per mutant', { timeout: 1200000 }, () => {
  const result = spawnSync(process.execPath, ['provider-session-mutants.mjs'], {
    cwd: fileURLToPath(new URL('../', import.meta.url)),
    encoding: 'utf8',
    timeout: 1100000,
    maxBuffer: 16 * 1024 * 1024,
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  const report = JSON.parse(result.stdout);
  assert.equal(report.status, 'PASS');
  assert.equal(report.cases.length, 116);
  console.log(JSON.stringify(report));
  for (const row of report.cases)
    assert.ok(row.kills >= 2 && new Set(row.killers).size >= 2, row.mutant);
});
