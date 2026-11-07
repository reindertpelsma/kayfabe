// The full public log of a run (2026-11-07; contract /root/night/REPORT_CONTRACT.md, DEPLOY.md
// "Public logs"). When a run closes, for any reason, the Worker writes the text the box streamed
// to the PUBLIC R2 bucket kayfabe-logs-public (https://logs.kayfabe.dev) as <run id>/log.txt.
//
// Policy (owner): the log is whole. No allow-list of characters, no display budget, no filter for
// any other motive: the ONLY change is secret redaction, each secret replaced by `[redacted]` (a
// log is never refused for holding one), plus ONE hard cap of 16 MiB that is cut with a visible
// marker line, never silently. GPU UUIDs and board facts are not stripped (rented boxes' UUIDs are
// public knowledge, owner ruling).
//
// What it redacts:
//  1. EXACT values this Worker knows (run token, nonces, the secret path, the Access pair, every
//     Worker secret, the Vast, GitHub and Cloudflare tokens it holds), and a 16-character prefix and
//     suffix of each long one (the box cuts a line at 4000 bytes and a cut can leave only a fragment).
//  2. SHAPES, for secrets the Worker does not hold: JWTs, `Bearer <token>`, any token carrying
//     presigned-URL parameters (X-Amz-*, AWSAccessKeyId, Signature=), private-key blocks,
//     Cloudflare Access headers, Vast API keys, GitHub tokens, URLs with credentials, and a named
//     assignment (`password=`, `api_key:`, `client_secret=`, ...) whose value is 8 or more characters.
//     The classes are those of the evidence scanner (scripts/ci/vast_evidence.py PATTERNS), applied
//     as replacement instead of refusal.
// Every scan is linear in the input: a regex is anchored on a literal and has a bounded
// quantifier, or the scan is a loop with indexOf (test/public-log.test.js feeds hostile input).
import { sha256Hex } from './tokens.js';

export const LOG_MAX_BYTES = 16 * 1024 * 1024;
export const REDACTED = '[redacted]';
export const EXACT_MIN = 8;
export const FRAGMENT = 16;
export const KEY_BLOCK_MAX = 16384;
// How long one R2 call may take (a variable only so that tests can shorten it).
export const publishTimeouts = { r2: 20000 };

// Who wrote a piece of the log. The Worker's own lines are stored between two private-use
// characters; the box's text is stored with those characters removed, so a box can neither write
// them nor pass off its own line as the Worker's. At publication every line that does not start
// with the Worker's mark and that starts like one of the Worker's own line kinds (`[Worker] `, or
// the `[kayfabe-log] ` cap markers) gets the prefix `[box] `. That is the one change to box text
// beyond redaction, and it exists only to keep the Worker's voice unforgeable (review of #282, F7).
export const WORKER_OPEN = '\ue000';
export const WORKER_CLOSE = '\ue001';
export const workerPiece = (text) => WORKER_OPEN + text + WORKER_CLOSE;
export const boxPiece = (text) => text.replace(/[\ue000\ue001]/g, '');
export function attribute(joined) {
  const text = joined
    .replace(/([^\n\ue001])\ue000/g, '$1\n\ue000') // a Worker line always starts a line
    .replace(/([^\n])\ue001/g, '$1\n') // and ends one
    .replace(/\ue001/g, '')
    .replace(/^(?!\ue000)(?=\[Worker\]|\[kayfabe-log\])/gm, '[box] ');
  return text.replace(/\ue000/g, '');
}

export const logsEnabled = (env) =>
  env?.LOGS_BUCKET !== undefined &&
  env.LOGS_BUCKET !== null &&
  typeof env.LOGS_BUCKET.put === 'function';

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8');

// ---- exact values -----------------------------------------------------------------------------

// The strings to replace by value: every value of at least EXACT_MIN characters, and the
// prefix and suffix of a long one. Longest first, so that a value holding another is replaced whole.
export function exactValues(values) {
  const out = new Set();
  for (const v of values) {
    if (typeof v !== 'string' || v.length < EXACT_MIN) continue;
    out.add(v);
    if (v.length >= 2 * FRAGMENT) {
      out.add(v.slice(0, FRAGMENT));
      out.add(v.slice(-FRAGMENT));
    }
  }
  return [...out].sort((a, b) => b.length - a.length);
}

// ---- shapes -----------------------------------------------------------------------------------

