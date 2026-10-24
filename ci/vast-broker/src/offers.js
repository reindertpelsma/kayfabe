// prettier-ignore
import {DEFAULTS} from './defaults.js';
// prettier-ignore
import {gpuClass,offerPrice,offerRejections,providerId,refuse} from './policy.js';
// Denied-country spellings: ISO alpha-2 and alpha-3 codes and the names Vast or a geolocation
// database may print (review V13: a long form must never pass as UNKNOWN). Matching ignores
// case, apostrophes, parentheses and extra spaces.
const normal = (text) =>
  text
    .toUpperCase()
    .replace(/['\u2018\u2019`]/g, '')
    .replace(/[()]/g, ' ')
    .replace(/\s*,\s*/g, ', ')
    .replace(/\s+/g, ' ')
    .trim();
const DENIED_NAMES = new Map(
  Object.entries({
    CU: ['CU', 'CUB', 'Cuba', 'Republic of Cuba'],
    IR: [
      'IR',
      'IRN',
      'Iran',
      'Islamic Republic of Iran',
      'Iran, Islamic Republic of',
      'Iran (Islamic Republic of)',
    ],
    KP: [
      'KP',
      'PRK',
      'North Korea',
      'DPRK',
      'DPR Korea',
      'Korea DPR',
      'Korea, North',
      "Democratic People's Republic of Korea",
      "Korea, Democratic People's Republic of",
      "Korea (Democratic People's Republic of)",
    ],
    SY: ['SY', 'SYR', 'Syria', 'Syrian Arab Republic'],
    // Owner ruling 2026-10-24: broken or very slow CDN/Ubuntu-mirror access there.
    CN: [
      'CN',
      'CHN',
      'China',
      'PRC',
      "People's Republic of China",
      "China, People's Republic of",
      'Mainland China',
    ],
    HK: [
      'HK',
      'HKG',
      'Hong Kong',
      'Hong Kong SAR',
      'Hong Kong SAR, China',
      'Hong Kong SAR China',
      'Hong Kong, China',
      'Hong Kong Special Administrative Region',
    ],
    MO: [
      'MO',
      'MAC',
      'Macau',
      'Macao',
      'Macau SAR',
      'Macao SAR',
      'Macau SAR, China',
      'Macao SAR, China',
      'Macao SAR China',
      'Macau, China',
      'Macao, China',
      'Macau Special Administrative Region',
      'Macao Special Administrative Region',
    ],
  }).flatMap(([code, names]) => names.map((name) => [normal(name), code])),
);
// Vast currently returns geolocation as "region, ISO country code". Region text is discarded;
// a denied name matches on any comma-separated suffix, longest first. Missing/odd metadata
// ranks last and never removes an offer.
export function offerCountry(offer) {
  if (typeof offer.geolocation !== 'string' || offer.geolocation.length > 256 || !offer.geolocation)
    return 'UNKNOWN';
  const parts = offer.geolocation.split(',');
  for (let i = 0; i < parts.length; i++) {
    const denied = DENIED_NAMES.get(normal(parts.slice(i).join(',')));
    if (denied) return denied;
  }
  const country = parts.at(-1).trim();
  return parts.length > 1 && /^[A-Z]{2}$/.test(country) ? country : 'UNKNOWN';
}
function countryRank(offer, cfg) {
  const index = cfg.preferredCountries.indexOf(offer.country);
  return index < 0 ? cfg.preferredCountries.length : index;
}
// One account/DO, one bounded market read per minute, shared across profiles.
export class OfferBook {
  constructor(now) {
    this.now = now;
    this.cached = null;
    this.at = null;
    this.failed = false;
  }
  async evaluate(vast, cfg, ledger, profile) {
    let hit = this.at !== null && this.now() - this.at < DEFAULTS.availabilityCacheMs;
    if (!hit) {
      this.at = this.now();
      this.cached = null;
      this.failed = true;
      this.cached = await vast.offers();
      this.failed = false;
    }
    if (this.failed || !this.cached) refuse();
    const bad = [...(await ledger.storage.list({ prefix: 'bad-machine:', limit: 257 }))];
    if (bad.length > 256) refuse();
    const badMachines = bad.map(([k]) => Number(k.slice('bad-machine:'.length)));
    const reserved = (await ledger.openRecords())
      .map((r) => r.machineId)
      .filter((id) => Number.isSafeInteger(id));
    const samples = new Map(),
      verified = new Map(),
      rejected = {},
      rejectedOffers = [],
      allowed = [];
    for (const offer of this.cached) {
      const die = gpuClass(offer, cfg);
      if (!die) continue;
      const row = verified.get(die) ?? { die, total: 0, verified: 0 };
      row.total++;
      const trusted =
        offer.verified === true ||
        (offer.verified === undefined && offer.verification === 'verified');
      if (trusted) row.verified++;
      verified.set(die, row);
      if (
        trusted &&
        typeof offer.dlperf === 'number' &&
        Number.isFinite(offer.dlperf) &&
        offer.dlperf > 0
      ) {
        const x = samples.get(die) ?? [];
        x.push(offer.dlperf);
        samples.set(die, x);
      }
    }
    for (const offer of this.cached) {
      const die = gpuClass(offer, cfg),
        values = (samples.get(die) ?? []).sort((a, b) => a - b),
        mid = Math.floor(values.length / 2),
        median = values.length
          ? values.length % 2
            ? values[mid]
            : (values[mid - 1] + values[mid]) / 2
          : null;
      const reasons = offerRejections(offer, cfg, { profile, median, badMachines, reserved });
      const country = offerCountry(offer);
      if (
        cfg.deniedCountries.includes(country) ||
        (country === 'UNKNOWN' && !cfg.allowUnknownCountry)
      )
        reasons.push('COUNTRY');
      for (const reason of reasons) rejected[reason] = (rejected[reason] ?? 0) + 1;
      if (reasons.length)
        rejectedOffers.push({
          machine_id: providerId(offer.machine_id),
          host_id: providerId(offer.host_id),
          reasons,
        });
      if (!reasons.length)
        allowed.push({
          ...offer,
          die,
          country: offerCountry(offer),
          pricing: offerPrice(offer, cfg),
        });
    }
    const grouped = new Map();
    for (const offer of allowed) {
      const prior = grouped.get(offer.die);
      if (!prior)
        grouped.set(offer.die, {
          die: offer.die,
          gpu_name: offer.gpu_name.toUpperCase().replaceAll(' ', '_'),
          compute_cap: offer.compute_cap,
          count: 1,
          effective_hourly: offer.pricing.effective,
          estimated_total: offer.pricing.total,
          kvm: true,
          // The row's representative: the cheapest allowed offer of this die.
          machine_id: providerId(offer.machine_id),
          host_id: providerId(offer.host_id),
        });
      else {
        prior.count++;
        if (offer.pricing.effective < prior.effective_hourly) {
          prior.gpu_name = offer.gpu_name.toUpperCase().replaceAll(' ', '_');
          prior.compute_cap = offer.compute_cap;
          prior.effective_hourly = offer.pricing.effective;
          prior.estimated_total = offer.pricing.total;
          prior.machine_id = providerId(offer.machine_id);
          prior.host_id = providerId(offer.host_id);
        }
      }
    }
    const view = {
      profile,
      offers: [...grouped.values()].sort((a, b) => a.die.localeCompare(b.die)),
      thresholds: {
        ...cfg.quality[profile],
        price: profile === 'check' ? cfg.price : cfg.sweepPrice,
        bandwidth_down_gb: cfg.bandwidthDown,
        bandwidth_up_gb: cfg.bandwidthUp,
        verified: 1,
      },
      rejected,
      rejected_offers: rejectedOffers,
      verified_counts: [...verified.values()].sort((a, b) => a.die.localeCompare(b.die)),
      cache: { hit, age_ms: this.now() - this.at },
    };
    return { allowed, view };
  }
  async select(evaluation, cfg, ledger, profile, preferences) {
    const set = profile === 'check' ? cfg.checkDies : cfg.sweepDies,
      rotate =
        profile === 'sweep' &&
        (preferences === undefined || (preferences.length === 1 && preferences[0] === 'rotate'));
    let ignored = 0,
      order;
    if (rotate) {
      const previous = await ledger.get('rotation:lastDie'),
        at = set.indexOf(previous);
      order = [...set.slice(at + 1), ...set.slice(0, at + 1)];
      if (cfg.skipPassedDays > 0) {
        const filtered = [];
        for (const die of order) {
          const passed = await ledger.get('passed:' + die);
          if (typeof passed !== 'number' || this.now() - passed >= cfg.skipPassedDays * 86400000)
            filtered.push(die);
        }
        order = filtered;
      }
    } else if (preferences !== undefined) {
      order = preferences.filter((x) => set.includes(x));
      ignored = preferences.length - order.length;
      if (!order.length) refuse('UNAVAILABLE', 503, 'NO_ALLOWED_PREFERENCE');
    } else order = set;
    const candidates = evaluation.allowed
      .filter((x) => order.includes(x.die))
      .sort(
        (a, b) =>
          order.indexOf(a.die) - order.indexOf(b.die) ||
          countryRank(a, cfg) - countryRank(b, cfg) ||
          a.pricing.effective - b.pricing.effective ||
          a.id - b.id,
      );
    if (!candidates.length) refuse('UNAVAILABLE', 503, 'NO_ALLOWED_OFFERS');
    return {
      offer: candidates[0],
      ignored,
      reason: rotate ? 'ROTATE' : preferences === undefined ? 'DEFAULT_ORDER' : 'PREFERENCE',
    };
  }
}
