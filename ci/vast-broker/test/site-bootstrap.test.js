import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
for (const [event, ref, source] of [
  ['pull_request', 'refs/pull/7/merge', 'e'],
  ['push', 'refs/heads/master', 'a'],
  ['push', 'refs/tags/v1', 'd'],
  ['release', 'refs/tags/v1', 'd'],
])
  test('site bootstrap pins reviewed tooling ' + event + ' ' + ref, async () => {
    const directory = await mkdtemp(join(tmpdir(), 'wk2-site-bootstrap-'));
    try {
      const text = await readFile(
        new URL('../../../.github/workflows/site.yml', import.meta.url),
        'utf8',
      );
      const script = text
        .split('  policy:\n')[1]
        .split('  build:\n')[0]
        .split('        run: |\n')[1]
        .split('\n')
        .map((line) => (line.startsWith('          ') ? line.slice(10) : line))
        .join('\n');
      await writeFile(
        join(directory, 'gh'),
        `#!/usr/bin/env python3
import sys
assert sys.argv[1:]==['api','repos/reindertpelsma/kayfabe/commits/master','--jq','.sha']
print('d'*40)
`,
        { mode: 0o700 },
      );
      await writeFile(
        join(directory, 'curl'),
        `#!/usr/bin/env python3
import os,sys
a=sys.argv[1:]
assert a[-1]=='https://raw.githubusercontent.com/reindertpelsma/kayfabe/'+os.environ['EXPECTED']+'/ci/vast-broker/site_policy.py'
assert '--proto' in a and '=https' in a and '--max-time' in a and '--max-filesize' in a
source="import os,sys\\nassert sys.flags.isolated==1\\nopen(os.environ['GITHUB_OUTPUT'],'w').write('run=1\\\\ndeploy=1\\\\nknown=1\\\\n')\\n"
open(a[a.index('-o')+1],'w').write(source)
`,
        { mode: 0o700 },
      );
      const run = spawnSync('bash', ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script], {
        encoding: 'utf8',
        timeout: 5000,
        env: {
          ...process.env,
          PATH: directory + ':' + process.env.PATH,
          GITHUB_EVENT_NAME: event,
          GITHUB_REF: ref,
          GITHUB_SHA: 'a'.repeat(40),
          POLICY_REF: (event === 'pull_request' ? 'e' : 'a').repeat(40),
          EXPECTED: source.repeat(40),
          GITHUB_OUTPUT: join(directory, 'output'),
          GH_TOKEN: 'mock-read-only-token',
        },
      });
      assert.equal(run.status, 0, run.stderr);
      assert.match(await readFile(join(directory, 'output'), 'utf8'), /deploy=1/);
    } finally {
      await rm(directory, { recursive: true });
    }
  });
