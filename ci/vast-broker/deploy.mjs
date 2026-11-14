// Owner deploy helper (no dependencies of its own): node ci/vast-broker/deploy.mjs <vars-file>
// [--dry-run]. Reads public Worker vars from a local KEY=VALUE file kept OUTSIDE the checkout,
// lets the lockfile-pinned wrangler bundle src/ (`deploy --dry-run --outdir`), refuses unless the
// emitted module's SHA-256 equals deployment.json and wrangler.toml, then uploads exactly that
// verified file (`--no-bundle`) with one --var per entry. It never prints a value: wrangler output
// is redacted and refusals name only the key or line number.
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync, realpathSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
// Regression 2026-11-11: a vars file with SHA_GAP_HOURS=0 deployed fine and then made every Worker
// call answer 503, because the Worker's own configuration() refuses it. The one import from src/
// is the Worker's own validator, so the two cannot drift. deploy.mjs itself is not in the bundle,
// so this import does not change the bundle hash.
import { configuration } from './src/policy.js';

const broker = process.env.KF_DEPLOY_ROOT ?? dirname(fileURLToPath(import.meta.url));
const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
// Installed by `npm ci --ignore-scripts` from package-lock.json.
const wrangler = process.env.WRANGLER ?? resolve(broker, 'node_modules/.bin/wrangler');
export const WRANGLER_VERSION = '4.152.0';
// Public, non-secret vars a deploy may set. Everything else is refused: secrets belong in
// `wrangler secret put`, and Cloudflare Access is dashboard-only (the Worker reads no ACCESS_*;
// the box pair BOX_ACCESS_ID/BOX_ACCESS_SECRET is a secret it only passes through).
// Boolean-valued public vars (exactly 'true' or 'false'); equal to src/policy.js BOOLEAN_VARS
// (test/round-deploy-helper.test.js compares them). Beyond that one list, the only src/ import is
// configuration() (see above), run on the vars file merged over wrangler.toml [vars].
export const BOOLEAN_VARS = Object.freeze([
  'RENTING_ENABLED',
  'RELEASE_ENABLED',
  'ALLOW_UNKNOWN_COUNTRY',
  'TREE_KEY_CARRY_OVER',
  'MIDDLE_APPS_ENABLED',
]);
export const ALLOWED = new Set([
  'WORKER_URL',
  'OWNER_GITHUB_ID',
  'RENTING_ENABLED',
  'DENIED_COUNTRIES',
  'ALLOW_UNKNOWN_COUNTRY',
  'PREFERRED_COUNTRIES',
  'MAX_INET_USD_PER_GB',
  'MAX_PRICE_USD_HOUR',
  'DAILY_SPEND_USD',
  'ROLLING_SPEND_USD',
  'DAILY_RENTALS',
  'CREDIT_RESERVE_USD',
  'BLOB_REPO',
  'BLOB_ALLOWED_TAG_PREFIXES',
  // Private R2 benchmark copy (public ids; the key pair is a secret).
  'R2_ACCOUNT_ID',
  'R2_BUCKET',
  // Owner-edited lists and retention (owner ruling 2026-10-24): edit the var and redeploy.
  'BAD_MACHINE_IDS',
  'BAD_MACHINE_IDS_EXTRA',
  'BAD_HOST_IDS',
  'LOG_RETENTION_HOURS',
  // Same-SHA gap in hours (1-720, default 24): a required gate must be re-runnable after an infra flake.
  'SHA_GAP_HOURS',
  // Concurrent rentals (owner ruling 2026-10-24 item 8): account cap 1-4, per-profile lower caps.
  'MAX_CONCURRENT_RENTALS',
  'MAX_CONCURRENT_CHECK',
  'MAX_CONCURRENT_SWEEP',
  // Result store (src/results.js): 'true' lets a Worker-verified tree key answer for another
  // commit. Keep it unset/'false' until boxes delete ignored files after cloning (review of #258).
  'TREE_KEY_CARRY_OVER',
  'MIDDLE_APPS_ENABLED',
  // Custom hosts (docs/design/V3_CUSTOM_HOSTS.md): the 40-hex commit of scripts/kfhost that the admin
  // enrol route pins in its bootstrap command. No default: the route refuses while it is unset.
  'HOST_DAEMON_COMMIT',
  // The public Cloudflare Access client id printed in the enrol command (the secret is never known).
  'HOST_ACCESS_ID',
  // Tiers (wtier, 2026-11-05; src/tier.js, src/policy.js): the release (large) profile's explicit
  // caps and lists, and the middle tier's rotation and VRAM floor. Account cap is now 1-8.
  'MAX_CONCURRENT_RELEASE',
  'RELEASE_ENABLED',
  'ACCOUNT_DAILY_USD',
  'RELEASE_MAX_PRICE_USD_HOUR',
  'RELEASE_DAILY_USD',
  'RELEASE_DAILY_RENTALS',
  'RELEASE_COST_CAP_USD',
  'RELEASE_HARD_RUNTIME_MINUTES',
  'RELEASE_RERUNS_PER_LEG',
  'RELEASE_STAGES',
  'RELEASE_DIES',
  'MIDDLE_ROTATION',
  'MIDDLE_VRAM_FLOOR_MB',
  'PINNED_MIDDLE_DIES',
  'PROVIDER_CONFIRM_SECONDS',
  'PROVIDER_OBSERVATION_SPACING_SECONDS',
  'PROVIDER_MAX_ATTEMPTS',
  'PROVIDER_SESSION_MINUTES',
  'PROVIDER_PENALTY_BASE_USD_HOUR',
  'PROVIDER_PENALTY_HALF_LIFE_HOURS',
  'PROVIDER_PENALTY_CAP_USD_HOUR',
  'PROVIDER_FATAL_SIGNATURES',
  'PROVIDER_IGNORABLE_SIGNATURES',
]);
const bareOrigin = (value) => {
  try {
    const u = new URL(value);
    return u.protocol === 'https:' && value === u.origin;
  } catch {
    return false;
  }
};
// The files wrangler 4.152.0 writes next to the bundle in a dry run (README.md, index.js.map).
const EMITTED = new Set(['index.js', 'index.js.map', 'README.md']);
class Refusal extends Error {}
const fail = (message) => {
  throw new Refusal(message);
};

