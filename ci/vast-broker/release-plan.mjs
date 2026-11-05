#!/usr/bin/env node
// Release (large tier) plan, offline: which dies, machines and legs a release WOULD rent from a
// given offer list, and its worst-case cost per family, by the Worker's own code (configuration,
// offer rules, src/release.js). It rents nothing and needs no credential (wtier, 2026-11-05).
//
//   node ci/vast-broker/release-plan.mjs --market <offers.json> [--var NAME=VALUE ...] [--credit N]
//
// <offers.json>: a Vast bundles answer ({"offers": [...]}) or a plain array, for example
// test/fixtures/release-market-synthetic.json. --var sets a Worker variable as the deployed vars
// file would (MAX_CONCURRENT_RENTALS=8, RELEASE_COST_CAP_USD=30, ...). The live plan of the
// deployed Worker is the admin page's Plan button (POST /api/plan): the same function on the live
// market, with the ledger's real spend and the account credit.
import { readFileSync } from 'node:fs';
import { configuration } from './src/policy.js';
import { applyProfile } from './src/tier.js';
import { OfferBook } from './src/offers.js';
import { Ledger } from './src/ledger.js';
import { planRelease } from './src/release.js';

class EmptyStorage {
  data = new Map();
  async get(key) {
    return this.data.get(key);
  }
  async put(key, value) {
    this.data.set(key, value);
  }
  async delete(key) {
    return this.data.delete(key);
  }
  async list({ prefix = '' } = {}) {
    return new Map([...this.data].filter(([k]) => k.startsWith(prefix)));
  }
  async transaction(action) {
    return action(this);
  }
}
export async function offlinePlan(market, vars = {}, { credit } = {}) {
  // Placeholders for the identity variables the configuration requires; nothing is contacted.
  const env = {
    OWNER_GITHUB_ID: '1',
    REPOSITORY_GITHUB_ID: '1',
    REPOSITORY: 'reindertpelsma/kayfabe',
    WORKER_URL: 'https://plan.invalid',
    ...vars,
  };
  const cfg = applyProfile(configuration(env), 'release');
  const ledger = new Ledger(new EmptyStorage());
  const evaluation = await new OfferBook(Date.now).evaluate(null, cfg, ledger, 'release', {
    market,
  });
  return planRelease(evaluation.allowed, cfg, {
    budget: await ledger.budgetSummary(cfg, Date.now()),
    ...(credit === undefined ? {} : { credit }),
  });
}
function table(plan) {
  const lines = [
    'release plan (offline; rents nothing): legs=' +
      plan.legs +
      ' parallel=' +
      plan.parallel +
      ' estimate_usd=' +
      plan.estimate_usd +
      ' cost_cap_usd=' +
      plan.cost_cap_usd +
      ' admitted=' +
      plan.admitted +
      (plan.refusals.length ? ' refusals=' + plan.refusals.join(',') : ''),
  ];
  for (const f of plan.families) {
    lines.push('  ' + f.family + ' est_usd=' + f.est_usd + (f.complete ? '' : ' INCOMPLETE'));
    for (const c of f.cells)
      for (const l of c.legs)
        lines.push(
          '    ' +
            c.die +
            ' machine=' +
            l.machine_id +
            ' host=' +
            l.host_id +
            ' ' +
            l.gpu_name +
            ' dph=' +
            l.dph_total +
            ' windows_runs=' +
            l.windows_runs +
            ' est_usd=' +
            l.est_usd,
        );
    for (const m of f.missing) lines.push('    ' + m.die + ' ' + m.reason);
  }
  return lines.join('\n');
}
if (import.meta.url === 'file://' + process.argv[1]) {
  const args = process.argv.slice(2);
  let market, credit;
  const vars = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--market') {
      const doc = JSON.parse(readFileSync(args[++i], 'utf8'));
      market = Array.isArray(doc) ? doc : doc.offers;
    } else if (args[i] === '--var') {
      const [name, ...value] = args[++i].split('=');
      vars[name] = value.join('=');
    } else if (args[i] === '--credit') credit = Number(args[++i]);
    else throw Error('usage: release-plan.mjs --market <offers.json> [--var N=V] [--credit N]');
  }
  if (!Array.isArray(market)) throw Error('--market <offers.json> is required');
  const plan = await offlinePlan(market, vars, { credit });
  console.log(table(plan));
  console.log(JSON.stringify(plan, null, 1));
}
