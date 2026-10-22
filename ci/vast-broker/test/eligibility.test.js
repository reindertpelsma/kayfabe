import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, sha, nonce, repo, clock } from './helpers.js';
const json = async (response) => response.json();
const dispatch = { event_name: 'workflow_dispatch', sha, ref: 'refs/heads/owner-pr' };
for (const base of ['integration/candidate', 'another-pr'])
  test('automatic non-master base is NOT_ELIGIBLE ' + base, async () => {
    const h = await setup();
    h.cloud.prs[0].base.ref = base;
    assert.equal(
      (
        await json(
          await h.call('/rent', {
            claims: { base_ref: 'master', allowed_base_branches: ['master'] },
          }),
        )
      ).state,
      'NOT_ELIGIBLE',
    );
    assert.equal(h.cloud.creates, 0);
  });
test('base allowlist is explicit Worker config', async () => {
  const h = await setup({ ALLOWED_BASE_BRANCHES: '["stable"]' });
  h.cloud.prs[0].base.ref = 'stable';
  assert.equal((await h.call()).status, 200);
  assert.equal(h.cloud.creates, 1);
});
test('missing API base never trusts an OIDC base claim', async () => {
  const h = await setup();
  delete h.cloud.prs[0].base.ref;
  assert.equal(
    (await json(await h.call('/rent', { claims: { base_ref: 'master' } }))).state,
    'NOT_ELIGIBLE',
  );
});
test('owner dispatch permits a non-master-base PR', async () => {
  const h = await setup();
  h.cloud.run.event = 'workflow_dispatch';
  h.cloud.prs[0].base.ref = 'integration/test';
  assert.equal((await h.call('/rent', { claims: dispatch })).status, 200);
});
for (const branch of ['owner-pr', 'integration/test'])
  test('owner dispatch permits branch without a PR ' + branch, async () => {
    const h = await setup();
    h.cloud.run.event = 'workflow_dispatch';
    h.cloud.prs = [];
    h.cloud.run.head_branch = branch;
    const r = await h.call('/rent', { claims: { ...dispatch, ref: 'refs/heads/' + branch } });
    assert.equal(r.status, 200);
    assert.equal((await r.json()).pr, null);
    await h.report({ seq: 1, kind: 'result', result: { nonce, verdict: 'PASS' } });
    assert.equal(
      (
        await json(
          await h.call('/status/123-1', { claims: { ...dispatch, ref: 'refs/heads/' + branch } }),
        )
      ).verdict,
      'PASS',
    );
  });
for (const files of [
  ['README.md'],
  [
    'docs/design.md',
    'traces/evidence.txt',
    'archive/old.rs',
    'LICENSE-MIT',
    '.github/ISSUE_TEMPLATE/bug.yml',
  ],
  ['docs/guide.md', 'nested/notes.md'],
])
  test('complete exclusion list is NOT_NEEDED ' + files.join(','), async () => {
    const h = await setup();
    h.cloud.files = files.map((filename) => ({ filename, status: 'modified' }));
    h.cloud.prs[0].changed_files = files.length;
    const r = await h.call();
    assert.equal(r.status, 200);
    const data = await r.json();
    assert.equal(data.state, 'NOT_NEEDED');
    assert.equal(data.verdict, 'PASS');
    assert.equal(data.sha, sha);
    assert.equal(data.pr, 7);
    assert.equal(h.cloud.creates, 0);
    assert.equal(h.storage.data.size, 0);
  });
for (const filename of [
  'src/new.rs',
  'Cargo.lock',
  '.github/workflows/ci.yml',
  'scripts/new.sh',
  'qemu/new.c',
  'cuda/new.cu',
  'ci/vast-broker/README.md',
  'ci/vast-broker/test/example.md',
])
  test('unlisted path runs despite adjacent docs ' + filename, async () => {
    const h = await setup();
    h.cloud.files = [
      { filename: 'docs/ok.md', status: 'modified' },
      { filename, status: 'added' },
    ];
    h.cloud.prs[0].changed_files = 2;
    assert.equal((await h.call()).status, 200);
    assert.equal(h.cloud.creates, 1);
  });
for (const [filename, previous_filename] of [
  ['src/code.rs', 'docs/old.md'],
  ['docs/new.md', 'src/code.rs'],
  ['docs/new.md', 'docs/../src/code.md'],
])
  test('rename checks both paths ' + filename + ' from ' + previous_filename, async () => {
    const h = await setup();
    h.cloud.files = [{ filename, previous_filename, status: 'renamed' }];
    await h.call();
    assert.equal(h.cloud.creates, 1);
  });
for (const filename of [
  'docs/../src/x.md',
  'docs//guide.md',
  'docs/./guide.md',
  '/docs/guide.md',
  'docs\\guide.md',
  'DOCS/config.rs',
  'guide.MD',
  'docs/@link.md',
  'docs/\u202eguide.md',
  'docs/foo\nbar.md',
])
  test('path trickery runs ' + JSON.stringify(filename), async () => {
    const h = await setup();
    h.cloud.files = [{ filename, status: 'modified' }];
    await h.call();
    assert.equal(h.cloud.creates, 1);
  });
