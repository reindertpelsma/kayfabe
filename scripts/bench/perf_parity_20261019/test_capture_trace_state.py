import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import host_measure


class GuestCommandDispatched(Exception):
    pass


class TraceStateTests(unittest.TestCase):
    def exercise(self, kind, enabled=True):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            dump = root / 'edges'
            flags = f'KF3_DIAG_NSI_TRACE={int(enabled)} KF3_DIAG_NSI_TRACE_DUMP={dump}'
            (root / 'active-arm.json').write_text(json.dumps({'flags': flags}))
            if kind == 'request':
                dump.touch()
            elif kind == 'request_directory':
                dump.mkdir()
            elif kind == 'complete_output':
                Path(str(dump) + '.out').write_text('kf3: DIAG_NSI_TRACE_END\n')
            elif kind == 'partial_output':
                Path(str(dump) + '.out').write_text('kf3: DIAG_NSI_TRACE_BEGIN\n')
            with mock.patch.object(host_measure, 'stats_checkpoint', return_value=None), \
                    mock.patch.object(host_measure, 'agent', side_effect=GuestCommandDispatched):
                if enabled and kind != 'fresh':
                    with self.assertRaisesRegex(RuntimeError, 'trace already frozen'):
                        host_measure.capture(root, 'sample', 0)
                else:
                    with self.assertRaises(GuestCommandDispatched):
                        host_measure.capture(root, 'sample', 0)

    def test_a_dump_request_prevents_another_capture_before_guest_dispatch(self):
        self.exercise('request')

    def test_a_request_directory_cannot_hide_an_already_requested_freeze(self):
        self.exercise('request_directory')

    def test_removing_the_request_does_not_make_a_completed_dump_active(self):
        self.exercise('complete_output')

    def test_a_partial_dump_also_means_the_recorder_was_frozen(self):
        self.exercise('partial_output')

    def test_a_fresh_enabled_recorder_can_start_capture(self):
        self.exercise('fresh')

    def test_disabled_recording_is_a_separate_explicit_control(self):
        self.exercise('complete_output', enabled=False)


if __name__ == '__main__':
    unittest.main()
