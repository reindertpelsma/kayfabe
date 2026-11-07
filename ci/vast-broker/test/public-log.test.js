import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import {
  LOG_MAX_BYTES,
  REDACTED,
  buildPublicLog,
  exactValues,
  logsEnabled,
  redactPublic,
  stageOffsets,
} from '../src/publiclog.js';
// FALSIFIER: the published log loses or changes anything that is not a secret (an address, a URL,
// a UUID, a hash, a control sequence, a long line), keeps a secret the Worker holds or a secret
// SHAPE it is meant to catch (a run token cut in two by the box's line cap included), refuses a log
// that holds a secret instead of redacting it, is cut below 16 MiB or above it without a visible
// marker, takes super-linear time on hostile input, or reports an offset that is not a byte offset
// of the text it published.
const sha = (text) => createHash('sha256').update(text).digest('hex');
const jwt = (n = 43) =>
  'eyJhbGciOiJIUzI1NiJ9.eyJydW4iOiIxMjMiLCJhdHRlbXB0IjoiMSJ9.' + 'S'.repeat(n);
const ONE = (text, exact = []) => redactPublic(text, exactValues(exact));

test('nothing but secrets changes: addresses, URLs, UUIDs, hashes, ANSI and unicode survive', () => {
  const text = [
    'host 203.0.113.7 and 2001:db8::1 mac 52:54:00:12:34:56',
    'url https://example.test/a/b?x=1&y=2#frag and file:///tmp/x',
    'GPU-3f2504e0-4f89-41d3-9a0c-0305e82c3301 serial=ABC123DEF456',
    'commit ' + 'a'.repeat(40) + ' tree ' + 'b'.repeat(64),
    '\x1b[31mred\x1b[0m tab\tbell\x07 café ☃ \u{1f600}',
    'name_with_a_very_long_snake_case_test_identifier_that_is_over_32_characters --- FAILED',
    'password is mentioned but no value: password',
  ].join('\n');
  assert.equal(ONE(text), text);
});

test('exact values: whole, repeated, as part of a larger word, and by prefix and suffix', () => {
  const token = jwt();
  const out = ONE('a ' + token + ' b ' + token + 'x', [token]);
  assert.doesNotMatch(out, /eyJ|SSSSSSSS/);
  assert.equal((out.match(/\[redacted\]/g) ?? []).length >= 2, true);
  // The box cuts a line at 4000 bytes: only a fragment of a long secret may remain.
  const secret = 'Zq9'.repeat(30);
  const cutLine = 'prefix ' + secret.slice(0, 20) + ' ... ' + secret.slice(-20);
  assert.doesNotMatch(ONE(cutLine, [secret]), /Zq9Zq9Zq9Zq9Zq9/);
  // Too-short values are not redacted by value (they would shred ordinary words).
  assert.equal(ONE('dev ci kayfabe', ['dev', 'ci', 'kayfabe']), 'dev ci kayfabe');
  assert.equal(ONE('12345678 here', ['12345678']), '[redacted] here');
  // Longest first: a value that contains another is replaced whole.
  assert.equal(
    ONE('xx-' + 'a'.repeat(10) + '-yy', ['a'.repeat(10), 'xx-' + 'a'.repeat(10)]),
    '[redacted]-yy',
  );
});

test('every secret shape class is redacted, and only the secret part', () => {
  const cases = {
    jwt: ['token=' + jwt(), (o) => assert.doesNotMatch(o, /eyJ/)],
    bearer: [
      'Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789',
      (o) => assert.equal(o, 'Authorization: Bearer [redacted]'),
    ],
    presigned: [
      'GET https://bucket.r2.test/obj?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AK%2F2026&X-Amz-Signature=' +
        'ab'.repeat(32) +
        ' done',
      (o) => assert.equal(o, 'GET [redacted] done'),
    ],
    awskey: [
      'curl "https://s3.test/o?AWSAccessKeyId=AKIAXXXXXXXXXXXXXXXX&Expires=1"',
      (o) => assert.doesNotMatch(o, /AKIA/),
    ],
    signature: [
      'u=https://x.test/p?a=1&Signature=' + 'cd'.repeat(20),
      (o) => assert.doesNotMatch(o, /cdcdcd/),
    ],
    privateKey: [
      'before\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nAAAA\n-----END OPENSSH PRIVATE KEY-----\nafter',
      (o) => assert.equal(o, 'before\n[redacted]\nafter'),
    ],
    access: [
      'CF-Access-Client-Secret: abcdef0123456789abcdef',
      (o) => assert.equal(o, 'CF-Access-Client-Secret: [redacted]'),
    ],
    accessId: [
      'CF-Access-Client-Id=abcd1234.access',
      (o) => assert.equal(o, 'CF-Access-Client-Id=[redacted]'),
    ],
    vast: [
      '{"instance_api_key": "' + 'A1'.repeat(16) + '"}',
      (o) => assert.doesNotMatch(o, /A1A1A1/),
    ],
    github: [
      'gh token ghp_' + 'a1'.repeat(15) + ' end',
      (o) => assert.equal(o, 'gh token [redacted] end'),
    ],
    githubPat: ['github_pat_' + 'Ab1_'.repeat(8), (o) => assert.equal(o, '[redacted]')],
    credentialsUrl: [
      'git clone https://user:hunter2hunter2@host.test/repo',
      (o) => assert.equal(o, 'git clone https://[redacted]@host.test/repo'),
    ],
    named: ['db_password=correcthorsebattery', (o) => assert.equal(o, 'db_password=[redacted]')],
    namedQuoted: [
      '"api_key": "0123456789abcdef"',
      (o) => assert.equal(o, '"api_key": "[redacted]"'),
    ],
    cloudflare: [
      'CLOUDFLARE_API_TOKEN=abcdefghijklmnop1234',
      (o) => assert.equal(o, 'CLOUDFLARE_API_TOKEN=[redacted]'),
    ],
  };
  for (const [name, [input, check]] of Object.entries(cases)) {
    const out = ONE(input);
    try {
      check(out);
    } catch (error) {
      error.message = name + ': ' + error.message + ' (got ' + JSON.stringify(out) + ')';
      throw error;
    }
  }
});