// A run of token characters starting at "eyJ" is a JWT when it splits on dots into a header part
// of 9+ characters, a part of 6+ and a part of 20+ (the same test as the evidence scanner). One
// regex match per run (a match consumes its whole run), then a split: linear.
function redactJwt(text) {
  return text.replace(/eyJ[A-Za-z0-9_.-]*/g, (run) => {
    const parts = run.split('.');
    for (let i = 0; i + 2 < parts.length; i++) {
      const at = parts[i].indexOf('eyJ');
      if (
        at >= 0 &&
        parts[i].length - at >= 9 &&
        parts[i + 1].length >= 6 &&
        parts[i + 2].length >= 20
      )
        return REDACTED;
    }
    return run;
  });
}

// The whole whitespace-delimited token around a marker is replaced (a presigned URL carries its
// secret in several parameters), at most `reach` characters each side. A loop over the marker's
// matches with a moving cursor: linear, no backtracking.
const TOKEN_BREAK = /[\s"'<>`]/;
function redactAround(text, marker, reach = 8192) {
  let out = '';
  let cursor = 0;
  const re = new RegExp(
    marker.source,
    marker.flags.includes('g') ? marker.flags : marker.flags + 'g',
  );
  let m;
  while ((m = re.exec(text)) !== null) {
    if (m.index < cursor) {
      re.lastIndex = cursor;
      continue;
    }
    let start = m.index;
    const floor = Math.max(cursor, m.index - reach);
    while (start > floor && !TOKEN_BREAK.test(text[start - 1])) start--;
    let end = m.index + m[0].length;
    const ceil = Math.min(text.length, end + reach);
    while (end < ceil && !TOKEN_BREAK.test(text[end])) end++;
    out += text.slice(cursor, start) + REDACTED;
    cursor = end;
    re.lastIndex = end;
    if (m[0].length === 0) re.lastIndex++;
  }
  return cursor === 0 ? text : out + text.slice(cursor);
}

// A private-key block: from the BEGIN line to the END line, or KEY_BLOCK_MAX characters if no END
// follows (the log may be cut inside the block). indexOf loop: linear.
function redactKeyBlocks(text) {
  const begin = /-----BEGIN [A-Z0-9 ]{0,30}PRIVATE KEY-----/g;
  let out = '';
  let cursor = 0;
  let m;
  while ((m = begin.exec(text)) !== null) {
    const from = m.index;
    const after = from + m[0].length;
    let end = text.indexOf('-----END ', after);
    if (end === -1 || end - after > KEY_BLOCK_MAX)
      end = Math.min(text.length, after + KEY_BLOCK_MAX);
    else {
      const close = text.indexOf('-----', end + 9);
      end = close === -1 || close - end > 64 ? end + 9 : close + 5;
    }
    out += text.slice(cursor, from) + REDACTED;
    cursor = end;
    begin.lastIndex = end;
  }
  return cursor === 0 ? text : out + text.slice(cursor);
}

const NAMED =
  '(?:api[_-]?key|api[_-]?token|access[_-]?key|access[_-]?token|auth[_-]?token|secret[_-]?key|' +
  'client[_-]?secret|access[_-]?secret|private[_-]?key|passwd|password|(?:github|gh|cloudflare|cf|vast)[_-]?(?:api[_-]?)?(?:token|key))';

// Shapes, in this order. Each is (regex, replacement); the key words stay readable.
const SHAPES = [
  [/\bBearer[ \t]{1,32}[A-Za-z0-9._~+/=-]{20,256}/gi, 'Bearer ' + REDACTED],
  [
    /(CF-Access-Client-(?:Id|Secret)[ \t]{0,32}[:=][ \t]{0,32})[A-Za-z0-9._-]{8,256}/gi,
    '$1' + REDACTED,
  ],
  [
    /((?:instance_api_key|vast[_-]?api[_-]?key)["']?[ \t]{0,32}[:=][ \t]{0,32}["']?)[A-Za-z0-9]{24,256}/gi,
    '$1' + REDACTED,
  ],
  [/\b(?:gh[pousr]_[A-Za-z0-9]{20,255}|github_pat_[A-Za-z0-9_]{20,255})/g, REDACTED],
  [/(\bhttps?:\/\/)[^\s/:@]{1,64}:[^\s/@]{1,128}@/gi, '$1' + REDACTED + '@'],
  [
    new RegExp(
      '((?<![A-Za-z0-9])' + NAMED + '["\']?[ \\t]{0,32}[:=][ \\t]{0,32}["\']?)[^\\s"\'&,;]{8,256}',
      'gi',
    ),
    '$1' + REDACTED,
  ],
];
const PRESIGNED =
  /X-Amz-(?:Signature|Credential|Security-Token)=|AWSAccessKeyId=|[?&]Signature=[0-9a-f]{32,256}/gi;

// The redaction of one text. `exact` comes from exactValues().
export function redactPublic(text, exact = []) {
  // Shapes first, then values: every run token starts with the same 16 characters (the HS256 JWT
  // header), so replacing the fragment of OUR token first would break the shape of every other JWT.
  let out = redactKeyBlocks(text);
  out = redactJwt(out);
  out = redactAround(out, PRESIGNED);
  for (const [pattern, replacement] of SHAPES) out = out.replace(pattern, replacement);
  for (const value of exact) if (out.includes(value)) out = out.split(value).join(REDACTED);
  return out;
}

// ---- the cap ----------------------------------------------------------------------------------

// UTF-8 prefix / suffix of `text` of at most `bytes` bytes, never cutting a character.
function prefixBytes(text, bytes) {
  const b = encoder.encode(text);
  if (b.length <= bytes) return text;
  let cut = bytes;
  while (cut > 0 && (b[cut] & 0xc0) === 0x80) cut--;
  return decoder.decode(b.slice(0, cut));
}
function suffixBytes(text, bytes) {
  const b = encoder.encode(text);
  if (b.length <= bytes) return text;
  let cut = b.length - bytes;
  while (cut < b.length && (b[cut] & 0xc0) === 0x80) cut++;
  return decoder.decode(b.slice(cut));
}

// The published log of a run from the stored pieces (the texts the box streamed, in order):
// redacted, then capped. `omitted` = bytes the ledger could not store (its own cap) and said so.
// Over the cap, the head and the tail stay and a marker line says how much is gone.
export async function buildPublicLog(parts, exact, { omitted = 0, max = LOG_MAX_BYTES } = {}) {
  let text = redactPublic(attribute(parts.join('')), exact);
  let bytes = encoder.encode(text).length;
  let cut = 0;
  if (omitted > 0) {
    text += '\n[kayfabe-log] ' + omitted + ' further bytes were not stored (16 MiB cap)\n';
    bytes = encoder.encode(text).length;
  }
  if (bytes > max) {
    const tail = 1024 * 1024;
    const room = max - 4096 - tail;
    const head = prefixBytes(text, room);
    const end = suffixBytes(text, tail);
    cut = bytes - encoder.encode(head).length - encoder.encode(end).length;
    text = head + '\n[kayfabe-log] CUT: ' + cut + ' bytes omitted here (16 MiB cap)\n' + end;
    bytes = encoder.encode(text).length;
  }
  return { text, bytes, cut, sha256: await sha256Hex(text) };
}

// ---- stage markers -> byte offsets ------------------------------------------------------------

// Byte offsets of the stage markers the payload prints (KF_PAYLOAD_STAGE_START/END name=<stage>) in
// the log THE WORKER WROTE, so that a viewer can jump to a subrun. The box cannot know these
// offsets: redaction and the Worker's own lines move every byte. Returns, per stage name,
// {start_byte, end_byte}: the start of the first START line and the end of the last END line.
export function stageOffsets(text) {
  const found = new Map();
  const re = /KF_PAYLOAD_STAGE_(START|END) name=([a-z][a-z0-9_]{0,47})/g;
  let m;
  while ((m = re.exec(text)) !== null) {
    const begin = text.lastIndexOf('\n', m.index) + 1;
    const stop = text.indexOf('\n', m.index);
    const e = found.get(m[2]) ?? { start: null, end: null };
    if (m[1] === 'START' && e.start === null) e.start = begin;
    if (m[1] === 'END') e.end = stop === -1 ? text.length : stop + 1;
    found.set(m[2], e);
  }
  // Characters to bytes in one pass over the sorted positions.
  const positions = [...found.values()].flatMap((e) => [e.start, e.end]).filter((x) => x !== null);
  positions.sort((a, b) => a - b);
  const bytesAt = new Map();
  let chars = 0;
  let total = 0;
  for (const p of positions) {
    if (bytesAt.has(p)) continue;
    total += encoder.encode(text.slice(chars, p)).length;
    chars = p;
    bytesAt.set(p, total);
  }
  const out = new Map();
  for (const [name, e] of found)
    if (e.start !== null && e.end !== null && e.end >= e.start)
      out.set(name, { start_byte: bytesAt.get(e.start), end_byte: bytesAt.get(e.end) });
  return out;
}

// ---- storage ----------------------------------------------------------------------------------

export async function withTimeout(promise, ms) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => (timer = setTimeout(() => reject(new Error('timeout')), ms))),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

// Writes one object to the public bucket. Throws on failure or timeout (the caller fails open).
export async function putPublic(env, key, body, contentType) {
  await withTimeout(
    env.LOGS_BUCKET.put(key, body, { httpMetadata: { contentType } }),
    publishTimeouts.r2,
  );
}

export const logKey = (runId) => runId + '/log.txt';
export const reportKey = (runId) => runId + '/report.json';