for (const kind of [
  'empty',
  'missing-count',
  'truncated',
  'cap',
  'malformed',
  'missing-name',
  'unknown-status',
  'rename-no-old',
])
  test('incomplete or unknown file metadata runs ' + kind, async () => {
    const h = await setup();
    h.cloud.files = [{ filename: 'docs/guide.md', status: 'modified' }];
    if (kind === 'empty') h.cloud.files = [];
    if (kind === 'missing-count') delete h.cloud.prs[0].changed_files;
    if (kind === 'truncated') h.cloud.prs[0].changed_files = 2;
    if (kind === 'cap') h.cloud.prs[0].changed_files = 3000;
    if (kind === 'malformed') h.cloud.files = { files: h.cloud.files };
    if (kind === 'missing-name') h.cloud.files = [{ status: 'modified' }];
    if (kind === 'unknown-status') h.cloud.files[0].status = 'unknown';
    if (kind === 'rename-no-old') h.cloud.files[0].status = 'renamed';
    assert.equal((await h.call()).status, 200);
    assert.equal(h.cloud.creates, 1);
  });
test('paginated all-docs list is complete before skipping', async () => {
  const h = await setup();
  h.cloud.prs[0].changed_files = 101;
  h.cloud.filePages = [
    Array.from({ length: 100 }, (_, i) => ({ filename: `docs/${i}.md`, status: 'modified' })),
    [{ filename: 'docs/last.md', status: 'modified' }],
  ];
  assert.equal((await json(await h.call())).state, 'NOT_NEEDED');
  assert.equal(h.cloud.calls.filter((x) => x.url.includes('/files?')).length, 2);
});
for (const code of [403, 500])
  test('file API failure is unavailable not a skip ' + code, async () => {
    const h = await setup();
    h.cloud.filesError = code;
    assert.equal((await json(await h.call())).state, 'UNAVAILABLE');
    assert.equal(h.cloud.creates, 0);
  });
test('head change while reading files cannot skip or rent', async () => {
  const h = await setup();
  h.cloud.files = [{ filename: 'docs/guide.md', status: 'modified' }];
  const original = h.cloud.fetch;
  h.cloud.fetch = async (u, i) => {
    const r = await original(u, i);
    if (String(u).includes('/files?')) h.cloud.prs[0].head.sha = 'e'.repeat(40);
    return r;
  };
  h.restart();
  assert.notEqual((await h.call()).status, 200);
  assert.equal(h.cloud.creates, 0);
});
test('owner dispatch explicitly runs docs-only branch', async () => {
  const h = await setup();
  h.cloud.run.event = 'workflow_dispatch';
  h.cloud.files = [{ filename: 'README.md', status: 'modified' }];
  assert.equal((await h.call('/rent', { claims: dispatch })).status, 200);
  assert.equal(h.cloud.creates, 1);
});
test('configured exclusions cannot skip broker code', async () => {
  const h = await setup({ PATH_SKIP_PATTERNS: '["**"]' });
  h.cloud.files = [{ filename: 'ci/vast-broker/src/index.js', status: 'modified' }];
  await h.call();
  assert.equal(h.cloud.creates, 1);
});
for (const path of ['/rent', '/status/123-1'])
  test('request GitHub bearer used only for read-only GitHub calls ' + path, async () => {
    const h = await setup();
    if (path !== '/rent') await h.call();
    h.cloud.calls = [];
    await h.call(path, { ghToken: 'per-request-bearer' });
    const gh = h.cloud.calls.filter((x) => x.url.startsWith('https://api.github.com/'));
    assert.ok(gh.length);
    assert.ok(
      gh.every(
        (x) => x.method === 'GET' && x.headers.get('Authorization') === 'Bearer per-request-bearer',
      ),
    );
    assert.ok(
      h.cloud.calls
        .filter((x) => !x.url.startsWith('https://api.github.com/'))
        .every(
          (x) =>
            x.headers.get('Authorization') !== 'Bearer per-request-bearer' &&
            !x.headers.has('X-GH-Token'),
        ),
    );
    assert.doesNotMatch(JSON.stringify([...h.storage.data]), /per-request-bearer/);
  });