test('a short or valueless assignment is left alone (no shredding of ordinary text)', () => {
  const text = 'password=short\napi_key:\nsecret mentioned\nBearer short\nghp_tooshort';
  assert.equal(ONE(text), text);
});

test('hostile secret-shaped input: nothing leaks, nothing throws', () => {
  const evil = [
    'eyJ'.repeat(5000),
    ('eyJ' + 'a'.repeat(9) + '.').repeat(200),
    '-----BEGIN PRIVATE KEY-----' + 'k'.repeat(40000),
    '-----BEGIN PRIVATE KEY-----'.repeat(300),
    'Bearer ' + ' '.repeat(100000) + 'x',
    'X-Amz-Signature='.repeat(3000),
    'password' + ' '.repeat(100000) + '=',
    'https://' + 'u'.repeat(100000) + ':p@h',
    'ghp_' + 'z'.repeat(100000),
    '\u0000𐏿'.repeat(1000),
  ];
  for (const text of evil) assert.equal(typeof ONE(text, ['q'.repeat(20)]), 'string');
  // The unterminated key block is cut at its bound, not at the end of the text.
  const out = ONE('-----BEGIN PRIVATE KEY-----' + 'k'.repeat(40000) + '\nTAIL');
  assert.match(out, /^\[redacted\]/);
  assert.ok(out.length < 40000 && out.includes('k'));
});

test('redaction is linear: 8 MiB of hostile text finishes in a few seconds', () => {
  const unit =
    'eyJ' +
    'a'.repeat(5) +
    ' Bearer ' +
    'x'.repeat(10) +
    ' X-Amz-Signature= -----BEGIN PRIVATE KEY----- password=\n';
  const text = unit.repeat(Math.floor((8 * 1024 * 1024) / unit.length));
  const t0 = Date.now();
  redactPublic(text, exactValues(['s'.repeat(40)]));
  assert.ok(Date.now() - t0 < 8000, 'took ' + (Date.now() - t0) + ' ms');
  const spaces = 'password' + ' '.repeat(4 * 1024 * 1024);
  const t1 = Date.now();
  redactPublic(spaces);
  assert.ok(Date.now() - t1 < 4000);
});

test('the log is whole under the cap: byte-identical but for secrets, digest of the bytes', async () => {
  const parts = ['line 1 é\n', 'line 2 ', 'secret-value-123456 end\n', 'last'];
  const built = await buildPublicLog(parts, exactValues(['secret-value-123456']));
  assert.equal(built.text, 'line 1 é\nline 2 [redacted] end\nlast');
  assert.equal(built.bytes, new TextEncoder().encode(built.text).length);
  assert.equal(built.sha256, sha(built.text));
  assert.equal(built.cut, 0);
});

test('a value split across two posts is still caught (joined before redaction)', async () => {
  const token = jwt();
  const built = await buildPublicLog([token.slice(0, 30), token.slice(30)], []);
  assert.equal(built.text, REDACTED);
});

