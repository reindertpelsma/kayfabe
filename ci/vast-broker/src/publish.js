// Publishes the full log and the structured report of settled runs (2026-11-07). Driven by the
// cron, AFTER the serialized teardown sweep and OUTSIDE its queue (like src/ingest.js storeTrees):
// the verdict, the teardown and the budget never wait for, or depend on, the bucket or the
// database. Each run has one queue entry, written by Ledger.settle:
//   plogq:<run key>:<leg>  {attempts, chunks, dropped, rchunks, leg, key, run, attempt, sha, nonce,
//                           runnerNonce, reportExpiry, at, logPut?, reportPut?, d1?}
// A tick handles at most PER_TICK entries within BUDGET_MS, counts the attempt BEFORE the work (a
// call that never returns still counts), writes each object with a bounded wait, marks every
// finished step in the entry (so a retry repeats only what is missing) and deletes the entry when
// nothing is left. Every failure is ONE log line with a fixed word and the run is retried by the
// next due tick (it backs off, see backoffMs), at most 60 times; after that the entry is dropped (the
// pieces expire with the run).
//   1. <run id>/log.txt     the whole stored text, redacted (src/publiclog.js)         -> R2
//   2. <run id>/report.json the box's claims with the Worker's own identity             -> R2, D1
// Step 2 needs the run's FINAL row in D1 (identity, verdict, times): it waits for the store queue.
import {
  buildPublicLog,
  exactValues,
  logKey,
  logsEnabled,
  putPublic,
  reportKey,
  stageOffsets,
} from './publiclog.js';
import { PUBLISH_PREFIX, reportChunkKey } from './ledger.js';
import { RunTokens } from './tokens.js';
import { storeEnabled } from './ingest.js';
import {
  buildReport,
  cleanClaimsReport,
  fitReport,
  readRunRow,
  redactDoc,
  storeReport,
} from './runreport.js';

export const MAX_ATTEMPTS = 60;
export const PER_TICK = 4;
// An entry that could not finish waits before its next try (review of #282, F3), so a few stuck
// runs never keep the others from being published: 1, 2, 4 minutes, then 5. 60 attempts therefore
// span about 4.5 hours, inside the 6 hours the pieces are kept.
export const backoffMs = (attempts) => Math.min(60000 * 2 ** Math.max(0, attempts - 1), 300000);
export const BUDGET_MS = 40000;
export const DEFAULT_LOGS_BASE = 'https://logs.kayfabe.dev';

const event = (fields) => console.log(JSON.stringify({ event: 'public_log', ...fields }));

// The public https origin of the log bucket, for the url inside a report. Only a https origin on
// kayfabe.dev is accepted (the public API applies the same rule to its LOGS_BASE).
export function logsBase(env) {
  const v = env?.LOGS_BASE ?? DEFAULT_LOGS_BASE;
  try {
    const u = new URL(v);
    const ok =
      u.protocol === 'https:' &&
      (u.hostname === 'kayfabe.dev' || u.hostname.endsWith('.kayfabe.dev')) &&
      !u.username &&
      !u.password &&
      !u.port &&
      !u.search &&
      !u.hash &&
      u.pathname === '/';
    return ok ? u.origin : null;
  } catch {
    return null;
  }
}

// Everything this Worker holds that must not appear in a public log.
async function secretsOf(env, entry) {
  const values = [
    env.RUN_TOKEN_SECRET,
    env.OWNER_STOP_TOKEN,
    env.VAST_API_KEY,
    env.GITHUB_API_TOKEN,
    env.BLOB_GITHUB_TOKEN,
    env.BOX_ACCESS_ID,
    env.BOX_ACCESS_SECRET,
    env.R2_READ_ACCESS_KEY_ID,
    env.R2_READ_SECRET_ACCESS_KEY,
    env.ROW_TAG_KEY,
    env.BROKER_PATH_PREFIX,
    entry.nonce,
    entry.runnerNonce,
    ...(entry.priorNonces ?? []),
    ...(entry.known ?? []),
  ];
  if (env.BROKER_PATH_PREFIX && env.WORKER_URL)
    values.push(String(env.WORKER_URL).replace(/\/+$/, '') + '/' + env.BROKER_PATH_PREFIX);
  // The token of every leg of this run (a replacement leg's log carries the earlier legs' text).
  for (const nonce of [entry.nonce, ...(entry.priorNonces ?? [])])
    try {
      values.push(
        await new RunTokens(env.RUN_TOKEN_SECRET).issue({
          run: entry.run,
          attempt: entry.attempt,
          sha: entry.sha,
          nonce,
          key: entry.key,
          label: 'kf-ci-' + entry.key,
          reportExpiry: entry.reportExpiry,
        }),
      );
    } catch {}
  return exactValues(values);
}