export function parseVars(text) {
  const vars = new Map();
  text.split('\n').forEach((raw, index) => {
    const line = raw.replace(/\r$/, '');
    if (!line.trim() || line.trimStart().startsWith('#')) return;
    const match = /^([A-Z][A-Z0-9_]{0,63})=([\x21-\x7e]{1,4096})$/.exec(line);
    if (!match) fail('line ' + (index + 1) + ' is not KEY=VALUE (no spaces, printable ASCII)');
    const [, key, value] = match;
    if (vars.has(key)) fail('duplicate key ' + key);
    if (/^ACCESS_/.test(key))
      fail(key + ' is retired: Cloudflare Access is configured in the dashboard only');
    if (/^BOX_ACCESS_/.test(key)) fail(key + ' is a secret: use wrangler secret put');
    if (!ALLOWED.has(key))
      fail(key + ' is not an allowed public var (secrets: wrangler secret put)');
    vars.set(key, value);
  });
  if (!/^[1-9][0-9]{0,19}$/.test(vars.get('OWNER_GITHUB_ID') ?? ''))
    fail('OWNER_GITHUB_ID must be present and numeric');
  // Review of PR #269, R3: every boolean-valued var is exactly true or false (the Worker refuses
  // any other value as CONFIG_VAR:<name>; src/policy.js BOOLEAN_VARS lists the same names).
  for (const key of BOOLEAN_VARS)
    if (vars.has(key) && !['true', 'false'].includes(vars.get(key)))
      fail(key + ' must be true or false');
  if (vars.has('HOST_DAEMON_COMMIT') && !/^[0-9a-f]{40}$/.test(vars.get('HOST_DAEMON_COMMIT')))
    fail('HOST_DAEMON_COMMIT must be a 40-hex commit sha');
  if (vars.has('WORKER_URL') && !bareOrigin(vars.get('WORKER_URL')))
    fail('WORKER_URL must be the bare https origin (no trailing slash, path, query or user)');
  return vars;
}

