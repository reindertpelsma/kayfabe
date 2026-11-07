import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import {
  parseRequirements,
  evaluate,
  familyOf,
  normalTier,
  RequirementsError,
  INFLIGHT_MAX_MS,
} from '../src/requirements.js';
// ci/requirements.toml: a strict TOML subset, evaluated per tier.
// FALSIFIER: a file outside the subset parses; the committed file reads differently in TOML;
// min_dies/min_passes/platform coverage/tier rank are not enforced; an invalid row counts.

const FILE = fileURLToPath(new URL('../../requirements.toml', import.meta.url));
const text = readFileSync(FILE, 'utf8');

test('requirements: the committed file parses and reads the same in Python tomllib', () => {
  const parsed = parseRequirements(text);
  assert.equal(parsed.version, 1);
  assert.deepEqual([...new Set(parsed.blocks.map((b) => b.tier))].sort(), [
    'largest',
    'middle',
    'small',
  ]);
  const py = spawnSync(
    'python3',
    [
      '-c',
      'import json,sys,tomllib; print(json.dumps(tomllib.load(open(sys.argv[1],"rb"))))',
      FILE,
    ],
    { encoding: 'utf8' },
  );
  if (py.status !== 0) return; // no python3/tomllib: the JS parse above still ran
  const toml = JSON.parse(py.stdout);
  assert.equal(toml.version, 1);
  assert.equal(toml.require.length, parsed.blocks.length);
  toml.require.forEach((raw, i) => {
    const b = parsed.blocks[i];
    assert.equal(normalTier(raw.tier), b.tier);
    assert.equal(raw.family, b.family);
    assert.deepEqual(raw.dies, b.dies === 'any' ? ['any'] : b.dies);
    assert.deepEqual(raw.platforms, b.platforms);
    assert.deepEqual(raw.stages ?? [], b.stages);
    assert.equal(raw.min_dies ?? 1, b.min_dies);
    assert.equal(raw.min_passes ?? 1, b.min_passes);
    assert.equal(raw.min_windows_runs ?? null, b.min_windows_runs);
  });
});

test('requirements: anything outside the subset is refused', () => {
  const head = 'version = 1\n[[require]]\ntier = "middle"\nfamily = "any"\ndies = ["any"]\n';
  const ok = head + 'platforms = ["linux"]\n';
  assert.equal(parseRequirements(ok).blocks.length, 1);
  for (const bad of [
    '',
    'version = 2\n',
    'version = 1\nversion = 1\n',
    '[require]\n',
    ok + 'extra = "x"\n',
    head + 'platforms = ["mac"]\n',
    head + 'platforms = ["linux", "linux"]\n',
    head + 'platforms = ["linux"]\nplatforms = ["linux"]\n',
    head + "platforms = ['linux']\n",
    head + 'platforms = ["lin\\ux"]\n',
    head + 'platforms = ["linux"]\nstages = ["Bad"]\n',
    head + 'platforms = ["linux"]\nmin_dies = 0\n',
    head + 'platforms = ["linux"]\nmin_passes = 9\n',
    head.replace('"any"]', '"GA102"]') + 'platforms = ["linux"]\nmin_dies = 2\n',
    head.replace('family = "any"', 'family = "ada"').replace('"any"]', '"GA102"]') +
      'platforms = ["linux"]\n',
    head.replace('"middle"', '"huge"') + 'platforms = ["linux"]\n',
    head + 'platforms = ["linux"]\r\n',
    'version = 1\n[[require]]\n',
    'version = 1\nkey value\n',
  ])
    assert.throws(() => parseRequirements(bad), RequirementsError, JSON.stringify(bad));
  assert.throws(() => parseRequirements('version = 1\n' + '[[require]]\n'.repeat(70)));
  assert.throws(() => parseRequirements('#'.repeat(70000)));
  // Comments, trailing commas and 'large' are accepted.
  const c = parseRequirements(
    '# c\nversion = 1 # c\n[[require]] # c\ntier = "large"\nfamily = "ada"\n' +
      'dies = ["AD104", "AD102",]\nplatforms = ["windows"]\n',
  );
  assert.equal(c.blocks[0].tier, 'largest');
  assert.deepEqual(c.blocks[0].dies, ['AD104', 'AD102']);
});

test('requirements: families of dies and compute classes', () => {
  assert.equal(familyOf('TU106'), 'turing');
  assert.equal(familyOf('GA102'), 'ampere');
  assert.equal(familyOf('AD104'), 'ada');
  assert.equal(familyOf('GB202'), 'blackwell');
  assert.equal(familyOf('GH100'), 'hopper');
  assert.equal(familyOf('CC_89'), 'ada');
  assert.equal(familyOf('CC_120'), 'blackwell');
  assert.equal(familyOf('CC_61'), 'unknown');
  assert.equal(familyOf('XX123'), 'unknown');
  assert.equal(familyOf('bad'), 'unknown');
});

