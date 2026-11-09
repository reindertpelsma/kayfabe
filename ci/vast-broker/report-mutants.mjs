// Mutation check for the public log, the run report and their publisher (src/publiclog.js,
// src/runreport.js, src/publish.js and their wiring in src/index.js, src/ledger.js, src/routes.js),
// GPU-free. Each mutant is applied in place (exact-once replacements), the three test files run, and
// every file is restored before the next mutant (also on error). Every mutant must make at least
// TWO named tests fail. Run with no other test running here:
//   node ci/vast-broker/report-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const TESTS = [
  here + 'test/public-log.test.js',
  here + 'test/run-report.test.js',
  here + 'test/run-report-extra.test.js',
];
const L = 'src/publiclog.js',
  R = 'src/runreport.js',
  P = 'src/publish.js',
  I = 'src/index.js',
  G = 'src/ledger.js',
  T = 'src/routes.js';
// [name, [[file, before, after], ...]]
const MUTANTS = [
  // ---- redaction ----
  [
    'exact values not replaced',
    [[L, 'if (out.includes(value)) out = out.split(value).join(REDACTED);', 'void value;']],
  ],
  [
    'exact minimum lowered to one',
    [[L, 'export const EXACT_MIN = 8;', 'export const EXACT_MIN = 1;']],
  ],
  ['fragments dropped', [[L, 'if (v.length >= 2 * FRAGMENT) {', 'if (false) {']]],
  ['jwt shape not redacted', [[L, '  out = redactJwt(out);\n', '']]],
  [
    'jwt signature length unchecked',
    [[L, 'parts[i + 2].length >= 20', 'parts[i + 2].length >= 0']],
  ],
  [
    'bearer shape not redacted',
    [[L, "  [/\\bBearer[ \\t]{1,32}[A-Za-z0-9._~+/=-]{20,256}/gi, 'Bearer ' + REDACTED],\n", '']],
  ],
  ['presigned urls kept', [[L, '  out = redactAround(out, PRESIGNED);\n', '']]],
  [
    'presigned redaction leaves the rest of the url',
    [[L, 'while (end < ceil && !TOKEN_BREAK.test(text[end])) end++;', 'void ceil;']],
  ],
  ['private key blocks kept', [[L, '  let out = redactKeyBlocks(text);\n', '  let out = text;\n']]],
  [
    'private key block keeps its body',
    [
      [
        L,
        '    out += text.slice(cursor, from) + REDACTED;\n    cursor = end;\n    begin.lastIndex = end;',
        '    out += text.slice(cursor, from) + REDACTED;\n    cursor = after;\n    begin.lastIndex = after;',
      ],
    ],
  ],
  [
    'access headers kept',
    [
      [
        L,
        "  [\n    /(CF-Access-Client-(?:Id|Secret)[ \\t]{0,32}[:=][ \\t]{0,32})[A-Za-z0-9._-]{8,256}/gi,\n    '$1' + REDACTED,\n  ],\n",
        '',
      ],
    ],
  ],
  [
    'github tokens kept',
    [
      [
        L,
        '  [/\\b(?:gh[pousr]_[A-Za-z0-9]{20,255}|github_pat_[A-Za-z0-9_]{20,255})/g, REDACTED],\n',
        '',
      ],
    ],
  ],
  [
    'url credentials kept',
    [
      [
        L,
        "  [/(\\bhttps?:\\/\\/)[^\\s/:@]{1,64}:[^\\s/@]{1,128}@/gi, '$1' + REDACTED + '@'],\n",
        '',
      ],
    ],
  ],
  [
    'named assignments kept',
    [[L, "      '((?<![A-Za-z0-9])' + NAMED +", "      '((?<![A-Za-z0-9])' + 'nevermatches' +"]],
  ],
  ['named assignment minimum dropped', [[L, '[^\\\\s"\\\'&,;]{8,256}', '[^\\\\s"\\\'&,;]{1,256}']]],
  [
    'addresses redacted too (filtering beyond secrets)',
    [
      [
        L,
        '  let out = redactKeyBlocks(text);\n',
        "  let out = redactKeyBlocks(text).replace(/\\b(?:[0-9]{1,3}\\.){3}[0-9]{1,3}\\b/g, '[address]');\n",
      ],
    ],
  ],
  [
    'long hex redacted too (filtering beyond secrets)',
    [
      [
        L,
        '  let out = redactKeyBlocks(text);\n',
        "  let out = redactKeyBlocks(text).replace(/[0-9a-f]{40,}/g, '[redacted]');\n",
      ],
    ],
  ],
  [
    'control characters stripped (filtering beyond secrets)',
    [
      [
        L,
        '  let out = redactKeyBlocks(text);\n',
        "  let out = redactKeyBlocks(text).replace(/[\\x00-\\x08\\x0b-\\x1f]/g, '');\n",
      ],
    ],
  ],
  [
    'pieces redacted one by one (a split secret survives)',
    [
      [
        L,
        "let text = redactPublic(attribute(parts.join('')), exact);",
        "let text = parts.map((p) => redactPublic(attribute(p), exact)).join('');",
      ],
    ],
  ],
  // ---- the cap ----
  [
    'cap raised silently',
    [
      [
        L,
        'export const LOG_MAX_BYTES = 16 * 1024 * 1024;',
        'export const LOG_MAX_BYTES = 64 * 1024 * 1024;',
      ],
    ],
  ],
  [
    'cap lowered',
    [
      [
        L,
        'export const LOG_MAX_BYTES = 16 * 1024 * 1024;',
        'export const LOG_MAX_BYTES = 8 * 1024 * 1024;',
      ],
    ],
  ],
  [
    'cut without a marker',
    [
      [
        L,
        "text = head + '\\n[kayfabe-log] CUT: ' + cut + ' bytes omitted here (16 MiB cap)\\n' + end;",
        'text = head + end;',
      ],
    ],
  ],
  ['cut drops the tail', [[L, '    const end = suffixBytes(text, tail);', "    const end = '';"]]],
  ['cut splits a character', [[L, '  while (cut > 0 && (b[cut] & 0xc0) === 0x80) cut--;', '']]],
  [
    'not-stored bytes unnamed',
    [
      [
        L,
        "    text += '\\n[kayfabe-log] ' + omitted + ' further bytes were not stored (16 MiB cap)\\n';",
        '',
      ],
    ],
  ],
  [
    'digest of the wrong text',
    [[L, 'sha256: await sha256Hex(text) };', "sha256: await sha256Hex(parts.join('')) };"]],
  ],
  [
    'offsets in characters',
    [[L, 'total += encoder.encode(text.slice(chars, p)).length;', 'total += p - chars;']],
  ],
  [
    'offset end before the end marker',
    [
      [
        L,
        "if (m[1] === 'END') e.end = stop === -1 ? text.length : stop + 1;",
        "if (m[1] === 'END') e.end = m.index;",
      ],
    ],
  ],
  // ---- the report validator ----
  [
    'unknown sub rows kept',
    [
      [
        R,
        "if (!object(t) || typeof t.sub !== 'string' || !ids.has(t.sub)) continue;",
        'if (!object(t)) continue;',
      ],
    ],
  ],
  [
    'specviewperf not dropped',
    [
      [
        R,
        'const FORBIDDEN = /specviewperf|spec[ _-]?viewperf|viewperf/i;',
        'const FORBIDDEN = /(?!)/;',
      ],
    ],
  ],
  [
    'tokens and urls not dropped',
    [
      [
        R,
        "if (t === '' || TOKENISH.test(t) || FORBIDDEN.test(t)) return null;",
        "if (t === '' || FORBIDDEN.test(t)) return null;",
      ],
    ],
  ],
  ['non-ASCII kept', [[R, ".replace(/[^\\x20-\\x7e]/g, '?')", '']]],
  ['strings unbounded', [[R, '    .slice(0, max);', ';']]],
  [
    'non-finite numbers kept',
    [
      [
        R,
        "typeof value === 'number' && Number.isFinite(value) && Math.abs(value) <= 1e15 ? value : null;",
        "typeof value === 'number' ? value : null;",
      ],
    ],
  ],
  [
    'arrays uncapped',
    [
      [
        R,
        'const list = (value, cap) => (Array.isArray(value) ? value.slice(0, cap) : []);',
        'const list = (value) => (Array.isArray(value) ? value : []);',
      ],
    ],
  ],
  ['subrun id repeated', [[R, 'if (c && !ids.has(c.sub)) {', 'if (c) {']]],
  [
    'subrun contradicting itself kept',
    [
      [
        R,
        "if (raw.os !== os || (os === 'host' ? sub !== 'host' : !sub.startsWith(os))) return null;",
        '',
      ],
    ],
  ],
  [
    'driver of the id unchecked',
    [
      [
        R,
        "if (/^(?:linux|windows)-[0-9]/.test(sub) && sub.slice(sub.indexOf('-') + 1) !== guestDriver)\n    return null;",
        '',
      ],
    ],
  ],
  [
    'not run without a reason',
    [[R, "if (verdict === 'NOT_RUN' && reason === null) reason = 'not reported';", '']],
  ],
  [
    'unknown keys passed through',
    [
      [
        R,
        '    schema: 1,\n    guest_drivers: {',
        '    ...raw,\n    schema: 1,\n    guest_drivers: {',
      ],
    ],
  ],
  [
    'schema not required',
    [[R, 'if (!object(raw) || raw.schema !== 1) return null;', 'if (!object(raw)) return null;']],
  ],
  [
    'report not fitted',
    [[R, '    while (size(out) > max && out[key].length > 0) {', '    while (false) {']],
  ],
  [
    'truncation unannounced',
    [[R, 'return cut.length ? { ...out, truncated: cut } : out;', 'return out;']],
  ],
  [
    'verdict from the box',
    [
      [
        R,
        '    verdict: row.verdict,\n    stages: claims.stages,',
        "    verdict: claims.verdict ?? 'PASS',\n    stages: claims.stages,",
      ],
    ],
  ],
  [
    'tree of a commit-kind key shown',
    [[R, "tree: row.tree_kind === 'tree' ? row.tree_hash : null,", 'tree: row.tree_hash,']],
  ],
  [
    'log range guessed',
    [
      [
        R,
        '    let range = null;\n    if (offsets && log_stages.length) {',
        '    let range = { start_byte: 0, end_byte: 0 };\n    if (offsets && log_stages.length) {',
      ],
    ],
  ],
  [
    'report written for an in-flight run',
    [
      [
        R,
        'if (!row || row.verdict === null || row.verdict === undefined) return null;',
        'if (!row) return null;',
      ],
    ],
  ],
  [
    'old rows kept beside new',
    [
      [
        R,
        'bind(SQL.dropTests, runId),\n    bind(SQL.dropPerf, runId),',
        'bind(SQL.dropPerf, runId),',
      ],
    ],
  ],
  // ---- the route ----
  [
    'run report without the run token',
    [
      [
        I,
        "    const r = await ledger.get('record:' + key);\n    if (\n      !(await tokens.verify(bearer, r)) ||\n      !r ||\n      r.key !== key ||\n      r.ended !== null ||\n      r.closed ||\n      now() >= r.reportExpiry * 1000\n    )\n      refuse('UNAUTHORIZED', 401);\n    if (!storeEnabled(env)) refuse('REPORTS_DISABLED', 404);",
        "    const r = await ledger.get('record:' + key);\n    if (!r) refuse('UNAUTHORIZED', 401);\n    if (!storeEnabled(env)) refuse('REPORTS_DISABLED', 404);",
      ],
    ],
  ],
  [
    'run report accepted from a closed rental',
    [
      [
        I,
        "      r.key !== key ||\n      r.ended !== null ||\n      r.closed ||\n      now() >= r.reportExpiry * 1000\n    )\n      refuse('UNAUTHORIZED', 401);\n    if (!storeEnabled(env)) refuse('REPORTS_DISABLED', 404);",
        "      r.key !== key ||\n      now() >= r.reportExpiry * 1000\n    )\n      refuse('UNAUTHORIZED', 401);\n    if (!storeEnabled(env)) refuse('REPORTS_DISABLED', 404);",
      ],
    ],
  ],
  ['second report replaces the first', [[I, "if (r.runReport) refuse('REPORT_EXISTS', 409);", '']]],
  [
    'report body above the cap read',
    [
      [
        I,
        'body = await readBounded(request, REPORT_MAX_BYTES);',
        'body = await readBounded(request, REPORT_MAX_BYTES * 64);',
      ],
    ],
  ],
  [
    'report stored unfitted',
    [[I, 'const doc = claims === null ? null : fitReport(claims);', 'const doc = claims;']],
  ],
  ['bad report accepted', [[I, "if (doc === null) refuse('BAD_REQUEST', 400);", '']]],
  [
    'reports without a result store',
    [[I, "if (!storeEnabled(env)) refuse('REPORTS_DISABLED', 404);", '']],
  ],
  [
    'box told to send without a store',
    [[I, "...(storeEnabled(env) ? { KF_RUN_REPORT: '1' } : {}),", "KF_RUN_REPORT: '1',"]],
  ],
  ['box never told', [[I, "...(storeEnabled(env) ? { KF_RUN_REPORT: '1' } : {}),", '']]],
  [
    'run report not a box route',
    [[T, '/^\\/(?:report|run-report|kill|evidence)\\/', '/^\\/(?:report|kill|evidence)\\/']],
  ],
  [
    'run report not authorised at the gateway',
    [
      [
        I,
        '/^\\/(?:report|run-report|kill|blob-url|bench-data|evidence)\\//.test(path)) {',
        '/^\\/(?:report|kill|blob-url|bench-data|evidence)\\//.test(path)) {',
      ],
    ],
  ],
  [
    'gateway buffers the report at the log limit',
    [[I, '        ? REPORT_MAX_BYTES\n', '        ? 65536\n']],
  ],
  // ---- the ledger and the queue ----
  [
    'public log not stored (box text)',
    [[I, 'if (logsEnabled(env)) await ledger.appendPublicLog(r, payload.text);', '']],
  ],
  [
    'public log without the Worker lines',
    [[I, 'if (logsEnabled(env)) await ledger.appendPublicLog(r, text, undefined, true);', '']],
  ],
  [
    'public log stored without a bucket',
    [
      [
        I,
        'if (logsEnabled(env)) await ledger.appendPublicLog(r, payload.text);',
        'await ledger.appendPublicLog(r, payload.text);',
      ],
    ],
  ],
  ['ledger cap not applied', [[G, 'if (record.plogBytes + bytes > cap) {', 'if (false) {']]],
  [
    'ledger cap drops silently',
    [[G, 'record.plogDropped = (record.plogDropped ?? 0) + bytes;', 'void bytes;']],
  ],
  [
    'settle queues nothing',
    [[G, 'if ((record.plogChunks ?? 0) > 0 || record.runReport) {', 'if (false) {']],
  ],
  [
    'settle queue without the report',
    [[G, 'rchunks: record.runReport?.chunks ?? 0,', 'rchunks: 0,']],
  ],
  [
    'expiry keeps the pieces',
    [[G, "        await this.storage.delete('plog:' + key + ':' + i);", '        void i;']],
  ],
  [
    'expiry keeps the report chunks and entries',
    [
      [
        G,
        "      for (const prefix of ['rreport:' + key + ':', PUBLISH_PREFIX + key + ':'])",
        '      for (const prefix of [])',
      ],
    ],
  ],
  [
    'replacement leg drops the public log',
    [[I, "        'plogChunks',\n        'plogBytes',\n        'plogDropped',\n", '']],
  ],
  // ---- the publisher ----
  [
    'publisher inside the serial queue',
    [
      [
        I,
        '    async publishRuns() {\n      if (publishing) return;',
        '    async publishRuns() {\n      return serial(0, () => publishTick({ ledger, env, now: now() }));\n      if (publishing) return;',
      ],
    ],
  ],
  [
    'failure deletes the entry',
    [
      [
        P,
        "if (outcome === 'DONE' || outcome === 'DROP') await ledger.storage.delete(key);\n    else await ledger.put(key, { ...entry, nextAt: now + backoffMs(entry.attempts) });",
        'await ledger.storage.delete(key);',
      ],
    ],
  ],
  ['attempts unbounded', [[P, 'if (entry.attempts > MAX_ATTEMPTS) {', 'if (false) {']]],
  [
    'attempt counted after the work',
    [
      [
        P,
        "    await ledger.put(key, entry);\n    let outcome = 'WAIT';",
        "    let outcome = 'WAIT';",
      ],
    ],
  ],
  [
    'log written again on retry',
    [[P, 'if (built && logs && !entry.logPut) {', 'if (built && logs) {']],
  ],
  ['report waits for nothing', [[P, 'if (row === null) {', 'if (false) {']]],
  [
    'log identity recorded without a published log',
    [[P, 'log: built && entry.logPut ? built : null,\n      now,', 'log: built,\n      now,']],
  ],
  ['secrets of the run not redacted', [[P, '    entry.nonce,\n    entry.runnerNonce,\n', '']]],
  [
    'worker secrets not redacted',
    [[P, '    env.RUN_TOKEN_SECRET,\n    env.OWNER_STOP_TOKEN,', '    env.OWNER_STOP_TOKEN,']],
  ],
  [
    'bucket outage logged with its message',
    [
      [
        P,
        "why: /timeout/i.test(String(error?.message)) ? 'timeout' : 'error'",
        'why: String(error?.message)',
      ],
    ],
  ],
  [
    'publishing with neither bucket nor store',
    [[P, 'if (!logsEnabled(env) && !storeEnabled(env)) return;', '']],
  ],
  [
    'expired pieces retried forever',
    [
      [
        P,
        "      event({ state: 'DROPPED', why: 'expired' });\n      return 'DROP';",
        "      return 'WAIT';",
      ],
    ],
  ],
  [
    'report object written without the log url base check',
    [[P, "u.hostname === 'kayfabe.dev' || u.hostname.endsWith('.kayfabe.dev')", 'true']],
  ],
  // ---- review of #282 ----
  [
    'report strings not redacted',
    [[P, 'doc = fitReport(redactDoc(full, exact));', 'doc = fitReport(full);']],
  ],
  [
    'report redaction skips arrays',
    [
      [
        R,
        'if (Array.isArray(value)) return value.map((v) => redactDoc(v, exact));',
        'if (Array.isArray(value)) return value;',
      ],
    ],
  ],
  [
    'report redaction skips objects',
    [
      [
        R,
        '    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, redactDoc(v, exact)]));',
        '    return value;',
      ],
    ],
  ],
  [
    'queue without backoff',
    [
      [
        P,
        'else await ledger.put(key, { ...entry, nextAt: now + backoffMs(entry.attempts) });',
        'else await ledger.put(key, entry);',
      ],
    ],
  ],
  [
    'queue ignores the backoff',
    [
      [
        P,
        'if (Number.isFinite(stored?.nextAt) && stored.nextAt > now) continue; // backing off',
        '',
      ],
    ],
  ],
  [
    'queue backoff is zero',
    [[P, 'Math.min(60000 * 2 ** Math.max(0, attempts - 1), 300000);', '0;']],
  ],
  [
    'prior nonces not redacted',
    [
      [
        P,
        '    ...(entry.priorNonces ?? []),\n    ...(entry.known ?? []),',
        '    ...(entry.known ?? []),',
      ],
    ],
  ],
  [
    'prior tokens not redacted',
    [
      [
        P,
        'for (const nonce of [entry.nonce, ...(entry.priorNonces ?? [])])',
        'for (const nonce of [entry.nonce])',
      ],
    ],
  ],
  [
    'rent-time values not redacted',
    [
      [
        P,
        '    ...(entry.priorNonces ?? []),\n    ...(entry.known ?? []),',
        '    ...(entry.priorNonces ?? []),',
      ],
    ],
  ],
  [
    'rent-time values not recorded',
    [
      [
        I,
        'known: [env.BOX_ACCESS_ID, env.BOX_ACCESS_SECRET, env.BROKER_PATH_PREFIX].filter(',
        'known: [].filter(',
      ],
    ],
  ],
  [
    'prior nonces not carried to the replacement leg',
    [
      [
        I,
        'r.priorNonces = [...(retrySource.priorNonces ?? []), retrySource.nonce];',
        'r.priorNonces = [];',
      ],
    ],
  ],
  [
    'settle entry without prior nonces',
    [[G, 'priorNonces: record.priorNonces ?? [],', 'priorNonces: [],']],
  ],
  ['settle entry without rent-time values', [[G, 'known: record.known ?? [],', 'known: [],']]],
  [
    'access secret shape missing',
    [
      [
        L,
        'client[_-]?secret|access[_-]?secret|private[_-]?key',
        'client[_-]?secret|private[_-]?key',
      ],
    ],
  ],
  [
    'box lines never attributed',
    [[L, "    .replace(/^(?!\\ue000)(?=\\[Worker\\]|\\[kayfabe-log\\])/gm, '[box] ');", ';']],
  ],
  [
    'worker lines not marked',
    [[G, 'text = worker ? workerPiece(text) : boxPiece(text);', 'text = boxPiece(text);']],
  ],
  [
    'box text may carry the mark',
    [
      [
        L,
        "export const boxPiece = (text) => text.replace(/[\\ue000\\ue001]/g, '');",
        'export const boxPiece = (text) => text;',
      ],
    ],
  ],
  [
    'worker line glued to a box line',
    [
      [
        L,
        "    .replace(/([^\\n\\ue001])\\ue000/g, '$1\\n\\ue000') // a Worker line always starts a line\n",
        '',
      ],
    ],
  ],
  [
    'box line glued to a worker line',
    [[L, "    .replace(/([^\\n])\\ue001/g, '$1\\n') // and ends one\n", '']],
  ],
];