async function readPieces(ledger, name, count) {
  const parts = [];
  for (let i = 0; i < count; i++) {
    const piece = await ledger.get(name(i));
    if (typeof piece !== 'string') return null; // expired or never stored
    parts.push(piece);
  }
  return parts;
}

// One entry. Returns 'DONE', 'WAIT' or 'DROP'.
async function publishOne({ ledger, env, entry, key, now, runId }) {
  const logs = logsEnabled(env);
  const store = storeEnabled(env);
  let built = null;
  const exact = await secretsOf(env, entry);
  if (entry.chunks > 0) {
    const parts = await readPieces(ledger, (i) => 'plog:' + entry.key + ':' + i, entry.chunks);
    if (parts === null) {
      event({ state: 'DROPPED', why: 'expired' });
      return 'DROP';
    }
    built = await buildPublicLog(parts, exact, {
      omitted: entry.dropped,
      endpoints: entry.endpointRedactions,
    });
  }
  if (built && logs && !entry.logPut) {
    await putPublic(env, logKey(runId), built.text, 'text/plain; charset=utf-8');
    entry.logPut = true;
    await ledger.put(key, entry);
    event({ state: 'LOG_STORED', bytes: built.bytes, cut: built.cut });
  }
  const wantsReport = store && (entry.rchunks > 0 || built !== null) && !entry.d1;
  if (wantsReport) {
    const row = await readRunRow(env, runId);
    if (row === null) {
      event({ state: 'RETRY_LATER', why: 'row_missing' });
      return 'WAIT';
    }
    let doc = null;
    if (entry.rchunks > 0) {
      const parts = await readPieces(
        ledger,
        (i) => reportChunkKey(entry.key, entry.leg, i),
        entry.rchunks,
      );
      let claims = null;
      try {
        claims = parts === null ? null : cleanClaimsReport(JSON.parse(parts.join('')));
      } catch {}
      if (claims !== null) {
        const full = buildReport({
          claims,
          row,
          runId,
          log: built && entry.logPut ? built : null,
          offsets: built ? stageOffsets(built.text) : null,
          logsBase: logsBase(env),
        });
        doc = fitReport(redactDoc(full, exact, entry.endpointRedactions));
      }
    }
    if (doc !== null && logs && !entry.reportPut) {
      await putPublic(
        env,
        reportKey(runId),
        JSON.stringify(doc),
        'application/json; charset=utf-8',
      );
      entry.reportPut = true;
      await ledger.put(key, entry);
    }
    const state = await storeReport(env, {
      runId,
      doc,
      hostDriver: row.host_driver,
      log: built && entry.logPut ? built : null,
      now,
    });
    if (state !== 'STORED') {
      event({ state: 'RETRY_LATER', why: 'row_missing' });
      return 'WAIT';
    }
    entry.d1 = true;
    await ledger.put(key, entry);
    event({ state: 'REPORT_STORED', report: doc !== null });
  }
  return 'DONE';
}

export async function publishTick({ ledger, env, now, clock = Date.now, budgetMs = BUDGET_MS }) {
  if (!logsEnabled(env) && !storeEnabled(env)) return;
  const deadline = clock() + budgetMs;
  let worked = 0;
  for (const [key, stored] of await ledger.storage.list({ prefix: PUBLISH_PREFIX, limit: 64 })) {
    if (worked >= PER_TICK || clock() >= deadline) break;
    if (Number.isFinite(stored?.nextAt) && stored.nextAt > now) continue; // backing off
    const m = /^plogq:([1-9][0-9]{0,19}-[1-9][0-9]{0,19}):((?:[0-9]|[12][0-9]|3[01]))$/.exec(key);
    const entry = stored;
    if (
      !m ||
      entry === null ||
      typeof entry !== 'object' ||
      entry.key !== m[1] ||
      entry.leg !== Number(m[2])
    ) {
      await ledger.storage.delete(key);
      event({ state: 'DROPPED', why: 'entry' });
      continue;
    }
    worked++;
    entry.attempts = (entry.attempts ?? 0) + 1;
    if (entry.attempts > MAX_ATTEMPTS) {
      await ledger.storage.delete(key);
      event({ state: 'DROPPED', why: 'attempts' });
      continue;
    }
    await ledger.put(key, entry);
    let outcome = 'WAIT';
    try {
      outcome = await publishOne({
        ledger,
        env,
        entry,
        key,
        now,
        runId: 'vast-' + m[1] + '-' + m[2],
      });
    } catch (error) {
      // R2, D1 or a timeout: fixed word only (an error message may name a bucket or a key).
      event({
        state: 'RETRY_LATER',
        why: /timeout/i.test(String(error?.message)) ? 'timeout' : 'error',
      });
    }
    if (outcome === 'DONE' || outcome === 'DROP') await ledger.storage.delete(key);
    else await ledger.put(key, { ...entry, nextAt: now + backoffMs(entry.attempts) });
  }
}