const NOW = Date.UTC(2026, 10, 5, 12);
let n = 0;
const r = (patch) => {
  const row = {
    run_id: 'run-' + String(++n).padStart(4, '0'),
    tier: 'largest',
    family: 'ada',
    die: 'AD104',
    platform: 'both',
    verdict: 'PASS',
    source: 'reproduced',
    started_at: '2026-11-05T10:00:00Z',
    sort_at: '2026-11-05T11:00:00Z',
    valid: true,
    stages: [
      { name: 'gates', result: 'PASS' },
      { name: 'windows_gate', result: 'PASS' },
    ],
    ...patch,
  };
  // The tagged end time orders answers; sort_at is ignored by evaluate.
  row.ended_at = patch.ended_at ?? (row.verdict === null ? null : row.sort_at);
  return row;
};
const blocks = parseRequirements(text).blocks;
const large = (rows) =>
  evaluate(
    blocks.filter((b) => b.family === 'ada' || b.tier !== 'largest'),
    'largest',
    rows,
    NOW,
  );

test('requirements: the release tier needs 2 dies per family with 2 passes each', () => {
  const two = (die) => [r({ die }), r({ die, sort_at: '2026-11-05T11:30:00Z' })];
  assert.equal(large([...two('AD104')]).met, false, 'one die');
  assert.equal(large([...two('AD104'), r({ die: 'AD102' })]).missing[0].reason, 'TOO_FEW_PASSES');
  assert.equal(large([...two('AD104'), ...two('AD102')]).met, true);
  // Platforms: a linux-only die does not count for Windows.
  const linux = [r({ die: 'AD102', platform: 'linux' }), r({ die: 'AD102', platform: 'linux' })];
  assert.equal(large([...two('AD104'), ...linux]).met, false);
  // An invalid (untagged) row or a community row never counts.
  assert.equal(
    large([...two('AD104'), ...two('AD102').map((x) => ({ ...x, valid: false }))]).met,
    false,
  );
  assert.equal(
    large([...two('AD104'), ...two('AD102').map((x) => ({ ...x, source: 'reported' }))]).met,
    false,
  );
  // A newest FAIL answers: the die is not OK even with older passes.
  const failed = [
    ...two('AD102'),
    r({ die: 'AD102', verdict: 'FAIL', sort_at: '2026-11-05T12:00:00Z' }),
  ];
  const res = large([...two('AD104'), ...failed]);
  assert.equal(res.met, false);
  assert.equal(res.missing[0].reason, 'FAIL');
  assert.equal(res.missing[0].need, 2);
  assert.equal(res.missing[0].have, 1);
  // min_passes counts the trailing PASS streak: PASS, FAIL, PASS (newest last) is a streak of 1.
  const broken = [
    r({ die: 'AD102', sort_at: '2026-11-05T11:00:00Z' }),
    r({ die: 'AD102', verdict: 'FAIL', sort_at: '2026-11-05T11:10:00Z' }),
    r({ die: 'AD102', sort_at: '2026-11-05T11:20:00Z' }),
  ];
  assert.equal(large([...two('AD104'), ...broken]).missing[0].reason, 'TOO_FEW_PASSES');
  // ...and a newer PASS after the FAIL makes a streak of 2 (re-run, never bypass).
  const rerun = [...broken, r({ die: 'AD102', sort_at: '2026-11-05T11:30:00Z' })];
  assert.equal(large([...two('AD104'), ...rerun]).met, true);
  // Ordering is by the tagged ended_at, not sort_at.
  const moved = [
    r({
      die: 'AD102',
      verdict: 'FAIL',
      ended_at: '2026-11-05T12:00:00Z',
      sort_at: '2026-11-05T09:00:00Z',
    }),
    ...two('AD102'),
  ];
  assert.equal(large([...two('AD104'), ...moved]).missing[0].reason, 'FAIL');
  // A tampered row is never skipped: an older valid PASS cannot answer past it.
  const garbled = [
    ...two('AD102'),
    r({ die: 'AD102', verdict: 'FAIL', valid: false, sort_at: '2026-11-05T12:00:00Z' }),
  ];
  const tampered = large([...two('AD104'), ...two('AD103'), ...garbled]);
  assert.equal(tampered.met, false);
  assert.deepEqual(
    tampered.missing.map((m) => m.reason),
    ['TAMPERED'],
  );
  // A middle-tier row does not satisfy the release tier.
  assert.equal(
    large([...two('AD104'), ...two('AD102').map((x) => ({ ...x, tier: 'middle' }))]).met,
    false,
  );
});

