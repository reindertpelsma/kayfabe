#!/usr/bin/env node
// Release (large tier) plan, offline: which dies, machines and legs a release WOULD rent from a
// given offer list, and its worst-case cost per family, by the Worker's own code (configuration,
// offer rules, src/release.js). It rents nothing and needs no credential (wtier, 2026-11-05).
//
//   node ci/vast-broker/release-plan.mjs --market <offers.json> [--var NAME=VALUE ...] [--credit N] [--rehearsal DIE]
//   node ci/vast-broker/release-plan.mjs --changed-files <list.txt>
//   node ci/vast-broker/release-plan.mjs --app-lane --market <offers.json> --daily-cap USD (--first-release | --rotation <table.json>)
//        [--spent-today USD] [--images a,b] [--families f1,f2] [--var NAME=VALUE ...]
//
// --changed-files: one path per line (git diff --name-only --no-renames master...HEAD); prints whether the app lane is
// required (core_paths of app-lane.json, ruling section AZ item 5) and the matching paths. --app-lane: the app-lane plan
// (src/app-lane.js, docs/design/V3_APP_LANE_PLANNER.md): every image on every family (--first-release) or the rotation of
// the given table; exits 2 when the plan is refused (a missing offer, or an estimate over the daily cap). DESIGN-ONLY: the
// Worker admin start kind 'app' is not built; this prints a plan and rents nothing.
//
// --rehearsal DIE plans the named one-die rehearsal scope (admin start {tier:'large', die, rehearsal:true}); it never
// clears a matrix blocker and its plan says the driver sub-run matrix is NOT executed.
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
import { planRelease, planRehearsal } from './src/release.js';
import { APP_LANE, appLaneRequired, planAppLane } from './src/app-lane.js';

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
export function releaseConfig(vars = {}) {
  // Placeholders for the identity variables the configuration requires; nothing is contacted.
  const env = {
    OWNER_GITHUB_ID: '1',
    REPOSITORY_GITHUB_ID: '1',
    REPOSITORY: 'reindertpelsma/kayfabe',
    WORKER_URL: 'https://plan.invalid',
    ...vars,
  };
  return applyProfile(configuration(env), 'release');
}
// The app-lane plan (src/app-lane.js) over the same offer evaluation as a release plan; see the header.
export async function offlineAppPlan(market, vars = {}, opts = {}) {
  const cfg = releaseConfig(vars);
  const evaluation = await new OfferBook(Date.now).evaluate(
    null,
    cfg,
    new Ledger(new EmptyStorage()),
    'release',
    { market },
  );
  return planAppLane(evaluation.allowed, cfg, opts);
}
export async function offlinePlan(market, vars = {}, { credit, rehearsal } = {}) {
  const cfg = releaseConfig(vars);
  const ledger = new Ledger(new EmptyStorage());
  const evaluation = await new OfferBook(Date.now).evaluate(null, cfg, ledger, 'release', {
    market,
  });
  if (rehearsal !== undefined)
    return planRehearsal(evaluation.allowed, cfg, {
      budget: await ledger.budgetSummary(cfg, Date.now()),
      ...(credit === undefined ? {} : { credit }),
      die: rehearsal,
    });
  return planRelease(evaluation.allowed, cfg, {
    budget: await ledger.budgetSummary(cfg, Date.now()),
    ...(credit === undefined ? {} : { credit }),
  });
}
export function appTable(plan) {
  const lines = [
    'app-lane plan (offline; rents nothing; DESIGN-ONLY, the Worker start kind app is not built) mode=' +
      plan.mode +
      ' release_seq=' +
      plan.release_seq +
      ' legs=' +
      plan.legs.length +
      ' parallel=' +
      plan.parallel +
      ' estimate_usd=' +
      plan.estimate_usd +
      ' daily_cap_usd=' +
      plan.daily_cap_usd +
      ' admitted=' +
      plan.admitted +
      (plan.refusals.length ? ' refusals=' + plan.refusals.join(',') : ''),
  ];
  for (const l of plan.legs)
    lines.push(
      '  ' +
        l.image +
        ' on ' +
        l.family +
        ' die=' +
        l.die +
        ' machine=' +
        l.machine_id +
        ' ' +
        l.gpu_name +
        ' dph=' +
        l.dph_total +
        ' down_gb=' +
        l.bandwidth_down_gb +
        ' est_usd=' +
        l.est_usd,
    );
  for (const u of plan.unplaced) lines.push('  ' + u.image + ' on ' + u.family + ' ' + u.reason);
  return lines.join('\n');
}
export function table(plan) {
  const lines = [
    'release plan (offline; rents nothing)' +
      (plan.rehearsal
        ? ' REHEARSAL ' + plan.rehearsal_die + ' (driver sub-run matrix NOT executed)'
        : '') +
      ': legs=' +
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
  if (plan.cpu_vendor)
    lines.push(
      '  cpu_vendor required=' +
        JSON.stringify(plan.cpu_vendor.required) +
        ' planned=' +
        JSON.stringify(plan.cpu_vendor.planned) +
        ' swaps=' +
        plan.cpu_vendor.swaps.length +
        (plan.cpu_vendor.missing.length ? ' MISSING=' + plan.cpu_vendor.missing.join(',') : ''),
    );
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
            ' tier=' +
            (l.leg_tier ?? 'large') +
            ' cpu=' +
            l.cpu_vendor +
            (l.pin_cpu_vendor ? '(pinned)' : '') +
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
  let market, credit, rehearsal, changed, rotation;
  let appLane = false;
  const app = {};
  const vars = {};
  const need = (i) => {
    if (i >= args.length) throw Error('missing value for ' + args[i - 1]);
    return args[i];
  };
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--market') {
      const doc = JSON.parse(readFileSync(need(++i), 'utf8'));
      market = Array.isArray(doc) ? doc : doc.offers;
    } else if (args[i] === '--var') {
      const [name, ...value] = need(++i).split('=');
      vars[name] = value.join('=');
    } else if (args[i] === '--credit') credit = Number(need(++i));
    else if (args[i] === '--rehearsal') rehearsal = need(++i);
    else if (args[i] === '--changed-files')
      changed = readFileSync(need(++i), 'utf8')
        .split('\n')
        .filter((line) => line !== '');
    else if (args[i] === '--app-lane') appLane = true;
    else if (args[i] === '--first-release') app.firstRelease = true;
    else if (args[i] === '--rotation') app.rotation = JSON.parse(readFileSync(need(++i), 'utf8'));
    else if (args[i] === '--daily-cap') app.dailyCapUsd = Number(need(++i));
    else if (args[i] === '--spent-today') app.spentTodayUsd = Number(need(++i));
    else if (args[i] === '--images') app.images = need(++i).split(',');
    else if (args[i] === '--families') app.families = need(++i).split(',');
    else
      throw Error(
        'usage: release-plan.mjs --market <offers.json> [--var N=V] [--credit N] [--rehearsal DIE] | --changed-files <list> | --app-lane --market <offers.json> --daily-cap USD (--first-release | --rotation <table.json>)',
      );
  }
  if (changed !== undefined) {
    const r = appLaneRequired(changed, APP_LANE.core_paths);
    console.log('app lane ' + (r.required ? 'REQUIRED' : 'not required'));
    for (const f of r.matches) console.log('  core: ' + f);
    console.log(JSON.stringify(r, null, 1));
  } else {
    if (!Array.isArray(market)) throw Error('--market <offers.json> is required');
    if (appLane) {
      const plan = await offlineAppPlan(market, vars, app);
      console.log(appTable(plan));
      console.log(JSON.stringify(plan, null, 1));
      if (!plan.admitted) process.exitCode = 2;
    } else {
      const plan = await offlinePlan(market, vars, { credit, rehearsal });
      console.log(table(plan));
      console.log(JSON.stringify(plan, null, 1));
    }
  }
}
