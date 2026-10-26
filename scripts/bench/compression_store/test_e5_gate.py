"""GPU-free checks against false-positive E5 interpretations."""
import copy
import unittest

from e5_gate import evaluate


def complete():
    rows = [
        dict(event='start', uid=65534, gid=65534),
        dict(event='allocation_params', phase='returned', compr=1, format=8, zbc=1),
        dict(event='coverage', format=1, line_min=1, line_max=16384),
        dict(event='finish', failures=0),
    ]
    for page in (65536, 4096):
        for kind in (8, 0):
            rows += [dict(event='map', requested_page=page, requested_kind=kind, rc=0,
                          status=0, va=0x200000),
                     dict(event='pte', requested_page=page, requested_kind=kind, block=0,
                          page=page, kind=kind, flags=1, comptag_line=0)]
    return rows


class GateTests(unittest.TestCase):
    def test_complete_ab_is_verified(self):
        self.assertTrue(evaluate(complete())['ab_verified'])

    def test_zero_comptag_field_is_not_failure_in_raw_mode(self):
        self.assertTrue(evaluate(complete())['ab_verified'])

    def test_ab_never_certifies_task2_without_c_and_d(self):
        self.assertFalse(evaluate(complete())['task2_eligible'])

    def test_no_zbc_readback_never_certifies_rop_behavior(self):
        self.assertFalse(evaluate(complete())['zbc_suppression_verified'])

    def test_returned_compr_none_is_rejected(self):
        rows = complete()
        rows[1]['compr'] = 0
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_returned_uncompressed_format_is_rejected(self):
        rows = complete()
        rows[1]['format'] = 6
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_missing_returned_attrs_is_rejected(self):
        rows = [r for r in complete() if r['event'] != 'allocation_params']
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_no_coverage_is_rejected(self):
        rows = complete()
        rows[2]['format'] = 0
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_inverted_coverage_is_rejected(self):
        rows = complete()
        rows[2].update(line_min=0xffffffff, line_max=0)
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_empty_capture_is_rejected(self):
        self.assertFalse(evaluate([])['ab_verified'])

    def test_missing_4k_pitch_pte_is_rejected(self):
        rows = [r for r in complete() if not (
            r['event'] == 'pte' and r['requested_page'] == 4096
            and r['requested_kind'] == 0)]
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_missing_64k_compressed_map_is_rejected(self):
        rows = [r for r in complete() if not (
            r['event'] == 'map' and r['requested_page'] == 65536
            and r['requested_kind'] == 8)]
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_downgraded_kind_is_rejected(self):
        rows = complete()
        rows[5]['kind'] = 6
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_pitch_alias_with_inherited_compressed_kind_is_rejected(self):
        rows = complete()
        rows[7]['kind'] = 8
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_invalid_64k_pte_is_rejected(self):
        rows = complete()
        rows[5]['flags'] = 0
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_wrong_4k_page_size_is_rejected(self):
        rows = complete()
        rows[9]['page'] = 65536
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_map_rm_failure_is_rejected(self):
        rows = complete()
        rows[4]['status'] = 0x1f
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_control_rm_failure_is_rejected(self):
        rows = complete() + [dict(event='control', label='pte', rc=0, status=0x1f)]
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_probe_failure_is_rejected(self):
        rows = complete()
        rows[3]['failures'] = 1
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_truncated_capture_is_rejected(self):
        rows = [r for r in complete() if r['event'] != 'finish']
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_prefer_zbc_is_policy_mismatch(self):
        rows = complete()
        rows[1]['zbc'] = 2
        self.assertFalse(evaluate(rows)['zbc_preference_retained'])

    def test_default_zbc_is_policy_mismatch(self):
        rows = complete()
        rows[1]['zbc'] = 0
        self.assertFalse(evaluate(rows)['zbc_preference_retained'])

    def test_root_capture_is_rejected(self):
        rows = complete()
        rows[0]['uid'] = 0
        self.assertFalse(evaluate(rows)['ab_verified'])

    def test_missing_uid_is_rejected(self):
        rows = copy.deepcopy(complete())
        del rows[0]['uid']
        self.assertFalse(evaluate(rows)['ab_verified'])


if __name__ == '__main__':
    unittest.main()
