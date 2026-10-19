import unittest
from block_layout import select_ntfs

G=1024**3
ROOT={'path':'/dev/nbd15','size':80*G,'fstype':None}
WIN={'path':'/dev/nbd15p3','size':78*G,'fstype':'ntfs'}
REC={'path':'/dev/nbd15p4','size':G//2,'fstype':'ntfs'}
OTHER={'path':'/dev/nbd14p3','size':90*G,'fstype':'ntfs'}
class VerifiedNtfs(unittest.TestCase):
 def test_flat_actual_shape_selects_large_windows(self):
  self.assertEqual(select_ntfs({'blockdevices':[ROOT,REC,WIN]}),WIN['path'])
 def test_flat_single_windows_partition(self):
  self.assertEqual(select_ntfs({'blockdevices':[ROOT,WIN]}),WIN['path'])
 def test_nested_selects_windows_over_recovery(self):
  self.assertEqual(select_ntfs({'blockdevices':[{**ROOT,'children':[REC,WIN]}]}),WIN['path'])
 def test_flat_unknown_filesystem_refused(self):
  with self.assertRaises(AssertionError):select_ntfs({'blockdevices':[ROOT,{**WIN,'fstype':None}]})
 def test_nested_unknown_filesystem_refused(self):
  with self.assertRaises(AssertionError):select_ntfs({'blockdevices':[{**ROOT,'children':[{**WIN,'fstype':None}]}]})
 def test_tiny_flat_recovery_refused(self):
  with self.assertRaises(AssertionError):select_ntfs({'blockdevices':[ROOT,REC]})
 def test_tiny_nested_recovery_refused(self):
  with self.assertRaises(AssertionError):select_ntfs({'blockdevices':[{**ROOT,'children':[REC]}]})
 def test_flat_other_device_refused(self):
  with self.assertRaises(AssertionError):select_ntfs({'blockdevices':[ROOT,OTHER]})
 def test_nested_other_device_refused(self):
  with self.assertRaises(AssertionError):select_ntfs({'blockdevices':[{**ROOT,'children':[OTHER]}]})
if __name__=='__main__':unittest.main()
