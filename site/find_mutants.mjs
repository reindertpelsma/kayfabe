// Mutation check for the "Find my setup" page (status/status.js, find view). Each mutant is applied
// in place, site/test_status_dom.mjs runs, and the file is restored before the next one (also on
// error). Every mutant must make at least one named test fail. Run alone, from anywhere:
//   node site/find_mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const target = here + 'status/status.js';
const TEST = here + 'test_status_dom.mjs';

const MUTANTS = [
  ['community on by default', "const SHOW_DEFAULT = ['reproduced', 'imported'];", "const SHOW_DEFAULT = ['reproduced', 'imported', 'community'];"],
  ['imported off by default', "const SHOW_DEFAULT = ['reproduced', 'imported'];", "const SHOW_DEFAULT = ['reproduced'];"],
  ['nothing ticked gives an empty page', 'picked.length ? [...new Set(picked)] : SHOW_DEFAULT', '[...new Set(picked)]'],
  ['community requested without opt-in', "(wanted.includes('community') ? '&source=' + wanted.join(',') : '')", "'&source=' + wanted.join(',')"],
  ['source order follows the address', "['reproduced', 'community'].filter((x) => f.show.includes(x))", 'f.show.filter((x) => x === \'reproduced\' || x === \'community\')'],
  ['cpu filter ignored', "const byCpu = f.cpu === 'any' ? rows : rows.filter((s) => s.cpu_vendor === f.cpu);", 'const byCpu = rows;'],
  ['driver filter host only', 's.host_driver === f.driver || s.guest_driver === f.driver', 's.host_driver === f.driver'],
  ['laptop gets results', "if (/\\b(?:laptop|mobile|notebook|max-?q)\\b/i.test(typed)) return { kind: 'laptop' };", ''],
  ['longer names beat the exact one', 'if (exact.length) return exact.map((r) => r.entry).slice(0, limit);', ''],
  ['os parameter unvalidated', 'os: /^(?:linux|windows)$/,', 'os: /./,'],
  ['tier parameter unvalidated', 'tier: /^(?:largest|middle|small|adhoc)$/,', 'tier: /./,'],
  ['driver parameter unvalidated', "\n    driver: /^[0-9]{3}\\.[0-9]{1,3}(?:\\.[0-9]{1,3})?$/,", '\n    driver: /./,'],
  ['host driver parameter unvalidated', "\n    host_driver: /^[0-9]{3}\\.[0-9]{1,3}(?:\\.[0-9]{1,3})?$/,", '\n    host_driver: /./,'],
  ['guest driver parameter unvalidated', "\n    guest_driver: /^[0-9]{3}\\.[0-9]{1,3}(?:\\.[0-9]{1,3})?$/,", '\n    guest_driver: /./,'],
  ['not-tested reason dropped', "else if (cell?.state === 'not_covered' && text(cell.reason))", 'else if (false)'],
  ['evidence heading dropped', "el('h3', {}, 'Evidence in the repository, not CI gate results'),", ''],
  ['claims note weakened', 'are reported by the test box and are not verified: they sort the runs', 'are reported by the test box: they sort the runs'],
  ['cpu filter not sent to the API', "(mode === 'live' && f.cpu !== 'any' ? '&cpu=' + f.cpu : '') +", "''  +"],
  ['driver filter not sent to the API', "(mode === 'live' && f.driver ? '&driver=' + f.driver : '') +", "''  +"],
  ['run page drops the box driver', "if (text(data.host?.host_driver)) facts.push(['Host driver (reported)', text(data.host.host_driver)]);", ''],
  ['arm selectable', "el('input', { type: 'radio', name, value: id, disabled: true }),", "el('input', { type: 'radio', name, value: id }),"],
  ['other-guest FAIL shown as fail', "latest.result === 'UNATTRIBUTED' && latest.verdict === 'FAIL'", 'false'],
  ['filters lost from links', "el('a', { href: page('find', keep(e.name)) }, e.name),", "el('a', { href: page('find', { gpu: e.name }) }, e.name),"],
  // Run report, session, stats and nearest subruns (contract of 2026-11-07).
  ['log button accepts any origin', "href: want && log.url === want ? want : '', log };", "href: log.url, log };"],
  ['failures not listed first', 'rank(a.t.result) - rank(b.t.result) || a.i - b.i', 'a.i - b.i'],
  ['viewport row drawn on the run page', "      (r) => isObj(r) && text(r.name) && finite(r.value) !== null && !/spec\\s*view/i.test(text(r.name) + ' ' + text(r.suite)),\n    );\n    const head = 'Performance numbers';", "      (r) => isObj(r) && text(r.name) && finite(r.value) !== null,\n    );\n    const head = 'Performance numbers';"],
  ['viewport row drawn in the stats', "        (r) => isObj(r) && text(r.name) && finite(r.value) !== null && !/spec\\s*view/i.test(text(r.name) + ' ' + text(r.suite)),\n      ),\n    };", "        (r) => isObj(r) && text(r.name) && finite(r.value) !== null,\n      ),\n    };"],
  ['subrun rows not filtered', 'const own = (rows) => arr(rows).filter((r) => isObj(r) && r.sub === s.sub);', 'const own = (rows) => arr(rows).filter((r) => isObj(r));'],
  ['relaxed subrun called same', "'different ' + relaxLabel(r.dim),", "'same ' + relaxLabel(r.dim),"],
  ['relax list unvalidated', 'return list.every((d) => RELAX_DEFAULT.includes(d)) && new Set(list).size === list.length ? list : RELAX_DEFAULT;', 'return list;'],
  ['relax not sent to the API', "if (f.relaxGiven) q.set('relax', f.relax.length ? f.relax.join(',') : 'none');", ''],
  ['stats sort unvalidated', "sort: SORTS[query.get('sort')] ? query.get('sort') : null,", "sort: query.get('sort'),"],
  ['unknown result drawn as pass', 'if (VERDICTS.has(result)) return verdict(result, false);\n    if (result === \'REFUSED\')', 'if (result) return verdict(\'PASS\', false);\n    if (result === \'REFUSED\')'],
  ['missing report drawn as empty table', "if (!isObj(report))\n      return section(head, scope + '-tests', emptyNote('Not measured yet: this run has no stored report.'));", ''],
];

const selected = process.argv.slice(2);
const original = readFileSync(target, 'utf8');
let failed = false;
try {
  for (const [name, before, after] of MUTANTS) {
    if (selected.length && !selected.includes(name)) continue;
    if (original.split(before).length !== 2) {
      console.log(JSON.stringify({ mutant: name, ok: false, why: 'ambiguous or missing' }));
      failed = true;
      continue;
    }
    writeFileSync(target, original.replace(before, () => after));
    const run = spawnSync(process.execPath, ['--test', '--test-reporter=tap', TEST], {
      cwd: here,
      encoding: 'utf8',
      timeout: 300000,
      maxBuffer: 64 * 1024 * 1024,
    });
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)].map((m) => m[1]);
    const ok = run.status !== 0 && kills.length >= 1 && !run.error;
    failed ||= !ok;
    console.log(JSON.stringify({ mutant: name, status: run.status, kills, ok }));
  }
} finally {
  writeFileSync(target, original);
}
if (failed) {
  console.log('SURVIVING MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
