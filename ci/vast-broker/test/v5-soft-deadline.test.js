import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { repoSource } from './repo-onstart.js';
import { clock } from './helpers.js';
for (const elapsed of [173, 181])
  test('V5 expired creation soft deadline exits timeout ' + elapsed, async () => {
    const dir = await mkdtemp(tmpdir() + '/v5-soft-');
    try {
      await writeFile(dir + '/payload', 'touch ' + dir + '/executed\n');
      const source = repoSource('vast_onstart.sh');
      const a = source.indexOf('now=$(date +%s)');
      const z = source.indexOf("\n\n{\n    printf 'rc=%d", a);
      assert.ok(a >= 0 && z > a);
      const prefix = `repo='${dir}'\npayload=payload\nlog='${dir}/log'\nrc=0\ntimed_out=0\nreason=\nKF_SOFT_DEADLINE=${clock / 1000 + 170 * 60}\ndate(){ printf '%s\\n' ${clock / 1000 + elapsed * 60}; }\n`;
      const r = spawnSync('bash', ['-s'], {
        input: prefix + source.slice(a, z) + '\nprintf \'%s %s\' "$rc" "$timed_out"\n',
        encoding: 'utf8',
      });
      assert.equal(r.status, 0, r.stderr);
      assert.equal(r.stdout, '124 1');
      await assert.rejects(readFile(dir + '/executed'));
    } finally {
      await rm(dir, { recursive: true });
    }
  });
