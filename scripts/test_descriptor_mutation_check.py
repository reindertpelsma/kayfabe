"""Regression checks for distinguishing malformed-entry kills from setup failures."""
import unittest

import descriptor_mutation_check as subject


class FormatKillAudit(unittest.TestCase):
    def source_and_log(self):
        source = []
        blocks = []
        for name, marker in subject.FORMAT_KILL_SITES.items():
            source.extend([f'fn {name}() {{', '    healthy_setup();',
                           f'    bad_entry().expect_err("{marker}");', '}'])
            blocks.append(
                f'---- {name} stdout ----\n'
                f"thread '{name}' (42) panicked at crates/kf-chan/tests/user_turn_unit.rs:{len(source)-1}:5:\n"
                f'{marker}: ()\n')
        controls = '\n'.join(f'test {name} ... ok' for name in subject.FORMAT_CONTROLS)
        return '\n'.join(source), controls + '\n' + '\n'.join(blocks)

    def test_accepts_both_own_bad_entry_assertions_and_healthy_controls(self):
        source, log = self.source_and_log()
        sites = subject.audit_format_kills(log, source)
        self.assertEqual(set(sites), set(subject.FORMAT_KILL_SITES))
        self.assertTrue(all(v['assertion_marker'] in log for v in sites.values()))

    def test_rejects_shared_setup_panics_even_when_both_names_fail(self):
        source, log = self.source_and_log()
        for marker in subject.FORMAT_KILL_SITES.values():
            log = log.replace(marker + ': ()', 'assertion failed: Scan versus Pointer')
        with self.assertRaisesRegex(ValueError, 'bad-entry assertion'):
            subject.audit_format_kills(log, source)

    def test_rejects_correct_marker_at_wrong_source_line(self):
        source, log = self.source_and_log()
        log = log.replace('.rs:3:5:', '.rs:2:5:')
        with self.assertRaisesRegex(ValueError, 'bad-entry assertion'):
            subject.audit_format_kills(log, source)

    def test_rejects_marker_from_another_failed_test(self):
        source, log = self.source_and_log()
        name, marker = next(iter(subject.FORMAT_KILL_SITES.items()))
        log = log.replace(marker + ': ()', 'other panic')
        log += f'\n---- unrelated stdout ----\n{marker}: ()\n'
        with self.assertRaisesRegex(ValueError, 'bad-entry assertion'):
            subject.audit_format_kills(log, source)

    def test_rejects_a_failed_positive_control(self):
        source, log = self.source_and_log()
        name = subject.FORMAT_CONTROLS[0]
        log = log.replace(f'test {name} ... ok', f'test {name} ... FAILED')
        with self.assertRaisesRegex(ValueError, 'positive control'):
            subject.audit_format_kills(log, source)


if __name__ == '__main__':
    unittest.main()
