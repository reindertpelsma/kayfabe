import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

// Gate diagnosability, 2026-10-26 (live gate run 40263112941): the FINAL SUMMARY's first failing line
// arrived as `[redacted] --- FAILED`. libtest's terse mode prints `<test name> --- FAILED`, and the
// generic "32 or more [A-Za-z0-9_-] characters" scrubber blanked the name (test names run to 60+
// characters). FALSIFIER: a snake_case Rust test name of 32+ characters is blanked, OR a token-shaped
// string of 32+ characters (hex, base64url, mixed case, hyphenated, digit-heavy) survives.
// KF_SANITIZE_PATH lets scripts/ci/merge_bar_mutants.py aim this file at a mutated copy.
const target = process.env.KF_SANITIZE_PATH
  ? pathToFileURL(process.env.KF_SANITIZE_PATH).href
  : new URL('../src/sanitize.js', import.meta.url).href;
const { sanitizeLog, sanitizeBoxText } = await import(target);
const vectors = JSON.parse(
  readFileSync(new URL('./fixtures/scrub-vectors.json', import.meta.url), 'utf8'),
);
const run = (text, secrets = []) =>
  sanitizeBoxText(new TextEncoder().encode(text), 8192, secrets).text;
const identifier = (line) => line.match(/[a-z0-9_]{32,}/)[0];

for (const [name, line] of Object.entries(vectors.keep))
  test('test-name line keeps its identifier: ' + name, () => {
    const out = run(line);
    assert.ok(out.includes(identifier(line)), out);
    assert.doesNotMatch(out, /\[redacted\]/);
  });

for (const [name, text] of Object.entries(vectors.redact))
  test('token-shaped text is still redacted: ' + name, () => {
    const out = run('prefix ' + text + ' suffix');
    assert.equal(out, '[box] prefix [redacted] suffix\n');
  });

test('a configured secret is redacted even when it looks like a test name', () => {
  const secret = 'my_secret_snake_case_token_value_is_long';
  const out = run('value ' + secret + ' end', [secret]);
  assert.equal(out, '[box] value [redacted] end\n');
});

test('the observed failure line is no longer blanked', () => {
  const out = sanitizeLog(
    new TextEncoder().encode(
      'registered_tokens_skip_the_exit_and_everything_else_traps_in_order --- FAILED\n',
    ),
  );
  assert.equal(
    out,
    '[box] registered_tokens_skip_the_exit_and_everything_else_traps_in_order --- FAILED\n',
  );
});

test('a test name beside a token keeps the name and blanks the token', () => {
  const out = run(
    'a_very_long_test_name_that_exceeds_thirty_two_characters_bad saw ' + vectors.redact.hex40,
  );
  assert.ok(out.includes('a_very_long_test_name_that_exceeds_thirty_two_characters_bad'));
  assert.ok(!out.includes(vectors.redact.hex40));
  assert.match(out, /\[redacted\]/);
});
