#!/usr/bin/env python3
import copy
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('boundary_compare', Path(__file__).with_name('compare.py'))
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)


def kf(fn=76, selector='cmd=0x20801111', status='0x0', handle='0xff001000'):
    return f'prefix kf-rm: rpc-trace fn={fn} RmControl seq=0 {selector} client=0xc1000000 object={handle} result={status}\n'


def native(direction='reply', command='0x20801111', status='0x0', gap=0, prefix=False):
    return dict(direction=direction, rpc_function=76, rpc_status=status, missing_before=gap,
                prefix_unknown=prefix, rpc_sequence=0, table_pa='0xdeadbeef', payload_hex='secret',
                control=dict(command=command, status='0x56', params_bytes_declared=40,
                             params_bytes_observed=40, params_complete=True, params_hex='secret'))


def decoded(rows):
    return json.dumps(dict(schema=compare.NATIVE_SCHEMA, complete=False,
                           records=len(rows), observations=rows)).encode()


class CompareTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.identity = dict(baseline_sha256='a' * 64, windows_driver='580.88',
                             driver_hash='sha256:provided-exact-driver', gpu='RTX4070', qemu_revision='b431aeaf')
        self.doc = dict(schema=1, experiment=self.identity, runs=[])

    def run_row(self, name, arm='kayfabe', events=None, words=None):
        row = dict(id=name, arm=arm, variant='L' if arm == 'kayfabe' else 'native',
                   flags={'tmo_constructor_probe': arm == 'kayfabe'}, instrumentation='bounded observer',
                   metadata=self.identity.copy())
        if events is not None:
            path = self.root / (name + '.rpc')
            path.write_bytes(events.encode() if isinstance(events, str) else events)
            row['rpc'] = dict(path=path.name, format='kf-rm' if arm == 'kayfabe' else 'kgwt-decoded-json',
                              coverage='partial', window='startup', note='selected startup capture')
        if words is not None:
            path = self.root / (name + '.bin')
            path.write_bytes(struct.pack('<1024I', *words))
            row['display_caps'] = dict(path=path.name, milestone='status sampled after fixed wait',
                                       source='BAR0+0x640000', note='exact4096byte snapshot')
        self.doc['runs'].append(row)
        return row

    def report(self):
        path = self.root / 'manifest.json'
        path.write_text(json.dumps(self.doc))
        return compare.build_report(path)

    def test_handles_not_identity_and_none_not_numeric_error(self):
        a, _ = compare.parse_kf(kf(handle='0xff001000').encode())
        b, _ = compare.parse_kf(kf(handle='0xff777000').encode())
        self.assertEqual(a, b)
        refused, _ = compare.parse_kf(kf(status='none').encode())
        self.assertIsNone(refused[0]['status'])
        numeric, _ = compare.parse_kf(kf(status='0x56').encode())
        self.assertNotEqual(refused, numeric)

    def test_same_counts_different_order_detected(self):
        x, y = kf(), kf(selector='cmd=0x20801110')
        self.run_row('k1', events=x + y)
        self.run_row('k2', events=y + x)
        stream = self.report()['groups'][0]['rpc'][0]['streams'][0]
        self.assertFalse(stream['identical_observed_order'])
        self.assertEqual(stream['first_differences'][0]['first_difference'], 0)
        self.assertTrue(all(row['stable'] for row in stream['counts']))

    def test_counts_preserve_absence_vs_missing_capture(self):
        self.run_row('k1', events=kf())
        self.run_row('k2', events='non-RPC line\n')
        self.run_row('k3', words=[0] * 1024)
        report = self.report()
        row = report['groups'][0]['rpc'][0]['streams'][0]['counts'][0]
        self.assertEqual(row['counts'], dict(k1=1, k2=0, k3=None))
        self.assertTrue(any('k3: RPC coverage unavailable' in x for x in report['warnings']))

    def test_native_partial_status_and_no_pairing(self):
        rows = [native('request', status='0xffffffff', gap=3, prefix=True), native()]
        rows[0]['control']['params_complete'] = False
        rows[0]['control']['params_bytes_observed'] = 8
        self.run_row('v1', 'vfio', decoded(rows))
        self.run_row('v2', 'vfio', decoded([]))
        report = self.report()
        rpc = report['runs'][0]['rpc']
        self.assertEqual(rpc['details']['observed_missing'], 3)
        self.assertEqual(rpc['details']['incomplete_control_records'], 1)
        self.assertEqual(rpc['events'][0]['status'], '0xffffffff')
        self.assertEqual(rpc['events'][1]['control_status'], '0x00000056')
        self.assertEqual([s['direction'] for s in report['groups'][0]['rpc'][0]['streams']], ['request', 'reply'])
        self.assertNotIn('secret', json.dumps(report))
        self.assertNotIn('deadbeef', json.dumps(report))
        self.assertTrue(any('never proved absent' in x for x in report['interpretation']))

    def test_native_unknown_class_not_guessed(self):
        row = native()
        row.pop('control')
        row['rpc_function'] = 103
        events, _ = compare.parse_native(decoded([row]), 'kgwt-decoded-json')
        self.assertEqual(events[0]['selector_kind'], 'class')
        self.assertIsNone(events[0]['selector'])

    def test_cross_arm_records_keep_status_and_coverage_distinct(self):
        self.run_row('v1', 'vfio', decoded([native('request'), native()]))
        self.run_row('v2', 'vfio', decoded([]))
        self.run_row('k1', events=kf(status='none') + kf(fn=103, selector='class=0xb297'))
        report = self.report()
        cross = report['cross_arm_rpc'][0]
        control = next(row for row in cross['observations'] if row['function'] == 76)
        self.assertEqual(control['vfio_reply']['v1']['statuses'],
                         [dict(status='0x00000000', control_status='0x00000056', count=1)])
        self.assertEqual(control['vfio_reply']['v2']['observed_count'], 0)
        self.assertEqual(control['kayfabe_completed']['k1']['statuses'],
                         [dict(status=None, control_status=None, count=1)])
        allocation = next(row for row in cross['observations'] if row['function'] == 103)
        self.assertIsNone(allocation['control'])
        self.assertNotIn('0x0000b297', json.dumps(cross))

    def test_native_decoded_jsonl_and_alias(self):
        row = native()
        row['control']['cmd'] = row['control'].pop('command')
        header = dict(schema=compare.NATIVE_SCHEMA, kind='header', complete=False, records=1)
        raw = (json.dumps(header) + '\n' + json.dumps(row) + '\n').encode()
        events, _ = compare.parse_native(raw, 'kgwt-decoded-jsonl')
        self.assertEqual(events[0]['selector'], '0x20801111')
        with self.assertRaisesRegex(compare.InvalidEvidence, 'count mismatch'):
            compare.parse_native(json.dumps(header).encode(), 'kgwt-decoded-jsonl')

    def test_native_list_records(self):
        raw = json.dumps(dict(schema=compare.NATIVE_SCHEMA, complete=False, records=[native()])).encode()
        self.assertEqual(len(compare.parse_native(raw, 'kgwt-decoded-json')[0]), 1)

    def test_native_malformed_and_contradictory_coverage_refused(self):
        for edit in ('direction', 'control', 'count', 'complete', 'raw'):
            with self.subTest(edit=edit):
                row = native()
                doc = json.loads(decoded([row]))
                if edit == 'direction':
                    doc['observations'][0]['direction'] = 0
                elif edit == 'control':
                    doc['observations'][0]['control']['params_bytes_observed'] = 4
                elif edit == 'count':
                    doc['records'] = 2
                elif edit == 'complete':
                    doc['complete'] = True
                else:
                    doc['schema'] = 'kayfabe-gsp-text/1'
                with self.assertRaises(compare.InvalidEvidence):
                    compare.parse_native(json.dumps(doc).encode(), 'kgwt-decoded-json')
        row = self.run_row('v1', 'vfio', decoded([]))
        row['rpc']['coverage'] = 'complete-window'
        with self.assertRaisesRegex(compare.InvalidEvidence, 'must be partial'):
            self.report()

    def test_capability_variability_all_values_and_stable_differences(self):
        for arm in ('vfio', 'kayfabe'):
            for i in range(3):
                words = [0] * 1024
                words[1] = i
                words[2] = 4 if arm == 'vfio' else 8
                self.run_row(arm + str(i), arm, words=words)
        report = self.report()
        for group in report['groups']:
            caps = group['display_caps'][0]
            self.assertEqual(caps['states'], dict(stable=1023, variable=1))
            self.assertEqual(set(caps['words'][1]['values'].values()), {'0x00000000', '0x00000001', '0x00000002'})
        self.assertEqual(report['cross_arm_caps'][0]['stable_differences'],
                         [dict(offset='0x0008', vfio='0x00000004', kayfabe='0x00000008')])
        self.assertIn('not automatically defects', compare.markdown(report))

    def test_single_and_missing_caps_not_stable(self):
        self.run_row('k1', words=[0] * 1024)
        self.run_row('k2', events=kf())
        caps = self.report()['groups'][0]['display_caps'][0]
        self.assertEqual(caps['states'], {'single-observation': 1024})
        self.assertEqual(caps['words'][0]['values'], dict(k1='0x00000000', k2=None))

    def test_wrong_caps_size_rejected(self):
        row = self.run_row('k1', words=[0] * 1024)
        path = self.root / row['display_caps']['path']
        for size in (4095, 4097):
            path.write_bytes(bytes(size))
            with self.assertRaises(compare.InvalidEvidence):
                self.report()

    def test_mismatched_flags_identities_windows_not_pooled(self):
        a = self.run_row('k1', events=kf(), words=[0] * 1024)
        b = self.run_row('k2', events=kf(), words=[0] * 1024)
        b['flags'] = {'tmo_constructor_probe': False}
        c = self.run_row('k3', events=kf(), words=[0] * 1024)
        c['metadata']['windows_driver'] = 'other'
        d = self.run_row('k4', events=kf(), words=[0] * 1024)
        d['rpc']['window'] = 'restart'
        d['display_caps']['milestone'] = 'different stage'
        report = self.report()
        self.assertEqual(len(report['groups']), 3)
        group = next(g for g in report['groups'] if 'k1' in g['runs'])
        self.assertEqual(len(group['rpc']), 2)
        self.assertEqual(len(group['display_caps']), 2)
        self.assertTrue(any('not a matched-identity' in x for x in report['warnings']))

    def test_duplicate_run_and_json_keys_rejected(self):
        self.run_row('k1', events=kf())
        self.doc['runs'].append(copy.deepcopy(self.doc['runs'][0]))
        with self.assertRaisesRegex(compare.InvalidEvidence, 'duplicate run id'):
            self.report()
        with self.assertRaisesRegex(compare.InvalidEvidence, 'duplicate JSON key'):
            compare.parse_json(b'{"a":1,"a":2}')

    def test_malformed_logger_lines_refused_not_silently_skipped(self):
        for raw in ('kf-rm: rpc-trace fn=76 broken', kf(selector=''), kf(status='0x100000000'),
                    kf(selector='cmd=0x1 cmd=0x2')):
            with self.subTest(raw=raw), self.assertRaises(compare.InvalidEvidence):
                compare.parse_kf(raw.encode())

    def test_native_cmd_alias_conflict_and_unsigned_bounds(self):
        row = native()
        row['control']['cmd'] = '0x1'
        with self.assertRaisesRegex(compare.InvalidEvidence, 'conflicting'):
            compare.parse_native(decoded([row]), 'kgwt-decoded-json')
        for value in (True, -1, 2 ** 32, '0x100000000'):
            with self.assertRaises(compare.InvalidEvidence):
                compare.u32(value, 'field')

    def test_deterministic_report_and_exact_input_hashes(self):
        self.run_row('k1', events=kf(), words=[0] * 1024)
        one, two = self.report(), self.report()
        self.assertEqual(one, two)
        self.assertEqual(compare.markdown(one), compare.markdown(two))
        self.assertEqual(one['runs'][0]['rpc']['sha256'], compare.digest(kf().encode()))

    def test_bounded_read_and_record_limit(self):
        path = self.root / 'small'
        path.write_bytes(b'12345')
        with self.assertRaises(compare.InvalidEvidence):
            compare.read_bounded(path, 4)
        old = compare.MAX_EVENTS
        try:
            compare.MAX_EVENTS = 1
            with self.assertRaises(compare.InvalidEvidence):
                compare.parse_kf((kf() * 2).encode())
            with self.assertRaises(compare.InvalidEvidence):
                compare.parse_native(decoded([native(), native()]), 'kgwt-decoded-json')
        finally:
            compare.MAX_EVENTS = old

    def test_cli_outputs_never_overwrite_evidence_or_existing_report(self):
        self.run_row('k1', events=kf())
        self.report()
        manifest = self.root / 'manifest.json'
        report = self.root / 'report.json'
        md = self.root / 'report.md'
        self.assertEqual(compare.main([str(manifest), '--json', str(report), '--markdown', str(md)]), 0)
        before = report.read_bytes()
        with self.assertRaises(SystemExit):
            compare.main([str(manifest), '--json', str(report), '--markdown', str(md)])
        self.assertEqual(before, report.read_bytes())
        with self.assertRaises(SystemExit):
            compare.main([str(manifest), '--json', str(manifest), '--markdown', str(self.root / 'new.md')])


if __name__ == '__main__':
    unittest.main()
