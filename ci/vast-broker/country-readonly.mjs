// Local diagnostic only. A billed key comes from the environment; no create call exists.
import { configuration } from './src/policy.js';
import { Vast } from './src/vast.js';
import { OfferBook, offerCountry } from './src/offers.js';
const cfg = configuration({
  OWNER_GITHUB_ID: '229738317',
  REPOSITORY_GITHUB_ID: '1311106387',
  REPOSITORY: 'reindertpelsma/kayfabe',
  WORKER_URL: 'https://readonly.invalid',
  PREFERRED_COUNTRIES: process.env.PREFERRED_COUNTRIES ?? '["US","CA"]',
});
let calls = 0,
  rows = [];
const readOnly = async (url, init) => {
  const u = new URL(url);
  if (
    u.origin !== 'https://console.vast.ai' ||
    u.pathname !== '/api/v0/bundles/' ||
    init.method !== 'POST'
  )
    throw Error('READ_ONLY_BOUNDARY');
  const q = JSON.parse(init.body);
  if ('geolocation' in q || 'country' in q) throw Error('COUNTRY_HARD_FILTER');
  calls++;
  const r = await fetch(url, init);
  const value = await r.clone().json();
  rows = value.offers ?? [];
  return r;
};
if (!process.env.VAST_API_KEY) throw Error('Missing local key');
const vast = new Vast(readOnly, cfg, process.env.VAST_API_KEY),
  book = new OfferBook(Date.now),
  ledger = { storage: { list: async () => new Map() }, get: async () => undefined };
const profiles = [];
for (const profile of ['check', 'sweep']) {
  const evaluation = await book.evaluate(vast, cfg, ledger, profile);
  let selected = null;
  try {
    const r = await book.select(evaluation, cfg, ledger, profile);
    selected = {
      die: r.offer.die,
      country: r.offer.country,
      effective_hourly: r.offer.pricing.effective,
    };
  } catch (e) {
    if (e.reason !== 'NO_ALLOWED_OFFERS') throw e;
  }
  profiles.push({
    profile,
    selected,
    allowed: evaluation.allowed.length,
    verified_counts: evaluation.view.verified_counts,
    rejected: evaluation.view.rejected,
  });
}
const counts = {};
for (const row of rows) {
  const c = offerCountry(row);
  counts[c] = (counts[c] ?? 0) + 1;
}
console.log(
  JSON.stringify(
    {
      run: process.env.RUN_ID ?? 'v5-country-readonly-20261023',
      provider_searches: calls,
      provider_writes: 0,
      field: 'geolocation',
      shape: 'region, uppercase country suffix',
      country_counts: counts,
      preferred: cfg.preferredCountries,
      profiles,
    },
    null,
    2,
  ),
);
