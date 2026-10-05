#!/usr/bin/env python3
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import compare
import native_cohort


def observation(fn=76, generation=1, direction='reply'):
    row = dict(direction=direction, rpc_function=fn, rpc_status=0,
               missing_before=0, prefix_unknown=True, generation=generation, trigger=1,
               payload_hex='private-payload', table_pa='private-address')
    if fn == 76:
        row['control'] = dict(command=0x20801111, status=0x56,
                              params_bytes_declared=40, params_bytes_observed=40, params_complete=True)
    return row


def decoded(rows, **extra):
    return json.dumps(dict(schema=compare.NATIVE_SCHEMA, complete=False, records=len(rows),
                           observations=rows, **extra)).encode()


class NativeCohortTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root/'values.tsv').write_text(
            'rpc_functions\tNV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL\t76\n'
            'rpc_events\tNV_VGPU_MSG_EVENT_FIRST_EVENT\t4096\n'
            'rpc_events\tNV_VGPU_MSG_EVENT_GSP_INIT_DONE\t4097\n'
            'rpc_events\tNV_VGPU_MSG_EVENT_NUM_EVENTS\t4131\n'
            'ctrl_cmds\tNV2080_CTRL_CMD_TEST\t545263889\n')
        self.source = dict(values='values.tsv',version='test',commit='source-declaration')
        self.maps, _ = native_cohort.source_map(self.source, self.root)

    def test_repeated_attachment_prefix_is_retained_not_boot_deduplicated(self):
        rows = [observation(72, 1, 'request'), observation(72, 2, 'request'), observation(4097, 2)]
        events, details = compare.parse_native(decoded(rows), 'kgwt-decoded-json')
        summary = native_cohort.native_summary(dict(rpc=dict(events=events,details=details,coverage='partial')), self.maps)
        self.assertEqual(summary['records'], 3)
        groups = summary['attachment_generations']
        self.assertEqual([(g['generation'],g['records']) for g in groups], [(1,1),(2,2)])
        self.assertEqual(groups[0]['streams']['request']['sequence_sha256'],groups[1]['streams']['request']['sequence_sha256'])
        self.assertEqual(groups[1]['streams']['asynchronous_status_queue']['records'],1)
        self.assertEqual(groups[1]['streams']['rpc_status_queue']['records'],0)
        self.assertNotIn('private',json.dumps(summary))

    def test_unknown_event_not_guessed_from_numeric_range_and_markers_not_events(self):
        for fn in (4096,4131,9999):
            event, _ = compare.parse_native(decoded([observation(fn)]),'kgwt-decoded-json')
            self.assertEqual(native_cohort.stream_kind(event[0],self.maps),'unknown_status_queue')

    def test_missing_generation_old_decoder_preserved_as_unknown(self):
        row=observation();row.pop('generation');row.pop('trigger')
        events,_=compare.parse_native(decoded([row]),'kgwt-decoded-json')
        self.assertIsNone(events[0]['generation'])
        for mutate in ({'generation':0,'trigger':1},{'generation':1},{'generation':1,'trigger':5},
                       {'generation':True,'trigger':1},{'generation':1<<64,'trigger':1}):
            with self.subTest(mutate=mutate), self.assertRaises(compare.InvalidEvidence):
                compare.parse_native(decoded([{**row,**mutate}]),'kgwt-decoded-json')

    def test_outer_success_inner_failure_is_not_success_and_requests_are_excluded(self):
        reply=observation();request=observation(direction='request');request['control']['status']=0
        events,_=compare.parse_native(decoded([request,reply]),'kgwt-decoded-json')
        counts=native_cohort.control_statuses(events,'0x20801111',True)
        self.assertEqual(counts['observed_count'],1)
        self.assertFalse(counts['observed_control_success'])
        self.assertEqual(counts['statuses'][0]['outer_or_result'],'0x00000000')
        self.assertEqual(counts['statuses'][0]['control_status'],'0x00000056')
        reply['rpc_status']=0x56;reply['control']['status']=0
        events,_=compare.parse_native(decoded([reply]),'kgwt-decoded-json')
        self.assertFalse(native_cohort.control_statuses(events,'0x20801111',True)['observed_control_success'])

    def test_stats_coverage_and_observation_hash_preserved_without_unknown_fields(self):
        raw=decoded([observation()],driver_stats=dict(recorded=1,dropped=0,unstable_snapshots=7,
                    invalid_elements=1000,limited=False,private_field='secret'),
                    text_export=dict(source_sha256='s'*64,observations_sha256='o'*64))
        _,details=compare.parse_native(raw,'kgwt-decoded-json')
        self.assertEqual(details['driver_stats']['unstable_snapshots'],7)
        self.assertEqual(details['source_export']['observations_sha256'],'o'*64)
        self.assertTrue(details['sampled'])
        self.assertNotIn('secret',json.dumps(details))

    def test_no_native_only_todo_and_missing_remains_not_observed(self):
        identity=dict(baseline_sha256='a',windows_driver='test',driver_hash='b',gpu='test',qemu_revision='c')
        native=observation();native['control']['command']=0x20800042
        (self.root/'n.json').write_bytes(decoded([native]))
        (self.root/'k.log').write_text('kf-rm: rpc-trace fn=76 RmControl seq=0 cmd=0x20801111 result=none\n')
        runs=[]
        for name,arm,path,fmt in [('n','vfio','n.json','kgwt-decoded-json'),('k','kayfabe','k.log','kf-rm')]:
            runs.append(dict(id=name,arm=arm,variant='test',flags={},instrumentation='test',metadata=identity,
                             rpc=dict(path=path,format=fmt,coverage='partial',window='test',note='test')))
        doc=dict(schema=1,experiment=identity,source=self.source,runs=runs)
        (self.root/'manifest.json').write_text(json.dumps(doc))
        report=native_cohort.build(self.root/'manifest.json')
        rows=report['controls_observed_by_kayfabe']
        self.assertEqual(len(rows),1)
        self.assertEqual(rows[0]['command'],'0x20801111')
        self.assertEqual(rows[0]['native_status_queue']['n']['observed_count'],0)
        self.assertIsNone(rows[0]['native_status_queue']['n']['first_observation'])
        self.assertIsNone(rows[0]['kayfabe']['k']['statuses'][0]['outer_or_result'])
        self.assertNotIn('private',json.dumps(report))

    def test_incomplete_source_vocabulary_fails(self):
        (self.root/'values.tsv').write_text('ctrl_cmds\tNV2080_CTRL_CMD_TEST\t545263889\n')
        with self.assertRaises(compare.InvalidEvidence):
            native_cohort.source_map(self.source,self.root)

    def test_many_attachment_epochs_refused_without_dropping_prefixes(self):
        rows=[observation(72,n,'request') for n in range(1,native_cohort.MAX_ATTACHMENTS+2)]
        events,details=compare.parse_native(decoded(rows),'kgwt-decoded-json')
        with self.assertRaisesRegex(compare.InvalidEvidence,'attachment generations'):
            native_cohort.native_summary(dict(rpc=dict(events=events,details=details,coverage='partial')),self.maps)

    def test_repeat_comparison_separates_binary_and_generation_and_compact_retains_counts_hash(self):
        def run(name, binary, rows):
            events,details=compare.parse_native(decoded(rows),'kgwt-decoded-json')
            return dict(id=name,arm='vfio',variant='native',flags={},instrumentation='observer',
                        metadata=dict(qemu_sha256=binary),rpc=dict(events=events,details=details,coverage='partial',window='startup'))
        runs=[run('a','same',[observation(72,1,'request'),observation(76,2)]),
              run('b','same',[observation(72,1,'request'),observation(76,2),observation(4097,2)]),
              run('c','different',[observation(72,1,'request')])]
        groups=native_cohort.repeated_native(runs,self.maps)
        self.assertEqual([g['runs'] for g in groups],[['a','b'],['c']])
        status=next(s for s in groups[0]['strata'] if s['generation']==2 and s['stream']=='rpc_status_queue')
        self.assertEqual(status['common_observed_prefix_records'],1)
        report=dict(runs=[dict(arm='vfio',rpc=native_cohort.native_summary(runs[0],self.maps))])
        original=report['runs'][0]['rpc']['attachment_generations'][1]['streams']['rpc_status_queue']['counts']
        expected=compare.digest(compare.canonical(original).encode())
        native_cohort.compact(report)
        stream=report['runs'][0]['rpc']['attachment_generations'][1]['streams']['rpc_status_queue']
        self.assertEqual(stream['counts_sha256'],expected)
        self.assertEqual(stream['records'],1)
        self.assertNotIn('counts',stream)


if __name__ == '__main__':
    unittest.main()
