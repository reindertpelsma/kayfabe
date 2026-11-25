// Checks that the Worker bundle pin names the bundle the source produces. Unlike test/bundle.test.js
// it never skips: a missing wrangler is a failure, so an install without the dev dependencies cannot
// turn the check into a silent pass.
//   node ci/vast-broker/scripts/check-bundle-pin.mjs [--broker <directory>]
// The pin is deployment.json `bundle_sha256` and wrangler.toml `DEPLOYED_BUNDLE_SHA256`; the bundle
// is the SHA-256 of index.js from `wrangler deploy --config wrangler.toml --dry-run --outdir <dir>`
// (no network, no credentials). It exits 0 when both pins equal two identical dry-run bundles.
// `node_modules` must be a real directory inside the checked tree: a symlinked one changes the hash.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tomlVar = (toml, key) => new RegExp('^' + key + ' = "([^"]*)"$', 'm').exec(toml)?.[1];

function emit(broker, wrangler) {
  const out = mkdtempSync(join(tmpdir(), 'kf-pin-check-'));
  try {
    const run = spawnSync(
      wrangler,
      ['deploy', '--config', 'wrangler.toml', '--dry-run', '--outdir', out],
      {
        cwd: broker,
        encoding: 'utf8',
        timeout: 120000,
        maxBuffer: 16 * 1024 * 1024,
        env: { ...process.env, WRANGLER_SEND_METRICS: 'false' },
      },
    );
    if (run.status !== 0) return { error: 'wrangler dry run failed: ' + (run.stderr || run.error) };
    return {
      digest: createHash('sha256')
        .update(readFileSync(join(out, 'index.js')))
        .digest('hex'),
    };
  } catch (e) {
    return { error: 'no bundle emitted: ' + e.message };
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
}

// Returns the list of problems; empty means the pin is the bundle.
export function check(broker) {
  const wrangler = join(broker, 'node_modules/.bin/wrangler');
  if (!existsSync(wrangler))
    return ['wrangler is not installed in ' + broker + ' (npm ci --ignore-scripts, no --omit=dev)'];
  const manifest = JSON.parse(readFileSync(join(broker, 'deployment.json'), 'utf8'));
  const toml = readFileSync(join(broker, 'wrangler.toml'), 'utf8');
  const pin = manifest.bundle_sha256;
  const problems = [];
  if (!/^[0-9a-f]{64}$/.test(pin)) problems.push('deployment.json bundle_sha256 is not a SHA-256');
  if (tomlVar(toml, 'DEPLOYED_BUNDLE_SHA256') !== pin)
    problems.push('deployment.json and wrangler.toml DEPLOYED_BUNDLE_SHA256 differ');
  const a = emit(broker, wrangler);
  const b = emit(broker, wrangler);
  if (a.error || b.error) return [...problems, a.error || b.error];
  if (a.digest !== b.digest) problems.push('two dry runs differ: ' + a.digest + ' and ' + b.digest);
  else if (a.digest !== pin)
    problems.push(
      'the source bundles to ' +
        a.digest +
        ' but the pin is ' +
        pin +
        ': regenerate the pin ' +
        '(DEPLOY.md, "Bundle pin") in deployment.json and wrangler.toml',
    );
  return problems;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  let broker = fileURLToPath(new URL('..', import.meta.url));
  if (args[0] === '--broker' && args[1]) broker = resolve(args[1]);
  else if (args.length) {
    console.error('usage: check-bundle-pin.mjs [--broker <directory>]');
    process.exit(2);
  }
  const problems = check(broker);
  for (const p of problems) console.error('bundle pin: ' + p);
  if (!problems.length) console.log('bundle pin: the source bundles to the pinned SHA-256');
  process.exit(problems.length ? 1 : 0);
}
