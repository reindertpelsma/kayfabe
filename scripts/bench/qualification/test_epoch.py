"""Tests-first proof of Q1's explicitly approved pre-arm history contract."""
import importlib.util
from pathlib import Path
import unittest

PATH = Path(__file__).with_name('epoch.py')
spec = importlib.util.spec_from_file_location('epoch', PATH)
E = importlib.util.module_from_spec(spec)
spec.loader.exec_module(E)

OLD = 'old kernel: NVRM: Xid (PCI:0000:01:00): 13, old fault'
NEW = 'new kernel: NVRM: Xid (PCI:0000:01:00): 31, new fault'

class EpochTests(unittest.TestCase):
    def test_history_is_excluded_from_delta(self):
        self.assertEqual(E.delta(OLD+'\n', (OLD,)), ())
    def test_other_journal_records_do_not_change_epoch(self):
        self.assertEqual(E.delta('header\n'+OLD+'\nother\n', (OLD,)), ())
    def test_new_xid_is_preserved(self):
        self.assertEqual(E.delta(OLD+'\n'+NEW, (OLD,)), (NEW,))
    def test_duplicate_new_xid_is_preserved(self):
        self.assertEqual(E.delta(OLD+'\n'+NEW+'\n'+NEW, (OLD,)), (NEW,NEW))
    def test_removed_history_is_refused(self):
        with self.assertRaises(ValueError): E.delta(NEW, (OLD,))
    def test_changed_history_is_refused(self):
        with self.assertRaises(ValueError): E.delta(OLD+' altered\n'+NEW, (OLD,))
    def test_windows_zero_delta_passes(self):
        E.zero_delta(OLD+'\n', (OLD,))
    def test_windows_new_xid_is_fatal(self):
        with self.assertRaises(ValueError): E.zero_delta(OLD+'\n'+NEW, (OLD,))
    def test_windows_duplicate_xid_is_fatal(self):
        with self.assertRaises(ValueError): E.zero_delta(OLD+'\n'+OLD, (OLD,))
    def test_wrong_candidate_revision_directory_refused(self):
        with self.assertRaises(ValueError): E.pin_binary('/tmp/wrong/qemu-system-x86_64', 'f'*40)
    def test_wrong_executable_name_refused(self):
        with self.assertRaises(ValueError): E.pin_binary('/tmp/'+('f'*8)+'/other', 'f'*40)
    def test_separate_product_pin_is_recorded(self):
        self.assertEqual(E.pin_binary('/tmp/'+('f'*8)+'/qemu-system-x86_64', 'f'*40), 'f'*40)
    def test_invalid_product_pin_refused(self):
        with self.assertRaises(ValueError): E.pin_binary('/tmp/ffff/qemu-system-x86_64', 'ffff')
    def test_filtered_reader_requests_all_xids(self):
        seen=[]
        def run(argv, **kw):
            seen.append((argv,kw))
            return type('Result',(),{'returncode':0,'stdout':(OLD+'\n'+NEW+'\n').encode(),'stderr':b''})()
        self.assertEqual(E.read_xids(run), (OLD,NEW))
        self.assertIn('--grep=NVRM: Xid',seen[0][0])
    def test_filtered_reader_preserves_duplicate_records(self):
        run=lambda *a,**k:type('Result',(),{'returncode':0,'stdout':(NEW+'\n'+NEW).encode(),'stderr':b''})()
        self.assertEqual(E.read_xids(run),(NEW,NEW))
    def test_filtered_reader_refuses_over_bound(self):
        run=lambda *a,**k:type('Result',(),{'returncode':0,'stdout':b'x'*(16*1024*1024+1),'stderr':b''})()
        with self.assertRaises(ValueError): E.read_xids(run)
    def test_filtered_reader_refuses_failed_command(self):
        run=lambda *a,**k:type('Result',(),{'returncode':1,'stdout':b'','stderr':b'permission denied'})()
        with self.assertRaises(ValueError): E.read_xids(run)
    def test_filtered_reader_refuses_oversized_xid_data(self):
        data=(NEW+'\n').encode()*400000
        run=lambda *a,**k:type('Result',(),{'returncode':0,'stdout':data,'stderr':b''})()
        with self.assertRaises(ValueError): E.read_xids(run)
    def test_failed_reader_cannot_supply_plausible_xids(self):
        run=lambda *a,**k:type('Result',(),{'returncode':1,'stdout':NEW.encode(),'stderr':b'failed'})()
        with self.assertRaises(ValueError): E.read_xids(run)
    def test_filtered_reader_refuses_truncated_utf8(self):
        run=lambda *a,**k:type('Result',(),{'returncode':0,'stdout':b'\xff','stderr':b''})()
        with self.assertRaises(ValueError): E.read_xids(run)

if __name__ == '__main__': unittest.main()
