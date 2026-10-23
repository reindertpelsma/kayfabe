// Owner deploy helper (no dependencies of its own): node ci/vast-broker/deploy.mjs <vars-file>
// [--dry-run]. Reads public Worker vars from a local KEY=VALUE file kept OUTSIDE the checkout,
// lets the lockfile-pinned wrangler bundle src/ (`deploy --dry-run --outdir`), refuses unless the
// emitted module's SHA-256 equals deployment.json and wrangler.toml, then uploads exactly that
// verified file (`--no-bundle`) with one --var per entry. It never prints a value: wrangler output
// is redacted and refusals name only the key or line number.
import { mkdtempSync, readFileSync, rmSync, statSync, realpathSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const broker = process.env.KF_DEPLOY_ROOT ?? dirname(fileURLToPath(import.meta.url));
const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
// Installed by `npm ci --ignore-scripts` from package-lock.json.
const wrangler = process.env.WRANGLER ?? resolve(broker, 'node_modules/.bin/wrangler');
export const WRANGLER_VERSION = '4.152.0';
// Public, non-secret vars a deploy may set. Everything else is refused: secrets belong in
// `wrangler secret put`, and Cloudflare Access is dashboard-only (the Worker reads no ACCESS_*;
// the box pair BOX_ACCESS_ID/BOX_ACCESS_SECRET is a secret it only passes through).
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
  // Owner-edited lists and retention (owner ruling 2026-10-24): edit the var and redeploy.
  'BAD_MACHINE_IDS',
  'BAD_HOST_IDS',
  'LOG_RETENTION_HOURS',
]);
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

    const redact = (text) => {
      let result = String(text ?? '');
      for (const [key, value] of vars) result = result.split(value).join('<' + key + '>');
      return result;
    };
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
    // Upload exactly the verified bytes; wrangler does not bundle a second time.
    const run = wrangle([
      ...(dryRun ? ['--dry-run'] : []),
      '--no-bundle',
      join(out, 'index.js'),
      ...[...vars].flatMap(([key, value]) => ['--var', key + ':' + value]),
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
