// Run explicitly from the broker directory, with no other test/build running.
// Each mutant is restored before the next; only real workerd subtest kills count.
import { readFile, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';

const path = new URL('../src/ledger.js', import.meta.url);
const original = await readFile(path, 'utf8');
const mutants = [
  ['account slots', 'open.length >= cfg.maxConcurrent ||', 'false ||'],
  ['profile slots', "cfg.maxConcurrentProfile[record.profile ?? 'check']", 'Infinity'],
  ['daily count', 'if (count >= cfg.dailyCount)', 'if (false)'],
  [
    'daily spend',
    "if (daily + cost > (cfg.profile === 'sweep' ? cfg.sweepDaily : cfg.daily))",
    'if (false)',
  ],
  ['rolling spend', 'if (monthly + cost > cfg.rolling)', 'if (false)'],
  ['credit reserve', 'if (credit < cfg.reserve + cost + outstanding)', 'if (false)'],
  ['run attempt', "if (await ledger.get('cost:' + record.key))", 'if (false)'],
  ['SHA gap', 'if (cached && (!force || now - cached.started < cfg.gap))', 'if (false)'],
  ['older branch', 'if (branch.length)', 'if (false)'],
  [
    'newer branch',
    'if (branch.some((o) => numericId(o.run) && BigInt(o.run) > BigInt(record.run)))',
    'if (false)',
  ],
  ['bad-machine cap', 'bad.size > 256 ||', 'false ||'],
  ['bad-machine entry', "bad.has('bad-machine:' + record.machineId)", 'false'],
  ['configured bad machine', 'cfg.badMachines.includes(record.machineId)', 'false'],
  [
    'reserved machine',
    'open.some((o) => Number.isSafeInteger(o.machineId) && o.machineId === record.machineId)',
    'false',
  ],
  [
    'transaction removed',
    'await this.storage.transaction(async (storage) => {',
    'await (async (action) => action(this.storage))(async (storage) => {',
  ],
];
let failed = false;
const selected = process.argv.slice(2);
try {
  for (const [name, before, after] of mutants) {
    if (selected.length && !selected.includes(name)) continue;
    if (original.split(before).length !== 2) throw Error('ambiguous mutation: ' + name);
    await writeFile(path, original.replace(before, after));
    const run = spawnSync(process.execPath, ['--test', 'workerd/admission.test.js'], {
      encoding: 'utf8',
      timeout: 90000,
      maxBuffer: 4 * 1024 * 1024,
      env: { ...process.env, WRANGLER_SEND_METRICS: 'false' },
    });
    const kills = [...(run.stdout ?? '').matchAll(/^    not ok \d+ - (.+)$/gm)].map(
      (match) => match[1],
    );
    const ok = run.status === 1 && kills.length >= 2 && !run.error;
    failed ||= !ok;
    console.log(JSON.stringify({ mutant: name, status: run.status, kills, ok }));
    await writeFile(path, original);
  }
} finally {
  await writeFile(path, original);
}
if (failed) process.exitCode = 1;
