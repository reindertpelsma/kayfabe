"""Every published SPECviewperf number is stamped '(nc)' and carries the disclaimer.

Owner decision 2026-10-23: scores may be public only as non-compliant numbers. Result tables
therefore come from spec_stamp.py, never from hand-typed text, and a table that lacks the
disclaimer or holds an unlabelled score fails the checker.
"""
import contextlib
import io
import re
import unittest

import spec_stamp as s

RELEASE = 'SPECviewperf 15.0.1'
CONFIG = 'host RTX 4070 Ti; VM guest Windows 11; kayfabe virtual GPU; driver 580.xx'
DISCLAIMER = (
    '(nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and '
    'not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance '
    'Evaluation Corporation, https://www.spec.org')
RESEARCH = '(nc) research comparison, not a SPEC-ranked result'
ROWS = [
    {'workload': 'blender-01', 'subtest': 2, 'fps': 27.36, 'status': 'Completed'},
    {'workload': 'catia-06', 'subtest': 3, 'fps': 40.5, 'status': 'Failed'},
]
ARMS = {
    'vfio': {'blender-01': 61.61, 'catia-06': 80.0, 'maya-06': None},
    'kayfabe': {'blender-01': 36.03, 'catia-06': 50.53, 'maya-06': 12.0},
}


def md(**kw):
    args = dict(title='Composites', rows=ROWS, fields=['workload', 'subtest', 'fps', 'status'],
                score_fields=['fps'], release=RELEASE, config=CONFIG)
    args.update(kw)
    return s.render_markdown(**args)


def csv_text(**kw):
    args = dict(rows=ROWS, fields=['workload', 'subtest', 'fps', 'status'],
                score_fields=['fps'], release=RELEASE, config=CONFIG)
    args.update(kw)
    return s.render_csv(**args)


class Constants(unittest.TestCase):
    def test_disclaimer_is_the_owner_text_verbatim(self):
        self.assertEqual(s.DISCLAIMER, DISCLAIMER)

    def test_research_heading_is_the_owner_text_verbatim(self):
        self.assertEqual(s.RESEARCH, RESEARCH)


class Labelling(unittest.TestCase):
    def test_every_score_gets_the_suffix(self):
        self.assertEqual(s.label(27.36), '27.36 (nc)')
        self.assertEqual(s.label(40), '40 (nc)')

    def test_missing_score_is_blank_not_labelled(self):
        self.assertEqual(s.label(None), '')

    def test_non_numbers_and_nonfinite_are_refused(self):
        for bad in ('27.36', float('nan'), float('inf'), True):
            with self.assertRaises((TypeError, ValueError)):
                s.label(bad)


class Release(unittest.TestCase):
    def test_exact_release_names_pass(self):
        for ok in ('SPECviewperf 15.0.1', 'SPECviewperf 15.x (patch level not recorded)'):
            self.assertEqual(s.check_release(ok), ok)

    def test_guessing_is_refused(self):
        for bad in ('', None, '15', 'latest', 'unknown', 'SPECviewperf 2020 v3.1',
                    'SPECviewperf 15', 'viewperf 15.0.1'):
            with self.assertRaises(ValueError, msg=repr(bad)):
                s.check_release(bad)

    def test_renderers_require_the_release(self):
        with self.assertRaises(ValueError):
            md(release='')
        with self.assertRaises(ValueError):
            csv_text(release='15')

    def test_renderers_require_the_configuration(self):
        with self.assertRaises(ValueError):
            md(config='')
        with self.assertRaises(ValueError):
            csv_text(config='   ')


class Stamping(unittest.TestCase):
    def test_markdown_labels_every_score_and_not_other_numbers(self):
        text = md()
        self.assertIn('27.36 (nc)', text)
        self.assertIn('40.5 (nc)', text)
        self.assertNotIn('27.36 |', text)
        self.assertIn('| blender-01 | 2 | 27.36 (nc) | Completed |', text)

    def test_markdown_carries_release_configuration_and_disclaimer(self):
        text = md()
        self.assertIn(DISCLAIMER, text)
        self.assertIn(RELEASE, text)
        self.assertIn(CONFIG, text)
        self.assertLess(text.index(DISCLAIMER), text.index('| workload'))

    def test_csv_header_comments_precede_the_table(self):
        lines = csv_text().splitlines()
        self.assertTrue(lines[0].startswith('# '))
        self.assertIn('# ' + DISCLAIMER, lines)
        self.assertIn('# Release: ' + RELEASE, lines)
        first_data = next(i for i, ln in enumerate(lines) if not ln.startswith('#'))
        self.assertEqual(lines[first_data], 'workload,subtest,fps,status')
        self.assertEqual(lines[first_data + 1], 'blender-01,2,27.36 (nc),Completed')

    def test_blank_scores_stay_blank_in_csv(self):
        rows = [{'workload': 'a', 'fps': None}]
        text = csv_text(rows=rows, fields=['workload', 'fps'])
        self.assertTrue(text.rstrip().endswith('a,'))

    def test_generated_tables_pass_the_checker(self):
        self.assertEqual(s.check_text(md(), 'md'), [])
        self.assertEqual(s.check_text(csv_text(), 'csv'), [])


