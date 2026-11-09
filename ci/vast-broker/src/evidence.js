// Failed-run evidence (2026-11-06; docs/BOX_PROTOCOL.md, "Evidence upload"; DEPLOY.md, "Evidence
// bucket"). A box that holds a live run token may POST ONE bounded, scrubbed bundle for its own
// rental to POST /evidence/<run>-<attempt>. The Worker stores it in a PRIVATE R2 bucket through a
// write binding (EVIDENCE_BUCKET, bucket kayfabe-ci-evidence) and never links, lists or serves it:
// only the coordinator reads it, with its own Cloudflare credentials (scripts/ci/fetch_evidence.py).
// The Worker never opens the archive (no decompression, so no bomb); it checks the size before it
// reads a byte, the gzip magic, the SHA-256 the box declared, and the shape of the small metadata
// header. Nothing the box sends becomes a verdict, a log line or a public field.
// prettier-ignore
import {BrokerError,refuse,budgetClass,readBounded} from './policy.js';
// prettier-ignore
import {sha256Hex} from './tokens.js';
// Hard cap of the compressed bundle; scripts/ci/vast_evidence.py builds to the same number.
export const EVIDENCE_MAX_BYTES = 8 * 1024 * 1024;
export const EVIDENCE_META_MAX_BYTES = 4096;
// Reservations open at once across the whole ledger (each is a body held in a gateway isolate).
export const EVIDENCE_MAX_INFLIGHT = 4;
// How long one R2 call may take; a variable only so that tests can shorten it.
export const evidenceTimeouts = { r2: 20000 };
export const evidenceEnabled = (env) =>
  env.EVIDENCE_BUCKET !== undefined &&
  env.EVIDENCE_BUCKET !== null &&
  typeof env.EVIDENCE_BUCKET.put === 'function';
const own = (object, keys) =>
  object !== null &&
  typeof object === 'object' &&
  !Array.isArray(object) &&
  Object.keys(object).every((k) => keys.includes(k));
