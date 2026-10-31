import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
// FALSIFIER: the box helper prints a token or presigned URL, sends Authorization to R2, keeps bytes
// that fail the caller's sha256 pin, or skips the dry-run/argument checks. The suite itself is
// scripts/bench/box/test_fetch_bench_data.sh (loopback only, real curl through a scheme shim).
const script = fileURLToPath(
  new URL('../../../scripts/bench/box/test_fetch_bench_data.sh', import.meta.url),
);
const tools = ['bash', 'curl', 'zstd', 'tar', 'python3', 'sha256sum', 'mkfifo'];
const missing = tools.filter((t) => spawnSync('sh', ['-c', 'command -v ' + t]).status !== 0);
test(
  'bench-data box helper: shell test suite passes',
  { skip: missing.length ? 'missing tools: ' + missing.join(' ') : false, timeout: 120000 },
  () => {
    const r = spawnSync('bash', [script], { encoding: 'utf8', timeout: 110000 });
    assert.equal(
      r.status,
      0,
      r.stdout
        .split('\n')
        .filter((l) => l.startsWith('FAIL'))
        .join('\n') + r.stderr,
    );
    assert.match(r.stdout, /passed \d+, failed 0/);
  },
);
