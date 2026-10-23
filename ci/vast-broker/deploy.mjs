// Owner deploy helper (zero dependencies): node ci/vast-broker/deploy.mjs <vars-file> [--dry-run]
// Reads public Worker vars from a local KEY=VALUE file kept OUTSIDE the checkout, verifies the
// reviewed bundle hash, then runs the pinned wrangler with one --var per entry. It never prints
// a value: wrangler output is redacted and refusals name only the key or line number.
import { readFileSync, statSync, realpathSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { dirname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const broker = process.env.KF_DEPLOY_ROOT ?? dirname(fileURLToPath(import.meta.url));
const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const wrangler = process.env.WRANGLER ?? '/tmp/kf-vast-wrangler/node_modules/.bin/wrangler';
export const WRANGLER_VERSION = '4.152.0';
// Public, non-secret vars a deploy may set. Everything else is refused: secrets belong in
// `wrangler secret put`, and Cloudflare Access is dashboard-only (the Worker reads no ACCESS_*).
export const ALLOWED = new Set([
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
]);
const fail = (message) => {
  process.stderr.write('deploy: refused: ' + message + '\n');
  process.exit(2);
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
    if (/^(?:BOX_)?ACCESS_/.test(key))
      fail(key + ' is retired: Cloudflare Access is configured in the dashboard only');
    if (!ALLOWED.has(key))
      fail(key + ' is not an allowed public var (secrets: wrangler secret put)');
    vars.set(key, value);
  });
  if (!/^[1-9][0-9]{0,19}$/.test(vars.get('OWNER_GITHUB_ID') ?? ''))
    fail('OWNER_GITHUB_ID must be present and numeric');
  if (vars.has('RENTING_ENABLED') && !['true', 'false'].includes(vars.get('RENTING_ENABLED')))
    fail('RENTING_ENABLED must be true or false');
  return vars;
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
  let bundle;
  try {
    bundle = readFileSync(resolve(broker, 'dist/worker.js'));
  } catch {
    fail('dist/worker.js missing: run node bundle.js first');
  }
  const digest = createHash('sha256').update(bundle).digest('hex');
  const manifest = JSON.parse(readFileSync(resolve(broker, 'deployment.json'), 'utf8'));
  if (digest !== manifest.bundle_sha256) fail('dist/worker.js does not match deployment.json');
  if (digest !== tomlVar(toml, 'DEPLOYED_BUNDLE_SHA256'))
    fail('dist/worker.js does not match wrangler.toml DEPLOYED_BUNDLE_SHA256');

  const version = spawnSync(wrangler, ['--version'], { encoding: 'utf8', timeout: 60000 });
  if (version.status !== 0 || !/(?:^|[^0-9.])4\.152\.0(?:$|[^0-9.])/m.test(String(version.stdout)))
    fail('wrangler ' + WRANGLER_VERSION + ' not found at the pinned path');

  const redact = (text) => {
    let out = String(text ?? '');
    for (const [key, value] of vars) out = out.split(value).join('<' + key + '>');
    return out;
  };
  process.stdout.write(
    'deploy: bundle ' +
      digest +
      ' verified; vars ' +
      [...vars.keys()].join(',') +
      ' (values withheld)' +
      (dryRun ? '; dry run' : '') +
      '\n',
  );
  const run = spawnSync(
    wrangler,
    [
      'deploy',
      ...(dryRun ? ['--dry-run'] : []),
      ...[...vars].flatMap(([key, value]) => ['--var', key + ':' + value]),
    ],
    { cwd: broker, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 },
  );
  process.stdout.write(redact(run.stdout));
  process.stderr.write(redact(run.stderr));
  process.exit(run.status ?? 1);
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url))
  main(process.argv);
