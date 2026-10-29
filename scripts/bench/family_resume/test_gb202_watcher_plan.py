"""The independent composed watcher must consume the same root-admitted fixed plan."""

import datetime as dt
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


@unittest.skipUnless(os.environ.get('SW5_PROFILE_OPERATOR'), 'private watcher needs env')
class WatcherPlan(unittest.TestCase):
    def fixture(self, field, value):
        sys.path.insert(0, os.environ['SW5_PROFILE_OPERATOR'])
        W = importlib.import_module('precreate_watchdog')
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            now = dt.datetime.now(dt.timezone.utc)
            approved = dict(
                seconds=5400,
                capture_lead_seconds=600,
                deadline_utc=(now + dt.timedelta(seconds=5400)).isoformat(),
            )
            actual = dict(approved, **{field: value})
            (root / 'plan.json').write_text(json.dumps(actual))
            argv = [
                'fixture',
                '--plan',
                str(root / 'plan.json'),
                '--ledger',
                str(root / 'owned'),
                '--ready',
                str(root / 'ready'),
                '--result',
                str(root / 'result'),
                '--intent',
                str(root / 'intent'),
                '--intent-sha256',
                '0' * 64,
            ]
            with patch.object(sys, 'argv', argv), patch.object(
                W.approval_gate, 'require_go', return_value=dict(plan=approved)
            ), patch.object(W.G, 'load', return_value=dict(plan=actual)), patch.object(
                W.G, 'watch', return_value={}
            ) as watch:
                try:
                    W.main()
                except (ValueError, KeyError):
                    pass
                watch.assert_not_called()

    def test_independent_watcher_rejects_other_duration(self):
        self.fixture('seconds', 7200)

    def test_independent_watcher_rejects_other_capture_lead(self):
        self.fixture('capture_lead_seconds', 660)


if __name__ == '__main__':
    unittest.main()
