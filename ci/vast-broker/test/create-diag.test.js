import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
// MEASURED 2026-10-25 live: POST /rent answered 503 UNAVAILABLE repeatedly, nothing was created
// on Vast, and the Worker gave no reason. The Worker now logs ONE static structured line when
// Vast's create answer is not a success: status, a fixed class and a redacted, bounded excerpt
// of the provider's own error text. The runner-facing response is unchanged.
// FALSIFIER: no line is logged, the line carries a header/key/body sent/onstart/env/whole
// response, key-like provider text survives, or the runner response changes or reflects text.
const capture = async (fn) => {
  const logged = [],
    original = console.log;
  console.log = (...args) => logged.push(args.join(' '));
  try {
    await fn();
  } finally {
    console.log = original;
  }
  return logged.map((l) => JSON.parse(l)).filter((l) => l.event === 'create_failed');
};
async function rentWith(answer) {
  const h = await setup();
  const base = h.cloud.fetch;
  let sent;
  h.cloud.fetch = async (url, init) => {
    if (String(url).includes('/asks/')) {
      sent = init;
      return answer();
    }
    return base(url, init);
  };
  h.restart();
  let response, text;
  const lines = await capture(async () => {
    response = await h.call();
    text = await response.text();
  });
  return { lines, response, text, sent, h };
}
const json = (status, body) => () =>
  new Response(typeof body === 'string' ? body : JSON.stringify(body), { status });
const cases = [
  [
    '401 http refusal',
    json(401, { success: false, error: 'Unauthorized key' }),
    401,
    'http_refusal',
    'Unauthorized key',
  ],
  [
    '403 lacks right',
    json(403, { error: 'api key lacks permission instance_write' }),
    403,
    'http_refusal',
    'api key lacks permission instance_write',
  ],
  [
    '429 rate',
    json(429, { msg: 'Rate limited, slow down' }),
    429,
    'http_refusal',
    'Rate limited, slow down',
  ],
  ['502 gateway', json(502, 'Bad gateway html'), 502, 'unparsable', ''],
  ['503 json', json(503, { error: 'try later' }), 503, 'http_refusal', 'try later'],
  [
    '200 success false',
    json(200, { success: false, error: 'no_such_ask', msg: 'Ask unavailable' }),
    200,
    'success_false',
    'no_such_ask',
  ],
  [
    '400 success false msg',
    json(400, { success: false, msg: 'Insufficient credit' }),
    400,
    'success_false',
    'Insufficient credit',
  ],
  ['404 success false', json(404, { success: false, error: 'gone' }), 404, 'success_false', 'gone'],
  [
    '410 success false',
    json(410, { success: false, error: 'ask expired' }),
    410,
    'success_false',
    'ask expired',
  ],
  ['200 unparsable', json(200, '<html>nope'), 200, 'unparsable', ''],
  ['200 no contract', json(200, { success: true }), 200, 'no_contract', ''],
  ['200 bad contract', json(200, { success: true, new_contract: -4 }), 200, 'no_contract', ''],
];
for (const [name, answer, status, klass, error] of cases)
  test('create failure logs one static line: ' + name, async () => {
    const { lines, response, text } = await rentWith(answer);
    assert.equal(lines.length, 1);
    assert.deepEqual(lines[0], {
      event: 'create_failed',
      status,
      class: klass,
      provider_error: error,
    });
    // The runner sees exactly what it saw before: a bare 503 and no provider text.
    assert.equal(response.status, 503);
    assert.ok(!text.includes(error || 'zzz-none'));
    assert.ok(!text.includes('create_failed'));
  });
for (const [name, thrown] of [
  ['network error', () => Promise.reject(new TypeError('connect ECONNRESET secret-host'))],
  ['timeout', () => Promise.reject(new DOMException('timed out', 'TimeoutError'))],
])
  test('create failure logs one static line: ' + name, async () => {
    const { lines, response, text } = await rentWith(thrown);
    assert.equal(lines.length, 1);
    assert.deepEqual(lines[0], {
      event: 'create_failed',
      status: null,
      class: 'transport',
      provider_error: '',
    });
    assert.equal(response.status, 503);
    assert.ok(!text.includes('ECONNRESET'));
  });
test('create failure log redacts key-like provider text', async () => {
  for (const evil of [
    'Bearer abc123',
    'bad key abcdefghijklmnopqrstuvwxyz0123456789',
    'token a1b2c3d4-e5f6-a1b2-c3d4-e5f6a1b2c3d4',
    'xx_ABCDEFGHIJKLMNOPQRSTUVWX yy',
    'authorization: BEARER x',
  ]) {
    const { lines } = await rentWith(json(403, { error: evil }));
    assert.equal(lines.length, 1);
    assert.equal(lines[0].provider_error, '', evil);
  }
});
test('create failure log restricts characters and bounds length', async () => {
  const { lines } = await rentWith(
    json(400, { success: false, error: '<b>"Bad"</b> {ask}\n[x] a=b ' + 'word '.repeat(60) }),
  );
  assert.equal(lines.length, 1);
  assert.ok(lines[0].provider_error.length <= 120);
  assert.match(lines[0].provider_error, /^[A-Za-z0-9 _.,:;'()/=-]*$/);
  assert.ok(lines[0].provider_error.includes('Bad'));
  // A non-string provider error field never reaches the log.
  const odd = await rentWith(json(400, { success: false, error: { token: 'x' } }));
  assert.equal(odd.lines[0].provider_error, '');
});
test('create failure log never carries the key, headers, body sent, onstart, env or response', async () => {
  const { lines, sent, h } = await rentWith(
    json(403, { error: 'denied', detail: 'extra-field-value', onstart: 'leak-onstart' }),
  );
  const line = JSON.stringify(lines);
  assert.deepEqual(Object.keys(lines[0]).sort(), ['class', 'event', 'provider_error', 'status']);
  const key = String(h.bindings.VAST_API_KEY ?? '');
  if (key) assert.ok(!line.includes(key));
  assert.ok(!/Bearer|Authorization|onstart|KF_|extra-field-value|leak-onstart/i.test(line));
  assert.ok(!line.includes(JSON.parse(sent.body).label));
});
test('a successful create logs no create_failed line', async () => {
  const h = await setup();
  const lines = await capture(async () => {
    assert.equal((await h.call()).status, 200);
  });
  assert.deepEqual(lines, []);
});