class Ratios(unittest.TestCase):
    def cmp(self, **kw):
        args = dict(arms=ARMS, release=RELEASE, config=CONFIG, kind='md')
        args.update(kw)
        return s.render_comparison(**args)

    def test_no_ratio_column_by_default(self):
        for kind in ('md', 'csv'):
            text = self.cmp(kind=kind)
            self.assertIsNone(re.search(r'(?i)\bratio\b', text))
            self.assertNotIn(RESEARCH, text)
            self.assertIn('36.03 (nc)', text)
            self.assertIn('61.61 (nc)', text)

    def test_ratio_needs_both_the_flag_and_a_baseline(self):
        with self.assertRaises(ValueError):
            self.cmp(ratios=True)
        with self.assertRaises(ValueError):
            self.cmp(ratios=True, baseline='vfio', candidate='nope')

    def test_ratio_column_is_headed_as_research_and_labelled(self):
        text = self.cmp(ratios=True, baseline='vfio', candidate='kayfabe')
        header = next(ln for ln in text.splitlines() if ln.startswith('| workload'))
        self.assertIn(RESEARCH, header)
        self.assertIn('0.58 (nc)', text)
        self.assertEqual(s.check_text(text, 'md'), [])

    def test_ratio_csv_header_and_values(self):
        text = self.cmp(ratios=True, baseline='vfio', candidate='kayfabe', kind='csv')
        header = next(ln for ln in text.splitlines() if not ln.startswith('#'))
        self.assertIn(RESEARCH, header)
        self.assertEqual(s.check_text(text, 'csv'), [])

    def test_no_ratio_over_a_missing_workload(self):
        text = self.cmp(ratios=True, baseline='vfio', candidate='kayfabe')
        row = next(ln for ln in text.splitlines() if ln.startswith('| maya-06'))
        self.assertEqual(row.split('|')[-2].strip(), '')

    def test_comparison_passes_the_checker_with_disclaimer(self):
        self.assertIn(DISCLAIMER, self.cmp())
        self.assertEqual(s.check_text(self.cmp(), 'md'), [])


class Checker(unittest.TestCase):
    def test_table_without_disclaimer_fails(self):
        text = md().replace(DISCLAIMER, 'scores are provisional')
        self.assertTrue(any('disclaimer' in p for p in s.check_text(text, 'md')))
        text = csv_text().replace(DISCLAIMER, 'scores are provisional')
        self.assertTrue(any('disclaimer' in p for p in s.check_text(text, 'csv')))

    def test_truncated_disclaimer_fails(self):
        text = md().replace(', https://www.spec.org', '')
        self.assertTrue(any('disclaimer' in p for p in s.check_text(text, 'md')))

    def test_unlabelled_float_fails(self):
        text = md().replace('27.36 (nc)', '27.36')
        problems = s.check_text(text, 'md')
        self.assertTrue(any('unlabelled' in p and '27.36' in p for p in problems))
        text = csv_text().replace('27.36 (nc)', '27.36')
        self.assertTrue(any('unlabelled' in p for p in s.check_text(text, 'csv')))

    def test_unlabelled_integer_in_a_score_column_fails(self):
        text = md().replace('| 40.5 (nc) |', '| 40 |')
        self.assertTrue(any('unlabelled' in p for p in s.check_text(text, 'md')))

    def test_ratio_column_without_research_heading_fails(self):
        text = md(rows=[{'workload': 'a', 'ratio': 0.5}], fields=['workload', 'ratio'],
                  score_fields=['ratio'])
        self.assertTrue(any('research' in p for p in s.check_text(text, 'md')))

    def test_missing_release_line_fails(self):
        text = md().replace(RELEASE, 'a benchmark')
        self.assertTrue(any('release' in p for p in s.check_text(text, 'md')))

    def test_a_unit_before_the_suffix_is_still_labelled(self):
        for cell in ('1.64% (nc)', '0.58x (nc)'):
            text = md(rows=[{'workload': 'a', 'fps': 1.5, 'range': cell}],
                      fields=['workload', 'fps', 'range'])
            self.assertEqual(s.check_text(text, 'md'), [], cell)
        for cell in ('1.64%', '0.58x', '1.64% nc', '0.58 x (nc)'):
            text = md(rows=[{'workload': 'a', 'fps': 1.5, 'range': cell}],
                      fields=['workload', 'fps', 'range'])
            self.assertTrue(any('unlabelled' in p for p in s.check_text(text, 'md')), cell)

    def test_names_and_dates_are_not_numbers(self):
        text = md(rows=[{'workload': 'creo-03', 'run': 'result_2026-10-22', 'fps': 1.5}],
                  fields=['workload', 'run', 'fps'])
        self.assertEqual(s.check_text(text, 'md'), [])

    def test_require_valid_raises_on_a_bad_table(self):
        with self.assertRaises(s.StampError):
            s.require_valid(md().replace(DISCLAIMER, ''), 'md')
        s.require_valid(md(), 'md')

    def test_cli_check_returns_nonzero_for_a_bad_file(self):
        import tempfile
        import pathlib
        with tempfile.TemporaryDirectory() as tmp:
            good = pathlib.Path(tmp) / 'good.md'
            bad = pathlib.Path(tmp) / 'bad.csv'
            good.write_text(md())
            bad.write_text(csv_text().replace('27.36 (nc)', '27.36'))
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(s.main(['check', str(good)]), 0)
                self.assertEqual(s.main(['check', str(good), str(bad)]), 1)


if __name__ == '__main__':
    unittest.main()