test('new request replaces prior bearer and cron retains none', async () => {
  const h = await setup();
  await h.call('/rent', { ghToken: 'first-request-token' });
  h.cloud.calls = [];
  await h.call('/status/123-1', { ghToken: 'second-request-token' });
  assert.ok(
    h.cloud.calls
      .filter((x) => x.url.includes('api.github.com'))
      .every((x) => x.headers.get('Authorization') === 'Bearer second-request-token'),
  );
  h.cloud.calls = [];
  await h.service.sweep();
  assert.ok(
    h.cloud.calls
      .filter((x) => x.url.includes('api.github.com'))
      .every((x) => !x.headers.has('Authorization')),
  );
  assert.doesNotMatch(
    JSON.stringify([...h.storage.data]),
    /first-request-token|second-request-token/,
  );
});
test('missing bearer can use public facts', async () => {
  const h = await setup();
  assert.equal((await h.call('/rent', { ghToken: null })).status, 200);
  assert.ok(
    h.cloud.calls
      .filter((x) => x.url.includes('api.github.com'))
      .every((x) => !x.headers.has('Authorization')),
  );
});
for (const phase of ['upstream error', 'box output'])
  test('request token absent from responses and ledger ' + phase, async () => {
    const h = await setup();
    const token = 'ghs' + '_' + 'Q'.repeat(36);
    let text;
    if (phase === 'upstream error') {
      h.cloud.githubError = 401;
      text = await (await h.call('/rent', { ghToken: token })).text();
    } else {
      await h.call('/rent', { ghToken: token });
      await h.report({ seq: 1, kind: 'log', text: token });
      text = await (await h.call('/status/123-1', { ghToken: token })).text();
    }
    assert.doesNotMatch(text, new RegExp(token));
    assert.doesNotMatch(JSON.stringify([...h.storage.data]), new RegExp(token));
  });
test('GitHub bearer is never an identity', async () => {
  const h = await setup();
  h.cloud.run.actor.id = 43;
  h.cloud.run.triggering_actor.id = 43;
  assert.equal(
    (await json(await h.call('/rent', { claims: { actor_id: '43' }, ghToken: 'owner-token-text' })))
      .state,
    'NEEDS_APPROVAL',
  );
  assert.equal(h.cloud.creates, 0);
});
test('static GitHub facts have a short token-free cache', async () => {
  const { Github } = await import('../src/github.js');
  const { configuration } = await import('../src/policy.js');
  const h = await setup();
  const cache = new Map();
  const g = new Github(h.cloud.fetch, configuration(h.bindings), 'cache-bearer', {
    cache,
    now: () => h.time.value,
  });
  await g.get('commits/' + sha);
  await g.get('commits/' + sha);
  assert.equal(h.cloud.calls.length, 1);
  assert.doesNotMatch(JSON.stringify([...cache]), /cache-bearer/);
  h.time.value += 6000;
  await g.get('commits/' + sha);
  assert.equal(h.cloud.calls.length, 2);
});
test('invalid read token plus unavailable public API never allows', async () => {
  const h = await setup();
  h.cloud.githubError = 401;
  const r = await h.call('/rent', { ghToken: 'expired-token' });
  assert.equal((await r.json()).state, 'UNAVAILABLE');
  assert.equal(h.cloud.creates, 0);
});
for (const status of [401, 403])
  test('unusable read bearer falls back to authenticated public facts ' + status, async () => {
    const h = await setup();
    const original = h.cloud.fetch;
    h.cloud.fetch = async (u, i) =>
      String(u).startsWith('https://api.github.com/') &&
      new Headers(i?.headers).has('Authorization')
        ? new Response('TOKEN-REJECTED', { status })
        : original(u, i);
    h.restart();
    assert.equal((await h.call('/rent', { ghToken: 'expired-job-bearer' })).status, 200);
    assert.equal(h.cloud.creates, 1);
    assert.ok(
      h.cloud.calls
        .filter((x) => x.url.includes('api.github.com'))
        .every((x) => !x.headers.has('Authorization')),
    );
  });
test('runner token never appears in Worker console events', async () => {
  const h = await setup();
  const logs = [];
  const original = console.log;
  console.log = (...values) => logs.push(values.join(' '));
  try {
    await h.call('/rent', { ghToken: 'ephemeral-console-bearer' });
    await h.report({ seq: 1, kind: 'log', text: 'ephemeral-console-bearer' });
    await h.call('/status/123-1', { ghToken: 'ephemeral-console-bearer' });
  } finally {
    console.log = original;
  }
  assert.doesNotMatch(logs.join('\n'), /ephemeral-console-bearer/);
  assert.match(logs.join('\n'), /rented/);
});
test('static response cache works across request clients without retaining bearers', async () => {
  const { Github } = await import('../src/github.js');
  const { configuration } = await import('../src/policy.js');
  const h = await setup();
  const cache = new Map();
  for (const token of ['first-cache-token', 'second-cache-token']) {
    const g = new Github(h.cloud.fetch, configuration(h.bindings), token, {
      cache,
      now: () => clock,
    });
    const result = await g.get('commits/' + sha);
    assert.equal(result.sha, sha);
    result.sha = 'mutated-client-value';
  }
  assert.equal(h.cloud.calls.length, 1);
  assert.doesNotMatch(JSON.stringify([...cache]), /cache-token|mutated-client-value/);
});
