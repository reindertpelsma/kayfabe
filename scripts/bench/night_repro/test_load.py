# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import unittest
import metric


def rows(*pids): return [(i,'Heaven.exe',pid) for pid in pids for i in range(100)]


class Load(unittest.TestCase):
    def test_two_roles_are_present(self): self.assertTrue(metric.valid_load('heaven-heaven',rows(1,2),{'Heaven':{1},'Heaven2':{2}}))
    def test_one_role_two_sequential_pids_is_not_two_clients(self): self.assertFalse(metric.valid_load('heaven-heaven',rows(1,2),{'Heaven':{1,2}}))
    def test_unobserved_second_role_is_not_load(self): self.assertFalse(metric.valid_load('heaven-heaven',rows(1,2),{'Heaven':{1,2},'Heaven2':{3}}))
    def test_missing_second_role(self): self.assertFalse(metric.valid_load('heaven-heaven',rows(1),{'Heaven':{1}}))
    def test_single_pid_cannot_serve_both_roles(self): self.assertFalse(metric.valid_load('heaven-heaven',rows(1),{'Heaven':{1},'Heaven2':{1}}))
    def test_normal_vulkan_pair(self): self.assertTrue(metric.valid_load('heaven-gravity',rows(1)+[(i,'GravityMark.exe',2) for i in range(100)],{}))
    def test_missing_vulkan_client(self): self.assertFalse(metric.valid_load('heaven-gravity',rows(1),{}))


if __name__=='__main__': unittest.main()
