import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const repoRoot = fileURLToPath(new URL('../../../', import.meta.url));
// FALSIFIER: policy downloads selected/PR code, needs undeclared modules, or prints credentials.
for (const eventName of ['workflow_dispatch', 'unknown'])
  test('round actual policy bootstrap runs only BASE modules ' + eventName, async () => {
    const d = await mkdtemp(join(tmpdir(), 'v1-policy-bootstrap-'));
    try {
      const yaml = await readFile(repoRoot + '.github/workflows/gpu-vast.yml', 'utf8');
      const script = yaml
        .split('  policy:\n')[1]
        .split('  gpu:\n')[0]
        .split('        run: |\n')[1]
        .split('\n')
        .map((x) => (x.startsWith('          ') ? x.slice(10) : x))
        .join('\n');
      const curl = `#!/usr/bin/env python3
import os,sys,shutil
from pathlib import Path
args=sys.argv[1:];url=args[-1]
base='https://raw.githubusercontent.com/reindertpelsma/kayfabe/'+'d'*40+'/ci/vast-broker/'
assert url.startswith(base)
file=url[len(base):]
assert file in {'trigger-policy.mjs','package.json','defaults.json','src/defaults.js',
               'src/policy.js','src/github.js','src/workflow-authority.js'}
assert '-L' not in args and '--location' not in args
shutil.copyfile(Path(os.environ['SOURCE_ROOT'])/file,args[args.index('-o')+1])
`;
      await writeFile(join(d, 'curl'), curl, { mode: 0o700 });
      await writeFile(join(d, 'event'), '{}');
      const r = spawnSync(
        'bash',
        ['--noprofile', '--norc', '-x', '-eo', 'pipefail', '-c', script],
        {
          encoding: 'utf8',
          timeout: 5000,
          cwd: repoRoot,
          env: {
            ...process.env,
            PATH: d + ':' + process.env.PATH,
            SOURCE_ROOT: new URL('../', import.meta.url).pathname,
            GH_TOKEN: 'mock-policy-read-token',
            GITHUB_SHA: 'd'.repeat(40),
            TARGET_SHA: 'e'.repeat(40),
            GITHUB_EVENT_NAME: eventName,
            GITHUB_REPOSITORY: 'reindertpelsma/kayfabe',
            GITHUB_EVENT_PATH: join(d, 'event'),
            GITHUB_OUTPUT: join(d, 'output'),
            PATH_SKIP_PATTERNS: '["docs/**"]',
          },
        },
      );
      assert.equal(r.status, 0, r.stderr);
      assert.equal(await readFile(join(d, 'output'), 'utf8'), 'run=1\n');
      assert.ok(!(r.stdout + r.stderr).includes('mock-policy-read-token'));
    } finally {
      await rm(d, { recursive: true });
    }
  });
