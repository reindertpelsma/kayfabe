import { sha256Hex } from './tokens.js';

export function fatalSignature(instance, log, id, policy) {
  for (const rule of policy.fatalSignatures) {
    const text = rule.source === 'status' ? instance?.status_msg : log;
    if (
      typeof text === 'string' &&
      rule.contains.every((s) =>
        text.toLowerCase().includes(s.replaceAll('{id}', id).toLowerCase()),
      )
    )
      return rule.reason;
  }
  return null;
}
export function providerMessageClass(instance, log, id, policy) {
  const reason = fatalSignature(instance, log, id, policy);
  if (reason) return { kind: 'FATAL', reason };
  const text = typeof instance?.status_msg === 'string' ? instance.status_msg : '';
  const ignored = policy.ignorableSignatures.some((s) =>
    text.toLowerCase().includes(s.toLowerCase()),
  );
  return { kind: ignored ? 'IGNORABLE' : 'UNKNOWN', reason: null };
}
// Keep hashes, not retained provider text. A rewritten/truncated log is not proof of new output.
export async function bootLogDelta(prior, text) {
  if (typeof text !== 'string') return { cursor: prior, fresh: '', stable: false };
  const hash = await sha256Hex(text);
  const lines = text.split('\n');
  if (text.endsWith('\n')) lines.pop();
  const tail = await Promise.all(lines.slice(-64).map(sha256Hex));
  const cursor = { length: text.length, hash, tail };
  if (!prior) return { cursor, fresh: text, stable: true };
  if (prior.hash === hash && prior.length === text.length)
    return { cursor, fresh: '', stable: true };
  if (
    Number.isSafeInteger(prior.length) &&
    prior.length >= 0 &&
    prior.length <= text.length &&
    (await sha256Hex(text.slice(0, prior.length))) === prior.hash
  )
    return { cursor, fresh: text.slice(prior.length), stable: true };
  const head = await Promise.all(lines.slice(0, 64).map(sha256Hex));
  for (let n = Math.min(prior.tail?.length ?? 0, head.length); n > 0; n--) {
    if (head.slice(0, n).every((value, i) => value === prior.tail[prior.tail.length - n + i]))
      return { cursor, fresh: lines.slice(n).join('\n'), stable: true };
  }
  return { cursor, fresh: '', stable: false };
}

const BOOT_FAILURE_STATES = new Set([
  'created',
  'loading',
  'starting',
  'scheduling',
  'running',
  'offline',
  'null',
  'error',
  'exited',
  'stopped',
]);

function bootState(state) {
  return state == null || BOOT_FAILURE_STATES.has(state) ? 'pending' : state;
}

export function orderedBootFailure(text, id, policy) {
  let reason = null;
  let progress = false;
  let healthy = false;
  let fragment = '';
  for (const line of text.split('\n')) {
    fragment += line + '\n';
    const fatal = fatalSignature(null, fragment, id, policy);
    if (fatal) {
      reason = fatal;
      healthy = false;
      fragment = '';
    } else if (
      policy.ignorableSignatures.some((s) => line.toLowerCase().includes(s.toLowerCase()))
    ) {
      reason = null;
      progress = true;
      healthy = policy.recoverySignatures.some((s) => line.toLowerCase().includes(s.toLowerCase()));
      fragment = '';
    }
  }
  return { reason, progress, healthy };
}

