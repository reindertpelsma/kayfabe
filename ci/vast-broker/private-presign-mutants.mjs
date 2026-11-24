// Mutation check for the Worker's private-object presign, the lease-time trust scope, the estimate as the price and releaseSeq
// (src/privatedata.js, src/trust.js, src/tokens.js, src/policy.js, src/legs.js, src/publish.js, the data files), GPU-free. Each mutant is
// applied in a temporary COPY of the tree (mutant-copy.mjs), the test files below run, and the copy is restored before the next
// mutant. Every mutant must make at least TWO named tests fail. Run with no other test running here:
//   node ci/vast-broker/private-presign-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tempCopy } from './mutant-copy.mjs';

const here = tempCopy();
const TESTS = [
  'private-presign',
  'trust',
  'bandwidth-pricing',
  'release-seq',
  'run-token',
  'run-report-extra',
].map((name) => here + 'test/' + name + '.test.js');
const P = 'src/privatedata.js',
  T = 'src/trust.js',
  K = 'src/tokens.js',
  Y = 'src/policy.js',
  L = 'src/legs.js',
  U = 'src/publish.js',
  X = 'src/index.js',
  PO = 'private-objects.json',
  TJ = 'trust.json',
  D = 'defaults.json';
// [name, [[file, before, after], ...]]  or  [name, [file, { change(json) }]] for an edit of a JSON data file
const MUTANTS = [
  // ---- the private route
  [
    'scope check removed',
    [[P, 'if (!hasPrivateScope(fields.scopes) || !hasPrivateScope(r.scopes))', 'if (false)']],
  ],
  ['token scope ignored', [[P, '!hasPrivateScope(fields.scopes) || ', '']]],
  ['record scope ignored', [[P, ' || !hasPrivateScope(r.scopes)', '']]],
  ['tier check removed', [[P, 'if (!allowed) refuse(', 'if (false) refuse(']]],
  [
    'profile rule matches every profile',
    [[P, '(a.profile === undefined || budgetClass(r) === a.profile) &&', 'true &&']],
  ],
  [
    'per-run limit removed',
    [[P, 'if ((r.privateMints ?? 0) >= value.urls_per_run) refuse(', 'if (false) refuse(']],
  ],
  ['mint not counted', [[P, 'r.privateMints = used + 1;', 'r.privateMints = used;']]],
  [
    'lifetime cap removed',
    [
      [
        P,
        'const seconds = Math.min(value.max_ttl_seconds, Math.floor((deadlineMs - nowMs) / 1000));',
        'const seconds = Math.floor((deadlineMs - nowMs) / 1000);',
      ],
    ],
  ],
  [
    'run deadline ignored',
    [
      [
        P,
        'const seconds = Math.min(value.max_ttl_seconds, Math.floor((deadlineMs - nowMs) / 1000));',
        'const seconds = value.max_ttl_seconds;',
      ],
    ],
  ],
  [
    'prefix check removed',
    [[P, 'if (!value.prefixes.some((p) => p.prefix === prefix))', 'if (false)']],
  ],
  [
    'dot segment accepted',
    [
      [
        P,
        "if (parts.some((part) => part === '.' || part === '..' || /^\\.+$/.test(part)))",
        'if (false)',
      ],
    ],
  ],
  ['existence not checked', [[P, 'if (response.status === 200) return true;', 'return true;']]],
  [
    'expired run served',
    [[P, 'if (r.ended !== null || r.closed || nowMs >= r.reportExpiry * 1000)', 'if (false)']],
  ],
  [
    'a closed run named invalid',
    [
      [
        X,
        "refuse('UNAUTHORIZED', 401, named('TOKEN_EXPIRED'));",
        "refuse('UNAUTHORIZED', 401, named('TOKEN_INVALID'));",
      ],
    ],
  ],
  [
    'expired token reported as invalid',
    [
      [
        K,
        'if (!Number.isSafeInteger(exp) || this.now() < exp * 1000) return false;',
        'return false;',
      ],
    ],
  ],
  [
    'url logged on a mint',
    [
      [
        X,
        '            object: objectKey,\n            expires: Math.floor(at / 1000) + expires,',
        '            object: objectKey,\n            url: location,\n            expires: Math.floor(at / 1000) + expires,',
      ],
    ],
  ],
  // ---- the trust scope
  [
    'fork head accepted',
    [[T, 'if (provenance.headRepository !== repository) return no(', 'if (false) return no(']],
  ],
  [
    'repository not compared',
    [[T, 'if (provenance.repository !== repository) return no(', 'if (false) return no(']],
  ],
  [
    'event not checked',
    [
      [
        T,
        'if (!p.events[provenance.source].includes(provenance.event)) return no(',
        'if (false) return no(',
      ],
    ],
  ],
  [
    'actor not checked',
    [
      [
        T,
        'if (p.require_owner_actor && provenance.actorIsOwner !== true) return no(',
        'if (false) return no(',
      ],
    ],
  ],
  [
    'unreachable admin start trusted',
    [
      [
        T,
        'if (p.admin_requires_reachable && provenance.reachable !== true) return no(',
        'if (false) return no(',
      ],
    ],
  ],
  [
    'collaborator rule trusts every push',
    [[T, 'if (pr && p.collaborator_pr_heads)', 'if (p.collaborator_pr_heads)']],
  ],
  [
    'branch list ignored',
    [[T, 'if (p.branches.some((pattern) => branchMatches(pattern, branch)))', 'if (true)']],
  ],
  ['no provenance trusted', [[T, "if (!isObject(provenance)) return no('NO_PROVENANCE');", '']]],
  [
    'malformed scopes read as private',
    [
      [
        T,
        "    return ['public'];\n  return [...value];",
        '    return [...SCOPES];\n  return [...value];',
      ],
    ],
  ],
  [
    'public always in the record',
    [
      [
        T,
        "({ scopes: ['public'], trusted: false, why: 'UNTRUSTED:' + why })",
        "({ scopes: [...SCOPES], trusted: false, why: 'UNTRUSTED:' + why })",
      ],
    ],
  ],
  [
    'scope not put in the token',
    [[K, '      ...(r.scopes === undefined ? {} : { scopes: r.scopes }),\n    })', '    })']],
  ],
  [
    'missing scope claim read as private',
    [
      [
        K,
        'scopes: normalizeScopes(c.scopes),',
        "scopes: c.scopes === undefined ? ['public', 'private'] : normalizeScopes(c.scopes),",
      ],
    ],
  ],
  ['every lease trusted', [[X, 'scopes: trust.scopes,', "scopes: ['public', 'private'],"]]],
  [
    'admin provenance not handed to the leg',
    [[X, '            provenance: batch.provenance,\n', '']],
  ],
  [
    'publisher redacts no scoped token',
    [[U, '      : [entry.scopes, undefined];', '      : [undefined];']],
  ],
  // ---- the estimate as the price
  [
    'estimate only raises',
    [[Y, 'if (d !== undefined) down = d;', 'if (d !== undefined) down = Math.max(down, d);']],
  ],
  [
    'estimate not capped',
    [
      [
        Y,
        'const pick = (n, max) => (usable(n) ? Math.min(Math.ceil(n), max) : undefined);',
        'const pick = (n) => (usable(n) ? Math.ceil(n) : undefined);',
      ],
    ],
  ],
  [
    'zero estimate accepted',
    [
      [
        Y,
        "const usable = (n) => typeof n === 'number' && Number.isFinite(n) && n > 0;",
        "const usable = (n) => typeof n === 'number' && Number.isFinite(n) && n >= 0;",
      ],
    ],
  ],
  [
    'string estimate accepted',
    [
      [
        Y,
        "const usable = (n) => typeof n === 'number' && Number.isFinite(n) && n > 0;",
        'const usable = (n) => Number.isFinite(Number(n)) && Number(n) > 0;',
      ],
    ],
  ],
  [
    'override stops raising',
    [
      [
        Y,
        'bandwidthDownGb: Math.max(down, cfg.bandwidthDownOverride ?? 0),',
        'bandwidthDownGb: down,',
      ],
    ],
  ],
  ['upload estimate ignored', [[Y, 'if (u !== undefined) up = u;', '']]],
  ['maximum below the row accepted', [[Y, 'top.downGb < base.bandwidthDownGb ||', 'false ||']]],
  [
    'large maximum lowered below the row in the data',
    [D, { change: (d) => (d.payloadBandwidthMax.large.downGb = 100) }],
  ],
  // ---- releaseSeq and the instruction
  ['memo ignored', [[L, 'if (memo !== undefined && memo !== null) {', 'if (false) {']]],
  [
    'request moves a known sha',
    [[L, 'if (requested !== undefined && requested !== memo.seq) {', 'if (false) {']],
  ],
  [
    'counter not advanced',
    [[L, "if (seq >= next) await store.put('release-seq-next', seq + 1);", '']],
  ],
  ['request ignored', [[L, 'let seq = requested;', 'let seq;']]],
  [
    'flag ignored',
    [
      [
        X,
        "if (cfg.propimagesEnabled && profile === 'release' && leg) {",
        "if (profile === 'release' && leg) {",
      ],
    ],
  ],
  [
    'flag on by default',
    [
      [
        Y,
        "propimagesEnabled: env.PROPIMAGES_ENABLED === 'true',",
        "propimagesEnabled: env.PROPIMAGES_ENABLED !== 'false',",
      ],
    ],
  ],
  // ---- the data files
  ['lifetime above 12 hours in the data', [PO, { change: (d) => (d.max_ttl_seconds = 86400) }]],
  ['limit raised in the data', [PO, { change: (d) => (d.urls_per_run = 1000) }]],
  [
    'proprietary prefix removed from the data',
    [PO, { change: (d) => (d.prefixes = d.prefixes.slice(1)) }],
  ],
  [
    'public prefix made private in the data',
    [PO, { change: (d) => d.prefixes.push({ prefix: 'specviewperf-15.0.0', basis: 'x' }) }],
  ],
  [
    'pull request gate allowed in the data',
    [PO, { change: (d) => d.allow.push({ source: 'github', profile: 'check', why: 'x' }) }],
  ],
  [
    'feature branches trusted in the data',
    [TJ, { change: (d) => d.private.branches.push('feature-*') }],
  ],
  [
    'collaborator heads switched off in the data',
    [TJ, { change: (d) => (d.private.collaborator_pr_heads = false) }],
  ],
  [
    'owner actor not required in the data',
    [TJ, { change: (d) => (d.private.require_owner_actor = false) }],
  ],
];

