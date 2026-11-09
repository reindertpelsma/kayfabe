import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { publicLogRenderer } from '../public-log.js';
import { ipPrefix } from '../src/rental.js';
for (const version of [4, 6]) {
  test('public renderer masks all endpoint paths IPv' + version, async () => {
    const dir = await mkdtemp(join(tmpdir(), 'kf-provider-public-'));
    try {
      const ip = version === 4 ? '203.0.113.77' : '2001:db8:1234:5678:90ab:cdef:1234:5678';
      const raw = 'failure at [' + ip + ']:45678 ssh_port=45678 "ssh_port": 45678 ssh -p 45678\n';
      const data = {
        id: '900',
        machine_id: 1,
        host_id: 2,
        datacenter: true,
        country: 'US',
        ip_prefix: ipPrefix(ip),
        gpu_model: 'GTX_1660',
        dph_total: 0.82,
        log_increment_sanitised: raw,
        log_tail_sanitised: raw,
        geolocation: raw,
        verdict: 'FAIL',
        state: 'NONE',
      };
      const meta = join(dir, 'data.json'),
        log = join(dir, 'log.txt'),
        summary = join(dir, 'summary');
      await writeFile(meta, JSON.stringify(data));
      await writeFile(log, raw);
      await writeFile(summary, '');
      for (const mode of ['live', 'final', 'summary']) {
        const result = spawnSync('python3', ['-I', '-c', publicLogRenderer, mode, meta, log], {
          encoding: 'utf8',
          env: { ...process.env, GITHUB_STEP_SUMMARY: summary },
        });
        assert.equal(result.status, 0, result.stderr);
        const text = result.stdout + (await readFile(summary, 'utf8'));
        assert.equal(text.includes(ip), false);
        assert.equal(text.includes('45678'), false);
        if (mode === 'live')
          assert.match(
            result.stdout,
            /^RENTAL instance=900 host=2 machine=1 datacenter=true country=US ip=/,
          );
      }
    } finally {
      await rm(dir, { recursive: true });
    }
  });
}