const selected = process.argv.slice(2);
let failed = false;
for (const [name, edits, mustKill] of MUTANTS) {
  if (selected.length && !selected.includes(name)) continue;
  const originals = new Map();
  let ambiguous = false;
  for (const [file, before] of edits) {
    const path = here + file;
    const text = originals.get(path) ?? readFileSync(path, 'utf8');
    originals.set(path, text);
    if (text.split(before).length !== 2) ambiguous = true;
  }
  if (ambiguous) {
    console.log(JSON.stringify({ mutant: name, ok: false, why: 'ambiguous or missing' }));
    failed = true;
    continue;
  }
  try {
    const current = new Map(originals);
    for (const [file, before, after] of edits) {
      const path = here + file;
      current.set(path, current.get(path).split(before).join(after));
    }
    for (const [path, text] of current) writeFileSync(path, text);
    const run = spawnSync(
      process.execPath,
      ['--no-warnings', '--test', '--test-reporter=tap', '--test-timeout=60000', ...TESTS],
      { cwd: here, encoding: 'utf8', timeout: 300000, maxBuffer: 64 * 1024 * 1024 },
    );
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)]
      .map((m) => m[1])
      .filter((t) => !t.endsWith('.test.js'));
    const complete =
      /^# tests \d+$/m.test(run.stdout ?? '') && /^# fail \d+$/m.test(run.stdout ?? '');
    const ok =
      run.status !== 0 &&
      kills.length >= 2 &&
      !run.error &&
      complete &&
      (mustKill === undefined || kills.includes(mustKill));
    failed ||= !ok;
    console.log(
      JSON.stringify({
        mutant: name,
        status: run.status,
        kills: kills.length,
        complete,
        ok,
        first: kills.slice(0, 3),
      }),
    );
  } finally {
    for (const [path, text] of originals) writeFileSync(path, text);
  }
}
if (failed) {
  console.log('SURVIVING OR UNDER-KILLED MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
