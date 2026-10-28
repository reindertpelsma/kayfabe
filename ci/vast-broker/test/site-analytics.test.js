import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const site = readFileSync(new URL('../../../.github/workflows/site.yml', import.meta.url), 'utf8');
const block = (id) => site.split('  ' + id + ':\n')[1]?.split(/^  [a-z][a-z-]*:/m)[0] ?? '';

test('analytics variable is scoped only to the Pages deploy job', () => {
  assert.doesNotMatch(block('policy') + block('build'), /WEB_ANALYTICS_SNIPPET|vars\./);
  assert.match(block('deploy'), /WEB_ANALYTICS_SNIPPET: \$\{\{ vars.WEB_ANALYTICS_SNIPPET \}\}/);
});
test('ordinary render and output tests run before the uninstrumented upload', () => {
  assert.match(block('build'), /python3 site\/test_output.py/);
  assert.doesNotMatch(block('build'), /python3 (?:-I )?site\/deploy_analytics\.py/);
  assert.match(block('deploy'), /name: github-pages-deploy/);
  assert.match(block('deploy'), /artifact_name: github-pages-deploy/);
});
test('analytics transformer uses the exact build tooling revision without checkout', () => {
  assert.match(block('build'), /tooling_sha: \$\{\{ steps.tooling.outputs.sha \}\}/);
  assert.match(block('deploy'), /TOOLING_REF: \$\{\{ needs.build.outputs.tooling_sha \}\}/);
  assert.match(block('deploy'), /\$source_origin\/\$TOOLING_REF\/site\/deploy_analytics.py/);
  assert.doesNotMatch(block('deploy'), /actions\/checkout|contents:|actions: write/);
});
test('deploy preserves policy guards and passes HTML only as environment data', () => {
  assert.match(block('deploy'), /SITE_DEPLOY_ENVIRONMENT: github-pages/);
  assert.match(block('deploy'), /needs.policy.outputs.deploy == '1'/);
  assert.match(block('deploy'), /github.event_name != 'pull_request'/);
  assert.match(block('deploy'), /python3 -I .*deploy_analytics.py/);
  assert.doesNotMatch(block('deploy').split('run: |')[1], /vars.WEB_ANALYTICS_SNIPPET/);
});

test('deploy publishes the transformed archive under its distinct artifact identity', () => {
  const deploy = block('deploy');
  const uploads = deploy.split('actions/upload-artifact@')[1];
  const publishing = deploy.split('actions/deploy-pages@')[1];
  const uploadedName = uploads.match(/name: (\S+)/)?.[1];
  const publishedName = publishing.match(/artifact_name: (\S+)/)?.[1];
  assert.equal(uploadedName, publishedName);
  assert.notEqual(publishedName, 'github-pages');
  assert.match(uploads, /pages-deploy\/artifact.tar/);
});

for (const sha of ['a'.repeat(40), 'b'.repeat(40)])
  test(
    'deploy shell fetches only its exact tested tooling and treats HTML as data ' + sha[0],
    async () => {
      const { mkdtemp, mkdir, writeFile, readFile, rm } = await import('node:fs/promises');
      const { join } = await import('node:path');
      const { tmpdir } = await import('node:os');
      const { spawnSync } = await import('node:child_process');
      const directory = await mkdtemp(join(tmpdir(), 'site-deploy-analytics-'));
      try {
        await mkdir(join(directory, 'pages-deploy'));
        const archive = join(directory, 'pages-deploy', 'artifact.tar');
        const create = spawnSync('python3', [
          '-c',
          `
import io,sys,tarfile
page=b'<html><head><title>X</title></head><body>Y</body></html>'
with tarfile.open(sys.argv[1],'w') as target:
    member=tarfile.TarInfo('index.html'); member.size=len(page)
    target.addfile(member,io.BytesIO(page))
`,
          archive,
        ]);
        assert.equal(create.status, 0);
        await writeFile(
          join(directory, 'curl'),
          `#!/usr/bin/env python3
import os,shutil,sys
a=sys.argv[1:]
origin='https://raw.githubusercontent.com/reindertpelsma/kayfabe/'
assert a[-1]==origin+os.environ['TOOLING_REF']+'/site/deploy_analytics.py'
assert '--proto' in a and '=https' in a and '--max-filesize' in a
shutil.copyfile(os.environ['TEST_TRANSFORMER'],a[a.index('-o')+1])
`,
          { mode: 0o700 },
        );
        const script = block('deploy')
          .split('        run: |\n')[1]
          .split('\n      - uses:')[0]
          .split('\n')
          .map((line) => (line.startsWith('          ') ? line.slice(10) : line))
          .join('\n');
        const snippet = '<script data-example="$(touch shell-executed)"></script>';
        const run = spawnSync('bash', ['--noprofile', '--norc', '-c', script], {
          cwd: directory,
          encoding: 'utf8',
          timeout: 5000,
          env: {
            ...process.env,
            PATH: directory + ':' + process.env.PATH,
            RUNNER_TEMP: directory,
            TOOLING_REF: sha,
            TEST_TRANSFORMER: new URL('../../../site/deploy_analytics.py', import.meta.url)
              .pathname,
            GITHUB_ACTIONS: 'true',
            GITHUB_REPOSITORY: 'reindertpelsma/kayfabe',
            GITHUB_JOB: 'deploy',
            GITHUB_EVENT_NAME: 'push',
            GITHUB_REF: 'refs/heads/master',
            SITE_DEPLOY_ENVIRONMENT: 'github-pages',
            WEB_ANALYTICS_SNIPPET: snippet,
          },
        });
        assert.equal(run.status, 0, run.stderr);
        assert.doesNotMatch(run.stdout + run.stderr, /data-example/);
        await assert.rejects(readFile(join(directory, 'shell-executed')));
        const inspect = spawnSync(
          'python3',
          [
            '-c',
            `
import sys,tarfile
with tarfile.open(sys.argv[1]) as source:
    text=source.extractfile('index.html').read().decode()
assert text.count('data-example')==1
assert text.index('data-example')<text.index('</head>')
`,
            archive,
          ],
          { encoding: 'utf8' },
        );
        assert.equal(inspect.status, 0, inspect.stderr);
      } finally {
        await rm(directory, { recursive: true });
      }
    },
  );
