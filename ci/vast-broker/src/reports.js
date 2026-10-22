// prettier-ignore
import {checkedResult} from './results.js';
// prettier-ignore
import {refuse} from './policy.js';
export async function readReportBody(request, record, cfg) {
  const remaining = cfg.reportBytes - record.reportBytes,
    limit = Math.min(65536, remaining);
  const reader = request.body?.getReader();
  if (!reader) refuse('BAD_REPORT', 400);
  const chunks = [];
  let length = 0,
    oversized = false;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > limit) {
        oversized = true;
        break;
      }
      chunks.push(value);
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
  record.reportBytes = Math.min(cfg.reportBytes, record.reportBytes + length);
  if (oversized) refuse('REPORT_TOO_LARGE', 413); // MUTANT body-cap
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  return new TextDecoder('utf-8').decode(bytes);
}
export function parseReport(text) {
  // Refuse excess nesting before invoking even the native key-string decoder.
  {
    let depth = 0,
      quoted = false,
      escaped = false;
    for (const char of text) {
      if (quoted) {
        if (escaped) escaped = false;
        else if (char === '\\') escaped = true;
        else if (char === '"') quoted = false;
      } else if (char === '"') quoted = true;
      else if (char === '{' || char === '[') {
        if (++depth > 4) refuse('BAD_REPORT', 400);
      } else if (char === '}' || char === ']') {
        if (--depth < 0) refuse('BAD_REPORT', 400);
      }
    }
  }
  const stack = [];
  let quoted = false,
    escaped = false,
    start = 0;
  for (let i = 0; i < text.length; i++) {
    const char = text[i];
    if (quoted) {
      if (escaped) escaped = false;
      else if (char === '\\') escaped = true;
      else if (char === '"') {
        quoted = false;
        let next = i + 1;
        while (next < text.length && ' \t\r\n'.includes(text[next])) next++;
        if (stack.at(-1)?.kind === '{' && text[next] === ':') {
          let key;
          try {
            key = JSON.parse(text.slice(start, i + 1));
          } catch {
            refuse('BAD_REPORT', 400);
          }
          if (stack.at(-1).keys.has(key)) refuse('BAD_REPORT', 400);
          stack.at(-1).keys.add(key);
        }
      }
    } else if (char === '"') {
      quoted = true;
      start = i;
    } else if (char === '{' || char === '[') {
      stack.push({ kind: char, keys: new Set() });
      if (stack.length > 4) refuse('BAD_REPORT', 400);
    } else if (char === '}' || char === ']') {
      if (!stack.length) refuse('BAD_REPORT', 400);
      stack.pop();
    }
  }
  let p;
  try {
    p = JSON.parse(text);
  } catch {
    refuse('BAD_REPORT', 400);
  }
  const object = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);
  const keys = (x, allowed) => Object.keys(x).every((k) => allowed.includes(k));
  if (
    !object(p) ||
    !keys(p, ['seq', 'kind', 'text', 'result']) ||
    !Number.isSafeInteger(p.seq) ||
    p.seq < 1 ||
    p.seq > 2147483647 ||
    !['progress', 'log', 'result'].includes(p.kind)
  )
    refuse('BAD_REPORT', 400);
  if (
    p.text !== undefined &&
    (typeof p.text !== 'string' || new TextEncoder().encode(p.text).length > 60000)
  )
    refuse('BAD_REPORT', 400);
  if (p.kind === 'result') {
    checkedResult(p.result);
  } else if (typeof p.text !== 'string' || p.result !== undefined) refuse('BAD_REPORT', 400);
  return {
    seq: p.seq,
    kind: p.kind,
    ...(p.text === undefined ? {} : { text: p.text }),
    ...(p.result === undefined ? {} : { result: p.result }),
  };
}