// wrangler.toml [vars] as a plain map (double- or single-quoted values).
export function tomlVars(toml) {
  const section = (toml.split('[vars]')[1] ?? '').split(/^\[/m)[0];
  const env = {};
  for (const m of section.matchAll(/^([A-Z][A-Z0-9_]*) = (?:"([^"]*)"|'([^']*)')$/gm))
    env[m[1]] = m[2] ?? m[3];
  return env;
}

// Runs the Worker's own configuration() on toml vars overlaid with the vars file; a refusal names
// the file var(s) whose removal makes it pass (values are never printed).
export function validateWithWorker(vars, toml) {
  const base = tomlVars(toml);
  const check = (skip) => {
    const env = { ...base };
    for (const [key, value] of vars) if (key !== skip) env[key] = value;
    try {
      configuration(env);
      return true;
    } catch {
      return false;
    }
  };
  if (check(null)) return;
  const culprits = [...vars.keys()].filter((key) => check(key));
  fail(
    'the Worker would refuse this configuration at runtime (every call would answer 503)' +
      (culprits.length ? ': check ' + culprits.join(', ') : ''),
  );
}

function tomlVar(toml, key) {
  return new RegExp('^' + key + ' = "([^"]*)"$', 'm').exec(toml.split('[vars]')[1] ?? '')?.[1];
}

function main(argv) {
  const args = argv.slice(2);
  const dryRun = args.includes('--dry-run');
  const files = args.filter((x) => x !== '--dry-run');
  if (files.length !== 1 || args.length > 2) fail('usage: deploy.mjs <vars-file> [--dry-run]');
  let path;
  try {
    path = realpathSync(files[0]);
    if (!statSync(path).isFile()) throw Error();
  } catch {
    fail('vars file is not a readable regular file');
  }
  if ((path + sep).startsWith(realpathSync(checkout) + sep))
    fail('keep the vars file outside the repository checkout');
  const vars = parseVars(readFileSync(path, 'utf8'));

  const toml = readFileSync(resolve(broker, 'wrangler.toml'), 'utf8');
  if (tomlVar(toml, 'OWNER_GITHUB_ID') !== vars.get('OWNER_GITHUB_ID'))
    fail('OWNER_GITHUB_ID differs from wrangler.toml');
  if (!bareOrigin(tomlVar(toml, 'WORKER_URL')))
    fail('wrangler.toml WORKER_URL must be the bare https origin');
  if (vars.has('WORKER_URL') && tomlVar(toml, 'WORKER_URL') !== vars.get('WORKER_URL'))
    fail('WORKER_URL differs from wrangler.toml');
  validateWithWorker(vars, toml);
  const manifest = JSON.parse(readFileSync(resolve(broker, 'deployment.json'), 'utf8'));
  if (manifest.bundle_sha256 !== tomlVar(toml, 'DEPLOYED_BUNDLE_SHA256'))
    fail('deployment.json and wrangler.toml DEPLOYED_BUNDLE_SHA256 differ');

  const version = spawnSync(wrangler, ['--version'], { encoding: 'utf8', timeout: 60000 });
  if (version.status !== 0 || !/(?:^|[^0-9.])4\.152\.0(?:$|[^0-9.])/m.test(String(version.stdout)))
    fail(
      'wrangler ' + WRANGLER_VERSION + ' not found at the pinned path (npm ci --ignore-scripts)',
    );

  // Telemetry off. The bundling dry run needs no Cloudflare credentials.
  const env = { ...process.env, WRANGLER_SEND_METRICS: 'false' };
  const wrangle = (args) =>
    spawnSync(wrangler, ['deploy', '--config', 'wrangler.toml', ...args], {
      cwd: broker,
      env,
      encoding: 'utf8',
      maxBuffer: 16 * 1024 * 1024,
    });
  const out = mkdtempSync(join(tmpdir(), 'kf-vast-bundle-'));
  try {
    const build = wrangle(['--dry-run', '--outdir', out]);
    if (build.status !== 0) fail('wrangler could not bundle src/ (dry run failed)');
    let bundle;
    try {
      bundle = readFileSync(join(out, 'index.js'));
    } catch {
      fail('wrangler emitted no index.js');
    }
    const digest = createHash('sha256').update(bundle).digest('hex');
    if (digest !== manifest.bundle_sha256)
      fail('wrangler bundle ' + digest + ' does not match deployment.json');
    // Review F8 (2026-11-13): the hash covers index.js only, and the upload has module discovery on,
    // so any other module wrangler finds beside it would be uploaded unhashed. The dry run's
    // directory may hold wrangler's own three files and nothing else.
    const extra = readdirSync(out).filter((n) => !EMITTED.has(n));
    if (extra.length)
      fail('wrangler emitted files beyond the hashed bundle (' + extra.length + ')');

    const redact = (text) => {
      let result = String(text ?? '');
      for (const [key, value] of vars) result = result.split(value).join('<' + key + '>');
      return result;
    };
    // The Worker reports DEPLOYED_BUNDLE_SHA256 from a var. It is set here from the bytes that are
    // uploaded, not read from wrangler.toml, so the self-report is the hash this helper verified
    // (and the file is re-read once more below, just before the upload).
    const upload = [...vars, ['DEPLOYED_BUNDLE_SHA256', digest]];
    process.stdout.write(
      'deploy: bundle ' +
        digest +
        ' (' +
        bundle.length +
        ' bytes) verified; vars ' +
        [...vars.keys()].join(',') +
        ' (values withheld)' +
        (dryRun ? '; dry run' : '') +
        '\n',
    );
    // A secret and a var cannot have the same name. Only the coordinator runs this migration.
    process.stdout.write(
      'deploy: before first public-var deploy, coordinator must run: ' +
        'wrangler secret delete WORKER_URL\n',
    );
    if (
      createHash('sha256')
        .update(readFileSync(join(out, 'index.js')))
        .digest('hex') !== digest
    )
      fail('the verified bundle changed before the upload');
    // Upload exactly the verified bytes; wrangler does not bundle a second time.
    // --x-provision=false (hidden flag, default true in the pinned wrangler 4.152.0): without it
    // wrangler CREATES any bucket or database that a binding names and that does not exist, with
    // no location hint and no lifecycle rule (the evidence bucket must be created first, DEPLOY.md).
    const run = wrangle([
      ...(dryRun ? ['--dry-run'] : []),
      '--x-provision=false',
      '--no-bundle',
      join(out, 'index.js'),
      ...upload.flatMap(([key, value]) => ['--var', key + ':' + value]),
    ]);
    process.stdout.write(redact(run.stdout));
    process.stderr.write(redact(run.stderr));
    process.exitCode = run.status ?? 1;
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv);
  } catch (error) {
    if (!(error instanceof Refusal)) throw error;
    process.stderr.write('deploy: refused: ' + error.message + '\n');
    process.exitCode = 2;
  }
}
