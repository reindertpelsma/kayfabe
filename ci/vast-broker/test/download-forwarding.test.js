import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
// Execute only the log-filter function; never execute provisioning or a Windows VM.
for (const name of ['provision_full', 'windows_gate']) {
  const source = readFileSync(
    new URL('../../../scripts/bench/box/' + name + '.sh', import.meta.url),
    'utf8',
  );
  const fn = source.match(/^emit_download_metrics\(\) \{ .* \}$/m)?.[0];
  for (const mode of ['filter', 'bound'])
    test('download forwarding ' + name + ' ' + mode, () => {
      assert.ok(fn);
      const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-forward-'));
      try {
        const log = join(scratch, 'fixture.log');
        writeFileSync(
          log,
          mode === 'bound'
            ? 'KF_DOWNLOAD_METRIC seconds=1 bytes=20\n'.repeat(50)
            : 'private-log-token\nhttps://example.invalid/private\nKF_DOWNLOAD_METRIC seconds=2.5 bytes=40\nKF_DOWNLOAD_METRIC seconds=secret bytes=50\n',
        );
        const r = spawnSync('bash', ['-c', fn + '\nemit_download_metrics "$1"', 'fixture', log], {
          encoding: 'utf8',
        });
        assert.equal(r.status, 0, r.stderr);
        if (mode === 'bound') assert.equal(r.stdout.trim().split('\n').length, 32);
        else assert.equal(r.stdout, 'KF_DOWNLOAD_METRIC seconds=2.5 bytes=40\n');
      } finally {
        rmSync(scratch, { recursive: true, force: true });
      }
    });
}
