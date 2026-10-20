# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Scoped wait coverage: parser mutation fixtures and actual channel worker."""
import importlib.util
from pathlib import Path
import unittest
spec=importlib.util.spec_from_file_location('wait_audit',Path(__file__).with_name('wait_audit.py'))
audit=importlib.util.module_from_spec(spec);spec.loader.exec_module(audit)
ROOT=Path(__file__).resolve().parents[3]
class WaitAuditTests(unittest.TestCase):
    def test_new_raw_wait_before_a_wrapper_is_rejected(self):
        self.assertEqual(len(audit.unwrapped('let x=p.wait(&mut a, b); prof_wait!(ChanWorkerPark,p.wait(&mut a,b));')),1)
    def test_multiline_raw_wait_after_a_wrapper_is_rejected(self):
        self.assertEqual(len(audit.unwrapped('prof_wait!(ChanWorkerPark,p.wait(&mut a,b)); let y=p\n.wait\n(&mut a,b);')),1)
    def test_raw_identifier_wait_cannot_evade_the_scope_gate(self):
        self.assertEqual(len(audit.unwrapped('let x=p.r#wait(&mut a, b);')),1)
    def test_qualified_wait_cannot_evade_the_scope_gate(self):
        self.assertEqual(len(audit.unwrapped('let x=Poller::wait(&p, &mut a, b);')),1)
    def test_a_character_literal_cannot_leave_the_wrapper_scope_open(self):
        self.assertEqual(len(audit.unwrapped("prof_wait!(S, { let c='('; p.wait(a,b) }); p.wait(a,b);")),1)
    def test_comments_and_strings_are_not_waits(self):
        self.assertEqual(audit.unwrapped('/* p.wait(); /* nested */ */ let x=".wait()"; // p.wait()\n'),[])
    def test_wrapped_waits_keep_nested_arguments_inside_the_wrapper(self):
        self.assertEqual(audit.unwrapped('prof_wait!(ChanWorkerPark, p.wait(&mut a, PollTimeout::Millis(f(x))));'),[])
    def test_channel_worker_has_no_unwrapped_wait(self):
        self.assertEqual(audit.unwrapped((ROOT/'crates/kf-chan/src/worker.rs').read_text()),[])
    def test_worker_blocking_park_has_its_own_static_site(self):
        s=(ROOT/'crates/kf-chan/src/worker.rs').read_text()
        self.assertRegex(s,r'prof_wait!\(\s*ChanWorkerPark,')
    def test_worker_immediate_poll_has_its_own_static_site(self):
        s=(ROOT/'crates/kf-chan/src/worker.rs').read_text()
        self.assertRegex(s,r'prof_wait!\(\s*ChanWorkerImmediate,')
if __name__=='__main__':unittest.main()
