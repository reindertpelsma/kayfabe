// The Worker side of a leg's instruction (docs/design/V3_WORKER_PRIVATE_PRESIGN.md, "releaseSeq and the instruction"):
//   - where `releaseSeq` comes from, and
//   - the instruction a release leg hands to its box (built with src/propimages.js, priced with src/bandwidth.js).
// Pure apart from the two-method store the caller passes (get/put: the Worker's ledger). Nothing here runs unless the owner switches
// PROPIMAGES_ENABLED on, so no live leg changes.
import { buildLeg, canonicalInstruction } from './propimages.js';
import { estimateBandwidth, bandwidthEstimateInput } from './bandwidth.js';
import { middleAppsRequest } from './tier.js';

// releaseSeq is the ordinal of a RELEASE, and a retry of the same release must carry the same number (propimagePlan: one number, one
// image assignment). The Worker keeps one counter and one memo per tested sha in the ledger it already has:
//   release-seq:<sha>   { seq }   the number this commit's release got the first time it was started
//   release-seq-next    n         the number the next NEW sha gets
// An owner start may pass `release_seq` (the number of the release tag): it wins for a new sha and is recorded; for a sha that already
// has a number a different request is refused (RELEASE_SEQ_CONFLICT), so the same sha can never move an image. A sha that is started
// again (a new batch, a retry, a re-run) reads its memo. Consecutive new shas alternate parity, which rotates the images.
export async function releaseSeqFor(store, sha, requested) {
  if (!/^[0-9a-f]{40}$/.test(sha)) throw Error('release seq sha');
  if (requested !== undefined && !(Number.isSafeInteger(requested) && requested >= 0))
    throw Error('release seq request');
  const memo = await store.get('release-seq:' + sha);
  if (memo !== undefined && memo !== null) {
    if (!Number.isSafeInteger(memo.seq) || memo.seq < 0) throw Error('release seq memo');
    if (requested !== undefined && requested !== memo.seq) {
      const error = Error('release seq conflict');
      error.conflict = true;
      throw error;
    }
    return { seq: memo.seq, source: 'memo' };
  }
  let seq = requested;
  let source = 'request';
  if (seq === undefined) {
    seq = (await store.get('release-seq-next')) ?? 0;
    if (!Number.isSafeInteger(seq) || seq < 0) throw Error('release seq counter');
    source = 'counter';
  }
  await store.put('release-seq:' + sha, { seq });
  // the counter only moves forward: a request above it raises it, so a later counter number is never one already given
  const next = (await store.get('release-seq-next')) ?? 0;
  if (seq >= next) await store.put('release-seq-next', seq + 1);
  return { seq, source };
}

// How many distinct dies of the family this batch rents (1 means one box runs both images).
export const familyDiesOf = (batch, family) =>
  new Set(batch.legs.filter((l) => l.family === family && l.die).map((l) => l.die)).size;

// The instruction of one release leg and its estimate. `tierName` is the LEG's tier ('large' or 'middle'). The tier options are the
// ones src/index.js passes to tierEnvironment for the same leg, so the base fields are exactly what the box receives today.
export function legInstruction({ profile, tierName, leg, cfg, releaseSeq, familyDies }) {
  const options = {
    tierName,
    windowsRuns: leg.windowsRuns ?? 1,
    stages: leg.stages ?? [],
    releaseMatrix: leg.releaseMatrix,
    spec: leg.spec,
    ...(leg.rehearsal === true ? { rehearsal: true } : {}),
    ...middleAppsRequest(profile, tierName, cfg.middleAppsEnabled, leg.apps),
  };
  const { instruction, plan } = buildLeg(profile, {
    ...options,
    propimages: true,
    die: leg.die,
    releaseSeq,
    familyDies,
  });
  const estimate = estimateBandwidth(instruction, tierName);
  return {
    instruction,
    plan,
    text: canonicalInstruction(instruction),
    estimate,
    pricing: bandwidthEstimateInput(estimate),
  };
}
