from pathlib import Path
import unittest

class ObserverWiring(unittest.TestCase):
    def setUp(self):self.s=Path(__file__).with_name('arm_fields.sh').read_text();self.enable=self.s.index('timeout 600 bash')
    def test_observer_starts_before_enabling_the_driver(self):self.assertLess(self.s.index('init_observer.py'),self.enable)
    def test_disabled_ready_receipt_is_required_before_enable(self):
        self.assertLess(self.s.index('INIT_OBSERVER_READY'),self.enable)
        self.assertLess(self.s.index('[ -f "$init_ready" ]'),self.enable)
    def test_cleanup_stops_own_init_observer_before_clearing_phase(self):self.assertLess(self.s.index('kill -TERM "$init_observer_pid"'),self.s.index("<<'PHASE_END'"))
    def test_late_diagnostic_cannot_erase_the_early_observation(self):self.assertNotIn('phase_xid_guard.py record-init',self.s)

if __name__=='__main__':unittest.main()
