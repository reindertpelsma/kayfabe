# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import unittest
import metric
import tempfile,json
from pathlib import Path


class Classes(unittest.TestCase):
    def stderr_time(self,line):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)
            (p/'guest.json').write_text(json.dumps(dict(frequency=1000,start_qpc=10000,end_qpc=80000,pair='heaven-gravity')))
            (p/'pm_live.csv').write_text('Application,ProcessID,TimeInQPC\n10000,Heaven.exe,1,10000\n')
            (p/'trig.log').write_text('')
            (p/'life-GravityMark.tsv').write_text('START\t2\t12000\t0\n')
            (p/'GravityMark.stderr.txt').write_text(line)
            return metric.analyze(p)['classes']

    def test_seconds_style_device_loss_time(self):
        self.assertAlmostEqual(self.stderr_time('E:  24.611 s: VK::error(): device lost')['time_to_first_device_loss_s'],26.611)

    def test_seconds_style_first_failure_time(self):
        self.assertAlmostEqual(self.stderr_time('E:  24.611 s: VK::error(): device lost')['time_to_first_failure_s'],26.611)

    def test_unknown_device_loss_timestamp_is_null(self):
        self.assertIsNone(self.stderr_time('E: device lost')['time_to_first_device_loss_s'])

    def test_unknown_time_still_classifies_device_loss(self):
        r=self.stderr_time('E: device lost')
        self.assertEqual(r['failure_class'],'DEVICE_LOST_TDR')
        self.assertIsNone(r['time_to_first_failure_s'])

    def test_nvlddmkm_153_is_device_lost_class(self):
        r=metric.classify({'a':{'count':1,'total_s':5,'longest_s':5}},[{'provider':'nvlddmkm','id':153,'elapsed_s':60}],[])
        self.assertEqual(r['failure_class'],'DEVICE_LOST_TDR')
        self.assertEqual(r['time_to_first_event153_s'],60)

    def test_kernel_boot_153_is_not_tdr(self):
        r=metric.classify({},[{'provider':'Microsoft-Windows-Kernel-Boot','id':153,'elapsed_s':1}],[])
        self.assertIsNone(r['time_to_first_event153_s'])

    def test_earliest_event_wins(self):
        events=[{'provider':'nvlddmkm','id':153,'elapsed_s':61},{'provider':'nvlddmkm','id':153,'elapsed_s':60}]
        self.assertEqual(metric.classify({},events,[])['time_to_first_event153_s'],60)

    def test_device_loss_excludes_independent_count(self):
        r=metric.classify({'a':{'count':4,'total_s':20,'longest_s':5}},[{'provider':'nvlddmkm','id':153,'elapsed_s':60}],[])
        self.assertEqual(r['independent_silences_count'],0)

    def test_silence_without_153_is_separate_class(self):
        r=metric.classify({'a':{'count':1,'total_s':5,'longest_s':5}},[],[])
        self.assertEqual(r['failure_class'],'INDEPENDENT_FREEZE')
        self.assertEqual(r['independent_silences_count'],1)

    def test_stderr_device_loss_is_not_independent(self):
        r=metric.classify({'a':{'count':2,'total_s':10,'longest_s':5}},[],[{'elapsed_s':58,'line':'VK error device lost'}])
        self.assertEqual(r['independent_silences_count'],0)
        self.assertEqual(r['time_to_first_device_loss_s'],58)

    def test_device_loss_without_events_has_device_lost_class(self):
        self.assertEqual(metric.classify({},[],[{'elapsed_s':58,'line':'device lost'}])['failure_class'],'DEVICE_LOST_TDR')

    def test_kernel_boot_does_not_exclude_independent_silence(self):
        r=metric.classify({'a':{'count':1,'total_s':5,'longest_s':5}},[{'provider':'Microsoft-Windows-Kernel-Boot','id':153,'elapsed_s':1}],[])
        self.assertEqual(r['independent_silences_count'],1)

    def test_healthy_result_is_not_absence_claim(self):
        self.assertEqual(metric.classify({},[],[])['failure_class'],'NO_EVENT_OBSERVED')


if __name__=='__main__': unittest.main()
