# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import importlib.util
from pathlib import Path
import unittest

s=importlib.util.spec_from_file_location('packets',Path(__file__).with_name('packets.py'))
packets=importlib.util.module_from_spec(s)
if Path(s.origin).exists():
    s.loader.exec_module(packets)


def ev(event, context, sequence, pid=12):
    return (event, context, sequence, pid, '2026-10-20T01:00:00Z')


class PacketTests(unittest.TestCase):
    def test_matching_pair_removed(self):
        self.assertEqual(packets.pair([ev(178,'a','1'),ev(180,'a','1')])['pending'],[])

    def test_other_sequence_not_a_completion(self):
        self.assertEqual(len(packets.pair([ev(178,'a','1'),ev(180,'a','2')])['pending']),1)

    def test_other_context_not_a_completion(self):
        self.assertEqual(len(packets.pair([ev(178,'a','1'),ev(180,'b','1')])['pending']),1)

    def test_pid_preserved(self):
        self.assertEqual(packets.pair([ev(178,'a','1',42)])['pending'][0]['execution_pid'],42)

    def test_complete_count(self):
        self.assertEqual(packets.pair([ev(178,'a','1'),ev(180,'a','1')])['completed'],1)

    def test_two_pending_contexts(self):
        r=packets.pair([ev(178,'a','1'),ev(178,'b','1')])
        self.assertEqual(len(r['pending']),2)

    def test_two_pending_sequences(self):
        r=packets.pair([ev(178,'a','1'),ev(178,'a','2')])
        self.assertEqual(len(r['pending']),2)

    def test_window_leading_stop_not_invented_start(self):
        r=packets.pair([ev(180,'a','1')])
        self.assertEqual(r['pending'],[])
        self.assertEqual(r['unmatched_stop'],1)


if __name__=='__main__':
    unittest.main()
