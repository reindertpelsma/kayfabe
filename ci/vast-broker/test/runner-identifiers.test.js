import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

// The runner scrubs the Worker's text a second time before it reaches the public job log, with its
// own copy of the 32-character rule (runner.py scrub). Same vectors, same falsifier as
// log-identifiers.test.js: a long snake_case test name survives, a token-shaped string does not.
const vectors = JSON.parse(
  readFileSync(new URL('./fixtures/scrub-vectors.json', import.meta.url), 'utf8'),
);
function scrub(cases) {
  const p = spawnSync(
    'python3',
    ['-I', new URL('runner_scrub_identifiers_fixture.py', import.meta.url).pathname],
    {
      input: JSON.stringify(cases),
      encoding: 'utf8',
      timeout: 30000,
      env: { PATH: process.env.PATH, KF_RUNNER_PATH: process.env.KF_RUNNER_PATH || '' },
    },
  );
  assert.equal(p.status, 0, p.stderr);
  return JSON.parse(p.stdout);
}
// scrub() appends a "[N bytes dropped by runner]" line when redaction shortened the text.
const first = (out) => out.split('\n')[0];
const identifier = (line) => line.match(/[a-z0-9_]{32,}/)[0];

for (const [name, line] of Object.entries(vectors.keep))
  test('runner scrub keeps a test-name identifier: ' + name, () => {
    const [out] = scrub([{ text: line }]);
    assert.ok(out.includes(identifier(line)), out);
    assert.doesNotMatch(out, /\[redacted\]/);
  });

for (const [name, text] of Object.entries(vectors.redact))
  test('runner scrub still redacts token-shaped text: ' + name, () => {
    const [out] = scrub([{ text: 'prefix ' + text + ' suffix' }]);
    assert.equal(first(out), '[untrusted] prefix [redacted] suffix');
  });

test('runner scrub redacts a configured secret that looks like a test name', () => {
  const secret = 'my_secret_snake_case_token_value_is_long';
  const [out] = scrub([{ text: 'value ' + secret + ' end', redactions: [secret] }]);
  assert.equal(first(out), '[untrusted] value [redacted] end');
});

test('runner scrub keeps the name and blanks a token on the same line', () => {
  const name = 'a_very_long_test_name_that_exceeds_thirty_two_characters_bad';
  const [out] = scrub([{ text: name + ' saw ' + vectors.redact.hex40 }]);
  assert.ok(out.includes(name));
  assert.ok(!out.includes(vectors.redact.hex40));
});
