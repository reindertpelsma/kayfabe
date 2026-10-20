# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import unittest
import metric


class Classes(unittest.TestCase):
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

    def test_healthy_result_is_not_absence_claim(self):
        self.assertEqual(metric.classify({},[],[])['failure_class'],'NO_EVENT_OBSERVED')


if __name__=='__main__': unittest.main()