const count = (v, max = 2 ** 40) => Number.isSafeInteger(v) && v >= 0 && v <= max;
const SECTION = /^[a-z][a-z0-9_]{0,23}$/;
// The box-claimed metadata: sections kept, the scrub counts and the scan result. Strict shape, small
// numbers and fixed words only, so that nothing free-form is stored next to the evidence.
export function parseEvidenceMeta(header) {
  if (typeof header !== 'string' || header.length === 0 || header.length > EVIDENCE_META_MAX_BYTES)
    refuse('BAD_REQUEST', 400, 'EVIDENCE_META');
  let meta;
  try {
    if (!/^[\x20-\x7e]+$/.test(header)) throw Error();
    meta = JSON.parse(header);
  } catch {
    refuse('BAD_REQUEST', 400, 'EVIDENCE_META');
  }
  const scrubKeys = ['uuid', 'serial', 'mac', 'ipv4', 'ipv6'];
  if (
    !own(meta, ['v', 'sections', 'dropped', 'raw_bytes', 'scrub', 'scan']) ||
    meta.v !== 1 ||
    meta.scan !== 'clean' ||
    !Array.isArray(meta.sections) ||
    meta.sections.length > 16 ||
    !count(meta.dropped, 100000) ||
    !count(meta.raw_bytes, 2 ** 31) ||
    !own(meta.scrub, scrubKeys) ||
    !scrubKeys.every((k) => count(meta.scrub[k], 100000000))
  )
    refuse('BAD_REQUEST', 400, 'EVIDENCE_META');
  for (const s of meta.sections)
    if (
      !own(s, ['name', 'files', 'raw_bytes', 'dropped']) ||
      typeof s.name !== 'string' ||
      !SECTION.test(s.name) ||
      !count(s.files, 100000) ||
      !count(s.raw_bytes, 2 ** 31) ||
      !count(s.dropped, 100000)
    )
      refuse('BAD_REQUEST', 400, 'EVIDENCE_META');
  return meta;
}
// <profile>/<run id>/<attempt>.tgz and .json, and <attempt>-r1 for the replacement rental after a
// provider fault (it reuses the run key, so without the suffix its bundle would meet the faulted
// box's object). A release or admin leg is an ordinary rental with its own run key.
export function evidenceKeys(record) {
  const m = /^([1-9][0-9]{0,19})-([1-9][0-9]{0,19})$/.exec(String(record.key));
  const profile = budgetClass(record);
  const retries = record.providerRetries ?? 0;
  if (
    !m ||
    !/^[a-z]{3,16}$/.test(profile) ||
    !(Number.isInteger(retries) && retries >= 0 && retries < 32)
  )
    refuse('UNAVAILABLE', 503, undefined, 'evidence_key');
  const base = profile + '/' + m[1] + '/' + m[2] + (retries ? '-r' + retries : '');
  return { run: m[1], attempt: m[2], profile, object: base + '.tgz', manifest: base + '.json' };
}
// What the ledger tells the gateway at reservation: the object keys, and the facts of the rental
// that the manifest records (all small; nothing the box sent).
export function evidenceReservation(record) {
  return {
    keys: evidenceKeys(record),
    facts: {
      tier: record.tier ?? null,
      family: record.family ?? null,
      die: record.die ?? null,
      batch: record.batch ?? null,
      verdict: record.verdict ?? null,
    },
  };
}
// Before a byte is read: a length the box declared, within the cap. (The gateway repeats this check
// before it buffers; the ledger repeats it before it reads.)
export function evidenceLength(request) {
  const raw = request.headers.get('Content-Length');
  if (raw === null || !/^[1-9][0-9]{0,9}$/.test(raw)) refuse('LENGTH_REQUIRED', 411);
  if (Number(raw) > EVIDENCE_MAX_BYTES) refuse('PAYLOAD_TOO_LARGE', 413);
  return Number(raw);
}
async function withTimeout(promise, ms) {
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
// Reads the body, verifies it and writes it to the bucket. Runs in the gateway (never in the
// ledger Durable Object): `reservation` comes from the ledger's reserve step, which has already
// authenticated the rental and set its once-only flag.
export async function storeEvidence(env, reservation, request, now) {
  const length = evidenceLength(request);
  const meta = parseEvidenceMeta(request.headers.get('X-KF-Evidence-Meta'));
  const claimed = request.headers.get('X-KF-Evidence-SHA256');
  const { keys, facts } = reservation;
  let body;
  try {
    body = await readBounded(request, EVIDENCE_MAX_BYTES);
  } catch (error) {
    // readBounded refuses a body above the cap; anything else is a body that broke off.
    if (error instanceof BrokerError) refuse('PAYLOAD_TOO_LARGE', 413);
    refuse('BAD_REQUEST', 400, 'EVIDENCE_BODY');
  }
  if (body.length !== length || body.length < 20) refuse('BAD_REQUEST', 400, 'EVIDENCE_LENGTH');
  // A gzip stream; the Worker does not open it.
  if (body[0] !== 0x1f || body[1] !== 0x8b) refuse('BAD_REQUEST', 400, 'EVIDENCE_FORMAT');
  const sha256 = await sha256Hex(body);
  // The box must have declared exactly the digest of the bytes it sent (an absent header never matches).
  if (sha256 !== claimed) refuse('BAD_REQUEST', 400, 'EVIDENCE_SHA256');
  const bucket = env.EVIDENCE_BUCKET;
  // Best effort, on top of the ledger's once-per-rental flag (src/index.js): never overwrite.
  try {
    if (
      typeof bucket.head === 'function' &&
      (await withTimeout(bucket.head(keys.object), evidenceTimeouts.r2))
    )
      refuse('EVIDENCE_EXISTS', 409);
  } catch (error) {
    if (error instanceof BrokerError) throw error;
    refuse('UNAVAILABLE', 503, undefined, 'evidence_head');
  }
  try {
    await withTimeout(
      bucket.put(keys.object, body, {
        httpMetadata: { contentType: 'application/gzip' },
        customMetadata: { run: keys.run, attempt: keys.attempt, sha256 },
      }),
      evidenceTimeouts.r2,
    );
  } catch {
    refuse('UNAVAILABLE', 503, undefined, 'evidence_put');
  }
  const manifest = {
    format: 1,
    run: keys.run,
    attempt: keys.attempt,
    profile: keys.profile,
    object: keys.object,
    sha256,
    size: body.length,
    uploaded_at: new Date(now).toISOString(),
    // The next fields are what the BOX said about its own bundle; the Worker verified only their
    // shape. Never evidence of what the bundle really holds.
    box_claim: { sections: meta.sections, dropped: meta.dropped, raw_bytes: meta.raw_bytes },
    scrub: meta.scrub,
    scan: meta.scan,
    // Facts of the rental the Worker knows itself.
    ...facts,
  };
  let manifestStored = true;
  try {
    await withTimeout(
      bucket.put(keys.manifest, JSON.stringify(manifest), {
        httpMetadata: { contentType: 'application/json' },
      }),
      evidenceTimeouts.r2,
    );
  } catch {
    manifestStored = false;
  }
  console.log(
    JSON.stringify({
      event: 'evidence_stored',
      run: keys.run + '-' + keys.attempt,
      bytes: body.length,
      manifest: manifestStored,
    }),
  );
  return { state: 'STORED', run: keys.run + '-' + keys.attempt, size: body.length, sha256 };
}