// Review of PR #269, R2 (2026-11-05). FALSIFIER: one fully passing release with one row per die
// (each box ran the Windows gate twice) is not met; one Windows run per die is met; a Linux-only
// streak or a FAIL in the streak counts Windows runs; min_windows_runs parses without windows.
test('requirements: largest counts Windows gate runs like the release verdict', () => {
  const twice = [
    { name: 'gates', result: 'PASS' },
    { name: 'windows_gate', result: 'PASS' },
    { name: 'windows_gate_2', result: 'PASS' },
  ];
  const all = blocks.filter((b) => b.tier === 'largest');
  assert.ok(all.length === 4 && all.every((b) => b.min_passes === 1 && b.min_windows_runs === 2));
  const DIES = { turing: ['TU106', 'TU116'], ampere: ['GA106', 'GA104'], ada: ['AD104', 'AD102'] };
  DIES.blackwell = ['GB205', 'GB206'];
  const release = Object.entries(DIES).flatMap(([family, dies]) =>
    dies.map((die) => r({ family, die, stages: twice })),
  );
  assert.deepEqual(evaluate(blocks, 'largest', release, NOW).missing, []);
  assert.equal(evaluate(blocks, 'largest', release, NOW).met, true, 'one row per die');
  // Mixed: one die on two machines (two rows, one run each), the other on one box run twice.
  assert.equal(
    large([r({ die: 'AD104' }), r({ die: 'AD104' }), r({ die: 'AD102', stages: twice })]).met,
    true,
  );
  // One Windows run on a die is not enough.
  const one = large([r({ die: 'AD104', stages: twice }), r({ die: 'AD102' })]);
  assert.deepEqual([one.met, one.missing[0].reason], [false, 'TOO_FEW_PASSES']);
  // A failed second run is not a pass: the row is FAIL (the box verdict) and answers FAIL.
  const failedSecond = [
    { name: 'gates', result: 'PASS' },
    { name: 'windows_gate', result: 'PASS' },
    { name: 'windows_gate_2', result: 'FAIL' },
  ];
  const second = large([
    r({ die: 'AD104', stages: twice }),
    r({ die: 'AD102', verdict: 'FAIL', stages: failedSecond }),
  ]);
  assert.deepEqual([second.met, second.missing[0].reason], [false, 'FAIL']);
  // A PASS row whose windows_gate_2 row did not pass contributes one run only.
  assert.equal(
    large([r({ die: 'AD104', stages: twice }), r({ die: 'AD102', stages: failedSecond })]).met,
    false,
  );
  // Runs before a FAIL do not count: PASS(x2), FAIL, PASS(x1) newest last is one run.
  const broken = [
    r({ die: 'AD102', stages: twice, sort_at: '2026-11-05T11:00:00Z' }),
    r({ die: 'AD102', verdict: 'FAIL', sort_at: '2026-11-05T11:10:00Z' }),
    r({ die: 'AD102', sort_at: '2026-11-05T11:20:00Z' }),
  ];
  assert.equal(large([r({ die: 'AD104', stages: twice }), ...broken]).met, false);
  // Linux is unaffected: the Linux cell needs one PASS row.
  const linuxOnly = parseRequirements(
    'version = 1\n[[require]]\ntier = "large"\nfamily = "ada"\ndies = ["AD104"]\n' +
      'platforms = ["linux"]\n',
  ).blocks;
  assert.equal(evaluate(linuxOnly, 'largest', [r({ die: 'AD104' })], NOW).met, true);
  // Parsing: only with Windows, 1..8.
  const block = (extra, platforms = '["linux", "windows"]') =>
    'version = 1\n[[require]]\ntier = "large"\nfamily = "ada"\ndies = ["any"]\n' +
    'platforms = ' +
    platforms +
    '\n' +
    extra +
    '\n';
  assert.equal(parseRequirements(block('min_windows_runs = 3')).blocks[0].min_windows_runs, 3);
  assert.equal(parseRequirements(block('')).blocks[0].min_windows_runs, null);
  for (const bad of [
    block('min_windows_runs = 2', '["linux"]'),
    block('min_windows_runs = 0'),
    block('min_windows_runs = 9'),
  ])
    assert.throws(() => parseRequirements(bad), RequirementsError);
});

test('requirements: explicit dies report each missing die and platform', () => {
  const b = parseRequirements(
    'version = 1\n[[require]]\ntier = "small"\nfamily = "ampere"\ndies = ["GA102", "GA106"]\n' +
      'platforms = ["linux", "windows"]\n',
  ).blocks;
  const rows = [
    r({ tier: 'small', family: 'ampere', die: 'GA102', platform: 'linux' }),
    r({
      tier: 'small',
      family: 'ampere',
      die: 'GA106',
      verdict: null,
      started_at: '2026-11-05T11:59:00Z',
    }),
  ];
  const res = evaluate(b, 'small', rows, NOW);
  assert.deepEqual(
    res.missing.map((m) => [m.die, m.platform, m.reason]),
    [
      ['GA102', 'windows', 'NO_RESULT'],
      ['GA106', 'linux', 'INFLIGHT'],
      ['GA106', 'windows', 'INFLIGHT'],
    ],
  );
  assert.deepEqual(res.inflight, [rows[1].run_id]);
  const stale = evaluate(
    b,
    'small',
    [{ ...rows[1], started_at: new Date(NOW - INFLIGHT_MAX_MS - 1).toISOString() }],
    NOW,
  );
  assert.deepEqual(stale.inflight, []);
  assert.equal(evaluate(b, 'middle', rows, NOW).missing[0].reason, 'NO_REQUIREMENTS');
});
