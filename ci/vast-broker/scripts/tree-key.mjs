// Prints the kf-tree-v2 key of a commit, computed with the Worker's own matcher (src/treekey.js)
// from `git ls-tree` and the commit's own .workerignore. Used by scripts/ci/ignore_ladder.py
// keycheck, which compares it with the Python matcher's key (the strip step's matcher).
// Usage: node tree-key.mjs [REV]. Exit 0 and the key on stdout; exit 2 when the ignore file is
// missing or outside the subset (the Worker would produce no key either). PROTECTED is reported on
// stderr and not applied: the ladder keeps /ci/vast-broker ignored on purpose (see .workerignore).
import { execFileSync } from 'node:child_process';
import { guarded, parseIgnore, treeKeyOf } from '../src/treekey.js';

const rev = process.argv[2] ?? 'HEAD';
const git = (...args) => execFileSync('git', args, { maxBuffer: 1 << 28 });
const patterns = parseIgnore(git('show', rev + ':.workerignore').toString('utf8'));
if (patterns === null) {
  console.error('the commit .workerignore is outside the treekey.js subset');
  process.exit(2);
}
if (guarded(patterns) === null) console.error('PROTECTED refuses this ignore set (no Worker key)');
const entries = [];
for (const record of git('ls-tree', '-r', '-z', '--full-tree', rev).toString('utf8').split('\0')) {
  if (!record) continue;
  const tab = record.indexOf('\t');
  const [mode, , sha] = record.slice(0, tab).split(' ');
  entries.push({ path: record.slice(tab + 1), mode, sha });
}
const key = await treeKeyOf(entries, patterns);
if (key === null) {
  console.error('no key');
  process.exit(2);
}
console.log(key);