test('over the cap: head and tail stay, a visible marker says how much is gone', async () => {
  const line = 'x'.repeat(1023) + '\n';
  const parts = [];
  const count = Math.ceil((16 * 1024 * 1024 + 3 * 1024 * 1024) / line.length);
  for (let i = 0; i < count; i++)
    parts.push(i === 0 ? 'HEAD-MARK\n' + line : i === count - 1 ? line + 'TAIL-MARK\n' : line);
  const built = await buildPublicLog(parts, []);
  assert.ok(built.bytes <= 16 * 1024 * 1024, 'bytes ' + built.bytes);
  assert.ok(built.bytes > 16 * 1024 * 1024 - 64 * 1024, 'does not cut far below the cap');
  assert.ok(built.cut > 3 * 1024 * 1024 - 8192 && built.cut < 4 * 1024 * 1024);
  assert.match(built.text, /\n\[kayfabe-log\] CUT: [0-9]+ bytes omitted here \(16 MiB cap\)\n/);
  assert.ok(built.text.startsWith('HEAD-MARK'));
  assert.ok(built.text.endsWith('TAIL-MARK\n'));
  assert.equal(built.sha256, sha(built.text));
  // Exactly at the cap is not cut.
  const exact = await buildPublicLog(['y'.repeat(1024)], [], { max: 1024 });
  assert.equal(exact.cut, 0);
  const over = await buildPublicLog(['y'.repeat(10 * 1024 * 1024)], [], { max: 8 * 1024 * 1024 });
  assert.ok(over.cut > 0 && over.bytes <= 8 * 1024 * 1024);
});

test('a multi-byte character is never cut in half at the cap', async () => {
  for (const max of [2 * 1024 * 1024 + 1, 2 * 1024 * 1024 + 2]) {
    const built = await buildPublicLog(['\u2603'.repeat(1_000_000)], [], { max });
    assert.doesNotMatch(built.text, /\ufffd/, 'max ' + max);
    assert.ok(built.bytes <= max);
  }
});

test('the cap is 16 MiB', () => {
  assert.equal(LOG_MAX_BYTES, 16 * 1024 * 1024);
});

test('a file name that starts like a token is not redacted (jwt needs three token-shaped parts)', () => {
  assert.equal(
    ONE('eyJabcdefghij.txt and eyJabcdefghij.kkkkkk.short'),
    'eyJabcdefghij.txt and eyJabcdefghij.kkkkkk.short',
  );
});

test('private key block: the body and the END line go with the BEGIN line, the text around stays', () => {
  const out = ONE(
    'a\n-----BEGIN RSA PRIVATE KEY-----\nAAAA\nBBBB\n-----END RSA PRIVATE KEY-----\nb\n-----BEGIN PRIVATE KEY-----\nCCCC\n-----END PRIVATE KEY-----\nc',
  );
  assert.equal(out, 'a\n[redacted]\nb\n[redacted]\nc');
});

test('a Cloudflare Access id is redacted by its own class (no named-assignment rule covers it)', () => {
  assert.equal(ONE('CF-Access-Client-Id: abcdef0123.access'), 'CF-Access-Client-Id: [redacted]');
});

test('a named assignment needs eight characters of value', () => {
  assert.equal(ONE('api_key=1234567 password=12345678'), 'api_key=1234567 password=[redacted]');
});

test('a presigned url goes whole, the words around it stay', () => {
  const url =
    'https://b.test/o?X-Amz-Algorithm=A&X-Amz-Credential=AK%2Fx&X-Amz-Signature=' + 'ab'.repeat(32);
  assert.equal(ONE('before ' + url + ' after'), 'before [redacted] after');
  assert.equal(ONE('"' + url + '",'), '"[redacted]",');
});

test('bytes the ledger could not store are named in the log', async () => {
  const built = await buildPublicLog(['text\n'], [], { omitted: 1234 });
  assert.match(built.text, /\[kayfabe-log\] 1234 further bytes were not stored/);
});

test('stage offsets are byte offsets into the published text (multi-byte text before them)', () => {
  const text =
    'café ☃\nKF_PAYLOAD_STAGE_START name=cuda_host\nbody \u{1f600}\nKF_PAYLOAD_STAGE_END name=cuda_host rc=0 duration_ms=5\n' +
    '[box] KF_PAYLOAD_STAGE_START name=merge_bar\nmore\nKF_PAYLOAD_STAGE_END name=merge_bar rc=0 duration_ms=9\ntail';
  const marks = stageOffsets(text);
  const bytes = Buffer.from(text);
  const host = marks.get('cuda_host');
  assert.equal(
    bytes.subarray(host.start_byte).toString().startsWith('KF_PAYLOAD_STAGE_START name=cuda_host'),
    true,
  );
  assert.equal(bytes.subarray(0, host.end_byte).toString().endsWith('duration_ms=5\n'), true);
  const merge = marks.get('merge_bar');
  assert.ok(
    bytes
      .subarray(merge.start_byte)
      .toString()
      .startsWith('[box] KF_PAYLOAD_STAGE_START name=merge_bar'),
  );
  assert.ok(merge.start_byte >= host.end_byte);
  assert.equal(marks.size, 2);
  // A stage with a START and no END (the box was cut off) has no range; never a guessed one.
  assert.equal(stageOffsets('KF_PAYLOAD_STAGE_START name=lonely\n').size, 0);
});

test('logsEnabled needs a writable bucket binding', () => {
  assert.equal(logsEnabled({}), false);
  assert.equal(logsEnabled({ LOGS_BUCKET: null }), false);
  assert.equal(logsEnabled({ LOGS_BUCKET: {} }), false);
  assert.equal(logsEnabled({ LOGS_BUCKET: { put() {} } }), true);
});
