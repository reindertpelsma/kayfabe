"""Provider status labels nominate an endpoint; pinned SSH/KVM still prove usability."""
import importlib
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'windows'))


class ProviderState(unittest.TestCase):
    def ready(self, actual=None, current=None, ports=None):
        return importlib.import_module('readiness').ssh_route_is_ready(
            dict(actual_status=actual, cur_state=current,
                 ports={'22/tcp': [{'HostPort': '2222'}]} if ports is None else ports))

    def test_created_actual_with_running_current_can_be_probed(self):
        self.assertTrue(self.ready('created', 'running'))

    def test_missing_actual_with_running_current_can_be_probed(self):
        self.assertTrue(self.ready(None, 'running'))

    def test_actual_running_without_current_remains_supported(self):
        self.assertTrue(self.ready('running', None))

    def test_actual_running_with_both_labels_remains_supported(self):
        self.assertTrue(self.ready('running', 'running'))

    def test_loading_without_running_state_is_not_a_candidate(self):
        self.assertFalse(self.ready('loading', 'loading'))

    def test_stopped_without_running_state_is_not_a_candidate(self):
        self.assertFalse(self.ready('stopped', 'stopped'))

    def test_missing_ssh_route_is_not_a_candidate(self):
        self.assertFalse(self.ready('running', 'running', {}))

    def test_empty_ssh_route_is_not_a_candidate(self):
        self.assertFalse(self.ready('created', 'running', {'22/tcp': []}))


if __name__ == '__main__':
    unittest.main()