export async function currentBootFailure(prior, cursor, instance, log, id, policy) {
  const state = instance.actual_status;
  if (state != null && state !== 'running' && !BOOT_FAILURE_STATES.has(state))
    return { reason: null, cursor };
  const statusHash = await sha256Hex(instance.status_msg ?? '');
  const delta = await bootLogDelta(cursor, log);
  const outcome = orderedBootFailure(delta.fresh, id, policy);
  const recoveredStatus =
    cursor?.healthyStatusHash !== statusHash &&
    !fatalSignature(instance, '', id, policy) &&
    policy.recoverySignatures.some((s) =>
      (instance.status_msg ?? '').toLowerCase().includes(s.toLowerCase()),
    );
  if ((outcome.healthy && !outcome.reason) || recoveredStatus) {
    // Recovery establishes a fresh baseline, never authority from earlier boot history.
    const recoveryCursor = typeof log === 'string' ? delta.cursor : {};
    recoveryCursor.healthyStatusHash = statusHash;
    return { reason: null, cursor: recoveryCursor };
  }
  const historicalStatus = cursor?.healthyStatusHash === statusHash;
  const nextCursor = { ...delta.cursor };
  delete nextCursor.healthyStatusHash;
  if (historicalStatus) nextCursor.healthyStatusHash = statusHash;
  const direct = historicalStatus ? null : fatalSignature(instance, '', id, policy);
  if (direct)
    return {
      reason: direct,
      source: 'status',
      state,
      statusHash,
      witness: bootState(state) + ':status:' + statusHash,
      cursor: nextCursor,
    };
  let reason = outcome.reason;
  let witness = bootState(state) + ':boot_log:' + statusHash;
  // No new output plus the same failing state/message preserves the original witness.
  // New progress, changed message or an unreadable log cancels it. Pending-state flaps do not.
  if (
    !reason &&
    delta.stable &&
    !delta.fresh.trim() &&
    prior?.source === 'boot_log' &&
    bootState(prior.state) === bootState(state) &&
    prior.statusHash === statusHash
  ) {
    reason = prior.reason;
    witness = prior.witness;
  } else if (outcome.progress && reason) witness += ':' + delta.cursor.hash;
  else if (
    prior?.source === 'boot_log' &&
    prior.reason === reason &&
    bootState(prior.state) === bootState(state) &&
    prior.statusHash === statusHash
  )
    witness = prior.witness;
  return {
    reason,
    source: 'boot_log',
    state,
    statusHash,
    witness,
    cursor: nextCursor,
    historicalStatus,
  };
}

export function observeFatal(prior, reason, at, policy, witness = reason) {
  if (!reason) return null;
  if (!prior || prior.reason !== reason || prior.witness !== witness || at < prior.observedAt)
    return { reason, witness, first: at, last: at, observedAt: at, count: 1, confirmed: false };
  if (at === prior.observedAt) return prior;
  const separated = at - prior.last >= policy.observationSpacingSeconds * 1000;
  const next = {
    ...prior,
    observedAt: at,
    ...(separated ? { last: at, count: Math.min(2, prior.count + 1) } : {}),
  };
  next.confirmed = next.count >= 2 && at - next.first >= policy.confirmationSeconds * 1000;
  return next;
}
export function faultPenalty(history, at, policy) {
  if (!history) return 0;
  if (!Array.isArray(history.faults)) return policy.penaltyCapUsdHour;
  let sum = 0;
  for (const f of history.faults) {
    if (!Number.isFinite(f.at) || !Number.isFinite(f.weight) || f.weight < 0)
      return policy.penaltyCapUsdHour;
    sum +=
      policy.penaltyBaseUsdHour *
      f.weight *
      2 ** (-Math.max(0, at - f.at) / (policy.penaltyHalfLifeHours * 3600000));
  }
  return Math.min(policy.penaltyCapUsdHour, sum);
}
export function retryEligible(record, at, policy) {
  return (
    at >= (record.originalStarted ?? record.started) &&
    at >= record.ended &&
    record.firstReportAt === undefined &&
    record.verdict === 'PROVIDER_FAULT' &&
    record.closed === 'PROVIDER_FAULT' &&
    Number.isFinite(record.ended) &&
    !record.replacementExhausted &&
    (record.providerRetries ?? 0) + 1 < policy.maxAttempts &&
    at <
      (record.sessionDeadline ??
        (record.originalStarted ?? record.started) + policy.sessionMinutes * 60000) &&
    at < record.softDeadline
  );
}
export function attemptReceipt(record, at = record.ended) {
  return {
    instance: record.id,
    machine: record.machineId,
    host: record.hostId,
    reason: record.faultReason ?? record.closed ?? 'UNKNOWN',
    seconds: Math.max(0, Math.floor((at - record.started) / 1000)),
  };
}

export const rankingPrice = (offer) => offer.pricing.effective + (offer.penalty ?? 0);

export function providerLossVerdict(record) {
  return record.firstReportAt === undefined ? 'PROVIDER_FAULT' : 'FAIL';
}
