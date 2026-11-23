// Mutation check for scripts/check-bundle-pin.mjs: the unmutated copy must pass and every mutant,
// a one-byte change to what the bundle is built from or to the pin, must make the check fail.
//   node ci/vast-broker/scripts/bundle-pin-mutants.mjs [name ...]
// The mutants are applied to a throwaway copy of ci/vast-broker with its real node_modules (a
// symlinked node_modules changes the bundle hash), never to the checkout.
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { check } from './check-bundle-pin.mjs';

const source = fileURLToPath(new URL('..', import.meta.url));
const pin = JSON.parse(readFileSync(join(source, 'deployment.json'), 'utf8')).bundle_sha256;
const other = pin.slice(0, -1) + (pin.endsWith('0') ? '1' : '0');
// [name, file, before, after]
const MUTANTS = [
  [
    'one byte of src/index.js',
    'src/index.js',
    "state: 'RENTING_DISABLED',",
    "state: 'RENTING_DISABLEd',",
  ],
  ['one byte of defaults.json', 'defaults.json', '"dailyCount": 4,', '"dailyCount": 5,'],
  ['pin changed in deployment.json only', 'deployment.json', pin, other],
  [
    'pin changed in wrangler.toml only',
    'wrangler.toml',
    'BUNDLE_SHA256 = "' + pin,
    'BUNDLE_SHA256 = "' + other,
  ],
];

const root = mkdtempSync(join(tmpdir(), 'kf-pin-mutants-'));
process.on('exit', () => rmSync(root, { recursive: true, force: true }));
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => process.exit(130));
const copy = join(root, 'broker');
cpSync(source, copy, { recursive: true, filter: (p) => !p.includes('/.wrangler') });

const wanted = process.argv.slice(2);
let failed = 0;
const baseline = check(copy);
if (baseline.length) {
  console.error('baseline: the unmutated copy fails the check: ' + baseline.join('; '));
  process.exit(1);
}
console.log('baseline: pass');
for (const [name, file, before, after] of MUTANTS) {
  if (wanted.length && !wanted.includes(name)) continue;
  const path = join(copy, file);
  const text = readFileSync(path, 'utf8');
  if (text.split(before).length !== 2) {
    console.error('mutant "' + name + '": anchor not found exactly once in ' + file);
    failed++;
    continue;
  }
  writeFileSync(
    path,
    text.replace(before, () => after),
  );
  const problems = check(copy);
  writeFileSync(path, text);
  if (problems.length) console.log('killed: ' + name);
  else {
    console.error('SURVIVED: ' + name);
    failed++;
  }
}
process.exit(failed ? 1 : 0);
