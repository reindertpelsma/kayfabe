# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mechanical adapter contracts; native behavior is covered by existing relay tests."""
from pathlib import Path
import unittest
ROOT=Path(__file__).resolve().parents[3]
class RelayWiring(unittest.TestCase):
    def code(self):return (ROOT/'crates/kf-qemu/src/userpump.rs').read_text()
    def method(self,start,end):return self.code().split(start,1)[1].split(end,1)[0]
    def test_private_get_and_reference_are_observed_at_the_real_existing_read(self):
        s=self.method('fn hw_words(', 'fn write_guest_get(');self.assertIn('prof_private_get',s);self.assertIn('prof_reference',s)
        self.assertEqual(s.count('storage.gpu_get()'),1);self.assertEqual(s.count('storage.gpu_reference()'),1)
    def test_successful_put_writes_are_profiled_at_all_six_existing_sites(self):
        s=self.code();self.assertEqual(s.count('prof_private_put = Some(put)'),6);self.assertEqual(s.count('storage.publish_put(put)'),6)
    def test_wait_kind_and_target_are_published_after_the_real_arm_transition(self):
        s=self.method('fn user_ring_arm(', 'pub(crate) fn user_ring_waiting(')
        self.assertLess(s.index('p.wait = wait_of(need)'),s.index('p.prof_observe('))
    def test_existing_peek_refreshes_private_words_without_new_reads(self):
        s=self.method('fn user_ring_peek_one(', 'pub(super) fn user_ring_wake(')
        self.assertIn('prof_private_get',s);self.assertIn('prof_reference',s);self.assertIn('p.prof_observe(',s)
        self.assertEqual(s.count('storage.gpu_get()'),1);self.assertEqual(s.count('storage.gpu_reference()'),1)
    def test_birth_registration_and_observation_are_feature_gated(self):
        s=self.code();self.assertRegex(s,r'#\[cfg\(feature = "prof"\)\]\s*profile:')
        self.assertIn('register_relay_global',s);self.assertIn('if !kf_prof::runtime::is_on()',s)
    def test_wake_reasons_come_from_the_existing_peek_cause(self):
        s=self.code()
        for cause in ['Wake::Event','Wake::Spin','Wake::Backstop','Wake::Demand']:self.assertIn(cause,s)
    def test_source_registration_is_in_birth_setup(self):
        s=self.method('pub(super) fn new(me: &ChanPlane','pub(super) fn release(')
        self.assertEqual(s.count('register_relay_global'),1)
    def test_per_entry_publications_capture_all_three_real_put_stores(self):
        s=self.method('fn publish_entry(', 'fn event(');self.assertEqual(s.count('prof_private_put = Some(put)'),3)
    def test_segment_publications_capture_the_other_three_real_put_stores(self):
        s=self.code().split('pub(super) fn new(me: &ChanPlane',1)[1]
        self.assertEqual(s.count('prof_private_put = Some(put)'),3)
    def test_peek_wake_classification_matches_the_existing_cause(self):
        s=' '.join(self.code().split())
        for pair in ['Peek::Event => Wake::Event','Peek::Spin => Wake::Spin','Peek::Backstop => Wake::Backstop']:self.assertTrue(pair in s,pair)
    def test_durable_demand_does_not_claim_an_interrupt_wake(self):
        s=self.method('pub(super) fn user_ring_serve(', 'fn user_ring_arm(')
        self.assertTrue('Wake::Demand' in s,'durable demand must retain its own reason')
    def test_private_storage_lock_has_its_own_wait_site(self):
        s=self.method('fn hw_words(', 'fn write_guest_get(');self.assertTrue('Site::RelayStorage' in s)
    def test_peek_map_lock_has_its_own_wait_site(self):
        s=self.method('pub(crate) fn user_ring_peek(', 'fn user_ring_peek_one(');self.assertTrue('Site::RelayMapLock' in s)
    def test_guest_progress_is_separately_captured(self):
        s=self.method('fn write_guest_get(', 'fn begin_entry(')
        self.assertIn('prof_guest_get',s);self.assertIn('prof_guest_put',s)
if __name__=='__main__':unittest.main()
