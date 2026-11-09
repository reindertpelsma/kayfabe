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
export function observeFatal(prior, reason, at, policy) {
  if (!reason) return null;
  if (!prior || prior.reason !== reason || at < prior.last)
    return { reason, first: at, last: at, count: 1, confirmed: false };
  if (at === prior.last) return prior;
  const next = { ...prior, last: at, count: prior.count + 1 };
  next.confirmed = next.count >= 2 || at - next.first >= policy.confirmationSeconds * 1000;
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
