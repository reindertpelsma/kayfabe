#!/usr/bin/env python3
"""Mutation check for spec_stamp.py: every mutant must be killed by the tests.

Run: python3 scripts/bench/specviewperf/mutate_spec_stamp.py
Each mutant weakens one stamping rule in a scratch copy; the unit tests must then fail. A
mutant that survives means a rule has no test. Exit status 1 if any mutant survives.
"""
import pathlib
import shutil
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
TESTS = ['test_spec_stamp.py', 'test_result_table.py']
MUTANTS = [
    ('score loses the (nc) suffix',
     "return '%s %s' % (value, NC)", "return '%s' % (value,)"),
    ('disclaimer dropped from the header',
     "            DISCLAIMER]", "            ]"),
    ('ratio column on by default',
     "ratios=False, baseline=None", "ratios=True, baseline=None"),
    ('release guessing accepted',
     "if not isinstance(release, str) or not RELEASE_RE.match(release):", "if False:"),
    ('blank configuration accepted',
     "if not isinstance(config, str) or not config.strip() or '\\n' in config:", "if False:"),
    ('ratio header loses the research text',
     "return 'ratio %s/%s -- %s' % (candidate, baseline, RESEARCH)",
     "return 'ratio %s/%s' % (candidate, baseline)"),
    ('checker ignores a missing disclaimer',
     "if DISCLAIMER not in text:", "if False:"),
    ('checker ignores unlabelled decimals',
     "floaty = '.' in token or 'e' in token.lower()", "floaty = False"),
    ('checker ignores unlabelled integers in score columns',
     "(floaty or SCORE_HEADER_RE.search(head)) and not labelled", "floaty and not labelled"),
    ('checker accepts any text after a number as the label',
     "LABEL_TAIL_RE = re.compile(r'[%x]? ' + re.escape(NC))", "LABEL_TAIL_RE = re.compile(r'')"),
    ('checker flags dotted versions as scores',
     "if VERSION_RE.fullmatch(cell):", "if False:"),
    ('checker ignores an unresearched ratio column',
     "if RATIO_HEADER_RE.search(column) and RESEARCH not in column:", "if False:"),
]


def main():
    survivors = []
    for name, old, new in MUTANTS:
        with tempfile.TemporaryDirectory() as tmp:
            for path in HERE.glob('*.py'):
                shutil.copy(path, tmp)
            target = pathlib.Path(tmp) / 'spec_stamp.py'
            source = target.read_text()
            if source.count(old) != 1:
                print('STALE MUTANT (pattern count %d): %s' % (source.count(old), name))
                survivors.append(name)
                continue
            target.write_text(source.replace(old, new))
            run = subprocess.run([sys.executable, '-I', '-m', 'unittest', *[t[:-3] for t in TESTS]],
                                 cwd=tmp, capture_output=True, text=True)
            killed = run.returncode != 0
            print('%-8s %s' % ('killed' if killed else 'SURVIVED', name))
            if not killed:
                survivors.append(name)
    print('%d mutants, %d survived' % (len(MUTANTS), len(survivors)))
    return 1 if survivors else 0


if __name__ == '__main__':
    sys.exit(main())
