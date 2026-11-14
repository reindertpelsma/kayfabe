// Shared by the mutant scripts that used to edit src/ in place (evidence, report, store,
// provider-tolerance; owner review 2026-11-13): a killed run must never leave a mutated Worker
// source in the checkout or on a runner. tempCopy() builds a throwaway tree with the broker (and
// what its tests read) and returns the broker directory of the COPY, with a trailing slash. The
// copy is removed when the process exits.
import { cpSync, mkdirSync, mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const broker = fileURLToPath(new URL('.', import.meta.url));
const repo = fileURLToPath(new URL('../..', import.meta.url));
const keep = (p) => !p.includes('/node_modules') && !p.includes('/.wrangler');
export function tempCopy() {
  const root = mkdtempSync(join(tmpdir(), 'kf-mutants-'));
  const copy = join(root, 'ci/vast-broker');
  cpSync(broker, copy, { recursive: true, filter: keep });
  symlinkSync(join(broker, 'node_modules'), join(copy, 'node_modules'), 'dir');
  for (const path of [
    'scripts/ci',
    'ci/vast-reaper',
    'ci/platform-api/migrations',
    'ci/requirements.toml',
  ]) {
    mkdirSync(join(root, path, '..'), { recursive: true });
    cpSync(join(repo, path), join(root, path), { recursive: true, filter: keep });
  }
  const cleanup = () => rmSync(root, { recursive: true, force: true });
  process.on('exit', cleanup);
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => process.exit(130));
  return copy + '/';
}
