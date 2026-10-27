function utf8Prefix(text, limit) {
  let bytes = new TextEncoder().encode(text);
  if (bytes.length <= limit) return text;
  bytes = bytes.slice(0, Math.max(0, limit));
  for (let i = 0; i < 4; i++) {
    try {
      return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    } catch {
      bytes = bytes.slice(0, -1);
    }
  }
  return '';
}
// A run of 32+ [A-Za-z0-9_-] characters is blanked as a possible token, EXCEPT a snake_case test name
// (2026-10-26, gate run 40263112941: libtest prints `<name> --- FAILED`, and 60-character names were
// blanked, leaving `[redacted] --- FAILED`). Kept only if: lowercase words joined by `_`, at least three,
// each at most 20 characters, at most 128 characters in all, at most one digit in five. Hex, base64url,
// mixed-case, hyphenated and digit-heavy strings still match the generic rule, and known secrets and
// token families are replaced by value or by pattern regardless. runner.py scrub and
// scripts/ci/vast_payload.py (is_test_identifier) hold the same grammar; test/fixtures/scrub-vectors.json
// pins all three.
const SNAKE_NAME = /^[a-z][a-z0-9]{0,19}(?:_[a-z0-9]{1,20}){2,}$/;
function keepsRun(value) {
  if (value.length > 128 || !SNAKE_NAME.test(value)) return false;
  return value.replace(/[^0-9]/g, '').length * 5 <= value.length;
}
export function sanitizeBoxText(bytes, cap = 8192, secrets = [], { tail = true } = {}) {
  const raw = new TextDecoder('utf-8').decode(bytes),
    encoder = new TextEncoder();
  let text = raw
    .replace(/\r\n?/g, '\n')
    .replace(/[\u2028\u2029]/g, '\n')
    .replace(/\x1b\][\s\S]*?(?:\x07|\x1b\\|$)/g, '')
    .replace(/\x1b[P^_][\s\S]*?(?:\x1b\\|$)/g, '')
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
    .replace(/\x1b[ -/]*[@-~]/g, '')
    .replace(/[\x00-\x08\x0b-\x1f\x7f-\x9f]/g, '')
    .replace(/\p{Cf}/gu, '')
    .replace(
      /\b(?:gh[pousr]_[A-Za-z0-9]{20,255}|github_pat_[A-Za-z0-9_]{20,255}|v1\.[0-9]+\.[0-9]+\.[0-9]+\.[A-Za-z0-9_-]{43})\b/g,
      '[redacted]',
    );
  text = text
    .replace(/\bBearer\s+[^\s]+/gi, 'Bearer [redacted]')
    .replace(/\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}\b/g, '[address]')
    .replace(
      /(?<![A-Za-z0-9_:-])(?:[0-9a-f]{0,4}:){2,7}[0-9a-f]{0,4}(?:%[A-Za-z0-9]{1,32})?(?![A-Za-z0-9_:-])/gi,
      '[address]',
    )
    .replace(/[A-Za-z0-9_-]+/g, (value) =>
      value.length >= 32 && !keepsRun(value) ? '[redacted]' : value,
    );
  text = text.replace(/https?:\/\/[^\s<>"']+/gi, '[url redacted]');
  for (const secret of secrets) if (secret) text = text.split(secret).join('[redacted]');
  text = text.replace(/:{2,}/g, (value) => value.split('').join(' ')).replaceAll('##[', '# #[');
  const lines = text.split('\n');
  if (lines.at(-1) === '') lines.pop();
  let retained = [],
    size = 0,
    shown = 0;
  for (const line of tail ? lines.slice(-200) : lines) {
    const safe = utf8Prefix(line, 1018);
    const rendered = '[box] ' + safe + '\n';
    const length = encoder.encode(rendered).length;
    retained.push({ rendered, length, shown: encoder.encode(safe).length + 1 });
    size += length;
    shown += encoder.encode(safe).length + 1;
    if (!tail && size > cap) {
      retained.pop();
      shown -= encoder.encode(safe).length + 1;
      break;
    }
    while (size > cap || (tail && retained.length > 200)) {
      const old = retained.shift();
      size -= old.length;
      shown -= old.shown;
    }
  }
  return {
    text: retained.map((x) => x.rendered).join(''),
    bytesDropped: Math.max(0, bytes.byteLength - shown),
  };
}
export function appendBoxText(record, bytes, secrets = []) {
  const next = sanitizeBoxText(bytes, 8192, secrets);
  record.bytesDropped = (record.bytesDropped || 0) + next.bytesDropped;
  appendWorkerText(record, next.text);
}
export function appendWorkerText(record, text) {
  const lines = ((record.log || '') + text).split('\n');
  if (lines.at(-1) === '') lines.pop();
  let size = new TextEncoder().encode(lines.join('\n') + (lines.length ? '\n' : '')).length;
  while (size > 8192 || lines.length > 200) {
    const old = lines.shift();
    const length = new TextEncoder().encode(old + '\n').length;
    size -= length;
    record.bytesDropped += Math.max(0, length - 6);
  }
  record.log = lines.length ? lines.join('\n') + '\n' : '';
}
export function sanitizeLog(bytes, cap = 8192, secrets = []) {
  return sanitizeBoxText(bytes, cap, secrets).text;
}
