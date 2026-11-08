import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { plan } from '../index.js';
import { fixture } from './helpers.js';
const cli = new URL('../../kfci', import.meta.url);
for (const option of [undefined, '--without-seeds', '--assignments'])
  test('kfci plan shares pure implementation ' + option, () => {
    const f = fixture(),
      p = plan(f),
      r = spawnSync(process.execPath, [cli.pathname, 'plan', '-', ...(option ? [option] : [])], {
        input: JSON.stringify(f),
        encoding: 'utf8',
      });
    assert.equal(r.status, 0, r.stderr);
    assert.deepEqual(JSON.parse(r.stdout), option === '--assignments' ? p.assignments : p);
  });
for (const args of [[], ['rent', '-'], ['plan', '-', '--deploy']])
  test('CLI refuses unsupported side effects ' + args.join(' '), () => {
    const r = spawnSync(process.execPath, [cli.pathname, ...args], {
      input: '{}',
      encoding: 'utf8',
    });
    assert.equal(r.status, 2);
    assert.equal(JSON.parse(r.stderr).error, 'USAGE');
  });