const selected = process.argv.slice(2);
let failed = false;
const tap = (run) => {
  const out = run.stdout ?? '';
  return {
    kills: [...out.matchAll(/^not ok \d+ - (.+)$/gm)]
      .map((m) => m[1])
      .filter((t) => !t.endsWith('.test.js')),
    complete: /^# tests \d+$/m.test(out) && /^# fail \d+$/m.test(out),
  };
};
const test = () =>
  spawnSync(
    process.execPath,
    ['--no-warnings', '--test', '--test-reporter=tap', '--test-timeout=60000', ...TESTS],
    { cwd: here, encoding: 'utf8', timeout: 300000, maxBuffer: 64 * 1024 * 1024 },
  );
// the unmutated copy must pass first, or a "kill" proves nothing
const baseline = test();
if (baseline.status !== 0 || !tap(baseline).complete) {
  console.log(
    JSON.stringify({ mutant: 'baseline', ok: false, why: 'the unmutated copy does not pass' }),
  );
  process.exit(1);
}
for (const [name, edits] of MUTANTS) {
  if (selected.length && !selected.includes(name)) continue;
  const originals = new Map();
  const current = new Map();
  let ambiguous = false;
  if (typeof edits[1]?.change === 'function') {
    const path = here + edits[0];
    const text = readFileSync(path, 'utf8');
    originals.set(path, text);
    const doc = JSON.parse(text);
    edits[1].change(doc);
    current.set(path, JSON.stringify(doc, null, 2) + '\n');
  } else {
    for (const [file, before] of edits) {
      const path = here + file;
      const text = originals.get(path) ?? readFileSync(path, 'utf8');
      originals.set(path, text);
      if (text.split(before).length !== 2) ambiguous = true;
    }
    for (const [path, text] of originals) current.set(path, text);
    if (!ambiguous)
      for (const [file, before, after] of edits) {
        const path = here + file;
        current.set(path, current.get(path).split(before).join(after));
      }
  }
  if (ambiguous) {
    console.log(JSON.stringify({ mutant: name, ok: false, why: 'ambiguous or missing' }));
    failed = true;
    continue;
  }
  try {
    for (const [path, text] of current) writeFileSync(path, text);
    const run = test();
    const { kills, complete } = tap(run);
    // The run must have finished (the TAP summary is printed last): a crash or a hang with no result lines is never a kill.
    const ok = run.status !== 0 && kills.length >= 2 && !run.error && complete;
    failed ||= !ok;
    console.log(
      JSON.stringify({
        mutant: name,
        status: run.status,
        kills: kills.length,
        complete,
        ok,
        first: kills.slice(0, 3),
      }),
    );
  } finally {
    for (const [path, text] of originals) writeFileSync(path, text);
  }
}
if (failed) {
  console.log('SURVIVING OR UNDER-KILLED MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
