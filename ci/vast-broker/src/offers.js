// prettier-ignore
import {DEFAULTS} from './defaults.js';
// prettier-ignore
import {gpuClass,offerPrice,offerRejections,refuse} from './policy.js';
// Vast currently returns geolocation as "region, ISO country code". Region text
// is discarded. Missing/odd metadata ranks last and never removes an offer.
export function offerCountry(offer) {
  if (typeof offer.geolocation !== 'string' || offer.geolocation.length > 256 || !offer.geolocation)
    return 'UNKNOWN';
  const country = offer.geolocation.split(',').at(-1).trim();
  const deniedNames = {
    CUBA: 'CU',
    IRAN: 'IR',
    'NORTH KOREA': 'KP',
    SYRIA: 'SY',
    // Owner ruling 2026-10-24: broken or very slow CDN/Ubuntu-mirror access there.
    CHINA: 'CN',
    'HONG KONG': 'HK',
    MACAU: 'MO',
    MACAO: 'MO',
    CU: 'CU',
    IR: 'IR',
    KP: 'KP',
    SY: 'SY',
    CN: 'CN',
    HK: 'HK',
    MO: 'MO',
  };
  if (deniedNames[country.toUpperCase()]) return deniedNames[country.toUpperCase()];
  return offer.geolocation.includes(',') && /^[A-Z]{2}$/.test(country) ? country : 'UNKNOWN';
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
    const samples = new Map(),
      verified = new Map(),
      rejected = {},
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
      const reasons = offerRejections(offer, cfg, { profile, median, badMachines });
      const country = offerCountry(offer);
      if (
        cfg.deniedCountries.includes(country) ||
        (country === 'UNKNOWN' && !cfg.allowUnknownCountry)
      )
        reasons.push('COUNTRY');
      for (const reason of reasons) rejected[reason] = (rejected[reason] ?? 0) + 1;
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
        });
      else {
        prior.count++;
        if (offer.pricing.effective < prior.effective_hourly) {
          prior.gpu_name = offer.gpu_name.toUpperCase().replaceAll(' ', '_');
          prior.compute_cap = offer.compute_cap;
          prior.effective_hourly = offer.pricing.effective;
          prior.estimated_total = offer.pricing.total;
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
