import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { shouldRunTrigger } from '../trigger-policy.mjs';
const text = readFileSync(
  new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
  'utf8',
);
const sets = [...text.matchAll(/PATH_SKIP_PATTERNS: >-\n((?: {12}[^\n]*\n)+)/g)].map((match) =>
  JSON.parse(match[1]),
);
const cases = JSON.parse(readFileSync(new URL('./site-gpu-skip-cases.json', import.meta.url)));
assert.ok(sets.length > 0, 'at least one reviewed workflow policy must exist');
// FALSIFIER: both actual workflow pattern lists already skip Site and preserve every mixed case.
for (const [index, patterns] of sets.entries())
  for (const item of cases)
    test('site GPU skip policy ' + index + ' ' + item.name, async () => {
      const repo = 'reindertpelsma/kayfabe',
        sha = 'a'.repeat(40);
      const files = item.files.map((filename) => ({ filename, status: 'modified' }));
      const pr = {
        number: 7,
        state: 'open',
        changed_files: files.length + (item.uncertain ? 1 : 0),
        head: { sha, repo: { full_name: repo } },
      };
      const run = await shouldRunTrigger({
        eventName: 'pull_request_target',
        event: { number: 7, pull_request: { head: { sha } } },
        repo,
        patterns,
        fetcher: async (url) => {
          assert.ok(String(url).startsWith('https://api.github.com/repos/' + repo + '/'));
          return Response.json(String(url).endsWith('/pulls/7') ? pr : files);
        },
      });
      assert.equal(run, item.run);
    });
