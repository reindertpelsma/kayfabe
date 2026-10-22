import unittest
from pathlib import Path
from kayfabe_arm import abort_reason, production_environment


class ArmContract(unittest.TestCase):
    def environment(self, flags=None):
        return production_environment({'flags': flags or {'KF3_TSG_MIRROR': '0'}},
                                      Path('/data/codex-K1/campaign'), '89fc8a222', 100)

    def test_retains_explicit_zero(self):
        self.assertIn('KF3_TSG_MIRROR=0', self.environment()['WIN_FLAGS'])

    def test_preserves_production_flags(self):
        self.assertIn('KF3_KERNEL_GR_WORK=1', self.environment({'KF3_KERNEL_GR_WORK': '1'})['WIN_FLAGS'])

    def test_prof_is_removed(self):
        self.assertNotIn('KF3_PROF=', self.environment({'KF3_PROF': '1'})['WIN_FLAGS'])

    def test_progress_is_enabled(self):
        self.assertIn('KF3_DIAG_PROGRESS=1000', self.environment()['WIN_FLAGS'])

    def test_state_is_lane_local(self):
        self.assertEqual(self.environment()['WIN_DIR'], '/data/codex-K1/campaign')

    def test_disk_is_reused_only_after_explicit_cow_preparation(self):
        self.assertEqual(self.environment()['WIN_REUSE'], '1')

    def test_new_xid_aborts(self):
        self.assertEqual(abort_reason(1, [], 100, 200), 'NEW_HOST_XID')

    def test_multiple_xids_abort(self):
        self.assertEqual(abort_reason(2, [99], 100, 200), 'NEW_HOST_XID')

    def test_four_cycles_abort(self):
        self.assertEqual(abort_reason(0, [1, 2, 3, 4], 100, 200), 'TDR_CYCLE_BUDGET')

    def test_only_recent_cycles_count(self):
        self.assertIsNone(abort_reason(0, [0, 900, 901, 902], 1000, 2000))

    def test_three_cycles_are_allowed(self):
        self.assertIsNone(abort_reason(0, [1, 2, 3], 100, 200))

    def test_rolling_window_not_total(self):
        self.assertEqual(abort_reason(0, [0, 901, 902, 903, 904], 1000, 2000), 'TDR_CYCLE_BUDGET')

    def test_deadline_aborts_at_boundary(self):
        self.assertEqual(abort_reason(0, [], 200, 200), 'RESTORE_DEADLINE')

    def test_deadline_aborts_after_boundary(self):
        self.assertEqual(abort_reason(0, [], 201, 200), 'RESTORE_DEADLINE')

    def test_before_deadline_is_allowed(self):
        self.assertIsNone(abort_reason(0, [], 199, 200))


if __name__ == '__main__':
    unittest.main()
