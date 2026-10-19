# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""A displayed refresh is counted once even when two feedback objects fire."""
import sys
import tempfile
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import broker_content


class DisplayedRefreshContract(unittest.TestCase):
    def analyze(self, body):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory) / 'trace.txt'
            p.write_text(body)
            return broker_content.summarize(p, 0, 100_000_000)

    def test_a_pump_and_content_feedback_for_one_refresh_are_one_new_present(self):
        r = self.analyze('B 1 10000000 1\nB 2 10000000 0\nB 3 60000000 1\nB 4 60000000 0\n')
        self.assertEqual(r['new_content_presents'], 2)
        self.assertEqual(r['repeated_presents_per_s'], 0)
        self.assertEqual(r['duplicate_feedback_records'], 2)

    def test_two_content_callbacks_at_one_display_time_cannot_create_two_frames(self):
        r = self.analyze('B 1 10000000 1\nB 2 10000000 1\nB 3 60000000 1\nB 4 60000000 1\n')
        self.assertEqual(r['new_content_presents'], 2)
        self.assertEqual(r['one_percent_low_fps'], 20)
        self.assertEqual(r['duplicate_feedback_records'], 2)

    def test_an_unknown_presentation_clock_before_the_samples_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'clock'):
            self.analyze('X CLOCK_ONLY 1\nB 2 10000000 1\nB 3 60000000 1\n')

    def test_mixing_a_fallback_clock_with_native_feedback_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'clock'):
            self.analyze('B 1 10000000 1\nX CLOCK_ONLY 2\nB 3 60000000 1\n')


if __name__ == '__main__':
    unittest.main()
