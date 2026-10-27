import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { sha, nonce, env } from './helpers.js';
const sanitize = async () => import('../src/sanitize.js');
for (const v of ['$(id)', '`id`', "'", '\n', 'a'.repeat(39), 'A'.repeat(40)])
  test('onstart rejects SHA injection ' + JSON.stringify(v), async () => {
    const { onstart } = await import('../src/onstart.js');
    assert.throws(() => onstart(v, env.REPOSITORY));
  });
test('onstart embeds the constrained commit', async () => {
  const { onstart } = await import('../src/onstart.js');
  const s = onstart(sha, env.REPOSITORY);
  assert.match(s, /git checkout --detach/);
  assert.match(s, /rev-parse HEAD/);
  assert.match(s, /vast_onstart.sh/);
  assert.doesNotMatch(s, /authorized_keys|ssh_key|API_KEY|KF_RESULT/);
});
for (const prefix of [
  '::error::hostile',
  '::set-output name=x::bad',
  '##[error]hostile',
  '::add-mask::secret',
  '::stop-commands::x',
])
  test('sanitizer prefixes every command-like line ' + prefix, async () => {
    const { sanitizeLog } = await sanitize();
    assert.equal(
      sanitizeLog(new TextEncoder().encode(prefix + '\n')),
      '[box] ' + prefix.replaceAll('::', ': :').replaceAll('##[', '# #[') + '\n',
    );
  });
test('sanitizer normalizes CR and removes ANSI and unsafe controls', async () => {
  const { sanitizeLog } = await sanitize();
  assert.equal(
    sanitizeLog(new TextEncoder().encode('\x1b[31mred\x1b[0m\x00\r\x07\t\n')),
    '[box] red\n[box] \t\n',
  );
});
test('sanitizer forces valid UTF8 with prefixed replacement', async () => {
  const { sanitizeLog } = await sanitize();
  assert.equal(sanitizeLog(Uint8Array.of(255, 254)), '[box] ��\n');
});
test('sanitizer redacts configured secret values', async () => {
  const { sanitizeLog } = await sanitize();
  assert.equal(
    sanitizeLog(new TextEncoder().encode('binding-secret'), 8192, ['binding-secret']),
    '[box] [redacted]\n',
  );
});
for (const control of ['\u200b', '\u202e'])
  test(
    'Unicode format control is removed before command prefix ' + control.codePointAt(0),
    async () => {
      const { sanitizeLog } = await sanitize();
      assert.equal(
        sanitizeLog(new TextEncoder().encode(control + '::error::bad')),
        '[box] : :error: :bad\n',
      );
    },
  );
for (const command of ['::error::', '##[error]'])
  test('tail never exposes bare command ' + command, async () => {
    const { sanitizeLog } = await sanitize();
    const log = sanitizeLog(
      new TextEncoder().encode(('x'.repeat(900) + '\n').repeat(20) + command + 'tail'),
      1000,
    );
    assert.doesNotMatch(log, /^(::|##\[)/m);
    assert.ok(Buffer.byteLength(log) <= 1000);
    assert.ok(
      log
        .trimEnd()
        .split('\n')
        .every((x) => x.startsWith('[box] ')),
    );
  });
for (const secret of ['ghs' + '_' + 'A'.repeat(36), 'github' + '_pat_' + 'B'.repeat(40)])
  test('token-shaped text is redacted ' + secret.slice(0, 4), async () => {
    const { sanitizeLog } = await sanitize();
    assert.equal(
      sanitizeLog(new TextEncoder().encode('prefix ' + secret + ' suffix')),
      '[box] prefix [redacted] suffix\n',
    );
  });
for (const text of ['z'.repeat(10 * 1024 * 1024), 'line\n'.repeat(1000)])
  test('sanitizer bounds huge lines or line counts ' + text.length, async () => {
    const { sanitizeBoxText } = await sanitize();
    const r = sanitizeBoxText(new TextEncoder().encode(text));
    assert.ok(Buffer.byteLength(r.text) <= 8192);
    assert.ok(r.text.trimEnd().split('\n').length <= 200);
    assert.ok(
      r.text
        .trimEnd()
        .split('\n')
        .every((x) => x.startsWith('[box] ') && Buffer.byteLength(x) <= 1024),
    );
    assert.ok(r.bytesDropped > 0);
  });
for (const payload of [
  'prefix ##[error]legacy',
  '::stop-commands::guessed\n::guessed::',
  'text ::add-mask::secret',
])
  test('no workflow command delimiter survives anywhere ' + payload.slice(0, 20), async () => {
    const { sanitizeLog } = await sanitize();
    assert.doesNotMatch(sanitizeLog(new TextEncoder().encode(payload)), /::|##\[/);
  });

for (const gpu of ['RTX 3090', 'NVIDIA Quadro RTX 6000'])
  test('trusted BASE runner publishes rental identity once ' + gpu, () => {
    const run = spawnSync(
      'python3',
      ['-I', new URL('./round_runner_fixture.py', import.meta.url).pathname, 'BUSY', gpu],
      { encoding: 'utf8' },
    );
    assert.equal(run.status, 0, run.stderr);
    const { stdout, summary } = JSON.parse(run.stdout);
    assert.equal((stdout.match(/RENTAL machine=/g) ?? []).length, 1);
    assert.match(stdout, /RENTAL machine=100001 host=3001 instance=900 gpu=/);
    assert.ok(stdout.includes('price=0.200000'));
    for (const value of ['900', '100001', '3001', gpu, '570.124.06', 'GREEN', '0.200000'])
      assert.ok(summary.includes(value), value);
    assert.ok(summary.includes('Texas, &lt;US&gt;'));
    assert.doesNotMatch(summary, /<US>/);
  });
