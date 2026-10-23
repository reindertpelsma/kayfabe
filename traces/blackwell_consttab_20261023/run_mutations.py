from pathlib import Path
import datetime
import hashlib
import json
import os
import re
import subprocess

D = Path(__file__).resolve().parent
W = Path('/workspace/bw-precomp-consttab-wt')
paths = ['crates/kf-rm/src/consttab.rs', 'crates/kf-rm/data/precomp_measured.tsv',
         'crates/kf-rm/src/precomp.rs', 'crates/kf-qemu/src/display.rs']
original = {p: (W / p).read_text() for p in paths}
env = dict(os.environ, CARGO_TARGET_DIR=str(D / 'target'), CARGO_TERM_COLOR='never')
mutants = []

def replace(path, old, new):
    def mutate():
        text = original[path]
        assert text.count(old) == 1, (path, old, text.count(old))
        return {path: text.replace(old, new, 1)}
    return mutate

mutants.append(('drop_precomp_table', replace(paths[0],
    'rows.extend(parse(PRECOMP_TSV)?);', 'rows.extend(Vec::new());'), 'rm'))

def wrong_field():
    lines = original[paths[1]].splitlines()
    layout = next(line.split('\t') for line in
        (W / 'crates/kf-disp/data/classes-595.91.07.tsv').read_text().splitlines()
        if line.startswith('F\tNVCA73_PRECOMP_WIN_PIPE_HDR_CAPC_CSC0LUT_FVLUT\t'))
    mask = 1 << int(layout[3])
    for index, line in enumerate(lines):
        p = line.split('\t')
        if p[0] == 'display.precomp.0xca73.0x0788' and p[1] == 'GB202':
            p[5] = hex(int(p[5], 0) ^ mask)
            lines[index] = '\t'.join(p)
            break
    else:
        raise AssertionError('word absent')
    return {paths[1]: '\n'.join(lines) + '\n'}
mutants.append(('wrong_gb202_fvlut_value', wrong_field, 'rm'))
mutants.append(('wrong_measurement_date', lambda: {paths[1]:
    original[paths[1]].replace(',2026-10-23\ttraces/', ',2026-10-24\ttraces/')}, 'rm'))
mutants.append(('missing_word_each_die', lambda: {paths[1]: '\n'.join(line for line in
    original[paths[1]].splitlines() if not line.startswith('display.precomp.0xca73.0x0874\t'))
    + '\n'}, 'rm'))
mutants.append(('bypass_table_serving', replace(paths[2],
    'let branch = major(context.host_driver)?;',
    'if true { return Ok(initial); }\n        let branch = major(context.host_driver)?;'), 'rm'))
mutants.append(('donor_before_exact_row', replace(paths[2],
    'let exact = rows.iter().any', 'let exact = false && rows.iter().any'), 'rm'))
mutants.append(('ignore_row_source_and_extent', replace(paths[2],
    'let mut why = source(row, origin, branch)?;', 'let mut why = String::from("mutant");'), 'rm'))
mutants.append(('ignore_actual_host_branch', replace(paths[2],
    'let branch = major(context.host_driver)?;', 'let branch = major("595.91.07")?;'), 'rm'))
mutants.append(('inherit_without_reason', replace(paths[2],
    'if rule.reason.trim().is_empty() {', 'if false && rule.reason.trim().is_empty() {'), 'rm'))
mutants.append(('inherit_across_display_ip', replace(paths[2],
    'if !same_layout(own, donor_row) {', 'if false && !same_layout(own, donor_row) {'), 'rm'))
mutants.append(('drop_explicit_zero_words', replace(paths[2],
    'initial.page.words = page.into_iter().collect();',
    'initial.page.words = page.into_iter().filter(|(_, v)| *v != 0).collect();'), 'rm'))
mutants.append(('lose_inherited_coverage_label', replace(paths[2],
    'inherited-with-reason:{id}<-{origin}', 'unlabelled:{id}<-{origin}'), 'rm'))
mutants.append(('qemu_keeps_only_derived_page', replace(paths[3],
    'kf_rm::precomp::apply(derived, context)', 'Ok(derived)'), 'qemu'))
results = []
(D / 'mutant_pristine_hashes.json').write_text(json.dumps(
    {p: hashlib.sha256(s.encode()).hexdigest() for p, s in original.items()}, indent=2) + '\n')
try:
    for name, mutate, group in mutants:
        available = os.statvfs('/').f_bavail * os.statvfs('/').f_frsize
        assert available >= 4 * 1024**3, 'disk guard before cargo'
        for path, text in mutate().items():
            (W / path).write_text(text)
        log = D / ('mutant_' + name + '.log')
        args = ['nice', '-n19', 'cargo', 'test', '--release', '-j2']
        args += (['-p', 'kf-rm', '--test', 'precomp_consttab_data', '--test', 'precomp_serving']
                 if group == 'rm' else ['-p', 'kf-qemu', '--test', 'precomp_serving'])
        try:
            with log.open('w') as output:
                p = subprocess.run(args, cwd=W, env=env, stdout=output,
                                   stderr=subprocess.STDOUT, timeout=480)
            text = log.read_text()
            killers = sorted(set(re.findall(r'^test (.+) \.\.\. FAILED$', text, re.MULTILINE)))
            result = {'name': name, 'returncode': p.returncode, 'killers': killers,
                      'killing_test_count': len(killers), 'compile_only_failure':
                      p.returncode != 0 and not killers,
                      'free_bytes_before': available,
                      'utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
            results.append(result)
            (D / 'mutations.json').write_text(json.dumps(results, indent=2) + '\n')
            print(name, 'KILLERS', len(killers), flush=True)
            assert p.returncode != 0 and len(killers) >= 2, result
        finally:
            for path, text in original.items():
                (W / path).write_text(text)
finally:
    for path, text in original.items():
        (W / path).write_text(text)
assert all((W / p).read_text() == text for p, text in original.items())
print('ALL_MUTANTS_KILLED_AND_SOURCE_RESTORED', len(results), flush=True)
